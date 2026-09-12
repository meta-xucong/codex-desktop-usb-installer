# Codex 更新默认回退修复记录

日期：2026-09-12

## 发现的问题

自动更新的默认源是国内镜像，但镜像失败后原实现通过
`should_try_legacy_relay` 跳过了内置 Relay，直接请求 GitHub。这样在国内测试机
上会把已经部署好的 Relay 形同虚设；同时 Relay 的 base URL 仍直接取镜像地址，若
强行启用 Relay 又会把 `latest.json` 请求到镜像协议上。

## 修复内容

- 默认自动链改为：镜像 → 内置 Relay → GitHub。
- 用户配置自定义 Relay 时，自动链第二步仍使用用户配置地址。
- `resolve_source`、`source_locator_for`、`source_matches_settings` 统一使用
  `relay_base_url_for`，默认镜像地址映射到内置 Relay 地址。
- check/apply 的 source locator 保持一致，仍执行 source-tagged identity 和 stale
  检查，不混用镜像与 Relay 的 manifest、checksum、MSIX。
- 新增默认 Relay 映射和完整自动回退顺序单测。

## 不属于代码缺陷的事项

- 当前本机签名器 DPAPI 密钥与现有 key216 U 盘的公钥不同。不能在这里生成或替换
  私钥，否则会使现有已签名 U 盘失效；新包可以绑定已知的 key216 公钥，但真正写
  入 U 盘仍必须在持有对应 DPAPI 私钥的签名机上完成。
- 镜像与 Relay 可能暂时发布不同的最新版本。客户端按候选源各自完整校验并接受满足
  版本门槛的结果，这是可运营的同步延迟，不应通过客户端混用元数据来“强行一致”。

## 验收要求

- Rust channels 单测通过，自动失败顺序必须为 `Mirror, Relay, Github`。
- settings、前端类型、lint、构建和前端测试回归通过。
- 使用 key216 公钥编译的本地新 bundle 必须通过静态 internal-signed verifier；不在
  当前机器上写入真实 U 盘，也不伪造 key216 签名。

## 本轮执行结果

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过。
- Codex channels 单测：33 项通过；settings store：10 项通过；Skill Hub 热更新：9
  项通过；`src-tauri` 全量：384 项通过、0 失败、2 项忽略。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`：通过；同时
  修复了 `build.rs` 及 Skill Hub 模块中阻断严格 lint 的 9 个既有问题。
- 自动回退最终实现已移除伪条件，`Auto` 路径无条件尝试 `Mirror → Relay → Github`；
  channels 33 项和全量 Rust 386 项（384 通过、2 忽略）复核通过。
- 前端：`npm run check`、`npm run lint`、`npm run build`、32 个 Vitest 文件/335 项
  测试均通过；Vite 仅保留既有的大 chunk 性能提示。
- 能力包：50 Skills、7 Agents、25 个 `openai.yaml`、19 个连接字段，离线审计无
  warning/error；适配器契约 38 项通过；Skill Hub Relay 契约 3 项通过、1 项因本机
  Python 环境未提供而跳过。
- 已用当前源码生成新 manager：
  `release/setup/CodexAppManager.exe`，SHA-256 为
  `639059d3c3198854b3218116d2ff2ee20495fd4c6003db97d593363df399317f`。
- 已生成新的 key216 公钥绑定包：
  `release/usb-oneclick-final-20260912-key216-r6`；内置 verifier、MSIX、完整清单、
  CCSwitch、VC++/WebView2 资源和能力包静态校验通过，包内没有 `usb-license.json`。
  包内能力包复核仍为 50 Skills、7 Agents、8 个 MCP/连接资源文件、58 个 bundled
  runtime 文件；根 `SHA256SUMS.txt` 的 281 行与实际发行文件逐项一致。

因此 r6 是“必须重新签名”的新候选包，不是已经授权的可直接运行 U 盘包。
