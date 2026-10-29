USB CDP collector

This read-only diagnostic attaches to the Codex renderer started by the
diagnostic USB manager. It does not start Codex, change its configuration,
install plugins, or write to the Codex profile.

Requirements: Node.js 22 or newer. No npm install is required.

Before launching the diagnostic manager, set
CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME to the existing Codex profile directory
you intend to inspect. For the MSI-15 test account, the known path is:

  C:\Users\MSI-15\AppData\Local\CodexAppManager\usb-codex-home

Then start Codex through CodexDesktopSetup.exe and open Codex from the manager.
In PowerShell at the USB bundle root, run:

  node .\diagnostics\usb-cdp\collector.mjs --port 9222 --seconds 180 --output "$env:TEMP\codex-cdp-diagnostic.jsonl"

If node is not on PATH but the managed runtime exists, run:

  & "$env:LOCALAPPDATA\CodexAppManager\usb-codex-home\codex-agent-kit\runtime\node-v22.22.0-win-x64\node.exe" .\diagnostics\usb-cdp\collector.mjs --port 9222 --seconds 180 --output "$env:TEMP\codex-cdp-diagnostic.jsonl"

The collector only connects to loopback (127.0.0.1). A missing event is
reported as not_observed; attachment can happen after the renderer's first
startup check. Stop the collector with Ctrl+C, close Codex normally, close the
manager, and confirm that port 9222 is no longer listening.
