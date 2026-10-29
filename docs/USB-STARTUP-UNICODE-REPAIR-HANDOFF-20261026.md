# USB 启动、旧入口与中文路径修复交接

## 本次范围

基于已交付 `0fcbbd89bcff1f3223067a59ce400448a34bdddd`，保留 CC Switch 4.0.20 升级，不改官方 Codex Desktop/CLI、app.asar 或认证协议。不新增构建门禁，不改 ESLint/Vitest 规则。

1. **启动实例归属**：受管启动不再把任意同目录旧窗口当作成功。仅复用本管理器持有存活 Child 句柄、且 root/home/调试模式一致的实例。未知旧核心实例先请求正常关闭，超时提示保存并关闭，不强杀；新窗口必须属于本次存活子进程。受管失败清理限制为核心程序 image，排除等待中的 shim；普通非受管启动保持原流程。
2. **历史快捷方式**：USB 管理器启动后，对有明确产品 Description 标记或精确记录的旧链接，修复为当前经验证的 shim。只处理已存在的链接，保留其他套件仍可用入口、自定义参数及归属不明入口，不创建/删除任意快捷方式。实际故障机旧链接若缺少归属证据，不能承诺自动修复，需回传 Description/记录供 Dot 处理。
3. **中文路径**：Skill Hub 的 Product Design 前置安装脚本显式输出 UTF-8 JSON，修复 Windows PowerShell 传统代码页与 Rust UTF-8 JSON 消费端不匹配。未改中文用户名、数据目录或系统代码页。编码缺口已证实，但原故障机无失败日志，不能声称这是其唯一历史根因。
4. **打包回归**：已显式导入 Buffer。可选静态测试更名为 `tools/check-ccswitch-upgrade-contract.mjs`，避免 Node test 文件被 Vitest 当作套件误收集；仍不接入构建脚本。

## 已执行证据

- `npm run lint`：通过，原有 `--max-warnings=0` 未修改。
- `npm run check`：通过。
- `npm run build`：通过，包括 renderer entry isolation。
- `npm test -- --reporter=dot`：41 文件 / 478 测试通过。
- `node --test tools/check-ccswitch-upgrade-contract.mjs`：7/7 通过（可选开发检查）。
- Windows engine USB 专项：28 通过、1 ignored；这是 Linux 上的跨平台测试，不是 Windows 实机运行。
- Windows engine Linux 全量首次执行有一个既有 Windows 反斜杠路径 canonicalize 用例失败。未修改或掩盖该用例；显式排除 `codex_app_tools_mcp_uses_bundled_gui_node_and_preserves_other_settings` 后，159 通过、1 ignored、1 filtered。Windows 下仍须运行该用例。
- `cargo check --locked --manifest-path crates/codex-win-engine/Cargo.toml --target x86_64-pc-windows-msvc --tests`：通过。是 Windows 目标交叉类型检查，不是 Windows 执行或链接。
- 实际 Windows PowerShell 5.1.26100.9711 微型 stdout 探针：默认 Console CP936，严格 UTF-8 解码失败；显式 UTF-8 后 JSON 解析成功，中文路径逐字一致。仅合成字符串，无真实用户数据。
- 新增完整 Product Design 中文路径离线安装夹具在 Linux 明确 SKIP；尚未执行 Windows 新旧脚本全安装对照。不能用微型探针代替完整安装验收。
- CC Switch 产品补丁内容未变，SHA256 仍为 B61B26D7939418D775E841390543D0E89950D4C28309C22022838BC982E53857；此前该产品源码 46/46 Windows 原生测试结果仍适用，不代表本轮管理器修复已做实机验收。

## 本地 Codex 接手

开发和代码修复仍由 Dot 负责。本地仅接手审计、Dot 环境受限的 Windows 测试、最终 EXE 打包。出现代码失败或语义冲突，请回传证据，不扩展开发，不绕过已有流程。

1. 保留隔离工作树及已有未提交修改。已应用 Buffer 修复则使用 `from-0fcbbd8.patch`；仅应用 6a31817 则使用 `from-6a31817.patch`，二选一，禁止重复叠加。普通 `git apply --check` 后 `git apply`，无需 --index。不要 reset/clean 或覆盖整个仓库。
2. Windows 补测：
   - `cargo test --locked --manifest-path crates/codex-win-engine/Cargo.toml --lib -- --test-threads=1`
   - `node tools/test-product-design-unicode-install.mjs`（在隔离工作树根目录；只生成和删除自身合成临时包，不安装到真实 home、不修改系统代码页）。
   - 原有管理器编译及完整 build-installer 流程，沿用 SignedRequired 与现有签名 profile；不用回传签名密钥。
3. 场景验收：先保存工作。分别从管理器和 apps/Codex/ChatGPT.exe 启动；已有默认 home 实例时应正常关闭后切到受管 home，若无法关闭则明确提示，不能误报成功。再次启动正确受管实例不应反复重启。仅运行 CodexCore.dat 和 shim 退出均是正常实现细节，不应据此认定绕过。
4. 检查有明确归属的旧 Core.dat 快捷方式被指向当前 ChatGPT.exe；未知归属的链接不应被擅自改写。
5. 中文账户 Skill Hub 真实重试，记录 Product Design 前置阶段和后续 Skill Hub 各自结果；失败时从管理器“打开日志目录”获取 codex-app-manager 日志，不要把 cc-switch.log 当作管理器日志。输出须脱敏，禁止回传 auth.json/key/token。
6. 沿用上一轮配置保留验收：插件、插件内 MCP、媒体/用户 MCP、权限、显式禁用值、旧数据迁移及重复运行。新 EXE 与最终签名包仍须本地构建；不得把源码测试通过写成实机故障已全部消失。

本轮 GitHub 源码传输曾被审批中断，未继续重试或改通道发布；没有新增 PR/远端提交。当前会话私有交接补丁不等于已发布代码。

## UAC 受管启动修复（2026-10-27）

本地 Codex 启动失败的现场第一失败 API 尚未确认。本次最小修正针对已发现的多余桌面 ACL 前提：提升的 USB 管理器调用 `CreateProcessWithTokenW` 时，`STARTUPINFO.lpDesktop` 从硬编码 `winsta0\\default` 改为 NULL，继承管理器当前交互 window station/desktop；普通非提升分支仍返回给既有 `Command::spawn()`，不回退为管理员身份启动。失败信息补充启动阶段/API、原始 Win32 码及 EXE 完整路径；成功日志仍记录 PID 和 EXE。

代码位置：`crates/codex-win-engine/src/windows_process.rs`、`crates/codex-win-engine/src/sys.rs`。OpenProcessToken 取得 Shell 进程 token 后直接用于 CreateProcessWithTokenW；没有 DuplicateTokenEx 本身不是本修复依据。USB 启动 EXE 的静态选择未改变：要求 direct-entry overlay 且校验成功时解析为 `CodexCore.dat`。

验证：Windows MSVC 目标 `cargo check --tests` 通过；当前 Windows 宿主完整 crate 测试为 225 passed / 39 failed / 3 ignored。失败用例主要在沙箱临时目录/文件系统操作上遇到拒绝访问或路径边界限制，包含 USB overlay 合成夹具；不能据此判定产品实机失败。Unicode/空格参数转义与受管窗口必须由本次存活子进程拥有的测试通过。需在用户授权的本机 Windows 环境补跑全套 crate 回归与提升/普通用户启动验收；未构建最终 EXE/签名包。

## Recheck after UAC launch candidate fix (2026-10-27; NOT READY)

Current-turn changes are limited to the desktop-token launch path diagnostics and inherited desktop setup in `windows_process.rs`, selected-EXE context in `sys.rs`, and this handoff note. The original failure API on the user's machine remains unknown. `lpDesktop=NULL` is an API-contract-based candidate to avoid named desktop ACL requirements; it is not a proven incident root cause.

Verification did not reach package readiness: Windows MSVC target tests type-check, but the host full crate run is 225 passed / 39 failed / 3 ignored. The detailed command output and exact failure list are in `.test-tmp/uac-launch-cargo-test-lib-20261027.log` and `.test-tmp/uac-launch-failed-tests-20261027.txt`; metadata and turn delta are in `.test-tmp/uac-launch-audit-20261027.txt`. Multiple fixture errors explicitly show ACCESS_DENIED / NTSTATUS `0xC0000022` and missing-path failures under the runner's AppContainer package temp tree; later assertion failures remain indeterminate until a normal-user rerun. No production logic was changed to force these tests green.

The normal-user rerun of the same crate test binary/filter requires action-specific approval through the un-sandboxed execution path. It has not been attempted because approval is a user-specific confirmation and the user requested that we ask before that step. No elevated/admin run, product EXE build, package build, or signing was performed. This branch is not ready to package until the ordinary-user suite and separate launch checks complete.

## Approved normal-user regression rerun and final diagnostic detail

The same `cargo test --locked --manifest-path crates/codex-win-engine/Cargo.toml --lib` command then ran under an approved normal, non-admin Windows user context using the same test executable (hash recorded in the audit file). It passed 264 tests, failed 0, ignored 3 (99.24s). This confirms the prior 39 AppContainer-run failures did not reproduce outside that container; they were test-environment failures, not established code/test failures. The fixtures use generated temporary trees and synthetic payloads/scripts/junctions; no real user Codex home was targeted.

The follow-up audit also requested specific error labels for Shell window/process/session discovery and OpenProcess. Those labels are now present. A NULL result from GetShellWindow and a zero result from GetWindowThreadProcessId are reported as stage/API outcomes without reading potentially stale GetLastError; raw Win32 codes are kept only where the failing API provides last-error semantics. Final `cargo fmt --check` and Windows MSVC `cargo check --tests` pass. The full suite predates only these final error-message labels; it was not rerun after this diagnostic-only edit.

The field's original failing API remains unconfirmed. `lpDesktop=NULL` remains an API-grounded candidate fix, not a proven field root cause. The repository is still NOT READY to package until independent local Codex review and a real launch check can validate the candidate.

## Final source test run and handoff gate (2026-10-27)

After the final API-stage diagnostic edit, the approved normal-user full crate suite was rerun: `cargo test --locked --manifest-path crates/codex-win-engine/Cargo.toml --lib` passed 264, failed 0, ignored 3 (97.34s). Complete stdout/stderr is `.test-tmp/uac-launch-final-normal-user-20261027.log`. Final test-binary and both changed source SHA256 values are recorded in `.test-tmp/uac-launch-audit-20261027.txt`. `git diff --check`, `cargo fmt --check`, and Windows MSVC `cargo check --tests` passed.

**Source is ready for independent local Codex cross-review and rebuilding a test package.** This is a separate gate from proving the user's machine is fixed. The actual machine startup regression remains unverified; no product EXE/signature package was built. Minimum field acceptance is one launch through each of the ordinary and elevated manager entry points, including a Chinese path and first-start initialization with no pre-existing `.codex`; do not rerun uninstall. Any actual elevation request beyond the already-approved test run remains unperformed and must use the normal user approval flow.

## Elevated desktop Shell token fallback (2026-10-27)

The latest screenshot reports `current desktop Shell token is elevated; refusing to launch managed Codex`. `desktop_shell_token()` now asks Windows for the Shell token's `TokenLinkedToken` only when the Shell token is elevated, and uses that handle only after confirming it is not elevated. If Windows provides no linked limited token, or the returned token is still elevated, managed launch continues to fail closed; Codex is never launched with the elevated token. The linked handle is closed through the existing `OwnedHandle` RAII wrapper. This follows Microsoft's `TOKEN_LINKED_TOKEN` and `CreateProcessWithTokenW` contracts; it does not create a hand-restricted token or alter ordinary non-elevated launch.

Verification after this change: ordinary-user Windows crate suite 264 passed / 0 failed / 3 ignored; `cargo fmt --check`, Windows MSVC `cargo check --tests`, and `git diff --check` pass. The suite does not exercise a live elevated Shell with a split UAC token. The screenshot machine's token type and whether it exposes a linked limited token remain unverified; if it has no linked token (for example, a non-split elevated token), this conservative path will still refuse launch and needs that machine's new API/error detail before considering any broader behavior change. Package status at that point: not yet produced; see the packaging result below.

## Signed test bundle for elevated Shell fallback (2026-10-27)

Built the full `SignedRequired` USB folder at `C:\Users\T14S\Desktop\Codex_USB_v0.5.25_UAC_shell-linked-token_fix_20261027` using the matching v0.5.25 profile already bundled with r3. The existing r3 release directory was retained. Bundle contains 584 files (1.69 GB); `CodexAppManager.exe` SHA256 is `9E3603AE5B373C1B3F3A3F96198E56F2352A197FC68063AD8701BC4605B0B825`, and the package checksum list includes it. Standalone `verify-usb.ps1 -SkipNetwork -InternalSigned` completed successfully during packaging. Build script passed TypeScript, lint, 409 frontend tests, 806 Rust tests (0 failed, 6 ignored), renderer build, and PE manifest checks. The default sandbox could not run Vitest because AppContainer blocked atomic renames; the same unchanged `SignedRequired` build passed in the normal non-admin user context. Python 3.12 was added to PATH only for that packaging process because the normal-user environment omitted the command.

The signer tool is unchanged by this launch-token fix and was not rebuilt. The target machine still needs to launch this new bundle through `CodexDesktopSetup.exe` and confirm the manager no longer reports the old elevated-Shell refusal and that Codex opens. That live split-token case has not been tested here. If it still fails, collect the new `codex-app-manager.log` error including API stage and Win32 code; a machine without a non-elevated linked token intentionally remains fail-closed.

## Win32 1312 follow-up (2026-10-28)

This section supersedes the earlier fail-closed statement in “Elevated desktop Shell token fallback” and the prior bundle's test instructions. Those described the previous build, which queried `TokenLinkedToken` for every elevated Shell token without first checking its elevation type.

The next test-machine screenshot reached `GetTokenInformation(TokenLinkedToken)` and failed with Win32 1312. The previous build therefore handled an elevated split token but not a desktop Shell token that has no linked token.

The launch path now reads `TokenElevationType` before requesting a linked token. For `TokenElevationTypeDefault`, Windows defines that the token has no linked token, so the manager uses the existing interactive Shell token and logs that choice. This preserves the desktop's native token behavior; on such a machine Codex may run elevated because the desktop itself has an elevated default token. For `TokenElevationTypeFull`, the manager continues to require the linked token and verify that it is not elevated. Any other type or API failure still stops the managed launch. No administrator-process token substitution or UAC policy change is made.

This is a deliberate compatibility boundary: if a machine has no split token, the manager cannot manufacture a non-elevated desktop identity. Using the actual Shell token is the only native identity available through this path. The target test machine must confirm the new package starts and record its `TokenElevationType` if it still fails. Local verification and package details will be appended after they are complete.

Verification for this follow-up: the new token-selection regression passed; the full Windows-host suite passed under the normal non-admin desktop context with 265 passed / 0 failed / 3 ignored. The sandboxed attempt reported 226 passed / 39 failed / 3 ignored because its AppContainer could not create several filesystem/junction/AppX fixtures; those failures did not reproduce in the normal-user rerun. Windows MSVC `cargo check --tests`, `cargo fmt --check`, and `git diff --check` passed. An independent read-only audit found the token selection boundaries correct and identified the historical fail-closed wording above; this section now explicitly supersedes it. The test machine's actual token type and a real elevated launch remain unverified until it tries the rebuilt package.

## Final full USB package after Win32 1312 fix (2026-10-28)

- Complete copy-ready folder: `D:\AI\AGENT_U盘\release\Codex_USB_v0.5.25_UAC_token_type_fix_20261028`.
- `verify-usb.ps1 -SkipNetwork -InternalSigned`: passed with `ready=true`; 584 files, 1.69 GiB. The Codex MSIX remains `26.1016.8622.0` x64, SHA-256 `9085CF2E2EB5081F894A2CF334E1F36DBB9438ECB409D20195AA2FE1ECFCB4C8`.
- `CodexAppManager.exe` SHA-256: `BDB63416539BED762C4D1239E313E5E555E2DC64E626FD45E020DA81629F1DFC`.
- Full packaging checks: 409 frontend tests and 806 Rust tests passed (6 ignored); the changed Windows engine separately passed 265 / 0 / 3 under normal non-admin execution. `cargo check --tests` for `x86_64-pc-windows-msvc`, `cargo fmt --check`, and `git diff --check` passed.
- The existing v0.5.25 signer GUI is unchanged by this manager-only token fix and was reused from `D:\AI\AGENT_U盘\.worktrees\ccswitch-4.0.20-final-20261026\release\usb-license-signer-gui-v0.5.25-uac-shell-linked-token-fix-20261027-r1`; its package manifest binds the same production key ID and public-key digest. No new signer binary was necessary.
- Removed the obsolete v0.5.24 release folder and four standalone v0.5.24 EXEs, the previous r3 base bundle after verifying the final copy, and generated staging/duplicate folders. The retained root `release` bundle is the single copy-ready final package.

The package is ready for the target-machine retest, but this does not prove the target machine's elevated Shell token behavior. Start `CodexDesktopSetup.exe` from the new folder. If it still reports 1312, first confirm that its manager hash matches the value above; the new code only requests `TokenLinkedToken` for `TokenElevationTypeFull`, so a matching build that still reports this API failure will narrow the case to a Full split token whose linked-token query fails.

## Click-to-launch follow-up and complete one-click bundle (2026-10-28)

The user's new screenshot confirms that clicking Launch still failed with a generic `io error: launch Codex` message and the managed executable path `apps\Codex\CodexCore.dat`. The historical manager log did not retain the inner Win32 API or error code, so the precise original failure remains unproven.

For an elevated Shell token whose `TokenElevationType` is `Default` (Windows reports no linked token), the managed path now launches with `CreateProcessAsUserW` using that exact primary token from the interactive Shell process. It does not substitute the manager's token, manufacture a token, or retry as administrator. Other token classes retain the existing `CreateProcessWithTokenW` path. A failed launch now writes the underlying API stage and Win32 error to the manager log. Independent read-only audit passed; this is an API-grounded fix candidate, not yet a confirmed field repair.

The complete one-click USB folder was rebuilt with `tools/build-oneclick-usb-layout.ps1` and verified from its bundled verifier:

- Folder: `D:\AI\AGENT_U盘\release\Codex_USB_v0.5.25_UAC_CreateProcessAsUser_fix_20261028`
- `verify-usb.ps1 -SkipNetwork -InternalSigned -UsbSigningProfilePath verification\usb-signing-release-profile.json`: `ready=true`.
- All 583 entries in `SHA256SUMS.txt` match; the folder has 584 files and is 1,819,435,329 bytes.
- Manager SHA256: `D5EDDF6AED67D6106DD22BAF5895B5650E987AC7D46572022405D718EDB20C14`; it matches the freshly built manager used by the one-click builder.
- The signer pairing check passed with `portableVaultChecked=true`, the same v0.5.25 release binding, and key ID `prod-2026-10-03`. The existing signer GUI at `release\usb-license-signer-gui-v0.5.25-uac-shell-linked-token-fix-20261027-r1` is unchanged and did not need rebuilding.
- `SignedRequired` here is the internal USB-license policy. The package intentionally does not include a device-specific `usb-license.json`; use the existing signer workflow on the copied USB. Product EXEs are not Authenticode-signed in this internal-license mode, as declared by the package verifier; external Microsoft payload/prerequisite signature checks remain in force.

Current verification: Windows engine crate 265 passed / 0 failed / 3 ignored; Windows-target `cargo check --tests`, `cargo fmt --check`, and `git diff --check` passed. The full installer pipeline also passed 409 frontend tests and 806 Rust tests (6 ignored), lint/type checks, renderer build, and PE execution-level checks. Raw logs are in `.test-tmp\codexcore-launch-fix-20261028\win-engine-final.log`, `build-installer.log`, and `build-oneclick-final.log`.

This is a complete copy-ready folder, not the inner USB layout stage. The target machine must install from its `CodexDesktopSetup.exe`, then click Launch in the manager. Real target-machine startup has not been verified. If it fails, collect the new `codex-app-manager.log` entry with API stage and Win32 code and confirm the manager hash above before further changes.

## Final r2 rebuild after source audit (2026-10-28)

The final `cargo fmt` change in `windows_process.rs` only changed import wrapping. Independent read-only review confirmed no semantic change to token selection or `lpDesktop=NULL`. Source was built from the current working tree based on `1d8966500faca64466a3fe427b5a69bbaac0cb63`; it is not a clean commit snapshot.

Final validation:

- Windows engine: 265 passed / 0 failed / 3 ignored under the normal non-admin Windows user context. The restricted AppContainer attempt produced 226/39/3 from fixture `ACCESS_DENIED` and missing-path errors; these did not reproduce in the normal-user run.
- Main Rust suite: 806 passed / 0 failed / 6 ignored.
- Frontend TypeScript and ESLint passed; 409 source tests and 478 full Vitest tests passed.
- Windows MSVC `cargo check --tests`, `cargo fmt --check`, `git diff --check`, renderer and PE manifest checks passed.
- The available Node 24.14.1 emitted a non-fatal EBADENGINE warning because jsdom declares Node 24.15+ or supported 22.x; build and tests completed.
- Independent audit found no new issue in the final source change. Actual field repair remains `INSUFFICIENT_EVIDENCE`; reviewer route metadata was `ROUTE_UNVERIFIED`.

Fresh one-click `SignedRequired` package:

- Folder: `D:\AI\AGENT_U盘\release\Codex_USB_v0.5.25_UAC_CreateProcessAsUser_fix_r2_20261028`
- Entry: `CodexDesktopSetup.exe`, SHA-256 `4775AA1D8A4881304AD24A85DA985490D1908A51B6FB71B6C4A20DEE36183698`.
- Manager: `CodexAppManager.exe`, SHA-256 `B8B72074B231132792FE5904C139875A45D8272DCF305616E465DE5ADFADEF03`.
- Shim: `CodexEntryShim.exe`, SHA-256 `C15981203C17CF3DD6909BFF4149CD899308F2474B27C2F110C799F96A155C92`.
- 584 files / 1,819,430,050 bytes; bundled verifier returned `ready=true`, all 583 SHA-256 entries matched, and the manager hash matches the newly staged executable. Setup manifest is `requireAdministrator`; direct-entry shim is `asInvoker`.
- Existing signer was unchanged and reused; pair check returned `ready=true`, `portableVaultChecked=true`. Signer GUI: `D:\AI\AGENT_U盘\.worktrees\ccswitch-4.0.20-final-20261026\release\usb-license-signer-gui-v0.5.25-uac-shell-linked-token-fix-20261027-r1\CodexUsbLicenseSigner.exe`, SHA-256 `2A9657278EF7060C9682B27223CDBE071C6D3946F8AB4B7BB712ED122CB5945C`.
- Logs: `.test-tmp\codexcore-launch-fix-20261028\win-engine-final-r2-normal-user.log`, `build-installer-r2.log`, and `build-oneclick-r2.log`.

**Acceptance boundary:** this is a fresh, fully verified test candidate, not proof that the original click-to-launch fault is fixed. The development machine's Shell token is `TokenElevationTypeLimited`; it cannot reproduce the reported elevated `TokenElevationTypeDefault` branch, and the historical failure did not record the failing Win32 API/code. Test this r2 package on the affected machine. If it still fails, collect the new manager log entry with API stage and Win32 code.
