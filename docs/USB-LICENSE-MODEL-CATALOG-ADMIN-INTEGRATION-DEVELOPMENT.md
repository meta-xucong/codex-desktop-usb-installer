# U 盘授权后台：模型目录一键刷新与整理


**状态：**实现、独立审计、本地测试和授权后台部署验收已完成；当前记录源码发布状态。

**日期：**2026-10-15

**目标：**在授权管理后台增加一个管理员入口，启动固定版本的 Sub2API 模型目录管理器，对配置目标的全账号执行目录同步、分类、按已批准规则应用映射并展示摘要。

**原则：**只改授权后台和独立管理器；不改 Sub2API 本体、镜像、容器或业务逻辑。一次点击会真实更新 目标实例的原生上游目录元数据，并可能按已批准规则写入账号映射。

## 1. 冻结基线、来源和现状

- 授权后台源码位于 `codex-desktop-usb-installer`；实施前基线提交为 `45413ffab1bab7b9697ef730ac992c711b940ab2`。
- 既有工作区内容均予以保留；本次集成和发布严格限定在 §6 列出的文件。
- 授权服务位于 server/usb-license-revocation。管理页由同一 Go 服务提供，页面资源嵌入二进制；管理员接口现有会话、Origin、CSRF 校验可复用。
- 部署目标仅为授权管理后台主机，不是 Sub2API VPS。服务器地址、域名解析、服务运行状态和磁盘信息不纳入公开开发文档。
- 管理器仓库：https://github.com/meta-xucong/sub2api-model-catalog-manager。设计冻结时的审查基线为 `ed86554f1eb6a3a6bca802293bb2caaf6230f6bc`；包含最终结构化结果协议和重复刷新门禁的实现提交为 `b02edf364074978d5af67695635a37554a53b631`，见 §9。
- 固定来源清单 SHA256：e6bb35b9969c34aa783e6f19a4f4a5011656301743020a37fe826ea97ccfb495。算法为下列路径及各自小写 SHA256，以“路径 空格 sha256 换行”拼接后计算 SHA256。该 SHA256 只固定设计期审查的来源清单。最终实现以 §9 列出的管理器提交为准；后续增量仅限本文明确记录的文件和行为：
  - cmd/model-catalog-manager/main.go
  - cmd/model-catalog-manager/main_test.go
  - internal/report/screen_summary.go
  - internal/config/config.go
  - internal/securecred/credential_linux.go
  - internal/securecred/credential_linux_test.go
  - internal/app/discover.go
  - internal/app/workflow_test.go
  - internal/app/apply.go
  - internal/app/inspect.go
  - internal/app/inspect_test.go
  - internal/adminapi/client.go
  - internal/adminapi/client_test.go
  - internal/state/state.go
  - internal/state/state_test.go
  - internal/officialcatalog/startup.go
  - internal/officialcatalog/cache_linux.go
  - internal/officialcatalog/officialcatalog_test.go
- 管理器与授权服务均已完成 Linux amd64 构建并在目标授权后台完成启动验证；构建产物和部署验收摘要见 §9。

## 2. 范围和边界

### 本次实现

1. 管理页增加“模型目录”面板和“刷新并整理模型”按钮。
2. 授权服务后端提供一个异步单任务执行器、任务状态读取和安全摘要展示。
3. 后端只启动固定管理器二进制和固定配置，执行全账号模式。管理器复用现有官方生命周期资料同步、账号目录同步、分类规则、映射备份、写入前复读和写后 readback。
4. 页面明确说明：点击会刷新所有符合条件的 目标实例 账号目录，并按当前已批准规则写入映射；目录出现不等于模型生成已验证。
5. 本地用合成 Sub2API API 和假管理器验证集成；部署后只验服务健康、登录保护和页面加载，不代用户启动真实全账号管理器任务。
6. 本地验收通过后，将编译好的授权服务与模型管理器部署到授权管理后台主机。

### 明确不做

- 不改 Sub2API 仓库、数据库、容器、provider、分组、用户数据、计费或业务逻辑。
- 本次测试不连接真实 Sub2API，不触发原生目录同步，不写任何真实账号映射，不发模型生成请求。部署后管理员可在页面自行点击开始真实任务。
- 不改签名、授权记录、授权数据库结构、吊销逻辑或 USB 客户端。
- 不把 Sub2API 管理 key、管理器配置、筛分规则、账号详情、凭据、原始子进程输出或任意服务器文件路径返回浏览器。
- 不提供浏览器可填写目标 URL、命令、配置文件、账号范围、模型规则或本地路径的能力。
- 不新增定时任务、SSH 远控功能、通用作业队列、数据库表、Docker 服务或后台框架。
- 原始冻结任务单曾排除 GitHub 提交/推送；2026-10-15 后续授权已允许发布本次管理器增量和授权后台集成源码，实际状态见 §9。

## 3. 来源行为映射和最小适配

| 行为 | 固定来源 | 处置 |
|---|---|---|
| 管理员会话、Origin、CSRF、同源请求 | 授权服务现有 admin_web.go 和管理页 JS 请求封装 | 原样复用 |
| 全账号目录发现、官方资料同步、规则筛分和映射应用 | 固定模型管理器 | 调用固定 full-screen 流程，不在授权服务复制规则 |
| 管理员凭据读取 | 管理器 Linux systemd credential reader | 用 LoadCredential 注入，key 不进 argv、环境变量、网页或日志 |
| 原生刷新状态检查 | 管理器 adminapi.RefreshStatus 和 inspect.go 的状态白名单 | full-screen 发现开始前、每个账号映射写入前调用；running、stale_running、disabled、unavailable、空/未知状态或 API 错误均阻断。completed、succeeded、partial_failure、never_run 视为已知非运行状态 |
| 刷新与映射写入竞争 | 原生 Sub2API 刷新没有可供外部管理器持有的原子互斥锁 | 两次检查是尽力而为的时间点保护，不是原子互斥；原生刷新可能在检查之后启动。不得宣称可绝对避免并发 |
| 账号重读、映射备份、写后 readback | 管理器 ApplyPrepared 和 state 包 | 原样复用；失败时停止，不自动重试 |
| 管理器任务锁 | 管理器 internal/state 的 run.lock | 原样复用，不自动删除遗留锁；PID/锁文件异常时以人工核实为准 |
| 完整终态结构 | 管理器 CLI | 增加一种受限 JSON 结果行，覆盖每种 full-screen 终态；不改筛分和映射语义 |
| 授权后台启动、轮询和显示 | 授权服务 | 只新增薄执行器和小面板 |

原生刷新检查的保守规则：检查失败、空值、disabled、unavailable、stale_running 或未列举的新状态一律 fail closed；查询结果为 running 时 fail closed。只有 completed、succeeded、partial_failure、never_run 允许继续；每次账号映射写入前重新查。这个检查只能降低并发概率，不能代替 Sub2API 原生锁。

## 4. 页面和 HTTP 契约

面板只显示当前/最近状态、开始/结束时间、受限原因、完成/失败计数、安全摘要和启动按钮。运行中每 2–3 秒轮询；不伪造逐模型实时进度。计数只统计计划中的映射账号；任务运行中尚无可信终态结果，或已启动但中断且没有可信结果时，计数显示“未知/不可用”，不可显示为 0。按钮旁固定说明会刷新所有符合条件账号并按批准规则更新映射。摘要用 textContent 展示，不把 Markdown 当 HTML 执行。

接口均为同源 /admin/api/ 路径，复用现有管理员 Cookie、Origin、CSRF：

- POST /admin/api/model-catalog/runs：无业务参数。没有运行任务时启动一个；已有任务运行时返回现有任务，不重复启动。
- GET /admin/api/model-catalog/runs/current：返回唯一当前/最近任务的受限状态和安全摘要。
- 不提供任意 run ID、命令、配置路径、目标 URL 或账号列表输入。

后台任务状态固定为 idle、running、succeeded、no_plan、partial、blocked、failed、interrupted。no_plan 表示“没有可应用计划”，不等于“模型无变更”；UI 必须显示准确字样，不可把它改称 no_changes。失败、阻断、partial、缺少有效终态结果均不得显示成功。partial 不自动重试。

状态映射固定如下：succeeded 仅表示 manager 正常退出、全账号扫描完整且计划账号全部完成；no_plan 表示全账号扫描完整但没有可应用计划；blocked 表示已知前置门禁阻止执行且本轮没有发送映射写入；partial 表示全账号扫描不完整，或非成功终止前至少一个映射写入已通过管理器验收。即使映射计划本身已全部写入、但后续本地状态保存或摘要生成失败，也仍标为 partial，具体结果看计数；failed 仅表示 manager 返回了有效失败终态，或可确认子进程尚未启动；interrupted 表示 manager 已启动但未返回有效终态（服务重启、超时、强制停止、缺失/重复/非法结果标记），映射结果可能不完整且未知。interrupted 不自动重试，UI 明确提示先核对报告和备份。已完成的 no-op 不算映射写入；只有 manager 正常退出、全账号扫描完整且全部计划均已写入或 no-op 才能显示 succeeded。

管理器 full-screen 输出恰好一行结果标记，JSON schema 固定为：
schema_version、status、reason_code、planned_count、applied_count、noop_count、failed_count、not_attempted_count、summary_basename。
字段约束：
- status 仅为 succeeded、no_plan、partial、blocked、failed。
- reason_code 只允许以下固定值：completed、no_applicable_plan、account_scan_partial、native_refresh_running、native_refresh_status_unavailable、native_refresh_status_unknown、manager_lock_requires_review、configuration_invalid、credential_unavailable、discovery_failed、approved_rules_blocked、apply_partial、apply_failed、process_timeout、manager_result_invalid、service_interrupted。native_refresh_running 表示 running 或 stale_running；native_refresh_status_unavailable 表示状态 API 请求失败或返回 unavailable；native_refresh_status_unknown 表示空值、disabled 或未列举状态。manager CLI 仅能输出管理器可知的终态原因码，包括 account_scan_partial；process_timeout、manager_result_invalid、service_interrupted 由授权服务映射，不由 manager 输出。不得包含错误原文、URL、key、账号详情或文件路径。
- 所有可信终态计数均以账号为单位且为非负整数。planned_count 是审批计划账号数；applied_count 是本轮 mapping 写入被管理器接受且 readback 与批准目标一致的账号数（完全沿用固定版本 ApplyPrepared 对返回结果和 readback 的接受语义）；noop_count 是计划目标已满足、经 Apply 预检核实且在逐账号阶段确认的账号数；failed_count 统计已进入单账号映射写请求调用、但未通过管理器完整响应/readback 验收的当前账号；进入调用不代表可证明网络端已收到请求。not_attempted_count = max(0, planned_count - applied_count - noop_count - failed_count)，表示其余计划账号在终止前未完成逐账号写入/核验。若错误发生在映射写请求调用前，failed_count=0，未完成的计划账号都计入 not_attempted_count。计划形成前受阻时所有计数为 0。代码需要给当前失败写入账号设置一个内部 FailedID，只在结果 JSON 暴露计数，不暴露账号 ID。
- summary_basename 只能是空值或 screen-summary-<UTC时间>-<32位小写hex runid>.md basename，不能是路径。
- partial/blocked/failed 可带已产生的安全摘要；如果没有摘要，basename 为空。
- 全部可正常返回的 full-screen 终态都输出且只输出该 JSON 行；阻断/失败返回非零退出码。操作系统强制终止可能没有结果行，此时授权服务将任务记为 interrupted。非 full-screen CLI 输出保持原样。
- 子进程一旦启动，若未输出恰好一个合法终态结果或达到 45 分钟后被终止，授权服务显示 interrupted，并分别使用 manager_result_invalid 或 process_timeout；这表示映射副作用未知，计数为 null 并显示“未知/不可用”，不从原始 stderr 猜状态，也不自动重试。若子进程无法启动且没有执行，才显示 failed，计数可显示已知的 0。合法 failed 结果若 failed_count 大于 0，页面明确提示该账号的写入结果未核实，需先检查摘要和备份；不自动重试。

## 5. 执行、凭据、状态和故障恢复

### 固定配置和秘密

- 服务端配置固定 manager 路径、配置路径、状态目录、最大运行时间 45 分钟、摘要上限 2 MiB；请求不得覆盖。
- 管理器 config 必须明确设置 预先配置的目标实例、account_scope=all、allow_full_screen_mode=true、allow_native_sync_metadata_write=true、accept_all_credential_response_memory_exposure=true、apply_mode=approved_rules、已审定规则 SHA256 和 systemd_credential_name=admin-api-key。缺任何全账号门槛即启动失败，不回退到 allowlist 或 preview。
- 凭据以 systemd LoadCredential 注入授权服务，文件内容使用管理器要求的 JSON 格式 {"api_key":"..."}，源文件仅 root 可读。授权服务进程只将 credential 路径传给 manager 进程；不把 key 展开进命令行、环境变量、应用状态、日志、HTTP 响应或页面。
- systemd 给服务设置 XDG_CACHE_HOME=/var/lib/codex-usb-license-revocation/model-catalog/cache。状态、报告和备份目录限定在该主机现有授权服务私有数据目录下，创建时 owner 为 codex-license-revocation、目录 0700、文件 0600。现有 ReadWritePaths 已包含 /var/lib/codex-usb-license-revocation；保留其余 systemd 沙箱，不增加宽泛可写目录。

### 子进程和任务状态

- POST 返回后，子任务使用独立于 HTTP request context 的 45 分钟超时 context；不承诺由 Go 服务生命周期 context 优雅取消。systemd 通过 KillMode=control-group 在停机时终止服务和 manager 子进程。单任务互斥。
- stdout/stderr 只保存在有大小上限的进程内缓冲区用于解析 JSON；总量超过 1 MiB 即终止已启动的子进程，并按 interrupted / manager_result_invalid 处理，因为映射副作用可能未知；结果标记缺失/重复/无效也按此处理。原始输出不落盘、不进服务日志、不返回浏览器。
- 授权服务由 systemd 管理，unit 必须显式设置 KillMode=control-group 和 TimeoutStopSec=15s，使服务和 manager 子进程由 systemd 一并停止。服务重启后把持久化的 running 转成 interrupted，不自动重跑；不声称服务本身已在停机阶段写入 interrupted。部署验收核对 unit 的 KillMode；本地合成测试验证 running 状态重启后变成 interrupted。
- 管理器 run.lock 由 O_EXCL 创建，进程异常退出可能留下锁文件。不得自动删除或仅凭 PID 断定锁已过期。下一次启动若发现该锁，显示 blocked / manager_lock_requires_review；管理员必须先核实对应管理器进程确已不存在，再由主机维护者手工删除唯一的 state/run.lock 文件。界面不提供清锁按钮，不自动重试。正常完成时由管理器自身释放锁。
- 授权后台当前任务状态以原子 JSON 文件持久化，启动时遗留 running 变为 interrupted；状态文件权限 0600。服务状态不含账号 key、上游 URL、原始日志或任意文件路径。
- 报告只允许读取固定 report_dir 内的 basename。必须用拒绝符号链接的安全打开方式（Linux openat/O_NOFOLLOW 或等价实现），核验 regular file、路径仍在固定目录、大小不超过 2 MiB；不接受符号链接和路径穿越。报告响应使用纯文本，不允许 HTML。
- 部署对象只限授权服务二进制、manager 二进制、受保护 manager config/rules、systemd unit 和新增 credential 文件。部署前将旧授权服务二进制、旧 unit、现有 manager 二进制（若存在）及非秘密 config/rules（若存在）备份到授权服务专用 root-only 目录；不得把 credential 内容复制进普通备份。回滚顺序固定为：停止服务；恢复旧授权服务二进制；若旧 manager/config/rules 存在则恢复原件，否则删除本次新建文件；恢复旧 unit；删除本次新增 credential 源文件；systemd daemon-reload 后启动服务并检查健康。若凭据文件部署前已存在则不得覆盖或删除。不得触碰 Sub2API VPS 或其他授权主机数据。

## 6. 实现文件边界

### 授权服务仓库

只允许修改以下已存在源码及新增的 runner/测试/文档；其他已有 dirty 文件一律保留：
- server/usb-license-revocation/internal/revocation/http.go
- server/usb-license-revocation/internal/revocation/config.go
- server/usb-license-revocation/internal/revocation/admin_web.go
- server/usb-license-revocation/internal/revocation/adminweb/index.html
- server/usb-license-revocation/internal/revocation/adminweb/app.js
- server/usb-license-revocation/internal/revocation/adminweb/style.css
- 新增 server/usb-license-revocation/internal/revocation/model_catalog_runner.go 和同目录测试
- 更新授权管理 HTTP/页面测试
- server/usb-license-revocation/deploy/codex-usb-license-revocation.service
- 本文或单独的本次验收记录

### 固定管理器来源仓库

只允许修改：
- cmd/model-catalog-manager/main.go 与 main_test.go：仅补 full-screen 结构化终态结果和安全 reason_code。
- internal/app/discover.go、internal/app/apply.go 与 internal/app/workflow_test.go：发现前和每个账号映射写入前检查原生刷新状态；错误、running、stale_running、unavailable、空值或未知状态均阻断。检查仍只是 best-effort。
- 冻结来源清单中的其他文件只用于来源核对，不修改。
- 不改筛分规则、官方生命周期数据、映射决策、报告格式、Sub2API API、其他管理器功能。保留该仓库已有内容；不向 GitHub 提交或推送。

## 7. 审计、测试和放行

### 文档先行审计

独立设计 A2 和来源保真 A2 对本文件的同一 SHA256 只读审计。任一未 PASS，不写实现代码。修改后计算新 SHA，并让两项审计均针对新 SHA 重跑。

### 本地实现验收

- 命令：授权服务仓库 gofmt、go test ./...、Linux amd64 build；管理器 gofmt、go test ./...、Linux amd64 build；git diff --check。
- 合成 HTTP/假管理器测试覆盖：未认证/Origin/CSRF；并发双击；请求返回后任务仍运行；结果状态；partial/block/failure；服务启动时 running 转 interrupted；超时终止子进程且结果标记为副作用未知、不自动重试；限制输出；合法/缺失/重复/非法 JSON；摘要 symlink/越界/超限/XSS；无 key/路径/原始错误泄露。systemd unit 静态测试及部署核对必须确认 KillMode=control-group、TimeoutStopSec=15s。
- 管理器合成测试覆盖：全账号 exposure gate；刷新状态运行、stale_running、unavailable、未知、空值和查询错误阻断；检查期间原生刷新启动后，在每个账号映射写入前能再次阻断；已知终态允许继续；结构化结果在所有 full-screen 终态恰有一行，计数、摘要 basename 和退出码一致；遗留 run.lock 阻断且不自动删除。
- 本地综合测试全部使用合成 Sub2API API 和假管理器，不连接 目标实例、不调用真实目录同步、不写真实映射、不发模型请求。真实执行仅由管理员部署后在页面手动启动。
- 独立代码 A2 与独立 source-fidelity A2 在同一冻结 diff 上审计；发现问题先修正，再对新冻结版本重审。
- 部署前只读核验授权服务主机至少 500 MiB 可用空间。建立授权服务专用、权限受限的可恢复备份；只保留当前/上一版授权服务二进制供回滚，不清理其他目录或 Sub2API 镜像。
- 部署后做服务健康、loopback HTTP、HTTPS 页面、登录/CSRF保护检查；不点击“刷新并整理模型”按钮，不执行真实管理器任务。检查失败时按 §5 的顺序停止服务，并恢复旧授权服务二进制、旧 manager 二进制（若存在）、manager config/rules（恢复旧文件或删除新文件）、unit；删除本次新增 credential 源文件而不备份其内容；daemon-reload 并启动服务。不改 Sub2API。
- 最终记录列出本地测试命令/退出码、审计结论、部署主机、变更文件和部署 smoke test；明确未执行真实全账号整理。

## 8. 原始冻结任务单（TaskSpec）与后续授权

以下 TaskSpec 记录实施前的范围冻结；§9 记录之后的授权变更和实际完成状态。

- **目标：**在授权管理页面增加单击入口，运行固定版本管理器，更新 目标实例 全账号上游目录、按已批准规则整理映射、向管理员展示准确结果。
- **原始不做项：**Sub2API 本体改动；模型生成请求；定时任务；任意账号/地址/命令输入；自动清理 manager 遗留锁；Git 提交或推送（GitHub 发布限制已由后续授权更新，见 §9）。
- **硬约束：**保留全部预先存在的工作区修改；只经原生 Sub2API 管理 API；浏览器和日志不接触 key；既有 admin/CSRF 保护；每账号写映射前都查原生刷新状态；不得把 best-effort 描述成原子互斥；结果异常闭锁。
- **授权后台基线：**HEAD 45413ffab1bab7b9697ef730ac992c711b940ab2，已有脏文件必须保留。
- **管理器审查基线：**commit `ed86554f1eb6a3a6bca802293bb2caaf6230f6bc`；设计期固定来源清单 SHA256 `e6bb35b9969c34aa783e6f19a4f4a5011656301743020a37fe826ea97ccfb495`。最终实现还包含 §9 记录的后续增量提交。
- **放行证据：**设计 A2 PASS；来源保真 A2 PASS；代码 A2 PASS；两仓库本地合成测试、Linux amd64 构建、diff 检查通过；授权后台主机部署 smoke test 通过。
- **原始授权范围：**按本文件开发、本地合成测试并部署编译产物到授权后台；不包括真实全账号任务或 Sub2API VPS/本体/数据库/镜像。GitHub 发布范围由后续明确授权扩展，实际情况见 §9。

## 9. 实施、验收与发布记录（2026-10-15）

- 模型管理器源码已发布到其 GitHub 仓库 `main`：`b02edf364074978d5af67695635a37554a53b631`。它以设计期审查基线 `ed86554f1eb6a3a6bca802293bb2caaf6230f6bc` 为基础，增加 full-screen 结构化终态结果和发现/逐账号写入前的原生刷新状态门禁。管理器最终 Linux amd64 部署二进制 SHA256：`95d50ea6038c57563ccf09f1573084ec1f79df5b3f5564900f1517242b9a56b9`。
- 授权后台集成只包含 §6 列出的服务端、页面、systemd、测试和本说明文档；预先存在的其他脏改动与未跟踪文件均未纳入本次发布。授权服务二进制 SHA256：`6499ecfcfa45b9ec1a0daa4670a36e17fe24a985171b8cdd574cf389e0302e79`。
- 两个 Go 仓库的 `go test -count=1 ./...` 均通过；已通过 Linux amd64 构建、代码边界审计和来源保真审计。授权后台部署后的服务健康、登录保护、页面加载及未认证 API 拒绝检查通过。管理器结构化输出与刷新门禁的修订已完成独立审计；本文件移除主机识别信息并更新版本状态。
- 本次没有启动真实 Sub2API 全账号目录刷新/模型分类任务，没有发送模型生成请求，没有修改 Sub2API 数据或部署 Sub2API。
- 后续明确授权允许将管理器实现增量和本授权后台集成源码发布到各自 GitHub 仓库；该授权替代本文件原始 TaskSpec 中“不推送”的限制。授权后台源码按本次审核过的文件边界提交，不包含其他工作区改动。
- 公开文档不记录授权后台主机 IP、域名解析结果、服务实时状态、磁盘信息、凭据或本地绝对路径。
