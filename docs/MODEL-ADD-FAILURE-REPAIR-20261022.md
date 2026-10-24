# 2026-10-22 模型添加失败修复方案与记录

## 依据

测试机日志 `codex-app-manager(14).log` 对应本地 `v0.5.24-20261022-log13-prepack-final`：

- `codex_gateway_probe ... stage=models error_code=upstream-connect-failed` 后，另一次目录请求成功。
- 添加阶段 `stage=runtime-executable-hash code=native-probe-timeout elapsed_ms=125571`。
- 同一日志确认 Codex 安装成功、能力包 v1.5.4 安装成功、CCSwitch 选中并加载 404token 凭证。Product Design 在测试机上能用，能力包成功记录与本次模型添加失败无关。

## 根因

1. **模型列表连接失败是一次 GET 请求的瞬时连接失败。** 模型管理页的 `gateway_model_probe::run` 对 `/models` 只发一次请求；该错误没有 HTTP 响应，日志不能证明 404token 服务端拒绝请求，也不能单凭此日志判断 Windows 代理/PAC 状态。后续独立请求成功，说明至少后一次路径恢复可用。
2. **模型添加预算被错误地共用。** `add_tested_model` 原先最多只有 270 秒，且在发出 30 秒超时的目录重核之前就开始倒计时。证明票据要求剩余至少 190 秒，保留 10 秒提交余量；前置目录重核消耗的时间直接从 USB 文件哈希预算扣除。大体积 Codex CLI 位于慢速/受防病毒扫描影响的 U 盘，约 125 秒即走到统一 deadline，触发 `native-probe-timeout`。这是本地验证超时，不是上游 Kimi/Claude 等模型不可用；失败在配置写入前。

## 约束与修复

- `/models` 是幂等只读 GET；最多进行三次短暂尝试，只对连接、网络、超时和服务端 HTTP 错误重试；认证失败、输入错误、取消、响应格式错误不重试。每次等待有界并遵守取消与总 deadline。模型生成请求/费用探测不自动重发。
- 模型添加将目录重核和本地可信验证分成两个有界阶段。每次添加的目录重核最多 100 秒；成功后根据仍有效的单次测试票据计算最多 10 分钟本地校验预算。票据 TTL 提升到 20 分钟，并要求添加开始时剩余至少 710 秒，确保最坏目录重核后仍有充分的 USB 校验时间。提交前继续校验绑定、目录摘要和票据有效性。
- 不跳过 USB CLI 哈希、MSIX Authenticode、文件固定/提取、Codex 原生目录验证、取消检查或原子提交；不复用过期测试票据。不提前读取或缓存可变安装文件的哈希。
- 诊断保持不含 API Key；连接失败仍明确显示网关目录不可用且未修改模型目录，可在网络恢复后重新核对。

## 验收

1. 自动目录重试：连接型故障后下一次成功可继续；认证/取消等永久错误不重试；达到总预算后以原错误返回。
2. 添加预算：即使目录重核耗时也不挤占本地校验窗口；证明过期/目录变化/绑定变化仍拒绝提交；本地验证失败不写模型目录。
3. 保留原有可信验证、Reset/Add 互斥、模型累加和 restart 标记测试。
4. Rust 目标测试、前端模型管理相关测试、格式检查、diff 检查通过。实机慢速 U 盘吞吐/病毒扫描性能不由单元测试伪称通过。

## 实施结果

实施：`gateway_model_probe.rs` 为模型目录 GET 增加三次有界重试；将添加的目录复核 deadline 与本地验证 deadline 分开；把本地哈希预算增至 10 分钟并扩展绑定票据有效期/起始余量。没有改签名校验、模型探测请求或 Product Design 内容。

代码级检查：

- 新票据剩余时间必须覆盖 100 秒目录复核 + 600 秒本地校验 + 10 秒提交余量。
- 目录重试仅针对 GET 的 Connect/Network/Timeout/HTTP server error；401/403、400、取消和无效响应不重试。
- Add 模型写盘仍仅发生在原有本机默认目录验证、绑定重验和票据消费步骤之后。

测试：直接调用 Rust 1.95 工具链目录（不使用失效的 cargo shim）完成 `rustfmt --edition 2021`；定向 `gateway_model_probe` 21 项、`model_catalog` 63 项、`model_native_probe` 22 项通过；串行运行全库测试并跳过一个需要外网的既有更新回退用例：763 passed、0 failed、6 ignored、1 filtered。该外网用例在隔离环境中等待时被中止；没有把它记为通过。并行全库首跑有一项既有 USB 撤销测试波动，单独重跑通过，串行全库通过。前端 `UpstreamModelManagerSheet.test.tsx` 39 项通过；`git diff --check` 通过。

复核限制：没有真实慢速 U 盘做 10 分钟压力实测，没有重新打包/实机点击添加，也没有独立子 Agent 审计；这是源码审查与自动测试结论。Product Design 的既有测试机可用记录维持有效；本次没有改插件。

## 发行包续验（2026-10-22）

- Windows SignedRequired 管理器、入口 shim 和启动器已构建；USB 管理器嵌入 `requireAdministrator`，入口 shim 为 `asInvoker`。
- 新 U 盘目录：`release/usb-oneclick-v0.5.24-20261022-model-add-fix-r1`。以此前已校验的离线 MSIX 和能力包 1.5.4 为基线，只更新本次管理器构建；MSIX 为 x64 `26.1002.7124.0`，SHA-256 `f2913fdf027a0924ef42198222e325ac8f6331580cbffc51c7a8d18d8ce73c78`。
- 包内 `verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork -UsbSigningProfilePath <bundle profile>` 通过；`verify-usb-signing-pair.ps1` 与现有 `release/usb-license-signer-gui-v0.5.24-20261021` 配对通过。新管理器 SHA-256 为 `825369850d9b7b9fe570fa37c1ed8121379ce3041f29335707191ecc35cbab87`，与 setup 目录中的构建结果一致。
- 签名工具没有重编：签名协议、密钥绑定和注册流程未变；其内置管理员 Key 与 portable vault 仍在 package manifest 声明，pair preflight 确认与新 U 盘的 keyId/releaseId 匹配。
- 该验收是完整发行目录和签名器配对的静态校验，不等于慢速实体 U 盘上的真实模型添加/冷启动。真实 Codex 配置没有被本次构建或验证改写。
