param([string] $RedistDir = $env:VCToolsRedistDir)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'vc-runtime.ps1')
if ([string]::IsNullOrWhiteSpace($RedistDir)) {
    throw 'Run the Visual Studio x64 development environment before staging Microsoft runtime files.'
}
# The CRT folder name follows the toolset (VC143 for VS 2022, newer for later
# Visual Studio releases); take the single newest x64 one.
$crt = @(Get-ChildItem -LiteralPath (Join-Path $RedistDir 'x64') -Directory -Filter 'Microsoft.VC*.CRT' |
    Sort-Object { [int]($_.Name -replace '^Microsoft\.VC(\d+)\.CRT$', '$1') } -Descending)
if ($crt.Count -eq 0) { throw "No Microsoft.VC*.CRT folder under $RedistDir\x64." }
$source = $crt[0].FullName
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
