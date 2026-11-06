param(
    [Parameter(Mandatory = $true)]
    [string] $InstallDir,
    [string] $ExpectedVersion,
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string] $ExpectedAppSha256,
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string] $ExpectedCpuServerSha256,
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string] $ExpectedGpuServerSha256,
    [string] $ExpectedVcRuntimeDir
)

# Read-only inspection. Hashes are compared when their expected values are
# supplied; a reported hash alone does not prove a file's provenance.
$ErrorActionPreference = 'Stop'
$problems = [System.Collections.Generic.List[string]]::new()
$report = [ordered]@{
    ok = $false
    install_dir = $InstallDir
    expected_version = $ExpectedVersion
    app = $null
    cpu_server = $null
    gpu_server = $null
    vc_runtime = $null
    runtime_hashes_verified = $false
    uninstall = $null
    protocol = $null
    notification_identity = $null
    shortcut = $null
    hashes_verified = $false
    errors = $problems
}

function Complete-Report {
    $report.ok = ($problems.Count -eq 0)
    $report | ConvertTo-Json -Depth 8
    if ($report.ok) { exit 0 }
    exit 1
}

if (-not [System.IO.Path]::IsPathRooted($InstallDir)) {
    $problems.Add('InstallDir must be an absolute path.')
    Complete-Report
}
if (-not (Test-Path -LiteralPath $InstallDir -PathType Container)) {
    $problems.Add('The installation directory does not exist.')
    Complete-Report
}

$resolvedDir = (Resolve-Path -LiteralPath $InstallDir).ProviderPath.TrimEnd('\')
$report.install_dir = $resolvedDir
$appPath = Join-Path $resolvedDir 'rhythm.exe'

function Read-Executable([string] $Path, [string] $ExpectedHash, [string] $Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        $problems.Add("$Label executable is missing.")
        return $null
    }
    try {
        $item = Get-Item -LiteralPath $Path
        $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        $verified = $null
        if ($ExpectedHash) {
            $verified = $hash -ieq $ExpectedHash
            if (-not $verified) { $problems.Add("$Label executable hash does not match.") }
        }
        return [ordered]@{
            path = $item.FullName
            sha256 = $hash
            hash_verified = $verified
            file_version = $item.VersionInfo.FileVersion
            product_version = $item.VersionInfo.ProductVersion
        }
    } catch {
        $problems.Add("Could not inspect $Label executable: $($_.Exception.Message)")
        return $null
    }
}

$report.app = Read-Executable $appPath $ExpectedAppSha256 'Rhythm'
# Both bundled llama.cpp builds ship with every installer.
$report.cpu_server = Read-Executable (Join-Path $resolvedDir 'llama\cpu\llama-server.exe') $ExpectedCpuServerSha256 'CPU llama-server'
$report.gpu_server = Read-Executable (Join-Path $resolvedDir 'llama\vulkan\llama-server.exe') $ExpectedGpuServerSha256 'Vulkan llama-server'
try {
    . (Join-Path $PSScriptRoot 'vc-runtime.ps1')
    $report.vc_runtime = @(Get-RhythmVcRuntime $resolvedDir)
    if ($ExpectedVcRuntimeDir) {
        $expectedRuntime = @(Get-RhythmVcRuntime $ExpectedVcRuntimeDir)
        $allMatch = $true
        foreach ($item in $report.vc_runtime) {
            $expected = $expectedRuntime | Where-Object { $_.name -eq $item.name }
            if ($null -eq $expected -or $expected.sha256 -ine $item.sha256) {
                $allMatch = $false
                $problems.Add("Installed runtime file hash does not match: $($item.name)")
            }
        }
        $report.runtime_hashes_verified = $allMatch
    }
} catch {
    $problems.Add("Could not verify app-local Microsoft runtime: $($_.Exception.Message)")
}
$report.hashes_verified = $null -ne $report.app -and $report.app.hash_verified -eq $true -and
    $null -ne $report.cpu_server -and $report.cpu_server.hash_verified -eq $true -and
    $null -ne $report.gpu_server -and $report.gpu_server.hash_verified -eq $true -and
    $report.runtime_hashes_verified

$registry = $null
$uninstallKey = $null
$protocolKey = $null
$commandKey = $null
$identityKey = $null
try {
    $registry = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    $uninstallKey = $registry.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Uninstall\Rhythm', $false)
    if ($null -eq $uninstallKey) {
        $problems.Add('The current-user uninstall registration is missing.')
    } else {
        $location = [string] $uninstallKey.GetValue('InstallLocation', $null, 'DoNotExpandEnvironmentNames')
        $version = [string] $uninstallKey.GetValue('DisplayVersion', $null, 'DoNotExpandEnvironmentNames')
        $report.uninstall = [ordered]@{ install_location = $location; display_version = $version }
        $expectedLocation = '"' + $resolvedDir + '"'
        if ($location -ine $expectedLocation) {
            $problems.Add('The registered uninstall location does not match the installation directory.')
        }
        if ([string]::IsNullOrWhiteSpace($version)) {
            $problems.Add('The registered display version is missing.')
        } elseif ($ExpectedVersion -and $version -cne $ExpectedVersion) {
            $problems.Add('The registered display version does not match the expected version.')
        }
        if ($null -ne $report.app) {
            $versionToCheck = if ($ExpectedVersion) { $ExpectedVersion } else { $version }
            if ([string]::IsNullOrWhiteSpace($report.app.product_version)) {
                $problems.Add('The executable product version is missing.')
            } elseif ($versionToCheck -and $report.app.product_version -cne $versionToCheck) {
                $problems.Add('The executable product version does not match the expected version.')
            }
            if ([string]::IsNullOrWhiteSpace($report.app.file_version)) {
                $problems.Add('The executable file version is missing.')
            } elseif ($versionToCheck -and $report.app.file_version -cne $versionToCheck) {
                $problems.Add('The executable file version does not match the expected version.')
            }
        }
    }

    $protocolKey = $registry.OpenSubKey('Software\Classes\rhythm-win', $false)
    $commandKey = $registry.OpenSubKey('Software\Classes\rhythm-win\shell\open\command', $false)
    if ($null -eq $protocolKey -or $null -eq $commandKey) {
        $problems.Add('The current-user notification protocol registration is missing.')
    } else {
        $names = $protocolKey.GetValueNames()
        $hasFlag = $names -contains 'URL Protocol'
        $flagKind = if ($hasFlag) { $protocolKey.GetValueKind('URL Protocol').ToString() } else { $null }
        $flag = if ($hasFlag) { $protocolKey.GetValue('URL Protocol', $null, 'DoNotExpandEnvironmentNames') } else { $null }
        $command = [string] $commandKey.GetValue('', $null, 'DoNotExpandEnvironmentNames')
        $expectedCommand = '"' + $appPath + '" "%1"'
        $report.protocol = [ordered]@{ command = $command; url_protocol_present = $hasFlag; url_protocol_kind = $flagKind; url_protocol_value = $flag }
        if (-not $hasFlag -or $flagKind -ne 'String' -or $flag -cne '') {
            $problems.Add('The URL Protocol marker is missing or invalid.')
        }
        if ($command -ine $expectedCommand) {
            $problems.Add('The notification protocol does not target this executable with the expected quoting.')
        }
    }
    $identityKey = $registry.OpenSubKey('Software\Classes\AppUserModelId\io.github.hiyabye.rhythm', $false)
    if ($null -eq $identityKey) {
        $problems.Add('The current-user notification identity is missing.')
    } else {
        $name = [string] $identityKey.GetValue('DisplayName', $null, 'DoNotExpandEnvironmentNames')
        $owner = [string] $identityKey.GetValue('RhythmInstallPath', $null, 'DoNotExpandEnvironmentNames')
        $report.notification_identity = [ordered]@{ display_name = $name; install_path = $owner }
        if ($name -cne 'Rhythm' -or $owner -ine $resolvedDir) {
            $problems.Add('The notification identity does not belong to this installation.')
        }
    }
} catch {
    $problems.Add("Could not inspect current-user registration: $($_.Exception.Message)")
} finally {
    if ($null -ne $commandKey) { $commandKey.Dispose() }
    if ($null -ne $identityKey) { $identityKey.Dispose() }
    if ($null -ne $protocolKey) { $protocolKey.Dispose() }
    if ($null -ne $uninstallKey) { $uninstallKey.Dispose() }
    if ($null -ne $registry) { $registry.Dispose() }
}

try {
    $programs = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
    if ([string]::IsNullOrWhiteSpace($programs)) { throw 'Current-user Programs folder is unavailable.' }
    $shortcutPath = Join-Path $programs 'Rhythm.lnk'
    if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
        $problems.Add('The required current-user Start Menu shortcut is missing.')
    } else {
        $shell = New-Object -ComObject Shell.Application
        $folder = $shell.Namespace($programs)
        if ($null -eq $folder) { throw 'Could not open the Programs shell folder.' }
        $item = $folder.ParseName('Rhythm.lnk')
        if ($null -eq $item -or -not $item.IsLink) { throw 'The Start Menu item is not a Shell shortcut.' }
        $target = [string] $item.GetLink.Path
        $aumid = [string] $item.ExtendedProperty('System.AppUserModel.ID')
        $activator = $item.ExtendedProperty('System.AppUserModel.ToastActivatorCLSID')
        $report.shortcut = [ordered]@{ path = $shortcutPath; target = $target; aumid = $aumid; toast_activator_clsid = [string] $activator }
        if ([string]::IsNullOrWhiteSpace($target) -or [System.IO.Path]::GetFullPath($target) -ine $appPath) {
            $problems.Add('The Start Menu shortcut targets a different executable.')
        }
        if ($aumid -cne 'io.github.hiyabye.rhythm') {
            $problems.Add('The Start Menu shortcut AUMID is missing or incorrect.')
        }
        $parsedActivator = [guid]::Empty
        if (-not [guid]::TryParse([string] $activator, [ref] $parsedActivator) -or
            $parsedActivator -ne [guid]'2860d0e5-a45a-499e-a803-ea87ee435c32') {
            $problems.Add('The Start Menu shortcut toast activator is missing or incorrect.')
        }
    }
} catch {
    $problems.Add("Could not inspect the Start Menu shortcut: $($_.Exception.Message)")
}

Complete-Report
