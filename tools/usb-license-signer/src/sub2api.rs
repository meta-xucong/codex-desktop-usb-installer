//! Minimal, fail-closed Sub2API client used only by the trusted USB signer.
//!
//! The signer calls the existing administrator/user endpoints in a bounded
//! sequence.  No administrator credential is accepted in a command-line
//! argument, written to a license, or included in reports.  This module is
//! deliberately independent of the manager GUI so its contract can be tested
//! with a local HTTP fixture without touching the production VPS.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::redirect::Policy;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use codex_usb_license::{UsbAccountBinding, UsbCcswitchBinding};

use crate::windows::{atomic_replace, dpapi_protect, dpapi_unprotect};

pub const DEFAULT_BASE_URL: &str = "https://404token.xyz";
pub const DEFAULT_EMAIL_DOMAIN: &str = "u.404token.xyz";
pub const TARGET_GROUP_NAME: &str = "unified-api-internal";
pub const MAX_PRELOAD_AMOUNT: f64 = 1_000_000.0;
/// Product-defined initial password for newly provisioned 404token accounts.
///
/// This is intentionally fixed for the USB registration workflow so the
/// operator and end user have a stable initial credential.  Existing accounts
/// and encrypted recovery records are never reset by this default.
pub const DEFAULT_INITIAL_PASSWORD: &str = "12345678";
pub const ADMIN_API_KEY_ENV: &str = "CODEX_404TOKEN_ADMIN_API_KEY";
pub const ADMIN_API_KEY_FALLBACK_ENV: &str = "SUB2API_ADMIN_API_KEY";
const USER_PAGE_SIZE: u64 = 1000;
const MAX_USER_PAGES: u64 = 10_000;
const PROVISION_RECOVERY_SCHEMA_VERSION: u32 = 1;
const MAX_TRANSIENT_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY_MS: u64 = 500;

// Generated from the release build environment. The source file is kept out of
// version control and the credential is never written to a license or bundle.
include!(concat!(env!("OUT_DIR"), "/embedded_admin_key.rs"));

const API_PREFIX: &str = "/api/v1";
const USER_AGENT: &str = "codex-usb-license-signer/1";

#[derive(Debug, Error)]
pub enum Sub2ApiError {
    #[error("配置无效：{0}")]
    Config(String),
    #[error("网络请求失败：{operation}")]
    Network { operation: &'static str },
    #[error(
        "网络请求失败：{operation}（已重试 {attempts} 次；404token 上游暂时不可达，请稍后重试）"
    )]
    RetryExhausted {
        operation: &'static str,
        attempts: u32,
    },
    #[error("HTTP {status}：{operation}（{kind}）")]
    Http {
        operation: &'static str,
        status: u16,
        kind: &'static str,
    },
    #[error("响应格式无效：{operation}")]
    Response { operation: &'static str },
    #[error("账号状态无法安全续作：{0}")]
    ExistingAccount(String),
    #[error("待处理预充值无法安全续作：{0}")]
    Pending(String),
    #[error("账号恢复记录无法安全续作：{0}")]
    Recovery(String),
    #[error("本地配置读写失败")]
    Local(#[source] std::io::Error),
}

#[derive(Clone)]
pub struct Sub2ApiConfig {
    pub base_url: String,
    pub admin_api_key: String,
    pub fake_email_domain: String,
}

impl Default for Sub2ApiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            admin_api_key: default_admin_api_key(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_email_domain")]
    fake_email_domain: String,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

fn default_email_domain() -> String {
    DEFAULT_EMAIL_DOMAIN.to_string()
}

fn non_empty_admin_api_key(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 512 || trimmed.chars().any(char::is_control) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn embedded_admin_api_key() -> Option<String> {
    for value in [EMBEDDED_ADMIN_API_KEY, EMBEDDED_ADMIN_API_KEY_FALLBACK]
        .into_iter()
        .flatten()
    {
        if let Some(key) = non_empty_admin_api_key(value.to_string()) {
            return Some(key);
        }
    }
    None
}

/// Resolve the signer administrator credential without requiring an operator to
/// type it into the GUI. Runtime environment variables are the preferred source
/// for managed deployments, followed by the production compile-time embedded
/// credential. A local JSON file is deliberately not a credential source: the
/// signer may persist non-secret endpoint settings, but must never persist or
/// load an administrator key from disk.
///
/// The value is never logged by this module. An empty return means that the
/// release was not provisioned with an administrator credential yet.
pub fn default_admin_api_key() -> String {
    for variable in [ADMIN_API_KEY_ENV, ADMIN_API_KEY_FALLBACK_ENV] {
        if let Ok(value) = std::env::var(variable) {
            if let Some(key) = non_empty_admin_api_key(value) {
                return key;
            }
        }
    }
    embedded_admin_api_key().unwrap_or_default()
}

/// Non-secret build/runtime diagnostics used only by release verification.
pub fn admin_api_key_diagnostics() -> Value {
    let runtime_primary = std::env::var(ADMIN_API_KEY_ENV).ok();
    let runtime_fallback = std::env::var(ADMIN_API_KEY_FALLBACK_ENV).ok();
    let resolved = default_admin_api_key();
    json!({
        "resolvedLength": resolved.trim().len(),
        "source": if runtime_primary.as_ref().is_some_and(|v| !v.trim().is_empty()) {
            "runtime-primary"
        } else if runtime_fallback.as_ref().is_some_and(|v| !v.trim().is_empty()) {
            "runtime-fallback"
        } else if embedded_admin_api_key().is_some() {
            "embedded"
        } else {
            "missing"
        },
        "runtimePrimaryLength": runtime_primary.map(|v| v.trim().len()).unwrap_or(0),
        "runtimeFallbackLength": runtime_fallback.map(|v| v.trim().len()).unwrap_or(0),
        "embeddedPrimaryLength": EMBEDDED_ADMIN_API_KEY.map(|v| v.trim().len()).unwrap_or(0),
        "embeddedFallbackLength": EMBEDDED_ADMIN_API_KEY_FALLBACK.map(|v| v.trim().len()).unwrap_or(0),
    })
}

impl From<StoredConfig> for Sub2ApiConfig {
    fn from(value: StoredConfig) -> Self {
        Self {
            base_url: value.base_url,
            admin_api_key: default_admin_api_key(),
            fake_email_domain: value.fake_email_domain,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProvisionedAccount {
    pub binding: UsbAccountBinding,
    pub initial_password: Zeroizing<String>,
    pub reused: bool,
    pub recovered: bool,
}

/// Encrypted local state used only to resume a registration after the remote
/// account exists but the USB license has not been written yet. The plaintext
/// form contains credentials, but the file on disk is Windows DPAPI protected
/// for the current signer user and is never placed in the USB bundle.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingProvisionRecord {
    schema_version: u32,
    recovery_key_digest: String,
    usb_asset_id: String,
    login_email: String,
    sub2api_user_id: u64,
    group_id: u64,
    password: String,
    /// Per-registration idempotency key for the admin create-user POST.
    /// Persisted before the request so retries are stable, while a fresh
    /// registration that reuses a USB number cannot replay a prior account.
    #[serde(default)]
    provision_idempotency_key: Option<String>,
    #[serde(default)]
    device_api_key: Option<String>,
    /// Per-registration idempotency key for the user API-key POST. This must
    /// be persisted before the request so retries of one registration reuse
    /// it, while a fresh registration that reuses a USB number gets a new key.
    #[serde(default)]
    user_api_key_idempotency_key: Option<String>,
    created_at_unix_ms: u128,
}

impl Drop for PendingProvisionRecord {
    fn drop(&mut self) {
        self.password.zeroize();
        if let Some(api_key) = self.device_api_key.as_mut() {
            api_key.zeroize();
        }
    }
}

/// Local, non-secret recovery metadata for a post-signing preload operation.
///
/// This file deliberately contains neither the administrator key, the user
/// password, nor the generated user API key.  Its idempotency key is derived
/// from the USB asset and license digest, so a retry cannot accidentally turn
/// one logical preload into a second charge when the backend honors the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingPreload {
    pub schema_version: u32,
    pub usb_asset_id: String,
    pub sub2api_user_id: u64,
    pub amount: f64,
    pub license_digest: String,
    pub idempotency_key: String,
    pub created_at_unix_ms: u128,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPreloadResult {
    pub pending: PendingPreload,
}

#[derive(Debug, Clone)]
struct UserRecord {
    id: u64,
    username: String,
    email: String,
}

pub fn config_path() -> Result<PathBuf, Sub2ApiError> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Sub2ApiError::Config("LOCALAPPDATA 不可用".to_string()))?;
    Ok(PathBuf::from(local)
        .join("CodexUsbLicenseSigner")
        .join("sub2api-config.json"))
}

fn pending_dir() -> Result<PathBuf, Sub2ApiError> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Sub2ApiError::Config("LOCALAPPDATA 不可用".to_string()))?;
    Ok(PathBuf::from(local)
        .join("CodexUsbLicenseSigner")
        .join("pending"))
}

fn pending_provision_dir() -> Result<PathBuf, Sub2ApiError> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Sub2ApiError::Config("LOCALAPPDATA 不可用".to_string()))?;
    Ok(PathBuf::from(local)
        .join("CodexUsbLicenseSigner")
        .join("provisioning"))
}

/// Build the stable, non-secret identity used for a registration recovery
/// record. A copied Bundle on another physical volume intentionally receives a
/// different identity, so a stale record cannot bind to an unrelated drive.
pub fn make_provision_recovery_key(
    volume_serial: u32,
    anchor_sha256: &str,
    bundle_relative_path: &str,
) -> String {
    format!("volume={volume_serial};anchor={anchor_sha256};bundle={bundle_relative_path}")
}

fn recovery_key_digest(recovery_key: &str) -> Result<String, Sub2ApiError> {
    if recovery_key.is_empty()
        || recovery_key.len() > 1024
        || recovery_key.chars().any(char::is_control)
    {
        return Err(Sub2ApiError::Recovery("恢复标识无效".to_string()));
    }
    let digest = Sha256::digest(recovery_key.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn pending_provision_path(recovery_key: &str) -> Result<PathBuf, Sub2ApiError> {
    let digest = recovery_key_digest(recovery_key)?;
    Ok(pending_provision_dir()?.join(format!("{digest}.bin")))
}

fn validate_pending_provision(
    pending: &PendingProvisionRecord,
    recovery_key: &str,
) -> Result<(), Sub2ApiError> {
    if pending.schema_version != PROVISION_RECOVERY_SCHEMA_VERSION {
        return Err(Sub2ApiError::Recovery("恢复记录版本不受支持".to_string()));
    }
    if pending.recovery_key_digest != recovery_key_digest(recovery_key)? {
        return Err(Sub2ApiError::Recovery(
            "恢复记录与当前 Bundle 不一致".to_string(),
        ));
    }
    validate_asset_id(&pending.usb_asset_id)
        .map_err(|_| Sub2ApiError::Recovery("恢复记录中的 U 盘编号无效".to_string()))?;
    validate_email_domain(
        pending
            .login_email
            .rsplit_once('@')
            .map(|(_, domain)| domain)
            .unwrap_or_default(),
    )
    .map_err(|_| Sub2ApiError::Recovery("恢复记录中的邮箱无效".to_string()))?;
    if !pending.login_email.contains('@')
        || (pending.sub2api_user_id == 0 && pending.device_api_key.is_some())
        || pending.group_id == 0
        || pending.created_at_unix_ms == 0
    {
        return Err(Sub2ApiError::Recovery("恢复记录参数无效".to_string()));
    }
    if let Some(key) = pending.user_api_key_idempotency_key.as_deref() {
        if !valid_user_api_key_idempotency_key(&pending.usb_asset_id, key) {
            return Err(Sub2ApiError::Recovery(
                "恢复记录中的 API Key 请求标识无效".to_string(),
            ));
        }
    }
    if let Some(key) = pending.provision_idempotency_key.as_deref() {
        if !valid_provision_idempotency_key(&pending.usb_asset_id, key) {
            return Err(Sub2ApiError::Recovery(
                "恢复记录中的账号创建请求标识无效".to_string(),
            ));
        }
    }
    validate_initial_password(&pending.password)
        .map_err(|_| Sub2ApiError::Recovery("恢复记录中的密码无效".to_string()))?;
    if let Some(api_key) = pending.device_api_key.as_deref() {
        if api_key.trim().is_empty()
            || api_key.len() > 2048
            || api_key.chars().any(char::is_control)
        {
            return Err(Sub2ApiError::Recovery(
                "恢复记录中的用户 Key 无效".to_string(),
            ));
        }
    }
    Ok(())
}

fn load_pending_provision(
    recovery_key: &str,
) -> Result<Option<PendingProvisionRecord>, Sub2ApiError> {
    let path = pending_provision_path(recovery_key)?;
    let encrypted = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Sub2ApiError::Local(error)),
    };
    let mut plaintext = dpapi_unprotect(&encrypted)
        .map_err(|_| Sub2ApiError::Recovery("本机无法解密账号恢复记录".to_string()))?;
    let parsed = serde_json::from_slice::<PendingProvisionRecord>(&plaintext)
        .map_err(|_| Sub2ApiError::Recovery("账号恢复记录格式无效".to_string()));
    plaintext.zeroize();
    let pending = parsed?;
    validate_pending_provision(&pending, recovery_key)?;
    Ok(Some(pending))
}

fn save_pending_provision(
    recovery_key: &str,
    pending: &PendingProvisionRecord,
) -> Result<(), Sub2ApiError> {
    validate_pending_provision(pending, recovery_key)?;
    let path = pending_provision_path(recovery_key)?;
    if let Some(existing) = load_pending_provision(recovery_key)? {
        let identity_changed = existing.recovery_key_digest != pending.recovery_key_digest
            || existing.group_id != pending.group_id
            || existing.password != pending.password
            || existing.created_at_unix_ms != pending.created_at_unix_ms;
        let candidate_changed = existing.usb_asset_id != pending.usb_asset_id
            || existing.login_email != pending.login_email;
        // A login-401 reconciliation may replace a stale remote user ID only
        // after the exact USB username+email lookup proves the stored ID is
        // obsolete. Such a repair is allowed only before a user API key has
        // been saved and without changing the local registration identity.
        let remote_identity_repair = existing.sub2api_user_id != pending.sub2api_user_id
            && existing.device_api_key.is_none()
            && pending.device_api_key.is_none()
            && !candidate_changed
            && !identity_changed;
        let user_id_changed_unsafely = existing.sub2api_user_id != 0
            && existing.sub2api_user_id != pending.sub2api_user_id
            && !remote_identity_repair;
        let candidate_changed_after_creation =
            candidate_changed && (existing.sub2api_user_id != 0 || pending.sub2api_user_id != 0);
        let key_changed = existing
            .device_api_key
            .as_deref()
            .is_some_and(|key| pending.device_api_key.as_deref() != Some(key));
        let api_key_idempotency_changed_unsafely = existing.sub2api_user_id != 0
            && !remote_identity_repair
            && existing
                .user_api_key_idempotency_key
                .as_deref()
                .is_some_and(|key| pending.user_api_key_idempotency_key.as_deref() != Some(key));
        let provision_idempotency_changed_unsafely = existing.sub2api_user_id != 0
            && !remote_identity_repair
            && existing
                .provision_idempotency_key
                .as_deref()
                .is_some_and(|key| pending.provision_idempotency_key.as_deref() != Some(key));
        if identity_changed
            || user_id_changed_unsafely
            || candidate_changed_after_creation
            || key_changed
            || api_key_idempotency_changed_unsafely
            || provision_idempotency_changed_unsafely
        {
            return Err(Sub2ApiError::Recovery(
                "已有不同账号占用当前 Bundle 的恢复记录".to_string(),
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| Sub2ApiError::Recovery("恢复记录目录无效".to_string()))?;
    fs::create_dir_all(parent).map_err(Sub2ApiError::Local)?;
    let mut plaintext = serde_json::to_vec(pending)
        .map_err(|_| Sub2ApiError::Recovery("恢复记录序列化失败".to_string()))?;
    let encrypted = dpapi_protect(&plaintext)
        .map_err(|_| Sub2ApiError::Recovery("本机无法保护账号恢复记录".to_string()));
    plaintext.zeroize();
    let encrypted = encrypted?;
    let temporary = parent.join(format!(".provision-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, encrypted).map_err(Sub2ApiError::Local)?;
    atomic_replace(&temporary, &path).map_err(|error| Sub2ApiError::Recovery(error.to_string()))?;
    Ok(())
}

fn clear_pending_provision(recovery_key: &str) -> Result<(), Sub2ApiError> {
    let path = pending_provision_path(recovery_key)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Sub2ApiError::Local(error)),
    }
}

pub fn has_provision_recovery(recovery_key: &str) -> Result<bool, Sub2ApiError> {
    Ok(load_pending_provision(recovery_key)?.is_some())
}

pub fn pending_preload_path(asset_id: &str) -> Result<PathBuf, Sub2ApiError> {
    validate_asset_id(asset_id)?;
    Ok(pending_dir()?.join(format!("{asset_id}.json")))
}

pub fn has_pending_preload(asset_id: &str) -> Result<bool, Sub2ApiError> {
    Ok(load_pending_preload(asset_id)?.is_some())
}

pub fn load_pending_preload(asset_id: &str) -> Result<Option<PendingPreload>, Sub2ApiError> {
    let path = pending_preload_path(asset_id)?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Sub2ApiError::Local(error)),
    };
    let pending: PendingPreload = serde_json::from_slice(&bytes)
        .map_err(|_| Sub2ApiError::Pending("本地记录格式无效".to_string()))?;
    validate_pending_preload(&pending)?;
    if pending.usb_asset_id != asset_id {
        return Err(Sub2ApiError::Pending(
            "本地记录与当前 U 盘编号不一致".to_string(),
        ));
    }
    Ok(Some(pending))
}

fn save_pending_preload(pending: &PendingPreload) -> Result<(), Sub2ApiError> {
    validate_pending_preload(pending)?;
    let path = pending_preload_path(&pending.usb_asset_id)?;
    if let Some(existing) = load_pending_preload(&pending.usb_asset_id)? {
        if existing.idempotency_key != pending.idempotency_key {
            return Err(Sub2ApiError::Pending(
                "已有另一条待处理预充值记录，请先完成或核对后再开始新操作".to_string(),
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| Sub2ApiError::Config("待处理记录目录无效".to_string()))?;
    fs::create_dir_all(parent).map_err(Sub2ApiError::Local)?;
    let bytes = serde_json::to_vec_pretty(pending)
        .map_err(|_| Sub2ApiError::Pending("本地记录序列化失败".to_string()))?;
    let temporary = parent.join(format!(".pending-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes).map_err(Sub2ApiError::Local)?;
    atomic_replace(&temporary, &path).map_err(|error| Sub2ApiError::Config(error.to_string()))?;
    Ok(())
}

fn clear_pending_preload(asset_id: &str) -> Result<(), Sub2ApiError> {
    let path = pending_preload_path(asset_id)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Sub2ApiError::Local(error)),
    }
}

fn validate_asset_id(asset_id: &str) -> Result<(), Sub2ApiError> {
    let Some(suffix) = asset_id.strip_prefix("USB-") else {
        return Err(Sub2ApiError::Pending("U 盘编号格式无效".to_string()));
    };
    if suffix.is_empty() || suffix.len() > 20 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Sub2ApiError::Pending("U 盘编号格式无效".to_string()));
    }
    Ok(())
}

fn validate_pending_preload(pending: &PendingPreload) -> Result<(), Sub2ApiError> {
    if pending.schema_version != 1 {
        return Err(Sub2ApiError::Pending("本地记录版本不受支持".to_string()));
    }
    validate_asset_id(&pending.usb_asset_id)?;
    if pending.sub2api_user_id == 0
        || !pending.amount.is_finite()
        || pending.amount <= 0.0
        || pending.amount > MAX_PRELOAD_AMOUNT
        || pending.license_digest.is_empty()
        || pending.license_digest.len() > 256
        || pending.license_digest.chars().any(char::is_control)
        || pending.idempotency_key
            != make_idempotency_key(&pending.usb_asset_id, &pending.license_digest)
    {
        return Err(Sub2ApiError::Pending("本地记录参数无效".to_string()));
    }
    Ok(())
}

fn new_pending_preload(
    user_id: u64,
    amount: f64,
    license_digest: &str,
    asset_id: &str,
) -> Result<PendingPreload, Sub2ApiError> {
    let pending = PendingPreload {
        schema_version: 1,
        usb_asset_id: asset_id.to_string(),
        sub2api_user_id: user_id,
        amount,
        license_digest: license_digest.to_string(),
        idempotency_key: make_idempotency_key(asset_id, license_digest),
        created_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Sub2ApiError::Pending("系统时间无效".to_string()))?
            .as_millis(),
    };
    validate_pending_preload(&pending)?;
    Ok(pending)
}

pub fn load_config() -> Result<Sub2ApiConfig, Sub2ApiError> {
    let path = config_path()?;
    match fs::read(&path) {
        Ok(bytes) => {
            let contains_legacy_secret = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .map(|object| {
                    object.contains_key("adminApiKey") || object.contains_key("admin_api_key")
                })
                .unwrap_or(false);
            let mut config = serde_json::from_slice::<StoredConfig>(&bytes)
                .map(Sub2ApiConfig::from)
                .map_err(|_| Sub2ApiError::Config("本地 Sub2API 配置格式无效".to_string()))?;
            // Endpoint settings are user preferences. The administrator key is
            // always resolved from the managed environment or the embedded
            // release credential; legacy adminApiKey JSON fields are ignored.
            config.admin_api_key = default_admin_api_key();
            // Older signer builds persisted the administrator key in this JSON
            // file. Migrate that file on read so merely opening the signer also
            // removes the legacy plaintext credential. Fail closed if the
            // sanitized replacement cannot be written.
            if contains_legacy_secret {
                write_non_secret_config(&config)?;
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Sub2ApiConfig::default()),
        Err(error) => Err(Sub2ApiError::Local(error)),
    }
}

pub fn save_config(config: &Sub2ApiConfig) -> Result<PathBuf, Sub2ApiError> {
    validate_config(config)?;
    write_non_secret_config(config)
}

fn write_non_secret_config(config: &Sub2ApiConfig) -> Result<PathBuf, Sub2ApiError> {
    let path = config_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| Sub2ApiError::Config("Sub2API 配置目录无效".to_string()))?;
    fs::create_dir_all(parent).map_err(Sub2ApiError::Local)?;
    let value = json!({
        "baseUrl": config.base_url.trim_end_matches('/'),
        "fakeEmailDomain": config.fake_email_domain,
    });
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|_| Sub2ApiError::Config("Sub2API 配置序列化失败".to_string()))?;
    let temporary = parent.join(format!(".sub2api-config-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes).map_err(Sub2ApiError::Local)?;
    atomic_replace(&temporary, &path).map_err(|error| Sub2ApiError::Config(error.to_string()))?;
    Ok(path)
}

pub fn validate_config(config: &Sub2ApiConfig) -> Result<(), Sub2ApiError> {
    let normalized = normalize_base_url(&config.base_url)?;
    if normalized != config.base_url.trim_end_matches('/') {
        return Err(Sub2ApiError::Config(
            "API 基地址包含多余空白或格式不标准".to_string(),
        ));
    }
    if config.admin_api_key.trim().is_empty()
        || config.admin_api_key.len() > 512
        || config.admin_api_key.chars().any(char::is_control)
    {
        return Err(Sub2ApiError::Config(
            "管理员 API Key 为空或包含非法字符".to_string(),
        ));
    }
    validate_email_domain(&config.fake_email_domain)?;
    Ok(())
}

/// Validate the production destination before sending the compiled-in
/// administrator credential. Generic `Sub2ApiClient::new` remains available
/// for loopback contract tests, while the shipped GUI must use this pinned
/// constructor so a changed local preference cannot redirect the admin key.
pub fn validate_trusted_admin_endpoint(config: &Sub2ApiConfig) -> Result<(), Sub2ApiError> {
    let normalized = normalize_base_url(&config.base_url)?;
    if normalized != DEFAULT_BASE_URL {
        return Err(Sub2ApiError::Config(
            "管理员凭据仅允许发送到固定的 404token 服务地址".to_string(),
        ));
    }
    Ok(())
}

pub fn parse_preload_amount(raw: &str) -> Result<f64, Sub2ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(0.0);
    }
    let amount = trimmed
        .parse::<f64>()
        .map_err(|_| Sub2ApiError::Config("预充值金额必须是非负数字".to_string()))?;
    if !amount.is_finite() || !(0.0..=MAX_PRELOAD_AMOUNT).contains(&amount) {
        return Err(Sub2ApiError::Config(format!(
            "预充值金额必须在 0 到 {MAX_PRELOAD_AMOUNT} 之间"
        )));
    }
    Ok(amount)
}

/// A repeated signing/ensure flow must not silently add balance again. A
/// positive preload is only eligible for a newly provisioned account; a
/// later explicit recharge flow can use its own endpoint and idempotency key.
pub fn should_precharge(amount: f64, reused_account: bool) -> bool {
    amount.is_finite() && amount > 0.0 && !reused_account
}

pub fn make_idempotency_key(asset_id: &str, license_digest: &str) -> String {
    let digest = Sha256::digest(format!("{asset_id}:{license_digest}").as_bytes());
    format!(
        "usb-preload-{asset_id}-{}",
        digest
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// Create a fresh idempotency key for one registration's account-creation
/// transaction. The random suffix avoids replaying a deleted account's server
/// idempotency record if USB numbering is later reset.
pub fn make_provision_idempotency_key(asset_id: &str) -> String {
    format!("usb-provision-{asset_id}-{}", Uuid::new_v4().simple())
}

fn valid_provision_idempotency_key(asset_id: &str, key: &str) -> bool {
    valid_registration_idempotency_key("usb-provision", asset_id, key)
}

/// Create a fresh key for one registration's user API-key operation. The
/// random suffix prevents a deleted/re-numbered test account from reusing a
/// server idempotency record; callers persist the result before the POST.
fn make_user_api_key_idempotency_key(asset_id: &str) -> String {
    format!("usb-user-key-{asset_id}-{}", Uuid::new_v4().simple())
}

fn valid_user_api_key_idempotency_key(asset_id: &str, key: &str) -> bool {
    valid_registration_idempotency_key("usb-user-key", asset_id, key)
}

fn valid_registration_idempotency_key(prefix: &str, asset_id: &str, key: &str) -> bool {
    let Some(suffix) = key.strip_prefix(&format!("{prefix}-{asset_id}-")) else {
        return false;
    };
    suffix.len() == 32
        && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        && Uuid::parse_str(suffix).is_ok()
}

pub fn generate_initial_password() -> Result<Zeroizing<String>, Sub2ApiError> {
    Ok(Zeroizing::new(DEFAULT_INITIAL_PASSWORD.to_string()))
}

pub fn validate_initial_password(password: &str) -> Result<(), Sub2ApiError> {
    let length = password.chars().count();
    if !(8..=128).contains(&length) || password.chars().any(char::is_control) {
        return Err(Sub2ApiError::Config(
            "初始密码长度必须为 8 到 128 个字符，且不能包含控制字符".to_string(),
        ));
    }
    Ok(())
}

pub struct Sub2ApiClient {
    config: Sub2ApiConfig,
    client: Client,
}

impl Sub2ApiClient {
    pub fn new(config: Sub2ApiConfig) -> Result<Self, Sub2ApiError> {
        validate_config(&config)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            return Err(Sub2ApiError::Config("TLS 加密组件初始化失败".to_string()));
        }
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            // Admin requests carry x-api-key. Never forward it through an
            // HTTP redirect to a second host.
            .redirect(Policy::none())
            .build()
            .map_err(|_| Sub2ApiError::Config("网络客户端初始化失败".to_string()))?;
        Ok(Self { config, client })
    }

    /// Construct the production client used by the signer GUI. The generic
    /// constructor is retained for local loopback fixtures only.
    pub fn new_for_trusted_admin(config: Sub2ApiConfig) -> Result<Self, Sub2ApiError> {
        validate_config(&config)?;
        validate_trusted_admin_endpoint(&config)?;
        Self::new(config)
    }

    pub fn provision_account(
        &self,
        existing_binding: Option<&UsbAccountBinding>,
    ) -> Result<ProvisionedAccount, Sub2ApiError> {
        self.provision_account_with_recovery(existing_binding, None, None)
    }

    pub fn provision_account_with_password(
        &self,
        existing_binding: Option<&UsbAccountBinding>,
        initial_password: Option<&str>,
    ) -> Result<ProvisionedAccount, Sub2ApiError> {
        self.provision_account_with_recovery(existing_binding, initial_password, None)
    }

    /// Provision or resume one account for a physical Bundle. The recovery
    /// key is local, non-secret identity data supplied by the GUI. Once the
    /// remote user exists, the account password and generated API key are
    /// stored only in a DPAPI-protected record until the license is verified.
    pub fn provision_account_with_recovery(
        &self,
        existing_binding: Option<&UsbAccountBinding>,
        initial_password: Option<&str>,
        recovery_key: Option<&str>,
    ) -> Result<ProvisionedAccount, Sub2ApiError> {
        if let Some(binding) = existing_binding {
            codex_usb_license::validate_account_binding(binding)
                .map_err(|error| Sub2ApiError::ExistingAccount(error.to_string()))?;
            return Ok(ProvisionedAccount {
                binding: binding.clone(),
                initial_password: Zeroizing::new(String::new()),
                reused: true,
                recovered: false,
            });
        }

        let recovery_key = recovery_key
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(recovery_key) = recovery_key {
            if let Some(mut pending) = load_pending_provision(recovery_key)? {
                let mut group_checked = false;
                if pending.sub2api_user_id == 0 {
                    let mut users = self.list_usb_users()?;
                    if let Some(user) =
                        find_user_identity(&users, &pending.usb_asset_id, &pending.login_email)
                    {
                        pending.sub2api_user_id = user.id;
                        save_pending_provision(recovery_key, &pending)?;
                    } else {
                        let current_group = self.ensure_group_available()?;
                        if current_group != pending.group_id {
                            return Err(Sub2ApiError::Recovery(
                                "目标统一 API 分组已变化，不能自动续作旧账号".to_string(),
                            ));
                        }
                        group_checked = true;
                        self.create_pending_user(&mut pending, &mut users, Some(recovery_key))?;
                    }
                }
                return self.finish_pending_provision(
                    pending,
                    Some(recovery_key),
                    true,
                    group_checked,
                );
            }
        }

        let password = match initial_password {
            Some(value) => {
                validate_initial_password(value)?;
                Zeroizing::new(value.to_string())
            }
            None => generate_initial_password()?,
        };
        let group_id = self.ensure_group_available()?;
        let mut users = self.list_usb_users()?;
        let number = next_usb_number(&users);
        let asset_id = format!("USB-{number:04}");
        let email = short_email(number, &self.config.fake_email_domain)?;
        let mut pending = PendingProvisionRecord {
            schema_version: PROVISION_RECOVERY_SCHEMA_VERSION,
            recovery_key_digest: recovery_key
                .map(recovery_key_digest)
                .transpose()?
                .unwrap_or_default(),
            usb_asset_id: asset_id.clone(),
            login_email: email,
            sub2api_user_id: 0,
            group_id,
            password: password.to_string(),
            provision_idempotency_key: Some(make_provision_idempotency_key(&asset_id)),
            device_api_key: None,
            user_api_key_idempotency_key: Some(make_user_api_key_idempotency_key(&asset_id)),
            created_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Sub2ApiError::Recovery("系统时间无效".to_string()))?
                .as_millis(),
        };
        // Write the intended identity before the remote POST. If the response
        // is lost or the process exits, the next launch can reconcile this same
        // account instead of allocating a new USB number.
        if let Some(recovery_key) = recovery_key {
            save_pending_provision(recovery_key, &pending)?;
        }
        self.create_pending_user(&mut pending, &mut users, recovery_key)?;
        self.finish_pending_provision(pending, recovery_key, false, true)
    }

    fn create_pending_user(
        &self,
        pending: &mut PendingProvisionRecord,
        users: &mut Vec<UserRecord>,
        recovery_key: Option<&str>,
    ) -> Result<(), Sub2ApiError> {
        let mut number = pending
            .usb_asset_id
            .strip_prefix("USB-")
            .and_then(|suffix| suffix.parse::<u64>().ok())
            .ok_or_else(|| Sub2ApiError::Recovery("恢复记录中的 U 盘编号无效".to_string()))?;
        let mut attempted_numbers = BTreeSet::new();
        for _ in 0..8 {
            while attempted_numbers.contains(&number) {
                number = number.saturating_add(1);
            }
            let asset_id = format!("USB-{number:04}");
            let email = short_email(number, &self.config.fake_email_domain)?;
            if users
                .iter()
                .any(|user| user.username == asset_id || user.email == email)
            {
                attempted_numbers.insert(number);
                number = next_usb_number(users).max(number.saturating_add(1));
                continue;
            }
            let candidate_changed = pending.usb_asset_id != asset_id;
            pending.usb_asset_id = asset_id.clone();
            pending.login_email = email.clone();
            if candidate_changed {
                pending.provision_idempotency_key = Some(make_provision_idempotency_key(&asset_id));
                pending.user_api_key_idempotency_key =
                    Some(make_user_api_key_idempotency_key(&asset_id));
            }
            if pending.provision_idempotency_key.is_none() {
                pending.provision_idempotency_key = Some(make_provision_idempotency_key(&asset_id));
            }
            if let Some(recovery_key) = recovery_key {
                save_pending_provision(recovery_key, pending)?;
            }
            match self.create_user_with_recovery(
                &asset_id,
                &email,
                &pending.password,
                pending.group_id,
                pending
                    .provision_idempotency_key
                    .as_deref()
                    .expect("provision idempotency key created before request"),
                users,
            ) {
                Ok(user_id) => {
                    pending.sub2api_user_id = user_id;
                    if let Some(recovery_key) = recovery_key {
                        save_pending_provision(recovery_key, pending)?;
                    }
                    return Ok(());
                }
                Err(error) if error.is_conflict() => {
                    attempted_numbers.insert(number);
                    number = next_usb_number(users).max(number.saturating_add(1));
                }
                Err(error) => return Err(error),
            }
        }
        Err(Sub2ApiError::ExistingAccount(
            "连续发生编号冲突，未继续创建账号；下次可自动续作".to_string(),
        ))
    }

    fn finish_pending_provision(
        &self,
        mut pending: PendingProvisionRecord,
        recovery_key: Option<&str>,
        recovered: bool,
        group_checked: bool,
    ) -> Result<ProvisionedAccount, Sub2ApiError> {
        if pending.sub2api_user_id == 0 {
            return Err(Sub2ApiError::Recovery(
                "账号恢复记录尚未关联远端用户".to_string(),
            ));
        }
        let recovered_password = Zeroizing::new(pending.password.clone());
        let device_api_key = match pending.device_api_key.clone() {
            Some(api_key) => api_key,
            None => {
                if !group_checked {
                    let group_id = self.ensure_group_available()?;
                    if group_id != pending.group_id {
                        return Err(Sub2ApiError::Recovery(
                            "目标统一 API 分组已变化，不能自动续作旧账号".to_string(),
                        ));
                    }
                }
                // Older encrypted recovery records predate this field. Give
                // them one fresh operation key and persist it before login or
                // POST, so a restart cannot generate a different key.
                if pending.user_api_key_idempotency_key.is_none() {
                    pending.user_api_key_idempotency_key =
                        Some(make_user_api_key_idempotency_key(&pending.usb_asset_id));
                    if let Some(recovery_key) = recovery_key {
                        save_pending_provision(recovery_key, &pending)?;
                    }
                }
                let user_token = match self.login(&pending.login_email, &pending.password) {
                    Ok(token) => token,
                    Err(error) if error.is_unauthorized() => {
                        self.reconcile_after_login_unauthorized(&mut pending, recovery_key)?;
                        self.login(&pending.login_email, &pending.password)?
                    }
                    Err(error) => return Err(error),
                };
                let idempotency_key = pending
                    .user_api_key_idempotency_key
                    .as_deref()
                    .expect("API key operation key persisted before login");
                let api_key = self.create_user_api_key(
                    &user_token,
                    &pending.usb_asset_id,
                    pending.group_id,
                    &idempotency_key,
                )?;
                pending.device_api_key = Some(api_key.clone());
                if let Some(recovery_key) = recovery_key {
                    save_pending_provision(recovery_key, &pending)?;
                }
                api_key
            }
        };
        let binding = UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: pending.usb_asset_id.clone(),
            account_ref: pending.usb_asset_id.clone(),
            account_name: pending.usb_asset_id.clone(),
            login_email: pending.login_email.clone(),
            sub2api_user_id: pending.sub2api_user_id,
            ccswitch: UsbCcswitchBinding {
                provider_id: "404token-sub2api".to_string(),
                credential_id: format!("sub2api-user-key-{}", pending.sub2api_user_id),
                device_api_key,
                group_id: pending.group_id,
            },
        };
        codex_usb_license::validate_account_binding(&binding)
            .map_err(|error| Sub2ApiError::Recovery(error.to_string()))?;
        Ok(ProvisionedAccount {
            binding,
            initial_password: recovered_password,
            reused: false,
            recovered,
        })
    }

    /// A deleted account may leave the server's idempotency cache intact. If
    /// an older signer persisted the stale user ID returned by that cache,
    /// login can fail even though the current registration is otherwise
    /// recoverable. Reconcile only against the exact USB username+email pair.
    fn reconcile_after_login_unauthorized(
        &self,
        pending: &mut PendingProvisionRecord,
        recovery_key: Option<&str>,
    ) -> Result<(), Sub2ApiError> {
        let mut users = self.list_usb_users()?;
        if let Some(user) = find_user_identity(&users, &pending.usb_asset_id, &pending.login_email)
        {
            if user.id == pending.sub2api_user_id {
                return Err(Sub2ApiError::ExistingAccount(
                    "U 盘编号与邮箱对应的账号存在，但登录密码未通过；为避免覆盖账号，未自动重置密码"
                        .to_string(),
                ));
            }
            if pending.device_api_key.is_some() {
                return Err(Sub2ApiError::Recovery(
                    "账号 ID 与本地已保存的 API Key 不一致，已停止自动修复".to_string(),
                ));
            }
            pending.sub2api_user_id = user.id;
            pending.user_api_key_idempotency_key =
                Some(make_user_api_key_idempotency_key(&pending.usb_asset_id));
            if let Some(recovery_key) = recovery_key {
                save_pending_provision(recovery_key, pending)?;
            }
            return Ok(());
        }

        if users
            .iter()
            .any(|user| user.username == pending.usb_asset_id || user.email == pending.login_email)
        {
            return Err(Sub2ApiError::ExistingAccount(
                "远端存在相同 U 盘编号或邮箱，但两者不匹配；已停止自动创建以避免串号".to_string(),
            ));
        }
        if pending.device_api_key.is_some() {
            return Err(Sub2ApiError::Recovery(
                "远端账号不存在，但恢复记录已有 API Key；已停止自动重建".to_string(),
            ));
        }

        // The stored ID no longer names a visible account. Reset the remote
        // link and rotate both operation keys before safely creating the exact
        // persisted identity again. A later retry can find and resume it.
        pending.sub2api_user_id = 0;
        pending.provision_idempotency_key =
            Some(make_provision_idempotency_key(&pending.usb_asset_id));
        pending.user_api_key_idempotency_key =
            Some(make_user_api_key_idempotency_key(&pending.usb_asset_id));
        if let Some(recovery_key) = recovery_key {
            save_pending_provision(recovery_key, pending)?;
        }
        self.create_pending_user(pending, &mut users, recovery_key)
    }

    /// Remove the encrypted registration recovery record after the same
    /// account binding has been written to, and read back from, the license.
    pub fn clear_provision_recovery(&self, recovery_key: &str) -> Result<(), Sub2ApiError> {
        clear_pending_provision(recovery_key)
    }

    pub fn precharge(
        &self,
        user_id: u64,
        amount: f64,
        idempotency_key: &str,
        asset_id: &str,
    ) -> Result<(), Sub2ApiError> {
        if !amount.is_finite() || amount <= 0.0 || amount > MAX_PRELOAD_AMOUNT {
            return Err(Sub2ApiError::Config("预充值金额不在允许范围内".to_string()));
        }
        if user_id == 0 || idempotency_key.is_empty() || asset_id.is_empty() {
            return Err(Sub2ApiError::Config("预充值参数无效".to_string()));
        }
        let path = format!("{API_PREFIX}/admin/users/{user_id}/balance");
        let body = json!({
            "balance": amount,
            "operation": "add",
            "notes": format!("Codex USB {asset_id} initial preload"),
        });
        self.admin_request(
            "precharge",
            Method::POST,
            &path,
            Some(&body),
            Some(idempotency_key),
        )?;
        Ok(())
    }

    /// Persist a non-secret retry record before the network mutation.  The
    /// record is removed only after the backend call succeeds.  If the call
    /// or local cleanup fails, the same idempotency key remains available for
    /// an explicit retry instead of silently charging twice.
    pub fn precharge_with_pending(
        &self,
        user_id: u64,
        amount: f64,
        license_digest: &str,
        asset_id: &str,
    ) -> Result<PendingPreload, Sub2ApiError> {
        let pending = new_pending_preload(user_id, amount, license_digest, asset_id)?;
        save_pending_preload(&pending)?;
        self.precharge(
            pending.sub2api_user_id,
            pending.amount,
            &pending.idempotency_key,
            &pending.usb_asset_id,
        )?;
        clear_pending_preload(&pending.usb_asset_id)?;
        Ok(pending)
    }

    /// Retry only the exact pending operation associated with the signed USB
    /// binding.  There is no account lookup or new-key generation here.
    pub fn retry_pending_precharge(
        &self,
        binding: &UsbAccountBinding,
        current_license_digest: &str,
    ) -> Result<RetryPreloadResult, Sub2ApiError> {
        codex_usb_license::validate_account_binding(binding)
            .map_err(|error| Sub2ApiError::Pending(error.to_string()))?;
        let Some(pending) = load_pending_preload(&binding.usb_asset_id)? else {
            return Err(Sub2ApiError::Pending(
                "没有找到该 U 盘的待处理预充值记录".to_string(),
            ));
        };
        if pending.sub2api_user_id != binding.sub2api_user_id {
            return Err(Sub2ApiError::Pending(
                "待处理记录与许可证账号不一致，已停止".to_string(),
            ));
        }
        ensure_pending_license_digest(&pending, current_license_digest)?;
        self.precharge(
            pending.sub2api_user_id,
            pending.amount,
            &pending.idempotency_key,
            &pending.usb_asset_id,
        )?;
        clear_pending_preload(&pending.usb_asset_id)?;
        Ok(RetryPreloadResult { pending })
    }

    fn list_usb_users(&self) -> Result<Vec<UserRecord>, Sub2ApiError> {
        let mut page = 1u64;
        let mut all_users = BTreeMap::new();
        loop {
            let value = self.admin_request_with_retry(
                "list-users",
                Method::GET,
                &format!(
                    "{API_PREFIX}/admin/users?search=USB-&page={page}&page_size={USER_PAGE_SIZE}"
                ),
                None,
                None,
            )?;
            let page_users = parse_user_records(&value);
            let page_user_count = page_users.len();
            all_users.extend(page_users.into_iter().map(|user| (user.id, user)));

            let should_continue = match pagination_pages(&value) {
                Some(total_pages) => page < total_pages,
                None => page_user_count >= USER_PAGE_SIZE as usize,
            };
            if !should_continue {
                break;
            }
            page = page.saturating_add(1);
            if page > MAX_USER_PAGES {
                return Err(Sub2ApiError::Response {
                    operation: "list-users",
                });
            }
        }
        Ok(all_users.into_values().collect())
    }

    fn ensure_group_available(&self) -> Result<u64, Sub2ApiError> {
        let value = self.admin_request_with_retry(
            "list-groups",
            Method::GET,
            "/api/v1/admin/groups/all",
            None,
            None,
        )?;
        target_group_id(&value).ok_or_else(|| {
            Sub2ApiError::Config(format!(
                "目标分组 {TARGET_GROUP_NAME} 不存在、未启用或当前管理员无权访问"
            ))
        })
    }

    /// Create one account with a stable idempotency key and recover an
    /// ambiguous response by looking up the exact USB identity before sending
    /// another POST.  This is important for a timeout after Sub2API accepted
    /// the request: blindly repeating the POST could create a second account.
    fn create_user_with_recovery(
        &self,
        asset_id: &str,
        email: &str,
        password: &str,
        group_id: u64,
        idempotency_key: &str,
        users: &mut Vec<UserRecord>,
    ) -> Result<u64, Sub2ApiError> {
        let mut last_error = None;
        for attempt in 1..=MAX_TRANSIENT_ATTEMPTS {
            match self.create_user(asset_id, email, password, group_id, idempotency_key) {
                Ok(id) => return Ok(id),
                Err(error) if error.is_conflict() => {
                    let refreshed = self.list_usb_users()?;
                    if let Some(user) = find_user_identity(&refreshed, asset_id, email) {
                        return Ok(user.id);
                    }
                    *users = refreshed;
                    return Err(error);
                }
                Err(error) if error.is_retryable() => {
                    last_error = Some(error);
                    match self.list_usb_users() {
                        Ok(refreshed) => {
                            if let Some(user) = find_user_identity(&refreshed, asset_id, email) {
                                return Ok(user.id);
                            }
                            *users = refreshed;
                        }
                        Err(lookup_error) => {
                            // Keep the original transport error as the final
                            // diagnostic; the lookup itself must not expose
                            // a credential or abort the safe retry sequence.
                            let _ = lookup_error;
                        }
                    }
                    if attempt < MAX_TRANSIENT_ATTEMPTS {
                        std::thread::sleep(retry_delay(attempt));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error
            .expect("create-user retry must retain the last transient error")
            .retry_exhausted())
    }

    fn create_user(
        &self,
        asset_id: &str,
        email: &str,
        password: &str,
        group_id: u64,
        idempotency_key: &str,
    ) -> Result<u64, Sub2ApiError> {
        let body = json!({
            "email": email,
            "password": password,
            "username": asset_id,
            "notes": format!("Codex USB binding {asset_id}"),
            "balance": 0,
            "allowed_groups": [group_id],
        });
        let value = self.admin_request(
            "create-user",
            Method::POST,
            &format!("{API_PREFIX}/admin/users"),
            Some(&body),
            Some(idempotency_key),
        )?;
        find_id(&value).ok_or(Sub2ApiError::Response {
            operation: "create-user",
        })
    }

    fn login(&self, email: &str, password: &str) -> Result<String, Sub2ApiError> {
        let value = self.user_request(
            "login",
            Method::POST,
            &format!("{API_PREFIX}/auth/login"),
            Some(&json!({ "email": email, "password": password })),
            None,
            None,
        )?;
        find_string(&value, &["access_token", "accessToken", "token"])
            .ok_or(Sub2ApiError::Response { operation: "login" })
    }

    fn create_user_api_key(
        &self,
        token: &str,
        asset_id: &str,
        group_id: u64,
        idempotency_key: &str,
    ) -> Result<String, Sub2ApiError> {
        let value = self.user_request(
            "create-api-key",
            Method::POST,
            &format!("{API_PREFIX}/keys"),
            Some(&json!({
                "name": format!("CCSwitch-{asset_id}"),
                "group_id": group_id,
            })),
            Some(token),
            Some(idempotency_key),
        )?;
        find_string(&value, &["key", "api_key", "apiKey", "token"]).ok_or(Sub2ApiError::Response {
            operation: "create-api-key",
        })
    }

    fn admin_request(
        &self,
        operation: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Sub2ApiError> {
        let mut request = self.request(operation, method, path)?;
        request = request.header("x-api-key", &self.config.admin_api_key);
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        self.send_json(operation, request)
    }

    fn admin_request_with_retry(
        &self,
        operation: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Sub2ApiError> {
        self.retry_json_request(operation, || {
            self.admin_request(operation, method.clone(), path, body, idempotency_key)
        })
    }

    fn retry_json_request<F>(
        &self,
        _operation: &'static str,
        mut request: F,
    ) -> Result<Value, Sub2ApiError>
    where
        F: FnMut() -> Result<Value, Sub2ApiError>,
    {
        let mut last_error = None;
        for attempt in 1..=MAX_TRANSIENT_ATTEMPTS {
            match request() {
                Ok(value) => return Ok(value),
                Err(error) if error.is_retryable() => {
                    last_error = Some(error);
                    if attempt < MAX_TRANSIENT_ATTEMPTS {
                        std::thread::sleep(retry_delay(attempt));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error
            .expect("retry_json_request must retain the last transient error")
            .retry_exhausted())
    }

    fn user_request(
        &self,
        operation: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
        bearer_token: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Sub2ApiError> {
        let mut request = self.request(operation, method, path)?;
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        self.send_json(operation, request)
    }

    fn request(
        &self,
        _operation: &'static str,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, Sub2ApiError> {
        let url = build_url(&self.config.base_url, path)?;
        Ok(self
            .client
            .request(method, url)
            .header("Accept", "application/json"))
    }

    fn send_json(
        &self,
        operation: &'static str,
        request: RequestBuilder,
    ) -> Result<Value, Sub2ApiError> {
        let response = request
            .send()
            .map_err(|_| Sub2ApiError::Network { operation })?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|_| Sub2ApiError::Network { operation })?;
        if !status.is_success() {
            return Err(Sub2ApiError::Http {
                operation,
                status: status.as_u16(),
                kind: http_error_kind(status, operation),
            });
        }
        if bytes.is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_slice(&bytes).map_err(|_| Sub2ApiError::Response { operation })
    }
}

impl Sub2ApiError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::Network { .. } | Self::Response { .. } => true,
            Self::Http { status, .. } => {
                matches!(*status, 408 | 425 | 429) || (500..=599).contains(status)
            }
            _ => false,
        }
    }

    fn retry_exhausted(self) -> Self {
        let operation = match self {
            Self::Network { operation }
            | Self::Response { operation }
            | Self::Http { operation, .. } => operation,
            other => return other,
        };
        Self::RetryExhausted {
            operation,
            attempts: MAX_TRANSIENT_ATTEMPTS,
        }
    }

    fn is_conflict(&self) -> bool {
        matches!(
            self,
            Self::Http {
                status: 409 | 422,
                ..
            }
        )
    }

    fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Http { status: 401, .. })
    }
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(RETRY_BASE_DELAY_MS.saturating_mul(1_u64 << attempt.saturating_sub(1)))
}

fn http_error_kind(status: StatusCode, operation: &'static str) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "授权失败",
        StatusCode::NOT_FOUND => "接口不存在",
        StatusCode::CONFLICT if operation == "create-api-key" => "API Key 创建冲突",
        StatusCode::CONFLICT => "资源冲突",
        StatusCode::UNPROCESSABLE_ENTITY => "参数被拒绝",
        status if status.is_server_error() => "服务端错误",
        _ => "请求被拒绝",
    }
}

fn normalize_base_url(raw: &str) -> Result<String, Sub2ApiError> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed)
        .map_err(|_| Sub2ApiError::Config("API 基地址不是有效 URL".to_string()))?;
    if parsed.username() != "" || parsed.password().is_some() || parsed.query().is_some() {
        return Err(Sub2ApiError::Config(
            "API 基地址不能包含账号、密码或查询参数".to_string(),
        ));
    }
    let scheme = parsed.scheme();
    let host = parsed.host_str().unwrap_or_default();
    if scheme != "https" && !(scheme == "http" && is_loopback(host)) {
        return Err(Sub2ApiError::Config(
            "生产 API 基地址必须使用 HTTPS（本地测试仅允许回环 HTTP）".to_string(),
        ));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn build_url(base: &str, path: &str) -> Result<Url, Sub2ApiError> {
    let normalized = normalize_base_url(base)?;
    let base_url = Url::parse(&(normalized + "/"))
        .map_err(|_| Sub2ApiError::Config("API 基地址无效".to_string()))?;
    base_url
        .join(path.trim_start_matches('/'))
        .map_err(|_| Sub2ApiError::Config("Sub2API 路径无效".to_string()))
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
}

fn validate_email_domain(raw: &str) -> Result<(), Sub2ApiError> {
    if raw.is_empty()
        || raw.len() > 253
        || raw.contains('/')
        || raw.contains('@')
        || raw.chars().any(char::is_control)
        || !raw
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
    {
        return Err(Sub2ApiError::Config("模拟邮箱域名无效".to_string()));
    }
    Ok(())
}

fn short_email(number: u64, domain: &str) -> Result<String, Sub2ApiError> {
    validate_email_domain(domain)?;
    Ok(format!("u{number}@{domain}"))
}

fn next_usb_number(users: &[UserRecord]) -> u64 {
    users
        .iter()
        .filter_map(|user| user.username.strip_prefix("USB-"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn pagination_pages(value: &Value) -> Option<u64> {
    match value {
        Value::Object(object) => object
            .get("pages")
            .and_then(value_u64)
            .or_else(|| object.values().find_map(pagination_pages)),
        Value::Array(items) => items.iter().find_map(pagination_pages),
        _ => None,
    }
}

fn parse_user_records(value: &Value) -> Vec<UserRecord> {
    let mut found = BTreeMap::new();
    collect_user_records(value, &mut found);
    found.into_values().collect()
}

fn target_group_id(value: &Value) -> Option<u64> {
    match value {
        Value::Array(items) => items.iter().find_map(target_group_id),
        Value::Object(object) => {
            let is_target_group = object
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == TARGET_GROUP_NAME)
                && object.get("id").and_then(value_u64).is_some()
                && object
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status.eq_ignore_ascii_case("active"));
            if is_target_group {
                return object.get("id").and_then(value_u64);
            }
            object.values().find_map(target_group_id)
        }
        _ => None,
    }
}

fn ensure_pending_license_digest(
    pending: &PendingPreload,
    current_license_digest: &str,
) -> Result<(), Sub2ApiError> {
    if current_license_digest.is_empty() || pending.license_digest != current_license_digest {
        return Err(Sub2ApiError::Pending(
            "待处理记录与当前许可证摘要不一致，已停止".to_string(),
        ));
    }
    Ok(())
}

fn collect_user_records(value: &Value, found: &mut BTreeMap<u64, UserRecord>) {
    match value {
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_user_records(item, found)),
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(value_u64) {
                let username = object
                    .get("username")
                    .or_else(|| object.get("userName"))
                    .or_else(|| object.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let email = object
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if !username.is_empty() || !email.is_empty() {
                    found.insert(
                        id,
                        UserRecord {
                            id,
                            username,
                            email,
                        },
                    );
                }
            }
            object
                .values()
                .for_each(|child| collect_user_records(child, found));
        }
        _ => {}
    }
}

fn find_id(value: &Value) -> Option<u64> {
    match value {
        Value::Object(object) => object
            .get("id")
            .and_then(value_u64)
            .or_else(|| object.values().find_map(find_id)),
        Value::Array(items) => items.iter().find_map(find_id),
        _ => None,
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(value) = object.get(*key).and_then(Value::as_str) {
                    if !value.is_empty() && !value.contains("****") {
                        return Some(value.to_string());
                    }
                }
            }
            object.values().find_map(|child| find_string(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| find_string(child, keys)),
        _ => None,
    }
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}

fn find_user_identity<'a>(
    users: &'a [UserRecord],
    asset_id: &str,
    email: &str,
) -> Option<&'a UserRecord> {
    users
        .iter()
        .find(|user| user.username == asset_id && user.email == email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;
    use std::thread;

    static CONFIG_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end;
        loop {
            let count = stream.read(&mut buffer).expect("read request");
            assert!(count > 0, "connection closed before request headers");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = index + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let count = stream.read(&mut buffer).expect("read request body");
            assert!(count > 0, "connection closed before request body");
            bytes.extend_from_slice(&buffer[..count]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn send_json(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    }

    fn send_error(stream: &mut TcpStream, status: &str) {
        let body = "{}";
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write error response");
    }

    #[test]
    fn defaults_are_simple_and_non_secret() {
        let config = Sub2ApiConfig::default();
        assert_eq!(config.base_url, DEFAULT_BASE_URL);
        assert_eq!(config.admin_api_key, default_admin_api_key());
        assert_eq!(
            short_email(12, DEFAULT_EMAIL_DOMAIN).unwrap(),
            "u12@u.404token.xyz"
        );
    }

    #[test]
    fn production_base_url_requires_https_but_loopback_http_is_testable() {
        assert!(normalize_base_url("https://404token.xyz/").is_ok());
        assert!(normalize_base_url("http://127.0.0.1:18080").is_ok());
        assert!(normalize_base_url("http://404token.xyz").is_err());
        assert!(normalize_base_url("https://user:pass@404token.xyz").is_err());
    }

    #[test]
    fn production_admin_credential_is_pinned_to_the_404token_origin() {
        let mut config = Sub2ApiConfig {
            admin_api_key: "test-key".to_string(),
            ..Sub2ApiConfig::default()
        };
        assert!(validate_trusted_admin_endpoint(&config).is_ok());

        config.base_url = "https://404token.xyz/".to_string();
        assert!(validate_trusted_admin_endpoint(&config).is_ok());

        for unsafe_destination in [
            "https://example.invalid",
            "https://404token.xyz.example.invalid",
            "https://404token.xyz:443",
            "https://404token.xyz/api/v1",
            "http://127.0.0.1:18080",
        ] {
            config.base_url = unsafe_destination.to_string();
            assert!(
                validate_trusted_admin_endpoint(&config).is_err(),
                "unexpectedly accepted admin destination: {unsafe_destination}"
            );
        }
    }

    #[test]
    fn preload_parser_is_bounded() {
        assert_eq!(parse_preload_amount("").unwrap(), 0.0);
        assert_eq!(parse_preload_amount("10.50").unwrap(), 10.5);
        assert!(parse_preload_amount("-1").is_err());
        assert!(parse_preload_amount("NaN").is_err());
        assert!(parse_preload_amount("1000001").is_err());
    }

    #[test]
    fn default_initial_password_is_fixed_and_validation_rejects_unsafe_values() {
        let password = generate_initial_password().expect("generate initial password");
        assert_eq!(password.as_str(), DEFAULT_INITIAL_PASSWORD);
        assert!(validate_initial_password(&password).is_ok());
        assert!(validate_initial_password("short").is_err());
        assert!(validate_initial_password("12345678\n").is_err());
        assert!(validate_initial_password(&"a".repeat(129)).is_err());
    }

    #[test]
    fn target_group_parser_requires_the_unified_internal_group() {
        assert_eq!(
            target_group_id(&json!({
                "data": [{
                    "id": 7,
                    "name": "unified-api-internal",
                    "platform": "openai",
                    "status": "active"
                }]
            })),
            Some(7)
        );
        assert_eq!(
            target_group_id(&json!({
                "data": [{
                    "id": 7,
                    "name": "Codex",
                    "platform": "openai",
                    "status": "active"
                }]
            })),
            None
        );
        assert_eq!(
            target_group_id(&json!({
                "data": [{
                    "id": 7,
                    "name": "unified-api-internal",
                    "platform": "openai",
                    "status": "inactive"
                }]
            })),
            None
        );
        assert_eq!(target_group_id(&json!({"data": []})), None);
    }

    #[test]
    fn pending_preload_requires_the_current_license_digest() {
        let pending = new_pending_preload(23, 10.5, "license-digest", "USB-0001").unwrap();
        assert!(ensure_pending_license_digest(&pending, "license-digest").is_ok());
        assert!(ensure_pending_license_digest(&pending, "different-digest").is_err());
    }

    #[test]
    fn repeated_binding_never_triggers_implicit_precharge() {
        assert!(should_precharge(10.0, false));
        assert!(!should_precharge(10.0, true));
        assert!(!should_precharge(0.0, false));
        assert!(!should_precharge(f64::NAN, false));
    }

    #[test]
    fn config_save_replaces_an_existing_file() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("usb-signer-config-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let first = Sub2ApiConfig {
            admin_api_key: "first-key".to_string(),
            ..Sub2ApiConfig::default()
        };
        let second = Sub2ApiConfig {
            admin_api_key: "second-key".to_string(),
            ..Sub2ApiConfig::default()
        };
        save_config(&first).expect("write first config");
        save_config(&second).expect("replace config");
        let raw = fs::read_to_string(config_path().unwrap()).expect("read config");
        assert!(!raw.contains("adminApiKey"));
        assert!(!raw.contains("first-key"));
        assert!(!raw.contains("second-key"));
        let loaded = load_config().expect("read replaced config");
        // A production build may carry the administrator credential in the
        // executable.  That managed value is authoritative over stale local
        // configuration, so a copied signer cannot silently keep using an
        // older key after deployment rotation.
        assert_eq!(loaded.admin_api_key, default_admin_api_key());
        assert_ne!(loaded.admin_api_key, "second-key");

        fs::write(
            config_path().unwrap(),
            r#"{"baseUrl":"https://404token.xyz","fakeEmailDomain":"u.404token.xyz","adminApiKey":"legacy-secret"}"#,
        )
        .expect("write legacy config");
        load_config().expect("migrate legacy config");
        let migrated = fs::read_to_string(config_path().unwrap()).expect("read migrated config");
        assert!(!migrated.contains("adminApiKey"));
        assert!(!migrated.contains("legacy-secret"));
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provision_recovery_record_is_dpapi_protected_and_round_trips() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("usb-signer-provision-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let recovery_key = make_provision_recovery_key(123, &"a".repeat(64), "CodexUsb");
        let pending = PendingProvisionRecord {
            schema_version: PROVISION_RECOVERY_SCHEMA_VERSION,
            recovery_key_digest: recovery_key_digest(&recovery_key).unwrap(),
            usb_asset_id: "USB-0042".to_string(),
            login_email: "u42@u.404token.xyz".to_string(),
            sub2api_user_id: 0,
            group_id: 7,
            password: "Test-only-USB-password-42".to_string(),
            provision_idempotency_key: Some(make_provision_idempotency_key("USB-0042")),
            device_api_key: None,
            user_api_key_idempotency_key: Some(make_user_api_key_idempotency_key("USB-0042")),
            created_at_unix_ms: 1,
        };
        let mut legacy_json = serde_json::to_value(&pending).expect("serialize current record");
        legacy_json
            .as_object_mut()
            .expect("record is an object")
            .remove("provisionIdempotencyKey");
        legacy_json
            .as_object_mut()
            .expect("record is an object")
            .remove("userApiKeyIdempotencyKey");
        let legacy: PendingProvisionRecord =
            serde_json::from_value(legacy_json).expect("read legacy recovery record");
        assert!(legacy.provision_idempotency_key.is_none());
        assert!(legacy.user_api_key_idempotency_key.is_none());
        save_pending_provision(&recovery_key, &pending).expect("save recovery record");
        let path = pending_provision_path(&recovery_key).unwrap();
        let raw = fs::read(&path).expect("read encrypted recovery record");
        assert!(!String::from_utf8_lossy(&raw).contains("Test-only-USB-password-42"));
        let mut linked = pending;
        linked.sub2api_user_id = 23;
        save_pending_provision(&recovery_key, &linked).expect("link created remote account");
        linked.device_api_key = Some("sk-device-key".to_string());
        save_pending_provision(&recovery_key, &linked).expect("save generated user key");
        let raw = fs::read(&path).expect("read encrypted completed recovery record");
        assert!(!String::from_utf8_lossy(&raw).contains("sk-device-key"));
        assert!(!String::from_utf8_lossy(&raw).contains("Test-only-USB-password-42"));
        let restored = load_pending_provision(&recovery_key).unwrap().unwrap();
        assert_eq!(restored.sub2api_user_id, 23);
        assert_eq!(
            restored.user_api_key_idempotency_key,
            linked.user_api_key_idempotency_key
        );
        clear_pending_provision(&recovery_key).expect("clear recovery record");
        assert!(!path.exists());

        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_preload_round_trip_contains_no_credentials() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("usb-signer-pending-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);

        let pending = new_pending_preload(23, 10.5, &"a".repeat(64), "USB-0001")
            .expect("create pending record");
        save_pending_preload(&pending).expect("save pending record");
        let path = pending_preload_path("USB-0001").expect("pending path");
        let raw = fs::read_to_string(path).expect("read pending record");
        assert!(!raw.contains("admin"));
        assert!(!raw.contains("password"));
        assert!(!raw.contains("apiKey"));
        assert_eq!(load_pending_preload("USB-0001").unwrap(), Some(pending));
        clear_pending_preload("USB-0001").expect("clear pending record");
        assert!(!has_pending_preload("USB-0001").unwrap());

        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_different_pending_operation_cannot_overwrite_an_unconfirmed_one() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("usb-signer-pending-guard-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);

        let first = new_pending_preload(23, 10.5, "first-license", "USB-0010").unwrap();
        let second = new_pending_preload(24, 20.0, "second-license", "USB-0010").unwrap();
        save_pending_preload(&first).expect("save first pending record");
        let error = save_pending_preload(&second).expect_err("reject overwrite");
        assert!(error.to_string().contains("另一条待处理"));
        assert_eq!(load_pending_preload("USB-0010").unwrap(), Some(first));

        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn next_number_never_reuses_a_deleted_suffix() {
        let value = json!([
            {"id": 1, "username": "USB-0001", "email": "u1@u.404token.xyz"},
            {"id": 2, "username": "USB-0007", "email": "u7@u.404token.xyz"},
            {"id": 3, "username": "ordinary", "email": "x@example.test"}
        ]);
        let users = parse_user_records(&value);
        assert_eq!(next_usb_number(&users), 8);
    }

    #[test]
    fn idempotency_keys_keep_scope_and_registration_keys_are_unique() {
        let first = make_idempotency_key("USB-0002", &"a".repeat(64));
        assert_eq!(first, make_idempotency_key("USB-0002", &"a".repeat(64)));
        assert!(first.starts_with("usb-preload-USB-0002-"));
        assert!(!first.contains("sk-"));

        let provision = make_provision_idempotency_key("USB-0002");
        let provision_retry = provision.clone();
        let next_provision = make_provision_idempotency_key("USB-0002");
        assert!(provision.starts_with("usb-provision-USB-0002-"));
        assert!(!provision.contains("password"));
        assert!(valid_provision_idempotency_key("USB-0002", &provision));
        assert_eq!(provision_retry, provision);
        assert_ne!(provision, next_provision);
        assert!(!valid_provision_idempotency_key("USB-0003", &provision));

        let user_key = make_user_api_key_idempotency_key("USB-0002");
        let retry_key = user_key.clone();
        let fresh_registration_key = make_user_api_key_idempotency_key("USB-0002");
        assert!(user_key.starts_with("usb-user-key-USB-0002-"));
        assert!(!user_key.contains("password"));
        assert!(valid_user_api_key_idempotency_key("USB-0002", &user_key));
        assert_eq!(
            retry_key, user_key,
            "a retry reuses the persisted operation key"
        );
        assert_ne!(
            user_key, fresh_registration_key,
            "a fresh registration must not reuse a prior server idempotency record"
        );
        assert!(!valid_user_api_key_idempotency_key("USB-0003", &user_key));
    }

    #[test]
    fn account_recovery_requires_both_usb_username_and_generated_email() {
        let users = vec![
            UserRecord {
                id: 1,
                username: "USB-0001".to_string(),
                email: "someone-else@example.test".to_string(),
            },
            UserRecord {
                id: 2,
                username: "ordinary-user".to_string(),
                email: "u1@u.404token.xyz".to_string(),
            },
            UserRecord {
                id: 3,
                username: "USB-0001".to_string(),
                email: "u1@u.404token.xyz".to_string(),
            },
        ];
        assert_eq!(
            find_user_identity(&users, "USB-0001", "u1@u.404token.xyz").map(|u| u.id),
            Some(3)
        );
        assert!(find_user_identity(&users[..2], "USB-0001", "u1@u.404token.xyz").is_none());
    }

    #[test]
    fn parser_accepts_nested_admin_response_shapes() {
        let value = json!({
            "data": {
                "items": [
                    {"id": "23", "username": "USB-0023", "email": "u23@u.404token.xyz"}
                ]
            }
        });
        let users = parse_user_records(&value);
        assert_eq!(users[0].id, 23);
        assert_eq!(next_usb_number(&users), 24);
    }

    #[test]
    fn pagination_parser_accepts_nested_page_count() {
        assert_eq!(pagination_pages(&json!({"data": {"pages": 3}})), Some(3));
        assert_eq!(pagination_pages(&json!({"data": []})), None);
    }

    #[test]
    fn list_users_fetches_all_declared_pages_before_allocating_a_number() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind pagination fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            for page in 1..=2 {
                let (mut stream, _) = listener.accept().expect("accept pagination request");
                let request = read_http_request(&mut stream);
                assert!(request.starts_with(&format!(
                    "GET /api/v1/admin/users?search=USB-&page={page}&page_size=1000 "
                )));
                if page == 1 {
                    send_json(
                        &mut stream,
                        r#"{"data":{"items":[{"id":1,"username":"USB-0001","email":"u1@u.404token.xyz"}]},"pages":2}"#,
                    );
                } else {
                    send_json(
                        &mut stream,
                        r#"{"data":{"items":[{"id":2,"username":"USB-0042","email":"u42@u.404token.xyz"}]},"pages":2}"#,
                    );
                }
            }
        });
        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "pagination-test-key".to_string(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        })
        .expect("fixture client");

        let users = client.list_usb_users().expect("list all pages");
        assert_eq!(users.len(), 2);
        assert_eq!(next_usb_number(&users), 43);
        handle.join().expect("pagination fixture thread");
    }

    #[test]
    fn live_http_contract_keeps_admin_key_out_of_user_requests_and_precharges_idempotently() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("usb-signer-http-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            for step in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
                let request = read_http_request(&mut stream);
                let request_lower = request.to_ascii_lowercase();
                match step {
                    0 => {
                        assert!(request.starts_with("GET /api/v1/admin/groups/all "));
                        assert!(request_lower.contains("x-api-key: admin-test-key"));
                        send_json(
                            &mut stream,
                            r#"{"data":[{"id":7,"name":"unified-api-internal","platform":"openai","status":"active"}]}"#,
                        );
                    }
                    1 => {
                        assert!(request.starts_with("GET /api/v1/admin/users?search=USB-"));
                        assert!(request_lower.contains("x-api-key: admin-test-key"));
                        send_json(&mut stream, r#"{"data":[]}"#);
                    }
                    2 => {
                        assert!(request.starts_with("POST /api/v1/admin/users "));
                        assert!(request_lower.contains("x-api-key: admin-test-key"));
                        assert!(request_lower.contains("idempotency-key: usb-provision-usb-0001-"));
                        assert!(request.contains("USB-0001"));
                        assert!(request.contains(DEFAULT_INITIAL_PASSWORD));
                        assert!(request.contains(r#""allowed_groups":[7]"#));
                        send_json(&mut stream, r#"{"id":23}"#);
                    }
                    3 => {
                        assert!(request.starts_with("POST /api/v1/auth/login "));
                        assert!(!request_lower.contains("admin-test-key"));
                        assert!(request.contains("u1@u.404token.xyz"));
                        send_json(&mut stream, r#"{"access_token":"user-token"}"#);
                    }
                    4 => {
                        assert!(request.starts_with("POST /api/v1/keys "));
                        assert!(request_lower.contains("authorization: bearer user-token"));
                        assert!(!request_lower.contains("admin-test-key"));
                        assert!(request_lower.contains("idempotency-key: usb-user-key-usb-0001-"));
                        assert!(request.contains(r#""name":"CCSwitch-USB-0001""#));
                        assert!(request.contains(r#""group_id":7"#));
                        send_json(&mut stream, r#"{"key":"sk-device-key"}"#);
                    }
                    5 => {
                        assert!(request.starts_with("POST /api/v1/admin/users/23/balance "));
                        assert!(request_lower.contains("x-api-key: admin-test-key"));
                        assert!(request_lower.contains("idempotency-key: usb-preload-usb-0001-"));
                        assert!(request.contains("\"operation\":\"add\""));
                        send_json(&mut stream, "{}");
                    }
                    _ => unreachable!(),
                }
            }
        });

        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "admin-test-key".to_string(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        })
        .expect("fixture client");
        let provisioned = client.provision_account(None).expect("provision account");
        assert_eq!(provisioned.binding.usb_asset_id, "USB-0001");
        assert_eq!(provisioned.binding.sub2api_user_id, 23);
        assert_eq!(provisioned.binding.ccswitch.device_api_key, "sk-device-key");
        assert_eq!(provisioned.binding.ccswitch.group_id, 7);
        let idempotency_key = make_idempotency_key("USB-0001", "license-digest");
        client
            .precharge_with_pending(23, 10.5, "license-digest", "USB-0001")
            .expect("precharge");
        assert!(!has_pending_preload("USB-0001").unwrap());
        assert_eq!(
            idempotency_key,
            make_idempotency_key("USB-0001", "license-digest")
        );
        handle.join().expect("fixture thread");
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_user_recovers_an_ambiguous_timeout_without_a_duplicate_post() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("usb-signer-http-recovery-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind recovery fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            for step in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept recovery request");
                let request = read_http_request(&mut stream);
                let request_lower = request.to_ascii_lowercase();
                match step {
                    0 => {
                        assert!(request.starts_with("GET /api/v1/admin/groups/all "));
                        send_json(
                            &mut stream,
                            r#"{"data":[{"id":7,"name":"unified-api-internal","status":"active"}]}"#,
                        );
                    }
                    1 => {
                        assert!(request.starts_with("GET /api/v1/admin/users?search=USB-"));
                        send_json(&mut stream, r#"{"data":[]}"#);
                    }
                    2 => {
                        assert!(request.starts_with("POST /api/v1/admin/users "));
                        assert!(request_lower.contains("idempotency-key: usb-provision-usb-0001-"));
                        // The server has accepted the request, but the client
                        // receives a transient response before the body.
                        send_error(&mut stream, "503 Service Unavailable");
                    }
                    3 => {
                        assert!(request.starts_with("GET /api/v1/admin/users?search=USB-"));
                        assert!(request_lower.contains("x-api-key: admin-test-key"));
                        send_json(
                            &mut stream,
                            r#"{"data":[{"id":23,"username":"USB-0001","email":"u1@u.404token.xyz"}]}"#,
                        );
                    }
                    4 => {
                        assert!(request.starts_with("POST /api/v1/auth/login "));
                        send_json(&mut stream, r#"{"access_token":"user-token"}"#);
                    }
                    5 => {
                        assert!(request.starts_with("POST /api/v1/keys "));
                        assert!(request_lower.contains("authorization: bearer user-token"));
                        assert!(request_lower.contains("idempotency-key: usb-user-key-usb-0001-"));
                        send_json(&mut stream, r#"{"key":"sk-device-key"}"#);
                    }
                    _ => unreachable!(),
                }
            }
        });

        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "admin-test-key".to_string(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        })
        .expect("fixture client");
        let provisioned = client.provision_account(None).expect("recover account");
        assert_eq!(provisioned.binding.sub2api_user_id, 23);
        assert_eq!(provisioned.binding.ccswitch.device_api_key, "sk-device-key");
        handle.join().expect("recovery fixture thread");
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn key_creation_retry_reuses_the_same_idempotency_key_and_saved_account() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("usb-signer-key-recovery-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let recovery_key = make_provision_recovery_key(123, &"a".repeat(64), "CodexUsb");
        let fixture_recovery_key = recovery_key.clone();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind key recovery fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            let mut first_key_idempotency = None;
            for step in 0..8 {
                let (mut stream, _) = listener.accept().expect("accept key recovery request");
                let request = read_http_request(&mut stream);
                let request_lower = request.to_ascii_lowercase();
                match step {
                    0 | 5 => {
                        assert!(request.starts_with("GET /api/v1/admin/groups/all "));
                        send_json(
                            &mut stream,
                            r#"{"data":[{"id":7,"name":"unified-api-internal","status":"active"}]}"#,
                        );
                    }
                    1 => {
                        assert!(request.starts_with("GET /api/v1/admin/users?search=USB-"));
                        send_json(&mut stream, r#"{"data":[]}"#);
                    }
                    2 => {
                        assert!(request.starts_with("POST /api/v1/admin/users "));
                        assert!(request_lower.contains("idempotency-key: usb-provision-usb-0001-"));
                        let persisted = load_pending_provision(&fixture_recovery_key)
                            .expect("load write-ahead recovery record")
                            .expect("intent saved before account POST");
                        assert_eq!(persisted.usb_asset_id, "USB-0001");
                        assert_eq!(persisted.login_email, "u1@u.404token.xyz");
                        assert_eq!(persisted.sub2api_user_id, 0);
                        assert!(persisted.device_api_key.is_none());
                        let request_idempotency = request_lower
                            .lines()
                            .find_map(|line| line.strip_prefix("idempotency-key:"))
                            .expect("create-user idempotency header")
                            .trim();
                        assert_eq!(
                            persisted
                                .provision_idempotency_key
                                .as_deref()
                                .map(str::to_ascii_lowercase)
                                .as_deref(),
                            Some(request_idempotency),
                            "the exact account-create key is durably recorded before POST"
                        );
                        send_json(&mut stream, r#"{"id":23}"#);
                    }
                    3 | 6 => {
                        assert!(request.starts_with("POST /api/v1/auth/login "));
                        send_json(&mut stream, r#"{"access_token":"user-token"}"#);
                    }
                    4 => {
                        assert!(request.starts_with("POST /api/v1/keys "));
                        let key = request_lower
                            .lines()
                            .find_map(|line| line.strip_prefix("idempotency-key:"))
                            .expect("API key idempotency header")
                            .trim()
                            .to_string();
                        first_key_idempotency = Some(key);
                        let pending = load_pending_provision(&fixture_recovery_key)
                            .expect("read pending API key operation")
                            .expect("pending record retained before API key POST");
                        assert_eq!(
                            pending
                                .user_api_key_idempotency_key
                                .as_deref()
                                .map(str::to_ascii_lowercase)
                                .as_deref(),
                            first_key_idempotency.as_deref(),
                            "the exact key sent to the provider is durably recorded first"
                        );
                        send_error(&mut stream, "503 Service Unavailable");
                    }
                    7 => {
                        assert!(request.starts_with("POST /api/v1/keys "));
                        let key = request_lower
                            .lines()
                            .find_map(|line| line.strip_prefix("idempotency-key:"))
                            .expect("retry API key idempotency header")
                            .trim();
                        assert_eq!(Some(key.to_string()), first_key_idempotency);
                        send_json(&mut stream, r#"{"key":"sk-device-key"}"#);
                    }
                    _ => unreachable!(),
                }
            }
        });

        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "admin-test-key".to_string(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        })
        .expect("fixture client");
        assert!(client
            .provision_account_with_recovery(None, None, Some(&recovery_key))
            .is_err());
        let pending = load_pending_provision(&recovery_key)
            .expect("load pending after ambiguous key response")
            .expect("pending record retained");
        assert_eq!(pending.sub2api_user_id, 23);
        assert!(pending.device_api_key.is_none());

        let provisioned = client
            .provision_account_with_recovery(None, None, Some(&recovery_key))
            .expect("resume API key operation");
        assert!(provisioned.recovered);
        assert_eq!(provisioned.binding.sub2api_user_id, 23);
        assert_eq!(provisioned.binding.ccswitch.device_api_key, "sk-device-key");
        handle.join().expect("key recovery fixture thread");
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn login_401_recovers_stale_deleted_account_idempotency_result() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("usb-signer-login-recovery-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let recovery_key = make_provision_recovery_key(123, &"b".repeat(64), "CodexUsb");
        let stale_provision_key = format!("usb-provision-USB-0001-{}", "a".repeat(32));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stale account fixture");
        let address = listener.local_addr().expect("fixture address");
        let fixture_recovery_key = recovery_key.clone();
        let fixture_stale_provision_key = stale_provision_key.clone();
        let handle = thread::spawn(move || {
            for step in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept stale account request");
                let request = read_http_request(&mut stream);
                let request_lower = request.to_ascii_lowercase();
                match step {
                    0 => {
                        assert!(request.starts_with("GET /api/v1/admin/groups/all "));
                        send_json(
                            &mut stream,
                            r#"{"data":[{"id":7,"name":"unified-api-internal","status":"active"}]}"#,
                        );
                    }
                    1 => {
                        assert!(request.starts_with("POST /api/v1/auth/login "));
                        send_error(&mut stream, "401 Unauthorized");
                    }
                    2 => {
                        assert!(request.starts_with("GET /api/v1/admin/users?search=USB-"));
                        send_json(&mut stream, r#"{"data":[]}"#);
                    }
                    3 => {
                        assert!(request.starts_with("POST /api/v1/admin/users "));
                        let key = request_lower
                            .lines()
                            .find_map(|line| line.strip_prefix("idempotency-key:"))
                            .expect("replacement create-user idempotency key")
                            .trim();
                        assert!(key.starts_with("usb-provision-usb-0001-"));
                        assert_ne!(key, fixture_stale_provision_key);
                        let pending = load_pending_provision(&fixture_recovery_key)
                            .expect("load repaired recovery record")
                            .expect("recovery record remains available");
                        assert_eq!(pending.sub2api_user_id, 0);
                        assert_ne!(
                            pending.provision_idempotency_key.as_deref(),
                            Some(fixture_stale_provision_key.as_str())
                        );
                        send_json(&mut stream, r#"{"id":24}"#);
                    }
                    4 => {
                        assert!(request.starts_with("POST /api/v1/auth/login "));
                        send_json(&mut stream, r#"{"access_token":"recovered-user-token"}"#);
                    }
                    5 => {
                        assert!(request.starts_with("POST /api/v1/keys "));
                        assert!(
                            request_lower.contains("authorization: bearer recovered-user-token")
                        );
                        send_json(&mut stream, r#"{"key":"sk-recovered-device-key"}"#);
                    }
                    _ => unreachable!(),
                }
            }
        });

        let stale_pending = PendingProvisionRecord {
            schema_version: PROVISION_RECOVERY_SCHEMA_VERSION,
            recovery_key_digest: recovery_key_digest(&recovery_key).unwrap(),
            usb_asset_id: "USB-0001".to_string(),
            login_email: "u1@u.404token.xyz".to_string(),
            sub2api_user_id: 23,
            group_id: 7,
            password: DEFAULT_INITIAL_PASSWORD.to_string(),
            provision_idempotency_key: Some(stale_provision_key),
            device_api_key: None,
            user_api_key_idempotency_key: Some(format!("usb-user-key-USB-0001-{}", "b".repeat(32))),
            created_at_unix_ms: 1,
        };
        save_pending_provision(&recovery_key, &stale_pending)
            .expect("save old-format pending state");
        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "admin-test-key".to_string(),
            fake_email_domain: DEFAULT_EMAIL_DOMAIN.to_string(),
        })
        .expect("fixture client");
        let provisioned = client
            .provision_account_with_recovery(None, None, Some(&recovery_key))
            .expect("recreate account after stale deleted-user id");
        assert_eq!(provisioned.binding.sub2api_user_id, 24);
        assert_eq!(
            provisioned.binding.ccswitch.device_api_key,
            "sk-recovered-device-key"
        );
        handle.join().expect("stale account fixture thread");
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_precharge_keeps_the_same_pending_record_for_retry() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("usb-signer-pending-failure-{}", Uuid::new_v4()));
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &root);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fixture request");
            let _request = read_http_request(&mut stream);
            send_error(&mut stream, "503 Service Unavailable");
        });
        let client = Sub2ApiClient::new(Sub2ApiConfig {
            base_url: format!("http://{address}"),
            admin_api_key: "admin-test-key".to_string(),
            ..Sub2ApiConfig::default()
        })
        .expect("fixture client");
        let result = client.precharge_with_pending(23, 10.5, "license-digest", "USB-0009");
        assert!(result.is_err());
        let pending = load_pending_preload("USB-0009")
            .unwrap()
            .expect("pending record retained");
        assert_eq!(
            pending.idempotency_key,
            make_idempotency_key("USB-0009", "license-digest")
        );
        handle.join().expect("fixture thread");
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
