# 连接配置向导

## 目的

`Install-Codex-Starter.cmd` 无参数运行时打开 `scripts/install-wizard.ps1`。向导负责两件事：

1. 安装完整能力包：54 个 Skills、7 个 Agents、Python/Node 运行时、离线依赖、Feishu 与统一媒体 MCP、以及 Codex 托管配置；
2. 让用户按需启用外部连接，而不是把密钥或第三方服务默认打开。

向导不是第二套清单。表单从以下文件生成：

- `manifest/connection-fields.json`：字段标签、类型、是否敏感、环境变量名、默认值、校验和帮助文本；
- `manifest/mcp-servers.json`：MCP 服务、关联 Skills 和运行时要求；
- `manifest/api-services.json`：直连 HTTP/API 服务、关联 Skills 和必需字段。

当前表单包含 2 个本地 MCP（Feishu/Lark、统一媒体生成）、4 个远程 OAuth MCP（Cloudflare、Figma、Linear、Notion）和 1 个直连 API（DashScope 网页搜索）。U 盘版媒体 MCP 默认随包安装；Image-2 自动使用签名 License 中的设备 Key，默认模型为 `gpt-image-2.5`，无需用户重复填写地址或 Key。

## 使用规则

- 每个服务默认不勾选；不勾选就不启用该服务，但完整安装仍继续；
- 选择服务后，向导校验该服务的必填字段、长度、正则、URL 协议/主机/路径和条件必填字段；
- Feishu 的 `user` 认证方式会额外要求 User Access Token；`tenant` 和 `oauth` 不要求该字段；
- 选择 API 或统一媒体 MCP 会同时启用它关联的非核心 Skill；没有选择的服务 Skill 保持禁用；
- 选择 Feishu 会启用 `feishu-doc`、`feishu-drive`、`feishu-perm` 和 `feishu-wiki`，并将 Feishu MCP 配置为启用；
- 选择远程 MCP 会生成单独的 HTTPS MCP 配置并开启 Codex 原生 OAuth 流程；用户仍须在 Codex 中完成账号授权。未选择的服务会生成 `enabled = false` 配置。
- 选择统一媒体 MCP 会启用本地服务并运行 Python 健康检查。Image-2 默认使用签名授权连接的 `gpt-image-2.5` Responses API；Grok、Seedance 和未来开放的 MiniMax 共用同一授权。Seedance 模型或路由 ID 只有在上游要求时才需配置。MiniMax 模型槽位保持空白，直到统一上游确实开放；不调用 MiniMax 官方 API，也不会自动改用其他模型。每次可能计费的生成仍须单独确认，未知提交不会重试。
- 点击“跳过配置，直接安装”会安装完整包，但所有外部连接保持关闭。

敏感字段使用密码框。向导只把非空且已选择服务的值写入当前 Windows 用户级环境变量和当前安装进程；Codex TOML、公开安装包、
readiness、安装清单和日志不保存密钥值。秘密字段如果已经存在于本机用户环境变量，界面不回显；留空会沿用已有值，除非明确勾选
“清理本机已有环境变量”。

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
相关 Skill 必须保持关闭或安全失败，不能声称已访问飞书、已搜索网络、已生成付费图像/视频。U 盘授权无效时，启动器不会向媒体上游发送请求。
