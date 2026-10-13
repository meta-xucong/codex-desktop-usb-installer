# USB 正式授权签名工具与全链路签名门禁开发文档

状态：实现已落盘；自动化回归与代码级审计复核已通过；正式 Windows/NTFS U 盘签名发行验收待外部证据。  
目标版本：面向当前 v0.5.24 USB 程序包。  
更新日期：2026-09-10。  
实现修订：USB-LICENSE-SIGNER-IMPLEMENTED-V1。  

本文把“USB 正式授权签名工具 + 全链路签名门禁”冻结为已实施契约，并说明它如何与当前程序包配合。本文本身不授权提交、推送、发布或接触生产私钥。

## 0. 任务记录

### 0.1 用户原始要求

本轮用户要求：把前述 USB 正式授权签名工具与全链路签名门禁方案落成开发文档，完成审计，通过后给出验收；同时确认它如何与当前程序包配合，并提供操作方法。

继承的硬约束：

| ID | 硬约束 | 检查对象 |
| --- | --- | --- |
| C1 | 复用现有 USB Guard 的算法、字段和 schema，不另造平行 License 格式 | usb_guard.rs、共享核心库、签名测试向量 |
| C2 | unsigned 升级为 signed 前必须重新观察真实物理介质，并保留 installationId | 签名器 sign 流程和回读校验 |
| C3 | 多盘、盘符变化、指纹不匹配、非 NTFS、结构缺失、已有签名不一致时 fail-closed | 签名器和运行时门禁 |
| C4 | 私钥只在离线签名环境/HSM/受保护 DPAPI 存储中；正式私钥不进入安装包、U 盘或仓库 | 构建产物扫描、密钥运行手册 |
| C5 | 普通安装器和 Codex 管理器不能在正常流程生成正式签名 | start_for_manager、安装器、发布流程 |
| C6 | 所有 Codex 安装、修复、启动、更新和写入路径统一经过 signed gate，并在实际写入/提交前再次检查 | commands.rs、offline_payload.rs、win_update.rs、channels/codex.rs |
| C7 | 能力包/CCSwitch 等非 Codex 能力与 Codex 签名门禁解耦 | capability_pack.rs、前端组件入口 |
| C8 | 既有 unsigned Codex 不自动删除；保留卸载、日志和诊断 | 卸载命令和前端状态 |
| C9 | usb-license.json 是有类型的可变身份元数据，不进入 SHA256SUMS；v2 Anchor 额外保护 SHA256SUMS 的全部非 Payload 行 | 构建脚本、verify-oneclick-usb.ps1、共享核心 |
| C10 | 正常、失败、边界、回归、真实 Windows/NTFS U 盘场景均需有可复现证据 | R1-R15 验收矩阵 |

### 0.2 基线与协作门禁

~~~text
BASELINE_COMMIT=2f20c330757a1e5e47d618526cf1a185723c752
BRANCH=checkpoint/pre-package-2026-09-09
PACKAGE_VERSION=0.5.24
WORKTREE_DIRTY=true
CONTRACT_REV=USB-LICENSE-SIGNER-V1
MAD_CONTRACT_REV=v1.3.1-route-guard
COMPLEXITY_GATE=ESCALATE_REQUIRED
CONTEXT_MODE=ARMED
~~~

工作树已有 USB、更新、能力包、卸载、构建脚本和前端改动，尤其包括：

- src-tauri/src/app/usb_guard.rs
- src-tauri/src/app/channels/codex.rs
- src-tauri/src/app/win_update.rs
- src-tauri/src/commands.rs
- src-tauri/src/app/capability_pack.rs
- src/services/managerApi.ts
- src/shared/types.ts
- tools/build-oneclick-usb-layout.ps1
- tools/build-usb-layout.ps1

这些改动属于起始基线。实施时必须使用独立工作区或先建立用户认可的基线快照，不能 reset、覆盖或把既有差异冒充为本功能成果。

本文由主控维护；实现按模块保留唯一写入边界。版本冻结后必须由不同于实现作者的只读审计职责复核。若平台没有可用的独立审计 Agent，只能报告“主控自审”，不能宣称独立审计通过。

## 1. 当前程序包事实与实现交付物

当前程序包布局由 README.offline.md 和 verify-oneclick-usb.ps1 约束：

~~~text
<USB bundle>/
├─ CodexDesktopSetup.exe
├─ CodexAppManager.exe
├─ manifest.json
├─ usb-manifest.json
├─ SHA256SUMS.txt
├─ payload/OpenAI.Codex_x64.msix
├─ apps/
│  └─ Codex/                       安装后生成
├─ content/
│  ├─ skills/
│  ├─ mcp/
│  └─ codex-starter-kit/
├─ ccswitch/CodexSwitch.exe
└─ resources/prerequisites/
   ├─ vc_redist.x64.exe
   └─ MicrosoftEdgeWebView2RuntimeInstallerX64.exe
~~~

当前已确认：

1. 现有 usb_guard.rs 已有 UsbFingerprint、UsbLicense、v1/v2 Anchor、Ed25519 验签和 USB Lease/Watchdog。
2. `crates/codex-usb-license` 已成为 USB License/Anchor/Ed25519 的共享核心；`tools/usb-license-signer` 已提供 `scan`、`inspect`、`sign`、`verify` 四个命令。
3. `sign` 只接受可移动 NTFS U 盘上的单一明确 Bundle，保留 unsigned License 的 `installationId`，使用 Windows DPAPI 密钥 Blob，并在写入后重新观察、回读、验签和深度校验。
4. `src-tauri/src/app/usb_bundle.rs`、发布脚本和独立 signer 均拒绝路径逃逸、重解析点、嵌套 `usb-license.json`、`.bak` 残留和未列入清单的不可变文件；`usb-manifest.json` 的关键哈希元数据会与实际文件交叉核对。
5. Codex 安装、修复、启动、更新、stage、commit、payload 写入和 U 盘介质删除路径已接入统一 signed gate；能力包、CCSwitch、日志、诊断和普通卸载保持独立边界。
6. `build-installer.ps1` 在正式策略下要求公钥和 keyId，并以 `custom-protocol,signed-required` 编译 USB 管理器；运行时不存在可降级到 unsigned 的正式策略路径。
7. 当前 v0.5.24 包仍可作为结构联调 Bundle，但其 `CodexDesktopSetup.exe` 等 PE 当前为 `NotSigned`，因此只能通过开发态结构验证，不能直接宣称正式授权发行包。
8. 当前最终联调候选 `release/usb-oneclick-v0.5.24-current-dev-candidate-final-20260910` 中，CodexDesktopSetup.exe 和 CodexAppManager.exe 的 Authenticode 状态实测为 NotSigned；严格 verify-oneclick-usb.ps1 会在 Authenticode 阶段退出码 1。
9. `verify-oneclick-usb.ps1` 的 `-SkipSignature` 只绕过 Authenticode 检查，不能当作 USB License 签名开关；USB License 必须由独立 signer 在真实物理介质上签发并由管理器运行时验证。

## 2. 冻结的 License 与物理身份契约

### 2.1 物理指纹

继续使用现有四元组：

~~~text
volumeSerial
anchorSha256
bundleRelativePath
anchorSchema
~~~

盘符不是运行时物理身份的一部分。运行时允许同一物理介质更换盘符，但必须重新发现四元组完全一致的 Bundle。

签名器比运行时更严格：一次 sign 操作从确认到提交期间，如果目标盘符变化，直接失败并要求重新 scan。

### 2.2 License JSON

第一版继续使用现有 schemaVersion=1，不增加新的正式字段：

~~~json
{
  "schemaVersion": 1,
  "product": "codex-usb-oneclick",
  "installationId": "uuid",
  "fingerprint": {
    "volumeSerial": 123456,
    "anchorSha256": "64 lowercase hex characters",
    "bundleRelativePath": "codex-usb",
    "anchorSchema": 2
  },
  "issuedAt": "1770000000000",
  "keyId": "prod-2026-01",
  "signature": "standard-base64-ed25519-signature"
}
~~~

签名输入是去掉 signature 后的现有 UnsignedLicense 紧凑 JSON：

- UTF-8；
- 无空白；
- 使用现有 serde 字段顺序，不自行改成字母排序；
- fingerprint 使用现有 camelCase 字段；
- signature 不参与自身签名；
- signature 是 64 字节 Ed25519 原始签名的标准 Base64；
- keyId 必须参与签名；
- 正式 signed License 必须有 keyId；
- 签名器和管理器都必须拒绝未知字段、非法路径和不符合 schema 的类型。

现有 v1 旧签名对缺少 anchorSchema 的 legacy payload 保持验证兼容。新签名器始终生成包含 anchorSchema 的当前 payload。新正式 Bundle 默认 anchorSchema=2；v1 只作为旧包兼容，不作为新正式签名的默认输出。
### 2.3 installationId 与 issuedAt

升级已有 unsigned license 时：

1. 重新观察当前物理介质；
2. 读取旧 License；
3. 验证旧 installationId 可解析且旧指纹与当前介质一致；
4. 保留原 installationId；
5. 用当前观察到的指纹重建待签名 License；
6. issuedAt 更新为本次签名的 UTC 毫秒时间；
7. 不生成替代 installationId 来掩盖指纹不匹配。

已有签名但签名错误、keyId 不符合、指纹不匹配或 JSON 非法时，禁止覆盖。

### 2.4 为什么 License 不进入 SHA256SUMS

SHA256SUMS 保护的是复制前已经确定的不可变发行文件；USB License 必须在 Bundle 复制到某一块真实 U 盘后才绑定卷序列号、Anchor 和相对路径。如果把 License 加入 SHA256SUMS，签名动作会同时破坏自己刚生成的发行清单。

因此：

- 授权契约只允许 Bundle 根目录的 usb-license.json 作为可变身份元数据；
- 任意子目录中的同名文件都不是 License，应按未知文件拒绝；
- usb-license.json.bak 不是合法 SHA256SUMS 排除项，而是事务临时残留；成功状态必须清理，当前严格验证器发现它时应失败；
- Manifest、Payload、入口文件、能力包和其他不可变文件继续由 SHA256SUMS 保护；
- License 自己由 Ed25519、实时物理指纹和严格 JSON 校验保护；
- 删除 License 不会被当作“完整性通过”，而会让 Codex signed gate 失败。

当前构建脚本、运行时校验器和验证脚本都只把根目录相对路径 `usb-license.json` 作为 SHA256SUMS 排除项，并拒绝任意层级的同名残留、`.bak` 和重解析点。

### 2.5 三类签名的边界

本项目有三种互不替代的签名/完整性机制：

| 机制 | 保护对象 | 结论 |
| --- | --- | --- |
| USB License Ed25519 | 当前物理 U 盘、Bundle 相对路径和授权元数据 | 证明“这块介质被授权”，不是 Payload 内容签名 |
| MSIX Authenticode | 官方 Codex MSIX 容器 | 证明载荷来源和签名状态 |
| PE Authenticode | Launcher、Manager、依赖程序 | 证明 Windows 可执行文件发布签名 |

当前 v2 Anchor 不直接覆盖 Payload 文件本身，也不覆盖 SHA256SUMS 中唯一的当前 Payload 行；它会覆盖 SHA256SUMS 的规范化全部其他行（路径、哈希、排序和重复项语义）。因此文档和 UI 不得把 Anchor 单独表述为“全包内容签名”。正式完整性结论必须同时依赖 Anchor、SHA256SUMS、Payload 检查和相应的 Authenticode 校验。改变任何稳定文件或其清单行都必须重新计算 Anchor 并重新签发 License；仅改变当前 Payload 行是 USB 版本更新的显式动态范围。

## 3. 正式签名工具

### 3.1 形态选择

第一阶段已采用独立 Rust CLI：

~~~text
crates/codex-usb-license/       纯核心：schema、canonical payload、Anchor、签名/验签
tools/usb-license-signer/       Windows 签名器：扫描 NTFS U 盘、确认、签名、报告
src-tauri/src/app/usb_guard.rs  运行时：卷观察、Lease、Watchdog、门禁适配
~~~

不把私钥能力放进 CodexAppManager.exe、CodexDesktopSetup.exe 或 U 盘包。当前已提供独立签名器 GUI；它复用同一 Rust 核心，且只能在签名机上运行。GUI 的交付、联调和双击操作详见 `docs/USB-LICENSE-SIGNER-GUI-DEVELOPMENT.md`。

CLI 更适合第一版的原因：

- 确认口令、退出码和审计报告容易复现；
- 不会把正式签名入口暴露给普通用户；
- 可在隔离签名机/HSM 上运行；
- 可以直接纳入发布前人工清单；
- GUI 只是 CLI 核心的操作层，不改变安全契约；其“签名授权”按钮只有在 CLI 深度校验、DPAPI Blob、本地路径和单候选条件全部满足时才可用。

建议命令：

~~~text
usb-license-signer scan
usb-license-signer inspect --drive E: --bundle-relative-path CodexUsb
usb-license-signer sign --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "SIGN <volumeSerial>-<anchorSha256-first12>"
usb-license-signer verify --drive E: --bundle-relative-path CodexUsb
~~~

### 3.2 scan/inspect 阶段

scan 可以列出候选盘，但 sign 必须满足：

- 用户明确指定目标；
- 只有一个候选 Bundle；
- 目标卷为 DRIVE_REMOVABLE；
- 文件系统为 NTFS；
- 目标 Bundle 路径合法且位于当前卷内；
- 不能通过 UNC、junction、symbolic link、mount point 或 reparse point 绕过边界；
- 确认后再次检查盘符、卷序列号和 Bundle 路径。

签名器必须复用现有 Bundle 校验逻辑，而不是只检查几个文件名。至少包括：

- manifest.json 和 usb-manifest.json 存在且 schema 正确；
- updateCapability、programLayout、dataLayout、managedContent 等契约正确；
- SHA256SUMS.txt 存在、格式正确、清单项唯一且与文件实际 hash 一致；
- Payload 路径、大小、hash、MSIX 身份、架构和 Authenticode 校验一致；
- CodexDesktopSetup.exe、CodexAppManager.exe、CCSwitch、WebView2、VC++ 依赖存在并符合清单；
- content/codex-starter-kit 的 pack.json、dependencies.json、file-manifest.json、checksums.sha256 和安装脚本完整；
- 不存在未知文件、临时事务残留或跨卷链接；
- 新鲜发行包不要求 apps/Codex 已经存在；若目录已存在，签名器只接受空目录。该目录是安装后生成的受管程序目录，运行时会在独立的受控边界内允许它出现，但不能把安装后运行时树重新当成待签发行包。

### 3.3 用户确认、签名和回读

确认界面或 CLI 输出必须包含：

- 盘符、卷标、卷序列号、文件系统；
- Bundle 相对路径；
- anchorSha256、anchorSchema；
- installationId；
- 当前 License 状态；
- Payload hash/size；
- SHA256SUMS 结果；
- keyId。

用户必须输入精确确认字符串，例如：

~~~text
SIGN <volumeSerial>-<anchorSha256前12位>
~~~

写入策略：

1. 记录旧 License 字节到签名机本地审计目录；
2. 在同一 NTFS 卷生成临时 License；
3. 原子替换 usb-license.json；
4. 重新读取 License；
5. 重新观察盘符、卷序列号、Bundle 相对路径和 Anchor；
6. 用目标公钥完成 Ed25519 验签；
7. 再跑完整 Bundle、SHA256SUMS、Payload 和路径校验；
8. 成功后只把报告写到签名机本地。

介质在写入后消失或发生变化时，不能报告“签名成功”。如果替换后无法安全判断介质状态，不得盲目覆盖恢复旧文件，必须报告“结果未知，需要人工复核”。

### 3.4 密钥策略

正式管理器只内置公钥。正式构建通过编译期输入，并固定 SignedRequired 策略：

~~~text
CODEX_USB_GUARD_PUBLIC_KEY_B64
CODEX_USB_GUARD_PUBLIC_KEY_ID
CODEX_USB_GUARD_POLICY=SignedRequired
~~~

正式构建不从当前电脑的 DPAPI 公钥描述文件自动选择发行密钥。发行公钥必须从当前版本
显式传入的 release profile 读取（只含公钥，不含私钥）；正式脚本省略 profile 参数时直接
失败，不再回退到仓库中的固定 profile。环境变量为空时只临时初始化为该版本 profile，环境
变量已设置但摘要不一致时直接失败。密钥轮换或版本绑定变化必须先生成新的版本 profile，
再重新编译管理器、生成 Bundle 和签名器，并在匹配的 portable vault 环境签名。

为支持轮换，可增加仅包含公钥的编译期 keyring；现有变量作为 active key 兼容入口。每个 keyId 映射到一个公钥和状态：

- active：签名器当前使用；
- verify-only：仍可验证历史 Bundle；
- revoked：不再授权。

管理器必须先按 keyId 选择允许的公钥，再验签；不得把 License 中的 keyId 当作可信配置。

私钥存储优先级：

1. HSM/Windows CNG/KSP；
2. 离线签名机上的 DPAPI 机器/用户绑定文件；
3. 不允许明文私钥、命令行私钥、环境变量私钥或 U 盘私钥。

测试 key、生产 key、签名机和报告目录必须分离。生产私钥不得出现在安装包、U 盘、Git 仓库、CI 日志或崩溃报告中。

### 3.5 两种构建策略

构建时必须有明确、不可由普通用户运行时降级的策略：

| 策略 | 适用范围 | 缺少或未签名 License 时 |
| --- | --- | --- |
| DevelopmentAutoEnroll | 本地开发、自动化夹具和联调包 | 可以生成 unsigned License，明确标记开发状态；不得作为发布证据 |
| SignedRequired | 正式管理器和正式 U 盘包 | 不写 License、不建立 Codex 授权 lease、不执行 Codex 相关副作用；保留受限诊断、日志和用户主动卸载通道 |

SignedRequired 不是一个用户可修改的运行时开关。正式构建必须由编译配置和发布清单固定，并在构建报告中记录策略、公钥 keyId 和公钥摘要。

build-installer.ps1 和 USB layout/verify 脚本在 SignedRequired 发布模式下会检查策略、预期 keyId 和公钥摘要，并拒绝当前环境与发行 profile 不一致的公钥；缺少公钥、keyId 或正式 PE 签名时失败。构建日志不得打印私钥或完整秘密环境变量。当前代码没有私钥 provision CLI，DPAPI Blob 必须由受控离线流程/HSM 预置。

## 4. 全链路签名门禁

新增统一后端策略入口：

~~~text
require_signed_codex_bundle(phase)
~~~

它返回带物理快照的 SignedUsbAuthorization，而不是只返回 bool：

~~~text
bundleRoot
codexRoot
fingerprint
installationId
keyId
licenseDigest
checkedAt
~~~

USB 模式下的检查顺序：

1. guard_required 为真；
2. status 为 authorized；
3. watchdog 为 running；
4. 当前卷为 removable 且为 NTFS；
5. 当前管理器 Bundle 路径有效；
6. 重新观察真实卷和 Bundle；
7. Lease 所属进程、路径和指纹与当前观察一致；
8. 读取 usb-license.json；
9. License 指纹与当前观察完全匹配；
10. keyId 命中编译期公钥；
11. Ed25519 验签成功；
12. 路径、reparse point、Bundle 结构和 Codex 目标检查通过；
13. 写入/更新阶段额外执行完整 SHA256SUMS、Payload 和 Manifest 校验。

status.signed 只能用于前端提示，不能作为最终授权依据。

必须调用正式 signed gate 的入口：

- offline_payload_stage；
- USB 模式 win_install_historical_release；
- Codex 安装和修复；
- win_perform_update；
- payload_check_codex；
- payload_apply_codex；
- win_launch_codex；
- authorized_usb_codex_root；
- USB 模式 win_adopt 和 win_adopt_path；
- USB 模式 win_stage_update 及其实际 Codex 写入路径；
- win_plan_update 可以只展示计划，但不得为 unsigned USB 创建或提交 Codex 更新；
- Codex Payload 事务 commit；
- 任意写入 apps/Codex 或本地 Codex 程序目录的后端路径；
- 任意会启动 USB 中 Codex 的主题、辅助或重启路径。

### 4.1 TOCTOU 门禁

一个操作至少在三处校验：

~~~text
命令入口
  ↓
开始实际写入前
  ↓
rename/replace/commit 前
~~~

下载期间不持有永久授权。若 USB 被拔出、替换、改盘符、License 被改写、Manifest 变化或 SHA256SUMS 变化，提交必须失败，旧程序仍保留。

通用 `win_perform_update` 在检测到当前进程属于 USB Bundle 时强制解析为 `portable-usb`，不会继承本地管理器的 `msix`/普通 `portable` 偏好；其共享提交尾部以 `usb_portable=true` 执行最终 `require_signed_codex_bundle` 和授权 `codexRoot` 对比。历史 USB 安装命令同样使用该提交尾部。

### 4.2 不要求 signed gate 的路径

这些操作不是 Codex 授权使用，可以保留较弱检查：

- 状态查看、Payload probe、日志和诊断；
- 用户明确发起的卸载；
- 受路径安全和 Bundle 所有权保护的清理；
- 能力包状态、能力包安装和 CCSwitch 单独重试。

能力包使用独立的 require_usb_content_access，只校验 Bundle 和能力包自身，不要求 Codex 已安装、License 已签名或 Feishu/API Key 已填写。

### 4.3 启动顺序与前置副作用

正式 SignedRequired USB 启动必须把授权预检前移到第一个受保护副作用之前。尤其要覆盖当前 lib.rs 启动序列中可能发生的：

- WebView2 或 VC++ 运行库自动安装；
- 旧事务恢复；
- 快捷方式清理；
- USB Lease 建立；
- Codex/CCSwitch 自动启动。

无有效 signed License 时，正式构建不得仅记录 warning 后继续。可以保留一个只读错误/诊断/卸载入口，但不能通过它安装依赖、建立 Codex lease、写入 Codex 程序或启动 Codex。所有隐藏或未在 UI 展示的 Tauri command 也必须按同一策略审计，不能因为按钮不可见就视为安全。

## 5. unsigned、旧包和密钥轮换兼容

| License 状态 | 状态展示 | Codex 安装/修复/启动/更新 | 能力包/诊断/卸载 |
| --- | --- | --- | --- |
| 缺失 | missing | 禁止 | 允许安全诊断；不得正式自动生成 |
| unsigned | unsigned-development | 禁止 | 允许 |
| signed v1 + 已知 keyId | legacy-signed | 可按兼容策略运行；Payload 更新建议锁定 | 允许 |
| signed v2 + 已知 active/verify-only key | valid | 允许 | 允许 |
| unknown key / invalid signature | invalid | 禁止 | 允许诊断和卸载 |
| fingerprint mismatch | fingerprint-mismatch | 禁止 | 禁止依赖该身份的操作 |

正式构建不自动把 unsigned license 升级为 signed。dev/test 构建可以保留 auto-enroll，但必须明确标记为开发状态，且不能通过运行时环境变量把正式构建降级成 dev 策略。

旧 unsigned Codex 不自动删除。USB 管理器不允许启动、修复或更新它，但保留用户主动卸载、状态查看、日志和诊断。

### 5.1 卸载与删除介质的区分

普通 Codex/CCSwitch 卸载不属于授权使用，应保留 unsigned 状态下的用户主动卸载。

删除整个 USB 安装介质是破坏性动作，不能与普通卸载混为一谈。自动删除介质必须持有当前物理指纹、License digest、不可变清单校验和明确确认；正式策略下 unsigned、缺失 License 或 License 无法验签时，保留介质并报告原因，用户可以在确认后自行处理文件。Detached finalizer 必须携带卸载前冻结的身份快照和计划，不能在删除 License 后重新依赖已删除文件。

## 6. 前端状态与错误

在现有 UsbGuardStatus 的 signed 字段之外，建议增加：

~~~text
signatureState:
  not-applicable
  missing
  unsigned-development
  valid
  invalid
  fingerprint-mismatch
  unknown-key
  legacy-unsupported

codexAllowed
codexBlockReason
~~~

前端必须禁用：

- unsigned/invalid 状态下的 Codex 安装；
- Codex 修复；
- Codex 启动；
- Payload 检查和更新；
- USB 模式任何 Codex 写入。

前端不能把签名错误显示成 Feishu/API Key 错误，也不能因为能力包未安装或 Codex 尚未安装而阻断独立能力包操作。

建议稳定错误码：

~~~text
usb_license_missing
usb_license_unsigned
usb_license_invalid
usb_license_unknown_key
usb_fingerprint_mismatch
usb_anchor_mismatch
usb_bundle_integrity_failed
usb_bundle_path_invalid
usb_not_ntfs
usb_multiple_candidates
usb_drive_changed
usb_toctou_changed
usb_watchdog_not_running
usb_codex_path_invalid
~~~

## 7. 与当前 v0.5.24 程序包的配合方法

### 7.1 当前状态的重要说明

当前仓库已经包含 `tools/usb-license-signer` 源码和可构建的 `usb-license-signer.exe`。当前 v0.5.24 联调候选包的 PE 仍未完成正式 Authenticode，因此它只能：

- 作为合法 Bundle 结构的构建和验证输入；
- 作为 unsigned 开发包进行结构、启动受限态、能力包和安装事务门禁联调；unsigned 状态不能执行 Codex 安装、启动或更新；
- 在完成项目拥有的 PE/MSIX Authenticode 和 DPAPI 密钥预置后，作为正式 signer 输入。

当前包如果直接运行，管理器可能自动生成 unsigned usb-license.json。这个行为不能作为正式授权成功，也不能作为正式发布验收。

### 7.2 生成并检查当前 Bundle

在仓库根目录执行：

~~~powershell
Set-Location 'D:/AI/AGENT_U盘/codex-desktop-usb-installer'
# 开发联调包必须显式选择开发策略；脚本默认 SignedRequired。
pwsh ./tools/build-installer.ps1 -UsbPolicy DevelopmentAutoEnroll
$installer = './release/setup/CodexUsbLauncher.exe'
$manager = './release/setup/CodexAppManager.exe'
$inputBundle = './release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910'
if (-not (Test-Path -LiteralPath $inputBundle)) { throw "缺少构建输入 Bundle：$inputBundle" }
pwsh ./tools/build-oneclick-usb-layout.ps1 -InstallerPath $installer -ManagerPath $manager -BundlePath $inputBundle -OutputPath ./release/usb-oneclick-v0.5.24-signing-candidate -SkipSignature
pwsh ./tools/verify-oneclick-usb.ps1 -BundlePath ./release/usb-oneclick-v0.5.24-signing-candidate -SkipNetwork -SkipSignature
~~~

当前工作树不存在默认的 release/usb-final，不能省略 -BundlePath、-InstallerPath 或 -ManagerPath。仓库中已观察到可供当前联调的结构合格目录是：

~~~text
release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910
release/usb-oneclick-v0.5.24-current-dev-candidate-final-20260910
~~~

该目录的 manifest.json 和 usb-manifest.json 均为 schemaVersion=2，且包含 content/codex-starter-kit。较早的 release/usb-oneclick-v0.5.24-final 不满足当前 verify-oneclick-usb.ps1 的结构要求，不应继续作为操作示例。

正式发布不要使用 -SkipSignature。现有参数主要控制 MSIX/AuthentiCode 测试夹具，不能被解释为 USB License 签名开关。

如果当前构建产物本身尚未有 Windows Authenticode 签名，开发阶段可以使用以下结构校验命令：

~~~powershell
pwsh ./tools/verify-oneclick-usb.ps1 -BundlePath ./release/usb-oneclick-v0.5.24-signing-candidate -SkipNetwork -SkipSignature
~~~

这只能证明 Bundle 结构、Payload、Manifest 和 SHA256SUMS 通过，不能证明正式发布资格，也不能代替 USB License 签名。正式发布必须使用没有 -SkipSignature 的命令，并先完成项目规定的 Windows Authenticode 签名。

### 7.3 复制到真实 U 盘

先确认目标盘为 NTFS，并将完整输出目录复制到 U 盘，不要只复制 EXE：

~~~powershell
$source = (Resolve-Path './release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910').Path
$target = 'E:/CodexUsb'
robocopy $source $target /E /COPY:DAT /DCOPY:DAT /XJ /R:2 /W:1
if ($LASTEXITCODE -gt 7) {
  throw "Bundle 复制失败，退出码=$LASTEXITCODE"
}
~~~

E: 只是示例，实际盘符必须由签名器扫描确认。复制完成后不要在签名前运行会改写 Bundle 的安装、更新或能力包操作。

### 7.4 正式签名后的操作

先在离线签名机准备 DPAPI Blob。Blob 由受控 provision 流程或 HSM 生成，解密后的 envelope 必须是 `CODEX-USB-SIGNER-DPAPI`、version=1、目标 keyId 和 32 字节 Ed25519 seed；本 CLI 不接受明文私钥，也没有从命令行创建私钥 Blob 的入口。然后执行：

~~~powershell
usb-license-signer.exe scan
usb-license-signer.exe inspect --drive E: --bundle-relative-path CodexUsb
usb-license-signer.exe sign --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "SIGN <volumeSerial>-<anchorSha256前12位>" --report 'C:/UsbSigningReports/v0.5.24-E.json'
usb-license-signer.exe verify --drive E: --bundle-relative-path CodexUsb
~~~

sign 命令成功必须同时满足：

- 输出 Bundle 结构检查通过；
- 实际卷为 NTFS；
- License 写入后回读成功；
- Ed25519 验签成功；
- 当前指纹与 License 一致；
- SHA256SUMS、Payload、Manifest 和路径检查通过；
- 报告写入签名机本地；
- U 盘根目录没有临时签名残留。

`inspect` 可以先只读确认 Bundle 和当前 License 状态；`verify` 需要已存在的 signed License，并默认从本机 DPAPI Blob 取得公钥，也可在只读审计场景显式传入公钥 Base64。签名报告和旧 License 审计副本只写入签名机本地，不能写入 U 盘。

### 7.5 使用正式管理器

正式管理器构建时只注入发行 profile 中的公钥和 keyId。不要把当前电脑自动生成的 DPAPI 公钥当作发行公钥：

~~~powershell
$profile = './release-profiles/usb-v0.5.24-g28.json'
pwsh ./tools/build-installer.ps1 -UsbPolicy SignedRequired -UsbSigningProfilePath $profile
$installer = './release/setup/CodexUsbLauncher.exe'
$manager = './release/setup/CodexAppManager.exe'
$inputBundle = './release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910'
if (-not (Test-Path -LiteralPath $inputBundle)) { throw "缺少构建输入 Bundle：$inputBundle" }
pwsh ./tools/build-oneclick-usb-layout.ps1 -InstallerPath $installer -ManagerPath $manager -BundlePath $inputBundle -OutputPath ./release/usb-oneclick-v0.5.24-signed-ready -UsbSigningProfilePath $profile
~~~

每次发行都必须显式使用该版本已审阅的公开 profile；禁止直接把本机
`%LOCALAPPDATA%\CodexUsbLicenseSigner\keys\*.public.json` 复制进构建环境冒充发行配置。

然后重新把最终构建输出复制到目标 NTFS U 盘，再执行签名器。不要先签名、再替换管理器、入口、Manifest、Payload 或 SHA256SUMS；任何不可变发行文件变化都必须重新构建、复制和签名。

正式使用步骤：

1. 插入已经签名的目标 U 盘；
2. 双击 U 盘根目录 CodexDesktopSetup.exe；
3. 接受标准 UAC；
4. 等待管理器验证当前物理指纹、License、Bundle 和 Payload；
5. 首次安装 Codex；
6. 安装或重试独立能力包；
7. 日常始终从 CodexDesktopSetup.exe 启动；
8. Payload 更新前后均由后端重新验证签名和 Bundle。

日常建议使用 CodexDesktopSetup.exe。apps/Codex 中由 MSIX manifest 声明的入口必须替换为受清单保护的 CodexEntryShim.exe；shim 通过同一用户会话的命名管道把启动请求交给 USB 管理器，统一校验本地签名、租约和在线可获得的远程作废状态。缺少 shim、管理器或校验失败时必须拒绝启动，不得回退到直接启动 Codex.exe。发行包必须包含并校验 CodexEntryShim.exe；该保证不覆盖用户自行复制、改写或替换掉受清单保护的入口文件。

### 7.6 正式发布顺序

正式包不能先签 USB License 再替换程序文件。顺序固定为：

1. 以 SignedRequired、当前版本 profile、公钥和预期 keyId 构建 Manager；
2. 完成 Launcher、Manager 和项目拥有的 PE 层 Authenticode；
3. 生成最终 Manifest、usb-manifest 和 SHA256SUMS；
4. 使用没有 -SkipSignature 的严格 Bundle 验证；
5. 将最终 Bundle 复制到目标 NTFS U 盘的最终相对路径；
6. 在该真实物理 U 盘上运行独立 signer；
7. 在同一 U 盘上运行 signer verify 和严格 Bundle 验证；
8. 归档版本、源码 revision、文件 hash、keyId、公钥摘要和签名报告。

当前仓库的 code-signing-policy.md 明确记录 Windows 工件仍可能是未签名状态；在 Authenticode 尚未完成时，不能把 v0.5.24 的结构验证结果包装成正式发布通过。

## 8. 实施拆分、唯一写入边界与当前状态

契约冻结后已按以下顺序实施；表中的证据指向当前工作树，最终版本冻结后由独立审计复核：

| 阶段 | 唯一写入范围 | 前置条件 | 主要证据 |
| --- | --- | --- | --- |
| A | crates/codex-usb-license、usb_guard 适配 | 已完成 | 7/7 核心测试；JSON/Anchor/签名向量 |
| B | tools/usb-license-signer | 已完成 | 18/18 signer 测试；release build/help/scan |
| C | usb_gate、commands、offline/update/channels、lib.rs 启动顺序 | 已完成 | Tauri 入口与 pre-write/pre-commit 回归 |
| D | capability_pack、codex_theme、usb_media_cleanup、卸载边界 | 已完成 | unsigned/能力包/卸载/清理测试 |
| E | types.ts、managerApi.ts、OfflineHome、组件页和 i18n | 已完成 | 395/395 前端测试 |
| F | build-installer、USB build/verify 脚本、发布和密钥手册 | 已完成 | 候选包 SkipSignature 结构校验；严格模式拒绝未签名 PE |
| G | 独立审计与整改复核 | 已完成（代码级） | Singer/Laplace 独立问题清单已整改；主控复核与聚焦门禁复核通过；实机证据仍待补 |

### 8.1 当前自动化证据

截至 2026-09-10，代码与前端回归结果如下：

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path crates/codex-usb-license/Cargo.toml` | 7 passed |
| `cargo test --manifest-path tools/usb-license-signer/Cargo.toml` | 18 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 369 passed, 1 ignored |
| `cargo test --manifest-path src-tauri/Cargo.toml --features signed-required` | 369 passed, 1 ignored |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml` | 178 passed, 1 ignored |
| `npm test` | 395 passed in 34 files |
| `npm run check` / `npm run lint` / `npm run build` | 全部通过 |
| `pwsh ./tools/test-offline-bundle.ps1` | 通过；合成 MSIX，`outputFiles=277`、`checksumEntries=276`，未生成 WindowsApps/WpSystem |
| `build-installer.ps1 -UsbPolicy DevelopmentAutoEnroll -SkipNpmInstall -SkipTests` | 通过；manager/launcher 的 requireAdministrator 资源清单已验证 |
| 候选包 `verify-oneclick-usb.ps1 -SkipNetwork -SkipSignature` | 通过，`ready=true` |
| 候选包严格验证 | 按预期失败，`EXPECTED_STRICT_EXIT=1`，原因是当前 PE `NotSigned` |
| signer release build、`--help`、`scan` | 通过；当前 F: NTFS 介质发现 `candidateCount=1` |
| 当前 dev candidate 的嵌入 `verify-usb.ps1` | 通过；包含运行期根 `usb-license.json` 不得进入 SHA256SUMS 的保护断言 |
| 当前 F: 已安装运行树上的 signer `inspect` | 按预期拒绝；明确提示只能对 fresh Bundle 签名，不覆盖 `apps/Codex` |

Tauri 编译期间反复出现仓库既存 `codex-theme-engine` 增量元数据目录的 `拒绝访问` warning，但编译、测试退出码均为 0；该 warning 不属于本功能错误。上述证据不包含真实可移动 NTFS 介质、真实 DPAPI Blob、生产公钥或正式 Authenticode 证书。

执行 Agent 不得同时修改共享 schema、前端 DTO、门禁语义或其他 Agent 的文件。任何需要改公共契约的发现必须先提交 DESIGN_QUESTION 并回到契约冻结。

## 9. 验收矩阵

| ID | 场景 | 预期 |
| --- | --- | --- |
| R1 | 新 NTFS Bundle 签名 | 成功，回读和验签通过 |
| R2 | 多个候选可移动盘 | sign 拒绝 |
| R3 | FAT/exFAT/未知文件系统 | sign 拒绝 |
| R4 | unsigned 升级 | 保留 installationId，生成有效 signed License |
| R5 | 已有错误签名 | 不覆盖，失败 |
| R6 | Bundle 复制到另一块 USB | 指纹不匹配，失败 |
| R7 | 确认后盘符变化 | sign 失败并要求重新扫描 |
| R8 | Manifest/Payload/SHA256SUMS 被修改 | 安装、启动、更新失败 |
| R9 | 删除 License 或 signature | 正式 Codex gate 失败 |
| R10 | UI 检查后 USB 被替换 | pre-write/pre-commit 失败且无部分提交 |
| R11 | unsigned Codex 已存在 | 不自动删除；不可启动/修复/更新；可卸载 |
| R12 | 能力包独立安装 | 不因 Codex 未安装或 Feishu 未配置而失败 |
| R13 | keyId 轮换和 revoked key | 仅允许策略内公钥 |
| R14 | 非 USB 管理器 | 现有正常 Windows 行为不变 |
| R15 | 真实 Windows NTFS U 盘 | 完成签名、安装、启动、更新、卸载实机证据 |

自动测试至少覆盖：

- canonical payload 的 JSON 字段、空白、字段顺序和 Base64；
- v1 legacy payload 与 v2 Anchor；
- keyId、未知 key、错误签名和签名字段篡改；
- Bundle 路径、reparse point、未知文件和临时残留；
- SHA256SUMS 排除 License 但拒绝删除 License；
- unsigned/signed/invalid 前端状态；
- install、repair、launch、payload check/apply、stage、commit 各入口；
- 真实 NTFS U 盘、拔插、盘符变化和介质替换。

建议验证命令：

~~~powershell
npm run check
npm run test:offline
npm test
cargo test --manifest-path ./src-tauri/Cargo.toml
cargo test --manifest-path ./src-tauri/Cargo.toml --features signed-required
cargo test --manifest-path ./crates/codex-usb-license/Cargo.toml
cargo test --manifest-path ./tools/usb-license-signer/Cargo.toml
cargo build --release --manifest-path ./tools/usb-license-signer/Cargo.toml
pwsh ./tools/test-offline-bundle.ps1
pwsh ./tools/verify-oneclick-usb.ps1 -BundlePath ./release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910 -SkipNetwork -SkipSignature
pwsh ./tools/verify-oneclick-usb.ps1 -BundlePath ./release/usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910 -SkipNetwork
git diff --check
~~~

当前证据中，第一条 Bundle 验证通过；第二条严格验证按预期因候选包 PE 为 `NotSigned` 失败。后者是发布门禁的负向证据，不是正式包通过证据。真实签名器的 `scan` 已在当前 F: NTFS 介质发现一个候选，但该介质上的 Bundle 已经生成 `apps/Codex` 安装树，因此 `inspect/sign` 会按新鲜目标规则拒绝并提示先使用未安装的复制目标。自动化夹具已经覆盖 R8-R12 及关键边界；R1-R7、R10、R13、R15 仍需在正式 AuthentiCode、DPAPI Blob、生产 key 配置和真实签名目标上补证。

## 10. 审计与交付门禁

文档审计必须逐条回答：

| 审计项 | 方法 | 通过条件 |
| --- | --- | --- |
| 范围 | 对照 C1-C10 和实际文件清单 | 无未授权业务功能 |
| 当前包配合 | 对照 README、build、verify 脚本 | 命令、路径、v0.5.24 结构一致 |
| Schema | 对照 usb_guard.rs 与测试 | 无平行格式或字段漂移 |
| 安全边界 | 对照 signer、key、gate 设计 | 私钥隔离、fail-closed、TOCTOU |
| 全链路 | 对照 commands.rs 和模块调用图 | 所有 Codex 写入/启动入口覆盖 |
| 能力包 | 对照 capability_pack 与前端入口 | 与 Codex 签名门禁解耦 |
| 兼容 | 对照 unsigned、v1、v2、卸载 | 不自动删除、不误锁普通管理器 |
| 验收 | 对照 R1-R15 | 未验证项单独列出，不包装成通过 |

当前交付结论分两层：代码实现、静态门禁和自动化回归通过；正式发行验收仍为条件未满足。Singer/Laplace 的独立审计问题已经逐项整改并复核，当前没有遗留 P0/P1 代码问题；但当前候选 PE 实测仍为 `NotSigned`，且没有生产公钥、DPAPI/HSM 私钥供给和真实 NTFS U 盘端到端记录，因此不能把本轮结果表述为正式发行通过。

## 11. 回滚与未验证项

### 11.1 签名器失败

- 临时文件不进入成功状态；
- 已有 unsigned License 不被破坏；
- 已有错误 signed License 不被覆盖；
- Bundle 变化时要求重新 scan；
- 结果未知时暂停人工复核，不自动重复签名。

### 11.2 安装/更新失败

- 使用现有 staged/backup 事务逻辑；
- 签名门禁失败时不得删除旧 Codex；
- 提交阶段失败时恢复旧程序树；
- License 仍保持原字节，除非签名器明确完成一次独立签名操作。

### 11.3 当前未验证项

- 真实 NTFS U 盘的卷序列号与盘符变化；
- 真实拔插/替换造成的 TOCTOU；
- HSM 或 DPAPI 私钥托管；
- 生产公钥轮换和撤销；
- 现有 v0.5.24 二进制在签名 License 下的端到端运行；
- 当前外部发布服务和正式 Windows Authenticode 发布链路。

这些项目必须在最终实机验收中补证，不能用临时目录测试代替。
