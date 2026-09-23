# Update Testing — schema 3 Codex payload capability

状态：**实现后测试记录**。最终是否可发布以实际命令输出和独立审计为准；
没有物理 Windows USB、arm64 v2 fixture 和真实发布下载链路时，不得宣称完整
E2E 通过。

## 2026-09-09 最终代码阶段复核

以下结果是 2026-09-09 的历史 Relay/GitHub 自动回退验收记录。当前 Codex payload
自动源契约为“国内镜像 → 内置兼容 Relay（或用户配置的 Relay）→ GitHub”；实现与
新增契约见 `docs/UPDATE-FAILURE-G19-DEVELOPMENT.md` 和 `docs/UPDATE-CHANNELS.md`。
内置 Relay 是镜像不可用时的真实默认回退，不需要用户先手工填写 URL。

以下结果覆盖当时 Relay/GitHub 自动回退实现及其安全边界：

- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --no-fail-fast`：175 项通过、0 失败、1 项忽略。
- `cargo test --manifest-path src-tauri/Cargo.toml --no-fail-fast`：348 项通过、0 失败、1 项忽略。
- `cargo test --manifest-path src-tauri/Cargo.toml app::channels::codex --no-fail-fast`：29 项通过。
- `cargo test --manifest-path src-tauri/Cargo.toml app::settings_store --no-fail-fast`：10 项通过。
- Relay 无重定向下载层：7 项通过；`cargo check --all-targets`、Rust 格式检查通过。
- `npm run check`、`npm run lint`、`npm test`：34 个文件、381 项通过。

代码阶段仍不能替代真实 Windows 可移动 U 盘、真实签名 MSIX 和真实中国大陆网络下的
check/apply 验收；这些边界见“Windows人工E2E”。Relay 的 VPS 同步、容量拒绝、TLS、
公网 HTTPS 路由和证书续期证据见 `docs/UPDATE-RELAY-DESIGN.md`。

## 2026-09-09 Relay 公网边缘验收

从 VPS 直连和本机绕过代理直连均验证通过：

- `https://update.404token.xyz/__codex_update__/latest.json` 返回 200。
- 无斜杠入口返回 308；根路径和 `/releases/...` 均返回 404；POST 返回 403。
- MSIX Range 请求返回 206，`Content-Type: application/octet-stream`，长度与 metadata 一致。
- 响应包含 HSTS、`nosniff`、`no-store`；TLS 证书有效期为 2026-09-09 至 2026-12-08。
- 独立 edge 容器的 `nginx -t`、证书续期服务（未到续期窗口时正确处理 acme.sh 返回码 2）和 timer 均通过。
- 现有心理测试站 80 端口、根路径响应和 Web/App/DB 容器未受影响。

## 自动测试

必须执行：

    cd src-tauri
    $env:CARGO_INCREMENTAL = "0"
    cargo test --lib app::channels
    cargo test --lib app::update_channel
    cargo test --lib app::settings_store
    cargo test --lib app::usb_guard
    cargo test --lib app::usb_media_cleanup
    cargo fmt --check
    cargo clippy --lib -- -D warnings
    cargo test --lib
    cd ..
    npx tsc --noEmit
    npm run lint
    npm run build
    npx vitest run src
    npm run test:release

    cd cloudflare/manager-download-router
    npm test

脚本语法检查：

    powershell -NoProfile -Command "& {
      $files = 'tools/build-oneclick-usb-layout.ps1','tools/build-usb-layout.ps1','tools/verify-payload.ps1','tools/verify-oneclick-usb.ps1','tools/update-from-github-fork.ps1'
      foreach ($file in $files) { [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $file), [ref]$null, [ref]$null) | Out-Null }
    }"

## 单元与集成覆盖

Rust channel tests cover strict repo validation, digest validation, confined
relative paths, payload metadata integrity, deterministic root SHA256
generation (excluding mutable usb-license.json, .staging, and the portable
manager staging directory), all asset-id
stale fields, deterministic committed-transaction recovery/rollback, and a
fully mocked pre-commit apply pipeline.

本轮实际结果：完整 `cargo test --lib` 为 342 passed、0 failed、1 ignored；
其中 ignored 是既有的只读本地 release hash 审计。`cargo fmt --check` 和未
豁免的 `cargo clippy --lib -- -D warnings` 均通过；TypeScript 通过，Vitest
30 files/308 tests 通过；五个 PowerShell AST 解析通过。

新增代码闭环证据：mock transport 已覆盖无网络的 apply happy path、下载错误、
SHA256SUMS 错误、GitHub digest 错误、Authenticode/identity 错误、进度回调和
提交前回滚；`resolve_latest` 已覆盖缺资产、非 HTTPS、malformed/mismatched
release manifest 和非法 digest。fake Authenticode 不等价于真实签名验证。

USB guard tests cover v1 raw-anchor compatibility, v2 canonical anchor
allow-list behavior, legal mutable metadata changes retaining the anchor, and
immutable-field tampering changing the anchor. Existing media cleanup tests
cover license and atomic-backup handling.

前端 tests cover Settings repo save/clear/invalid behavior, manager source
semantic separation, About Windows/authorization gate and stale pending
cleanup, plus Tauri-boundary refusal in managerApi.

## schema 3 capability fixture 验收

应准备一个完整的 x64 和 arm64 schema 3 capability fixture。每个 fixture 需要：

1. bundle 根有 manifest.json、usb-manifest.json、SHA256SUMS.txt；
2. payload path 由 root manifest 的 architecture 派生；
3. apps/Codex 不重复放置根 manifest 或 payload；
4. root/usb manifest 顶层 `schemaVersion == 2`，且两侧声明
   `updateCapability.schemaVersion == 3`、`anchorSchema == 3`；
5. 可选一份 payload/_backup/OpenAI.Codex_<arch>_<version>.msix，
   并列入根 SHA256SUMS；
6. 不含 .bak、.part、.tmp 或成功事务留下的 .staging / portable manager staging 残留；
7. 运行 verify-oneclick-usb.ps1 -SkipSignature -SkipNetwork。

当前仓库的 release/usb-oneclick-v0.5.24-final 是历史 schemaVersion=1
fixture；它应被 verifier 明确拒绝，不能作为 schema 3 通过证据。历史 v2
capability fixture 同样只能验证启动兼容，不能作为当前 Codex 更新通过证据。

当前已基于历史 x64 payload、`release/setup` 产物和完整能力包生成过
`release/usb-oneclick-v2-fixture-r8`。Windows PowerShell 5.1 下构建成功，随后
执行 `verify-oneclick-usb.ps1 -SkipSignature -SkipNetwork` 通过；MSIX 使用锁定的
OpenAI Marketplace 证书主体，VC++/WebView2 使用 Microsoft 签名与固定哈希。
加入 Launcher/Manager 严格 Authenticode 门禁后，默认 `verify-oneclick-usb.ps1
-SkipNetwork` 按预期拒绝该开发 fixture（两个 PE 为 NotSigned），非跳过签名的
构建也在创建输出前拒绝。第一次构建暴露了
旧输入 manifest 缺失 `schemaVersion` 时 PowerShell 追加属性失败的问题，已在
`tools/build-usb-layout.ps1` 改为兼容新增/覆盖属性并重跑通过。

仍缺少 arm64 fixture、真实可移动 USB、真实 GitHub 网络/代理和真实签名资产，
以及真实故障注入；因此 R2/R4/R5/R7-R11 的真实 apply/guard 证据仍不足，R1/R3
和 R13 的 x64 静态部分已有 fixture 证据。完整 Rust 测试已在当前工作盘通过；
这不替代真实 USB、真实签名、真实网络和跨架构发布验收。

本轮额外门禁：`npm run build` 通过（renderer entry isolation verified）；
完整 `npm test` 为 33 files/373 tests；
`npm run test:release` 为 2 files/56 tests，Cloudflare download router 为
1 file/9 tests。Vite 的大 chunk 提示是性能建议，不影响构建正确性；当前不
把它冒充为发布阻断。`npm run test:offline-bundle` 通过，合成 MSIX 模拟收集、
清单、SHA-256、U 盘布局和 276 个清单项逐文件复核均通过。

## R1-R13验收矩阵

| ID | 场景 | 必须证明 |
| --- | --- | --- |
| R1 | v2 bundle 根布局 | check 从根 manifest 读版本，不访问 apps/Codex |
| R2 | 有效 apply | 仅替换 USB payload，三个元数据同步 |
| R3 | 成功 verifier | 完整清单通过，无残留和遗漏 |
| R4 | guard re-observe | v2 anchor 不变，license/lease 仍授权 |
| R5 | 不可变字段篡改 | fail closed，不发生 swap |
| R6 | v1 bundle | 远端请求前拒绝，不改 license/payload |
| R7 | stale | repo、release、任一资产、版本或本地 manifest 变化均拒绝 |
| R8 | 网络元数据异常 | 缺资产、非法 digest、错误 repo 清晰失败 |
| R9 | 下载/校验失败 | staged 与 .part 清理，旧 bundle 不变 |
| R10 | 事务故障/retry | 恢复为旧完整状态或完成为新完整状态 |
| R11 | 并发/拔盘/reparse | 单一 operation，授权边界 fail closed |
| R12 | Settings/About | repo 语义、清空、非法值和平台 gating 正确 |
| R13 | x64/arm64 | verifier 不硬编码 x64，backup 入清单 |

## Windows人工E2E

以下项目必须在真实 Windows、真实可移动卷、真实 OpenAI 签名 MSIX 上执行：

开发 fixture 只允许用于结构和离线哈希链验证：

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-oneclick-usb.ps1 -BundlePath <v2-fixture> -SkipSignature -SkipNetwork

正式签名包必须去掉 `-SkipSignature`；Launcher/Manager、MSIX、VC++ 和
WebView2 的签名均需通过：

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-oneclick-usb.ps1 -BundlePath <signed-v2-fixture> -SkipNetwork

随后依次验证：check、显式确认 apply、进度事件、payload 与三个 metadata
更新、backup 清单、重新观察 USB guard，以及 Windows 已部署 Codex 不被自动
改变。改变 repo、拔盘、断网、取消、digest/SHA mismatch、权限失败和两个
并发 apply 时，应看到清晰错误且能安全 retry。

未连接真实 USB 或未提供签名产物的运行，只能记录为“证据不足”。
