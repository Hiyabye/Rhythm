# Dot-source before local npm/cargo commands. CI uses its own installed toolchain.
$ErrorActionPreference = 'Stop'
$rhythmTools = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\.build-tools'))
$env:CARGO_HOME = Join-Path $rhythmTools 'cargo'
$env:RUSTUP_HOME = Join-Path $rhythmTools 'rustup'
$env:CARGO_TARGET_DIR = Join-Path $rhythmTools 'target'
$env:npm_config_cache = Join-Path $rhythmTools 'npm-cache'
$env:PLAYWRIGHT_BROWSERS_PATH = Join-Path $rhythmTools 'playwright'
$env:TEMP = Join-Path $rhythmTools 'tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force $env:TEMP | Out-Null
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:PATH = "$($env:CARGO_HOME)\bin;$env:PATH"
$rhythmDevShell = Join-Path $rhythmTools 'vs\Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
if (Test-Path -LiteralPath $rhythmDevShell) {
  Import-Module $rhythmDevShell
  Enter-VsDevShell -VsInstallPath (Join-Path $rhythmTools 'vs') -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
}
