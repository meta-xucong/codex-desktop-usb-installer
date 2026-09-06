//! Windows elevation hand-off for the removable USB entry point.
//!
//! The USB bundle has a separate small launcher with an embedded
//! `requireAdministrator` manifest.  Windows therefore grants the manager a
//! full administrator token before Tauri/WebView startup, so AppX-volume
//! registration and the offline install do not depend on a late, hidden
//! permission failure.  The installed manager (which has no sibling
//! `manifest.json` + `payload` layout) is not affected.  Directly opening the
//! manager still has a native `runas` fallback for diagnostics and legacy
//! bundles.
//!
//! This is consented elevation, not a UAC bypass.  Windows still decides
//! whether the current user may elevate, and a cancelled prompt is surfaced as
//! a clear startup error.

#[cfg(target_os = "windows")]
use std::ffi::OsStr;

pub const ELEVATED_ARG: &str = "--codex-usb-elevated";

#[cfg(target_os = "windows")]
const USB_INSTANCE_MUTEX_NAME: &str = r"Local\CodexUsbInstaller.FullAdmin.v1";

/// Keep the Windows kernel object alive for the entire USB manager lifetime.
/// The normal install/update operation also has an fs4 interprocess lock;
/// this earlier singleton additionally protects runtime prerequisite installs
/// before Tauri/ManagerState exists. It is deliberately distinct from the
/// normal manager plugin, so an older medium-integrity window cannot receive
/// and replace this elevated launch.
#[cfg(target_os = "windows")]
pub struct UsbManagerInstance {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl Drop for UsbManagerInstance {
    fn drop(&mut self) {
        // We hold the object's lifetime, not mutex ownership (bInitialOwner
        // is FALSE). CloseHandle suffices; never release another thread's lock.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
fn validate_usb_instance_creation(created: bool, last_error: u32) -> Result<(), String> {
    if !created {
        return Err(format!(
            "无法建立 U 盘安装器会话锁（Win32 {last_error}: {}），为防止并发安装已停止启动",
            std::io::Error::from_raw_os_error(last_error as i32)
        ));
    }
    if last_error == windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS {
        return Err("本 Windows 会话中已有 U 盘安装器正在运行。请使用已打开的窗口；如需重新启动，请先正常退出原窗口。不会终止已有安装进程。".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn acquire_usb_manager_instance() -> Result<Option<UsbManagerInstance>, String> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    if !crate::app::usb_guard::guard_required() {
        return Ok(None);
    }
    if !current_process_is_elevated()? {
        return Err(
            "U 盘安装器未获得管理员令牌，请从 CodexDesktopSetup.exe 启动并确认 UAC。".to_string(),
        );
    }
    let name = wide_null(USB_INSTANCE_MUTEX_NAME);
    // No ownership waiting and no global-session privilege is needed.
    // Atomic named-object creation also covers rapid repeated double clicks.
    let (handle, last_error) = unsafe {
        SetLastError(0);
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        (handle, GetLastError())
    };
    if handle.is_null() {
        validate_usb_instance_creation(false, last_error)?;
        unreachable!("null handle must be an error");
    }
    let guard = UsbManagerInstance { handle };
    validate_usb_instance_creation(true, last_error)?;
    Ok(Some(guard))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationAction {
    /// The current process may continue normally.
    Continue,
    /// A new elevated copy was launched; the caller must exit this process.
    Relaunched,
}

/// Pure decision helper kept platform-independent for deterministic tests.
pub fn should_relaunch_usb_manager(
    usb_bundle: bool,
    process_elevated: bool,
    elevation_marker_present: bool,
) -> bool {
    usb_bundle && !process_elevated && !elevation_marker_present
}

/// Request elevation for a USB bundle before Tauri/WebView startup.
///
/// On non-Windows targets this is a no-op.  On Windows, a failed or cancelled
/// UAC request returns an error so the medium-integrity parent cannot continue
/// into a partial USB installation path.
pub fn maybe_relaunch_usb_manager() -> Result<ElevationAction, String> {
    #[cfg(not(target_os = "windows"))]
    {
        Ok(ElevationAction::Continue)
    }

    #[cfg(target_os = "windows")]
    {
        if !crate::app::usb_guard::guard_required() {
            return Ok(ElevationAction::Continue);
        }

        let marker_present = std::env::args().any(|arg| arg == ELEVATED_ARG);
        let process_elevated = current_process_is_elevated()?;
        log::info!(
            "USB manager startup full-admin-token probe elevated={} marker_present={}",
            process_elevated,
            marker_present
        );
        if process_elevated {
            return Ok(ElevationAction::Continue);
        }
        if marker_present {
            // Never treat a user-supplied marker as proof of elevation.  This
            // protects the install path from silently falling back to a
            // medium-integrity process after a failed RunAs hand-off.
            let message =
                "Codex U 盘入口未获得管理员令牌。请在 Windows UAC 对话框中选择“是”后重试。";
            show_elevation_error(message);
            return Err(message.to_string());
        }

        let executable =
            std::env::current_exe().map_err(|error| format!("无法确定 U 盘安装器路径：{error}"))?;
        let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
        relaunch_with_runas(&executable, &arguments)?;
        Ok(ElevationAction::Relaunched)
    }
}

#[cfg(target_os = "windows")]
pub fn current_process_is_elevated() -> Result<bool, String> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE};
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, EqualSid, GetTokenInformation, TokenElevation, TokenElevationType,
        TokenElevationTypeDefault, TokenElevationTypeFull, TokenElevationTypeLimited, TokenGroups,
        WinBuiltinAdministratorsSid, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES, TOKEN_ELEVATION,
        TOKEN_GROUPS, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(format!(
            "无法读取当前 Windows 令牌：{}",
            std::io::Error::last_os_error()
        ));
    }

    // Check the actual process token, not the account's group membership.
    // Elevated child processes normally inherit this token; an ACL denial
    // does not imply the token was lost. Require all three signals for the
    // child path: elevated token, full/default elevation type, and effective
    // membership in the built-in Administrators group.
    let result = (|| {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0_u32;
        let queried = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        };
        if queried == 0 {
            return Err(format!(
                "无法读取 Windows 提权状态：{}",
                std::io::Error::last_os_error()
            ));
        }
        if elevation.TokenIsElevated == 0 {
            return Ok(false);
        }

        let mut elevation_type = 0_i32;
        returned = 0;
        let queried = unsafe {
            GetTokenInformation(
                token,
                TokenElevationType,
                (&mut elevation_type as *mut i32).cast::<c_void>(),
                size_of::<i32>() as u32,
                &mut returned,
            )
        };
        if queried == 0 {
            return Err(format!(
                "无法读取 Windows 令牌类型：{}",
                std::io::Error::last_os_error()
            ));
        }
        if elevation_type == TokenElevationTypeLimited {
            return Ok(false);
        }
        if elevation_type != TokenElevationTypeFull && elevation_type != TokenElevationTypeDefault {
            return Ok(false);
        }

        // Windows requires SID storage to be DWORD-aligned.
        let mut admin_sid = [0_u32; SECURITY_MAX_SID_SIZE as usize / size_of::<u32>()];
        let mut admin_sid_size = std::mem::size_of_val(&admin_sid) as u32;
        let admin_sid_ptr = admin_sid.as_mut_ptr().cast::<c_void>();
        let created = unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                admin_sid_ptr,
                &mut admin_sid_size,
            )
        };
        if created == 0 {
            return Err(format!(
                "无法构造本地 Administrators SID：{}",
                std::io::Error::last_os_error()
            ));
        }

        // CheckTokenMembership requires an impersonation token; on a primary
        // token (the one returned by OpenProcessToken) it fails with
        // ERROR_NO_IMPERSONATION_TOKEN (1309).  That failure is exactly what
        // made a fully-elevated USB manager exit at startup under UAC consent
        // or "Run as administrator".  Inspect the primary token's group list
        // directly with GetTokenInformation(TokenGroups) instead, which is
        // legal on a primary token, and compare each SID against the local
        // Administrators well-known SID.
        let mut groups_size = 0_u32;
        let size_probe = unsafe {
            GetTokenInformation(
                token,
                TokenGroups,
                std::ptr::null_mut(),
                0,
                &mut groups_size,
            )
        };
        let probe_err = std::io::Error::last_os_error();
        // We expect the probe to fail with ERROR_INSUFFICIENT_BUFFER and to
        // report the required size. Any other outcome means the token did not
        // behave like a normal user token, so fail loud instead of guessing.
        if size_probe != 0 {
            return Err("无法读取当前令牌的组列表大小：探测调用应失败但返回成功".to_string());
        }
        if probe_err.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(format!("无法读取当前令牌的组列表大小：{}", probe_err));
        }
        if groups_size == 0 {
            return Err("无法读取当前令牌的组列表大小：返回长度为 0".to_string());
        }
        // Vec<u8> provides only byte alignment by contract; casting it into
        // TOKEN_GROUPS creates an unaligned reference (undefined behavior).
        // usize storage covers the pointer alignment of SID_AND_ATTRIBUTES.
        let words = (groups_size as usize).div_ceil(size_of::<usize>());
        let mut groups_buffer = vec![0_usize; words];
        let mut groups_returned = 0_u32;
        let groups_queried = unsafe {
            GetTokenInformation(
                token,
                TokenGroups,
                groups_buffer.as_mut_ptr().cast::<c_void>(),
                groups_size,
                &mut groups_returned,
            )
        };
        if groups_queried == 0 {
            return Err(format!(
                "无法读取当前令牌的组列表：{}",
                std::io::Error::last_os_error()
            ));
        }
        let entries_offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
        if groups_returned as usize > groups_size as usize
            || (groups_returned as usize) < entries_offset
        {
            return Err("Windows 令牌组列表返回了无效长度".to_string());
        }
        let groups_ptr = groups_buffer.as_ptr().cast::<TOKEN_GROUPS>();
        // Read only GroupCount: the header of a zero-group response need not
        // contain a complete one-element TOKEN_GROUPS structure.
        let group_count = unsafe { std::ptr::addr_of!((*groups_ptr).GroupCount).read() } as usize;
        if group_count
            > (groups_returned as usize - entries_offset) / size_of::<SID_AND_ATTRIBUTES>()
        {
            return Err("Windows 令牌组列表计数越界".to_string());
        }
        let sid_attrs = unsafe {
            std::slice::from_raw_parts(
                groups_buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(entries_offset)
                    .cast::<SID_AND_ATTRIBUTES>(),
                group_count,
            )
        };
        let mut is_member = false;
        for entry in sid_attrs {
            if entry.Sid.is_null() {
                continue;
            }
            // A deny-only or disabled Administrators SID must not be treated
            // as proof of elevation; require SE_GROUP_ENABLED.
            if !group_is_effective(entry.Attributes, SE_GROUP_ENABLED as u32) {
                continue;
            }
            let equal = unsafe { EqualSid(entry.Sid, admin_sid_ptr) };
            if equal != 0 {
                is_member = true;
                break;
            }
        }
        Ok(is_member)
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}

#[cfg(target_os = "windows")]
fn group_is_effective(attributes: u32, enabled_flag: u32) -> bool {
    // SE_GROUP_USE_FOR_DENY_ONLY; never treat this group as granting rights,
    // even if a malformed/restricted token also carries an enabled bit.
    const DENY_ONLY: u32 = 0x0000_0010;
    attributes & enabled_flag != 0 && attributes & DENY_ONLY == 0
}

#[cfg(not(target_os = "windows"))]
pub fn current_process_is_elevated() -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "windows")]
fn relaunch_with_runas(
    executable: &std::path::Path,
    arguments: &[std::ffi::OsString],
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::HINSTANCE;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SHOW_WINDOW_CMD, SW_SHOWNORMAL};

    let mut command_line = arguments
        .iter()
        .map(|argument| quote_windows_arg(argument))
        .collect::<Vec<_>>();
    command_line.push(quote_windows_arg(OsStr::new(ELEVATED_ARG)));
    let parameters = command_line.join(" ");

    let verb = wide_null("runas");
    let file = executable
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let params = parameters
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let directory = executable.parent().map(|path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    });
    let directory_ptr = directory
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());

    let result: HINSTANCE = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            directory_ptr,
            SW_SHOWNORMAL as SHOW_WINDOW_CMD,
        )
    };
    // ShellExecute returns a value <= 32 for failure.  The exact low value is
    // intentionally included in the diagnostic because Windows maps UAC
    // cancellation and policy denial differently across versions.
    if (result as isize) <= 32 {
        let code = result as isize;
        let message = format!(
            "Windows 未能启动管理员确认（ShellExecute 错误码 {code}）。请确认当前登录账户具备管理员权限，并在 UAC 提示中选择“是”。"
        );
        show_elevation_error(&message);
        return Err(message);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Quote one Windows command-line argument using the CommandLineToArgvW
/// rules.  We keep the quoting local rather than passing a shell script, so
/// the elevation boundary does not involve PowerShell parsing or policy
/// changes.
#[cfg(target_os = "windows")]
fn quote_windows_arg(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    let mut output = String::with_capacity(text.len() + 2);
    output.push('"');
    let mut backslashes = 0_usize;
    for character in text.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                output.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                output.push('"');
                backslashes = 0;
            }
            _ => {
                output.extend(std::iter::repeat_n('\\', backslashes));
                output.push(character);
                backslashes = 0;
            }
        }
    }
    output.extend(std::iter::repeat_n('\\', backslashes * 2));
    output.push('"');
    output
}

#[cfg(target_os = "windows")]
fn show_elevation_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title = wide_null("Codex App 管理器");
    let body = wide_null(&format!(
        "需要管理员确认才能安装到 U 盘 AppX 卷。\n\n{message}"
    ));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{should_relaunch_usb_manager, ElevationAction};

    #[test]
    fn only_an_unelevated_usb_entry_without_marker_is_relaunched() {
        assert!(should_relaunch_usb_manager(true, false, false));
        assert!(!should_relaunch_usb_manager(false, false, false));
        assert!(!should_relaunch_usb_manager(true, true, false));
        assert!(!should_relaunch_usb_manager(true, false, true));
    }

    #[test]
    fn action_values_are_explicit() {
        assert_ne!(ElevationAction::Continue, ElevationAction::Relaunched);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn deny_only_admin_groups_never_grant_elevation() {
        assert!(super::group_is_effective(4, 4));
        assert!(!super::group_is_effective(0, 4));
        assert!(!super::group_is_effective(0x10, 4));
        assert!(!super::group_is_effective(0x14, 4));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_elevation_probe_reads_the_real_primary_token() {
        // Safe read-only probe; exercises the primary token API contract.
        super::current_process_is_elevated().expect("read primary token elevation");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn usb_singleton_uses_a_separate_local_name_and_rejects_existing_instances() {
        // Pure classification only: never create or open the live mutex in
        // tests, since a real USB manager may be running on this machine.
        assert_eq!(
            super::USB_INSTANCE_MUTEX_NAME,
            r"Local\CodexUsbInstaller.FullAdmin.v1"
        );
        assert!(super::validate_usb_instance_creation(true, 0).is_ok());
        let existing = super::validate_usb_instance_creation(
            true,
            windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS,
        )
        .unwrap_err();
        assert!(existing.contains("已有 U 盘安装器"));
        assert!(super::validate_usb_instance_creation(
            false,
            windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
        )
        .is_err());
    }
}
