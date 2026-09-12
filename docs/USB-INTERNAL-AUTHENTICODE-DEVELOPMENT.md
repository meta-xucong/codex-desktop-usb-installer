# USB 内部签名与 Authenticode 边界修复

## 目标

修复已完成 U 盘授权后，另一台 Windows 电脑启动管理器却提示“启动前完整 bundle 校验失败，Authenticode 无效”的问题。

正式 U 盘包采用两层不同的信任边界，不能把它们混成一个布尔开关：

1. **U 盘内部授权签名**：授权文件绑定可移动 NTFS 卷、卷序列号、bundle 相对路径、不可变文件锚点和 Ed25519 公钥。`SHA256SUMS.txt` 再逐项保护 bundle 内容。
2. **Windows Authenticode**：只用于操作系统/第三方可信输入，例如 OpenAI 的 MSIX、Microsoft VC++ 运行库和 Microsoft WebView2 离线安装包。

当前发布包中的 `CodexDesktopSetup.exe`、`CodexAppManager.exe` 和 CCSwitch 是本项目自己的发行文件，默认没有企业 Authenticode 证书；它们已经由 U 盘内部授权和逐文件 SHA-256 保护。把这些文件强制当成 Authenticode 输入，必然让合法签过 U 盘的包无法启动。

## 校验模式

运行期和工具链使用三个显式模式：

| 模式 | 用途 | 自有 EXE Authenticode | 外部可信输入 Authenticode |
| --- | --- | --- | --- |
| `HashOnly` | 开发/普通内容访问 | 不要求 | 不要求 |
| `InternalSigned` | 正式 U 盘 `SignedRequired` 运行期 | 不要求；由许可证、锚点和 SHA-256 保护 | 必须：OpenAI MSIX、Microsoft VC++、Microsoft WebView2 |
| `StrictAuthenticode` | 需要证书签名的严格发布验收/兼容旧调用 | 必须 | 必须 |

`SignedRequired` 不等于“所有 EXE 必须 Authenticode”。它表示必须通过 `InternalSigned`：U 盘许可证和清单签名绑定必须成立，外部供应商输入仍须是可信签名。

## 修复范围

- Rust 管理器：以枚举区分上述模式；正式 USB 看门狗、启动前预检和管理器启动路径统一使用 `InternalSigned`。
- PowerShell 打包：新增 `-InternalSigned`，要求 `SignedRequired`，保留 `-SkipSignature` 作为开发包开关；正式内部签名包仍强制校验 MSIX、VC++ 和 WebView2 的 Authenticode。
- PowerShell 验证：新增 `-InternalSigned`，不再错误要求本项目自有 EXE 的 Authenticode，但继续检查外部可信输入和全部哈希/清单约束。
- 发布清单：明确记录内部授权模式，避免以后把“U 盘授权签名”和“Windows 程序证书签名”误解为同一件事。

## 安全不变量

- 不降低 U 盘许可证的 Ed25519 验签、卷/序列号/路径/锚点绑定。
- 不把私钥放入发行包；签名工具仍在受控环境中执行。
- 不放宽 `SHA256SUMS.txt`、重解析点、清单交叉字段、payload 身份和架构校验。
- OpenAI MSIX、Microsoft VC++ 和 Microsoft WebView2 的 Authenticode 仍是正式包的硬条件。
- 默认验证器仍保留严格模式；只有显式 `-InternalSigned` 才允许自有 EXE 没有 Authenticode。
- 本次修复不写入、不修改当前物理 U 盘。新包复制到 U 盘后必须重新生成该包对应的许可证；旧包许可证不能复用到新锚点。

## 验收标准

1. `cargo fmt --check`、`cargo test` 和 PowerShell 语法检查通过。
2. 内部签名包用 `verify-oneclick-usb.ps1 -InternalSigned` 通过。
3. 同一包不加 `-InternalSigned` 时，在自有 EXE 未签 Authenticode 的环境下应明确失败，而不是误报为“内部签名损坏”。
4. 正式包的 OpenAI MSIX、VC++ 和 WebView2 仍必须通过对应 Authenticode/供应商校验。
5. 用户把新包复制到格式化后的 U 盘并用签名 GUI 重新签名后，管理器启动前不再因自有 EXE 的 Authenticode 缺失而阻断；篡改任一受保护文件、复制到另一只 U 盘或许可证不匹配仍必须阻断。

## 发布操作

```powershell
$env:CODEX_USB_GUARD_POLICY = "SignedRequired"
$env:CODEX_USB_GUARD_PUBLIC_KEY_ID = "prod-2026-01"
$env:CODEX_USB_GUARD_PUBLIC_KEY_B64 = "<受控环境中的 32 字节公钥 Base64>"

powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-oneclick-usb-layout.ps1 `
  -InstallerPath release/setup/CodexDesktopSetup.exe `
  -ManagerPath release/setup/CodexAppManager.exe `
  -BundlePath release/usb-oneclick-v0.5.24-current-dev-candidate-final-20260910 `
  -OutputPath release/usb-oneclick-v0.5.24-internal-signed-authfix-20260911 `
  -UsbPolicy SignedRequired `
  -InternalSigned

powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-oneclick-usb.ps1 `
  -BundlePath release/usb-oneclick-v0.5.24-internal-signed-authfix-20260911 `
  -InternalSigned -SkipNetwork
```

输出目录只是本地发行包，不会自动写入 U 盘。复制到目标 U 盘后，必须使用受控的 U 盘授权工具为新 bundle 生成新的 `usb-license.json`。

