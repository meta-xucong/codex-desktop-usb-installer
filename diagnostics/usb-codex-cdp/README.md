# USB Codex CDP collector (source and mock only)

This is an external Node.js collector for a Codex renderer that is already started with Chromium remote debugging on loopback. It does not start Codex, change application state, install plugins, or write to the Codex profile. It only attaches to page targets whose URL contains both `Codex` and `app.asar`; it uses CDP target discovery plus explicit attach, not global `Target.setAutoAttach`, so unrelated targets are not paused.

It samples the exact guard expression from Desktop `26.1016.62342` and reports only a fixed whitelist of booleans plus a capped reason enum. The IPC breakpoint is placed after the renderer `dispatchMessage(...)` call returns. A hit is reported as `renderer-ipc-call-returned`; it does not prove main-process receipt. Every final summary states main acceptance as `not_observed`. Missing targets, breakpoints, events, timeout, or disconnect are represented as `not_observed`, never as false.

Before calling a breakpoint armed, the collector compares CDP's `actualLocation` with the requested source location. A mismatch removes that breakpoint and emits only a mismatch marker. A `Debugger.paused` event is resumed only if its hit breakpoint ID belongs to this collector. On normal stop it resumes only a pause it recorded itself, removes its breakpoints, and detaches. After a CDP socket disconnect, remote resume is impossible; the collector reports the disconnect and does not claim recovery.

## Important startup timing limit

The collector attaches after CDP exposes the page target. It may miss the renderer's first guard/IPC run if that happened before attachment. A missing hit means `not_observed`, not gate failure or success. There is no global startup pause because that would pause non-target pages.

## Current authorization boundary

The repository has a feature-gated `codex-usb-cdp-diagnostic` Cargo target. When compiled with `usb-cdp-diagnostic`, it keeps `SignedRequired`, uses the separate local helper/lease namespace `%LOCALAPPDATA%\CodexUsbCdpDiagnosticGuard` and helper `CodexUsbCdpGuard.exe`, and has its own Tauri application identifier, settings/provenance/cache/log roots, and elevated-session mutex. The regular `codex-app-manager` binary explicitly refuses this diagnostic feature so `--all-features` cannot silently turn the ordinary entry point into a diagnostic build. The diagnostic manager requests renderer debugging on loopback port 9222 only as part of a user-initiated Codex launch; its manager UI does not open the CDP port on its own. The ordinary target retains its existing guard namespace and `remote_debugging_port: None` behavior.

For a diagnostic run, set `CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME` to an existing absolute profile directory before starting the diagnostic manager, for example:

```powershell
$env:CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME = 'F:\CodexUSB\existing-codex-home'
```

The manager refuses a missing, relative, nonexistent, or symlinked path and never copies or creates that directory. Codex normally writes startup state to its selected profile. Choose the existing USB manager CODEX_HOME or another profile explicitly; do not point it at the currently running official Desktop profile unless that normal write behavior is intended.

`build\CodexUsbCdpDiagnostic.exe` is a compiled, unsigned build artifact for inspection only. It is not a runnable test-machine package: it must be included in a separately signed USB bundle whose manifest, SHA ledger, anchor, and USB license match that bundle. Do not copy it into the current bundle or launch it outside the USB authorization path. The build used a version-matched public signing profile; it did not access a signing vault or create credentials.

To rebuild the setup artifacts, use `tools\build-installer.ps1 -UsbCdpDiagnostic -UsbPolicy SignedRequired -UsbSigningProfilePath <version-matched-public-profile> -OutputDir release\setup-usb-cdp-diagnostic`. The script refuses an existing output directory and stages a `USB-CDP-DIAGNOSTIC.txt` marker. It keeps the internal manager filename `CodexAppManager.exe` because the existing USB layout contract requires that exact name, but the diagnostic setup directory is separate from production output. Then run `tools\build-oneclick-usb-layout.ps1 -UsbCdpDiagnostic -InternalSigned` with that manager, its matching launcher/shim, the existing USB payload source, and a new output directory whose name includes `diagnostic`. This writes a diagnostic marker and manifest profile, and regenerates the usual immutable SHA ledger. Finally, the USB license signer must sign the finalized diagnostic bundle on the intended removable volume; the license remains tied to that bundle's anchor/fingerprint. No package signing was run here.

## Usage after a separately authorized diagnostic build exists

The ZIP does not include Node.js. The test machine must have Node.js 22 or newer available; the collector uses only Node.js built-in modules and requires no `npm install` or third-party packages. Node.js 24.14.1 was used for the mock tests.

```powershell
node .\collector.mjs --port 9222 --seconds 180 --output "$env:TEMP\codex-cdp-diagnostic.jsonl"
```

The output must be a new file. The collector accepts only `ws://` loopback debugger URLs. Collection lasts 1–300 seconds; `Ctrl+C` stops normally and attempts to detach/resume its own pauses. Do not expose the port to the LAN or leave it open after diagnosis.

## Normal shutdown and recovery

Use this order after collection:

1. Stop the collector with `Ctrl+C` and wait for its final summary.
2. Close the diagnostic Codex window normally. This closes its CDP endpoint on `127.0.0.1:9222`.
3. Close the diagnostic manager normally.
4. Confirm port 9222 is no longer listening, for example with `Get-NetTCPConnection -LocalPort 9222 -State Listen -ErrorAction SilentlyContinue`. If a listener remains, identify it before taking any action; do not kill unrelated processes.
5. Restore or remove the temporary `CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME` value from the shell used to start the diagnostic manager. Do not change the persistent user or machine environment and do not delete the selected profile.

When setting the variable in PowerShell, preserve any prior process value and restore it afterward:

```powershell
$name = 'CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME'
$previous = [Environment]::GetEnvironmentVariable($name, 'Process')
try {
    $env:CODEX_USB_CDP_DIAGNOSTIC_CODEX_HOME = 'F:\CodexUSB\existing-codex-home'
    # Start the diagnostic manager from this shell and perform the approved run.
    # Then stop the collector, close Codex and the manager, and confirm port 9222 is closed.
}
finally {
    if ($null -eq $previous) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
    else { Set-Item "Env:$name" $previous }
}
```

If the collector crashes or its terminal is forcibly closed, a CDP disconnect makes remote resume impossible; the collector cannot guarantee cleanup of its breakpoints in that case. Close diagnostic Codex normally to end the renderer session and its debugging endpoint, then close the diagnostic manager and confirm port 9222 stopped. Do not terminate unrelated processes or delete the selected Codex profile. If Codex itself remains open and the debugger is no longer listening, exit Codex normally before attempting another diagnostic run.

## Offline checks

```powershell
node --check .\collector.mjs
node --test .\collector.test.mjs
.\verify-msix-renderer.ps1 -MsixPath <path-to-exact-msix>
```

CDP lifecycle tests use a local mock WebSocket server. They are not a live Codex attachment test.
