# 便携式 U 盘签名器开发与验收文档

状态：实现中已落地，待真实 U 盘完成一次硬件验收。  
日期：2026-09-13。  
适用：v0.5.24 USB Bundle、`usb-license-signer-gui`。

## 1. 目标

签名工具需要满足两个同时成立的要求：

1. 当前发行签名电脑可以继续签名；
2. 把签名工具目录复制到另一台 Windows x64 电脑后，不依赖原电脑的用户目录、DPAPI
   或机器指纹，也能签名同一发行公钥绑定的 Bundle。

“把私钥直接放进 EXE”不满足安全要求：任何拿到 EXE 的人都能伪造授权。因此本方案把
Ed25519 私钥放进旁置的加密 vault，运行时由用户输入密码解锁。

## 2. 密钥与发行绑定

发行 profile 是“当前版本”的公共输入，不再固定为某台机器或某个永久文件内容。每个版本都必须
显式提供自己的 profile；profile 中的 `keyId + publicKeySha256` 可以延续，也可以随版本轮换。
历史 profile 只允许用于对应历史 Bundle 的复盘，不能与新 Bundle 混用。密钥轮换或版本绑定变化时，
必须同时更新 profile、管理器、完整 Bundle、签名器说明，并对新介质重新签名。

Bundle 的 `usb-manifest.json/usbGuard/signing`、管理器运行时和签名器都必须绑定同一个
`keyId + publicKeySha256`。签名器在写入 `usb-license.json` 前执行比较；不一致时保持
U 盘未修改的 fail-closed 行为。

## 3. Portable vault 格式

文件名固定为 `codex-usb-signing-key.vault`，放在签名 EXE 同目录。格式为：

```text
magic + version + keyId length + keyId + 16-byte salt + 12-byte nonce
+ AES-256-GCM(ciphertext of 32-byte Ed25519 seed + tag)
```

- KDF：Argon2id，64 MiB、3 次迭代、1 lane，输出 32 字节；
- AES-GCM AAD：从 magic 到 nonce 的完整头部，防止 keyId/参数被替换；
- vault 采用 create-only 写入，不覆盖已有文件；
- 密码最少 12 字节，密码、派生密钥和解密 seed 只在内存中短暂存在并尽量清零；
- 密码不通过命令行、环境变量、日志或 JSON 报告传递；
- vault 丢失或忘记密码时，不能从公钥恢复私钥，应走密钥轮换流程。

实现位置：`tools/usb-license-signer/src/key_blob.rs`。

## 4. GUI 行为

### 发行签名器

1. 由受控密钥迁移流程从发行签名电脑的密钥生成并验收 portable vault；
2. 用当前版本 profile、该 vault 和完整 Bundle 构建发行签名器；
3. 双击 GUI，插入复制了完整 Bundle 的可移动 NTFS U 盘；
4. 深度检查通过后点击“一键签名 U 盘”；
5. 通过原生 Windows 密码对话框输入 vault 密码，然后签名并自动复核。

正式发行 GUI 不从运行电脑的 DPAPI 生成密钥，也不把本机公钥作为运行门槛。任何一步失败
都不写入 U 盘授权文件；已存在 vault 时不会自动覆盖。

### 其他电脑

把以下文件完整复制到另一台 Windows x64 电脑的同一目录：

```text
CodexUsbLicenseSigner.exe
codex-usb-signing-key.vault
README.txt
DEVELOPMENT.md
manifest.json
SHA256SUMS.txt
```

双击 EXE、插入新 Bundle，等待检查完成，输入同一个密码即可。目标电脑不需要安装
Rust/Python，不需要原签名用户的 `%LOCALAPPDATA%` DPAPI Blob，也不需要联网。

没有 vault 的目录不是完整签名器；GUI 会明确提示复制完整目录，保持签名按钮不可用，
不会检查本机 DPAPI，也不会生成一个“同名但不同钥匙”的假发行密钥。

## 5. CLI 边界

GUI 直接调用共享 `run_with_portable_password` 管线。CLI 也支持：

```text
sign ... --portable-vault PATH --password-stdin
verify ... --portable-vault PATH --password-stdin
export-portable --portable-vault PATH --password-stdin
```

CLI 的 stdin 仅用于自动化测试或受控流水线；正式操作优先使用 GUI。任何私钥环境变量、
明文私钥参数、`--portable-vault` 与 DPAPI `--key-blob` 混用都会被拒绝。

## 6. 发布脚本

正式构建签名器必须显式提供本版本 profile 和匹配的 portable vault：

```powershell
$out = '.\release\usb-license-signer-gui-<timestamp>'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build-usb-license-signer-gui.ps1 `
  -OutputPath $out `
  -SigningProfilePath '.\release-profiles\usb-v0.5.24-g28.json' `
  -PortableVaultPath '.\path\codex-usb-signing-key.vault' `
  -PortableVaultDescriptorPath '.\path\codex-usb-signing-key.vault.public.json'
```

已有 vault 后，可将它纳入发布目录：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build-usb-license-signer-gui.ps1 `
  -OutputPath '.\release\usb-license-signer-gui-<timestamp>-portable' `
  -SigningProfilePath '.\release-profiles\usb-v0.5.24-g28.json' `
  -PortableVaultPath '.\path\codex-usb-signing-key.vault' `
  -PortableVaultDescriptorPath '.\path\codex-usb-signing-key.vault.public.json'
```

构建脚本在 `manifest.json` 中记录本版本 profile、`portableVaultIncluded` 和
`portableVaultBindingVerified`，并把 profile、vault 及其公共描述纳入 `SHA256SUMS.txt`。
脚本不会自动生成 vault，也不会在没有用户密码的情况下复制私钥。

## 7. 验收矩阵

### 自动化

- signer Rust 单元测试：vault 正确密码往返、错误密码拒绝、短密码拒绝、截断拒绝、
  create-only 不覆盖；
- CLI/GUI parser 测试：不提供密码时 portable sign/verify 拒绝；
- release profile、Bundle `usb-manifest`、签名器 manifest 和 portable vault 公共描述的
  keyId/公钥摘要一致；
- 独立 CLI `init-key` 或构建机 DPAPI 只能作为辅助信息，不能阻断或授权便携签名器；
- 发布目录清单和 SHA256 完整，vault 为二进制密文，不出现 32 字节 seed 明文；
- 新管理器和新 Bundle 的静态完整性、内部授权校验、50 Skills/7 Agents 审计继续通过。

### 人工硬件

1. 携带 vault 的发行目录在第一台 Windows x64 电脑输入正确密码，签名成功；
2. 同一目录复制到第二台 Windows：输入正确密码，签名成功；
3. 第二台输入错误密码，签名失败且 `usb-license.json` 不新增/不改变；
4. 删除/截断 vault，GUI 给出明确错误且不写盘；
5. 只复制 EXE 或删除 vault，GUI 不检查本机公钥并明确提示目录不完整；
6. 拔出/替换 U 盘、双候选 Bundle、已签名 Bundle、已安装运行树，都保持不可签名；
7. 新 U 盘签名后运行一键安装、检查更新和 Skill Hub 流程，确认与签名绑定一致。

## 8. 不可绕过的限制

“任何电脑”指 Windows x64 运行环境，不等于无密码、无 vault、无发行包绑定。若把
密码硬编码到 EXE，或把明文 seed 一起打包，工具虽然表面上可以在任何机器运行，但任何
复制者也可以伪造签名，这与 U 盘唯一授权目标冲突，禁止采用。
