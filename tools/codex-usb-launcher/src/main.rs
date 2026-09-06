//! USB-only entry point with an embedded Windows `requireAdministrator`
//! manifest. The tiny launcher asks for the ordinary UAC confirmation and
//! starts the separately patched USB manager, which carries the same manifest
//! as a defence-in-depth boundary for direct/legacy launches.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = run() {
        show_error(&error);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Codex USB launcher is only supported on Windows");
}

#[cfg(target_os = "windows")]
fn run() -> Result<(), String> {
    use std::process::Command;

    let launcher =
        std::env::current_exe().map_err(|error| format!("无法确定 U 盘入口路径：{error}"))?;
    let root = launcher
        .parent()
        .ok_or_else(|| "U 盘入口没有可用的父目录".to_string())?;
    let manager = root.join("CodexAppManager.exe");
    if !manager.is_file() {
        return Err(format!(
            "缺少同目录的 CodexAppManager.exe：{}。请将 U 盘一键包完整复制，不要只复制入口 EXE。",
            manager.display()
        ));
    }
    if !root.join("manifest.json").is_file() || !root.join("payload").is_dir() {
        return Err(
            "未找到 manifest.json 或 payload 目录。请从 U 盘一键包根目录启动，保持目录结构不变。"
                .to_string(),
        );
    }

    let mut arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if !arguments.iter().any(|arg| arg == "--codex-usb-elevated") {
        arguments.push("--codex-usb-elevated".into());
    }
    Command::new(&manager)
        .args(arguments)
        .current_dir(root)
        .spawn()
        .map_err(|error| format!("启动 CodexAppManager.exe 失败：{error}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn show_error(message: &str) {
    const MB_OK: u32 = 0x0000_0000;
    const MB_ICONERROR: u32 = 0x0000_0010;
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }
    let title = wide_null("Codex App 管理器");
    let body = wide_null(&format!("U 盘入口无法启动管理器。\n\n{message}"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
