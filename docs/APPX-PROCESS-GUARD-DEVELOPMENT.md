# Codex U 盘版 AppX 进程绑定改造文档

状态：P2 设计基线，待实现。本文基于当前已完成的 P1 U 盘看门狗，目标是增加“从开始菜单直接启动的 Codex 也受 U 盘授权约束”，同时尽量减少不必要的进程终止和误报特征。

日期：2026-08-31

适用仓库：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

## 1. 改造目标

当前 P1 已经能够：

- 从可移动盘读取并校验离线 Codex MSIX；
- 以卷类型、卷序列号、清单锚点哈希和包内相对路径绑定 U 盘；
- 启动本地看门狗；
- 登记由管理器启动的 Codex/CCSwitch PID；
- U 盘拔出后约 5 秒结束本次管理器登记的进程。

P2 需要补足：

1. 用户关闭管理器后，看门狗仍可以在用户明确开启的情况下继续工作；
2. 用户从开始菜单直接启动 Codex 时，也能识别并纳入授权检查；
3. 识别依据为“AppX 身份 + 安装路径 + PID + 进程创建时间”，不使用全局进程名杀进程；
4. 不修改 `WindowsApps` 内的 Codex 文件，不重打包官方 MSIX，不注入 DLL；
5. 看门狗、任务计划、状态和卸载入口对用户可见；
6. 保留 P1 作为兼容降级路径，P2 探测失败时不能误杀进程。

本改造不是 DRM 或反逆向方案。拥有本机管理员权限的用户仍可停止任务、结束看门狗或修改本地文件。它解决的是普通用户误启动、U 盘拔出后仍继续使用和复制包后的运行约束。

## 2. 设计结论

推荐采用：

```text
可见的用户级登录看门狗（可选启用）
        ↓
枚举进程并读取 AppX 包身份
        ↓
比对包身份、安装路径、PID、进程创建时间
        ↓
检查绑定 U 盘是否存在
        ↓
U 盘缺失达到 5 秒后，仅关闭匹配的 Codex 进程
```

不采用全局 `taskkill /IM Codex.exe`，原因是：

- 它会误伤用户已经运行的其他 Codex 实例；
- 它不能防止 PID 被回收后指向新进程；
- 它比精确身份校验更容易被安全软件判断为高风险行为；
- 它只省掉最后几行过滤代码，不能省掉“常驻看门狗、授权状态和卸载”这些主要工作。

如果官方 Codex 与本产品安装的 Codex 具有完全相同的 AppX 身份，Windows 会把它们视为同一个包。此时 P2 可以明确提供“本机 Codex 独占保护”选项，但不能在进程层再区分“官方安装”与“本产品安装”。默认实现应保护本产品记录的包身份和安装路径；开启独占模式后，才允许影响同包的其他进程。

## 3. 当前实现与缺口

### 3.1 当前代码

| 位置 | 当前职责 | P2 改造方向 |
| --- | --- | --- |
| `src-tauri/src/app/usb_guard.rs` | U 盘指纹、许可证、租约、看门狗、P1 PID 终止 | 增加持久安装记录、AppX 进程探测、进程创建时间校验和直接启动保护 |
| `src-tauri/src/commands.rs` | 安装、启动、CCSwitch、卸载命令 | 安装后保存 AppX 身份；开始菜单保护开关；卸载时删除任务 |
| `src-tauri/src/lib.rs` | Tauri 启动/退出、P1 看门狗启动 | 根据功能开关安装或修复登录任务；管理器退出不再无条件关闭 P2 看门狗 |
| `src/app/views/OfflineHome.tsx` | 离线主页和 P1 U 盘状态卡 | 增加开始菜单保护状态、包身份、修复和明确授权提示 |
| `src/shared/types.ts` | 前后端类型 | 增加 AppX 身份、受管进程和保护模式字段 |
| `tools/build-oneclick-usb-layout.ps1` | 生成发布目录和 `usb-manifest.json` | 写入 P2 能力、默认策略和任务名称 |

### 3.2 当前 P1 的限制

- `lease.json` 的所有者是当前管理器 PID；管理器退出时会移除租约；
- Codex 进程主要通过启动前后 PID 差集登记，开始菜单启动没有经过登记；
- 进程复核目前使用 `tasklist` 的 PID 和镜像名；
- 看门狗是管理器启动期间的本地副本，不是登录后持续运行的安装组件；
- 当前 Release 使用未签名开发许可证，不能作为严格发行防复制信任根。

## 4. 数据模型

### 4.1 持久安装记录

新增 `%LOCALAPPDATA%\CodexUsbGuard\managed-app.json`，不保存 API Key：

```json
{
  "schemaVersion": 1,
  "product": "codex-usb-oneclick",
  "installationId": "uuid",
  "protectionMode": "manager-only",
  "featureEnabled": false,
  "package": {
    "name": "OpenAI.Codex",
    "familyName": "OpenAI.Codex_...",
    "fullName": "OpenAI.Codex_26.825.5331.0_x64__...",
    "publisherId": "...",
    "version": "26.825.5331.0",
    "architecture": "x64",
    "installPath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex_...",
    "executables": ["ChatGPT.exe", "Codex.exe"]
  },
  "usbFingerprint": {
    "volumeSerial": 0,
    "anchorSha256": "...",
    "bundleRelativePath": "codex-usb"
  },
  "createdAtMs": 0,
  "updatedAtMs": 0
}
```

字段约束：

- `package.fullName` 用于严格匹配当前安装版本；
- `package.familyName` 和 `package.name` 用于诊断及版本升级后的重新绑定；
- `package.installPath` 必须是通过 Windows API 得到的实际 WindowsApps 路径，不能接受用户输入的任意路径；
- `package.executables` 从已验证的 AppX 清单提取，初版至少覆盖 `ChatGPT.exe` 和 `Codex.exe`；
- `protectionMode` 取值为 `manager-only`、`start-menu` 或 `exclusive`；
- `featureEnabled=false` 时保持 P1 行为；
- 许可证和 API Key 不复制到本地安装记录中。

### 4.2 受管进程记录

扩展现有 `ManagedProcess`：

```json
{
  "pid": 1234,
  "role": "codex",
  "executable": "C:\\Program Files\\WindowsApps\\...\\ChatGPT.exe",
  "packageFullName": "OpenAI.Codex_26.825.5331.0_x64__...",
  "packageFamilyName": "OpenAI.Codex_...",
  "processCreationTimeFiletime": 133000000000000000,
  "parentPid": 1000,
  "source": "start-menu",
  "startedAtMs": 0
}
```

`processCreationTimeFiletime` 是防止 PID 重用的关键字段。Windows 的 `GetProcessTimes` 可以读取进程创建时间，时间以 FILETIME 的 100 纳秒单位表示。[GetProcessTimes 官方文档](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)

## 5. Windows 进程识别实现

### 5.1 枚举进程

新增 `src-tauri/src/app/appx_process.rs`，职责只限于 Windows 进程查询和匹配，不直接决定产品策略。

推荐调用链：

1. `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)` 获取进程快照；
2. `Process32First` / `Process32Next` 枚举 PID、镜像名和父 PID；
3. 对可能的 Codex 镜像调用 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, ...)`；
4. `QueryFullProcessImageNameW` 读取绝对可执行文件路径；
5. `GetPackageFullName` 读取该进程的 AppX `PackageFullName`；
6. `GetProcessTimes` 读取进程创建时间；
7. 关闭句柄，返回不可变的 `ProcessSnapshot`。

`CreateToolhelp32Snapshot`、`Process32First` 和 `Process32Next` 是 Windows 提供的进程枚举接口。[CreateToolhelp32Snapshot 官方文档](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot)

`GetPackageFullName` 需要对目标进程拥有 `PROCESS_QUERY_INFORMATION` 或 `PROCESS_QUERY_LIMITED_INFORMATION` 权限；未打包的普通进程会返回 `APPMODEL_ERROR_NO_PACKAGE`，此时不能仅凭文件名把它当成 Codex。[GetPackageFullName 官方文档](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagefullname)

### 5.2 安装包身份解析

安装或接管 Codex 成功后，使用当前已经通过 MSIX 身份校验的包记录：

- `packageFullName`：当前版本的完整身份；
- `packageFamilyName`：包名和发布者的稳定组合；
- `publisherId`：发布者标识；
- `version`、`architecture`：版本和架构；
- `installPath`：根据包全名调用 `GetPackagePathByFullName` 获取，不从前端输入；
- `executables`：从 `AppxManifest.xml` 的 Application 节点提取。

`GetPackagePathByFullName` 用包全名解析实际安装路径，可用于防止只比较进程名称。[GetPackagePathByFullName 官方文档](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagepathbyfullname)

### 5.3 匹配规则

将匹配实现为纯函数 `matches_managed_app(process, managed_app, mode)`，便于跨平台单测：

```text
1. executable basename 必须在 managed_app.package.executables 中
2. packageFullName 必须存在，且 packageName / familyName / publisherId 匹配
3. process executable path 必须位于 managed_app.package.installPath 下
4. package path 通过 API 重新解析后，必须与记录的路径一致（Windows 大小写不敏感）
5. process creation time 必须是本次观测到的时间，并在后续复核中保持不变
6. 不满足任一条件：分类为 unknown，不得终止
```

对于版本升级：

- 只有经过现有 MSIX 哈希、身份和签名校验的升级，才允许原子更新 `managed-app.json`；
- 仅因包名相同而出现的新版本不能自动信任；
- 更新完成后重新读取包全名、路径和入口，旧版本进程先按旧记录处理；
- P2 初版不自动放行用户从 Store 或其他渠道安装的未知版本。

对于便携回退：

- P2 的 AppX 保护只覆盖有包身份的 Codex；
- 如果机器进入便携回退模式，继续使用 P1 的“管理器登记 PID + 路径/标记”策略；
- 不能把无 AppX 身份的同名 `Codex.exe` 直接归入受管集合。

## 6. 看门狗生命周期

### 6.1 两类状态分离

当前 `lease.json` 既包含会话所有者又包含受管 PID。P2 应拆成：

- `managed-app.json`：长期安装记录和功能开关；
- `lease.json`：当前管理器会话、动态 PID 和临时状态；
- `status.json`：前端展示；
- `guard.lock` 或 Windows 用户级命名 Mutex：防止多个看门狗并行。

管理器退出时：

- `manager-only`：沿用 P1，停止会话看门狗；
- `start-menu` / `exclusive`：只移除会话 PID，不删除长期安装记录，看门狗继续工作；
- 用户点击“关闭开始菜单保护”或卸载时，才停止并删除长期任务。

### 6.2 登录启动

仅在用户明确启用 `start-menu` 或 `exclusive` 后，创建一个用户级、可见、可卸载的 Task Scheduler 任务：

```text
任务名：Codex USB Guard
触发器：ONLOGON（当前用户）
运行级别：LIMITED
动作：%LOCALAPPDATA%\\CodexUsbGuard\\CodexUsbGuard.exe --codex-usb-guard
```

Windows `schtasks` 支持 `ONLOGON` 触发器和用户级运行级别。[schtasks create 官方文档](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create)

要求：

- 不创建隐藏 Windows 服务；
- 任务描述中写明用途、产品名和卸载方式；
- 任务创建失败时显示明确错误，不能静默回退为隐藏驻留；
- 卸载必须删除任务、配置和本地看门狗目录；
- 普通用户权限即可完成，只有 Codex MSIX 部署本身按系统要求处理 UAC。

### 6.3 直接启动处理流程

看门狗每 500～1000 ms 进行一次快照：

```text
读取 managed-app.json
读取并验证 U 盘指纹/许可证
枚举候选 Codex 进程
对每个候选进程执行 AppX + path + PID + creationTime 匹配

USB present:
    清除 missing_since
    status = authorized

USB missing:
    第一次发现时记录 monotonic missing_since
    0～5 秒：status = grace，只显示倒计时
    >= 5 秒：重新校验每个进程后关闭，status = missing
```

如果用户在 5 秒内重新插回同一设备，清除倒计时；盘符变化只影响显示，不影响指纹匹配。

## 7. 安全关闭进程

### 7.1 关闭前二次校验

所有待关闭进程都必须重新读取：

- PID 仍然存在；
- PID 对应的创建时间等于记录值；
- 可执行文件路径仍在受管安装路径下；
- `GetPackageFullName` 返回值仍符合 `managed-app.json`；
- 进程镜像名仍是清单允许的入口。

任何一项失败都跳过该 PID，并写入诊断日志。

### 7.2 关闭顺序

建议顺序：

1. 对进程顶层窗口发送正常关闭请求；
2. 等待 250～500 ms；
3. 重新执行身份和创建时间校验；
4. 仅对仍匹配的 PID 调用精确终止；
5. 枚举已确认属于该进程树的子进程，逐个重复校验后处理。

P2 不使用按镜像名全局终止。第一版可以暂时复用现有 `taskkill /PID /T`，但最终应迁移到原生 Windows API，减少额外命令进程和脚本型行为；这只能降低部分行为特征，不能保证杀毒软件不报警。

### 7.3 PID 重用测试

必须有测试证明：

- 记录 PID 1000 后，PID 1000 退出并被其他进程复用；
- 新进程的创建时间不同；
- 看门狗跳过新进程，不执行终止。

## 8. 用户界面和用户同意

在现有 OfflineHome 的 U 盘状态卡中增加：

- `U 盘绑定`：当前状态；
- `开始菜单启动保护`：关闭/已启用/看门狗异常；
- 当前包名、版本、安装路径（路径过长时可复制/展开）；
- 当前保护模式：仅管理器、开始菜单保护、Codex 独占；
- 看门狗最近检查时间、倒计时和日志入口；
- `修复看门狗`、`关闭开始菜单保护`、`卸载 Codex 与 CCSwitch`。

首次启用时必须展示确认：

```text
开启后，未插入绑定 U 盘时，从开始菜单启动的本产品 Codex 会在约 5 秒内关闭。
如果本机存在相同 AppX 身份的官方 Codex，Windows 无法区分来源，独占模式可能同时影响它。
```

默认策略：

- 兼容升级：默认 `manager-only`，由用户主动开启开始菜单保护；
- 新 USB 版：安装成功后显示一次确认，可选择 `start-menu`；
- `exclusive` 永远不能静默启用。

## 9. 卸载与修复

卸载顺序：

```text
禁用开始菜单保护
        ↓
删除 Task Scheduler 任务
        ↓
停止看门狗并删除 managed-app.json / lease / status / lock
        ↓
按现有确认流程卸载 Codex
        ↓
按所有权标记卸载 CCSwitch
```

要求：

- 看门狗停止失败时显示路径和重试按钮；
- 不删除其他 CCSwitch 的共享数据；
- 不因为 U 盘已经拔出而拒绝卸载；
- 修复功能可以重新读取已安装 AppX 身份并重建记录/任务；
- 任务已经不存在时，修复/卸载视为幂等成功。

## 10. 误报和免费发布策略

本改造不承诺消除杀毒软件报警。没有付费代码签名时，未签名 EXE 的 SmartScreen 提示仍可能出现；微软说明，新文件和未签名文件需要单独积累信誉。[SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)

在免费条件下按以下顺序降低行为误报：

1. 看门狗改成独立可执行文件，不复制安装器自身；
2. 不使用混淆、加壳、伪装系统文件或隐藏任务；
3. 任务名、安装目录、日志和卸载入口透明；
4. 用 AppX 身份、路径、PID、创建时间联合校验，禁止全局进程名杀进程；
5. 尽量用原生 Windows API，减少 `tasklist`、`taskkill` 和临时 PowerShell 子进程；
6. 固定 GitHub Release、SHA256、源代码和构建说明；
7. 出现误报时提交文件、哈希、行为说明和误报名称给 Microsoft，而不是指导用户关闭 Defender。[Microsoft 文件提交入口](https://learn.microsoft.com/en-us/defender-office-365/submissions-submit-files-to-microsoft)

这些措施可以降低误报概率，但不能保证未签名、从 U 盘运行、持续监控并结束进程的程序完全不被报警。告警本身是可接受的产品代价时，应保留清晰的发布说明和用户确认，不应以安全软件失效为验收标准。

## 11. 代码改造清单

### 11.1 新增模块

- `src-tauri/src/app/appx_process.rs`
  - `ProcessSnapshot`、`ManagedAppIdentity`；
  - Windows FFI：进程枚举、包身份、路径和创建时间；
  - `matches_managed_app` 纯匹配函数；
  - Windows 之外返回 `UnsupportedPlatform`，纯函数仍可测试。

### 11.2 修改模块

- `src-tauri/src/app/usb_guard.rs`
  - 增加 `managed-app.json`；
  - 将长期安装记录与会话租约拆分；
  - 增加 P2 任务创建/删除、登录看门狗和直接启动扫描；
  - 统一二次校验和进程关闭；
  - 保留 P1 `manager-only` 降级。
- `src-tauri/src/commands.rs`
  - 安装/接管/更新成功后保存 AppX 身份；
  - 增加 `set_start_menu_protection`、`repair_usb_guard`、`appx_guard_status`；
  - 所有受保护操作复用现有 U 盘授权门禁。
- `src-tauri/src/lib.rs`
  - 启动时根据 `managed-app.json` 恢复 P2 看门狗；
  - 管理器退出时按保护模式决定是否停止 helper；
  - 卸载/退出路径保持幂等。
- `src/shared/types.ts`、`src/services/managerApi.ts`
  - 增加 AppX 身份、保护模式、任务状态和受管进程字段。
- `src/app/views/OfflineHome.tsx`、`src/app/i18n.tsx`、`src/app/styles.css`
  - 增加开始菜单保护开关、确认弹窗、包身份详情和故障提示。
- `tools/build-oneclick-usb-layout.ps1`
  - `usb-manifest.json` 增加 P2 策略和默认关闭状态；
  - README 增加“同包官方 Codex 无法区分”的说明。

### 11.3 Windows 构建依赖

需要在 `windows-sys` 中按实际 FFI 增加最小 feature，优先使用 Kernel32/AppModel/Toolhelp 接口。不得因为加入 P2 而引入常驻服务框架、驱动或 DLL 注入组件。

## 12. 测试计划

### 12.1 纯逻辑单测

- 包名、FamilyName、FullName、PublisherId 匹配；
- Windows 路径大小写、尾部斜杠和路径越界；
- 包安装路径外的同名 EXE 被拒绝；
- 无 AppX 身份的同名进程被拒绝；
- 允许的 `ChatGPT.exe`/`Codex.exe` 与未知入口区分；
- PID 相同但创建时间不同被视为不同进程；
- 版本升级必须经过重新绑定；
- `manager-only`、`start-menu`、`exclusive` 三种模式；
- 任务状态缺失时进入 degraded，不自动执行终止；
- 5 秒状态机与盘符变化。

### 12.2 Windows 集成测试

在 Windows 10 22H2 和 Windows 11 空白用户环境分别验证：

| 场景 | 预期 |
| --- | --- |
| U 盘在，开始菜单启动 Codex | 允许运行，记录 AppX/路径/PID/创建时间 |
| U 盘不在，开始菜单启动 Codex | 看到受控提示，约 5 秒内关闭匹配进程 |
| 管理器关闭后再从开始菜单启动 | P2 开启时仍受保护 |
| 5 秒内重新插回同一 U 盘 | 清除宽限，不关闭进程 |
| 更换盘符 | 不误判 |
| 换另一块 U 盘 | mismatch，约 5 秒后关闭 |
| 同名但无 AppX 身份的普通程序 | 不关闭 |
| PID 被复用 | 不关闭新进程 |
| Codex 正在更新 | 不误杀更新事务；更新后重新记录身份 |
| 看门狗任务被用户停止 | UI 显示 degraded，不能假装 authorized |
| 卸载 | 删除任务、配置和 helper，不删除无标记的其他 CCSwitch 数据 |
| 睡眠/唤醒、用户切换 | 状态恢复，计时使用单调时钟 |

### 12.3 发布与误报检查

- `cargo test --manifest-path src-tauri/Cargo.toml`；
- `cargo check --target x86_64-pc-windows-msvc --features custom-protocol`；
- `npm run check`、`npm run lint`、前端测试、`npm run build`；
- `verify-usb.ps1 -SkipNetwork`；
- 记录 `Get-AuthenticodeSignature` 状态，但不把未签名状态伪装成已签名；
- Windows 安全中心保护历史记录中记录具体威胁名、文件路径和哈希；
- 不通过关闭 Defender 或添加排除项作为测试通过条件。

## 13. 分阶段实施顺序

### P2.1：模型和只读探测

- 新增 `appx_process.rs`；
- 实现 Windows 条件编译和纯匹配函数；
- 安装后保存 `managed-app.json`；
- 增加只读诊断命令，不改变终止行为。

### P2.2：持久看门狗

- 拆分安装记录和会话租约；
- 创建/删除用户级登录任务；
- 管理器退出后恢复 helper；
- UI 增加开关和确认。

### P2.3：直接启动保护

- 快照扫描和 AppX 身份匹配；
- 5 秒宽限、二次校验和精确关闭；
- 加入 PID 重用、盘符变化和睡眠唤醒测试。

### P2.4：发布加固

- 独立 helper 二进制；
- 原生进程 API 替代外部 `tasklist`/`taskkill`；
- 更新/卸载/修复完整回归；
- 提交误报样本并整理现场排障手册。

## 14. 验收标准

P2 只有在以下条件全部满足后，才可以标记为可选发布功能：

1. 开始菜单启动的 Codex 能被 AppX 身份、路径、PID 和创建时间同时验证；
2. 未插入绑定 U 盘时，P2 受管 Codex 在 5 秒宽限后退出；
3. PID 重用、未知包身份、路径越界和普通同名程序不会被关闭；
4. 管理器退出后，已明确开启 P2 的看门狗仍能工作；
5. 用户可以在 UI 中看到、关闭、修复和卸载这项功能；
6. 卸载不会全局删除其他 CCSwitch/Codex 数据；
7. Windows 10/11 实机通过开始菜单、拔盘、换盘符、睡眠/唤醒和升级测试；
8. 文档明确说明未签名构建可能触发 SmartScreen/杀毒软件提示，且没有以关闭安全软件作为验收条件。

## 15. 最终决策

当前项目下一步应实现“AppX 身份 + 路径 + PID + 进程创建时间”的精确保护，不应退化为全局进程名杀进程。

实现顺序上，先做只读身份探测和诊断，再启用持久看门狗，最后接入终止动作。这样即使 Windows API、包身份或用户环境不符合预期，也会进入 `unknown/degraded` 状态而不是误杀。

在免费发布前提下，未签名警告可以作为已知成本接受；但应通过独立 helper、透明任务、精确进程范围和公开误报申诉流程，尽量减少被判定为自复制/隐蔽控制程序的行为特征。

