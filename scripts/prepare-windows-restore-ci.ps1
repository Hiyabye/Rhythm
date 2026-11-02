[CmdletBinding()]
param([switch]$CheckOnly)

$ErrorActionPreference = 'Stop'
$notUnderSyncRoot = 0x80070186L
# The application must fail closed when Cloud Files cannot classify its data
# directory. Prepare the disposable hosted runner instead of bypassing that
# guard in tests. -CheckOnly is safe for local verification: it never loads or
# attaches a filter and never changes the environment of later commands.
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required.' }
if (-not $CheckOnly -and ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted')) {
  throw 'Filter preparation is restricted to disposable GitHub-hosted runners; use -CheckOnly locally.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace Rhythm.RestoreCi {
    public static class CloudFiles {
        [DllImport("cldapi.dll", CharSet = CharSet.Unicode)]
        private static extern int CfGetSyncRootInfoByPath(string path, int infoClass,
            IntPtr buffer, uint bufferLength, out uint returnedLength);

        public static uint Query(string path) {
            // CF_SYNC_ROOT_INFO_BASIC contains one LARGE_INTEGER.
            IntPtr buffer = Marshal.AllocHGlobal(8);
            try {
                uint returned;
                return unchecked((uint)CfGetSyncRootInfoByPath(path, 0, buffer, 8, out returned));
            } finally { Marshal.FreeHGlobal(buffer); }
        }
    }
}
'@

$testRoot = if ($CheckOnly) { [IO.Path]::GetTempPath() } else { $env:RUNNER_TEMP }
if (-not $testRoot -or -not [IO.Path]::IsPathFullyQualified($testRoot)) { throw 'An absolute test temporary directory is required.' }
$testRoot = [IO.Path]::GetFullPath($testRoot)
if (-not [IO.Directory]::Exists($testRoot)) { throw "Temporary directory is missing: $testRoot" }
$probe = Join-Path $testRoot ('rhythm-cloud-probe-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($probe) | Out-Null
try {
  $status = [Rhythm.RestoreCi.CloudFiles]::Query($probe)
  Write-Host ('Cloud Files query before preparation: 0x{0:X8} ({1})' -f $status, $testRoot)
  if ($status -eq 0) { throw 'The test temporary directory belongs to a cloud sync root.' }
  if ($status -ne $notUnderSyncRoot -and -not $CheckOnly) {
    # Microsoft documents sc start / fltmc load and explicit volume attachment:
    # https://learn.microsoft.com/windows-hardware/drivers/ifs/loading-and-unloading
    # A loaded filter need not already have an instance on the runner's data disk.
    $sc = Join-Path $env:WINDIR 'System32/sc.exe'
    $fltmc = Join-Path $env:WINDIR 'System32/fltmc.exe'
    & $sc query CldFlt
    if ($LASTEXITCODE -ne 0) { throw 'The runner does not provide the Windows Cloud Files filter.' }
    $startType = Get-ItemPropertyValue -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\CldFlt' -Name Start
    if ($startType -eq 4) {
      & $sc config CldFlt start= demand
      if ($LASTEXITCODE -ne 0) { throw 'Cannot enable the Cloud Files filter on this disposable runner.' }
    }
    & $fltmc load CldFlt
    Write-Host "CldFlt load exit code: $LASTEXITCODE (already loaded is harmless only if the final query succeeds)."
    $volume = [IO.Path]::GetPathRoot($testRoot).TrimEnd('\')
    if ($volume -notmatch '^[A-Za-z]:$') { throw 'The runner temporary directory must use a local drive letter.' }
    & $fltmc attach CldFlt $volume
    Write-Host "CldFlt attach exit code: $LASTEXITCODE (an existing instance is harmless only if the final query succeeds)."
    & $fltmc instances -f CldFlt
    $status = [Rhythm.RestoreCi.CloudFiles]::Query($probe)
    Write-Host ('Cloud Files query after preparation: 0x{0:X8}' -f $status)
  }
  # Accept exactly the same outside-root condition as the application. Neither
  # a failed driver command nor a generic API error establishes a safe root.
  if ($status -ne $notUnderSyncRoot) {
    throw ('Cloud Files cannot validate the restore test directory (HRESULT 0x{0:X8}).' -f $status)
  }
  if (-not $CheckOnly) {
    if (-not $env:GITHUB_ENV) { throw 'GITHUB_ENV is missing.' }
    "TEMP=$testRoot" >> $env:GITHUB_ENV
    "TMP=$testRoot" >> $env:GITHUB_ENV
  }
  Write-Host 'Restore test directory is outside Cloud Files sync roots.'
  # GitHub wraps pwsh steps with exit $LASTEXITCODE. Native-command failures
  # above are superseded only by the successful final API query.
  $global:LASTEXITCODE = 0
} finally {
  # Only our newly created, empty probe is removed; never recurse into a runner
  # or user temporary directory.
  [IO.Directory]::Delete($probe, $false)
}
