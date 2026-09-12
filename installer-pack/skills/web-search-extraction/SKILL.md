---
name: web-search-extraction
description: 通过用户显式配置的 DashScope OpenAI 兼容接口执行联网搜索摘要。仅在用户明确调用本技能、明确选择 DashScope，或既有工作流已选择该外部适配器时使用；普通网页任务优先使用 Codex 原生网页能力，缺少专用凭据、根地址或实际模型时不得调用。
---

# DashScope 网页搜索适配器

本技能是 `guided-config` 的直连 HTTP 适配器，不是 MCP，也不替代 Codex 原生网页搜索。普通网页任务优先使用当前 Codex 的原生搜索/浏览能力；只有用户明确调用 `$web-search-extraction` 或选择 DashScope 时才使用本技能。

## 脚本路径

先将 `<skill-directory>` 解析为本 `SKILL.md` 所在目录，再使用绝对路径执行：

    <python> "<skill-directory>/scripts/web_search.py" "<查询>" --json

不要假设当前工作目录是仓库根目录。

## 启用条件

- `DASHSCOPE_API_KEY`：DashScope 专用 API Key，只从进程环境读取。
- `DASHSCOPE_BASE_URL`：默认可使用 `https://dashscope.aliyuncs.com/compatible-mode/v1`；必须是无内嵌凭据、查询参数或 fragment 的 HTTPS `compatible-mode/v1` 根路径。不要在这里附加 `/chat/completions`。
- `DASHSCOPE_MODEL`，或命令中的 `--model`：服务当前支持且可使用联网搜索的实际模型名。本技能不猜测模型默认值。
- Python 3.12 与联网能力健康。

脚本会把 `/chat/completions` 拼到根地址后，并发送 `enable_search=true`。缺少 API Key 或模型时必须在网络请求前失败；不得回退读取其他服务的凭据。

## 使用流程

1. 明确查询、时间范围、地区和输出要求。
2. 避免把秘密、个人敏感信息或无必要的内部材料发送给外部服务。
3. 对时效性或高风险结论，优先核验搜索结果中给出的第一方来源；适配器返回的摘要本身不是来源证明。
4. 需要多条查询时逐条执行，并记录每条查询的结果与失败，不把缺失结果伪造成已检索。

示例：

    <python> "<skill-directory>/scripts/web_search.py" "<准确查询>" --model "<实际模型>"
    <python> "<skill-directory>/scripts/web_search.py" "<准确查询>" --model "<实际模型>" --max-words 500 --json

`--max-words` 必须是 1–5000 的整数。响应体最多读取 5 MiB；超限、空结果、HTTP 错误、JSON 错误或网络失败都返回明确错误并以非零状态退出，即使使用 `--json` 也不会虚假成功。

## 输出契约

成功时脚本返回服务的 `message.content`：文本模式直接打印；JSON 模式为：

```json
{"success": true, "content": "服务返回的搜索摘要"}
```

失败时 JSON 为 `{"error":"..."}`，不保证服务返回固定的标题/URL/snippet 数组。只有实际响应中包含可核验链接时才能把它们列为来源；关键事实仍需交叉验证。

## 安全边界

- 不在仓库、命令输出或结果文件中保存 API Key。
- 不把这个 HTTP 适配器登记成 MCP 工具，也不让其他 Skill 硬编码依赖它。
- 健康检查只运行 `--help`、缺配置负例和无网络单元测试；不要为了测试产生不必要的计费请求。
- 用户未明确选择本适配器时，继续使用 Codex 原生网页能力或用户提供的材料。
