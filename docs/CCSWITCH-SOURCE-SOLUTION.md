# CCSwitch 产品化源码获取与构建方案

**生成时间:** 2026-09-20  
**状态:** 待执行  
**相关任务:** G39 阶段 1 - CCSwitch 自动配置闭环

---

## 1. 问题概述

当前随包的 `CodexSwitch.exe` (SHA256: `75B847CA...`) 是旧版本,不会读取环境变量。虽然管理器已在 `ccswitch.rs` 中注入环境变量,但 CCSwitch 不会自动使用。

**阻塞原因:**
- 产品化 CCSwitch 源码位置未找到
- 本地仅存在编译后的 EXE 二进制文件 (161 个不同版本的发布包)
- 构建脚本期望外部目录 `D:\AI\AGENT_U盘\release\ccswitch-product\`,但该目录不在当前挂载路径

**关键信息 (来自 manifest.json):**
```json
{
  "sourceRepository": "https://github.com/farion1231/cc-switch",
  "sourceCommit": "1d44b1ba41987cddeb8096fadba8190fab465cbb",
  "sourceVersion": "3.14.1",
  "artifact": {
    "file": "CodexSwitch.exe",
    "sha256": "75B847CA54D207CBE92866154DA8F504C6874F6D253423899C7CDDFA5FEE0045"
  }
}
```

---

## 2. 推荐方案：从 GitHub 重建产品化源码

### 2.1 方案概述

从上游仓库克隆指定 commit,应用产品化改造,实现环境变量补丁,然后构建新版本。

**优点:**
- 完全可追溯,基于确定的源码版本
- 避免寻找可能不存在的本地源码
- 确保产品化改造的完整性

**缺点:**
- 需要重新应用产品化改造 (404token provider 预设、关闭上游更新、Codex 专用界面)
- 构建时间较长 (~10-15 分钟)

### 2.2 执行步骤

#### 步骤 1: 克隆源码到指定位置

```powershell
# 在项目父目录创建 CCSwitch 源码目录
cd D:\AI\AGENT_U盘
mkdir -Force release\ccswitch-product-source
cd release\ccswitch-product-source

# 克隆指定 commit
git clone https://github.com/farion1231/cc-switch.git .
git checkout 1d44b1ba41987cddeb8096fadba8190fab465cbb

# 验证版本
git log --oneline -1
# 应显示: 1d44b1b <commit message>
```

#### 步骤 2: 应用产品化改造

**必须的改造项 (来自 SOURCE-NOTICE.txt):**
1. 默认仅展示 Codex
2. 预置 404TOKEN (Sub2API) 供应商
3. 关闭上游更新源
4. 保留用户运行时输入 API Key 的配置流程

**具体文件修改:**

**(A) 添加 404token provider 预设**

需要检查当前 `providers.json` 的结构,然后添加或确认包含:

```json
{
  "id": "404token-sub2api",
  "name": "404TOKEN (Sub2API)",
  "type": "codex",
  "endpoint": "https://404token.xyz/v1",
  "description": "404token Sub2API relay service",
  "requiresApiKey": true
}
```

**(B) 关闭上游更新**

查找更新检查相关代码 (通常在 `src-tauri/src/updater.rs` 或类似路径),禁用或移除更新检查逻辑。

**(C) 默认仅展示 Codex**

修改界面配置或过滤逻辑,确保只显示 Codex 相关 provider。

**⚠️ 重要:** 这些改造的具体实现细节需要查看源码后确定。当前随包的 EXE 已经包含这些改造,但我们无法直接从二进制文件逆向获取改造细节。

**建议:** 
1. 先克隆源码
2. 对比当前随包 `providers.json` 与克隆后的版本
3. 查看源码中的配置文件和构建脚本,确认改造点
4. 参考 `.learnings/` 或文档中是否有历史改造记录

#### 步骤 3: 实现环境变量补丁

按照 `CCSWITCH-USB-ENV-RUNTIME-PATCH.md` 的规范,在 `src-tauri/src/proxy/providers/codex.rs` 的 `CodexAdapter::extract_key` 函数最前面添加:

```rust
if provider.id == "404token-sub2api" {
    if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
}
```

**验证补丁:**
```powershell
cargo check --locked
```

#### 步骤 4: Release 构建

```powershell
# 在源码目录
cargo build --release --locked

# 构建成功后,可执行文件位于
# target/release/CodexSwitch.exe
```

**预计构建时间:** 10-15 分钟 (首次构建,含依赖下载)

#### 步骤 5: 计算 SHA256 并准备产物

```powershell
# 计算新 EXE 的 SHA256
$newExe = "target\release\CodexSwitch.exe"
$newSha256 = (Get-FileHash -LiteralPath $newExe -Algorithm SHA256).Hash
Write-Host "新 EXE SHA256: $newSha256"

# 创建产物目录
$artifactDir = "D:\AI\AGENT_U盘\release\ccswitch-product"
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null

# 复制产物
Copy-Item -LiteralPath $newExe -Destination "$artifactDir\CodexSwitch.exe"
Copy-Item -LiteralPath "providers.json" -Destination "$artifactDir\providers.json"

# 复制验证脚本 (如果存在)
if (Test-Path "verify-product.ps1") {
    Copy-Item -LiteralPath "verify-product.ps1" -Destination "$artifactDir\verify-product.ps1"
}
```

#### 步骤 6: 更新 manifest.json

编辑 `codex-desktop-usb-installer\src-tauri\resources\ccswitch\manifest.json`:

```json
{
  "schemaVersion": 1,
  "product": "codex-usb",
  "displayName": "Codex Switch",
  "sourceRepository": "https://github.com/farion1231/cc-switch",
  "sourceCommit": "1d44b1ba41987cddeb8096fadba8190fab465cbb",
  "sourceVersion": "3.14.1",
  "artifact": {
    "file": "CodexSwitch.exe",
    "sha256": "<新的 SHA256>",
    "kind": "portable-executable",
    "installerBundle": false
  },
  "providerCatalog": "providers.json",
  "secretsIncluded": false,
  "notes": [
    "404TOKEN is a third-party Sub2API relay, not an OpenAI official endpoint.",
    "Environment variable CCSWITCH_404TOKEN_DEVICE_KEY is read at runtime for automatic configuration.",
    "The first launch without environment variable opens a 404TOKEN API Key dialog.",
    "The Windows MSI/NSIS wrapper is not included in this P0 artifact."
  ]
}
```

**关键变更:**
1. 更新 `artifact.sha256` 为新计算的值
2. 新增 notes 说明环境变量支持

#### 步骤 7: 运行构建验证

```powershell
cd D:\AI\AGENT_U盘\codex-desktop-usb-installer

# 使用新产物构建管理器
.\tools\build-installer.ps1 -CcswitchArtifactDir "D:\AI\AGENT_U盘\release\ccswitch-product" -SkipTests -UsbPolicy DevelopmentAutoEnroll

# 如果有 verify-product.ps1
pwsh.exe -NoProfile -File "D:\AI\AGENT_U盘\release\ccswitch-product\verify-product.ps1" -ArtifactDir "D:\AI\AGENT_U盘\release\ccswitch-product" -SkipNetwork
```

---

## 3. 备选方案：搜索本地已有源码

### 3.1 可能的位置

根据构建脚本和项目结构,本地源码可能在:

1. `D:\AI\AGENT_U盘\release\ccswitch-product-source\`
2. `D:\AI\AGENT_U盘\ccswitch-source\`
3. `C:\Projects\cc-switch\`
4. `D:\Projects\cc-switch\`
5. 用户桌面或文档目录

### 3.2 搜索命令

```powershell
# 搜索包含 Cargo.toml 且提及 cc-switch 的目录
Get-ChildItem -Path D:\AI\AGENT_U盘 -Recurse -Filter "Cargo.toml" -ErrorAction SilentlyContinue | Where-Object {
    (Get-Content $_.FullName -Raw) -match "cc-switch|ccswitch|CodexSwitch"
} | Select-Object Directory

# 搜索 git 仓库
Get-ChildItem -Path D:\AI\AGENT_U盘 -Recurse -Directory -Filter ".git" -ErrorAction SilentlyContinue | ForEach-Object {
    $repoDir = $_.Parent.FullName
    Push-Location $repoDir
    $remote = git remote get-url origin 2>$null
    if ($remote -match "cc-switch") {
        Write-Host "Found: $repoDir"
        Write-Host "  Remote: $remote"
        Write-Host "  Commit: $(git log -1 --oneline)"
    }
    Pop-Location
}
```

### 3.3 验证找到的源码

如果找到本地源码,验证:

```powershell
cd <找到的源码目录>

# 检查 commit
git log -1 --oneline
# 应包含或接近 1d44b1ba

# 检查是否有产品化改造
Get-Content providers.json | Select-String "404token"
# 应找到 404token-sub2api provider

# 检查是否可编译
cargo check --locked
```

**如果验证通过:** 在该目录应用步骤 3 (环境变量补丁) 和步骤 4-7 (构建与验证)。

---

## 4. 临时构建记录分析 (不推荐)

在总结会话历史中提到存在临时构建记录:
```
C:\Users\T14S\AppData\Local\Temp\cc-switch-source-review-f2abd0aeaaa641db8c080ed25525947e
```

**问题:**
- 构建版本为 3.20.3,与声称的 3.14.1 不匹配
- 没有证据证明切到了 commit `1d44b1ba`
- 临时替换后验证失败,已回滚

**结论:** 临时构建记录**不可靠**,不应作为构建源。

---

## 5. 风险评估与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| 产品化改造细节未知 | 中 | 🔴 高 | 对比随包 providers.json,查看源码配置文件 |
| 构建失败 (依赖问题) | 低 | 🟡 中 | 使用 `--locked` 标志,确保依赖版本固定 |
| 新 EXE 与现有功能不兼容 | 低 | 🔴 高 | 充分回归测试 (阶段 3) |
| GitHub 仓库不可用 | 低 | 🔴 高 | 提前克隆并本地备份 |

---

## 6. 时间估算

| 步骤 | 预计时间 | 备注 |
|------|----------|------|
| 克隆源码 | 5-10 分钟 | 含网络下载 |
| 分析产品化改造 | 30-60 分钟 | 需要对比和查看源码 |
| 应用改造 + 补丁 | 20-30 分钟 | 手动修改代码 |
| 首次构建 | 10-15 分钟 | 含依赖下载 |
| 验证与测试 | 20-30 分钟 | 运行验证脚本 |
| **总计** | **1.5-2.5 小时** | 首次完整流程 |

**后续构建:** 如果产品化改造已固化,后续补丁 + 构建仅需 10-15 分钟。

---

## 7. 推荐执行顺序

**优先级 P0 (立即执行):**
1. 搜索本地已有源码 (备选方案 3.2)
   - 如果找到且验证通过 → 直接应用补丁 (步骤 3-7)
   - 如果未找到或验证失败 → 执行推荐方案

**优先级 P1 (推荐方案):**
2. 从 GitHub 克隆源码 (步骤 1)
3. 分析并应用产品化改造 (步骤 2)
4. 实现环境变量补丁 (步骤 3)
5. 构建与验证 (步骤 4-7)

**优先级 P2 (闭环验证):**
6. 端到端测试 (PHASE-G39-ROADMAP.md § 阶段 1.4)

---

## 8. 决策点

**需要用户确认的问题:**

1. **本地源码搜索:**
   - 是否存在本地产品化 CCSwitch 源码?
   - 如果存在,位置在哪里?

2. **产品化改造细节:**
   - 是否有文档记录历史改造的具体细节?
   - 随包 `providers.json` 是否是最新的产品化版本?

3. **构建环境:**
   - Windows 宿主机是否已安装 Rust 工具链?
   - 是否有网络访问 GitHub 和 crates.io?

**如果用户未响应:** 按照自主决策授权,我将执行推荐方案 (从 GitHub 重建)。

---

## 9. 下一步行动 (自主决策)

根据用户授权 ("后面再遇到需要决策的问题,按你的推荐直接决定,无需经过我"),我将:

1. **立即执行:** 快速搜索本地源码 (5 分钟内完成)
2. **如果未找到:** 开始执行推荐方案 - 从 GitHub 克隆并重建
3. **持续推进:** 完成构建后立即进入阶段 1.4 (端到端闭环验证)

**当前状态:** 阶段 1.1 已完成,正在准备执行阶段 1.2。

---

## 10. 参考资料

- [PHASE-G39-ROADMAP.md](./PHASE-G39-ROADMAP.md) - G39 完整路线图
- [CCSWITCH-USB-ENV-RUNTIME-PATCH.md](./CCSWITCH-USB-ENV-RUNTIME-PATCH.md) - 补丁规范
- [G39-STAGE2-COMPLETION-REPORT.md](./G39-STAGE2-COMPLETION-REPORT.md) - 阶段 2 完成报告
- `src-tauri/resources/ccswitch/manifest.json` - 当前版本信息
- `src-tauri/resources/ccswitch/SOURCE-NOTICE.txt` - 来源说明
- `tools/build-installer.ps1` - 构建脚本 (第 49-120 行)

---

**文档生成:** 2026-09-20  
**下一个文档:** 构建执行日志 (待生成)
