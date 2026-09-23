# Claude Desktop MCP 修正配置

**问题:** 原配置使用的 Shell MCP 包名 `@executeautomation/mcp-server-shell` 不存在

**解决方案:** 使用正确的包名 `mcp-shell-server`

---

## 正确的配置文件

**配置文件位置:**
```
%APPDATA%\Claude\claude_desktop_config.json
```

**完整路径:**
```
C:\Users\T14S\AppData\Roaming\Claude\claude_desktop_config.json
```

**正确的配置内容:**

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
        "mcp-shell-server"
      ]
    }
  }
}
```

---

## 快速修复步骤

### 方法 1: 手动编辑配置文件

在 PowerShell 中执行:

```powershell
# 打开配置文件
notepad "$env:APPDATA\Claude\claude_desktop_config.json"
```

将 `"@executeautomation/mcp-server-shell"` 改为 `"mcp-shell-server"`

保存并关闭,然后重启 Claude Desktop。

### 方法 2: 重新运行配置脚本

配置脚本已经更新,直接重新运行:

```powershell
cd "D:\AI\AGENT_U盘\codex-desktop-usb-installer"
.\tools\setup-claude-desktop-mcp.ps1
```

---

## 验证步骤

1. **确认配置文件已更新:**
   ```powershell
   Get-Content "$env:APPDATA\Claude\claude_desktop_config.json"
   ```

2. **测试 MCP Server 是否可用:**
   ```powershell
   npx -y mcp-shell-server
   ```
   应该启动成功(按 Ctrl+C 退出)

3. **完全重启 Claude Desktop:**
   - 关闭所有窗口
   - 在任务管理器中确认 Claude 进程已退出
   - 重新启动

4. **在新会话中测试:**
   - 开启全新会话
   - 测试命令执行:"运行 cargo --version 命令"
   - 测试文件访问:"列出项目目录内容"

---

## 关于 Rust 安装

配置生效后,如果 Rust 仍未安装,我可以通过 Shell MCP 直接在你的 Windows 环境中安装 Rust:

```powershell
# 下载 rustup 安装器
Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile "$env:TEMP\rustup-init.exe"

# 运行安装器(静默安装)
& "$env:TEMP\rustup-init.exe" -y

# 刷新环境变量
$env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
```

或者手动安装:
1. 访问 https://rustup.rs/
2. 下载并运行 rustup-init.exe
3. 按照提示完成安装

---

## 配置完成后的能力

一旦配置正确并且 Rust 安装完成,我就能:

✅ 直接在 Windows 环境执行命令
✅ 访问 Windows 文件系统
✅ 使用 cargo、git、npm 等 Windows 工具链
✅ 克隆 GitHub 仓库(绕过 Linux VM 网络限制)
✅ 自主完成完整的 CCSwitch 构建任务
✅ 不再需要你手动干预

---

**下一步:** 手动编辑配置文件或重新运行脚本,然后重启 Claude Desktop 并开启新会话。

**生成时间:** 2026-09-20
