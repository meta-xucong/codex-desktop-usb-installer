---
name: fund-portfolio
description: 核验基金公开数据，并进行字段透明的筛选、用户审阅配置换算、定投敏感性和风险场景计算。
---

# 基金组合分析

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行 `scripts/...` 时使用绝对路径，
不要假设当前工作目录是仓库根目录。结果写入当前项目或用户指定目录，不写入已安装 Skill 树。

## 数据与决策边界

基金净值、排名、费率、持仓和风险指标会变化。优先使用当前 Codex 的网页搜索/浏览能力打开基金定期报告、招募说明书、
管理人公告或用户认可的数据源，记录产品代码、份额类别、统计区间、数据时点和链接。网页能力不可用时，不提供“最新”
判断。

脚本职责：

- `fund_screener.py` 只筛选 AkShare 排行接口实际读取的类型、近 1 年和近 3 年收益字段。
- `portfolio_builder.py` 只把用户审阅的配置模型换算为金额。
- `sip_calculator.py` 只比较显式恒定收益率场景。
- `risk_assessor.py` 只计算显式波动率、相关性、收益和回撤倍数下的场景指标。

脚本不执行申购、赎回或交易，也不内置适配人群、止损线、预期收益、风险等级或基金推荐。

## Workflow 1：基金字段筛选

```text
<python> "<skill-directory>/scripts/fund_screener.py" \
  --type "混合" \
  --min-return-1y 10 \
  --min-return-3y 5 \
  --top 20 \
  --output "<project-data>/fund-screen.json"
```

脚本记录抓取时点和 `akshare.fund_open_fund_rank_em` 接口名。它不接受最大回撤、夏普比率或费率筛选参数；需要这些
指标时，先取得同口径的完整数据并进入风险场景流程。接口失败或没有返回数据时非零退出；筛选后零条是有效结果。

## Workflow 2：配置金额换算

风险标签和比例必须来自用户确认的投资政策或可核验专业建议。模型 JSON 必须包含 `risk_level`、`description`、
`as_of`、非空 `sources`、合计 100% 的 `allocation`、`scenario_annual_return`、`scenario_max_drawdown`，以及可选
`fund_type_notes`。

```text
<python> "<skill-directory>/scripts/portfolio_builder.py" \
  --model "<reviewed-allocation-model.json>" \
  --amount 100000 \
  --period-years 3 \
  --output "<project-data>/portfolio.json"
```

`fund_type_notes` 只是模型文件中的用户说明，输出仍使用 `fund_type_notes`，不会改名为“基金推荐”或“投资官建议”。
结果中的收益与回撤是模型标签，不是预测。

## Workflow 3：定投敏感性

`--expected-return` 是调用者提供的恒定年化收益场景。选择 `智能定投` 时必须另行提供
`--smart-expected-return`；两者差异只用于敏感性比较，不能证明策略优劣。

```text
<python> "<skill-directory>/scripts/sip_calculator.py" \
  --monthly 3000 \
  --years 5 \
  --expected-return 8 \
  --strategy "普通定投" \
  --json
```

模型未计入费用、税、通胀和收益波动。

## Workflow 4：组合风险场景

先从同一口径、同一时点的数据源取得每只基金年化波动率，并把全部假设写入 JSON：

```json
{
  "as_of": "<数据时点>",
  "sources": ["<来源名称与 URL>"],
  "allocations": {"FUND_A": 50, "FUND_B": 50},
  "annual_volatility": {"FUND_A": 0, "FUND_B": 0},
  "asset_classes": {"FUND_A": "equity", "FUND_B": "bond"},
  "correlation": 0,
  "expected_return": 0,
  "risk_free_rate": 0,
  "drawdown_multiplier": 1
}
```

数值仅表示字段类型，不是市场默认。`asset_classes` 只接受 `equity/bond/cash/other`，配置合计必须为 100；
统一相关系数必须形成有效矩阵。

```text
<python> "<skill-directory>/scripts/risk_assessor.py" \
  --portfolio "<verified-risk-scenario.json>" \
  --output "<project-data>/risk-scenario.json"
```

输出包含组合波动率、显式倍数回撤场景、Sharpe、资产类别合计、最大单一仓位和 HHI。它不把数值映射为风险等级、
适配人群、加减仓或止损建议。

## 输出要求

1. 产品代码和份额类别明确，数据时点与来源可追溯。
2. 区分历史观测、用户政策、场景假设和机械计算。
3. 展示缺失字段、相关性简化、费用/税/现金流等模型限制。
4. 不按短期排名暗示未来表现，不把场景数值改写成收益承诺。
5. 任何配置或交易决策须由用户结合自身约束确认。

## 数据存储

建议写入 `${CODEX_DATA_DIR:-./codex-data}/fund-portfolio/`。公开能力包不包含用户持仓、净值快照或评估结果。
