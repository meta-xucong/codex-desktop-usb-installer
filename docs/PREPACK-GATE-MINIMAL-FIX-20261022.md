# 打包前门禁最小修复方案

## 冻结基线

- 仓库：`meta-xucong/codex-desktop-usb-installer`
- 当前提交：`b9940aed14a9b74cbe9561021aa5d6a195d3a991`
- 保留基线中的所有既有改动及未跟踪材料；不得清理历史 release、临时目录或证据文件。
- 本文只处理发行前检查误报/测试发现范围和包输入状态，不改 Product Design 插件本体、媒体 MCP、Codex 配置或用户数据；不签名、不发布、不上传。
- D/I/A：D0；I1（两个有复现证据的发行门禁问题）；A1（检查器和发布测试范围）。

## 发现与证据

### Product Design 哈希审计是换行格式假报

Windows 工作树中的两个 JSON 文件为 CRLF，而能力包清单按 LF 规范化后记录哈希：

- `installer-pack/plugins/product-design-offline/.agents/plugins/marketplace.json`
- `installer-pack/plugins/product-design-offline/plugins/product-design/integrity.json`

对两文件计算 raw SHA-256 与 LF-normalized SHA-256 后，后者分别与 `file-manifest.json`、`checksums.sha256` 相同。Product Design 的 `integrity.json` 内部 98 个文件逐项检查全匹配。测试机已确认 Product Design 可用；本修复不触碰插件内容或其内部清单。

根因是 `tools/refresh-capability-pack-metadata.py` 已规范化文本换行，而 `docs/audit-capability-pack.py` 用原始字节计算文本哈希。两个工具的校验口径不一致。

### 默认 Vitest 扫入历史发行目录

`npm test -- --run` 会发现 `release/**` 内嵌的 Product Design 模板测试，导致旧包因缺少模板级 Playwright 依赖及 Windows 文件 URL 行为失败。应用当前源码测试本身为 438 项通过。历史发行文件是发布证据，不属于根应用测试入口。

### 离线 USB 布局测试输入条件

`tools/test-offline-bundle.ps1` 默认从 `release/setup` 读取 installer、manager 和 `CodexEntryShim.exe`。当前目录没有 shim，故默认运行在布局复制前按安全约束失败。`tools/build-installer.ps1` 的正常构建阶段会编译 shim、嵌入 asInvoker manifest 并复制到 setup 目录。使用现有包含匹配三件套的 USB release 目录执行相同合成 MSIX 布局测试，已通过（573 个输出文件、572 个校验和条目）。

这不是通过弱化 builder 来规避缺件：builder 必须继续拒绝缺少 shim 的 USB 输入。最终发行前仍必须从同次当前构建产出 installer/manager/shim，再以该目录运行离线布局测试。

## 最小改动

1. 在 `docs/audit-capability-pack.py` 为 `file-manifest.json` 和 `checksums.sha256` 比较新增 canonical SHA-256，文本文件按 `tools/refresh-capability-pack-metadata.py` 的 LF bytes 口径，二进制仍按原始字节校验。当前标注为 raw-file-sha256 的 tree fingerprint 保持原始字节语义不变。不得修改 Product Design 文件或重写其内部 `integrity.json`。
2. 在根项目配置 `vite.config.ts` 将 `release/**` 排除于根应用测试发现；保留现有 source、脚本专项及独立 CCSwitch 测试命令，不移动或删除 release 文件。
3. 不改 `test-offline-bundle.ps1` / `build-usb-layout.ps1` 的缺 shim 拒绝行为。预打包检查必须分清“构建输入尚未生成”和“构建产物不正确”。

## 验收步骤

1. `python docs/audit-capability-pack.py installer-pack` 返回 0，且不报告外层哈希差异；再执行 `python tools/refresh-capability-pack-metadata.py --check installer-pack`。
2. `npm test -- --run` 只运行根项目测试，不加载 release 内嵌模板测试；所有根项目测试通过。
3. `npm run build`、`npm run lint`、`npm run test:release` 通过。
4. 重新执行现有安装器链、MCP adapter、readiness、Rust 定向测试。
5. 执行 `tools/test-offline-bundle.ps1`，显式传入含同批 installer、manager、shim 的现有 USB release EXE，验证合成 MSIX 布局；记录它不覆盖真实签名和 Codex 启动。
6. 本轮不运行会生成正式 EXE 的 `tools/build-installer.ps1`。后续正式打包时，该脚本须先在同一 setup 目录生成 installer、manager、shim，再以该目录运行离线布局测试；真实 MSIX Authenticode、实体 U 盘启动仍是独立发行验收，不得用合成测试代替。

## 禁止事项

- 不改 Product Design 插件源文件、98 条内部哈希、manifest 行为或安装流程。
- 不排除 `release/**` 于能力包清单，不删除旧 release，也不改能力包打包过滤规则。
- 不通过允许缺失 shim、绕过其清单校验或更改授权门禁来让测试变绿。
- 不构建正式 EXE、不签名、不推送 GitHub、不运行真实 Codex；这些不属于本轮预打包门禁修复。

## 实施、独立审计与验证记录

- 方案独立审计：首轮指出配置文件路径、正式打包步骤边界、tree fingerprint 哈希语义三处问题；已修正文档，复审 PASS。
- 实施：`docs/audit-capability-pack.py` 清单/checksum 比对改用包元数据 helper 的 canonical hash；tree fingerprint 仍用原始字节。`vite.config.ts` 排除 `release/**`，没有改或删任何历史 release 文件。Product Design 插件及内部 98 个文件哈希均未修改。
- 独立只读代码审计：PASS，确认 canonical/raw 哈希用途分离、Vitest 排除范围正确、shim fail-closed 未放宽，且 Product Design 内容未改动。
- 通过：`docs/audit-capability-pack.py installer-pack`；`refresh-capability-pack-metadata.py --check installer-pack`（542 个清单文件、543 个校验和）；根 `npm test -- --run`（41 files、506 passed、1 skipped）；`npm run build`；`npm run lint`；`npm run test:release`（56）；installer adapter suite（58）；installer chain static suite（10）；Rust media readiness 定向测试（1）。
- USB 合成布局测试：传入含同批 installer/manager/shim 的既有 USB release 目录后通过，输出 573 文件并核对 572 个校验项。它使用合成未签名 MSIX，不代表真实 Authenticode 或 Codex 启动。
- 当前 `release/setup` 目录还没有本次正式构建生成的 `CodexEntryShim.exe`，因此不带参数运行 `test-offline-bundle.ps1` 会按预期 fail closed。正式构建脚本负责生成并验证同批 shim；本轮没有为让默认测试变绿而复制旧 shim 或弱化检查。正式构建后须以新 setup 目录重跑离线布局测试。
- 未执行正式 EXE/签名构建、真实签名 MSIX 验签、实体 U 盘冷启动或 Codex 本体启动；这些属于后续发行验收。
