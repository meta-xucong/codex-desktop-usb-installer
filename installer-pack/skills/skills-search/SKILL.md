---
name: skills-search
description: 在当前 Codex 能力包或用户明确提供的本地目录中搜索可用技能，并返回功能、状态和依赖。
---

# 技能目录搜索

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

这个技能默认只搜索本地可审计目录，不连接未经配置的远程市场，也不会自动下载或安装第三方内容。
搜索结果会包含技能名称、功能说明、兼容性状态和依赖提示。

## 命令行

在技能目录中执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/skills_search.ps1 -Keyword "视频"
powershell -ExecutionPolicy Bypass -File scripts/skills_search.ps1 -Keyword "图像生成" -Json
powershell -ExecutionPolicy Bypass -File scripts/skills_search.ps1 -Keyword "表格" -Root "C:\path\to\installer-pack"
```

如果不传 `--root`，脚本会从自身位置向上查找 `manifest/imported-skills.txt`，再扫描对应的
`skills/` 目录。脚本只读 `SKILL.md` 的元数据，不执行技能脚本。

## 使用边界

- 本技能只负责发现和解释，不代表某个技能已经安装或具备运行时。
- 安装前应检查 `dependencies.json`、兼容性状态和许可证/来源说明。
- 远程市场、私有仓库或企业内部目录需要用户明确提供地址并单独审查。
- 本技能使用 PowerShell 和文件读取，不依赖 Python；在已安装目录中找不到总清单时，会退化为扫描邻近的
  `SKILL.md` 并把状态标为 `unknown`。
