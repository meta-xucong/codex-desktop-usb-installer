---
name: macro-research
description: 宏观研究工作流，核验政策与经济数据，并进行来源明确的周期、政策和市场情绪情景分析。
---

# 宏观研究

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。结果写入当前项目或用户明确指定的位置，不写入已安装 Skill 目录。

## 适用场景

- 核验并解读最新货币、财政或产业政策
- 用明确口径的数据做经济周期四象限分析
- 对市场情绪数据做确定性汇总
- 列出宏观判断的证据、反例、条件场景和待监测指标

## 数据与结论边界

宏观政策和市场数据具有强时效性。先使用当前 Codex 会话提供的网页搜索/浏览能力打开统计部门、央行、监管机构、
交易所或其他用户认可的一手来源，记录原文链接、发布日期、统计期和单位。网页能力不可用时，只分析用户提供的
材料，不补写“最新”数值。

三个脚本均不抓取数据，也不内置当前市场结论：

- `cycle_analyzer.py` 不内置 GDP/CPI 阈值、配置比例或转向概率。
- `policy_analyzer.py` 不内置“某政策必然利好某板块”、止损线或调仓指令。
- `sentiment_monitor.py` 不提供模拟指标或缺失字段默认值。

输出是给定数据和规则下的情景分析，不是投资承诺。任何配置比例必须来自用户审阅的政策/规则文件，并保留依据。

## Workflow 1：经济周期四象限

先确认增长指标、通胀指标及各自趋势值采用同一口径与时点。`cycle_analyzer.py` 只比较这四个数值，再按审阅后的
规则映射四象限。

规则文件必须包含 `as_of`、至少一个 `sources`，以及以下四个 `phase_policies`：

- `growth_up_inflation_down`
- `growth_up_inflation_up`
- `growth_down_inflation_up`
- `growth_down_inflation_down`

每个阶段至少含 `label`、`description`、非空 `characteristics`。可选 `allocation` 必须合计 100，并同时提供
`allocation_basis`；可选 `transition` 必须含 `next_phase`、`assessment`、`basis`。脚本不会把文字“高/中/低”
解释成统计概率。

```text
<python> "<skill-directory>/scripts/cycle_analyzer.py" \
  --gdp-growth <已核验数值> \
  --inflation <已核验数值> \
  --gdp-trend <同口径趋势值> \
  --inflation-trend <同口径趋势值> \
  --data-as-of "<数据时点>" \
  --source "<来源名称与 URL>" \
  --rules "<reviewed-cycle-rules.json>" \
  --json
```

禁止使用旧示例 `--current`；脚本没有该参数，也不会自行判断“当前”。

## Workflow 2：政策情景分析

先阅读政策原文，再由 Codex 或用户把证据与假设整理成 JSON：

```json
{
  "as_of": "<政策/分析时点>",
  "sources": ["<政策原文名称与 URL>"],
  "policy_type": "<类型>",
  "action": "<措施>",
  "context": "<适用范围、规模、期限与背景>",
  "evidence": ["<原文支持的事实>"],
  "transmission_hypotheses": [
    {
      "statement": "<待验证的传导假设>",
      "basis": "<支持或类比依据>",
      "confidence": "<置信说明及原因>"
    }
  ],
  "scenario_implications": [
    {
      "scenario": "<成立条件>",
      "implication": "<该条件下可能影响>",
      "monitor": ["<可证伪/确认指标>"]
    }
  ],
  "risks": ["<反例、滞后或执行风险>"]
}
```

```text
<python> "<skill-directory>/scripts/policy_analyzer.py" \
  --input "<verified-policy-scenario.json>" \
  --json
```

缺少来源、证据、传导依据、条件场景或风险时脚本非零退出。报告中不得把传导假设改写成确定因果或直接交易建议。

## Workflow 3：市场情绪

先取得同一时点、同一市场口径的数据。输入 JSON 必须包含 `as_of`、至少一个 `sources`、六项 0–100 指标、
用户审阅且合计为 1 的六项权重、涨跌家数、当前/平均成交量、价格变化和三类资金流：

```json
{
  "as_of": "<数据时点>",
  "sources": ["<来源名称与 URL>"],
  "indicators": {
    "market_momentum": 0,
    "stock_price_strength": 0,
    "stock_price_breadth": 0,
    "put_call_ratio": 0,
    "market_volatility": 0,
    "safe_haven_demand": 0
  },
  "weights": {
    "market_momentum": 0,
    "stock_price_strength": 0,
    "stock_price_breadth": 0,
    "put_call_ratio": 0,
    "market_volatility": 0,
    "safe_haven_demand": 0
  },
  "breadth": {"advancing_stocks": 0, "declining_stocks": 0},
  "volume": {"current_volume": 0, "avg_volume": 1, "price_change": 0},
  "fund_flow": {"northbound_flow": 0, "main_force_flow": 0, "retail_flow": 0}
}
```

零值仅表示字段类型，不是可用于真实报告的数据；实际权重必须合计为 1。脚本只计算合成值、广度、量比和资金流，
不会把数值映射成“恐惧/贪婪”、加减仓或止损动作。替换为核验值后执行：

```text
<python> "<skill-directory>/scripts/sentiment_monitor.py" \
  --input "<verified-sentiment-input.json>" \
  --index "<市场或指数>" \
  --json
```

## 输出要求

最终报告至少包含：

1. 数据时点、统计口径和逐项来源。
2. 事实与解释分栏，假设明确标注。
3. 基准场景、相反场景及各自触发条件。
4. 主要限制、缺失数据和可能证伪判断的指标。
5. 若涉及资产配置，注明比例来自哪份用户审阅规则，而非脚本推荐。

## 失败处理

- 来源不可访问或时点不一致：停止“当前”判断，列出待补材料。
- JSON 缺字段、过大或类型错误：脚本非零退出，不生成部分成功报告。
- 规则权重不等于 100：拒绝配置场景。
- 真实市场/政策发生变化：重新获取数据并生成新文件，不覆盖来源时点。

## 数据存储

建议写入 `${CODEX_DATA_DIR:-./codex-data}/macro-research/`。公开能力包不携带用户政策材料、行情快照或分析结果。
