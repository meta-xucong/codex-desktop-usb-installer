//! Windows AppX removable-volume support for the USB build.
//!
//! The Codex MSIX remains a signed, opaque package.  The USB installer only
//! chooses the AppX volume used by Windows; it never unpacks or patches the
//! package.  Windows normally places a packaged app's user data on the same
//! volume as the package.  The documented `RestrictAppDataToSystemVolume`
//! policy is therefore enabled for the USB edition to request local app data.
//! Reading the policy back is not proof that every Windows SKU implements it;
//! actual package and data placement remain separate acceptance checks.
//!
//! That policy is machine-wide.  We make the scope explicit: the previous
//! registry value is journaled under the current user's local app data and is
//! restored when this product's managed Codex package is uninstalled.  A UAC
//! prompt is handled by the USB entry before the manager starts. No hidden
//! service, scheduled task, or machine security-policy change is used. The
//! short-lived PowerShell child inherits that token and executes a fixed
//! inline command, not a privileged script in a user-writable temp directory.
//! `-ExecutionPolicy Bypass` is only a process-local compatibility switch; it never
//! writes execution-policy settings and does not disable security software.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "windows")]
use std::thread;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::atomic_file;
use crate::errors::AppError;

const POLICY_SUBKEY: &str = r"Software\Policies\Microsoft\Windows\Appx";
const POLICY_VALUE: &str = "RestrictAppDataToSystemVolume";
const JOURNAL_DIR: &str = "CodexUsbAppx";
const JOURNAL_FILE: &str = "data-policy.json";
const JOURNAL_SCHEMA_VERSION: u32 = 1;
const VOLUME_JOURNAL_PREFIX: &str = "volume-";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct VolumeJournal {
    schema_version: u32,
    product: String,
    /// Pending is written before Add-AppxVolume. Only a confirmed successful
    /// creation may become owned. A crash cannot turn an existing/shared volume
    /// into ours merely because its drive letter matches.
    state: String,
    volume_root: String,
    volume_id: String,
    volume_name: String,
    package_store_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppxSystemCleanupItem {
    pub target: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppxSystemCleanupReport {
    pub items: Vec<AppxSystemCleanupItem>,
    pub messages: Vec<String>,
    pub warnings: Vec<String>,
    pub pending: bool,
}

impl AppxSystemCleanupReport {
    fn record(&mut self, target: &str, status: &str, message: String) {
        let incomplete = matches!(status, "pending" | "failed");
        if incomplete {
            self.pending = true;
            self.warnings.push(message.clone());
        } else {
            self.messages.push(message.clone());
        }
        self.items.push(AppxSystemCleanupItem {
            target: target.to_owned(),
            status: status.to_owned(),
            message,
        });
    }
}

/// The volume helper is deliberately bounded; a stalled service must not
/// leave the Tauri operation waiting forever.
#[cfg(target_os = "windows")]
const ELEVATED_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// Removing a staged 800 MB package and unregistering its removable AppX
/// volume can legitimately outlive the small policy-operation budget on slow
/// media with Defender scanning. Keep this bounded, but never kill the system
/// deployment command at the old two-minute boundary.
const ELEVATED_USB_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[cfg(target_os = "windows")]
const ELEVATED_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PolicyJournal {
    schema_version: u32,
    product: String,
    previous_value: Option<u32>,
    applied_value: u32,
    applied_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppxDataPolicyReport {
    /// Current value observed after the operation. `1` means AppX data is
    /// restricted to the system volume; `0`/None means Windows may follow the
    /// package volume's default data location.
    pub current_value: Option<u32>,
    /// Whether this invocation changed the registry.
    pub changed: bool,
    /// Whether a journal owned by this product exists and can be restored.
    pub managed: bool,
    pub message: String,
}

fn local_dir() -> Result<PathBuf, AppError> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(root).join(JOURNAL_DIR));
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join(JOURNAL_DIR))
        .ok_or_else(|| AppError::Engine("无法确定 AppX 数据策略记录目录".to_string()))
}

fn journal_path() -> Result<PathBuf, AppError> {
    Ok(local_dir()?.join(JOURNAL_FILE))
}

fn read_journal() -> Result<Option<PolicyJournal>, AppError> {
    let path = journal_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::read(atomic_file::backup_path(&path)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(AppError::Engine(format!(
                        "读取 AppX 策略备份记录失败: {error}"
                    )))
                }
            }
        }
        Err(error) => return Err(AppError::Engine(format!("读取 AppX 策略记录失败: {error}"))),
    };
    let journal = serde_json::from_slice::<PolicyJournal>(&bytes).map_err(|error| {
        AppError::Engine(format!("AppX 策略记录损坏，停止自动覆盖策略: {error}"))
    })?;
    if journal.schema_version == JOURNAL_SCHEMA_VERSION
        && journal.product == "codex-usb-oneclick"
        && journal.applied_value == 1
    {
        Ok(Some(journal))
    } else {
        Err(AppError::Engine(
            "AppX 策略记录格式不受支持，停止自动覆盖策略".to_string(),
        ))
    }
}

fn write_journal(journal: &PolicyJournal) -> Result<(), AppError> {
    let path = journal_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::Engine(format!("创建 AppX 数据策略记录目录失败: {error}"))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| AppError::Engine(format!("序列化 AppX 数据策略记录失败: {error}")))?;
    atomic_file::write_atomic(&path, &bytes)
        .map_err(|error| AppError::Engine(format!("写入 AppX 数据策略记录失败: {error}")))
}

fn remove_journal() -> Result<(), AppError> {
    let path = journal_path()?;
    for candidate in [&path, &atomic_file::backup_path(&path)] {
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "删除 AppX 数据策略记录失败（{}）：{error}",
                    candidate.display()
                )))
            }
        }
        match fs::symlink_metadata(candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(AppError::Engine(format!(
                    "删除 AppX 数据策略记录后仍存在：{}",
                    candidate.display()
                )))
            }
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "无法核验 AppX 数据策略记录已删除（{}）：{error}",
                    candidate.display()
                )))
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn read_policy_value() -> Result<Option<u32>, AppError> {
    registry_policy().read()
}

#[cfg(not(target_os = "windows"))]
fn read_policy_value() -> Result<Option<u32>, AppError> {
    Ok(None)
}

#[cfg(target_os = "windows")]
fn powershell_exe() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

#[cfg(target_os = "windows")]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(target_os = "windows")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevatedResult {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

#[cfg(target_os = "windows")]
fn elevated_child_script(script: &str, result_path: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$resultPath = {result_path}
$utf8 = [System.Text.UTF8Encoding]::new($false)
try {{
  $operationData = & {{
{script}
  }}
  $payload = @{{ success = $true; data = $operationData }} | ConvertTo-Json -Depth 8 -Compress
  [System.IO.File]::WriteAllText($resultPath, $payload, $utf8)
  exit 0
}} catch {{
  $detail = (($_ | Out-String).Trim())
  if ([string]::IsNullOrWhiteSpace($detail)) {{ $detail = $_.Exception.Message }}
  # Use a plain hashtable rather than a typed object conversion.  Constrained
  # Language mode permits the former on more enterprise images, while still
  # keeping the result file a fixed, non-executable JSON diagnostic contract.
  $payload = @{{ success = $false; error = $detail }} | ConvertTo-Json -Compress
  try {{ [System.IO.File]::WriteAllText($resultPath, $payload, $utf8) }} catch {{}}
  exit 1
}}"#,
        result_path = ps_quote(&result_path.to_string_lossy()),
        script = script,
    )
}

#[cfg(target_os = "windows")]
fn elevated_token_guard_script() -> &'static str {
    r#"
# A normal CreateProcess child inherits its parent's token. Verify effective
# group membership nevertheless, without parsing localized whoami attributes.
# This verifies elevation, not permission to every protected registry key.
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
try {
  $principal = [System.Security.Principal.WindowsPrincipal]::new($identity)
  if (-not $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw '管理员子进程未获得完整管理员令牌；请完成 UAC 授权后重试'
  }
} finally {
  $identity.Dispose()
}
"#
}

#[cfg(all(test, target_os = "windows"))]
fn write_new_utf8_script(path: &Path, script: &str) -> Result<(), std::io::Error> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    // Windows PowerShell 5.1 otherwise interprets a UTF-8 script as the
    // system ANSI code page. The BOM preserves Chinese diagnostics and
    // non-ASCII user-profile paths when the elevated child reads the file.
    file.write_all(&[0xEF, 0xBB, 0xBF])?;
    file.write_all(script.as_bytes())?;
    file.flush()
}

#[cfg(target_os = "windows")]
fn cleanup_elevated_files(result_path: &Path) {
    let _ = fs::remove_file(result_path);
    let _ = fs::remove_file(result_path.with_extension("stderr"));
}

#[cfg(target_os = "windows")]
fn validate_inline_command_length(script: &str) -> Result<(), AppError> {
    // Windows' 32767 UTF-16 command-line limit includes executable, options,
    // and quoting expansion. Reserve >8K even in the worst case where quoting
    // doubles every script character; never fall back to a mutable .ps1.
    if script.encode_utf16().count() > 12_000 {
        return Err(AppError::Engine(
            "USB AppX 固定命令过长，停止执行（避免 Windows 命令行截断）".to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn terminate_elevated_wrapper(child: &mut Child) {
    let pid = child.id().to_string();
    // Cover helper descendants before killing the direct PowerShell child.
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Ok(mut killer) = Command::new("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(killer.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(ELEVATED_POLL_INTERVAL);
        }
        let _ = killer.kill();
        let _ = killer.wait();
    }
    // Kill the parent only after trying its tree; otherwise the child tree
    // may no longer be discoverable by taskkill.
    let _ = child.kill();
    let _ = child.wait();
}

/// Run a fixed, narrowly-scoped PowerShell operation in an elevated process.
///
/// The startup launcher already requested UAC. Reject an unelevated manager
/// instead of adding a second, hard-to-observe elevation boundary. No .ps1 is
/// written to user TEMP and no encoded/obfuscated command is used. The direct
/// child inherits the manager's token, with a bounded wait and no output pipes.
#[cfg(target_os = "windows")]
fn run_elevated_powershell_data(
    script: &str,
    operation: &str,
) -> Result<Option<serde_json::Value>, AppError> {
    run_elevated_powershell_data_with_timeout(script, operation, ELEVATED_OPERATION_TIMEOUT)
}

#[cfg(target_os = "windows")]
fn run_elevated_powershell_data_with_timeout(
    script: &str,
    operation: &str,
    timeout: Duration,
) -> Result<Option<serde_json::Value>, AppError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let process_full_admin =
        crate::app::win_elevation::current_process_is_elevated().map_err(AppError::Engine)?;
    log::info!("run elevated PowerShell operation={operation} current_process_full_admin={process_full_admin}");
    if !process_full_admin {
        return Err(AppError::Engine("USB AppX 操作需要完整管理员令牌；请退出后从 CodexDesktopSetup.exe 启动，并在 UAC 中选择“是”".to_string()));
    }
    let result_path = std::env::temp_dir().join(format!(
        "codex-usb-uac-{}-{}.json",
        std::process::id(),
        crate::app::usb_guard::now_ms()
    ));
    // Verify the child's effective membership before touching AppX metadata.
    let guarded_script = format!("{}\n{}", elevated_token_guard_script(), script);
    let child_script = elevated_child_script(&guarded_script, &result_path);
    validate_inline_command_length(&child_script)?;
    let child_path = powershell_exe();
    // Never wait on a process with undrained pipes: an AppX error with a long
    // stack trace can fill stderr and make an otherwise finished command hang.
    let stderr_path = result_path.with_extension("stderr");
    let stderr_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stderr_path)
        .map_err(|error| {
            cleanup_elevated_files(&result_path);
            AppError::Engine(format!("创建管理员操作诊断文件失败: {error}"))
        })?;
    let mut child = Command::new(&child_path)
        // Process-local compatibility switch only; never change HKLM/HKCU
        // execution policy or disable Defender/enterprise restrictions.
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &child_script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|error| {
            cleanup_elevated_files(&result_path);
            AppError::Engine(format!("启动管理员 {operation}失败: {error}"))
        })?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_elevated_wrapper(&mut child);
                cleanup_elevated_files(&result_path);
                return Err(AppError::Engine(format!(
                    "管理员 {operation}超时（{} 秒）。请检查 AppX 服务和 U 盘状态后重试",
                    timeout.as_secs()
                )));
            }
            Ok(None) => thread::sleep(ELEVATED_POLL_INTERVAL),
            Err(error) => {
                terminate_elevated_wrapper(&mut child);
                cleanup_elevated_files(&result_path);
                return Err(AppError::Engine(format!(
                    "等待管理员 {operation}失败: {error}"
                )));
            }
        }
    };
    let stderr = fs::read(&stderr_path).unwrap_or_default();
    let result = fs::read_to_string(&result_path)
        .ok()
        .and_then(|text| serde_json::from_str::<ElevatedResult>(text.trim()).ok());
    cleanup_elevated_files(&result_path);
    if status.success() && result.as_ref().is_some_and(|value| value.success) {
        return Ok(result.and_then(|value| value.data));
    }

    let result_detail = result
        .and_then(|value| value.error)
        .unwrap_or_default()
        .trim()
        .to_string();
    let outer_detail = String::from_utf8_lossy(&stderr).trim().to_string();
    let detail = if !result_detail.is_empty() {
        result_detail
    } else {
        outer_detail
    };
    let exit = status.code().unwrap_or(-1);
    if exit == 124 {
        return Err(AppError::Engine(format!(
            "管理员 {operation}超时（{} 秒）。请检查 AppX 服务和 U 盘状态后重试",
            timeout.as_secs()
        )));
    }
    if detail.is_empty() {
        Err(AppError::Engine(format!(
            "管理员 {operation}失败（exit {exit}）。请检查 AppX 服务、卷状态，或企业执行策略/WDAC/AppLocker 是否阻止了 PowerShell 子进程"
        )))
    } else {
        Err(AppError::Engine(format!(
            "管理员 {operation}失败（exit {exit}）：{detail}"
        )))
    }
}

#[cfg(target_os = "windows")]
fn set_policy_value(value: u32) -> Result<(), AppError> {
    set_policy_value_win32(value)
}

#[cfg(target_os = "windows")]
fn remove_policy_value() -> Result<(), AppError> {
    remove_policy_value_win32()
}

/// Access the 64-bit machine policy with precise native status codes and the
/// minimum rights needed. Access denied does NOT prove token loss: the key's
/// ACL, parent creation rights, registry view, or an enforced policy may deny
/// even an elevated process. Never change ACLs or take ownership to force it.
#[cfg(target_os = "windows")]
fn set_policy_value_win32(value: u32) -> Result<(), AppError> {
    registry_policy().set(value)
}

#[cfg(target_os = "windows")]
fn remove_policy_value_win32() -> Result<(), AppError> {
    registry_policy().remove()
}

#[cfg(target_os = "windows")]
struct RegistryDword<'a> {
    root: windows_sys::Win32::System::Registry::HKEY,
    subkey: &'a str,
    name: &'a str,
}

#[cfg(target_os = "windows")]
struct RegistryHandle(windows_sys::Win32::System::Registry::HKEY);

#[cfg(target_os = "windows")]
impl Drop for RegistryHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn registry_policy() -> RegistryDword<'static> {
    RegistryDword {
        root: windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        subkey: POLICY_SUBKEY,
        name: POLICY_VALUE,
    }
}

#[cfg(target_os = "windows")]
impl RegistryDword<'_> {
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn error(&self, operation: &str, status: u32) -> AppError {
        AppError::Engine(format!(
            "{operation} 注册表 {}\\{} 失败（Win32 {status}: {}）；管理员令牌不保证该键可写，请检查键 ACL/企业策略（不自动修改权限）",
            self.subkey, self.name, std::io::Error::from_raw_os_error(status as i32)
        ))
    }

    fn open(&self, access: u32, create: bool) -> Result<Option<RegistryHandle>, AppError> {
        use windows_sys::Win32::Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        };
        use windows_sys::Win32::System::Registry::{
            RegCreateKeyExW, RegOpenKeyExW, KEY_WOW64_64KEY, REG_OPTION_NON_VOLATILE,
        };
        let path = Self::wide(self.subkey);
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                self.root,
                path.as_ptr(),
                0,
                access | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        if status == ERROR_SUCCESS {
            return Ok(Some(RegistryHandle(handle)));
        }
        if status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND {
            return Err(self.error("RegOpenKeyExW", status));
        }
        if !create {
            return Ok(None);
        }
        // Opening an existing writable value must not require parent/subkey
        // creation rights. Create only after a precise 'not found' result.
        let status = unsafe {
            RegCreateKeyExW(
                self.root,
                path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                access | KEY_WOW64_64KEY,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(self.error("RegCreateKeyExW", status));
        }
        Ok(Some(RegistryHandle(handle)))
    }

    fn read(&self) -> Result<Option<u32>, AppError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{RegQueryValueExW, KEY_QUERY_VALUE, REG_DWORD};
        let Some(key) = self.open(KEY_QUERY_VALUE, false)? else {
            return Ok(None);
        };
        let name = Self::wide(self.name);
        let mut data = [0u8; 4];
        let mut size = data.len() as u32;
        let mut kind = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(self.error("RegQueryValueExW", status));
        }
        if kind != REG_DWORD || size != 4 {
            return Err(AppError::Engine(format!(
                "{}\\{} 不是有效 REG_DWORD（type={kind}, bytes={size}），保留原值",
                self.subkey, self.name
            )));
        }
        Ok(Some(u32::from_le_bytes(data)))
    }

    fn set(&self, value: u32) -> Result<(), AppError> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{RegSetValueExW, KEY_SET_VALUE, REG_DWORD};
        let key = self
            .open(KEY_SET_VALUE, true)?
            .expect("create returns a key or error");
        let name = Self::wide(self.name);
        let data = value.to_le_bytes();
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_DWORD,
                data.as_ptr(),
                data.len() as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(self.error("RegSetValueExW", status));
        }
        self.verify(Some(value))
    }

    fn remove(&self) -> Result<(), AppError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{RegDeleteValueW, KEY_SET_VALUE};
        if let Some(key) = self.open(KEY_SET_VALUE, false)? {
            let name = Self::wide(self.name);
            let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
                return Err(self.error("RegDeleteValueW", status));
            }
        }
        self.verify(None)
    }

    fn verify(&self, expected: Option<u32>) -> Result<(), AppError> {
        let actual = self.read()?;
        if actual != expected {
            return Err(AppError::Engine(format!("注册表策略回读不一致：expected={expected:?}, actual={actual:?}；可能被其他策略管理器修改")));
        }
        Ok(())
    }
}

fn apply_policy_change(previous_value: Option<u32>) -> Result<AppxDataPolicyReport, AppError> {
    if read_journal()?.is_some() {
        return Err(AppError::Engine(
            "AppX 策略与本产品记录不一致；可能被外部管理策略修改，保留现状并停止自动覆盖"
                .to_string(),
        ));
    }
    let journal = PolicyJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        product: "codex-usb-oneclick".to_string(),
        previous_value,
        applied_value: 1,
        applied_at_ms: crate::app::usb_guard::now_ms(),
    };
    // Write ownership/restoration evidence BEFORE touching machine state.
    // A crash or drive removal during volume registration can then be recovered.
    write_journal(&journal)?;
    if let Err(error) = set_policy_value(1) {
        if read_policy_value().ok() == Some(previous_value) {
            remove_journal()?;
        }
        return Err(error);
    }
    Ok(AppxDataPolicyReport {
        current_value: Some(1),
        changed: true,
        managed: true,
        message: "已将 AppX 用户数据限制在系统卷，并已回读确认".to_string(),
    })
}

#[cfg(not(target_os = "windows"))]
fn set_policy_value(_value: u32) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

#[cfg(not(target_os = "windows"))]
fn remove_policy_value() -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

/// Ensure AppX user data is kept on the system volume.  The first invocation
/// that changes the value records the previous value; subsequent launches are
/// idempotent and do not request another UAC prompt.
pub fn ensure_system_app_data_policy() -> Result<AppxDataPolicyReport, AppError> {
    let current = read_policy_value()?;
    if current == Some(1) {
        return Ok(AppxDataPolicyReport {
            current_value: current,
            changed: false,
            managed: read_journal()?.is_some(),
            message: "AppX 用户数据已限制在系统卷".to_string(),
        });
    }
    apply_policy_change(current)
}

#[derive(Debug, PartialEq, Eq)]
enum RestoreAction {
    Restore,
    AlreadyRestored,
    ExternalChange,
}

fn restore_action(current: Option<u32>, journal: &PolicyJournal) -> RestoreAction {
    if current == journal.previous_value {
        RestoreAction::AlreadyRestored
    } else if current == Some(journal.applied_value) {
        RestoreAction::Restore
    } else {
        RestoreAction::ExternalChange
    }
}

/// Restore the registry value recorded by this product. This is deliberately
/// idempotent and does nothing when no managed journal exists.
pub fn restore_system_app_data_policy() -> Result<Option<AppxDataPolicyReport>, AppError> {
    let Some(journal) = read_journal()? else {
        return Ok(None);
    };
    let before = read_policy_value()?;
    let action = restore_action(before, &journal);
    if action == RestoreAction::Restore {
        match journal.previous_value {
            Some(value) => set_policy_value(value)?,
            None => remove_policy_value()?,
        }
    }
    // Never delete restoration evidence until the actual value is known.
    // If another administrator/policy has changed it, relinquish ownership
    // without undoing their newer choice.
    let current = read_policy_value()?;
    if action != RestoreAction::ExternalChange && current != journal.previous_value {
        return Err(AppError::Engine(
            "恢复 AppX 策略后回读不一致，保留策略恢复记录".to_string(),
        ));
    }
    remove_journal()?;
    Ok(Some(AppxDataPolicyReport {
        current_value: current,
        changed: action == RestoreAction::Restore,
        managed: false,
        message: if action == RestoreAction::ExternalChange {
            "AppX 策略已被外部修改，保留其新值并移除本产品恢复记录".to_string()
        } else {
            "已确认恢复安装前的 AppX 用户数据策略".to_string()
        },
    }))
}

/// Read-only diagnostics used by tests and future settings UI.
pub fn system_app_data_policy_status() -> Result<AppxDataPolicyReport, AppError> {
    let current = read_policy_value()?;
    Ok(AppxDataPolicyReport {
        current_value: current,
        changed: false,
        managed: read_journal()?.is_some(),
        message: if current == Some(1) {
            "AppX 用户数据已限制在系统卷".to_string()
        } else {
            "AppX 用户数据仍遵循包卷默认位置".to_string()
        },
    })
}

/// Validate a volume root before it reaches the PowerShell installer. Only a
/// local drive root is accepted; UNC paths and a path below the root would
/// make the package volume ambiguous or permit accidental writes elsewhere.
pub fn validate_volume_root(path: &Path) -> Result<PathBuf, AppError> {
    let text = path.to_string_lossy().replace('/', "\\");
    let bytes = text.as_bytes();
    if bytes.len() != 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\'
    {
        return Err(AppError::Engine(format!(
            "AppX USB 卷必须是本地盘根路径（例如 F:\\），收到 {}",
            path.display()
        )));
    }
    Ok(PathBuf::from(format!(
        "{}:\\",
        (bytes[0] as char).to_ascii_uppercase()
    )))
}

/// Capture the mounted filesystem identity, not just a reusable drive letter.
/// The caller brackets this read with the USB lease/anchor validation; the
/// engine verifies the same GUID against AppX and storage before data access.
pub fn filesystem_volume_guid(volume_root: &Path) -> Result<String, AppError> {
    let root = validate_volume_root(volume_root)?;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut buffer = [0_u16; 64];
        // Both buffers are owned here and remain valid across this synchronous
        // read-only Windows call. The root is restricted to an absolute drive.
        if unsafe {
            GetVolumeNameForVolumeMountPointW(
                wide.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        } == 0
        {
            return Err(AppError::Engine(format!(
                "无法读取 U 盘卷 GUID（{}）：{}",
                root.display(),
                std::io::Error::last_os_error()
            )));
        }
        let length = buffer
            .iter()
            .position(|value| *value == 0)
            .ok_or_else(|| AppError::Engine("卷 GUID 输出未终止".into()))?;
        let value = String::from_utf16(&buffer[..length])
            .map_err(|_| AppError::Engine("卷 GUID 输出不是有效 Unicode".into()))?;
        validate_filesystem_volume_guid(&value)?;
        Ok(value)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = root;
        Err(AppError::UnsupportedPlatform)
    }
}

fn validate_filesystem_volume_guid(value: &str) -> Result<(), AppError> {
    let guid = value
        .strip_prefix(r"\\?\Volume{")
        .and_then(|value| value.strip_suffix("}\\"))
        .ok_or_else(|| AppError::Engine("无法确认稳定的文件系统卷 GUID".into()))?;
    if uuid::Uuid::parse_str(guid).is_err() {
        return Err(AppError::Engine("文件系统卷 GUID 格式无效".into()));
    }
    Ok(())
}

/// Register the exact removable volume as an AppX package volume under the
/// startup-approved administrator token.
///
/// `Add-AppxVolume` changes machine-level AppX volume metadata and Windows can
/// reject an unelevated call with `Access is denied`, even when the caller can
/// create files on the USB drive.  The recommended USB launcher has already
/// granted the manager a full administrator token, so this helper normally runs
/// directly without a second consent dialog; a legacy unelevated caller gets
/// a clear restart instruction. The child validates the drive root and NTFS filesystem
/// again, registers only that root's `WindowsApps` directory, and verifies an
/// online volume before returning. This keeps the privileged operation small
/// and avoids elevating an arbitrary package command.
#[cfg(all(test, target_os = "windows"))]
fn appx_volume_registration_script(root: &Path) -> String {
    appx_volume_registration_script_with_journal(root, None)
}

#[cfg(target_os = "windows")]
fn appx_volume_registration_script_with_journal(root: &Path, journal: Option<&Path>) -> String {
    let root = ps_quote(&root.to_string_lossy());
    // `root` is validated before this helper is called.  Keep the script's
    // target explicit nevertheless: a malformed value must fail closed in the
    // elevated process rather than turning into an arbitrary path write.
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$mountPoint = {mount_point}
$ownershipJournal = {ownership_journal}
$created = $false
$packageStorePath = Join-Path $mountPoint 'WindowsApps'
if ($mountPoint -notmatch '^[A-Za-z]:\\$') {{
  throw "USB AppX volume must be a local drive root: $mountPoint"
}}
if ($mountPoint.TrimEnd('\') -ieq $env:SystemDrive) {{
  throw 'Refusing to register or track the system volume as a USB volume'
}}
$driveLetter = $mountPoint.Substring(0, 1)
$getVolume = Get-Command Get-Volume -ErrorAction SilentlyContinue
if ($null -eq $getVolume) {{
  throw 'Get-Volume is unavailable; cannot validate the removable volume'
}}
$volumeProbe = @(Get-Volume -DriveLetter $driveLetter -ErrorAction Stop |
  Select-Object -First 1)[0]
if ($null -eq $volumeProbe) {{
  throw "USB AppX volume $mountPoint is not a local Windows volume"
}}
$fileSystem = [string]$volumeProbe.FileSystem
$volumeId = [string]$volumeProbe.UniqueId
if ([string]::IsNullOrWhiteSpace($volumeId)) {{
  throw 'Cannot establish stable USB volume identity; registration ownership would be unsafe'
}}
if ($fileSystem -notmatch '(?i)^NTFS$') {{
  throw "USB AppX volume $mountPoint uses filesystem '$fileSystem'; NTFS is required"
}}
$getAppxVolumeCmd = Get-Command Get-AppxVolume -ErrorAction Stop
function Find-OnlineAppxVolume {{
  # Never turn a failed inventory into 'absent' and claim shared ownership.
  $all = @(Get-AppxVolume -ErrorAction Stop)
  $matches = @($all | Where-Object {{
    -not [bool]$_.IsOffline -and
    ([string]$_.PackageStorePath).TrimEnd('\') -ieq $packageStorePath.TrimEnd('\')
  }})
  if ($matches.Count -gt 1) {{ throw 'Ambiguous AppX volume inventory' }}
  if ($matches.Count -eq 1) {{ return $matches[0] }}
  return $null
}}
$volume = @(& Find-OnlineAppxVolume)[0]
if ($null -eq $volume) {{
  # Existing WindowsApps directories are service-owned. Do not force-create
  # them before checking whether their volume is already registered.
  $offline = @(Get-AppxVolume -ErrorAction Stop | Where-Object {{
    [bool]$_.IsOffline -and
    (([string]$_.MountPoint).TrimEnd('\') -ieq $mountPoint.TrimEnd('\') -or
     ([string]$_.PackageStorePath).TrimEnd('\') -ieq $packageStorePath.TrimEnd('\'))
  }} | Select-Object -First 1)
  if ($offline.Count -gt 0) {{
    Mount-AppxVolume -Volume $offline[0] -ErrorAction Stop | Out-Null
    $volume = @(& Find-OnlineAppxVolume)[0]
  }}
}}
if ($null -eq $volume) {{
  if ($null -eq (Get-Command Add-AppxVolume -ErrorAction SilentlyContinue)) {{
    throw 'Add-AppxVolume is unavailable; USB AppX installation is unsupported'
  }}
  if (-not [string]::IsNullOrWhiteSpace($ownershipJournal)) {{
    # Write-ahead only: a crash after Add but before successful final evidence
    # leaves state=pending, which uninstall conservatively refuses to claim.
    $intent = @{{schemaVersion=1; product='codex-usb-oneclick'; state='pending';
      volumeRoot=$mountPoint; volumeId=$volumeId; volumeName=''; packageStorePath=$packageStorePath}}
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes(($intent | ConvertTo-Json -Compress))
    $stream = [System.IO.File]::Open($ownershipJournal, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
    try {{ $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) }} finally {{ $stream.Dispose() }}
  }}
  if (-not (Test-Path -LiteralPath $packageStorePath -PathType Container -ErrorAction Stop)) {{
    New-Item -ItemType Directory -Path $packageStorePath -ErrorAction Stop | Out-Null
  }}
  Add-AppxVolume -Path $packageStorePath -ErrorAction Stop | Out-Null
  $created = $true
  # AppX service publication is asynchronous on slower removable media.
  for ($attempt = 0; $attempt -lt 12 -and $null -eq $volume; $attempt++) {{
    Start-Sleep -Milliseconds 250
    $volume = @(& Find-OnlineAppxVolume)[0]
  }}
}}
if ($null -eq $volume) {{
  throw "Windows did not publish an online AppX volume for $mountPoint after registration"
}}
if ([bool]$volume.IsOffline) {{
  throw "Removable AppX volume $mountPoint is offline; reconnect the drive and retry"
}}
if ([bool]$volume.IsSystemVolume -or $mountPoint.TrimEnd('\') -ieq $env:SystemDrive) {{
  throw 'Refusing to track the system AppX volume as a USB volume'
}}
if ([string]::IsNullOrWhiteSpace([string]$volume.Name)) {{
  throw 'Windows returned an AppX volume without stable Name'
}}
@{{created=$created; volumeRoot=$mountPoint; volumeId=$volumeId;
  volumeName=[string]$volume.Name; packageStorePath=[string]$volume.PackageStorePath}}
    "#,
        mount_point = root,
        ownership_journal = journal
            .map(|path| ps_quote(&path.to_string_lossy()))
            .unwrap_or_else(|| "''".to_string()),
    )
}

/// Retry only the removable AppX-volume registration after the unelevated
/// installer receives the characteristic `Access is denied` response.
#[cfg(target_os = "windows")]
pub fn ensure_appx_volume_registered(volume_root: &Path) -> Result<(), AppError> {
    let root = validate_volume_root(volume_root)?;
    register_volume_with_ownership(&root).map_err(|error| {
        AppError::Engine(format!(
            "USB AppX 卷注册需要管理员权限（administrator approval is required）：{error}"
        ))
    })
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_appx_volume_registered(_volume_root: &Path) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

fn validate_external_deployment_policy(value: Option<u32>) -> Result<(), AppError> {
    match value {
        None | Some(0) => Ok(()),
        Some(1) => Err(AppError::Engine("系统策略 RestrictAppToSystemVolume 禁止非系统卷部署；无法安装到 U 盘，本程序不会覆盖企业/管理员策略".to_string())),
        Some(value) => Err(AppError::Engine(format!(
            "系统策略 RestrictAppToSystemVolume 值无效（{value}，预期为 0 或 1），不能判定是否允许 USB 部署；请管理员检查策略，本程序不会改写该值"
        ))),
    }
}

/// Ensure the USB edition's two machine-level prerequisites with one bounded
/// operation. The recommended
/// launcher has already obtained consent before the manager starts, so this is
/// an inherited full-administrator operation; unelevated callers are rejected.
/// If the policy is already
/// `1`, it is left untouched, but the USB AppX volume is still verified.
///
/// The policy write is performed in-process via Win32 registry APIs because
/// the manager already holds a full administrator token (the USB launcher is
/// requireAdministrator). Native access avoids PowerShell provider rights and
/// registry-view ambiguity, but does not bypass ACLs. Volume registration uses
/// the supported `Add-AppxVolume` cmdlet. If volume registration
/// fails, the policy write is rolled back so the machine is not left with a
/// stray machine-wide policy.
#[cfg(target_os = "windows")]
pub fn ensure_usb_appx_prerequisites(volume_root: &Path) -> Result<AppxDataPolicyReport, AppError> {
    let root = validate_volume_root(volume_root)?;
    let deployment_policy = RegistryDword {
        root: windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        subkey: POLICY_SUBKEY,
        name: "RestrictAppToSystemVolume",
    }
    .read()?;
    validate_external_deployment_policy(deployment_policy)?;
    log::info!(
        "USB deployment policy RestrictAppToSystemVolume={deployment_policy:?}; absent/0 is not a non-system-volume restriction; actual package placement still requires verification"
    );
    let report = ensure_system_app_data_policy()?;

    // Step 2: register the removable AppX volume via the existing elevated
    // PowerShell path. On failure, roll the policy back to its previous state
    // so a partially-applied prerequisite is not left behind.
    if let Err(error) = register_volume_with_ownership(&root) {
        if report.changed {
            if let Err(rollback_error) = restore_system_app_data_policy() {
                return Err(AppError::Engine(format!(
                    "USB AppX 卷注册失败（{error}），且策略回滚也失败（{rollback_error}）；已保留策略恢复记录"
                )));
            }
        }
        return Err(AppError::Engine(format!(
            "USB AppX 卷注册失败，本次策略更改已撤销（原有策略不变）：{error}"
        )));
    }
    if read_policy_value()? != Some(1) {
        return Err(AppError::Engine(
            "USB AppX 卷已注册，但系统卷数据策略被外部修改，停止安装".to_string(),
        ));
    }
    Ok(report)
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_usb_appx_prerequisites(
    _volume_root: &Path,
) -> Result<AppxDataPolicyReport, AppError> {
    Err(AppError::UnsupportedPlatform)
}

fn validate_volume_journal(journal: &VolumeJournal) -> Result<(), AppError> {
    let root = validate_volume_root(Path::new(&journal.volume_root))?;
    let expected_store = format!("{}WindowsApps", root.to_string_lossy());
    let valid_id = |value: &str| {
        let value = value.trim_end_matches('\\');
        value
            .strip_prefix(r"\\?\Volume{")
            .and_then(|tail| tail.strip_suffix('}'))
            .is_some_and(|guid| uuid::Uuid::parse_str(guid).is_ok())
    };
    if journal.schema_version != 1
        || journal.product != "codex-usb-oneclick"
        || !matches!(journal.state.as_str(), "pending" | "owned")
        || !valid_id(&journal.volume_id)
        || (journal.state == "owned" && !valid_id(&journal.volume_name))
        || !journal
            .package_store_path
            .eq_ignore_ascii_case(&expected_store)
    {
        return Err(AppError::Engine(
            "AppX 卷所有权记录不完整或格式不受支持；不注销任何卷，保留记录供诊断".to_string(),
        ));
    }
    Ok(())
}

fn remove_volume_journal(path: &Path) -> Result<(), AppError> {
    for candidate in [path.to_path_buf(), atomic_file::backup_path(path)] {
        match fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "删除已处理的 AppX 卷记录失败（{}）：{error}",
                    candidate.display()
                )));
            }
        }
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(AppError::Engine("AppX 卷记录删除后仍存在".into())),
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "核验 AppX 卷记录删除失败：{error}"
                )))
            }
        }
    }
    Ok(())
}

fn volume_journal_paths(dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(AppError::Engine(format!("枚举 AppX 卷记录失败：{error}"))),
    };
    let mut paths = std::collections::BTreeSet::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| AppError::Engine(format!("读取 AppX 卷记录目录失败：{error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(VOLUME_JOURNAL_PREFIX) {
            continue;
        }
        // Inspect both normal and backup records, deduplicating atomic pairs.
        let name = name.strip_suffix(".bak").unwrap_or(&name);
        if !name.ends_with(".json") {
            continue;
        }
        let id = name
            .trim_start_matches(VOLUME_JOURNAL_PREFIX)
            .trim_end_matches(".json");
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(AppError::Engine(
                "AppX 卷记录文件名无效，保留目录供诊断".into(),
            ));
        }
        paths.insert(dir.join(name));
    }
    Ok(paths.into_iter().collect())
}

fn load_volume_journal(path: &Path) -> Result<VolumeJournal, AppError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::read(atomic_file::backup_path(path))
                .map_err(|error| AppError::Engine(format!("读取 AppX 卷备份记录失败：{error}")))?
        }
        Err(error) => return Err(AppError::Engine(format!("读取 AppX 卷记录失败：{error}"))),
    };
    let journal = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Engine(format!("AppX 卷记录损坏，拒绝注销卷：{error}")))?;
    validate_volume_journal(&journal)?;
    Ok(journal)
}

#[cfg(target_os = "windows")]
fn register_volume_with_ownership(root: &Path) -> Result<(), AppError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RegistrationResult {
        created: bool,
        volume_root: String,
        volume_id: String,
        volume_name: String,
        package_store_path: String,
    }
    let dir = local_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|error| AppError::Engine(format!("创建 AppX 卷恢复记录目录失败：{error}")))?;
    let path = dir.join(format!(
        "{VOLUME_JOURNAL_PREFIX}{}.json",
        uuid::Uuid::new_v4()
    ));
    let script = appx_volume_registration_script_with_journal(root, Some(&path));
    let data = run_elevated_powershell_data(&script, "USB AppX 卷注册操作")?.ok_or_else(|| {
        AppError::Engine("AppX 卷操作没有返回身份数据，保留恢复记录并停止安装".into())
    })?;
    let result: RegistrationResult = serde_json::from_value(data)
        .map_err(|error| AppError::Engine(format!("AppX 卷身份结果无效：{error}")))?;
    if !result.created {
        // A volume present before Add is never adopted into our journal.
        return Ok(());
    }
    let intent = load_volume_journal(&path)?;
    let journal = VolumeJournal {
        schema_version: 1,
        product: "codex-usb-oneclick".into(),
        state: "owned".into(),
        volume_root: result.volume_root,
        volume_id: result.volume_id,
        volume_name: result.volume_name,
        package_store_path: result.package_store_path,
    };
    validate_volume_journal(&journal)?;
    if intent.state != "pending"
        || !intent.volume_id.eq_ignore_ascii_case(&journal.volume_id)
        || !intent
            .volume_root
            .eq_ignore_ascii_case(&journal.volume_root)
        || !journal
            .volume_root
            .eq_ignore_ascii_case(&root.to_string_lossy())
    {
        return Err(AppError::Engine(
            "AppX 卷创建结果与写前记录不符，保留记录并停止安装".into(),
        ));
    }
    let bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|error| AppError::Engine(format!("序列化 AppX 卷创建记录失败：{error}")))?;
    atomic_file::write_atomic(&path, &bytes)
        .map_err(|error| AppError::Engine(format!("保存 AppX 卷创建证据失败：{error}")))?;
    // Verify committed ownership before install proceeds.
    if load_volume_journal(&path)? != journal {
        return Err(AppError::Engine(
            "AppX 卷创建记录回读不一致，停止安装".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn appx_volume_cleanup_script_with_claim(
    journal: &VolumeJournal,
    authorized_package_full_name: Option<&str>,
) -> String {
    r#"
$ErrorActionPreference = 'Stop'
$expectedName = __NAME__
$expectedId = __ID__
$ownershipState = __STATE__
$authorizedFullName = __AUTHORIZED_FULL_NAME__
$officialFamily = 'OpenAI.Codex_2p2nqsd0c76g0'
function Get-CodexPackageLocation($package) {
  $script:packageAliasError = $null
  $location = $package.InstallLocation
  if ($null -ne $location -and $location.PSObject.Properties.Name -contains 'Path') { $location = $location.Path }
  $path = [string]$location
  if ($path.StartsWith('\\?\')) { $path = $path.Substring(4) }
  $fullName = [string]$package.PackageFullName
  $programFiles = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
  if ($fullName -eq $authorizedFullName -and $programFiles -and $expectedStore -and
      $path.TrimEnd('\') -ieq (Join-Path (Join-Path $programFiles 'WindowsApps') $fullName)) {
    try {
      $entry = Get-Item -LiteralPath $path -Force -ErrorAction Stop
      if ([string]$entry.LinkType -eq 'Junction' -and @($entry.Target).Count -eq 1) {
        $target = ([string]@($entry.Target)[0]).TrimEnd('\')
        if ($target.StartsWith('\\?\') -or $target.StartsWith('\??\')) { $target = $target.Substring(4) }
        if ($target -ieq (Join-Path $expectedStore $fullName)) {
          foreach ($probe in @($expectedStore, $target)) {
            $item = Get-Item -LiteralPath $probe -Force -ErrorAction Stop
            if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'AppX alias target is not a plain directory' }
          }
          return $target
        }
      }
    } catch { $script:packageAliasError=[string]$_.Exception.Message; return $null }
  }
  return $path.TrimEnd('\')
}
function Get-CodexPackageEvidence($package) {
  $owners = @()
  if ($package.PSObject.Properties.Name -contains 'PackageUserInformation') {
    foreach ($owner in @($package.PackageUserInformation)) {
      $owners += ([string]$owner.UserSecurityId + '=' + [string]$owner.InstallState)
    }
  } else { $owners += '<unavailable>' }
  $resolvedLocation = Get-CodexPackageLocation $package
  @{
    fullName=[string]$package.PackageFullName
    family=[string]$package.PackageFamilyName
    path=$resolvedLocation
    reportedPath=[string]$package.InstallLocation
    pathQueryError=$script:packageAliasError
    architecture=[string]$package.Architecture
    users=@($owners)
  }
}
function Format-CodexPackageEvidence($packages) {
  $limited = @($packages | Select-Object -First 16 | ForEach-Object { Get-CodexPackageEvidence $_ })
  $suffix = if (@($packages).Count -gt 16) { '; truncated=true' } else { '' }
  return (($limited | ConvertTo-Json -Depth 5 -Compress) + $suffix)
}
function Test-CodexStagedCleanupCandidate($package, [string]$expectedStore) {
  if ([string]::IsNullOrWhiteSpace($authorizedFullName)) { return @{allowed=$false; reason='current USB payload identity is unavailable'} }
  foreach ($property in @('PackageFullName','PackageFamilyName','InstallLocation','PackageUserInformation','IsFramework','IsResourcePackage','Architecture')) {
    if ($package.PSObject.Properties.Name -notcontains $property) { return @{allowed=$false; reason=('package property unavailable: ' + $property)} }
  }
  if ([string]$package.PackageFullName -ine $authorizedFullName) { return @{allowed=$false; reason='FullName is not the current verified USB payload'} }
  if ([string]$package.PackageFamilyName -ine $officialFamily) { return @{allowed=$false; reason='PackageFamilyName is not the pinned official Codex family'} }
  if ([bool]$package.IsFramework -or [bool]$package.IsResourcePackage -or [string]$package.Architecture -ine 'X64') {
    return @{allowed=$false; reason='package is not the expected main x64 Codex package'}
  }
  if ($package.PSObject.Properties.Name -contains 'NonRemovable' -and [bool]$package.NonRemovable) {
    return @{allowed=$false; reason='package is marked non-removable'}
  }
  $expectedLocation = (Join-Path $expectedStore $authorizedFullName).TrimEnd('\')
  if ((Get-CodexPackageLocation $package) -ine $expectedLocation) { return @{allowed=$false; reason='package location is outside the exact owned USB package store'} }
  foreach ($owner in @($package.PackageUserInformation)) {
    if ($owner.PSObject.Properties.Name -notcontains 'UserSecurityId' -or $owner.PSObject.Properties.Name -notcontains 'InstallState') {
      return @{allowed=$false; reason='package owner state is incomplete'}
    }
    if ([string]$owner.InstallState -ine 'Staged') {
      return @{allowed=$false; reason=('package has a non-staged user state: ' + [string]$owner.UserSecurityId + '=' + [string]$owner.InstallState)}
    }
  }
  $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop | Where-Object {
    [string]$_.PackageName -ieq $authorizedFullName
  })
  if ($provisioned.Count -ne 0) { return @{allowed=$false; reason='package has an online provisioned record'} }
  return @{allowed=$true; reason='exact staged-only current USB Codex payload on the owned volume'}
}
$all = @(Get-AppxVolume -ErrorAction Stop)
# Match physical/AppX identity, NEVER just a potentially reused drive letter.
$matches = @($all | Where-Object {
  ([string]$_.Name).TrimEnd('\') -ieq $expectedName.TrimEnd('\') -or
  ([string]$_.Name).TrimEnd('\') -ieq $expectedId.TrimEnd('\')
})
if ($matches.Count -eq 0) {
  @{status='absent'; message='AppX 卷注册已不存在；无需注销，已核验全部卷清单'}
  return
}
if ($matches.Count -ne 1) { throw 'Ambiguous AppX volume identity; no volume removed' }
$volume = $matches[0]
if ($ownershipState -ne 'owned') {
  @{status='pending'; message='AppX 卷创建在上次运行中断，无法证明独占创建；保留卷及写前记录，需人工核验'}
  return
}
if ($volume.PSObject.Properties.Name -notcontains 'IsSystemVolume' -or
    $volume.PSObject.Properties.Name -notcontains 'IsOffline') {
  throw 'AppX volume state is unknown; refusing removal'
}
if ([bool]$volume.IsSystemVolume) { throw 'Refusing to remove system AppX volume' }
if ([bool]$volume.IsOffline) {
  @{status='pending'; message='本产品创建的 AppX 卷离线；请重新连接原 U 盘后重试，未假定系统已注销'}
  return
}
$physical = @(Get-Volume -UniqueId $expectedId -ErrorAction Stop)
if ($physical.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$physical[0].DriveLetter)) {
  throw 'Cannot resolve the original physical volume; no volume removed'
}
if (-not ([string]$physical[0].UniqueId).TrimEnd('\').Equals($expectedId.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase)) {
  throw 'Physical volume identity changed; no volume removed'
}
$driveLetter = [string]$physical[0].DriveLetter
$currentRoot = $driveLetter + ':\'
if ($driveLetter -notmatch '^[A-Za-z]$' -or $currentRoot.TrimEnd('\') -ieq $env:SystemDrive) {
  throw 'Refusing to remove system or invalid volume'
}
$expectedStore = $currentRoot + 'WindowsApps'
if (-not ([string]$volume.PackageStorePath).TrimEnd('\').Equals($expectedStore, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw 'AppX package store does not match the original physical USB volume'
}
$disks = @(Get-Partition -DriveLetter $driveLetter -ErrorAction Stop | Get-Disk -ErrorAction Stop)
if ($disks.Count -ne 1 -or [string]$disks[0].BusType -ine 'USB') {
  throw 'Original volume is not confirmed on a USB disk; no volume removed'
}
# All users/types, including staged packages. AppX rechecks atomically.
$packages = @(Get-AppxPackage -AllUsers -Volume $volume -PackageTypeFilter Main,Framework,Resource,Bundle,Optional,Xap -ErrorAction Stop)
if ($packages.Count -gt 0) {
  $initialEvidence = Format-CodexPackageEvidence $packages
  $authorized = @($packages | Where-Object { [string]$_.PackageFullName -ieq $authorizedFullName })
  if ($authorized.Count -eq 1) {
    $eligibility = Test-CodexStagedCleanupCandidate $authorized[0] $expectedStore
    if ([bool]$eligibility.allowed) {
      # Re-query ownership immediately before removal.
      $live = @(Get-AppxPackage -AllUsers -Volume $volume -PackageTypeFilter Main,Framework,Resource,Bundle,Optional,Xap -ErrorAction Stop)
      $liveAuthorized = @($live | Where-Object { [string]$_.PackageFullName -ieq $authorizedFullName })
      if ($liveAuthorized.Count -ne 1) {
        @{status='pending'; message="AppX 卷包状态在清理前发生变化；未删除任何包。packages=$(Format-CodexPackageEvidence $live)"}
        return
      }
      $liveEligibility = Test-CodexStagedCleanupCandidate $liveAuthorized[0] $expectedStore
      if (-not [bool]$liveEligibility.allowed) {
        @{status='pending'; message="Codex staged 包在清理前不再满足无用户条件；未删除：$($liveEligibility.reason)。packages=$(Format-CodexPackageEvidence $live)"}
        return
      }
      try {
        Remove-AppxPackage -Package $authorizedFullName -AllUsers -ErrorAction Stop | Out-Null
      } catch {
        throw "精确 staged-only Codex 包清理失败；卷和恢复记录已保留；fullName=$authorizedFullName；$($_.Exception.Message)"
      }
      $packages = @(Get-AppxPackage -AllUsers -Volume $volume -PackageTypeFilter Main,Framework,Resource,Bundle,Optional,Xap -ErrorAction Stop)
      $remainingAuthorized = @($packages | Where-Object { [string]$_.PackageFullName -ieq $authorizedFullName })
      if ($remainingAuthorized.Count -ne 0) {
        throw "Remove-AppxPackage 返回后精确 Codex staged 包仍在；卷和恢复记录已保留；fullName=$authorizedFullName"
      }
      if ($packages.Count -gt 0) {
        @{status='pending'; message="已清理本产品精确 staged-only Codex 包 $authorizedFullName；AppX 卷仍含 $($packages.Count) 个其他包，保留共享卷。packages=$(Format-CodexPackageEvidence $packages)"}
        return
      }
    } else {
      @{status='pending'; message="AppX 卷仍含 $($packages.Count) 个包；精确 Codex 候选不满足 staged-only 安全清理条件：$($eligibility.reason)。未删除任何包；packages=$initialEvidence"}
      return
    }
  } else {
    $reason = if ([string]::IsNullOrWhiteSpace($authorizedFullName)) { '当前 U 盘载荷身份未通过校验' } elseif ($authorized.Count -gt 1) { '精确 Codex FullName 候选不唯一' } else { '卷内没有当前 U 盘载荷的精确 FullName' }
    @{status='pending'; message="AppX 卷仍含 $($packages.Count) 个所有用户/暂存包；$reason；保留共享卷且未删除任何包。packages=$initialEvidence"}
    return
  }
}
Remove-AppxVolume -Volume $volume -Confirm:$false -ErrorAction Stop | Out-Null
$remaining = @(Get-AppxVolume -ErrorAction Stop | Where-Object {
  ([string]$_.Name).TrimEnd('\') -ieq $expectedName.TrimEnd('\') -or
  ([string]$_.Name).TrimEnd('\') -ieq $expectedId.TrimEnd('\')
})
if ($remaining.Count -ne 0) { throw 'AppX volume registration remains after removal' }
# Do not recurse into WindowsApps or change its ACLs; Windows owns that store.
@{status='removed'; message='已注销本产品创建且所有用户/暂存包均为空的 USB AppX 卷；未强删 WindowsApps'}
"#
    // Only this fixed, single-line-string PowerShell literal is compacted:
    // no here-strings or user input. Preserve statement newlines; do not
    // increase the Windows command-line safety budget as this helper grows.
    .lines()
    .map(str::trim_start)
    .filter(|line| !line.is_empty() && !line.starts_with('#'))
    .collect::<Vec<_>>()
    .join("\n")
    .replace("__NAME__", &ps_quote(&journal.volume_name))
    .replace("__ID__", &ps_quote(&journal.volume_id))
    .replace("__STATE__", &ps_quote(&journal.state))
    .replace(
        "__AUTHORIZED_FULL_NAME__",
        &ps_quote(authorized_package_full_name.unwrap_or("")),
    )
}

#[cfg(target_os = "windows")]
fn system_package_entry_audit_script(full_name: Option<&str>) -> Result<String, AppError> {
    let full_name = full_name.filter(|value| !value.is_empty()).ok_or_else(|| {
        AppError::Engine(
            "未取得精确载荷身份，无法核验系统包目录残留；保留卷清理记录，请恢复完整安装介质后重试"
                .into(),
        )
    })?;
    Ok(format!(
        "{}\nGet-CodexSystemPackageEntryAudit {}",
        codex_win_engine::sys::APPX_PACKAGE_PATH_SCRIPT,
        ps_quote(full_name)
    ))
}

fn pending_system_package_entry(audit: &serde_json::Value) -> Result<Option<String>, AppError> {
    let status = audit.get("status").and_then(|v| v.as_str());
    let message = audit
        .get("message")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match (status, message) {
        (Some("absent"), Some(_)) => Ok(None),
        (Some("pending"), Some(detail)) => Ok(Some(detail.to_owned())),
        _ => Err(AppError::Engine(
            "系统包目录核验结果不完整；保留清理记录".into(),
        )),
    }
}

/// Must be called only after current-user Codex absence is verified. Each
/// restoration is independent so a bad policy does not hide a volume result.
/// This never deletes machine policy keys or guesses legacy previous values.
pub fn cleanup_system_after_uninstall(
    authorized_package_full_name: Option<&str>,
) -> AppxSystemCleanupReport {
    let mut report = AppxSystemCleanupReport::default();
    match restore_system_app_data_policy() {
        Ok(Some(result)) => report.record(POLICY_VALUE, "removed", result.message),
        Ok(None) => report.record(POLICY_VALUE, "preserved", "无本产品数据策略恢复记录；不修改任何现有系统策略。AllowDeploymentInExternalVolumes / AppModelUnlock 值不是本版本写入项，保持原状".into()),
        Err(error) => report.record(POLICY_VALUE, "failed", error.to_string()),
    }
    let paths = match local_dir().and_then(|dir| volume_journal_paths(&dir)) {
        Ok(paths) => paths,
        Err(error) => {
            report.record("AppX volume journals", "failed", error.to_string());
            return report;
        }
    };
    if paths.is_empty() {
        report.record("AppX volumes", "preserved", "无本产品创建卷记录；未注销任何既有/旧版本 AppX 卷（不能证明所有权），这不表示这些卷已清除".into());
    }
    for path in paths {
        let target = path.display().to_string();
        let result = (|| -> Result<(String, String), AppError> {
            let journal = load_volume_journal(&path)?;
            #[cfg(target_os = "windows")]
            let data = run_elevated_powershell_data_with_timeout(
                &appx_volume_cleanup_script_with_claim(&journal, authorized_package_full_name),
                "USB AppX 卷卸载清理",
                ELEVATED_USB_CLEANUP_TIMEOUT,
            )?
            .ok_or_else(|| AppError::Engine("AppX 卷清理未返回可验证结果".into()))?;
            #[cfg(not(target_os = "windows"))]
            let data: serde_json::Value = {
                let _ = journal;
                return Err(AppError::UnsupportedPlatform);
            };
            let status = data
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let message = data
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !matches!(status, "removed" | "absent" | "pending") || message.is_empty() {
                return Err(AppError::Engine(
                    "AppX 卷清理结果不完整；保留恢复记录".into(),
                ));
            }
            if matches!(status, "removed" | "absent") {
                // Windows owns its system-store junction. Even after native
                // removal succeeded, a leftover/unknown entry must keep the
                // retry journal and must not be reported as fully clean.
                #[cfg(target_os = "windows")]
                {
                    let audit = run_elevated_powershell_data(
                        &system_package_entry_audit_script(authorized_package_full_name)?,
                        "核验精确 Codex 系统包目录残留（只读）",
                    )?
                    .ok_or_else(|| {
                        AppError::Engine("系统包目录核验未返回结果；保留清理记录".into())
                    })?;
                    if let Some(detail) = pending_system_package_entry(&audit)? {
                        return Ok(("pending".into(), format!("{message}；{detail}")));
                    }
                }
                remove_volume_journal(&path)?;
            }
            Ok((status.to_owned(), message.to_owned()))
        })();
        match result {
            Ok((status, message)) => report.record(&target, &status, message),
            Err(error) => report.record(&target, "failed", error.to_string()),
        }
    }
    report
}

/// Filesystem cleanup must not erase the only rollback evidence after an
/// unsuccessful system operation. Read failures mean unknown, never absent.
pub fn has_pending_system_cleanup() -> Result<bool, AppError> {
    Ok(read_journal()?.is_some() || !volume_journal_paths(&local_dir()?)?.is_empty())
}

/// A payload signature scan is only needed when a product-owned AppX volume
/// journal exists. Policy-only cleanup must stay fast and must not depend on
/// the removable MSIX still being present.
pub fn has_pending_volume_cleanup() -> Result<bool, AppError> {
    Ok(!volume_journal_paths(&local_dir()?)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::validate_volume_root;
    use std::path::Path;

    #[test]
    fn filesystem_guid_requires_canonical_windows_volume_identity() {
        assert!(super::validate_filesystem_volume_guid(
            r"\\?\Volume{11111111-1111-4111-8111-111111111111}\"
        )
        .is_ok());
        for value in [
            r"D:\",
            r"\\server\share\",
            r"\\?\Volume{not-a-guid}\",
            r"\\?\Volume{11111111-1111-4111-8111-111111111111}\WpSystem",
        ] {
            assert!(
                super::validate_filesystem_volume_guid(value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn external_deployment_policy_does_not_treat_absence_as_a_block() {
        assert!(super::validate_external_deployment_policy(None).is_ok());
        assert!(super::validate_external_deployment_policy(Some(0)).is_ok());
    }

    #[test]
    fn external_deployment_policy_rejects_restrictions_and_invalid_values() {
        let blocked = super::validate_external_deployment_policy(Some(1))
            .unwrap_err()
            .to_string();
        assert!(blocked.contains("禁止非系统卷部署"));
        let invalid = super::validate_external_deployment_policy(Some(2))
            .unwrap_err()
            .to_string();
        assert!(invalid.contains("值无效"));
        assert!(super::validate_external_deployment_policy(Some(u32::MAX)).is_err());
    }

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("codex-appx-cleanup-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            // Only this test's freshly-created UUID fixture, never a profile,
            // workspace, or a real AppX/store directory.
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codex-appx-cleanup-test-"));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn volume_fixture(state: &str) -> super::VolumeJournal {
        super::VolumeJournal {
            schema_version: 1,
            product: "codex-usb-oneclick".into(),
            state: state.into(),
            volume_root: r"D:\".into(),
            volume_id: r"\\?\Volume{11111111-1111-4111-8111-111111111111}\".into(),
            volume_name: r"\\?\Volume{11111111-1111-4111-8111-111111111111}".into(),
            package_store_path: r"D:\WindowsApps".into(),
        }
    }

    #[test]
    fn volume_ownership_rejects_arbitrary_paths_and_unknown_states() {
        let journal = volume_fixture("owned");
        super::validate_volume_journal(&journal).unwrap();
        let mut pending = journal.clone();
        pending.state = "pending".into();
        pending.volume_name.clear();
        super::validate_volume_journal(&pending).unwrap();
        for (field, value) in [
            ("root", "D:\\other"),
            ("store", "D:\\elsewhere"),
            ("state", "assumed"),
            ("id", "D:"),
            ("name", ""),
        ] {
            let mut altered = journal.clone();
            match field {
                "root" => altered.volume_root = value.into(),
                "store" => altered.package_store_path = value.into(),
                "state" => altered.state = value.into(),
                "id" => altered.volume_id = value.into(),
                _ => altered.volume_name = value.into(),
            }
            assert!(super::validate_volume_journal(&altered).is_err(), "{field}");
        }
    }

    #[test]
    fn volume_journal_backup_and_removal_are_exact_and_idempotent() {
        let dir = TestDir::new();
        let path = dir
            .path()
            .join(format!("volume-{}.json", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&volume_fixture("owned")).unwrap();
        std::fs::write(super::atomic_file::backup_path(&path), &bytes).unwrap();
        let unrelated = dir.path().join("unrelated.json");
        std::fs::write(&unrelated, "keep").unwrap();
        assert_eq!(
            super::volume_journal_paths(dir.path()).unwrap(),
            vec![path.clone()]
        );
        assert_eq!(
            super::load_volume_journal(&path).unwrap(),
            volume_fixture("owned")
        );
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(super::volume_journal_paths(dir.path()).unwrap().len(), 1);
        super::remove_volume_journal(&path).unwrap();
        super::remove_volume_journal(&path).unwrap();
        assert!(super::volume_journal_paths(dir.path()).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "keep");
    }

    #[test]
    fn corrupt_or_unreadable_volume_evidence_is_not_missing() {
        let dir = TestDir::new();
        let path = dir
            .path()
            .join(format!("volume-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, "not json").unwrap();
        assert!(super::load_volume_journal(&path).is_err());
        assert!(!super::volume_journal_paths(dir.path()).unwrap().is_empty());
        assert!(super::volume_journal_paths(&path).is_err());
    }

    #[cfg(target_os = "windows")]
    fn cleanup_fixture(state: &str, setup: &str) -> std::process::Output {
        cleanup_fixture_with_claim(state, None, setup)
    }

    #[cfg(target_os = "windows")]
    fn cleanup_fixture_with_claim(
        state: &str,
        authorized_full_name: Option<&str>,
        setup: &str,
    ) -> std::process::Output {
        // Every privileged/system command is shadowed. No HKLM or AppX state
        // on this machine is consulted or modified by these tests.
        let mocks = r#"
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
$env:SystemDrive='C:'
$script:removeCount=0
$script:packageRemoveCount=0
$script:packageQueryCount=0
$script:packages=@()
$script:provisioned=@()
$script:queryFailure=$false
$script:provisionedFailure=$false
$script:mutateOnSecondQuery=$false
$script:keepRegistration=$false
$script:keepPackage=$false
$script:packageRemoveFailure=$false
$script:physical=[pscustomobject]@{DriveLetter='D'; UniqueId='\\?\Volume{11111111-1111-4111-8111-111111111111}\'}
$script:volume=[pscustomobject]@{Name='\\?\Volume{11111111-1111-4111-8111-111111111111}'; IsOffline=$false; IsSystemVolume=$false; PackageStorePath='D:\WindowsApps'}
function Get-AppxVolume { [CmdletBinding()]param() if ($null -ne $script:volume) { $script:volume } }
function Get-Volume { [CmdletBinding()]param($UniqueId) $script:physical }
function Get-Partition { [CmdletBinding()]param($DriveLetter) [pscustomobject]@{DiskNumber=9} }
function Get-Disk { [CmdletBinding()]param([Parameter(ValueFromPipeline=$true)]$InputObject) process { [pscustomobject]@{BusType='USB'} } }
function Get-AppxPackage { [CmdletBinding()]param([switch]$AllUsers,$Volume,$PackageTypeFilter)
  if (-not $AllUsers -or @($PackageTypeFilter).Count -lt 6) { throw 'Incomplete package query' }
  if ($script:queryFailure) { throw 'mock access denied' }
  $script:packageQueryCount++
  if ($script:mutateOnSecondQuery -and $script:packageQueryCount -eq 2 -and $script:packages.Count -gt 0) {
    $script:packages[0].PackageUserInformation[0].InstallState='Installed'
  }
  $script:packages
}
function Get-AppxProvisionedPackage { [CmdletBinding()]param([switch]$Online)
  if (-not $Online) { throw 'Incomplete provisioned package query' }
  if ($script:provisionedFailure) { throw 'mock provisioned query failure' }
  $script:provisioned
}
function Remove-AppxPackage { [CmdletBinding()]param($Package,[switch]$AllUsers)
  if (-not $AllUsers) { throw 'Remove-AppxPackage must cover all users' }
  if ($script:packageRemoveFailure) { throw 'mock package removal failure' }
  $script:packageRemoveCount++
  if (-not $script:keepPackage) {
    $script:packages=@($script:packages | Where-Object { [string]$_.PackageFullName -ine [string]$Package })
  }
}
function Remove-AppxVolume { [CmdletBinding(SupportsShouldProcess=$true)]param($Volume)
  $script:removeCount++
  if (-not $script:keepRegistration) { $script:volume=$null }
}
"#;
        let cleanup = super::appx_volume_cleanup_script_with_claim(
            &volume_fixture(state),
            authorized_full_name,
        );
        let script = format!("{mocks}\n{setup}\ntry {{ $result = & {{ {cleanup} }}; @{{result=$result; removed=$script:removeCount; packageRemoved=$script:packageRemoveCount}} | ConvertTo-Json -Depth 6 -Compress }} catch {{ Write-Output ('blocked:' + $_.Exception.Message + ';removed=' + $script:removeCount + ';packageRemoved=' + $script:packageRemoveCount); exit 2 }}");
        super::validate_inline_command_length(&super::elevated_child_script(
            &format!("{}\n{cleanup}", super::elevated_token_guard_script()),
            Path::new(r"C:\Users\fixture Chinese 空格\AppData\Local\Temp\codex-usb-uac-123456-1788356474731.json"),
        ))
        .unwrap();
        std::process::Command::new(super::powershell_exe())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap()
    }

    #[cfg(target_os = "windows")]
    fn staged_codex_package(full_name: &str, install_state: &str) -> String {
        format!(
            r#"$script:packages=@([pscustomobject]@{{PackageFullName='{full_name}'; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'; InstallLocation='D:\WindowsApps\{full_name}'; PackageUserInformation=@([pscustomobject]@{{UserSecurityId='S-1-5-18'; InstallState='{install_state}'}}); IsFramework=$false; IsResourcePackage=$false; Architecture='X64'; NonRemovable=$false}})"#
        )
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn volume_cleanup_only_removes_verified_owned_empty_usb_volume() {
        let output = cleanup_fixture("owned", "");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"]["status"], "removed");
        assert_eq!(value["removed"], 1);
        let moved = cleanup_fixture(
            "owned",
            "$script:physical.DriveLetter='F'; $script:volume.PackageStorePath='F:\\WindowsApps'",
        );
        assert!(
            moved.status.success(),
            "{}",
            String::from_utf8_lossy(&moved.stdout)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shared_offline_and_unproven_volume_cleanup_preserves_journal() {
        for (state, setup) in [
            ("pending", ""),
            ("owned", "$script:volume.IsOffline=$true"),
            ("owned", "$script:packages=@([pscustomobject]@{Name='OtherApp'; PackageUserInformation=@('Staged')})"),
        ] {
            let output = cleanup_fixture(state, setup);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["result"]["status"], "pending");
            assert_eq!(value["removed"], 0);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn exact_staged_only_current_payload_is_removed_before_owned_volume() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        let setup = staged_codex_package(FULL_NAME, "Staged");
        let output = cleanup_fixture_with_claim("owned", Some(FULL_NAME), &setup);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"]["status"], "removed");
        assert_eq!(value["packageRemoved"], 1);
        assert_eq!(value["removed"], 1);
    }

    #[test]
    fn only_confirmed_absent_system_entry_authorizes_cleanup_journal_removal() {
        use serde_json::json;
        assert!(
            super::pending_system_package_entry(&json!({"status":"absent","message":"gone"}))
                .unwrap()
                .is_none()
        );
        assert!(super::pending_system_package_entry(
            &json!({"status":"pending","message":"dangling junction remains"})
        )
        .unwrap()
        .is_some());
        for bad in [
            json!(null),
            json!({}),
            json!({"status":"absent"}),
            json!({"status":"absent","message":""}),
            json!({"status":"unknown","message":"denied"}),
        ] {
            assert!(super::pending_system_package_entry(&bad).is_err());
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn missing_payload_identity_cannot_skip_the_system_entry_postcondition() {
        assert!(super::system_package_entry_audit_script(None).is_err());
        assert!(super::system_package_entry_audit_script(Some("")).is_err());
        let script = super::system_package_entry_audit_script(Some(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
        ))
        .unwrap();
        super::validate_inline_command_length(&super::elevated_child_script(
            &format!("{}\n{script}", super::elevated_token_guard_script()),
            Path::new(r"C:\Users\fixture Chinese 空格\AppData\Local\Temp\codex-usb-uac-123456-1788356474731.json"),
        )).unwrap();
        assert!(script.contains(
            "Get-CodexSystemPackageEntryAudit 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'"
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn staged_cleanup_accepts_exact_system_alias_but_not_foreign_target() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        let mocks = r#"
$env:ProgramW6432='C:\Program Files'
$script:packages[0].InstallLocation='C:\Program Files\WindowsApps\'+$script:packages[0].PackageFullName
function Get-Item { [CmdletBinding()]param($LiteralPath,[switch]$Force)
  if ($LiteralPath -eq $script:packages[0].InstallLocation) {
    return [pscustomobject]@{PSIsContainer=$true; Attributes=[IO.FileAttributes]::ReparsePoint; LinkType='Junction'; Target=@($script:aliasTarget)}
  }
  return [pscustomobject]@{PSIsContainer=$true; Attributes=[IO.FileAttributes]::Directory}
}
"#;
        for (target, expected_status, removed) in [
            (format!(r"D:\WindowsApps\{FULL_NAME}"), "removed", 1),
            (r"D:\WindowsApps\Other.App".into(), "pending", 0),
        ] {
            let setup = format!(
                "{}\n{mocks}\n$script:aliasTarget='{}'",
                staged_codex_package(FULL_NAME, "Staged"),
                target
            );
            let output = cleanup_fixture_with_claim("owned", Some(FULL_NAME), &setup);
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(data["result"]["status"], expected_status);
            assert_eq!(data["packageRemoved"], removed);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn staged_cleanup_contract_runs_in_constrained_language() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        let output = cleanup_fixture_with_claim(
            "owned",
            Some(FULL_NAME),
            &format!(
                "{}; $ExecutionContext.SessionState.LanguageMode='ConstrainedLanguage'",
                staged_codex_package(FULL_NAME, "Staged")
            ),
        );
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"]["status"], "removed");
        assert_eq!(value["packageRemoved"], 1);
        assert_eq!(value["removed"], 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn staged_cleanup_requires_verified_claim_and_no_installed_or_provisioned_owner() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        for (claim, setup, expected_reason) in [
            (
                None,
                staged_codex_package(FULL_NAME, "Staged"),
                "身份未通过校验",
            ),
            (
                Some(FULL_NAME),
                staged_codex_package(FULL_NAME, "Installed"),
                "non-staged user state",
            ),
            (
                Some(FULL_NAME),
                format!(
                    "{}; $script:provisioned=@([pscustomobject]@{{PackageName='{}'}})",
                    staged_codex_package(FULL_NAME, "Staged"),
                    FULL_NAME
                ),
                "provisioned record",
            ),
        ] {
            let output = cleanup_fixture_with_claim("owned", claim, &setup);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["result"]["status"], "pending");
            assert_eq!(value["packageRemoved"], 0);
            assert_eq!(value["removed"], 0);
            assert!(
                value["result"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(expected_reason),
                "{}",
                value["result"]["message"]
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn staged_cleanup_rejects_wrong_location_or_incomplete_identity() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        for setup in [
            format!(
                "{}; $script:packages[0].InstallLocation='C:\\Program Files\\WindowsApps\\{}'",
                staged_codex_package(FULL_NAME, "Staged"),
                FULL_NAME
            ),
            format!(
                "{}; $script:packages[0].PackageFamilyName='Unrelated_123'",
                staged_codex_package(FULL_NAME, "Staged")
            ),
            format!(
                "{}; $script:packages[0].PSObject.Properties.Remove('PackageUserInformation')",
                staged_codex_package(FULL_NAME, "Staged")
            ),
        ] {
            let output = cleanup_fixture_with_claim("owned", Some(FULL_NAME), &setup);
            assert!(output.status.success());
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["result"]["status"], "pending");
            assert_eq!(value["packageRemoved"], 0);
            assert_eq!(value["removed"], 0);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn staged_cleanup_verifies_package_removal_and_preserves_shared_volume() {
        const FULL_NAME: &str = "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0";
        let retained = cleanup_fixture_with_claim(
            "owned",
            Some(FULL_NAME),
            &format!(
                "{}; $script:keepPackage=$true",
                staged_codex_package(FULL_NAME, "Staged")
            ),
        );
        assert!(!retained.status.success());
        let retained_text = String::from_utf8_lossy(&retained.stdout);
        assert!(retained_text.contains("精确 Codex staged 包仍在"));
        assert!(retained_text.contains("removed=0"));
        assert!(retained_text.contains("packageRemoved=1"));

        let shared = cleanup_fixture_with_claim(
            "owned",
            Some(FULL_NAME),
            &format!(
                "{}; $script:packages += [pscustomobject]@{{PackageFullName='Other.App_1.0.0.0_x64__other'; PackageFamilyName='Other.App_other'; InstallLocation='D:\\WindowsApps\\Other.App_1.0.0.0_x64__other'; PackageUserInformation=@([pscustomobject]@{{UserSecurityId='S-1-5-18'; InstallState='Staged'}}); IsFramework=$false; IsResourcePackage=$false; Architecture='X64'}}",
                staged_codex_package(FULL_NAME, "Staged")
            ),
        );
        assert!(shared.status.success());
        let value: serde_json::Value = serde_json::from_slice(&shared.stdout).unwrap();
        assert_eq!(value["result"]["status"], "pending");
        assert_eq!(value["packageRemoved"], 1);
        assert_eq!(value["removed"], 0);
        assert!(value["result"]["message"]
            .as_str()
            .unwrap()
            .contains("Other.App_1.0.0.0_x64__other"));

        let changed = cleanup_fixture_with_claim(
            "owned",
            Some(FULL_NAME),
            &format!(
                "{}; $script:mutateOnSecondQuery=$true",
                staged_codex_package(FULL_NAME, "Staged")
            ),
        );
        assert!(changed.status.success());
        let value: serde_json::Value = serde_json::from_slice(&changed.stdout).unwrap();
        assert_eq!(value["result"]["status"], "pending");
        assert_eq!(value["packageRemoved"], 0);
        assert_eq!(value["removed"], 0);
        assert!(value["result"]["message"]
            .as_str()
            .unwrap()
            .contains("清理前不再满足无用户条件"));

        for failure_flag in ["packageRemoveFailure", "provisionedFailure"] {
            let failed = cleanup_fixture_with_claim(
                "owned",
                Some(FULL_NAME),
                &format!(
                    "{}; $script:{}=$true",
                    staged_codex_package(FULL_NAME, "Staged"),
                    failure_flag
                ),
            );
            assert!(!failed.status.success(), "{failure_flag}");
            let output = String::from_utf8_lossy(&failed.stdout);
            assert!(output.contains("removed=0"), "{output}");
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn volume_cleanup_never_treats_query_error_or_reused_drive_as_empty() {
        for setup in [
            "$script:queryFailure=$true",
            "$script:volume.IsSystemVolume=$true",
            "$script:physical.DriveLetter='C'",
            "$script:physical.UniqueId='\\\\?\\Volume{22222222-2222-4222-8222-222222222222}\\'",
            "$script:volume.PackageStorePath='D:\\Unrelated'",
        ] {
            let output = cleanup_fixture("owned", setup);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("removed=0"));
        }
        let output = cleanup_fixture("owned", "$script:keepRegistration=$true");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("registration remains"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn absent_volume_is_idempotent_even_for_interrupted_creation() {
        for state in ["owned", "pending"] {
            let output = cleanup_fixture(state, "$script:volume=$null");
            assert!(output.status.success());
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["result"]["status"], "absent");
            assert_eq!(value["removed"], 0);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn new_volume_registration_records_intent_before_system_write() {
        let temp = TestDir::new();
        let path = temp
            .path()
            .join(format!("volume-{}.json", uuid::Uuid::new_v4()));
        let mocks = format!(
            r#"
$env:SystemDrive='C:'
$script:published=$false
$intentPath={}
function Get-Volume {{ [CmdletBinding()]param($DriveLetter) [pscustomobject]@{{FileSystem='NTFS'; UniqueId='\\?\Volume{{11111111-1111-4111-8111-111111111111}}\'}} }}
function Get-AppxVolume {{ [CmdletBinding()]param()
  if ($script:published) {{ [pscustomobject]@{{Name='\\?\Volume{{11111111-1111-4111-8111-111111111111}}'; IsOffline=$false; IsSystemVolume=$false; PackageStorePath='D:\WindowsApps'}} }}
}}
function Test-Path {{ [CmdletBinding()]param($LiteralPath,$PathType) $false }}
function New-Item {{ [CmdletBinding()]param($ItemType,$Path) }}
function Start-Sleep {{ param($Milliseconds) }}
function Add-AppxVolume {{ [CmdletBinding()]param($Path)
  $intent = [System.IO.File]::ReadAllText($intentPath) | ConvertFrom-Json
  if ($intent.state -ne 'pending') {{ throw 'No durable intent before registration' }}
  $script:published=$true
}}
function Mount-AppxVolume {{ throw 'unexpected mount' }}
"#,
            super::ps_quote(&path.to_string_lossy())
        );
        let registration =
            super::appx_volume_registration_script_with_journal(Path::new(r"D:\"), Some(&path));
        let script = format!("{mocks}\n& {{ {registration} }} | ConvertTo-Json -Compress");
        let output = std::process::Command::new(super::powershell_exe())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["created"], true);
        let journal = super::load_volume_journal(&path).unwrap();
        assert_eq!(journal.state, "pending");
        assert!(journal.volume_name.is_empty());
    }

    #[test]
    fn report_distinguishes_preserved_system_state_from_cleanup_failure() {
        let mut report = super::AppxSystemCleanupReport::default();
        report.record("shared", "preserved", "not ours".into());
        assert!(!report.pending);
        assert_eq!(report.items[0].status, "preserved");
        report.record("offline", "pending", "reconnect".into());
        assert!(report.pending);
        assert_eq!(report.warnings, vec!["reconnect"]);
    }

    #[test]
    fn usb_volume_cleanup_has_a_long_but_bounded_admin_budget() {
        assert!(super::ELEVATED_USB_CLEANUP_TIMEOUT > super::ELEVATED_OPERATION_TIMEOUT);
        assert!(super::ELEVATED_USB_CLEANUP_TIMEOUT <= std::time::Duration::from_secs(15 * 60));
    }

    #[test]
    fn accepts_only_drive_roots() {
        assert_eq!(
            validate_volume_root(Path::new("f:\\")).unwrap(),
            Path::new("F:\\")
        );
        assert!(validate_volume_root(Path::new(r"F:\usb")).is_err());
        assert!(validate_volume_root(Path::new(r"\\server\share")).is_err());
        assert!(validate_volume_root(Path::new("relative")).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn elevated_registration_script_is_root_scoped() {
        let script = super::appx_volume_registration_script(Path::new(r"D:\"));
        assert!(script.contains("$mountPoint = 'D:\\'"));
        assert!(script.contains("Get-Volume -DriveLetter"));
        assert!(script.contains("Add-AppxVolume -Path $packageStorePath"));
        assert!(script.contains("Get-AppxVolume"));
        assert!(!script.contains("Add-AppxPackage"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn inline_helper_is_bounded_and_works_with_restricted_execution_policy() {
        let result_path = std::env::temp_dir().join(format!(
            "codex-usb-inline-{}-{}.json",
            std::process::id(),
            crate::app::usb_guard::now_ms()
        ));
        let fixed_body = format!(
            "{}\n{}",
            super::elevated_token_guard_script(),
            super::appx_volume_registration_script(Path::new(r"D:\"))
        );
        let actual_command = super::elevated_child_script(&fixed_body, &result_path);
        super::validate_inline_command_length(&actual_command).unwrap();
        assert!(super::validate_inline_command_length(&"x".repeat(12001)).is_err());
        // Same direct invocation contract with a harmless body. Restricted
        // disables .ps1 files but must not reject an inline fixed command.
        let fixture = super::elevated_child_script("Write-Output 'fixture'", &result_path);
        let output = std::process::Command::new(super::powershell_exe())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Restricted",
                "-Command",
                &fixture,
            ])
            .output()
            .unwrap();
        let result = std::fs::read_to_string(&result_path).unwrap();
        let _ = std::fs::remove_file(&result_path);
        assert!(
            output.status.success() && result.contains("\"success\":true"),
            "{} result={result}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn elevated_operations_include_a_full_admin_token_guard() {
        let guard = super::elevated_token_guard_script();
        assert!(guard.contains("WindowsIdentity]::GetCurrent()"));
        assert!(guard.contains(".IsInRole("));
        assert!(!guard.contains("whoami.exe"));
        assert!(guard.contains("未获得完整管理员令牌"));
    }

    #[test]
    fn policy_restore_never_overwrites_an_external_change() {
        let mut journal = super::PolicyJournal {
            schema_version: super::JOURNAL_SCHEMA_VERSION,
            product: "codex-usb-oneclick".into(),
            previous_value: None,
            applied_value: 1,
            applied_at_ms: 1,
        };
        assert_eq!(
            super::restore_action(Some(1), &journal),
            super::RestoreAction::Restore
        );
        assert_eq!(
            super::restore_action(None, &journal),
            super::RestoreAction::AlreadyRestored
        );
        assert_eq!(
            super::restore_action(Some(0), &journal),
            super::RestoreAction::ExternalChange
        );
        journal.previous_value = Some(0);
        assert_eq!(
            super::restore_action(Some(0), &journal),
            super::RestoreAction::AlreadyRestored
        );
        assert_eq!(
            super::restore_action(None, &journal),
            super::RestoreAction::ExternalChange
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_registry_roundtrip_and_type_validation_use_hkcu_only() {
        use windows_sys::Win32::System::Registry::{
            RegDeleteKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WOW64_64KEY,
            REG_SZ,
        };
        let subkey = format!(
            "Software\\CodexUsbInstallerTests\\Policy-{}-{}",
            std::process::id(),
            crate::app::usb_guard::now_ms()
        );
        let target = super::RegistryDword {
            root: HKEY_CURRENT_USER,
            subkey: &subkey,
            name: "Policy",
        };
        let unrelated = super::RegistryDword {
            root: HKEY_CURRENT_USER,
            subkey: &subkey,
            name: "Unrelated",
        };
        assert_eq!(target.read().unwrap(), None);
        target.set(1).unwrap();
        unrelated.set(77).unwrap();
        assert_eq!(target.read().unwrap(), Some(1));
        target.set(0).unwrap();
        assert_eq!(target.read().unwrap(), Some(0));
        target.remove().unwrap();
        target.remove().unwrap();
        assert_eq!(target.read().unwrap(), None);
        assert_eq!(unrelated.read().unwrap(), Some(77));
        // A string value must not be parsed as a DWORD or treated as missing.
        let key = target.open(KEY_SET_VALUE, false).unwrap().unwrap();
        let value_name = super::RegistryDword::wide("Policy");
        let data = [b'1', 0, 0, 0];
        assert_eq!(
            unsafe { RegSetValueExW(key.0, value_name.as_ptr(), 0, REG_SZ, data.as_ptr(), 4) },
            0
        );
        assert!(target.read().is_err());
        drop(key);
        target.remove().unwrap();
        unrelated.remove().unwrap();
        let key_name = super::RegistryDword::wide(&subkey);
        assert_eq!(
            unsafe { RegDeleteKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), KEY_WOW64_64KEY, 0) },
            0
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registered_volume_does_not_write_service_owned_windowsapps() {
        // Shadow every platform command touched by this path. No real AppX
        // operations, volume changes, HKLM writes, or elevation take place.
        let mocks = r#"
function Get-Volume { param($DriveLetter) [pscustomobject]@{FileSystem='NTFS'; UniqueId='\\?\Volume{11111111-1111-4111-8111-111111111111}\'} }
function Get-AppxVolume { param($Path) [pscustomobject]@{Name='\\?\Volume{11111111-1111-4111-8111-111111111111}'; IsOffline=$false; IsSystemVolume=$false; MountPoint='D:\'; PackageStorePath='D:\WindowsApps'} }
function New-Item { throw 'must not touch existing WindowsApps' }
function Add-AppxVolume { throw 'must not register an existing volume' }
function Mount-AppxVolume { throw 'must not mount an online volume' }
"#;
        let script = format!(
            "{mocks}\n{}",
            super::appx_volume_registration_script(Path::new(r"D:\"))
        );
        let output = std::process::Command::new(super::powershell_exe())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn elevated_child_result_file_captures_success_and_failure() {
        for (label, body, expected_success) in [
            ("ok", "Write-Output 'fixture'", true),
            ("error", "throw 'fixture failure'", false),
        ] {
            let path = std::env::temp_dir().join(format!(
                "codex-usb-uac-child-{label}-{}-{}.json",
                std::process::id(),
                crate::app::usb_guard::now_ms()
            ));
            let script = super::elevated_child_script(body, &path);
            let script_path = std::env::temp_dir().join(format!(
                "codex-usb-uac-child-{label}-{}-{}.ps1",
                std::process::id(),
                crate::app::usb_guard::now_ms()
            ));
            super::write_new_utf8_script(&script_path, &script)
                .expect("write elevated child fixture");
            let output = std::process::Command::new(super::powershell_exe())
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    &script_path.to_string_lossy(),
                ])
                .output()
                .expect("run elevated child fixture");
            let result = std::fs::read_to_string(&path).expect("read elevated result fixture");
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&script_path);
            let success = result.contains("\"success\":true");
            assert_eq!(success, expected_success, "result={result}");
            if expected_success {
                assert!(output.status.success());
            } else {
                assert!(!output.status.success());
                assert!(result.contains("fixture failure"));
            }
        }
    }
}
