# Test Log

## 2026-09-13 G22/G23 release candidate

- `npm.cmd test -- --run`: 36 files, 411 passed, 1 skipped.
- `npm.cmd run build`, `npm.cmd run check`, `npm.cmd run lint`: passed.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib --quiet`: 390 passed, 2 ignored; existing Windows incremental `.rmeta` access warning only.
- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --quiet`: 26 passed.
- `installer-pack/scripts/build-pack.py --check/--verify`: 253 files and source tree hash passed.
- `installer-pack/audit-codex-compatibility.py --root installer-pack --require-codex`: 50 Skills, 7 Agents, model policy, hashes and compatibility passed.
- `installer-pack/test-adapter-contracts.py -q`: 41 passed.
- `release/.../scripts/verify-package.ps1`: 253-file closure passed; target machine has no Python so optional compatibility audit was skipped by design.
- `tools/verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork`: latest MSIX identity/hash, CCSwitch, VC++/WebView2, USB manager binding and PE manifests passed.
- USB signer package contains no private key; signer crate tests passed. Static bundle contains no `usb-license.json`.
- After the independent audit corrected the package-audit robustness boundary, `build-pack.py --check` regenerated metadata; full source and copied-bundle closure verification passed again. Final rebuilt artifacts are the `r4` bundle and signer directories.

## 2026-09-13 G22/G23 follow-up baseline

- Baseline evidence: Skill Repo Rust targeted tests 9/9 and SkillRepoUpdateCard UI tests 8/8 pass before this change; attached G22 log is an older packaged build and is treated as a regression requirement, not as proof of the current source.
- Required new gates: Skill Hub safe detail/logging, model-catalog schema and no-secret audit, idempotent installer migration, independent uninstall report root, and no recreation of deleted manager data.

## 2026-09-06 v0.5.24 最终验证

- 许可证/介质清理定向回归 17 通过、0 失败、1 默认忽略；Tauri manager 305 通过、0 失败、1 默认忽略；Windows engine 173 通过、0 失败、1 默认忽略；Windows x64/custom-protocol 编译与 Rust 格式通过。
- TypeScript、ESLint、生产 build 通过；前端 32 个文件/364 项全部通过；发布工作流 2 个文件/56 项全部通过。并发下的 Git/PowerShell 夹具改为显式、充足的外部进程超时，消除单跑通过而全套误超时。
- 合成 offline bundle 通过；启用忽略项后，真实 802,145,236-byte MSIX 事务式解包实际执行 1/1 并通过（55.81 秒）；最终发行目录固定句柄/哈希只读预检实际执行 1/1 并通过。
- `tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 与全新 `release/usb-oneclick-v0.5.24-final` 构建通过；VC++、WebView2、CCSwitch、MSIX 哈希/签名及两个 `requireAdministrator` PE 清单均通过。
- 发布包自带验证器、源码验证器、Windows PowerShell 5.1 验证器均未跳过 Authenticode 并通过。目录 29 文件、28 条 SHA-256、1,158,086,569 字节；入口 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`，manager `b97e40f0e5526267a6bf8790dc4756b2da277044f3047b2833a4f02b28f3abcf`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，`SHA256SUMS.txt` `b1962de571277cd95047b827b701640cafe380eb11ae00f51a4a7251309db7f8`。
- 自动测试仅删除测试自建临时夹具，没有执行开发机真实卸载或修改真实 AppX/HKLM/U 盘内容；目标机需完成一次带自动许可的安装→彻底卸载→删除安装介质验收。

## 2026-09-06 v0.5.23 最终验证

- Windows engine：173 通过、0 失败、1 默认忽略；Tauri manager：301 通过、0 失败、1 默认忽略；前端 364/364；发布工作流 56/56；TypeScript、ESLint、Windows x64/custom-protocol 编译和 `git diff --check` 通过。
- 额外启用默认忽略的真实 802,145,236-byte MSIX USB 事务测试：完整安全解包到临时 `apps\Codex`、入口/manifest/事务边界通过，测试后无 staging 父目录残留（1/1，47.38 秒）。
- `tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 成功；VC++/WebView2/CCSwitch 固定哈希和签名门禁通过，两个 USB PE 均为 `requireAdministrator`。最后一次 CCSwitch 稳定复核补丁已以 v0.5.23 重新构建到空目录。
- 从相同已验证 v0.5.21 固定载荷生成全新 `release/usb-oneclick-v0.5.23-final`；目录 29 文件、28 条 SHA-256、1,158,037,323 字节。发布目录自带验证器与源码验证器均在不跳过 Authenticode 模式下通过；Windows PowerShell 5.1 对两份改动的发布脚本语法检查通过。
- 入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`；manager `8295f8c54d5ee8a599702428b01b97589f7777502a042790ab475893c44cac57`；MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；`SHA256SUMS.txt` `6ed302458a7c0c9100b8378ff5f9dd9ed771746a77be89ca70012d4f5ff7bd63`。
- 自动测试不会伪造 Intel GPU 驱动或 Windows 前景锁；目标机最终验收应确认首次 Codex 自动回退、CCSwitch 窗口实际置前和下一次同版本兼容记忆。

## 2026-09-06 v0.5.19 最终验证

- Windows engine 170/170；Tauri manager 293 通过、0 失败、1 默认忽略；PowerShell 5.1 恢复夹具 57/57；前端 364/364；发布工作流 56/56，共 940 项通过。
- `cargo check --tests --target x86_64-pc-windows-msvc --features custom-protocol`、`npm run check`、`npm run lint`、生产 build、离线 bundle、Rust 格式和 scoped `git diff --check` 通过。
- `tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 成功；从 v0.5.18 的相同已验证固定载荷生成全新 `release/usb-oneclick-v0.5.19-final`，未覆盖旧版。发布目录 25 文件、24 条 SHA-256、1,157,977,451 字节。
- 发布目录自带验证器和源码验证器均通过；两个 EXE 均验证 `requestedExecutionLevel=requireAdministrator`；真实 MSIX Authenticode `Valid`（16.12 秒），VC++/WebView2/CCSwitch 固定载荷门禁通过。
- 入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`；manager `d3da411362a8b148f4d5161d7f785bfebb54a8a56ac454dc303b70d4e3991665`；MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；最终 `SHA256SUMS.txt` `70b7dd5d81d55c965c6048b4850135ad5f026e8af5f1cb6de0af99f4977c0f0d`。
- 开发机未执行真实 AppX/HKLM/WindowsApps 变更；目标机需验收历史孤儿重水合、注册收敛、开始菜单真实启动和卸载终态。ChatGPT 空白故障等待独立报告。

## 2026-09-06 v0.5.18 最终验证

- Windows engine 170/170；Tauri manager 293 通过、0 失败、1 默认忽略；PowerShell 5.1 恢复夹具 54/54；前端 364/364；发布工作流 56/56，共 937 项通过。
- `cargo check --tests --target x86_64-pc-windows-msvc --features custom-protocol`、`npm run check`、`npm run lint`、生产 build、离线 bundle、Rust 格式和 scoped `git diff --check` 通过。
- `tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 成功；`tools/build-oneclick-usb-layout.ps1` 使用真实签名载荷生成 `release/usb-oneclick-v0.5.18-final`。发布目录 25 文件、24 条 SHA-256、1,157,966,897 字节。
- 发布目录自带验证器和源码验证器均在 Windows PowerShell 5.1、未跳过签名模式下通过；两个 EXE 均验证 `requestedExecutionLevel=requireAdministrator`；MSIX、VC++、WebView2 Authenticode 均为 `Valid`。
- 入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`；manager `743b2cab6a7c1a182121a71e0dbe588c55b237ddb45b227fd686f9bb33e0c0df`；MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；最终 `SHA256SUMS.txt` `833d877b73c62bddbf85fac0bf89e4c975c94cdad05e23a07532eb699d8f6864`。
- 未在开发机执行真实 AppX 安装/卸载、HKLM 写入或 WindowsApps 修改；目标 Windows 10 的孤儿重水合、USB 放盘、首次启动数据位置和联合卸载仍需实机验收。

## 2026-09-04 v0.5.8 最终验证

- manager241通过、0失败、1项默认忽略（真实发行包只读预检已单独通过）；engine134通过；前端280通过；发布56通过；常规计数711。
- check/lint/manager格式/diff检查通过；静态CRT两个EXE导入检查通过；源包只读预检25文件/SHA24条目通过。
- build-installer与独立build-oneclick成功；Windows PowerShell5.1最终verify-usb不跳过签名、`ready:true`。新包 `release/usb-oneclick-v0.5.8` 共25文件/1,157,624,959字节，manager SHA256 `d1fb4b632e104492535c41a6b7d7501edce8faf44182c1d9a4d24fd64080bc7d`。
- 原生临时目录模拟涵盖自删除、CWD释放、readonly/锁定/重解析保护、元数据恢复、退出后数据复核；不代表对目标机AppX/HKLM的真实验收。旧测试副作用与策略阻止清理的开发机测试残留已在审计披露。
- 详见 `docs/releases/v0.5.8-validation.md`。

## 2026-09-04 v0.5.7 最终验证

- manager 196/196，engine 130/130，前端 277/277，发布工作流 56/56；合计 659 项通过。
- check/lint/build、离线打包测试、manager 格式检查、sys.rs 格式检查、git diff --check 均通过；engine 其他原有文件的全量格式差异未改写。
- 最终 build-installer / build-oneclick-usb-layout 通过；Windows PowerShell 5.1 对完整 release/usb-oneclick 验证通过，未跳过真实 MSIX/微软运行库签名检查。
- 两 EXE requireAdministrator 清单通过；dumpbin 确认无外部 VCRUNTIME/MSVCP/api-ms-win-crt 导入。
- mock/HKCU/静态验收不等同于目标机器真实 HKLM/UAC/AppX 安装验收。完整命令与文件哈希见 `docs/releases/v0.5.7-validation.md`。

以下保留历史验证记录。

- 2026-09-03：追加完整回归：`cargo test --manifest-path crates\\codex-win-engine\\Cargo.toml --lib --quiet -- --test-threads=1` 通过，117/117；`cargo check --manifest-path src-tauri\\Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol` 通过（仅既有 dead-code 警告）。
- 2026-09-03 `rustfmt --check crates/codex-win-engine/src/sys.rs --edition 2021`：通过；`cargo test --manifest-path crates\\codex-win-engine\\Cargo.toml --lib --quiet -- --test-threads=1`：通过，117/117，包含实际 PowerShell/.NET regex 分类测试（0x80073CFA 正例、Access denied/普通策略反例）。
- 2026-09-03 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过，v0.5.5 管理器与入口重建；`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next14 -SkipSignature`：通过。
- 2026-09-03 正式目录覆盖 23 个非 MSIX 文件后，`release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork`、`tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork -SkipSignature`、`verify-usb-launcher.ps1`（入口/管理器）均通过；入口 SHA-256 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`，管理器 `7b1554329feb82822821bd8f11b43907df3d63d3987b727494cca8e9108e66f3`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-02：目标机日志审计确认 Administrators 组成员但当前进程为 UAC 过滤令牌；旧 asInvoker 管理器后置 PowerShell 写 HKLM 失败为 `UnauthorizedAccessException`。
- 2026-09-02：独立 `codex-usb-launcher` 构建通过；`verify-usb-launcher.ps1` 读取真实 PE `RT_MANIFEST` 并确认 `requireAdministrator`；`CodexDesktopSetup.exe` 与 `CodexUsbLauncher.exe` 哈希均为 `343b19d759b85552ec3b3775cf49ea3e707efd738eec9117135838db5562121d`，`CodexAppManager.exe` 哈希为 `a8a36c61e043b8931c6f714ec280b4c40872ffa0bdba1c2cc6529c0acab0e213`。
- 2026-09-02：`cargo fmt`（Tauri/launcher）、`cargo test --features custom-protocol --lib`（186/186）、launcher Windows target check、`npm run check`、`npm run lint`、`npm test -- --run src`（29/275）、`npm test -- --run`（32/340）、`npm run test:release`（56/56）全部通过。
- 2026-09-02：`npm run test:offline-bundle` 和 Windows PowerShell 5.1 直接执行 `tools/test-offline-bundle.ps1` 均通过（20 文件/19 哈希条目）；补齐 BOM 与 `System.IO.Compression` 加载，修复了原模拟脚本在 Windows PowerShell 5.1 上的解析/运行兼容性。
- 2026-09-02：最终 `release/usb-oneclick` 完整 Authenticode、SHA256/清单、`tools/verify-oneclick-usb.ps1 -SkipNetwork -SkipSignature` 和发布包 `verify-usb.ps1 -SkipNetwork` 均通过；24 文件/23 SHA256 条目，真实 MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-02 UAC 状态机回归：`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 通过；`cargo test --manifest-path src-tauri/Cargo.toml --lib --quiet` 通过，184/184，新增“组合 UAC 失败不得抑制目标卷重试”测试；`cargo check --tests --target x86_64-pc-windows-msvc --features custom-protocol` 通过。
- 2026-09-02 修复版发布：`tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next -SkipSignature` 均通过；最终 `release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork`（默认 Authenticode）和 `tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork -SkipSignature` 均通过。
- 2026-09-02 修复版证据：目录 22 文件/21 校验条目；入口 SHA-256 `f1b71106c5911fbb771fe7ed3ad7b3f91fe55f2285bef5dd7b862086ad637904`（20,001,792 bytes）；MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；CCSwitch `75b847ca54d207cbe92866154da8f504c6874f6d253423899c7cddfa5fee0045`；入口 `--codex-usb-guard` 通过（exit 0）。
- 2026-09-02 修复后前端回归：`npm run check`、`npm run lint`、`npm test -- --run src`（29 文件/275 测试）、`npm run test:release`（2 文件/56 测试）和 `npm run test:offline-bundle` 全部通过。

- 2026-09-02 发布前最终回归：`cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib --quiet -- --test-threads=1` 通过，114/114；`cargo test --manifest-path src-tauri/Cargo.toml --lib --quiet` 通过，183/183；`cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol` 通过。
- 2026-09-02 前端/离线回归：`npm run check`、`npm run lint`、`npm test -- --run src`（29 文件/275 测试）、`npm run test:release`（2 文件/56 测试）、`npm run test:offline-bundle`（18 个输出文件/17 个校验条目）全部通过；`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 和 `git diff --check` 通过。
- 2026-09-02 Windows PowerShell 5.1 兼容回归：7 个构建/收集/验证脚本语法解析通过；入口与嵌入 CCSwitch 校验脚本均为 UTF-8 BOM；无参数执行发布 `verify-usb.ps1` 通过；源脚本 `tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick` 通过。
- 2026-09-02 发布回归：`tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 通过（Rust release 2m29s）；`tools/build-oneclick-usb-layout.ps1` 通过；最终 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`（默认 Authenticode 检查）通过；验证器对临时未列入 `SHA256SUMS.txt` 的文件按预期拒绝。
- 2026-09-02 最终证据：`release/usb-oneclick` 共 22 个文件/21 个 SHA256 条目；入口 SHA-256 `e60314ac8a1facc49fd988d6184023ba12897e5521bf5d7a82adcabb98c156ac`、802,145,236-byte MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`、CCSwitch SHA-256 `75b847ca54d207cbe92866154da8f504c6874f6d253423899c7cddfa5fee0045`。
- 2026-09-02 最终入口 `--codex-usb-guard` smoke：退出码 0；直接读取最终包签名状态：MSIX `Valid`（Marketplace 证书）、VC redist `Valid`（Microsoft Corporation）。

- 2026-09-01 AppX 卷权限修复：`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 通过；`cargo test --manifest-path src-tauri/Cargo.toml --lib` 通过，179/179（含 `Add-AppxVolume` Access denied 分类和 elevated 脚本范围测试）；`cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol` 通过。
- 2026-09-01 前端回归：`npm run check`、`npm run lint` 通过；发布入口实测 `Get-AuthenticodeSignature` 对真实 MSIX 返回 `Valid`，入口 20,030,464 bytes，SHA-256 `0df0a904c26ce8a0a1e87fe32e1625217cf95fed4954ee731c8f60ed1c29a74d`。
- 2026-09-01 发布回归：`tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 通过；`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next -SkipSignature` 通过后同步到推荐目录；`release/usb-oneclick/verify-usb.ps1 -SkipNetwork` 通过；入口 `Start-Process ... --codex-usb-guard -Wait` 退出码 0；`npm run test:release` 56/56；`npm run test:offline-bundle` 通过。
- 2026-09-01 预期实机行为：第一次普通权限 `Add-AppxVolume` 拒绝后，界面应出现 Windows 标准 UAC；批准后只执行当前 USB 卷注册脚本并重试 MSIX。拒绝/无管理员凭据/企业策略禁止时应保留 `Access is denied` 与“需要管理员确认”诊断，不回退 C: portable。尚未在真实目标机点击确认，不能宣称实机部署已验证。

- 2026-09-01 Authenticode 专用路径：`RunLimits::authenticode()` 使用 5 分钟总时限；`Get-AuthenticodeSignature` 已从普通 45 秒探测预算中分离。
- 2026-09-01 离线载荷安全链路：`offline_payload_probe` 延迟签名检查；`stage_offline_payload` 先复制至 `%TEMP%\\CodexDesktopInstaller\\<version>`、二次 SHA-256，再对本地副本执行强制 Authenticode 校验。
- 2026-09-01 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path src-tauri/Cargo.toml --lib`：177/177；Windows x64 target `cargo test --lib`：177/177；Windows/custom-protocol `cargo check`：通过。
- 2026-09-01 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；`tools/build-oneclick-usb-layout.ps1` 使用真实签名 MSIX 重建通过；`release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`CodexDesktopSetup.exe --codex-usb-guard`：退出码 0。
- 2026-09-01 最终目录证据：入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA-256 `9ae60a07c0325cdad19ef5e2e7f7eb2c8f2ecf45ff5c7d452be9028086ee0242`（19,988,480 bytes）；MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；MSIX Authenticode `Valid`，签名者 `CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B`；静态入口动态 CRT imports 为空。

- 2026-08-31 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，168/168；包含 USB 指纹/许可证/5 秒状态机和 CCSwitch 哈希门禁测试。
- 2026-08-31 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-08-31 `npm run check`：通过；`npm run lint`：通过；`npm test -- --run src`：通过，29 文件/272 测试。
- 2026-08-31 `npm run build`：通过；renderer entry isolation verified。
- 2026-08-31 `npm run test:offline-bundle`：通过；合成 MSIX、清单、哈希、U 盘目录复核通过。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；真实 MSIX Authenticode/Add-AppxPackage/拔盘未覆盖。
- 2026-08-31 Release EXE `--codex-usb-guard` headless smoke：退出码 0（无租约时正常退出）。
- 2026-08-31 完整 `npm test`：业务相关测试通过；release-workflow 的 bash/WSL fixture 在本机失败，windowMode 首次并行运行偶发失败，单独重跑通过。该环境问题未改动业务代码。
- 2026-08-31 `git diff --check`：通过（仅 Git 行尾转换提示）。
- 2026-08-31 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；锁和 CCSwitch PID 校验收紧后 `cargo test` 仍为 168/168。
- 2026-08-31 `npm run test:offline-bundle`：通过；重新生成 Release U 盘目录后逐文件验证通过。
- 2026-08-31 Release EXE `--codex-usb-guard` headless smoke：退出码 0；当前 Release SHA256 为 `e7c858ea195fbde1f86f8121b193041c3ee7fcaee197c6cfcd3b0b248560fc25`。
- 2026-08-31 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，171/171；新增 USB AppXVolume 无 portable 回退、注册卷一致性和数据策略边界测试。
- 2026-08-31 `cargo check --manifest-path crates/codex-win-engine/Cargo.toml --tests --target x86_64-pc-windows-msvc` 与 `cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-08-31 `npm run check`、`npm run lint`、`npm run test:offline`：通过，离线 UI 7/7。
- 2026-08-31 `npm run test:offline-bundle`：通过；合成 MSIX bundle 17 个输出文件、16 个校验条目，联合清单声明 USB AppXVolume、`portableFallback=false` 和 `ccswitch.launchMode=usb-direct`。
- 2026-08-31 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；重新编译带 `custom-protocol` 的发布入口 EXE，避免 `127.0.0.1:1420` 开发服务器依赖。
- 2026-08-31 `tools/build-oneclick-usb-layout.ps1`：通过；真实签名 MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，输出 `release/usb-oneclick`。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；Codex MSIX、根目录 CCSwitch、嵌入资源、联合清单和逐文件 SHA256 均通过。
- 2026-08-31 `release/usb-oneclick/CodexDesktopSetup.exe --codex-usb-guard`：通过，无界面看门狗 smoke 正常退出；真实 AppX 注册/UAC/拔盘仍需 Windows 实机。
- 2026-08-31 `npm test -- --run`：产品相关测试通过（31 文件 / 336 测试）；唯一失败为 `scripts/release-workflow.test.mjs` 的 WSL `/bin/bash` fixture 超时，主机报 `CreateProcessCommon: execvpe(/bin/bash) failed`，与本次业务改造无关。
- 2026-08-31 最终发布入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA256：`d4719faabfb5afc2fbcac8912759413406d8cc28b9dbe9aac5ecb1e301a3f7eb`；重新编译后再次通过 `verify-usb.ps1 -SkipNetwork` 和 guard smoke。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`（默认启用 MSIX Authenticode 检查）：通过；`-SkipSignature` 仅保留给无签名合成 fixture。
- 2026-08-31 修复 AppX 卷冲突：USB 安装脚本在 `Add-AppxPackage` 前检测同一用户已有的 Codex 注册，跨卷时先调用 `Move-AppxPackage`，覆盖 Windows `0x80073CF9` 应用数据卷冲突；AppX 用户数据系统卷策略提升失败改为可见警告，不再阻断 USB 程序安装。
- 2026-08-31 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，171/171；`npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/272 测试。
- 2026-08-31 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；发布入口已重新编译并生成 `release/usb-oneclick`。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；最终入口 SHA256 `70ebac7ad1d0320c0e15cdf73be599799732c6c38b28d183f855e8fdb28a6100`。
- 2026-08-31 卸载残留审计：当前用户仍注册 `OpenAI.Codex` 于 `C:\Program Files\WindowsApps`，而管理器 `provenance.json` 缺失；旧版联合卸载允许点击但后端按“外部安装”拒绝移除 Codex，造成“卸载后仍在 C 盘”的表象。
- 2026-08-31 修复联合卸载：USB 明确确认路径可清理无 provenance 的当前用户 `OpenAI.Codex` MSIX；普通“卸载 Codex”仍拒绝未托管外部安装；`Remove-AppxPackage` 后增加当前用户注册消失核验，防止假报成功。
- 2026-08-31 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`、`cargo test --manifest-path src-tauri/Cargo.toml`：通过，171/171；`npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/273 测试。
- 2026-08-31 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；入口 EXE 已包含联合卸载外部 MSIX 残留修复。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；入口 EXE SHA256 `b4f4c88e9f36b7fda05d6573b7691cfdb0b481e260f60ec5f06cc4b3b2c8148d`；`--codex-usb-guard` smoke 正常返回。
- 2026-08-31 更新 U 盘 README/联合清单中的卸载说明并重新生成目录；`release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；EXE SHA256 保持 `b4f4c88e9f36b7fda05d6573b7691cfdb0b481e260f60ec5f06cc4b3b2c8148d`。
- 2026-08-31 安装/卸载非杀毒因素复核：发现并修复无 provenance 的外部 C: MSIX 被前端/后端历史入口同时拒绝的问题；USB 模式现在仅对 `source=msix` 允许 `Move-AppxPackage` 迁移，外部 portable 仍要求显式接管；已托管但仍在 C: 的同版本包不会再被离线主页误报为“已安装”。
- 2026-08-31 `npm run check`、`npm run lint`：通过；`npm test -- --run src`：通过，29 文件 / 275 测试；`npm run test:release`：通过，2 文件 / 56 测试；Windows 无 bash 时改用 PowerShell 祖先提交校验脚本。
- 2026-08-31 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`、`cargo test --manifest-path src-tauri/Cargo.toml`：通过，172/172。
- 2026-08-31 `npm run test:offline-bundle`：通过；合成 MSIX 收集、清单、SHA-256、U 盘布局和逐文件复核通过。
- 2026-08-31 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；`release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；入口 EXE SHA256 `5bec5a57d39b69a2f05a208b0dd7ccefda11641d04f3fbcf978181742aa4e5c6`，MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-08-31 `release/usb-oneclick/CodexDesktopSetup.exe --codex-usb-guard`：退出码 0；只读主机预检通过：Windows 11 Pro Build 26200 x64、F: NTFS/Removable/Healthy、F: AppXVolume Online、C: 约 54.8 GB 可用、F: 约 30.6 GB 可用、Get/Add-AppxVolume、Add/Move/Remove-AppxPackage 和 Get-AppxLog 均可用。当前主机仍有官方 C: Codex 进程，未执行真实安装/卸载以避免破坏用户环境。
- 2026-08-31 `npm test -- --run` 二次全量回归：通过，32 文件 / 340 测试；首次并行运行的焦点时序失败未复现。
- 2026-08-31 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过；与最终 U 盘 EXE 相同的 Windows/custom-protocol 特性可编译。
- 2026-08-31 当前主机非破坏性 AppX 预演：`Add-AppxVolume -WhatIf`、`Move-AppxPackage -WhatIf`、`Add-AppxPackage -Volume ... -WhatIf` 均被 Windows 接受；未执行真实迁移、安装或卸载。
- 2026-09-01 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-09-01 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path src-tauri/Cargo.toml`：通过，173/173（含 CCSwitch 所有权标记测试和 Codex 用户数据后置校验）。
- 2026-09-01 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试。
- 2026-09-01 `npm run test:offline-bundle`：通过；合成 MSIX 收集、清单、哈希、U 盘布局和逐文件复核通过（17 个输出文件/16 个校验条目）。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；最终入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA256 `812d6c9f6cc31cf5911b07c9a249603ec3d69a1d08a990345543b57a5d4d5dda`，真实 MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`Start-Process ... --codex-usb-guard -Wait`：退出码 0；当前主机仍有官方 C: Codex，未执行真实安装/卸载。
- 2026-09-01 更新 U 盘 README/联合清单的卸载边界说明并重新生成 `release/usb-oneclick`；静态校验再次通过，入口 SHA256 保持 `812d6c9f6cc31cf5911b07c9a249603ec3d69a1d08a990345543b57a5d4d5dda`。

- 2026-09-01 AppXDeploymentServer 事件 404/756/401：确认 `0x80073CF9` 的具体原因是 C: 上保留的 Codex AppX 用户数据与 USB AppX 卷安装目标不一致；未删除用户数据。
- 2026-09-01 `cargo test --manifest-path crates/codex-win-engine/Cargo.toml sys::tests:: -- --nocapture`：通过，23/23；含新增“保留 C: 数据→系统卷 bootstrap→Move-AppxPackage 到 USB”实机 PowerShell 模拟。
- 2026-09-01 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，173/173。
- 2026-09-01 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过；`npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA256 `1fc55ab85c7dc5d56a51a6e19aa1ddb621c67e2dfa64a6a27cf66fe8fde3cfe2`，MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`CodexDesktopSetup.exe --codex-usb-guard`：退出码 0。
- 2026-09-01 `rustfmt --check crates/codex-win-engine/src/sys.rs --edition 2021`：通过；保留数据 bootstrap 针对性测试再次通过（1/1）。
- 2026-09-01 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path crates/codex-win-engine/Cargo.toml sys::tests:: -- --nocapture`：通过，24/24，新增 Explorer 参数激活回归测试。
- 2026-09-01 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，173/173；`cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-09-01 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试。
- 2026-09-01 `npm run test:offline-bundle`：通过；合成 MSIX 离线布局和逐文件校验通过。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1`：通过；真实 MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，新版入口 SHA-256 `9e52863b0f8764fbc83bf9f8cd2eb57abd99ef4a39915ae6afdf37e6b478fe0c`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；新版 `CodexDesktopSetup.exe --codex-usb-guard`：退出码 0。
- 2026-09-01 最新截图分支复核：用户文案为 `MSIX 离线安装失败`，因此不是健康检查 URI 分支；当前修复保留 Explorer 激活加固，同时新增 USB AppX 卷本地/NTFS 预检、`operation`/目标卷诊断和 `Add-AppxVolume` 返回对象复用。
- 2026-09-01 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path crates/codex-win-engine/Cargo.toml sys::tests:: -- --nocapture`：通过，25/25，包含非 NTFS 卷在 AppX 变更前被拒绝的回归测试。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；`tools/build-oneclick-usb-layout.ps1`：通过；新入口 SHA-256 `484c7b737db904818cf7c52c02c1d7720b0dd5151c9e29d029e43e7755c75bf3`，MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`CodexDesktopSetup.exe --codex-usb-guard`：退出码 0；`cargo test --manifest-path src-tauri/Cargo.toml`：173/173；Windows MSVC target `cargo check`：通过；`npm run check`、`npm run lint`、`npm test -- --run src`：29 文件/275 测试；`npm run test:release`：56/56；`npm run test:offline-bundle`：通过。
- 2026-09-01 当前主机只读卷检查：F: 为 `NTFS/Removable/Healthy`，E: 为 `WebDAV` 映射盘且 `Get-Volume -DriveLetter E` 无本地卷；未执行真实 Add-AppxPackage/卸载，以避免破坏现有 Codex。
- 2026-09-01 `Get-Command Add-AppxPackage`/`Get-AppxVolume`/`-WhatIf` 只读审计：当前系统 `-Volume` 类型为原生 `AppxVolume`，文档定义的 `-Path + -Volume`（含强制更新/关闭进程参数）在系统卷 `-WhatIf` 下被接受；未执行真实安装。
- 2026-09-01 `cargo test --manifest-path crates/codex-win-engine/Cargo.toml sys::tests:: -- --nocapture`：通过，28/28；新增在线 AppxVolume 刷新、离线旧卷挂载和 USB URI→系统 bootstrap/Move 模拟。
- 2026-09-01 `rustfmt --check crates/codex-win-engine/src/sys.rs --edition 2021`：通过；`git diff --check`：通过（仅已有行尾转换警告）。
- 2026-09-01 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，173/173。
- 2026-09-01 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol` 与 `cargo check --tests ...`：通过；仅已有未使用函数警告。
- 2026-09-01 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1`：通过；重新编译入口 `release/usb-oneclick/CodexDesktopSetup.exe`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`CodexDesktopSetup.exe --codex-usb-guard`：退出码 0；入口 SHA-256 `85c498d75fe056e2bb43b1f2e066123ab44b101b27569752b983ed46a409c0ff`，MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-01 文档收尾复核：`README.offline.md`、`release/usb-oneclick/README.txt` 均已说明在线原生 AppxVolume 刷新、离线旧卷挂载、`-Path/-Volume` 绑定和 URI→系统卷 bootstrap→Move 回退；最终产物哈希与 `SHA256SUMS.txt` 一致。真实 `Add-AppxPackage` 仍需目标 Windows/NTFS U 盘执行。
- 2026-09-01 最新 URI 回退修复：`cargo test --manifest-path crates/codex-win-engine/Cargo.toml sys::tests:: -- --nocapture` 通过，29/29；新增原生 `PackageManager.AddPackageAsync` 成功分支和原生失败后系统卷 bootstrap→Move 模拟。
- 2026-09-01 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，173/173；`npm run test:release`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA256 `258437ab946585b81ba1b487a0b57b433a076bd5e38c74d8fadb402d321e48d1`，真实 MSIX SHA256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-01 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；`CodexDesktopSetup.exe --codex-usb-guard`：退出码 0；SHA256SUMS 与最终目录一致。真实 `Add-AppxPackage`/WinRT 部署仍需目标 Windows + 本地 NTFS U 盘验收。
- 2026-09-01 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path src-tauri/Cargo.toml`：通过，177/177（新增 runtime 候选路径、哈希、退出码和缺包状态测试）。
- 2026-09-01 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过；静态 CRT 配置下 Windows 目标可编译。
- 2026-09-01 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-01 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；VC redist `vc_redist.x64.exe` SHA-256 `843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C`、Microsoft Authenticode 有效；构建资源已包含 `resources/prerequisites`。
- 2026-09-01 `tools/build-oneclick-usb-layout.ps1 -SkipSignature`：通过；`release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过，MSIX/CCSwitch/运行库哈希与清单一致；`Start-Process release/usb-oneclick/CodexDesktopSetup.exe --codex-usb-guard -Wait`：退出码 0。
- 2026-09-01 最终产物：入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA-256 `F24269DD37081BCF31AAB7A9F53CE726D93222B9DD4E2BE1022AB18B2F254FC0`（19,995,648 bytes）；运行库 SHA-256 `843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C`（18,731,856 bytes）；CCSwitch SHA-256 `75B847CA54D207CBE92866154DA8F504C6874F6D253423899C7CDDFA5FEE0045`；真实 MSIX SHA-256 `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。自定义 PE 导入审计确认动态 CRT 导入为空；尚未在缺运行库的真实空白机执行 redist。
- 2026-09-01 用户截图诊断：错误详情明确为 `Get-AuthenticodeSignature: process exceeded total deadline`；源码 `crates/codex-win-engine/src/authenticode.rs:146` 使用 `RunLimits::probe()`，其总时限为 `crates/codex-win-engine/src/process.rs:18` 的 45 秒。该错误发生在 `offline_payload_probe` 的 SHA-256/身份检查之后、`Add-AppxPackage` 之前。
- 2026-09-01 只读对照：开发机 NTFS 固定卷上的同一 802,145,236-byte MSIX 执行 `Get-AuthenticodeSignature` 返回 `Valid / Signature verified.`，签名者 `CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B`，用时约 6.08 秒；无法代表用户可移动 U 盘的读速和安全软件扫描时延，因此暂不把此问题归因于签名损坏。
- 2026-09-02 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib --quiet`：通过，186/186，包含 `win_elevation` 的 USB 入口重启决策测试。
- 2026-09-02 `cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`：通过；新增 `windows-sys` Security/Threading 特性可编译。
- 2026-09-02 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过；重新生成静态 CRT 入口。
- 2026-09-02 `tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next2 -SkipSignature`：通过；复制到最终 `release/usb-oneclick` 后，默认 `verify-usb.ps1 -SkipNetwork` 和 `tools/verify-oneclick-usb.ps1 -SkipNetwork` 均通过；最终入口 SHA-256 `51e1184976829fad88d02d394c5e69619ec8af534d43889cdccdec8f8c5288d4`。
- 2026-09-02 `release/usb-oneclick/CodexDesktopSetup.exe --codex-usb-guard`：退出码 0；未在当前开发机点击真实启动 UAC，以免改变当前桌面会话，真实管理员确认仍列入目标机验收。
- 2026-09-02：发布验证脚本新增 `startupElevation` 元数据门禁（`usb-entry-runas`、标准 `runas`、正确 marker、取消即安装前退出、禁止 silent bypass）；复制到最终包并重新计算 SHA256SUMS 后，`release/usb-oneclick/verify-usb.ps1 -SkipNetwork` 通过。
- 2026-09-02：提权链路二次审计后增加“已高完整性令牌直接执行固定 AppX helper”分支，避免入口 UAC 通过后再次弹同一授权；`cargo test` 186/186、Windows target check、重新打包及最终 `verify-usb.ps1 -SkipNetwork` 和 guard smoke 均通过，入口 SHA-256 仍为 `cabce88d096c95c6627bd837c45856e56a15ff35da3004263b71937bac550b7f`。
- 2026-09-02：完成 direct-high-integrity wrapper 修正后的最终重建；入口 `release/usb-oneclick/CodexDesktopSetup.exe` 20,053,504 bytes，SHA-256 `84646112d24b4c0177424be0ec5e4cc9ece194378e77ba3916ef3bd8d6375259`；发布目录静态/AuthentiCode/清单校验、`--codex-usb-guard` smoke 通过。
- 2026-09-02：回归用户提供的 `Restricted/Undefined` PowerShell 场景：`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 通过；`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib --quiet` 通过 186/186（含外层 Restricted + 子进程 Bypass wrapper 和普通哈希表失败 JSON）；Windows MSVC tests target `cargo check` 通过。
- 2026-09-02：`npm run check`、`npm run lint`、`npm test -- --run src`（29 文件/275 测试）和 `npm run test:offline-bundle` 通过；发布镜像测试一次受默认 5 秒 host timing 影响，使用 `--testTimeout=20000` 后 2 文件/56 测试通过。
- 2026-09-02：`tools/build-installer.ps1 -SkipNpmInstall -SkipTests`、`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next7 -SkipSignature` 通过；逐文件覆盖最终目录后 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`、`tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork` 通过，22 文件/21 SHA256 条目，入口 `19e84a298c1feac6bff597f9d712f5db6264fe74bfc2212b8988f989d8b57eca`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，`--codex-usb-guard` exit 0。
- 2026-09-02：v0.5.3 权限/URI 修复候选包已覆盖正式目录（跳过相同的 802 MB MSIX）；`tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork` 通过，确认 MSIX 身份 `OpenAI.Codex 26.825.5331.0 x64`、payload SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`、CCSwitch/VC++ 哈希、逐文件 SHA256 和无残留门禁。
- 2026-09-02：`release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork` 通过；`verify-usb-launcher.ps1` 对 `CodexDesktopSetup.exe` 与 `CodexAppManager.exe` 均报告 PE `requestedExecutionLevel=requireAdministrator`；`git diff --check` 通过。
- 2026-09-02：发布验证器新增管理器角色门禁并同步到正式 U 盘目录；重新计算 23 个 SHA256 条目后，`release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork` 仍通过，确认联合清单不再残留 `asInvoker` 角色。
- 2026-09-02 `rustfmt --check crates/codex-win-engine/src/sys.rs --edition 2021`、`rustfmt --check src-tauri/src/app/appx_volume.rs --edition 2021`、`rustfmt --check src-tauri/src/app/win_elevation.rs --edition 2021`、`rustfmt --check src-tauri/src/app/win_update.rs --edition 2021`：通过。
- 2026-09-02 `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib --quiet -- --test-threads=1`：通过，116/116；包含“Add-AppxPackage 静默落 C: 时失败并清理”的回归。
- 2026-09-02 `cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib --quiet`：通过，187/187；包含高完整性 USB 提交边界和管理员 helper token guard。
- 2026-09-02 `cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-09-02 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release -- --testTimeout=20000`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-02 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过，v0.5.4 USB manager/launcher 重建并由 Windows SDK `mt.exe` 写入 `requireAdministrator`；`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next12 -SkipSignature`：通过。
- 2026-09-02 正式目录同步后，`release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork`、`tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork` 均通过；两个 PE 清单验证通过，真实 MSIX 802,145,236 bytes/AuthentiCode `Valid`，`--codex-usb-guard` smoke exit 0。
- 2026-09-02 最终 v0.5.4 产物：入口 SHA-256 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`；管理器 `c3a82c8b05c95e0ec821d7e74beb04855d7b75eaf4fdd29ef1800dae4cc1501d`；MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 2026-09-03 `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib --quiet -- --test-threads=1`：通过，122/122；新增旧模块缺少 `PreserveApplicationData` 参数时的安装错盘回滚与显式卸载回归，并保留普通 Access denied 不降级。
- 2026-09-03 `cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib --quiet -- --test-threads=1`：通过，187/187；`cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-09-03 `npm run check`、`npm run lint`、`npm test -- --run src`：通过，29 文件/275 测试；`npm run test:release -- --testTimeout=20000`：通过，2 文件/56 测试；`npm run test:offline-bundle`：通过。
- 2026-09-03 `tools/build-installer.ps1 -SkipNpmInstall -SkipTests`：通过，v0.5.6 管理器/入口重建；`tools/build-oneclick-usb-layout.ps1 -BundlePath .\\release\\usb-oneclick -OutputPath .\\release\\usb-oneclick-next18 -SkipSignature`：通过；正式目录从 next18 同步并保留真实 MSIX。
- 2026-09-03 `release/usb-oneclick/verify-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork`、`tools/verify-oneclick-usb.ps1 -BundlePath .\\release\\usb-oneclick -SkipNetwork -SkipSignature`、两个 PE `requireAdministrator` 清单校验：全部通过；24 文件/23 校验项，入口 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`，管理器 `b89c4ad5e4e6595fa5e38ef6e49c337673f2f15e7827bfb514eca5521760a830`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。

2026-09-03: final formal release verification rerun
- command: `release/usb-oneclick/verify-usb.ps1 -BundlePath release/usb-oneclick -SkipNetwork`; `tools/verify-oneclick-usb.ps1 -BundlePath release/usb-oneclick -SkipNetwork -SkipSignature`; `tools/verify-usb-launcher.ps1` for both entry and manager with `requireAdministrator`
- result: passed; payload/CCSwitch hashes and package metadata valid; both PE manifests requireAdministrator
- next: clean Windows 10/11 + NTFS removable-volume acceptance

## 2026-09-21 G39.4 complete delivery rebuild

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --all-targets`: 45 library tests + 4 GUI tests passed after correcting the stale config-precedence assertion for compile-time embedded admin credentials.
- `cargo fmt --manifest-path tools/usb-license-signer/Cargo.toml --all -- --check`: passed.
- Signer release build passed; manifest reports `adminApiKeyEmbedded=true`, `adminApiKeySource=compile-time-embedded`, `sub2ApiConfigIncluded=false`, and portable vault included.
- Signer startup smoke passed: process remained alive for 3 seconds and showed `Codex U 盘签名授权`; embedded credential presence/length/hash matched the trusted prior signer without printing the secret.
- `tools/build-installer.ps1 ... -UsbPolicy SignedRequired ...`: passed; renderer, manager, PE `requireAdministrator`, Skill Hub/base resource and CCSwitch resource gates passed.
- `tools/build-oneclick-usb-layout.ps1 ... -UsbPolicy SignedRequired ... -InternalSigned`: passed; complete package generated.
- `release/usb-oneclick-g39.4-20260921-r6-final/verify-usb.ps1 -InternalSigned -SkipNetwork`: passed; MSIX identity/hash, payload, both CCSwitch copies, prerequisites, manager binding and package checks passed.
- `tools/verify-usb-signing-pair.ps1` with the new signer GUI and current profile: passed; portable vault/profile/bundle bindings match.
- Final hashes: signer `D772C72FBFA42BD3EE6F305581CD8CBB0BB0E3339A714C0FBDE4C7837EDE63AC`; manager `73ECE3B4D507E2F10C5B789D6B33CB7EC93C980200F14BF07DA13FB2BA6F6D18`; launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`; CCSwitch `2725C147176A5E8F3A4EAC3A380B5D68FBDA08F64978A0427ECFC670CBFF5312`; USB archive `A254BC05B8A1C42B8BFCB32D7FD29829A09AF2A2CB95D3487E5E69F916C79766`; signer archive `4D972E1A06F864EAB630C35CB5F71080E82CB710DE4972056B3E6E6414F37ADA`.
- Archive listing confirmed the USB archive contains both EXEs, root/embedded CCSwitch, `usb-manifest.json` and verification script; signer archive contains EXE, portable vault, current profile, manifest and checksum file.

## 2026-09-21 G39.5/r7 final verification

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --all-targets`: passed, 48 library + 4 GUI tests.
- `cargo clippy --manifest-path tools/usb-license-signer/Cargo.toml --all-targets -- -D warnings`: passed; signer rustfmt check passed.
- Manager `cargo test --manifest-path src-tauri/Cargo.toml --lib`: passed, 436/436; manager rustfmt check passed.
- CCSwitch frontend: `pnpm format:check`, `pnpm typecheck`, and serial `pnpm test:unit -- --run`: passed, 37 files / 223 tests. Serial execution is intentional because App integration tests share a local MSW/DOM runtime.
- Nested CCSwitch Rust `cargo test --locked --lib`: 1,092 passed; one external port-15721 single-instance failure remained, and the separate Windows temp-path test passed on isolated rerun.
- Rebuilt package checks: packaged `verify-usb.ps1 -InternalSigned -SkipNetwork`, `tools/verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork`, `tools/verify-usb-signing-pair.ps1`, manifest/hash cross-check, no-admin-sidecar scan, and PE administrator-manifest checks passed.
- Final ZIP hashes: USB `7543DB9021A35CD30E19EF0B29799EAAA572931C479CBC93054E71C1B7E20DAE`; signer `99BF9EECF25CC0AD0F129A02545DBD0DB45BB9D78A03D5161942E781BE643DE3`.
- Product EXEs report `NotSigned` under Windows Authenticode by design; internal Ed25519 USB signing is the release policy, while the OpenAI MSIX and Microsoft prerequisite installers remain Authenticode-verified.

## 2026-09-22 G39.5/r8 implementation and package verification

- `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::model_catalog`: passed, 7/7.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::install_tx`: passed, 13/13.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib commands::tests`: passed, 27/27.
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml windows_process::tests`: passed, 4/4.
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml download::tests`: passed, 7/7.
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml portable::tests`: passed, 28 passed, 1 intentionally ignored real-MSIX environment test.
- `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`: passed; one existing `same_identity` dead-code warning.
- `npm run check`: passed; focused Vitest (`OfflineHome`, `WinHome`, download/operation hooks): 4 files / 82 tests passed.
- `npm run lint`: passed after excluding the separately governed `CCSwitch/` and generated `release/` trees from the manager ESLint scope.
- `npm test -- --run src`: passed, 32 files / 361 tests.
- `git diff --check`: passed; existing line-ending normalization warnings only.
- `tools/build-installer.ps1 -CcswitchArtifactDir release/ccswitch-product-g39.5-20260921-r7 -UsbPolicy SignedRequired -UsbSigningProfilePath <r7-profile> -SkipNpmInstall -SkipTests`: passed; renderer build, optimized Windows manager, `requireAdministrator` manifests and resource gates passed.
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r8-model-catalog -InternalSigned`: passed; MSIX identity/hash and manager/profile binding passed.
- New package `verify-usb.ps1 -InternalSigned -SkipNetwork`: passed; `verify-payload.ps1`: passed; `tools/verify-oneclick-usb.ps1` with the bundled r8 profile: passed; `tools/verify-usb-signing-pair.ps1`: passed.
- CCSwitch product verifier: passed; root and embedded CCSwitch SHA-256 both `D9656D97BF0EB663A5EA090D965573C86B7774633D1E2D00971B4428C7BB5A55`.

## 2026-09-22 G39.5/r8 final regression and package

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --all-targets -- --test-threads=1`: 48 core tests passed; GUI target passed; signer clippy `-D warnings` and rustfmt check passed.
- Direct same-process signer rebuild verified one `admin-` production credential candidate in the final EXE and matched its hash to the prior validated embedded signer; no credential value was printed. Final manifest reports `adminApiKeyEmbedded=true`, `compile-time-embedded`; no `sub2api-config.json` exists.
- First nested-PowerShell signer build was rejected from delivery because its manifest marker did not match the EXE bytes; the build was repeated directly in the current PowerShell process and the final EXE passed the binary embedding check. This is recorded as a fixed build-pipeline issue, not a release artifact.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`: 445 passed, 3 ignored. `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib -- --test-threads=1`: 179 passed, 1 intentionally ignored real-MSIX test after replacing pre-mount `Join-Path` resolution with lexical `[System.IO.Path]::Combine` in the generated USB AppX script.
- `npm run check`, `npm run lint`, `npm test -- --run src`: passed; frontend 32 files / 361 tests. Windows x64/custom-protocol target check and rustfmt checks passed; the only warning remains the pre-existing `same_identity` dead-code warning.
- `tools/build-installer.ps1` and `tools/build-oneclick-usb-layout.ps1` rebuilt the final manager and `release/usb-oneclick-g39.5-20260922-r8-final`. Package/payload/one-click/signing-pair/CCSwitch/PE-manifest gates all passed.
- Final package inventory: USB 289 files / 1,317,441,470 bytes; signer 8 files / 5,035,151 bytes. Signer startup remained alive after 3 seconds with title `Codex U 盘签名授权`.
- Archives: `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r8-final.zip` = 1,260,625,783 bytes, SHA-256 `30b41e18851343e0e90a10e62414c8739060a8e1c1576088dfcc68e4a92170c0`; `release/CodexUsb-LicenseSigner-G39.5-v0.5.24-20260922-r7-final-embedded.zip` = 2,278,707 bytes, SHA-256 `e090a9551de844e82e62a6cc238df7b6535cd634968692c6f07a3602445695a4`. ZIP central-directory checks confirmed all required entries.
- Physical NTFS U-disk, real 404token registration, Codex native model menu, 360-on/off install attribution, update and uninstall remain target-device acceptance items; the unrelated external CCSwitch process on port 15721 was not terminated.

## 2026-09-22 G39.5/r12 install-stall and second-stage-uninstall repair

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`: passed, 447/447; 3 intentionally ignored. The new UTF-16LE encoded-finalizer round-trip, uninstall transaction reconciliation, persistent handoff generation, and portable fallback phase paths are covered.
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`: passed; only the pre-existing `same_identity` dead-code warning remains.
- `npm.cmd test -- --run src`: passed, 32 files / 364 tests; `npm.cmd run check`, `npm.cmd run lint`, and `git diff --check` passed.
- `tools/build-installer.ps1 -UsbPolicy SignedRequired -UsbSigningProfilePath release/usb-signing-release-profile-g39.5-20260922-r12-install-recovery.json -SkipNpmInstall -SkipTests`: passed; production renderer, manager, launcher, resource, policy-binding and PE `requireAdministrator` gates passed.
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r12-install-recovery -InternalSigned`: passed; package has 289 files / 1,317,353,472 bytes. Manager `B3DBAC10DFC27BD2A2858A28882E8E969716E77AF394BD53D247041B35C18A8D`, launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`, payload `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`.
- Packaged `verify-usb.ps1 -InternalSigned -SkipNetwork`, `tools/verify-payload.ps1`, `tools/verify-oneclick-usb.ps1`, `tools/verify-usb-signing-pair.ps1`, both PE manifest checks, CCSwitch/resource checks and archive central-directory checks: all passed.
- USB ZIP: `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r12-install-recovery.zip`, 1,260,593,407 bytes, SHA-256 `08AE0767D39524C532F76ACA3FF04A62FB5D93F44968464BFC25AB35731EEE91`.
- Signer package: `release/usb-license-signer-gui-v0.5.24-g39.5-r12-20260922-install-recovery` and ZIP SHA-256 `888F555EEA3282593CC149ED2BE07E66844D2312589AD4E11131FBF0B89F656C`; signer source was unchanged this round, so the previously verified embedded signer executable was retained while its release profile/manifest/vault binding was refreshed.
- Physical NTFS U-disk sign/install/update/uninstall and real file-lock/finalizer acceptance remain target-device checks; they were not claimed from this development workspace.

## 2026-09-22 G39.5/r13 portable extraction root-cause repair

- `cargo fmt --manifest-path crates/codex-win-engine/Cargo.toml -- --check`: passed after the optimized extraction change.
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib -- --test-threads=1`: 180 passed / 1 intentionally ignored real-MSIX test. New fixture `portable_payload_extraction_copies_only_declared_app_tree` passed and verifies only the manifest-declared app tree is written.
- The formerly ignored real-package hook was also run with the r13 `OpenAI.Codex_x64.msix`: `portable::tests::real_usb_payload_extracts_transactionally --ignored` passed in 49.29s on local storage, without launching Codex or registering AppX.
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --target x86_64-pc-windows-msvc --features custom-protocol`: passed; only the pre-existing `same_identity` dead-code warning remains.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`: 447 passed / 3 intentionally ignored.
- `npm.cmd test -- --run src`: 32 files / 364 tests passed. `npm.cmd run check`, `npm.cmd run lint`, and `git diff --check`: passed.
- `tools/build-installer.ps1 -UsbPolicy SignedRequired -UsbSigningProfilePath release/usb-signing-release-profile-g39.5-20260922-r13-portable-extract.json -SkipNpmInstall -SkipTests`: passed; production manager/launcher and embedded administrator manifests rebuilt.
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r13-portable-extract -InternalSigned`: passed; 289 files / 1,317,364,224 bytes. Manager `C1464191A9EB3A84283CF77B311173DC7E6FCDEDE373BFEAF37001C2AD3DF677`, launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`, payload `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`.
- Package/payload/one-click/signing-pair/CCSwitch/PE manifest checks and ZIP central-directory checks passed. USB ZIP `799393F47F0207CD21458617546C3767B64CF103BF366A0EFDBBF2B2261B0A83`; signer ZIP `7BF8ACCE06E29ABA890FF7FE20883303C307D853AD76345B774AE9541217CFAD`.
- Physical NTFS U-disk sign/install/update/uninstall and real slow-media timing remain target-device checks; the optimized path is covered by synthetic extraction and full regression, not a physical USB run in this workspace.

## 2026-09-22 G39.5/r15 CCSwitch auto-provisioning and upstream retry

- CCSwitch source rebuilt with USB environment bootstrap, idempotent 3-attempt provider save/switch retry, background startup and exact-root window hiding. Renderer build and `pnpm.cmd tauri build --no-bundle --ci` passed; product verifier passed. New CCSwitch SHA-256: `927AF2AA40E2F5E1B08ECEF90C9BA8063DFB9667007E4F7AF12395463E867378`.
- Native model catalog backend now uses three bounded gateway attempts (8s connect / 20s request / 500ms+1s backoff), preserves stale catalog, classifies upstream authorization/response/unavailable errors and exposes explicit Chinese/English upstream messages. Frontend auto-refresh watchdog is 60s.
- `npx vitest run src/app/views/OfflineHome.test.tsx src/services/managerApi.test.ts`: 2 files / 60 tests passed. `cargo test --manifest-path src-tauri/Cargo.toml app::model_catalog --lib`: 7/7. `cargo test --manifest-path src-tauri/Cargo.toml app::ccswitch --lib`: 9/9. `cargo test --manifest-path crates/codex-win-engine/Cargo.toml windows_process --lib`: 5 normal + 1 helper test passed. `npm run check`: passed. `npm run test:release`: 56/56. `npm run test:offline-bundle`: passed. Existing `npm run lint`/full Rust release build were already passed before this packaging turn.
- `tools/build-oneclick-usb-layout.ps1` generated `release/usb-oneclick-g39.5-20260922-r15-upstream-retry-tray` (289 files, 1,317,450,042 bytes). Manager SHA-256 `2E6E02CE3DBD43412EFC7DBCE16C8C68A5B11890F6742272C0B0ACCCCD829C38`; launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`; payload `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`.
- Packaged `verify-usb.ps1 -InternalSigned -SkipNetwork`, repository `verify-oneclick-usb.ps1` with bundled profile, `verify-usb-signing-pair.ps1` against the r14 portable-vault signer, packaged payload, CCSwitch product verifier, PE manifest and SHA-256 manifest checks all passed. The repository verifier requires the explicit bundled profile path by design.

## 2026-09-22 G39.5/r14 extraction progress and recharge

- `cargo fmt`（engine/manager）通过；新增解压进度 fixture 通过。
- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib -- --test-threads=1`: 180 passed / 1 ignored；另外用 r14 真实 MSIX 执行 `real_usb_payload_extracts_transactionally --ignored` 通过，30.91 秒。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`: 447 passed / 3 ignored。
- `npm.cmd test -- --run src`: 32 files / 365 tests；新增绑定账户充值码成功提示测试通过。
- `npm.cmd run check`、`npm.cmd run lint`、Windows x64/custom-protocol target check、`git diff --check`: 通过；仅保留既有 `same_identity` dead-code warning。
- `tools/build-installer.ps1 -CcswitchArtifactDir release/ccswitch-product-g39.5-20260922-r9-unified-api -UsbPolicy SignedRequired -UsbSigningProfilePath release/usb-signing-release-profile-g39.5-20260922-r14-progress-recharge.json -SkipNpmInstall -SkipTests`: 通过。
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r14-progress-recharge -InternalSigned`: 通过；289 文件 / 1,317,433,148 字节。包内 `verify-usb.ps1`、`tools/verify-oneclick-usb.ps1`、`tools/verify-usb-signing-pair.ps1` 均通过。
- Manager `DC75E35C0C1A050235E5211E3879E3573C29CAD81D1FE6399ED11F885DF85262`；launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`；payload `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。
- Signer 目录 `release/usb-license-signer-gui-v0.5.24-g39.5-r14-20260922-progress-recharge`：8 文件 / 5,032,684 字节；EXE `F922D125C0334187D8B31F667596D34944F8BEC2C718355DAB75505F534C74F2`。本轮签名器源码未变更，复用已验证嵌入式二进制并刷新 profile/manifest/vault 绑定。

## 2026-09-22 G39.5/r16 404token account retry closure

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --all-targets -- --test-threads=1`: 49 library tests + 4 GUI tests passed. New `create_user_recovers_an_ambiguous_timeout_without_a_duplicate_post` fixture confirms a 503 after accepted `create-user` is recovered by identity lookup and does not issue a duplicate create POST.
- `cargo test --manifest-path src-tauri/Cargo.toml app::account --lib`: 10/10 passed, including transient 503 recovery, exhausted connection retry with upstream wording, stable redeem idempotency key, and retry classification. Full manager `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 451 passed / 3 ignored.
- `npm run check`: passed. Targeted renderer regression `npx vitest run src/app/views/OfflineHome.test.tsx src/services/managerApi.test.ts`: 2 files / 60 tests passed. `npm run test:release`: 2 files / 56 tests passed. `npm run test:offline-bundle`: passed. A blind `npm test` still reports the pre-existing 37 CCSwitch source-test alias-resolution failures (433 tests passed / 1 skipped); this is unrelated to the manager/account change and the release workflow intentionally runs the root test set.
- `cargo build --release --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui --target-dir tools/usb-license-signer/target-g39.5-r16-account-retry-compile`: passed as a source compile without embedding a replacement production administrator credential.
- `tools/build-installer.ps1 ... -UsbPolicy SignedRequired -UsbSigningProfilePath release/usb-signing-release-profile-g39.5-r14-progress-recharge.json -SkipNpmInstall -SkipTests`: passed; manager PE `requireAdministrator` manifest and static upstream retry strings present in `release/setup/CodexAppManager.exe`.
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r16-account-retry-manager -InternalSigned`: passed. `release/usb-oneclick.../verify-usb.ps1 -InternalSigned -SkipNetwork`, `tools/verify-oneclick-usb.ps1 ... -InternalSigned -SkipNetwork`, and `tools/verify-usb-signing-pair.ps1 -SignerGuiPath ... -ProfilePath ...` all passed. Package has 289 files / 1,317,521,722 bytes and manager SHA-256 `95711EE5FEA0BB375789075BF3FFDFF40C2C92E58F4F700C62A72ADA3888BDC0`.

## 2026-09-22 G39.5/r16 final watchdog/package rebuild

- `npm run check`: passed.
- `npm run lint`: passed.
- `npx vitest run src/app/views/OfflineHome.test.tsx src/services/managerApi.test.ts`: 2 files / 60 tests passed after changing `NATIVE_MODEL_REFRESH_TIMEOUT_MS` to 75 seconds.
- `tools/build-installer.ps1 ... -UsbPolicy SignedRequired -UsbSigningProfilePath release/usb-signing-release-profile-g39.5-r14-progress-recharge.json -SkipNpmInstall -SkipTests`: passed; optimized manager release build and requireAdministrator manifest passed.
- `tools/build-oneclick-usb-layout.ps1 ... -OutputPath release/usb-oneclick-g39.5-20260922-r16-account-retry-manager-final -InternalSigned`: passed; 289 files / 1,317,521,722 bytes.
- Final directory verifier, repository one-click verifier, and `tools/verify-usb-signing-pair.ps1` all passed. Manager SHA-256 `9C216EAC5AF7EB2761699B4908003CE7DE8D4E592A82D4DD5CE7D10759A52BD8`; CCSwitch SHA-256 `927AF2AA40E2F5E1B08ECEF90C9BA8063DFB9667007E4F7AF12395463E867378`; payload SHA-256 `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`.

## 2026-10-19 USB audit incremental checks

- `python -B docs/capability-audit-evidence-20261018/S003-financial-valuation-source-smoke-20261019.py`: 11/11; independently rerun/reviewed.
- `python -B docs/capability-audit-evidence-20261018/S049-wealth-allocation-source-smoke-20261019.py`: 9/9; independently rerun/reviewed.
- `python -B docs/capability-audit-evidence-20261018/S017-fitness-db-source-smoke-20261019.py`: 4/4; isolated temp cleaned; independently rerun/reviewed.
- `docs/capability-audit-evidence-20261018/S043-skills-search-source-smoke-20261019.ps1`: 5/5; independently rerun/reviewed.
- Evidence manifest validator: 350 unique IDs, zero missing paths, zero SHA mismatches.
- Longrun launcher attempt: failed before starting supervisor (`longrun_supervisor is unavailable`); ServerChan retries failed with WinError 10013. Fallback check found no `supervisor.py` under the installed long-running-task skill. Continue manually; next run should resume remaining thin-coverage PARTIAL/INSUFFICIENT Skills and G4 gaps.

## 2026-10-19 S051/S019 incremental evidence

- S051 PowerShell isolated helper: 10/10, independently reviewed.
- S019 `test_compile_prompt.py`: 29/29.
- S019 `test_upload_reference.py`: 5/5 under workspace-isolated TEMP/TMP; `FakeSession` only; cleanup passed. Independent rerun passed.
- Initial S019 run under default OS temp failed due sandbox PermissionError; preserved as NOT_TESTABLE environment attempt and excluded from pass count.
- Manifest closure validation: 362 records, zero duplicate IDs, missing paths, or SHA mismatches.

## 2026-10-19 G4 official custom-agent schema check

- 13/13 source/schema and installer-mapping assertions passed; independent rerun passed.
- Official documentation confirms personal custom-agent TOMLs live in `~/.codex/agents/` and require `name`, `description`, `developer_instructions`; custom agents inherit parent permissions/provider/MCP.
- No install or Codex runtime/child lifecycle call was made. G4 remains INSUFFICIENT_EVIDENCE.
- Manifest closure validation passed: 362 records, zero duplicate IDs, missing paths, or SHA mismatches.

## 2026-10-19 — S042 no-save remediation / package revalidation

| Command / check | Result | Scope and limit |
|---|---|---|
| `S042-filesystem-protocol-smoke-20261019.ps1` | PASS 7/7 | Source text contract plus guarded synthetic project-local file protocol. No Codex/model/Skill route. |
| package metadata refresh + `--check` | PASS | 313 file manifest entries; 314 checksums. |
| `audit-codex-compatibility.py --root installer-pack --require-codex` | PASS 15/15 | Codex CLI 0.157.1; 56 Skills; 7 Agents. |
| `verify-capability-pack-powershell.ps1 -Root installer-pack` | PASS | UTF-8 BOM / Windows PowerShell 5.1 syntax for 14 files. |
| `audit-capability-pack.py installer-pack` | PASS closure | 56 Skills / 7 Agents; no missing/extra paths or package hash mismatch; tree `adbf19c5b466aa1b318da5de35f756f4b3313905e6528f761e633c16b70ffe2c`. |
| Independent S042 patch review | PASS for review scope | Confirms smoke is protocol-only; S042 remains PARTIAL. |
| evidence-manifest validation | PASS | 373 unique case IDs; zero missing paths or hash mismatches. |

## 2026-10-19 — S009 / evidence-manifest closure

| Check | Result | Limit |
|---|---|---|
| Synthetic DOCX package/XML assertions | PASS | Placeholder-only artifact; no model route or generated factual summary. |
| LibreOffice render | NOT_TESTABLE | `soffice.exe` unavailable; visual QA not performed. |
| Evidence manifest closure | PASS | 387 unique records; 0 duplicate IDs, missing paths, or hash mismatches. Three stale S042 current-package hashes refreshed to post-S009 files. |

## 2026-10-19 — S012 source-pinned spreadsheet smoke

| Check | Result | Limit |
|---|---|---|
| Synthetic workbook round-trip | PASS 7/7 | Formula preserved but not evaluated; cached result/native Excel or LibreOffice rendering not tested. |
| Pack closure and compatibility | PASS | G1 56/56 Skills, 7/7 Agents, no closure/hash errors; compatibility 15/15. |
| Skill Creator validator | NOT_TESTABLE | Could not import PyYAML in bundled Python; package-native validators passed. |
| Explicit routing policy | PARTIAL | Metadata intentionally keeps non-core Skills explicit-only; no native route/model turn tested. |
| Evidence manifest closure after S012 | PASS | 400 unique records; all paths exist and all SHA-256 values match. |
| Final evidence manifest after independent S012 review | PASS | 401 unique records; zero path/hash/ID mismatches. |

## 2026-10-19 — S038 presentation Skill package cleanup

| Check | Result | Limit |
|---|---|---|
| S038 source cleanup | PASS | Legacy ClawHub install/update/feedback commands removed. |
| G1 package closure | PASS | 56 Skills / 7 Agents; zero closure/hash mismatches. |
| Codex compatibility | PASS 15/15 | No model route or native PowerPoint application test. |
| Final ledger after S038 | PASS | 405 unique records; zero path/hash/ID mismatches. |
| S038 independent audit and final ledger | PASS | Checklist points to matching current G1; 406 unique evidence rows with no path/hash/ID mismatch. |

## 2026-10-19 — S052 offline fetch-adapter test

| Check | Result | Limit |
|---|---|---|
| Fetch helper fake-module smoke | PASS 4/4 | No real Scrapling/html2text, network, browser, Skill/model route. |
| Package G1 | PASS | 56 Skills / 7 Agents; tree `8d627667…`; zero closure/hash mismatches. |
| Codex compatibility | PASS 15/15 | No warnings/errors. |
| Evidence manifest | PASS | 417 unique rows; zero path/hash/ID mismatches; independent review pending. |


## 2026-10-19 — S045 persisted ID fix verification

- `python -B -m unittest discover -s installer-pack/skills/travel-planner/scripts -p test_travel_db_trip_id_uniqueness.py -v` — PASS 3/3 (trip, expense, itinerary item uniqueness under fixed clock).
- `python -B -m unittest discover -s installer-pack/skills/travel-planner/scripts -p test_travel_db_corrupt_json.py -v` — PASS 3/3.
- `python -B -m unittest discover -s installer-pack/skills/travel-planner/scripts -p test_plan_generator_itinerary.py -v` — PASS 13/13.
- `python -B installer-pack/test-adapter-contracts.py` with TEMP/TMP under repository tmp — PASS 80, SKIP 25.
- `python -B installer-pack/audit-codex-compatibility.py --root installer-pack --json` — PASS 15 checks, zero warnings/errors.
- `python -B tools/refresh-capability-pack-metadata.py installer-pack --check` and `python -B docs/audit-capability-pack.py installer-pack` — PASS, 330/331 metadata, 56/7 closure, zero hash/manifest mismatches.
- `python docs/capability-audit-evidence-20261018/finalize-evidence-manifest.py` and verifier — PASS 710 unique rows, zero missing/stale/duplicate records.
- Debug note: first G1 attempt saw raw SHA mismatch for two edited Python files because Windows write_text introduced CRLF; normalized those two owned files to LF, refreshed package metadata, reran G1 successfully.


## 2026-10-19 — S018 SIP bool-input verification

- `python -B -m unittest discover -s installer-pack/skills/fund-portfolio/scripts -p test_sip_calculator_overflow.py -v` — PASS 5/5; independent reviewer reran 5/5.
- `python -B installer-pack/test-adapter-contracts.py` with TEMP/TMP under repository tmp — PASS 80, SKIP 25.
- `python -B installer-pack/audit-codex-compatibility.py --root installer-pack --json` — PASS 15 checks, no warnings/errors.
- `python -B docs/audit-capability-pack.py installer-pack` — PASS 56 Skills/7 Agents, 332 files/18 transient, no closure/hash mismatch.
- `python -B tools/refresh-capability-pack-metadata.py installer-pack --check` — PASS 330 manifest/331 checksum; sourceTree `12cc76113a7ee4f6d0e99a611ce1c8cd33655dad7ec1c8415e2bf84357fedf47`.
- Evidence manifest finalizer/verifier — PASS 720 unique records, no missing/stale/duplicate.


## 2026-10-19 — Cross-skill financial helper bool validation

- `python -B installer-pack/test-adapter-contracts.py` with TEMP/TMP under repository tmp — PASS 81, SKIP 25; independent reviewer reran same suite with same result.
- `python -B installer-pack/audit-codex-compatibility.py --root installer-pack --json` — PASS 15 checks; no warnings/errors.
- `python -B docs/audit-capability-pack.py installer-pack` — PASS 56 Skills/7 Agents; 332 files/19 transient; zero manifest/checksum/hash mismatches; tree `20c87e0c12e1e5fd7ebab5f733d4a3544c07907cf28ab2cd245f23d44ea0ba9d`.
- `python -B tools/refresh-capability-pack-metadata.py installer-pack --check` — PASS 330/331; sourceTree `d049445a91f01fbf2bb9839281d5445fc7e317eab71ed8c2ea8fcca6db88fc31`.
- Evidence manifest finalizer/verifier — PASS 733 unique rows, zero missing/stale/duplicate.


S004 final expansion: added asset allocator amount/monthly-expense/period bool rejection and exercised all five valuation score booleans. Adapter suite independently rerun: 81 passed /25 skipped. Current G1: 332 package files/20 transient, zero hash/manifest mismatches, tree `e5b847d7c6555dc59f29c78804acb9a7b10b6ad79c571dea738ddf2e6a6d75d9`; metadata check 330/331, sourceTree `736b40a87f693d873056b97edf86305e0fbc72f3d7781eb43337a3e514f089b3`; G2 15 checks.
- 2026-10-19 `test_pdf_tool_output_aliases.py -v`：6 tests，5 passed，1 skipped（Windows WinError 5无法创建真实硬链接；mock samefile覆盖）；独立复跑相同。`installer-pack/test-adapter-contracts.py`：81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、333 files/20 transient、零闭包/哈希差异；metadata 331/332；manifest 748条路径/哈希/唯一ID复算通过。
- 2026-10-19 `test_rebalance_output_aliases.py -v`：4/4通过，current同路径与target规范化别名均JSON失败且两输入字节/SHA不变；独立复跑4/4。`installer-pack/test-adapter-contracts.py`：81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、334 files/20 transient、零闭包/哈希差异；metadata 332/333；manifest 759条路径/哈希/唯一ID复算通过。
- 2026-10-19 `test_plan_export_output_aliases.py -v`：4/4通过，覆盖 trips DB、preferences DB规范化路径别名、两份数据库字节/SHA不变和独立输出；独立复跑4/4。Travel planner script discovery：23/23通过。adapter contracts：81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、335 files/20 transient、零闭包/哈希差异；metadata 333/334；manifest 771条路径/哈希/唯一ID复算通过。
- 2026-10-19 `test_travel_db_trip_status.py -v`：3/3通过，验证 ideas alias、current/past/idea持久化及非法状态无数据库/参数副作用；独立复跑3/3。Travel planner script discovery 26/26通过。adapter 81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、336 files/20 transient、零闭包/哈希差异；metadata 334/335；manifest 783条路径/哈希/唯一ID复算通过。
- 2026-10-19 `test_travel_db_atomic_save.py -v`：2/2通过，验证不可序列化trip和os.replace故障均保留旧库字节/旧行程且无临时文件；独立复跑2/2。Travel planner 全部脚本28/28通过。adapter 81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、337 files/20 transient、零闭包/哈希差异；metadata 335/336；manifest 795条路径/哈希/唯一ID复算通过。
- 2026-10-19 `test-skill-db-atomic-save.py -v`：跨Skill 3/3，测试序列化失败、os.replace失败和有效写入，失败时旧库 bytes/可读记录保持且无temp泄漏；独立重跑通过。Meal 10/10、Fitness 22/22、Reflection 5/5，均独立复审通过。adapter 81 passed/25 skipped。G2 15项无警告错误；G1 56 Skills/7 Agents、338 files/20 transient、零闭包/哈希差异；metadata 336/337；manifest 811条路径/哈希/唯一ID复算通过。

- 2026-10-19 `test_data_fetcher_cache_years.py`：5/5；合成provider，覆盖1→10、10→1 horizon隔离、同horizon缓存命中、all键差异/非法年数及非财报键稳定；独立复核5/5。adapter contracts 81/25，G2 15项无警告错误，G1 56 Skills/7 Agents、339 files/20 transient、零闭包/哈希差异，metadata 337/338；证据账本819条路径/哈希/ID复算闭合。

- 2026-10-19 `test-agent-file-operation-boundaries.py`：3/3通过，独立审计重跑3/3；G1 56 Skills/7 Agents、340 files/20 transient、零闭包/哈希差异，G2 15项无warnings/errors，metadata 338/339；证据账本826条完整。

- 2026-10-19 `test_travel_budget_validation.py`：6/6通过，覆盖负/非有限/非数/超大整数预算、bool、0/正预算、无效天数、generate_trip_plan调用链及CLI错误输出且不落盘；独立复核6/6。Travel Planner script suite 34/34，adapter 81/25，G2 15项，G1 56 Skills/7 Agents、341 files/20 transient无差异，metadata 339/340；证据账本835条闭合。

- 2026-10-19 `test_travel_budget_validation.py` 6/6；Travel Planner script suite 34/34；adapter 81/25；G1 56/7、341 files/20 transient零差异；G2 15项；metadata 339/340；evidence manifest 835 unique closed。

- 2026-10-19 `test_web_search_errors.py`：13/13通过，urlopen全mock，覆盖7种JSON结构、非法UTF-8、1万层JSON、5000位整数、空/成功/超限、HTTP/URL错误及密钥缺失；独立复核PASS。Adapter 81/25；G1 56 Skills/7 Agents、341 files/20 transient零差异；G2 15项；metadata 339/340；证据账本843条闭合。

- 2026-10-19 S007 cross-store partial-write：旧写序顺序合成基线复现“反思已落盘但习惯库损坏而报错”，调用参数也已被改写。修复为两库预读取/校验、延迟参数变更、成对暂存和失败补偿回滚；补测损坏库仅文件场景、fsync残片清理、replace失败回滚（原文件存在/不存在）及成功upsert保留ID。focused 6/6，Daily Reflection全套19/19，独立审计PASS。G1 56 Skills/7 Agents无闭包错；G2 15项无警告错误；adapter81/25；metadata357/358；证据账本990条全部存在、哈希匹配且无重复。回滚不提供两次replace之间断电/进程崩溃原子性。

- 2026-10-19 S047 Venture Analysis `business_evaluator.py` 输出覆盖输入：旧流程合成基线确认 CLI exit 0、输入 SHA 变化且输入文档被结果报告替换。现检查规范化路径与samefile；identity检查不确定时拒绝写入。focused 5/5，Venture Analysis全套13/13，独立审计PASS；G1 56/7无闭包差异，G2 15项，adapter81/25，metadata358/359；账本999条路径齐全、哈希匹配、ID唯一。物理硬链接因WinError 5未能创建，TOCTOU并发替换未测试。

- 2026-10-19 S027 `cycle_analyzer.py` 规则文件曾允许 `--output` 覆盖 `--rules`；旧行为基线复现成功写入报告并丢失 `phase_policies`。现加规范化路径和同文件身份检查，身份无法确认时fail-closed，并同步Skill契约。focused5/5、Macro Research全套9/9、独立审计PASS；G1 56/7无闭包差异，G2 15，adapter81/25，metadata359/360；证据账本1012条零缺失/过期/重复。物理硬链接受WinError 5限制，TOCTOU未测。

- 2026-10-19 S027 `policy_analyzer.py` 规则情景输入此前可被 `--output` 覆盖：旧序列 exit 0 且 `as_of` 被改成 `data_as_of`。现加入规范化路径/同文件身份拒绝与 identity-error fail-closed。focused5/5，Macro Research全套14/14，独立审计PASS；G1 56/7无闭包差异、G2 15、adapter81/25、metadata360/361；证据账本1021条全部闭合。物理硬链接受WinError 5限制，TOCTOU未测。

- 2026-10-19 S030 Meal Planner `duration_days=-1` 曾生成零天计划但成功保存并输出。旧等效流程复现 active_plans 新增；现 `generate_weekly_plan` 和 CLI 均在营养计算/档案/写盘前要求正整数。直接API拒绝负/零/bool/float/string；CLI负/零无output、无DB变更，合法1天仍成功。focused4/4、全套19/19、独立审计PASS；G1 56/7、G2 15、adapter81/25、metadata361/362；账本1030条零缺失/过期/重复。未设正数最大天数限制。

## 2026-10-19 S003 cache nested-shape recovery
- Baseline reproduced AttributeError for `completeness:null`; provider_calls=0.
- Fixed cache loader/saver schema guard; malformed metadata becomes miss/no-op; provider recovery rewrites valid cache.
- China Stock Analysis suite: 23 passed; independent audit PASS.
- G1/G2/adapter/metadata all passed; adapter 81 passed, 25 superseded skips, using repository-local TEMP/TMP due system-temp sandbox denial.
- Evidence manifest finalized at 1038 unique cases; all evidence paths present and hashes current.

## 2026-10-19 S003 flat-series trend/RSI
- Baseline: 60 constant prices returned strong downtrend -3, RSI 100, overbought=true.
- Fixed zero-slope classification to sideways/0; zero gains and losses yield RSI 50; actual rising/falling cases preserved.
- Targeted 5/5, full China Stock Analysis suite 28/28; independent audit PASS including mixed/cross paths.
- G1/G2/adapter/metadata passed; ledger now 1046 unique evidence cases, no missing/stale hashes.
- Finite-window slopes remain strict without a tolerance band; real provider and natural routing remain untested.

## 2026-10-19 S018 Fund Screener finite threshold
- Legacy NaN threshold bypassed filtering and serialized non-standard NaN. Fixed threshold validation before provider access and strict JSON encoding.
- Independent first pass found Decimal direct-API issue; rejected non-int/float threshold types and added Decimal regression. Second pass PASS.
- Focused 2/2, Fund Portfolio suite 22/22; CLI NaN exit 1 with parseable JSON error.
- G1/G2/adapter/metadata passed; evidence ledger finalized at 1055 unique rows with current hashes.

## 2026-10-19 S018 Risk Assessor oversized JSON integers
- Baseline reproduced uncaught OverflowError/traceback for a JSON integer 10**400.
- _required_number now maps float conversion overflow to stable field-specific ValueError.
- CLI regression: exit 1, valid JSON error, no traceback/output, portfolio bytes and SHA unchanged.
- Focused 2/2, Fund Portfolio suite 24/24; independent audit PASS; G1/G2/adapter/metadata passed.
- Evidence ledger finalized at 1063 unique rows with no missing/stale hashes.

## 2026-10-19 S003 realtime minute-volume reconciliation
- Baseline: 09:25 auction volume entered total denominator but no bucket; 300 total vs 200 assigned.
- Added exclusive auction/open/morning/afternoon/close buckets; lunch and out-of-session bars excluded consistently.
- Focused 3/3, China Stock Analysis suite 31/31; independent audit PASS (baseline attribution corrected: 14:59 was already in close bucket).
- G1/G2/adapter/metadata passed; evidence ledger 1071 unique cases, no missing/stale hashes.
- Live upstream timestamp format remains unverified.

- 2026-10-19 S030 Meal Planner output-write failure baseline reproduced: nonexistent output parent returned exit 1 after persisting a plan; retry increased active plans 1→2. Fixed by staging/fsyncing output before DB write, atomic report replacement, and plan compensation on publication failure; rollback failure reports its concrete cause. Focused/full Meal Planner suite 22/22; independent audit and independent full rerun PASS 22/22. G1 56 Skills/7 Agents and 368 audit files with zero closure/hash mismatches; G2 15 checks; adapter 81 passed/25 superseded skips; metadata 366/367; sourceTree `fc0fd6b511ad44fb6f2120cc7d3c904bc88dfa65799d85b4569d48aa923c38a0`; evidence manifest 1079 unique records, no missing/stale paths/hashes. Crash between two file operations is not an atomic transaction; route/runtime remains unproven.

- 2026-10-19 S003 stock screener finite JSON: offline baseline showed optional latest price/change/PB NaN values passed through as invalid JSON `NaN`. Optional missing/non-finite numeric values now become null, non-finite sort rows are excluded, and main encoding uses `allow_nan=False` before output writes. Focused 4/4 and China Stock Analysis suite 35/35; independent audit and independent 35/35 rerun PASS. G1 56 Skills/7 Agents, 369 files/20 transient, zero mismatches; G2 15; adapter 81/25; metadata 367/368; sourceTree `add68f9cf32031a0ec7945d5fd76db5517d9d9f9a3524764d4f128845ae4b506`; evidence ledger 1089 unique IDs and all paths/hashes valid. Live AkShare, natural route, and USB runtime remain unproven.

## 2026-10-19 S003 technical_analysis 非有限 K 线输入
- 旧代码对末行 close=NaN 的60行合成序列返回成功并输出 JSON `NaN`。
- 现对 close/high/low/volume 在provider返回与直接指标入口校验有限数值；错误包含字段和1-based观测序号。CLI严格JSON；错误时不新建或覆盖输出。
- Focused 4/4，China Stock Analysis全套39/39，独立A0审计和独立全套复跑PASS。
- G1/G2/adapter/metadata通过：56 Skills/7 Agents，G2 15，adapter81通过/25旧fixture跳过，metadata368/369；sourceTree `9f30e42e719e0a6ba155a1a15f06af38e7429b8da9d5ac0a4574e711e68f0adc`。
- Evidence ledger 1099条，零缺失路径、重复ID或过期hash。
- 真实AkShare、自然路由与USB runtime未覆盖。

## 2026-10-19 S017 Fitness Coach 输出失败一致性
- 基线：缺失输出父目录使CLI退出1，但计划已写入DB；重试活动计划数1→2。
- 修复：先stage/fsync报告，再保存计划，最后os.replace发布；发布失败原子恢复programs.json，清理临时文件并返回稳定错误。
- Focused3/3、Fitness Coach全套37/37，独立审计/独立重跑PASS。
- G1 56 Skills/7 Agents、371 audit files/20 transient、零闭包差异；G2 15，adapter81通过/25 superseded skip，metadata369/370；sourceTree `719a932a6324ef6a92cc3864f11fe15aef3aefbc4c96dc9f1e301a2bc920f00f`。
- Evidence ledger 1110条，零缺失路径/重复ID/过期hash。进程强杀、并发DB写入、rollback失败注入及真实路由未覆盖。

## 2026-10-19 S037 PDF merge 部分写失败
- 旧目标先被`wb`截断，fake writer写入partial bytes后抛OSError；基线证明sentinel SHA改变。
- 现改为目标同目录暂存、flush/fsync/close、非空检查后os.replace；失败删暂存并保留旧目标。
- Focused2/2；PDF套件8通过、1硬链接测试因Windows ACL跳过；PDF smoke通过。独立审计PASS并完成双输入合并2页及抽取顺序核验。
- G1 56/7、372 audit files/21 transient、零差异；G2通过；adapter81/25；metadata370/371；sourceTree `7927b4c7714992e848933d0cae89b58998889157874f89233cbbe894bcd6f749`。
- Evidence ledger 1122条无缺失路径/重复ID/过期hash。强杀/断电、既有ACL元数据、硬链接真实权限、任意PDF和自然路由未覆盖。

## 2026-10-19 S045 Travel Planner 嵌套字段形状
- 旧CLI对destination null/list和budget null三种合成行程泄漏AttributeError traceback。
- 现对top-level trip_data/destination/budget校验Mapping，给出字段级错误；CLI非零且无traceback/输出；有效Mapping行为保留。
- Focused4/4、Travel Planner全套43/43；独立审计及独立suite复跑PASS。
- G1 56/7、373 audit files/21 transient零差异；G2通过、adapter81/25、metadata371/372、sourceTree `89a0bf2e844921ff1b8615e1c9fae2a919d86451ba814fe547329664d4563e57`；ledger1133条零缺失/重复/过期。

## 2026-10-19 S052 max_chars边界
- 旧 API 在401字符模拟正文下传-1返回400字符、传0返回空；已复现。
- 现API拒绝bool/非整数/非正值，CLI argparse在抓取前拒绝；正数仍精确截断。
- Focused3/3，全套10/10；独立审计PASS。G1 56/7/374 files/24 transient无差异；G2通过，adapter81/25，metadata372/373，sourceTree `2e9b1a77f0f0e88920df49750c94f516b329c0b7f5455bc95a549fdaa6749060`；账本1144条闭合。

## 2026-10-19 S047 尽调清单输出失败保护
- 旧write_text被注入截断、写prefix后OSError，原哨兵字节和SHA改变。
- 现同目录暂存、flush/fsync后原子替换；focused3/3、Markdown/JSON成功路径、Venture suite16/16。独立审计PASS并额外replace失败注入。
- G1 56/7、375 audit files/31 transient无差异；G2、adapter81/25、metadata373/374；sourceTree `52c7fad5568d64595bd808e9cd0a9504c535a9af3088a4a7ea0ac6fef47f141a`；账本1155条闭合。

## 2026-10-19 S054 封面扩展名副作用
- 旧代码对不存在父目录下的cover.txt先mkdir再ValueError；已复现。
- 现前置casefold后缀检查；unsupported无渲染/建目录，jpg/JPEG/PNG成功可重开。Focused2/2、全套14/14、独立PASS。
- G1 56/7、376 audit files/36 transient无差异；G2通过、adapter81/25、metadata374/375，sourceTree `f18b58e4e78a3dbc2eb1fd33a2c8485d6d89a8fc68d5023f76bc58dd2ecfebb9`；ledger1166条闭合。

## 2026-10-19 S003 data_fetcher safe_float overflow
- fake provider对总市值给`10**400`，旧get_stock_info泄漏OverflowError；现安全转为None，保留其他字段。
- Focused3/3、S003全套42/42；独立审计PASS。G1 56/7、377 files/37 transient零差异；G2通过，adapter81/25，metadata375/376，sourceTree `9889740c02991d9cc23cf03f8c44214ce7c6b1204bd7f6ab3da8ab9cfeb8ed7e`；ledger1177条闭合。

## 2026-10-19 S030 BMR gender字段验证
- 旧gender:null在CLI泄漏AttributeError；现拒绝缺失/null/非字符串/空/未知值，并保留male/female计算。
- Focused3/3、Meal Planner全套25/25；独立审计PASS。G1 56/7、378 files/37 transient无差异；G2通过，adapter81/25、metadata376/377，sourceTree `41a5e3ec15b92c95dfe448a3a79544eaa145d62084abcca7f5fa78e440b09e84`；ledger1188条闭合。

## 2026-10-19 S030 Meal Planner numeric profile boundaries
- Legacy CLI baseline with age=true, height_cm=false, weight_kg=false reproduced ZeroDivisionError and no report.
- Numeric body fields now require finite positive non-bool real values; derived BMR/TDEE/target and macro denominator also fail closed when unusable.
- Focused5/5; full Meal Planner suite30/30; independent read-only audit PASS, including persistent oversized-int and derived-infinity cases.
- G1: 56 Skills/7 Agents, 379 tree files/37 transient, zero closure/hash mismatches; G2 ok; adapter81 passed/25 superseded skips; metadata377/378 sourceTree b6e8088c9f2f61ddd3057484958b65184a58e83b0ca704ed49a011d7d1e4857f.
- Evidence ledger finalized at 1200 unique rows with every path/hash current. Synthetic profiles only; natural route, USB runtime source, and clinical suitability remain unverified.

## 2026-10-19 S054 draft filename collision baseline
- Fixed datetime.now and created the same sanitized title twice in isolated WECHAT_ARTICLE_DATA_DIR.
- Both calls returned the same path; second call truncated/replaced the first draft sentinel with a new template.
- No network or external account; remediation and acceptance contract frozen.

## 2026-10-19 S054 WeChat Article draft collision and partial-write safety
- Frozen-clock same-title baseline overwrote prior sentinel; injected post-exclusive-create OSError left a 50-byte partial second draft.
- Final implementation reserves with `Path.open("x")`, retries `_2`, `_3`, cleans only a path it created after write/close failure, and gives concise nonzero CLI error.
- Focused3/3, full WeChat Article Creator17/17; independent second review PASS; G1/G2/adapter/metadata pass; evidence ledger1214 rows, zero missing/stale paths/hashes.
- Narrow limitation: cleanup failure paired with non-OSError exception may not attach residual path; no real multiprocess stress. Natural route/USB runtime remain unproven.

## 2026-10-19 S051 weather config partial-write baseline
- Isolated `WEATHER_DATA_DIR` config seeded with valid BeforeCity JSON. Parent-scope Set-Content injection wrote `{"partial":` and threw IOException.
- Injection reached, command failure caught, SHA changed, JSON parse failed, only isolated config present; baseline reproduced. Source remains unchanged.

## 2026-10-19 S051 Weather Forecast config write-failure atomicity
- Baseline Set-Content injected partial invalid JSON and changed isolated config SHA.
- Write-Config now stages in same directory with GUID/CreateNew, then File.Replace/File.Move; failures clean owned stage/backup.
- PS5.1 and PS7 harnesses passed successful set/get, partial stage-write failure, locked target replace failure, unchanged config bytes/SHA, no stage leftovers; independent audit PASS and CLI exit1 verified.
- G1/G2/adapter/metadata pass: 56/7, 381 tree files/37 transient zero mismatches, adapter81/25, metadata379/380 sourceTree 16c55e13fdd06587b804447ce43bc82326558dfbbd589097aa0e764bc6962d0c.
- Evidence ledger1226 rows; no missing/stale paths or hashes. No real weather provider, natural route or USB runtime.

## 2026-10-19 S037 PDF generate partial-output baseline
- Fake Canvas was constrained to one explicit temp output path. Legacy direct final-path creation truncated sentinel and wrote a 27-byte prefix before injected save OSError.
- Failure caught, only isolated output changed; no real renderer. Frozen contract/harness/log saved; implementation pending.


## 2026-10-19 S037 PDF generate atomic publication
- Frozen fake-Canvas baseline: legacy target overwritten with partial bytes before injected save failure.
- Focused 2/2; PDF scripts suite 9 passed, 1 hardlink environment skip (WinError 5); local PDF smoke passed; independent audit PASS.
- G1/G2/adapter/metadata pass: 56/7, 383 tree files/37 transient, adapter81/25, metadata381/382, sourceTree `3c2922b8fb08f18babd3e3adc67368ab482d45cf4872426cd1f1738ae2dad92b`.

## 2026-10-19 S047 business evaluator oversized integer
- Frozen 401-digit `market.tam` baseline leaked OverflowError traceback and did not create output.
- Fixed numeric conversion to field-scoped ValueError; CLI exit1 without traceback/stdout/output; finite input accepted. Focused3/3, Venture suite19/19, independent audit PASS.
- Same package gate run as S037 above; evidence ledger finalization pending.


## 2026-10-19 S045 trips.json top-level shape
- Legacy valid-JSON [] database leaked TypeError traceback in CLI lookup; isolated DB hashes unchanged and no plan output.
- Added object-shape validation at load boundary and concise CLI handling. Focused2/2, full Travel Planner45/45, independent audit PASS; valid Mapping still generates plan.
- G1/G2/adapter/metadata pass: 56/7, 384 tree files/37 transient, adapter81/25, metadata382/383, sourceTree `fd929011b0280b8c916816b21855ec58f4a5612f4c2409e87f89f9f8e35c6c1d`.


## 2026-10-19 S027 cycle allocation oversized integer
- Legacy 401-digit reviewed allocation leaked OverflowError traceback; output absent and rules SHA unchanged.
- `_number()` maps OverflowError to field-specific ValueError. Focused2/2, Macro Research16/16, independent PASS; finite allocation accepted.
- G1/G2/adapter/metadata passed: 56/7, 385 tree/37 transient, adapter81/25, metadata383/384, sourceTree `a007241c3107220f7f5584001f21e2a34c8d2ccad3dabd810c0d82e9bc710a9a`.


## 2026-10-19 S003 realtime minute volume finite parsing
- Fully mocked provider baseline `volume=1e10000` leaked OverflowError; JSON error envelope absent and no network.
- Parser now rejects nonfinite/negative before int conversion and uses stable CLI envelope; zero/positive remain valid. Focused4/4, China Stock suite46/46, independent PASS.
- G1/G2/adapter/metadata pass: 56/7, 386 tree/37 transient, adapter81/25, metadata384/385, sourceTree `a0312956af098f3618ec5bb5efaf97247ea43befa5f3c95d074f2a805d60a040`.


## 2026-10-19 S003 minute amount aggregate finite guard
- Fully mocked baseline: two finite 1e308 amounts emitted nonstandard Infinity with exit0/success=true; strict JSON rejected it.
- Aggregate now fails with stable ValueError envelope; normal total20 and volume buckets pass. Focused2/2, full suite48/48, independent PASS.
- G1/G2/adapter/metadata: 56/7, 387 tree/37 transient, adapter81/25, metadata385/386, sourceTree `f63b39b1a2fb6be6d0bb6d1cf503496c5b942a0446105cfab5c18a5cd76f7454`.


## 2026-10-19 S045 persisted trip row shape
- Baseline `current_trips:[null]` leaked AttributeError traceback; DB hashes unchanged and no plan output.
- Lookup now requires object rows before accessing ID; focused2/2, suite47/47, independent PASS; valid row generates plan.
- G1/G2/adapter/metadata pass: 56/7, 388 tree/37 transient, adapter81/25, metadata386/387, sourceTree `077ede912405e5ab040d14d6c8630382c3560150e8a3cb3c416b2915dd360f7e`.


## 2026-10-19 S030 vegan filter fail-closed behavior
- Baseline vegan profile generated tea egg, egg and fish because diet_type was ignored.
- Filter now excludes whole recipes with known animal-derived ingredients, requires ingredient detail, and fails clearly when none remain; no ingredient stripping. Regular control unchanged. Focused4/4, full34/34, independent PASS.
- Residual usability gap: current Chinese breakfast recipe catalog has no vegan-safe option, so the daily vegan plan still cannot succeed. Keto/low-carb/vegetarian semantics not implemented.
- Package G1/G2/adapter/metadata pass: 56/7, 389 tree/37 transient, adapter81/25, metadata387/388, sourceTree `7e86b62e807e01c430d68872d1346c349a604cf1ecf6bc4be43515f65cc9b561`.

2026-10-19 S030 vegan availability focused: 5/5; Meal Planner: 35/35; independent behavior audit PASS; G1/G2/adapter/metadata PASS; evidence manifest 1310 unique, missing0/stale0. Residual nutrition adequacy not established (sample ~1109 kcal/31g protein against illustrative 2000/100 target).

2026-10-19 S049 tracker huge integer: focused3/3; Wealth Allocation28/28; independent audit PASS; G1/G2/adapter/metadata PASS; evidence manifest1321 unique, missing0/stale0.

2026-10-19 S019 off-token: focused4/4; Grok suite16/16; compiler29/29; upload5/5 mock; independent PASS; G1/G2/adapter/metadata PASS; ledger1334 unique, missing0/stale0.

2026-10-19 Fund Screener atomic output: focused3/3; Fund Portfolio27/27; independent PASS; G1/G2/adapter/metadata PASS; ledger1345 unique, missing0/stale0.

2026-10-19 Web Fetch missing optional dependency: focused3/3; full13/13; independent PASS; G1/G2/adapter/metadata PASS; ledger1356 unique, missing0/stale0.

2026-10-19 Valuation output alias: focused4/4; Venture Analysis23/23; independent PASS; G1/G2/adapter/metadata PASS; ledger1367 unique, missing0/stale0.

2026-10-19 sentiment_monitor alias: focused4/4; Macro Research20/20; independent PASS; G1/G2/adapter/metadata PASS; ledger1378 unique, missing0/stale0.

2026-10-19 Skills Search installed fallback: PS5.1+PS7 focused PASS; G1/G2/adapter/metadata PASS; ledger1389 unique, missing0/stale0.

2026-10-19 Financial Analyzer atomic write: focused3/3; China Stock Analysis51/51; independent PASS; G1/G2/adapter/metadata PASS; ledger1400 unique, missing0/stale0.

2026-10-19 policy text shape: focused3/3; Macro Research23/23; independent PASS; G1/G2/adapter/metadata PASS; ledger1411 unique, missing0/stale0.

2026-10-19 S052 truncation disclosure: focused5/5; CLI/dependency3/3; suite15/15; independent behavior PASS; G1 56/7 + 405/406 closure; G2 16 files PASS; adapter81/25; metadata check PASS; diff check PASS; evidence1420 unique, missing0/stale0. PDF partial-output candidate reviewed as proposed contract only, not confirmed violation.

2026-10-19 S019 reference-upload HTTPS redirects: baseline reproduced verified=true after HTTPS→HTTP redirect; focused7/7; Grok suite16/16; independent audit PASS; G1/G2/adapter81/25/metadata/diff-check PASS. Fake requests only, no network. Uguu live redirect not established.

2026-10-19: S045 date3/3+Travel53; S030 shopping restrictions11/11+Meal56; S003 partial persistence6/6+China62; Feishu node floor2/2+adapter83 pass/25 skips; all independent audits PASS. Final package G1/G2/metadata/adapter and diff check PASS; evidence ledger finalization pending this wave.

2026-10-19 package final wave: G1/G2/adapter/metadata/diff gates PASS; 56/7, 408/409, adapter83/25 skipped. Evidence ledger1476 unique, missing0/stale0. Authorized three Meal Planner data files are absent.

2026-10-19 Painpoint/S045 final package refresh: G1 56/7, 408/409 closure; G2 Windows PowerShell5.1 16 files; adapter84/25 skips; metadata and diff-check pass. Painpoint focused1/1, S045 focused4/4 & suite54/54; independent reviews pass.

2026-10-19 S037 atomic output: focused4/4; reviewer second audit PASS; PDF helper10/1 ACL skip; smoke PASS; final gates G1/G2/adapter/metadata/diff PASS; 409/410 closure.

2026-10-19 S049 normalized benchmark collision: focused2/2, Wealth Allocation30/30, independent CLI preserves output; G1/G2/adapter/metadata/diff all pass; 410/411 closure.

2026-10-19 S038 RGBColor docs: focused2/2 and locked-wheel PPTX structure PASS. S012 Excel scope: focused4/4, workbook7/7. Independent reviews PASS; final gates 412/413, adapter84/25.

2026-10-19 current-source provenance refresh: 23/56 Skills match at least one local copy; 33 differ everywhere; Agent TOMLs 4/7 exact. Authorized Meal Planner cleanup removed only three named default files and verified absence. Package gates remain G1 56/7, 412/413; G2 16 PowerShell files; adapter84/25 expected skips.

2026-10-19 S002 focused3/3; S0068/8; S022 PS5.1 version-health8 scenarios; S053 max-words2/2; all independently reviewed PASS. G1 56/7,416/417; PS5.1 17; dedicated media MCP64/64. Full adapter runner remained incomplete with observed app-side errors, not reported as PASS. sourceTree 3e4e71ad6d610357d53166cb8eb965c4c4e162947490321b568a29602dddec47.
2026-10-19 S010: focused static contract 5/5; independent read-only audit PASS; report clarifies no natural-route/runtime interception evidence. Package G1 56/7, 417 manifest files/418 checksums; PS5.1 17 files; media MCP64/64; metadata and diff-check PASS. Evidence 1541 unique, missing0/stale0. Full adapter suite still PARTIAL/incomplete. sourceTree 3d520f24305db1d09ec0070d399b813ed58750f0bd77f4307dbdebc40b1564e0.
2026-10-19 G2 recovery: `test-adapter-contracts.py` 84 passed, 25 expected skips when `TEMP/TMP` point into workspace `tmp`; metadata 417/418 PASS. Prior permission failures were sandbox temp placement. G2 current-source status PASS.
2026-10-19 S010 password guidance: focused4/4 and independent PASS; NIST citations recorded. Final gates G1 56/7, 418/419; PS5.1 17, media64/64, adapter84/25 skipped, metadata/diff PASS. Evidence ledger 1553 unique; missing0/stale0; sourceTree b89b3ccc11cd85b0147bd8746f06b4dc8a0d38ca7f65830b14406d53bbfe668a.
2026-10-19 S040: recursive reparse safety source contract6/6 and independent review PASS; no real junction runtime tested. Gates G1 56/7, 419/420; PS5.1 17, media64/64, adapter84/25 skipped, metadata/diff PASS. Evidence ledger 1562 unique, missing0/stale0.

2026-10-19 S029 weighted SAM/ARPU: focused3/3, original source smoke PASS, independent PASS. S046 inference/privacy boundaries: focused4/4, independent PASS. Neither static proxy proves natural route or real-world evidence.
2026-10-19 compatibility audit marker: AST focused5/5 including multiline/comment/docstring edge cases; compatibility audit15 checks PASS, no warnings/errors; independent review PASS.
2026-10-19 final package: G1 56 Skills/7 Agents, 422 manifest/423 checksums; PS5.1 17 files; compatibility15 checks; media MCP64/64; adapter84 passed/25 expected skips; metadata/diff PASS. G0 18/56 current local Skill trees match, Agent TOMLs4/7. Evidence ledger 1586 unique, missing0/stale0; sourceTree 66b18db14b98a713ed0d955cbf33872ac539be2d7fbd38715e9fb2abf2610a44.

2026-10-19 final focused rerun: S029 formula3/3; source smoke confirms6 refs/3 synthetic examples; S0464/4. Evidence ledger finalized: 1589 unique IDs, missing0/stale0.

2026-10-19 S029 final structure regression: focused4/4; source smoke PASS (6 refs, 3 synthetic examples); independent review PASS (all 9 Markdown fences paired). Current package checks remain G1/G2/metadata/diff PASS, evidence1600 unique/missing0/stale0.

2026-10-19 S031/S035 remediation: both focused contracts 4/4; independent read-only review PASS. Current pack: G1 56/7, metadata424/425; compatibility15, AST markers5/5, PS51 17 files, media64/64, adapter84 passed/25 expected skips, metadata/diff PASS. G1 tree SHA 573380093852b60fe81043b7ef1b349a193aa9843f95c29e5fc4899b8d6cc0b5; pack sourceTreeSha256 3061d38e9d23e839820f4fc0b070232605e7542a0615cfc7617fd184bf511fb6.

2026-10-19 S036/S050: focused5/5 and6/6; independent reviews PASS. G1 56/7, manifest426/checksum427; G1 tree b4f8577132780f71e2c2844389718e87b15697f066ea18474bddc30cdd671ee4; package sourceTree 186732dddefb34bdfd9ef97ed40197d6d0dde231acceaa8f805a9fc11549bb2e; compatibility15/markers5, PS51 17, media64, adapter84/25 expected skips; metadata/diff pass.

## 2026-10-19 S024 image-prompt-generator 源修复与验收

- 源合同测试 9/9；独立只读复核 PASS。修复了事实证据与品牌授权边界、参考图能力门控、风格库计数/别名/矩阵一致性、2:3 比例缺项、默认写盘、生成失败时文件输出/工具回退歧义及简单任务强制追问。
- G1：56 Skills / 7 Agents，429 文件、96 transient excluded，零闭包/哈希差异；树 SHA `b584580b69c1448cd45b92a6347ec7a2408024d43d88237b6f2c13c7381709da`。
- G2：Codex compatibility 15/15；Windows PowerShell 5.1 17 文件；media MCP 64/64；adapter contracts 84 passed / 25 expected skips。Metadata 427/428、sourceTree `79a89f4e8cb75181db213105f2a82367064d1479d02f93da0c5ea9c8c12ef2d0`；git diff --check 通过（仅既有换行符提示）。
- Evidence manifest：1632 unique，missing=0，stale=0。S024 整体仍 PARTIAL，未证明自然路由、USB runtime、参考图实传或真实上游出图。

## 2026-10-19 S048 video-prompt-generator 源修复与验收

- 源合同测试 6/6；独立只读复核 PASS。时间段半开区间覆盖 `[0,T)`，短时长降级为单镜头/减少分镜；规格参数按端点能力门控；人物/艺术家/品牌授权边界和章节结构检查已补齐。
- G1：56 Skills / 7 Agents，430 文件、97 transient excluded，零闭包/哈希差异；树 SHA `67cfbb991e057a37a5ab627f0c80f24a8a9f2106638ae4176d191a0c5d1aebfa`。
- G2：Codex compatibility 15/15；Windows PowerShell 5.1 17 文件；media MCP64/64；adapter84 passed/25 expected skips。Metadata428/429，sourceTree `0907d0e9104ecdb23c5da4b844f35e2d94e9a130f4d6bf5373b426970ba0fec3`；git diff --check通过（既有换行符提示）。
- Evidence manifest：1636 unique，missing=0，stale=0。S048仍PARTIAL，未证明自然路由、USB runtime、端点真实支持或实际视频效果。

## 2026-10-19 S056 xiaohongshu-writing
- Focused source-contract tests: 9/9 PASS (`S056-focused-source-contract-20261019.log`).
- Independent source review: PASS (`independent-audit-S056-source-remediation-20261019.md`).
- G1/G2, PowerShell 5.1, media MCP 64/64, adapter 84 passed / 25 expected skips, metadata and diff-check passed.
- S056 remains PARTIAL: natural routing, USB runtime provenance and real writing task not verified.

## 2026-10-19 S055 word-docx
- Source contract: 5/5 PASS (`S055-focused-source-contract-20261019.log`).
- Synthetic DOCX structural roundtrip: 1/1 PASS (`S055-docx-structural-smoke-20261019.log`); system Python 3.12.10 / python-docx 1.2.0, not USB-provisioned runtime.
- Independent review: PASS; reviewer corrected a prior `Path.parents` indexing error after repository containment was verified.
- G1 56/7, 431 manifest files/432 checksums; compatibility 15 checks; metadata and diff check pass. S055 remains PARTIAL for natural routing, USB runtime provenance and revised-source visual QA.

## 2026-10-19 S013–S016 Feishu 静态契约加固
- python -B -m unittest tests.test_feishu_skill_contracts -v — PASS 8/8。installer-pack/tests/test_feishu_skill_contracts.py 精确闭包工具识别、分类、危险写入授权、fail-closed、allowlist 和 approval overrides；拒绝注释/重复默认键、重复工具审批键和额外弱审批覆盖。最终独立复核 PASS。
- G1 — PASS: 56 Skills/7 Agents, 432 manifest files, no file/hash/checksum mismatch.
- G2 — PASS: 15 checks, zero warnings/errors.
- metadata check — PASS: 432 manifest / 433 checksum rows; sourceTree `2860cce3a8f6e654fb454d6780d1134680519ed4954e18217b18c1dde580ea90`.
- git diff --check — exit 0.
- Live tool inventory returned no Feishu/Lark tools. Static test does not prove package tools/list, tenant credentials, Codex approval enforcement, natural route or USB runtime; S013–S016 remain NOT_TESTABLE.

2026-10-19 S028 route contract rerun 2/2 PASS; checklist reconciliation corrected aggregate to 9 PASS / 42 PARTIAL / 0 FAIL / 5 NOT_TESTABLE; G1 latest package metadata 433/434.
2026-10-19 S028 negative vector: text-only explanation observed, 3^2+4^2=5^2 correct, no render/write under read-only sandbox; source/local runtime hashes match; exit code unavailable; evidence ledger 1654 unique, missing0/stale0.
2026-10-19 G4 isolated runtime retry: source TOMLs staged 7/7; API 401 missing bearer; no child events; exit1. Do not copy user auth into isolated home. Evidence ledger 1655 unique, missing0/stale0.

2026-10-19 scope/evidence reconciliation: finalize-evidence-manifest PASS (1669 unique IDs, all paths present/hashes refreshed); verify-evidence-manifest PASS (missing=0, stale=0); current Codex compatibility audit PASS (15 checks, ok=true, warnings/errors empty); package static audit PASS (56 Skills/7 Agents, no closure/hash mismatches); pack metadata --check PASS. S052 remains PARTIAL only for external website quality/reachability, natural route, and USB provisioner lifecycle.

2026-10-19 S054 natural route attempt disqualified for USB-source evidence: Codex CLI loaded a divergent same-name user-profile skill; `--sandbox workspace-write` still yielded read-only behavior and no draft. Recorded in S054-natural-route-source-attribution-20261019.md; no source files changed.

2026-10-19 G0 hash-only source/local comparison: PASS; 56 Skills -> 16 exact, 40 differing, 0 missing; 7 Agent TOMLs -> 4 exact, 3 differing. Current sourceTreeSha256 c928d399...f3b037. Evidence G0-current-local-proxy-equivalence-post-S052-20261019.json.
2026-10-19 S056 natural route pair: project-local source tree matched USB files 3/3; positive trace read SKILL.md, unrelated HR email negative did not. Overall PARTIAL. S054 route attempt is inconclusive because trace read divergent CODEX_HOME content.

2026-10-19 S031 current-source route pair: positive used frozen task + approved synthetic fixture (not a byte-identical full prompt); positive trace read current project source, negative exact vector did not; exit 0/0. Focused contract 4/4; G1 metadata check 442 manifest / 443 checksum entries, sourceTree `c928d3991293b7201114da9dbfddc7f1d106de600722ffbd501b53fcd6f3b037`; independent review PASS with caveat that negative answer included an unnecessary but clearly fictional example.

2026-10-19 S048 exact frozen prompt pair: 4/4 source tree matched; positive project Skill read, negative no target Skill, both completed exit0; no MCP/media generation. Focused contract6/6, independent review PASS. Generic plugin/model-catalog timeout warnings non-blocking.

2026-10-19 S006 exact frozen pair: current source 2/2 hashes matched; positive project Skill read; negative no target Skill; both exit0. Direct focused contract 8/8. Independent review PASS. No deployment or external tool calls.

2026-10-19 S035 positive-only smoke: explicit vector; source read from harness, 3 files matched; useful Offer output; exit0; independent PASS. No negative or boundary tests.

2026-10-19 S036/S024 current-source positive usability smokes: one ordinary task each, exact source copies read in project harness, both exit 0 and independently scope-reviewed PASS. No negative, security, permission, or edge-case tests added. Checklist 16 PASS / 35 PARTIAL / 5 NOT_TESTABLE; evidence ledger 1769 unique, missing0/stale0.

2026-10-19 S046 current-source positive usability smoke: one ordinary task, exact source copied/read, exit 0 and independent scope review PASS. Default CLI attempt did not start a turn and remains recorded as environment-only; one authorized invocation completed the same prompt. No security/negative/boundary test added.
