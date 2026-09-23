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
