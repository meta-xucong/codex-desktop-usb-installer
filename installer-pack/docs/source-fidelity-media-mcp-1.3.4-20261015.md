# 媒体 MCP 1.3.4 来源保真记录（2026-10-15）

## 固定来源

- 来源仓库：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`
- 来源提交：`45413ffab1bab7b9697ef730ac992c711b940ab2`
- 来源路径：`installer-pack/`
- 媒体服务文件：`mcp/media-generation-mcp.py`
- 服务版本：`1.3.4`

## 来源文件 SHA-256

| 来源文件 | SHA-256 |
|---|---|
| `mcp/media-generation-mcp.py` | `98CF9C7960AD2B4843D9209399C57D957958FCA94CF363E96CA51ADAA7109C8B` |
| `mcp/start-media-generation-mcp.ps1` | `AF1532340D08AE57980F4C40169047D34E60AB14FD7BE160FCDEE0D1C4962A1D` |
| `mcp/media-generation.template.env` | `3DA56C9D830F9F48B9401A12DE8AE06F844AF38A9CC0681E4A762B1C2E36EA7B` |
| `manifest/media-generation-tools.json` | `B62DEBEDF94D940C73B50016969DBE6D2BF6EE137588847B327CEA442AAC7FDF` |
| `config-fragments/media-generation-stdio.template.toml` | `E256D4D513304885A00F4496D994068EDBB1C531FF82C8F506E61F8F9FF0622A` |
| `manifest/mcp-servers.json` | `A34FE825B72ED07E72C287D528F87A687719EA8B565D7F08063BB1A011A02000` |

## Starter-kit 基线与映射

- 目标仓库基线提交：`1ee1a9640175710fff52a3c98b0e6cf9fe459c09`。
- 目标工作区已有大量修改和未跟踪文件；这些内容不属于本次媒体同步，不得重置、清理或覆盖。
- 目标服务和包内服务原为 1.3.3、SHA-256 `4AC0EE6D235B4A9FDBEA7CD64AC9F710FB397A4B17E3CCB1C7C869CD3327376A`。
- 本次把固定来源服务字节同步到 `scripts/media-generation-mcp.py` 和 `installer-pack/mcp/media-generation-mcp.py`，两处必须与来源服务 SHA-256 完全一致。
- 同步的工具契约清单目标为 `manifest/media-generation-tools.json` 与 `installer-pack/manifest/media-generation-tools.json`。
- 同步的 Codex 配置模板目标为 `config-fragments/media-generation-stdio.template.toml` 与 `installer-pack/config-fragments/media-generation-stdio.template.toml`。
- MCP 注册清单只改媒体 MCP 节点的版本与健康检查文案：根目录 `manifest/mcp-servers.json` 和包内 `installer-pack/mcp/mcp-servers.json`。保留两份文件中的其他服务器、starter-kit 自有字段和本地改动。
- 启动包装器保留 starter-kit 凭据适配，只把版本期望值从 1.3.3 更新为 1.3.4：`scripts/start-media-generation-mcp.ps1` 与 `installer-pack/mcp/start-media-generation-mcp.ps1`。
- `.env` 模板已与固定来源一致，不需要覆盖。

## 不能复制的差异

USB 包包装器通过已签名的 USB 启动 tuple / 当前用户 DPAPI 读取授权；starter-kit 包装器还读取 starter-kit 的连接配置和进程环境，并从当前用户 DPAPI 读取凭据。整份复制 USB 包包装器会改变 starter-kit 的凭据来源和启动约束。因此只复制服务端字节，包装器仅更新版本检查和配套健康检查文本。

## 1.3.4 变化边界

从固定来源与 starter-kit 1.3.3 服务比较，服务端的变化仅为：版本 1.3.3→1.3.4；内部 HTTP 超时由 60 秒提高到 600 秒，并用于结果下载；能力信息报告该超时；Windows mutex 平台判断抽取；非 Windows `flock` 使用 `LOCK_EX | LOCK_NB`。Starter-kit 配置模板把 Codex 单次工具超时从 180 秒提升到 1500 秒，以覆盖内部网络等待及同步结果下载。该版本同步不代表下文的偶发失败修复已完成。

## 来源保真验收

1. 重新计算来源文件 SHA-256；来源提交或上述任一 hash 变化时，停止并更新此记录与开发文档，不得静默同步新来源。
2. `scripts/media-generation-mcp.py` 与 `installer-pack/mcp/media-generation-mcp.py` 都必须为 `98CF9C...109C8B`。
3. 根目录与包内工具清单、TOML 模板两两字节一致；版本为 1.3.4，工具超时为 1500 秒。
4. 两份 starter-kit wrapper 保留凭据读取与不泄露凭据行为，只要求 serverVersion=1.3.4。
5. 两个 `mcp-servers.json` 中 media-generation 节点声明 1.3.4、600 秒内部超时、1500 秒外部工具超时；不得把 USB wrapper 的凭据描述复制进 starter-kit。
6. 不得把 `scripts/build-pack.py` 的全量重建作为一次定点同步的验证方式；该脚本会覆盖当前包内大量既有差异。正式发布时应在干净、固定的分支上执行完整构建。

