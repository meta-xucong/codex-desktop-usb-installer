# Codex Desktop U 盘安装器 V1：本机模拟测试报告

测试日期：2026-08-30
测试主机：Windows x64，PowerShell 7，Node/Vitest，Rust stable/MSVC

## 已通过

- `npm run check`：TypeScript 通过。
- `npm run lint`：ESLint 通过。
- `npm run test:offline`：5/5 通过。
- `npm test -- --run src`：29 个测试文件、267 个测试通过。
- `cargo test --manifest-path .\src-tauri\Cargo.toml`：159/159 通过。
- `tools/*.ps1`：6 个脚本 AST 解析通过。
- `npm run test:offline-bundle`：合成 MSIX 收集、清单校验、SHA-256 校验、U 盘布局生成和最终逐文件哈希复核通过（最终目录 5 个文件，`SHA256SUMS.txt` 覆盖其中 4 个文件）。
- GitHub Fork `meta-xucong/codex-app-mirror` 已创建；自动同步工作流运行 `33276570073` 成功，并生成 `codex-app-26.825.41651` Release。
- GitHub Release 中的真实 x64 MSIX 已下载并校验：802,145,236 bytes，SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，Authenticode `Valid`，签名主体 `CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B`。
- 真实 MSIX 通过 `collect-payload.ps1`、`verify-payload.ps1` 和 `build-usb-layout.ps1`，识别为 `OpenAI.Codex`、package `26.825.5331.0`、桌面入口 `app/ChatGPT.exe`。
- `release\usb-final` 已生成，可直接作为 U 盘文件集（总载荷约 821 MB）。
- `pwsh .\tools\build-installer.ps1 -SkipNpmInstall -SkipTests`：生产 renderer 和直接 Tauri EXE 构建通过。
- `cargo test --features custom-protocol --manifest-path .\src-tauri\Cargo.toml --target x86_64-pc-windows-msvc`：159/159 通过。
- 修复发布构建未启用 `custom-protocol` 导致的 `127.0.0.1:1420` 拒绝连接：重新生成的
  `release\usb-final\CodexDesktopSetup.exe` 在无 1420 监听、无 Vite 开发服务器条件下启动
  15 秒，窗口正常显示，日志出现 `frontend ready ... degraded=false`。
- `CodexDesktopSetup.exe`：最终 U 盘目录直接启动烟测通过，随后仅结束本次测试进程。
- 固定离线路径修复：MSIX 部署被 Windows 拒绝时，若载荷来自 U 盘且已通过哈希/签名校验，
  会从同一份 MSIX 解包为本地便携安装；失败报告同时保留 HRESULT、ActivityId 和有限长度
  的 `Get-AppxLog` 诊断，不修改系统信任策略。
- 最新 `release\\usb-final`：安装器 19,264,000 bytes，SHA-256
  `e82cf7c000ee3f52cb703f8439ab902747c797c8e2c4d5a16eda70f8d9858201`；目录校验清单 4/4 通过。
- `git diff --check`：通过。

## 尚不能在本机模拟的边界

1. 尚未在空白系统调用真实 `Add-AppxPackage`，也没有验证真实 WindowsApps 注册、开始菜单 AppUserModelId 和 Codex 启动；
   本机已有 Codex 注册包，不能代替干净机部署验收。
2. 自有安装器 EXE 当前为 `NotSigned`，正式发行前需要组织 Authenticode 签名。
3. 尚未在完全干净的 Windows 电脑验证 WebView2、AppX 侧载策略和可能的框架依赖。
4. 完整 `npm test -- --run` 为 331/332；唯一失败是上游 `scripts/release-workflow.test.mjs` 需要 `/bin/bash`，当前机器的 WSL 缺少该文件并在 5 秒后超时，与离线产品源码无关。

## 生成正式 U 盘目录的下一步

以后上游有新版本时，在构建机执行（不要加 `-SkipSignature`）：

```powershell
npm run offline:update-fork
```

只有 `release\usb-final` 中的真实 MSIX、`manifest.json`、`SHA256SUMS.txt`、
`CodexDesktopSetup.exe` 和 resources 才能拷贝到 U 盘。当前仓库中的
`release\usb\manifest.example.json` 只是占位示例。

## 一键 Codex + CCSwitch 增量回归（2026-08-30）

- `npm run check`：通过；
- `npm run test:offline -- --run`：6/6 通过，覆盖安装成功后的 CCSwitch 自动启动和手动重试；
- `npm test -- --run src`：29 个测试文件、268 个测试通过；
- `cargo fmt -- --check`：通过；
- `cargo test --manifest-path .\src-tauri\Cargo.toml app::ccswitch::tests`：1/1 通过；
- 集成版 `build-installer.ps1 -SkipNpmInstall -SkipTests`：release EXE 构建通过；
- `build-oneclick-usb-layout.ps1 -SkipSignature`：生成 `release\usb-oneclick`；
- 最终一键目录为 21 个文件、878,899,391 bytes（约 838 MiB），无重复嵌套 CCSwitch 目录；
- `release\usb-oneclick\verify-usb.ps1 -SkipNetwork`：MSIX、安装器、资源哈希和 CCSwitch 静态验证通过；
- `release\usb-oneclick\verify-usb.ps1`：404TOKEN `/health` 返回 200，无 Key 的 `/v1/models` 返回 401；
- 一键安装器 SHA-256：`9FFEE602B642A6609305115AD00E53F7D4B7A03B41A973B104E3B47F2D38420A`；
- 一键目录保留旧的 `release\usb-validation` 两步验证包作为回退，不覆盖原目录。

这组检查仍然是当前电脑上的模拟/静态验证；真实空白 Windows 电脑上的 AppX 部署、
WebView2、首次启动和有效 API Key 请求需要用户在 U 盘上做最终验收。

## Codex + CCSwitch 一键卸载增量回归（2026-08-30）

- `npm run check`：通过；
- `npm run lint`：通过；
- `npm test -- --run src`：29 个测试文件、271 个测试通过；
- `cargo check --manifest-path .\src-tauri\Cargo.toml`：通过；
- `cargo test --manifest-path .\src-tauri\Cargo.toml --lib`：161/161 通过；
- 新增 `win_uninstall_bundle`：关闭 `CodexSwitch.exe` 子进程，移除本安装器部署目录和匹配的开机启动项；
- 默认保留 `%USERPROFILE%\\.codex`、`%USERPROFILE%\\.cc-switch`；关闭“保留我的数据”后才清理配置和 Tauri WebView 缓存；
- `build-installer.ps1 -SkipNpmInstall -SkipTests`：生产安装器重新编译通过；
- `build-oneclick-usb-layout.ps1 -SkipSignature`：最终 U 盘目录重新生成；
- `verify-usb.ps1`：静态校验、404TOKEN 网络预检和 MSIX 载荷校验全部通过；
- 最终 U 盘目录：21 个文件、879,013,974 bytes；
- 最终安装器 SHA-256：`7CF01DB7D88F999399D8C556716EF9CC956A045D51078C6199C95EB54D38DCDB`。

本次仍未在空白 Windows 电脑执行真实卸载；请在测试机先关闭 Codex/CCSwitch，再从设置或主页点击
“卸载 Codex 与 CCSwitch”进行验收。
