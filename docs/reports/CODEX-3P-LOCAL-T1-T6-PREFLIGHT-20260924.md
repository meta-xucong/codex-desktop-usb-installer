# Codex U 盘本地 T1–T6：协作开发与源码审查记录

日期：2026-09-24（用户时区 +08:00）
当前状态：`CODE_ONLY_WITH_OPEN_FINDINGS_NOT_ACCEPTED`；源码 v3 已保留，仍有代码阻断项和独立审计缺口。第 1–12 节为历史记录，当前交接以第 13–15 节及 source-manifest-v3.json 为准。

## 1. 依据与执行边界

项目：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`。
核心依据：`docs/CODEX-THIRD-PARTY-MODEL-COMPATIBILITY-DEVELOPMENT.md`，文档版本 1.0.0、契约 `CODEX-3P-COMPAT-v1`。
核心文档 SHA256：`EB642E6AD8000782D7137F37FE46446FD985238B21FBB45D75CF9035EB7E9DDB`。
已完整读取本机 `multi-agent-dev` 的 SKILL.md、adaptive-four-role-workflow.md 和 model-routing-hard-gate-development.md。
Skill 根目录：`C:\Users\T14S\.codex\skills\multi-agent-dev\skills\multi-agent-dev`。
协作契约：`v1.3.1-route-guard`；`COMPLEXITY_GATE=ESCALATE_REQUIRED`。
本轮预检在派发前阻断；`CONTEXT_MODE=NOT_NEEDED`，不需要压缩或恢复流程。
用户本轮明确允许先做本地 mock 实现和验证；T0 不作为本地开工前置，仍阻断最终真实兼容性验收。

用户原文边界：
> 本次只开发 Codex U 盘本地侧 T1-T6，不开发 T0，不修改 Sub2API/VPS。
> T0 会在其他会话中单独开发。等 T0 完成、我完成真实模型验证后，再进行最终联调。
> 必须使用 multi-agent-dev skill 协作开发。

## 2. 起始基线与已有改动保护

主仓库 HEAD：`e9514d08c343f594b0e452617bdf4a45bc0216b0`。
分支：`archive/skill-pack-1.3.0-20260924`；起始工作区干净，记录了 872 个文件 SHA256。
内嵌 CCSwitch HEAD：`1d44b1ba41987cddeb8096fadba8190fab465cbb`；记录了 894 个文件 SHA256。

内嵌仓库已有 9 个已跟踪文件改动、3 个未跟踪文件；它们均属于起始基线，不属于本轮成果：

```text
 M package.json
 M pnpm-workspace.yaml
 M src-tauri/src/lib.rs
 M src-tauri/src/proxy/http_client.rs
 M src-tauri/src/proxy/providers/codex.rs
 M src-tauri/src/services/provider/mod.rs
 M src/components/providers/forms/CodexFormFields.tsx
 M src/components/providers/forms/shared/ModelInputWithFetch.tsx
 M src/lib/api/model-fetch.ts
?? src/lib/model-aliases.ts
?? tests/components/ModelInputWithFetch.test.tsx
?? tests/utils/modelAliases.test.ts
```

基线文件：`.codex-audit/local-t1-t6-20260924/baseline.json`。
基线文件 SHA256：`c734ba1b36d0754e47e6c297bb749142ee06ffb0b19c6dae159d06abc87393af`。
结束前逐项重算上述 1,766 个文件哈希：`root=[]`、`ccswitch=[]`，没有发现本轮对既有文件的变更。
主仓库干净不能代表内嵌仓库干净；后续每次派发仍须分别核对两个仓库。

## 3. 实测协作环境阻断

本机 CLI：`codex-cli 0.153.4`。`codex features list` 显示 `hooks=true`、`multi_agent=true`。
本地配置中的 Luna/high 仅是 CLI 配置，不能证明当前 ChatGPT 主控或尚未创建的子 Agent 实际路由。

当前 `C:\Users\T14S\.codex\hooks.json` 指向：
`C:\Users\T14S\.codex\skills\multi-agent-dev\hooks\pre_spawn_route_guard.py`。
该文件不存在；实际文件位于：
`C:\Users\T14S\.codex\skills\multi-agent-dev\skills\multi-agent-dev\hooks\pre_spawn_route_guard.py`。
Windows 和非 Windows command 均缺少中间的 `skills/multi-agent-dev` 路径。

用本机当前 CLI 生成 schema 后，启动独立 app-server，仅调用 initialize、initialized 和 hooks/list；没有创建线程/回合或发起模型请求。
运行时返回 `enabled=true`、`trustStatus=untrusted`，加载的 command 仍是不存在的路径。
该只读 app-server PID 74248 已正常退出，退出码 0。
直接运行配置中的脚本路径，Python 报文件不存在，退出码 2。
实际脚本的 22 项标准库单测全部通过，退出码 0；这不是真实派发回执，也不能代替用户信任。

## 4. DESIGN_QUESTION ENV-001

触发：用户要求必须使用 multi-agent-dev；其硬门禁要求有效脚本、已信任 hook 和真实派发 receipt。现有运行时不满足。
现行契约：未信任或缺失 receipt 必须 fail-closed；不能通过禁用 hook、绕过信任或伪造 receipt 放行。
影响：阻断执行 Agent、依赖它们的 T1–T6 实施及独立审计，不是 T0 尚未完成造成的阻断。
最小范围外修复建议：仅将用户级 `hooks.json` 中这一个 hook 的 command/commandWindows 脚本路径修正为实际位置，其他配置不变。
所需授权：该文件不在用户指定项目目录内，本轮尚未获准修改，故保持原样。
所需用户操作：修正后，在 Codex 的 `/hooks` 中审阅并信任该 hook；不能由本会话绕过信任机制。
恢复门：重新核对 hooks/list，使用符合 Skill 主控/派发路由的新任务，取得实际 MAD_ROUTE_RECEIPT，并重新比较两个仓库基线后才派发 T1。
新任务路由和 receipt 尚未验证；不得声称修正路径就已经恢复完整协作能力。
不受影响的工作：只读检查、保存基线、记录问题与分工准备。本轮仅完成这些工作。
结论：`OPEN / NEEDS_USER_AUTHORIZATION_AND_HOOK_TRUST`。

## 5. 任务状态、写入归属与验收

当前会话只负责本预检记录及审计辅助产物；没有生产代码写入者，没有派发任何子 Agent。
`WRITER_STATUS=NOT_STARTED`（T1–T6）；`AUDIT_OWNER=UNASSIGNED`；`AUDIT_STATUS=NOT_STARTED`。

| 任务 | 修改文件 | 起始/最终产品基线 | 任务测试/退出码 | 未验证项 | 验收 |
| --- | --- | --- | --- | --- | --- |
| T1 | 无 | 相同，详见 baseline.json | NOT_RUN / N/A | 能力状态、保守目录、sidecar | 未达到 |
| T2 | 无 | 相同 | NOT_RUN / N/A | 自动探测、状态区分、失败恢复及重试 | 未达到 |
| T3 | 无 | 相同；保留 CCSwitch 原改动 | NOT_RUN / N/A | 默认模型、绑定隔离、资源一致性 | 未达到 |
| T4 | 无 | 相同 | NOT_RUN / N/A | 脱敏诊断字段及泄漏反例 | 未达到 |
| T5 | 无 | 相同 | NOT_RUN / N/A | mock gateway、全部本地回归及门禁 | 未达到 |
| T6 | 无发布包变更 | 相同 | NOT_RUN / N/A | 集成打包、安装启动、独立审计 | 未达到 |

六项任务共同受 ENV-001 阻断；尚未进入产品公共 API/schema 实现，未产生产品契约变更。
T1 → T2 → T3/T4 → T5 → T6 的推荐顺序保留。T5 执行者须与 T1/T2 作者分离，最终审计须与所有被审改动作者分离。

## 6. 已执行的预检命令与结果

- `git status --porcelain=v1`、`git rev-parse HEAD`（分别在主仓库和 CCSwitch/CCSwitch）：退出码 0。
- `codex app-server generate-json-schema --experimental --out .codex-audit/local-t1-t6-20260924/app-server-schema`：退出码 0。
- `codex features list`：hooks 和 multi_agent 为 true；不是 hook 已信任的证明。
- `python.exe -B C:\Users\T14S\.codex\skills\multi-agent-dev\hooks\pre_spawn_route_guard.py`：退出码 2，文件不存在。
- `python.exe -B -m unittest discover -s C:\Users\T14S\.codex\skills\multi-agent-dev\skills\multi-agent-dev\tests -p test_pre_spawn_route_guard.py`：22 tests，退出码 0。
- app-server `hooks/list`：trustStatus=untrusted；只读进程正常退出 0。
这些都是预检，不是 T1–T6 的 Rust/TypeScript 回归，不构成产品验收。

## 7. 用户硬约束原文

1. 不修改 Sub2API、VPS 或其他外部服务。
2. 不修改 Codex 官方二进制或官方前端资源。
3. 不增加固定模型白名单。
4. 不把模型能显示等同于模型已兼容。
5. 不泄露任何密钥、账号密码或完整请求内容。
6. 每个任务必须有唯一写入负责人。
7. 发现需要改变公共 API、schema、状态、事件、安全或验收标准时，先提交 DESIGN_QUESTION。
8. 不要覆盖当前工作区已有改动；先建立文件基线。
9. T0 未完成、真实网关未验证前，只能报告本地实现完成，不能报告最终兼容完成。

本轮连本地实现也尚未完成，不能使用上述第 9 条作为宣布本地完成的理由。
未修改 Sub2API/VPS、Codex 官方文件、用户级 hook 配置或既有发布包；未执行真实模型联调。
未暂存、提交、推送或部署；没有使用 trust bypass，也没有伪造路由 receipt。
ServerChan 通知：`NOT_SENT_SCOPE_BOUNDARY`。未调用外部通知服务，未读取通知密钥。

## 8. 证据位置

- `.codex-audit/local-t1-t6-20260924/baseline.json`：两仓库提交、起始状态、逐文件 SHA256。
- `.codex-audit/local-t1-t6-20260924/runtime-preflight.json`：实际 hooks/list、退出码、测试计数及基线复核。
- runtime-preflight.json SHA256：`fa6c1bf2ee31e375a7cd064cf4d78a5aee6c1ba5d39e64d01ab24ccdf8d58e2e`。
- `.codex-audit/local-t1-t6-20260924/app-server-schema/`：本机 CLI 生成的协议定义，仅作只读诊断依据。
- 本文件为唯一预检任务记录；当前终态是阻断，不是开发或独立审计完成。

## 9. ENV-001 授权后路径修复（2026-09-24T00:48:45.567929+00:00）

- 用户已授权仅修正 hooks.json 的 command / commandWindows 两处路径；唯一写入者：当前主控会话。
- 两处路径已补入 skills/multi-agent-dev；其他 JSON 字段及原始格式保持不变，未修改信任配置。
- 备份：`C:\Users\T14S\.codex\hooks.json.bak-env001-20260924T004731005061Z`。
- 修复前 SHA256：`3d9be026d7a9115e8fc82ed3892474fcfac4864bab26938f9e159c43d05c5dee`；修复后 SHA256：`864577d2244b3f07a046323cf66133b1572e89ccb3059a03a9a931ea9d6a8ddd`。
- 直接执行修正后的 Windows command：退出码 0，非 spawn 输入 stdout/stderr 均为空。
- 路由脚本单测：22/22 通过，退出码 0；非 Windows command 仅完成路径/JSON 静态核验，未在非 Windows 系统运行。
- 新的只读 app-server hooks/list 已加载修正后的 Windows command；enabled=true、trustStatus=untrusted；进程退出码 0。
- 本轮修复前后主仓库与 CCSwitch 的 HEAD/Git 状态一致：{"root": true, "ccswitch": true}；仅随后追加本节证据记录。
- WRITER_STATUS=PATH_REPAIR_COMPLETE_WAITING_USER_HOOK_TRUST；DESIGN_QUESTION=ENV-001（路径问题已解决，信任待用户完成）。
- 未启动执行 Agent；未生成真实派发 receipt；独立审计未开始；T1-T6 未开始，未改变产品代码/发布包或执行外部联调。
- 后续：用户在 Codex /hooks 中审阅并信任该 Hook，随后重新核验信任和实际派发回执；不使用 trust bypass。
- 本轮证据：`.codex-audit\local-t1-t6-20260924\env001-path-repair-20260924T004731005061Z.json`。


## 10. 用户确认信任后的运行时复核（2026-09-24）

- 实测 hooks/list：enabled=true，trustStatus=trusted；脚本路径正确。ENV-001 的路径和信任问题均已解决。
- 本轮新基线：`.codex-audit/local-t1-t6-20260924/resume-20260924T014638Z/baseline.json`。主仓库 873 文件，CCSwitch 894 文件。
- 主仓库新增的 `docs/G39.5-R7-FIELD-ISSUES-REPAIR-DEVELOPMENT.md`、`src-tauri/src/app/install_tx.rs`、`src-tauri/src/commands.rs` 改动属于用户现有改动；已纳入新基线并保存保护副本，不得覆盖。
- 创建了只读原生工具宿主，显式模型 gpt-5.6-luna、effort=max、read-only；没有执行产品代码任务。
- 向宿主提交首个子智能体派发任务的工具请求被 OpenAI 平台安全检查阻止。未取得子智能体 ID 或 MAD_ROUTE_RECEIPT；没有换工具、伪造回执或绕过检查重试。该阻断不是 hooks/list 的信任失败，具体安全判断原因未提供。
- 已关闭本轮只读 app-server，退出码 0；没有留下本轮启动的执行 Agent。
- 记录更新前，两仓库本轮基线文件哈希均未变化。只新增审计证据并追加本任务记录；未修改产品代码、发布包、Sub2API/VPS、官方 Codex 文件或信任配置。
- T1–T6：WRITER_STATUS=NOT_STARTED；测试 NOT_RUN/退出码 N/A；AUDIT_OWNER=UNASSIGNED；验收均未达到。
- 本轮没有产品 DESIGN_QUESTION 或产品 schema 改动。协作派发受平台工具安全检查阻断；不得将单会话分析包装成 multi-agent 开发或独立审计。
- 证据：同目录 `hooks-trusted.json`、`baseline.json`、`resume-result.json`。
- ServerChan：NOT_SENT_SCOPE_BOUNDARY；未读取通知凭据、未调用外部通知服务。

## 11. 本轮本地实现（2026-09-24）

- 用户指令：“你继续，完成所有开发”。T0/真实网关/外部服务仍不在范围。
- 基线：`.codex-audit/local-t1-t6-20260924/implementation-20260924T024516Z/baseline.json`；两仓库既有修改均保留。
- `COMPLEXITY_GATE=ESCALATE_REQUIRED`；`CONTEXT_MODE=ARMED`。
- 已重新读取 multi-agent-dev；前次派发被平台安全检查阻止，不绕过、不从另一路径重做被阻止的派发。
- 当前主控不能声称已切换为 Skill 指定的 Luna。按 Skill 第 2 节不可用分支推进授权内实现/作者自测，独立审计保持未完成。
- 唯一实现写入者：当前会话（T1/T2/T3/T4）；`WRITER_STATUS=ACTIVE_SELF_IMPLEMENTATION`。
- T5 独立测试、T6 独立审计负责人尚未取得；作者测试不得冒充 T5 独立验收。
- 不修改 install_tx.rs、commands.rs 的既有用户改动；不修改官方 Codex、Sub2API、VPS 或用户级信任配置。

### DESIGN_QUESTION DQ-LOCAL-001：目录状态与能力状态

旧接口 ready/fresh 只证明目录写入/缓存新鲜；用户 T1/T2 明确要求不能作为 Code Agent 就绪。
主控在实现前冻结用户已授权的本地设计：保留安装状态及 command 参数，增加独立能力报告；发现成功只报 discovered/unverified。
自定义状态只写 sidecar；原生格式未验证时解除本管理器的目录引用并保留用户配置，不注入未经验证的 Codex 字段。
旧目录/sidecar 不能自行恢复成已验证；缓存先检查授权及绑定；绑定只保留非明文指纹。
Gateway 探测通过不证明 Shell、Skills 或原生 Code Agent 已验证；证据不足保持 unknown，不产生 ready 默认模型。
本决定执行用户原文，不改变上游协议、不放宽验收。后续超出该边界的契约变化仍须另提 DESIGN_QUESTION。

### 修改及证据边界

T1：model_catalog、能力类型、作者 Rust 单测；T2：本地探测、既有返回类型、OfflineHome 及作者测试。
T3：provider 资源/自定义模板和必要生成链，不批量改其他应用的预设；T4：仅脱敏元数据，不存正文。
无独立审计前不发布正式包、不标记 ACCEPTED、不提交或推送。

## 12. 本轮终态：草稿保留，产品基线恢复

`WRITER_STATUS=STOPPED_BASELINE_RESTORED`；`AUDIT_OWNER=UNASSIGNED`；`AUDIT_STATUS=NOT_RUN`。
本轮没有完成 T1–T6，没有正式发布包，没有最终第三方兼容性结论。

实际草稿：T1 的保守发现、能力状态 sidecar、绑定指纹/缓存隔离、旧托管目录停用及作者单测；T4 的元数据诊断构造器及作者单测。
T4 模块尚未接入调用链或注册编译，不能称为实现完成。
草稿编译遇到 `E0277`：本仓库 sha2 0.11 的 digest 输出未实现 LowerHex；出错位置是草稿 model_catalog.rs 的 digest 十六进制格式化。
后续最小修正的 interact_with_process 请求被 OpenAI 安全检查拦截；没有退出码、没有执行该修正，也没有改工具/换路径重试该修正。

为不留下不能编译的产品代码，已先保存草稿，再仅撤回当前会话自身的产品改动；恢复操作逐项校验草稿与原始备份 SHA256。
草稿位置：`.codex-audit/local-t1-t6-20260924/implementation-20260924T024516Z/unaccepted-draft/src-tauri/src/app/`。
草稿含 model_catalog.rs、model_capability.rs、model_diagnostics.rs、mod.rs。它们不是可发布或已验收版本。

| 文件 | 起始/恢复后 SHA256 | 未验收草稿 SHA256 |
| --- | --- | --- |
| model_catalog.rs | E96FD2E4806ECD1603222088C5A481124415DDD42FA50DD926D1C5FAD26A052B | 3AA561214B2BE71B77B4B2A162FCDF0B621A1ADD063EF701C4CF4A4E7A00744B |
| mod.rs | 31440711A17AE1E1E3B45B4C595FFEC4E63388E28B70715E11D2B55220F449A8 | 22580ABBF485F3AE4C710433560FD28EE4C1FEE3CE29AF9B96E3B2E0A65ABC12 |
| model_capability.rs | 原本不存在；已移出产品源码 | A606C31E7D2B55EF1B5E836A09E70E0E0AC872217F4D98872D1D94BCF954EDC5 |
| model_diagnostics.rs | 原本不存在；已移出产品源码 | 428BFEF26691B3C77D5B9B02BCF379FC16AB5EA9FBE0F5ED4D7475810B78AD89 |

最终逐文件对照：主仓库 872 文件、CCSwitch 894 文件无差异；仅本任务记录按预期更新。用户既有改动均保留。
证据：同次 implementation 目录下 baseline.json、rollback-result.json、final-baseline-comparison.json。

### 实际测试与退出码

- 改动前：`cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::model_catalog::tests -- --nocapture`，10 passed，退出码 0。
- 草稿格式化：`rustfmt --edition 2021 --config skip_children=true src-tauri/src/app/model_catalog.rs src-tauri/src/app/model_capability.rs`，退出码 0。
- 草稿：`cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::model_ -- --nocapture`，编译错误 E0277，退出码 101；新增单测未执行，不能算通过。
- 恢复后：再次运行上述原模型目录 10 项测试，10 passed，退出码 0。仅证明原基线恢复，不证明任何新功能完成。
- 日志：本 implementation 目录的 `t1-tests.log`、`restored-baseline-tests.log`；改动前日志在上一层 `model-catalog-baseline-tests.log`。

| 任务 | WRITER_STATUS | 最终产品差异 | 测试/退出码 | DESIGN_QUESTION | 未验证项/验收 |
| --- | --- | --- | --- | --- | --- |
| T1 | DRAFT_RETAINED_NOT_ACCEPTED | 无；恢复原基线 | 草稿编译 101；恢复基线测试 0 | DQ-LOCAL-001 已记录 | 草稿修正、完整测试及独立审计；未达到 |
| T2 | NOT_STARTED | 无 | NOT_RUN / N/A | 沿用 DQ-LOCAL-001，未实施 | 自动探测、界面和失败恢复；未达到 |
| T3 | NOT_STARTED | 无；CCSwitch 原改动保留 | NOT_RUN / N/A | 无新增 | 默认模型/资源一致性/绑定隔离整合；未达到 |
| T4 | DRAFT_ONLY_NOT_INTEGRATED | 无；未接入的草稿归档 | NOT_RUN / N/A | 无新增 | 调用链接入、测试及独立审计；未达到 |
| T5 | NOT_STARTED | 无 | NOT_RUN / N/A | 无新增 | 独立夹具与完整回归；未达到 |
| T6 | NOT_STARTED | 无；未生成发布包 | NOT_RUN / N/A | 无新增 | 集成、打包、独立审计及验收；未达到 |

未修改或调用 Sub2API/VPS；未运行真实模型请求；未修改官方 Codex 文件或既有发行包；未提交或推送。
原始第三方模型能力伪造问题仍在基线中，本次安全恢复不是该问题的修复。
ServerChan：NOT_SENT_SCOPE_BOUNDARY，未读取通知密钥、未调用外部通知服务。

## 13. Code-only continuation contract（2026-09-24）
用户原文：“给我一个继续推进的方案，我要把代码全部改好。然后测试我会在本地跑，这个你不用管。只管把代码部分修改、审计好”；随后确认“按你的方案，继续往前推进”。
本轮契约：CODEX-3P-COMPAT-v1/code-only-r1；COMPLEXITY_GATE=ESCALATE_REQUIRED；CONTEXT_MODE=ARMED。
不运行编译、测试、安装、打包或真实模型请求；测试代码仍交付。运行验收保留，执行责任转交用户。
基线：.codex-audit/local-t1-t6-20260924/code-only-20260924T081359Z/baseline.json。
两个仓库的原有未提交改动均为基线；每次写入前比较原始字节，发生并发变化则停止该文件写入。
唯一产品/配套代码写入者：本会话。无可用原生子 Agent 派发工具；沿用 skill 的不可用分支，不能把作者自检称为独立审计。
AUDIT_OWNER=UNASSIGNED；独立原生只读 review 如可执行则另行记录真实执行身份和结果，不绕过已发生的工具阻断。
T5 独立作者要求目前未满足；作者补充的回归代码不冒充独立验收。T6 不生成安装包，不发布或推送。

### DESIGN_QUESTION DQ-LOCAL-002：经用户方案确认后的本地接口约定
依据：核心文档 6.1–6.5，用户确认的推进方案第三至六节。仅落实本地职责，不发明 T0 接口。
保留 refresh_native_model_catalog 的 force 参数；增加可选 requestId 及同 ID 取消命令，避免迟到结果和重入写入。
目录刷新状态与 capability 状态分离；返回逐模型检查、gatewayStatus、readyCount、quarantinedCount、runId。
sidecar 保存 schemaVersion、绑定非秘密指纹、观测时间、版本和逐模型证据；不得保存 Key、密码或正文。
磁盘缓存只用于发现/诊断，读回后不能作为 ready 的认证输入；mock/gateway 证据不代替 CodexRuntime 证据。
ready 由统一纯函数对完整检查、当前绑定/版本/时效和内存中的原生验证证据计算，不接受任意 ready 字符串。
原生目录格式/原生 Shell/Skills 缺少真实证据时保持 unknown，停用本管理器旧目录引用，保留用户配置。
不实现新的证书服务、第三方协议转换器、模型白名单、远程诊断端点或真实网关自动验收旁路。
探测仅使用合成文本及无副作用的 echo 函数；不得执行模型返回的任意 Shell；原生运行时检查单独标注待用户证据。
网络/超时失败可重试；协议结构失败仅隔离对应模型；有界预算耗尽的模型保持未验证，下次重试优先未完成项。
provider 资源、模板、USB bootstrap 同步去固定默认值；没有 ready 模型时不自动激活第三方默认模型。
诊断仅写约定元数据和稳定错误码；外部字符串不得经通用异常/对象 Debug 落日志；缺失 adapterVersion 保持 null。
WRITER_STATUS=ACTIVE_CODE_ONLY；测试=NOT_RUN_BY_USER_REQUEST/退出码 N/A；最终兼容结论仍待 T0 和真实验证。

## 14. Continuation / static-review repair（2026-09-24）
用户继续指令沿用第 13 节 code-only 契约；不运行编译、测试、安装、打包或真实请求。
已恢复同一作者的 Python 会话；没有活动子进程；28 个源码文件与 source-manifest-v2.json 全部一致。
本轮增量基线：code-only-20260924T081359Z/continuation-20260924T090811Z；仅在逐文件字节比较通过后编辑。
WRITER_STATUS=ACTIVE_CODE_ONLY；唯一写入者仍为本会话；AUDIT_OWNER=UNASSIGNED。
作者静态检查不能充当独立审计。保留既有阻断，不换工具重做被阻止的 bootstrap 修复、sidecar 测试或原生审计请求。

### Static findings and bounded repair scope
- SR-01：工具续接期望答案提前写在初始提示中，模型可以不消费工具结果仍答对；改用仅工具输出携带的随机挑战。
- SR-02：流式最终函数参数没有与实际 delta/done 参数完整比较；补齐 item/call/index/arguments 关联和反例。
- SR-03：取消与最终持久化之间缺少共同串行边界，且取消早于请求登记可能丢失；审查并修复取消生命周期。
- SR-04：前端接受不匹配/缺失运行身份及未知成功状态时应保持保守；审查迟到结果和状态卡片。
这些修正落实既有工具结果、流式完整性和取消语义，不增加上游协议或新的认证来源。
继续保留 T0/原生运行时证据生产、独立 T5 作者与最终独立审计的未完成边界。

### DESIGN_QUESTION DQ-LOCAL-003：取消确认与最终写入的竞争边界
触发：cancel_native_model_catalog 与最终 sidecar 提交使用不同锁；cancel 早于异步 refresh 登记时会丢失。
依据：已确认方案要求同 ID 取消、迟到结果丢弃、无重入写入；不增加 command 或改变返回类型。
主控按该授权冻结：true 表示取消已记录；已经完成的运行返回 false，不承诺撤销已提交结果。
取消登记与最终提交共用文件串行边界；提前取消使用有界待取消集合，不静默驱逐已确认的取消。
待取消集合满时返回 false/拒绝新运行，不能谎称已接受取消；已完成运行记录有界保留，UI 不复用运行 ID。
取消、完成、重复 ID、容量不足的纯内存状态机补充作者测试代码；不涉及 sidecar 边界测试或运行验收。
此约定仅消除现有异步取消缺口；测试执行仍由用户负责。
SR-01 写入中断：新增 helper 的工具请求被安全检查阻止；仅本轮未闭合的工具续接函数改动已恢复到 v2，不重试被阻止的修改。

## 15. Source v3 handoff：改动保留，审查仍有阻断项
当前源码状态：`CODE_ONLY_WITH_OPEN_FINDINGS_NOT_ACCEPTED`。
`WRITER_STATUS=STOPPED_VERSION_FROZEN_CODE_ONLY`；唯一代码作者为本会话，不是多名独立作者。
`AUDIT_OWNER=UNASSIGNED`；`AUTHOR_STATIC_REVIEW=REWORK_REQUIRED`；独立审计没有获得结果。
本轮在 v2 的 28 个任务文件中继续修改了 12 个；共计仍为 9 个新增、19 个修改文件。
未恢复旧草稿、未整批回滚本地实现；只恢复了 SR-01 中被拦截后无法闭合的函数修改。

### 已落盘修正（仅源码层确认，不是运行通过）
- SR-02：流式最终参数必须与 delta/done 字节一致，item/call/response 身份及 output_index 必须关联；拒绝重复索引、重复 item 和失败的完成事件。
- SR-03：提前取消有界登记，取消与最终提交共享串行边界；已提交运行不再返回取消成功；取消命令等待慢速磁盘时不阻塞 UI 线程。
- SR-04：界面拒绝缺失／不匹配的 runId、过期／未来／非法时间、重复模型和未知成功状态；取消未获确认时显示独立提示。
- SR-05：显示明确的网关不可用／授权失败及逐模型证据来源；修正“Key 已自动启用”的错误安装完成提示；浏览器预览返回同请求 ID 的 skipped 状态。
- 配套测试源码：新增流式反例、纯内存取消状态机反例、前端时效／重复行／身份／取消确认／网关状态／mock 来源用例。没有执行这些测试。

### 版本、起始基线与差异
主仓库起始 HEAD：`e9514d08c343f594b0e452617bdf4a45bc0216b0`；CCSwitch：`1d44b1ba41987cddeb8096fadba8190fab465cbb`。
本次 code-only 起始基线：`.codex-audit/local-t1-t6-20260924/code-only-20260924T081359Z/baseline.json`。
最终清单：同目录 `source-manifest-v3.json`，含每个文件 beforeSha256 / previousVersionSha256 / afterSha256。
清单 SHA256：`274885010e03f9501c1caaa8e13493d1f83c815a25e4e186039ed23cb6da438a`。
完整源码快照：同目录 `snapshot-v3/`；可读差异：`diffs-v3/`（仅差异文本规范化换行，快照保留原字节）。
核对了 1,766 个既有文件（不含任务记录）；19 个任务修改文件之外的 1,747 个文件哈希未变，无意外基线差异。
本轮进一步修改文件：model_catalog.rs、model_probe.rs、commands.rs、i18n.tsx、NativeModelStatusCard.tsx、OfflineHome.tsx、OfflineHome.test.tsx、nativeModelStatus.ts、nativeModelStatus.test.ts、managerApi.ts、managerApi.test.ts、shared/types.ts。

### T1–T6 逐项交接
下列全部任务的起始／最终文件基线见 v3 清单；运行测试状态统一为 `NOT_RUN_BY_USER_REQUEST`，退出码 `N/A`。
唯一写入负责人／WRITER_STATUS：本会话／`STOPPED_VERSION_FROZEN_CODE_ONLY`；独立 T5 作者及审计者要求尚未满足。
| 任务 | 主要修改文件（完整路径见清单） | 源码状态及未完成／未验证项 | DESIGN_QUESTION | 是否达到完整验收 |
| --- | --- | --- | --- | --- |
| T1 | src-tauri/src/app/model_catalog.rs、model_capability.rs、app/mod.rs | 保守发现、能力类型、sidecar、旧目录隔离已接入；真实原生目录生成及证据生产未接入，sidecar 边界覆盖不全 | DQ-LOCAL-002 | 否 |
| T2 | model_probe.rs、commands.rs、src-tauri/src/lib.rs、shared/types.ts、managerApi.ts、OfflineHome.tsx、NativeModelStatusCard.tsx、nativeModelStatus.ts、i18n.tsx | 合成探测、取消、逐模型展示已接通；工具续接判定 SR-01 未修完，原生 Shell/Skills 验证仍未知 | DQ-LOCAL-002/003 | 否 |
| T3 | providers.json、CCSwitch/CCSwitch/src/config/codexTemplates.ts、CCSwitch provider/mod.rs、provider/live.rs、src-tauri/src/lib.rs、资源生成脚本 | 固定默认值已删除，未验证默认值不自动激活；已有 provider 配置覆盖问题及已验证默认值自动激活接线未完成 | DQ-LOCAL-002 | 否 |
| T4 | model_diagnostics.rs、model_probe.rs 的诊断调用点 | 仅元数据诊断已接入；adapter 版本无 T0 可信来源，保持缺失；运行泄漏反例及独立静态审计未完成 | DQ-LOCAL-002 | 否 |
| T5 | model_probe_fixture.rs、各 model_* Rust 内嵌单测、OfflineHome.test.tsx、nativeModelStatus.test.ts、managerApi.test.ts、CCSwitch tests/utils/codexTemplates.compatibility.test.ts | 作者测试代码及回环 mock 已有；缺少独立作者、部分 sidecar 边界用例，且未执行 | 沿用上述约定 | 否 |
| T6 | tools/verify-model-provider-resource.ps1、build-ccswitch-with-env-patch.ps1、build-usb-layout.ps1、build-oneclick-usb-layout.ps1、src-tauri/resources/ccswitch/verify-product.ps1；v3 快照与清单 | 源码集成及资源一致性入口已修改；不构建新包；最终独立审计、用户本地运行和真实联调待完成 | code-only-r1 | 否 |

### 供用户本地执行的命令（本轮均未执行）
项目根目录：
```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::model_
npx --no-install vitest run src/app/views/OfflineHome.test.tsx src/app/views/nativeModelStatus.test.ts src/services/managerApi.test.ts
```
CCSwitch 子工程目录 `CCSwitch/CCSwitch`：
```powershell
pnpm exec vitest run tests/utils/codexTemplates.compatibility.test.ts
cargo test --offline --manifest-path src-tauri/Cargo.toml pending_usb_provider_cannot_get_a_default_from_shared_config
```
这些命令覆盖部分目标检查，不是完整发行验收清单；已有代码阻断项仍须先修复，不能用测试运行替代未实现的目标。

### 未解决的代码／独立审查阻断项
1. SR-01（model_probe.rs:231，tool_start/tool_finish）：初始提示已透露期望最终答案，当前工具续接用例不能充分证明模型消费了工具输出。补强 helper 写入被工具安全检查拦截；只恢复未闭合修改，不换工具重试。该缺陷未修复。
2. T3-PRESERVE（CCSwitch/CCSwitch/src-tauri/src/services/provider/mod.rs:1052）：bootstrap 重建 provider 后写入数据库，可能覆盖该 provider 已有自定义配置。上一轮该修复被拦截，当前未重试、未宣称已解决。
3. NATIVE-INTEGRATION（model_catalog.rs:644/659）：finalize_report 当前不接收真实 NativeRuntimeEvidence，默认选择收到空证据集合。能力判断函数存在，但生产证据采集、已验证原生目录生成及自动默认值激活没有完整接线；不能把永久 unknown 当作目标已实现。
4. SIDECAR-COVERAGE：上一轮额外 sidecar 边界测试代码被阻止，缺口保留；本轮新增的是取消登记纯状态测试，不冒充该覆盖。
5. INDEPENDENT-REVIEW：此前原生只读 review 没有结果，已停止进程树；无有效独立结论。作者自检不能替代 multi-agent 独立 T5 作者或 T6 审计。
原生 Shell／Skills、实际 Codex 目录字段、T0 adapter 元数据和真实模型联调均没有新增证据。不得报告最终第三方适配完成。

### 本轮实际执行的源码检查
- Rust：对 model_catalog.rs、model_probe.rs、commands.rs 的源文本调用 `rustfmt --edition 2021 --config skip_children=true --emit stdout`，3 个退出码均 0。输入走 stdin，未运行编译器／程序，也没有应用格式化输出。
- TypeScript：使用已安装 TypeScript 的 createSourceFile 对 11 个 TS/TSX 源文件做语法解析，进程退出码 0、解析诊断为空；没有执行 tsc、类型检查、构建或测试。
- 两个仓库的范围内已跟踪文件 `git diff --check -- <任务文件>` 均退出码 0；该检查不覆盖未跟踪文件，不能替代功能验收。
- 本轮所有编译、类型检查、Rust/TypeScript 测试、安装、打包、真实请求：`NOT_RUN_BY_USER_REQUEST`／退出码 N/A。
- 最新语法与哈希绑定证据：`code-only-20260924T081359Z/continuation-20260924T090811Z/syntax-evidence.json`。
- 差异基线保护证据：同目录 `baseline-protection.json`；定位记录：`review-locations.json`。
源码解析无错误只证明解析层面，不证明跨模块类型、行为、安全约束或独立验收全部通过。
未修改 Sub2API/VPS、官方 Codex 文件或既有发行包；未暂存、提交、推送、部署或调用外部通知服务。
ServerChan：`NOT_SENT_SCOPE_BOUNDARY`，未读取通知凭据。

## 16. Closure continuation（2026-09-24）
用户原文：“继续改，目标全部做完”。沿用 code-only-r1；不运行编译、测试、安装、打包或真实模型请求。
本轮起始：28 个任务源码均与 v3 清单相同；本轮基线在 `.codex-audit/local-t1-t6-20260924/closure-20260924T102606Z/`，两仓库共 1776 个文件哈希。
已确认前轮作者编辑进程停止；当前存在的终端 PID 75452 属于 alchemy_video_OS，不接管、不停止。
COMPLEXITY_GATE=ESCALATE_REQUIRED；CONTEXT_MODE=ARMED；唯一源码写入者=当前会话。
已读取 multi-agent-dev 及执行参考。当前没有原生 spawn 工具，不伪造路由 receipt 或指定模型运行证明。
按 skill 不可用分支推进授权内修改；独立审计仅在取得实际独立结果后计入，不以作者静态复核代替。
修复顺序：SR-01（工具输出随机挑战）→ T3-PRESERVE（既有 DB 配置保护）→ SIDECAR-COVERAGE → NATIVE-INTEGRATION 接线 → 最终快照与独立审计。
原始硬约束继续有效：不修改 Sub2API/VPS、官方 Codex 文件、不增固定模型白名单、不泄露凭据/正文、不覆盖他人改动。
API/schema/状态变更先登记 DESIGN_QUESTION；普通范围内修复无需再次授权。
WRITER_STATUS=ACTIVE_CODE_ONLY；TEST_STATUS=NOT_RUN_BY_USER_REQUEST；AUDIT_OWNER=UNASSIGNED。

### DESIGN_QUESTION DQ-LOCAL-004：USB provider 既有配置的原子保护
触发：bootstrap 调用 save_provider 会覆盖用户配置；简单 get 后 save 仍有并发丢失更新窗口。
依据：T3-PRESERVE 及用户已批准的“保留既有配置，仅更新必要绑定字段”；不改变公开 API、数据库 schema 或上游协议。
本地最小落点：CCSwitch 的现有 providers DAO 增加 crate 内部 USB 专用事务入口；存在时仅替换 auth.OPENAI_API_KEY，其他字段/模型/端点/顺序/元数据不变；不存在时插入 DB-only 初始 provider，不激活。
遇到无效 JSON/无效 auth 结构返回脱敏错误，不用新默认配置覆盖损坏记录。查询、修改在同一数据库事务/锁内完成。
主控确认这是既定配置保护要求的必要实现，不扩展到其他应用，不运行真实数据库修改；仅交付源码。
SR-01 产品源码已改为 function_call_output 内随机挑战；配套 mock/单测写入被工具安全检查拦截，未执行；不重试该被拦截调用或伪称测试完成。

### 本轮中间交接：T3 事务保护和实际独立审计
T3-PRESERVE 产品源码已接通：bootstrap 调用 prepare_usb_codex_provider；在同一数据库事务中读取/更新，既有记录只替换凭据字段。
追加安全边界：既有配置选择的 base_url 与绑定不同时保留整条记录，不注入设备凭据；对初始输入中的自定义端点/故障转移设置明确拒绝，避免误用为通用保存 API。
配套作者测试源码覆盖配置/元数据保留、非自动 current、损坏 auth 不覆盖、不同 endpoint 不注入新凭据；未执行。
首次实际独立审计线程：01a0d2f9-39db-7b32-8600-116bdf551445；CLI 请求 gpt-5.6-luna/max，read-only，进程退出码 0。
审计证据：本轮目录 t3-review-result.md、t3-review-events.jsonl、t3-review-input/ 固定源码快照。
审计原文静态层未发现当前 bootstrap 路径覆盖配置、自动 live 激活或敏感错误泄露；因未执行运行验证，总 verdict 为 insufficient evidence，不能写为完整验收通过。
审计提出专用 DAO 不应接受任意带端点/故障转移的 Provider；已收紧该输入边界。
较新的 endpoint 保护及输入边界已固定到 t3-review-input-v2；独立复审线程 01a0d308-836a-7091-876d-3a762cdf76de，当前等待结果；旧审计不能替代新版本复审。
原生集成的只读设计分析使用单独 CLI 会话，请求 gpt-5.6-sol/max、read-only；不授权该分析会话写入产品文件或运行测试/真实请求。
上述为独立 CLI 会话的实际证据，不是原生 spawn 工具派发；没有伪造 MAD_ROUTE_RECEIPT，不宣称当前 ChatGPT 主控模型已被切换。
T4 补齐了校验失败时的脱敏 response_id，以及工具续接失败的 call_id；不记录请求/响应正文。未执行泄漏测试。

### SR-01 配套源码与 T5 唯一写入者交接
SR-01 的后续明确局部编辑已成功落盘：model_probe_fixture.rs 解析新提示与 JSON challenge 输出；正常/多工具/错误输出均按 token 排序拼接挑战答案。
新增 ignored_tool_outputs_do_not_pass_continuation 和 output_challenges_are_fresh_and_bound_to_the_original_calls 测试源码；未执行。此前“配套源码尚未写入”状态在本节更新，不代表被拦截调用曾执行。
T5 独立写入者：通过本地 Codex CLI 启动的 Luna/max 专属测试会话，终端 PID 41056；唯一允许写入 src-tauri/src/app/model_catalog_storage_tests.rs。
该文件派发时不存在；主控不同时编辑。依赖的 model_catalog.rs/model_capability.rs 源码保持稳定，完成后由主控追加 cfg(test) 模块声明。
任务限定为测试源码编写和只读检查，不执行测试/编译/应用，不读取真实凭据、不改变全局 CODEX_HOME 或其他用户配置。
证据路径：本轮目录 t5-writer-brief.txt、t5-writer-events.jsonl；最终状态和哈希须取得实际结果后补记。

### T3-PRESERVE 独立静态复审完成
复审线程 01a0d308-836a-7091-876d-3a762cdf76de，终端 PID 72480 正常退出 0；结果在本轮目录 t3-review-result-v2.md。
原文结论：“静态复核结论（仅本次 delta）：代码层面符合，未发现可直接认定的源码缺陷。”
已审最终哈希：providers.rs = 6b759ad769b3b5c9af236a10f64b21c7b89bfeb896911f5af22f52d58a05bf35；provider/mod.rs = 91e966bd76d94f75b05579f0c74cb6a3b2b1c466467686573edeb73d0c6513d6。
覆盖：现有 config/元数据保留、事务写入前 endpoint 检查、拒绝非 USB 初始输入、稳定脱敏错误，以及对应回归源码。
限制：canonical TOML 结构以外配置 fail-closed；未执行测试/编译。该结论只关闭 T3-PRESERVE 的源码问题，不认证 ready 默认模型接线或整个 T1–T6。
T3-PRESERVE=SOURCE_FIXED_INDEPENDENT_STATIC_REVIEW_CONFORMING；运行验收仍 NOT_RUN_BY_USER_REQUEST。

### SR-01 固定快照送审状态
工具续接及诊断的最新源文件已固定在本轮目录 probe-review-input/，附 probe-review-brief.txt。
独立只读审计启动请求被工具安全检查拦截：没有执行、没有取得新线程 ID 或结果，不把送审材料存在称为审计完成。
SR-01 当前为 SOURCE_FIXED_AWAITING_INDEPENDENT_REVIEW；正常/反例测试源码已写入，运行状态仍为 NOT_RUN_BY_USER_REQUEST。
T3-PRESERVE 的已取得独立静态结论不受此影响，也不被扩用到 SR-01 或其余任务。

### T5 存储边界测试源码完成与接入
独立写入者线程 01a0d30c-aaad-7b00-9643-b2ff0cd7b331；终端 PID 41056 正常退出 0，结果 WRITER_STATUS=STOPPED；不是产品代码作者。
唯一写入文件：src-tauri/src/app/model_catalog_storage_tests.rs，SHA256 cd507b71af006ebc7369136842640cc7ec42d74d19b8f11dab5bcebdafd9be4e。
结果文件：本轮目录 t5-writer-result.md；日志 t5-writer-events.jsonl。完成后由主控加入 model_catalog.rs 的 cfg(test) 模块声明，原文件由独立写入者所有，主控未改。
11 项测试源码：sidecar 所有权、非托管文件保护、损坏 JSON/未知 schema、大小/数量上限、绑定与版本不匹配、磁盘 ready 去权、未来时间戳、用户配置保留与拒绝误删、目录冒充文件。
测试使用各自 RAII 临时目录、合成值；不修改 CODEX_HOME、不调用 USB/真实模型/全局清理状态。独立作者源码复核未发现产品缺陷。
所有测试与编译仍 NOT_RUN_BY_USER_REQUEST/退出码 N/A。独立作者进程退出 0 仅代表源码任务完成，不是测试退出码。
主控接入后全基线字节核对无范围外变化。SIDECAR-COVERAGE=SOURCE_ADDED_BY_INDEPENDENT_T5_WRITER；完整 T5 运行验收尚未授予。

### DESIGN_QUESTION DQ-LOCAL-005：原生证据产生及激活的安全边界
原生设计会话最终未完成：CLI 返回 usage limit，turn.failed；终端输出 NATIVE_DESIGN_EXIT=1（外层 PowerShell 会话正常结束），不能把中间意见当作最终设计通过。
已核实的协议事实：.analysis/schema-out/v2/ThreadShellCommandParams.json 明确声明 thread/shellCommand 是 unsandboxed/full access。
因此即使执行的是管理器固定命令，也不能单独以此证明模型驱动的 unified_exec/Code Agent 调用链；不以手工线程 Shell 替代目标验收。
现存 schema 支持 thread/start、turn/start 的 skill input、skills/list、model/list、commandExecution 事件与命令审批响应；ThreadStartParams 没有动态工具注册字段，不能臆造本地动态 echo API。
待冻结的最小原生链路：隔离 CODEX_HOME + 当前已安装官方 runtime；当前 parser 校验候选目录；Skill 内独有随机路径/固定只读表达式；仅允许与预生成安全表达式精确匹配的原生命令；以同线程原生输出和续接答案验证，不接受磁盘 ready 声明。
还须确定 adapter 版本的非臆造来源及经验证选择到现有 CCSwitch live writer 的唯一激活通道，不能只接一个永远传空证据的函数。
这些路径在设计闭合前保持未接通，不降低 ready 条件、不修改官方二进制、不增加默认模型白名单。

### DESIGN_QUESTION DQ-LOCAL-006 / SR-06：能力包同步脚本仍自动选择 GPT
新增证据：installer-pack/sync-model-catalog.ps1:160–202 会在 model 缺失/退役/隐藏时写入 bundled preferredModel；install-to-codex.ps1:509–529 在安装能力包时调用它。这会绕过本轮 T3 的 ready 门禁。
主控按既有“无 ready 不自动选默认模型、保留用户配置”约定冻结最小修复：同步脚本仅同步安装器自己的目录元数据，不再新增/替换用户 config.toml 中的 model，不根据历史 snapshot 宣称能力已验证。
保留既有调用参数；返回 status 的 ready 仅表示元数据同步结束以兼容安装器调用方，preferredModel 返回 null，增加 modelSelectionStatus=deferred-until-verified 明示没有默认选择。
已有用户配置（含历史托管块）按原始字节保留；不借迁移删除用户设置。正式默认激活仍只能由真实 ready 路径负责。
唯一产品写入者仍为主控；新增 tools/test-model-catalog-sync.ps1 合成临时目录测试源码，不执行。
同步更新 installer-pack/pack.json、file-manifest.json、checksums.sha256 的源码哈希元数据；只执行已审阅的纯元数据生成入口，不执行安装、测试、打包或发布。

### DQ-LOCAL-005 本地实现契约补充（代码阶段）
只读 Sol 设计恢复会话已正常完成（NATIVE_RECOVERY_EXIT=0，native-design-recovery-result.md），确认可用专属 app-server 连接和命令/线程/输出/最终文本联合证据，不使用手工 thread/shellCommand 代替原生工具证明。
主控修正其两处不适用于跨进程项目的建议：远端 adapterVersion 不能用本地 Cargo 版本冒充；Manager 不能直接调用另一二进制中的 ProviderService。
冻结实现：新增明确的原生验证同意参数，仅在用户确认 T0 已完成并提供部署修订标识后执行原生探针；本轮仍只编写源码、不实际运行。部署修订标识标注为用户确认元数据，不作为 ready 证明。
原生生产器只使用已安装且属于当前授权 USB 的官方 codex.exe；临时 CODEX_HOME/workspace、收紧的只读权限、精简 shell 环境、禁用代理跳转/无关工具、精确命令审批、线程/事件身份验证、有界 stdout/进程树清理。运行时不支持所需约束或目录字段则明确返回 unsupported，不填写虚构容量或推理等级。
只有当前进程观察到固定只读命令、隐藏随机输出和原生续接，且当前网关检查完整，才构造不可反序列化的 NativeRuntimeEvidence。模型名称、前端布尔值、磁盘 sidecar 不可提供证据。
默认模型应用复用 CCSwitch 的现有 ProviderService 写入路径：新增小型本地 headless 配置选择入口，接收目标模型/目录路径/预期配置修订，不接收 ready/proof 或完整配置；设备凭据仅沿既有环境变量传递。入口本身只是明确配置操作，不宣称兼容；自动调用方必须是 Manager 的当前内存 ready 选择路径。
入口在 GUI 单实例插件前处理以免旧实例吞掉配置交接；同一时刻仅一个写入者，配置/端点/凭据变化则拒绝；不会默默覆盖不同的用户模型选择或不归本工具所有的目录。
当前 API 的普通自动发现仍保守运行；没有原生验证同意时不会发起真实原生模型调用。新增阶段报告明确区分原生验证与默认模型应用成功。

### 原生实现与配置应用桥的唯一写入分工
Native writer（CLI Luna/max、workspace-write，终端 PID 70856）：仅新建 src-tauri/src/app/model_native_probe.rs（含自身单测源码）；接口及限制在 native-writer-brief.txt。主控不并发编辑此文件。
CCSwitch bridge writer（CLI Luna/max、workspace-write，终端 PID 55792）：usb_model_selection.rs、main.rs/lib.rs 最小入口及 provider service/DAO 的协作锁；具体边界在 bridge-writer-brief.txt，移交前副本在 bridge-before/。
主控暂停对上述 CCSwitch 文件写入；T3-PRESERVE 旧复审仍只对应其固定版本，桥接新增差异必须重新审计。
桥接采用有界 headless stdin/stdout 配置操作，不把 ready/proof/凭据塞进命令参数，不启动额外常驻服务；同步复用现有 provider writer，并用 Windows 可重入命名互斥量协调 GUI 与 helper 的 Codex 配置写入。其他应用不改变。
主控负责 Manager 端同意参数、调用/取消/提交接线、UI 和资源一致性。两执行者均禁止执行测试/编译/原生模型请求、修改其他任务文件、提交或推送。

### Completion integration handoff（2026-09-24，继续同一 code-only 任务）
本次增量基线：closure-20260924T102606Z/completion-20260924T131821Z/baseline.json；当前 1781 个源码/文档文件已重新取样，旧 v3 不作为最新工作区。
主控沿用已存在的作者 Python 会话 44412；其无活动子进程。原生 writer 70856 仍有当前 file_change 活动，主控不编辑 model_native_probe.rs；其他项目终端不动。
Bridge writer 55792 的 turn.completed 和 WRITER_STATUS=STOPPED 已确认，结果文件及最终哈希存在；现将其冻结版本移交主控做接口集成和已发现问题修正。
独立 T5 存储测试作者已完成；T3-PRESERVE 的独立静态复审有效，但不扩用到新增 bridge/native 集成。
DQ-LOCAL-005 具体化：同一 refresh 命令接受可选 nativeVerification={confirmedT0Complete,deploymentRevision}；默认 null。用户显式同意前不运行原生验证，也不自动激活默认模型。
部署修订是用户确认的上下文标签，不是伪造的上游证据。ready 仍要求本次进程观察到的 Gateway+CodexRuntime 完整证据。
主控负责前后端类型、版本/授权目标解析、真实证据接入、同版本目录生成、跨进程选择结果处理和最终快照；所有新执行分支仅交付源码，本轮不运行。
静态发现：bridge 暂未核对 CODEX_USB_EXPECTED_CODEX_HOME，catalog key 仅检查类型而不检查所有权；需要在激活前补齐，不能吞掉用户目录引用。
当前 WRITER_STATUS=ACTIVE_CODE_ONLY；TEST_STATUS=NOT_RUN_BY_USER_REQUEST；原生写入者完成前不接管该文件。

### 本轮原生 writer 收束与独立审核
原生 writer 70856 正常退出，NATIVE_WRITER_EXIT=0；result WRITER_STATUS=STOPPED，最终 model_native_probe.rs SHA256=0cc9ea25147743b479e9d53712a5d22e54848344a936ace9f8e57b87dfadf8b6。该退出码不是运行测试结果。
完成后主控接收唯一写入权并冻结 native-review-v1；独立只读审计 61384 正在审查；集成快照独立审计 16500 正在审查。
独立 T5 集成测试源码 writer 76688：仅允许新建 model_catalog_activation_tests.rs；主控不写该文件，结束后仅做模块注册。
原生入口静态核对发现两个阻断：hash_executable 对摘要再次做 SHA256，和主控单次哈希不一致；context.codex_version 是桌面版本，runtime --version 是内部 CLI 版本，不能强制两者字符串相等。
主控修正口径：桌面版本由受管安装检测和原始签名包/签名文件身份链核对；CLI 版本由同一原生文件实际 --version 获取，随文件哈希记录；二者分开验证，不降低版本和文件身份门禁。
原生 writer 的修正差异须再交独立审核，不沿用修正前快照结论。测试/编译仍未运行。

### 跨进程写入边界补齐（沿用 DQ-LOCAL-005）
静态调用图显示 Codex GUI 启动同步与代理恢复也会直达 codex_config::write_codex_live_atomic，不一定经过新增 ProviderService 上层锁。
将同一个 Windows 可重入互斥量补到该既有原子写函数入口，覆盖 auth/config 两文件的快照和替换；不改变其他应用，不改数据库 schema。
该文件纳入本轮唯一主控写入边界并保存原始字节。命名互斥量出现 abandoned 状态时拒绝本次自动操作，避免把异常退出遗留状态当作一致状态。
新桥接的无 provider 初始配置可合并既有普通偏好；冲突字段拒绝而不是覆盖。应用前检查 common-config 合并后的实际 endpoint/protocol/auth 路由。
这些是本地配置保存与一致性修复，不修改上游协议、不引入新的外部服务。

### DQ-LOCAL-006：原生临时文件与真正的只读边界
发现 Windows ReadOnly 文件属性会使临时目录清理失败，而该属性本身不是代码执行权限边界。尝试增加递归属性清理的请求未执行，已停止该方案。
采用更小的源码修正：探针仅 create_new 独占创建普通临时文件，不设置/清除任何 Windows 文件属性，不新增 chmod/ACL/递归权限修改逻辑。
模型命令的只读和网络限制由已确认的原生 sandbox 承担；临时文件的属性从来不作为 Shell/Skills 验证证据。目录删除仍限定在本探针新建的唯一临时目录，不涉及用户既有目录。
该决定消除模拟权限与真实 sandbox 的混淆；需要在独立复审中核对。未运行探针、未改变本机任何用户数据文件属性。

### DQ-LOCAL-007：真实运行时版本、当前选择与目录门禁
依据独立审计：CapabilityContext 增加可空 nativeRuntimeVersion（读取旧发现记录时默认缺失），桌面版本仍单独保存；只有本次原生进程实际观测到的 CLI 版本才能填入并参与 ready 判断。
本地 sidecar 为新格式版本 2，继续读取版本 1 作为发现/诊断缓存；任何磁盘记录均不能恢复 ready。当前未发布这些新 sidecar，因此不更改官方 Codex schema。
真实二进制通过当前签名 MSIX 的相同文件字节和对应桌面版本一起核对，不能仅凭“同属 OpenAI 签名”把旧桌面版本套到新运行时。
LiveCatalog 记录精确 selectedModel/ownedBy；同 raw model ID 多 owner 的歧义模型不得自动选用。发布前对完整最终 JSON 重新执行同一原生 parser 门禁。
TurnStartResponse 的保留本机 schema 只有 turn，没有 threadId 或有效 sandbox 回显。使用已确认策略的独占 thread，turn/start 不再覆盖策略；JSON-RPC response ID 和随后 threadId/turnId 事件共同建立关联，不发明 schema 字段。
安全许可回调仅接受已知字段、明确 kind=command 和明确 accept 选项。所有进程挂起创建、先绑定 Job Object 再恢复，避免先运行后附加隔离的竞态。
运行命令和模型验证仍交用户执行，本轮仅修改相应源码。

### DQ-LOCAL-008：配置应用与失败补偿
独立审计确认通用 ProviderService::update/switch 会产生额外 MCP 同步及分步写入，不能用于本次有限模型切换。
专用 helper 改用既有 Codex 原子文件写入器和限定 USB provider 的 SQLite 事务；只修改该 provider 的设置/当前选择，保存原内存快照用于普通错误和超时边界的补偿。
同一 apply 操作包含已验证 catalog 的路径及 SHA256，协议升至 2；helper 核对预期 home、文件所有权、hash 和模型成员后，在同一 config 写入中安装目录引用。helper 不接收任意 ready/capability 证书。
未确认应用时主控按字节 CAS 恢复自己写入的候选目录；任何恢复冲突必须报告，不能覆盖用户新增修改或宣称未发生变更。
该补偿不是对断电/强杀/文件系统故障的跨数据库 ACID 承诺；异常退出后的锁遗留、版本/文件哈希冲突继续禁止自动放行并交本地恢复验证。
追加的独立 T5 writer 请求未启动；主控仅作为接口集成者补齐两个独立测试夹具的新 context 字段，原有测试断言不改。所有测试仍未执行，最终静态审计必须包含这些集成差异。

## 17. T1–T6 sole-writer continuation（2026-09-25）
用户明确：其他会话仅推进 T0，现已结束本地仓库写入；Sub2API 的 VPS 部署不属于本会话范围。沿用 code-only-r1，不运行编译、测试、安装、打包或真实模型请求。
已读取并核对现有 multi-agent-dev（SKILL/参考文件哈希未变）；COMPLEXITY_GATE=ESCALATE_REQUIRED；CONTEXT_MODE=ARMED。
旧源码编辑终端 73812 只有交互 Python、无执行子任务；已调用停止并由 list_sessions 确认消失。75452 属于另一项目，未触碰。
本轮唯一产品写入者：当前会话；WRITER_STATUS=ACTIVE_CODE_ONLY。已完成的独立 T5 作者记录保留；新增差异须重新独立审计。
新基线：.codex-audit/local-t1-t6-20260925/closure-20260925T020144Z/baseline.json；主仓库 889、CCSwitch 896 个文件。
基线 SHA256：1d272c13f50adc2bee57e0043cb3e95de0fc6f9b0bc570a3e3417f0a7052d3db。
主仓库 HEAD=e9514d08c343f594b0e452617bdf4a45bc0216b0；CCSwitch HEAD=1d44b1ba41987cddeb8096fadba8190fab465cbb。
每次写入前比较本轮原始字节；保留所有既有改动。T0 部署/用户口头确认不冒充真实模型验证或运行证据。
SOURCE-AUTH-01：应用已读回的认证文件内存指纹补丁；只编辑源码，不读取/修改本机 auth.json，不保存或打印任何凭据。
继续逐项核对当前原生协议、配置补偿、状态展示和测试源码；独立 CLI 审计在取得真实结果后计入，不伪造原生 spawn receipt、不声称已切换当前 ChatGPT 模型。
原 v3 状态不能代表当前已接通的后续代码，所有完成/未完成结论以本轮最终快照为准。

### DESIGN_QUESTION DQ-LOCAL-009：本地收尾边界核对
触发：USB 专用 DAO 的入口仍接受任意 provider ID/初始 endpoint；配置 helper 的部分失败出口未显式确认进程清理；原生验证与默认应用需要分别展示。
依据：已有 T3 配置保护和 DQ-LOCAL-004/008、T2 状态分离、T6 本地集成要求。主控仅收紧这些既有语义，不新增 provider/model 白名单、公共协议或外部副作用。
最小决定：DAO 只接受其既定 USB provider 身份和 canonical endpoint，未知输入在数据库写入前拒绝；保留现有记录的配置/元数据不变。相应合成测试使用同一已存在 provider 常量，不改变断言目的。
helper 仍是已有有限子进程；完善错误清理与确认，不增加后台服务。验证已通过但应用失败要保留两个不同的结果，不以应用成功替代能力证据。
所有改动先固定版本后独立审计；测试/编译/真实请求继续由用户执行，NOT_RUN_BY_USER_REQUEST。

### 本轮独立职责与固定对象
只读审计线程由本地 CLI 启动，进程 70612，目标为本轮 audit-v1 的固定快照（495 个源文件/规则）；已观察到实际只读命令事件，最终结论待返回。模型/强度仅记录请求 Luna/max，不伪称平台实际路由凭证。
T5-RPC-TESTS 分配给独立 CLI 写入者，仅允许新建 model_selection_client_tests.rs；派发时文件不存在。父模块 model_selection_client.rs 已固定 SHA256=06f0edd47874af9387d41d8efae1f631e54f24c71e611b7194d97a7dec88f1ae，主控在交接前不修改该父模块或其新测试文件。
该测试子任务为单模块、纯解码器、无公共语义变更，COMPLEXITY_GATE=SIMPLE_PROVEN；请求 Luna/high。只编写源码，不运行任何测试或编译；如发现产品缺陷，只返回给主控。
源码补充：默认选择统一函数现已拒绝重复 raw ID（含不同 owner）与重复原生证据，正常唯一模型的路径保留；未增加固定模型名单。

T5-RPC-TESTS 启动请求被工具安全检查拦截，未执行、无进程/线程 ID，WRITER_STATUS=NOT_STARTED_TOOL_BLOCKED。未换工具重发、未代写同一被阻止的新测试文件。此前的独立 T5 存储/集成测试源码及其作者记录保留。
父模块没有被移交给任何新写入者，后续产品修正仍由主控负责。已在运行的只读审计 70612 与该被拒绝调用无关，继续核对其真实结果。

### 配套源码遗漏：旧默认模型迁移断言（沿用 DQ-LOCAL-006）
发现 installer-pack/test-adapter-contracts.py 仍要求同步时把隐藏/不支持模型迁移到固定 GPT，README 与 audit-codex-compatibility.py 的成功标签也仍声称默认模型迁移。当前产品脚本已不再执行该行为。
修复仅对齐已批准的“保留用户模型配置、无验证不选择默认值”契约：更新原测试为精确配置保留/缺失配置不创建，并保留严格字段验证与幂等反例；不运行测试、不弱化安全断言。
同步更新 README 和审计标签，补齐元数据同步的静态约束；使用已完整阅读的 installer-pack/scripts/build-pack.py 的 write_generated_metadata 函数更新三个源码哈希清单，不调用其 main/verify，不构建或运行任何发行包。
本变化为原有 T3/T5/T6 配套收口，与未启动的 T5-RPC-TESTS 新文件无关。

### DESIGN_QUESTION DQ-LOCAL-010：原生命令回调按已保留协议判断
现有原生审批匹配强制要求 availableDecisions，但 .analysis/schema-out/CommandExecutionRequestApprovalParams.json 不定义该字段；kind 在该 schema 中明确默认 command，startedAtMs 才是必需整数。对应 Response 定义正常一次性 decision=accept。
原实现会拒绝满足已知 schema 的合法固定只读命令回调。主控冻结最小兼容修正：缺失 kind 只按该 schema 的 command 默认值解释；显式 null/其他 kind 拒绝；availableDecisions 缺失时采用 Response 声明的一次性 accept，显式给出时仍必须包含 accept；补齐 startedAtMs 整数校验。
原有精确 command/cwd/thread/turn/item 身份、未知字段拒绝、附加权限/网络/策略修改拒绝全部保留；永不返回 acceptForSession 或任何持久策略授权。不修改全局审批设置，不运行命令，不改变上游请求协议。
相关 schema 文本和哈希绑定于本轮审计快照；作者测试改为已知 schema 正例与错误类型、stdin、受限决定、额外权限反例。必须随最终快照交独立复审。

### 本轮独立审计 v1 已完成并进入返工
审计进程 70612 正常退出 0，audit-result-v1.md 结论 STATIC_VERDICT=nonconforming；495 个快照文件哈希全部匹配。退出码仅表示审计任务完成，不是产品测试结果。
指出：原生协议/目录依据未完整纳入快照、Codex 第三方 presets 仍默认模型、配置已应用后缺少 outcome_unconfirmed 状态、installer catalog 输入未入快照、直接 helper 的绑定边界需要复核。
只读审计没有修改源码。主控按范围内修正和补证推进，最终版本必须复审；不把当前拒收结论写成通过。

### DESIGN_QUESTION DQ-LOCAL-011：剩余 presets 与应用未确认结果
T3：codexProviderPresets.ts 属于同一个 Codex 产品可达入口。删除第三方 preset 中静态模型/推理/容量默认值，保留供应商连接配置；既有生成函数第三个参数变为显式可选选择，未传值时不写 model，不把显式选择当能力证据。官方 Codex 程序不变，其他应用 presets 不变。
T2/T6：新增 activationStatus=outcome_unconfirmed，表示 helper 请求可能或已经应用，但后置核对/响应尚未确认；不能显示为“未发生修改”。只有确认回滚后才返回 incomplete；确定应用且仅重启标记失败时仍单独显示 applied_restart_pending。
这些语义落实既有配置保护和安装/验证/应用分离要求。新增状态同步 Rust 报告、TypeScript 类型、文案及配套测试源码，交同一独立审计复核。
关于 helper 审计意见：Manager 是唯一原生能力验证权威；helper 是本地配置操作而非认证服务，其响应不产生 ready。不得把普通随机 nonce 冒充授权证明，不新增远程或本地常驻认证服务；仍需复核完整上下文一致性与直接调用限制。

### 原生协议补证与目录检查收紧（审计 v1-1）
将已有 .analysis/schema-out 中的 JSON-RPC ClientRequest/ServerRequest/ServerNotification、初始化、模型/Skills、命令审批及事件定义一并纳入最终只读快照，记录原始路径与 SHA256。它们是此前本机 runtime 导出的接口依据，不冒称已经在本轮验证当前发行二进制。
目录候选字段不是发现证据：只在隔离原生探针中使用，且必须由当前签名 runtime 的真实 parser 读回后与完整候选结构严格一致，才可能提升 native_catalog。收紧原有子集比较：根 schema/identity/client_version/fetched_at 和全部模型字段必须保持，额外/缺失/替换字段均按 unsupported；最终待发布 JSON 再走同一门禁。
该动态 parser 门禁不声称存在未取得的官方静态 catalog schema；运行时无法给出可核对结构时保持未知，不写入或激活未经确认的目录。没有运行任何 parser/probe 产品命令；这里只修改源码并保存既有 schema 依据。

### DESIGN_QUESTION DQ-LOCAL-012：配置 helper 的完整绑定上下文一致性
针对审计 v1-5，新增的不是认证服务或 ready 证书：Manager 仍通过既有签名 USB 授权和真实原生检查决定资格；helper 本身仅执行本地显式配置操作。
最小加强：有限 stdin 协议升至 3，Manager 在请求中附加 provider/accountRef/credentialId/groupId/sub2apiUserId 的非秘密指纹；沿既有子进程环境注入补齐 credentialId/userId。helper 在任何数据库初始化/写入之前校验所有身份字段、正整数 ID 与指纹一致，snapshot 修订也绑定该上下文；endpoint 和 key 继续独立核对。
指纹不包含 API Key、密码或请求正文，也不声称可认证拥有同等本机权限的恶意进程；随机 requestId 只用于关联，不冒充安全证明。缺字段/旧协议/不同上下文一律拒绝并返回稳定错误，不读取或修改其他服务。
新协议尚未发布，两端源码/测试同步升级；旧 helper 在版本不匹配时安全失败，不通过 fallback 静默应用。测试和实际 helper 均不运行。

### 独立复审对象冻结（本轮 audit-v3）
本轮产品写入已暂停，WRITER_STATUS=INACTIVE_VERSION_FROZEN_CODE_ONLY；仅继续只读检查和证据记录。
固定快照：.codex-audit/local-t1-t6-20260925/closure-20260925T020144Z/audit-v3，506 个源码/规则文件及 301 个本机既有协议 schema；MANIFEST SHA256=28c25efdcba2dc717a28dc38aa3865a7a34a21fb1c40b674a108dbce88d5e8cb。
独立审计复用同一非作者线程 01a0d64e-9bfd-7992-9fd0-80668e434140，先前回合已 turn.completed/进程退出 0；复审终端 52916，read-only。审查只对应新快照，不沿用先前 nonconforming 版本放行。
五项反馈已分别形成源码修正或协议/资源补证，具体差异在 audit-v3/DELTA-MANIFEST.json 和 review-diffs/；旧审计原文在 PREVIOUS-AUDIT.md。等待实际复审结论后更新最终任务状态。

### T3 补充遗漏：Universal Provider 的 Codex 分支
在复审固定快照期间继续只读定位，发现统一供应商入口仍在前端默认 gpt-5.4/high、Rust to_codex_provider 默认 gpt-4o/high。该路径可同步到 Codex，属于同一未验证默认选择问题，不涉及 Claude/Gemini 模型逻辑。
主控恢复仅这些文件的唯一写入权，v3 审计快照不变；新增差异将另行固定交复审。
沿用 DQ-LOCAL-011：统一预设不默认启用 Codex，也不预填 Codex 模型/推理；用户显式启用 Codex 时必须填写模型。Rust 缺少选择时不生成 Codex provider；同步函数在任何应用写入前拒绝该缺失状态，不能把缺模型误作禁用而删除原有 Codex 配置。
保持 Claude/Gemini 默认和生成行为不变；更新已有地址生成测试的显式模型输入并增加无默认值反例源码，不运行测试。该修正不是扩大到其他应用的协议/模型整改。

### 最终补充快照与审计分工（audit-v4）
本轮新增 Universal Codex 分支已完成源文本修正并停止产品写入。audit-v4 包含 507 个源码/规则文件和原有协议依据，MANIFEST SHA256=7684273aacfdec8324b8e3a4320495780eab1ede41dec99755c900c7dc282e4f。
相对 audit-v3 仅五个文件有产品差异：provider.rs、services/provider/mod.rs、UniversalProviderFormModal.tsx、universalProviderPresets.ts、codexTemplates.compatibility.test.ts；完整 diff/前后 SHA256 在该快照内。
原独立审计继续审查不变的原生/集成 v3；另派独立只读审计仅核对上述五文件及必要调用方，不重复全仓扫描。二者无写入权限、无测试执行授权。最终结论必须同时满足 v3 集成与 v4 增量审计，且按最终文件哈希核对未审差异。
WRITER_STATUS=INACTIVE_VERSION_FROZEN_CODE_ONLY；COMPLEXITY_GATE=ESCALATE_REQUIRED（跨前后端模型选择契约）；不把审计任务启动或源码解析通过当成最终审计通过。

### 应用未确认的认证文件补偿边界（沿用 DQ-LOCAL-011/008）
最终只读核对发现：helper 失败后 Manager 只比较 config.toml 是否恢复，若进程在 auth 写入后失败但 config 未变，可能误把结果视为普通 incomplete。
最小修正限定 model_catalog_activation.rs：派发前同时记录 config/auth 原始字节；失败确认时两者都必须未变，才恢复候选目录并标记 incomplete；任一变化/不可读都保持 outcome_unconfirmed，不覆盖认证数据、不宣称回滚成功。
只保存本次内存快照，不新增磁盘凭据日志/副本。该一文件增量将在审计最终阶段额外复核，正在审查的 v3/v4 快照不变。

## 18. 原生协议四项返工与最终交付收尾（2026-09-25）
用户继续任务；复用同一作者会话 74860，23 个已改文件均与最新内存基线一致，无漂移。增量检查点 resume-20260925T033101Z/baseline.json。
两独立回合已确认 turn.completed：集成 01a0d64e-9bfd-7992-9fd0-80668e434140 的 v3 结论 nonconforming；Universal 五文件 01a0d681-377f-71d2-a642-ef4c3de82d38 的 v4 结论 conforming。后者不扩大为整体通过。
WRITER_STATUS=ACTIVE_CODE_ONLY；唯一产品作者仍为当前会话；不运行测试、编译、真实请求，不修改 T0/Sub2API/VPS。

### DESIGN_QUESTION DQ-LOCAL-013：按冻结 schema 消除四项原生返工
依据 audit-result-v3.md：只改 model_native_probe.rs 内部协议校验与反例源码，不新增公共 API/状态或上游协议。
1. 移除已知回调 schema 不定义的 availableDecisions/additionalPermissions 可接受字段；任何出现均拒绝，正常一次性 accept 仍按已知响应 schema。
2. legacy applyPatchApproval 拒绝值用 abort；未知方法使用顶层 JSON-RPC error 而非 result.error；其他已知方法保持各自合法拒绝结构。
3. thread/start 的 approvalsReviewer 必须显式等于请求的 user，缺失/null/auto_review 均不产生证据。
4. 使用本模块私有 serde 类型校验 ModelListResponse/SkillsListResponse 的完整必需和已知可选结构、枚举、错误类型；通过结构校验后才使用原有身份/path/nonce/权限检查。不引入通用 schema 引擎或新依赖。
冻结 schema 来自 audit-v3/protocol-schema，其 ModelList/SkillsList/Approval 三个文件与当前保留导出字节相同。严格校验仅允许支持的接口，不把新字段/未验证运行行为当通过；新增反例代码不执行。

### v6 原生修正落盘及独立复审阻断
四项原生问题已修正于 model_native_probe.rs，SHA256=9c1bbc4c1713610db4c749e3cedf9d9a78e1f6b2f2a8fd635d943f0d0e7a6abc；配套新增/修正反例源码，Rust 语法解析退出码 0，测试/编译未运行。
最后的 auth/config 双文件补偿检查已落盘于 model_catalog_activation.rs，SHA256=ff5c5d4038e04a5a6b64005eb8609eec8fa34bd39a48b31d208954ac1681cfb7。
固定 v6 快照 507 个源码/规则文件及 301 个保留协议 schema，MANIFEST SHA256=2ab77bff8439cbca4e89ff7202eb9f9e5a5ec04504f3fda944707e3da22fbb53；Universal 五文件与已通过审计的 v4 字节完全一致。
独立复审启动请求被工具安全检查拦截：未执行、无新进程或回合结果。未改换工具/命令重试该被拦截操作。audit-followup-v6.txt 仅是准备好的交接材料，不是审计结果。
AUDIT_STATUS=LATEST_TWO_FILES_AWAIT_INDEPENDENT_REVIEW；WRITER_STATUS=INACTIVE_SOURCE_FROZEN_CODE_ONLY。已知四项源码问题的作者修正不代表独立复审已经通过，整体不标记 ACCEPTED。
后续仅进行授权内源码解析、哈希/差异核对和交接记录，不运行产品、测试、部署或替换发行包。

### v6 源码交接记录
已生成 `docs/reports/CODEX-3P-LOCAL-T1-T6-CODE-HANDOFF-20260925.md`，包含逐任务状态、修改文件范围、起始/最终基线、DESIGN_QUESTION、待执行命令、退出码及未验证项。
任务累计 52 个源码/资源/测试差异（17 新增、35 修改）；本轮继续只修改原生协议文件，其他任务源码保持此前冻结结果。最终 52 文件与 audit-v6 逐字节一致。
最终检查：23 Rust 源码语法解析退出码均 0；14 TS/TSX 解析退出码 0、无诊断；2 Python AST/3 JSON 解析正常；两个仓库已跟踪任务路径 git diff --check 均退出码 0。均不是产品编译或测试。
本轮基线检查：不含任务追加记录的 1,784 个既有文件中，23 个范围内变化，其他 1,761 个哈希未变，范围外变化为 0。
最终源码清单 source-manifest-final.json SHA256=b632f462876f7c6d16057deb35b5024476c7c39f7b79d5304e36ece5b4b4afe1。
当前状态 SOURCE_UPDATED_PENDING_LAST_INDEPENDENT_REVIEW；最新两个文件仍待独立复审，不标记整体 ACCEPTED。编译/类型检查/测试/打包/安装/真实模型验证均 NOT_RUN_BY_USER_REQUEST，退出码 N/A。
源代码写入已停止。没有提交、推送、部署、替换现有发行包或操作 T0/VPS；外部通知 NOT_SENT_SCOPE_BOUNDARY。

## 19. 交付前代码收尾（2026-09-25）
用户要求：先完成剩余全部开发，再交测试同事。沿用 code-only-r1：不运行编译、测试、安装、打包、真实模型请求，不修改 T0/Sub2API/VPS/官方 Codex。
已重新读取 multi-agent-dev 全文及执行参考、核心契约、最新交接和实际源码；52 个任务文件均与上一版 source-manifest-final.json 一致。
本轮基线：.codex-audit/local-t1-t6-20260925/closure-20260925T020144Z/completion-20260925T040053Z/baseline.json（主仓库 890、CCSwitch 896 文件）；已有工作区修改全部保留。
唯一产品写入者=当前会话；WRITER_STATUS=ACTIVE_CODE_ONLY；COMPLEXITY_GATE=ESCALATE_REQUIRED；CONTEXT_MODE=ARMED。此前作者已退出，其他终端不接管。
独立审计仍必须由不同于作者的真实审查者对固定版本完成；不伪造 MAD_ROUTE_RECEIPT，不宣称已切换主会话模型。缺少原生 spawn 时只记录实际可观察的独立 CLI 审查。

### DESIGN_QUESTION DQ-LOCAL-014：有限 RPC 成功响应一致性
发现 model_selection_client::decode 仅检查响应 schema/id，尚未拒绝未知请求 operation 或 ok=true 但 code 为失败值的矛盾响应。现有正式 helper 的唯一成功码是 ok。
主控按既有 schema-v3 和 fail-closed 契约确认最小修正：请求/响应版本一致、请求操作为 snapshot/apply、成功码严格为 ok，apply 请求必须带非空模型；错误响应继续映射静态错误码，不回传任意外部内容。
补齐父模块中的纯解码测试源码，覆盖版本/操作/身份/哈希/布尔值/失败码脱敏/凭据反射；不新增测试框架、不执行任何测试、不把作者配套测试冒充独立作者成果。
该修正不新增公共 API/schema、不改变正确请求的配置行为，不修改正式运行配置。最新差异和此前两个未复审文件一起冻结送审。

### 当前独立审查已实际启动
在重新核对调用内容和固定新版本后，正常 read-only CLI 审查请求已成功启动，终端 PID=69580；无全权限/跳过信任参数，未修改任何安全设置。
审查对象为 completion-20260925T040053Z/review-final，MANIFEST SHA256=0558c6c2071c6f1ff603acda8d3a59c2c130448dda8db9297d0e010d9ddfb004；重点为最后两个原生文件及新增 RPC 解码差异，不复用旧版通过结论。
独立作者身份由本次 CLI 线程/事件结果记录；请求模型/强度为 Luna/max，不宣称原生 spawn receipt 或当前 ChatGPT 模型切换。
WRITER_STATUS=INACTIVE_SOURCE_FROZEN_CODE_ONLY。源码改动暂止，只读核查与交接资料准备可继续；发现审计问题则按明确范围返工后重审。
此前未生成的额外 RPC 测试文件不再作为悬空交付项：同一父模块已接入四组等效纯解码反例测试源码，属于主控作者配套测试，不冒称独立 T5 作者。既有独立存储/激活测试作者记录保持。

### 独立审查结果及最后夹具返工
线程 01a0d6bb-fc34-7b01-9be6-89f0e5f74789、终端 69580 已正常退出 0；实际报告 completion-20260925T040053Z/independent-review.md 为 nonconforming。
该审查确认：原生四项历史问题已解决，配置补偿/内存绑定检查源码正确；唯一遗留是 RPC 测试正例夹具不满足正式 helper 的操作参数要求（apply 缺 revision/hash 不完整，snapshot 携带仅 apply 使用的字段）。
按意见仅修正 model_selection_client.rs 内测试夹具，生产函数不改；snapshot 不携带 catalog 字段，apply 使用绝对路径和 64 位十六进制 revision/hash，旧手写正例复用同一夹具。测试仍不运行。
本返工为 SIMPLE_PROVEN：单模块纯测试输入修正，无公共 schema/API/状态/安全或持久化语义变更。唯一写入者=当前会话；完成后重新冻结，再由同一独立审查者复核此差异。
此前非符合结论保留，不修改其原文；最终审计不能仅凭进程退出码 0 判定通过。

### 最后一处测试夹具修正已固定
model_selection_client.rs 最终 SHA256=cf0697e424851806deb18ad4458e3a447581491ca3855dbbb73db66e2f9d1dc3；相对前次审查仅 34 行差异，全部位于 cfg(test) 模块，生产代码不变。
Rust 源文本解析退出码 0；没有运行测试。原手写非法正例已复用操作专用夹具，避免重复维护。
同一独立审查线程 01a0d6bb-fc34-7b01-9be6-89f0e5f74789 继续复核此小范围差异，终端 PID=46536，read-only，请求 Luna/high（SIMPLE_PROVEN）。先前审查进程已正常结束，未并行启动同一线程的审查回合。
固定对象在 completion-20260925T040053Z/review-fixture；WRITER_STATUS=INACTIVE_SOURCE_FROZEN_CODE_ONLY，最后结论等待实际返回。

## 20. 本地源码与独立静态审查闭环（2026-09-25）
最终独立复审已实际返回 STATIC_VERDICT=conforming，原文明确：“The correction closes the sole remaining source finding.”
独立审查线程：01a0d6bb-fc34-7b01-9be6-89f0e5f74789；主审终端 69580 与最后夹具复审终端 46536 均正常退出 0。该退出码为审查任务退出码，不是产品测试退出码。
主审已确认原生四项修正、认证内存绑定和 config/auth 补偿检查正确；最后 34 行 cfg(test) 夹具修正经同一独立审查者复核通过。原非符合报告保留，以最终复审关闭唯一遗留问题。
已有 Universal Codex 五文件独立审计与当前源码哈希仍一致；当前 52 个任务文件全部固定，没有未审查的源码增量或未关闭的已发现源码阻断项。
最终源码状态：LOCAL_T1_T6_SOURCE_AND_STATIC_REVIEW_COMPLETE；WRITER_STATUS=STOPPED_SOURCE_DELIVERED；T1–T6 代码阶段均已完成，可以进入用户安排的编译和本地测试。
所有编译、类型检查、运行测试、安装/启动回归、打包及真实模型验证仍为 NOT_RUN_BY_USER_REQUEST，退出码 N/A；运行验收未授予，真实第三方模型最终兼容结论未授予。
证据目录：.codex-audit/local-t1-t6-20260925/closure-20260925T020144Z/completion-20260925T040053Z/。
source-manifest.json SHA256=e596a81e9fe6f7ad05bfd9ab2661d045481c4782d46b79bac530c7b892c45cf2。
source-final/MANIFEST.json SHA256=3c8be3c81037b323710ecd4878253880aae169f30fcaa93123132e86f0fe688a；source-final 仅为 52 个任务文件快照，不是完整独立项目或安装包。
independent-fixture-review.md SHA256=3a67c3d25afc84aa8ce39674ad194b9fa49486fca12df9442757e58e84349b35。
最终交接说明已更新：docs/reports/CODEX-3P-LOCAL-T1-T6-CODE-HANDOFF-20260925.md，SHA256=a311b4ecab4f4d413d98d4d97d54d6f54a5c36c967c5b339308e8936a1550c7a。
交接必须包含 17 个新增文件和 35 个修改文件；Manager 与 CCSwitch 必须一起从新源码构建，旧发行二进制未替换。
未提交、推送、部署、操作 Sub2API/VPS、修改官方 Codex 或现有安装包。未发送外部通知、未读取通知凭据；无新的产品进程留在后台。

## 20. 用户本地验收反馈：五项定向修复（2026-09-25）
本轮原文目标：补齐 Windows 子模块 same_id 导入；将 nativeModelStatus.test.ts 的 Object.fromEntries 类型断言改成显式完整 ModelCapabilityChecks；Vitest exclude 永久加入 .codex-audit/**，不排除正式 src 测试；修复测试脚本 $home 与只读 $HOME 冲突；将 provider 资源默认路径计算从 param 表达式移至 param 后，兼容显式 SourcePath。
允许五文件：src-tauri/src/app/model_native_probe.rs；src/app/views/nativeModelStatus.test.ts；vite.config.ts；tools/test-model-catalog-sync.ps1；tools/verify-model-provider-resource.ps1。除交接/证据外不扩大范围。
用户本轮明确授权运行：cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::model_；npm run check；npx --no-install vitest run src/app/views/OfflineHome.test.tsx src/app/views/nativeModelStatus.test.ts src/services/managerApi.test.ts；两个 PowerShell 脚本。此授权替代此前本轮相关命令的 code-only 禁令。
正式 Vitest 目标为 3 套件、83/83；只以实际输出为准，不能通过删测试或修改严格配置达标。
禁止：修改 T0/Sub2API/VPS/上游协议、Codex 官方二进制、tsconfig 严格检查、真实用户配置/认证/选择；禁止真实 API Key/模型/网关请求。本轮不发送外部通知。
DQ-LOCAL-005 保持不变：安装后自动发现，原生 Shell/Skills 验证需用户手动确认。本轮无改变公共协议、状态或安全语义的授权，也没有新增此类设计决定。
T0 完成后，如 Responses 字段/事件顺序/错误语义变化，必须重新审查受影响源码与测试，旧结论不能直接沿用。
基线：.codex-audit/local-t1-t6-five-fixes-20260925T050704Z/baseline.json，主仓库 890 文件、CCSwitch 896 文件；52 个交付文件均与上一清单一致。已有用户改动全部保留。
旧清单保持不动：.codex-audit/local-t1-t6-20260925/closure-20260925T020144Z/completion-20260925T040053Z/source-manifest.json。新清单保留 52 条并记录本轮额外 vite.config.ts。
CONTRACT_REV=v1.3.1-route-guard + LOCAL-FIVE-FIXES-v1；COMPLEXITY_GATE=ESCALATE_REQUIRED（多语言验收及清单集成），需求明确无需另设思考者；CONTEXT_MODE=ARMED。
唯一写入者：当前会话。独立只读审查由非作者本机 CLI 会话对固定差异进行；不伪造 spawn/路由 receipt，不声称切换当前 ChatGPT 模型。审计者不能改文件或执行真实产品请求。
WRITER_STATUS=ACTIVE；先记录同一工作区 Rust/TS 修复前错误及 warnings，再做最小修正和用户指定测试。

### 本轮实测与必要夹具修正 R6
修复前 npm run check 退出 2，复现 TS2352；Rust 退出 101，复现 same_id E0425。修复后 npm 退出 0；两个 PowerShell 退出 0；显式 SourcePath/ArtifactPath 也退出 0。Vitest 正式 3 套件（45+6+32）、83/83，退出 0，没有历史快照套件。
Rust 导入修复后成功编译并执行 63 项测试，但 malformed_call_and_stream_are_isolated_per_model 失败：测试监听器设 nonblocking=true 后，accept 得到的 TcpStream 在第一次 read 返回 Windows 10035/WouldBlock，后台夹具线程 panic（model_probe_fixture.rs:100），导致后续期望 Verified 得到 Unknown；62 通过、1 失败，退出 101。
为满足用户指定 Rust 验收，最小补充只改 test-only model_probe_fixture.rs：accepted stream 在读取前明确 set_nonblocking(false)，继续保留原 5 秒读写超时、监听器非阻塞停止循环、所有模型断言与生产探测代码。本地 loopback，未使用真实凭据或真实网关。
这是现有测试 IO 设置修正，不改变公共 API/schema/状态/协议/验收口径，不新增 DESIGN_QUESTION。必要范围从五文件增加这一已在旧 52 清单中的夹具文件；禁止用重试偶然成功、删断言或单线程测试参数代替修复。
原五文件审查快照保持不变；新增夹具差异和实际运行记录须追加独立只读复审，不能凭旧快照放行。

### 六文件冻结与验收结果
最终六文件均已保存，WRITER_STATUS=INACTIVE_SOURCE_FROZEN；修复补丁 88 行，SHA256=3fedffce2c438654a3695f392cea04ffaea30261b7605f41c15cdbb7985e1684。
Rust fixture 修正后原始 cargo 验收命令连续两次 63/63、0 ignored、451 filtered out，退出码均 0；没有追加单线程/缩小测试过滤参数。
Vitest 原始命令只收集 src/app/views/OfflineHome.test.tsx（45）、nativeModelStatus.test.ts（6）、src/services/managerApi.test.ts（32），合计 3 套件83/83，退出0。npm check及两个PS脚本退出0；显式资源输入附加验证退出0。
修复前后 Rust 均只有 model_catalog_activation.rs:238 的 unused variable run_id，新增 warning=0；未作无关消警修改。
1,784 个既有文件（排除两份允许更新的报告）比对未发现范围外修改。旧52文件清单SHA256仍为e596a81e9fe6f7ad05bfd9ab2661d045481c4782d46b79bac530c7b892c45cf2，保持原文不变；新关联清单保留52条，额外登记vite.config.ts，共覆盖53个文件。
原五项独立只读审计已conforming，线程01a0d6f8-ad2a-7aa3-865b-c1700d19edce、终端57864退出0；其审查时尚未读取验收日志，不把该报告当运行通过证据。
同一独立审查者继续核对新增加的test-only socket两行修正及全部本地验收日志，终端39392；本增量为单模块、无公共语义变化，COMPLEXITY_GATE=SIMPLE_PROVEN，显式请求Luna/high，仍read-only。
独立审查不改变被审代码，不运行真实模型；最终交付等待此增量真实结论。所有作者测试进程均已退出。

### 本轮最终交付：LOCAL_REPAIRS_VALIDATED_WAITING_T0
同一独立非作者会话完成两轮只读复审，原五项及新增test-only socket修正均conforming；补充结论直接核对了六个最终SHA256和全部验收日志，确认本轮边界内证据充分，无具体剩余缺陷。两个审查进程均退出0。
指定本地验收最终均退出0：npm check；cargo app::model_ 63/63（连续两次）；Vitest正式3套件83/83；两个PS脚本；显式SourcePath/ArtifactPath兼容检查。修复前的错误日志保留，不充当最终结果。
最后六文件SHA256见 .codex-audit/local-t1-t6-five-fixes-20260925T050704Z/repair-files.sha256；补丁SHA256=3fedffce2c438654a3695f392cea04ffaea30261b7605f41c15cdbb7985e1684。
新关联 source-manifest.json SHA256=6b6751edfc3cbf6d3efdbdeb39c0769ef1933262e2a8c4716cbf063b3d42fa29。原52条全部保留，新增覆盖项vite.config.ts单列；旧清单未改。
详细修复报告 repair-report.md SHA256=651e71be9c6fe61d9a2f9aee16ae99c4e60378e1839541d87d1f8c244fa0b5f5；现有CODE-HANDOFF报告已追加最新验收段落。
新增warning=0，仍保留已有model_catalog_activation.rs:238 unused run_id。没有顺带更改生产协议、认证/用户配置、官方二进制、tsconfig严格性、DQ-LOCAL-005、T0/Sub2API/VPS，也未发布新安装包。
WRITER_STATUS=STOPPED_SOURCE_DELIVERED。后续等待T0；若Responses字段/事件顺序/错误语义有变化，重新审查并回归影响范围，当前局部复审结论不能直接作为新上游契约的验收。
