# 飞书 MCP 实现核查

核查日期：2026-09-07。结论只用于能力包的连接引导，不代表本仓库分发第三方运行时或凭据。

## 推荐候选

| 候选 | 许可证/发布物 | Windows 运行时 | 维护与制品 | 风险判断 |
|---|---|---|---|---|
| [larksuite/lark-openapi-mcp](https://github.com/larksuite/lark-openapi-mcp) | MIT；npm `@larksuiteoapi/lark-mcp@0.5.1` | Node.js >=20；锁定本地 Node/npm cache | Lark 官方仓库；提供 npm 包；README 明确标注 Beta | 最可信候选，但 API 可能变化，仍需应用权限、App ID/Secret；官方 CLI 接受 `-s/--app-secret` 参数，因此使用本机进程环境变量加本地包装器，避免 Secret 落入 Codex 配置和日志 |
| [cso1z/Feishu-MCP](https://github.com/cso1z/Feishu-MCP) | MIT；npm `feishu-mcp`，仓库有 v0.2.3 | Node.js + pnpm/npm；支持 STDIO/SSE | 第三方，近期仍有提交；README 提供 npm、源码和 Docker 路径 | 功能覆盖广但权限面较大；第三方包、凭据缓存和 user/tenant 模式需要独立审查 |
| [shou-nian/feishu-mcp-server](https://github.com/shou-nian/feishu-mcp-server) | 仓库未声明许可证；Python `>=3.11` + uv | Python/uv；STDIO | 代码和测试目录完整，但暂无 release、预编译 Windows 制品或许可证声明 | 可读源码但不能作为可合法再分发的默认制品；保持候选/人工批准 |
| [Lark-Base-Team/lark-base-mcp-node-server](https://github.com/Lark-Base-Team/lark-base-mcp-node-server) | README 声称 MIT；npm `@lark-base-open/mcp-node-server` | Node.js；只覆盖 Base/Bitable | 面向多维表格，仓库规模和发布信息有限 | 不是文档/云盘/Wiki 的完整替代；仅在用户明确需要 Base 时单独接入 |

## 结论与安装边界

官方实现已经足够可靠，可以登记为 `mcp.feishu` 的推荐候选，但它仍是 `guided-config`：

1. Starter Kit 捆绑经版本和 SHA-256 锁定的 Node.js、npm closure 与 CLI，但不捆绑 App Secret 或 OAuth 会话；安装器只从包内介质离线解压。
2. 安装器部署包装器与工具清单；Node/npm 制品的来源、SHA-256、npm lock 和许可证信息随运行时 manifest 保存。包装器禁止 `npx` 和网络下载。
3. 连接板块让用户选择 tenant/user/OAuth 模式并配置应用权限。Codex 的 `env_vars` 只转发 `FEISHU_APP_ID`、`FEISHU_APP_SECRET` 和可选的 `FEISHU_USER_ACCESS_TOKEN`；Secret 不得写入 `config.toml`、日志或安装记录。上游子进程仍会收到 Secret 参数，本机高权限进程检查可能看到它。
4. 只有离线包、进程环境变量和包装器 dry-run 全部通过后，配置生成器才允许生成 `enabled=true` 的 `[mcp_servers.feishu]`；失败时保持禁用。
5. 工具名统一使用官方 dot case，并由 `manifest/feishu-tools.json` 锁定。当前文档工具只读；官方实现明确不支持文件上传、文件下载和直接编辑云文档正文。
6. 写操作默认进入 Codex 的 `writes` 审批模式；删除文件、删除权限成员和转移所有者强制逐次 `prompt`。

来源：

- [官方仓库 README](https://github.com/larksuite/lark-openapi-mcp/blob/main/README.md)
- [官方 CLI 文档](https://github.com/larksuite/lark-openapi-mcp/blob/main/docs/reference/cli/cli.md)
- [官方 package.json](https://raw.githubusercontent.com/larksuite/lark-openapi-mcp/main/package.json)
- [官方 MIT 许可证](https://raw.githubusercontent.com/larksuite/lark-openapi-mcp/main/LICENSE)
