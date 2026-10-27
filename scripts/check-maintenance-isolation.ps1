param([string]$Executable)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Executable) { $Executable = Join-Path $projectRoot 'outputs\desktop\Local Dictation\local-dictation.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$fixtureRoot = Join-Path $projectRoot ('.cache\maintenance-isolation-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$fixtureExe = Join-Path $fixtureRoot 'local-dictation.exe'
Copy-Item -LiteralPath $Executable -Destination $fixtureExe
Get-ChildItem -LiteralPath (Split-Path -Parent $Executable) -Filter '*.dll' -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $fixtureRoot
}
Set-Content -LiteralPath (Join-Path $fixtureRoot 'portable') -Value 'Handy Portable Mode' -NoNewline
$bundledModel = Join-Path $fixtureRoot 'resources\models\sense-voice-int8'
New-Item -ItemType Directory -Path $bundledModel -Force | Out-Null
foreach ($name in @('model.int8.onnx', 'tokens.txt')) {
    Set-Content -LiteralPath (Join-Path $bundledModel $name) -Value 'model-seeding sentinel; not a real model'
}
$fixtureData = Join-Path $fixtureRoot 'Data'
$fixtureAudio = Join-Path $fixtureData 'recordings'
New-Item -ItemType Directory -Path $fixtureAudio -Force | Out-Null
$history = Join-Path $fixtureData 'history.db'
Set-Content -LiteralPath $history -Value 'opaque history sentinel; maintenance must never open or alter it'
$historyHash = (Get-FileHash -LiteralPath $history).Hash
$expired = Join-Path $fixtureAudio 'expired.wav'
$recent = Join-Path $fixtureAudio 'recent.wav'
$other = Join-Path $fixtureAudio 'keep.txt'
foreach ($path in @($expired, $recent, $other)) { Set-Content -LiteralPath $path -Value 'synthetic fixture, no real audio' }
(Get-Item -LiteralPath $expired).LastWriteTime = (Get-Date).AddDays(-8)
(Get-Item -LiteralPath $other).LastWriteTime = (Get-Date).AddDays(-8)
$report = Join-Path $fixtureRoot 'report.json'
$arguments = @('--cleanup-only', '--maintenance-report', ('"' + $report + '"'))
$process = Start-Process -FilePath $fixtureExe -ArgumentList $arguments -WorkingDirectory $fixtureRoot -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $fixtureRoot 'stdout.log') -RedirectStandardError (Join-Path $fixtureRoot 'stderr.log')
if ($process.ExitCode -ne 0) { throw "Maintenance isolation failed ($($process.ExitCode)); evidence: $fixtureRoot" }
$result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
if (-not $result.audio_cleanup_ok) { throw 'Audio-only maintenance did not succeed' }
if (Test-Path -LiteralPath $expired) { throw 'Expired audio was not deleted' }
if (-not (Test-Path -LiteralPath $recent)) { throw 'Recent audio was deleted' }
if (-not (Test-Path -LiteralPath $other)) { throw 'Non-audio file was deleted' }
if ((Get-FileHash -LiteralPath $history).Hash -ne $historyHash) { throw 'Text history changed' }
if (Test-Path -LiteralPath (Join-Path $fixtureData 'models')) { throw 'Maintenance seeded a model' }
if (Test-Path -LiteralPath (Join-Path $fixtureData 'history.db-wal')) { throw 'Maintenance opened the history database' }
[pscustomobject]@{audio_cleanup_ok=$true;history_untouched=$true;recent_audio_preserved=$true;non_audio_preserved=$true;model_not_seeded=$true;fixture=$fixtureRoot} | ConvertTo-Json -Compress
