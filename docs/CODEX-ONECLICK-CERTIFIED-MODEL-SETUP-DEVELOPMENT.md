# U 盘版 Codex 与 404token：恢复原流程及高级探测规划

> 任务：USB-CODEX-AUTO-MODEL-SETUP-20260926
> 规划修订：RESTORE-WORKING-FLOW-OPTIONAL-PROBE-V1
> 日期：2026-09-27
> 状态：**本地实现、模拟回归、404token 线上复测和正式内部签名包静态核验已完成；执行者复核完成，不代表独立审计或物理 U 盘验收通过。**
> 本修订替代本文档此前的“服务端强制认证目录 + 安装时自动验证 + 验证后才应用”方案。

## 1. 用户目标与决策

目标是排除本任务引入的阻塞，恢复 U 盘版 Codex 原先可用的安装、配置和 CCSwitch 使用流程。**不改变正常用户流程，不要求用户先运行模型探测，也不因探测结果阻止 CCSwitch 配置、Provider 切换或 Codex 启动。**

上游可用性探测仍有价值，但作为高级功能中的可选工具，由用户自行打开和运行。探测失败只显示诊断结果，不改变配置，不影响安装和启动。

用户明确指出：

> 我要的是你把 BUG 排掉，不是把流程改了。探测上游模型可用性有意义，可以收进高级功能，让高级用户可选使用。

## 2. 当前确认的回归来源

对照 U 盘仓库 HEAD=e9514d08c343f594b0e452617bdf4a45bc0216b0 与当前未提交工作区，本任务引入的模型自动配置门禁包括：

1. OfflineHome.performInstall 要求勾选模型调用费用确认；安装程序和能力包处理之后，先运行 runAutomaticModelSetup(true)，成功后才启动 CCSwitch。模型探测失败会跳过 CCSwitch 启动并显示 recoverable_incomplete。
2. 页面恢复既有安装状态时调用 nativeModelSetupReady()；没有当前进程模型验证证明会被显示成安装配置未完成。
3. win_launch_codex 调用 model_catalog::prepare_launch()；无当前验证证明时会停用受管理模型目录并返回错误，阻止 Codex 启动。
4. 自动配置流程强依赖新增的 GET /v1/codex/compatibility，并包含服务端 revision/优先级证明、逐模型网关矩阵、Codex 原生会话以及 apply/readback 完成证明。

这几项不应是安装、CCSwitch 或 Codex 启动的前置条件。旧方案要求的新接口并非连接模型上游所必需；之前一次不带凭证的线上 GET 返回 HTTP 404，当前也没有部署该接口的证据。此结论是执行者对当前源码和工作区的审计结论，不代表独立审计者签收。

## 3. 目标默认流程

默认安装和使用流程恢复为：

1. 校验现有 USB license、设备绑定、安装包签名与哈希，并按既有事务流程把 Codex 安装到当前 U 盘。
2. 安装本地能力包，遵循此前已有的成功/部分成功提示行为。
3. 按原流程尝试安装并启动随 U 盘提供的 CCSwitch；CCSwitch 启动失败按原有可恢复提示处理，不回滚已完成的 Codex 安装。
4. 按当前产品既有方式向 CCSwitch 提供签名 USB 账号绑定和设备 API Key。404token Provider 使用现有 https://404token.xyz/v1 配置及 /v1/models 模型刷新能力。
5. 用户可以在 CCSwitch 中选择模型、Apply、切换到其他 Provider，再按 CCSwitch 原有流程使用。U 盘 Manager 不以“已通过模型探测”为条件拦截这些操作。
6. Codex 能否启动只由原有 USB 授权、程序完整性和安装状态门禁决定。不得因为用户在 CCSwitch 改了 Provider 或模型而要求重新通过 404token 专用模型验证。

“离线 U 盘”指安装器、Codex 程序和本地配置在 U 盘上运行；向 404token 请求模型仍需网络。基本连接不需要增加专用服务端接口：由 CCSwitch 使用设备 Key 调用现有 /v1/models，Codex 推理请求再经 HTTPS 发往 404token。

## 4. 高级功能：可选上游模型探测

将探测作为独立的高级诊断入口，例如“高级 → 404token 模型可用性”，不能自动运行，也不能作为安装向导步骤。

### 4.1 轻量连通检查

- 使用当前 USB 绑定的设备 Key 请求现有 /v1/models。
- 显示 API Key 认证及目录请求是否成功、服务端返回的目录和检查时间；界面只显示 Key 的标签或指纹，不显示凭证本身。
- 不发生成请求，不修改 CCSwitch 或 Codex 配置。
- T0 的 /v1/models 使用 API Key 分组上下文，但目录在没有实际账号模型映射时可能回退到平台默认项。因此返回某个模型只说明目录接口返回了该 ID，不证明当前上游账号可路由或模型兼容 Codex；轻量检查也不证明生成请求可用。

### 4.2 可选完整能力探测

- 用户显式选择模型并再次确认后，才发该模型的网关生成请求。
- 使用已有网关能力矩阵，不为降低费用跳过既有断言；按当前矩阵每个候选最多 12 个生成请求。
- 最多允许用户选择 3 个候选；按当前矩阵上限披露最多 36 个网关生成请求。此功能不启动 Codex，也不运行 Shell/Skills 原生会话。
- 真实调用可能产生费用且无固定金额上限；必须在本次探测开始前显示最多 36 个网关生成请求和费用不封顶说明，并取得明确同意。
- 结果仅是指定账号、endpoint、模型和网关协议矩阵下的诊断记录，不证明未来可用性或 Codex 本地 Shell/Skills 兼容。不得据此阻止普通 Provider Apply、改变默认 Provider 或自动覆盖模型选择。
- 取消、认证失败、超时、探测失败都只影响诊断结果；不修改已安装状态、CCSwitch 配置或 Codex 活动配置。
- 本方案不把网关探测结果写成可授予权限的持久化证明；诊断报告只在当前界面展示。取消或中断后，不自动重放当前模型；用户再次点击属于一次新的、需重新确认费用的请求。

### 4.3 不做的事

- 不新增 /v1/codex/compatibility 路由，也不要求 T0 服务部署配合。
- 不把 API 返回顺序、账号权重或字典序解释成产品默认模型优先级。
- 不自动遍历模型并发起生成请求。
- 不自动把探测结果应用为默认模型。
- 不把服务端认证目录、模型 revision、原生能力 proof 或 native_model_setup_ready 作为安装、Provider Apply、CCSwitch 启动或 Codex 启动的门禁。
- 不把 Manager 针对 404token 的回读校验用于用户之后在 CCSwitch 中切换 Provider 的路径。

## 5. 实施计划

### P0：保全工作区并建立精确差异清单

- U 盘仓库：D:\AI\AGENT_U盘\codex-desktop-usb-installer，分支 archive/skill-pack-1.3.0-20260924，HEAD e9514d08c343f594b0e452617bdf4a45bc0216b0。
- U 盘仓库存在大量既有未提交和未跟踪改动；不执行 reset/clean，不覆盖非本任务文件。
- T0 专用 worktree：D:\AI\SSH\_worktrees\codex-oneclick-cert-model-20260926，分支 codex/oneclick-certified-model-setup-20260926，HEAD c4d48cf753c62f166d848c06c1fc25a23430316d。此前为本任务新增的 compatibility 路由和部署改动已逐文件撤回；当前 worktree 与 HEAD 一致，无待处理差异。
- D:\AI\cc-switch 和 D:\AI\cc-switch-product 含有其他未提交改动。先确认 USB Provider Apply 门禁的实际源码和变更归属，只处理本任务新增的验证前置条件，不恢复或覆盖其他工作。
- 不修改普通 D:\AI\SSH\sub2api 工作区。

### P1：恢复默认流程

- 移除安装前/安装中要求模型费用确认、安装后强制 runAutomaticModelSetup、恢复安装时强制 nativeModelSetupReady 的逻辑。
- 移除 win_launch_codex 对 model_catalog::prepare_launch 的强制调用，确保更换 Provider 不会因 404token 的模型验证状态而无法启动。
- 移除只服务于强制安装门禁的 UI 状态、自动重试、安装费用勾选、native_model_setup_ready 及旧 refresh_native_model_catalog/cancel_native_model_catalog 接口胶水；按 P2 提供独立、只读的高级网关探测入口。
- 恢复 CCSwitch 原有 Apply 及 Provider 切换语义，不要求模型验证 proof。USB 签名、设备绑定及 Provider 资源完整性保护继续保留。
- 评估 codex_config_readback：不得在通用启动或 Provider 切换时要求配置仍为 404token；如只用于 Manager 自己的 404token 写入，必须与普通 CCSwitch 操作解耦。
- 删除或撤回 T0 专用 worktree 中只服务于 /v1/codex/compatibility 的改动，不部署、不推送。

### P2：把探测移入高级功能

- 新增独立的 Manager 诊断命令，复用已存在的 ProbeClient 网关矩阵；不得调用旧 refresh_native_model_catalog，因为旧路径会读写 sidecar、撤销或应用托管模型目录并可能执行 Codex 原生 Shell/Skills 流程。此规划不把 Codex Shell/Skills 原生测试纳入高级探测。
- 目录按钮只请求现有 /v1/models。完整探测先取同一设备绑定下的模型目录，再只接受用户从该目录选中的精确、唯一模型 ID；最多选 3 个。矩阵请求数单独统计，每模型最多 12 个生成请求；目录 GET 不计入 36 个生成请求上限。
- 每个候选矩阵最长运行 600 秒；最多三个候选按顺序运行，总时长最多约 30 分钟。用户可以中途取消；已发出的上游请求仍可能计费。
- 探测客户端的单次轻量 HTTP 请求（健康、模型目录和兼容性目录）默认超时 60 秒；Responses 生成、工具调用和流式请求默认超时 90 秒。单候选 600 秒矩阵预算仍是更高层的总边界，不能用延长单次请求来突破。
- 完整探测必须在发出任何目录或生成请求前验证选中数、模型 ID 和本次费用同意；没有有效同意时，生成请求为零。每次运行用新 request ID，不对取消／中断的候选自动重试。
- 确认轻量 /v1/models 检查不调用生成 API。
- 网关矩阵探测使用独立费用确认，结果只供用户参考；探测代码不写 sidecar、CCSwitch/Codex 实时配置，也不启动 Codex。
- 高级界面应标清当前设备凭证的短 SHA-256 指纹、endpoint、模型 ID、检查阶段、时间、通过/失败项目及实际生成请求计数；绝不显示或记录 Key。
- 探测结果不充当启动授权。由于高级探测不写配置，即使 CCSwitch 中同时切换 Provider，也不得覆盖用户新配置。

### P3：回归验收

必须验证：

1. 没有 404token 登录态、网络不可用或模型探测失败时，U 盘 Codex 安装仍按原流程完成，模型探测不会阻断 CCSwitch 启动。
2. 模型目录为空、缺失、未经探测或探测失败时，安装状态和 Codex 启动状态不被错误地设为 incomplete。
3. CCSwitch 中将 404token 切换到另一个 Provider，再 Apply 并启动 Codex，Manager 不得以 404token 专用 proof 阻止。
4. 从其他 Provider 切回 404token 后仍可按 CCSwitch 原有流程选择模型并 Apply。
5. USB license、设备绑定、签名、安装事务恢复、Codex 程序位置及哈希门禁继续有效。
6. 轻量连通检查只请求 /v1/models；完整网关矩阵最多发 36 个生成请求（另有 1 个 /v1/models GET），无明确费用确认时不发任何网络请求；失败/取消不改变配置或持久状态。
7. 运行受影响的 Rust、前端和 CCSwitch 测试；更新 mock 回归用例，确认“无模型 proof 仍可正常安装、启动与换 Provider”。
8. 构建新的未签名测试 Manager 只用于本地/mock 验收；发行包和实测必须绑定最终产物哈希与真实签名状态。

## 6. 当前证据与限制

- 规划形成时只做了源码和工作区只读核对；随后用户明确授权进入实现与测试阶段。
- 与 U 盘仓库 HEAD 对照，确认并移除了 OfflineHome 安装费用确认、安装后强制探测、恢复态证明检查、普通页面自动探测，以及 win_launch_codex → prepare_launch 的启动门禁。原有签名 U 盘授权、安装路径/哈希验证、事务恢复和 CCSwitch best-effort 尝试仍保留。
- 高级探测使用独立只读 Tauri 命令：只读取签名账号绑定和内置 provider 资源，且只接受精确的 404token provider ID；轻量模式只发 /v1/models，完整模式先重新读取模型目录并校验所选 ID，不写 sidecar/配置、不应用模型、不启动 Codex。
- T0 专用 worktree 此前有 16 个已修改文件和 3 个新增文件，均围绕新 compatibility endpoint；已逐文件撤回到 HEAD，未部署。此前不带凭证的线上 GET 曾返回 404 page not found；本轮未发送 API Key，未调用真实模型生成。
- 本轮本地验证已针对恢复流程重跑：`cargo test --manifest-path src-tauri/Cargo.toml` 通过（552 passed; 3 ignored）；`npm run check`、`npm run lint`、`npm run test` 通过（37 files; 443 passed; 1 skipped）；`npm run build` 和 `npm run tauri:build -- --no-bundle` 通过。renderer 构建有大 chunk 提示。
- 新测试 Manager 位于 D:\AI\AGENT_U盘\_codex-oneclick-test-artifacts\2026-09-27\restore-flow-optional-probe\CodexAppManager.exe，SHA256 `DC30E521AEB5007992E1DD7134CA443776AE1421ADD964B0CC9C06E8E091F617`，Authenticode 状态 `NotSigned`。它是本地验收 EXE，不是可发给用户的签名 U 盘包。
- 当时尚未完成物理 U 盘 GUI 安装/卸载和 Codex 原生会话；随后已在本机可访问的 E: 可移动卷上完成正式包静态核验，并使用 USB-0013 绑定完成线上矩阵联调。物理 GUI 验收仍不能由静态核验替代。
- 当前会话未修改 D:\AI\cc-switch、D:\AI\cc-switch-product 或随 U 盘携带的 CCSwitch 二进制。源码检索未确认这些源码目录中存在本任务所述的模型证明 Apply 门禁；随 U 盘的实际二进制仍需在有效签名 U 盘上现场核对。
- 本轮执行者在同一会话逐条复核了文档与源码、前后端边界和测试结果；按用户要求没有使用多 Agent，因此这不是独立审计。真实账号联调、CCSwitch 窗口内换 Provider、正式签名包和物理 U 盘结果仍待补齐。

## 7. 完成定义

默认流程恢复并通过本地模拟后，可报告“代码层安装/启动回归已修复”；只有真实签名 U 盘验收后，才能报告“正式 U 盘实测通过”。

高级探测要单独验收，不能以其结果作为正常安装、CCSwitch Apply、Provider 切换或 Codex 启动的条件。

在线连通验收分两档报告：

- /v1/models 成功：设备 Key 通过目录接口认证并取得一份目录响应；不代表响应中的模型实际可路由。
- 网关能力矩阵通过：指定模型在本次请求矩阵中通过了已列出的网关行为检查。真实 Codex 端到端推理需要另行实测；如将来增加此项，须作为单独可选动作并在调用前取得本次费用同意。

物理 USB 结果只代表对应 license、设备绑定、签名包、Manager/CCSwitch 哈希及所测 Provider。

## 8. 本轮执行结论与剩余现场项

- 默认流程模拟通过：无 404token 登录或模型证明时仍可完成 Manager 安装步骤并尝试启动 CCSwitch；既有安装可显示 Codex 启动入口。模型探测 API 没有被默认流程调用。
- 高级诊断模拟通过：目录模式零生成请求；完整探测按钮在费用同意前不可用；后端在创建客户端/发网络请求前拒绝无同意的矩阵请求；目录 ID 必须唯一且来自最新 /v1/models 响应；现有 loopback fixture 证明完整矩阵为每模型 12 个生成请求。
- 构建产物仅是 `--no-bundle` 本地 EXE。未制作签名安装包，也未把该 EXE 写入 U 盘发行目录。
- 执行者复核确认：默认安装、恢复 UI 与 `win_launch_codex` 不再引用模型探测证明；旧 native readiness/catalog refresh 命令已从 Tauri IPC 注册表撤下。CCSwitch 自身的 Apply/provider UI 未被本轮修改。
- 仍需现场联调：CCSwitch 窗口内切换/Apply/回切、Codex 原生会话和卸载清理。真实模型矩阵已在正式 USB-0013 绑定下执行；后续重复调用仍可能扣费，必须在开始前再次说明请求数并取得当次同意。
- 因本轮按用户要求在同一会话处理，没有独立文档审计者或代码审计者；不得将本文件或代码状态标记为独立审计通过。

## 9. 2026-09-28 线上复测与正式包复核（执行者记录）

本节覆盖第 8 节之后的实际复测，不能替代独立审计或物理 U 盘验收。

### 9.1 404token 线上协议复测

- `/v1/responses/compact`：`glm-5.2`、`deepseek-v4-flash`、`claude-fable-5` 均返回 HTTP 200、`status=completed`、一个 `compaction` 输出项和非空 `encrypted_content`；把该压缩项继续提交到 `/v1/responses` 后，三个模型均返回 HTTP 200、`status=completed`。
- 独立普通 Responses 流式请求：三个模型均 HTTP 200，均观察到 `response.in_progress` 和 `response.completed`，事件序号严格递增。
- U 盘 `ProbeClient` 完整矩阵（每模型最多 12 个生成请求）结果不全通过：
  - `deepseek-v4-flash`：12 次请求，text、single tool、tool result、multiple tools、tool error、stream、multi-turn 全部 `Verified`。
  - `glm-5.2`：一次完整矩阵在第 8 个请求的 stream 阶段为 `stream-invalid`；随后单独重跑时首个 text 请求出现 `upstream-response`。另行连续 3 次基础 text 请求均成功，说明完整上下文矩阵仍存在不稳定或上游响应差异，不能签发通过证明。
  - `claude-fable-5`：在第 2 个请求的 single tool 阶段为 `tool-correlation-invalid`。独立复测 3 次均返回 HTTP 200，但 `function_call.arguments` 不是合法 JSON，且带有额外 `{}` 前缀；客户端保持 fail-closed，不放宽校验。
- 上述探测失败只影响高级诊断结果；默认安装、CCSwitch 启动、Provider Apply 和 Codex 启动路径不调用该命令，也不读取其结果。

### 9.2 当前 U 盘程序与包复核

- `npm run test:offline`：37/37 通过。
- `cargo test --locked --manifest-path src-tauri/Cargo.toml model_probe::tests -- --quiet`：13/13 通过；`model_probe.rs` `rustfmt --check` 通过。
- `npm run test:offline-bundle`：离线目录模拟通过；该模拟使用合成 MSIX，不替代真实 Authenticode 和物理安装。
- 现有包 `E:\usb-oneclick-v0.5.24-timeout60-90-20260927` 使用 `verify-usb.ps1 -InternalSigned -SkipNetwork` 通过；签名器配对预检也通过，绑定 `prod-2026-01 / 0.5.24 / g39.5-20260924-r23-codex917-consolefix`。
- 包内自有 EXE 使用内部 USB license 签名模型；默认 Authenticode 模式会因自有 EXE `NotSigned` 失败，这是当前代码签名政策的已知边界，不应把 `-SkipSignature` 的结果描述成 Authenticode 通过。
- 本次线上复测使用的临时测试代码和临时凭证环境变量均已删除，未留下产品代码改动，也未向负责 404token 的会话发送新消息。

### 9.3 当前放行结论

在上游修复部署前，默认 U 盘安装闭环和正式内部签名包的静态校验可以继续使用；当时高级模型探测只能报告为“DeepSeek 当前矩阵通过，GLM/Claude 仍待上游协议/稳定性修复”。该历史结论由 9.4 的修复后复测更新，不应再作为当前三模型状态的唯一依据。

### 9.4 2026-09-28 上游修复后的重新复测

第 9.1 节记录的是服务端修复前的失败样本。随后在 404token 服务端部署了提交
`49e52b9d809c229f0bb638332d6595972c4d7304`（Claude function-call arguments done 修复），
使用已有构建缓存完成增量构建并通过中继部署。当前容器和二进制证据为：

- 镜像：`sub2api-adapted:49e52b9d809c229f0bb638332d6595972c4d7304`。
- 容器状态：`running|healthy`；启动时间 `2026-09-28T14:12:33.028964208Z`。
- 运行中 `/app/sub2api` SHA-256：
  `50e40245631b1384a514bb51cb73134d6b9c296bf530b0dc18c54ee2db7f49e2`。
- 部署前备份：`/opt/sub2api-deploy/backups/49e52b9d809c229f0bb638332d6595972c4d7304-20260928-221230`。

使用正式 USB-0013 绑定（账号行 `CCSwitch-USB-0013`，group 16）重新执行 U 盘同边界的 3 模型矩阵：
`glm-5.2`、`deepseek-v4-flash`、`claude-fable-5` 各 8 个阶段，共 24 个 HTTP 200 响应；
流式事件均连续并以 `response.completed` 结束，Claude 的
`response.function_call_arguments.done.arguments` 与累计 delta 及
`response.output_item.done.arguments` 一致。保留证据文件：

`_audit-artifacts/usb0013-matrix-49e52-live.log`

证据 SHA-256 为
`8030E8EB5C73CD01E2B50DE55822EE1632382653EB6BDE9E90CFC8BEBD057A94`。
日志已检查不含 `Bearer`、`Authorization` 或 `sk-` 凭证模式。该复测证明当前服务端协议和
三模型的本次矩阵调用已恢复；它仍不替代物理 U 盘 GUI、Codex 原生会话或独立审计。

正式包 `E:\usb-oneclick-v0.5.24-timeout60-90-20260927` 以
`verify-usb.ps1 -InternalSigned -SkipNetwork` 通过，返回 `ready=true`；独立
`verify-payload.ps1 -SkipSignature` 和 `verify-usb-launcher.ps1` 也通过。包内自有 EXE
仍按内部 USB license 签名模型运行，Windows Authenticode 显示 `NotSigned` 是已声明的
发行策略，不应将其误报成微软 Authenticode 签名通过。

该目录当前位于本机 `E:` 可移动卷；`usb-license.json` 的 USB-0013 卷序列号与
`fsutil fsinfo volumeinfo E:` 一致，设备绑定与实际介质匹配。当前系统已有更高版本的
官方 Codex 进程（26.924.2738.0），而包载荷为 26.917.8451.0；因此本轮没有强行启动
会触发版本冲突的安装 GUI，也没有关闭用户当前 Codex。物理 GUI 安装、CCSwitch 窗口内
Apply/切换和 Codex 原生会话仍需在无更高版本冲突的测试环境完成。

### 9.5 2026-09-29 fullfix4 本机发布前复核

在不改变上述历史证据的前提下，已重新审计当前源码并完成新的管理器、入口和一键目录构建：
`release/usb-oneclick-v0.5.24-fullfix4-20260929`。默认安装路径没有调用模型探测；探测入口只在已安装页面的“高级功能”折叠区，CCSwitch 通过签名 USB 绑定直接注入 404token provider，不再要求模型证明。安装、卸载、Codex 便携黑框补丁和 CCSwitch 自动注入的单元/前端回归均通过。

完整命令、失败边界、产物哈希和现场限制记录在
[`docs/reports/CODEX-USB-FULLFIX4-AUDIT-20260929.md`](./reports/CODEX-USB-FULLFIX4-AUDIT-20260929.md)。
该复核仍不替代有效签名 U 盘上的 GUI 安装、真实 CCSwitch Provider Apply、Codex 原生会话和物理卸载验收。
