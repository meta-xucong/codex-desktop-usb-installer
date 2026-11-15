# Codex 26.1111.21308 USB 兼容适配

## 范围与证据

旧版本 26.1016.62342 的精确补丁无法匹配 26.1111.21308；新 CUA native-pipe resolver 不读包内 helper 覆盖路径，新 IAB 显示调用没有等待实际界面状态。详细证据见同目录 `USB-OLD-MANAGER-UPGRADE-COMPATIBILITY-1115-AUDIT.md`。

只适配这些已确认调用链及实测暴露出的同版本必要启动依赖。保留现有媒体 MCP、能力包、账号/设置保留、四阶段进度和无代理 Relay 更新流程。原始 MSIX、签名机制与 helper 实现不改。

## 最小实现

1. 在现有 portable 兼容处理旁增加新版本的精确片段，保留旧片段。继续使用既有定长 ASAR 替换和完整性更新；不在运行时做模糊替换。
2. CUA：使 native resolver 消费现有 `OAI_SKY_WINDOWS_BIN`；允许现有 v3 helper 的 notify 包装被识别；Sky 截图前调用既有窗口激活入口。
3. IAB：显示命令传递显式等待标记、host 返回 promise；复用现有 browserUseOpenRequests waiter，只在真实 visible/bounds 和对应窗口/标签一致后完成；显式显示恢复拥有窗口并选择正确面板/路由。普通非等待调用保留原语义，超时沿用原 waiter，不新增时限。
4. USB 部署/启动前确认每个必要组是受支持原始形态或已修补形态，修补后确认目标形态。缺失时走明确兼容错误，部署阶段不得被既有“延期到启动再修补”的 catch 吞掉，沿原安装事务保留旧程序；不把进程存活作为功能修补成功。
5. 旧版原始与已修补状态均纳入检查；非 USB portable 路径保留原行为。
6. 新版实际首次启动暴露 Owl early-bootstrap 在 Electron native app 路径初始化之前调用 `app.getPath("appData")` 并退出 13。仅对这个新版本精确片段，使用原桌面用户的 `APPDATA`，保留既有显式 user-data 路径及应用名称后缀。正常生产启动入口已验证，不在产品中新增诊断启动参数。
7. 新版 non_coding 引导仍向配置写入 PROSE，旧函数形态已不匹配。适配新版实际 renderer 的原分支，只将其 `L.O$` 改为 `L._e`；已核对 qixt/r8WG 导出分别是 PROSE/COMMANDS，其他角色、字体、权限显示不改。该组加入新版必要补丁验证。新版工作模式另有 unsupported-auth→coding 的现有消费者，不盲目移植旧 `/work` 函数。

## 验证与发布条件

- 新旧真实 ASAR 片段匹配，所有替换字节长度不增加；新 JS 完整语法解析通过。
- IAB 行为测试覆盖：set 在状态确认前不完成，正确窗口/标签后完成，错误标签不能放行，hide/普通调用、旧版重入保留；CUA 覆盖 helper 路由、notify 及截图激活。
- 真实安装事务覆盖：旧→新升级成功；缺失必要片段时报失败并保留原安装；重复启动不会重复改坏。
- 隔离 HOME 对比模型/profile/MCP/媒体设置，实际新 Codex 原生 node_repl、CUA 新窗口索引/坐标点击/输入回读/遮挡截图、IAB 同标签十分钟显示与读取、媒体只读 capabilities/目录 GET，以及正常退出重启。
- 复用既有定向更新源和前端回归，检查新包实际内容与原包差异，配套 preload10 签名器配对。未经真实运行回归不得发布为通过。

## 审计口径

本次修改以开始时备份为差异基线，保留既有未提交改动。未获本次授权前不执行 SHA/hash/checksum；授权后仅使用既有完整性、编译和发布流程。不得进行真实签名、登记、充值或付费媒体任务。
