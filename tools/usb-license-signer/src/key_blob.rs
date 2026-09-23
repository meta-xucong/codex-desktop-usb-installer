use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use codex_usb_license::validate_key_id;
use ed25519_dalek::SigningKey;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::error::SignerError;
use crate::path_security::ensure_no_reparse_components;
use crate::windows::{atomic_replace, dpapi_protect, dpapi_unprotect, ensure_fixed_local_path};

const KEY_BLOB_MAGIC: &[u8] = b"CODEX-USB-SIGNER-DPAPI\0";
const KEY_BLOB_VERSION: u8 = 1;
const PORTABLE_VAULT_MAGIC: &[u8] = b"CODEX-USB-SIGNER-VAULT\0";
const PORTABLE_VAULT_VERSION: u8 = 1;
const PORTABLE_VAULT_SALT_LEN: usize = 16;
const PORTABLE_VAULT_NONCE_LEN: usize = 12;
const PORTABLE_VAULT_SECRET_LEN: usize = 32;
const PORTABLE_VAULT_TAG_LEN: usize = 16;
const PORTABLE_PASSWORD_MIN_BYTES: usize = 12;
const PORTABLE_PASSWORD_MAX_BYTES: usize = 4096;
const PORTABLE_ARGON_MEMORY_KIB: u32 = 64 * 1024;
const PORTABLE_ARGON_ITERATIONS: u32 = 3;
pub const PORTABLE_KEY_VAULT_FILE_NAME: &str = "codex-usb-signing-key.vault";
pub const PORTABLE_KEY_VAULT_DESCRIPTOR_SUFFIX: &str = ".public.json";
pub const DEFAULT_KEY_ID: &str = "prod-2026-01";

pub struct ProtectedKey {
    pub key_id: String,
    pub signing_key: SigningKey,
}

impl fmt::Debug for ProtectedKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl ProtectedKey {
    pub fn public_key_b64(&self) -> String {
        BASE64.encode(self.signing_key.verifying_key().to_bytes())
    }

    /// Digest the raw 32-byte Ed25519 public key, not its Base64 text.  The
    /// digest is the public binding written into a formal USB manifest.
    pub fn public_key_sha256(&self) -> String {
        let digest = Sha256::digest(self.signing_key.verifying_key().to_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[derive(Debug)]
pub struct AutoResolvedKey {
    pub path: PathBuf,
    pub key: ProtectedKey,
    pub generated: bool,
}

/// Return the portable encrypted vault placed beside the signer executable.
/// The path is deliberately not derived from LOCALAPPDATA or DPAPI, so a
/// complete signer folder can be copied to another Windows machine.
pub fn default_portable_vault_path() -> Result<PathBuf, SignerError> {
    let executable = std::env::current_exe().map_err(|error| {
        SignerError::KeyBlob(format!(
            "cannot resolve signer executable location: {error}"
        ))
    })?;
    let directory = executable.parent().ok_or_else(|| {
        SignerError::KeyBlob("signer executable has no parent directory".to_string())
    })?;
    let path = directory.join(PORTABLE_KEY_VAULT_FILE_NAME);
    ensure_no_reparse_components(directory)?;
    Ok(path)
}

pub fn portable_password_is_valid(password: &str) -> Result<(), SignerError> {
    let length = password.len();
    if length < PORTABLE_PASSWORD_MIN_BYTES {
        return Err(SignerError::KeyBlob(format!(
            "portable signing password must contain at least {PORTABLE_PASSWORD_MIN_BYTES} bytes"
        )));
    }
    if length > PORTABLE_PASSWORD_MAX_BYTES {
        return Err(SignerError::KeyBlob(format!(
            "portable signing password must contain at most {PORTABLE_PASSWORD_MAX_BYTES} bytes"
        )));
    }
    Ok(())
}

fn portable_derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, SignerError> {
    portable_password_is_valid(password)?;
    if salt.len() != PORTABLE_VAULT_SALT_LEN {
        return Err(SignerError::KeyBlob(
            "portable key vault salt has an invalid length".to_string(),
        ));
    }
    let params = Params::new(
        PORTABLE_ARGON_MEMORY_KIB,
        PORTABLE_ARGON_ITERATIONS,
        1,
        Some(32),
    )
    .map_err(|error| {
        SignerError::KeyBlob(format!("portable KDF parameters are invalid: {error}"))
    })?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut derived = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), salt, derived.as_mut())
        .map_err(|error| {
            SignerError::KeyBlob(format!("portable key derivation failed: {error}"))
        })?;
    Ok(derived)
}

fn portable_header(key_id: &str, salt: &[u8], nonce: &[u8]) -> Result<Vec<u8>, SignerError> {
    validate_key_id(key_id)?;
    if salt.len() != PORTABLE_VAULT_SALT_LEN || nonce.len() != PORTABLE_VAULT_NONCE_LEN {
        return Err(SignerError::KeyBlob(
            "portable key vault salt or nonce has an invalid length".to_string(),
        ));
    }
    let key_id_bytes = key_id.as_bytes();
    let key_id_len = u16::try_from(key_id_bytes.len())
        .map_err(|_| SignerError::KeyBlob("portable key vault keyId is too long".to_string()))?;
    let mut header = Vec::with_capacity(
        PORTABLE_VAULT_MAGIC.len() + 1 + 2 + key_id_bytes.len() + salt.len() + nonce.len(),
    );
    header.extend_from_slice(PORTABLE_VAULT_MAGIC);
    header.push(PORTABLE_VAULT_VERSION);
    header.extend_from_slice(&key_id_len.to_le_bytes());
    header.extend_from_slice(key_id_bytes);
    header.extend_from_slice(salt);
    header.extend_from_slice(nonce);
    Ok(header)
}

/// Write an encrypted, machine-independent copy of a protected signing key.
/// The file is create-only: an existing vault is never replaced implicitly.
pub fn write_portable_key_vault(
    path: &Path,
    key: &ProtectedKey,
    password: &str,
) -> Result<(), SignerError> {
    portable_password_is_valid(password)?;
    ensure_no_reparse_components(path.parent().ok_or_else(|| {
        SignerError::KeyBlob(format!(
            "portable vault has no parent directory: {}",
            path.display()
        ))
    })?)?;
    if path.exists() {
        return Err(SignerError::KeyBlob(format!(
            "portable key vault already exists: {}",
            path.display()
        )));
    }

    let mut salt = [0u8; PORTABLE_VAULT_SALT_LEN];
    let mut nonce_bytes = [0u8; PORTABLE_VAULT_NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|error| {
        SignerError::KeyBlob(format!("portable vault salt generation failed: {error}"))
    })?;
    getrandom::fill(&mut nonce_bytes).map_err(|error| {
        SignerError::KeyBlob(format!("portable vault nonce generation failed: {error}"))
    })?;
    let header = portable_header(&key.key_id, &salt, &nonce_bytes)?;
    let derived = portable_derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).map_err(|_| {
        SignerError::KeyBlob("portable vault cipher initialization failed".to_string())
    })?;
    let secret = Zeroizing::new(key.signing_key.to_bytes());
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: secret.as_ref(),
                aad: &header,
            },
        )
        .map_err(|_| SignerError::KeyBlob("portable vault encryption failed".to_string()))?;

    let mut bytes = header;
    bytes.extend_from_slice(&encrypted);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| SignerError::io("creating portable key vault", path, error))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| SignerError::io("writing portable key vault", path, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

/// Return the public, non-secret descriptor written beside a portable vault.
/// It is deliberately separate from the encrypted vault so release tooling
/// can reject a mismatched vault without asking for (or logging) its password.
pub fn portable_key_vault_descriptor_path(path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}{}",
        path.display(),
        PORTABLE_KEY_VAULT_DESCRIPTOR_SUFFIX
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PortableKeyDescriptor<'a> {
    schema_version: u8,
    key_id: &'a str,
    public_key_b64: String,
    public_key_sha256: String,
    protected_by: &'static str,
}

/// Write the public binding descriptor for a portable vault.  The descriptor
/// contains no private material and is safe to ship as release metadata; the
/// signer still decrypts the vault and compares the actual key before signing.
pub fn write_portable_key_vault_descriptor(
    vault_path: &Path,
    key: &ProtectedKey,
) -> Result<PathBuf, SignerError> {
    ensure_no_reparse_components(vault_path)?;
    let vault_metadata = fs::symlink_metadata(vault_path).map_err(|error| {
        SignerError::io(
            "reading portable vault before writing its descriptor",
            vault_path,
            error,
        )
    })?;
    if !vault_metadata.is_file() {
        return Err(SignerError::KeyBlob(format!(
            "portable vault is not a regular file: {}",
            vault_path.display()
        )));
    }
    let descriptor_path = portable_key_vault_descriptor_path(vault_path);
    ensure_no_reparse_components(vault_path.parent().ok_or_else(|| {
        SignerError::KeyBlob(format!(
            "portable vault has no parent directory: {}",
            vault_path.display()
        ))
    })?)?;
    let descriptor = PortableKeyDescriptor {
        schema_version: 1,
        key_id: &key.key_id,
        public_key_b64: key.public_key_b64(),
        public_key_sha256: key.public_key_sha256(),
        protected_by: "portable AES-256-GCM vault; private seed is not in this file",
    };
    let bytes = serde_json::to_vec_pretty(&descriptor)
        .map_err(|error| SignerError::KeyBlob(format!("序列化 portable 公钥描述失败：{error}")))?;
    let temporary = descriptor_path.with_file_name(format!(
        ".{}.public-{}.tmp",
        descriptor_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("vault"),
        Uuid::new_v4()
    ));
    fs::write(&temporary, bytes).map_err(|error| {
        SignerError::io("writing portable public key descriptor", &temporary, error)
    })?;
    let result = atomic_replace(&temporary, &descriptor_path);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|_| descriptor_path)
}

/// Decrypt a portable key vault using a password supplied by the user.  The
/// password is never accepted from an environment variable or persisted by
/// this module.
pub fn read_portable_key_vault(path: &Path, password: &str) -> Result<ProtectedKey, SignerError> {
    portable_password_is_valid(password)?;
    ensure_no_reparse_components(path)?;
    let bytes = fs::read(path)
        .map_err(|error| SignerError::io("reading portable key vault", path, error))?;
    let minimum = PORTABLE_VAULT_MAGIC.len()
        + 1
        + 2
        + PORTABLE_VAULT_SALT_LEN
        + PORTABLE_VAULT_NONCE_LEN
        + PORTABLE_VAULT_SECRET_LEN
        + PORTABLE_VAULT_TAG_LEN;
    if bytes.len() < minimum || !bytes.starts_with(PORTABLE_VAULT_MAGIC) {
        return Err(SignerError::KeyBlob(
            "portable key vault has an unknown header or truncated content".to_string(),
        ));
    }
    let mut offset = PORTABLE_VAULT_MAGIC.len();
    if bytes[offset] != PORTABLE_VAULT_VERSION {
        return Err(SignerError::KeyBlob(format!(
            "unsupported portable key vault version: {}",
            bytes[offset]
        )));
    }
    offset += 1;
    let key_id_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
    offset += 2;
    let key_id_end = offset
        .checked_add(key_id_len)
        .ok_or_else(|| SignerError::KeyBlob("portable vault keyId length overflow".to_string()))?;
    let salt_end = key_id_end
        .checked_add(PORTABLE_VAULT_SALT_LEN)
        .ok_or_else(|| SignerError::KeyBlob("portable vault salt length overflow".to_string()))?;
    let nonce_end = salt_end
        .checked_add(PORTABLE_VAULT_NONCE_LEN)
        .ok_or_else(|| SignerError::KeyBlob("portable vault nonce length overflow".to_string()))?;
    if nonce_end > bytes.len()
        || bytes.len() != nonce_end + PORTABLE_VAULT_SECRET_LEN + PORTABLE_VAULT_TAG_LEN
    {
        return Err(SignerError::KeyBlob(
            "portable key vault has an unexpected length".to_string(),
        ));
    }
    let key_id = std::str::from_utf8(&bytes[offset..key_id_end])
        .map_err(|_| SignerError::KeyBlob("portable vault keyId is not UTF-8".to_string()))?;
    validate_key_id(key_id)?;
    let salt = &bytes[key_id_end..salt_end];
    let nonce_bytes = &bytes[salt_end..nonce_end];
    let header = &bytes[..nonce_end];
    let ciphertext = &bytes[nonce_end..];
    let derived = portable_derive_key(password, salt)?;
    let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).map_err(|_| {
        SignerError::KeyBlob("portable vault cipher initialization failed".to_string())
    })?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: ciphertext,
                    aad: header,
                },
            )
            .map_err(|_| {
                SignerError::KeyBlob(
                    "portable vault password is incorrect or the vault was modified".to_string(),
                )
            })?,
    );
    if plaintext.len() != PORTABLE_VAULT_SECRET_LEN {
        return Err(SignerError::KeyBlob(
            "portable vault decrypted key has an invalid length".to_string(),
        ));
    }
    let mut secret = [0u8; PORTABLE_VAULT_SECRET_LEN];
    secret.copy_from_slice(&plaintext);
    let signing_key = SigningKey::from_bytes(&secret);
    secret.zeroize();
    Ok(ProtectedKey {
        key_id: key_id.to_string(),
        signing_key,
    })
}

/// Read a local Windows DPAPI-protected key blob.  The command never accepts
/// raw private-key bytes: only this encrypted file is opened by production
/// signing paths.
pub fn read_dpapi_key_blob(
    path: &Path,
    expected_key_id: &str,
) -> Result<ProtectedKey, SignerError> {
    validate_key_id(expected_key_id)?;
    ensure_no_reparse_components(path)?;
    let encrypted =
        fs::read(path).map_err(|error| SignerError::io("reading DPAPI key blob", path, error))?;
    let plaintext = dpapi_unprotect(&encrypted)?;
    decode_key_blob_plaintext(&plaintext, expected_key_id)
}

/// Parse the post-DPAPI envelope.  This is public for a controlled offline
/// provisioning/test utility, but the CLI has no command that accepts this
/// plaintext representation.
pub fn decode_key_blob_plaintext(
    plaintext: &[u8],
    expected_key_id: &str,
) -> Result<ProtectedKey, SignerError> {
    decode_key_blob_plaintext_with_expected(plaintext, Some(expected_key_id))
}

/// Decode a DPAPI envelope when the key id is not known in advance.  The
/// automatic GUI path only calls this inside the fixed local signer key
/// directory and still validates the envelope header, size, UTF-8 and legal
/// key-id contract.
pub fn decode_key_blob_plaintext_auto(plaintext: &[u8]) -> Result<ProtectedKey, SignerError> {
    decode_key_blob_plaintext_with_expected(plaintext, None)
}

fn decode_key_blob_plaintext_with_expected(
    plaintext: &[u8],
    expected_key_id: Option<&str>,
) -> Result<ProtectedKey, SignerError> {
    if plaintext.len() < KEY_BLOB_MAGIC.len() + 1 + 2 + 32 || !plaintext.starts_with(KEY_BLOB_MAGIC)
    {
        return Err(SignerError::KeyBlob(
            "DPAPI plaintext has an unknown key-blob header".to_string(),
        ));
    }
    let mut offset = KEY_BLOB_MAGIC.len();
    if plaintext[offset] != KEY_BLOB_VERSION {
        return Err(SignerError::KeyBlob(format!(
            "unsupported DPAPI key-blob version: {}",
            plaintext[offset]
        )));
    }
    offset += 1;
    let key_id_len = u16::from_le_bytes([plaintext[offset], plaintext[offset + 1]]) as usize;
    offset += 2;
    let key_id_end = offset
        .checked_add(key_id_len)
        .ok_or_else(|| SignerError::KeyBlob("keyId length overflow".to_string()))?;
    let secret_end = key_id_end
        .checked_add(32)
        .ok_or_else(|| SignerError::KeyBlob("private key length overflow".to_string()))?;
    if secret_end != plaintext.len() {
        return Err(SignerError::KeyBlob(
            "DPAPI key blob has an unexpected length".to_string(),
        ));
    }
    let key_id = std::str::from_utf8(&plaintext[offset..key_id_end])
        .map_err(|_| SignerError::KeyBlob("DPAPI key blob keyId is not UTF-8".to_string()))?;
    validate_key_id(key_id)?;
    if let Some(expected_key_id) = expected_key_id {
        if key_id != expected_key_id {
            return Err(SignerError::KeyBlob(format!(
                "DPAPI key blob keyId does not match requested keyId {expected_key_id}"
            )));
        }
    }
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&plaintext[key_id_end..secret_end]);
    Ok(ProtectedKey {
        key_id: key_id.to_string(),
        signing_key: SigningKey::from_bytes(&secret),
    })
}

pub fn read_dpapi_key_blob_auto(path: &Path) -> Result<ProtectedKey, SignerError> {
    ensure_no_reparse_components(path)?;
    let encrypted = fs::read(path)
        .map_err(|error| SignerError::io("reading automatic DPAPI key blob", path, error))?;
    let plaintext = dpapi_unprotect(&encrypted)?;
    decode_key_blob_plaintext_auto(&plaintext)
}

/// Resolve the only signer key without asking the user to select a file.  A
/// fresh local key is generated only when the managed key directory is empty;
/// existing malformed or duplicate blobs are errors, never candidates for
/// silent replacement or arbitrary selection.
pub fn auto_resolve_key() -> Result<AutoResolvedKey, SignerError> {
    // Reuse the signer-wide transaction mutex so two GUI/CLI instances cannot
    // provision or rotate the same local key directory concurrently.
    let _signing_mutex = crate::windows::acquire_signing_mutex()?;
    let directory = default_key_blob_directory()?;
    ensure_fixed_local_path(&directory)?;
    fs::create_dir_all(&directory)
        .map_err(|error| SignerError::io("creating automatic key directory", &directory, error))?;
    ensure_fixed_local_path(&directory)?;

    let mut blobs = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| {
        SignerError::io("enumerating automatic key directory", &directory, error)
    })? {
        let entry = entry.map_err(|error| {
            SignerError::io("reading automatic key directory entry", &directory, error)
        })?;
        let path = entry.path();
        if !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("dpapi"))
        {
            continue;
        }
        ensure_no_reparse_components(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            SignerError::io("reading automatic key blob attributes", &path, error)
        })?;
        if !metadata.is_file() {
            return Err(SignerError::KeyBlob(format!(
                "automatic key entry is not a regular file: {}",
                path.display()
            )));
        }
        let key = read_dpapi_key_blob_auto(&path).map_err(|error| {
            SignerError::KeyBlob(format!(
                "cannot use automatic DPAPI key blob {}: {error}",
                path.display()
            ))
        })?;
        blobs.push((path, key));
    }

    if blobs.len() > 1 {
        return Err(SignerError::KeyBlob(
            "本机存在多个可用签名密钥；为避免误用，未自动选择任何一个".to_string(),
        ));
    }
    if let Some((path, key)) = blobs.pop() {
        write_public_descriptor(&directory, &key)?;
        return Ok(AutoResolvedKey {
            path,
            key,
            generated: false,
        });
    }

    let key_id = DEFAULT_KEY_ID;
    let path = default_key_blob_path(key_id)?;
    if path.exists() {
        // A race or an interrupted previous attempt must never overwrite an
        // existing secret.  Read it and fail closed if it is malformed.
        let key = read_dpapi_key_blob(&path, key_id)?;
        write_public_descriptor(&directory, &key)?;
        return Ok(AutoResolvedKey {
            path,
            key,
            generated: false,
        });
    }

    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| SignerError::KeyBlob(format!("生成本机签名密钥随机数失败：{error}")))?;
    let plaintext = encode_key_blob_plaintext(key_id, &secret)?;
    let encrypted = dpapi_protect(&plaintext)?;
    write_new_blob(&path, &encrypted)?;
    let key = read_dpapi_key_blob(&path, key_id)?;
    write_public_descriptor(&directory, &key)?;
    Ok(AutoResolvedKey {
        path,
        key,
        generated: true,
    })
}

fn encode_key_blob_plaintext(key_id: &str, secret: &[u8; 32]) -> Result<Vec<u8>, SignerError> {
    validate_key_id(key_id)?;
    let key_id_bytes = key_id.as_bytes();
    let key_id_len = u16::try_from(key_id_bytes.len()).map_err(|_| {
        SignerError::KeyBlob("keyId is too long for the DPAPI envelope".to_string())
    })?;
    let mut value = Vec::with_capacity(KEY_BLOB_MAGIC.len() + 1 + 2 + key_id_bytes.len() + 32);
    value.extend_from_slice(KEY_BLOB_MAGIC);
    value.push(KEY_BLOB_VERSION);
    value.extend_from_slice(&key_id_len.to_le_bytes());
    value.extend_from_slice(key_id_bytes);
    value.extend_from_slice(secret);
    Ok(value)
}

fn default_key_blob_directory() -> Result<PathBuf, SignerError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        SignerError::KeyBlob(
            "LOCALAPPDATA is required to resolve the automatic signer key store".to_string(),
        )
    })?;
    Ok(PathBuf::from(local_app_data)
        .join("CodexUsbLicenseSigner")
        .join("keys"))
}

fn write_new_blob(path: &Path, encrypted: &[u8]) -> Result<(), SignerError> {
    ensure_fixed_local_path(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| SignerError::io("creating automatic DPAPI key blob", path, error))?;
    file.write_all(encrypted)
        .and_then(|_| file.sync_all())
        .map_err(|error| SignerError::io("writing automatic DPAPI key blob", path, error))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicKeyDescriptor<'a> {
    key_id: &'a str,
    public_key_b64: String,
    public_key_sha256: String,
    protected_by: &'static str,
}

fn write_public_descriptor(directory: &Path, key: &ProtectedKey) -> Result<(), SignerError> {
    let path = directory.join(format!("{}.public.json", key.key_id));
    ensure_fixed_local_path(&path)?;
    let descriptor = PublicKeyDescriptor {
        key_id: &key.key_id,
        public_key_b64: key.public_key_b64(),
        public_key_sha256: key.public_key_sha256(),
        protected_by: "Windows DPAPI current-user key blob",
    };
    let bytes = serde_json::to_vec_pretty(&descriptor)
        .map_err(|error| SignerError::KeyBlob(format!("序列化公钥状态失败：{error}")))?;
    let temporary = directory.join(format!(".{}.public-{}.tmp", key.key_id, Uuid::new_v4()));
    fs::write(&temporary, bytes)
        .map_err(|error| SignerError::io("writing local public key status", &temporary, error))?;
    let result = atomic_replace(&temporary, &path);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn default_key_blob_path(key_id: &str) -> Result<PathBuf, SignerError> {
    validate_key_id(key_id)?;
    Ok(default_key_blob_directory()?.join(format!("{key_id}.dpapi")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> ProtectedKey {
        ProtectedKey {
            key_id: "test-key".to_string(),
            signing_key: SigningKey::from_bytes(&[9u8; 32]),
        }
    }

    fn temp_vault_path() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("codex-usb-signer-vault-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        directory.join(PORTABLE_KEY_VAULT_FILE_NAME)
    }

    fn plaintext(key_id: &str) -> Vec<u8> {
        let mut value = KEY_BLOB_MAGIC.to_vec();
        value.push(KEY_BLOB_VERSION);
        value.extend_from_slice(&(key_id.len() as u16).to_le_bytes());
        value.extend_from_slice(key_id.as_bytes());
        value.extend_from_slice(&[9u8; 32]);
        value
    }

    #[test]
    fn controlled_plaintext_envelope_requires_matching_key_id() {
        let material = decode_key_blob_plaintext(&plaintext("test-key"), "test-key").unwrap();
        assert_eq!(material.key_id, "test-key");
        assert_eq!(material.public_key_b64().len(), 44);
        assert_eq!(material.public_key_sha256().len(), 64);
        assert!(decode_key_blob_plaintext(&plaintext("test-key"), "other-key").is_err());
        assert_eq!(
            decode_key_blob_plaintext_auto(&plaintext("test-key"))
                .unwrap()
                .key_id,
            "test-key"
        );
    }

    #[test]
    fn portable_vault_round_trip_preserves_public_binding() {
        let path = temp_vault_path();
        let key = test_key();
        write_portable_key_vault(&path, &key, "a-strong-test-password").unwrap();
        let restored = read_portable_key_vault(&path, "a-strong-test-password").unwrap();
        assert_eq!(restored.key_id, key.key_id);
        assert_eq!(restored.public_key_b64(), key.public_key_b64());
        assert_eq!(restored.public_key_sha256(), key.public_key_sha256());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn portable_vault_descriptor_contains_only_the_public_binding() {
        let path = temp_vault_path();
        let key = test_key();
        write_portable_key_vault(&path, &key, "a-strong-test-password").unwrap();
        let descriptor = write_portable_key_vault_descriptor(&path, &key).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&descriptor).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["keyId"], key.key_id);
        assert_eq!(value["publicKeySha256"], key.public_key_sha256());
        assert!(!value.to_string().contains("signingKey"));
        assert!(value.get("signingKey").is_none());
        assert!(value.get("privateKey").is_none());
        assert!(value.get("password").is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn portable_vault_rejects_wrong_password_and_short_password() {
        let path = temp_vault_path();
        let key = test_key();
        assert!(write_portable_key_vault(&path, &key, "short").is_err());
        write_portable_key_vault(&path, &key, "a-strong-test-password").unwrap();
        let error = read_portable_key_vault(&path, "a-different-password").unwrap_err();
        assert!(error.to_string().contains("incorrect") || error.to_string().contains("modified"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn portable_vault_is_create_only_and_detects_truncation() {
        let path = temp_vault_path();
        let key = test_key();
        write_portable_key_vault(&path, &key, "a-strong-test-password").unwrap();
        let existing = write_portable_key_vault(&path, &key, "a-strong-test-password");
        assert!(existing.is_err());
        fs::write(&path, PORTABLE_VAULT_MAGIC).unwrap();
        assert!(read_portable_key_vault(&path, "a-strong-test-password").is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
