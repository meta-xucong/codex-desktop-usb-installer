//! Offline Windows runtime bootstrap for the USB entry point and CCSwitch.
//!
//! The manager is linked against the static MSVC CRT (see `.cargo/config.toml`)
//! so the entry point can start on a machine that does not have the VC runtime
//! installed yet.  CCSwitch is a separately-built Windows executable and still
//! uses the dynamic CRT, therefore this module performs a small preflight and,
//! when necessary, runs the unmodified Microsoft redistributable bundled in
//! the release layout.  We deliberately never copy individual DLLs to a system
//! directory and never download a prerequisite at runtime.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::Digest as _;

use crate::errors::AppError;

pub const VC_REDIST_FILE_NAME: &str = "vc_redist.x64.exe";
pub const VC_REDIST_SHA256: &str =
    "843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C";
pub const WEBVIEW2_INSTALLER_FILE_NAME: &str = "MicrosoftEdgeWebView2RuntimeInstallerX64.exe";
pub const WEBVIEW2_INSTALLER_SHA256: &str =
    "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D";
const VC_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const WEBVIEW2_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

const REQUIRED_DLLS: &[&str] = &["VCRUNTIME140.dll", "VCRUNTIME140_1.dll", "MSVCP140.dll"];
const ACCEPTED_INSTALL_EXIT_CODES: &[i32] = &[0, 1638, 1641, 3010];
const MSI_BUSY_EXIT_CODE: i32 = 1618;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A structured, non-secret status used by the native callers and tests.  The
/// frontend does not need to know the installer path, but including it in the
/// status makes diagnostics and support bundles actionable.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcRuntimeStatus {
    pub ready: bool,
    pub installed_now: bool,
    pub installer_found: bool,
    pub installer_path: Option<String>,
    pub exit_code: Option<i32>,
    pub message: String,
}

fn status_ready(message: impl Into<String>) -> VcRuntimeStatus {
    VcRuntimeStatus {
        ready: true,
        installed_now: false,
        installer_found: false,
        installer_path: None,
        exit_code: None,
        message: message.into(),
    }
}

fn status_missing(message: impl Into<String>) -> VcRuntimeStatus {
    VcRuntimeStatus {
        ready: false,
        installed_now: false,
        installer_found: false,
        installer_path: None,
        exit_code: None,
        message: message.into(),
    }
}

/// Return candidate paths in deterministic priority order.  A sibling of the
/// running executable is the portable USB layout; `resource_dir` covers a
/// future Tauri bundle in which resources are extracted by the framework.
pub fn installer_candidates(executable: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    prerequisite_candidates(executable, resource_dir, VC_REDIST_FILE_NAME)
}

fn prerequisite_candidates(
    executable: &Path,
    resource_dir: Option<&Path>,
    file_name: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = executable.parent() {
        candidates.push(
            parent
                .join("resources")
                .join("prerequisites")
                .join(file_name),
        );
        candidates.push(parent.join("prerequisites").join(file_name));
    }
    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join("prerequisites").join(file_name));
        candidates.push(resource_dir.join(file_name));
    }
    candidates
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn find_installer(executable: &Path, resource_dir: Option<&Path>) -> Option<PathBuf> {
    installer_candidates(executable, resource_dir)
        .into_iter()
        .find(|candidate| is_regular_file(candidate))
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut source = fs::File::open(path).map_err(|error| {
        AppError::Engine(format!("读取组件安装包 {} 失败: {error}", path.display()))
    })?;
    let mut hash = sha2::Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = source.read(&mut buffer).map_err(|error| {
            AppError::Engine(format!("校验组件安装包 {} 失败: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let digest = hash.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(target_os = "windows")]
fn library_loadable(name: &str) -> bool {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut system_directory = [0u16; 32768];
    let length = unsafe {
        GetSystemDirectoryW(system_directory.as_mut_ptr(), system_directory.len() as u32)
    } as usize;
    if length == 0 || length >= system_directory.len() {
        return false;
    }
    let mut wide = system_directory[..length].to_vec();
    wide.push(b'\\' as u16);
    wide.extend(name.encode_utf16());
    wide.push(0);
    // Probe only System32; a same-named DLL beside the USB executable must
    // not be accepted merely because another dependency already loaded it.
    unsafe {
        let handle = LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if handle.is_null() {
            return false;
        }
        let _ = FreeLibrary(handle);
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn library_loadable(_name: &str) -> bool {
    // The helper is only used by the Windows CCSwitch path.  Treating it as
    // ready elsewhere keeps cross-platform unit tests and macOS builds free of
    // Windows loader assumptions.
    true
}

pub fn required_dlls() -> &'static [&'static str] {
    REQUIRED_DLLS
}

pub fn runtime_ready() -> bool {
    REQUIRED_DLLS.iter().all(|name| library_loadable(name))
}

fn accepted_exit_code(code: i32) -> bool {
    ACCEPTED_INSTALL_EXIT_CODES.contains(&normalized_installer_exit_code(code))
}

fn normalized_installer_exit_code(code: i32) -> i32 {
    // Burn/Edge installers can return HRESULT_FROM_WIN32(code), represented
    // by Rust as a negative i32, rather than the raw MSI result.
    if (code as u32 & 0xffff_0000) == 0x8007_0000 {
        (code as u32 & 0xffff) as i32
    } else {
        code
    }
}

fn run_installer(
    installer: &Path,
    args: &[&str],
    label: &str,
    timeout: Duration,
) -> Result<i32, AppError> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let mut child = Command::new(installer)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| {
                AppError::Engine(format!(
                    "启动 {label} 离线安装包 {} 失败: {error}",
                    installer.display()
                ))
            })?;
        let pid = child.id();
        poll_installer_exit(
            || {
                child
                    .try_wait()
                    .map(|status| status.map(|value| value.code().unwrap_or(-1)))
            },
            label,
            pid,
            timeout,
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (installer, args, label, timeout);
        Err(AppError::UnsupportedPlatform)
    }
}

fn poll_installer_exit(
    mut poll: impl FnMut() -> std::io::Result<Option<i32>>,
    label: &str,
    pid: u32,
    timeout: Duration,
) -> Result<i32, AppError> {
    let started = Instant::now();
    loop {
        if let Some(code) = poll()
            .map_err(|error| AppError::Engine(format!("等待 {label} 安装结果失败: {error}")))?
        {
            return Ok(code);
        }
        if started.elapsed() >= timeout {
            // Never kill an MSI/Edge servicing transaction mid-write. The
            // caller stops startup and reports the exact tracked PID instead.
            return Err(AppError::Engine(format!("{label} 离线安装等待超过 {} 秒（PID {pid}）；本次启动已停止，组件安装可能仍在进行。请等待 Windows 安装结束后重试；不要连续重复启动安装器。", timeout.as_secs())));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_for_runtime(timeout: Duration) -> bool {
    let interval = Duration::from_millis(250);
    let mut elapsed = Duration::ZERO;
    while elapsed <= timeout {
        if runtime_ready() {
            return true;
        }
        thread::sleep(interval);
        elapsed += interval;
    }
    runtime_ready()
}

/// Ensure the dynamic VC++ runtime needed by CCSwitch is available.
///
/// A missing bundled installer is intentionally returned as a `ready=false`
/// status. USB startup reports it with a native dialog; ordinary installed
/// managers may still show diagnostics. CCSwitch callers must reject it.
pub fn ensure_bundled_vc_runtime(
    executable: &Path,
    resource_dir: Option<&Path>,
) -> Result<VcRuntimeStatus, AppError> {
    if runtime_ready() {
        return Ok(status_ready("VC++ 运行库已就绪"));
    }

    let Some(installer) = find_installer(executable, resource_dir) else {
        return Ok(status_missing(format!(
            "缺少 VCRUNTIME140.dll；未找到离线运行库 {}，请使用完整 U 盘目录重试",
            VC_REDIST_FILE_NAME
        )));
    };

    let actual = sha256_file(&installer)?;
    if !actual.eq_ignore_ascii_case(VC_REDIST_SHA256) {
        return Err(AppError::Engine(format!(
            "VC++ 运行库安装包校验失败: {actual} != {VC_REDIST_SHA256}"
        )));
    }

    let exit_code = run_installer(
        &installer,
        &["/install", "/quiet", "/norestart"],
        "VC++ 运行库",
        VC_INSTALL_TIMEOUT,
    )?;
    if !accepted_exit_code(exit_code) {
        let message = if normalized_installer_exit_code(exit_code) == MSI_BUSY_EXIT_CODE {
            "VC++ 运行库安装被 Windows Installer 占用（退出码 1618），请关闭其他安装程序后重试"
                .to_string()
        } else {
            format!("VC++ 运行库离线安装失败（退出码 {exit_code}），请以管理员权限重试")
        };
        return Err(AppError::Engine(message));
    }

    if !wait_for_runtime(Duration::from_secs(8)) {
        return Err(AppError::Engine(
            "VC++ 运行库安装程序已退出，但仍未找到 VCRUNTIME140.dll/VCRUNTIME140_1.dll；请重启后重试"
                .to_string(),
        ));
    }

    Ok(VcRuntimeStatus {
        ready: true,
        installed_now: true,
        installer_found: true,
        installer_path: Some(installer.to_string_lossy().into_owned()),
        exit_code: Some(exit_code),
        message: "已从 U 盘自动安装 VC++ 运行库".to_string(),
    })
}

/// Ask the very same statically-linked WebView2 loader used by Wry, before
/// any Tauri window exists. Registry entries alone can survive a broken or
/// partially removed runtime, and the Edge browser is not the runtime.
#[cfg(target_os = "windows")]
fn webview2_version() -> Option<String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    use windows_core::{PCWSTR, PWSTR};
    let mut version = PWSTR::null();
    let result =
        unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) };
    let value = webview2_com::take_pwstr(version);
    (result.is_ok() && usable_webview2_version(&value)).then_some(value)
}

fn usable_webview2_version(version: &str) -> bool {
    // The native browser-accelerator safety gate needs Settings3. Runtime
    // 109 is a conservative baseline covering that interface on Win10+.
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 109)
}

#[cfg(target_os = "windows")]
pub fn ensure_bundled_webview2_runtime(
    executable: &Path,
    resource_dir: Option<&Path>,
) -> Result<(), AppError> {
    if let Some(version) = webview2_version() {
        eprintln!("WebView2 runtime ready: {version}");
        return Ok(());
    }
    let installer = prerequisite_candidates(executable, resource_dir, WEBVIEW2_INSTALLER_FILE_NAME)
        .into_iter().find(|path| is_regular_file(path))
        .ok_or_else(|| AppError::Engine(format!("缺少或损坏 Microsoft Edge WebView2 Runtime，无法显示安装界面。完整离线包必须包含 resources\\prerequisites\\{WEBVIEW2_INSTALLER_FILE_NAME}；请复制完整目录后重试（不是约 2 MB 的在线 Bootstrapper）。")))?;
    let actual = sha256_file(&installer)?;
    if !actual.eq_ignore_ascii_case(WEBVIEW2_INSTALLER_SHA256) {
        return Err(AppError::Engine(format!(
            "WebView2 离线安装包校验失败: {actual} != {WEBVIEW2_INSTALLER_SHA256}"
        )));
    }
    // This exact, unmodified standalone installer is Authenticode-verified
    // during release creation and hash-pinned here. No network trust lookup
    // is required at offline startup.
    let code = run_installer(
        &installer,
        &["/silent", "/install"],
        "WebView2 Runtime",
        WEBVIEW2_INSTALL_TIMEOUT,
    )?;
    let started = Instant::now();
    loop {
        if let Some(version) = webview2_version() {
            eprintln!("WebView2 runtime ready after offline setup: {version}, exit={code}");
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(10) {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(AppError::Engine(format!("WebView2 离线安装后仍不可用（退出码 {code}, 0x{:08X}）。请检查 Windows 安装限制、磁盘空间或待重启状态后重试；安装器未继续部署 Codex。", code as u32)))
}

#[cfg(target_os = "windows")]
pub fn show_startup_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let title: Vec<u16> = "Codex App 管理器 — 必要组件检查"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let body: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
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
    use super::*;

    #[test]
    fn installer_candidates_prefer_usb_resources() {
        let paths = installer_candidates(
            Path::new(r"F:\usb-oneclick\CodexDesktopSetup.exe"),
            Some(Path::new(r"C:\Program Files\CodexManager\resources")),
        );
        assert_eq!(
            paths[0],
            Path::new(r"F:\usb-oneclick\resources\prerequisites\vc_redist.x64.exe")
        );
        assert!(paths.iter().any(|path| {
            path == Path::new(
                r"C:\Program Files\CodexManager\resources\prerequisites\vc_redist.x64.exe",
            )
        }));
    }

    #[test]
    fn runtime_hash_is_a_sha256_hex_digest() {
        assert_eq!(VC_REDIST_SHA256.len(), 64);
        assert!(VC_REDIST_SHA256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn only_expected_redist_exit_codes_are_accepted() {
        for code in [0, 1638, 1641, 3010] {
            assert!(accepted_exit_code(code));
        }
        for code in [-1, 1, 1603, 1618, 3011] {
            assert!(!accepted_exit_code(code));
        }
        assert!(accepted_exit_code(0x8007_0666u32 as i32));
        assert_eq!(normalized_installer_exit_code(0x8007_0652u32 as i32), 1618);
        assert!(!accepted_exit_code(0x8007_0643u32 as i32));
        assert_eq!(
            normalized_installer_exit_code(0x8000_4005u32 as i32),
            0x8000_4005u32 as i32
        );
    }

    #[test]
    fn prerequisite_wait_is_bounded_without_running_or_killing_installers() {
        let mut polls = 0;
        let result = poll_installer_exit(
            || {
                polls += 1;
                Ok(None)
            },
            "test",
            1234,
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(polls, 1);
        assert!(result.to_string().contains("PID 1234"));
        assert!(result.to_string().contains("可能仍在进行"));
        assert_eq!(
            poll_installer_exit(|| Ok(Some(3010)), "test", 1234, Duration::ZERO).unwrap(),
            3010
        );
        assert!(poll_installer_exit(
            || Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "probe denied"
            )),
            "test",
            1234,
            Duration::ZERO
        )
        .unwrap_err()
        .to_string()
        .contains("probe denied"));
    }

    #[test]
    fn webview2_requires_supported_version_and_pinned_full_installer() {
        assert!(usable_webview2_version("150.0.1.2"));
        assert!(usable_webview2_version("109.0.1518.78"));
        for invalid in ["", "0.0.0.0", "108.0.0.0", "not-installed"] {
            assert!(!usable_webview2_version(invalid));
        }
        assert_eq!(WEBVIEW2_INSTALLER_SHA256.len(), 64);
        assert!(WEBVIEW2_INSTALLER_SHA256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            prerequisite_candidates(
                Path::new(r"F:\usb-oneclick\CodexAppManager.exe"),
                None,
                WEBVIEW2_INSTALLER_FILE_NAME
            )[0],
            Path::new(
                r"F:\usb-oneclick\resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
            )
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_webview_probe_needs_no_ui_or_installer() {
        // Read-only host probe: a missing runtime is a valid result on clean
        // Windows. Do not call ensure_bundled_* in this test.
        if let Some(version) = webview2_version() {
            assert!(usable_webview2_version(&version));
        }
    }

    #[test]
    fn missing_installer_is_nonfatal_status() {
        let status = ensure_bundled_vc_runtime(
            Path::new(r"F:\usb-oneclick\CodexDesktopSetup.exe"),
            Some(Path::new(r"C:\does-not-exist")),
        )
        .expect("missing prerequisite should be represented as status");
        // On a host that already has the runtime the preflight is ready; on a
        // minimal host it must be a clear nonfatal missing-installer status.
        assert!(status.ready || !status.installer_found);
    }
}
