---
name: feishu-doc
description: 通过已配置的官方飞书/Lark MCP 读取云文档标题、纯文本和块结构。Use when 用户提供飞书 docx 链接或文档 token，或要求读取、摘要、提取和分析飞书云文档；当前基线只读，编辑请求应输出待粘贴内容并说明连接限制。
---

# 飞书云文档读取

## 前置检查

1. 确认 MCP server `feishu` 已连接且健康。
2. 确认工具名按配置使用 dot case；不要臆造旧式单工具 action。
3. 连接或权限不可用时，说明缺失项并停止外部调用。可以继续处理用户已经粘贴的内容。

## Token 处理

从 `https://example.feishu.cn/docx/ABC123def` 提取 `ABC123def` 作为文档 token。所有 token、block ID、revision ID
都按不透明字符串传递，即使内容只包含数字也不要转成数值。

## 工具映射

| 目的 | 官方 MCP 工具 |
|---|---|
| 读取文档基本信息 | `docx.v1.document.get` |
| 读取纯文本正文 | `docx.v1.document.rawContent` |
| 深度列出文档块 | `docx.v1.documentBlock.list` |
| 读取单个块 | `docx.v1.documentBlock.get` |

调用时遵循 MCP 返回的实际参数 schema，不猜测字段名。如果所列工具未出现在当前工具目录中，把它视为连接或工具 allowlist
未就绪，不要替换成相似名称。

## 读取流程

1. 用 `docx.v1.document.get` 获取标题和版本信息。
2. 普通摘要或全文分析先用 `docx.v1.document.rawContent`。
3. 用户需要表格、图片位置、代码块或层级结构时，再用 `docx.v1.documentBlock.list` 分页读取全部块。
4. 只需补查一个块时使用 `docx.v1.documentBlock.get`。
5. 输出标题、来源链接或 token、读取时间、内容摘要和结构化信息；分页未完成时明确标注。

## 写入边界

当前锁定的官方 Lark MCP README 仍声明不支持直接编辑云文档，并且不支持文件上传/下载。因此：

- 不承诺替换全文、追加、更新块、创建表格、上传图片或附件。
- 用户要求编辑时，先读取现有内容，再生成可审阅的 Markdown/结构化修改稿。
- 只有未来锁定版本、工具清单和健康检查都明确支持写入后，才能另行扩展；不得仅因为 OpenAPI 工具列表出现写接口就假定当前 MCP 已支持。

## 权限

至少需要文档只读权限，例如 `docx:document:readonly`。权限不足时返回实际错误和所需 scope，不声称已经读取。
