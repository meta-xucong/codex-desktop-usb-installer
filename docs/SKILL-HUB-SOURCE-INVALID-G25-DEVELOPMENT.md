# G25：Skill Hub 来源无效与更新失败修复开发文档

## 1. 任务目标

与 G24 Codex payload 更新确认阶段修复一起收口 Skill Hub 热更新链路：

1. 修复发行包中来源目录缺失、路径不一致或旧资源残留导致的 `SKILL_REPO_SOURCE_INVALID`；
2. 区分本地来源配置错误、远端 metadata 无效、来源不可达和确认身份过期；
3. 让前端在失败时显示可操作的原因与来源尝试结果，不再只显示“来源无效”；
4. 维持中国大陆网络场景的既定顺序：自有菲律宾 Relay 优先，GitHub 回退；不引入新的派生仓库、账号或用户可编辑更新地址；
5. 在打包和 U 盘验收阶段强制检查来源资源，避免“源码正确、EXE 资源不完整”的错配；
6. 不削弱 G24 的 Codex payload 完整性、快照和提交前独立复核。

## 2. 现场证据与基线

测试截图显示：

- 面板为“Skill Hub 热更新”；
- 错误码为 `SKILL_REPO_SOURCE_INVALID`；
- 前端提示“更新包无效或过期”，详情提示后端未返回公开的进一步详情。

当前源码对照发现：

- `src-tauri/src/app/skill_repo_update.rs` 只从 `app.path().resource_dir()/skill_hub_sources.json` 读取来源目录；便携 U 盘布局的可执行文件旁边也有 `resources/skill_hub_sources.json`，但运行时没有统一的候选路径策略；
- 资源缺失、读取失败被压缩成没有 detail 的 `SKILL_REPO_SOURCE_INVALID`；
- 同一个错误码也用于远端 metadata 身份无效和 apply 的 expected identity 无效，前端无法区分本地资源问题与网络/远端问题；
- `tauri.conf.json` 和 `build.rs` 有资源声明/构建门禁，但 `verify-oneclick-usb.ps1` 没有明确把 Skill Hub 来源目录作为 U 盘发行包必检项；
- 当前 Relay 现场复测可返回 200，metadata 身份为固定自有仓库、`archive.zip`、commit `794cc94ac690da9c0d919db82322699def53d2f3`，因此截图不能直接归因于 GitHub 不可达。

本任务工作区已有用户改动，所有实现必须以现状为基线，不回滚或覆盖无关改动。

## 3. 固定协议与安全边界

### 3.1 来源身份

默认发行资源只允许：

```text
仓库：meta-xucong/skill_hub
分支：master
Relay：https://update.404token.xyz/__codex_update__/skill-hub/
GitHub：客户端固定构造 api.github.com 与 github.com/codeload.github.com 地址
```

Relay 的 `latest.json` 和 `archive.zip` 必须同源、无重定向；metadata 只能声明固定 `archive.zip`，不得提供任意下载 URL。GitHub 仅作为回退，不改变 check/apply 已确认的 `sourceId`。

### 3.2 本地来源资源

- 优先使用运行中的便携 U 盘布局 `可执行文件同级/resources/skill_hub_sources.json`；
- 兼容 Tauri `resource_dir()` 和开发/安装布局；
- 只接受普通文件，拒绝符号链接/重解析点；
- 缺失时使用编译期同一份 canonical JSON 作为安全兜底，并记录“外部资源缺失、使用内置来源目录”；
- 外部文件存在但格式/身份错误时不得静默回退，必须报出明确原因，避免把被篡改或错配的发行包伪装成正常包；
- 外部来源仍经过现有严格 schema、HTTPS、固定 Relay/GitHub 身份校验。

### 3.3 错误分类

不改变既有稳定错误码兼容性，但 detail 必须可诊断：

| 场景 | 稳定码 | detail 要求 |
| --- | --- | --- |
| 来源资源全部缺失，已使用内置目录 | 成功/日志 warning | 记录候选路径与 fallback，不阻断正常检查 |
| 外部来源 JSON 缺失/不可读/非法 | `SKILL_REPO_SOURCE_INVALID` | 说明资源阶段、候选路径类别、校验失败原因 |
| Relay/GitHub 网络不可达 | `SKILL_REPO_SOURCE_UNAVAILABLE` / `SKILL_REPO_TIMEOUT` / `SKILL_REPO_ALL_SOURCES_FAILED` | 返回脱敏的固定来源标签和 attempts |
| 远端 metadata 身份或归档不合法 | `SKILL_REPO_SOURCE_INVALID` / `SKILL_REPO_ARCHIVE_INVALID` | 明确是 metadata/归档校验，不写入目标目录 |
| check 后 apply 的身份变化 | `SKILL_REPO_STALE` | 要求重新检查，不允许静默换源 |

detail 不得包含 API key、凭据、令牌或完整用户隐私路径；来源尝试只暴露固定 sourceId、阶段、结果、HTTP 状态和耗时。

## 4. 实现方案

### 4.1 Rust 来源加载

在 `skill_repo_update.rs`：

1. 引入编译期 `include_bytes!("../../resources/skill_hub_sources.json")` canonical fallback；
2. 增加确定性候选路径搜索：便携 EXE 同级 `resources`、Tauri `resource_dir()`、EXE 同级资源文件；去重并检查普通文件/非 reparse；
3. 记录来源加载阶段日志（候选、命中、fallback、解析结果），不记录敏感信息；
4. 仅在所有外部候选都不存在时使用 fallback；一旦发现存在但读取或解析失败，返回有 detail 的 `SKILL_REPO_SOURCE_INVALID`；
5. 对 `expected` 做完整 sourceId/sourceKind/仓库/分支/commit/size/SHA 校验，使旧前端或跨版本 expected 不再落入模糊错误；
6. 为本地资源、fallback、远端 metadata、过期 expected 分别补单测。

### 4.2 发行包与 U 盘门禁

在构建/验收脚本中：

- `build-installer.ps1`、`build-oneclick-usb-layout.ps1` 在产物复制后检查 `resources/skill_hub_sources.json` 存在且通过同一固定身份规则；
- `verify-oneclick-usb.ps1` 检查来源资源、基础 Skill manifest 和资源文件都在包内，且来源资源未被错误旧版本替换；
- 将两个 Skill Hub 静态资源纳入卸载/清理白名单，避免卸载把它们当未知残留；
- 生成的 `SHA256SUMS.txt` 必须覆盖来源资源，签名后不得再改动来源配置；
- 打包验收增加“EXE 同级 resources 与 canonical JSON 一致”的检查。

### 4.3 前端诊断

- `SKILL_REPO_SOURCE_INVALID` 的详情可显示“本地来源资源/远端 metadata/确认身份”阶段；
- 失败时保留 attempts；若后端返回空 attempts，显示明确的本地来源加载提示和“重新校验/打开日志”；
- 不把仓库账号或内部来源实现写入普通用户文案；诊断复制内容只保留固定 sourceId 和状态字段。

### 4.4 与 G24 合并边界

- 保留 G24 的 Codex payload metadata-first、SHA256SUMS digest 复用、快照一致性和提交前独立 payload hash；
- 不把 Skill Hub 的无签名 overlay 与 Codex/U 盘签名绑定；Skill Hub 继续独立安装；
- 不修改 Codex payload 更新的签名、确认和事务协议；
- 只共享日志/构建验收的清晰阶段命名，不复用跨通道状态或 hash 缓存。

## 5. 测试与审计要求

### 5.1 自动化测试

- Rust：source directory 正常解析、缺失 fallback、外部非法 JSON、错误 detail、候选路径优先级、expected 身份完整校验、Relay/GitHub attempts；
- 前端：`SKILL_REPO_SOURCE_INVALID` 的本地资源/远端 metadata 文案与详情、空 attempts 诊断；
- PowerShell：来源资源缺失、身份错误、SHA256SUMS 漏列、正常包四类 fixture；
- 既有 G24 Codex 定向测试、全量 Rust/TypeScript 测试和 lint/check 全部回归。

### 5.2 发布前人工/产物审计

必须逐项核对：

1. `release/setup/resources/skill_hub_sources.json` 存在；
2. U 盘包 `resources/skill_hub_sources.json` 存在并在 `SHA256SUMS.txt` 中；
3. EXE/资源/manifest 来源版本一致；
4. Relay 200 时能完成 check/apply；Relay 不可达时 GitHub 回退或给出完整 attempts；
5. 无网双击仍有提示，不出现无反应；
6. G24 Codex 更新仍能在本地文件变化时拒绝提交；
7. 卸载不误删 U 盘内容，也不留下来源资源造成的“未知残留”误报。

## 6. 变更允许范围

允许修改：

- `docs/SKILL-HUB-SOURCE-INVALID-G25-DEVELOPMENT.md`；
- `src-tauri/src/app/skill_repo_update.rs`；
- `src-tauri/resources/skill_hub_sources.json`；
- `src-tauri/build.rs`、`src-tauri/tauri.conf.json`（仅资源/门禁）；
- `tools/build-installer.ps1`、`tools/build-oneclick-usb-layout.ps1`、`tools/verify-oneclick-usb.ps1`；
- `src-tauri/src/app/usb_media_cleanup.rs`；
- 对应 Rust/TS/脚本测试、错误文案和 `.learnings/` 经验记录。

不得借本任务重写 Skill Hub 事务协议、Codex payload 更新协议、USB 签名体系或引入第三方派生仓库。

## 7. 验收结论标准

只有在代码审计、自动化测试和产物资源审计均通过后，才能称为“源码已修复”。未在真实中国大陆网络、真实签名 U 盘和打包 EXE 上验证的项目必须明确列为待实测，不能宣称线上问题已完全消失。

## 8. 记录

| 日期 | 记录 |
| --- | --- |
| 2026-09-15 | 根据现场截图建立 G25 契约。当前 Relay metadata 可返回 200；截图的 `SKILL_REPO_SOURCE_INVALID` 需要同时修复本地资源路径/打包门禁和远端错误可观测性，不能简单归因于 GitHub 网络。 |
