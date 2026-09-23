[CmdletBinding()]
param([switch]$RunSync,[switch]$VerifyPublic)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$sshRoot = 'D:\AI\SSH\VPS_SSH_KEY'
$sshEngine = Join-Path $sshRoot 'scripts\Invoke-EncryptedSsh.ps1'
$encryptedKey = Join-Path $sshRoot 'hosts\菲律宾跳板机\key.enc.json'
function B64([string]$v) { [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($v)) }
function Src([string]$n) { Get-Content -LiteralPath (Join-Path $PSScriptRoot $n) -Raw -Encoding UTF8 }
$sync = B64 (Src 'codex-update-relay-sync.py')
$config = B64 (Src 'codex-update-relay-config.json')
$service = B64 (Src 'codex-update-relay-sync.service')
$failure = B64 (Src 'codex-update-relay-sync-failure.service')
$timer = B64 (Src 'codex-update-relay-sync.timer')
$run = if ($RunSync) { '1' } else { '0' }
$verify = '1'
$remote = @'
set -eu
root=/srv/codex-update-relay
stamp=$(date -u +%Y%m%dT%H%M%SZ)
backup=$root/state/backups/$stamp
install -d -o root -g root -m 0700 $backup
if [ -L $root/export/current ]; then
  readlink $root/export/current > $backup/current.pointer
  readlink -e $root/export/current > $backup/current.resolved
  sha256sum $root/export/current/latest.json > $backup/current.latest.sha256
fi
for f in /etc/codex-update-relay/config.json /etc/systemd/system/codex-update-relay-sync.service /etc/systemd/system/codex-update-relay-sync.timer; do
  if [ -f $f ]; then cp -p $f $backup/$(basename $f); sha256sum $f >> $backup/config-files.sha256; fi
done
echo backup=$backup
getent passwd codexrelay >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin codexrelay
install -d -o codexrelay -g codexrelay -m 0755 $root $root/export $root/export/releases $root/work $root/state
install -d -o root -g root -m 0755 /etc/codex-update-relay /usr/local/libexec
tmp=$(mktemp); printf '%s' '__SYNC__' | base64 -d > $tmp; install -o root -g root -m 0755 $tmp /usr/local/libexec/codex-update-relay-sync.py; rm -f $tmp
tmp=$(mktemp); printf '%s' '__CONFIG__' | base64 -d > $tmp; install -o root -g root -m 0644 $tmp /etc/codex-update-relay/config.json; rm -f $tmp
tmp=$(mktemp); printf '%s' '__SERVICE__' | base64 -d > $tmp; install -o root -g root -m 0644 $tmp /etc/systemd/system/codex-update-relay-sync.service; rm -f $tmp
tmp=$(mktemp); printf '%s' '__FAILURE__' | base64 -d > $tmp; install -o root -g root -m 0644 $tmp /etc/systemd/system/codex-update-relay-sync-failure.service; rm -f $tmp
tmp=$(mktemp); printf '%s' '__TIMER__' | base64 -d > $tmp; install -o root -g root -m 0644 $tmp /etc/systemd/system/codex-update-relay-sync.timer; rm -f $tmp
systemctl daemon-reload
systemctl enable codex-update-relay-sync.timer >/dev/null
runuser -u codexrelay -- /usr/bin/python3 /usr/local/libexec/codex-update-relay-sync.py --config /etc/codex-update-relay/config.json --dry-run
systemctl start codex-update-relay-sync.timer
if [ '__RUN__' = 1 ]; then systemctl reset-failed codex-update-relay-sync.service || true; systemctl start codex-update-relay-sync.service; fi
systemctl is-enabled codex-update-relay-sync.timer
systemctl is-active codex-update-relay-sync.timer
systemctl show codex-update-relay-sync.timer -p NextElapseUSecRealtime -p Persistent
if [ '__VERIFY__' = 1 ]; then
/usr/bin/python3 - <<'PY'
import json, urllib.request
base='https://update.404token.xyz/__codex_update__/'
def fetch(url):
    with urllib.request.urlopen(urllib.request.Request(url,headers={'Cache-Control':'no-cache'}),timeout=60) as r:
        return json.loads(r.read().decode()),r.status
up,_=fetch('https://api.github.com/repos/Wangnov/codex-app-mirror/releases/latest')
pub,status=fetch(base+'latest.json')
if status != 200 or int(pub['releaseId']) != int(up['id']) or pub.get('tag') != up.get('tag_name'): raise SystemExit('public latest mismatch')
assets={a['name']:a for a in up['assets']}
for key,item in pub['assets'].items():
    a=assets.get(item['path'])
    if not a or int(item['id']) != int(a['id']) or int(item['size']) != int(a['size']) or item.get('githubDigest') != a.get('digest'): raise SystemExit('public asset identity mismatch '+key)
    request=urllib.request.Request(base+item['path'],headers={'Range':'bytes=0-0'})
    with urllib.request.urlopen(request,timeout=60) as r:
        content_range=r.headers.get('Content-Range','')
        if r.status not in (200,206) or (r.status == 206 and not content_range.endswith('/'+str(item['size']))): raise SystemExit('public asset download/size mismatch '+key)
        r.read(1)
print('PUBLIC_ACCEPTANCE_OK releaseId=%s tag=%s' % (pub['releaseId'],pub['tag']))
PY
fi
'@
$remote = $remote.Replace('__SYNC__',$sync).Replace('__CONFIG__',$config).Replace('__SERVICE__',$service).Replace('__FAILURE__',$failure).Replace('__TIMER__',$timer).Replace('__RUN__',$run).Replace('__VERIFY__',$verify)
$local = Join-Path ([IO.Path]::GetTempPath()) ('codex-relay-' + [Guid]::NewGuid().ToString('N') + '.sh')
Set-Content -LiteralPath $local -Value $remote -Encoding UTF8
try {
  & $sshEngine -EncryptedKeyPath $encryptedKey -HostName '141.11.138.152' -Port 13226 -User 'root' -RemoteScriptPath $local
  if ($LASTEXITCODE -ne 0) { throw "remote relay installation failed with exit code $LASTEXITCODE" }
} finally { Remove-Item -LiteralPath $local -Force -ErrorAction SilentlyContinue }
