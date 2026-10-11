# U 盘许可证远程作废与自动清理开发文档

- 文档状态：待实施设计
- 版本：1.0
- 日期：2026-10-09
- 目标产品：Codex USB Installer
- 部署目标：`https://license.404token.xyz`；需在本地验收通过后再配置到正式构建并部署

## 1. 目标、范围与硬约束

### 1.1 目标

为已签名的 U 盘许可证增加远程状态检查。通过 U 盘管理器启动 Codex 时，管理器先访问 VPS 并完成授权检查：

- 服务端确认许可证有效：按现有本地许可证门禁继续启动。
- 服务端明确、签名确认许可证已作废：阻止启动，并在后台清理这套 U 盘安装所管理的 Codex 程序和可重新安装载荷。
- 服务端不可访问、超时、响应格式异常、签名不通过或状态不明确：不触发卸载；本地许可证及设备指纹有效时按离线策略启动。

此功能只针对单个 USB 安装身份，不停用用户电脑上的其他 Codex 安装。

### 1.2 不可变硬约束

| ID | 约束 |
|---|---|
| C1 | 只有验签成功、请求随机数匹配、产品/许可证/安装身份匹配且状态明确为 revoked 的 VPS 响应，才能启动自动清理。 |
| C2 | DNS、连接、TLS、超时、HTTP 错误、服务端异常、未知记录、响应解析错误、签名错误和身份不匹配不能产生新的 revoked 凭证或启动新的清理。若此前已保存、且重新验签和身份绑定均有效的 revoked 凭证，只能继续该凭证已经授权的幂等清理；网络错误本身不是清理授权。 |
| C3 | 本地许可证签名、U 盘指纹、Bundle 完整性以及策略要求的登记声明均有效而 VPS 不可用时，按离线规则允许启动；不得把网络错误误判为作废。 |
| C4 | 本地保留的有效服务端状态按最高 stateVersion 决定当前状态：最高状态为 revoked 时，即使后续离线也继续阻止启动并重试清理；只有更高状态版本的有效 active 响应才能解除作废门禁。历史 revoked 回执可作为清理记录保留，不得覆盖更高 active。状态文件可被有文件访问权的用户删除/回滚；在离线 fail-open 策略下不承诺抵抗主动篡改。 |
| C5 | 作废响应到达后先保存作废凭证并阻止 Codex，再清理文件；任何清理失败都保持阻止状态并可重试。 |
| C6 | 自动清理只针对本产品清单中明确管理的 Codex 运行文件和重装载荷。不得清理用户配置、项目、CCSwitch、其他 Codex 安装、任意用户文件或整个 U 盘。 |
| C7 | VPS 私钥只存在 VPS 端；安装器内只包含服务端验签公钥。不得把管理员密钥、API 秘钥或签名私钥写进 U 盘客户端。 |
| C8 | 卸载过程幂等、可中断、可恢复；不能把部分成功报告成彻底完成。 |
| C9 | 不改变现有 UsbLicense schema、既有许可证签名载荷或正常离线启动策略。正式 USB 签名流程额外生成独立登记声明；远程状态是附加门禁。 |
| C10 | 不能把启动器描述成防篡改 DRM。用户能读取 U 盘文件，文件级门禁不能阻止有能力的用户复制或绕过程序。 |
| C11 | 新版远程作废 Bundle 使用 manifest 文档 schemaVersion=3，并在现有签名锚定的不可变 manifest 中声明登记声明必需。该策略启用时，登记声明缺失或无效属于本地不完整，必须阻止 Codex 操作并要求修复；VPS 网络不可用仍按 C3 离线放行。经验证的历史 schemaVersion=2 manifest 未启用此策略，才允许 remote_unenrolled 离线运行。USB anchor 算法仍使用现有 anchorSchema=3；不得混淆文档版本和 anchor 算法版本。 |
| C12 | 只有服务端状态信封验签通过、请求 nonce/requestId 和产品、许可证摘要、installationId 均匹配，且状态明确为 revoked，才允许触发作废清理。本地许可证/签名/指纹异常、其他状态、身份不匹配、网络离线、服务异常或响应无效均不得触发自动清理。 |
| C13 | 每次由管理器实际启动 Codex（包括主题切换后的重启）都执行新鲜的远程状态检查。U 盘 manifest 入口由 shim 接管并转交管理器；普通双击必须受同一门禁控制。主动篡改或提取载荷不属于 DRM 防护目标。 |

### 1.3 本轮非目标

- 不替换现有许可证签名器，也不改变 UsbLicense 的签名 schema；为其增加向后兼容的远程登记凭证生成能力。
- 不自动清理用户的 %USERPROFILE%\\.codex、%APPDATA%\\Codex 或 %LOCALAPPDATA%\\Codex。
- 不清理 CCSwitch、MCP 配置、Skill 能力包或用户项目。
- 不扫描电脑磁盘并删除其他位置的 Codex。
- 不在 VPS 故障时阻止本地签名有效的用户启动。
- 不立即远程关闭一个已经运行、且尚未再次请求状态的客户端。
- 不新增面向终端用户的注册/账单/客户关系门户；运维管理界面按本文第 13 节增补设计实现。
- 不上传完整 UsbLicense/accountBinding；API Key、邮箱等账号绑定字段留在 U 盘本地。

## 2. 当前实现基线与复用评估

本方案是新增文档，实施前仍需在代码变更任务中重新确认基线。当前观察到的实现如下：

| 当前模块 | 已有职责 | 本方案的复用方式 |
|---|---|---|
| crates/codex-usb-license/src/lib.rs | UsbLicense、installationId、USB 指纹和本地 Ed25519 验签 | 保持现有许可证签名字节格式；增加独立登记声明的固定字节构造/校验，远程状态响应另设协议结构，禁止混用签名语义。 |
| tools/usb-license-signer/src/ | 正式许可证验签、签名及 USB 目标确认 | 扩展 sign/register 流程，在已验签 USB 许可证上生成不含 accountBinding 的独立登记声明；不得把完整许可证上传 VPS。 |
| src-tauri/src/app/usb_guard.rs | 正式启动前本地许可/指纹检查、受限状态、lease、由管理器启动的进程登记 | 复用本地预检与受限启动结构；增加远程状态检查和作废状态；Codex 在远程结果未确定前不可启动。 |
| src-tauri/src/app/usb_bundle.rs | USB Bundle 清单、SHA256SUMS、目录结构和不可变内容验证 | 复用现有清单校验。兼容 schemaVersion=2 历史 manifest；schemaVersion=3 必须验证由现有签名锚定保护的 `revocationPolicy.registrationRequired` 策略，以及登记声明与状态文件；仅在有效 revoked 回执下允许清理目标缺失，登记/状态文件不加入普通不可变摘要。 |
| src-tauri/src/app/atomic_file.rs | 原子写入工具 | 复用写入机制保存作废凭证和清理事务状态。 |
| src-tauri/src/app/uninstall_cleanup.rs | 现有卸载后的脱离进程清理交接模式 | 可复用“当前进程退出后由 helper 处理”的运行模式；必须使用专用、窄范围的作废清理计划。 |
| src-tauri/src/app/usb_media_cleanup.rs | 清理整个 USB Bundle 的路径、清单、reparse point 保护 | 仅复用安全校验思路或提取小型共用函数；不得直接调用其整盘/整个 Bundle 清理入口。 |
| src-tauri/src/app/win_update.rs | Codex 运行树和 Windows 安装卸载实现 | 复用路径识别、进程关闭和结果报告的安全模式；不得直接调用宽范围的 usb bundle 完整卸载。 |
| src-tauri/src/commands.rs | win_uninstall_bundle 完整卸载协调，包含 Codex、CCSwitch、机器包状态和用户选择 | 不直接调用。它包含额外产品、数据及系统状态，不能作为静默自动作废清理器。 |
| tools/build-oneclick-usb-layout.ps1、src-tauri/build.rs | 正式 Bundle 构建、签名策略和编译期策略 | 增加正式服务地址、公钥 ID、公钥构建配置，并校验正式构建不能遗漏或使用占位值。 |
| src/app/views/OfflineHome.tsx、src/shared/types.ts | 前端展示启动、许可证及受限状态 | 展示“在线检查中”“授权已作废/清理中”“清理未完成”等面向用户的状态，不显示内部工程术语。 |

当前 U 盘运行时路径由 Bundle 描述为 apps/Codex，恢复/安装载荷位于 payload/OpenAI.Codex_x64.msix。普通卸载保留用户数据和 U 盘安装包，完整卸载协调器还管理 CCSwitch 与其他 Windows 状态。因此作废流程需要专用清理计划，不复用完整卸载命令，也不能只删除 apps/Codex 而留下可直接重装的 Codex MSIX 载荷。

### 2.1 文件写入范围建议

实施任务首次冻结后，预计改动边界如下；如源码现状不符，应先更新 TaskSpec，不可顺手扩大范围。

本地客户端：

- 新增 src-tauri/src/app/usb_revocation.rs：登记声明校验、HTTP 检查、响应验签、状态分类、作废记录读写。
- 修改 src-tauri/src/app/mod.rs：注册新模块。
- 修改 src-tauri/src/app/usb_guard.rs：门禁状态、启动前远程检查、作废后不创建授权 lease。
- 修改 src-tauri/src/lib.rs：确保状态检查在任何 Codex 操作开放之前完成。
- 修改 src-tauri/src/app/win_update.rs 或新增 src-tauri/src/app/usb_revocation_cleanup.rs：只卸载本 Bundle 拥有的 Codex 运行时和载荷。
- 修改 src-tauri/src/app/uninstall_cleanup.rs：仅在确需复用 detached finalizer 时增加作废任务类型，不能改变普通卸载语义。
- 修改 src-tauri/src/commands.rs：提供受限的状态查询/清理重试命令；不添加可由前端任意传入路径的删除接口。
- 修改 src-tauri/src/app/usb_bundle.rs 和 tools/build-oneclick-usb-layout.ps1：新 root manifest 与 usb-manifest 使用文档 schemaVersion=3，两个版本字段必须一致；root manifest 必须包含 `revocationPolicy.registrationRequired=true`，由现有 USB anchor/manifest 签名链保护；anchorSchema 仍为 3。读取端继续兼容 schemaVersion=2 历史 Bundle，但不为它们承诺远程作废。固定登记声明与 revocation state 位置、签名校验、可变文件清单、作废恢复完整性例外及正式构建配置。schemaVersion=3 缺字段或策略无效必须校验失败，不能默认为 false。
- 修改 crates/codex-usb-license/src/lib.rs、tools/usb-license-signer/src/transaction.rs：增加登记声明结构、固定 payload 字节构造与验证；保持既有 UsbLicense schema 和 license 签名载荷不变。
- 修改 tools/usb-license-signer/src/command.rs、gui.rs：新 schemaVersion=3 Bundle 签名完成后原子生成登记声明；register 仅为已具备必需登记策略且现有 UsbLicense 指纹仍匹配的 Bundle 补发丢失声明，并继续使用现有受保护签名密钥和目标盘确认。schemaVersion=2 历史 Bundle 若要获得远程作废能力，须先升级 Bundle、重算 anchor，再由授权签名器重新签发匹配新指纹的 UsbLicense 与登记声明。
- 修改 src/app/views/OfflineHome.tsx、src/services/managerApi.ts、src/shared/types.ts：状态显示和按钮门禁。
- 新增或调整对应 Rust 与前端测试。

VPS 服务：

- 新增独立目录 server/usb-license-revocation/，包括服务、数据库迁移、管理 CLI、部署文件和测试。
- VPS 部署与 Sub2API 分离，使用独立服务用户、数据库文件、日志和子域名。

以下文件不得在本功能中修改其既有语义：crates/codex-usb-license 的 UsbLicense 签名格式、用户数据清理策略、普通卸载的确认/选项流程、完整媒体清理的授权范围。

## 3. VPS 端设计

### 3.1 服务边界与默认部署形态

VPS 端提供专用许可证状态 API，不处理模型请求、图片/视频任务、Sub2API 账号或 Codex 上游转发。

推荐第一阶段部署：

- 单独服务目录：/opt/codex-usb-license-revocation
- 服务监听：127.0.0.1:8787，仅反向代理可访问
- 外部地址：`https://<revocation-subdomain>`（部署时替换为实际域名）
- 反向代理：使用 VPS 当前维护的 Nginx 或 Caddy；不为此功能另开公网管理端口
- 服务实现：Go 单二进制 + SQLite，SQLite WAL 模式；低流量单 VPS 足够，减少数据库和运行依赖
- 进程管理：systemd 专用服务用户；Restart=on-failure；只读代码目录；数据库和私钥目录单独授权
- 数据文件：/var/lib/codex-usb-license-revocation/state.db
- 独立服务端 Ed25519 状态签名密钥：/etc/codex-usb-license-revocation/status-signing-key；仅 root/服务用户可读

Go + SQLite 是本文规定的参考实现，不代表必须迁移 VPS 上已有服务。若实施时 VPS 现有标准明确要求 Docker 或其他反代，可替换部署包装，但不得改变 API 和签名协议。最终部署前将子域名替换为实际域名，并配置有效 HTTPS 证书。

### 3.2 公共 API

唯一公开业务接口：

    POST /v1/licenses/check

请求 Content-Type 为 application/json。客户端不发送完整 UsbLicense，因为其已签名 accountBinding 可能包含邮箱和 CCSwitch deviceApiKey。正式签名器在 USB 上额外生成 usb-license-revocation-registration.json。它是由许可证签发密钥签名的、与单个许可证绑定的最小声明，不包含 accountBinding。

新版本 USB 的 root manifest.json 与 usb-manifest.json 使用文档 schemaVersion=3；root manifest.json 必须包含 `revocationPolicy.registrationRequired=true`，并由现有 USB anchor/manifest 签名链保护。anchorSchema 仍为 3，两个 manifest 的 schemaVersion 必须一致。登记声明文件可变且不加入 SHA256SUMS，但“必须存在”的策略不可由普通文件删除或改写。经验证的历史 schemaVersion=2 manifest 按未登记旧版处理；schemaVersion=3 缺字段、类型错误或值不是 true 时不得默认放行。旧版 USB 不能仅补侧文件就升级为受远程作废管理，因为许可证签名绑定旧 anchor；必须升级 Bundle 并重新签发匹配新指纹的许可证。

登记声明文件为信封：

    {
      "payloadB64": "登记声明精确 UTF-8 字节的 Base64",
      "signatureB64": "许可证签发私钥对 payload 原始字节的 Ed25519 签名"
    }

payloadB64 解码后的 JSON 字段：

    {
      "schemaVersion": 1,
      "domain": "codex-usb-revocation-registration-v1",
      "product": "codex-usb-oneclick",
      "installationId": "现有 UsbLicense.installationId",
      "licenseKeyId": "现有 UsbLicense.keyId",
      "licenseSignatureSha256": "许可证 signature 解码后 64 字节的 SHA-256，小写十六进制",
      "fingerprintSha256": "当前 USB 指纹的规范摘要",
      "issuedAt": "现有 UsbLicense.issuedAt"
    }

登记声明由现有许可证签名器在确认目标 USB、验证 UsbLicense 签名后生成；旧 anchorSchema 许可证必须复用现有 core 的当前/历史载荷验签逻辑。声明的 payload 原始字节作为签名输入，服务端无需重新实现 Rust 的 UsbLicense JSON 序列化。客户端启动时同时验证 UsbLicense 和登记声明，并检查 product、installationId、licenseKeyId、licenseSignatureSha256 与本地许可证一致，再比较 USB 当前指纹摘要。

fingerprintSha256 的摘要输入固定为以下二进制串，避免 Rust/Go JSON 序列化差异：

    ASCII("codex-usb-fingerprint-v1") || 0x00
    || anchorSchema 的 4 字节无符号大端整数
    || volumeSerial 的 4 字节无符号大端整数
    || anchorSha256 解码后的 32 字节
    || bundleRelativePath UTF-8 字节长度的 4 字节无符号大端整数
    || bundleRelativePath 的 UTF-8 字节

构造前将 anchorSha256 转小写十六进制并解码；将 bundleRelativePath 的反斜线换成正斜线并按 ASCII 小写规范化；拒绝绝对路径、空组件、.、..、冒号和 NUL。摘要为以上串的 SHA-256 小写十六进制。许可证 signature 同样先做标准 Base64 解码，必须恰为 64 字节，再计算 licenseSignatureSha256。

每次 check 请求包含登记声明和此刻 USB 指纹：

    {
      "schemaVersion": 1,
      "requestId": "32 个小写十六进制字符，来自 16 个 CSPRNG 随机字节",
      "product": "codex-usb-oneclick",
      "installationId": "登记声明中的值",
      "licenseSignatureSha256": "登记声明中的值",
      "registration": {
        "payloadB64": "...",
        "signatureB64": "..."
      },
      "observedFingerprint": {
        "volumeSerial": 123,
        "anchorSha256": "64 位十六进制",
        "bundleRelativePath": "USB Bundle 相对路径",
        "anchorSchema": 3
      }
    }

VPS 验证登记声明签名、公钥 keyId、domain、product、字段完整性和已登记指纹摘要；并重新计算 observedFingerprint 摘要，确保与签名声明和数据库相同。每块 USB 第一次签名/登记时，管理员 CLI 导入的也是该登记声明，不是 UsbLicense 全文。服务只保存许可证签名摘要和指纹摘要，不接收或记录 accountBinding、deviceApiKey、loginEmail、邮箱、模型 Key 或用户项目。

返回 200 时，业务体为签名信封：

    {
      "payloadB64": "payload 精确 UTF-8 字节的 Base64",
      "signatureB64": "对 payload 原始字节执行 Ed25519 签名后的 Base64"
    }

Base64 解码得到的 payload JSON 示例：

    {
      "schemaVersion": 1,
      "product": "codex-usb-oneclick",
      "installationId": "匹配请求",
      "licenseSignatureSha256": "与请求登记声明一致",
      "requestId": "必须逐字匹配请求",
      "status": "active 或 revoked",
      "stateVersion": 4,
      "issuedAt": "RFC3339 UTC",
      "signingKeyId": "revocation-2026-01"
    }

客户端验证的是 payloadB64 解出的原始字节，随后才解析 JSON。这样不依赖不同语言对 JSON 字段顺序、空格或 Unicode 转义的序列化一致性。状态签名密钥与 USB 许可证签发密钥分离。

服务状态只有 active 与 revoked 两种可触发决策的值。数据库未登记、签名错误、身份不匹配等情况不得伪装成 revoked。服务端返回未签名错误响应或非 200 时，客户端归类为“服务不可用/状态不明”，不得清理。

健康检查接口：

    GET /healthz

只返回服务进程健康，不公开许可证查询、密钥、数据库内容或管理信息。

### 3.3 数据模型

SQLite 表 licenses：

| 字段 | 含义 |
|---|---|
| product | 固定产品标识 |
| installation_id | 许可证唯一安装 ID，主键组成之一 |
| key_id | 签发许可证使用的 keyId |
| license_signature_sha256 | 登记声明绑定的许可证 signature 摘要 |
| fingerprint_digest | 许可证绑定 USB 指纹摘要 |
| status | active 或 revoked |
| state_version | 每次状态迁移严格递增的整数 |
| created_at / updated_at | UTC 时间 |
| revoked_at | 可空；最近一次作废时间 |
| revoke_reason | 管理员填写的短原因；不写进客户端展示内容 |

SQLite 表 audit_events：

| 字段 | 含义 |
|---|---|
| event_id | UUID |
| installation_id | 目标许可证 |
| actor | 管理员身份/操作来源 |
| action | import、revoke、reactivate |
| old_status / new_status | 状态变化 |
| old_version / new_version | 版本变化 |
| reason | 操作原因 |
| occurred_at | UTC 时间 |

许可证键使用 product + installation_id 唯一约束。首次导入有效登记声明时，由管理员明确设为 active，stateVersion=1，并写入 import 审计事件；revoke 变为 version 2，reactivate 变为 version 3，后续每次状态迁移都严格递增。写状态与写审计事件必须在同一数据库事务中完成。

状态历史不能只依赖 SQLite 快照。每次 import/revoke/reactivate 都须先写入本机追加式审计日志，并同步到独立备份位置；数据库事务与审计事件成功后，API 才能签发新状态。启动时检测数据库损坏、备份恢复标记或审计日志缺口时，服务进入状态不明模式，不签发任何 active/revoked 响应，公共 API 返回 503。恢复时先从追加日志重放并核对最高 stateVersion；无法证明历史完整时必须人工修复状态，禁止用旧 active 快照恢复服务。

由于客户端规定 VPS 不可用时离线放行，服务处于 503 恢复模式仍可能允许没有本地作废回执的有效许可证离线启动；这不能被服务器误报成 active，也不能据此触发卸载。已有有效 revoked 回执的客户端继续封锁。服务端状态历史和客户端 fail-open 规则共同决定真实撤销可用性边界。

### 3.4 私有管理操作

首版不暴露公网管理网页。提供服务器本机命令：

    codex-license-admin import --registration-file <USB 登记声明 JSON> --initial-status active
    codex-license-admin revoke --installation-id <ID> --reason <简短原因>
    codex-license-admin reactivate --installation-id <ID> --reason <简短原因>
    codex-license-admin inspect --installation-id <ID>

约束：

- 管理 CLI 只在 VPS 本机执行，或通过 SSH 登录后执行。
- 公共反代不暴露管理 API。若未来必须提供远程管理接口，另设 VPN/仅回环绑定 + 强认证与审计，不能复用 USB 客户端凭据。
- import 仅接受通过许可证签发公钥验签的正式登记声明；同一个 installationId 重复导入但 licenseSignatureSha256 或 fingerprintSha256 冲突时拒绝覆盖。
- 新记录初始状态固定为 active、stateVersion=1；仅允许明确指定初始状态的 import 命令创建记录。
- revoke/reactivate 使用数据库事务、递增 stateVersion、写 audit_events；重复执行同一目标状态为幂等。
- CLI 输出不包含私钥内容或完整指纹。用户提供理由由管理员录入，避免写个人敏感信息。

签名器新增操作示意：

    usb-license-signer register --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "REGISTER <volumeSerial>-<anchorSha256前12位>"

新签名流程在 license 写入前已确定同一安装身份和指纹，license 与登记声明必须作为一个签名事务原子落盘并回读复验。register 操作只为已经采用 schemaVersion=3 必需登记策略、且现有 UsbLicense 指纹仍匹配的 Bundle 补发登记声明；必须重新观察目标 USB、验证现有签名/设备绑定、使用同一个 keyId 签名，不能改写 UsbLicense。登记声明通过受控方式交给 VPS 操作员导入；导入 active 且从 USB 实际查询通过后再交付。schemaVersion=2 旧 Bundle 继续使用现有本地/离线 gate，不具备远程作废能力；升级时必须重建新格式 Bundle、重算 anchor，并由授权签名器重新签发匹配新指纹的 UsbLicense 与登记声明。

### 3.5 VPS 密钥与运维

- 使用专用 Ed25519 状态签名密钥；不要复用许可证签名私钥。
- 配置单独的许可证登记声明公钥 keyring，按 licenseKeyId 映射到现有许可证签发公钥。公钥不是秘密；私钥仍只在现有签名器的受保护签名环境中使用。
- 私钥不得写进 Git、Docker 镜像、日志、API 响应或客户端配置。
- VPS 使用 root 管理的密钥文件或等价秘密管理；权限仅服务用户可读；定期离线备份。
- 状态验签公钥以 signingKeyId 区分。客户端可内置多个验签公钥以支持轮换；新私钥启用前先发布包含新公钥的客户端版本。许可证登记公钥 keyring 必须保留已签发许可证所用 keyId，以支持旧版许可证和 anchorSchema 兼容验证。
- TLS 负责链路保密和服务器域名认证；Ed25519 响应签名用于离线验证服务端状态。两者都要保留。
- 数据库定期备份；备份需验证可恢复。陈旧备份恢复时必须按追加式审计日志重放；无法证明状态历史完整时返回 503，绝不能对旧 active 记录签名。
- 日志只记录请求结果类别、安装 ID、stateVersion、耗时和 requestId；不记完整许可证、API key、私钥、完整指纹或可复用认证材料。
- 公共接口做请求体大小上限、速率限制、超时、SQLite busy timeout 和并发写入保护。

## 4. 本地端设计

### 4.1 启动时序

1. 定位当前 USB Bundle，确认可移动 NTFS 卷及固定 Bundle 根目录；执行最小只读校验，读取本地 UsbLicense、manifest 文档版本/锚定策略、可选的签名登记声明、物理指纹及本地状态记录。
2. 本地 UsbLicense 或设备指纹无效时，继续走现有本地拒绝/受限界面；不触发远程清理。
3. 在完整 Bundle 校验前验签已有的状态信封并绑定当前有效 UsbLicense 的 installationId、licenseSignatureSha256 和 USB 指纹，按最高 stateVersion 决定当前状态。同版本出现 active/revoked 冲突时按 revoked 处理并报告状态冲突。最高状态为 revoked 时立即关闭 Codex gate，进入作废恢复；若更高有效 active 覆盖历史 revoked，作废门禁已解除，但仍保留历史回执用于判断是否允许恢复被清理的目标文件。
4. 作废恢复完整性校验只允许因上次作废清理而缺失的精确 Codex 目标路径；它仍须校验 USB 身份、root manifest、usb-manifest、SHA256SUMS、所有非目标文件及所有仍存在的目标文件。恢复仍为 revoked 时不依赖登记声明文件，服务器签名回执本身足以证明作废；策略要求的登记声明缺失时仍维持禁止状态。若状态已由更高 active 覆盖历史 revoked，仅当有效历史 revoked 回执存在且缺失路径完全属于清理白名单时，才可用 recovery validator 进入受限重装状态。
5. 最高状态为 revoked 且受限完整性校验通过时，保持 revoked gate，再用有效登记声明向 VPS 请求最新状态。只有验签成功、身份/requestId 匹配且 stateVersion 高于本地 revoked 状态的 active 响应，才可原子保存新 active 状态并解除封锁；因运行载荷已删除，随后进入 reactivated_payload_missing，引导用户通过安装器重新下载/安装。历史 revoked 回执和清理历史继续保留，直到载荷恢复完整。没有有效登记声明时维持封锁；应由授权签名器 register 操作补发登记声明后再尝试恢复。
6. 对已有作废凭证，VPS 离线、响应错误、状态仍为 revoked、响应版本不高于本地 revoked 版本或任何验签/身份检查失败，均继续保持 revoked gate，并重试清理；不得离线解除作废。
7. 若最新有效状态为高于历史 revoked 的 active，且历史作废回执有效但受管 Codex 目标缺失，则使用 recovery validator 确认只有清理白名单目标缺失，进入 reactivated_payload_missing。Codex 不启动；仅开放重新下载/安装所需的 manager 操作。登记声明有效且网络可用时，开放重装前再查询 VPS：更高版本 revoked 立即重新封锁并清理；有效 active 保存为最新状态后允许重装；网络不可用或响应无效时，保留已保存的更高版本 active，只允许重装、不允许启动 Codex。若 Codex 目标均存在且普通完整 Bundle 校验通过，则按最高有效 active 状态继续启动流程。
8. 没有当前有效 revoked 状态时运行现有完整 Bundle 校验。若缺少/修改任何必需载荷或锚定 manifest 则按完整性错误受限，不把它当成远程作废。schemaVersion=3 必须有锚定策略 `revocationPolicy.registrationRequired=true`；登记声明缺失/无效即进入 registration_required_repair，禁止 Codex 启动、安装、更新和修复；仅由授权签名器 register 操作补发并回读验证后继续。经验证的 schemaVersion=2 历史 manifest 没有远程作废能力，进入 remote_unenrolled 并按历史离线策略运行。schemaVersion=2 的已签名 U 盘不能仅补一个声明就获得可靠远程作废：升级必须按新格式重建 Bundle、重新计算 anchor，并由授权签名器对新指纹重新签发 UsbLicense/登记声明。
9. 仅对 schemaVersion=3、registrationRequired=true 且登记声明有效的 Bundle，向 VPS 发一次状态请求；请求完成前 Codex 操作保持禁用。收到有效 active 后放行；收到有效 revoked 后先原子保存凭证，再转入禁止状态并启动清理 helper。网络不可用、超时或响应不可验证时不创建作废记录、不删文件；本地许可证、完整 Bundle 和必需登记声明均有效时按 C3 离线放行。schemaVersion=2 历史 Bundle 不查询 VPS，按 remote_unenrolled 离线运行。

管理器启动时以及管理器实际启动 Codex 的每次操作前执行新鲜远程查询，不做后台持续轮询。服务器作废对已经运行且未发起新查询的进程不会即时生效。U 盘内 MSIX manifest 声明入口由 direct-entry shim 接管，普通双击通过管理器重新执行远程查询和本地门禁。

现有完整 Bundle 校验会要求 MSIX payload 和 apps/Codex 存在。清理后下次启动不得先调用该严格校验并退出；必须在最小身份校验后先验证服务器签名的 revoked 回执，再调用仅允许受管 Codex 目标缺失的 recovery validator。除此以外的 Bundle 缺失、哈希变化或路径异常仍然阻止所有 Codex 操作。

recovery validator 的缺失例外必须是由已校验 root manifest/usb-manifest 解出的固定目标：manifest 声明的唯一 Codex MSIX payload、apps/Codex、以及 Bundle 明确登记的 Codex updater staging/backup。它不得允许任意 payload、manifest、签名、MCP、CCSwitch 或能力包缺失。`revocationPolicy.registrationRequired` 必须由现有签名 anchor 绑定在不可变 root manifest 中；新版本必需为 true。usb-license-revocation-registration.json 与 revocation receipt 是独立签名的可变状态文件，不加入 SHA256SUMS；登记声明缺失是否阻止启动由锚定策略决定。网络查询时登记声明必须验签，恢复已确认 revoked 时由状态回执签名和当前 UsbLicense 绑定校验授权恢复路径。

### 4.2 状态机

状态建议使用明确枚举，不复用现有 signatureState 的 unsigned/invalid 字符串：

| 状态 | 含义 | Codex 操作 |
|---|---|---|
| local_invalid | 本地签名/指纹校验失败 | 禁止，沿用现有错误恢复路径；不自动清理 |
| remote_checking | 本地有效，远程状态请求中 | 暂不开放 Codex 操作 |
| active_online | 收到有效 active 签名响应 | 允许 |
| active_offline | VPS 不可用或无可信状态，且本地许可证、Bundle 和策略要求的登记声明均有效 | 允许离线启动 |
| remote_unenrolled | 经验证的历史 schemaVersion=2 manifest | 按历史策略允许本地/离线启动；不具备远程作废能力 |
| registration_required_repair | 锚定策略要求登记，但声明缺失/无效 | 禁止 Codex 操作；提示用授权签名器补发声明 |
| revoked_cleanup_pending | 已有有效 revoked 凭证，等待/正在清理 | 禁止 |
| revoked_cleanup_partial | 已作废，部分文件受占用或无法删除 | 禁止；保留 manager 并允许重试 |
| revoked_cleaned | 已作废且受管 Codex 文件清理完成 | 禁止；保留许可证和作废凭证 |
| reactivated_payload_missing | 本地最高有效签名状态为高版本 active，且有有效 revoked 清理历史、Codex 载荷已清除 | 禁止启动；只允许通过安装器重新下载/安装/修复 |

服务响应验签失败、身份不匹配、HTTP 错误或解析失败归入远程不可用，不设置 revoked；在本地许可证和 Bundle 正常时按 C3 离线放行。锚定策略要求登记声明而声明缺失/无效是本地完整性/登记修复状态，不得归入远程不可用或离线放行。只有 valid signed revoked 可以进入 revoked_cleanup_pending。

### 4.3 请求超时与错误分类

- 连接超时建议 2 秒，总请求超时建议不超过 5 秒；实际值由实测调整。
- 全部网络请求使用 HTTPS；不允许静默降级到 HTTP。
- DNS 失败、连接失败、TLS 证书错误、超时、HTTP 非 200、响应过大、JSON 解析失败、签名无效、nonce 不匹配、installationId 不匹配、licenseSignatureSha256 不匹配均不得触发删除。
- 日志记录错误类别和 requestId；不把网络堆栈或内部错误原文直接展示给小白用户。
- 离线策略是 fail-open：攻击者若主动阻断网络，客户端也会认为 VPS 不可用并允许有效本地许可证离线启动。这是离线可用性与即时远程撤销之间的固有取舍，不能宣称可以同时消除。

### 4.4 作废凭证与本地持久状态

收到 revoked 后保存原始签名信封及本地事务状态。建议位置：

- U 盘 Bundle 可变状态：CodexUsbState/revocation.json
- 当前 Windows 用户备份状态：%LOCALAPPDATA%\\CodexUsbGuard\\revocation\\<installationId>.json

U 盘端状态在同一个 USB 上跨电脑保留；本机副本可应对 U 盘状态目录异常。每份均保存可独立验签的服务端状态信封历史和本地清理进度，不保存管理员密钥。状态文件不得只保存派生的 `status` 字符串。

写入规则：

- 使用现有 atomic_file 原子替换写入。
- 写入前校验父目录在已验证 Bundle 内、没有 junction/symlink/reparse point。
- 状态文件必须加入 Bundle 的可变文件白名单，不加入 SHA256SUMS 的不可变内容摘要。
- 读取时重新验服务端 Ed25519 签名、产品、installationId、licenseSignatureSha256 和状态版本；不得相信本地 JSON 的 status 字符串。
- 分别验签两份状态中的服务器信封，按 stateVersion 取最高有效状态；同版本状态冲突时失败关闭。最高状态为 revoked 时封锁；只有更高 stateVersion=active 才解除封锁。收到更高 active 后，保留此前有效 revoked 回执和清理历史，直到 Bundle 的 Codex 载荷重新完整验证；这使后续离线启动可进入受限重装流程，同时不会把旧 revoked 误当成当前作废。载荷完整恢复后可清除历史清理事务，但保留最高版本状态信封。
- 如果首次收到作废后两处都不能持久化，当前会话必须立即阻止 Codex，保持受限页面并重试保存/清理；不得显示授权恢复。日志提示状态盘不可写。
- 签名只能证明回执由服务端发出，不能阻止用户删除/回滚两个本地状态副本。若两份回执均丢失且网络不可用，fail-open 策略会允许本地签名有效的 Bundle 离线启动；本产品不提供反回滚保证。

### 4.5 自动清理事务

清理在 revoked 凭证落盘并且 Codex gate 已关闭后开始，采用可重入步骤：

1. 验证作废凭证再次通过签名及身份验证。
2. 使用当前授权 Bundle 清单解析受管目标路径；所有规范化路径必须落在当前 U 盘 Bundle 内。
3. 拒绝目标目录、祖先目录或文件是 reparse point、symlink、junction、硬链接绕行或路径替换的情形。
4. 只查找本 USB Bundle 启动的 Codex 进程，使用进程完整映像路径与该 Bundle 的 apps/Codex 子树比对；不能按 ChatGPT.exe/Codex.exe 名称全局 taskkill。
5. 先请求受管进程正常退出；等待有限时间后，仅对仍运行且映像路径复核匹配的进程终止。
6. 通过清理清单移除 manifest 唯一声明的 Codex payload（当前为 payload/OpenAI.Codex_x64.msix）、apps/Codex 运行树及该运行树由产品创建的 updater staging/backup。
7. 每个路径单独写清理结果；完整复核不存在可由安装器直接恢复的 Codex payload。
8. 写入 revoked_cleaned 或 revoked_cleanup_partial 结果。清理不删除 manager、launcher、guard helper、usb-license.json、usb-license-revocation-registration.json、revocation receipt、能力包、CCSwitch、用户数据或 U 盘根目录。

任何目标路径缺少归属证明、清单不匹配、安装事务未结束、权限不足、文件占用或卷断开时，停止相应删除步骤并报告部分失败。不得因此清除作废标记或恢复 Codex。清理完成后重新打开管理器必须进入 revoked recovery 状态；不因 manifest 预期目标已缺失而启动普通安装或退出受限界面。

对历史版本留下的 Windows AppX 注册/机器级 package、或 provenance 不明的安装，不由自动作废流程处理。原因是现有完整 USB 卸载会同时处理系统注册、CCSwitch 和其他恢复状态，且允许一个用户确认过的旧版兼容路径删除外部 OpenAI.Codex 注册；静默执行可能误删其他 Codex。第一阶段只清理当前产品 manifest 能明确归属的便携运行树和安装载荷。

### 4.6 管理器启动与直接运行边界

管理器是受远程状态检查保护的 Codex 启动路径。每次管理器准备启动 Codex 时，都必须在实际启动点重新完成本地许可/指纹/Bundle 校验和新鲜 VPS 状态查询；有效 active 才正常启动。网络不可用或服务异常时，只有本地许可、Bundle 和策略要求的登记声明均有效且没有更高有效 revoked 状态，才按离线 fail-open 规则启动。已保存的有效 revoked 状态仍保持封锁并重试窄范围清理。

schema 3 签名 Bundle 的 root manifest 必须锚定 `directEntryPolicy.required=true`、`shim=CodexEntryShim.exe` 和 `failure=closed`；管理器只有在许可证锚点、Bundle 完整性及现场指纹全部核验后才读取此策略。schema 2 旧 Bundle 不强制安装 shim，保持原来的入口兼容。新 schema 3 安装会把 MSIX 声明入口替换为 shim，并将官方入口保存在受管目录的非 `.exe` 文件 `CodexCore.dat`。如果包内还包含另一个 `Codex.exe` 启动别名，也会保存在 `CodexAliasCore.dat` 并由 shim 接管。运行时还要确认两个入口都与 Bundle 根目录中受签名锚点保护的 shim 文件一致；缺少别名记录或哈希不匹配时拒绝启动。

普通双击会先连接 USB 管理器，由管理器重新执行签名、远程状态和作废清理检查，再启动官方程序。即使管理器因维护使用提权令牌，Codex 仍必须通过已验证的当前桌面 Explorer 用户令牌启动；直接入口管道只授权给该桌面用户 SID，并再次核验会话和客户端 SID。无法连接管理器、UAC 被取消、参数格式不支持或授权未就绪时，shim 必须拒绝启动 Codex。官方原始 MSIX 载荷不被改写，shim 只作为安装事务中的运行时 overlay。

该机制阻止正常使用路径直接双击绕过，不构成对可读取 U 盘的不可绕过 DRM。具有文件修改能力的用户仍可能自行恢复或提取原始 MSIX；产品文案不得承诺防止技术性复制或篡改。离线时仍按本地签名和已持久化状态允许启动；只有成功验签的 revoked 回执触发锁定和清理。

### 4.7 恢复/重新激活

- revoked 用户在清理后不能离线恢复。需有成功验签的更高 stateVersion active 响应。
- 服务端恢复 active 后，客户端持久保存更高 active 信封并解除当前封锁；历史 revoked 回执仅作为清理证据保留。因为运行文件和 MSIX payload 已删除，不自动恢复程序文件。
- 用户通过安装器使用正常安装/更新流程重新获得经 Authenticode、SHA256 和 USB Bundle 清单验证的 Codex 载荷。
- 重装期间仍只开放重装/修复，Codex 不启动。安装完成且完整 Bundle 校验通过后，若 VPS 暂时不可访问，按本地有效许可证及已持久化的更高 active 状态离线启动，符合 fail-open 策略。
- 如果服务器在之后再次作废但设备持续离线，设备无法即时获知；下次成功联网检查收到更高 revoked 后才封锁并清理。若首次收到更高 active 响应却未能持久化，仍按之前的 revoked 状态封锁。
- 如果产品决定许可证作废不可逆，可禁用 reactivate 命令；但这属于 VPS 管理策略选择，不改变签名协议。

## 5. 本地与 VPS 的协议和密钥协作

| 项目 | VPS | 本地 |
|---|---|---|
| USB 许可证签发公钥 | 用于验证签名器生成的登记声明；VPS 不接收完整 UsbLicense | 已由当前构建策略持有 |
| USB 许可证签发私钥 | 不新增；仍由现有签名器流程保管 | 绝不进入客户端 |
| 远程状态签名私钥 | 独立保存于 VPS | 不可访问 |
| 远程状态验签公钥 | 对应私钥的公钥 | 编译进正式 manager 和 launcher |
| API 域名 | 部署后绑定子域名与 HTTPS 证书 | 正式构建时写入固定地址 |
| 客户端 API Key | 不需要 | 不存在 |
| 作废数据 | SQLite 状态和审计事件 | 保存签名收据及清理进度 |

公网 API 不使用客户端共享 bearer secret。客户端共享密钥一旦放入 U 盘即可被复制；服务端响应验签才是客户端唯一允许触发删除的可信证据。

## 6. 失败、边界与用户体验

| 场景 | 应有处理 |
|---|---|
| 服务正常、许可证 active | 正常启动，无多余弹窗 |
| 服务正常、明确 revoked | 显示易懂的作废说明，Codex 按钮禁用，后台清理 |
| 已有有效作废凭证、VPS 离线或状态未能更高版本解封 | 始终保持禁止；继续重试清理，不按普通离线许可证启动 |
| 已有有效作废凭证、收到更高版本签名 active | 保存新状态并解除作废门禁；因 Codex 文件已清理，进入重新下载/安装引导 |
| VPS 关机/域名无法解析 | 本地许可证、完整 Bundle 和必需登记声明均有效时正常离线启动，不删除 |
| VPS TLS 证书错误 | 不信任状态，不删除；本地许可证、完整 Bundle 和必需登记声明均有效时按离线规则 |
| 新版必需登记声明缺失/无效 | 进入登记修复受限页；禁止 Codex 操作，不创建作废标记、不删除文件 |
| 历史 manifest 未启用登记要求 | 按 remote_unenrolled 运行，不具备远程作废能力 |
| 返回 revoked 但签名无效 | 当作服务异常，不删除 |
| 签名正确但 requestId/installationId/licenseSignatureSha256 不匹配 | 当作服务异常，不删除 |
| revoked 后拔掉 U 盘 | 保留状态；下次插入、启动 manager 后重试 |
| Codex 文件被占用 | 不启动新进程；报告“程序关闭后会继续清理”，下次重试 |
| 载荷清理完成但用户数据存在 | 保留数据；安装器显示 Codex 程序已移除 |
| 数据库没有该许可证记录 | 不能返回 revoked；记录服务端错误，客户端走离线策略 |
| stateVersion 倒退或重复冲突 | 不接受旧 active；保留已知 revoked 状态 |
| USB 文件被复制/改写 | 现有 bundle 指纹和 integrity gate 继续处理；不得把复制文件误认为服务器状态 |
| 用户已从 U 盘复制运行文件到别处 | 本产品不进行全盘查杀；不属于安装清理范围 |

前端采用简短文案：

- 检查中：“正在检查 U 盘授权…”
- 服务不可用：“暂时无法连接授权服务，已按离线方式启动。”
- 已作废：“此 U 盘授权已停用，Codex 程序正在移除。”
- 部分清理：“授权已停用，部分程序文件暂时无法删除。关闭 Codex 后重新打开安装器即可继续。”
- 已完成：“此 U 盘的 Codex 程序已移除。个人文件和配置仍保留。”

不要展示 revoked reason、stateVersion、nonce、签名、公钥 ID、HTTP 状态或 Rust 错误堆栈。

## 7. 安全与隐私约束

- 客户端只上传执行授权检查所需的许可证签名字段和指纹；服务端只存指纹摘要，不上传/记录模型 Key、Codex 用户 token、邮箱、项目内容或配置文件。
- 请求全程 HTTPS；响应另有 Ed25519 验签。
- 请求随机数至少 128 位不可预测；禁止接受旧 requestId 的缓存响应。
- API 请求体大小建议限制为 64 KiB；连接和整体请求有超时。
- 清理 helper 只接收内部生成的清理 Job ID，不接受前端提供的任意路径。
- 清理之前再检查 licenseSignatureSha256、installationId、USB 指纹、路径边界和清单摘要。
- 任何用户数据路径、环境变量路径或任意文件名都不能成为清理目标。
- 管理动作有审计记录；管理私钥和客户端状态签名私钥不得输出日志。
- 服务端数据库备份、签名密钥备份分开；丢失状态签名私钥时采用新 keyId + 客户端升级轮换，不在旧公钥下伪造响应。

## 8. 开发阶段与依赖顺序

### 阶段 0：冻结接口与管理器门禁

- 用当前正式 Bundle 识别 manager、Codex manifest 入口、resources、MSIX payload 和运行时更新目录。
- 验证 direct-entry shim、管理器 IPC、MSIX 解包 overlay、官方程序启动 override、进程追踪、更新事务和作废清理之间的端到端关系。
- 固化状态签名 envelope 测试向量、服务 URL 配置位置和本地清理清单。

### 阶段 1：VPS 服务

- 实现 SQLite schema、许可证导入验证、check API、签名响应、管理 CLI 和审计事件。
- 为 active、revoked、unknown、错误响应建立固定测试向量。
- 建立 systemd/反向代理部署配置、HTTPS、服务健康检查、备份与恢复说明。
- 先在 staging 子域名验证；尚无正式域名时不得编译正式 USB 包。

### 阶段 2：本地协议客户端

- 实现 envelope 验签、nonce/身份/digest 检查、超时分类和离线 fallback。
- 实现 USB/Windows profile 双位置原子状态文件及版本比较。
- 把 remote_checking/revoked 状态接入现有 Codex gate，不开放先启动后检查路径。

### 阶段 3：管理器启动门禁和窄范围清理

- 在 manager 启动 Codex 的实际调用点执行新鲜授权检查；将 shim 作为 USB runtime overlay 安装到 manifest 声明入口，并保留官方可执行文件供受控启动。
- manager 完成 USB 授权检查和启动事务恢复后，开启仅限当前用户/会话的 named-pipe IPC；shim 请求必须复用与首页启动按钮相同的 `win_launch_codex` 授权和启动路径。
- 正式 bundle、签名、清单和验收必须包含 direct-entry shim；缺失、哈希不符、原始入口丢失、shim/IPC 不可用时均不得启动 Codex。
- 实现作废凭证落盘、停止该 Bundle 管理的 Codex 进程、删除 Codex runtime 与 MSIX payload。
- 复用安全路径验证和 detached finalizer 思路；不得接入 CCSwitch、profile purge 或整盘清理。
- 增加部分失败记录、重试、用户提示和恢复 active 后的重装引导。

### 阶段 4：打包、真实环境验收与发布

- 设置真实子域名、有效 TLS 证书、正式验签公钥和正式构建 URL。
- 使用正式签名策略生成 launcher、manager、helper 与 Bundle。
- 在隔离测试 USB 上逐项执行测试矩阵；不得用 mock 结果替代正式签名包真实启动测试。
- 通过独立安全审计与功能测试后再生成发布包。

## 9. 测试与验收清单

### 9.1 VPS 测试

- 正确登记声明导入；无效签名、错误 licenseKeyId、重复但摘要不同的导入被拒绝；新记录固定 active/version=1。
- active 查询返回可验证的签名信封，字段、installationId、licenseSignatureSha256、requestId 一致。
- revoke 后 stateVersion 递增且查询返回 revoked；重复 revoke 幂等。
- reactivate 后 stateVersion 再递增；旧 revoked 响应不能覆盖新 active。
- 数据库事务失败时不出现“状态已变、审计记录未写”的半完成状态。
- API 不泄露私钥、完整指纹或数据库内容；公网不能访问管理 CLI/管理端点。
- 服务停止、数据库锁定、数据库丢失和反代 5xx 均不能生成 revoked 响应。
- 恢复 revoked 前的旧数据库快照后，不能签发旧 active；必须重放追加审计记录，历史无法证明时保持 503。
- 备份可恢复，并保留 revoked 状态与审计记录。

### 9.2 本地协议测试

- active 正常放行；revoked 只有完整验签后阻止并清理。
- 修改 payload 一个字节、错误签名、公钥 ID 未知、nonce 重放、安装身份不匹配、licenseSignatureSha256 不匹配均不清理。
- DNS、连接、TLS、超时、HTTP 4xx/5xx、畸形 JSON、超限响应均在本地许可证有效时离线放行且不写 revoked。
- 本地许可证失效仍按当前 gate 拒绝；不误触发自动卸载。
- 新版签名 manifest 的 registrationRequired=true 时，删除或损坏登记声明即使 VPS 在线且该许可证已 revoked，也必须保持 Codex 禁止；签名器补发有效声明后重新查询并处理 revoked。
- 修改或删除锚定 manifest 的 registrationRequired 策略导致现有 anchor/manifest 验证失败；新 manifest 缺字段不能按旧版 remote_unenrolled 放行。
- 只有通过签名验证的历史 manifest 缺少登记要求时才进入 remote_unenrolled；新版策略要求登记但 VPS 离线仍须提示修复登记，不得把本地登记缺失伪装成网络故障。
- 有效 revoked marker 在网络离线时仍拒绝；损坏/伪造 marker 不能充当服务器作废证明。
- 两份 marker 被人为删除且 VPS 离线时，按既定 fail-open 规则离线放行；产品不声称防止这种回滚。
- 带 accountBinding 的 USB License 完整 JSON 不出现在网络请求或服务日志中；只上传登记声明和不含账号字段的当前指纹。
- 旧 anchorSchema License 可由现有 signer core 校验并签发登记声明；Go 服务验证登记声明的原始字节签名，不重建 Rust License payload。
- 较低 stateVersion active 不解除较高版本 revoked；更高签名 active 才清除 tombstone。
- 同 stateVersion 的 active/revoked 冲突失败关闭；跨 U 盘/本机两份状态副本选择最高有效签名版本。
- revoked 清理后收到更高 active，保存新 active 和历史 revoked 清理证据；目标载荷仍缺失时，下次启动即使 VPS 离线，也只进入受限重装界面，不能启动缺失或残留的 Codex。
- 已缓存更高 active 后，若 VPS 在线返回再高版本 revoked，必须重新封锁并清理；网络离线时，目标载荷未恢复前只允许重装。重装完成且完整 Bundle 验证通过后，VPS 离线按 fail-open 规则允许启动；若服务器后来再次作废，要等下一次成功联网检查才封锁。
- manager 完成远程检查前不能启动、安装、更新或修复 Codex。

### 9.3 清理安全测试

- 仅删除 manifest 指向的 apps/Codex、Codex MSIX payload、产品升级 staging/backup。
- 同目录的用户文件、能力包、CCSwitch、usb-license.json、usb-license-revocation-registration.json、revocation.json、launcher/manager/helper 都保留。
- 保留 %USERPROFILE%\\.codex、Roaming/Local Codex 数据、项目及其他安装。
- 使用 junction、symlink、reparse point、路径大小写变体、盘符替换、路径穿越、清单改写等样例，确认不越界删除。
- Codex 正在运行时只终止映像路径落在当前 Bundle 的受管进程；另一目录中同名进程不受影响。
- 占用、断盘、只读卷、断电/进程中断后保持 revoked 并幂等重试。
- Codex payload 被移除后，安装器离线修复不能从已保留副本重新部署。
- 清理报告只有在逐项目标复核完成后才显示完成。
- 清理完成后重新打开管理器能验证 revoked receipt、通过只容许预期目标缺失的 recovery validator，并保持受限页可重试。
- 目标 MSIX 或 apps/Codex 缺失但没有有效 revoked receipt 时，普通完整性校验拒绝 Codex 操作，不进入自动清理。

### 9.4 用户路径验收

管理器启动路径必须覆盖：

1. 首次有效签名、VPS active，管理器启动成功。
2. 有效签名、VPS DNS/网络断开或 TLS 错误，符合离线 fail-open 且不清理。
3. 服务端真实 revoke 后，通过管理器再次启动；只有签名、nonce/requestId、installationId、许可证摘要和 revoked 状态都匹配时才阻止并清理。
4. 本地签名/指纹异常、服务端响应无效、身份不匹配或其他错误，不触发作废清理。
5. 每次管理器启动 Codex（包括主题切换后重启）都刷新远程授权状态。
6. 作废后立即拔盘，再插入重试；进程占用导致部分清理后，关闭进程并重试。
7. revoke 后服务端 reactivation，按受限路径重新下载/修复程序文件。
8. 启动另一个非本 U 盘目录的 Codex，确认未被停止或删除。

直接双击 U 盘 `apps/Codex` 的 manifest 声明入口时，必须经过 shim 和管理器授权链。验收覆盖：管理器已运行、管理器冷启动/UAC、在线 active、服务器离线且本地授权有效、签名 revoked、UAC 取消、管道启动失败、非法请求和载荷损坏；任何拒绝场景都不能启动官方 Codex 进程。还需确认管理器启动按钮仍使用同一官方程序，GPU 首试/兼容回退、`CODEX_HOME`、关闭自更新、进程登记/关闭和清理行为不退化。

## 10. 发布阻断条件

以下任一项未完成，都不得宣称“管理器检测到有效作废后会自动清理受管 Codex 内容”：

- 正式域名及 HTTPS 证书未就绪。
- 客户端没有内置正确 revocation 验签公钥，或正式包仍使用占位域名/测试密钥。
- VPS 签名响应没有覆盖 requestId、installationId、licenseSignatureSha256、status 和 stateVersion。
- “服务离线绝不卸载”负向测试未通过。
- 清理器不能证明所有删除路径归属当前 Bundle。
- 管理器启动路径未在实际 Codex 启动点重新执行状态检查。
- 直接入口 shim 或 manager IPC 未进入构建包，或其授权请求未复用管理器的实际 Codex 启动门禁。
- 清理失败会释放 Codex gate，或会误删用户数据/其他 Codex。
- 作废记录不能跨进程重启保存，或 offline 状态下已确认 revoked 仍能启动。
- VPS 数据备份与状态签名密钥备份/轮换流程未验证。

## 11. 重要技术边界

本设计能实现“管理器连上 VPS 并收到与当前 nonce、installationId 和许可证摘要匹配的可信 revoked 响应后，阻止管理器启动 Codex，并自动删除本产品明确管理的运行文件和安装载荷”。服务器无法联系离线 U 盘；根据既定规则，服务器不可访问时，本地状态有效且没有已知 revoked 状态的管理器允许离线启动。

U 盘文件可被读取和复制，因此本设计不是防止技术用户复制程序的 DRM。正常双击 USB Codex 入口必须进入 shim 和管理器授权链；必须如实说明技术用户可修改可读文件或自行提取 MSIX，产品不承诺阻止这种主动绕过。

## 12. 实施交付物

最终实现至少交付：

1. VPS 服务源代码、SQLite 迁移、管理 CLI、systemd/反代部署文件、备份恢复和密钥轮换说明。
2. OpenAPI/协议说明和固定签名测试向量。
3. 本地远程状态客户端、状态存储、管理器启动门禁、清理 helper 及前端状态。
4. 本文验收矩阵对应的 VPS/本地自动测试和真实隔离 U 盘验收记录。
5. 独立审计结论、最终发行构建信息、域名/API keyId/公钥指纹及二进制哈希。

## 13. VPS 管理页增补设计（2026-10-10）

本节按用户追加需求补充原设计中的 VPS 管理能力。它取代 §1.3 中“第一阶段仅使用管理 CLI”的范围限制，但不改变 U 盘客户端验签、离线 fail-open、已撤销状态和清理边界。

### 13.1 目标和注册语义

- 在同一个 Go 服务中提供 `/admin/` 管理页；正式反代域名为 `https://license.404token.xyz`，服务仍只监听 loopback。
- 管理员通过环境配置的用户名和密码登录。未配置完整凭据时，管理端点不可用；不得设置默认账号或默认密码。
- 签名器在许可证和登记声明写入、回读验签成功后，自动向 `POST /v1/licenses/register` 上传最小化的 Ed25519 登记凭证。服务端验证签名及 key ID，成功后建立 `active / version 1` 记录；同一凭证的重试幂等，内容冲突的 installationId 必须拒绝。
- 这里的“注册产生 key”指在 VPS 数据库创建授权记录并保留许可证 keyId，不代表签发可用于 API 调用的 bearer/API key。系统不生成或展示此类密钥。

### 13.2 页面与操作

- 登录页提供简单的账号/密码表单。
- 管理列表显示产品、安装 ID、许可证签名 keyId、状态、状态版本、登记/更新时间、撤销时间，以及掩码后的许可证摘要和 USB 指纹摘要。不显示完整指纹、原始登记包、签名私钥或 API 密钥。
- “登记新 U 盘”按钮上传签名器生成的登记 JSON；成功后列表出现新记录，默认 active/version 1。
- active 记录提供“作废”按钮。点击后打开二次确认对话框，展示目标安装 ID、要求填写审计原因，并要求再次确认后才调用作废接口。服务端按登录用户名记录 actor；操作写入现有双审计日志和 SQLite 事务。
- 页面说明：作废状态需要 U 盘下次联网并经管理器或 U 盘 Codex 入口启动后才会同步；离线仍遵循既有 fail-open 规则。正常双击 U 盘 Codex 入口会先由 shim 交给管理器检查。

### 13.3 认证、会话和 HTTP 边界

- 使用 `USB_REVOCATION_ADMIN_USERNAME`、`USB_REVOCATION_ADMIN_PASSWORD` 和 `USB_REVOCATION_ADMIN_ORIGIN` 配置；生产 Origin 固定为 `https://license.404token.xyz`。两项凭据缺一时服务启动失败，二者都未配置时只开放授权状态 API，管理页返回 404。
- HTTPS Origin 必须启用 Secure Cookie；只有 HTTP loopback 测试地址允许关闭 Secure Cookie。
- 密码至少 12 个字符；进程启动时以随机 salt 和 PBKDF2-HMAC-SHA256 600,000 次派生内存 verifier。不得将密码写入命令行、日志或响应。
- 首次成功登录必须先改掉环境文件提供的 bootstrap 密码并绑定 Google Authenticator，完成前不能查看、导入或作废授权。首登状态独立持久化；即使预置了 TOTP seed，也不能跳过改密。
- 首登未配置 TOTP seed 时，服务端在登录后为该会话生成 160-bit 随机 Base32 seed，仅向该待初始化会话返回；用户在 Google Authenticator 添加后提交当前验证码，成功时同时保存新密码 verifier、seed 和已使用 TOTP step。中断或重启后必须重新登录并完成设置。
- 管理凭据状态写入 owner-only auth-state 文件（0600），schema 版本升级失败时拒绝启动；数据库配置只接受普通文件路径，拒绝 SQLite URI 和内存 DSN；auth-state 路径必须与 SQLite 数据库及其 `-wal`、`-shm`、`-journal` sidecar、审计日志及其 `.lock`、镜像审计日志和 restore marker 不同。改密/首设提交期间原子清理会话；认证状态写盘不确定时当前进程 fail-closed。
- 登录成功后创建进程内随机会话，Cookie 使用 HttpOnly、Secure、SameSite=Strict、Path=/admin/，最长 8 小时；服务重启使所有会话失效。
- 登录失败限速；所有有 JSON 请求体的状态修改接口要求有效会话、与配置完全相同的 Origin、会话 CSRF token 和 JSON Content-Type；退出登录要求有效会话、Origin 和 CSRF token。失败时不透露账号是否存在或数据库内部错误。
- `/v1/licenses/check` 和 `/healthz` 的既有请求/响应不变。管理员接口不接受 U 盘端调用；公网 TLS 由 Caddy 终止，反代仍只放行明确列出的路径。
- Caddy 只代理 `/admin`、`/admin/*`、`POST /v1/licenses/check`、`POST /v1/licenses/register` 和 `GET /healthz`，其他路径 404；管理页不提供账户创建、密码找回或匿名注册入口。
- 首登 TOTP seed 可通过 `USB_REVOCATION_ADMIN_TOTP_SECRET` 预置，也可在首登时安全生成；每个时间步只接受一次。完成首登后，修改管理密码要求当前密码和 Google Authenticator 兼容的 6 位 TOTP 动态码；登录仍使用账号和密码。

### 13.4 管理 API

| 方法和路径 | 认证 | 作用 |
|---|---|---|
| `GET /admin/`、`GET /admin/static/*` | 页面静态资源 | 登录页与管理界面 |
| `POST /admin/api/login` | Origin 校验 | 账号/密码登录并创建会话 |
| `GET /admin/api/initial-setup` | 首登待初始化会话 | 获取该会话的 TOTP 绑定密钥；响应 no-store |
| `POST /admin/api/initial-setup` | 首登会话、Origin、CSRF、TOTP | 设置新密码、绑定 TOTP 并注销全部会话 |
| `POST /admin/api/change-password` | 会话、Origin、CSRF、当前密码、TOTP | 更新密码哈希并注销所有活动会话 |
| `GET /admin/api/session` | 会话 Cookie | 返回管理员、CSRF token 和是否需完成首登设置 |
| `POST /admin/api/logout` | 会话、Origin、CSRF | 注销当前会话 |
| `GET /admin/api/licenses?offset=&limit=` | 会话 Cookie | 分页读取安全展示字段 |
| `POST /admin/api/licenses` | 会话、Origin、CSRF | 导入签名登记 JSON，创建 active/version 1 记录 |
| `POST /admin/api/licenses/{installationId}/revoke` | 会话、Origin、CSRF | 二次确认后作废并追加审计事件 |

### 13.5 新增本地验收

1. 用临时 Ed25519 测试密钥生成一份新的登记声明，通过管理页登录和导入；确认列表出现 installationId、keyId、active/version 1，重复导入被拒绝。
2. 对测试记录执行二次确认作废；确认页面显示 revoked/version 2，审计事件 actor 为登录账号。
3. 通过公共状态接口验证服务返回匹配新 requestId 的签名 revoked 状态；使用本机临时 Bundle 调用 USB 客户端 startup check，确认只在验签成功后返回 revoked。
4. 在隔离临时目录执行实际 cleanup helper，确认只删除测试 Codex 目录/MSIX 载荷，保留管理器、用户 sentinel、登记文件和撤销回执。
5. 覆盖匿名访问、错误密码、缺 Origin/CSRF、错误 Origin、重复导入、active/revoked/离线状态、服务端错误；全程使用 loopback、临时 DB 和测试密钥，不触碰实体 U 盘或真实 VPS。

生产部署前仍需将状态公钥和正式 HTTPS 域名编入签名客户端、按本节配置管理员凭据与反代，然后在独立测试 U 盘上验收。不得把 VPS 私钥、管理员密码或测试密钥放入 USB 包。

### 13.6 本机验收记录（2026-10-10）

- `go test ./...`：通过，包含管理台登录、权限/Origin/CSRF 检查、登记导入、列表字段脱敏、重复导入拒绝、二次确认作废、审计 actor 和状态签名检查。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::usb_revocation::tests -j 1`：23 项通过；新增的本地 Go 服务闭环测试为显式 ignored 项，随后单独执行并通过。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib loopback_admin_revoke_syncs_into_usb_startup_and_cleanup -j 1 -- --ignored --nocapture`：通过。测试用 Rust 许可证库生成新签名登记，在本机临时 Go 服务的管理 API 导入，验证 active/v1，再从网页 API 流程作废并验证 revoked/v2；Rust 启动检查收到已验签回执后，经生产和测试入口共用的作废决策 helper 调用真实固定句柄清理，临时 Bundle 中的 Codex/MSIX 被清除，用户 sentinel、许可证、登记包和撤销回执保留。
- `node --check server/usb-license-revocation/internal/revocation/adminweb/app.js`：通过。
- 整个测试均使用临时数据库、测试密钥、回环地址和临时目录；没有运行实际签名器 GUI、实体 U 盘或真实 VPS。E2E 经测试 wrapper 注入 observer、恒定“无进程”探针、no-op close 和 no-op 状态写入；生产与测试 wrapper 共用同一作废决策 helper，但测试未执行完整 `start_for_manager_inner`、真实 USB 识别、生产 strict process probe/关闭、manager 状态落盘或 helper lease 退出。`caddy` 未安装在当前环境，反代示例尚未经过 Caddy 二进制校验。
- 签名器在 VPS 不可用状态下可重试登记：同一签名凭证只会得到同一 active 记录，不会多写审计事件；作废后重试必须维持 revoked。此处“服务端产生 key”是创建 `licenseKeyId` 绑定的授权记录，不是签发 API bearer key。
- 实体 U 盘下次联网并经管理器或 shim 入口启动后同步作废并触发清理。服务器离线时，签名正常且本地无已持久化 revoked 状态则允许启动；正常双击 U 盘 Codex 入口必须进入相同授权链。主动篡改或提取 MSIX 的技术性绕过不在此机制防护范围内。

任何正式 API 地址、服务器私钥、管理员凭据都不得写入此文档或 USB 发布包。域名确定后只把公开 URL 与公钥指纹写入正式构建记录。

### 13.7 签名器自动激活与管理员改密验证补充

- 签名器先完成 U 盘签名、登记声明写入、回读验签和 Bundle 深检，再提交登记信封；提交体只含 `payloadB64`/`signatureB64`。Sub2API 管理 Key、用户 Key、邮箱及 `accountBinding` 不参与此请求。
- `POST /v1/licenses/register` 在 Go 服务上独立限速并验证 Ed25519 原始 payload。首次登记写入既有双审计日志与 SQLite，记录 actor=`usb-signer`；精确身份重复请求不新增事件；凭证字段冲突返回 409；已作废记录重试返回 `license_revoked`，不自动恢复。
- 签名器无法访问 VPS 时保留本地签名，报告 `activationStatus=pending`；签名工具提供“同步/补发并激活”按钮，重新验证本地介质与凭证后安全重试。客户端只有拿到安装编号匹配的 `active` 响应才报告 VPS 激活成功。
- 一键建号流程只有在本地签名复核通过且 VPS 确认 active 后才允许预充值；pending、冲突或已作废状态均跳过预充值。
- 管理页首次登录要求修改 bootstrap 密码并绑定 Google Authenticator；未预置 TOTP secret 时，服务端在登录后生成 160-bit Base32 seed，首设完成前只开放初始化与退出接口。首登状态保存在 auth-state schema v3，预置 seed 不能绕过改密。改密要求当前密码及 RFC 6238 TOTP，提交时原子清理所有会话；认证状态写盘不确定时 fail-closed。
- auth-state 路径与普通文件形式的 SQLite 数据库、`-wal`/`-shm`/`-journal` sidecar、主/镜像审计日志及审计锁、restore marker 做不重合校验；不接受 SQLite URI 或内存 DSN。schema 1/2 迁移到 schema 3 时要求管理员重新完成一次首登安全设置。
- 本轮自动化验收：Go `go test ./...`（含首次登记、幂等重试、作废后不恢复、冲突凭证和 RFC 6238 固定码校验）；签名器 `cargo test --lib`（64 项）及 GUI `cargo test --bin usb-license-signer-gui`（6 项）；签名器 `cargo check --bin usb-license-signer-gui`；管理页 `node --check`。均使用本地/loopback fixture，未触碰真实 U 盘或真实 VPS。
