# U 盘签名授权 GUI 开发与操作文档

> 版本化密钥绑定规则：正式包必须显式使用当前版本的 release profile。
> 不要使用当前开发机自动生成的 DPAPI 公钥替换版本 profile；不同环境即使 `keyId` 相同，
> 公钥摘要也可能不同。若 GUI 报“签名环境不匹配”，先对照当前版本 profile 和 portable vault，
> 不要通过修改 Bundle 或删除密钥文件绕过门禁。

状态：已实现；GUI 内部授权模式与跨机器加密便携签名库已落地。正式发行目录必须携带匹配 vault，运行电脑不参与密钥选择。
更新日期：2026-09-13。
适用版本：v0.5.24 USB one-click Bundle。

> 当前操作以 [`docs/PORTABLE-USB-SIGNER-DEVELOPMENT.md`](PORTABLE-USB-SIGNER-DEVELOPMENT.md)
> 和 [`docs/USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md`](USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md)
> 为准。每个版本的 profile、Bundle 和 signer 都可以有不同的 `keyId + publicKeySha256`，只允许
> 同一版本三方完全匹配。

## 1. 交付形态

GUI 是独立的 Windows x64 原生窗口程序：

```text
tools/usb-license-signer/src/bin/usb-license-signer-gui.rs
tools/usb-license-signer/src/gui.rs
tools/usb-license-signer/src/main.rs       # CLI 与 GUI 共用的命令管线
```

构建并生成可双击目录：

```powershell
Set-Location 'D:\AI\AGENT_U盘\codex-desktop-usb-installer'
$guiOutput = Join-Path .\release ("usb-license-signer-gui-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
pwsh .\tools\build-usb-license-signer-gui.ps1 -OutputPath $guiOutput `
  -SigningProfilePath .\release-profiles\usb-v<version>-<release>.json `
  -PortableVaultPath .\path\codex-usb-signing-key.vault `
  -PortableVaultDescriptorPath .\path\codex-usb-signing-key.vault.public.json
```

输出目录必须是新的空目录；构建脚本会拒绝覆盖仓库根目录、target 目录或已有文件的发布目录。

产物是：

```text
release/usb-license-signer-gui-<timestamp>/
├─ CodexUsbLicenseSigner.exe   # 双击启动
├─ README.txt
├─ DEVELOPMENT.md
├─ codex-usb-signing-key.vault       # 加密 portable vault，正式包必需
├─ codex-usb-signing-key.vault.public.json
├─ manifest.json
└─ SHA256SUMS.txt
```

正式复制到其他电脑时，必须把 vault 和公共描述与 EXE 一起复制；它们是加密密钥库及
公开绑定元数据，不是明文私钥。

GUI 不复制 Codex 安装器，也不把 GUI 放进 U 盘 Bundle。它是签名机上的离线工具；安装器 Bundle 仍由现有 `build-oneclick-usb-layout.ps1` 生成。

## 2. 与当前安装器程序包的配合

当前 Bundle 的根目录结构仍为：

```text
CodexDesktopSetup.exe
CodexAppManager.exe
manifest.json
usb-manifest.json
SHA256SUMS.txt
payload/OpenAI.Codex_x64.msix
content/codex-starter-kit/
ccswitch/CodexSwitch.exe
resources/prerequisites/
```

最简单的正式操作链是：

```text
构建/签名安装器 EXE
        ↓
生成完整 USB Bundle
        ↓
验证 Bundle 结构、清单、哈希和 Payload
        ↓
复制 Bundle 到目标 NTFS U 盘
        ↓
双击 CodexUsbLicenseSigner.exe
        ↓
自动扫描 → 深度检查 → 点击“注册账号并签名” → 自动复核
        ↓
从 U 盘根目录双击 CodexDesktopSetup.exe
```

GUI 与 CLI 调用同一个签名管线（便携密钥使用 `run_with_portable_password`），因此不会形成第二套 License schema 或第二套签名算法。GUI 只负责扫描、显示确认摘要、启动后台任务和展示结果；实际写入仍由 signer 的原子事务完成。

## 3. 一次性签名机准备

正式管理器只内置本版本 profile 的公钥和 `keyId`。正式签名器只从 EXE 旁的加密便携
vault 取得签名私钥；跨电脑签名使用与该版本匹配的 vault 和用户密码，不依赖运行电脑的
DPAPI、公钥或机器指纹。DPAPI 仅可在受控的密钥迁移流程中生成下一版 vault。
本机兼容密钥位置为：

```text
%LOCALAPPDATA%\CodexUsbLicenseSigner\keys\prod-2026-01.dpapi
```

CLI/受控迁移流程的自动密钥仅公钥状态描述也会写入同一目录；便携 vault 位置为
`codex-usb-signing-key.vault`（EXE 同目录）。私钥不会进入明文文件、环境变量、命令行
或 U 盘 Bundle。密码通过原生 Windows 对话框输入，不写入日志。

## 4. 双击签名操作

1. 先确认 Bundle 是全量复制的干净发行目录；不要对已经运行过安装、含 `apps/Codex` 的目录签名。
2. 把 Bundle 内容复制到目标可移动 NTFS U 盘。Bundle 可以在 U 盘根目录，也可以放在一个普通子目录中。
3. 双击 `CodexUsbLicenseSigner.exe`。
4. 工具自动扫描可移动 NTFS 卷。扫描结果必须只有一个候选 Bundle；多候选、非 NTFS、路径穿越、重解析点或结构不完整都会停止。
5. 等待深度检查完成。摘要会显示盘符、卷序列号、Bundle 相对路径、Anchor、License 状态、Payload hash/size 和自动签名环境状态。
6. 点击“注册账号并签名”，输入随 vault 设置的密码。不需要选择文件、填写 KeyId 或选择
   目标目录；运行电脑无需持有指定公钥。
7. 等待“签名完成，自动复核通过”。成功后，工具会再次 inspect；失败或状态未知时不要直接重复点击，先重新扫描并查看本机报告。
8. 安全弹出 U 盘后，在 U 盘根目录双击 `CodexDesktopSetup.exe`，按原有安装器流程完成 UAC、载荷安装和管理器启动。

签名器只会产生一个根目录 `usb-license.json`。它不进入 `SHA256SUMS.txt`，因为授权动作要把 License 绑定到实际 U 盘的卷序列号、Anchor 和相对路径；GUI 内部授权模式仍执行其余发行文件的 SHA256、Payload 和路径检查，但不执行 PE/MSIX Authenticode。

命令行默认仍执行 Authenticode；若验收目标只要求 U 盘内部授权签名，显式增加 `--skip-authenticode`。GUI 已固定使用该模式，并在界面、确认框和报告中标记 `authenticodeMode=skipped`。

## 5. GUI 的 fail-closed 条件

“注册账号并签名”按钮只有在以下条件全部满足时才启用：

- 只有一个可移动 NTFS 候选；
- inspect 的深度 Bundle 校验通过；
- License 不是已签名、损坏或指纹不一致状态；
- Bundle 声明 `SignedRequired`，且签名器同目录存在与声明的 KeyId/公钥摘要一致的 portable vault；
- 没有正在进行的 GUI 操作或其他 GUI/CLI 签名事务；

sign 仍会在后台再次 scan/observe，写入前后重新观察真实物理介质，使用原子替换写入，回读并验签，再做完整深度检查。盘符、卷序列号、Anchor、相对路径或文件系统发生变化时不报告成功。

## 6. 历史联调边界（旧包，仅用于复盘）

当前工作区已有的 v0.5.24 readiness/G17 候选包可以用于内部授权结构联调；其中项目 PE 的 Authenticode 当前为 `NotSigned`。此前生成的候选包曾绑定到另一执行环境的 `4d14072263e1cbba65591032314ba17aeaa152bf1367663dab8d7dc9d31b9eef`，而实际 Windows 用户的 `prod-2026-01.dpapi` 公钥摘要是 `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`；两者虽然 `keyId` 相同，实际密钥并不相同，因此旧包必须被签名器拒绝。已按实际用户公钥重新生成 `release/usb-oneclick-final-20260912-key216-r3`，并重新编译管理器；当前复核环境没有可移动 NTFS 候选，不能用它替代真实 U 盘 sign/verify 证据。因此：

- GUI 可以编译、启动、扫描和展示诊断；
- 使用 GUI 内部授权模式时，干净候选不会因为 PE `NotSigned` 被系统签名条件拦截；
- 已有匹配密钥时，工具会在有效 Bundle 深检后自动复用；仅在密钥目录没有可用 Blob 时才自动初始化；若当前包不是该密钥构建，按钮仍保持禁用；
- 不能用 `-SkipSignature` 把它变成正式签名包；
- 不能把测试密钥或临时自签证书冒充生产授权证据；
- 本次 `usb-license-signer scan` 返回 `candidateCount=0`，没有向任何 U 盘写入 `usb-license.json`；插入真实 NTFS U 盘后仍必须重新执行 scan/inspect。

若产品只要求 U 盘内部授权签名，可使用 GUI 的内部授权模式；但正式 Bundle 仍必须声明与
管理器构建时相同的 Ed25519 公钥和 KeyId，否则 GUI 与运行时都会拒绝该 License。自动
生成的新密钥必须先用于构建匹配的正式包；仅生成密钥不会让旧包接受它。

## 7. 报告与故障处理

```text
%LOCALAPPDATA%\CodexUsbLicenseSigner\reports\sign-*.json
%LOCALAPPDATA%\CodexUsbLicenseSigner\reports\verify-sign-*.json
%LOCALAPPDATA%\CodexUsbLicenseSigner\audit\
```

发生“结果未知”时，不覆盖、不删除 `usb-license.json`，保留 U 盘和报告交人工复核。发生“多个候选”时只移走或改用正确的干净 Bundle，不要在 GUI 内强行选择一个绕过扫描门禁。U 盘断开、文件被占用或用户在写入期间拔盘，都应按失败/未知处理。

## 8. 2026-09-11 GUI 竞态修复与 2026-09-13 便携密钥库

曾出现“同一 U 盘和同一 DPAPI 密钥实际检查通过，但窗口显示签名环境不满足、按钮不亮”的偶发假阴性。根因是后台任务完成事件使用单个可覆盖的 pending 槽位，且状态刷新依赖排队消息；在扫描→深检连续完成时，旧刷新消息可能先于最终状态生效。

当前实现改为：

- 后台事件使用 `VecDeque` 排队，按顺序消费，不覆盖扫描、深检和签名结果；
- 所有状态变更先释放互斥锁，再在 GUI 线程应用最终控件快照；
- 文本控件通过 `WM_SETTEXT` 更新，避免跨进程窗口枚举把 EDIT 的窗口标题误读为初始占位文本；
- `signingReady.ok=true`、深检通过且 License 未签名时，签名按钮稳定启用。

便携密钥库行为补充：

- Bundle 已有旁置 vault 时，检查只确认 vault 文件存在；签名时才解密并验证 GCM 标签；
- Bundle 没有 vault 时，GUI 明确提示签名器目录不完整，不读取当前本机 DPAPI；
- 受控迁移使用 create-only，不能覆盖已有 vault；错误密码或损坏 vault 不写 U 盘；
- GUI 通过 `run_with_portable_password` 调用共享签名管线，密码不进入参数数组或日志。

当前新 GUI 发布目录由 `build-usb-license-signer-gui.ps1` 生成。每个版本必须显式传入该版本
profile、匹配的 portable vault 和对应公共描述文件；脚本拒绝生成没有 vault 的正式签名器。
任何历史 GUI/Bundle 都不能与其他版本 profile 混用。

## 9. 历史 G27/G28 密钥诊断记录（不再作为便携签名运行门槛）

G27/G28 纠正了“命令行 `init-key` 通过就认为双击 GUI 可签名”的历史发布漏洞。旧 GUI 日志曾显示
`4d1407…`，而实际交互式签名 GUI 的诊断为 `216ae3…`；两者属于不同执行上下文，不允许混用。
该诊断命令现在只用于历史 DPAPI 排查：

```powershell
CodexUsbLicenseSigner.exe --diagnose-key $env:TEMP\CodexUsbLicenseSigner-key.json
```

正式发行的 `verify-usb-signing-pair.ps1` 现在以 portable vault 公共描述为密钥来源，并强制
校验 vault 已随签名器交付；它不再执行本机 DPAPI 诊断，也不要求运行电脑持有 216 公钥。
交付前关闭旧窗口、从新目录只启动一个 GUI；GUI 只需解密同目录 vault 并在真实签名时与 Bundle
绑定复核。

## 10. 验收命令

```powershell
Set-Location 'D:\AI\AGENT_U盘\codex-desktop-usb-installer'
cargo fmt --manifest-path tools/usb-license-signer/Cargo.toml -- --check
cargo test --manifest-path tools/usb-license-signer/Cargo.toml
cargo build --release --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui
$guiOutput = Join-Path .\release ("usb-license-signer-gui-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
pwsh .\tools\build-usb-license-signer-gui.ps1 -OutputPath $guiOutput `
  -SigningProfilePath .\release-profiles\usb-v<version>-<release>.json `
  -PortableVaultPath .\path\codex-usb-signing-key.vault `
  -PortableVaultDescriptorPath .\path\codex-usb-signing-key.vault.public.json
```

硬件验收仍需在真实 Windows 可移动 NTFS U 盘上记录：单候选签名/复核、重复签名拒绝、盘符变化、拔插/替换介质、安装、启动、更新和卸载。自动化测试和本机当前 unsigned 包不能替代这些证据。
