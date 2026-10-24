# Log 13 最小修复开发文档

## 目标与冻结基线

- 目标：修复 log13 中能力包安装状态、Product Design 展示和 U 盘 Codex 完整访问权限的可见性/状态问题。
- 基线提交：`b9940aed14a9b74cbe9561021aa5d6a195d3a991`，分支 `codex/windows-usb-ci-scope-20261016`。
- 已知证据：log13 记录能力包安装脚本成功退出，随后失败于媒体 MCP readiness：runtime/protocol 已通过，`registered=false`。Product Design 在 Skill Hub Apply 阶段报告 ready。完整访问开关的参数传递和配置写入代码已存在于基线。
- D/I/A：D0；I1（三处已定位、复用现有机制）；A2（涉及 Codex 沙盒/审批配置）。

## 范围与不做事项

只处理以下现有代码路径，不重构安装器状态机：

1. `installer-pack/install-to-codex.ps1` 的 readiness 写入。
2. `src-tauri/src/app/capability_pack.rs` 的媒体失败提示。
3. Skill Hub Apply 结果里现有的 Product Design 提示。
4. `install-all.ps1` → `install-to-codex.ps1` → `configure-codex.ps1` 的完整访问参数链只验证；若隔离模拟通过，不再改生产逻辑。
5. 因 `install-to-codex.ps1` 属于能力包签名清单，代码修改后用现有 `tools/refresh-capability-pack-metadata.py` 刷新 `pack.json`、`file-manifest.json`、`checksums.sha256`，并用 `--check` 复核。
6. 修正整套适配器回归中三处已定位的测试漂移，不改媒体 MCP/runtime 生产逻辑：删除参考图恢复测试使用的无效占位图片字节，改用有效 PNG；让 MiniMax 错误断言匹配当前拒绝语义；让 runtime 静态断言匹配当前参数表调用形式。

不新增网络请求、插件目录扫描、状态查询 IPC、凭据读取、Codex 启动/关闭、provider 调用或安装过程中的额外探测。Product Design 继续作为插件独立显示，不加进 Skill Hub 在线 Skills 数量。不得修改主机 `%USERPROFILE%\.codex`。工作区现有未跟踪临时目录和审计证据均属于本任务基线，不得清理、覆盖或纳入提交；隔离测试只可创建并清理本次新建且带唯一标识的临时目录。

## 根因与最小修法

### 1. 媒体 readiness 字段缺失

Rust 的 `readiness_summary` 读取 `connections.mediaGeneration.registered` 和 `enabled`，缺失时按 `false` 处理；`media_install_ready` 要求两者为真。安装脚本已生成媒体 MCP 配置，并有 `$renderStatus.mediaGenerationEnabled`、`$configStatus.merged`，但 `connectionStatus.mediaGeneration` 未输出这两个字段。

在现有 `connectionStatus.mediaGeneration` 对象里增加：

```powershell
$mediaRegistered = [bool]($mediaGenerationEnabled -and $renderStatus.mediaGenerationEnabled -and $configStatus.merged)
registered = $mediaRegistered
enabled = $mediaRegistered
```

用同一结果同时填两字段，避免一个为真、另一个为假的分叉状态。配置未成功合并或媒体开关未开时，两者必须为 `false`。不改 Rust readiness 条件，不把字段硬编码为 `true`。

媒体 readiness 失败的错误文案改为“核心能力包已安装，但媒体 MCP 未就绪……”，避免把媒体连接问题理解成整个能力包没有安装。保持现有 fail-fast 行为和诊断路径。

### 2. Product Design 提示

基线已有 `SkillRepoApplyReport.productDesignSkillCount` 和 Apply 成功结果行。本任务只把提示文案改成直白状态，例如“Product Design 插件已安装，包含 {skills} 项技能”。继续使用现有 Apply 报告，不添加检查 API 或持久状态。该提示只在本次 Apply 结果中展示；离开后重新进入页面不保留这条提示。

基线中上一轮临时加入的 `skill_repo_product_design_status` 查询、`productDesignPluginStatus` 文件扫描、`SkillRepoUpdateInfo.productDesignSkillCount` 检查状态字段及对应页面查询逻辑均应移除；它们不是本方案需要的持久能力。

### 3. 完整访问权限

基线已有完整链路：USB 安装调用传 `-EnableFullAccess`；`install-all.ps1` 接收并转发；`install-to-codex.ps1` 传给配置合并器；`configure-codex.ps1` 写入 `approval_policy = "never"` 与 `sandbox_mode = "danger-full-access"`。复用现有静态契约测试及 `test-adapter-contracts.py` 的隔离配置测试。

若测试通过，不修改这条链路。不得用启动器 `no_sandbox` 字段代表 Codex 沙盒状态。测试只确认目标临时 `CODEX_HOME/config.toml`；不访问或改写真实用户配置。

## 允许修改的文件

- `docs/LOG13-MINIMAL-CAPABILITY-STATUS-REPAIR-20261022.md`（本文件）
- `installer-pack/install-to-codex.ps1`
- `installer-pack/test-adapter-contracts.py`
- `installer-pack/pack.json`
- `installer-pack/file-manifest.json`
- `installer-pack/checksums.sha256`
- `tools/test-installer-chain-window-static.py`
- `src-tauri/src/app/capability_pack.rs`
- `src/app/i18n.tsx`
- `src/app/views/SkillRepoUpdateCard.tsx`
- `src/app/views/SkillRepoUpdateCard.test.tsx`
- `src-tauri/src/app/skill_repo_update.rs`
- `src-tauri/src/commands.rs`
- `src-tauri/src/lib.rs`
- `src/services/managerApi.ts`
- `src/shared/types.ts`

后面 5 个 Product Design 状态查询文件只允许删除上一轮新增的查询路径/临时字段；保留原有 Apply report 和结果行。

## 最小验收

1. 在唯一临时 CODEX_HOME 中运行正式 `render-codex-config.ps1` 和 `configure-codex.ps1`，渲染时调用真实媒体 MCP 本地 healthcheck；启用媒体时回读 `config.toml`，确认 `[mcp_servers.media_generation] enabled=true`，并验证 readiness 公式得到 true；媒体关闭时确认配置为 disabled 且公式为 false，模拟配置合并失败时公式也为 false。完整 `install-to-codex.ps1` 会调用 runtime provisioner 设置当前 Windows 用户级环境变量，本地不执行整段安装以免改变宿主机状态；readiness 生产表达式由静态契约测试核对。
2. 对 readiness JSON 做管理器读取回归：字段缺失仍 fail closed；明确写入成功状态时通过注册/启用门槛。继续保留既有 runtime、protocol、文件与授权检查。
3. 在 `tools/test-installer-chain-window-static.py` 增加两条静态断言：`capability_pack.rs` 将 `-EnableFullAccess` 传给 `install-all.ps1`；`install-to-codex.ps1` 收到该开关后继续传给 `configure-codex.ps1`。已有断言覆盖 `install-all.ps1` 接收/转发。再运行 `installer-pack/test-adapter-contracts.py` 验证隔离配置写入；用临时 `CODEX_HOME` 回读 TOML 确认两个值。不得对正在运行的 Codex 做冷启动验收。
4. 前端已有 Apply 结果测试增加/调整一条断言，确认成功安装后显示 Product Design 插件及技能数；Skill Hub 在线安装数保持不变。
5. 运行 `python tools/refresh-capability-pack-metadata.py` 后再以 `--check` 验证能力包 source tree、文件清单和校验和一致；然后运行聚焦测试、`installer-pack/test-adapter-contracts.py` 全套及必要的 Rust 编译/测试。最后固定 diff，由独立只读审计复核方案边界、结果及测试证据。

## 发布边界

本任务目标是源码和隔离安装验收，不包含 EXE/USB 包签名、实体 U 盘冷启动、GitHub 合并或部署。基础 checkpoint 已推送到上述 GitHub 分支；最终修复需要另一个提交，只有独立审计和聚焦测试通过后才可交付验收。

## 实施与验收记录

- 实现完成；独立只读代码审计：PASS（源码范围）；主控执行的隔离配置链模拟：PASS。审计确认 Product Design 状态查询/目录扫描已移除，Apply 结果提示保留；媒体 readiness 与完整访问参数链符合本文约束。
- 隔离模拟：调用正式 `render-codex-config.ps1`、`configure-codex.ps1` 和真实媒体 MCP `--healthcheck`；在唯一临时 CODEX_HOME 中回读启用、关闭两种 `config.toml`，验证注册门槛真/假及完整访问值；通过。未运行整段 `install-to-codex.ps1`/runtime provisioner，因为其会写当前用户级环境变量；未操作真实用户 Codex 配置或做 Codex 冷启动。
- 通过：前端定向测试 12 项；TypeScript `npm run check`；完整访问隔离配置契约 7 项；安装链静态测试 10 项；Rust 媒体 readiness 定向测试 1 项；能力包元数据 `--check`（542 个清单文件、543 个校验和）。
- 全套 `installer-pack/test-adapter-contracts.py` 最终 58 项通过。只修正了无效 PNG fixture 和与当前行为不符的断言（MiniMax 未适配时明确拒绝、runtime 使用参数表调用）；没有改媒体 MCP/runtime 生产逻辑。
- 最终能力包元数据检查通过：542 个清单文件、543 个校验和；source tree SHA-256：`990831cb3c432059425fb484597887456e71a5347421b2200fa94e0d04b7b0cc`。
