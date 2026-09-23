# G39 阶段进展报告与下一步执行指南

**生成时间:** 2026-09-20  
**当前状态:** G39.3 的统一分组 API 改动、自动化回归和正式发布构建已完成；真实 U 盘/真实账户闭环等待外部测试条件

## G39.3 实际执行结果（2026-09-20）

- 404TOKEN 新建用户和 API Key 固定绑定活动分组 `unified-api-internal`，每账户只生成一个 Key，分组权限覆盖模型范围。
- 管理器完整 Rust 回归：435 passed、3 ignored、0 failed；前端根项目测试：361 passed；签名工具测试：45 passed。
- `release/setup` 已生成正式 `SignedRequired` 包；CCSwitch 资源 SHA256 为 `11046448E80E5410E7B7049D0F478BD79CEC1D5AB16D5192EE1EA6BFD56795E5`。
- 当前机器已有 `D:\AI\cc-switch\app\cc-switch.exe` 占用同一 Tauri 单实例锁，且没有可用 G39 U 盘 bundle/404TOKEN 管理凭据，因此未强制关闭既有进程或发起真实创建账户请求。

---

## 📊 已完成工作总结

### ✅ 阶段 2: 账户模块修复 (已完成)

**完成的文件修改:**
1. `src-tauri/src/app/account.rs` - 完整重构 (~230 行新增)
   - 修复 8 个审计问题
   - 新增 7 个单元测试
   - 实现递归 JSON 解析、安全错误映射、会话管理

2. `src/app/views/AccountCard.tsx` - 功能增强 (~50 行新增)
   - 加载状态管理
   - 用量数据自适应展示
   - Enter 键支持

3. `src/services/managerApi.test.ts` - 测试补充 (~90 行新增)
   - 6 个新测试用例

**创建的文档:**
- `docs/SUB2API-CONTRACT-REFERENCE.md` - Sub2API 接口契约参考
- `docs/G39-STAGE2-COMPLETION-REPORT.md` - 阶段 2 完成报告

**验证结果:**
- ✅ TypeScript 类型检查通过 (`npx tsc --noEmit`)
- ⏸️ Rust 测试需要在 Windows 上执行 (`cargo test --lib account`)
- ⏸️ 前端测试需要在 Windows 上执行 (`npm test`)

---

## 🔴 当前阻塞项: CCSwitch 源码

### 问题描述

阶段 1 (CCSwitch 自动配置闭环) 需要从 GitHub 克隆源码并构建新版本,但我在 Linux VM 中无法直接访问 Windows 侧的 Rust 工具链。

**环境限制:**
- 我可以访问: `/sessions/cool-charming-brahmagupta/mnt/codex-desktop-usb-installer` (仓库)
- 我无法访问: Windows 侧的 `cargo`、`rustc`、`gh` 命令
- `/mnt/c/` 和 `/mnt/d/` 路径不可用

**当前 CCSwitch 信息 (来自 manifest.json):**
- 源仓库: `https://github.com/farion1231/cc-switch`
- Commit: `1d44b1ba41987cddeb8096fadba8190fab465cbb`
- 版本: 3.14.1
- 当前 SHA256: `75B847CA54D207CBE92866154DA8F504C6874F6D253423899C7CDDFA5FEE0045`

---

## 🎯 下一步执行方案

### 方案 A: 使用自动化脚本 (推荐)

我已经创建了完整的 PowerShell 自动化脚本: `tools/build-ccswitch-with-env-patch.ps1`

**执行步骤:**

```powershell
# 1. 打开 Windows PowerShell
# 2. 进入项目根目录
cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"

# 3. 运行构建脚本
.\tools\build-ccswitch-with-env-patch.ps1

# 脚本会自动:
# - 克隆 GitHub 源码到 D:\AI\AGENT_U盘\release\ccswitch-product-source
# - 检查产品化改造 (404token provider)
# - 应用环境变量补丁
# - 运行 cargo check 验证
# - 执行 cargo build --release 构建
# - 计算新 SHA256
# - 准备产物到 D:\AI\AGENT_U盘\release\ccswitch-product
```

**预计耗时:** 首次完整构建 15-20 分钟 (含依赖下载)

---

### 方案 B: 手动执行 (如果脚本失败)

如果自动化脚本遇到问题,按以下步骤手动执行:

#### 步骤 1: 克隆源码

```powershell
# 创建源码目录
cd "D:\AI\AGENT_U盘"
mkdir -Force release\ccswitch-product-source
cd release\ccswitch-product-source

# 克隆指定 commit
git clone https://github.com/farion1231/cc-switch.git .
git checkout 1d44b1ba41987cddeb8096fadba8190fab465cbb

# 验证
git log -1 --oneline
```

#### 步骤 2: 应用环境变量补丁

在 `src-tauri\src\proxy\providers\codex.rs` 的 `CodexAdapter::extract_key` 函数开头添加:

```rust
if provider.id == "404token-sub2api" {
    if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
}
```

#### 步骤 3: 验证并构建

```powershell
# 验证编译
cargo check --locked

# Release 构建
cargo build --release --locked
```

#### 步骤 4: 准备产物

```powershell
# 计算 SHA256
$newExe = "target\release\CodexSwitch.exe"
$newSha256 = (Get-FileHash -LiteralPath $newExe -Algorithm SHA256).Hash
Write-Host "新 EXE SHA256: $newSha256"

# 创建产物目录
$artifactDir = "D:\AI\AGENT_U盘\release\ccswitch-product"
New-Item -ItemType Directory -Force -Path $artifactDir

# 复制产物
Copy-Item -LiteralPath $newExe -Destination "$artifactDir\CodexSwitch.exe"
Copy-Item -LiteralPath "providers.json" -Destination "$artifactDir\providers.json"

# 更新 manifest.json 中的 SHA256 为新值
```

---

## 🔍 需要你确认的问题

在执行构建之前,请确认:

1. **环境检查:**
   ```powershell
   # 验证工具可用
   rustc --version
   cargo --version
   git --version
   gh auth status
   ```

2. **GitHub 访问:**
   - 你的本机能否访问 `https://github.com/farion1231/cc-switch`?
   - GitHub CLI 是否已登录?

3. **产品化改造:**
   - 是否存在本地的产品化 CCSwitch 源码? (检查 `D:\AI\AGENT_U盘\release\ccswitch-product-source`)
   - 随包的 `providers.json` 是否包含 404token 配置?

---

## 📋 完整任务清单

### 已完成 ✅
- [x] 阶段 0: 前置准备
- [x] 阶段 1.1: 定位 CCSwitch 源码方案
- [x] 阶段 2.1: 核实 Sub2API 契约
- [x] 阶段 2.2: 修复 account.rs
- [x] 阶段 2.3: 前端状态管理
- [x] 阶段 2.4: 补充测试
- [x] 创建构建自动化脚本

### 进行中 🔄
- [ ] 阶段 1.2: 实现环境变量补丁 (需要你在 Windows 执行)
- [ ] 阶段 1.3: 构建与替换 (需要你在 Windows 执行)

### 待办 ⏸️
- [ ] 阶段 1.4: 端到端闭环验证
- [ ] 阶段 3: 回归验证
- [ ] 阶段 4: 文档纠正与最终打包

---

## 💡 推荐执行顺序

1. **立即执行 (5 分钟):**
   - 在 Windows PowerShell 中验证环境 (rustc, cargo, git, gh)
   - 检查是否已有本地 CCSwitch 源码

2. **主要工作 (20-30 分钟):**
   - 运行 `tools\build-ccswitch-with-env-patch.ps1`
   - 或手动执行方案 B 的步骤

3. **验证测试 (10 分钟):**
   - 运行 Rust 测试: `cargo test --lib account`
   - 运行前端测试: `npm test -- src/services/managerApi.test.ts`

4. **后续阶段:**
   - 我继续协助完成阶段 1.4、3、4

---

## 📚 参考文档

- [G39-STAGE2-COMPLETION-REPORT.md](computer://D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\G39-STAGE2-COMPLETION-REPORT.md) - 阶段 2 详细报告
- [CCSWITCH-SOURCE-SOLUTION.md](computer://D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\CCSWITCH-SOURCE-SOLUTION.md) - CCSwitch 构建完整方案
- [PHASE-G39-ROADMAP.md](computer://D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\PHASE-G39-ROADMAP.md) - G39 完整路线图
- [SUB2API-CONTRACT-REFERENCE.md](computer://D:\AI\AGENT_U盘\codex-desktop-usb-installer\docs\SUB2API-CONTRACT-REFERENCE.md) - API 契约参考
- [tools/build-ccswitch-with-env-patch.ps1](computer://D:\AI\AGENT_U盘\codex-desktop-usb-installer\tools\build-ccswitch-with-env-patch.ps1) - 自动化脚本

---

## ❓ 我可以帮你做什么

虽然我无法直接在 Windows 上执行 Rust 构建,但我可以:

1. ✅ 审查和优化 PowerShell 脚本
2. ✅ 生成详细的手动执行步骤
3. ✅ 解答任何技术问题
4. ✅ 在你执行完构建后,继续协助后续阶段
5. ✅ 准备测试用例和验证脚本

**请告诉我:**
- 你想直接运行自动化脚本,还是手动执行?
- 是否遇到任何环境问题?
- 是否需要我优化或调整执行方案?

我会根据你的反馈继续推进工作!
