# G19 一键更新连通性修复

状态：源码已修复，待能力热更新器合并后统一打包。

## 1. 问题结论

G19 报告把两个更新通道混在了一起：

| 通道 | 用途 | 正确入口 |
| --- | --- | --- |
| 管理器自更新 | 更新 Codex App 管理器本身 | `https://codexapp.agentsmirror.com/manager/latest.json` |
| Codex 载荷更新 | 更新已授权 U 盘中的 Windows MSIX 与根元数据 | `https://codexapp.agentsmirror.com/latest/manifest`、`latest/checksums`、`latest/win-{arch}` |

`manager/latest.json` 是 Tauri 管理器自更新清单，不能作为 Codex 载荷的
`latest.json` 描述符解析。旧版 Codex 载荷默认 Relay
`update.404token.xyz/__codex_update__/` 已实测返回 404，因此自动更新会先撞到
废弃地址，再依赖 GitHub API/CDN 的偶发可达性。

## 2. 修复策略

1. Codex 载荷新增独立的 `mirror` 源，读取镜像的 Windows manifest、checksum
   与按架构的 MSIX 直链；不再把管理器自更新清单当作载荷清单。
2. `auto` 顺序调整为：国内镜像 → 仅在配置了非镜像地址时尝试兼容旧 Relay → GitHub。
3. 旧版默认 404token 地址自动迁移到国内镜像；用户自定义的 Relay 地址不被改写。
4. 元数据源按网络/超时错误最多重试一次并退避；校验、解析类错误不盲目重复。
5. 镜像固定域名允许其 HTTPS 对象存储跳转；下载仍必须通过 MSIX Authenticode、镜像
   checksums、包身份、架构和尺寸校验后才进入 U 盘事务提交。
6. 检查、源选择、下载、checksum、Authenticode/identity、提交和管理器自更新均写入
   管理器日志；错误保留 source、host、阶段和底层 curl 错误。
7. 元数据请求使用 10 秒连接超时、15 秒请求上限、20 秒进程兜底；MSIX 大文件下载
   继续使用独立的长时下载限制，不会被元数据超时误伤。

## 3. 关键实现

- `src-tauri/src/app/channels/codex.rs`
  - 增加镜像 manifest/checksum 解析。
  - 增加镜像优先、旧 Relay 兼容、GitHub 后备和元数据重试。
  - 增加更新阶段日志，apply 重新解析同一源并继续执行 TOCTOU/完整性校验。
- `src-tauri/src/app/settings_store.rs`
  - 新增 `mirror` 源类型。
  - 默认地址改为 `https://codexapp.agentsmirror.com/`。
  - 自动迁移旧版内置 404token 地址。
- `src-tauri/src/commands.rs`
  - 管理器自更新检查与安装加入日志边界。
- `crates/codex-win-engine/src/sys.rs`
  - 收紧小型元数据请求超时；下载超时保持独立。
- `src/shared/types.ts`、`src/services/managerApi.ts`、`src/app/views/Settings.tsx`、
  `src/app/i18n.tsx`
  - 同步镜像源类型、默认值、设置项和说明。

## 4. 安全边界

- `manager/latest.json` 继续只由 Tauri updater 使用。
- Codex 载荷更新只接受 HTTPS；镜像资产只能从配置源同源派生。
- 镜像重定向只对内置 `codexapp.agentsmirror.com` 源开放；自定义 Relay 默认不跟随跨域跳转。
- 任何下载都必须通过 SHA256、OpenAI Authenticode、MSIX identity、架构与版本检查；失败
  走事务回滚，不修改现有可用 payload。
- 更新仍要求已授权、未变化的签名 U 盘；不联网时在短超时后给出可诊断错误，不静默挂死。

## 5. 验收要求

### 自动化

- 类型检查、前端单测、前端构建、lint。
- Rust 格式检查、管理器单测、Windows engine 单测。
- 镜像源 fixture：正确解析 x64/arm64、拒绝架构不匹配、拒绝缺失 checksum、拒绝非法包尺寸。
- 自动策略 fixture：镜像成功不访问 GitHub；镜像网络失败重试后才回退；最终错误包含源和阶段。

### 手工

1. 在线且 GitHub 不稳定时，点击“检查并安装更新”，日志应先出现 mirror，能够从
   `latest/manifest` 发现版本并进入确认。
2. 镜像元数据可达但资产发生跳转时，下载可继续，最终仍必须通过 SHA256 和 Authenticode。
3. 断网启动后点击更新，约 20 秒内显示失败提示，日志包含失败 host/stage，不出现无响应。
4. 在设置中选择 Github 时，不得访问镜像；选择旧 Relay 时保留旧 `latest.json` 兼容行为。
5. 管理器自更新仍使用 `/manager/latest.json`，不受 Codex payload 源解析影响。

本次只修改源码和文档，未执行 Tauri 打包，也未写入 U 盘。
