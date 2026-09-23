# Codex VPS Relay 同步恢复开发文档

日期：2026-09-16  
状态：契约冻结，进入实现与远端验收

## 1. 目标与非目标

目标是修复 `update.404token.xyz` 的 Codex payload relay“新 Release 已发布但中转长期不更新”问题，并在菲律宾跳板机上完成可复核的公网验收。修复只覆盖 `Wangnov/codex-app-mirror` 的 x64 Release relay。

非目标：不修改 Skill Hub 源或同步器，不修改 80 端口心理测试站、Docker 业务容器或菲律宾-sub2api，不改变客户端 USB/Codex 更新协议，不把 relay 变成新的签名根，不删除当前好版本来换取空间。

## 2. 冻结的根因

1. 现 timer 使用 `OnBootSec`/`OnUnitActiveSec`，`Persistent=true` 没有错过 `OnCalendar` 任务的补偿语义；服务失败后没有独立恢复触发器或告警。
2. 同步器失败只写 journald，不保存 last-attempt、last-success、last-error、下一次重试等持久状态；API、资产未齐、下载、校验、权限和空间错误无法自动收敛。
3. 发布先把 staging 改名为固定 `releases/<releaseId>`，再切换 `current`；切换失败留下已存在目录，后续永远拒绝同一 release。
4. `current_release_id()` 只比较 `releaseId`，无法识别同一 Release 的资产补齐、资产修订或 manifest/checksum 变化。
5. 当前空间预检把“保留 current + 下载新版本 + 安全余量”作为硬门槛是正确的；本次远端约 1.511 GB 可用、旧发布约 1.566 GB、新下载约 775 MB，故必须扩容/增加挂载空间或由运维提供清理后的可用空间，不能删除 current 绕过门禁。

## 3. 实现契约

### 3.1 触发、重试和观测

- 主 timer 使用 `OnCalendar`、`Persistent=true`、明确 `AccuracySec` 和随机延迟，覆盖关机错过后的补偿；服务启动也能立即恢复未完成 journal。
- 同步器对 GitHub API、资产未齐、HTTP 下载、校验前临时失败和发布切换失败执行有界指数退避；每次尝试必须更新 `state/sync-status.json`。不可重试的配置、身份、磁盘不足或权限错误也必须记录并退出非零。
- `sync-status.json` 至少包含 schemaVersion、lastAttemptAt、lastSuccessAt、lastError、attemptCount、nextRetryAt、currentIdentity、desiredIdentity、journalPhase；不保存 token 或响应正文。
- service 使用 `OnFailure` 指向独立失败观察 unit，写入 journald 可检索告警并保留状态文件；失败观察本身不得触碰发布内容。
- 任何失败都保留 `export/current` 和已验证版本；重试成功后状态自动收敛到 success。

### 3.2 完整身份

同步与短路判断必须绑定以下规范化身份：Release `id`、tag、publishedAt；目标 asset 的 GitHub asset `id`、name、size、digest；本地下载 SHA-256；`release-manifest.json` 的 asset id/size/SHA-256 与本地 digest；`SHA256SUMS-windows.txt` 的 asset id/size/SHA-256 与本地 digest。缺任一身份字段时 fail-closed。当前 descriptor 缺少新字段时视为未知并重新同步，不能把旧 descriptor 当作已验证。

同一 Release ID 但上述任一字段变化必须重新下载/验证并可替换 current；来自不同 Release、不同 asset 或不同 manifest/checksum 的文件禁止混包。

### 3.3 幂等发布 journal

发布事务使用 `/srv/codex-update-relay/state/publish-journal.json`，记录 transaction id、release id、stage、incoming、final、旧 current 指针、旧 final（如有）、完整目标 identity 和阶段。阶段至少为 `staged`、`incoming-ready`、`current-switched`、`finalized`。

- staging 完整校验后才持久化 journal。
- 已存在的 final 若 identity 完全相同则复用；若是同 Release 修订，则写入新的 incoming 目录，不覆盖旧目录，原子切换 current 后再清理旧目录。
- 切 current 失败只保留 journal 和旧 current；下一轮先恢复/重试，不因残留目录永久阻塞。
- 启动和每次 sync 前恢复 journal；恢复不确定时 fail-closed，不删除 current。
- 清理只允许作用于明确的非 current、已验证的发布目录；清理失败必须告警但不能回滚 current。

## 4. 远端边界与回滚

部署目标固定为 `141.11.138.152:13226` 的 `update.404token.xyz` relay。部署前备份 `/etc/codex-update-relay`、systemd unit、`/srv/codex-update-relay/export/current`、edge 配置和相关哈希；不触碰 Skill Hub、80 端口站点、心理测试容器和 `141.11.138.220` sub2api。

回滚顺序：停止新 timer/service → 保留现场状态和 journal → 恢复脚本、配置、unit 和 edge 变更前副本 → `daemon-reload`/配置验证 → 恢复旧 current 指针 → 重新做 Codex/Skill Hub/80 端口公网回归。不得用删除 current 或删除唯一旧版本来“回滚”。

## 5. 验收矩阵

- A1：`OnCalendar` + `Persistent` 生效，错过触发可补偿；OnFailure/状态文件可检索。
- A2：API、资产不全、下载、错误 SHA/size、权限、空间不足和 symlink 切换故障注入均有重试/退避/恢复证据。
- A3：发布中断后重启可继续或安全回滚，重复运行幂等，不被已有 final 目录永久阻塞。
- A4：同 Release 资产 id/size/digest 或 manifest/checksum digest 变化会重新同步；混包拒绝。
- A5：真实 GitHub 最新 Release 的 metadata、三项资产、size、SHA-256 与公网 relay 完全一致。
- A6：公网 `latest.json`、MSIX 下载与 Range/SHA-256 正常；非目标路径 404/方法限制、TLS/HSTS/nosniff/no-store 正常；旧 Codex relay 与 Skill Hub 路由保持正常；80 端口和业务容器不回归。
- A7：变更范围只包含本同步器、其 unit/config/deploy/test/doc 和必要的远端独立 relay 文件；不提交、不暂存、不推送。

## 6. 交付记录要求

最终 receipt 必须给出：完成项、未完成项、测试命令/退出码、远端 service/timer/journal 状态、current 指针、Release/asset id/size/digest、本地与公网 SHA-256、TLS/权限/非目标路由回归、空间状态及是否需要扩容。不能把未执行的公网或 VPS 状态写成通过。预计不需要重新打包 U 盘，因为本次只修复 VPS relay 同步链路。

## 7. 本次实施记录（2026-09-16）

已在统一工作区实现并通过本地门槛：

- 同步器：有限指数退避、state/sync-status.json 持久状态、完整 Release/asset/manifest/checksum 身份、重复资产拒绝、同 Release 修订重同步、发布 journal 和启动恢复、current 受控 symlink。
- systemd：OnCalendar + Persistent + AccuracySec + 随机延迟；失败观察 unit；配置增加重试参数。
- 部署脚本：部署前备份 Codex 配置、systemd unit 和 current 指针/哈希；实施后不再仅以 timer active 作为成功条件，公网 metadata、资产身份和 Range 下载必须验证。
- 测试：tools/test_codex_update_relay_sync.py 的 8 个固定夹具覆盖重试、身份修订、manifest 绑定、journal 路径越界、journal 恢复、同 Release 发布和并发锁；发布测试用受控 switch fixture 验证事务，生产 symlink 仍需在 VPS/Linux 复跑。

远端当前已知硬门槛：根分区可用约 1.51 GB，current 为 385165615，另有旧非 current 发布 383395485。按冻结契约，新 MSIX 下载、current 保留和 512 MiB 安全余量需要更多空间；不得删除 current 或降低门禁。公网最终目标 Release/asset/SHA、远端 service/journal、TLS/路由回归必须在容量条件满足后记录于下方 receipt，未验证不得标为通过。

## 8. 实施与公网验收 receipt

8.1 是容量恢复前的历史阻塞记录；最终状态以 8.2 为准。

待完成远端部署后填写：备份路径与变更前/后哈希、命令和退出码、timer/service/journal 状态、current 指针、GitHub 与公网 latest 的 Release/asset id/size/digest、本地与公网 SHA-256、Range/TLS/HSTS/nosniff/no-store、非目标路径、Skill Hub、80 端口和容量结果。因本次不改变 U 盘本体，预期无需重新打包 U 盘。

### 8.1 容量恢复前的阻塞记录

- 上游实时核验：Release `388674726`，tag `codex-app-26.908.70816`；x64 MSIX asset `564527780` / `774919598` / `sha256:163f5c14ce376c98711396aacad34099df275ab67d5b0e76ee2b76ba5c13d7b9`；manifest asset `565697122` / `19922` / `sha256:57fedd87ac0bc207c28dd322520e4b11a22822230dbc58386c0b61dcd6d89ad4`；checksums asset `564718820` / `236` / `sha256:121f4e9cc44a1f490bfcc96166cbf0dd75119bb4f8f0022ed8446c05eca646b1`。
- 公网当前仍是旧版本：`latest.json=200`，Release `385165615` / `codex-app-26.903.61454`；旧 x64 MSIX `769792128` / `f8a845dd58f177fbcd71b01c7831d742fc90855e4b207ee2d8ce3ab5f9bcc30f`。响应为 `Cache-Control: no-store`、`Last-Modified: Wed, 09 Sep 2026 08:12:52 GMT`、HSTS/nosniff；发布目录和根路径 404，POST 403，Skill Hub latest/archive 为 200。
- 远端只读状态：current=`releases/385165615`；timer 仍为 active/waiting、Persistent=yes，旧 service 为 failed/exit 2；可用空间 `1509416960` bytes；两个 Codex 目录分别为 `383395485=796617381` bytes、`385165615=769817165` bytes。Skill Hub current=`794cc94ac690da9c0d919db82322699def53d2f3`，未改动；80 端口只读探测为 302。
- 远端变更：未执行。原因是新发布所需空间门槛不能满足，且本任务不能由智能体代为删除远端目录或扩容；因此没有生成“变更后哈希”、没有切换 current，也没有把公网目标 Release 标为通过。待运维人工扩容或清理后，应执行 `tools/install-codex-update-relay.ps1 -RunSync -VerifyPublic` 并补录完整 A1-A6 证据。

### 8.2 容量恢复后的最终验收（2026-09-16 17:14 CST）

- 部署脚本退出码 0；备份目录为 `/srv/codex-update-relay/state/backups/20260916T091136Z`。远端安装文件 SHA-256 与工作区版本一致：sync `eff8dfa9d280ecb4496b937b74eaeb518880aec48668917e1923de9a2c76bb60`，config `6928edf4989e9cc9623fa63a0e43d56c4226f4b86181a2565082dff4203432eb`，service `b3e2d4a111440310e9b75716f3ce83b623f28812ea3bafc5c51284c8d95774d0`，timer `5ea398dd3ef73283fcb6144671944f2bf9109d655457bf37ff43b1fcfc6f7feb`，failure unit `75607155b111357b4ce6e72616cb0c2be726485d871cd746c0ebb9e46b8d06f5`。
- timer：`active/waiting`、`Persistent=yes`、`OnCalendar=*-*-* 00/6:00:00`、`AccuracySec=1min`，下一次触发为 `2026-09-16 18:01:20 CST`。service 最终 `inactive/dead`、`Result=success`、退出码 0；journal 为 `none`。部署期间首次并发启动得到 `another sync is already running` 并触发 OnFailure，随后重试成功，证明锁与失败观察生效。
- current=`releases/388674726`。目标 x64 MSIX `OpenAI.Codex_26.908.9136.0_x64__2p2nqsd0c76g0.Msix`：asset id `564527780`、size `774919598`、GitHub/local/public SHA-256 `163f5c14ce376c98711396aacad34099df275ab67d5b0e76ee2b76ba5c13d7b9`。manifest：asset id `565697122`、size `19922`、SHA-256 `57fedd87ac0bc207c28dd322520e4b11a22822230dbc58386c0b61dcd6d89ad4`。checksums：asset id `564718820`、size `236`、SHA-256 `121f4e9cc44a1f490bfcc96166cbf0dd75119bb4f8f0022ed8446c05eca646b1`。
- 公网 `latest.json` 为 200，release `388674726` / `codex-app-26.908.70816`；远端文件 SHA 与公网流式下载 SHA 三项完全一致。MSIX 公网 Range 返回 `206 bytes 0-0/774919598`；headers 为 `Cache-Control: no-store`、`Last-Modified: Wed, 16 Sep 2026 09:11:55 GMT`、HSTS `max-age=31536000`、`nosniff`。根路径、发布目录、未知路径均 404，POST 403。
- 边界回归：Skill Hub current 仍为 `794cc94ac690da9c0d919db82322699def53d2f3`，latest SHA `dadc28db8c9e7f675fb7e920cff23c6027a8949ae6c53f85232ea1e0b3db4bf7`；Skill Hub latest/archive 均 200；80 端口探测仍为 302。未触碰 sub2api。
- 容量：清理后可用 `2043666432` bytes，current 和新版本均保留；未删除 current、未降低 `min_free_bytes`。本次只修复 VPS relay，不需要重新打包 U 盘。

### 8.3 旧版本自动清理补充

- `max_published_releases` 已从 2 调整为 1：每次新版本完成校验并切换 current 后，只保留 current，其余数字命名的 Codex 发布目录自动清理。
- “已经是最新版本”不再直接绕过清理；正常非 dry-run 执行也会清理残留旧版本。dry-run 不执行删除。
- 2026-09-16 17:37 CST 已在 VPS 实际验证：`export/releases` 仅剩 `388674726`，current 指向该目录，service `Result=success`，journalPhase=`none`，Skill Hub current 未变化。
