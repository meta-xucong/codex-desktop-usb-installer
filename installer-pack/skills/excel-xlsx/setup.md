# Setup — Excel / XLSX

Read this when preference or memory handling is relevant. Ask only questions needed for the current task or requested ongoing personalization.

## File and calculation limits

This Skill's bundled `openpyxl` path handles `.xlsx` and `.xlsm`. For `.xls`, ask the user to convert it with Excel or LibreOffice, or stop if no converter is available; do not imply that this runtime can process it. When modifying `.xlsm`, load with `keep_vba=True` and save with the `.xlsm` extension to preserve the VBA project. `openpyxl` does not run macros or calculate formulas. Cached formula results can be read only if already present; Excel or LibreOffice must recalculate and save the file when fresh results are required.

## Your Attitude

You're helping someone who works with Excel files. They might be frustrated with date bugs, precision issues, or cross-platform headaches. Show that you understand these pains and will help them avoid them.

**Use natural language:** Talk about Excel pitfalls and best practices, not about "memory files" or "config". The user cares about getting their spreadsheets right.

## Preference handling

For a one-off spreadsheet request, handle the requested task first and ask only questions needed to avoid a material mistake. Do not require an integration-preference interview before working.

If the user wants ongoing personalization, ask concise questions about relevant tools, platform, date conventions, ID handling, or large-file needs. Skip questions already answered or irrelevant to the current workflow.

### Saving preferences

Ask open questions to understand how they work with Excel:
- What do they use Excel for? (reports, data import/export, analysis, templates?)
- What tools/libraries do they use? (openpyxl, pandas, SheetJS, xlsxwriter, manual?)
- What platforms? (Windows, Mac, Linux, or mixed?)
- Any recurring pain points? (dates, encoding, large files?)

Start broad, then narrow based on what matters to them.

Some users have strong preferences about:
- Date system (1900 vs 1904)
- Date format (DD/MM/YYYY vs MM/DD/YYYY vs ISO)
- How to handle numeric IDs (always text, or only when needed)
- Large file handling (when to suggest streaming)

Adapt to preferences they share. Don't push for details they don't care about.

## What may be saved (only with explicit consent)

Only save after the user shares it:
- Integration preference (when to activate)
- Tools/libraries they use
- Primary platform (Windows/Mac/Linux)
- Date preferences (if mentioned)
- Known pain points to watch for

Save only the preferences the user explicitly agrees to retain. Never create or update a memory file merely because the user shared information during a one-off task. If the user declines or asks not to save, continue without persistence.

Preferences are optional. Continue helping when no preference file exists or setup is incomplete, using workbook inspection and conservative defaults where needed.
