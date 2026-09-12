# U 盘签名授权 GUI 审计附录

- 审计日期：2026-09-11
- 审计对象：`tools/usb-license-signer/`、`tools/build-usb-license-signer-gui.ps1`、GUI 发布目录和配套操作文档
- 结论：代码级自审通过；U 盘内部授权模式有条件通过
- 独立审计状态：未完成，不能宣称独立审计通过

本轮用户复现的“EXE 双击无窗口”已定位为 Win32 控件创建期间持有 GUI 状态锁，Windows
同步回调 `WM_COMMAND` 后再次取同一把锁造成窗口线程死锁；修复为控件创建不持锁、控件
刷新通过 `WM_APP` 异步消息执行。

## 1. 审计边界

本附录只审计新增的独立 Windows GUI、CLI/GUI 共享管线、签名并发保护、GUI 打包和操作文档。USB License schema、运行时 signed gate、安装/更新/卸载门禁的历史审计结论仍以 `docs/USB-LICENSE-SIGNER-AUDIT.md` 为准。

本轮结论仍为“主控自审”，不宣称独立 Agent 签字通过。新增 GUI 事件队列和控件刷新修复已完成代码级复核与 3 次冷启动回归。

## 2. 代码级检查结果

| 检查项 | 结果 | 证据 |
| --- | --- | --- |
| CLI/GUI 是否使用同一签名实现 | PASS | GUI 调用 library 导出的 `run_with_args`；CLI 入口也只调用该函数 |
| 单候选、可移动 NTFS、路径/重解析点门禁 | PASS | GUI 只消费 signer `scan`；sign 再次执行 scan/observe，底层拒绝非目标介质 |
| 深度 Bundle 校验 | PASS | GUI 只有 `deepVerify.ok=true` 才启用按钮；底层 sign/verify 在 GUI 模式仍强制 Manifest、Payload、SHA256SUMS、路径和结构检查，仅跳过 PE/MSIX Authenticode |
| 精确确认与目标复核 | PASS | GUI 显示卷序列号、Anchor、License、Payload、KeyId，并传递精确 `SIGN <serial>-<anchor前12位>`；底层写入前后重观察 |
| 私钥隔离 | PASS | GUI 只保存 KeyId/Blob 路径；sign 只读取本机 DPAPI Blob；没有明文私钥参数或 U 盘私钥路径 |
| GUI/CLI 并发写入 | PASS | `Local\\CodexUsbLicenseSigner.Signing` named mutex 覆盖完整签名事务；第二个事务立即失败 |
| 后台任务和窗口关闭 | PASS | 扫描/检查/签名在线程中执行；操作中禁止关闭窗口；完成通过 `WM_APP` 回到 UI 线程 |
| 后台完成事件不丢失/不覆盖 | PASS | `pending` 改为 `VecDeque<WorkerEvent>`，扫描、深检和签名事件按序消费 |
| 最终状态刷新不依赖锁内同步回调 | PASS | 状态锁释放后直接在 GUI 线程应用快照；文本控件使用 `WM_SETTEXT` |
| 发布目录安全 | PASS | 构建脚本拒绝仓库/target 覆盖和非空输出目录，复制固定文件，生成无 BOM manifest/SHA256SUMS |
| 当前未签名开发包处理 | PASS | 干净 Bundle 在内部授权模式可通过；含 `apps/Codex` 的已安装运行树仍 fail-closed；文档明确 `-SkipSignature` 不是正式发行证明 |

代码级 P0/P1/P2/P3 问题：0。上述结论是主控只读复核，不是独立 Agent 签字。

## 3. 可复现证据

工作区：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

```text
cargo fmt --manifest-path tools/usb-license-signer/Cargo.toml -- --check  PASS
cargo check --manifest-path tools/usb-license-signer/Cargo.toml --bins  PASS
cargo clippy --manifest-path tools/usb-license-signer/Cargo.toml --all-targets -- -D warnings  PASS
cargo test --manifest-path tools/usb-license-signer/Cargo.toml  PASS: 22 core + 3 GUI
cargo build --release --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui  PASS
tools/build-usb-license-signer-gui.ps1  PASS
发布目录 SHA256SUMS 与 manifest 哈希回读  PASS
manifest 标记 `verificationMode=usb-internal-license-only`、`systemAuthenticodeRequired=false`  PASS
GUI release 进程启动后存活 3 秒并被测试进程关闭  PASS
修复后 GUI 进程 2 秒内获得有效主窗口句柄且标题为“Codex U 盘签名授权”  PASS
最终 GUI 包连续 3 次冷启动  PASS：`signingReady.ok=true`，签名按钮 enabled=true
usb-license-signer scan  PASS: candidateCount=1（当前 F:）
干净 readiness Bundle 内部模式深度验证  PASS（结构/清单/哈希/Payload；Authenticode skipped）
已安装运行树内部模式 inspect  FAIL-CLOSED: exit 1（apps/Codex 不可作为待签 Bundle）
```

测试没有执行 sign 写入，没有创建或使用生产密钥，没有改变 U 盘文件。

最终可双击目录：

```text
D:\AI\AGENT_U盘\codex-desktop-usb-installer\release\usb-license-signer-gui-final-20260911\
```

## 4. 正式发行阻断项

这些是外部发行条件，不是 GUI 代码缺陷：

1. 本次只执行了真实 U 盘的只读 `inspect`；尚未执行真实 sign/verify 写入，以避免在用户当前 U 盘上未经确认创建 `usb-license.json`。
2. 当前 GUI 发布 EXE 自身是 `NotSigned`；这不影响本次“只验证 U 盘内部签名”的功能目标，但正式分发前仍应使用组织代码签名证书签署 GUI。
3. 尚未在真实可移动 NTFS U 盘上完成盘符变化、拔插/替换介质、安装、启动、更新和卸载证据。

因此当前 GUI 可作为“只做 U 盘内部授权”的可双击工具交付；要宣称真实签名成功，仍必须在干净 Bundle U 盘和匹配 DPAPI Blob 上完成一次 sign → 回读 → verify。
