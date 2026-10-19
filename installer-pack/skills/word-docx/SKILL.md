---
name: word-docx
description: Read and generate Word documents with correct structure, styles, and cross-platform compatibility.
---

## 文件创建、依赖与交付

- 用户明确要求创建/修改文件时才写文件；用户给出输出位置时使用该路径，否则在工作区内选择唯一的新文件名。已有目标不得覆盖；请用户选择其他路径或明确授权覆盖。
- 将用户原文件视为输入材料，默认另存为新文件，不修改源文件。仅处理本 Skill 支持的 `.docx`；`.docm` 可能含宏，不运行、启用或移除宏，也不把它伪装成已安全处理的普通文档。
- 创建或编辑 `.docx` 需要 U 盘包提供的 Python 3.12 与锁定的 `python-docx` 依赖。按包内健康检查确认环境；依赖缺失或导入失败时报告具体缺项，不创建占位文件，也不声称已生成。
- 写入后重新打开产物并检查 ZIP/XML 可读性、预期文本和语义结构（标题层级、列表、表格等）；检查失败时保留原输入，不报告成功。
- 结构检查不等于页面视觉验收。只有在可用渲染器确实生成 PDF/页面图并检查所有相关页面后，才能报告已做视觉检查；渲染器缺失或失败时，明确列出已完成的结构检查并说明页面分页、字体替换、孤行、表格跨页和打印效果尚未验证。
- 只报告本次实际执行的检查与工具；不承诺任何本机/跨平台渲染器一定可用。

## 中文公文与正式文件

用户要求公文、制度、正式函件或严格版式时，先确认文种、收文对象、适用格式依据和是否有现成模板。分清内容校对与版式规范，不凭空声称符合某一机关的最新强制标准。对标题层级、正文、附件、落款、日期、页眉页脚统一样式；优先使用 Word 样式而非逐段手工格式。若需视觉验收，按“文件创建、依赖与交付”流程渲染并检查页面；没有完成渲染时不得声称已检查字体替换、分页、孤行、表格跨页或打印效果。需要匹配特定地区/组织规范时请用户提供模板或可核查依据。


## Structure

- DOCX is a ZIP containing XML files—`word/document.xml` has main content, `word/styles.xml` has styles
- Text splits into runs (`<w:r>`)—each run has uniform formatting; one word may span multiple runs
- Paragraphs (`<w:p>`) contain runs—never assume one paragraph = one text block
- Sections control page layout—headers/footers, margins, orientation are per-section

## Styles vs Direct Formatting

- Styles (Heading 1, Normal) are named and reusable—direct formatting is inline and overrides style
- Removing direct formatting reveals underlying style—useful for cleanup
- Character styles apply to runs, paragraph styles to paragraphs—they layer together
- Linked styles can be both—applying to paragraph or selected text behaves differently

## Lists & Numbering

- Numbering is complex: `abstractNum` defines pattern, `num` references it, paragraphs reference `numId`
- Restart numbering not automatic—need explicit `<w:numPr>` with restart flag
- Bullets and numbers share the numbering system—both use `numId`
- Indentation controlled separately from numbering—list can exist without visual indent

## Headers, Footers, Sections

- Each section can have different headers/footers—first page, odd, even pages
- Section breaks: next page, continuous, even/odd page—affects pagination
- Headers/footers stored in separate XML files—referenced by section properties
- Page numbers are fields, not static text—update on open or print

## Track Changes & Comments

- Track changes stores original and revised in same document—accept/reject to finalize
- Deleted text still present with `<w:del>` wrapper—don't assume visible = all content
- Comments reference ranges via bookmark IDs—`<w:commentRangeStart>` to `<w:commentRangeEnd>`
- Revision IDs track who changed what—metadata persists even after accepting

## Fields & Dynamic Content

- Fields have code and cached result—`{ DATE \@ "yyyy-MM-dd" }` vs displayed date
- TOC, page numbers, cross-references are fields—update fields to refresh
- Hyperlinks can be fields or direct `<w:hyperlink>`—both valid
- MERGEFIELD for mail merge—placeholder until merge executes

## Compatibility

- Compatibility mode limits features to earlier Word version—check `w:compat` settings
- Page size defaults vary by tool and region—set US Letter vs A4 explicitly or pagination and table widths can drift
- LibreOffice/Google Docs: complex formatting may shift—test roundtrip
- Embedded fonts may not transfer—fallback fonts substitute
- DOCM contains macros (security risk); DOC is legacy binary format

## Common Pitfalls

- Empty paragraphs for spacing—prefer space before/after in paragraph style
- Manual page breaks inside paragraphs—use section breaks for layout control
- Images in headers: relationship IDs are per-part—same image needs separate relationship in header
- Copy-paste brings source styles—can pollute style gallery with duplicates
