# Windows PowerShell script encoding gate

## Failure mechanism

Windows PowerShell 5.1 uses the active legacy ANSI code page when it reads a script file that has no encoding signature. A UTF-8 file without a byte-order mark can therefore turn Chinese comments or string literals into different characters before parsing. The parser may then report a later, misleading syntax error such as an unterminated string. The same bytes commonly parse in PowerShell 7, whose default is UTF-8, so a developer-only PowerShell 7 check is insufficient.

The fixed capability package is consumed through Windows PowerShell entrypoints. Every packaged `.ps1`, `.psm1`, and `.psd1` therefore uses strict UTF-8 with a BOM. Newline style is left unchanged; both CRLF and LF are supported. `installer-pack/file-manifest.json` and `checksums.sha256` bind the actual packaged bytes, so the encoding marker is covered by package integrity checks.

## Release gate

`tools/verify-capability-pack-powershell.ps1` runs under Windows PowerShell 5.1 and:

1. enumerates all PowerShell source files in the selected tree;
2. rejects a missing UTF-8 BOM;
3. rejects invalid UTF-8 byte sequences;
4. parses every file with the Windows PowerShell parser and reports file/line/column diagnostics.

`tools/build-usb-layout.ps1` runs the gate against both the source capability pack and the copied USB payload. `tools/verify-oneclick-usb.ps1` repeats the check against the final USB directory using the verifier copied under `verification/`. This catches encoding damage or stale files after copy, not just a source-tree defect. The normal one-click build invokes the copied final verifier before it reports success.

The build and verification scripts resolve the in-box Windows PowerShell 5.1 executable from `%WINDIR%` first, rather than trusting a same-named command earlier on `PATH`. Child stderr is captured as diagnostic output, but success is determined by the child exit code; this avoids a host-only provider warning turning a successful check into a false failure under `$ErrorActionPreference = 'Stop'`.

The offline regression suite also invokes the same Windows PowerShell 5.1 gate and uses temporary fixtures to verify that valid BOM-marked Chinese text passes while BOM-less text, malformed UTF-8, and invalid PowerShell syntax fail.

## Scope boundary

All scripts in `installer-pack/` are required to have UTF-8 BOMs, including currently ASCII-only scripts, so future localization does not silently create a code-page-dependent package. Tracked PowerShell scripts elsewhere that contain non-ASCII text also use BOMs. The two scripts embedded into Rust with `include_str!` remain UTF-8 source text without a file BOM because they are passed to PowerShell as Unicode command text rather than loaded by PowerShell from disk; they are outside the packaged-script file gate.

This change does not modify Codex model configuration, the media MCP protocol, provider routing, or Codex startup logic.
