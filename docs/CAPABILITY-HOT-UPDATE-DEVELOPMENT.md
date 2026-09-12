# Skill 热更新能力包开发文档

状态：设计回流版本，待独立复审

契约版本：`skill-hot-update-v1.1`

任务记录模式：`MONITOR_MODE=DELIVERY_SILENT`

复杂度门禁：`COMPLEXITY_GATE=ESCALATE_REQUIRED`

## 1. 用户原始要求与本次目标

用户原始要求：

> 备注信息：落文档阶段先做，开始落代码时，和codex://threads/01a07097-9c26-76f2-95e6-5b44e679fef3联调，确保工作目录统一，不要因各自的工作导致内容混乱。可以等对方完成当前的工作，再开始你这里的落代码任务

结合前序要求，本任务目标冻结为：

1. `installer-pack` 基础能力包继续永久随程序预装，不参与热更新。
2. 以 GitHub `meta-xucong/skill_hub` 的 `master` 分支为唯一内容源。
3. 在 Codex App 管理器中增加独立的“检查能力包更新”入口，用户确认后一次更新。
4. 中国大陆网络不可直连 GitHub 时，优先使用现有 Codex 更新 relay；relay 不可用时再直连 GitHub。
5. 热更新只安装额外 Skills，不改 U 盘 Codex 本体、USB 清单、签名/Anchor、默认能力包、MCP、Agent、运行时或凭据。
6. 不新增签名体系；下载后仍校验 HTTPS 响应、固定仓库/分支、commit 和 SHA-256。Skill Hub 是独立的能力覆盖层，刻意不纳入 Codex/U 盘签名门禁，正常状态不显示“未签名”警告。

契约决定：接受 `add-only`。基础包同名 Skill 不通过热层优先级覆盖；热更新只增加或更新不属于基础包的独立 Skill。

## 2. 非目标与硬边界

- 不改 `content/codex-starter-kit`、USB `manifest`、`usb-manifest`、`SHA256SUMS.txt`、Anchor 或 USB license。
- 不调用现有 USB 授权守卫作为热更新前置条件；热更新不要求插入 U 盘，也不因 U 盘签名状态失败而阻断。
- 不覆盖基础包中的 Skill；同名直接报告冲突并跳过，不能通过热更新替换基础实现。
- 不覆盖用户已有且未被热更新器登记所有权的目录或文件。
- 不删除仓库中已从上游移除的本地热 Skill；V1 只新增和更新，避免误删用户内容。
- 不更新 Agent、MCP 配置、Python/Node 运行时、可执行文件、DLL、安装包、API Key 或环境变量。
- 不自动运行仓库中的脚本；下载和安装阶段只解压、校验、复制和记录状态。
- 不做后台自动更新、不改 Codex 普通自更新、不改 manager 自更新。
- 不要求 GitHub Release、GitHub Actions、独立发布签名或新增数据库/常驻服务。
- 本次本地代码任务不提交、暂存、推送或部署；代码阶段也必须保留基线中的既有未提交改动。第 6 节的 relay 部署只是外部交付阶段的固定操作契约，不授权本地任务执行公网部署。

V1 的更新对象是仓库顶层可识别的 Skill 目录：目录内必须有 `SKILL.md`。目录内的 `scripts`、`references`、`templates` 等静态附属文件可以随 Skill 一起安装，但管理器不执行它们。

## 3. 基线、分工与联调门禁

代码仓库：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

起始基线：

- 分支：`checkpoint/pre-package-2026-09-09`
- HEAD：`2f20c330757a1e5e47d618526cf1a185723c7525`
- 基线工作区已有大量用户改动和未跟踪文件；设计及后续实现不得覆盖、回滚或冒充这些改动。
- 当前相关实现：`src-tauri/src/app/capability_pack.rs`、`src-tauri/src/app/channels/codex.rs`、`src-tauri/src/app/update_channel.rs`、`src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`、`src/services/managerApi.ts`、`src/app/views/OfflineHome.tsx`。
- 现有 relay 设计及路由：`docs/UPDATE-RELAY-DESIGN.md` 的静态镜像、独立 HTTPS edge 和原子发布方案。

分工：

| 职责 | 负责人 | 写入边界 |
| --- | --- | --- |
| 主控/集成 | 当前会话 | 任务记录、契约冻结、集成、最终复验 |
| 设计思考 | 独立只读 Agent | 不得写文件，只输出契约风险 |
| 代码执行 | 后续唯一写入者 | 仅修改第 7 节允许文件 |
| 设计/代码审计 | 与执行者分离的只读 Agent | 不修改被审版本 |
| 联调对象 | `codex://threads/01a07097-9c26-76f2-95e6-5b44e679fef3` | 代码阶段前必须确认其回合已终止、无进行中写入 |

代码启动条件：联调对象已确认 `idle`，最新回合 `completed`，但代码阶段开始前仍必须再次读取状态和工作区；只有状态为 `idle` 且没有进行中回合/写入，当前会话才能成为唯一写入者。若状态未知或重新变为活动，暂停代码，不启动替代写入者，不覆盖其文件。

联调状态复核记录（本轮读取）：`thread=01a07097-9c26-76f2-95e6-5b44e679fef3`，`cwd=D:\AI\AGENT_U盘`，`status=idle`，最新回合 `completed`，未观察到进行中写入。此记录只允许代码启动前作为初步 receipt；开始代码前必须重新读取并记录新的状态证据。

## 4. 最小方案

### 4.1 内容与版本

固定源：

```text
仓库：meta-xucong/skill_hub
分支：master
检查提交：https://api.github.com/repos/meta-xucong/skill_hub/commits/master
GitHub 归档模板：https://github.com/meta-xucong/skill_hub/archive/<commit>.zip
```

不建立新的包格式。客户端下载仓库 ZIP，按 commit 记录版本；relay 只提供同一 ZIP 的静态副本和 metadata。

relay 使用独立的 skill-hub 命名空间，不复用 Codex payload 的 `export/current`：

```text
https://update.404token.xyz/__codex_update__/skill-hub/latest.json
https://update.404token.xyz/__codex_update__/skill-hub/archive.zip
```

`latest.json` 最小结构：

```json
{
  "schemaVersion": 1,
  "repo": "meta-xucong/skill_hub",
  "branch": "master",
  "commit": "<40-char-sha>",
  "archive": "archive.zip",
  "size": 123456,
  "sha256": "<64-lowercase-hex>"
}
```

客户端顺序固定为 relay → GitHub。`archive` 只能等于 `archive.zip`，不接受 metadata 提供的任意 URL。relay 规范定位固定为 `https://update.404token.xyz/__codex_update__/skill-hub/archive.zip`；GitHub 规范定位固定为 `https://github.com/meta-xucong/skill_hub/archive/<commit>.zip`。客户端不得把 `master.zip` 作为下载地址。

GitHub 回退先请求 commits API 得到 commit，再请求该 commit 的归档。由于 GitHub 回退没有独立远端 digest，check 会完整下载、计算 size/SHA-256 并把结果放入 expected identity；apply 必须重新下载同一 commit 归档并与 check 的 size/SHA-256 完全一致，否则返回 stale/verify error，不安装。check 选定的 `sourceKind`、repo、branch、commit、size、SHA-256 在 apply 中必须保持一致；apply 不从 relay 静默切换到 GitHub，也不反向切换。两源均不可用时不进入安装。

apply 的远端一致性检查是固定的：relay 在下载前请求 `latest.json`，下载后再次请求同一地址，并要求两次 metadata 的 repo、branch、commit、archive、size、sha256 完全一致，且下载期间 `current` 仍解析到同一 commit release；任一变化返回 `Err(SKILL_REPO_STALE)`。GitHub 在下载前后各请求一次 `commits/master`，两次必须得到同一 commit；归档只能由该 commit 重建，下载后的 size/SHA-256 必须同时匹配 check 的 expected identity，否则返回 `Err(SKILL_REPO_STALE)` 或 `Err(SKILL_REPO_HASH_MISMATCH)`。上述校验失败不进入解压和安装。

其中“下载期间 `current` 保持同一 release”是 relay 服务端的原子发布不变量，不要求客户端读取不可公开的 symlink。客户端可执行的验收固定为：apply 前 metadata、归档实际 size/SHA-256、apply 后 metadata 和 check 的 expected identity 全部一致；服务端的 `current` 只允许从完整 release 原子切换，旧 release 必须保留到发布成功且 metadata/归档同源校验完成。

传输边界固定为：relay 的两个公开文件禁止 HTTP 重定向；GitHub 只允许 HTTPS，commits API 和归档最多跟随 3 次重定向，重定向目标主机只能是 `api.github.com`、`github.com` 或 `codeload.github.com`，最终 URL 不写入 identity。metadata 响应最多 64 KiB，单次网络请求超时为 10 秒，单次 check/apply 网络阶段总时限为 45 秒，前端桥接保护为 50 秒；超时必须退出 busy 状态并显示可重试的网络错误。归档仍受原始 64 MiB 上限和 expected size 限制，`Content-Length` 只作早期拒绝提示，最终以实际字节数为准。下载写入 staging 后、解压前以及每次目标目录交换前，重新检查目标根和所有父级不是 reparse point；发现变化立即返回 `SKILL_REPO_ARCHIVE_INVALID`，不写目标目录。

### 4.2 本地热覆盖层

本地热更新状态目录：

```text
%USERPROFILE%\.codex\skill-hub-updater\
  state.json
  journal.json
  logs\
```

实际 Skill 仍安装到 Codex 现有标准目录：

```text
%USERPROFILE%\.agents\skills\<skill-id>\
```

本地 staging 和 backup 必须位于目标父卷下，避免跨卷 rename；state/journal 临时文件只在自己的 state 目录内原子替换：

```text
%USERPROFILE%\.agents\.skill-hub-staging\<transaction>\
%USERPROFILE%\.agents\.skill-hub-backups\<generation>\
```

不另造 Codex 运行时搜索路径；“热层独立”由所有权账本和基础包冲突规则保证，而不是让 Codex 同时读取两个目录。

首次运行时若 `state.json` 不存在，且基础清单校验通过，按下面 schema 创建空 ledger（`skills={}`）是合法的；已存在但缺失、损坏、版本错误或路径不一致的 ledger 必须 fail-closed。

首次创建的实际字节内容固定为 `{"schemaVersion":1,"repo":"meta-xucong/skill_hub","branch":"master","lastSeenCommit":null,"lastApplyStatus":"never","lastGoodGeneration":null,"skills":{}}`（UTF-8、无 BOM、LF），随后按 state 目录内的临时文件加文件级 SHA-256 校验后原子替换。普通 Manager 不依赖 USB 才能做基础冲突判定，因此构建时从 `installer-pack/manifest/skill-audit.json` 生成只读资源 `src-tauri/resources/skill_repo_base_manifest.json`，随 Manager 永久预装；该资源只包含基础 Skill ID 和源清单 SHA-256，不复制或修改基础 Skill 内容。资源固定 schema 为 `schemaVersion=1`、`source="installer-pack/manifest/skill-audit.json"`、`sourceSha256="e6c6a610b4bab5dc9001b3846c087bdaba83fcd8c26908fd6eabc135f4ca2a9c"`、`skills[].id`，`skills[].id` 必须覆盖源清单的全部 ID 且唯一。构建阶段必须用现有根目录 `installer-pack/file-manifest.json`、`installer-pack/checksums.sha256` 校验源文件并将资源纳入普通 Manager 的 `resources/*`；运行时资源缺失、版本错误、ID 重复、源 SHA 不符或无法定位时统一返回 `SKILL_REPO_LEDGER_INVALID`，不得把 USB、用户运行时目录或猜测出的路径当作基础清单。`installer-pack` 基础能力文件本身不改。

`state.json` 固定 schema：

```json
{
  "schemaVersion": 1,
  "repo": "meta-xucong/skill_hub",
  "branch": "master",
  "lastSeenCommit": "<40-char-sha>|null",
  "lastApplyStatus": "never|success|partial|failed|blocked",
  "lastGoodGeneration": "<generation>|null",
  "skills": {
    "<normalized-id>": {
      "installedCommit": "<40-char-sha>",
      "directoryVolumeId": "<windows-volume-id>",
      "directoryFileId": "<windows-file-id>",
      "files": [{"path": "SKILL.md", "size": 123, "sha256": "<64-hex>", "volumeId": "<windows-volume-id>", "fileId": "<windows-file-id>"}]
    }
  }
}
```

`journal.json` 固定 schema：

```json
{
  "schemaVersion": 1,
  "transactionId": "<uuid>",
  "phase": "prepared|backed-up|swapped|state-committed|rollback-required|done",
  "expectedIdentity": {"repo": "...", "branch": "master", "commit": "...", "sourceKind": "relay|github", "archiveSize": 123, "archiveSha256": "..."},
  "previousStateSha256": "<64-hex>|null",
  "previousStateFile": "<generation>/previous-state.json|null",
  "backupGeneration": "<generation>",
  "rollbackRoot": "rollback",
  "skills": [{"id": "<normalized-id>", "action": "add|replace", "targetRelative": "<id>", "backupRelative": "<generation>/<id>|null", "rollbackRelative": "<id>|null", "oldInstalledCommit": "<40-char-sha>|null", "newInstalledCommit": "<40-char-sha>", "oldDirectoryVolumeId": "<windows-volume-id>|null", "oldDirectoryFileId": "<windows-file-id>|null", "newDirectoryVolumeId": "<windows-volume-id>|null", "newDirectoryFileId": "<windows-file-id>|null", "oldFiles": [{"path": "...", "size": 1, "sha256": "...", "volumeId": "<windows-volume-id>|null", "fileId": "<windows-file-id>|null"}], "newFiles": [{"path": "...", "size": 1, "sha256": "...", "volumeId": "<windows-volume-id>|null", "fileId": "<windows-file-id>|null"}]}]
}
```

`rollbackRoot="rollback"` 相对于本次 staging transaction 目录，`rollbackRelative` 只能是 ID 对应的单层子路径，实际恢复路径为 `transaction/rollback/<id>`；新增 Skill 的旧目录和 rollbackRelative 为 `null`，replace Skill 才可记录旧目录。journal 与 generation 的所有路径均由后端按规范化 ID 重算，不接受 renderer 提供的路径。

`previousStateFile` 是本次 apply 前完整 `state.json` 的逐字节副本；`previousStateSha256` 必须匹配该副本，初次无 state 时二者均为 `null`。state、journal 和 generation 的每个文件记录都必须同时持久化 `volumeId`、`fileId`，每个 Skill 目录记录 `directoryVolumeId`、`directoryFileId`；只有尚未物化的新 Skill 可为 `null`。恢复不得只根据逐 Skill 文件列表猜测旧 ledger。

阶段只允许单向转移：`prepared`（staging 和 journal 完成）→ `backed-up`（全部旧热 Skill 已备份）→ `swapped`（全部新目录已替换）→ `state-committed`（state 临时文件已校验并原子替换）→ `done`（staging 清理完成）。任一阶段失败先写 `rollback-required`；冲突项不进入 `skills` 变更集合，也不算事务失败。

进程启动、check、apply 和 rollback 都先在同一跨进程锁下恢复 journal：`prepared` 按下一段规则恢复 rollback 条目、确认旧状态后再清理 staging 并保留旧状态；`backed-up` 或 `swapped` 恢复全部旧目录和 `previousStateSha256`；`state-committed` 只有在新目录、state 和 generation 清单全部哈希一致时才标记 `done`，否则回滚；`rollback-required` 只允许恢复或报告阻断。恢复失败返回 `SKILL_REPO_ROLLBACK_FAILED`，禁止新的写入。

持久化顺序固定为：先在 state 目录写入并 flush `journal.json.tmp`，原子替换为 `journal.json`；staging 完整写入、逐文件核对后，逐个 copy 旧热 Skill 到 generation 临时目录，核对源/备份清单，再把旧目录原子 rename 到本事务的 rollback 目录；只有所有备份完成并 flush 后才把 journal 推进到 `backed-up`。随后把 staging 目录原子 rename 到目标目录，重新核对并 flush 后才推进 `swapped`；generation.json 写临时文件并 flush，最后写 `complete.marker`（内容为 generation.json SHA-256）并 flush；state 写临时文件、核对完整内容与哈希后原子替换，才推进 `state-committed`；最后清理 staging/rollback 临时目录并把 journal 推进 `done`。任何“操作已完成但 phase 尚未持久化”的崩溃窗口，启动恢复按更保守的前一阶段检查目标、staging、rollback 和 generation marker，能证明新状态完整才前进，否则恢复旧目录和旧 state。Windows 文件 flush 使用 `FlushFileBuffers`，目录/rename 持久化使用平台可用的父目录 flush；无法确认持久化结果时返回 `SKILL_REPO_ROLLBACK_FAILED` 并禁止新的写入。备份采用“先完整 copy 并核对，再 rename 当前目录到事务 rollback 目录”的顺序，不以未核对的 rename 充当备份。

为闭合“旧目录已移动但 `backed-up` 尚未持久化”的窗口，`prepared` 恢复不能直接删除 staging：必须逐项扫描 journal 的 `rollbackRelative`，对目标缺失且 rollback 存在的项先恢复旧目录，对目标和 rollback 同时存在的项先将仅当作新清单匹配的目标移入隔离目录再恢复旧目录；每项恢复后核对旧 file-id、文件清单和 `previousStateSha256`，任一不一致即 `SKILL_REPO_ROLLBACK_FAILED` 并保留所有证据目录。只有全部旧目录恢复成功，才清理 staging/隔离目录并结束恢复。

generation 固定为 `%USERPROFILE%\.agents\.skill-hub-backups\<generation>\generation.json`，其完整 schema 为：

```json
{
  "schemaVersion": 1,
  "generationId": "<generation>",
  "commit": "<40-char-sha>",
  "complete": true,
  "previousStateFile": "previous-state.json|null",
  "previousStateSha256": "<64-hex>|null",
  "skills": [{"id": "<normalized-id>", "installedCommit": "<40-char-sha>", "backupRelative": "<id>|null", "directoryVolumeId": "<windows-volume-id>", "directoryFileId": "<windows-file-id>", "files": [{"path": "...", "size": 1, "sha256": "...", "volumeId": "<windows-volume-id>", "fileId": "<windows-file-id>"}]}]
}
```

另写同目录 `complete.marker`，内容为 generation.json 的 SHA-256；有旧 state 时必须同时存在并匹配 `previous-state.json`。`lastGoodGeneration` 只能指向存在、marker 一致、`complete=true` 且全部哈希核对通过的 generation。账本缺失、版本错误、JSON 损坏、路径越界或 rollback 失败均 fail-closed，不猜测所有权。

`state.json` 只记录上述状态、文件清单和备份引用，不写入凭据、用户内容或任意 renderer URL。

### 4.3 冲突与所有权

基础 Skill 集合只来自随 Manager 预装的 `resources/skill_repo_base_manifest.json`，该资源在构建阶段由 `installer-pack/manifest/skill-audit.json` 生成，集合取源文件 `skills[]` 的全部 `id`，不按 `defaultEnabled` 缩小；构建时必须按安装包根目录的 `file-manifest.json`/`checksums.sha256` 和现有文本 canonical-LF 规则核对源文件，并要求源 `schemaVersion=2`、ID 唯一。运行时只读取资源副本，不读取 USB 输出路径；资源缺失、损坏或无法定位时 fail-closed，不把运行时目录猜作基础清单。hot 模块不得 import 或调用 `usb_guard`、`usb_bundle`、`capability_pack` 或 Codex payload 通道来完成这项读取。

Skill ID 只允许 ASCII `[a-z0-9][a-z0-9._-]{0,63}`，按 ASCII 小写规则比较，目录名不得是 Windows 保留名；归档内路径同样只接受 ASCII，以避免 Windows Unicode 规范化差异造成的路径碰撞。账本中的路径永远由 ID 和固定目标根重新计算，不能作为写入指令。

每个 Skill 逐项按以下优先级判定，不能因目标目录缺失跳过更高优先级：

1. ID 属于基础包：报告 `SKILL_REPO_BASE_CONFLICT`，不修改；基础 Skill 目录缺失也不能收养为热 Skill。
2. ID 在热账本：账本本身有效且目标目录存在时，当前文件集合与账本逐路径、逐大小、逐 SHA-256 完全一致才允许更新；目标缺失、账本损坏或 ledger 结构不符报告 `SKILL_REPO_LEDGER_INVALID` 并阻断该项；账本有效但目标文件集合或哈希被用户改变时报告 `SKILL_REPO_HOT_MODIFIED`，不猜测所有权。
3. ID 不属于基础包且不在热账本，但目标目录已存在：报告 `SKILL_REPO_USER_CONFLICT`，不修改。
4. ID 不属于基础包、不在热账本且目标目录不存在：安装为热 Skill。

冲突只影响对应 Skill；其他无冲突 Skill 可以继续安装，最终状态只能是 `success` 或 `partial`，不能把 `lastSeenCommit` 冒充成完整同步。只有所有归档 Skill 均已成功处理且没有冲突、跳过或失败时，才把 `lastSeenCommit` 推进到本次 commit；`partial`、`failed` 或 `blocked` 均保留旧 `lastSeenCommit`，且只要当前 latest commit 仍未完整处理，check 的 `updateAvailable` 必须为 `true`。一次 apply 仍使用事务记录：任一复制、重命名或账本写入失败时，恢复本次全部已替换 Skill 和旧 `state.json`，保持旧版本可用。

V1 不做“上游仓库已删除 Skill”的全局删除同步，不覆盖用户文件，不提供强制覆盖开关。更新已登记热 Skill 时，如新归档移除某个旧文件，只有在该文件仍逐文件哈希匹配 ledger、且 journal 已记录为本次 owned file 的情况下才删除；否则整项报告冲突。显式 rollback 在任何删除、替换或恢复前，必须逐文件核对当前目标与该 generation 的 `newFiles`、文件 ID、目录清单和 reparse 状态；额外文件、用户修改、file-id 变化或目录身份变化均返回 `SKILL_REPO_HOT_MODIFIED`（目标内容变化）或 `SKILL_REPO_ROLLBACK_FAILED`（恢复证据不足），保留目标和证据目录，不执行删除。只有完整核对通过，rollback 才可以删除本次 generation 新增的热 Skill/owned 文件并恢复上一完整 generation；这不属于上游删除同步。

### 4.4 下载和解压边界

- 只接受固定仓库、固定分支、固定 commit 的 metadata；renderer 不能提供任意 URL、路径或目标目录。
- 下载完成后同时检查 HTTP 成功、文件大小、SHA-256 和 ZIP 可读性。
- 解压前拒绝绝对路径、`..` 穿越、重复目标、大小写/Unicode 碰撞、符号链接/重解析点、特殊文件、Windows ADS/保留名、原生二进制（`.exe`、`.dll`、`.msi`、`.sys`、`.ocx`、`.scr`）和无法归属到 Skill 目录的文件。
- 路径规范化算法固定为：目录条目先去除恰好一个末尾 `/`（仅用于识别目录，不允许把内部 `/` 变为空段），文件条目不得以 `/` 结尾；随后 UTF-8 解码 → Unicode NFC → `/` 转 `\` → 按 `\` 分段 → 拒绝空段、`.`、`..`、NUL、`:`、尾随空格/点、`CON`/`PRN`/`AUX`/`NUL`/`CONIN$`/`CONOUT$` 及 `COM1..COM9`/`LPT1..LPT9`（忽略扩展名）→ 以 Windows OrdinalIgnoreCase 比较作为最终碰撞判定，Unicode 小写键只能作预筛选；目标根不存在时，只能在已核对为非 reparse 的 `.agents` 父目录下安全创建；叶 Skill 目录及缺失的中间目录可由安装器创建，并在创建后逐级重新检查不是 reparse point；绝不要求新 Skill 的叶目录事先存在。
- Windows 写入实现必须使用目标根句柄相对的、禁止跟随 reparse point 的打开/创建/复制/rename 操作（`CreateFileW` 配合 `FILE_FLAG_OPEN_REPARSE_POINT` 或等价安全 API）；不允许先按字符串路径检查再用普通路径 API 写入。每次创建/打开子目录后重新核对句柄的 volume/file-id 和 reparse 属性，句柄身份变化或出现 reparse 立即中止并回滚；staging、backup、state 目录同样适用。rename 只能在已核对的同一父卷、句柄相对目标内执行。
- 压缩包原始大小最多 64 MiB、最多 4096 个条目、解压后总计不超过 256 MiB、单文件不超过 32 MiB；超限直接拒绝，避免 ZIP bomb 或无界 staging。
- GitHub 前缀规则固定：所有非目录条目必须共有一个形如 `skill_hub-<40hex>` 的首级目录；只剥离这一层，任何条目落在该前缀之外都拒绝。剥离后根目录只允许 `README.md`、`LICENSE`、`LICENSE.txt`、`.gitignore` 四个文件名；其余内容必须位于一个 Skill 根目录，每个根目录必须有唯一大小写精确的 `SKILL.md`。
- `SKILL.md` 必须是无 BOM UTF-8；LF/CRLF 均接受，读取时仅移除每行末尾一个 CR，首行必须精确为 `---`，在前 64 KiB 内有唯一结束行 `---`。frontmatter 允许空行；其他行必须先按单个 `:` 分成 key 和 value，key 只允许 ASCII 字母开头、后接 ASCII 字母/数字/`_`/`-`，冒号后必须恰好一个空格，value 不得含 CR/LF、tab、重复 key、嵌套 YAML 或多行内容；必须有非空 `name` 和 `description`，未知 key 仅允许标量值；`name` 必须与规范化目录 ID 一致。所有文件除扩展名黑名单外，还必须拒绝内容前缀 `MZ`、ELF `0x7f454c46` 或 Mach-O `0xfeedface/0xfeedfacf/0xcffaedfe/0xcefaedfe`。
- 归档结构或任一 Skill 的安全/frontmatter 校验失败时拒绝整个归档，不做部分安装；只有基础/用户/已修改热 Skill 冲突才逐项报告并跳过。
- 只将 staging 中经过检查的 Skill 复制到目标目录；不执行压缩包内任何程序或脚本。
- 失败清理 staging，不改现有基础 Skill、用户 Skill 或当前热 Skill。

### 4.5 UI 与公共接口

目标平台固定为 Windows。`src/app/views/WinHome.tsx` 和 `src/app/views/OfflineHome.tsx` 的现有页面内容区各挂载一次独立 `SkillRepoUpdateCard`；`src/app/views/Home.tsx` 只负责既有平台分发，不改路由。该卡片在普通 WinHome（无 USB）和 OfflineHome 均可见，不读取 USB 状态，不要求 U 盘；MacHome 明确不显示。组件实现位于 `src/app/views/SkillRepoUpdateCard.tsx`，文案位于 `src/app/i18n.tsx`；它不并入 `OfflineComponentsSheet` 的基础包修复表单。

用户流程：

```text
点击按钮
  → check：下载并校验 staging 归档，显示当前 commit、最新 commit、可更新/冲突/跳过数量
  → 用户明确确认
  → apply：下载、校验、冲突过滤、事务安装
  → 显示成功/部分成功/失败及日志位置
```

check 只写独立 staging，不写目标目录、U 盘或基础包；apply 必须携带 check 返回的完整 expected identity，apply 内部重新解析同一 source kind、repo、branch、commit，并用固定规则重建 archive locator，发现 source switch/stale 时拒绝并要求重新 check。不会自动重启 Codex；提示用户新建会话或重新加载。UI 以紧凑的“安装 Skill Hub”入口承载详情，弹层展示检查、安装、部分成功语义和 `回退到上一完整版本` 入口；不展示固定仓库账号、URL 或 source kind，也不把 Skill Hub 的独立签名状态渲染成警告。

拟新增的最小 Tauri 命令（唯一 wire error 类型为现有 `CommandError`）：

```text
skill_repo_check_update() -> Result<SkillRepoUpdateInfo, CommandError>
skill_repo_apply_update(expected: SkillRepoExpected) -> Result<SkillRepoApplyReport, CommandError>
skill_repo_rollback() -> Result<SkillRepoRollbackReport, CommandError>
```

`SkillRepoExpected` 固定字段：`schemaVersion=1`、`repo`、`branch`、`commit`、`sourceKind`（`relay|github`）、`archiveSize`、`archiveSha256`。下载定位只由后端根据 `sourceKind+commit` 构造，renderer 不得传 URL、路径或目标目录。

`SkillRepoUpdateInfo` 固定字段：`schemaVersion=1`、`currentCommit`（无记录时为 `null`）、`latestCommit`、`sourceKind`、`archiveSize`、`archiveSha256`、`expected: SkillRepoExpected`、`updateAvailable`、`installableCount`、`conflictCount`、`skippedCount`、`warning`（无警告为 `null`）。Skill Hub 的正常响应将 `warning` 置为 `null`；该字段保留用于未来真正的非致命运行时提示，不用于表达其不受 USB 签名约束这一设计。

`SkillRepoItem` 固定字段：`id`、`status`（`applied|conflict|skipped|failed`）、`code`（成功为 `null`，否则只能为统一错误码）、`fileCount`。

`SkillRepoApplyReport` 固定字段：`schemaVersion=1`、`status`（`success|partial|failed|blocked`）、`identity`（`SkillRepoExpected`）、`applied: SkillRepoItem[]`、`conflicts: SkillRepoItem[]`、`skipped: SkillRepoItem[]`、`failed: SkillRepoItem[]`、`lastGoodGeneration`（无可用 generation 为 `null`）、`reloadedRequired: boolean`、`logPath: string|null`。`stale` 不作为 report 状态，只以 `Err(SKILL_REPO_STALE)` 返回。

`SkillRepoRollbackReport` 固定字段：`schemaVersion=1`、`status`（`success|failed|blocked`）、`restoredGeneration: string|null`（失败为 `null`）、`restoredSkills: string[]`、`reloadedRequired: boolean`、`logPath: string|null`。

Rust/TypeScript 的字段和 nullability 以同一 wire schema 为准，禁止只靠文本约定：

```text
SkillRepoExpected { schemaVersion: number; repo: string; branch: string; commit: string; sourceKind: "relay"|"github"; archiveSize: number; archiveSha256: string }
SkillRepoUpdateInfo { schemaVersion: number; currentCommit: string|null; latestCommit: string; sourceKind: "relay"|"github"; archiveSize: number; archiveSha256: string; expected: SkillRepoExpected; updateAvailable: boolean; installableCount: number; conflictCount: number; skippedCount: number; warning: string|null }
SkillRepoItem { id: string; status: "applied"|"conflict"|"skipped"|"failed"; code: string|null; fileCount: number }
SkillRepoApplyReport { schemaVersion: number; status: "success"|"partial"|"failed"|"blocked"; identity: SkillRepoExpected; applied: SkillRepoItem[]; conflicts: SkillRepoItem[]; skipped: SkillRepoItem[]; failed: SkillRepoItem[]; lastGoodGeneration: string|null; reloadedRequired: boolean; logPath: string|null }
SkillRepoRollbackReport { schemaVersion: number; status: "success"|"failed"|"blocked"; restoredGeneration: string|null; restoredSkills: string[]; reloadedRequired: boolean; logPath: string|null }
```

Rust 结构体必须 `Serialize + Deserialize + #[serde(rename_all = "camelCase")]`，Tauri 错误只序列化为 `CommandError { code: String, message: String }`。非 Windows 后端的三个命令统一 fail-closed 返回 `SKILL_REPO_UNSUPPORTED`，不访问源、不触碰目标目录；该行为加入 R8/R12 验收。

状态转换固定为：`idle` → `checking` → `ready|uptodate|error`；`uptodate|error` 只能回到 `idle` 后重新 check；确认后 `ready` → `applying` → `success|partial|failed|blocked`；`failed|blocked` 只能在 journal 恢复成功或显式刷新后回到 `idle`；只有存在完整 `lastGoodGeneration` 时 `success|partial` → `rolling-back` → `success|blocked`。网络源变化、commit/size/hash 变化统一由 apply 返回 `Err(SKILL_REPO_STALE)` 或 `Err(SKILL_REPO_HASH_MISMATCH)`，不生成半成品 report。同一时间只允许一个 check/apply/rollback，第二个命令返回 `SKILL_REPO_LOCKED`；check 也在整个下载/解析期间持有同一 `OperationManager` 更新锁。

错误码固定为：`SKILL_REPO_UNAVAILABLE`（源不可达）、`SKILL_REPO_TIMEOUT`（网络阶段超时）、`SKILL_REPO_SOURCE_INVALID`（固定源身份不符）、`SKILL_REPO_STALE`（check/apply 身份变化）、`SKILL_REPO_HASH_MISMATCH`（size/SHA 不符）、`SKILL_REPO_ARCHIVE_INVALID`（ZIP/Skill 结构不符）、`SKILL_REPO_BASE_CONFLICT`、`SKILL_REPO_USER_CONFLICT`、`SKILL_REPO_HOT_MODIFIED`、`SKILL_REPO_LEDGER_INVALID`、`SKILL_REPO_LOCKED`、`SKILL_REPO_ROLLBACK_FAILED`、`SKILL_REPO_UNSUPPORTED`。Tauri 命令统一返回 `Result<T, CommandError>`，wire shape 固定为现有 `CommandError { code: string, message: string }`，Rust/TS 业务结构统一 `#[serde(rename_all="camelCase")]`/camelCase；源错误和事务错误以 `Err(CommandError)` 返回，逐 Skill 冲突进入 report，不使其他安全 Skill 自动失败。前端只显示稳定错误码对应文案，不回显 URL、凭据或原始异常。

前端 API 只包装上述命令：

```text
checkSkillRepoUpdate()
applySkillRepoUpdate(expected)
rollbackSkillRepo()
```

不复用现有 `payload_apply_codex`，不把 Skill 更新伪装成 USB payload 更新。V1 禁止复用现有 `ChannelKind::SkillRepo`、`UpdateContext`、Codex payload channel 或任何携带 USB 根目录/授权快照的更新实现；它们的既有语义属于 U 盘资源。若需 UI 显示分类，只能使用前端本地字符串，不新增或改写既有 channel 枚举。

## 5. 无签名方案的明确取舍

本版本不做发布签名。SHA-256 只能发现下载损坏或 relay 与 metadata 不一致，不能在 GitHub 仓库、relay 或 HTTPS 信任链被攻陷时证明内容来源未被替换。因此：

- 只允许固定的自有仓库和 `master` 源；不提供用户可编辑源配置。
- relay 只做可达性缓存，不是新的信任根。
- 用户文档保留独立热层的信任边界说明；UI 不把该设计渲染成异常或警告状态，基础包仍保持原有离线安装保护。
- 未来若需要更强真实性，可在不改变本地热层接口的情况下增加签名字段和校验，但不属于本次实现。

## 6. Relay 扩展

复用现有 relay 的独立目录、只读 edge、锁、原子发布和失败保留 current 版本的机制；Skill Hub 固定布局如下，不改变现有 Codex payload 的 `export/current`、资产或 URL：

```text
/srv/codex-update-relay/export/skill-hub/
  current -> releases/<commit>/
  releases/<commit>/
    latest.json
    archive.zip
/srv/codex-update-relay/state/skill-hub-sync.lock
/etc/codex-update-relay/skill-hub.json
```

`current` 是同一文件系统内的相对 symlink，edge 只读挂载 `export`；公网只暴露 `skill-hub/latest.json` 和 `skill-hub/archive.zip`，不暴露 `releases`。

edge 只允许两个精确公开文件，不能用宽 alias 暴露 current 目录：

```nginx
location = /__codex_update__/skill-hub/latest.json {
    alias /srv/codex-update-relay/export/skill-hub/current/latest.json;
    default_type application/json;
    limit_except GET HEAD { deny all; }
    add_header Cache-Control "no-store" always;
    add_header X-Content-Type-Options "nosniff" always;
}

location = /__codex_update__/skill-hub/archive.zip {
    alias /srv/codex-update-relay/export/skill-hub/current/archive.zip;
    limit_except GET HEAD { deny all; }
    add_header Cache-Control "no-store" always;
    add_header X-Content-Type-Options "nosniff" always;
}

location ^~ /__codex_update__/skill-hub/ {
    return 404;
}
```
同步任务执行：

1. 请求固定仓库 `master` 的 commit。
2. 下载该 commit 的不可变 ZIP，而不是 branch-head ZIP。
3. 在独立 relay staging 计算 ZIP 大小和 SHA-256。
4. 生成并校验 `latest.json`。
5. 完整文件写入并校验后，以临时目录原子切换 `skill-hub/current`。
6. 同步失败时继续提供旧的 current，不生成半包。

relay 不执行 Skill 内脚本，不把仓库内容复制到现有心理测试服务目录，不新增数据库、常驻 API 服务或明文 HTTP 下载入口。

独立配置 `tools/skill-hub-relay-config.json` 固定字段为 `repository`、`branch`、`githubCommitUrl`、`githubArchiveTemplate`、`exportRoot`、`workRoot`、`stateRoot`、`lockFile` 和 `maxPublishedReleases`；同步器只写 `workRoot`、`exportRoot/skill-hub` 和 `stateRoot`，失败保留旧 `current`。

部署由现有 relay 主机的独立 systemd 单元负责，不改 Codex payload 同步单元：`tools/skill-hub-relay-sync.service` 的唯一执行入口固定为 `/usr/bin/python3 /opt/codex-update-relay/skill-hub-relay-sync.py --config /etc/codex-update-relay/skill-hub.json`，`tools/skill-hub-relay-sync.timer` 固定为 `OnCalendar=hourly`、`Persistent=true`。单元使用独立 service user、工作目录、锁文件和日志目标；禁止通过安装脚本、常驻 API 或共享 Codex payload state 注入配置。部署检查必须同时验证两个新单元、配置路径和三处独立根目录存在。

源文件到部署路径映射固定为：`tools/skill-hub-relay-sync.py` → `/opt/codex-update-relay/skill-hub-relay-sync.py`（root:codexskillrelay、`0755`）；`tools/skill-hub-relay-config.json` → `/etc/codex-update-relay/skill-hub.json`（root:codexskillrelay、`0640`）；`tools/skill-hub-relay-sync.service` → `/etc/systemd/system/skill-hub-relay-sync.service`（root:root、`0644`）；`tools/skill-hub-relay-sync.timer` → `/etc/systemd/system/skill-hub-relay-sync.timer`（root:root、`0644`）；`tools/codex-update-relay-edge.conf` → `/opt/codex-update-relay-edge/conf.d/default.conf`（root:root、`0644`），并同步保留 `/opt/codex-update-relay-edge/nginx.conf` 作为同内容 root-owned 审计副本。部署前后配置必须严格校验 `repository=meta-xucong/skill_hub`、`branch=master`、`githubCommitUrl=https://api.github.com/repos/meta-xucong/skill_hub/commits/master`、`githubArchiveTemplate=https://github.com/meta-xucong/skill_hub/archive/{commit}.zip`，拒绝任意自定义源或模板；relay service user 仅对独立 work/export/state 根和独立 lock 有写权限。

部署配置的固定值为：

```json
{
  "repository": "meta-xucong/skill_hub",
  "branch": "master",
  "githubCommitUrl": "https://api.github.com/repos/meta-xucong/skill_hub/commits/master",
  "githubArchiveTemplate": "https://github.com/meta-xucong/skill_hub/archive/{commit}.zip",
  "exportRoot": "/srv/codex-update-relay/export",
  "workRoot": "/srv/codex-update-relay/work/skill-hub",
  "stateRoot": "/srv/codex-update-relay/state",
  "lockFile": "/srv/codex-update-relay/state/skill-hub-sync.lock",
  "maxPublishedReleases": 5
}
```

`skill-hub-relay-sync.service` 固定 `User=codexskillrelay`、`Group=codexskillrelay`、`WorkingDirectory=/opt/codex-update-relay`、`ReadWritePaths=/srv/codex-update-relay/export/skill-hub /srv/codex-update-relay/work/skill-hub /srv/codex-update-relay/state`、`StandardOutput=journal` 和 `StandardError=journal`；该用户不能写 `/srv/codex-update-relay/export/current` 或 Codex payload 的其他路径。同步器启动时拒绝配置中任何与 Codex payload `export/current`、work、state 或 lock 重叠的路径。

edge 部署继续使用现有 `/opt/codex-update-relay-edge/docker-compose.yml` 的配置目录 bind mount：宿主机目录 `/opt/codex-update-relay-edge/conf.d` 只读挂载到容器目录 `/etc/nginx/conf.d`，源文件写入宿主机的 `conf.d/default.conf`，`/opt/codex-update-relay-edge/nginx.conf` 仅作审计副本；部署前执行 `docker compose -f /opt/codex-update-relay-edge/docker-compose.yml config`，更新配置后执行容器内 `nginx -t`，两者均通过才 reload/重建独立 edge。不得在容器内直接编辑配置，也不得修改现有 80 端口 Web Nginx。

## 7. 允许修改的文件与唯一写入边界

设计阶段本文件是唯一新增文件。

代码阶段预计只允许以下范围，若需超出必须提交 `DESIGN_QUESTION` 并回到契约审查：

- Rust：`src-tauri/src/app/skill_repo_update.rs`（新增）、`src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/app/mod.rs`、`src-tauri/src/app/paths.rs`、`src-tauri/src/errors.rs`、`src-tauri/resources/skill_repo_base_manifest.json`（新增构建资源）；必须复用现有 `app::oplock::OperationManager` 的跨进程 `Update`/`Install` 互斥，不新增第二套锁。
- 前端：`src/services/managerApi.ts`、`src/services/managerApi.test.ts`、`src/shared/types.ts`、`src/app/views/WinHome.tsx`、`src/app/views/OfflineHome.tsx`、新增 `src/app/views/SkillRepoUpdateCard.tsx`、新增 `src/app/views/SkillRepoUpdateCard.test.tsx`、`src/app/i18n.tsx`；不修改 `Home.tsx`、MacHome 或全局样式。
- 测试：上述 Rust/TypeScript 文件内的定向测试，以及新增 `tools/skill-hub-relay-sync.test.mjs` 的固定 ZIP/ledger/journal/relay 夹具；不改无关测试夹具。
- relay：新增 `tools/skill-hub-relay-sync.py`、`tools/skill-hub-relay-config.json`、`tools/skill-hub-relay-sync.test.mjs`、`tools/skill-hub-relay-sync.service`、`tools/skill-hub-relay-sync.timer`，以及 `tools/codex-update-relay-edge.conf` 的独立 location；不把 skill-hub 逻辑混入 Codex payload 同步脚本。
- 文档：本文件的实现状态、审计记录和验证证据。

禁止修改：USB bundle 内容、`installer-pack` 基础能力文件、USB license/Anchor、Codex payload 更新实现、普通 manager updater、第三方依赖版本。

唯一写入者：代码阶段开始前根据联调对象状态重新指定一名执行者；执行者完成后停止写入并固定版本，审计 Agent 只读。热更新 `check/apply/rollback` 全程使用现有 `app::oplock::OperationManager` 的跨进程 `OperationKind::Update` 锁（check 的网络等待也在锁内，保证同一时间只有一个 updater）；基础包 install/repair 在实际写入前使用同一 manager 的 `OperationKind::Install` 锁；Codex/manager 卸载沿用其现有 `Uninstall`/destructive token 锁。三类操作共享同一个 manager lock file 的作用域，不能并发改 `%USERPROFILE%\.agents\skills`。

基础包入口的硬门禁是：`install_capability_pack` 在调用任何会创建、替换或删除 `%USERPROFILE%\.agents\skills` 内容的安装/修复实现之前，必须先取得 `OperationKind::Install`；锁获取失败直接返回既有锁错误，不能先写后锁。代码审计必须以 hot apply、基础 install/repair、卸载和第二个 Manager 实例的交叉并发测试证明这一点。

阶段 A 只冻结上述必须满足的契约，不把当前工作区的既有实现误当作已通过；已知 `install_capability_pack` 当前缺少该 Install 锁属于阶段 B 的明确修复项，代码阶段必须在允许的 `commands.rs` 内补齐，并在未修复前不得宣称 R11 通过。

## 8. 验收矩阵

| ID | 验收条件 | 证据 |
| --- | --- | --- |
| R1 | relay metadata/commit-specific ZIP 可成功检查，显示 commit 变化；无更新时不替换目标目录 | 单测或可复现 mock 记录 |
| R2 | relay 失败后可切换 GitHub commit archive；两源失败时 apply 不开始，apply 不跨源混用 | 网络失败和 source-switch 测试 |
| R3 | ZIP 大小、SHA-256、ZIP 结构、commit 或 check/apply identity 不符时拒绝并保留旧状态 | 失败测试、旧状态哈希 |
| R4 | 新 Skill 可安装到标准目录；热 Skill 且哈希未变可更新 | 安装后文件清单和 state.json |
| R5 | 基础 Skill、用户已有目录、被用户修改的热 Skill 均不被覆盖 | 冲突测试及目标目录前后哈希 |
| R6 | ZIP-slip、大小写/Unicode 碰撞、链接/重解析点、特殊文件、原生二进制、64 MiB 归档/4096 条目/256 MiB 解压/32 MiB 单文件配额、非法 frontmatter 和无 `SKILL.md` 均拒绝整个归档；仅基础/用户/热 Skill 冲突逐项跳过；不执行包内脚本 | 固定 ZIP 夹具逐项断言 |
| R7 | 在 `prepared`、`backed-up`、`swapped`、`state-committed`、state 写入和清理各故障点模拟失败/进程重启；按 journal 恢复全部 Skill 和旧 state，staging 被清理；保留 marker 完整的 last-good generation，可显式 rollback | 故障注入、启动恢复、回退测试 |
| R8 | UI check/apply/rollback 独立于 USB 插入、USB 签名和 Codex payload 状态；入口在 `WinHome.tsx` 和 `OfflineHome.tsx` 可见，MacHome 不显示 | 无 USB 命令测试、UI 测试 |
| R9 | 不写 U 盘、基础包、MCP/Agent/runtime/凭据；代码阶段新增差异只在允许范围 | 代码阶段开始前保存基线 `git status --porcelain=v1` 和允许文件清单/哈希；结束后计算两份清单差集，再做路径审计；不得用裸 `git diff --name-only` |
| R10 | relay 使用独立 `skill-hub/current -> releases/<commit>`、固定 `latest.json/archive.zip`、独立配置/锁/location；在下载、校验、metadata 写入和 symlink 切换各故障点原子发布；失败保留旧 current；现有 Codex relay URL/文件/指针前后哈希不变 | relay 固定夹具、配置检查和 before/after manifest；外部部署交付时另附 systemd 单元 active、配置/权限、Compose config、容器内 `nginx -t`、固定路由/非目标 404、HTTPS 访问及现有 payload 路径前后哈希不变 receipt；本地任务不执行该部署 |
| R11 | 基础包清单哈希核对、ledger/journal schema、热/基础/卸载/第二 Manager 互斥均通过 | Rust/集成测试 |
| R12 | `npm run check`、前端测试、`cargo fmt --check`、`cargo check` 和相关 Rust 测试通过 | 实际命令、退出码 |
| R13 | 设计审计和代码审计均绑定固定版本，结论只能为“符合/不符合/证据不足” | 本文审计记录 |

## 9. 阶段门禁

### 阶段 A：设计

- [x] 读取当前能力包、Codex payload 更新和 relay 实现。
- [x] 记录用户要求、非目标、基线、联调门禁和唯一写入边界。
- [x] 冻结最小公共契约和验收矩阵。
- [ ] 独立只读设计复审通过。

设计复审通过后，第 1–9 节及其范围 SHA-256 不得再修改；实现阶段只能更新第 10 节的阶段状态、版本 receipt 和证据，不得改变契约。契约变更必须回到阶段 A，重新计算范围 SHA 并重新审计。

### 阶段 B：代码

只有阶段 A 复审通过，且联调对象已确认终止并重新核对工作区后才能开始。

- [ ] 重新读取工作区状态，记录联调对象停止证据。
- [ ] 保存基线快照：`git status --porcelain=v1` 的完整行（含状态列）以及每个路径的 SHA-256；保存允许文件集合的规范化排序副本。
- [ ] 唯一执行者按本文件实现。
- [ ] 代码版本冻结，停止写入。
- [ ] 独立只读代码审计通过。

### 阶段 C：测试与交付

- [ ] 代码审计通过后执行集成复验。
- [ ] 记录真实命令、退出码、版本哈希和未验证项。
- [ ] 不提交、不暂存、不推送、不部署。

范围审计算法固定为：`currentSnapshot - baselineSnapshot` 只比较新增状态行、未跟踪路径和路径前后 SHA-256；允许差异集合只能是第 7 节文件及其明确列出的对应测试/样式文件，既有基线行不计为本次改动。任何允许集合外的新增、状态变化或内容哈希变化均为范围违规；不能用未区分基线的裸 `git diff --name-only` 代替。

设计范围 SHA-256 的算法也固定：读取本文原始 UTF-8 字节（不做换行归一化），从文件开头或某个 LF 字节之后定位第一行，其起始 8 字节为十六进制 `23 23 20 31 30 2e 20`（即行首的二级标题 `10.`）；截取该行起始前缀并计算 SHA-256，审计记录本身不计入范围。该八字节行首哨兵不会在本算法说明中重复出现。审计 Agent 必须按该字节算法复算，不得使用 `Get-Content`/文本模式造成 CRLF/LF 变化后的哈希。

## 10. 设计审计记录

审计负责人：独立只读审计 Agent `01a08fc7-fd0e-7270-8fb7-3d46d877a8b9`（task_id `skill-hot-update-doc-audit-018`）

审计版本范围：本文件第 1–9 节（不含本审计记录），范围 SHA-256：`efe40a42f6dd42f83dddc60b17c896081ea89e2f3a704a74ee823ffed8e9dccd`

审计结论：本版本“符合”。阶段 A 审计只审设计契约的可执行性，不要求代码/relay 实现已经存在；实现证据留给阶段 B/C 的代码审计。阻断项：无。

审计证据：

| 要求 | 检查 | 证据 | 结论 |
| --- | --- | --- | --- |
| C1 用户目标/非目标 | 对照第 1、2 节及用户原文 | add-only、无 USB 依赖、人工确认、回退入口和非目标完整 | 符合 |
| C2 基础包和 U 盘隔离 | 对照第 2、4、7 节及现有实现 | 普通 Manager 随包只读基础 ID 资源；hot 模块禁止 USB/payload 依赖；基础安装锁列为阶段 B 硬门禁 | 符合 |
| C3 固定源、relay 回退、无签名取舍 | 对照第 4、5、6 节 | commit archive、relay→GitHub、二次 metadata/commit/hash 校验和无签名边界完整 | 符合 |
| C4 冲突、所有权、事务恢复 | 对照第 4.2–4.4 节 | ledger/journal/generation、文件身份、reparse 防护、原子交换、故障恢复和 rollback 删除前核对完整 | 符合 |
| C5 UI/API 与验收可执行 | 对照第 4.5、8 节 | WinHome/OfflineHome 入口、MacHome 排除、完整 wire、错误码、状态机及 R1–R13 完整 | 符合 |
| C6 范围和过度开发 | 对照第 7 节及工作区基线 | 第 7 节允许文件、唯一写入者、基线差集和阶段门禁明确；不引入签名体系或运行时改造 | 符合 |

### 阶段 B/C 实现审计 receipt（2026-09-11）

独立只读代码审计 Agent `01a09057-a46f-78c1-a698-8925cbca38c2`（task_id `skill-hot-update-code-audit-001`）对代码阶段版本给出“不通过”。该审计期间实现仍在迭代，随后已修复 Rust 编译阻断、基础资源源码回退、relay 配置字段、timer、UI 原始错误展示、跳过数量展示和部分事务恢复边界；以下阻断仍未宣称闭合：Windows 目标目录尚未使用完整的句柄相对/禁止跟随 reparse point 写入层；relay 服务端尚未完成真实外部部署验收；故障注入、启动恢复、并发互斥和完整 ZIP 安全夹具不足；build-time 基础资源生成/校验尚未纳入现有 build.rs。审计结论保持“不通过”，不得据此发布生产包。

本轮可复验证据：`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo test --manifest-path src-tauri/Cargo.toml skill_repo_update -- --nocapture` 通过（3/3）；`npm run check` 通过；`npm test -- --reporter=dot` 通过（36 files / 397 tests）；Skill Hub 定向 UI/relay 测试通过（26/26 与 1/1）。Cargo 仍报告既有 theme-engine 增量目录访问被拒绝的 warning；本机没有 Python，relay 测试未执行 Python 解释器分支，仅执行静态契约断言。未提交、未暂存、未推送、未部署。

### 阶段 B/C 后续实现 receipt（2026-09-11）

按 `multi-agent-dev` 的 `MAD_ROUTE_V1` 执行了三个互不重叠写入 Agent：Rust 事务模块、Skill Hub relay、前端/API；前端和 relay Agent 已返回回执并停止写入，Rust Agent 长时间无回执后由主控安全停止。主控随后仅在 Rust 模块内补充 relay 重定向拒绝和 check staging 失败清理，并重新格式化、编译和测试。

新增/修正证据：UI/API 具备结构化错误收敛、完整 expected identity、partial/conflict/skipped/rollback 展示；relay 固定仓库/分支/commit archive、GitHub 三域重定向白名单、精确 `latest.json/archive.zip` 路由、原子 current 发布和独立锁；Rust 已通过 Rust 2021 编译、journal/generation/previous-state 校验及 7 个模块单测。`src-tauri/resources/skill_repo_base_manifest.json` 静态校验通过（50 个 Skill ID，源清单 SHA 匹配）。

本轮实际证据：`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 通过；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo test --manifest-path src-tauri/Cargo.toml skill_repo_update -- --nocapture` 通过（7/7）；`npm run check` 通过；定向 UI/relay 测试通过（32 passed、1 skipped）；全量 Vitest 通过（36 files、403 passed、1 skipped）；`node --check tools/skill-hub-relay-sync.test.mjs` 通过；`git diff --check` 通过。跳过项仍是本机无 Python；Cargo 的 theme-engine 增量目录 access denied 仍为既有 warning。真实 Linux/systemd/nginx/公网 relay 联调已另行完成，见下方现场 receipt。

最终状态：代码与测试层面的明显阻断已收口，但尚未达到冻结契约的生产验收。当前仍需回到契约审查/补充授权后处理：Windows 句柄相对且全链路禁止 reparse 的写入层尚未完全实现；显式 rollback 尚无独立可恢复 journal；Unicode NFC/Windows OrdinalIgnoreCase 及完整故障注入/并发集成夹具不足；基础资源尚未由现有 `build.rs` 自动生成并按安装包清单校验；工作区存在大量既存脏改动，无法证明本轮完整范围差集。两次新独立终审 Agent 在限定窗口内均未返回 receipt，因此不得将 C1–C6 或 R1–R13 标记为通过，也不得发布生产包。上述结论不影响已完成的 relay 现场联调，但不能把 relay 可达性等同于客户端生产包验收。

### relay 现场联调 receipt（2026-09-11）

按 `multi-agent-dev` 协作约束，先与任务 `01a084d1-f450-7fd2-bc44-0c754fd30207` 只读核对目标主机和既有 Codex relay，再由主控作为唯一外部写入者部署；未修改 `菲律宾-sub2api` 主机。实际目标为 `141.11.138.152:13226`，域名 `update.404token.xyz`。

- 新增独立 `/srv/codex-update-relay/export/skill-hub`、`/srv/codex-update-relay/work/skill-hub`、独立 state/lock、`skill-hub-relay-sync.service` 和 hourly `skill-hub-relay-sync.timer`。
- Nginx 仅追加精确 `skill-hub/latest.json`、`skill-hub/archive.zip` location 和非目标 404；没有覆盖原 Codex broad location。
- Skill Hub 当前发布 commit 为 `794cc94ac690da9c0d919db82322699def53d2f3`；服务最近结果为 `Result=success`、`ExecMainStatus=0`；清单和 archive 公网均返回 200，非法子路径返回 404。
- 原 Codex `export/current` 仍为 `releases/385165615`，既有 `latest.json` SHA-256 仍为 `d188269893eac4c469012bd62a337fb54d6220861f351cb84eb6ed12bf941567`；原 Codex 公网入口返回 200；证书续期 timer 和既有 Codex timer 均 active。
- 部署中修复了两类实际主机差异：配置校验返回值遗漏 URL 字段，以及 `work`/`export` 跨文件系统导致的发布 rename 失败；公开静态目录权限已显式设为可被 edge 容器遍历。

### 代码修复与重打包 receipt（2026-09-11）

- `skill_repo_update.rs`：补充 Windows 非 reparse 父链句柄保护、ASCII 归档路径约束、ZIP 碰撞/原生二进制夹具，以及独立 durable rollback journal 和启动恢复路径；基础包仍只读，未接入 USB/Codex payload 更新通道。
- `build.rs`：增加基础源清单 schema、canonical-LF SHA-256、ID 唯一性、`file-manifest`、`checksums.sha256` 和静态资源一致性构建门禁；未修改 `resources/skill_repo_base_manifest.json`。
- `eslint.config.js`：为 Node relay 合约测试声明 Node globals，正式构建 lint 阻断已修复。
- 证据：`npm run check`、`npm run lint`、前端 335 项测试、Rust 383 项测试（2 项真实环境测试 ignored）、签名器 CLI 23 项测试、GUI 3 项测试、signer clippy `-D warnings` 均通过；one-click `verify-usb.ps1 -InternalSigned -SkipNetwork` 通过，GUI 包 5 文件/0 checksum mismatch。
- 新生成产物：`release/setup`、`release/usb-oneclick-final-20260911-r3`、`release/usb-license-signer-gui-final-r3-20260911`。主包使用 `SignedRequired` 编译并使用当前 `prod-2026-01` 公钥绑定；没有把私钥放入产物。
- 交付边界：当前主机无真实可移动 NTFS U 盘，未执行真实 `sign → verify`；主程序/签名器 Authenticode 状态为 `NotSigned`，因此上述是内部 USB 签名模型的可验证构建包，不是已完成组织 Authenticode 认证的公开生产包。严格句柄相对 Windows API、故障注入/跨进程集成证据和新一轮独立终审仍需单独完成，不能用本次构建成功替代这些证据。
