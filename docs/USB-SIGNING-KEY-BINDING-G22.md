# G22：U 盘发行签名密钥绑定与重复误判修复

> 历史复盘文档：其中的 `216ae3…`、`4d1407…` 和“当前发行 profile”只属于当时的 G22/G28
> 现场，不是永久生产密钥。新的版本化规则以
> `docs/USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md` 为准；正式脚本必须显式接收当前版本 profile。

## 历史发行状态（2026-09-15）

根据 2026-09-15 对“即将交付的 GUI 二进制”执行的实时诊断，当前这台 T14S 电脑的
`prod-2026-01` 公钥发行绑定为：

```text
publicKeySha256=216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80
```

当时生成的管理器、Bundle 和签名器使用该 profile。此前 `4d140722…` 绑定的历史包及 Codex 宿主状态
保留为历史复盘版本，不得拿当时的 signer 混用。为了让签名器跨电脑运行，
新 GUI 首次在匹配的 T14S 密钥环境中把 DPAPI 密钥迁移为 EXE 旁的加密
`codex-usb-signing-key.vault`；之后复制完整签名目录并输入密码即可，不依赖目标电脑的
DPAPI 或用户目录。

## 历史结论：2026-09-13 r11 配对失败

2026-09-13 测试机弹出的“签名环境未匹配”不是签名器误判，也不是 U 盘损坏。
截图中的测试机 DPAPI 签名环境是：

```text
keyId=prod-2026-01
publicKeySha256=216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80
```

此前重新打包的 `usb-oneclick-v0.5.24-g21-final-20260913` 却绑定了开发机的另一把公钥：

```text
keyId=prod-2026-01
publicKeySha256=4d14072263e1cbba65591032314ba17aeaa152bf1367663dab8d7dc9d31b9eef
```

`keyId` 相同不能代表密钥相同。签名器拒绝写入 `usb-license.json` 是正确的安全行为；不能通过修改 manifest 摘要、删除 DPAPI Blob 或放宽比较来绕过。

## 反复发生的根因

过去的正式构建脚本要求从当前 PowerShell 环境读取
`CODEX_USB_GUARD_PUBLIC_KEY_ID` 和 `CODEX_USB_GUARD_PUBLIC_KEY_B64`，但没有要求它们必须属于受控的发行签名配置。于是，在不同电脑或不同 Windows DPAPI 用户环境中执行打包时，可能把当前电脑的另一把公钥编译进管理器和 Bundle。

结果是：

1. 构建命令本身成功；
2. Bundle 的 `usb-manifest.json`、管理器和构建环境彼此一致；
3. 目标签名机读取到另一把 DPAPI 私钥；
4. 签名器按设计 fail-closed，界面显示“签名环境未匹配”。

## 修复

- 新增 `tools/usb-signing-release-profile.json`，只保存发行公钥、KeyId 和公钥摘要，不保存私钥或 DPAPI Blob。
- 新增 `tools/usb-signing-profile.ps1`，统一验证公钥长度、摘要和环境变量。
- `build-installer.ps1`、`build-oneclick-usb-layout.ps1` 默认从该配置加载发行公钥。
- 如果环境变量为空，自动使用发行配置；如果环境变量只配置一半或与发行配置不一致，立即失败，禁止生成错误绑定的包。
- `verify-oneclick-usb.ps1` 默认按同一发行配置验包，避免“构建用一把、验包用另一把”。
- 允许通过 `-UsbSigningProfilePath` 显式指定受控的密钥轮换/历史 profile；不再允许无意中使用当前机器的任意本地密钥。

## 正确操作顺序

1. 使用本仓库当前正式发行 profile（`4d1407…`）重新构建管理器和 U 盘包。
2. 将完整新包复制到 U 盘根目录；静态包不要自行放入 `usb-license.json`。
3. 在当前 T14S 上运行新版图形化签名工具；首次签名时输入密码，自动生成旁置加密 vault。
4. 看到“签名环境已匹配”后执行一键签名；以后可把完整签名工具目录复制到其他 Windows x64 电脑输入同一密码。
5. 以后更换 Codex、Skill Hub 或任何受 `SHA256SUMS.txt`/Anchor 保护的内容，都必须重新生成完整包并重新签名；不要复用旧许可证。

## 防回归验收

- 正确 profile 构建：管理器编译期公钥摘要、`usb-manifest.json` 摘要和当前签名机公钥摘要均为 `4d140722…`。
- 错误 profile 构建：必须在构建阶段失败，不产生可交付包。
- 历史 `216ae3…` 包与当前 `4d1407…` 签名器必须继续拒绝混用；这是预期安全负例。
- 新 `4d1407…` 包在当前签名器或其解锁的便携 vault 上才允许写入许可证。

## 2026-09-13 r11 再现：包正确，但当前签名机不是匹配环境

本次对 U 盘中的 `r11-local-msix-fix-20260913` 做了只读取证：

- `usb-manifest.json/usbGuard/signing`：`keyId=prod-2026-01`，公钥摘要为 `216ae358…`；
- Bundle 深度完整性检查通过，载荷、清单和文件数量均正常；
- 当前 `T14S` 签名电脑运行 `usb-license-signer.exe init-key` 得到公钥摘要 `4d140722…`；
- U 盘根目录没有写入 `usb-license.json`，说明签名器在写入前正确拒绝，U 盘内容没有被错误修改。

因此这次不是 U 盘损坏，也不是签名算法故障，而是把需要 `216ae3…` 私钥的发行包拿到了只有 `4d1407…` 私钥的电脑上。公钥摘要不能反推出私钥；不能通过修改 Bundle 清单、删除 DPAPI Blob 或放宽比较来“修复”。实际签名必须转到持有 `216ae3…` 对应 DPAPI 私钥的签名电脑完成。

为防止再次把“可构建”误认为“可签名”，新增 `tools/verify-usb-signing-pair.ps1`：交付前必须同时核对发行 profile、Bundle `usb-manifest` 和实际签名机 `init-key` 的 `keyId + publicKeySha256`。构建机没有私钥时可用 `-SkipLocalSigner` 只检查 Bundle 绑定，但不得把该结果当作签名环境可用。

## G27 纠偏

2026-09-15 的现场日志曾出现 `216ae3…`，但该结果来自旧 GUI/旧进程状态；当前源码重新编译的
交互式签名 GUI 的现场诊断返回 `216ae3…`。以后以实际签名桌面的 GUI 诊断为准，不能以 Codex
宿主、历史日志、独立 CLI 或静态 Bundle 检查代替。`build-usb-license-signer-gui.ps1`
与 `verify-usb-signing-pair.ps1 -SignerGuiPath` 已把这一规则固化为 fail-closed 门禁。
