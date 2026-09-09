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
| 源 | AppSettings.codex_payload_repo；空值使用 Wangnov/codex-app-mirror |
| 格式 | 严格 owner/repo，仅字母、数字、-、_、. |
| API | https://api.github.com/repos/{repo}/releases/latest |
| API 请求头 | Accept: application/vnd.github+json、稳定 User-Agent |
| 资产 | 当前架构的 MSIX、SHA256SUMS-windows.txt、release-manifest.json |
| 比较版本 | release manifest 中当前架构的 appVersion |
| 本地事实 | 根 manifest.json.codexDesktopVersion |
| 校验 | 大小、SHA256SUMS、GitHub digest（存在时）、Windows Authenticode、MSIX identity |
| 更新范围 | USB payload 与三个根元数据文件；不自动部署本机 AppX |

packageVersion/MSIX identity version 与用户可见的 appVersion 不同，不能
互换。资产 URL 只取 fresh API response 的 browser_download_url，apply
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

## Settings 与 UI

codex_payload_repo 与现有 UpdateSource::Custom/custom_url 完全分离。
Settings 支持填写、保存、清空恢复默认和非法值提示。About 的 Codex payload
入口只在 Windows 且当前受保护 USB 状态可用时呈现，保留显式确认；apply
返回 stale 后清除 pending 并要求重新 check。

刷新 USB payload 不等于更新当前 Windows 已部署 Codex。后者仍是独立、显式
确认的部署动作，本通道不会自动调用。

## 失败语义

| 场景 | 结果 |
| --- | --- |
| v1/未知 capability | 远端请求前拒绝，要求重建/重新授权 |
| source/release/asset/本地版本变化 | StaleExpectation |
| 下载、大小、checksum、digest、签名或 identity 失败 | 清理 staged 文件并保持旧 bundle |
| metadata 或 rename 中途失败 | 回滚 old payload + 全部 old metadata，保留可重试性 |
| 非 Windows | UnsupportedPlatform |

manager 自更新仍由 Tauri updater 负责；ccswitch 和 skill-repo 仍未实现。
