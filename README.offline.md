# Codex Desktop USB Installer（程序在 U 盘、数据在本机）

> 当前版本：v0.5.24。程序/数据分离见
> [v0.5.20 审计](docs/releases/v0.5.20-usb-program-local-data-audit.md)，租约路径修复见
> [v0.5.21 审计](docs/releases/v0.5.21-usb-lease-path-audit.md)，启动兼容与窗口验收见
> [v0.5.24 审计](docs/releases/v0.5.24-usb-license-finalizer-audit.md)。

这是面向 Windows 小白用户的一键离线套件。用户双击 U 盘中的
`CodexDesktopSetup.exe`，确认一次标准 UAC 后，由管理器完成载荷核验、Codex
便携程序安装、依赖检查和 Codex Switch 启动。

## 当前架构

v0.5.20+ 不再把 Codex 注册为 U 盘 AppX 包，也不再依赖 Windows 的外部
`AppXVolume`。固定 MSIX 仍是受信载荷容器，但安装时按 ZIP/MSIX 结构安全解包：

```text
<USB bundle>/
├─ CodexDesktopSetup.exe          唯一建议入口
├─ CodexAppManager.exe            管理器本体
├─ manifest.json
├─ usb-manifest.json
├─ SHA256SUMS.txt
├─ payload/OpenAI.Codex_x64.msix  固定、验签的离线载荷
├─ apps/
│  └─ Codex/                      安装后生成；只放 Codex 程序文件
├─ content/
│  ├─ skills/                     预装 Skill 载荷边界
│  └─ mcp/                        预装 MCP 程序/静态资源边界
├─ ccswitch/CodexSwitch.exe
└─ resources/prerequisites/
   ├─ vc_redist.x64.exe
   └─ MicrosoftEdgeWebView2RuntimeInstallerX64.exe
```

数据边界：

| 内容 | 位置 |
| --- | --- |
| Codex 程序 | `<USB bundle>\apps\Codex` |
| Codex CLI/账号/配置状态 | `%USERPROFILE%\.codex` |
| 桌面端漫游数据 | `%APPDATA%\Codex` |
| 桌面端本地缓存/日志 | `%LOCALAPPDATA%\Codex` |
| Codex Switch 程序 | U 盘 `ccswitch` |
| Codex Switch 用户配置 | 应用定义的本机用户目录 |
| Skill 载荷（后续导入） | U 盘 `content\skills` |
| MCP 可执行文件/静态资源（后续导入） | U 盘 `content\mcp` |
| MCP/Codex 配置与密钥 | 本机 `%USERPROFILE%\.codex\config.toml` 等用户目录 |

“数据在本机”按 Windows 用户实际配置判断，通常就是 C 盘。安装前同时核验环境变量
声明盘符和目录解析后的真实盘符；若用户配置目录或其父级 Junction 实际落在产品 U
盘，安装会停止，不能把“看起来是 C 盘”误报为数据分离成功。

## 为什么废弃 USB AppXVolume 路线

目标机已经证明：即使请求系统数据卷，Windows 10 仍可能把打包应用数据物化为
`%LOCALAPPDATA%\Packages\OpenAI.Codex_*` 到 `<USB>\WpSystem\...` 的 Junction。
因此“策略已写入”“注册 API 已完成”和“数据尚未创建”都不能证明数据最终在 C 盘。

v0.5.20+ 从结构上消除这类不确定性：生产安装路线不调用
`Add-AppxPackage`、`StagePackageAsync`、`RegisterPackageAsync`、`Add-AppxVolume` 或
`Move-AppxPackage`，也不创建 `WindowsApps`、`WpSystem` 或跨卷包数据 Junction。
旧 AppX 代码仅保留用于识别和清理早期版本遗留状态，不是新装路径。

## 安装流程

1. `CodexDesktopSetup.exe` 通过内嵌 `requireAdministrator` 清单请求标准 UAC，并启动同目录管理器。
2. 核验 U 盘身份、授权锚点、固定 manifest、MSIX 大小、SHA-256、包身份、x64 架构和桌面入口。
3. 将 MSIX 复制到本机临时目录后，以独立长时限完成 Authenticode 校验，避免慢 U 盘导致误超时。
4. 核验目标必须是当前已授权 U 盘的精确 `apps\Codex`，拒绝 Junction、符号链接、UNC、相对路径和持久化的旧 C 盘路径。
5. 核验 `%USERPROFILE%`、`%APPDATA%`、`%LOCALAPPDATA%` 的声明位置与真实解析位置均不在产品 U 盘。
6. 在 `apps` 下建立同卷暂存目录，逐项安全解包；拒绝绝对路径、`..`、链接类条目和越界写入，并复核写入字节数。
7. 事务性把旧程序树改名为 rollback，再将新树原子改名为 `apps\Codex`；入口和 manifest 后置条件失败时恢复旧树。
8. USB 模式不创建 Codex 直接开始菜单入口，也不在 API 配置完成前启动 Codex，避免绕过看门狗。
9. 检查 WebView2 与 VC++ x64 运行库；只运行 U 盘内哈希固定、微软签名的完整离线安装器，不手工复制 DLL。
10. 以仅作用于子进程的软件渲染参数从 U 盘启动 Codex Switch；只有检测到尺寸正常的
    可操作窗口并将其恢复到屏幕内、前置显示后才报告成功，再引导用户填写 API Key。
    密钥不预置在发行包中。

## 启动与 U 盘约束

安装完成后仍从 `CodexDesktopSetup.exe` 进入。管理器只会启动当前授权 U 盘中精确
`apps\Codex` 下的程序，并显式传入本机 `%USERPROFILE%\.codex` 作为 `CODEX_HOME`。
拔出 U 盘后，看门狗按既有五秒宽限终止本产品启动的 Codex/CCSwitch 进程。

USB 启动会先使用 `--disable-gpu` 软件渲染，并等待可操作窗口稳定存在；仅当该安全模式
仍无法显示窗口时，才自动清理本次失败进程并以目标机已经验证可用的
`--disable-gpu --no-sandbox` 重试。成功的兼容选择只按当前 Codex 版本记录在本机，升级
Codex 后会重新从安全模式探测。`--no-sandbox` 会降低 Chromium 子进程隔离强度，因此
不是全局设置，也不会用于其他 Codex 或浏览器进程。

每次启动管理器还会核对本机看门狗副本与 U 盘当前版本；不一致时先让旧 helper 正常
退出，再做哈希校验后的原子刷新，避免升级后继续运行旧版路径/租约逻辑。

不创建直接 Codex 快捷方式是有意设计：快捷方式指向 `Codex.exe` 会绕过 U 盘租约。
发行包复制到另一位置后，卷序列号、锚点哈希和 bundle 相对路径不匹配时不能获得租约。

## Skill 与 MCP 边界

v0.5.20+ 先固化目录与安全约束，尚未声称自动导入 Skill/MCP：

- Skill 的程序和说明文件可预置到 `content\skills\<skill>`。下一阶段导入器可在 Codex
  支持的发现目录创建指向 U 盘的目录链接，使载荷仍留在 U 盘。
- MCP 的可执行文件和只读资源可预置到 `content\mcp`；Codex 的 MCP 配置仍写入本机
  `~/.codex/config.toml`，并在盘符变化时由管理器更新绝对路径。
- API Key、OAuth token、登录状态和用户会话不得写进 `content`，也不得加入发行校验清单。

## 卸载

普通卸载：

- 关闭并移除当前 bundle 的 `apps\Codex`；
- 清理本产品管理器/看门狗/CCSwitch 的受管组件；
- 默认保留 `%USERPROFILE%\.codex`、`%APPDATA%\Codex` 和
  `%LOCALAPPDATA%\Codex`，避免误删官方 Codex/CLI 共用数据；
- 可识别并通过 Windows 支持的 API 清理早期版本遗留的精确 OpenAI.Codex AppX 状态，
  但不会接管 `WindowsApps` ACL 或编辑私有包数据库。

高级“清除个人数据”是独立、默认关闭的破坏性选项，会明确列出路径并再次确认。
“同时删除 U 盘安装介质”先允许精确的 `apps\Codex` 作为待卸载运行目录；主卸载完成后
重新执行严格清单核验。管理器自己生成的 `usb-license.json` 及原子 `.bak` 被建模为
“有类型的可变身份元数据”，而不是不可变发行文件：卸载前从当前进程持有的租约捕获
完整 U 盘指纹，二阶段再核对卷身份、发行清单哈希、许可证字节和完整许可证身份，最后
才删除许可证与重试入口。清理直接使用受限文件句柄，不依赖可移动盘回收站。

只有运行目录已消失、以上证据仍一致且没有未知文件时才删除介质。未知 Skill/MCP、
用户文件、伪造/损坏/错指纹许可证都会逐项列在卸载报告中并被保留，不会静默删除。

## 构建

先准备固定的真实 x64 MSIX：

```powershell
pwsh .\tools\collect-payload.ps1 `
  -SourcePath D:\input\OpenAI.Codex.msix `
  -CodexDesktopVersion 26.825.41651
pwsh .\tools\verify-payload.ps1 -BundlePath .\release\usb
```

也可从已配置的 GitHub Fork 更新固定载荷：

```powershell
gh auth status
npm run offline:update-fork
```

构建入口与最终目录：

```powershell
pwsh .\tools\build-installer.ps1
pwsh .\tools\build-oneclick-usb-layout.ps1 `
  -OutputPath .\release\usb-oneclick-v0.5.24-final
pwsh .\tools\verify-oneclick-usb.ps1 `
  -BundlePath .\release\usb-oneclick-v0.5.24-final `
  -SkipNetwork
```

正式发布不能使用 `-SkipSignature`。`-SkipSignature` 仅允许合成 MSIX 测试夹具。

## 验证

```powershell
npm run check
npm run test:offline
npm test
cargo test --manifest-path .\src-tauri\Cargo.toml
pwsh .\tools\test-offline-bundle.ps1
```

开发机自动测试不会执行真实 AppX 安装/卸载、HKLM 写入或修改
`C:\Program Files\WindowsApps`。最终仍需在目标 Windows 10/11 空白机验证：程序目录位于
U 盘、首次运行后本机用户数据产生在正常配置目录、U 盘根没有 `WindowsApps/WpSystem`、
拔盘五秒终止、重插后历史数据仍可用，以及普通/彻底卸载的终态。

## 历史资料

v0.5.19 及以前的 AppXVolume、跨卷数据恢复、孤儿包和 Junction 审计保留在
`docs/releases/` 与 `docs/USB-APPXVOLUME-PORTABLE-*.md`，仅用于迁移、卸载兼容和问题
溯源，不再代表当前安装路线。
