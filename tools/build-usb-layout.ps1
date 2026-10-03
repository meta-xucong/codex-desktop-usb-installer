[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [string]$BundlePath = "",
    [string]$OutputPath = "",
    [string]$CapabilityPackSource = "",
    [switch]$InternalSigned,
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = Join-Path $repo "release\usb" }
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $repo "release\usb-final" }
if ([string]::IsNullOrWhiteSpace($CapabilityPackSource)) {
    $CapabilityPackSource = Join-Path $repo "installer-pack"
}
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path

# Windows PowerShell 5.1/.NET Framework does not provide
# [System.IO.Path]::GetRelativePath (the release builder is intentionally
# usable on a stock Windows installation). Keep the fallback root-scoped so a
# future output-path change cannot make the checksum list point outside the
# generated bundle.
function Get-RelativePathCompat([string]$BasePath, [string]$FilePath) {
    $base = [System.IO.Path]::GetFullPath($BasePath).TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath($FilePath)
    if (-not $full.StartsWith($base, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "文件不在 U 盘输出目录内：$FilePath"
    }
    return $full.Substring($base.Length)
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

function Read-JsonFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "缺少 JSON 清单：$Path"
    }
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8) | ConvertFrom-Json
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

function Assert-NoSuccessfulStateResidue([string]$RootPath) {
    $rootLicensePath = Join-Path $RootPath "usb-license.json"
    if (Test-Path -LiteralPath $rootLicensePath) {
        throw "静态发行目录不得携带 usb-license.json；请在真实 U 盘复制完成后使用签名工具写入运行期授权"
    }
    foreach ($entry in Get-ChildItem -LiteralPath $RootPath -Recurse -Force) {
        $relative = (Get-RelativePathCompat $RootPath $entry.FullName).Replace('\', '/')
        if ($entry.Name -ieq "usb-license.json" -and $relative -ine "usb-license.json") {
            throw "usb-license.json 只能出现在 U 盘 bundle 根目录：$relative"
        }
        if ($entry.Name -ieq "usb-license.json.bak") {
            throw "U 盘 bundle 不得包含 usb-license.json.bak 残留：$relative"
        }
        $forbidden = $false
        foreach ($segment in $relative.Split('/')) {
            if ($segment -ieq '.staging' -or $segment -ieq 'staging' -or
                $segment -ieq '.transaction' -or $segment -ieq 'transaction' -or
                $segment -match '(?i)^transaction([.-]|$)' -or
                $segment -match '(?i)\.(bak|part|tmp)$') {
                $forbidden = $true
                break
            }
        }
        if ($forbidden) {
            throw "构建输出包含成功状态不允许的事务/临时残留：$relative"
        }
    }
}

function Assert-AuthenticodePe([string]$Path, [string]$Label) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne "Valid") {
        throw "$Label Authenticode 状态不是 Valid：$($signature.Status)"
    }
    if ($null -eq $signature.SignerCertificate -or
        [string]::IsNullOrWhiteSpace([string]$signature.SignerCertificate.Subject)) {
        throw "$Label 缺少有效签名主体"
    }
}

$sourceManifest = Read-JsonFile (Join-Path $bundle "manifest.json")
$architecture = [string]$sourceManifest.architecture
if ($architecture -notin @("x64", "arm64")) {
    throw "manifest.architecture 无效：$architecture"
}
$payloadRelative = Normalize-BundleRelativePath ([string]$sourceManifest.payload.path)
if (([System.IO.Path]::GetExtension($payloadRelative) -ine ".msix")) {
    throw "manifest.payload.path 必须指向 .msix 文件：$payloadRelative"
}
$sourcePayload = Join-Path $bundle ($payloadRelative.Replace('/', '\'))
if (-not (Test-Path -LiteralPath $sourcePayload -PathType Leaf)) {
    throw "manifest.payload.path 指向的载荷不存在：$sourcePayload"
}
$sourcePayload = (Resolve-Path -LiteralPath $sourcePayload).Path
$bundleFull = [System.IO.Path]::GetFullPath($bundle).TrimEnd('\') + '\'
$sourcePayloadFull = [System.IO.Path]::GetFullPath($sourcePayload)
if (-not $sourcePayloadFull.StartsWith($bundleFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "manifest.payload.path 不得逃逸 bundle 根目录：$payloadRelative"
}
$payloadDirectoryRelative = $payloadRelative -replace '/[^/]+$', ''
$backupDirectory = if ([string]::IsNullOrWhiteSpace($payloadDirectoryRelative)) {
    "_backup"
} else {
    "$payloadDirectoryRelative/_backup"
}
$installerItem = Get-Item -LiteralPath $installer
if ($installerItem.PSIsContainer -or $installerItem.Extension -ine ".exe") {
    throw "InstallerPath 必须是可直接运行的 Tauri 应用 EXE，而不是目录或其他文件"
}
if ($installerItem.Name -match "-setup\.exe$") {
    throw "不能把 NSIS -setup.exe 当作 U 盘入口；请传入 build-installer.ps1 生成的 CodexUsbLauncher.exe"
}
$isUsbLauncher = $false
if ($installerItem.Name -ieq "CodexUsbLauncher.exe" -or $installerItem.Name -ieq "CodexDesktopSetup.exe") {
    $launcherVerifier = Join-Path $PSScriptRoot "verify-usb-launcher.ps1"
    if (-not (Test-Path -LiteralPath $launcherVerifier -PathType Leaf)) {
        throw "缺少 USB 入口清单验证脚本：$launcherVerifier"
    }
    $launcherCheck = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path $installer 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) {
        $isUsbLauncher = $true
        Write-Output $launcherCheck.Trim()
    } elseif ($installerItem.Name -ieq "CodexUsbLauncher.exe") {
        throw "CodexUsbLauncher.exe 未通过 embedded requireAdministrator 清单验证：$launcherCheck"
    }
}
& (Join-Path $PSScriptRoot "verify-payload.ps1") -BundlePath $bundle -SkipSignature:$SkipSignature

$managerSource = Join-Path $installerItem.DirectoryName "CodexAppManager.exe"
if ($isUsbLauncher) {
    if (-not (Test-Path -LiteralPath $managerSource -PathType Leaf)) {
        throw "CodexUsbLauncher.exe 必须与同目录 CodexAppManager.exe 一起构建"
    }
    if (-not $SkipSignature -and -not $InternalSigned) {
        Assert-AuthenticodePe $installer "CodexUsbLauncher.exe"
        Assert-AuthenticodePe $managerSource "CodexAppManager.exe"
    }
}

$outputFull = [System.IO.Path]::GetFullPath($OutputPath).TrimEnd('\')
$volumeRoot = [System.IO.Path]::GetPathRoot($outputFull).TrimEnd('\')
foreach ($unsafeRoot in @($volumeRoot, $repo.TrimEnd('\'), (Split-Path -Parent $repo).TrimEnd('\'), $bundle.TrimEnd('\'), $installerItem.DirectoryName.TrimEnd('\'))) {
    if ($outputFull -ieq $unsafeRoot) { throw "输出目录不能覆盖磁盘根目录、工作区或输入目录：$outputFull" }
}
if ((Test-Path -LiteralPath $outputFull) -and
    @(Get-ChildItem -LiteralPath $outputFull -Force -ErrorAction Stop).Count -gt 0) {
    throw "发布输出目录必须为空；请使用新的版本目录，避免残留旧 EXE、资源或许可证：$outputFull"
}
$output = (New-Item -ItemType Directory -Force -Path $outputFull).FullName
if ((Get-Item -LiteralPath $output).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
    throw "U 盘输出目录不能是链接或联接点：$output"
}
New-Item -ItemType Directory -Force -Path (Join-Path $output "payload") | Out-Null
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
foreach ($directory in @("apps", "content", "content\skills", "content\mcp")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $output $directory) | Out-Null
}

# Ship the complete capability pack in the generated USB layout. The manager
# installs this local tree after Codex verification, so the USB workflow does
# not depend on GitHub, npm, PyPI, or a second installer window.
$capabilityPackSourceFull = (Resolve-Path -LiteralPath $CapabilityPackSource -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $capabilityPackSourceFull -PathType Container)) {
    throw "能力包源目录不存在：$CapabilityPackSource"
}
foreach ($requiredPackFile in @(
    "pack.json",
    "dependencies.json",
    "file-manifest.json",
    "checksums.sha256",
    "install-all.ps1",
    "install-to-codex.ps1",
    "runtime\provision-runtime.ps1"
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $capabilityPackSourceFull $requiredPackFile) -PathType Leaf)) {
        throw "能力包缺少必要文件：$(Join-Path $capabilityPackSourceFull $requiredPackFile)"
    }
}
$packReparse = @(Get-ChildItem -LiteralPath $capabilityPackSourceFull -Recurse -Force |
    Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 } |
    Select-Object -First 1)
if ($packReparse.Count -gt 0) {
    throw "能力包包含符号链接或重解析点，拒绝复制：$($packReparse[0].FullName)"
}
$capabilityPackOutput = Join-Path $output "content\codex-starter-kit"
New-Item -ItemType Directory -Force -Path $capabilityPackOutput | Out-Null
$packTransientDirectories = @('.git', '.pytest_cache', '__pycache__')
foreach ($packFile in Get-ChildItem -LiteralPath $capabilityPackSourceFull -Recurse -Force -File) {
    $packRelative = (Get-RelativePathCompat $capabilityPackSourceFull $packFile.FullName).Replace('\', '/')
    $packSegments = $packRelative.Split('/')
    if (@($packSegments | Where-Object { $packTransientDirectories -contains $_ }).Count -gt 0 -or
        $packFile.Extension -ieq '.pyc') {
        continue
    }
    $packDestination = Join-Path $capabilityPackOutput ($packRelative.Replace('/', '\'))
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $packDestination) | Out-Null
    Copy-Item -LiteralPath $packFile.FullName -Destination $packDestination -Force
}
[System.IO.File]::WriteAllText(
    (Join-Path $output "apps\README.txt"),
    "此目录由安装器管理。Codex 程序将解包到 apps\Codex；账号、会话、缓存和配置不写入这里。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\README.txt"),
    "U 盘托管内容目录。Skill 与 MCP 的程序/资源可放在对应子目录；API Key、登录凭证和用户配置不得放入 U 盘。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\skills\README.txt"),
    "兼容性内容目录。完整能力包位于同级 codex-starter-kit，安装器会将其中的 Skills 导入当前用户；用户数据仍保存在本机。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\mcp\README.txt"),
    "兼容性 MCP 资源目录。完整能力包位于同级 codex-starter-kit；MCP 配置写入本机 %USERPROFILE%\.codex\config.toml，密钥不得写入 U 盘。`r`n",
    $utf8NoBom
)
Copy-Item -LiteralPath $installer -Destination (Join-Path $output "CodexDesktopSetup.exe") -Force
if ($isUsbLauncher) {
    Copy-Item -LiteralPath $managerSource -Destination (Join-Path $output "CodexAppManager.exe") -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-usb-launcher.ps1") -Destination (Join-Path $output "verify-usb-launcher.ps1") -Force
}
$resourceSource = Join-Path $installerItem.DirectoryName "resources"
# Reject resource drift before copying a stale provider default into either
# the embedded resources/ccswitch or the root ccswitch copy.
$providerSource = Join-Path $resourceSource "ccswitch\providers.json"
if (Test-Path -LiteralPath (Join-Path $resourceSource "ccswitch") -PathType Container) {
    & (Join-Path $PSScriptRoot "verify-model-provider-resource.ps1") -ArtifactPath $providerSource
}

if (Test-Path -LiteralPath $resourceSource -PathType Container) {
    Copy-Item -LiteralPath $resourceSource -Destination $output -Recurse -Force
    # Keep a user-visible root-level copy for the validation/USB layout.  The
    # installer command can also resolve resources/ccswitch when this folder
    # is embedded by Tauri, but the root copy makes the fallback manual launch
    # and post-install diagnostics obvious to a non-technical user.
    $ccswitchSource = Join-Path $resourceSource "ccswitch"
    if (Test-Path -LiteralPath $ccswitchSource -PathType Container) {
        $ccswitchOutput = Join-Path $output "ccswitch"
        # The output directory is intentionally a generated release folder.
        # Remove only its previous root-level CCSwitch directory before
        # copying, otherwise repeated builds can create ccswitch\ccswitch.
        if (Test-Path -LiteralPath $ccswitchOutput -PathType Container) {
            $resolvedCcswitchOutput = (Resolve-Path -LiteralPath $ccswitchOutput).Path
            if ($resolvedCcswitchOutput -ine (Join-Path $output "ccswitch") -or
                ((Get-Item -LiteralPath $ccswitchOutput).Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
                throw "拒绝清理输出目录之外的 CCSwitch 或链接目录：$ccswitchOutput"
            }
            Remove-Item -LiteralPath $ccswitchOutput -Recurse -Force
        }
        Copy-Item -LiteralPath $ccswitchSource -Destination $ccswitchOutput -Recurse -Force
    }
}
$vcRedistOutput = Join-Path $output "resources\prerequisites\vc_redist.x64.exe"
if (-not (Test-Path -LiteralPath $vcRedistOutput -PathType Leaf)) {
    throw "安装器资源中缺少离线 VC++ x64 运行库：$vcRedistOutput"
}
$webView2Output = Join-Path $output "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
if (-not (Test-Path -LiteralPath $webView2Output -PathType Leaf)) {
    throw "安装器资源中缺少 WebView2 x64 完整离线安装包：$webView2Output"
}
[void](Add-Member -InputObject $sourceManifest -MemberType NoteProperty -Name "schemaVersion" -Value 2 -Force)
[void](Add-Member -InputObject $sourceManifest -MemberType NoteProperty -Name "updateCapability" -Value ([ordered]@{
    codexPayload = $true
    schemaVersion = 3
    anchorSchema = 3
    mutableFields = @(
        "codexDesktopVersion",
        "packageVersion",
        "releaseTag",
        "payload/sha256",
        "payload/size"
    )
    checksumFile = "SHA256SUMS.txt"
    backupDirectory = $backupDirectory
    backupPolicy = "single-msix-listed-in-SHA256SUMS"
    transactionDirectory = ".staging"
    successState = "clean"
}) -Force)
if ((Get-JsonInteger $sourceManifest "schemaVersion") -ne 2 -or
    (Get-JsonInteger $sourceManifest.updateCapability "schemaVersion") -ne 3 -or
    (Get-JsonInteger $sourceManifest.updateCapability "anchorSchema") -ne 3) {
    throw "输出 root manifest 的 schema/capability 数字字段无效"
}
[System.IO.File]::WriteAllText(
    (Join-Path $output "manifest.json"),
    ($sourceManifest | ConvertTo-Json -Depth 8),
    $utf8NoBom
)
$outputPayload = Join-Path $output ($payloadRelative.Replace('/', '\'))
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $outputPayload) | Out-Null
Copy-Item -LiteralPath $sourcePayload -Destination $outputPayload -Force

Assert-NoSuccessfulStateResidue $output

$lines = Get-ChildItem -LiteralPath $output -File -Recurse -Force |
    Sort-Object -Property FullName |
    Where-Object {
        $relative = (Get-RelativePathCompat $output $_.FullName).Replace('\', '/')
        $_.Name -ne "SHA256SUMS.txt" -and $relative -ine "usb-license.json"
    } |
    ForEach-Object {
        $relative = Get-RelativePathCompat $output $_.FullName
        $relative = $relative.Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)
Write-Output "已生成 U 盘目录：$output"
