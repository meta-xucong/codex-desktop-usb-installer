# Test Log

- 2026-08-31 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，168/168；包含 USB 指纹/许可证/5 秒状态机和 CCSwitch 哈希门禁测试。
- 2026-08-31 `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol`：通过。
- 2026-08-31 `npm run check`：通过；`npm run lint`：通过；`npm test -- --run src`：通过，29 文件/272 测试。
- 2026-08-31 `npm run build`：通过；renderer entry isolation verified。
- 2026-08-31 `npm run test:offline-bundle`：通过；合成 MSIX、清单、哈希、U 盘目录复核通过。
- 2026-08-31 `release/usb-oneclick/verify-usb.ps1 -SkipNetwork`：通过；真实 MSIX Authenticode/Add-AppxPackage/拔盘未覆盖。
- 2026-08-31 Release EXE `--codex-usb-guard` headless smoke：退出码 0（无租约时正常退出）。
- 2026-08-31 完整 `npm test`：业务相关测试通过；release-workflow 的 bash/WSL fixture 在本机失败，windowMode 首次并行运行偶发失败，单独重跑通过。该环境问题未改动业务代码。
- 2026-08-31 `git diff --check`：通过（仅 Git 行尾转换提示）。
- 2026-08-31 `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过；锁和 CCSwitch PID 校验收紧后 `cargo test` 仍为 168/168。
- 2026-08-31 `npm run test:offline-bundle`：通过；重新生成 Release U 盘目录后逐文件验证通过。
- 2026-08-31 Release EXE `--codex-usb-guard` headless smoke：退出码 0；当前 Release SHA256 为 `e7c858ea195fbde1f86f8121b193041c3ee7fcaee197c6cfcd3b0b248560fc25`。

