# Codex Desktop USB Installer（第一版实现）

这是基于 `Codex App Manager` Windows/MSIX 安装能力的第一版离线产品层。正式构建时，安装器和固定版本 MSIX 放在同一个 USB bundle 中：

```text
CodexDesktopSetup.exe
manifest.json
SHA256SUMS.txt
payload/OpenAI.Codex_x64.msix
ccswitch/CodexSwitch.exe
resources/ccswitch/CodexSwitch.exe
```

生产模式启动后只读取安装器目录下的 `manifest.json` 和 `payload/*.msix`，不会访问 GitHub、Microsoft Store、镜像站或动态 Release API。流程为：

1. 读取并校验 manifest 路径、版本、架构、最低 Windows Build；
2. 校验 MSIX 文件大小和 SHA-256；
3. 读取 `AppxManifest.xml`，验证 `OpenAI.Codex` 身份、包版本、架构和桌面入口；
4. 在 Windows 上验证 OpenAI Marketplace Authenticode；
5. 将 MSIX 复制到 `%TEMP%\\CodexDesktopInstaller\\<version>` 并再次校验；
6. 优先调用现有受保护的 `Add-AppxPackage` 安装路径；
7. 如果 Windows AppX 服务/策略阻断部署，则从同一份已校验的 MSIX 解包到本地便携目录，
   不下载、不替换为其他版本；
8. 安装成功后通过系统注册信息或便携入口启动 Codex，清理暂存文件。
9. Codex 版本确认成功后，自动寻找 `ccswitch/CodexSwitch.exe` 或
   `resources/ccswitch/CodexSwitch.exe`，复制到
   `%LOCALAPPDATA%\\Programs\\CodexSwitch` 并启动本机副本。CCSwitch 失败不会回滚
   已成功的 Codex 安装，安装完成页提供“打开 Codex Switch”重试按钮。

## 从 GitHub Fork 更新固定载荷

已配置的上游 Fork 为 `meta-xucong/codex-app-mirror`，其 GitHub Actions 每 3 小时检查
`Wangnov/codex-app-mirror`，并同步最新稳定版 Windows x64 MSIX 到 Fork Release。安装器
构建机只需登录 GitHub CLI，即可自动下载固定 Release、读取版本清单、校验 MSIX 签名，
再生成离线 U 盘目录：

```powershell
gh auth status
npm run offline:update-fork
```

也可以固定某个 Release，便于复现构建：

```powershell
pwsh .\tools\update-from-github-fork.ps1 `
  -Repository meta-xucong/codex-app-mirror `
  -ReleaseTag codex-app-26.825.41651
```

脚本默认使用 x64；ARM64 构建机可以添加 `-Architecture arm64`。正式流程不使用
`-SkipSignature`，下载目录中的 MSIX 必须通过 Authenticode、身份、架构、版本和 SHA-256
校验。

## 手工构建固定载荷

如果不使用 GitHub CLI，也可以手工提供一个真实 MSIX：

```powershell
pwsh .\tools\collect-payload.ps1 `
  -SourcePath D:\input\OpenAI.Codex.msix `
  -CodexDesktopVersion 26.602.40724
pwsh .\tools\verify-payload.ps1 -BundlePath .\release\usb
```

`-SkipSignature` 只用于本地合成 fixture 测试，不能用于正式包。

## 构建和生成 U 盘目录

```powershell
pwsh .\tools\build-installer.ps1
pwsh .\tools\build-usb-layout.ps1 `
  -InstallerPath .\release\setup\CodexDesktopSetup.exe `
  -BundlePath .\release\usb
# 生成 Codex + CCSwitch 一键验证目录
pwsh .\tools\build-oneclick-usb-layout.ps1 -SkipSignature
pwsh .\release\usb-oneclick\verify-usb.ps1 -SkipNetwork
```

`build-installer.ps1` 生成的是可直接从 U 盘运行的 Tauri 应用 EXE，不是
NSIS 的二级 `*-setup.exe`。这样运行时的 `current_exe()` 仍位于 U 盘，才能
发现旁边的 `manifest.json` 和 `payload`；Tauri `resources` 目录会一并复制。
脚本会以 `--features custom-protocol` 构建发布二进制，把 `dist` 内嵌进 EXE；
如果省略该特性，Tauri 会回到 `devUrl`（默认 `127.0.0.1:1420`），在没有开发服务器
的 U 盘电脑上会出现 `ERR_CONNECTION_REFUSED`。

第一版保留上游 Tauri 窗口样式，但生产构建在 Windows 上进入离线主页；在线目录、自动更新、API、Skill、MCP 和拔盘退出均未启用。离线入口使用固定 `msix-fixed` 路径，始终优先安装 MSIX；只有本地 MSIX 已通过校验而 Windows 拒绝 AppX 部署时，才启用同一文件的本地解包应急路径。

`release\\usb-oneclick` 是当前推荐的交付目录。它的入口 EXE 内置“Codex 安装成功后
启动 CCSwitch”的状态机，但 802 MB 的 MSIX 仍作为 `payload` 放在 EXE 旁边，避免
把超大载荷复制到进程资源或触发 Windows 更新引擎的离线载荷错误。`manifest.json`
始终是 Codex 安装器清单；联合包信息单独放在 `usb-manifest.json`。

### 一键卸载 Codex 与 CCSwitch

离线版在 Codex 已安装页面提供“卸载 Codex 与 CCSwitch”入口；“设置 → 更多”中也有
同名入口。它会先关闭 `CodexSwitch.exe` 及其子进程，移除本安装器复制到
`%LOCALAPPDATA%\\Programs\\CodexSwitch` 的程序、匹配的开机启动项，再调用现有受保护
的 Codex 卸载流程。默认保留 `%USERPROFILE%\\.codex` 和 `%USERPROFILE%\\.cc-switch`，
因此不会误删登录、会话和供应商配置；关闭“保留我的数据”并完成第二次确认后，才会
清除这两个目录以及 CCSwitch 的 Tauri WebView 缓存。清理步骤可重复执行，遇到进程锁定
时关闭托盘程序后重新进入该页面即可重试。其他位置单独安装的 CCSwitch 不会被删除。

MSIX 部署失败报告会附带 Windows HRESULT、ActivityId 和有限长度的 `Get-AppxLog` 内容，
可在“打开日志”或错误详情中复制给维护人员。安装器不会自动修改
`AllowAllTrustedApps`、证书信任库或企业策略；如果连本地解包也失败，应根据这些诊断修复系统，
或改用受支持的 Windows 版本。

## 测试

```powershell
npm ci
npm run check
npm test -- --run src
npm run test:offline
npm run test:offline-bundle
cargo test --manifest-path .\src-tauri\Cargo.toml
# 发布特性编译检查（与 U 盘 EXE 相同的 Cargo 特性）
cargo test --features custom-protocol --manifest-path .\src-tauri\Cargo.toml --target x86_64-pc-windows-msvc
```

Rust 离线模块的测试使用合成 MSIX ZIP，覆盖 manifest 路径穿越、身份/入口、哈希校验、暂存后二次校验和 Windows Build 解析；不会调用真实 `Add-AppxPackage`。真实安装必须在断网 Windows 10/11 x64 虚拟机中完成验收。

`npm run test:offline-bundle` 会生成一个仅用于测试的无签名合成 MSIX，完整跑通
`collect-payload.ps1`、`verify-payload.ps1`、`build-usb-layout.ps1`，并逐文件复核最终
U 盘目录的 `SHA256SUMS.txt`。该测试产物不会写入 `release\usb-final`，也不能拿来安装
或对外发行；正式构建必须替换为已获授权且 Authenticode 有效的 Codex Desktop MSIX。

仓库完整 `npm test` 还包含上游 release/WSL fixture；在没有可用 WSL `/bin/bash`
的构建机上，这部分会因环境超时，不影响上述产品源码测试。需要完整回归时请先准备
WSL，再执行 `npm test`。

本机本次回归记录见 `TEST-REPORT.offline.md`。

## 合规边界

本仓库只提供安装器代码。Codex Desktop MSIX、商标、签名和再分发权不由 MIT 许可覆盖。当前产物仅用于内部技术验证，正式商业发行前必须取得相应授权并对安装器 EXE 做 Authenticode 签名。
