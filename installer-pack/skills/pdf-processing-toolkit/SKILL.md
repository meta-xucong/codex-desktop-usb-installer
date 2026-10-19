---
name: pdf-processing-toolkit
description: 使用安装包提供的 Python 运行时读取、提取、合并、写入和生成 PDF，并对依赖健康状态如实报告。
---

# PDF 处理

本 Skill 由本项目自行维护，许可证见同目录 `LICENSE.txt`。它只提供薄的、可审计的工作流，
不包含任何第三方 Skill 的上游正文；`pypdf`、`pdfplumber`、`reportlab` 仍按各自上游许可证
使用，并由安装包的离线 wheelhouse 安装。

## 运行条件

- 使用安装包生成的 `CODEX_PYTHON`（Windows 上通常是 `...\\.codex\\codex-agent-kit\\bin\\python.cmd`），
  不依赖系统 PATH 中碰巧存在的 Python。
- 运行前确认 `runtime-state.json` 中 Python 健康检查为 `passed`；缺包或运行时异常时返回明确
  的缺失依赖，不伪造提取、合并或生成成功。
- 默认只读写用户明确指定的输入和输出路径；脚本输入和输出必须使用绝对路径或工作区内
  `<skill-directory>` 之外的项目数据目录；不联网、不上传 PDF、不把中间文件写入 Skill 目录。

## 提供的操作

随包脚本 `scripts/pdf_tool.py` 提供 `info`、`extract`、`merge`、`generate` 和 `smoke` 子命令。
其中 `smoke` 会在临时目录生成一页 PDF，使用 `pypdf` 读取、使用 `pdfplumber` 提取文本，
再写出合并结果并校验页数；安装器和审计器可用它验证运行时，而不接触用户文件。

示例：

```text
CODEX_PYTHON scripts/pdf_tool.py info input.pdf
CODEX_PYTHON scripts/pdf_tool.py extract input.pdf --output text.txt
CODEX_PYTHON scripts/pdf_tool.py merge output.pdf first.pdf second.pdf
CODEX_PYTHON scripts/pdf_tool.py generate output.pdf --text "示例"
CODEX_PYTHON scripts/pdf_tool.py smoke
```

`extract --output` 会先在目标目录写入完整暂存文本，再发布到目标路径。暂存写入失败时，已有目标文件保持原字节内容，脚本会清理本次创建的暂存文件；成功时才用完整提取文本替换目标。省略 `--output` 时仍将文本输出到标准输出。

如果 `pypdf`、`pdfplumber` 或 `reportlab` 无法导入，应停止操作并报告具体包名和运行时路径；
不要退回到猜测 PDF 内容或声称已经写入文件。
