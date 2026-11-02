param(
    [Parameter(Mandatory = $true)] [string] $Installer,
    [Parameter(Mandatory = $true)] [string] $OutputDir,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{64}$')] [string] $ExpectedInstallerSha256
)

# Run only after the final build and source edits are complete. The new output
# directory is never overwritten, and model weights/build caches are excluded.
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 or later is required.' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$workspace = [IO.Directory]::GetParent($repo).FullName.TrimEnd('\')
if (-not [IO.Path]::IsPathFullyQualified($Installer) -or -not [IO.Path]::IsPathFullyQualified($OutputDir)) {
    throw 'Installer and OutputDir must be absolute paths.'
}
$installerPath = [IO.Path]::GetFullPath($Installer)
$outputPath = [IO.Path]::GetFullPath($OutputDir).TrimEnd('\')
if (-not $installerPath.StartsWith($workspace + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The installer must be inside the Rhythm workspace.'
}
if ([IO.Directory]::GetParent($outputPath).FullName -ine $workspace) {
    throw 'OutputDir must be a new direct child of the Rhythm workspace.'
}
if ([IO.File]::Exists($outputPath) -or [IO.Directory]::Exists($outputPath)) {
    throw 'OutputDir already exists; choose a new directory.'
}
if (-not [IO.File]::Exists($installerPath) -or [IO.Path]::GetExtension($installerPath) -ine '.exe') {
    throw 'The specified installer executable does not exist.'
}
if ((Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash -ine $ExpectedInstallerSha256) {
    throw 'The installer SHA-256 does not match the explicitly supplied hash.'
}

# NUL-separated Git output retains spaces and non-ASCII names. Git includes
# tracked changes and untracked non-ignored files; deleted tracked files are
# omitted below. Neither .git nor ignored caches are enumerated.
$git = (Get-Command git -ErrorAction Stop).Source
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $git
$start.WorkingDirectory = $repo
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
foreach ($arg in @('-c', 'core.quotepath=false', 'ls-files', '--cached', '--others', '--exclude-standard', '-z')) {
    [void] $start.ArgumentList.Add($arg)
}
$process = [Diagnostics.Process]::Start($start)
$buffer = [IO.MemoryStream]::new()
$process.StandardOutput.BaseStream.CopyTo($buffer)
$gitError = $process.StandardError.ReadToEnd()
$process.WaitForExit()
if ($process.ExitCode -ne 0) { throw "Could not list repository files: $gitError" }
$names = [Text.Encoding]::UTF8.GetString($buffer.ToArray()).Split([char]0, [StringSplitOptions]::RemoveEmptyEntries)
$buffer.Dispose()
$process.Dispose()

$allowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($ext in @('.css','.html','.icns','.ico','.json','.lock','.md','.mjs','.nsh','.nsi','.pem','.png','.ps1','.rs','.sql','.svelte','.svg','.toml','.ts','.txt','.yml','.yaml','.gitignore')) {
    [void] $allowed.Add($ext)
}
$selected = [Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
foreach ($name in $names) {
    if ($name -match '(^|/)\.git(/|$)' -or $name -match '(^|/)\.\.?(/|$)' -or $name.Contains('\') -or $name.Contains(':')) {
        throw 'Unexpected Git path; refusing to package.'
    }
    $path = [IO.Path]::GetFullPath((Join-Path $repo ($name.Replace('/', '\'))))
    if (-not $path.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'A source path escaped the repository.' }
    if (-not [IO.File]::Exists($path)) { continue }
    $entry = Get-Item -LiteralPath $path -Force
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Source file is a link: $name" }
    $parent = $entry.Directory
    while ($parent.FullName -ine $repo) {
        if (($parent.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Source directory is a link: $name" }
        $parent = $parent.Parent
        if ($null -eq $parent) { throw 'Unexpected source directory.' }
    }
    if ($entry.Length -gt 10MB) { throw "Source file exceeds 10 MiB: $name" }
    $ext = [IO.Path]::GetExtension($name)
    if ($name -ne 'LICENSE' -and -not $allowed.Contains($ext)) {
        throw "Unexpected or binary file in source selection: $name"
    }
    if ($ext -ieq '.pem' -and [IO.File]::ReadAllText($path).Contains('PRIVATE KEY')) {
        throw "A private key was found in the source selection: $name"
    }
    [void] $selected.Add($name)
}
$documents = @('WINDOWS_USAGE.md', 'WINDOWS_BUILD.md', 'WINDOWS_VALIDATION.md', 'WINDOWS_AI_RESULTS.md')
$requiredFiles = @('LICENSE','Cargo.lock','package-lock.json','src-tauri/tauri.conf.json','src-tauri/icons/icon.ico','assets/licenses/Tauri-template-MIT.txt','assets/licenses/Pretendard-OFL.txt')
$requiredFiles += $documents | ForEach-Object { 'docs/' + $_ }
foreach ($required in $requiredFiles) {
    if (-not $selected.Contains($required)) { throw "Required source file is missing: $required" }
}

Add-Type -AssemblyName System.IO.Compression
[void] [IO.Directory]::CreateDirectory($outputPath)
$installerName = [IO.Path]::GetFileName($installerPath)
$installedCopy = Join-Path $outputPath $installerName
[IO.File]::Copy($installerPath, $installedCopy, $false)
if ((Get-FileHash -LiteralPath $installedCopy -Algorithm SHA256).Hash -ine $ExpectedInstallerSha256) {
    throw 'The copied installer does not match the expected SHA-256.'
}
$sourceName = 'Rhythm-Windows-source.zip'
$sourcePath = Join-Path $outputPath $sourceName
$file = [IO.File]::Open($sourcePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
$zip = [IO.Compression.ZipArchive]::new($file, [IO.Compression.ZipArchiveMode]::Create, $false)
try {
    foreach ($name in $selected) {
        $entry = $zip.CreateEntry('Rhythm-source/' + $name, [IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
        $input = [IO.File]::OpenRead((Join-Path $repo ($name.Replace('/', '\'))))
        $output = $entry.Open()
        try { $input.CopyTo($output) } finally { $output.Dispose(); $input.Dispose() }
    }
} finally { $zip.Dispose(); $file.Dispose() }
# Verify each archived file against the same selected source path, not merely
# the ZIP structure, and fail if a source changed while packaging.
$checkFile = [IO.File]::OpenRead($sourcePath)
$checkZip = [IO.Compression.ZipArchive]::new($checkFile, [IO.Compression.ZipArchiveMode]::Read, $false)
try {
    if ($checkZip.Entries.Count -ne $selected.Count) { throw 'Source archive entry count differs.' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        foreach ($name in $selected) {
            $entry = $checkZip.GetEntry('Rhythm-source/' + $name)
            if ($null -eq $entry) { throw "Source archive entry is missing: $name" }
            $stream = $entry.Open()
            try { $archiveHash = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') } finally { $stream.Dispose() }
            $sourceHash = (Get-FileHash -LiteralPath (Join-Path $repo ($name.Replace('/', '\'))) -Algorithm SHA256).Hash
            if ($archiveHash -ine $sourceHash) { throw "Source archive entry differs: $name" }
        }
    } finally { $sha.Dispose() }
} finally { $checkZip.Dispose(); $checkFile.Dispose() }
foreach ($name in $documents) {
    $sourceDocument = Join-Path $repo (Join-Path 'docs' $name)
    $copiedDocument = Join-Path $outputPath $name
    [IO.File]::Copy($sourceDocument, $copiedDocument, $false)
    if ((Get-FileHash -LiteralPath $sourceDocument -Algorithm SHA256).Hash -ne
        (Get-FileHash -LiteralPath $copiedDocument -Algorithm SHA256).Hash) {
        throw "Copied document does not match: $name"
    }
}
$summary = foreach ($name in (@($installerName, $sourceName) + $documents)) {
    '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $outputPath $name) -Algorithm SHA256).Hash.ToLowerInvariant(), $name
}
[IO.File]::WriteAllLines((Join-Path $outputPath 'SHA256SUMS.txt'), $summary, [Text.UTF8Encoding]::new($false))
Write-Output "Created $outputPath with $($selected.Count) source files."
