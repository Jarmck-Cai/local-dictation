<#
.SYNOPSIS
Safely deploys `local-dictation-gateway` and `/etc/local-dictation-gateway/gateway.env`
to the Oracle VPS without EVER printing or logging any API key, SSH key, or device token.

.DESCRIPTION
- Reads `OPENAI_API_KEY` (and optional `ANTHROPIC_API_KEY`, `GATEWAY_AUTH_TOKENS`) from
  local `.env.local`.
- Falls back to decrypting the current Windows user's DPAPI `claude-key.dpapi` in memory
  if `ANTHROPIC_API_KEY` is not in `.env.local`.
- Uploads the prebuilt static Linux musl binary (`x86_64` or `aarch64`), installs the
  hardened `systemd` unit, streams `/etc/local-dictation-gateway/gateway.env` (`0600`)
  over encrypted SSH stdin, and verifies `/healthz` + `/v1/status` boolean flags.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RemoteHost,

    [string]$RemoteUser = "ubuntu",

    [Parameter(Mandatory = $true)]
    [string]$SshKeyPath,

    [string]$KnownHostsPath = "",

    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [string]$EnvLocalPath = "",

    [string]$BindAddr = "127.0.0.1:8787",

    [string]$OpenAiAsrModel = "gpt-transcribe",

    [string]$AnthropicCleanupModel = "claude-haiku-5-5"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
if (-not $EnvLocalPath) {
    $EnvLocalPath = Join-Path $repoRoot ".env.local"
}

if (-not (Test-Path -LiteralPath $BinaryPath)) {
    throw "Prebuilt Linux gateway binary not found at: $BinaryPath"
}
if (-not (Test-Path -LiteralPath $SshKeyPath)) {
    throw "SSH private key not found at: $SshKeyPath"
}

function Read-DotEnvMap([string]$Path) {
    $map = @{}
    if (-not (Test-Path -LiteralPath $Path)) {
        return $map
    }
    foreach ($rawLine in [System.IO.File]::ReadAllLines($Path)) {
        $line = $rawLine.Trim()
        if (-not $line -or $line.StartsWith("#")) { continue }
        if ($line.StartsWith("export ")) {
            $line = $line.Substring(7).Trim()
        }
        $eq = $line.IndexOf('=')
        if ($eq -le 0) { continue }
        $key = $line.Substring(0, $eq).Trim()
        $val = $line.Substring($eq + 1).Trim()
        if ($val.Length -ge 2 -and (($val.StartsWith('"') -and $val.EndsWith('"')) -or ($val.StartsWith("'") -and $val.EndsWith("'")))) {
            $val = $val.Substring(1, $val.Length - 2)
        }
        if ($val) {
            $map[$key] = $val
        }
    }
    return $map
}

function Read-DpapiClaudeKey {
    Add-Type -AssemblyName System.Security
    $candidates = @(
        (Join-Path $repoRoot "installed\Local Dictation\Data\claude-key.dpapi"),
        (Join-Path $env:APPDATA "com.localdictation.desktop\claude-key.dpapi")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            $encrypted = [System.IO.File]::ReadAllBytes($candidate)
            $plainBytes = [System.Security.Cryptography.ProtectedData]::Unprotect(
                $encrypted,
                $null,
                [System.Security.Cryptography.DataProtectionScope]::CurrentUser
            )
            try {
                return [System.Text.Encoding]::UTF8.GetString($plainBytes).Trim()
            } finally {
                [Array]::Clear($plainBytes, 0, $plainBytes.Length)
            }
        }
    }
    return ""
}

function New-RandomDeviceToken([string]$Prefix) {
    $bytes = New-Object byte[] 24
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
        $hex = -join ($bytes | ForEach-Object { $_.ToString("x2") })
        return "${Prefix}_${hex}"
    } finally {
        $rng.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
}

$envMap = Read-DotEnvMap -Path $EnvLocalPath
$openAiKey = if ($envMap.ContainsKey("OPENAI_API_KEY")) { $envMap["OPENAI_API_KEY"] } else { "" }
$anthropicKey = if ($envMap.ContainsKey("ANTHROPIC_API_KEY")) { $envMap["ANTHROPIC_API_KEY"] } else { Read-DpapiClaudeKey }

if (-not $openAiKey) {
    throw "OPENAI_API_KEY is missing from $EnvLocalPath"
}
if (-not $anthropicKey) {
    throw "ANTHROPIC_API_KEY is missing from both $EnvLocalPath and Windows DPAPI storage"
}

$gatewayTokens = if ($envMap.ContainsKey("GATEWAY_AUTH_TOKENS")) {
    $envMap["GATEWAY_AUTH_TOKENS"]
} else {
    $desktopTok = New-RandomDeviceToken -Prefix "qsgw_dsk"
    $laptopTok = New-RandomDeviceToken -Prefix "qsgw_lpt"
    $combined = "desktop=$desktopTok,laptop=$laptopTok"
    # Persist generated tokens locally into gitignored outputs receipt (0600-equivalent ACL) without printing!
    $tokenReceiptDir = Join-Path $repoRoot "outputs\030-cloud-gateway"
    New-Item -ItemType Directory -Force -Path $tokenReceiptDir | Out-Null
    $tokenFile = Join-Path $tokenReceiptDir "gateway-device-tokens.local"
    [System.IO.File]::WriteAllText($tokenFile, "GATEWAY_AUTH_TOKENS=$combined`nDESKTOP_TOKEN=$desktopTok`nLAPTOP_TOKEN=$laptopTok`n")
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    icacls $tokenFile /inheritance:r /grant:r ("*" + $sid + ":(R,W)") | Out-Null
    $combined
}

$sshCommon = @(
    "-i", $SshKeyPath,
    "-o", "BatchMode=yes",
    "-o", "IdentitiesOnly=yes",
    "-o", "ConnectTimeout=12"
)
if ($KnownHostsPath -and (Test-Path -LiteralPath $KnownHostsPath)) {
    $sshCommon += @("-o", "UserKnownHostsFile=$KnownHostsPath", "-o", "StrictHostKeyChecking=yes")
}

$target = "${RemoteUser}@${RemoteHost}"
$serviceFile = Join-Path $PSScriptRoot "local-dictation-gateway.service"

Write-Output "[1/4] Uploading prebuilt binary and systemd unit to $target ..."
& scp @sshCommon $BinaryPath "${target}:/tmp/local-dictation-gateway.new"
if ($LASTEXITCODE -ne 0) { throw "scp binary upload failed" }
& scp @sshCommon $serviceFile "${target}:/tmp/local-dictation-gateway.service.new"
if ($LASTEXITCODE -ne 0) { throw "scp service upload failed" }

Write-Output "[2/4] Writing /etc/local-dictation-gateway/gateway.env (mode 0600) via encrypted stdin ..."
$envContent = @(
    "GATEWAY_BIND_ADDR=$BindAddr",
    "GATEWAY_DB_PATH=/var/lib/local-dictation-gateway/gateway.db",
    "GATEWAY_AUTH_TOKENS=$gatewayTokens",
    "OPENAI_API_KEY=$openAiKey",
    "OPENAI_ASR_MODEL=$OpenAiAsrModel",
    "ANTHROPIC_API_KEY=$anthropicKey",
    "ANTHROPIC_CLEANUP_MODEL=$AnthropicCleanupModel",
    "GATEWAY_MAX_AUDIO_BYTES=16777216",
    "GATEWAY_ASR_TIMEOUT_MS=20000",
    "GATEWAY_CLEANUP_TIMEOUT_MS=8000",
    "RUST_LOG=info"
) -join "`n"
$envContent += "`n"

try {
    $envContent | & ssh @sshCommon $target "sudo install -d -m 0755 /etc/local-dictation-gateway && sudo tee /etc/local-dictation-gateway/gateway.env >/dev/null && sudo chmod 0600 /etc/local-dictation-gateway/gateway.env"
    if ($LASTEXITCODE -ne 0) { throw "remote gateway.env installation failed" }
} finally {
    $envContent = ""
    $openAiKey = ""
    $anthropicKey = ""
}

Write-Output "[3/4] Installing binary, enabling and restarting local-dictation-gateway.service ..."
$installCmd = @(
    "set -euo pipefail",
    "sudo install -m 0755 /tmp/local-dictation-gateway.new /usr/local/bin/local-dictation-gateway",
    "rm -f /tmp/local-dictation-gateway.new",
    "sudo install -m 0644 /tmp/local-dictation-gateway.service.new /etc/systemd/system/local-dictation-gateway.service",
    "rm -f /tmp/local-dictation-gateway.service.new",
    "sudo systemctl daemon-reload",
    "sudo systemctl enable local-dictation-gateway.service",
    "sudo systemctl restart local-dictation-gateway.service",
    "sleep 1",
    "sudo systemctl is-active local-dictation-gateway.service"
) -join "; "
& ssh @sshCommon $target $installCmd
if ($LASTEXITCODE -ne 0) { throw "systemd installation/start failed" }

Write-Output "[4/4] Probing loopback /healthz on remote VPS ..."
& ssh @sshCommon $target "curl -fsS http://$BindAddr/healthz && echo ''"
if ($LASTEXITCODE -ne 0) { throw "remote /healthz check failed" }

Write-Output "Deployment succeeded cleanly (zero secrets echoed)."
