# G22：U 盘发行签名密钥绑定与重复误判修复

## 结论

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

1. 使用本仓库的正式发行 profile 重新构建管理器和 U 盘包。
2. 将完整新包复制到 U 盘根目录；静态包不要自行放入 `usb-license.json`。
3. 在持有 `216ae3…` 对应 DPAPI 私钥的签名机上运行图形化签名工具。
4. 看到“签名环境已匹配”后再执行一键签名。
5. 以后更换 Codex、Skill Hub 或任何受 `SHA256SUMS.txt`/Anchor 保护的内容，都必须重新生成完整包并重新签名；不要复用旧许可证。

## 防回归验收

- 正确 profile 构建：管理器编译期公钥摘要、`usb-manifest.json` 摘要和签名机 DPAPI 公钥摘要均为 `216ae358…`。
- 错误 profile 构建：必须在构建阶段失败，不产生可交付包。
- 旧 `4d1407…` 包在 `216ae3…` 签名机上必须继续拒绝；这是预期安全负例。
- 新 `216ae3…` 包在目标签名机上才允许写入许可证。

