# G28-R3：目标签名桌面绑定与构建宿主隔离

状态：目标桌面密钥已确认；正式交付已切换为 portable vault 模式，待真实 U 盘完成硬件验收

## 问题

G28-R2 将构建宿主诊断出的 `4d1407…` 写入发行 profile，但用户实际双击签名器的桌面
持有 `216ae3…`。两者 `keyId` 相同而公钥不同，导致完整 Bundle 深检通过、签名按钮仍被
严格禁用。这个问题不是 U 盘损坏，也不是 Payload 或文件复制失败。

## 处理原则

1. 本次受控发行绑定固定为目标交互式签名桌面确认的 `keyId + publicKeySha256`：
   `prod-2026-01 + 216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`。
2. 新版本使用独立 `releaseId=g28-r3-final-20260915`，不得与 G28-R2 的 `4d1407…` Bundle、
   管理器或签名器混用。
3. 正式签名器必须携带与该 profile 匹配的加密 portable vault；构建宿主和运行电脑的
   DPAPI 都不是签名运行门槛。
4. 不删除、覆盖或伪造目标桌面的 DPAPI Blob；不把机器指纹写进 Bundle。

## 交付组成

- `tools/usb-signing-release-profile.json`：G28-R3 当前 profile；
- `release-profiles/usb-v0.5.24-g28-r3-final-20260915-key216.json`：版本化公共 profile；
- 重新构建的 `CodexAppManager.exe`、`CodexDesktopSetup.exe` 和完整 USB Bundle；
- 重新构建的、携带 `codex-usb-signing-key.vault` 的 `CodexUsbLicenseSigner.exe`。

## 验收

构建侧完成 profile、管理器、Bundle、签名器 manifest 和 portable vault 公共描述的静态
配对检查。真实验收只需在任意 Windows x64 电脑的新版 GUI 中输入 vault 密码，完成一次
sign → readback → verify；不再要求该电脑的 DPAPI 公钥为 216。

## 防回归

- 交付链接只能指向同一个 R3 目录，不能让用户在 `release` 历史目录中挑选 EXE；
- 每次版本升级必须把 `releaseId`、profile、管理器、Bundle、签名器 manifest 一起核对；
- 报告必须记录实际 GUI 二进制、portable vault 公共描述和 Bundle 的密钥摘要，不能把
  构建机的 CLI 结果当成便携签名能力证明；
- 旧 profile 只用于历史复盘，不得用于新 Bundle。
