# U 盘版 Codex fullfix4 诊断、回归与打包记录

日期：2026-09-29<br>
工作区：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`<br>
发布目录：`release\usb-oneclick-v0.5.24-fullfix4-20260929`

## 结论

源码审计确认默认一键安装路径不调用模型探测。模型可用性探测只在已安装页面的“高级功能”折叠区出现；安装测试断言确认一键安装调用 CCSwitch，但不会调用 `gatewayModelProbe`。CCSwitch 使用已验证 USB 账号绑定把 404token 设备 Key 注入子进程，由产品自身执行普通 provider 写入和切换，不需要模型探测证明。

本轮重建的管理器、CCSwitch 资源、Codex payload、入口和内部签名绑定均通过包级检查。物理 U 盘未挂载，因此本记录不把 GUI 安装、真实卸载或 Codex 原生会话描述为已完成。

## 对实测问题的核对

1. **状态提示与维护按钮**：`OfflineHome` 使用单一通知栏；重新校验、修复、日志、组件修复和模型探测收进 `advanced-tools` 折叠区。
2. **CCSwitch 模型验证门禁**：`performInstall` 只执行能力包安装和 `installAndLaunchCcswitch`；旧的“受验证保护的 U 盘 Provider”提示在新 CCSwitch 二进制中不存在。探测失败不会阻止安装、Provider Apply 或 Codex 启动。
3. **Codex 黑框**：便携启动在进程创建前应用 app.asar 固定宽度 console spawn 补丁，并启动受管理进程树的 WinEvent/轮询隐藏器；Codex 26.917 的 `cwd` spawn 形状有专门单测。
4. **卸载残留**：USB 卸载默认启用 Codex 数据、CCSwitch 数据和 manager cleanup；后端在进程、AppX 机器状态、暂存目录、事务和共享清理完成前不会报告成功。无本产品所有权标记的独立 CCSwitch 会按安全策略保留。

## 测试证据

- `npm.cmd check`：通过。
- `npm.cmd run lint`：通过。
- `npm.cmd test -- --run`：37 个文件，443 通过，1 跳过。
- `cargo test --manifest-path src-tauri\Cargo.toml --lib -- --test-threads=1`：554 通过，3 忽略。
- `cargo test --manifest-path crates\codex-win-engine\Cargo.toml --lib -- --test-threads=1`：186 通过，1 忽略。
- `cargo test --manifest-path tools\usb-license-signer\Cargo.toml --all-targets -- --test-threads=1`：51 个库测试和 4 个 GUI 测试通过。
- `cargo check --locked --manifest-path D:\AI\cc-switch-product\src-tauri\Cargo.toml`：通过。
- CCSwitch USB 自动注入单测：通过；证明写入 provider Key、切换 Codex provider，且不触发模型探测。
- `npm.cmd run test:release`：56 通过。
- `npm.cmd run test:offline-bundle`：合成 MSIX 离线包模拟通过。
- `verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork`：通过，`ready=true`。
- `verify-usb-signing-pair.ps1`：通过，`prod-2026-01 / 0.5.24 / g39.5-20260924-r23-codex917-consolefix` 绑定一致。

CCSwitch 产品全量 Rust 测试为 1092 通过、2 失败；失败项是既有 `usage_rollup::test_rollup_merges_with_existing` 固定数据期望和代理测试端口 10048 占用，不涉及 USB 自动注入代码。新增 USB 测试及 `cargo check` 通过。

## 产物哈希

| 文件 | SHA-256 |
|---|---|
| `CodexAppManager.exe` | `DCAA18FFF5EBE9D8978D92EB9BEE1741E146ECE17F1B7D56C8539BFE5ED02D56` |
| `CodexDesktopSetup.exe` | `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464` |
| `ccswitch\CodexSwitch.exe` | `3FA7E6E15263945CAFB083E0A7C02676B0D8AA90C6C9C7CDF4074A36C9C441DC` |
| `payload\OpenAI.Codex_x64.msix` | `ccd6002255042589b6ee1e711818a6703509d4a288851bea455da5a8125f25d0` |
| 签名器 `CodexUsbLicenseSigner.exe`（复用已验证版本） | `572ED752E989BCFABE3CAEE7A5B35DF78E23972292724B8AACF784DC413BDF1C` |

签名器本轮源码和嵌入式管理员凭证没有变化，因此没有用无密钥临时构建替换已验证版本；pair 预检通过。包内没有 `usb-license.json`、`.staging`、`.transaction` 或 `.bak` 残留，入口为 `CodexDesktopSetup.exe`。

## 尚待现场完成

- 将该目录复制到有效签名且设备绑定匹配的 NTFS U 盘，完成 GUI 安装、CCSwitch 托盘自动配置、Provider 切换、Codex 原生启动和彻底卸载。
- 若现场仍出现旧的“完成 Codex 原生测试后应用”错误，应先确认运行的是 fullfix4 包内的 `CodexSwitch.exe`（哈希如上），而不是旧的 C 盘 CCSwitch 或旧 U 盘目录。
