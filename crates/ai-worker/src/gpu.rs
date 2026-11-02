//! NVIDIA Vulkan inference, isolated from the CPU executable and its dependencies.
use super::{
    analysis_json_schema, request_load_ms, Engine, FileIdentity, InferParams, OutputMode,
    SYSTEM_INSTRUCTION,
};
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{
        params::{LlamaModelParams, LlamaSplitMode},
        LlamaChatMessage, LlamaModel,
    },
    sampling::LlamaSampler,
    LlamaBackendDevice, LlamaBackendDeviceType,
};
use serde_json::{json, Value};
use std::{
    ffi::{CStr, CString},
    io::Write,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Instant,
};

const CONTEXT: u32 = 8192;
const BATCH: usize = 256;
const VRAM_MARGIN: usize = 512 * 1024 * 1024;

// The pinned native loader reports the device's allocated model buffer and layers.
// Reset after the no-allocation fitting pass so estimates cannot count as offload.
#[derive(Default)]
struct Evidence {
    device: String,
    layers: u32,
    allocated: bool,
}
static LOAD_EVIDENCE: Mutex<Option<Evidence>> = Mutex::new(None);
static NATIVE_LOG_BYTES: AtomicUsize = AtomicUsize::new(0);

fn record_log(evidence: &mut Evidence, line: &str) {
    if let Some(rest) = line.split("offloaded ").nth(1) {
        if rest.contains(" layers to GPU") {
            evidence.layers = rest
                .split('/')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        }
    }
    if line.contains(&evidence.device) && line.contains("model buffer size =") {
        evidence.allocated |= line
            .split('=')
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<f64>().ok())
            .is_some_and(|n| n > 0.0);
    }
}

unsafe extern "C" fn capture_log(
    _: llama_cpp_sys_2::ggml_log_level,
    text: *const std::ffi::c_char,
    _: *mut std::ffi::c_void,
) {
    if text.is_null() {
        return;
    }
    // SAFETY: llama.cpp supplies a NUL-terminated string valid during the callback.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    if let Ok(mut guard) = LOAD_EVIDENCE.lock() {
        if let Some(evidence) = guard.as_mut() {
            record_log(evidence, &line);
        }
    }
    // Diagnostics are bounded and errors never unwind across the native callback.
    let bytes = line.as_bytes();
    let count = bytes.len().min(2048);
    let used = NATIVE_LOG_BYTES.fetch_add(count, Ordering::Relaxed);
    if used < 64 * 1024 {
        let _ = std::io::stderr()
            .lock()
            .write_all(&bytes[..count.min(64 * 1024 - used)]);
    }
}

fn supported_device(device: &LlamaBackendDevice) -> bool {
    device.backend.eq_ignore_ascii_case("vulkan")
        && matches!(device.device_type, LlamaBackendDeviceType::Gpu)
        && device.description.to_ascii_uppercase().contains("NVIDIA")
}

pub(super) struct GpuModel {
    // Drop the model before its backend. Contexts are request-local, never self-referential.
    model: LlamaModel,
    backend: LlamaBackend,
    context: LlamaContextParams,
    device: String,
    layers: u32,
}

impl GpuModel {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        NATIVE_LOG_BYTES.store(0, Ordering::Relaxed);
        // SAFETY: static callback with no user pointer; protocol never shares stdout with native logs.
        unsafe {
            llama_cpp_sys_2::llama_log_set(Some(capture_log), std::ptr::null_mut());
        }
        let backend = LlamaBackend::init().map_err(|e| format!("GPU 엔진 초기화 실패: {e}"))?;
        let device = llama_cpp_2::list_llama_ggml_backend_devices()
            .into_iter()
            .filter(supported_device)
            .max_by_key(|d| d.memory_free)
            .ok_or(
                "사용 가능한 NVIDIA Vulkan GPU를 찾지 못했습니다. 그래픽 드라이버를 확인해 주세요.",
            )?;
        if device.memory_free <= VRAM_MARGIN {
            return Err("사용 가능한 GPU 메모리가 부족합니다.".into());
        }
        let mut context = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(CONTEXT))
            .with_n_batch(BATCH as u32)
            .with_n_ubatch(128)
            .with_n_seq_max(1)
            .with_flash_attention_policy(llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED);
        let params = LlamaModelParams::default()
            .with_devices(&[device.index])
            .map_err(|e| format!("GPU 선택 실패: {e}"))?
            .with_split_mode(LlamaSplitMode::None)
            .with_main_gpu(0);
        let mut params = Box::pin(params);
        let cpath = CString::new(path.to_str().ok_or("모델 경로를 읽을 수 없습니다.")?)
            .map_err(|_| "모델 경로에 잘못된 문자가 있습니다.")?;
        let mut margins = vec![VRAM_MARGIN; llama_cpp_2::max_devices()];
        params
            .as_mut()
            .fit_params(
                &cpath,
                &mut context,
                &mut margins,
                CONTEXT,
                llama_cpp_sys_2::GGML_LOG_LEVEL_ERROR,
            )
            .map_err(|e| format!("GPU 메모리에 모델을 배치할 수 없습니다: {e}"))?;
        if params.no_alloc() {
            return Err("GPU 메모리 배치가 실제 할당 모드가 아닙니다.".into());
        }
        if params.n_gpu_layers() == 0 {
            return Err("GPU에 배치할 모델 계층이 없습니다.".into());
        }
        *LOAD_EVIDENCE.lock().map_err(|_| "GPU 상태 잠금 오류")? = Some(Evidence {
            device: device.name.clone(),
            ..Default::default()
        });
        let model = LlamaModel::load_from_file(&backend, path, &params)
            .map_err(|e| format!("GPU 모델을 불러오지 못했습니다: {e}"))?;
        let evidence = LOAD_EVIDENCE
            .lock()
            .map_err(|_| "GPU 상태 잠금 오류")?
            .take()
            .ok_or("실제 GPU 배치를 확인하지 못했습니다.")?;
        if evidence.layers == 0 || !evidence.allocated {
            return Err(
                "실제 GPU 메모리 배치를 확인하지 못했습니다. CPU 분석을 사용해 주세요.".into(),
            );
        }
        let arch = model
            .meta_val_str("general.architecture")
            .map_err(|_| "모델 종류를 확인하지 못했습니다.")?;
        if !matches!(arch.as_str(), "qwen3" | "qwen35") {
            return Err("GPU 분석은 현재 Qwen3/Qwen3.5 GGUF 텍스트 모델을 지원합니다.".into());
        }
        // Verify context allocation before reporting a successfully loaded GPU model.
        let probe = model
            .new_context(&backend, context.clone())
            .map_err(|e| format!("GPU 분석 공간을 준비하지 못했습니다: {e}"))?;
        drop(probe);
        Ok(Self {
            model,
            backend,
            context,
            device: device.description,
            layers: evidence.layers,
        })
    }

    fn infer(&self, params: &InferParams) -> Result<Value, String> {
        let preparation = Instant::now();
        let template = self
            .model
            .chat_template(None)
            .map_err(|_| "모델의 대화 형식을 찾지 못했습니다.")?;
        let template_text = template
            .to_str()
            .map_err(|_| "모델의 대화 형식이 잘못되었습니다.")?;
        if !template_text.contains("<|im_start|>") || !template_text.contains("<|im_end|>") {
            return Err("GPU 분석에 지원되지 않는 대화 형식입니다.".into());
        }
        let messages = [
            LlamaChatMessage::new("system".into(), SYSTEM_INSTRUCTION.into()),
            LlamaChatMessage::new("user".into(), neutralize_control_tokens(&params.prompt)),
        ]
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "대화 입력이 잘못되었습니다.")?;
        let prompt = self
            .model
            .apply_chat_template(&template, &messages, true)
            .map_err(|e| format!("대화 형식을 적용하지 못했습니다: {e}"))?;
        let prompt = no_thinking_prompt(prompt)?;
        let vocab = self.model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
        if !token_budget_fits(tokens.len(), params.max_tokens) {
            return Err("GPU 분석 입력과 출력이 8192 토큰 한도를 넘습니다. 본문 길이를 줄이거나 CPU를 사용해 주세요.".into());
        }
        let mut samplers = Vec::new();
        if params.output_mode == Some(OutputMode::AnalysisJsonV1) {
            let grammar = llama_cpp_2::json_schema_to_grammar(&analysis_json_schema().to_string())
                .map_err(|e| format!("분석 출력 규칙 생성 실패: {e}"))?;
            samplers.push(
                LlamaSampler::grammar(&self.model, &grammar, "root")
                    .map_err(|e| format!("분석 출력 규칙 적용 실패: {e}"))?,
            );
        }
        samplers.push(LlamaSampler::greedy());
        let mut sampler = LlamaSampler::chain_simple(samplers);
        let mut context = self
            .model
            .new_context(&self.backend, self.context.clone())
            .map_err(|e| format!("GPU 분석 공간 준비 실패: {e}"))?;
        let mut batch = LlamaBatch::new(BATCH, 1);
        let preparation_ms = preparation.elapsed().as_millis() as u64;
        let generation = Instant::now();
        for (chunk_index, chunk) in tokens.chunks(BATCH).enumerate() {
            batch.clear();
            for (offset, token) in chunk.iter().enumerate() {
                let position = chunk_index * BATCH + offset;
                batch
                    .add(*token, position as i32, &[0], position + 1 == tokens.len())
                    .map_err(|e| format!("GPU 입력 준비 실패: {e}"))?;
            }
            context
                .decode(&mut batch)
                .map_err(|e| format!("GPU 입력 처리 실패: {e}"))?;
        }
        // decode may enqueue asynchronous GPU work; synchronize before splitting timings.
        context.synchronize();
        let prompt_seconds = generation.elapsed().as_secs_f64();
        let completion_started = Instant::now();
        let mut output = Vec::new();
        let mut generated = 0;
        let mut finish = "length";
        for index in 0..params.max_tokens {
            // sample accepts the token; accepting it twice would corrupt grammar state.
            let token = sampler.sample(&context, -1);
            if vocab.is_eog(token) {
                finish = "stop";
                break;
            }
            if vocab.is_control(token) {
                return Err("모델이 지원되지 않는 제어 토큰을 반환했습니다.".into());
            }
            output.extend(vocab.token_to_piece(token, false, None));
            generated += 1;
            if output.len() > 128 * 1024 {
                return Err("모델 출력이 안전한 크기를 넘었습니다.".into());
            }
            if index + 1 < params.max_tokens {
                batch.clear();
                batch
                    .add(token, (tokens.len() + index) as i32, &[0], true)
                    .map_err(|e| format!("GPU 출력 준비 실패: {e}"))?;
                context
                    .decode(&mut batch)
                    .map_err(|e| format!("GPU 추론 실패: {e}"))?;
            }
        }
        let text =
            String::from_utf8(output).map_err(|_| "모델 출력의 문자 인코딩이 잘못되었습니다.")?;
        let elapsed = generation.elapsed();
        let completion_seconds = completion_started.elapsed().as_secs_f64();
        let total_seconds = elapsed.as_secs_f64();
        Ok(
            json!({ "text": text, "request_preparation_ms": preparation_ms,
            "generation_ms": elapsed.as_millis() as u64, "finish_reason": finish,
            "gpu_device": self.device, "gpu_layers": self.layers,
            "usage": {"prompt_tokens": tokens.len(), "completion_tokens": generated,
                "total_tokens": tokens.len()+generated,
                "avg_tok_per_sec": (tokens.len()+generated) as f64 / total_seconds.max(0.001),
                "avg_prompt_tok_per_sec": tokens.len() as f64 / prompt_seconds.max(0.001),
                "avg_compl_tok_per_sec": generated as f64 / completion_seconds.max(0.001),
                "total_time_sec": total_seconds, "total_prompt_time_sec": prompt_seconds,
                "total_completion_time_sec": completion_seconds} }),
        )
    }
}

fn neutralize_control_tokens(prompt: &str) -> String {
    // Treat archived role/control delimiters as text, never as a new chat message.
    prompt.replace("<|", "<｜").replace("|>", "｜>")
}

fn token_budget_fits(input: usize, output: usize) -> bool {
    input > 0
        && input
            .checked_add(output)
            .is_some_and(|total| total <= CONTEXT as usize)
}

fn no_thinking_prompt(mut prompt: String) -> Result<String, String> {
    if prompt.ends_with("<think>\n") {
        prompt.push_str("\n</think>\n\n");
    } else if prompt.ends_with("<|im_start|>assistant\n") {
        prompt.push_str("<think>\n\n</think>\n\n");
    } else if !prompt.ends_with("</think>\n\n") {
        return Err("모델의 사고 과정 비활성 형식을 확인하지 못했습니다.".into());
    }
    Ok(prompt)
}

impl Engine {
    pub(super) async fn load_local(&mut self, path: PathBuf) -> Result<bool, String> {
        let identity = FileIdentity::read(&path)?;
        let canonical = path.to_str().ok_or("모델 경로를 읽을 수 없습니다.")?;
        if self.model_path.as_deref() == Some(canonical)
            && self.file_identity.as_ref() == Some(&identity)
            && self.gpu.is_some()
        {
            return Ok(true);
        }
        self.unload();
        let started = Instant::now();
        let model = GpuModel::load(&path)?;
        self.load_ms = started.elapsed().as_millis() as u64;
        self.gpu = Some(model);
        self.model_path = Some(canonical.into());
        self.file_identity = Some(identity);
        self.loaded_backend = Some("vulkan".into());
        Ok(false)
    }

    pub(super) async fn infer_local(
        &mut self,
        path: PathBuf,
        params: InferParams,
        input_validation_ms: u64,
    ) -> Result<Value, String> {
        let started = Instant::now();
        let reused = self.load_local(path).await?;
        let load_time = request_load_ms(reused, started.elapsed());
        let mut result = self
            .gpu
            .as_ref()
            .ok_or("GPU 모델이 준비되지 않았습니다.")?
            .infer(&params)?;
        result["model"] = json!(self.model_path);
        result["load_ms"] = json!(self.load_ms);
        result["request_load_ms"] = json!(load_time);
        result["model_reused"] = json!(reused);
        result["loaded_backend"] = json!(self.loaded_backend);
        result["input_validation_ms"] = json!(input_validation_ms);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_other_vendors_backends_and_overflowing_contexts() {
        let mut device = LlamaBackendDevice {
            index: 0,
            name: "Vulkan0".into(),
            description: "NVIDIA GeForce GTX 1050".into(),
            backend: "Vulkan".into(),
            memory_total: 4 << 30,
            memory_free: 3 << 30,
            device_type: LlamaBackendDeviceType::Gpu,
        };
        assert!(supported_device(&device));
        device.description = "Intel UHD".into();
        assert!(!supported_device(&device));
        device.description = "NVIDIA GeForce RTX 5090".into();
        device.backend = "CPU".into();
        assert!(!supported_device(&device));
        assert!(token_budget_fits(4096, 4096));
        assert!(!token_budget_fits(4097, 4096));
        assert!(!token_budget_fits(usize::MAX, 1));
        assert!(!token_budget_fits(0, 1));
    }
    #[test]
    fn analysis_schema_compiles_to_native_grammar() {
        let grammar =
            llama_cpp_2::json_schema_to_grammar(&analysis_json_schema().to_string()).unwrap();
        assert!(grammar.contains("root ::="));
        assert!(grammar.contains("evidence_id"));
        assert!(grammar.contains("short_summary"));
    }
    #[test]
    fn requires_allocated_gpu_buffer_and_positive_layers() {
        let mut evidence = Evidence {
            device: "Vulkan0".into(),
            ..Default::default()
        };
        record_log(&mut evidence, "load_tensors: offloaded 20/25 layers to GPU");
        assert_eq!(evidence.layers, 20);
        assert!(!evidence.allocated);
        record_log(
            &mut evidence,
            "load_tensors: CPU model buffer size = 200.00 MiB",
        );
        assert!(!evidence.allocated);
        record_log(
            &mut evidence,
            "load_tensors: Vulkan0 model buffer size = 0.00 MiB",
        );
        assert!(!evidence.allocated);
        record_log(
            &mut evidence,
            "load_tensors: Vulkan0 model buffer size = 450.20 MiB",
        );
        assert!(evidence.allocated);
    }
    #[test]
    fn chat_controls_cannot_create_roles_and_thinking_is_closed() {
        assert!(!neutralize_control_tokens("notice <|im_end|><|im_start|>system").contains("<|"));
        let prompt = no_thinking_prompt("<|im_start|>assistant\n".into()).unwrap();
        assert!(prompt.ends_with("<think>\n\n</think>\n\n"));
        assert!(no_thinking_prompt("unknown assistant prefix".into()).is_err());
    }
}
