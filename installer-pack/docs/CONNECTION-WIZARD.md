# 连接配置向导

## 目的

`Install-Codex-Starter.cmd` 无参数运行时打开 `scripts/install-wizard.ps1`。向导负责两件事：

1. 安装完整能力包：50 个 Skills、7 个 Agents、Python/Node 运行时、离线依赖、Feishu MCP 包和 Codex 托管配置；
2. 让用户按需启用外部连接，而不是把密钥或第三方服务默认打开。

向导不是第二套清单。表单从以下文件生成：

- `manifest/connection-fields.json`：字段标签、类型、是否敏感、环境变量名、默认值、校验和帮助文本；
- `manifest/mcp-servers.json`：MCP 服务、关联 Skills 和运行时要求；
- `manifest/api-services.json`：直连 HTTP/API 服务、关联 Skills 和必需字段。

当前表单包含 1 个 MCP（Feishu/Lark）和 3 个直连 API（DashScope 网页搜索、Image-2、Seedance），共 19 个连接字段。

## 使用规则

- 每个服务默认不勾选；不勾选就不启用该服务，但完整安装仍继续；
- 选择服务后，向导校验该服务的必填字段、长度、正则、URL 协议/主机/路径和条件必填字段；
- Feishu 的 `user` 认证方式会额外要求 User Access Token；`tenant` 和 `oauth` 不要求该字段；
- 选择 API 服务会同时启用它关联的非核心 Skill；没有选择的 API Skill 保持禁用；
- 选择 Feishu 会启用 `feishu-doc`、`feishu-drive`、`feishu-perm` 和 `feishu-wiki`，并将 Feishu MCP 配置为启用；
- 点击“跳过配置，直接安装”会安装完整包，但所有外部连接保持关闭。

敏感字段使用密码框。向导只把非空且已选择服务的值写入当前 Windows 用户级环境变量和当前安装进程；Codex TOML、公开安装包、
readiness、安装清单和日志不保存密钥值。秘密字段如果已经存在于本机用户环境变量，界面不回显；留空会沿用已有值。明确勾选
“清理本机已有环境变量”时，也只会清理本安装器此前记录拥有的变量名；没有所有权记录的变量永远不会被删除。

## 自动化入口

无界面执行：

```powershell
.\install-all.ps1
```

`.cmd` 的无界面兼容方式：

```powershell
$env:CODEX_STARTER_NONINTERACTIVE = '1'
.\Install-Codex-Starter.cmd -TargetUserProfile 'D:\Temp\CodexProfile'
```

`-TargetUserProfile` 隔离安装时不会把向导中的凭据写入目标 profile；需要真实启用外部连接时，应在安装进程环境中以安全方式
提供对应变量，并单独传入服务启用参数。向导本身面向当前用户安装。

## 安全边界

“安装完成”只表示本地包、运行时和配置闭环完成，不代表第三方服务已经授权或健康。没有凭据、权限、可用模型、端点或健康检查时，
相关 Skill 必须保持关闭或安全失败，不能声称已访问飞书、已搜索网络、已生成付费图像/视频。
