use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use codex_usb_license::{
    anchor_sha256, bundle_anchor_schema, validate_key_id, ANCHOR_SCHEMA_V3,
    REVOCATION_REGISTRATION_FILE_NAME,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::error::SignerError;
use crate::path_security::{
    ensure_tree_has_no_reparse_points, normalize_bundle_relative_path, path_is_within,
};
use crate::windows::{verify_authenticode, AuthenticodeSubject};

pub const EXPECTED_WEBVIEW2_SHA256: &str =
    "1f4638309f3d82c31a3028c3cf7d75998f58e4d1407380f5cb8a8e9172caf17d";
pub const EXPECTED_VC_REDIST_SHA256: &str =
    "843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c";
pub const EXPECTED_MSIX_SIGNER_FRAGMENT: &str = "50bdfd77-8903-4850-9ffe-6e8522f64d5b";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningBinding {
    pub policy: String,
    pub key_id: String,
    pub public_key_sha256: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticodeMode {
    Required,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummary {
    pub bundle_path: String,
    pub manifest_schema_version: u32,
    pub anchor_schema: u32,
    pub anchor_sha256: String,
    pub payload_path: String,
    pub payload_sha256: String,
    pub payload_size: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone)]
struct PayloadInfo {
    relative_path: String,
    sha256: String,
    size: u64,
    architecture: String,
}

pub fn deep_verify_bundle(bundle_root: &Path) -> Result<BundleSummary, SignerError> {
    verify_bundle(bundle_root, AuthenticodeMode::Required)
}

pub fn deep_verify_bundle_with_mode(
    bundle_root: &Path,
    authenticode: AuthenticodeMode,
) -> Result<BundleSummary, SignerError> {
    verify_bundle(bundle_root, authenticode)
}

/// The skipped mode is intended for the USB-internal-license workflow.  It
/// still performs every bundle structure, manifest, hash, and payload check;
/// it only omits Windows PE/MSIX Authenticode verification.  The default
/// command-line mode remains `Required`.
pub fn verify_bundle(
    bundle_root: &Path,
    authenticode: AuthenticodeMode,
) -> Result<BundleSummary, SignerError> {
    let root = fs::canonicalize(bundle_root)
        .map_err(|error| SignerError::io("canonicalizing bundle root", bundle_root, error))?;
    let metadata = fs::symlink_metadata(&root)
        .map_err(|error| SignerError::io("reading bundle root", &root, error))?;
    if !metadata.is_dir() {
        return Err(SignerError::bundle(format!(
            "bundle root is not a directory: {}",
            root.display()
        )));
    }
    ensure_tree_has_no_reparse_points(&root)?;
    reject_root_license_directory(&root)?;

    let manifest_path = required_file(&root, "manifest.json")?;
    let usb_manifest_path = required_file(&root, "usb-manifest.json")?;
    let sums_path = required_file(&root, "SHA256SUMS.txt")?;
    let manifest = read_json(&manifest_path)?;
    let usb_manifest = read_json(&usb_manifest_path)?;
    validate_root_manifest(&manifest)?;
    validate_usb_manifest(&usb_manifest)?;
    let manifest_schema = require_matching_document_schemas(&manifest, &usb_manifest)?;
    let root_anchor = required_u64(&manifest, "/updateCapability/anchorSchema")?;
    let usb_anchor = required_u64(&usb_manifest, "/updateCapability/anchorSchema")?;
    if root_anchor != ANCHOR_SCHEMA_V3 as u64 || usb_anchor != ANCHOR_SCHEMA_V3 as u64 {
        return Err(SignerError::bundle(
            "only schema 3 update bundles may be signed by this release signer",
        ));
    }
    if root_anchor != usb_anchor {
        return Err(SignerError::bundle(
            "root and USB update anchor schemas do not match",
        ));
    }

    let architecture = required_string(&manifest, "/architecture")?;
    let payload_relative =
        normalize_bundle_relative_path(required_string(&manifest, "/payload/path")?)?;
    if !payload_relative.to_ascii_lowercase().ends_with(".msix") {
        return Err(SignerError::bundle(
            "manifest payload.path must point to an .msix file",
        ));
    }
    let payload_path = required_file(&root, &payload_relative)?;
    let payload_size = required_u64(&manifest, "/payload/size")?;
    let payload_expected_hash = required_hash(&manifest, "/payload/sha256")?;
    let payload_actual = file_hash_and_size(&payload_path)?;
    if payload_actual.1 != payload_size || payload_actual.0 != payload_expected_hash {
        return Err(SignerError::bundle(format!(
            "payload hash/size mismatch: expected {} bytes {}, got {} bytes {}",
            payload_size, payload_expected_hash, payload_actual.1, payload_actual.0
        )));
    }

    let payload_info = verify_msix_payload(
        &payload_path,
        &payload_relative,
        architecture,
        manifest
            .pointer("/packageVersion")
            .and_then(Value::as_str)
            .map(str::to_string),
        payload_actual.0.clone(),
        payload_actual.1,
    )?;
    validate_cross_file_metadata(&root, &usb_manifest, &payload_info)?;
    validate_required_layout(&root)?;
    validate_backup_directory(&root, &payload_relative, architecture)?;

    let listed = verify_sha256_sums(&root, &sums_path)?;
    reject_unlisted_files(&root, &listed)?;

    if matches!(authenticode, AuthenticodeMode::Required) {
        verify_authenticode(
            &required_file(&root, "CodexDesktopSetup.exe")?,
            "CodexDesktopSetup.exe",
            AuthenticodeSubject::Any,
        )?;
        verify_authenticode(
            &required_file(&root, "CodexAppManager.exe")?,
            "CodexAppManager.exe",
            AuthenticodeSubject::Any,
        )?;
        verify_authenticode(
            &required_file(&root, "ccswitch/CodexSwitch.exe")?,
            "CCSwitch",
            AuthenticodeSubject::Any,
        )?;
        verify_authenticode(
            &required_file(&root, "resources/ccswitch/CodexSwitch.exe")?,
            "embedded CCSwitch",
            AuthenticodeSubject::Any,
        )?;
        verify_authenticode(
            &payload_path,
            "MSIX payload",
            AuthenticodeSubject::Contains(EXPECTED_MSIX_SIGNER_FRAGMENT),
        )?;
        verify_authenticode(
            &required_file(&root, "resources/prerequisites/vc_redist.x64.exe")?,
            "VC++ runtime",
            AuthenticodeSubject::Contains("Microsoft Corporation"),
        )?;
        verify_authenticode(
            &required_file(
                &root,
                "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
            )?,
            "WebView2 runtime",
            AuthenticodeSubject::Contains("Microsoft Corporation"),
        )?;
    }

    let anchor_schema = bundle_anchor_schema(&root)?;
    let anchor = anchor_sha256(&root)?;
    Ok(BundleSummary {
        bundle_path: root.display().to_string(),
        manifest_schema_version: manifest_schema as u32,
        anchor_schema,
        anchor_sha256: anchor,
        payload_path: payload_relative,
        payload_sha256: payload_info.sha256,
        payload_size: payload_info.size,
        file_count: listed.len(),
    })
}

/// Read the public trust binding declared by a formal USB Bundle.  This is
/// deliberately separate from the broad development Bundle validator: a
/// development/auto-enroll Bundle may be inspected, but it must never be
/// accepted as a target for offline signing.
pub fn read_signing_binding(bundle_root: &Path) -> Result<SigningBinding, SignerError> {
    let root = fs::canonicalize(bundle_root)
        .map_err(|error| SignerError::io("canonicalizing bundle root", bundle_root, error))?;
    let manifest = read_json(&required_file(&root, "usb-manifest.json")?)?;
    parse_signing_binding(&manifest)
}

fn parse_signing_binding(manifest: &Value) -> Result<SigningBinding, SignerError> {
    let signing = required_value(manifest, "/usbGuard/signing")?;
    let policy = required_string(signing, "/policy")?.to_string();
    if policy != "SignedRequired" {
        return Err(SignerError::bundle(format!(
            "USB Bundle 的签名策略不是 SignedRequired：{policy}"
        )));
    }
    let key_id = required_string(signing, "/keyId")?.to_string();
    validate_key_id(&key_id)
        .map_err(|error| SignerError::bundle(format!("USB Bundle 的签名 keyId 无效：{error}")))?;
    let public_key_sha256 = required_string(signing, "/publicKeySha256")?;
    if public_key_sha256.len() != 64
        || !public_key_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SignerError::bundle(
            "USB Bundle 的 publicKeySha256 必须是 64 位十六进制摘要",
        ));
    }
    Ok(SigningBinding {
        policy,
        key_id,
        public_key_sha256: public_key_sha256.to_ascii_lowercase(),
    })
}

fn validate_root_manifest(manifest: &Value) -> Result<(), SignerError> {
    let schema = require_supported_manifest_schema(manifest, "root manifest")?;
    validate_registration_policy(manifest, schema)?;
    require_string_equals(manifest, "/packageName", "OpenAI.Codex")?;
    let architecture = required_string(manifest, "/architecture")?;
    if !matches!(architecture, "x64" | "arm64") {
        return Err(SignerError::bundle(format!(
            "manifest architecture is not x64/arm64: {architecture}"
        )));
    }
    let update = required_value(manifest, "/updateCapability")?;
    validate_update_capability(update, "root updateCapability")?;
    let payload = required_value(manifest, "/payload")?;
    if !payload.is_object() {
        return Err(SignerError::bundle("manifest.payload must be an object"));
    }
    required_string(payload, "/path")?;
    required_hash(payload, "/sha256")?;
    required_u64(payload, "/size")?;
    let payload_relative = normalize_bundle_relative_path(required_string(payload, "/path")?)?;
    let parent = payload_relative.rsplit_once('/').map(|(parent, _)| parent);
    let expected_backup = parent
        .map(|value| format!("{value}/_backup"))
        .unwrap_or_else(|| "_backup".to_string());
    require_string_equals(update, "/backupDirectory", &expected_backup)?;
    Ok(())
}

fn validate_usb_manifest(manifest: &Value) -> Result<(), SignerError> {
    require_supported_manifest_schema(manifest, "USB manifest")?;
    require_string_equals(manifest, "/product", "codex-usb-oneclick")?;
    require_string_equals(manifest, "/status", "oneclick")?;
    require_bool_equals(manifest, "/secretsIncluded", false)?;
    require_bool_equals(manifest, "/automaticPostInstallLaunch", true)?;
    validate_update_capability(
        required_value(manifest, "/updateCapability")?,
        "USB updateCapability",
    )?;

    require_string_equals(manifest, "/programLayout/mode", "verified-msix-extraction")?;
    require_string_equals(manifest, "/programLayout/codexRoot", "apps/Codex")?;
    require_bool_equals(manifest, "/programLayout/usesAppxVolume", false)?;
    require_bool_equals(manifest, "/programLayout/createsWindowsApps", false)?;
    require_bool_equals(manifest, "/programLayout/createsWpSystem", false)?;
    require_bool_equals(manifest, "/programLayout/directShellLaunch", false)?;

    require_string_equals(
        manifest,
        "/dataLayout/codexHome",
        "%LOCALAPPDATA%/CodexAppManager/usb-codex-home",
    )?;
    require_string_equals(manifest, "/dataLayout/roaming", "%APPDATA%/Codex")?;
    require_string_equals(manifest, "/dataLayout/local", "%LOCALAPPDATA%/Codex")?;
    require_bool_equals(manifest, "/dataLayout/onUsb", false)?;
    require_bool_equals(manifest, "/dataLayout/observedBeforeInstall", true)?;
    require_string_equals(
        manifest,
        "/managedContent/skillsDirectory",
        "content/skills",
    )?;
    require_string_equals(manifest, "/managedContent/mcpDirectory", "content/mcp")?;
    require_string_equals(
        manifest,
        "/managedContent/capabilityPackDirectory",
        "content/codex-starter-kit",
    )?;
    require_string_equals(
        manifest,
        "/managedContent/configLocation",
        "%LOCALAPPDATA%/CodexAppManager/usb-codex-home/config.toml",
    )?;
    require_bool_equals(manifest, "/managedContent/secretsIncluded", false)?;
    require_string_equals(
        manifest,
        "/managedContent/status",
        "bundled-auto-installable",
    )?;

    require_string_equals(
        manifest,
        "/startupElevation/mode",
        "usb-launcher-requireAdministrator",
    )?;
    require_string_equals(
        manifest,
        "/startupElevation/mechanism",
        "embedded requireAdministrator manifest + standard Windows UAC consent",
    )?;
    require_string_equals(manifest, "/startupElevation/marker", "--codex-usb-elevated")?;
    require_string_equals(
        manifest,
        "/startupElevation/onCancel",
        "exit-before-install",
    )?;
    require_bool_equals(manifest, "/startupElevation/silentBypass", false)?;
    require_string_equals(
        manifest,
        "/startupElevation/executionPolicy",
        "process-local-bypass",
    )?;
    require_string_equals(
        manifest,
        "/startupElevation/executionPolicyScope",
        "temporary PowerShell child only",
    )?;

    require_string_equals(manifest, "/ccswitch/launchMode", "usb-direct")?;
    require_string_equals(
        manifest,
        "/ccswitch/graphicsMode",
        "child-scoped-webview2-software-rendering",
    )?;
    require_string_equals(
        manifest,
        "/ccswitch/successPostcondition",
        "stable-sizeable-window",
    )?;
    require_string_equals(
        manifest,
        "/ccswitch/foregroundPolicy",
        "restore-clamp-to-work-area-and-present",
    )?;

    require_string_equals(manifest, "/codexLaunch/initialMode", "software-rendering")?;
    require_array_equals(
        manifest,
        "/codexLaunch/initialArguments",
        &["--disable-gpu"],
    )?;
    require_array_equals(
        manifest,
        "/codexLaunch/fallbackArguments",
        &["--disable-gpu", "--no-sandbox"],
    )?;
    require_bool_equals(
        manifest,
        "/codexLaunch/fallbackOnlyAfterNoUsableWindow",
        true,
    )?;
    require_string_equals(
        manifest,
        "/codexLaunch/successPostcondition",
        "stable-sizeable-window",
    )?;
    require_string_equals(
        manifest,
        "/codexLaunch/compatibilityMemory",
        "local-exact-codex-version",
    )?;
    require_string_equals(
        manifest,
        "/codexLaunch/selfUpdate",
        "disabled-for-usb-child",
    )?;
    require_bool_equals(manifest, "/codexLaunch/globalEnvironmentModified", false)?;

    require_string_equals(
        manifest,
        "/usbGuard/localHelperRefresh",
        "length-and-sha256-before-new-lease",
    )?;
    require_u64_equals(manifest, "/sourceMediaCleanup/planSchema", 3)?;
    require_string_equals(
        manifest,
        "/sourceMediaCleanup/runtimeLicenseMode",
        "typed-manager-mutable-metadata",
    )?;
    require_string_equals(
        manifest,
        "/sourceMediaCleanup/authorization",
        "active-lease-fingerprint+volume+immutable-manifest+license-snapshot",
    )?;
    require_string_equals(
        manifest,
        "/sourceMediaCleanup/deleteOrder",
        "payload-first-license-and-retry-entry-last",
    )?;
    require_string_equals(
        manifest,
        "/sourceMediaCleanup/unknownPolicy",
        "enumerate-and-preserve",
    )?;
    require_bool_equals(manifest, "/sourceMediaCleanup/usesRecycleBin", false)?;
    Ok(())
}

fn require_supported_manifest_schema(manifest: &Value, label: &str) -> Result<u64, SignerError> {
    let schema = required_u64(manifest, "/schemaVersion")?;
    if !matches!(schema, 2 | 3) {
        return Err(SignerError::bundle(format!(
            "{label} document schema is unsupported: {schema}"
        )));
    }
    Ok(schema)
}

fn validate_registration_policy(manifest: &Value, schema: u64) -> Result<(), SignerError> {
    if schema == 3 {
        require_bool_equals(manifest, "/revocationPolicy/registrationRequired", true).map_err(
            |_| {
                SignerError::bundle(
                    "schemaVersion=3 root manifest must require revocation registration",
                )
            },
        )?;
    }
    Ok(())
}

fn require_matching_document_schemas(root: &Value, usb: &Value) -> Result<u64, SignerError> {
    let root_schema = require_supported_manifest_schema(root, "root manifest")?;
    let usb_schema = require_supported_manifest_schema(usb, "USB manifest")?;
    if root_schema != usb_schema {
        return Err(SignerError::bundle(
            "root and USB manifest document schemas do not match",
        ));
    }
    Ok(root_schema)
}

fn validate_update_capability(value: &Value, label: &str) -> Result<(), SignerError> {
    if !value.is_object() {
        return Err(SignerError::bundle(format!("{label} must be an object")));
    }
    require_bool_equals(value, "/codexPayload", true)?;
    require_u64_equals(value, "/schemaVersion", 3)?;
    require_u64_equals(value, "/anchorSchema", 3)?;
    require_string_equals(value, "/checksumFile", "SHA256SUMS.txt")?;
    require_string_equals(value, "/transactionDirectory", ".staging")?;
    require_string_equals(value, "/backupPolicy", "single-msix-listed-in-SHA256SUMS")?;
    require_string_equals(value, "/successState", "clean")?;
    Ok(())
}

fn validate_required_layout(root: &Path) -> Result<(), SignerError> {
    for directory in [
        "payload",
        "apps",
        "content",
        "content/skills",
        "content/mcp",
        "content/codex-starter-kit",
        "ccswitch",
        "resources/ccswitch",
        "resources/prerequisites",
    ] {
        required_directory(root, directory)?;
    }
    for file in [
        "CodexDesktopSetup.exe",
        "CodexAppManager.exe",
        "verify-usb-launcher.ps1",
        "ccswitch/CodexSwitch.exe",
        "resources/ccswitch/CodexSwitch.exe",
        "resources/prerequisites/vc_redist.x64.exe",
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
        "apps/README.txt",
        "content/README.txt",
        "content/skills/README.txt",
        "content/mcp/README.txt",
        "content/codex-starter-kit/pack.json",
        "content/codex-starter-kit/dependencies.json",
        "content/codex-starter-kit/file-manifest.json",
        "content/codex-starter-kit/checksums.sha256",
        "content/codex-starter-kit/install-all.ps1",
        "content/codex-starter-kit/install-to-codex.ps1",
        "content/codex-starter-kit/runtime/provision-runtime.ps1",
    ] {
        required_file(root, file)?;
    }
    for forbidden in ["WindowsApps", "WpSystem"] {
        if root.join(forbidden).exists() {
            return Err(SignerError::bundle(format!(
                "bundle must not contain {forbidden}"
            )));
        }
    }
    let pack = read_json(&required_file(root, "content/codex-starter-kit/pack.json")?)?;
    require_u64_equals(&pack, "/schemaVersion", 2)?;
    require_string_equals(&pack, "/id", "codex-starter")?;
    Ok(())
}

fn validate_cross_file_metadata(
    root: &Path,
    usb_manifest: &Value,
    payload: &PayloadInfo,
) -> Result<(), SignerError> {
    let installer = required_file(root, "CodexDesktopSetup.exe")?;
    let manager = required_file(root, "CodexAppManager.exe")?;
    let ccswitch = required_file(root, "ccswitch/CodexSwitch.exe")?;
    let embedded = required_file(root, "resources/ccswitch/CodexSwitch.exe")?;
    let vc = required_file(root, "resources/prerequisites/vc_redist.x64.exe")?;
    let webview = required_file(
        root,
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
    )?;
    let installer_meta = required_value(usb_manifest, "/codexInstaller")?;
    let manager_meta = required_value(usb_manifest, "/manager")?;
    let ccswitch_meta = required_value(usb_manifest, "/ccswitch")?;
    let vc_meta = required_value(usb_manifest, "/vcRuntime")?;
    let webview_meta = required_value(usb_manifest, "/webView2Runtime")?;

    require_string_equals(installer_meta, "/file", "CodexDesktopSetup.exe")?;
    require_string_equals(installer_meta, "/architecture", &payload.architecture)?;
    require_relative_path_equals(installer_meta, "/payload", &payload.relative_path)?;
    require_hash_equals(
        installer_meta,
        "/sha256",
        &file_hash_and_size(&installer)?.0,
    )?;
    require_u64_equals(installer_meta, "/size", file_hash_and_size(&installer)?.1)?;
    require_hash_equals(installer_meta, "/payloadSha256", &payload.sha256)?;
    require_u64_equals(installer_meta, "/payloadSize", payload.size)?;

    require_string_equals(manager_meta, "/file", "CodexAppManager.exe")?;
    require_string_equals(
        manager_meta,
        "/role",
        "requireAdministrator USB Tauri manager started by the launcher or directly",
    )?;
    require_hash_equals(manager_meta, "/sha256", &file_hash_and_size(&manager)?.0)?;
    require_u64_equals(manager_meta, "/size", file_hash_and_size(&manager)?.1)?;

    let ccswitch_hash = file_hash_and_size(&ccswitch)?.0;
    require_string_equals(ccswitch_meta, "/executable", "ccswitch/CodexSwitch.exe")?;
    require_string_equals(
        ccswitch_meta,
        "/embeddedExecutable",
        "resources/ccswitch/CodexSwitch.exe",
    )?;
    require_hash_equals(ccswitch_meta, "/sha256", &ccswitch_hash)?;
    require_u64_equals(ccswitch_meta, "/size", file_hash_and_size(&ccswitch)?.1)?;
    if file_hash_and_size(&embedded)?.0 != ccswitch_hash {
        return Err(SignerError::bundle(
            "root and embedded CCSwitch payload hashes differ",
        ));
    }

    require_string_equals(
        vc_meta,
        "/file",
        "resources/prerequisites/vc_redist.x64.exe",
    )?;
    require_hash_equals(vc_meta, "/sha256", EXPECTED_VC_REDIST_SHA256)?;
    require_u64_equals(vc_meta, "/size", file_hash_and_size(&vc)?.1)?;
    if file_hash_and_size(&vc)?.0 != EXPECTED_VC_REDIST_SHA256 {
        return Err(SignerError::bundle(
            "VC++ runtime hash does not match the pinned release input",
        ));
    }

    require_string_equals(
        webview_meta,
        "/file",
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
    )?;
    require_hash_equals(webview_meta, "/sha256", EXPECTED_WEBVIEW2_SHA256)?;
    require_u64_equals(webview_meta, "/size", file_hash_and_size(&webview)?.1)?;
    require_bool_equals(webview_meta, "/beforeUi", true)?;
    if file_hash_and_size(&webview)?.0 != EXPECTED_WEBVIEW2_SHA256 {
        return Err(SignerError::bundle(
            "WebView2 runtime hash does not match the pinned release input",
        ));
    }
    Ok(())
}

fn verify_msix_payload(
    path: &Path,
    relative_path: &str,
    architecture: &str,
    expected_version: Option<String>,
    sha256: String,
    size: u64,
) -> Result<PayloadInfo, SignerError> {
    let file = File::open(path).map_err(|error| SignerError::io("opening MSIX", path, error))?;
    let mut archive = ZipArchive::new(file).map_err(|error| {
        SignerError::bundle(format!("MSIX is not a readable ZIP archive: {error}"))
    })?;
    let mut manifest_bytes = None;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| SignerError::bundle(format!("reading MSIX entry failed: {error}")))?;
        let entry_name = entry.name().to_string();
        if entry_name == "AppxManifest.xml" {
            if manifest_bytes.is_some() {
                return Err(SignerError::bundle(
                    "MSIX contains duplicate AppxManifest.xml entries",
                ));
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|error| {
                SignerError::bundle(format!("reading AppxManifest.xml failed: {error}"))
            })?;
            manifest_bytes = Some(bytes);
        }
        let archive_name = entry_name.trim_end_matches('/');
        if archive_name.is_empty()
            || entry_name.starts_with('/')
            || archive_name
                .split('/')
                .any(|segment| segment == ".." || segment.is_empty())
        {
            return Err(SignerError::bundle(format!(
                "MSIX contains an unsafe archive entry: {}",
                entry_name
            )));
        }
    }
    let manifest_bytes =
        manifest_bytes.ok_or_else(|| SignerError::bundle("MSIX is missing AppxManifest.xml"))?;
    let (identity, applications) = parse_msix_manifest(&manifest_bytes)?;
    if identity.0 != "OpenAI.Codex" {
        return Err(SignerError::bundle(format!(
            "MSIX Identity is not OpenAI.Codex: {}",
            identity.0
        )));
    }
    if identity.1 != architecture && identity.1 != "neutral" {
        return Err(SignerError::bundle(format!(
            "MSIX architecture differs from manifest: {}",
            identity.1
        )));
    }
    if let Some(expected) = expected_version.as_deref() {
        if identity.2.as_deref() != Some(expected) {
            return Err(SignerError::bundle(format!(
                "MSIX package version differs from manifest: {:?} != {expected}",
                identity.2
            )));
        }
    }
    if applications.is_empty() {
        return Err(SignerError::bundle(
            "MSIX is missing a desktop Application executable",
        ));
    }
    Ok(PayloadInfo {
        relative_path: relative_path.to_string(),
        sha256,
        size,
        architecture: architecture.to_string(),
    })
}

type MsixIdentity = (String, String, Option<String>);

fn parse_msix_manifest(bytes: &[u8]) -> Result<(MsixIdentity, Vec<String>), SignerError> {
    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut identity = None;
    // Current Codex MSIX packages legitimately declare more than one full-trust
    // desktop application (the main UI and the command runner).  Keep every
    // executable for structural validation, while still requiring at least one
    // Application element below.
    let mut applications = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) => {
                let qualified_name = start.name();
                let name = local_name(qualified_name.as_ref());
                if name == b"Identity" {
                    if identity.is_some() {
                        return Err(SignerError::bundle(
                            "MSIX AppxManifest.xml contains multiple Identity elements",
                        ));
                    }
                    let package_name = xml_attribute(&start, b"Name")?;
                    let processor_architecture = xml_attribute(&start, b"ProcessorArchitecture")?;
                    let version = xml_attribute(&start, b"Version").ok();
                    identity = Some((package_name, processor_architecture, version));
                } else if name == b"Application" {
                    applications.push(xml_attribute(&start, b"Executable")?);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(SignerError::bundle(format!(
                    "MSIX AppxManifest.xml XML parse failed: {error}"
                )))
            }
            _ => {}
        }
        buffer.clear();
    }
    let identity = identity.ok_or_else(|| SignerError::bundle("MSIX missing Identity element"))?;
    if applications.is_empty() {
        return Err(SignerError::bundle("MSIX missing Application element"));
    }
    Ok((identity, applications))
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn xml_attribute(start: &BytesStart<'_>, name: &[u8]) -> Result<String, SignerError> {
    for attribute in start.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| {
            SignerError::bundle(format!("MSIX XML attribute parse failed: {error}"))
        })?;
        if local_name(attribute.key.as_ref()) == name {
            let value = attribute.unescape_value().map_err(|error| {
                SignerError::bundle(format!("MSIX XML attribute decode failed: {error}"))
            })?;
            return Ok(value.into_owned());
        }
    }
    Err(SignerError::bundle(format!(
        "MSIX XML element is missing attribute {}",
        String::from_utf8_lossy(name)
    )))
}

fn validate_backup_directory(
    root: &Path,
    payload_relative: &str,
    architecture: &str,
) -> Result<(), SignerError> {
    let parent = payload_relative.rsplit_once('/').map(|(parent, _)| parent);
    let relative = parent
        .map(|value| format!("{value}/_backup"))
        .unwrap_or_else(|| "_backup".to_string());
    let directory = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !directory.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|error| SignerError::io("reading payload backup directory", &directory, error))?;
    if !metadata.is_dir() {
        return Err(SignerError::bundle(
            "payload backup path is not a directory",
        ));
    }
    let entries = fs::read_dir(&directory)
        .map_err(|error| SignerError::io("reading payload backup directory", &directory, error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| SignerError::io("reading payload backup entry", &directory, error))?;
    if entries.len() > 1 {
        return Err(SignerError::bundle(
            "payload backup directory may contain at most one file",
        ));
    }
    if let Some(entry) = entries.first() {
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            SignerError::io("reading payload backup attributes", &entry.path(), error)
        })?;
        if !metadata.is_file() {
            return Err(SignerError::bundle(
                "payload backup directory may contain only one file",
            ));
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let prefix = format!("OpenAI.Codex_{architecture}_");
        if !name.starts_with(&prefix) || !name.to_ascii_lowercase().ends_with(".msix") {
            return Err(SignerError::bundle(format!(
                "payload backup filename is invalid: {name}"
            )));
        }
        let version = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".msix"))
            .unwrap_or_default();
        if version.is_empty()
            || !version.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
        {
            return Err(SignerError::bundle(format!(
                "payload backup filename is invalid: {name}"
            )));
        }
        if metadata.len() == 0 {
            return Err(SignerError::bundle("payload backup must not be empty"));
        }
    }
    Ok(())
}

fn verify_sha256_sums(root: &Path, sums_path: &Path) -> Result<HashMap<String, u64>, SignerError> {
    let text = fs::read_to_string(sums_path)
        .map_err(|error| SignerError::io("reading SHA256SUMS.txt", sums_path, error))?;
    let mut listed = HashMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let split = line
            .find(char::is_whitespace)
            .ok_or_else(|| SignerError::bundle(format!("invalid SHA256SUMS.txt line: {line}")))?;
        let expected = line[..split].trim();
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SignerError::bundle(format!(
                "invalid SHA256SUMS.txt hash: {line}"
            )));
        }
        let relative = normalize_bundle_relative_path(line[split..].trim())?;
        if relative.eq_ignore_ascii_case("sha256sums.txt") {
            return Err(SignerError::bundle("SHA256SUMS.txt must not list itself"));
        }
        if relative.eq_ignore_ascii_case("usb-license.json") {
            return Err(SignerError::bundle(
                "root usb-license.json must remain outside SHA256SUMS.txt",
            ));
        }
        if relative.eq_ignore_ascii_case(REVOCATION_REGISTRATION_FILE_NAME)
            || is_revocation_state_path(&relative)
        {
            return Err(SignerError::bundle(
                "registration and revocation runtime files must remain outside SHA256SUMS.txt",
            ));
        }
        if listed
            .insert(relative.clone(), expected.to_ascii_lowercase())
            .is_some()
        {
            return Err(SignerError::bundle(format!(
                "duplicate SHA256SUMS.txt entry: {relative}"
            )));
        }
    }
    if listed.is_empty() {
        return Err(SignerError::bundle("SHA256SUMS.txt contains no entries"));
    }
    let mut sizes = HashMap::new();
    for (relative, expected) in &listed {
        let file = required_file(root, relative)?;
        let (actual, size) = file_hash_and_size(&file)?;
        if &actual != expected {
            return Err(SignerError::bundle(format!(
                "SHA256SUMS.txt hash mismatch for {relative}"
            )));
        }
        sizes.insert(relative.clone(), size);
    }
    Ok(sizes)
}

fn reject_unlisted_files(root: &Path, listed: &HashMap<String, u64>) -> Result<(), SignerError> {
    let mut pending = vec![root.to_path_buf()];
    let mut seen = HashSet::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| SignerError::io("enumerating bundle files", &directory, error))?
        {
            let entry = entry
                .map_err(|error| SignerError::io("reading bundle file entry", &directory, error))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| SignerError::bundle("bundle entry escaped bundle root"))?
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("usb-license.json") {
                if !relative.eq_ignore_ascii_case("usb-license.json") {
                    return Err(SignerError::bundle(format!(
                        "nested usb-license.json is forbidden: {relative}"
                    )));
                }
                continue;
            }
            if name.eq_ignore_ascii_case(REVOCATION_REGISTRATION_FILE_NAME) {
                if !relative.eq_ignore_ascii_case(REVOCATION_REGISTRATION_FILE_NAME) {
                    return Err(SignerError::bundle(format!(
                        "nested registration declaration is forbidden: {relative}"
                    )));
                }
                let metadata = fs::symlink_metadata(&path).map_err(|error| {
                    SignerError::io("reading registration file attributes", &path, error)
                })?;
                if !metadata.is_file() {
                    return Err(SignerError::bundle(
                        "root registration declaration must be a regular file",
                    ));
                }
                continue;
            }
            if is_revocation_state_path(&relative) {
                let metadata = fs::symlink_metadata(&path).map_err(|error| {
                    SignerError::io("reading revocation state attributes", &path, error)
                })?;
                if !metadata.is_file() {
                    return Err(SignerError::bundle(
                        "revocation runtime metadata must be a regular file",
                    ));
                }
                continue;
            }
            if name.eq_ignore_ascii_case("revocation.json")
                || name
                    .to_ascii_lowercase()
                    .starts_with("revocation.json.tmp-")
            {
                return Err(SignerError::bundle(format!(
                    "revocation state file is outside its exact runtime location: {relative}"
                )));
            }
            if name.eq_ignore_ascii_case("sha256sums.txt") && relative != "sha256sums.txt" {
                return Err(SignerError::bundle(format!(
                    "nested SHA256SUMS.txt is forbidden: {relative}"
                )));
            }
            if has_transaction_residue(&relative) {
                return Err(SignerError::bundle(format!(
                    "transaction or backup residue is forbidden: {relative}"
                )));
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| SignerError::io("reading bundle file attributes", &path, error))?;
            if metadata.is_dir() {
                if relative == "apps/codex"
                    && fs::read_dir(&path)
                        .map_err(|error| {
                            SignerError::io("checking installed Codex tree", &path, error)
                        })?
                        .next()
                        .is_some()
                {
                    return Err(SignerError::bundle(
                        "apps/Codex contains an installed runtime tree; sign only a fresh Bundle before installation",
                    ));
                }
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(SignerError::bundle(format!(
                    "bundle contains a non-regular entry: {relative}"
                )));
            }
            if relative == "sha256sums.txt" {
                continue;
            }
            if !listed.contains_key(&relative) {
                return Err(SignerError::bundle(format!(
                    "file is not listed in SHA256SUMS.txt: {relative}"
                )));
            }
            seen.insert(relative);
        }
    }
    if seen.len() != listed.len() {
        return Err(SignerError::bundle(
            "SHA256SUMS.txt contains entries not present in the bundle",
        ));
    }
    Ok(())
}

fn is_revocation_state_path(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    lower == "codexusbstate/revocation.json" || is_revocation_state_tmp_path(&lower)
}

fn is_revocation_state_tmp_path(relative_lowercase: &str) -> bool {
    let Some(name) = relative_lowercase.strip_prefix("codexusbstate/revocation.json.tmp-") else {
        return false;
    };
    let mut parts = name.split('-');
    let (Some(pid), Some(counter), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    pid.parse::<u32>()
        .is_ok_and(|value| value > 0 && value.to_string() == pid)
        && counter
            .parse::<u64>()
            .is_ok_and(|value| value > 0 && value.to_string() == counter)
}

fn has_transaction_residue(relative: &str) -> bool {
    relative.split('/').any(|segment| {
        let lower = segment.to_ascii_lowercase();
        lower == ".staging"
            || lower == "staging"
            || lower == ".transaction"
            || lower == "transaction"
            || lower.starts_with("transaction.")
            || lower.starts_with("transaction-")
            || lower.ends_with(".bak")
            || lower.ends_with(".part")
            || lower.ends_with(".tmp")
    })
}

fn reject_root_license_directory(root: &Path) -> Result<(), SignerError> {
    let path = root.join("usb-license.json");
    if path.exists()
        && !fs::symlink_metadata(&path)
            .map_err(|error| SignerError::io("reading license attributes", &path, error))?
            .is_file()
    {
        return Err(SignerError::bundle(
            "root usb-license.json must be a regular file",
        ));
    }
    Ok(())
}

fn required_file(root: &Path, relative: &str) -> Result<PathBuf, SignerError> {
    let relative = normalize_bundle_relative_path(relative)?;
    let candidate = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| SignerError::io("reading required bundle file", &candidate, error))?;
    if !metadata.is_file() {
        return Err(SignerError::bundle(format!(
            "required bundle file is not a regular file: {relative}"
        )));
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| SignerError::io("canonicalizing bundle root", root, error))?;
    let canonical_candidate = fs::canonicalize(&candidate).map_err(|error| {
        SignerError::io("canonicalizing required bundle file", &candidate, error)
    })?;
    if !path_is_within(&canonical_root, &canonical_candidate) {
        return Err(SignerError::PathRejected(format!(
            "required file escapes bundle root: {relative}"
        )));
    }
    Ok(candidate)
}

fn required_directory(root: &Path, relative: &str) -> Result<PathBuf, SignerError> {
    let relative = normalize_bundle_relative_path(relative)?;
    let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| SignerError::io("reading required bundle directory", &path, error))?;
    if !metadata.is_dir() {
        return Err(SignerError::bundle(format!(
            "required bundle directory is not a directory: {relative}"
        )));
    }
    Ok(path)
}

fn read_json(path: &Path) -> Result<Value, SignerError> {
    let bytes = fs::read(path).map_err(|error| SignerError::io("reading JSON", path, error))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| SignerError::bundle(format!("JSON is not valid UTF-8: {}", path.display())))?;
    serde_json::from_str(&text).map_err(|error| {
        SignerError::bundle(format!("invalid JSON at {}: {error}", path.display()))
    })
}

fn required_value<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, SignerError> {
    value
        .pointer(pointer)
        .ok_or_else(|| SignerError::bundle(format!("missing JSON field {pointer}")))
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, SignerError> {
    required_value(value, pointer)?
        .as_str()
        .ok_or_else(|| SignerError::bundle(format!("JSON field {pointer} must be a string")))
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, SignerError> {
    required_value(value, pointer)?.as_u64().ok_or_else(|| {
        SignerError::bundle(format!("JSON field {pointer} must be an unsigned integer"))
    })
}

fn required_hash(value: &Value, pointer: &str) -> Result<String, SignerError> {
    let text = required_string(value, pointer)?;
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} is not a SHA-256 hex string"
        )));
    }
    Ok(text.to_ascii_lowercase())
}

fn require_string_equals(value: &Value, pointer: &str, expected: &str) -> Result<(), SignerError> {
    let actual = required_string(value, pointer)?;
    if actual != expected {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} must equal {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn require_relative_path_equals(
    value: &Value,
    pointer: &str,
    expected: &str,
) -> Result<(), SignerError> {
    let actual = normalize_bundle_relative_path(required_string(value, pointer)?)?;
    let expected = normalize_bundle_relative_path(expected)?;
    if actual != expected {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} must equal {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn require_hash_equals(value: &Value, pointer: &str, expected: &str) -> Result<(), SignerError> {
    if required_hash(value, pointer)? != expected.to_ascii_lowercase() {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} hash does not match expected value"
        )));
    }
    Ok(())
}

fn require_u64_equals(value: &Value, pointer: &str, expected: u64) -> Result<(), SignerError> {
    let actual = required_u64(value, pointer)?;
    if actual != expected {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} must equal {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn require_bool_equals(value: &Value, pointer: &str, expected: bool) -> Result<(), SignerError> {
    let actual = required_value(value, pointer)?
        .as_bool()
        .ok_or_else(|| SignerError::bundle(format!("JSON field {pointer} must be a boolean")))?;
    if actual != expected {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} must equal {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn require_array_equals(
    value: &Value,
    pointer: &str,
    expected: &[&str],
) -> Result<(), SignerError> {
    let actual = required_value(value, pointer)?
        .as_array()
        .ok_or_else(|| SignerError::bundle(format!("JSON field {pointer} must be an array")))?;
    if actual.len() != expected.len()
        || actual
            .iter()
            .zip(expected)
            .any(|(actual, expected)| actual.as_str() != Some(*expected))
    {
        return Err(SignerError::bundle(format!(
            "JSON field {pointer} has an unexpected value"
        )));
    }
    Ok(())
}

fn file_hash_and_size(path: &Path) -> Result<(String, u64), SignerError> {
    let mut file =
        File::open(path).map_err(|error| SignerError::io("hashing file", path, error))?;
    let mut hasher = Sha256::new();
    // Keep the large streaming buffer on the heap. The Windows process main
    // thread has a small default stack; a megabyte stack array can overflow
    // before the first read on a real MSIX payload.
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| SignerError::io("hashing file", path, error))?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok((
        digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn unsafe_transaction_and_nested_license_names_are_detected() {
        assert!(has_transaction_residue(
            "payload/_backup/OpenAI.Codex_x64.msix.bak"
        ));
        assert!(has_transaction_residue(".staging/license.tmp"));
        assert!(!has_transaction_residue("payload/OpenAI.Codex_x64.msix"));
    }

    #[test]
    fn checksum_parser_rejects_license_and_duplicate_entries() {
        let root = test_root("checksums");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("file.bin"), b"payload").unwrap();
        fs::write(
            root.join("SHA256SUMS.txt"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  usb-license.json\n",
        )
        .unwrap();
        let result = verify_sha256_sums(&root, &root.join("SHA256SUMS.txt"));
        assert!(result.is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_three_registration_policy_is_mandatory_and_schema_two_stays_compatible() {
        assert!(validate_registration_policy(&json!({}), 2).is_ok());
        assert!(validate_registration_policy(
            &json!({"revocationPolicy":{"registrationRequired":true}}),
            3
        )
        .is_ok());
        assert!(validate_registration_policy(&json!({}), 3).is_err());
        assert!(validate_registration_policy(
            &json!({"revocationPolicy":{"registrationRequired":false}}),
            3
        )
        .is_err());
        assert!(validate_registration_policy(
            &json!({"revocationPolicy":{"registrationRequired":"true"}}),
            3
        )
        .is_err());
    }

    #[test]
    fn manifest_document_schema_mismatch_fails_closed() {
        assert_eq!(
            require_matching_document_schemas(
                &json!({"schemaVersion":3}),
                &json!({"schemaVersion":3})
            )
            .unwrap(),
            3
        );
        assert!(require_matching_document_schemas(
            &json!({"schemaVersion":3}),
            &json!({"schemaVersion":2})
        )
        .is_err());
        assert!(require_matching_document_schemas(
            &json!({"schemaVersion":4}),
            &json!({"schemaVersion":4})
        )
        .is_err());
    }

    #[test]
    fn only_exact_runtime_registration_and_state_files_are_allowed_unlisted() {
        let root = test_root("runtime-files");
        let state = root.join("CodexUsbState");
        fs::create_dir_all(&state).unwrap();
        fs::write(root.join(REVOCATION_REGISTRATION_FILE_NAME), b"envelope").unwrap();
        fs::write(state.join("revocation.json"), b"state").unwrap();
        fs::write(
            state.join("revocation.json.tmp-123-4"),
            b"interrupted write",
        )
        .unwrap();
        assert!(reject_unlisted_files(&root, &HashMap::new()).is_ok());
        fs::remove_dir_all(&root).unwrap();

        let root = test_root("nested-registration");
        let nested = root.join("content");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(REVOCATION_REGISTRATION_FILE_NAME), b"nested").unwrap();
        assert!(reject_unlisted_files(&root, &HashMap::new()).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn revocation_state_temp_path_requires_canonical_decimal_pid_and_counter() {
        assert!(is_revocation_state_path(
            "codexusbstate/revocation.json.tmp-123-4"
        ));
        assert!(!is_revocation_state_path(
            "codexusbstate/revocation.json.tmp-000123-4"
        ));
        assert!(!is_revocation_state_path(
            "codexusbstate/revocation.json.tmp-123-four"
        ));
        assert!(!is_revocation_state_path("other/revocation.json.tmp-123-4"));
    }

    #[test]
    fn path_metadata_comparison_is_case_insensitive() {
        let value = serde_json::json!({
            "payload": "payload/OpenAI.Codex_x64.msix"
        });
        assert!(
            require_relative_path_equals(&value, "/payload", "payload/openai.codex_x64.msix")
                .is_ok()
        );
    }

    #[test]
    fn signer_rejects_post_install_codex_runtime_tree() {
        let root = test_root("post-install-runtime");
        let runtime = root.join("apps").join("Codex");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("ChatGPT.exe"), b"installed-runtime").unwrap();

        let error = reject_unlisted_files(&root, &HashMap::new()).unwrap_err();
        assert!(error.to_string().contains("fresh Bundle"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn formal_signing_binding_requires_policy_key_id_and_raw_public_key_digest() {
        let manifest = json!({
            "usbGuard": {
                "signing": {
                    "policy": "SignedRequired",
                    "keyId": "prod-2026-01",
                    "publicKeySha256": "a".repeat(64)
                }
            }
        });
        let binding = parse_signing_binding(&manifest).unwrap();
        assert_eq!(binding.policy, "SignedRequired");
        assert_eq!(binding.key_id, "prod-2026-01");
        assert!(parse_signing_binding(&json!({
            "usbGuard": { "signing": { "policy": "DevelopmentAutoEnroll" } }
        }))
        .is_err());
        assert!(parse_signing_binding(&json!({
            "usbGuard": { "signing": {
                "policy": "SignedRequired",
                "keyId": "prod-2026-01",
                "publicKeySha256": "not-a-digest"
            } }
        }))
        .is_err());
    }

    #[test]
    fn msix_manifest_accepts_multiple_desktop_applications() {
        let manifest = br#"
            <Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
                     xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10">
                <Identity Name="OpenAI.Codex" ProcessorArchitecture="x64" Version="26.917.8451.0" />
                <Applications>
                    <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication" />
                    <Application Id="CodexCoreCommandRunner" Executable="app/resources/codex-command-runner.exe" EntryPoint="Windows.FullTrustApplication" />
                </Applications>
            </Package>
        "#;

        let (identity, applications) = parse_msix_manifest(manifest).unwrap();
        assert_eq!(identity.0, "OpenAI.Codex");
        assert_eq!(identity.1, "x64");
        assert_eq!(identity.2.as_deref(), Some("26.917.8451.0"));
        assert_eq!(
            applications,
            vec![
                "app/ChatGPT.exe".to_string(),
                "app/resources/codex-command-runner.exe".to_string()
            ]
        );
    }

    #[test]
    fn msix_manifest_still_rejects_multiple_identity_elements() {
        let manifest = br#"
            <Package>
                <Identity Name="OpenAI.Codex" ProcessorArchitecture="x64" Version="1.0.0.0" />
                <Identity Name="OpenAI.Codex" ProcessorArchitecture="x64" Version="1.0.0.0" />
                <Application Id="App" Executable="app/ChatGPT.exe" />
            </Package>
        "#;

        let error = parse_msix_manifest(manifest).unwrap_err();
        assert!(error.to_string().contains("multiple Identity elements"));
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "usb-license-signer-{name}-{}",
            uuid::Uuid::new_v4()
        ))
    }
}
