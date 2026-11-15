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

    if let Some(product) = installed_legacy_security_software()? {
        return Err(format!(
            "检测到已安装的旧版安全软件：{product}。为避免其干扰安装，请先卸载该软件，再重新运行安装器。"
        ));
    }

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
fn installed_legacy_security_software() -> Result<Option<&'static str>, String> {
    use std::ffi::c_void;
    use std::ptr;

    type Hkey = *mut c_void;
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as i32 as isize as Hkey;
    const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002u32 as i32 as isize as Hkey;
    const KEY_READ: u32 = 0x0002_0019;
    const KEY_WOW64_64KEY: u32 = 0x0000_0100;
    const KEY_WOW64_32KEY: u32 = 0x0000_0200;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_NO_MORE_ITEMS: i32 = 259;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    #[link(name = "Advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            key: Hkey,
            sub_key: *const u16,
            options: u32,
            desired_access: u32,
            result: *mut Hkey,
        ) -> i32;
        fn RegEnumKeyExW(
            key: Hkey,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            last_write: *mut c_void,
        ) -> i32;
        fn RegQueryValueExW(
            key: Hkey,
            value_name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            data_len: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn display_name(key: Hkey) -> Option<String> {
        let value_name = wide("DisplayName");
        let mut value_type = 0;
        let mut byte_len = 0;
        let status = unsafe {
            RegQueryValueExW(
                key,
                value_name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                ptr::null_mut(),
                &mut byte_len,
            )
        };
        if status != 0 || !matches!(value_type, REG_SZ | REG_EXPAND_SZ) || byte_len < 2 || byte_len > 65_536 {
            return None;
        }
        let mut data = vec![0u16; (byte_len as usize).div_ceil(2)];
        let status = unsafe {
            RegQueryValueExW(
                key,
                value_name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                data.as_mut_ptr().cast(),
                &mut byte_len,
            )
        };
        if status != 0 || !matches!(value_type, REG_SZ | REG_EXPAND_SZ) {
            return None;
        }
        let end = data.iter().position(|unit| *unit == 0).unwrap_or(data.len());
        Some(String::from_utf16_lossy(&data[..end]))
    }

    fn product_for(name: &str) -> Option<&'static str> {
        let name = name.to_lowercase();
        if ["360安全卫士", "360杀毒", "360 total security"].iter().any(|needle| name.contains(needle)) {
            Some("360 安全软件")
        } else if ["腾讯电脑管家", "qq电脑管家", "tencent pc manager"].iter().any(|needle| name.contains(needle)) {
            Some("腾讯电脑管家")
        } else if ["金山毒霸", "kingsoft internet security", "kingsoft antivirus"].iter().any(|needle| name.contains(needle)) {
            Some("金山毒霸")
        } else {
            None
        }
    }

    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let path = wide(UNINSTALL_PATH);
            let mut root: Hkey = ptr::null_mut();
            let status = unsafe { RegOpenKeyExW(hive, path.as_ptr(), 0, KEY_READ | view, &mut root) };
            if status == ERROR_FILE_NOT_FOUND {
                continue;
            }
            if status != 0 {
                return Err(format!("无法检查已安装软件登记（Windows 错误码 {status}）。"));
            }

            let mut index = 0;
            loop {
                let mut sub_key = vec![0u16; 512];
                let mut name_len = sub_key.len() as u32;
                let status = unsafe {
                    RegEnumKeyExW(
                        root,
                        index,
                        sub_key.as_mut_ptr(),
                        &mut name_len,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                };
                if status == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if status != 0 {
                    unsafe { RegCloseKey(root) };
                    return Err(format!("无法读取已安装软件登记（Windows 错误码 {status}）。"));
                }

                let child_path = wide(&String::from_utf16_lossy(&sub_key[..name_len as usize]));
                let mut child: Hkey = ptr::null_mut();
                let child_status = unsafe { RegOpenKeyExW(root, child_path.as_ptr(), 0, KEY_READ, &mut child) };
                if child_status == 0 {
                    if let Some(name) = display_name(child) {
                        if let Some(product) = product_for(&name) {
                            unsafe {
                                RegCloseKey(child);
                                RegCloseKey(root);
                            }
                            return Ok(Some(product));
                        }
                    }
                    unsafe { RegCloseKey(child) };
                }
                index += 1;
            }
            unsafe { RegCloseKey(root) };
        }
    }

    Ok(None)
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
    let body = wide_null(&format!("U 盘入口无法继续。\n\n{message}"));
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
