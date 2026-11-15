# Skill Hub 刷新与安装交互修复

日期：2026-10-22
基线提交：`b9940aed14a9b74cbe9561021aa5d6a195d3a991`
基线工作树：存在其他未提交改动；本任务只允许修改下列 Skill Hub 文件和本开发记录。

## 用户目标

1. Skill Hub 第一次刷新偶发显示不可用，第二次刷新成功；降低临时网络波动导致的误失败。
2. 安装交互简化为“检查更新 → 点击更新完成”，不再出现更新前的二次确认弹窗；回退操作的确认保留。
3. 在任何更新检查或安装之前，就显示随 U 盘包提供的 Product Design 插件及其 10 项技能；远程 Skill Hub 的可安装 Skills 数量仍单独显示，不把插件虚报成远程目录项目。

## 诊断依据与边界

- 本次没有提供对应运行日志，因此不能断言此前具体失败的来源、HTTP 状态或代理错误。用户观察到同一操作第二次成功，符合一次性网络/服务端失败的表现。
- `skill_repo_update.rs` 已有 Relay→GitHub 备用来源、每个网络阶段的 deadline 和诊断 attempt 记录；共用的 `get_bytes` 对每个 GET 只发送一次请求。重试仅由用户再次点击触发，短暂的单次连接/服务端故障因此会先呈现为所有源不可用。
- 更新按钮目前先打开一个 `alertdialog`，用户还要在弹层内再次确认；这是“检查 → 更新 → 确认 → 安装”的额外步骤。
- USB `skill_repo_apply_update` 会一并安装本地 Product Design 插件，安装报告固定校验其包含 10 项技能；检查页面当前只展示远程 Skill Hub 数量，因此更新前看不到本地插件。
- 当前工作树已存在本任务之前的 Skill Hub 修改：后端 `SkillRepoUpdateInfo` 的 `product_design_skill_count` 字段已被移除；前端打开面板时的 `skillRepoProductDesignStatus` 查询和单独状态横幅也已移除；成功报告仍显示插件安装结果。实施不得回退这些改动。

## 冻结范围

允许修改：

- `src-tauri/src/app/skill_repo_update.rs`
- `src/app/views/SkillRepoUpdateCard.tsx`
- `src/app/views/SkillRepoUpdateCard.test.tsx`
- `src/app/i18n.tsx`
- 本文件的实施/验收记录

禁止：改远程来源地址/优先级、取消 TLS/哈希/metadata/归档校验、扩大更新文件权限、自动安装远程 Skills 而不经用户点击、改变回退语义、改 Product Design 插件包/脚本或其他能力包内容、修改其他现有未提交文件、打包/发布/推送。

## 实施约束

### 网络恢复

- 复用 `get_bytes`，每次逻辑 GET 最多自动重试一次，即该次调用最多执行两次请求尝试；所有尝试共享原有绝对 operation deadline。一次 check/apply 可能按既有一致性流程再次读取同一个 metadata URL（例如下载归档前后），这些是独立逻辑读取，不共享一个 URL 级总次数预算。保留这些复核读取可避免一次临时网络重试耗尽预算后跳过下载后的 metadata 一致性校验；每次底层 `get_bytes` 调用仍严格最多两次尝试，整个操作总时长仍由绝对 deadline 限制。reqwest 当前有界重定向跟随属于一次逻辑读取的既有传输行为；重定向循环/策略拒绝会终止这次读取，不再触发第二次逻辑读取重试。
- 只对可恢复网络错误重试：连接/传输失败、metadata/verify 小响应超时、HTTP 408/425/429/5xx。固定短退避，并遵守剩余 deadline。
- 大归档读取超时不在同一来源上重试，立即让现有下一来源 fallback 继续，避免把剩余总预算耗尽。
- 对其他 4xx、来源身份错误、重定向/响应格式错误、文件尺寸错误、SHA 不匹配和 ZIP 内容校验失败不重试、不放宽处理。
- 每次 `get_bytes` 发起的请求尝试都独立记录 attempt；同一次 reqwest 调用内部的有界重定向链按该次逻辑尝试记录。即使重试失败，也保留原来的最终错误分类和“未写入本地能力包”保证。
- reqwest 重定向循环/策略拒绝归类为不可重试的来源无效错误，不得被当成一般传输故障再次发起逻辑 GET。既有来源身份及重定向目标校验保持不变。
- 增加本机 HTTP fixture 回归：503→200 成功、404 不重试、归档超时仍进入原备用来源逻辑。

### 安装交互

- 保留“检查更新”与结果区中的唯一“更新”按钮；点击更新立即调用现有 `skill_repo_apply_update`，不再弹出安装确认。
- 保持忙碌状态、operation lock、前端防重、expected identity 绑定、应用失败信息与人工重试逻辑。
- “回退到上一版本”继续保留独立确认弹窗。

### Product Design 提示

- 在 Skill Hub 面板初始说明中明确列出“随包包含 Product Design 插件（10 项技能），更新时一并安装”，无需先检查远程目录才显示。
- 远程可安装数量继续使用后端 `installableCount`；说明本地插件是独立的随包内容，不改变 Skill Hub 数量。
- 更新成功后的既有 `Product Design 插件已安装` 成功提示保留。

## 风险分级

- D0：用户已明确指定交互结果，且现有组件/网络 client 给出可复用路径，无产品语义歧义。
- I2：来源 fallback、操作总 deadline、attempt 诊断与安装交互状态互相影响；用现有固定阶段和 fixture 做最小局部改动。
- A1：需审计网络错误分类、fallback/deadline、安装按钮状态和既有未提交改动是否保留。

## 验收

1. Rust 测试证明 transient GET 最多重试一次且可从临时 503 恢复；404/其他永久输入错误不重试；慢归档仍保留原备用源机会；现有哈希/来源校验用例通过。
2. 前端测试证明检查后一次点击更新即调用原有 apply API，无安装 `alertdialog`；回退仍有确认弹窗。
3. 初始 Skill Hub 面板不检查网络也能看到 Product Design 随包说明；结果页仍明确区分远程可安装数量与 Product Design 插件，成功页保留安装完成提示。
4. 运行 Skill Hub Rust 定向测试、组件测试、相关 TypeScript 检查及 `git diff --check`。
5. 冻结最终 diff 后由不同 Agent 独立只读审计。没有实体 USB 的实际现场日志，不把模拟测试描述成用户现场复现；本任务不做发行构建。

## 实施与审计记录

### 实施

- `src-tauri/src/app/skill_repo_update.rs`：为幂等 GET 增加单次有界重试，最大 2 次，固定 250 ms 退避并始终服从原绝对 operation deadline。仅重试传输错误、metadata/verify 超时，以及 HTTP 408/425/429/5xx。归档读取超时不在同一源重试，立即沿用既有来源 fallback；永久 4xx、重定向/来源身份、响应大小、哈希及 ZIP 校验错误仍不重试。每次 `get_bytes` 请求尝试写入独立 attempt 诊断。
- 审计后补强：`reqwest::Error::is_redirect()` 单独分类为 `SKILL_REPO_SOURCE_INVALID`，不再落入一般 transport 重试。测试使用本机 302 自重定向 fixture 和 `Policy::limited(1)` 实测；两次 HTTP 是 reqwest 在一次逻辑 GET 内的受限跳转，诊断 attempt 仅一条，未再发起第二次逻辑 GET。
- 再次复审补强：对最终 `response.url()` 的来源身份校验移至 HTTP 状态分类之前。静态来源跳转到不同路径后，即使目标返回 503，也先以 Invalid 拒绝，不会被当作可重试的 transient 5xx。回归 fixture 验证原源 302 → 不同路径 503：reqwest 跟随一次跳转，记录一条逻辑 GET attempt，无第二次逻辑重试。GitHub host allowlist 同样在判断 HTTP 状态前执行。
- `src/app/views/SkillRepoUpdateCard.tsx`：检查结果中的“更新”按钮直接调用原有 apply API；取消仅用于更新的确认弹窗。回退确认和忙碌状态、防重及失败诊断不变。
- `src/app/i18n.tsx`：初始说明已显示随 U 盘包提供的 Product Design 插件（10 项技能），并说明其随更新安装。远程 Skill Hub `installableCount` 仍单独展示；既有更新成功安装提示保留。
- `src/app/views/SkillRepoUpdateCard.test.tsx`：覆盖未联网检查时的 Product Design 说明、远程数量区分、一次点击即调用更新且不出现确认框，以及原回退确认行为。

### 验收记录

- Rust Skill Hub 定向测试：34 passed、0 failed、1 ignored（需要真实 Skill Hub 中转网络的 live round-trip）。HTTP fixture 覆盖 503→200、永久 404 不重试、503→503 时每次逻辑 GET 严格止于 2 次尝试、本机 302 重定向循环不触发第二次逻辑 GET，以及静态源 302 到不同路径且目标 503 时按 Invalid 拒绝而不重试；既有归档超时 fallback 测试确认超时后仅请求一次原源并进入中转源。
- `SkillRepoUpdateCard.test.tsx`：12 passed、0 failed。
- `npm run check`：通过（TypeScript `tsc --noEmit`）。
- `git diff --check`：通过；未跟踪开发文档无尾随空格；Rust 文件 `rustfmt --check`：通过。
- 未做正式打包、推送或实体 U 盘网络现场复现。独立只读审计由主控安排，本记录不代替审计结论。

## 后续现场问题记录：Skill Hub 更新被离线插件报告解析中断（2026-10-26）

### 现象

测试机点击 Skill Hub 更新后失败，界面错误为：

```text
update engine error: 离线 Product Design 安装结果无效: invalid unicode code point at line 1 column 313
```

### 已确认的证据

- 随附 `codex-app-manager.log` 中有三轮 Skill Hub 检查；三轮均通过 `ph-relay` 完成 metadata、archive 和 verify，选中的提交为 `351e3c0364e91130e57ae0aef8dfcc3529c4823b`。因此，这次失败没有证据指向 Skill Hub 检查阶段或远程归档下载。
- `src-tauri/src/commands.rs` 的 `skill_repo_apply_update` 在 USB 模式下先调用 `install_product_design_plugin`，成功返回后才调用远程 `apply_skill_repo_update`。
- `src-tauri/src/app/capability_pack.rs` 启动 `powershell.exe` 并捕获 stdout；子进程退出码成功后，Rust 直接将 stdout 当作 UTF-8 JSON 解析。解析失败会返回截图中的错误，因此远程 Skill Hub 应用步骤不会继续执行。
- `installer-pack/install-product-design-plugin.ps1` 在输出 JSON 前会写入 Product Design 配置，并验证插件文件；结果 JSON 还包含 `runtimePath` 与 `installedPath`。安装脚本没有显式声明与 Rust 捕获端一致的 stdout UTF-8 编码契约。
- 日志和截图未提供该次 PowerShell stdout 原始字节，无法据此证明文件复制、配置写入或 Product Design 插件安装最终状态；也不能把错误归因到某个具体字符的编码字节。

### 当前判断与影响

PowerShell stdout 与 Rust UTF-8 JSON 解析之间的编码不一致是一个待核实假设。测试机日志显示运行配置路径含非 ASCII 用户目录；脚本输出 JSON 又包含本机路径，这与 `invalid unicode code point` 相符，但由于原始 stdout 字节未留存，尚不能确认根因。

该调用顺序还意味着：即使离线 Product Design 脚本已完成文件和配置写入，只要结果 JSON 解析失败，整个 `skill_repo_apply_update` 就会报错，远程 Skill Hub Skills 不会应用。现场应分别检查 Product Design 文件/配置与远程 Skill Hub 代际状态，不能只根据界面错误推断两者都未安装。

### 后续诊断与修复门槛（本次仅登记，未实施）

1. 先在发生故障的测试机上，对同一 PowerShell 版本、同一安装脚本和非 ASCII 临时目标路径捕获子进程 stdout 原始字节，并记录 PowerShell 版本、代码页、退出码及 JSON 解析结果。不得输出凭据或完整用户隐私路径。
2. 只有在原始字节证实编码不一致后，才为 PowerShell stdout 与 Rust JSON 捕获建立明确、相同的 UTF-8 字节约定；若字节本身是合法 UTF-8，则继续检查 stdout 是否混入额外输出、脚本实际版本/内容及目标路径字符，不先改编码逻辑。
3. 根因确认后增加非 ASCII `CODEX_HOME`/用户目录路径回归，并验证 Product Design 安装报告成功后远程 Skill Hub apply 会继续执行。
4. 修复后检查失败重试是否幂等，并确认 MCP 配置及其他既有 `config.toml` 内容保持不变。

### 本机补充探针（2026-10-26）

- 使用本机 Windows PowerShell 子进程，通过管道原始字节流捕获与脚本相同形状的 `ConvertTo-Json` 输出；字段中包含非 ASCII（中文）Windows 路径。
- 子进程退出码为 0；stdout 经严格 UTF-8 解码及 JSON 解析均通过。
- 探针只在临时目录创建并随后清理脚本文件，未启动 CodexAppManager，未调用 Product Design 安装脚本，也未访问或修改真实 `CODEX_HOME`。
- 该结果说明本机该 PowerShell/系统设置下没有复现编码错误，削弱了“默认 stdout 编码必然不兼容”的推断；不能代表测试机的 PowerShell 版本、区域设置或实际安装器输出，也不能排除现场字节损坏或额外 stdout 内容。测试机原始 stdout 仍是定位根因的关键证据。

本节记录现场问题、诊断边界与本机探针结果；本次没有修改生产代码、运行 Product Design 安装器、改动测试或打包。
