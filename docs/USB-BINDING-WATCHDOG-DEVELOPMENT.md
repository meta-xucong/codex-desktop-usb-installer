# 旧文档已废弃

本文件不再是开发基线。项目已从“把 Codex 安装到 C 盘，再用长期 AppX/PID 看门狗拦截
开始菜单启动”的路线切换为：

- Codex MSIX 注册到当前 U 盘的 Windows AppXVolume；
- CCSwitch 可执行文件在 USB 模式直接从 U 盘启动；
- Codex/CCSwitch 用户数据留在本机用户目录；
- 现有 USB guard 只承担安装器会话授权和辅助清理，不承诺全局进程拦截。

请只阅读和维护新的基线文档：

[USB-APPXVOLUME-PORTABLE-DEVELOPMENT.md](./USB-APPXVOLUME-PORTABLE-DEVELOPMENT.md)

旧的 `APPX-PROCESS-GUARD-DEVELOPMENT.md` 已从仓库移除。历史审计中的相关内容仅用于解释
为什么不采用该路线，不得作为新增功能的验收标准。
