# G28：交互式签名环境与构建宿主密钥分离修复

> 历史记录：G28 的 `216ae3…` 只是当时版本的绑定证据，不是永久生产 profile。后续版本按
> `docs/USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md` 使用各自的 profile 和 signer。

状态：已实现，待用户用新目录完成一次真实 U 盘签名验收
日期：2026-09-15

## 1. 用户现场结论

用户在实际 Windows 桌面运行签名器，检查全新 G27 Bundle 时得到：

```text
Bundle: usb-oneclick-v0.5.24-g27-final-20260915
License: missing
Bundle deep verify: passed
Signer keyId: prod-2026-01
Signer publicKeySha256: 216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80
Bundle publicKeySha256: 4d14072263e1cbba65591032314ba17aeaa152bf1367663dab8d7dc9d31b9eef
```

因此 G27 不是 U 盘损坏，也不是 Bundle 完整性问题，而是发行包误绑定了 Codex 构建宿主
的 `4d1407…`，实际交互式签名桌面持有的是 `216ae3…`。

## 2. 根因

之前把“Codex 执行环境中运行的 CLI/GUI 诊断”当作正式签名环境证据。Windows DPAPI 按
用户、会话和执行上下文保护；构建宿主与用户双击签名器的桌面上下文可能返回不同的公钥。
因此出现：

1. Bundle 深度校验通过；
2. 构建脚本与宿主环境的 `4d1407…` 自洽；
3. 用户桌面签名器读到 `216ae3…`；
4. 签名器正确 fail-closed，拒绝写入 License。

## 3. 本次修复

正式 profile 恢复为实际交互式签名桌面的公钥：

```text
keyId=prod-2026-01
publicKeyB64=Vn+Kzai6rtqaF0mS5BASVyxLOsTOkoXbtcCfs1veJO8=
publicKeySha256=216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80
```

G28 重新构建：

- 管理器编译绑定：`216ae3…`；
- Bundle `usb-manifest.json`：`216ae3…`；
- 新 GUI 签名器 manifest：期望 `216ae3…`；
- 构建宿主无法证明交互式 DPAPI 时，签名器包明确标记为“需在目标签名桌面验收”，不再
  把宿主的 `4d1407…` 偷换成正式发行密钥；
- 新包不携带私钥、DPAPI Blob 或 portable vault，首次在匹配签名桌面签名时由用户设置密码。

## 4. 防回归规则

1. 正式公钥只能来自受控发行 profile，不得根据当前 Codex 宿主临时生成的 DPAPI 公钥旋转；
2. 构建机与签名机分离时，构建机只能证明公共绑定，不能宣称“签名机可用”；
3. 交付前必须在实际用户将要双击的 Windows 桌面，从新发布目录运行 GUI 的
   `--diagnose-key <json>`，确认结果等于 Bundle 和 profile；
4. 旧 GUI 窗口、旧日志和旧 Bundle 的公钥摘要不得作为新发布证据；
5. 真实 U 盘 sign→readback→verify 仍是最终硬件验收，未完成时不标记“已签名”。

## 5. 验收边界

本地已完成结构、清单、哈希、能力包和代码测试；本轮没有写入 U 盘。G28 的最终签名结论
必须以用户在实际交互式桌面使用新签名器完成一次真实 sign/readback/verify 为准。
