---
name: feishu-wiki
description: 通过已配置的官方飞书/Lark MCP 浏览知识空间、读取节点信息和子节点，并在用户明确要求时复制、创建、移动或重命名 Wiki 节点。Use when 用户提到飞书知识库、Wiki 空间、页面树或 wiki 链接；页面正文读取交给 feishu-doc。
---

# 飞书知识库导航

## 前置检查

确认 MCP server `feishu` 已连接且工具名使用 dot case。Wiki `space_id`、`node_token` 和 `obj_token` 都是不透明字符串，
即使只包含数字也必须保持字符串类型。

## Token 提取

从 `https://example.feishu.cn/wiki/ABC123def` 提取 `ABC123def` 作为节点 token。不要把 URL 中其他路径片段或查询参数拼入 token。

## 工具映射

| 目的 | 官方 MCP 工具 | 行为 |
|---|---|---|
| 读取空间信息 | `wiki.v2.space.get` | 只读 |
| 列出可访问空间 | `wiki.v2.space.list` | 只读 |
| 读取节点信息 | `wiki.v2.space.getNode` | 只读 |
| 列出子节点 | `wiki.v2.spaceNode.list` | 只读 |
| 复制节点 | `wiki.v2.spaceNode.copy` | 写入 |
| 创建节点 | `wiki.v2.spaceNode.create` | 写入 |
| 移动节点 | `wiki.v2.spaceNode.move` | 写入 |
| 更新节点标题 | `wiki.v2.spaceNode.updateTitle` | 写入 |

## 导航流程

1. URL 只有节点 token 时，用 `wiki.v2.space.getNode` 解析 `space_id`、`obj_token` 和 `obj_type`。
2. 浏览空间时用 `wiki.v2.space.list` 或 `wiki.v2.space.get`，再用 `wiki.v2.spaceNode.list` 分页列出子节点。
3. 读取 docx 页面正文时，把节点返回的 `obj_token` 交给 `$feishu-doc`；不要把 node token 当作文档 token。
4. 用户明确要求复制、创建、移动或重命名且目标唯一时，调用对应写工具；完成后重新读取节点验证。
5. 当前基线不编辑 Wiki 页面的正文。正文修改请求应生成待粘贴内容并说明 `$feishu-doc` 的只读边界。

## 权限与失败处理

至少需要 `wiki:wiki:readonly` 才能导航，写操作需要更高权限。空列表可能是权限过滤而不是空间不存在；分页标志仍为 true 时继续读取。
连接、权限或工具缺失时返回真实限制，不臆造页面树。
