# 能力包与 CCSwitch 安装器集成

## 目标

USB 一键流程在 Codex 便携程序完成位置、哈希和运行状态校验后，自动安装随包的
`content/codex-starter-kit`。该目录包含全部 Skills、Agents、离线 Python/Node
运行时、MCP wrapper、配置模板和连接清单，不依赖 GitHub、npm 或 PyPI 下载。

能力包安装失败不回滚已成功安装的 Codex。用户可以从安装器底部低调的“组件修复”
入口再次安装能力包，或只重试 CCSwitch。

## 前端行为

- 一键安装成功后自动调用 `install_capability_pack`，随后调用
  `install_and_launch_ccswitch`。
- 两个辅助步骤都是可重试的非主流程；失败会显示提示，但不会宣称 Codex 安装失败。
- “组件修复”面板默认不启用任何外部连接。勾选服务并填写完整字段后，才会配置飞书
  MCP、DashScope 搜索、Image-2 或 Seedance。
- “跳过”不会执行能力包安装；“安装 / 修复能力包”只安装本地包和用户明确勾选的连接。

## 后端边界

`src-tauri/src/app/capability_pack.rs` 只从当前 EXE 的 `content`、Tauri resource
或相邻目录寻找能力包，并拒绝符号链接/重解析点。PowerShell 子进程运行包内的
`install-all.ps1`，由包自身校验 `file-manifest.json`、`checksums.sha256` 和完整
离线 runtime media。管理器只向这个外层包装脚本传递其公开参数；全量安装所需的
`IncludeNonDefault`、`IncludeUnsupported`、`Overwrite` 由包装脚本继续传给内部的
`install-to-codex.ps1`，避免把内部参数误传到公开入口。

外部连接参数通过当前 PowerShell 子进程环境传递，绝不写入 U 盘、日志或 readiness
文件。手动面板提交后，选中的连接写入当前用户 HKCU 环境变量；未选中的连接会清除
本包管理的同名变量。一键自动安装使用 `persistConnections=false`，不会覆盖用户已有
的连接配置。

## 发行包契约

`tools/build-usb-layout.ps1` 和 `tools/build-oneclick-usb-layout.ps1` 默认从相邻的
`openclaw-codex-kit/installer-pack` 复制完整能力包到：

```text
content/codex-starter-kit/
```

如果源目录不在默认位置，可显式传入 `-CapabilityPackSource`。最终构建会把能力包
所有文件加入根目录 `SHA256SUMS.txt`；`verify-oneclick-usb.ps1` 还会检查能力包身份、
schema、核心脚本、runtime provisioner 和内部文件清单。

## 验收命令

```powershell
npm run check
npx eslint src --max-warnings=0
npm run test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:offline-bundle -- -InstallerPath .\release\setup\CodexDesktopSetup.exe
pwsh .\tools\verify-oneclick-usb.ps1 `
  -BundlePath .\release\usb-oneclick-v2-fixture-r8 `
  -SkipSignature -SkipNetwork
```

完整 USB 发行包必须包含 `CodexAppManager.exe`、`CodexDesktopSetup.exe`、
`content/codex-starter-kit`、`ccswitch`、离线载荷以及 VC++/WebView2 安装包；
`CodexAppManager.exe` 和入口均需经过 USB 专用 `requireAdministrator` 清单验证。
