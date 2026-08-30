//! USB-bound launch guard for the offline Codex bundle.
//!
//! The manager stays a normal Tauri application.  A short-lived local copy of
//! the same executable is started with `--codex-usb-guard` and runs without a
//! window.  It owns a small lease file under `%LOCALAPPDATA%`; the lease
//! contains the removable-volume fingerprint and the exact PIDs started by
//! this product.  The helper polls once per second and, after a five-second
//! monotonic grace period, terminates only those PIDs when the authorized USB
//! volume can no longer be observed.
//!
//! This is a product access gate, not a DRM boundary.  An unsigned license is
//! auto-enrolled on first run so a developer can assemble a bundle without a
//! signing service.  Release builds may provide `CODEX_USB_GUARD_PUBLIC_KEY_B64`
//! at compile time and ship a signed `usb-license.json`; signatures are then
//! verified before a lease is created.  No API key or private signing key is
//! ever written to the USB or local guard directory.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::atomic_file;
use crate::errors::AppError;

pub const GUARD_ARG: &str = "--codex-usb-guard";
pub const GUARD_EXE_NAME: &str = "CodexUsbGuard.exe";
pub const GUARD_DIR_NAME: &str = "CodexUsbGuard";
pub const LICENSE_FILE_NAME: &str = "usb-license.json";
pub const LEASE_FILE_NAME: &str = "lease.json";
pub const STATUS_FILE_NAME: &str = "status.json";
pub const LOCK_FILE_NAME: &str = "guard.lock";
pub const PRODUCT_ID: &str = "codex-usb-oneclick";
pub const POLL_INTERVAL_MS: u64 = 1_000;
pub const DISCONNECT_GRACE_MS: u64 = 5_000;
const LICENSE_SCHEMA_VERSION: u32 = 1;
const LEASE_SCHEMA_VERSION: u32 = 1;
const STATUS_SCHEMA_VERSION: u32 = 1;

/// Public fingerprint fields are deliberately small and stable.  A drive
/// letter is not part of the identity, so a Windows letter reassignment does
/// not break an authorized session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbFingerprint {
    pub volume_serial: u32,
    pub anchor_sha256: String,
    pub bundle_relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbLicense {
    pub schema_version: u32,
    pub product: String,
    pub installation_id: String,
    pub fingerprint: UsbFingerprint,
    pub issued_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProcess {
    pub pid: u32,
    pub role: String,
    pub executable: Option<String>,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbLease {
    pub schema_version: u32,
    pub product: String,
    pub lease_id: String,
    pub bundle_path: String,
    pub fingerprint: UsbFingerprint,
    pub owner_pid: u32,
    pub managed_processes: Vec<ManagedProcess>,
    pub issued_at: String,
}

/// A renderer-friendly snapshot.  `status` is one of authorized, grace,
/// missing, mismatch, unsupported, disabled, or stopped.  `watchdog` is
/// running/not-running so a UI can distinguish a healthy gate from a stale
/// status file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbGuardStatus {
    pub schema_version: u32,
    pub status: String,
    pub watchdog: String,
    pub removable: bool,
    pub drive_root: Option<String>,
    pub bundle_path: Option<String>,
    pub bundle_relative_path: Option<String>,
    pub signed: bool,
    pub last_checked_at_ms: u64,
    pub missing_since_ms: Option<u64>,
    pub countdown_seconds: Option<u64>,
    pub managed_processes: Vec<ManagedProcess>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectDecision {
    pub missing_since_ms: Option<u64>,
    pub expired: bool,
    pub countdown_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct UsbObservation {
    pub removable: bool,
    pub drive_root: Option<String>,
    pub bundle_path: String,
    pub fingerprint: Option<UsbFingerprint>,
}

fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(format!("USB 看门狗：{}", message.into()))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Normalize a Windows volume root to `X:\\`.  Returning `None` for UNC or
/// relative paths prevents a copied bundle from accidentally being treated as
/// a removable volume.
pub fn normalize_drive_root(input: &str) -> Option<String> {
    let raw = input.trim().replace('/', "\\");
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    Some(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()))
}

/// Return a lower-case, slash-separated path below a volume root.  It is
/// intentionally independent of the host OS so the identity remains stable
/// in tests running on Linux/macOS.
pub fn bundle_relative_path(bundle_path: &str, drive_root: &str) -> Option<String> {
    let path = bundle_path.trim().replace('/', "\\");
    let root = normalize_drive_root(drive_root)?;
    let path_lower = path.to_ascii_lowercase();
    let root_lower = root.to_ascii_lowercase();
    if !path_lower.starts_with(&root_lower) {
        return None;
    }
    let relative_text = path[root.len()..].replace('\\', "/");
    let relative = relative_text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if relative.iter().any(|part| *part == "..") {
        return None;
    }
    Some(relative.join("/").to_ascii_lowercase())
}

pub fn fingerprints_match(expected: &UsbFingerprint, actual: &UsbFingerprint) -> bool {
    expected.volume_serial == actual.volume_serial
        && expected
            .anchor_sha256
            .eq_ignore_ascii_case(&actual.anchor_sha256)
        && expected
            .bundle_relative_path
            .eq_ignore_ascii_case(&actual.bundle_relative_path)
}

/// Pure disconnect state transition used by the helper and unit-tested without
/// Windows device APIs.  `grace_ms` is injected to make boundary tests fast.
pub fn next_disconnect_state(
    missing_since_ms: Option<u64>,
    present: bool,
    now: u64,
    grace_ms: u64,
) -> DisconnectDecision {
    if present {
        return DisconnectDecision {
            missing_since_ms: None,
            expired: false,
            countdown_seconds: None,
        };
    }
    let since = missing_since_ms.unwrap_or(now);
    let deadline = since.saturating_add(grace_ms);
    if now >= deadline {
        return DisconnectDecision {
            missing_since_ms: Some(since),
            expired: true,
            countdown_seconds: Some(0),
        };
    }
    let remaining = deadline - now;
    DisconnectDecision {
        missing_since_ms: Some(since),
        expired: false,
        countdown_seconds: Some((remaining.saturating_add(999)) / 1_000),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedLicense<'a> {
    schema_version: u32,
    product: &'a str,
    installation_id: &'a str,
    fingerprint: &'a UsbFingerprint,
    issued_at: &'a str,
    key_id: &'a Option<String>,
}

fn unsigned_license_payload(license: &UsbLicense) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec(&UnsignedLicense {
        schema_version: license.schema_version,
        product: &license.product,
        installation_id: &license.installation_id,
        fingerprint: &license.fingerprint,
        issued_at: &license.issued_at,
        key_id: &license.key_id,
    })
    .map_err(|error| engine(format!("序列化许可证失败: {error}")))
}

/// Verify a license with an explicit public key.  Keeping this function
/// public allows the release packer and deterministic unit tests to share the
/// exact canonical payload format.
pub fn verify_signature_with_key(
    license: &UsbLicense,
    public_key_b64: &str,
) -> Result<(), AppError> {
    let signature = license
        .signature
        .as_deref()
        .ok_or_else(|| engine("许可证没有签名"))?;
    let key_bytes = B64
        .decode(public_key_b64.trim())
        .map_err(|error| engine(format!("许可证公钥不是有效 Base64: {error}")))?;
    let key_array: [u8; 32] = key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| engine("许可证公钥必须是 32 字节"))?;
    let key = VerifyingKey::from_bytes(&key_array)
        .map_err(|error| engine(format!("许可证公钥无效: {error}")))?;
    let sig_bytes = B64
        .decode(signature.trim())
        .map_err(|error| engine(format!("许可证签名不是有效 Base64: {error}")))?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| engine("许可证签名必须是 64 字节"))?;
    let sig = Signature::from_bytes(&sig_array);
    let payload = unsigned_license_payload(license)?;
    key.verify(&payload, &sig)
        .map_err(|_| engine("许可证签名校验失败"))
}

fn verify_configured_signature(license: &UsbLicense) -> Result<bool, AppError> {
    let Some(signature) = license.signature.as_ref() else {
        return Ok(false);
    };
    let Some(public_key) = option_env!("CODEX_USB_GUARD_PUBLIC_KEY_B64") else {
        return Err(engine(
            "许可证带签名，但当前构建没有配置 CODEX_USB_GUARD_PUBLIC_KEY_B64",
        ));
    };
    if public_key.trim().is_empty() {
        return Err(engine("许可证带签名，但构建公钥为空"));
    }
    let _ = signature;
    verify_signature_with_key(license, public_key)?;
    Ok(true)
}

pub fn validate_license(license: &UsbLicense, expected: &UsbFingerprint) -> Result<bool, AppError> {
    if license.schema_version != LICENSE_SCHEMA_VERSION {
        return Err(engine(format!(
            "许可证版本不兼容: {}",
            license.schema_version
        )));
    }
    if license.product != PRODUCT_ID {
        return Err(engine("许可证产品标识不匹配"));
    }
    if !fingerprints_match(&license.fingerprint, expected) {
        return Err(engine("许可证与当前 U 盘指纹不匹配"));
    }
    verify_configured_signature(license)
}

pub fn new_unsigned_license(fingerprint: UsbFingerprint) -> UsbLicense {
    UsbLicense {
        schema_version: LICENSE_SCHEMA_VERSION,
        product: PRODUCT_ID.to_string(),
        installation_id: Uuid::new_v4().to_string(),
        fingerprint,
        issued_at: now_ms().to_string(),
        key_id: None,
        signature: None,
    }
}

fn anchor_sha256(bundle_root: &Path) -> Result<String, AppError> {
    let mut hasher = Sha256::new();
    // These files are part of the tracked USB layout.  Hashing both makes a
    // copied/edited manifest fail even if the large MSIX payload is untouched.
    for name in ["manifest.json", "usb-manifest.json"] {
        let path = bundle_root.join(name);
        let bytes = fs::read(&path)
            .map_err(|error| engine(format!("读取锚点 {} 失败: {error}", path.display())))?;
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn bundle_root_for_executable(executable: &Path) -> Option<PathBuf> {
    let parent = executable.parent()?.to_path_buf();
    if parent.join("manifest.json").is_file() && parent.join("payload").is_dir() {
        return Some(parent);
    }
    if parent.file_name().and_then(|name| name.to_str()) == Some("resources") {
        let root = parent.parent()?.to_path_buf();
        if root.join("manifest.json").is_file() {
            return Some(root);
        }
    }
    None
}

fn license_path(bundle_root: &Path) -> PathBuf {
    bundle_root.join(LICENSE_FILE_NAME)
}

fn read_license(bundle_root: &Path) -> Result<Option<UsbLicense>, AppError> {
    let path = license_path(bundle_root);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| engine(format!("读取许可证失败: {error}")))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| engine(format!("许可证 JSON 无效: {error}")))
}

fn write_license(bundle_root: &Path, license: &UsbLicense) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(license)
        .map_err(|error| engine(format!("序列化许可证失败: {error}")))?;
    atomic_file::write_atomic(&license_path(bundle_root), &bytes)
        .map_err(|error| engine(format!("写入许可证失败: {error}")))
}

fn local_guard_dir() -> Result<PathBuf, AppError> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(root).join(GUARD_DIR_NAME));
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join(GUARD_DIR_NAME))
        .ok_or_else(|| engine("无法确定本地看门狗目录"))
}

fn guard_path(name: &str) -> Result<PathBuf, AppError> {
    Ok(local_guard_dir()?.join(name))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| engine(format!("写入 {} 序列化失败: {error}", path.display())))?;
    atomic_file::write_atomic(path, &bytes)
        .map_err(|error| engine(format!("写入 {} 失败: {error}", path.display())))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    atomic_file::read_with_recovery(path).0
}

fn status_message(status: &str) -> String {
    match status {
        "authorized" => "U 盘已验证，看门狗运行中".to_string(),
        "grace" => "暂时未检测到 U 盘，正在等待重新连接".to_string(),
        "missing" => "未检测到授权 U 盘，已停止受保护进程".to_string(),
        "mismatch" => "检测到 U 盘，但设备或目录指纹不匹配".to_string(),
        "unsupported" => "当前启动位置不是可移动 U 盘".to_string(),
        "stopped" => "U 盘看门狗已停止".to_string(),
        _ => "U 盘看门狗不可用".to_string(),
    }
}

fn base_status(status: &str, watchdog: &str) -> UsbGuardStatus {
    UsbGuardStatus {
        schema_version: STATUS_SCHEMA_VERSION,
        status: status.to_string(),
        watchdog: watchdog.to_string(),
        removable: false,
        drive_root: None,
        bundle_path: None,
        bundle_relative_path: None,
        signed: false,
        last_checked_at_ms: now_ms(),
        missing_since_ms: None,
        countdown_seconds: None,
        managed_processes: Vec::new(),
        message: status_message(status),
    }
}

pub fn disabled_status() -> UsbGuardStatus {
    let mut status = base_status("disabled", "not-running");
    status.message = "当前平台不是 Windows，U 盘绑定未启用".to_string();
    status
}

fn status_path() -> Result<PathBuf, AppError> {
    guard_path(STATUS_FILE_NAME)
}

fn lease_path() -> Result<PathBuf, AppError> {
    guard_path(LEASE_FILE_NAME)
}

fn lock_path() -> Result<PathBuf, AppError> {
    guard_path(LOCK_FILE_NAME)
}

pub fn current_status() -> UsbGuardStatus {
    status_path()
        .ok()
        .and_then(|path| read_json::<UsbGuardStatus>(&path))
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                base_status("stopped", "not-running")
            } else {
                disabled_status()
            }
        })
}

/// Only the fixed USB layout is mandatory-bound.  A normal installed manager
/// (which has no sibling `manifest.json` + `payload` directory) keeps the
/// existing online/historical Windows behavior.
pub fn guard_required() -> bool {
    #[cfg(target_os = "windows")]
    {
        return std::env::current_exe()
            .ok()
            .and_then(|path| bundle_root_for_executable(&path))
            .is_some();
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn require_authorized() -> Result<(), AppError> {
    if !guard_required() {
        return Ok(());
    }
    let status = current_status();
    if status.status == "authorized" && status.watchdog == "running" {
        return Ok(());
    }
    Err(engine(format!(
        "未检测到有效 U 盘租约（status={} watchdog={}），拒绝执行受保护操作",
        status.status, status.watchdog
    )))
}

pub fn write_status(status: &UsbGuardStatus) -> Result<(), AppError> {
    write_json_atomic(&status_path()?, status)
}

fn write_lease(lease: &UsbLease) -> Result<(), AppError> {
    write_json_atomic(&lease_path()?, lease)
}

fn read_lease() -> Option<UsbLease> {
    lease_path().ok().and_then(|path| read_json(&path))
}

fn remove_lease() {
    if let Ok(path) = lease_path() {
        let backup = atomic_file::backup_path(&path);
        let _ = fs::remove_file(&path);
        // atomic_file keeps a recovery backup.  Leaving it behind would make
        // the headless helper resurrect a supposedly stopped lease.
        let _ = fs::remove_file(backup);
    }
}

pub fn register_process(
    pid: u32,
    role: impl Into<String>,
    executable: Option<&Path>,
) -> Result<(), AppError> {
    let role = role.into();
    let Some(mut lease) = read_lease() else {
        return Ok(());
    };
    if lease.owner_pid != std::process::id() && lease.owner_pid != pid {
        // A stale manager must not append to a newer session's lease.
        return Ok(());
    }
    if let Some(existing) = lease
        .managed_processes
        .iter_mut()
        .find(|item| item.pid == pid)
    {
        existing.role = role.clone();
        existing.executable = executable.map(|path| path.to_string_lossy().into_owned());
        return write_lease(&lease);
    }
    lease.managed_processes.push(ManagedProcess {
        pid,
        role,
        executable: executable.map(|path| path.to_string_lossy().into_owned()),
        started_at_ms: now_ms(),
    });
    write_lease(&lease)
}

fn executable_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
}

#[cfg(target_os = "windows")]
fn tasklist_pids(image_name: &str) -> Vec<u32> {
    let output = Command::new("tasklist")
        .args([
            "/FI",
            &format!("IMAGENAME eq {image_name}"),
            "/FO",
            "CSV",
            "/NH",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let fields = line
                .split("\",\"")
                .map(|part| part.trim_matches('"'))
                .collect::<Vec<_>>();
            fields.get(1)?.replace(',', "").parse::<u32>().ok()
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn tasklist_pids(_image_name: &str) -> Vec<u32> {
    Vec::new()
}

pub fn snapshot_processes(image_name: &str) -> Vec<u32> {
    tasklist_pids(image_name)
}

/// Register processes that appeared after a manager-owned launch.  The
/// before/after PID comparison avoids claiming a user's pre-existing Codex
/// process, while the stored image marker gives taskkill a second scope check.
pub fn register_new_processes(before: &[u32], image_name: &str, role: &str) {
    let executable = Path::new(image_name);
    for pid in snapshot_processes(image_name) {
        if before.contains(&pid) {
            continue;
        }
        if let Err(error) = register_process(pid, role.to_string(), Some(executable)) {
            log::warn!("USB guard could not register pid={pid} role={role}: {error}");
        }
    }
}

pub fn register_new_codex_processes(before_codex: &[u32], before_chatgpt: &[u32]) {
    register_new_processes(before_codex, "Codex.exe", "codex");
    register_new_processes(before_chatgpt, "ChatGPT.exe", "codex");
}

/// Return the PIDs registered by the current lease for a role.  Consumers use
/// this instead of discovering every process with the same image name.
pub fn managed_processes_for_role(role: &str) -> Vec<ManagedProcess> {
    read_lease()
        .map(|lease| {
            lease
                .managed_processes
                .into_iter()
                .filter(|process| process.role == role)
                .collect()
        })
        .unwrap_or_default()
}

/// Terminate only the processes for one lease role.  This is shared by the
/// watchdog expiry path and the CCSwitch uninstall path so both flows apply
/// the same PID plus executable-name recheck before calling taskkill.
pub fn terminate_managed_processes_for_role(role: &str) -> usize {
    let Some(lease) = read_lease() else {
        return 0;
    };
    let mut scoped = lease;
    scoped
        .managed_processes
        .retain(|process| process.role == role);
    terminate_managed_processes(&scoped)
}

#[cfg(target_os = "windows")]
fn process_matches(pid: u32, expected_image: Option<&str>) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else { return false };
    let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    if !text.contains(&format!("\"{pid}\"")) {
        return false;
    }
    expected_image
        .map(|name| text.contains(&format!("\"{}\"", name.to_ascii_lowercase())))
        .unwrap_or(true)
}

#[cfg(not(target_os = "windows"))]
fn process_matches(_pid: u32, _expected_image: Option<&str>) -> bool {
    false
}

#[cfg(target_os = "windows")]
fn terminate_managed_processes(lease: &UsbLease) -> usize {
    let mut terminated = 0;
    let self_pid = std::process::id();
    for managed in &lease.managed_processes {
        if managed.pid == self_pid {
            continue;
        }
        let expected_image = managed
            .executable
            .as_deref()
            .and_then(executable_name)
            .map(|value| value.to_ascii_lowercase());
        if !process_matches(managed.pid, expected_image.as_deref()) {
            continue;
        }
        // Give the application a short graceful close window first.  The
        // forceful retry is still PID-scoped and only happens if that exact
        // process (and image marker) remains alive.
        let pid_text = managed.pid.to_string();
        let graceful = Command::new("taskkill")
            .args(["/PID", &pid_text, "/T"])
            .output();
        if graceful.as_ref().is_err() || !graceful.as_ref().is_ok_and(|out| out.status.success()) {
            log::debug!(
                "USB guard graceful close did not complete pid={} role={}",
                managed.pid,
                managed.role
            );
        }
        std::thread::sleep(Duration::from_millis(250));
        if !process_matches(managed.pid, expected_image.as_deref()) {
            terminated += 1;
            continue;
        }
        match Command::new("taskkill")
            .args(["/PID", &pid_text, "/T", "/F"])
            .output()
        {
            Ok(output) if output.status.success() => terminated += 1,
            Ok(output) => log::warn!(
                "USB guard could not terminate pid={} role={} error={}",
                managed.pid,
                managed.role,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => log::warn!(
                "USB guard taskkill failed pid={} role={} error={error}",
                managed.pid,
                managed.role
            ),
        }
    }
    terminated
}

#[cfg(not(target_os = "windows"))]
fn terminate_managed_processes(_lease: &UsbLease) -> usize {
    0
}

#[cfg(target_os = "windows")]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_os = "windows")]
fn observe_windows_bundle(bundle_root: &Path) -> Result<UsbObservation, AppError> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOVABLE;

    let path = bundle_root.to_string_lossy().replace('/', "\\");
    let drive_root = normalize_drive_root(&path)
        .ok_or_else(|| engine(format!("无法从路径识别 Windows 卷: {path}")))?;
    let root_wide = wide(&drive_root);
    let drive_type = unsafe { GetDriveTypeW(root_wide.as_ptr()) };
    if drive_type != DRIVE_REMOVABLE {
        return Ok(UsbObservation {
            removable: false,
            drive_root: Some(drive_root),
            bundle_path: path,
            fingerprint: None,
        });
    }

    let mut serial = 0_u32;
    let mut max_component = 0_u32;
    let mut flags = 0_u32;
    let mut volume_name = [0_u16; 256];
    let mut fs_name = [0_u16; 256];
    let ok = unsafe {
        GetVolumeInformationW(
            root_wide.as_ptr(),
            volume_name.as_mut_ptr(),
            volume_name.len() as u32,
            &mut serial,
            &mut max_component,
            &mut flags,
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(engine(format!(
            "读取 U 盘卷序列号失败: {}",
            io::Error::last_os_error()
        )));
    }
    let relative =
        bundle_relative_path(&path, &drive_root).ok_or_else(|| engine("U 盘目录不在卷根路径下"))?;
    let fingerprint = UsbFingerprint {
        volume_serial: serial,
        anchor_sha256: anchor_sha256(bundle_root)?,
        bundle_relative_path: relative,
    };
    Ok(UsbObservation {
        removable: true,
        drive_root: Some(drive_root),
        bundle_path: path,
        fingerprint: Some(fingerprint),
    })
}

#[cfg(not(target_os = "windows"))]
fn observe_windows_bundle(_bundle_root: &Path) -> Result<UsbObservation, AppError> {
    Err(AppError::UnsupportedPlatform)
}

pub fn observe_bundle(bundle_root: &Path) -> Result<UsbObservation, AppError> {
    observe_windows_bundle(bundle_root)
}

#[cfg(target_os = "windows")]
fn scan_removable_for(expected: &UsbFingerprint) -> Option<UsbObservation> {
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let relative = if expected.bundle_relative_path.is_empty() {
            String::new()
        } else {
            format!("\\{}", expected.bundle_relative_path.replace('/', "\\"))
        };
        let candidate = PathBuf::from(format!("{root}{relative}"));
        if let Ok(observation) = observe_windows_bundle(&candidate) {
            if observation
                .fingerprint
                .as_ref()
                .is_some_and(|actual| fingerprints_match(expected, actual))
            {
                return Some(observation);
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn scan_removable_for(_expected: &UsbFingerprint) -> Option<UsbObservation> {
    None
}

fn ensure_local_guard_binary(current_exe: &Path) -> Result<PathBuf, AppError> {
    let dir = local_guard_dir()?;
    fs::create_dir_all(&dir).map_err(|error| engine(format!("创建本地看门狗目录失败: {error}")))?;
    let target = dir.join(GUARD_EXE_NAME);
    if target != current_exe && !target.exists() {
        let temp = dir.join(format!("{GUARD_EXE_NAME}.tmp-{}", std::process::id()));
        fs::copy(current_exe, &temp)
            .map_err(|error| engine(format!("复制本地看门狗失败: {error}")))?;
        if let Err(error) = fs::rename(&temp, &target) {
            let _ = fs::remove_file(&temp);
            return Err(engine(format!("安装本地看门狗失败: {error}")));
        }
    }
    if target.exists() {
        return Ok(target);
    }
    Err(engine("本地看门狗可执行文件不存在"))
}

#[cfg(target_os = "windows")]
fn spawn_guard(executable: &Path, lease_path: &Path) -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(executable)
        .args([GUARD_ARG, "--lease-file", &lease_path.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| engine(format!("启动本地看门狗失败: {error}")))
}

#[cfg(not(target_os = "windows"))]
fn spawn_guard(_executable: &Path, _lease_path: &Path) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn start_for_manager() -> Result<UsbGuardStatus, AppError> {
    let executable =
        std::env::current_exe().map_err(|error| engine(format!("无法确定安装器路径: {error}")))?;
    let Some(bundle_root) = bundle_root_for_executable(&executable) else {
        let mut status = base_status("unsupported", "not-running");
        status.message = "未找到离线安装器根目录，U 盘绑定未启用".to_string();
        write_status(&status)?;
        return Ok(status);
    };
    let observation = observe_windows_bundle(&bundle_root)?;
    let Some(fingerprint) = observation.fingerprint.clone() else {
        let mut status = base_status("unsupported", "not-running");
        status.removable = observation.removable;
        status.drive_root = observation.drive_root;
        status.bundle_path = Some(observation.bundle_path);
        write_status(&status)?;
        return Ok(status);
    };

    let existing_license = match read_license(&bundle_root) {
        Ok(value) => value,
        Err(error) => {
            let mut status = base_status("mismatch", "not-running");
            status.removable = true;
            status.drive_root = observation.drive_root;
            status.bundle_path = Some(observation.bundle_path);
            status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
            status.message = error.to_string();
            let _ = write_status(&status);
            return Ok(status);
        }
    };
    let (license, signed) = match existing_license {
        Some(license) => match validate_license(&license, &fingerprint) {
            Ok(signed) => (license, signed),
            Err(error) => {
                let mut status = base_status("mismatch", "not-running");
                status.removable = true;
                status.drive_root = observation.drive_root;
                status.bundle_path = Some(observation.bundle_path);
                status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
                status.message = error.to_string();
                let _ = write_status(&status);
                return Ok(status);
            }
        },
        None => {
            let license = new_unsigned_license(fingerprint.clone());
            write_license(&bundle_root, &license)?;
            log::warn!(
                "USB license auto-enrolled as unsigned; sign release bundles before shipping"
            );
            (license, false)
        }
    };

    let lease = UsbLease {
        schema_version: LEASE_SCHEMA_VERSION,
        product: PRODUCT_ID.to_string(),
        lease_id: Uuid::new_v4().to_string(),
        bundle_path: observation.bundle_path.clone(),
        fingerprint: license.fingerprint.clone(),
        owner_pid: std::process::id(),
        managed_processes: vec![ManagedProcess {
            pid: std::process::id(),
            role: "manager".to_string(),
            executable: Some(executable.to_string_lossy().into_owned()),
            started_at_ms: now_ms(),
        }],
        issued_at: now_ms().to_string(),
    };
    write_lease(&lease)?;
    let local_binary = ensure_local_guard_binary(&executable)?;
    let lease_file = lease_path()?;
    let spawn_result = spawn_guard(&local_binary, &lease_file);
    let mut status = base_status(
        "authorized",
        if spawn_result.is_ok() {
            "running"
        } else {
            "not-running"
        },
    );
    status.removable = true;
    status.drive_root = observation.drive_root;
    status.bundle_path = Some(observation.bundle_path);
    status.bundle_relative_path = Some(license.fingerprint.bundle_relative_path);
    status.signed = signed;
    status.managed_processes = lease.managed_processes;
    status.message = if signed {
        "U 盘已验证，签名许可证有效".to_string()
    } else {
        "U 盘已验证（开发许可证未签名），看门狗运行中".to_string()
    };
    if let Err(error) = spawn_result {
        status.message = format!("{}；{error}", status.message);
    }
    write_status(&status)?;
    Ok(status)
}

#[cfg(not(target_os = "windows"))]
pub fn start_for_manager() -> Result<UsbGuardStatus, AppError> {
    let status = disabled_status();
    let _ = write_status(&status);
    Ok(status)
}

pub fn stop_for_manager() {
    remove_lease();
    let mut status = base_status("stopped", "not-running");
    status.message = status_message("stopped");
    let _ = write_status(&status);
}

pub fn stop_and_uninstall() -> Vec<String> {
    stop_for_manager();
    let mut notes = Vec::new();
    if let Ok(dir) = local_guard_dir() {
        // The helper may be between its one-second polling ticks after the
        // lease is removed.  Give it a few seconds to observe the clean stop
        // before declaring the local executable locked.
        for attempt in 0..12 {
            match fs::remove_dir_all(&dir) {
                Ok(()) => {
                    notes.push(format!("已删除本地 U 盘看门狗目录：{}", dir.display()));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(_error) if attempt < 11 => std::thread::sleep(Duration::from_millis(250)),
                Err(error) => notes.push(format!("本地 U 盘看门狗目录清理失败：{error}")),
            }
        }
    }
    notes
}

/// Headless entry point used by the copied local helper.  It deliberately
/// keeps the lease under the user profile so deleting/copying the USB tree
/// cannot create a second valid lease.
pub fn run_guard_cli() {
    #[cfg(not(target_os = "windows"))]
    {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let args = std::env::args().collect::<Vec<_>>();
        let lease_override = args
            .windows(2)
            .find(|pair| pair[0] == "--lease-file")
            .map(|pair| PathBuf::from(&pair[1]));
        let Some(lease_file) = lease_override.or_else(|| lease_path().ok()) else {
            return;
        };
        let Ok(lock) = acquire_guard_lock() else {
            return;
        };
        let _lock = lock;
        let mut missing_since: Option<std::time::Instant> = None;
        loop {
            let Some(lease) = read_json::<UsbLease>(&lease_file) else {
                break;
            };
            if lease.schema_version != LEASE_SCHEMA_VERSION || lease.product != PRODUCT_ID {
                break;
            }
            let present = scan_removable_for(&lease.fingerprint);
            let now = now_ms();
            if let Some(observation) = present {
                missing_since = None;
                let mut status = base_status("authorized", "running");
                status.removable = true;
                status.drive_root = observation.drive_root;
                status.bundle_path = Some(observation.bundle_path);
                status.bundle_relative_path = Some(lease.fingerprint.bundle_relative_path.clone());
                status.signed = read_json::<UsbLicense>(
                    &PathBuf::from(&lease.bundle_path).join(LICENSE_FILE_NAME),
                )
                .and_then(|license| license.signature)
                .is_some();
                status.last_checked_at_ms = now;
                status.managed_processes = lease.managed_processes.clone();
                let _ = write_json_atomic(
                    &status_path().unwrap_or_else(|_| lease_file.with_file_name(STATUS_FILE_NAME)),
                    &status,
                );
            } else {
                let since = missing_since.get_or_insert_with(std::time::Instant::now);
                let elapsed = since.elapsed();
                let remaining = Duration::from_millis(DISCONNECT_GRACE_MS).saturating_sub(elapsed);
                if elapsed >= Duration::from_millis(DISCONNECT_GRACE_MS) {
                    let terminated = terminate_managed_processes(&lease);
                    let mut status = base_status("missing", "not-running");
                    status.last_checked_at_ms = now;
                    status.missing_since_ms = Some(now.saturating_sub(elapsed.as_millis() as u64));
                    status.countdown_seconds = Some(0);
                    status.bundle_path = Some(lease.bundle_path.clone());
                    status.bundle_relative_path =
                        Some(lease.fingerprint.bundle_relative_path.clone());
                    status.managed_processes = lease.managed_processes.clone();
                    status.message = format!(
                        "{}（已结束 {terminated} 个受保护进程）",
                        status_message("missing")
                    );
                    let _ = write_json_atomic(
                        &status_path()
                            .unwrap_or_else(|_| lease_file.with_file_name(STATUS_FILE_NAME)),
                        &status,
                    );
                    let _ = fs::remove_file(&lease_file);
                    let _ = fs::remove_file(atomic_file::backup_path(&lease_file));
                    break;
                }
                let mut status = base_status("grace", "running");
                status.last_checked_at_ms = now;
                status.missing_since_ms = Some(now.saturating_sub(elapsed.as_millis() as u64));
                status.countdown_seconds =
                    Some((remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)).max(1));
                status.bundle_path = Some(lease.bundle_path.clone());
                status.bundle_relative_path = Some(lease.fingerprint.bundle_relative_path.clone());
                status.managed_processes = lease.managed_processes.clone();
                let _ = write_json_atomic(
                    &status_path().unwrap_or_else(|_| lease_file.with_file_name(STATUS_FILE_NAME)),
                    &status,
                );
            }
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }
    }
}

#[cfg(target_os = "windows")]
fn acquire_guard_lock() -> Result<File, AppError> {
    let path = lock_path()?;
    if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&path) {
        file.write_all(std::process::id().to_string().as_bytes())
            .map_err(|error| engine(format!("写入看门狗锁失败: {error}")))?;
        file.flush()
            .map_err(|error| engine(format!("刷新看门狗锁失败: {error}")))?;
        return Ok(file);
    }
    // The lock stores the owning helper PID.  Never reclaim a lock while that
    // process is alive; this prevents a second manager launch from starting a
    // competing helper and both writing the same status file.
    let stale = fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .map(|pid| !process_matches(pid, None))
        .unwrap_or(true);
    if stale {
        let _ = fs::remove_file(&path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| engine(format!("获取看门狗锁失败: {error}")))?;
        file.write_all(std::process::id().to_string().as_bytes())
            .map_err(|error| engine(format!("写入看门狗锁失败: {error}")))?;
        file.flush()
            .map_err(|error| engine(format!("刷新看门狗锁失败: {error}")))?;
        return Ok(file);
    }
    Err(engine("已有看门狗实例运行"))
}

#[cfg(not(target_os = "windows"))]
fn acquire_guard_lock() -> Result<File, AppError> {
    Err(AppError::UnsupportedPlatform)
}

pub fn guard_requested() -> bool {
    std::env::args().any(|arg| arg == GUARD_ARG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn normalizes_drive_roots_and_rejects_unc() {
        assert_eq!(normalize_drive_root("f:/"), Some("F:\\".to_string()));
        assert_eq!(
            normalize_drive_root("F:\\bundle\\setup.exe"),
            Some("F:\\".to_string())
        );
        assert_eq!(normalize_drive_root("\\\\server\\share"), None);
        assert_eq!(normalize_drive_root("relative\\setup.exe"), None);
    }

    #[test]
    fn relative_path_is_case_insensitive_but_rejects_escape() {
        assert_eq!(
            bundle_relative_path("F:\\Codex\\Setup", "f:\\"),
            Some("codex/setup".to_string())
        );
        assert_eq!(bundle_relative_path("F:\\..\\other", "F:\\"), None);
        assert_eq!(bundle_relative_path("G:\\Codex", "F:\\"), None);
    }

    #[test]
    fn fingerprint_allows_drive_letter_change_only() {
        let expected = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: "Aa".repeat(32),
            bundle_relative_path: "codex/setup".to_string(),
        };
        let same = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: expected.anchor_sha256.to_uppercase(),
            bundle_relative_path: "CODEX/SETUP".to_string(),
        };
        let copied = UsbFingerprint {
            volume_serial: 43,
            ..same.clone()
        };
        assert!(fingerprints_match(&expected, &same));
        assert!(!fingerprints_match(&expected, &copied));
    }

    #[test]
    fn disconnect_state_has_five_second_boundary() {
        let grace = next_disconnect_state(None, false, 10_000, 5_000);
        assert_eq!(grace.countdown_seconds, Some(5));
        assert!(!grace.expired);
        let grace_late = next_disconnect_state(grace.missing_since_ms, false, 14_999, 5_000);
        assert_eq!(grace_late.countdown_seconds, Some(1));
        let expired = next_disconnect_state(grace.missing_since_ms, false, 15_000, 5_000);
        assert!(expired.expired);
        assert_eq!(expired.countdown_seconds, Some(0));
        assert_eq!(
            next_disconnect_state(expired.missing_since_ms, true, 15_001, 5_000).missing_since_ms,
            None
        );
    }

    #[test]
    fn signed_license_round_trip_and_tamper_rejection() {
        let signing = SigningKey::from_bytes(&[11u8; 32]);
        let mut license = new_unsigned_license(UsbFingerprint {
            volume_serial: 7,
            anchor_sha256: "b".repeat(64),
            bundle_relative_path: "codex".to_string(),
        });
        let payload = unsigned_license_payload(&license).unwrap();
        license.signature = Some(B64.encode(signing.sign(&payload).to_bytes()));
        let public = B64.encode(signing.verifying_key().to_bytes());
        assert!(verify_signature_with_key(&license, &public).is_ok());
        license.fingerprint.volume_serial = 8;
        assert!(verify_signature_with_key(&license, &public).is_err());
    }

    #[test]
    fn process_scope_uses_pid_and_image_marker() {
        assert_eq!(
            executable_name(r"C:\Programs\CodexSwitch.exe"),
            Some("codexswitch.exe".to_string())
        );
        assert!(executable_name("Codex.exe").is_some());
    }
}
