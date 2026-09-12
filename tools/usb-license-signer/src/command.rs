use crate::{bundle, key_blob, media, path_security, transaction, windows};

use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::SignerError;
use bundle::{deep_verify_bundle_with_mode, read_signing_binding, AuthenticodeMode};
use codex_usb_license::validate_key_id;
use key_blob::{auto_resolve_key, default_key_blob_path, read_dpapi_key_blob, ProtectedKey};
use media::{
    observe_target, scan_removable_candidates, sole_candidate_is_target, PhysicalObservation,
};
use path_security::{
    ensure_no_reparse_components, normalize_bundle_relative_path, normalize_drive_root,
};
use serde::Serialize;
use serde_json::{json, Value};
use transaction::{
    build_signing_candidate, digest_bytes, inspect_license, license_path, load_existing_license,
    now_ms, read_back_license, sign_candidate, verify_signed_license, write_license_atomic,
    write_old_license_audit,
};
use windows::{acquire_signing_mutex, atomic_replace, ensure_local_path};

const PRIVATE_KEY_ENV_NAMES: &[&str] = &[
    "CODEX_USB_SIGNER_PRIVATE_KEY",
    "CODEX_USB_PRIVATE_KEY",
    "USB_SIGNER_PRIVATE_KEY",
];

#[derive(Debug, Default)]
struct Arguments {
    command: String,
    drive: Option<String>,
    bundle_relative_path: Option<String>,
    key_id: Option<String>,
    confirm: Option<String>,
    report: Option<PathBuf>,
    key_blob: Option<PathBuf>,
    public_key_b64: Option<String>,
    skip_authenticode: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationReport {
    drive_root: String,
    volume_serial: u32,
    file_system: String,
    bundle_relative_path: String,
    bundle_path: String,
    anchor_schema: u32,
    anchor_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignOutput {
    command: &'static str,
    status: &'static str,
    observation: ObservationReport,
    installation_id: String,
    key_id: String,
    license_digest: String,
    payload_sha256: String,
    payload_size: u64,
    authenticode_mode: &'static str,
    old_license_audit_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyOutput {
    command: &'static str,
    verified: bool,
    observation: ObservationReport,
    license_digest: String,
    installation_id: String,
    key_id: String,
    payload_sha256: String,
    payload_size: u64,
    authenticode_mode: &'static str,
}

pub fn run_with_args<I>(arguments: I) -> Result<Value, SignerError>
where
    I: IntoIterator<Item = OsString>,
{
    reject_private_key_environment_inputs()?;
    let parsed = parse_args(arguments)?;
    match parsed.command.as_str() {
        "scan" => run_scan(&parsed),
        "inspect" => run_inspect(&parsed),
        "sign" => run_sign(&parsed),
        "verify" => run_verify(&parsed),
        "init-key" => run_init_key(&parsed),
        _ => Err(SignerError::InvalidArgument(format!(
            "unknown command {}; use scan, inspect, sign, verify, or init-key",
            parsed.command
        ))),
    }
}

fn parse_args<I>(arguments: I) -> Result<Arguments, SignerError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut values = arguments.into_iter();
    let command = values
        .next()
        .ok_or_else(|| SignerError::InvalidArgument(usage().to_string()))?
        .into_string()
        .map_err(|_| SignerError::InvalidArgument("command is not valid UTF-8".to_string()))?;
    if command == "--help" || command == "-h" {
        return Err(SignerError::HelpRequested(usage().to_string()));
    }
    let mut parsed = Arguments {
        command,
        ..Arguments::default()
    };
    let mut seen = std::collections::HashSet::new();
    while let Some(raw) = values.next() {
        let text = raw
            .into_string()
            .map_err(|_| SignerError::InvalidArgument("argument is not valid UTF-8".to_string()))?;
        let (flag, inline_value) = match text.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (text, None),
        };
        if flag == "--help" || flag == "-h" {
            return Err(SignerError::HelpRequested(usage().to_string()));
        }
        let mut value = || -> Result<String, SignerError> {
            if let Some(value) = inline_value.clone() {
                return Ok(value);
            }
            values
                .next()
                .ok_or_else(|| SignerError::InvalidArgument(format!("{flag} requires a value")))?
                .into_string()
                .map_err(|_| SignerError::InvalidArgument(format!("{flag} value is not UTF-8")))
        };
        if !seen.insert(flag.clone()) {
            return Err(SignerError::InvalidArgument(format!(
                "duplicate argument: {flag}"
            )));
        }
        match flag.as_str() {
            "--drive" => parsed.drive = Some(value()?),
            "--bundle-relative-path" => parsed.bundle_relative_path = Some(value()?),
            "--key-id" => parsed.key_id = Some(value()?),
            "--confirm" => parsed.confirm = Some(value()?),
            "--report" => parsed.report = Some(PathBuf::from(value()?)),
            "--key-blob" => parsed.key_blob = Some(PathBuf::from(value()?)),
            "--public-key-b64" => parsed.public_key_b64 = Some(value()?),
            "--skip-authenticode" => {
                if inline_value.is_some() {
                    return Err(SignerError::InvalidArgument(
                        "--skip-authenticode does not accept a value".to_string(),
                    ));
                }
                parsed.skip_authenticode = true;
            }
            _ => {
                return Err(SignerError::InvalidArgument(format!(
                    "unknown argument: {flag}"
                )))
            }
        }
    }
    Ok(parsed)
}

fn run_scan(arguments: &Arguments) -> Result<Value, SignerError> {
    reject_unused(arguments, &["scan"])?;
    let candidates = scan_removable_candidates()?;
    Ok(json!({
        "command": "scan",
        "candidateCount": candidates.len(),
        "candidates": candidates,
    }))
}

fn run_init_key(arguments: &Arguments) -> Result<Value, SignerError> {
    reject_unused(arguments, &["init-key"])?;
    let resolved = auto_resolve_key()?;
    Ok(json!({
        "command": "init-key",
        "generated": resolved.generated,
        "keyId": resolved.key.key_id,
        "publicKeyB64": resolved.key.public_key_b64(),
        "publicKeySha256": resolved.key.public_key_sha256(),
        "keyBlobPath": resolved.path.display().to_string(),
    }))
}

fn run_inspect(arguments: &Arguments) -> Result<Value, SignerError> {
    let (drive, relative) = target_arguments(arguments)?;
    let observation = observe_target(&drive, &relative)?;
    let license = inspect_license(&observation.bundle_path, Some(&observation.fingerprint))?;
    let deep_verify =
        inspect_deep_verify_with_mode(&observation.bundle_path, authenticode_mode(arguments))?;
    let signing = match read_signing_binding(&observation.bundle_path) {
        Ok(binding) => json!({
            "ok": true,
            "policy": binding.policy,
            "keyId": binding.key_id,
            "publicKeySha256": binding.public_key_sha256,
        }),
        Err(error) => json!({
            "ok": false,
            "error": error.to_string(),
        }),
    };
    let output = json!({
        "command": "inspect",
        "observation": observation_report(&observation),
        "license": license,
        "deepVerify": deep_verify,
        "signing": signing,
    });
    if let Some(report) = arguments.report.as_deref() {
        write_local_report(report, &drive, &output)?;
    }
    Ok(output)
}

fn inspect_deep_verify(bundle_path: &Path) -> Result<Value, SignerError> {
    inspect_deep_verify_with_mode(bundle_path, AuthenticodeMode::Required)
}

fn inspect_deep_verify_with_mode(
    bundle_path: &Path,
    authenticode: AuthenticodeMode,
) -> Result<Value, SignerError> {
    deep_verify_bundle_with_mode(bundle_path, authenticode)
        .map(|summary| {
            json!({
                "ok": true,
                "summary": summary,
                "authenticodeMode": authenticode_mode_label(authenticode),
            })
        })
        .map_err(|error| {
            SignerError::BundleVerification(format!(
                "inspect deep verification failed for {}: {error}",
                bundle_path.display()
            ))
        })
}

fn run_sign(arguments: &Arguments) -> Result<Value, SignerError> {
    let _signing_mutex = acquire_signing_mutex()?;
    let (drive, relative) = target_arguments(arguments)?;
    let confirm = required_string(arguments.confirm.as_deref(), "--confirm")?;
    if arguments.key_id.is_none() && arguments.key_blob.is_some() {
        return Err(SignerError::InvalidArgument(
            "--key-blob cannot be used without --key-id".to_string(),
        ));
    }
    let authenticode = authenticode_mode(arguments);

    let candidates = scan_removable_candidates()?;
    sole_candidate_is_target(&candidates, &drive, &relative)?;
    let before = observe_target(&drive, &relative)?;
    let preflight = deep_verify_bundle_with_mode(&before.bundle_path, authenticode)?;
    let binding = read_signing_binding(&before.bundle_path)?;
    let old_inspection = inspect_license(&before.bundle_path, Some(&before.fingerprint))?;
    let key_context = if let Some(key_id) = arguments.key_id.as_deref() {
        validate_key_id(key_id)?;
        let key_blob_path = arguments
            .key_blob
            .clone()
            .unwrap_or(default_key_blob_path(key_id)?);
        ensure_local_path(&key_blob_path, &drive)?;
        let key = read_dpapi_key_blob(&key_blob_path, key_id)?;
        (key_blob_path, key)
    } else {
        let resolved = auto_resolve_key()?;
        (resolved.path, resolved.key)
    };
    let _key_blob_path = key_context.0;
    let key = key_context.1;
    let key_id = key.key_id.clone();
    ensure_key_matches_binding(&binding, &key)?;
    eprintln!(
        "SIGN REVIEW drive={} volumeSerial={} filesystem={} bundleRelativePath={} anchorSchema={} anchorSha256={} installationId={} licenseState={:?} payloadSha256={} payloadSize={} keyId={}",
        before.volume.drive_root,
        before.volume.volume_serial,
        before.volume.file_system,
        before.fingerprint.bundle_relative_path,
        before.fingerprint.anchor_schema,
        before.fingerprint.anchor_sha256,
        old_inspection
            .installation_id
            .as_deref()
            .unwrap_or("<new>"),
        old_inspection.state,
        preflight.payload_sha256,
        preflight.payload_size,
        key_id
    );
    let expected_confirmation = format!(
        "SIGN {}-{}",
        before.volume.volume_serial,
        before
            .fingerprint
            .anchor_sha256
            .chars()
            .take(12)
            .collect::<String>()
    );
    if confirm != expected_confirmation {
        return Err(SignerError::ConfirmationRequired {
            expected: expected_confirmation,
        });
    }

    let (mut candidate, old_license) =
        build_signing_candidate(&before.bundle_path, &before, &key_id, now_ms())?;
    let audit_directory = default_audit_directory(&drive)?;
    let old_audit = write_old_license_audit(&old_license.bytes, &audit_directory)?;
    sign_candidate(&mut candidate, &key)?;
    verify_signed_license(&candidate, &before, &key.public_key_b64())?;

    let pre_write = observe_target(&drive, &relative).map_err(|error| {
        SignerError::DriveChanged(format!("pre-write physical observation failed: {error}"))
    })?;
    ensure_observation_unchanged(&before, &pre_write)?;
    write_license_atomic(&pre_write.bundle_path, &candidate)?;

    let after = observe_target(&drive, &relative).map_err(|error| {
        SignerError::OutcomeUnknown(format!("post-write physical observation failed: {error}"))
    })?;
    ensure_observation_unchanged(&before, &after).map_err(|error| {
        SignerError::OutcomeUnknown(format!("post-write physical observation changed: {error}"))
    })?;
    let readback = read_back_license(&after.bundle_path).map_err(|error| {
        SignerError::OutcomeUnknown(format!("post-write license readback failed: {error}"))
    })?;
    verify_signed_license(&readback, &after, &key.public_key_b64()).map_err(|error| {
        SignerError::OutcomeUnknown(format!("post-write license verification failed: {error}"))
    })?;
    let postflight =
        deep_verify_bundle_with_mode(&after.bundle_path, authenticode).map_err(|error| {
            SignerError::OutcomeUnknown(format!(
                "post-write deep bundle verification failed: {error}"
            ))
        })?;
    let written_bytes = fs::read(license_path(&after.bundle_path)).map_err(|error| {
        SignerError::OutcomeUnknown(format!("post-write license digest read failed: {error}"))
    })?;
    let output = serde_json::to_value(SignOutput {
        command: "sign",
        status: "success",
        observation: observation_report(&after),
        installation_id: readback.installation_id,
        key_id,
        license_digest: digest_bytes(&written_bytes),
        payload_sha256: postflight.payload_sha256,
        payload_size: postflight.payload_size,
        authenticode_mode: authenticode_mode_label(authenticode),
        old_license_audit_path: old_audit.map(|path| path.display().to_string()),
    })
    .map_err(|error| SignerError::Report(format!("serializing sign report failed: {error}")))?;
    if let Some(report) = arguments.report.as_deref() {
        write_local_report(report, &drive, &output)?;
    }
    Ok(output)
}

fn ensure_observation_unchanged(
    expected: &PhysicalObservation,
    actual: &PhysicalObservation,
) -> Result<(), SignerError> {
    let expected_drive = normalize_drive_root(&expected.volume.drive_root).map_err(|error| {
        SignerError::DriveChanged(format!("expected drive root is invalid: {error}"))
    })?;
    let actual_drive = normalize_drive_root(&actual.volume.drive_root).map_err(|error| {
        SignerError::DriveChanged(format!("observed drive root is invalid: {error}"))
    })?;
    let expected_relative = normalize_bundle_relative_path(
        &expected.fingerprint.bundle_relative_path,
    )
    .map_err(|error| {
        SignerError::DriveChanged(format!("expected bundle-relative path is invalid: {error}"))
    })?;
    let actual_relative = normalize_bundle_relative_path(&actual.fingerprint.bundle_relative_path)
        .map_err(|error| {
            SignerError::DriveChanged(format!("observed bundle-relative path is invalid: {error}"))
        })?;

    let expected_bundle_path = normalized_path_text(&expected.bundle_path);
    let actual_bundle_path = normalized_path_text(&actual.bundle_path);
    let mut changes = Vec::new();
    if !expected_drive.eq_ignore_ascii_case(&actual_drive) {
        changes.push(format!(
            "drive root changed from {expected_drive} to {actual_drive}"
        ));
    }
    if expected.volume.volume_serial != actual.volume.volume_serial {
        changes.push(format!(
            "volume serial changed from {} to {}",
            expected.volume.volume_serial, actual.volume.volume_serial
        ));
    }
    if !expected
        .volume
        .file_system
        .eq_ignore_ascii_case(&actual.volume.file_system)
    {
        changes.push(format!(
            "filesystem changed from {} to {}",
            expected.volume.file_system, actual.volume.file_system
        ));
    }
    if expected_bundle_path != actual_bundle_path {
        changes.push(format!(
            "normalized bundle path changed from {expected_bundle_path} to {actual_bundle_path}"
        ));
    }
    if expected_relative != actual_relative {
        changes.push(format!(
            "normalized bundle-relative path changed from {expected_relative} to {actual_relative}"
        ));
    }
    if expected.fingerprint.volume_serial != actual.fingerprint.volume_serial {
        changes.push(format!(
            "fingerprint volume serial changed from {} to {}",
            expected.fingerprint.volume_serial, actual.fingerprint.volume_serial
        ));
    }
    if expected.fingerprint.anchor_schema != actual.fingerprint.anchor_schema {
        changes.push(format!(
            "fingerprint anchor schema changed from {} to {}",
            expected.fingerprint.anchor_schema, actual.fingerprint.anchor_schema
        ));
    }
    if !expected
        .fingerprint
        .anchor_sha256
        .eq_ignore_ascii_case(&actual.fingerprint.anchor_sha256)
    {
        changes.push(format!(
            "fingerprint anchor SHA-256 changed from {} to {}",
            expected.fingerprint.anchor_sha256, actual.fingerprint.anchor_sha256
        ));
    }

    if changes.is_empty() {
        Ok(())
    } else {
        Err(SignerError::DriveChanged(changes.join("; ")))
    }
}

fn normalized_path_text(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let value = value
        .strip_prefix("//?/")
        .or_else(|| value.strip_prefix("//./"))
        .unwrap_or(&value);
    let value = value.trim_end_matches('/');
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value.to_string()
    }
}

fn run_verify(arguments: &Arguments) -> Result<Value, SignerError> {
    let (drive, relative) = target_arguments(arguments)?;
    let observation = observe_target(&drive, &relative)?;
    let authenticode = authenticode_mode(arguments);
    let bundle = deep_verify_bundle_with_mode(&observation.bundle_path, authenticode)?;
    let binding = read_signing_binding(&observation.bundle_path)?;
    let existing = load_existing_license(&observation.bundle_path)?;
    let license = existing.license.ok_or_else(|| {
        SignerError::ExistingLicense("verify requires a signed license".to_string())
    })?;
    if license.signature.is_none() {
        return Err(SignerError::ExistingLicense(
            "verify requires a signed license; unsigned development metadata is not enough"
                .to_string(),
        ));
    }
    let license_key_id = license.key_id.as_deref().ok_or_else(|| {
        SignerError::ExistingLicense("signed license is missing keyId".to_string())
    })?;
    validate_key_id(license_key_id)?;
    if let Some(requested) = arguments.key_id.as_deref() {
        if requested != license_key_id {
            return Err(SignerError::ExistingLicense(
                "requested keyId does not match the license keyId".to_string(),
            ));
        }
    }
    let public_key_b64 = if let Some(public_key) = arguments.public_key_b64.as_deref() {
        let digest = public_key_sha256_b64(public_key)?;
        if digest != binding.public_key_sha256 || binding.key_id != license_key_id {
            return Err(SignerError::KeyBlob(
                "显式公钥与当前 Bundle 的签名绑定不一致".to_string(),
            ));
        }
        public_key.to_string()
    } else {
        let key = if arguments.key_id.is_none() && arguments.key_blob.is_none() {
            auto_resolve_key()?.key
        } else {
            let path = arguments
                .key_blob
                .clone()
                .unwrap_or(default_key_blob_path(license_key_id)?);
            ensure_local_path(&path, &drive)?;
            read_dpapi_key_blob(&path, license_key_id)?
        };
        ensure_key_matches_binding(&binding, &key)?;
        key.public_key_b64()
    };
    verify_signed_license(&license, &observation, &public_key_b64)?;
    let output = serde_json::to_value(VerifyOutput {
        command: "verify",
        verified: true,
        observation: observation_report(&observation),
        license_digest: digest_bytes(&existing.bytes),
        installation_id: license.installation_id,
        key_id: license_key_id.to_string(),
        payload_sha256: bundle.payload_sha256,
        payload_size: bundle.payload_size,
        authenticode_mode: authenticode_mode_label(authenticode),
    })
    .map_err(|error| SignerError::Report(format!("serializing verify report failed: {error}")))?;
    if let Some(report) = arguments.report.as_deref() {
        write_local_report(report, &drive, &output)?;
    }
    Ok(output)
}

fn ensure_key_matches_binding(
    binding: &bundle::SigningBinding,
    key: &ProtectedKey,
) -> Result<(), SignerError> {
    if binding.key_id != key.key_id {
        return Err(SignerError::KeyBlob(format!(
            "本机密钥 keyId={} 与当前 Bundle 要求的 keyId={} 不一致",
            key.key_id, binding.key_id
        )));
    }
    if !binding
        .public_key_sha256
        .eq_ignore_ascii_case(&key.public_key_sha256())
    {
        return Err(SignerError::KeyBlob(
            "本机密钥公钥与当前 Bundle 的公钥绑定不一致；未写入 U 盘".to_string(),
        ));
    }
    Ok(())
}

fn public_key_sha256_b64(value: &str) -> Result<String, SignerError> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    let bytes = BASE64
        .decode(value.trim())
        .map_err(|error| SignerError::KeyBlob(format!("显式公钥不是有效 Base64：{error}")))?;
    if bytes.len() != 32 {
        return Err(SignerError::KeyBlob(
            "显式公钥必须是 32 字节 Ed25519 公钥".to_string(),
        ));
    }
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn target_arguments(arguments: &Arguments) -> Result<(String, String), SignerError> {
    let drive = required_string(arguments.drive.as_deref(), "--drive")?;
    let relative = required_string(
        arguments.bundle_relative_path.as_deref(),
        "--bundle-relative-path",
    )?;
    Ok((drive, relative))
}

fn reject_unused(arguments: &Arguments, allowed: &[&str]) -> Result<(), SignerError> {
    let _ = allowed;
    if arguments.drive.is_some()
        || arguments.bundle_relative_path.is_some()
        || arguments.key_id.is_some()
        || arguments.confirm.is_some()
        || arguments.report.is_some()
        || arguments.key_blob.is_some()
        || arguments.public_key_b64.is_some()
        || arguments.skip_authenticode
    {
        return Err(SignerError::InvalidArgument(
            "scan does not accept target or key arguments".to_string(),
        ));
    }
    Ok(())
}

fn required_string(value: Option<&str>, flag: &str) -> Result<String, SignerError> {
    let value = value.ok_or_else(|| SignerError::InvalidArgument(format!("{flag} is required")))?;
    if value.is_empty() {
        return Err(SignerError::InvalidArgument(format!(
            "{flag} must not be empty"
        )));
    }
    Ok(value.to_string())
}

fn authenticode_mode(arguments: &Arguments) -> AuthenticodeMode {
    if arguments.skip_authenticode {
        AuthenticodeMode::Skipped
    } else {
        AuthenticodeMode::Required
    }
}

fn authenticode_mode_label(mode: AuthenticodeMode) -> &'static str {
    match mode {
        AuthenticodeMode::Required => "required",
        AuthenticodeMode::Skipped => "skipped",
    }
}

fn observation_report(observation: &PhysicalObservation) -> ObservationReport {
    ObservationReport {
        drive_root: observation.volume.drive_root.clone(),
        volume_serial: observation.volume.volume_serial,
        file_system: observation.volume.file_system.clone(),
        bundle_relative_path: observation.fingerprint.bundle_relative_path.clone(),
        bundle_path: observation.bundle_path.display().to_string(),
        anchor_schema: observation.fingerprint.anchor_schema,
        anchor_sha256: observation.fingerprint.anchor_sha256.clone(),
    }
}

fn default_audit_directory(drive: &str) -> Result<PathBuf, SignerError> {
    let local = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        SignerError::Report("LOCALAPPDATA is required for the local audit directory".to_string())
    })?;
    let path = PathBuf::from(local)
        .join("CodexUsbLicenseSigner")
        .join("audit");
    ensure_local_path(&path, drive)?;
    Ok(path)
}

fn write_local_report(path: &Path, target_drive: &str, value: &Value) -> Result<(), SignerError> {
    ensure_local_path(path, target_drive)
        .map_err(|error| SignerError::Report(error.to_string()))?;
    let parent = path.parent().ok_or_else(|| {
        SignerError::Report(format!("report path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        SignerError::Report(format!("creating report directory failed: {error}"))
    })?;
    ensure_no_reparse_components(parent).map_err(|error| SignerError::Report(error.to_string()))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| SignerError::Report(format!("serializing report failed: {error}")))?;
    let temporary = parent.join(format!(".usb-signer-report-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                SignerError::Report(format!("creating report temporary file failed: {error}"))
            })?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| SignerError::Report(format!("flushing report failed: {error}")))?;
        atomic_replace(&temporary, path).map_err(|error| SignerError::Report(error.to_string()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn reject_private_key_environment_inputs() -> Result<(), SignerError> {
    for name in PRIVATE_KEY_ENV_NAMES {
        if env::var_os(name).is_some() {
            return Err(SignerError::InvalidArgument(format!(
                "{name} is forbidden; production signing accepts only a Windows DPAPI key blob"
            )));
        }
    }
    Ok(())
}

fn usage() -> &'static str {
    "usb-license-signer scan\nusb-license-signer inspect --drive E: --bundle-relative-path CodexUsb [--skip-authenticode] [--report PATH]\nusb-license-signer sign --drive E: --bundle-relative-path CodexUsb --confirm \"SIGN <volumeSerial>-<anchorSha256-first12>\" [--key-id ID --key-blob PATH] [--skip-authenticode] [--report PATH]\nusb-license-signer verify --drive E: --bundle-relative-path CodexUsb [--key-id ID --key-blob PATH | --public-key-b64 BASE64] [--skip-authenticode] [--report PATH]\nusb-license-signer init-key\nGUI/自动流程：不传 key 参数时自动发现或初始化本机 DPAPI 密钥；init-key 只输出公钥信息"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn parser_keeps_confirmation_as_one_exact_value() {
        let args = parse_args(
            [
                "sign",
                "--drive",
                "E:",
                "--bundle-relative-path",
                "CodexUsb",
                "--key-id",
                "prod-2026-01",
                "--confirm",
                "SIGN 123-aabbccddeeff",
            ]
            .into_iter()
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(args.confirm.as_deref(), Some("SIGN 123-aabbccddeeff"));
    }

    #[test]
    fn parser_does_not_offer_a_private_key_argument() {
        let result = parse_args(
            ["sign", "--private-key", "secret"]
                .into_iter()
                .map(OsString::from),
        );
        assert!(result.is_err());
    }

    #[test]
    fn parser_accepts_explicit_internal_license_mode() {
        let args = parse_args(
            ["inspect", "--skip-authenticode"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap();
        assert!(args.skip_authenticode);
        assert_eq!(authenticode_mode(&args), AuthenticodeMode::Skipped);
    }

    #[test]
    fn scan_rejects_internal_license_mode_flag() {
        let args = parse_args(
            ["scan", "--skip-authenticode"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap();
        assert!(reject_unused(&args, &["scan"]).is_err());
    }

    #[test]
    fn help_is_a_successful_cli_control_request() {
        let result = parse_args(["--help"].into_iter().map(OsString::from));
        assert!(matches!(result, Err(SignerError::HelpRequested(_))));
    }

    #[test]
    fn output_types_do_not_include_signature_or_private_key_fields() {
        let value = serde_json::to_string(&json!({
            "installationId": "id",
            "keyId": "test",
            "licenseDigest": "digest"
        }))
        .unwrap();
        assert!(!value.contains("signature"));
        assert!(!value.contains("private"));
    }

    #[test]
    fn formal_bundle_binding_accepts_matching_key_and_rejects_wrong_digest() {
        let key = ProtectedKey {
            key_id: "prod-2026-01".to_string(),
            signing_key: ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]),
        };
        let binding = bundle::SigningBinding {
            policy: "SignedRequired".to_string(),
            key_id: key.key_id.clone(),
            public_key_sha256: key.public_key_sha256(),
        };
        assert!(ensure_key_matches_binding(&binding, &key).is_ok());

        let mut wrong_digest = binding;
        wrong_digest.public_key_sha256 = "0".repeat(64);
        assert!(matches!(
            ensure_key_matches_binding(&wrong_digest, &key),
            Err(SignerError::KeyBlob(_))
        ));
    }

    fn test_observation() -> PhysicalObservation {
        PhysicalObservation {
            volume: crate::windows::VolumeInfo {
                drive_root: "E:\\".to_string(),
                volume_serial: 1,
                file_system: "NTFS".to_string(),
            },
            bundle_path: PathBuf::from(r"E:\CodexUsb"),
            fingerprint: codex_usb_license::UsbFingerprint {
                volume_serial: 1,
                anchor_sha256: "a".repeat(64),
                bundle_relative_path: "codexusb".to_string(),
                anchor_schema: 2,
            },
        }
    }

    #[test]
    fn physical_identity_comparison_normalizes_equivalent_observations() {
        let mut actual = test_observation();
        if cfg!(windows) {
            actual.volume.drive_root = "e:".to_string();
            actual.volume.file_system = "ntfs".to_string();
            actual.bundle_path = PathBuf::from(r"\\?\e:\CODEXUSB");
        } else {
            actual.bundle_path = PathBuf::from("E:/CodexUsb");
        }
        actual.fingerprint.bundle_relative_path = "CodexUsb".to_string();
        actual.fingerprint.anchor_sha256 = "A".repeat(64);

        assert!(ensure_observation_unchanged(&test_observation(), &actual).is_ok());
    }

    #[test]
    fn physical_identity_changes_are_drive_changed_errors() {
        let base = test_observation();
        let mut changed = base.clone();
        changed.volume.drive_root = "F:\\".to_string();
        let cases = [
            ("drive root", changed),
            ("volume serial", {
                let mut observation = base.clone();
                observation.volume.volume_serial = 2;
                observation
            }),
            ("filesystem", {
                let mut observation = base.clone();
                observation.volume.file_system = "exFAT".to_string();
                observation
            }),
            ("normalized bundle path", {
                let mut observation = base.clone();
                observation.bundle_path = PathBuf::from(r"E:\OtherBundle");
                observation
            }),
            ("normalized bundle-relative path", {
                let mut observation = base.clone();
                observation.fingerprint.bundle_relative_path = "OtherBundle".to_string();
                observation
            }),
            ("fingerprint volume serial", {
                let mut observation = base.clone();
                observation.fingerprint.volume_serial = 2;
                observation
            }),
            ("fingerprint anchor schema", {
                let mut observation = base.clone();
                observation.fingerprint.anchor_schema = 1;
                observation
            }),
            ("fingerprint anchor SHA-256", {
                let mut observation = base.clone();
                observation.fingerprint.anchor_sha256 = "b".repeat(64);
                observation
            }),
        ];

        for (field, changed) in cases {
            let error = ensure_observation_unchanged(&base, &changed)
                .expect_err("changed physical identity must fail closed");
            let message = error.to_string();
            assert!(
                matches!(error, SignerError::DriveChanged(_)),
                "{field}: {message}"
            );
            assert!(message.contains(field), "{field}: {message}");
        }
    }

    #[test]
    fn inspect_deep_verify_failure_is_an_error_with_diagnostics() {
        let bundle_path = std::env::temp_dir().join(format!(
            "usb-license-signer-missing-inspect-{}",
            uuid::Uuid::new_v4()
        ));
        let path_text = bundle_path.display().to_string();
        let error = inspect_deep_verify(&bundle_path).expect_err("missing bundle must fail");

        match error {
            SignerError::BundleVerification(message) => {
                assert!(message.contains("inspect deep verification failed"));
                assert!(message.contains(&path_text));
            }
            other => panic!("inspect deep verification returned the wrong error: {other:?}"),
        }
    }
}
