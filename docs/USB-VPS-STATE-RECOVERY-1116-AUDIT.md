# VPS 授权服务修复审计与验收

完成时间：2026-11-16，北京时间。修补已部署至现有 VPS 授权服务；未改客户端安装包、签名器、404token 账号或充值逻辑。

## 确认的根因

1. 原 PID 650366 仍运行，服务实际数据库、主审计、镜像审计路径与源码配置对应。
2. 56 条授权、58 条 SQL 审计与两份 JSONL 审计一致，既有链校验通过，数据库状态/版本合法。无恢复标记、遗留锁或当前磁盘不足证据。
3. 旧程序没有诊断接口暴露原因。本次只读按实际 Go Store 布局、数据库路径及固定原因白名单定位运行状态，得到唯一匹配：`unknown=true`、`license record integrity failed`。未暂停/写入进程、保存内存转储或输出其他内存内容。首轮按常见 Go 堆地址限定未命中，按实际映射定位后命中，未据未命中推断状态。
4. 对应 `LookupForCheck` 分支把有效签名凭证与库绑定不一致，和数据库记录无效混为一类，全局设置 unknown。后续所有登记/检查直接失败。隔离测试以三个有效但冲突的签名凭证重现此结果。
5. 最初触发请求已不在保留日志中；无法指认具体 U 盘或用户操作。确认的是当前服务被锁定的原因和可复现代码缺陷，不推测某次签名或升级触发了它。

## 最小修改审计

- 产品逻辑仅改 `internal/revocation/store.go`、`http.go`。
- 已验证的绑定冲突返回 unsigned HTTP 409，只拒绝当前请求，不标记整库 unknown，不覆盖旧绑定。
- 真正的数据不一致、恢复标记及原有异常阻断不变；双审计完整性、撤销状态与版本约束不变。
- 日志新增固定原因分类，可区分凭证冲突、未登记与已知存储异常。不记录凭证、摘要、密钥、SQL 或请求正文。
- `/healthz` 在 Store 已知 unknown 时返回 503，仅公开 `status=unavailable`。健康 200 表示当前没有已知异常，并不替代业务请求原有的一致性检查。
- 同步 README、PROTOCOL、OpenAPI；新增一份定向回归文件。未新增重试、自动改库或降级放行。
- 检查失败日志和公开错误响应的实际消费者；客户端仍只接受成功的签名响应，409 不伪造授权状态。管理器原离线策略未改。

## 回归证据

Windows、Linux 均通过 **50 个顶层测试、含子用例 98 项**；`go vet ./...` 与差异格式检查通过。生产服务为同次构建的 Linux amd64 候选。

- 修补前：许可证摘要、指纹、key ID 三种冲突均返回 503 并令整个 Store 不健康；恢复标记和库不一致两种健康检查均错误返回 200。五项预期失败确认复现。
- 修补后：三种冲突返回 409，不返回状态签名；其他 active/revoked 授权仍可检查，新登记正常；原授权与审计无修改。
- 未登记仍返回原协议 503，日志明确 `license_not_registered`，不污染全局状态。
- 真正不一致与恢复标记仍阻断，健康 503；日志记录原因，公开响应不泄露内部信息。
- 原有注册幂等、禁止注册复活已撤销许可、失败事务、审计回放/镜像不一致、管理员鉴权/TOTP/CSRF、模型目录管理等全量回归通过。

首轮环境失败均保留而未当作产品故障：Go 标准库识别/外部缓存访问问题后改用现有 Go 1.26.4 和任务内缓存；Windows 沙箱阻止三个回环 HTTP 测试，宿主同套测试通过；Linux 首轮缺少两个相对路径测试夹具，补齐现有夹具及正确工作目录后全量通过。没有为测试修改产品权限或绕过安全机制。

证据目录：`tmp/vps-state-recovery-1116/`，包括 Windows JSONL、Linux 测试日志、`deployment-result.json`、`public-smoke-result.json`、`post-deployment-safe.json`。部署脚本为一次性操作记录，不是产品新增恢复机制。

## 实际部署与保留性检查

- 2026-11-16 01:54:20 启动新服务，PID 1279212；旧 PID 650366。
- 部署前确认服务没有活动子任务。备份旧二进制、SQLite 和双份审计，只替换 `revocationd`，正常重启并重新执行原有完整一致性校验。记录的停止到健康时间约 0.663 秒。
- 备份保留于 VPS：`<private deployment backup>`。备份目录权限 0700，未导出生产业务数据。
- 上线前后逐字段比较：**56 条授权、58 条审计 SQL 记录、双份审计文件全部未变**。未改状态、版本、私钥、配置或签名工具，未重新登记、签名或充值。
- 正式 HTTPS 域名直接连接：health 200 → 公开测试密钥的只读 check 被 400 拒绝且无签名状态 → health 仍 200。没有走 register。
- 生产日志确认新增原因字段实际生效：`proof_rejected` / `invalid_registration`；该窗口没有新的全局 state_unknown。检查后授权与审计数量仍为 56/58。


## Publication scope

Customer-specific registration records, production backups, credentials and diagnostic dumps are not included in this repository. This source update does not register, re-sign or recharge any USB account.
