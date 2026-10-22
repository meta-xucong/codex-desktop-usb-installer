---
name: venture-analysis
description: 基于可核验证据和显式假设进行创业项目评分、财务场景、估值计算与尽调清单整理。
---

# 创业项目分析

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行 `scripts/...` 时使用绝对路径，
不要假设当前工作目录是仓库根目录。结果写入当前项目或用户明确指定的位置，不写入 Skill 安装目录。

## 数据与决策边界

行业规模、竞品、融资、政策和可比交易具有时效性。优先用当前 Codex 的网页搜索/浏览能力打开公司材料、监管原文、
统计资料或用户认可的数据源，记录发布日期、统计期、币种、单位和链接。网页能力不可用时，只分析用户提供的材料。

四个脚本的边界：

- `business_evaluator.py` 只计算用户审阅评分表，不从描述长度或市场金额生成“质量分”。
- `financial_model.py` 只运行恒定参数场景，不内置健康阈值、融资轮次或 6 个月缓冲。
- `valuation_analyzer.py` 只使用带来源的可比、倍数、Scorecard 与退出假设。
- `due_diligence.py` 生成带法域、行业和交易结构上下文的通用起点清单，不替代律师、会计师、技术或数据合规专家意见。

不得把任何加权分、模型值或清单转换成自动“投/不投”决定。

## Workflow 1：商业评估评分表

先把事实证据、市场口径和用户认可的评估维度写入 JSON。`criteria` 每项含唯一 `id`、`label`、0–1 `weight`、
0–10 `score`、具体 `basis` 和 `risks`；权重必须合计 1。

```json
{
  "project_name": "<项目名>",
  "industry": "<行业>",
  "as_of": "<输入时点>",
  "sources": ["<商业计划或外部来源>"],
  "evidence": ["<已核验事实>"],
  "market": {
    "tam": 0,
    "sam": 0,
    "som": 0,
    "currency": "CNY",
    "as_of": "<市场数据时点>",
    "sources": ["<市场规模来源>"],
    "basis": "<TAM/SAM/SOM 定义与计算口径>"
  },
  "criteria": [
    {
      "id": "team",
      "label": "团队证据",
      "weight": 1,
      "score": 0,
      "basis": "<为何给出该分数>",
      "risks": []
    }
  ]
}
```

必须满足 `0 ≤ SOM ≤ SAM ≤ TAM`。零值仅是结构示意，不是实际市场输入。

```text
<python> "<skill-directory>/scripts/business_evaluator.py" \
  --input "<reviewed-business-evaluation.json>" \
  --json
```

脚本只输出 `score × weight` 的贡献与总和，不输出评级、投资比例或推荐结论。

## Workflow 2：财务场景

模型显式区分年净活跃用户增长率、月流失率、每新增活跃用户 CAC、月 ARPU、固定成本和其他现金流出。
它用月流失率估算年留存，再计算达到年末净增长目标需要补充的活跃用户；收入按年初/年末平均活跃用户计算。

```text
<python> "<skill-directory>/scripts/financial_model.py" \
  --years 5 \
  --initial-active-users 1000 \
  --annual-net-user-growth-rate 50 \
  --monthly-arpu 100 \
  --cac-per-new-user 50 \
  --gross-margin 70 \
  --monthly-churn-rate 3 \
  --monthly-fixed-costs 50000 \
  --monthly-other-cash-outflow 10000 \
  --initial-cash 500000 \
  --liquidity-buffer-months 6 \
  --as-of "<输入时点>" \
  --source "<数据来源或假设依据>" \
  --json
```

`liquidity-buffer-months` 是调用者显式场景，不是脚本建议。LTV、LTV/CAC 和回本期只展示算式结果，不用固定阈值
标记“健康”。输出未包含税、营运资本、资本开支、融资稀释、季节性或 cohort 差异。

## Workflow 3：早期估值场景

先从可比交易或用户材料形成 `assumptions.json`，必须包含：

- `as_of`、非空 `sources`
- `comparable_valuation_range`（万元）
- `revenue_multiple_range`
- `scorecard_base_valuation`（万元）
- 团队、产品、市场、竞争、时机五项 `scorecard_weights`，合计 1
- 把 0–10 加权分线性映射到估值因子的 `scorecard_factor_range`
- `range_factor`

公司输入、目标回报和退出倍数也必须显式填写：

```text
<python> "<skill-directory>/scripts/valuation_analyzer.py" \
  --assumptions "<reviewed-valuation-assumptions.json>" \
  --stage "<阶段>" \
  --industry "<行业>" \
  --revenue <收入> \
  --growth-rate <增长率> \
  --team-score <0-10> \
  --product-score <0-10> \
  --market-score <0-10> \
  --competition-score <0-10> \
  --timing-score <0-10> \
  --target-return <目标回报率> \
  --exit-multiple <退出倍数> \
  --years <年数> \
  --json
```

结果只是这些假设下的估算，不是最新交易价格或投资承诺。脚本并列展示三种方法及总包络，不再平均方法得到
“公允值”，也不自动生成谈判价、对赌条款或投资建议。

## Workflow 4：尽调清单

```text
<python> "<skill-directory>/scripts/due_diligence.py" \
  --stage "A轮" \
  --jurisdiction "<法域>" \
  --industry "<行业>" \
  --transaction-structure "<股权/可转债等结构>" \
  --output "<project-data>/dd_checklist.md"
```

把脚本输出视为通用起点。所有条目的初始状态均为 `not_reviewed`、适用性为 `confirm`；脚本不自动分配高/中优先级，
也不生成固定周期、负责人、适用人群或投资处置。根据法域、行业、交易结构、数据处理、知识产权和用户要求增删项目；
负责人、周期、证据标准和材料充分性均需人工确认。不得因生成了清单就声称已完成尽调。

## 输出要求

1. 所有外部事实、市场口径、假设与评分依据均带时点和来源。
2. 区分事实、用户评分、机械计算、分析假设和专业判断。
3. 展示敏感性、反例、缺失材料和可证伪指标。
4. 不生成固定投资比例、自动推荐、止损线或融资轮次。
5. 法律、税务、会计和证券问题明确转交相应持证专业人士。

## 数据存储

建议使用 `${CODEX_DATA_DIR:-./codex-data}/venture-analysis/` 下的 `business/`、`financial-models/`、
`valuations/` 和 `due-diligence/`。公开能力包不包含项目材料、模型输入或尽调结果。
