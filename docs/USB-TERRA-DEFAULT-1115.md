# 最新运行时与 Terra 默认可选

## 已定位的调用链

2026-11-15 查询现有官方包镜像 latest，最新为 Codex 26.1111.21308 / MSIX 26.1111.2228.0；上一轮 usb-runtime-compat-1115 已包含这份原始 MSIX。

原生 `codex.exe debug models` 包含 `gpt-6.1-terra`，visibility=list、supported_in_api=true。真实 app-server `model/list` 在全新 HOME、已有 model/profile/desktop/MCP 设置的合成 HOME 中均返回该模型，hidden=false；各重启一次仍在，配置字节不变。未发送 turn/start 或付费请求。

但已有管理器模型目录经 config.toml 的 `model_catalog_json` 替换整个原生列表。用只包含 Sol 的旧目录实际运行 debug models，只返回 Sol：升级本体不会自动补全旧目录。能力包的 model-catalog.json 是模型策略摘要，不能修复这个实际消费者。

## 最小修补

- 仅在现有管理器托管目录启动校验内，补入缺少的 Terra；条目原样来自本次真实随包 CLI 的内置目录，保存为 bundled_gpt_6_1_terra.json。不拼造模型能力或复用其他模型的提示词。
- 复用既有目录归属判断、配置锁、备份、原子写入、重载标记、损坏目录恢复；不创建第二套迁移、缓存或后台探测。
- 不改已有 Terra 条目、不改已有其他模型、不改 model/profile/provider/思考档位，不接管外部自定义目录，不恢复已停用的目录。全新 HOME 仍使用原生列表，不新建覆盖目录。
- 不改 MSIX、CUA/IAB、媒体包、进度、签名工具、备用源或更新空间事务。

## 验收

须覆盖旧托管目录缺 Terra → 启动自动补齐 → 重启幂等；现有 Terra 自定义条目保持；外部或停用目录不触碰；配置及原有条目保持；损坏当前目录从既有备份恢复时补齐；新目录被实际新版 app-server 接受且 Terra 可选。

已有模型目录单元回归和原发布检查通过后，用上一轮已验收的 26.1111.21308 payload 与当前能力包重新发布。原生 CUA/IAB/媒体实测及受限卷证据沿用该相同运行时的上一轮记录，不宣称本轮重复完成。

## 开发机结果（2026-11-16）

- 最新发布核对：北京时间 00:02 再查既有镜像，仍为 26.1111.21308，原始 x64 MSIX 为 26.1111.2228.0、1,057,802,768 字节。
- 模型目录测试 80/80 通过；另启用需要明确隔离目录的生产迁移验收 1/1 通过。
- 新建 HOME、已有模型/profile/desktop/MCP 设置：各启动两次实际 app-server，默认列表中 Terra hidden=false，配置未改变。
- 真实旧托管目录经生产函数迁移后，新 app-server PID 63536、65136 均只返回预期的 Sol 和 Terra，确认实际消费迁移目录，而非忽略目录后退回内置列表。旧 Sol 条目、配置字节和重启后的目录字节全部保持。
- Terra 元数据逐字段等于随包 CLI 内置条目；包括原生工具能力与思考档位，不按通用 API 文档改写 Codex 的原生默认值。
- 代码审计：目录归属、并发锁、备份和重载流程沿用原机制。补入失败在已有有效目录选定后返回，不会将该目录当成损坏文件恢复为较旧备份。模型选择/provider/MCP/CUA/IAB/更新容量/备用源代码未改。
- 未发送模型 turn、付费媒体生成、签名、登记或充值。真实后端列表与元数据验收不替代网关的付费模型响应验证。

首次回归中一个并发写入用例仍断言仅两个旧条目，已调整为保留两条并追加 Terra。真实夹具首次使用 JSON Unicode 转义路径，与管理器原有字面路径序列化不符；另一轮夹具重复转换 CRLF。均仅修正测试夹具，未放宽产品校验；原始失败日志保留，最终结果明确要求实际列表来自迁移目录。

证据：`tmp/usb-terra-default-1115/` 的 latest-release.json、model-catalog-tests-final.log、migration-production-test-final.log、production-migration-acceptance-final.json、model-list-acceptance.json、source-audit.json、own-diff.patch。

## 已完成发布

发布目录：`release/usb-terra-default-1115/`，入口 `CodexDesktopSetup.exe`，需整目录交付。最终构建来源 `release/setup-terra-default-1115/`。正式编译（含 TypeScript 和生产前端构建）、完整包自检、18 个 PowerShell 文件的 Windows PowerShell 5.1 编码/解析检查、CCSwitch 与入口清单校验均通过。

包对比：能力包 548 个文件与 usb-runtime-compat-1115 逐文件一致；CUA v3 helper、CCSwitch、原始 MSIX 一致；编译后的管理器保留九组 26.1111 适配补丁，并嵌入了本次精确验证的 Terra 元数据。manifest 语义一致，usb-manifest 仅 manager 记录随新编译结果变化。四阶段进度、离线能力包、备用源更新、更新空间与回滚流程本轮未改；不把前序的 CUA/IAB/受限容量真实回归称为本轮重测。

签名器无需重打包：沿用 `C:/Users/T14S/Documents/Codex/2026-10-31/task/usb-cua-helper-experiment-20261101/release/usb-license-signer-gui-v0.5.25-preload10-layout-20261106/CodexUsbLicenseSigner.exe`。本包 pair ready=true、portableVaultChecked=true；必须携带该签名器完整目录，未执行实际签名或充值。

新增发布证据：同任务目录的 manager-build.log、package.log、package-provenance.json、signing-pair-result.json。授权范围仅为用户批准的既有发布校验流程，未新增或绕过完整性机制。
