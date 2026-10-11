#![cfg(target_os = "windows")]

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::time::Duration;
use tauri::Manager;

const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_CURRENT_DIR_UTF16_UNITS: usize = 4 * 1024;
const MANAGER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryRequest {
    schema_version: u32,
    request_id: String,
    args: Vec<String>,
    current_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryResponse {
    ok: bool,
    message: String,
}

const MAX_CACHED_REQUESTS: usize = 64;

#[derive(Default)]
struct EntryResponseCache {
    entries: VecDeque<(String, EntryResponse)>,
}

impl EntryResponseCache {
    fn get(&self, request_id: &str) -> Option<EntryResponse> {
        self.entries
            .iter()
            .find(|(cached_id, _)| cached_id == request_id)
            .map(|(_, response)| response.clone())
    }

    fn insert(&mut self, request_id: String, response: EntryResponse) {
        self.entries
            .retain(|(cached_id, _)| cached_id != &request_id);
        self.entries.push_back((request_id, response));
        while self.entries.len() > MAX_CACHED_REQUESTS {
            self.entries.pop_front();
        }
    }
}

impl EntryResponse {
    fn ok() -> Self {
        Self {
            ok: true,
            message: "Codex 已通过 U 盘管理器启动。".to_string(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

pub fn start(app: tauri::AppHandle) {
    if !crate::app::usb_guard::guard_required() {
        return;
    }
    let Some(bundle_root) = crate::app::usb_guard::current_bundle_root() else {
        log::error!("USB direct-entry IPC did not find the manager bundle root");
        return;
    };
    let shim = bundle_root.join("CodexEntryShim.exe");
    if !shim.is_file() {
        log::error!(
            "USB direct-entry IPC is unavailable because the bundle shim is missing path={}",
            shim.display()
        );
        return;
    }
    let session = match current_session_id() {
        Ok(session) => session,
        Err(error) => {
            log::error!("USB direct-entry IPC cannot resolve the Windows session: {error}");
            return;
        }
    };
    let shell_user_sid = match shell_user_sid(session) {
        Ok(sid) => sid,
        Err(error) => {
            log::error!("USB direct-entry IPC cannot resolve the desktop user SID: {error}");
            return;
        }
    };
    let pipe_name = match pipe_name_for_bundle(&bundle_root, session) {
        Ok(name) => name,
        Err(error) => {
            log::error!("USB direct-entry IPC cannot derive its pipe name: {error}");
            return;
        }
    };
    log::info!("USB direct-entry IPC starting session={session}");
    let spawn = std::thread::Builder::new()
        .name("codex-usb-entry-ipc".to_string())
        .spawn(move || server_loop(&pipe_name, session, &shell_user_sid, app));
    if let Err(error) = spawn {
        log::error!("USB direct-entry IPC thread could not start: {error}");
    }
}

fn server_loop(pipe_name: &str, session_id: u32, shell_user_sid: &str, app: tauri::AppHandle) {
    let mut first_instance = true;
    let mut response_cache = EntryResponseCache::default();
    loop {
        let handle = match create_named_pipe(pipe_name, shell_user_sid, first_instance) {
            Ok(handle) => handle,
            Err(error) => {
                log::error!("USB direct-entry IPC pipe creation failed: {error}");
                return;
            }
        };
        first_instance = false;
        if let Err(error) = connect_named_pipe(handle) {
            log::warn!("USB direct-entry IPC client connection failed: {error}");
            close_handle(handle);
            continue;
        }
        let response = match validate_client_session(handle, session_id, shell_user_sid) {
            Ok(()) => {
                let mut pipe = unsafe { std::fs::File::from_raw_handle(handle as _) };
                handle_connection(&mut pipe, app.clone(), &mut response_cache)
            }
            Err(error) => {
                let response = EntryResponse::error(error);
                let mut pipe = unsafe { std::fs::File::from_raw_handle(handle as _) };
                let _ = write_response(&mut pipe, &response);
                continue;
            }
        };
        if let Err(error) = response {
            log::warn!("USB direct-entry IPC request rejected: {error}");
        }
    }
}

fn handle_connection(
    pipe: &mut std::fs::File,
    app: tauri::AppHandle,
    response_cache: &mut EntryResponseCache,
) -> Result<(), String> {
    let response = match read_request(pipe) {
        Ok(request) => {
            dispatch_cached_request(request, response_cache, || launch_through_manager(app))
        }
        Err(error) => EntryResponse::error(error),
    };
    write_response(pipe, &response)
}

fn dispatch_cached_request(
    request: EntryRequest,
    response_cache: &mut EntryResponseCache,
    launch: impl FnOnce() -> Result<(), String>,
) -> EntryResponse {
    let request_id = request.request_id.clone();
    let cacheable = valid_request_id(&request_id);
    if cacheable {
        if let Some(response) = response_cache.get(&request_id) {
            log::info!("USB direct-entry duplicate request reused cached response");
            return response;
        }
    }
    let response = dispatch_request(request, launch);
    if cacheable {
        response_cache.insert(request_id, response.clone());
    }
    response
}

fn dispatch_request(
    request: EntryRequest,
    launch: impl FnOnce() -> Result<(), String>,
) -> EntryResponse {
    if request.schema_version != 1 || !valid_request_id(&request.request_id) {
        return EntryResponse::error("U 盘直接入口请求格式无效，Codex 未启动。");
    }
    if !request.args.is_empty() {
        return EntryResponse::error("U 盘直接入口不支持附带启动参数，Codex 未启动。");
    }
    if request.current_dir.as_ref().is_some_and(|path| {
        path.contains('\0') || path.encode_utf16().count() > MAX_CURRENT_DIR_UTF16_UNITS
    }) {
        return EntryResponse::error("U 盘直接入口工作目录无效，Codex 未启动。");
    }
    match launch() {
        Ok(()) => EntryResponse::ok(),
        Err(error) => EntryResponse::error(error),
    }
}

fn valid_request_id(request_id: &str) -> bool {
    request_id.len() == 32 && request_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn launch_through_manager(app: tauri::AppHandle) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::state::ManagerState>();
        let result = crate::commands::win_launch_codex(state, None)
            .await
            .map_err(|error| error.message);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(MANAGER_LAUNCH_TIMEOUT)
        .map_err(|error| format!("等待管理器完成授权和启动超时或失败：{error}"))?
}

fn read_request(pipe: &mut std::fs::File) -> Result<EntryRequest, String> {
    let mut header = [0_u8; 4];
    pipe.read_exact(&mut header)
        .map_err(|error| format!("读取直接入口请求长度失败：{error}"))?;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err("直接入口请求超过安全长度限制。".to_string());
    }
    let mut bytes = vec![0; length];
    pipe.read_exact(&mut bytes)
        .map_err(|error| format!("读取直接入口请求失败：{error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("直接入口请求格式无效：{error}"))
}

fn write_response(pipe: &mut std::fs::File, response: &EntryResponse) -> Result<(), String> {
    let payload =
        serde_json::to_vec(response).map_err(|error| format!("编码管理器启动响应失败：{error}"))?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err("管理器启动响应超过安全长度限制。".to_string());
    }
    pipe.write_all(&(payload.len() as u32).to_le_bytes())
        .and_then(|()| pipe.write_all(&payload))
        .and_then(|()| pipe.flush())
        .map_err(|error| format!("发送管理器启动响应失败：{error}"))
}

fn pipe_name_for_bundle(root: &Path, session_id: u32) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("规范化 U 盘根目录失败：{error}"))?;
    let folded = canonical.to_string_lossy().to_lowercase();
    let digest = Sha256::digest(folded.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(r"\\.\pipe\CodexUsbDirect-{session_id}-{suffix}"))
}

#[cfg(windows)]
fn current_session_id() -> Result<u32, String> {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let mut session = 0_u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(format!(
            "无法读取 Windows 会话编号（错误 {}）",
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        ));
    }
    Ok(session)
}

#[cfg(windows)]
fn validate_client_session(
    handle: windows_sys::Win32::Foundation::HANDLE,
    expected: u32,
    expected_user_sid: &str,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut client_pid = 0_u32;
    if unsafe { GetNamedPipeClientProcessId(handle, &mut client_pid) } == 0 {
        return Err(format!("无法确认 IPC 客户端进程（错误 {}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let mut client_session = 0_u32;
    if unsafe { ProcessIdToSessionId(client_pid, &mut client_session) } == 0
        || client_session != expected
    {
        return Err("IPC 客户端不属于当前 Windows 会话，Codex 未启动。".to_string());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, client_pid) };
    if process.is_null() {
        return Err(format!("无法核验 IPC 客户端用户（错误 {}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let client_sid = process_user_sid(process);
    unsafe {
        CloseHandle(process);
    }
    if client_sid? != expected_user_sid {
        return Err("IPC 客户端不属于当前桌面用户，Codex 未启动。".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn shell_user_sid(expected_session: u32) -> Result<String, String> {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
    let window = unsafe { GetShellWindow() };
    if window.is_null() {
        return Err("当前 Windows 会话没有可见桌面 Shell".to_string());
    }
    let mut process_id = 0_u32;
    if unsafe { GetWindowThreadProcessId(window, &mut process_id) } == 0 || process_id == 0 {
        return Err(format!("无法识别桌面 Shell 进程（错误 {}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let mut shell_session = 0_u32;
    if unsafe { ProcessIdToSessionId(process_id, &mut shell_session) } == 0
        || shell_session != expected_session
    {
        return Err("桌面 Shell 不属于当前 Windows 会话".to_string());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(format!("无法打开桌面 Shell 进程（错误 {}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let result = process_user_sid(process);
    unsafe {
        windows_sys::Win32::Foundation::CloseHandle(process);
    }
    result
}

#[cfg(windows)]
fn process_user_sid(process: windows_sys::Win32::Foundation::HANDLE) -> Result<String, String> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::OpenProcessToken;
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!("无法读取进程用户令牌（错误 {}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let result = (|| {
        let mut needed = 0_u32;
        let probe =
            unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        if probe != 0
            || unsafe { windows_sys::Win32::Foundation::GetLastError() }
                != ERROR_INSUFFICIENT_BUFFER
            || needed < size_of::<TOKEN_USER>() as u32
        {
            return Err("无法读取进程用户 SID 的长度".to_string());
        }
        let mut storage = vec![0_usize; (needed as usize).div_ceil(size_of::<usize>())];
        let mut returned = 0_u32;
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast::<c_void>(),
                needed,
                &mut returned,
            )
        } == 0
            || returned as usize > storage.len() * size_of::<usize>()
        {
            return Err(format!("无法读取进程用户 SID（错误 {}）", unsafe {
                windows_sys::Win32::Foundation::GetLastError()
            }));
        }
        // SAFETY: GetTokenInformation returned a TOKEN_USER in aligned storage.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        if user.User.Sid.is_null() {
            return Err("进程用户令牌缺少 SID".to_string());
        }
        let mut string_sid = std::ptr::null_mut();
        if unsafe {
            windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW(
                user.User.Sid,
                &mut string_sid,
            )
        } == 0
            || string_sid.is_null()
        {
            return Err(format!("无法格式化进程用户 SID（错误 {}）", unsafe {
                windows_sys::Win32::Foundation::GetLastError()
            }));
        }
        let mut length = 0_usize;
        unsafe {
            while *string_sid.add(length) != 0 {
                length += 1;
            }
        }
        let value =
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string_sid, length) });
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(string_sid as HLOCAL);
        }
        Ok(value)
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}

#[cfg(windows)]
fn create_named_pipe(
    name: &str,
    shell_user_sid: &str,
    first_instance: bool,
) -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GetLastError, LocalFree, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let wide = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let open_mode = PIPE_ACCESS_DUPLEX
        | if first_instance {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // The manager can be elevated with alternate administrator credentials.
    // Grant only the current desktop user's SID and verify that SID again for
    // each client. This keeps that supported UAC path working without opening
    // the pipe to every interactive user.
    let sddl = format!("D:P(A;;GA;;;{shell_user_sid})S:(ML;;NW;;;ME)");
    let sddl = std::ffi::OsStr::new(&sddl)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut security_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut security_descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "无法为 U 盘直接入口管道创建安全描述符（错误 {}）",
            unsafe { GetLastError() }
        ));
    }
    let mut security_attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_FRAME_BYTES as u32,
            MAX_FRAME_BYTES as u32,
            0,
            &mut security_attributes,
        )
    };
    let last_error = if handle == INVALID_HANDLE_VALUE {
        unsafe { GetLastError() }
    } else {
        0
    };
    unsafe {
        LocalFree(security_descriptor);
    }
    if handle == INVALID_HANDLE_VALUE {
        Err(format!("创建本地管理器管道失败（错误 {last_error}）"))
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn connect_named_pipe(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED};
    use windows_sys::Win32::System::Pipes::ConnectNamedPipe;
    if unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0 {
        return Ok(());
    }
    let error = unsafe { GetLastError() };
    if error == ERROR_PIPE_CONNECTED {
        Ok(())
    } else {
        Err(format!("等待本地直接入口请求失败（错误 {error}）"))
    }
}

#[cfg(windows)]
fn close_handle(handle: windows_sys::Win32::Foundation::HANDLE) {
    unsafe {
        windows_sys::Win32::Foundation::CloseHandle(handle);
    }
}

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn request(args: Vec<String>) -> EntryRequest {
        EntryRequest {
            schema_version: 1,
            request_id: "0123456789abcdef0123456789abcdef".to_string(),
            args,
            current_dir: None,
        }
    }

    #[test]
    fn active_and_offline_paths_reach_the_shared_manager_launch_handler() {
        for _status in ["active", "offline"] {
            let mut called = false;
            let response = dispatch_request(request(Vec::new()), || {
                called = true;
                Ok(())
            });
            assert!(response.ok);
            assert!(called);
        }
    }

    #[test]
    fn revoked_response_is_denied_and_invalid_requests_never_reach_launch_handler() {
        let mut called = false;
        let revoked = dispatch_request(request(Vec::new()), || {
            called = true;
            Err("已收到已验签的作废状态，Codex 已锁定。".to_string())
        });
        assert!(!revoked.ok);
        assert!(called);

        called = false;
        let invalid = dispatch_request(request(vec!["--some-arg".to_string()]), || {
            called = true;
            Ok(())
        });
        assert!(!invalid.ok);
        assert!(!called);
    }

    #[test]
    fn duplicate_submitted_request_reuses_response_without_launching_again() {
        let mut cache = EntryResponseCache::default();
        let calls = std::cell::Cell::new(0);
        let first = dispatch_cached_request(request(Vec::new()), &mut cache, || {
            calls.set(calls.get() + 1);
            Err("Codex 核心启动失败".to_string())
        });
        let second = dispatch_cached_request(request(Vec::new()), &mut cache, || {
            calls.set(calls.get() + 1);
            Ok(())
        });

        assert!(!first.ok);
        assert_eq!(first, second);
        assert_eq!(calls.get(), 1);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    #[ignore = "spawns the compiled USB shim and a Windows named-pipe test server"]
    fn compiled_shim_named_pipe_active_revoked_and_offline_e2e() {
        // The PowerShell harness supplies the actual compiled shim path and
        // invokes this test on Windows. The manager adapter is the same
        // dispatch function used by the production pipe server.
        run_compiled_shim_e2e().expect("compiled shim IPC E2E");
    }

    #[test]
    #[ignore = "spawns the compiled debug USB shim to exercise fail-closed UAC cancellation"]
    fn compiled_shim_uac_cancel_path_fails_closed_without_core() {
        run_compiled_shim_uac_cancel().expect("compiled shim UAC cancellation");
    }

    fn run_compiled_shim_e2e() -> Result<(), String> {
        use std::process::Command;
        let shim = std::env::var_os("CODEX_USB_ENTRY_SHIM_PATH")
            .map(PathBuf::from)
            .ok_or_else(|| "CODEX_USB_ENTRY_SHIM_PATH missing".to_string())?;
        for entry_name in ["ChatGPT.exe", "Codex.exe"] {
            for (mode, should_succeed) in [("active", true), ("offline", true), ("revoked", false)]
            {
                let root = std::env::temp_dir().join(format!(
                    "codex-usb-entry-{entry_name}-{mode}-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4().simple()
                ));
                let entry = root.join("apps/Codex").join(entry_name);
                std::fs::create_dir_all(entry.parent().unwrap())
                    .map_err(|error| format!("create shim fixture: {error}"))?;
                std::fs::copy(&shim, &entry)
                    .map_err(|error| format!("copy shim fixture: {error}"))?;
                for name in [
                    "manifest.json",
                    "usb-manifest.json",
                    "CodexDesktopSetup.exe",
                ] {
                    std::fs::write(root.join(name), b"fixture")
                        .map_err(|error| format!("write bundle fixture: {error}"))?;
                }
                let session = current_session_id()?;
                let shell_sid = shell_user_sid(session)?;
                let pipe = pipe_name_for_bundle(&root, session)?;
                let server_mode = mode.to_string();
                let server_sid = shell_sid.clone();
                let server = std::thread::spawn(move || {
                    let handle = create_named_pipe(&pipe, &server_sid, true)?;
                    connect_named_pipe(handle)?;
                    validate_client_session(handle, session, &server_sid)?;
                    let mut stream = unsafe { std::fs::File::from_raw_handle(handle as _) };
                    let request = read_request(&mut stream)?;
                    let result = dispatch_request(request, || match server_mode.as_str() {
                        "active" | "offline" => Ok(()),
                        _ => Err("已收到已验签的作废状态，Codex 已锁定。".to_string()),
                    });
                    write_response(&mut stream, &result)?;
                    Ok::<(), String>(())
                });
                let status = Command::new(&entry)
                    .current_dir(&root)
                    .status()
                    .map_err(|error| format!("launch compiled shim for {mode}: {error}"))?;
                server
                    .join()
                    .map_err(|_| format!("{mode} named-pipe server panicked"))??;
                let _ = std::fs::remove_dir_all(&root);
                if status.success() != should_succeed {
                    return Err(format!(
                    "compiled {entry_name} shim returned unexpected exit code in {mode} mode: {status}"
                ));
                }
            }
        }
        Ok(())
    }

    fn run_compiled_shim_uac_cancel() -> Result<(), String> {
        use std::process::Command;
        let shim = std::env::var_os("CODEX_USB_ENTRY_CANCEL_SHIM_PATH")
            .map(PathBuf::from)
            .ok_or_else(|| "CODEX_USB_ENTRY_CANCEL_SHIM_PATH missing".to_string())?;
        let root = std::env::temp_dir().join(format!(
            "codex-usb-entry-uac-cancel-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let entry = root.join("apps/Codex/ChatGPT.exe");
        std::fs::create_dir_all(entry.parent().unwrap())
            .map_err(|error| format!("create UAC fixture: {error}"))?;
        std::fs::copy(shim, &entry).map_err(|error| format!("copy UAC shim: {error}"))?;
        for name in [
            "manifest.json",
            "usb-manifest.json",
            "CodexDesktopSetup.exe",
        ] {
            std::fs::write(root.join(name), b"fixture")
                .map_err(|error| format!("write UAC fixture: {error}"))?;
        }
        let status = Command::new(&entry)
            .env("CODEX_USB_ENTRY_TEST_UAC_CANCEL", "1")
            .env("CODEX_USB_ENTRY_TEST_QUIET", "1")
            .current_dir(&root)
            .status()
            .map_err(|error| format!("run UAC cancellation shim: {error}"))?;
        let _ = std::fs::remove_dir_all(&root);
        if status.success() {
            return Err("UAC cancellation unexpectedly allowed Codex launch".to_string());
        }
        Ok(())
    }
}
