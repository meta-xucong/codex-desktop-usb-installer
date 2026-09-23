# Codex 模型列表刷新与同类发布别名合并开发文档

**文档版本:** 1.0  
**状态:** `IMPLEMENTED_AND_AUDITED`（纯前端合并逻辑、Codex 下拉映射、自动测试已落地；最终发行包待本轮重建）  
**适用版本:** CCSwitch Codex 供应商编辑面板；不改变签名工具、账户注册、CCSwitch 自动注入 Key 和 U 盘授权链路。  
**基线时间:** 2026-09-21  

## 1. 目标

用户在 Codex 的供应商配置中点击“获取/刷新模型列表”后，程序从当前供应商的 OpenAI 兼容 `GET /v1/models` 获取最新列表，并按以下原则展示：

1. 保留所有厂商和所有模型家族，不建立厂商白名单，不限制为 GPT。
2. 新出现的模型直接进入列表，例如 `gpt-6`、`deepseek-v4`、`glm-5`、`qwen-next`，不需要预先改代码中的模型名单。
3. 只把明显属于同一模型的发布别名合并为一个选项：日期后缀、`latest`、`stable`。
4. `mini`、`pro`、`reasoner`、`coder`、`vision`、`instruct`、`thinking` 等语义后缀不做特殊判断，因而天然保持为独立选项。
5. 下拉列表只出现一个模型选项；真正发送给上游的仍是一个确定的原始模型 ID，不能把展示名称误当成请求 ID。

本方案解决的是“刷新后列表太乱”的问题，不负责判断某模型是否可调用，也不自动更换用户当前已经选择的模型。

## 2. 审计后的最小落点

当前 CCSwitch 已经具备完整的刷新链路，不需要新增安装器页面或另造一套 HTTP 接口：

| 现有能力 | 代码证据 | 审计结论 |
| --- | --- | --- |
| 调用模型接口 | `CCSwitch/CCSwitch/src-tauri/src/services/model_fetch.rs:50-100` | 已通过 Bearer Key 请求兼容的 `/v1/models`，超时 15 秒，解析 `data[].id/owned_by`。复用。 |
| Codex 刷新按钮 | `CCSwitch/CCSwitch/src/components/providers/forms/CodexFormFields.tsx:80-105,146-181` | 已有获取按钮和加载/错误提示。只在成功结果进入下拉前增加合并。 |
| 下拉分组 | `CCSwitch/CCSwitch/src/components/providers/forms/shared/ModelInputWithFetch.tsx:37-79` | 已按 `ownedBy` 分组；当前缺少同类别名合并和展示名/原始 ID 分离。 |
| 离线目录 | `installer-pack/model-catalog.json`、`installer-pack/sync-model-catalog.ps1` | 这是安装阶段快照，不是运行时刷新入口。本任务不把它误当作在线列表，也不改其离线职责。 |
| 既有模型归一化 | `CCSwitch/CCSwitch/src-tauri/src/services/session_usage_codex.rs:59-99` | 仅用于用量/定价归一化，且会剥离 provider 前缀；不能直接复用到 UI 去重，避免跨供应商误合并。 |

因此，本任务的最小实现位置是 CCSwitch 的 Codex 供应商编辑面板：复用现有“获取模型列表”按钮和 API，只新增一个纯前端、可单元测试的别名折叠函数，并让下拉项支持“展示名”和“请求 ID”分离。

如果验收目标改成“官方 Codex 原生窗口的模型下拉框必须被外部程序实时改写”，则不是本方案范围：当前工作区没有已确认的官方运行时模型目录写入契约，不能猜测 `models_cache.json` 或其他内部文件路径。该目标需要另立设计和真实版本验证。

## 3. 范围与非范围

本文件继续只约束 CCSwitch 的 Codex 供应商编辑面板。现场要求“统一网关模型进入官方 Codex 原生模型菜单”已经超出本文件范围，具体运行时探测、目录桥接、版本门禁和降级策略见 [`CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md`](CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md)。不得用本文件的 CCSwitch 下拉测试代替原生 Codex 菜单验收。

### 3.1 本次范围

- 在现有 Codex 模型刷新结果上做别名折叠。
- 保留 `owned_by`，以供应商归属作为去重命名空间和 UI 分组依据。
- 保存每个展示项对应的一个原始请求 ID。
- 刷新失败、鉴权失败、超时或空列表时，保留上一次可用列表和当前模型输入，不覆盖用户状态。
- 为规则、选择优先级和边界情况补充离线测试。

### 3.2 明确不做

- 不维护 GPT、DeepSeek、GLM、Qwen 或其他厂商的硬编码白名单。
- 不过滤“看起来小众”的模型，不根据模型名猜能力，不根据日期新旧决定是否可用。
- 不把 `mini/pro/reasoner/coder/vision/instruct/thinking` 等后缀合并。
- 不后台定时联网，不在启动时强行修改默认模型。
- 不把 API Key、模型列表或用户选择写入签名文件、许可证、U 盘授权数据或公开安装包。
- 不修改 `unified-api-internal` 分组、注册编号、账户绑定和 CCSwitch 环境变量注入逻辑。

## 4. 数据契约

现有接口的有效响应最小形态为：

```json
{
  "object": "list",
  "data": [
    { "id": "deepseek-chat-20260101", "owned_by": "deepseek" },
    { "id": "deepseek-chat-latest", "owned_by": "deepseek" }
  ]
}
```

内部展示项建议扩展为：

```ts
type DisplayModel = {
  // 发送给上游的真实 ID
  id: string;
  // 下拉框展示文本；没有别名时可与 id 相同
  displayName: string;
  // 保留现有供应商分组信息
  ownedBy: string | null;
};
```

`id` 是唯一的请求值；`displayName` 只用于下拉菜单。用户仍可手动编辑模型 ID，刷新不会强制覆盖输入框。

## 5. 别名折叠规则

### 5.1 只移除明确的发布别名

对模型 ID 仅做以下尾部处理，大小写匹配不敏感：

- `-latest`
- `-stable`
- `-YYYY-MM-DD`，且日期必须是有效日历日期
- `-YYYYMMDD`，且日期必须是有效日历日期

如存在多个连续的上述尾缀，可从右到左重复处理，直到没有可识别尾缀。除此之外不删除任何字符，不猜测任何厂商规则。

伪代码：

```text
canonicalId(rawId):
    id = trim(rawId)
    repeat:
        if id ends with -latest or -stable:
            remove that suffix
        else if id ends with a valid -YYYY-MM-DD or -YYYYMMDD:
            remove that suffix
        else:
            stop
    return id
```

### 5.2 去重键必须保留供应商边界

```text
groupKey = caseFold(trim(ownedBy or "Other")) + "\u001f" + caseFold(canonicalId(rawId))
```

这意味着：

- 相同 `owned_by` 下的 `deepseek-chat`、`deepseek-chat-latest`、`deepseek-chat-20260101` 合并。
- `deepseek-chat` 与 `deepseek-reasoner` 不合并。
- `deepseek-chat` 与 `deepseek-chat-pro` 不合并。
- 不同 `owned_by` 的同名模型不合并。
- 没有 `owned_by` 的模型进入 `Other` 命名空间，不与有归属信息的同名模型合并。

### 5.3 原始 ID 选择优先级

同一分组有多个原始 ID 时，选择一个稳定、可复现的请求值：

1. 精确等于规范化基础 ID 的原始 ID；
2. `-stable`；
3. `-latest`；
4. 有效日期后缀中日期最新者；
5. 仍相同则按大小写不敏感的字典序选择。

展示名使用规范化基础 ID；如果原始列表只有日期/别名版本，也仍显示基础 ID，但发送所选的原始 ID。这样既不丢失上游真实 ID，也不会在下拉框中重复显示一堆日期版本。

## 6. 示例

输入列表中包含：

```text
deepseek-chat
deepseek-chat-2025-01-01
deepseek-chat-20260101
deepseek-chat-latest
deepseek-reasoner
glm-5
glm-5-20260101
glm-5-pro
gpt-6
gpt-6-20260901
qwen-next
```

下拉列表应显示为：

```text
deepseek-chat       -> 请求 deepseek-chat
deepseek-reasoner   -> 请求 deepseek-reasoner
glm-5               -> 请求 glm-5
glm-5-pro           -> 请求 glm-5-pro
gpt-6               -> 请求 gpt-6
qwen-next           -> 请求 qwen-next
```

如果接口没有返回 `deepseek-chat`，只有 `deepseek-chat-20260101`，仍显示一个 `deepseek-chat`，并将 `deepseek-chat-20260101` 作为请求 ID。下一次刷新若上游返回更新日期，映射随刷新更新；不会静默修改已保存的用户配置。

## 7. 建议改动文件

本次实际写入边界如下：

| 文件 | 改动 |
| --- | --- |
| `CCSwitch/CCSwitch/src/lib/model-aliases.ts`（新文件，或等价的现有纯函数模块） | 实现 `collapseReleaseAliases(models)`、规范化、分组和原始 ID 选择。不得发网络请求、读写配置或持有 Key。 |
| `CCSwitch/CCSwitch/src/lib/api/model-fetch.ts` | 如需扩展 `FetchedModel` 类型，增加可选 `displayName` 类型字段；不改变现有请求契约。 |
| `CCSwitch/CCSwitch/src/components/providers/forms/CodexFormFields.tsx` | 成功获取后先调用折叠函数，再把结果放入 `fetchedModels`；成功数量提示应使用折叠后的选项数，并可附带原始数量。 |
| `CCSwitch/CCSwitch/src/components/providers/forms/shared/ModelInputWithFetch.tsx` | 下拉项显示 `displayName`，选择时仍回调原始 `id`；其他供应商调用保持兼容。 |
| `CCSwitch/CCSwitch/tests/` | 增加纯函数测试和至少一个 Codex 下拉选择测试。 |

不修改 `installer-pack/model-catalog.json`、`installer-pack/sync-model-catalog.ps1`、`src-tauri` 管理器账户模块、签名工具或 404TOKEN 后端。

## 8. 测试与验收

### 8.1 纯函数测试

必须覆盖：

- `latest`、`stable`、ISO 日期、紧凑日期均可合并。
- 无效日期（例如月份/日期越界）不被删除。
- `mini`、`pro`、`reasoner`、`coder`、`vision`、`instruct`、`thinking` 等只因没有发布别名尾缀而保持独立；不需要维护后缀白名单。
- `deepseek`、`glm`、`qwen`、`gpt` 等不同厂商全部保留。
- 相同规范化 ID 但不同 `owned_by` 不合并。
- 原始 ID 选择优先级稳定，尤其是“基础 ID优先、日期取最新”。
- 空 ID 被忽略或由现有接口层拒绝，不能生成空下拉项。
- 输入顺序变化不会导致分组结果随机变化。

### 8.2 组件测试

- 刷新成功后，日期/`latest` 版本只出现一个展示项。
- 选择展示项时回调的是原始 ID，而不是规范化展示名。
- 刷新失败时保留旧列表和当前输入。
- 接口返回空 `data` 时不清空已有可用列表。
- 没有 Key 或 Base URL 时沿用现有友好错误提示。

### 8.3 构建与回归

在 `CCSwitch/CCSwitch` 执行前端命令，在 `CCSwitch/CCSwitch/src-tauri` 执行 Rust 命令：

```powershell
# 前端：CCSwitch/CCSwitch
pnpm typecheck
pnpm test:unit --run

# Rust：CCSwitch/CCSwitch/src-tauri
cargo test --locked --lib model_fetch
cargo clippy --locked --all-targets -- -D warnings
```

然后重新构建随包 CCSwitch，更新 `src-tauri/resources/ccswitch/manifest.json` 的 SHA-256，并运行现有 `tools/test-ccswitch-env-patch.ps1`。环境变量注入、自动保存统一 API Key、签名工具和账户注册回归必须保持原有结果。

### 8.4 通过标准

只有同时满足以下条件才可标记完成：

- 新模型不需要改白名单即可在刷新后出现。
- 同一模型的日期/`latest`/`stable` 版本只显示一个选项。
- 语义不同的模型不被误合并。
- 下拉展示名与真实请求 ID 分离正确。
- 网络失败不会破坏已有选择。
- CCSwitch 前端、Rust、打包哈希和既有 USB/404TOKEN 回归全部针对最终版本通过。

## 9. 安全与兼容性

- 刷新请求继续复用现有 Key 注入和 Bearer 认证；Key 不进入日志、模型目录文件或下拉展示。
- 模型 ID 来自服务端，展示时只做文本处理，不执行、不拼接脚本、不写入 TOML 之外的命令参数。
- 本功能只更新当前表单内存中的候选项；用户点击保存前不改变持久化配置。
- 上游若返回未知格式或没有 `data`，按现有错误/空结果处理，不用本地猜测列表冒充成功。
- 离线安装仍使用现有静态快照；刷新失败不影响已安装的 Codex、Skills、账户或 CCSwitch 启动。

## 10. 审计结论

### 已确认没有的设计问题

- 不会因为硬编码厂商或 GPT 白名单而漏掉未来的 `gpt-6`、DeepSeek、GLM、Qwen 等模型。
- 不会把 `mini/pro/reasoner/coder` 等实际不同模型错误合并；算法只识别有限的发布别名尾缀。
- 不需要修改签名、账户、统一分组 Key 或 U 盘授权协议。
- 复用现有 `/v1/models` 和 Codex 刷新按钮，改动面小、无新增网络权限。

### 必须保持的边界

- 本文交付的是“CCSwitch Codex 供应商编辑面板刷新列表”的开发契约，不宣称已经改写官方 Codex 原生模型缓存。
- 实施前必须保持 `displayName -> raw id` 映射；只做字符串去重而丢失原始 ID 属于拒收。
- 任何将别名折叠扩展为厂商白名单、能力筛选、自动默认模型切换或后台定时刷新，均属于范围外变更，需要重新定界。

**最终审计结论：符合，可交付实现。已确认不会建立厂商白名单；日期/latest/stable 只在同一 owned_by 命名空间内合并，语义后缀保留；下拉显示名与真实请求 ID 分离。最终发行包仍须使用本轮构建产物完成哈希和完整包验证。**
