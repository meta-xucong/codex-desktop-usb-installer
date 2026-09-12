# 能力包 G16：就绪判定、配置兼容与安装状态修复开发文档

状态：已完成（2026-09-09；源码、包审计与隔离安装已验证，完整签名 USB EXE 尚未重建）  
范围：v0.5.24 能力包安装/修复链路，包含 Skills、Agents、本地 runtime 与 Codex MCP 配置的状态展示。  
本轮明确不改：卸载清理、CCSwitch、更新引擎、U 盘绑定与发布版本号。

## 1. 问题背景与复现证据

测试机使用的包为 `usb-oneclick-v0.5.24-readiness-fixed-20260909-v2`，界面先显示：

```text
update engine error: 能力包核心文件已返回，但 readiness 校验未通过
(Skills: 0, Agents: 0, 路径: C:\Users\37800\.codex\codex-agent-kit\readiness.json,
runtime: unknown, config: unknown)
```

随后同一机器生成的 `readiness.json` 已显示核心就绪：50 个 Skills、7 个 Agents、Python 3.12.10、52 个 wheel，健康检查通过；Feishu 未勾选且无凭据，状态为等待凭据。这说明“没有填写可选 API Key”不是失败原因，主要问题是状态读取、版本/包来源和配置状态表达不够稳健。

本次审计还发现，测试包内 `CodexAppManager.exe` 的 SHA-256 为：

```text
78A334C7034AB55543109E413FC16A64CD65AE48619125A931A396EDED1094C3
```

它早于当前源码构建的管理器二进制。以后必须把“测试的安装包是否包含本次源码”作为发布验收项，不能把不同构建的日志和源码混为同一版本。

## 2. 审计结论

### 2.1 必须修复的问题

| 编号 | 问题 | 影响 | 修复策略 |
| --- | --- | --- | --- |
| G16-UI-01 | 组件面板打开时异步调用 `capabilityPackStatus()`，安装动作完成后，旧查询仍可能回写旧状态 | 新安装结果被覆盖，用户看到 Skills/Agents 为 0 或错误 readiness | 引入请求序号/失效机制；安装开始时使旧查询失效；只有当前请求可更新状态 |
| G16-UI-02 | 查询进行中仍可点击安装；没有单独的 loading 状态 | 重复执行安装、状态抖动、结果难以解释 | 状态查询期间禁用能力包安装按钮；查询失败只提示，不阻止用户重试 |
| G16-UI-03 | `coreReady=true` 但 `configStatus=conflict/partial` 时，界面把 `success=false` 泛化为“运行时待修复” | 把“配置需要确认”误报成 Skills/runtime 未安装 | 按核心、runtime、配置三层分别展示；核心就绪时明确提示 Codex 配置冲突/需确认 |
| G16-CFG-01 | 未带新 managed marker、但内容与当前生成的 Feishu 配置完全一致的旧配置被当作冲突 | 老版本升级后无法幂等接管自己的配置 | 仅在规范化后完全一致时安全迁移到新 marker；内容有任何实质差异仍保留 conflict，不覆盖用户配置 |

### 2.2 记录但本轮不作为代码缺陷的问题

1. 未勾选飞书、DashScope、Image-2、Seedance，且没有 API Key/账号密码：这是可选项的正常跳过条件，不应失败，也不应写入半成品凭据配置。
2. 当前源码的 Rust 调用使用阻塞等待 PowerShell 子进程完成；安装脚本内部也同步等待并在末尾写入 readiness。因此“当前源码一定没有等待子进程”未被证实，不在本轮凭推测重写执行引擎。测试包与当前源码存在二进制版本漂移，应先用同一构建复测。
3. 配置 `conflict` 不等价于核心能力失败。它表示已有未托管内容未被覆盖，属于需要用户确认的配置状态。

## 3. 目标行为合同

### 3.1 安装状态模型

安装结果按三层解释：

```text
coreReady
  ├─ false  -> 核心文件/Skills/Agents 未就绪，允许“安装/修复”重试
  └─ true
       ├─ runtime ready + config ready -> 全部就绪
       ├─ runtime ready + config conflict/partial -> 核心已就绪，配置需确认
       └─ runtime partial/failed -> 核心已就绪，runtime 需要修复
```

`success=false` 不能单独决定文案。UI 必须优先检查 `coreReady`，再分别检查 runtime 与 config 状态。

### 3.2 前端并发合同

- 每一次打开面板产生一个 status request token。
- 关闭面板、开始安装、或产生更新结果时，旧 token 失效。
- 只有 token 仍有效的 status 请求可以写入 `packReport`/`error`。
- status 请求进行中，能力包安装按钮不可用；请求失败后恢复可用。
- 安装按钮的 busy 状态和 CCSwitch 重试状态继续相互独立。

### 3.3 配置迁移合同

- 新 marker 管理区块继续由 `configure-codex.ps1` 独占维护。
- 旧的 markerless Feishu 区块只有在去除空白/注释后，与当前生成的 Feishu 区块完全一致时才允许迁移。
- 区块内容不同、嵌套表结构无法完整识别、或存在多个候选区块时，返回 conflict，绝不删除或覆盖。
- 未启用的可选 MCP 不生成配置区块，也不要求凭据。
- 操作保持幂等：同一输入重复执行不会产生重复 Feishu 区块。

## 4. 实施方案

### 4.1 前端状态查询竞态

在 `OfflineComponentsSheet` 中使用单调递增的 request id（或等价的取消失效机制）：

1. `open` 从 false 变为 true 时开始查询并显示 loading。
2. 查询返回时比较 request id；旧查询只丢弃，不得覆盖最新结果。
3. 点击安装前使所有旧查询失效，并进入 `busy=pack`。
4. 安装结果直接成为当前报告；finally 中恢复按钮状态。
5. 查询失败不把安装入口锁死，用户仍可直接安装/修复。

`OfflineHome` 的一键安装继续等待 `installCapabilityPack()` 返回；它不与组件面板的状态查询并行启动。自动安装失败时保留静默入口，但必须留下可读的组件修复入口。

### 4.2 配置状态文案

新增准确文案：

- 核心能力已就绪；Codex 配置存在未托管冲突，未覆盖现有配置。
- 核心能力已就绪；配置需要确认。

保留独立的 runtime 修复文案，避免把配置冲突误称为运行时缺失。

### 4.3 旧配置安全迁移

脚本按 TOML 顶层区块边界识别 `[mcp_servers.feishu]` 及其 `mcp_servers.feishu.*` 子区块，不用单一短正则截断嵌套表。迁移前对区块做稳定规范化比较：去除空行和注释、统一换行与首尾空白，但不隐藏实际键值差异。仅 exact match 才迁移；否则沿用 conflict。

### 4.4 构建与包来源控制

本轮源码修复完成后必须重新构建管理器和安装包。验收记录至少包括：

- 管理器 EXE 的绝对路径、修改时间和 SHA-256；
- 包内 EXE SHA-256 与构建输出一致；
- `file-manifest.json` 与 `checksums.sha256` 在脚本变更后重新生成；
- 安装包中的 `install-to-codex.ps1`、`configure-codex.ps1`、前端构建产物与源码提交属于同一构建。

## 5. 写入边界与协作分工

为避免并行 agent 互相覆盖，本轮采用唯一写入者：

| 角色 | 唯一写入边界 | 交付 |
| --- | --- | --- |
| 前端执行 agent | `src/app/views/OfflineComponentsSheet.tsx`、对应测试、`OfflineHome.tsx`、对应测试、`src/app/i18n.tsx` | 竞态保护、按钮状态、准确文案和 Vitest |
| 安装脚本执行 agent | `installer-pack/configure-codex.ps1`；必要时 `installer-pack/test-adapter-contracts.py` | 安全迁移、冲突保留和脚本测试 |
| 主控 agent | 本开发文档、构建/集成、生成清单、最终审计记录 | 合同冻结、集成验证、回归与发布证据 |
| 独立审计 agent | 只读，不改代码 | 检查范围、竞态、配置安全、测试覆盖和版本一致性 |

任何 agent 不得回滚不属于自己边界的既有用户改动。

## 6. 验收与测试矩阵

| ID | 验收项 | 通过标准 |
| --- | --- | --- |
| R1 | 旧 status 查询晚于安装结果返回 | 最终 UI 保留安装返回的报告，不回退到旧报告 |
| R2 | status 查询期间点击安装 | 按钮不可点击；查询失败后可重试；无重复请求 |
| R3 | `coreReady=true` + config conflict | 显示配置冲突/需确认，不显示 runtime 未就绪 |
| R4 | 所有可选服务未勾选、凭据为空 | 安装成功或按核心结果展示，不因空凭据抛错 |
| R5 | markerless exact-match Feishu 配置 | 迁移为 marker 管理区块，重复执行幂等 |
| R6 | markerless modified Feishu 配置 | 返回 conflict，原文保持不变 |
| R7 | 脚本契约/前端单测/类型检查 | 全部通过，无新增 lint/type 错误 |
| R8 | 安装包来源一致性 | 包内 EXE、源码构建、manifest/checksum 可相互核验 |

建议命令（在仓库根目录执行）：

```powershell
npm run test -- src/app/views/OfflineComponentsSheet.test.tsx src/app/views/OfflineHome.test.tsx
npm run typecheck
python installer-pack/test-adapter-contracts.py
python installer-pack/audit-codex-compatibility.py --root installer-pack
```

若仓库实际脚本名不同，以 `package.json` 和现有构建脚本为准，但不得跳过 R8。

## 7. 审计记录模板

最终合并前补齐以下证据：

```text
源码提交/工作树：
管理器构建产物：
管理器 SHA-256：
安装包目录：
包内管理器 SHA-256：
R1：PASS/FAIL，证据：
R2：PASS/FAIL，证据：
R3：PASS/FAIL，证据：
R4：PASS/FAIL，证据：
R5：PASS/FAIL，证据：
R6：PASS/FAIL，证据：
R7：PASS/FAIL，证据：
R8：PASS/FAIL，证据：
独立审计结论：
剩余风险：
```

## 8. 不在本轮掩盖的剩余风险

- 测试包如果不是本次源码重新构建，任何行为结论都必须标注“包版本漂移”，不能以旧 EXE 证明新代码失败。
- 真实 Codex 配置可能包含用户自定义 TOML 结构；脚本无法证明归属时必须宁可 conflict，不得自动覆盖。
- 本轮不会把配置冲突自动改写成成功，也不会为了让 UI 变绿而放宽 readiness 核心计数。
- PowerShell 子进程、文件系统权限及杀毒软件造成的真实安装失败，仍需使用同一构建的完整日志单独诊断。

## 9. 本轮执行结果（2026-09-09）

代码、测试与包审计已完成：

- 前端全量 Vitest：34 个测试文件、391 项通过；能力包/组件定向测试 38 项通过。
- `npm run check`（由 build 执行）、`npm run lint`、`npm run build`：通过；renderer entry isolation 校验通过。
- `cargo fmt -- --check`、`cargo check`：通过。第一次并行检查出现增量目录权限 warning，关闭并行后以 `CARGO_INCREMENTAL=0` 重跑通过。
- `test-adapter-contracts.py`：38 项离线契约通过，含 Feishu exact-match、注释/空行、普通嵌套表、array-table、修改冲突和幂等测试。
- `audit-codex-compatibility.py --root installer-pack`：通过，包含 50 Skills、7 Agents、19 个连接字段、25 个 Feishu 工具；249 个 manifest 文件与 250 条 checksum 记录通过 canonical hash 校验。
- `install-wizard.ps1 -ValidateOnly`：通过，19 个连接字段；所有连接默认 opt-in，空字段不会启用服务。
- 使用最终 `installer-pack` 在隔离临时用户目录执行 `install-all.ps1 -DisableFeishu`：退出码 0；readiness 为 `overall=ready`、50 Skills、7 Agents、Python 3.12.10、52 wheels、Feishu disabled、config merged、secretsStored=false。该测试目录位于系统 Temp，不是用户实际 Codex 目录。
- 发布镜像双源/公开路由集成测试在 Windows 上显式设置 30 秒超时，避免默认 5 秒超时触发异步清理竞争；该测试单独通过，未修改发布逻辑。
- 第二轮独立只读审计确认 R1–R7 通过，无 P0/P1/P2 代码缺陷；runtime 失败与 config conflict 的组合优先级已单独覆盖。R8 的能力包部分由主控以 canonical hash 审计和隔离一键安装复核通过。

哈希口径也已统一：`install-to-codex.ps1` 和 `audit-codex-compatibility.py` 对 `.cmd/.env/.in/.json/.lock/.md/.ps1/.py/.sha256/.toml/.txt/.yaml/.yml` 先统一为 LF，再计算 SHA-256；二进制仍按原始字节计算。这样不会因 Windows CRLF 把合法包误判为损坏。

本轮没有重新构建并签名完整 USB EXE，也没有执行真实 Feishu/API RPC；因此“最终 USB EXE 与源码同源”和真实外部服务可用性仍应在发布构建/异机验收阶段复核，不能用本地 renderer 构建代替。
