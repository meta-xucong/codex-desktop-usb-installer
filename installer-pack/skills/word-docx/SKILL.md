---
name: word-docx
description: Read and generate Word documents with correct structure, styles, and cross-platform compatibility.
---

## 中文公文与正式文件

用户要求公文、制度、正式函件或严格版式时，先确认文种、收文对象、适用格式依据和是否有现成模板。分清内容校对与版式规范，不凭空声称符合某一机关的最新强制标准。对标题层级、正文、附件、落款、日期、页眉页脚统一样式；优先使用 Word 样式而非逐段手工格式。输出前检查字体替换、分页、孤行、表格跨页和打印效果；需要匹配特定地区/组织规范时请用户提供模板或可核查依据。


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
