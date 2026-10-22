---
name: wealth-allocation
description: 使用用户审阅策略模板换算资产金额，并计算目标权重差额和带来源的组合历史表现。
---

# 财富配置场景

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行 `scripts/...` 时使用绝对路径，
不要假设当前工作目录是仓库根目录。结果写入当前项目或用户指定目录，不写入 Skill 安装目录。

## 数据与决策边界

当前估值、产品规则、基准收益和无风险利率具有时效性。需要这些数据时，优先用当前 Codex 的网页搜索/浏览能力
打开用户认可的一手资料，记录时点和链接。网页能力不可用时，只能基于用户提供的输入做情景计算。

三个脚本都不执行交易：

- `asset_allocator.py` 只读取用户审阅的“三桶”策略模板并换算金额。
- `rebalance_planner.py` 只计算当前权重到目标权重的差额和显式卖出成本场景。
- `portfolio_tracker.py` 只计算无期间申赎假设下的收益、波动、回撤和显式基准差。

脚本不推断风险偏好、不内置标准配置、基准收益、交易阈值、止盈或止损线。

## Workflow 1：策略金额换算

策略模板 JSON 必须包含 `risk_level`、`description`、`as_of`、非空 `sources`、`allocation`、
`scenario_annual_return`、`scenario_max_drawdown`、`emergency_months` 和 `rebalance_threshold`。
`allocation` 必须且只能包含 `survival/growth/aggressive`，合计 100。

```text
<python> "<skill-directory>/scripts/asset_allocator.py" \
  --template "<reviewed-policy.json>" \
  --amount 1000000 \
  --period-years 5 \
  --monthly-expense 20000 \
  --output "<project-data>/allocation.json"
```

输出只包含三桶金额、应急金目标/缺口、策略时点/来源和方法边界；不推荐具体产品。

## Workflow 2：目标权重差额

当前与目标配置均为“资产代码到百分比”的 JSON 对象，各自合计必须为 100。组合价值、纳入计算的最小偏离百分点和
卖出交易成本率全部显式传入。

```text
<python> "<skill-directory>/scripts/rebalance_planner.py" \
  --current "<current.json>" \
  --target "<reviewed-target.json>" \
  --value 100000 \
  --threshold 5 \
  --transaction-cost-rate 0.1 \
  --output "<project-data>/target-differences.json"
```

输出中的 `increase/decrease` 只是达到用户目标所需的数学方向。脚本不下单，也不处理税、买入费用、滑点、最小交易单位、
账户限制或目标策略是否适合用户。

## Workflow 3：历史表现计算

`history` 必须是按月排列的 JSON 数组，每项含正数 `value`；点数恰好为 `months + 1`，首末值与
`--initial/--current` 一致。同期无风险利率、数据时点和来源必填。只有提供已核验的“基准名称到同期年化收益率”
JSON 对象时才生成差值比较。

```text
<python> "<skill-directory>/scripts/portfolio_tracker.py" \
  --initial 100000 \
  --current 108000 \
  --history "<monthly-history.json>" \
  --months 12 \
  --risk-free-rate 2 \
  --benchmarks "<verified-benchmarks.json>" \
  --as-of "<数据时点>" \
  --source "<来源名称与 URL>" \
  --output "<project-data>/tracking.json"
```

模型假设没有期间申赎；波动率是月收益总体标准差年化。输出不做收益归因，不生成综合评分、调仓、止盈或止损建议。

## 输出要求

1. 列出策略/数据时点、来源、币种和单位。
2. 区分用户目标、历史观测、场景假设和机械计算。
3. 显示费用、现金流、数据频率和相关性等限制。
4. 不把历史表现、基准差或情景配置写成未来收益承诺。
5. 执行任何配置变化前，由用户另行确认产品、金额、账户、费用和税务影响。

## 数据存储

建议写入 `${CODEX_DATA_DIR:-./codex-data}/wealth-allocation/`。公开能力包不包含账户、持仓、净值或个人财务数据。
