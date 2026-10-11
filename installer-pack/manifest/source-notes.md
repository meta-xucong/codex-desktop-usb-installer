# 整理与审计说明

当前公开树基于上游 commit `671f2f486ce10825e02ea547e3ce3667062676f0` 整改。构建器在有 Git
时把构建输入的 `HEAD` 记录为 `sourceCommit`，角色标为 `build-input`；没有 `.git` 的源码快照
则读取 `manifest/source-commit.txt`，角色标为 `upstream-base`。两者都是来源指针，不能替代
`sourceTreeSha256`。后者才是生成包必须与当前可编辑源码相等的可复现身份；提交生成包后，
它不要求也不能要求等于包含生成包的最终 commit。

发布前运行 `python scripts/build-pack.py --check` 重建 `file-manifest.json` 与
`checksums.sha256`，再运行 `python scripts/build-pack.py --verify`。`verify-package.ps1`
提供不依赖 Python 的 Windows 闭包、哈希和来源树校验；若本机有 Python，还会继续执行兼容性审计。
`--verify` 只读计算源码树并拒绝过期的 `installer-pack/`，避免 squash merge 或文档变更后继续发布旧生成物。

`sourceTreeSha256` 的计算范围是包内除 `pack.json`、`file-manifest.json` 和
`checksums.sha256` 之外的全部文件；每项按包内 POSIX 相对路径长度、路径和文件内容 SHA-256
进行有界拼接，文本先按 LF 规范化。这样不会产生 `pack.json` 自引用循环，同时能检测脚本、文档、Skill、Agent、MCP 和运行时介质的漂移。

本目录记录公开能力包的范围、兼容性和部署边界。内容以功能为中心，不包含个人身份、账号配置或一次性运行状态。

## 已纳入

- 55 个带独立元数据和功能说明的 Skill；5 个竞品能力被改写/归并为原创、可移植工作流
- 7 个按功能命名的 Agent TOML
- Windows 10/11 优先的安装、打包和验证脚本
- Feishu 与统一媒体生成本地 MCP（含预留 MiniMax 模型槽位）、4 个官方远程 OAuth MCP；DashScope 直连 API 的无密钥配置清单
- 47 个默认启用 Skill（含 18 个可自动安装运行时 Skill、4 个新增本地流程 Skill 和 3 个安全降级 Skill）、完整依赖闭包与 SHA-256 文件清单
- Python 运行时直接输入、CPython 3.12 安装器、77-wheel 离线 wheelhouse、Manim 哈希锁、Node 22/FFmpeg/Feishu 介质、逐文件哈希和 Windows per-user 安装契约

## 明确不纳入

- API Key、Token、Cookie、`.env`、私有 URL 和账号配置
- 会话历史、记忆数据库、日志、缓存、草稿和生成产物
- 依赖特定宿主协议的远程画布、设备节点和网关控制
- 用户秘密不纳入；包内只包含公开且经清单锁定的运行时介质

## 兼容性原则

每个 Skill 必须在 `manifest/skill-audit.json` 中声明原始状态、整改后状态、依赖对象、启用条件和健康检查。
每个 Agent 必须在 `manifest/agent-audit.json` 中声明必需 Skill、可选 Skill、运行时依赖、连接依赖和降级路径。
无法在当前 Codex 环境确认的能力会标记为 `auto-installable-runtime`、`guided-config` 或 `unsupported`。
`unsupported` 可以进入默认配置以便模型明确说明边界，但缺失外部后端时必须安全失败，不能以“已经可用”冒充真实能力。
