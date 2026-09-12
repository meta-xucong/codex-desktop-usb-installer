---
name: china-stock-analysis
description: 核验 A 股行情与财务数据，并执行字段透明的筛选、财务检查、技术指标和显式假设估值场景。
---

# China Stock Analysis

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行 `scripts/...` 时使用绝对路径，
不要假设当前工作目录是仓库根目录。输出写入当前项目或用户指定目录，不写入已安装 Skill 树。

## 适用场景

- 查询 A 股行情并记录数据时点
- 按实时行情接口实际字段筛选股票
- 对已获取的财务报表做确定性指标检查
- 基于真实 K 线计算技术指标
- 用用户审阅、来源明确的假设做 DCF/DDM 情景计算

## 数据规范

行情、公告和财务数据会变化。优先使用当前 Codex 的网页搜索/浏览能力打开交易所、公司公告或用户认可的数据源，
再与脚本结果交叉核验。网页能力不可用时，不得把缓存或脚本结果描述为“最新”。每份报告都保留来源、统计期、
抓取时点、复权方式和假设时点。

依赖 AkShare 的脚本只在锁定运行时健康后启用。接口失败、字段缺失或样本不足时必须停止，不能生成随机数据、
静默忽略筛选条件或返回看似成功的空结果。

## Workflow 1：实时行情

`realtime_quote.py` 使用新浪行情接口，保留行情时点、UTC 抓取时点和接口名。返回值需与交易所/行情终端的时间和
交易状态交叉核验。批量查询中任一代码缺失会在 JSON 包络的 `errors` 中列出并返回非零退出码；不会把部分结果包装成
完整成功。

```text
<python> "<skill-directory>/scripts/realtime_quote.py" 600519 000858 --json
<python> "<skill-directory>/scripts/realtime_quote.py" 600519 --minute --json
```

分时模式只汇总各时段成交量与最高成交量记录，不再从成交量推断“主力”“抢筹”或“出货”。接口失败或个股没有
有效数据时脚本非零退出，也不给出实时交易结论。

## Workflow 2：数据获取

单只股票：

```text
<python> "<skill-directory>/scripts/data_fetcher.py" \
  --code 600519 \
  --data-type all \
  --years 5 \
  --output "<project-data>/stock_data.json"
```

多只股票只支持逗号分隔代码；`data-type` 只接受 `all/basic/financial/valuation/holder`：

```text
<python> "<skill-directory>/scripts/data_fetcher.py" \
  --codes "600519,000858" \
  --data-type basic \
  --output "<project-data>/stocks.json"
```

指数代码列表使用 `--scope hs300/zz500/zz1000/cyb/kcb/all`。不存在 `--industry`、`--top` 或
`--data-type comparison` 入口。

每个结果包含 `requested_components`、`sources` 和 `completeness`。默认只在所有请求组件成功时退出 0；确实希望保留
部分结果时必须显式加 `--allow-partial`，并逐项呈现 `completeness.errors`。只缓存完整结果，接口全失败时不得生成
看似正常的空数据文件。

缓存默认在 `${CODEX_DATA_DIR:-./codex-data}/china-stock-analysis/cache/`，也可用
`CHINA_STOCK_CACHE_DIR` 覆盖。缓存文件名经过净化；已安装 Skill 目录不得用作缓存目录。

## Workflow 3：字段透明的股票筛选

`stock_screener.py` 只暴露实时行情接口中实际使用的动态 PE、PB、总市值和范围筛选。它没有综合评分，也不接受
ROE、负债率或股息率参数；这些财务字段需从定期报告另行核验。

```text
<python> "<skill-directory>/scripts/stock_screener.py" \
  --scope hs300 \
  --pe-min 0 \
  --pe-max 20 \
  --pb-max 3 \
  --market-cap-min 100 \
  --sort-by pe \
  --top 30 \
  --output "<project-data>/screening.json"
```

`scope` 还可取 `all/zz500/zz1000/cyb/kcb/custom:600519,000858`；`sort-by` 只接受
`pe/pb/market_cap`。请求字段不在接口响应中时，脚本非零退出并明确说明条件未执行。

## Workflow 4：财务分析

先取得并核对财报期、是否合并口径、币种和单位，再运行：

```text
<python> "<skill-directory>/scripts/financial_analyzer.py" \
  --input "<project-data>/stock_data.json" \
  --level standard \
  --mode single \
  --output "<project-data>/financial_analysis.json"
```

`level` 为 `summary/standard/deep`，`mode` 为 `single/comparison`。输入必须包含非空 `sources`、
`retrieved_at_utc/data_as_of` 和至少一条 `financial_indicators` 记录。脚本只整理最新值、相邻记录差值、现金流/利润等
可复算比值；不再给盈利/偿债分档，不再输出 0–100 分数、风险评级或股票排名。比较模式只接受包含 `stocks` 数组的
数据文件，并列字段但不自动判定优劣。

`templates/analysis_report.md` 同样只保留来源、原始字段、机械计算、估值场景和人工核验项，不含行业均值占位、目标价或
买卖建议字段。

## Workflow 5：显式假设估值

`valuation_calculator.py` 不再内置 10% 折现率、3% 永续增长、默认现金流/股息、安全边际，也不从历史分位数
反推“合理 PE/目标价”。不同方法不会自动平均，也不输出“高估/低估”或买卖结论。

估值假设文件示意：

```json
{
  "as_of": "<假设时点>",
  "sources": ["<WACC、增长、现金流与股息假设依据>"],
  "dcf": {
    "base_free_cash_flow": 0,
    "cash_flow_growth": 0,
    "discount_rate": 0,
    "terminal_growth": 0,
    "forecast_years": 1,
    "total_shares": 1,
    "basis": "<现金流和股本单位/口径>"
  },
  "ddm": {
    "current_dividend_per_share": 0,
    "dividend_growth": 0,
    "required_return": 1,
    "basis": "<股息口径及增长依据>"
  },
  "margin_of_safety": 0
}
```

零值只表示字段类型，不是默认市场假设。选择 `dcf` 时必须满足折现率大于永续增长率；选择 `ddm` 时要求回报率
必须大于股息增长率。

```text
<python> "<skill-directory>/scripts/valuation_calculator.py" \
  --input "<project-data>/stock_data.json" \
  --assumptions "<reviewed-valuation-assumptions.json>" \
  --methods dcf,ddm,relative \
  --data-as-of "<股票数据时点>" \
  --data-source "<来源名称与 URL>" \
  --output "<project-data>/valuation.json"
```

相对估值只报告输入 PE/PB 和历史分位区间。DCF 当前仅按显式自由现金流计算企业价值场景，未自动调整净债务、
少数股东权益或非经营资产；调用者必须在 `basis` 中说明口径。

## Workflow 6：技术指标

`technical_analysis.py` 只调用 AkShare `stock_zh_a_hist` 的真实前复权 K 线，保留来源、数据时点、复权方式和
样本数；不会生成随机行情兜底。缺少 AkShare、网络失败、字段缺失或有效样本少于 60 条时非零退出。

```text
<python> "<skill-directory>/scripts/technical_analysis.py" \
  --code 000001 \
  --period daily \
  --days 100 \
  --json
```

技术信号只是对输入样本的规则描述，不是价格预测。

## 输出要求

1. 先列股票代码、市场、数据时点、来源和缓存/复权口径。
2. 区分原始事实、脚本启发式、用户假设和 Codex 解释。
3. 不把历史收益、技术信号、分位数或情景估值改写成保证性结论。
4. 缺失字段明确列出；未执行的筛选条件不得出现在“已应用条件”中。
5. 涉及决策时提供反例、敏感性和需人工核验项。

## 数据存储

建议使用 `${CODEX_DATA_DIR:-./codex-data}/china-stock-analysis/` 下的 `stock_data/`、`analysis/`、
`screening/`、`valuation/` 和 `cache/` 子目录。公开能力包不包含用户行情、缓存或分析结果。
