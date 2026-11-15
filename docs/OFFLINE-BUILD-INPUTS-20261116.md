# USB 离线构建输入恢复

本说明绑定 GitHub Release `usb-offline-20261116`，其源码标签与当前已核对的 USB 源码提交对应。该 Release 是补充构建输入的预发布，不是 Codex 安装器或 USB 发布包本身。

## 下载并还原能力包输入

在仓库根目录运行以下 PowerShell。命令只下载三个明确命名的 Release 附件，并将其放回生成器清单使用的原路径；不要解压或改名：

```powershell
$repo = 'meta-xucong/codex-desktop-usb-installer'
$tag = 'usb-offline-20261116'
$bundle = 'installer-pack/runtime/bundled'
New-Item -ItemType Directory -Force -Path $bundle | Out-Null
gh release download $tag --repo $repo --dir $bundle `
  --pattern 'chrome-headless-shell-152.0.7977.30-win64.zip' `
  --pattern 'ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip' `
  --pattern 'hyperframes-0.8.134-win-x64-node-modules.zip'
```

Release 页面：[usb-offline-20261116](https://github.com/meta-xucong/codex-desktop-usb-installer/releases/tag/usb-offline-20261116)。这三份附件与 `usb-update-cleanup-1116` 已验收发行目录中的对应文件逐字节一致，名称、长度和安装清单路径均已核对。

| Release 附件 | 还原后的仓库路径 | 用途 |
| --- | --- | --- |
| `chrome-headless-shell-152.0.7977.30-win64.zip` | `installer-pack/runtime/bundled/chrome-headless-shell-152.0.7977.30-win64.zip` | HyperFrames 离线浏览器 |
| `ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip` | `installer-pack/runtime/bundled/ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip` | 视频编码与探测 |
| `hyperframes-0.8.134-win-x64-node-modules.zip` | `installer-pack/runtime/bundled/hyperframes-0.8.134-win-x64-node-modules.zip` | 锁定版本的 HyperFrames CLI 依赖 |

## 构建

恢复三个附件后，`installer-pack` 中受 Git 管理的文件与发行输入齐全，能按项目原构建流程进行能力包验证和封装。安装时由随包部署脚本在本机解压依赖；不需要联网下载这三个组件。

若要从源码重新构建完整 Windows 管理器/USB 发行目录，还要提供与该发行版本匹配的 Codex 签名 MSIX、Microsoft VC++ 和完整离线 WebView2 安装程序，以及构建所需的本地正式签名配置。管理器构建脚本对 Microsoft 文件执行既有签名和完整性检查；本说明不移除、不替换这些门槛。它们不是上述三个能力包 ZIP，也不由本 Release 下载命令代替。正式签名配置、签名库、密钥、账号数据和用户授权文件必须继续从受控的本地位置提供，不得放入 GitHub Release。

所有 ZIP 都保持原始发行文件；完整性清单仍由项目原发布流程维护。此处不生成新的校验值，也不改变现有校验逻辑。
