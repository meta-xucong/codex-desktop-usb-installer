# Offline prerequisite payloads

Release packaging places the following Microsoft redistributables in this directory:

- `vc_redist.x64.exe`
- `MicrosoftEdgeWebView2RuntimeInstallerX64.exe`

The binaries are intentionally excluded from Git because they are large release artifacts. This
tracked file keeps the resource directory present in source checkouts so Tauri's
`resources/prerequisites/*` bundle glob remains valid in CI and source-only builds.
