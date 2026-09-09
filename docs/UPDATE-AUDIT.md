# Update Channels — Implementation Audit Record

状态：实现已落地，主控静态复核完成；Bernoulli 已对 r6 做出独立只读审计并
发现并推动修复 runtime wrapper 与源包缓存污染问题，r8 的再次独立复审在本轮
未返回可交付结论，因此仍不能称为 release approved。本文不替代命令输出，也不
把缺少真实 USB、arm64 fixture 或完整下载/apply E2E 的项目标为通过。

## Scope

本轮按 docs/UPDATE-REMEDIATION-DEVELOPMENT.md 的基础契约
CONTRACT_REV=v2.0-remediation-r1，并按
docs/UPDATE-CODE-CLOSURE-DEVELOPMENT.md 的增量契约
CONTRACT_REV=v2.2-engineering-gate-r1 实施，重点包括：

- v2 canonical USB anchor 与 v1/unknown fail-closed；
- authorized bundle root 路径接线；
- GitHub API headers 与完整 stale identity；
- staged MSIX 校验、真实本地大小和 backup；
- root manifest、usb-manifest、SHA256SUMS 的可恢复事务；
- update lock、reparse/path boundary、Settings/About 门控；
- x64/arm64 派生 verifier 与残留文件策略。
- 内部 mock transport、完整 apply 前置校验链和离线失败回滚证据。
- workspace fmt/clippy 工程门禁收口、可移植能力包路径和不分配 8 GiB 的逻辑大小超限测试。

## Current evidence

已执行：

- Rust app::channels::codex：24 passed；新增 mock transport apply happy path、
  下载/SHA/digest/Authenticode/identity 失败回滚和远端 manifest/asset 拒绝矩阵；
- Rust app::channels：24 passed；
- Rust app::update_channel：5 passed；
- Rust app::usb_guard：15 passed；
- Rust app::usb_media_cleanup：17 passed、1 ignored；
- TypeScript：通过；
- Vitest：30 files、308 tests passed。
- 完整 `npm test`：33 files、373 tests passed。
- ESLint：`npm run lint` 通过；生产 renderer 构建通过，`renderer entry
  isolation verified`；Vite 的 chunk size 仅为非阻断性能提示。
- release tests：2 files、56 tests passed；Cloudflare download router：1 file、
  9 tests passed。
- offline bundle simulation：`npm run test:offline-bundle` 通过，输出 277 个
  文件、276 个 SHA256 清单项，合成 MSIX 的布局/哈希/WindowsApps/WpSystem
  禁建约束均通过。
- workspace `cargo fmt --check`：通过；未豁免的
  `cargo clippy --lib -- -D warnings`：通过；完整 `cargo test --lib`：342
  passed、0 failed、1 ignored。ignored 是既有只读本地 release hash 审计，
  不属于本轮失败。
- PowerShell AST：5/5 通过；历史 v1 fixture 按预期以 schemaVersion=2 错误拒绝。
- 使用历史 x64 payload、重新构建的 `release/setup` 资产和完整能力包生成
  `release/usb-oneclick-v2-fixture-r8`；Windows PowerShell 5.1 下构建成功，
  `verify-oneclick-usb.ps1 -SkipSignature -SkipNetwork` 通过。MSIX 的
  Authenticode 主体为锁定的 OpenAI Marketplace 证书，VC++/WebView2 的
  Authenticode 主体为 Microsoft；默认严格 verifier 按预期拒绝未签名的
  Launcher/Manager。
- 首次构建发现 `tools/build-usb-layout.ps1` 不能向缺失属性的
  `PSCustomObject` 直接赋值 `schemaVersion/updateCapability`；已改为
  `Add-Member -Force`，并用新输出目录重跑构建和 verifier 通过。
- 严格构建模式同样在创建输出目录前拒绝 `CodexUsbLauncher.exe` 的
  `NotSigned` 状态，避免产生未签名发布半成品。
- 主控静态复核补强了 backup 精确恢复、reparse ancestor/path 防护、v2
  payload 元数据一致性门禁、清单遍历 reparse 拒绝，以及构建脚本对可变
  license 的清单排除；受影响 Codex channel 测试已重跑通过，并覆盖 committed
  事务成功复核和不一致回滚。
- 已将默认 Codex 源统一到现有产品链路和当前发布使用的
  `Wangnov/codex-app-mirror`，避免清空 Settings 后回落到历史仓库。
- 复核发现 r3 fixture 未包含新增能力包目录契约，属于过期生成物；未修改生产
  verifier，而是按当前构建脚本生成 r8 并重新通过离线 verifier。r3/r4/r5/r6/r7 不再
  作为当前通过证据。
- 能力包一键调用复核发现管理器曾把内部安装开关直接传给外层
  `install-all.ps1`；已删除这三个越界参数，改由外层包装脚本按其公开契约转发到
  `install-to-codex.ps1`，并同步更新能力包安装文档。能力包源与 r8 fixture 的
  schema/version、50 个 Skills、7 个 Agents、249 条 file-manifest 和 251 个实际
  文件再次对齐。
- 独立 runtime 复核发现 `provision-runtime.ps1` 用 `\"` 生成 cmd wrapper，导致
  已安装的四个 runtime 入口只剩占位内容；已改为 PowerShell backtick quoting，
  同步更新能力包内部两份哈希并重新构建 r8。源包与 r8 的 manifest/checksum
  逐文件复核均为 0 个不匹配，四个 wrapper 的 AST 参数结构和引号均通过检查。
- 由于 runtime 修复改变了能力包行为，包版本已从 1.2.0 升至 1.2.1，并同步
  更新 `pack.json`、`dependencies.json`、`file-manifest.json` 及其哈希；这样旧
  readiness 不会把修复前的 1.2.0 误判为当前能力包。
- Bernoulli 的独立只读 r6 审计确认参数、Skills/Agents/MCP/runtime 安装选择和
  r6 静态清单闭合，并发现上述两个缺陷；修复后由主控对 r8 重新做了同等检查。
- `git diff --check` 与 `git diff --cached --check` 均因仓库缺少可读取的初始
  commit 对象而失败（fatal unable to read object）；这属于当前 Git 基线限制，
  不能作为代码通过证据。

仍缺少或仅能在外部 Windows 条件下确认：

- arm64 v2 fixture 的 verifier 通过；
- 真实 Windows 的 apply happy-path / 故障注入 retry、真实 GitHub 网络/代理和
  真实 Authenticode；当前 mock transport 覆盖的是生产 apply 前置校验与提交同一
  条代码路径，但不等价于真实网络、真实签名和实体 USB E2E；
- 真实可移动 USB、授权 license/lease 下的 R1-R11；
- r8 最终候选的再次独立只读复审未在有界等待内返回可交付结论；Bernoulli 的
  r6 独立审计结论已返回并作为修复依据。

当前 fixture 只证明 x64 的构建/静态 verifier 链路；它不证明物理 USB 上的
guard、lease、拔盘恢复或 destructive apply。Launcher/Manager 本身当前为
未签名 PE，因此严格 verifier 和非跳过签名构建会拒绝它；取得发布证书并签名
后，必须重跑同一门禁。CCSwitch 当前仍按现有策略只做哈希校验，签名属于后续
独立发布策略，不能把它混入本轮 MSIX/Launcher/Manager 门禁结论。

历史 release/usb-oneclick-v0.5.24-final 为 schemaVersion=1，不能作为本轮
v2 通过证据。

## Audit handoff

| 字段 | 值 |
| --- | --- |
| Contract | v2.2-engineering-gate-r1（基于 v2.0-remediation-r1，含 v2.1 code closure） |
| Writer | Codex 主控；补强后重新冻结 |
| Audit owner | Bernoulli（r6 独立审计）；r8 再审未返回可交付结论 |
| Conclusion | r8 代码/静态包主控复核通过；整体发布证据不足，因缺少 r8 独立结论与真实 E2E |
| Final hashes | 由本次最终交接消息绑定；本文不自引用自身哈希 |

独立审计若恢复并发现问题，必须按“问题编号 → 修正/补证 → 新版本 →
受影响测试 → 再审计”回流；在取得独立结论和真实 E2E 证据前不得称为
release approved。

## Frozen artifact hashes

以下是本次交接绑定的关键实现/契约文件 SHA-256；本文自身不自引用哈希。

```text
cb6ce84e7a2af1f5154e7605fa50fe7d32ef5dc5fbac6f3f2150d9fff6626300  src-tauri/src/app/channels/codex.rs
b55c1c77d90fd4e326497f23aae278c334ccb20bdbf462cdd03fdfb9f2ab32dd  src-tauri/src/app/channels/mod.rs
26b7e4bf4074affab2cbf889a59bd739c8c18e7f1524c3089c5cde20f562b5d1  src-tauri/src/app/update_channel.rs
f5b04959275bb1fd0157d864cb76cdde0e076db06e1e266c6d7cfa32bc7eef1b  src-tauri/src/app/usb_guard.rs
36b1bf5c37d818151a699f5dbab4d18f13563beae944f978815a0004edc58411  src-tauri/src/app/usb_media_cleanup.rs
f7d41948e1ba228a31138e618a33e0221c1ea1610cd180a82af90e1a24b54f2c  src-tauri/src/commands.rs
d0263eee20ce8228c94197e1551fc6eb74222f0088c9e2220e6cdedfd9806298  crates/codex-win-engine/src/sys.rs
ee9318c5cdb4aa07f425bfcb569769d07f909c5a7681a4c2bab8f68c4ee08720  crates/codex-mac-engine/src/sys.rs
8d12fac57d75a4fef465284c3720427f37c70d22058ffb0f5c35c0238602a6f8  src-tauri/src/app/capability_pack.rs
d4aa8b2965439f4022055a08e945f4153f2ae0b88dfc6219b4ff03d9bdc61589  src-tauri/src/app/mac_update.rs
2baac4ff7a2b359d419a663d4ad7b6027766d1e9a8f69cd0cb13c1cd12a6f30e  src/app/views/Settings.tsx
f4baab3bf3b50dddd0ef5e606cd66abfa5af7a06cca9513c2157bfff81db6afd  src/app/views/About.tsx
2d862a068380a7b5017a0e11985469b45057806eaa1dca529242149a920c187c  src/services/managerApi.ts
ca241103578c795dd6b543df2cfee710a2524e538b0cbdd96003c38b0795eb9e  eslint.config.js
833c2d4e63466d2c4250537cf40c0bfb708723cbe23f287109135301d317870a  vite.config.ts
ab50e18fe4a28a4dde66d9f1877f9965ea155346755c99ff5de88d5fa123f2f6  tools/build-usb-layout.ps1
2965dde2f7adbad589483b7ba39a3527364acb7bd503a2ea3f0ff43525f31531  tools/build-oneclick-usb-layout.ps1
f06667109e9d4f06d6ccdef65c124c058b04ea8fd62b5ea4341efb101ab0d913  tools/verify-payload.ps1
d973ecef2bf2f836ff70749d28b819e38e0bc376c023f1b172cbc62904692e87  tools/verify-oneclick-usb.ps1
9635f1d39273afc6b1d2ea48cf4149f73fdc3c65208b064b3fc0f60c8c7f0d85  tools/update-from-github-fork.ps1
34b4e66c95847f8298dfa65ac74dee9b4d4a00c7a6d2ee6969d195744894761d  docs/UPDATE-REMEDIATION-DEVELOPMENT.md
b287daf49eacad543f6d377250b621f25f35d153d4774c69815b549d4d40f8e4  docs/UPDATE-CHANNELS.md
5d22f227358a615fd1f83a07ce289eb988fcd43312749ed0195e9e2930cb5ef5  docs/UPDATE-TESTING.md
d9633252d1fd1266268a8a5e029bf1e3a02cc2a5f76a0139d3643d0f943c272f  docs/UPDATE-CODE-CLOSURE-DEVELOPMENT.md
28399791f0b1ec70b271edd6c18234101f6bb77364f18f9ee12b71d32790957c  docs/UPDATE-ARCHITECTURE.md
962a78631d6ea0c70b614bebbde4d05efbf0989132390ea11125266519deeefc  docs/UPDATE-WORKFLOWS.md
```
