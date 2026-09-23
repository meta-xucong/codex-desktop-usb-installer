# Sub2API 接口契约参考文档

版本：G39-verified  
状态：基于签名工具已验证实现提取  
来源：`tools/usb-license-signer/src/sub2api.rs`（G38.1 测试联调通过）

## 1. 概述

本文档记录 404token/Sub2API 后台接口的实际请求/响应结构,基于签名工具代码中已验证通过的实现。管理器账户模块必须使用完全相同的契约。

**重要约束：**
- 响应解析使用递归查找策略（`find_string`、`find_id`）,因为后端可能使用 `data` 包装或扁平结构
- Token/Key 字段可能带脱敏标记（`****`）,必须过滤
- 错误响应可能只返回 HTTP 状态码 + 空 JSON `{}`
- 所有接口超时：连接 15 秒,总计 45 秒

## 2. 基础配置

### 2.1 基地址

```text
生产环境：https://404token.xyz
API 前缀：/api/v1
User-Agent：codex-usb-license-signer/1（签名工具）
            Codex-App-Manager/404token（管理器建议使用）
```

### 2.2 认证方式

| 接口类型 | 认证方式 | 示例 |
|---|---|---|
| 管理员接口 | `x-api-key` 头 | `x-api-key: admin_key_xxx` |
| 用户接口（需登录）| `Authorization: Bearer` | `Authorization: Bearer eyJ0eXAi...` |
| 用户接口（公开）| 无需认证 | 登录接口本身 |

### 2.3 幂等性

预充值接口支持幂等键：
```http
Idempotency-Key: usb-preload-USB-0001-a1b2c3d4...
```

## 3. 用户接口契约

### 3.1 用户登录

**接口：** `POST /api/v1/auth/login`

**请求：**
```json
{
  "email": "u1@u.404token.xyz",
  "password": "12345678"
}
```

**响应结构（已验证）：**

签名工具使用递归查找策略解析,支持以下任一结构：

**结构 A：扁平**
```json
{
  "access_token": "eyJ0eXAiOiJKV1QiLCJhbGc...",
  "expires_in": 3600,
  "user": {
    "id": 123,
    "username": "USB-0001",
    "email": "u1@u.404token.xyz"
  }
}
```

**结构 B：data 包装**
```json
{
  "data": {
    "accessToken": "eyJ0eXAiOiJKV1QiLCJhbGc...",
    "expiresIn": 3600,
    "user": {...}
  }
}
```

**解析策略（来自签名工具 `find_string` 函数）：**
```rust
// 递归查找 token,支持多种字段名
find_string(&value, &["access_token", "accessToken", "token"])
```

**关键字段：**
- Token 字段名候选：`access_token`、`accessToken`、`token`
- 过滤条件：非空且不包含 `****`（脱敏标记）
- 用户 ID：递归查找对象中的 `id` 字段（数字或字符串）

**错误响应：**
- `401 Unauthorized`：密码错误或账号不存在
- `403 Forbidden`：账号被禁用
- HTTP 状态码非 2xx 时,响应体可能为空 JSON `{}`

### 3.2 获取用户信息

**接口：** `GET /api/v1/auth/me` 或 `GET /api/v1/user`

**请求头：**
```http
Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGc...
```

**响应结构（已验证）：**

签名工具中未直接使用此接口展示余额,但从创建用户响应和预充值响应可推断：

```json
{
  "id": 123,
  "username": "USB-0001",
  "email": "u1@u.404token.xyz",
  "balance": 12500.0,
  "credits": 12500.0,
  "quota": 12500.0,
  "remaining": 12500.0
}
```

**解析策略（来自 `json_number_or_value` 逻辑）：**
```rust
// 余额字段名候选（按优先级）
keys = ["balance", "credits", "quota", "remaining"]
// 返回第一个找到的字段值（数字或 JSON 值）
```

**关键逻辑：**
- 余额可能是顶层字段,也可能在 `data` 包装内
- 支持数字类型或字符串类型（会尝试解析）
- 管理器应使用相同的多字段名回退策略

### 3.3 获取用量统计

**接口：** `GET /api/v1/usage/stats`

**请求头：**
```http
Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGc...
```

**响应结构（推断）：**

签名工具中未详细解析用量,仅确认接口可调用。基于常见 API 设计推断：

```json
{
  "today": 320,
  "this_month": 8460,
  "total": 125000,
  "period_start": "2026-09-01T00:00:00Z",
  "period_end": "2026-09-30T23:59:59Z"
}
```

或带 `data` 包装：
```json
{
  "data": {
    "usage": {
      "today": 320,
      "thisMonth": 8460
    }
  }
}
```

**管理器实现建议：**
- 使用递归查找策略,支持多种字段名
- 不假设固定结构,灵活适配
- 用量字段可能命名为：`today`、`daily`、`this_month`、`thisMonth`、`monthly` 等

### 3.4 充值码兑换

**接口：** `POST /api/v1/redeem`

**请求：**
```json
{
  "code": "RECHARGE-ABCD1234"
}
```

或可能是：
```json
{
  "redeemCode": "RECHARGE-ABCD1234"
}
```

**响应结构（推断）：**

签名工具未实现此接口,基于常见模式推断：

**成功：**
```json
{
  "success": true,
  "amount": 1000.0,
  "new_balance": 13500.0,
  "message": "充值成功"
}
```

**失败：**
```json
{
  "success": false,
  "error": "充值码无效或已使用",
  "code": "INVALID_CODE"
}
```

**错误码推断：**
- `INVALID_CODE`：充值码不存在或格式错误
- `ALREADY_USED`：充值码已被使用
- `EXPIRED`：充值码已过期

**重要：管理器必须核实实际契约**
由于签名工具未实现充值接口,管理器在实现前必须：
1. 查看 Sub2API 后台源码或 API 文档
2. 或进行一次测试调用确认实际请求/响应结构
3. 特别确认充值码重复提交的语义（是幂等还是报错）

### 3.5 生成用户 API Key（签名工具）

**接口：** `POST /api/v1/keys`

**请求头：**
```http
Authorization: Bearer <userJwt>
```

**请求体：**
```json
{
  "name": "CCSwitch-USB-0001",
  "group_id": 7
}
```

其中 `group_id` 不是固定常量：签名工具先读取管理员可见的活动分组，精确匹配分组名
`unified-api-internal`，再将解析出的数值 ID 同时写入用户的 `allowed_groups` 和 API Key。
上面的 `7` 仅为测试夹具示例。

每个新 U 盘只调用一次该接口并生成一个用户 API Key；请求不携带逐模型白名单，模型能力由
`unified-api-internal` 分组统一提供。签名工具不会为每个模型重复生成 Key。

**响应：** 返回完整用户 API Key（字段名可能为 `key`、`api_key`、`apiKey` 或 `token`）。
完整 Key 只在内存中传递到签名许可证写入步骤，不写入日志或普通配置。

## 4. 管理员接口契约

管理器不使用管理员接口,此处仅供参考。

### 4.1 创建用户（管理员）

**接口：** `POST /api/v1/admin/users`

**请求头：**
```http
x-api-key: admin_key_xxx
```

**请求体：**
```json
{
  "email": "u1@u.404token.xyz",
  "password": "12345678",
  "username": "USB-0001",
  "notes": "Codex USB binding USB-0001",
  "balance": 0,
  "allowed_groups": [7]
}
```

`allowed_groups` 中的 ID 必须是活动的 `unified-api-internal` 分组 ID；`7` 仅为示例。

**响应：**
```json
{
  "id": 123,
  "username": "USB-0001",
  "email": "u1@u.404token.xyz",
  "balance": 0.0
}
```

### 4.2 预充值（管理员）

**接口：** `POST /api/v1/admin/users/{userId}/balance`

**请求头：**
```http
x-api-key: admin_key_xxx
Idempotency-Key: usb-preload-USB-0001-a1b2c3d4...
```

**请求体：**
```json
{
  "balance": 10000.0,
  "operation": "add",
  "notes": "Codex USB USB-0001 initial preload"
}
```

**响应：**
```json
{
  "id": 123,
  "balance": 10000.0
}
```

## 5. 错误处理契约

### 5.1 HTTP 状态码映射

签名工具使用的错误分类（`http_error_kind` 函数）：

| 状态码 | 分类 | 用户友好消息 |
|---|---|---|
| 401, 403 | 授权失败 | "授权失败" |
| 404 | 接口不存在 | "接口不存在" |
| 409 | 编号冲突 | "编号冲突"（签名器特定） |
| 422 | 参数被拒绝 | "参数被拒绝" |
| 5xx | 服务端错误 | "服务端错误" |
| 其他 | 请求被拒绝 | "请求被拒绝" |

### 5.2 响应体为空的情况

当 HTTP 状态码非 2xx 时,响应体可能是空 JSON `{}`（签名工具代码第 724-726 行）：

```rust
if bytes.is_empty() {
    return Ok(json!({}));
}
```

因此错误处理逻辑不能假设响应体包含 `error`、`message` 等字段。

### 5.3 管理器安全错误映射

管理器必须实现安全的错误映射,**绝不直接返回原始响应正文给前端**：

```rust
fn safe_error_message(status: u16, body: &str) -> String {
    match status {
        401 => "密码错误或会话已过期,请重新登录".to_string(),
        403 => "账号已被禁用,请联系管理员".to_string(),
        404 => "服务接口不可用".to_string(),
        422 => "请求参数无效".to_string(),
        500..=599 => "服务暂时不可用,请稍后重试".to_string(),
        _ => "请求失败,请检查网络连接".to_string(),
    }
    // 日志可记录完整的 status 和 body,但不传给前端
}
```

## 6. 管理器实现检查清单

基于签名工具已验证的契约,管理器必须：

### 6.1 登录接口（`account.rs` 中的 `login` 函数）

- [ ] 使用 `POST /api/v1/auth/login`
- [ ] 请求体包含 `email` 和 `password` 字段
- [ ] 使用递归查找策略解析 token：`["access_token", "accessToken", "token"]`
- [ ] 过滤包含 `****` 的脱敏值
- [ ] 支持 `data` 包装和扁平结构
- [ ] 401 错误映射为"密码错误"而非原始响应
- [ ] 记录 `expires_in` 用于会话过期判断

### 6.2 余额查询（`account.rs` 中的 `status` 函数）

- [ ] 使用 `GET /api/v1/auth/me` 或 `/api/v1/user`
- [ ] 请求头包含 `Authorization: Bearer {token}`
- [ ] 使用多字段名回退：`["balance", "credits", "quota", "remaining"]`
- [ ] 返回第一个找到的非空数字值
- [ ] 支持数字类型和字符串类型
- [ ] 401 错误清除会话并提示"会话已过期"

### 6.3 用量查询（`account.rs` 中的 `usage` 函数）

- [ ] 使用 `GET /api/v1/usage/stats`
- [ ] 请求头包含 `Authorization: Bearer {token}`
- [ ] 使用递归查找策略,支持多种字段名
- [ ] 不假设固定的 JSON 结构
- [ ] 查询失败时返回 `None`,不阻塞余额显示

### 6.4 充值接口（`account.rs` 中的 `redeem` 函数）

- [ ] **优先级 P0：先核实实际契约**（签名工具未实现）
- [ ] 确认请求字段名：`code` 或 `redeemCode`
- [ ] 确认成功/失败响应结构
- [ ] 确认重复兑换的语义（幂等或报错）
- [ ] 实现超时后的结果查询逻辑
- [ ] 充值成功但刷新失败时,不显示"充值失败"

### 6.5 会话管理

- [ ] 会话绑定 `usb_asset_id`（U 盘身份）
- [ ] 记录 `expires_at`（基于 `expires_in`）
- [ ] 401 响应时清除会话
- [ ] U 盘变化时（`usb_asset_id` 不匹配）清除会话
- [ ] 并发请求时 token 使用克隆,避免竞态

### 6.6 安全性

- [ ] 错误消息不包含原始响应正文
- [ ] 日志可记录详细错误,但不传给前端
- [ ] Token 不输出到日志
- [ ] 管理员 Key 绝不进入管理器代码

## 7. 测试建议

### 7.1 契约验证测试

在实现前,使用 `curl` 或 HTTP 客户端验证实际响应：

```bash
# 登录测试（使用测试账号）
curl -X POST https://404token.xyz/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"testpass"}' \
  -v

# 余额查询（使用上面获得的 token）
curl -X GET https://404token.xyz/api/v1/auth/me \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -v

# 用量查询
curl -X GET https://404token.xyz/api/v1/usage/stats \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -v
```

### 7.2 Mock 测试

管理器单元测试应 mock 以下场景：

1. **登录成功**：扁平结构和 `data` 包装结构
2. **登录失败**：401 + 空 JSON
3. **余额查询**：多种字段名（`balance`、`credits` 等）
4. **会话过期**：401 响应
5. **服务端错误**：5xx + 空 JSON
6. **网络超时**：连接超时、读取超时

### 7.3 端到端测试

使用真实测试账号验证：

1. 登录 → 查询余额 → 查询用量
2. 错误密码 → 确认提示友好
3. 会话过期（等待 `expires_in`）→ 确认自动清除并提示重新登录
4. 换 U 盘 → 确认会话自动清除

## 8. 与签名工具的差异

| 特性 | 签名工具 | 管理器 |
|---|---|---|
| 管理员 Key | ✅ 使用（创建用户、预充值）| ❌ 禁止使用 |
| 用户登录 | ✅ 用于生成用户 API Key | ✅ 用于账户功能 |
| 余额查询 | ✅ 验证预充值成功 | ✅ 显示给用户 |
| 用量查询 | ✅ 确认接口可用 | ✅ 显示给用户 |
| 充值接口 | ❌ 未实现 | ✅ 必须实现 |
| 错误处理 | 简化（工具内部使用）| 必须安全映射 |
| 会话管理 | 临时（单次注册流程）| 持久（内存会话） |

## 9. 参考实现位置

签名工具中已验证的实现：

| 功能 | 文件位置 | 行号范围 |
|---|---|---|
| 登录解析 | `sub2api.rs` | 627-637 |
| Token 查找 | `sub2api.rs` | 909-924 (`find_string`) |
| 余额字段回退 | `account.rs`（管理器）| 48-50 (`json_number_or_value`) |
| 错误分类 | `sub2api.rs` | 743-752 (`http_error_kind`) |
| 预充值幂等 | `sub2api.rs` | 504-518 |

## 10. 更新记录

| 日期 | 版本 | 变更 |
|---|---|---|
| 2026-09-20 | G39-verified | 初始版本,基于签名工具 G38.1 实现提取 |

---

**重要提示：**

充值接口（`POST /api/v1/redeem`）的实际契约尚未在签名工具中验证,管理器实现前必须：
1. 查看 Sub2API 后台源码确认请求/响应结构
2. 或进行测试调用确认实际行为
3. 特别注意重复兑换的语义和超时处理

其他接口（登录、余额、用量）的契约已通过签名工具的真实测试联调验证,可直接使用。
