# Codex U 盘更新稳定锚点与来源降级开发文档

状态：本地发布门禁已通过，待目标机/U盘实测（G29）  
契约版本：`v2.3-stable-anchor-r2`  
适用范围：Codex U 盘发行包的本体更新链路；Skill Hub 仍是独立的本地能力包更新链路。

## 1. 需求与问题边界

用户要求 Codex 本体可以通过“检查并安装更新”安全升级，同时保留 U 盘签名保护；当中国大陆无法直连 GitHub 时，应按已配置的中转/镜像顺序继续寻找可用更新源。更新失败必须给出可理解的原因，不能把网络暂时不可用、源版本落后或旧签名包不支持更新混成同一个“超时”。

本次审计发现两个确定问题：

1. 现有 v2 锚点把 `manifest.json`、`usb-manifest.json` 以及 `payload/_backup` 中的 SHA256SUMS 行算入信任锚点；一次合法更新会修改这些发布元数据或生成备份行，因而更新完成后会被 USB Guard 判定为 `usb_bundle_anchor_changed`。
2. 自动来源选择在首个 relay 健康但版本不高于本机版本时直接返回“无更新”，没有继续尝试后续的 GitHub/镜像源；当前配置中的 relay 已经出现过落后于镜像版本的实际情况。

## 2. 目标、非目标与安全底线

### 2.1 目标

- 新发行包使用 schema 3 稳定更新锚点：合法更新只改变允许的动态元数据、当前 payload 和单一备份记录，不改变锚点。
- schema 1/2 保持读取和启动兼容；旧包不伪装成可更新包，更新入口明确提示“需要重新打包签名”。
- 更新在任何破坏性 rename 前完成签名锚点预检；提交后再次验证锚点、签名授权、SHA256SUMS 和元数据；任一校验失败都走原子回滚。
- Auto 来源在候选源“不可达、响应非法、版本过旧、版本不高于本机”时继续尝试下一个候选；显式选择的单一来源不越权切换。
- apply 阶段锁定 check 阶段确认的来源与 release identity，防止检查和安装之间静默换源。
- 只有全部门禁通过后才生成新的安装包和签名工具，不写入真实 U 盘。

### 2.2 非目标

- 不关闭 USB 签名校验，不自动重新签名，不把本机公钥或电脑硬件绑定作为更新条件。
- 不把所有 `.msix`、所有未知文件或所有目录从锚点中排除；未知/额外文件仍必须参与完整性校验。
- 不在本任务中部署或承诺外部 relay、GitHub、镜像服务器的可用性；客户端只消费已验证的配置。
- 不改变 Skill Hub 的独立更新授权边界；Skill Hub 不需要 Codex U 盘签名，但仍需显示来源、超时和诊断原因。

## 3. 锚点契约（schema 3）

### 3.1 版本兼容

| schema | 读取/启动 | Codex 本体更新 | 说明 |
|---|---:|---:|---|
| 1 | 仅旧版校验器兼容 | 拒绝并提示 | 历史原始锚点；当前一键包校验器不再接受它 |
| 2 | 允许读取/启动 | 拒绝并提示 | 旧实现会受发布元数据/备份行影响 |
| 3 | 允许 | 允许 | `updateCapability.anchorSchema=3`，算法名 `stable-update-v1` |

这里的 schema 3 是 `updateCapability.anchorSchema` 和许可证 `fingerprint.anchorSchema` 的密码算法版本；根清单和 U 盘清单顶层 `schemaVersion` 仍是 2，不能把文档格式版本误当成锚点版本。拒绝旧 schema 只影响 Codex 本体更新，不影响从 U 盘启动、卸载或独立安装 Skill Hub。重新打包时必须由根/U 盘两侧能力声明、签名工具和许可证共同产出 schema 3；禁止只替换其中一方。

### 3.2 schema 3 的规范化内容

锚点由三部分组成，并使用稳定、排序后的规范文本计算 SHA-256：

1. 根 `manifest.json`：移除现有明确允许变动的 Codex 版本、package 版本、release tag 和 payload SHA/size 字段。
2. 根 `usb-manifest.json`：移除现有明确允许变动的 Codex payload SHA/size 字段。
3. `SHA256SUMS.txt` 的规范行：
   - 当前 manifest 声明的 payload、根 `manifest.json`、根 `usb-manifest.json` 和规范化恢复目录 `payload/_backup/` 使用固定的 `MUTABLE` 哨兵行；
   - 继续拒绝 license、SHA256SUMS 自身和 `.bak` 行；
   - 其余所有行（含未知文件、资源、启动器、配置和其他 MSIX）使用 `IMMUTABLE + sha256 + exact-path` 行照常参与，并按规范行排序；
   - manifest、usb-manifest、payload 行必须存在；备份目录最多一条行，目录匹配必须是 payload 同级的精确 `_backup`，不能用通配规则。

`payload/_backup/` 是事务恢复历史，不是发行包身份；它在锚点投影中使用固定哨兵，但不能由外部元数据任意扩大范围。运行时还要求该目录是普通目录、最多一个直接子文件、文件名符合当前架构的 `OpenAI.Codex_<arch>_<version>.msix` 且非空；嵌套目录、任意二进制和多余文件均拒绝。完整 `SHA256` 校验仍必须覆盖恢复目录和动态元数据，从而保证回滚可验证。

### 3.3 锚点生命周期

更新器必须保存授权前锚点和 license/lease 身份，先计算虚拟提交树的 schema 3 锚点，确认与授权锚点相同后才能进行 rename。提交后重新读取磁盘并执行同一算法；失败时保留事务目录并回滚到旧 payload、旧清单、旧 SHA256SUMS。不得在更新过程中改写 `usb-license.json` 或生成新的授权。

## 4. 更新源契约

Auto 的候选顺序由发布配置的 priority 决定（当前为可用 relay/镜像优先，GitHub 为回退），每个候选都必须完成：解析、版本比较、release identity 校验和可下载性校验。健康但版本 `<= current` 只能记为 `stale/no-newer`，不能终止 Auto 流程。

显式选择 `mirror`、`relay` 或 `github` 时，只使用该来源；来源失败、超时、版本过旧时返回具体错误，不偷偷切换。Auto 找到第一个合法且高于本机的候选即可确认；apply 重新请求同一来源，并要求 tag/release id/package SHA/size 与 check 结果一致。

日志和 UI 至少区分：`source_unreachable`、`source_invalid`、`source_stale`、`source_no_newer`、`source_identity_changed`、`package_download_failed`、`anchor_precommit_failed`、`anchor_postcommit_failed` 和 `rollback_failed`。

## 5. 实施任务与写入边界

| 任务 | 责任 | 唯一写入范围 | 交付 |
|---|---|---|---|
| ANCHOR-001 | 锚点实现 Agent | `crates/codex-usb-license/src/lib.rs` | schema 3 算法、兼容映射、纯单元测试 |
| SOURCE-002 | 更新源 Agent | `src-tauri/src/app/channels/codex.rs` | Auto stale fallback、显式源不越权、来源锁定测试 |
| GUARD-003 | 主控 | `src-tauri/src/app/usb_guard.rs`、更新事务相关调用方 | schema 3 更新门禁、预提交/提交后校验、回滚错误信息 |
| BUILD-004 | 主控 | `tools/`、`src-tauri` 发布元数据/资源校验 | 新包默认 schema 3、签名工具读取同一算法 |
| DOC-005 | 主控 | `docs/` | 契约、测试、经验记录同步 |
| AUDIT-006 | 独立审计 Agent | 只读 | 反例审计、测试复核、发布门禁复核 |

所有 Agent 都必须先声明 `MAD_ROUTE_V1`，不得回滚其他人的修改；主控整合前逐文件检查 diff，禁止跨写入边界扩展。

## 6. 测试与验收矩阵

### 6.1 锚点

- schema 1/2 的历史向量保持原结果。
- schema 3 版本字段、payload SHA/size、release tag 改变时锚点不变。
- manifest/usb-manifest/SHA256SUMS 行同步更新并生成一个备份时锚点不变。
- `payload/_backup/` 之外任意文件增删改、未知文件、资源文件改动都会改变锚点并拒绝提交。
- 路径穿越、重复路径、非法 UTF-8/大小写碰撞、伪造备份目录不能扩大豁免范围。
- license 锚点、公钥摘要、keyId、installationId 或签名正文变化必须拒绝。

### 6.2 更新事务

- 预提交锚点不匹配时，payload、清单、SHA256SUMS 和授权文件均保持原样。
- 正常更新后：新 payload 可验证、元数据一致、完整 SHA256SUMS 通过、schema 3 锚点与旧授权锚点一致。
- 在每个 rename/替换/清理阶段注入失败，均可回滚或留下可恢复事务，不能把半更新报告为成功。
- 启动器对旧 schema、离线、源过旧和身份变化都显示明确动作建议，不出现双击无反应。

### 6.3 来源

- relay 不可达 → mirror/GitHub 可用时 Auto 能继续并选中更新。
- relay 可达但版本落后 → Auto 继续后续候选。
- 所有源均无新版本 → 返回无更新，并列出尝试结果。
- check 后源 identity 改变 → apply 拒绝，不能安装错包。
- 中国大陆网络下 relay/镜像超时 → UI 有超时、来源和日志路径；重试不会残留半下载文件。

### 6.4 发布

- Rust 单测、Clippy `-D warnings`、前端检查和测试、兼容性审计全部通过。
- 资源完整性、schema 3 manifest、签名工具与安装包 profile 公钥/算法一致。
- 新安装包本地验证通过；签名工具对同一新包的签名/验证闭环通过。
- 只生成本地 release 目录和签名工具目录，不向任何物理 U 盘写入。

## 7. 发布前明确限制

本机可以验证算法、事务、静态资源和已配置网络源，但不能替代目标测试机的真实网络环境、Windows 权限、Authenticode、WebView2、U 盘插拔及真实签名后的跨机器启动测试。若任一发布门禁只能“理论通过”，最终结论必须标记为“待目标机实测”，不能写成已完全验收。

## 8. 本轮最终审计与发布证据（2026-09-16）

本轮已完成代码审计、构建和本地发布门禁：

- 安装器构建成功；`CodexAppManager.exe`、`CodexUsbLauncher.exe`、`CodexDesktopSetup.exe` 均写入并验证 `requireAdministrator` 清单。
- U 盘包 `release/usb-oneclick-v0.5.24-g29-final-20260916` 生成成功；内置 Codex payload 深度检查通过，SHA-256 为 `ae01e44b60f3c42fada0cf7deb220104bb8e9111ee79cda16bd57ce7e59c72b0`。
- U 盘包和便携签名器均绑定 profile `usb-v0.5.24-g29-final-20260916-key216.json`：`keyId=prod-2026-01`，公钥摘要为 `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`，签名器配对检查通过。
- 生成的 U 盘包自带 `verify-usb.ps1`，使用 `-InternalSigned -SkipNetwork` 执行“全部验证通过”；签名配对检查确认 portable vault 存在且 `portableVaultBindingVerified=true`。
- 构建期回归：Rust manager 415 passed、license 12 passed、signer 32 passed、前端 419 passed/1 skipped，Clippy `-D warnings`、TypeScript 检查、Lint、Vite 构建、发布契约测试及 PowerShell AST 检查均通过。
- 本轮未写入任何物理 U 盘。最终仍需在目标 Windows 机器完成：签名工具对该包签名、签名后完整校验、Codex 更新实际提交/回滚、Skill Hub 网络源实测及卸载实测。
