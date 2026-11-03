# Windows 11 x64 빌드 환경

> 출처: 이 문서는 PR #1(Windows 이식)의 기여자가 자신의 Windows 11 기기에서 작성한 기록입니다. 당시 앱 식별자는 `app.rhythm.windows`였고, 현재는 macOS와 같은 `io.github.hiyabye.rhythm`을 씁니다. macOS·Windows 공동 지원으로 병합한 뒤에는 Windows에서 다시 검증하지 않았습니다.

Rhythm은 macOS Apple Silicon과 Windows 11 x64를 지원합니다. 이 문서는 Windows에서 Rust MSVC, CPU 및 NVIDIA GPU 워커, 사용자별 NSIS 설치 파일을 빌드하는 환경을 설명합니다. GPU 경로의 대상 기종·추가 도구·검증 한계는 [Windows GPU 안내](WINDOWS_GPU.md)에, 결과는 [Windows 검증표](WINDOWS_VALIDATION.md)에 있습니다. 빌드 환경을 만든다고 설치 파일과 프로그램 동작이 검증되지는 않습니다.

## 현재 기기에서 확인한 도구

이번 작업 환경에서 Visual Studio Build Tools 설치 프로세스는 종료 코드 0으로 끝났고, `vswhere`에서 설치 완료 및 재부팅 불필요 상태를 확인했습니다. 확인한 버전은 Visual Studio Build Tools **17.14.44** (`vswhere` 설치 버전 `17.14.37827.9`), MSVC **14.44.35207**, Windows SDK **10.0.26100.0**, Rust **1.99.1 x86_64-pc-windows-msvc**입니다. 이는 해당 시점의 환경 정보이며 앱 빌드나 설치 테스트의 성공을 의미하지 않습니다.

프로젝트의 보조 도구 위치는 저장소의 상위 폴더에 있는 `D:\Codex_Rhythm\.build-tools`입니다. 그 아래에 Rust 도구와 캐시, Visual Studio Build Tools 및 Cargo 빌드 산출물이 놓입니다. Windows SDK와 Visual Studio Installer의 공유 구성 요소는 Windows 시스템 드라이브에도 설치될 수 있습니다. 이 폴더는 소스 저장소에 포함하거나 배포하지 않습니다.

## 필요한 환경

- Windows 11 x64, Node.js 22.12 이상과 npm, Microsoft Edge WebView2 Runtime.
- 인터넷 연결, C:에 최소 10 GiB 및 D:에 최소 30 GiB의 사용 가능한 공간. Rust 의존성·로컬 AI 모델·빌드 산출물은 추가 공간을 사용할 수 있습니다.
- Visual Studio Build Tools의 x64 MSVC와 Windows 11 SDK 10.0.26100.0. Visual Studio 설치에는 관리자 권한이 필요합니다.
- Rust 1.94 이상의 `x86_64-pc-windows-msvc` 안정 도구 체인. 잠긴 Windows 의존성 중 mistral.rs가 1.94를 요구하며, 이번 빌드는 1.99.1로 확인했습니다.
- GPU 워커 빌드에는 Vulkan SDK와 libclang이 추가로 필요합니다. 프로젝트와 CI가 사용하는 버전은 Vulkan SDK 1.4.363.0 및 LLVM 20.1.8입니다. 기존 `setup-windows.ps1`은 이 GPU 도구를 설치하지 않으므로 [GPU 빌드 안내](WINDOWS_GPU.md)에 따라 별도로 준비합니다. 사용자 PC의 실행에는 SDK 전체가 필요하지 않습니다.

[Tauri의 Windows 사전 준비](https://v2.tauri.app/start/prerequisites/)도 함께 확인하세요. Node.js나 WebView2가 없다면 먼저 공식 설치 경로에서 준비해야 합니다. 모델 가중치는 도구 설치에 포함되지 않습니다.

## 프로젝트 도구 준비

저장소 루트의 PowerShell에서 다음을 실행합니다.

```powershell
& .\scripts\setup-windows.ps1
```

스크립트는 운영체제·C/D 여유 공간·Node/npm·WebView2를 확인합니다. 완전한 프로젝트 경로의 Build Tools와 Rust가 이미 있으면 버전을 확인하고 다시 설치하지 않습니다. 설치가 불완전하거나 예상과 다르면 파일을 덮어쓰지 않고 중단합니다.

Build Tools가 없다면 관리자 PowerShell에서 다시 실행해야 합니다. 스크립트는 Microsoft의 HTTPS 부트스트래퍼를 내려받아 유효한 Microsoft Authenticode 서명을 확인한 뒤 D:의 프로젝트 도구 폴더에 설치합니다. 설치 프로그램이 재시작을 요구하면 자동으로 재부팅하지 않고 중단합니다. Rust가 없다면 공식 HTTPS 경로의 `rustup-init.exe`와 SHA-256을 받아 검증하고, 프로젝트 전용 `CARGO_HOME`과 `RUSTUP_HOME`에 설치합니다. 설치 파일은 `.build-tools\downloads`에 남습니다. 설치 중 예기치 않은 오류가 나면 기록을 확인하고 기존 폴더를 임의로 지우지 마세요.

설정 스크립트는 영구적인 사용자 또는 시스템 PATH를 수정하지 않습니다. 빌드할 PowerShell 창마다 다음을 실행해 프로젝트 전용 도구와 MSVC 환경을 불러옵니다.

```powershell
. .\scripts\windows-env.ps1
npm ci
npm run check
npm run desktop
```

`windows-env.ps1`은 현재 PowerShell 실행 파일을 `RHYTHM_POWERSHELL`로 전달하므로, PowerShell 7에서 시작한 빌드도 VC 런타임 준비 단계에서 같은 호스트를 사용합니다. 이 변수를 미리 지정했다면 그 선택을 유지합니다. 프로젝트 환경 스크립트 없이 PowerShell 7에서 전역 도구를 사용하는 경우에는 `$env:RHYTHM_POWERSHELL = (Join-Path $PSHOME 'pwsh.exe')`처럼 실행 파일 경로를 지정하세요. 지정하지 않으면 기존 `powershell.exe`를 사용합니다. 선택한 호스트의 실행 정책을 그대로 적용하며, 실패했을 때 정책을 바꾸거나 다른 호스트로 재시도하지 않습니다. VC 런타임 준비가 실패하면 긴 워커 빌드 전에 중단합니다.

릴리스 설치 프로그램을 만들 때:

```powershell
npm run desktop:build
```

`windows-env.ps1`을 쓴 경우 앱·CPU Cargo 산출물은 `D:\Codex_Rhythm\.build-tools\target` 아래에, GPU 워커 산출물은 짧은 별도 경로 `D:\Codex_Rhythm\.g` 아래에 생성됩니다. NSIS 산출물은 앱 target의 `release\bundle\nsis`에서 확인합니다. `CARGO_TARGET_DIR`과 `RHYTHM_GPU_TARGET_DIR`을 따로 지정할 수도 있으며, GPU 경로는 셰이더 생성 도구의 Windows 경로 길이 문제를 피하기 위해 32자 이하로 제한합니다. 배포용 설치 프로그램에는 CPU와 GPU 워커가 모두 필요합니다. 이전 버전이 이미 실행 중이면 완전히 종료한 뒤 새 설치 파일을 수동으로 실행합니다. 제거 프로그램은 사용자 데이터와 모델을 항상 보존하도록 구성되어 있으며, 실제 시험 결과는 검증 기록을 확인해야 합니다.

설치 도구 캐시는 `bundle.useLocalToolsDir` 설정으로 Cargo 산출물 폴더 아래 `.tauri`에 둡니다. 여러 PC에서 사용할 빌드이므로 `target-cpu=native`나 CPU 기능을 강제로 켜는 개발용 환경 설정을 추가하지 마세요. 첫 CPU AI 엔진 빌드는 많은 메모리와 시간이 필요하므로 실제 모델 시험과 동시에 실행하지 않는 편이 좋습니다.

## 복원 검사에 필요한 Windows 기능

복원 검사는 실제 Cloud Files API로 시험 폴더가 동기화 루트 밖인지 확인합니다. 일반 PowerShell에서 프로젝트 환경을 불러온 뒤 다음 명령으로 현재 임시 폴더를 점검할 수 있습니다.

```powershell
. .\scripts\windows-env.ps1
& .\scripts\prepare-windows-restore-ci.ps1 -CheckOnly
```

`-CheckOnly`는 새로 만든 빈 시험 폴더만 조회하고 삭제합니다. 드라이버나 시스템 설정은 바꾸지 않습니다. 정상적인 비동기화 폴더의 결과는 `0x80070186`이며, 다른 오류는 안전한 폴더라는 뜻으로 처리하지 않습니다. 제한된 실행 토큰에서는 접근 거부가 발생할 수 있으므로 오류의 HRESULT와 실행 환경을 함께 확인하세요.

GitHub의 Windows Server 기반 호스팅 러너에서는 같은 검사를 빌드 전에 실행합니다. 필요하면 일회용 러너의 Cloud Files 필터(`CldFlt`)를 활성화·로드하고 시험 드라이브에 연결한 뒤 다시 조회합니다. 최종 조회가 실패하면 즉시 중단하며 복원 검사를 건너뛰지 않습니다. 이 준비 모드는 GitHub 호스팅 러너에서만 허용됩니다. Windows Server CI 결과는 Windows 11 설치본 사용 시험과 구분합니다.

## 설정 스크립트의 출처와 범위

설치 스크립트는 Microsoft의 `https://aka.ms/vs/17/release/vs_BuildTools.exe`와 Rust의 `https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe`를 사용합니다. Rust의 SHA-256은 같은 공식 HTTPS 배포 위치에서 받습니다. Visual Studio의 서명 검사나 Rust의 체크섬이 실패하면 설치하지 않습니다. 인증서 신뢰 설정이나 보안 정책은 변경하지 않습니다.

참고: [Visual Studio 명령줄 설치 옵션](https://learn.microsoft.com/en-us/visualstudio/install/use-command-line-parameters-to-install-visual-studio), [Rustup의 설치 경로 설정](https://rust-lang.github.io/rustup/installation/), [WebView2 설치 확인](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution).
