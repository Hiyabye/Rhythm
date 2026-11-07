use super::*;

/// Validates the local GGUF container and streams its complete identity. No weights are loaded.
pub fn verify_file(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    verify_artifact(path, expected, cancel, true)
}

/// A projector is a GGUF multimodal encoder, not a text model: its
/// architecture is `clip`, so only the container and the pinned identity apply.
pub(super) fn verify_projector_file(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    verify_artifact(path, expected, cancel, false)
}

pub(super) fn verify_artifact(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
    require_architecture: bool,
) -> Result<(String, u64), String> {
    let mut file = File::open(path).map_err(|_| "모델 파일을 열지 못했습니다.")?;
    let size = file
        .metadata()
        .map_err(|_| "모델 파일 크기를 읽지 못했습니다.")?
        .len();
    if size == 0 || size > 32 * 1024 * 1024 * 1024 || expected.is_some_and(|(_, n)| n != size) {
        return Err("모델 파일 크기가 올바르지 않습니다.".into());
    }
    validate_gguf(&file, size, require_architecture)?;
    file.rewind().map_err(|_| "모델 파일을 읽지 못했습니다.")?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        check_cancel(cancel)?;
        let n = file
            .read(&mut buffer)
            .map_err(|_| "모델 파일을 검증하지 못했습니다.")?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > size {
            return Err("검증 중 모델 파일이 변경되었습니다.".into());
        }
        hash.update(&buffer[..n]);
    }
    let hash = format!("{:x}", hash.finalize());
    if bytes != size || expected.is_some_and(|(h, _)| h != hash) {
        return Err("모델 SHA-256 무결성 검증에 실패했습니다. 다시 설치해 주세요.".into());
    }
    Ok((hash, size))
}
/// Buffered header reader. It tracks its own offset because the metadata walk
/// issues many tiny reads and skips, and `position` bounds every skip.
pub(super) struct GgufReader<'a> {
    pub(super) reader: BufReader<&'a File>,
    pub(super) position: u64,
}
impl GgufReader<'_> {
    fn read_exact(&mut self, buffer: &mut [u8]) -> std::io::Result<()> {
        self.reader.read_exact(buffer)?;
        self.position += buffer.len() as u64;
        Ok(())
    }
}
pub(super) fn read_u32(file: &mut GgufReader) -> Result<u32, String> {
    let mut b = [0; 4];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 메타데이터가 완전하지 않습니다.")?;
    Ok(u32::from_le_bytes(b))
}
pub(super) fn read_u64(file: &mut GgufReader) -> Result<u64, String> {
    let mut b = [0; 8];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 메타데이터가 완전하지 않습니다.")?;
    Ok(u64::from_le_bytes(b))
}
pub(super) fn gguf_string(file: &mut GgufReader, max: u64) -> Result<String, String> {
    let n = read_u64(file)?;
    if n > max {
        return Err("GGUF 메타데이터 문자열이 너무 큽니다.".into());
    }
    let mut b = vec![0; n as usize];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 문자열을 읽지 못했습니다.")?;
    String::from_utf8(b).map_err(|_| "GGUF 문자열이 UTF-8이 아닙니다.".into())
}
pub(super) fn skip(file: &mut GgufReader, bytes: u64, size: u64) -> Result<(), String> {
    let next = file
        .position
        .checked_add(bytes)
        .filter(|n| *n <= size && *n <= 64 * 1024 * 1024)
        .ok_or("GGUF 메타데이터 범위를 초과했습니다.")?;
    // Within the buffer this costs no syscall, unlike a seek on the bare file.
    file.reader
        .seek_relative(bytes as i64)
        .map_err(|_| "GGUF 메타데이터를 읽지 못했습니다.")?;
    file.position = next;
    Ok(())
}
pub(super) fn skip_value(
    file: &mut GgufReader,
    kind: u32,
    size: u64,
    in_array: bool,
) -> Result<(), String> {
    match kind {
        0 | 1 | 7 => skip(file, 1, size),
        2 | 3 => skip(file, 2, size),
        4..=6 => skip(file, 4, size),
        10..=12 => skip(file, 8, size),
        8 => {
            let n = read_u64(file)?;
            skip(file, n, size)
        }
        9 if !in_array => {
            let kind = read_u32(file)?;
            let count = read_u64(file)?;
            if count > 1_000_000 {
                return Err("GGUF 배열이 너무 큽니다.".into());
            }
            for _ in 0..count {
                skip_value(file, kind, size, true)?;
            }
            Ok(())
        }
        _ => Err("지원하지 않는 GGUF 메타데이터입니다.".into()),
    }
}
pub(super) fn validate_gguf(
    file: &File,
    size: u64,
    require_architecture: bool,
) -> Result<(), String> {
    let file = &mut GgufReader {
        reader: BufReader::new(file),
        position: 0,
    };
    let mut magic = [0; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "GGUF 파일이 완전하지 않습니다.")?;
    if magic != *b"GGUF" || ![2, 3].contains(&read_u32(file)?) {
        return Err("지원하는 GGUF 2/3 형식이 아닙니다.".into());
    }
    let tensors = read_u64(file)?;
    let metadata = read_u64(file)?;
    if tensors == 0 || tensors > 100_000 || metadata == 0 || metadata > 100_000 {
        return Err("GGUF 모델 메타데이터가 올바르지 않습니다.".into());
    }
    for _ in 0..metadata {
        let key = gguf_string(file, 1024)?;
        let kind = read_u32(file)?;
        if key == "general.architecture" {
            if !require_architecture {
                return Ok(());
            }
            if kind != 8 {
                return Err("GGUF 모델 아키텍처 형식이 올바르지 않습니다.".into());
            }
            let architecture = gguf_string(file, 128)?;
            if !["qwen3", "qwen35", "gemma4"].contains(&architecture.as_str()) {
                return Err(format!(
                    "이 GGUF 아키텍처는 Rhythm에서 검증하지 않았습니다: {architecture}"
                ));
            }
            return Ok(());
        }
        skip_value(file, kind, size, false)?;
    }
    if require_architecture {
        Err("GGUF 모델 아키텍처를 찾지 못했습니다.".into())
    } else {
        Ok(())
    }
}
