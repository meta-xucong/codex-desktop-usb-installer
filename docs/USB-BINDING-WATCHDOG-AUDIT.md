# 当前 Codex U 盘项目审计报告

审计对象：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

审计范围：当前离线安装、CCSwitch 集成、卸载流程、发布目录和为下一阶段 U 盘绑定准备的安全边界。此次审计为只读审计，没有实现 U 盘看门狗，也没有修改 Codex 本体。

## 1. 验证结果

本机回归结果：

| 检查 | 结果 |
| --- | --- |
| `npm run check` | 通过 |
| `npm run lint` | 通过 |
| `npm run test:offline` | 6/6 通过 |
| `npm run test:offline-bundle` | 通过；覆盖合成 MSIX、清单、哈希和 U 盘布局 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 161/161 通过 |
| `git diff --check` | 通过 |
| 真实 Windows Authenticode、真实 `Add-AppxPackage`、真实拔盘 | 未在本机自动化测试中覆盖 |

`test:offline-bundle` 使用的是无签名合成 MSIX，只能证明离线校验链路，不代表真实 Codex MSIX 在空白电脑上安装成功。

## 2. 发现项

### A-01：当前没有 U 盘绑定和运行期看门狗（高优先级，功能缺口）

现状：安装器能从 U 盘读取 MSIX，但 Codex 安装后进入 WindowsApps，CCSwitch 进入 `%LOCALAPPDATA%\Programs\CodexSwitch`。没有本地守护进程、硬件指纹或拔盘计时逻辑。

影响：拔掉 U 盘不会自动关闭已运行的 Codex/CCSwitch；复制安装器目录后仍可能安装或启动。

结论：这是下一阶段必须增加的架构能力，不能通过现有 `manifest.json` 或 `SHA256SUMS.txt` 单独解决。

### A-02：CCSwitch 复制前未在 Rust 运行路径中强制校验签名/哈希（中高优先级）

位置：`src-tauri/src/app/ccswitch.rs` 的 `install_and_launch`、`copy_payload`。

现状：代码会寻找旁边的 `ccswitch\CodexSwitch.exe` 或资源目录，检查“是普通文件”后复制并启动。静态验证脚本会校验文件，但安装器命令本身没有把 `usb-manifest` 的签名或 CCSwitch 哈希作为信任根。

影响：如果攻击者替换 U 盘上的 CCSwitch 文件并绕过脚本，安装器可能执行被替换的 EXE。第一版应至少在 Rust 路径再次校验固定哈希和 Authenticode；正式绑定版本应由签名清单覆盖。

### A-03：卸载按进程名全局终止 CCSwitch（中高优先级）

位置：`src-tauri/src/app/ccswitch.rs` 的 `terminate_running_process`。

现状：使用 `taskkill /IM CodexSwitch.exe /T /F`。这会匹配同名的其他安装实例，而不是只匹配本产品复制到 `%LOCALAPPDATA%\Programs\CodexSwitch` 的进程。

影响：可能误杀用户单独安装的 CCSwitch。绑定版本必须改为记录 PID/进程路径/安装标记，并在路径和文件哈希匹配时才终止。

### A-04：完整清理会删除共享 CCSwitch 数据目录（中优先级）

位置：`data_dirs` 和 `uninstall(true)`。

现状：清理路径包含 `%USERPROFILE%\.cc-switch` 和 `com.ccswitch.desktop` WebView 数据，没有区分是否由本安装器创建。

影响：用户可能在其他位置安装过 CCSwitch，选择清理时其配置也会被删除。UI/文档中“其他位置单独安装的 CCSwitch 不会被删除”与共享数据清理存在边界不一致。

建议：默认保留共享数据；全量清理前显示精确路径，并只删除带有本产品安装标记或由本产品创建的目录。

### A-05：`SHA256SUMS.txt` 不是防篡改机制（中优先级）

现状：`verify-oneclick-usb.ps1` 会按清单逐文件计算 SHA-256，但清单本身位于同一 U 盘，攻击者可以同时修改文件和哈希。

影响：它适合发现复制损坏，不适合证明“来自可信发行版”。

建议：为 `usb-manifest.json` 增加离线签名；看门狗内置公钥，签名覆盖 Codex、CCSwitch、看门狗文件和设备绑定策略。

### A-06：真实安装和拔盘行为尚未自动化验证（中优先级）

现有测试没有覆盖：真实 Authenticode、真实 AppX 部署、开始菜单直接启动、拔盘后的 5 秒退出、睡眠/唤醒、盘符变化和另一块 U 盘替换。

建议建立 Windows 10/11 空白虚拟机测试矩阵；拔盘测试需记录进程 PID、设备重新枚举时间、`missingAt` 和 `terminatedAt`。

## 3. 已有实现的优点

- 离线 MSIX 载荷使用固定路径、大小、SHA-256、身份、架构和 Application 入口校验。
- 安装前会将载荷复制到临时目录并再次校验，降低 U 盘读取中断造成的半文件风险。
- Tauri 发布构建显式使用 `custom-protocol`，避免空白机器回落到 `127.0.0.1` 开发服务器。
- CCSwitch 安装目录和卸载流程具有幂等性，默认不清除 Codex/CCSwitch 用户数据。
- 当前回归测试全部通过，说明既有安装器功能没有因本次设计文档而改变。

## 4. 审计结论

当前版本可以作为“离线 Codex + CCSwitch 安装器”留档，但还不能宣称具备 U 盘授权或防复制能力。下一阶段应优先解决 A-01，再同时修复 A-02、A-03 和 A-04；A-05 是发行前必须完成的信任链补强，A-06 是跨设备验收前置条件。

推荐的安全顺序为：

```text
签名清单/设备指纹协议
        ↓
本地看门狗 MVP
        ↓
Codex/CCSwitch 精确进程管理
        ↓
开始菜单直接启动防绕过
        ↓
空白 Windows 实机/虚拟机验证
```
