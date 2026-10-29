param([switch]$CheckOnly, [switch]$PortableOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$desktopRoot = Join-Path $projectRoot 'desktop'
$bunPath = Join-Path $projectRoot '.cache\tools\bun-windows-x64\bun.exe'
$ortPath = Join-Path $projectRoot '.cache\native\onnxruntime-win-x64-1.24.2\lib'
$cmakePath = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
if (-not (Test-Path -LiteralPath $bunPath)) { throw 'Run scripts/prepare-desktop.ps1 first (Bun missing).' }
if (-not (Test-Path -LiteralPath (Join-Path $ortPath 'onnxruntime.dll'))) { throw 'Run scripts/prepare-desktop.ps1 first (ONNX Runtime missing).' }
if (-not $CheckOnly) {
    # A native build can compile without these runtime files. Fail here rather
    # than emit an installer that cannot prepare the offline recognizer.
    $modelChecks = @(
        @('silero_vad_v4.onnx', 'a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28'),
        @('sense-voice-int8\model.int8.onnx', 'c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51'),
        @('sense-voice-int8\tokens.txt', 'f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc')
    )
    foreach ($modelCheck in $modelChecks) {
        $modelPath = Join-Path $desktopRoot ('src-tauri\resources\models\' + $modelCheck[0])
        if (-not (Test-Path -LiteralPath $modelPath -PathType Leaf)) { throw 'Run scripts/prepare-desktop.ps1 first (bundled model resource missing).' }
        if ((Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash -ne $modelCheck[1]) { throw 'Bundled model checksum mismatch; run scripts/prepare-desktop.ps1 to restore resources.' }
    }
}
$env:Path = (Split-Path -Parent $bunPath) + ';' + $cmakePath + ';' + $env:Path
$env:ORT_LIB_LOCATION = $ortPath
$env:ORT_PREFER_DYNAMIC_LINK = '1'
$env:MEETING_APM_DIR = Join-Path $projectRoot '.cache\native\webrtc-apm-2.1-msvc-x64-md'
if (-not (Test-Path -LiteralPath (Join-Path $env:MEETING_APM_DIR 'manifest.json'))) {
    throw 'Run scripts/prepare-meeting-apm.ps1 first (meeting audio backend is not prepared).'
}
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
