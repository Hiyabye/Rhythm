# Explicit, opt-in Windows CPU validation. Model downloads total several GB.
# Reports from an archive contain source text and are kept locally; never upload them.
param(
    [string]$ArchivePath = 'D:\Codex_Rhythm\.build-tools\validation\자료 보관\rhythm.sqlite3',
    [string]$ArchiveIds = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'windows-env.ps1')
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$validationRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot '..\.build-tools\ai-validation'))
$runId = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), ([guid]::NewGuid().ToString('N').Substring(0, 8))
$runRoot = Join-Path $validationRoot (Join-Path 'runs' $runId)
$modelRoots = Join-Path $validationRoot 'models-by-id'
New-Item -ItemType Directory -Force -Path $runRoot, $modelRoots | Out-Null
$mainLog = Join-Path $runRoot 'validation.log'
$summaryPath = Join-Path $runRoot 'summary.json'
$results = [ordered]@{ started_at = (Get-Date).ToString('o'); run_directory = $runRoot; backend = 'cpu'; models = @(); status = 'failed' }

function Write-Log([string]$Message) {
    $line = '{0} {1}' -f (Get-Date -Format o), $Message
    Write-Host $line
    Add-Content -LiteralPath $mainLog -Value $line -Encoding utf8
}
function Invoke-Cargo([string]$Stage, [string[]]$Arguments, [string]$ExpectedTest = '') {
    $logPath = Join-Path $runRoot "$Stage.log"
    Write-Log "START $Stage (details: $logPath)"
    # Capture both Cargo diagnostics and test output locally. Native stderr is
    # expected during compilation and is not itself interpreted as failure.
    $oldPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $PSNativeCommandUseErrorActionPreference = $false
        & cargo @Arguments 2>&1 | Tee-Object -FilePath $logPath
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $oldPreference }
    if ($code -ne 0) { throw "$Stage failed with exit code $code; see $logPath" }
    if ($ExpectedTest) {
        $output = Get-Content -LiteralPath $logPath -Raw
        # --nocapture can interleave model logs between the test name and `ok`.
        # Require the named test and its one-test passing summary, not one line.
        if ($output -notmatch '(?m)^running 1 test\s*$' -or
            $output -notmatch [regex]::Escape($ExpectedTest) -or
            $output -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored') {
            throw "$Stage did not prove that the requested test ran and passed; see $logPath"
        }
    }
    Write-Log "PASS $Stage"
}
function Check-Model([string]$Path, $Entry) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Model file missing: $Path" }
    $item = Get-Item -LiteralPath $Path
    if ($item.Length -ne [long]$Entry.bytes) { throw "Model size mismatch: $Path" }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Entry.sha256) { throw "Model SHA-256 mismatch: $Path" }
    Write-Log "PASS model size and SHA-256: $($Entry.id)"
}
function Read-Report([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Expected report missing: $Path" }
    return Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json
}
function Test-ReportTrue($Value) {
    return $Value -is [bool] -and $Value
}

try {
    Set-Location -LiteralPath $projectRoot
    if ($env:RUSTFLAGS -match 'target-cpu|target-feature' -or
        $env:CARGO_ENCODED_RUSTFLAGS -match 'target-cpu|target-feature' -or
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS -match 'target-cpu|target-feature') {
        throw 'CPU-specific or target-feature Rust flags are set. Remove them before a portable validation build.'
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [Runtime.InteropServices.Architecture]::X64) {
        throw 'Validation requires native Windows x64.'
    }
    if ($ArchiveIds) {
        if ($ArchiveIds -notmatch '^\d+(,\d+){0,9}$') { throw 'ArchiveIds must contain 1–10 comma-separated positive document IDs.' }
        $ids = @($ArchiveIds.Split(',') | ForEach-Object { [long]$_ })
        if (@($ids | Where-Object { $_ -le 0 }).Count -or @($ids | Select-Object -Unique).Count -ne $ids.Count) {
            throw 'ArchiveIds must be distinct positive integers.'
        }
        if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) { throw "Archive not found: $ArchivePath" }
    }
    $env:RHYTHM_AI_BACKEND = 'cpu'
    $env:CARGO_TERM_COLOR = 'never'
    Remove-Item Env:RHYTHM_AI_EVAL_TRACE -ErrorAction SilentlyContinue
    Write-Log 'Building the release CPU worker and application test binary; no GPU features are selected.'
    Invoke-Cargo 'build-worker' @('build', '--release', '--locked', '-p', 'rhythm-ai-worker')
    $builtWorker = Join-Path $env:CARGO_TARGET_DIR 'release\rhythm-ai-worker.exe'
    if (-not (Test-Path -LiteralPath $builtWorker -PathType Leaf)) { throw "Release worker missing: $builtWorker" }
    $workerCopy = Join-Path $runRoot 'rhythm-ai-worker-cpu.exe'
    Copy-Item -LiteralPath $builtWorker -Destination $workerCopy
    $sidecarDir = Join-Path $projectRoot 'src-tauri\binaries'
    New-Item -ItemType Directory -Force -Path $sidecarDir | Out-Null
    Copy-Item -LiteralPath $builtWorker -Destination (Join-Path $sidecarDir 'rhythm-ai-worker-x86_64-pc-windows-msvc.exe') -Force
    $env:RHYTHM_WORKER_TEST_BIN = $workerCopy
    Invoke-Cargo 'build-app-tests' @('test', '--release', '--locked', '-p', 'rhythm', '--no-run')

    $catalog = @(Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri\src\intelligence\catalog.json') -Raw -Encoding utf8 | ConvertFrom-Json)
    foreach ($id in @('qwen35-2b', 'qwen35-4b')) {
        $result = [ordered]@{ id = $id; install = 'not_run'; inference = 'not_run'; synthetic = 'not_run'; recovery = 'not_run'; archive = 'not_run'; error = $null }
        $results.models += $result
        try {
            $matches = @($catalog | Where-Object { $_.id -eq $id })
            if ($matches.Count -ne 1 -or $matches[0].sha256 -notmatch '^[0-9a-f]{64}$') { throw "Invalid pinned model entry: $id" }
            $entry = $matches[0]
            $root = Join-Path $modelRoots $id
            New-Item -ItemType Directory -Force -Path $root | Out-Null
            $file = Join-Path $root (Join-Path 'models' ($entry.sha256 + '.gguf'))
            if (Test-Path -LiteralPath $file) {
                Check-Model $file $entry
                $result.install = 'reused_verified'
            } else {
                $env:RHYTHM_MODEL_INSTALL_DIR = $root
                $env:RHYTHM_MODEL_INSTALL_ID = $id
                Invoke-Cargo "$id-install" @('test', '--release', '--locked', '-p', 'rhythm', 'intelligence::models::tests::real_curated_model_install', '--', '--ignored', '--nocapture') 'real_curated_model_install'
                Check-Model $file $entry
                $result.install = 'downloaded_verified'
            }
            try {
                $result.inference = 'failed'
                $env:RHYTHM_TEST_MODEL = $file
                Invoke-Cargo "$id-inference" @('test', '--release', '--locked', '-p', 'rhythm-ai-worker', '--test', 'process', 'actual_local_inference', '--', '--ignored', '--nocapture') 'actual_local_inference'
                $result.inference = 'passed'
            } catch {
                $result.error = "inference: $($_.Exception.Message)"
                Write-Log "FAIL $id inference: $($_.Exception.Message)"
            }

            $env:RHYTHM_MODEL_EVAL_ROOT = $root
            $env:RHYTHM_MODEL_EVAL_ID = $id
            $env:RHYTHM_AI_EVAL_CASES = 'scholarship'
            $env:RHYTHM_AI_EVAL_LIMIT = '1'
            $env:RHYTHM_AI_EVAL_BACKEND = 'cpu'
            $env:RHYTHM_AI_EVAL_DECODING = 'analysis_json_v1'
            $env:RHYTHM_AI_EVAL_RECOVERY = '1'
            $env:RHYTHM_AI_EVAL_REPORT = Join-Path $runRoot "$id-synthetic.json"
            $result.synthetic = 'failed'
            $result.recovery = 'failed'
            $report = $null
            try {
                Invoke-Cargo "$id-synthetic" @('test', '--release', '--locked', '-p', 'rhythm', 'intelligence::evaluation::actual_qwen35_analysis_evaluation', '--', '--ignored', '--nocapture') 'actual_qwen35_analysis_evaluation'
            } catch {
                # A quality failure makes the test exit nonzero even if recovery
                # passed. Retain that failure and inspect the report separately.
                $result.error = "$(if ($result.error) { $result.error + '; ' })synthetic test: $($_.Exception.Message)"
                Write-Log "FAIL $id synthetic test: $($_.Exception.Message)"
            }
            try {
                $report = Read-Report $env:RHYTHM_AI_EVAL_REPORT
                if ($null -eq $report) { throw 'synthetic report is empty' }
            } catch {
                $result.error = "$(if ($result.error) { $result.error + '; ' })synthetic report: $($_.Exception.Message)"
                Write-Log "FAIL $id synthetic report: $($_.Exception.Message)"
            }
            if ($null -ne $report) {
                try {
                    if ($report.quality.status -ne 'passed' -or @($report.quality.failures).Count -ne 0 -or
                        @($report.samples).Count -ne 1 -or $report.samples[0].case -ne 'scholarship') {
                        throw 'quality report did not show one passing scholarship sample'
                    }
                    $sample = $report.samples[0]
                    if ($sample.job.status -ne 'completed' -or -not (Test-ReportTrue $sample.assessment.schema_valid) -or
                        -not (Test-ReportTrue $sample.assessment.literal_grounding_valid) -or -not (Test-ReportTrue $sample.assessment.category_correct) -or
                        -not (Test-ReportTrue $sample.assessment.deadline_correct) -or -not (Test-ReportTrue $sample.assessment.partial_disclosure_correct) -or
                        @($sample.assessment.missing_summary_terms).Count -ne 0) {
                        throw 'synthetic assessment did not satisfy all quality checks'
                    }
                    $result.synthetic = 'passed'
                    Write-Log "PASS $id synthetic quality"
                } catch {
                    $result.error = "$(if ($result.error) { $result.error + '; ' })synthetic quality: $($_.Exception.Message)"
                    Write-Log "FAIL $id synthetic quality: $($_.Exception.Message)"
                }
                try {
                    if ($report.recovery.status -eq 'skipped' -or $report.recovery.status -eq 'not_requested') {
                        $result.recovery = 'skipped'
                        throw "recovery was $($report.recovery.status)"
                    }
                    if ($report.recovery.status -ne 'passed' -or -not (Test-ReportTrue $report.recovery.recovered) -or
                        $report.recovery.cancelled_status -ne 'cancelled' -or -not (Test-ReportTrue $report.recovery.prior_result_preserved) -or
                        -not (Test-ReportTrue $report.recovery.child_killed) -or $report.recovery.crashed_status -ne 'failed' -or
                        $report.recovery.recovery_status -ne 'completed') {
                        throw 'cancellation/crash recovery was not fully verified'
                    }
                    $result.recovery = 'passed'
                    Write-Log "PASS $id cancellation/crash recovery"
                } catch {
                    $result.error = "$(if ($result.error) { $result.error + '; ' })recovery: $($_.Exception.Message)"
                    Write-Log "FAIL $id recovery: $($_.Exception.Message)"
                }
            }

            if ($ArchiveIds) {
                try {
                    $result.archive = 'failed'
                    $env:RHYTHM_AI_EVAL_ARCHIVE = (Resolve-Path -LiteralPath $ArchivePath).Path
                    $env:RHYTHM_AI_ARCHIVE_IDS = $ArchiveIds
                    $env:RHYTHM_AI_EVAL_REPORT = Join-Path $runRoot "$id-archived.json"
                    Invoke-Cargo "$id-archived" @('test', '--release', '--locked', '-p', 'rhythm', 'intelligence::archived_evaluation::actual_archived_notice_evaluation', '--', '--ignored', '--nocapture') 'actual_archived_notice_evaluation'
                    $archived = Read-Report $env:RHYTHM_AI_EVAL_REPORT
                    if (@($archived.samples).Count -ne $ids.Count -or @($archived.failures).Count -ne 0) { throw "$id archived evaluation did not complete all requested samples" }
                    $result.archive = 'passed_requires_human_review'
                } catch {
                    $result.error = "$(if ($result.error) { $result.error + '; ' })archive: $($_.Exception.Message)"
                    Write-Log "FAIL $id archive: $($_.Exception.Message)"
                }
            } else {
                $result.archive = 'skipped_no_explicit_archive_ids'
                Write-Log "SKIP $id archived evaluation: no explicit archive document IDs; real-source analysis is unverified."
            }
        } catch {
            $result.error = $_.Exception.Message
            Write-Log "FAIL $id`: $($result.error)"
        }
    }
    if (@($results.models | Where-Object { $_.error }).Count -eq 0 -and $results.models.Count -eq 2) {
        $results.status = if (@($results.models | Where-Object { $_.archive -like 'skipped*' }).Count) { 'partial_archive_skipped' } else { 'automated_checks_passed_human_review_required' }
    }
} catch {
    $results['error'] = $_.Exception.Message
    Write-Log "FAIL validation setup: $($_.Exception.Message)"
} finally {
    $results['finished_at'] = (Get-Date).ToString('o')
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8
    Write-Log "Summary: $summaryPath"
    Write-Log 'Reports and logs are local. Synthetic checks do not prove semantic accuracy or native UI delivery.'
}
if ($results.status -eq 'failed') { exit 1 }
if ($results.status -eq 'partial_archive_skipped') { exit 2 }
