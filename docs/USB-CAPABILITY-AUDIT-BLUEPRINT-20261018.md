# U 盘 Codex Skills 与 Agents 能力审计蓝图

审计日期：2026-10-18  
目标：U 盘 Codex 开发源 `codex-desktop-usb-installer/installer-pack`，不是本机 `%USERPROFILE%` 下的安装目录。  
发布基线：能力包 1.5.4，`sourceCommit=3cfa9fca12e24b25be4b2f7f5b273e6b9fb22040`。  
开发仓库 HEAD 与该 sourceCommit 一致；审计时 `installer-pack/` 有 22 项预先存在的未提交状态，必须原样保留。基线树共 315 个文件。可复现冻结指纹由 `docs/audit-capability-pack.py` 计算：递归列出包目录下所有文件，按相对 POSIX 路径排序；每行写 `relative/path|sha256(raw file bytes)`；以 LF 连接、不追加末尾 LF，UTF-8 无 BOM 编码后再取 SHA-256。指纹为 `fa6670eabe4d5a921ba42e21e1a9a969687a098961e65eda3d8538114e5c800c`。运行命令：`python -B docs/audit-capability-pack.py installer-pack`。新增审计文档不在 `installer-pack/` 下，因此不改变包树。本轮最初在 G2 前已观察并记录 22 项 dirty 与 315 文件，但 `G0-source-baseline.txt` 持久化快照是在 G2 后写入，故该持久证据只能用于事后核对，G0 在清单中降为 PARTIAL；测试前的观察仅有会话记录，未归档原始快照。
目录基线：56 个 skill 目录、7 个 Agent TOML；`pack.json` 默认启用 49 个 skill、7 个 Agent。其余 7 个 skill 是 `guided-config`，默认关闭。

## 1. 冻结目标与边界

### 用户目标

全面检查 U 盘 Codex 开发源内预装的 56 个 Skills 和 7 个 Agents 是否可发现、能否按说明工作、依赖缺失时能否正确降级，并为每项保留可复核证据。

### 非目标

- 本机用户级安装状态只作为测试环境准备信息，不作为 U 盘源审计结论。
- 不覆盖或删除现存 skill/agent，不改 MCP、配置、凭据或系统运行时。用户在后续明确授权后，允许将 U 盘源中本机确实缺失的项目追加安装；追加安装需独立记账，不能据此冒充源版本隔离测试。
- 不发布内容、不发送邮件、不改云端文件、不进行真实交易或其他外部写入。
- 不发起会计费的图片/视频/API 请求；媒体能力先用已有离线协议测试和 mock 验证。
- 不把阅读说明、静态文件存在或模型自述当作运行成功。

### 用户确认的本机运行代理口径（2026-10-18）

用户明确指定：U 盘 Codex 安装后的实际效果理论上应与本机 Codex 相同，因此**本机 Codex 中可复现的实际运行结果可作为 U 盘安装后行为的验收代理**。后续验收按此口径推进，不因没有隔离加载 `installer-pack` 源路径而自动把行为项判为缺证。

- 本机测试仍需记录实际触发方式、执行环境、依赖、结果和副作用；静态阅读/平台自述仍不算运行通过。
- 这一代理口径是用户批准的验收假设，不等于证明 USB 源文件被当前运行时直接加载，也不抹平同名本机文件与 USB 源内容哈希不一致的事实。哈希差异单独作为版本等价风险记录；行为验收结论和源文件路由证据分列。
- 对已完成的本机 Codex 回合，按具体向量重新判定：行为与安全断言完整满足的，可从 PARTIAL 升为 `PASS (本机运行代理)`；依赖、输入、负例、工具调用或副作用边界仍未覆盖的，继续保留 PARTIAL/FAIL/NOT_TESTABLE。
- `FAIL` 仍代表观察到错误路由、违反合同或越权副作用；用户代理口径不把失败转为通过。

### 允许改动

只允许新增/更新本审计目录下的蓝图、执行清单和证据记录；另有用户明确授权的本机 additive 安装例外：仅把缺失项目追加到用户目录，不覆盖同名项目。2026-10-18 已向 `C:\Users\T14S\.agents\skills` 新增 5 个缺失 Skill；7 个 Agent 均已存在，未修改；14 个同名但内容不同的 Skill 均跳过。不得改动 `installer-pack/`、构建脚本、配置、凭据及其他现存未提交文件。

## 2. 基线与风险分类

- 设计歧义：D0。范围明确为当前开发源内的 56 Skills 与 7 Agents。
- 执行难度：I1。分阶段盘点、静态核验、无副作用冒烟和依赖门控验证。
- 审计风险：A1。涉及跨能力依赖、错误降级、Agent 与 Skill 的组合，以及必须遵守的无付费/无外部副作用约束。
- 独立审计：由未参与本轮执行的只读 Agent 对冻结版清单、证据和结论复核。当前运行路由证据未知，不能声称特定模型/effort 或 Hook 已验证。

## 3. 组件验收标准

每个 Skill 都要分别回答以下问题。逐字自然触发正向/负向提示和验收断言见 `USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md`；执行清单只引用对应 ID，不允许临场改写提示后声称同一测试。

1. **包完整性**：目录、`SKILL.md`、frontmatter、引用资源和 manifest 项存在且相互匹配。
2. **触发与边界**：在新对话逐字提交正向探针、再单独提交相邻负向探针；分别记录 Codex 可观察的源 Skill 加载/路由证据。显式点名、把 SKILL.md 贴入上下文、执行者自称使用某 Skill 均只能证明显式工作流/内容质量，不能证明自然发现。若无法证明当前会话加载的是本次 U 盘包中同一版本，则将 **USB 源文件加载/路由来源状态** 标为 `INSUFFICIENT_EVIDENCE`；行为质量状态按本蓝图中用户批准的本机运行代理口径另行判定，不因来源未证自动降级。
3. **代表任务**：用一个小型、无副作用任务检查输出是否具体、完整、符合 skill 的方法与格式。
4. **依赖与失败路径**：分别核对可用依赖路径和无凭据/MCP/运行时路径；后者必须明确说明缺项，不得伪造成功。
5. **副作用约束**：要求发送、发布、删除、付费或修改外部状态的技能只检查准备/预览与安全停步，不实际执行。

每个 Agent 都核验 TOML 结构、身份/职责、required/optional/blocked skills、依赖闭包、典型任务委派结果、边界遵守和失败降级。具体提示见测试向量文件。配置正确不等于运行时已发现或真实调用；无运行时事件时必须保留触发/路由证据缺口。

### 结果状态

- `PASS`：当前包版本上有可复核运行证据，代表任务通过，依赖与负向边界也符合约定。
- `PARTIAL`：核心行为有证据，但真实依赖、触发来源或某条边界路径未完成。
- `FAIL`：缺失、不能执行、结果违反合同，或出现虚构成功/越权副作用。
- `NOT_TESTABLE`：需要未提供的账号、MCP、软件、权限或计费调用；记录具体原因与可执行的后续条件。
- `INSUFFICIENT_EVIDENCE`：只有静态检查或自述，无法证明目标版本上的真实调用/行为。

测试质量、实际触发证据、依赖可用性、模型/effort 路由和 Hook 状态分开记录。依赖不可用不是 PASS；也不能仅因此断定 skill 实现失败。

## 4. 执行阶段与顺序

严格按 `USB-CAPABILITY-AUDIT-CHECKLIST-20261018.md` 的编号顺序推进，先 56 个 Skills，再 7 个 Agents。

| 阶段 | 内容 | 放行条件 |
|---|---|---|
| 0 基线冻结 | 提交号、工作树差异、包版本、目录数、清单数、关键哈希、执行环境 | 基线记录完整，用户现存改动未被覆盖 |
| 1 静态闭包 | 56 个 `SKILL.md`、7 个 TOML、frontmatter、引用资源、依赖 ID、默认状态、文件清单/校验和 | 每项静态结论有路径/字段/哈希证据 |
| 2 现有离线测试 | 跑能力包现有兼容性、adapter、媒体 MCP mock/contract 测试；只用本地临时产物 | 测试命令、退出码、摘要可复核，无真实外部提交 |
| 3 Skills 冒烟 | 按编号逐个检查触发意图、代表任务、失败降级与副作用边界 | 每个 Skill 有输入、关键步骤、输出摘要和状态 |
| 4 Agents 冒烟 | 按编号逐个核验角色路由、任务拆解/交付、依赖组合和越权边界 | 每个 Agent 有独立样例证据；不把 Agent TOML 当运行证据 |
| 5 汇总与独立复核 | 统计结论、列阻塞/缺证项、独立审核同一冻结版本 | 审核给出 PASS/FAIL/INSUFFICIENT_EVIDENCE，修复后重审 |

如果当前 Codex 无法临时加载 U 盘源目录中的精确版本，按上述用户批准的本机运行代理口径验收行为，并明确保留源路由/哈希差异；不得把“代理验收 PASS”表述为“运行时已证明读取 USB 源文件”。本轮本机缺失项追加安装仅用于补足缺件，已有同名版本不覆盖。

## 5. 测试输入和证据规范

- 每个 Skill 用 manifest 的能力描述构造最小案例；Guided-config/unsupported 项优先验证“依赖缺失时安全停步”，不得填入假凭据。
- 对每项固定记录以下字段：`case_id`, `target_source`, `source_commit`, `source_tree_sha256`, `prompt_id`, `trigger_mode`（natural/explicit/static/offline-test）, `observed_route`（runtime event/source path/unavailable）, `input_fixture`, `expected_assertions`, `actual_summary`, `dependency_state`, `side_effect_check`, `status`, `evidence_path`, `evidence_sha256`, `run_at_utc`, `executor`。触发证据与行为证据分列；没有运行时事件时 `observed_route=unavailable`，不得补猜。
- 证据文档/安全合成产物存放在 `docs/capability-audit-evidence-20261018/`（本审计新增目录），不得写到包本体。记录任务编号、源文件路径、短摘要、时间、命令/工具结果和哈希；不得记录密钥、私有内容或完整个人数据。
- 现有离线测试按清单中的完整 PowerShell 命令执行。Windows 沙箱默认 C: 临时目录可能因权限失败；只在明确看到 temp 权限错误时，建立 `D:\AI\AGENT_U盘\.tmp-capability-audit-20261018`，把 `TEMP`/`TMP` 临时指向该目录重跑一次，并原样记录两个结果。测试用 mock/临时目录；不运行任何真实提交。
- 写文件测试只在新建的临时测试目录中进行，完成后留存最小可复现产物或清理记录；不覆盖任何用户文件。
- 网络查询若本身免费且只读，仍标记时间与来源；涉及登录、权限、付费或外发数据则跳过真实调用。
- 测试失败最多按新的证据路径复试两次；同一路径第三次失败即记录阻塞，不重复碰运气。

## 6. 已发现的基线问题

初始冻结基线的 README 曾写“55 个 Skill、47 个默认、8 个 guided-config”，而包目录和 `manifest/skill-audit.json` 已是 56/49/7。后续已按用户授权更新 USB 源 README；当前 README、包目录与 manifest 均为 56 个 Skill、49 个默认、7 个 guided-config。旧差异作为历史审计发现保留。

`file-manifest.json` 与当前脏工作树中两个文件的哈希不一致：`mcp/media-generation-mcp.py`、`test-media-generation-mcp.py`。此外 `__pycache__` 中 4 个 `.pyc` 文件不在 `file-manifest.json` 里；其创建时间为 2026-10-13/17，早于本次审计，故当作预存工作树内容保留，不清理。它们使“所有包文件均在 manifest 中”的闭包也失败。这属于冻结源的完整性/生成物卫生问题；不得在本审计中重写 manifest/checksum 或删除缓存来消除证据。

## 7. 运行命令与判定

在仓库根目录运行，先验证结构脚本，再按 G2 的顺序执行包内离线测试：

```powershell
python -B docs\audit-capability-pack.py installer-pack
python -B installer-pack\audit-codex-compatibility.py --root installer-pack --json
$env:TEMP = 'D:\AI\AGENT_U盘\.tmp-capability-audit-20261018'
$env:TMP = $env:TEMP
python -B installer-pack\test-media-generation-mcp.py
python -B installer-pack\test-adapter-contracts.py -q
```

退出码 `0` 才能通过相应脚本；非零退出必须记完整失败摘要。兼容性审计的 file hash mismatch 是当前包 FAIL。Adapter suite 的媒体子套件断言仍硬编码 `Ran 39 tests`，而当前 mock 套件报告 `Ran 53 tests ... OK`，所以 adapter suite 整体 FAIL，即使 53 个媒体 mock 本身全通过也不得把 gate 记 PASS。25 个 skip 需按用例标记解释，不能用总体成功掩盖。

## 8. 完成条件

- 清单覆盖 56 个 Skills 和 7 个 Agents，不遗漏、不重计。
- 每个组件有状态、证据、依赖限制和后续动作；未完成项目明确保持未完成状态。
- 所有高风险/外部依赖项均遵守“无真实付费、无外部写入”的范围。
- 汇总结论区分功能质量、触发/路由证据、Hook 与依赖健康；独立只读审核完成或明确报告其不可用。

## 9. 当前执行补充（2026-10-19）

第 1、6、7 节中的初始包指纹、文件闭包与旧 G2 失败断言是历史证据，不代表当前修复后的源包。随后按用户授权对 USB 开发源做源级修复与回归，包括 S017 动作禁用边界、S030 过敏原约束、S045 行程备选回传、S053 HTTP 错误凭据脱敏。**截至本节对应的早期增量快照（已被后续源修复快照取代）**，`installer-pack` SHA-256 为 `d5c9964c4c9b7f5d298b957d245b1ce4d1ecad75abaf747248da8567e785ac92`，315 个包文件（10 个 transient cache 排除于序列化），313 个 manifest 项、314 个 checksum 项；G1 闭包与 metadata `--check` 通过。G2 兼容性15项、media MCP 64/64、adapter contracts 76通过/25旧媒体 fixture 跳过；Windows 系统 Temp 的权限失败在将 TEMP/TMP 改到工作区路径后消失。该历史阶段报告见 `capability-audit-evidence-20261018/G2-post-S031-status-guard-20261019.md`；当前包指纹见本节末尾最新快照。

S008/S034 配置审阅发现 manifest 描述把默认安装与隐式调用混为一谈，而兼容性检查明确要求除 `image-2` 外其他 Skill 显式调用。源政策文字和两条正向固定向量已同步为显式 `$skill`；一次尝试打开自然调用被兼容性 gate 拒绝并回滚，失败输出作为负向证据保留。S008 显式代理行为已有历史运行证据；S034 当前源的显式正负提示词受控代理通过，但不能证明 Codex 原生路由或 USB 安装读取，真实配对后端也不可用，仍为 PARTIAL。

截至早期增量复核，S031 批准 fixture 的修订源代理正负边界通过；状态未提供时标“待确认”，保留决议、相对日期与待验证事项；不证明 Codex 原生路由。初测错误 fixture 已标为 superseded。证据见 `S031-approved-fixture-status-guard-proxy-20261019.md`。S036 当前源固定向量代理通过；当时 S050 代理尚未执行实时天气查询。后续 2026-10-19 已在当前执行环境通过网页回退取到来源/时间可核验的上海实况与预报，并按源保留预报差异；USB Skill 自然路由和 curl 回退仍未测，详见 `S050-postpatch-web-fallback-behavior-20261019.md`。

S035 当前 USB 源已将虚构示例中的具体薪资、ROI、价格、退款和限额承诺改为待核验占位符，并规定赠品、风险逆转和稀缺性为可选组件；概念问题不进入完整 Offer 模板。source-contract 检查、与更新后固定正负向量逐字匹配的 source-pinned 代理以及独立只读复审均通过。当前仍为 PARTIAL，直到运行时路由/USB 加载有证据。最终报告见 `S035-exact-vector-source-pinned-proxy-20261019.md`、`S035-independent-audit-20261019.md` 和 `S035-offer-guardrails-20261019.log`；较早非精确代理另标为 superseded。

S038 三页合成 PPTX 已经由 Artifact Tool 渲染并逐页目检通过；S043 搜索代理、S049 财富配置负向代理和 S031 会议代理已补充独立复核。S050 现已在当前 Codex 执行环境验证网页回退、观测/预报区分、来源/更新时间标注和预报冲突保留；USB Skill 自然路由仍未证明。S028 又通过 USB provision-runtime 的隔离 profile 验证 Manim/Node/FFmpeg 运行与输出；内置 Python 引导和自然路由仍未证。这些源级回归没有证明 56 个 Skill 的自然触发/运行时来源，也没有证明 USB Agent TOML 被 Codex 运行时发现。最新清单按源/本机逐树等价审计后，G3 为 12 PASS/39 PARTIAL/0 FAIL/5 NOT_TESTABLE；S047 当前 USB 源四个脚本的 source-direct smoke 均通过并经独立复核，但自然路由/源加载未证，因此按 PARTIAL 记录。仅29/56个USB Skill与至少一个本机副本内容一致，另27项本机行为不能代理当前源。G4 仍为 INSUFFICIENT_EVIDENCE。S053 通过9项包内测试和20项source-direct全mock离线契约断言但自然路由和真实服务账户未测，整体仍为 `NOT_TESTABLE`。逐项最新状态以 `USB-CAPABILITY-AUDIT-CHECKLIST-20261018.md` 为准；不得把源代理或静态检查说成运行时路由结论。


2026-10-19 增量：S054 `wechat-article-creator` 将字符计数字段从 `word_count` 更正为 `character_count`；USB源脚本9项、包内回归2/2、元数据/兼容性/PowerShell/G1门禁通过并独立复核。该修复不证明自然Skill路由、USB源加载或文章质量；S054仍PARTIAL。当前包树 SHA-256 `0d9bb260753490771ad9af934b5cc746a59e8970e65b0570bb5b2b40df340f6d`。


2026-10-19 增量：S037 PDF helper 对当前 U 盘源执行9项合成 source-direct CLI 检查，覆盖生成、读取、抽取、合并及错误边界；只说明脚本层可用，不证明自然 Skill 路由或 USB 运行时加载。详见逐项清单。


2026-10-19 S028 Manim 锁定依赖离线安装和基础场景 H.264 渲染通过（33项、854×480、17帧）；随后 USB provision-runtime 在隔离 profile 中完成复测，证明已有 CODEX_PYTHON 条件下 Manim/Node/FFmpeg 健康并成功渲染/ffprobe，内置 Python 引导和自然路由仍未测。S050网页回退后测通过来源/更新时间、实况与预报分离及分歧保留检查；自然路由与 curl fallback 未测。两项仍为PARTIAL。源修复闭环：S052修复识别占位符、标准Base64 1×1 GIF/PNG与无引号懒加载`src`提升，包内回归7/7，真实html2text与假Fetcher验证6种图片形态/26断言和Markdown链接；S053拒绝bool型`max_words`，9项包内测试和20项mock契约断言通过。G1当前USB包树 `53a5f41453a44041e8b26379702cd6e236e1cfd80d7efe41d3e00a96816ae895`（322文件，16瞬时缓存排除）；G3为12 PASS/39 PARTIAL/0 FAIL/5 NOT_TESTABLE。S052因当前源与历史本机副本不等价、自然路由/真实抓取未测，保持PARTIAL；S053仍NOT_TESTABLE for natural routing/runtime. G4新增独立只读闭包复查及可复跑脚本：7份 Agent TOML 的51个常规引用+2个blocked PDF引用全部闭合；运行时加载/child lifecycle仍未证。当前逐源比对29/56本机副本树等价，另27项在本机全部已安装副本中均有差异。S052占位符探测边界限于已识别字符串及标准Base64 1×1 GIF/PNG Data URI，不表示支持任意图片Data URI。该段当时证据账本为519条；S028 provisioner、S050网页回退、S022在线npm代理探测、S051当前树复跑及S028 Python安装器签名预检证据亦已归档。


2026-10-19 S002 增量：本地 Canvas HTML fallback 的真实 `flow.js` 与 HTML inline script 已核对一致，并用 Node VM + in-memory DOM stub 对事件和状态流转执行54项断言通过。该项仅验证逻辑，不等同浏览器渲染/视觉或真实键鼠验收；浏览器策略拒绝仍遵守，S002保持PARTIAL。证据见 `S002-flow-behavior-followup-20261019.md`。证据账本增至486条。

2026-10-19 后续增量：S028 固定正向勾股预览先被独立审计发现斜边图形是长方形，已修正为等边长且垂直的斜边正方形，并加入运行时几何断言；重渲染后四帧通过独立目检，H.264 854×480、131帧、8.733008秒。只闭合正向渲染，负向“不渲染”、自然路由和安装后源加载仍未测，S028仍PARTIAL。S037 通过包provisioned `CODEX_PYTHON` 执行生成/重开/info/抽取/单输入合并子测试，12项显式断言通过；新增两输入合并补测12项通过、输入顺序正确且输入/runtime-state哈希未变，独立只读复核通过。自然路由/负向对话仍未测，整体PARTIAL。S026 使用真实provisioner生成的隔离profile FFmpeg/FFprobe wrappers 完成含中文和空格路径的合成A/V裁切，18项显式断言通过、输出3秒且音画时长差0；独立只读复核确认 wrapper/runtime/source/output 证据相符。S051修订后的当前树harness 10/10经独立复跑；旧独立报告已标为修订前历史。S009当前 Word COM 在应用创建前再现 `0x80070005 E_ACCESSDENIED`，未打开源DOCX、未生成PDF/页图，源哈希不变；按同一入口停止该视觉渲染尝试，未安装renderer或换入口。当前证据账本595条，路径/哈希闭包校验通过。S026自然Skill路由、USB安装生命周期和任意用户视频兼容性仍未证，保持PARTIAL。


2026-10-19 S009 历史增量（当时账本495条，现已更新至557条）：当前源生成的 DOCX 结构已有检查；打包 `render_docx.py` 使用 bundled Python 时因 Windows 缺 `soffice.exe` 失败，仓库内临时目录重跑复现；Word COM fallback 被 `E_ACCESSDENIED` 阻断，未生成页面图。S009保持PARTIAL，视觉验收缺口仍开放；首次失败的临时目录在工作区外且清理遇访问拒绝，已记录且未擅自删除。详见 `S009-current-render-attempt-20261019.md`。证据账本增至495条。


2026-10-19 S054 增量：基线长中文标题封面存在裁切与副标题重叠；`generate_cover.py` 改为像素宽度换行/字号适配并新增4项布局回归。当前 USB provisioned profile 生成长/短标题合成封面，14项断言、独立目检、G1完整性、兼容性和 PowerShell 5.1 门禁均通过。布局专项闭环；自然路由、USB运行时源加载、写作质量和发布未测，S054总体PARTIAL。当前 G1 包树 323 文件（排除16瞬时文件），SHA `78de7f83d82ccedef804c1fa3150f75184fd33abb179273f516cbe5925fdc1e7`。


2026-10-19 S007 增量：冻结秒级时钟后，当前源七类记录 ID 均可重复；已通过最小 UUID4 修复并保留按日期/按习惯日期 upsert 的原 ID。修复后冻结时钟 source-direct 七类各两条均唯一，包内3项回归通过；G1/兼容性/PowerShell 门禁通过。S007仍PARTIAL：对话流、自然路由和运行时源加载未证。当前 G1 包树 324 文件（排除16瞬时文件），SHA `e43c42340001a66e1639c4213cf1883e0665d7063a2000d47102b146adb02e5c`。


2026-10-19 S017 增量：冻结合成 profile 复现 Bro Split 手臂日因 `biceps`/`triceps` 只查顶层库而不可用；upper/lower推拉日也漏手臂目标。修复只映射这两个嵌套组；输出动作类别按库归类，回归覆盖器械、避开全部手臂动作及 negative fixture。完整程序生成器回归16/16通过，G1/兼容性/PowerShell门通过。S017仍PARTIAL，训练科学性/个体安全及路由未证。当前 G1 树325文件（排除16瞬时缓存），SHA `20c864b3584af33a131235a08ed87f9fd40419cf0ed6f5370d43a92643579027`。


2026-10-19 S018 增量：冻结输入 `monthly=1000, years=100, annual_return=1000` 在声明允许域内复现 `OverflowError` 与 CLI traceback；修复把溢出/非有限数转成稳定错误并阻止 JSON `Infinity`，新增 4/4 回归，30年8% 对照符合独立年金公式。四个基金 helper 既有 10/10 smoke 和 adapter 套件 78 tests/25 skipped 在工作区 TEMP/TMP 下通过。另从历史 build-pack.py 恢复精确 `sourceTreeSha256` 规范，接入包元数据刷新及只读检查；324 manifest/325 checksum、G1 326文件（排除18瞬时），G2 15项与 PowerShell 5.1 的14入口检查通过。S018仍PARTIAL：负向对话、USB 自然路由/源加载和实时 AkShare 未验证。独立只读审计 PASS；补充测试覆盖刷新后篡改源文件时 `--check` fail-closed。当前 G1 树 SHA `aa15db7d3a33e8c40b8b12972fdd5ea502fb8002015965261c0ecd8950d5c96f`，sourceTree SHA `a1185b66748aa03df1e8282f691392543a84c959d2e8d59375481fb9c8210370`。

2026-10-19 S049 增量：复现再平衡权重键 "A"/" A " 在去空格后冲突并被字典覆盖，导致原始150%当前权重表被误判为100%且输出零偏差。输入校验现拒绝 current 或 target 中规范化后重复的资产代码；独有空格修剪仍正常。新增 CLI/API 回归，source-direct smoke 11/11；adapter 80 tests/25 skipped；metadata 324/325、G1 326文件（排除18瞬时）、G2 15项、PowerShell 14入口通过。自然路由/实时数据仍未验证，S049保持PARTIAL；独立只读审计 PASS。G1 树 SHA `2138751f47c192675dfbc9795eae6b1294e8b1ea993da864f9e411c301f5f4eb`，sourceTree SHA `49eae6a10beec20fc4105105daefae8ba3816cb033b68729fc7f4991ce5ef377`。

2026-10-19 S030 增量：冻结时钟复现 `meal_db.add_plan()` 与 `log_meal()` 的秒级 ID 碰撞；按第二个计划 ID 更新时错误命中首条。两处现改 UUID4 字符串；固定时钟和隔离 JSON DB 回归2/2，确认两份计划分别查更、两条餐食日志各自保留。既有营养 source smoke 61/61、adapter 80 passed/25 skipped、metadata 325/326、G1 327 文件（18瞬时）、G2 15项及 PowerShell 14入口通过。S030 同秒 ID 修复经独立只读审计 PASS，报告见 `independent-audit-S030-meal-db-id-collision-20261019.md`。自然路由/真实档案和营养质量仍未验证；S030总体PARTIAL。G1树 `10553feee54dca17f6c7c986f6beaf52679c4e60ae9714bd134cd2085d28f51f`，sourceTree `0904d321fc21293385957eb3c5a31c8932a87f24882fa0cbe28ae23c78eef5ee`。


2026-10-19 S017 增量：冻结时钟复现 `fitness_db.add_program()` 与 `log_workout()` 秒级 ID 冲突；按第二个方案 ID 更新却改到首条，按第二个日志 ID 查询返回首条。两处改用 UUID4 字符串；新增固定时钟隔离回归2/2，并复跑原生成器13/13、肌群覆盖3/3、adapter 80/25。独立只读审计 PASS，独立复跑固定时钟回归和 metadata/G1；G2 15项、PowerShell gate 14文件通过已有本轮日志。S017整体仍PARTIAL；不迁移历史重复ID，自然路由、USB运行时加载和训练科学性未证。当前快照：G1 328包文件/18瞬时，SHA `0892aeaac222c2846a03819b7c17efa20ee97d6bbc782406beac6df114a0decc`；sourceTree `24f3b5abfe5aa6c671f8f9f1fc8e90702035b46b9fa3feb00597bea17746c2a2`。


2026-10-19 S007 增量：冻结基线证实 `reflection_db.load_json()` 将损坏 JSON 当作空对象，`save_profile()` / `log_reflection()` 随后覆盖 `profile.json` 与 `reflections.json`。现区分 JSONDecodeError 和 FileNotFoundError：损坏内容抛 ValueError，缺失文件继续返回 `{}`。新增读/写负向固定数据测试2/2，确认异常后字节和 SHA-256 不变；既有 ID/upsert 3/3、隔离 smoke5/5、adapter80通过/25跳过、G2 15项和 PowerShell 14文件门通过。独立审计 PASS；审计本轮独立重跑 focused、ID、缺失文件回退、metadata 和 G1；G2/PS读取已存日志。损坏文件仍需人工恢复，S007整体PARTIAL。当前 G1树329文件/18瞬时，SHA `28c0b0c87062b04dc1c9bc8abbfc20ef30081709b5f00d59bbf2ab3d99398b57`；sourceTree `86d226b9954978157d46841982172adb26ea9d487798086d26d6b5153da8b35b`。


2026-10-19 S030 腐坏 JSON 增量：冻结基线确认 `meal_db.load_json()` 将 malformed profile/plans/logs JSON 返回 `{}`；profile/log 写操作会覆盖损坏内容，plans 被伪装为空并在新增时抛 KeyError。现 JSONDecodeError 显式 ValueError，缺失文件仍 `{}`；读写负向回归4/4确认 profile/plans/logs/recipes 原 bytes/SHA不变。独立审计 PASS，复跑腐坏JSON4/4、ID2/2、missing-file回退、metadata/G1；adapter80/25、营养烟测61/61、G2/PS通过现有当轮日志。坏文件仍不自动恢复，S030整体PARTIAL。当前G1树330文件/18瞬时 SHA `633f8e58d4e7e4aa709c43201e696a46e37e27811d8de6e0ced13f38e2d510c3`；sourceTree `1ed1a9b28c574c2a96ae254aa9383bfd1059606a8802610209fb30ddf7f546f8`。


2026-10-19 S045 trip ID 增量：冻结时钟下两次 `add_trip()` 生成相同时间戳 ID，按第二个 ID 更新错误命中第一条。改为 UUID4 字符串，固定时钟回归1/1通过；验证两条可独立读取且只更新目标记录。腐坏 JSON 3/3、adapter 80/25、metadata/G1/G2/PowerShell 门禁通过。独立只读审计复跑回归及元数据/G1并 PASS。旧重复 ID 不迁移；自然路由、运行时源加载、真实地点和实时价格未证，S045总体PARTIAL。


2026-10-19 S045 子记录 ID 增量：固定时钟下 `add_expense()` 与 `add_itinerary_item()` 均会为同一秒内记录分配重复时间戳 ID。改为 UUID4 字符串；扩展的固定时钟回归对 trip/expense/itinerary 三类测试3/3通过。独立只读审计确认包内调用方仅存储/返回 ID、无数值解析或格式依赖，PASS。adapter80/25、G2 15项、metadata330/331、G1 332文件/18瞬时无差异。旧子记录数字 ID 不迁移；S045仍PARTIAL。


2026-10-19 S018 SIP 输入边界增量：`calculate_regular_sip()` 将 bool 静默作为数字接收，`True` 可变成每月1元、1年或1%收益率。对 monthly_amount/years/annual_return 显式拒绝 bool；smart-sip 通过相同 helper 校验。bool四向量及原公式/CLI/溢出回归5/5，独立只读审计PASS。adapter80/25、G2 15项、metadata330/331、G1 332文件/18瞬时零差异。其他Python错误类型未覆盖；自然路由、源加载和实时AkShare未验证，S018仍PARTIAL。


2026-10-19 财务 helper bool 数值输入增量（fund-portfolio / wealth-allocation / venture-analysis）：直接 helper 调用曾把 bool 当成金额、期限、月份、初始用户数或估值评分并输出结果。四个 helper 显式拒绝对应 bool 参数；adapter 增加7个参数路径回归，81通过/25旧 fixture 跳过；独立只读审计PASS。G1 332文件/19瞬时、无差异；G2 15项；metadata330/331。CLI原有类型解析未变；自然路由/运行时源加载仍未证。
