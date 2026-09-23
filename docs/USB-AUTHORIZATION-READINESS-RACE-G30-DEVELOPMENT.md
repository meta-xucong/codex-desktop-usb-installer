# U 盘授权就绪竞态与“签名校验失败”误报修复文档（G30）

状态：源码已修复，G30 安装器/签名器已重新打包；待在签名 U 盘目标机实测  
适用范围：U 盘版 Codex 安装、修复、更新及其 USB Guard 授权门禁

## 1. 现场结论

测试截图中的后端详情包含：

```text
usb_license_gate phase=offline-payload-stage
未检测到有效 U 盘租约或看门狗（status=authorized ...）
```

这不是“许可证公钥/签名正文验证失败”的证据。它表示安装器读到了旧的
`authorized/running` 状态快照，但在真正进入 MSIX 暂存时没有观察到当前进程
拥有的活动租约或 `CodexUsbGuard.exe` 锁。旧错误分类器把所有 `usb_license_*`、
`usb_watchdog*` 错误都映射为 `signature`，所以首页显示成了“更新包签名校验失败”。

## 2. 确定根因

1. `spawn()` 只表示 Windows 接受了进程创建请求，不表示看门狗已经完成启动、
   获取 `guard.lock` 并开始持有租约。
2. 管理器在后台启动看门狗并写入 `status.json`，旧代码在 `spawn()` 成功后立即
   写入 `watchdog=running`、`codexAllowed=true`；首次点击可能早于锁文件出现。
3. `status.json` 是持久化渲染快照。管理器崩溃、升级或旧进程退出后，快照可能仍
   是 `authorized/running`，但租约或 helper 已经不存在。
4. 安装页面原来先调用 `offline_payload_stage`，再做 `usbGuardStatus` 复核，
   于是后端先拒绝并将底层门禁错误暴露给错误分类器。

## 3. 修复方案

### 3.1 不放宽安全门

- 正式包仍必须有有效签名许可证、匹配的物理 U 盘、活动租约和活动看门狗。
- 没有租约/锁、签名不正确、锚点变化或物理介质不匹配，仍然拒绝安装和更新。
- 仅把“授权尚未就绪”和“签名确实无效”分成不同的用户提示，不自动绕过任何
  签名或完整性检查。

### 3.2 启动顺序修复

- 每次管理器启动都先回收旧 helper，即使新旧 helper 二进制字节相同，避免旧进程
  抢占新租约。
- `spawn()` 成功后，最多等待 5 秒确认 `guard.lock` 由正确的
  `CodexUsbGuard.exe` 进程持有；未确认前不发布 `watchdog=running` 或
  `codexAllowed=true`。
- 状态快照若显示 `authorized/running`，但实时租约或 helper 锁缺失，状态层自动
  降级为不可操作的 `starting/not-running`，不再让旧快照解锁按钮。
- 后端授权门在启动过渡期做最多 3 秒的有界重试；`mismatch`、无效许可证等终态
  立即失败，保持 fail-closed。

### 3.3 安装页面顺序修复

一键安装变为：

```text
实时 USB Guard 复核
        ↓ 失败：提示授权未就绪，不产生暂存文件
MSIX 暂存与完整校验
        ↓
安装前/安装后再次复核
        ↓
能力包与 CCSwitch 的独立善后
```

前端只把按钮快照当作优化，不把它当作授权凭证；后端仍是最终权威。

### 3.4 错误分类修复

新增稳定错误码 `usb_authorization`，用于：

- `usb_watchdog_not_ready`
- `usb_watchdog_not_running`
- “未检测到有效 U 盘租约或看门狗”
- `usb_license_gate` 中明确显示 `lease=false` 或 `helper=false` 的启动竞态

真正的 `usb_license_missing`、`usb_license_unsigned`、`usb_license_invalid`、
Authenticode/Ed25519 校验失败仍归入 `signature`；Bundle 锚点和文件完整性问题
仍归入 `bundle_integrity`。

## 4. 回归验收

- Rust 错误分类测试：启动租约缺失/看门狗缺失不再分类为 `signature`；真实许可证
  无效仍分类为 `signature`。
- USB Guard 纯逻辑测试：只有 `starting` 或已发布但尚待 helper 锁确认的状态允许
  有界等待；`mismatch` 和 `authorized/not-running` 不无限等待。
- 状态快照测试：`authorized/running + helper=false` 被降级为不可操作状态；完整
  live 证据存在时保持原状态。
- 前端测试：安装前实时复核失败时不调用 `offlinePayloadStage`，不调用安装命令，
  显示“U 盘授权尚未就绪”；正常流程仍进入暂存、安装和能力包流程。
- 发布后仍需目标机验证：新包签名、冷启动锁文件、首次点击一键安装、Codex 更新、
  拔插 U 盘和真实无效签名包。源码/本机测试不能替代这些 Windows 现场证据。

本轮已生成并完成本机静态配对验收：

- U 盘包：`release/usb-oneclick-v0.5.24-g30-final-20260916`
- 便携签名器：`release/usb-license-signer-gui-v0.5.24-g30-final-20260916-portable`
- 发行绑定：`keyId=prod-2026-01`，公钥摘要
  `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`，
  `releaseId=g30-final-20260916`
- 包内 `verify-usb.ps1`、Payload 校验和 `verify-usb-signing-pair.ps1` 均通过。
- 新 G30 的 `CodexAppManager.exe` 与 G29 产物哈希不同，且已包含新的
  `usb_watchdog_not_ready` / `usb_authorization` 路径；物理 U 盘签名和目标机首击仍待现场验证。

## 5. 预防规则

以后凡是看到“签名失败”且详情同时出现 `lease`、`helper`、`watchdog` 或
`status=authorized`，必须先区分：

1. 签名/许可证/锚点是否真的失败；
2. 启动状态、租约和 helper 是否实时一致；
3. UI 是否在后端门禁前完成了实时复核。

不能再用“所有 USB 错误都显示签名失败”的兜底分类，也不能用持久化
`status.json` 单独解锁破坏性操作。

## 6. 发布前复核记录（2026-09-16）

- 源码能力包和 G30 包兼容性审计均通过：50 个 Skills、7 个 Agents、模型目录、
  MCP/运行时清单、无链接文件、逐文件哈希均满足约束。
- G30 包内能力包验证通过 253 个文件；隔离用户目录完整安装通过，50 个 Skills、
  7 个 Agents、Python health 均通过，且 `python.cmd`/`pip.cmd` 生成为正确的
  ASCII wrapper。开发机已有的 1.2.0 旧安装中发现的 14 字节 `\\` 壳不能作为 G30
  包失败证据。
- Skill Hub 中转源现场探测返回 200，Relay 元数据的 commit、大小和 SHA-256 与
  GitHub `master` 一致；Codex 中转源的元数据和校验文件也可读。目标机实际 ISP、
  代理和真实 U 盘签名/更新仍需现场验证。
- 签名器与 G30 包的 portable vault/profile 配对预检通过，使用同一
  `keyId=prod-2026-01`、公钥摘要和 `releaseId=g30-final-20260916`；没有读取构建机
  或运行机 DPAPI 公钥作为门槛。
