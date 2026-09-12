use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use codex_usb_license::{
    fingerprints_match, new_unsigned_license, sign_license, validate_fingerprint, validate_key_id,
    validate_license, UsbFingerprint, UsbLicense,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::SignerError;
use crate::key_blob::ProtectedKey;
use crate::media::PhysicalObservation;
use crate::path_security::ensure_no_reparse_components;
use crate::windows::atomic_replace;

const LICENSE_FILE_NAME: &str = "usb-license.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenseState {
    Missing,
    Unsigned,
    Signed,
    InvalidJson,
    Invalid,
    FingerprintMismatch,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInspection {
    pub state: LicenseState,
    pub installation_id: Option<String>,
    pub key_id: Option<String>,
    pub license_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExistingLicense {
    pub bytes: Vec<u8>,
    pub license: Option<UsbLicense>,
}

pub fn license_path(bundle_root: &Path) -> PathBuf {
    bundle_root.join(LICENSE_FILE_NAME)
}

pub fn inspect_license(
    bundle_root: &Path,
    expected_fingerprint: Option<&UsbFingerprint>,
) -> Result<LicenseInspection, SignerError> {
    let path = license_path(bundle_root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LicenseInspection {
                state: LicenseState::Missing,
                installation_id: None,
                key_id: None,
                license_digest: None,
            })
        }
        Err(error) => return Err(SignerError::io("reading license", &path, error)),
    };
    let digest = digest_bytes(&bytes);
    let parsed = match serde_json::from_slice::<UsbLicense>(&bytes) {
        Ok(license) => license,
        Err(_) => {
            return Ok(LicenseInspection {
                state: LicenseState::InvalidJson,
                installation_id: None,
                key_id: None,
                license_digest: Some(digest),
            })
        }
    };
    let state = if let Some(expected) = expected_fingerprint {
        match validate_license(&parsed, expected, None) {
            Ok(_) if parsed.signature.is_some() => LicenseState::Signed,
            Ok(_) => LicenseState::Unsigned,
            Err(codex_usb_license::LicenseError::FingerprintMismatch) => {
                LicenseState::FingerprintMismatch
            }
            Err(_) => LicenseState::Invalid,
        }
    } else if parsed.signature.is_some() {
        LicenseState::Signed
    } else {
        LicenseState::Unsigned
    };
    Ok(LicenseInspection {
        state,
        installation_id: Some(parsed.installation_id),
        key_id: parsed.key_id,
        license_digest: Some(digest),
    })
}

pub fn load_existing_license(bundle_root: &Path) -> Result<ExistingLicense, SignerError> {
    let path = license_path(bundle_root);
    ensure_no_reparse_components(&path)?;
    match fs::read(&path) {
        Ok(bytes) => {
            let license = serde_json::from_slice::<UsbLicense>(&bytes).map_err(|error| {
                SignerError::ExistingLicense(format!("existing license JSON is invalid: {error}"))
            })?;
            Ok(ExistingLicense {
                bytes,
                license: Some(license),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ExistingLicense {
            bytes: Vec::new(),
            license: None,
        }),
        Err(error) => Err(SignerError::io("reading existing license", &path, error)),
    }
}

pub fn build_signing_candidate(
    bundle_root: &Path,
    observed: &PhysicalObservation,
    key_id: &str,
    issued_at_ms: u64,
) -> Result<(UsbLicense, ExistingLicense), SignerError> {
    validate_key_id(key_id)?;
    let existing = load_existing_license(bundle_root)?;
    let mut license = match existing.license.as_ref() {
        None => new_unsigned_license(observed.fingerprint.clone(), issued_at_ms),
        Some(existing) => {
            if existing.signature.is_some() {
                return Err(SignerError::ExistingLicense(
                    "an existing signed license is never overwritten".to_string(),
                ));
            }
            validate_fingerprint(&existing.fingerprint)?;
            if !fingerprints_match(&existing.fingerprint, &observed.fingerprint) {
                return Err(SignerError::ExistingLicense(
                    "existing unsigned license fingerprint does not match the observed bundle"
                        .to_string(),
                ));
            }
            Uuid::parse_str(&existing.installation_id).map_err(|_| {
                SignerError::ExistingLicense(
                    "existing unsigned license installationId is not a UUID".to_string(),
                )
            })?;
            existing.clone()
        }
    };
    license.fingerprint = observed.fingerprint.clone();
    license.issued_at = issued_at_ms.to_string();
    license.key_id = Some(key_id.to_string());
    license.signature = None;
    Ok((license, existing))
}

pub fn sign_candidate(license: &mut UsbLicense, key: &ProtectedKey) -> Result<(), SignerError> {
    sign_license(license, &key.signing_key)?;
    Ok(())
}

pub fn verify_signed_license(
    license: &UsbLicense,
    observed: &PhysicalObservation,
    public_key_b64: &str,
) -> Result<(), SignerError> {
    validate_license(license, &observed.fingerprint, Some(public_key_b64))?;
    Ok(())
}

pub fn write_old_license_audit(
    old_bytes: &[u8],
    audit_directory: &Path,
) -> Result<Option<PathBuf>, SignerError> {
    if old_bytes.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(audit_directory).map_err(|error| {
        SignerError::io(
            "creating local license audit directory",
            audit_directory,
            error,
        )
    })?;
    ensure_no_reparse_components(audit_directory)?;
    let path = audit_directory.join(format!("license-before-{}.json", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| SignerError::io("recording old license audit bytes", &path, error))?;
    file.write_all(old_bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| SignerError::io("recording old license audit bytes", &path, error))?;
    Ok(Some(path))
}

pub fn write_license_atomic(bundle_root: &Path, license: &UsbLicense) -> Result<(), SignerError> {
    let destination = license_path(bundle_root);
    ensure_no_reparse_components(&destination)?;
    let bytes = serde_json::to_vec_pretty(license).map_err(|error| {
        SignerError::Report(format!("serializing signed license failed: {error}"))
    })?;
    let temp = bundle_root.join(format!("usb-license.json.signing-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| {
                SignerError::io("creating same-volume license temporary file", &temp, error)
            })?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                SignerError::io("flushing same-volume license temporary file", &temp, error)
            })?;
        atomic_replace(&temp, &destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn read_back_license(bundle_root: &Path) -> Result<UsbLicense, SignerError> {
    let path = license_path(bundle_root);
    let bytes = fs::read(&path)
        .map_err(|error| SignerError::io("reading signed license back", &path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        SignerError::OutcomeUnknown(format!("license readback JSON is invalid: {error}"))
    })
}

pub fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_usb_license::{UsbFingerprint, ANCHOR_SCHEMA_V2, LICENSE_SCHEMA_VERSION, PRODUCT_ID};

    fn observation() -> PhysicalObservation {
        PhysicalObservation {
            volume: crate::windows::VolumeInfo {
                drive_root: "E:\\".to_string(),
                volume_serial: 1,
                file_system: "NTFS".to_string(),
            },
            bundle_path: PathBuf::from("E:\\CodexUsb"),
            fingerprint: UsbFingerprint {
                volume_serial: 1,
                anchor_sha256: "a".repeat(64),
                bundle_relative_path: "codexusb".to_string(),
                anchor_schema: ANCHOR_SCHEMA_V2,
            },
        }
    }

    #[test]
    fn old_unsigned_license_keeps_installation_id() {
        let root = std::env::temp_dir().join(format!("usb-license-tx-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let old = UsbLicense {
            schema_version: LICENSE_SCHEMA_VERSION,
            product: PRODUCT_ID.to_string(),
            installation_id: "00000000-0000-0000-0000-000000000001".to_string(),
            fingerprint: observation().fingerprint.clone(),
            issued_at: "1".to_string(),
            key_id: None,
            signature: None,
        };
        fs::write(license_path(&root), serde_json::to_vec(&old).unwrap()).unwrap();
        let (candidate, _) = build_signing_candidate(&root, &observation(), "test-key", 2).unwrap();
        assert_eq!(candidate.installation_id, old.installation_id);
        assert_eq!(candidate.issued_at, "2");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn internal_license_sign_and_readback_verify_without_authenticode() {
        use ed25519_dalek::SigningKey;

        let root = std::env::temp_dir().join(format!("usb-license-internal-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let observed = observation();
        let key = ProtectedKey {
            key_id: "test-key".to_string(),
            signing_key: SigningKey::from_bytes(&[7u8; 32]),
        };
        let (mut candidate, old) =
            build_signing_candidate(&root, &observed, &key.key_id, 2).unwrap();
        assert!(old.bytes.is_empty());
        sign_candidate(&mut candidate, &key).unwrap();
        verify_signed_license(&candidate, &observed, &key.public_key_b64()).unwrap();
        write_license_atomic(&root, &candidate).unwrap();
        let readback = read_back_license(&root).unwrap();
        verify_signed_license(&readback, &observed, &key.public_key_b64()).unwrap();
        assert!(readback.signature.is_some());
        fs::remove_dir_all(root).unwrap();
    }
}
