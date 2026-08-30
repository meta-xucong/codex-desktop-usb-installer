# 当前进度归档说明

本仓库用于归档 Codex Desktop 离线安装器第一版、CCSwitch 集成、卸载流程以及下一阶段 U 盘绑定/本地看门狗设计。

## 归档内容

- `src/`、`src-tauri/`、`crates/`：当前实现源码；
- `tools/`：离线载荷收集、验证、构建和测试脚本；
- `release/usb-oneclick/`：当前推荐的一键 U 盘布局（保留轻量包装器、清单和验证脚本）；
- `README.offline.md`、`TEST-REPORT.offline.md`：第一版使用说明和回归记录；
- `docs/USB-BINDING-WATCHDOG-DEVELOPMENT.md`：下一阶段开发需求；
- `docs/USB-BINDING-WATCHDOG-AUDIT.md`：当前实现审计报告。

## 未放入 Git 历史的大文件

Codex MSIX 约 802 MB，GitHub 普通 Git blob 上限为 100 MB，因此以下文件通过 `.gitignore` 排除，不会被误提交：

```text
release/usb-oneclick/payload/OpenAI.Codex_x64.msix
```

当前本地归档信息：

```text
size:   802145236
sha256: ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e
```

拿到完整载荷后，放回上述相对路径，再执行 `tools/verify-payload.ps1` 和 `tools/verify-oneclick-usb.ps1` 即可复核。其他 `release/setup`、`release/usb`、`release/usb-final` 和 `release/upstream` 目录是可重建的重复输出，不进入归档。

## 复现与测试

```powershell
npm ci
npm run check
npm run lint
npm run test:offline
npm run test:offline-bundle
cargo test --manifest-path .\src-tauri\Cargo.toml
```

当前归档前已完成上述检查（离线 bundle 测试使用合成、无签名 MSIX；真实签名和真实 AppX 安装仍需在 Windows 空白环境验收）。

## 敏感信息

本归档不包含 API Key、用户登录数据或 CCSwitch 用户配置。404TOKEN 只作为非敏感的 Base URL 出现在产品目录和验证逻辑中，API Key 必须由最终用户在本机输入。
