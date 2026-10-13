# 统一媒体 MCP 可靠性改造设计（v1.3.3）

## 范围与验收边界

本次固定目标：让统一媒体 MCP 的 stdio 协议、任务恢复、凭据注入和安装状态表达具备可复现证据；安装后从正常 Codex 启动入口可加载 MCP。只做离线/模拟验收，不发送真实付费生图或视频请求。真实模型线路是否可生成不能由离线健康检查代替。

发布对象是能力包源文件、starter-kit 安装包和 USB 安装器内嵌安装包中的同一媒体 MCP 版本。禁止只更新其中一份。

## 风险判断与设计决定

1. **stdout/stderr 隔离：合理，必须修。** 正常服务进程 stdout 仅写 UTF-8 JSON-RPC 帧；启动/诊断信息走 stderr。无效 JSON-RPC 输入返回协议错误，不能静默丢弃超限帧或无效请求。
2. **UTF-8 原始字节：合理，强化。** wrapper 在启动 Python 前固定 `PYTHONIOENCODING=utf-8`、`PYTHONUTF8=1`；服务端继续显式 UTF-8 bytes。健康检查区分 runtime 与 protocol，并在清除 Python UTF-8 环境变量的子进程中用中文 `initialize`/`tools/list` 验证。
3. **生命周期/超时：部分已满足。** 视频已采用 submit/check/save 异步工具，EOF 会退出，HTTP 已设超时；补齐 JSON-RPC 请求上限/结构错误回包及统一的网络超时错误映射。单次 MCP 请求不启动后台线程。提交按 provider 现有异步任务创建契约等待网关受理，最多设置 60 秒 socket timeout（urllib 无法保证 trickling stream 的严格总时限）；超时落入 unknown 状态，禁止自动重 POST，并测试慢响应。
4. **健康分层：合理，必须修。** `healthcheck` 只证明 Python 与本地协议。readiness 分别记录 runtime、protocol、gateway catalog、generation capability；未做的层标 `not_checked`。模型目录 GET 成功只代表 gateway/catalog 可达，不代表图像或视频生成能力已验证。UI 不得将本地 health passed 显示为生图/视频已就绪。
5. **旧进程/配置：合理。** 安装/修复完成后报告 `codexRestartRequired=true`，若能可靠探测则附 `codexRunning`。不强杀 Codex 或 MCP 子进程；提示关闭并重开 Codex，新会话加载磁盘配置。安装前无需干预其他进程。
6. **版本一致：合理，必须修。** 发布版本统一为 1.3.3，覆盖服务器、启动 wrapper、配置渲染器、安装脚本、manifest、readiness、诊断输出及两份正式包。
7. **异步任务恢复：大体已有，需固化。** 保留 confirmation id/fingerprint、provider/model、task id、状态、时间、响应摘要和 `api_request_sent`。网络调用前先持久化保守标记 `api_request_sent=true` 和 `state=submitting`；进程崩溃留下 submitting 时，下次 submit/check 必须持久化转为 `submission_unknown` 并拒绝重发；无 task id 时提示核对网关/账单，不能猜测恢复。POST 超时/响应解析异常同样进入 unknown；只有已知 task id 的任务可轮询。原始上游响应不完整写入任务文件，只保存脱敏摘要与必要解析字段。
8. **相对 URL/授权传播：v1.3.2 已实现，需回归锁定。** 相对 URL 按当前网关解析；Bearer 仅在 scheme/host/effective port 完全匹配时发送，禁止重定向跟随。保留现有实现并加测试。
9. **参考文件删除：v1.3.2 已实现，需回归锁定。** prepare/submit 前校验本地素材；提交后的 check/save 不再要求原文件存在。
10. **解压包测试：合理，必须做。** 从正式包复制/解压到临时随机目录运行离线回归与 healthcheck；校验 manifest/checksums 与无明文凭据。测试不得依赖开发工作区绝对路径。
11. **验收证据同包同入口：合理，必须遵守。** 全部本地验收针对同一冻结版本、同一解压包、同一 wrapper/config 来源。切换模型或会话不视为修复证据。真实计费验收需要另行明确授权。
12. **凭据来源：当前设计部分满足。** 正常 MCP 启动经过统一 PowerShell wrapper，从受保护的当前用户 DPAPI/签名启动环境取 key；Codex 配置仅指向 wrapper，不持久化 key。增加验收证明普通入口使用同一注入路径，禁止把安装器临时环境变量当唯一来源；日志和包内不得含 key。DPAPI CurrentUser 在不同 Windows 用户/电脑无法解密时应 fail-closed，不允许明文降级。
13. **提交/查询并发：独立审计发现 P2，必须修。** `media_check` 和 `media_save` 与 `media_submit` 共同使用确认记录操作锁。提交进行中时，查询和保存应返回可重试的忙碌错误，不得改写 `submitting` 状态；Windows 使用 `Local\` 命名内核 Mutex，POSIX 使用 `flock`。两者都由操作系统在进程异常退出时回收；Windows 的 `WAIT_ABANDONED` 会接管已被异常放弃的互斥量。不能依赖 Windows CRT 文件字节锁处理进程被强制终止的恢复，本轮回归实际证明该锁在目标环境中会残留阻塞。回归测试覆盖 POST 期间并发查询、子进程持锁被终止后立即恢复，以及正常查询完成。
14. **能力包引用闭包：解压安装验收发现缺项，必须修。** 媒体 MCP 声明依赖 `grok-video-prompt-compiler`，但其 Skill 文件未进入正式包，导致安装中止。将该技能及依赖审计项加入能力包源清单，检查所有 MCP `relatedSkills` 均有完整 Skill 目录，再从独立解压包实际执行安装。
15. **安装 readiness 对象闭包：首次安装演练发现 P1 阻断，必须修。** 运行时 provisioner 返回的状态对象没有 `mediaGeneration.protocol` 等字段，安装器却直接动态赋值；PowerShell 对缺失属性报错，导致 Python/媒体 MCP 本地协议明明通过，安装最终仍标失败。安装器要在解析 provisioner 返回值后补齐媒体状态字段，并在回退初始对象中也预声明同一组字段；用启用媒体 MCP 的隔离用户目录执行完整安装回归。
16. **Grok 参考图上传依赖缺失：独立审计发现 P2，必须修。** 上传脚本原先导入 `httpx`，但离线 CPython wheelhouse 未包含该包；只做本地图像检查的测试没有覆盖上传/下载。改用已经锁定并打入 wheelhouse 的 `requests`，Skill 依赖清单显式锁定 `requests==2.34.2`，预装 Python 健康探针执行 `import requests`；审计同时核对依赖清单、哈希锁、wheelhouse 和运行时探针。新增全离线模拟测试覆盖 multipart 上传、返回 URL、远端 MIME/哈希校验，安装后不再需要临时 pip 安装。
17. **完整包分发一致性：必须复核。** 最新 MCP、Windows Mutex 实现、Grok 上传脚本/测试/文档已同步至 starter-kit 包和 USB 安装器内嵌包；两份清单/校验和均须刷新。USB 包的 Codex 启动器基线哈希与其 `skill-audit.json` 一致。

## 实施文件清单

- `scripts/media-generation-mcp.py`：JSON-RPC 错误回包、UTF-8/health 分层、限时/unknown 任务状态、响应脱敏摘要、工具 capability 健康语义。
- `scripts/start-media-generation-mcp.ps1`：固定 UTF-8 环境；继续保持正常 stdout 只转发 MCP；wrapper 健康输出明确标为本地协议健康。
- `scripts/install-to-codex.ps1`、`scripts/render-codex-config.ps1`：统一版本锁；readiness 分层字段；完成后记录重启要求，不宣称媒体生成已验证。
- `manifest/mcp-servers.json`、`manifest/media-generation-tools.json`、依赖/能力清单及维护文档：记录工具版本和分层验收语义。
- `scripts/test-adapter-contracts.py`：加入协议超限/错误帧、中文 UTF-8、EOF、异步未知态无重发、提交与查询竞态、相对下载 URL/鉴权边界、删除源素材后恢复等离线回归。
- `skills/grok-video-prompt-compiler/scripts/upload_reference.py`：只使用预装锁定的 `requests`；`test_upload_reference.py` 无网络覆盖上传和远端校验。
- `src-tauri/src/app/capability_pack.rs`、`src/shared/types.ts`、`src/app/views/OfflineComponentsSheet.tsx`、对应测试/中英 i18n：展示“本地协议已检查 / 网关目录已检查 / 生图视频能力未做真实付费验证 / 重启 Codex 加载”，不误报就绪；不得杀正在运行的 Codex。
- 两个 `installer-pack` 由各自的 pack builder 同步生成，刷新 `file-manifest.json` 与 `checksums.sha256`。

## 验收条件

1. Python 原始 stdio 在 Windows 非 UTF-8 默认环境下，以 UTF-8 解出中文 JSON-RPC；服务 stderr 中诊断与 stdout 严格分离。
2. `initialize`、`tools/list`、`media_capabilities` 都有结构化响应；EOF 后进程正常退出；超时/超限/无效请求无静默无响应。错误码/ID 明确：JSON parse error `-32700/id=null`，invalid request / oversize `-32600/id=null`；合法 notification 不回包。
3. 所有计费 POST 前保守落盘 `api_request_sent=true`；超时/不确定解析或崩溃残留 submitting 均转为 `submission_unknown`，重复 submit 被拒绝；GET 查询只针对已知 task_id。
4. 相对 `content_url` 可下载；外域、不同端口、重定向不会收到 Bearer Token。
5. 已提交任务即使参考文件被删除，仍可 check/save。
6. wrapper 使用 DPAPI/统一启动注入，不在 TOML、环境永久变量、日志或 ZIP 中泄露 key；版本字段全链路相同。正式 wrapper + rendered MCP config 的本地 host harness 清除安装器临时环境后仍可完成完整协议握手与 capability 调用；测试不同用户无法解密时 fail-closed。
7. 解压后的正式包不依赖仓库路径，通过离线测试、healthcheck、manifest/hash/secret 检查。
8. 安装结果清楚提示需要重启 Codex；媒体生成功能状态保持“能力未真实验证”，除非有明示的付费生成证据。
9. Windows 两个独立进程争用同一 confirmation 时只有一个拿到 Mutex；持锁进程被终止后，下一进程可立即取得 `WAIT_ABANDONED` 并继续恢复状态。
10. Grok 参考图上传使用包内 requests；Skill 清单、哈希锁、wheelhouse、安装后的 Python `import requests` 健康探针都闭合；离线回归检查 multipart 字段、禁用代理、超时、重定向、远端 MIME/大小/SHA-256；全新解压环境不需要联网安装 Python 包。

## 状态字段精确定义

- `runtime`: CPython 3.12+ 与 MCP 文件可运行。
- `protocol`: 隔离 stdio 子进程通过 `initialize`、`tools/list`、`media_capabilities`；中文响应按 UTF-8 原始字节解码。
- `gatewayCatalog`: 仅当带授权的 GET `/v1/models` 成功时为 `verified`，同时记录验证时刻和模型数量；此字段表示最近一次目录读取，不承诺当前上游仍可用。
- `generationCapability`: 本地 key/base/model/route 配置和 MCP 工具契约是否齐备；即使齐备，也不代表真实图片/视频线路已试生成。真实生成字段默认 `not_tested`。
- `codexRestartRequired`: 每次写入 MCP 配置后均为 true。若可检测正在运行的 Codex，则另外报告 `codexRunning`；不因运行状态而自动终止进程。
- MCP `media_capabilities`: 只返回本地凭据是否存在、每个 provider 是否配置、最近一次 catalog 验证时间/目录数、真实 generation `not_tested`；不返回 key 内容，也不自行发网关请求。

## 崩溃恢复规则

`prepared -> submitting` 与“可能已发出请求”标记在同一次原子文件替换中写入。之后任一阶段遇到超时、解析错误或进程重启，都只能转入 `submission_unknown`；不重 POST。只有已知 `task_id` 的记录可由 `media_check` 查询并由 `media_save` 保存。若崩溃残留 `submitting` 且没有 task ID，工具说明用户去网关核对记录和计费；本 MCP 不假设上游支持按幂等键查询。

## 不纳入本轮

- 不强制终止 Codex/MCP 进程；风险高且会丢会话。只检测/提示重启。
- 不做无确认的真实图片/视频生成或模型计费验收。
- 不改变 provider 路由、模型选择、签名注册/授权机制。
