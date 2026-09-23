# 签名工具一键注册、生成 API Key 与预充值开发文档

版本：G39.5
状态：注册/签名链路已落地；新建 API Key 固定到 `unified-api-internal` 分组，并保证每个新 U 盘只生成一个可用于该分组全部模型的 Key；管理员 Key 编译期内置、固定初始密码、加密恢复、分页编号和冲突重试已回归；实际 U 盘写入、真实账户激活和目标机安装仍需实机验收
适用范围：USB 签名工具、404token/Sub2API、`usb-license.json`、CCSwitch 启动注入
上位文档：[USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md](./USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md)

> 本文件是总文档中“签名工具注册与授权发放”部分的最终细分规范。两份文档有冲突时，以本文件为准；总文档已按本文件同步。本文只规定本期临时直连方案，不要求先开发新的 VPS Provisioning 服务。

## 1. 本期目标

签名人员把完整 U 盘安装包放入目标 U 盘后，只需要在签名工具中填写预充值金额并点击一次按钮，即可完成：

```text
扫描 U 盘
→ 校验 Bundle
→ 自动分配 U 盘编号
→ 创建 Sub2API 用户
→ 自动生成用户 API Key
→ 写入并签名 usb-license.json
→ 签名回读校验
→ 按填写金额预充值
```

签名工具直接使用 404token/Sub2API 管理员 API Key 完成后台操作。本期接受“签名工具是受信任生产工具、管理员 Key 可被提取”的取舍；管理员 Key 不进入 U 盘安装包、安装器或 `usb-license.json`。

## 2. 已确认的 Sub2API 能力边界

当前部署已经具备以下接口和能力：

| 目的 | 接口 | 本期结论 |
|---|---|---|
| 创建用户 | `POST /api/v1/admin/users` | 支持；管理员接口要求邮箱和密码 |
| 查询用户/分配编号 | `GET /api/v1/admin/users?search=...` | 支持；用于查找已注册的 `USB-*` 用户 |
| 用户登录 | `POST /api/v1/auth/login` | 支持；返回用户 JWT |
| 生成用户 API Key | `POST /api/v1/keys` | 支持；要求用户 JWT，不是管理员直接创建接口 |
| 预充值 | `POST /api/v1/admin/users/:id/balance` | 支持；使用 `operation=add` |
| 查看余额和用量 | 用户认证后的现有 `/api/v1/user`、`/api/v1/usage` 接口 | 支持 |

因此“一键”是签名工具内部自动完成多次 HTTP 调用，而不是要求 Sub2API 当前已有一个组合接口。

当前管理员 API Key 只作为签名工具后台凭证使用。不得调用重新生成管理员 Key 的接口作为正常注册流程，因为这会立即使其他签名工具失效。

## 3. U 盘编号规则

### 3.1 显示编号

新 U 盘采用人类可读的顺序编号：

```text
USB-0001
USB-0002
USB-0003
```

编号是账号业务编号，不等同于 Windows 卷序列号，也不等同于签名指纹。卷序列号、Bundle Anchor 和其他硬件信息继续由现有 USB 授权校验负责。

### 3.2 自动顺延

本期不新增独立序号数据库，使用 Sub2API 用户列表作为现有注册记录：

1. 签名工具查询保留前缀 `USB-` 的用户；
2. 提取用户名末尾的数字并取最大值加一；
3. 新用户名使用该编号；
4. 如果创建时发生唯一性冲突，重新查询并顺延重试；
5. 创建成功后编号写入签名授权文件。

保留前缀必须是系统专用前缀，普通用户不能使用 `USB-` 命名。删除用户不回收编号，避免历史授权混淆。

### 3.3 已注册 U 盘

如果当前许可证已经包含 `accountBinding`，签名工具显示原编号并执行 `ensure`：

- 不创建第二个账号；
- 不重复生成用户 API Key；
- 不重复预充值；
- 只有用户明确选择“重新发放 Key”时才轮换。

如果本地签名中断，签名工具保留一份不含密码和完整 Key 的 pending 元数据，用于下一次继续，不依赖重新猜测编号。

## 4. 简短模拟邮箱规则

Sub2API 当前管理员创建用户接口强制校验 `email`，不能只提交用户名。因此使用内部短邮箱，不要求真实邮箱可收信：

```text
显示账号名：USB-0001
Sub2API 用户名：USB-0001
登录邮箱：u1@u.404token.xyz
```

规则：

- `u<数字>@u.404token.xyz` 是保留格式；
- 数字部分使用 U 盘顺序编号，不补充冗长随机串；
- 登录邮箱写入 `accountBinding.loginEmail`，但安装器界面默认不显示；
- 当前邮箱验证关闭，管理员创建用户不需要真实收件箱；
- 如果未来打开邮箱验证，仍以管理员创建接口为准，不依赖发送邮件；
- 若后台域名变化，邮箱域名作为签名工具配置项更新，不改变已注册账号。

## 5. 签名工具界面

### 5.1 主界面

```text
404token 注册与签名

当前 U 盘：F:\
Bundle：校验通过
注册编号：USB-0001（已注册）
          USB-0002（待注册）

预充值金额：[              ]
说明：0 表示不预充值，单位沿用 Sub2API 余额单位

初始密码：[12345678（固定初始密码）]

[注册账号并签名]
```

工具为新建 404token 账号统一使用初始密码 `12345678`；执行前会显示目标 U 盘盘符、卷序列号和待分配的 `USB-xxxx` 编号。密码不写入 U 盘、日志或 `usb-license.json`，成功结果页会再次显示，便于交付。已有账号绑定只复用原账号和原用户 API Key，不修改密码；中断恢复也沿用已创建账号的原密码，不重新生成或重置。

### 5.2 后台设置

非敏感偏好可在签名工具界面调整：

- 404token API 基地址；
- 404token 目标分组固定为 `unified-api-internal`（签名器会在注册前动态解析其活动数值 ID）；
- 模拟邮箱域名，默认 `u.404token.xyz`。

管理员 Key 由受信任部署自动提供：运行时环境变量仅用于受控构建/轮换，正式签名器 EXE 使用编译期内置 Key；GUI 只读显示，不要求操作员手填。管理员 Key 不进入签名器发布目录的旁置文件、运行期 U 盘、许可证、诊断包或日志。仅 API 地址和邮箱域名等非敏感配置写入签名器用户配置区。

### 5.3 分组配置

当前 Sub2API 的 API Key 可以不带分组创建，但未绑定可用分组的 Key 可能无法调用。签名工具固定使用活动的 `unified-api-internal` 分组，该分组承担全部模型能力：

- 先读取管理员可见的活动分组列表，按名称精确解析 `unified-api-internal` 的数值 ID；
- 创建用户时将解析出的 ID 写入 `allowed_groups`；
- 创建用户 API Key 时将同一 ID 写入 `group_id`；
- 分组不存在、停用或无权限时，在创建用户前停止；
- 不让每次签名人员重新选择分组；
- 每个新 U 盘只调用一次 `/api/v1/keys`，生成一个 Key，不按模型拆分或重复生成。

## 6. 一键执行顺序

### 6.1 预检

1. 扫描可移动 NTFS U 盘；
2. 定位完整 Bundle；
3. 完成现有 Anchor、Payload、文件清单和签名环境检查；
4. 读取已有许可证；
5. 验证 API 基地址、管理员 Key 和目标分组可用；
6. 读取或计算当前 U 盘编号；
7. 验证预充值金额为有限的非负数，超过策略上限则停止。

预检失败时不创建用户、不生成 API Key、不修改余额。

### 6.2 新 U 盘注册

按以下顺序调用：

```text
GET  /api/v1/admin/users?search=USB-
POST /api/v1/admin/users
POST /api/v1/auth/login
POST /api/v1/keys
```

创建用户请求的逻辑字段：

```json
{
  "email": "u2@u.404token.xyz",
  "password": "<initial password>",
  "username": "USB-0002",
  "notes": "Codex USB binding USB-0002",
  "balance": 0,
  "allowed_groups": [7]
}
```

以上 `7` 仅为请求示例；生产运行时使用动态解析出的 `unified-api-internal` 分组 ID。

生成用户 API Key 时使用用户 JWT：

```json
{
  "name": "CCSwitch-USB-0002",
  "group_id": 7
}
```

以上 `7` 同样仅为示例。返回的完整用户 API Key 只保存在内存，随后写入待签名授权文件；不得写入日志。
该单个 Key 的模型范围由 `unified-api-internal` 分组提供，不需要额外的 `models` 字段。

### 6.3 签名与预充值

1. 将账号绑定和用户 API Key 写入临时许可证；
2. 对完整许可证执行现有签名流程；
3. 回读许可证并验证签名、编号、用户 ID、分组和 Key 摘要；
4. 只有回读成功后，且预充值金额大于 0，才调用：

```text
POST /api/v1/admin/users/:id/balance
```

请求逻辑字段：

```json
{
  "balance": 100,
  "operation": "add",
  "notes": "Codex USB USB-0002 initial preload"
}
```

预充值请求必须带稳定幂等键，例如：

```text
usb-preload-USB-0002-<license-fingerprint>
```

这样重复点击或网络重试不会重复加余额。

## 7. 失败、重试与回滚

| 阶段 | 失败行为 |
|---|---|
| Bundle 预检 | 立即停止，不产生后台数据 |
| 用户创建 | 报错停止；编号冲突时自动顺延重试 |
| 用户登录 | 报错停止；若已保存加密恢复记录，下一次续作同一账号 |
| API Key 创建 | 已创建用户时把密码/Key 写入当前 Windows 用户 DPAPI 加密恢复记录，下一次续作同一账号 |
| 本地签名 | 不预充值；恢复记录保留到签名和回读验证完成 |
| 签名回读 | 不报告成功，不预充值；不生成预充值 pending |
| 预充值 | 签名保留，显示“预充值待重试”，使用同一幂等键重试 |
| 管理员 Key 失效 | 所有新注册停止，提示更新签名工具后台配置 |
| 目标分组失效 | 注册前停止，不创建用户 |

不能把“账号已创建”误报成“注册成功”。只有许可证回读通过才显示“注册并签名成功”。

### 7.1 当前接口边界与安全降级

本期实现对“账号已创建、但本地签名中断”的处理采用同机加密恢复：恢复标识绑定卷序列号、Bundle 锚点和相对路径；密码与用户 Key 只写入当前 Windows 用户可解密的 DPAPI 记录，不进入 U 盘或发布目录。下一次对同一 Bundle 会先恢复原账号；记录损坏、解密失败、分组变化或 Bundle 不匹配时 fail-closed，要求人工核对，不猜测编号、不创建第二个账号。

签名成功但预充值失败时，当前版本保留签名结果并报告失败，不伪报成功；重复的“注册并签名”流程不会对已有绑定隐式再次预充值。G38.2 已补充签名器侧的显式“重试预充值”入口：预充值前写入只含账号 ID、金额、许可证摘要和稳定幂等键的本地待处理记录，成功后清除；失败时保留，用户必须在已签名 U 盘上主动重试。该入口不查询或重新生成用户 API Key，也不解决“已建号但签名中断”的完整自动续作限制。

## 8. 授权文件字段

`accountBinding` 参与现有 Ed25519 签名，建议结构：

```json
{
  "accountBinding": {
    "schemaVersion": 1,
    "usbAssetId": "USB-0002",
    "accountRef": "USB-0002",
    "accountName": "USB-0002",
    "loginEmail": "u2@u.404token.xyz",
    "sub2apiUserId": 23,
    "ccswitch": {
      "providerId": "404token-sub2api",
      "credentialId": "sub2api-user-key-23",
      "deviceApiKey": "sk-...",
      "groupId": 7
    }
  }
}
```

约束：

- 不写入账号密码；
- 不写入管理员 API Key；
- `deviceApiKey` 只写入已签名许可证，不进入静态安装包、`providers.json`、日志或诊断包；
- `loginEmail` 属于账号定位信息，安装器默认不显示；
- `sub2apiUserId` 和 `groupId` 用于一致性检查；
- 修改任一绑定字段后，签名验证必须失败。

## 9. 与安装器和 CCSwitch 的衔接

安装器读取并验证 `accountBinding` 后：

1. 页面只显示 `accountName`；
2. 用户输入密码；
3. 管理器使用隐藏的 `loginEmail` 调用现有登录接口；
4. 余额、用量和充值使用已认证的用户接口；
5. 启动 CCSwitch 时通过环境变量注入 `deviceApiKey` 和 `accountRef`。

CCSwitch 不读取管理员 API Key，也不负责账号创建、余额调整或充值。U 盘未绑定账号时，Codex 本地安装仍可继续，但 CCSwitch 自动配置提示“请重新签名”。

## 10. 安全与运维边界

本期接受管理员 Key 放在签名工具中的风险，采取以下最低限度措施：

- 管理员 Key 只存在于受信任签名器部署，不存在于运行期 U 盘、安装器和 CCSwitch；
- 支持通过受信任部署替换默认配置或环境变量，不在 GUI 中要求手工录入；
- Key 只放在 HTTP Header，不放在 URL、日志或错误文本；
- 任何 HTTP 日志、诊断包和截图自动掩码；
- 修改或删除后台管理员 Key 后，立即阻断新 U 盘注册；
- 已发放的用户 API Key 通过后台用户管理单独撤销；
- 签名工具不提供普通用户可访问的管理员操作入口。

后续若把签名工具交付给不完全可信的生产人员，再升级为 VPS 端窄权限 Provisioning 接口；不属于本期实现。

## 11. 验收标准

本次代码验收边界：以下本地验收已完成；另已使用一次明确授权的测试金额完成真实后台联调，未修改 Sub2API 服务端。

- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --lib`：45 passed / 0 failed；
- `cargo test --manifest-path crates/codex-usb-license/Cargo.toml`：14 passed / 0 failed；
- `cargo test --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui`：4 passed / 0 failed；
- `cargo check --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui`：通过；
- `cargo clippy --manifest-path tools/usb-license-signer/Cargo.toml --lib --bin usb-license-signer-gui -- -D warnings`：通过；
- `cargo build --release --manifest-path tools/usb-license-signer/Cargo.toml --bin usb-license-signer-gui`：通过；产物 SHA-256 为 `87FFC9CDB892E518B9558F3954F5B04570E794DD1A0E1F19B9297C621C0D4FED`；
- 本地 HTTP fixture 覆盖管理员查询/建号、用户登录、用户 Key、幂等预充值，并断言管理员 Key 不进入用户请求。
- 本地 HTTP fixture 先验证活动目标分组，再继续建号流程；缺失/停用分组会在创建用户前 fail-closed。
- 预充值重试会把 pending 中的许可证摘要与当前 U 盘 `usb-license.json` 摘要逐字比对，不匹配时不调用余额接口。
- 真实 404token 测试联调：新建测试用户、生成 1 个用户 API Key、预充值 10、pending 成功清除；未在文档记录管理员 Key、用户 API Key 或密码。

真实联调复用了签名器的 `Sub2ApiClient` 注册/Key/预充值代码路径，但当前插入的 F: 盘是已安装 runtime tree，不是可重新签名的干净发行 Bundle，因此没有伪称完成“GUI 点击→写入许可证→真实后台”的完整桌面端到端测试；GUI 本身已编译，完整桌面链路仍需用干净 Bundle 实机验收。

### 新 U 盘

- 自动显示下一个未使用的 `USB-xxxx` 编号；
- 创建短模拟邮箱，不要求真实邮箱；
- GUI 对新账号固定使用初始密码 `12345678`；注册摘要显示 U 盘盘符、卷序列号和 `USB-xxxx` 编号；
- 自动创建用户、登录并生成用户 API Key；
- API Key 和分组写入签名许可证；
- 签名成功后按金额只预充值一次；
- 预充值失败后可以使用同一幂等键显式重试，成功后自动清除本地待处理记录；
- 结果页显示账号编号和初始密码一次。

### 重复操作

- 同一 U 盘再次执行不会创建第二个账号；
- 不会重复生成 Key；
- 不会重复预充值；
- 网络重试不会重复扣/加余额；
- 预充值 pending 记录不含管理员 Key、用户 API Key 或密码；注册恢复记录使用 Windows DPAPI 加密，并在成功回读后删除；
- 更换管理员 Key 后新注册立即失败并给出明确提示。

### 安全与兼容

- U 盘和安装器中不存在管理员 API Key；
- 日志不出现完整管理员 Key、用户 API Key 或密码；
- 旧许可证没有绑定信息时仍可安装 Codex，但不能自动配置 CCSwitch；
- 不出现 CMD/Powershell 弹窗；
- 失败不报告“成功”。

## 12. 实施顺序

1. 扩展 `UsbLicense.accountBinding` 字段并补充序列化/签名测试；
2. 在签名工具设置中加入后台地址、管理员 Key 和目标分组配置；
3. 实现用户编号查询、冲突重试和短模拟邮箱；
4. 实现创建用户、用户登录、创建 API Key 的内存态流水线；
5. 实现签名成功后的幂等预充值；
6. 实现 pending 状态和失败恢复；
7. 做真实后台联调前，先用无充值测试账号验证全链路；
8. 通过审计后再重新打包签名工具和 U 盘包。
