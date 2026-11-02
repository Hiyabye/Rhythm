# Dot-source before local npm/cargo commands. CI uses its own installed toolchain.
$ErrorActionPreference = 'Stop'
# Child build scripts should use this host and its existing execution policy.
# Keep an explicit caller selection; never retry under another host after a denial.
if (-not $env:RHYTHM_POWERSHELL) {
  $rhythmPowerShellExe = if ($PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
  $env:RHYTHM_POWERSHELL = Join-Path $PSHOME $rhythmPowerShellExe
}
$rhythmTools = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\.build-tools'))
$env:CARGO_HOME = Join-Path $rhythmTools 'cargo'
$env:RUSTUP_HOME = Join-Path $rhythmTools 'rustup'
$env:CARGO_TARGET_DIR = Join-Path $rhythmTools 'target'
$env:RHYTHM_GPU_TARGET_DIR = [IO.Path]::GetFullPath((Join-Path $rhythmTools '..\.g'))
$env:npm_config_cache = Join-Path $rhythmTools 'npm-cache'
$env:PLAYWRIGHT_BROWSERS_PATH = Join-Path $rhythmTools 'playwright'
$env:TEMP = Join-Path $rhythmTools 'tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force $env:TEMP | Out-Null
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:PATH = "$($env:CARGO_HOME)\bin;$env:PATH"
$rhythmVulkan = Join-Path $rhythmTools 'vulkan\1.4.363.0'
if (Test-Path -LiteralPath (Join-Path $rhythmVulkan 'Bin\glslc.exe')) {
  $env:VULKAN_SDK = $rhythmVulkan
  $env:PATH = "$rhythmVulkan\Bin;$env:PATH"
}
$rhythmClang = Join-Path $rhythmTools 'llvm\bin'
if (Test-Path -LiteralPath (Join-Path $rhythmClang 'libclang.dll')) {
  $env:LIBCLANG_PATH = $rhythmClang
}
$env:CMAKE_BUILD_PARALLEL_LEVEL = '2'
$rhythmDevShell = Join-Path $rhythmTools 'vs\Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
if (Test-Path -LiteralPath $rhythmDevShell) {
  Import-Module $rhythmDevShell
  Enter-VsDevShell -VsInstallPath (Join-Path $rhythmTools 'vs') -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
}
