//! Immutable USB bundle verification.
//!
//! usb-license.json is intentionally not part of SHA256SUMS.txt; it is
//! mutable identity metadata written by the manager or provisioned by the
//! offline signer. Manager-owned runtime directories (`.staging`,
//! `apps/Codex`, and `apps/.codex-app-manager-staging`) are also excluded from
//! static release rows because they are created during an authenticated
//! operation. Every other bundle file must be listed exactly once and match
//! its recorded digest. The shared v2 Anchor additionally binds the canonical
//! non-Payload rows of SHA256SUMS, so replacing the checksum file cannot be
//! paired with replacement of an immutable bundle file. This module owns the
//! release-tree check so the runtime guard and the Codex payload channel use
//! the same boundary.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::errors::AppError;

const ROOT_MANIFEST: &str = "manifest.json";
const USB_MANIFEST: &str = "usb-manifest.json";
const SUMS_FILE: &str = "SHA256SUMS.txt";
const LICENSE_FILE: &str = "usb-license.json";
const LICENSE_BACKUP: &str = "usb-license.json.bak";
const PORTABLE_STAGING_DIR: &str = "apps/.codex-app-manager-staging";
const EXPECTED_WEBVIEW2_SHA256: &str =
    "1f4638309f3d82c31a3028c3cf7d75998f58e4d1407380f5cb8a8e9172caf17d";
const EXPECTED_VC_REDIST_SHA256: &str =
    "843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleIntegrityReport {
    pub root: PathBuf,
    pub anchor_schema: u32,
    pub anchor_sha256: String,
    pub files_verified: usize,
    pub payload_path: PathBuf,
    pub payload_sha256: String,
}

type VerifiedFileLedger = HashMap<String, (String, u64)>;

/// The release tree has two independent trust boundaries.  The USB license
/// and SHA-256 anchor protect product-owned files, while Authenticode is used
/// for trusted external inputs.  Keep these modes explicit so a formal USB
/// license cannot accidentally be treated as a requirement for certificates
/// on every product-owned executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleVerificationMode {
    HashOnly,
    InternalSigned,
    StrictAuthenticode,
}

fn bundle_error(message: impl Into<String>) -> AppError {
    AppError::Engine(format!("USB bundle：{}", message.into()))
}

fn relative_path(raw: &str) -> Result<String, AppError> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(bundle_error(format!("清单路径非法：{raw}")));
    }
    Ok(normalized)
}

fn relative_of(root: &Path, path: &Path) -> Result<String, AppError> {
    path.strip_prefix(root)
        .map_err(|_| bundle_error(format!("路径逃逸 bundle 根目录：{}", path.display())))
        .and_then(|value| relative_path(&value.to_string_lossy()))
}

fn ensure_inside(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    let resolved = fs::canonicalize(path).map_err(|error| {
        bundle_error(format!("无法解析 bundle 文件 {}：{error}", path.display()))
    })?;
    let root_resolved = fs::canonicalize(root).map_err(|error| {
        bundle_error(format!(
            "无法解析 bundle 根目录 {}：{error}",
            root.display()
        ))
    })?;
    if resolved != root_resolved && !resolved.starts_with(&root_resolved) {
        return Err(bundle_error(format!(
            "bundle 文件不得逃逸根目录：{}",
            path.display()
        )));
    }
    Ok(resolved)
}

#[cfg(target_os = "windows")]
fn is_reparse(path: &Path) -> Result<bool, AppError> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .map_err(|error| bundle_error(format!("读取重解析属性失败 {}：{error}", path.display())))
}

#[cfg(not(target_os = "windows"))]
fn is_reparse(path: &Path) -> Result<bool, AppError> {
    Ok(fs::symlink_metadata(path)
        .map_err(|error| bundle_error(format!("读取链接属性失败 {}：{error}", path.display())))?
        .file_type()
        .is_symlink())
}

fn reject_reparse_tree(root: &Path) -> Result<(), AppError> {
    if is_reparse(root)? {
        return Err(bundle_error(format!(
            "bundle 根目录不能是重解析点：{}",
            root.display()
        )));
    }
    let root_license = root.join(LICENSE_FILE);
    match fs::symlink_metadata(&root_license) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(bundle_error(
                "bundle 根目录的 usb-license.json 必须是普通文件",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(bundle_error(format!(
                "读取根目录 usb-license.json 属性失败：{error}"
            )));
        }
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| {
            bundle_error(format!("枚举 bundle 失败 {}：{error}", directory.display()))
        })? {
            let entry =
                entry.map_err(|error| bundle_error(format!("读取 bundle 项失败：{error}")))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| bundle_error(format!("bundle 项逃逸根目录：{}", path.display())))?
                .to_string_lossy()
                .replace('\\', "/");
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(LICENSE_FILE)
                && !relative.eq_ignore_ascii_case(LICENSE_FILE)
            {
                return Err(bundle_error(format!(
                    "usb-license.json 只能位于 bundle 根目录：{relative}"
                )));
            }
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(LICENSE_BACKUP)
            {
                return Err(bundle_error(format!(
                    "发现 usb-license.json.bak 残留：{relative}"
                )));
            }
            if is_reparse(&path)? {
                return Err(bundle_error(format!(
                    "bundle 不得包含符号链接或重解析点：{}",
                    path.display()
                )));
            }
            if entry
                .file_type()
                .map_err(|error| bundle_error(format!("读取 bundle 项类型失败：{error}")))?
                .is_dir()
            {
                stack.push(path);
            }
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path)
        .map_err(|error| bundle_error(format!("打开文件失败 {}：{error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| bundle_error(format!("读取文件失败 {}：{error}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn read_json(root: &Path, name: &str) -> Result<serde_json::Value, AppError> {
    let path = root.join(name);
    let bytes =
        fs::read(&path).map_err(|error| bundle_error(format!("读取 {name} 失败：{error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| bundle_error(format!("解析 {name} 失败：{error}")))
}

fn listed_path(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let relative = relative_path(relative)?;
    let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !path.is_file() {
        return Err(bundle_error(format!("SHA 清单文件不存在：{relative}")));
    }
    ensure_inside(root, &path)
}

fn is_mutable_runtime_path(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    lower == "apps/codex"
        || lower.starts_with("apps/codex/")
        // The portable engine extracts a new MSIX here before the atomic
        // install-root swap. It is manager-owned runtime state, not release
        // payload, so static SHA coverage must not reject its transient files.
        || lower == PORTABLE_STAGING_DIR
        || lower.starts_with("apps/.codex-app-manager-staging/")
        || lower == ".staging"
        || lower.starts_with(".staging/")
}

fn verify_sums(
    root: &Path,
    payload_relative: &str,
) -> Result<(usize, String, VerifiedFileLedger), AppError> {
    let sums_path = root.join(SUMS_FILE);
    let text = fs::read_to_string(&sums_path)
        .map_err(|error| bundle_error(format!("读取 {SUMS_FILE} 失败：{error}")))?;
    let mut listed = HashSet::<String>::new();
    let mut ledger = VerifiedFileLedger::new();
    let mut payload_sha = None;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut parts = line.split_whitespace();
        let expected = parts
            .next()
            .ok_or_else(|| bundle_error(format!("SHA 清单格式错误：{line}")))?;
        let raw_relative = parts
            .next()
            .ok_or_else(|| bundle_error(format!("SHA 清单格式错误：{line}")))?;
        if parts.next().is_some()
            || expected.len() != 64
            || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(bundle_error(format!("SHA 清单格式错误：{line}")));
        }
        let relative = relative_path(raw_relative)?;
        if relative.eq_ignore_ascii_case(SUMS_FILE) || relative.eq_ignore_ascii_case(LICENSE_FILE) {
            return Err(bundle_error(format!("SHA 清单不得包含 {relative}")));
        }
        if !listed.insert(relative.clone()) {
            return Err(bundle_error(format!("SHA 清单存在重复项：{relative}")));
        }
        let path = listed_path(root, &relative)?;
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(bundle_error(format!("SHA 不匹配：{relative}")));
        }
        let size = fs::metadata(&path)
            .map_err(|error| bundle_error(format!("读取文件大小失败 {}：{error}", path.display())))?
            .len();
        ledger.insert(relative.clone(), (actual.clone(), size));
        if relative.eq_ignore_ascii_case(payload_relative) {
            payload_sha = Some(actual);
        }
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| bundle_error(format!("枚举 SHA 范围失败：{error}")))?
        {
            let entry =
                entry.map_err(|error| bundle_error(format!("读取 SHA 范围失败：{error}")))?;
            let path = entry.path();
            let relative = relative_of(root, &path)?;
            let file_type = entry
                .file_type()
                .map_err(|error| bundle_error(format!("读取 SHA 范围类型失败：{error}")))?;
            if relative.eq_ignore_ascii_case("apps/codex") && !file_type.is_dir() {
                return Err(bundle_error("apps/Codex 必须是目录"));
            }
            if relative.eq_ignore_ascii_case(".staging") && !file_type.is_dir() {
                return Err(bundle_error(".staging 必须是目录"));
            }
            if relative.eq_ignore_ascii_case(PORTABLE_STAGING_DIR) && !file_type.is_dir() {
                return Err(bundle_error("apps/.codex-app-manager-staging 必须是目录"));
            }
            if is_mutable_runtime_path(&relative) {
                if file_type.is_dir() {
                    continue;
                }
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if relative.eq_ignore_ascii_case(SUMS_FILE)
                || relative.eq_ignore_ascii_case(LICENSE_FILE)
            {
                continue;
            }
            if !listed.contains(&relative) {
                return Err(bundle_error(format!(
                    "文件未列入 SHA256SUMS.txt：{relative}"
                )));
            }
        }
    }
    Ok((
        listed.len(),
        payload_sha.ok_or_else(|| bundle_error("SHA 清单中没有 MSIX 载荷"))?,
        ledger,
    ))
}

fn verify_manifest_contract(
    root: &Path,
) -> Result<(u32, PathBuf, String, serde_json::Value, serde_json::Value), AppError> {
    let manifest = read_json(root, ROOT_MANIFEST)?;
    let usb_manifest = read_json(root, USB_MANIFEST)?;
    if manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(2)
    {
        return Err(bundle_error("root manifest 必须是 schemaVersion=2"));
    }
    if usb_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(2)
        || usb_manifest
            .get("product")
            .and_then(serde_json::Value::as_str)
            != Some("codex-usb-oneclick")
    {
        return Err(bundle_error(
            "usb-manifest 必须是 codex-usb-oneclick schemaVersion=2",
        ));
    }
    if manifest
        .pointer("/updateCapability/codexPayload")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
        || manifest
            .pointer("/updateCapability/checksumFile")
            .and_then(serde_json::Value::as_str)
            != Some(SUMS_FILE)
    {
        return Err(bundle_error(
            "root manifest 未声明完整 Codex payload 更新能力",
        ));
    }
    if usb_manifest
        .pointer("/updateCapability/codexPayload")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
        || usb_manifest
            .pointer("/updateCapability/checksumFile")
            .and_then(serde_json::Value::as_str)
            != Some(SUMS_FILE)
    {
        return Err(bundle_error(
            "usb-manifest 未声明完整 Codex payload 更新能力",
        ));
    }
    let payload_relative = manifest
        .pointer("/payload/path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bundle_error("root manifest 缺少 payload.path"))?;
    let payload_relative = relative_path(payload_relative)?;
    if !payload_relative.to_ascii_lowercase().ends_with(".msix") {
        return Err(bundle_error("payload.path 必须指向 MSIX"));
    }
    let payload = listed_path(root, &payload_relative)?;
    let anchor_schema = usb_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| bundle_error("usb-manifest schemaVersion 无效"))?
        as u32;
    Ok((
        anchor_schema,
        payload,
        payload_relative,
        manifest,
        usb_manifest,
    ))
}

fn required_value<'a>(
    value: &'a serde_json::Value,
    pointer: &str,
) -> Result<&'a serde_json::Value, AppError> {
    value
        .pointer(pointer)
        .ok_or_else(|| bundle_error(format!("清单缺少字段：{pointer}")))
}

fn required_string<'a>(value: &'a serde_json::Value, pointer: &str) -> Result<&'a str, AppError> {
    required_value(value, pointer)?
        .as_str()
        .ok_or_else(|| bundle_error(format!("清单字段必须是字符串：{pointer}")))
}

fn required_u64(value: &serde_json::Value, pointer: &str) -> Result<u64, AppError> {
    required_value(value, pointer)?
        .as_u64()
        .ok_or_else(|| bundle_error(format!("清单字段必须是无符号整数：{pointer}")))
}

fn required_hash(value: &serde_json::Value, pointer: &str) -> Result<String, AppError> {
    let hash = required_string(value, pointer)?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bundle_error(format!("清单字段不是 SHA-256：{pointer}")));
    }
    Ok(hash.to_ascii_lowercase())
}

fn require_string(
    value: &serde_json::Value,
    pointer: &str,
    expected: &str,
) -> Result<(), AppError> {
    let actual = required_string(value, pointer)?;
    if actual != expected {
        return Err(bundle_error(format!(
            "清单字段 {pointer} 不匹配：expected={expected} actual={actual}"
        )));
    }
    Ok(())
}

fn require_u64(value: &serde_json::Value, pointer: &str, expected: u64) -> Result<(), AppError> {
    let actual = required_u64(value, pointer)?;
    if actual != expected {
        return Err(bundle_error(format!(
            "清单字段 {pointer} 不匹配：expected={expected} actual={actual}"
        )));
    }
    Ok(())
}

fn required_bundle_file(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !path.is_file() {
        return Err(bundle_error(format!("缺少发行文件：{relative}")));
    }
    ensure_inside(root, &path)
}

#[cfg(test)]
fn hash_and_size(path: &Path) -> Result<(String, u64), AppError> {
    let size = fs::metadata(path)
        .map_err(|error| bundle_error(format!("读取文件大小失败 {}：{error}", path.display())))?
        .len();
    Ok((sha256_file(path)?, size))
}

fn verify_file_metadata(
    root: &Path,
    ledger: &VerifiedFileLedger,
    object: &serde_json::Value,
    label: &str,
    file_pointer: &str,
    hash_pointer: &str,
    size_pointer: &str,
    expected_relative: &str,
) -> Result<(String, u64), AppError> {
    require_string(object, file_pointer, expected_relative)?;
    let _path = required_bundle_file(root, expected_relative)?;
    let (actual_hash, actual_size) = ledger
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case(expected_relative))
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bundle_error(format!("SHA 清单缺少联合清单文件：{expected_relative}")))?;
    let expected_hash = required_hash(object, hash_pointer)?;
    let expected_size = required_u64(object, size_pointer)?;
    if !actual_hash.eq_ignore_ascii_case(&expected_hash) || actual_size != expected_size {
        return Err(bundle_error(format!(
            "{label} 文件与清单不一致：path={expected_relative}"
        )));
    }
    Ok((actual_hash, actual_size))
}

/// SHA256SUMS protects the bytes that were copied, while the signed Anchor
/// protects the release metadata and canonical non-Payload checksum rows.
/// Both must agree: otherwise an attacker could rewrite SHA256SUMS together
/// with a replaced launcher or manager without changing the signed USB
/// identity.
fn verify_cross_file_metadata(
    root: &Path,
    manifest: &serde_json::Value,
    usb_manifest: &serde_json::Value,
    _payload_path: &Path,
    payload_relative: &str,
    payload_sha256: &str,
    ledger: &VerifiedFileLedger,
) -> Result<(), AppError> {
    require_u64(manifest, "/schemaVersion", 2)?;
    require_string(manifest, "/packageName", "OpenAI.Codex")?;
    let architecture = required_string(manifest, "/architecture")?;
    if !matches!(architecture, "x64" | "arm64") {
        return Err(bundle_error(format!(
            "manifest architecture 无效：{architecture}"
        )));
    }
    let root_payload = required_value(manifest, "/payload")?;
    require_string(root_payload, "/path", payload_relative)?;
    let _payload_path = required_bundle_file(root, payload_relative)?;
    let (actual_payload_hash, actual_payload_size) = ledger
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case(payload_relative))
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bundle_error("SHA 清单缺少 MSIX 载荷"))?;
    let expected_payload_hash = required_hash(root_payload, "/sha256")?;
    let expected_payload_size = required_u64(root_payload, "/size")?;
    if !actual_payload_hash.eq_ignore_ascii_case(payload_sha256)
        || !actual_payload_hash.eq_ignore_ascii_case(&expected_payload_hash)
        || actual_payload_size != expected_payload_size
    {
        return Err(bundle_error("MSIX 载荷与 root manifest/SHA256SUMS 不一致"));
    }

    require_u64(usb_manifest, "/schemaVersion", 2)?;
    require_string(usb_manifest, "/product", "codex-usb-oneclick")?;
    require_string(usb_manifest, "/status", "oneclick")?;
    let installer = required_value(usb_manifest, "/codexInstaller")?;
    require_string(installer, "/architecture", architecture)?;
    require_string(installer, "/payload", payload_relative)?;
    required_hash(installer, "/payloadSha256").and_then(|hash| {
        if hash.eq_ignore_ascii_case(&actual_payload_hash) {
            Ok(())
        } else {
            Err(bundle_error(
                "codexInstaller.payloadSha256 与实际 MSIX 不一致",
            ))
        }
    })?;
    require_u64(installer, "/payloadSize", actual_payload_size)?;
    let _ = verify_file_metadata(
        root,
        ledger,
        installer,
        "CodexDesktopSetup.exe",
        "/file",
        "/sha256",
        "/size",
        "CodexDesktopSetup.exe",
    )?;

    let _ = verify_file_metadata(
        root,
        ledger,
        required_value(usb_manifest, "/manager")?,
        "CodexAppManager.exe",
        "/file",
        "/sha256",
        "/size",
        "CodexAppManager.exe",
    )?;
    let ccswitch = required_value(usb_manifest, "/ccswitch")?;
    require_string(ccswitch, "/executable", "ccswitch/CodexSwitch.exe")?;
    require_string(
        ccswitch,
        "/embeddedExecutable",
        "resources/ccswitch/CodexSwitch.exe",
    )?;
    let _ccswitch_path = required_bundle_file(root, "ccswitch/CodexSwitch.exe")?;
    let _embedded_path = required_bundle_file(root, "resources/ccswitch/CodexSwitch.exe")?;
    let (ccswitch_hash, ccswitch_size) = ledger
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case("ccswitch/CodexSwitch.exe"))
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bundle_error("SHA 清单缺少 CCSwitch 文件"))?;
    let (embedded_hash, _) = ledger
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case("resources/ccswitch/CodexSwitch.exe"))
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bundle_error("SHA 清单缺少 CCSwitch 内嵌文件"))?;
    if !ccswitch_hash.eq_ignore_ascii_case(&embedded_hash)
        || !ccswitch_hash.eq_ignore_ascii_case(&required_hash(ccswitch, "/sha256")?)
        || ccswitch_size != required_u64(ccswitch, "/size")?
    {
        return Err(bundle_error("CCSwitch 文件与联合清单不一致"));
    }

    let vc = required_value(usb_manifest, "/vcRuntime")?;
    let (vc_hash, _) = verify_file_metadata(
        root,
        ledger,
        vc,
        "VC++ runtime",
        "/file",
        "/sha256",
        "/size",
        "resources/prerequisites/vc_redist.x64.exe",
    )?;
    if !vc_hash.eq_ignore_ascii_case(EXPECTED_VC_REDIST_SHA256) {
        return Err(bundle_error("VC++ runtime hash 不是固定发布输入"));
    }
    let webview = required_value(usb_manifest, "/webView2Runtime")?;
    let (webview_hash, _) = verify_file_metadata(
        root,
        ledger,
        webview,
        "WebView2 runtime",
        "/file",
        "/sha256",
        "/size",
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
    )?;
    if !webview_hash.eq_ignore_ascii_case(EXPECTED_WEBVIEW2_SHA256) {
        return Err(bundle_error("WebView2 runtime hash 不是固定发布输入"));
    }
    Ok(())
}

fn verify_formal_signing_binding(usb_manifest: &serde_json::Value) -> Result<(), AppError> {
    let signing = required_value(usb_manifest, "/usbGuard/signing")?;
    require_string(signing, "/policy", "SignedRequired")?;
    let declared_key_id = required_string(signing, "/keyId")?;
    let active_key_id = option_env!("CODEX_USB_GUARD_PUBLIC_KEY_ID")
        .ok_or_else(|| bundle_error("SignedRequired 构建缺少编译期 USB active keyId"))?;
    if declared_key_id != active_key_id {
        return Err(bundle_error(format!(
            "Bundle signing keyId 与当前管理器 active keyId 不一致：{} != {}",
            declared_key_id, active_key_id
        )));
    }

    let declared_digest = required_string(signing, "/publicKeySha256")?;
    if declared_digest.len() != 64 || !declared_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(bundle_error(
            "Bundle signing publicKeySha256 必须是 64 位十六进制摘要",
        ));
    }
    let public_key_b64 = option_env!("CODEX_USB_GUARD_PUBLIC_KEY_B64")
        .ok_or_else(|| bundle_error("SignedRequired 构建缺少编译期 USB 公钥"))?;
    let public_key = BASE64
        .decode(public_key_b64.trim())
        .map_err(|error| bundle_error(format!("编译期 USB 公钥 Base64 无效：{error}")))?;
    if public_key.len() != 32 {
        return Err(bundle_error("编译期 USB 公钥必须是 32 字节"));
    }
    let digest = Sha256::digest(public_key);
    let actual_digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if !declared_digest.eq_ignore_ascii_case(&actual_digest) {
        return Err(bundle_error(
            "Bundle signing publicKeySha256 与当前管理器公钥不一致",
        ));
    }
    Ok(())
}

fn verify_pe_authenticode(
    root: &Path,
    relative: &str,
    label: &str,
    microsoft: bool,
) -> Result<(), AppError> {
    let path = required_bundle_file(root, relative)?;
    let report = codex_win_engine::verify_authenticode(&path)
        .map_err(|error| bundle_error(format!("{label} Authenticode 读取失败：{error}")))?;
    if !report.is_valid() {
        return Err(bundle_error(format!(
            "{label} Authenticode 无效：status={} subject={}",
            report.status, report.subject
        )));
    }
    if microsoft
        && !report
            .subject
            .to_ascii_lowercase()
            .contains("microsoft corporation")
    {
        return Err(bundle_error(format!(
            "{label} Authenticode 签名者不是 Microsoft Corporation：{}",
            report.subject
        )));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_external_authenticode(root: &Path, payload_path: &Path) -> Result<(), AppError> {
    // These are unmodified third-party inputs.  Their Authenticode identity
    // remains mandatory even when the product-owned USB executables are
    // protected by the internal USB license and bundle anchor.
    verify_pe_authenticode(
        root,
        "resources/prerequisites/vc_redist.x64.exe",
        "VC++ runtime",
        true,
    )?;
    verify_pe_authenticode(
        root,
        "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
        "WebView2 runtime",
        true,
    )?;
    let report = codex_win_engine::verify_openai_authenticode(payload_path)
        .map_err(|error| bundle_error(format!("MSIX Authenticode 读取失败：{error}")))?;
    if !report.is_valid_openai() {
        return Err(bundle_error(format!(
            "MSIX Authenticode 无效：status={} subject={}",
            report.status, report.subject
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn verify_external_authenticode(_root: &Path, _payload_path: &Path) -> Result<(), AppError> {
    Err(bundle_error("正式 Authenticode 校验只能在 Windows 执行"))
}

#[cfg(target_os = "windows")]
fn verify_strict_authenticode(root: &Path, payload_path: &Path) -> Result<(), AppError> {
    verify_pe_authenticode(
        root,
        "CodexDesktopSetup.exe",
        "CodexDesktopSetup.exe",
        false,
    )?;
    verify_pe_authenticode(root, "CodexAppManager.exe", "CodexAppManager.exe", false)?;
    verify_pe_authenticode(root, "ccswitch/CodexSwitch.exe", "CCSwitch", false)?;
    verify_pe_authenticode(
        root,
        "resources/ccswitch/CodexSwitch.exe",
        "embedded CCSwitch",
        false,
    )?;
    verify_external_authenticode(root, payload_path)
}

#[cfg(not(target_os = "windows"))]
fn verify_strict_authenticode(_root: &Path, _payload_path: &Path) -> Result<(), AppError> {
    Err(bundle_error("正式 Authenticode 校验只能在 Windows 执行"))
}

fn verify_bundle_integrity_mode(
    root: &Path,
    mode: BundleVerificationMode,
) -> Result<BundleIntegrityReport, AppError> {
    if !root.is_dir() {
        return Err(bundle_error(format!(
            "bundle 根目录不存在：{}",
            root.display()
        )));
    }
    reject_reparse_tree(root)?;
    let (anchor_schema, payload_path, payload_relative, manifest, usb_manifest) =
        verify_manifest_contract(root)?;
    let (files_verified, payload_sha256, ledger) = verify_sums(root, &payload_relative)?;
    verify_cross_file_metadata(
        root,
        &manifest,
        &usb_manifest,
        &payload_path,
        &payload_relative,
        &payload_sha256,
        &ledger,
    )?;
    match mode {
        BundleVerificationMode::HashOnly => {}
        BundleVerificationMode::InternalSigned => {
            verify_formal_signing_binding(&usb_manifest)?;
            verify_external_authenticode(root, &payload_path)?;
        }
        BundleVerificationMode::StrictAuthenticode => {
            verify_formal_signing_binding(&usb_manifest)?;
            verify_strict_authenticode(root, &payload_path)?;
        }
    }
    let anchor_sha256 = codex_usb_license::anchor_sha256(root)
        .map_err(|error| bundle_error(format!("计算 bundle 锚点失败：{error}")))?;
    Ok(BundleIntegrityReport {
        root: root.to_path_buf(),
        anchor_schema,
        anchor_sha256,
        files_verified,
        payload_path,
        payload_sha256,
    })
}

/// Verify the complete immutable release tree using the legacy boolean API.
/// `true` remains the strict certificate mode for callers that explicitly ask
/// for it; formal USB runtime paths use `verify_signed_internal_bundle` below.
pub fn verify_bundle_integrity(
    root: &Path,
    enforce_authenticode: bool,
) -> Result<BundleIntegrityReport, AppError> {
    verify_bundle_integrity_mode(
        root,
        if enforce_authenticode {
            BundleVerificationMode::StrictAuthenticode
        } else {
            BundleVerificationMode::HashOnly
        },
    )
}

/// Verify a formal USB bundle.  Product-owned executables are covered by the
/// signed USB license, immutable anchor, and SHA-256 rows; only the external
/// MSIX/runtime installers require Windows Authenticode here.
pub fn verify_signed_internal_bundle(root: &Path) -> Result<BundleIntegrityReport, AppError> {
    verify_bundle_integrity_mode(root, BundleVerificationMode::InternalSigned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn relative_paths_reject_escape_and_absolute_forms() {
        assert!(relative_path("payload/a.msix").is_ok());
        assert!(relative_path("../payload/a.msix").is_err());
        assert!(relative_path("/payload/a.msix").is_err());
        assert!(relative_path(r"C:\payload\a.msix").is_err());
    }

    #[test]
    fn portable_staging_is_mutable_but_nearby_unknown_paths_are_not() {
        assert!(is_mutable_runtime_path(
            "apps/.codex-app-manager-staging/portable-123/payload/app.manifest"
        ));
        assert!(is_mutable_runtime_path("APPS/.CODEX-APP-MANAGER-STAGING"));
        assert!(!is_mutable_runtime_path(
            "apps/.codex-app-manager-staging-evil/payload/app.manifest"
        ));
        assert!(!is_mutable_runtime_path(
            "other/.codex-app-manager-staging/file"
        ));
    }

    #[test]
    fn verify_sums_ignores_portable_staging_but_rejects_unowned_unknown_files() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-staging-sums-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        let staging = root
            .join("apps/.codex-app-manager-staging/portable-test/payload/151.0.7922.174.manifest");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::create_dir_all(staging.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload").unwrap();
        fs::write(&staging, b"runtime staging").unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();

        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix").is_ok());

        fs::write(root.join("unowned-runtime.bin"), b"must be listed").unwrap();
        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix").unwrap_err();
        assert!(error.to_string().contains("unowned-runtime.bin"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn portable_staging_path_cannot_be_a_mutable_file() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-staging-file-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload").unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::write(root.join(PORTABLE_STAGING_DIR), b"not a directory").unwrap();

        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix").unwrap_err();
        assert!(error
            .to_string()
            .contains("apps/.codex-app-manager-staging 必须是目录"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_nested_license_and_backup_residue() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-bundle-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested").join(LICENSE_FILE), b"{}").unwrap();
        assert!(verify_bundle_integrity(&root, false).is_err());
        fs::remove_dir_all(&root).unwrap();

        fs::create_dir_all(root.join(LICENSE_FILE)).unwrap();
        assert!(verify_bundle_integrity(&root, false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn contract_requires_schema_two() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-bundle-contract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("payload")).unwrap();
        fs::write(
            root.join(ROOT_MANIFEST),
            serde_json::to_vec(&json!({"schemaVersion": 1})).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(USB_MANIFEST),
            serde_json::to_vec(&json!({"schemaVersion": 2, "product": "codex-usb-oneclick"}))
                .unwrap(),
        )
        .unwrap();
        assert!(verify_bundle_integrity(&root, false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[ignore = "requires a full Windows InternalSigned release bundle"]
    fn internal_signed_mode_accepts_unsigned_product_executables() {
        let bundle = std::env::var_os("CODEX_USB_TEST_BUNDLE")
            .expect("CODEX_USB_TEST_BUNDLE must point to a local release bundle");
        let report = verify_signed_internal_bundle(Path::new(&bundle)).unwrap();
        assert!(report.files_verified > 0);
        assert!(report.payload_sha256.len() == 64);
    }

    #[test]
    fn cross_file_metadata_rejects_payload_hash_metadata_mismatch() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-cross-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload-bytes").unwrap();
        let (actual_hash, actual_size) = hash_and_size(&payload).unwrap();
        let mut ledger = HashMap::new();
        ledger.insert(
            "payload/OpenAI.Codex_x64.msix".to_string(),
            (actual_hash.clone(), actual_size),
        );
        let manifest = json!({
            "schemaVersion": 2,
            "packageName": "OpenAI.Codex",
            "architecture": "x64",
            "payload": {
                "path": "payload/OpenAI.Codex_x64.msix",
                "sha256": "0".repeat(64),
                "size": actual_size
            }
        });
        let result = verify_cross_file_metadata(
            &root,
            &manifest,
            &json!({}),
            &payload,
            "payload/OpenAI.Codex_x64.msix",
            &actual_hash,
            &ledger,
        );
        assert!(result.is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
