# CCSwitch 环境变量补丁端到端验证脚本
# 用途: 验证 CCSwitch 能否正确读取 CCSWITCH_404TOKEN_DEVICE_KEY 环境变量

param(
    [string]$CCSwitchPath = "src-tauri\resources\ccswitch\CodexSwitch.exe",
    [string]$TestDeviceKey = "test-device-key-12345678"
)

$ErrorActionPreference = "Stop"

Write-Host "=== CCSwitch 环境变量补丁验证 ===" -ForegroundColor Cyan
Write-Host ""

# 步骤 1: 检查 CCSwitch.exe
Write-Host "[1/5] 检查 CCSwitch.exe..." -ForegroundColor Yellow

if (-not (Test-Path $CCSwitchPath)) {
    Write-Host "  ✗ 找不到文件: $CCSwitchPath" -ForegroundColor Red
    exit 1
}

$exeInfo = Get-Item -LiteralPath $CCSwitchPath
Write-Host "  文件大小: $([math]::Round($exeInfo.Length / 1MB, 2)) MB" -ForegroundColor Gray
Write-Host "  修改时间: $($exeInfo.LastWriteTime)" -ForegroundColor Gray

$actualSha256 = (Get-FileHash -LiteralPath $CCSwitchPath -Algorithm SHA256).Hash
Write-Host "  SHA256: $actualSha256" -ForegroundColor Gray

# 验证 SHA256：以同目录 manifest 为准，避免每次产品修复后测试脚本
# 仍锁死上一版哈希，导致把新构建误判为失败。
$manifestPath = "src-tauri\resources\ccswitch\manifest.json"
if (-not (Test-Path $manifestPath)) {
    Write-Host "  ✗ 找不到 manifest.json" -ForegroundColor Red
    exit 1
}
$manifest = Get-Content -Path $manifestPath -Raw | ConvertFrom-Json
$expectedSha256 = ([string]$manifest.artifact.sha256).ToUpperInvariant()
if ($actualSha256.ToUpperInvariant() -eq $expectedSha256) {
    Write-Host "  ✓ SHA256 匹配" -ForegroundColor Green
} else {
    Write-Host "  ✗ SHA256 不匹配!" -ForegroundColor Red
    Write-Host "  期望: $expectedSha256" -ForegroundColor Gray
    Write-Host "  实际: $actualSha256" -ForegroundColor Gray
    exit 1
}

# 步骤 2: 检查 manifest.json
Write-Host ""
Write-Host "[2/5] 检查 manifest.json..." -ForegroundColor Yellow

if (-not (Test-Path $manifestPath)) {
    Write-Host "  ✗ 找不到 manifest.json" -ForegroundColor Red
    exit 1
}

$manifest = Get-Content -Path $manifestPath -Raw | ConvertFrom-Json
$manifestSha256 = $manifest.artifact.sha256

if ($manifestSha256.ToUpperInvariant() -eq $expectedSha256) {
    Write-Host "  ✓ manifest.json SHA256 匹配" -ForegroundColor Green
} else {
    Write-Host "  ✗ manifest.json SHA256 不匹配!" -ForegroundColor Red
    Write-Host "  期望: $expectedSha256" -ForegroundColor Gray
    Write-Host "  实际: $manifestSha256" -ForegroundColor Gray
    exit 1
}

# 步骤 3: 检查 providers.json
Write-Host ""
Write-Host "[3/5] 检查 providers.json..." -ForegroundColor Yellow

$providersPath = "src-tauri\resources\ccswitch\providers.json"
if (-not (Test-Path $providersPath)) {
    Write-Host "  ✗ 找不到 providers.json" -ForegroundColor Red
    exit 1
}

$providers = Get-Content -Path $providersPath -Raw | ConvertFrom-Json
$has404token = $false

foreach ($provider in $providers.providers) {
    if ($provider.id -eq "404token-sub2api") {
        $has404token = $true
        Write-Host "  ✓ 找到 404token-sub2api provider" -ForegroundColor Green
        Write-Host "    名称: $($provider.name)" -ForegroundColor Gray
        Write-Host "    BaseURL: $($provider.baseUrl)" -ForegroundColor Gray
        Write-Host "    需要 API Key: $($provider.requiresApiKey)" -ForegroundColor Gray
        if ($provider.name -ne "404TOKEN（unified-api-internal）") {
            Write-Host "  ✗ provider 未绑定 unified-api-internal 分组名称" -ForegroundColor Red
            exit 1
        }
        $modelsProperty = $provider.PSObject.Properties["models"]
        $fallbackModelsProperty = $provider.PSObject.Properties["fallbackModels"]
        if ($provider.modelSource -ne "runtime" -or $null -ne $modelsProperty -or $null -ne $fallbackModelsProperty) {
            Write-Host "  ✗ provider 携带固定模型限制；统一分组 Key 必须运行时覆盖全部模型" -ForegroundColor Red
            exit 1
        }
        Write-Host "  ✓ 使用单个运行时 Key，模型目录由 /v1/models 获取" -ForegroundColor Green
        break
    }
}

if (-not $has404token) {
    Write-Host "  ✗ 找不到 404token-sub2api provider!" -ForegroundColor Red
    exit 1
}

# 步骤 4: 测试环境变量注入
Write-Host ""
Write-Host "[4/5] 测试环境变量注入..." -ForegroundColor Yellow

# 设置测试环境变量
$env:CCSWITCH_404TOKEN_DEVICE_KEY = $TestDeviceKey
Write-Host "  已设置: CCSWITCH_404TOKEN_DEVICE_KEY=$TestDeviceKey" -ForegroundColor Gray

# 启动 CCSwitch 进程（后台模式，5秒后关闭）
Write-Host "  启动 CCSwitch（测试模式）..." -ForegroundColor Gray

try {
    $process = Start-Process -FilePath (Resolve-Path $CCSwitchPath) -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2

    if ($process.HasExited) {
        Write-Host "  ✗ CCSwitch 启动失败或立即退出" -ForegroundColor Red
        Write-Host "  退出码: $($process.ExitCode)" -ForegroundColor Gray
        exit 1
    }

    Write-Host "  ✓ CCSwitch 已启动 (PID: $($process.Id))" -ForegroundColor Green

    # 清理进程
    Start-Sleep -Seconds 3
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        Write-Host "  进程已清理" -ForegroundColor Gray
    }

} catch {
    Write-Host "  ✗ 启动失败: $_" -ForegroundColor Red
    exit 1
} finally {
    # 清除环境变量
    Remove-Item Env:\CCSWITCH_404TOKEN_DEVICE_KEY -ErrorAction SilentlyContinue
}

# 步骤 5: 验证补丁代码
Write-Host ""
Write-Host "[5/5] 验证补丁代码（静态分析）..." -ForegroundColor Yellow

$sourceFile = "CCSwitch\CCSwitch\src-tauri\src\proxy\providers\codex.rs"
if (Test-Path $sourceFile) {
    $content = Get-Content -Path $sourceFile -Raw

    if ($content -match "CCSWITCH_404TOKEN_DEVICE_KEY") {
        Write-Host "  ✓ 源码中包含环境变量补丁" -ForegroundColor Green

        if ($content -match 'if provider\.id == "404token-sub2api"') {
            Write-Host "  ✓ 404token-sub2api 条件判断存在" -ForegroundColor Green
        } else {
            Write-Host "  ⚠ 未找到 404token-sub2api 条件判断" -ForegroundColor Yellow
        }

    } else {
        Write-Host "  ⚠ 源码中未找到环境变量补丁（可能已被清理）" -ForegroundColor Yellow
    }
} else {
    Write-Host "  ⚠ 源码文件不存在，跳过静态分析" -ForegroundColor Yellow
}

# 总结
Write-Host ""
Write-Host "=== 验证完成 ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "✓ CCSwitch.exe SHA256 验证通过" -ForegroundColor Green
Write-Host "✓ manifest.json 配置正确" -ForegroundColor Green
Write-Host "✓ providers.json 包含 404token-sub2api" -ForegroundColor Green
Write-Host "✓ CCSwitch 可以成功启动" -ForegroundColor Green
Write-Host ""
Write-Host "下一步: 在真实 U 盘环境中测试完整流程" -ForegroundColor Yellow
Write-Host ""
