param([switch]$ImportValidationKey)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$appRoot = Join-Path $projectRoot 'outputs\desktop\Local Dictation'
$executable = Join-Path $appRoot 'local-dictation.exe'
$report = Join-Path $projectRoot 'outputs\desktop-startup-check.json'
$stdout = Join-Path $projectRoot 'outputs\desktop-startup-check.stdout'
$stderr = Join-Path $projectRoot 'outputs\desktop-startup-check.stderr'
Set-Content -LiteralPath $report -Value '{"pending":true}' -Encoding utf8
$arguments = @('--cleanup-only','--maintenance-report', ('"' + $report + '"'))
if ($ImportValidationKey) {
    $arguments += @('--import-key-file', ('"' + (Join-Path $projectRoot '.env.local') + '"'))
}
$process = Start-Process -FilePath $executable -ArgumentList $arguments -WorkingDirectory $appRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru -Wait
if ($process.ExitCode -ne 0) { throw "Startup check failed ($($process.ExitCode)); see outputs/desktop-startup-check.stderr" }
if (-not (Test-Path -LiteralPath $report)) { throw 'Startup result is missing' }
$result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
if (-not $result.audio_cleanup_ok) { throw 'Audio cleanup initialization failed' }
if ($ImportValidationKey -and -not $result.key_imported) { throw 'Credential import failed' }
$result | Add-Member -NotePropertyName executable_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant())
$result | ConvertTo-Json | Set-Content -LiteralPath $report -Encoding utf8
$result | ConvertTo-Json -Compress
