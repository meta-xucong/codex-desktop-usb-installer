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

// The public non-Windows stubs keep cross-platform manager builds possible;
// the lease helper, process matching and removable-media scanner run on Windows.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(target_os = "windows")]
use std::fs::OpenOptions;
use std::fs::{self, File};
#[cfg(target_os = "windows")]
use std::io::Write;
use std::io::{self, Read};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::atomic_file;
use crate::errors::AppError;

use codex_usb_license as license_core;
pub use license_core::{
    UsbFingerprint, UsbLicense, ANCHOR_SCHEMA_V1, ANCHOR_SCHEMA_V2, ANCHOR_SCHEMA_V3,
    LICENSE_SCHEMA_VERSION, PRODUCT_ID,
};

pub const GUARD_ARG: &str = "--codex-usb-guard";
pub const GUARD_EXE_NAME: &str = "CodexUsbGuard.exe";
pub const GUARD_DIR_NAME: &str = "CodexUsbGuard";
pub const USB_APPS_DIR_NAME: &str = "apps";
pub const USB_CODEX_DIR_NAME: &str = "Codex";
pub const USB_CONTENT_DIR_NAME: &str = "content";
pub const LICENSE_FILE_NAME: &str = "usb-license.json";
pub const LEASE_FILE_NAME: &str = "lease.json";
pub const STATUS_FILE_NAME: &str = "status.json";
pub const LOCK_FILE_NAME: &str = "guard.lock";
pub const POLL_INTERVAL_MS: u64 = 1_000;
pub const DISCONNECT_GRACE_MS: u64 = 5_000;
const LEASE_SCHEMA_VERSION: u32 = 1;
const STATUS_SCHEMA_VERSION: u32 = 1;
const USB_MANIFEST_POLICY_MAX_BYTES: u64 = 256 * 1024;
const GUARD_READY_TIMEOUT_MS: u64 = 5_000;
const AUTHORIZATION_RETRY_TIMEOUT_MS: u64 = 3_000;
const AUTHORIZATION_RETRY_INTERVAL_MS: u64 = 100;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[used]
static COMPILED_USB_POLICY_MARKER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/codex-usb-policy-marker.txt"));
static SUPPRESS_STATUS_AFTER_UNINSTALL: AtomicBool = AtomicBool::new(false);
/// Formal USB bundles do a complete SHA/AuthentiCode verification after the
/// local window exists.  Keep this fail-closed marker visible to read-only
/// status calls so a stale authorized status cannot enable an operation while
/// the first full scan is still running.
static STARTUP_INTEGRITY_PENDING: AtomicBool = AtomicBool::new(false);

/// Called only after the detached cleanup hand-off is accepted. Normal exit
/// callbacks must not recreate the very directory that the helper removes.
pub fn suppress_status_for_uninstall() {
    SUPPRESS_STATUS_AFTER_UNINSTALL.store(true, Ordering::SeqCst);
    remove_lease();
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
    /// Session snapshot fields prevent a later valid-looking replacement of
    /// the mutable license from being accepted by an already running manager.
    /// They are optional only so an older lease can be read and then replaced;
    /// formal signed gates reject a lease that lacks them.
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub license_digest: Option<String>,
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
    /// missing / unsigned / valid / invalid / unknown-key / revoked
    #[serde(default = "default_signature_state")]
    pub signature_state: String,
    /// Whether Codex operations are currently permitted by the signed gate.
    /// The default is false for status files written by older managers.
    #[serde(default)]
    pub codex_allowed: bool,
    #[serde(default)]
    pub codex_block_reason: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedUsbAuthorization {
    pub bundle_root: PathBuf,
    pub codex_root: PathBuf,
    pub fingerprint: UsbFingerprint,
    pub installation_id: String,
    pub key_id: String,
    pub license_digest: String,
    pub checked_at_ms: u64,
}

/// Startup may continue into a restricted manager UI when the immutable USB
/// bundle is authentic but its mutable authorization metadata is missing or
/// invalid. The normal path first exposes a local read-only shell while the
/// expensive immutable scan is pending; no Codex operation, prerequisite,
/// recovery, or shortcut write is allowed in that state. Structural/media
/// failures still return `Err` and abort before Tauri side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupPreflight {
    Ready,
    IntegrityPending { reason: String },
    AuthorizationBlocked { reason: String },
}

impl StartupPreflight {
    pub fn is_authorization_blocked(&self) -> bool {
        matches!(self, Self::AuthorizationBlocked { .. })
    }

    pub fn is_integrity_pending(&self) -> bool {
        matches!(self, Self::IntegrityPending { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::IntegrityPending { reason } | Self::AuthorizationBlocked { reason } => {
                Some(reason)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupGuardPlan {
    establish_lease: bool,
    prepare_or_start_helper: bool,
    clear_current_lease: bool,
    preserve_current_manager: bool,
    codex_allowed: bool,
}

fn startup_guard_plan(policy_signed_required: bool, license_is_signed: bool) -> StartupGuardPlan {
    if policy_signed_required && !license_is_signed {
        StartupGuardPlan {
            establish_lease: false,
            prepare_or_start_helper: false,
            clear_current_lease: true,
            preserve_current_manager: true,
            codex_allowed: false,
        }
    } else {
        StartupGuardPlan {
            establish_lease: true,
            prepare_or_start_helper: true,
            clear_current_lease: false,
            preserve_current_manager: false,
            codex_allowed: license_is_signed,
        }
    }
}

fn default_signature_state() -> String {
    "unknown".to_string()
}

/// Local data paths used by the USB portable edition. Program and managed
/// content live on the removable bundle; account, session, cache and Electron
/// profile data remain in the user's ordinary Windows profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbLocalDataPaths {
    pub codex_home: PathBuf,
    pub roaming_app_data: PathBuf,
    pub local_app_data: PathBuf,
}

fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(format!("USB 看门狗：{}", message.into()))
}

const COMPILETIME_PUBLIC_KEY_B64: Option<&str> = option_env!("CODEX_USB_GUARD_PUBLIC_KEY_B64");
const COMPILETIME_PUBLIC_KEY_ID: Option<&str> = option_env!("CODEX_USB_GUARD_PUBLIC_KEY_ID");

/// Formal release policy is compile-time only.  An environment variable may
/// select the same policy for a locally reproduced build, but never weakens a
/// binary compiled with signed-required.
pub fn signed_required() -> bool {
    cfg!(feature = "signed-required")
        || option_env!("CODEX_USB_GUARD_POLICY").is_some_and(|value| value == "SignedRequired")
}

pub fn policy_name() -> &'static str {
    if signed_required() {
        "SignedRequired"
    } else {
        "DevelopmentAutoEnroll"
    }
}

/// The release builder writes the compile-time USB policy into the immutable
/// usb-manifest.  Keep this check separate from the cryptographic bundle
/// verifier so a development manager can never auto-enrol a license into a
/// formal Bundle merely because it was copied beside that Bundle.
fn validate_bundle_policy_binding(bundle_root: &Path) -> Result<(), AppError> {
    let path = bundle_root.join("usb-manifest.json");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| engine(format!("读取 USB 联合清单失败：{error}")))?;
    if !metadata.is_file()
        || path_has_reparse_point(&path)?
        || metadata.len() > USB_MANIFEST_POLICY_MAX_BYTES
    {
        return Err(engine("USB 联合清单不是可核验的普通文件"));
    }
    let bytes =
        fs::read(&path).map_err(|error| engine(format!("读取 USB 联合清单失败：{error}")))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| engine(format!("USB 联合清单 JSON 无效：{error}")))?;
    let Some(declared) = manifest
        .pointer("/usbGuard/signing/policy")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(engine(
            "usb_policy_missing：USB 联合清单缺少 usbGuard.signing.policy，拒绝自动注册或授权",
        ));
    };
    if !matches!(declared, "SignedRequired" | "DevelopmentAutoEnroll") {
        return Err(engine(format!(
            "usb_policy_invalid：USB 联合清单声明了未知策略：{declared}"
        )));
    }
    let expected = policy_name();
    if declared != expected {
        return Err(engine(format!(
            "usb_policy_mismatch：管理器策略 {expected} 与 USB 联合清单策略 {declared} 不一致；已禁止自动注册和受保护操作"
        )));
    }
    Ok(())
}

fn verify_runtime_bundle_integrity(
    bundle_root: &Path,
) -> Result<crate::app::usb_bundle::BundleIntegrityReport, AppError> {
    if signed_required() {
        crate::app::usb_bundle::verify_signed_internal_bundle(bundle_root)
    } else {
        crate::app::usb_bundle::verify_bundle_integrity(bundle_root, false)
    }
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
    // `std::fs::canonicalize` returns verbatim DOS paths on Windows. Treat
    // `\\?\C:\...` / `\\.\C:\...` as the same local volume, while still
    // rejecting verbatim UNC and volume-GUID namespaces.
    let raw = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\\.\"))
        .unwrap_or(&raw);
    let bytes = raw.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    Some(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()))
}

/// Normalize an absolute local DOS path without using its text representation
/// as an authorization identity.  Existing paths are canonicalized by the
/// caller first; this final lexical pass removes verbatim prefixes, repeated
/// separators and `.` while rejecting drive-relative paths and `..` escapes.
fn normalize_bundle_path(input: &str) -> Option<String> {
    let raw = input.trim().replace('/', "\\");
    let raw = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\\.\"))
        .unwrap_or(&raw);
    let root = normalize_drive_root(raw)?;
    let mut parts = Vec::new();
    for part in raw[3..].split('\\') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\0') {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        Some(root)
    } else {
        Some(format!("{root}{}", parts.join("\\")))
    }
}

/// Build the bundle path found on a scanned drive from validated components.
/// `drive_root` already owns the one separator after `X:`; the relative path
/// must never add another one by raw string concatenation.
fn bundle_path_on_drive(drive_root: &str, relative: &str) -> Option<String> {
    let root = normalize_drive_root(drive_root)?;
    let candidate = format!("{root}{}", relative.replace('/', "\\"));
    normalize_bundle_path(&candidate)
}

fn observed_drive_root(path: &Path) -> Result<String, AppError> {
    // The final leaf (notably `~/.codex`) may not exist on a clean machine.
    // Resolve the nearest existing ancestor so a redirected/junction-backed
    // profile cannot masquerade as a C: path while physically landing on USB.
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        match fs::canonicalize(current) {
            Ok(resolved) => {
                return normalize_drive_root(&resolved.to_string_lossy()).ok_or_else(|| {
                    engine(format!(
                        "本机数据目录解析后不是本地 Windows 盘路径：{} -> {}",
                        path.display(),
                        resolved.display()
                    ))
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                candidate = current.parent();
            }
            Err(error) => {
                return Err(engine(format!(
                    "无法核验本机数据目录的真实位置 {}：{error}",
                    path.display()
                )));
            }
        }
    }
    Err(engine(format!(
        "本机数据目录没有可核验的现有父目录：{}",
        path.display()
    )))
}

/// Return a lower-case, slash-separated path below a volume root.  It is
/// intentionally independent of the host OS so the identity remains stable
/// in tests running on Linux/macOS.
pub fn bundle_relative_path(bundle_path: &str, drive_root: &str) -> Option<String> {
    let path = normalize_bundle_path(bundle_path)?;
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
    if relative.contains(&"..") {
        return None;
    }
    Some(relative.join("/").to_ascii_lowercase())
}

pub fn is_supported_anchor_schema(anchor_schema: u32) -> bool {
    license_core::is_supported_anchor_schema(anchor_schema)
}

pub fn fingerprints_match(expected: &UsbFingerprint, actual: &UsbFingerprint) -> bool {
    license_core::fingerprints_match(expected, actual)
}

fn fingerprint_excerpt(value: &str) -> String {
    value.chars().take(12).collect()
}

/// Validate the current physical observation against the manager-owned lease.
/// Full paths are deliberately diagnostic-only: Windows can represent one
/// object as `D:\foo`, `D:\\foo`, `\\?\D:\foo`, different case, or an 8.3
/// spelling.  The stable volume serial + bundle-relative path + anchor digest
/// is the authorization identity.
fn validate_lease_observation(
    lease: &UsbLease,
    observation: &UsbObservation,
    owner_pid: u32,
) -> Result<(), AppError> {
    if lease.schema_version != LEASE_SCHEMA_VERSION || lease.product != PRODUCT_ID {
        return Err(engine("U 盘租约版本或产品标识不匹配"));
    }
    if lease.owner_pid != owner_pid {
        return Err(engine(format!(
            "U 盘租约不属于当前安装器进程（leasePid={} currentPid={owner_pid}）",
            lease.owner_pid
        )));
    }
    let actual = observation
        .fingerprint
        .as_ref()
        .ok_or_else(|| engine("当前安装器目录不在可移动 U 盘上"))?;
    if !is_supported_anchor_schema(lease.fingerprint.anchor_schema)
        || !is_supported_anchor_schema(actual.anchor_schema)
    {
        return Err(engine("租约锚点版本不兼容"));
    }
    if !fingerprints_match(&lease.fingerprint, actual) {
        return Err(engine(format!(
            "当前安装器与租约的 U 盘指纹不一致（currentPath={} leasePath={} currentSerial={} leaseSerial={} currentRelative={} leaseRelative={} currentAnchorSchema={} leaseAnchorSchema={} currentAnchor={} leaseAnchor={}）",
            observation.bundle_path,
            lease.bundle_path,
            actual.volume_serial,
            lease.fingerprint.volume_serial,
            actual.bundle_relative_path,
            lease.fingerprint.bundle_relative_path,
            actual.anchor_schema,
            lease.fingerprint.anchor_schema,
            fingerprint_excerpt(&actual.anchor_sha256),
            fingerprint_excerpt(&lease.fingerprint.anchor_sha256),
        )));
    }
    Ok(())
}

fn validate_lease_license_snapshot(
    lease: &UsbLease,
    license: &UsbLicense,
    bundle: &Path,
) -> Result<(), AppError> {
    if let Some(expected) = lease.installation_id.as_deref() {
        if expected != license.installation_id {
            return Err(engine(
                "usb_toctou_changed：当前许可证 installationId 与本会话快照不一致",
            ));
        }
    }
    if let Some(expected) = lease.key_id.as_deref() {
        if Some(expected) != license.key_id.as_deref() {
            return Err(engine(
                "usb_toctou_changed：当前许可证 keyId 与本会话快照不一致",
            ));
        }
    }
    if let Some(expected) = lease.license_digest.as_deref() {
        let actual = license_digest(bundle)?;
        if !expected.eq_ignore_ascii_case(&actual) {
            return Err(engine(
                "usb_toctou_changed：当前许可证摘要与本会话快照不一致",
            ));
        }
    }
    Ok(())
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

fn map_license_error(error: license_core::LicenseError) -> AppError {
    match error {
        license_core::LicenseError::FingerprintMismatch => engine("许可证：指纹不匹配"),
        license_core::LicenseError::InvalidProduct { .. } => engine("许可证：产品标识不匹配"),
        error => engine(format!("许可证：{error}")),
    }
}

/// These adapters deliberately delegate serialization, validation and
/// cryptography to the shared core crate. The manager owns only policy and
/// Windows observation; it must not grow a second signing format.
pub fn unsigned_license_payload(license: &UsbLicense) -> Result<Vec<u8>, AppError> {
    license_core::unsigned_license_payload(license).map_err(map_license_error)
}

pub fn legacy_unsigned_license_payload(license: &UsbLicense) -> Result<Vec<u8>, AppError> {
    license_core::legacy_unsigned_license_payload(license).map_err(map_license_error)
}

pub fn verify_signature_with_key(
    license: &UsbLicense,
    public_key_b64: &str,
) -> Result<(), AppError> {
    license_core::verify_signature_with_key(license, public_key_b64).map_err(map_license_error)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfiguredUsbKey {
    key_id: String,
    public_key_b64: String,
    #[serde(default)]
    revoked: bool,
}

fn decode_configured_public_key(value: &str, label: &str) -> Result<Vec<u8>, AppError> {
    let decoded = BASE64
        .decode(value.trim())
        .map_err(|error| engine(format!("{label} Base64 无效: {error}")))?;
    if decoded.len() != 32 {
        return Err(engine(format!("{label} 必须是 32 字节 Ed25519 公钥")));
    }
    Ok(decoded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveKeyBinding {
    Matched,
    MissingActiveKeyId,
    MissingLicenseKeyId,
    InvalidActiveKeyId,
    InvalidLicenseKeyId,
    Mismatch,
}

fn active_key_binding_state(
    active_key_id: Option<&str>,
    license_key_id: Option<&str>,
) -> ActiveKeyBinding {
    let Some(active_key_id) = active_key_id else {
        return ActiveKeyBinding::MissingActiveKeyId;
    };
    if license_core::validate_key_id(active_key_id).is_err() {
        return ActiveKeyBinding::InvalidActiveKeyId;
    }
    let Some(license_key_id) = license_key_id else {
        return ActiveKeyBinding::MissingLicenseKeyId;
    };
    if license_core::validate_key_id(license_key_id).is_err() {
        return ActiveKeyBinding::InvalidLicenseKeyId;
    }
    if active_key_id == license_key_id {
        ActiveKeyBinding::Matched
    } else {
        ActiveKeyBinding::Mismatch
    }
}

fn require_active_key_binding<'a>(
    active_key_id: Option<&'a str>,
    license_key_id: Option<&str>,
) -> Result<&'a str, AppError> {
    match active_key_binding_state(active_key_id, license_key_id) {
        ActiveKeyBinding::Matched => Ok(active_key_id.expect("matched key binding has active id")),
        ActiveKeyBinding::MissingActiveKeyId => Err(engine(
            "签名许可证验证缺少编译期 CODEX_USB_GUARD_PUBLIC_KEY_ID",
        )),
        ActiveKeyBinding::MissingLicenseKeyId => Err(engine("正式签名许可证缺少 keyId")),
        ActiveKeyBinding::InvalidActiveKeyId => Err(engine("编译期 active keyId 不是合法 keyId")),
        ActiveKeyBinding::InvalidLicenseKeyId => Err(engine("正式签名许可证包含非法 keyId")),
        ActiveKeyBinding::Mismatch => Err(engine(format!(
            "USB 许可证 keyId 与当前编译期 active keyId 不一致: {}",
            license_key_id.unwrap_or_default()
        ))),
    }
}

fn configured_key_for(license: &UsbLicense) -> Result<Option<String>, AppError> {
    if license.signature.is_none() {
        return Ok(None);
    }

    let active_key_id =
        require_active_key_binding(COMPILETIME_PUBLIC_KEY_ID, license.key_id.as_deref())?;
    let compiletime_public_key = COMPILETIME_PUBLIC_KEY_B64;
    let compiletime_public_key_bytes = compiletime_public_key
        .map(|value| decode_configured_public_key(value, "编译期 USB 公钥"))
        .transpose()?;

    if let Some(raw_keyring) = option_env!("CODEX_USB_GUARD_KEYRING_JSON") {
        let keyring = serde_json::from_str::<Vec<ConfiguredUsbKey>>(raw_keyring)
            .map_err(|error| engine(format!("构建内置 USB 公钥环无效: {error}")))?;
        let key = keyring
            .into_iter()
            .find(|entry| entry.key_id == active_key_id)
            .ok_or_else(|| {
                engine(format!(
                    "USB 许可证 active keyId 未被当前构建公钥环配置: {active_key_id}"
                ))
            })?;
        if key.revoked {
            return Err(engine(format!(
                "USB 许可证 active keyId 已撤销: {active_key_id}"
            )));
        }
        let keyring_public_key_bytes =
            decode_configured_public_key(&key.public_key_b64, "USB 公钥环 active 公钥")?;
        if let Some(compiletime_public_key_bytes) = &compiletime_public_key_bytes {
            if keyring_public_key_bytes != *compiletime_public_key_bytes {
                return Err(engine(format!(
                    "USB 公钥环 active keyId 与编译期 USB 公钥不一致: {active_key_id}"
                )));
            }
        }
        return Ok(Some(key.public_key_b64.trim().to_string()));
    }
    let Some(public_key) = compiletime_public_key else {
        return Err(engine(
            "许可证带签名，但当前构建没有配置 USB 公钥；请使用 SignedRequired 配置构建",
        ));
    };
    let _ = compiletime_public_key_bytes
        .as_ref()
        .ok_or_else(|| engine("许可证带签名，但构建公钥为空"))?;
    Ok(Some(public_key.trim().to_string()))
}

fn verify_configured_signature(license: &UsbLicense) -> Result<bool, AppError> {
    let Some(public_key) = configured_key_for(license)? else {
        return Ok(false);
    };
    verify_signature_with_key(license, &public_key)?;
    Ok(true)
}

pub fn validate_license(license: &UsbLicense, expected: &UsbFingerprint) -> Result<bool, AppError> {
    let configured_key = configured_key_for(license)?;
    license_core::validate_license(license, expected, configured_key.as_deref())
        .map_err(map_license_error)?;
    if license.signature.is_some() {
        verify_configured_signature(license)
    } else {
        Ok(false)
    }
}

pub fn new_unsigned_license(fingerprint: UsbFingerprint) -> UsbLicense {
    license_core::new_unsigned_license(fingerprint, now_ms())
}

pub fn raw_anchor_sha256(bundle_root: &Path) -> Result<String, AppError> {
    license_core::raw_anchor_sha256(bundle_root).map_err(map_license_error)
}

pub fn canonical_anchor_sha256(bundle_root: &Path) -> Result<String, AppError> {
    license_core::canonical_anchor_sha256(bundle_root).map_err(map_license_error)
}

pub fn stable_anchor_sha256_from_bytes(
    manifest_bytes: &[u8],
    usb_manifest_bytes: &[u8],
    checksum_bytes: &[u8],
) -> Result<String, AppError> {
    license_core::stable_anchor_sha256_from_bytes(
        manifest_bytes,
        usb_manifest_bytes,
        checksum_bytes,
    )
    .map_err(map_license_error)
}

pub fn bundle_anchor_schema(bundle_root: &Path) -> Result<u32, AppError> {
    license_core::bundle_anchor_schema(bundle_root).map_err(map_license_error)
}

pub fn anchor_sha256(bundle_root: &Path) -> Result<String, AppError> {
    license_core::anchor_sha256(bundle_root).map_err(map_license_error)
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

/// Return the exact fixed-bundle directory containing the running manager.
/// No renderer-supplied path participates in this decision.
pub fn current_bundle_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|path| bundle_root_for_executable(&path))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

pub fn usb_codex_root(bundle_root: &Path) -> PathBuf {
    bundle_root.join(USB_APPS_DIR_NAME).join(USB_CODEX_DIR_NAME)
}

pub fn usb_content_root(bundle_root: &Path) -> PathBuf {
    bundle_root.join(USB_CONTENT_DIR_NAME)
}

pub fn current_usb_codex_root() -> Option<PathBuf> {
    current_bundle_root().map(|root| usb_codex_root(&root))
}

pub fn current_usb_content_root() -> Option<PathBuf> {
    current_bundle_root().map(|root| usb_content_root(&root))
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
    let path = license_path(bundle_root);
    atomic_file::write_atomic(&path, &bytes)
        .map_err(|error| engine(format!("写入许可证失败: {error}")))?;
    // usb-license.json is mutable manager state and is intentionally outside
    // SHA256SUMS. Do not leave the atomic writer's backup in a successful
    // bundle: the bundle verifier treats backup files as interrupted state.
    let _ = fs::remove_file(atomic_file::backup_path(&path));
    Ok(())
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
    let path = local_guard_dir()?.join(name);
    codex_win_engine::validate_cleanup_path(&path).map_err(|error| engine(error.to_string()))?;
    Ok(path)
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
        signature_state: if signed_required() {
            "missing".to_string()
        } else {
            "unsigned-development".to_string()
        },
        codex_allowed: false,
        codex_block_reason: None,
        last_checked_at_ms: now_ms(),
        missing_since_ms: None,
        countdown_seconds: None,
        managed_processes: Vec::new(),
        message: status_message(status),
    }
}

fn mark_invalid_license(status: &mut UsbGuardStatus, detail: String) {
    status.signature_state = "invalid".to_string();
    status.codex_block_reason = Some(detail.clone());
    status.message = detail;
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
    if STARTUP_INTEGRITY_PENDING.load(Ordering::SeqCst) && guard_required() {
        let mut status = base_status("starting", "not-running");
        status.removable = true;
        status.codex_block_reason = Some("U 盘能力包正在进行启动完整性校验".to_string());
        status.message = "正在校验 U 盘能力包，完成后才会开放 Codex 操作".to_string();
        return status;
    }
    let mut status = status_path()
        .ok()
        .and_then(|path| read_json::<UsbGuardStatus>(&path))
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                base_status("stopped", "not-running")
            } else {
                disabled_status()
            }
        });
    // `status.json` is a persisted renderer snapshot. A process crash or a
    // slow helper launch can leave an old `authorized/running` value behind;
    // never expose that stale value as an actionable authorization state when
    // the live lease or helper lock is absent. Keep the status readable and
    // retryable so the UI can poll until startup settles.
    #[cfg(target_os = "windows")]
    if guard_required() {
        let lease_live = read_lease().is_some();
        let helper_live = guard_helper_is_live();
        status = mask_stale_authorized_status(status, lease_live, helper_live);
    }
    status
}

/// Only the fixed USB layout is mandatory-bound.  A normal installed manager
/// (which has no sibling `manifest.json` + `payload` directory) keeps the
/// existing online/historical Windows behavior.
pub fn guard_required() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|path| bundle_root_for_executable(&path))
            .is_some()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Return the observed physical drive root containing the running USB bundle.
/// It is derived from the current executable and canonicalized through the
/// same observation path as the lease, never from renderer-supplied text.
pub fn current_bundle_volume_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let bundle_root = current_bundle_root()?;
        let observation = observe_bundle(&bundle_root).ok()?;
        let root = observation.drive_root?;
        Some(PathBuf::from(root))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn path_has_reparse_point(path: &Path) -> Result<bool, AppError> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(engine(format!(
            "读取 U 盘程序目录属性失败 {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(not(target_os = "windows"))]
fn path_has_reparse_point(_path: &Path) -> Result<bool, AppError> {
    Ok(false)
}

fn license_digest(bundle_root: &Path) -> Result<String, AppError> {
    let bytes = fs::read(license_path(bundle_root))
        .map_err(|error| engine(format!("读取许可证摘要失败: {error}")))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Capability content is independent from the formal Codex signature. It
/// still requires the live removable-volume lease and physical re-observation,
/// but a development bundle may use the documented unsigned license.
pub fn require_usb_content_access(phase: &str) -> Result<(), AppError> {
    if !guard_required() {
        return Ok(());
    }
    // Do not trust a stale authorized status while the asynchronous startup
    // integrity scan is still pending. This gate intentionally accepts an
    // unsigned development lease, but it still requires the live lease and
    // watchdog that prove this process completed startup authorization.
    require_authorized().map_err(|error| {
        engine(format!(
            "usb_watchdog_not_ready phase={phase}：能力包操作尚未获得活动 U 盘租约：{error}"
        ))
    })?;
    let bundle = current_bundle_root().ok_or_else(|| {
        engine(format!(
            "usb_data_boundary phase={phase}：无法确定当前 USB bundle"
        ))
    })?;
    validate_bundle_policy_binding(&bundle).map_err(|error| {
        engine(format!(
            "usb_policy_gate phase={phase}：能力包操作被策略互锁拒绝：{error}"
        ))
    })?;
    let observation = observe_bundle(&bundle).map_err(|error| {
        engine(format!(
            "usb_data_boundary phase={phase}：现场 U 盘复核失败：{error}"
        ))
    })?;
    let fingerprint = observation.fingerprint.as_ref().ok_or_else(|| {
        engine(format!(
            "usb_not_ntfs phase={phase}：能力包只允许从可移动 NTFS U 盘读取"
        ))
    })?;
    let integrity =
        crate::app::usb_bundle::verify_bundle_integrity(&bundle, false).map_err(|error| {
            engine(format!(
                "usb_bundle_integrity_failed phase={phase}：{error}"
            ))
        })?;
    if !integrity
        .anchor_sha256
        .eq_ignore_ascii_case(&fingerprint.anchor_sha256)
    {
        return Err(engine(format!(
            "usb_anchor_mismatch phase={phase}：能力包所在 Bundle 已变化"
        )));
    }
    let content = usb_content_root(&bundle);
    if !content.is_dir() || path_has_reparse_point(&content)? {
        return Err(engine(format!(
            "usb_data_boundary phase={phase}：能力包目录不可用：{}",
            content.display()
        )));
    }
    log::debug!(
        "USB content gate passed phase={phase} bundle={} serial={} relative={}",
        bundle.display(),
        fingerprint.volume_serial,
        fingerprint.bundle_relative_path
    );
    Ok(())
}

/// Unified Codex gate. Every launch, install, repair, update and commit path
/// calls this boundary. The returned snapshot is captured from the live
/// observation immediately before the operation; it is never reconstructed
/// from renderer-provided paths.
pub fn require_signed_codex_bundle(
    phase: &str,
) -> Result<Option<SignedUsbAuthorization>, AppError> {
    if !guard_required() {
        return Ok(None);
    }
    // Give startup a short bounded opportunity to publish the helper lease,
    // and keep this failure distinct from an invalid package signature. The
    // renderer also performs a preflight, but this backend gate remains the
    // final authority for every protected operation.
    require_authorized().map_err(|error| {
        engine(format!(
            "usb_watchdog_not_ready phase={phase} failed: {error}"
        ))
    })?;
    let (bundle, fingerprint) = authorized_bundle_identity()
        .map_err(|error| engine(format!("usb_license_gate phase={phase} failed: {error}")))?;
    let license = read_license(&bundle)?.ok_or_else(|| {
        engine(format!(
            "usb_license_missing phase={phase}：当前 U 盘缺少 usb-license.json"
        ))
    })?;
    let signed = validate_license(&license, &fingerprint)
        .map_err(|error| engine(format!("usb_license_invalid phase={phase}：{error}")))?;
    if signed && license.key_id.is_none() && signed_required() {
        return Err(engine(format!(
            "usb_license_missing_key_id phase={phase}：正式许可证必须包含 keyId"
        )));
    }
    if !signed {
        return Err(engine(format!(
            "usb_license_unsigned phase={phase}：未签名许可证只允许诊断和能力包操作，Codex 操作必须使用正式签名许可证"
        )));
    }
    let lease = read_lease().ok_or_else(|| {
        engine(format!(
            "usb_watchdog_not_running phase={phase}：当前没有有效 U 盘租约"
        ))
    })?;
    if lease.installation_id.is_none() || lease.key_id.is_none() || lease.license_digest.is_none() {
        return Err(engine(format!(
            "usb_toctou_changed phase={phase}：当前租约缺少正式授权快照，请重启管理器"
        )));
    }

    // Hash/path/manifest verification is required for every route. Formal
    // USB builds additionally require the internal license/anchor binding and
    // Authenticode on the external MSIX/prerequisites; product-owned EXEs are
    // protected by the same signed USB anchor and SHA-256 rows.
    let integrity = verify_runtime_bundle_integrity(&bundle)
        .map_err(|error| engine(format!("usb_bundle_invalid phase={phase}：{error}")))?;
    if !integrity
        .anchor_sha256
        .eq_ignore_ascii_case(&fingerprint.anchor_sha256)
    {
        return Err(engine(format!(
            "usb_bundle_anchor_changed phase={phase}：现场锚点与物理观察不一致"
        )));
    }
    let apps = bundle.join(USB_APPS_DIR_NAME);
    let codex_root = usb_codex_root(&bundle);
    for candidate in [&bundle, &apps, &codex_root] {
        if path_has_reparse_point(candidate)? {
            return Err(engine(format!(
                "usb_bundle_reparse phase={phase}：U 盘程序目录不能是 Junction/符号链接：{}",
                candidate.display()
            )));
        }
    }
    let _ = local_data_paths()
        .map_err(|error| engine(format!("usb_data_boundary phase={phase} failed: {error}")))?;
    Ok(Some(SignedUsbAuthorization {
        bundle_root: bundle.clone(),
        codex_root,
        fingerprint: fingerprint.clone(),
        installation_id: license.installation_id,
        key_id: license.key_id.unwrap_or_default(),
        license_digest: license_digest(&bundle)?,
        checked_at_ms: now_ms(),
    }))
}

/// Re-observe and authorize the exact physical bundle owned by this manager
/// session.  Callers that hand work to a detached helper must persist this
/// fingerprint; a renderer path or a drive letter is not an authorization
/// token.
pub fn authorized_bundle_identity() -> Result<(PathBuf, UsbFingerprint), AppError> {
    require_authorized()?;
    let bundle_candidate =
        current_bundle_root().ok_or_else(|| engine("无法确定当前 U 盘套件目录"))?;
    // Re-observe the running bundle at the mutating boundary.  Do not trust
    // `status.json`'s renderer-oriented path snapshot: the watchdog may have
    // rediscovered the same volume using another valid Windows path spelling.
    let observation = observe_bundle(&bundle_candidate)?;
    let lease = read_lease().ok_or_else(|| engine("当前没有有效的 U 盘租约"))?;
    validate_lease_observation(&lease, &observation, std::process::id())?;
    let current_fingerprint = observation
        .fingerprint
        .as_ref()
        .ok_or_else(|| engine("当前安装器目录不在可移动 U 盘上"))?;
    let bundle = PathBuf::from(&observation.bundle_path);
    validate_bundle_policy_binding(&bundle)?;
    if path_has_reparse_point(&bundle)? {
        return Err(engine(format!(
            "当前 U 盘 bundle 根目录不能是 Junction/符号链接：{}",
            bundle.display()
        )));
    }
    let license = read_license(&bundle)?.ok_or_else(|| engine("当前 U 盘缺少许可证"))?;
    validate_license(&license, current_fingerprint)?;
    validate_lease_license_snapshot(&lease, &license, &bundle)?;
    if !fingerprints_match(&license.fingerprint, &lease.fingerprint) {
        return Err(engine(format!(
            "当前许可证与本会话租约不一致（currentPath={} leasePath={}）",
            observation.bundle_path, lease.bundle_path
        )));
    }
    log::debug!(
        "USB install destination authorized by fingerprint current_path={} lease_path={} serial={} relative={} anchor_schema={} anchor={}",
        observation.bundle_path,
        lease.bundle_path,
        current_fingerprint.volume_serial,
        current_fingerprint.bundle_relative_path,
        current_fingerprint.anchor_schema,
        fingerprint_excerpt(&current_fingerprint.anchor_sha256)
    );
    Ok((bundle, current_fingerprint.clone()))
}

/// Codex payload refresh is a release operation, not a development-mode
/// bundle operation.  Keep the general USB authorization API permissive for
/// local development, but require a configured signed license at this update
/// boundary so an unsigned auto-enrolled bundle can never refresh its payload.
pub fn authorized_signed_bundle_identity() -> Result<(PathBuf, UsbFingerprint), AppError> {
    let authorization = require_signed_codex_bundle("codex-payload")?
        .ok_or_else(|| engine("当前进程不是 USB bundle，不能执行 USB payload 更新"))?;
    if guard_required() {
        let license = read_license(&authorization.bundle_root)?
            .ok_or_else(|| engine("usb_license_missing：当前 U 盘缺少许可证"))?;
        if license.signature.is_none() {
            return Err(engine(
                "usb_license_unsigned：Codex payload 远程更新要求签名许可证",
            ));
        }
    }
    Ok((authorization.bundle_root, authorization.fingerprint))
}

/// Return the account binding from the currently authorized signed bundle.
/// The binding is consumed only by the backend launch hand-off and is never
/// returned to the renderer or written to local settings.
pub fn authorized_account_binding() -> Result<Option<license_core::UsbAccountBinding>, AppError> {
    if !guard_required() {
        return Ok(None);
    }
    let (bundle, fingerprint) = authorized_bundle_identity()?;
    let license =
        read_license(&bundle)?.ok_or_else(|| engine("usb_license_missing：当前 U 盘缺少许可证"))?;
    // Development/legacy USB bundles may still launch CCSwitch manually. Do
    // not reject those bundles merely because they cannot provide automatic
    // account injection; only a signed binding is eligible for the hand-off.
    if license.signature.is_none() {
        return Ok(None);
    }
    let signed = validate_license(&license, &fingerprint)
        .map_err(|error| engine(format!("usb_license_invalid：{error}")))?;
    if !signed {
        return Err(engine(
            "usb_license_unsigned：自动 CCSwitch 账号注入需要签名许可证",
        ));
    }
    let binding = license.account_binding;
    if let Some(binding) = binding.as_ref() {
        license_core::validate_account_binding(binding)
            .map_err(|error| engine(format!("usb_account_binding_invalid：{error}")))?;
    }
    Ok(binding)
}

/// Resolve and authorize the only install destination permitted to the fixed
/// USB product. Existing `apps`/`Codex` reparse points are rejected so a copied
/// or tampered bundle cannot redirect the payload onto C: or another volume.
pub fn authorized_usb_codex_root() -> Result<PathBuf, AppError> {
    let authorization = require_signed_codex_bundle("codex-install-destination")?
        .ok_or_else(|| engine("当前进程不是 USB bundle，不能解析 USB Codex 目录"))?;
    let bundle = authorization.bundle_root;
    let apps = bundle.join(USB_APPS_DIR_NAME);
    let codex = usb_codex_root(&bundle);
    for candidate in [&apps, &codex] {
        if path_has_reparse_point(candidate)? {
            return Err(engine(format!(
                "U 盘程序目录不能是 Junction/符号链接：{}",
                candidate.display()
            )));
        }
    }
    Ok(codex)
}

fn required_profile_path(name: &str) -> Result<PathBuf, AppError> {
    let value = std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| engine(format!("Windows 用户环境缺少 {name}")))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(engine(format!("Windows 用户环境 {name} 不是绝对路径")));
    }
    Ok(path)
}

/// Validate that all mutable Codex state stays outside the removable product
/// volume. This is an observed boundary, not an AppX policy assumption.
pub fn local_data_paths() -> Result<UsbLocalDataPaths, AppError> {
    let user_profile = required_profile_path("USERPROFILE")?;
    let roaming_app_data = required_profile_path("APPDATA")?;
    let local_app_data = required_profile_path("LOCALAPPDATA")?;
    let codex_home = user_profile.join(".codex");
    let usb_root = current_bundle_volume_root()
        .and_then(|root| normalize_drive_root(&root.to_string_lossy()))
        .ok_or_else(|| engine("无法确定当前 U 盘卷，不能核验本地数据边界"))?;
    for (label, path) in [
        ("CODEX_HOME", &codex_home),
        ("APPDATA", &roaming_app_data),
        ("LOCALAPPDATA", &local_app_data),
    ] {
        let declared_root = normalize_drive_root(&path.to_string_lossy()).ok_or_else(|| {
            engine(format!(
                "{label} 不是本地 Windows 盘路径：{}",
                path.display()
            ))
        })?;
        let observed_root = observed_drive_root(path)?;
        if declared_root.eq_ignore_ascii_case(&usb_root)
            || observed_root.eq_ignore_ascii_case(&usb_root)
        {
            return Err(engine(format!(
                "{label} 当前声明或解析后指向产品 U 盘（{}，实际卷 {}），无法保证程序/数据分离",
                path.display(),
                observed_root
            )));
        }
    }
    Ok(UsbLocalDataPaths {
        codex_home,
        roaming_app_data,
        local_app_data,
    })
}

/// Re-check the physical USB identity at a privileged-operation boundary.
///
/// `require_authorized()` checks the status, lease and live helper lock for the
/// command boundary. A UAC prompt can remain open long enough for a user to
/// remove the drive, reassign its letter, or insert another removable disk.
/// Before a machine-level AppX-volume write we therefore resolve the current
/// lease fingerprint against the live removable volumes again and require the
/// requested root to be the same physical volume.  This keeps an elevated
/// helper from registering a different disk that happens to reuse the old
/// drive letter.
#[cfg(target_os = "windows")]
pub fn verify_authorized_volume_root(volume_root: &Path) -> Result<(), AppError> {
    let _ = require_signed_codex_bundle("appx-volume-boundary")?;
    let expected_root = normalize_drive_root(&volume_root.to_string_lossy())
        .ok_or_else(|| engine("AppX 目标卷不是本地盘根路径"))?;
    let status = current_status();
    if status.status != "authorized" || status.watchdog != "running" {
        return Err(engine(format!(
            "U 盘看门狗状态已变化（status={} watchdog={}），拒绝执行管理员卷操作",
            status.status, status.watchdog
        )));
    }
    let Some(lease) = read_lease() else {
        return Err(engine("当前没有有效的 U 盘租约，拒绝执行管理员卷操作"));
    };
    if lease.schema_version != LEASE_SCHEMA_VERSION || lease.product != PRODUCT_ID {
        return Err(engine("U 盘租约版本或产品标识不匹配，拒绝执行管理员卷操作"));
    }
    if lease.owner_pid != std::process::id() {
        return Err(engine("U 盘租约不属于当前安装器进程，拒绝执行管理员卷操作"));
    }
    let Some(observation) = scan_removable_for(&lease.fingerprint) else {
        return Err(engine("授权 U 盘在提权前未能重新验证，可能已拔出或被替换"));
    };
    let actual_root = observation
        .drive_root
        .as_deref()
        .and_then(normalize_drive_root)
        .ok_or_else(|| engine("无法确定已验证 U 盘的实际盘符"))?;
    if actual_root != expected_root {
        return Err(engine(format!(
            "U 盘盘符在提权前发生变化（请求 {expected_root}，当前 {actual_root}），请重新启动安装器"
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn verify_authorized_volume_root(_volume_root: &Path) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

pub fn require_authorized() -> Result<(), AppError> {
    if !guard_required() {
        return Ok(());
    }
    let deadline =
        std::time::Instant::now() + Duration::from_millis(AUTHORIZATION_RETRY_TIMEOUT_MS);
    loop {
        let status = current_status();
        let lease_live = read_lease().is_some();
        #[cfg(target_os = "windows")]
        let helper_live = guard_helper_is_live();
        #[cfg(not(target_os = "windows"))]
        let helper_live = true;
        if status.status == "authorized"
            && status.watchdog == "running"
            && lease_live
            && helper_live
        {
            return Ok(());
        }
        // Startup publishes the status and helper lock asynchronously. Only
        // wait for states that can legitimately settle; a mismatch/invalid
        // license must fail immediately and remain fail-closed.
        if !authorization_may_still_settle(&status) || std::time::Instant::now() >= deadline {
            return Err(engine(format!(
                "未检测到有效 U 盘租约或看门狗（status={} watchdog={} lease={} helper={}），拒绝执行受保护操作",
                status.status, status.watchdog, lease_live, helper_live
            )));
        }
        std::thread::sleep(Duration::from_millis(AUTHORIZATION_RETRY_INTERVAL_MS));
    }
}

fn authorization_may_still_settle(status: &UsbGuardStatus) -> bool {
    status.status == "starting" || (status.status == "authorized" && status.watchdog == "running")
}

fn mask_stale_authorized_status(
    mut status: UsbGuardStatus,
    lease_live: bool,
    helper_live: bool,
) -> UsbGuardStatus {
    if status.status == "authorized"
        && status.watchdog == "running"
        && (!lease_live || !helper_live)
    {
        status.status = "starting".to_string();
        status.watchdog = "not-running".to_string();
        status.codex_allowed = false;
        status.codex_block_reason = Some(format!(
            "usb_watchdog_not_ready：活动 U 盘租约或看门狗尚未就绪（lease={} helper={}）",
            lease_live, helper_live
        ));
        status.message = "U 盘授权正在准备中，请保持连接并稍后重试".to_string();
    }
    status
}

#[cfg(target_os = "windows")]
fn guard_helper_is_live() -> bool {
    let Some(pid) = lock_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| text.trim().parse::<u32>().ok())
    else {
        return false;
    };
    process_matches(pid, Some(GUARD_EXE_NAME))
}

#[cfg(target_os = "windows")]
fn wait_for_guard_helper_ready() -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(GUARD_READY_TIMEOUT_MS);
    loop {
        if guard_helper_is_live() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(AUTHORIZATION_RETRY_INTERVAL_MS));
    }
}

/// Perform the small, read-only portion of startup validation. The expensive
/// SHA/AuthentiCode walk is deliberately deferred until after the local shell
/// exists when the machine already has the runtimes needed to render it. No
/// install, recovery, lease, or Codex operation is allowed while the returned
/// `IntegrityPending` state is active.
fn startup_preflight_impl(complete_integrity: bool) -> Result<StartupPreflight, AppError> {
    if !guard_required() {
        STARTUP_INTEGRITY_PENDING.store(false, Ordering::SeqCst);
        return Ok(StartupPreflight::Ready);
    }
    STARTUP_INTEGRITY_PENDING.store(!complete_integrity, Ordering::SeqCst);
    let bundle = current_bundle_root()
        .ok_or_else(|| engine("usb_license_bundle_missing：无法确定当前 USB bundle"))?;
    validate_bundle_policy_binding(&bundle)?;
    let observation = observe_bundle(&bundle)?;
    let fingerprint = observation
        .fingerprint
        .as_ref()
        .ok_or_else(|| engine("usb_license_media_invalid：当前介质必须是可移动 NTFS U 盘"))?;
    let license_reason = match read_license(&bundle) {
        Err(error) => Some(format!(
            "usb_license_invalid：启动前许可证读取失败：{error}"
        )),
        Ok(Some(license)) => match validate_license(&license, fingerprint) {
            Err(error) => Some(format!(
                "usb_license_invalid：启动前许可证校验失败：{error}"
            )),
            Ok(signed) if signed_required() && !signed => {
                Some("usb_license_unsigned：SignedRequired 启动前拒绝未签名许可证".to_string())
            }
            Ok(_) if signed_required() && license.key_id.is_none() => Some(
                "usb_license_missing_key_id：SignedRequired 启动前拒绝缺少 keyId 的许可证"
                    .to_string(),
            ),
            Ok(_) => None,
        },
        Ok(None) if signed_required() => {
            Some("usb_license_missing：SignedRequired 启动前缺少 usb-license.json".to_string())
        }
        Ok(None) => None,
    };
    if complete_integrity {
        verify_runtime_bundle_integrity(&bundle).map_err(|error| {
            engine(format!(
                "usb_bundle_invalid：启动前完整 bundle 校验失败：{error}"
            ))
        })?;
    }
    let _ = local_data_paths()?;
    if complete_integrity {
        STARTUP_INTEGRITY_PENDING.store(false, Ordering::SeqCst);
        Ok(match license_reason {
            Some(reason) => StartupPreflight::AuthorizationBlocked { reason },
            None => StartupPreflight::Ready,
        })
    } else {
        Ok(StartupPreflight::IntegrityPending {
            reason: license_reason.unwrap_or_else(|| {
                "启动完整性校验将在本地界面显示后完成；校验期间不开放 Codex 操作".to_string()
            }),
        })
    }
}

/// Read-only launch preflight. It returns quickly after validating the fixed
/// bundle shape, removable NTFS identity, license syntax/binding and local
/// data boundary. The full immutable tree check is performed by the manager
/// worker before any lease, recovery, shortcut or Codex operation is enabled.
pub fn startup_preflight() -> Result<StartupPreflight, AppError> {
    startup_preflight_impl(false)
}

/// Complete the preflight synchronously for machines that need a bundled
/// runtime installed before a Tauri window can be created. This preserves the
/// old fail-closed prerequisite path while the normal path gets a visible
/// local shell first.
pub fn complete_startup_preflight() -> Result<StartupPreflight, AppError> {
    startup_preflight_impl(true)
}

pub fn startup_integrity_pending() -> bool {
    STARTUP_INTEGRITY_PENDING.load(Ordering::SeqCst)
}

pub fn write_status(status: &UsbGuardStatus) -> Result<(), AppError> {
    if SUPPRESS_STATUS_AFTER_UNINSTALL.load(Ordering::SeqCst) {
        return Ok(());
    }
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
    path.rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
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
        .creation_flags(CREATE_NO_WINDOW)
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
        .creation_flags(CREATE_NO_WINDOW)
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
            .creation_flags(CREATE_NO_WINDOW)
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
            .creation_flags(CREATE_NO_WINDOW)
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

#[cfg(target_os = "windows")]
fn terminate_managed_processes_without_manager(lease: &UsbLease) -> usize {
    let mut scoped = lease.clone();
    scoped
        .managed_processes
        .retain(|process| process.pid != lease.owner_pid && process.role != "manager");
    terminate_managed_processes(&scoped)
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

    // `canonicalize` resolves 8.3 spellings and directory reparse points;
    // `normalize_bundle_path` then converts the verbatim result (`\\?\D:\...`)
    // to one stable DOS form for status/log output. Authorization itself still
    // uses the fingerprint below, never this string.
    let resolved = fs::canonicalize(bundle_root).map_err(|error| {
        engine(format!(
            "无法解析 U 盘套件真实路径 {}: {error}",
            bundle_root.display()
        ))
    })?;
    let path = normalize_bundle_path(&resolved.to_string_lossy()).ok_or_else(|| {
        engine(format!(
            "U 盘套件解析后不是本地 Windows 绝对路径: {}",
            resolved.display()
        ))
    })?;
    let normalized_bundle_root = PathBuf::from(&path);
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
    let fs_name_len = fs_name
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(fs_name.len());
    let file_system = String::from_utf16_lossy(&fs_name[..fs_name_len]);
    if !file_system.eq_ignore_ascii_case("NTFS") {
        // A removable FAT/exFAT volume is visible but is not an authorized
        // release medium. Returning no fingerprint makes every lease and
        // signed operation fail closed without treating a different
        // filesystem as an equivalent USB identity.
        return Ok(UsbObservation {
            removable: true,
            drive_root: Some(drive_root),
            bundle_path: path,
            fingerprint: None,
        });
    }
    let relative =
        bundle_relative_path(&path, &drive_root).ok_or_else(|| engine("U 盘目录不在卷根路径下"))?;
    let anchor_schema = bundle_anchor_schema(&normalized_bundle_root)?;
    let fingerprint = UsbFingerprint {
        volume_serial: serial,
        anchor_sha256: anchor_sha256(&normalized_bundle_root)?,
        bundle_relative_path: relative,
        anchor_schema,
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
fn scan_removable_matches(expected: &UsbFingerprint) -> Vec<UsbObservation> {
    let mut matches = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let Some(candidate) = bundle_path_on_drive(&root, &expected.bundle_relative_path) else {
            continue;
        };
        let candidate = PathBuf::from(candidate);
        if let Ok(observation) = observe_windows_bundle(&candidate) {
            if observation
                .fingerprint
                .as_ref()
                .is_some_and(|actual| fingerprints_match(expected, actual))
            {
                matches.push(observation);
            }
        }
    }
    matches
}

#[cfg(target_os = "windows")]
fn scan_removable_for(expected: &UsbFingerprint) -> Option<UsbObservation> {
    let matches = scan_removable_matches(expected);
    (matches.len() == 1).then(|| matches.into_iter().next().expect("one match"))
}

#[cfg(not(target_os = "windows"))]
fn scan_removable_for(_expected: &UsbFingerprint) -> Option<UsbObservation> {
    None
}

#[cfg(not(target_os = "windows"))]
fn scan_removable_matches(_expected: &UsbFingerprint) -> Vec<UsbObservation> {
    Vec::new()
}

fn file_sha256(path: &Path) -> Result<[u8; 32], AppError> {
    let mut file = File::open(path)
        .map_err(|error| engine(format!("读取看门狗程序失败 {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| engine(format!("校验看门狗程序失败 {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn guard_binary_is_current(source: &Path, target: &Path) -> Result<bool, AppError> {
    if !target.is_file() {
        return Ok(false);
    }
    let source_len = fs::metadata(source)
        .map_err(|error| engine(format!("读取当前安装器信息失败: {error}")))?
        .len();
    let target_len = fs::metadata(target)
        .map_err(|error| engine(format!("读取本地看门狗信息失败: {error}")))?
        .len();
    if source_len != target_len {
        return Ok(false);
    }
    Ok(file_sha256(source)? == file_sha256(target)?)
}

#[cfg(target_os = "windows")]
fn stop_previous_guard_for_refresh() -> Result<(), AppError> {
    remove_lease();
    let path = lock_path()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let owner = fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        match owner {
            Some(pid) if process_matches(pid, None) => {
                if std::time::Instant::now() >= deadline {
                    return Err(engine(format!(
                        "旧版看门狗进程 {pid} 未在 5 秒内退出，无法安全刷新本地组件"
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                if let Err(error) = fs::remove_file(&path) {
                    if error.kind() != io::ErrorKind::NotFound {
                        return Err(engine(format!("清理旧版看门狗锁失败: {error}")));
                    }
                }
                return Ok(());
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn stop_previous_guard_for_refresh() -> Result<(), AppError> {
    Ok(())
}

fn ensure_local_guard_binary(current_exe: &Path) -> Result<PathBuf, AppError> {
    let dir = local_guard_dir()?;
    fs::create_dir_all(&dir).map_err(|error| engine(format!("创建本地看门狗目录失败: {error}")))?;
    let target = dir.join(GUARD_EXE_NAME);
    if codex_win_engine::same_windows_path(&target, current_exe) {
        return Ok(target);
    }
    if guard_binary_is_current(current_exe, &target)? {
        return Ok(target);
    }

    // The helper is a local copy of the manager executable. Earlier builds
    // only copied it when missing, so a machine could keep running stale lease
    // and path-normalization code forever after the USB bundle was upgraded.
    // Stop that exact old helper before replacing its image, then verify the
    // new bytes before issuing a fresh lease.
    stop_previous_guard_for_refresh()?;
    let temp = dir.join(format!(
        "{GUARD_EXE_NAME}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    fs::copy(current_exe, &temp)
        .map_err(|error| engine(format!("复制新版本地看门狗失败: {error}")))?;
    if file_sha256(current_exe)? != file_sha256(&temp)? {
        let _ = fs::remove_file(&temp);
        return Err(engine("新版本地看门狗复制后校验不一致"));
    }
    if target.exists() {
        fs::remove_file(&target)
            .map_err(|error| engine(format!("替换旧版本地看门狗失败: {error}")))?;
    }
    if let Err(error) = fs::rename(&temp, &target) {
        let _ = fs::remove_file(&temp);
        return Err(engine(format!("安装新版本地看门狗失败: {error}")));
    }
    if !guard_binary_is_current(current_exe, &target)? {
        return Err(engine("本地看门狗更新后的完整性校验失败"));
    }
    Ok(target)
}

#[cfg(target_os = "windows")]
fn spawn_guard(executable: &Path, lease_path: &Path) -> Result<(), AppError> {
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
fn finish_restricted_start(status: UsbGuardStatus) -> Result<UsbGuardStatus, AppError> {
    // Publish the restricted state before clearing a prior lease. The helper
    // refresh path only removes the lease and waits for the detached helper;
    // it never terminates the current manager process. Even if the status
    // write fails, still perform the lease/helper cleanup so an old authorized
    // session cannot survive a newly rejected startup.
    let status_write = write_status(&status);
    let lease_cleanup = stop_previous_guard_for_refresh();
    match (status_write, lease_cleanup) {
        (Ok(()), Ok(())) => Ok(status),
        (Err(status_error), Ok(())) => Err(status_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(status_error), Err(cleanup_error)) => Err(engine(format!(
            "受限启动状态写入失败，且旧 U 盘租约清理失败：status={status_error}; cleanup={cleanup_error}"
        ))),
    }
}

#[cfg(target_os = "windows")]
pub fn start_for_manager() -> Result<UsbGuardStatus, AppError> {
    let result = start_for_manager_inner();
    // Keep the fail-closed `starting` status visible when an unexpected
    // initialization error occurs. A stale status.json must never become the
    // apparent authorization state after the pending marker is cleared.
    if result.is_ok() {
        STARTUP_INTEGRITY_PENDING.store(false, Ordering::SeqCst);
    }
    result
}

#[cfg(target_os = "windows")]
fn start_for_manager_inner() -> Result<UsbGuardStatus, AppError> {
    let executable =
        std::env::current_exe().map_err(|error| engine(format!("无法确定安装器路径: {error}")))?;
    let Some(bundle_root) = bundle_root_for_executable(&executable) else {
        let mut status = base_status("unsupported", "not-running");
        status.message = "未找到离线安装器根目录，U 盘绑定未启用".to_string();
        return finish_restricted_start(status);
    };
    let observation = observe_windows_bundle(&bundle_root)?;
    let Some(fingerprint) = observation.fingerprint.clone() else {
        let mut status = base_status("unsupported", "not-running");
        status.removable = observation.removable;
        status.drive_root = observation.drive_root;
        status.bundle_path = Some(observation.bundle_path);
        return finish_restricted_start(status);
    };
    let observed_bundle_root = PathBuf::from(&observation.bundle_path);
    validate_bundle_policy_binding(&observed_bundle_root)?;
    if let Err(error) = verify_runtime_bundle_integrity(&observed_bundle_root) {
        let mut status = base_status("mismatch", "not-running");
        status.removable = true;
        status.drive_root = observation.drive_root;
        status.bundle_path = Some(observation.bundle_path);
        status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
        status.signature_state = "invalid".to_string();
        status.codex_block_reason = Some(error.to_string());
        status.message = error.to_string();
        return finish_restricted_start(status);
    }

    let existing_license = match read_license(&observed_bundle_root) {
        Ok(value) => value,
        Err(error) => {
            let mut status = base_status("mismatch", "not-running");
            status.removable = true;
            status.drive_root = observation.drive_root;
            status.bundle_path = Some(observation.bundle_path);
            status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
            status.signature_state = "invalid".to_string();
            status.codex_block_reason = Some(error.to_string());
            status.message = error.to_string();
            return finish_restricted_start(status);
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
                mark_invalid_license(&mut status, error.to_string());
                return finish_restricted_start(status);
            }
        },
        None if signed_required() => {
            let mut status = base_status("mismatch", "not-running");
            status.removable = true;
            status.drive_root = observation.drive_root;
            status.bundle_path = Some(observation.bundle_path);
            status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
            status.signature_state = "missing".to_string();
            status.codex_block_reason =
                Some("SignedRequired 构建缺少 usb-license.json".to_string());
            status.message = "正式 USB 包缺少签名许可证，未创建租约".to_string();
            return finish_restricted_start(status);
        }
        None => {
            let license = new_unsigned_license(fingerprint.clone());
            write_license(&observed_bundle_root, &license)?;
            log::warn!(
                "USB license auto-enrolled as unsigned; sign release bundles before shipping"
            );
            (license, false)
        }
    };

    let startup_plan = startup_guard_plan(signed_required(), signed);
    if !startup_plan.establish_lease {
        debug_assert!(!startup_plan.prepare_or_start_helper);
        debug_assert!(startup_plan.clear_current_lease);
        debug_assert!(startup_plan.preserve_current_manager);
        let mut status = base_status("mismatch", "not-running");
        status.removable = true;
        status.drive_root = observation.drive_root;
        status.bundle_path = Some(observation.bundle_path);
        status.bundle_relative_path = Some(fingerprint.bundle_relative_path);
        status.signed = signed;
        status.signature_state = "unsigned-development".to_string();
        status.codex_allowed = startup_plan.codex_allowed;
        status.codex_block_reason = Some(
            "usb_license_unsigned：SignedRequired 下未签名许可证不能建立租约或执行 Codex 操作"
                .to_string(),
        );
        status.message =
            "正式 USB 包许可证未签名，未创建租约或启动看门狗；管理器保持受限".to_string();

        return finish_restricted_start(status);
    }

    let lease = UsbLease {
        schema_version: LEASE_SCHEMA_VERSION,
        product: PRODUCT_ID.to_string(),
        lease_id: Uuid::new_v4().to_string(),
        bundle_path: observation.bundle_path.clone(),
        fingerprint: license.fingerprint.clone(),
        installation_id: Some(license.installation_id.clone()),
        key_id: license.key_id.clone(),
        license_digest: Some(license_digest(&observed_bundle_root)?),
        owner_pid: std::process::id(),
        managed_processes: vec![ManagedProcess {
            pid: std::process::id(),
            role: "manager".to_string(),
            executable: Some(executable.to_string_lossy().into_owned()),
            started_at_ms: now_ms(),
        }],
        issued_at: now_ms().to_string(),
    };
    // Refresh the local helper before creating the new lease. A stale helper
    // must never see and retain a newly-issued lease while we are trying to
    // replace its executable.
    // This stop is also required when the helper binary bytes are unchanged:
    // a previous manager instance may still own the lock and otherwise cause
    // the new manager to publish a false `running` status.
    stop_previous_guard_for_refresh()?;
    let local_binary = ensure_local_guard_binary(&executable)?;
    write_lease(&lease)?;
    let lease_file = lease_path()?;
    let spawn_result = spawn_guard(&local_binary, &lease_file);
    // `spawn()` only means Windows accepted process creation. Do not publish
    // an actionable status until the detached helper has acquired the lock;
    // otherwise the first click can race the helper and fail at the backend
    // lease gate even though the USB signature is valid.
    let helper_ready = spawn_result.is_ok() && wait_for_guard_helper_ready();
    let mut status = base_status(
        "authorized",
        if helper_ready {
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
    status.signature_state = if signed {
        "valid".to_string()
    } else {
        "unsigned-development".to_string()
    };
    status.codex_allowed = helper_ready && signed;
    status.codex_block_reason = (!status.codex_allowed).then(|| {
        if !signed {
            "未签名开发许可证不能执行 Codex 操作".to_string()
        } else if spawn_result.is_err() {
            "USB 看门狗未能启动".to_string()
        } else {
            "usb_watchdog_not_ready：看门狗进程未在启动窗口内登记活动锁，请稍后重试".to_string()
        }
    });
    status.managed_processes = lease.managed_processes;
    status.message = if signed && helper_ready {
        "U 盘已验证，签名许可证有效".to_string()
    } else if signed {
        "U 盘已验证，但看门狗尚未就绪，请稍后重试".to_string()
    } else if helper_ready {
        "U 盘已验证（开发许可证未签名），看门狗运行中".to_string()
    } else {
        "U 盘已验证（开发许可证未签名），但看门狗尚未就绪，请稍后重试".to_string()
    };
    if let Err(error) = spawn_result {
        status.message = format!("{}；{error}", status.message);
    }
    write_status(&status)?;
    Ok(status)
}

#[cfg(not(target_os = "windows"))]
pub fn start_for_manager() -> Result<UsbGuardStatus, AppError> {
    STARTUP_INTEGRITY_PENDING.store(false, Ordering::SeqCst);
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
        if let Err(error) = codex_win_engine::validate_cleanup_path(&dir) {
            notes.push(format!("本地 U 盘看门狗目录清理失败：{error}"));
            return notes;
        }
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
#[cfg(not(target_os = "windows"))]
pub fn run_guard_cli() {}

#[cfg(target_os = "windows")]
pub fn run_guard_cli() {
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
    while let Some(lease) = read_json::<UsbLease>(&lease_file) {
        if lease.schema_version != LEASE_SCHEMA_VERSION || lease.product != PRODUCT_ID {
            break;
        }
        let present = scan_removable_for(&lease.fingerprint);
        let now = now_ms();
        if let Some(observation) = present {
            missing_since = None;
            let bundle = PathBuf::from(&observation.bundle_path);
            let license_check = read_license(&bundle).and_then(|license| {
                let license = license.ok_or_else(|| engine("USB 看门狗发现许可证缺失"))?;
                validate_lease_observation(&lease, &observation, lease.owner_pid)?;
                let signed = validate_license(
                    &license,
                    observation
                        .fingerprint
                        .as_ref()
                        .ok_or_else(|| engine("USB 看门狗发现物理指纹缺失"))?,
                )?;
                if signed_required() && !signed {
                    return Err(engine("USB 看门狗发现未签名许可证"));
                }
                if signed
                    && (lease.installation_id.is_none()
                        || lease.key_id.is_none()
                        || lease.license_digest.is_none())
                {
                    return Err(engine("USB 看门狗发现租约缺少签名授权快照"));
                }
                validate_lease_license_snapshot(&lease, &license, &bundle)?;
                Ok((license, signed))
            });
            let (license, signed) = match license_check {
                Ok(value) => value,
                Err(error) => {
                    // A mutable-license failure can race with a restricted
                    // manager startup that is clearing the lease. Stop only
                    // Codex/other managed children; never taskkill the
                    // manager that must remain available for diagnostics and
                    // user-initiated uninstall.
                    let terminated = terminate_managed_processes_without_manager(&lease);
                    let mut status = base_status("mismatch", "not-running");
                    status.removable = true;
                    status.drive_root = observation.drive_root;
                    status.bundle_path = Some(observation.bundle_path);
                    status.bundle_relative_path =
                        Some(lease.fingerprint.bundle_relative_path.clone());
                    status.signature_state = "invalid".to_string();
                    status.codex_block_reason = Some(error.to_string());
                    status.last_checked_at_ms = now;
                    status.managed_processes = lease.managed_processes.clone();
                    status.message = format!(
                        "USB 许可证现场复核失败，已停止 {terminated} 个受保护进程：{error}"
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
            };
            let mut status = base_status("authorized", "running");
            status.removable = true;
            status.drive_root = observation.drive_root;
            status.bundle_path = Some(observation.bundle_path);
            status.bundle_relative_path = Some(lease.fingerprint.bundle_relative_path.clone());
            status.signed = signed;
            status.signature_state = if signed {
                "valid".to_string()
            } else {
                "unsigned-development".to_string()
            };
            status.codex_allowed = signed;
            status.codex_block_reason =
                (!status.codex_allowed).then(|| "未签名开发许可证不能执行 Codex 操作".to_string());
            let _ = license;
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
                status.bundle_relative_path = Some(lease.fingerprint.bundle_relative_path.clone());
                status.managed_processes = lease.managed_processes.clone();
                status.message = format!(
                    "{}（已结束 {terminated} 个受保护进程）",
                    status_message("missing")
                );
                let _ = write_json_atomic(
                    &status_path().unwrap_or_else(|_| lease_file.with_file_name(STATUS_FILE_NAME)),
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
    use base64::engine::general_purpose::STANDARD as B64;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn startup_preflight_restricted_ui_never_counts_as_ready() {
        let blocked = StartupPreflight::AuthorizationBlocked {
            reason: "usb_license_missing".to_string(),
        };
        assert!(blocked.is_authorization_blocked());
        assert_eq!(blocked.reason(), Some("usb_license_missing"));
        let pending = StartupPreflight::IntegrityPending {
            reason: "bundle scan deferred".to_string(),
        };
        assert!(!pending.is_authorization_blocked());
        assert!(pending.is_integrity_pending());
        assert_eq!(pending.reason(), Some("bundle scan deferred"));
        assert!(!StartupPreflight::Ready.is_authorization_blocked());
        assert!(!StartupPreflight::Ready.is_integrity_pending());
        assert_eq!(StartupPreflight::Ready.reason(), None);
    }

    #[test]
    fn authorization_retry_only_covers_transient_startup_states() {
        let starting = base_status("starting", "not-running");
        assert!(authorization_may_still_settle(&starting));

        let published = base_status("authorized", "running");
        assert!(authorization_may_still_settle(&published));

        let mismatch = base_status("mismatch", "not-running");
        assert!(!authorization_may_still_settle(&mismatch));

        let stopped = base_status("authorized", "not-running");
        assert!(!authorization_may_still_settle(&stopped));
    }

    #[test]
    fn stale_authorized_status_is_masked_when_live_guard_evidence_is_missing() {
        let published = base_status("authorized", "running");
        let masked = mask_stale_authorized_status(published, true, false);
        assert_eq!(masked.status, "starting");
        assert_eq!(masked.watchdog, "not-running");
        assert!(!masked.codex_allowed);
        assert!(masked
            .codex_block_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("helper=false")));

        let live = base_status("authorized", "running");
        let unchanged = mask_stale_authorized_status(live.clone(), true, true);
        assert_eq!(unchanged, live);
    }

    #[test]
    fn bundle_policy_binding_rejects_missing_or_mismatched_policy() {
        let root = std::env::temp_dir().join(format!("codex-usb-policy-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("usb-manifest.json");

        fs::write(&manifest, br#"{"usbGuard":{"signing":{}}}"#).unwrap();
        let missing = validate_bundle_policy_binding(&root)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("usb_policy_missing"), "{missing}");

        let mismatched = if policy_name() == "SignedRequired" {
            "DevelopmentAutoEnroll"
        } else {
            "SignedRequired"
        };
        fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({
                "usbGuard": {"signing": {"policy": mismatched}}
            }))
            .unwrap(),
        )
        .unwrap();
        let mismatch = validate_bundle_policy_binding(&root)
            .unwrap_err()
            .to_string();
        assert!(mismatch.contains("usb_policy_mismatch"), "{mismatch}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_policy_binding_accepts_the_manager_compile_policy() {
        let root = std::env::temp_dir().join(format!("codex-usb-policy-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("usb-manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "usbGuard": {"signing": {"policy": policy_name()}}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(validate_bundle_policy_binding(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_required_unsigned_license_clears_lease_without_stopping_manager() {
        let plan = startup_guard_plan(true, false);
        assert_eq!(
            plan,
            StartupGuardPlan {
                establish_lease: false,
                prepare_or_start_helper: false,
                clear_current_lease: true,
                preserve_current_manager: true,
                codex_allowed: false,
            }
        );

        let development_plan = startup_guard_plan(false, false);
        assert!(development_plan.establish_lease);
        assert!(development_plan.prepare_or_start_helper);
        assert!(!development_plan.clear_current_lease);
        assert!(!development_plan.codex_allowed);
    }

    #[test]
    fn invalid_license_status_is_explicitly_blocked() {
        let mut status = base_status("mismatch", "not-running");
        mark_invalid_license(&mut status, "fingerprint mismatch".to_string());
        assert_eq!(status.signature_state, "invalid");
        assert_eq!(
            status.codex_block_reason.as_deref(),
            Some("fingerprint mismatch")
        );
        assert_eq!(status.message, "fingerprint mismatch");
        assert!(!status.codex_allowed);
    }

    #[test]
    fn signed_license_key_id_must_match_the_compile_time_active_key() {
        assert_eq!(
            active_key_binding_state(None, Some("arbitrary-key")),
            ActiveKeyBinding::MissingActiveKeyId
        );
        assert_eq!(
            active_key_binding_state(Some("active-key"), None),
            ActiveKeyBinding::MissingLicenseKeyId
        );
        assert_eq!(
            active_key_binding_state(Some("active-key"), Some("other-key")),
            ActiveKeyBinding::Mismatch
        );
        assert_eq!(
            active_key_binding_state(Some("active-key"), Some("active-key")),
            ActiveKeyBinding::Matched
        );
        assert_eq!(
            active_key_binding_state(Some("active key"), Some("active key")),
            ActiveKeyBinding::InvalidActiveKeyId
        );
        assert_eq!(
            active_key_binding_state(Some("active-key"), Some("other/key")),
            ActiveKeyBinding::InvalidLicenseKeyId
        );
    }

    #[test]
    fn normalizes_drive_roots_and_rejects_unc() {
        assert_eq!(normalize_drive_root("f:/"), Some("F:\\".to_string()));
        assert_eq!(
            normalize_drive_root("F:\\bundle\\setup.exe"),
            Some("F:\\".to_string())
        );
        assert_eq!(normalize_drive_root("\\\\server\\share"), None);
        assert_eq!(
            normalize_drive_root(r"\\?\c:\Users\someone\.codex"),
            Some("C:\\".to_string())
        );
        assert_eq!(
            normalize_drive_root(r"\\.\D:\portable\Codex"),
            Some("D:\\".to_string())
        );
        assert_eq!(normalize_drive_root(r"\\?\UNC\server\share"), None);
        assert_eq!(
            normalize_drive_root(r"\\?\Volume{11111111-1111-1111-1111-111111111111}\"),
            None
        );
        assert_eq!(normalize_drive_root("relative\\setup.exe"), None);
        assert_eq!(normalize_drive_root("D:relative\\setup.exe"), None);
    }

    #[test]
    fn normalizes_equivalent_bundle_path_spellings() {
        let expected = r"D:\usb-oneclick\payload";
        assert_eq!(normalize_bundle_path(expected).as_deref(), Some(expected));
        assert_eq!(
            normalize_bundle_path(r"d:\\usb-oneclick\.\payload\").as_deref(),
            Some(r"D:\usb-oneclick\payload")
        );
        assert_eq!(
            normalize_bundle_path(r"\\?\D:\usb-oneclick\payload").as_deref(),
            Some(expected)
        );
        assert_eq!(
            normalize_bundle_path(r"\\.\d:\usb-oneclick\payload").as_deref(),
            Some(expected)
        );
        assert_eq!(normalize_bundle_path(r"D:\usb\..\other"), None);
        assert_eq!(normalize_bundle_path(r"D:usb-oneclick"), None);
        assert_eq!(normalize_bundle_path(r"\\server\share\bundle"), None);
    }

    #[test]
    fn scanned_bundle_path_has_exactly_one_root_separator() {
        assert_eq!(
            bundle_path_on_drive(r"D:\", "usb-oneclick-v0.5.20-final").as_deref(),
            Some(r"D:\usb-oneclick-v0.5.20-final")
        );
        assert_eq!(
            bundle_path_on_drive(r"d:\", r"\usb-oneclick\payload").as_deref(),
            Some(r"D:\usb-oneclick\payload")
        );
        assert_eq!(bundle_path_on_drive(r"D:\", r"usb\..\other"), None);
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
            anchor_schema: ANCHOR_SCHEMA_V1,
        };
        let same = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: expected.anchor_sha256.to_uppercase(),
            bundle_relative_path: "CODEX/SETUP".to_string(),
            anchor_schema: ANCHOR_SCHEMA_V1,
        };
        let copied = UsbFingerprint {
            volume_serial: 43,
            ..same.clone()
        };
        assert!(fingerprints_match(&expected, &same));
        assert!(!fingerprints_match(&expected, &copied));
        assert!(!fingerprints_match(
            &expected,
            &UsbFingerprint {
                anchor_schema: ANCHOR_SCHEMA_V2,
                ..same
            }
        ));
    }

    #[test]
    fn lease_authorization_uses_fingerprint_not_full_path_text() {
        let fingerprint = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: "a".repeat(64),
            bundle_relative_path: "usb-oneclick-v0.5.20-final".to_string(),
            anchor_schema: ANCHOR_SCHEMA_V1,
        };
        let lease = UsbLease {
            schema_version: LEASE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            lease_id: "lease".to_string(),
            bundle_path: r"D:\usb-oneclick-v0.5.20-final".to_string(),
            fingerprint: fingerprint.clone(),
            installation_id: None,
            key_id: None,
            license_digest: None,
            owner_pid: 77,
            managed_processes: Vec::new(),
            issued_at: "now".to_string(),
        };
        let same_object_different_text = UsbObservation {
            removable: true,
            drive_root: Some(r"D:\".to_string()),
            bundle_path: r"D:\\usb-oneclick-v0.5.20-final".to_string(),
            fingerprint: Some(UsbFingerprint {
                anchor_sha256: fingerprint.anchor_sha256.to_uppercase(),
                ..fingerprint.clone()
            }),
        };
        assert!(validate_lease_observation(&lease, &same_object_different_text, 77).is_ok());

        let mut wrong_folder = same_object_different_text.clone();
        wrong_folder
            .fingerprint
            .as_mut()
            .unwrap()
            .bundle_relative_path = "copied-folder".to_string();
        let error = validate_lease_observation(&lease, &wrong_folder, 77).unwrap_err();
        let detail = error.to_string();
        assert!(detail.contains("currentRelative=copied-folder"));
        assert!(detail.contains("leaseRelative=usb-oneclick-v0.5.20-final"));
        assert!(validate_lease_observation(&lease, &same_object_different_text, 78).is_err());
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
            anchor_schema: ANCHOR_SCHEMA_V1,
        });
        let payload = unsigned_license_payload(&license).unwrap();
        license.signature = Some(B64.encode(signing.sign(&payload).to_bytes()));
        let public = B64.encode(signing.verifying_key().to_bytes());
        assert!(verify_signature_with_key(&license, &public).is_ok());
        license.fingerprint.volume_serial = 8;
        assert!(verify_signature_with_key(&license, &public).is_err());
    }

    #[test]
    fn legacy_v1_signed_license_without_anchor_schema_still_verifies() {
        let signing = SigningKey::from_bytes(&[12u8; 32]);
        let license = new_unsigned_license(UsbFingerprint {
            volume_serial: 7,
            anchor_sha256: "c".repeat(64),
            bundle_relative_path: "codex".to_string(),
            anchor_schema: ANCHOR_SCHEMA_V1,
        });
        let signature = B64.encode(
            signing
                .sign(&legacy_unsigned_license_payload(&license).unwrap())
                .to_bytes(),
        );
        let mut legacy_json = serde_json::to_value(&license).unwrap();
        legacy_json["fingerprint"]
            .as_object_mut()
            .unwrap()
            .remove("anchorSchema");
        legacy_json["signature"] = serde_json::Value::String(signature);
        let legacy: UsbLicense = serde_json::from_value(legacy_json).unwrap();
        let public = B64.encode(signing.verifying_key().to_bytes());
        assert_eq!(legacy.fingerprint.anchor_schema, ANCHOR_SCHEMA_V1);
        assert!(verify_signature_with_key(&legacy, &public).is_ok());
    }

    fn anchor_fixture(manifest: &str, usb_manifest: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("codex-usb-anchor-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("payload")).unwrap();
        fs::write(root.join("manifest.json"), manifest).unwrap();
        fs::write(root.join("usb-manifest.json"), usb_manifest).unwrap();
        fs::write(
            root.join("SHA256SUMS.txt"),
            format!("{}  payload/OpenAI.Codex_x64.msix\n", "a".repeat(64)),
        )
        .unwrap();
        root
    }

    #[test]
    fn v2_anchor_ignores_only_allow_listed_mutable_metadata() {
        let root = anchor_fixture(
            r#"{
                "packageName": "OpenAI.Codex",
                "codexDesktopVersion": "26.1.1",
                "packageVersion": "26.1.1.0",
                "releaseTag": "v26.1.1",
                "architecture": "x64",
                "payload": {
                    "path": "payload/OpenAI.Codex_x64.msix",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "size": 11
                },
                "immutable": {"z": 2, "a": [true, null]}
            }"#,
            r#"{
                "schemaVersion": 2,
                "product": "codex-usb-oneclick",
                "codexInstaller": {
                    "file": "CodexDesktopSetup.exe",
                    "payload": "payload/OpenAI.Codex_x64.msix",
                    "payloadSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "payloadSize": 11
                },
                "programLayout": {"mode": "verified-msix-extraction", "codexRoot": "apps/Codex"}
            }"#,
        );
        assert_eq!(bundle_anchor_schema(&root).unwrap(), ANCHOR_SCHEMA_V2);
        let original = anchor_sha256(&root).unwrap();

        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        manifest["codexDesktopVersion"] = serde_json::Value::String("26.2.2".to_string());
        manifest["packageVersion"] = serde_json::Value::String("26.2.2.0".to_string());
        manifest["releaseTag"] = serde_json::Value::String("v26.2.2".to_string());
        manifest["payload"]["sha256"] = serde_json::Value::String("b".repeat(64));
        manifest["payload"]["size"] = serde_json::Value::Number(22.into());
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let mut usb_manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("usb-manifest.json")).unwrap())
                .unwrap();
        usb_manifest["codexInstaller"]["payloadSha256"] = serde_json::Value::String("b".repeat(64));
        usb_manifest["codexInstaller"]["payloadSize"] = serde_json::Value::Number(22.into());
        fs::write(
            root.join("usb-manifest.json"),
            serde_json::to_vec_pretty(&usb_manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(anchor_sha256(&root).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v2_anchor_changes_when_an_immutable_field_changes() {
        let root = anchor_fixture(
            r#"{
                "packageName": "OpenAI.Codex",
                "architecture": "x64",
                "payload": {"path": "payload/OpenAI.Codex_x64.msix", "sha256": "a", "size": 1}
            }"#,
            r#"{
                "schemaVersion": 2,
                "product": "codex-usb-oneclick",
                "codexInstaller": {"payload": "payload/OpenAI.Codex_x64.msix", "payloadSha256": "a", "payloadSize": 1}
            }"#,
        );
        let original = anchor_sha256(&root).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        manifest["payload"]["path"] =
            serde_json::Value::String("payload/OpenAI.Codex_arm64.msix".to_string());
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("SHA256SUMS.txt"),
            format!("{}  payload/OpenAI.Codex_arm64.msix\n", "a".repeat(64)),
        )
        .unwrap();
        assert_ne!(anchor_sha256(&root).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v1_anchor_keeps_historical_raw_bytes() {
        let root = anchor_fixture(
            r#"{"packageName":"OpenAI.Codex","codexDesktopVersion":"26.1.1"}"#,
            r#"{"schemaVersion":1,"product":"codex-usb-oneclick"}"#,
        );
        assert_eq!(bundle_anchor_schema(&root).unwrap(), ANCHOR_SCHEMA_V1);
        assert_eq!(
            anchor_sha256(&root).unwrap(),
            raw_anchor_sha256(&root).unwrap()
        );
        let original = anchor_sha256(&root).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{ "packageName": "OpenAI.Codex", "codexDesktopVersion": "26.1.1" }"#,
        )
        .unwrap();
        assert_ne!(anchor_sha256(&root).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_fingerprint_json_defaults_anchor_schema_to_v1() {
        let fingerprint_json = serde_json::json!({
            "volumeSerial": 42,
            "anchorSha256": "a".repeat(64),
            "bundleRelativePath": "codex"
        });
        let fingerprint: UsbFingerprint = serde_json::from_value(fingerprint_json.clone()).unwrap();
        assert_eq!(fingerprint.anchor_schema, ANCHOR_SCHEMA_V1);

        let license: UsbLicense = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "product": PRODUCT_ID,
            "installationId": "00000000-0000-0000-0000-000000000002",
            "fingerprint": fingerprint_json,
            "issuedAt": "1"
        }))
        .unwrap();
        assert_eq!(license.fingerprint.anchor_schema, ANCHOR_SCHEMA_V1);
        assert_eq!(
            serde_json::to_value(&license).unwrap()["fingerprint"]["anchorSchema"],
            ANCHOR_SCHEMA_V1
        );

        let lease: UsbLease = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "product": PRODUCT_ID,
            "leaseId": "legacy-lease",
            "bundlePath": "D:\\codex",
            "fingerprint": {
                "volumeSerial": 42,
                "anchorSha256": "a".repeat(64),
                "bundleRelativePath": "codex"
            },
            "ownerPid": 7,
            "managedProcesses": [],
            "issuedAt": "now"
        }))
        .unwrap();
        assert_eq!(lease.fingerprint.anchor_schema, ANCHOR_SCHEMA_V1);
    }

    #[test]
    fn process_scope_uses_pid_and_image_marker() {
        assert_eq!(
            executable_name(r"C:\Programs\CodexSwitch.exe"),
            Some("codexswitch.exe".to_string())
        );
        assert_eq!(
            executable_name("/opt/codex/Codex.exe"),
            Some("codex.exe".to_string())
        );
        assert!(executable_name("Codex.exe").is_some());
        assert!(executable_name("C:\\Programs\\").is_none());
    }

    #[test]
    fn local_guard_refresh_compares_content_not_presence() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-guard-refresh-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.exe");
        let target = root.join("target.exe");
        fs::write(&source, b"new-manager-binary").unwrap();
        assert!(!guard_binary_is_current(&source, &target).unwrap());
        fs::write(&target, b"old-manager-binary").unwrap();
        assert!(!guard_binary_is_current(&source, &target).unwrap());
        fs::write(&target, b"new-manager-binary").unwrap();
        assert!(guard_binary_is_current(&source, &target).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
