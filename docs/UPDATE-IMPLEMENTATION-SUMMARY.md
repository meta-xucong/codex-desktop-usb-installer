# Codex Payload 更新通道 — 实现总结与验收交接书

日期：2026-09-08（2026-09-09 复核补充）
实现者：Amadeus（multi-agent-dev 流程：实现 → 独立审计 → 修复 → 复审 → 全量测试）
状态：**已通过两轮独立审计与全量测试验收**，本文档供第三方（Codex）再审计复核。

> 复核补充（2026-09-09）：本文主体记录 2026-09-08 的 GitHub 直连 payload 通道，后续的
> Relay/GitHub 自动回退、VPS 静态中继、独立 HTTPS edge 和客户端同源校验以
> [`UPDATE-RELAY-DESIGN.md`](D:/AI/AGENT_U盘/codex-desktop-usb-installer/docs/UPDATE-RELAY-DESIGN.md)
> 为准。当前代码阶段已完成最终回归：`codex-win-engine` 全量 175 项通过、0 失败、1 项忽略；
> 前端 Vitest 34 个文件、381 项通过；Relay/GitHub 客户端 29 项、下载层 7 项、settings_store
> 10 项通过。以下第 7 节中的失败记录是 2026-09-08 的历史环境记录，不代表当前最终测试结果。

## 0. 项目位置（务必先看，防止找错目录）

**项目根目录（本文所有相对路径均以此为基准）：**

```text
D:\AI\AGENT_U盘\codex-desktop-usb-installer
```

⚠️ 注意：
- 项目在 `D:\AI\AGENT_U盘\` 下，**不是** WorkBuddy 工作目录 `C:\Users\T14S\WorkBuddy\2026-09-07-20-47-51`。
- Rust 代码在 `<根>/src-tauri/src/`，前端在 `<根>/src/`，文档在 `<根>/docs/`，任务记录在 `<根>/.workbuddy/`。
- 路径含中文「AGENT_U盘」，cargo 增量编译偶发失败时用 `CARGO_INCREMENTAL=0`（见 §7.3）。

---

## 1. 任务目标

为 codex-desktop-usb-installer（Tauri 2 + React 19 + Rust，v0.5.24）实现「Codex payload 更新通道」：

- 检测 U 盘上 Codex MSIX 是否有新版本（上游：`wangzhen3691/codex-app` → 用户 fork `meta-xucong/codex-app-mirror`）
- 用户手动确认后刷新 U 盘 payload（绝不自动更新、绝不动本机已部署的 Codex）
- 源可配置（用户级 `codex_payload_repo` + 维护者级 `DEFAULT_CODEX_PAYLOAD_REPO` 常量），为上游跑路留后路

## 2. 依据文档（实现范围已冻结）

均位于 `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\`：

| 绝对路径 | 作用 |
|---|---|
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-ARCHITECTURE.md` | 总架构、enum dispatch、两阶段部署语义、安全不变量 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-CHANNELS.md` | codex 通道完整契约（源解析/版本比较/payload identity/apply 步骤/错误码） |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-WORKFLOWS.md` | 运维 SOP（非代码范围） |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-TESTING.md` | 测试矩阵 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-AUDIT.md` | 文档审计轨迹（两轮，findings 1-12） |

## 3. 硬约束（C1-C9，审计逐条核验）

- **C1** 手动确认：check 无副作用，绝不自动 apply
- **C2** 零新增依赖：Cargo.toml 不加 crate（不用 async-trait/tempfile）
- **C3** settings 五触点：RawAppSettings / AppSettings / Default / into_settings / normalize 全链路支持 `codex_payload_repo`
- **C4** 版本来源：U 盘根 `manifest.json` 的 `codexDesktopVersion`；`payload/version.txt` 仅可选诊断缓存；apply 后原子更新根 manifest 四字段
- **C5** 不信任前端回传：apply 重新解析 repo + 重新拉 GitHub Release API + 用新 `browser_download_url`；release_id/asset_id/asset_name/package_moniker/latest_version 任何不一致 → `StaleExpectation`
- **C6** 校验链：SHA256（SHA256SUMS-windows.txt，按 package_moniker 匹配）→ GitHub digest（advisory）→ Authenticode（`verify_openai_authenticode` + `is_valid_openai`）→ MSIX identity（`validate_codex_identity`）
- **C7** 两阶段分离：本通道只刷 U 盘；「更新此电脑上的 Codex」是未来独立动作，不在本实现内
- **C8** `usb_guard::require_authorized()` 在 apply 第一行
- **C9** 范围：只做 codex 通道；不改 `UpdateSource::Custom` 既有语义

## 4. 改动清单（最终版）

以下路径均为相对 §0 项目根 `D:\AI\AGENT_U盘\codex-desktop-usb-installer\` 的路径。

### 新增
| 绝对路径 | 内容 |
|---|---|
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\update_channel.rs` | ChannelKind（kebab-case serde）/ UpdateInfo / UpdateContext / compare_versions（分段数值比较，解析失败回退字符串不等）/ read_usb_codex_desktop_version；5 单测 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\channels\mod.rs` | 通道模块声明 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\channels\codex.rs` | 完整通道实现（见 §5）；5 单测 |

### 修改
| 绝对路径 | 内容 |
|---|---|
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\settings_store.rs` | `codex_payload_repo` 五触点 + 3 单测 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\mod.rs` | 声明 `channels` / `update_channel` |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\commands.rs` | `payload_check_codex`（只读）、`payload_apply_codex`（两阶段 apply + 进度/done 事件） |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\lib.rs` | 命令注册 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src\shared\types.ts` | `AppSettings.codexPayloadRepo` + DEFAULT_SETTINGS |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src\services\managerApi.ts` | UpdateInfo / CodexPayloadCheck 类型 + checkCodexPayloadUpdate / applyCodexPayloadUpdate + normalizeSettings 同步 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src\app\views\About.tsx` | 「检查 Codex 安装包更新」入口 + 确认 Sheet + 应用流程 |
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src\app\i18n.tsx` | 6 新键 × zh/en（codexFound/codexUpToDate/codexUnavailable/codexConfirmTitle/codexConfirmBody/codexApplied） |

### 文档同步
| 绝对路径 | 内容 |
|---|---|
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-CHANNELS.md` | staging 文件名改为 `codex-<msix_version>.Msix`、事件字段 camelCase、step10 失败保留备份的语义说明 |

## 5. 关键实现决策（Codex 审计时请重点理解）

1. **网络层不用 reqwest async**：manager crate 的 reqwest 0.13 关闭了默认特性、无 async runtime。复用 `codex_win_engine::fetch_text_with_network`（curl 同步）+ `download_to_with_progress_bounded_with_network`，代理/超时与产品其他部分一致。
2. **`MAX_PACKAGE_BYTES` 引用路径**是 `codex_win_engine::limits::MAX_PACKAGE_BYTES`（不在 crate root re-export）。
3. **GitHub API 无 token**（公开 release），Accept 头 + UA 必带；资产 URL 一律取 `browser_download_url`，绝不手拼。
4. **MSIX 扩展名大小写不敏感匹配**（`.Msix` vs `.msix`），staging 本地文件名用 `codex-<msix_version>.Msix`。
5. **appVersion ≠ MSIX version**：比较用 `appVersion`（如 26.825.51511），MSIX 文件名内嵌的是 package version（如 26.825.6671.0），两者故意不同。
6. **serde enum tag 冲突**：`#[serde(tag="channel")]` 与变体字段 `channel` 同名会编译错误，ChannelCheckOutcome 用外部位移命名（无 tag）。
7. **tempfile 不在依赖里**：测试用自建 `unique_test_dir` + `TempDirGuard`（进程计数器 + 时间戳）。
8. **备份策略**：`_backup/` 保留 1 份；MSIX 替换失败自动恢复备份；manifest 写失败保留备份（旧包可手工恢复）。
9. **done 事件只在成功路径 emit**（`??` 提前返回失败）。

## 6. 审计与测试证据（2026-09-08 原始实现记录）

### 审计
- **首轮独立审计**（Explore 只读 Agent，对照 C1-C9 逐条）：全部「符合」，另发现 4 项非阻塞缺陷
- **修复**：① `update_root_manifest` 补写 `payload.size`；② apply 成功后 emit `phase:"done"` 事件；③ staging 文件名按 msix_version；④ 遗留注释清理
- **复审**（第二个独立 Agent）：4 项修复落地（codex.rs:516-522/536-539、commands.rs:567-573、codex.rs:371）、未破坏 C1-C9、前后端一致（命令名/事件名/字段名核对通过）

### 测试（原始实现相关，全绿）
| 测试 | 结果 |
|---|---|
| `cargo test --lib app::channels` | 5/5 |
| `cargo test --lib app::update_channel` | 5/5 |
| `cargo test --lib app::settings_store` | 9/9 |
| `npx tsc --noEmit` | 通过 |
| `cargo clippy --lib` | 本次文件零告警（仅 2 条既有 mac-engine dead_code） |
| vitest `src/` 29 个测试文件 | 全绿（Home/Settings/OfflineHome/InstallOtherVersion 等均含） |

## 7. 基线环境事实（2026-09-08 历史记录，重要！避免误判）

以下是首次交接时记录的**基线环境问题**，不是当前最终回归结论（当时本次改动文件
`git diff --stat -- scripts/ cloudflare/` 为空；失败用例单测 `-t` 过滤单跑通过）：

1. **cargo test --lib 全量有 27 个既有失败**：`usb_media_cleanup` / `uninstall_cleanup` / `mac_update`。根因是 Windows 下中文路径 + 只读文件删除权限（os error 5「拒绝访问」），在 USB 测试目录中复现。
2. **vitest 的 `scripts/mirror-release.test.mjs`（38 fail）与 `scripts/release-workflow.test.mjs`（2 fail）**：本机无 `aws` CLI、无 `pwsh`，`runProcess` spawn 失败/超时（5s/10s/15s/60s 超时模式）；且测试大量使用真实 git/pwsh 子进程。
3. **cargo 增量编译在中文路径下偶发 rmeta 复制失败**（os error 5）：`CARGO_INCREMENTAL=0` 可绕过。
4. **仓库无初始提交**（`git log` 报 "does not have any commits yet"，全部文件处于 `A` 暂存态），`git stash` 不可用；本次用 `/tmp/impl-backup` 做了实现文件备份。

## 8. 给 Codex 的验收清单

建议按此顺序复核（均为只读）。**所有命令都必须在项目根目录执行：**

```bash
cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"
```

1. **范围圈定**：`git status --porcelain` 排除 `A`（基线）后，本次改动应恰好是 §4 清单 + `?? src-tauri/src/app/channels/`、`?? src-tauri/src/app/update_channel.rs`、`?? docs/UPDATE-*`、`?? .workbuddy/`、`?? codex-learnings/`
2. **跑本次相关测试**：
   ```bash
   cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri"
   CARGO_INCREMENTAL=0 cargo test --lib app::channels
   CARGO_INCREMENTAL=0 cargo test --lib app::update_channel
   CARGO_INCREMENTAL=0 cargo test --lib app::settings_store
   cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"
   npx tsc --noEmit
   ```
   预期全绿。
3. **逐条核对 C1-C9**（§3），重点：
   - `src-tauri\src\app\channels\codex.rs` apply 的重校验顺序（§5.5/5.8）
   - `src-tauri\src\commands.rs` payload_apply_codex 失败路径不 emit done
   - `src-tauri\src\app\settings_store.rs` 五触点
4. **对照文档**：`docs\UPDATE-CHANNELS.md` § Apply semantics 与 `src-tauri\src\app\channels\codex.rs` 实现逐步比对
5. **回归确认**：`npm test` 中 src/ 全绿；scripts/ 失败按 §7 归因，不要算到本次头上

## 9. 遗留事项（不属于本次验收范围）

1. 真实 U 盘 E2E（需要物理设备）
2. fork 仓库部署 `sync-upstream.yml`（meta-xucong/codex-app-mirror 操作）
3. 进度事件前端消费（事件已 emit，UI 暂用文案状态）
4. 建议尽快做初始 commit 固化基线（当前仓库零提交）
5. Authenticode 校验依赖 PowerShell 子进程（复用 win-engine 既有实现，文档已标记为已知局限）

## 10. 关键文件入口（绝对路径）

| 角色 | 绝对路径 |
|---|---|
| 实现核心（约 700 行，含 5 单测） | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\channels\codex.rs` |
| 抽象层 | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\update_channel.rs` |
| 命令桥（搜 `payload_check_codex`） | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\commands.rs` |
| 前端入口（搜 `checkCodexPayload`） | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src\app\views\About.tsx` |
| 设置存储（搜 `codex_payload_repo`） | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\src-tauri\src\app\settings_store.rs` |
| 任务全程记录 | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\.workbuddy\PROGRESS.md` |
| 本文（交接书） | `D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\UPDATE-IMPLEMENTATION-SUMMARY.md` |

## 11. 2026-09-09 公网 Relay 与证书收尾补充

- `update.404token.xyz` 已解析到 `141.11.138.152`；独立 Nginx edge 已使用 Let’s Encrypt TLS 证书在 443 提供 Relay。
- 证书通过固定版本的 acme.sh 使用 TLS-ALPN-01 自动签发；未登记通知邮箱，不需要用户注册账户，但也没有到期邮件提醒。
- 证书续期脚本和 systemd timer 已部署并实际执行成功；acme.sh 在未到续期窗口时返回码 2，脚本已将其作为“无需续期”成功处理。
- 曾发现并修复 edge 配置目录挂载路径不一致：Compose 现在挂载 `/opt/codex-update-relay-edge/conf.d`，与准备脚本写入位置一致。
- 公网矩阵通过：metadata 200、无斜杠入口 308、根路径和 release 内部路径 404、POST 403、MSIX Range 206；心理测试站 80 端口和容器保持不变。
- 尚未完成的只有真实中国大陆测试机插入 U 盘后的 check/apply、真实安装/回滚和不同运营商网络验证；正式 Authenticode 发布证书仍需外部 CA/签名服务或组织账户，不能由当前环境凭空取得。
