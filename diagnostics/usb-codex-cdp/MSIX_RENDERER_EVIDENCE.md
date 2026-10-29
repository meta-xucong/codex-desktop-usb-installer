# Exact MSIX / renderer evidence

Verified in this workspace from `D:\AI\AGENT_U盘\.codex-browser-availability-diagnostic-20261025\release\browser-diagnostic-20261025-1554\payload\OpenAI.Codex_x64.msix`.

- MSIX bytes: 968,540,851; SHA-256 `9085CF2E2EB5081F894A2CF334E1F36DBB9438ECB409D20195AA2FE1ECFCB4C8`.
- `AppxManifest.xml` package version: `26.1016.8622.0`.
- `app/resources/app.asar` root `package.json` internal version: `26.1016.62342`.
- ASAR entry `webview/assets/app-initial-ad16eaadf81f.js`: 14,606,938 bytes, SHA-256 `84d1c97245fec7be606fd1d263459d9a4c841836751c6ba5e9b348b69687332f`.
- The previously extracted `nSu-original.js` (5,926 bytes, SHA-256 `18e0922d49006faa5ee3e059ba9d08c3de16277410c19cc4c67af26b33475fc1`) is an exact substring of that renderer entry at UTF-16 offset 12,720,381.
- The exact guard expression occurs once, at CDP zero-based `lineNumber=5006`, `columnNumber=53137` (UTF-16 offset 12,723,946). It is `f||_e.isLoading||ve.isLoading||!p&&ve.reason===\`statsig-disabled\`||ye.isLoading||Ee.isLoading||Oe.isLoading||U===\`loading\`||L.status===\`loading\`||Je.status===\`loading\`||Ye.status===\`loading\`||(Ei.dispatchMessage`.
- The `electron-desktop-features-changed` event string occurs once. The call-return breakpoint candidate is after `Ei.dispatchMessage(...)` at the following `iu.info(...)` call, CDP zero-based `lineNumber=5006`, `columnNumber=54946` (UTF-16 offset 12,725,755). This only proves the renderer call returned when hit; it does not prove main-process receipt or acceptance.

Reproduce with:

```powershell
.\verify-msix-renderer.ps1 -MsixPath <path-to-exact-msix>
```

The PowerShell wrapper verifies the pinned MSIX hash/package version, copies only `app/resources/app.asar` to a unique temporary file, runs the ASAR verifier, then removes the temp copy. The verifier checks the internal version, renderer hash, unique expression positions, and emits no source or user data.

Static source offsets are not V8's runtime `actualLocation`. The collector checks the returned CDP location and refuses to treat a shifted breakpoint as armed. Runtime resolution against a live 26.1016.62342 renderer remains untested.

