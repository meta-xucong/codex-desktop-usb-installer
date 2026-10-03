# Codex USB 本地代码收尾：并发写入与认证绑定审查

状态：REVIEW_WITH_OPEN_FINDING_NOT_ACCEPTED。不是完整代码验收或独立审计通过。
依据：CODEX-3P-COMPAT-v1/code-only；本轮不运行编译、类型检查、测试、安装、打包或真实网关调用。
审查证据目录：.codex-audit/local-t1-t6-20260924/final-closure-20260924T143514Z/。
本轮读取的主仓库 HEAD 为 e9514d08c343f594b0e452617bdf4a45bc0216b0；CCSwitch 为 1d44b1ba41987cddeb8096fadba8190fab465cbb。
起始基线：baseline.json，root 888 个文件、ccswitch 896 个文件。
旧编辑终端 44412 已终止并核对不存在；随后发现终端 73812 正在继续修改同一项目，未接管或终止该后续执行链。
第一次快照因四个源码文件并发变化而失效，review-v1/SNAPSHOT_INVALID.txt 已明确禁止用它作审计依据。
review-v2 成功捕获 37 个源码文件，捕获完成时哈希无漂移；后续实际工作区继续变化，不能用旧快照批准新的工作区。
本轮没有直接修改产品源码；保留现有唯一写入者，未覆盖其模型目录、原生探针或 CCSwitch 改动。

## 本轮源码发现 SOURCE-AUTH-01（需修复）
model_catalog_activation.rs 中 LiveCatalog 记录 config/catalog/runtime 哈希，但没有记录实际 auth.json。
current_catalog_is_verified() 只比较许可证绑定对象和上述文件；auth.json 单独变化时，旧原生证据仍可能被视为适用于当前认证上下文。
最小修复：成功应用后核对实际 OPENAI_API_KEY 与合法绑定一致，在内存记录 auth.json 字节哈希；之后文件变化/读取失败均使目录验证失效。不得修改认证文件、输出凭据或添加认证来源。
设计问题 DQ-LOCAL-AUTH-001 及详细解释位于 REVIEW-HANDOFF.md；这是既有账号变化隔离要求的内部实现补齐，不改变公开 schema。
补丁：proposed-auth-binding/source-auth-01.patch；完整候选：model_catalog_activation.rs.proposed；元数据：metadata.json。
补丁基于目标源码 SHA256 5c9b9f0ef05db7a316e646d6e880b116c959710cb2fbc571ff6de4644aa94d70，共 78 行差异，包含合成认证反例测试源码。
补丁状态 PROPOSED_NOT_APPLIED；由于产品文件仍有其他执行链写入，未直接应用。应用前必须重新检查基线或逐段合并。
仅对候选源码调用 rustfmt stdin 解析，退出码 0；不是编译或测试通过。测试 NOT_RUN_BY_USER_REQUEST，退出码 N/A。

## 独立审查实际结果
原生审查线程 01a0d3db-aa42-70c1-bab3-e9d3959cd2ae，终端 32220；集成审查线程 01a0d3dc-77ae-7840-a3d3-39f5b9afdc0b，终端 71920。
两条 CLI 均请求 Luna/max、read-only；均以退出码 1、turn.failed 结束，错误为 usage limit；没有有效终态审计结论，不以作者判断替代。
它们是实际独立 CLI 会话，不是原生 spawn 派发，没有伪造 MAD_ROUTE_RECEIPT；没有更换账户或绕过额度限制。
下一交接条件：确认本项目唯一写入负责人；其在固定版本中应用/处理 SOURCE-AUTH-01，再对最终快照完成有效独立静态审计。
