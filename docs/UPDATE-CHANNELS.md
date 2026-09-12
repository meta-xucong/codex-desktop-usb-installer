# Update Channels — 实现契约

状态：**已实现（Codex USB payload 通道）**。安全与事务边界以
UPDATE-REMEDIATION-DEVELOPMENT.md 的 v2.0-remediation-r1 为准。

## 公共类型与命令

src-tauri/src/app/update_channel.rs 定义 ChannelKind、UpdateInfo 和
UpdateContext。当前 Codex 命令保持为：

    payload_check_codex() -> Option<UpdateInfo>
    payload_apply_codex(info) -> ()

UpdateContext.bundle_root 是当前 manager 实例授权的 USB bundle 根目录，
不是 <bundle>/apps/Codex 提取目录。根目录至少包含：

    manifest.json
    usb-manifest.json
    SHA256SUMS.txt
    payload/OpenAI.Codex_<arch>.msix
    apps/Codex/

命令桥接从 usb_guard::authorized_bundle_identity().0 获取路径；renderer
不能提供路径。UpdateInfo.payload 只保存远端 artifact identity，不保存可
直接信任的下载 URL。

## Codex channel

实现文件：src-tauri/src/app/channels/codex.rs。

| 项目 | 实现 |
| --- | --- |
| 源 | `auto` 依次尝试国内镜像、内置兼容 Relay（或用户配置的 Relay）、GitHub；也可显式选择其中一种 |
| 格式 | 严格 owner/repo，仅字母、数字、-、_、. |
| 国内镜像 API | `https://codexapp.agentsmirror.com/latest/manifest`、`latest/checksums`、`latest/win-x64` / `latest/win-arm64` |
| 兼容 Relay API | 配置目录下的 `latest.json` 描述符；仅保留兼容用途 |
| GitHub API | https://api.github.com/repos/{repo}/releases/latest |
| API 请求头 | Accept: application/vnd.github+json、稳定 User-Agent |
| 资产 | 镜像：`latest/manifest`、`latest/checksums`、`latest/win-{arch}`；GitHub：当前架构的 MSIX、SHA256SUMS-windows.txt、release-manifest.json |
| 比较版本 | release manifest 中当前架构的 appVersion |
| 本地事实 | 根 manifest.json.codexDesktopVersion |
| 校验 | 大小、SHA256SUMS、GitHub digest（存在时）、Windows Authenticode、MSIX identity |
| 更新范围 | USB payload 与三个根元数据文件；不自动部署本机 AppX |

`/manager/latest.json` 是管理器自更新清单，不属于 Codex payload channel；不能
拿它代替上述镜像 manifest。packageVersion/MSIX identity version 与用户可见的
appVersion 不同，不能互换。镜像资产 URL 只从同源 `/latest` 路径派生，GitHub
资产 URL 只取 fresh API response 的 browser_download_url，apply
会重新获取并完整比较 checked identity：repo、release id/tag、三个资产的
id/name、package moniker、架构、MSIX version、appVersion 和 digest。

## v2 capability 与 USB anchor

可更新 bundle 必须同时满足 root manifest.schemaVersion == 2、
usb-manifest.schemaVersion == 2，并声明 updateCapability：

    {
      "codexPayload": true,
      "schemaVersion": 2,
      "checksumFile": "SHA256SUMS.txt",
      "transactionDirectory": ".staging",
      "backupPolicy": "single-msix-listed-in-SHA256SUMS",
      "successState": "clean"
    }

usb_guard 的 v2 anchor 对两个 manifest 做确定性 JSON key 排序，只移除
明确允许变化的字段：

    manifest.json:
      codexDesktopVersion, packageVersion, releaseTag,
      payload.sha256, payload.size
    usb-manifest.json:
      codexInstaller.payloadSha256, codexInstaller.payloadSize

其它字段、字段类型、布局、架构和产品身份都会影响 anchor。旧 v1 bundle
继续使用旧 raw anchor，但 Codex 更新在远端请求前拒绝 v1/未知版本，要求重建
并重新授权；不会通过更新命令自动 enroll 或重签 license。

## Apply 事务

apply 的第一条可执行语句是 usb_guard::require_authorized()?，随后重新观察
授权 bundle 并确认与 context 根一致。整个 apply 使用进程内 Codex update
lock。

下载和校验在 <bundle>/.staging/codex-update-<uuid>/ 完成。destructive
rename 前会重新读取当前版本、payload 路径、架构和 payload 状态。成功提交：

1. 将旧 MSIX 移至 payload/_backup/OpenAI.Codex_<arch>_<old-version>.msix，
   只保留一份；
2. 替换 payload；
3. 生成 next root manifest、next usb-manifest 和覆盖全部实际发行文件的
   根 SHA256SUMS.txt（usb-license.json 是 mutable runtime metadata，
   不列入清单）；
4. 通过事务状态逐个提交三个元数据文件；
5. 重新校验清单与 payload 元数据，最后清除 staging 和 writer .bak。

异常会依据 old metadata snapshot 和 backup 回滚；下次 apply 会先处理未完成
事务。check 不执行写操作；发现待恢复事务时返回明确错误。

Windows portable fallback 使用 `<bundle>/apps/.codex-app-manager-staging/` 暂存
MSIX 解包结果，再在提交边界原子替换 `apps/Codex`。该目录与 `.staging/`、
`apps/Codex/` 一样属于管理器拥有的运行期目录：它不进入静态
`SHA256SUMS.txt`，但只能位于这两个精确路径下，bundle 其他未知文件仍必须列入
清单。USB portable 的 precommit 检查发生在移动旧 `apps/Codex` 之前，检查失败
不会破坏可用旧版本。

## Settings 与 UI

codex_payload_repo 与现有 UpdateSource::Custom/custom_url 完全分离。
Settings 支持填写、保存、清空恢复默认和非法值提示。About 的 Codex payload
入口只在 Windows 且当前受保护 USB 状态可用时呈现，保留显式确认；apply
返回 stale 后清除 pending 并要求重新 check。

`payload_apply_codex` 本身只刷新 USB payload，不自动部署当前 Windows 安装；这条
后端通道边界保持不变。USB 主页的“检查并安装更新”入口在用户明确确认后，先调用
该通道，再串接既有的 `offline_payload_stage` + `win_install_historical_release`
受保护部署链，因此用户可以一键完成“检查并安装”，但仍不会在后台自动更新或绕过
确认。About 页的 Codex 更新入口仍只负责刷新 payload，适合不立即部署的维护场景。

### Automatic source fallback

自动模式的实际顺序固定为：

1. `https://codexapp.agentsmirror.com/` 的镜像协议（`latest/manifest`）；
2. 内置 `https://update.404token.xyz/__codex_update__/` 的兼容 Relay 协议
   （`latest.json`）；如果用户填写了自定义 Relay 地址，则使用该地址；
3. GitHub Release API。

镜像和 Relay 是两种不同的 metadata 协议，不能只把镜像 URL 改名后请求
`latest.json`。客户端会为 Relay 统一计算有效 base URL，并将该 URL 同时用于
解析、日志 locator、check/apply stale identity 和同源资产派生，避免出现“检查时
走了一个地址、应用时却被判定为另一来源”的不一致。任一候选源失败都会保留前面
候选源的诊断信息；全部失败时不修改 U 盘。

## 失败语义

| 场景 | 结果 |
| --- | --- |
| v1/未知 capability | 远端请求前拒绝，要求重建/重新授权 |
| source/release/asset/本地版本变化 | StaleExpectation |
| 下载、大小、checksum、digest、签名或 identity 失败 | 清理 staged 文件并保持旧 bundle |
| metadata 或 rename 中途失败 | 回滚 old payload + 全部 old metadata，保留可重试性 |
| 非 Windows | UnsupportedPlatform |

manager 自更新仍由 Tauri updater 负责；ccswitch 和 skill-repo 仍未实现。
