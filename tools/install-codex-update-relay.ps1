[CmdletBinding()]
param(
    [switch]$RunSync
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$sshRoot = 'D:\AI\SSH\VPS_SSH_KEY'
$sshEngine = Join-Path $sshRoot 'scripts\Invoke-EncryptedSsh.ps1'
$encryptedKey = Join-Path $sshRoot 'hosts\菲律宾跳板机\key.enc.json'
$syncSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-sync.py') -Raw -Encoding UTF8
$configSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-config.json') -Raw -Encoding UTF8
$serviceSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-sync.service') -Raw -Encoding UTF8
$timerSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-sync.timer') -Raw -Encoding UTF8

function Convert-ToBase64 {
    param([Parameter(Mandatory)][string]$Value)
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value))
}

$syncB64 = Convert-ToBase64 $syncSource
$configB64 = Convert-ToBase64 $configSource
$serviceB64 = Convert-ToBase64 $serviceSource
$timerB64 = Convert-ToBase64 $timerSource
$runSyncValue = if ($RunSync) { '1' } else { '0' }

$remoteScript = @'
set -eu

if ! getent passwd codexrelay >/dev/null 2>&1; then
  useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin codexrelay
fi

install -d -o codexrelay -g codexrelay -m 0755 /srv/codex-update-relay
install -d -o codexrelay -g codexrelay -m 0755 /srv/codex-update-relay/export
install -d -o codexrelay -g codexrelay -m 0755 /srv/codex-update-relay/export/releases
install -d -o codexrelay -g codexrelay -m 0755 /srv/codex-update-relay/work
install -d -o codexrelay -g codexrelay -m 0755 /srv/codex-update-relay/state
install -d -o root -g root -m 0755 /etc/codex-update-relay
install -d -o root -g root -m 0755 /usr/local/libexec
chown -R codexrelay:codexrelay /srv/codex-update-relay

tmp_sync=$(mktemp)
printf '%s' '__SYNC_B64__' | base64 -d > "$tmp_sync"
install -o root -g root -m 0755 "$tmp_sync" /usr/local/libexec/codex-update-relay-sync.py
rm -f "$tmp_sync"

tmp_config=$(mktemp)
printf '%s' '__CONFIG_B64__' | base64 -d > "$tmp_config"
install -o root -g root -m 0644 "$tmp_config" /etc/codex-update-relay/config.json
rm -f "$tmp_config"

tmp_service=$(mktemp)
printf '%s' '__SERVICE_B64__' | base64 -d > "$tmp_service"
install -o root -g root -m 0644 "$tmp_service" /etc/systemd/system/codex-update-relay-sync.service
rm -f "$tmp_service"

tmp_timer=$(mktemp)
printf '%s' '__TIMER_B64__' | base64 -d > "$tmp_timer"
install -o root -g root -m 0644 "$tmp_timer" /etc/systemd/system/codex-update-relay-sync.timer
rm -f "$tmp_timer"

systemctl daemon-reload
runuser -u codexrelay -- /usr/bin/python3 /usr/local/libexec/codex-update-relay-sync.py --config /etc/codex-update-relay/config.json --dry-run
systemctl enable codex-update-relay-sync.timer
if [ '__RUN_SYNC__' = '1' ]; then
  systemctl start codex-update-relay-sync.service
fi
systemctl start codex-update-relay-sync.timer
systemctl is-enabled codex-update-relay-sync.timer
systemctl is-active codex-update-relay-sync.timer
if [ '__RUN_SYNC__' = '1' ]; then
  systemctl --no-pager --full status codex-update-relay-sync.service | sed -n '1,80p' || true
fi
'@

$remoteScript = $remoteScript.Replace('__SYNC_B64__', $syncB64)
$remoteScript = $remoteScript.Replace('__CONFIG_B64__', $configB64)
$remoteScript = $remoteScript.Replace('__SERVICE_B64__', $serviceB64)
$remoteScript = $remoteScript.Replace('__TIMER_B64__', $timerB64)
$remoteScript = $remoteScript.Replace('__RUN_SYNC__', $runSyncValue)

$remotePath = Join-Path ([IO.Path]::GetTempPath()) ('codex-update-relay-install-' + [Guid]::NewGuid().ToString('N') + '.sh')
Set-Content -LiteralPath $remotePath -Value $remoteScript -Encoding UTF8

try {
    & $sshEngine `
        -EncryptedKeyPath $encryptedKey `
        -HostName '141.11.138.152' `
        -Port 13226 `
        -User 'root' `
        -RemoteScriptPath $remotePath
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        throw "Remote relay installation failed with exit code $code."
    }
}
finally {
    Remove-Item -LiteralPath $remotePath -Force -ErrorAction SilentlyContinue
}
