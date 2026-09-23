[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BundlePath,
    [string]$SignerCliPath = "",
    [string]$SignerGuiPath = "",
    [string]$ProfilePath = "",
    [switch]$SkipLocalSigner
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $repo "tools\usb-signing-profile.ps1")

if ([string]::IsNullOrWhiteSpace($ProfilePath)) {
    throw "正式签名 pair preflight 必须显式传入当前版本 ProfilePath；不会读取仓库中的固定 profile"
}

function Read-JsonFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing JSON file: $Path"
    }
    try {
        return Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
    } catch {
        throw "Cannot read JSON file: $Path; $($_.Exception.Message)"
    }
}

function Get-JsonPropertyValue($Object, [string]$Name) {
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Assert-Same([string]$Label, [string]$Expected, [string]$Actual) {
    if ([string]::IsNullOrWhiteSpace($Actual) -or $Actual -cne $Expected) {
        throw "$Label mismatch: actual=$Actual; expected=$Expected"
    }
}

function Assert-SignerPackage([string]$GuiPath, $Profile) {
    $signerDirectory = Split-Path -Parent $GuiPath
    $manifestPath = Join-Path $signerDirectory "manifest.json"
    $profilePath = Join-Path $signerDirectory "usb-signing-release-profile.json"
    $vaultPath = Join-Path $signerDirectory "codex-usb-signing-key.vault"
    $vaultDescriptorPath = "$vaultPath.public.json"
    $manifest = Read-JsonFile $manifestPath
    if ([bool](Get-JsonPropertyValue $manifest "portableVaultIncluded") -ne $true -or
        [bool](Get-JsonPropertyValue $manifest "portableVaultBindingVerified") -ne $true) {
        throw "正式签名器必须携带已绑定的 portable vault；拒绝依赖某台电脑 DPAPI 的引导包：$manifestPath"
    }
    if ([bool](Get-JsonPropertyValue $manifest "targetDesktopOnly") -eq $true) {
        throw "签名器 manifest 仍声明 targetDesktopOnly；正式交付必须是跨电脑 portable vault 模式：$manifestPath"
    }
    if ([bool](Get-JsonPropertyValue $manifest "adminApiKeyEmbedded") -ne $true -or
        [string](Get-JsonPropertyValue $manifest "adminApiKeySource") -cne "compile-time-embedded") {
        throw "正式签名器必须内置管理员 Key，不能要求操作员手填或依赖旁置配置：$manifestPath"
    }
    if ([bool](Get-JsonPropertyValue $manifest "sub2ApiConfigIncluded") -ne $false) {
        throw "正式签名器不得发布包含管理员 Key 的 sub2api-config.json：$manifestPath"
    }
    $legacyConfigPath = Join-Path $signerDirectory "sub2api-config.json"
    if (Test-Path -LiteralPath $legacyConfigPath) {
        throw "正式签名器目录发现已禁止的旁置 sub2api-config.json：$legacyConfigPath"
    }
    if (-not (Test-Path -LiteralPath $vaultPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $vaultDescriptorPath -PathType Leaf)) {
        throw "签名器缺少 portable vault 或其公共绑定描述：$signerDirectory"
    }
    $packageBinding = $manifest.expectedSigningBinding
    if ($null -eq $packageBinding) {
        throw "Signer package manifest 缺少 expectedSigningBinding：$manifestPath"
    }
    Assert-Same "Signer package keyId" $Profile.KeyId ([string](Get-JsonPropertyValue $packageBinding "keyId"))
    Assert-Same "Signer package publicKeySha256" $Profile.PublicKeySha256 ([string](Get-JsonPropertyValue $packageBinding "publicKeySha256")).ToLowerInvariant()
    Assert-Same "Signer package releaseVersion" $Profile.ReleaseVersion ([string](Get-JsonPropertyValue $packageBinding "releaseVersion"))
    Assert-Same "Signer package releaseId" $Profile.ReleaseId ([string](Get-JsonPropertyValue $packageBinding "releaseId"))
    $packagedProfile = Read-JsonFile $profilePath
    Assert-Same "Packaged profile keyId" $Profile.KeyId ([string](Get-JsonPropertyValue $packagedProfile "keyId"))
    Assert-Same "Packaged profile publicKeySha256" $Profile.PublicKeySha256 ([string](Get-JsonPropertyValue $packagedProfile "publicKeySha256")).ToLowerInvariant()
    Assert-Same "Packaged profile releaseVersion" $Profile.ReleaseVersion ([string](Get-JsonPropertyValue $packagedProfile "releaseVersion"))
    Assert-Same "Packaged profile releaseId" $Profile.ReleaseId ([string](Get-JsonPropertyValue $packagedProfile "releaseId"))
}

$profile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $ProfilePath -RequireExplicitPath
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path
$usbManifest = Read-JsonFile (Join-Path $bundle "usb-manifest.json")
$binding = $usbManifest.usbGuard.signing
if ($null -eq $binding) {
    throw "Bundle is missing usb-manifest.json/usbGuard/signing; formal signing is blocked"
}

Assert-Same "Bundle keyId" $profile.KeyId ([string](Get-JsonPropertyValue $binding "keyId"))
Assert-Same "Bundle publicKeySha256" $profile.PublicKeySha256 ([string](Get-JsonPropertyValue $binding "publicKeySha256")).ToLowerInvariant()
Assert-Same "Bundle releaseVersion" $profile.ReleaseVersion ([string](Get-JsonPropertyValue $binding "releaseVersion"))
Assert-Same "Bundle releaseId" $profile.ReleaseId ([string](Get-JsonPropertyValue $binding "releaseId"))

$signerResult = $null
$signerSource = $null
if (-not $SkipLocalSigner) {
    if (-not [string]::IsNullOrWhiteSpace($SignerGuiPath)) {
        $gui = (Resolve-Path -LiteralPath $SignerGuiPath).Path
        Assert-SignerPackage -GuiPath $gui -Profile $profile
        # A formal GUI package is portable by contract.  Its encrypted vault
        # is the signing-key source; do not launch --diagnose-key here because
        # that command intentionally reports the unrelated host DPAPI key.
        $signerResult = [pscustomobject]@{
            keyId = $profile.KeyId
            publicKeySha256 = $profile.PublicKeySha256
        }
        $signerSource = "portable-vault:$gui"
    } else {
        if ([string]::IsNullOrWhiteSpace($SignerCliPath)) {
            $SignerCliPath = Join-Path $repo "tools\usb-license-signer\target\release\usb-license-signer.exe"
        }
        $signer = (Resolve-Path -LiteralPath $SignerCliPath).Path
        $raw = @(& $signer init-key 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw "Signer DPAPI environment check failed: $($raw -join ' ')"
        }
        try {
            $signerResult = ($raw -join [Environment]::NewLine) | ConvertFrom-Json
        } catch {
            throw "Signer init-key did not return parseable JSON: $($_.Exception.Message)"
        }
        $signerSource = "cli:$signer"
    }
    Assert-Same "Signer keyId" $profile.KeyId ([string]$signerResult.keyId)
    Assert-Same "Signer publicKeySha256" $profile.PublicKeySha256 ([string]$signerResult.publicKeySha256).ToLowerInvariant()
}

[ordered]@{
    ready = $true
    bundle = $bundle
    keyId = $profile.KeyId
    publicKeySha256 = $profile.PublicKeySha256
    releaseVersion = $profile.ReleaseVersion
    releaseId = $profile.ReleaseId
    localSignerChecked = (-not $SkipLocalSigner) -and [string]::IsNullOrWhiteSpace($SignerGuiPath)
    portableVaultChecked = (-not $SkipLocalSigner) -and -not [string]::IsNullOrWhiteSpace($SignerGuiPath)
    signerSource = $signerSource
} | ConvertTo-Json -Depth 4
