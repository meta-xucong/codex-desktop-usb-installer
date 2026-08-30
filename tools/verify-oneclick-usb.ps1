[CmdletBinding()]
param(
    [string]$ApiKey,
    [switch]$PromptForApiKey,
    [switch]$SkipNetwork
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "验证失败：$Message" }
}

function Read-JsonFile([string]$Path) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "缺少 $Path"
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

$manifestPath = Join-Path $root "manifest.json"
$usbManifestPath = Join-Path $root "usb-manifest.json"
$sumsPath = Join-Path $root "SHA256SUMS.txt"
$payloadVerifierPath = Join-Path $root "verify-payload.ps1"
$manifest = Read-JsonFile $manifestPath
$usbManifest = Read-JsonFile $usbManifestPath

Assert-Condition ($usbManifest.product -eq "codex-usb-oneclick") "不是 Codex 一键安装包"
Assert-Condition ($usbManifest.status -eq "oneclick") "联合清单状态不是 oneclick"
Assert-Condition ($usbManifest.secretsIncluded -eq $false) "U 盘包不应包含 API Key"
Assert-Condition ($usbManifest.automaticPostInstallLaunch -eq $true) "未声明安装后自动启动 CCSwitch"
Assert-Condition ($manifest.packageName -eq "OpenAI.Codex") "根 manifest 不是 Codex 离线安装清单"
Assert-Condition ($manifest.payload.path -eq "payload/OpenAI.Codex_x64.msix") "根 manifest 的 MSIX 路径不正确"

$payloadReport = & $payloadVerifierPath -BundlePath $root -SkipSignature | ConvertFrom-Json
Assert-Condition ($payloadReport.ready -eq $true) "MSIX 身份、架构、大小或内部清单校验失败"

$installer = Join-Path $root "CodexDesktopSetup.exe"
$ccswitch = Join-Path $root "ccswitch\CodexSwitch.exe"
$embeddedCcswitch = Join-Path $root "resources\ccswitch\CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) "缺少 CodexDesktopSetup.exe"
Assert-Condition (Test-Path -LiteralPath $ccswitch -PathType Leaf) "缺少根目录 ccswitch\CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $embeddedCcswitch -PathType Leaf) "安装器资源中缺少 resources\ccswitch\CodexSwitch.exe"

$failed = @()
Assert-Condition (Test-Path -LiteralPath $sumsPath -PathType Leaf) "缺少 SHA256SUMS.txt"
foreach ($line in Get-Content -LiteralPath $sumsPath) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line -split '\s+', 2
    Assert-Condition ($parts.Count -eq 2) "SHA256SUMS.txt 格式错误：$line"
    $relative = $parts[1].Trim()
    $file = Join-Path $root ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        $failed += "$relative（文件不存在）"
        continue
    }
    $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $parts[0].ToLowerInvariant()) {
        $failed += "$relative（哈希不匹配）"
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
    codexPayload = $payloadReport
} | ConvertTo-Json -Depth 8
