# Codex USB Fullfix5 audit

日期：2026-09-29<br>
工作区：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`<br>
发布目录：`release\usb-oneclick-v0.5.24-fullfix5-20260929`

## 本轮修复

- 能力包重新生成并校验 `file-manifest.json`、`checksums.sha256`；管理器记录 PowerShell 退出码、脱敏子进程输出和 readiness 失败原因。
- CCSwitch 分离首次 Bootstrap 与用户 Open：首次成功注入授权后只隐藏一次并写入 marker；后续打开先恢复已有窗口，不关闭/重启并再次隐藏。
- 普通模型目录恢复为只读元数据同步：只请求 `/health` 与 `/v1/models`，写入由管理器拥有的 `model_catalog_json`，不发送生成请求；完整模型可用性探测保留在高级功能。
- Codex 便携启动兼容性补丁覆盖所有已知 app-server spawn 形状；启动探测排除 `ConsoleWindowClass`，并使用 `CREATE_NO_WINDOW` 与稳定窗口复核。
- lint 忽略本地审计产物目录，避免第三方运行时 JavaScript 污染产品 lint 门禁。

## 验证证据

- `cargo test --manifest-path src-tauri/Cargo.toml`：554 passed，0 failed，3 ignored。
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml`：187 passed，0 failed（1 ignored）。
- `npm run check`：通过。
- `npm run lint`：通过。
- `npm test`：37 个测试文件通过，443 passed，1 skipped。
- `npm run test:release`：56 passed。
- `npm run test:offline-bundle`：通过，合成 MSIX、清单、SHA-256、U 盘布局和逐文件复核完成。
- `installer-pack/scripts/build-pack.py --verify`：255 files，`sourceTreeSha256=6f727a930f00ff466787fdd86fb0bed25f8e4a32da04df12ee5663ba039ed2b2`。
- 最终包能力包隔离安装：`overallStatus=ready`、core `skills=50`、`agents=7`、runtime `ready`、config `merged`、modelSync `ready`。
- 最终包 `SHA256SUMS.txt`：290 个条目，逐项校验无错误。
- `verify-usb.ps1 -SkipNetwork -InternalSigned -UsbSigningProfilePath ...\usb-signing-release-profile.json`：通过。
- `verify-usb-signing-pair.ps1`：通过；`keyId=prod-2026-01`、`publicKeySha256=216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`。

## 交付物哈希

| 文件 | SHA-256 |
| --- | --- |
| `CodexAppManager.exe` | `0C75302CD76FCC395B2BFA7E5DB220DEC46F17D6C54BB7E4A1FCFBD85AEE4A5F` |
| `CodexDesktopSetup.exe` | `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464` |
| `ccswitch\CodexSwitch.exe` | `3FA7E6E15263945CAFB083E0A7C02676B0D8AA90C6C9C7CDF4074A36C9C441DC` |
| `payload\OpenAI.Codex_x64.msix` | `CCD6002255042589B6EE1E711818A6703509D4A288851BEA455DA5A8125F25D0` |
| `CodexUsbLicenseSigner.exe` | `572ED752E989BCFABE3CAEE7A5B35DF78E23972292724B8AACF784DC413BDF1C` |

签名器源码和编译期管理员 Key 本轮没有变化，正式构建环境也没有提供该 Key，因此复用已验证且与本包同一 profile/releaseId 绑定的 portable-vault 签名器，没有伪造一个“新版本”目录。

## 独立审计结论与边界

独立只读终审确认：能力包、发布包清单/签名绑定、CCSwitch 一次隐藏代码路径和普通模型目录同步机制通过静态与离线证据。Codex 黑框、CCSwitch 实际恢复显示、真实 404token 模型数量以及 Codex UI 的最终渲染仍需要在带有效签名 `usb-license.json` 的真实 U 盘上冷启动验证；本机没有挂载该 U 盘，也没有发送真实上游生成请求，因此这些物理行为不能宣称已经现场闭环。

模型目录会按供应商和规范化模型 ID 合并 `-latest`、`-stable`、日期别名；不同规范化模型会全部保留。高级完整探测仍是可选功能，不是安装、CCSwitch Apply 或 Codex 启动门禁。
