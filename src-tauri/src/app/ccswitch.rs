//! Product configurator hand-off for the Codex USB installer.
//!
//! The Codex installer itself is intentionally kept separate from the
//! CCSwitch product binary.  The normal manager edition copies the verified
//! payload into a per-user, non-removable location.  The USB edition starts
//! the verified executable directly from the removable bundle, so removing
//! the USB also removes the program bytes needed for a new launch.  CCSwitch's
//! own settings/database continue to use its ordinary Windows user-profile
//! locations; the manager never redirects those to the USB or stores an API
//! key in the bundle.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use tauri::{AppHandle, Manager};

use crate::errors::AppError;

pub const CCSWITCH_EXECUTABLE: &str = "CodexSwitch.exe";
const CCSWITCH_SOURCE_DIR: &str = "ccswitch";
pub const CCSWITCH_INSTALL_DIR: &str = "CodexSwitch";
const CCSWITCH_MANAGED_MARKER: &str = ".codex-usb-managed.json";
/// USB mode does not create a local program directory, so ownership is kept
/// in the manager data directory instead.  This lets a later explicit purge
/// remove CCSwitch data even after the USB executable has been unplugged.
const CCSWITCH_USB_MARKER: &str = "ccswitch-usb-managed.json";
const CCSWITCH_DATA_DIR: &str = ".cc-switch";
const CCSWITCH_IDENTIFIER: &str = "com.ccswitch.desktop";
#[cfg(target_os = "windows")]
const CCSWITCH_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const CCSWITCH_RUN_VALUE: &str = "CC Switch";
const WEBVIEW2_ARGUMENTS_ENV: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
const WEBVIEW2_DISABLE_GPU_ARG: &str = "--disable-gpu";
const CCSWITCH_USB_BACKGROUND_ENV: &str = "CODEX_USB_CCSWITCH_BACKGROUND";
const CCSWITCH_404TOKEN_DEVICE_KEY_ENV: &str = "CCSWITCH_404TOKEN_DEVICE_KEY";
const CCSWITCH_404TOKEN_ACCOUNT_ENV: &str = "CCSWITCH_404TOKEN_ACCOUNT";
const CCSWITCH_404TOKEN_GROUP_ENV: &str = "CCSWITCH_404TOKEN_GROUP_ID";
const CCSWITCH_404TOKEN_PROVIDER_ENV: &str = "CCSWITCH_404TOKEN_PROVIDER";
const CCSWITCH_USB_CODEX_HOME_ENV: &str = "CCSWITCH_USB_CODEX_HOME";
const CCSWITCH_404TOKEN_HANDOFF_PATH_ENV: &str = "CCSWITCH_404TOKEN_HANDOFF_PATH";
const CCSWITCH_404TOKEN_HANDOFF_NONCE_ENV: &str = "CCSWITCH_404TOKEN_HANDOFF_NONCE";
const CCSWITCH_404TOKEN_HANDOFF_PROTOCOL: &str = "CODEX_USB_404TOKEN_HANDOFF_V1";
const CCSWITCH_404TOKEN_PROVIDER_ID: &str = "404token-sub2api";
const CCSWITCH_HANDOFF_ACK_MAX_BYTES: u64 = 4096;
const CCSWITCH_HANDOFF_ACK_TIMEOUT: Duration = Duration::from_secs(15);

/// Files copied when they exist beside the product executable.  Only the EXE
/// is required at runtime; the catalog and notices make the local install
/// self-describing and preserve the open-source attribution.
const CCSWITCH_PAYLOAD_FILES: &[&str] = &[
    "CodexSwitch.exe",
    "providers.json",
    "manifest.json",
    "LICENSE",
    "SOURCE-NOTICE.txt",
];

#[derive(Debug, Deserialize)]
struct CcswitchManifest {
    artifact: CcswitchArtifact,
}

#[derive(Debug, Deserialize)]
struct CcswitchArtifact {
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CcswitchHandoffAck {
    protocol: String,
    nonce: String,
    status: String,
    provider_id: String,
    credential_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcswitchHandoffOutcome {
    Verified,
    Failed,
    Unconfirmed,
}

/// A single-use challenge passed only to the signed USB CCSwitch child.
/// The acknowledgement contains a one-way key fingerprint, never the key.
struct CcswitchHandoffRequest {
    path: PathBuf,
    nonce: String,
    credential_sha256: String,
}

impl CcswitchHandoffRequest {
    fn new(device_key: &str) -> Result<Self, AppError> {
        let temp_dir = fs::canonicalize(std::env::temp_dir())
            .map_err(|error| AppError::Internal(format!("无法确认 CCSwitch 交接目录: {error}")))?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let path = temp_dir.join(format!("codex-usb-ccswitch-handoff-{nonce}.json"));
        if fs::symlink_metadata(&path).is_ok() {
            return Err(AppError::Internal(
                "CCSwitch 交接确认文件名冲突".to_string(),
            ));
        }
        let digest = sha2::Sha256::digest(device_key.as_bytes());
        let credential_sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            path,
            nonce,
            credential_sha256,
        })
    }

    fn outcome_from_file(&self) -> Option<CcswitchHandoffOutcome> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => return Some(CcswitchHandoffOutcome::Unconfirmed),
        };
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() == 0
            || metadata.len() > CCSWITCH_HANDOFF_ACK_MAX_BYTES
        {
            return Some(CcswitchHandoffOutcome::Unconfirmed);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Some(CcswitchHandoffOutcome::Unconfirmed);
            }
        }
        let bytes = match fs::read(&self.path) {
            Ok(bytes) if bytes.len() as u64 <= CCSWITCH_HANDOFF_ACK_MAX_BYTES => bytes,
            _ => return Some(CcswitchHandoffOutcome::Unconfirmed),
        };
        let ack: CcswitchHandoffAck = match serde_json::from_slice(&bytes) {
            Ok(ack) => ack,
            Err(_) => return Some(CcswitchHandoffOutcome::Unconfirmed),
        };
        if ack.protocol != CCSWITCH_404TOKEN_HANDOFF_PROTOCOL
            || ack.nonce != self.nonce
            || ack.provider_id != CCSWITCH_404TOKEN_PROVIDER_ID
        {
            return Some(CcswitchHandoffOutcome::Unconfirmed);
        }
        match ack.status.as_str() {
            "verified" if ack.credential_sha256.as_deref() == Some(&self.credential_sha256) => {
                Some(CcswitchHandoffOutcome::Verified)
            }
            "failed" | "missing-credential" | "invalid-provider" => {
                Some(CcswitchHandoffOutcome::Failed)
            }
            _ => Some(CcswitchHandoffOutcome::Unconfirmed),
        }
    }

    fn wait_for_outcome(&self, timeout: Duration) -> CcswitchHandoffOutcome {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(outcome) = self.outcome_from_file() {
                return outcome;
            }
            if Instant::now() >= deadline {
                return CcswitchHandoffOutcome::Unconfirmed;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for CcswitchHandoffRequest {
    fn drop(&mut self) {
        // This path is generated under the system temporary directory with a
        // fresh UUID and is the only manager-owned hand-off file removed here.
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CcswitchLaunchReport {
    /// Whether a CCSwitch executable was found in the installer bundle.
    pub available: bool,
    /// Whether the payload is available in the selected launch mode. In USB
    /// mode this is true even though no local copy is created.
    pub installed: bool,
    /// Whether the local executable was successfully spawned.
    pub launched: bool,
    pub source_path: Option<String>,
    pub installed_path: Option<String>,
    pub copied_files: usize,
    /// `usb` means the source executable itself remains on the removable
    /// bundle; `local` means the normal per-user copy was launched.
    #[serde(default)]
    pub launch_mode: String,
    /// Human-facing data scope. CCSwitch stores settings in the current user's
    /// profile in both modes.
    #[serde(default)]
    pub data_scope: String,
    /// Process creation is not enough for a GUI hand-off. This becomes true
    /// only after a sizeable top-level window under the exact CCSwitch program
    /// root has been restored and presented.
    #[serde(default)]
    pub usable_window_confirmed: bool,
    #[serde(default)]
    pub graphics_mode: String,
    /// Whether the signed USB binding was passed to the CCSwitch child.
    /// This only means it was handed to the process; it does not imply that
    /// CCSwitch saved the key or selected the provider.
    #[serde(default)]
    pub account_injected: bool,
    /// `pending`, `verified`, `unconfirmed`, `not-attempted`, `missing-binding`,
    /// `failed`, or `unavailable`.
    #[serde(default)]
    pub account_injection_status: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcswitchLaunchIntent {
    /// First install/bootstrap: apply the one-time tray hand-off after the
    /// account has been injected.
    Bootstrap,
    /// Explicit user action: reuse the managed process when possible and
    /// restore its window; never hide it after launch.
    UserOpen,
}

/// Result of removing the per-user CCSwitch copy installed by this manager.
/// The operation is deliberately idempotent: a missing directory is treated
/// as already removed so a second cleanup attempt can finish after a locked
/// process has exited.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CcswitchUninstallReport {
    pub success: bool,
    pub installed_before: bool,
    pub removed_install: bool,
    pub removed_autostart: bool,
    pub purged_user_data: bool,
    pub install_dir: String,
    pub data_dirs: Vec<String>,
    pub message: String,
    pub notes: Vec<String>,
}

fn report_missing(message: impl Into<String>) -> CcswitchLaunchReport {
    CcswitchLaunchReport {
        available: false,
        installed: false,
        launched: false,
        source_path: None,
        installed_path: None,
        copied_files: 0,
        launch_mode: "none".to_string(),
        data_scope: "unknown".to_string(),
        usable_window_confirmed: false,
        graphics_mode: "unknown".to_string(),
        account_injected: false,
        account_injection_status: "unavailable".to_string(),
        message: message.into(),
    }
}

fn webview2_software_rendering_arguments(existing: Option<&str>) -> String {
    let existing = existing.unwrap_or_default().trim();
    if existing
        .split_ascii_whitespace()
        .any(|argument| argument.eq_ignore_ascii_case(WEBVIEW2_DISABLE_GPU_ARG))
    {
        return existing.to_string();
    }
    if existing.is_empty() {
        WEBVIEW2_DISABLE_GPU_ARG.to_string()
    } else {
        format!("{existing} {WEBVIEW2_DISABLE_GPU_ARG}")
    }
}

fn ccswitch_account_environment(
    binding: Option<&codex_usb_license::UsbAccountBinding>,
) -> Vec<(&'static str, String)> {
    let Some(binding) = binding else {
        return Vec::new();
    };
    if binding.ccswitch.provider_id != "404token-sub2api"
        || binding.ccswitch.device_api_key.trim().is_empty()
    {
        return Vec::new();
    }
    vec![
        (
            CCSWITCH_404TOKEN_DEVICE_KEY_ENV,
            binding.ccswitch.device_api_key.trim().to_string(),
        ),
        (CCSWITCH_404TOKEN_ACCOUNT_ENV, binding.account_ref.clone()),
        (
            CCSWITCH_404TOKEN_GROUP_ENV,
            binding.ccswitch.group_id.to_string(),
        ),
        (
            CCSWITCH_404TOKEN_PROVIDER_ENV,
            binding.ccswitch.provider_id.clone(),
        ),
    ]
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> AppError {
    AppError::Engine(format!("写入 {} 失败: {error}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    let digest = sha2::Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn managed_marker_path(install_dir: &Path) -> PathBuf {
    install_dir.join(CCSWITCH_MANAGED_MARKER)
}

fn local_install_is_managed(install_dir: &Path) -> bool {
    if codex_win_engine::validate_cleanup_path(install_dir).is_err() {
        return false;
    }
    let Ok(install_metadata) = fs::symlink_metadata(install_dir) else {
        return false;
    };
    if !install_metadata.is_dir() || install_metadata.file_type().is_symlink() {
        return false;
    }
    let marker = managed_marker_path(install_dir);
    if !is_regular_file(&marker) {
        return false;
    }
    let Ok(bytes) = fs::read(marker) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    value.get("product").and_then(|v| v.as_str()) == Some("codex-usb-oneclick")
        && value.get("component").and_then(|v| v.as_str()) == Some("ccswitch")
        && value.get("executable").and_then(|v| v.as_str()) == Some(CCSWITCH_EXECUTABLE)
}

fn usb_marker_path() -> Result<PathBuf, AppError> {
    crate::app::paths::data_dir()
        .map(|dir| dir.join(CCSWITCH_USB_MARKER))
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch USB 所有权记录目录".to_string()))
}

fn usb_marker_background_hidden_for(source: &Path) -> bool {
    let path = match usb_marker_path() {
        Ok(path) => path,
        Err(_) => return false,
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let Some(previous) = value.get("sourcePath").and_then(|value| value.as_str()) else {
        return false;
    };
    value
        .get("backgroundHidden")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        && same_windows_path(Path::new(previous), source)
}

fn write_usb_managed_marker(source: &Path, background_hidden: bool) -> Result<(), AppError> {
    let path = usb_marker_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let marker = serde_json::json!({
        "product": "codex-usb-oneclick",
        "component": "ccswitch",
        "launchMode": "usb",
        "sourcePath": source.to_string_lossy(),
        "backgroundHidden": background_hidden,
        "createdAtMs": crate::app::usb_guard::now_ms(),
    });
    let bytes = serde_json::to_vec_pretty(&marker).map_err(|error| {
        AppError::Engine(format!("序列化 CCSwitch USB 所有权记录失败: {error}"))
    })?;
    crate::app::atomic_file::write_atomic(&path, &bytes)
        .map_err(|error| AppError::Engine(format!("写入 CCSwitch USB 所有权记录失败: {error}")))
}

fn read_usb_managed_source() -> Option<PathBuf> {
    let path = usb_marker_path().ok()?;
    codex_win_engine::validate_cleanup_path(&path).ok()?;
    let bytes = fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("product").and_then(|v| v.as_str()) != Some("codex-usb-oneclick")
        || value.get("component").and_then(|v| v.as_str()) != Some("ccswitch")
        || value.get("launchMode").and_then(|v| v.as_str()) != Some("usb")
    {
        return None;
    }
    let source = value.get("sourcePath").and_then(|v| v.as_str())?.trim();
    if source.is_empty() {
        return None;
    }
    let path = PathBuf::from(source);
    // The marker is user-writable state, not an authorization token.  Accept
    // only the exact executable name and a non-root parent so a corrupted
    // marker can never broaden the process-close scan to an entire drive.
    let valid_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(CCSWITCH_EXECUTABLE));
    let valid_parent = path.parent().is_some_and(|parent| {
        parent.parent().is_some()
            && parent
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(CCSWITCH_SOURCE_DIR))
    });
    (path.is_absolute()
        && valid_name
        && valid_parent
        && codex_win_engine::validate_cleanup_path(&path).is_ok())
    .then_some(path)
}

fn remove_usb_managed_marker() -> Result<bool, AppError> {
    let path = usb_marker_path()?;
    let removed = remove_path_retry(&path)?;
    let backup = crate::app::atomic_file::backup_path(&path);
    let _ = remove_path_retry(&backup)?;
    Ok(removed)
}

fn write_managed_marker(install_dir: &Path) -> Result<(), AppError> {
    let marker = serde_json::json!({
        "product": "codex-usb-oneclick",
        "component": "ccswitch",
        "executable": CCSWITCH_EXECUTABLE,
        "createdAtMs": crate::app::usb_guard::now_ms(),
    });
    fs::write(
        managed_marker_path(install_dir),
        serde_json::to_vec_pretty(&marker).unwrap(),
    )
    .map_err(|error| io_error(&managed_marker_path(install_dir), error))
}

fn verify_source_executable(source: &Path) -> Result<(), AppError> {
    let source_dir = source
        .parent()
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch 载荷目录".to_string()))?;
    let manifest_path = source_dir.join("manifest.json");
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|error| io_error(&manifest_path, error))?;
    // Windows PowerShell 5.1's `Set-Content -Encoding UTF8` writes a UTF-8
    // BOM. Older USB bundles therefore contain valid JSON that serde_json
    // rejects at byte zero. Accept that legacy encoding while keeping the
    // executable hash gate unchanged.
    let manifest_json = manifest_bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
        .unwrap_or(&manifest_bytes);
    let manifest: CcswitchManifest =
        serde_json::from_slice(manifest_json).map_err(|error| io_error(&manifest_path, error))?;
    let expected = manifest.artifact.sha256.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Engine(
            "CCSwitch manifest 中的 sha256 无效".to_string(),
        ));
    }
    let actual = sha256_file(source)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::Engine(format!(
            "CCSwitch 载荷校验失败: {actual} != {expected}"
        )));
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Return the candidate locations in deterministic priority order.  A
/// sibling of the running installer is the USB layout; resource_dir covers a
/// future NSIS/MSIX bundle that embeds `ccswitch/*` as Tauri resources.
fn source_candidates(executable: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = executable.parent() {
        candidates.push(parent.join(CCSWITCH_SOURCE_DIR).join(CCSWITCH_EXECUTABLE));
        candidates.push(
            parent
                .join("resources")
                .join(CCSWITCH_SOURCE_DIR)
                .join(CCSWITCH_EXECUTABLE),
        );
    }
    if let Some(resource_dir) = resource_dir {
        candidates.push(
            resource_dir
                .join(CCSWITCH_SOURCE_DIR)
                .join(CCSWITCH_EXECUTABLE),
        );
        candidates.push(resource_dir.join(CCSWITCH_EXECUTABLE));
    }
    candidates
}

fn find_verified_source(
    executable: &Path,
    resource_dir: Option<&Path>,
) -> Result<Option<PathBuf>, AppError> {
    let mut rejected = Vec::new();
    for candidate in source_candidates(executable, resource_dir) {
        if !is_regular_file(&candidate) {
            continue;
        }
        match verify_source_executable(&candidate) {
            Ok(()) => return Ok(Some(candidate)),
            Err(error) => {
                log::warn!(
                    "CCSwitch candidate rejected by adjacent manifest path={} error={}",
                    candidate.display(),
                    error
                );
                rejected.push(format!("{}: {}", candidate.display(), error));
            }
        }
    }
    if rejected.is_empty() {
        Ok(None)
    } else {
        Err(AppError::Engine(format!(
            "找到 CCSwitch 程序，但所有候选都未通过相邻清单校验：{}",
            rejected.join("；")
        )))
    }
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    let left = left
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let right = right
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    left == right
}

pub fn local_install_dir() -> Result<PathBuf, AppError> {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local_app_data)
            .join("Programs")
            .join(CCSWITCH_INSTALL_DIR));
    }

    // This fallback keeps the command testable on non-Windows hosts while
    // still following the per-user data convention when LOCALAPPDATA is not
    // present (for example in a restricted Windows shell).
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join(CCSWITCH_INSTALL_DIR))
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch 的本地安装目录".to_string()))
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(base_dirs) = directories::BaseDirs::new() {
        dirs.push(base_dirs.home_dir().join(CCSWITCH_DATA_DIR));
    }

    // Tauri's WebView profile is separate from CCSwitch's SQLite/config
    // directory.  Include both Windows profile roots so a full purge also
    // removes the first-run marker and cached UI state, regardless of the
    // WebView runtime version used on the target machine.
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            dirs.push(PathBuf::from(root).join(CCSWITCH_IDENTIFIER));
        }
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

fn existence_from_probe(probe: std::io::Result<()>) -> std::io::Result<bool> {
    match probe {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn path_exists_checked(path: &Path) -> Result<bool, AppError> {
    existence_from_probe(fs::symlink_metadata(path).map(|_| ()))
        .map_err(|error| io_error(path, error))
}

fn paths_absent(paths: &[PathBuf]) -> Result<bool, AppError> {
    for path in paths {
        if path_exists_checked(path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A directory at the conventional path without this product's ownership
/// marker is intentionally preserved.  It may belong to a standalone
/// CCSwitch installation, so it must not be deleted; its presence therefore
/// is not a failed postcondition for this product's uninstall.
fn local_install_cleanup_settled(local_install_present: bool, managed_marker: bool) -> bool {
    !local_install_present || !managed_marker
}

fn remove_path_retry(path: &Path) -> Result<bool, AppError> {
    let mut last_error = None;
    // CCSwitch/WebView can release a handle shortly after its process exits.
    // Give Windows a bounded 10-second window rather than failing after the
    // old 750 ms budget and leaving a false "partially uninstalled" result.
    for attempt in 0..40 {
        codex_win_engine::validate_cleanup_path(path)
            .map_err(|error| AppError::Engine(error.to_string()))?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(attempt > 0),
            Err(error) => return Err(io_error(path, error)),
        };
        let result = if metadata.file_type().is_symlink() || !metadata.is_dir() {
            fs::remove_file(path)
        } else {
            fs::remove_dir_all(path)
        };
        match result {
            Ok(()) => {
                // Removal returning Ok does not prove postcondition: only an
                // explicit NotFound probe confirms absence. AccessDenied and
                // other lookup errors must never turn into successful cleanup.
                if path_exists_checked(path)? {
                    last_error = Some(std::io::Error::other("path still exists after removal"));
                } else {
                    return Ok(true);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Another cleanup attempt may have won the race. Verify the
                // same path rather than treating that benign race as a hard
                // failure.
                match fs::symlink_metadata(path) {
                    Err(check) if check.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                    Err(check) => return Err(io_error(path, check)),
                    Ok(_) => {}
                }
                last_error = Some(error);
            }
            Err(error) => {
                let transient = matches!(error.raw_os_error(), Some(5 | 32 | 33))
                    || matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    );
                last_error = Some(error);
                if !transient {
                    return Err(io_error(
                        path,
                        last_error
                            .as_ref()
                            .expect("remove_path_retry must retain non-transient error"),
                    ));
                }
            }
        }
        if attempt < 39 {
            thread::sleep(Duration::from_millis(250));
        }
    }

    Err(io_error(
        path,
        last_error.expect("remove_path_retry must record its last error"),
    ))
}

#[cfg(target_os = "windows")]
fn terminate_running_process(local_install_owned: bool) -> Result<bool, AppError> {
    // Prefer the USB lease's exact PID + image check.  Normal (non-USB)
    // launches have no lease, so also scan the two paths this product can own
    // and close only processes whose executable image is physically beneath
    // one of those roots.  This avoids a global `/IM CodexSwitch.exe` kill that
    // could touch an unrelated CCSwitch installation while still allowing the
    // uninstall button to work after a lease write failed or an old build was
    // used.
    let mut closed = crate::app::usb_guard::terminate_managed_processes_for_role("ccswitch") > 0;
    let mut roots = Vec::new();
    if local_install_owned {
        roots.push(local_install_dir()?);
    }
    if let Some(source) = read_usb_managed_source() {
        if let Some(parent) = source.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    roots.sort();
    roots.dedup();
    for root in roots {
        match codex_win_engine::processes_running_for_root(&root) {
            Ok(true) => {
                codex_win_engine::close_processes_for_root(10, &root)
                    .map_err(|error| AppError::Engine(error.to_string()))?;
                closed = true;
            }
            Ok(false) => {}
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "无法检查 CCSwitch 进程（{}）：{error}",
                    root.display()
                )))
            }
        }
    }
    if closed {
        thread::sleep(Duration::from_millis(150));
    }
    Ok(closed)
}

#[cfg(not(target_os = "windows"))]
fn terminate_running_process(_local_install_owned: bool) -> Result<bool, AppError> {
    Ok(false)
}

#[cfg(target_os = "windows")]
fn remove_autostart_entry_for_paths(paths: &[&Path]) -> Result<bool, AppError> {
    use windows_sys::Win32::System::Registry::*;
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn failure(status: u32) -> AppError {
        AppError::Engine(format!("CCSwitch 启动项注册表操作失败（Win32 {status}）"))
    }
    fn open(access: u32) -> Result<Option<Key>, AppError> {
        let path: Vec<u16> = CCSWITCH_RUN_KEY
            .trim_start_matches("HKCU\\")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut key = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) };
        if status == 2 {
            return Ok(None);
        }
        if status != 0 {
            return Err(failure(status));
        }
        Ok(Some(Key(key)))
    }
    fn query(key: &Key, name: &[u16]) -> Result<Option<String>, AppError> {
        let (mut kind, mut size) = (0, 0);
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == 2 {
            return Ok(None);
        }
        if status != 0 {
            return Err(failure(status));
        }
        if kind != REG_SZ || size > 65536 || size % 2 != 0 {
            return Err(AppError::Engine(
                "CCSwitch 启动项不是正常 REG_SZ，保留该值".into(),
            ));
        }
        let mut value = vec![0u16; size as usize / 2];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                value.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return Err(failure(status));
        }
        value.truncate(size as usize / 2);
        if kind != REG_SZ || size % 2 != 0 {
            return Err(AppError::Engine(
                "读取期间 CCSwitch 启动项类型发生变化，已保留".into(),
            ));
        }
        while value.last() == Some(&0) {
            value.pop();
        }
        String::from_utf16(&value)
            .map(Some)
            .map_err(|e| AppError::Engine(format!("CCSwitch 启动项编码无效：{e}")))
    }
    let Some(key) = open(KEY_QUERY_VALUE)? else {
        return Ok(false);
    };
    let name: Vec<u16> = CCSWITCH_RUN_VALUE
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let Some(value) = query(&key, &name)? else {
        return Ok(false);
    };
    let expected = paths
        .iter()
        .map(|path| {
            path.join(CCSWITCH_EXECUTABLE)
                .to_string_lossy()
                .to_ascii_lowercase()
                .replace('/', "\\")
        })
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if !expected.iter().any(|path| autostart_targets(&value, path)) {
        return Ok(false);
    }
    let Some(write_key) = open(KEY_SET_VALUE | KEY_QUERY_VALUE)? else {
        return Ok(false);
    };
    if query(&write_key, &name)?.as_deref() != Some(value.as_str()) {
        return Err(AppError::Engine(
            "CCSwitch 启动项已发生外部变化，保留新值".into(),
        ));
    }
    let status = unsafe { RegDeleteValueW(write_key.0, name.as_ptr()) };
    if status != 0 && status != 2 {
        return Err(failure(status));
    }
    if query(&write_key, &name)?.is_some() {
        return Err(AppError::Engine(
            "CCSwitch 启动项删除后仍存在，请关闭其他配置程序后重试".into(),
        ));
    }
    Ok(true)
}

#[cfg(any(target_os = "windows", test))]
fn autostart_targets(command: &str, executable: &str) -> bool {
    let normalized = command.trim().to_ascii_lowercase().replace('/', "\\");
    let expected = executable.to_ascii_lowercase().replace('/', "\\");
    if let Some(quoted) = normalized.strip_prefix('"') {
        return quoted.split_once('"').is_some_and(|(path, tail)| {
            path == expected && (tail.is_empty() || tail.starts_with(char::is_whitespace))
        });
    }
    normalized == expected
        || normalized
            .strip_prefix(&expected)
            .is_some_and(|tail| tail.starts_with(char::is_whitespace))
}

#[cfg(not(target_os = "windows"))]
fn remove_autostart_entry_for_paths(_paths: &[&Path]) -> Result<bool, AppError> {
    Ok(false)
}

/// Remove the local CCSwitch program copy and, when requested, its per-user
/// configuration/WebView data.  No Codex data is touched here; that remains
/// the responsibility of the Codex uninstall engine.
pub fn uninstall(purge_user_data: bool) -> Result<CcswitchUninstallReport, AppError> {
    let install_dir = local_install_dir()?;
    let usb_source = read_usb_managed_source();
    let local_install_present = path_exists_checked(&install_dir)?;
    let installed_before = local_install_present || usb_source.is_some();
    let managed_marker = local_install_is_managed(&install_dir);
    let owned_by_bundle = managed_marker || usb_source.is_some();
    let data_paths = data_dirs();
    let mut notes = Vec::new();

    // A default path is not ownership proof: a user may have installed an
    // unrelated CCSwitch at the same location.  Only close/remove the local
    // tree when this product's marker is present; USB mode has its separate
    // validated source marker and exact-root scan.
    let terminated = terminate_running_process(managed_marker)?;
    if terminated {
        notes.push("已关闭正在运行的 CCSwitch 进程。".to_string());
    }

    let mut autostart_roots = Vec::new();
    if managed_marker {
        autostart_roots.push(install_dir.as_path());
    }
    if let Some(source) = usb_source.as_deref() {
        if let Some(parent) = source.parent() {
            autostart_roots.push(parent);
        }
    }
    let removed_autostart = remove_autostart_entry_for_paths(&autostart_roots)?;
    if removed_autostart {
        notes.push("已移除 CCSwitch 开机启动项。".to_string());
    }

    let removed_install = if managed_marker {
        remove_path_retry(&install_dir)?
    } else {
        false
    };
    if local_install_present && !managed_marker {
        notes.push(format!(
            "检测到 {}，但缺少本产品所有权标记；为避免删除其他安装，已保留该目录。",
            install_dir.display()
        ));
    }
    if removed_install {
        notes.push(format!(
            "已删除 CCSwitch 程序目录：{}",
            install_dir.display()
        ));
    }

    let mut purged_user_data = false;
    if purge_user_data {
        let mut removed_any = false;
        for data_path in &data_paths {
            if remove_path_retry(data_path)? {
                removed_any = true;
                notes.push(format!("已删除 CCSwitch 数据目录：{}", data_path.display()));
            }
        }
        purged_user_data = removed_any || paths_absent(&data_paths)?;
        if !owned_by_bundle {
            notes.push(
                "未找到 CCSwitch 所有权标记；因你明确选择了清除数据，仍按联合卸载确认清理已知用户目录。"
                    .to_string(),
            );
        }
    } else {
        notes.push("已保留 CCSwitch 配置与用户数据。".to_string());
    }

    // The marker is operational metadata, not user data.  Remove it on every
    // completed uninstall so a stale USB session cannot make the next launch
    // look installed.  If data purge failed, retain it as a retry signal.
    let marker_removed = if !purge_user_data || paths_absent(&data_paths)? {
        remove_usb_managed_marker()?
    } else {
        false
    };
    if marker_removed {
        notes.push("已清除 CCSwitch USB 所有权记录。".to_string());
    }
    let local_install_absent = !path_exists_checked(&install_dir)?;
    let data_absent = paths_absent(&data_paths)?;
    let marker_absent = !path_exists_checked(&usb_marker_path()?)?;
    let local_install_settled =
        local_install_cleanup_settled(!local_install_absent, managed_marker);
    let success = local_install_settled && (!purge_user_data || data_absent) && marker_absent;
    let message = if !success {
        "CCSwitch 程序已部分清理，请关闭占用文件的进程后重试。".to_string()
    } else if local_install_present && !managed_marker {
        "CCSwitch 本产品已卸载；检测到其他 CCSwitch 安装，已按安全策略保留。".to_string()
    } else if installed_before || removed_autostart || purged_user_data {
        if purge_user_data {
            "CCSwitch 已卸载并清除配置。".to_string()
        } else {
            "CCSwitch 已卸载，配置已保留。".to_string()
        }
    } else {
        "未检测到 CCSwitch 安装。".to_string()
    };

    Ok(CcswitchUninstallReport {
        success,
        installed_before,
        removed_install,
        removed_autostart,
        purged_user_data,
        install_dir: install_dir.to_string_lossy().into_owned(),
        data_dirs: data_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        message,
        notes,
    })
}

fn copy_payload(source_executable: &Path, target_dir: &Path) -> Result<usize, AppError> {
    fs::create_dir_all(target_dir).map_err(|error| io_error(target_dir, error))?;
    let source_dir = source_executable
        .parent()
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch 载荷目录".to_string()))?;
    let target_executable = target_dir.join(CCSWITCH_EXECUTABLE);
    let temp_executable = target_dir.join(format!(".{CCSWITCH_EXECUTABLE}.new"));

    // Copy the executable through a sibling temporary file.  This avoids a
    // partially written EXE if the USB read is interrupted.  Replacing a
    // currently running executable can fail on Windows; in that case the
    // existing installation remains intact and the error is surfaced to the
    // caller rather than deleting it.
    let mut copied = 0usize;
    for file_name in CCSWITCH_PAYLOAD_FILES {
        let source = source_dir.join(file_name);
        if !is_regular_file(&source) {
            continue;
        }
        let target = target_dir.join(file_name);
        if *file_name == CCSWITCH_EXECUTABLE {
            let already_current = target.exists()
                && sha256_file(&source)
                    .ok()
                    .zip(sha256_file(&target).ok())
                    .is_some_and(|(left, right)| left.eq_ignore_ascii_case(&right));
            if !already_current {
                fs::copy(&source, &temp_executable)
                    .map_err(|error| io_error(&temp_executable, error))?;
                if target.exists() {
                    // Do not delete a locked executable before we know the
                    // replacement can be committed; this was the source of
                    // the previous "Access is denied (os error 5)" report.
                    if let Err(error) = fs::remove_file(&target) {
                        let _ = fs::remove_file(&temp_executable);
                        return Err(io_error(&target, error));
                    }
                }
                if let Err(error) = fs::rename(&temp_executable, &target) {
                    let _ = fs::remove_file(&temp_executable);
                    return Err(io_error(&target, error));
                }
            }
        } else {
            fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        }
        copied += 1;
    }

    if !is_regular_file(&target_executable) {
        return Err(AppError::Engine(
            "CCSwitch 载荷中缺少 CodexSwitch.exe".to_string(),
        ));
    }
    write_managed_marker(target_dir)?;
    Ok(copied)
}

/// Verify CCSwitch and launch it. In the fixed USB layout the executable is
/// deliberately *not* copied to C:; only the app's normal user-profile data
/// remains local. A missing source is represented as a non-fatal report so the
/// caller can still mark Codex installation complete.
fn launch_with_intent(
    app: &AppHandle,
    intent: CcswitchLaunchIntent,
) -> Result<CcswitchLaunchReport, AppError> {
    if !cfg!(target_os = "windows") {
        return Ok(report_missing("当前平台不支持自动启动 Windows CCSwitch"));
    }

    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let resource_dir = app.path().resource_dir().ok();
    log::info!(
        "CCSwitch launch requested manager={} resource_dir={} candidates={:?}",
        executable.display(),
        resource_dir
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string()),
        source_candidates(&executable, resource_dir.as_deref())
            .iter()
            .map(|path| format!(
                "{}{}",
                path.display(),
                if is_regular_file(path) { " [file]" } else { "" }
            ))
            .collect::<Vec<_>>()
    );
    let Some(source) = find_verified_source(&executable, resource_dir.as_deref())? else {
        return Ok(report_missing(
            "未找到 ccswitch\\CodexSwitch.exe；Codex 已安装，但 CCSwitch 未自动启动",
        ));
    };
    log::info!("CCSwitch source selected path={}", source.display());
    log::info!("CCSwitch source hash verified path={}", source.display());
    let runtime =
        crate::app::runtime::ensure_bundled_vc_runtime(&executable, resource_dir.as_deref())?;
    if !runtime.ready {
        return Err(AppError::Engine(runtime.message));
    }
    log::info!(
        "CCSwitch VC runtime ready installed_now={} installer={}",
        runtime.installed_now,
        runtime
            .installer_path
            .as_deref()
            .unwrap_or("system runtime")
    );
    let usb_mode = crate::app::usb_guard::guard_required();
    let (target, working_dir, copied_files, installed, launch_mode) = if usb_mode {
        let target_dir = source
            .parent()
            .ok_or_else(|| AppError::Engine("无法确定 USB CCSwitch 工作目录".to_string()))?;
        (source.clone(), target_dir.to_path_buf(), 0, true, "usb")
    } else {
        let target_dir = local_install_dir()?;
        let copied_files = copy_payload(&source, &target_dir)?;
        (
            target_dir.join(CCSWITCH_EXECUTABLE),
            target_dir,
            copied_files,
            true,
            "local",
        )
    };
    // Read the previous ownership record before writing the current source.
    // Otherwise a drive-letter/path change would overwrite the only pointer
    // to the old USB process and leave that stale single-instance process
    // running indefinitely.
    let previous_managed_source = if usb_mode {
        read_usb_managed_source()
    } else {
        None
    };
    let background_was_hidden = usb_mode && usb_marker_background_hidden_for(&source);
    if usb_mode {
        // USB mode has no local executable directory in which to place the
        // normal ownership marker.  Persist a small, non-secret marker beside
        // the manager settings so the combined uninstaller can identify and
        // clean this product's CCSwitch profile on a later run.
        if let Err(error) = write_usb_managed_marker(&source, background_was_hidden) {
            log::warn!("CCSwitch launched from USB but ownership marker was not written: {error}");
        }
    }
    let inherited_webview2_arguments = std::env::var(WEBVIEW2_ARGUMENTS_ENV).ok();
    let webview2_arguments =
        webview2_software_rendering_arguments(inherited_webview2_arguments.as_deref());
    // Pass the signed binding to the USB CCSwitch child. A nonce-bound
    // acknowledgement is also required before automatic minimization, so a
    // launched process is never mistaken for a saved-and-selected provider.
    let account_binding = if usb_mode && intent == CcswitchLaunchIntent::Bootstrap {
        match crate::app::usb_guard::authorized_account_binding() {
            Ok(binding) => binding,
            Err(error) => {
                log::error!(
                    "CCSwitch account binding unavailable; launching without injection: {error}"
                );
                None
            }
        }
    } else {
        None
    };
    let account_environment = if usb_mode && intent == CcswitchLaunchIntent::Bootstrap {
        ccswitch_account_environment(account_binding.as_ref())
    } else {
        Vec::new()
    };
    let account_injected = !account_environment.is_empty();
    let handoff_request = if account_injected {
        account_environment
            .iter()
            .find(|(name, _)| *name == CCSWITCH_404TOKEN_DEVICE_KEY_ENV)
            .and_then(|(_, key)| match CcswitchHandoffRequest::new(key) {
                Ok(request) => Some(request),
                Err(error) => {
                    log::warn!(
                        "CCSwitch provider hand-off confirmation could not be prepared: {error}"
                    );
                    None
                }
            })
    } else {
        None
    };
    let mut injection_status = if !usb_mode || intent == CcswitchLaunchIntent::UserOpen {
        "not-attempted"
    } else if account_injected {
        if handoff_request.is_some() {
            "pending"
        } else {
            "unconfirmed"
        }
    } else {
        "missing-binding"
    };
    log::info!(
        "CCSwitch launch mode={} target={} account_injection_status={}",
        launch_mode,
        target.display(),
        injection_status
    );

    // An explicit user open should restore the existing managed process rather
    // than close/relaunch it. Relaunching here used to reapply the background
    // environment and hide the window every time the button was clicked.
    if intent == CcswitchLaunchIntent::UserOpen
        && codex_win_engine::processes_running_for_root(&working_dir).unwrap_or(false)
    {
        let shown_windows = codex_win_engine::show_windows_for_root(&working_dir)
            .map_err(|error| AppError::Engine(format!("无法恢复 CCSwitch 窗口：{error}")))?;
        let window = codex_win_engine::wait_for_usable_window_for_root(
            &working_dir,
            Duration::from_secs(10),
        )
        .map_err(|error| AppError::Engine(format!("检查 CCSwitch 窗口失败：{error}")))?
        .ok_or_else(|| AppError::Engine("CCSwitch 进程仍在运行，但没有可操作窗口".into()))?;
        log::info!(
            "CCSwitch existing process restored root={} shown_windows={} pid={}",
            working_dir.display(),
            shown_windows,
            window.pid
        );
        return Ok(CcswitchLaunchReport {
            available: true,
            installed,
            launched: true,
            source_path: Some(source.to_string_lossy().into_owned()),
            installed_path: Some(target.to_string_lossy().into_owned()),
            copied_files: 0,
            launch_mode: launch_mode.to_string(),
            data_scope: "user-profile".to_string(),
            usable_window_confirmed: true,
            graphics_mode: "software-rendering".to_string(),
            account_injected: false,
            account_injection_status: "not-attempted".to_string(),
            message: "Codex Switch 已恢复显示；后台进程保持运行".to_string(),
        });
    }

    // An earlier USB bundle may still own CCSwitch's single-instance window.
    // The ownership marker is exact and lets us close only that prior managed
    // root; unrelated standalone CCSwitch installations remain untouched.
    if let Some(previous_source) = previous_managed_source {
        if let Some(previous_root) = previous_source.parent() {
            if !same_windows_path(previous_root, &working_dir)
                && codex_win_engine::processes_running_for_root(previous_root).unwrap_or(false)
            {
                log::info!(
                    "closing previous USB CCSwitch root before relaunch root={}",
                    previous_root.display()
                );
                codex_win_engine::close_processes_for_root(10, previous_root).map_err(|error| {
                    AppError::Engine(format!("无法关闭旧版 USB CCSwitch 进程：{error}"))
                })?;
            }
        }
    }

    // A previous launch from this exact bundle can own CCSwitch's single
    // instance listener and keep the old process environment (and therefore
    // the old/missing API key). Close only processes physically loaded from
    // this managed root before starting a fresh child. Unrelated CCSwitch
    // installations are never touched.
    if intent == CcswitchLaunchIntent::Bootstrap
        && codex_win_engine::processes_running_for_root(&working_dir).unwrap_or(false)
    {
        codex_win_engine::close_processes_for_root(10, &working_dir).map_err(|error| {
            AppError::Engine(format!("无法关闭同一套件中的旧 CCSwitch 进程：{error}"))
        })?;
    }

    let mut command = Command::new(&target);
    command
        .current_dir(&working_dir)
        // This marker is informational for the product build and is ignored
        // by CCSwitch itself; it avoids inventing an unsupported CLI flag for
        // its data directory. Tauri/Windows continue to resolve app data from
        // the user's profile, not from the removable working directory.
        .env("CODEX_USB_CCSWITCH_MODE", launch_mode)
        // The bundled USB edition is a background configurator: its provider
        // bootstrap runs before the window becomes interactive, then the
        // manager hides the main window while leaving the tray process alive.
        .env(
            CCSWITCH_USB_BACKGROUND_ENV,
            if intent == CcswitchLaunchIntent::Bootstrap && account_injected {
                "1"
            } else {
                "0"
            },
        )
        // The target-machine report proved that this WebView2 renderer stalls
        // with hardware acceleration on Windows 10/Intel graphics. Keep the
        // workaround scoped to this child instead of changing a machine/user
        // environment variable used by unrelated WebView2 applications.
        .env(WEBVIEW2_ARGUMENTS_ENV, &webview2_arguments);
    let usb_codex_home = if usb_mode {
        Some(
            crate::app::usb_guard::local_data_paths()
                .map_err(|error| AppError::Engine(format!("无法确定 USB Codex 配置目录：{error}")))?
                .codex_home,
        )
    } else {
        None
    };
    for name in [
        CCSWITCH_404TOKEN_DEVICE_KEY_ENV,
        CCSWITCH_404TOKEN_ACCOUNT_ENV,
        CCSWITCH_404TOKEN_GROUP_ENV,
        CCSWITCH_404TOKEN_PROVIDER_ENV,
        CCSWITCH_USB_CODEX_HOME_ENV,
        CCSWITCH_404TOKEN_HANDOFF_PATH_ENV,
        CCSWITCH_404TOKEN_HANDOFF_NONCE_ENV,
    ] {
        // Clear inherited USB credentials and paths first. The USB Codex home
        // is re-added for managed USB launches; the device key remains limited
        // to a verified bootstrap below.
        command.env_remove(name);
    }
    if let Some(home) = &usb_codex_home {
        command.env(CCSWITCH_USB_CODEX_HOME_ENV, home);
    }
    for (name, value) in &account_environment {
        command.env(name, value);
    }
    if let Some(request) = &handoff_request {
        command
            .env(CCSWITCH_404TOKEN_HANDOFF_PATH_ENV, &request.path)
            .env(CCSWITCH_404TOKEN_HANDOFF_NONCE_ENV, &request.nonce);
    }
    let child = command.spawn().map_err(|error| {
        log::error!(
            "CCSwitch process spawn failed target={} error={error}",
            target.display()
        );
        io_error(&target, error)
    })?;
    // The USB guard records the exact child PID.  Unplug handling can then
    // terminate only this managed CCSwitch instance instead of using a global
    // `/IM CodexSwitch.exe` kill that might affect another installation.
    if let Err(error) =
        crate::app::usb_guard::register_process(child.id(), "ccswitch", Some(&target))
    {
        log::warn!("CCSwitch started but USB guard registration failed: {error}");
    }

    let window =
        codex_win_engine::wait_for_usable_window_for_root(&working_dir, Duration::from_secs(45))
            .map_err(|error| AppError::Engine(format!("检查 CCSwitch 窗口失败：{error}")))?;
    let Some(window) = window else {
        // Do not leave a headless single-instance process behind: the next
        // launch would merely signal that incompatible instance and could not
        // apply the child-scoped WebView2 compatibility environment.
        if codex_win_engine::processes_running_for_root(&working_dir).unwrap_or(false) {
            let _ = codex_win_engine::close_processes_for_root(5, &working_dir);
        }
        return Err(AppError::Engine(
            "CCSwitch 已启动但 45 秒内没有出现可操作窗口；可能有旧版/其他 CCSwitch 占用单实例 localhost 端口。已关闭本次隐藏进程，请先关闭所有 CCSwitch 后重试，并查看日志"
                .to_string(),
        ));
    };

    // As with Electron, an HWND can exist briefly before the WebView renderer
    // fatally exits. Do not turn that transient window into a false success.
    std::thread::sleep(Duration::from_secs(3));
    let stable_window =
        codex_win_engine::wait_for_usable_window_for_root(&working_dir, Duration::ZERO)
            .map_err(|error| AppError::Engine(format!("复核 CCSwitch 窗口失败：{error}")))?;
    let Some(stable_window) = stable_window else {
        if codex_win_engine::processes_running_for_root(&working_dir).unwrap_or(false) {
            let _ = codex_win_engine::close_processes_for_root(5, &working_dir);
        }
        return Err(AppError::Engine(
            "CCSwitch 窗口出现后未能稳定保留；可能有旧版/其他 CCSwitch 占用单实例 localhost 端口。已关闭本次异常进程，请先关闭所有 CCSwitch 后重试，并查看日志".to_string(),
        ));
    };

    if account_injected && intent == CcswitchLaunchIntent::Bootstrap {
        let outcome = handoff_request
            .as_ref()
            .map_or(CcswitchHandoffOutcome::Unconfirmed, |request| {
                request.wait_for_outcome(CCSWITCH_HANDOFF_ACK_TIMEOUT)
            });
        injection_status = match outcome {
            CcswitchHandoffOutcome::Verified => "verified",
            CcswitchHandoffOutcome::Failed => "failed",
            CcswitchHandoffOutcome::Unconfirmed => "unconfirmed",
        };
        log::info!(
            "CCSwitch provider hand-off result status={}",
            injection_status
        );
    }

    // Hide only after CCSwitch confirms that the matching credential is saved
    // and 404token is the active Codex provider. Explicit user opens remain
    // visible and are never subject to the bootstrap hand-off.
    let should_hide = intent == CcswitchLaunchIntent::Bootstrap
        && account_injected
        && injection_status == "verified";
    let hidden_windows = if should_hide {
        // WebView2 can hand the first top-level HWND to a renderer child a
        // fraction after the stable-window probe.  Retry the exact managed
        // process tree for a short bounded interval so the one-time bootstrap
        // hide is reliable without turning an explicit user open into a
        // background launch.
        let hide_deadline = Instant::now() + Duration::from_secs(3);
        let mut hidden_total = 0_usize;
        loop {
            let hidden =
                codex_win_engine::hide_windows_for_root(&working_dir).map_err(|error| {
                    AppError::Engine(format!("CCSwitch 已注入授权，但无法收至托盘：{error}"))
                })?;
            hidden_total = hidden_total.saturating_add(hidden);
            // Keep polling through the bounded window even after the first
            // HWND was hidden. WebView2 can create a renderer-owned top-level
            // HWND shortly after that first hide; stopping early would report
            // success while leaving the late window visible.
            if Instant::now() >= hide_deadline {
                break hidden_total;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    } else {
        0
    };
    let focused = stable_window;
    log::info!(
        "CCSwitch usable window confirmed pid={} size={}x{} initially_visible={} foreground_accepted={} hidden_windows={} account_injected={} webview2_args={}",
        focused.pid,
        focused.width,
        focused.height,
        window.was_visible,
        focused.foreground_accepted,
        hidden_windows,
        account_injected,
        webview2_arguments
    );

    if should_hide && hidden_windows > 0 && usb_mode {
        if let Err(error) = write_usb_managed_marker(&source, true) {
            log::warn!("CCSwitch background hand-off completed but marker update failed: {error}");
        }
    }

    Ok(CcswitchLaunchReport {
        available: true,
        installed,
        launched: true,
        source_path: Some(source.to_string_lossy().into_owned()),
        installed_path: Some(target.to_string_lossy().into_owned()),
        copied_files,
        launch_mode: launch_mode.to_string(),
        data_scope: "user-profile".to_string(),
        usable_window_confirmed: true,
        graphics_mode: "software-rendering".to_string(),
        account_injected,
        account_injection_status: injection_status.to_string(),
        message: if usb_mode {
            if injection_status == "verified" && should_hide {
                "Codex Switch 已从 U 盘启动，404token 授权已提交，已收至系统托盘后台运行"
                    .to_string()
            } else if injection_status == "verified" {
                "Codex Switch 已从 U 盘启动，404token 已保存并设为当前 Provider，配置窗口已显示"
                    .to_string()
            } else if injection_status == "failed" {
                "Codex Switch 已启动，但未能保存并启用 404token Provider；配置窗口保持打开，请检查配置后重试".to_string()
            } else if account_injected {
                "404token API Key 已传给 CCSwitch，但未确认其已保存并设为当前 Provider；配置窗口保持打开，请检查 CCSwitch".to_string()
            } else if intent == CcswitchLaunchIntent::UserOpen {
                "Codex Switch 已从 U 盘启动或恢复显示，配置窗口已打开".to_string()
            } else if injection_status == "missing-binding" {
                "Codex Switch 已启动，但未读取到可用的签名 404token 账户绑定；尚未确认 Codex 登录配置，配置窗口保持打开，请检查账户绑定并重新签名".to_string()
            } else {
                "Codex Switch 已启动，但未确认 404token API Key 已保存并启用；尚未确认 Codex 登录配置，配置窗口保持打开，请重试或手动配置".to_string()
            }
        } else {
            "Codex Switch 已复制到本机、启动并显示配置窗口，请输入 API Key 完成配置".to_string()
        },
    })
}

/// Automatic first-install path. It keeps the historical bootstrap behavior
/// but records that the one-time tray hand-off already happened.
pub fn install_and_launch(app: &AppHandle) -> Result<CcswitchLaunchReport, AppError> {
    launch_with_intent(app, CcswitchLaunchIntent::Bootstrap)
}

/// Explicit user-facing open path. It restores an existing managed process and
/// only starts a new visible process when none is running.
pub fn open(app: &AppHandle) -> Result<CcswitchLaunchReport, AppError> {
    launch_with_intent(app, CcswitchLaunchIntent::UserOpen)
}

/// Resolve only the signed USB product binary for a finite configuration RPC.
/// This does not start/stop the normal GUI or touch another CCSwitch install.
pub(crate) fn verified_usb_selection_executable() -> Result<PathBuf, AppError> {
    crate::app::usb_guard::require_usb_content_access("ccswitch-model-selection")?;
    let status = crate::app::usb_guard::current_status();
    if !status.signed || !status.codex_allowed {
        return Err(AppError::Engine(
            "USB model selection is not authorized".into(),
        ));
    }
    let bundle = status
        .bundle_path
        .ok_or_else(|| AppError::Engine("USB bundle is unavailable".into()))?;
    let anchor = Path::new(&bundle).join("CodexAppManager.exe");
    let executable = find_verified_source(&anchor, None)?
        .ok_or_else(|| AppError::Engine("Bundled CCSwitch is unavailable".into()))?;
    Ok(executable)
}

pub(crate) fn selection_environment(
    binding: &codex_usb_license::UsbAccountBinding,
) -> Vec<(&'static str, String)> {
    ccswitch_account_environment(Some(binding))
}

#[cfg(test)]
mod tests {
    use super::{
        autostart_targets, ccswitch_account_environment, find_verified_source,
        local_install_is_managed, source_candidates, verify_source_executable,
        webview2_software_rendering_arguments, CcswitchHandoffOutcome, CcswitchHandoffRequest,
        CCSWITCH_404TOKEN_HANDOFF_PROTOCOL, CCSWITCH_404TOKEN_PROVIDER_ID, CCSWITCH_EXECUTABLE,
        CCSWITCH_MANAGED_MARKER, CCSWITCH_SOURCE_DIR,
    };
    use codex_usb_license::{UsbAccountBinding, UsbCcswitchBinding};
    use std::fs;
    use std::path::PathBuf;

    fn fixture_temp_dir(name: &str) -> PathBuf {
        let temp = std::env::temp_dir();
        fs::canonicalize(&temp).unwrap_or(temp).join(name)
    }

    #[test]
    fn sibling_usb_payload_is_checked_before_embedded_resources() {
        let usb_root = PathBuf::from("usb-volume").join("usb-oneclick");
        let resources = PathBuf::from("installed-manager").join("resources");
        let candidates =
            source_candidates(&usb_root.join("CodexDesktopSetup.exe"), Some(&resources));
        assert_eq!(
            candidates[0],
            usb_root.join(CCSWITCH_SOURCE_DIR).join(CCSWITCH_EXECUTABLE)
        );
        assert!(candidates.iter().any(|candidate| {
            candidate
                == &resources
                    .join(CCSWITCH_SOURCE_DIR)
                    .join(CCSWITCH_EXECUTABLE)
        }));
    }

    #[test]
    fn account_binding_is_reduced_to_child_environment_contract() {
        let binding = UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "USB-0001".into(),
            account_ref: "USB-0001".into(),
            account_name: "USB-0001".into(),
            login_email: "u1@u.404token.xyz".into(),
            sub2api_user_id: 1,
            ccswitch: UsbCcswitchBinding {
                provider_id: "404token-sub2api".into(),
                credential_id: "key-1".into(),
                device_api_key: "sk-secret".into(),
                group_id: 7,
            },
        };
        let environment = ccswitch_account_environment(Some(&binding));
        assert_eq!(environment.len(), 4);
        assert!(environment.iter().any(|(name, value)| {
            *name == "CCSWITCH_404TOKEN_DEVICE_KEY" && value == "sk-secret"
        }));
        assert!(environment
            .iter()
            .any(|(name, value)| { *name == "CCSWITCH_404TOKEN_ACCOUNT" && value == "USB-0001" }));
    }

    #[test]
    fn ccswitch_handoff_ack_requires_matching_nonce_provider_and_key_fingerprint() {
        let request = CcswitchHandoffRequest::new("fixture-device-key").unwrap();
        let verified_ack = serde_json::json!({
            "protocol": CCSWITCH_404TOKEN_HANDOFF_PROTOCOL,
            "nonce": request.nonce,
            "status": "verified",
            "providerId": CCSWITCH_404TOKEN_PROVIDER_ID,
            "credentialSha256": request.credential_sha256,
        });
        fs::write(&request.path, serde_json::to_vec(&verified_ack).unwrap()).unwrap();
        assert_eq!(
            request.outcome_from_file(),
            Some(CcswitchHandoffOutcome::Verified)
        );

        let mut mismatched_key = verified_ack.clone();
        mismatched_key["credentialSha256"] = serde_json::Value::String("0".repeat(64));
        fs::write(&request.path, serde_json::to_vec(&mismatched_key).unwrap()).unwrap();
        assert_eq!(
            request.outcome_from_file(),
            Some(CcswitchHandoffOutcome::Unconfirmed)
        );

        let mut failed_ack = verified_ack;
        failed_ack["status"] = serde_json::Value::String("failed".to_string());
        failed_ack["credentialSha256"] = serde_json::Value::Null;
        fs::write(&request.path, serde_json::to_vec(&failed_ack).unwrap()).unwrap();
        assert_eq!(
            request.outcome_from_file(),
            Some(CcswitchHandoffOutcome::Failed)
        );
    }

    #[test]
    fn ccswitch_handoff_ack_missing_or_malformed_is_not_verified() {
        let request = CcswitchHandoffRequest::new("fixture-device-key").unwrap();
        assert_eq!(request.outcome_from_file(), None);
        fs::write(&request.path, b"not-json").unwrap();
        assert_eq!(
            request.outcome_from_file(),
            Some(CcswitchHandoffOutcome::Unconfirmed)
        );
    }

    #[test]
    fn invalid_or_missing_binding_does_not_inject_environment() {
        assert!(ccswitch_account_environment(None).is_empty());
        let binding = UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "USB-0001".into(),
            account_ref: "USB-0001".into(),
            account_name: "USB-0001".into(),
            login_email: "u1@u.404token.xyz".into(),
            sub2api_user_id: 1,
            ccswitch: UsbCcswitchBinding {
                provider_id: "other".into(),
                credential_id: "key-1".into(),
                device_api_key: "sk-secret".into(),
                group_id: 7,
            },
        };
        assert!(ccswitch_account_environment(Some(&binding)).is_empty());
    }

    #[test]
    fn webview2_software_rendering_is_scoped_and_idempotent() {
        assert_eq!(webview2_software_rendering_arguments(None), "--disable-gpu");
        assert_eq!(
            webview2_software_rendering_arguments(Some("--enable-logging")),
            "--enable-logging --disable-gpu"
        );
        assert_eq!(
            webview2_software_rendering_arguments(Some("--DISABLE-GPU --enable-logging")),
            "--DISABLE-GPU --enable-logging"
        );
    }

    #[test]
    fn source_manifest_hash_gate_rejects_tampered_executable() {
        let dir = fixture_temp_dir(&format!("codex-ccswitch-hash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let executable = dir.join(CCSWITCH_EXECUTABLE);
        fs::write(&executable, b"known payload").unwrap();
        let hash = super::sha256_file(&executable).unwrap();
        let manifest = serde_json::json!({ "artifact": { "sha256": hash } }).to_string();
        let mut manifest_with_utf8_bom = vec![0xEF, 0xBB, 0xBF];
        manifest_with_utf8_bom.extend_from_slice(manifest.as_bytes());
        fs::write(dir.join("manifest.json"), manifest_with_utf8_bom).unwrap();
        assert!(verify_source_executable(&executable).is_ok());
        fs::write(&executable, b"tampered payload").unwrap();
        assert!(verify_source_executable(&executable).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verified_source_falls_back_to_a_valid_resource_copy() {
        let dir = fixture_temp_dir(&format!(
            "codex-ccswitch-source-fallback-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let usb_root = dir.join("usb");
        let primary_dir = usb_root.join(CCSWITCH_SOURCE_DIR);
        let resources_dir = usb_root.join("resources");
        let fallback_dir = resources_dir.join(CCSWITCH_SOURCE_DIR);
        fs::create_dir_all(&primary_dir).unwrap();
        fs::create_dir_all(&fallback_dir).unwrap();

        let primary_executable = primary_dir.join(CCSWITCH_EXECUTABLE);
        fs::write(&primary_executable, b"stale USB CCSwitch").unwrap();
        fs::write(
            primary_dir.join("manifest.json"),
            serde_json::json!({
                "artifact": { "sha256": super::sha256_file(&primary_executable).unwrap() }
            })
            .to_string(),
        )
        .unwrap();
        // Simulate the exact reported failure: the preferred USB copy exists,
        // but its adjacent manifest was generated for different executable
        // bytes. A valid packaged resource copy must still be selected.
        fs::write(&primary_executable, b"USB CCSwitch replaced after manifest").unwrap();

        let fallback_executable = fallback_dir.join(CCSWITCH_EXECUTABLE);
        fs::write(&fallback_executable, b"known-good packaged CCSwitch").unwrap();
        fs::write(
            fallback_dir.join("manifest.json"),
            serde_json::json!({
                "artifact": { "sha256": super::sha256_file(&fallback_executable).unwrap() }
            })
            .to_string(),
        )
        .unwrap();

        let anchor = usb_root.join("CodexAppManager.exe");
        let selected = find_verified_source(&anchor, Some(&resources_dir))
            .unwrap()
            .expect("the valid packaged resource copy should be selected");
        assert_eq!(selected, fallback_executable);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_ccswitch_cleanup_requires_a_valid_ownership_marker() {
        let dir = fixture_temp_dir(&format!(
            "codex-ccswitch-marker-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(!local_install_is_managed(&dir));

        fs::write(
            dir.join(CCSWITCH_MANAGED_MARKER),
            serde_json::json!({
                "product": "codex-usb-oneclick",
                "component": "ccswitch",
                "executable": CCSWITCH_EXECUTABLE,
            })
            .to_string(),
        )
        .unwrap();
        assert!(local_install_is_managed(&dir));

        fs::write(
            dir.join(CCSWITCH_MANAGED_MARKER),
            serde_json::json!({
                "product": "other-installer",
                "component": "ccswitch",
                "executable": CCSWITCH_EXECUTABLE,
            })
            .to_string(),
        )
        .unwrap();
        assert!(!local_install_is_managed(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn autostart_cleanup_matches_executable_not_substring() {
        let path = r"C:\Program Files\CodexSwitch\CodexSwitch.exe";
        assert!(autostart_targets(&format!("\"{path}\" --minimized"), path));
        assert!(!autostart_targets(&format!("\"{path}.other.exe\""), path));
        assert!(!autostart_targets(&format!("cmd.exe /c \"{path}\""), path));
        assert!(!autostart_targets(&format!("\"{path}\"injected"), path));
    }

    #[test]
    fn removal_postcondition_requires_not_found_not_probe_failure() {
        use std::io::{Error, ErrorKind};

        assert!(super::existence_from_probe(Ok(())).unwrap());
        assert!(!super::existence_from_probe(Err(Error::from(ErrorKind::NotFound))).unwrap());
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::Interrupted,
            ErrorKind::Other,
        ] {
            let error = super::existence_from_probe(Err(Error::from(kind))).unwrap_err();
            assert_eq!(error.kind(), kind);
        }
    }

    #[test]
    fn unowned_external_install_is_preserved_without_failing_our_uninstall() {
        assert!(super::local_install_cleanup_settled(false, false));
        assert!(super::local_install_cleanup_settled(true, false));
        assert!(super::local_install_cleanup_settled(false, true));
        assert!(!super::local_install_cleanup_settled(true, true));
    }
}
