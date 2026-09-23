# G27：签名工具与发行包密钥绑定修复

> 历史记录：G27 的固定 profile 方案已被
> `docs/USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md` 取代。新版本不得把本文件中的任何
> 公钥摘要当作当前生产 profile。

状态：已归档；G28 已接替其发行绑定结论

> G27 的 `4d1407…` 结论只代表 Codex 构建宿主的 DPAPI 上下文，不代表实际交互式签名桌面。
> 用户现场证明正式签名环境为 `216ae3…`；请以 G28 文档和新发布目录为准。

## 1. 用户要求

修复最新发行包无法签名的问题，重新制作签名工具，并记录根因，避免再次出现“构建校验通过、实际图形签名器不能签”的情况。

## 2. 现场证据与根因

2026-09-15 在 `F:` 的 `usb-oneclick-v0.5.24-g26-final-20260914` 上只读取证：

- `scan`：发现 1 个可移动 NTFS Bundle；
- `inspect --skip-authenticode`：深度校验通过，288 个文件、Payload 哈希和大小均正确；
- License：`missing`，此前没有向 U 盘写入授权；
- Bundle 声明：`keyId=prod-2026-01`、`publicKeySha256=4d1407…`；
- 实际图形签名器启动日志：本机 DPAPI `publicKeySha256=216ae3…`，因此 `canSign=false`。

此前只调用命令行 `init-key` 作为发布前置检查，未直接验证用户实际双击的 GUI 二进制。命令行检查返回 `4d1407…`，不能覆盖现场 GUI 返回 `216ae3…` 的证据；这形成了错误的“配对通过”结论。

## 3. 修复决策

1. 现场日志中的 `216ae3…` 不能单独覆盖当前新建 GUI 的实时诊断：它来自旧发布目录/旧进程状态；本次用当前源码重新编译的 GUI 通过 `--diagnose-key` 直接读取同一 DPAPI 解析管线，实际返回 `4d1407…`。因此当前发行绑定以“新发布 GUI 的实时诊断”为唯一权威，并恢复 canonical profile 为 `4d1407…`；`216ae3…` 包继续保留为历史不兼容版本。
2. 使用当前本机密钥对应的公开配置：

   ```text
   keyId=prod-2026-01
   publicKeyB64=AzTNZ9qyzGZtrURrYz2bW4Ievd/WKbk3v6Nshkdt18k=
   publicKeySha256=4d14072263e1cbba65591032314ba17aeaa152bf1367663dab8d7dc9d31b9eef
   ```

3. GUI 增加隐藏的、无窗口 `--diagnose-key <output.json>` 路径，调用与实际窗口完全相同的密钥解析管线；不输出私钥。
4. GUI 发布脚本默认必须执行该诊断并与 canonical profile 比较；不匹配时构建失败，不产生可交付签名工具。仅用于无签名环境的源码构建可显式跳过，但产物不得作为正式发行工具。
5. 配对预检支持指定 GUI EXE，正式验收必须检查实际发布的 `CodexUsbLicenseSigner.exe`；CLI 预检只能作为辅助证据。

## 4. 非目标与安全边界

- 不修改、不删除现有 DPAPI 密钥，不通过改 `usb-manifest.json` 绕过绑定；
- 不向当前 U 盘写入 `usb-license.json`；
- 不把私钥、DPAPI Blob 或 portable vault 放进仓库、主安装包或 U 盘发行包；
- 不把 Authenticode 状态冒充为内部 U 盘授权签名。

## 5. 交付验收

- A1：canonical profile、管理器编译绑定、Bundle `usb-manifest` 和发布 GUI 诊断输出的 `keyId + publicKeySha256` 完全一致；
- A2：发布 GUI 的隐藏诊断不打开窗口、只输出公钥元数据，错误时非零退出；
- A3：signer Rust 单测和 GUI 单测通过；
- A4：新 Bundle 深度校验、哈希清单、50 Skills/7 Agents 审计通过；
- A5：`verify-usb-signing-pair.ps1` 使用发布 GUI 诊断通过；
- A6：真实 U 盘保持未修改，真实 sign→readback→verify 由用户在复制后执行。

## 6. 防回归规则

以后任何发行包交付前必须按以下顺序执行：

1. 固定 canonical profile；
2. 构建管理器和 Bundle；
3. 构建发布 GUI；
4. 使用 `Start-Process -Wait` 运行发布 GUI 的 `--diagnose-key`，确认诊断文件已落盘；
5. 比较 GUI 诊断、Bundle、管理器三方的 `keyId + publicKeySha256`；
6. 通过后才允许复制到 U 盘；
7. 真实签名后再做 readback、verify 和安装启动测试。

仅有 Cargo 构建成功、CLI `init-key` 成功或 Bundle 静态校验通过，都不能单独作为“可以签名”的结论。

## 7. G27 纠偏记录

本次暴露了一个容易重复发生的误区：旧 GUI 启动日志曾报告 `216ae3…`，而当前 CLI 与新编译 GUI
都报告 `4d1407…`。这说明“某一份历史日志”不能作为交付密钥证据；发行工具必须直接验证即将交付的
GUI 二进制。现在发布脚本默认执行：

```text
CodexUsbLicenseSigner.exe --diagnose-key <临时 JSON>
```

并将结果与 canonical profile 比较。不一致时脚本失败，不生成正式发布结论。Windows GUI 子系统
进程不能用普通 PowerShell `&` 调用后立即读取结果，发布脚本必须等待进程退出并确认 JSON 文件存在。
配对预检也优先接收
`-SignerGuiPath`，CLI `init-key` 只作为辅助诊断。发布前必须关闭旧签名器窗口，从新目录启动唯一的
GUI；不能把旧窗口、旧日志或旧 Bundle 的公钥摘要混入新版本。

## 8. 本轮自动验收结果

- 发布 GUI：`release/usb-license-signer-gui-v0.5.24-g27-20260915-r2`，`localKeyVerified=true`；
- 发布 GUI 的 `--diagnose-key`：`prod-2026-01 / 4d140722…`，退出码 0，带空格路径诊断通过；
- signer：release 单测 27 + GUI 单测 4 全部通过，`clippy --all-targets -D warnings` 通过；
- 主程序：`src-tauri` 测试 400 passed、2 ignored；
- 能力包：离线审计 50 Skills、7 Agents，Codex 0.153.4 兼容检查通过；契约测试 42 passed；
- Bundle：深度校验通过，SHA256SUMS 288 项，`usb-license.json`/vault 均为 0；
- 配对预检：发布 GUI、Bundle、profile 三方完全一致；用历史 `216ae3…` profile 的负例按预期失败；
- U 盘：本轮没有写入或修改任何可移动介质。
