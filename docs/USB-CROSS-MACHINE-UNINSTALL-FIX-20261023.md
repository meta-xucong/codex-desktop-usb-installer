# 跨电脑 USB 卸载归属修复

## 日志与原因

`codex-app-manager(15).log` 中，USB guard 已为 authorized / running / signed，联合卸载却返回 `external-not-managed`。其后“关闭进程重试”是剩余程序检测产生的后续提示，不是此次失败的首因。

`win_update.rs` 的检测和状态查询接受当前 USB 的 `apps/Codex`，不要求本机 provenance。卸载原先只接受本机准确路径及版本记录，或联合卸载的 current-user MSIX 特例。USB 转移到另一台电脑，或盘符变化后，没有匹配的本机记录，便触发该矛盾。

盘上已有程序时显示已安装是既有便携设计；这不等于新电脑的账户和能力包已初始化。原有初始化在安装流程执行，本次不改成每次启动覆盖本机配置。

## 最小修复

产品代码仅修改 `src-tauri/src/app/win_update.rs`：

1. 保留准确版本 provenance 的原有通过路径。
2. 仅联合 USB 卸载、且 portable 未被本机 provenance 认可时，启用补充归属判断。
3. 首先检查候选为当前 bundle 的 portable 路径，再复用 `require_signed_codex_bundle("codex-uninstall-destination")` 核验签名、授权租约、bundle 完整性及目录非 reparse 边界。必须与返回的 `codex_root` 精确匹配。
4. 缺少授权、授权失败或不同目录不通过。普通管理器及 MSIX 策略保持原样。
5. 通过后复用既有 portable 卸载、进程关闭、元数据清理及后置检查，不建立第二套删除逻辑。新增日志标明“无本机记录、由当前 USB 授权”。

没有修改安装、启动、模型、MCP、账户注入、签名器或前端代码；没有写入伪造的本机安装记录。已有工作区其他任务的改动保留。本次不打包或覆盖正式程序。

## 代码审计范围

- 显式确认和操作锁在既有上层入口保持有效。
- 新分支限于明确的联合卸载，不能由外部路径字符串直接授予删除权限。
- 生产授权调用与用于测试的回调分离；测试回调不进入正式入口。
- 校验错误传播，未把错误转为授权成功。
- 原有托管构建短路通过，不增加该路径的签名核验或网络请求。
- MSIX、旧版便携目录、用户数据选项和最终卸载后置条件未修改。
- 新分支只修复归属；不宣称解决所有文件占用、设备拔出等独立失败。

## 回归设计

- 隔离临时目录含可识别的 Codex fixture，本机记录为空，当前 USB 授权后可通过；判断自身不删除程序或新增本机记录。
- 当前盘符变化后接受新的精确根目录；旧盘符、另一 bundle、相似前缀及父目录拒绝。
- 普通管理器、MSIX、未知来源不调用补充 USB 授权。
- 无授权、签名失败、看门狗失效、拔盘及 reparse 拒绝信号保留为失败。
- 运行现有 Windows 更新/卸载测试，以及首页和卸载前端测试，检查旧路径回归。

测试临时文件和编译输出使用 D 盘。未对实体 USB 或当前使用中的 Codex 执行卸载。

## 执行结果

本轮代码复核通过：新增授权只用于既有联合卸载入口，读取的目标来自后端已验证的 bundle；错误不降级放行，原有本机准确版本记录和 MSIX 分支未回退。此次为本会话源码审计，未声称另一个独立审计者复核。

实际执行：

- Windows Rust 测试库重新编译成功；新增 `cross_machine_usb_uninstall` 4 项通过。
- `cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::win_update::tests -- --test-threads=1`：51 项通过，包含上述 4 项，以及原有安装检测、启动目标、更新和卸载结果测试。
- `npm.cmd test -- src/app/views/OfflineHome.test.tsx src/app/views/Uninstall.test.tsx --pool=threads --maxWorkers=1`：2 个文件、76 项通过。
- 合计 127 项不同测试通过。首次前端默认 forks 运行发生 worker 启动超时，未执行用例；上述单线程池重跑完整通过，没有改产品或测试配置规避断言。
- 修改文件 `rustfmt --check --edition 2021` 与 `git diff --check` 通过。

未执行真实签名 U 盘卸载、生产程序替换、正式打包或 GitHub 推送。真实跨电脑签名 U 盘卸载仍需在测试机使用包含本修复的新 EXE 验证；不能用隔离回调测试代替实体操作。当前正在运行的程序不会因为本次源码修复而改变。

## 后续打包交付（2026-10-23，用户另行授权）

已生成 `release/usb-oneclick-v0.5.24-20261023-cross-machine-fix`，管理器、启动器和 shim 来自本次正式构建。随包独立 verifier、公钥编译绑定、MSIX 哈希和签名、PowerShell 解析、能力包清单及签名器配套检查通过。管理器二进制包含新增跨电脑卸载分支的日志标记，三个 EXE 与本次 setup 构建目录哈希一致。交付目录没有 Python 字节码、单个 U 盘的 license 或签名器凭据。

补充发布检查：TypeScript、ESLint、前端 438 项、安装链 10 项通过；adapter 58 项在包内离线 wheel 建立的 D 盘隔离环境中全部通过。首次 adapter 用系统 Python 运行时 PDF 测试缺少 pdfplumber，改用隔离环境配齐包内依赖后通过；未修改产品或测试断言。

现有 `release/usb-license-signer-gui-v0.5.24-20261021` 继续配套，无需重打。7 个文件校验通过，清除环境变量覆盖后的 EXE 诊断确认管理员 Key 从内置来源加载；没有回显 Key，也未新建账号。新包与它的 releaseVersion、releaseId、keyId 和公钥摘要全部匹配，portable vault 公共绑定检查通过。

产物哈希及源码修改文件摘要见 `release/DELIVERY-20261023-cross-machine-fix.json`。仍未替换当前安装、操作实体 U 盘、真实签发或推送 GitHub。
