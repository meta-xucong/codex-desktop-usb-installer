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
if ($SkipBuild) { throw "CCSwitch 升级构建不允许 -SkipBuild；必须重新构建并验证 4.0.20 产物" }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$productSourcePatchPath = Join-Path $repoRoot "src-tauri\resources\ccswitch\source-patches\ccswitch-usb-product-85ad4969.patch"
$sourceManifestPath = Join-Path $repoRoot "src-tauri\resources\ccswitch\manifest.json"
if (-not (Test-Path -LiteralPath $productSourcePatchPath -PathType Leaf)) {
    throw "缺少固定 CCSwitch 产品源码补丁：$productSourcePatchPath"
}
$sourceManifest = Get-Content -LiteralPath $sourceManifestPath -Raw | ConvertFrom-Json
if ($sourceManifest.sourceCommit -ne "85ad4969b3399500baaea9d46b1e08f87aba80e1" -or
    $sourceManifest.sourceVersion -ne "4.0.20" -or
    $sourceManifest.sourcePatch -ne "source-patches/ccswitch-usb-product-85ad4969.patch") {
    throw "CCSwitch 源码清单不是固定 4.0.20 产品基线"
}
$actualSourcePatchHash = (Get-FileHash -LiteralPath $productSourcePatchPath -Algorithm SHA256).Hash
if ($actualSourcePatchHash -ne ([string]$sourceManifest.sourcePatchSha256).ToUpperInvariant()) {
    throw "CCSwitch 产品源码补丁 SHA256 与 manifest 不匹配：$actualSourcePatchHash"
}

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

            git checkout 85ad4969b3399500baaea9d46b1e08f87aba80e1
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

# Clean upstream clones need the complete USB product source overlay. A
# pre-patched worktree is accepted only when it is exactly the pinned commit
# plus this patch; marker/callsite checks alone do not prove source identity.
$sourceHead = (& git -C $SourceDir rev-parse HEAD 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $sourceHead -ne "85ad4969b3399500baaea9d46b1e08f87aba80e1") {
    throw "CCSwitch 源码不是固定基线 85ad4969b3399500baaea9d46b1e08f87aba80e1：$SourceDir"
}
$indexFlags = @(& git -C $SourceDir ls-files -v)
if ($LASTEXITCODE -ne 0) { throw "无法读取 CCSwitch Git index flags" }
$assumeUnchanged = @($indexFlags | Where-Object { ([string]$_).Length -gt 2 -and ([string]$_)[0] -cmatch '[a-z]' })
$skipWorktree = @(& git -C $SourceDir ls-files -t | Where-Object { ([string]$_).StartsWith("S ") })
if ($LASTEXITCODE -ne 0) { throw "无法读取 CCSwitch skip-worktree flags" }
if ($assumeUnchanged.Count -gt 0 -or $skipWorktree.Count -gt 0) {
    throw "CCSwitch Git index 含 assume-unchanged 或 skip-worktree 路径；拒绝构建可能隐藏源码改动的工作树"
}
$sourceStatus = @(& git -C $SourceDir status --porcelain --untracked-files=all)
$ignoredStatus = @(& git -C $SourceDir status --ignored --porcelain --untracked-files=normal)
if ($LASTEXITCODE -ne 0) { throw "无法检查 CCSwitch 被忽略的文件" }
$allowedIgnoredPaths = @("node_modules", "dist", "release", "src-tauri/gen", "src-tauri/target")
$unexpectedIgnoredPaths = @($ignoredStatus | ForEach-Object {
    $line = [string]$_
    if ($line.StartsWith("!! ")) { $line.Substring(3).TrimEnd('/') -replace '\\', '/' }
} | Where-Object {
    $path = [string]$_
    if ([string]::IsNullOrWhiteSpace($path)) { return $false }
    if ($allowedIgnoredPaths -contains $path) { return $false }
    if ($path -match '^(\.claude|\.codex|\.gemini|\.vscode)(/|$)' -or
        $path -match '^(CLAUDE\.md|GEMINI\.md|\.DS_Store|vitest-report\.json)$' -or
        $path -match '\.(log|tsbuildinfo)$') { return $false }
    return $true
})
if ($unexpectedIgnoredPaths.Count -gt 0) {
    throw "CCSwitch 构建树含未审计的 Git 忽略输入：$($unexpectedIgnoredPaths -join ', ')"
}
if ($sourceStatus.Count -eq 0) {
    & git -C $SourceDir apply --check $productSourcePatchPath
    if ($LASTEXITCODE -ne 0) { throw "固定 CCSwitch 产品源码补丁无法应用：$productSourcePatchPath" }
    & git -C $SourceDir apply $productSourcePatchPath
    if ($LASTEXITCODE -ne 0) { throw "应用固定 CCSwitch 产品源码补丁失败：$productSourcePatchPath" }
    Write-Host "✓ 已在固定上游基线上应用 USB 产品源码补丁" -ForegroundColor Green
} else {
    & git -C $SourceDir apply --reverse --check $productSourcePatchPath
    if ($LASTEXITCODE -ne 0) {
        throw "源码虽有产品标记，但不能完整反向匹配固定产品补丁；拒绝构建混合源码：$SourceDir"
    }
    $expectedPatchPaths = @(& git apply --numstat $productSourcePatchPath | ForEach-Object {
        $parts = $_ -split "`t"
        if ($parts.Count -ge 3) { $parts[2] }
    } | Sort-Object -Unique)
    $dirtyPaths = @(& git -C $SourceDir status --porcelain --untracked-files=all | ForEach-Object {
        $line = [string]$_
        if ($line.Length -ge 4) { $line.Substring(3).Trim('"') }
    } | Sort-Object -Unique)
    $expectedSorted = @($expectedPatchPaths | Sort-Object -Unique)
    if (($dirtyPaths -join "`n") -ne ($expectedSorted -join "`n")) {
        throw "预打补丁源码包含补丁之外的文件变更；请使用干净固定基线重建。源码：$SourceDir"
    }
    $validationRoot = Join-Path $repoRoot (".tmp-ccswitch-source-audit-" + [guid]::NewGuid().ToString("N"))
    $archivePath = "$validationRoot.tar"
    New-Item -ItemType Directory -Path $validationRoot -Force | Out-Null
    try {
        & git -C $SourceDir archive --format=tar --output=$archivePath HEAD
        if ($LASTEXITCODE -ne 0) { throw "无法导出固定 CCSwitch 基线用于源码核对" }
        & tar -xf $archivePath -C $validationRoot
        if ($LASTEXITCODE -ne 0) { throw "无法展开固定 CCSwitch 基线用于源码核对" }
        & git -C $validationRoot init --quiet
        if ($LASTEXITCODE -ne 0) { throw "无法初始化临时源码核对目录" }
        & git -C $validationRoot apply $productSourcePatchPath
        if ($LASTEXITCODE -ne 0) { throw "固定产品补丁无法应用到临时固定基线" }
        foreach ($relativePath in $expectedSorted) {
            $actualFile = Join-Path $SourceDir $relativePath
            $expectedFile = Join-Path $validationRoot $relativePath
            if (-not (Test-Path -LiteralPath $actualFile -PathType Leaf) -or
                -not (Test-Path -LiteralPath $expectedFile -PathType Leaf)) {
                throw "预打补丁源码文件缺失或与固定产品补丁不一致：$relativePath"
            }
            $actualHash = (Get-FileHash -LiteralPath $actualFile -Algorithm SHA256).Hash
            $expectedHash = (Get-FileHash -LiteralPath $expectedFile -Algorithm SHA256).Hash
            if ($actualHash -ne $expectedHash) {
                throw "预打补丁源码与固定产品补丁内容不一致：$relativePath"
            }
        }
    } finally {
        Remove-Item -LiteralPath $validationRoot -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $archivePath -Force -ErrorAction SilentlyContinue
    }
    Write-Host "✓ 预打补丁源码的 dirty path 集合与固定产品补丁一致" -ForegroundColor Green
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

# Source identity was verified above. Never inject code by regex afterward:
# all USB changes must be part of the pinned, auditable source patch.
Write-Host "[步骤 3/6] 校验 4.x 产品协议与原生字段写入路径..." -ForegroundColor Green
$providerServicePath = Join-Path $cargoProjectDir "src\services\provider\usb_bootstrap.rs"
$providerServiceText = [System.IO.File]::ReadAllText($providerServicePath)
foreach ($marker in @("CODEX_USB_404TOKEN_HANDOFF_V1", "CODEX_USB_V4_FIELD_WRITE_V1")) {
    if (-not $providerServiceText.Contains($marker)) {
        throw "CCSwitch 产品源码缺少协议标记 $marker；拒绝构建旧快照写入版本"
    }
}
if ($providerServiceText -notmatch 'Self::switch_codex_direct\s*\(state,\s*&projection\.provider,\s*&providers\)') {
    throw "CCSwitch USB handoff 未调用 4.x Codex direct 字段写入路径"
}
$providerModuleText = [System.IO.File]::ReadAllText((Join-Path $cargoProjectDir "src\services\provider\mod.rs"))
if (-not $providerModuleText.Contains("usb_bootstrap") -or -not $providerModuleText.Contains("switch_codex_direct")) {
    throw "CCSwitch USB handoff 模块未接入 provider service"
}
$codexDirectPath = Join-Path $cargoProjectDir "src\services\provider\codex_direct.rs"
if (-not (Test-Path -LiteralPath $codexDirectPath -PathType Leaf)) { throw "CCSwitch 4.x Codex direct writer 缺失" }
$codexWriterPath = Join-Path $cargoProjectDir "src\live\project\codex.rs"
if (-not (Test-Path -LiteralPath $codexWriterPath -PathType Leaf)) {
    throw "CCSwitch 4.x 原生 Codex 字段写入模块缺失"
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
        & pnpm.cmd install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw "CCSwitch 锁定依赖安装失败" }
        & pnpm.cmd run typecheck
        if ($LASTEXITCODE -ne 0) { throw "CCSwitch TypeScript 检查失败" }
        & pnpm.cmd exec vitest run `
            tests/components/CodexFormFields.modelFetch.test.tsx `
            tests/components/ModelDropdown.test.tsx `
            tests/components/ProviderForm.codexCopilot.test.tsx `
            tests/components/ProviderForm.presetRows.golden.test.tsx `
            tests/config `
            tests/utils/codexTemplates.compatibility.test.ts `
            tests/utils/modelAliases.test.ts
        if ($LASTEXITCODE -ne 0) { throw "CCSwitch USB 模型选择/预设前端回归失败" }
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
    $requiredTests = @(
        @{ Filter = "usb_404token_bootstrap_"; Count = 10 },
        @{ Filter = "usb_model_selection::tests"; Count = 17 },
        @{ Filter = "usb_upgrade_"; Count = 23 }
    )
    foreach ($test in $requiredTests) {
        $testOutput = & cargo test --locked --lib $test.Filter -- --test-threads=1 2>&1
        $testExitCode = $LASTEXITCODE
        $testText = $testOutput | Out-String
        Write-Host $testText
        if ($testExitCode -ne 0 -or $testText -notmatch "test result: ok\.\s+$($test.Count) passed; 0 failed") {
            throw "CCSwitch 4.x USB/handoff 回归测试未按预期通过：$($test.Filter)，期望 $($test.Count) 项"
        }
    }

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
    -not $binaryText.Contains("CCSWITCH_404TOKEN_DEVICE_KEY") -or
    -not $binaryText.Contains("CODEX_USB_V4_FIELD_WRITE_V1") -or
    -not $binaryText.Contains("CODEX_USB_FULL_IMPORT_V1")) {
    throw "构建产物缺少 404token handoff 或 4.x 原生字段写入协议标记；可能选中了旧缓存 EXE：$builtExePath"
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
New-Item -ItemType Directory -Force -Path (Join-Path $OutputDir "source-patches") | Out-Null
Copy-Item -LiteralPath $productSourcePatchPath -Destination (Join-Path $OutputDir "source-patches\ccswitch-usb-product-85ad4969.patch") -Force

# 复制或更新 manifest.json
$currentManifestPath = Join-Path $repoRoot "src-tauri\resources\ccswitch\manifest.json"
$newManifestPath = Join-Path $OutputDir "manifest.json"

if (Test-Path -LiteralPath $currentManifestPath) {
    $manifest = Get-Content -LiteralPath $currentManifestPath -Raw | ConvertFrom-Json
    $manifest.artifact.sha256 = $newSha256
    $manifest.artifact | Add-Member -NotePropertyName buildStatus -NotePropertyValue "built" -Force
    $manifest.artifact | Add-Member -NotePropertyName sourceVersion -NotePropertyValue $manifest.sourceVersion -Force
    $manifest.artifact | Add-Member -NotePropertyName sourceCommit -NotePropertyValue $manifest.sourceCommit -Force
    $manifest.notes = @(
        "404TOKEN is a third-party Sub2API relay, not an OpenAI official endpoint.",
        "A verified USB license injects one device API key for the unified-api-internal group at runtime.",
        "CCSwitch saves the USB key to its normal private provider database, switches the provider, and returns a nonce-bound acknowledgement only after local readback succeeds.",
        "Codex provider handoff uses the upstream 4.x key-field writer; user-global plugin, permission and MCP settings are not restored from provider snapshots.",
        "USB product source changes are captured in sourcePatch and must be applied to sourceCommit before building.",
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
    $notice = @"
CodexSwitch USB product source notice
===================================
Upstream: https://github.com/farion1231/cc-switch
Version: $($manifest.sourceVersion)
Commit: $($manifest.sourceCommit)
Product patch: $($manifest.sourcePatch)
Patch SHA256: $($manifest.sourcePatchSha256)
EXE SHA256: $newSha256
Apply the complete audited product patch to the pinned upstream commit to reproduce this build.
This product retains the upstream 4.x key-field writer and adds the USB handoff, isolated home and explicit model-selection contract.
License: see LICENSE. 404TOKEN is a third-party service, not an OpenAI official endpoint.
This artifact contains no API keys, cookies or user credentials.
"@
    [System.IO.File]::WriteAllText((Join-Path $OutputDir "SOURCE-NOTICE.txt"), $notice, $utf8NoBom)
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
Write-Host "   .\tools\build-installer.ps1 -CcswitchArtifactDir `"$OutputDir`" -UsbPolicy SignedRequired" -ForegroundColor Gray
Write-Host ""
Write-Host "2. 不要只复制 EXE 或手改哈希；使用上面的构建入口暂存完整产物、manifest 和源码补丁。" -ForegroundColor White
Write-Host "   按现有 SignedRequired 签名流程构建并验证整包，不要跳过测试或降级策略。" -ForegroundColor Gray
Write-Host ""
