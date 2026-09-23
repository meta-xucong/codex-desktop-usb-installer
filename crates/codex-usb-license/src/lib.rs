//! Stable USB license schema and cryptographic core.
//!
//! The JSON types in this crate intentionally mirror the public types in the
//! existing USB guard.  Keep field order and the `anchorSchema` default
//! stable: the order is part of the v1 license signing format.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const LICENSE_SCHEMA_VERSION: u32 = 1;
pub const ANCHOR_SCHEMA_V1: u32 = 1;
pub const ANCHOR_SCHEMA_V2: u32 = 2;
pub const ANCHOR_SCHEMA_V3: u32 = 3;
pub const PRODUCT_ID: &str = "codex-usb-oneclick";
pub const LICENSE_FILE_NAME: &str = "usb-license.json";

/// Errors are deliberately owned by this crate so an adapter can map them to
/// its application error type without making the core depend on the manager.
#[derive(Debug, Error)]
pub enum LicenseError {
    #[error("license JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("license I/O error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid {what} Base64: {source}")]
    Base64 {
        what: &'static str,
        #[source]
        source: base64::DecodeError,
    },
    #[error("invalid public key: {0}")]
    PublicKey(ed25519_dalek::SignatureError),
    #[error("public key must be 32 bytes")]
    InvalidPublicKeyLength,
    #[error("license signature is missing")]
    MissingSignature,
    #[error("license signature is already present")]
    ExistingSignature,
    #[error("license signature verification failed")]
    InvalidSignature,
    #[error("license schemaVersion must be {expected}, got {actual}")]
    InvalidLicenseSchema { expected: u32, actual: u32 },
    #[error("license product must be {expected}, got {actual}")]
    InvalidProduct {
        expected: &'static str,
        actual: String,
    },
    #[error("unsupported anchor schema: {0}")]
    UnsupportedAnchorSchema(u32),
    #[error("invalid anchor SHA-256: {0}")]
    InvalidAnchorSha256(String),
    #[error("invalid bundle-relative path: {0}")]
    InvalidBundleRelativePath(String),
    #[error("invalid SHA256SUMS.txt: {0}")]
    InvalidChecksumFile(String),
    #[error("keyId is required for a formal signed license")]
    MissingKeyId,
    #[error("invalid keyId: {0}")]
    InvalidKeyId(String),
    #[error("license fingerprint does not match the observed fingerprint")]
    FingerprintMismatch,
    #[error("UUID installationId is invalid: {0}")]
    InvalidInstallationId(String),
    #[error("issuedAt must be an unsigned decimal millisecond timestamp: {0}")]
    InvalidIssuedAt(String),
    #[error("new signed licenses must use anchor schema v2")]
    NewSignatureRequiresV2,
    #[error("usb-manifest schemaVersion must be an integer")]
    InvalidAnchorSchemaType,
    #[error("invalid account binding: {0}")]
    InvalidAccountBinding(String),
}

pub type Result<T> = std::result::Result<T, LicenseError>;

fn default_anchor_schema() -> u32 {
    ANCHOR_SCHEMA_V1
}

/// Physical identity of a bundle on a removable volume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbFingerprint {
    pub volume_serial: u32,
    pub anchor_sha256: String,
    pub bundle_relative_path: String,
    /// Missing in old v1 JSON.  It is intentionally a serde default rather
    /// than an optional field so new serialization always emits the field.
    #[serde(default = "default_anchor_schema")]
    pub anchor_schema: u32,
}

/// Optional 404token/Sub2API account binding issued by the trusted USB
/// signer.  It is part of the signed payload when present, but its absence
/// remains valid for legacy licenses and for bundles that do not use the
/// account/CCSwitch integration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbAccountBinding {
    pub schema_version: u32,
    pub usb_asset_id: String,
    pub account_ref: String,
    pub account_name: String,
    pub login_email: String,
    pub sub2api_user_id: u64,
    pub ccswitch: UsbCcswitchBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbCcswitchBinding {
    pub provider_id: String,
    pub credential_id: String,
    pub device_api_key: String,
    pub group_id: u64,
}

/// Public USB license schema.  `signature` is excluded from the unsigned
/// signing payload, while `keyId` remains part of that payload even when it is
/// null in an old unsigned/legacy value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbLicense {
    pub schema_version: u32,
    pub product: String,
    pub installation_id: String,
    pub fingerprint: UsbFingerprint,
    pub issued_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_binding: Option<UsbAccountBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Return whether an anchor schema is understood by this version of the
/// adapter core.
pub fn is_supported_anchor_schema(anchor_schema: u32) -> bool {
    matches!(
        anchor_schema,
        ANCHOR_SCHEMA_V1 | ANCHOR_SCHEMA_V2 | ANCHOR_SCHEMA_V3
    )
}

/// Validate the fields that make up a physical fingerprint.  Hex case is
/// accepted for legacy data; newly generated hashes are lower-case.
pub fn validate_fingerprint(fingerprint: &UsbFingerprint) -> Result<()> {
    if !is_supported_anchor_schema(fingerprint.anchor_schema) {
        return Err(LicenseError::UnsupportedAnchorSchema(
            fingerprint.anchor_schema,
        ));
    }
    if fingerprint.anchor_sha256.len() != 64
        || !fingerprint
            .anchor_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(LicenseError::InvalidAnchorSha256(
            fingerprint.anchor_sha256.clone(),
        ));
    }
    validate_bundle_relative_path(&fingerprint.bundle_relative_path)
}

/// Validate a key identifier without making it a filesystem path or command
/// argument.  The format stays deliberately narrower than arbitrary Unicode
/// so reports and key-blob filenames remain unambiguous.
pub fn validate_key_id(key_id: &str) -> Result<()> {
    if key_id.is_empty()
        || key_id.len() > 128
        || key_id.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || character == '/'
                || character == '\\'
        })
        || !key_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(LicenseError::InvalidKeyId(key_id.to_string()));
    }
    Ok(())
}

fn validate_bundle_relative_path(path: &str) -> Result<()> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(LicenseError::InvalidBundleRelativePath(path.to_string()));
    }
    Ok(())
}

/// Compare two physical identities.  This preserves the existing guard's
/// case-insensitive comparison for hashes and relative paths.
pub fn fingerprints_match(expected: &UsbFingerprint, actual: &UsbFingerprint) -> bool {
    is_supported_anchor_schema(expected.anchor_schema)
        && is_supported_anchor_schema(actual.anchor_schema)
        && expected.anchor_schema == actual.anchor_schema
        && expected.volume_serial == actual.volume_serial
        && expected
            .anchor_sha256
            .eq_ignore_ascii_case(&actual.anchor_sha256)
        && expected
            .bundle_relative_path
            .eq_ignore_ascii_case(&actual.bundle_relative_path)
}

/// Build the current compact JSON signing payload.  This is intentionally a
/// dedicated struct instead of serializing `UsbLicense`: it excludes the
/// signature and retains the existing serde field order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedLicense<'a> {
    schema_version: u32,
    product: &'a str,
    installation_id: &'a str,
    fingerprint: &'a UsbFingerprint,
    issued_at: &'a str,
    key_id: &'a Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_binding: &'a Option<UsbAccountBinding>,
}

#[derive(Serialize)]
struct LegacyUsbFingerprint<'a> {
    volume_serial: u32,
    anchor_sha256: &'a str,
    bundle_relative_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyUnsignedLicense<'a> {
    schema_version: u32,
    product: &'a str,
    installation_id: &'a str,
    fingerprint: LegacyUsbFingerprint<'a>,
    issued_at: &'a str,
    key_id: &'a Option<String>,
}

/// Serialize the current unsigned payload exactly as the existing guard does.
pub fn unsigned_license_payload(license: &UsbLicense) -> Result<Vec<u8>> {
    serde_json::to_vec(&UnsignedLicense {
        schema_version: license.schema_version,
        product: &license.product,
        installation_id: &license.installation_id,
        fingerprint: &license.fingerprint,
        issued_at: &license.issued_at,
        key_id: &license.key_id,
        account_binding: &license.account_binding,
    })
    .map_err(LicenseError::from)
}

/// Serialize the historical v1 payload which omitted fingerprint.anchorSchema.
pub fn legacy_unsigned_license_payload(license: &UsbLicense) -> Result<Vec<u8>> {
    serde_json::to_vec(&LegacyUnsignedLicense {
        schema_version: license.schema_version,
        product: &license.product,
        installation_id: &license.installation_id,
        fingerprint: LegacyUsbFingerprint {
            volume_serial: license.fingerprint.volume_serial,
            anchor_sha256: &license.fingerprint.anchor_sha256,
            bundle_relative_path: &license.fingerprint.bundle_relative_path,
        },
        issued_at: &license.issued_at,
        key_id: &license.key_id,
    })
    .map_err(LicenseError::from)
}

fn validate_license_metadata(license: &UsbLicense) -> Result<()> {
    if license.schema_version != LICENSE_SCHEMA_VERSION {
        return Err(LicenseError::InvalidLicenseSchema {
            expected: LICENSE_SCHEMA_VERSION,
            actual: license.schema_version,
        });
    }
    if license.product != PRODUCT_ID {
        return Err(LicenseError::InvalidProduct {
            expected: PRODUCT_ID,
            actual: license.product.clone(),
        });
    }
    validate_fingerprint(&license.fingerprint)?;
    if Uuid::parse_str(&license.installation_id).is_err() {
        return Err(LicenseError::InvalidInstallationId(
            license.installation_id.clone(),
        ));
    }
    if license.issued_at.parse::<u64>().is_err() {
        return Err(LicenseError::InvalidIssuedAt(license.issued_at.clone()));
    }
    if let Some(key_id) = license.key_id.as_deref() {
        validate_key_id(key_id)?;
    }
    if let Some(binding) = license.account_binding.as_ref() {
        validate_account_binding(binding)?;
    }
    Ok(())
}

/// Validate account-binding fields before they are signed or consumed.  The
/// checks deliberately reject control characters and path-like values so a
/// bound credential cannot become an unsafe UI/log/config fragment.
pub fn validate_account_binding(binding: &UsbAccountBinding) -> Result<()> {
    if binding.schema_version != 1 {
        return Err(LicenseError::InvalidAccountBinding(format!(
            "schemaVersion must be 1, got {}",
            binding.schema_version
        )));
    }
    for (field, value) in [
        ("usbAssetId", binding.usb_asset_id.as_str()),
        ("accountRef", binding.account_ref.as_str()),
        ("accountName", binding.account_name.as_str()),
        ("loginEmail", binding.login_email.as_str()),
        ("providerId", binding.ccswitch.provider_id.as_str()),
        ("credentialId", binding.ccswitch.credential_id.as_str()),
        ("deviceApiKey", binding.ccswitch.device_api_key.as_str()),
    ] {
        if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(LicenseError::InvalidAccountBinding(format!(
                "{field} is empty, too long, or contains control characters"
            )));
        }
    }
    if binding.usb_asset_id != binding.account_ref || binding.usb_asset_id != binding.account_name {
        return Err(LicenseError::InvalidAccountBinding(
            "usbAssetId, accountRef, and accountName must match".to_string(),
        ));
    }
    let asset_suffix = binding
        .usb_asset_id
        .strip_prefix("USB-")
        .unwrap_or_default();
    if asset_suffix.is_empty()
        || !asset_suffix
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(LicenseError::InvalidAccountBinding(
            "usbAssetId must use the USB-<digits> format".to_string(),
        ));
    }
    if !binding.login_email.contains('@') {
        return Err(LicenseError::InvalidAccountBinding(
            "loginEmail must contain @".to_string(),
        ));
    }
    if binding.sub2api_user_id == 0 || binding.ccswitch.group_id == 0 {
        return Err(LicenseError::InvalidAccountBinding(
            "sub2apiUserId and ccswitch.groupId must be positive".to_string(),
        ));
    }
    Ok(())
}

/// Sign a license in place.  The caller must set `key_id` before calling this
/// function; requiring it here makes it impossible to accidentally sign a
/// formal license whose key identifier was omitted from the payload.
pub fn sign_license(license: &mut UsbLicense, signing_key: &SigningKey) -> Result<()> {
    validate_license_metadata(license)?;
    if license.fingerprint.anchor_schema == ANCHOR_SCHEMA_V1 {
        return Err(LicenseError::NewSignatureRequiresV2);
    }
    let key_id = license
        .key_id
        .as_deref()
        .ok_or(LicenseError::MissingKeyId)?;
    validate_key_id(key_id)?;
    if license.signature.is_some() {
        return Err(LicenseError::ExistingSignature);
    }
    let payload = unsigned_license_payload(license)?;
    let signature = signing_key.sign(&payload);
    license.signature = Some(BASE64.encode(signature.to_bytes()));
    Ok(())
}

/// Verify a signature against an explicitly supplied Base64 Ed25519 public
/// key.  The current payload is tried first; a v1 fingerprint additionally
/// gets the historical payload fallback for old signed bundles.
pub fn verify_signature_with_key(license: &UsbLicense, public_key_b64: &str) -> Result<()> {
    validate_license_metadata(license)?;
    let signature = license
        .signature
        .as_deref()
        .ok_or(LicenseError::MissingSignature)?;
    let key_bytes =
        BASE64
            .decode(public_key_b64.trim())
            .map_err(|source| LicenseError::Base64 {
                what: "public key",
                source,
            })?;
    let key_array: [u8; 32] = key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| LicenseError::InvalidPublicKeyLength)?;
    let key = VerifyingKey::from_bytes(&key_array).map_err(LicenseError::PublicKey)?;
    let sig_bytes = BASE64
        .decode(signature.trim())
        .map_err(|source| LicenseError::Base64 {
            what: "signature",
            source,
        })?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| LicenseError::InvalidSignature)?;
    let sig = Signature::from_bytes(&sig_array);
    let payload = unsigned_license_payload(license)?;
    if key.verify(&payload, &sig).is_ok() {
        return Ok(());
    }
    if license.fingerprint.anchor_schema == ANCHOR_SCHEMA_V1 {
        let legacy_payload = legacy_unsigned_license_payload(license)?;
        if key.verify(&legacy_payload, &sig).is_ok() {
            return Ok(());
        }
    }
    Err(LicenseError::InvalidSignature)
}

/// Validate the schema and physical identity, then verify when a public key
/// is supplied.  `Ok(false)` means a valid unsigned development license.
pub fn validate_license(
    license: &UsbLicense,
    expected: &UsbFingerprint,
    public_key_b64: Option<&str>,
) -> Result<bool> {
    validate_license_metadata(license)?;
    validate_fingerprint(expected)?;
    if !fingerprints_match(&license.fingerprint, expected) {
        return Err(LicenseError::FingerprintMismatch);
    }
    match public_key_b64 {
        Some(public_key) => {
            verify_signature_with_key(license, public_key)?;
            Ok(true)
        }
        None => Ok(license.signature.is_some()),
    }
}

/// Construct a schema-compatible unsigned license for a newly provisioned
/// bundle.  Signers normally preserve an existing installationId instead.
pub fn new_unsigned_license(fingerprint: UsbFingerprint, issued_at_ms: u64) -> UsbLicense {
    UsbLicense {
        schema_version: LICENSE_SCHEMA_VERSION,
        product: PRODUCT_ID.to_string(),
        installation_id: Uuid::new_v4().to_string(),
        fingerprint,
        issued_at: issued_at_ms.to_string(),
        key_id: None,
        account_binding: None,
        signature: None,
    }
}

fn io_error(path: &Path, source: std::io::Error) -> LicenseError {
    LicenseError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn update_anchor_part(hasher: &mut Sha256, name: &str, bytes: &[u8]) {
    hasher.update(name.as_bytes());
    hasher.update([0u8]);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn finish_anchor(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Preserve the original v1 anchor: raw manifest bytes and historical framing.
pub fn raw_anchor_sha256(bundle_root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    for name in ["manifest.json", "usb-manifest.json"] {
        let path = bundle_root.join(name);
        let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
        update_anchor_part(&mut hasher, name, &bytes);
    }
    Ok(finish_anchor(hasher))
}

fn read_anchor_json(bundle_root: &Path, name: &str) -> Result<serde_json::Value> {
    let path = bundle_root.join(name);
    let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
    serde_json::from_slice(&bytes).map_err(LicenseError::from)
}

fn remove_v2_mutable_fields(value: &mut serde_json::Value, name: &str) {
    match name {
        "manifest.json" => {
            if let Some(object) = value.as_object_mut() {
                for key in ["codexDesktopVersion", "packageVersion", "releaseTag"] {
                    object.remove(key);
                }
            }
            if let Some(payload) = value
                .get_mut("payload")
                .and_then(serde_json::Value::as_object_mut)
            {
                payload.remove("sha256");
                payload.remove("size");
            }
        }
        "usb-manifest.json" => {
            if let Some(codex_installer) = value
                .get_mut("codexInstaller")
                .and_then(serde_json::Value::as_object_mut)
            {
                codex_installer.remove("payloadSha256");
                codex_installer.remove("payloadSize");
            }
        }
        _ => {}
    }
}

fn write_canonical_json(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {
            serde_json::to_writer(output, value).map_err(LicenseError::from)
        }
        serde_json::Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
            Ok(())
        }
        serde_json::Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            output.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key).map_err(LicenseError::from)?;
                output.push(b':');
                write_canonical_json(value, output)?;
            }
            output.push(b'}');
            Ok(())
        }
    }
}

fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn canonical_checksum_rows(bundle_root: &Path, payload_relative_path: &str) -> Result<Vec<u8>> {
    let path = bundle_root.join("SHA256SUMS.txt");
    let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
    canonical_checksum_rows_from_bytes(&bytes, payload_relative_path, ChecksumRowsMode::LegacyV2)
}

#[derive(Clone, Copy)]
enum ChecksumRowsMode {
    LegacyV2,
    StableUpdateV1,
}

fn canonical_checksum_rows_from_bytes(
    bytes: &[u8],
    payload_relative_path: &str,
    mode: ChecksumRowsMode,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| LicenseError::InvalidChecksumFile("文件必须是 UTF-8 文本".to_string()))?;
    let payload_relative_path = payload_relative_path.trim().replace('\\', "/");
    validate_bundle_relative_path(&payload_relative_path)?;
    let payload_relative_path = payload_relative_path.to_ascii_lowercase();
    let backup_directory = payload_relative_path
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/_backup"))
        .unwrap_or_else(|| "_backup".to_string());
    let backup_prefix = format!("{backup_directory}/");
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let mut saw_payload = false;
    let mut saw_manifest = false;
    let mut saw_usb_manifest = false;
    let mut saw_backup = false;
    let mut stable_markers = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (raw_hash, raw_relative_path) = line
            .split_once(|character: char| character.is_ascii_whitespace())
            .ok_or_else(|| {
                LicenseError::InvalidChecksumFile(format!("缺少哈希与路径分隔符：{line}"))
            })?;
        let hash = raw_hash.trim().to_ascii_lowercase();
        if hash.len() != 64 || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(LicenseError::InvalidChecksumFile(format!(
                "SHA-256 不合法：{raw_hash}"
            )));
        }
        let relative_path = raw_relative_path
            .trim()
            .strip_prefix('*')
            .unwrap_or(raw_relative_path.trim())
            .trim()
            .replace('\\', "/");
        validate_bundle_relative_path(&relative_path)?;
        let relative_path = relative_path.to_ascii_lowercase();
        let file_name = relative_path.rsplit('/').next().unwrap_or_default();
        if file_name.eq_ignore_ascii_case(LICENSE_FILE_NAME)
            || file_name.eq_ignore_ascii_case("SHA256SUMS.txt")
            || file_name.ends_with(".bak")
        {
            return Err(LicenseError::InvalidChecksumFile(format!(
                "禁止把授权/校验清单/备份文件写入哈希清单：{relative_path}"
            )));
        }
        if !seen.insert(relative_path.clone()) {
            return Err(LicenseError::InvalidChecksumFile(format!(
                "路径重复：{relative_path}"
            )));
        }
        if relative_path == payload_relative_path {
            saw_payload = true;
            if matches!(mode, ChecksumRowsMode::StableUpdateV1) {
                stable_markers.push(format!("MUTABLE  payload  {relative_path}\n"));
            }
            continue;
        }
        if matches!(mode, ChecksumRowsMode::StableUpdateV1) {
            if relative_path == "manifest.json" {
                saw_manifest = true;
                stable_markers.push("MUTABLE  manifest  manifest.json\n".to_string());
                continue;
            }
            if relative_path == "usb-manifest.json" {
                saw_usb_manifest = true;
                stable_markers.push("MUTABLE  usb-manifest  usb-manifest.json\n".to_string());
                continue;
            }
            if relative_path.starts_with(&backup_prefix) {
                if saw_backup {
                    return Err(LicenseError::InvalidChecksumFile(
                        "schema 3 只允许一条 payload backup 行".to_string(),
                    ));
                }
                saw_backup = true;
                stable_markers.push(format!("MUTABLE  backup  {backup_directory}/\n"));
                continue;
            }
        }
        rows.push((relative_path, hash));
    }

    if !saw_payload {
        return Err(LicenseError::InvalidChecksumFile(format!(
            "缺少当前 payload 行：{payload_relative_path}"
        )));
    }
    if matches!(mode, ChecksumRowsMode::StableUpdateV1) {
        if !saw_manifest || !saw_usb_manifest {
            return Err(LicenseError::InvalidChecksumFile(
                "schema 3 必须列出 manifest.json 与 usb-manifest.json".to_string(),
            ));
        }
        // A first legal update may create the single backup file even when
        // the originally signed bundle has no backup row yet.  Anchor the
        // authorized backup slot itself, rather than its current occupancy,
        // so that this expected lifecycle transition does not invalidate the
        // license.  A second row is still rejected above.
        if !saw_backup {
            stable_markers.push(format!("MUTABLE  backup  {backup_directory}/\n"));
        }
        stable_markers.sort_unstable();
    }
    rows.sort_unstable();
    let mut canonical = Vec::new();
    for (relative_path, hash) in rows {
        if matches!(mode, ChecksumRowsMode::StableUpdateV1) {
            canonical.extend_from_slice(b"IMMUTABLE  ");
        }
        canonical.extend_from_slice(hash.as_bytes());
        canonical.extend_from_slice(b"  ");
        canonical.extend_from_slice(relative_path.as_bytes());
        canonical.push(b'\n');
    }
    if matches!(mode, ChecksumRowsMode::StableUpdateV1) {
        let mut stable = canonical;
        stable.extend(stable_markers.into_iter().flat_map(String::into_bytes));
        let mut lines = stable
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| line.to_vec())
            .collect::<Vec<_>>();
        lines.sort_unstable();
        let mut sorted = Vec::new();
        for line in lines {
            sorted.extend_from_slice(&line);
            sorted.push(b'\n');
        }
        return Ok(sorted);
    }
    Ok(canonical)
}

/// Hash the v2 canonical representation.  Only the explicitly allow-listed
/// mutable release metadata is removed; every other JSON value participates.
pub fn canonical_anchor_sha256(bundle_root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut payload_relative_path = None;
    for name in ["manifest.json", "usb-manifest.json"] {
        let mut value = read_anchor_json(bundle_root, name)?;
        if name == "manifest.json" {
            let payload_path = value
                .pointer("/payload/path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    LicenseError::InvalidChecksumFile(
                        "manifest.json 缺少 payload.path，无法确定动态 payload 行".to_string(),
                    )
                })?;
            validate_bundle_relative_path(payload_path)?;
            payload_relative_path = Some(payload_path.to_string());
        }
        remove_v2_mutable_fields(&mut value, name);
        let canonical = canonical_json_bytes(&value)?;
        update_anchor_part(&mut hasher, name, &canonical);
    }
    let payload_relative_path = payload_relative_path.ok_or_else(|| {
        LicenseError::InvalidChecksumFile("无法确定当前 payload 路径".to_string())
    })?;
    let canonical_sums = canonical_checksum_rows(bundle_root, &payload_relative_path)?;
    update_anchor_part(&mut hasher, "SHA256SUMS.txt", &canonical_sums);
    Ok(finish_anchor(hasher))
}

/// Hash the schema 3 stable-update-v1 representation from only the three
/// small metadata files.  The current payload bytes are deliberately not an
/// input: SHA256SUMS still carries its payload row for the full verifier, but
/// that row is outside this update-stability anchor.
pub fn stable_anchor_sha256_from_bytes(
    manifest_bytes: &[u8],
    usb_manifest_bytes: &[u8],
    checksum_bytes: &[u8],
) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut manifest = serde_json::from_slice::<serde_json::Value>(manifest_bytes)?;
    let mut usb_manifest = serde_json::from_slice::<serde_json::Value>(usb_manifest_bytes)?;
    for (label, value) in [
        ("manifest.json", &manifest),
        ("usb-manifest.json", &usb_manifest),
    ] {
        let capability = value
            .get("updateCapability")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                LicenseError::InvalidChecksumFile(format!("schema 3 缺少 {label} updateCapability"))
            })?;
        let schema = capability
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64);
        let anchor_schema = capability
            .get("anchorSchema")
            .and_then(serde_json::Value::as_u64);
        if schema != Some(ANCHOR_SCHEMA_V3 as u64) || anchor_schema != Some(ANCHOR_SCHEMA_V3 as u64)
        {
            return Err(LicenseError::InvalidChecksumFile(format!(
                "{label} 不是 schema 3 更新能力声明"
            )));
        }
    }
    let payload_relative_path = manifest
        .pointer("/payload/path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            LicenseError::InvalidChecksumFile(
                "manifest.json 缺少 payload.path，无法确定动态 payload 行".to_string(),
            )
        })?;
    validate_bundle_relative_path(payload_relative_path)?;
    let payload_relative_path = payload_relative_path.trim().replace('\\', "/");
    let payload_relative_path = payload_relative_path.to_ascii_lowercase();

    remove_v2_mutable_fields(&mut manifest, "manifest.json");
    remove_v2_mutable_fields(&mut usb_manifest, "usb-manifest.json");
    let canonical_manifest = canonical_json_bytes(&manifest)?;
    let canonical_usb_manifest = canonical_json_bytes(&usb_manifest)?;
    update_anchor_part(&mut hasher, "manifest.json", &canonical_manifest);
    update_anchor_part(&mut hasher, "usb-manifest.json", &canonical_usb_manifest);

    let canonical_sums = canonical_checksum_rows_from_bytes(
        checksum_bytes,
        &payload_relative_path,
        ChecksumRowsMode::StableUpdateV1,
    )?;
    update_anchor_part(&mut hasher, "SHA256SUMS.txt", &canonical_sums);
    Ok(finish_anchor(hasher))
}

/// Read the schema 3 stable-update-v1 anchor from a bundle without reading
/// the potentially very large payload.
pub fn stable_anchor_sha256(bundle_root: &Path) -> Result<String> {
    let manifest_path = bundle_root.join("manifest.json");
    let usb_manifest_path = bundle_root.join("usb-manifest.json");
    let checksum_path = bundle_root.join("SHA256SUMS.txt");
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|source| io_error(&manifest_path, source))?;
    let usb_manifest_bytes =
        fs::read(&usb_manifest_path).map_err(|source| io_error(&usb_manifest_path, source))?;
    let checksum_bytes =
        fs::read(&checksum_path).map_err(|source| io_error(&checksum_path, source))?;
    stable_anchor_sha256_from_bytes(&manifest_bytes, &usb_manifest_bytes, &checksum_bytes)
}

/// Select the anchor representation from the explicit update capability.  The
/// top-level manifest schema remains a document-format version (currently 2),
/// so it must not be overloaded as a cryptographic algorithm selector.  A
/// missing or malformed legacy manifest remains v1-compatible; an explicit
/// unknown numeric schema is rejected rather than silently downgraded.
pub fn bundle_anchor_schema(bundle_root: &Path) -> Result<u32> {
    let path = bundle_root.join("usb-manifest.json");
    let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(ANCHOR_SCHEMA_V1),
    };
    if let Some(anchor_value) = value.pointer("/updateCapability/anchorSchema") {
        let anchor_schema = anchor_value
            .as_u64()
            .ok_or(LicenseError::InvalidAnchorSchemaType)?;
        return match anchor_schema {
            1 => Ok(ANCHOR_SCHEMA_V1),
            2 => Ok(ANCHOR_SCHEMA_V2),
            3 => Ok(ANCHOR_SCHEMA_V3),
            other => Err(LicenseError::UnsupportedAnchorSchema(other as u32)),
        };
    }
    let Some(schema_value) = value.get("schemaVersion") else {
        return Ok(ANCHOR_SCHEMA_V1);
    };
    let Some(schema) = schema_value.as_u64() else {
        return Err(LicenseError::InvalidAnchorSchemaType);
    };
    match schema {
        1 => Ok(ANCHOR_SCHEMA_V1),
        2 => Ok(ANCHOR_SCHEMA_V2),
        other => Err(LicenseError::UnsupportedAnchorSchema(other as u32)),
    }
}

/// Hash the bundle using the representation selected by its explicit
/// capability.
pub fn anchor_sha256(bundle_root: &Path) -> Result<String> {
    match bundle_anchor_schema(bundle_root)? {
        ANCHOR_SCHEMA_V1 => raw_anchor_sha256(bundle_root),
        ANCHOR_SCHEMA_V2 => canonical_anchor_sha256(bundle_root),
        ANCHOR_SCHEMA_V3 => stable_anchor_sha256(bundle_root),
        other => Err(LicenseError::UnsupportedAnchorSchema(other)),
    }
}

/// Require the two stable license metadata fields without checking a key.
pub fn validate_formal_license_metadata(license: &UsbLicense) -> Result<()> {
    validate_license_metadata(license)?;
    let key_id = license
        .key_id
        .as_deref()
        .ok_or(LicenseError::MissingKeyId)?;
    validate_key_id(key_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;
    use serde_json::json;

    fn sample(anchor_schema: u32) -> UsbLicense {
        UsbLicense {
            schema_version: LICENSE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            installation_id: "00000000-0000-0000-0000-000000000001".to_string(),
            fingerprint: UsbFingerprint {
                volume_serial: 0x1234_5678,
                anchor_sha256: "a".repeat(64),
                bundle_relative_path: "codex-usb".to_string(),
                anchor_schema,
            },
            issued_at: "1770000000000".to_string(),
            key_id: Some("prod-2026-01".to_string()),
            account_binding: None,
            signature: None,
        }
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn stable_fixture() -> (serde_json::Value, serde_json::Value, String) {
        let manifest = json!({
            "schemaVersion": 2,
            "codexDesktopVersion": "old",
            "packageVersion": "old",
            "releaseTag": "old",
            "payload": {
                "path": "payload/current.msix",
                "sha256": "old",
                "size": 10
            },
            "updateCapability": {
                "schemaVersion": 3,
                "anchorSchema": 3,
                "backupDirectory": "payload/_backup"
            },
            "stable": {"value": true}
        });
        let usb_manifest = json!({
            "schemaVersion": 2,
            "codexInstaller": {
                "payloadSha256": "old",
                "payloadSize": 10,
                "file": "CodexDesktopSetup.exe"
            },
            "updateCapability": {
                "schemaVersion": 3,
                "anchorSchema": 3
            },
            "stable": [1, 2, 3]
        });
        let sums = format!(
            "{}  payload/current.msix\n{}  manifest.json\n{}  usb-manifest.json\n{}  payload/_backup/previous.msix\n{}  content/unknown.bin\n{}  payload/other.msix\n{}  other/_backup/previous.msix\n{}  payload/_backupish/previous.msix\n",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
            "0".repeat(64),
            "1".repeat(64),
            "2".repeat(64),
        );
        (manifest, usb_manifest, sums)
    }

    fn stable_anchor_for(
        manifest: &serde_json::Value,
        usb_manifest: &serde_json::Value,
        sums: &str,
    ) -> String {
        stable_anchor_sha256_from_bytes(
            &serde_json::to_vec(manifest).unwrap(),
            &serde_json::to_vec(usb_manifest).unwrap(),
            sums.as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn current_payload_is_compact_ordered_and_excludes_signature() {
        let mut license = sample(ANCHOR_SCHEMA_V2);
        let payload = unsigned_license_payload(&license).unwrap();
        let text = String::from_utf8(payload).unwrap();
        assert_eq!(
            text,
            r#"{"schemaVersion":1,"product":"codex-usb-oneclick","installationId":"00000000-0000-0000-0000-000000000001","fingerprint":{"volumeSerial":305419896,"anchorSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bundleRelativePath":"codex-usb","anchorSchema":2},"issuedAt":"1770000000000","keyId":"prod-2026-01"}"#
        );
        license.signature = Some("should-not-be-in-payload".to_string());
        assert_eq!(unsigned_license_payload(&license).unwrap(), text.as_bytes());
    }

    #[test]
    fn signing_and_verification_are_deterministic() {
        let signing_key = key();
        let public_key = BASE64.encode(signing_key.verifying_key().to_bytes());
        let mut license = sample(ANCHOR_SCHEMA_V2);
        sign_license(&mut license, &signing_key).unwrap();
        assert_eq!(license.signature.as_ref().unwrap().len(), 88);
        verify_signature_with_key(&license, &public_key).unwrap();
        license.key_id = Some("other-key".to_string());
        assert!(matches!(
            verify_signature_with_key(&license, &public_key),
            Err(LicenseError::InvalidSignature)
        ));
    }

    #[test]
    fn legacy_payload_is_accepted_only_for_v1_anchor() {
        let signing_key = key();
        let public_key = BASE64.encode(signing_key.verifying_key().to_bytes());
        let mut license = sample(ANCHOR_SCHEMA_V1);
        let signature = signing_key.sign(&legacy_unsigned_license_payload(&license).unwrap());
        license.signature = Some(BASE64.encode(signature.to_bytes()));
        verify_signature_with_key(&license, &public_key).unwrap();
        license.fingerprint.anchor_schema = ANCHOR_SCHEMA_V2;
        assert!(matches!(
            verify_signature_with_key(&license, &public_key),
            Err(LicenseError::InvalidSignature)
        ));
    }

    #[test]
    fn new_signatures_require_v2_and_valid_license_metadata() {
        let signing_key = key();
        let mut legacy = sample(ANCHOR_SCHEMA_V1);
        assert!(matches!(
            sign_license(&mut legacy, &signing_key),
            Err(LicenseError::NewSignatureRequiresV2)
        ));

        let mut bad_installation = sample(ANCHOR_SCHEMA_V2);
        bad_installation.installation_id = "not-a-uuid".to_string();
        assert!(matches!(
            sign_license(&mut bad_installation, &signing_key),
            Err(LicenseError::InvalidInstallationId(_))
        ));

        let mut bad_timestamp = sample(ANCHOR_SCHEMA_V2);
        bad_timestamp.issued_at = "now".to_string();
        assert!(matches!(
            sign_license(&mut bad_timestamp, &signing_key),
            Err(LicenseError::InvalidIssuedAt(_))
        ));
    }

    #[test]
    fn schema3_license_is_supported_and_signable() {
        assert!(is_supported_anchor_schema(ANCHOR_SCHEMA_V3));
        let signing_key = key();
        let public_key = BASE64.encode(signing_key.verifying_key().to_bytes());
        let mut license = sample(ANCHOR_SCHEMA_V3);
        sign_license(&mut license, &signing_key).unwrap();
        verify_signature_with_key(&license, &public_key).unwrap();
    }

    #[test]
    fn account_binding_is_signed_and_mutation_is_rejected() {
        let signing_key = key();
        let public_key = BASE64.encode(signing_key.verifying_key().to_bytes());
        let mut license = sample(ANCHOR_SCHEMA_V2);
        license.account_binding = Some(UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "USB-0001".to_string(),
            account_ref: "USB-0001".to_string(),
            account_name: "USB-0001".to_string(),
            login_email: "u1@u.404token.xyz".to_string(),
            sub2api_user_id: 23,
            ccswitch: UsbCcswitchBinding {
                provider_id: "404token-sub2api".to_string(),
                credential_id: "sub2api-user-key-23".to_string(),
                device_api_key: "sk-test-device-key".to_string(),
                group_id: 7,
            },
        });
        sign_license(&mut license, &signing_key).unwrap();
        verify_signature_with_key(&license, &public_key).unwrap();
        license.account_binding.as_mut().unwrap().ccswitch.group_id = 8;
        assert!(matches!(
            verify_signature_with_key(&license, &public_key),
            Err(LicenseError::InvalidSignature)
        ));
    }

    #[test]
    fn legacy_unsigned_payload_stays_byte_compatible_without_account_binding() {
        let license = sample(ANCHOR_SCHEMA_V2);
        let text = String::from_utf8(unsigned_license_payload(&license).unwrap()).unwrap();
        assert!(!text.contains("accountBinding"));
        assert!(text.ends_with("\"keyId\":\"prod-2026-01\"}"));
    }

    #[test]
    fn missing_anchor_schema_defaults_to_v1_and_unknown_fields_are_rejected() {
        let value = json!({
            "schemaVersion": 1,
            "product": PRODUCT_ID,
            "installationId": "00000000-0000-0000-0000-000000000001",
            "fingerprint": {
                "volumeSerial": 1,
                "anchorSha256": "a".repeat(64),
                "bundleRelativePath": "codex-usb"
            },
            "issuedAt": "1",
            "unknown": true
        });
        assert!(serde_json::from_value::<UsbLicense>(value).is_err());
        let mut value = json!({
            "schemaVersion": 1,
            "product": PRODUCT_ID,
            "installationId": "00000000-0000-0000-0000-000000000001",
            "fingerprint": {
                "volumeSerial": 1,
                "anchorSha256": "a".repeat(64),
                "bundleRelativePath": "codex-usb"
            },
            "issuedAt": "1"
        });
        let parsed = serde_json::from_value::<UsbLicense>(value.clone()).unwrap();
        assert_eq!(parsed.fingerprint.anchor_schema, ANCHOR_SCHEMA_V1);
        value["fingerprint"]["unknown"] = json!(true);
        assert!(serde_json::from_value::<UsbLicense>(value).is_err());
    }

    #[test]
    fn v2_anchor_ignores_only_allowlisted_mutations() {
        let directory = std::env::temp_dir().join(format!("codex-usb-license-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let manifest = json!({
            "schemaVersion": 2,
            "codexDesktopVersion": "old",
            "packageVersion": "old",
            "releaseTag": "old",
            "payload": {"path": "payload/a.msix", "sha256": "old", "size": 1},
            "stable": {"value": true}
        });
        let usb_manifest = json!({
            "schemaVersion": 2,
            "codexInstaller": {"payloadSha256": "old", "payloadSize": 1, "file": "CodexDesktopSetup.exe"},
            "stable": [1, 2, 3]
        });
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("usb-manifest.json"),
            serde_json::to_vec(&usb_manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("SHA256SUMS.txt"),
            format!(
                "{}  payload/a.msix\n{}  content/file.txt\n",
                "a".repeat(64),
                "b".repeat(64)
            ),
        )
        .unwrap();
        let first = canonical_anchor_sha256(&directory).unwrap();
        let mut reordered = manifest.clone();
        reordered["codexDesktopVersion"] = json!("new");
        reordered["payload"]["sha256"] = json!("new");
        reordered["payload"]["size"] = json!(2);
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&reordered).unwrap(),
        )
        .unwrap();
        let mut usb_reordered = usb_manifest.clone();
        usb_reordered["codexInstaller"]["payloadSha256"] = json!("new");
        usb_reordered["codexInstaller"]["payloadSize"] = json!(2);
        fs::write(
            directory.join("usb-manifest.json"),
            serde_json::to_vec(&usb_reordered).unwrap(),
        )
        .unwrap();
        assert_eq!(first, canonical_anchor_sha256(&directory).unwrap());
        fs::write(
            directory.join("SHA256SUMS.txt"),
            format!(
                "{}  payload/a.msix\n{}  content/file.txt\n",
                "a".repeat(64),
                "c".repeat(64)
            ),
        )
        .unwrap();
        assert_ne!(first, canonical_anchor_sha256(&directory).unwrap());
        reordered["stable"] = json!({"value": false});
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&reordered).unwrap(),
        )
        .unwrap();
        assert_ne!(first, canonical_anchor_sha256(&directory).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn schema3_stable_anchor_excludes_only_authorized_update_rows() {
        let (manifest, usb_manifest, sums) = stable_fixture();
        let first = stable_anchor_for(&manifest, &usb_manifest, &sums);

        let mut dynamic_manifest = manifest.clone();
        dynamic_manifest["codexDesktopVersion"] = json!("new");
        dynamic_manifest["packageVersion"] = json!("new");
        dynamic_manifest["releaseTag"] = json!("new");
        dynamic_manifest["payload"]["sha256"] = json!("new");
        dynamic_manifest["payload"]["size"] = json!(20);
        let mut dynamic_usb_manifest = usb_manifest.clone();
        dynamic_usb_manifest["codexInstaller"]["payloadSha256"] = json!("new");
        dynamic_usb_manifest["codexInstaller"]["payloadSize"] = json!(20);
        assert_eq!(
            first,
            stable_anchor_for(&dynamic_manifest, &dynamic_usb_manifest, &sums)
        );

        let ignored_rows = sums
            .replace(&"a".repeat(64), &"1".repeat(64))
            .replace(&"b".repeat(64), &"2".repeat(64))
            .replace(&"c".repeat(64), &"3".repeat(64))
            .replace(&"d".repeat(64), &"4".repeat(64));
        assert_eq!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &ignored_rows)
        );
        let reordered_rows = sums.lines().rev().collect::<Vec<_>>().join("\n") + "\n";
        assert_eq!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &reordered_rows)
        );
        let normalized_rows = sums.replace("content/unknown.bin", r"CONTENT\UNKNOWN.BIN");
        assert_eq!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &normalized_rows)
        );

        let without_backup = sums
            .lines()
            .filter(|line| !line.ends_with("payload/_backup/previous.msix"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &without_backup)
        );

        let changed_unknown = sums.replace(&"e".repeat(64), &"1".repeat(64));
        assert_ne!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &changed_unknown)
        );
        let changed_other_msix = sums.replace(&"0".repeat(64), &"8".repeat(64));
        assert_ne!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &changed_other_msix)
        );
        let changed_external_backup = sums.replace(&"1".repeat(64), &"9".repeat(64));
        assert_ne!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &changed_external_backup)
        );
        let changed_backup_prefix = sums.replace(&"2".repeat(64), &"8".repeat(64));
        assert_ne!(
            first,
            stable_anchor_for(&manifest, &usb_manifest, &changed_backup_prefix)
        );

        let mut stable_manifest = manifest.clone();
        stable_manifest["stable"]["value"] = json!(false);
        assert_ne!(
            first,
            stable_anchor_for(&stable_manifest, &usb_manifest, &sums)
        );
        let mut stable_usb_manifest = usb_manifest.clone();
        stable_usb_manifest["stable"] = json!([3, 2, 1]);
        assert_ne!(
            first,
            stable_anchor_for(&manifest, &stable_usb_manifest, &sums)
        );

        let mut expanded_backup_metadata = manifest.clone();
        expanded_backup_metadata["updateCapability"]["backupDirectory"] = json!("other/_backup");
        assert_ne!(
            first,
            stable_anchor_for(&expanded_backup_metadata, &usb_manifest, &sums)
        );
    }

    #[test]
    fn schema3_anchor_dispatch_and_byte_api_do_not_read_payload() {
        let directory =
            std::env::temp_dir().join(format!("codex-usb-license-v3-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let (manifest, usb_manifest, sums) = stable_fixture();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let usb_manifest_bytes = serde_json::to_vec(&usb_manifest).unwrap();
        fs::write(directory.join("manifest.json"), &manifest_bytes).unwrap();
        fs::write(directory.join("usb-manifest.json"), &usb_manifest_bytes).unwrap();
        fs::write(directory.join("SHA256SUMS.txt"), sums.as_bytes()).unwrap();

        let expected =
            stable_anchor_sha256_from_bytes(&manifest_bytes, &usb_manifest_bytes, sums.as_bytes())
                .unwrap();
        assert_eq!(bundle_anchor_schema(&directory).unwrap(), ANCHOR_SCHEMA_V3);
        assert_eq!(stable_anchor_sha256(&directory).unwrap(), expected);
        assert_eq!(anchor_sha256(&directory).unwrap(), expected);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn schema3_checksum_paths_are_fail_closed() {
        let (manifest, usb_manifest, _) = stable_fixture();
        for invalid_path in [
            "payload/../outside.msix",
            r"payload\..\outside.msix",
            "payload/_backup/../outside.msix",
            "payload/_backup/evil.bin",
            "payload/_backup/nested/previous.msix",
            "payload//outside.msix",
            "payload/./outside.msix",
            "/outside.msix",
            "C:/outside.msix",
        ] {
            let sums = format!(
                "{}  payload/current.msix\n{}  {invalid_path}\n",
                "a".repeat(64),
                "b".repeat(64),
            );
            assert!(
                stable_anchor_sha256_from_bytes(
                    &serde_json::to_vec(&manifest).unwrap(),
                    &serde_json::to_vec(&usb_manifest).unwrap(),
                    sums.as_bytes(),
                )
                .is_err(),
                "invalid path was accepted: {invalid_path}"
            );
        }

        let duplicate = format!(
            "{}  payload/current.msix\n{}  PAYLOAD\\CURRENT.MSIX\n",
            "a".repeat(64),
            "b".repeat(64),
        );
        assert!(stable_anchor_sha256_from_bytes(
            &serde_json::to_vec(&manifest).unwrap(),
            &serde_json::to_vec(&usb_manifest).unwrap(),
            duplicate.as_bytes(),
        )
        .is_err());

        for forbidden_path in [
            "usb-license.json",
            "nested/usb-license.json",
            "SHA256SUMS.txt",
            "nested/SHA256SUMS.txt",
            "payload/file.bak",
        ] {
            let sums = format!(
                "{}  payload/current.msix\n{}  {forbidden_path}\n",
                "a".repeat(64),
                "b".repeat(64),
            );
            assert!(
                stable_anchor_sha256_from_bytes(
                    &serde_json::to_vec(&manifest).unwrap(),
                    &serde_json::to_vec(&usb_manifest).unwrap(),
                    sums.as_bytes(),
                )
                .is_err(),
                "forbidden path was accepted: {forbidden_path}"
            );
        }

        let invalid_utf8 = [
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  payload/current.msix\n".as_slice(),
            &[0xff],
        ]
        .concat();
        assert!(stable_anchor_sha256_from_bytes(
            &serde_json::to_vec(&manifest).unwrap(),
            &serde_json::to_vec(&usb_manifest).unwrap(),
            &invalid_utf8,
        )
        .is_err());
    }

    #[test]
    fn legacy_anchor_vectors_remain_fixed() {
        let directory =
            std::env::temp_dir().join(format!("codex-usb-license-vectors-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let manifest = br#"{"payload":{"path":"payload/current.msix","sha256":"old","size":10},"stable":true}"#;
        let usb_manifest =
            br#"{"codexInstaller":{"payloadSha256":"old","payloadSize":10},"stable":[1,2,3]}"#;
        let sums = format!(
            "{}  payload/current.msix\n{}  content/file.txt\n",
            "a".repeat(64),
            "b".repeat(64),
        );
        fs::write(directory.join("manifest.json"), manifest).unwrap();
        fs::write(directory.join("usb-manifest.json"), usb_manifest).unwrap();
        fs::write(directory.join("SHA256SUMS.txt"), sums).unwrap();

        assert_eq!(
            raw_anchor_sha256(&directory).unwrap(),
            "d4efdc902ab03c4a4732059ba5ad63c8be07f695723956a3d59df77a763d83c2"
        );
        assert_eq!(
            canonical_anchor_sha256(&directory).unwrap(),
            "551b28bd31d3f56fe2fce9b6cc23bb87ddb0353b560687beb862a909d8dd4ccc"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_non_integer_anchor_schema_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("codex-usb-license-anchor-type-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("usb-manifest.json"),
            br#"{"schemaVersion":"2"}"#,
        )
        .unwrap();
        assert!(matches!(
            bundle_anchor_schema(&directory),
            Err(LicenseError::InvalidAnchorSchemaType)
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
