# 全量安装包与一键部署开发文档

## 1. 目标

把本仓库审计过的 Skills、Agents、MCP 描述、运行时源码和可离线安装介质放进同一个可复现安装包，使一台全新的 Windows x64 电脑可以通过一个入口完成：

1. 安装/复用 CPython 3.12；
2. 创建独立 Python 虚拟环境并从本地 wheelhouse 安装完整传递依赖；
3. 离线部署 Node.js 22、FFmpeg/FFprobe 和 Manim；解压锁定的飞书 MCP npm 依赖；联网时再安装 HyperFrames CLI 并准备 Chrome；
4. 安装全部 56 个 Skill 和 7 个 Agent；
5. 生成 Codex 的托管配置片段，并保留用户原有配置；
6. 记录运行时、配置和连接健康状态；
7. 在没有凭据时保持 Feishu/API 能力关闭，而不是伪装成可用。

本文件是对 `docs/POST-MERGE-CORRECTIVE-DEVELOPMENT.md` 的后续落地说明。公开安装包不包含 App Secret、OAuth 会话、API Key 或租户数据。

## 2. 范围与能力边界

### 包内自动完成

- Skills、Agents、审计清单、MCP/API 配置表和工具 allowlist；
- CPython 3.12.10 Windows x64 安装器；
- Node.js 22.22.0 Windows x64 ZIP、FFmpeg 9 LGPL shared ZIP；
- `@larksuiteoapi/lark-mcp@0.5.1` 的 npm closure、lockfile 和 CLI；
- 77 个 Python wheel、基础与 Manim 哈希锁文件和 wheelhouse manifest；
- 安装脚本、配置渲染器、健康检查、契约测试和文件哈希清单。

### 仍需用户首次配置

- Feishu App ID、App Secret、租户授权/可访问资源；
- image-2、Seedance、Sub2API、ServerChan 等外部服务的密钥和 endpoint；
- 需要登录或用户授权的第三方服务；
- Codex 本身的账号登录与模型配额。

因此，“一键安装”指本地代码、运行时、依赖和 Codex 接入自动就绪，不代表外部 SaaS 在没有用户凭据时自动获得访问权。

## 3. 入口与安装行为

安装包根目录提供两个入口：

- `Install-Codex-Starter.cmd`：适合双击，调用 PowerShell 全量入口；
- `install-all.ps1`：适合脚本化，默认启用全部非核心和暂不支持项，并覆盖安装包拥有的文件。

两者最终调用 `install-to-codex.ps1`，参数等价于：

```powershell
./install-to-codex.ps1 -IncludeNonDefault -IncludeUnsupported -Overwrite
```

如需隔离验证，可传入 `-TargetUserProfile <目录>`；隔离模式只把环境和配置写入目标目录，不修改当前用户的真实 PATH 或用户级环境变量。

## 4. 安装顺序

```text
校验 pack.json / checksums
        ↓
校验 bundled media manifest、文件字节数和 SHA-256
        ↓
安装或复用 CPython 3.12
        ↓
创建 runtime/python-env
        ↓
--no-index + --require-hashes 安装 77 个 Python wheel
        ↓
导入健康检查（含 PIL、jsonpath、Office/PDF/数据栈）
        ↓
解压 Node.js 22 与 FFmpeg/FFprobe
        ↓
离线安装 Manim 0.21 独立环境；检查 HyperFrames CLI/Chrome（npm/浏览器下载需要网络）；解压并验证 Feishu MCP npm closure / CLI
        ↓
复制 56 Skills、7 Agents、manifest、连接模板
        ↓
渲染托管 Codex 配置并合并到用户 config.toml
        ↓
写入 runtime-state.json、connection-status.json、readiness.json
```

Python、Manim、Node 和 FFmpeg 的介质哈希/健康检查失败会终止运行时安装；若 HyperFrames 的 npm 或 Chrome 下载离线失败，只将该能力标记为 `requires-network`，其它本地能力照常安装。

## 5. 配置合并规则

`configure-codex.ps1` 只管理带有以下标记的区块：

```text
# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1
# END CODEX-STARTER-KIT MANAGED CONFIG v1
```

- 第一次安装：备份原 `config.toml` 后追加托管区块；
- 再次安装：只替换旧托管区块；
- 发现用户已有未托管的 `[mcp_servers.feishu]`：返回 `conflict`，不覆盖用户配置；
- 所有密钥只从当前进程环境读取，绝不写入公开包、manifest 或源码；
- 无 Feishu 凭据时，生成禁用状态；有凭据时也必须通过 Node/MCP 健康检查后才标记为待租户验证。

## 6. 介质与可复现性

权威文件：

- `manifest/runtime-artifacts.json`：运行时和 MCP 介质的版本、来源、哈希、签名/校验要求；
- `runtime/bundled/media-manifest.json`：包内实际介质的相对路径、字节数和 SHA-256；
- `runtime/wheelhouse-manifest.json`：86 个 wheel 的逐文件哈希；
- `runtime/bundled/python-requirements.lock`：带哈希的离线 pip 安装锁；
- `installer-pack/checksums.sha256`：最终安装包文件清单。

`build-pack.py` 是唯一的安装包构建入口。它会重新复制源文件、排除 `drafts`、`output`、`__pycache__` 等用户生成目录，并重写包级 manifest、依赖清单和哈希清单。

## 7. 验收标准

完整验收必须同时满足：

- source audit：56 Skills、7 Agents、manifest 引用闭合；
- package audit：安装包没有缺失源文件或 transient 用户目录；
- media audit：Python、Node、FFmpeg、Manim、Feishu npm closure、77 wheel 全部存在且哈希相符；
- default smoke：核心安装可完成，runtime ready，Codex config merged；
- full smoke：56 Skills / 7 Agents 全量安装可完成；
- dependency health：Python 导入探针、Node/FFmpeg 版本、Manim 导入和 Feishu CLI help 均通过；HyperFrames 在线初始化状态单独记录；
- security：公开包不含密钥；高风险工具仍受 allowlist 和写操作确认策略约束；
- repeatability：同一包在全新隔离 profile 上重复安装，结果一致且不污染主用户配置。

推荐执行：

```powershell
python scripts/build-pack.py
powershell -ExecutionPolicy Bypass -File scripts/verify-package.ps1
```

## 8. 已知限制

- 仅针对 Windows x64 / CPython 3.12 介质闭环；其他平台仍需重新物化 runtime；
- Python/Node 本地介质可以离线安装，但外部 API、Feishu 租户权限和第三方登录不能离线伪造；
- 安装程序不自动替用户写入秘密，也不绕过 Codex 或外部服务的授权流程；
- 安装包体积明显增大，发布前应确认 GitHub Release/LFS 或其他介质分发策略；
- 当用户配置存在未托管的同名 Feishu MCP 时，安装会报告冲突并保守退出该合并步骤。
