# 当前 Codex U 盘项目审计报告

审计对象：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

审计范围：当前离线安装、CCSwitch 集成、卸载流程、发布目录以及已落地的 P1 U 盘绑定看门狗。审计不修改 Codex MSIX/WindowsApps；只检查本产品管理器和本地辅助进程。

## 1. 验证结果

本机回归结果：

| 检查 | 结果 |
| --- | --- |
| `npm run check` | 通过 |
| `npm run lint` | 通过 |
| `npm run test:offline` | 7/7 通过（含 U 盘状态卡/倒计时） |
| `npm run test:offline-bundle` | 通过；覆盖合成 MSIX、清单、哈希和 U 盘布局 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 168/168 通过 |
| `cargo check --target x86_64-pc-windows-msvc --features custom-protocol` | 通过 |
| `npm run build` | 通过 |
| `git diff --check` | 通过 |
| 真实 Windows Authenticode、真实 `Add-AppxPackage`、真实拔盘 | 未在本机自动化测试中覆盖 |

`test:offline-bundle` 使用的是无签名合成 MSIX，只能证明离线校验链路，不代表真实 Codex MSIX 在空白电脑上安装成功。

## 2. 发现项

### A-01：本地看门狗已实现；开始菜单直接启动仍是 P2（高优先级残留）

现状：P1 已增加 `usb_guard.rs`、本地 `CodexUsbGuard.exe`、租约和 5 秒状态机。管理器启动的 Codex/CCSwitch 会登记 PID 并在拔盘后退出。Codex 安装后仍可从开始菜单独立启动；P2 尚未按 AppX 包身份拦截。

影响：直接开始菜单启动的 Codex 不在 P1 租约范围内；复制到非可移动位置会显示 unsupported，复制到另一 U 盘会因卷序列号/锚点不匹配而进入宽限后退出。

结论：P1 已解决管理器工作流；P2 仍需独立做 AppX 身份监控并验证误杀边界。

### A-02：CCSwitch 运行路径仍缺少发行版固定哈希信任根（中高优先级残留）

位置：`src-tauri/src/app/ccswitch.rs` 的 `install_and_launch`、`copy_payload`。

现状：代码会避免覆盖同哈希的正在使用文件，并写入 `.codex-usb-managed.json`；但尚未把 `usb-manifest.json` 的 CCSwitch 哈希/Authenticode 再次作为 Rust 信任根。正式发行仍必须依靠签名许可证/清单和构建产物签名。

影响：如果攻击者替换 U 盘上的 CCSwitch 文件并绕过脚本，安装器可能执行被替换的 EXE；这是发行前必须补强的信任链问题。

### A-03：卸载已改为租约 PID 终止（已修复）

位置：`src-tauri/src/app/ccswitch.rs` 的 `terminate_running_process`。

现状：CCSwitch 启动 PID 写入租约；卸载只读取 `role=ccswitch` 的租约 PID，先优雅关闭再对相同 PID 强制关闭，不再使用全局 `/IM`。

影响：P1 已消除主要误杀路径；仍建议在 Windows 实机用“另一个 CCSwitch 同时运行”回归。

### A-04：共享数据清理增加所有权标记（已修复）

位置：`data_dirs` 和 `uninstall(true)`。

现状：安装器在本地 CCSwitch 目录写入 `.codex-usb-managed.json`；只有标记存在且用户明确选择清理时，才删除共享数据目录；无标记时保留并在报告中说明。

影响：标记本身属于用户可写文件，不能抵抗本机管理员或逆向修改；它解决的是普通误删而非对抗性安全。

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
- P1 新增纯逻辑指纹、签名许可证、5 秒状态机和前端状态卡测试；Windows MSVC 条件编译通过。

## 5. 审计结论

当前版本可以作为“离线 Codex + CCSwitch + P1 U 盘绑定看门狗”候选构建，但不应宣称能阻止专业逆向或拦截所有开始菜单启动。A-03/A-04 已修复；A-02（发行哈希/签名信任根）、A-05（签名清单）和 A-06（真实 Windows 拔盘/空白机）仍是发布前工作。

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
