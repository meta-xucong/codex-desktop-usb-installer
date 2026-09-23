# 版本化 USB 签名 profile 与可迁移签名器

状态：本轮按用户新要求修正设计；真实 U 盘签名仍需在目标环境验收。  
日期：2026-09-15。

## 1. 用户要求

1. 生产 profile 不能固定绑定某台电脑、某个 Codex 执行上下文或某个永久文件内容。
2. 每次发行版本都可以使用自己的 `keyId + publicKeySha256`；是否换钥匙由该版本发行流程决定。
3. 每个版本的管理器、U 盘 Bundle、签名器必须使用同一份该版本 profile。
4. 签名器目录可以复制到另一台 Windows x64 电脑；只要该目录带有与 Bundle 对应的加密 portable vault，并输入正确密码，就可以签名。
5. 本机 DPAPI 只能作为首次导出 portable vault 的受保护密钥来源，不能作为发行 profile，也不能因为当前机器生成了新 key 就改变已发布 Bundle 的绑定。

## 2. 正确的信任模型

`profile` 是一次发行的公共绑定声明，不是“这台机器的 profile”：

```text
release profile (public metadata)
        │
        ├── SignedRequired 管理器编译期公钥
        ├── Bundle usb-manifest.json/usbGuard/signing
        └── 同版本 signer manifest / portable vault 说明
```

真正能签名的是对应的 Ed25519 私钥。私钥不进入仓库、管理器、Bundle 或明文命令行，而是放在
`codex-usb-signing-key.vault` 中，用密码保护。其他电脑不需要原电脑的 DPAPI；它只需要完整的
签名器目录和同一个 portable vault。若 vault 解出的公钥与 Bundle 不匹配，必须拒绝签名。

因此下面两件事必须分开：

- “这份包声明使用哪把公钥”：由本版本 profile 决定；
- “当前电脑是否拥有对应私钥”：由 portable vault 在实际签名时决定。

## 3. 发行 profile 契约

正式构建必须显式传入 `-UsbSigningProfilePath` / `-SigningProfilePath`，不能依赖仓库中某个
固定文件作为默认生产钥匙。profile 只包含公共信息，例如：

```json
{
  "schemaVersion": 1,
  "profile": "production",
  "releaseVersion": "0.5.24",
  "releaseId": "g28-final-20260915",
  "keyId": "prod-2026-09",
  "publicKeyB64": "<32-byte Ed25519 public key>",
  "publicKeySha256": "<raw public key SHA-256>"
}
```

`releaseVersion` 和 `releaseId` 是版本记录字段；构建脚本会校验其存在、格式和版本一致性。
`keyId` 可以延续，也可以随版本轮换。profile 不得包含私钥、DPAPI Blob、portable vault 密码
或明文 seed。

仓库中保留的旧 profile 只能作为历史复盘/测试输入，不能被正式构建隐式选中。

可用 `tools/new-usb-signing-profile.ps1` 从本版本的公共钥匙信息生成一个新的 profile；该工具
只写公共字段并自动计算原始 32 字节公钥的 SHA-256，不接收或保存私钥：

```powershell
pwsh -File .\tools\new-usb-signing-profile.ps1 `
  -ReleaseVersion 0.5.24 `
  -ReleaseId g29-final-20260915 `
  -KeyId prod-2026-09 `
  -PublicKeyB64 '<本版本公钥 Base64>' `
  -OutputPath .\release-profiles\usb-v0.5.24-g29.json
```

## 4. 版本发布流程

每个版本按以下顺序执行：

1. 在受控签名环境生成或导入本版本的私钥，并导出其公共 `keyId/publicKeyB64/publicKeySha256`。
2. 为该版本单独生成 profile 文件；需要换钥匙时只替换这个版本的 profile，不修改旧版本证明。
3. 用同一个 profile 构建 SignedRequired 管理器。
4. 用同一个 profile 构建 U 盘 Bundle。
5. 用同一个 profile 构建该版本 GUI signer；若要让新电脑直接使用，随 signer 交付匹配的
   `codex-usb-signing-key.vault`。
6. 运行 pair preflight，确认 profile、Bundle、管理器和 signer manifest 的 `keyId + digest` 一致。
7. 在实际签名桌面或携带 portable vault 的任意 Windows x64 电脑上，对干净 Bundle 完成
   `sign → read-back → verify`。未完成这一步只能标记“待硬件验收”。

示例（路径和 profile 名称必须按当前版本替换）：

```powershell
$profile = '.\release-profiles\usb-v0.5.24-g28.json'

pwsh -File .\tools\build-installer.ps1 `
  -UsbPolicy SignedRequired `
  -UsbSigningProfilePath $profile

pwsh -File .\tools\build-oneclick-usb-layout.ps1 `
  -UsbSigningProfilePath $profile `
  -InternalSigned

pwsh -File .\tools\build-usb-license-signer-gui.ps1 `
  -SigningProfilePath $profile `
  -PortableVaultPath '.\secure\codex-usb-signing-key.vault'

pwsh -File .\tools\verify-usb-signing-pair.ps1 `
  -BundlePath '.\release\usb-oneclick-<version>' `
  -ProfilePath $profile `
  -SignerGuiPath '.\release\usb-license-signer-gui-<version>\CodexUsbLicenseSigner.exe'
```

正式构建省略 profile 参数时必须直接失败；不能回退到当前机器的本地 DPAPI，也不能回退到
仓库中的旧 profile。

## 5. 防止本次错误再次发生

- 构建宿主的 DPAPI 公钥不再有权改变 release profile。
- profile、Bundle、管理器或 signer 任意一方不一致，构建/验收失败。
- `-SkipLocalKeyCheck` 只能表示“本机没有可验证私钥”，不能表示“签名工具可交付”；要在其他电脑工作，必须交付匹配 vault 并在真实环境验证。
- 签名器运行时仍以 Bundle 实际声明和 vault 实际解出的公钥为最终门禁，不信任文件名、`keyId` 单字段或用户手工改写的摘要。旧版 vault 在一次成功密码解锁时会自动补写公共描述文件；描述文件缺失不会替代实际 vault 解密校验。
- 历史包、历史 signer 和新版本 profile 不混用；每个输出目录必须使用唯一版本标识。

## 6. 验收矩阵

| 条件 | 预期 |
| --- | --- |
| 正式构建未传 profile | 立即失败，不使用固定默认 profile |
| 传入版本 profile，环境变量为空 | 环境变量由该 profile 临时初始化 |
| 环境变量与版本 profile 不一致 | 立即失败 |
| Bundle 使用 profile A，signer manifest 使用 profile B | pair preflight 失败 |
| 另一台电脑只有 EXE、没有匹配 vault | 签名不可用，不能自动生成同名假钥匙 |
| 另一台电脑携带匹配 vault、密码正确 | 可签名并自动回读验签 |
| vault 解出的公钥与 Bundle 不匹配 | fail-closed，U 盘不写入 |

## 7. 未替代的人工证据

脚本与单元测试只能证明 profile 传递和静态绑定逻辑；不能替代真实 Windows 电脑上的
portable vault 解锁和真实 NTFS U 盘写入。最终交付前仍须保存同一版本的 sign/read-back/verify
报告和产物哈希。

## 8. 本轮代码验证记录

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --locked`：核心 28 项、GUI 4 项全部通过。
- `cargo clippy --manifest-path tools/usb-license-signer/Cargo.toml --all-targets --all-features -- -D warnings`：通过。
- `cargo test --manifest-path src-tauri/Cargo.toml --locked`：400 项通过、2 项忽略。
- 版本化 PowerShell 脚本解析、profile 版本门禁、缺失 profile 的正式构建拒绝、旧 Bundle 的版本不匹配拒绝：通过。
- 旧 G28 产物不会被新 pair preflight 当作当前版本有效产物；必须按当前版本重新构建 Bundle 和 signer。

以上是代码和静态门禁证据，不等同于真实 U 盘签名完成；新版本交付前仍必须完成一次匹配 portable vault 的跨电脑 sign/read-back/verify。
