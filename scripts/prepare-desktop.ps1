[CmdletBinding()]
param([string]$Python = 'python')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
function Get-VerifiedFile([string]$Uri, [string]$RelativePath, [string]$Sha256) {
    $target = Join-Path $projectRoot $RelativePath
    New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
    if (-not (Test-Path -LiteralPath $target)) { Invoke-WebRequest -Uri $Uri -OutFile $target }
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $Sha256) { throw "Checksum mismatch: $RelativePath" }
    return $target
}
$bunArchive = Get-VerifiedFile 'https://github.com/oven-sh/bun/releases/download/bun-v1.4.4/bun-windows-x64.zip' '.cache\tools\bun-windows-x64.zip' 'cd2c0fccc78f3ede1f50ca827a15f063eda13d4d150a3d0674f4f6e323d864a5'
Expand-Archive -LiteralPath $bunArchive -DestinationPath (Join-Path $projectRoot '.cache\tools') -Force
$ortArchive = Get-VerifiedFile 'https://github.com/microsoft/onnxruntime/releases/download/v1.24.2/onnxruntime-win-x64-1.24.2.zip' '.cache\native\onnxruntime-win-x64-1.24.2.zip' '8e3e9c826375352e29cb2614fe44f3d7a4b0ff7b8028ad7a456af9d949a7e8b0'
Expand-Archive -LiteralPath $ortArchive -DestinationPath (Join-Path $projectRoot '.cache\native') -Force
$null = Get-VerifiedFile 'https://blob.handy.computer/silero_vad_v4.onnx' 'desktop\src-tauri\resources\models\silero_vad_v4.onnx' 'a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28'
$modelArchive = Get-VerifiedFile 'https://blob.handy.computer/sense-voice-int8.tar.gz' '.cache\models\sense-voice-int8.tar.gz' '171d611fe5d353a50bbb741b6f3ef42559b1565685684e9aa888ef563ba3e8a4'
$modelDest = Join-Path $projectRoot 'desktop\src-tauri\resources\models'
tar -xzf $modelArchive -C $modelDest 'sense-voice-int8/model.int8.onnx' 'sense-voice-int8/tokens.txt'
if ($LASTEXITCODE -ne 0) { throw 'Model extraction failed' }
$bunPath = Join-Path $projectRoot '.cache\tools\bun-windows-x64\bun.exe'
$env:Path = (Split-Path -Parent $bunPath) + ';' + $env:Path
& (Join-Path $PSScriptRoot 'prepare-meeting-apm.ps1') -Python $Python
if ($LASTEXITCODE -ne 0) { throw 'Meeting audio backend preparation failed' }
Push-Location (Join-Path $projectRoot 'desktop')
try {
    & $bunPath install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'Dependency installation failed' }
} finally { Pop-Location }
