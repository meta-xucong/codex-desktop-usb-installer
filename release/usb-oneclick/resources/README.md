# Bundled resources

## `BinaryDelta`

Sparkle's `BinaryDelta` command-line tool (MIT, from
[sparkle-project/Sparkle](https://github.com/sparkle-project/Sparkle),
version-aligned with OpenAI Codex's Sparkle 2.9.x). The manager invokes it to
reconstruct a new Codex bundle from a published binary delta
(`apply <old.app> <new.app> <patch>`).

It is **not committed** to this repo — it is a build/CI artifact. Release
builds vendor it here so it ships inside the app bundle and is resolved at
runtime via `BaseDirectory::Resource`.

How the manager finds it at runtime (`commands::resolve_binary_delta`):

1. `CODEX_BINARY_DELTA` env var (explicit override — dev / a system Sparkle);
2. this resources dir (`resources/BinaryDelta`) inside the packaged app.

### Provide it for a release build

Drop a **universal** `BinaryDelta` (arm64 + x86_64) into this directory before
`tauri build`, e.g. from a Sparkle release:

```sh
# from a Sparkle 2.9.x distribution (XCFramework / SwiftPM artifact)
cp "$SPARKLE/bin/BinaryDelta" src-tauri/resources/BinaryDelta
chmod +x src-tauri/resources/BinaryDelta
lipo -archs src-tauri/resources/BinaryDelta   # expect: x86_64 arm64
```

### Run it in dev (no bundle)

```sh
export CODEX_BINARY_DELTA=/path/to/Sparkle/bin/BinaryDelta
```
