# CCSwitch 源码克隆与构建脚本
# 用途: 从 GitHub 克隆指定 commit 的 CCSwitch 源码,应用环境变量补丁,并构建新版本

param(
    [string]$TargetDir = "D:\AI\AGENT_U盘\release\ccswitch-source",
    [string]$Commit = "1d44b1ba41987cddeb8096fadba8190fab465cbb",
    [switch]$SkipClone,
    [switch]$BuildOnly
)

$ErrorActionPreference = "Stop"

Write-Host "=== CCSwitch 源码克隆与构建 ===" -ForegroundColor Cyan
Write-Host ""

# 步骤 1: 克隆源码
if (-not $SkipClone -and -not $BuildOnly) {
    Write-Host "[1/5] 克隆 GitHub 源码..." -ForegroundColor Yellow

    $parentDir = Split-Path -Parent $TargetDir
    if (-not (Test-Path $parentDir)) {
        New-Item -ItemType Directory -Path $parentDir -Force | Out-Null
        Write-Host "  创建目录: $parentDir" -ForegroundColor Gray
    }

    if (Test-Path $TargetDir) {
        Write-Host "  目标目录已存在,删除旧版本..." -ForegroundColor Gray
        Remove-Item -Path $TargetDir -Recurse -Force
    }

    try {
        git clone https://github.com/farion1231/cc-switch.git $TargetDir 2>&1 | Out-Null
        Write-Host "  ✓ 克隆成功" -ForegroundColor Green
    } catch {
        Write-Host "  ✗ 克隆失败: $_" -ForegroundColor Red
        exit 1
    }

    # 步骤 2: 切换到指定 commit
    Write-Host "[2/5] 切换到 commit $Commit..." -ForegroundColor Yellow
    Push-Location $TargetDir
    try {
        git checkout $Commit 2>&1 | Out-Null
        $actualCommit = git log -1 --format="%H"
        Write-Host "  当前 commit: $actualCommit" -ForegroundColor Gray

        if ($actualCommit -ne $Commit) {
            Write-Host "  ✗ Commit 不匹配!" -ForegroundColor Red
            Pop-Location
            exit 1
        }
        Write-Host "  ✓ Commit 验证通过" -ForegroundColor Green
    } catch {
        Write-Host "  ✗ 切换失败: $_" -ForegroundColor Red
        Pop-Location
        exit 1
    }
    Pop-Location
} else {
    Write-Host "[1/5] 跳过克隆步骤" -ForegroundColor Gray
}

# 步骤 3: 应用环境变量补丁
if (-not $BuildOnly) {
    Write-Host "[3/5] 应用环境变量补丁..." -ForegroundColor Yellow

    $codexAdapterPath = Join-Path $TargetDir "src-tauri\src\proxy\providers\codex.rs"

    if (-not (Test-Path $codexAdapterPath)) {
        Write-Host "  ✗ 找不到文件: $codexAdapterPath" -ForegroundColor Red
        exit 1
    }

    # 读取文件内容
    $content = Get-Content -Path $codexAdapterPath -Raw

    # 检查是否已经应用补丁
    if ($content -match "CCSWITCH_404TOKEN_DEVICE_KEY") {
        Write-Host "  补丁已存在,跳过应用" -ForegroundColor Gray
    } else {
        # 查找 extract_key 函数
        if ($content -match "(?s)(fn extract_key\([^)]+\)[^{]*\{)") {
            $functionStart = $matches[1]

            # 构建补丁代码
            $patch = @"
$functionStart
        // USB 环境变量补丁 - 从管理器注入的设备 Key
        if provider.id == "404token-sub2api" {
            if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
                if !key.trim().is_empty() {
                    return Some(key);
                }
            }
        }

"@

            # 应用补丁
            $patchedContent = $content -replace [regex]::Escape($functionStart), $patch
            Set-Content -Path $codexAdapterPath -Value $patchedContent -NoNewline

            Write-Host "  ✓ 补丁应用成功" -ForegroundColor Green
        } else {
            Write-Host "  ✗ 找不到 extract_key 函数" -ForegroundColor Red
            exit 1
        }
    }
} else {
    Write-Host "[3/5] 跳过补丁步骤" -ForegroundColor Gray
}

# Always refresh the embedded renderer before the release Rust build.  A
# copied/stale dist directory otherwise makes the packaged Tauri window fall
# back to its localhost devUrl on a blank machine.
if (Test-Path -LiteralPath (Join-Path $TargetDir "package.json") -PathType Leaf) {
    Write-Host "[3.5/5] 构建 production renderer..." -ForegroundColor Yellow
    & pnpm.cmd --dir $TargetDir run build:renderer
    if ($LASTEXITCODE -ne 0) { throw "CCSwitch renderer 构建失败" }
    $rendererIndex = Join-Path $TargetDir "dist\index.html"
    $rendererAssets = Join-Path $TargetDir "dist\assets"
    if (-not (Test-Path -LiteralPath $rendererIndex -PathType Leaf) -or
        -not (Test-Path -LiteralPath $rendererAssets -PathType Container) -or
        @(Get-ChildItem -LiteralPath $rendererAssets -File -ErrorAction SilentlyContinue).Count -eq 0) {
        throw "CCSwitch production renderer 缺少 dist/index.html 或 dist/assets 产物"
    }
    Write-Host "  ✓ production renderer 已生成" -ForegroundColor Green
}

# 步骤 4: 验证编译
Write-Host "[4/5] 验证 Rust 编译..." -ForegroundColor Yellow
Push-Location $TargetDir

try {
    cargo check --locked 2>&1 | Out-Null
    Write-Host "  ✓ 编译检查通过" -ForegroundColor Green
} catch {
    Write-Host "  ✗ 编译检查失败: $_" -ForegroundColor Red
    Pop-Location
    exit 1
}

# 步骤 5: Release 构建
Write-Host "[5/5] 构建 Release 版本 (预计 10-15 分钟)..." -ForegroundColor Yellow
Write-Host "  开始时间: $(Get-Date -Format 'HH:mm:ss')" -ForegroundColor Gray

try {
    cargo build --release --locked
    Write-Host "  ✓ 构建完成" -ForegroundColor Green
    Write-Host "  结束时间: $(Get-Date -Format 'HH:mm:ss')" -ForegroundColor Gray
} catch {
    Write-Host "  ✗ 构建失败: $_" -ForegroundColor Red
    Pop-Location
    exit 1
}

Pop-Location

# 计算 SHA256
Write-Host ""
Write-Host "=== 构建产物信息 ===" -ForegroundColor Cyan
$exePath = Join-Path $TargetDir "target\release\CodexSwitch.exe"

if (Test-Path $exePath) {
    $hash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash
    $size = (Get-Item -LiteralPath $exePath).Length

    Write-Host "EXE 路径: $exePath" -ForegroundColor White
    Write-Host "SHA256:   $hash" -ForegroundColor White
    Write-Host "大小:     $([math]::Round($size / 1MB, 2)) MB" -ForegroundColor White
    Write-Host ""

    # 创建产物目录
    $artifactDir = "D:\AI\AGENT_U盘\release\ccswitch-product"
    if (-not (Test-Path $artifactDir)) {
        New-Item -ItemType Directory -Path $artifactDir -Force | Out-Null
    }

    # 复制产物
    Copy-Item -LiteralPath $exePath -Destination "$artifactDir\CodexSwitch.exe" -Force
    Write-Host "✓ 产物已复制到: $artifactDir\CodexSwitch.exe" -ForegroundColor Green
    Write-Host ""

    # 输出下一步指令
    Write-Host "=== 下一步操作 ===" -ForegroundColor Cyan
    Write-Host "1. 更新 manifest.json 中的 SHA256 为:" -ForegroundColor Yellow
    Write-Host "   $hash" -ForegroundColor White
    Write-Host ""
    Write-Host "2. 替换管理器中的 CCSwitch.exe:" -ForegroundColor Yellow
    Write-Host "   Copy-Item -Path '$artifactDir\CodexSwitch.exe' -Destination 'D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\resources\ccswitch\CodexSwitch.exe' -Force" -ForegroundColor Gray
    Write-Host ""
    Write-Host "3. 运行验证脚本:" -ForegroundColor Yellow
    Write-Host "   .\tools\verify-product.ps1" -ForegroundColor Gray

} else {
    Write-Host "✗ 找不到构建产物: $exePath" -ForegroundColor Red
    exit 1
}
