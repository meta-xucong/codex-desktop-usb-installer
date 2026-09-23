# G34：Skill Hub Windows 本地持久化失败修复

## 1. 目标

修复 Windows 用户点击 Skill Hub「检查更新」或「安装」时，远端来源已经成功返回，但本地 staging/ledger 初始化因目录刷新失败而中断的问题。用户应看到可操作的本地磁盘错误，而不是“更新包无效或已过期”。

本任务不改变 Codex 本体更新、U 盘授权/签名、Skill Hub 来源顺序、VPS/镜像协议或远端仓库内容。

## 2. 现场证据与根因

- 测试日志已记录 `ph-relay` metadata、archive、verify 均为 HTTP 200，且已选出合法 commit；日志没有进入 apply 阶段，因此不能把本次失败归因于 GitHub 或中转源。
- Windows 原生复现：对普通 NTFS 目录使用 `CreateFileW(..., GENERIC_READ, FILE_FLAG_BACKUP_SEMANTICS)` 后调用 `FlushFileBuffers`，返回 `false`、Win32 错误 5；同一目录改用 `GENERIC_WRITE` 后刷新成功。
- Rust 旧实现正是以 `GENERIC_READ` 打开目录，再调用 `FlushFileBuffers`。
- 旧实现把该本地 I/O 错误包装为 `SKILL_REPO_LEDGER_INVALID`；前端因此显示“更新包无效或已过期”。前端的 `[local path]` 是诊断脱敏后的路径占位符，不是远端请求地址。

## 3. 修复契约

1. Windows 目录刷新句柄必须具备 `GENERIC_WRITE`，同时保留目录备份语义、重解析点保护和全部共享模式。
2. 目录打开失败或 `FlushFileBuffers` 失败统一返回 `SKILL_REPO_LOCAL_IO`，保留 Win32 错误码供展开详情使用。
3. 前端把 `SKILL_REPO_LOCAL_IO` 映射为“无法写入磁盘，请检查磁盘权限与可用空间后重试”，并标记为可重试；不再伪装成远端包无效。
4. check staging 创建在本地刷新失败时做尽力清理，避免留下空的 UUID 临时目录；原始错误仍然返回给用户。
5. Windows 回归测试必须实际创建临时目录并走生产 `sync_directory` 路径，不能只测试字符串或 mock。

## 4. 变更范围

- `src-tauri/src/app/skill_repo_update.rs`
  - 可写目录句柄、Win32 错误详情、本地 I/O 错误码、staging 失败清理、Windows 回归测试。
- `src/shared/types.ts`
  - 增加稳定错误码 `SKILL_REPO_LOCAL_IO`。
- `src/app/errorCopy.ts`、`src/app/errorCopy.test.ts`
  - 本地写入错误的用户文案、可重试属性和映射回归测试。
- `tools/build-usb-license-signer-gui.ps1`
  - 修复 portable vault 正式分支的 PowerShell 结构，并保存为 Windows PowerShell 5.1 可解析的 UTF-8 BOM 文件；该脚本问题在本次重新构建时发现并一并修复。
- `.learnings/ERRORS.md`、`.learnings/LEARNINGS.md`
  - 记录根因和发布前防复发检查项。

## 5. 验收矩阵

| 编号 | 场景 | 预期 |
|---|---|---|
| G34-R1 | Windows 临时 NTFS 目录刷新 | 生产 `sync_directory` 返回成功 |
| G34-R2 | Skill Hub check staging 创建 | 不因正常目录 flush 返回错误 |
| G34-R3 | 本地权限/磁盘写入失败 | 返回 `SKILL_REPO_LOCAL_IO`，不返回 ledger/artifact 错误 |
| G34-R4 | 前端收到 `SKILL_REPO_LOCAL_IO` | 显示本地磁盘提示，详情可展开，可重试 |
| G34-R5 | 远端 metadata/archive/verify 成功后进入本地阶段 | 日志与错误阶段可区分，不误判为来源失败 |
| G34-R6 | 正式安装器与签名器构建 | 使用同一版本化 profile，包内配对预检通过 |

## 6. 验证记录

本次实现与发行验证结果：

- Rust `cargo fmt --check`：通过；Skill Hub 定向测试 `22 passed`；Rust 全量测试 `422 passed, 3 ignored`。
- TypeScript：通过；lint：通过；前端测试 `353 passed`；生产 renderer 构建：通过。
- Windows USB 安装器：从当前源码构建通过；包内 `verify-usb.ps1`：通过；U 盘包 SHA-256 `288` 项全部通过。
- 签名器构建脚本 Windows PowerShell 5.1 解析：通过；签名器构建通过；portable vault/profile/Bundle/manager 公钥摘要、版本和 releaseId 配对预检：通过。
- 物理 U 盘签名、目标机网络与安装：需用户现场验收，不由本地构建替代。

本次交付目录：

- `release/usb-oneclick-v0.5.24-g34-skill-hub-windows-flush-20260917-r2`
- `release/usb-license-signer-gui-v0.5.24-g34-skill-hub-windows-flush-20260917-portable`
- `release-profiles/usb-v0.5.24-g34-skill-hub-windows-flush-20260917-key216.json`

## 7. 发布前防复发清单

- 任何 `FlushFileBuffers` 目录调用都必须用可写句柄，并保留 Windows API 回归测试。
- 网络来源通过不代表本地 apply 通过；日志必须按 source/phase 分层判断。
- `SKILL_REPO_LEDGER_INVALID` 只表示真正的 ledger/期望清单语义错误；权限、磁盘、句柄等 OS I/O 必须使用本地 I/O 错误码。
- 每次构建必须从当前源代码生成 manager，再由同一版本化 signing profile 生成 U 盘包与签名器；不得把历史 manager、旧 profile 或当前机器 DPAPI 公钥混入新发布物。
- 发行 PowerShell 脚本必须用 Windows PowerShell 5.1 的 `-File` 实际解析/冒烟；仅在 PowerShell 7 中能解析不算通过，含中文的脚本必须保留 BOM 或改成无歧义编码。
