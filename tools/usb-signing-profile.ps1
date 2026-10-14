Set-StrictMode -Version Latest

function Test-CodexUsbJsonInteger {
    param([object]$Value)

    return $null -ne $Value -and (
        $Value -is [sbyte] -or $Value -is [byte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64]
    )
}

function Get-CodexUsbProfileField {
    param(
        [Parameter(Mandatory = $true)]
        $Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-CodexUsbSigningProfile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [string]$ProfilePath = "",
        [switch]$RequireExplicitPath
    )

    if ([string]::IsNullOrWhiteSpace($ProfilePath)) {
        throw "必须显式传入当前版本 USB signing profile；不能回退到仓库中的固定 profile"
    }
    if (-not (Test-Path -LiteralPath $ProfilePath -PathType Leaf)) {
        throw "Missing versioned USB release signing profile: $ProfilePath; refusing to package with an arbitrary local key"
    }

    try {
        $profile = Get-Content -Raw -LiteralPath $ProfilePath | ConvertFrom-Json
    } catch {
        throw "USB release signing profile is not valid JSON: $ProfilePath; $($_.Exception.Message)"
    }
    if (-not (Test-CodexUsbJsonInteger $profile.schemaVersion) -or
        $profile.schemaVersion -ne 1 -or [string]$profile.profile -ne "production") {
        throw "USB release signing profile has an invalid schema or profile: $ProfilePath"
    }

    $releaseVersion = ([string](Get-CodexUsbProfileField -Object $profile -Name "releaseVersion")).Trim()
    $releaseId = ([string](Get-CodexUsbProfileField -Object $profile -Name "releaseId")).Trim()
    if ($RequireExplicitPath -and
        ($releaseVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9._-]+)?$' -or
         $releaseId -notmatch '^[A-Za-z0-9._-]{1,128}$')) {
        throw "版本化 USB signing profile 必须包含合法 releaseVersion 和 releaseId：$ProfilePath"
    }

    $keyId = [string]$profile.keyId
    $publicKeyB64 = ([string]$profile.publicKeyB64).Trim()
    $declaredDigest = ([string]$profile.publicKeySha256).Trim().ToLowerInvariant()
    if ($keyId -notmatch '^[A-Za-z0-9._-]{1,128}$') {
        throw "USB release signing profile has an invalid keyId: $keyId"
    }
    try {
        $publicKeyBytes = [Convert]::FromBase64String($publicKeyB64)
    } catch {
        throw "USB release signing profile has invalid publicKeyB64: $ProfilePath"
    }
    if ($publicKeyBytes.Length -ne 32) {
        throw "USB release public key must decode to 32 bytes: $ProfilePath"
    }
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $digestBytes = $hasher.ComputeHash($publicKeyBytes)
    } finally {
        $hasher.Dispose()
    }
    $computedDigest = (($digestBytes | ForEach-Object { $_.ToString('x2') }) -join '')
    if ($declaredDigest -notmatch '^[0-9a-f]{64}$' -or $declaredDigest -cne $computedDigest) {
        throw "USB release signing profile publicKeySha256 does not match its public key: declared=$declaredDigest, computed=$computedDigest"
    }

    return [pscustomobject]@{
        Path = (Resolve-Path -LiteralPath $ProfilePath).Path
        KeyId = $keyId
        PublicKeyB64 = $publicKeyB64
        PublicKeySha256 = $computedDigest
        ReleaseVersion = $releaseVersion
        ReleaseId = $releaseId
        Explicit = $RequireExplicitPath.IsPresent
    }
}

function Initialize-CodexUsbSigningEnvironment {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        $Profile
    )

    $configuredKeyId = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_ID
    $configuredPublicKey = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_B64
    $hasKeyId = -not [string]::IsNullOrWhiteSpace($configuredKeyId)
    $hasPublicKey = -not [string]::IsNullOrWhiteSpace($configuredPublicKey)
    if (-not $hasKeyId -and -not $hasPublicKey) {
        $env:CODEX_USB_GUARD_PUBLIC_KEY_ID = [string]$Profile.KeyId
        $env:CODEX_USB_GUARD_PUBLIC_KEY_B64 = [string]$Profile.PublicKeyB64
        return
    }
    if (-not $hasKeyId -or -not $hasPublicKey) {
        throw "USB release signing environment is only partially configured; clear CODEX_USB_GUARD_PUBLIC_KEY_ID/CODEX_USB_GUARD_PUBLIC_KEY_B64 or configure both"
    }

    try {
        $configuredBytes = [Convert]::FromBase64String($configuredPublicKey.Trim())
    } catch {
        throw "CODEX_USB_GUARD_PUBLIC_KEY_B64 is not valid Base64; refusing to create a package that may not be signable"
    }
    if ($configuredBytes.Length -ne 32) {
        throw "Configured USB public key is not 32 bytes; refusing to create a package that may not be signable"
    }
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $digestBytes = $hasher.ComputeHash($configuredBytes)
    } finally {
        $hasher.Dispose()
    }
    $configuredDigest = (($digestBytes | ForEach-Object { $_.ToString('x2') }) -join '')
    if ($configuredKeyId -cne [string]$Profile.KeyId -or
        $configuredDigest -cne [string]$Profile.PublicKeySha256) {
        throw "Configured release key does not match the supplied version profile: configured keyId=$configuredKeyId / digest=$configuredDigest; profile keyId=$($Profile.KeyId) / digest=$($Profile.PublicKeySha256). Do not replace a release profile with the current machine's DPAPI key"
    }
}

function Assert-CodexUsbSigningProfileVersion {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        $Profile,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedVersion
    )

    if ([string]::IsNullOrWhiteSpace([string]$Profile.ReleaseVersion)) {
        throw "版本化 USB signing profile 缺少 releaseVersion；拒绝把未标版本的 profile 用于正式构建"
    }
    if ([string]$Profile.ReleaseVersion -cne $ExpectedVersion) {
        throw "USB signing profile releaseVersion 与当前包版本不一致：profile=$($Profile.ReleaseVersion)，package=$ExpectedVersion"
    }
}
