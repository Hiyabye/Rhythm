# Prepare the project-local Rust and VS Build Tools on Windows 11 x64.
# Invoke with `& .\scripts\setup-windows.ps1`; this does not persist PATH.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$toolsRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot '..\.build-tools'))
$vsRoot = Join-Path $toolsRoot 'vs'
$cargoRoot = Join-Path $toolsRoot 'cargo'
$rustupRoot = Join-Path $toolsRoot 'rustup'
$downloads = Join-Path $toolsRoot 'downloads'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'

function Assert-Prerequisites {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
        [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [Runtime.InteropServices.Architecture]::X64) {
        throw 'Windows x64 환경이 필요합니다.'
    }
    $build = (Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -Name CurrentBuildNumber).CurrentBuildNumber
    if ([int]$build -lt 22000) { throw "Windows 11이 필요합니다. 현재 빌드: $build" }
    if ([IO.Path]::GetPathRoot($toolsRoot) -ne 'D:\') {
        throw "이 설정은 D:에 있는 저장소와 인접 .build-tools 폴더를 사용합니다: $toolsRoot"
    }
    foreach ($requirement in @(@('C', 10), @('D', 30))) {
        $drive = [IO.DriveInfo]::new([string]$requirement[0])
        if (-not $drive.IsReady -or $drive.AvailableFreeSpace -lt ([int64]$requirement[1] * 1GB)) {
            throw "$($requirement[0]): 드라이브에 최소 $($requirement[1]) GiB의 여유 공간이 필요합니다."
        }
    }
    $node = Get-Command node.exe -ErrorAction SilentlyContinue
    $npm = Get-Command npm.cmd -ErrorAction SilentlyContinue
    if (-not $node -or -not $npm) { throw 'Node.js 22.12 이상과 npm을 먼저 설치해 주세요.' }
    $nodeText = (& $node.Source --version).Trim().TrimStart('v')
    try { $nodeVersion = [Version]$nodeText } catch { throw "Node.js 버전을 확인할 수 없습니다: $nodeText" }
    if ($nodeVersion -lt [Version]'22.12.0') { throw "Node.js 22.12 이상이 필요합니다: $nodeText" }
    $runtimeKeys = @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    )
    $webviewFound = $false
    foreach ($key in $runtimeKeys) {
        $entry = Get-ItemProperty -LiteralPath $key -Name pv -ErrorAction SilentlyContinue
        if ($null -ne $entry -and $entry.pv -and $entry.pv -ne '0.0.0.0') { $webviewFound = $true }
    }
    if (-not $webviewFound) { throw 'Microsoft Edge WebView2 Runtime을 먼저 설치해 주세요.' }
    Write-Host "Node.js $nodeText, WebView2 및 디스크 여유 공간 확인 완료."
}

function Find-LocalBuildTools {
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { return $null }
    $json = & $vswhere -all -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json -utf8
    if ($LASTEXITCODE -ne 0) { throw 'Visual Studio 설치 상태를 확인하지 못했습니다.' }
    $instances = @(($json -join "`n") | ConvertFrom-Json)
    $matching = @($instances | Where-Object {
        $null -ne $_ -and $_.installationPath -and [IO.Path]::GetFullPath($_.installationPath).TrimEnd('\') -ieq $vsRoot.TrimEnd('\')
    })
    if ($matching.Count -gt 1) { throw '프로젝트 Build Tools 설치 정보가 중복되어 수동 확인이 필요합니다.' }
    if ($matching.Count -eq 0) { return $null }
    $instance = $matching[0]
    if (-not $instance.isComplete -or $instance.isRebootRequired) {
        throw 'Build Tools 설치가 미완료이거나 다시 시작이 필요합니다. 수동으로 상태를 확인해 주세요.'
    }
    $compilers = @(Get-ChildItem -LiteralPath (Join-Path $vsRoot 'VC\Tools\MSVC') -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin\Hostx64\x64\cl.exe') -PathType Leaf })
    if ($compilers.Count -eq 0) { throw 'x64 MSVC 컴파일러를 찾지 못했습니다. Build Tools 구성을 확인해 주세요.' }
    $sdkInclude = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Include\10.0.26100.0\um\Windows.h'
    if (-not (Test-Path -LiteralPath $sdkInclude -PathType Leaf)) { throw 'Windows 11 SDK 10.0.26100.0을 찾지 못했습니다.' }
    return $instance
}

function Assert-MicrosoftSignature([string]$path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or
        $signature.SignerCertificate.Subject -notmatch '(^|,\s*)CN=Microsoft Corporation(,|$)') {
        throw 'Visual Studio 설치 파일의 유효한 Microsoft 서명을 확인하지 못했습니다.'
    }
}

function Install-LocalBuildTools {
    if (Test-Path -LiteralPath $vsRoot) {
        throw "기존 Build Tools 폴더가 있지만 완전한 설치로 확인되지 않았습니다. 자동으로 덮어쓰지 않습니다: $vsRoot"
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Visual Studio Build Tools 설치에는 관리자가 필요합니다. 관리자 PowerShell에서 이 스크립트를 다시 실행해 주세요.'
    }
    $installer = Join-Path $downloads ("vs-buildtools-{0}.exe" -f [Guid]::NewGuid().ToString('N'))
    Invoke-WebRequest -Uri 'https://aka.ms/vs/17/release/vs_BuildTools.exe' -OutFile $installer
    Assert-MicrosoftSignature $installer
    $arguments = @('--quiet', '--wait', '--norestart', '--nocache', '--installPath', ('"{0}"' -f $vsRoot),
        '--add', 'Microsoft.VisualStudio.Workload.VCTools',
        '--add', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
        '--add', 'Microsoft.VisualStudio.Component.VC.CMake.Project',
        '--add', 'Microsoft.VisualStudio.Component.Windows11SDK.26100')
    $process = Start-Process -FilePath $installer -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -eq 3010) { throw 'Build Tools 설치에 다시 시작이 필요합니다. 자동 재부팅하지 않았습니다.' }
    if ($process.ExitCode -ne 0) { throw "Build Tools 설치 실패 (종료 코드 $($process.ExitCode))." }
    if (-not (Find-LocalBuildTools)) { throw '설치 후 Build Tools 상태를 확인하지 못했습니다.' }
}

function Assert-LocalRust {
    $rustc = Join-Path $cargoRoot 'bin\rustc.exe'
    $rustup = Join-Path $cargoRoot 'bin\rustup.exe'
    $cargo = Join-Path $cargoRoot 'bin\cargo.exe'
    if (-not (Test-Path -LiteralPath $rustc -PathType Leaf) -or -not (Test-Path -LiteralPath $rustup -PathType Leaf) -or
        -not (Test-Path -LiteralPath $cargo -PathType Leaf)) { return $false }
    $details = & $rustc -vV
    if ($LASTEXITCODE -ne 0) { throw '기존 프로젝트 Rust 도구 체인을 실행할 수 없습니다. 덮어쓰지 않았습니다.' }
    $hostLine = @($details | Where-Object { $_ -match '^host:' })
    $versionLine = @($details | Where-Object { $_ -match '^release:' })
    if ($hostLine.Count -ne 1 -or $hostLine[0] -ne 'host: x86_64-pc-windows-msvc' -or $versionLine.Count -ne 1) {
        throw '프로젝트 Rust 도구 체인의 호스트 또는 버전을 확인할 수 없습니다.'
    }
    $versionText = $versionLine[0].Substring('release: '.Length)
    try { $version = [Version]$versionText } catch { throw "안정 Rust 버전이 아닙니다: $versionText" }
    if ($version -lt [Version]'1.94.0') { throw "Rust 1.94 이상이 필요합니다: $versionText" }
    Write-Host "프로젝트 Rust $versionText x86_64-pc-windows-msvc 확인 완료."
    return $true
}

function Install-LocalRust {
    if ((Test-Path -LiteralPath $cargoRoot) -or (Test-Path -LiteralPath $rustupRoot)) {
        throw '프로젝트 Rust 폴더가 이미 존재하지만 정상 설치로 확인되지 않았습니다. 자동으로 덮어쓰지 않습니다.'
    }
    $base = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
    $installer = Join-Path $downloads ("rustup-init-{0}.exe" -f [Guid]::NewGuid().ToString('N'))
    $checksum = "$installer.sha256"
    Invoke-WebRequest -Uri $base -OutFile $installer
    Invoke-WebRequest -Uri "$base.sha256" -OutFile $checksum
    $rawChecksum = (Get-Content -LiteralPath $checksum -Raw).Trim()
    if ($rawChecksum -notmatch '^([0-9a-fA-F]{64})(?:\s|$)') { throw '공식 Rust 설치 파일의 체크섬 형식이 올바르지 않습니다.' }
    $expected = $Matches[1]
    if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ine $expected) { throw 'Rust 설치 파일 SHA-256이 일치하지 않습니다.' }
    $arguments = @('-y', '--no-modify-path', '--default-host', 'x86_64-pc-windows-msvc', '--profile', 'default', '--default-toolchain', 'stable')
    $process = Start-Process -FilePath $installer -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "Rust 설치 실패 (종료 코드 $($process.ExitCode))." }
    if (-not (Assert-LocalRust)) { throw '설치 후 Rust 도구 체인을 확인하지 못했습니다.' }
}

Assert-Prerequisites
New-Item -ItemType Directory -Path $downloads -Force | Out-Null
$savedCargo = $env:CARGO_HOME
$savedRustup = $env:RUSTUP_HOME
try {
    $env:CARGO_HOME = $cargoRoot
    $env:RUSTUP_HOME = $rustupRoot
    $instance = Find-LocalBuildTools
    if ($instance) { Write-Host "기존 Build Tools $($instance.installationVersion) 확인 완료." }
    else { Install-LocalBuildTools }
    if (-not (Assert-LocalRust)) { Install-LocalRust }
    Write-Host '준비가 끝났습니다. 현재 터미널에서 . .\scripts\windows-env.ps1 을 실행해 빌드 환경을 불러오세요.'
} finally {
    $env:CARGO_HOME = $savedCargo
    $env:RUSTUP_HOME = $savedRustup
}
