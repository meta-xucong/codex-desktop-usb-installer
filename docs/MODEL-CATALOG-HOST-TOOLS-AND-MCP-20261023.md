# 新增模型与宿主工具 / MCP 诊断及修复（2026-10-23）

## 目标

调查添加第三方模型后 Codex 看不到媒体 MCP、文件和终端工具的报告，修正模型目录中可由源码确认的错误默认值，并明确尚需测试机验证的运行时边界。

## 当前结论

新测试机反馈：添加新模型后，即使再次运行修复组件并修复能力包，MCP 仍不可见。该结果使模型目录或所选模型的运行模式成为主要调查方向，但单凭先后顺序不能确定具体失效字段。

源码能确认新增条目曾把 `include_skills_usage_instructions`、`include_plugin_usage_instructions`、`include_apps_usage_instructions` 都设为 `false`，并设置 `node_repl_disabled=true`。这些是明确的负向默认值：前面三项会省略 Codex 向模型说明如何使用宿主 Skills、插件和 App 能力的说明，最后一项会关闭 Node REPL。模型只通过一次文本响应验证时，不能据此推断这些宿主能力不适用。

模型目录更新只管理 `model_catalog_json` 标记块。`merged_config` 会先去除旧的管理器标记块，再保留其余 TOML 设置并写入新的标记块；其中已有的 `[mcp_servers.*]` 配置不会被模型目录合并主动删除。本次回归测试覆盖替换旧标记块时保留媒体 MCP 段。

因此，已能修复的是新增模型条目不再主动抑制 Skills、插件和 App 的宿主能力使用说明。Node REPL 仍保持禁用。现有证据不能证明使用说明缺失会让 Codex Desktop 整个 MCP、文件和终端工具注册表消失，也不能证明这一源码修复已经解决测试机的完整故障。

## 运行时证据与边界

- 本机使用 Codex MSIX `26.1002.7124.0` 内的 CLI、独立临时 `CODEX_HOME` 和本机回环 Responses 模拟服务做过隔离测试；没有调用 404token 或产生外部模型费用。
- 独立 Think 对四组 CLI 请求做了对照：切换 `node_repl_disabled` 没改变工具数组；打开 Code Mode 改变了工具数组，但测试副本未包含 `codex-code-mode-host.exe`，该路径报告 Code Mode fail closed。
- 这些是 CLI 请求级观察，不是 Codex Desktop 的完整宿主工具注册表，也没有覆盖用户报告中的媒体 MCP 配置和宿主桥。因此它们既不能排除 Desktop 的模型能力过滤，也不能证明媒体 MCP 已注册。
- 新测试机当前没有连入本会话可用的远程设备通道，本轮无法读取其实际 `config.toml`、Codex 启动日志或模型请求工具清单。测试机上的端到端验收仍是开放门禁。

## 修改

新增的已测试模型条目现在允许 Codex 注入 Skills、插件和 App 使用说明。未验证的 Node REPL 能力仍显式禁用，避免把“提供使用说明”误当成“已证明工具可用”。保留 `experimental_supported_tools: []`，因为当前 Codex 目录解析器要求该字段，且部分原生目录条目也使用空数组；不能据此声称已验证完整工具支持。

测试覆盖模型条目字段，以及写入模型目录标记块时保留现有 `[mcp_servers.media_generation]` 内容。

### 媒体 MCP 注册保障

自动安装和组件修复不再把上游媒体模型目录是否可读、是否返回可识别模型作为 MCP 注册开关。能力包照常部署并启用媒体 MCP；缺少模型时继续使用安装器现有默认图片模型，用户仍可在“连接与扩展”重新检测和选择路由，手动关闭 MCP 的设置仍有效。目录请求失败时组件页会保留可读取的本机模型设置。安装器在合并后回读当前 `CODEX_HOME/config.toml`，只有确认 `[mcp_servers.media_generation]` 下 `enabled = true` 才把 readiness 标为已注册。未通过回读时，现有能力包就绪检查会让安装失败并提示修复，而不是报告虚假的成功。

本机验证：在仓库内临时 `CODEX_HOME` 对新 USB 包运行 `install-to-codex.ps1 -EnableMediaGeneration -Overwrite`，退出码为 0；安装后的 `config.toml` 匹配 `[mcp_servers.media_generation] enabled = true`，readiness 的 `registered` 和 `enabled` 都为 true，安装器本地 MCP 协议自检通过。`OfflineHome` 与 `OfflineComponentsSheet` 定向测试共 59 项通过；TypeScript 检查、改动文件 ESLint、安装器契约 58 项、安装链静态测试 10 项及 PowerShell 解析通过。安装包的 542 项文件清单与 543 项校验和逐项匹配。没有调用媒体上游或产生生图费用。

包内 542 个文件及 543 项校验和核对后，使用仓库元数据工具把 `pack.json.sourceTreeSha256` 更新为当前完整能力包树指纹 `3c7bb05a…`，`--check` 通过。SignedRequired 管理器 EXE 位于 `release/setup-v0.5.24-20261023-media-mcp-register-r1`；完整 U 盘目录位于 `release/usb-oneclick-v0.5.24-20261023-media-mcp-register-r1`。构建脚本成功退出，内置验证通过 Codex 载荷、包清单、签名策略、公钥绑定和入口清单；能力包入口脚本与源码 SHA-256 一致。版本化签名 profile 的 key ID 和公钥摘要与现有签名器公开指纹一致，因此不需要更新签名器。独立 `verify-oneclick-usb.ps1 -SkipNetwork -InternalSigned` 最终通过，报告 `ready=true`；Codex MSIX、CCSwitch、VC++ 与 WebView2 均通过验证。

## 本机验证

- `app::model_catalog::tests`: 64 passed。
- Rust 全量测试串行运行：775 passed、0 failed、6 ignored。
- 前端：37 个测试文件、438 项通过；TypeScript 检查和 ESLint 通过。
- 首次发行脚本全量运行时，有 1 项 Skill Hub 重试测试在并行套件中偶发失败；该项单独重跑通过，随后串行全量 Rust 套件全部通过。发行构建因该次测试失败先行停止，后续使用已完成的全量验证结果构建测试 EXE。
- 本机隔离 Codex CLI 测试不等同于 Desktop 或测试机端到端验证，不能关闭下一节所述门禁。

## 测试机验收步骤

1. 安装新管理器后先不修复能力包，记录实际 `CODEX_HOME`、`config.toml` 中 MCP 段、`codex mcp list` 结果及 Codex 重启前后差异。
2. 用管理器添加已测试模型并重启 Codex，再记录相同信息；确认媒体 MCP 配置段仍在，区分“配置存在但未启动”和“配置已删除”。
3. 分别用原生模型和新增模型，在同一 CODEX_HOME 下检查 Codex 的工具注册及本地文件读取能力，并各自实际调用一次媒体 MCP 完成端到端验证。保存脱敏后的 app-server 初始化/工具清单、调用结果和对应模型目录行。
4. 若原生模型可用而新增模型缺工具，将新增模型行与原生模型行逐字段对照，并用同版本 Codex CLI/App Server 的隔离配置复现；若两种模型都不可用，转查宿主桥、MCP 启动和会话初始化，不将其归因于模型条目。
5. 单独执行能力包修复后再重启，检查是否改变上述 MCP 段和工具清单。修复组件成功提示不能代替端到端工具调用。

只有第 3 步确认新模型工具清单正常、本地文件读取成功，并实际成功调用媒体 MCP，才能关闭本次测试机验收门禁。
