# {{stock_name}}（{{stock_code}}）可核验分析记录

> 本模板区分原始观测、机械计算、用户假设和分析解释，不包含综合评分、股票排名、目标价或买卖建议。

## 1. 身份与来源

| 项目 | 内容 |
|---|---|
| 市场 / 股票代码 | {{exchange}} / {{stock_code}} |
| 股票名称 | {{stock_name}} |
| 数据抓取时点 | {{retrieved_at}} |
| 数据统计期 | {{data_periods}} |
| 来源及链接 | {{sources}} |
| 复权 / 币种 / 单位 | {{data_basis}} |
| 完整性状态 | {{completeness_status}} |

### 缺失或失败的组件

{{component_errors}}

## 2. 公司与行情原始观测

| 字段 | 值 | 来源时点 / 接口 |
|---|---:|---|
| 行业 | {{industry}} | {{basic_source}} |
| 总市值 | {{market_cap}} | {{basic_source}} |
| 流通市值 | {{float_cap}} | {{basic_source}} |
| 最新价 | {{latest_price}} | {{price_source}} |
| 动态 PE | {{pe_ttm}} | {{valuation_source}} |
| PB | {{pb}} | {{valuation_source}} |

## 3. 财务字段

### 最新记录

| 指标 | 值 | 统计期 | 口径说明 |
|---|---:|---|---|
| ROE | {{roe}} | {{financial_period}} | {{roe_basis}} |
| ROA | {{roa}} | {{financial_period}} | {{roa_basis}} |
| 毛利率 | {{gross_margin}} | {{financial_period}} | {{gross_margin_basis}} |
| 净利率 | {{net_margin}} | {{financial_period}} | {{net_margin_basis}} |
| 资产负债率 | {{debt_ratio}} | {{financial_period}} | {{debt_ratio_basis}} |
| 流动比率 | {{current_ratio}} | {{financial_period}} | {{current_ratio_basis}} |
| 总资产周转率 | {{asset_turnover}} | {{financial_period}} | {{asset_turnover_basis}} |

### 可复算差值与比值

{{mechanical_diagnostics}}

这些值只反映输入记录之间的机械计算。使用者必须核验记录顺序、合并范围、会计政策、币种和单位；模板不使用固定阈值自动评级。

## 4. 技术指标观测（可选）

| 项目 | 内容 |
|---|---|
| K 线来源 | {{technical_source}} |
| 数据时点 | {{technical_as_of}} |
| 周期 / 样本数 / 复权 | {{technical_basis}} |
| 指标值 | {{technical_indicators}} |
| 规则观测 | {{technical_observations}} |

历史 K 线和指标不构成价格预测或交易信号。

## 5. 估值场景（可选）

| 项目 | 内容 |
|---|---|
| 假设时点 | {{assumptions_as_of}} |
| 假设来源 | {{assumption_sources}} |
| DCF 场景 | {{dcf_scenario}} |
| DDM 场景 | {{ddm_scenario}} |
| 相对估值观测 | {{relative_observations}} |
| 未建模项目 | {{valuation_limitations}} |

各方法必须并列展示，不自动平均为“公允价值”。安全边际仅是用户输入值下的算术场景，不是买入价。

## 6. 证据、反例与待核验项

### 已核验事实

{{verified_facts}}

### 用户提供的假设

{{user_assumptions}}

### 可能推翻当前解释的证据

{{counterevidence}}

### 下一步人工核验

{{human_review}}

## 方法边界

{{methodology}}

本记录用于组织可追溯信息和机械场景，不构成证券研究评级、信用评级、收益承诺或投资建议。
