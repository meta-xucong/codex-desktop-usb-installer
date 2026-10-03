# CCSwitch 环境变量补丁构建脚本
# 用途：从 GitHub 克隆指定 commit,应用产品化改造和环境变量补丁,构建新版本

[CmdletBinding()]
param(
    [string]$SourceDir = "",
    [string]$OutputDir = "",
    [switch]$SkipClone,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

# 默认源码目录和输出目录
if ([string]::IsNullOrWhiteSpace($SourceDir)) {
    $SourceDir = Join-Path (Split-Path -Parent $repoRoot) "release\ccswitch-product-source"
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path (Split-Path -Parent $repoRoot) "release\ccswitch-product"
}

Write-Host "================================================" -ForegroundColor Cyan
Write-Host "CCSwitch 环境变量补丁构建脚本" -ForegroundColor Cyan
Write-Host "源码目录: $SourceDir" -ForegroundColor Yellow
Write-Host "输出目录: $OutputDir" -ForegroundColor Yellow
Write-Host "================================================" -ForegroundColor Cyan
Write-Host ""

# 步骤 1: 克隆源码
if (-not $SkipClone) {
    Write-Host "[步骤 1/6] 克隆 CCSwitch 源码..." -ForegroundColor Green

    if (Test-Path -LiteralPath $SourceDir) {
        Write-Warning "源码目录已存在: $SourceDir"
        $response = Read-Host "是否删除并重新克隆? (y/N)"
        if ($response -eq 'y' -or $response -eq 'Y') {
            Remove-Item -LiteralPath $SourceDir -Recurse -Force
        } else {
            Write-Host "跳过克隆,使用现有源码" -ForegroundColor Yellow
            $SkipClone = $true
        }
    }

    if (-not $SkipClone) {
        New-Item -ItemType Directory -Force -Path $SourceDir | Out-Null
        Push-Location $SourceDir
        try {
            git clone https://github.com/farion1231/cc-switch.git .
            if ($LASTEXITCODE -ne 0) { throw "git clone 失败" }

            git checkout 1d44b1ba41987cddeb8096fadba8190fab465cbb
            if ($LASTEXITCODE -ne 0) { throw "git checkout 失败" }

            $currentCommit = git log -1 --oneline
            Write-Host "当前 commit: $currentCommit" -ForegroundColor Cyan
        } finally {
            Pop-Location
        }
    }
    Write-Host "✓ 源码准备完成" -ForegroundColor Green
    Write-Host ""
} else {
    Write-Host "[步骤 1/6] 跳过克隆,使用现有源码" -ForegroundColor Yellow
    if (-not (Test-Path -LiteralPath $SourceDir)) {
        throw "源码目录不存在: $SourceDir"
    }
    Write-Host ""
}

$cargoProjectDir = if (Test-Path -LiteralPath (Join-Path $SourceDir "Cargo.toml") -PathType Leaf) {
    $SourceDir
} elseif (Test-Path -LiteralPath (Join-Path $SourceDir "src-tauri\Cargo.toml") -PathType Leaf) {
    Join-Path $SourceDir "src-tauri"
} else {
    throw "源码目录中未找到 Cargo.toml 或 src-tauri\\Cargo.toml：$SourceDir"
}

# 步骤 2: 检查并应用产品化改造
Write-Host "[步骤 2/6] 检查产品化改造..." -ForegroundColor Green

$resourceSourceDir = Join-Path $repoRoot "src-tauri\resources\ccswitch"
# The repository resource is authoritative. A stale source-tree catalog must
# never override the resource used by the manager's capability gate.
$providersJsonPath = Join-Path $resourceSourceDir "providers.json"
& (Join-Path $PSScriptRoot "verify-model-provider-resource.ps1") -SourcePath $providersJsonPath

Write-Host ""

# 步骤 3: 应用环境变量补丁
Write-Host "[步骤 3/6] 应用环境变量补丁..." -ForegroundColor Green

$codexAdapterPath = Join-Path $cargoProjectDir "src\proxy\providers\codex.rs"
if (-not (Test-Path -LiteralPath $codexAdapterPath)) {
    Write-Warning "未找到 codex.rs,路径可能不同"
    Write-Host "请手动定位 CodexAdapter::extract_key 函数并添加补丁" -ForegroundColor Yellow
} else {
    $codexContent = Get-Content -LiteralPath $codexAdapterPath -Raw

    if ($codexContent -match "CCSWITCH_404TOKEN_DEVICE_KEY") {
        Write-Host "✓ 环境变量补丁已存在" -ForegroundColor Green
    } else {
        Write-Host "正在添加环境变量补丁..." -ForegroundColor Yellow

        # 查找 extract_key 函数
        if ($codexContent -match "fn extract_key") {
            # 创建补丁内容
            $patchContent = @'
    // USB 环境变量读取补丁 (G39)
    if provider.id == "404token-sub2api" {
        if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
    }

'@

            # 在 extract_key 函数开始处插入补丁
            $updatedContent = $codexContent -replace "(fn extract_key[^\{]*\{)", "`$1`n$patchContent"

            Set-Content -LiteralPath $codexAdapterPath -Value $updatedContent -Encoding UTF8
            Write-Host "✓ 环境变量补丁已添加" -ForegroundColor Green
        } else {
            Write-Warning "未找到 extract_key 函数,请手动添加补丁"
            Write-Host "补丁代码:" -ForegroundColor Gray
            Write-Host @'
if provider.id == "404token-sub2api" {
    if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
}
'@ -ForegroundColor Gray
        }
    }
}
Write-Host ""

# The environment adapter alone only makes the key available to CCSwitch. It
# does not prove the key reached its database or the active Codex provider.
# Require the product-side persistence/switch/readback acknowledgement before
# building anything that the manager may advertise as auto-configured.
$providerServicePath = Join-Path $cargoProjectDir "src\services\provider\mod.rs"
if (-not (Test-Path -LiteralPath $providerServicePath -PathType Leaf)) {
    throw "CCSwitch 源码缺少 provider service：$providerServicePath"
}
$providerServiceText = [System.IO.File]::ReadAllText($providerServicePath)
if (-not $providerServiceText.Contains("CODEX_USB_404TOKEN_HANDOFF_V1")) {
    throw "CCSwitch 源码缺少确认式 API Key 持久化与回读协议；拒绝构建旧版自动注入实现"
}

# A release CCSwitch must contain the production renderer.  Running only
# `cargo build --release` can reuse a stale `dist` directory (or, in a copied
# source tree, leave the Tauri devUrl as the only usable page), which presents
# to users as a packaged window showing `localhost` connection refused.  Build
# the renderer explicitly and fail closed if the generated entry/assets are
# missing before compiling the Rust shell.
$rendererProjectDir = Split-Path -Parent $cargoProjectDir
if (Test-Path -LiteralPath (Join-Path $rendererProjectDir "package.json") -PathType Leaf) {
    Write-Host "[步骤 3.5/6] 构建 CCSwitch production renderer..." -ForegroundColor Green
    Push-Location $rendererProjectDir
    try {
        & pnpm.cmd run build:renderer
        if ($LASTEXITCODE -ne 0) { throw "CCSwitch renderer 构建失败" }
    } finally {
        Pop-Location
    }
    $rendererIndex = Join-Path $rendererProjectDir "dist\index.html"
    $rendererAssets = Join-Path $rendererProjectDir "dist\assets"
    if (-not (Test-Path -LiteralPath $rendererIndex -PathType Leaf) -or
        -not (Test-Path -LiteralPath $rendererAssets -PathType Container) -or
        @(Get-ChildItem -LiteralPath $rendererAssets -File -ErrorAction SilentlyContinue).Count -eq 0) {
        throw "CCSwitch production renderer 缺少 dist/index.html 或 dist/assets 产物"
    }
    Write-Host "✓ production renderer 已生成并通过静态门禁" -ForegroundColor Green
}

# 步骤 4: 验证编译
Write-Host "[步骤 4/6] 验证编译..." -ForegroundColor Green

Push-Location $cargoProjectDir
try {
    cargo check --locked
    if ($LASTEXITCODE -ne 0) {
        throw "cargo check 失败,请检查代码"
    }
    Write-Host "✓ 编译检查通过" -ForegroundColor Green
} finally {
    Pop-Location
}
Write-Host ""

# 步骤 5: Release 构建
if (-not $SkipBuild) {
    Write-Host "[步骤 5/6] Release 构建..." -ForegroundColor Green
    Write-Host "这可能需要 10-15 分钟..." -ForegroundColor Yellow

    Push-Location $rendererProjectDir
    try {
        $startTime = Get-Date
        # `cargo build --release` only compiles the Rust shell.  A production
        # Tauri build is required here so the current renderer is embedded in
        # the executable instead of leaving a stale devUrl/localhost window.
        & pnpm.cmd tauri build --no-bundle --ci
        if ($LASTEXITCODE -ne 0) {
            throw "tauri production build 失败"
        }
        $elapsed = (Get-Date) - $startTime
        Write-Host "✓ 构建完成 (耗时: $($elapsed.TotalMinutes.ToString('0.0')) 分钟)" -ForegroundColor Green
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[步骤 5/6] 跳过构建" -ForegroundColor Yellow
}
Write-Host ""

# 步骤 6: 准备产物
Write-Host "[步骤 6/6] 准备产物..." -ForegroundColor Green

$builtExeCandidates = @(
    (Join-Path $cargoProjectDir "target\release\cc-switch.exe"),
    (Join-Path $cargoProjectDir "target\release\CodexSwitch.exe")
)
$builtExePath = $builtExeCandidates |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if ([string]::IsNullOrWhiteSpace([string]$builtExePath)) {
    throw "未找到构建产物，已检查：$($builtExeCandidates -join ', ')"
}
$binaryText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($builtExePath))
if (-not $binaryText.Contains("CODEX_USB_404TOKEN_HANDOFF_V1") -or
    -not $binaryText.Contains("CCSWITCH_404TOKEN_DEVICE_KEY")) {
    throw "构建产物缺少 404token 确认式注入协议标记；可能选中了旧缓存 EXE：$builtExePath"
}

# 计算 SHA256
$newSha256 = (Get-FileHash -LiteralPath $builtExePath -Algorithm SHA256).Hash
Write-Host "新 EXE SHA256: $newSha256" -ForegroundColor Cyan

# 创建输出目录
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

# 复制产物
Copy-Item -LiteralPath $builtExePath -Destination "$OutputDir\CodexSwitch.exe" -Force
Write-Host "✓ EXE 已复制到: $OutputDir\CodexSwitch.exe" -ForegroundColor Green

# 复制 provider catalog 和发行说明。上游源码有时不携带产品化 catalog，
# 此时使用仓库中已经过审计的静态资源，确保该脚本单独运行也能生成完整产物。
if (Test-Path -LiteralPath $providersJsonPath) {
    Copy-Item -LiteralPath $providersJsonPath -Destination "$OutputDir\providers.json" -Force
    Write-Host "✓ providers.json 已复制" -ForegroundColor Green
}
foreach ($fileName in @("LICENSE", "SOURCE-NOTICE.txt", "verify-product.ps1")) {
    $sourceFile = Join-Path $resourceSourceDir $fileName
    if (Test-Path -LiteralPath $sourceFile -PathType Leaf) {
        Copy-Item -LiteralPath $sourceFile -Destination (Join-Path $OutputDir $fileName) -Force
        Write-Host "✓ $fileName 已复制" -ForegroundColor Green
    }
}

# 复制或更新 manifest.json
$currentManifestPath = Join-Path $repoRoot "src-tauri\resources\ccswitch\manifest.json"
$newManifestPath = Join-Path $OutputDir "manifest.json"

if (Test-Path -LiteralPath $currentManifestPath) {
    $manifest = Get-Content -LiteralPath $currentManifestPath -Raw | ConvertFrom-Json
    $manifest.artifact.sha256 = $newSha256
    $manifest.notes = @(
        "404TOKEN is a third-party Sub2API relay, not an OpenAI official endpoint.",
        "A verified USB license injects one device API key for the unified-api-internal group at runtime.",
        "CCSwitch saves the USB key to its normal private provider database, switches the provider, and returns a nonce-bound acknowledgement only after local readback succeeds.",
        "The USB manager reports injection success only after validating that acknowledgement; on failure it keeps the CCSwitch window visible and shows an error on its home page.",
        "Model discovery does not gate normal provider activation or Codex launch.",
        "The device key is never included in this static resource bundle or providers.json.",
        "The Windows MSI/NSIS wrapper is not included in this P0 artifact."
    )
    $manifestJson = $manifest | ConvertTo-Json -Depth 10
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($newManifestPath, $manifestJson, $utf8NoBom)
    $manifestBytes = [System.IO.File]::ReadAllBytes($newManifestPath)
    if ($manifestBytes.Length -ge 3 -and
        $manifestBytes[0] -eq 0xEF -and
        $manifestBytes[1] -eq 0xBB -and
        $manifestBytes[2] -eq 0xBF) {
        throw "manifest.json 写入后仍包含 UTF-8 BOM：$newManifestPath"
    }
    Write-Host "✓ manifest.json 已更新 (SHA256 已更新)" -ForegroundColor Green
}

$productVerifier = Join-Path $OutputDir "verify-product.ps1"
if (-not (Test-Path -LiteralPath $productVerifier -PathType Leaf)) {
    throw "CCSwitch 产物缺少 verify-product.ps1：$productVerifier"
}
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $productVerifier -ArtifactDir $OutputDir -SkipNetwork
if ($LASTEXITCODE -ne 0) { throw "CCSwitch 输出产物未通过确认式 API Key 注入门禁" }

Write-Host ""
Write-Host "================================================" -ForegroundColor Cyan
Write-Host "构建完成!" -ForegroundColor Green
Write-Host "================================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "产物位置: $OutputDir" -ForegroundColor Yellow
Write-Host "新 SHA256: $newSha256" -ForegroundColor Yellow
Write-Host ""
Write-Host "下一步:" -ForegroundColor Cyan
Write-Host "1. 使用新产物构建管理器:" -ForegroundColor White
Write-Host "   .\tools\build-installer.ps1 -CcswitchArtifactDir `"$OutputDir`" -SkipTests -UsbPolicy DevelopmentAutoEnroll" -ForegroundColor Gray
Write-Host ""
Write-Host "2. 或者直接复制到资源目录并更新 manifest:" -ForegroundColor White
Write-Host "   Copy-Item `"$OutputDir\CodexSwitch.exe`" `"$repoRoot\src-tauri\resources\ccswitch\CodexSwitch.exe`" -Force" -ForegroundColor Gray
Write-Host "   # 然后手动更新 src-tauri\resources\ccswitch\manifest.json 中的 SHA256" -ForegroundColor Gray
Write-Host ""
