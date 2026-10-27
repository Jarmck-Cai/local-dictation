$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $projectRoot 'desktop\src-tauri'
$exePath = Join-Path $sourceRoot 'target\release\local-dictation.exe'
if (-not (Test-Path -LiteralPath $exePath)) { throw 'Build the current desktop release first.' }
$deliveryRoot = Join-Path $projectRoot 'outputs\desktop'
$appRoot = Join-Path $deliveryRoot 'Local Dictation'
New-Item -ItemType Directory -Force $appRoot | Out-Null
Copy-Item -LiteralPath $exePath -Destination $appRoot -Force
Get-ChildItem -LiteralPath (Join-Path $sourceRoot 'transcribe-libs') -Filter '*.dll' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $appRoot -Force }
Copy-Item -LiteralPath (Join-Path $sourceRoot 'resources') -Destination $appRoot -Recurse -Force
foreach ($notice in @('LICENSE','UPSTREAM.md','RESOURCES.md')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot ('desktop\' + $notice)) -Destination $appRoot -Force
}
$usage = Join-Path $projectRoot 'docs\desktop-usage.md'
if (Test-Path -LiteralPath $usage) { Copy-Item -LiteralPath $usage -Destination (Join-Path $appRoot '使用说明.md') -Force }
$bundleRoot = Join-Path $sourceRoot 'target\release\bundle\nsis'
if (Test-Path -LiteralPath $bundleRoot) {
    Get-ChildItem -LiteralPath $bundleRoot -Filter '*-setup.exe' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $deliveryRoot -Force }
}
$manifest = Get-ChildItem -LiteralPath $appRoot -File -Recurse | Where-Object { $_.FullName -notlike '*\Data\*' } | ForEach-Object {
    [pscustomobject]@{path=$_.FullName.Substring($appRoot.Length + 1);bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $deliveryRoot 'manifest.json') -Encoding utf8
Write-Output $appRoot
