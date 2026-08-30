[CmdletBinding()]
param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$CcswitchArtifactDir = "",
    [switch]$SkipNpmInstall,
    [switch]$SkipTests,
    [switch]$FullTests
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repo
try {
    if ([string]::IsNullOrWhiteSpace($CcswitchArtifactDir)) {
        $workspace = (Split-Path -Parent $repo)
        $CcswitchArtifactDir = Join-Path $workspace "release\ccswitch-product"
    }
    $ccswitchSource = Join-Path $CcswitchArtifactDir "CodexSwitch.exe"
    $ccswitchResourceDir = Join-Path $repo "src-tauri\resources\ccswitch"
    if (-not (Test-Path -LiteralPath $ccswitchSource -PathType Leaf)) {
        throw "未找到 CodexSwitch.exe：$ccswitchSource。请先构建 CCSwitch 产品版，或通过 -CcswitchArtifactDir 指定产物目录。"
    }
    $ccswitchVerifier = Join-Path $CcswitchArtifactDir "verify-product.ps1"
    if (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf) {
        # The verifier checks both the provider endpoint and the embedded
        # renderer marker.  The latter prevents a release built without
        # Tauri's custom-protocol feature (and therefore dependent on
        # localhost:3000) from entering the offline installer.
        & pwsh.exe -NoProfile -File $ccswitchVerifier -ArtifactDir $CcswitchArtifactDir -SkipNetwork
        if ($LASTEXITCODE -ne 0) { throw "CCSwitch 产品版验证失败，拒绝打包未嵌入 renderer 的版本" }
    } else {
        Write-Warning "CCSwitch 产物缺少 verify-product.ps1，跳过嵌入 renderer 检查"
    }
    New-Item -ItemType Directory -Force -Path $ccswitchResourceDir | Out-Null
    foreach ($fileName in @("CodexSwitch.exe", "providers.json", "manifest.json", "LICENSE", "SOURCE-NOTICE.txt", "verify-product.ps1")) {
        $sourceFile = Join-Path $CcswitchArtifactDir $fileName
        if (Test-Path -LiteralPath $sourceFile -PathType Leaf) {
            Copy-Item -LiteralPath $sourceFile -Destination (Join-Path $ccswitchResourceDir $fileName) -Force
        }
    }
    Write-Output "CCSwitch 资源已暂存：$ccswitchResourceDir"
    if (-not $SkipNpmInstall) {
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci 失败" }
    }
    if (-not $SkipTests) {
        & npm.cmd run check
        if ($LASTEXITCODE -ne 0) { throw "TypeScript 检查失败" }
        & npm.cmd run lint
        if ($LASTEXITCODE -ne 0) { throw "前端 lint 失败" }
        # The product build only needs deterministic renderer tests by default.
        # The upstream release-workflow tests create WSL-backed git fixtures and
        # are available explicitly via -FullTests for release CI.
        & npm.cmd test -- --run src
        if ($LASTEXITCODE -ne 0) { throw "前端测试失败" }
        if ($FullTests) {
            & npm.cmd test -- --run
            if ($LASTEXITCODE -ne 0) { throw "完整前端测试失败" }
        }
        & cargo.exe test --manifest-path src-tauri\Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw "Rust 测试失败" }
    }
    # `cargo build` consumes the already-built Vite renderer from `dist`; make
    # that dependency explicit so a clean checkout cannot accidentally package
    # a stale development UI.  Vite's production mode enables the fixed USB
    # Windows screen, while the runtime dispatcher still keeps macOS online.
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw "前端生产构建失败" }
    # Build the Tauri application binary itself.  The USB entry must remain
    # next to manifest.json/payload at runtime; a normal NSIS setup EXE would
    # move the running process into Program Files before our offline probe.
    # `custom-protocol` is required for an offline release.  Without it,
    # Tauri deliberately keeps `cfg(dev)` enabled and opens build.devUrl
    # (127.0.0.1:1420), which only exists while `tauri dev` is running.
    & cargo.exe build --release --features custom-protocol --manifest-path src-tauri\Cargo.toml --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Tauri 应用二进制构建失败" }
    $releaseDir = Join-Path $repo "src-tauri\target\$Target\release"
    $binary = Join-Path $releaseDir "codex-app-manager.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "未找到 Tauri 应用二进制：$binary"
    }
    $setupDir = (New-Item -ItemType Directory -Force -Path (Join-Path $repo "release\setup")).FullName
    Copy-Item -LiteralPath $binary -Destination (Join-Path $setupDir "CodexDesktopSetup.exe") -Force
    $resources = Join-Path $releaseDir "resources"
    if (Test-Path -LiteralPath $resources -PathType Container) {
        Copy-Item -LiteralPath $resources -Destination $setupDir -Recurse -Force
    }
    Write-Output "U 盘入口产物：$(Join-Path $setupDir 'CodexDesktopSetup.exe')"
}
finally { Pop-Location }
