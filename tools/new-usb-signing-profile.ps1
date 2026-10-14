[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReleaseVersion,
    [Parameter(Mandatory = $true)]
    [string]$ReleaseId,
    [Parameter(Mandatory = $true)]
    [string]$KeyId,
    [Parameter(Mandatory = $true)]
    [string]$PublicKeyB64,
    [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"

if ($ReleaseVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9._-]+)?$') {
    throw "ReleaseVersion 不是合法的 semver-like 值：$ReleaseVersion"
}
if ($ReleaseId -notmatch '^[A-Za-z0-9._-]{1,128}$') {
    throw "ReleaseId 不是合法的安全标识：$ReleaseId"
}
if ($KeyId -notmatch '^[A-Za-z0-9._-]{1,128}$') {
    throw "KeyId 不是合法的安全标识：$KeyId"
}

try {
    $publicKeyBytes = [Convert]::FromBase64String($PublicKeyB64.Trim())
} catch {
    throw "PublicKeyB64 不是有效 Base64"
}
if ($publicKeyBytes.Length -ne 32) {
    throw "PublicKeyB64 必须解码为 32 字节 Ed25519 公钥"
}

$hasher = [System.Security.Cryptography.SHA256]::Create()
try {
    $digestBytes = $hasher.ComputeHash($publicKeyBytes)
} finally {
    $hasher.Dispose()
}
$digest = (($digestBytes | ForEach-Object { $_.ToString('x2') }) -join '')

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path (Join-Path (Get-Location) "release-profiles") "usb-$ReleaseVersion-$ReleaseId.json"
}
$fullPath = [System.IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $fullPath) {
    throw "输出 profile 已存在，为避免覆盖历史发行配置而停止：$fullPath"
}
$parent = Split-Path -Parent $fullPath
if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
}

$profile = [ordered]@{
    schemaVersion = 1
    profile = "production"
    releaseVersion = $ReleaseVersion
    releaseId = $ReleaseId
    keyId = $KeyId
    publicKeyB64 = $PublicKeyB64.Trim()
    publicKeySha256 = $digest
}
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($fullPath, ($profile | ConvertTo-Json -Depth 4), $utf8NoBom)
Write-Output "Generated versioned USB signing profile: $fullPath"
Write-Output "Binding: keyId=$KeyId; publicKeySha256=$digest"
