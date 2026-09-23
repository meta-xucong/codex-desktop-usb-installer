# G39 阶段：404token 账号联动完成开发路线图

版本：G39.0  
状态：规划中，待执行  
创建时间：2026-09-20  
前置文档：[USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md](./USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md)、[USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md](./USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md)、[CCSWITCH-USB-ENV-RUNTIME-PATCH.md](./CCSWITCH-USB-ENV-RUNTIME-PATCH.md)

## 1. 当前状态确认

### 1.1 已完成且验证通过的部分

| 组件 | 功能 | 状态 | 验证范围 |
|---|---|---|---|
| 签名工具 | 账号自动编号与创建 | ✅ 已实现 | Mock 合约测试、真实后台测试通过 |
| 签名工具 | 用户 API Key 生成 | ✅ 已实现 | 真实账号创建、Key 生成成功 |
| 签名工具 | 预充值与幂等重试（G38.2）| ✅ 已实现 | 包含失败恢复逻辑 |
| 签名工具 | 许可证签名与回读验证 | ✅ 已实现 | 签名完整性校验通过 |
| 管理器 | USB 授权读取与验证 | ✅ 已实现 | `usb_guard.rs` 读取 accountBinding |
| 管理器 | 环境变量注入到 CCSwitch | ✅ 已实现 | `ccswitch.rs` 注入 4 个环境变量 |
| 管理器 | 账户功能初版 | ⚠️ 已实现但未验收 | 仅通过编译和基础测试 |
| CCSwitch | 产品版本静态验收 | ✅ 通过 | SHA256 校验、manifest 完整性 |

### 1.2 关键未完成项

#### 1.2.1 CCSwitch 自动配置闭环（阻塞整个链路）

**当前问题：**
- 随包 `CodexSwitch.exe` 是旧版本（SHA256: `75B847CA...`）,不会读取环境变量
- 虽然管理器已注入环境变量,但 CCSwitch 不会自动使用
- 存在临时构建记录（`C:\Users\T14S\AppData\Local\Temp\cc-switch-source-review-f2abd0aeaaa641db8c080ed25525947e`）,但：
  - 构建版本为 3.20.3,与随包声称的 3.14.1 不匹配
  - 没有证据证明工作区切到了声称的 commit `1d44b1ba41987cddeb8096fadba8190fab465cbb`
  - 临时替换后验证失败,已回滚

**必须找到真正的产品化源码：**
- 本地某处应有带 Codex 定制的完整 CCSwitch 源码及构建脚本
- 该源码包含：404token provider 预设、关闭上游更新、Codex 专用界面
- **不能使用纯上游源码**,会丢失产品化改造

#### 1.2.2 管理器账户模块的 8 个契约与安全问题

已实现账户功能但存在以下必须修复的问题：

1. **接口响应结构未核实**：当前登录从顶层读取 token,余额按多个字段名猜测
2. **充值接口未确认**：使用 `POST /api/v1/redeem` 和 `{code}`,未核实契约
3. **安全边界不足**：错误处理直接返回服务端响应,可能泄露敏感信息
4. **会话生命周期不完整**：未处理 U 盘拔出、换盘、401/过期等场景
5. **授权错误被掩盖**：签名异常可能统一变成"未绑定"
6. **前端状态加载不完整**：只在挂载时查一次,授权未就绪时入口可能隐藏
7. **用量未展示**：后端有字段但界面主要显示余额
8. **测试不足**：缺少 HTTP 契约测试和完整组件测试

## 2. 执行路线图

### 阶段 0：前置准备与冲突检查

**目标：** 确保开发环境安全,避免同时修改导致冲突

**执行步骤：**

1. **确认并行任务状态**
   - 用户确认任务 `01a04f20-1c47-7060-a966-e2f88664a3f1` 已停止
   - 避免两个会话同时修改同一目录

2. **差异归属确认**
   - 运行 `git status` 和 `git diff --stat`
   - 确认工作区所有未提交改动属于此任务
   - 不执行整体重置或清理

3. **基线快照**
   - 记录当前 CCSwitch EXE 的 SHA256（应为 `75B847CA...`）
   - 备份当前 `manifest.json` 和 `SOURCE-NOTICE.txt`

**验收标准：**
- 用户确认无并行修改
- 差异清单已记录
- CCSwitch 产品版本完整性确认

---

### 阶段 1：CCSwitch 自动配置闭环（P0 阻塞项）

**目标：** 完成 CCSwitch 运行时环境变量读取,实现自动配置免手工输入 Key

#### 1.1 定位产品化 CCSwitch 源码

**执行步骤：**

1. **搜索本地源码**
   - 查找可能的位置：
     - `D:\AI\AGENT_U盘\codex-desktop-usb-installer\ccswitch-source\`
     - `C:\Projects\`、`D:\Projects\`、用户桌面、文档等常见开发目录
     - 检查签名工具配置文件或构建脚本中是否有源码路径引用
   - 搜索关键文件：`Cargo.toml` 中包含 `cc-switch` 且有 Codex 定制

2. **验证源码完整性**
   - 确认包含 `404token-sub2api` provider 预设
   - 确认关闭上游更新的代码
   - 确认 Codex 专用界面定制
   - 检查构建脚本（`build.ps1` 或 `Makefile`）

3. **如果找不到源码**
   - 询问用户源码位置
   - 或从临时目录的构建记录逆向确认实际使用的版本
   - **绝不能直接使用纯上游源码替代**

**验收标准：**
- 找到包含产品化定制的完整源码
- 源码可通过 `cargo check` 且包含现有产品特性

#### 1.2 实现环境变量读取补丁

**执行步骤：**

1. **在产品源码中定位 `CodexAdapter::extract_key` 函数**
   - 文件应在 `src-tauri/src/proxy/providers/codex.rs` 或类似路径

2. **添加环境变量读取代码**（按 `CCSWITCH-USB-ENV-RUNTIME-PATCH.md`）：

```rust
// 在 extract_key 函数最前面加入
if provider.id == "404token-sub2api" {
    if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
}
```

3. **确认其他环境变量的使用**
   - `CCSWITCH_404TOKEN_PROVIDER`：自动选择 provider
   - `CCSWITCH_404TOKEN_ACCOUNT`：可选,用于日志标识
   - `CCSWITCH_404TOKEN_GROUP_ID`：验证分组匹配

4. **安全性检查**
   - Key 不写入数据库（`extract_key` 返回内存值,不调用持久化）
   - Key 不写入 `providers.json`
   - Key 不输出到日志或错误弹窗
   - 无环境变量时保留原有手动配置能力

**验收标准：**
- `cargo check --locked` 通过
- 补丁代码审查确认不泄露 Key
- 环境变量缺失时不破坏现有功能

#### 1.3 构建与替换

**执行步骤：**

1. **Release 构建**
   ```bash
   cargo build --release --locked
   ```

2. **提取可执行文件**
   - 从 `target/release/` 提取 `CodexSwitch.exe`
   - 计算新 EXE 的 SHA256

3. **更新随包资源**
   - 备份旧 `src-tauri/resources/ccswitch/CodexSwitch.exe`
   - 替换为新构建的 EXE
   - 更新 `manifest.json` 中的 SHA256
   - 更新 `manifest.json` 中的 `sourceVersion`（如果变化）
   - 更新 `SOURCE-NOTICE.txt`（如果版本或 commit 变化）

4. **运行静态验收**
   ```powershell
   .\tools\verify-product.ps1
   ```

**验收标准：**
- Release 构建成功
- `verify-product.ps1` 全部检查通过
- 新 SHA256 已记录在 manifest

#### 1.4 端到端闭环验证

**执行步骤：**

1. **启动测试准备**
   - 使用已签名的测试 U 盘（包含有效 accountBinding）
   - 或创建测试许可证（包含测试账号的 deviceApiKey）

2. **管理器启动 CCSwitch**
   - 运行管理器,点击"启动 CCSwitch"
   - 使用进程监视器确认子进程环境变量：
     ```powershell
     # 在 CCSwitch 进程启动后立即检查
     (Get-Process CodexSwitch).StartInfo.EnvironmentVariables
     ```

3. **验证自动配置**
   - CCSwitch 首次启动**不应弹出** API Key 输入框
   - 直接进入 Codex 可用状态
   - 发起一次 Codex 请求,确认使用注入的 Key

4. **验证秘密不落盘**
   - 检查 CCSwitch 数据库（通常在 `%APPDATA%/.cc-switch/`）
   - 检查 `providers.json`
   - 确认均不包含 deviceApiKey

5. **验证降级兼容**
   - 删除环境变量或使用无绑定的许可证
   - CCSwitch 应回退到手动配置模式,不崩溃

6. **验证单实例行为**
   - CCSwitch 已运行时再次启动管理器
   - 确认不会误用旧环境变量或忽略新环境变量

**验收标准：**
- ✅ 有绑定时不弹 Key 输入框
- ✅ Codex 请求实际使用注入的 Key
- ✅ Key 不存在于数据库或配置文件
- ✅ 无绑定时正常降级
- ✅ 单实例进程环境变量处理正确

---

### 阶段 2：账户模块契约修复（P1 必须项）

**目标：** 修复账户功能的接口契约、安全问题、会话管理和测试

#### 2.1 核实 Sub2API 接口契约

**执行步骤：**

1. **参照签名工具实现**
   - 读取 `tools/usb-license-signer/src/sub2api.rs`（第 150-400 行）
   - 签名工具已验证通过的契约是最可靠的参考

2. **确认登录接口**
   ```rust
   // 从签名工具代码中确认实际响应结构
   POST /api/v1/auth/login
   请求：{ "email": "...", "password": "..." }
   响应：{
       "access_token": "...",
       "expires_in": 3600,
       "user": { "id": 123, "username": "..." }
   }
   // 或是否有 data 包装？
   ```

3. **确认余额与用量接口**
   ```rust
   GET /api/v1/auth/me  或 GET /api/v1/user
   响应中的余额字段：balance? credits? quota? remaining?
   
   GET /api/v1/usage/stats
   响应结构和字段
   ```

4. **确认充值接口**
   ```rust
   POST /api/v1/redeem
   请求：{ "code": "..." }  或  { "redeemCode": "..." }？
   响应：成功/失败/重复兑换的判断标准
   ```

5. **创建契约文档**
   - 在 `docs/SUB2API-CONTRACT-REFERENCE.md` 记录实际契约
   - 包含请求/响应示例和字段说明

**验收标准：**
- 所有接口的请求/响应结构明确
- 契约文档与签名工具代码一致
- 错误码和业务状态码含义明确

#### 2.2 修复 `src-tauri/src/app/account.rs`

**执行步骤：**

1. **修复响应解析**（基于 2.1 确认的契约）
   - `login()`: 按实际响应结构解析 token
   - `status()`: 按实际字段解析余额
   - `usage()`: 按实际结构解析用量数据

2. **安全错误映射**
   - 创建 `fn safe_error_message(response: &str) -> String`
   - 不直接返回原始响应正文
   - 映射为安全的用户友好消息
   - 日志可记录详细错误,但不传给前端

3. **完善会话管理**
   ```rust
   // 会话应绑定 USB 授权身份
   struct Session {
       account_ref: String,
       usb_asset_id: String,  // 新增：绑定 U 盘
       token: String,
       expires_at: SystemTime,  // 新增：过期时间
   }
   ```

4. **处理生命周期事件**
   - 401 响应：清除会话,返回"会话已过期,请重新登录"
   - U 盘变化：检测 `account_ref` 不匹配时清除会话
   - 并发请求：使用 token 克隆避免竞态

5. **区分授权状态**
   ```rust
   pub enum AuthorizationState {
       Loading,           // 授权检查中
       NotBound,          // 许可证无账号绑定
       BoundNotLoggedIn,  // 有绑定但未登录
       LoggedIn,          // 已登录
       AuthInvalid,       // 授权失效（签名错误）
       NetworkUnavailable // 服务不可用
   }
   ```

**验收标准：**
- 响应解析使用实际字段名
- 错误消息不泄露凭据或后台详情
- 会话与 USB 授权身份绑定
- 401/换盘/拔盘场景处理正确
- 授权状态明确区分

#### 2.3 前端状态管理改进

**执行步骤：**

1. **修改 `src/app/views/AccountCard.tsx`**
   - 添加加载状态显示
   - 监听授权就绪事件,动态更新入口显示
   - 区分"加载中"、"未绑定"、"服务不可用"、"已登录"

2. **用量数据展示**
   - 根据 2.1 确认的响应结构,显示关键用量指标
   - 例如："今日 320 / 本月 8,460"（单位按后端实际）

3. **错误状态友好提示**
   - 网络错误："服务暂不可用,请稍后重试"
   - 密码错误："密码错误,请重新输入"
   - 授权失效："U 盘授权异常,请重新签名"

**验收标准：**
- 启动时授权未就绪不导致入口隐藏
- 加载/错误状态有明确 UI 反馈
- 用量数据正确展示

#### 2.4 补充测试

**执行步骤：**

1. **更新 Mock 文件**
   - `src/services/__tests__/managerApi.test.ts`
   - 按实际契约更新 Mock 响应
   - 移除 `managerApi.accountStatus?.()` 这类回退写法

2. **HTTP 契约测试**
   - 为 `account.rs` 添加单元测试
   - Mock reqwest 客户端
   - 测试响应解析、错误处理、会话管理

3. **组件测试**
   - `AccountCard.test.tsx` 补充场景：
     - 加载状态渲染
     - 登录成功/失败
     - 余额刷新
     - 充值流程
     - 错误状态显示

4. **集成测试**（可选,如时间允许）
   - 端到端测试：从启动管理器到登录成功

**验收标准：**
- 所有新测试通过
- Mock 契约与实际接口一致
- 测试覆盖关键场景（登录、刷新、充值、错误）

---

### 阶段 3：回归验证（P1 质量门）

**目标：** 确保新功能不破坏现有功能,所有场景稳定

#### 3.1 离线与网络异常场景

**测试用例：**

1. **完全离线**
   - 断网后启动管理器
   - 验证：Codex 本地安装不受阻,账户卡片显示"服务不可用"

2. **登录超时**
   - 模拟网络慢（使用防火墙延迟）
   - 验证：超时后显示友好错误,不卡死 UI

3. **服务端 5xx 错误**
   - 验证：显示"服务暂时不可用"而非崩溃

**验收标准：**
- 所有离线场景不阻塞 Codex 核心功能
- 错误提示友好且安全

#### 3.2 账号会话场景

**测试用例：**

1. **错误密码**
   - 输入错误密码
   - 验证：提示"密码错误",不锁定账号,可重试

2. **会话过期**
   - 登录后等待 token 过期（或手动清除后端 session）
   - 刷新余额
   - 验证：提示"会话已过期,请重新登录"

3. **换 U 盘**
   - 登录 U 盘 A
   - 拔出,插入 U 盘 B（不同 accountRef）
   - 验证：会话自动清除,显示 U 盘 B 的账号信息

4. **重复充值码**
   - 使用相同充值码两次
   - 验证：第二次显示"充值码已使用"或类似提示

**验收标准：**
- 所有会话场景处理正确
- 换盘不会误用其他账号的会话

#### 3.3 CCSwitch 启动场景

**测试用例：**

1. **有效绑定**
   - 使用已签名且有效的 U 盘
   - 验证：CCSwitch 自动配置成功,不弹 Key 输入框

2. **无绑定**
   - 使用旧版无 accountBinding 的许可证
   - 验证：显示"U 盘尚未绑定账号",CCSwitch 回退手动模式

3. **授权失效**
   - 使用签名损坏的许可证
   - 验证：显示"授权失效,请重新签名",不启动 CCSwitch

4. **CCSwitch 已运行**
   - CCSwitch 已启动时再次点击启动
   - 验证：不创建重复进程,或正确处理单实例逻辑

**验收标准：**
- 所有启动场景不弹 CMD/PowerShell 窗口
- 错误提示明确且可操作

#### 3.4 现有功能回归

**测试用例：**

1. **离线安装**
   - 完全断网,运行 Codex 安装
   - 验证：安装成功,Skill Hub、MCP 可用

2. **更新流程**
   - 检查更新、下载、安装
   - 验证：更新器不受账户模块影响

3. **卸载流程**
   - 卸载 Codex 和 CCSwitch
   - 验证：清理干净,不留下账户会话残留

4. **Skill Hub**
   - 浏览、安装、运行 Skill
   - 验证：功能正常

**验收标准：**
- 所有现有功能测试通过
- 无引入新的回归缺陷

---

### 阶段 4：文档纠正与最终打包（P2 交付项）

**目标：** 文档与代码一致,准备发布

#### 4.1 纠正 CCSwitch 补丁文档

**执行步骤：**

1. **更新 `docs/CCSWITCH-USB-ENV-RUNTIME-PATCH.md`**
   - 纠正版本溯源错误：
     - 移除"精确历史源码已构建"的说法
     - 说明实际使用的产品化源码位置（阶段 1.1 找到的路径）
     - 更新 `sourceVersion` 和 `sourceCommit` 为实际使用的值
   - 补充构建步骤和验收记录

2. **确认 manifest 与文档一致**
   - `manifest.json` 中的版本、commit、SHA256 与文档描述匹配

**验收标准：**
- 文档不包含错误的版本声称
- 构建过程可追溯

#### 4.2 更新总文档

**执行步骤：**

1. **更新 `docs/USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md`**
   - 状态改为："G39 完成：CCSwitch 自动配置闭环、管理器账户模块验收通过"
   - 移除"空密码随机生成"等已废弃描述
   - 确认默认密码规则：`12345678`（G38.1 确认）

2. **当前验收范围说明**
   - 明确哪些已实测：CCSwitch 闭环、账户登录/余额/充值
   - 明确哪些仅模拟：大规模并发、长期会话稳定性
   - 明确待用户实机确认：多 U 盘批量签名、生产环境后台压力

**验收标准：**
- 总文档与 G39 实际状态一致
- 无误导性描述

#### 4.3 重新打包与签名工具配对

**执行步骤：**

1. **生成新的管理器安装包**
   ```bash
   # 假设构建脚本
   npm run build
   cargo build --release
   # 打包逻辑...
   ```

2. **确认签名工具与安装包发行密钥一致**
   - 检查签名工具和管理器使用同一套公私钥对
   - 验证签名工具签名的许可证能被新管理器验证

3. **版本号协调**
   - 如果是正式发布,按 `CLAUDE.md` 规范 bump 版本号（5 个文件 6 处）
   - 如果是内部测试,保持版本号或使用 `-alpha`/`-beta` 后缀

**验收标准：**
- 新安装包包含更新后的 CCSwitch
- 签名工具与安装包密钥配对
- 版本号统一

#### 4.4 创建 G39 完成总结文档

**执行步骤：**

1. **创建 `docs/G39-COMPLETION-SUMMARY.md`**
   - 列出本阶段完成的所有功能
   - 列出验证通过的场景
   - 列出已知限制和待优化项
   - 提供下一步建议（如生产部署前的压力测试）

2. **附录测试报告**
   - CCSwitch 闭环验证截图或日志
   - 账户功能测试用例结果
   - 回归测试通过记录

**验收标准：**
- 完成文档清晰记录交付范围
- 与实际验证一致

---

## 3. 风险与依赖

### 3.1 关键依赖

| 依赖项 | 类型 | 阻塞阶段 | 缓解措施 |
|---|---|---|---|
| 找到产品化 CCSwitch 源码 | 外部 | 阶段 1.1 | 如果找不到,需用户提供或从临时构建逆向 |
| Sub2API 后台可用性 | 外部 | 阶段 2.1 | 使用签名工具已验证的契约,降低风险 |
| 测试 U 盘可用性 | 外部 | 阶段 1.4 | 可用现有测试账号或创建新测试许可证 |

### 3.2 风险识别

| 风险 | 影响 | 概率 | 应对策略 |
|---|---|---|---|
| 找不到产品化源码 | 🔴 高 | 中 | 询问用户;或从旧版本 manifest 的 commit 重建 |
| CCSwitch 新版本与现有功能不兼容 | 🔴 高 | 低 | 充分回归测试（阶段 3） |
| Sub2API 接口变更 | 🟡 中 | 低 | 签名工具已验证,变更概率低 |
| 账户模块性能问题 | 🟢 低 | 中 | 使用连接池和超时,先验收功能再优化性能 |

### 3.3 阶段间依赖

```
阶段 0 (前置准备)
    ↓
阶段 1 (CCSwitch 闭环) ← 🔴 阻塞：必须完成才能端到端验证
    ↓
阶段 2 (账户模块) ← 可与阶段 1 部分并行（2.1 契约核实可先做）
    ↓
阶段 3 (回归验证) ← 必须阶段 1、2 完成
    ↓
阶段 4 (文档与打包) ← 必须阶段 3 通过
```

---

## 4. 时间估算（仅供参考）

| 阶段 | 预计工作量 | 备注 |
|---|---|---|
| 阶段 0 | 0.5-1 小时 | 取决于差异清单复杂度 |
| 阶段 1 | 4-8 小时 | **关键路径**：取决于找源码时间 |
| 阶段 2 | 6-10 小时 | 契约核实 + 代码修复 + 测试 |
| 阶段 3 | 4-6 小时 | 回归测试较耗时 |
| 阶段 4 | 2-3 小时 | 文档更新和打包 |
| **总计** | **16.5-28 小时** | 约 2-4 个工作日 |

**注意：** 这是纯开发时间,不包括等待用户确认、后台服务问题排查等外部因素。

---

## 5. 下一步行动

### 立即执行（现在）

1. **用户确认：** 任务 `01a04f20-1c47-7060-a966-e2f88664a3f1` 是否已停止
2. **运行 `git status`** 确认工作区差异归属

### 第一优先级（今日）

3. **搜索产品化 CCSwitch 源码**（阶段 1.1）
4. **找到后立即实现环境变量补丁**（阶段 1.2）

### 第二优先级（明日）

5. **构建新 CCSwitch 并完成闭环验证**（阶段 1.3-1.4）
6. **核实 Sub2API 契约**（阶段 2.1）

### 后续优先级

7. **修复账户模块**（阶段 2.2-2.4）
8. **完整回归测试**（阶段 3）
9. **文档纠正与打包**（阶段 4）

---

## 6. 成功标准

G39 阶段成功的明确标准：

### 6.1 功能标准

- ✅ 用户插入已签名 U 盘,启动管理器,点击"启动 CCSwitch",**不需要手动输入 API Key**,Codex 直接可用
- ✅ 用户在账户卡片输入密码,点击登录,能看到**真实余额和用量**（不是猜测的字段）
- ✅ 用户输入充值码,点击充值,余额正确增加
- ✅ U 盘拔出或换盘时,会话自动清除,不误用其他账号信息
- ✅ 网络不可用时,Codex 本地功能（安装、Skill Hub）不受影响
- ✅ CCSwitch、日志、配置文件中**不包含 deviceApiKey 明文**

### 6.2 质量标准

- ✅ 所有 Rust 单元测试通过
- ✅ 所有 TypeScript 测试通过
- ✅ `cargo clippy` 无新增警告（基线警告可暂存）
- ✅ `npx tsc --noEmit` 通过
- ✅ `verify-product.ps1` CCSwitch 验证通过
- ✅ 回归测试覆盖：离线、超时、错误密码、换盘、CCSwitch 启动等 15+ 场景

### 6.3 文档标准

- ✅ `CCSWITCH-USB-ENV-RUNTIME-PATCH.md` 不包含错误的版本溯源
- ✅ 总文档状态描述与实际验收范围一致
- ✅ G39 完成总结文档包含测试证据

---

## 7. 与历史文档的兼容性

本路线图与以下文档完全兼容：

- **[USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md](./USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md)**：总方案不变,本路线图是完成剩余部分的执行计划
- **[USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md](./USB-SUB2API-ONE-CLICK-PROVISIONING-DEVELOPMENT.md)**：签名工具部分已完成（G38.1）,本路线图聚焦管理器与 CCSwitch
- **[USB-SUB2API-RECOVERY-G38.2-DEVELOPMENT.md](./USB-SUB2API-RECOVERY-G38.2-DEVELOPMENT.md)**：预充值重试已完成,不在本路线图范围
- **[CCSWITCH-USB-ENV-RUNTIME-PATCH.md](./CCSWITCH-USB-ENV-RUNTIME-PATCH.md)**：补丁规范保持不变,但会纠正其中的版本溯源错误
- **[CLAUDE.md](../CLAUDE.md)**：发版流程、质量门、提交规范等项目规范保持不变

---

## 附录 A：关键文件清单

### A.1 需要修改的文件

| 文件 | 修改内容 | 阶段 |
|---|---|---|
| `src-tauri/src/app/account.rs` | 修复契约解析、安全错误、会话管理 | 2.2 |
| `src/app/views/AccountCard.tsx` | 状态管理、用量展示、错误提示 | 2.3 |
| `src/services/__tests__/managerApi.test.ts` | 更新 Mock,补充测试 | 2.4 |
| `src-tauri/resources/ccswitch/CodexSwitch.exe` | 替换为新构建 | 1.3 |
| `src-tauri/resources/ccswitch/manifest.json` | 更新 SHA256 和版本 | 1.3 |
| `docs/CCSWITCH-USB-ENV-RUNTIME-PATCH.md` | 纠正版本溯源 | 4.1 |
| `docs/USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md` | 更新状态为 G39 完成 | 4.2 |

### A.2 需要创建的文件

| 文件 | 内容 | 阶段 |
|---|---|---|
| `docs/SUB2API-CONTRACT-REFERENCE.md` | 接口契约参考 | 2.1 |
| `docs/G39-COMPLETION-SUMMARY.md` | G39 完成总结 | 4.4 |

### A.3 CCSwitch 源码（待定位）

| 预期位置 | 类型 | 说明 |
|---|---|---|
| `D:\AI\AGENT_U盘\codex-desktop-usb-installer\ccswitch-source\` | 可能 | 最可能的位置 |
| 其他项目目录 | 可能 | 需用户确认或搜索 |
| `C:\Users\T14S\AppData\Local\Temp\cc-switch-source-review-*` | 临时 | 仅供参考,不应作为构建源 |

---

## 附录 B：验收检查清单

使用此清单确保每个阶段完成后再进入下一阶段。

### 阶段 0 检查清单

- [ ] 用户确认无并行任务修改
- [ ] `git status` 和 `git diff --stat` 输出已记录
- [ ] CCSwitch 当前 SHA256 确认为 `75B847CA...`
- [ ] `manifest.json` 和 `SOURCE-NOTICE.txt` 已备份

### 阶段 1 检查清单

- [ ] 找到产品化 CCSwitch 源码,路径：\_\_\_\_\_\_\_\_\_\_
- [ ] 源码包含 404token provider 预设
- [ ] 源码包含关闭上游更新逻辑
- [ ] 环境变量补丁代码已添加
- [ ] `cargo check --locked` 通过
- [ ] `cargo build --release` 成功
- [ ] 新 EXE SHA256：\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_
- [ ] `manifest.json` 已更新
- [ ] `verify-product.ps1` 通过
- [ ] CCSwitch 启动不弹 Key 输入框（有绑定时）
- [ ] Codex 请求使用注入的 Key（已验证）
- [ ] Key 不存在于数据库或配置文件（已验证）
- [ ] 无绑定时正常降级（已验证）

### 阶段 2 检查清单

- [ ] `docs/SUB2API-CONTRACT-REFERENCE.md` 已创建
- [ ] 登录接口契约明确
- [ ] 余额/用量接口契约明确
- [ ] 充值接口契约明确
- [ ] `account.rs` 响应解析已修复
- [ ] 安全错误映射已实现
- [ ] 会话绑定 USB 授权身份
- [ ] 401/换盘/拔盘场景已处理
- [ ] 授权状态明确区分（6 种状态）
- [ ] `AccountCard.tsx` 加载状态已添加
- [ ] 用量数据已展示
- [ ] Mock 文件已更新
- [ ] HTTP 契约测试已添加
- [ ] 组件测试已补充
- [ ] 所有测试通过

### 阶段 3 检查清单

- [ ] 完全离线测试通过
- [ ] 登录超时测试通过
- [ ] 服务端 5xx 测试通过
- [ ] 错误密码测试通过
- [ ] 会话过期测试通过
- [ ] 换 U 盘测试通过
- [ ] 重复充值码测试通过
- [ ] CCSwitch 有效绑定测试通过
- [ ] CCSwitch 无绑定测试通过
- [ ] CCSwitch 授权失效测试通过
- [ ] CCSwitch 已运行单实例测试通过
- [ ] 离线安装测试通过
- [ ] 更新流程测试通过
- [ ] 卸载流程测试通过
- [ ] Skill Hub 测试通过

### 阶段 4 检查清单

- [ ] `CCSWITCH-USB-ENV-RUNTIME-PATCH.md` 已纠正
- [ ] `USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md` 状态已更新
- [ ] 总文档默认密码规则已确认
- [ ] 新安装包已生成
- [ ] 签名工具与安装包密钥配对验证通过
- [ ] 版本号已协调
- [ ] `docs/G39-COMPLETION-SUMMARY.md` 已创建
- [ ] 测试报告已附录

---

**文档结束**

本路线图将随执行进度更新。每个阶段完成后,在对应检查清单打勾,并记录实际遇到的问题和解决方案。
