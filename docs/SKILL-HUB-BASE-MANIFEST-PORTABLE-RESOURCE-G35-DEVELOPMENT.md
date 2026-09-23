# G35：Skill Hub 基础 Skill 清单便携资源解析修复

## 1. 用户问题与目标

用户在 U 盘版管理器中点击 Skill Hub 安装/更新时，界面仍显示：

> 基础 Skill ID 资源缺失：路径不存在：[local path]
> `SKILL_REPO_LEDGER_INVALID`

目标是让完整发行包在便携启动、普通 Tauri 资源目录异常或资源文件被遗漏时，都能稳定找到并验证基础 Skill ID 清单；同时不能再把“发行资源路径缺失”误报成热更新账本损坏。基础清单只用于区分基础 Skill 与可热更新 Skill，不改变 Codex 本体更新、U 盘签名、远端来源或用户数据边界。

## 2. 证据与根因

- G34 发行包实物包含：`resources/skill_repo_base_manifest.json`、`resources/skill_hub_sources.json`、`content/codex-starter-kit/manifest/skill-audit.json`，且内容可通过源码侧构建校验。
- `skill_hub_sources.json` 已使用“运行中 EXE 同级 `resources` → 同级文件 → Tauri `resource_dir` → 编译内置”的解析顺序。
- `base_manifest()` 仍只执行 `app.path().resource_dir()/skill_repo_base_manifest.json`。便携 U 盘入口运行时，Tauri 的资源目录可能解析到本机安装/缓存目录，该目录没有该文件，于是正常的便携资源被跳过。
- `metadata_required()` 将不存在路径包装为 `SKILL_REPO_LEDGER_INVALID`，导致 UI 错误地显示“更新包无效或已过期”，掩盖了资源解析缺陷。

结论：本次是运行时资源根路径解析缺陷，属于 G34 修复遗漏；不是 U 盘损坏、Skill Hub 远端不可达、基础 Skill 内容缺失，也不是上一次 Windows `FlushFileBuffers` 本地写入问题。

## 3. 冻结契约

1. 基础资源解析顺序固定为：
   - 当前管理器 EXE 同级 `resources/skill_repo_base_manifest.json`；
   - 当前管理器 EXE 同级 `skill_repo_base_manifest.json`；
   - Tauri `resource_dir()/skill_repo_base_manifest.json`；
   - 编译时已经由 `build.rs` 校验过的内置副本。
2. 每个外部候选路径都必须经过现有重解析点、普通文件、字节读取和 schema/source/hash/ID 校验；候选文件存在但内容无效时 fail closed，不跳过篡改或损坏的文件。
3. 所有候选都不存在时使用内置副本，并写入候选缺失的日志；内置副本本身无效时才返回明确的基础资源错误。
4. 基础资源错误不能复用 `SKILL_REPO_LEDGER_INVALID`。新增稳定错误码 `SKILL_REPO_BASE_RESOURCE_INVALID`，前端显示“安装器基础资源不可用，请重新复制完整安装包”，详情保留阶段信息并允许重试。
5. U 盘版和普通 Tauri 版都必须使用同一解析函数；不得依赖开发机 checkout 路径或用户本机历史目录。
6. 打包阶段必须显式检查 `resources/skill_repo_base_manifest.json` 的 schema、source、sourceSha256、非空 skills，并校验其 ID 顺序与 `content/codex-starter-kit/manifest/skill-audit.json` 一致。

## 4. 允许改动范围

- `src-tauri/src/app/skill_repo_update.rs`：基础资源候选解析、内置副本、结构校验、日志和单元测试。
- `src/shared/types.ts`、`src/app/errorCopy.ts`、`src/app/errorCopy.test.ts`：新增稳定错误码及用户可见错误分类。
- `src-tauri/build.rs`：继续作为内置资源的编译期校验门禁，不改变资源 schema。
- `tools/build-installer.ps1`、`tools/build-oneclick-usb-layout.ps1`：增加发行包基础资源存在性/身份/ID 顺序门禁。
- `.learnings/ERRORS.md`、`.learnings/LEARNINGS.md`：记录根因和发布前防复发规则。
- 本文档及测试证据。

不在本次范围：远端仓库、VPS/镜像同步、Codex 更新协议、签名密钥/公钥、卸载流程、Skill 内容本身和 UI 布局。

## 5. 验收矩阵

| 编号 | 场景 | 预期 |
|---|---|---|
| G35-R1 | U 盘 EXE 同级 `resources` 存在 | 优先读取便携资源，不访问开发机/本机历史目录 |
| G35-R2 | Tauri `resource_dir` 缺失但 EXE 同级资源存在 | 基础清单解析成功 |
| G35-R3 | 所有外部候选不存在 | 使用编译内置副本并记录 fallback，不阻断 Skill Hub |
| G35-R4 | 外部候选存在但 schema/hash/ID 无效 | fail closed，返回 `SKILL_REPO_BASE_RESOURCE_INVALID`，不伪装 ledger 错误 |
| G35-R5 | check/apply/rollback 前置校验 | 均经过同一基础清单解析逻辑 |
| G35-R6 | 构建 U 盘/普通安装包 | 缺少或错配基础清单时构建失败；完整包通过 |
| G35-R7 | 既有 Skill Hub 回归 | Rust、前端、包审计和现有 G34 Windows 本地 I/O 测试继续通过 |

## 6. 实施与审计要求

- 先修改后固定版本，再由独立只读审计复核候选顺序、fail-closed 行为、错误码映射和打包门禁。
- 不能把本机现有 `.codex`、Cargo target 或历史 release 目录当作发行包证据。
- 发行验证必须同时检查源码、最终包内资源、管理器二进制和安装器脚本；旧的非 `-r2`/失败目录不得作为交付物。
- 若目标机仍出现错误，必须从日志中的 `skill_hub_base_manifest_loaded` / `skill_hub_base_manifest_missing` 记录和最终包路径继续定位，而不是再次笼统归类为网络超时。

## 7. 验证证据

- Rust 全量单元测试：425 通过、3 忽略（仅实时网络/完整发行环境测试）。
- 前端全量测试：421 通过、1 跳过；TypeScript 检查和 ESLint 均通过。
- 生产前端构建、Tauri USB Manager release 构建和 Windows SDK `mt.exe` 嵌入式 `requireAdministrator` 清单校验均通过。
- 新包自带 `verify-usb.ps1 -InternalSigned -SkipNetwork` 通过，包含基础清单门禁、来源清单门禁、MSIX 身份/哈希、资源和无残留检查。
- 新包与新签名器通过 `verify-usb-signing-pair.ps1`：`keyId=prod-2026-01`、公钥摘要 `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`、`releaseId=g35-skill-hub-base-resource-20260917` 完全一致。
- G35 包管理器二进制包含新的 `SKILL_REPO_BASE_RESOURCE_INVALID`、`skill_hub_base_manifest_loaded` 和用户文案，证明不是复用旧 Manager。

## 8. 状态

- 状态：已实现并通过源码、包内验证和签名 pair preflight；仍需用户在目标测试机上实际点击 Skill Hub 做联网验收。
- 问题编号：`ERR-20260917-G35`。
- 基线：当前工作树及 G34 `-r2` 包；工作树已有用户/历史改动，不能覆盖。
- 交付包：`release/usb-oneclick-v0.5.24-g35-skill-hub-base-resource-20260917`。
- 匹配签名器：`release/usb-license-signer-gui-v0.5.24-g35-skill-hub-base-resource-20260917-portable`。
- 未验证项：目标机真实安装、U 盘签名后点击 Skill Hub 的真实远端请求；这部分受目标机网络与 U 盘状态影响，需实测确认。
