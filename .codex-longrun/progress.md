# Progress

- 2026-08-31：已阅读开发文档和当前源码；确认 Codex 是本机 MSIX，不能依赖 U 盘文件本身做运行期看门狗。
- 2026-08-31：开始 P1，优先实现可见、可卸载的本地看门狗；不修改 WindowsApps、不做 DLL 注入、不全局杀进程。
- 2026-08-31：完成 `usb_guard.rs`：可移动卷序列号、锚点哈希、包内相对路径、可选 Ed25519 许可证、原子租约/状态文件、5 秒单调宽限和本地无窗口 helper。
- 2026-08-31：接入 Tauri 主入口、Codex/CCSwitch PID 登记、授权门禁、卸载清理和 OfflineHome 状态卡；CCSwitch 增加源文件哈希校验及所有权标记。
- 2026-08-31：重新构建 `release/setup/CodexDesktopSetup.exe` 并组装 `release/usb-oneclick`，完成 Windows MSVC 条件编译和全量回归。
- 2026-08-31：收紧辅助进程锁为“锁文件记录 helper PID + 存活校验”，并让 CCSwitch 卸载复用 PID/镜像名二次校验；重新构建 Release 入口并复核 U 盘目录。
- 2026-08-31：新增 `docs/APPX-PROCESS-GUARD-DEVELOPMENT.md`，记录 P2 开始菜单保护的 AppX 身份、路径、PID、创建时间方案、任务生命周期、测试矩阵和误报边界。

