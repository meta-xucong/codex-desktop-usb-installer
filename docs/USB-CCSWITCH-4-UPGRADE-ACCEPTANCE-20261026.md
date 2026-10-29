# USB CC Switch 4.0.20 升级与验收边界

## 目标和基线

升级 USB 产品集成，而不是把官方 EXE 直接换进包内。官方稳定版本为 `v4.0.20`，固定提交 `85ad4969b3399500baaea9d46b1e08f87aba80e1`。注解标签对象 `79738d9bf9fe846f698fe3586ba5b7b75309649c` 与提交不同是正常的，须比较 `v4.0.20^{}`。

安装器基线为 `1d8966500faca64466a3fe427b5a69bbaac0cb63`。本地最新包包含其他未提交修改，因此将本分支合入本地时必须保留原有修改，不能以此分支整树覆盖本地或宣称旧管理器 EXE 已被精确重建。

## 上游证据与适用范围

- 维护者解释旧 3.x 的整份 Provider 快照写入设计：[issue 5770](https://github.com/farion1231/cc-switch/issues/5770#issuecomment-5131641705)。
- Windows bundled 插件及动态配置被旧快照影响的报告：[issue 4683](https://github.com/farion1231/cc-switch/issues/4683)。此报告的关闭本身不是官方修复证明。
- 维护者确认 4.x 改为关键字段写入：[issue 5886](https://github.com/farion1231/cc-switch/issues/5886#issuecomment-6091696965)。
- 官方 Codex 写入机制迁移提交：[0e6430ab](https://github.com/farion1231/cc-switch/commit/0e6430abdb2255a36956f3cf8fb440fae4ce460e)。
- [PR 4940](https://github.com/farion1231/cc-switch/pull/4940#issuecomment-6092153907) 已关闭且未合并，不能将其中方案当作已发布实现。相关审查揭示了旧模型参数串入与删除配置被旧备份复活的风险。

这些资料和本仓代码可以证明旧机制的条件性配置保留缺口，不能证明测试机 08:46 那次修改的实际写入 PID。升级验收也不能替代该历史事件的取证。

## 已确定的产品要求

1. 使用官方 4.x 字段写入机制；USB bootstrap 和模型选择不得绕回旧全量配置覆盖函数。
2. 保留签名授权环境注入、nonce ACK、精确回读、schemaVersion 3 模型选择协议及既有绑定/版本/目录校验。
3. GUI 和模型选择子进程必须使用同一 USB Codex home 与独立的 CC Switch v4 状态目录。
4. 不参与独立安装的单实例 IPC 或共享深链接，不自动导入、修改其他应用的配置。
5. 迁移范围为完整 USB 历史数据，而非仅默认账号。旧共享数据库只读复制，新版迁移隔离副本；不删除或原地升级原库。
6. 完整保存旧数据不等于重新执行旧行为。旧恢复日志、接管快照、路径覆盖与云同步状态须安全处理，不能将旧 Provider/common 快照重新覆盖到 live 配置。
7. 现有目标不得被重试覆盖；迁移中断、备份失败、损坏源库或不明确源目录不能伪装成成功。
8. 官方迁移可能重建 Codex 派生用量统计。原始历史的保留、归档与新版可见统计的重建须分别验收，不能把归档存在等同于界面已完整恢复。

## 验收层次

以下层次必须分别记录，不能混报：

- 云端静态审计、TypeScript、前端测试和 renderer 构建。
- 原样生产 Rust 模块/方法在隔离夹具中的测试；明确应用外壳及平台分支未覆盖范围。
- Windows 固定工具链 `cargo check --locked` 与 `cargo check --locked --tests`。
- Windows 合成 profile 的真实 bootstrap、重复启动、模型 apply、失败回滚、旧库迁移测试，确保不访问个人真实数据或供应商 API。
- production `--no-bundle` EXE 构建；源码补丁、清单、EXE 与两份整包副本一致性检查。
- 管理器重建（包含模型选择子进程的 USB 环境接线）、原有签名/profile 流程与测试机最终验收。

重点断言：插件显式禁用不变；插件内 MCP 策略、媒体及用户 MCP、全局权限保持；重复启动不替换管理器受管模型目录；旧快照不复活已删除项；迁移保留完整管理数据且不污染独立安装；无隐式模型选择；失败不留下半升级或假成功状态。

## 交付状态约束

当前仓库仍保留旧 EXE，清单须明确 `pending-rebuild`。源码、抽取测试或静态标记通过都不等于新 EXE 已完成。

特殊 USB 模型选择事务采用字段补丁与既有 CAS，不应宣称完整具备官方普通切换路径的多文件崩溃恢复能力。此边界须在最终报告中保留。

真实用户数据迁移、测试机启动验证、签名发行和部署不是云端源代码修改的自动副作用，必须在实际执行并确认后逐项记录。

## 本轮已取得的证据

当前完整产品补丁 SHA256：`69B135C95A04F06CDF8ED2002E0D3224E43219AF5A101299A786A0A7966210CA`。

- 前端 50 个测试文件共 1,237 项通过；TypeScript、renderer build 通过。
- 精简后原样生产迁移/导入/settings 方法的隔离 Rust 测试 20 项，以及生产路径函数 3 项通过。不是完整 Tauri 初始化测试。
- 模型 CLI/官方 TOML 字段写入的隔离 Rust 测试 24 项通过；Windows 专属分支未据此验收。
- USB 启动隔离策略 2 项、管理器子进程环境接线 1 项独立测试通过。各夹具边界须保留，不将数量简单相加冒充完整套件。
- 打包静态契约 7 项通过。
- 对全新固定官方源码，补丁检查、应用成功，49 个变更文件与审计工作树逐字节一致。
- Windows 官方基线 renderer 与 `cargo check --locked` 通过；此前 C3 候选亦通过 `cargo check --locked` 和 `cargo check --locked --tests`，但没有执行原生测试，不能代替本次精简候选验收。使用已有 Rust 1.95、MSVC、任务内 pnpm 10.12.3；Node 24.14.1 与仓库指定 22.12.0 不同，已明确记录。

**2026-10-26 本次首次绑定修复验证：** `cargo check --locked --tests` 通过；`usb_model_selection::tests` 17/17 通过。`usb_404token_bootstrap_` 共 10 项（3 passed、7 因 Windows sandbox 临时 profile 原子替换返回 `PermissionDenied` 未能完成）；`usb_upgrade_` 共 23 项（17 passed、6 因 sandbox 临时目录访问返回 `PermissionDenied` 未能完成）。拒绝后未重试或绕过。请本地 Codex 在权限可用的 Windows 环境复跑两组受阻测试；受阻项不计通过。此前 46/46 是更早候选的历史结果，不代表当前修复通过全套测试。

仍未执行：最终 EXE/管理器重建、完整签名包及 MSI-15 实机验收。上述原生测试不能替代这些交付步骤。

源码交接仅能表述为候选实现及以上有限验证完成，不得表述为故障已在测试机修复。

## 按用户要求精简的范围

已撤回新增的直接启动限制/提示框、容量倍率预检、重复 USB 专属迁移备份门禁、来源路径必须逐字一致的恢复约束，以及新增历史采集路径硬限制。迁移采用原库只读一致快照与隔离副本，沿用官方升级行为；不覆盖现有目标和中断复制冲突处理仍保留。无效 UI 设置保留原始归档，跳过可选偏好，不阻断有效数据库迁移。

保留的 Windows 目标文件 flush 修复属于实际平台兼容问题，不是新增门禁：[Microsoft FlushFileBuffers 文档](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)要求可写句柄。仅打开新目标文件为可写、不截断，原始文件仍只读。

本地任务曾因恢复接口失败中断，之后已恢复并完成上述精简候选原生测试。完整 GitHub PR 仍未创建：三个旧打包脚本的上传被自动审查拒绝，未绕过限制。获准上游源码对象的读取和测试是独立操作，不代表完整发布。

职责按用户最新要求：Dot 负责全部开发、修复及自己可完成的测试；接手的本地 Codex 仅补环境受限测试、范围审计与最终 EXE 打包。遇代码失败或语义合并冲突，返回证据，由 Dot 修改，不能将开发转交本地。不要新增功能或校验。

本次命令为 `cargo check --locked --tests`，以及对 `usb_404token_bootstrap_`、`usb_model_selection::tests`、`usb_upgrade_` 分别执行 `cargo test --locked --lib <filter> -- --test-threads=1`。本次首次绑定修复已合入本地完整产品 overlay，并重新导出、核对 SHA256。详细结果与交接边界见 `docs/USB-CCSWITCH-BOOTSTRAP-LOCAL-HANDOFF-20261026.md`。

## 本地交付审计修订（2026-10-26）

移除 `tools/build-ccswitch-with-env-patch.ps1` 对新增 `ccswitch-upgrade-contract.test.mjs` 的强制调用及失败阻断。该测试只保留为开发者可选工具，不接入构建门禁；本地按原有构建和验证流程复测即可。本次修复已更新上游产品 overlay 与 builder 的既有 Rust 测试计数；当前补丁 SHA256 为 `69B135C95A04F06CDF8ED2002E0D3224E43219AF5A101299A786A0A7966210CA`。

当前源码通过 `cargo check --locked --tests` 与 17 项模型选择测试；首次绑定和升级过滤器另有 13 项因 Windows sandbox 权限被拒而未完成。最终 EXE 和 MSI-15 实机验收仍待完成。
