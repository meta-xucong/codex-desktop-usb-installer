# Roadmap

Objective: 采用 USB 便携程序 + 本地用户数据方案，把 Codex 程序和 CCSwitch 载荷留在 U 盘，保留早期 AppX 状态的安全兼容清理，并完成可复制、可验证的一键发布目录。

1. [完成] 审计既有离线安装器，删除 `APPX-PROCESS-GUARD-DEVELOPMENT.md`，将旧看门狗文档标记为历史记录。
2. [完成] 在 Rust Windows 引擎中增加 `Add-AppxVolume`/`Add-AppxPackage -Volume` 路径，并从当前 EXE 派生 U 盘卷根。
3. [完成] 增加 `RestrictAppDataToSystemVolume` 的 UAC、前值 journal、失败回滚和卸载恢复。
4. [完成] USB 模式禁止 portable 回退；安装后确认注册包的 `packageVolume` 与当前 U 盘一致。
5. [完成] CCSwitch 在 USB 模式直接启动 U 盘 EXE，普通模式继续保留本地复制兼容路径。
6. [完成] 更新离线主页、卸载提示、联合清单、README、静态验证和实现审计。
7. [完成] 运行 Rust 173 项、前端源代码 275 项、发布 56 项、Windows 目标编译检查、离线 bundle 验证，并重建 `release/usb-oneclick`。
8. [完成] 加固联合卸载：AppX/便携目录/快捷方式/卸载项/CCSwitch/策略/用户数据均有有限重试和后置校验；非本产品 CCSwitch 需有效所有权标记才会处理。
9. [完成] 修复卸载后保留 C: 用户数据引发的 `0x80073CF9`：USB 安装预扫描/错误驱动双路径先在系统卷 bootstrap，再用 `Move-AppxPackage` 迁移到 U 盘；诊断输出固定为 UTF-8。
10. [待实机] 在 Windows 10/11 + NTFS U 盘空白机验证 AppXVolume、UAC、用户数据目录、卸载恢复和拔盘行为；该项不能由当前主机模拟替代。
11. [完成] 修复实机暴露的 MSIX 健康检查 URI 错误：不再把 `shell:AppsFolder\\...` 作为 `Start-Process` 的可执行路径，统一改为 `explorer.exe` 参数激活；正常启动路径同步修复并完成发布重建。
12. [完成（待用户实机确认）] 诊断并修复最新实机“MSIX 离线安装失败/无效 URI”：确认截图走的是 sideload 失败分支；预检本地 NTFS 卷，保留可读的 operation、目标卷、文件系统和 ActivityId 证据，复用 Add-AppxVolume 返回对象，重新构建并执行全量回归。若截图中的 E: 对应当前主机，E: 为 WebDAV 映射盘，必须改用真正的 NTFS 可移动盘。
13. [完成（待用户实机确认）] 修复可移动介质上的 Authenticode 超时：普通 45 秒探测改为独立 5 分钟预算，离线快速预检延迟签名扫描，安装阶段先复制/哈希校验本地暂存副本，再执行强制签名验证；重新构建并通过 Rust、前端、发布、离线 bundle 和真实 MSIX 静态验证。
14. [完成（待用户实机确认）] 修复 `Add-AppxVolume` 在普通权限下返回 `Access is denied`：首轮保持最小权限，命中明确卷注册权限错误时通过标准 UAC 仅提升当前 USB 卷注册脚本，复核 NTFS/在线卷后重试同一份 MSIX；新增分类/脚本范围测试，更新文档并重建发布目录。真实目标机仍需确认 UAC 接受后的卷注册和安装重试。
15. [完成（待用户实机确认）] 发布前收尾加固：USB 部署独立 10 分钟超时；PowerShell 5.1 UTF-8 BOM/默认路径兼容；CCSwitch JSON 显式 UTF-8；发布校验拒绝 SHA256 清单之外的残留文件；最终发布包重建并通过默认签名校验。仅剩真实 Windows 10/11 空白机 AppX/UAC/VC++ 验收。
16. [完成（待用户实机确认）] 修复组合 UAC 状态机：只有组合“数据策略+USB AppX 卷注册”真正成功时才抑制后续卷注册提示；组合操作被拒绝、超时或失败后，若普通安装明确返回 `Add-AppxVolume Access is denied`，仍执行一次当前卷的标准 UAC 注册和同一 MSIX 重试。已加入回归测试、更新用户文档并重建最终候选包。
17. [完成（待用户实机确认）] 按管理员账户使用场景增加 USB 入口启动自动提权：检测 U 盘布局后通过原生 `ShellExecuteW` 的 `runas` 动词重启自身，验证高完整性令牌后才进入 Tauri；取消/拒绝/未获得管理员令牌时在安装前退出。普通本地版与看门狗 CLI 不受影响；文档、联合清单、Windows 目标编译、Rust 186 项、发布校验和入口 smoke 均已更新。
18. [完成（待用户实机确认）] 修复 v0.5.3 “sideload completed 但包装到 C:”假成功：USB PowerShell 在所有部署路径后强制核验 `InstallLocation`，仅清理本次新建的错盘注册并做一次原生 USB 重试；管理器/管理员子进程增加高完整性门禁，长脚本改用临时 BOM 文件，发布版本升级为 v0.5.4 并完成正式目录重建、静态校验和回归测试。
19. [完成（待用户实机确认）] 修复旧版/普通 sideload MSIX 卸载的 `0x80073CFA`：当 `-PreserveApplicationData` 被 Windows 明确判定为非 deployment-mode 不兼容时，仅移除该参数并重试同一当前用户包；保留卸载后注册状态后置校验、数据保留警告和正式 v0.5.5 发布清单。
20. [完成（待用户实机确认）] v0.5.6 安装/卸载收口：管理员判定升级为完整令牌（含有效 Administrators SID），安装错盘回滚、数据冲突清理和显式卸载统一兼容 `0x80073CFA` 与缺少 `PreserveApplicationData` 参数的旧模块；重建正式目录并完成源码、前端、Windows 目标、发布和离线验证。
21. [完成（待用户实机确认）] v0.5.19 注册收敛：保留 v0.5.18 精确 staged 孤儿重水合，取消 `DeploymentResult.IsRegistered` 的成功门禁，以当前用户精确身份、健康、非部分暂存、USB 路径和所有权的 60 秒有界后置条件验收；日志区分原生完成与投影收敛，超时续接不重复部署；发布新版本目录并完成全量验证。
22. [等待独立报告] 开始菜单启动 ChatGPT 空白；在 WorkBuddy 返回完整运行时日志后单独诊断，不与安装成功误报修复混合。
23. [完成（待用户实机确认）] v0.5.23 启动可用性收口：USB Codex 先用软件渲染并以稳定可操作窗口为成功条件，仅失败时自动使用 no-sandbox 兼容重试且按版本记忆；CCSwitch 使用子进程级 WebView2 软件渲染、两秒稳定复核、恢复/屏内钳制/前置主窗口；升级时哈希刷新本地看门狗，消除旧 helper 状态格式残留。源码、真实 MSIX 解包事务和正式整包均已验证，目标机只需验收真实 GPU/前景窗口行为。
24. [完成（待用户实机确认）] v0.5.24 卸载许可证收口：把 U 盘根目录的 `usb-license.json`/`.bak` 从“未知发行文件”改为按活动租约、物理卷、完整指纹和配置签名验证的可变产品元数据；二阶段计划 schema 3 固化许可证快照，许可与重试入口最后删除；未知项目逐项列名并继续 fail closed。定向、全量、真实 MSIX、Windows PowerShell 5.1 和新版本整包验证均通过，仅剩目标机实际卸载生命周期验收。

