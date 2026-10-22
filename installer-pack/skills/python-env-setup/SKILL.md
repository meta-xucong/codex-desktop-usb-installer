---
name: python-env-setup
description: 检测 Windows 机器上的 Python 解释器并给出安装或 PATH 修复建议；不会未经确认改动系统。
---

# Python 环境检查（Windows 原生）

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

用于确认目标机器是否有可调用的 Windows x64 CPython 3.12。由于离线 wheelhouse 使用 `cp312` ABI，
Python 3.11、3.13 或非 CPython 解释器都不能复用；不匹配时应安装能力包锁定的私有 3.12.10。

## 检测

```powershell
powershell -ExecutionPolicy Bypass -File scripts/check_python_env.ps1 -Json
```

脚本按 `python`、`py -3`、`python3` 顺序检查解释器，验证 `cpython-312`、`win_amd64` 和 x64，返回命令、绝对路径、ABI 和版本。它只调用解释器探针，
不安装软件、不修改 PATH、不读取用户配置。

## 安装边界

如果没有可用解释器，应先说明需要用户批准，并由用户使用组织批准的离线安装包或 Windows 软件管理
工具安装。安装包名称和静默参数见 `ENVIRONMENT-MCP-INVENTORY.md` 与 `dependencies.json`。

不要自动下载、执行未知安装包，也不要假设 Python、Node、Git 或浏览器驱动已经存在。这个检查器本身
只依赖 Windows PowerShell，因此即使 Python 缺失也可以运行。
