---
name: pdf-processing-toolkit
description: PDF capability quarantine marker. Use the current Codex PDF capability or user-provided instructions until redistribution evidence for this imported Skill is approved.
---

# PDF Skill 已隔离

`CODEX_PUBLIC_QUARANTINE_STUB_V1`

本公开仓库不再分发该 Skill 的上游指南正文。原始元数据引用了缺失的 `LICENSE.txt`，再分发许可目前不可核验，因此该 Skill 保持 `unsupported`，不得被 Agent 或安装器当作可执行 PDF 实现。

处理 PDF 请求时，优先使用当前 Codex 已提供的 PDF 能力；如果当前环境没有对应能力，只能基于用户提供的材料给出步骤，并明确说明未执行文件操作。

只有在取得可核验的再分发授权，或用许可证清晰、由仓库自行维护的实现替换本占位后，才能重新设计依赖、健康检查和启用条件。不要通过 `-IncludeUnsupported` 绕过这个许可门禁。
