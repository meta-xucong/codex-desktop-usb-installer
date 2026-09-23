//! Local, offline Codex Desktop payload handling.
//!
//! The first product build deliberately keeps the Codex MSIX as an opaque,
//! signed payload.  This module only discovers the payload beside the setup
//! executable, validates the release manifest and package identity, and stages
//! a byte-for-byte copy in the local temporary directory before the existing
//! Windows engine performs the AppX deployment.

use std::fs::{self, File};
use std::io::Read;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::app::disk::available_space;
use crate::app::release_install::{
    local_package_from_resolved, resolved_local_asset, LocalReleasePackage, ReleaseArchitecture,
};
use crate::errors::AppError;

const MANIFEST_FILE_NAME: &str = "manifest.json";
const TEMP_INSTALLER_DIR: &str = "CodexDesktopInstaller";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Manifest written by `tools/collect-payload.ps1` and shipped at the root of
/// the USB layout.  All fields are intentionally explicit: a user machine
/// must never ask GitHub, Store or a mirror to fill in missing metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfflinePayloadManifest {
    pub product_version: String,
    pub codex_desktop_version: String,
    pub package_name: String,
    pub architecture: String,
    pub min_windows_build: String,
    pub payload: OfflinePayloadSpec,
    #[serde(default)]
    pub package_version: Option<String>,
    #[serde(default)]
    pub release_tag: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfflinePayloadSpec {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// Read-only result displayed by the minimal offline UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflinePayloadReport {
    pub ready: bool,
    pub manifest_path: String,
    pub payload_path: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub hash_verified: bool,
    pub package_name: String,
    pub package_version: String,
    pub codex_desktop_version: String,
    pub architecture: String,
    pub min_windows_build: String,
    pub app_entry: Option<String>,
    pub app_version_verified: bool,
    pub release_tag: String,
    pub authenticode: Option<codex_win_engine::AuthenticodeReport>,
    pub warning: Option<String>,
}

/// Result of copying the USB payload into `%TEMP%`.  The nested report keeps
/// the exact verification evidence next to the path that will be installed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineStageReport {
    pub source_path: String,
    pub staged_path: String,
    pub size: u64,
    pub sha256: String,
    pub payload: OfflinePayloadReport,
}

/// Machine-level cleanup authority derived from one exact, locally present
/// and Authenticode-verified offline MSIX.  Keep the path beside the FullName:
/// a damaged staged record may need the same signed bytes re-staged before the
/// documented AppX removal API can retire it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPackageClaim {
    pub package_full_name: String,
    pub payload_path: PathBuf,
    pub payload_sha256: String,
}

#[derive(Debug, Clone)]
struct OfflineBundle {
    manifest_path: PathBuf,
    manifest: OfflinePayloadManifest,
    payload_path: PathBuf,
}

fn engine_error(error: impl std::fmt::Display) -> AppError {
    AppError::Engine(format!("离线 Codex 载荷校验失败: {error}"))
}

fn normalized_architecture(value: &str) -> Option<ReleaseArchitecture> {
    match value.trim().to_ascii_lowercase().as_str() {
        "x64" | "x86_64" | "amd64" => Some(ReleaseArchitecture::X64),
        "arm64" | "aarch64" => Some(ReleaseArchitecture::Arm64),
        _ => None,
    }
}

fn normalized_architecture_name(value: &str) -> Option<&'static str> {
    match normalized_architecture(value) {
        Some(ReleaseArchitecture::X64) => Some("x64"),
        Some(ReleaseArchitecture::Arm64) => Some("arm64"),
        None => None,
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_non_empty_version(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn parse_build_number(value: &str) -> Result<u32, AppError> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(engine_error("minWindowsBuild 必须是数字"));
    }
    value
        .parse::<u32>()
        .map_err(|_| engine_error("minWindowsBuild 超出范围"))
}

/// Validate a manifest independently of the filesystem.  This is kept pure so
/// malformed USB metadata can be covered by fast unit tests on every host OS.
pub fn validate_manifest(manifest: &OfflinePayloadManifest) -> Result<(), AppError> {
    if !valid_non_empty_version(&manifest.product_version) {
        return Err(engine_error("productVersion 为空或包含非法字符"));
    }
    if !valid_non_empty_version(&manifest.codex_desktop_version) {
        return Err(engine_error("codexDesktopVersion 为空或包含非法字符"));
    }
    if manifest.package_name.trim() != codex_win_engine::OPENAI_PACKAGE_IDENTITY {
        return Err(engine_error(format!(
            "packageName 必须为 {}",
            codex_win_engine::OPENAI_PACKAGE_IDENTITY
        )));
    }
    if normalized_architecture_name(&manifest.architecture).is_none() {
        return Err(engine_error("architecture 只支持 x64 或 arm64"));
    }
    parse_build_number(&manifest.min_windows_build)?;
    if let Some(package_version) = manifest.package_version.as_deref() {
        if !valid_non_empty_version(package_version) {
            return Err(engine_error("packageVersion 为空或包含非法字符"));
        }
    }
    if manifest.payload.path.trim().is_empty() {
        return Err(engine_error("payload.path 不能为空"));
    }
    if !valid_sha256(manifest.payload.sha256.trim()) {
        return Err(engine_error("payload.sha256 必须是 64 位十六进制字符串"));
    }
    if manifest.payload.size == 0
        || manifest.payload.size > codex_win_engine::limits::MAX_PACKAGE_BYTES
    {
        return Err(engine_error(format!(
            "payload.size {} 超出允许范围",
            manifest.payload.size
        )));
    }
    let relative = manifest.payload.path.replace('\\', "/");
    let path = Path::new(&relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(engine_error("payload.path 必须是 bundle 内的相对路径"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir))
    {
        return Err(engine_error("payload.path 不应包含 . 路径段"));
    }
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("msix"))
    {
        return Err(engine_error("第一版离线载荷必须是 .msix 文件"));
    }
    Ok(())
}

pub fn parse_manifest_json(text: &str) -> Result<OfflinePayloadManifest, AppError> {
    let manifest: OfflinePayloadManifest = serde_json::from_str(text)
        .map_err(|error| engine_error(format!("解析 manifest.json 失败: {error}")))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn path_text(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        // `canonicalize()` returns an extended Win32 path on Windows
        // (`\\?\C:\...`).  Keep paths crossing the PowerShell/AppX boundary
        // in the ordinary drive/UNC spelling: several AppX cmdlet and .NET
        // Framework versions parse the extended spelling as a malformed URI.
        if value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
        {
            return format!(r"\\{}", &value[8..]);
        }
        if value
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\"))
            || value
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\??\"))
        {
            return value[4..].to_string();
        }
    }
    value
}

fn path_is_within(root: &Path, candidate: &Path) -> bool {
    if candidate.starts_with(root) {
        return true;
    }
    // Windows path component comparisons are case-insensitive, while Rust's
    // `Path::starts_with` is lexical.  The lowercase fallback preserves the
    // containment check after `canonicalize` on volumes with mixed casing.
    let root_text = path_text(root)
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let candidate_text = path_text(candidate).replace('/', "\\").to_ascii_lowercase();
    candidate_text == root_text || candidate_text.starts_with(&(root_text + "\\"))
}

fn safe_payload_path(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let relative = relative.replace('\\', "/");
    let path = Path::new(&relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(engine_error("payload.path 越过 bundle 根目录"));
    }
    let candidate = root.join(path);
    let candidate_metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| engine_error(format!("读取离线 MSIX 信息失败: {error}")))?;
    if candidate_metadata.file_type().is_symlink() {
        return Err(engine_error("离线 MSIX 不能是符号链接"));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| engine_error(format!("解析 bundle 根目录失败: {error}")))?;
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|error| engine_error(format!("找不到离线 MSIX: {error}")))?;
    if !path_is_within(&canonical_root, &canonical_candidate) {
        return Err(engine_error("离线 MSIX 不在 bundle 根目录内"));
    }
    let metadata = fs::metadata(&canonical_candidate)
        .map_err(|error| engine_error(format!("读取离线 MSIX 信息失败: {error}")))?;
    if !metadata.is_file() {
        return Err(engine_error("离线 MSIX 必须是普通文件"));
    }
    Ok(canonical_candidate)
}

fn manifest_candidates(root: &Path) -> [PathBuf; 3] {
    [
        root.join(MANIFEST_FILE_NAME),
        root.join("payload").join(MANIFEST_FILE_NAME),
        root.join("resources")
            .join("offline")
            .join(MANIFEST_FILE_NAME),
    ]
}

fn discover_bundle(
    executable: &Path,
    package_override: Option<&Path>,
) -> Result<OfflineBundle, AppError> {
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let root = executable
        .parent()
        .ok_or_else(|| engine_error("无法确定安装器所在目录"))?
        .to_path_buf();
    let manifest_path = manifest_candidates(&root)
        .into_iter()
        .find(|candidate| {
            fs::symlink_metadata(candidate)
                .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            engine_error(format!(
                "未找到 {}（期望位于安装器目录或 payload 目录）",
                MANIFEST_FILE_NAME
            ))
        })?;
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|error| engine_error(format!("解析 manifest.json 失败: {error}")))?;
    if !path_is_within(&root, &manifest_path) {
        return Err(engine_error("manifest.json 必须位于安装器目录内"));
    }
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|error| engine_error(format!("读取 manifest.json 失败: {error}")))?;
    let manifest = parse_manifest_json(&manifest_text)?;
    // The executable directory is the trust boundary even when a developer
    // places a diagnostic copy under `payload/` or `resources/offline/`.
    // Keeping this root stable prevents a nested manifest from redefining the
    // bundle and accidentally authorizing files outside the USB directory.
    let bundle_root = root.clone();
    let payload_path = if let Some(override_path) = package_override {
        let candidate = if override_path.is_absolute() {
            override_path.to_path_buf()
        } else {
            bundle_root.join(override_path)
        };
        let candidate_metadata = fs::symlink_metadata(&candidate)
            .map_err(|error| engine_error(format!("读取指定 MSIX 信息失败: {error}")))?;
        if candidate_metadata.file_type().is_symlink() {
            return Err(engine_error("指定 MSIX 不能是符号链接"));
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|error| engine_error(format!("解析指定 MSIX 失败: {error}")))?;
        if !path_is_within(&bundle_root, &canonical) {
            return Err(engine_error("指定 MSIX 必须位于安装器 bundle 内"));
        }
        let metadata = fs::metadata(&canonical)
            .map_err(|error| engine_error(format!("读取指定 MSIX 信息失败: {error}")))?;
        if !metadata.is_file() {
            return Err(engine_error("指定 MSIX 必须是普通文件"));
        }
        canonical
    } else {
        safe_payload_path(&bundle_root, &manifest.payload.path)?
    };
    Ok(OfflineBundle {
        manifest_path,
        manifest,
        payload_path,
    })
}

fn read_app_entry(path: &Path) -> Result<Option<String>, AppError> {
    let file =
        File::open(path).map_err(|error| engine_error(format!("打开 MSIX 失败: {error}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| engine_error(format!("读取 MSIX 压缩结构失败: {error}")))?;
    let mut manifest = archive
        .by_name("AppxManifest.xml")
        .map_err(|error| engine_error(format!("MSIX 缺少 AppxManifest.xml: {error}")))?;
    let mut xml = String::new();
    manifest
        .read_to_string(&mut xml)
        .map_err(|error| engine_error(format!("读取 AppxManifest.xml 失败: {error}")))?;
    Ok(codex_win_engine::parse_appx_application_executable(&xml))
}

fn validate_host_architecture(
    manifest_architecture: &str,
    expected_architecture: Option<&str>,
) -> Result<(), AppError> {
    let Some(expected) = expected_architecture.and_then(normalized_architecture_name) else {
        return Ok(());
    };
    let actual = normalized_architecture_name(manifest_architecture)
        .ok_or_else(|| engine_error("manifest architecture 无效"))?;
    if actual != expected {
        return Err(engine_error(format!(
            "载荷架构 {} 与当前主机架构 {} 不匹配",
            actual, expected
        )));
    }
    Ok(())
}

/// Parse a PowerShell/OS version output into a Windows build number.  Kept
/// public for tests and for future diagnostics; it does not execute anything.
pub fn parse_windows_build_number(output: &str) -> Option<u32> {
    output
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .next_back()
}

#[cfg(windows)]
fn current_windows_build_number() -> Result<u32, AppError> {
    let powershell = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|windir| {
            windir
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::OSVersion.Version.Build",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| engine_error(format!("读取 Windows Build 失败: {error}")))?;
    if !output.status.success() {
        return Err(engine_error("读取 Windows Build 失败"));
    }
    parse_windows_build_number(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| engine_error("无法解析 Windows Build"))
}

fn validate_windows_requirement(min_build: &str) -> Result<Option<u32>, AppError> {
    let minimum = parse_build_number(min_build)?;
    #[cfg(windows)]
    {
        let current = current_windows_build_number()?;
        if current < minimum {
            return Err(engine_error(format!(
                "当前 Windows Build {} 低于最低要求 {}",
                current, minimum
            )));
        }
        Ok(Some(current))
    }
    #[cfg(not(windows))]
    {
        let _ = minimum;
        Ok(None)
    }
}

/// Derive the one exact package FullName that the current USB kit is allowed
/// to treat as its own staged-only cleanup candidate. The Authenticode
/// signature is the trust anchor: `AppxManifest.xml` and the adjacent JSON
/// manifest alone are both writable USB files and must not authorize a
/// machine-wide Remove-AppxPackage operation. The elevated cleanup separately
/// requires the Windows-owned package to have the exact official PFN/FullName,
/// expected target path, no installed users and no provisioned record.
pub fn cleanup_package_claim(executable: &Path) -> Result<CleanupPackageClaim, AppError> {
    // Copy first and verify both the manifest SHA-256 and Authenticode on the
    // local immutable staging slot. Passing the writable USB source path to a
    // later PackageManager call would leave a replacement race between trust
    // verification and orphan rehydration.
    let staged = stage_offline_payload(executable, None, Some("x64"), true)?;
    let payload_path = PathBuf::from(&staged.staged_path);
    let identity = codex_win_engine::read_msix_identity(&payload_path)
        .map_err(|error| engine_error(format!("读取卸载暂存载荷身份失败: {error}")))?;
    codex_win_engine::validate_codex_identity(
        &identity,
        &staged.payload.package_version,
        Some(staged.payload.architecture.as_str()),
    )
    .map_err(|error| engine_error(format!("卸载暂存载荷身份校验失败: {error}")))?;
    let package_full_name = codex_win_engine::canonical_codex_package_moniker(&identity)
        .map_err(|error| engine_error(format!("生成卸载包身份失败: {error}")))?;
    Ok(CleanupPackageClaim {
        package_full_name,
        payload_path,
        payload_sha256: staged.sha256,
    })
}

#[cfg(test)]
fn cleanup_package_claim_with_verifier(
    executable: &Path,
    verify_signature: impl FnOnce(&Path) -> Result<(), AppError>,
) -> Result<CleanupPackageClaim, AppError> {
    let bundle = discover_bundle(executable, None)?;
    validate_manifest(&bundle.manifest)?;
    let metadata = fs::metadata(&bundle.payload_path)
        .map_err(|error| engine_error(format!("读取卸载载荷信息失败: {error}")))?;
    if !metadata.is_file() || metadata.len() != bundle.manifest.payload.size {
        return Err(engine_error(
            "卸载载荷大小与清单不一致；不授权清理 staged 包",
        ));
    }
    let payload_sha256 = codex_win_engine::sha256_file(&bundle.payload_path)
        .map_err(|error| engine_error(format!("计算卸载载荷 SHA-256 失败: {error}")))?;
    if !payload_sha256.eq_ignore_ascii_case(bundle.manifest.payload.sha256.trim()) {
        return Err(engine_error(
            "卸载载荷 SHA-256 与清单不一致；不授权清理 staged 包",
        ));
    }
    verify_signature(&bundle.payload_path)?;
    let identity = codex_win_engine::read_msix_identity(&bundle.payload_path)
        .map_err(|error| engine_error(format!("读取卸载载荷身份失败: {error}")))?;
    if !identity
        .publisher
        .eq_ignore_ascii_case(codex_win_engine::OPENAI_MARKETPLACE_PUBLISHER_SUBJECT)
    {
        return Err(engine_error(format!(
            "卸载载荷发布者不匹配；不授权清理 staged 包: {}",
            identity.publisher
        )));
    }
    if identity
        .resource_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(engine_error(
            "卸载载荷不是 Codex 主包；不授权清理 staged 包",
        ));
    }
    let expected_version = bundle
        .manifest
        .package_version
        .as_deref()
        .unwrap_or(identity.version.as_str());
    codex_win_engine::validate_codex_identity(
        &identity,
        expected_version,
        Some(bundle.manifest.architecture.as_str()),
    )
    .map_err(|error| engine_error(format!("卸载载荷身份校验失败: {error}")))?;
    let package_full_name = codex_win_engine::canonical_codex_package_moniker(&identity)
        .map_err(|error| engine_error(format!("生成卸载包身份失败: {error}")))?;
    Ok(CleanupPackageClaim {
        package_full_name,
        payload_path: bundle.payload_path,
        payload_sha256,
    })
}

/// Verify the bundle's manifest, hash, identity and application metadata.
/// When `enforce_authenticode` is true, also verify the source file's
/// Authenticode signature.  The production USB preflight passes false so a
/// slow removable-volume signature scan cannot trip the short UI probe; the
/// production staging path verifies the local copy instead.
pub fn probe_offline_payload(
    executable: &Path,
    package_override: Option<&Path>,
    expected_architecture: Option<&str>,
    enforce_authenticode: bool,
) -> Result<OfflinePayloadReport, AppError> {
    let bundle = discover_bundle(executable, package_override)?;
    validate_manifest(&bundle.manifest)?;
    validate_host_architecture(&bundle.manifest.architecture, expected_architecture)?;
    let windows_build = validate_windows_requirement(&bundle.manifest.min_windows_build)?;

    let metadata = fs::metadata(&bundle.payload_path)
        .map_err(|error| engine_error(format!("读取离线 MSIX 信息失败: {error}")))?;
    if metadata.len() != bundle.manifest.payload.size {
        return Err(engine_error(format!(
            "MSIX 大小不匹配（实际 {}，清单 {}）",
            metadata.len(),
            bundle.manifest.payload.size
        )));
    }
    let actual_hash = codex_win_engine::sha256_file(&bundle.payload_path)
        .map_err(|error| engine_error(format!("计算 MSIX SHA-256 失败: {error}")))?;
    if !actual_hash.eq_ignore_ascii_case(bundle.manifest.payload.sha256.trim()) {
        return Err(engine_error(format!(
            "MSIX SHA-256 不匹配（实际 {}，清单 {}）",
            actual_hash, bundle.manifest.payload.sha256
        )));
    }

    let identity = codex_win_engine::read_msix_identity(&bundle.payload_path)
        .map_err(|error| engine_error(format!("读取 MSIX 身份失败: {error}")))?;
    let expected_identity_version = bundle
        .manifest
        .package_version
        .as_deref()
        .unwrap_or(identity.version.as_str());
    codex_win_engine::validate_codex_identity(
        &identity,
        expected_identity_version,
        Some(bundle.manifest.architecture.as_str()),
    )
    .map_err(|error| engine_error(format!("MSIX 身份校验失败: {error}")))?;
    let app_entry = read_app_entry(&bundle.payload_path)?;
    if app_entry.is_none() {
        return Err(engine_error("AppxManifest.xml 缺少桌面 Application 入口"));
    }

    let embedded_version = codex_win_engine::read_codex_app_version_from_msix(&bundle.payload_path);
    if let Some(version) = embedded_version.as_deref() {
        if version != bundle.manifest.codex_desktop_version.trim() {
            return Err(engine_error(format!(
                "MSIX 内 Codex 版本 {} 与清单 {} 不一致",
                version, bundle.manifest.codex_desktop_version
            )));
        }
    }

    #[cfg(windows)]
    let authenticode = if enforce_authenticode {
        let report = codex_win_engine::verify_openai_authenticode(&bundle.payload_path)
            .map_err(|error| engine_error(format!("Authenticode 校验失败: {error}")))?;
        if !report.is_valid_openai() {
            return Err(engine_error(format!(
                "Authenticode 校验失败：status={} subject={}",
                report.status, report.subject
            )));
        }
        Some(report)
    } else {
        // The initial USB preflight deliberately avoids scanning the large
        // removable-volume file.  The install-stage path verifies the copied
        // local file before handing it to Add-AppxPackage.
        None
    };
    #[cfg(not(windows))]
    let authenticode = {
        let _ = enforce_authenticode;
        None
    };

    let file_name = bundle
        .payload_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| engine_error("MSIX 文件名无效"))?
        .to_string();
    let architecture = normalized_architecture_name(&bundle.manifest.architecture)
        .ok_or_else(|| engine_error("manifest architecture 无效"))?
        .to_string();
    // The root manifest tag records payload provenance and may be changed by
    // the Codex payload hot-update channel (for example, `upstream-*`). The
    // historical local-installer contract uses its own stable namespace. Do
    // not pass the provenance tag into resolve_windows_local_package(), which
    // intentionally returns `local-signed-*` for a verified local MSIX.
    let release_tag = format!("local-signed-{}", bundle.manifest.codex_desktop_version);
    let warning = if embedded_version.is_none() {
        Some(
            "未读取到 resources/app.asar 的应用版本；安装阶段仍会再次执行完整 MSIX 校验。"
                .to_string(),
        )
    } else if windows_build.is_none() {
        Some("当前不是 Windows 运行环境，未执行 Windows Build 检查。".to_string())
    } else {
        None
    };

    Ok(OfflinePayloadReport {
        ready: true,
        manifest_path: path_text(&bundle.manifest_path),
        payload_path: path_text(&bundle.payload_path),
        file_name,
        size: metadata.len(),
        sha256: actual_hash,
        hash_verified: true,
        package_name: identity.name,
        package_version: identity.version,
        codex_desktop_version: bundle.manifest.codex_desktop_version,
        architecture,
        min_windows_build: bundle.manifest.min_windows_build,
        app_entry,
        app_version_verified: embedded_version.is_some(),
        release_tag,
        authenticode,
        warning,
    })
}

fn stage_component(value: &str) -> String {
    let mut output: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if output.is_empty() {
        output.push_str("unknown");
    }
    output.truncate(80);
    output
}

fn secure_stage_directory(version: &str) -> Result<PathBuf, AppError> {
    let temp_root = std::env::temp_dir().join(TEMP_INSTALLER_DIR);
    fs::create_dir_all(&temp_root)
        .map_err(|error| engine_error(format!("创建临时安装目录失败: {error}")))?;
    let root_metadata = fs::symlink_metadata(&temp_root)
        .map_err(|error| engine_error(format!("读取临时安装目录失败: {error}")))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(engine_error("临时安装目录不能是符号链接或普通文件"));
    }
    let canonical_root = temp_root
        .canonicalize()
        .map_err(|error| engine_error(format!("解析临时安装目录失败: {error}")))?;
    let requested = temp_root.join(stage_component(version));
    fs::create_dir_all(&requested)
        .map_err(|error| engine_error(format!("创建版本暂存目录失败: {error}")))?;
    let metadata = fs::symlink_metadata(&requested)
        .map_err(|error| engine_error(format!("读取版本暂存目录失败: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(engine_error("版本暂存目录不能是符号链接或普通文件"));
    }
    let canonical = requested
        .canonicalize()
        .map_err(|error| engine_error(format!("解析版本暂存目录失败: {error}")))?;
    if !path_is_within(&canonical_root, &canonical) || canonical == canonical_root {
        return Err(engine_error("版本暂存目录不在临时安装目录内"));
    }
    Ok(canonical)
}

/// Verify an MSIX after it has been copied to the local temporary directory.
///
/// The source lives on a removable volume and a PowerShell Authenticode probe
/// reads the complete package.  Keeping this operation after the copy avoids
/// combining slow USB I/O, Defender scanning and certificate-chain work under
/// the short UI preflight deadline.  The hash is checked before this function
/// is called, so the verified local bytes still correspond to the manifest.
fn verify_staged_authenticode(
    path: &Path,
    enforce_authenticode: bool,
) -> Result<Option<codex_win_engine::AuthenticodeReport>, AppError> {
    #[cfg(windows)]
    {
        if !enforce_authenticode {
            return Ok(None);
        }
        let report = codex_win_engine::verify_openai_authenticode(path)
            .map_err(|error| engine_error(format!("Authenticode 校验失败: {error}")))?;
        if !report.is_valid_openai() {
            return Err(engine_error(format!(
                "Authenticode 校验失败：status={} subject={}",
                report.status, report.subject
            )));
        }
        Ok(Some(report))
    }
    #[cfg(not(windows))]
    {
        let _ = (path, enforce_authenticode);
        Ok(None)
    }
}

pub fn stage_offline_payload(
    executable: &Path,
    package_override: Option<&Path>,
    expected_architecture: Option<&str>,
    enforce_authenticode: bool,
) -> Result<OfflineStageReport, AppError> {
    // Do not verify Authenticode on the removable source.  The full package is
    // copied and hash-checked first; the trust gate runs on the local copy
    // below with the dedicated long Authenticode deadline.
    let mut payload =
        probe_offline_payload(executable, package_override, expected_architecture, false)?;
    let stage_root = secure_stage_directory(&payload.codex_desktop_version)?;
    let destination = copy_or_reuse_staged_payload(&payload, &stage_root, available_space)?;
    let staged_hash = codex_win_engine::sha256_file(&destination)
        .map_err(|error| engine_error(format!("再次计算暂存 MSIX SHA-256 失败: {error}")))?;
    if !staged_hash.eq_ignore_ascii_case(&payload.sha256) {
        return Err(engine_error("暂存 MSIX 二次校验失败，拒绝安装"));
    }
    payload.authenticode = verify_staged_authenticode(&destination, enforce_authenticode)?;
    Ok(OfflineStageReport {
        source_path: payload.payload_path.clone(),
        staged_path: path_text(&destination),
        size: payload.size,
        sha256: staged_hash,
        payload,
    })
}

fn copy_or_reuse_staged_payload(
    payload: &OfflinePayloadReport,
    stage_root: &Path,
    free_space: impl Fn(&Path) -> Result<Option<u64>, AppError>,
) -> Result<PathBuf, AppError> {
    let mut destination = stage_root.join(&payload.file_name);
    let destination_parent = destination
        .parent()
        .ok_or_else(|| engine_error("暂存目标路径无效"))?
        .canonicalize()
        .map_err(|error| engine_error(format!("解析暂存目标目录失败: {error}")))?;
    if destination_parent != stage_root {
        return Err(engine_error("暂存目标目录不在受保护的版本目录内"));
    }
    let source_canonical = PathBuf::from(&payload.payload_path)
        .canonicalize()
        .map_err(|error| engine_error(format!("解析源 MSIX 失败: {error}")))?;
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(engine_error("暂存目标不能是符号链接"));
        }
        Ok(metadata) => {
            // A retry may find the exact verified bytes still held open by
            // Windows. Reuse them without requesting write/delete access or
            // demanding enough free space for a redundant second copy.
            if metadata.is_file()
                && metadata.len() == payload.size
                && codex_win_engine::sha256_file(&destination)
                    .is_ok_and(|hash| hash.eq_ignore_ascii_case(&payload.sha256))
            {
                return Ok(destination);
            }
            // Preserve an invalid/locked/readonly earlier attempt for diagnosis.
            // Never delete it or inherit its attributes into the new attempt.
            if !valid_sha256(&payload.sha256) {
                return Err(engine_error("重试缓存 SHA-256 无效"));
            }
            let base = stage_root
                .parent()
                .ok_or_else(|| engine_error("暂存根目录无效"))?;
            // A stable hash slot lets the next attempt reuse this fallback.
            // Otherwise a permanently locked legacy slot creates another
            // 800 MB copy on every deployment retry until the disk fills.
            let cache_root = base.join(format!("cache-{}", payload.sha256.to_ascii_lowercase()));
            match fs::create_dir(&cache_root) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let meta = fs::symlink_metadata(&cache_root)
                        .map_err(|e| engine_error(format!("读取缓存目录失败: {e}")))?;
                    if !meta.is_dir()
                        || meta.file_type().is_symlink()
                        || !cache_root.canonicalize().ok().is_some_and(|canonical| {
                            path_text(&canonical).eq_ignore_ascii_case(&path_text(&cache_root))
                        })
                    {
                        return Err(engine_error("重试缓存目录不是原位置的普通目录"));
                    }
                }
                Err(error) => return Err(engine_error(format!("创建重试缓存目录失败: {error}"))),
            }
            destination = cache_root.join(&payload.file_name);
            match fs::symlink_metadata(&destination) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(engine_error("重试缓存不能是符号链接"));
                    }
                    if meta.is_file()
                        && meta.len() == payload.size
                        && codex_win_engine::sha256_file(&destination)
                            .is_ok_and(|hash| hash.eq_ignore_ascii_case(&payload.sha256))
                    {
                        return Ok(destination);
                    }
                    let retry_root = base.join(format!("retry-{}", uuid::Uuid::new_v4().simple()));
                    fs::create_dir(&retry_root)
                        .map_err(|e| engine_error(format!("创建隔离暂存目录失败: {e}")))?;
                    destination = retry_root.join(&payload.file_name);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(engine_error(format!("读取重试缓存失败: {error}"))),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(engine_error(format!("读取旧暂存 MSIX 信息失败: {error}")));
        }
    }
    let required = payload.size.saturating_add(payload.size / 2);
    if let Some(free) = free_space(stage_root)? {
        if free < required {
            return Err(engine_error(format!(
                "临时磁盘空间不足：需要至少 {required} 字节，可用 {free} 字节"
            )));
        }
    }
    let mut source = File::open(&source_canonical)
        .map_err(|error| engine_error(format!("打开源 MSIX 失败: {error}")))?;
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| engine_error(format!("创建新暂存 MSIX 失败: {error}")))?;
    let copied = std::io::copy(&mut source, &mut target)
        .map_err(|error| engine_error(format!("复制 MSIX 到本地暂存目录失败: {error}")))?;
    target
        .sync_all()
        .map_err(|error| engine_error(format!("写入暂存 MSIX 未完成: {error}")))?;
    if copied != payload.size {
        return Err(engine_error("源 MSIX 大小在复制过程中改变，拒绝安装"));
    }
    Ok(destination)
}

fn deployment_space_required(unpacked_bytes: u64, entries: u64) -> Result<u64, AppError> {
    // The compressed MSIX size understates the actual package footprint.
    // Include per-file allocation/metadata slack plus a bounded working margin.
    unpacked_bytes
        .checked_add(
            entries
                .checked_mul(4096)
                .ok_or_else(|| engine_error("MSIX 文件数溢出"))?,
        )
        .and_then(|bytes| bytes.checked_add(512 * 1024 * 1024))
        .ok_or_else(|| engine_error("MSIX 展开大小溢出"))
}

/// Read only the verified ZIP directory; never unpack or alter the signed MSIX.
/// Check the actual target, before policy changes or stopping existing apps.
pub fn validate_usb_deployment_space(payload: &Path, target: &Path) -> Result<(), AppError> {
    let file = File::open(payload)
        .map_err(|error| engine_error(format!("读取 MSIX 展开大小失败：{error}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| engine_error(format!("读取 MSIX 目录失败：{error}")))?;
    let mut unpacked = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| engine_error(format!("读取 MSIX 文件大小失败：{error}")))?;
        unpacked = unpacked
            .checked_add(entry.size())
            .ok_or_else(|| engine_error("MSIX 展开大小溢出"))?;
    }
    let required = deployment_space_required(unpacked, archive.len() as u64)?;
    if let Some(free) = available_space(target)? {
        if free < required {
            return Err(engine_error(format!(
                "目标 U 盘磁盘空间不足：{} 需要至少 {} MiB（按 MSIX 展开大小计算），可用 {} MiB；未修改现有安装",
                target.display(), required.div_ceil(1024 * 1024), free / (1024 * 1024)
            )));
        }
    }
    Ok(())
}

/// Remove only a file created beneath `%TEMP%\CodexDesktopInstaller`.  This
/// narrow check keeps the cleanup command from becoming an arbitrary delete
/// primitive if the renderer is ever compromised.
pub fn cleanup_staged_payload(path: &Path) -> Result<(), AppError> {
    let root = std::env::temp_dir().join(TEMP_INSTALLER_DIR);
    let root_metadata = fs::symlink_metadata(&root).map_err(|_| engine_error("暂存目录不存在"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(engine_error("暂存目录不能是符号链接或普通文件"));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| engine_error("暂存目录不存在"))?;
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| engine_error(format!("读取暂存文件失败: {error}")))?;
    if path_metadata.file_type().is_symlink() {
        return Err(engine_error("暂存路径不能是符号链接"));
    }
    let canonical_path = path
        .canonicalize()
        .map_err(|error| engine_error(format!("解析暂存文件失败: {error}")))?;
    if !path_is_within(&canonical_root, &canonical_path)
        || canonical_path.parent() == Some(canonical_root.as_path())
        || !canonical_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("msix"))
    {
        return Err(engine_error("拒绝清理目录外的暂存文件"));
    }
    let parent = canonical_path
        .parent()
        .ok_or_else(|| engine_error("暂存文件路径无效"))?;
    if parent.parent() != Some(canonical_root.as_path()) {
        return Err(engine_error("拒绝清理非版本目录中的暂存文件"));
    }
    if fs::symlink_metadata(path.parent().unwrap_or(path))
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(engine_error("暂存文件的版本目录不能是符号链接"));
    }
    let metadata = fs::metadata(&canonical_path)
        .map_err(|error| engine_error(format!("读取暂存文件失败: {error}")))?;
    if !metadata.is_file() {
        return Err(engine_error("暂存路径不是普通文件"));
    }
    fs::remove_file(&canonical_path)
        .map_err(|error| engine_error(format!("删除暂存 MSIX 失败: {error}")))
}

/// Convert a verified offline report to the existing historical-install
/// descriptor.  Keeping this adapter here lets the old guarded install tail be
/// reused without reintroducing any network/catalog dependency.
pub fn local_selection(report: &OfflinePayloadReport) -> Result<LocalReleasePackage, AppError> {
    let architecture = normalized_architecture(&report.architecture)
        .ok_or_else(|| engine_error("离线报告中的架构无效"))?;
    let resolved = resolved_local_asset(
        Path::new(&report.payload_path),
        &report.codex_desktop_version,
        architecture,
        crate::app::release_install::ReleasePackageFormat::Msix,
        Some(report.package_version.clone()),
    )?;
    Ok(local_package_from_resolved(
        Path::new(&report.payload_path),
        &resolved,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_space_uses_uncompressed_bytes_and_detects_overflow() {
        let gib = 1024 * 1024 * 1024;
        assert_eq!(
            deployment_space_required(2 * gib, 100).unwrap(),
            2 * gib + 100 * 4096 + 512 * 1024 * 1024
        );
        assert!(deployment_space_required(u64::MAX, 1).is_err());
        assert!(deployment_space_required(1, u64::MAX).is_err());
    }
    use std::io::Write;

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "codex-offline-payload-test-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_fake_msix_with_publisher(path: &Path, publisher: &str) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("AppxManifest.xml", options).unwrap();
        let manifest = format!(
            r#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="{publisher}" Version="26.602.3474.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app/Codex.exe" EntryPoint="Windows.FullTrustApplication" /></Applications>
</Package>"#
        );
        writer.write_all(manifest.as_bytes()).unwrap();
        writer.finish().unwrap();
    }

    fn write_fake_msix(path: &Path) {
        write_fake_msix_with_publisher(
            path,
            codex_win_engine::OPENAI_MARKETPLACE_PUBLISHER_SUBJECT,
        );
    }

    fn fixture_manifest(msix: &Path) -> OfflinePayloadManifest {
        OfflinePayloadManifest {
            product_version: "0.1.0".to_string(),
            codex_desktop_version: "26.602.40724".to_string(),
            package_name: "OpenAI.Codex".to_string(),
            architecture: "x64".to_string(),
            min_windows_build: "19041".to_string(),
            payload: OfflinePayloadSpec {
                path: "payload/OpenAI.Codex_x64.msix".to_string(),
                sha256: codex_win_engine::sha256_file(msix).unwrap(),
                size: fs::metadata(msix).unwrap().len(),
            },
            package_version: Some("26.602.3474.0".to_string()),
            release_tag: Some("upstream-26.602.40724".to_string()),
            source: Some("test-fixture".to_string()),
        }
    }

    #[test]
    fn manifest_rejects_path_traversal_and_bad_hash() {
        let manifest = OfflinePayloadManifest {
            product_version: "0.1.0".to_string(),
            codex_desktop_version: "26.602.40724".to_string(),
            package_name: "OpenAI.Codex".to_string(),
            architecture: "x64".to_string(),
            min_windows_build: "19041".to_string(),
            payload: OfflinePayloadSpec {
                path: "../payload/Codex.msix".to_string(),
                sha256: "0".repeat(64),
                size: 1,
            },
            package_version: None,
            release_tag: None,
            source: None,
        };
        assert!(validate_manifest(&manifest).is_err());

        let mut safe = manifest;
        safe.payload.path = "payload/Codex.msix".to_string();
        safe.payload.sha256 = "not-a-hash".to_string();
        assert!(validate_manifest(&safe).is_err());
    }

    #[test]
    fn probe_checks_hash_identity_and_application_entry() {
        let root = test_root("probe");
        let payload = root.join("payload");
        fs::create_dir_all(&payload).unwrap();
        let msix = payload.join("OpenAI.Codex_x64.msix");
        write_fake_msix(&msix);
        let manifest = fixture_manifest(&msix);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let executable = root.join("CodexDesktopSetup.exe");
        fs::write(&executable, b"test").unwrap();

        let report = probe_offline_payload(&executable, None, Some("x86_64"), false).unwrap();
        assert!(report.ready);
        assert!(report.hash_verified);
        assert_eq!(report.package_name, "OpenAI.Codex");
        assert_eq!(report.package_version, "26.602.3474.0");
        assert_eq!(report.release_tag, "local-signed-26.602.40724");
        assert_eq!(report.app_entry.as_deref(), Some("app/Codex.exe"));
        assert!(!report.app_version_verified);
        assert!(
            report.authenticode.is_none(),
            "quick preflight must defer the expensive signature scan"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_claim_requires_the_exact_main_package_publisher_and_identity() {
        let root = test_root("cleanup-claim");
        let payload = root.join("payload");
        fs::create_dir_all(&payload).unwrap();
        let msix = payload.join("OpenAI.Codex_x64.msix");
        write_fake_msix(&msix);
        let manifest = fixture_manifest(&msix);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let executable = root.join("CodexAppManager.exe");
        fs::write(&executable, b"fixture").unwrap();
        let claim = cleanup_package_claim_with_verifier(&executable, |_| Ok(())).unwrap();
        assert_eq!(
            claim.package_full_name,
            "OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0"
        );
        assert_eq!(claim.payload_path, msix.canonicalize().unwrap());
        assert_eq!(claim.payload_sha256, manifest.payload.sha256);

        let denied = cleanup_package_claim_with_verifier(&executable, |_| {
            Err(engine_error("fixture signature rejected"))
        })
        .unwrap_err()
        .to_string();
        assert!(denied.contains("fixture signature rejected"));

        write_fake_msix_with_publisher(&msix, "CN=not-openai");
        let mut altered = fixture_manifest(&msix);
        // Keep the small manifest internally coherent so rejection is caused
        // by the publisher boundary, not a stale size field.
        altered.payload.size = fs::metadata(&msix).unwrap().len();
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&altered).unwrap(),
        )
        .unwrap();
        assert!(cleanup_package_claim_with_verifier(&executable, |_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("发布者不匹配"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stage_copies_and_rehashes_local_payload() {
        let root = test_root("stage");
        let payload = root.join("payload");
        fs::create_dir_all(&payload).unwrap();
        let msix = payload.join("OpenAI.Codex_x64.msix");
        write_fake_msix(&msix);
        let manifest = fixture_manifest(&msix);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let executable = root.join("CodexDesktopSetup.exe");
        fs::write(&executable, b"test").unwrap();

        let staged = stage_offline_payload(&executable, None, Some("x64"), false).unwrap();
        assert!(Path::new(&staged.staged_path).is_file());
        assert_eq!(staged.sha256, manifest.payload.sha256);
        assert!(staged.payload.authenticode.is_none());
        cleanup_staged_payload(Path::new(&staged.staged_path)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn windows_build_parser_uses_last_numeric_component() {
        assert_eq!(parse_windows_build_number("10.0.19045\r\n"), Some(19045));
        assert_eq!(parse_windows_build_number("Build: 22631"), Some(22631));
        assert_eq!(parse_windows_build_number("unknown"), None);
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn staging_reuses_valid_bytes_even_when_locked_and_disk_is_full() {
        let root = test_root("stage-reuse");
        let source_dir = root.join("payload");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("OpenAI.Codex_x64.msix");
        write_fake_msix(&source);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&fixture_manifest(&source)).unwrap(),
        )
        .unwrap();
        let executable = root.join("CodexDesktopSetup.exe");
        fs::write(&executable, "fixture").unwrap();
        let payload = probe_offline_payload(&executable, None, Some("x64"), false).unwrap();
        let stage_root = root.join("stage");
        fs::create_dir(&stage_root).unwrap();
        let stage_root = stage_root.canonicalize().unwrap();
        let staged =
            copy_or_reuse_staged_payload(&payload, &stage_root, |_| Ok(Some(u64::MAX))).unwrap();
        #[cfg(windows)]
        let locked = {
            use std::os::windows::fs::OpenOptionsExt;
            let mut permissions = fs::metadata(&staged).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&staged, permissions).unwrap();
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&staged)
                .unwrap()
        };
        let reused = copy_or_reuse_staged_payload(&payload, &stage_root, |_| {
            panic!("a validated cache requires no additional copy space")
        })
        .unwrap();
        assert_eq!(reused, staged);
        assert_eq!(
            codex_win_engine::sha256_file(&reused).unwrap(),
            payload.sha256
        );
        #[cfg(windows)]
        {
            drop(locked);
            let mut permissions = fs::metadata(&staged).unwrap().permissions();
            permissions.set_readonly(false);
            fs::set_permissions(&staged, permissions).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn staging_preserves_broken_locked_attempt_and_does_not_inherit_readonly() {
        let root = test_root("stage-broken");
        let source_dir = root.join("payload");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("OpenAI.Codex_x64.msix");
        write_fake_msix(&source);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&fixture_manifest(&source)).unwrap(),
        )
        .unwrap();
        let executable = root.join("CodexDesktopSetup.exe");
        fs::write(&executable, "fixture").unwrap();
        let payload = probe_offline_payload(&executable, None, Some("x64"), false).unwrap();
        let stage_root = root.join("stage");
        fs::create_dir(&stage_root).unwrap();
        let stage_root = stage_root.canonicalize().unwrap();
        let broken = stage_root.join(&payload.file_name);
        fs::write(&broken, "incomplete previous attempt").unwrap();
        #[cfg(windows)]
        let locked = {
            use std::os::windows::fs::OpenOptionsExt;
            for path in [&source, &broken] {
                let mut permissions = fs::metadata(path).unwrap().permissions();
                permissions.set_readonly(true);
                fs::set_permissions(path, permissions).unwrap();
            }
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&broken)
                .unwrap()
        };
        assert!(copy_or_reuse_staged_payload(&payload, &stage_root, |_| Ok(Some(0))).is_err());
        let staged =
            copy_or_reuse_staged_payload(&payload, &stage_root, |_| Ok(Some(u64::MAX))).unwrap();
        assert_ne!(staged, broken);
        assert_eq!(staged.parent().unwrap().parent(), stage_root.parent());
        assert_eq!(
            fs::read_to_string(&broken).unwrap(),
            "incomplete previous attempt"
        );
        assert_eq!(
            codex_win_engine::sha256_file(&staged).unwrap(),
            payload.sha256
        );
        assert_eq!(
            copy_or_reuse_staged_payload(&payload, &stage_root, |_| {
                panic!("repeat fallback must reuse verified cache without another copy")
            })
            .unwrap(),
            staged
        );
        assert!(!fs::metadata(&staged).unwrap().permissions().readonly());
        #[cfg(windows)]
        {
            drop(locked);
            for path in [&source, &broken] {
                let mut permissions = fs::metadata(path).unwrap().permissions();
                permissions.set_readonly(false);
                fs::set_permissions(path, permissions).unwrap();
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
