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
//! the same boundary. The top-level manifest format remains schema 2; the
//! update capability carries the explicit cryptographic anchor schema so a
//! document-format revision cannot silently change the signing algorithm.

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
const REVOCATION_REGISTRATION_FILE: &str = codex_usb_license::REVOCATION_REGISTRATION_FILE_NAME;
const REVOCATION_STATE_FILE: &str = "CodexUsbState/revocation.json";
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_SHA_SUMS_BYTES: usize = 64 * 1024 * 1024;
const PORTABLE_STAGING_DIR: &str = "apps/.codex-app-manager-staging";
const EXPECTED_WEBVIEW2_SHA256: &str =
    "1f4638309f3d82c31a3028c3cf7d75998f58e4d1407380f5cb8a8e9172caf17d";
const EXPECTED_VC_REDIST_SHA256: &str =
    "843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c";

#[cfg(test)]
thread_local! {
    static TEST_EXPECTED_RUNTIME_HASHES: std::cell::RefCell<HashMap<&'static str, String>> =
        std::cell::RefCell::new(HashMap::new());
}

#[cfg(test)]
fn with_test_runtime_hashes<T>(vc: &str, webview: &str, action: impl FnOnce() -> T) -> T {
    TEST_EXPECTED_RUNTIME_HASHES.with(|hashes| {
        hashes.borrow_mut().insert("vc", vc.to_ascii_lowercase());
        hashes
            .borrow_mut()
            .insert("webview", webview.to_ascii_lowercase());
    });
    let result = action();
    TEST_EXPECTED_RUNTIME_HASHES.with(|hashes| hashes.borrow_mut().clear());
    result
}

fn expected_runtime_hash(kind: &'static str) -> String {
    #[cfg(test)]
    if let Some(hash) =
        TEST_EXPECTED_RUNTIME_HASHES.with(|hashes| hashes.borrow().get(kind).cloned())
    {
        return hash;
    }
    match kind {
        "vc" => EXPECTED_VC_REDIST_SHA256.to_string(),
        "webview" => EXPECTED_WEBVIEW2_SHA256.to_string(),
        _ => unreachable!("unknown fixed runtime kind"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleIntegrityReport {
    pub root: PathBuf,
    pub anchor_schema: u32,
    pub anchor_sha256: String,
    /// True only when the signed root manifest requires Codex to be launched
    /// through the manager's direct-entry shim. Legacy schema 2 bundles keep
    /// their previous launch behavior.
    pub direct_entry_required: bool,
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

/// Check the existing ancestor chain of a bundle-relative path without
/// walking the whole removable drive.  This closes the junction-at-parent
/// case for the cheap capability path while keeping check startup bounded.
fn reject_reparse_ancestors(root: &Path, path: &Path) -> Result<(), AppError> {
    let original_root = root;
    let root = fs::canonicalize(original_root)
        .map_err(|error| bundle_error(format!("解析 bundle 根目录失败：{error}")))?;
    let relative = path
        .strip_prefix(original_root)
        .map_err(|_| bundle_error(format!("bundle 路径不得越过根目录：{}", path.display())))?;
    let mut current = root.join(relative);
    loop {
        match fs::symlink_metadata(&current) {
            Ok(_) if is_reparse(&current)? => {
                return Err(bundle_error(format!(
                    "bundle 路径不得包含重解析点：{}",
                    current.display()
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(bundle_error(format!(
                    "读取 bundle 路径属性失败 {}：{error}",
                    current.display()
                )))
            }
        }
        if current == root {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| bundle_error("bundle 路径缺少根目录祖先"))?
            .to_path_buf();
    }
    Ok(())
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

#[cfg(test)]
fn read_json(root: &Path, name: &str) -> Result<serde_json::Value, AppError> {
    read_json_snapshot(root, name).map(|(value, _)| value)
}

fn read_json_snapshot(root: &Path, name: &str) -> Result<(serde_json::Value, Vec<u8>), AppError> {
    let path = root.join(name);
    let bytes = read_bounded_file(&path, MAX_MANIFEST_BYTES, name)?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| bundle_error(format!("解析 {name} 失败：{error}")))?;
    Ok((value, bytes))
}

fn read_bounded_file(path: &Path, max_bytes: usize, label: &str) -> Result<Vec<u8>, AppError> {
    let mut file = fs::File::open(path)
        .map_err(|error| bundle_error(format!("读取 {label} 失败：{error}")))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| bundle_error(format!("读取 {label} 失败：{error}")))?;
    if bytes.len() > max_bytes {
        return Err(bundle_error(format!("{label} 超过读取上限")));
    }
    Ok(bytes)
}

#[cfg(test)]
fn read_sums_text(root: &Path) -> Result<String, AppError> {
    let bytes = read_bounded_file(&root.join(SUMS_FILE), MAX_SHA_SUMS_BYTES, SUMS_FILE)?;
    String::from_utf8(bytes)
        .map_err(|error| bundle_error(format!("{SUMS_FILE} 不是 UTF-8：{error}")))
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

/// Validate the one backup slot owned by the Codex payload updater.  The
/// checksum verifier intentionally treats backup files as immutable release
/// files, so allowing an arbitrary subtree here would let a path that the
/// stable anchor compresses to the same `MUTABLE backup` marker escape the
/// release contract.
pub fn validate_backup_layout(
    root: &Path,
    payload_relative: &str,
    architecture: &str,
) -> Result<(), AppError> {
    if !matches!(architecture, "x64" | "arm64") {
        return Err(bundle_error(format!(
            "backup 校验收到无效架构：{architecture}"
        )));
    }
    let payload_relative = relative_path(payload_relative)?;
    let payload_parent = payload_relative.rsplit_once('/').map(|(parent, _)| parent);
    let backup_relative = payload_parent
        .map(|parent| format!("{parent}/_backup"))
        .unwrap_or_else(|| "_backup".to_string());
    let backup_directory = root.join(backup_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    reject_reparse_ancestors(root, &backup_directory)?;
    if !backup_directory.exists() {
        return Ok(());
    }
    if is_reparse(&backup_directory)? || !backup_directory.is_dir() {
        return Err(bundle_error(format!(
            "backup 目录不是普通目录：{backup_relative}"
        )));
    }
    let entries = fs::read_dir(&backup_directory)
        .map_err(|error| bundle_error(format!("读取 backup 目录失败：{error}")))?;
    let mut count = 0_u8;
    for entry in entries {
        let entry = entry.map_err(|error| bundle_error(format!("读取 backup 项失败：{error}")))?;
        let path = entry.path();
        if is_reparse(&path)? {
            return Err(bundle_error(format!(
                "backup 目录不得包含重解析项：{}",
                path.display()
            )));
        }
        let file_type = entry
            .file_type()
            .map_err(|error| bundle_error(format!("读取 backup 项类型失败：{error}")))?;
        if !file_type.is_file() {
            return Err(bundle_error(
                "backup 目录只能包含一个直接子文件，不能包含子目录",
            ));
        }
        count = count.saturating_add(1);
        if count > 1 {
            return Err(bundle_error("backup 目录最多只能包含一个直接子文件"));
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let prefix = format!("OpenAI.Codex_{architecture}_");
        let Some(version) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".msix"))
        else {
            return Err(bundle_error(format!("backup 文件名无效：{name}")));
        };
        if version.is_empty()
            || !version.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
        {
            return Err(bundle_error(format!("backup 文件名无效：{name}")));
        }
        let size = fs::metadata(&path)
            .map_err(|error| bundle_error(format!("读取 backup 文件大小失败：{error}")))?
            .len();
        if size == 0 {
            return Err(bundle_error("backup 文件不能为空"));
        }
    }
    Ok(())
}

#[cfg(test)]
fn verify_sums(
    root: &Path,
    payload_relative: &str,
    allow_missing_targets: Option<&HashSet<String>>,
) -> Result<(usize, String, VerifiedFileLedger), AppError> {
    let text = read_sums_text(root)?;
    verify_sums_from_text(root, payload_relative, allow_missing_targets, &text, None)
}

fn verify_sums_from_text(
    root: &Path,
    payload_relative: &str,
    allow_missing_targets: Option<&HashSet<String>>,
    text: &str,
    metadata_snapshot: Option<(&[u8], &[u8])>,
) -> Result<(usize, String, VerifiedFileLedger), AppError> {
    let mut listed = HashSet::<String>::new();
    let mut ledger = VerifiedFileLedger::new();
    let mut payload_sha = None;
    let mut atomic_temp_count = 0usize;
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
        if relative.eq_ignore_ascii_case(SUMS_FILE)
            || relative.eq_ignore_ascii_case(LICENSE_FILE)
            || is_mutable_revocation_file(&relative)
        {
            return Err(bundle_error(format!("SHA 清单不得包含 {relative}")));
        }
        if !listed.insert(relative.clone()) {
            return Err(bundle_error(format!("SHA 清单存在重复项：{relative}")));
        }
        if let Some((label, bytes)) = metadata_snapshot.and_then(|(manifest, usb_manifest)| {
            if relative.eq_ignore_ascii_case(ROOT_MANIFEST) {
                Some((ROOT_MANIFEST, manifest))
            } else if relative.eq_ignore_ascii_case(USB_MANIFEST) {
                Some((USB_MANIFEST, usb_manifest))
            } else {
                None
            }
        }) {
            let digest = Sha256::digest(bytes);
            let snapshot_hash = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if !snapshot_hash.eq_ignore_ascii_case(expected) {
                return Err(bundle_error(format!(
                    "SHA 清单与已解析的 {label} 快照不一致"
                )));
            }
        }
        let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let allowed_missing = allow_missing_targets.is_some_and(|targets| {
            targets
                .iter()
                .any(|target| target.eq_ignore_ascii_case(&relative))
        }) && !path.exists();
        let actual = if allowed_missing {
            expected.to_ascii_lowercase()
        } else {
            let path = listed_path(root, &relative)?;
            sha256_file(&path)?
        };
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(bundle_error(format!("SHA 不匹配：{relative}")));
        }
        let size = if allowed_missing {
            0
        } else {
            fs::metadata(&path)
                .map_err(|error| {
                    bundle_error(format!("读取文件大小失败 {}：{error}", path.display()))
                })?
                .len()
        };
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
            if is_mutable_revocation_sidecar(&relative) {
                validate_mutable_revocation_sidecar(&path)?;
                continue;
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
            if crate::app::usb_revocation::is_atomic_state_tmp_relative(&relative) {
                atomic_temp_count += 1;
                if atomic_temp_count > crate::app::usb_revocation::MAX_ATOMIC_STATE_TEMPS {
                    return Err(bundle_error("授权状态原子临时项超过安全数量上限"));
                }
                validate_atomic_state_temp(&path)?;
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

pub(crate) fn validate_mutable_revocation_sidecar(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| bundle_error(format!("读取授权运行时文件属性失败：{error}")))?;
    if !metadata.is_file() || is_reparse(path)? || !has_exactly_one_link(path) {
        return Err(bundle_error(format!(
            "授权运行时文件必须是单链接普通文件：{}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_atomic_state_temp(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| bundle_error(format!("读取授权状态临时项属性失败：{error}")))?;
    if !metadata.is_file() || is_reparse(path)? || !has_exactly_one_link(path) {
        return Err(bundle_error("授权状态原子临时项必须是单链接普通文件"));
    }
    Ok(())
}

#[cfg(windows)]
fn has_exactly_one_link(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    else {
        return false;
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    (unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) != 0 })
        && info.nNumberOfLinks == 1
        && info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0
}

#[cfg(unix)]
fn has_exactly_one_link(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1
    })
}

#[cfg(not(any(windows, unix)))]
fn has_exactly_one_link(_path: &Path) -> bool {
    false
}

pub(crate) fn is_mutable_revocation_file(relative: &str) -> bool {
    is_mutable_revocation_sidecar(relative)
        || crate::app::usb_revocation::is_atomic_state_tmp_relative(relative)
}

fn is_mutable_revocation_sidecar(relative: &str) -> bool {
    relative.eq_ignore_ascii_case(REVOCATION_REGISTRATION_FILE)
        || relative.eq_ignore_ascii_case(REVOCATION_STATE_FILE)
}

#[allow(clippy::type_complexity)]
fn verify_manifest_contract(
    root: &Path,
    allow_missing_payload: bool,
) -> Result<
    (
        u32,
        bool,
        PathBuf,
        String,
        serde_json::Value,
        serde_json::Value,
        Vec<u8>,
        Vec<u8>,
    ),
    AppError,
> {
    let (manifest, manifest_bytes) = read_json_snapshot(root, ROOT_MANIFEST)?;
    let (usb_manifest, usb_manifest_bytes) = read_json_snapshot(root, USB_MANIFEST)?;
    let root_schema = manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| bundle_error("root manifest schemaVersion 无效"))?;
    let usb_schema = usb_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| bundle_error("usb-manifest schemaVersion 无效"))?;
    if !matches!(root_schema, 2 | 3) {
        return Err(bundle_error("root manifest 必须是 schemaVersion=2 或 3"));
    }
    if !matches!(usb_schema, 2 | 3)
        || usb_schema != root_schema
        || usb_manifest
            .get("product")
            .and_then(serde_json::Value::as_str)
            != Some("codex-usb-oneclick")
    {
        return Err(bundle_error(
            "usb-manifest 必须与 root 同为 codex-usb-oneclick schemaVersion=2 或 3",
        ));
    }
    if root_schema == 3
        && manifest
            .pointer("/revocationPolicy/registrationRequired")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(bundle_error(
            "schemaVersion=3 root manifest 必须锚定 revocationPolicy.registrationRequired=true",
        ));
    }
    let direct_entry_required = direct_entry_policy_required(&manifest, root_schema)?;
    let anchor_schema = declared_anchor_schema(&usb_manifest, usb_schema)?;
    for (label, value) in [
        (
            "root updateCapability.anchorSchema",
            manifest.pointer("/updateCapability/anchorSchema"),
        ),
        (
            "usb updateCapability.anchorSchema",
            usb_manifest.pointer("/updateCapability/anchorSchema"),
        ),
    ] {
        if let Some(value) = value {
            let value = value
                .as_u64()
                .ok_or_else(|| bundle_error(format!("{label} 必须是无符号整数")))?;
            if value != anchor_schema as u64 {
                return Err(bundle_error(format!(
                    "{label} 与 anchorSchema 不一致：{value} != {anchor_schema}"
                )));
            }
        }
    }
    for (label, value) in [
        (
            "root updateCapability.schemaVersion",
            manifest.pointer("/updateCapability/schemaVersion"),
        ),
        (
            "usb updateCapability.schemaVersion",
            usb_manifest.pointer("/updateCapability/schemaVersion"),
        ),
    ] {
        if let Some(value) = value {
            let value = value
                .as_u64()
                .ok_or_else(|| bundle_error(format!("{label} 必须是无符号整数")))?;
            if value != anchor_schema as u64 {
                return Err(bundle_error(format!(
                    "{label} 与 anchorSchema 不一致：{value} != {anchor_schema}"
                )));
            }
        }
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
    let architecture = manifest
        .get("architecture")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bundle_error("root manifest 缺少 architecture"))?;
    validate_backup_layout(root, &payload_relative, architecture)?;
    let payload = root.join(payload_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if payload.exists() {
        let _ = ensure_inside(root, &payload)?;
    } else if !allow_missing_payload {
        return Err(bundle_error(format!(
            "SHA 清单文件不存在：{payload_relative}"
        )));
    } else {
        reject_reparse_ancestors(root, &payload)?;
    }
    Ok((
        anchor_schema,
        direct_entry_required,
        payload,
        payload_relative,
        manifest,
        usb_manifest,
        manifest_bytes,
        usb_manifest_bytes,
    ))
}

fn direct_entry_policy_required(
    manifest: &serde_json::Value,
    root_schema: u64,
) -> Result<bool, AppError> {
    match root_schema {
        2 => Ok(false),
        3 => {
            if manifest
                .pointer("/directEntryPolicy/required")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
                || manifest
                    .pointer("/directEntryPolicy/shim")
                    .and_then(serde_json::Value::as_str)
                    != Some("CodexEntryShim.exe")
                || manifest
                    .pointer("/directEntryPolicy/failure")
                    .and_then(serde_json::Value::as_str)
                    != Some("closed")
            {
                return Err(bundle_error(
                    "schemaVersion=3 root manifest 必须要求 CodexEntryShim.exe 且声明 failure=closed",
                ));
            }
            Ok(true)
        }
        _ => Err(bundle_error(format!(
            "不支持的 direct-entry manifest schema：{root_schema}"
        ))),
    }
}

/// Read the direct-entry policy only when it is bound to the expected signed
/// anchor. Recovery transactions cannot always run the full tree verifier
/// because an interrupted update may leave payload metadata in a transitional
/// state, but they must still reject a policy read from a different manifest
/// snapshot than the signed USB identity.
pub(crate) fn direct_entry_policy_for_signed_anchor(
    root: &Path,
    expected_anchor_sha256: &str,
) -> Result<bool, AppError> {
    let (manifest, manifest_bytes) = read_json_snapshot(root, ROOT_MANIFEST)?;
    let schema = manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| bundle_error("root manifest schemaVersion 无效"))?;
    if !matches!(schema, 2 | 3) {
        return Err(bundle_error("不支持的 root manifest schemaVersion"));
    }
    let direct_entry_required = direct_entry_policy_required(&manifest, schema)?;
    if !direct_entry_required {
        return Ok(false);
    }

    let (_, usb_manifest_bytes) = read_json_snapshot(root, USB_MANIFEST)?;
    let checksum_bytes = read_bounded_file(&root.join(SUMS_FILE), MAX_SHA_SUMS_BYTES, SUMS_FILE)?;
    let anchor_sha256 = codex_usb_license::anchor_sha256_from_bytes(
        &manifest_bytes,
        &usb_manifest_bytes,
        Some(&checksum_bytes),
    )
    .map_err(|error| bundle_error(format!("校验入口策略签名锚点失败：{error}")))?;
    if !anchor_sha256.eq_ignore_ascii_case(expected_anchor_sha256) {
        return Err(bundle_error("入口策略与当前签名许可证锚点不一致"));
    }
    Ok(true)
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

fn declared_anchor_schema(
    manifest: &serde_json::Value,
    top_level_schema: u64,
) -> Result<u32, AppError> {
    let schema = match manifest.pointer("/updateCapability/anchorSchema") {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| bundle_error("updateCapability.anchorSchema 必须是无符号整数"))?,
        None => top_level_schema,
    };
    if schema != codex_usb_license::ANCHOR_SCHEMA_V2 as u64
        && schema != codex_usb_license::ANCHOR_SCHEMA_V3 as u64
    {
        return Err(bundle_error(format!("不支持的更新锚点 schema：{schema}")));
    }
    Ok(schema as u32)
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

#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
fn verify_cross_file_metadata(
    root: &Path,
    manifest: &serde_json::Value,
    usb_manifest: &serde_json::Value,
    _payload_path: &Path,
    payload_relative: &str,
    payload_sha256: &str,
    ledger: &VerifiedFileLedger,
    allow_missing_payload: bool,
) -> Result<(), AppError> {
    let document_schema = required_u64(manifest, "/schemaVersion")?;
    if !matches!(document_schema, 2 | 3) {
        return Err(bundle_error("root manifest schemaVersion 必须是 2 或 3"));
    }
    if document_schema == 3
        && required_value(manifest, "/revocationPolicy")?
            .get("registrationRequired")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(bundle_error(
            "schemaVersion=3 必须锚定 registrationRequired=true",
        ));
    }
    require_string(manifest, "/packageName", "OpenAI.Codex")?;
    let architecture = required_string(manifest, "/architecture")?;
    if !matches!(architecture, "x64" | "arm64") {
        return Err(bundle_error(format!(
            "manifest architecture 无效：{architecture}"
        )));
    }
    let root_payload = required_value(manifest, "/payload")?;
    require_string(root_payload, "/path", payload_relative)?;
    if !allow_missing_payload || root.join(payload_relative).is_file() {
        let _payload_path = required_bundle_file(root, payload_relative)?;
    }
    let (actual_payload_hash, actual_payload_size) = ledger
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case(payload_relative))
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bundle_error("SHA 清单缺少 MSIX 载荷"))?;
    let expected_payload_hash = required_hash(root_payload, "/sha256")?;
    let expected_payload_size = required_u64(root_payload, "/size")?;
    if !actual_payload_hash.eq_ignore_ascii_case(payload_sha256)
        || !actual_payload_hash.eq_ignore_ascii_case(&expected_payload_hash)
        || (actual_payload_size != expected_payload_size
            && !(allow_missing_payload && actual_payload_size == 0))
    {
        return Err(bundle_error("MSIX 载荷与 root manifest/SHA256SUMS 不一致"));
    }

    require_u64(usb_manifest, "/schemaVersion", document_schema)?;
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
    if allow_missing_payload && actual_payload_size == 0 {
        require_u64(installer, "/payloadSize", expected_payload_size)?;
    } else {
        require_u64(installer, "/payloadSize", actual_payload_size)?;
    }
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
    if !vc_hash.eq_ignore_ascii_case(&expected_runtime_hash("vc")) {
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
    if !webview_hash.eq_ignore_ascii_case(&expected_runtime_hash("webview")) {
        return Err(bundle_error("WebView2 runtime hash 不是固定发布输入"));
    }
    Ok(())
}

fn verify_formal_signing_binding(usb_manifest: &serde_json::Value) -> Result<(), AppError> {
    let signing = required_value(usb_manifest, "/usbGuard/signing")?;
    require_string(signing, "/policy", "SignedRequired")?;
    let declared_key_id = required_string(signing, "/keyId")?;
    #[cfg(test)]
    let test_key_config = crate::app::usb_guard::test_license_public_key_config(declared_key_id);
    #[cfg(test)]
    let active_key_id = test_key_config
        .as_ref()
        .map(|(key_id, _)| key_id.as_str())
        .or(option_env!("CODEX_USB_GUARD_PUBLIC_KEY_ID"))
        .ok_or_else(|| bundle_error("SignedRequired 构建缺少编译期 USB active keyId"))?;
    #[cfg(not(test))]
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
    #[cfg(test)]
    let public_key_b64 = test_key_config
        .as_ref()
        .map(|(_, public_key)| public_key.as_str())
        .or(option_env!("CODEX_USB_GUARD_PUBLIC_KEY_B64"))
        .ok_or_else(|| bundle_error("SignedRequired 构建缺少编译期 USB 公钥"))?;
    #[cfg(not(test))]
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
    let (
        anchor_schema,
        direct_entry_required,
        payload_path,
        payload_relative,
        manifest,
        usb_manifest,
        manifest_bytes,
        usb_manifest_bytes,
    ) = verify_manifest_contract(root, false)?;
    let checksum_bytes = read_bounded_file(&root.join(SUMS_FILE), MAX_SHA_SUMS_BYTES, SUMS_FILE)?;
    let checksum_text = String::from_utf8(checksum_bytes.clone())
        .map_err(|error| bundle_error(format!("{SUMS_FILE} 不是 UTF-8：{error}")))?;
    let (files_verified, payload_sha256, ledger) = verify_sums_from_text(
        root,
        &payload_relative,
        None,
        &checksum_text,
        Some((&manifest_bytes, &usb_manifest_bytes)),
    )?;
    verify_cross_file_metadata(
        root,
        &manifest,
        &usb_manifest,
        &payload_path,
        &payload_relative,
        &payload_sha256,
        &ledger,
        false,
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
    let anchor_sha256 = codex_usb_license::anchor_sha256_from_bytes(
        &manifest_bytes,
        &usb_manifest_bytes,
        Some(&checksum_bytes),
    )
    .map_err(|error| bundle_error(format!("计算 bundle 锚点失败：{error}")))?;
    Ok(BundleIntegrityReport {
        root: root.to_path_buf(),
        anchor_schema,
        anchor_sha256,
        direct_entry_required,
        files_verified,
        payload_path,
        payload_sha256,
    })
}

/// Verify a bundle after signed revocation cleanup. Only the manifest-declared
/// MSIX payload and the single exact payload backup row may be missing. Every
/// other immutable row remains fully SHA-256 checked; manager runtime/staging
/// paths remain governed by their existing narrow mutable-path rules.
pub fn verify_revocation_recovery_bundle(
    root: &Path,
    license: &crate::app::usb_guard::UsbLicense,
    fingerprint: &crate::app::usb_guard::UsbFingerprint,
) -> Result<BundleIntegrityReport, AppError> {
    verify_revocation_recovery_bundle_with_observer(
        root,
        license,
        fingerprint,
        crate::app::usb_guard::observe_bundle,
    )
}

fn verify_revocation_recovery_bundle_with_observer(
    root: &Path,
    license: &crate::app::usb_guard::UsbLicense,
    fingerprint: &crate::app::usb_guard::UsbFingerprint,
    observe: impl FnOnce(&Path) -> Result<crate::app::usb_guard::UsbObservation, AppError>,
) -> Result<BundleIntegrityReport, AppError> {
    reject_reparse_tree(root)?;
    if !root.is_dir() {
        return Err(bundle_error("作废恢复时 Bundle 根目录不可用"));
    }
    let observation =
        observe(root).map_err(|error| bundle_error(format!("重新读取 U 盘身份失败：{error}")))?;
    if !observation.removable {
        return Err(bundle_error("作废恢复要求原 U 盘仍处于连接状态"));
    }
    let current_fingerprint = observation
        .fingerprint
        .as_ref()
        .ok_or_else(|| bundle_error("无法从当前 Bundle 根目录读取完整 U 盘指纹"))?;
    validate_recovery_fingerprint_binding(license, fingerprint, current_fingerprint)?;
    match crate::app::usb_guard::validate_license(license, current_fingerprint) {
        Ok(true) => {}
        Ok(false) => {
            return Err(bundle_error(
                "作废恢复需要与当前 U 盘身份匹配的有效签名许可证",
            ));
        }
        Err(error) => {
            return Err(bundle_error(format!("作废恢复许可证校验失败：{error}")));
        }
    }
    validate_recovery_authorization(root, license, fingerprint, current_fingerprint)?;
    let (
        anchor_schema,
        direct_entry_required,
        payload_path,
        payload_relative,
        manifest,
        usb_manifest,
        manifest_bytes,
        usb_manifest_bytes,
    ) = verify_manifest_contract(root, true)?;
    if manifest
        .pointer("/revocationPolicy/registrationRequired")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(bundle_error(
            "作废恢复只允许登记策略必需的 schemaVersion=3 Bundle",
        ));
    }
    let architecture = required_string(&manifest, "/architecture")?;
    let payload_parent = payload_relative
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let backup_directory = if payload_parent.is_empty() {
        "_backup".to_string()
    } else {
        format!("{payload_parent}/_backup")
    };
    let mut allowed_missing = HashSet::new();
    allowed_missing.insert(payload_relative.to_ascii_lowercase());
    let checksum_bytes = read_bounded_file(&root.join(SUMS_FILE), MAX_SHA_SUMS_BYTES, SUMS_FILE)?;
    let sums = String::from_utf8(checksum_bytes.clone())
        .map_err(|error| bundle_error(format!("{SUMS_FILE} 不是 UTF-8：{error}")))?;
    let backup_prefix = format!("{backup_directory}/OpenAI.Codex_{architecture}_");
    let mut backup_rows = 0usize;
    for line in sums.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut parts = line.split_whitespace();
        let _digest = parts.next().unwrap_or_default();
        let Some(path) = parts.next() else { continue };
        if parts.next().is_some() {
            continue;
        }
        let normalized = relative_path(path)?;
        if normalized
            .to_ascii_lowercase()
            .starts_with(&backup_prefix.to_ascii_lowercase())
        {
            let file_name = normalized.rsplit('/').next().unwrap_or_default();
            let Some(version) = file_name
                .strip_prefix(&format!("OpenAI.Codex_{architecture}_"))
                .and_then(|name| name.strip_suffix(".msix"))
            else {
                return Err(bundle_error("payload backup 清单行名称无效"));
            };
            if version.is_empty()
                || !version
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
            {
                return Err(bundle_error("payload backup 清单行版本无效"));
            }
            backup_rows += 1;
            if backup_rows > 1 {
                return Err(bundle_error("作废恢复检测到多条 payload backup 清单行"));
            }
            allowed_missing.insert(normalized.to_ascii_lowercase());
        }
    }
    let (files_verified, payload_sha256, ledger) = verify_sums_from_text(
        root,
        &payload_relative,
        Some(&allowed_missing),
        &sums,
        Some((&manifest_bytes, &usb_manifest_bytes)),
    )?;
    verify_cross_file_metadata(
        root,
        &manifest,
        &usb_manifest,
        &payload_path,
        &payload_relative,
        &payload_sha256,
        &ledger,
        true,
    )?;
    verify_formal_signing_binding(&usb_manifest)?;
    if payload_path.is_file() {
        verify_external_authenticode(root, &payload_path)?;
    }
    let anchor_sha256 = codex_usb_license::anchor_sha256_from_bytes(
        &manifest_bytes,
        &usb_manifest_bytes,
        Some(&checksum_bytes),
    )
    .map_err(|error| bundle_error(format!("计算 Bundle 锚点失败：{error}")))?;
    Ok(BundleIntegrityReport {
        root: root.to_path_buf(),
        anchor_schema,
        anchor_sha256,
        direct_entry_required,
        files_verified,
        payload_path,
        payload_sha256,
    })
}

fn validate_recovery_fingerprint_binding(
    license: &crate::app::usb_guard::UsbLicense,
    supplied: &crate::app::usb_guard::UsbFingerprint,
    observed: &crate::app::usb_guard::UsbFingerprint,
) -> Result<(), AppError> {
    if !codex_usb_license::fingerprints_match(observed, supplied)
        || !codex_usb_license::fingerprints_match(observed, &license.fingerprint)
    {
        return Err(bundle_error(
            "当前 Bundle 根目录的卷序列号、锚点或相对路径与许可证和恢复请求不一致",
        ));
    }
    Ok(())
}

fn validate_recovery_authorization(
    root: &Path,
    license: &crate::app::usb_guard::UsbLicense,
    supplied: &crate::app::usb_guard::UsbFingerprint,
    observed: &crate::app::usb_guard::UsbFingerprint,
) -> Result<crate::app::usb_revocation::RevocationStateFile, AppError> {
    validate_recovery_fingerprint_binding(license, supplied, observed)?;
    let state = crate::app::usb_revocation::read_merged_state_for_recovery(root, license, observed)
        .map_err(|error| bundle_error(format!("作废恢复状态回执无效：{error}")))?;
    if !crate::app::usb_revocation::has_verified_revoked_history(&state, license, observed) {
        return Err(bundle_error(
            "作废恢复缺少与当前许可证和 U 盘身份绑定的已作废签名回执",
        ));
    }
    Ok(state)
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
    use crate::app::usb_guard::{UsbFingerprint, UsbLicense};
    use serde_json::json;

    #[test]
    fn direct_entry_policy_preserves_schema2_and_requires_signed_schema3_values() {
        assert!(!direct_entry_policy_required(&json!({}), 2).unwrap());
        let policy = json!({
            "directEntryPolicy": {
                "required": true,
                "shim": "CodexEntryShim.exe",
                "failure": "closed"
            }
        });
        assert!(direct_entry_policy_required(&policy, 3).unwrap());
        for invalid in [
            json!({}),
            json!({"directEntryPolicy": {"required": false, "shim": "CodexEntryShim.exe", "failure": "closed"}}),
            json!({"directEntryPolicy": {"required": true, "shim": "other.exe", "failure": "closed"}}),
            json!({"directEntryPolicy": {"required": true, "shim": "CodexEntryShim.exe", "failure": "open"}}),
        ] {
            assert!(direct_entry_policy_required(&invalid, 3).is_err());
        }
        assert!(direct_entry_policy_required(&policy, 4).is_err());
    }

    #[test]
    fn direct_entry_policy_and_anchor_stay_bound_to_the_same_manifest_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-entry-policy-snapshot-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let original_manifest = json!({
            "schemaVersion": 3,
            "architecture": "x64",
            "payload": {"path": "payload/current.msix"},
            "updateCapability": {"schemaVersion": 3, "anchorSchema": 3},
            "directEntryPolicy": {
                "required": true,
                "shim": "CodexEntryShim.exe",
                "failure": "closed"
            }
        });
        let usb_manifest = json!({
            "schemaVersion": 3,
            "updateCapability": {"schemaVersion": 3, "anchorSchema": 3}
        });
        let original_manifest_bytes = serde_json::to_vec(&original_manifest).unwrap();
        let usb_manifest_bytes = serde_json::to_vec(&usb_manifest).unwrap();
        let checksum_bytes = format!(
            "{}  manifest.json\n{}  usb-manifest.json\n{}  payload/current.msix\n",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        )
        .into_bytes();
        fs::write(root.join(ROOT_MANIFEST), &original_manifest_bytes).unwrap();
        fs::write(root.join(USB_MANIFEST), &usb_manifest_bytes).unwrap();
        fs::write(root.join(SUMS_FILE), &checksum_bytes).unwrap();

        let (captured_manifest, captured_manifest_bytes) =
            read_json_snapshot(&root, ROOT_MANIFEST).unwrap();
        let (_, captured_usb_manifest_bytes) = read_json_snapshot(&root, USB_MANIFEST).unwrap();
        let requires_shim = direct_entry_policy_required(
            &captured_manifest,
            captured_manifest["schemaVersion"].as_u64().unwrap(),
        )
        .unwrap();
        let signed_snapshot_anchor = codex_usb_license::anchor_sha256_from_bytes(
            &captured_manifest_bytes,
            &captured_usb_manifest_bytes,
            Some(&checksum_bytes),
        )
        .unwrap();
        assert!(direct_entry_policy_for_signed_anchor(&root, &signed_snapshot_anchor).unwrap());

        let changed_manifest = json!({
            "schemaVersion": 3,
            "architecture": "arm64",
            "payload": {"path": "payload/current.msix"},
            "updateCapability": {"schemaVersion": 3, "anchorSchema": 3},
            "directEntryPolicy": {
                "required": true,
                "shim": "CodexEntryShim.exe",
                "failure": "closed"
            }
        });
        fs::write(
            root.join(ROOT_MANIFEST),
            serde_json::to_vec(&changed_manifest).unwrap(),
        )
        .unwrap();
        assert!(direct_entry_policy_for_signed_anchor(&root, &signed_snapshot_anchor).is_err());

        assert!(requires_shim);
        assert_eq!(
            signed_snapshot_anchor,
            codex_usb_license::anchor_sha256_from_bytes(
                &original_manifest_bytes,
                &usb_manifest_bytes,
                Some(&checksum_bytes),
            )
            .unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

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

        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).is_ok());

        fs::write(root.join("unowned-runtime.bin"), b"must be listed").unwrap();
        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
        assert!(error.to_string().contains("unowned-runtime.bin"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn checksum_rows_must_match_captured_manifest_bytes_as_well_as_live_files() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-snapshot-sums-{}", uuid::Uuid::new_v4()));
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        let manifest_bytes = b"captured root manifest";
        let usb_manifest_bytes = b"captured usb manifest";
        fs::write(root.join(ROOT_MANIFEST), manifest_bytes).unwrap();
        fs::write(root.join(USB_MANIFEST), usb_manifest_bytes).unwrap();
        fs::write(&payload, b"payload").unwrap();
        let (manifest_hash, _) = hash_and_size(&root.join(ROOT_MANIFEST)).unwrap();
        let (usb_manifest_hash, _) = hash_and_size(&root.join(USB_MANIFEST)).unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        let sums = format!(
            "{manifest_hash}  {ROOT_MANIFEST}\n{usb_manifest_hash}  {USB_MANIFEST}\n{payload_hash}  payload/OpenAI.Codex_x64.msix\n"
        );

        assert!(verify_sums_from_text(
            &root,
            "payload/OpenAI.Codex_x64.msix",
            None,
            &sums,
            Some((manifest_bytes, usb_manifest_bytes)),
        )
        .is_ok());

        // Keep the captured snapshot and checksum fixed while replacing the
        // live file. The independent live hash check must still reject it.
        fs::write(root.join(ROOT_MANIFEST), b"replacement on disk").unwrap();
        let live_mismatch = verify_sums_from_text(
            &root,
            "payload/OpenAI.Codex_x64.msix",
            None,
            &sums,
            Some((manifest_bytes, usb_manifest_bytes)),
        )
        .unwrap_err();
        assert!(live_mismatch.to_string().contains("SHA 不匹配"));
        fs::write(root.join(ROOT_MANIFEST), manifest_bytes).unwrap();
        let error = verify_sums_from_text(
            &root,
            "payload/OpenAI.Codex_x64.msix",
            None,
            &sums,
            Some((b"different captured root manifest", usb_manifest_bytes)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("快照不一致"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_exact_revocation_sidecars_are_mutable_and_unhashed() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-revocation-sums-{}",
            uuid::Uuid::new_v4()
        ));
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload").unwrap();
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(
            root.join(REVOCATION_REGISTRATION_FILE),
            b"registration sidecar",
        )
        .unwrap();
        fs::write(root.join(REVOCATION_STATE_FILE), b"state sidecar").unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();
        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).is_ok());

        fs::write(root.join("CodexUsbState/revocation-extra.json"), b"unowned").unwrap();
        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
        assert!(error.to_string().contains("revocation-extra.json"));
        fs::remove_file(root.join("CodexUsbState/revocation-extra.json")).unwrap();

        fs::write(
            root.join(SUMS_FILE),
            format!(
                "{payload_hash}  payload/OpenAI.Codex_x64.msix\n{}  {}\n",
                "0".repeat(64),
                REVOCATION_REGISTRATION_FILE
            ),
        )
        .unwrap();
        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
        assert!(error.to_string().contains("SHA 清单不得包含"));
        fs::remove_dir_all(&root).unwrap();
    }

    fn mutable_sidecar_fixture(relative: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-mutable-sidecar-{}",
            uuid::Uuid::new_v4()
        ));
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload").unwrap();
        let sidecar = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = sidecar.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();
        (root, sidecar)
    }

    #[test]
    fn mutable_revocation_sidecars_must_be_regular_files_not_directories() {
        for relative in [REVOCATION_REGISTRATION_FILE, REVOCATION_STATE_FILE] {
            let (root, sidecar) = mutable_sidecar_fixture(relative);
            fs::create_dir(&sidecar).unwrap();

            let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
            assert!(error.to_string().contains("单链接普通文件"), "{error}");

            fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn mutable_revocation_sidecars_reject_hardlinks() {
        for relative in [REVOCATION_REGISTRATION_FILE, REVOCATION_STATE_FILE] {
            let (root, sidecar) = mutable_sidecar_fixture(relative);
            let payload = root.join("payload/OpenAI.Codex_x64.msix");
            fs::hard_link(&payload, &sidecar).unwrap();

            let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
            assert!(error.to_string().contains("单链接普通文件"), "{error}");

            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn exact_atomic_state_temp_is_tolerated_only_when_single_link_and_bounded() {
        let root =
            std::env::temp_dir().join(format!("codex-usb-revocation-tmp-{}", uuid::Uuid::new_v4()));
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload").unwrap();
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        let exact = root.join("CodexUsbState/revocation.json.tmp-321-7");
        fs::write(&exact, b"interrupted atomic write").unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();
        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).is_ok());
        assert!(verify_sums(
            &root,
            "payload/OpenAI.Codex_x64.msix",
            Some(&HashSet::new())
        )
        .is_ok());

        fs::rename(&exact, root.join("CodexUsbState/revocation.json.tmp-321-x")).unwrap();
        assert!(verify_sums(
            &root,
            "payload/OpenAI.Codex_x64.msix",
            Some(&HashSet::new())
        )
        .is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bounded_bundle_metadata_reads_reject_streamed_oversize_files() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-bounded-metadata-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        for (name, limit) in [
            (ROOT_MANIFEST, MAX_MANIFEST_BYTES),
            (USB_MANIFEST, MAX_MANIFEST_BYTES),
            (SUMS_FILE, MAX_SHA_SUMS_BYTES),
        ] {
            let path = root.join(name);
            let file = fs::File::create(&path).unwrap();
            file.set_len(limit as u64 + 1).unwrap();
            drop(file);
            if name == SUMS_FILE {
                assert!(read_sums_text(&root)
                    .unwrap_err()
                    .to_string()
                    .contains("读取上限"));
            } else {
                assert!(read_json(&root, name)
                    .unwrap_err()
                    .to_string()
                    .contains("读取上限"));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strict_bundle_verifier_rejects_hardlinked_or_excess_state_temps() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-bounded-state-temps-{}",
            uuid::Uuid::new_v4()
        ));
        let payload = root.join("payload/OpenAI.Codex_x64.msix");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(&payload, b"payload").unwrap();
        let (payload_hash, _) = hash_and_size(&payload).unwrap();
        fs::write(
            root.join(SUMS_FILE),
            format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
        )
        .unwrap();
        let outside = root.join("outside.txt");
        let linked_temp = root.join("CodexUsbState/revocation.json.tmp-321-7");
        fs::write(&outside, b"keep").unwrap();
        fs::hard_link(&outside, &linked_temp).unwrap();
        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).is_err());
        fs::remove_file(&linked_temp).unwrap();
        fs::remove_file(&outside).unwrap();

        for index in 1..=crate::app::usb_revocation::MAX_ATOMIC_STATE_TEMPS + 1 {
            fs::write(
                root.join(format!("CodexUsbState/revocation.json.tmp-321-{index}")),
                b"interrupted",
            )
            .unwrap();
        }
        assert!(verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None)
            .unwrap_err()
            .to_string()
            .contains("数量上限"));
        fs::remove_dir_all(root).unwrap();
    }

    fn recovery_fixture(name: &str) -> (PathBuf, UsbLicense, UsbFingerprint) {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-recovery-auth-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let fingerprint = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: "a".repeat(64),
            bundle_relative_path: "bundle".to_string(),
            anchor_schema: 3,
        };
        let license = UsbLicense {
            schema_version: codex_usb_license::LICENSE_SCHEMA_VERSION,
            product: codex_usb_license::PRODUCT_ID.to_string(),
            installation_id: uuid::Uuid::new_v4().to_string(),
            fingerprint: fingerprint.clone(),
            issued_at: "1760000000000".to_string(),
            key_id: Some("test-license-key".to_string()),
            account_binding: None,
            signature: Some(BASE64.encode([0_u8; 64])),
        };
        (root, license, fingerprint)
    }

    #[test]
    fn signed_revoked_history_authorizes_only_same_identity_recovery_with_missing_payload() {
        use ed25519_dalek::{Signer, SigningKey};

        let (root, mut license, fingerprint) = recovery_fixture("signed-recovery");
        let license_signing_key = SigningKey::from_bytes(&[17_u8; 32]);
        let license_public_key = BASE64.encode(license_signing_key.verifying_key().as_bytes());
        license.signature = None;
        codex_usb_license::sign_license(&mut license, &license_signing_key).unwrap();
        codex_usb_license::verify_signature_with_key(&license, &license_public_key).unwrap();

        let status_signing_key = SigningKey::from_bytes(&[23_u8; 32]);
        let status_key_id = "test-recovery-status-key";
        let status_public_key = BASE64.encode(status_signing_key.verifying_key().as_bytes());
        let payload = crate::app::usb_revocation::StatusPayload {
            schema_version: 1,
            product: codex_usb_license::PRODUCT_ID.to_string(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            request_id: "ab".repeat(16),
            status: "revoked".to_string(),
            state_version: 7,
            issued_at: "2026-10-09T00:00:00Z".to_string(),
            signing_key_id: status_key_id.to_string(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let envelope = json!({
            "payloadB64": BASE64.encode(&payload_bytes),
            "signatureB64": BASE64.encode(status_signing_key.sign(&payload_bytes).to_bytes()),
        });
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(
            root.join(REVOCATION_STATE_FILE),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "product": codex_usb_license::PRODUCT_ID,
                "installationId": license.installation_id,
                "licenseSignatureSha256": payload.license_signature_sha256,
                "fingerprintSha256": codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
                "receipts": [envelope],
                "cleanup": {
                    "payloadRemoved": false,
                    "runtimeRemoved": false,
                    "stagingRemoved": false,
                    "lastError": null,
                    "updatedAt": null
                }
            }))
            .unwrap(),
        )
        .unwrap();

        crate::app::usb_revocation::with_test_status_public_key(
            status_key_id,
            &status_public_key,
            || {
                crate::app::usb_revocation::verify_status_envelope(
                    &serde_json::from_value(envelope.clone()).unwrap(),
                    &payload.request_id,
                    &license,
                    &codex_usb_license::license_signature_sha256(&license).unwrap(),
                )
                .unwrap();
                let state =
                    validate_recovery_authorization(&root, &license, &fingerprint, &fingerprint)
                        .unwrap();
                assert!(crate::app::usb_revocation::has_verified_revoked_history(
                    &state,
                    &license,
                    &fingerprint
                ));

                let payload_hash = "c".repeat(64);
                fs::write(
                    root.join(SUMS_FILE),
                    format!("{payload_hash}  payload/OpenAI.Codex_x64.msix\n"),
                )
                .unwrap();
                let allowed_missing = HashSet::from(["payload/OpenAI.Codex_x64.msix".to_string()]);
                assert!(verify_sums(
                    &root,
                    "payload/OpenAI.Codex_x64.msix",
                    Some(&allowed_missing)
                )
                .is_ok());

                let different_volume = UsbFingerprint {
                    volume_serial: fingerprint.volume_serial + 1,
                    ..fingerprint.clone()
                };
                assert!(validate_recovery_authorization(
                    &root,
                    &license,
                    &fingerprint,
                    &different_volume
                )
                .is_err());
                let different_root = UsbFingerprint {
                    bundle_relative_path: "another/bundle".to_string(),
                    ..fingerprint.clone()
                };
                assert!(validate_recovery_authorization(
                    &root,
                    &license,
                    &fingerprint,
                    &different_root
                )
                .is_err());
            },
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_recovery_verifier_accepts_signed_revocation_and_only_missing_payload_for_observed_identity(
    ) {
        use ed25519_dalek::{Signer, SigningKey};

        let (root, mut license, mut fingerprint) = recovery_fixture("full-signed-recovery");
        fs::create_dir_all(root.join("payload")).unwrap();
        let files = [
            ("CodexDesktopSetup.exe", b"test installer".as_slice()),
            ("CodexAppManager.exe", b"test manager".as_slice()),
            ("ccswitch/CodexSwitch.exe", b"test switch".as_slice()),
            (
                "resources/ccswitch/CodexSwitch.exe",
                b"test switch".as_slice(),
            ),
            (
                "resources/prerequisites/vc_redist.x64.exe",
                b"test vc runtime".as_slice(),
            ),
            (
                "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
                b"test webview runtime".as_slice(),
            ),
        ];
        let mut file_metadata = HashMap::new();
        for (relative, bytes) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            file_metadata.insert(relative.to_string(), hash_and_size(&path).unwrap());
        }

        let payload_relative = "payload/OpenAI.Codex_x64.msix";
        let payload_hash = "c".repeat(64);
        let payload_size = 123_u64;
        let root_manifest = json!({
            "schemaVersion": 3,
            "packageName": "OpenAI.Codex",
            "architecture": "x64",
            "revocationPolicy": {"registrationRequired": true},
            "directEntryPolicy": {
                "required": true,
                "shim": "CodexEntryShim.exe",
                "failure": "closed"
            },
            "updateCapability": {
                "schemaVersion": 3,
                "anchorSchema": 3,
                "codexPayload": true,
                "checksumFile": SUMS_FILE
            },
            "payload": {
                "path": payload_relative,
                "sha256": payload_hash,
                "size": payload_size
            }
        });
        let (switch_hash, switch_size) = file_metadata.get("ccswitch/CodexSwitch.exe").unwrap();
        let (installer_hash, installer_size) = file_metadata.get("CodexDesktopSetup.exe").unwrap();
        let (manager_hash, manager_size) = file_metadata.get("CodexAppManager.exe").unwrap();
        let (vc_hash, vc_size) = file_metadata
            .get("resources/prerequisites/vc_redist.x64.exe")
            .unwrap();
        let (webview_hash, webview_size) = file_metadata
            .get("resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
            .unwrap();
        let test_license_key = SigningKey::from_bytes(&[17_u8; 32]);
        let license_public_key_digest: String =
            Sha256::digest(test_license_key.verifying_key().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
        let usb_manifest = json!({
            "schemaVersion": 3,
            "product": "codex-usb-oneclick",
            "status": "oneclick",
            "updateCapability": {
                "schemaVersion": 3,
                "anchorSchema": 3,
                "codexPayload": true,
                "checksumFile": SUMS_FILE
            },
            "codexInstaller": {
                "architecture": "x64",
                "payload": payload_relative,
                "payloadSha256": payload_hash,
                "payloadSize": payload_size,
                "file": "CodexDesktopSetup.exe",
                "sha256": installer_hash,
                "size": installer_size
            },
            "manager": {
                "file": "CodexAppManager.exe",
                "sha256": manager_hash,
                "size": manager_size
            },
            "ccswitch": {
                "executable": "ccswitch/CodexSwitch.exe",
                "embeddedExecutable": "resources/ccswitch/CodexSwitch.exe",
                "sha256": switch_hash,
                "size": switch_size
            },
            "vcRuntime": {
                "file": "resources/prerequisites/vc_redist.x64.exe",
                "sha256": vc_hash,
                "size": vc_size
            },
            "webView2Runtime": {
                "file": "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
                "sha256": webview_hash,
                "size": webview_size
            },
            "usbGuard": {
                "signing": {
                    "policy": "SignedRequired",
                    "keyId": "test-license-key",
                    "publicKeySha256": license_public_key_digest
                }
            }
        });
        fs::write(
            root.join(ROOT_MANIFEST),
            serde_json::to_vec(&root_manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(USB_MANIFEST),
            serde_json::to_vec(&usb_manifest).unwrap(),
        )
        .unwrap();

        let mut checksum_rows = Vec::new();
        for (relative, _) in files {
            let (hash, _) = file_metadata.get(relative).unwrap();
            checksum_rows.push(format!("{hash}  {relative}"));
        }
        for relative in [ROOT_MANIFEST, USB_MANIFEST] {
            let (hash, _) = hash_and_size(&root.join(relative)).unwrap();
            checksum_rows.push(format!("{hash}  {relative}"));
        }
        checksum_rows.push(format!("{payload_hash}  {payload_relative}"));
        checksum_rows.sort();
        fs::write(
            root.join(SUMS_FILE),
            format!("{}\n", checksum_rows.join("\n")),
        )
        .unwrap();

        fingerprint.anchor_sha256 = codex_usb_license::anchor_sha256(&root).unwrap();
        license.fingerprint = fingerprint.clone();
        license.key_id = Some("test-license-key".to_string());
        license.signature = None;
        codex_usb_license::sign_license(&mut license, &test_license_key).unwrap();

        let status_signing_key = SigningKey::from_bytes(&[23_u8; 32]);
        let status_key_id = "test-full-recovery-status-key";
        let status_public_key = BASE64.encode(status_signing_key.verifying_key().as_bytes());
        let payload = crate::app::usb_revocation::StatusPayload {
            schema_version: 1,
            product: codex_usb_license::PRODUCT_ID.to_string(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            request_id: "de".repeat(16),
            status: "revoked".to_string(),
            state_version: 9,
            issued_at: "2026-10-09T00:00:00Z".to_string(),
            signing_key_id: status_key_id.to_string(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let envelope = json!({
            "payloadB64": BASE64.encode(&payload_bytes),
            "signatureB64": BASE64.encode(status_signing_key.sign(&payload_bytes).to_bytes()),
        });
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(
            root.join(REVOCATION_STATE_FILE),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "product": codex_usb_license::PRODUCT_ID,
                "installationId": license.installation_id,
                "licenseSignatureSha256": payload.license_signature_sha256,
                "fingerprintSha256": codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
                "receipts": [envelope],
                "cleanup": {
                    "payloadRemoved": false,
                    "runtimeRemoved": false,
                    "stagingRemoved": false,
                    "lastError": null,
                    "updatedAt": null
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let observed = crate::app::usb_guard::UsbObservation {
            removable: true,
            drive_root: Some(root.to_string_lossy().into_owned()),
            bundle_path: root.to_string_lossy().into_owned(),
            fingerprint: Some(fingerprint.clone()),
        };
        let (vc_hash, _) = file_metadata
            .get("resources/prerequisites/vc_redist.x64.exe")
            .unwrap();
        let (webview_hash, _) = file_metadata
            .get("resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
            .unwrap();
        let license_public_key = BASE64.encode(test_license_key.verifying_key().as_bytes());
        let result = crate::app::usb_guard::with_test_license_public_key(
            "test-license-key",
            &license_public_key,
            || {
                crate::app::usb_revocation::with_test_status_public_key(
                    status_key_id,
                    &status_public_key,
                    || {
                        with_test_runtime_hashes(vc_hash, webview_hash, || {
                            verify_revocation_recovery_bundle_with_observer(
                                &root,
                                &license,
                                &fingerprint,
                                |_| Ok(observed.clone()),
                            )
                        })
                    },
                )
            },
        );
        let report = result.unwrap();
        assert!(!report.payload_path.exists());
        assert_eq!(report.payload_sha256, payload_hash);

        let mismatched_observation = crate::app::usb_guard::UsbObservation {
            fingerprint: Some(UsbFingerprint {
                volume_serial: fingerprint.volume_serial + 1,
                ..fingerprint.clone()
            }),
            ..observed
        };
        let error =
            verify_revocation_recovery_bundle_with_observer(&root, &license, &fingerprint, |_| {
                Ok(mismatched_observation)
            })
            .unwrap_err();
        assert!(error.to_string().contains("卷序列号"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_identity_accepts_same_root_and_rejects_other_usb_or_bundle_path() {
        let (root, license, same_root) = recovery_fixture("identity-binding");
        assert!(validate_recovery_fingerprint_binding(&license, &same_root, &same_root).is_ok());

        for different_root in [
            UsbFingerprint {
                volume_serial: same_root.volume_serial + 1,
                ..same_root.clone()
            },
            UsbFingerprint {
                anchor_schema: same_root.anchor_schema - 1,
                ..same_root.clone()
            },
            UsbFingerprint {
                anchor_sha256: "b".repeat(64),
                ..same_root.clone()
            },
            UsbFingerprint {
                bundle_relative_path: "another/bundle".to_string(),
                ..same_root.clone()
            },
        ] {
            assert!(
                validate_recovery_fingerprint_binding(&license, &different_root, &same_root)
                    .is_err()
            );
            assert!(
                validate_recovery_fingerprint_binding(&license, &same_root, &different_root)
                    .is_err()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_requires_current_identity_bound_revoked_receipt_before_missing_payload_exception() {
        let (root, license, fingerprint) = recovery_fixture("none");
        let error = crate::app::usb_revocation::read_merged_state_for_recovery(
            &root,
            &license,
            &fingerprint,
        )
        .unwrap_err();
        assert!(error.contains("没有可验证的授权状态回执"));
        let error = verify_revocation_recovery_bundle(&root, &license, &fingerprint).unwrap_err();
        assert!(error
            .to_string()
            .contains("作废恢复要求原 U 盘仍处于连接状态"));
        fs::remove_dir_all(&root).unwrap();

        let (root, license, fingerprint) = recovery_fixture("invalid");
        let signature_digest = codex_usb_license::license_signature_sha256(&license).unwrap();
        let fingerprint_digest = codex_usb_license::fingerprint_digest(&fingerprint).unwrap();
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(
            root.join(REVOCATION_STATE_FILE),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "product": "codex-usb-oneclick",
                "installationId": license.installation_id,
                "licenseSignatureSha256": signature_digest,
                "fingerprintSha256": fingerprint_digest,
                "receipts": [{"payloadB64": "not-base64", "signatureB64": "invalid"}],
                "cleanup": {}
            }))
            .unwrap(),
        )
        .unwrap();
        let error = crate::app::usb_revocation::read_merged_state_for_recovery(
            &root,
            &license,
            &fingerprint,
        )
        .unwrap_err();
        assert!(error.contains("没有可验证的授权状态回执"));
        fs::remove_dir_all(&root).unwrap();

        let (root, license, fingerprint) = recovery_fixture("identity");
        let signature_digest = codex_usb_license::license_signature_sha256(&license).unwrap();
        let fingerprint_digest = codex_usb_license::fingerprint_digest(&fingerprint).unwrap();
        fs::create_dir_all(root.join("CodexUsbState")).unwrap();
        fs::write(
            root.join(REVOCATION_STATE_FILE),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "product": "codex-usb-oneclick",
                "installationId": "other-installation",
                "licenseSignatureSha256": signature_digest,
                "fingerprintSha256": fingerprint_digest,
                "receipts": [{"payloadB64": "not-base64", "signatureB64": "invalid"}],
                "cleanup": {}
            }))
            .unwrap(),
        )
        .unwrap();
        let error = crate::app::usb_revocation::read_merged_state_for_recovery(
            &root,
            &license,
            &fingerprint,
        )
        .unwrap_err();
        assert!(error.contains("没有可验证的授权状态回执"));
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

        let error = verify_sums(&root, "payload/OpenAI.Codex_x64.msix", None).unwrap_err();
        assert!(error
            .to_string()
            .contains("apps/.codex-app-manager-staging 必须是目录"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn backup_layout_accepts_one_direct_msix_and_rejects_arbitrary_subtrees() {
        let root = std::env::temp_dir().join(format!(
            "codex-usb-backup-layout-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let backup = root.join("payload/_backup");
        fs::create_dir_all(&backup).unwrap();

        fs::write(backup.join("OpenAI.Codex_x64_1.0.0.msix"), b"old").unwrap();
        assert!(validate_backup_layout(&root, "payload/current.msix", "x64").is_ok());

        fs::write(backup.join("extra.msix"), b"extra").unwrap();
        assert!(validate_backup_layout(&root, "payload/current.msix", "x64").is_err());
        fs::remove_file(backup.join("extra.msix")).unwrap();

        fs::remove_file(backup.join("OpenAI.Codex_x64_1.0.0.msix")).unwrap();
        fs::create_dir_all(backup.join("nested")).unwrap();
        assert!(validate_backup_layout(&root, "payload/current.msix", "x64").is_err());
        fs::remove_dir_all(backup.join("nested")).unwrap();

        fs::write(backup.join("evil.bin"), b"not a release backup").unwrap();
        assert!(validate_backup_layout(&root, "payload/current.msix", "x64").is_err());
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
    fn explicit_anchor_schema_is_typed_and_not_silently_downgraded() {
        assert_eq!(
            declared_anchor_schema(&json!({"updateCapability": {"anchorSchema": 3}}), 2,).unwrap(),
            codex_usb_license::ANCHOR_SCHEMA_V3
        );
        assert!(
            declared_anchor_schema(&json!({"updateCapability": {"anchorSchema": "3"}}), 2,)
                .is_err()
        );
        assert!(
            declared_anchor_schema(&json!({"updateCapability": {"anchorSchema": 99}}), 2,).is_err()
        );
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
            false,
        );
        assert!(result.is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
