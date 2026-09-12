# Codex 适配合并后整改开发文档

任务 ID：`CODEX-POST-MERGE-CORRECTIVE-001`
原始请求：`根据你检测出的问题，把剩下的问题落个开发文档，并落代码，解决`
契约修订：`v1.0-post-merge-corrective`
状态：已完成；运行时闭环由 [`FULL-BUNDLE-ONE-CLICK-DEVELOPMENT.md`](FULL-BUNDLE-ONE-CLICK-DEVELOPMENT.md) 继续落地；固定版本只读复核通过（当前环境无独立子 Agent 派发接口）

## 1. 目标与非目标

### 目标

1. 修复合并后生成的 `installer-pack/` 与当前源码树之间的 provenance 漂移。
2. 让 `--SkipRebuild` 验收也能发现生成包来自旧源码，而不是只检查包内自洽的文件哈希。
3. 从公开仓库移除疑似专有的 PDF 指南正文，保留一个不含上游正文的隔离占位，维持 50 Skill 的状态和目录契约。
4. 将运行时、飞书真实租户和外部付费 API 的未验证条件保留为明确阻断，不把静态 dry-run 描述成真实可用。
5. 将 Windows x64 / CPython 3.12、Node 20、Feishu npm closure 和 Python wheelhouse 纳入可复现的一键安装包。

### 非目标

- 不在公开仓库内放入 App Secret、OAuth 会话、API Key 或租户数据；公开运行时介质必须有逐文件哈希和来源记录。
- 不调用真实飞书租户、DashScope、Image-2、Seedance 或行情付费接口。
- 不改变 50 个 Skill、7 个 Agent、25 个默认 Skill、7 个 guided-config 和 4 个 unsupported 的业务分类。
- 不修改用户机器上的全局 Codex 配置，不删除用户已有 Skill/Agent，不自动提交或推送。

## 2. 基线、问题与证据

基线提交：`08f50d716f22a38aa0086667ee0226da0558207b`。基线工作区无未提交改动。

| 编号 | 问题 | 基线证据 | 处理结论 |
| --- | --- | --- | --- |
| `P0-PROVENANCE` | 生成包 `installer-pack/pack.json` 声明 `sourceCommitRole=git-head`，但 `sourceCommit` 是旧提交 `671f2f4...`，当前合并提交是 `08f50d7...`，且 `sourceTreeSha256` 与当前源码树不一致 | 读取合并提交中的 `installer-pack/pack.json`，再在当前树运行构建器后摘要发生变化 | 代码化为 `build-input` 语义；新增只读 provenance 校验；重新生成包 |
| `P1-PDF-REDIST` | `pdf-processing-toolkit` 已被禁用，但公开树仍包含疑似带专有许可声明的长篇导入正文 | `skills/pdf-processing-toolkit/SKILL.md` 声明缺失 `LICENSE.txt`，正文仍可被公开复制 | 删除导入正文，换成带固定 marker 和大小门限的公共隔离占位；继续禁止启用 |
| `E1-RUNTIME` | Python 3.12、wheelhouse、Node 20 和完整 Feishu npm closure 尚未公开物化 | 基线为 `materializationStatus=pending`、`installReady=false` | 已由全量安装包补齐；安装器逐项校验哈希并执行离线健康检查 |
| `E2-FEISHU-E2E` | 没有真实租户、权限和凭据，无法证明真实 Feishu RPC | 当前只做 wrapper/config/dry-run 契约 | 保持 guided-config；文档明确未执行，不能改成通过 |

## 3. 冻结执行契约

- `COMPLEXITY_GATE`：`ESCALATE_REQUIRED`。原因是生成物、发布 provenance、许可证隔离和安装门禁跨越多个模块。
- 唯一写入者：当前主控会话；允许修改范围为本任务文档、构建/验证/审计脚本、PDF 隔离元数据与占位、README/限制说明，以及由构建器生成的 `installer-pack/`。
- 不变项：Codex Agent TOML 必须继续含 `name`、`description`、`developer_instructions`；Skill 元数据和非核心 `agents/openai.yaml` 契约不变；安装器不得删除未拥有文件；所有密钥继续只来自本地环境。
- `AUDIT_OWNER`：固定版本后的主控只读复核。当前平台没有可用的独立子 Agent 派发接口，因此不把本次自检称为独立 Agent 审计；交付中会明确这个证据局限。

## 4. 实施设计

### 4.1 生成包 provenance

`sourceTreeSha256` 是可复现发布的权威标识，因为 Git commit 不能在同一个 commit 内无循环地记录包含自身的生成包。生成器在有 Git 时记录构建输入的 `HEAD`，并将角色标记为 `build-input`；在无 Git 的快照中继续使用 `manifest/source-commit.txt`，角色为 `upstream-base`。

新增 `python scripts/build-pack.py --verify` 只读检查：

- `pack.json` 的 commit、角色和 SHA-256 格式正确；
- 包内 `sourceTreeSha256` 等于当前可编辑源码输入的计算值；
- 不重建、不修改任何文件。

`verify-package.ps1` 无论是否使用 `-SkipRebuild` 都执行该检查。这样旧生成包会在安装前失败，而不是只因包内自洽而通过。

### 4.2 PDF 隔离

公开树只保留自有的短占位，包含固定 marker `CODEX_PUBLIC_QUARANTINE_STUB_V1`、许可证阻断说明和降级路径。清单声明 `sourceBodyIncluded=false`，静态审计同时检查 marker、文件大小上限和 unsupported 状态。只有获得可核验再分发证据并重新审查后，才允许替换占位实现；本任务不执行该替换。

### 4.3 外部条件

公开仓库仍不含真实凭据，因此 `E2` 不会被静态测试“修成绿色”。`E1` 的本地介质闭环由全量安装文档和
`runtime/provision-runtime.ps1` 负责；验收同时证明状态准确、缺条件时安全失败、模板不泄密、默认安装不启用不可验证能力。

## 5. 验收矩阵

| ID | 成功条件 | 方法 |
| --- | --- | --- |
| `R1` | 旧生成包 provenance 漂移可被发现；重新构建后只读验证通过 | `python scripts/build-pack.py --verify`；`verify-package.ps1 -SkipRebuild` |
| `R2` | 源树和生成包均只有 PDF 隔离占位，且默认/非默认安装仍排除该 Skill | Codex compatibility audit、安装 smoke、文件大小/marker 检查 |
| `R3` | 50 Skill、7 Agent、25 默认 Skill、哈希、路径安全和 adapter 回归不退化 | `verify-package.ps1`、`test-adapter-contracts.py` |
| `R4` | 包内运行时介质可离线安装，Feishu 租户授权仍 fail-closed | 检查 ready manifest、逐文件哈希、隔离安装、无密钥渲染、wrapper dry-run |

## 6. 交付状态

固定版本复核结果：`--verify`、源树/生成包兼容性审计、34 项适配器回归、PowerShell 7 和 Windows PowerShell 5.1 的
`verify-package.ps1` 均通过；50 Skill / 7 Agent 全量隔离安装、Python 52-wheel 导入、Node 20、Feishu CLI、配置合并、
重复安装和篡改包拒绝也通过。任何代码或源文档再次变更都会使生成包和既有 provenance 证据失效，必须先重建再复核。
真实租户/API 调用仍未执行，不能把 guided-config 能力写成已获得外部权限。
