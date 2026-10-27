param([switch]$CheckOnly, [switch]$PortableOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$desktopRoot = Join-Path $projectRoot 'desktop'
$bunPath = Join-Path $projectRoot '.cache\tools\bun-windows-x64\bun.exe'
$ortPath = Join-Path $projectRoot '.cache\native\onnxruntime-win-x64-1.24.2\lib'
$cmakePath = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
if (-not (Test-Path -LiteralPath $bunPath)) { throw 'Run scripts/prepare-desktop.ps1 first (Bun missing).' }
if (-not (Test-Path -LiteralPath (Join-Path $ortPath 'onnxruntime.dll'))) { throw 'Run scripts/prepare-desktop.ps1 first (ONNX Runtime missing).' }
$env:Path = (Split-Path -Parent $bunPath) + ';' + $cmakePath + ';' + $env:Path
$env:ORT_LIB_LOCATION = $ortPath
$env:ORT_PREFER_DYNAMIC_LINK = '1'
$env:GGML_NATIVE = 'OFF'
$env:HANDY_DISABLE_UPDATER = '1'
$vcInstall = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Redist\MSVC'
$vcRedist = Get-ChildItem -LiteralPath $vcInstall -Directory | Where-Object { $_.Name -match '^\d+\.' -and (Test-Path -LiteralPath (Join-Path $_.FullName 'x64')) } | Sort-Object Name -Descending | Select-Object -First 1
if ($vcRedist) {
    $vcRoot = Join-Path $vcRedist.FullName 'x64'
    $env:HANDY_VC_REDIST_DIRS = (Get-ChildItem -LiteralPath $vcRoot -Directory | Where-Object Name -Match 'CRT|OpenMP' | ForEach-Object FullName) -join ';'
}
Push-Location $desktopRoot
try {
    if ($CheckOnly) {
        & $bunPath run build
        if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
        cargo check --locked --manifest-path src-tauri\Cargo.toml
    } elseif ($PortableOnly) {
        & $bunPath run tauri build --no-bundle
    } else {
        & $bunPath run tauri build --bundles nsis
    }
    if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
} finally { Pop-Location }
