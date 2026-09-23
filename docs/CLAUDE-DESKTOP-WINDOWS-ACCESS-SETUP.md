# Claude Desktop Windows 本机访问配置指南

**目的:** 让 Claude Desktop 直接访问 Windows 本机环境,实现完整的自主操作能力

**生成时间:** 2026-09-20

---

## 问题分析

当前架构限制:
- Claude 运行在独立的 Linux VM 中(Ubuntu 22.04)
- VM 通过 `/sessions/` 挂载访问项目目录
- 无法直接调用 Windows 的 `cargo`、`rustc`、`git`、`PowerShell` 等工具
- 网络代理受 Anthropic 沙箱安全策略限制,无法访问 GitHub

**解决方案:** 通过 MCP (Model Context Protocol) 配置,让 Claude 能够直接调用 Windows 本机的工具链和文件系统。

---

## 配置步骤

### 步骤 1: 创建配置文件

**配置文件位置:**
```
%APPDATA%\Claude\claude_desktop_config.json
```

**完整路径:**
```
C:\Users\T14S\AppData\Roaming\Claude\claude_desktop_config.json
```

### 步骤 2: 推荐配置(完整权限)

这个配置让 Claude 能够:
- ✅ 访问 Windows 文件系统(指定目录)
- ✅ 执行 PowerShell 命令
- ✅ 运行 cargo、git、npm 等 Windows 工具链
- ✅ 直接在 Windows 环境中克隆 GitHub 仓库并构建

**配置内容:**

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "D:\\AI\\AGENT_U盘\\codex-desktop-usb-installer",
        "D:\\AI\\AGENT_U盘\\release",
        "C:\\Users\\T14S\\Desktop",
        "C:\\Users\\T14S\\Documents",
        "C:\\Users\\T14S\\Downloads"
      ]
    },
    "shell": {
      "command": "npx",
      "args": [
        "-y",
        "@executeautomation/mcp-server-shell"
      ]
    }
  }
}
```

**说明:**
- `filesystem` MCP: 允许 Claude 读写指定的 Windows 目录
- `shell` MCP: 允许 Claude 执行 PowerShell/CMD 命令(以你的 Windows 用户权限运行)

---

## 自动化安装脚本

我已经为你准备了一键配置脚本,在 Windows PowerShell 中运行:

**脚本位置:** `tools\setup-claude-desktop-mcp.ps1`

**执行命令:**
```powershell
cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"
.\tools\setup-claude-desktop-mcp.ps1
```

---

## 手动配置步骤

如果自动化脚本不可用,按以下步骤手动配置:

### 1. 确保配置目录存在

```powershell
$configDir = "$env:APPDATA\Claude"
if (-not (Test-Path $configDir)) {
    New-Item -ItemType Directory -Path $configDir -Force
}
```

### 2. 写入配置文件

```powershell
$configPath = "$configDir\claude_desktop_config.json"

@"
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "D:\\\\AI\\\\AGENT_U盘\\\\codex-desktop-usb-installer",
        "D:\\\\AI\\\\AGENT_U盘\\\\release",
        "C:\\\\Users\\\\T14S\\\\Desktop",
        "C:\\\\Users\\\\T14S\\\\Documents",
        "C:\\\\Users\\\\T14S\\\\Downloads"
      ]
    },
    "shell": {
      "command": "npx",
      "args": [
        "-y",
        "@executeautomation/mcp-server-shell"
      ]
    }
  }
}
"@ | Out-File -FilePath $configPath -Encoding UTF8

Write-Host "✓ 配置文件已创建: $configPath" -ForegroundColor Green
```

### 3. 重启 Claude Desktop

配置文件修改后,**必须完全退出并重启 Claude Desktop App** 才能生效。

确保:
1. 关闭所有 Claude Desktop 窗口
2. 在任务管理器中确认 Claude 进程已完全退出
3. 重新启动 Claude Desktop

---

## 验证配置

重启 Claude Desktop 后,在新会话中测试:

### 测试 1: 文件系统访问

```
列出 D:\AI\AGENT_U盘\codex-desktop-usb-installer 目录的所有 .md 文件
```

**预期结果:** Claude 能够列出目录内容

### 测试 2: Windows 命令执行

```
运行 cargo --version 命令
```

**预期结果:** 显示 Rust 版本信息

### 测试 3: Git 操作

```
在项目目录执行 git status
```

**预期结果:** 显示 Git 状态

### 测试 4: GitHub 克隆

```
在 D:\AI\AGENT_U盘\release 目录克隆 https://github.com/farion1231/cc-switch.git
```

**预期结果:** 成功克隆仓库

---

## 配置后的能力提升

配置完成后,Claude 将能够:

1. **直接访问 Windows 文件:**
   - 不再需要通过 Linux VM 间接访问
   - 可以读取、创建、修改项目文件

2. **执行 Windows 命令:**
   - 直接运行 `cargo build`
   - 执行 `git clone` 和 `git checkout`
   - 运行 `npm install` 和其他 Node.js 工具
   - 使用 Windows 完整工具链

3. **自主完成构建任务:**
   - 不再需要你手动运行 PowerShell 脚本
   - 可以直接克隆 GitHub 仓库(绕过 VM 网络限制)
   - 能够自动化执行完整的 CCSwitch 构建流程

4. **解决网络限制:**
   - Windows 环境可以直接访问 GitHub
   - 不受 Linux VM 代理白名单限制
   - 可以下载依赖和克隆仓库

---

## 安全说明

⚠️ **重要提示:**

1. **执行权限:** `shell` MCP 以**你的 Windows 用户权限**执行命令,相当于给 Claude 较高的本机操作能力

2. **目录限制:** `filesystem` MCP 只能访问你明确指定的目录,不会访问整个 `C:\`

3. **可信来源:** 
   - `@modelcontextprotocol/server-filesystem` - 官方文件系统 MCP
   - `@executeautomation/mcp-server-shell` - 社区维护的 Shell MCP

4. **随时修改:** 如果担心权限过大,可以随时编辑配置文件:
   - 删除 `shell` 部分(仅保留文件访问)
   - 减少 `filesystem` 的目录列表
   - 完全禁用 MCP(删除配置文件)

---

## 替代配置方案

### 方案 A: 仅文件系统访问(保守)

如果你暂时只想让 Claude 访问文件,不执行命令:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "D:\\AI\\AGENT_U盘\\codex-desktop-usb-installer"
      ]
    }
  }
}
```

### 方案 B: 添加更多目录

需要访问更多目录时,在 `args` 数组中添加:

```json
"args": [
  "-y",
  "@modelcontextprotocol/server-filesystem",
  "D:\\AI\\AGENT_U盘\\codex-desktop-usb-installer",
  "D:\\Projects",
  "E:\\Workspace"
]
```

---

## 故障排除

### 问题 1: 重启后看不到新工具

**可能原因:** JSON 格式错误

**解决方法:**
1. 检查配置文件语法(JSON 不允许尾随逗号)
2. 使用 JSON 验证器检查格式
3. 查看 Claude Desktop 日志(如果有)

### 问题 2: MCP Server 启动失败

**可能原因:** npx 不可用或网络问题

**解决方法:**
```powershell
# 验证 npx 可用
npx --version

# 手动测试 MCP Server
npx -y @modelcontextprotocol/server-filesystem "D:\AI\AGENT_U盘"
```

### 问题 3: 权限被企业策略阻止

**可能原因:** 企业 MDM 策略限制

**解决方法:**
1. 打开 Claude Desktop 设置 → Developer
2. 确保 "Allow local MCP servers" 已启用
3. 检查是否有企业策略阻止 `isLocalDevMcpEnabled`

### 问题 4: Shell 命令执行失败

**可能原因:** PATH 环境变量不完整

**解决方法:**
Shell MCP 继承 Windows 用户的 PATH,确保:
- `cargo` 在 PATH 中
- `git` 在 PATH 中
- 重启 Claude Desktop 以刷新环境变量

---

## 配置完成后的工作流

配置完成后,你可以直接对 Claude 说:

**示例 1: 自动构建 CCSwitch**
```
克隆 https://github.com/farion1231/cc-switch.git 到 D:\AI\AGENT_U盘\release\ccswitch-source,
切换到 commit 1d44b1ba,应用环境变量补丁,然后执行 cargo build --release
```

**示例 2: 执行测试**
```
在项目目录运行 cargo test --lib account
```

**示例 3: Git 操作**
```
在项目目录创建新分支 feature/g39-account-fix,提交所有修改
```

**示例 4: 依赖安装**
```
在项目目录运行 npm install
```

Claude 将直接在 Windows 环境中执行这些操作,不再需要你手动干预!

---

## 下一步行动

1. **立即执行:** 运行自动化配置脚本
   ```powershell
   cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"
   .\tools\setup-claude-desktop-mcp.ps1
   ```

2. **重启 Claude Desktop:** 完全退出并重新启动

3. **验证配置:** 在新会话中测试上述 4 个验证命令

4. **开始自主工作:** 让 Claude 直接完成 CCSwitch 构建任务

---

## 相关文档

- [Claude Desktop MCP 官方文档](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)
- [MCP Protocol 规范](https://github.com/modelcontextprotocol/docs)
- [Filesystem MCP Server](https://github.com/modelcontextprotocol/server-filesystem)
- [Shell MCP Server](https://www.npmjs.com/package/@executeautomation/mcp-server-shell)

---

**配置完成后,Claude 将像"原来的 Codex"一样,能够直接在 Windows 环境中自主完成大部分开发任务!**
