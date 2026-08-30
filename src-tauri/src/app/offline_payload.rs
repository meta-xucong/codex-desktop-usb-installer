//! Local, offline Codex Desktop payload handling.
//!
//! The first product build deliberately keeps the Codex MSIX as an opaque,
//! signed payload.  This module only discovers the payload beside the setup
//! executable, validates the release manifest and package identity, and stages
//! a byte-for-byte copy in the local temporary directory before the existing
//! Windows engine performs the AppX deployment.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::app::disk::available_space;
use crate::app::release_install::{
    local_package_from_resolved, resolved_local_asset, LocalReleasePackage, ReleaseArchitecture,
};
use crate::errors::AppError;

const MANIFEST_FILE_NAME: &str = "manifest.json";
const TEMP_INSTALLER_DIR: &str = "CodexDesktopInstaller";

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
    path.to_string_lossy().into_owned()
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
        .last()
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

/// Verify the bundle.  `enforce_authenticode` is true only for the Windows
/// production command; tests may pass false to exercise the ZIP/manifest path
/// with a synthetic unsigned fixture.
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
    let authenticode = {
        let report = codex_win_engine::verify_openai_authenticode(&bundle.payload_path)
            .map_err(|error| engine_error(format!("Authenticode 校验失败: {error}")))?;
        if enforce_authenticode && !report.is_valid_openai() {
            return Err(engine_error(format!(
                "Authenticode 校验失败：status={} subject={}",
                report.status, report.subject
            )));
        }
        Some(report)
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
    let release_tag = bundle
        .manifest
        .release_tag
        .clone()
        .unwrap_or_else(|| format!("local-signed-{}", bundle.manifest.codex_desktop_version));
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

pub fn stage_offline_payload(
    executable: &Path,
    package_override: Option<&Path>,
    expected_architecture: Option<&str>,
    enforce_authenticode: bool,
) -> Result<OfflineStageReport, AppError> {
    let payload = probe_offline_payload(
        executable,
        package_override,
        expected_architecture,
        enforce_authenticode,
    )?;
    let stage_root = secure_stage_directory(&payload.codex_desktop_version)?;
    let required = payload.size.saturating_add(payload.size / 2);
    if let Some(free) = available_space(&stage_root)? {
        if free < required {
            return Err(engine_error(format!(
                "临时磁盘空间不足：需要至少 {} 字节，可用 {} 字节",
                required, free
            )));
        }
    }
    let destination = stage_root.join(&payload.file_name);
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
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(engine_error(format!("读取旧暂存 MSIX 信息失败: {error}")));
        }
    }
    let destination_canonical = destination.canonicalize().ok();
    if destination_canonical.as_ref() != Some(&source_canonical) {
        if destination.exists() {
            fs::remove_file(&destination)
                .map_err(|error| engine_error(format!("清理旧暂存 MSIX 失败: {error}")))?;
        }
        fs::copy(&source_canonical, &destination)
            .map_err(|error| engine_error(format!("复制 MSIX 到本地暂存目录失败: {error}")))?;
    }
    let staged_hash = codex_win_engine::sha256_file(&destination)
        .map_err(|error| engine_error(format!("再次计算暂存 MSIX SHA-256 失败: {error}")))?;
    if !staged_hash.eq_ignore_ascii_case(&payload.sha256) {
        return Err(engine_error("暂存 MSIX 二次校验失败，拒绝安装"));
    }
    Ok(OfflineStageReport {
        source_path: payload.payload_path.clone(),
        staged_path: path_text(&destination),
        size: payload.size,
        sha256: staged_hash,
        payload,
    })
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
    use std::io::Write;

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "codex-offline-payload-test-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_fake_msix(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("AppxManifest.xml", options).unwrap();
        writer
            .write_all(
                br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI" Version="26.602.3474.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app/Codex.exe" EntryPoint="Windows.FullTrustApplication" /></Applications>
</Package>"#,
            )
            .unwrap();
        writer.finish().unwrap();
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
            release_tag: Some("local-signed-26.602.40724".to_string()),
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
        assert_eq!(report.app_entry.as_deref(), Some("app/Codex.exe"));
        assert!(!report.app_version_verified);

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
        cleanup_staged_payload(Path::new(&staged.staged_path)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn windows_build_parser_uses_last_numeric_component() {
        assert_eq!(parse_windows_build_number("10.0.19045\r\n"), Some(19045));
        assert_eq!(parse_windows_build_number("Build: 22631"), Some(22631));
        assert_eq!(parse_windows_build_number("unknown"), None);
    }
}
