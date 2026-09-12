---
name: feishu-perm
description: 通过已配置的官方飞书/Lark MCP 检查文档权限、列出协作者，并在用户明确指定目标和成员时增加、修改、移除协作者或转移所有者。Use when 用户提到飞书共享、访问权限、协作者或所有权；权限变更属于敏感写操作。
---

# 飞书权限管理

## 前置检查

确认 MCP server `feishu` 健康，并确认当前身份具备所需 `drive:permission` 权限。所有 token、成员 ID 和部门 ID 都按不透明字符串传递。

## 工具映射

| 目的 | 官方 MCP 工具 | 风险 |
|---|---|---|
| 检查当前用户权限 | `drive.v1.permissionMember.auth` | 只读 |
| 列出协作者 | `drive.v1.permissionMember.list` | 只读 |
| 读取公开访问设置 | `drive.v1.permissionPublic.get` | 只读 |
| 添加协作者 | `drive.v1.permissionMember.create` | 写入 |
| 修改协作者权限 | `drive.v1.permissionMember.update` | 写入 |
| 移除协作者 | `drive.v1.permissionMember.delete` | 高风险写入 |
| 转移所有者 | `drive.v1.permissionMember.transferOwner` | 高风险且可能难以恢复 |

## 工作流

1. 先读取当前权限和协作者，确认目标 token、文件类型、成员标识类型和当前权限。
2. 对写操作要求用户请求中明确包含目标、成员和期望权限；缺一项就先询问，不猜测 email、open_id 或 user_id。
3. 不自行扩大权限范围，不把 `view` 自动升级为 `edit` 或 `full_access`。
4. 移除协作者或转移所有者前，复述精确目标；如果原始请求已经清晰授权该精确操作，可直接执行，否则先补齐歧义。
5. 调用后重新列出协作者或检查权限，报告变更前后差异和任何未完成项。

## 失败处理

权限不足、成员不存在、token 类型错误或工具未启用时，返回实际错误和所需信息。不要声称权限已经生效，也不要使用其他写工具绕过审批策略。
