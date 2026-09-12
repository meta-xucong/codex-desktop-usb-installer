# 一键更新整合与正式打包前验收

> 任务记录：2026-09-09
> CONTRACT_REV：`UPDATE-INTEGRATION-2026-09-09-v1`

## 用户原始要求

> [$multi-agent-dev](C:\Users\T14S\\.codex\\skills\\multi-agent-dev\\skills\\multi-agent-dev\\SKILL.md) 下面继续整合一键更新，把所有工作都做好，复盘审计，确认是不是真的可以执行打包后正式测试了？如果发现任何BUG就指出并修复

## 目标与非目标

目标是以 checkpoint `2f20c330757a1e5e47d618526cf1a185723c7525` 为基线，完成 Codex USB 一键更新链与一键安装/打包链的整合，确保：

1. 更新代码进入新构建的 USB 管理器 EXE，而不是继续使用旧的 `release/setup` 产物。
2. Codex payload 检查、用户确认、远端重解析、签名/哈希/identity 校验、事务替换和恢复在同一构建版本中闭合。
3. 完整能力包源文件、Skills、Agents、MCP、离线运行时和配置模板能从仓库源码生成 `content/codex-starter-kit`，不依赖工作区外的 sibling 仓库。
4. 一键 U 盘布局的清单、SHA256、启动器、管理器、CCSwitch、VC++ 和 WebView2 资源可以通过现有验证器复核。
5. 前端入口、失败提示、重复点击保护和已有 CCSwitch/组件修复路径不回归。

非目标：本轮不改变 Codex payload 的公共更新契约，不把“刷新 U 盘 payload”扩展为自动替换本机已部署 Codex，不提交/推送 GitHub，不凭空取得 Authenticode 或外部服务凭据，不把真实中国大陆 U 盘/多网络 E2E 冒充为已完成。

## 基线与门禁

- 起始版本：`checkpoint/pre-package-2026-09-09` / `2f20c330757a1e5e47d618526cf1a185723c7525`。
- 起始工作树：代码与暂存区已与 checkpoint 一致；`.workbuddy/`、`codex-learnings/` 为本机未跟踪记录，不属于本轮交付。
- `COMPLEXITY_GATE=ESCALATE_REQUIRED`：跨 Rust/Tauri、React、PowerShell、USB 载荷、离线能力包和生成物边界，涉及外部副作用与正式发布门禁。
- 唯一写入者：当前主控会话；只读审计线不得修改仓库。
- `AUDIT_OWNER`：独立只读审计 Agent（更新链、打包链各一条）；若平台结果不可复核，结论只能是“证据不足”。
- `CONTEXT_MODE=ARMED`：任务包含大量文件和长构建，进入压缩/交接前必须保存本记录、版本、差异、问题和测试状态。

## 允许改动范围

- `src-tauri/src/app/channels/codex.rs`、`update_channel.rs`、`commands.rs`、`lib.rs` 及其直接测试。
- `src/app/views/About.tsx`、相关 API/类型/测试；必要时修复组件重试错误展示。
- `tools/build-installer.ps1`、`build-usb-layout.ps1`、`build-oneclick-usb-layout.ps1`、`verify-*.ps1`、相关文档。
- 仓库内新增的 `installer-pack/` 完整能力包源目录及其必要清单/资源。
- `docs/UPDATE-INTEGRATION-DEVELOPMENT.md` 本任务记录和与最终行为直接相关的更新说明。

禁止顺手重构、升级无关依赖、修改既有公开更新 schema、上传生成 EXE/MSIX、写入凭据或改变卸载/本机 Codex 部署语义。

## 验收矩阵

| ID | 验收条件 | 证据 |
|---|---|---|
| R1 | Rust/前端类型、lint、相关单测通过 | 实际命令、退出码、固定版本 |
| R2 | Codex 更新正常、失败、旧版本、远端变化、校验错误和事务恢复路径有测试 | Rust 测试/最小夹具 |
| R3 | USB 授权、Windows 平台和命令注册闭合 | 源码核对 + 编译 |
| R4 | 仓库源码可找到完整能力包，且构建默认不依赖 sibling 目录 | 源码树、打包 smoke |
| R5 | 新构建 manager/launcher 的版本和 USB 清单契约一致 | 构建日志、PE/清单验证 |
| R6 | 最终布局含 50 Skills、7 Agents、MCP、runtime、配置模板、CCSwitch、VC++、WebView2 和 payload | `verify-oneclick-usb.ps1`、`test-offline-bundle.ps1` |
| R7 | 完整签名/哈希验证在真实资源存在时通过；未签名或缺资源明确阻断 | verifier 退出码与限制说明 |
| R8 | 已知外部限制单独记录，不把 fixture 或旧 EXE 当正式验收 | 本文及最终审计记录 |

## 问题记录

| 编号 | 状态 | 证据/影响 | 处理 |
|---|---|---|---|
| P1 | 已修复（工作树） | 能力包源默认位于仓库外 `openclaw-codex-kit\\installer-pack`，干净 checkout 无法构建完整 U 盘包。 | 已补齐当前仓库 `installer-pack/`（251 文件），两个布局脚本默认使用仓库内源；提交发布前必须把该目录纳入 Git 版本。 |
| P2 | 已修复 | `build-installer.ps1` 默认要求仓库外 `release\\ccswitch-product`，当前 checkout 已有经过验证的 `src-tauri\\resources\\ccswitch`，导致干净 checkout 无法默认打包。 | 已加入默认 fallback；显式 `-CcswitchArtifactDir` 仍保持严格失败。 |
| P3 | 已修复 | `release/setup/CodexAppManager.exe` 是旧生成物，不能证明当前更新代码已进入最终 EXE。 | 已从当前源码重建；最终包中的 manager 与 `release/setup/CodexAppManager.exe` SHA-256 一致，时间戳为 `2026-09-09 15:44:39`。 |
| P4 | 已修复 | 更新入口只在 About 中依据一次 USB 状态读取显示，启动竞态时可能永久不显示。 | 已加入最多 3 次、递增间隔的有界复检，并补充启动竞态测试；不会形成无限刷新。 |

## 本轮二次审计新增问题与处理

| 编号 | 状态 | 处理与剩余边界 |
|---|---|---|
| AUD-001 | 已修复 | Codex check/apply 现在要求签名许可证；开发版自动登记的 unsigned U 盘只能运行开发/安装路径，不能刷新 payload。 |
| AUD-002 | 已修复 | apply 在恢复事务前、恢复后分别重新确认授权 U 盘；check 会先恢复未完成事务；事务状态使用原子写入并在损坏时 fail closed。 |
| AUD-003 | 已修复 | check/apply/commit 都验证 `SHA256SUMS.txt`；确认对象携带 payload、两个 manifest 和 checksum 的本地快照，协调篡改会要求重新检查。新增回归测试覆盖。 |
| AUD-004 | 已修复 | staging、transaction、backup 和下载后的 staged 文件增加重解析点检查；提交后的事务清理失败不再触发错误回滚，而返回 `post_commit_cleanup` 可重试错误。 |
| AUD-005 | 已验证 | 最终包 `final-test2` 的能力包与当前 `installer-pack` 251 个文件逐一 SHA-256 一致；静态包有 50 Skills、7 Agents、1 个 Feishu MCP 描述及离线 runtime/config。 |
| AUD-006 | 外部阻断 | 当前环境没有组织 Authenticode 发布证书；最终功能测试包使用 `-SkipSignature`。严格 verifier 不带该参数会按设计拒绝未签名的 Launcher/Manager，不能把当前包称为公开发布包。 |
| AUD-007 | 证据不足 | 已完成隔离目录安装、离线合成 MSIX、静态 U 盘清单和所有自动化测试；尚未完成真实实体 U 盘拔插、真实 WebView2/VC 安装、真实 Codex 启动和真实远端更新/回滚。 |

## 版本冻结规则

代码修正后必须重新记录送审版本/差异和受影响测试；任何新增代码都使旧的“可打包”结论失效。最终结论只能是“符合”“不符合”或“证据不足”。

## 当前复核结论（2026-09-09）

当前工作树已经达到“可生成并开始正式功能测试包”的条件，但尚未达到“可公开发布”的条件：

- 可测试包：`release/usb-oneclick-v0.5.24-update-integrated-final-test2`。
- 打包与静态验证：`build-oneclick-usb-layout.ps1 -SkipSignature`、`verify-oneclick-usb.ps1 -SkipSignature -SkipNetwork` 通过。
- 离线模拟：`test-offline-bundle.ps1` 通过，输出 277 个文件、276 条根清单记录，不预建 `WindowsApps/WpSystem`。
- 能力包安装：直接从最终 U 盘包 `final-test2/content/codex-starter-kit/install-all.ps1` 安装到隔离用户目录成功：50 Skills、7 Agents，Python/Node/Feishu runtime 均 healthy，配置已合并，Feishu/API 未填凭据时保持关闭。
- 自动化回归：前端 31 个文件 / 317 个测试通过；Rust 主 crate 串行 349 passed、1 ignored；Windows engine 175 passed、1 ignored；能力包兼容性审计 PASS；适配器契约 34/34 PASS。

因此：可以进入“当前机器上的正式功能测试”，但必须把 `installer-pack/` 纳入最终 Git 提交，并在拥有真实发布证书后去掉 `-SkipSignature` 做发布验收；真实 U 盘、WebView2、VC、Codex 启动和远端更新仍需人工 Windows 测试记录，不能由本地 fixture 代替。

## 2026-09-09 主页可见性与页脚诊断补充

### 用户现象

USB 安装器主页只有“一键安装 / 重新校验 / 组件修复”等按钮，看不到上一轮
Codex payload 更新能力；页脚出现被截断的 `WindowsApps`、程序目录和 `AppX`
卷路径，容易被误认为报错。

### 根因

1. 生产 Windows 构建由 `src/app/views/Home.tsx` 选择 `OfflineHome`。更新入口此前
   只在 `About.tsx`，而且只对“签名 USB + watchdog 正常”显示；因此离线主页没有可见
   入口。这个隐藏条件是安全门禁，但不应该连入口说明一起隐藏。
2. `OfflineHome` 的 `.foot` 直接渲染 `installed.path` 和 `installed.packageVolume`。
   固定 400px 窗口无法容纳长路径，导致文字横向溢出/截断，并非后端错误。

### 本轮修复

- 主页新增“检查并安装更新”入口。正式签名 U 盘点击后执行：远端检查 → 用户确认 →
  `payload_apply_codex` 刷新 U 盘载荷 → 重新走既有受保护的暂存、校验和 U 盘程序部署
  链。后端 payload channel 仍保持“检查无副作用、apply 重新校验”的安全契约；入口在
  未签名开发包中保留但置灰，并通过悬浮提示说明原因，不绕过签名门禁。
- 长路径从可见页脚移到悬浮详情，页脚只显示“程序已就绪 · 悬停查看位置”，同时保留
  程序路径、AppX 卷和保持 U 盘连接的诊断信息。
- `OfflineHome.test.tsx` 新增签名更新链和未签名开发包可见性回归用例。

### 本轮证据

- `npm run check`：通过
- `npm run lint`：通过
- `npm run test:offline -- --reporter=dot`：18/18 通过

注意：当前 `final-test2` 测试包是未签名开发包；要实际执行远端更新，必须重新生成
并配置签名许可证的正式 USB 包。未签名包的更新按钮置灰是预期安全行为，不是入口
缺失。
