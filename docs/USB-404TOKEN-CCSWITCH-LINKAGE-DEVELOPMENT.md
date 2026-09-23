# U 盘账号、404token 与 CCSwitch 联动方案

版本：G39.3 总方案（已同步一键注册细分规范）  
状态：签名器一键注册代码已落地；新建 API Key 固定使用 `unified-api-internal` 分组且每个账号只生成一个 Key；管理员 Key 支持运行时环境变量、生产 EXE 编译时嵌入值和签名器同目录配置自动加载；管理器已接入已签名 `accountBinding` 的后端消费、内存账户会话和 CCSwitch 子进程环境注入；CCSwitch 运行时读取补丁已完成构建、替换资源、网络门禁和正式发行包回归；未修改 VPS。真实 U 盘账户激活仍需实际 bundle 与 404TOKEN 管理凭据。

局部最终规范：[USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md](./USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md)

## 1. 目标

为每个实体 U 盘固定绑定一个 404token 账号，并完成以下体验：

1. 签名工具签名时自动注册或匹配对应账号；
2. 自动生成该 U 盘专属的 CCSwitch 设备 API Key；
3. 用户在安装器中不再手工输入 CCSwitch API Key；
4. 安装器显示该账号的余额、用量和充值入口；
5. 用户只需在账号信息页输入一次账号密码；
6. 现有 U 盘签名、安装、更新、Skill Hub 和卸载流程不被破坏。

## 2. 最小化原则

本方案只增加一条“账号绑定 + 设备凭证”链路，不把四个组件互相改造成业务系统：

- 404token/Sub2API 后台：唯一的账号、余额、用量和充值数据源；
- 签名工具：受信任的生产期注册者、管理员 API 调用者和授权信息写入者；
- 安装器：授权校验、账号页面和 CCSwitch 启动注入器；
- CCSwitch：只增加一个稳定的设备 API Key 接入口，不承载账号管理。

不新增二维码登录、设备码登录、支付系统、离线余额、第二套账户数据库或复杂同步器。

## 3. 关键决策

### 3.1 一个 U 盘对应一个账号

本期使用签名工具自动顺延的业务编号作为 `usbAssetId`，由 Sub2API 中保留的 `USB-*` 用户记录作为序号来源：

```text
usbAssetId: USB-0001
accountRef: USB-0001
accountName: USB-0001
loginEmail: u1@u.404token.xyz
```

签名工具查询现有 `USB-*` 用户，取最大数字加一；创建时发生冲突则重新查询并顺延重试。删除用户不回收编号。U 盘硬件序列号、Windows 卷序列号、Bundle Anchor 和 Payload 摘要仍由现有 USB 授权流程作为设备/内容指纹，不作为账号编号主键。

Sub2API 当前创建用户接口强制要求邮箱，因此账号使用短模拟邮箱 `u<编号>@u.404token.xyz`。邮箱只用于后台登录和唯一性校验，安装器默认只显示 `accountName`。

账号绑定写入已签名的 `usb-license.json`，普通用户界面不提供重新绑定入口。只有后台管理员可以冻结、换绑或重新发放凭证。

### 3.2 CCSwitch 使用用户 API Key

本期不新增后台 Provisioning 接口，签名工具通过管理员 API Key 创建普通用户，再用该用户 JWT 调用现有用户 API Key 接口。写入许可证的 `deviceApiKey` 实际是该用户的 Sub2API API Key：

- 只属于一个账号和一个 U 盘；
- 通过固定的 CCSwitch 服务分组限制可用线路；
- 可在后台按用户或 API Key 单独停用、删除和轮换；
- 不具备管理员创建用户、调整余额或管理其他用户的权限。

为了实现跨电脑免配置，该 Key 随签名授权信息写入 U 盘。管理员 API Key 只存在于签名工具，绝不写入 U 盘。

这是本期对“安全性”和“免配置”的明确取舍。若未来需要防止签名工具持有人提取管理员 Key，再升级为 VPS 端窄权限 Provisioning 接口，不纳入本期。

### 3.3 不修改 CCSwitch 的账号页面

安装器负责显示 404token 账号信息和充值功能；CCSwitch 只接收运行时设备 Key。

优先采用环境变量注入，避免把 Key 再复制到用户磁盘：

```text
CCSWITCH_404TOKEN_DEVICE_KEY=<device api key>
CCSWITCH_404TOKEN_ACCOUNT=<account ref>
```

安装器通过现有启动链启动 CCSwitch 时注入环境变量。CCSwitch 启动阶段会幂等地用该 Key 创建或更新 `404token-sub2api` provider，写入 CCSwitch 自己的用户配置数据库和 live Codex 配置，并立即切换为当前 provider，因此首次启动不需要再打开编辑器点击“保存/应用”。环境变量不存在时保留原有手动配置能力，便于非 U 盘版本继续工作。Key 不写入 `providers.json`、静态发布资源或安装包；CCSwitch 的正常用户配置写入仅发生在已验证的 USB 启动链中。

### 3.4 账号密码的产生与使用

签名工具的新账号注册流程统一使用初始密码 `12345678`。账号密码不写入 U 盘，也不传给 CCSwitch；成功结果页会显示该初始密码，便于交付。已有账号只做幂等匹配，不自动重置密码；中断恢复沿用已创建账号的原密码，避免重复创建或把恢复流程切换到另一套凭据。

用户在安装器账号卡片中输入密码；管理器使用签名授权中的隐藏 `loginEmail` 调用现有登录接口。登录会话只保存在内存中，关闭管理器即失效。密码用于账号信息和用户侧充值码兑换，CCSwitch 使用独立的用户 API Key。这样既保留“只输入一次密码”的体验，也避免把密码复制到 U 盘。

同一 U 盘再次签名时默认执行 `ensure`：复用原账号和原用户 API Key，不自动轮换；只有明确选择“撤销并重新发放”才轮换。

## 4. 四部分职责

### 4.1 404token 后台

本期直接复用现有 Sub2API 接口，不新增后台 Provisioning 服务。管理员 API Key 只由签名工具使用。

签名工具调用的管理员接口：

```text
GET  /api/v1/admin/users?search=USB-
POST /api/v1/admin/users
POST /api/v1/admin/users/:id/balance
```

签名工具为新用户调用的用户接口：

```text
POST /api/v1/auth/login
POST /api/v1/keys
GET  /api/v1/auth/me
GET  /api/v1/usage/stats
POST /api/v1/redeem
```

其中：

- 管理员接口创建用户、查找编号，并在签名成功后增加余额；
- 用户登录接口使用隐藏的模拟邮箱和初始密码，返回用户 JWT；
- 用户 API Key 接口生成用于 CCSwitch 的 Key；
- 用户信息、用量和充值码兑换继续使用现有接口。

更新包继续使用 `update.404token.xyz`；账户操作沿用 404token/Sub2API 的现有 API 基地址，不新增第二套账户数据库。

本期实际调用契约：

```text
POST /api/v1/admin/users
请求：email、password、username、notes、balance=0、allowed_groups
响应：sub2apiUserId、accountName、balance

POST /api/v1/auth/login
请求：loginEmail、password
响应：access_token、expires_in、user

POST /api/v1/keys（每个新账号只调用一次）
请求：Authorization: Bearer <userJwt>、name、group_id
其中 `group_id` 是活动 `unified-api-internal` 分组的动态数值 ID；不传逐模型白名单
响应：一个可使用该分组全部模型的完整用户 API Key

POST /api/v1/admin/users/:id/balance
请求：x-api-key、balance、operation=add、notes、Idempotency-Key
响应：更新后的用户余额
```

管理员接口的 `x-api-key` 只存在签名工具内存/配置中，不进入用户接口和 U 盘。编号冲突时重新查询并顺延；相同 U 盘已有绑定时复用已有账号和 API Key。预充值使用稳定 `Idempotency-Key`，相同请求不得重复增加余额。

### 4.2 签名工具

签名按钮内部顺序固定为：

```text
扫描 U 盘
→ 校验 Bundle 完整性
→ 读取已有 accountBinding 或查询下一个 USB 编号
→ 管理员创建 Sub2API 用户（新 U 盘）
→ 用户登录并生成 Sub2API API Key（新 U 盘）
→ 生成 accountBinding
→ 写入待签名 usb-license.json
→ 对完整 license 签名
→ 回读并验证签名、账号和设备 Key
→ 按金额执行管理员余额增加
→ 显示“签名及账号绑定成功”
```

任意一步失败，都不能留下“签名成功”的半成品。

本期签名工具需要 404token API 基地址和模拟邮箱域名；正式签名器由受信任部署在编译时内置管理员 API Key，GUI 中只读显示，不接受手工输入或发布目录旁置 Key 文件。目标分组固定为 `unified-api-internal`，由签名器在注册前动态解析活动数值 ID。管理员 API Key 可被提取，因此签名工具必须视为受信任生产工具。日志只记录 `accountRef`、用户 ID 和结果，不记录密码、用户 API Key 或管理员 API Key。

### 4.3 安装包/安装器

安装包保持通用，不为每个账号重新打包 Codex、Skill 或 MCP。

每个 U 盘只在签名阶段写入自己的 `usb-license.json`。安装器启动后：

1. 先完成现有 USB 签名和 Bundle 校验；
2. 读取已验证的 `accountBinding`；
3. 将账号名称以只读方式显示；
4. 使用隐藏的 `loginEmail` 和用户输入的密码调用现有账户接口；
5. 查询余额、用量和充值状态；
6. 启动 CCSwitch 时注入设备 Key。

账户服务不可用时，不影响 Codex 离线安装和本地启动；账户卡片显示“服务暂不可用”。设备 Key 缺失或签名授权中的账号绑定无效时，CCSwitch 自动配置失败并显示明确错误，不弹出 CMD/Powershell 窗口。

### 4.4 CCSwitch

只增加一个受控入口：

```text
读取 CCSWITCH_404TOKEN_DEVICE_KEY
→ 绑定 404token provider
→ 使用该 Key 发起请求
```

不修改 `providers.json` 写入 API Key，不把 Key 写入安装包静态资源，不增加账号、充值和用量页面。

现有 provider catalog 继续只保存非秘密信息：地址、模型、协议和 provider ID。

当前实现进度：管理器启动前会从已验证的签名许可证读取绑定，并把设备 Key、账号引用、分组和 provider ID 只注入 CCSwitch 子进程环境；旧版或未绑定许可证不会注入。当前随包 `CodexSwitch.exe` 已支持环境变量读取，并在启动阶段幂等保存/启用 `404token-sub2api` provider；用户不需要再打开编辑器点击第二次保存。

## 5. 授权数据结构

在现有 `UsbLicense` 中增加可选、参与签名的字段：

```json
{
  "accountBinding": {
    "schemaVersion": 1,
    "usbAssetId": "USB-0001",
    "accountRef": "USB-0001",
    "accountName": "USB-0001",
    "loginEmail": "u1@u.404token.xyz",
    "sub2apiUserId": 23,
    "ccswitch": {
      "providerId": "404token-sub2api",
      "credentialId": "sub2api-user-key-23",
      "deviceApiKey": "...",
      "groupId": 7
    }
  }
}
```

约束：

- `accountBinding` 参与 Ed25519 签名；
- 旧版无该字段的许可证仍可读取，但新正式包要求该字段完整；
- `deviceApiKey` 只允许出现在授权文件，不得出现在 `manifest.json`、`SHA256SUMS.txt`、日志和安装包静态资源；
- `loginEmail` 只用于后台登录定位，安装器默认不显示；
- `sub2apiUserId` 和 `groupId` 用于回读一致性检查；
- 管理员 API Key 不得出现在授权文件；
- 读取后只用于 CCSwitch 进程启动环境，不回显完整 Key；
- 错误信息只显示 `credentialId` 的短摘要。

## 6. 安装器页面

首页保持简洁，只增加一张折叠卡片：

```text
404token
账号：USB-0001
状态：未登录

密码：[              ]   [登录]
```

登录后：

```text
404token
账号：USB-0001
余额：12,500
今日用量：320
本月用量：8,460

[充值] [刷新]
```

充值弹窗只包含充值码输入框和确认按钮。充值码由后台一次性核销，安装器不保存余额、不本地计算余额、不保存充值码。界面显示单位必须以后端实际余额单位为准，不在本期擅自把余额标成 Token。

## 7. 失败与恢复规则

| 场景 | 行为 |
|---|---|
| 后台管理员 Key 失效 | 新注册停止，提示更新签名工具设置 |
| 后台注册失败 | 签名停止，不写入正式许可证 |
| 账号已存在 | 幂等复用该账号和用户 API Key，不重复预充值 |
| 签名回读不一致 | 报错并保留旧许可证，不报告成功 |
| 账号密码错误 | 只提示登录失败，CCSwitch 仍可使用设备 Key |
| 账户服务暂时不可用 | 不影响 Codex 安装和启动 |
| 设备 Key 缺失 | CCSwitch 不启动自动配置，显示“请重新签名” |
| U 盘丢失 | 后台撤销该设备 Key，其他账号不受影响 |
| 签名成功但预充值失败 | 保留签名，显示待重试，使用同一幂等键重试 |
| 充值码重复提交 | 使用幂等键，只产生一笔流水 |

## 8. 实施范围

### 本期必须做

1. `UsbLicense.accountBinding` 扩展及签名器 GUI 注册流程；
2. 签名工具管理员 Key、目标分组、邮箱域名配置；
3. 自动编号、Sub2API 用户创建、用户登录和 API Key 生成；
4. 签名成功后的幂等预充值；
5. 管理器读取授权绑定并展示账户页面；
6. 管理器启动 CCSwitch 时注入环境变量；
7. CCSwitch 读取设备 Key 并自动选择 404token provider；
8. 失败时不弹控制台窗口、不泄露秘密、不留下半签名状态。

### 本期明确不做

- 在线支付、自动续费、优惠券、退款；
- 二维码或设备码登录；
- 复杂的多设备账号中心；
- U 盘离线查询余额；
- 将账号密码或管理员 API Key 写入 U 盘；
- 把设备 Key 放进静态安装包或 `providers.json`；
- 本期新增独立 VPS Provisioning 服务；
- 修改 Codex 更新、Skill Hub、USB Anchor 和卸载协议。

### 8.1 旧版本与迁移

- 发行包保持通用；不把账号和设备 Key 固定编译进 EXE。
- 旧 `usb-license.json` 没有 `accountBinding` 时，Codex 安装和启动仍按原流程工作，但账号卡片显示“此 U 盘尚未绑定账号”，CCSwitch 不自动填充 Key，并提供“请重新签名”提示。
- 新签名必须补齐 `accountBinding`；绑定完成后才允许显示“免手工配置 CCSwitch”。
- 重新签名不改变 `usbAssetId`，默认复用原绑定；新 U 盘由 `USB-*` 序号自动顺延，禁止复制旧许可证冒充新设备。
- 后台撤销设备 Key 后，CCSwitch 请求必须失败并提示重新授权；撤销不影响账号余额，也不删除充值流水。

## 9. 验收标准

### 签名工具

- 新 U 盘扫描后自动得到下一个 `USB-*` 编号和短模拟邮箱；
- 管理员 Key 可完成用户创建、用户 API Key 生成和预充值流水；
- 签名后的许可证包含正确账号和 `credentialId`；
- 修改账号绑定或设备 Key 后签名验证失败；
- 日志和诊断包不包含密码、用户 API Key 和管理员 API Key。
- 同一 U 盘重复点击签名不会产生第二个账号或第二把 Key；明确轮换后旧 Key 立即失效。
- 更换电脑运行签名工具时，只要随签名器目录部署默认管理员配置并扫描到目标 U 盘，就能复用或轮换绑定；不能依赖某台电脑的本机公钥。

### 安装器

- 不输入 API Key 即可启动 CCSwitch；
- 账号名称自动显示且不可编辑；
- 登录后余额、用量、充值结果来自后台；
- 账户接口故障不阻断 Codex 本地安装；
- 设备 Key 缺失时给出明确提示，不出现控制台窗口。

### CCSwitch

- 通过环境变量自动使用 404token provider；
- 不再出现 API Key 输入弹窗；
- 环境变量缺失时保留原有手动模式；
- API 请求不把设备 Key 输出到日志、错误弹窗或诊断包。
- 从未绑定、已撤销或已过期的授权启动时，给出可理解的修复提示，不回退到要求用户手工粘贴 Key 的死循环。

## 10. 推荐开发顺序

1. 先冻结 `accountBinding` JSON、短邮箱和目标分组契约；
2. 签名器接入现有 Sub2API 管理员/用户接口并完成 license 回读校验；
3. 增加签名成功后的幂等预充值和 pending 恢复；
4. 管理器实现账户卡片、内存会话和 CCSwitch 环境变量注入；
5. CCSwitch 增加环境变量读取并重新构建随包版本；
6. 做四部分联调，再重新构建 U 盘包和签名器；
7. 用一只新 U 盘验收，再用重复注册、错误 Key、撤销 Key、分组失效和重复预充值回归。

本方案的核心是：账号映射由后台负责，授权绑定由签名器负责，注入由安装器负责，调用由 CCSwitch 负责；每个组件只做一件事，避免形成新的互相依赖循环。
