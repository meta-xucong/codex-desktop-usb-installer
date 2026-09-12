# 卸载残留与收尾竞态修复记录

状态：已落地代码修复与隔离回归，未在当前开发机执行真实数据删除。

## 1. 用户目标与边界

用户选择明确清理时，本机的 Codex、CCSwitch、能力包 Skills/Agents、能力包运行时与相关配置应进入同一次清理契约；未选择清理时仍保留共享用户数据。

U 盘发行目录是独立边界。`removeUsbMedia=false` 时，清理计划不得包含 U 盘路径；因此 `content/codex-starter-kit`、`content/skills`、`content/mcp` 和安装包本身不会被本机数据清理触碰。只有用户另行开启“删除当前 U 盘安装包”并通过卷身份、发行清单、许可证和文件类型复核时，才允许进入 U 盘清理分支。

## 2. 报告诊断

开始菜单与桌面快捷方式是本机 Shell 元数据，不属于 U 盘清理。卸载会回收本产品能证明归属的用户/公共开始菜单和桌面链接；U 盘中的 apps\Codex 仅作为链接目标被引用，不会因为回收链接而删除发行包。

测试机本次报告中的：

```text
临时组件收尾进程尚未就绪即退出：exit code: 0
```

不是 API Key、飞书或可选 MCP 未填写造成的失败，也不等同于 5 个本机目标文件删除失败。旧构建的 PowerShell 收尾脚本可能已经把临时目录清掉并以 0 退出，但 `finalizer.ready` 是一个短生命周期握手标记；父进程在轮询间隔内错过它后，只做了一次报告读取。报告正处在写入/被系统扫描器短暂占用时，就会形成“exit code 0、没有错误文件、却被判定为失败”的 false negative，并额外登记重启兜底。

本轮修复同时补齐两层确认：

1. PowerShell 收尾脚本写完报告后立即回读，只有确认 `helperFinalization=removed` 且 `status` 为终态才允许返回 `exit 0`；
2. Rust 父进程错过 `finalizer.ready` 时，对终态报告做最多 5 秒、100 毫秒间隔的重试；读取/解析错误不再被 `unwrap_or(false)` 吞掉，而是保留在最终诊断中。

因此，新构建的收尾判定为：

```text
子进程 exit code == 0
且卸载报告 helperFinalization == removed
且报告 status == completed 或 partialFailure
=> 收尾成功（partialFailure 仍按报告保留实际失败项）
```

只有 5 秒确认窗口结束后仍缺少上述持久化证据时，才报告收尾失败；此时报告会给出“报告未进入终态”或实际读取错误，而不再显示矛盾的“exit code 0 + 未写入错误详情”。测试机必须使用包含本轮修复的新版管理器重新执行卸载，旧报告本身不会被自动改写。

## 2.1 G15：旧版快捷方式死链与桌面报告残留

G15 的 ChatGPT.lnk 不是 U 盘程序仍在运行，而是旧版便携流程创建的本机 Shell 元数据；
目标 ChatGPT.exe 被删除后，链接仍可被开始菜单索引。旧版由提权进程写入时还可能留下
普通用户没有删除权的 ACL，桌面上的多份 JSON 则是旧报告输出位置造成的残留。

本轮修复已覆盖三条路径：

1. 新建快捷方式写入归属记录，并给当前交互用户完全控制权；
2. 卸载时按归属证据回收 Codex/ChatGPT/CCSwitch 等固定候选名，覆盖用户/公共开始菜单和桌面，
   并做有限重试与删除后复核；
3. 管理器启动时只扫描“已证明归属且目标已不存在”的死链，删除后发送 Shell 目录刷新通知；
   新报告改写入本机日志目录，旧桌面报告只按精确前缀和扩展名清理。

同名但目标属于其他目录、归属证据不足或处于重解析点下的链接都会保留并记录，不通过模糊
文件名匹配扩大删除范围。

## 3. 本机清理范围

### 3.1 Shell 残留与卸载报告

- 便携版安装记录快捷方式创建清单；清理优先使用清单、受管卸载项和链接目标三重归属证据，覆盖 Codex.lnk、历史 ChatGPT.lnk、CCSwitch.lnk 等固定命名；
- 用户开始菜单、公共开始菜单、用户桌面和公共桌面均只检查固定候选名；无法证明属于本产品的同名链接保留，不做全目录或模糊删除；
- 快捷方式由提权进程创建时，创建脚本授予当前用户对该 .lnk 的完全控制；卸载阶段仍以提权上下文执行，避免留下普通权限无法删除的死链；
- 管理器启动时会做一次轻量死链扫描：只移除已证明属于本产品且目标已不存在的链接，并通知 Windows Shell 刷新目录；有效链接和无法证明归属的链接不动；
- 新的卸载报告统一写入 %LOCALAPPDATA%\CodexAppManager\logs（不可写时回退到 %TEMP%\CodexAppManager\logs），不再写到桌面；新版本会在本地清理中回收旧版本桌面上的 Codex-Uninstall-Report-*.json 精确文件。

显式勾选 Codex 数据清理时，新增纳入：

- `%USERPROFILE%\\.agents\\skills`：离线能力包实际安装的本机 Skills 根目录；
- `.codex` 下的 Agents、能力包运行时、MCP wrapper、readiness、诊断和配置片段仍由已有 `.codex` 精确目标覆盖；
- 能力包登记过的当前用户环境变量（例如 API Key、端点和模型名）会先从 `.codex\\codex-agent-kit\\connection-ownership.json` 读取名称，再只删除这些已登记名称的 `HKCU\\Environment` 值；不会扫描或清空其他环境变量；
- Codex AppX 数据、桌面运行时缓存、Roaming 数据和精确恢复备份仍按原白名单处理。

显式勾选 CCSwitch 数据清理时，继续清理 `.cc-switch`、Local/Roaming `com.ccswitch.desktop`。普通卸载仍不删除这些共享用户数据。

空的 `.agents` 容器只在 Codex 数据清理分支中做非递归移除；含有其他内容时保留，不扩大到整个 `.agents`。

## 4. 安全验收

- 默认清理目标不包含 `.codex`、`.agents\\skills`、CCSwitch 用户数据或任何 U 盘路径。
- Codex 数据清理目标包含 `.codex` 和 `.agents\\skills`；CCSwitch 数据清理目标与 Codex 目标保持独立。
- 收尾报告的 `removed` 证据可覆盖握手标记被错过的时序；`pending/finalizing` 不会被当成成功。
- 缺少或损坏连接所有权记录时，不会猜测 API Key 所在变量；报告会保留失败项，避免把用户自己的环境变量误删。
- 快捷方式清理失败会保留 Start Menu shortcut cleanup failed 证据并提供重试；未证明归属的同名链接会被记录为保留，而不是误删。
- 旧的 U 盘内容保留契约不变，未为修复 false negative 而扩大介质删除范围。
- 本轮只使用源码、合成报告和隔离测试；没有对当前用户的 Codex、CCSwitch、Skills 或 U 盘执行真实删除。

## 5. 发布前仍需验证

在测试机上运行正式卸载前，应确认 UI 的“高级数据清理”二次确认明确列出 `.agents\\skills` 和能力包登记的当前用户环境变量，并在 `removeUsbMedia=false` 下对 U 盘做前后哈希对照。若选择 U 盘介质删除，则必须另行记录该选择，不得把它与本机数据清理混为默认行为。
