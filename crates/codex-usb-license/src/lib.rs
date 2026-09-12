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
    pub signature: Option<String>,
}

/// Return whether an anchor schema is understood by this version of the
/// adapter core.
pub fn is_supported_anchor_schema(anchor_schema: u32) -> bool {
    matches!(anchor_schema, ANCHOR_SCHEMA_V1 | ANCHOR_SCHEMA_V2)
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
    Ok(())
}

/// Sign a license in place.  The caller must set `key_id` before calling this
/// function; requiring it here makes it impossible to accidentally sign a
/// formal license whose key identifier was omitted from the payload.
pub fn sign_license(license: &mut UsbLicense, signing_key: &SigningKey) -> Result<()> {
    validate_license_metadata(license)?;
    if license.fingerprint.anchor_schema != ANCHOR_SCHEMA_V2 {
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
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| LicenseError::InvalidChecksumFile("文件必须是 UTF-8 文本".to_string()))?;
    let payload_relative_path = payload_relative_path.trim().replace('\\', "/");
    validate_bundle_relative_path(&payload_relative_path)?;
    let payload_relative_path = payload_relative_path.to_ascii_lowercase();
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let mut saw_payload = false;

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
            continue;
        }
        rows.push((relative_path, hash));
    }

    if !saw_payload {
        return Err(LicenseError::InvalidChecksumFile(format!(
            "缺少当前 payload 行：{payload_relative_path}"
        )));
    }
    rows.sort_unstable();
    let mut canonical = Vec::new();
    for (relative_path, hash) in rows {
        canonical.extend_from_slice(hash.as_bytes());
        canonical.extend_from_slice(b"  ");
        canonical.extend_from_slice(relative_path.as_bytes());
        canonical.push(b'\n');
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

/// Select the anchor representation from usb-manifest.schemaVersion.  A
/// missing or malformed legacy manifest remains v1-compatible; an explicit
/// unknown numeric schema is rejected rather than silently downgraded.
pub fn bundle_anchor_schema(bundle_root: &Path) -> Result<u32> {
    let path = bundle_root.join("usb-manifest.json");
    let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(ANCHOR_SCHEMA_V1),
    };
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
            signature: None,
        }
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
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
