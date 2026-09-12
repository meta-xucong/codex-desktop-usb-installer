---
name: feishu-drive
description: 通过已配置的官方飞书/Lark MCP 浏览云盘文件和文件夹、读取元数据，并在用户明确要求时复制、建文件夹、移动或删除已有项目。Use when 用户提到飞书云空间、文件夹、云盘链接或文件整理；不支持文件内容上传和下载。
---

# 飞书云盘操作

## 前置检查

确认 MCP server `feishu` 已连接，工具名使用 dot case，且目标文件或文件夹已对当前应用或用户授权。连接不可用时只给配置步骤，
不要声称已列出或修改云盘。

## Token 处理

从 `https://example.feishu.cn/drive/folder/ABC123` 提取文件夹 token。所有 file、folder 和 task token 均作为不透明字符串处理。

## 工具映射

| 目的 | 官方 MCP 工具 | 行为 |
|---|---|---|
| 列出文件夹内容 | `drive.v1.file.list` | 只读 |
| 批量读取元数据 | `drive.v1.meta.batchQuery` | 只读 |
| 复制文件 | `drive.v1.file.copy` | 写入 |
| 创建文件夹 | `drive.v1.file.createFolder` | 写入 |
| 移动文件或文件夹 | `drive.v1.file.move` | 写入 |
| 删除文件或文件夹 | `drive.v1.file.delete` | 高风险写入，进入回收站 |

始终使用 MCP 工具提供的实际 schema，不复用旧平台 action 参数。

## 工作流

1. 先用 `drive.v1.file.list` 确认父目录和目标项目；有分页时读取到满足用户范围或分页结束。
2. 需要核对标题、所有者、创建时间和类型时用 `drive.v1.meta.batchQuery`。
3. 只有用户明确要求相应变更且目标唯一时，才调用复制、创建、移动或删除工具。
4. 删除、移动或覆盖语义不清时，先列出解析出的目标 token、名称和父目录并询问缺失信息。
5. 变更后重新读取目标目录，报告真实结果；调用失败时不要把计划描述成已完成。

## 限制

- 官方 MCP 当前不支持文件内容上传或下载。不要尝试用 prepare/finish 分片接口拼装替代实现。
- tenant token 下的机器人通常没有个人根目录，只能访问已共享给应用的文件或文件夹。
- 用户要求读取文档正文时转用 `$feishu-doc`，不要把云盘元数据当成正文。
