# Skill Hub 无期限等待与卸载二阶段竞态修复（G32）

## 1. 任务与结论

本轮处理两个现场问题：

1. 点击 Skill Hub“检查更新”后长期停留在“正在检查…”，没有明确失败出口。
2. 卸载主界面显示已完成，但随后弹出“二阶段卸载未成功”。

当前代码修复结论：两条链路的无限等待/错误交接路径已收紧；仍需在真实 Windows 测试机上做一次带实际卸载的端到端验收，才能把“现场不再复现”作为最终结论。

## 2. 基线与范围

- 工作区：`codex-desktop-usb-installer`
- 基线提交：`6e38cfb`（工作区已有大量用户改动，本轮不回退、不覆盖）
- 本轮唯一写入范围：Skill Hub 检查请求、卸载收尾 helper 交接、对应回归测试和本记录。
- 不改变：USB 授权/签名规则、Skill Hub 来源白名单与校验、Codex 更新逻辑、U 盘内容清理范围。

## 3. 根因确认

### 3.1 Skill Hub

前端 `SkillRepoUpdateCard` 直接等待 `skillRepoCheckUpdate()`；此前没有 renderer 侧 watchdog。只要旧运行时、Tauri invoke 通道或某个异常 transport future 永不 settle，按钮就会永久显示“正在检查…”。已有测试甚至把“50 秒后仍保持 checking”当成预期，因此没有覆盖用户可恢复出口。

后端虽然对来源阶段设置了 deadline，但 reqwest client 没有统一 request timeout，检查命令也没有在来源解析边界增加独立的 outer timeout。这使得阶段超时保护与实际命令返回之间存在盲区。

### 3.2 卸载二阶段

`CodexUninstallCleanup.exe` 启动 PowerShell finalizer 后，`finalizer.ready` 只表示 finalizer 已接管并等待 `lifetime.lock` 释放。旧流程释放锁后继续调用 `confirm_finalizer_report`，而 helper 自身仍在运行并持有自己的 EXE 映像。finalizer 随即删除 helper 文件，Windows 可能因映像占用返回 sharing violation；helper 再把这个竞态误判为二阶段失败，于是用户看到“主流程完成 + 最后失败弹窗”。

## 4. 修复设计

### 4.1 Skill Hub 检查

- renderer 保留 50 秒 watchdog：超时转换为稳定的 `SKILL_REPO_TIMEOUT`，恢复按钮状态，显示可展开详情和重试入口；该值比后端 45 秒总期限加 2 秒 join grace 略长，确保当前后端错误先返回。
- watchdog 不假装取消 native invoke；它只防止界面永久卡死。后端硬超时保证正常新版本会在 watchdog 前结束；旧/异常运行时即使迟到，结果也不会重新覆盖超时错误。
- reqwest client 增加 45 秒 request timeout。
- `check_update` 的来源解析增加 45 秒 + 2 秒的 outer deadline，记录 `skill_hub_check_start` / `skill_hub_check_timeout`，便于区分网络阶段和运行时异常。
- 所有源失败仍保持 fail-closed：不修改本地能力包，并保留来源尝试摘要。

### 4.2 卸载二阶段

- `schedule_self_cleanup` 返回成功后，helper 只做两件事：释放 `lifetime.lock`，立即返回成功并由启动入口退出进程。
- finalizer 负责等待锁关闭、删除 helper 自身、更新持久报告、展示真实失败提示及登记重启兜底。
- 仅在 finalizer 未就绪/启动失败时由 helper 进入失败报告和重启兜底路径。
- 不再在“已 ready、但 helper 尚未退出”的窗口内读取最终报告；这正是此前触发自循环失败的竞态点。

## 5. 修改文件

- `src/app/views/SkillRepoUpdateCard.tsx`
- `src/app/views/SkillRepoUpdateCard.test.tsx`
- `src-tauri/src/app/skill_repo_update.rs`
- `src-tauri/src/app/uninstall_cleanup.rs`

## 6. 验收矩阵

| ID | 场景 | 期望 |
| --- | --- | --- |
| G32-S1 | Skill Hub 请求成功 | 展示版本信息，按钮恢复可用 |
| G32-S2 | Skill Hub 来源返回错误/超时 | 在后端 deadline 内返回稳定错误，展示来源诊断，可重试 |
| G32-S3 | renderer 收到永不 settle 的旧 invoke | 最多 60 秒后离开 checking，展示 timeout，迟到结果不覆盖错误 |
| G32-S4 | 正常卸载，二阶段 finalizer ready | helper 释放锁后立即退出，finalizer 删除自身并写 completed 报告，不弹失败框 |
| G32-S5 | finalizer 未启动或 helper 文件确实被占用 | 报告记录具体失败项并走既有重启兜底，不宣称彻底完成 |
| G32-S6 | `remove_usb_media=false` | 二阶段修复不扩大 U 盘删除范围 |

## 7. 防回归规则

- 任何会等待 native/Tauri Promise 的前端动作都必须有明确的 UI 超时出口；测试不得再把“无限等待”作为成功标准。
- 任何自删除 helper 都必须先完成 handoff，再释放生命周期锁并退出；不得在 helper 仍运行时确认其自身最终报告。
- “已完成”只能表示主清理已完成或已安全交接；finalizer 的失败只能由 finalizer 的持久报告和真实失败项判定。
- 网络失败必须可见、可重试、可诊断；不能用永久 spinner 掩盖来源或 transport 问题。

## 8. 已执行验证

- Skill Hub 前端定向测试：20 项通过。
- 前端全量测试：36 个测试文件，421 项通过、1 项既有跳过。
- TypeScript 检查、Lint、生产构建和 renderer entry isolation：通过。
- Skill Hub Rust 定向测试：20 项通过。
- 卸载清理 Rust 定向测试：39 项通过。
- Rust 全量库测试：417 项通过、2 项既有忽略；`cargo fmt --check` 与 `cargo clippy --lib -- -D warnings`：通过。
- 关键静态不变量审计：`UNINSTALL_HANDOFF_AUDIT=PASS`、`SKILL_HUB_WATCHDOG_AUDIT=PASS`。

## 9. 剩余现场边界

真实 Windows 端仍需人工确认：运行管理器卸载、等待二阶段收尾、确认临时 helper 目录消失、确认报告为 `completed` 且无失败弹窗；另需用断网或阻断来源场景确认 Skill Hub 在限定时间内落到可重试错误。上述现场步骤未完成前，不把“实测完全不复现”写成已证明事实。
