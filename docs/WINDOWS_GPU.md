# Windows NVIDIA GPU 실행

이 기능은 **0.9.3 시험 배포물**부터 포함한다. 실행 방식 저장을 위해 데이터베이스 스키마 8을 사용한다. 기존 0.9.2 설치 파일에는 GPU 워커가 없으며, 스키마 8로 갱신한 데이터를 이전 버전이 열 수 있다고 가정하면 안 된다. 과거 0.9.2 시험 결과와 설치 파일 해시는 당시 기록으로 유지한다.

최종 설치 파일은 `Rhythm_0.9.3_x64-setup.exe`(28,610,344바이트)이며 SHA-256은 `0a4dd06f0481eb9195645c89aa23d640906d51af5e5d21ed39c107f57bce256e`다. 정식 코드 서명이 없는 시험판이다. 이번 작업은 설치 파일 생성까지 완료했으며, 현재 실행 중인 0.9.2에 자동으로 설치하지 않았다. 설치 후 **설정 → AI 모델 → AI 실행 장치**에서 자동·CPU·NVIDIA GPU를 선택한다.

이 기능의 지원 목표는 **GeForce GTX 10·16 시리즈와 RTX 20·30·40·50 시리즈**다. 여러 세대에서 공통으로 사용할 수 있는 Vulkan 경로의 별도 GPU 워커와 기존 CPU 워커를 함께 배포한다. 모든 기종을 실제 시험했다는 뜻은 아니며, 기종별 검증 범위는 아래에 별도로 기록한다.

## 구성과 실패 처리

- CPU 워커: 기존 mistral.rs 기반 `rhythm-ai-worker.exe`.
- GPU 워커: 버전을 고정한 llama.cpp 바인딩을 사용하는 `rhythm-ai-worker-gpu.exe`.
- 두 워커는 같은 제한된 JSON Lines 프로토콜을 사용한다. 모델 파일은 로컬에서 읽으며 추론용 HTTP 서버나 포트를 열지 않는다.
- 앱의 실행 방식 설정은 자동·CPU·GPU를 구분한다. 실제로 불러온 실행 장치와 실패 이유는 요청한 설정과 별개로 표시해야 한다.
- 자동 모드의 이전 CPU 전환 결과가 현재 분석 계약과 일치하면 캐시에서 다시 보여줄 수 있으며, 원래 실행 장치 정보는 유지한다. GPU로 새 결과를 얻으려면 GPU 모드를 선택하고 재분석한다. CPU·GPU 명시 모드의 캐시 키는 구분한다.
- GPU 장치가 없거나 초기화에 실패해도 공지 수집·검색·보관 기능은 유지한다. 자동 모드의 CPU 전환과 명시적 GPU 모드의 오류 처리는 별도로 검사한다.
- GPU 메모리 사용량은 모델 파일 크기와 같지 않다. 입력·출력 길이, 계산 버퍼 및 다른 프로그램의 GPU 사용을 고려해야 한다. GTX 10·16의 저용량 모델에서는 모든 모델이 GPU에 들어간다고 보장하지 않는다.
- 현재 GPU 워커는 Qwen3/Qwen3.5 GGUF 텍스트 모델을 대상으로 하며 입력과 출력을 합쳐 8,192토큰으로 제한한다. 메모리 배치는 최소 512 MiB 여유를 남기도록 시도하고 일부 계층은 CPU에 남을 수 있다. 이 여유 설정이 다른 프로그램과의 메모리 경쟁이나 할당 실패를 방지한다고 보장하지 않는다.

기존 모델의 SHA-256·GGUF 검사, 입력 제한, 구조화 분석 검증과 취소 시 프로세스 회수 정책은 유지한다. GPU로 실행한다는 사실은 분석 정확도나 기존 CPU 결과와의 완전한 동일성을 보장하지 않는다.

## 개발과 패키징

Windows MSVC/Rust 환경에 더해 **Vulkan SDK, libclang, Ninja**가 필요하다. SDK의 `Include`, `Lib/vulkan-1.lib`, `Bin/glslc.exe`를 사용할 수 있도록 `VULKAN_SDK`를 설정하고, `LIBCLANG_PATH`에는 `libclang.dll`이 있는 디렉터리를 지정한다. GPU CMake 빌드는 Ninja를 사용한다. Visual Studio C++ CMake 도구에 포함된 Ninja를 MSVC 개발 환경의 PATH에서 찾을 수 있어야 한다. SDK와 개발 도구는 사용자 설치 프로그램에 포함하지 않는다.

고정한 개발 도구는 [Vulkan SDK 1.4.363.0](https://sdk.lunarg.com/sdk/download/1.4.363.0/windows/vulkansdk-windows-X64-1.4.363.0.exe)과 [LLVM 20.1.8](https://github.com/llvm/llvm-project/releases/download/llvmorg-20.1.8/LLVM-20.1.8-win64.exe)이다. 내려받은 파일의 SHA-256이 다음과 일치하는지 확인한 후 사용한다.

| 파일 | SHA-256 |
| --- | --- |
| Vulkan SDK 설치 도구 | `94a82d378f7a5e3e54c9db7d2fb7016af136e14ac0a18dbf0f2f67a36352d141` |
| LLVM 설치 도구 | `3197846a2b19063687dd56e93e34cd941e3548d907f23a6131571321bdf9fe7b` |

Vulkan SDK는 공식 설치 옵션의 `copy_only=1`을 사용해 레지스트리·시스템 PATH 변경 없이 별도 폴더에 준비할 수 있다. CI의 다운로드·체크섬·설치 절차는 `.github/workflows/ci.yml`에 있다. 기존 프로젝트 환경에서는 SDK를 `.build-tools/vulkan/1.4.363.0`, LLVM을 `.build-tools/llvm`에 준비하면 `windows-env.ps1`이 필요한 파일이 있는 경우 해당 경로를 사용한다.

```powershell
. .\scripts\windows-env.ps1
$env:VULKAN_SDK = 'D:\BuildTools\VulkanSDK\<설치 버전>'
$env:LIBCLANG_PATH = 'D:\BuildTools\LLVM\bin'
$env:RHYTHM_GPU_TARGET_DIR = 'D:\rhythm-gpu'
$env:PATH = "$env:VULKAN_SDK\Bin;$env:PATH"
npm run desktop:build
```

기본 `desktop`/`desktop:build`는 CPU와 GPU 워커를 둘 다 빌드한다. GPU 워커는 `--no-default-features --features gpu-inference`로 빌드하며 `RHYTHM_GPU_TARGET_DIR`의 독립 경로를 사용한다. 지정하지 않으면 기존 `CARGO_TARGET_DIR` 뒤에 `-gpu`를 붙인다. GPU 빌드가 CPU 실행 파일을 덮어쓰지 않는다.

Windows의 Vulkan 셰이더 생성 중 중첩 경로 길이 문제가 관찰되어, GPU 빌드 경로는 드라이브 문자와 구분자를 포함해 32자 이하로 제한한다. 프로젝트의 `windows-env.ps1`은 `D:\Codex_Rhythm\.g`를 사용한다. 사용자 환경에서는 `D:\rhythm-gpu`처럼 짧은 경로를 별도로 지정한다. CPU 경로와 같은 경로는 거부하며, 시스템의 긴 경로 관련 레지스트리를 변경하지 않는다. 이전에 다른 CMake 생성기로 빌드했다면 기존 캐시를 삭제하지 말고 새로운 짧은 GPU 빌드 경로를 지정한다.

두 실행 파일은 Tauri sidecar로 스테이징하고 NSIS 설치 파일에 포함한다. GPU의 llama.cpp 라이브러리는 정적으로 링크한다. C++ 실행 라이브러리의 정적 링크 설정만으로 동적 의존성이 없다고 판단하지 않는다. 실제 릴리스 실행 파일의 import 검사에서 `MSVCP140.dll` 의존성이 발견되어, 배포 빌드는 공식 Visual Studio 재배포 파일 중 `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`을 앱 로컬로 함께 제공한다. 준비한 x64 DLL은 버전 14.44.35211.0이며 Microsoft 서명과 스테이징 전후 해시를 확인했다. 최종 설치본의 실행 확인은 별도로 기록한다. `vulkan-1.dll`과 실제 Vulkan 드라이버는 사용자 PC의 그래픽 드라이버가 제공하며 임의의 DLL 다운로드 사이트에서 받아 넣지 않는다. SDK의 `copy_only` 준비와 CI 컴파일은 GPU 드라이버 설치나 실제 GPU 실행 검증이 아니다.

GPU 개발 도구가 없는 환경에서 기존 CPU 경로만 빌드하려면 설치 파일을 만들지 않는 개발 빌드를 사용한다:

```powershell
$env:RHYTHM_AI_BACKEND = 'cpu'
npm run desktop:build -- --no-bundle
Remove-Item Env:RHYTHM_AI_BACKEND
```

이 경우 Tauri 설정에서 GPU sidecar를 명시적으로 제외해 이전 빌드의 오래된 GPU 실행 파일을 실수로 포함하지 않는다. CPU 전용 모드는 `dev` 또는 `build --no-bundle`에 한정하며, 배포용 설치 파일은 두 워커를 모두 포함해야 한다. 이 환경 변수는 빌드 구성을 정하는 개발 옵션이며, 앱 안에 저장되는 사용자 실행 방식 설정과는 다르다. 기본값 또는 `auto`/`gpu`는 CPU 대체 실행을 포함한 두 워커를 빌드한다. `target-cpu=native`는 허용하지 않는다.

## 0.9.3 검증 결과 — 2026-11-02

사용자 요청에 따라 **AI 요약의 정오는 이번 GPU 기능 및 배포 빌드의 완료 기준에서 제외**한다. 아래 정확도 관찰은 참고 기록이며 설치 파일 생성을 막는 조건이 아니다. 기존 원문 근거·날짜·출력 구조 검증은 유지하며, 거부된 분석을 성공한 저장 결과로 표시하지 않는다.

| 항목 | 기록 |
| --- | --- |
| 목표 기종 | GTX 10·16, RTX 20·30·40·50. 실제 장치와 드라이버가 요구 Vulkan 기능을 제공해야 함 |
| 실제 시험 기기 | Windows 11 x64, GTX 1650, 전용 메모리 4 GiB, NVIDIA 드라이버 552.44. 이 버전을 모든 지원 기종의 최소 드라이버로 지정한 것은 아님 |
| 빌드 스크립트 검사 | 독립 산출물 경로, CPU 대체 실행 보존, GPU 빌드 실패 시 설치 파일 생성 중단, CPU 전용 설정의 오래된 sidecar 제외를 자동 검사 |
| GPU 실제 추론·취소·복구 | Qwen3.5 2B와 4B 모두 통과. llama.cpp/Vulkan 상태, 실제 NVIDIA 장치, 양수 GPU 계층 수, 두 번의 모델 재사용 추론, 요청 중 워커 강제 종료·회수, 재시작·로드·짧은 추론·정상 종료를 확인 |
| 구조화 출력 | 두 모델 모두 고정된 한국어 장학 공지에서 JSON 구조 검사 통과. 분류·날짜 종류 검사는 실패했으며 참고 결과로 구분 |
| 앱 분석 연결 | 2B로 실제 앱 분석 로직의 두 차례 Vulkan 추론을 확인. 원문 날짜 근거 검증이 출력을 거부하여 새 분석 결과·최종 캐시 검사는 완료되지 않음. 네이티브 화면 시험은 아님 |
| 자동 검사 | 화면 192/192, 앱 Rust 320 통과·13 제외, 앱 Clippy 통과. 별도 GPU 기능 빌드는 워커 단위 12개·프로세스 4개 통과, 실제 모델 프로세스 검사 1개는 기본 검사에서 제외 |
| GTX 10, 다른 GTX 16 및 RTX 20·30·40·50 실기기 | 미검증. 현재 PC의 결과를 다른 기종이나 세대의 결과로 표시하지 않음 |

| 실제 GPU 실행 | 2B | 4B |
| --- | --- | --- |
| 실제 GPU 계층 수 | 25 | 31 |
| 최초 모델 로드 | 9.078초 | 17.818초 |
| 구조화 추론 요청 전체 | 56.867초 | 92.243초 |
| 해당 생성 시간 | 56.794초 | 92.035초 |
| 종료 후 재로드 | 6.110초 | 19.927초 |
| 전체 검사 | 111.141초, 통과 | 159.602초, 통과 |

이 값은 다른 빌드가 동시에 실행된 개발 PC에서 관찰한 시간이다. 독립 성능 측정이나 CPU 대비 속도 비교에 사용하지 않는다. GPU 계층 수는 워커가 관찰한 실제 할당과 오프로딩 정보를 확인한 값이다. 취소 검사는 추론 요청 후 500 ms에 소유한 워커를 종료했으며, 그 순간 GPU 계산 커널이 실행 중이었다는 증거는 아니다. 시스템 전체 GPU 메모리 부족, 드라이버 재시작, OS 수준 네트워크 차단 및 설치본 화면에서의 취소는 이 검사에 포함하지 않는다.

기록은 `D:\Codex_Rhythm\.build-tools\validation\gpu-1650-2b-01.json`, `gpu-1650-4b-01.json`, `gpu-app-analysis-2b.json`에 있다. 최초 두 검사에 사용한 GPU 워커의 SHA-256은 `00e285cfd8290a62471a5719ad395fb03819445a8231a4349544d7d2d1f981fc`다. 이후 재빌드한 배포물의 해시와 혼동하지 않는다. 검사 및 설치본 기록은 [Windows 검증](WINDOWS_VALIDATION.md), 모델별 상세 관찰은 [Windows AI 결과](WINDOWS_AI_RESULTS.md)를 따른다.

최종 GPU 워커의 SHA-256은 `debd13f5e18bc13f7197eeaf6ba1ff71b118f799a1a7b896f8ffe871e276d554`다. 이 실행 파일로 2B 모델의 짧은 실제 추론과 정상 종료를 다시 확인했고, GTX 1650의 GPU 계층 25개를 사용했다(`gpu-final-smoke.json`, 전체 19.771초). 최종 GPU 워커 단위 12개·프로세스 4개 검사도 통과했다. 별도 폴더에서 Microsoft 실행 라이브러리 세 파일이 그 폴더에서 실제로 불러와지는 것을 확인했다. 이는 설치본 UI나 다른 PC 검증을 대신하지 않는다.

각 GPU에서 2B 모델부터 로드·한국어 공지 분석·구조화 결과·취소·반복 실행·메모리 회수를 확인하고, 가능하면 4B를 추가한다. GPU 없음, 지원하지 않는 드라이버, VRAM 부족, GPU 워커 누락·종료 및 오프라인 실행도 별도 확인한다. 성능은 전체 분석 시간과 메모리 최고 사용량을 함께 측정한다.

동일 검사를 재현하려면 준비된 GPU 워커와 검증한 로컬 모델을 사용한다. 이 도구는 새 보고서 파일만 생성하며 앱 데이터나 기존 실행 중인 앱에 접근하지 않는다. 보고서 경로는 기존 파일과 겹치지 않게 지정한다.

```powershell
node .\scripts\validate-gpu.mjs --worker 'D:\rhythm-gpu\release\rhythm-ai-worker.exe' --model 'D:\models\Qwen3.5-2B-Q4_K_M.gguf' --report 'D:\validation\gpu-2b.json'
```

## 기술 선택의 근거

기존 고정 mistral.rs CUDA 의존성에는 Pascal보다 새로운 GPU 기능을 사용하는 커널이 있어 단순 CUDA 기능 활성화만으로 GTX 10 지원을 주장하지 않는다. 이번 경로는 Windows와 구형·신형 NVIDIA GPU를 함께 대상으로 하는 Vulkan 워커다. 그래픽 드라이버나 하드웨어 제한을 우회하지 않는다.

참고: [llama.cpp 빌드 문서](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md), [llama-cpp-rs](https://github.com/utilityai/llama-cpp-rs), [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows). 현재 작업에서 고정한 실제 버전은 Cargo 잠금 파일을 기준으로 한다.
