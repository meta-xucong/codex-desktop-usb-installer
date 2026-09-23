# 更新源与 Skill Hub 网络韧性修复（G33）

状态：代码、真实 Relay 原生探针、定向/全量回归和正式构建已完成；已生成新的安装器与便携签名器；未写入物理 U 盘、未推送 GitHub。

## 1. 目标与范围

修复两个现场问题：

1. Codex 已经是镜像源当前版本时，因备用 Relay/GitHub 不可用而被误报为“更新源不可用”；
2. Skill Hub 检查在中国大陆网络或应用自定义代理环境下无法沿用有效网络配置，最终只显示前端 watchdog 超时。

本轮修改 Codex payload 来源解析、Skill Hub 网络客户端与 frontmatter 兼容、相关回归测试和本记录；构建了新的发行输出。Codex/U 盘签名根、payload 事务、VPS 远端文件和安装/卸载范围不变。

## 2. 已核实根因

### 2.1 Codex Relay 协议漂移

生产 `latest.json` 已发布 `protocol=2`，并在 `identity` 节点重复 release、tag 和资产身份。客户端此前只接受 `protocol=1`，因此稳定产生 `source=relay reason=source_invalid`。现有单测只覆盖协议 1，未覆盖生产协议 2。

### 2.2 Auto 聚合错误

Auto 顺序仍固定为镜像 → Relay → GitHub。镜像成功解析但版本相同/更旧时，代码会继续探测备用源；只要任一备用源失败，旧逻辑就返回整体错误。这样把“有可信源证明当前无更新”误报成“全部源不可用”。

### 2.3 Skill Hub 网络配置断链

Skill Hub 使用独立 reqwest client。此前它只依赖 reqwest 默认系统代理，不读取管理器已保存的 `proxy_mode/custom_proxy_url`；当用户只配置应用级自定义代理时，中转和 GitHub 都可能直连失败。更直接的阻塞点是依赖树因 Tauri updater 启用了 `reqwest` 的 `rustls-no-provider`，但 Skill Hub 创建 client 前没有安装 Rustls crypto provider，`Client::build()` 会 panic；前端因此只能等 watchdog，无法得到后端错误。前端 60 秒 watchdog 只负责退出等待，不会修复 native 请求链路。

### 2.4 Skill Hub 内容格式契约漂移

真实 Relay 归档的 `SKILL.md` 使用合法的 YAML 多行描述（`description: >-` 加缩进行）。旧解析器把缩进行当成新的 frontmatter 键值行，返回“frontmatter 行格式无效”，因此即使网络、大小和 SHA-256 全部通过，安装仍会失败。

修复为受限的确定性解析：仅允许已出现的 `description` 键使用 `> / >- / >+ / | / |- / |+` 块标量，并只接受空格缩进的连续内容；不执行通用 YAML，不放宽路径、文件类型、大小、归档前缀或哈希检查。

## 3. 冻结修复契约

### C1：Relay 协议

- 保留协议 1 兼容能力；
- 接受协议 2，但必须存在 `identity`；
- 协议 2 的 `identity.releaseId`、`identity.tag` 和全部资产的 path、size、sha256、githubDigest、id 必须与顶层一致；
- 未知协议、缺失 identity 或身份不一致必须 fail closed，不能下载资产。

### C2：Codex Auto 结果

- 任一源成功解析出合法 release，即形成 `valid_source` 证据；
- 合法 release 均没有新版本时，即使后续备用源失败，也返回无更新；
- 只有没有任何合法源且至少一个源失败时，才返回“更新源不可用”；
- 显式选择单一源的行为不变，仍对过期/无更新返回明确错误。

### C3：Skill Hub 网络

- `system`：使用 reqwest 的系统代理能力；
- `direct`：明确禁用代理；
- `custom`：复用设置中已通过 URL guard 的代理 URL，支持 http/https/socks5/socks5h，不把代理凭据写入日志或错误详情；
- 在构建 client 前安装应用内置的 Rustls `ring` provider；并发初始化时允许已安装的 provider 获胜，初始化失败返回可展示的 `SKILL_REPO_UNAVAILABLE`，禁止 panic；
- check/apply 均在创建 client 前读取当前持久化设置，并记录脱敏的 proxy mode；
- 每个请求继续受阶段 deadline、总 deadline 和响应大小限制约束；
- 网络传输/响应读取错误返回稳定阶段和 source id，便于诊断，不回显 URL、凭据或机器路径。
- `SKILL.md` 支持 Skill Hub 当前使用的受限多行 description frontmatter；非法缩进、空块、重复键和其他危险内容仍 fail closed。

### C4：前端等待边界

Rust Skill Hub 总 deadline 为 45 秒，加入 2 秒 join grace。renderer watchdog 调整为 50 秒，使当前后端的明确错误先于 UI watchdog 到达；watchdog 仍保留用于旧/异常 native runtime，不能作为安全或并发控制。

### C5：G39.5-r7 大文件更新可见进度

G39.5-r7 现场补充显示：Relay 元数据解析成功后，约 824 MB 的 MSIX 在关闭 360 的环境中下载到 3,272,704 字节后停滞。该问题不归因于 360，而归入大文件下载恢复策略和用户可见状态。

后续实现必须把下载进度和安装阶段进度分成两条独立进度条：下载按真实字节显示，安装按 `verifying/applying/committing/finishing` 阶段显示；下载未完成或安装未收尾时不得报告更新成功。完整契约见 [`G39.5-R7-FIELD-ISSUES-REPAIR-DEVELOPMENT.md`](G39.5-R7-FIELD-ISSUES-REPAIR-DEVELOPMENT.md) 第 4 章。

## 4. 允许修改的文件

- `src-tauri/src/app/channels/codex.rs`
- `src-tauri/src/app/skill_repo_update.rs`
- `src/app/views/SkillRepoUpdateCard.tsx`
- 对应 Rust/React 测试、更新源契约文档和本记录

不得借本轮修改签名根、USB anchor、Skill Hub 来源身份、仓库内容、VPS 服务或打包发布物。

## 5. 验收矩阵

| ID | 场景 | 预期 |
| --- | --- | --- |
| G33-R1 | 生产格式 protocol 2 + matching identity | Relay descriptor 可解析并继续走既有 manifest/checksum/asset 校验 |
| G33-R2 | protocol 2 缺 identity/identity 不一致 | source invalid，未派生/下载资产 |
| G33-R3 | 镜像合法且 no-newer，Relay/GitHub 失败 | Codex Auto 返回 `Ok(None)`，日志结果为 `no-update` |
| G33-R4 | 所有 Auto 源失败 | 仍返回 aggregate source-unavailable 错误 |
| G33-R5 | Skill Hub system/direct/custom 三种代理模式 | client 创建策略与管理器设置一致；custom URL 不泄露 |
| G33-R5a | Rustls no-provider 依赖组合 | client 构建前安装 `ring` provider；不 panic，返回明确错误 |
| G33-R6 | Skill Hub 请求/响应超时或传输失败 | 在 native deadline 内返回稳定错误与 source/phase attempts，不无限等待 |
| G33-R7 | renderer watchdog | 当前后端约 47 秒内的错误先被呈现；异常旧 runtime 最迟 50 秒恢复可重试 |
| G33-R8 | 既有安全回归 | 不改变 Skill Hub archive 校验、Codex payload 事务、USB 授权和显式源语义 |
| G33-R9 | 当前生产 Relay 原生往返 | 使用当前源码 client 实际获取 metadata、archive、复核 metadata，并成功解析至少一个 Skill |

## 6. 测试与审计记录

实现后必须执行：

```text
cargo fmt --check
cargo test --manifest-path src-tauri/Cargo.toml app::channels::codex -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml app::skill_repo_update -- --nocapture
npx tsc --noEmit
npx vitest run src/app/views/SkillRepoUpdateCard.test.tsx
```

还要执行一次不改文件的公网探测，确认当前 Relay `latest.json` 仍是生产格式并记录 HTTP 状态；本次进一步运行了当前 Rust client 的真实 metadata/archive/复核/解析探针。真实中国大陆网络、真实自定义代理、真实已签名 U 盘仍属于现场验收，不能由开发机网络代替。

最终审计必须逐项检查 C1-C4 和 G33-R1～R9，区分“代码/单测/开发机现场探针已证明”和“目标机仍需实测”。本次审计通过后才执行正式构建；构建产物仍不能替代目标机现场验收。

## 7. 变更 receipt

| 字段 | 值 |
| --- | --- |
| 起始基线 | 工作区已有未提交改动；本轮不覆盖、不回滚 |
| 实际代码变更 | Relay protocol 2 身份校验；Auto 来源结果聚合；Skill Hub proxy 继承、TLS provider 初始化、阶段错误详情；受限多行 frontmatter；renderer watchdog 50 秒；回归测试 |
| 测试结果 | Rust 全量：421 passed, 0 failed, 3 ignored；Skill Hub 定向：21 passed；Codex 定向：42 passed；签名器：32 passed, 0 failed；`npm test`：421 passed, 1 skipped；`npm run check`、`npm run lint`、`npm run build`、Rust fmt：通过；当前 Rust client 真实 Relay metadata/archive/复核/解析通过；Relay/Skill Hub 公网 HTTP 200 且归档 size/SHA-256 匹配 |
| 正式构建 | 安装器：`release/usb-oneclick-v0.5.24-g33-skill-hub-frontmatter-20260917`；签名器：`release/usb-license-signer-gui-v0.5.24-g33-skill-hub-frontmatter-20260917-portable`；包内 `verify-usb.ps1`、入口清单、包—签名器—portable vault 一致性验收通过 |
| 只读审计结论 | 符合（源码、开发机真实 Relay、构建产物和签名器配对）；真实中国大陆网络、真实 custom proxy、真实目标机 native runtime 和物理 U 盘仍需现场验收 |
| 未验证边界 | 目标机大陆网络/代理、目标机 WebView2/VC++ 安装、真实 U 盘写入签名及跨电脑输入密码的实际签名事务 |
