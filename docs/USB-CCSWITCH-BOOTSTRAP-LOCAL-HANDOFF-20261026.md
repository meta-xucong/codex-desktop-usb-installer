# CCSwitch USB 首次绑定修复：本地接手记录（2026-10-26）

## 修复内容

在 `release/ccswitch-source-v4.0.20-clean-r2-20261026/src-tauri/src/usb_model_selection.rs` 的 `prepare_usb_bootstrap_projection_at` 中，临时 provider row 的 model/reasoning 叶子现在以 live `config.toml` 为准。fresh row 不需要已有 model；旧 row model 不得覆盖 live model；live 缺 model 或 reasoning 时，从临时 row 删除该叶子，不能把数据库旧值复活。row/live TOML 类型校验仍保留。reasoning 从 live 复制时不再要求 live 已有 provider route。

保留 catalog 对 live model 的校验，以及现有 catalog/hash/竞态、认证、普通 provider switch 和模型选择 CAS 路径。本改动不写数据库模型，也不复制整份 live 配置或注入默认模型。三个新的端到端用例位于 `src-tauri/src/services/provider/usb_bootstrap.rs`，只使用合成凭据。

## Windows 验证记录

- 原 AppContainer TEMP 上下文中的 `cargo test --locked --lib usb_404token_bootstrap_ -- --test-threads=1`：10 项，3 passed、7 failed。失败在 `live-state.json.tmp -> live-state.json` 原子替换处收到 Windows `PermissionDenied`（OS error 5）。
- `cargo test --locked --lib usb_model_selection::tests -- --test-threads=1`：17 passed、0 failed。
- 原 AppContainer TEMP 上下文中的 `cargo test --locked --lib usb_upgrade_ -- --test-threads=1`：23 项，17 passed、6 failed。失败处收到 Windows `PermissionDenied`（OS error 5）。
- `cargo check --locked --tests`：通过。仅有既有 `SystemKeychain::named` dead-code 警告。

### 同一测试 EXE 的非提权桌面上下文对照

经工具原生单次审批机制，在普通非提权桌面用户上下文使用同一已编译 test EXE（未重新编译）复跑两组原过滤器；源码、测试断言与单线程设置不变。运行前核对 EXE SHA-256，并确认当前 token 为非 AppContainer、非提权、Medium IL：

- EXE：`release/ccswitch-source-v4.0.20-clean-r2-20261026/src-tauri/target/debug/deps/cc_switch_lib-f49e61fe93dd5e4c.exe`
- SHA-256：`18A5906C7D75AC430DD74DC3AD3D00253A74632A9A9A2C26A3A05E9AD35152D2`
- `usb_404token_bootstrap_ --test-threads=1`：10 passed、0 failed，exit code 0。完整日志：[desktop-usb-bootstrap-20261026.log](../.test-tmp/desktop-usb-bootstrap-20261026.log)
- `usb_upgrade_ --test-threads=1`：23 passed、0 failed，exit code 0。完整日志：[desktop-usb-upgrade-20261026.log](../.test-tmp/desktop-usb-upgrade-20261026.log)
- 执行用户/token/TEMP 记录：[desktop-usb-context-20261026.log](../.test-tmp/desktop-usb-context-20261026.log)。该轮 TEMP 为 `C:\Users\T14S\AppData\Local\Temp\`；`AppContainer=False`、`Restricted=False`、`Elevated=False`、Medium IL；Administrators 组为 deny-only。
- 取证报告及原始 AppContainer 失败分析：[usb-permission-diagnosis-20261026.md](../.test-tmp/usb-permission-diagnosis-20261026.md)。

桌面上下文两组通过、原 AppContainer TEMP 上下文两组失败，支持失败与**进程上下文和 TEMP 的组合**有关；具体 Windows 拒绝机制仍未确定，不能仅据此归因于某个 ACL、安全策略或源码缺陷。原 AppContainer 结果仍保持失败记录，不将桌面对照冒充原上下文通过。

## 源补丁和产物状态

- 完整产品 overlay：`src-tauri/resources/ccswitch/source-patches/ccswitch-usb-product-85ad4969.patch`
- 上游固定基线：`85ad4969b3399500baaea9d46b1e08f87aba80e1`（4.0.20）
- 补丁 SHA256：`69B135C95A04F06CDF8ED2002E0D3224E43219AF5A101299A786A0A7966210CA`（与 manifest / SOURCE-NOTICE 一致）。
- builder 现有过滤器计数同步为 bootstrap 10、model selection 17、upgrade 23。
- manifest 的 `artifact.buildStatus` 为 `pending-rebuild`。旧 `CodexSwitch.exe` 及其 SHA256/sourceVersion/sourceCommit provenance 字段保持不变；本任务未构建 EXE、签名、部署或生成安装包。

本地接手边界：本机普通非提权上下文的两组复跑已通过，可交回原 localCodex 按既有流程重建/签名；实际设备验收仍待完成。`artifact.buildStatus` 继续为 `pending-rebuild`。旧 `CodexSwitch.exe` 未改，本轮也未重新构建、签名、部署或打包。无需扩展产品功能或新增构建门禁。
