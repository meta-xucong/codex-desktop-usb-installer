[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$sshRoot = 'D:\AI\SSH\VPS_SSH_KEY'
$sshEngine = Join-Path $sshRoot 'scripts\Invoke-EncryptedSsh.ps1'
$encryptedKey = Join-Path $sshRoot 'hosts\菲律宾跳板机\key.enc.json'
$edgeConfig = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-edge.conf') -Raw -Encoding UTF8
$edgeCompose = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-edge-compose.yml') -Raw -Encoding UTF8
$certScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'issue-codex-update-relay-cert.sh') -Raw -Encoding UTF8
$renewScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'renew-codex-update-relay-cert.sh') -Raw -Encoding UTF8
$renewService = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-cert-renew.service') -Raw -Encoding UTF8
$renewTimer = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codex-update-relay-cert-renew.timer') -Raw -Encoding UTF8

function Convert-ToBase64 {
    param([Parameter(Mandatory)][string]$Value)
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value))
}

$configB64 = Convert-ToBase64 $edgeConfig
$composeB64 = Convert-ToBase64 $edgeCompose
$certB64 = Convert-ToBase64 $certScript
$renewB64 = Convert-ToBase64 $renewScript
$renewServiceB64 = Convert-ToBase64 $renewService
$renewTimerB64 = Convert-ToBase64 $renewTimer

$remoteScript = @'
set -eu

install -d -o root -g root -m 0755 /opt/codex-update-relay-edge
install -d -o root -g root -m 0755 /opt/codex-update-relay-edge/conf.d
install -d -o root -g root -m 0700 /srv/codex-update-relay/tls

tmp_config=$(mktemp)
printf '%s' '__CONFIG_B64__' | base64 -d > "$tmp_config"
install -o root -g root -m 0644 "$tmp_config" /opt/codex-update-relay-edge/nginx.conf
install -o root -g root -m 0644 "$tmp_config" /opt/codex-update-relay-edge/conf.d/default.conf
rm -f "$tmp_config"

tmp_compose=$(mktemp)
printf '%s' '__COMPOSE_B64__' | base64 -d > "$tmp_compose"
install -o root -g root -m 0644 "$tmp_compose" /opt/codex-update-relay-edge/docker-compose.yml
rm -f "$tmp_compose"

tmp_cert=$(mktemp)
printf '%s' '__CERT_B64__' | base64 -d > "$tmp_cert"
install -o root -g root -m 0700 "$tmp_cert" /opt/codex-update-relay-edge/issue-cert.sh
rm -f "$tmp_cert"

tmp_renew=$(mktemp)
printf '%s' '__RENEW_B64__' | base64 -d > "$tmp_renew"
install -o root -g root -m 0700 "$tmp_renew" /opt/codex-update-relay-edge/renew-cert.sh
rm -f "$tmp_renew"

tmp_renew_service=$(mktemp)
printf '%s' '__RENEW_SERVICE_B64__' | base64 -d > "$tmp_renew_service"
install -o root -g root -m 0644 "$tmp_renew_service" /etc/systemd/system/codex-update-relay-cert-renew.service
rm -f "$tmp_renew_service"

tmp_renew_timer=$(mktemp)
printf '%s' '__RENEW_TIMER_B64__' | base64 -d > "$tmp_renew_timer"
install -o root -g root -m 0644 "$tmp_renew_timer" /etc/systemd/system/codex-update-relay-cert-renew.timer
rm -f "$tmp_renew_timer"

if command -v docker-compose >/dev/null 2>&1; then
  docker-compose -f /opt/codex-update-relay-edge/docker-compose.yml config >/dev/null
fi

systemctl daemon-reload
systemctl enable --now codex-update-relay-cert-renew.timer

echo 'edge prework installed; container not started and port 443 unchanged'
stat -c '%n mode=%a owner=%U group=%G' \
  /opt/codex-update-relay-edge/nginx.conf \
  /opt/codex-update-relay-edge/docker-compose.yml \
  /opt/codex-update-relay-edge/issue-cert.sh \
  /opt/codex-update-relay-edge/renew-cert.sh \
  /etc/systemd/system/codex-update-relay-cert-renew.service \
  /etc/systemd/system/codex-update-relay-cert-renew.timer \
  /srv/codex-update-relay/tls
systemctl is-enabled codex-update-relay-cert-renew.timer
systemctl is-active codex-update-relay-cert-renew.timer
'@

$remoteScript = $remoteScript.Replace('__CONFIG_B64__', $configB64)
$remoteScript = $remoteScript.Replace('__COMPOSE_B64__', $composeB64)
$remoteScript = $remoteScript.Replace('__CERT_B64__', $certB64)
$remoteScript = $remoteScript.Replace('__RENEW_B64__', $renewB64)
$remoteScript = $remoteScript.Replace('__RENEW_SERVICE_B64__', $renewServiceB64)
$remoteScript = $remoteScript.Replace('__RENEW_TIMER_B64__', $renewTimerB64)

$remotePath = Join-Path ([IO.Path]::GetTempPath()) ('codex-update-relay-edge-' + [Guid]::NewGuid().ToString('N') + '.sh')
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
        throw "Remote relay edge preparation failed with exit code $code."
    }
}
finally {
    Remove-Item -LiteralPath $remotePath -Force -ErrorAction SilentlyContinue
}
