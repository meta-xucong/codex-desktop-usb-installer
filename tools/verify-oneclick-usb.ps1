[CmdletBinding()]
param(
    [string]$BundlePath = "",
    [string]$ApiKey,
    [switch]$PromptForApiKey,
    [switch]$SkipNetwork,
    [switch]$InternalSigned,
    [string]$UsbSigningProfilePath = "",
    [switch]$SkipSignature,
    [switch]$SelfTestSchemaPolicy
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = $PSScriptRoot }
$root = (Resolve-Path -LiteralPath $BundlePath).Path
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$bundledVerificationDir = Join-Path $PSScriptRoot "verification"
$bundledProfileHelper = Join-Path $bundledVerificationDir "usb-signing-profile.ps1"
$repositoryProfileHelper = Join-Path $repo "tools\usb-signing-profile.ps1"
if (Test-Path -LiteralPath $bundledProfileHelper -PathType Leaf) {
    . $bundledProfileHelper
} elseif (Test-Path -LiteralPath $repositoryProfileHelper -PathType Leaf) {
    # Source-tree verification keeps the existing developer workflow. A
    # generated USB bundle uses the public-only helper copied under
    # verification\ so the script remains self-contained after the bundle is
    # moved to another drive or computer.
    . $repositoryProfileHelper
} else {
    throw "缺少 USB 签名校验辅助脚本：$bundledProfileHelper 或 $repositoryProfileHelper"
}
if ([string]::IsNullOrWhiteSpace($UsbSigningProfilePath)) {
    $bundledProfile = Join-Path $bundledVerificationDir "usb-signing-release-profile.json"
    if (Test-Path -LiteralPath $bundledProfile -PathType Leaf) {
        $UsbSigningProfilePath = $bundledProfile
    }
}

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "验证失败：$Message" }
}

function Read-JsonFile([string]$Path) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "缺少 $Path"
    # Windows PowerShell 5.1 defaults to the active ANSI code page for
    # BOM-less UTF-8. Release metadata is UTF-8, so decode it explicitly.
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8) | ConvertFrom-Json
}

function Get-JsonPropertyValue($Object, [string]$Name) {
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Test-IntegerValue([object]$Value) {
    return $null -ne $Value -and (
        $Value -is [sbyte] -or $Value -is [byte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64]
    )
}

function Get-JsonInteger([object]$Object, [string]$Name) {
    if ($null -eq $Object) {
        return $null
    }
    if ($Object -is [System.Collections.IDictionary]) {
        if (-not $Object.Contains($Name)) {
            return $null
        }
        $value = $Object[$Name]
    } else {
        $property = $Object.PSObject.Properties[$Name]
        if ($null -eq $property) {
            return $null
        }
        $value = $property.Value
    }
    if (-not (Test-IntegerValue $value)) {
        return $null
    }
    return $value
}

function Assert-ManifestSchemaPair($RootManifest, $UsbManifest) {
    $rootSchema = Get-JsonInteger $RootManifest "schemaVersion"
    $usbSchema = Get-JsonInteger $UsbManifest "schemaVersion"
    Assert-Condition ($rootSchema -in @(2, 3)) "根 manifest schemaVersion 只支持 2 或 3"
    Assert-Condition ($usbSchema -in @(2, 3)) "联合清单 schemaVersion 只支持 2 或 3"
    Assert-Condition ($rootSchema -eq $usbSchema) "根 manifest 与联合清单 schemaVersion 不一致"
    if ($rootSchema -eq 3) {
        $policy = Get-JsonPropertyValue $RootManifest "revocationPolicy"
        $registrationRequired = Get-JsonPropertyValue $policy "registrationRequired"
        Assert-Condition ($registrationRequired -is [bool] -and $registrationRequired -eq $true) "schema 3 根 manifest 必须锚定 registrationRequired=true"
    }
    return [int]$rootSchema
}

function Test-RevocationRuntimePath([string]$RelativePath) {
    if ($RelativePath -ieq "usb-license-revocation-registration.json" -or
        $RelativePath -ieq "CodexUsbState/revocation.json") {
        return $true
    }
    if ($RelativePath -notmatch '(?i)^CodexUsbState/revocation\.json\.tmp-([1-9][0-9]*)-([1-9][0-9]*)$') {
        return $false
    }
    [uint32]$pidValue = 0
    [uint64]$counterValue = 0
    return [uint32]::TryParse($Matches[1], [ref]$pidValue) -and
        [uint64]::TryParse($Matches[2], [ref]$counterValue) -and
        $pidValue -gt 0 -and $counterValue -gt 0
}

if ($SelfTestSchemaPolicy) {
    $v2Root = '{"schemaVersion":2}' | ConvertFrom-Json
    $v2Usb = '{"schemaVersion":2}' | ConvertFrom-Json
    $v3Root = '{"schemaVersion":3,"revocationPolicy":{"registrationRequired":true}}' | ConvertFrom-Json
    $v3Usb = '{"schemaVersion":3}' | ConvertFrom-Json
    $v3RootWithoutPolicy = '{"schemaVersion":3}' | ConvertFrom-Json
    $v3RootWithFalsePolicy = '{"schemaVersion":3,"revocationPolicy":{"registrationRequired":false}}' | ConvertFrom-Json
    Assert-Condition ((Assert-ManifestSchemaPair $v2Root $v2Usb) -eq 2) "schema 2 fixture 未被接受"
    Assert-Condition ((Assert-ManifestSchemaPair $v3Root $v3Usb) -eq 3) "schema 3 fixture 未被接受"
    $failed = 0
    foreach ($pair in @(
        [pscustomobject]@{ root = $v3Root; usb = $v2Usb },
        [pscustomobject]@{ root = $v3RootWithoutPolicy; usb = $v3Usb },
        [pscustomobject]@{ root = $v3RootWithFalsePolicy; usb = $v3Usb }
    )) {
        try { [void](Assert-ManifestSchemaPair $pair.root $pair.usb) }
        catch { $failed++ }
    }
    Assert-Condition ($failed -eq 3) "schema mismatch 或无效 v3 策略 fixture 未被拒绝"
    [pscustomobject]@{ passed = $true; validSchema2 = $true; validSchema3 = $true; rejectedCases = $failed } | ConvertTo-Json -Compress
    exit 0
}

function Normalize-BundleRelativePath([string]$RelativePath) {
    $relative = ([string]$RelativePath).Trim().Replace('/', '\')
    if ([string]::IsNullOrWhiteSpace($relative) -or [System.IO.Path]::IsPathRooted($relative)) {
        throw "manifest 中的路径必须是 bundle 内的相对路径：$RelativePath"
    }
    foreach ($segment in $relative.Split('\')) {
        if ([string]::IsNullOrWhiteSpace($segment) -or $segment -eq '.' -or $segment -eq '..' -or $segment.Contains(':')) {
            throw "manifest 中的路径包含非法片段：$RelativePath"
        }
    }
    return $relative.Replace('\', '/')
}

function Resolve-BundleRelativeFile([string]$RootPath, [string]$RelativePath) {
    $relative = Normalize-BundleRelativePath $RelativePath
    $candidate = Join-Path $RootPath ($relative.Replace('/', '\'))
    Assert-Condition (Test-Path -LiteralPath $candidate -PathType Leaf) "manifest.payload.path 指向的文件不存在：$relative"
    $resolved = (Resolve-Path -LiteralPath $candidate).Path
    $rootFull = [System.IO.Path]::GetFullPath($RootPath).TrimEnd('\') + '\'
    $resolvedFull = [System.IO.Path]::GetFullPath($resolved)
    Assert-Condition $resolvedFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase) "manifest.payload.path 不得逃逸 bundle 根目录"
    return $resolved
}

function Get-RelativePathCompat([string]$BasePath, [string]$FilePath) {
    $base = [System.IO.Path]::GetFullPath($BasePath).TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath($FilePath)
    Assert-Condition $full.StartsWith($base, [System.StringComparison]::OrdinalIgnoreCase) "文件不在 bundle 根目录内：$FilePath"
    return $full.Substring($base.Length)
}

function Assert-SkillHubSourceResource([string]$RootPath) {
    $relative = "resources/skill_hub_sources.json"
    $path = Join-Path $RootPath ($relative.Replace('/', '\'))
    Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) "缺少 Skill Hub 来源目录：$relative"
    $item = Get-Item -LiteralPath $path -Force
    Assert-Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "Skill Hub 来源目录不能是链接或联接点：$relative"
    $source = Read-JsonFile $path
    Assert-Condition ((Get-JsonInteger $source "schemaVersion") -eq 1 -and [string]$source.channel -ceq "skill-hub") "Skill Hub 来源目录 schema/channel 无效"
    $sources = @($source.sources)
    Assert-Condition ($sources.Count -ge 2) "Skill Hub 来源目录至少需要 Relay 与 GitHub"
    $relay = @($sources | Where-Object { [string]$_.id -ceq "ph-relay" -and [string]$_.kind -ceq "relay" })
    $github = @($sources | Where-Object { [string]$_.id -ceq "github" -and [string]$_.kind -ceq "github" })
    Assert-Condition ($relay.Count -eq 1 -and $relay[0].enabled -eq $true -and [string]$relay[0].base -ceq "https://update.404token.xyz/__codex_update__/skill-hub/") "Skill Hub Relay 来源身份或地址无效"
    Assert-Condition ($github.Count -eq 1 -and $github[0].enabled -eq $true -and $null -eq (Get-JsonPropertyValue $github[0] "base")) "Skill Hub GitHub 回退来源身份无效"
    $ids = @($sources | ForEach-Object { [string]$_.id })
    Assert-Condition (@($ids | Select-Object -Unique).Count -eq $ids.Count) "Skill Hub 来源 ID 重复"
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $sumPath = Join-Path $RootPath "SHA256SUMS.txt"
    Assert-Condition (Test-Path -LiteralPath $sumPath -PathType Leaf) "缺少 SHA256SUMS.txt，无法核对 Skill Hub 来源目录"
    $sumMatch = @(Get-Content -LiteralPath $sumPath | Where-Object {
        $_ -match "^$hash\s+resources/skill_hub_sources\.json$"
    })
    Assert-Condition ($sumMatch.Count -eq 1) "SHA256SUMS.txt 未覆盖当前 Skill Hub 来源目录"
}

function Assert-SkillHubBaseManifestResource([string]$RootPath) {
    $relative = "resources/skill_repo_base_manifest.json"
    $path = Join-Path $RootPath ($relative.Replace('/', '\'))
    Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) "缺少基础 Skill ID 清单：$relative"
    $item = Get-Item -LiteralPath $path -Force
    Assert-Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "基础 Skill ID 清单不能是链接或联接点：$relative"
    $base = Read-JsonFile $path
    $sourceRelative = "content/codex-starter-kit/manifest/skill-audit.json"
    $sourcePath = Join-Path $RootPath ($sourceRelative.Replace('/', '\'))
    Assert-Condition (Test-Path -LiteralPath $sourcePath -PathType Leaf) "缺少基础 Skill 源清单：$sourceRelative"
    $source = Read-JsonFile $sourcePath
    $sourceHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $baseSkills = @($base.skills)
    $sourceSkills = @($source.skills)
    Assert-Condition ((Get-JsonInteger $base "schemaVersion") -eq 1 -and
        [string]$base.source -ceq "installer-pack/manifest/skill-audit.json" -and
        [string]$base.sourceSha256 -ceq $sourceHash -and
        $baseSkills.Count -gt 0 -and $baseSkills.Count -eq $sourceSkills.Count) "基础 Skill ID 清单 schema/hash/数量无效"
    for ($index = 0; $index -lt $sourceSkills.Count; $index++) {
        Assert-Condition ([string]$baseSkills[$index].id -ceq [string]$sourceSkills[$index].id) "基础 Skill ID 顺序与源清单不一致：index=$index"
    }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $sumPath = Join-Path $RootPath "SHA256SUMS.txt"
    Assert-Condition (Test-Path -LiteralPath $sumPath -PathType Leaf) "缺少 SHA256SUMS.txt，无法核对基础 Skill ID 清单"
    $sumMatch = @(Get-Content -LiteralPath $sumPath | Where-Object {
        $_ -match "^$hash\s+resources/skill_repo_base_manifest\.json$"
    })
    Assert-Condition ($sumMatch.Count -eq 1) "SHA256SUMS.txt 未覆盖基础 Skill ID 清单"
}

# A directory or reparse point with the mutable license name must not be
# mistaken for the one authorized root metadata file.
$rootLicensePath = Join-Path $root "usb-license.json"
if (Test-Path -LiteralPath $rootLicensePath) {
    $rootLicenseItem = Get-Item -LiteralPath $rootLicensePath -Force
    Assert-Condition (-not $rootLicenseItem.PSIsContainer) "根目录 usb-license.json 必须是普通文件"
    Assert-Condition (($rootLicenseItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "根目录 usb-license.json 不能是链接或联接点"
}

# The license is the one mutable root metadata file.  Do not let a nested
# same-name file, an atomic-writer backup, or a copied license become an
# untracked authorization source.
$licenseEntries = @(Get-ChildItem -LiteralPath $root -Recurse -Force |
    Where-Object { $_.Name -ieq "usb-license.json" -or $_.Name -ieq "usb-license.json.bak" })
foreach ($licenseEntry in $licenseEntries) {
    $licenseRelative = (Get-RelativePathCompat $root $licenseEntry.FullName).Replace('\', '/')
    if ($licenseEntry.Name -ieq "usb-license.json.bak") {
        throw "禁止存在 usb-license.json.bak 残留：$licenseRelative"
    }
    if ($licenseRelative -ine "usb-license.json") {
        throw "usb-license.json 只能位于 bundle 根目录：$licenseRelative"
    }
}

$registrationEntries = @(Get-ChildItem -LiteralPath $root -Recurse -Force |
    Where-Object { $_.Name -ieq "usb-license-revocation-registration.json" })
foreach ($registrationEntry in $registrationEntries) {
    $registrationRelative = (Get-RelativePathCompat $root $registrationEntry.FullName).Replace('\', '/')
    Assert-Condition ($registrationRelative -ieq "usb-license-revocation-registration.json") "登记声明只能位于 bundle 根目录：$registrationRelative"
    Assert-Condition (-not $registrationEntry.PSIsContainer) "根登记声明必须是普通文件"
    Assert-Condition (($registrationEntry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "根登记声明不能是链接或联接点"
}
$registrationPath = Join-Path $root "usb-license-revocation-registration.json"
if (Test-Path -LiteralPath $registrationPath) {
    $registrationItem = Get-Item -LiteralPath $registrationPath -Force
    Assert-Condition (-not $registrationItem.PSIsContainer) "根登记声明必须是普通文件"
    Assert-Condition (($registrationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "根登记声明不能是链接或联接点"
}

function Assert-NoSuccessfulStateResidue([string]$RootPath) {
    foreach ($entry in Get-ChildItem -LiteralPath $RootPath -Recurse -Force) {
        $relative = (Get-RelativePathCompat $RootPath $entry.FullName).Replace('\', '/')
        foreach ($segment in $relative.Split('/')) {
            if ($segment -ieq '.staging' -or $segment -ieq 'staging' -or
                $segment -ieq '.transaction' -or $segment -ieq 'transaction' -or
                $segment -match '(?i)^transaction([.-]|$)' -or
                $segment -match '(?i)\.(bak|part|tmp)$') {
                throw "成功状态不得包含事务/临时残留：$relative"
            }
        }
    }
}

function Assert-AuthenticodePe([string]$Path, [string]$Label) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    Assert-Condition ($signature.Status -eq "Valid") "$Label Authenticode 状态不是 Valid：$($signature.Status)"
    Assert-Condition ($null -ne $signature.SignerCertificate -and
        -not [string]::IsNullOrWhiteSpace([string]$signature.SignerCertificate.Subject)) "$Label 缺少有效签名主体"
}

function Assert-CompiledUsbManagerBinding([string]$Path, [string]$KeyId, [string]$PublicKeyB64, [string]$PublicKeySha256) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "USB 管理器不存在，无法验证编译期签名绑定：$Path"
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    Assert-Condition ($ascii.IndexOf($KeyId, [System.StringComparison]::Ordinal) -ge 0) "USB 管理器未嵌入预期 active keyId：$KeyId"
    Assert-Condition ($ascii.IndexOf($PublicKeyB64.Trim(), [System.StringComparison]::Ordinal) -ge 0) "USB 管理器未嵌入与清单一致的公钥：publicKeySha256=$PublicKeySha256"
}

function Assert-CompiledUsbManagerPolicy([string]$Path, [string]$ExpectedPolicy) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "USB 管理器不存在，无法验证编译期策略绑定：$Path"
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    $marker = "codex-usb-compiled-policy:$ExpectedPolicy"
    Assert-Condition ($ascii.IndexOf($marker, [System.StringComparison]::Ordinal) -ge 0) "USB 管理器编译期策略与联合清单不一致：期望 $ExpectedPolicy；未找到标记 $marker"
}

function Assert-CompiledCcswitchHandoff([string]$Path, [string]$Label) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "$Label CCSwitch 不存在，无法验证 API Key 注入协议：$Path"
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    foreach ($marker in @(
        "CODEX_USB_404TOKEN_HANDOFF_V1",
        "CCSWITCH_404TOKEN_HANDOFF_PATH",
        "CCSWITCH_404TOKEN_HANDOFF_NONCE",
        "CCSWITCH_404TOKEN_DEVICE_KEY"
    )) {
        Assert-Condition ($ascii.IndexOf($marker, [System.StringComparison]::Ordinal) -ge 0) "$Label CCSwitch 缺少确认式 404token API Key 注入协议标记：$marker"
    }
}

$manifestPath = Join-Path $root "manifest.json"
$usbManifestPath = Join-Path $root "usb-manifest.json"
$sumsPath = Join-Path $root "SHA256SUMS.txt"
$payloadVerifierPath = Join-Path $root "verify-payload.ps1"
$launcherVerifierPath = Join-Path $root "verify-usb-launcher.ps1"
$manifest = Read-JsonFile $manifestPath
$usbManifest = Read-JsonFile $usbManifestPath

$manifestSchema = Assert-ManifestSchemaPair $manifest $usbManifest
$rootUpdateCapability = $manifest.updateCapability
Assert-Condition ($null -ne $rootUpdateCapability -and $rootUpdateCapability.codexPayload -eq $true) "根 manifest 未声明 Codex payload update capability"
Assert-Condition ((Get-JsonInteger $rootUpdateCapability "schemaVersion") -eq 3) "根 manifest 的 update capability 必须声明 schemaVersion=3"
Assert-Condition ((Get-JsonInteger $rootUpdateCapability "anchorSchema") -eq 3) "根 manifest 的 update capability 必须声明 anchorSchema=3"
Assert-Condition ($manifest.directEntryPolicy.required -eq $true) "根 manifest 未要求 U 盘 Codex 直接入口门禁"
Assert-Condition ($manifest.directEntryPolicy.shim -eq "CodexEntryShim.exe") "根 manifest direct-entry shim 身份不正确"
Assert-Condition ($manifest.directEntryPolicy.failure -eq "closed") "根 manifest 未声明直接入口失败时拒绝启动"
$updateCapability = $usbManifest.updateCapability
Assert-Condition ($null -ne $updateCapability -and $updateCapability.codexPayload -eq $true) "联合清单未声明 Codex payload update capability"
Assert-Condition ((Get-JsonInteger $updateCapability "schemaVersion") -eq 3) "update capability 必须声明 schemaVersion=3"
Assert-Condition ((Get-JsonInteger $updateCapability "anchorSchema") -eq 3) "update capability 必须声明 anchorSchema=3"
Assert-Condition ($updateCapability.checksumFile -eq "SHA256SUMS.txt") "update capability 的 SHA256 清单路径不正确"
Assert-Condition ($updateCapability.transactionDirectory -eq ".staging") "update capability 的事务目录不正确"
Assert-Condition ($updateCapability.backupPolicy -eq "single-msix-listed-in-SHA256SUMS") "update capability 的 backup 策略不正确"
Assert-Condition ($updateCapability.successState -eq "clean") "update capability 未声明成功状态清理策略"
Assert-Condition ($usbManifest.product -eq "codex-usb-oneclick") "不是 Codex 一键安装包"
Assert-Condition ($usbManifest.status -eq "oneclick") "联合清单状态不是 oneclick"
Assert-Condition ($usbManifest.secretsIncluded -eq $false) "U 盘包不应包含 API Key"
Assert-Condition ($usbManifest.automaticPostInstallLaunch -eq $true) "未声明安装后自动启动 CCSwitch"
$signingBinding = $usbManifest.usbGuard.signing
Assert-Condition ($null -ne $signingBinding) "联合清单缺少 usbGuard.signing 签名绑定声明"
$signingPolicy = [string]$signingBinding.policy
if ($InternalSigned -and $SkipSignature) {
    throw "InternalSigned 不能与 SkipSignature 同时使用"
}
if ($InternalSigned) {
    Assert-Condition ($signingPolicy -eq "SignedRequired") "InternalSigned 正式校验只接受 SignedRequired 签名策略"
}
if ($SkipSignature) {
    Assert-Condition ($signingPolicy -in @("SignedRequired", "DevelopmentAutoEnroll")) "usbGuard.signing.policy 无效：$signingPolicy"
} else {
    Assert-Condition ($signingPolicy -in @("SignedRequired", "DevelopmentAutoEnroll")) "正式校验只接受 SignedRequired 或 DevelopmentAutoEnroll 签名策略"
}
if ($signingPolicy -eq "SignedRequired") {
    if ([string]::IsNullOrWhiteSpace($UsbSigningProfilePath)) {
        throw "SignedRequired Bundle 缺少随包版本 signing profile；不会回退到仓库中的固定 profile"
    }
    $signingProfile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $UsbSigningProfilePath
    Initialize-CodexUsbSigningEnvironment -Profile $signingProfile
    $signingKeyId = [string]$signingBinding.keyId
    $signingDigest = [string]$signingBinding.publicKeySha256
    Assert-Condition ($signingKeyId -match '^[A-Za-z0-9._-]{1,128}$') "usbGuard.signing.keyId 无效"
    Assert-Condition ($signingDigest -match '^[0-9a-fA-F]{64}$') "usbGuard.signing.publicKeySha256 无效"
    $configuredKeyId = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_ID
    $configuredPublicKey = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_B64
    Assert-Condition (-not [string]::IsNullOrWhiteSpace($configuredKeyId) -and
        -not [string]::IsNullOrWhiteSpace($configuredPublicKey)) "SignedRequired 校验必须配置当前构建的 USB 公钥和 keyId"
    Assert-Condition ($configuredKeyId -eq $signingKeyId) "Bundle signing keyId 与当前构建配置不一致"
    try {
        $configuredPublicKeyBytes = [Convert]::FromBase64String($configuredPublicKey.Trim())
    } catch {
        throw "当前构建配置的 CODEX_USB_GUARD_PUBLIC_KEY_B64 不是有效 Base64"
    }
    Assert-Condition ($configuredPublicKeyBytes.Length -eq 32) "当前构建配置的 USB 公钥必须是 32 字节"
    $configuredHasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $configuredDigestBytes = $configuredHasher.ComputeHash($configuredPublicKeyBytes)
    } finally {
        $configuredHasher.Dispose()
    }
    $configuredDigest = (($configuredDigestBytes | ForEach-Object { $_.ToString('x2') }) -join '')
    Assert-Condition ($configuredDigest -ieq $signingDigest) "Bundle signing 公钥摘要与当前构建配置不一致"
    Assert-Condition ([string](Get-JsonPropertyValue $signingBinding "releaseVersion") -ceq [string]$signingProfile.ReleaseVersion) "Bundle signing releaseVersion 与随包 profile 不一致"
    Assert-Condition ([string](Get-JsonPropertyValue $signingBinding "releaseId") -ceq [string]$signingProfile.ReleaseId) "Bundle signing releaseId 与随包 profile 不一致"
}
Assert-Condition ($usbManifest.programLayout.mode -eq "verified-msix-extraction") "Codex 程序必须使用验签 MSIX 解包模式"
Assert-Condition ($usbManifest.programLayout.codexRoot -eq "apps/Codex") "Codex U 盘程序目录声明不正确"
Assert-Condition ($usbManifest.programLayout.usesAppxVolume -eq $false) "当前架构不得使用外部 AppXVolume"
Assert-Condition ($usbManifest.programLayout.createsWindowsApps -eq $false) "当前架构不得创建 WindowsApps"
Assert-Condition ($usbManifest.programLayout.createsWpSystem -eq $false) "当前架构不得创建 WpSystem"
Assert-Condition ($usbManifest.programLayout.directShellLaunch -eq $false) "不得创建自动生成的 Codex 开始菜单或桌面直启快捷方式"
Assert-Condition ($usbManifest.dataLayout.codexHome -eq "%USERPROFILE%/.codex") "CODEX_HOME 本机路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.roaming -eq "%APPDATA%/Codex") "Roaming 数据路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.local -eq "%LOCALAPPDATA%/Codex") "Local 数据路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.onUsb -eq $false) "可变 Codex 数据不得声明位于 U 盘"
Assert-Condition ($usbManifest.dataLayout.observedBeforeInstall -eq $true) "安装前必须核验数据实际卷"
Assert-Condition ($usbManifest.managedContent.skillsDirectory -eq "content/skills") "Skill U 盘目录声明不正确"
Assert-Condition ($usbManifest.managedContent.mcpDirectory -eq "content/mcp") "MCP U 盘目录声明不正确"
Assert-Condition ($usbManifest.managedContent.capabilityPackDirectory -eq "content/codex-starter-kit") "能力包 U 盘目录声明不正确"
Assert-Condition ($usbManifest.managedContent.configLocation -eq "%USERPROFILE%/.codex/config.toml") "MCP/Codex 配置必须保存在本机"
Assert-Condition ($usbManifest.managedContent.secretsIncluded -eq $false) "托管内容目录不得包含密钥"
Assert-Condition ($usbManifest.managedContent.status -eq "bundled-auto-installable") "能力包必须声明已随包提供并可自动安装"
Assert-Condition ($usbManifest.startupElevation.mode -eq "usb-launcher-requireAdministrator") "USB 入口必须声明 requireAdministrator 启动提权模式"
Assert-Condition ($usbManifest.startupElevation.mechanism -eq "embedded requireAdministrator manifest + standard Windows UAC consent") "USB 入口提权机制不是标准 Windows UAC"
Assert-Condition ($usbManifest.startupElevation.marker -eq "--codex-usb-elevated") "USB 入口提权标记不正确"
Assert-Condition ($usbManifest.startupElevation.onCancel -eq "exit-before-install") "UAC 取消时必须在安装前退出"
Assert-Condition ($usbManifest.startupElevation.silentBypass -eq $false) "USB 入口不允许静默绕过 UAC"
Assert-Condition ($usbManifest.startupElevation.executionPolicy -eq "process-local-bypass") "PowerShell 执行策略必须声明为进程级兼容参数"
Assert-Condition ($usbManifest.startupElevation.executionPolicyScope -eq "temporary PowerShell child only") "PowerShell 执行策略作用域不正确"
Assert-Condition ($usbManifest.ccswitch.launchMode -eq "usb-direct") "CCSwitch 必须声明 USB 直接启动"
Assert-Condition ($usbManifest.ccswitch.graphicsMode -eq "child-scoped-webview2-software-rendering") "CCSwitch 必须声明子进程级 WebView2 软件渲染"
Assert-Condition ($usbManifest.ccswitch.successPostcondition -eq "stable-sizeable-window") "CCSwitch 启动成功必须以稳定可操作窗口为后置条件"
Assert-Condition ($usbManifest.ccswitch.foregroundPolicy -eq "restore-clamp-to-work-area-and-present") "CCSwitch 必须声明窗口恢复、屏内钳制和呈现策略"
Assert-Condition ($usbManifest.codexLaunch.initialMode -eq "software-rendering") "USB Codex 首次启动必须声明软件渲染"
Assert-Condition (($usbManifest.codexLaunch.initialArguments -join " ") -eq "--disable-gpu") "USB Codex 初始兼容参数不正确"
Assert-Condition (($usbManifest.codexLaunch.fallbackArguments -join " ") -eq "--disable-gpu --no-sandbox") "USB Codex 末级兼容参数不正确"
Assert-Condition ($usbManifest.codexLaunch.fallbackOnlyAfterNoUsableWindow -eq $true) "no-sandbox 只能在可操作窗口验收失败后使用"
Assert-Condition ($usbManifest.codexLaunch.runtimeInitialMode -eq "portable") "USB Codex 实际首次启动必须使用常规渲染"
Assert-Condition (($usbManifest.codexLaunch.runtimeInitialArguments -join " ") -eq "") "USB Codex 实际首次启动不得强制 --disable-gpu"
Assert-Condition ($usbManifest.codexLaunch.runtimeFallbackMode -eq "software-rendering-no-sandbox") "USB Codex 实际兼容回退模式不正确"
Assert-Condition (($usbManifest.codexLaunch.runtimeFallbackArguments -join " ") -eq "--disable-gpu --no-sandbox") "USB Codex 实际兼容回退参数不正确"
Assert-Condition ($usbManifest.codexLaunch.runtimeFallbackOnlyAfterNoUsableWindow -eq $true) "USB Codex 实际兼容回退必须只在可操作窗口验收失败后使用"
Assert-Condition ($usbManifest.codexLaunch.successPostcondition -eq "stable-sizeable-window") "USB Codex 不得只以进程存活作为启动成功"
Assert-Condition ($usbManifest.codexLaunch.compatibilityMemory -eq "local-exact-codex-version") "USB Codex 兼容状态必须按精确版本保存在本机"
Assert-Condition ($usbManifest.codexLaunch.selfUpdate -eq "disabled-for-usb-child") "USB Codex 必须对子进程关闭原地自更新"
Assert-Condition ($usbManifest.codexLaunch.globalEnvironmentModified -eq $false) "启动兼容参数不得修改全局环境"
Assert-Condition ($usbManifest.usbGuard.localHelperRefresh -eq "length-and-sha256-before-new-lease") "本机看门狗必须在新租约前按哈希刷新"
if ($InternalSigned) {
    Assert-Condition ($usbManifest.usbGuard.verification.runtimeMode -eq "internal-signed-usb-license") "联合清单未声明 internal-signed-usb-license 运行期校验模式"
    Assert-Condition ($usbManifest.usbGuard.verification.productAuthenticode -eq "not-required-internal-license-mode") "内部授权模式不应把自有 EXE 声明为 Authenticode 必需"
    Assert-Condition ($usbManifest.usbGuard.verification.externalAuthenticode -eq "required-openai-msix-and-microsoft-prerequisites") "内部授权模式必须声明外部可信输入 Authenticode 必需"
}
Assert-Condition ((Get-JsonInteger $usbManifest.sourceMediaCleanup "planSchema") -eq 3) "安装介质二阶段计划版本不正确"
Assert-Condition ($usbManifest.sourceMediaCleanup.runtimeLicenseMode -eq "typed-manager-mutable-metadata") "运行期许可文件不得再被当作未知发行文件"
Assert-Condition ($usbManifest.sourceMediaCleanup.authorization -eq "active-lease-fingerprint+volume+immutable-manifest+license-snapshot") "安装介质清理授权证据不完整"
Assert-Condition ($usbManifest.sourceMediaCleanup.deleteOrder -eq "payload-first-license-and-retry-entry-last") "安装介质清理顺序不得提前删除重试身份"
Assert-Condition ($usbManifest.sourceMediaCleanup.unknownPolicy -eq "enumerate-and-preserve") "未知安装介质内容必须列名并保留"
Assert-Condition ($usbManifest.sourceMediaCleanup.usesRecycleBin -eq $false) "可移动介质清理不得依赖回收站"
Assert-Condition ($manifest.packageName -eq "OpenAI.Codex") "根 manifest 不是 Codex 离线安装清单"
$architecture = [string]$manifest.architecture
Assert-Condition ($architecture -in @("x64", "arm64")) "根 manifest 的 architecture 无效"
$payloadRelative = Normalize-BundleRelativePath ([string]$manifest.payload.path)
Assert-Condition ([System.IO.Path]::GetExtension($payloadRelative) -ieq ".msix") "根 manifest 的 payload.path 必须指向 .msix"
$payload = Resolve-BundleRelativeFile $root $payloadRelative
$payloadItem = Get-Item -LiteralPath $payload
$payloadDirectoryRelative = $payloadRelative -replace '/[^/]+$', ''
$backupDirectoryRelative = if ([string]::IsNullOrWhiteSpace($payloadDirectoryRelative)) {
    "_backup"
} else {
    "$payloadDirectoryRelative/_backup"
}
Assert-Condition ($updateCapability.backupDirectory -eq $backupDirectoryRelative) "update capability 的 backup 目录与 payload.path 不一致"
$backupRelative = $null
$backupDirectory = Join-Path $root ($backupDirectoryRelative.Replace('/', '\'))
if (Test-Path -LiteralPath $backupDirectory) {
    $backupDirectoryItem = Get-Item -LiteralPath $backupDirectory
    Assert-Condition ($backupDirectoryItem.PSIsContainer) "backup 路径必须是目录"
    Assert-Condition (-not (($backupDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0)) "backup 目录不能是链接或联接点"
    $backupEntries = @(Get-ChildItem -LiteralPath $backupDirectory -Force)
    Assert-Condition ($backupEntries.Count -le 1) "成功状态最多保留一份 payload backup"
    if ($backupEntries.Count -eq 1) {
        $backupEntry = $backupEntries[0]
        Assert-Condition (-not $backupEntry.PSIsContainer) "backup 目录只能包含一个文件"
        $backupPattern = '^OpenAI\.Codex_' + [regex]::Escape($architecture) + '_[A-Za-z0-9][A-Za-z0-9._-]*\.msix$'
        Assert-Condition ($backupEntry.Name -match $backupPattern) "payload backup 文件名或架构不正确"
        Assert-Condition ([int64]$backupEntry.Length -gt 0) "payload backup 不能为空"
        $backupRelative = (Get-RelativePathCompat $root $backupEntry.FullName).Replace('\', '/')
    }
}
Assert-NoSuccessfulStateResidue $root

$payloadArgs = @{ BundlePath = $root }
if ($SkipSignature) { $payloadArgs.SkipSignature = $true }
$payloadReport = & $payloadVerifierPath @payloadArgs | ConvertFrom-Json
Assert-Condition ($payloadReport.ready -eq $true) "MSIX 身份、架构、大小或内部清单校验失败"

$installer = Join-Path $root "CodexDesktopSetup.exe"
$manager = Join-Path $root "CodexAppManager.exe"
$entryShim = Join-Path $root "CodexEntryShim.exe"
$ccswitch = Join-Path $root "ccswitch\CodexSwitch.exe"
$embeddedCcswitch = Join-Path $root "resources\ccswitch\CodexSwitch.exe"
$vcRedist = Join-Path $root "resources\prerequisites\vc_redist.x64.exe"
$webView2 = Join-Path $root "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
$appsReadme = Join-Path $root "apps\README.txt"
$contentReadme = Join-Path $root "content\README.txt"
$skillsReadme = Join-Path $root "content\skills\README.txt"
$mcpReadme = Join-Path $root "content\mcp\README.txt"
$capabilityPack = Join-Path $root "content\codex-starter-kit"
$expectedWebView2Hash = "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D"
$expectedVcRedistHash = "843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C"
Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) "缺少 CodexDesktopSetup.exe"
Assert-Condition (Test-Path -LiteralPath $manager -PathType Leaf) "缺少 CodexAppManager.exe（USB 入口启动的管理器本体）"
Assert-Condition (Test-Path -LiteralPath $entryShim -PathType Leaf) "缺少 CodexEntryShim.exe（直接双击 Codex 必须经过此门禁）"
Assert-Condition (Test-Path -LiteralPath $launcherVerifierPath -PathType Leaf) "缺少 verify-usb-launcher.ps1"
Assert-Condition (Test-Path -LiteralPath $ccswitch -PathType Leaf) "缺少根目录 ccswitch\CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $embeddedCcswitch -PathType Leaf) "安装器资源中缺少 resources\ccswitch\CodexSwitch.exe"
Assert-CompiledCcswitchHandoff $ccswitch "USB 根目录"
Assert-CompiledCcswitchHandoff $embeddedCcswitch "安装器资源"
Assert-Condition (Test-Path -LiteralPath $vcRedist -PathType Leaf) "缺少 resources\prerequisites\vc_redist.x64.exe"
Assert-Condition (Test-Path -LiteralPath $webView2 -PathType Leaf) "缺少 WebView2 x64 完整离线安装包；空白机将无法显示安装界面"
Assert-SkillHubSourceResource $root
Assert-SkillHubBaseManifestResource $root
Assert-Condition (Test-Path -LiteralPath $appsReadme -PathType Leaf) "缺少 apps 程序目录契约"
Assert-Condition (Test-Path -LiteralPath $contentReadme -PathType Leaf) "缺少 content 托管内容目录契约"
Assert-Condition (Test-Path -LiteralPath $skillsReadme -PathType Leaf) "缺少 content/skills 目录契约"
Assert-Condition (Test-Path -LiteralPath $mcpReadme -PathType Leaf) "缺少 content/mcp 目录契约"
Assert-Condition (Test-Path -LiteralPath $capabilityPack -PathType Container) "缺少完整能力包目录 content/codex-starter-kit"
$capabilityPowerShellVerifier = Join-Path $PSScriptRoot "verify-capability-pack-powershell.ps1"
if (-not (Test-Path -LiteralPath $capabilityPowerShellVerifier -PathType Leaf)) {
    $capabilityPowerShellVerifier = Join-Path $PSScriptRoot "verification\verify-capability-pack-powershell.ps1"
}
Assert-Condition (Test-Path -LiteralPath $capabilityPowerShellVerifier -PathType Leaf) "缺少 Windows PowerShell 能力包编码/语法验证器"
$windowsPowerShell = Join-Path $env:WINDIR "System32\WindowsPowerShell\v1.0\powershell.exe"
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    $windowsPowerShellCommand = Get-Command powershell.exe -ErrorAction Stop
    $windowsPowerShell = $windowsPowerShellCommand.Source
}
$previousErrorActionPreference = $ErrorActionPreference
try {
    # Ignore stderr as a success/failure signal: stock Windows hosts may emit
    # provider initialization warnings before the verifier starts. The verifier
    # is fail-closed and its process exit code is authoritative.
    $ErrorActionPreference = 'Continue'
    $capabilityPowerShellCheck = & $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $capabilityPowerShellVerifier -Root $root 2>&1 | Out-String
    $capabilityPowerShellExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($capabilityPowerShellExitCode -ne 0) {
    throw "能力包 PowerShell 编码或 Windows PowerShell 5.1 语法检查失败：$($capabilityPowerShellCheck.Trim())"
}
Write-Output $capabilityPowerShellCheck.Trim()
Assert-CompiledUsbManagerPolicy $manager $signingPolicy
if ($signingPolicy -eq "SignedRequired") {
    Assert-CompiledUsbManagerBinding $manager $signingKeyId $configuredPublicKey $signingDigest
}
foreach ($packFile in @("pack.json", "dependencies.json", "file-manifest.json", "checksums.sha256", "install-all.ps1", "install-to-codex.ps1", "runtime\provision-runtime.ps1")) {
    Assert-Condition (Test-Path -LiteralPath (Join-Path $capabilityPack $packFile) -PathType Leaf) "能力包缺少 $packFile"
}
$packJson = Get-Content -LiteralPath (Join-Path $capabilityPack "pack.json") -Raw | ConvertFrom-Json
Assert-Condition ((Get-JsonInteger $packJson "schemaVersion") -eq 2 -and $packJson.id -eq "codex-starter") "能力包 pack.json 身份或 schema 不正确"
Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $root "WindowsApps"))) "发行包不得预建 WindowsApps"
Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $root "WpSystem"))) "发行包不得预建 WpSystem"
$actualWebView2Hash = (Get-FileHash -LiteralPath $webView2 -Algorithm SHA256).Hash
Assert-Condition ($actualWebView2Hash -ieq $expectedWebView2Hash) "WebView2 安装包哈希不匹配"
Assert-Condition ($usbManifest.webView2Runtime.file -eq "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe") "联合清单中的 WebView2 路径不正确"
Assert-Condition ($usbManifest.webView2Runtime.sha256 -ieq $expectedWebView2Hash) "联合清单中的 WebView2 哈希不正确"
Assert-Condition ((Get-JsonInteger $usbManifest.webView2Runtime "size") -eq (Get-Item -LiteralPath $webView2).Length) "联合清单中的 WebView2 大小不正确"
Assert-Condition ($usbManifest.webView2Runtime.beforeUi -eq $true) "WebView2 必须在创建安装界面前预检"
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $launcherVerification = & $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $launcherVerifierPath -Path $installer 2>&1 | Out-String
    $launcherVerificationExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($launcherVerificationExitCode -ne 0) {
    throw "USB 入口清单验证失败：$launcherVerification"
}
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $managerVerification = & $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $launcherVerifierPath -Path $manager -ExpectedLevel requireAdministrator 2>&1 | Out-String
    $managerVerificationExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($managerVerificationExitCode -ne 0) {
    throw "USB 管理器清单验证失败：$managerVerification"
}
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $entryShimVerification = & $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $launcherVerifierPath -Path $entryShim -ExpectedLevel asInvoker 2>&1 | Out-String
    $entryShimVerificationExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($entryShimVerificationExitCode -ne 0) {
    throw "USB Codex direct-entry shim 清单验证失败：$entryShimVerification"
}
$actualVcRedistHash = (Get-FileHash -LiteralPath $vcRedist -Algorithm SHA256).Hash
Assert-Condition ($actualVcRedistHash -ieq $expectedVcRedistHash) "VC++ 运行库哈希不匹配：$actualVcRedistHash != $expectedVcRedistHash"
Assert-Condition ($usbManifest.vcRuntime.file -eq "resources/prerequisites/vc_redist.x64.exe") "联合清单的 VC++ 运行库路径不正确"
Assert-Condition ($usbManifest.vcRuntime.sha256 -ieq $expectedVcRedistHash) "联合清单中的 VC++ 运行库哈希不正确"
Assert-Condition ((Get-JsonInteger $usbManifest.vcRuntime "size") -eq (Get-Item -LiteralPath $vcRedist).Length) "联合清单中的 VC++ 运行库大小不正确"

# Cross-check the generated metadata against the files that were just
# inspected. This catches a stale usb-manifest/SHA256SUMS pair after a partial
# copy or an interrupted release rebuild; checking only the root manifest can
# otherwise let a structurally valid but self-inconsistent bundle through.
$installerItem = Get-Item -LiteralPath $installer
$managerItem = Get-Item -LiteralPath $manager
$entryShimItem = Get-Item -LiteralPath $entryShim
$installerHash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$managerHash = (Get-FileHash -LiteralPath $manager -Algorithm SHA256).Hash.ToLowerInvariant()
$entryShimHash = (Get-FileHash -LiteralPath $entryShim -Algorithm SHA256).Hash.ToLowerInvariant()
$ccswitchItem = Get-Item -LiteralPath $ccswitch
$ccswitchHash = (Get-FileHash -LiteralPath $ccswitch -Algorithm SHA256).Hash.ToLowerInvariant()
$embeddedCcswitchHash = (Get-FileHash -LiteralPath $embeddedCcswitch -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-Condition ($usbManifest.codexInstaller.file -eq "CodexDesktopSetup.exe") "联合清单的入口文件名不正确"
Assert-Condition ($usbManifest.codexInstaller.sha256 -ieq $installerHash) "联合清单中的入口哈希与实际文件不一致"
Assert-Condition ((Get-JsonInteger $usbManifest.codexInstaller "size") -eq $installerItem.Length) "联合清单中的入口大小与实际文件不一致"
Assert-Condition ($usbManifest.codexInstaller.architecture -eq $architecture) "联合清单中的 MSIX 架构不正确"
Assert-Condition ($usbManifest.codexInstaller.payload -eq $payloadRelative) "联合清单的 MSIX 路径不正确"
Assert-Condition ($usbManifest.codexInstaller.payloadSha256 -ieq $payloadReport.sha256) "联合清单中的 MSIX 哈希与实际文件不一致"
Assert-Condition ((Get-JsonInteger $usbManifest.codexInstaller "payloadSize") -eq $payloadReport.size) "联合清单中的 MSIX 大小与实际文件不一致"
Assert-Condition ($usbManifest.manager.file -eq "CodexAppManager.exe") "联合清单中的管理器本体路径不正确"
Assert-Condition ($usbManifest.manager.role -eq "requireAdministrator USB Tauri manager started by the launcher or directly") "联合清单未声明 USB 管理器 requireAdministrator 边界"
Assert-Condition ($usbManifest.manager.sha256 -ieq $managerHash) "联合清单中的管理器本体哈希与实际文件不一致"
Assert-Condition ((Get-JsonInteger $usbManifest.manager "size") -eq $managerItem.Length) "联合清单中的管理器本体大小与实际文件不一致"
Assert-Condition ($usbManifest.entryShim.file -eq "CodexEntryShim.exe") "联合清单的 direct-entry shim 路径不正确"
Assert-Condition ($usbManifest.entryShim.sha256 -ieq $entryShimHash) "联合清单中的 direct-entry shim 哈希与实际文件不一致"
Assert-Condition ((Get-JsonInteger $usbManifest.entryShim "size") -eq $entryShimItem.Length) "联合清单中的 direct-entry shim 大小不正确"
Assert-Condition ($usbManifest.entryShim.role -eq "asInvoker Codex manifest-entry shim; every launch is handed to manager authorization") "联合清单未声明 direct-entry 授权链"
Assert-Condition ($usbManifest.ccswitch.executable -eq "ccswitch/CodexSwitch.exe") "联合清单的 CCSwitch 路径不正确"
Assert-Condition ($usbManifest.ccswitch.sha256 -ieq $ccswitchHash) "联合清单中的 CCSwitch 哈希与实际文件不一致"
Assert-Condition ((Get-JsonInteger $usbManifest.ccswitch "size") -eq $ccswitchItem.Length) "联合清单中的 CCSwitch 大小与实际文件不一致"
Assert-Condition ($usbManifest.ccswitch.embeddedExecutable -eq "resources/ccswitch/CodexSwitch.exe") "联合清单的嵌入 CCSwitch 路径不正确"
Assert-Condition ($embeddedCcswitchHash -ieq $ccswitchHash) "根目录与资源中的 CCSwitch 载荷不一致"
if (-not $SkipSignature) {
    if (-not $InternalSigned) {
        Assert-AuthenticodePe $installer "CodexDesktopSetup.exe"
        Assert-AuthenticodePe $manager "CodexAppManager.exe"
        Assert-AuthenticodePe $entryShim "CodexEntryShim.exe"
        Assert-AuthenticodePe $ccswitch "CCSwitch"
        Assert-AuthenticodePe $embeddedCcswitch "embedded CCSwitch"
    }
    $webView2Signature = Get-AuthenticodeSignature -LiteralPath $webView2
    Assert-Condition ($webView2Signature.Status -eq "Valid") "WebView2 安装包 Authenticode 签名无效"
    Assert-Condition ($webView2Signature.SignerCertificate.Subject -match "Microsoft Corporation") "WebView2 安装包签名者不是 Microsoft Corporation"
    $signature = Get-AuthenticodeSignature -LiteralPath $vcRedist
    Assert-Condition ($signature.Status -eq "Valid") "VC++ 运行库 Authenticode 签名无效：$($signature.Status)"
    Assert-Condition ($signature.SignerCertificate.Subject -match "Microsoft Corporation") "VC++ 运行库签名者不是 Microsoft Corporation"
}

$failed = @()
$listed = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
Assert-Condition (Test-Path -LiteralPath $sumsPath -PathType Leaf) "缺少 SHA256SUMS.txt"
foreach ($line in Get-Content -LiteralPath $sumsPath) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line -split '\s+', 2
    Assert-Condition ($parts.Count -eq 2) "SHA256SUMS.txt 格式错误：$line"
    $expected = $parts[0].Trim()
    Assert-Condition ($expected -match '^[0-9A-Fa-f]{64}$') "SHA256SUMS.txt 哈希格式错误：$line"
    $relative = $parts[1].Trim()
    $relative = Normalize-BundleRelativePath $relative
    Assert-Condition ($relative -ine "SHA256SUMS.txt") "SHA256SUMS.txt 不得列出自身"
    Assert-Condition ($relative -ine "usb-license.json") "SHA256SUMS.txt 不得列出运行期 usb-license.json"
    Assert-Condition (-not (Test-RevocationRuntimePath $relative)) "SHA256SUMS.txt 不得列出登记声明或 revocation runtime metadata：$relative"
    $leafName = [System.IO.Path]::GetFileName($relative)
    Assert-Condition ($leafName -inotmatch '(?i)^revocation\.json(?:\.tmp-.*)?$') "revocation state 名称只能出现在精确运行期路径：$relative"
    if (-not $listed.Add($relative)) {
        $failed += "$relative（SHA256SUMS.txt 重复清单项）"
        continue
    }
    $file = Join-Path $root ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        $failed += "$relative（文件不存在）"
        continue
    }
    $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected.ToLowerInvariant()) {
        $failed += "$relative（哈希不匹配）"
    }
}
if ($null -ne $backupRelative) {
    Assert-Condition ($listed.Contains($backupRelative)) "允许保留的 payload backup 必须列入 SHA256SUMS.txt：$backupRelative"
}
# A stale file left by an interrupted/repeated build must not be silently
# accepted: it could contain an older payload that the manifest never covers.
foreach ($fileItem in Get-ChildItem -LiteralPath $root -File -Recurse -Force) {
    if ($fileItem.Name -ieq "SHA256SUMS.txt") { continue }
    $relative = (Get-RelativePathCompat $root $fileItem.FullName).Replace('\', '/')
    # The manager-created license is mutable runtime identity metadata and is
    # deliberately outside the immutable release checksum set.
    if ($relative -ieq "usb-license.json") { continue }
    if (Test-RevocationRuntimePath $relative) {
        Assert-Condition (-not $fileItem.PSIsContainer) "revocation runtime metadata 必须是普通文件：$relative"
        Assert-Condition (($fileItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) "revocation runtime metadata 不能是链接或联接点：$relative"
        continue
    }
    $leafName = [System.IO.Path]::GetFileName($relative)
    Assert-Condition ($leafName -inotmatch '(?i)^revocation\.json(?:\.tmp-.*)?$') "revocation state 名称只能出现在精确运行期路径：$relative"
    if (-not $listed.Contains($relative)) {
        $failed += "$relative（未列入 SHA256SUMS.txt）"
    }
}
Assert-Condition ($failed.Count -eq 0) ($failed -join '; ')

$ccswitchVerifier = Join-Path $root "ccswitch\verify-product.ps1"
Assert-Condition (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf) "缺少 CCSwitch 验证脚本"
$args = @{ ArtifactDir = (Join-Path $root "ccswitch"); SkipNetwork = $SkipNetwork; PromptForApiKey = $PromptForApiKey }
if ($ApiKey) { $args.ApiKey = $ApiKey }
& $ccswitchVerifier @args

[pscustomobject]@{
    ready = $true
    mode = "oneclick"
    installer = $installer
    ccswitch = $ccswitch
    embeddedCcswitch = $embeddedCcswitch
    vcRuntime = $vcRedist
    webView2Runtime = $webView2
    codexPayload = $payloadReport
} | ConvertTo-Json -Depth 8
