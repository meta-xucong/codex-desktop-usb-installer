---
name: image-prompt-generator
description: 为 AI 图像生成创建结构化、高质量的提示词，支持信息图、场景图、流程图、对比图、框架图、时间线及多种视觉风格。Use when 用户只需要图片提示词、内容配图方案、信息图规划或社交媒体视觉文案；用户要求实际生成图片时，默认使用 U 盘统一媒体 MCP，除非用户明确指定其他方式。
---

# Image Prompt Generator - 图片提示词生成器

专业的图片提示词生成引擎，为 AI 图像生成创建结构化、高质量的提示词。

---

## 使用边界

先判断用户要“提示词”还是“实际图片”。只要提示词时，本技能输出完成后结束，不要求外部图像服务。
用户要求实际生成图片时，默认通过 U 盘预装的 `$image-2` Skill 和统一媒体 MCP 生成；可在内部整理提示词，但不增加用户选工具的步骤，也不先尝试 Codex 原生 ImageGen。只有用户明确指定其他方式时才切换。

```
用户需求：生成图片
    ↓
Step 1: image-prompt-generator（本技能）
        - 分析内容
        - 生成结构化提示词
        - 仅在用户明确要求保存/创建文件时，保存到其指定目录
    ↓
Step 2（仅在用户要求实际图片时）
        - 调用 `$image-2`，使用安装器从当前上游目录自动匹配且已配置的 Image-2 模型
        - 展示请求并等待用户确认后，由 MCP 在同一次提交中保存并校验图片
        - 没有可用的默认生成能力时，在对话中交付提示词并说明限制；不保存文件，也不自动切换生成方式
```

**禁止行为：**
- ❌ 在 U 盘版 Codex 中绕过 `$image-2` 的统一媒体 MCP 图片链路
- ❌ 在没有调用成功时声称图片已经生成
- ❌ 用户只要提示词时擅自提交可能计费的生成任务

---

## 内容类型（6种）

根据内容特征自动推荐或手动选择：

| 类型 | 适用场景 | 示例 |
|------|---------|------|
| `infographic` | 数据、指标、知识分享 | 数据图表、技术架构、概念解释 |
| `scene` | 叙事、情感、生活方式 | 故事场景、人物画面、氛围图 |
| `flowchart` | 流程、步骤、工作流 | 操作指南、流程图、步骤说明 |
| `comparison` | 对比、选项、优劣分析 | 产品对比、方案对比、前后对比 |
| `framework` | 模型、架构、思维框架 | 理论模型、系统架构、思维导图 |
| `timeline` | 历史、演进、发展历程 | 时间线、版本迭代、成长历程 |

### 自动匹配指南

| 内容信号 | 推荐类型 |
|----------|----------|
| 数据、指标、数字、对比 | infographic |
| 知识、概念、教程、学习 | infographic |
| 技术、AI、编程、开发 | infographic |
| 步骤、流程、操作指南 | flowchart |
| 模型、架构、框架 | framework |
| 优劣对比、方案选择 | comparison |
| 故事、情感、经历 | scene |
| 历史、发展、迭代 | timeline |

---

## 视觉风格（22种）

### 核心风格（快速选择）

| 风格 | 特点 | 最佳适用 |
|------|------|---------|
| `vector` | 简洁扁平矢量 | 知识文章、教程、技术内容 |
| `minimal` | 极简现代 | 通用、知识分享、SaaS |
| `blueprint` | 科幻科技风 | AI、前沿技术、系统设计 |
| `sketch` | 手绘自然 | 轻松、反思、休闲内容 |
| `editorial` | 编辑杂志风 | 流程、数据、新闻 |

### 完整风格库

| 风格 | 描述 | 最佳适用 |
|------|------|---------|
| `vector` | 简洁扁平矢量，大胆形状 | 知识文章、教程、技术 |
| `notion` | 极简手绘线条，知识美学 | 知识分享、SaaS、生产力 |
| `elegant` | 精致优雅，商务感 | 商业、思想领导力 |
| `warm` | 友好亲切，温暖治愈 | 个人成长、生活方式、教育 |
| `minimal` | 超简洁禅意 | 哲学、极简主义、核心概念 |
| `blueprint` | 技术示意图，工程风 | 架构、系统设计、工程 |
| `watercolor` | 柔和艺术，自然温暖 | 生活方式、旅行、创意 |
| `editorial` | 杂志风格信息图 | 技术解释、新闻 |
| `scientific` | 学术精确图表 | 生物、化学、技术研究 |
| `chalkboard` | 课堂粉笔手绘 | 教育、教学、解释 |
| `fantasy-animation` | 温暖自然色彩、细腻背景的手绘奇幻动画 | 故事书、魔幻、情感 |
| `flat` | 现代大胆几何 | 现代数字、当代 |
| `flat-doodle` | 可爱扁平粗轮廓 | 可爱、友好、亲切 |
| `intuition-machine` | 技术简报做旧纸张 | 技术简报、学术 |
| `nature` | 有机大地插画 | 环境、 wellness |
| `pixel-art` | 复古8位游戏美学 | 游戏、复古技术 |
| `playful` | 俏皮粉彩涂鸦 | 趣味、休闲、教育 |
| `retro` | 80/90年代霓虹几何 | 80/90怀旧、大胆 |
| `sketch` | 原始铅笔笔记本 | 头脑风暴、创意探索 |
| `sketch-notes` | 柔和手绘温暖笔记 | 教育、温暖笔记 |
| `vintage` | 做旧羊皮纸历史 | 历史、遗产 |
| `cute` | 柔和色彩、圆润造型、贴纸装饰 | 生活方式、美妆、日常分享 |

风格描述使用通用视觉特征；用户要求复刻特定艺术家、工作室或品牌的独特风格时，改用高层视觉属性（如配色、笔触、构图、年代感）描述，不声称官方关联或授权。

核心风格名称是完整风格库的别名：`minimal-flat` → `minimal`、`sci-fi` → `blueprint`、`hand-drawn` → `sketch`。`scene` 属于内容类型，不属于视觉风格。矩阵仅列常用风格组合，其他完整风格库条目仍可选；未列组合不代表不可用，遇到不确定时按内容与用户偏好选择并说明。

### 风格详细定义

#### notion 风格
```yaml
canvas:
  ratio: portrait-3-4
  grid: single | dual
image_effects:
  cutout: clean
  stroke: none | white-solid
  filter: none | muted-tons
typography:
  decorated: none | handwritten
  tags: black-white | pill
  direction: horizontal
decorations:
  emphasis: circle-mark | underline
  background: solid-white | paper-texture
  doodles: hand-drawn-lines | arrows-curvy
  frames: none | rounded-rect

color_palette:
  primary: "#1A1A1A, #4A4A4A"
  background: "#FFFFFF, #FAFAFA"
  accents: "#A8D4F0, #F9E79F, #FADBD8"

visual_elements:
  - 简单线条涂鸦，手绘摇摆效果
  - 几何形状、简笔人物
  - 最大留白，单重墨水线
  - 干净、简洁的构图

best_for:
  - 知识分享
  - 概念解释
  - SaaS内容
  - 生产力技巧
  - 技术教程
  - 专业内容
```

#### cute 风格
```yaml
canvas:
  ratio: portrait-3-4
  grid: single | dual | quad
image_effects:
  cutout: soft
  stroke: white-solid | colored-solid
  filter: clear-glow | cream-skin
typography:
  decorated: bubble | highlight
  tags: pill | bubble
  direction: horizontal
decorations:
  emphasis: star-burst | hearts
  background: solid-pastel | gradient-linear
  doodles: hearts | stars-sparkles | flowers
  frames: polaroid | tape-corners

color_palette:
  primary: "#FED7E2, #FEEBC8, #C6F6D5, #E9D8FD"
  background: "#FFFAF0, #FFF5F7"
  accents: "#FF69B4, #FF6B6B"

visual_elements:
  - 爱心、星星、闪光、可爱表情
  - 丝带装饰、贴纸风格
  - 可爱贴纸、emoji图标
  - 柔和圆润的形状

best_for:
  - 生活方式内容
  - 美妆护肤
  - 时尚穿搭
  - 日常技巧
  - 个人分享
```

---

## 布局系统（8种）

### 基于密度的布局

| 布局 | 信息密度 | 留白 | 每图要点 | 最佳适用 |
|------|---------|------|---------|---------|
| `sparse` | 低 | 60-70% | 1-2 | 封面、引言、冲击性陈述 |
| `balanced` | 中 | 40-50% | 3-4 | 标准内容、教程 |
| `dense` | 高 | 20-30% | 5-8 | 知识卡片、备忘单 |

### 基于结构的布局

| 布局 | 结构 | 项目数 | 最佳适用 |
|------|------|--------|---------|
| `list` | 垂直枚举 | 4-7 | 排行、清单、步骤指南 |
| `comparison` | 左右对比 | 2个部分 | 前后对比、优劣对比 |
| `flow` | 连接节点 | 3-6步 | 流程、时间线、工作流 |
| `mindmap` | 中心放射 | 4-8分支 | 概念图、头脑风暴、主题概览 |
| `quadrant` | 四象限网格 | 4个部分 | SWOT分析、优先级矩阵、分类 |

### 按位置推荐布局

| 位置 | 推荐布局 | 原因 |
|------|---------|------|
| Cover | sparse | 最大视觉冲击力，清晰标题 |
| Setup | balanced | 提供背景但不压倒 |
| Core | balanced/dense/list | 基于内容密度 |
| Payoff | balanced/list | 清晰要点 |
| Ending | sparse | 干净CTA，难忘收尾 |

---

## 画布规格

### 宽高比

| 名称 | 比例 | 像素 | 适用场景 |
|------|------|------|---------|
| portrait-3-4 | 3:4 | 1242×1660 | 小红书、社交媒体（推荐） |
| square | 1:1 | 1242×1242 | 通用、头像 |
| portrait-2-3 | 2:3 | 1242×1863 | 更高格式、长内容 |
| landscape-16-9 | 16:9 | 1920×1080 | 横屏、演示文稿 |

**默认**：portrait-3-4

### 安全区域

避免在这些区域放置关键内容：

| 区域 | 位置 | 原因 |
|------|------|------|
| bottom-overlay | 底部10% | 移动端标题栏覆盖 |
| top-right | 右上角 | 点赞/分享按钮覆盖 |
| bottom-right | 右下角 | 水印位置 |

```
┌─────────────────────────────┐
│                 [like/share]│  ← 右上角：避免
│                             │
│      ✓ 安全内容区域         │
│                             │
│  [底部10%：避免关键信息]    │  ← 底部：避免
└─────────────────────────────┘
```

---

## Type × Style 兼容性矩阵

本矩阵仅列常用风格组合，完整风格库中的其他风格仍可选择；未列组合不代表不可用，按内容与用户偏好判断并说明。

| | vector | notion | warm | minimal | blueprint | watercolor | elegant | editorial | scientific | cute |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| infographic | ✓✓ | ✓✓ | ✓ | ✓✓ | ✓✓ | ✓ | ✓✓ | ✓✓ | ✓✓ | ✓ |
| scene | ✓ | ✓ | ✓✓ | ✓ | ✗ | ✓✓ | ✓ | ✓ | ✗ | ✓✓ |
| flowchart | ✓✓ | ✓✓ | ✓ | ✓ | ✓✓ | ✗ | ✓ | ✓✓ | ✓ | ✓ |
| comparison | ✓✓ | ✓✓ | ✓ | ✓✓ | ✓ | ✓ | ✓✓ | ✓✓ | ✓ | ✓ |
| framework | ✓✓ | ✓✓ | ✓ | ✓✓ | ✓✓ | ✗ | ✓✓ | ✓ | ✓✓ | ✓ |
| timeline | ✓ | ✓✓ | ✓ | ✓ | ✓ | ✓✓ | ✓✓ | ✓✓ | ✓ | ✓ |

✓✓ = 强烈推荐 | ✓ = 兼容 | ✗ = 不推荐

---

## 工作流程

### Step 1: 内容分析

1. **接收用户内容**
   - 文件路径 → 只读取完成当前任务所需的内容
   - 粘贴内容 → 在当前对话内分析；仅在用户明确要求保存/创建文件时才写入

2. **深度分析**
   - 内容类型识别
   - 核心信息提取（2-5个要点）
   - 视觉机会识别
   - 目标受众分析
   - 推荐图片数量

3. **分析结果交付**
   - 默认在对话中交付；只有用户明确要求保存/创建文件时才写入 `analysis.md`

### Step 2: 参数确认 ⚠️

优先采用内容分析得出的推荐值。只有缺少会实质改变结果的关键信息、且无法安全合理地默认时，才向用户提出简短澄清；不得假定存在特定产品的 `AskUserQuestion` 工具，不得为简单提示词任务强制提问。必要时最多确认4个参数：

**Q1: 内容类型**（需要时确认）
- [基于分析的推荐]（推荐）
- infographic / scene / flowchart / comparison / framework / timeline

**Q2: 视觉风格**（需要时确认）
- [基于 Type × Style 兼容性的推荐]（推荐）
- 其他兼容风格
- 查看完整风格库

**Q3: 信息密度**（需要时确认）
- sparse (1-2点) - 封面、强调
- balanced (3-4点) - 标准内容
- dense (5-8点) - 知识卡片

**Q4: 布局类型**（可选）
- 自动推荐
- list / comparison / flow / mindmap / quadrant

### Step 3: 生成大纲

仅当用户要求保存/创建文件时，保存为 `outline.md`；否则在对话中交付：

```yaml
---
type: infographic
style: notion
density: balanced
layout: list
image_count: 4
aspect_ratio: portrait-3-4
---

## Image 1
**Position**: [位置]
**Purpose**: [目的]
**Visual Content**: [视觉内容描述]
**Layout**: sparse
**Filename**: 01-cover-topic.png

## Image 2
...
```

### Step 4: 生成提示词 ⛔ **阻塞步骤**

**先完成提示词并向用户展示；只有用户明确要求保存文件时才保存。用户要求实际生图时，需先展示请求并取得本次提交确认。**

1. **文件保存**：仅在用户明确要求保存/创建文件时，写入其指定目录下的 `prompts/NN-{type}-{slug}.md`。不覆盖已有文件；遇到同名文件时保留原文件并改用新文件名或请用户指定。

2. **提示词文件结构**：
   ```yaml
   ---
   image_id: 01
   type: infographic
   style: notion
   layout: sparse
   aspect_ratio: "3:4"
   ---

   ## Layout
   [整体构图描述]

   ## ZONES
   [每个视觉区域的具体内容]

   ## LABELS
   [仅填入有来源支持的数字、术语、指标；缺失时标注待核实或省略]

   ## COLORS
   [指定十六进制代码及语义]

   ## STYLE
   [线条处理、纹理、氛围]

   ## CONTENT
   [核心内容描述]
   ```

3. **提示词质量要求**（全部必需）：
   - `Layout`: 整体构图（grid / radial / hierarchical / left-right / top-down）
   - `ZONES`: 每个视觉区域的具体内容
   - `LABELS`: 仅使用用户提供或已核实来源支持的数字、术语、指标和引用；未提供/未核实的内容标为“待核实”占位符或省略，不得编造数据、引用、出处或事实。品牌、商标、Logo、产品功效与背书仅在用户提供准确素材并授权使用时纳入；不推断其真实性或授权。
   - `COLORS`: 指定十六进制代码及语义（如 `Coral (#E07A5F) for emphasis`）
   - `STYLE`: 线条处理、纹理、氛围、角色渲染
   - `ASPECT`: 指定比例

4. **验证**：如已获用户明确授权保存文件，先确认目标目录和文件名无冲突；否则直接在对话中交付提示词。

### Step 5: 参考图片处理（如提供）

**如用户提供参考图片**：仅在当前选定的生成能力明确支持参考图输入且能实际传入时，才将图片作为参考输入；不得假定 `$image-2`、统一媒体 MCP 或 Codex 工具接受 `--ref` 等特定参数。

1. **保存参考图片**
   - 仅在用户明确要求保存/创建文件时，保存到其指定目录；不覆盖已有文件。
   - 未获保存授权时，在当前任务中读取/分析用户提供的图像，不另存副本。

2. **分析参考图片**
   - 视觉特征：风格、颜色、构图
   - 内容/主题
   - 适用位置

3. **使用方式**

| 使用类型 | 场景 | 操作 |
|---------|------|------|
| `direct` | 参考与期望输出非常匹配 | 仅当当前生成能力明确支持且接口说明确认参数时传入；否则转为文字描述并告知用户参考图未直接传入 |
| `style` | 仅提取视觉风格特征 | 分析后追加到提示词 |
| `palette` | 仅提取配色方案 | 提取颜色后追加到提示词 |

### Step 6: 视觉一致性控制

**生成系列图片时**：

1. **第一张图片（封面）**：建立视觉锚点，不传入未经接口文档确认的参考图参数
2. **后续图片**：仅在所选生成能力明确支持参考图输入时，使用第一张输出作为参考；否则从第一张提取可见特征并写入后续提示词，不声称传入了参考图

生成图片 1 时使用对应提示词建立视觉锚点。生成后续图片时使用对应提示词，并在所选生成能力支持参考图时引用第一张输出。不要把某个外部服务特有的参数套用到 Codex 原生工具。

### Step 7: 完成报告

```
图片提示词生成完成！

内容类型: [type]
视觉风格: [style]
信息密度: [density]
布局类型: [layout]
图片数量: [count]
交付方式: [对话 / 用户指定目录中的文件]

文件（仅在用户明确要求保存时列出实际已写入的文件）:
[实际文件路径；未保存时不列文件清单]

下一步: 用户要求实际图片时，仅按使用边界调用默认的 `$image-2` 统一媒体 MCP；不可用或失败则说明限制并在对话中交付提示词。只有用户明确指定其他方式时才切换。仅需提示词时直接在对话中交付
```

---

## 提示词基础模板

### 通用结构

比例、分辨率、文件格式等参数必须与所选生成能力的已知支持范围一致。能力未知或未暴露该参数时，不得承诺能生成指定规格；可将其作为视觉意图描述，或明确标注为待确认。

```markdown
Create an image following these specifications:

## Image Specifications
- **Type**: [infographic/scene/flowchart/comparison/framework/timeline]
- **Orientation**: [Portrait/Landscape]
- **Aspect Ratio**: [3:4/1:1/2:3/16:9]
- **Style**: [style_name]

## Core Principles
- [风格核心原则]
- [内容表达原则]
- [视觉层次原则]

## Text Style
- [文字风格要求]
- [字体处理要求]

## Language
- Use the same language as the content
- Match punctuation style

---

## Style Section
[颜色、视觉元素、排版风格]

## Layout Section
[布局结构、信息密度、视觉平衡]

## Content Section
[具体内容描述]

## Watermark Section（如启用）
[水印内容、位置、透明度]
```

---

## 与图像生成能力协作

1. 使用最终提示词（文件仅在用户明确要求保存时读取）并确认用户确实要求生成图片。
2. 未指定其他生成方式时，调用 `$image-2`；先显示将要发送的提示词和参数，等待本次请求的明确确认。
3. 让 MCP 使用当前配置的 Image-2 模型；它会在同一次提交中保存 PNG 并校验文件。若模型未配置，先说明当前上游目录没有匹配项，不猜模型 ID。
4. 将返回的本地图片路径与提示词对应；系列图仅在所选能力明确支持时使用前一张作为参考。
5. 默认 MCP 不可用或调用失败时返回真实错误和提示词，不自动切换到其他工具、脚本或直连 API；只有用户明确指定其他方式时才切换。不伪造输出 URL 或本地文件。

---

## 文件保存边界

默认只在对话中交付分析、大纲和提示词，不自动创建目录或写入文件。只有用户明确要求保存/创建文件时，才使用用户指定的输出目录；如用户未指定，先询问目录或给出拟用目录等待确认。不得覆盖现有文件；名称冲突时使用新名称或请用户指定。源文件路径仅用于读取，除非用户明确要求修改源文件。

## 目录结构（仅在用户明确要求保存文件时）

```
{project-slug}/
├── source.{ext}               # 仅当用户明确要求保存源副本时
├── analysis.md                # 内容分析
├── outline.md                 # 大纲规划
├── references/                # 参考图片（如提供）
│   ├── 01-ref-{slug}.png
│   └── 01-ref-{slug}.md
├── prompts/                   # 提示词文件
│   ├── 01-cover-{slug}.md
│   ├── 02-content-{slug}.md
│   └── ...
└── output/                    # 仅在已成功调用图像生成能力时存在
    ├── 01-cover-{slug}.png
    ├── 02-content-{slug}.png
    └── ...
```

---

## 注意事项

1. **职责清晰**：提示词生成可独立完成；图片生成必须由当前可用工具真实执行
2. **关键步骤**：提示词交付、参考图能力核实等步骤不可跳过；文件保存须遵循用户授权边界
3. **提示词质量**：包含完整的视觉元素描述，确保生成质量
4. **一致性**：系列图片保持风格和视觉一致性
5. **参考图片**：如提供参考图片，必须正确处理
6. **水印**：如用户在偏好设置中启用水印，必须应用
7. **语言**：提示词语言与源内容语言一致
8. **兼容性**：遵循 Type × Style 兼容性矩阵选择最佳组合
