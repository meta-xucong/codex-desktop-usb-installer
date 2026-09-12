# Codex 功能 Agent 目录

这里的 Agent 都用功能名命名，名称本身不代表真人或固定人设。调用时点名功能即可；是否能执行具体动作，
由它引用的 Skill、运行时、外部连接和健康检查决定。

## 一览

| Agent | 主要负责 | 记忆词 |
|---|---|---|
| 全能官 | 综合统筹、任务拆解、跨领域协作 | 全能、统筹、综合 |
| 研究官 | 学术、行业、竞品、用户和市场研究 | 研究、调研、竞品、行业 |
| 创作官 | 公众号、小红书、文案、脚本、图片/视频提示词 | 创作、写作、文案、脚本 |
| 投资官 | A 股、基金、宏观、创业项目和资产配置 | 投资、股票、基金、配置 |
| 增长官 | 用户痛点、产品定位、获客、转化和增长实验 | 增长、商业、获客、转化 |
| 生活官 | 健身、饮食、旅行、天气和个人复盘 | 生活、健身、饮食、旅行 |
| 办公官 | 文档、表格、PDF、PPT、邮件、会议和文件整理 | 办公、文档、表格、会议 |

## 能力闭包与降级

| Agent | 必需 Skill | 可选 Skill | 外部/运行时依赖 | 缺依赖时的行为 |
|---|---|---|---|---|
| 全能官 | `multi-search-engine`、`image-prompt-generator`、`video-prompt-generator` | `web-search-extraction`、`data-assistant`、`docx-butler`、`excel-xlsx`、`word-docx`、`powerpoint-pptx` | Python wheelhouse；可选网页搜索 | 先交付文字、研究框架和提示词，文件任务说明缺口 |
| 研究官 | `academic-research`、`competitor-intelligence`、`industry-analysis`、`user-research`、`market-sizing-analysis`、`multi-search-engine` | `web-search-extraction` | 可选网页搜索连接 | 只基于用户材料，标注无法实时核验 |
| 创作官 | `xiaohongshu-writing`、`image-prompt-generator`、`video-prompt-generator` | `wechat-article-creator`、`web-search-extraction`、`image-2`、`seedance2-0-video-gen` | Python wheelhouse；图像/视频/搜索连接 | 输出成稿、提示词和分镜，不伪造图片/视频已生成 |
| 投资官 | `china-stock-analysis`、`fund-portfolio`、`macro-research`、`venture-analysis`、`wealth-allocation` | `web-search-extraction` | Python wheelhouse；可选网页搜索连接 | 没有数据时只给方法、假设和风险，不编造行情 |
| 增长官 | `painpoint-analyzer`、`offer-designer`、`traffic-hunter`、`conversion-optimizer`、`growth-analyst` | `web-search-extraction` | 可选网页搜索连接 | 基于用户业务数据和显式假设设计实验 |
| 生活官 | `daily-reflection`、`weather-forecast`、`weather` | `fitness-coach`、`meal-planner`、`travel-planner`、`multi-search-engine`、`web-search-extraction` | Python wheelhouse；可选搜索连接 | 保留计划、清单和复盘文本；不伪造实时天气/价格 |
| 办公官 | `efficiency-toolkit`、`email-expert`、`meeting-secretary` | `data-assistant`、`docx-butler`、`excel-xlsx`、`word-docx`、`powerpoint-pptx` | Python wheelhouse | 输出结构化文本和操作步骤，不声称文件已生成 |

所有表中的 `web-search-extraction` 都只是用户明确选择的外部适配器；检索默认优先使用当前 Codex 原生网页能力。
`pdf-processing-toolkit` 属于 Agent 的 `blockedSkills`，只用于提醒许可证隔离，不是可选执行能力；PDF 任务优先走
当前 Codex 已提供的 PDF 能力。

## 直接调用

在新任务第一句话点名正式名称：

```text
请使用自定义 Agent「研究官」，研究这个行业：……
请区分事实、来源、推测和建议，并标注数据日期。
```

```text
请使用自定义 Agent「创作官」，把下面材料改写成一篇小红书笔记，并先给成稿，再给标题和配图提示词：……
```

```text
请使用自定义 Agent「办公官」，处理下面的会议记录，输出摘要、决定、负责人、截止时间和待确认项：……
```

## 在当前任务中委派

```text
请把市场研究交给「研究官」独立完成，返回证据和结论；你最后再帮我汇总。
```

也可以并行委派不同功能：

```text
请并行处理：研究官负责市场证据，增长官负责商业化实验，创作官负责最终文案。
```

若 Codex 当前版本不显示单独的 Agent 按钮，点名文字仍是稳定入口；Agent TOML 安装在
`%USERPROFILE%\.codex\agents\` 后，重载 Codex 或打开新任务即可发现。

## 每个 Agent 的工作纪律

- 先判断相关 Skill 是否存在、是否通过健康检查，再决定能否执行。
- 缺少 Python、离线包、MCP、API、网络或凭据时，明确说出缺口和替代交付物。
- 不把静态模板、工作流说明或用户提供的数据说成已连接的外部工具。
- 涉及实时信息时给出来源和日期；涉及投资、健康、权限和外部写入时先说明边界。
- 只把用户要求的文件写到工作区，不生成隐藏的全局指令文件，不覆盖无关 Agent/Skill。
