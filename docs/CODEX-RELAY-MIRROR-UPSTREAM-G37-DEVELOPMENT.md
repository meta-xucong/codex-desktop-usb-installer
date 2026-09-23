# Codex Relay 直接镜像上游 G37

日期：2026-09-17  
状态：本地最小修复已完成；自动化与中转源只读复测通过；G37 U 盘包及便携签名器已生成；未修改 VPS，待真实 Windows/U 盘点击验收

## 1. 用户目标

用户不能修改第三方 GitHub 仓库，但 `Wangnov/codex-app-mirror` 仍是当前离线安装包可复用的来源，不能简单替换。最佳方案是让 VPS 自己做“候选源采集 + 架构化选择 + 同源发布”：保留 Wangnov GitHub 作为离线兼容源和回退源，同时只读采集国内镜像的 Windows x64 资产；VPS 发布自己的 protocol 2 relay 描述符，不让客户端直接感知上游来源。

## 2. 已冻结的事实

- 现有 VPS 定时器和同步服务本身可以正常执行，但 GitHub 上游可能长期落后于国内镜像。
- 初始设计快照中 relay/G35 的 x64 包是 `26.908.70816`；本次 2026-09-17 只读复核显示国内镜像的 x64 节点已经更新到 appVersion `26.911.61220`、MSIX package version `26.911.7940.0`。这只证明它现在具备“新 payload 候选”资格，不能把镜像响应直接当成完整 U 盘发行包。
- Wangnov 源的价值是已有离线安装所需的 MSIX、`release-manifest.json` 和 Windows checksum；它不需要被修改，不能因中转追新而破坏这条离线链路。
- 国内镜像 `/latest/manifest` 的顶层 `codexVersion` 不是架构选择结果；必须读取
  `sources.windows.architectures.x64`，使用该节点的 `appVersion`、package version、package moniker、content length。
- `currentForCodexVersion` 只能作为诊断字段，不能替代目标架构身份；Windows x64 是否有更新只由选中架构节点及其 checksum、大小和实际 MSIX identity 决定。当前镜像清单的 x64 `catalog` 仍残留旧的 `26.908.9136.0`/旧大小，必须在构建时以实际下载的 MSIX 做最终身份校验，不能仅凭该清单自动信任。
- 同步器发布前必须继续验证 manifest、Windows checksum、MSIX size/SHA-256，并保留现有原子 current 切换、锁、journal、磁盘门禁和旧版本保留策略。

## 3. 实现契约

### C1：候选源与发布源分离

VPS 内部增加两个候选源适配器，而不是把 config 的 `repository` 字符串改成 URL：

1. `wangnov-github`：读取 `Wangnov/codex-app-mirror` Release，继续提供离线兼容回退；
2. `agentsmirror`：只读固定镜像协议，作为追新候选源。

两者先分别完整解析为同一个 `ReleaseCandidate`，再按目标架构比较版本。不得出现“manifest 来自 A、MSIX/checksum 来自 B”的混包。VPS 对外仍只发布自己的 relay，不把候选源 URL 写入客户端配置。

镜像适配器只允许
`https://codexapp.agentsmirror.com/latest` 与其受控备用主机
`https://codexapp-r2.agentsmirror.com/latest`；禁止把任意 URL、HTTP、用户信息或跨域重定向当作上游。

### C2：架构准确的身份

读取固定路径 `manifest`、`checksums`、`win-x64`（配置架构为 arm64 时为
`win-arm64`），并从 manifest 的对应架构节点生成发布身份。顶层全局版本、其他架构版本和 `currentForCodexVersion` 不得直接替代目标架构版本。

发布 ID 使用架构、appVersion、packageVersion、packageMoniker、MSIX 大小、MSIX SHA、manifest SHA 和选中 checksum 记录的稳定摘要派生；相同版本的资产或校验内容变化必须生成新身份并重新发布。

候选选择规则固定为：

- 先过滤目标架构不可下载、版本格式非法、size/SHA/manifest/checksum 不完整的候选；
- 只比较 `sources.windows.architectures.x64.appVersion`（或配置架构对应节点），不比较顶层 `codexVersion`；
- 目标架构版本更高的候选优先；版本相同且完整身份相同则幂等；版本相同但 SHA、size、package identity 或 manifest 不同则判定为冲突并保留当前版本，不自动猜测；
- `currentForCodexVersion` 只记录诊断，不作为替代架构身份的版本字段；ARM 新版不能发布到 x64 relay。

### C3：完整性与同源

- manifest 必须是合法 JSON，包含 schema、publishedAt、derived.latestChecksums 和目标架构节点；目标架构必须可下载、大小为正且不超过上限。
- checksums 必须包含目标 MSIX 的 64 位 SHA-256；下载后的 MSIX、manifest、checksums 必须分别匹配预期 size/SHA。
- 下载 URL 只能由固定 base URL 加三个固定相对路径派生；允许的镜像重定向仍限于两个受控镜像主机。
- relay 描述符继续使用 protocol 2；资产没有 GitHub digest 时不伪造 GitHub 身份字段。

### C4：状态和失败

同步成功才切换 `export/current`；任何网络、源格式、校验、空间、权限或发布事务失败都保留旧 current。同步状态的读取和写入必须使用同一个 `sync-status.json`，避免 attemptCount 失真。

状态还应记录每个候选源的 `observedVersion`、`observedIdentity`、`result`，以及最终的 `selectedSource`、`selectedIdentity`、`sourceConflict`；不记录 token、响应正文或用户信息。没有新候选不是失败，源不可达、源格式错误、身份冲突和磁盘门禁则必须可区分。

### C5：非目标

不修改 Codex 客户端、U 盘 anchor/license、Skill Hub、签名工具、第三方镜像内容或业务站点；不降低完整性校验，不自动把全局/ARM 版本发布成 x64。

## 4. 为什么不直接把 Wangnov 替换成国内镜像

直接替换会同时带来三个风险：

1. Wangnov 是当前离线包链路的可复用来源，替换后可能出现“在线能下载、离线安装包无法复盘”的两套事实；
2. 国内镜像的顶层版本可能领先，但目标 Windows x64 仍是旧版本，容易把 ARM/全局版本误发布给 x64；
3. 直接改 `repository` 会绕过现有 GitHub 资产模型，若没有独立适配器，容易丢失 manifest、checksum、size 和同源校验。

因此采用“双候选、单发布”的方式：Wangnov 继续保证离线兼容，国内镜像负责发现新 x64 候选，VPS 只发布经过统一校验的一份内部版本。

## 5. 验收矩阵

- R1：国内镜像 manifest 的全局版本高于 x64 时，发布身份仍取 x64 实际版本。
- R2：只改变 generatedAt/无关架构字段不产生新 x64 身份；改变 x64 package、size、selected checksum 或 manifest 内容会产生新身份。
- R3：错误 checksum、manifest 绑定、size、非允许主机/协议和路径均 fail closed，旧 current 不变。
- R4：同一 x64 资产重复运行幂等；新 x64 资产可原子切换并清理超出保留数的旧版本。
- R5：timer/service 仍可执行；状态文件记录同一位置的 attempt/success/error。
- R6：本地 Python 单测和现有同步器夹具通过；远端只在本地版本固定后部署，并完成公网 latest.json、MSIX Range、SHA-256 和旧路由回归。
- R7：Wangnov 与国内镜像版本相同但身份不同会冲突停留；一个源不可达时另一个源仍可安全提供候选；两源都失败时 current 不变。
- R8：只读 dry-run 能显示两个源的目标架构版本、选中源和容量结果；不写 current、不删除旧版本。

## 6. 分阶段实施方案（本轮不执行）

### 阶段 A：本地设计和夹具

- 冻结 `ReleaseCandidate` 内部结构和源适配器接口；
- 用 Wangnov、agentsmirror 当前 x64 旧版、ARM 领先/x64 落后三种夹具验证版本选择；
- 覆盖同版本不同 SHA 冲突、镜像重定向、坏 checksum、manifest 绑定错误、磁盘不足和状态恢复。

### 阶段 B：VPS dry-run

- 只部署同步器和配置备份，不切换 current；
- 执行一次双源 dry-run，确认 Wangnov 与国内镜像观察值、目标 x64 身份、预计下载空间和选择结果；
- 若镜像只领先 ARM，dry-run 必须明确显示“x64 无新候选”，不能发布。

### 阶段 C：启用双源发布

- timer 改为 1 小时周期、10 分钟随机延迟，避免最多等待 6 小时；
- 新版本完整下载、校验、原子切换成功后才更新公网 `latest.json`；
- 公网回归 latest.json、MSIX Range、manifest/checksum SHA、客户端 Relay-only check/apply 和旧路由。

### 阶段 D：回滚

- 任何新源失败或身份冲突只保留旧 current；
- 可通过配置切回 `wangnov-github`，不删除当前有效版本；
- 不触碰 U 盘、客户端签名和 Skill Hub。

## 7. 完整双源方案的暂缓边界

本文件前述完整双源发布架构仍然暂不部署、不打包；本轮只实施第 8 节的本地最小修复。后续若批准完整方案，部署不需要重新打包 U 盘；但客户端只有在 relay 公网描述符切换到新的 x64 版本后，才会看到该更新。

## 8. G37-MIN：当前批准的最小本地修复

本节记录用户随后批准的“最小化改动”执行契约。它优先解决“检测到不可由 U 盘离线包安装的新版”这一故障，不提前实现本文件前述的 VPS 双候选发布架构。

### 8.1 允许改动

- Codex 自动源顺序从 `Mirror → Relay → Github` 收窄为 `Relay → Github`；两者均保持 Wangnov 离线包协议兼容链路。
- 保留 `Mirror` 枚举和解析分支以兼容旧配置，但旧配置在归一化时迁移为 `Auto`，并从设置界面移除“镜像源”可选项；不改变 Skill Hub 的源策略。
- 安装链的嵌套 Windows PowerShell、批处理入口和 Python 安装器调用显式使用隐藏窗口参数，避免一键安装过程中闪出 PowerShell；UAC、管理器、Codex、CCSwitch 等有意显示的 GUI 不隐藏。
- 为上述行为补充/调整最小契约测试、旧配置迁移测试和脚本静态检查。

### 8.2 不做的事

- 不把 `agentsmirror` 直接作为 Codex U 盘更新候选，不修改第三方仓库，不在本地实现 VPS 双候选同步器。
- 不删除已有 `Mirror` 协议代码，不重命名公开配置字段，不升级依赖，不改签名、授权、U 盘、Skill Hub、卸载和安装包发布逻辑。
- 不部署 VPS、不修改 Skill Hub 源策略；VPS 仍需单独验收。EXE/U 盘包和便携签名工具只在本地自动化验收通过后生成。

### 8.3 验收

- 自动模式的测试调用顺序只能为 `Relay → Github`，不得尝试 `Mirror`。
- 已保存的 `mirror` 配置归一化为 `auto`，并能继续使用 Wangnov relay/GitHub 兼容源；显式 `relay`/`github` 行为不回归。
- `Install-Codex-Starter.cmd`、`install-to-codex.ps1`、`install-wizard.ps1`、`install-all.ps1` 和 `runtime/provision-runtime.ps1` 的子进程调用均具备隐藏窗口证据；可见 GUI 入口仍不受影响。直接双击 `.cmd` 时的宿主 `cmd.exe` 短暂创建属于批处理宿主限制，不是其子 PowerShell 弹窗。
- 前端构建/定向测试、Rust 定向测试、脚本静态检查和 `git diff --check` 通过；未验证的真实离线安装和真实更新留待测试机验收。

## 9. G37 实际复测与产物回执（2026-09-18）

- Relay 现场只读检查：`https://update.404token.xyz/__codex_update__/latest.json` 返回 HTTP 200，协议 2、x64、`Wangnov/codex-app-mirror` 身份和 MSIX 资产字段均可解析；GitHub API 当时返回 rate limit，符合自动策略先使用 Relay、再尝试 GitHub 的预期。
- 自动源单元测试覆盖 Relay 命中、Relay 过期后 GitHub 选择、全源过期和后备失败，共 4/4 通过；Rust 全量测试 427 通过、3 忽略。
- 前端全量测试 423 通过、1 忽略；TypeScript、Lint、Vite 生产构建、安装链 5 项静态测试和安装包审计均通过。
- 新 U 盘包：`release/usb-oneclick-v0.5.24-g37-codex-update-source-20260918`；启动器和管理器使用同一版本化 profile，pair preflight 通过。
- 新便携签名工具：`release/usb-license-signer-gui-v0.5.24-g37-codex-update-source-20260918-portable`；portable vault 绑定复核通过，不依赖构建机/运行机 DPAPI。
- 当前仍需用户在真实签名 U 盘和干净 Windows 测试机上点击“检查更新”及执行安装；本回执不把网络只读检查冒充为真实 U 盘写入验收。
