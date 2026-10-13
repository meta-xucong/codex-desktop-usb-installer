//! Signed remote license status checks for removable Codex bundles.
//!
//! The server response is accepted only after its original payload bytes have
//! been verified with an embedded Ed25519 public key. Network/parse/signature
//! failures are deliberately represented as offline and can never create a
//! revoked receipt or start cleanup.

#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
#[cfg(not(windows))]
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::atomic_file;
use codex_usb_license::{
    fingerprint_digest, license_signature_sha256, verify_registration,
    RevocationRegistrationEnvelope, UsbFingerprint, UsbLicense, PRODUCT_ID,
    REVOCATION_REGISTRATION_FILE_NAME,
};

const STATE_SCHEMA_VERSION: u32 = 1;
const STATE_DIR_NAME: &str = "CodexUsbState";
const STATE_FILE_NAME: &str = "revocation.json";
const LOCAL_STATE_DIR_NAME: &str = "revocation";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_STATE_FILE_BYTES: usize = 256 * 1024;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECEIPTS_PER_STATE_FILE: usize = 32;
const MAX_MERGED_RECEIPTS: usize = MAX_RECEIPTS_PER_STATE_FILE * 2;
const MAX_COMPACTED_RECEIPTS: usize = 4;
const MAX_PENDING_REVOKED_INSTALLATIONS: usize = 16;
pub(crate) const MAX_ATOMIC_STATE_TEMPS: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const REGISTRATION_MAX_BYTES: u64 = 16 * 1024;
const STATE_LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const INSTALL_TRANSACTION_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Serializes every read/refresh/merge/write transaction for one USB license.
/// The Windows handle is intentionally thread-affine because mutex ownership
/// must be released by the thread that acquired it.
#[cfg(windows)]
pub(crate) struct RevocationStateLock {
    handle: *mut std::ffi::c_void,
    installation_id: String,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Serializes a complete update/revocation-cleanup transaction for one USB
/// installation. It is deliberately distinct from the shorter state lock so
/// network downloads never hold the state mutex.
#[cfg(windows)]
pub(crate) struct InstallationTransactionGuard {
    handle: *mut std::ffi::c_void,
    installation_id: String,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl InstallationTransactionGuard {
    pub(crate) fn installation_id(&self) -> &str {
        &self.installation_id
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *const std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
impl Drop for RevocationStateLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
impl Drop for InstallationTransactionGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct RevocationStateLock {
    file: File,
    installation_id: String,
}

#[cfg(not(windows))]
pub(crate) struct InstallationTransactionGuard {
    file: File,
    installation_id: String,
}

#[cfg(not(windows))]
impl Drop for RevocationStateLock {
    fn drop(&mut self) {
        let _ = fs4::FileExt::unlock(&self.file);
    }
}

#[cfg(not(windows))]
impl Drop for InstallationTransactionGuard {
    fn drop(&mut self) {
        let _ = fs4::FileExt::unlock(&self.file);
    }
}

fn state_lock_key(installation_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"codex-usb-revocation-state-lock\0");
    hash.update((installation_id.len() as u64).to_le_bytes());
    hash.update(installation_id.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn installation_transaction_lock_key(installation_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"codex-usb-installation-transaction-lock\0");
    hash.update((installation_id.len() as u64).to_le_bytes());
    hash.update(installation_id.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn installation_transaction_mutex_name(installation_id: &str) -> String {
    format!(
        "Global\\CodexUsbInstallTransaction-{}",
        installation_transaction_lock_key(installation_id)
    )
}

pub(crate) fn acquire_installation_transaction_guard(
    installation_id: &str,
) -> Result<InstallationTransactionGuard, String> {
    acquire_installation_transaction_guard_with_timeout(
        installation_id,
        INSTALL_TRANSACTION_LOCK_TIMEOUT,
    )
}

/// Production updater entry point. Keep it separate from cleanup so the
/// cross-process test can verify that the two real call sites share one key.
pub(crate) fn acquire_codex_update_transaction_lock(
    installation_id: &str,
) -> Result<InstallationTransactionGuard, String> {
    acquire_installation_transaction_guard(installation_id)
}

/// Production revoked-cleanup entry point. It deliberately resolves to the
/// same installation-scoped OS lock as updater apply.
pub(crate) fn acquire_revocation_cleanup_transaction_lock(
    installation_id: &str,
) -> Result<InstallationTransactionGuard, String> {
    acquire_installation_transaction_guard(installation_id)
}

fn acquire_installation_transaction_guard_with_timeout(
    installation_id: &str,
    timeout: Duration,
) -> Result<InstallationTransactionGuard, String> {
    #[cfg(windows)]
    {
        let name: Vec<u16> = installation_transaction_mutex_name(installation_id)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "无法创建 USB 安装事务互斥锁：{}",
                std::io::Error::last_os_error()
            ));
        }
        let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
        let status = unsafe { WaitForSingleObject(handle, millis) };
        if status != 0 && status != 0x80 {
            unsafe { CloseHandle(handle) };
            if status == 0x102 {
                return Err("等待 USB 安装事务互斥锁超时，拒绝继续修改或清理".to_string());
            }
            return Err(format!(
                "等待 USB 安装事务互斥锁失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(InstallationTransactionGuard {
            handle,
            installation_id: installation_id.to_string(),
            _thread: std::marker::PhantomData,
        })
    }
    #[cfg(not(windows))]
    {
        let key = installation_transaction_lock_key(installation_id);
        let directory = std::env::temp_dir().join("codex-usb-installation-transaction-locks");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建 USB 安装事务锁目录：{error}"))?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{key}.lock")))
            .map_err(|error| format!("无法打开 USB 安装事务锁文件：{error}"))?;
        let started = Instant::now();
        loop {
            match fs4::FileExt::try_lock(&file) {
                Ok(()) => {
                    return Ok(InstallationTransactionGuard {
                        file,
                        installation_id: installation_id.to_string(),
                    })
                }
                Err(fs4::TryLockError::WouldBlock) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(fs4::TryLockError::WouldBlock) => {
                    return Err("等待 USB 安装事务互斥锁超时，拒绝继续修改或清理".to_string());
                }
                Err(fs4::TryLockError::Error(error)) => {
                    return Err(format!("等待 USB 安装事务互斥锁失败：{error}"));
                }
            }
        }
    }
}

fn state_mutex_name(installation_id: &str) -> String {
    format!(
        "Global\\CodexUsbRevocationState-{}",
        state_lock_key(installation_id)
    )
}

fn acquire_state_lock(installation_id: &str) -> Result<RevocationStateLock, String> {
    acquire_state_lock_with_timeout(installation_id, STATE_LOCK_TIMEOUT)
}

pub(crate) fn acquire_cleanup_state_lock(
    license: &UsbLicense,
) -> Result<RevocationStateLock, String> {
    acquire_state_lock(&license.installation_id)
}

fn acquire_state_lock_with_timeout(
    installation_id: &str,
    timeout: Duration,
) -> Result<RevocationStateLock, String> {
    #[cfg(windows)]
    {
        let name: Vec<u16> = state_mutex_name(installation_id)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "无法创建授权状态互斥锁：{}",
                std::io::Error::last_os_error()
            ));
        }
        let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
        let status = unsafe { WaitForSingleObject(handle, millis) };
        // WAIT_OBJECT_0 and WAIT_ABANDONED both grant ownership. An abandoned
        // lock is recoverable because every state copy is atomically written
        // and the next transaction merges/verifies both copies from scratch.
        if status != 0 && status != 0x80 {
            unsafe { CloseHandle(handle) };
            if status == 0x102 {
                return Err("等待授权状态互斥锁超时，未读取或写入状态".to_string());
            }
            return Err(format!(
                "等待授权状态互斥锁失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(RevocationStateLock {
            handle,
            installation_id: installation_id.to_string(),
            _thread: std::marker::PhantomData,
        })
    }
    #[cfg(not(windows))]
    {
        let key = state_lock_key(installation_id);
        let directory = std::env::temp_dir().join("codex-usb-revocation-locks");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建授权状态互斥锁目录：{error}"))?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{key}.lock")))
            .map_err(|error| format!("无法打开授权状态互斥锁文件：{error}"))?;
        let started = Instant::now();
        loop {
            match fs4::FileExt::try_lock(&file) {
                Ok(()) => {
                    return Ok(RevocationStateLock {
                        file,
                        installation_id: installation_id.to_string(),
                    })
                }
                Err(fs4::TryLockError::WouldBlock) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(fs4::TryLockError::WouldBlock) => {
                    return Err("等待授权状态互斥锁超时，未读取或写入状态".to_string());
                }
                Err(fs4::TryLockError::Error(error)) => {
                    return Err(format!("等待授权状态互斥锁失败：{error}"));
                }
            }
        }
    }
}

const SERVICE_URL: Option<&str> = option_env!("CODEX_USB_REVOCATION_URL");
const STATUS_KEYRING_JSON: Option<&str> = option_env!("CODEX_USB_REVOCATION_KEYRING_JSON");

#[cfg(test)]
thread_local! {
    static TEST_STATUS_PUBLIC_KEYS: RefCell<BTreeMap<String, String>> = RefCell::new(BTreeMap::new());
}

#[cfg(test)]
pub(crate) fn with_test_status_public_key<T>(
    key_id: &str,
    public_key_b64: &str,
    action: impl FnOnce() -> T,
) -> T {
    TEST_STATUS_PUBLIC_KEYS.with(|keys| {
        keys.borrow_mut()
            .insert(key_id.to_string(), public_key_b64.to_string());
    });
    let result = action();
    TEST_STATUS_PUBLIC_KEYS.with(|keys| {
        keys.borrow_mut().remove(key_id);
    });
    result
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedEnvelope {
    pub payload_b64: String,
    pub signature_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusPayload {
    pub schema_version: u32,
    pub product: String,
    pub installation_id: String,
    pub license_signature_sha256: String,
    pub request_id: String,
    pub status: String,
    pub state_version: u64,
    pub issued_at: String,
    pub signing_key_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckRequest {
    schema_version: u32,
    request_id: String,
    product: String,
    installation_id: String,
    license_signature_sha256: String,
    registration: RevocationRegistrationEnvelope,
    observed_fingerprint: UsbFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevocationStateFile {
    schema_version: u32,
    product: String,
    installation_id: String,
    license_signature_sha256: String,
    fingerprint_sha256: String,
    receipts: Vec<SignedEnvelope>,
    #[serde(default)]
    cleanup: CleanupProgress,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupProgress {
    pub payload_removed: bool,
    pub runtime_removed: bool,
    pub staging_removed: bool,
    pub last_error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationPolicy {
    LegacyUnenrolled,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    LegacyUnenrolled,
    ActiveOnline { state_version: u64 },
    ActiveOffline { state_version: Option<u64> },
    Revoked { state_version: u64 },
    StatusConflict { state_version: u64 },
    RevokedPersistencePending { state_version: u64, message: String },
    RegistrationRequiredRepair,
}

/// Locally verified authorization view used before the manager performs any
/// startup side effects. This snapshot never makes an online claim: active
/// receipts are reported as offline until `check_startup` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSnapshot {
    pub policy: RegistrationPolicy,
    pub decision: Option<Decision>,
    pub had_verified_revoked_history: bool,
    pub highest_verified_revoked_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedState {
    pub payload: StatusPayload,
    pub envelope: SignedEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingRevocationIdentity {
    pub installation_id: String,
    pub license_signature_sha256: String,
    pub fingerprint: UsbFingerprint,
}

#[derive(Debug, Clone)]
struct PendingRevoked {
    identity: PendingRevocationIdentity,
    envelope: SignedEnvelope,
    state_version: u64,
}

static PENDING_REVOKED: OnceLock<Mutex<BTreeMap<String, PendingRevoked>>> = OnceLock::new();
static PENDING_REVOKED_OVERFLOW: AtomicBool = AtomicBool::new(false);

fn pending_revoked_map() -> &'static Mutex<BTreeMap<String, PendingRevoked>> {
    PENDING_REVOKED.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
static TEST_PENDING_STATE_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_LOCAL_STATE_BASE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_local_state_base<T>(base: &Path, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_LOCAL_STATE_BASE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = TEST_LOCAL_STATE_BASE.with(|slot| slot.borrow_mut().replace(base.to_path_buf()));
    let restore = Restore(previous);
    let result = action();
    drop(restore);
    result
}

#[cfg(test)]
thread_local! {
    static TEST_PERSIST_FAILURE_MASK: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

pub(crate) fn has_pending_revocation() -> bool {
    if PENDING_REVOKED_OVERFLOW.load(Ordering::SeqCst) {
        return true;
    }
    !pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_empty()
}

pub(crate) fn pending_revocation_identity_for_installation(
    installation_id: &str,
) -> Option<PendingRevocationIdentity> {
    pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(installation_id)
        .map(|pending| pending.identity.clone())
}

#[cfg(test)]
pub(crate) fn set_test_pending_revocation(
    identity: PendingRevocationIdentity,
    envelope: SignedEnvelope,
    state_version: u64,
) {
    pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(
            identity.installation_id.clone(),
            PendingRevoked {
                identity,
                envelope,
                state_version,
            },
        );
}

#[cfg(test)]
pub(crate) fn clear_test_pending_revocation(installation_id: &str) {
    pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(installation_id);
}

fn pending_revocation_for(
    license: &UsbLicense,
    signature_digest: &str,
    fingerprint: &UsbFingerprint,
) -> Result<Option<PendingRevoked>, String> {
    let slot = pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(pending) = slot.get(&license.installation_id) else {
        if PENDING_REVOKED_OVERFLOW.load(Ordering::SeqCst) {
            return Err("待持久化作废状态达到容量上限，当前会话保持关闭".to_string());
        }
        return Ok(None);
    };
    if pending.identity.license_signature_sha256 != signature_digest
        || pending.identity.fingerprint != *fingerprint
    {
        return Err("当前许可证或 USB 指纹与进程内待持久化作废回执不匹配".to_string());
    }
    Ok(Some(pending.clone()))
}

fn remember_pending_revocation(
    license: &UsbLicense,
    signature_digest: &str,
    fingerprint: &UsbFingerprint,
    envelope: &SignedEnvelope,
    state_version: u64,
) -> Result<(), String> {
    let mut slot = pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let identity = PendingRevocationIdentity {
        installation_id: license.installation_id.clone(),
        license_signature_sha256: signature_digest.to_string(),
        fingerprint: fingerprint.clone(),
    };
    match slot.get(&license.installation_id) {
        Some(existing) if existing.identity != identity => {
            Err("另一枚 USB 已有待持久化作废状态；当前许可证继续保持锁定".to_string())
        }
        Some(_) => Ok(()),
        None if slot.len() >= MAX_PENDING_REVOKED_INSTALLATIONS => {
            PENDING_REVOKED_OVERFLOW.store(true, Ordering::SeqCst);
            Err(format!(
                "待持久化作废状态超过 {} 个安装身份的安全上限；不覆盖已有状态，Codex 保持锁定",
                MAX_PENDING_REVOKED_INSTALLATIONS
            ))
        }
        None => {
            slot.insert(
                license.installation_id.clone(),
                PendingRevoked {
                    identity,
                    envelope: envelope.clone(),
                    state_version,
                },
            );
            Ok(())
        }
    }
}

fn clear_pending_revocation(pending: &PendingRevoked) {
    let mut slot = pending_revoked_map()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if slot
        .get(&pending.identity.installation_id)
        .is_some_and(|current| {
            current.identity == pending.identity && current.envelope == pending.envelope
        })
    {
        slot.remove(&pending.identity.installation_id);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyringDocument {
    keys: BTreeMap<String, String>,
}

pub fn policy(root: &Path) -> Result<RegistrationPolicy, String> {
    let root_manifest: serde_json::Value = read_manifest(root.join("manifest.json"))?;
    let usb_manifest: serde_json::Value = read_manifest(root.join("usb-manifest.json"))?;
    let root_version = root_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "根清单版本无效".to_string())?;
    let usb_version = usb_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "USB 清单版本无效".to_string())?;
    if root_version != usb_version {
        return Err("根清单与 USB 清单版本不一致".to_string());
    }
    match root_version {
        2 => Ok(RegistrationPolicy::LegacyUnenrolled),
        3 => {
            if root_manifest
                .pointer("/revocationPolicy/registrationRequired")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
            {
                return Err("schemaVersion=3 必须锚定 registrationRequired=true".to_string());
            }
            Ok(RegistrationPolicy::Required)
        }
        other => Err(format!("不支持的 USB manifest 文档版本：{other}")),
    }
}

pub fn load_registration(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Result<RevocationRegistrationEnvelope, String> {
    let path = root.join(REVOCATION_REGISTRATION_FILE_NAME);
    reject_reparse_path(&path)?;
    let bytes = read_bounded_file(&path, REGISTRATION_MAX_BYTES as usize, "登记声明")?;
    let envelope: RevocationRegistrationEnvelope =
        serde_json::from_slice(&bytes).map_err(|error| format!("登记声明格式无效：{error}"))?;
    let public_key = crate::app::usb_guard::license_public_key_for(license)?
        .ok_or_else(|| "许可证没有受信任的签名公钥".to_string())?;
    verify_registration(&envelope, license, fingerprint, &public_key)
        .map_err(|error| format!("登记声明验证失败：{error}"))?;
    Ok(envelope)
}

/// Load both signed receipt copies, make the fail-closed version decision,
/// and optionally issue a fresh online check. Only a fully verified `revoked`
/// response is persisted as a new receipt.
pub fn check_startup(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Result<Decision, String> {
    check_startup_with_url(root, license, fingerprint, SERVICE_URL)
}

pub fn check_startup_with_url(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    service_url: Option<&str>,
) -> Result<Decision, String> {
    check_startup_with_url_timeout(root, license, fingerprint, service_url, STATE_LOCK_TIMEOUT)
}

/// Read and verify the current persisted receipts without contacting the
/// status service. The same per-installation lock as `check_startup` prevents
/// a manager startup from observing a partially merged pair of state files.
pub fn local_snapshot(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Result<LocalSnapshot, String> {
    let _state_lock = acquire_state_lock(&license.installation_id)?;
    let policy = policy(root)?;
    if policy == RegistrationPolicy::LegacyUnenrolled {
        return Ok(LocalSnapshot {
            policy,
            decision: Some(Decision::LegacyUnenrolled),
            had_verified_revoked_history: false,
            highest_verified_revoked_version: None,
        });
    }

    let signature_digest = license_signature_sha256(license).map_err(|error| error.to_string())?;
    let fingerprint_sha = fingerprint_digest(fingerprint).map_err(|error| error.to_string())?;
    let previous = read_merged_state(
        &root.join(STATE_DIR_NAME).join(STATE_FILE_NAME),
        &local_state_path(&license.installation_id)?,
        license,
        &signature_digest,
        &fingerprint_sha,
    )?;
    let resolution = resolve_receipts_detailed(
        previous
            .as_ref()
            .map(|state| state.receipts.as_slice())
            .unwrap_or(&[]),
        license,
        fingerprint,
    );
    let had_verified_revoked_history = previous
        .as_ref()
        .is_some_and(|state| has_verified_revoked_history(state, license, fingerprint));
    let highest_verified_revoked_version = previous.as_ref().and_then(|state| {
        state
            .receipts
            .iter()
            .filter_map(|envelope| {
                verify_persisted_envelope(envelope, license, &signature_digest)
                    .ok()
                    .filter(|verified| verified.payload.status == "revoked")
                    .map(|verified| verified.payload.state_version)
            })
            .max()
    });
    let decision = resolution.map(|(state, conflict)| {
        if conflict {
            Decision::StatusConflict {
                state_version: state.payload.state_version,
            }
        } else if state.payload.status == "revoked" {
            Decision::Revoked {
                state_version: state.payload.state_version,
            }
        } else {
            Decision::ActiveOffline {
                state_version: Some(state.payload.state_version),
            }
        }
    });
    Ok(LocalSnapshot {
        policy,
        decision,
        had_verified_revoked_history,
        highest_verified_revoked_version,
    })
}

fn check_startup_with_url_timeout(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    service_url: Option<&str>,
    lock_timeout: Duration,
) -> Result<Decision, String> {
    let state_lock = acquire_state_lock_with_timeout(&license.installation_id, lock_timeout)?;
    let policy = policy(root)?;
    if policy == RegistrationPolicy::LegacyUnenrolled {
        return Ok(Decision::LegacyUnenrolled);
    }

    let signature_digest = license_signature_sha256(license).map_err(|error| error.to_string())?;
    let fingerprint_sha = fingerprint_digest(fingerprint).map_err(|error| error.to_string())?;
    let usb_state = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
    let local_state = local_state_path(&license.installation_id)?;
    let pending = pending_revocation_for(license, &signature_digest, fingerprint)?;
    let mut previous = read_merged_state(
        &usb_state,
        &local_state,
        license,
        &signature_digest,
        &fingerprint_sha,
    )?;
    if let Some(pending) = pending.as_ref() {
        let verified = verify_persisted_envelope(&pending.envelope, license, &signature_digest)?;
        if verified.payload.status != "revoked"
            || verified.payload.state_version != pending.state_version
        {
            return Err("进程内待持久化回执不再是原始已验签 revoked 状态".to_string());
        }
        previous = Some(merge_online_state(
            previous,
            &verified,
            license,
            &signature_digest,
            &fingerprint_sha,
        )?);
    }
    let previous_resolution = resolve_receipts_detailed(
        previous
            .as_ref()
            .map(|state| state.receipts.as_slice())
            .unwrap_or(&[]),
        license,
        fingerprint,
    );
    let had_revoked = previous_resolution
        .as_ref()
        .is_some_and(|(state, _)| state.payload.status == "revoked");

    let registration = match load_registration(root, license, fingerprint) {
        Ok(value) => value,
        Err(_) => {
            return Ok(if pending.is_some() {
                persist_pending_local_state(
                    root,
                    &usb_state,
                    &local_state,
                    previous,
                    &state_lock,
                    pending.as_ref().expect("pending checked"),
                    true,
                    license,
                    fingerprint,
                )
            } else {
                if had_revoked {
                    prior_offline_decision(previous_resolution.as_ref())
                } else {
                    Decision::RegistrationRequiredRepair
                }
            });
        }
    };

    let Some(service_url) = service_url else {
        return Ok(if let Some(pending) = pending.as_ref() {
            persist_pending_local_state(
                root,
                &usb_state,
                &local_state,
                previous,
                &state_lock,
                pending,
                false,
                license,
                fingerprint,
            )
        } else {
            prior_offline_decision(previous_resolution.as_ref())
        });
    };
    let Ok(url) = url::Url::parse(service_url) else {
        return Ok(if let Some(pending) = pending.as_ref() {
            persist_pending_local_state(
                root,
                &usb_state,
                &local_state,
                previous,
                &state_lock,
                pending,
                false,
                license,
                fingerprint,
            )
        } else {
            prior_offline_decision(previous_resolution.as_ref())
        });
    };
    if !service_url_is_allowed(&url) {
        return Ok(if let Some(pending) = pending.as_ref() {
            persist_pending_local_state(
                root,
                &usb_state,
                &local_state,
                previous,
                &state_lock,
                pending,
                false,
                license,
                fingerprint,
            )
        } else {
            prior_offline_decision(previous_resolution.as_ref())
        });
    }

    let online = check_online(&url, license, fingerprint, &signature_digest, registration);
    let Ok(online) = online else {
        return Ok(if let Some(pending) = pending.as_ref() {
            persist_pending_local_state(
                root,
                &usb_state,
                &local_state,
                previous,
                &state_lock,
                pending,
                false,
                license,
                fingerprint,
            )
        } else {
            prior_offline_decision(previous_resolution.as_ref())
        });
    };

    let state = merge_online_state(
        previous,
        &online,
        license,
        &signature_digest,
        &fingerprint_sha,
    )?;
    match persist_both(root, &usb_state, &local_state, &state, &state_lock) {
        Ok(()) => {
            if let Some(pending) = pending.as_ref() {
                clear_pending_revocation(pending);
            }
            Ok(decision_for_persisted_state(
                &state,
                license,
                fingerprint,
                true,
            ))
        }
        Err(error) => {
            if online.payload.status == "revoked" || pending.is_some() {
                let pending_error = remember_pending_revocation(
                    license,
                    &signature_digest,
                    fingerprint,
                    &online.envelope,
                    online.payload.state_version,
                )
                .err();
                return Ok(Decision::RevokedPersistencePending {
                    state_version: pending
                        .as_ref()
                        .map_or(online.payload.state_version, |pending| {
                            pending.state_version
                        }),
                    message: pending_error.unwrap_or(error),
                });
            }
            Err(error)
        }
    }
}

fn service_url_is_allowed(url: &url::Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    #[cfg(test)]
    if url.scheme() == "http" {
        return url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    }
    false
}

fn persist_pending_local_state(
    root: &Path,
    usb_state: &Path,
    local_state: &Path,
    state: Option<RevocationStateFile>,
    state_lock: &RevocationStateLock,
    pending: &PendingRevoked,
    registration_repair_required: bool,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Decision {
    let Some(state) = state else {
        return Decision::RevokedPersistencePending {
            state_version: pending.state_version,
            message: "待持久化作废状态缺少可验证的本地合并状态".to_string(),
        };
    };
    if let Err(message) = persist_both(root, usb_state, local_state, &state, state_lock) {
        return Decision::RevokedPersistencePending {
            state_version: pending.state_version,
            message,
        };
    }
    clear_pending_revocation(pending);
    let decision = decision_for_persisted_state(&state, license, fingerprint, false);
    if registration_repair_required
        && !matches!(
            decision,
            Decision::Revoked { .. } | Decision::StatusConflict { .. }
        )
    {
        Decision::RegistrationRequiredRepair
    } else {
        decision
    }
}

fn retry_pending_revocation_locked(
    root: &Path,
    usb_state: &Path,
    local_state: &Path,
    local_base: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    signature_digest: &str,
    fingerprint_sha: &str,
    state_lock: &RevocationStateLock,
    pending: &PendingRevoked,
) -> Decision {
    retry_pending_revocation_locked_with_local_base(
        root,
        usb_state,
        local_state,
        local_base,
        license,
        fingerprint,
        signature_digest,
        fingerprint_sha,
        state_lock,
        pending,
    )
}

fn retry_pending_revocation_locked_with_local_base(
    root: &Path,
    usb_state: &Path,
    local_state: &Path,
    local_base: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    signature_digest: &str,
    fingerprint_sha: &str,
    state_lock: &RevocationStateLock,
    pending: &PendingRevoked,
) -> Decision {
    let result = (|| {
        let verified = verify_persisted_envelope(&pending.envelope, license, signature_digest)?;
        if verified.payload.status != "revoked"
            || verified.payload.state_version != pending.state_version
        {
            return Err("进程内待持久化回执不再是原始已验签 revoked 状态".to_string());
        }
        let previous = read_merged_state(
            usb_state,
            local_state,
            license,
            signature_digest,
            fingerprint_sha,
        )?;
        let state = merge_online_state(
            previous,
            &verified,
            license,
            signature_digest,
            fingerprint_sha,
        )?;
        persist_both_with_local_base(root, usb_state, local_state, &state, state_lock, local_base)?;
        Ok(state)
    })();

    match result {
        Ok(state) => {
            clear_pending_revocation(pending);
            decision_for_persisted_state(&state, license, fingerprint, false)
        }
        Err(message) => Decision::RevokedPersistencePending {
            state_version: pending.state_version,
            message,
        },
    }
}

fn decision_for_persisted_state(
    state: &RevocationStateFile,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    online: bool,
) -> Decision {
    match resolve_receipts_detailed(&state.receipts, license, fingerprint) {
        Some((state, true)) => Decision::StatusConflict {
            state_version: state.payload.state_version,
        },
        Some((state, false)) if state.payload.status == "revoked" => Decision::Revoked {
            state_version: state.payload.state_version,
        },
        Some((state, false)) if online => Decision::ActiveOnline {
            state_version: state.payload.state_version,
        },
        Some((state, false)) => Decision::ActiveOffline {
            state_version: Some(state.payload.state_version),
        },
        None => Decision::ActiveOffline {
            state_version: None,
        },
    }
}

fn merge_online_state(
    previous: Option<RevocationStateFile>,
    online: &VerifiedState,
    license: &UsbLicense,
    signature_digest: &str,
    fingerprint_sha: &str,
) -> Result<RevocationStateFile, String> {
    let cleanup = previous
        .as_ref()
        .map(|state| state.cleanup.clone())
        .unwrap_or_default();
    let mut receipts = previous.map(|state| state.receipts).unwrap_or_default();
    receipts.push(online.envelope.clone());
    let mut state = RevocationStateFile {
        schema_version: STATE_SCHEMA_VERSION,
        product: PRODUCT_ID.to_string(),
        installation_id: license.installation_id.clone(),
        license_signature_sha256: signature_digest.to_string(),
        fingerprint_sha256: fingerprint_sha.to_string(),
        receipts,
        cleanup,
    };
    state.receipts = compact_receipts(&state.receipts, license, signature_digest)?;
    Ok(state)
}

fn prior_offline_decision(resolution: Option<&(VerifiedState, bool)>) -> Decision {
    match resolution {
        Some((state, true)) => Decision::StatusConflict {
            state_version: state.payload.state_version,
        },
        Some((state, false)) if state.payload.status == "revoked" => Decision::Revoked {
            state_version: state.payload.state_version,
        },
        Some((state, false)) => Decision::ActiveOffline {
            state_version: Some(state.payload.state_version),
        },
        None => Decision::ActiveOffline {
            state_version: None,
        },
    }
}

fn check_online(
    base_url: &url::Url,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    signature_digest: &str,
    registration: RevocationRegistrationEnvelope,
) -> Result<VerifiedState, String> {
    let request_id = new_request_id()?;
    let request = CheckRequest {
        schema_version: 1,
        request_id: request_id.clone(),
        product: PRODUCT_ID.to_string(),
        installation_id: license.installation_id.clone(),
        license_signature_sha256: signature_digest.to_string(),
        registration,
        observed_fingerprint: fingerprint.clone(),
    };
    let request_bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    if request_bytes.len() > MAX_RESPONSE_BYTES {
        return Err("授权检查请求过大".to_string());
    }
    let endpoint = base_url
        .join("/v1/licenses/check")
        .map_err(|error| error.to_string())?;
    let client = status_http_client()?;
    let mut response = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::CACHE_CONTROL, "no-store")
        .body(request_bytes)
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("授权服务返回 HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("授权服务响应过大".to_string());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("授权服务响应过大".to_string());
    }
    let envelope: SignedEnvelope =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    verify_status_envelope(&envelope, &request_id, license, signature_digest)
}

fn status_http_client() -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder.build().map_err(|error| error.to_string())
}

pub fn verify_status_envelope(
    envelope: &SignedEnvelope,
    request_id: &str,
    license: &UsbLicense,
    signature_digest: &str,
) -> Result<VerifiedState, String> {
    let payload_bytes = BASE64
        .decode(envelope.payload_b64.trim())
        .map_err(|error| format!("状态 payload Base64 无效：{error}"))?;
    if payload_bytes.len() > MAX_RESPONSE_BYTES {
        return Err("授权状态 payload 过大".to_string());
    }
    let signature_bytes = BASE64
        .decode(envelope.signature_b64.trim())
        .map_err(|error| format!("状态签名 Base64 无效：{error}"))?;
    let signature_array: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "状态签名长度无效".to_string())?;
    let signature = Signature::from_bytes(&signature_array);
    let payload: StatusPayload = serde_json::from_slice(&payload_bytes)
        .map_err(|error| format!("状态 payload JSON 无效：{error}"))?;
    let key = status_public_key(&payload.signing_key_id)?;
    verify_status_payload_bytes(
        &payload_bytes,
        &signature,
        request_id,
        &license.installation_id,
        signature_digest,
        &key,
    )?;
    Ok(VerifiedState {
        payload,
        envelope: envelope.clone(),
    })
}

fn verify_status_payload_bytes(
    payload_bytes: &[u8],
    signature: &Signature,
    request_id: &str,
    installation_id: &str,
    signature_digest: &str,
    key: &VerifyingKey,
) -> Result<(), String> {
    if payload_bytes.len() > MAX_RESPONSE_BYTES {
        return Err("授权状态 payload 过大".to_string());
    }
    let payload: StatusPayload = serde_json::from_slice(payload_bytes)
        .map_err(|error| format!("状态 payload JSON 无效：{error}"))?;
    if payload.schema_version != 1
        || payload.product != PRODUCT_ID
        || payload.installation_id != installation_id
        || payload.license_signature_sha256 != signature_digest
        || payload.request_id != request_id
        || payload.request_id.len() != 32
        || !payload
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(payload.status.as_str(), "active" | "revoked")
        || payload.state_version == 0
        || payload.signing_key_id.is_empty()
    {
        return Err("状态响应与当前许可证或请求不匹配".to_string());
    }
    key.verify(payload_bytes, signature)
        .map_err(|_| "状态响应签名无效".to_string())?;
    Ok(())
}

fn status_public_key(key_id: &str) -> Result<VerifyingKey, String> {
    #[cfg(test)]
    if let Some(encoded) = TEST_STATUS_PUBLIC_KEYS.with(|keys| keys.borrow().get(key_id).cloned()) {
        let bytes = BASE64
            .decode(encoded.trim())
            .map_err(|error| format!("授权状态测试公钥 Base64 无效：{error}"))?;
        let bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "授权状态测试公钥长度无效".to_string())?;
        return VerifyingKey::from_bytes(&bytes).map_err(|error| error.to_string());
    }
    let raw = STATUS_KEYRING_JSON.ok_or_else(|| "构建未配置授权状态验签公钥环".to_string())?;
    let keyring: KeyringDocument =
        serde_json::from_str(raw).map_err(|error| format!("内置授权状态公钥环无效：{error}"))?;
    let encoded = keyring
        .keys
        .get(key_id)
        .ok_or_else(|| format!("不受信任的授权状态 keyId：{key_id}"))?;
    let bytes = BASE64
        .decode(encoded.trim())
        .map_err(|error| format!("授权状态公钥 Base64 无效：{error}"))?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "授权状态公钥长度无效".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|error| error.to_string())
}

fn read_merged_state(
    usb_path: &Path,
    local_path: &Path,
    license: &UsbLicense,
    signature_digest: &str,
    fingerprint_sha: &str,
) -> Result<Option<RevocationStateFile>, String> {
    let mut valid = Vec::new();
    for path in [usb_path, local_path] {
        let Some(state) = read_state_file(path)? else {
            continue;
        };
        if state.schema_version != STATE_SCHEMA_VERSION
            || state.product != PRODUCT_ID
            || state.installation_id != license.installation_id
            || state.license_signature_sha256 != signature_digest
            || state.fingerprint_sha256 != fingerprint_sha
        {
            continue;
        }
        let receipts = compact_receipts(&state.receipts, license, signature_digest)?;
        if receipts.is_empty() {
            continue;
        }
        valid.push(RevocationStateFile { receipts, ..state });
    }
    let Some(first) = valid.first().cloned() else {
        return Ok(None);
    };
    let mut merged = first;
    for state in valid.iter().skip(1) {
        merged.receipts.extend(state.receipts.clone());
        merged.cleanup.payload_removed |= state.cleanup.payload_removed;
        merged.cleanup.runtime_removed |= state.cleanup.runtime_removed;
        merged.cleanup.staging_removed |= state.cleanup.staging_removed;
        if merged.cleanup.last_error.is_none() {
            merged.cleanup.last_error = state.cleanup.last_error.clone();
        }
    }
    merged.receipts = compact_receipts(&merged.receipts, license, signature_digest)?;
    if merged.receipts.len() > MAX_COMPACTED_RECEIPTS {
        return Err("授权状态包含过多签名回执，已停止启动".to_string());
    }
    Ok(Some(merged))
}

fn read_state_file(path: &Path) -> Result<Option<RevocationStateFile>, String> {
    reject_reparse_path(path)?;
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取授权状态失败：{error}")),
    };
    if file
        .metadata()
        .map_err(|error| format!("检查授权状态大小失败：{error}"))?
        .len()
        > MAX_STATE_FILE_BYTES as u64
    {
        return Err("授权状态文件超过大小上限，已停止启动".to_string());
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_STATE_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取授权状态失败：{error}"))?;
    if bytes.len() > MAX_STATE_FILE_BYTES {
        return Err("授权状态文件超过大小上限，已停止启动".to_string());
    }
    let Ok(state) = serde_json::from_slice::<RevocationStateFile>(&bytes) else {
        return Ok(None);
    };
    if state.receipts.len() > MAX_RECEIPTS_PER_STATE_FILE {
        return Err("授权状态回执数量超过上限，已停止启动".to_string());
    }
    Ok(Some(state))
}

fn resolve_receipts(
    envelopes: &[SignedEnvelope],
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Option<VerifiedState> {
    resolve_receipts_detailed(envelopes, license, fingerprint).map(|(state, _)| state)
}

fn resolve_receipts_detailed(
    envelopes: &[SignedEnvelope],
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Option<(VerifiedState, bool)> {
    let signature_digest = license_signature_sha256(license).ok()?;
    if !codex_usb_license::fingerprints_match(&license.fingerprint, fingerprint) {
        return None;
    }
    resolve_receipts_detailed_with(envelopes, &license.installation_id, |envelope| {
        verify_persisted_envelope(envelope, license, &signature_digest)
    })
}

fn resolve_receipts_detailed_with(
    envelopes: &[SignedEnvelope],
    installation_id: &str,
    mut verify: impl FnMut(&SignedEnvelope) -> Result<VerifiedState, String>,
) -> Option<(VerifiedState, bool)> {
    let valid = envelopes
        .iter()
        .filter_map(|envelope| verify(envelope).ok())
        .filter(|checked| {
            checked.payload.product == PRODUCT_ID
                && checked.payload.installation_id == installation_id
        })
        .collect::<Vec<_>>();
    resolve_verified_states_detailed(valid)
}

fn resolve_verified_states_detailed(
    mut valid: Vec<VerifiedState>,
) -> Option<(VerifiedState, bool)> {
    valid.sort_by_key(|state| state.payload.state_version);
    let highest_version = valid.last()?.payload.state_version;
    let at_version = valid
        .into_iter()
        .filter(|state| state.payload.state_version == highest_version)
        .collect::<Vec<_>>();
    let revoked = at_version
        .iter()
        .any(|state| state.payload.status == "revoked");
    let conflict = revoked
        && at_version
            .iter()
            .any(|state| state.payload.status == "active");
    let selected = at_version
        .into_iter()
        .find(|state| state.payload.status == if revoked { "revoked" } else { "active" })?;
    Some((selected, conflict))
}

fn compact_receipts(
    envelopes: &[SignedEnvelope],
    license: &UsbLicense,
    signature_digest: &str,
) -> Result<Vec<SignedEnvelope>, String> {
    if envelopes.len() > MAX_MERGED_RECEIPTS {
        return Err("授权状态签名回执数量超过上限，已停止启动".to_string());
    }
    let valid = envelopes
        .iter()
        .filter_map(|envelope| verify_persisted_envelope(envelope, license, signature_digest).ok())
        .collect::<Vec<_>>();
    compact_verified_receipts(valid)
}

fn compact_verified_receipts(states: Vec<VerifiedState>) -> Result<Vec<SignedEnvelope>, String> {
    let highest_version = states.iter().map(|state| state.payload.state_version).max();
    let highest_revoked_version = states
        .iter()
        .filter(|state| state.payload.status == "revoked")
        .map(|state| state.payload.state_version)
        .max();
    let mut by_version_and_status = BTreeMap::<(u64, String), SignedEnvelope>::new();
    for state in states {
        let version = state.payload.state_version;
        let status = state.payload.status.clone();
        if Some(version) == highest_version
            || (status == "revoked" && Some(version) == highest_revoked_version)
        {
            by_version_and_status
                .entry((version, status))
                .or_insert(state.envelope);
        }
    }
    let compacted = by_version_and_status.into_values().collect::<Vec<_>>();
    if compacted.len() > MAX_COMPACTED_RECEIPTS {
        return Err("授权状态回执压缩结果超过上限，已停止启动".to_string());
    }
    Ok(compacted)
}

fn verify_persisted_envelope(
    envelope: &SignedEnvelope,
    license: &UsbLicense,
    signature_digest: &str,
) -> Result<VerifiedState, String> {
    let payload_bytes = BASE64
        .decode(envelope.payload_b64.trim())
        .map_err(|error| error.to_string())?;
    if payload_bytes.len() > MAX_RESPONSE_BYTES {
        return Err("保存的授权回执过大".to_string());
    }
    let payload: StatusPayload =
        serde_json::from_slice(&payload_bytes).map_err(|error| error.to_string())?;
    verify_status_envelope(envelope, &payload.request_id, license, signature_digest)
}

fn persist_both(
    root: &Path,
    usb_path: &Path,
    local_path: &Path,
    state: &RevocationStateFile,
    state_lock: &RevocationStateLock,
) -> Result<(), String> {
    let local_base = local_state_base()?;
    persist_both_with_local_base(root, usb_path, local_path, state, state_lock, &local_base)
}

fn persist_both_with_local_base(
    root: &Path,
    usb_path: &Path,
    local_path: &Path,
    state: &RevocationStateFile,
    state_lock: &RevocationStateLock,
    local_base: &Path,
) -> Result<(), String> {
    if state.installation_id != state_lock.installation_id {
        return Err("授权状态互斥锁与待写入安装身份不匹配".to_string());
    }
    if state.receipts.len() > MAX_COMPACTED_RECEIPTS {
        return Err("授权状态回执数量超过写入上限".to_string());
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_STATE_FILE_BYTES {
        return Err("授权状态文件超过写入大小上限".to_string());
    }
    ensure_safe_parent(root, usb_path)?;
    let usb_result = write_state_copy(usb_path, &bytes, 0)
        .map_err(|error| format!("U 盘授权状态保存失败：{error}"));
    ensure_local_state_parent_under(local_path, local_base)?;
    let local_result = write_state_copy(local_path, &bytes, 1)
        .map_err(|error| format!("本机授权状态保存失败：{error}"));
    usb_result?;
    local_result?;
    Ok(())
}

fn write_state_copy(path: &Path, bytes: &[u8], _copy_index: u8) -> std::io::Result<()> {
    #[cfg(test)]
    {
        let should_fail = TEST_PERSIST_FAILURE_MASK.with(|mask| {
            let bit = 1_u8 << _copy_index;
            if mask.get() & bit == 0 {
                false
            } else {
                mask.set(mask.get() & !bit);
                true
            }
        });
        if should_fail {
            return Err(std::io::Error::other(format!(
                "injected state copy failure {_copy_index}"
            )));
        }
    }
    atomic_file::write_atomic_preserving_backup(path, bytes)
}

pub fn write_cleanup_progress(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    progress: CleanupProgress,
) -> Result<(), String> {
    let state_lock = acquire_state_lock(&license.installation_id)?;
    write_cleanup_progress_locked(root, license, fingerprint, progress, &state_lock)
}

pub(crate) fn write_cleanup_progress_locked(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    progress: CleanupProgress,
    state_lock: &RevocationStateLock,
) -> Result<(), String> {
    if state_lock.installation_id != license.installation_id {
        return Err("授权状态互斥锁与清理安装身份不匹配".to_string());
    }
    let signature_digest = license_signature_sha256(license).map_err(|error| error.to_string())?;
    let fingerprint_sha = fingerprint_digest(fingerprint).map_err(|error| error.to_string())?;
    let usb_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
    let local_path = local_state_path(&license.installation_id)?;
    let mut state = read_merged_state(
        &usb_path,
        &local_path,
        license,
        &signature_digest,
        &fingerprint_sha,
    )?
    .ok_or_else(|| "没有可验证的授权状态凭证".to_string())?;
    state.cleanup = progress;
    persist_both(root, &usb_path, &local_path, &state, state_lock)
}

pub fn merged_state(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Option<RevocationStateFile> {
    let signature_digest = license_signature_sha256(license).ok()?;
    let fingerprint_sha = fingerprint_digest(fingerprint).ok()?;
    read_merged_state(
        &root.join(STATE_DIR_NAME).join(STATE_FILE_NAME),
        &local_state_path(&license.installation_id).ok()?,
        license,
        &signature_digest,
        &fingerprint_sha,
    )
    .ok()?
}

pub fn highest_state(
    state: &RevocationStateFile,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Option<VerifiedState> {
    resolve_receipts(&state.receipts, license, fingerprint)
}

pub fn has_verified_revoked_history(
    state: &RevocationStateFile,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> bool {
    if !codex_usb_license::fingerprints_match(&license.fingerprint, fingerprint) {
        return false;
    }
    let Ok(signature_digest) = license_signature_sha256(license) else {
        return false;
    };
    state.receipts.iter().any(|envelope| {
        verify_persisted_envelope(envelope, license, &signature_digest)
            .is_ok_and(|state| state.payload.status == "revoked")
    })
}

pub fn read_merged_state_for_recovery(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Result<RevocationStateFile, String> {
    let signature_digest = license_signature_sha256(license).map_err(|error| error.to_string())?;
    let fingerprint_sha = fingerprint_digest(fingerprint).map_err(|error| error.to_string())?;
    read_merged_state(
        &root.join(STATE_DIR_NAME).join(STATE_FILE_NAME),
        &local_state_path(&license.installation_id)?,
        license,
        &signature_digest,
        &fingerprint_sha,
    )?
    .ok_or_else(|| "没有可验证的授权状态回执".to_string())
}

fn local_state_path(installation_id: &str) -> Result<PathBuf, String> {
    let base = local_state_base()?;
    Ok(base
        .join("CodexUsbGuard")
        .join(LOCAL_STATE_DIR_NAME)
        .join(format!("{installation_id}.json")))
}

pub(crate) fn is_atomic_state_tmp_relative(relative: &str) -> bool {
    let Some((parent, name)) = relative.rsplit_once('/') else {
        return false;
    };
    parent.eq_ignore_ascii_case(STATE_DIR_NAME) && is_atomic_state_tmp_name(name)
}

pub(crate) fn is_atomic_state_tmp_name(name: &str) -> bool {
    let Some(suffix) = name
        .to_ascii_lowercase()
        .strip_prefix("revocation.json.tmp-")
        .map(str::to_owned)
    else {
        return false;
    };
    let mut pieces = suffix.split('-');
    let (Some(pid), Some(counter), None) = (pieces.next(), pieces.next(), pieces.next()) else {
        return false;
    };
    pid.parse::<u32>()
        .is_ok_and(|value| value > 0 && value.to_string() == pid)
        && counter
            .parse::<u64>()
            .is_ok_and(|value| value > 0 && value.to_string() == counter)
}

fn ensure_safe_parent(root: &Path, path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "状态文件路径无父目录".to_string())?;
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| "状态目录不在指定根目录内".to_string())?;
    let mut current = canonical_root.clone();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_reparse_metadata(&metadata) => {
                return Err("状态目录包含符号链接".to_string())
            }
            Ok(metadata) if !metadata.is_dir() => return Err("状态目录路径不是目录".to_string()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    if let Some(path_parent) = path.parent() {
        fs::create_dir_all(path_parent).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn reject_reparse_path(path: &Path) -> Result<(), String> {
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_reparse_metadata(&metadata) => {
                return Err("授权状态路径包含符号链接".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    Ok(())
}

fn local_state_base() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(base) = TEST_LOCAL_STATE_BASE.with(|slot| slot.borrow().clone()) {
        return Ok(base);
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        Ok(PathBuf::from(path))
    } else {
        directories::BaseDirs::new()
            .map(|dirs| dirs.data_local_dir().to_path_buf())
            .ok_or_else(|| "无法定位本机授权状态目录".to_string())
    }
}

fn ensure_local_state_parent_under(path: &Path, local_base: &Path) -> Result<(), String> {
    let canonical_base = fs::canonicalize(local_base).map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "授权状态路径无父目录".to_string())?;
    let relative = parent
        .strip_prefix(local_base)
        .map_err(|_| "本机状态目录超出 LocalAppData".to_string())?;
    let mut current = canonical_base;
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_reparse_metadata(&metadata) => {
                return Err("本机授权状态目录包含重解析点".to_string())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("本机授权状态路径不是目录".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_metadata(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn read_manifest(path: PathBuf) -> Result<serde_json::Value, String> {
    reject_reparse_path(&path)?;
    let bytes = read_bounded_file(&path, MAX_MANIFEST_BYTES, "USB 清单")?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn read_bounded_file(path: &Path, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|error| format!("读取{label}失败：{error}"))?;
    if file
        .metadata()
        .map_err(|error| format!("读取{label}属性失败：{error}"))?
        .len()
        > max_bytes as u64
    {
        return Err(format!("{label}文件过大"));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取{label}失败：{error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{label}文件过大"));
    }
    Ok(bytes)
}

fn new_request_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    fill_csprng(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
fn fill_csprng(output: &mut [u8]) -> Result<(), String> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn SystemFunction036(random_buffer: *mut u8, random_buffer_length: u32) -> u8;
    }
    let length = u32::try_from(output.len()).map_err(|_| "随机数请求长度无效".to_string())?;
    // RtlGenRandom/SystemFunction036 uses the Windows system-preferred CSPRNG.
    if unsafe { SystemFunction036(output.as_mut_ptr(), length) } == 0 {
        return Err(format!(
            "Windows 系统随机数生成失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn fill_csprng(output: &mut [u8]) -> Result<(), String> {
    let mut source = File::open("/dev/urandom").map_err(|error| error.to_string())?;
    source
        .read_exact(output)
        .map_err(|error| format!("系统随机数生成失败：{error}"))
}

#[cfg(not(any(windows, unix)))]
fn fill_csprng(_output: &mut [u8]) -> Result<(), String> {
    Err("当前平台没有配置系统 CSPRNG".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::Write;

    fn decode_base32_secret(encoded: &str) -> Vec<u8> {
        let mut output = Vec::new();
        let mut buffer = 0_u32;
        let mut bits = 0_u8;
        for byte in encoded.bytes() {
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'2'..=b'7' => byte - b'2' + 26,
                b'=' => break,
                _ => panic!("invalid Base32 character in local admin setup secret"),
            };
            buffer = (buffer << 5) | u32::from(value);
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                output.push((buffer >> bits) as u8);
            }
        }
        output
    }

    fn totp_code(secret: &str, unix_seconds: u64) -> String {
        use sha1::{Digest as _, Sha1};

        let mut key = decode_base32_secret(secret);
        assert!(
            key.len() >= 20,
            "local TOTP secret must be at least 20 bytes"
        );
        if key.len() > 64 {
            key = Sha1::digest(&key).to_vec();
        }
        let counter = (unix_seconds / 30).to_be_bytes();
        let mut normalized_key = [0_u8; 64];
        normalized_key[..key.len()].copy_from_slice(&key);
        let mut inner_pad = [0x36_u8; 64];
        let mut outer_pad = [0x5c_u8; 64];
        for index in 0..64 {
            inner_pad[index] ^= normalized_key[index];
            outer_pad[index] ^= normalized_key[index];
        }

        let mut inner = Sha1::new();
        inner.update(inner_pad);
        inner.update(counter);
        let inner_digest = inner.finalize();
        let mut outer = Sha1::new();
        outer.update(outer_pad);
        outer.update(inner_digest);
        let digest = outer.finalize();
        let offset = usize::from(digest[19] & 0x0f);
        let value = (u32::from(digest[offset] & 0x7f) << 24)
            | (u32::from(digest[offset + 1]) << 16)
            | (u32::from(digest[offset + 2]) << 8)
            | u32::from(digest[offset + 3]);
        format!("{:06}", value % 1_000_000)
    }

    fn login_admin(
        http: &reqwest::blocking::Client,
        origin: &str,
        password: &str,
        stage: &str,
    ) -> (String, String) {
        let login = http
            .post(format!("{origin}/admin/api/login"))
            .header(reqwest::header::ORIGIN, origin)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "username": "e2e-admin",
                    "password": password
                }))
                .unwrap(),
            )
            .send()
            .unwrap();
        assert_eq!(
            login.status(),
            reqwest::StatusCode::OK,
            "admin login {stage} failed: {}",
            login.text().unwrap()
        );
        let session_cookie = login
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let login_body: serde_json::Value = serde_json::from_str(&login.text().unwrap()).unwrap();
        let csrf = login_body["csrfToken"].as_str().unwrap().to_string();
        (session_cookie, csrf)
    }

    fn test_license(installation_id: &str) -> UsbLicense {
        UsbLicense {
            schema_version: codex_usb_license::LICENSE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            installation_id: installation_id.to_string(),
            fingerprint: UsbFingerprint {
                volume_serial: 0x1234_abcd,
                anchor_sha256: "ab".repeat(32),
                bundle_relative_path: "CodexUSB".to_string(),
                anchor_schema: 3,
            },
            issued_at: "1780000000000".to_string(),
            key_id: Some("test-license-key".to_string()),
            account_binding: None,
            signature: Some(BASE64.encode([0x31_u8; 64])),
        }
    }

    fn signed_status(
        signing_key: &SigningKey,
        license: &UsbLicense,
        signature_digest: &str,
        status: &str,
        state_version: u64,
    ) -> SignedEnvelope {
        signed_status_for_request_id(
            signing_key,
            license,
            signature_digest,
            &format!("{state_version:032x}"),
            status,
            state_version,
        )
    }

    fn signed_status_for_request_id(
        signing_key: &SigningKey,
        license: &UsbLicense,
        signature_digest: &str,
        request_id: &str,
        status: &str,
        state_version: u64,
    ) -> SignedEnvelope {
        let payload = StatusPayload {
            schema_version: 1,
            product: PRODUCT_ID.to_string(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: signature_digest.to_string(),
            request_id: request_id.to_string(),
            status: status.to_string(),
            state_version,
            issued_at: "2026-10-09T00:00:00Z".to_string(),
            signing_key_id: "test-status-key".to_string(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let signature = signing_key.sign(&payload_bytes);
        SignedEnvelope {
            payload_b64: BASE64.encode(payload_bytes),
            signature_b64: BASE64.encode(signature.to_bytes()),
        }
    }

    fn write_registered_schema3_fixture(
        root: &Path,
        license: &UsbLicense,
        registration_key: &SigningKey,
    ) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("manifest.json"),
            br#"{"schemaVersion":3,"revocationPolicy":{"registrationRequired":true}}"#,
        )
        .unwrap();
        fs::write(root.join("usb-manifest.json"), br#"{"schemaVersion":3}"#).unwrap();
        let registration = codex_usb_license::sign_registration(license, registration_key).unwrap();
        fs::write(
            root.join(REVOCATION_REGISTRATION_FILE_NAME),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
    }

    fn serve_signed_status_once(
        listener: std::net::TcpListener,
        signing_key: SigningKey,
        license: UsbLicense,
        signature_digest: String,
        status: &'static str,
        state_version: u64,
    ) -> std::thread::JoinHandle<()> {
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let accept_deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < accept_deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("loopback service did not receive a request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            let header_end = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "request ended before headers");
                request.extend_from_slice(&buffer[..read]);
                if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
                assert!(request.len() <= 16 * 1024, "request headers too large");
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("request content length");
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "request ended before body");
                request.extend_from_slice(&buffer[..read]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
            let request_id = body["requestId"].as_str().expect("request ID");
            let envelope = signed_status_for_request_id(
                &signing_key,
                &license,
                &signature_digest,
                request_id,
                status,
                state_version,
            );
            let body = serde_json::to_vec(&envelope).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        })
    }

    fn with_status_test_key<T>(signing_key: &SigningKey, action: impl FnOnce() -> T) -> T {
        with_test_status_public_key(
            "test-status-key",
            &BASE64.encode(signing_key.verifying_key().to_bytes()),
            action,
        )
    }

    #[test]
    fn request_nonce_is_128_bits_and_hex() {
        let values = (0..8)
            .map(|_| new_request_id().unwrap())
            .collect::<Vec<_>>();
        assert!(values.iter().all(|value| value.len() == 32));
        assert!(values
            .iter()
            .all(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit())));
        // UUIDv4 fixes the version and variant bits, so this catches a return
        // to UUID formatting instead of exposing all 16 CSPRNG bytes as hex.
        assert!(values.iter().any(|value| value.as_bytes()[12] != b'4'));
        assert!(values
            .iter()
            .any(|value| !matches!(value.as_bytes()[16], b'8'..=b'b')));
    }

    #[test]
    fn state_lock_serializes_same_installation_but_not_other_installations() {
        let installation_id = format!("lock-test-{}", uuid::Uuid::new_v4());
        let same_installation_guard =
            acquire_state_lock_with_timeout(&installation_id, Duration::from_secs(2)).unwrap();

        let worker_id = installation_id.clone();
        let same_id_result = std::thread::spawn(move || {
            acquire_state_lock_with_timeout(&worker_id, Duration::from_millis(120)).map(|_| ())
        })
        .join()
        .unwrap();
        assert!(same_id_result.err().unwrap().contains("互斥锁超时"));

        let other_guard = acquire_state_lock_with_timeout(
            &format!("other-{installation_id}"),
            Duration::from_millis(120),
        )
        .expect("different installation IDs must not share a lock");
        drop(other_guard);
        drop(same_installation_guard);
    }

    #[test]
    fn transaction_lock_child_entry() {
        let Ok(mode) = std::env::var("CODEX_USB_TXN_TEST_CHILD_MODE") else {
            return;
        };
        let directory = PathBuf::from(
            std::env::var_os("CODEX_USB_TXN_TEST_CHILD_DIR").expect("child directory"),
        );
        let installation_id =
            std::env::var("CODEX_USB_TXN_TEST_CHILD_ID").expect("child installation ID");
        let guard = if mode.starts_with("cleanup") {
            acquire_revocation_cleanup_transaction_lock(&installation_id)
        } else {
            acquire_codex_update_transaction_lock(&installation_id)
        }
        .expect("child should acquire transaction lock");
        let ready = directory.join(format!("{mode}.ready"));
        fs::write(&ready, b"ready").unwrap();
        match mode.as_str() {
            "update-hold" | "cleanup-hold" => {
                let release = directory.join(format!("{mode}.release"));
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while !release.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(release.exists(), "parent did not release child transaction");
            }
            "cleanup-delete" => {
                fs::write(
                    directory.join("delete.marker"),
                    b"cleanup entered after lock",
                )
                .unwrap();
            }
            "update-recheck" => {
                let revoked = directory.join("revoked.marker");
                let result = if revoked.exists() {
                    b"denied".as_slice()
                } else {
                    b"allowed".as_slice()
                };
                fs::write(directory.join("update.result"), result).unwrap();
            }
            other => panic!("unknown child mode {other}"),
        }
        drop(guard);
    }

    fn spawn_transaction_lock_child(
        directory: &Path,
        installation_id: &str,
        mode: &str,
    ) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("app::usb_revocation::tests::transaction_lock_child_entry")
            .arg("--nocapture")
            .env("CODEX_USB_TXN_TEST_CHILD_MODE", mode)
            .env("CODEX_USB_TXN_TEST_CHILD_DIR", directory)
            .env("CODEX_USB_TXN_TEST_CHILD_ID", installation_id)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    fn wait_for_test_file(path: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {}", path.display());
    }

    #[test]
    fn installation_transaction_guard_serializes_update_and_cleanup_across_processes() {
        let directory = std::env::temp_dir().join(format!(
            "usb-install-transaction-lock-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let installation_id = format!("txn-process-{}", uuid::Uuid::new_v4());

        // An update process holds the actual OS transaction lock. A cleanup
        // contender must time out before its deletion phase can mutate anything.
        let mut update = spawn_transaction_lock_child(&directory, &installation_id, "update-hold");
        wait_for_test_file(&directory.join("update-hold.ready"));
        let mut cleanup =
            spawn_transaction_lock_child(&directory, &installation_id, "cleanup-delete");
        std::thread::sleep(Duration::from_millis(150));
        assert!(!directory.join("delete.marker").exists());

        // A different installation remains independent while the first lock is held.
        let other =
            acquire_revocation_cleanup_transaction_lock(&format!("other-{installation_id}"))
                .expect("different installation must not block");
        drop(other);
        fs::write(directory.join("update-hold.release"), b"release").unwrap();
        assert!(update.wait().unwrap().success());
        assert!(cleanup.wait().unwrap().success());
        assert!(directory.join("delete.marker").exists());

        // Cleanup holds the lock while the status changes to revoked. The
        // waiting updater rechecks only after acquiring it and must be denied.
        let mut cleanup =
            spawn_transaction_lock_child(&directory, &installation_id, "cleanup-hold");
        wait_for_test_file(&directory.join("cleanup-hold.ready"));
        fs::write(directory.join("revoked.marker"), b"signed revoked fixture").unwrap();
        let mut waiting_update =
            spawn_transaction_lock_child(&directory, &installation_id, "update-recheck");
        std::thread::sleep(Duration::from_millis(150));
        assert!(!directory.join("update.result").exists());
        fs::write(directory.join("cleanup-hold.release"), b"release").unwrap();
        assert!(cleanup.wait().unwrap().success());
        assert!(waiting_update.wait().unwrap().success());
        assert_eq!(
            fs::read(directory.join("update.result")).unwrap(),
            b"denied"
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn cleanup_lock_blocks_a_new_active_check_until_cleanup_finishes() {
        let installation_id = format!("cleanup-active-race-{}", uuid::Uuid::new_v4());
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let cleanup_lock = acquire_cleanup_state_lock(&license).unwrap();
        let worker_license = license.clone();
        let blocked = std::thread::spawn(move || {
            check_startup_with_url_timeout(
                Path::new("unused-until-lock-releases"),
                &worker_license,
                &worker_license.fingerprint,
                None,
                Duration::from_millis(120),
            )
        })
        .join()
        .unwrap();
        assert!(blocked.unwrap_err().contains("互斥锁超时"));

        // Once cleanup releases the installation lock, a status check can
        // enter its transaction and fail later on the deliberately absent
        // fixture path. It can never pass the active decision mid-cleanup.
        drop(cleanup_lock);
        let after_cleanup = check_startup_with_url_timeout(
            Path::new("unused-until-lock-releases"),
            &license,
            &fingerprint,
            None,
            Duration::from_secs(1),
        );
        assert!(after_cleanup.is_err());
        assert!(!after_cleanup.unwrap_err().contains("互斥锁超时"));
    }

    #[test]
    fn pending_revoked_stays_blocked_until_both_state_copies_are_persisted() {
        let _pending_test_guard = TEST_PENDING_STATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-pending-retry-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&local_base).unwrap();

        let installation_id = format!("pending-test-{}", uuid::Uuid::new_v4());
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let signature_digest = license_signature_sha256(&license).unwrap();
        let fingerprint_sha = fingerprint_digest(&fingerprint).unwrap();
        let usb_state = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
        let local_state = local_base
            .join("CodexUsbGuard")
            .join(LOCAL_STATE_DIR_NAME)
            .join(format!("{installation_id}.json"));
        let signing_key = SigningKey::from_bytes(&[0x61; 32]);

        with_status_test_key(&signing_key, || {
            let envelope = signed_status(&signing_key, &license, &signature_digest, "revoked", 41);
            remember_pending_revocation(&license, &signature_digest, &fingerprint, &envelope, 41)
                .unwrap();
            let pending = pending_revocation_for(&license, &signature_digest, &fingerprint)
                .unwrap()
                .unwrap();

            TEST_PERSIST_FAILURE_MASK.with(|mask| mask.set(0b11));
            let first_lock = acquire_state_lock(&installation_id).unwrap();
            let first = retry_pending_revocation_locked_with_local_base(
                &root,
                &usb_state,
                &local_state,
                &local_base,
                &license,
                &fingerprint,
                &signature_digest,
                &fingerprint_sha,
                &first_lock,
                &pending,
            );
            assert!(matches!(
                first,
                Decision::RevokedPersistencePending {
                    state_version: 41,
                    ..
                }
            ));
            assert!(has_pending_revocation());
            assert!(!usb_state.exists());
            assert!(!local_state.exists());
            drop(first_lock);

            let retry_lock = acquire_state_lock(&installation_id).unwrap();
            let still_pending = pending_revocation_for(&license, &signature_digest, &fingerprint)
                .unwrap()
                .unwrap();
            let recovered = retry_pending_revocation_locked_with_local_base(
                &root,
                &usb_state,
                &local_state,
                &local_base,
                &license,
                &fingerprint,
                &signature_digest,
                &fingerprint_sha,
                &retry_lock,
                &still_pending,
            );
            assert_eq!(recovered, Decision::Revoked { state_version: 41 });
            assert!(!has_pending_revocation());
            for path in [&usb_state, &local_state] {
                let persisted: RevocationStateFile =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                assert_eq!(persisted.receipts, vec![envelope.clone()]);
                assert_eq!(
                    highest_state(&persisted, &license, &fingerprint)
                        .unwrap()
                        .payload
                        .status,
                    "revoked"
                );
            }
        });

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_revoked_is_rechecked_online_and_higher_active_is_persisted_without_cleanup() {
        let _pending_test_guard = TEST_PENDING_STATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-pending-online-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        let kept_codex_file = root.join("apps/Codex/keep.txt");
        fs::create_dir_all(kept_codex_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&local_base).unwrap();

        let installation_id = uuid::Uuid::new_v4().to_string();
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let signature_digest = license_signature_sha256(&license).unwrap();
        let fingerprint_sha = fingerprint_digest(&fingerprint).unwrap();
        let registration_key = SigningKey::from_bytes(&[0x71; 32]);
        let status_key = SigningKey::from_bytes(&[0x72; 32]);
        write_registered_schema3_fixture(&root, &license, &registration_key);
        fs::write(&kept_codex_file, b"cleanup is not run by status check").unwrap();

        let revoked_v2 = signed_status(&status_key, &license, &signature_digest, "revoked", 2);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = serve_signed_status_once(
            listener,
            status_key.clone(),
            license.clone(),
            signature_digest.clone(),
            "active",
            3,
        );

        crate::app::usb_guard::with_test_license_public_key(
            "test-license-key",
            &BASE64.encode(registration_key.verifying_key().to_bytes()),
            || {
                with_status_test_key(&status_key, || {
                    remember_pending_revocation(
                        &license,
                        &signature_digest,
                        &fingerprint,
                        &revoked_v2,
                        2,
                    )
                    .unwrap();
                    let decision = with_test_local_state_base(&local_base, || {
                        check_startup_with_url(&root, &license, &fingerprint, Some(&endpoint))
                    })
                    .unwrap();
                    assert_eq!(decision, Decision::ActiveOnline { state_version: 3 });
                    assert!(
                        pending_revocation_identity_for_installation(&installation_id).is_none()
                    );

                    let usb_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
                    let local_path = local_base
                        .join("CodexUsbGuard")
                        .join(LOCAL_STATE_DIR_NAME)
                        .join(format!("{installation_id}.json"));
                    for path in [&usb_path, &local_path] {
                        let state: RevocationStateFile =
                            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                        let resolved =
                            resolve_receipts_detailed(&state.receipts, &license, &fingerprint)
                                .unwrap();
                        assert_eq!(resolved.0.payload.status, "active");
                        assert_eq!(resolved.0.payload.state_version, 3);
                        assert!(state.receipts.iter().any(|receipt| {
                            verify_persisted_envelope(receipt, &license, &signature_digest)
                                .is_ok_and(|checked| {
                                    checked.payload.status == "revoked"
                                        && checked.payload.state_version == 2
                                })
                        }));
                    }
                    assert_eq!(
                        fs::read(&kept_codex_file).unwrap(),
                        b"cleanup is not run by status check"
                    );
                })
            },
        );
        server.join().unwrap();
        clear_test_pending_revocation(&installation_id);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fresh_signed_revoked_response_is_persisted_without_cleanup_side_effect() {
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-fresh-revoked-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        let sentinel = root.join("apps/Codex/keep.txt");
        fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        fs::create_dir_all(&local_base).unwrap();
        fs::write(&sentinel, b"status check must not delete files").unwrap();

        let installation_id = uuid::Uuid::new_v4().to_string();
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let signature_digest = license_signature_sha256(&license).unwrap();
        let registration_key = SigningKey::from_bytes(&[0x61; 32]);
        let status_key = SigningKey::from_bytes(&[0x62; 32]);
        write_registered_schema3_fixture(&root, &license, &registration_key);

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = serve_signed_status_once(
            listener,
            status_key.clone(),
            license.clone(),
            signature_digest,
            "revoked",
            12,
        );

        crate::app::usb_guard::with_test_license_public_key(
            "test-license-key",
            &BASE64.encode(registration_key.verifying_key().to_bytes()),
            || {
                with_status_test_key(&status_key, || {
                    let decision = with_test_local_state_base(&local_base, || {
                        check_startup_with_url(&root, &license, &fingerprint, Some(&endpoint))
                    })
                    .unwrap();
                    assert_eq!(decision, Decision::Revoked { state_version: 12 });

                    let usb_state_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
                    let local_state = local_base
                        .join("CodexUsbGuard")
                        .join(LOCAL_STATE_DIR_NAME)
                        .join(format!("{installation_id}.json"));
                    for path in [&usb_state_path, &local_state] {
                        let state: RevocationStateFile =
                            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                        assert_eq!(
                            highest_state(&state, &license, &fingerprint)
                                .unwrap()
                                .payload
                                .status,
                            "revoked"
                        );
                    }
                    assert_eq!(
                        fs::read(&sentinel).unwrap(),
                        b"status check must not delete files",
                        "the status protocol test must not invoke cleanup"
                    );
                })
            },
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fresh_offline_start_without_prior_receipt_fails_open_without_writing_state() {
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-fresh-offline-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        fs::create_dir_all(&local_base).unwrap();
        let installation_id = uuid::Uuid::new_v4().to_string();
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let registration_key = SigningKey::from_bytes(&[0x63; 32]);
        write_registered_schema3_fixture(&root, &license, &registration_key);

        crate::app::usb_guard::with_test_license_public_key(
            "test-license-key",
            &BASE64.encode(registration_key.verifying_key().to_bytes()),
            || {
                let decision = with_test_local_state_base(&local_base, || {
                    check_startup_with_url(&root, &license, &fingerprint, None)
                })
                .unwrap();
                assert_eq!(
                    decision,
                    Decision::ActiveOffline {
                        state_version: None
                    }
                );
            },
        );

        let usb_state_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
        let local_state = local_state_path(&installation_id).unwrap();
        assert!(!usb_state_path.exists());
        assert!(!local_state.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pending_revoked_without_network_uses_highest_verified_local_state() {
        let _pending_test_guard = TEST_PENDING_STATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-pending-offline-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        fs::create_dir_all(&local_base).unwrap();
        let installation_id = uuid::Uuid::new_v4().to_string();
        let license = test_license(&installation_id);
        let fingerprint = license.fingerprint.clone();
        let signature_digest = license_signature_sha256(&license).unwrap();
        let fingerprint_sha = fingerprint_digest(&fingerprint).unwrap();
        let registration_key = SigningKey::from_bytes(&[0x73; 32]);
        let status_key = SigningKey::from_bytes(&[0x74; 32]);
        write_registered_schema3_fixture(&root, &license, &registration_key);
        let revoked_v2 = signed_status(&status_key, &license, &signature_digest, "revoked", 2);
        let active_v3 = signed_status(&status_key, &license, &signature_digest, "active", 3);
        let previous = RevocationStateFile {
            schema_version: STATE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            installation_id: installation_id.clone(),
            license_signature_sha256: signature_digest.clone(),
            fingerprint_sha256: fingerprint_sha,
            receipts: vec![active_v3],
            cleanup: CleanupProgress::default(),
        };

        crate::app::usb_guard::with_test_license_public_key(
            "test-license-key",
            &BASE64.encode(registration_key.verifying_key().to_bytes()),
            || {
                with_status_test_key(&status_key, || {
                    remember_pending_revocation(
                        &license,
                        &signature_digest,
                        &fingerprint,
                        &revoked_v2,
                        2,
                    )
                    .unwrap();
                    with_test_local_state_base(&local_base, || {
                        let state_lock = acquire_state_lock(&installation_id).unwrap();
                        let usb_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
                        let local_path = local_state_path(&installation_id).unwrap();
                        persist_both_with_local_base(
                            &root,
                            &usb_path,
                            &local_path,
                            &previous,
                            &state_lock,
                            &local_base,
                        )
                        .unwrap();
                        drop(state_lock);

                        let decision =
                            check_startup_with_url(&root, &license, &fingerprint, None).unwrap();
                        assert_eq!(
                            decision,
                            Decision::ActiveOffline {
                                state_version: Some(3)
                            }
                        );
                        assert!(
                            pending_revocation_identity_for_installation(&installation_id)
                                .is_none()
                        );
                        for path in [&usb_path, &local_path] {
                            let state: RevocationStateFile =
                                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                            let resolved =
                                resolve_receipts_detailed(&state.receipts, &license, &fingerprint)
                                    .unwrap();
                            assert_eq!(resolved.0.payload.status, "active");
                            assert_eq!(resolved.0.payload.state_version, 3);
                            assert!(state.receipts.iter().any(|receipt| {
                                verify_persisted_envelope(receipt, &license, &signature_digest)
                                    .is_ok_and(|checked| {
                                        checked.payload.status == "revoked"
                                            && checked.payload.state_version == 2
                                    })
                            }));
                        }
                    });
                })
            },
        );
        clear_test_pending_revocation(&installation_id);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pending_revocation_capacity_never_replaces_an_existing_installation() {
        let _pending_test_guard = TEST_PENDING_STATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut slot = pending_revoked_map()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        slot.clear();
        PENDING_REVOKED_OVERFLOW.store(false, Ordering::SeqCst);
        drop(slot);

        let envelope = SignedEnvelope {
            payload_b64: "test payload".to_string(),
            signature_b64: "test signature".to_string(),
        };
        let mut first_identity = None;
        for index in 0..MAX_PENDING_REVOKED_INSTALLATIONS {
            let license = test_license(&format!("capacity-{index}"));
            let digest = license_signature_sha256(&license).unwrap();
            remember_pending_revocation(
                &license,
                &digest,
                &license.fingerprint,
                &envelope,
                index as u64 + 1,
            )
            .unwrap();
            if index == 0 {
                first_identity = Some(pending_revocation_identity_for_installation(
                    &license.installation_id,
                ));
            }
        }

        let overflow_license = test_license("capacity-overflow");
        let overflow_digest = license_signature_sha256(&overflow_license).unwrap();
        assert!(remember_pending_revocation(
            &overflow_license,
            &overflow_digest,
            &overflow_license.fingerprint,
            &envelope,
            999,
        )
        .is_err());
        assert_eq!(
            pending_revoked_map()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .len(),
            MAX_PENDING_REVOKED_INSTALLATIONS
        );
        assert_eq!(
            pending_revocation_identity_for_installation("capacity-0"),
            first_identity.unwrap()
        );
        assert!(has_pending_revocation());
        assert!(pending_revocation_for(
            &overflow_license,
            &overflow_digest,
            &overflow_license.fingerprint
        )
        .is_err());

        pending_revoked_map()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
        PENDING_REVOKED_OVERFLOW.store(false, Ordering::SeqCst);
    }

    #[test]
    fn state_mutex_name_uses_global_namespace_and_installation_hash() {
        let installation_id = "global-mutex-test";
        let name = state_mutex_name(installation_id);
        assert!(name.starts_with("Global\\CodexUsbRevocationState-"));
        assert!(name.ends_with(&state_lock_key(installation_id)));
    }

    #[test]
    fn lower_active_arriving_after_persisted_revoked_cannot_roll_state_back() {
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-version-race-{}",
            uuid::Uuid::new_v4()
        ));
        let local_base = root.join("local-state");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&local_base).unwrap();

        let installation_id = format!("race-test-{}", uuid::Uuid::new_v4());
        let license = test_license(&installation_id);
        let signature_digest = license_signature_sha256(&license).unwrap();
        let fingerprint_sha = fingerprint_digest(&license.fingerprint).unwrap();
        let usb_path = root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
        let local_path = local_base
            .join("CodexUsbGuard")
            .join(LOCAL_STATE_DIR_NAME)
            .join(format!("{installation_id}.json"));
        let signing_key = SigningKey::from_bytes(&[0x51; 32]);

        with_status_test_key(&signing_key, || {
            let high_revoked =
                signed_status(&signing_key, &license, &signature_digest, "revoked", 21);
            let high_state = RevocationStateFile {
                schema_version: STATE_SCHEMA_VERSION,
                product: PRODUCT_ID.to_string(),
                installation_id: installation_id.clone(),
                license_signature_sha256: signature_digest.clone(),
                fingerprint_sha256: fingerprint_sha.clone(),
                receipts: vec![high_revoked],
                cleanup: CleanupProgress::default(),
            };

            // First transaction has completed and persisted the higher revoked
            // state to both copies before the delayed lower active response arrives.
            let first_lock = acquire_state_lock(&installation_id).unwrap();
            persist_both_with_local_base(
                &root,
                &usb_path,
                &local_path,
                &high_state,
                &first_lock,
                &local_base,
            )
            .unwrap();
            drop(first_lock);

            let lower_active =
                signed_status(&signing_key, &license, &signature_digest, "active", 20);
            let lower_payload: StatusPayload =
                serde_json::from_slice(&BASE64.decode(&lower_active.payload_b64).unwrap()).unwrap();
            let lower_verified = verify_status_envelope(
                &lower_active,
                &lower_payload.request_id,
                &license,
                &signature_digest,
            )
            .unwrap();

            // The next serialized transaction reads the previous highest
            // receipt, merges the stale response, and persists its compacted state.
            let second_lock = acquire_state_lock(&installation_id).unwrap();
            let previous = read_merged_state(
                &usb_path,
                &local_path,
                &license,
                &signature_digest,
                &fingerprint_sha,
            )
            .unwrap()
            .expect("previous revoked state must be loaded");
            let merged = merge_online_state(
                Some(previous),
                &lower_verified,
                &license,
                &signature_digest,
                &fingerprint_sha,
            )
            .unwrap();
            persist_both_with_local_base(
                &root,
                &usb_path,
                &local_path,
                &merged,
                &second_lock,
                &local_base,
            )
            .unwrap();
            drop(second_lock);

            let persisted = read_merged_state(
                &usb_path,
                &local_path,
                &license,
                &signature_digest,
                &fingerprint_sha,
            )
            .unwrap()
            .unwrap();
            let highest = resolve_receipts(&persisted.receipts, &license, &license.fingerprint)
                .expect("persisted state must retain a valid receipt");
            assert_eq!(highest.payload.status, "revoked");
            assert_eq!(highest.payload.state_version, 21);
            assert_eq!(persisted.receipts.len(), 1);
        });

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_reads_are_bounded_even_after_the_metadata_check() {
        let dir = std::env::temp_dir().join(format!(
            "usb-revocation-manifest-limit-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.json");
        fs::write(&path, vec![b' '; MAX_MANIFEST_BYTES + 1]).unwrap();
        assert!(read_manifest(path).unwrap_err().contains("文件过大"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn registration_file_read_has_a_hard_byte_limit() {
        let dir = std::env::temp_dir().join(format!(
            "usb-revocation-registration-limit-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(REVOCATION_REGISTRATION_FILE_NAME);
        fs::write(&path, vec![b' '; REGISTRATION_MAX_BYTES as usize + 1]).unwrap();
        let error =
            read_bounded_file(&path, REGISTRATION_MAX_BYTES as usize, "登记声明").unwrap_err();
        assert!(error.contains("登记声明文件过大"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn schema_three_policy_fails_closed_without_true_marker() {
        let root =
            std::env::temp_dir().join(format!("usb-revocation-policy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("manifest.json"),
            br#"{"schemaVersion":3,"revocationPolicy":{"registrationRequired":false}}"#,
        )
        .unwrap();
        fs::write(root.join("usb-manifest.json"), br#"{"schemaVersion":3,"product":"codex-usb-oneclick","revocationPolicy":{"registrationRequired":true}}"#).unwrap();
        assert!(policy(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schema_two_is_legacy_only_when_both_manifests_match() {
        let root =
            std::env::temp_dir().join(format!("usb-revocation-schema2-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("manifest.json"), br#"{"schemaVersion":2}"#).unwrap();
        fs::write(
            root.join("usb-manifest.json"),
            br#"{"schemaVersion":2,"product":"codex-usb-oneclick"}"#,
        )
        .unwrap();
        assert_eq!(policy(&root), Ok(RegistrationPolicy::LegacyUnenrolled));
        fs::write(
            root.join("usb-manifest.json"),
            br#"{"schemaVersion":3,"product":"codex-usb-oneclick"}"#,
        )
        .unwrap();
        assert!(policy(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_verification_uses_original_payload_bytes_and_binds_nonce() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let key = signing_key.verifying_key();
        let payload = StatusPayload {
            schema_version: 1,
            product: PRODUCT_ID.to_string(),
            installation_id: "install-1".to_string(),
            license_signature_sha256: "a".repeat(64),
            request_id: "1a".repeat(16),
            status: "active".to_string(),
            state_version: 2,
            issued_at: "2026-01-01T00:00:00Z".to_string(),
            signing_key_id: "test-key".to_string(),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let signature = signing_key.sign(&bytes);
        assert!(verify_status_payload_bytes(
            &bytes,
            &signature,
            &payload.request_id,
            &payload.installation_id,
            &payload.license_signature_sha256,
            &key,
        )
        .is_ok());
        assert!(verify_status_payload_bytes(
            &bytes,
            &signature,
            &"2b".repeat(16),
            &payload.installation_id,
            &payload.license_signature_sha256,
            &key,
        )
        .is_err());
        let tampered = String::from_utf8(bytes)
            .unwrap()
            .replace("\"active\"", "\"revoked\"")
            .into_bytes();
        assert!(verify_status_payload_bytes(
            &tampered,
            &signature,
            &payload.request_id,
            &payload.installation_id,
            &payload.license_signature_sha256,
            &key,
        )
        .is_err());
    }

    #[test]
    fn corrupt_receipt_does_not_mask_revoked_and_tie_prefers_revoked() {
        let mk = |status: &str, version: u64| VerifiedState {
            payload: StatusPayload {
                schema_version: 1,
                product: PRODUCT_ID.to_string(),
                installation_id: "install-1".to_string(),
                license_signature_sha256: "a".repeat(64),
                request_id: "1a".repeat(16),
                status: status.to_string(),
                state_version: version,
                issued_at: "2026-01-01T00:00:00Z".to_string(),
                signing_key_id: "test-key".to_string(),
            },
            envelope: SignedEnvelope {
                payload_b64: String::new(),
                signature_b64: String::new(),
            },
        };
        let envelopes = [
            SignedEnvelope {
                payload_b64: "corrupt".to_string(),
                signature_b64: String::new(),
            },
            SignedEnvelope {
                payload_b64: "revoked".to_string(),
                signature_b64: String::new(),
            },
            SignedEnvelope {
                payload_b64: "active-tie".to_string(),
                signature_b64: String::new(),
            },
        ];
        let resolved = resolve_receipts_detailed_with(&envelopes, "install-1", |envelope| {
            match envelope.payload_b64.as_str() {
                "corrupt" => Err("bad signature".to_string()),
                "revoked" => Ok(mk("revoked", 9)),
                "active-tie" => Ok(mk("active", 9)),
                _ => unreachable!(),
            }
        })
        .unwrap();
        assert_eq!(resolved.0.payload.status, "revoked");
        assert_eq!(resolved.0.payload.state_version, 9);
        assert!(resolved.1);
        assert_eq!(
            prior_offline_decision(Some(&resolved)),
            Decision::StatusConflict { state_version: 9 }
        );
    }

    #[test]
    fn higher_active_resolves_current_status_without_dropping_history() {
        let active = VerifiedState {
            payload: StatusPayload {
                schema_version: 1,
                product: PRODUCT_ID.to_string(),
                installation_id: "install-1".to_string(),
                license_signature_sha256: "a".repeat(64),
                request_id: "1a".repeat(16),
                status: "active".to_string(),
                state_version: 10,
                issued_at: "2026-01-01T00:00:00Z".to_string(),
                signing_key_id: "test-key".to_string(),
            },
            envelope: SignedEnvelope {
                payload_b64: String::new(),
                signature_b64: String::new(),
            },
        };
        let revoked = active.clone();
        let mut revoked = revoked;
        revoked.payload.status = "revoked".to_string();
        revoked.payload.state_version = 9;
        let resolved = resolve_verified_states_detailed(vec![revoked.clone(), active])
            .unwrap()
            .0;
        assert_eq!(resolved.payload.status, "active");
        assert_eq!(resolved.payload.state_version, 10);
        // The resolver selects the current decision only; the caller retains the
        // complete receipt vector when persisting a higher active response.
        assert_eq!(revoked.payload.status, "revoked");
    }

    #[test]
    fn compaction_deduplicates_and_keeps_highest_active_and_revoked_history() {
        let mk = |status: &str, version: u64, marker: &str| VerifiedState {
            payload: StatusPayload {
                schema_version: 1,
                product: PRODUCT_ID.to_string(),
                installation_id: "install-1".to_string(),
                license_signature_sha256: "a".repeat(64),
                request_id: "1a".repeat(16),
                status: status.to_string(),
                state_version: version,
                issued_at: "2026-01-01T00:00:00Z".to_string(),
                signing_key_id: "test-key".to_string(),
            },
            envelope: SignedEnvelope {
                payload_b64: marker.to_string(),
                signature_b64: marker.to_string(),
            },
        };
        let compacted = compact_verified_receipts(vec![
            mk("active", 1, "old-active"),
            mk("revoked", 4, "cleanup-history"),
            mk("active", 7, "latest-active"),
            mk("active", 7, "duplicate-active"),
            mk("revoked", 4, "duplicate-revoked"),
        ])
        .unwrap();
        assert_eq!(compacted.len(), 2);
        assert!(compacted
            .iter()
            .any(|receipt| receipt.payload_b64 == "latest-active"));
        assert!(compacted
            .iter()
            .any(|receipt| receipt.payload_b64 == "cleanup-history"));
    }

    #[test]
    fn oversized_state_and_receipt_count_fail_closed() {
        let dir = std::env::temp_dir().join(format!(
            "usb-revocation-state-limit-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("revocation.json");
        fs::write(&path, vec![b' '; MAX_STATE_FILE_BYTES + 1]).unwrap();
        assert!(read_state_file(&path).unwrap_err().contains("大小上限"));

        let envelope = SignedEnvelope {
            payload_b64: String::new(),
            signature_b64: String::new(),
        };
        let state = RevocationStateFile {
            schema_version: STATE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            installation_id: "install-limit".to_string(),
            license_signature_sha256: "a".repeat(64),
            fingerprint_sha256: "b".repeat(64),
            receipts: vec![envelope; MAX_RECEIPTS_PER_STATE_FILE + 1],
            cleanup: CleanupProgress::default(),
        };
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(read_state_file(&path)
            .unwrap_err()
            .contains("回执数量超过上限"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn status_http_client_does_not_follow_redirects() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /redirect-target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let response = status_http_client()
            .unwrap()
            .get(format!("http://{address}/first"))
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        server.join().unwrap();
    }

    /// Exercises the whole local control path with a freshly signed registration:
    /// Go admin import -> active startup check -> authenticated double-confirmed
    /// revoke -> next startup check -> the real pinned cleanup implementation.
    /// This is deliberately opt-in because it builds and launches the Go service.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires local Go toolchain; uses an isolated temp bundle and loopback server"]
    fn loopback_admin_revoke_syncs_into_usb_startup_and_cleanup() {
        use std::os::windows::ffi::OsStrExt;
        use std::process::{Child, Command, Stdio};
        use std::thread;
        use std::time::Instant;
        use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};

        struct TempTree(PathBuf);
        impl Drop for TempTree {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let temp = TempTree(std::env::temp_dir().join(format!(
            "codex-usb-revocation-admin-e2e-{}",
            uuid::Uuid::new_v4()
        )));
        let root = temp.0.join("CodexUSB");
        let local_state = temp.0.join("manager-state");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&local_state).unwrap();

        let canonical_root = fs::canonicalize(&root).unwrap();
        let canonical_root_wide = canonical_root
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut drive_root_wide = vec![0_u16; 32768];
        assert_ne!(
            unsafe {
                GetVolumePathNameW(
                    canonical_root_wide.as_ptr(),
                    drive_root_wide.as_mut_ptr(),
                    drive_root_wide.len() as u32,
                )
            },
            0,
            "read temporary fixture volume root"
        );
        let drive_root = String::from_utf16_lossy(
            &drive_root_wide[..drive_root_wide
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(drive_root_wide.len())],
        );
        let drive_root_path = PathBuf::from(&drive_root);
        let bundle_relative_path = canonical_root
            .strip_prefix(&drive_root_path)
            .unwrap()
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .expect("temporary fixture path is UTF-8")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("/");
        let mut volume_serial = 0_u32;
        assert_ne!(
            unsafe {
                GetVolumeInformationW(
                    drive_root_wide.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    &mut volume_serial,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            },
            0,
            "read temporary fixture volume serial"
        );
        let manifest = br#"{"schemaVersion":3,"architecture":"x64","payload":{"path":"payload/OpenAI.Codex_e2e_x64.msix"},"revocationPolicy":{"registrationRequired":true},"updateCapability":{"schemaVersion":3,"anchorSchema":3}}"#;
        let usb_manifest =
            br#"{"schemaVersion":3,"updateCapability":{"schemaVersion":3,"anchorSchema":3}}"#;
        let checksums = b"0000000000000000000000000000000000000000000000000000000000000000  payload/OpenAI.Codex_e2e_x64.msix\n0000000000000000000000000000000000000000000000000000000000000000  manifest.json\n0000000000000000000000000000000000000000000000000000000000000000  usb-manifest.json\n";
        fs::write(canonical_root.join("manifest.json"), manifest).unwrap();
        fs::write(canonical_root.join("usb-manifest.json"), usb_manifest).unwrap();
        fs::write(canonical_root.join("SHA256SUMS.txt"), checksums).unwrap();
        let fingerprint = UsbFingerprint {
            volume_serial,
            anchor_sha256: codex_usb_license::stable_anchor_sha256_from_bytes(
                manifest,
                usb_manifest,
                checksums,
            )
            .unwrap(),
            bundle_relative_path,
            anchor_schema: 3,
        };
        let license_key = SigningKey::from_bytes(&[0x41; 32]);
        let status_key = SigningKey::from_bytes(&[0x42; 32]);
        let mut license =
            codex_usb_license::new_unsigned_license(fingerprint.clone(), 1_780_000_000_000);
        license.key_id = Some("license-e2e-2026".to_string());
        codex_usb_license::sign_license(&mut license, &license_key).unwrap();
        let registration = codex_usb_license::sign_registration(&license, &license_key).unwrap();
        fs::write(
            canonical_root.join("usb-license.json"),
            serde_json::to_vec(&license).unwrap(),
        )
        .unwrap();
        fs::write(
            canonical_root.join(REVOCATION_REGISTRATION_FILE_NAME),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(canonical_root.join("payload")).unwrap();
        fs::write(
            canonical_root.join("payload/OpenAI.Codex_e2e_x64.msix"),
            b"temporary fake MSIX payload",
        )
        .unwrap();
        fs::create_dir_all(canonical_root.join("apps/Codex")).unwrap();
        fs::write(
            canonical_root.join("apps/Codex/Codex.exe"),
            b"temporary fake Codex executable",
        )
        .unwrap();
        fs::write(
            canonical_root.join("apps/Codex/AppxManifest.xml"),
            b"<Package><Applications><Application Executable=\"Codex.exe\" /></Applications></Package>",
        )
        .unwrap();
        fs::write(
            canonical_root.join("user-data-sentinel.txt"),
            b"preserve user data",
        )
        .unwrap();

        let service_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("server/usb-license-revocation");
        let binary = temp.0.join("revocationd.exe");
        let build = Command::new("cmd.exe")
            .current_dir(&service_dir)
            .args(["/D", "/C", "go", "build", "-o"])
            .arg(&binary)
            .arg("./cmd/revocationd")
            .output()
            .expect("run go build for loopback service");
        assert!(
            build.status.success(),
            "go build failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );

        let private_key = temp.0.join("status-signing-key");
        let license_keyring = temp.0.join("license-keyring.json");
        fs::write(&private_key, BASE64.encode(status_key.to_keypair_bytes())).unwrap();
        fs::write(
            &license_keyring,
            serde_json::to_vec(&serde_json::json!({
                "keys": {
                    "license-e2e-2026": BASE64.encode(license_key.verifying_key().to_bytes())
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let origin = format!("http://127.0.0.1:{port}");
        let mut service = ChildGuard(
            Command::new(&binary)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env("USB_REVOCATION_LISTEN_ADDR", format!("127.0.0.1:{port}"))
                .env("USB_REVOCATION_DATABASE", temp.0.join("state.db"))
                .env("USB_REVOCATION_AUDIT", temp.0.join("audit.jsonl"))
                .env(
                    "USB_REVOCATION_AUDIT_BACKUP",
                    temp.0.join("audit-backup.jsonl"),
                )
                .env(
                    "USB_REVOCATION_RESTORE_MARKER",
                    temp.0.join("restore-in-progress"),
                )
                .env("USB_REVOCATION_LICENSE_KEYRING", &license_keyring)
                .env("USB_REVOCATION_SIGNING_KEY", &private_key)
                .env("USB_REVOCATION_SIGNING_KEY_ID", "revocation-e2e-2026")
                .env("USB_REVOCATION_ADMIN_USERNAME", "e2e-admin")
                .env("USB_REVOCATION_ADMIN_PASSWORD", "local-e2e-password-123")
                .env(
                    "USB_REVOCATION_ADMIN_AUTH_STATE",
                    temp.0.join("admin-auth.json"),
                )
                .env_remove("USB_REVOCATION_ADMIN_TOTP_SECRET")
                .env("USB_REVOCATION_ADMIN_ORIGIN", &origin)
                .env("USB_REVOCATION_ADMIN_COOKIE_SECURE", "false")
                .spawn()
                .expect("start loopback revocation service"),
        );
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let health_url = format!("{origin}/healthz");
        let ready_deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if http
                .get(&health_url)
                .send()
                .is_ok_and(|response| response.status().is_success())
            {
                break;
            }
            if let Some(status) = service.0.try_wait().unwrap() {
                panic!("loopback service exited before health check: {status}");
            }
            assert!(
                Instant::now() < ready_deadline,
                "loopback service did not become ready"
            );
            thread::sleep(Duration::from_millis(50));
        }

        let (bootstrap_cookie, bootstrap_csrf) =
            login_admin(&http, &origin, "local-e2e-password-123", "bootstrap");
        let setup = http
            .get(format!("{origin}/admin/api/initial-setup"))
            .header(reqwest::header::ORIGIN, &origin)
            .header(reqwest::header::COOKIE, &bootstrap_cookie)
            .header("X-CSRF-Token", &bootstrap_csrf)
            .send()
            .unwrap();
        let setup_status = setup.status();
        let setup_text = setup.text().unwrap();
        assert_eq!(setup_status, reqwest::StatusCode::OK, "{setup_text}");
        let setup_body: serde_json::Value = serde_json::from_str(&setup_text).unwrap();
        let totp_secret = setup_body["totpSecret"]
            .as_str()
            .expect("first-login setup must return a TOTP secret");
        let new_admin_password = "local-e2e-admin-password-456";
        let complete_setup = http
            .post(format!("{origin}/admin/api/initial-setup"))
            .header(reqwest::header::ORIGIN, &origin)
            .header(reqwest::header::COOKIE, &bootstrap_cookie)
            .header("X-CSRF-Token", &bootstrap_csrf)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "newPassword": new_admin_password,
                    "totpCode": totp_code(totp_secret, std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs())
                }))
                .unwrap(),
            )
            .send()
            .unwrap();
        let complete_setup_status = complete_setup.status();
        let complete_setup_text = complete_setup.text().unwrap();
        assert_eq!(
            complete_setup_status,
            reqwest::StatusCode::OK,
            "{complete_setup_text}"
        );
        let (session_cookie, csrf) =
            login_admin(&http, &origin, new_admin_password, "after first setup");
        let registration_value: serde_json::Value = serde_json::from_slice(
            &fs::read(canonical_root.join(REVOCATION_REGISTRATION_FILE_NAME)).unwrap(),
        )
        .unwrap();
        let import = http
            .post(format!("{origin}/admin/api/licenses"))
            .header(reqwest::header::ORIGIN, &origin)
            .header(reqwest::header::COOKIE, &session_cookie)
            .header("X-CSRF-Token", &csrf)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({"registration": registration_value}))
                    .unwrap(),
            )
            .send()
            .unwrap();
        assert_eq!(
            import.status(),
            reqwest::StatusCode::CREATED,
            "{}",
            import.text().unwrap()
        );

        crate::app::usb_guard::with_test_license_public_key(
            "license-e2e-2026",
            &BASE64.encode(license_key.verifying_key().to_bytes()),
            || {
                with_test_status_public_key(
                    "revocation-e2e-2026",
                    &BASE64.encode(status_key.verifying_key().to_bytes()),
                    || {
                        let active = with_test_local_state_base(&local_state, || {
                            check_startup_with_url(
                                &canonical_root,
                                &license,
                                &fingerprint,
                                Some(&origin),
                            )
                        })
                        .unwrap();
                        assert_eq!(active, Decision::ActiveOnline { state_version: 1 });

                        let revoke = http
                            .post(format!(
                                "{origin}/admin/api/licenses/{}/revoke",
                                license.installation_id
                            ))
                            .header(reqwest::header::ORIGIN, &origin)
                            .header(reqwest::header::COOKIE, &session_cookie)
                            .header("X-CSRF-Token", &csrf)
                            .header(reqwest::header::CONTENT_TYPE, "application/json")
                            .body(
                                serde_json::to_vec(&serde_json::json!({
                                    "reason": "local end-to-end acceptance",
                                    "confirmInstallationId": license.installation_id
                                }))
                                .unwrap(),
                            )
                            .send()
                            .unwrap();
                        assert_eq!(
                            revoke.status(),
                            reqwest::StatusCode::OK,
                            "{}",
                            revoke.text().unwrap()
                        );

                        let revoked = with_test_local_state_base(&local_state, || {
                            check_startup_with_url(
                                &canonical_root,
                                &license,
                                &fingerprint,
                                Some(&origin),
                            )
                        })
                        .unwrap();
                        assert_eq!(revoked, Decision::Revoked { state_version: 2 });

                        let observation = crate::app::usb_guard::UsbObservation {
                            removable: true,
                            drive_root: Some(drive_root.clone()),
                            bundle_path: canonical_root.to_string_lossy().into_owned(),
                            fingerprint: Some(fingerprint.clone()),
                        };
                        let status = crate::app::usb_guard::handle_startup_revocation_decision_for_test(
                            &revoked,
                            &observation,
                            &canonical_root,
                            &license,
                            &fingerprint,
                            |bundle, license, fingerprint| {
                                crate::app::usb_revocation_cleanup::cleanup_revoked_bundle_for_test(
                                    bundle,
                                    license,
                                    fingerprint,
                                    |_| Ok(observation.clone()),
                                    |_| Ok(false),
                                )
                            },
                        )
                        .expect("manager startup must route revoked to cleanup")
                        .unwrap();
                        assert_eq!(status.revocation_state, "revoked_cleaned");
                        assert_eq!(status.revocation_state_version, Some(2));
                        assert!(!status.codex_allowed);
                        assert!(!canonical_root
                            .join("payload/OpenAI.Codex_e2e_x64.msix")
                            .exists());
                        assert!(!canonical_root.join("apps/Codex").exists());
                        assert!(canonical_root.join("user-data-sentinel.txt").exists());
                        assert!(canonical_root.join("usb-license.json").exists());
                        assert!(canonical_root
                            .join(REVOCATION_REGISTRATION_FILE_NAME)
                            .exists());
                        let usb_receipt = canonical_root.join(STATE_DIR_NAME).join(STATE_FILE_NAME);
                        assert!(
                            usb_receipt.exists(),
                            "revocation receipt must remain on the bundle"
                        );
                        let receipt: RevocationStateFile =
                            serde_json::from_slice(&fs::read(usb_receipt).unwrap()).unwrap();
                        assert_eq!(
                            highest_state(&receipt, &license, &fingerprint)
                                .unwrap()
                                .payload
                                .status,
                            "revoked"
                        );
                    },
                );
            },
        );
    }
}
