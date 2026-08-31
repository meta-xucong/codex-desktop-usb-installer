# Roadmap

Objective: 按 USB-BINDING-WATCHDOG-DEVELOPMENT.md 在 Codex U 盘安装器中实现本地看门狗、U 盘绑定和 5 秒拔盘退出，并补齐测试与审计

1. [完成] 冻结 P1 接口：设备指纹、签名租约、受管理进程、状态和日志。
2. [完成] 实现 Rust 看门狗核心及 Windows 设备/进程适配；非 Windows 保持可测试的纯逻辑。
3. [完成] 接入 Tauri 启动、Codex/CCSwitch、卸载和前端授权状态卡。
4. [完成] 补充单元、脚本和 Windows 条件编译测试；修复 A-03/A-04，并对 A-02 增加 CCSwitch 源哈希门禁。
5. [完成] 完成 Release/离线目录构建验证，更新使用说明、审计和归档记录；A-02 的签名清单、A-06 实机拔盘和 P2 开始菜单拦截保留为后续工作。
6. [完成] 形成 `docs/APPX-PROCESS-GUARD-DEVELOPMENT.md`，冻结 AppX 身份、路径、PID、进程创建时间和可选持久看门狗的 P2 改造基线。

