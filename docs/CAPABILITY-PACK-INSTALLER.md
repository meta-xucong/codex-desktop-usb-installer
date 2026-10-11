# 能力包与 CCSwitch 安装器集成

## 目标

USB 一键流程在 Codex 便携程序完成位置、哈希和运行状态校验后，自动安装随包的
`content/codex-starter-kit`。该目录包含全部 Skills、Agents、离线 Python/Node
运行时、MCP wrapper、配置模板和连接清单。Python、Feishu MCP、FFmpeg 与 Manim
依赖随包安装；HyperFrames CLI 首次安装需要联网访问 npm，没有可用 Chrome 时还会联网
准备浏览器。HyperFrames 的联网步骤失败会标记为 `requires-network`，其他组件继续安装，
恢复网络后可从“高级功能 → 组件修复”重试。

能力包安装失败不回滚已成功安装的 Codex。主界面的“连接与扩展”用于设置飞书、API
和远程 MCP；“高级功能”中的“组件修复”用于重装能力包或单独重试 CCSwitch。

## 前端行为

- 一键安装成功后自动调用 `install_capability_pack`，随后调用
  `install_and_launch_ccswitch`。
- 两个辅助步骤都是可重试的非主流程；失败会显示提示，但不会宣称 Codex 安装失败。
- “连接与扩展”面板默认不启用任何外部连接。勾选服务并填写完整字段后，才会配置飞书
  MCP、DashScope 搜索、统一媒体 MCP（Image-2、Grok、Seedance；MiniMax 仅在共享上游提供模型后启用）；远程 OAuth MCP 另需 Codex 登录授权。
- “组件修复”视图只提供能力包安装/修复和 CCSwitch 重试；运行时检查、程序修复和日志
  仍留在“高级功能”菜单。
- 未勾选的服务不会触发密钥校验、运行时健康检查或环境变量删除；若核心 Skills/Agents
  已完成而某个运行时失败，界面显示“核心能力已就绪，运行时待修复”，不把能力包误报为未安装。
- “保存并应用连接”只应用本地能力包和用户明确勾选的连接；“组件修复”视图中的
  “安装 / 修复能力包”用于手动维护。

## 后端边界

`src-tauri/src/app/capability_pack.rs` 只从当前 EXE 的 `content`、Tauri resource
或相邻目录寻找能力包，并拒绝符号链接/重解析点。PowerShell 子进程运行包内的
`install-all.ps1`，由包自身校验 `file-manifest.json`、`checksums.sha256` 和离线
runtime media。未启用飞书时 Node/飞书 MCP 媒体不进入运行时必需健康条件；管理器只向
这个外层包装脚本传递其公开参数；全量安装所需的
`IncludeNonDefault`、`IncludeUnsupported`、`Overwrite` 由包装脚本继续传给内部的
`install-to-codex.ps1`，避免把内部参数误传到公开入口。

外部连接参数通过当前 PowerShell 子进程环境传递，绝不写入 U 盘或 readiness 文件。
手动面板提交后，只有选中的、非空连接字段才会写入当前用户 HKCU 环境变量；未选中的
连接不会清除用户已有变量。一键自动安装使用 `persistConnections=false`，不会覆盖用户
已有的连接配置。运行时和配置阶段的子进程输出只写入能力包目录下的脱敏诊断文件。

## 发行包契约

`tools/build-usb-layout.ps1` 和 `tools/build-oneclick-usb-layout.ps1` 默认从当前仓库内的
`installer-pack` 复制完整能力包到：

```text
content/codex-starter-kit/
```

克隆仓库后无需准备同级目录即可复现构建。如果源目录不在默认位置，可显式传入
`-CapabilityPackSource`；旧的相邻 `openclaw-codex-kit/installer-pack` 目录仍可作为
外部源传入。最终构建会把能力包
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
