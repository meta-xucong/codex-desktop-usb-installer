---
name: manim-animation
description: Create mathematical or explanatory animations as editable Manim scenes; use when the desired output is a vector-style animated lesson, diagram, or equation sequence.
---

# Manim 动画

先设计叙事、证明过程和视觉层级，再写小而清晰的 Manim 场景。U 盘安装器会把 Manim Community 0.21.0 安装到独立 Python 环境；优先检查 `%CODEX_HOME%\codex-agent-kit\bin\manim.cmd` 或 `CODEX_MANIM_PYTHON`，不要误用系统里另一个 Manim。

这个隔离环境和 Python 依赖随能力包离线安装。Manim 0.21 使用 PyAV 处理视频；本地 FFmpeg 另供导入/导出和后期处理，不是普通 Manim 场景渲染的硬性前提。MiKTeX/LaTeX 是可选项，只在 `Tex`/`MathTex` 公式排版时需要。若未安装，改用原生 Manim 文本/图形表达公式，或明确说明公式渲染受限，不要假称已生成。

使用有节奏的动画、清晰标签和统一配色；先输出低成本预览，检查遮挡、可读性和时长，再按需渲染最终质量。区分交付物是场景源文件、预览还是完整视频。
