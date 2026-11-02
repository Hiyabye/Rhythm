param([string] $RedistDir = $env:VCToolsRedistDir)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'vc-runtime.ps1')
if ([string]::IsNullOrWhiteSpace($RedistDir)) {
    throw 'Run the Visual Studio x64 development environment before staging Microsoft runtime files.'
}
$source = Join-Path $RedistDir 'x64\Microsoft.VC143.CRT'
$files = @(Get-RhythmVcRuntime $source)
$destination = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\src-tauri\binaries\vc-runtime'))
New-Item -ItemType Directory -Force $destination | Out-Null
foreach ($item in $files) {
    Copy-Item -LiteralPath (Join-Path $source $item.name) -Destination (Join-Path $destination $item.name) -Force
}
$staged = @(Get-RhythmVcRuntime $destination)
for ($i = 0; $i -lt $files.Count; $i++) {
    if ($files[$i].sha256 -ne $staged[$i].sha256) { throw 'A staged Microsoft runtime hash differs from its source.' }
}
$staged | ConvertTo-Json -Depth 4
