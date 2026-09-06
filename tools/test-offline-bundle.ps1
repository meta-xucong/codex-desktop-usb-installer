[CmdletBinding()]
param(
    [string]$InstallerPath = "",
    [switch]$KeepArtifacts
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if ([string]::IsNullOrWhiteSpace($InstallerPath)) { $InstallerPath = Join-Path $PSScriptRoot "..\release\setup\CodexDesktopSetup.exe" }

function Assert-Condition {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) { throw $Message }
}

function New-FakeMsix {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    # Windows PowerShell 5.1 does not always load the enum assembly when only
    # the FileSystem facade is requested; load both pieces explicitly so the
    # synthetic MSIX fixture works on a stock Windows installation.
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::Open(
        $Path,
        [System.IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $entry = $archive.CreateEntry("AppxManifest.xml")
        $writer = [System.IO.StreamWriter]::new(
            $entry.Open(),
            [System.Text.UTF8Encoding]::new($false)
        )
        try {
            $writer.Write(@"
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI" Version="26.602.3474.0" ProcessorArchitecture="x64" />
  <Applications>
    <Application Id="App" Executable="app/Codex.exe" EntryPoint="Windows.FullTrustApplication" />
  </Applications>
</Package>
"@.Trim())
        }
        finally { $writer.Dispose() }

        $binary = $archive.CreateEntry("app/Codex.exe")
        $stream = $binary.Open()
        try {
            $bytes = [System.Text.Encoding]::ASCII.GetBytes("synthetic fixture")
            $stream.Write($bytes, 0, $bytes.Length)
        }
        finally { $stream.Dispose() }
    }
    finally { $archive.Dispose() }
}

function Read-Sha256Sums {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $records = @{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $match = [regex]::Match($line, '^([0-9a-fA-F]{64})\s{2}(.+)$')
        Assert-Condition $match.Success "SHA256SUMS.txt 行格式无效：$line"
        $records[$match.Groups[2].Value] = $match.Groups[1].Value.ToLowerInvariant()
    }
    return $records
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$installerItem = Get-Item -LiteralPath $installer
Assert-Condition (-not $installerItem.PSIsContainer) "InstallerPath 不能是目录"
Assert-Condition ($installerItem.Extension -ieq ".exe") "InstallerPath 必须是 EXE"
Assert-Condition ($installerItem.Name -notmatch "-setup\.exe$") "测试入口不能是 NSIS -setup.exe"

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("CodexDesktopUsbSimulation-" + [guid]::NewGuid().ToString("N"))
$sourceDir = Join-Path $tempRoot "source"
$bundleDir = Join-Path $tempRoot "usb"
$outputDir = Join-Path $tempRoot "usb-final"

try {
    New-Item -ItemType Directory -Force -Path $sourceDir, $bundleDir, $outputDir | Out-Null
    $sourcePath = Join-Path $sourceDir "OpenAI.Codex-test.msix"
    New-FakeMsix -Path $sourcePath
    Assert-Condition ((Get-Item -LiteralPath $sourcePath).Length -gt 0) "无法生成合成 MSIX"

    & (Join-Path $PSScriptRoot "collect-payload.ps1") `
        -SourcePath $sourcePath `
        -CodexDesktopVersion "26.602.40724" `
        -PackageVersion "26.602.3474.0" `
        -OutputDir $bundleDir `
        -SkipSignature

    $verifyJson = & (Join-Path $PSScriptRoot "verify-payload.ps1") `
        -BundlePath $bundleDir `
        -SkipSignature | Out-String
    $verify = $verifyJson | ConvertFrom-Json
    Assert-Condition ($verify.ready -eq $true) "合成载荷没有通过 verify-payload"
    Assert-Condition ($verify.packageName -eq "OpenAI.Codex") "合成载荷身份不正确"
    Assert-Condition ($verify.architecture -eq "x64") "合成载荷架构不正确"
    Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$verify.appEntry)) "合成载荷没有桌面入口"

    & (Join-Path $PSScriptRoot "build-usb-layout.ps1") `
        -InstallerPath $installer `
        -BundlePath $bundleDir `
        -OutputPath $outputDir `
        -SkipSignature

    $requiredFiles = @(
        "CodexDesktopSetup.exe",
        "CodexAppManager.exe",
        "manifest.json",
        "SHA256SUMS.txt",
        "payload/OpenAI.Codex_x64.msix",
        "apps/README.txt",
        "content/README.txt",
        "content/skills/README.txt",
        "content/mcp/README.txt",
        "resources/prerequisites/vc_redist.x64.exe",
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
    )
    foreach ($relativePath in $requiredFiles) {
        $candidate = Join-Path $outputDir ($relativePath.Replace('/', '\'))
        Assert-Condition (Test-Path -LiteralPath $candidate -PathType Leaf) "U 盘目录缺少 $relativePath"
    }
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $outputDir "WindowsApps"))) "便携布局不得预建 WindowsApps"
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $outputDir "WpSystem"))) "便携布局不得预建 WpSystem"

    $manifest = Get-Content -LiteralPath (Join-Path $outputDir "manifest.json") -Raw | ConvertFrom-Json
    Assert-Condition ($manifest.productVersion -eq "0.1.0") "输出 manifest 的 productVersion 不正确"
    Assert-Condition ($manifest.codexDesktopVersion -eq "26.602.40724") "输出 manifest 的 Codex 版本不正确"
    Assert-Condition ($manifest.packageName -eq "OpenAI.Codex") "输出 manifest 的包身份不正确"
    Assert-Condition ($manifest.architecture -eq "x64") "输出 manifest 的架构不正确"

    $sumPath = Join-Path $outputDir "SHA256SUMS.txt"
    $sums = Read-Sha256Sums -Path $sumPath
    Assert-Condition ($sums.Count -ge 4) "SHA256SUMS.txt 没有覆盖完整 U 盘目录"
    foreach ($relativePath in $sums.Keys) {
        $candidate = Join-Path $outputDir ($relativePath.Replace('/', '\'))
        Assert-Condition (Test-Path -LiteralPath $candidate -PathType Leaf) "SHA256SUMS 引用了不存在的文件 $relativePath"
        $actual = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Condition ($actual -eq $sums[$relativePath]) "文件哈希不匹配：$relativePath"
    }

    if ($KeepArtifacts) {
        $saved = Join-Path $repoRoot "release\simulation"
        if (Test-Path -LiteralPath $saved) {
            Remove-Item -LiteralPath $saved -Recurse -Force
        }
        Copy-Item -LiteralPath $outputDir -Destination $saved -Recurse -Force
        $savedPath = (Resolve-Path -LiteralPath $saved).Path
    }

    $result = [ordered]@{
        passed = $true
        simulation = "synthetic-msix-no-signature"
        installer = $installer
        outputFiles = @(Get-ChildItem -LiteralPath $outputDir -File -Recurse).Count
        checksumEntries = $sums.Count
        note = "已覆盖合成 MSIX 的收集、清单、SHA-256、U 盘程序/内容布局和逐文件复核；确认不预建 WindowsApps/WpSystem。未覆盖真实 Authenticode 与便携 Codex 启动。"
    }
    if ($KeepArtifacts) { $result.savedOutput = $savedPath }
    $result | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
