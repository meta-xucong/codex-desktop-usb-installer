# U 盘 Codex Skills 与 Agents 执行清单

关联蓝图：USB-CAPABILITY-AUDIT-BLUEPRINT-20261018.md
目标源：codex-desktop-usb-installer/installer-pack；当前包版本：1.5.4；`pack.json` sourceCommit：3cfa9fca12e24b25be4b2f7f5b273e6b9fb22040（包内 build-input 元数据提交）；当前仓库 HEAD：b61eaf7（G0‴ 包指纹采样时 HEAD 为 896086e；后续审计更新了 installer-pack，当前指纹见 G0⁗/G1 post-S045 结果）。
约定：按编号顺序逐项执行。逐字探针和断言见同目录 `USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md`；不得把显式点名/贴入说明当成自然触发证据。静态检查与行为/路由状态分别记录；证据按蓝图固定 schema，不写密钥或私人材料。结构重跑：`python -B docs\audit-capability-pack.py installer-pack`。

## 阶段门

| Gate | 内容 | 状态 | 证据/备注 |
|---|---|---|---|
| G0（历史） | 旧基线冻结 | PARTIAL | 历史运行记录对应 HEAD/source tree `fa6670eabe4d5a921ba42e21e1a9a969687a098961e65eda3d8538114e5c800c`，当时观察到 22 个 installer-pack dirty paths、315 文件；旧聚合 hash 无法还原逐文件内容。见 `capability-audit-evidence-20261018/G0-source-baseline.txt`。 |
| G0′（历史） | 早期 clean baseline | SUPERSEDED | 旧 fingerprint `cbbc92b7a566c4abf0d105e21594bbd7a2bf886e31b09b9ad86d56c4e4b0775b`，对应旧状态，不代表当前 1.3.10 包内容。 |
| G0″（历史） | ae5bccb dirty tracked-only 样本 | SUPERSEDED | 2026-10-17T21:48:19Z：311 tracked paths，11 dirty，fingerprint `67907ad5b0e8306f626e40e02fcbe874af2cdde1d543770bd0fd7b245ff7382d`。同一时期全文件 `b7cebf...` 快照因 pycache 漂移标记为瞬时/不可用。见 G0″/G0‴ evidence。 |
| G0‴（历史） | 旧 tracked-source baseline | SUPERSEDED | 1.5.4；tracked-only SHA `c4e841199a918a6c62e549274a19f58b03f175601b76367ee51a10de0d112060`；311/311 tracked files；采样 HEAD `896086e`，当时 installer-pack clean。该快照早于后续 40 个包内工作树修改，不代表当前源。pack.json sourceCommit `3cfa9fca12e24b25be4b2f7f5b273e6b9fb22040` 是 build-input 元数据。 |
| G0⁗（上一轮） | 上一轮 USB 源快照 | SUPERSEDED | HEAD `b61eaf783db12d35c46405f4effbf4973b876d4b`；1.5.4；S052/S053/S054、S029、S007修复及包内回归后的包树325个文件，静态审计排除16个瞬时缓存；SHA `20c864b3584af33a131235a08ed87f9fd40419cf0ed6f5370d43a92643579027`。见 `G1-post-S017-muscle-coverage-20261019.json`。
| G0⁙（当前） | 当前 USB 源快照与本机同哈希代理映射 | PARTIAL | 最近代理映射比较时 `pack.json sourceTreeSha256=c928d3991293b7201114da9dbfddc7f1d106de600722ffbd501b53fcd6f3b037`；S045/S050 两条最小提示更新后当前树 SHA 为 `c354bbbc968ba6ed7b29c28badde01b9c066dc7db469581e7ddc9e8900d67e11`。最近逐目录 SHA 比较：56 Skills中16个至少有一份本机副本与USB源完全同哈希、40个在已检查副本中均不同、0缺失；7个Agent TOML中4个同哈希、3个不同。该比较只证明内容代理资格，不证明运行时选中或U盘安装生命周期。见 `G0-current-local-proxy-equivalence-refresh-20261019.json`。
| G1（当前源） | 56 个 Skill、7 个 Agent 的静态闭包 | PASS | 本次 S045/S050 文案更新后重算：442 manifest files/443 checksum rows；结构56 Skills/7 Agents，闭包、引用、哈希差异为0。`pack.json sourceTreeSha256=c354bbbc968ba6ed7b29c28badde01b9c066dc7db469581e7ddc9e8900d67e11`；审计树444文件（101个瞬时文件排除）。此前 G1/G2 结果见历史行/记录，本次未重跑兼容性套件。
| G2（历史 G0‴） | 离线兼容性、media MCP与adapter契约测试 | SUPERSEDED | 旧数据只适用于旧指纹 `c4e841…`。当前指纹上的完整复跑见 `capability-audit-evidence-20261018/G2-current-source-rerun-61c-20261018.md`。 |
| G2（当前源） | Codex兼容性、media MCP与adapter契约测试 | PASS | 最新兼容性审计15项`ok=true`、无warning/error；PowerShell解析、media MCP64/64、adapter84通过/25个旧用例按预期跳过、metadata闭包与diff-check通过。见 `G2-post-S052-fetchers-runtime-20261019.json`、`S052-S053-final-powershell-parse-20261019.log`、`media-mcp-post-S052-fetchers-runtime-20261019.log`、`adapter-contracts-post-S052-fetchers-runtime-retry-20261019.log`、`S052-S053-final-metadata-check-20261019.log`。
| G3 | 56 个 Skill 普通任务可用性冒烟 | INSUFFICIENT_EVIDENCE | 按简化可用性口径复核：44 PASS、2 PARTIAL、0 FAIL、10 NOT_TESTABLE。PASS表示已有普通任务/当前源功能结果有用且无明显错误，不要求来源路径归因；PARTIAL表示要求的答案/产物未交付或存在明显问题；NOT_TESTABLE表示账号/服务等前置未配置或用户明确排除。最新缺口与证据见 `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md`。 |
| G4（平台 smoke） | 7 个 Agent 的角色与委派冒烟 | PASS（平台角色基本可用） | A01–A07 均有有用的平台角色任务输出；按简化口径基本角色可用性PASS。此结论不证明USB TOML运行时加载或安装生命周期；A02隔离委派缺少bearer的限制单独保留。详见 `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md`。 |
| G5 | 独立只读审计和问题复测 | PARTIAL | S031/S035/S036/S050此前源修复及 focused 检查均有独立复核；本轮 S045/S050 两处最小文案改动经常驻纠察 agent 只读复核通过。当前 G1 56/7、442 manifest/443 checksum；既有兼容性15项、media64/64、adapter84/25等结果未因纯文案更新重跑。证据账本本次复核1810条唯一记录，路径/哈希缺失与过期0；六项服务/运行环境结果缺口、自然路由及 USB Agent lifecycle仍未验证。 |

## 当前源复核（2026-10-19，替代 G0‴ 的 G1/G2 结论）

目标仍是 U 盘开发源 `installer-pack`，不是本机安装目录。当前包版本1.5.4，`pack.json sourceCommit` 为 build-input 元数据。此前源快照已由后续修复取代；本次 S045/S050 文案更新后静态闭包为442 manifest files/443 checksum rows，`sourceTreeSha256=c354bbbc968ba6ed7b29c28badde01b9c066dc7db469581e7ddc9e8900d67e11`；结构复核无差异。此次未重跑 G2，因为只有 Skill Markdown 提示变更。

| 检查 | 当前结果 | 证据 |
|---|---|---|
| G1 静态闭包 | PASS，exit 0；56 Skills、7 Agents、49/7 默认启用；零闭包差异；当前树 SHA `53a5f414…` | `S029-G1-post-market-sizing-fix-20261019.json` |
| Codex 兼容性 | PASS，15 项 checks，exit 0；Codex CLI 0.157.1；无 warning/error | `S029-compatibility-20261019.log` |
| media MCP | PASS，64/64 tests | `G2-media-mcp-rerun-20261019.log` |
| adapter contracts | PASS：76 项中 51 项通过、25 项跳过（旧媒体 fixture，由专用离线套件替代）；TEMP/TMP 指向工作区目录 | `G2-adapter-contracts-workspace-temp-rerun-20261019.log` |
| 完整性元数据 | PASS；320 manifest files / 321 checksums | `S052-S053-source-fix-metadata-check-20261019.log` |

历史增量快照（已被当前复核取代）：S023 Skill 文档修正后曾记录树 SHA-256 `85fd3c6aa5ada850412d4a21d2cea685610b41580ea6887490181ae1b57a937c` 和 builder 309/310 检查；该值不是当前 package tree。此前 `G1-current-package-static-rerun-20261019.json` 也属于较早快照（315文件/旧树哈希），已被 `S029-G1-post-market-sizing-fix-20261019.json` 及 `G1-rerun-post-provision-and-weather-20261019.txt` 取代；当前树以 `53a5f414…` 为准。

MiniMax 官方协议静态核对和“保留但硬禁用”的处理见 `S032-minimax-official-contract-review-20261018.md`。没有官方账号，所以 API 在线调用、计费和生成效果仍未验证。G3 全量 56 Skills 的自然语言路由/源加载与 G4 7 Agents 的源加载证据仍未完成；总审计保持未完成状态。

## G2 历史 G0‴ 精确命令与输出映射

当前 G0‴ G1/G2 运行于仓库根 `D:\AI\AGENT_U盘\codex-desktop-usb-installer`；`TEMP`/`TMP` 指向专用临时目录，设置 `PYTHONDONTWRITEBYTECODE=1`，命令使用 `python -B`。每项完成后复算 tracked-only fingerprint。

1. `python -B docs\audit-capability-pack.py installer-pack` → exit 1；结构56 Skills/7 Agents通过，5个未跟踪 pycache 未列入file-manifest/checksums；hash mismatches=0。日志 `G1-static.log`。
2. `python -B installer-pack\audit-codex-compatibility.py --root installer-pack --json` → exit 0；14 checks，ok=true，errors=[]。日志 `G2-compatibility.log`。
3. `python -B installer-pack\test-media-generation-mcp.py` → exit 0；54 tests OK。日志 `G2-media-mcp.log`。
4. `python -B installer-pack\test-adapter-contracts.py -q` → exit 0；76 tests，25 skipped，0 failures。日志 `G2-adapter-contracts.log`。

以下命令和输出映射仅描述当时的 G0‴ 历史运行，不代表当前工作树的逐项验证。其 fingerprint 在该轮 G1/G2 运行前后保持 `c4e841199a918a6c62e549274a19f58b03f175601b76367ee51a10de0d112060`。当前工作树对应的复跑记录在 `current-source-rerun-20261018.md` 和本清单 G2 当前结果表。

矩阵覆盖检查精确命令：`python -B docs\check-audit-vectors.py installer-pack docs\USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md` → exit 0；56 Skill IDs 和 7 Agent names 全覆盖、唯一，无漏项/多项。原始输出 `capability-audit-evidence-20261018/vector-matrix-check.txt`。

## 固定证据记录格式

每个项目记录：`case_id | target_source | source_commit | source_tree_sha256 | prompt_id | trigger_mode | observed_route | input_fixture | expected_assertions | actual_summary | dependency_state | side_effect_check | status | evidence_path | evidence_sha256 | run_at_utc | executor`。自然触发、显式调用、静态闭包分开记。自然路由/来源不可观察时填 `unavailable` 并将触发结论记 `INSUFFICIENT_EVIDENCE`；不能把行为质量状态覆盖成触发 PASS。

## 顺序执行：Skills

| ID | Skill | 状态类别 | 默认启用 | 最小验证探针 | 通过要点 | 结果 | 证据 |
|---|---|---|---:|---|---|---|---|
| S001 | academic-research | core-ready | 是 | 城市热岛与街道绿化近五年文献检索 | 自然触发源版 academic-research + multi-search-engine；检索并标注 Crossref 核验失败，不伪造中文结果 | PASS（本机运行代理） | `capability-audit-evidence-20261018/s001-isolated-source-runtime-20261018.md` |
| S002 | canvas | unsupported | 是 | 四节点订单流本地 HTML fallback；静态结构/JS语法检查通过，实际 flow.js 以 Node VM + DOM stub 做事件/状态 smoke 54 项通过 | 浏览器 loopback 预览超时、file URL 被策略拒绝；修复 openai.yaml 默认提示：先探测后端/权限，外部失败才本地 HTML，focused3/3并经独立复核；真实浏览器、远程Canvas、自然路由仍未证；DOM stub 不代替浏览器行为 | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S002-current-usb-local-html-artifact-20261019.md`; `S002-browser-preview-attempts-20261019.md`; `independent-audit-S002-local-html-20261019.md`; `S002-flow-behavior-followup-20261019.md`; `S002-flow-dom-mock-smoke-20261019.cjs`; `.log`; `S002-flow-dom-mock-attempt-history-20261019.md`; `S002-canvas-default-route-contract-remediation-20261019.md`; `final-verification/S002-focused-post-review-20261019.log`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S003 | china-stock-analysis | auto-installable-runtime | 是 | 600519 行情时点核验；另修data_fetcher safe_float对10**400可选市值泄漏OverflowError：fake provider基线复现，现转None保留其余记录；focused3/3、S003全套42/42、独立PASS，包门禁通过。真实AkShare、自然路由仍未验证。| 当前 USB 实时行情适配器离线 smoke 5/5，另外五个脚本合成输入 smoke 11/11通过并经独立复核；data_fetcher financial cache horizon 回归5/5；technical flat-market回归及minute-volume分桶31/31；stock_screener有限JSON focused4/4、Skill套件35/35，独立审计PASS；technical_analysis非有限K线 focused4/4、Skill全套39/39，独立审计PASS。可选展示 NaN/±Infinity→null、非有限排序值剔除；close/high/low/volume NaN/±Infinity按字段和行号拒绝，CLI严格JSON且失败不改输出。G1/G2/adapter及元数据闭包通过。实时数据/上游字段频率、自然路由和完整工作流未测，整体PARTIAL | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S003-realtime-quote-offline-smoke-20261019.md`; `S003-financial-valuation-source-smoke-20261019.md`; `S003-stock-screener-nan-remediation-20261019.md`; `independent-audit-S003-stock-screener-nan-20261019.md`; `S003-stock-screener-nan-baseline-20261019.log`; `S003-stock-screener-suite-20261019.log`; `S003-data-fetcher-overflow-remediation-20261019.md`; `S003-data-fetcher-overflow-baseline-20261019.py/.log`; `S003-data-fetcher-overflow-focused-20261019.log`; `S003-data-fetcher-overflow-suite-20261019.log`; `independent-audit-S003-data-fetcher-overflow-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S004 | codex-skin-design | core-ready | 是 | 记账 App 深色主题自然提示；隔离 harness 源版本复测 | USB 源 SKILL.md 哈希一致且 JSONL 观察到读取；交付 tokens、对比度、字体/间距、状态与代表页面；概念边界正确 | PASS（本机运行代理） | `capability-audit-evidence-20261018/s004-isolated-source-runtime-20261018.md`；原输出及对比度复算另见原 smoke 记录 |
| S005 | competitor-intelligence | core-ready | 是 | Notion、Confluence、语雀公开信息与定价比较 | 读取源版竞品与多源搜索 Skill；官方页面查证、日期/未知项和 20 人费用口径清楚 | PASS（本机运行代理） | `capability-audit-evidence-20261018/s002-s005-source-runtime-summary-20261018.md` |
| S006 | conversion-optimizer | core-ready | 是 | 落地页转化诊断/改写与普通邮件负向边界 | 当前 USB 源 SHA `3f394f7a050085c13870a6598dd576d282483d50090b15f33017b8b2786e20e8`；2/2 文件哈希匹配。冻结正负提示逐字完成且 exit 0；正向读取项目副本，只据原文提出一项阻力并改标题/价值点/CTA，明确效果待测，无编造数据/退款/稀缺/发布；负向仅索要邮件正文且未读目标 Skill。当前源合同8/8、独立审计PASS。 | PASS（项目级当前源代理） | `capability-audit-evidence-20261018/S006-current-source-smoke-20261019.md`; `S006-current-source-smoke-independent-review-20261019.md`; raw prompts/traces/answers/stderr/exitcode; `S006-current-source-contract-20261019.txt` |
| S007 | daily-reflection | core-ready | 是 | 逐字正/负探针；current USB `reflection_db.py` isolated synthetic read/write smoke | 对话代理仅提出首个 journaling 问题、不收集/保存档案；当前 USB 数据 helper 隔离烟测5/5及多记录同秒 ID 冲突修复通过；七类记录在冻结时钟下各写两条，UUID唯一，目标更新/习惯统计关联和两类 upsert 正确；本轮发现损坏 profile/reflections JSON 会被静默覆盖，已修复为 ValueError fail-closed，读/写负向回归2/2并验证字节/hash不变；现有 ID回归3/3、隔离源 smoke5/5、adapter80/25、G1/G2/PowerShell门通过。临时合成数据；S007损坏JSON独立只读审计PASS。对话端到端、自然路由及运行时源加载未验证 | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/s006-s012-cli-smoke-summary-20261018.md`; `S007-reflection-db-isolated-smoke-20261019.md`; `.py`; `.log`; `S007-record-id-remediation-20261019.md`; `S007-corrupt-json-remediation-20261019.md`; `S007-corrupt-json-baseline-20261019.py/.log`; `S007-corrupt-json-regression-20261019.log`; `S007-second-boundary-collision-smoke-20261019.py/.log`; `S007-second-boundary-unique-id-rerun-20261019.py/.log`; `S007-id-uniqueness-unittest-20261019.log`; `independent-audit-S007-record-id-20261019.md`; `independent-audit-S007-corrupt-json-20261019.md`; `test_reflection_db_id_uniqueness.py`; G1/compatibility/PowerShell gates; latest `G1-current-after-S007-20261019.json` / `G2-compatibility-current-after-S007-20261019.json`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S008 | data-assistant | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S008 显式正向/相邻负向探针 | 显式调用读取源匹配 Skill，计算正确；自然调用因包内显式调用策略未路由到目标 Skill，单独保留为边界证据 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/s006-s012-cli-smoke-summary-20261018.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S009 | docx-butler | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S009 逐字正/负探针 | 当前USB源新增输出路径/不覆盖规则；source-pinned合成DOCX结构 smoke 创建了有效文件并验证A/B/C标题、列表和一页摘要边界。源生成DOCX结构验证通过；本轮打包renderer因soffice.exe缺失失败；同一安全上下文的一次当前 Word COM 复测在创建应用前再次以0x80070005 E_ACCESSDENIED拒绝，源DOCX哈希不变且未生成PDF/页图；视觉QA、冻结负向提示和自然路由仍未验证，整体仍PARTIAL | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S009-docx-butler-source-remediation-20261019.md`; `S009-source-pinned-docx-smoke-20261019.py`; `.log`; `S009-source-pinned-artifact-20261019.docx`; `S009-docx-render-attempt-20261019.log`; `S009-test-attempt-history-20261019.md`; `G1-post-S009-source-patch-20261019.json`; `S009-current-render-attempt-20261019.md`; `S009-render-attempt-current-20261019.log`; `S009-native-word-current-recheck-20261019.ps1`; `.json`; `.md`; previous runtime summary ; `docs/capability-audit-evidence-20261018/S009-ordinary-positive-current-source-smoke-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S010 | efficiency-toolkit | core-ready | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S010 逐字正/负探针 | 读取效率与安全说明，只输出计划，不移动或删除文件 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S010-efficiency-toolkit-file-safety-remediation-20261019.md`; `S010-password-guidance-remediation-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S011 | email-expert | core-ready | 是 | 逐字正/负探针：供应商延迟邮件；会议纪要三条摘要（不写邮件） | 冻结正向自然路由读取与当前USB源逐文件哈希一致的项目 `.agents/skills/email-expert/SKILL.md`；草稿有主题/称呼/正文/明确请求/落款，未知信息用占位符，不代发。冻结负向请求因无会议正文而请用户补充，未读 email-expert、未写邮件。未连接邮箱或发送。仅证明项目级精确源内容代理 | PASS（项目级精确源内容代理） | `capability-audit-evidence-20261018/S011-email-expert-natural-route-20261019.md`; `S011-natural-route-positive-frozen-codex-exec-20261019.jsonl`; `S011-natural-route-positive-frozen-codex-exec-20261019.answer.md`; `S011-natural-route-positive-codex-exec-20261019.jsonl` (近似提示，仅补充路由观察); `S011-natural-route-negative-codex-exec-20261019.jsonl`; `S011-natural-route-negative-codex-exec-20261019.answer.md`; `s006-s012-cli-smoke-summary-20261018.md` |
| S012 | excel-xlsx | auto-installable-runtime | 是 | 用5笔虚构小额支出生成明细与分类汇总工作簿 | 本次单条正向尝试未读取目标 SKILL.md，也未调用文件/表格工具或生成 xlsx；trace只显示受只读执行策略阻止。exit 0仅表示对话回合结束。此为环境未验证，不是Skill失败；S012保持PARTIAL | PARTIAL | `capability-audit-evidence-20261018/S012-positive-attempt-environment-block-20261019.md`; 独立范围复核及原始prompt/trace/answer/exitcode/source-copy manifest |
| S013 | feishu-doc | guided-config | 否 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S013 逐字正/负探针 | 源静态契约 7/7：四个精确只读工具映射、docx 正文只读、fail-closed、锁定 allowlist 与审批模板通过；本任务无 Feishu MCP 工具/账号，实际读取与自然路由仍 NOT_TESTABLE | NOT_TESTABLE | `capability-audit-evidence-20261018/S013-S016-feishu-source-contracts-20261019.md`; `.log`; `G1-post-S013-S016-contracts-20261019.json`; `G2-post-S013-S016-contracts-20261019.json`; `installer-pack/tests/test_feishu_skill_contracts.py` |
| S014 | feishu-drive | guided-config | 否 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S014 逐字正/负探针 | 源静态契约通过：2 只读 + 4 写入映射逐项闭合；用户明确授权/唯一目标、删除和歧义保护均有约束；MCP/账号缺失，实际列云盘、变更和自然路由未证 | NOT_TESTABLE | `capability-audit-evidence-20261018/S013-S016-feishu-source-contracts-20261019.md`; `.log`; `G1-post-S013-S016-contracts-20261019.json`; `G2-post-S013-S016-contracts-20261019.json`; `installer-pack/tests/test_feishu_skill_contracts.py` |
| S015 | feishu-perm | guided-config | 否 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S015 逐字正/负探针 | 源静态契约通过：3 只读 + 4 写入映射逐项闭合；目标/成员/权限授权、不扩大范围及删除/转移所有者高风险保护通过；无账号，实际权限检查和自然路由未证 | NOT_TESTABLE | `capability-audit-evidence-20261018/S013-S016-feishu-source-contracts-20261019.md`; `.log`; `G1-post-S013-S016-contracts-20261019.json`; `G2-post-S013-S016-contracts-20261019.json`; `installer-pack/tests/test_feishu_skill_contracts.py` |
| S016 | feishu-wiki | guided-config | 否 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S016 逐字正/负探针 | 源静态契约通过：4 只读 + 4 写入映射逐项闭合；正文读取委派 feishu-doc、正文编辑限制及缺工具 fail-closed 通过；无账号，实际空间导航和自然路由未证 | NOT_TESTABLE | `capability-audit-evidence-20261018/S013-S016-feishu-source-contracts-20261019.md`; `.log`; `G1-post-S013-S016-contracts-20261019.json`; `G2-post-S013-S016-contracts-20261019.json`; `installer-pack/tests/test_feishu_skill_contracts.py` |
| S017 | fitness-coach | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S017 逐字正/负探针；Program Generator输出失败基线复现：缺失目录退出1但计划入库，重试重复；修复为先stage/fsync、后DB、原子发布，发布失败恢复计划库；focused3/3、全套37/37，独立审计PASS；G1/G2/adapter/metadata通过。进程强杀、并发DB写者、rollback失败注入未测。| 当前 USB 源生成器 `avoid_exercises` 边界 13/13通过；隔离 fitness_db helper 4/4通过且经独立复核；新增 biceps/triceps 嵌套库映射与动作category修正，覆盖 Bro Split 手臂日、upper/lower推拉日、器械过滤、全动作避开及 16/16 包内回归通过；新发现 fitness_db 秒级 ID 碰撞已修复，固定时钟测试2/2、原生成器13/13、肌群覆盖3/3、adapter 80/25通过，独立只读审计 PASS（2/2固定时钟回归、package metadata和G1复核；G2/PS通过现存日志）。本轮又修复损坏 profile/programs/workouts JSON 静默降空/覆盖风险，fail-closed读写与missing-file回退测试4/4；损坏JSON保护独立只读审计PASS。历史重复ID不迁移，腐坏数据不自动恢复。自然路由、USB源运行时来源和训练科学性/个体适宜性仍未验证 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S017-avoid-exercises-remediation-20261019.md`; `S017-avoid-exercises-regressions-20261019.log`; `S017-fitness-db-source-smoke-20261019.md`; `.py`; `.log`; `independent-audit-S017-fitness-db-20261019.md`; `independent-audit-S017-fitness-db-id-collision-20261019.md`; `S017-fitness-db-id-collision-remediation-20261019.md`; `S017-corrupt-json-remediation-20261019.md`; `S017-corrupt-json-baseline-20261019.py/.log`; `S017-corrupt-json-regression-20261019.log`; `independent-audit-S017-corrupt-json-20261019.md`; 旧代理证据; `S017-program-output-failure-remediation-20261019.md`; `independent-audit-S017-program-output-failure-20261019.md`; `S017-program-output-failure-baseline-20261019.py/.log`; `S017-program-output-failure-focused-20261019.log`; `S017-program-output-failure-suite-20261019.log`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S018 | fund-portfolio | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S018 逐字正/负探针 | 四个 helper 脚本 source smoke 10/10 通过；SIP 极值回归由4/4扩展到5/5，新增拒绝 monthly/years/annual_return 的 bool 向量，保留独立年金公式、正常CLI JSON和溢出失败路径；adapter tests 81 passed/25 skipped（含跨Skill bool边界回归），sourceTree metadata 与 G1 闭包通过。bool输入缺陷经独立复核PASS。负向对话、自然路由/源加载和实时 AkShare 上游仍未验证 | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S018-sip-overflow-remediation-20261019.md`; `S018-sip-bool-input-remediation-20261019.md`; `S018-sip-bool-input-baseline-20261019.py/.log`; `installer-pack/skills/fund-portfolio/scripts/test_sip_calculator_overflow.py`; `S018-sip-bool-input-regression-20261019.log`; `S018-sip-bool-adapter-regression-20261019.log`; `S018-sip-bool-G1-20261019.json`; `S018-sip-bool-G2-20261019.json`; `S018-sip-bool-metadata-20261019.log`; `independent-audit-S018-sip-bool-input-20261019.md`; `S004-financial-bool-input-remediation-20261019.md`; `independent-audit-S004-financial-bool-input-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S019 | grok-video-prompt-compiler | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S019 逐字正/负探针 | 当前 USB 源 prompt compiler/reference helper 本轮独立复跑34/34通过（29项编译 + 5项上传mock），workspace TEMP/TMP隔离和清理保护通过；无真实网络/上传/生成。自然路由、USB Skill加载及真实上传/生成未验证；2026-10-19单次普通提示任务读取当前副本并输出可用提示，但compiler脚本受执行策略拦截，手工整理；本次attempt PARTIAL，经纠察复核未重试 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S019-grok-compiler-source-tests-20261019.md`; `S019-grok-compiler-workspace-temp-rerun-20261019.log`; `independent-audit-S019-grok-compiler-20261019.md`; historical: `S019-current-usb-source-offline-regressions-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S020 | growth-analyst | core-ready | 是 | 逐字正/负探针：两段漏斗数据并设计实验；品牌故事不做指标复盘 | 当前USB源项目副本自然路由读取成功，树SHA与源一致。正向正确计算注册率10%→8%、注册激活率30%→25%并用百分点表达，因果仅作假设，实验含主指标和护栏且说明样本能力边界；负向未进入指标分析、未读取目标Skill。仅合成数据，无业务访问/外联。旧source-read失败由本次新harness补齐，旧记录保留。限项目级精确源内容代理，不证明U盘全局安装生命周期 | PASS（项目级精确源内容代理） | `capability-audit-evidence-20261018/S020-growth-analyst-natural-route-20261019.md`; `S020-natural-route-positive-codex-exec-20261019.jsonl`; `S020-natural-route-positive-codex-exec-20261019.answer.md`; `S020-natural-route-negative-codex-exec-20261019.jsonl`; `S020-natural-route-negative-codex-exec-20261019.answer.md`; prior `s020-current-ae5bccb-runtime-summary-20261018.md` |
| S021 | healthcheck | unsupported | 是 | 当前 USB 源精确正负向量 source-pinned 文本代理通过 | 管理员后端/系统探针未配置，且用户已排除安全测试；不以缺少该后端判作功能失败，按简化口径为 NOT_TESTABLE | NOT_TESTABLE | `capability-audit-evidence-20261018/S021-current-usb-source-proxy-20261019.md`; `independent-audit-S021-S022-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S022 | hyperframes-video | auto-installable-runtime | 是 | 当前源只读检查 CLI/Chrome/FFmpeg 和运行状态；随 S028 的隔离 provision 运行核对 HyperFrames 安装前置 | 本机已有 Chrome 与 Node/npm，FFmpeg 由包内 provisioner 在隔离 profile 安装并健康；强制 npm 离线时因缺少 `zod@3.25.76` 缓存报 `ENOTCACHED`；在线只读 `npm view` 又被配置代理 `127.0.0.1:7890` 以 `EACCES` 拒绝连接，未绕过代理。`browserHealth=unknown`，未安装/渲染；本轮新增版本门：元数据与CLI必须精确匹配锁定0.8.134才可标健康，focused8场景/PS5.1解析及独立复核通过。锁定包渲染、Chrome真实可用、自然路由/视频产物未验证 | PARTIAL | `capability-audit-evidence-20261018/S022-current-usb-runtime-readonly-probe-20261019.md`; `capability-audit-evidence-20261018/S022-hyperframes-prerequisite-snapshot-20261019.json`; `capability-audit-evidence-20261018/S022-npm-online-probe-20261019.md`; `independent-audit-S021-S022-20261019.md`; `S022-hyperframes-version-health-remediation-20261019.md`; `final-verification/S022-focused-PS51-post-review-20261019.log`; 历史本机运行摘要 |
| S023 | image-2 | auto-installable-runtime | 是 | 当前媒体 MCP `media_capabilities` 显示 Image-2 已配置；固定正向离线 `media_prepare` 成功；source-pinned 正负边界代理通过 | 用户已追查确认本次 HTTP 502 根因在上游 Sub2API，并要求 media MCP 标记 PASS；G2 中 media MCP 契约测试已 PASS。此次生图仍无产物/task_id，因此仅端到端生图任务保留 PARTIAL，不归因为 MCP 缺陷；不重发、不再处理 MCP。MiniMax adapter unavailable 不归 Image-2。自然路由、USB Skill 加载、实际图像质量仍未验证；历史错路由 FAIL 保留 | PARTIAL（端到端结果） | `capability-audit-evidence-20261018/S023-current-image2-offline-prepare-20261019.md`; `independent-audit-S023-live-image2-prepare-20261019.md`; `media-mcp-1.3.11-live-image-video-smoke-20261019.md`; `docs/capability-audit-evidence-20261018/S023-image-2-ordinary-positive-20261019.md` |
| S024 | image-prompt-generator | core-ready | 是 | 把“如何识别钓鱼邮件”规划成小红书竖版信息图；只交付结构化提示词，不生成图片 | 当前 USB 源副本正向任务完成：trace 读取项目 harness 中与 USB 源 SHA 一致的 SKILL.md，输出可用标题、分区、四项内容、行动条、色彩与风格提示词，exit 0；独立范围复核 PASS。仅证明这一个普通任务的项目级可用性，不据此声称全局安装或自动触发 | PASS（当前源项目副本正向可用） | `capability-audit-evidence-20261018/S024-current-source-positive-smoke-20261019.md`; 独立复核及原始 prompt/trace/answer/exitcode/source-copy manifest |
| S025 | industry-analysis | core-ready | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S025 逐字正/负探针 | 产业链、政策/技术变量、风险与事实来源日期；不作无据投资预测 | PASS（本机运行代理） | `capability-audit-evidence-20261018/s025-current-ae5bccb-runtime-summary-20261018.md`; `.../s025-current-ae5bccb-20261018.jsonl` |
| S026 | local-video-editing | auto-installable-runtime | 是 | 正向包内 FFmpeg/FFprobe synthetic trim/probe 通过；当前 USB archive/manifest/wrapper 相对路径静态契约及独立复核通过。负向只给 FFmpeg 命令草案 | 旧测试通过解压重建布局验证 wrapper 逻辑；本轮再用真实 provisioner 生成的隔离 profile `ffmpeg.cmd`/`ffprobe.cmd` 完成 6s 合成 A/V 裁切，路径含中文和空格，18项显式断言通过，输出3.0s、音视频差0s、源哈希未变。2026-10-19单次普通本地剪辑任务读取了当前USB Skill，但要求的用户级FFmpeg/FFprobe包装器及系统命令均缺失，按Skill说明停止且未产生产物；纠察复核为PARTIAL/依赖未配置，不重试。自然Skill自动路由和USB安装生命周期仍未验证 | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S026-provisioned-runtime-wrapper-smoke-20261019.md`; `S026-provisioned-wrapper-smoke-20261019.py`; `.json`; `.log`; `S026-invoke-provisioned-wrapper-20261019.ps1`; `independent-audit-S026-wrapper-runtime-smoke-20261019.md`; `S026-installer-wrapper-runtime-smoke-20261019.py`; `.log`; prior bundled executable smoke; wrapper static audit; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S027 | macro-research | auto-installable-runtime | 是 | 逐字自然触发，核验央行最近公开措施、官方原文/发布日期/统计时点，分事实与情景；不交易 | 当前 USB 三个源脚本合成输入离线烟测7/7通过：周期四象限、规则校验与显式配置总和，政策证据/假设 fail-closed，市场情绪显式权重/边界校验。未测实时宏观数据、官方央行来源检索、自然路由或 USB Skill 加载；2026-10-19单次当前源自然任务因CLI在模型回合前无法打开state_5.sqlite（os error 5）而未执行，记PARTIAL/环境未验证，纠察复核不重试；历史行为引用一手原文不足仍保留 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S027-macro-source-offline-smoke-20261019.md`; `.py`; `.log`; `s027-current-g0triple-runtime-summary-20261018.md`; historical JSONL; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S028 | manim-animation | auto-installable-runtime | 是 | 冻结正向任务为勾股定理故事动画和真实预览；负向只解释定理且不写/渲染。Manim 0.21 provisioned profile 上直接渲染了 3-4-5 勾股面积演示并做关键帧目检 | 正向固定向量的直接 Manim 渲染通过：面积方块9/16/25、公式与数值推导可读；H.264 854×480、131帧、8.733008秒，exit 0。显式调用策略保持 `allow_implicit_invocation:false`，源路由合同测试2/2通过；自然触发不作为已验证能力。中文字体缺字初稿已改为英文/ASCII并重渲染。负向“不渲染”向量经显式 Skill 调用输出纯文本并通过行为断言（CLI退出码未捕获）；自然Skill路由、USB安装后的运行时加载、内置Python bootstrap、LaTeX/HyperFrames仍未验证，整体PARTIAL | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S028-manim-runtime-smoke-20261019.md`; `S028-pythagoras-vector-20261019.py`; `S028-pythagoras-render-20261019.log`; `S028-pythagoras-video-metadata-20261019.json`; `S028-pythagoras-frames-20261019/`; provisioner/runtime evidence; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S029 | market-sizing-analysis | core-ready | 是 | 普通任务：按给定的虚构假设估算自由职业设计师协作工具 TAM/SAM/3年SOM。 | 当前 USB 源副本在单次 Codex 任务中被读取并完成；计算结果和假设说明可用，纠察复核 PASS。 | PASS | `capability-audit-evidence-20261018/S029-ordinary-positive-current-source-smoke-20261019.md` 及同名前缀的 prompt/trace/answer/exitcode/stderr/source-sha256 证据文件 |
| S030 | meal-planner | auto-installable-runtime | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S030 逐字正/负探针；最新修复gender:null导致BMR泄漏AttributeError：只接受规范化male/female，不静默选公式；基线复现；focused3/3、全套25/25，独立PASS；G1/G2/adapter/metadata通过。仅合成档案，饮食/医疗准确性和路由/runtime未验证。| 旧运行的技能路由证据保留。过敏修复与营养聚合 smoke 61/61通过；本轮冻结同秒时钟复现两计划 ID 碰撞、更新第二个 ID 改到首条记录，餐食 log_id 也碰撞。计划/日志改 UUID4；固定时钟隔离回归2/2，确认两计划各自查询、仅更新第二条，以及两条日志均保留。S030 同秒 ID 缺陷独立只读审计 PASS；本轮另修复损坏 profile/plans/logs/recipes JSON 静默降空/覆盖风险，fail-closed读写回归4/4、文件字节/hash不变；损坏JSON修复独立只读审计PASS；无效天数写入缺陷修复后全套19/19并独立PASS；新增报告输出失败事务边界修复：复现输出父目录失败后遗留计划并可重试重复（1→2），22/22、独立审计及独立全套复跑PASS，包含最终发布/补偿失败路径。跨文件强制崩溃窗口、自然路由、营养来源/临床准确度与真实档案流仍未验证 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S030-meal-db-id-collision-remediation-20261019.md`; `S030-plan-id-collision-baseline-20261019.py/.log`; `S030-nutrition-arithmetic-smoke-20261019.py/.log`; `S030-meal-output-failure-remediation-20261019.md`; `independent-audit-S030-meal-output-failure-20261019.md`; `S030-meal-output-write-failure-baseline-20261019.py/.log`; `S030-meal-output-failure-tests-20261019.log`; `S030-null-gender-bmr-remediation-20261019.md`; `S030-null-gender-bmr-baseline-20261019.py/.log`; `S030-null-gender-bmr-focused-20261019.log`; `S030-null-gender-bmr-suite-20261019.log`; `independent-audit-S030-null-gender-bmr-20261019.md` 2026-10-19 普通正向菜单任务由生活官角色返回有用三餐与份量；纠察确认范围合规，但无USB SKILL.md运行时读取trace，因此仅记PARTIAL/来源未证，不重跑。证据 `capability-audit-evidence-20261018/S030-ordinary-positive-current-source-smoke-20261019.md`。; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S031 | meeting-secretary | core-ready | 是 | 合成会议纪要/行动项样例与普通概念问答负向边界 | 当前 USB 源 SKILL.md SHA `c3d0e65ebecb09cdffb56dffdc9361be9183e354a4f54a7430cadc3499d36688`；完整源目录 3/3 文件哈希匹配到项目 harness。正向以冻结向量句子开头并追加已批准虚构速记，trace 读取项目副本；负向逐字冻结且未读取目标 Skill；两轮 exit 0。4/4 focused contract、G1 metadata closure 通过；独立审计 PASS。范围为当前源内容项目级代理，不证明全局安装生命周期。 | PASS（当前源项目级自然路由代理） | `capability-audit-evidence-20261018/S031-current-source-smoke-20261019.md`; `S031-current-source-smoke-independent-review-20261019.md`; 正负向量 raw prompts/traces/answers/stderr/exitcode；`S031-current-source-contract-20261019.txt`; `S031-current-G1-check-20261019.txt`; `capability-audit-evidence-20261018/S031-current-static-pack-audit-20261019.json` |
| S032 | minimax-video-gen | guided-config | 否 | 明确检查 MiniMax 视频能力并准备请求，不提交；不可用时停止，不切其他模型 | 当前 `media_capabilities` 实时结果为 `available=false/configured=false`，原因 `PROVIDER_ADAPTER_UNAVAILABLE`；没有选替代模型或提交。真实 MiniMax API/账号与自然路由未测试 | NOT_TESTABLE（上游配置缺失） |`capability-audit-evidence-20261018/S032-minimax-live-capability-probe-20261019.md`; official contract review and offline fail-closed regressions; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S033 | multi-search-engine | core-ready | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S033 逐字正/负探针 | 正向执行两次免费只读搜索并引用STA公告、EY/HKTDC交叉核对；区分政策事实与现金流推断；负向只摘要虚构文本且未搜索 | PASS（本机运行代理） | `capability-audit-evidence-20261018/s033-current-g0triple-runtime-summary-20261018.md`; 配套正负JSONL/stderr及exit文件；`s033-runner-wrapper-failure-{1,2}-20261018.txt`（工具历史转录） |
| S034 | node-connect | unsupported | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S034 显式正向/相邻负向探针 | USB 源提示词受控代理有结果；实际设备配对后端未配置，因此不能实测该普通任务。按简化口径记 NOT_TESTABLE，不把缺少配对后端判为 Skill 功能失败 | NOT_TESTABLE | `capability-audit-evidence-20261018/S034-explicit-source-pinned-proxy-20261019.md`; `s034-current-g0triple-runtime-summary-20261018.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S035 | offer-designer | core-ready | 是 | 自由职业者收款/催款 SaaS 的定位、产品档位、定价、销售页结构 | 当前 USB 源 `SKILL.md` SHA `81c249ddf6eebe07cf14c1a3e935e01073758c17fac0f1ed887c259cc2916f61`；3/3 项目副本文件 hash 匹配。冻结正向提示显式调用 `$offer-designer`，trace 读取项目源副本、exit 0；回答覆盖定位/档位/定价逻辑/销售页结构，独立审阅认为可用。 | PASS（项目级显式源加载与正向输出） | `capability-audit-evidence-20261018/S035-current-source-positive-smoke-20261019.md`; `S035-current-source-positive-independent-review-20261019.md`; `S035-positive-prompt-20261019.txt`; `S035-positive-codex-exec-20261019.jsonl`; `S035-positive-answer-20261019.md`; `S035-source-copy-sha256-20261019.json` |
| S036 | painpoint-analyzer | core-ready | 是 | 需求确认：自由职业者协作工具；先提问，不给方案 | 当前 USB 源副本正向任务已完成：trace 读取项目 harness 中与当前源 SHA 一致的 SKILL.md，回答提出具体需求澄清问题并遵守暂不出方案，exit 0；独立范围复核 PASS。仅证明这一个普通任务的项目级可用性，不据此声称全局安装或自动触发 | PASS（当前源项目副本正向可用） | `capability-audit-evidence-20261018/S036-current-source-positive-smoke-20261019.md`; 独立复核及原始 prompt/trace/answer/exitcode/source-copy manifest |
| S037 | pdf-processing-toolkit | auto-installable-runtime | 是 | 合成文本 PDF 生成、页数查询、文本抽取、PDF合并；缺失/损坏输入应明确失败；本轮新增合并写失败一致性：合成writer部分写入后抛错，旧目标先前被覆盖；现同目录stage/fsync/非空验证后原子replace。focused2/2、PDF套件8通过/1硬链接ACL跳过、当前runtime smoke和双输入合并/抽取通过；独立审计PASS；G1/G2/adapter/metadata通过。强杀/断电、既有ACL元数据保留和真实硬链接未验证。 | 当前 USB helper CLI source-direct smoke 9/9；provisioned CODEX_PYTHON profile 固定向量与双输入合并验证通过。新增输出路径别名保护：同路径/归一化别名的抽取与合并均写入前拒绝，原PDF SHA不变；专项回归6项（5通过、1项真实硬链接因WinError 5跳过）并经独立审计PASS；不同文本输出正常，adapter contracts 81/25，G1/G2/metadata通过。自然路由/当前USB源加载及真实硬链接仍未验证，整体PARTIAL；新增extract部分写失败原子发布及fdopen描述符/暂存清理修复，focused4/4、PDF回归10/1 ACL skip、smoke及独立审计PASS；包清单412/413后再次G1/G2/adapter/metadata通过。真实磁盘/掉电与自然路由仍未测。 | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S037-pdf-processing-source-smoke-20261019.py`; `.log`; `S037-bundled-runtime-smoke-20261019.py`; `.json`; `.log`; `.md`; `S037-two-input-merge-20261019.py`; `.json`; `.log`; `.md`; `S037-output-alias-remediation-20261019.md`; `S037-extract-same-path-baseline-20261019.py/.log`; `S037-output-alias-regression-20261019.log`; `S037-output-alias-adapter-regression-20261019.log`; `S037-output-alias-G1-20261019.json`; `S037-output-alias-G2-20261019.json`; `S037-output-alias-metadata-20261019.log`; `independent-audit-S037-output-alias-20261019.md`; `S037-pdf-merge-write-failure-remediation-20261019.md`; `S037-pdf-merge-write-failure-baseline-20261019.py/.log`; `S037-pdf-merge-write-failure-focused-20261019.log`; `S037-pdf-merge-write-failure-suite-20261019.log`; `independent-audit-S037-pdf-merge-write-failure-20261019.md` ; `docs/capability-audit-evidence-20261018/S037-ordinary-positive-current-source-smoke-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S038 | powerpoint-pptx | auto-installable-runtime | 是 | 在临时输出目录创建三页本地 PPTX：标题页、两方案对比、结论与下一步；主题简洁，并检查内容。 | 三页结构、文本和几何检查通过；2026-10-19 使用打包 Artifact Tool 2.8.99 渲染并逐张目检通过，无文字裁切/重叠。不是 PowerPoint 本体渲染；自然触发/USB Skill 加载未证明；修复python-pptx 1.0.2颜色类示例拼写，四份Markdown RGBColor统一；focused2/2，当前包wheel生成/重开带颜色PPTX结构PASS，独立审计通过。PowerPoint原生渲染/自然路由未测。 | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S038-current-usb-source-proxy-20261019.md` (pre-cleanup historical proxy; source SHA differs); `capability-audit-evidence-20261018/S038-powerpoint-pptx-source-cleanup-20261019.md`; `S038-powerpoint-rgbcolor-example-remediation-20261019.md`; `S029-G1-post-market-sizing-fix-20261019.json`; `S038-postpatch-compatibility-20261019.json`; `capability-audit-evidence-20261018/independent-audit-S038-powerpoint-cleanup-20261019.md`; `capability-audit-evidence-20261018/independent-audit-S038-source-proxy-20261019.md`; `capability-audit-evidence-20261018/s038-artifact-render-review-20261019.md`; 3 PNG renders; prior PPTX and reproducible script; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S039 | python-env-setup | core-ready | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S039 逐字正/负探针 | 新源直接检查器正向识别真实 CPython 3.12.10 为 cp312/win_amd64/x64；绝对路径脱敏，父 PATH 未更改。新增空 PATH 负向 source-direct smoke 3/3 通过，确认无可用解释器时 fail-closed、无 PATH 修改、`-Install` 不执行安装；source-pinned正负代理经独立复核。自然路由此前未验证；2026-10-19自然任务读取当前副本，但检查器被执行策略拦截、未取得版本/ABI，纠察判本次attempt PARTIAL/环境未验证，不重试 | PASS（简化标准：基本功能已有可用结果） |`capability-audit-evidence-20261018/S039-current-usb-source-proxy-20261019.md`; `capability-audit-evidence-20261018/independent-audit-S039-source-proxy-20261019.md`; `capability-audit-evidence-20261018/s039-compatible-runtime-check-smoke-20261018.md`; `S039-empty-path-negative-smoke-20261019.md`; `.ps1`; `.log`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S040 | safe-disk-operations | core-ready | 是 | 用户明确排除安全、边界及破坏性测试；不执行删除/移动 | 本轮没有进行磁盘修改或安全边界测试；按用户范围标为不可测试，不据此判功能失败 | NOT_TESTABLE（用户范围排除） | `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md`; 既有静态与代理记录 |
| S041 | seedance2-0-video-gen | guided-config | 否 | 用明确指定的 Seedance 准备5秒竖屏视频；不提交 | 当前 2026-10-19 `media_capabilities` 原始结果显示 Seedance 未配置（缺模型及 Agent ID），本次能力调用 `api_request_sent=false` 且未 refresh/prepare/submit。另有独立历史运行调用 `media_refresh_models`，其结果报告 `api_request_sent=true`、`generation_request_sent=false`；该历史副作用不归入本次。真实生成及自然路由仍 NOT TESTED | NOT_TESTABLE（上游配置缺失） |`capability-audit-evidence-20261018/S041-seedance-capability-fail-closed-20261019.md`; previous offline regression and redacted source-run evidence; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S042 | self-improvement-agent | core-ready | 是 | 详见 USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md 的 S042 逐字正/负探针 | 新增用户明确 no-save 优先规则；当前 USB 源条款检查+隔离文件协议 smoke 7/7通过，含正向合成项目内记录与负向规则/空目录断言。该 harness 未加载模型或触发技能，不能证明自然路由/模型服从。旧隔离 Codex 正向写入仍有效；5次目标SKILL读取失败、负向模型回合未启动。2026-10-19一次自然任务输出有用但trace未读隔离USB源副本，纠察判本次attempt PARTIAL；整体仍 PARTIAL | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S042-no-save-rule-source-patch-20261019.md`; `S042-filesystem-protocol-smoke-20261019.ps1`; `.log`; `G1-post-S042-source-patch-20261019.json`; prior runtime summary/events/metadata; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S043 | skills-search | core-ready | 是 | 在本地能力包搜索技能，说明功能、状态和依赖；负向旅行建议不查目录 | 当前 USB 搜索脚本直接调用 smoke 5/5通过并经独立复核；自然正向已触发精确源副本并找对 travel-planner，但 helper 执行被 Codex 命令策略拒绝、退回手工读元数据；负向未搜技能目录但触发通用 web_search，纯本地边界未满足。保持 PARTIAL，USB 全局发现/安装生命周期未证 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S043-skills-search-source-smoke-20261019.md`; `.ps1`; `.log`; `independent-audit-S043-skills-search-20261019.md`; `S043-skills-search-proxy-20261019.md`; `S043-natural-route-check-20261019.md`; natural-route JSONL/answers/stderr; `s043-current-source-search-20261018.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S044 | traffic-hunter | core-ready | 是 | 正向零预算/两周矩阵；负向写咖啡新品小红书文案 | 冻结正向在项目 harness 自然路由读取与当前 USB 源树逐文件哈希一致的 `.agents/skills/traffic-hunter/SKILL.md`；输出免费优先获客步骤、两周内容矩阵/节奏/指标并标明实验假设。负向只写咖啡新品文案，未读取 traffic-hunter；实际未发布/投放/联系门店。正向另有只读网页搜索。仅证明项目级精确源内容代理，不证明 U 盘全局安装生命周期 | PASS（项目级精确源内容代理） | `capability-audit-evidence-20261018/S044-natural-route-source-proxy-20261019.md`; `S044-natural-route-positive-codex-exec-20261019.jsonl`; `S044-natural-route-positive-codex-exec-20261019.answer.md`; `S044-natural-route-negative-codex-exec-20261019.jsonl`; `S044-natural-route-negative-codex-exec-20261019.answer.md`; `s044-current-g0triple-runtime-summary-20261018.md` |
| S045 | travel-planner | auto-installable-runtime | 是 | 规划2026-11-10至12上海至杭州两晚行程、路线预算和待核验清单；不订票；新增嵌套字段稳健性复核：旧CLI对destination null/列表、budget null泄漏AttributeError traceback；现显式要求trip_data/destination/budget为Mapping并返回字段级错误。focused4/4、Travel Planner全套43/43，独立审计PASS；G1/G2/adapter/metadata通过。顶层非Mapping直接API覆盖，未单独覆盖CLI；自然路由与USB runtime未验证。| 当前源 generator 13/13 定向测试通过；已有 travel_db 数据保护与 ID 修复仍按此前记录。新增一条合成半日行程给出晴天路线及有雨替代、时间/费用影响；虚构场馆信息明确待核实，不联网不预订。细节及既存修复来源见 `S045-S050-minimal-usability-fixes-20261019.md`。自然路由和真实地点/时刻/价格仍未验证 | PASS | `capability-audit-evidence-20261018/S045-S050-minimal-usability-fixes-20261019.md`; prior generator/database records |
| S046 | user-research | core-ready | 是 | 对8条虚构健身新手访谈摘录归纳主题、区分自述/行为/推断，并提出下一轮问题 | 当前 USB 源副本普通正向任务完成：trace读取与当前源SHA一致的项目harness SKILL.md，输出5个主题、编号引用、样本限制和后续问题，exit 0；独立范围复核PASS。仅证明这一普通项目级任务 | PASS（当前源项目副本正向可用） | `capability-audit-evidence-20261018/S046-current-source-positive-smoke-20261019.md`; 独立复核及原始prompt/trace/answer/exitcode/source-copy manifest |
| S047 | venture-analysis | auto-installable-runtime | 是 | 正负向量均按缺失输入/纯术语解释边界执行；尽调清单输出原先直接write_text，partial-write OSError会改坏旧文件；现同目录stage/fsync/os.replace。基线复现；focused3/3、Venture全套16/16，独立审计另注入replace失败并确认保留旧字节；G1/G2/adapter/metadata通过。真实磁盘故障、强杀、目录fsync和ACL元数据未测。| 本机正负运行代理有边界行为证据；当前USB源四个脚本均有合成 source-direct smoke：`financial_model.py` 15项算术+6项契约断言，`business_evaluator.py`、`valuation_analyzer.py`、`due_diligence.py` 另22项分组断言；零 churn/NaN、两项权重错误均拒绝且不生成报告，财务结果和尽调模板边界均经独立复核。venture financial_model/valuation_analyzer 的 bool 年限、初始用户和评分已新增拒绝回归并经独立审计；USB自然Skill路由/源加载、真实项目数据和投资适合度未证 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S047-financial-model-source-smoke-20261019.md`; `.py`; `.log`; `independent-audit-S047-financial-model-20261019.md`; `S047-venture-scripts-source-smoke-20261019.md`; `.py`; `.log`; `independent-audit-S047-venture-scripts-20261019.md`; `S004-financial-bool-input-remediation-20261019.md`; prior runtime summary and redacted events; `S047-due-diligence-output-failure-remediation-20261019.md`; `S047-due-diligence-output-failure-baseline-20261019.py/.log`; `S047-due-diligence-output-failure-focused-20261019.log`; `S047-due-diligence-output-failure-suite-20261019.log`; `independent-audit-S047-due-diligence-output-failure-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S048 | video-prompt-generator | core-ready | 是 | 冻结单镜头视频提示词正向与普通小说负向 | 当前 USB 源 SHA `193becc318b44a274340ff245ba5e315c3893290edecb0de0829aac611c9ca91`；4/4 文件源副本 hash 匹配。正负冻结提示逐字执行且 exit 0；正向读取 harness SKILL.md，覆盖单镜头8秒/时间动态/视觉要素/运镜/负面词；负向未读目标 Skill；无 MCP/视频生成调用。源合同6/6、独立审计PASS。范围为项目级精确源内容代理。 | PASS（项目级精确源内容代理） | `capability-audit-evidence-20261018/S048-current-source-smoke-20261019.md`; `S048-current-source-smoke-independent-review-20261019.md`; raw prompts/traces/answers/stderr/exitcode; `S048-current-source-contract-20261019.txt`; `S048-current-source-route-validation-20261019.json` |
| S049 | wealth-allocation | core-ready | 是 | 用审阅过的虚构三桶策略模板和¥100,000算金额及应急金差额；负向解释风险分散，不做个人配置 | 当前 USB 三个脚本的合成 source-direct smoke 11/11通过；新增复现并修复再平衡 current/target 资产代码去空格碰撞：两端重复均拒绝，唯一trim保留；CLI码1、JSON错误、无traceback、输出不落地；rebalance修复时adapter 80 tests/25 skipped，经独立只读审计PASS。asset_allocator 的总额/月支出/期限及 portfolio_tracker 的月份现拒绝 bool，五helper adapter regression 81/25并经独立审计（S004）；新增 rebalance_planner current/target 作为 output 时拒绝覆盖，CLI 4/4 且独立审计通过，adapter 81/25；自然Skill路由和真实数据流程未验证；另修Portfolio Tracker基准名trim后碰撞静默覆盖：规范化重复现拒绝，不同名结果保持；focused2/2、Wealth suite30/30、独立CLI旧输出保护PASS。自然路由/真实数据仍未测。 | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S049-rebalance-code-collision-remediation-20261019.md`; `S049-rebalance-code-collision-baseline-20261019.md`; `S049-wealth-allocation-source-smoke-20261019.py/.log`; `S049-rebalance-regression-20261019.log`; `G1-post-S049-rebalance-fix-20261019.json`; `G2-post-S049-rebalance-fix-20261019.json`; `independent-audit-S049-rebalance-code-collision-20261019.md`; `S004-financial-bool-input-remediation-20261019.md`; `independent-audit-S004-financial-bool-input-followup-asset-allocator-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S050 | weather | core-ready | 是 | 实时天气与理论解释负向边界、同名地点消歧、来源回退 | 本次修复后源合同6/6、独立复核PASS：地点/行政区回显、当地/IANA时区、观测/预报有效时间及fail-closed。针对此前 `month=6&year=2026` URL 错配，增加引用URL日期参数与当地所述日期一致规则；单次东京正向查询使用日期匹配来源，较早观测明确降级、雨情不确定如实说明。证据见 `S050-weather-timezone-location-remediation-20261019.md` 与 `S045-S050-minimal-usability-fixes-20261019.md`。自然路由和USB运行时来源仍未证明 | PASS | `capability-audit-evidence-20261018/S045-S050-minimal-usability-fixes-20261019.md`; `capability-audit-evidence-20261018/S050-weather-timezone-location-remediation-20261019.md`; prior source contract: `S036-S050-source-contract-checks-20261019.log` |
| S051 | weather-forecast | core-ready | 是 | 当前源 `weather_db.ps1` 在隔离目录 set/read 默认城市“苏州”；查询苏州当日天气 | 当前USB `weather_db.ps1` 隔离 source smoke 10/10通过；修正后的树 rerun 10/10由独立审计者复跑。新增本次普通正向任务：既有workspace临时目录内保存并读回苏州；一次查询返回日期匹配的当日预报，温度/降雨来源差异与当前实况未确认均明确标注，纠察判基本功能PASS。自然路由/USB Skill加载未证；旧 CLI 因 `state_5.sqlite` 权限错误在模型回合前失败的记录保留为环境限制 | PASS | `capability-audit-evidence-20261018/S051-ordinary-positive-recheck-20261019.md`; `S051-weather-db-source-smoke-20261019.md`; `S051-weather-db-source-rerun-20261019.log`; `independent-audit-S051-weather-db-current-rerun-20261019.md`; `local-proxy-rechecks-20261018.md`; `G0-current-local-proxy-equivalence-20261019.json` |
| S052 | web-content-fetcher | core-ready | 是 | 使用合成 HTML fixture 测正文/目录/链接提取；负向解释 Markdown 标题且不抓取；本轮max_chars边界复核：401字符fake正文下旧API传-1返回400字符、0返回空；现API/CLI要求正整数并在依赖导入/网络前校验。focused3/3、全套10/10、独立审计PASS、G1/G2/adapter/metadata通过。仅mock抓取，真实网络、自然路由及USB加载未测。| 独立复核捕获懒加载图假阳性及真实转换丢失图片；当前USB源已修复重复/占位`src`、常见1×1 GIF与无引号属性。包内回归7/7，mock adapter smoke 4/4，真实html2text+假Fetcher 6类图片形态与Markdown链接26断言通过（占位符支持边界限于标准Base64的1×1 GIF/PNG及已识别字符串）；缺依赖仍在Fetcher调用前停止。真实抓取探针在页面请求前因缺少Playwright退出；Scrapling 0.4.15/html2text 2025.4.15已安装，Playwright缺失。requirements.txt并非锁文件的说明已修正；SKILL/README澄清当前 HTTP Fetcher 不需要浏览器；runtime input 已改用 scrapling[fetchers] 并重建86个Python+Manim wheels（9个新增wheel）；原有S052 15项测试全过，locked Python profile离线安装后经loopback HTML取正文/图链smoke通过，no-index + require-hashes锁解析通过，8项依赖合同断言通过；未触真实网页，不证明外部可达性/内容质量。自然路由/USB安装生命周期未证，保持PARTIAL | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S052-S053-source-fixes-20261019.md`; `S052-fetch-adapter-offline-smoke-20261019.py`; `.log`; `S052-lazy-image-package-unittest-20261019.log`; `S052-lazy-image-real-converter-smoke-20261019.py`; `.log`（6情形/26断言）; `independent-audit-S052-web-content-fetcher-20261019.md`; `S029-G1-post-market-sizing-fix-20261019.json`; `S052-max-chars-boundary-remediation-20261019.md`; `S052-max-chars-boundary-baseline-20261019.py/.log`; `S052-max-chars-boundary-focused-20261019.log`; `S052-max-chars-boundary-suite-20261019.log`; `S052-scrapling-runtime-dependency-remediation-20261019.md`; `S052-scrapling-dependency-contract-20261019.log`; `G1-post-S052-fetchers-runtime-20261019.json`; `G2-post-S052-fetchers-runtime-20261019.json`; `S052-S053-final-metadata-check-20261019.log`; `independent-audit-S052-max-chars-boundary-20261019.md` `S052-focused-package-tests-20261019.log`; `S052-runtime-extra-contract-20261019.log`; `S052-offline-lock-resolution-20261019.log`; `S052-provisioned-http-fetch-local-smoke-20261019.log`; `adapter-contracts-post-S052-fetchers-runtime-retry-20261019.log`; `media-mcp-post-S052-fetchers-runtime-20261019.log`; `docs/capability-audit-evidence-20261018/S052-ordinary-positive-current-source-smoke-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S053 | web-search-extraction | guided-config | 否 | 自然触发：S048–S050 同类 app-server state DB 启动故障门控，未运行/NOT_TESTABLE；直接源脚本测试：缺 Key 和缺 Model 均在网络请求前退出；HTTP错误回显dummy key的缺陷已修复，9项mock测试通过 | 源脚本缺配置 fail-closed PASS；另复现并修复前确认HTTP错误正文会回显调用方key（pre-patch FAIL，见 `s053-prepatch-http-error-secret-leak-repro-20261018.md`）；自然路由/Skill 运行时读取未证，整体 NOT_TESTABLE。9项包内离线mock unittest通过（含bool整数边界）；新增源级契约smoke 20项通过（空查询/字数、非法HTTPS端点、畸形响应脱敏、成功请求payload；transport全mock），详见 `S053-web-search-offline-unittest-20261019.log`（9/9）; `S053-web-search-additional-contract-smoke-20261019.py`（20/20）; `.log`；`--max-words` 已澄清为prompt软约束、上游超长内容原样返回；focused2/2和独立复核通过。详见 `S053-max-words-soft-constraint-remediation-20261019.md`、`s053-source-script-smoke-20261018.md`、`s053-missing-model-guard-smoke-20261018.json`、`s053-postpatch-error-redaction-20261018.md` | NOT_TESTABLE | `capability-audit-evidence-20261018/s053-source-script-smoke-20261018.md`; `S053-max-words-soft-constraint-remediation-20261019.md`; `final-verification/S053-focused-post-review-20261019.log`; G3 当前记录。`max_words=True` 现于请求前被拒绝。
| S054 | wechat-article-creator | auto-installable-runtime | 是 | 本机根据主题/受众/立场规划公众号文章；负向正式通知未误触发文章格式；新发现不支持扩展名先建目录再失败：`.txt`旧基线已复现；现入口先casefold校验.jpg/.jpeg/.png，unsupported不渲染/不建目录；supported格式用例通过。focused2/2、全套14/14，独立审计PASS；G1/G2/adapter/metadata通过。仅本地Pillow，真实路由和runtime未测。| `polish_text.py` 字段/标签修复；当前USB source-direct合成smoke 9项、原包内unittest 2/2通过；新增封面像素宽度布局修复及4项回归，provisioned profile长/短标题图14项断言并经独立目检通过；G1、兼容性、PowerShell与包哈希门通过。自然路由/USB源加载仍未证：隔离项目尝试实际读取了同名但哈希不同的 `C:/Users/T14S/.codex/skills/wechat-article-creator`，且运行环境报告只读、未保存草稿；该次不计入USB源通过。2026-10-19本轮纯文本写作同样加载了哈希不同的本机Skill（USB BDFE9A… / local B053A8…），不计当前源通过，详见 `S054-ordinary-natural-current-source-smoke-20261019.*`。模型写作质量与发布也未测；旧本机代理不代表修改后的当前源 | PASS（简化标准：单项普通任务有用） |`capability-audit-evidence-20261018/S054-wechat-article-source-remediation-20261019.md`; `S054-wechat-article-scripts-source-smoke-20261019.py/.log`; `S054-polish-text-unittest-20261019.log`; `S054-cover-layout-remediation-20261019.md`; `S054-cover-layout-fixed-runtime-smoke-20261019.py/.json/.log`; `S054-cover-layout-unittest-20261019.log`; `independent-audit-S054-cover-layout-20261019.md`; `test_generate_cover_layout.py`; G1/compatibility/PowerShell gates; latest `G1-current-after-S007-20261019.json` / `G2-compatibility-current-after-S007-20261019.json`; `S054-cover-extension-side-effect-remediation-20261019.md`; `S054-cover-extension-side-effect-baseline-20261019.py/.log`; `S054-cover-extension-side-effect-focused-20261019.log`; `S054-cover-extension-side-effect-suite-20261019.log`; `independent-audit-S054-cover-extension-side-effect-20261019.md`; `S054-natural-route-source-attribution-20261019.md`; `S054-natural-route-codex-exec-20261019.jsonl/.answer.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S055 | word-docx | auto-installable-runtime | 是 | USB源生成含标题/二级标题/列表的DOCX；负向纯文本校对不生成文档 | 修复UI超出内容保证的视觉验收承诺，补路径/依赖/宏安全边界及结构与视觉验收区分；源合同5/5，结构roundtrip 1/1，独立复核PASS。解释器Python3.12.10/ python-docx1.2.0与锁定版本一致但非USB provisioned profile；当前源自然路由和修订后视觉QA未验证；2026-10-19 DOCX任务路由到本机meeting-secretary，未读USB word-docx且未生成DOCX，本次attempt PARTIAL，详见 `S055-ordinary-natural-current-source-smoke-20261019.*` | PASS（简化标准：单项普通结果有用） |`capability-audit-evidence-20261018/S055-word-docx-source-remediation-20261019.md`; `S055-focused-source-contract-20261019.log`; `S055-docx-structural-smoke-20261019.log`; `independent-audit-S055-source-remediation-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| S056 | xiaohongshu-writing | core-ready | 是 | USB源真实性/功效/商业披露与语气偏好静态合同 | 源合同9/9和G1/G2/PS5.1/media/adapter/metadata通过；冻结正向逐字复测读取与USB源3文件逐哈希一致的项目副本，输出3标题、短正文、标签且不虚构亲测/效果；冻结负向转为正式同事邮件且未读目标Skill。独立审计PASS。范围是此正负向量的项目级精确源内容代理，不证明全局安装或重载。 | PASS（项目级精确源内容代理） | `capability-audit-evidence-20261018/S056-frozen-natural-route-20261019.md`; `S056-frozen-natural-positive-prompt-20261019.txt`; `S056-frozen-natural-positive-codex-exec-20261019.jsonl`; `S056-frozen-natural-positive-answer-20261019.md`; `S056-frozen-natural-positive-exitcode-20261019.txt`; `S056-frozen-natural-negative-prompt-20261019.txt`; `S056-frozen-natural-negative-codex-exec-20261019.jsonl`; `S056-frozen-natural-negative-answer-20261019.md`; `S056-frozen-natural-negative-exitcode-20261019.txt`; `S056-xiaohongshu-writing-source-remediation-20261019.md`; `independent-audit-S056-source-remediation-20261019.md` |

## 顺序执行：Agents

| ID | Agent | 能力范围 | 依赖状态 | 最小验证探针 | 通过要点 | 结果 | 证据 |
|---|---|---|---|---|---|---|---|
| A01 | 创作官（content） | 新媒体内容架构与创意生产，覆盖公众号、小红书、图片提示词、视频提示词和脚本。 | core-ready | 合成咖啡馆小红书提纲 | 输出结构完整，不编造数据/评价/效果 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md` |
| A02 | 全能官（general） | 综合统筹与长期协作，覆盖跨领域分析、文件处理、计划制定和任务拆解。 | core-ready | 当前 USB 源隔离 CODEX_HOME 委派 + 平台 `agent_type` 合成探针 | 较早 CLI 显式请求未产生子 Agent，但未同次证明 USB TOML provenance；2026-10-19 的隔离重试已连通端点，但因 CODEX_HOME 无 bearer 收到401，未到模型/子Agent回合（NOT_TESTABLE）。不能归因于 TOML 配置；平台角色回答通过。 | PASS（简化标准：平台角色任务有用） |`capability-audit-evidence-20261018/agent-role-smokes-20261018.md`; `G4-isolated-agent-runtime-attempt-20261018.md`; `G4-isolated-agent-runtime-retry-20261019.md`/`.jsonl`/`.stderr.txt`; `G4-current-cli-exec-help-20261019.md`; `capability-audit-evidence-20261018/SIMPLE-USABILITY-RECHECK-20261019.md` |
| A03 | 增长官（growth） | 商业增长与变现设计，覆盖用户痛点、产品 Offer、流量、转化、留存和增长实验。 | core-ready | 无基线的咖啡馆增长诊断 | 缺数据识别和实验设计通过；USB TOML来源未证；项目级TOML尝试因collab spawn no thread失败，无child结果，不判源缺陷 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md`; `capability-audit-evidence-20261018/A03-growth-agent-project-runtime-attempt-20261019.md`; `.jsonl`; `.stderr.txt`; source-copy SHA-256 |
| A04 | 投资官（investment） | 投资研究与财富配置，覆盖 A 股、基金、宏观、创业投资和组合管理。 | auto-installable-runtime | 缺少关键数据的虚构持仓加仓问题 | 未给无依据买卖建议，列出信息缺口；USB TOML来源未证 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md` |
| A05 | 生活官（lifestyle） | 健康与生活方式规划，覆盖健身、饮食、旅行、天气和个人复盘。 | core-ready | 缺城市/预算/偏好的周六安排 | 给灵活模板并指出个性化信息；USB TOML来源未证 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md` |
| A06 | 办公官（office） | 职场效率与交付管家，覆盖文档、数据、邮件、会议、表格和文件整理。 | core-ready | 合成会议纪要提取 | 行动项/负责人/截止日/未决项准确；无发送保存；USB TOML来源未证 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md` |
| A07 | 研究官（research） | 深度研究与情报分析，覆盖学术、竞品、产业链、用户研究和市场测算。 | core-ready | 两家匿名咖啡馆竞品研究设计 | 证据框架和推断边界通过；USB TOML来源未证 | PASS（平台角色；TOML来源未证） | `capability-audit-evidence-20261018/agent-role-smokes-20261018.md` |

## 补充执行：本机缺失组件安装与运行冒烟

本阶段仅把发行源缺失项添加到本机用户级 Skill 目录，以便启动新 CLI 会话验证；发行源仍为唯一审计对象，现存同名组件一律不覆盖。

| 项目 | 状态 | 证据/备注 |
|---|---|---|
| 本机映射冻结与 Source Fidelity 复核 | PASS | `capability-audit-evidence-20261018/local-install-mapping-baseline-20261018.json`；56 Skills 中 51 个已存在并跳过、5 个缺失待补；7 个 Agent TOML 全部存在并跳过。独立只读复核 PASS。14 个同名 Skill 内容与源不同，均跳过；Grok 同名但内容不同，未覆盖。 |
| 添加缺失 Skills | PASS | 仅复制 `codex-skin-design`、`hyperframes-video`、`local-video-editing`、`manim-animation`、`minimax-video-gen` 至 `%USERPROFILE%\.agents\skills`；5/5 目录树与冻结发行源 SHA-256 匹配。见 `capability-audit-evidence-20261018/local-install-copy-verification-20261018.json`。 |
| Agent 本机更新 | SKIPPED | 7/7 本机文件均已存在；6/7 哈希与当前 U 盘源一致，`content.toml` 存在但哈希不同（源 `38151ddbef5e8157bf5b68b332111d46300b8c0d671466d113e42d81a75e4288`，本机 `991237be24c5690a9bf4668112c9c4cf453f4591f8b5846a8370afde37c52d46`）。按“仅补缺失、不覆盖”边界保留现文件；哈希映射见 `capability-audit-evidence-20261018/local-install-mapping-baseline-20261018.json`。文件存在/哈希匹配不证明 TOML 已被运行时加载。 |
| MiniMax CLI 自然提示冒烟 | PARTIAL | 首次默认审批策略为 `never` 时未完成检查，错误地仍给草稿；第二次自动审批审查下仅调用 `media_capabilities`，发现 MiniMax 未配置并停止，无刷新、准备、提交或换模型。第二次自然提示行为通过；第三次显式调用确证加载源哈希匹配 Skill。自然路由表现仍不一致。 |
| 本地渲染运行时 | NOT_TESTABLE | `runtime-state.json` 总体标记 ready，Python 3.12 health passed；Node 的状态字段为 skipped，但 PATH 有 Node 24.14.1/NPM，Chrome 存在。包内 `ffmpeg.cmd`、`ffprobe.cmd`、`manim.cmd`、`hyperframes.cmd` 均不存在；FFmpeg/FFprobe、Manim、HyperFrames 命令未找到，隔离 Manim 环境缺失。剪辑/动画/HyperFrames 端到端渲染未执行。完整快照 `capability-audit-evidence-20261018/local-runtime-preflight-20261018.json`。 |

## 非组件问题与收尾

| ID | 检查项 | 状态 | 证据/备注 |
|---|---|---|---|
| D001 | README 对 Skills 总数/default/guided 数量是否与 1.5.4 manifest 一致 | PASS | 当前源复核为 56 Skills / 49 默认 / 7 guided-config；与目录、manifest 和 README 对齐。旧 55/47/8 结果已由当前源复核取代。 |
| D002 | file-manifest 与 checksums 对包内文件闭包一致 | PASS | S004 五个财务 helper bool 修复纳入清单并刷新：330 manifest files、331 checksum entries；G1包树332个文件、排除20个瞬时文件，零缺失/多余/哈希差异。树 SHA `e5b847d7c6555dc59f29c78804acb9a7b10b6ad79c571dea738ddf2e6a6d75d9`；metadata `--check` 证据 `S004-financial-bool-metadata-20261019.log`。 |
| D003 | defaultEnabled 与实际 skill/agent id 集合一致 | PASS | 只读结构脚本：49/49 默认 Skill、7/7 默认 Agent 均有对应目录/文件；56/56 Skill 与 manifest、7/7 Agent 与 agent-audit manifest 集合一致。 |
| D004 | 汇总 PARTIAL/FAIL/NOT_TESTABLE/INSUFFICIENT_EVIDENCE 并复测 | PARTIAL | 当前 G1/G2 静态兼容和媒体契约检查通过；当前包树 `20c864b3…`，兼容性15项、media MCP64/64、adapter contracts76通过/25跳过。S009 DOCX与S012 XLSX合成结构验证已补；S009当前Word COM复测仍于应用创建前E_ACCESSDENIED，未生成页面图；S038清理旧ClawHub指引；S052修正网页提取器依赖契约并通过4/4 mock；S030营养聚合/4-4-9算术 smoke 61/61并经独立复核，S030仍PARTIAL；S026 使用真实provisioner生成的隔离profile wrappers 完成含中文/空格路径A/V裁切18项断言并经独立复核；USB安装生命周期/自然路由仍未证；S047当前USB四个脚本合成source-direct smoke均通过（财务模型21项、另三脚本22项分组断言），四个非法输入拒绝且不写文件，经独立复核；自然Skill路由未证，仍PARTIAL；已有PPTX渲染证据仍不代表PowerPoint本体或自然路由。S050原生天气工具无结果后网页回退成功，来源时间、实况/预报区分及预报分歧处理已复核；自然路由/curl未测；S032/S041上游配置缺失，MiniMax适配器不可用。39 PARTIAL/5 NOT_TESTABLE 与G4 Agent运行时来源/生命周期缺口仍在；证据账本596条记录，全量路径哈希与唯一ID复算闭环（含本轮S002/S009/S052/S053及G0/G1当前证据）。 S054 字符数修复及源脚本smoke 9项、unittest 2/2、封面布局回归4/4和provisioned视觉目检均已归档；封面布局专项闭环，S054整体仍PARTIAL。 S037 PDF helper新增9项源级合成验证并经独立复核；provisioned Python 两输入合并补测12项经独立复核通过；S053八项mock unittest重跑已通过。 S053额外20项source-direct mock契约检查通过，运行时路由仍NOT_TESTABLE。 S052懒加载图片假阳性已修复，7/7包内回归与真实html2text smoke 26/26通过并独立复核；S053 bool参数缺陷已修复，9项unittest及20项mock smoke通过并独立复核。当前本机等价快照为29/56匹配。
| D005 | 独立只读审计结果 | PARTIAL | S017原 avoid_exercises 13/13与 DB smoke 4/4经独立审计通过；新增肌群映射/分类及负向 fixture 完整回归16/16通过；S030数据库ID与损坏JSON、S017数据库ID、S007损坏JSON保护独立复核通过；S045腐坏JSON、trip ID及支出/行程条目 ID 碰撞修复均独立复核通过；S053凭据回显缺陷由mock复现并由8项离线测试覆盖；上游响应结构、非法UTF-8、深层JSON与大整数解析失败均新增稳定错误和13项离线覆盖，独立审计PASS；自然路由仍未测。S032/S041能力探测经独立只读复审通过；当前Agent源闭包、CLI help及本机与USB逐树等价检查已补充；S037 PDF helper source smoke 9项及S047四个source-direct脚本的计算/边界及负向校验经独立复核；S019 raw log闭环、S024补充确认门、S037/S038当前源代理、S051当前源代理、S003当前源离线smoke、S007隔离数据库helper smoke、S039空PATH负向smoke及S027三个宏观脚本离线smoke均经独立复核。证据账本843条，路径/哈希/唯一ID均已复算闭合；S027新增4条证据记录已独立审阅；S018新增4条当前源脚本测试证据已独立复核；S018 SIP 极值溢出、bool数值拒绝、S004五个财务helper bool拒绝、sourceTree 摘要规则与元数据门禁均经独立审计 PASS；S049 再平衡资产代码规范化冲突修复经独立审计 PASS；S037 PDF输入输出路径别名覆盖缺陷修复（抽取/合并）及CLI哈希保护经独立审计 PASS；S049再平衡current/target输出别名保护及源JSON哈希回归经独立审计 PASS；S045 plan export 输出路径覆盖trips/preferences持久化JSON修复及两库哈希保护经独立审计 PASS；S045 add_trip ideas误状态无写入却返回ID修复并由持久化回归独立验证；travel save_json 非原子覆盖修复与失败保留原库经独立复核；meal/fitness/reflection 三个DB同构非原子写均修复并经故障注入独立审计；S003扩展到11/11并加入筛选器，4条证据已独立复核；S049新增4条源码烟测证据已独立复核；S017新增4条fitness_db隔离烟测证据已独立复核；S043新增4条Skills Search源 smoke证据已独立复核；S051新增4条天气配置隔离 smoke 证据已独立复核；S019新增4条Grok compiler/helper源测试证据已独立复核；G4新增4条当前官方Agent schema结构对照证据已独立复核；34条未捕获精确UTC运行时刻的旧时间值已置空并注明原因。G3/G4仍有源行为、运行时路由与生命周期缺口。 | S052/S053当前源修复与独立复核已归档；S002 DOM stub 交互逻辑 smoke 54项通过（不代表浏览器/视觉通过）；S009渲染因无soffice且Word COM被拒而未通过；当前本机等价快照29/56匹配、27项全本机副本均有差异。
| D006 | 本机缺失项映射及无覆盖安装 | PASS | 仅新增 5 个缺失 Skill；5/5 源树哈希匹配；7 个 Agent 与所有已存在同名 Skill 均未覆盖。独立 Source Fidelity 复核通过。 |
| D007 | MiniMax 自然提示与本地依赖检查 | PARTIAL / NOT_TESTABLE | MiniMax 显式 Skill 路由与失败关闭通过；自然触发不稳定，首次审批策略阻断；FFmpeg/Manim/HyperFrames wrapper 缺失。见本节证据路径。 |

## 当前能力验证边界

- 当前 G1 证明 U 盘开发源静态结构与引用闭包通过；7 个瞬时 pycache 按审计规则排除。G1 不证明 Codex 会话能自动发现/加载这些源文件。
- 本运行时未提供可归属到该 U 盘源版本的 skill/agent 路由事件；因此 USB 文件加载/自然发现来源仍未证。按用户后来确认的代理规则，本机测试可以作为行为代理；但当前源树哈希比较仅29/56个Skill存在内容一致的本机副本，因此内容不同的本机回合不能代表当前USB源行为，且任何本机代理均不证明USB源路由。
- 截至 G0‴ 阶段的历史 G3/G4 快照曾记录覆盖不足；该快照不含下方新增本机代理复测。当前行为状态以本清单最新 Skills 状态列和 Agent 表为准；USB 源文件加载/路由来源仍为 `INSUFFICIENT_EVIDENCE`。G3/G4 阶段门的整体状态还包含失败项、部分项和未测试项，因此仍按上方阶段门评为 INSUFFICIENT_EVIDENCE。
- 若未能构造使用这份确切 U 盘包的隔离 runtime，USB 源文件加载/路由来源状态保持 `INSUFFICIENT_EVIDENCE`；行为状态按本机代理规则评估，不要求因此覆盖真实用户全局目录。






## 最新复核：本机运行代理状态（2026-10-18）

此处是早期本机代理统计，已由当前源/本机逐树等价复核 supersede；当前应以 Skills 表为准：**12 PASS/39 PARTIAL/0 FAIL/5 NOT_TESTABLE**。29/56个Skill至少有一个本机副本树哈希与当前USB源一致；27个Skill在所有本机副本中均有差异，本机行为不能代理其当前源内容。树哈希相同也不证明运行时加载了具体本机副本或 USB 源。详情见 `G0-current-local-proxy-equivalence-post-source-fixes-20261019.json`。S008 的 PASS 仅指显式调用代理行为；自然调用未读目标 Skill，不能作为自然路由 PASS。S034 当前源显式行为代理通过，但真实配对后端及原生路由未证。S031/S043/S049 均有固定向量代理及独立复核，仍为 PARTIAL。 S054 当前源脚本修复后不再由早期本机行为代理覆盖。

本机新增代理证据与限项见 `capability-audit-evidence-20261018/local-proxy-rechecks-20261018.md`。S049 与 S055 正向测试完成；两者负向回合因 CLI 在模型回合前遇到 `state_5.sqlite` 无法打开而未执行，不能算负向通过。S055 的 DOCX 结构检查和原生 Word 单页视觉检查通过；打包 LibreOffice renderer 因 Windows 主机无 `soffice.exe` 仍不可用，且先前清理临时 LibreOffice profile 时遇到 access denied；S055 保持 PARTIAL。S050 因天气观测来源/时效冲突为 PARTIAL；本轮原生查询工具报错且受限网络下fallback未测。S051 的城市写入/读取与查询不修改配置断言通过，带天气来源时间说明。

USB `installer-pack` 未被本机测试运行时加载的事实单独保留。用户代理规则允许将本机行为通过作为 USB 安装行为的预期代理，但不构成 USB Skill/Agent TOML 的加载来源证明。`content.toml` 本机/USB 哈希差异仍未解决；没有覆盖本机文件。当前 G1 PASS；G2 adapter contracts 最新 76 tests run/25 skipped；G3/G4 仍 INSUFFICIENT_EVIDENCE。

## 2026-10-18 追加复测：S023 / S032 当前工作树

> 本节是较早的复测快照。随后 S023/S032 的更新结果和状态以本文上方 Skills 表、G3 汇总及 `capability-audit-evidence-20261018/s023-current-runtime-probe-20261018.md`、`s032-current-source-review-20261018.md` 为准；历史越界调用仍保留。

本轮 G0 快照（不是 clean release baseline）：1.5.4 包，311 个 Git 跟踪文件，`installer-pack` 已有 11 个用户工作树修改；测试前后 tracked-only 指纹均为 `b0b05db5c5307916ab8fff02198599ba54ee3df9ef46e661e1e0cf734fc13ffd`。旧 G0‴ 指纹 `c4e841…` 保留为历史，不覆盖当前指纹。完整边界和逐文件 SHA 见 `capability-audit-evidence-20261018/S023-S032-current-worktree-freeze-20261018.md`。

| 项目 | 结果 | 说明 |
|---|---|---|
| 当前媒体 MCP 离线回归 | PASS | 在临时包副本运行 63 tests / 6.342s；fake transport，无真实网络/凭据。日志 `S023-S032-media-mcp-offline-test-20261018.log`。 |
| MiniMax 未配置安全停止 | PASS（隔离禁网探针） | `media_capabilities` 报 `configured=false`；`media_prepare` 安全拒绝；网络 transport 被哨兵禁止且未触发。日志 `S032-minimax-unconfigured-contract-20261018.log`。 |
| 旧 adapter MiniMax 用例 | SKIPPED | 文件中的类级装饰器已将该旧用例整体 skip；不计为通过。 |
| S023 / S032 自然语言 Skill 路由 | 未复测 | 这轮契约测试不覆盖自然路由；维持原 S023 错路由、S032 曾错误 refresh 的记录及 G3 状态，不将 MCP PASS 推广为 Skill PASS。 |

## 追加 G0/G2/G3 状态解释

本轮对当前有未提交改动的工作树做了定向补充验证，不重写历史 G0‴/G1/G2。可确认的是当前用户工作树中的 MCP 离线契约通过；完整 G1 闭包、56 Skill 自然触发和 7 Agent TOML 运行时来源依旧未整体验收，USB 行为代理规则只接受本机行为，不证明运行时从 U 盘加载了具体文件。见 `capability-audit-evidence-20261018/S023-S032-current-worktree-freeze-20261018.md`。

S045 trip ID 修复证据：`capability-audit-evidence-20261018/S045-trip-id-collision-baseline-20261019.py`; `.log`; `installer-pack/skills/travel-planner/scripts/test_travel_db_trip_id_uniqueness.py`; `S045-trip-id-uniqueness-regression-20261019.log`; `S045-trip-id-collision-remediation-20261019.md`; `independent-audit-S045-trip-id-collision-20261019.md`; current metadata/G1/G2/PowerShell and adapter logs.


## 2026-10-19 追加复核：封面 fallback 与 3 个生成器输出路径

本节 supersede 表格中较早的相应测试快照，不提升自然路由或 Skill 总体状态。当前 G1：56 Skills / 7 Agents，344 个审计树文件（排除 20 个瞬时 pycache），无 manifest/reference/checksum mismatch；包内 342 个 manifest 文件、343 个 checksum；package sourceTreeSha256 `ca392d63f1e8ca9f1fcce8da49dc4251cfbc35e1db6727fb218725b926a543c3`，审计树 SHA `3ad5dcaf432e031b92e46f504cb341f05680643a7c80ba72b4ccbaae5c7939b3`。G2 15 项通过，adapter contracts 81 passed / 25 skipped。

- **S054 字体 fallback：**独立审计发现并修复无候选字体时将 `None` 送入 `ImageFont.truetype()` 的崩溃。微信文章脚本套件 12/12 通过并独立复核。默认 bitmap fallback 字体中文字形视觉质量未验证；S054 仍 PARTIAL。证据见 `S054-cover-layout-remediation-20261019.md` 与 `independent-audit-S054-font-fallback-20261019.md`。
- **S049 wealth-allocation `portfolio_tracker.py`：**修复 `--output` 覆盖 history/benchmarks JSON。source 6/6、Skill suite 10/10，独立审计通过。samefile 由 mock 覆盖，真实 NTFS hardlink 因 WinError 5 未测试。总体自然路由仍未测。
- **S017 fitness-coach `program_generator.py`：**修复输出覆盖 profile/programs/workouts 数据库，guard 在档案读取和 add_program 前生效。source 5/5、全套 27/27，独立审计通过。
- **S030 meal-planner `recipe_generator.py`：**修复输出覆盖 profile/plans/logs/recipes 数据库，保留可选数据库缺失时的正常初始化。source 5/5、全套 15/15，独立审计通过。

上述路径保护只证明当前 U 盘源的直接 CLI 行为，不证明自然 Skill 路由或 runtime 加载来源；S017/S030/S049 整体状态仍见 Skills 表。证据账本当前 858 条，路径存在、SHA-256 和唯一 ID 全量复核闭合。

## 2026-10-19 追加复核：Fund Portfolio 与 Grok 编译器输出别名

本次仅关闭两个直接 CLI 数据覆盖缺陷，不提升总体技能/Agent运行状态。当前包元数据：344个 manifest 文件、345个 checksum，sourceTreeSha256 `e03ffa9858c2254755494dbd02d7aec7ebe97e7549d2b1a5820063f21c4cd3c9`。G1 56 Skills / 7 Agents，346审计树文件（20 transient），无闭包/哈希差异；G2 15项通过；adapter contracts 81通过 / 25过期 fixture 跳过；证据账本866条且路径、哈希、唯一ID复核全通过。

- **S018 Fund Portfolio：**`portfolio_builder.py --output` 原可覆盖输入模型 JSON。添加写入前路径/文件身份保护；基线和回归覆盖精确路径、规范化 alias、同文件身份拒绝、权限错误关闭和独立输出。专项5/5，全套10/10，独立审计通过。真实NTFS hardlink因沙箱拒绝未实测。
- **S019 Grok Prompt Compiler：**`compile_prompt.py --output` 原可覆盖输入 SceneSpec JSON。添加读取输入前路径/身份保护；同路径、规范化别名、samefile、身份访问错误、独立输出专项5/5，既有编译函数29/29，独立审计通过。真实硬链接未实测。

两项都只证明当前 U 盘源的直接 CLI 行为，不证明自然 Skill 路由、Codex runtime 从 U 盘加载来源或真实上游生成。S018/S019总体仍保持PARTIAL。证据分别见 `S018-portfolio-builder-output-alias-remediation-20261019.md`、`S019-grok-output-alias-remediation-20261019.md` 与对应 independent audit。

## 2026-10-19 追加复核：Grok 上传器与估值计算器输出别名

当前包元数据346 manifest /347 checksum，sourceTree `b73dd98318f87e03a191176683aa4d5d8f06613754d91f2d3065098d60e60653`。G1 56 Skills/7 Agents，348审计树文件（20 transient），零闭包/哈希差异；G2 15项通过；adapter 81通过/25个过期 fixture 跳过；证据账本875条，缺失、过期哈希、重复ID均为0。

- **S019 Grok Reference Uploader：**合成图片与 mocked uploader 基线证明输出指向输入图片会 exit 0 覆盖图片；当前在任何上传/文件处理前拒绝同路径、规范化路径和 samefile alias。identity/path解析错误 fail-closed。Grok 两个 CLI 合计专项12/12、既有 helper 34/34，独立审计PASS；不发起网络请求。
- **S003 估值计算器：**合成股票数据和假设文件基线证明 `--output` 指向任一输入均会 exit 0 覆盖。现于读取 JSON 前检查两份输入，focused 6/6，独立审计PASS。两类输入原字节哈希在拒绝用例保持一致，独立报告输出成功。

真实硬链接受沙箱权限限制未实测（`samefile` 由mock覆盖），并发修改路径的TOCTOU窗口未测；自然路由、运行时来源和实时上游也未由此验证。S003/S019总体状态仍为PARTIAL。

## 2026-10-19 追加复核：S003 财务分析输入输出保护

S003 `financial_analyzer.py` 基线会在 `--output` 等于财报 JSON 时成功覆盖原文件。现已在加载/分析前检查规范化路径与 samefile，路径身份检查异常时拒绝；专项6/6、独立审计PASS，独立报告正常输出且失败时原输入SHA不变。真实硬链接和并发路径替换未测试。

当前包：347 manifest 文件 /348 checksum，sourceTree `969db2798fd97dbc335ed1900e1a22e0cb6fe25369184b47fd47ef9acd86fa98`；G1 56 Skills/7 Agents、349审计文件/20 transient、零闭包差异；G2 15项；adapter 81通过/25过期 fixture skipped；证据账本881条，路径和SHA均闭合。S003仍因自然路由、实时上游数据等未证保持PARTIAL。

## 2026-10-19 追加复核：S018 创业估值溢出边界

创业估值 CLI 对收入 `1e200`、增长率/回报率/倍数10000%、年限100等有界输入，基线曾成功产生 `Infinity`。现对组装后的全部估值报告递归检查浮点有限性，JSON 编码禁止NaN/Infinity并稳定捕获算术溢出。极端合成场景返回错误，普通输入仍产生有效 JSON；测试2/2，独立审计PASS。暂只保证 `generate_valuation_report()` 完整 CLI 路径，直接调用单个情景 helper未单独覆盖。

包状态：348 manifest/349 checksum，sourceTree `aa4ca7b3d450b35be37e198485fe815802b7cc90016be279591d44c199bbc410`；G1 56 Skills/7 Agents、350审计树文件/20 transient、无差异；G2 15项；adapter 81通过/25旧fixture跳过；证据账本888项闭合。真实上游与自然路由依旧未证，S018仍PARTIAL。

## 2026-10-19 追加复核：S027 市场情绪算术有限性

极端但逐字段有限的合成输入会让成交量比值和三类资金流总额溢出成 `Infinity`，原 CLI 仍成功输出。现对组装报告递归检查浮点有限性，使用 `allow_nan=False`，并在写报告文件前返回错误。量比溢出、资金流溢出、失败不写文件、正常行情有效输出4/4通过，独立审计PASS。输入SHA在失败后不变。该量级不代表真实行情，只用于边界检查。

最新包：349 manifest /350 checksum，sourceTree `15ff825d1388318b050a5aae7a8fa1b816db165590de0e6c4d3b2b816357c3ff`；G1 56 Skills/7 Agents、351审计文件/20 transient，零闭包差异；G2 15项；adapter 81通过/25跳过；证据账本895行全闭合。S027自然路由、真实宏观来源仍未测。

## 2026-10-19 追加复核：S003 DCF 计算边界

DCF 折现率≤-100%曾触发未捕获 ZeroDivisionError；合法折现率下极端但有限现金流又可把派生估值溢出为 Infinity。现在拒绝 `discount_rate <= -100%`，CLI额外捕获算术异常；完整估值报告递归检查浮点有限性并设置 `allow_nan=False`，报告落盘之前失败关闭。与估值输入输出别名保护回归共9/9，独立复核PASS；正常DCF仍输出有效JSON。

当前包：350 manifest/351 checksum，sourceTree `0f3e1eb8055187f7880d43a51470a944a078add204c0447a1fe2846ab1e51250`；G1 56 Skills/7 Agents、352审计树/20 transient，无差异；G2 15项；adapter 81通过/25过期fixture跳过；证据账本902条闭合。单独调 `dcf_valuation()` helper 不做递归结果检查，需走 `calculate_valuation()` 或CLI完整路径。


## 2026-10-19 追加复核：S049 资产分配器与 S018 基金金额换算溢出

S049 `asset_allocator.py` 的极端有限金额可派生 Infinity/NaN 且 CLI 报成功；输出还可覆盖审核策略模板。现已于读取模板前拒绝输出别名，递归检查完整方案数值并用 `allow_nan=False`。专项8/8、财富配置脚本全套18/18、独立审计PASS。

S018 `portfolio_builder.py` 的有限金额 `1e308` 配100%比例会派生 Infinity，原 JSON 编码允许写出非标准值。现检查每个派生金额且 JSON 序列化禁止 NaN/Infinity；专项9/9、Fund Portfolio全套14/14，独立审计PASS。新增正向 JSON stdout 与溢出时保留已有输出文件的断言。

最新包：351 manifest /352 checksum，sourceTree `be5d5838b81240344aec7e5f72081c63e2128a3dd65b4057c5ddf28dd52b9569`；G1 56 Skills/7 Agents、353审计文件/20瞬时排除，无闭包差异；G2 15项；adapter 81通过/25个 superseded fixture 跳过；证据账本现更新至918条并校验闭合。samefile由mock覆盖，真实NTFS硬链接及并发路径替换未测；自然路由、U盘runtime加载和真实上游不因这些 CLI 测试而获证，相关Skill保持PARTIAL。


## 2026-10-19 追加复核：S018 创业财务模型派生溢出

`venture-analysis/scripts/financial_model.py` 对极端有限增长率递推后曾以成功状态输出 Infinity/NaN；401位初始用户整数还会暴露原始 float 转换错误。现递归检查完整报告数值、JSON编码启用 `allow_nan=False`，初始用户转换复用有限值校验并给出稳定错误。focused 6/6、venture-analysis全套8/8，独立审计PASS；覆盖标准输出错误、无traceback、目标文件不创建/不覆盖、巨大整数错误和正常JSON标准输出/文件写入。

最新包：352 manifest /353 checksum，sourceTree `bfb7615df46b01f09bf5c2429916dbd6af5fedc9dc9e7367689cd57bba0451b5`；G1 56 Skills/7 Agents、354审计文件、无闭包差异；G2 15项；adapter 81通过/25 superseded fixture跳过；证据账本将更新至926条并校验闭合。所有场景为合成离线输入；自然路由、runtime加载和真实上游未由此验证。


## 2026-10-19 追加复核：S049 再平衡计划器派生溢出

`rebalance_planner.py` 对有限组合价值 `1e308` 和合法100%当前/目标配置，基线会退出0并输出 `Infinity`。现对完整再平衡计划递归检查有限数值，JSON启用 `allow_nan=False`，且计算/序列化均先于输出写入。focused 7/7、wealth-allocation全套21/21，独立审计PASS；覆盖新建与既有输出保护、正常标准输出/文件、源别名保护。

最新包：352 manifest /353 checksum，sourceTree `e3d9f340cb74985d9f2db6055eb35615c988f61f684135238f8a17bbf054c528`；G1 56 Skills/7 Agents、354审计文件、无差异；G2 15项；adapter 81通过/25跳过；证据账本预计935条。合成输入，无真实账户/网络。


## 2026-10-19 追加复核：S018 风险评估器输入输出别名

`risk_assessor.py` 基线允许 `--output` 覆盖 `--portfolio`，exit 0且源SHA变化。现于读入JSON前拒绝规范化路径和 samefile alias，身份检查异常 fail-closed；focused 6/6、Fund Portfolio全套20/20、独立审计PASS。真实NTFS硬链接由samefile mock代替，并发TOCTOU未测。

最新包：353 manifest /354 checksum，sourceTree `ceee120dbede655b3ae6910f73bbcf009c4286431efbe3dabd27fb469c4e60d9`；G1 56 Skills/7 Agents、355审计文件、无差异；G2 15项；adapter 81/25；证据账本预计943条。


## 2026-10-19 追加复核：S049 历史组合追踪派生溢出

`portfolio_tracker.py` 接受的有限历史 `[1, 1e308, 1]` 可使月收益及波动率派生 Infinity；基线 exit 0 并输出非标准JSON。现对完整报告递归有限值检查，编码设 `allow_nan=False`，在写文件前失败。focused 10/10、wealth-allocation全套25/25、独立审计PASS；历史/基准输出别名旧防护仍通过。

最新包：353 manifest /354 checksum，sourceTree `138af7562bc846b73e470bb27ef18fb6aecac6d87086b4f789307b6f953b0006`；G1 56 Skills/7 Agents、355文件、无闭包差异；G2 15；adapter81/25；证据账本预计951条。测试均用合成历史。


## 2026-10-19 追加复核：S045 旅行费用持久化数值边界

旅行费用 API 原接受负金额，导致已保存的支出总额变负、剩余预算虚增；两笔各自有限的 `1e308` 费用还可累计溢出并写入 `Infinity`。现要求费用为有限非负数，拒绝布尔值、缺失/非数值和过大整数；先校验旧记录、候选合计，再原子保存，摘要读取也拒绝损坏/非法历史金额。拒绝时数据库字节和调用方对象保持不变。负值及累计溢出两个旧行为均有基线复现；focused 5/5、Travel Planner 全脚本 suite 39/39、独立审计PASS。

最新包：354 manifest /355 checksum，metadata sourceTree `d30108276b83f87fc5b23eda0bea6d5e6ae3b1bf29bfdd9885e41afea4637180`；G1 56 Skills/7 Agents、356 audit-tree 文件且无闭包差异；G2 15项；adapter 81通过/25个被新合同取代的旧fixture跳过；证据账本962条，缺失、哈希过期、重复ID均为0。金额合同是 Python helper/数据库边界，尚未证明自然Skill路由、USB运行时加载或实时旅行服务。


## 2026-10-19 追加复核：S017 Fitness Coach 训练组边界

`log_workout()` 原先直接持久化负重复次数，60 kg × -10 reps 会让进度统计记录 `-600` 训练量、1RM约40。现对写入和查询的 workout/set 结构进行校验：重量有限且非负，次数有限、非负且为整数，布尔值和缺项拒绝；单组衍生指标、单次训练及全历史累计量均不得溢出。写入前校验且拷贝顶层记录，失败不改数据库或调用对象；列表、ID查找、历史、进度和个人纪录查询拒绝非法旧行。SKILL.md已写明此契约。

负重复旧行为由合成基线复现。focused 7/7、Fitness Coach完整脚本套件34/34，独立审阅和独立全套重跑PASS。当前包 G1 56 Skills/7 Agents、357 audit-tree files、零闭包差异；G2 15项；adapter 81通过/25旧fixture被superseded跳过；metadata 355 manifest/356 checksums，sourceTree `49953fa8408c96b5ba8ccbb87956197082437e87e47d2cb3ec0eab9dcec1cc67`；证据账本971条将复核闭合。此处仅证明Python API/本地数据库，不证明自然路由或USB运行时加载；非法旧记录被拒绝但不会自动迁移。


## 2026-10-19 追加复核：S007 历史周报习惯统计日期错位

基线重现：请求 Oct 5–11 历史周报时，旧代码按“今天往回7天”取习惯记录，报告返回 Oct 12 的未完成记录，并将该周完成率写成0%（所选周内预期100%）。现将显式起止日期贯穿习惯日志、单项统计和总状态；周报统计限定在所选周，当前连续天数按周末计算，历史连续记录可提供上下文；旧的默认 `days` 相对窗口保持兼容。日期严格要求补零 `YYYY-MM-DD`；新写入和旧记录查询都验证日期，CLI 对错误日期给简洁错误并以2退出，不输出traceback。

定向8/8、Daily Reflection全脚本13/13；独立审阅PASS并重跑13/13。当前包 G1 56 Skills/7 Agents、358 audit-tree files、零闭包差异；G2 15项；adapter 81通过/25 superseded fixture跳过；metadata 356 manifest/357 checksums，sourceTree `3e054041cc15ce7cfe1e23698be4279f788d317a49ea87abdabc0d36a0732674`；证据账本将闭合至981行。S007仍为PARTIAL：该轮仅证实源级API/CLI，不证明自然路由或U盘运行时加载；异常的历史日期记录 fail-closed，不自动迁移。

### Agent 源指令复查

2026-10-19 对 USB 包七份 Agent TOML 做第二轮只读检查（排除此前已修复的破坏性文件操作授权边界），未发现新的具体高风险源级缺陷。检查覆盖能力声明与指令边界、指向的包内Skill引用及fallback。运行时工具授权、自然路由、委派和执行行为仍未验证。

## 2026-10-19 追加复核：S003 股票数据缓存嵌套结构

`data_fetcher.py` 对缓存 `completeness` 为 null/list/string/int 时曾因链式 `.get()` 抛异常，导致provider不执行。现对缓存顶层与嵌套元数据做类型检查；畸形缓存视为miss，save_cache拒绝畸形状态。synthetic provider重建并替换损坏缓存通过。新回归3/3、China Stock Analysis脚本套件23/23，独立审计PASS；G1/G2/adapter/metadata全部通过；证据账本1038行全部闭合。实时AkShare和并发缓存替换未验证，整体Skill仍不得据此标为自然路由已验证。

## 2026-10-19 追加复核：S003 技术分析平价分类

60个恒价样本旧输出为下降趋势-3、RSI100和超买true。现均线斜率全零判横盘0，完全无涨跌RSI=50；严格涨跌保持，混合均线及价格/长均线交叉另有回归测试。定向5/5，S003全脚本28/28，独立审计PASS。G1 56/7且365审计文件零闭包差异，G2 15，adapter81/25跳过，metadata363/364；账本1046条全路径和哈希闭合。斜率采用严格比较无容差带，真实上游与自然路由仍未测。

## 2026-10-19 追加复核：S018 基金筛选非有限阈值

`fund_screener.py` 曾接受 NaN 最低收益率：比较结果为false，20%基金仍通过，JSON中出现非标准NaN。现1/3年阈值只接受有限的非bool int/float，校验在provider调用前；字符串、Decimal、NaN和正负无穷均拒绝；编码开启`allow_nan=False`。旧基线复现。首轮独立审查发现Decimal边界，修复后二轮PASS。定向2/2、Fund Portfolio全套22/22、CLI NaN错误包络通过。G1 56/7、366审计文件零差异，G2 15，adapter81/25跳过，metadata364/365；账本1055条无缺路径/哈希过期/重复ID。未测试真实基金上游或自然路由。

## 2026-10-19 追加复核：S018 Risk Assessor 超大整数

组合JSON中年化波动率=`10**400` 曾因float转换抛未捕获OverflowError并打印traceback。现捕获并返回字段级ValueError；CLI标准错误包络可解析，无traceback/输出文件，源输入SHA和字节不变。基线复现；focused2/2、Fund Portfolio全套24/24、独立审计PASS。G1 56/7、367审计文件无闭包差异；G2 15、adapter81/25跳过、metadata365/366；账本1063条路径和hash闭合。当前证据只覆盖合成JSON，不覆盖真实用户组合或自然路由。

## 2026-10-19 追加复核：S003 realtime_quote 分时总量与时段桶

旧代码09:25集合竞价进入总量而没有对应分桶；合成09:25/09:30/14:59记录得总量300、桶合计200、百分比66.6666%。现使用五个不重叠左闭右开时段，集合竞价单列，午休及交易时段外排除于总量与桶。旧收盘桶已含14:59，缺口实际来自09:25。聚焦3/3、S003全套31/31，独立PASS。G1 56/7/368 audit files零差异；G2 15、adapter81/25跳过、metadata366/367；账本1071条闭合。真实新浪时间戳格式仍未验证。

## 2026-10-19 追加复核：S030 Meal Planner 报告输出失败一致性

旧CLI先写活动计划、再打开 `--output`。合成基线中输出父目录缺失使CLI退出1但留下计划；重试后计划从1增至2。现先在目标目录同级完整暂存/fsync，再写计划并原子替换报告；报告发布失败会按ID补偿回滚；若补偿失败，非零错误包含具体原因并要求先核查残留计划。输出父目录失败的计划数据库状态及其他数据哈希都保持不变；成功报告与计划记录一致。Meal Planner全套22/22，独立审计和独立重跑均PASS 22/22。包门禁：G1 56 Skills/7 Agents、368 audit files/20 transient、零差异；G2 15；adapter81/25；metadata366/367，sourceTree `fc0fd6b511ad44fb6f2120cc7d3c904bc88dfa65799d85b4569d48aa923c38a0`；证据账本1079条路径存在、SHA和ID均闭合。进程崩溃处于DB提交与报告replace之间时无法执行补偿；自然路由/USB runtime不因此视为已验证。报告见 `S030-meal-output-failure-remediation-20261019.md`。

## 2026-10-19 追加复核：S003 stock_screener 严格 JSON

旧实现的展示字段会让离线筛选报告输出 JSON 标准不接受的 NaN 字面值。现可选数值缺失/NaN/±Infinity写为null，非有限排序行排除；CLI JSON 编码严格拒绝残余非有限值，并在成功序列化前不打开输出文件。Focused4/4，China Stock Analysis全套35/35，独立审计且独立suite复跑PASS。G1 56/7、369 audit files/20 transient、零闭包/哈希差异；G2 15；adapter81/25；metadata367/368，sourceTree `add68f9cf32031a0ec7945d5fd76db5517d9d9f9a3524764d4f128845ae4b506`；证据账本1089条路径和哈希存在、唯一ID无重复。未验证真实 AkShare 的 NaN 频率、自然路由或USB runtime来源；S003总体仍PARTIAL。


## 2026-10-19 追加复核：S003 technical_analysis 非有限 K 线输入

旧实现会接受 K 线序列中的 NaN，成功返回并生成非标准 JSON。现对 close/high/low/volume 在取数返回边界及直接指标入口拒绝非数值与 NaN/±Infinity，并报告字段和1-based行号；JSON CLI 严格编码，失败不创建或覆盖目标文件。合成基线复现，focused4/4、China Stock Analysis全套39/39，独立PASS并独立重跑。G1 56 Skills/7 Agents零闭包/哈希差异；G2 15、adapter81/25、metadata368/369，sourceTree `9f30e42e719e0a6ba155a1a15f06af38e7429b8da9d5ac0a4574e711e68f0adc`；账本1099条零缺失、重复或过期。真实AkShare、自然路由和USB runtime仍未验证；S003总体PARTIAL。详见 `capability-audit-evidence-20261018/S003-technical-nonfinite-remediation-20261019.md`。

## 2026-10-19 追加复核：S017 Fitness Coach Program Generator 输出失败

旧CLI在计划入库后才打开`--output`；缺失父目录导致退出1但计划留存，重试重复创建。现先stage/fsync报告，DB写入后再原子替换；发布失败恢复原始计划库字节。基线0→1→2已复现；focused3/3、Fitness Coach全套37/37，独立审计PASS并独立复跑。G1 56/7、371 audit files/20 transient，零差异；G2 15、adapter81/25、metadata369/370，sourceTree `719a932a6324ef6a92cc3864f11fe15aef3aefbc4c96dc9f1e301a2bc920f00f`；账本1110条闭合。强杀进程、并发数据库写者和rollback失败注入未测；自然路由/USB runtime仍未验证。详见`capability-audit-evidence-20261018/S017-program-output-failure-remediation-20261019.md`。

## 2026-10-19 追加复核：S037 PDF 合并写失败保护

旧merge先`wb`截断已有目标，writer部分写后抛错会破坏旧文件；合成基线已复现。现先在同目录暂存写入、flush/fsync、关闭并确认非空，再原子replace；失败清理暂存、旧输出字节不变。Focused2/2，PDF套件8通过/1硬链接因WinError5跳过，当前PDF smoke通过；独立审计另验证双输入合并后2页、文本顺序。G1 56/7、372 audit files/21 transient、零差异；G2通过，adapter81/25，metadata370/371，sourceTree `7927b4c7714992e848933d0cae89b58998889157874f89233cbbe894bcd6f749`；账本1122条闭合。断电/强杀、硬链接权限、ACL元数据保留、任意PDF和自然路由未覆盖；S037总体PARTIAL。详见`capability-audit-evidence-20261018/S037-pdf-merge-write-failure-remediation-20261019.md`。

## 2026-10-19 追加复核：S045 Travel Planner 嵌套字段形状

旧CLI对`destination:null`、`destination:[]`和`budget:null`均泄漏AttributeError traceback；隔离基线已复现。现对trip_data/destination/budget要求Mapping，返回字段级ValueError，经CLI简洁处理，不生成输出；有效Mapping结果保持。Focused4/4、Travel Planner全套43/43，独立审计PASS并独立复跑。G1 56/7、373 audit files/21 transient零差异；G2通过，adapter81/25、metadata371/372，sourceTree `89a0bf2e844921ff1b8615e1c9fae2a919d86451ba814fe547329664d4563e57`；账本1133条闭合。顶层非Mapping未单独测CLI；真实出行源、自然路由与USB runtime未测。总体状态仍PARTIAL。详见`capability-audit-evidence-20261018/S045-malformed-trip-nested-fields-remediation-20261019.md`。

## 2026-10-19 追加复核：S052 Web Content Fetcher max_chars边界

401字符模拟正文下，旧实现max_chars=-1静默返回400字符、0返回空正文；现要求正整数，API拒绝bool/非整数/非正值，CLI在导入依赖和抓取前简短报错。正数精确截断保留。Focused3/3、全套10/10、独立审计PASS。G1 56/7、374 audit files/24 transient无差异；G2通过，adapter81/25、metadata372/373，sourceTree `2e9b1a77f0f0e88920df49750c94f516b329c0b7f5455bc95a549fdaa6749060`；账本1144条闭合。仅模拟Fetcher/转换器，真实网页、自然路由和USB runtime未测；总体仍PARTIAL。详见`capability-audit-evidence-20261018/S052-max-chars-boundary-remediation-20261019.md`。

## 2026-10-19 追加复核：S047 尽调清单输出持久性

旧直接写入在partial-write OSError后已把哨兵目标改成部分Markdown，基线复现。现以同目录暂存、flush/fsync后原子替换；失败清理stage并保留旧内容。Focused3/3、Venture Analysis全套16/16；独立审计通过并额外验证replace失败保持旧hash。G1 56/7、375 audit files/31 transient无差异；G2通过、adapter81/25、metadata373/374，sourceTree `52c7fad5568d64595bd808e9cd0a9504c535a9af3088a4a7ea0ac6fef47f141a`；账本1155条闭合。真实磁盘故障/断电/强杀、目录fsync、自定义ACL或文件元数据保留未测；S047总体PARTIAL。详见`capability-audit-evidence-20261018/S047-due-diligence-output-failure-remediation-20261019.md`。

## 2026-10-19 追加复核：S054 封面扩展名失败副作用

请求`new-folder/cover.txt`时旧逻辑先创建目录再报ValueError，基线复现。现入口先验证后缀并在渲染/建目录前拒绝；jpg/JPEG/PNG成功并可重开。Focused2/2、WeChat Article Creator全套14/14，独立审计PASS。G1 56/7、376 audit files/36 transient无差异；G2通过、adapter81/25、metadata374/375，sourceTree `f18b58e4e78a3dbc2eb1fd33a2c8485d6d89a8fc68d5023f76bc58dd2ecfebb9`；账本1166条闭合。仅本地Pillow，真实路由与USB runtime未测；整体PARTIAL。详见`capability-audit-evidence-20261018/S054-cover-extension-side-effect-remediation-20261019.md`。

## 2026-10-19 追加复核：S003 data_fetcher 超大数值转换

fake provider返回总市值`10**400`时旧safe_float泄漏OverflowError；现将OverflowError与其他非法/非有限可选字段统一转为None，保留剩余基本信息。Focused3/3、China Stock Analysis全套42/42，独立审计PASS。G1 56/7、377 audit files/37 transient零差异；G2通过，adapter81/25，metadata375/376，sourceTree `9889740c02991d9cc23cf03f8c44214ce7c6b1204bd7f6ab3da8ab9cfeb8ed7e`；账本1177条闭合。只测fake provider；AkShare真实字段、自然路由和USB runtime未验证。S003整体PARTIAL。详见`capability-audit-evidence-20261018/S003-data-fetcher-overflow-remediation-20261019.md`。

## 2026-10-19 追加复核：S030 Meal Planner BMR gender字段

gender:null的已初始化数值档案旧CLI在gender.lower()泄漏AttributeError，基线复现。现要求非空且规范化后为male/female，未知值给字段级ValueError，不默认套用公式。Focused3/3、Meal Planner全套25/25，独立审计PASS；male/female公式值保持1618/1452。G1 56/7、378 audit files/37 transient无差异；G2通过、adapter81/25、metadata376/377，sourceTree `41a5e3ec15b92c95dfe448a3a79544eaa145d62084abcca7f5fa78e440b09e84`；账本1188条闭合。仅合成档案，营养/医疗准确性与自然路由未测；S030总体PARTIAL。详见`capability-audit-evidence-20261018/S030-null-gender-bmr-remediation-20261019.md`。

## 2026-10-19 追加复核：S030 Meal Planner 布尔型身体数据

已初始化档案中的`age=true, height_cm=false, weight_kg=false`使旧CLI把宏量营养素总热量算为0，随后泄漏ZeroDivisionError；隔离基线复现。现要求年龄、身高、体重为有限正实数并排除bool；派生BMR/TDEE/目标热量和宏量营养素分母也需有限且为正。Focused5/5、Meal Planner全套30/30，独立只读审计PASS；有效档案营养目标保持，超大整数、BMR/TDEE溢出均有固定回归用例。G1 56/7、379 audit files/37 transient零闭包/哈希差异；G2通过，adapter81/25，metadata377/378，sourceTree `b6e8088c9f2f61ddd3057484958b65184a58e83b0ca704ed49a011d7d1e4857f`；账本关闭至1200条。仅合成档案；营养临床准确性、自然路由及USB runtime仍未验证，S030整体PARTIAL。详见`capability-audit-evidence-20261018/S030-bool-profile-nutrition-remediation-20261019.md`。

## 2026-10-19 追加复核：S054 同标题草稿文件名碰撞与写失败

冻结时间下，同标题两次调用原先覆盖首稿；第二个基线在新文件独占创建后注入写失败，留下50字节残稿。现用独占创建并以`_2`等后缀重试，写/关闭失败只清理由本次创建的残稿；CLI返回码1并给简洁错误。独立审计首轮发现残稿问题，修复后复审PASS；focused3/3、WeChat Article Creator全套17/17。G1 56/7、380 audit files/37 transient零差异；G2通过、adapter81/25、metadata378/379，sourceTree `f4c1dadd3d6253b951c90401fae3f748c1fc87fc5ea33a0eb6e396b9ed83a5aa`；账本将闭合至1214行。清理本身失败且原异常为非OSError时残稿路径未必附加；未跑真实并发压力。仅证明源码辅助函数，不证明自然路由/USB runtime；S054整体PARTIAL。详见`capability-audit-evidence-20261018/S054-draft-collision-remediation-20261019.md`。

## 2026-10-19 追加复核：S051 Weather Forecast 配置写入失败保护

基线对隔离配置注入部分`Set-Content`写入并抛IOException，旧文件SHA变化且JSON解析失败。现先序列化到同目录GUID暂存文件，再对既有配置用`File.Replace`、新配置用`File.Move`；失败清理暂存/备份。PowerShell5.1和7均通过：成功set/get、暂存部分写失败、目标锁定替换失败、旧配置字节/SHA保持、无残留暂存。G1 56/7、381 audit files/37 transient零差异；G2通过、adapter81/25、metadata379/380，sourceTree `16c55e13fdd06587b804447ce43bc82326558dfbbd589097aa0e764bc6962d0c`；独立审计待归档后账本将闭合至1226条。测试仅用隔离配置，无天气服务或网络；自然路由/USB runtime未验证，S051仍PARTIAL。详见`capability-audit-evidence-20261018/S051-weather-config-failure-atomicity-remediation-20261019.md`。

## 2026-10-19 追加复核：S037 PDF generate() 写失败保护

旧生成器直接打开目标文件；fake Canvas 将已有哨兵截断、写入PDF前缀后抛OSError，基线已复现。现同目录暂存渲染，save完成后检查非空与`%PDF-`签名，再`os.replace`；失败清理暂存并保留旧目标。Focused2/2、PDF脚本全套9通过/1硬链接因WinError5跳过；真实本地生成/重开/抽取/合并smoke通过，独立审计PASS。G1 56 Skills/7 Agents、383审计文件/37 transient零清单/哈希差异；G2无告警/错误，adapter81/25跳过，metadata381/382；sourceTree `3c2922b8fb08f18babd3e3adc67368ab482d45cf4872426cd1f1738ae2dad92b`；证据账本1242行待最终哈希闭合。Fake renderer为主；强杀/断电、ACL元数据、任意PDF兼容、自然路由和USB runtime未证，S037仍PARTIAL。

## 2026-10-19 追加复核：S047 Business Evaluator oversized integer

401位`market.tam`整数使旧`float(value)`泄漏OverflowError traceback，隔离CLI基线复现。现`_number()`捕获OverflowError并沿用字段级ValueError；CLI exit1、stderr提示有限数值错误，无traceback/stdout/输出文件；有限值通过。Focused3/3、Venture Analysis全套19/19，独立审计PASS。包级G1/G2/adapter/metadata与S037共用本轮验证并通过；账本1242行待finalize。未调用网络/真实投资数据；自然路由/USB runtime/投资适用性未证，S047仍PARTIAL。

## 2026-10-19 追加复核：S045 trips.json 顶层结构校验

合法JSON数组`[]`作为trips数据库顶层时，旧`get_trip_by_id()`抛出TypeError traceback；隔离CLI基线复现，输出未创建、preferences/trips哈希不变。现`load_json()`拒绝非对象JSON顶层并给文件名级ValueError；CLI查找错误简洁返回exit 1。Focused2/2、Travel Planner全套45/45，独立审计PASS；mapping有效输入仍生成计划。G1 56/7、384审计树文件/37 transient零差异；G2无警告错误，adapter81/25跳过，metadata382/383；sourceTree `fd929011b0280b8c916816b21855ec58f4a5612f4c2409e87f89f9f8e35c6c1d`。证据账本1252条待最终哈希校验。内部列表形状完整性、自然路由与USB runtime仍未证，S045总体PARTIAL。

## 2026-10-19 追加复核：S027 cycle allocation 超大整数

401位allocation权重使旧`float(value)`抛出OverflowError traceback，隔离基线复现；现归一为字段级有限数值错误。CLI exit1，无traceback/stdout/输出，输入规则hash不变；有限40/60仍接受。Focused2/2、Macro Research全套16/16，独立审计PASS。G1 56/7、385审计树/37 transient零差异，G2无告警错误，adapter81/25跳过，metadata383/384；sourceTree `a007241c3107220f7f5584001f21e2a34c8d2ccad3dabd810c0d82e9bc710a9a`。证据账本1262条待哈希闭合。真实宏观数据、自然路由、USB runtime未测；S027仍PARTIAL。

## 2026-10-19 追加复核：S003 分时成交量非有限输入

fake provider返回`volume="1e10000"`时，旧`int(float(...))`泄漏OverflowError且JSON CLI无错误envelope；基线复现、无网络。现先校验有限非负数再转整数，异常由既有CLI形成稳定JSON错误。Focused4/4、China Stock Analysis全套46/46、独立审计PASS；0和正常正数有效，原有交易时段桶测试保持通过。G1 56/7、386审计树/37 transient零差异，G2 clean，adapter81/25，metadata384/385；sourceTree `a0312956af098f3618ec5bb5efaf97247ea43befa5f3c95d074f2a805d60a040`。证据账本1272行待哈希闭合。真实Sina响应、自然路由与USB runtime仍未验证，S003总体PARTIAL。

## 2026-10-19 追加复核：S003 分时成交额聚合溢出

两个有限`amount_cny=1e308`相加后旧CLI exit0、success=true并输出严格JSON不接受的`Infinity`；全mock基线复现。现汇总后校验有限性并以稳定错误envelope/exit1拒绝；正常12.5+7.5=20及成交量桶通过。Focused2/2、China Stock全套48/48、独立审计PASS。G1 56/7、387 tree/37 transient零差异；G2 clean，adapter81/25，metadata385/386；sourceTree `f63b39b1a2fb6be6d0bb6d1cf503496c5b942a0446105cfab5c18a5cd76f7454`。证据账本1282条待最终哈希闭合。真实新浪响应、自然路由、USB runtime仍未测；S003总体PARTIAL。

## 2026-10-19 追加复核：S045 持久化行程记录形状

顶层对象的`current_trips:[null]`使旧查找CLI泄漏AttributeError；隔离基线复现，库hash不变、报告不存在。现按section检查trip row为JSON object后再读取ID，返回文件/section错误。Focused2/2、Travel Planner全套47/47、独立PASS；有效行程仍出计划。G1 56/7、388 audit tree/37 transient零差异，G2 clean，adapter81/25，metadata386/387，sourceTree `077ede912405e5ab040d14d6c8630382c3560150e8a3cb3c416b2915dd360f7e`。证据账本1292条待哈希闭合。其他变更路径、自然路由和USB runtime未全证；S045总体PARTIAL。

## 2026-10-19 追加复核：S030 纯素食材过滤

旧filter忽略diet_type，固定纯素向量返回茶叶蛋、鸡蛋、鲈鱼；基线复现。现整道排除含已知动物性别名配方，要求配料详情；无安全项明确失败，不删食材；regular结果不变。Focused4/4、Meal Planner34/34、独立PASS。当前中餐早餐库无安全纯素食谱，纯素日计划仍无法成功生成；将此列为剩余可用性缺口。G1 56/7、389 tree/37 transient零差异，G2 clean，adapter81/25，metadata387/388；sourceTree `7e86b62e807e01c430d68872d1346c349a604cf1ecf6bc4be43515f65cc9b561`。证据账本1302条已finalize并验证：唯一ID、路径与SHA均无缺失/过期。Keto/低碳/素食语义未实现，临床适用性、自然路由与USB runtime未测。


## 2026-10-19 追加复核：S030 纯素餐单目录可用性

增加的 vegan-only 中文早餐、午晚餐、加餐候选逐项深拷贝既有 RECIPE_DATABASE 食材记录，未改营养数值。默认三餐与四餐用例均生成安全计划；regular 选择保持原样；无安全项继续 fail-closed。Focused5/5、Meal Planner全套35/35、独立行为审计PASS；包G1/G2/adapter/metadata通过。账本1310条已finalize+verify，唯一ID且路径/hash无缺失或过期。重要限制：抽查示例三餐合计约1109 kcal/31g蛋白，远低于测试示例目标2000 kcal/100g，营养目标适配未证；单一早餐/加餐重复，菜系覆盖有限，S030总体PARTIAL。详见capability-audit-evidence-20261018/S030-vegan-availability-remediation-20261019.md。

## 2026-10-19 追加复核：S049 Portfolio Tracker 超大整数

历史JSON中value=10**400基线使_float转换泄漏OverflowError traceback，报告不存在、输入hash不变。现转换边界捕获OverflowError并给字段级有限数值错误；CLI JSON error envelope，无traceback/报告，输入不变，有限历史仍成功。Focused3/3、Wealth Allocation全套28/28、独立审计PASS；包G1/G2/adapter/metadata通过。账本1321条已finalize+verify，唯一ID且路径/hash均有效。未验证自然路由、USB runtime、真实历史或投资适用性；S049总体PARTIAL。详见capability-audit-evidence-20261018/S049-portfolio-tracker-large-int-remediation-20261019.md。

## 2026-10-19 追加复核：S019 full_body 显式 off

基线证明 `_optional_bool("off")` 返回 True，错误插入全身构图提示和 crop QA。现标准化 on/true/yes/1 为 True，off/false/no/0 为 False，未知字符串为 None。Focused 4/4、Grok scripts 16/16、prompt compiler 29/29、reference upload 5/5（fake session），独立审计 PASS。G1/G2/adapter/metadata 通过；证据账本 1334 行，finalize/verify 均零缺失或过期。自然路由、模型生成与 USB runtime 未测，S019 总体 PARTIAL。详见 `capability-audit-evidence-20261018/S019-grok-off-token-remediation-20261019.md`。

## 2026-10-19 追加复核：Fund Screener 部分写入保护

冻结基线在写入19字节后抛错，退出码1但旧报告SHA从`cc6c4…e3f2f`改变为`9a75f9…322f9`。现采用同目录暂存、flush/fsync/close后`os.replace`；失败保留旧报告并清理stage，成功新建/替换均产生可解析完整JSON。S018阈值校验仍早于provider调用。Focused3/3、Fund Portfolio全套27/27、独立审计PASS；G1/G2/adapter/metadata通过；证据账本1345行唯一ID且路径/hash无缺失/过期。无网络/AkShare；自然路由与USB runtime未测。详见`capability-audit-evidence-20261018/fund-screener-write-failure-atomic-remediation-20261019.md`。

## 2026-10-19 追加复核：Web Content Fetcher 缺可选依赖

缺少 html2text/Scrapling 时旧 CLI 泄露 RuntimeError traceback。现给出依赖安装和 Codex 原生网页读取提示，非零退出、无 traceback/输出文件；mock正常提取仍遵守 max_chars=37。Focused3/3、全套13/13（包含S052懒加载7项）、独立审计PASS；包级G1/G2/adapter/metadata通过。账本1356条唯一ID、路径/hash全有效。全程mock/.invalid，不证明真实网页质量或USB runtime。详见`capability-audit-evidence-20261018/web-fetch-missing-dependency-remediation-20261019.md`。

## 2026-10-19 追加复核：Venture Valuation 输入/输出别名

基线将 assumptions 与 output 指向同一路径，CLI exit0并覆盖输入。现 direct/normalized/samefile alias 在读输入前拒绝，distinct output仍生成可解析JSON；输入hash不变。Focused4/4、Venture Analysis23/23、独立审计PASS；G1/G2/adapter/metadata通过，账本1367条全闭合。真实硬链接因沙箱权限无法创建，samefile路径由mock覆盖。未测真实企业数据、自然路由、USB runtime。详见`capability-audit-evidence-20261018/venture-valuation-output-alias-remediation-20261019.md`。

## 2026-10-19 追加复核：S027 sentiment_monitor 输入/输出别名

合成输入/输出同路径基线会覆盖源数据、exit0并报告成功。现直接、规范化及samefile alias在读输入/生成报告前拒绝；nonzero无成功提示，输入hash保持，独立输出有效JSON；S027既有finite guard回归仍通过。Focused4/4、Macro Research全套20/20、独立审计PASS；包级G1/G2/adapter/metadata通过；证据账本1378行全闭合。samefile mock覆盖，真实硬链接/符号链接未测；自然路由和USB runtime未测。详见`capability-audit-evidence-20261018/sentiment-output-alias-remediation-20261019.md`。

## 2026-10-19 追加复核：Skills Search installed-layout fallback

冻结布局复现脚本错误搜索`.codex/skills/skills`，未找到旁邻travel-planner。现fallback根定位修正；PS5.1与PS7.6.5均通过：邻接Skill返回`unknown`、无匹配安全返回、package root仍保留`verified`；两个脚本都有UTF-8 BOM。包级G1/G2/adapter/metadata通过，账本1389条全闭合。旧S043 smoke仅PS5.1中文断言受无BOM harness影响，新测试两引擎均通过；实际Codex app路由/runtime未证。详见`capability-audit-evidence-20261018/skills-search-installed-layout-remediation-20261019.md`。

## 2026-10-19 追加复核：S003 Financial Analyzer 报告写失败保护

冻结基线在部分写入后抛错，把已有哨兵报告截成`{"partial":`。现同目录暂存后原子替换；写入或替换失败均保留旧报告及输入hash、清理stage、非零返回且不宣称保存成功；成功输出严格JSON。新focused3/3、既有别名+原子用例独立复核9/9、China Stock Analysis全套51/51、独立审计PASS；包级G1/G2/adapter/metadata通过；账本1400行全闭合。全为合成数据，无provider/网络，断电/真实磁盘故障未测。详见`capability-audit-evidence-20261018/financial-analyzer-atomic-remediation-20261019.md`。

## 2026-10-19 追加复核：S027 policy_analyzer 文本类型

基线把object/list字段`as_of/policy_type/sources`转成Python repr后exit0成功。现所有文本槽位只接受非空字符串，列表元素也逐项验证并给字段级错误；有效合成文本仍成功，原alias guard保持。Focused3/3、Macro Research全套23/23、独立审计PASS；审计另检查13个文本槽空白和3个集合字段形状；包G1/G2/adapter/metadata通过，账本1411条全闭合。未验证政策事实真伪、自然路由或USB runtime。详见`capability-audit-evidence-20261018/policy-text-shape-remediation-20261019.md`。

## 2026-10-19 追加复核：S019 reference-upload HTTPS 重定向降级

合同要求远端重新验证必须全程 HTTPS 才允许 `verified=true`。旧实现仅校验初始URL，允许跟随 HTTP 降级；mock基线证明其会错误地接受匹配内容并标记 verified。现对每个 history hop 与最终 response URL 验证绝对 HTTPS，再继续状态/MIME/体积/SHA检查。Focused7/7、Grok全套16/16，独立审计重跑PASS；G1/G2/adapter/metadata及`git diff --check`通过，sourceTree `adde286d139399f9b1ae153ef44e645ba9c1edfb796ed1c52d9d2eba37a2b826`。全程fake requests，无网络；不证明Uguu当前存在降级跳转。自然路由与USB runtime未证，S019仍PARTIAL。详见`capability-audit-evidence-20261018/S019-reference-upload-https-redirect-remediation-20261019.md`。

## 候选合同裁决：S049 Portfolio Tracker 规范化基准名冲突

`strip()` 后出现同名 benchmark 会导致后写值覆盖先写值，但 Skill 未明确规定名称需唯一或如何规范化，故这是建议性数据完整性合同，不列为已确认违约。基线与独立只读裁决见`capability-audit-evidence-20261018/candidate-portfolio-tracker-normalized-benchmark-collision-*`。

## 2026-10-19 追加复核：S045 严格出发日期

无效日期不再被替换为“现在+30天”；行前时间线与完整计划都要求有效日历日期格式`YYYY-MM-DD`，并在计算或读取偏好前拒绝无效、缺失、非字符串及带时区timestamp。Focused4/4、Travel Planner54/54、独立审计PASS；新增CLI子进程证明坏日期返回字段错误且无traceback/stdout/报告，空白/null/数字独立复测的数据库SHA不变。最终包级G1/G2/adapter/metadata/diff-check均已刷新通过（56/7，408/409，84通过/25预期跳过），sourceTree `8624020f048c3da5f7684234f7581270735693bcda99896bb7e8426f9c049c48`。无网络，临时数据在repo tmp。自然路由/USB runtime未测。详见`S045-departure-date-remediation-20261019.md`。

## 2026-10-19 追加复核：S030 购物清单过敏过滤与结构校验

购物清单现用计划自己的限制快照和共用别名规则过滤，中文/英文花生项不再入单；全计划嵌套结构与数量先校验后聚合，坏数据不输出部分结果或traceback。Focused11/11、Meal Planner56/56、两轮独立审计（首轮发现并推动修复days:null CLI traceback，第二轮PASS）。包级G1/G2/adapter/metadata通过（56/7，408/409，adapter83/25）。合成数据，无网络；自然路由和USB runtime未测，S030总体PARTIAL。详见`S030-shopping-list-restrictions-remediation-20261019.md`。

## 2026-10-19 追加复核：S003 部分结果需要显式 opt-in

按 Skill 中“希望保留部分结果时必须显式加`--allow-partial`”要求，CLI在无开关的partial情形不再落盘、输出JSON或提示保存成功；只在stderr列组件错误并exit1。带开关可保存partial并成功；complete路径保持，failed不会写空似结果。Focused6/6、China Stock Analysis62/62、独立审计PASS。合成provider无网络；包级G1/G2/adapter/metadata通过，sourceTree `7c9725950d2ff61cf792e3d278d5f1b3b3cc3a0cc1098e838484f92275f667fb`。需注意旧CLI help原先把flag描述为仅控制exit code；现已与Skill保留语义对齐。详见`S003-partial-result-opt-in-remediation-20261019.md`。

## 2026-10-19 追加复核：Feishu MCP Node.js floor

MCP清单要求Node.js >=22，但wrapper/config renderer原先接受>=20。现所有启用门槛统一>=22。Focused2/2、adapter83 passed/25 legacy skips，独立复核PASS；fake Node 20.19拒绝、22/24接受，拒绝时预置配置字节不变；PowerShell5.1解析与UTF-8 BOM通过。G1/G2/metadata package gates均通过。未启动MCP package或触网，真实Feishu启动/OAuth未验证。详见`feishu-node-22-floor-remediation-20261019.md`。

## 候选合同裁决：S003 data_fetcher partial persistence

复现“无`--allow-partial`仍落盘”与Skill要求相冲突，但CLI原help曾把开关限定为退出码，因此原候选置信度中高而非无歧义。按Skill现有显式“保留”语义完成修复并同步help，不需改变合同文本；见`S003-partial-result-opt-in-remediation-20261019.md`。

## 候选合同裁决：Feishu Node floor

已确认 manifest >=22 与 wrapper >=20 不一致，修复后通过离线验证。候选基线文件保留在`candidate-feishu-node-floor-contract-20261019.*`。

## 2026-10-19 追加复核：S052 Web Content Fetcher 截断提示

旧 helper 只返回截断后的正文，CLI 无法提示内容被 `max_chars` 截断；修复后 helper 返回正文、选择器和截断标记，CLI 仅在确实截断时说明正文上限。Focused5/5、CLI/依赖3/3、全套15/15，独立行为审计PASS。G1 56/7、405 manifest files/406 checksums闭包通过；G2 Windows PowerShell 5.1 16文件通过；adapter81/25、metadata --check及`git diff --check`通过。账本1420行已finalize/verify，missing/stale均为0。Fake fetcher/.invalid，无网络；选中正文恰好等于上限的单独用例仍可补充；真实网页、自然路由和USB runtime未测，S052仍PARTIAL。详见`capability-audit-evidence-20261018/web-fetch-truncation-disclosure-remediation-20261019.md`。

## 2026-10-19 候选合同裁决：S037 PDF extract 部分写入

隔离注入显示 extract 写入失败会将既有目标截断为部分内容，但现有 PDF Skill 文档未承诺输出原子替换；CLI 返回错误、未报成功，故不列为已确认合同违例。保留为建议增补的失败路径完整性合同，除非后续接受该合同，不按明文违约统计。基线与独立审阅见`capability-audit-evidence-20261018/candidate-pdf-extract-partial-output-*`。


## 2026-10-19 追加复核：Painpoint Analyzer 示例合同

发现示例断言“伪需求”并编造 CAC，与“先收集五项事实、信息不足不分级/不给方案”冲突。示例现列出五项完整假设输入，声明分析仅为条件推演，CAC 标为无实测并要求真实花费/转化后再计算。Focused1/1，adapter84/25跳过，独立内容审查PASS。最终包级G1/G2/metadata/adapter/diff检查通过，sourceTreeSha256 `8624020f048c3da5f7684234f7581270735693bcda99896bb7e8426f9c049c48`。自然路由、USB运行与真实用户验证未证；整体仍PARTIAL。详见`capability-audit-evidence-20261018/painpoint-analyzer-example-contract-remediation-20261019.md`。


## 2026-10-19 剩余源合同只读扫描

覆盖近期修复范围之外的文本型 Skill 与本地脚本/模板/资源引用；未找到新的强合同违例。S053 `max-words` 仍属证据不足的提示词软约束疑点，不升级为缺陷。扫描不证明自然路由、USB运行时或Agent子代生命周期。详见`capability-audit-evidence-20261018/remaining-contract-scan-20261019.md`。


## 2026-10-19 候选闭环：S037 PDF extract 部分写入

旧基线在写入部分目标后失败会截断既有输出；现同目录暂存、fsync后发布，fdopen/写入/替换失败均清理stage并保留旧文件，清理错误不覆盖原始错误。Focused4/4，独立二审PASS（PDF suite10/1 skip、smoke PASS）。新增测试已纳入包；最终G1/G2/adapter/metadata/diff-check通过，sourceTree `b7eb0ac9e555216a823d097098cc73579a6b22be35d0697db400a083645f4cec`。hardlink测试因ACL skip；真实磁盘故障、掉电与USB runtime未测。详见`capability-audit-evidence-20261018/S037-pdf-extract-atomic-output-remediation-20261019.md`。


## 2026-10-19 候选闭环：S049 Portfolio Tracker 基准名碰撞

`{"Index":5," Index ":99}`旧行为去空格后静默只保留99；现trim后重名拒绝，不同名称结果保持，CLI失败不触碰既有输出。Focused2/2，独立PASS；Wealth scripts30/30，adapter84/25；G1/G2/metadata/diff检查通过，sourceTree `c9c14634d2e084a85f74aea93b254a8e69afad397aab8885367934909922f1e3`，410/411包清单。合成数据，真实源/自然路由/USB runtime未测。详见`capability-audit-evidence-20261018/S049-portfolio-tracker-benchmark-name-collision-remediation-20261019.md`。


## 2026-10-19 追加复核：S052 Scrapling fetchers runtime closure

发现USB Python hashed lock只包含 `scrapling==0.4.15` 基础包，而源脚本导入 `scrapling.fetchers.Fetcher`；隔离profile因此在页面请求前因缺少 Playwright 模块失败。按官方依赖边界，将USB direct input改为 `scrapling[fetchers]==0.4.15`，重新物化并合并9个新wheel，保留原有锁定版本及Manim wheels；总计86 wheels / 246,323,935 bytes。纠正Skill/README：当前HTTP Fetcher无需安装浏览器；移除已弃用且无效的 `Fetcher(auto_match=False)` 参数。旧哈希锁 dry-run与新离线profile实际安装均通过；15项包测试、8项依赖合同、loopback本地HTTP正文/图片链接smoke全通过。外网页面未请求，natural routing及USB provisioner lifecycle仍未证；S052 PARTIAL。证据：`capability-audit-evidence-20261018/S052-scrapling-runtime-dependency-remediation-20261019.md`、`S052-focused-package-tests-20261019.log`、`S052-runtime-extra-contract-20261019.py/.log`、`S052-offline-lock-resolution-20261019.log`、`S052-provisioned-http-fetch-local-smoke-20261019.py/.log`。


## 2026-10-19 Media MCP 1.3.11 live 图像/视频 smoke

只读审查认为“请求可能已送达但回执不确定时标记 `submission_unknown` 并禁止自动重发”逻辑正确；缺 task_id 时无法经 MCP 对账是已知恢复限制。随后通过当前连接的 media_generation MCP 各提交一条已由用户确认的普通请求：Image-2 收到上游 HTTP 502 “Upstream access forbidden, please contact administrator”，无 task_id/产物，MCP 标未知且未重发；Grok Video 返回任务 ID，状态完成，MP4 已保存。前者不能单凭结果区分上游访问拒绝和 MCP 回执处理问题；后者证明该次 Grok 视频链路可用。此处不是 MiniMax 测试，也不证明 USB 源 Skill 自动路由。视频文件元数据未用独立 ffprobe 检查。详见 `capability-audit-evidence-20261018/media-mcp-1.3.11-live-image-video-smoke-20261019.md`。


