# U 盘签名工具与全链路签名门禁审计报告

- 审计日期：2026-09-10
- 审计对象：[USB-LICENSE-SIGNER-DEVELOPMENT.md](D:/AI/AGENT_U盘/codex-desktop-usb-installer/docs/USB-LICENSE-SIGNER-DEVELOPMENT.md)
- 审计基线：`2f20c330757a1e5e47d618526cf1a185723c752`
- 分支：`checkpoint/pre-package-2026-09-09`
- 仓库版本：`0.5.24`
- 工作区：存在用户原有未提交修改；本轮未执行 reset、clean、commit、push 或发布。
- 代码级结论：`ACCEPTED`
- 正式发行结论：`CONDITIONAL / NOT ACCEPTED`

本报告替换旧的“仅文档通过、功能尚未实现”报告。当前实现已经落盘并通过自动化回归；正式发行仍不能通过，因为现有候选包的项目 PE 为 `NotSigned`，且尚未提供生产公钥构建、生产 DPAPI/HSM 私钥 Blob 和真实 NTFS U 盘端到端记录。

本报告的历史结论不自动覆盖本轮新增的原生 GUI；GUI 的范围、证据和独立审计状态见 [USB-LICENSE-SIGNER-GUI-AUDIT.md](USB-LICENSE-SIGNER-GUI-AUDIT.md)。该附录明确记录：GUI 代码级主控自审通过，但两次独立只读审计 Agent 均超时，不能宣称 GUI 已独立审计通过。

## 1. 审计范围与证据方法

本次审计覆盖：

1. 共享 License/Anchor 核心、独立 signer CLI、构建脚本和 Bundle 验证脚本；
2. USB Guard 的启动、租约、看门狗、完整性和 signed Codex gate；
3. Codex 安装、修复、启动、payload stage/update/commit、卸载和介质清理入口；
4. 能力包、CCSwitch、诊断、日志和普通卸载与 Codex 授权门禁的边界；
5. 前端 unsigned/invalid/stopped/查询异常状态；
6. 当前 v0.5.24 程序包的结构验证、严格 PE 签名门禁和操作方法。

审计采用“独立审计发现问题—实现 Agent 修复—独立聚焦复核—主控复跑证据”的顺序：

- Singer：第一轮独立只读审计，发现 5 项 P1、2 项 P2；
- Laplace：第二轮独立只读审计，发现通用 Windows 更新最终门禁 1 项 P1 和 3 项 P2；
- Volta：整改后的最终聚焦只读复核，检查通用更新最终提交门禁、OfflineHome 查询失败路径和 SHA256SUMS License 排除规则，结论 PASS，P0/P1/P2/P3 均为 0。

未把本报告中的“代码级通过”扩展解释为真实硬件、真实证书或生产密钥已经验收。

## 2. 交付物审计

| 交付物 | 位置 | 结果 |
| --- | --- | --- |
| 共享 License/Anchor/Ed25519 核心 | `crates/codex-usb-license/` | 通过；统一 schema、标准 Base64、v1 兼容、v2 Anchor |
| 独立签名器 | `tools/usb-license-signer/` | 通过；`scan`、`inspect`、`sign`、`verify`，不接受明文私钥 |
| Bundle 运行时完整性验证 | `src-tauri/src/app/usb_bundle.rs` | 通过；Manifest、Payload、SHA256SUMS、Anchor、元数据、reparse 检查 |
| USB Guard 运行时门禁 | `src-tauri/src/app/usb_guard.rs` | 通过；SignedRequired fail-closed、lease/watchdog、keyId 绑定 |
| Codex 更新/安装提交门禁 | `src-tauri/src/app/channels/codex.rs`、`src-tauri/src/app/win_update.rs` | 通过；stage、pre-write、pre-commit、recovery 均有门禁 |
| 命令层和启动顺序 | `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs` | 通过；正式未授权时不创建 lease、不启动 Codex、不写入程序树 |
| 前端状态和按钮 | `src/app/views/OfflineHome.tsx` | 通过；unsigned/invalid/stopped/查询异常均不显示可用 Codex 操作 |
| 构建和验证脚本 | `tools/build-installer.ps1`、`tools/build-oneclick-usb-layout.ps1`、`tools/verify-oneclick-usb.ps1` | 通过；默认 SignedRequired，结构和严格签名门禁分离 |
| 开发与操作文档 | `docs/USB-LICENSE-SIGNER-DEVELOPMENT.md` | 通过；包含构建、签名、使用和正式发布顺序 |

## 3. 上轮独立审计问题闭环

| 问题 | 原风险 | 当前证据 | 结论 |
| --- | --- | --- | --- |
| S-01 | signer 写入前没有最终重新观察物理介质 | sign 在原子写入前重新 `observe_target`，比较 volumeSerial、Anchor、相对路径和 schema；写入后还会回读、验签、深检和再次观察 | PASS |
| S-02 | SHA256SUMS 可与内容一起被替换，Anchor 未锚定清单 | v2 Anchor 覆盖规范化全部非 Payload SHA 行，并显式绑定当前 Payload 行 | PASS |
| S-03 | SignedRequired 下 unsigned License 仍建立 lease/watchdog | 启动计划把 unsigned 正式状态限制为 restricted UI，清理旧 lease，不启动 helper，不允许 Codex | PASS |
| S-04 | Codex payload 更新在备份 rename 之后才门禁 | stage、pre-write 和真正 backup/payload 提交前均重新调用 signed gate；USB 更新强制 portable-usb | PASS |
| S-05 | 正式公钥和 keyId 可分别配置，可能错配 | compile-time active key/id、keyring、License keyId 三方绑定；缺失或不一致 fail-closed | PASS |
| S-06 | signer inspect 深检失败仍返回 0 | inspect 深度验证失败返回非零退出码 | PASS |
| S-07 | OfflineHome 只看 installed/hash，可能误报可用 | 只有 `authorized + watchdog running + signed + codexAllowed=true` 才显示已安装/启动/更新；初始安装按钮和 callback 同样受门禁 | PASS |
| L-01 | 通用 `win_perform_update` USB fallback 缺少最终 signed precommit | `portable-usb` 及大小写变体统一走 `is_usb_portable_install_mode`，最终 tail 调用 USB precommit | PASS |
| L-02 | invalid License 状态字段仍可能表现为 unsigned/未知 | `mark_invalid_license` 明确写入 `signatureState=invalid`、`codexBlockReason` 和错误消息 | PASS |
| L-03 | verify-oneclick 允许 SHA256SUMS 列出根 License | 脚本明确拒绝 `usb-license.json` 清单项；根 License 仍是唯一可变排除项 | PASS |
| L-04 | unsigned USB 的首次安装按钮可能仍可点 | 初始加载、轮询、安装前二次查询和安装后确认查询异常均 fail-closed；新增回归测试 | PASS |

## 4. C1-C10 硬约束结论

| 约束 | 代码级结果 | 正式证据状态 |
| --- | --- | --- |
| C1 复用现有算法、字段和 schema | 共享核心和 USB Guard 适配已统一 | PASS |
| C2 unsigned 升级前重观察并保留 installationId | signer sign 已实现并有测试 | 真实签名机待补 |
| C3 多盘、盘符变化、指纹、NTFS、结构失败均 fail-closed | scanner/signer/runtime 有对应拒绝分支 | 真实多盘/拔插待补 |
| C4 私钥隔离，正式私钥不入包/盘/仓库 | CLI 只接收受保护 DPAPI Blob；构建只注入公钥和 keyId | 生产 Blob/HSM 待补 |
| C5 普通管理器不生成正式签名 | manager 仅 auto-enroll unsigned（开发策略），SignedRequired 不写 License | PASS；正式构建待实测 |
| C6 Codex 所有写入/启动/更新路径统一过 signed gate | 命令、channel、win_update、startup 和 precommit 均覆盖 | 真实运行序列待补 |
| C7 能力包/CCSwitch/诊断与 Codex gate 解耦 | 使用独立 content gate；Codex 未授权不扩大能力包授权 | PASS |
| C8 unsigned Codex 不自动删除，保留卸载和诊断 | restricted UI、普通卸载和日志路径保留 | PASS |
| C9 License 不进入 SHA256SUMS，v2 Anchor 保护非 Payload 行 | build/verify/core/signer 一致实现 | PASS |
| C10 可复现测试和真实介质证据 | 自动化证据完整，实机证据尚缺 | CONDITIONAL |

## 5. 自动化验收证据

以下命令均在 `D:\AI\AGENT_U盘\codex-desktop-usb-installer` 执行；工作区没有因为审计而清理用户文件。

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --manifest-path crates/codex-usb-license/Cargo.toml -- --check` | 通过 |
| `cargo fmt --manifest-path tools/usb-license-signer/Cargo.toml -- --check` | 通过 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 通过 |
| `cargo test --manifest-path crates/codex-usb-license/Cargo.toml` | 7 passed |
| `cargo test --manifest-path tools/usb-license-signer/Cargo.toml` | 18 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 369 passed，0 failed，1 ignored |
| `cargo test --manifest-path src-tauri/Cargo.toml --features signed-required` | 369 passed，0 failed，1 ignored |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml` | 178 passed，0 failed，1 ignored |
| `npm test` | 34 files，395 passed |
| `npm run check` | 通过 |
| `npm run lint` | 通过，0 warning |
| `npm run build` | 通过；Vite production build 和 renderer entry isolation 通过 |
| `pwsh ./tools/test-offline-bundle.ps1` | 通过；`outputFiles=277`、`checksumEntries=276`，无 WindowsApps/WpSystem |
| `build-installer.ps1 -UsbPolicy DevelopmentAutoEnroll -SkipNpmInstall -SkipTests` | 通过；manager/launcher requireAdministrator 资源清单验证通过 |
| 当前 dev candidate `verify-oneclick-usb.ps1 -SkipNetwork -SkipSignature` | 通过，`ready=true` |
| 当前 dev candidate 严格验证 | 按预期退出 1；当前项目 PE 为 `NotSigned` |
| signer release build、`--help`、`scan` | 通过；当前 F: 介质为 NTFS，发现 `candidateCount=1` |
| 已安装运行树上的 signer `inspect` | 按预期退出 1；拒绝对已有 `apps/Codex` 的运行树签名 |
| `git diff --check` | 通过；仅有 Git 对 LF/CRLF 的提示，无 whitespace error |

`src-tauri` 测试期间出现既存 `codex-theme-engine` 增量元数据目录 `拒绝访问` warning，但编译、测试退出码为 0；该 warning 不改变本次结论。

## 6. R1-R15 验收矩阵

| ID | 场景 | 当前结论 |
| --- | --- | --- |
| R1 | 新 NTFS Bundle 签名 | 实现/单测 PASS；真实 DPAPI Blob 和真实目标待补 |
| R2 | 多个候选可移动盘 | 实现 PASS；真实多盘现场待补 |
| R3 | FAT/exFAT/未知文件系统 | 实现 PASS；真实设备现场待补 |
| R4 | unsigned 升级并保留 installationId | 实现/单测 PASS；真实签名报告待补 |
| R5 | 已有错误签名不覆盖 | 实现/单测 PASS |
| R6 | Bundle 复制到另一块 USB | 指纹 gate PASS；真实替换盘现场待补 |
| R7 | 确认后盘符变化 | 实现/单测 PASS；真实盘符变化现场待补 |
| R8 | Manifest/Payload/SHA256SUMS 被修改 | 自动化 PASS |
| R9 | 删除 License 或 signature | 自动化 PASS |
| R10 | UI 检查后 USB 被替换 | 代码级 pre-write/pre-commit PASS；真实拔插现场待补 |
| R11 | unsigned Codex 已存在 | 自动化 PASS；不自动删除，可卸载 |
| R12 | 能力包独立安装 | 自动化 PASS |
| R13 | keyId 轮换和 revoked key | 代码/单测 PASS；生产 keyring 轮换记录待补 |
| R14 | 非 USB 管理器 | 代码路径保持 `guard_required=false` 时的普通行为；正式安装环境待实测 |
| R15 | 真实 Windows NTFS U 盘 | 未完成；需要真实签名、安装、启动、更新、卸载全流程记录 |

因此，本轮“代码+自动化验收”可以通过；R1-R7、R10、R13、R15 的真实环境部分不能用临时目录或测试 key 代替。

## 7. 当前程序包与操作方法审计

当前结构联调候选：

```text
D:\AI\AGENT_U盘\codex-desktop-usb-installer\release\usb-oneclick-v0.5.24-current-dev-candidate-final-20260910
```

开发联调：

```powershell
Set-Location 'D:\AI\AGENT_U盘\codex-desktop-usb-installer'
pwsh .\tools\build-installer.ps1 -UsbPolicy DevelopmentAutoEnroll
$installer = '.\release\setup\CodexUsbLauncher.exe'
$manager = '.\release\setup\CodexAppManager.exe'
$inputBundle = '.\release\usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910'
pwsh .\tools\build-oneclick-usb-layout.ps1 -InstallerPath $installer -ManagerPath $manager -BundlePath $inputBundle -OutputPath '.\release\usb-oneclick-v0.5.24-current-dev-candidate-final-20260910' -SkipSignature
pwsh .\tools\verify-oneclick-usb.ps1 -BundlePath '.\release\usb-oneclick-v0.5.24-current-dev-candidate-final-20260910' -SkipNetwork -SkipSignature
```

开发包可以 auto-enroll unsigned License，但只能用于结构、安装事务和门禁联调。不要在签名前运行安装、更新或会修改 Bundle 的能力包操作，也不要对已经有 `apps/Codex` 运行树的 U 盘直接签名。

正式包：

```powershell
$env:CODEX_USB_GUARD_PUBLIC_KEY_B64 = '<生产公钥 Base64；不是私钥>'
$env:CODEX_USB_GUARD_PUBLIC_KEY_ID = 'prod-2026-01'
pwsh .\tools\build-installer.ps1 -UsbPolicy SignedRequired
pwsh .\tools\build-oneclick-usb-layout.ps1 -InstallerPath '.\release\setup\CodexUsbLauncher.exe' -ManagerPath '.\release\setup\CodexAppManager.exe' -BundlePath '.\release\usb-oneclick-v0.5.24-uninstall-finalizer-fix-20260910' -OutputPath '.\release\usb-oneclick-v0.5.24-signed-ready'
```

正式流程必须先完成 Launcher、Manager、CCSwitch 和项目拥有的 PE AuthentiCode，再执行没有 `-SkipSignature` 的严格 Bundle 验证；之后把完整 Bundle 复制到目标 NTFS U 盘，最后在签名机执行：

```powershell
usb-license-signer.exe scan
usb-license-signer.exe inspect --drive E: --bundle-relative-path CodexUsb
usb-license-signer.exe sign --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "SIGN <volumeSerial>-<anchorSha256前12位>" --report 'C:\UsbSigningReports\v0.5.24-E.json'
usb-license-signer.exe verify --drive E: --bundle-relative-path CodexUsb
```

E: 只是示例；目标盘符和 Bundle 必须以 signer `scan`/`inspect` 的实际结果为准。签名后从 U 盘根目录启动 `CodexDesktopSetup.exe`，接受标准 UAC，等待 signed License、物理指纹、Bundle 和 Payload 验证通过，再安装 Codex。更新前后管理器会重新校验；任何不可变文件变化都必须重新构建、复制和签名。

## 8. 正式发行阻断项与最终结论

当前候选包严格验证失败是预期的负向证据：

```text
CodexDesktopSetup.exe Authenticode 状态不是 Valid：NotSigned
```

在下列证据补齐前，不得把当前结果标记为正式发行通过：

- 项目拥有的 Launcher、Manager、CCSwitch 和必要依赖的生产 Authenticode；
- SignedRequired 构建输出与生产公钥/keyId 的归档记录；
- 受控离线 signer 环境中的 DPAPI/HSM 私钥 Blob，且报告不泄露私钥；
- 真实可移动 NTFS U 盘上的 sign/verify 报告；
- 同一真实 U 盘的安装、启动、拔出宽限、替换/盘符变化、payload 更新、卸载和失败回滚记录。

最终结论：本轮实现、审计整改、代码级独立复核和自动化测试通过，可以作为开发交付验收；正式授权发行验收条件尚未满足，不能宣称“正式签名包已完全生效”。

## 9. 2026-09-12 实际 DPAPI 密钥绑定纠正

实际 Windows 用户执行 `init-key` 返回的 `prod-2026-01` 公钥摘要为：

```text
216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80
```

此前的 `usb-oneclick-final-20260911-r3` 绑定的是另一执行环境中的 `4d140722...b9eef`，因此 GUI 签名器拒绝它是正确行为；不能通过修改 `usb-manifest.json` 摘要、删除 DPAPI Blob 或代签来绕过此门禁。

本次已完成以下根因修复：

- `build-oneclick-usb-layout.ps1` 在写发布清单前读取实际 `CodexAppManager.exe`，确认编译期 `keyId` 和公钥 Base64 都与当前 SignedRequired 构建配置一致；旧管理器与新清单拼接会直接失败；
- `src-tauri/build.rs` 在启用 `CODEX_USB_GUARD_KEYRING_JSON` 时，拒绝 active key 与编译期公钥不一致、重复或缺失；
- `src-tauri/src/app/usb_guard.rs` 运行时再次校验公钥环 active 公钥与编译期 active 公钥的原始 32 字节一致，避免清单校验和 License 验签分叉；
- 旧包错配反向测试按预期失败，公钥环错配反向测试按预期失败。

新的开发测试候选：

```text
D:\AI\AGENT_U盘\codex-desktop-usb-installer\release\usb-oneclick-final-20260912-key216-r2\
D:\AI\AGENT_U盘\codex-desktop-usb-installer\release\usb-license-signer-gui-key216-20260912\
```

新包的 `usb-manifest.json`、管理器内嵌公钥和管理器文件哈希已相互一致；包内没有 `usb-license.json`，也没有私钥。真实 U 盘上的签名、安装、启动、拔出宽限、替换/盘符变化、更新和卸载仍须在实际测试机完成，不能用本地目录验证替代。
