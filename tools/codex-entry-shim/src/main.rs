#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_UTF16_UNITS: usize = 16 * 1024;
const MAX_CWD_UTF16_UNITS: usize = 4 * 1024;
const CONNECT_ATTEMPTS: usize = 40;
// The manager allows up to five minutes for authorization and startup. Keep a
// small margin so the shim reports the manager's result instead of waiting
// forever on a disconnected or wedged pipe.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(330);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryRequest {
    schema_version: u32,
    request_id: String,
    args: Vec<String>,
    current_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryResponse {
    ok: bool,
    message: String,
}

fn main() {
    if let Err(error) = run() {
        show_error(&error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("无法确定直接入口位置：{error}"))?
        .canonicalize()
        .map_err(|error| format!("无法规范化直接入口位置：{error}"))?;
    let bundle_root = resolve_bundle_root(&executable)?;
    let request = build_request()?;
    let request_bytes =
        serde_json::to_vec(&request).map_err(|error| format!("无法编码启动请求：{error}"))?;
    if request_bytes.is_empty() || request_bytes.len() > MAX_FRAME_BYTES {
        return Err("启动参数过多，直接入口无法安全转发。".to_string());
    }
    let session_id = current_session_id()?;
    let pipe_name = pipe_name_for_bundle(&bundle_root, session_id)?;
    let framed = frame(&request_bytes);

    let mut manager_started = false;
    let mut last_error = "管理器通信管道尚未就绪".to_string();
    for attempt in 0..CONNECT_ATTEMPTS {
        match send_request(&pipe_name, &framed) {
            Ok(response) if response.ok => return Ok(()),
            Ok(response) => return Err(response.message),
            Err(SendRequestError::BeforeSubmission(error)) => {
                last_error = error;
                if !manager_started {
                    start_manager_with_uac(&bundle_root).map_err(|manager_error| {
                        format!("{manager_error} 管理器管道错误：{last_error}")
                    })?;
                    manager_started = true;
                }
                if attempt + 1 < CONNECT_ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
            Err(SendRequestError::AfterSubmission(error)) => {
                return Err(format!(
                    "启动请求已提交，但未收到管理器结果；为避免重复启动，本次不会重发请求。{error}"
                ));
            }
        }
    }
    Err(format!("无法连接 U 盘管理器；Codex 未启动。{last_error}"))
}

fn resolve_bundle_root(executable: &Path) -> Result<PathBuf, String> {
    let codex_dir = executable
        .parent()
        .ok_or_else(|| "直接入口没有父目录".to_string())?;
    let apps_dir = codex_dir
        .parent()
        .ok_or_else(|| "直接入口目录结构无效".to_string())?;
    let root = apps_dir
        .parent()
        .ok_or_else(|| "无法定位 U 盘 bundle 根目录".to_string())?;
    if !codex_dir
        .file_name()
        .is_some_and(|value| value.eq_ignore_ascii_case("Codex"))
        || !apps_dir
            .file_name()
            .is_some_and(|value| value.eq_ignore_ascii_case("apps"))
        || !executable.file_name().is_some_and(|value| {
            value.eq_ignore_ascii_case("ChatGPT.exe") || value.eq_ignore_ascii_case("Codex.exe")
        })
    {
        return Err("直接入口必须位于 apps\\Codex\\ChatGPT.exe 或 Codex.exe。".to_string());
    }
    for name in [
        "manifest.json",
        "usb-manifest.json",
        "CodexDesktopSetup.exe",
    ] {
        if !root.join(name).is_file() {
            return Err(format!("U 盘 bundle 缺少必要文件：{name}"));
        }
    }
    Ok(root
        .canonicalize()
        .map_err(|error| format!("规范化 U 盘根目录失败：{error}"))?)
}

fn build_request() -> Result<EntryRequest, String> {
    let args = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "启动参数包含无法安全转换的 Unicode 字符。".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.len() > MAX_ARGUMENTS
        || args.iter().any(|argument| argument.contains('\0'))
        || args
            .iter()
            .map(|argument| argument.encode_utf16().count())
            .sum::<usize>()
            > MAX_ARGUMENT_UTF16_UNITS
    {
        return Err("启动参数超过安全长度限制。".to_string());
    }
    let current_dir = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    if current_dir.as_ref().is_some_and(|path| {
        path.contains('\0') || path.encode_utf16().count() > MAX_CWD_UTF16_UNITS
    }) {
        return Err("当前工作目录超过安全长度限制。".to_string());
    }
    Ok(EntryRequest {
        schema_version: 1,
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        args,
        current_dir,
    })
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(payload.len() + 4);
    framed.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    framed.extend_from_slice(payload);
    framed
}

#[derive(Debug)]
enum SendRequestError {
    /// No request bytes were sent, so another connection attempt is safe.
    BeforeSubmission(String),
    /// The request may have reached the manager. Never retry it as a new
    /// launch solely because the response channel failed.
    AfterSubmission(String),
}

impl SendRequestError {
    #[cfg(test)]
    fn is_retryable(&self) -> bool {
        matches!(self, Self::BeforeSubmission(_))
    }
}

#[cfg(windows)]
fn send_request(pipe_name: &str, framed: &[u8]) -> Result<EntryResponse, SendRequestError> {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let wide = std::ffi::OsStr::new(pipe_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe { WaitNamedPipeW(wide.as_ptr(), 500) } == 0 {
        return Err(SendRequestError::BeforeSubmission(format!(
            "等待管理器管道失败（错误 {}）",
            unsafe { GetLastError() }
        )));
    }
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_name)
        .map_err(|error| {
            SendRequestError::BeforeSubmission(format!("打开管理器管道失败：{error}"))
        })?;
    pipe.write_all(framed).map_err(|error| {
        SendRequestError::AfterSubmission(format!("提交管理器启动请求失败：{error}"))
    })?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = read_response(pipe);
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(RESPONSE_TIMEOUT) {
        Ok(result) => result.map_err(SendRequestError::AfterSubmission),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(SendRequestError::AfterSubmission(
            format!("等待管理器响应超过 {} 秒。", RESPONSE_TIMEOUT.as_secs()),
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(
            SendRequestError::AfterSubmission("读取管理器响应的工作线程已退出。".to_string()),
        ),
    }
}

#[cfg(windows)]
fn read_response(mut pipe: std::fs::File) -> Result<EntryResponse, String> {
    use std::io::Read;
    let mut header = [0_u8; 4];
    pipe.read_exact(&mut header)
        .map_err(|error| format!("读取管理器响应失败：{error}"))?;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err("管理器响应超过安全长度限制。".to_string());
    }
    let mut bytes = vec![0_u8; length];
    pipe.read_exact(&mut bytes)
        .map_err(|error| format!("读取管理器响应内容失败：{error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("管理器响应格式无效：{error}"))
}

#[cfg(not(windows))]
fn send_request(_pipe_name: &str, _framed: &[u8]) -> Result<EntryResponse, SendRequestError> {
    Err(SendRequestError::BeforeSubmission(
        "U 盘直接入口只支持 Windows。".to_string(),
    ))
}

#[cfg(windows)]
fn start_manager_with_uac(bundle_root: &Path) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("CODEX_USB_ENTRY_TEST_UAC_CANCEL").is_some() {
        return Err("测试夹具模拟用户取消了 UAC；Codex 未启动。".to_string());
    }
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let setup = bundle_root.join("CodexDesktopSetup.exe");
    let verb = wide("runas");
    let setup_wide = setup
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let cwd_wide = bundle_root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            setup_wide.as_ptr(),
            std::ptr::null(),
            cwd_wide.as_ptr(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        let code = result as isize;
        let detail = if code == 1223 {
            "用户取消了 UAC 提示".to_string()
        } else {
            format!("Windows ShellExecute 错误 {code}（最后错误 {}）", unsafe {
                GetLastError()
            })
        };
        return Err(format!(
            "无法通过 Windows UAC 启动 U 盘管理器：{detail}。Codex 未启动。"
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn start_manager_with_uac(_bundle_root: &Path) -> Result<(), String> {
    Err("U 盘直接入口只支持 Windows。".to_string())
}

#[cfg(windows)]
fn current_session_id() -> Result<u32, String> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let mut session = 0_u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(format!(
            "无法读取 Windows 会话编号（错误 {}）",
            unsafe { GetLastError() }
        ));
    }
    Ok(session)
}

#[cfg(not(windows))]
fn current_session_id() -> Result<u32, String> {
    Err("U 盘直接入口只支持 Windows。".to_string())
}

fn pipe_name_for_bundle(root: &Path, session_id: u32) -> Result<String, String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("规范化管理器 IPC 根目录失败：{error}"))?;
    let folded = canonical.to_string_lossy().to_lowercase();
    let digest = Sha256::digest(folded.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(r"\\.\pipe\CodexUsbDirect-{session_id}-{suffix}"))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn show_error(message: &str) {
    if std::env::var_os("CODEX_USB_ENTRY_TEST_QUIET").is_some() {
        eprintln!("{message}");
        return;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let title = wide("Codex U 盘启动失败");
    let body = wide(message);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error(message: &str) {
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::{frame, MAX_FRAME_BYTES};

    #[test]
    fn frame_uses_bounded_little_endian_length_prefix() {
        let result = frame(b"hello");
        assert_eq!(&result[..4], &5_u32.to_le_bytes());
        assert_eq!(&result[4..], b"hello");
        assert!(result.len() < MAX_FRAME_BYTES);
    }

    #[test]
    fn only_pre_submission_transport_failures_are_retryable() {
        assert!(
            super::SendRequestError::BeforeSubmission("pipe unavailable".into()).is_retryable()
        );
        assert!(!super::SendRequestError::AfterSubmission("response lost".into()).is_retryable());
    }
}
