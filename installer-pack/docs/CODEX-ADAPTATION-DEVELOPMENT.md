# Codex 适配整改开发文档

状态：已完成（本地、无真实外部凭据验收）  
基线日期：2026-09-08  
目标 Codex：`codex-cli 0.147.0`，最低兼容版本 `0.144.0`  
上游源码快照：`meta-xucong/codex-starter-kit@671f2f486ce10825e02ea547e3ce3667062676f0`

## 1. 目标

把当前能力集合从“结构和打包可审计”推进到“契约准确、可安全安装、可自动验证”的 Codex 能力包：

1. Skill 使用当前 Codex 的原生网页、图像、文件和 MCP 能力，不再强依赖旧平台工具名。
2. 7 个自定义 Agent 保持 `name`、`description`、`developer_instructions` 的当前 Codex TOML 契约。
3. 飞书技能只声明官方 `@larksuiteoapi/lark-mcp@0.5.1` 实际暴露的工具，并明确读写边界。
4. 安装器区分“复制 Skill/Agent”“运行时就绪”“连接就绪”，不把复制成功描述为能力健康。
5. 包构建、静态审计、隔离安装、脚本 smoke test 和当前 Codex 版本检查可以一条命令执行。

## 2. 官方契约基线

- Skill 是带 `SKILL.md` 的目录；`SKILL.md` frontmatter 只使用 `name`、`description`。
- 仓库级 Skill 使用 `.agents/skills`，个人 Skill 使用用户级 Skill 根目录；相同名称不会合并。
- Skill 的 `agents/openai.yaml` 用于 UI 元数据、MCP 依赖和隐式调用策略。
- 自定义 Agent 放在 `~/.codex/agents/` 或项目 `.codex/agents/`，必需字段为 `name`、`description`、`developer_instructions`。
- MCP 配置位于 `~/.codex/config.toml` 或受信项目 `.codex/config.toml`；STDIO MCP 支持 `env_vars`、`enabled_tools` 和写操作审批策略。

参考：

- <https://learn.chatgpt.com/docs/build-skills>
- <https://learn.chatgpt.com/docs/agent-configuration/subagents>
- <https://learn.chatgpt.com/docs/extend/mcp>
- <https://github.com/larksuite/lark-openapi-mcp/blob/main/README.md>
- <https://github.com/larksuite/lark-openapi-mcp/blob/main/docs/reference/cli/cli.md>

## 3. 范围与非目标

### 本轮范围

- 修正 16 个跨技能硬编码的 `web-search-extraction` 依赖。
- 修正 7 个自定义 Agent 对外部搜索适配器、固定输出目录和隔离 PDF Skill 的错误优先级。
- 修正 `image-prompt-generator` 的 `image2` 旧名称和强制生成流程。
- 重写 `feishu-doc`、`feishu-drive`、`feishu-perm`、`feishu-wiki` 的工具契约。
- 增加无密钥飞书 MCP 启动 wrapper、真实 Codex TOML 模板和工具清单。
- 增加配置片段渲染、兼容性审计和隔离安装测试。
- 修复可选网页抓取脚本在缺少可选包时连 `--help` 都无法运行的问题。
- 将引用缺失专有许可证的 `pdf-processing-toolkit` 降为 unsupported，直到再分发证据可核验。
- 重写默认启用的 `safe-disk-operations`：移除无关财务触发器和“一律拒绝只读检查”的旧策略，改为精确授权、只读预检、可恢复操作、哈希/所有权保护和高影响操作单独确认。
- 审计并加固 Image-2 / Seedance 外部 HTTP 适配器：移除密钥回退和模型/费用猜测，修复同步响应重复 POST、参考素材静默去重、会话读取虚假说明、安装目录写状态和远程图片下载边界。
- 删除旅行规划器声明但不可执行的 Node 占位入口、Seedance 无法可靠绑定当前 Codex 任务的会话历史读取入口，并修复月度复盘主题占位实现。
- 审计所有带脚本 Skill 的本地写入：行情缓存、公众号草稿/封面和外部适配器状态统一写入项目数据目录，净化文件名并限制输入/输出大小。
- 严格化 DashScope 兼容接口：Base URL 只接受 `compatible-mode/v1` 根地址，模型必填，脚本只拼接已核验的 `/chat/completions` 路径。
- 审计宏观、A 股、基金、资产配置和创业项目脚本中的模拟数据、固定市场假设、忽略参数、隐式评分/建议和成功空结果；改为可归因输入、显式场景、完整性状态和非零失败。
- 删除公众号文本检查器未实现的 `--auto-fix` 和发布就绪分数，使输出只表示编辑提示。
- 更新包版本、源码 commit、文档和生成物。

### 本轮非目标

- 不写入真实 App ID、App Secret、API Key、OAuth token。
- 不修改用户现有 `~/.codex/config.toml`。
- 不调用真实飞书、图像、视频或付费 API。
- 不把尚未物化的 Python wheelhouse 标记为 ready。
- 不安装或覆盖用户现有的全局 Skill、Agent 或运行时。

## 4. 设计决策

### 4.1 网页能力路由

所有非适配器 Skill 统一采用以下顺序：

1. 使用当前 Codex 会话提供的原生网页搜索/浏览能力。
2. 原生网页能力不可用时，只处理用户给出的链接、文件和文本。
3. 明确标注无法实时核验，不伪造搜索结果。
4. `web-search-extraction` 保留为用户显式启用的 DashScope 适配器，但其他 Skill 不硬编码调用它，也不写仓库根目录相对命令。

需要修改：

- `academic-research`
- `china-stock-analysis`
- `competitor-intelligence`
- `data-assistant`
- `docx-butler`
- `efficiency-toolkit`
- `email-expert`
- `fund-portfolio`
- `industry-analysis`
- `macro-research`
- `market-sizing-analysis`
- `meeting-secretary`
- `travel-planner`
- `user-research`
- `venture-analysis`
- `wealth-allocation`

验收：除 `skills/web-search-extraction/SKILL.md` 自身外，任何 `SKILL.md` 都不得出现 `web-search-extraction` 或 `web_search.py`。

### 4.2 图像能力路由

`image-prompt-generator` 的职责是产生提示词，不依赖任何生成服务即可完成。只有用户明确要求生成图片时才进入第二阶段：

1. 优先使用当前 Codex 提供的原生图像生成能力。
2. 用户明确选择并已配置外部 Image-2 时，才显式调用 `$image-2`。
3. 没有生成能力时交付提示词文件，不声称图片已生成。

验收：仓库文本中不存在作为工具名使用的 `image2`；`image-2` 仍保持 `guided-config` 和禁止隐式调用。

### 4.3 飞书 MCP 契约

MCP server 固定使用 `feishu`，wrapper 启动官方包并强制 `--tool-name-case dot`。Skill 只引用清单中的点号工具名。

基础读取工具：

- 文档：`docx.v1.document.get`、`docx.v1.document.rawContent`、`docx.v1.documentBlock.list`、`docx.v1.documentBlock.get`
- 云盘：`drive.v1.file.list`、`drive.v1.meta.batchQuery`
- 权限读取：`drive.v1.permissionMember.auth`、`drive.v1.permissionMember.list`、`drive.v1.permissionPublic.get`
- Wiki：`wiki.v2.space.get`、`wiki.v2.space.list`、`wiki.v2.space.getNode`、`wiki.v2.spaceNode.list`

显式写操作工具：

- 云盘：`drive.v1.file.copy`、`drive.v1.file.createFolder`、`drive.v1.file.move`、`drive.v1.file.delete`
- 权限：`drive.v1.permissionMember.create`、`drive.v1.permissionMember.update`、`drive.v1.permissionMember.delete`、`drive.v1.permissionMember.transferOwner`
- Wiki：`wiki.v2.spaceNode.copy`、`wiki.v2.spaceNode.create`、`wiki.v2.spaceNode.move`、`wiki.v2.spaceNode.updateTitle`

文档编辑和文件上传/下载不在本轮承诺范围内。官方 README 对这两项仍有明确限制；`feishu-doc` 只读，写作请求降级为生成待粘贴内容。

安全要求：

- `config.toml` 只转发 `FEISHU_APP_ID`、`FEISHU_APP_SECRET` 的变量名；仅 user-token 模式再转发 `FEISHU_USER_ACCESS_TOKEN`，不保存任何值。
- wrapper 不打印凭据；`-DryRun` 只输出脱敏计划。
- 域名只允许 `https://open.feishu.cn` 和 `https://open.larksuite.com`。
- 写工具使用 Codex `default_tools_approval_mode = "writes"`，高风险权限和删除工具再覆盖为 `prompt`。
- 离线包不存在时立即失败，不使用联网 `npx -y` 兜底。
- wrapper 校验 npm package 的名称、版本和 `dist/cli.js` 入口，并直接调用 Node；不经过 npm 的 `.cmd` shim，避免 Secret 中的 Windows shell 元字符被二次解释。

### 4.4 安装与配置边界

安装器仍默认只复制 25 个核心 Skill 和 7 个 Agent，但增加：

- 可注入隔离目标用户目录，供测试和离线部署使用。
- 安装 MCP wrapper 到包自有目录。
- 写出不含凭据的 readiness 报告。
- 不自动合并用户 `config.toml`。
- 校验 checksum/file-manifest/实际文件三者闭包，拒绝额外文件、符号链接和目录联接。
- `-Overwrite` 只更新当前哈希仍等于上次安装记录的包文件；发现用户修改或用户新增文件时保留整棵 Skill 并跳过。
- `-IncludeNonDefault` 仍排除 unsupported；只有再显式加 `-IncludeUnsupported` 才复制隔离内容。

配置渲染器只生成待审阅 TOML 片段：

- 始终渲染非核心 Skill gating。
- 只有参数完整时才可把飞书段落渲染为 enabled。
- 未替换占位符、明文 secret 或非法路径都视为失败。

### 4.5 脚本路径

Skill 中的脚本均以当前 Skill 目录为基准。说明文本必须要求 Codex 根据已加载 Skill 的文件路径解析绝对脚本路径，不得假设当前工作目录是仓库根目录。

### 4.6 外部图像/视频网关

Image-2 与 Seedance 是直连 HTTP 适配器，不是 MCP，也不是 Codex 原生工具：

1. 仅在用户显式选择对应 Skill 且连接字段完整时启用；普通图像请求仍优先走 Codex 原生图片能力。
2. API Key 只能读取对应服务的专用环境变量；Image-2 禁止回退读取 `OPENAI_API_KEY`。
3. Base URL 必须是根级 HTTPS 地址，清单明确记录脚本实际拼接的固定兼容路径。
4. 模型与外部 `agent-id` 必须由服务方配置；Codex 没有可猜测替代的标准外部网关路由 ID。
5. Seedance 的预扣积分、返还规则和官方链接有效期必须显式配置；不保留 `20000`、`24 小时` 或模型名猜测值。
6. 两个适配器都严格保留参考素材的顺序、数量和重复项；不得静默去重。
7. Image-2 本地参考图写入大小和 SHA-256，正式执行前复验；远程下载只允许 HTTPS 公网目标，逐次校验重定向，限制为 25 MiB，并在任一图片失败时整体停止。
8. Seedance 不读取 Codex 会话/session 文件；调用者必须显式传入当前用户消息中的完整媒体 URL。
9. 确认清单、任务记录和 base64 产物进入项目 `codex-data/` 或明确数据目录，不写入安装后的 Skill 树。
10. 确认指纹、哈希、幂等键和清单路径仅供执行器内部校验；用户确认的是完整业务输入和真实服务方费用策略。

### 4.7 数据、假设与决策边界

高时效或高风险领域的脚本按以下层次输出，不得混写：

1. **外部观测**：保留产品/证券代码、来源接口、抓取时点、数据统计期、复权/币种/单位和组件完整性。
2. **用户政策或审阅规则**：配置权重、周期分类、评分 rubric、交易成本和尽调适用性必须来自显式 JSON/CLI 输入，且保留来源或制定依据。
3. **机械计算**：只执行字段筛选、比例/金额换算、相邻记录差值、公开公式和敏感性场景；公式限制进入输出。
4. **分析解释**：Codex 必须将事实、用户假设、脚本计算和推断分开，并列出反例、缺失字段和人工核验项。
5. **决策**：脚本不输出自动买卖、产品选择、目标价、风险等级、仓位、止损线、收益承诺、投/不投结论或法律/税务意见。

具体整改：

- 宏观周期要求增长/通胀水平及变化、带来源的四象限规则；政策要求原文证据、传导依据和条件场景；情绪合成值要求六项完整指标和显式合计为 1 的权重。
- A 股实时接口删除“主力意图”推断；AkShare 获取器按组件报告 `complete/partial/failed`，默认部分失败也返回非零状态且只缓存完整结果；筛选只暴露实际字段；财务分析和模板删除固定评级、综合分、排名、目标价和行业均值占位；估值只运行显式 DCF/DDM/相对观测。
- 基金筛选只使用接口实际字段；配置说明保持用户备注标签；定投、风险、资产配置、再平衡和追踪只计算显式场景，不生成产品或交易建议。
- 创业商业评分只计算用户 rubric；财务模型不内置健康阈值或融资轮次；估值方法不自动平均；尽调模板要求法域、行业、交易结构，所有项目初始为未审阅/适用性待确认，不生成固定负责人或周期。

### 4.8 本地状态与失败语义

- 已安装 Skill 树视为只读程序资产，缓存、草稿、确认清单、任务记录和生成物写入 `${CODEX_DATA_DIR:-./codex-data}/<skill-id>/` 或用户明确路径。
- 路径片段必须净化；外部下载、JSON、图片和缓存文件设置大小上限；适用处采用临时文件后原子替换。
- 网络/提供方失败不得退回随机、示例或旧缓存并声称最新；批量任务保留逐项错误，默认只在全部必需组件完整时退出 0。
- 可选依赖在真正执行对应能力时才加载，使 `--help` 和离线审计不被无关依赖阻断。

## 5. 实施阶段

### 阶段 A：契约与文档

1. 创建本开发文档和长期任务状态。
2. 新增源码 commit 锁文件，构建器在没有 `.git` 时使用它。
3. 将包版本升级为 `1.2.0`，记录本机验证的 Codex `0.147.0`。

验收：开发文档包含范围、文件、步骤、风险和测试矩阵；构建包得到 40 位上游基线 commit 和当前源码树 SHA-256。

### 阶段 B：Skill 适配

1. 替换 16 个 Skill 的网页路由。
2. 修正 `image-prompt-generator`。
3. 重写四个飞书 Skill，并同步 `agents/openai.yaml`。
4. 为全部 25 个非核心 Skill 补齐符合长度约束的 `short_description`、显式 `$skill-id` 的 `default_prompt` 和隐式调用门控。
5. 修正 7 个自定义 Agent：原生网页/图像优先，外部适配器显式选择，PDF 许可证隔离，输出路径不再固定。
6. 修复 `web-content-fetcher/scripts/fetch.py` 的可选依赖加载。
7. 为带脚本的 Skill 补充目录解析约定。
8. 重写 Image-2 / Seedance Skill 契约并加固脚本，补齐专用连接字段和项目数据路径。
9. 加固 DashScope 入口，删除猜测模型和宽松端点；删除 Seedance 旧会话读取器和旅行规划器占位入口。
10. 修正行情缓存、公众号草稿/封面等本地写边界以及月度主题占位实现。
11. 重写宏观周期/政策/情绪入口，要求显式规则、证据、来源、时点和权重。
12. 重写 A 股筛选、行情、数据获取、财务/技术分析、估值及旧报告模板，删除模拟值、忽略参数、静默失败、评分排名和交易结论。
13. 重写基金配置/风险/定投、家庭配置/再平衡/追踪以及创业评分/财务/估值/尽调入口，所有市场与政策假设显式化并删除自动建议。
14. 删除公众号检查器未实现的自动修复和发布就绪评分。

验收：静态契约审计通过；四个飞书 Skill 通过 `quick_validate.py`；`fetch.py --help` 在未安装 Scrapling 时也返回 0。

### 阶段 C：MCP 与安装闭环

1. 新增 `manifest/feishu-tools.json`。
2. 新增 `scripts/start-feishu-mcp.ps1`。
3. 更新飞书 TOML 模板、连接字段和 MCP 清单。
4. 新增 `scripts/render-codex-config.ps1`。
5. 扩展安装器，支持隔离目标、wrapper 安装和 readiness 报告。
6. 构建器把新增脚本、清单和文档复制到 `installer-pack/`。

验收：wrapper dry-run 不泄露假密钥；模板替换后可由 Python `tomllib` 解析；隔离安装只写测试目录。

### 阶段 D：自动审计与测试

1. 新增 `scripts/audit-codex-compatibility.py`。
2. 将契约审计、wrapper 测试、配置渲染和隔离安装 smoke test 接入 `verify-package.ps1`。
3. 检查当前 `codex --version`；不存在 Codex 时给 warning，版本低于最低要求时失败。
4. 运行构建、静态审计、语法检查、包 hash、PowerShell smoke tests。
5. 运行 50 个 Skill 的 frontmatter 校验；对本轮重写 Skill 运行 Skill Creator 快速校验。
6. 对源码树和生成包分别运行 `test-adapter-contracts.py`，无网络验证专用凭据隔离、单次 POST、URL 边界、素材重复项、文件哈希、显式模型/费用策略、金融数据来源/完整性、无隐式评级建议和数据路径。

验收：全部结构性测试退出码为 0；缺少可选 Python 包只进入 readiness warning，不导致虚假通过或虚假 ready。

### 阶段 E：最终复核

1. 重建 `installer-pack/`。
2. 检查源码与生成包没有旧工具名、凭据、缓存或用户内容。
3. 复核 README、环境清单、manifest 和生成物一致。
4. 记录测试命令、结果、已知限制和未执行的外部测试。

验收：没有未解释的失败；真实飞书/API 测试因无凭据明确标记为未执行，而非通过。

## 6. 测试矩阵

| 层级 | 命令/方法 | 通过条件 |
|---|---|---|
| 构建 | `python scripts/build-pack.py --check` | 50 Skill、7 Agent、25 默认 Skill，上游 commit 为 40 位十六进制且有当前源码树 SHA-256 |
| 契约审计 | `python scripts/audit-codex-compatibility.py --require-codex` | 无旧工具名、依赖遗漏、模板错误；Codex 版本满足最低要求 |
| 包验收 | `powershell -File scripts/verify-package.ps1` | hash、manifest、语法、smoke test 全部通过 |
| Skill 校验 | `python -X utf8 <skill-creator>/scripts/quick_validate.py <skill-dir>` | 50 个 Skill 均通过；Windows 下显式 UTF-8，规避系统校验器按 GBK 读取 UTF-8 文件 |
| 抓取脚本 | `python skills/web-content-fetcher/scripts/fetch.py --help` | 无可选包时仍退出 0 |
| 离线契约回归 | `python scripts/test-adapter-contracts.py`，并对生成包重复执行 | 34 个无网络回归测试通过；覆盖外部适配器、金融数据真实性、占位实现和写入边界，不调用真实付费 API |
| 飞书 wrapper | 使用假环境变量执行 `-DryRun` | 输出不含假 secret，参数和工具清单正确 |
| 配置渲染 | 渲染到临时目录并用 `tomllib` 解析 | 无占位符、无 secret、TOML 合法 |
| 隔离安装 | 安装到临时 `TargetUserProfile` | 默认 25、非核心 46、明确接受 unsupported 后 50；7 Agent、wrapper、readiness 均存在；本地修改和新增文件保留；真实用户目录未变化 |
| 运行时 | readiness import probe | 已安装包报告 ready，缺失包列为 missing；不改变公共 pending 状态 |

## 7. 发布与回滚

- `installer-pack/` 是生成物，只通过 `scripts/build-pack.py` 更新。
- 安装器只覆盖 ownership manifest 中已有的包自有文件。
- 配置渲染器不直接合并用户配置；用户可以删除生成片段完成回滚。
- 飞书 wrapper 和 readiness 文件位于包自有目录，不写入 Skill 目录或用户文档。
- 若某阶段测试失败，先修复并重跑该阶段最小测试，再进入下一阶段。

## 8. 完成定义

- 本文档中的阶段 A-E 均完成。
- `verify-package.ps1` 和兼容性审计通过。
- 50 个 Skill、7 个 Agent、manifest、安装器与 `installer-pack` 一致。
- 不存在旧平台工具契约或无声明的强依赖。
- 外部服务保持无密钥、未连接、诚实降级状态。

## 9. 最终实施结果（2026-09-08）

- 50 个 Skill、7 个 Agent、25 个非核心 `agents/openai.yaml`、25 个飞书 allowlist 工具和 19 个连接字段通过源树审计。
- 整改后状态为：25 个 `core-ready`、14 个 `auto-installable-runtime`、7 个 `guided-config`、4 个 `unsupported`；默认只安装 25 个核心 Skill。
- 34 个离线契约回归在源树和生成包各执行一次并通过；覆盖 DashScope、Image-2、Seedance、金融数据真实性、占位实现和本地写入边界。
- 50 个 Skill 使用 Skill Creator 的 `quick_validate.py` 在 Python UTF-8 模式下逐个通过。
- `verify-package.ps1` 已通过源码/生成包双审计、Python/PowerShell/TOML 语法、包闭包与哈希、飞书 dry-run、配置渲染、公众号路径穿越、三种隔离安装规模和本地修改保护测试。
- 验收中额外修复了两处仅在生成包/Windows 上暴露的问题：DashScope 环境模板的源码/包布局解析，以及 GBK 控制台打印 emoji 导致脚本写入成功后异常退出。
- 未执行真实飞书、DashScope、Image-2、Seedance、行情或其他付费/联网提供方调用；原因是本轮不读取真实凭据，也不以联网结果代替确定性契约测试。
- Python wheelhouse 仍保持 `materializationStatus=pending`，所以 14 个运行时 Skill 的结论是“依赖可自动安装的设计已闭环”，不是“公开包已携带可安装 wheel”。
