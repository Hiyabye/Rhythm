# Shared read-only verification of the app-local Microsoft redistributable files.
function Get-RhythmVcRuntime([string] $Directory) {
    $items = @()
    foreach ($name in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
        $path = Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required runtime file is missing: $name" }
        $signature = Get-AuthenticodeSignature -LiteralPath $path
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
            throw "Runtime file does not have a valid Microsoft signature: $name"
        }
        $bytes = [IO.File]::ReadAllBytes($path)
        if ($bytes.Length -lt 64 -or $bytes[0] -ne 77 -or $bytes[1] -ne 90) { throw "Invalid runtime PE file: $name" }
        $pe = [BitConverter]::ToInt32($bytes, 60)
        if ($pe -lt 0 -or $pe -gt ($bytes.Length - 6) -or [BitConverter]::ToUInt32($bytes, $pe) -ne 17744 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 34404) {
            throw "Runtime file is not an x64 PE image: $name"
        }
        $item = Get-Item -LiteralPath $path
        $items += [pscustomobject]@{
            name = $name
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            file_version = $item.VersionInfo.FileVersion
            signature_valid = $true
            signer = $signature.SignerCertificate.Subject
        }
    }
    if (@($items.file_version | Select-Object -Unique).Count -ne 1) { throw 'The Microsoft runtime file versions do not match.' }
    return $items
}
