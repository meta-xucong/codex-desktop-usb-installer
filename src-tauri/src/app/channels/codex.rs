//! Codex payload update channel.
//!
//! This module updates the authenticated USB bundle only. The deployed
//! Windows AppX is deliberately handled by a separate explicit action.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use url::{Host, Url};

use codex_win_engine::sys::{fetch_text_with_network_headers, GITHUB_API_HEADERS};
use codex_win_engine::{
    download_to_with_progress_bounded_with_network,
    download_to_with_progress_bounded_with_network_no_redirect, fetch_text_with_network,
    fetch_text_with_network_no_redirect, find_msix_sha256, limits::MAX_PACKAGE_BYTES,
    parse_manifest_for_arch, read_msix_identity, sha256_file, validate_codex_identity,
    MsixIdentity, NetworkConfig,
};

use crate::app::settings_store::{
    AppSettings, CodexPayloadSourceKind, LEGACY_CODEX_PAYLOAD_BASE_URL,
};
use crate::app::update_channel::{
    compare_versions, is_numeric_version, read_usb_codex_desktop_version, ChannelKind,
    UpdateContext, UpdateInfo,
};
use crate::errors::AppError;

pub const DEFAULT_CODEX_PAYLOAD_REPO: &str = "Wangnov/codex-app-mirror";

const STAGING_DIR: &str = ".staging";
const TRANSACTION_PREFIX: &str = "codex-update-";
const OWNER_SIDECAR_PREFIX: &str = ".codex-update-";
const OWNER_SIDECAR_SUFFIX: &str = ".owner.json";
const SHA256SUMS_NAME: &str = "SHA256SUMS.txt";
const ROOT_MANIFEST_NAME: &str = "manifest.json";
const USB_MANIFEST_NAME: &str = "usb-manifest.json";
const USB_LICENSE_NAME: &str = "usb-license.json";
const TRANSACTION_VERSION: u32 = 1;
const MAX_TRANSACTION_STATE_BYTES: u64 = 64 * 1024;
const SOURCE_METADATA_ATTEMPTS: usize = 2;
const SOURCE_METADATA_RETRY_DELAY_MS: u64 = 450;
// Auto only probes the Wangnov-compatible Codex payload chain.  Keep the
// public mirror branch below for explicit legacy configurations, but never
// treat its protocol as an automatic USB update candidate.
const AUTO_SOURCE_ORDER: [CodexPayloadSourceKind; 2] = [
    CodexPayloadSourceKind::Relay,
    CodexPayloadSourceKind::Github,
];
static CODEX_UPDATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    id: u64,
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhAsset {
    id: u64,
    name: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RelayLatest {
    protocol: u32,
    architecture: String,
    assets: RelayAssets,
    #[serde(rename = "releaseId")]
    release_id: u64,
    repository: String,
    tag: String,
    #[serde(default)]
    identity: Option<RelayIdentity>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct RelayAssets {
    checksums: RelayAsset,
    #[serde(rename = "release-manifest")]
    release_manifest: RelayAsset,
    #[serde(rename = "windows-msix")]
    windows_msix: RelayAsset,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct RelayAsset {
    #[serde(default)]
    id: Option<u64>,
    path: String,
    sha256: String,
    size: u64,
    #[serde(rename = "githubDigest", default)]
    github_digest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RelayIdentity {
    assets: RelayAssets,
    #[serde(rename = "releaseId")]
    release_id: u64,
    tag: String,
}

#[derive(Debug, Clone)]
struct ResolvedAsset {
    size: u64,
    url: String,
    expected_sha256: Option<String>,
}

/// Artifact identity only. Apply never trusts a URL from the renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexPayloadIdentity {
    /// Source used for this check (`relay` or `github`). This is part of the
    /// stale identity so a confirmed relay update cannot be applied through a
    /// different source after settings change.
    pub source_kind: CodexPayloadSourceKind,
    /// Canonical source locator: relay base URL or GitHub repository API URL.
    pub source_locator: String,
    pub repo: String,
    pub release_id: u64,
    pub release_tag: String,
    pub msix_asset_id: Option<u64>,
    pub msix_asset_name: String,
    pub sha256_asset_id: Option<u64>,
    pub sha256_asset_name: String,
    pub manifest_asset_id: Option<u64>,
    pub manifest_asset_name: String,
    pub package_moniker: String,
    pub architecture: String,
    pub msix_version: String,
    pub app_version: String,
    pub msix_size: u64,
    pub msix_sha256: Option<String>,
    pub checksums_sha256: Option<String>,
    pub manifest_sha256: Option<String>,
    #[serde(default)]
    pub github_digest: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedRelease {
    identity: CodexPayloadIdentity,
    app_version: String,
    msix: ResolvedAsset,
    checksums: ResolvedAsset,
}

#[derive(Debug, Clone)]
struct LocalBundle {
    manifest: serde_json::Value,
    usb_manifest: serde_json::Value,
    payload_path: PathBuf,
    payload_relative: String,
    architecture: String,
    current_version: String,
}

/// Immutable local state captured together with a check result. The remote
/// identity alone cannot detect a media change between check and apply when
/// the payload and all mutable manifests are changed together.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct LocalBundleSnapshot {
    current_version: String,
    payload_relative: String,
    architecture: String,
    payload_sha256: String,
    root_manifest_sha256: String,
    usb_manifest_sha256: String,
    checksums_sha256: String,
}

/// The public UpdateInfo shape remains unchanged. Codex keeps its remote
/// identity flat and adds an optional stale-state field inside the channel
/// payload so old callers can still deserialize the identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexUpdatePayload {
    #[serde(flatten)]
    identity: CodexPayloadIdentity,
    #[serde(default)]
    local_snapshot: Option<LocalBundleSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TransactionState {
    version: u32,
    phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryTransactionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<TransactionOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionOwner {
    transaction_id: String,
    installation_id: String,
    license_signature_sha256: String,
    fingerprint_sha256: String,
    #[serde(default, skip_serializing)]
    legacy_bundle_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryTransactionState {
    prior_payload_missing: bool,
    transaction_id: String,
    installation_id: String,
    license_signature_sha256: String,
    fingerprint_sha256: String,
    fingerprint: crate::app::usb_guard::UsbFingerprint,
    target_relative_path: String,
    #[serde(default)]
    staged_payload_sha256: Option<String>,
    #[serde(default, skip_serializing)]
    // Retained to deserialize pre-owner-marker recovery state written by older releases.
    #[allow(dead_code)]
    legacy_bundle_root: Option<String>,
}

struct StagedCommit<'a> {
    root: &'a Path,
    tx: &'a Path,
    local: &'a LocalBundle,
    fresh: &'a ResolvedRelease,
    expected: &'a UpdateInfo,
    staged: &'a Path,
    actual_sha: &'a str,
    total: u64,
    progress: &'a dyn Fn(u64, u64),
    reactivation_reinstall: bool,
    deferred_cleanup: &'a mut Option<crate::app::usb_guard::DeferredRevocationCleanup>,
}

type DownloadFn = dyn Fn(&str, &Path, u64, &dyn Fn(u64)) -> Result<(), String>;
type FetchTextFn = dyn Fn(&str) -> Result<String, String>;
type VerifyAuthenticodeFn = dyn Fn(&Path) -> Result<(), String>;
type ReadIdentityFn = dyn Fn(&Path) -> Result<MsixIdentity, String>;

/// Dependencies used by the pre-commit part of the update transaction.
///
/// The production wrapper below keeps the real network and Windows checks in
/// place. Tests can replace only these side effects, while the size, checksum,
/// identity, path-safety and commit code remains the same code path.
struct ApplyTransport {
    download: Box<DownloadFn>,
    fetch_text: Box<FetchTextFn>,
    verify_authenticode: Box<VerifyAuthenticodeFn>,
    read_identity: Box<ReadIdentityFn>,
}

pub fn validate_repo_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split('/').collect();
    let valid_segment = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if parts.len() != 2 || !valid_segment(parts[0]) || !valid_segment(parts[1]) {
        return Err(AppError::Engine(format!(
            "codex payload repo 必须是 owner/repo 形式：{trimmed}"
        )));
    }
    Ok(trimmed.to_string())
}

fn resolved_repo(settings: &AppSettings) -> Result<String, AppError> {
    if settings.codex_payload_repo.trim().is_empty() {
        Ok(DEFAULT_CODEX_PAYLOAD_REPO.to_string())
    } else {
        validate_repo_name(&settings.codex_payload_repo)
    }
}

fn repo_api_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases/latest")
}

fn is_builtin_codex_mirror_base(raw: &str) -> bool {
    let Ok(normalized) = normalize_relay_base_url(raw) else {
        return false;
    };
    let Ok(url) = Url::parse(&normalized) else {
        return false;
    };
    url.host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("codexapp.agentsmirror.com"))
        && url.path() == "/"
}

/// Normalize and validate the relay directory URL. The relay is deliberately
/// constrained to HTTPS, public DNS names, and a directory path; every asset
/// URL is then derived from this value and checked to remain on the same
/// origin. This prevents a renderer-supplied URL or manifest path from
/// redirecting the updater to another host.
pub fn normalize_relay_base_url(raw: &str) -> Result<String, AppError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Engine("Codex relay 地址不能为空".to_string()));
    }
    if raw
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\\')
    {
        return Err(AppError::Engine(
            "Codex relay 地址包含非法空白或控制字符".to_string(),
        ));
    }
    let mut url = Url::parse(raw)
        .map_err(|error| AppError::Engine(format!("Codex relay 地址无法解析：{error}")))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::Engine(
            "Codex relay 地址必须是无凭据的 HTTPS 域名 URL".to_string(),
        ));
    }
    match url.host() {
        Some(Host::Domain(domain))
            if !domain.eq_ignore_ascii_case("localhost")
                && !domain.ends_with(".localhost")
                && !domain.ends_with(".local")
                && !domain.ends_with(".internal")
                && !domain.ends_with(".home.arpa") => {}
        _ => {
            return Err(AppError::Engine(
                "Codex relay 地址必须使用公网域名，不能使用 IP 或本地域名".to_string(),
            ));
        }
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::Engine(
            "Codex relay 地址不能包含 query 或 fragment".to_string(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
    }
    Ok(url.to_string())
}

pub fn validate_relay_base_url(raw: &str) -> Result<String, AppError> {
    let normalized = normalize_relay_base_url(raw)?;
    crate::app::url_guard::validate_custom_source(&normalized)
        .map_err(|error| AppError::Engine(format!("Codex relay 地址无效：{error}")))
}

fn relay_asset_url(base: &str, relative: &str) -> Result<String, AppError> {
    let base = normalize_relay_base_url(base)?;
    let relative = normalize_relative_path(relative)?;
    let base_url = Url::parse(&base)
        .map_err(|error| AppError::Engine(format!("Codex relay 地址无法解析：{error}")))?;
    let joined = base_url
        .join(&relative)
        .map_err(|error| AppError::Engine(format!("拼接 Codex relay 资产地址失败：{error}")))?;
    if joined.scheme() != base_url.scheme()
        || joined.host_str() != base_url.host_str()
        || joined.port_or_known_default() != base_url.port_or_known_default()
        || !joined.path().starts_with(base_url.path())
        || joined.username() != base_url.username()
    {
        return Err(AppError::Engine(
            "Codex relay 资产地址越过了配置的来源边界".to_string(),
        ));
    }
    require_https_url(joined.as_str(), "Codex relay 资产地址")?;
    Ok(joined.to_string())
}

fn relay_asset_hash(asset: &RelayAsset, label: &str) -> Result<String, AppError> {
    normalize_digest(Some(asset.sha256.as_str()))?
        .ok_or_else(|| AppError::Engine(format!("Codex relay {label} 缺少 SHA256")))
}

fn validate_relay_asset(asset: &RelayAsset, label: &str) -> Result<(String, String), AppError> {
    let path = normalize_relative_path(&asset.path)?;
    if asset.size == 0 || asset.size > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine(format!(
            "Codex relay {label} 大小无效：{}",
            asset.size
        )));
    }
    let sha256 = relay_asset_hash(asset, label)?;
    Ok((path, sha256))
}

fn require_windows() -> Result<(), AppError> {
    if !cfg!(target_os = "windows") {
        return Err(AppError::UnsupportedPlatform);
    }
    Ok(())
}

fn network_from_settings(settings: &AppSettings) -> NetworkConfig {
    match settings.proxy_mode {
        crate::app::settings_store::ProxyMode::System => NetworkConfig::system(),
        crate::app::settings_store::ProxyMode::Direct => NetworkConfig::direct(),
        crate::app::settings_store::ProxyMode::Custom => {
            NetworkConfig::custom(settings.custom_proxy_url.clone())
        }
    }
}

fn fetch_json<T: for<'de> Deserialize<'de>>(
    fetch: &dyn Fn(&str) -> Result<String, AppError>,
    url: &str,
) -> Result<T, AppError> {
    let text = fetch(url)?;
    serde_json::from_str(&text)
        .map_err(|error| AppError::Engine(format!("解析 GitHub API 响应失败 ({url}): {error}")))
}

fn require_https_url(url: &str, what: &str) -> Result<(), AppError> {
    let parsed = Url::parse(url).map_err(|_| AppError::Engine(format!("{what} 不是有效 URL")))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(AppError::Engine(format!("{what} 不是安全的 HTTPS URL")));
    }
    Ok(())
}

fn select_assets(
    release: &GhRelease,
    architecture: &str,
) -> Result<(GhAsset, GhAsset, GhAsset), AppError> {
    let msix = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.starts_with("OpenAI.Codex_")
                && asset.name.to_ascii_lowercase().ends_with(".msix")
                && asset.name.contains(&format!("_{architecture}_"))
        })
        .cloned();
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS-windows.txt")
        .cloned();
    let manifest = release
        .assets
        .iter()
        .find(|asset| asset.name == "release-manifest.json")
        .cloned();
    match (msix, checksums, manifest) {
        (Some(msix), Some(checksums), Some(manifest)) => Ok((msix, checksums, manifest)),
        _ => Err(AppError::Engine(format!(
            "Release {} 缺少所需资产（架构 {architecture}）",
            release.tag_name
        ))),
    }
}

fn normalize_digest(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(raw) = raw else { return Ok(None) };
    let value = raw.trim();
    let value = if value.len() >= 7
        && value
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sha256:"))
    {
        &value[7..]
    } else {
        value
    };
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Engine(
            "GitHub digest 格式无效，拒绝继续更新".to_string(),
        ));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

fn manifest_arch_value(manifest_text: &str, architecture: &str, key: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(manifest_text).ok()?;
    value
        .pointer(&format!(
            "/sources/windows/architectures/{architecture}/{key}"
        ))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn app_version_from_manifest(manifest_text: &str, architecture: &str) -> Result<String, AppError> {
    let value: serde_json::Value = serde_json::from_str(manifest_text)
        .map_err(|error| AppError::Engine(format!("解析 release-manifest.json 失败: {error}")))?;
    manifest_arch_value(manifest_text, architecture, "appVersion")
        .or_else(|| {
            value
                .get("appVersion")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| AppError::Engine("release-manifest.json 缺少 appVersion".to_string()))
}

fn required_manifest_arch_value(
    manifest_text: &str,
    architecture: &str,
    key: &str,
) -> Result<String, AppError> {
    manifest_arch_value(manifest_text, architecture, key)
        .ok_or_else(|| AppError::Engine(format!("release-manifest.json 缺少 {architecture} {key}")))
}

fn resolve_latest(
    api_fetch: &dyn Fn(&str) -> Result<String, AppError>,
    asset_fetch: &dyn Fn(&str) -> Result<String, AppError>,
    repo: &str,
    architecture: &str,
) -> Result<ResolvedRelease, AppError> {
    let release: GhRelease = fetch_json(api_fetch, &repo_api_url(repo))?;
    let (msix, checksums, manifest) = select_assets(&release, architecture)?;
    for (asset, label) in [
        (&msix, "MSIX"),
        (&checksums, "SHA256SUMS"),
        (&manifest, "release-manifest"),
    ] {
        if asset.size == 0 || asset.size > MAX_PACKAGE_BYTES {
            return Err(AppError::Engine(format!(
                "GitHub {label} 资产大小无效：{}",
                asset.size
            )));
        }
    }
    require_https_url(&msix.browser_download_url, "MSIX 下载地址")?;
    require_https_url(&checksums.browser_download_url, "SHA256SUMS 下载地址")?;
    require_https_url(&manifest.browser_download_url, "release-manifest 下载地址")?;
    let manifest_text = asset_fetch(&manifest.browser_download_url)?;
    let app_version = app_version_from_manifest(&manifest_text, architecture)?;
    let package_moniker =
        required_manifest_arch_value(&manifest_text, architecture, "packageMoniker")?;
    let msix_version = required_manifest_arch_value(&manifest_text, architecture, "version")?;
    let release_architecture =
        required_manifest_arch_value(&manifest_text, architecture, "architecture")?;
    if !release_architecture.eq_ignore_ascii_case(architecture) {
        return Err(AppError::Engine(format!(
            "release-manifest 的架构与当前目标不一致：{release_architecture}"
        )));
    }
    let digest = normalize_digest(msix.digest.as_deref())?;
    let identity = CodexPayloadIdentity {
        source_kind: CodexPayloadSourceKind::Github,
        source_locator: repo_api_url(repo),
        repo: repo.to_string(),
        release_id: release.id,
        release_tag: release.tag_name,
        msix_asset_id: Some(msix.id),
        msix_asset_name: msix.name.clone(),
        sha256_asset_id: Some(checksums.id),
        sha256_asset_name: checksums.name.clone(),
        manifest_asset_id: Some(manifest.id),
        manifest_asset_name: manifest.name.clone(),
        package_moniker,
        architecture: architecture.to_string(),
        msix_version,
        app_version: app_version.clone(),
        msix_size: msix.size,
        msix_sha256: digest.clone(),
        checksums_sha256: None,
        manifest_sha256: Some(sha256_bytes(manifest_text.as_bytes())),
        github_digest: digest,
    };
    Ok(ResolvedRelease {
        identity,
        app_version,
        msix: ResolvedAsset {
            size: msix.size,
            url: msix.browser_download_url,
            expected_sha256: None,
        },
        checksums: ResolvedAsset {
            size: checksums.size,
            url: checksums.browser_download_url,
            expected_sha256: None,
        },
    })
}

/// Resolve the public mirror's Windows payload contract. This is deliberately
/// separate from the legacy `latest.json` relay contract: the mirror's
/// `/latest/manifest` is the same manifest consumed by the normal Windows
/// installer update path, while `/manager/latest.json` belongs to the Tauri
/// manager self-updater and must never be parsed as a Codex payload release.
fn resolve_mirror_latest(
    asset_fetch: &dyn Fn(&str) -> Result<String, AppError>,
    base_url: &str,
    architecture: &str,
) -> Result<ResolvedRelease, AppError> {
    let base_url = validate_relay_base_url(base_url)?;
    let manifest_path = "latest/manifest";
    let checksums_path = "latest/checksums";
    let msix_path = if architecture.eq_ignore_ascii_case("arm64") {
        "latest/win-arm64"
    } else {
        "latest/win-x64"
    };
    let manifest_url = relay_asset_url(&base_url, manifest_path)?;
    let checksums_url = relay_asset_url(&base_url, checksums_path)?;
    let msix_url = relay_asset_url(&base_url, msix_path)?;

    let manifest_text = asset_fetch(&manifest_url)?;
    let release = parse_manifest_for_arch(&manifest_text, Some(architecture))
        .map_err(|error| AppError::Engine(format!("解析 Codex 镜像 manifest 失败：{error}")))?;
    let release_architecture = release
        .architecture
        .as_deref()
        .unwrap_or(architecture)
        .trim();
    if !release_architecture.eq_ignore_ascii_case(architecture) {
        return Err(AppError::Engine(format!(
            "Codex 镜像 manifest 架构与当前目标不一致：{release_architecture}"
        )));
    }
    if !release.package_moniker.starts_with("OpenAI.Codex_")
        || !release
            .package_moniker
            .to_ascii_lowercase()
            .contains(&format!("_{}_", architecture.to_ascii_lowercase()))
    {
        return Err(AppError::Engine(
            "Codex 镜像 manifest 的 packageMoniker 与目标架构不一致".to_string(),
        ));
    }
    let msix_size = release.content_length.ok_or_else(|| {
        AppError::Engine("Codex 镜像 manifest 缺少 Windows MSIX 大小".to_string())
    })?;
    if msix_size == 0 || msix_size > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine(format!(
            "Codex 镜像 MSIX 大小无效：{msix_size}"
        )));
    }

    // Fetch the small checksum file during the check, rather than waiting
    // until after a 700+ MB download. This both proves the mirror is usable
    // and binds the latest alias to the exact package moniker advertised by
    // its manifest.
    let checksums_text = asset_fetch(&checksums_url)?;
    if checksums_text.is_empty() || checksums_text.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine(
            "Codex 镜像 checksums 响应大小无效".to_string(),
        ));
    }
    let msix_sha = find_msix_sha256(&checksums_text, &release.package_moniker)
        .map_err(|error| AppError::Engine(format!("解析 Codex 镜像 checksums 失败：{error}")))?;
    let checksums_sha = sha256_bytes(checksums_text.as_bytes());
    let selected_checksum_identity_sha =
        mirror_selected_checksum_identity_sha256(&release.package_moniker, &msix_sha);
    // The mirror manifest has no GitHub release id. Derive the identity from
    // stable release facts rather than the raw manifest bytes: latest aliases
    // may be regenerated with a new generatedAt/ETag while serving the same
    // package. The package/checksum/identity checks below still bind the
    // actual download to the exact release facts.
    let stable_manifest_sha = mirror_stable_identity_sha256(
        &manifest_text,
        architecture,
        &release.version,
        &release.package_version,
        &release.package_moniker,
        msix_size,
        &msix_sha,
        &selected_checksum_identity_sha,
    )?;
    let release_id = u64::from_str_radix(&stable_manifest_sha[..16], 16).unwrap_or(0);
    let release_tag = format!(
        "mirror-{}-{}",
        release.version.trim(),
        release.package_version.trim()
    );
    let msix_name = format!("{}.Msix", release.package_moniker);
    Ok(ResolvedRelease {
        identity: CodexPayloadIdentity {
            source_kind: CodexPayloadSourceKind::Mirror,
            source_locator: base_url,
            repo: DEFAULT_CODEX_PAYLOAD_REPO.to_string(),
            release_id,
            release_tag,
            msix_asset_id: None,
            msix_asset_name: msix_name,
            sha256_asset_id: None,
            sha256_asset_name: checksums_path.to_string(),
            manifest_asset_id: None,
            manifest_asset_name: manifest_path.to_string(),
            package_moniker: release.package_moniker,
            architecture: architecture.to_string(),
            msix_version: release.package_version,
            app_version: release.version.clone(),
            msix_size,
            msix_sha256: Some(msix_sha.clone()),
            // Bind the selected Windows checksum record, not unrelated
            // macOS/ARM entries that may be added to the same checksums file.
            checksums_sha256: Some(selected_checksum_identity_sha),
            // This field is the stable mirror identity for this source kind,
            // not the volatile raw hash of /latest/manifest.
            manifest_sha256: Some(stable_manifest_sha),
            github_digest: None,
        },
        app_version: release.version,
        msix: ResolvedAsset {
            size: msix_size,
            url: msix_url,
            expected_sha256: Some(msix_sha),
        },
        checksums: ResolvedAsset {
            size: checksums_text.len() as u64,
            url: checksums_url,
            expected_sha256: Some(checksums_sha),
        },
    })
}

fn mirror_selected_checksum_identity_sha256(package_moniker: &str, msix_sha256: &str) -> String {
    sha256_bytes(
        format!("codex-mirror-checksum-identity-v1|{package_moniker}|{msix_sha256}").as_bytes(),
    )
}

/// Build a deterministic mirror identity from the fields that affect the
/// selected Windows payload.  The public mirror's latest manifest is an alias
/// and may contain operational metadata (generation time, cache validators,
/// upstream timestamps) that is allowed to change without changing the
/// package.  Those fields must not invalidate a confirmation between check and
/// apply.  Conversely, a version, package identity, size, selected checksum,
/// or manifest schema change must produce a new identity.
#[allow(clippy::too_many_arguments)]
fn mirror_stable_identity_sha256(
    manifest_text: &str,
    architecture: &str,
    app_version: &str,
    msix_version: &str,
    package_moniker: &str,
    msix_size: u64,
    msix_sha256: &str,
    checksums_sha256: &str,
) -> Result<String, AppError> {
    let manifest: serde_json::Value = serde_json::from_str(manifest_text)
        .map_err(|error| AppError::Engine(format!("解析 Codex 镜像 manifest 失败：{error}")))?;
    let schema_version = manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let canonical = serde_json::json!({
        "identitySchema": 1,
        "manifestSchemaVersion": schema_version,
        "architecture": architecture,
        "appVersion": app_version,
        "msixVersion": msix_version,
        "packageMoniker": package_moniker,
        "msixSize": msix_size,
        "msixSha256": msix_sha256,
        "checksumsSha256": checksums_sha256,
    });
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| AppError::Engine(format!("序列化 Codex 镜像稳定身份失败：{error}")))?;
    Ok(sha256_bytes(&bytes))
}

fn validate_relay_descriptor(descriptor: &RelayLatest) -> Result<(), AppError> {
    match descriptor.protocol {
        // Protocol 1 is retained for already deployed/custom relay instances.
        1 => Ok(()),
        // The production relay currently publishes protocol 2.  It repeats
        // the release identity under `identity`; require the duplicate fields
        // to agree before any asset URL is derived from the descriptor.
        2 => {
            let identity = descriptor.identity.as_ref().ok_or_else(|| {
                AppError::Engine("Codex relay 协议 2 缺少 identity 发布身份，已拒绝".to_string())
            })?;
            if identity.release_id != descriptor.release_id
                || identity.tag != descriptor.tag
                || identity.assets != descriptor.assets
            {
                return Err(AppError::Engine(
                    "Codex relay 协议 2 的 identity 与顶层发布身份不一致，已拒绝".to_string(),
                ));
            }
            Ok(())
        }
        protocol => Err(AppError::Engine(format!(
            "Codex relay 协议版本不受支持：{protocol}"
        ))),
    }
}

fn resolve_relay_latest(
    asset_fetch: &dyn Fn(&str) -> Result<String, AppError>,
    base_url: &str,
    architecture: &str,
) -> Result<ResolvedRelease, AppError> {
    let base_url = validate_relay_base_url(base_url)?;
    let latest_url = relay_asset_url(&base_url, "latest.json")?;
    let latest_text = asset_fetch(&latest_url)?;
    let descriptor: RelayLatest = serde_json::from_str(&latest_text)
        .map_err(|error| AppError::Engine(format!("解析 Codex relay latest.json 失败：{error}")))?;
    validate_relay_descriptor(&descriptor)?;
    if !descriptor.architecture.eq_ignore_ascii_case(architecture) {
        return Err(AppError::Engine(format!(
            "Codex relay 架构与当前目标不一致：{}",
            descriptor.architecture
        )));
    }
    let repo = validate_repo_name(&descriptor.repository)?;
    if descriptor.tag.trim().is_empty() {
        return Err(AppError::Engine(
            "Codex relay latest.json 缺少 release tag".to_string(),
        ));
    }

    let (msix_path, msix_sha) = validate_relay_asset(&descriptor.assets.windows_msix, "MSIX")?;
    let (checksums_path, checksums_sha) =
        validate_relay_asset(&descriptor.assets.checksums, "SHA256SUMS")?;
    let (manifest_path, manifest_sha) =
        validate_relay_asset(&descriptor.assets.release_manifest, "release-manifest")?;
    let msix_name = Path::new(&msix_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::Engine("Codex relay MSIX 文件名无效".to_string()))?;
    if !msix_name.starts_with("OpenAI.Codex_")
        || !msix_name.to_ascii_lowercase().ends_with(".msix")
        || !msix_name.contains(&format!("_{architecture}_"))
    {
        return Err(AppError::Engine(
            "Codex relay MSIX 资产名称与目标架构不一致".to_string(),
        ));
    }

    let manifest_url = relay_asset_url(&base_url, &manifest_path)?;
    let manifest_text = asset_fetch(&manifest_url)?;
    if manifest_text.len() as u64 != descriptor.assets.release_manifest.size {
        return Err(AppError::Engine(
            "Codex relay release-manifest 大小与描述符不匹配".to_string(),
        ));
    }
    if !sha256_bytes(manifest_text.as_bytes()).eq_ignore_ascii_case(&manifest_sha) {
        return Err(AppError::Engine(
            "Codex relay release-manifest SHA256 不匹配".to_string(),
        ));
    }
    let app_version = app_version_from_manifest(&manifest_text, architecture)?;
    let package_moniker =
        required_manifest_arch_value(&manifest_text, architecture, "packageMoniker")?;
    let msix_version = required_manifest_arch_value(&manifest_text, architecture, "version")?;
    let release_architecture =
        required_manifest_arch_value(&manifest_text, architecture, "architecture")?;
    if !release_architecture.eq_ignore_ascii_case(architecture) {
        return Err(AppError::Engine(format!(
            "release-manifest 的架构与当前目标不一致：{release_architecture}"
        )));
    }
    let github_digest = normalize_digest(descriptor.assets.windows_msix.github_digest.as_deref())?;
    let msix_url = relay_asset_url(&base_url, &msix_path)?;
    let checksums_url = relay_asset_url(&base_url, &checksums_path)?;
    let identity = CodexPayloadIdentity {
        source_kind: CodexPayloadSourceKind::Relay,
        source_locator: base_url.clone(),
        repo,
        release_id: descriptor.release_id,
        release_tag: descriptor.tag,
        msix_asset_id: None,
        msix_asset_name: msix_path.clone(),
        sha256_asset_id: None,
        sha256_asset_name: checksums_path.clone(),
        manifest_asset_id: None,
        manifest_asset_name: manifest_path.clone(),
        package_moniker,
        architecture: architecture.to_string(),
        msix_version,
        app_version: app_version.clone(),
        msix_size: descriptor.assets.windows_msix.size,
        msix_sha256: Some(msix_sha.clone()),
        checksums_sha256: Some(checksums_sha.clone()),
        manifest_sha256: Some(manifest_sha.clone()),
        github_digest,
    };
    Ok(ResolvedRelease {
        identity,
        app_version,
        msix: ResolvedAsset {
            size: descriptor.assets.windows_msix.size,
            url: msix_url,
            expected_sha256: Some(msix_sha),
        },
        checksums: ResolvedAsset {
            size: descriptor.assets.checksums.size,
            url: checksums_url,
            expected_sha256: Some(checksums_sha),
        },
    })
}

fn resolve_source(
    settings: &AppSettings,
    architecture: &str,
    network: &NetworkConfig,
    source_kind: CodexPayloadSourceKind,
) -> Result<ResolvedRelease, AppError> {
    match source_kind {
        CodexPayloadSourceKind::Mirror => {
            let mirror_fetch = |url: &str| {
                require_https_url(url, "Codex 镜像地址")?;
                let result = if is_builtin_codex_mirror_base(&settings.codex_payload_base_url) {
                    // The public mirror may hand off metadata to its domestic
                    // object-storage backend. The base is fixed and the
                    // downloaded package is still bound by Authenticode and
                    // the mirror checksum, so following this trusted mirror
                    // handoff is intentional.
                    fetch_text_with_network(url, network)
                } else {
                    fetch_text_with_network_no_redirect(url, network)
                };
                result.map_err(|error| AppError::Engine(error.to_string()))
            };
            resolve_mirror_latest(
                &mirror_fetch,
                &settings.codex_payload_base_url,
                architecture,
            )
        }
        CodexPayloadSourceKind::Relay => {
            let relay_fetch = |url: &str| {
                require_https_url(url, "Codex relay 地址")?;
                // The relay descriptor and manifest are trust-bound metadata;
                // do not let an HTTP redirect move the updater to a different
                // origin. Relay asset URLs are derived from the same base and
                // are downloaded with the matching no-redirect policy below.
                fetch_text_with_network_no_redirect(url, network)
                    .map_err(|error| AppError::Engine(error.to_string()))
            };
            let relay_base = relay_base_url_for(settings);
            resolve_relay_latest(&relay_fetch, &relay_base, architecture)
        }
        CodexPayloadSourceKind::Github => {
            let api_fetch = |url: &str| {
                fetch_text_with_network_headers(url, network, GITHUB_API_HEADERS)
                    .map_err(|error| AppError::Engine(error.to_string()))
            };
            let asset_fetch = |url: &str| {
                require_https_url(url, "GitHub 资产下载地址")?;
                fetch_text_with_network(url, network)
                    .map_err(|error| AppError::Engine(error.to_string()))
            };
            let repo = resolved_repo(settings)?;
            resolve_latest(&api_fetch, &asset_fetch, &repo, architecture)
        }
        CodexPayloadSourceKind::Auto => {
            Err(AppError::Internal("resolve_source 不接受 auto".to_string()))
        }
    }
}

fn source_locator_for(settings: &AppSettings, source_kind: CodexPayloadSourceKind) -> String {
    match source_kind {
        CodexPayloadSourceKind::Mirror => settings.codex_payload_base_url.clone(),
        CodexPayloadSourceKind::Relay => relay_base_url_for(settings),
        CodexPayloadSourceKind::Github => resolved_repo(settings)
            .map(|repo| repo_api_url(&repo))
            .unwrap_or_else(|_| "https://api.github.com".to_string()),
        CodexPayloadSourceKind::Auto => "auto".to_string(),
    }
}

/// Source failures are deliberately reduced to stable categories before they
/// are put in a log or returned as an aggregate error.  Resolver errors may
/// contain URLs from a configured source, so the raw message must not cross
/// that boundary.
fn source_error_category(error: &AppError) -> &'static str {
    let message = error.to_string();
    if message.contains("source_unreachable:") {
        return "source_unreachable";
    }
    if message.contains("source_invalid:") {
        return "source_invalid";
    }
    match crate::errors::classify(&message) {
        crate::errors::ErrorKind::Network | crate::errors::ErrorKind::Timeout => {
            "source_unreachable"
        }
        _ => "source_invalid",
    }
}

fn source_error_detail(source_kind: CodexPayloadSourceKind, error: &AppError) -> String {
    format!(
        "source={} reason={} code={}",
        source_kind.as_str(),
        source_error_category(error),
        error.code()
    )
}

fn source_error_for_caller(source_kind: CodexPayloadSourceKind, error: &AppError) -> AppError {
    let category = source_error_category(error);
    AppError::Engine(format!(
        "{category}: Codex {} source metadata resolution failed (code={})",
        source_kind.as_str(),
        error.code()
    ))
}

fn redacted_source_locator(_locator: &str) -> &'static str {
    "<redacted>"
}

fn source_version_reason(current: &str, latest: &str) -> Option<&'static str> {
    if !is_numeric_version(current) || !is_numeric_version(latest) {
        return Some("source_invalid");
    }
    match compare_versions(current, latest) {
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => Some("source_no_newer"),
        std::cmp::Ordering::Greater => Some("source_stale"),
    }
}

fn source_version_error(
    source_kind: CodexPayloadSourceKind,
    current: &str,
    latest: &str,
) -> AppError {
    let reason = source_version_reason(current, latest).unwrap_or("source_invalid");
    AppError::Engine(format!(
        "{reason}: Codex {} source does not provide a newer release",
        source_kind.as_str()
    ))
}

fn validate_resolved_release_versions(release: &ResolvedRelease) -> Result<(), AppError> {
    if !is_numeric_version(&release.app_version)
        || !is_numeric_version(&release.identity.app_version)
    {
        return Err(AppError::Engine(
            "source_invalid: release appVersion 必须是点分非负整数版本".to_string(),
        ));
    }
    if !is_numeric_version(&release.identity.msix_version) {
        return Err(AppError::Engine(
            "source_invalid: MSIX package version 必须是点分非负整数版本".to_string(),
        ));
    }
    Ok(())
}

fn source_error_is_retryable(error: &AppError) -> bool {
    matches!(
        crate::errors::classify(&error.to_string()),
        crate::errors::ErrorKind::Network | crate::errors::ErrorKind::Timeout
    )
}

fn resolve_source_with_retry<F>(
    settings: &AppSettings,
    architecture: &str,
    network: &NetworkConfig,
    source_kind: CodexPayloadSourceKind,
    resolve: &mut F,
) -> Result<ResolvedRelease, AppError>
where
    F: FnMut(
        &AppSettings,
        &str,
        &NetworkConfig,
        CodexPayloadSourceKind,
    ) -> Result<ResolvedRelease, AppError>,
{
    let locator = redacted_source_locator(&source_locator_for(settings, source_kind));
    let mut last_error = None;
    for attempt in 1..=SOURCE_METADATA_ATTEMPTS {
        log::info!(
            "codex payload source attempt source={} attempt={}/{} locator={}",
            source_kind.as_str(),
            attempt,
            SOURCE_METADATA_ATTEMPTS,
            locator
        );
        match resolve(settings, architecture, network, source_kind) {
            Ok(release) => {
                validate_resolved_release_versions(&release)?;
                log::info!(
                    "codex payload source resolved source={} version={} package={} locator={}",
                    source_kind.as_str(),
                    release.app_version,
                    release.identity.package_moniker,
                    locator
                );
                return Ok(release);
            }
            Err(error) => {
                let retryable = source_error_is_retryable(&error);
                log::warn!(
                    "codex payload source failed stage=metadata source={} attempt={}/{} retryable={} locator={} reason={} code={}",
                    source_kind.as_str(),
                    attempt,
                    SOURCE_METADATA_ATTEMPTS,
                    retryable,
                    locator,
                    source_error_category(&error),
                    error.code()
                );
                last_error = Some(error);
                if !retryable || attempt == SOURCE_METADATA_ATTEMPTS {
                    break;
                }
                std::thread::sleep(Duration::from_millis(
                    SOURCE_METADATA_RETRY_DELAY_MS * attempt as u64,
                ));
            }
        }
    }
    Err(last_error
        .as_ref()
        .map(|error| source_error_for_caller(source_kind, error))
        .unwrap_or_else(|| {
            AppError::Engine(format!(
                "source_invalid: Codex {} source did not return metadata",
                source_kind.as_str()
            ))
        }))
}

fn relay_base_url_for(settings: &AppSettings) -> String {
    if is_builtin_codex_mirror_base(&settings.codex_payload_base_url) {
        LEGACY_CODEX_PAYLOAD_BASE_URL.to_string()
    } else {
        settings.codex_payload_base_url.clone()
    }
}

fn resolve_preferred_source(
    settings: &AppSettings,
    architecture: &str,
    network: &NetworkConfig,
    current_version: &str,
) -> Result<Option<ResolvedRelease>, AppError> {
    resolve_preferred_source_with(
        settings,
        architecture,
        network,
        current_version,
        |settings, architecture, network, source_kind| {
            resolve_source(settings, architecture, network, source_kind)
        },
    )
}

fn resolve_preferred_source_with<F>(
    settings: &AppSettings,
    architecture: &str,
    network: &NetworkConfig,
    current_version: &str,
    mut resolve: F,
) -> Result<Option<ResolvedRelease>, AppError>
where
    F: FnMut(
        &AppSettings,
        &str,
        &NetworkConfig,
        CodexPayloadSourceKind,
    ) -> Result<ResolvedRelease, AppError>,
{
    match settings.codex_payload_source_kind {
        CodexPayloadSourceKind::Auto => {
            let mut source_errors = false;
            let mut valid_source = false;
            let mut diagnostics = Vec::new();
            for source_kind in AUTO_SOURCE_ORDER {
                match resolve_source_with_retry(
                    settings,
                    architecture,
                    network,
                    source_kind,
                    &mut resolve,
                ) {
                    Ok(release) => {
                        // A successfully parsed release is valid evidence even
                        // when it is equal to or older than the local version.
                        // A later fallback failure must not erase that evidence.
                        valid_source = true;
                        if let Some(reason) =
                            source_version_reason(current_version, &release.app_version)
                        {
                            log::warn!(
                                "codex payload source skipped source={} reason={}",
                                source_kind.as_str(),
                                reason
                            );
                            diagnostics.push(format!(
                                "source={} reason={}",
                                source_kind.as_str(),
                                reason
                            ));
                            continue;
                        }
                        log::info!(
                            "codex payload source selected source={} reason=newer",
                            source_kind.as_str()
                        );
                        return Ok(Some(release));
                    }
                    Err(error) => {
                        source_errors = true;
                        let detail = source_error_detail(source_kind, &error);
                        log::warn!("codex payload source skipped {detail}");
                        diagnostics.push(detail);
                    }
                }
            }
            log::info!(
                "codex payload auto source exhausted result={} diagnostics={}",
                if valid_source || !source_errors {
                    "no-update"
                } else {
                    "error"
                },
                diagnostics.join(";")
            );
            if !valid_source && source_errors {
                Err(AppError::Engine(format!(
                    "Codex 更新源均不可用；{}",
                    diagnostics.join("；")
                )))
            } else {
                Ok(None)
            }
        }
        source_kind => {
            let release = resolve_source_with_retry(
                settings,
                architecture,
                network,
                source_kind,
                &mut resolve,
            )?;
            if let Some(reason) = source_version_reason(current_version, &release.app_version) {
                log::warn!(
                    "codex payload source rejected source={} reason={}",
                    source_kind.as_str(),
                    reason
                );
                return Err(source_version_error(
                    source_kind,
                    current_version,
                    &release.app_version,
                ));
            }
            Ok(Some(release))
        }
    }
}

fn source_matches_settings(
    settings: &AppSettings,
    identity: &CodexPayloadIdentity,
) -> Result<bool, AppError> {
    let allowed = match settings.codex_payload_source_kind {
        CodexPayloadSourceKind::Auto => true,
        configured => configured == identity.source_kind,
    };
    if !allowed {
        return Ok(false);
    }
    let locator = match identity.source_kind {
        CodexPayloadSourceKind::Mirror => {
            validate_relay_base_url(&settings.codex_payload_base_url)?
        }
        CodexPayloadSourceKind::Relay => validate_relay_base_url(&relay_base_url_for(settings))?,
        CodexPayloadSourceKind::Github => repo_api_url(&resolved_repo(settings)?),
        CodexPayloadSourceKind::Auto => return Ok(false),
    };
    Ok(identity.source_locator == locator)
}

fn read_json(path: &Path, label: &str) -> Result<serde_json::Value, AppError> {
    let bytes = fs::read(path).map_err(|error| {
        AppError::Engine(format!("读取 {label} 失败 ({}): {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Engine(format!("解析 {label} 失败 ({}): {error}", path.display()))
    })
}

fn exact_u64(value: Option<&serde_json::Value>, label: &str) -> Result<u64, AppError> {
    value
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::Engine(format!("{label} 必须是整数")))
}

fn normalize_relative_path(raw: &str) -> Result<String, AppError> {
    let value = raw.trim().replace('\\', "/");
    if value.is_empty() || value.starts_with('/') || value.contains(':') {
        return Err(AppError::Engine(format!("非法 bundle 相对路径：{raw}")));
    }
    let parts: Vec<&str> = value.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(AppError::Engine(format!("非法 bundle 相对路径：{raw}")));
    }
    Ok(parts.join("/"))
}

fn reject_reparse(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::Engine(format!("读取路径属性失败 ({}): {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Engine(format!(
            "bundle 路径不得是符号链接：{}",
            path.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(AppError::Engine(format!(
                "bundle 路径不得是 reparse point：{}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Reject links/reparse points anywhere in an existing path prefix. Checking
/// only the leaf is insufficient: a junction at `payload/` could redirect a
/// later rename to another volume while the leaf itself remains ordinary.
fn reject_reparse_chain(path: &Path) -> Result<(), AppError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match fs::symlink_metadata(candidate) {
            Ok(_) => reject_reparse(candidate)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Engine(format!(
                    "读取路径属性失败 ({}): {error}",
                    candidate.display()
                )))
            }
        }
        current = candidate.parent();
    }
    Ok(())
}

fn regular_file_exists(path: &Path) -> Result<bool, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_reparse(path)?;
            if !metadata.file_type().is_file() {
                return Err(AppError::Engine(format!(
                    "路径必须是普通文件：{}",
                    path.display()
                )));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::Engine(format!(
            "读取文件属性失败 ({}): {error}",
            path.display()
        ))),
    }
}

fn require_sha256(value: Option<&serde_json::Value>, label: &str) -> Result<String, AppError> {
    let value = value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| AppError::Engine(format!("{label} 必须是 64 位 SHA256")))?;
    Ok(value.to_ascii_lowercase())
}

fn require_capability(bundle_root: &Path) -> Result<LocalBundle, AppError> {
    require_capability_impl(bundle_root, true, false)
}

/// Read and validate the immutable capability metadata without doing the
/// expensive payload hash.  Update check/apply use this before resolving the
/// remote source so a stale or unreachable source does not wait behind a
/// removable-drive full-file read.  The caller must perform the full
/// SHA256SUMS verification before treating the bundle as trusted.
fn require_capability_metadata(bundle_root: &Path) -> Result<LocalBundle, AppError> {
    require_capability_impl(bundle_root, false, false)
}

/// Read capability metadata for the single authenticated reactivation path.
/// The only relaxed condition is that the manifest-declared MSIX file may be
/// absent; all manifest fields and the exact path remain validated.
fn require_capability_metadata_for_reinstall(bundle_root: &Path) -> Result<LocalBundle, AppError> {
    require_capability_impl(bundle_root, false, true)
}

fn require_capability_impl(
    bundle_root: &Path,
    verify_payload_hash: bool,
    allow_missing_payload: bool,
) -> Result<LocalBundle, AppError> {
    reject_reparse(bundle_root)?;
    for name in [ROOT_MANIFEST_NAME, USB_MANIFEST_NAME, SHA256SUMS_NAME] {
        reject_reparse_chain(&bundle_root.join(name))?;
    }
    let manifest = read_json(&bundle_root.join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let usb_manifest = read_json(&bundle_root.join(USB_MANIFEST_NAME), USB_MANIFEST_NAME)?;
    let _manifest_schema = require_supported_manifest_schemas(&manifest, &usb_manifest)?;
    let root_capability = manifest
        .get("updateCapability")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::Engine("root manifest 缺少 updateCapability".to_string()))?;
    if root_capability.get("codexPayload") != Some(&serde_json::Value::Bool(true))
        || exact_u64(
            root_capability.get("schemaVersion"),
            "root updateCapability.schemaVersion",
        )? != 3
        || exact_u64(
            root_capability.get("anchorSchema"),
            "root updateCapability.anchorSchema",
        )? != 3
    {
        return Err(AppError::Engine(
            "root manifest 的 Codex 更新能力声明不受支持".to_string(),
        ));
    }
    let capability = usb_manifest
        .get("updateCapability")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::Engine("U 盘包缺少 updateCapability".to_string()))?;
    if capability.get("codexPayload") != Some(&serde_json::Value::Bool(true))
        || exact_u64(
            capability.get("schemaVersion"),
            "updateCapability.schemaVersion",
        )? != 3
        || exact_u64(
            capability.get("anchorSchema"),
            "usb updateCapability.anchorSchema",
        )? != 3
        || capability.get("checksumFile").and_then(|v| v.as_str()) != Some(SHA256SUMS_NAME)
        || capability
            .get("transactionDirectory")
            .and_then(|v| v.as_str())
            != Some(STAGING_DIR)
        || capability.get("backupPolicy").and_then(|v| v.as_str())
            != Some("single-msix-listed-in-SHA256SUMS")
        || capability.get("successState").and_then(|v| v.as_str()) != Some("clean")
    {
        return Err(AppError::Engine(
            "U 盘包的 Codex 更新能力声明不受支持，请重建并重新授权".to_string(),
        ));
    }
    let payload_relative = normalize_relative_path(
        manifest
            .pointer("/payload/path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Engine("root manifest 缺少 payload.path".to_string()))?,
    )?;
    if !payload_relative.to_ascii_lowercase().ends_with(".msix") {
        return Err(AppError::Engine("payload.path 必须指向 .msix".to_string()));
    }
    let payload_object = manifest
        .get("payload")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::Engine("root manifest 缺少 payload 对象".to_string()))?;
    let manifest_payload_sha =
        require_sha256(payload_object.get("sha256"), "manifest.payload.sha256")?;
    let manifest_payload_size = exact_u64(payload_object.get("size"), "manifest.payload.size")?;
    let payload_dir = payload_relative
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default();
    let expected_backup = if payload_dir.is_empty() {
        "_backup".to_string()
    } else {
        format!("{payload_dir}/_backup")
    };
    if capability.get("backupDirectory").and_then(|v| v.as_str()) != Some(expected_backup.as_str())
    {
        return Err(AppError::Engine(
            "U 盘包的 backupDirectory 与 payload.path 不一致".to_string(),
        ));
    }
    let architecture = manifest
        .get("architecture")
        .and_then(serde_json::Value::as_str)
        .filter(|value| matches!(*value, "x64" | "arm64"))
        .ok_or_else(|| AppError::Engine("root manifest.architecture 无效".to_string()))?
        .to_string();
    // The stable anchor has a mutable backup marker, but the physical backup
    // slot still has a strict shape. Enforce it in the cheap capability path
    // so check cannot offer an update that apply will reject later.
    crate::app::usb_bundle::validate_backup_layout(bundle_root, &payload_relative, &architecture)?;
    if usb_manifest
        .pointer("/codexInstaller/payload")
        .and_then(serde_json::Value::as_str)
        != Some(payload_relative.as_str())
    {
        return Err(AppError::Engine(
            "usb-manifest 的 Codex payload 路径与 root manifest 不一致".to_string(),
        ));
    }
    let payload_path =
        bundle_root.join(payload_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    reject_reparse_chain(&payload_path)?;
    let payload_exists = regular_file_exists(&payload_path)?;
    if !payload_exists && !(allow_missing_payload && !payload_path.exists()) {
        return Err(AppError::Engine(
            "Codex payload 不存在或不是普通文件".to_string(),
        ));
    }
    if payload_exists {
        let actual_payload_size = fs::metadata(&payload_path)
            .map_err(|error| AppError::Engine(format!("读取 payload 大小失败: {error}")))?
            .len();
        if actual_payload_size != manifest_payload_size {
            return Err(AppError::Engine(
                "当前 payload 大小与 manifest 不一致".to_string(),
            ));
        }
        if verify_payload_hash {
            let actual_payload_sha = sha256_file(&payload_path).map_err(|error| {
                AppError::Engine(format!("计算当前 payload SHA256 失败: {error}"))
            })?;
            if !actual_payload_sha.eq_ignore_ascii_case(&manifest_payload_sha) {
                return Err(AppError::Engine(
                    "当前 payload SHA256 与 manifest 不一致".to_string(),
                ));
            }
        }
    }
    let installer = usb_manifest
        .pointer("/codexInstaller")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::Engine("usb-manifest 缺少 codexInstaller".to_string()))?;
    let usb_payload_sha = require_sha256(
        installer.get("payloadSha256"),
        "usb-manifest.codexInstaller.payloadSha256",
    )?;
    let usb_payload_size = exact_u64(
        installer.get("payloadSize"),
        "usb-manifest.codexInstaller.payloadSize",
    )?;
    if manifest_payload_sha != usb_payload_sha || manifest_payload_size != usb_payload_size {
        return Err(AppError::Engine(
            "root manifest 与 usb-manifest 的 payload 元数据不一致".to_string(),
        ));
    }
    let current_version = read_usb_codex_desktop_version(bundle_root)?;
    Ok(LocalBundle {
        manifest,
        usb_manifest,
        payload_path,
        payload_relative,
        architecture,
        current_version,
    })
}

fn require_supported_manifest_schemas(
    root: &serde_json::Value,
    usb: &serde_json::Value,
) -> Result<u64, AppError> {
    let root_schema = exact_u64(root.get("schemaVersion"), "root manifest.schemaVersion")?;
    let usb_schema = exact_u64(usb.get("schemaVersion"), "usb-manifest.schemaVersion")?;
    if !matches!(root_schema, 2 | 3) || root_schema != usb_schema {
        return Err(AppError::Engine(
            "root and USB manifest schema versions do not match or are unsupported".to_string(),
        ));
    }
    if root_schema == 3
        && root
            .pointer("/revocationPolicy/registrationRequired")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(AppError::Engine(
            "schema 3 root manifest must require registration".to_string(),
        ));
    }
    Ok(root_schema)
}

#[cfg(test)]
fn local_bundle_snapshot(
    bundle_root: &Path,
    local: &LocalBundle,
) -> Result<LocalBundleSnapshot, AppError> {
    let root_manifest = bundle_root.join(ROOT_MANIFEST_NAME);
    let usb_manifest = bundle_root.join(USB_MANIFEST_NAME);
    let checksums = bundle_root.join(SHA256SUMS_NAME);
    for path in [
        &root_manifest,
        &usb_manifest,
        &checksums,
        &local.payload_path,
    ] {
        reject_reparse_chain(path)?;
    }
    let digest = |path: &Path, label: &str| {
        sha256_file(path)
            .map_err(|error| AppError::Engine(format!("计算 {label} SHA256 失败: {error}")))
    };
    Ok(LocalBundleSnapshot {
        current_version: local.current_version.clone(),
        payload_relative: local.payload_relative.clone(),
        architecture: local.architecture.clone(),
        payload_sha256: digest(&local.payload_path, "payload")?,
        root_manifest_sha256: digest(&root_manifest, ROOT_MANIFEST_NAME)?,
        usb_manifest_sha256: digest(&usb_manifest, USB_MANIFEST_NAME)?,
        checksums_sha256: digest(&checksums, SHA256SUMS_NAME)?,
    })
}

/// Build a local snapshot from the digest table produced by the immediately
/// preceding full SHA256SUMS verification.  This intentionally does not
/// accept a digest from a previous check/apply invocation.
fn local_bundle_snapshot_from_digests(
    bundle_root: &Path,
    local: &LocalBundle,
    digests: &BTreeMap<String, String>,
) -> Result<LocalBundleSnapshot, AppError> {
    let root_manifest = bundle_root.join(ROOT_MANIFEST_NAME);
    let usb_manifest = bundle_root.join(USB_MANIFEST_NAME);
    let checksums = bundle_root.join(SHA256SUMS_NAME);
    for path in [
        &root_manifest,
        &usb_manifest,
        &checksums,
        &local.payload_path,
    ] {
        reject_reparse_chain(path)?;
    }
    Ok(LocalBundleSnapshot {
        current_version: local.current_version.clone(),
        payload_relative: local.payload_relative.clone(),
        architecture: local.architecture.clone(),
        payload_sha256: verified_digest(digests, &local.payload_relative, "payload")?,
        root_manifest_sha256: verified_digest(digests, ROOT_MANIFEST_NAME, ROOT_MANIFEST_NAME)?,
        usb_manifest_sha256: verified_digest(digests, USB_MANIFEST_NAME, USB_MANIFEST_NAME)?,
        // SHA256SUMS must not list itself, so it cannot come from the digest
        // table and is intentionally read once as a small metadata file.
        checksums_sha256: sha256_file(&checksums).map_err(|error| {
            AppError::Engine(format!("计算 {SHA256SUMS_NAME} SHA256 失败: {error}"))
        })?,
    })
}

fn verified_digest(
    digests: &BTreeMap<String, String>,
    relative: &str,
    label: &str,
) -> Result<String, AppError> {
    digests
        .get(relative)
        .cloned()
        .or_else(|| {
            digests
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(relative))
                .map(|(_, digest)| digest.clone())
        })
        .ok_or_else(|| {
            AppError::Engine(format!("完整性校验未返回 {label} 的摘要，拒绝生成本地快照"))
        })
}

#[cfg(test)]
fn codex_update_payload(
    bundle_root: &Path,
    local: &LocalBundle,
    identity: &CodexPayloadIdentity,
) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(CodexUpdatePayload {
        identity: identity.clone(),
        local_snapshot: Some(local_bundle_snapshot(bundle_root, local)?),
    })
    .map_err(|error| AppError::Engine(format!("序列化 codex 更新快照失败: {error}")))
}

fn codex_update_payload_with_digests(
    bundle_root: &Path,
    local: &LocalBundle,
    identity: &CodexPayloadIdentity,
    digests: &BTreeMap<String, String>,
) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(CodexUpdatePayload {
        identity: identity.clone(),
        local_snapshot: Some(local_bundle_snapshot_from_digests(
            bundle_root,
            local,
            digests,
        )?),
    })
    .map_err(|error| AppError::Engine(format!("序列化 codex 更新快照失败: {error}")))
}

fn parse_codex_update_payload(value: &serde_json::Value) -> Result<CodexUpdatePayload, AppError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        AppError::StaleExpectation(format!("更新确认对象无效，请重新检查：{error}"))
    })
}

fn verify_bundle_integrity_with_digests(
    bundle_root: &Path,
) -> Result<BTreeMap<String, String>, AppError> {
    let sums_path = bundle_root.join(SHA256SUMS_NAME);
    reject_reparse_chain(&sums_path)?;
    let sums = fs::read(&sums_path)
        .map_err(|error| AppError::Engine(format!("读取 {SHA256SUMS_NAME} 失败: {error}")))?;
    verify_sha256_sums_with_digests(bundle_root, &sums, &BTreeMap::new())
}

#[cfg(test)]
fn require_local_snapshot(
    bundle_root: &Path,
    local: &LocalBundle,
    expected: &LocalBundleSnapshot,
) -> Result<(), AppError> {
    let actual = local_bundle_snapshot(bundle_root, local)?;
    if &actual != expected {
        return Err(AppError::StaleExpectation(
            "本地 USB payload 或清单在确认后发生变化，请重新检查后再确认。".to_string(),
        ));
    }
    Ok(())
}

fn require_local_snapshot_from_digests(
    bundle_root: &Path,
    local: &LocalBundle,
    expected: &LocalBundleSnapshot,
    digests: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    let actual = local_bundle_snapshot_from_digests(bundle_root, local, digests)?;
    if &actual != expected {
        return Err(AppError::StaleExpectation(
            "本地 USB payload 或清单在确认后发生变化，请重新检查后再确认。".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn pending_transaction_dirs(bundle_root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let staging = bundle_root.join(STAGING_DIR);
    if !staging.exists() {
        return Ok(Vec::new());
    }
    reject_reparse(&staging)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(&staging)
        .map_err(|error| AppError::Engine(format!("读取事务目录失败: {error}")))?
    {
        let entry = entry.map_err(|error| AppError::Engine(format!("读取事务项失败: {error}")))?;
        if entry
            .file_type()
            .map_err(|error| AppError::Engine(error.to_string()))?
            .is_dir()
        {
            reject_reparse_chain(&entry.path())?;
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with(TRANSACTION_PREFIX)
            {
                return Err(AppError::Engine("发现未知事务目录，拒绝继续".to_string()));
            }
            result.push(entry.path());
        } else if entry
            .file_type()
            .map_err(|error| AppError::Engine(error.to_string()))?
            .is_file()
            && parse_owner_sidecar_name(&entry.file_name().to_string_lossy()).is_some()
        {
            reject_reparse_chain(&entry.path())?;
            let bytes = read_bounded_transaction_state_file(&entry.path())?
                .ok_or_else(|| AppError::Engine("owner sidecar 在读取期间消失".to_string()))?;
            let owner: TransactionOwner = serde_json::from_slice(&bytes)
                .map_err(|error| AppError::Engine(format!("事务 owner sidecar 无效：{error}")))?;
            let sidecar_id = parse_owner_sidecar_name(&entry.file_name().to_string_lossy())
                .expect("checked exact owner sidecar name");
            if owner.transaction_id != sidecar_id {
                return Err(AppError::Engine(
                    "事务 owner sidecar 与其文件名 ID 不一致".to_string(),
                ));
            }
        } else {
            return Err(AppError::Engine(
                "事务目录包含未知文件，拒绝继续".to_string(),
            ));
        }
    }
    Ok(result)
}

fn owner_sidecar_name(transaction_id: &str) -> String {
    format!("{OWNER_SIDECAR_PREFIX}{transaction_id}{OWNER_SIDECAR_SUFFIX}")
}

fn owner_sidecar_path(staging: &Path, transaction_id: &str) -> PathBuf {
    staging.join(owner_sidecar_name(transaction_id))
}

fn parse_owner_sidecar_name(name: &str) -> Option<String> {
    let transaction_id = name
        .strip_prefix(OWNER_SIDECAR_PREFIX)?
        .strip_suffix(OWNER_SIDECAR_SUFFIX)?;
    let parsed = uuid::Uuid::parse_str(transaction_id).ok()?;
    (parsed.to_string() == transaction_id).then(|| transaction_id.to_string())
}

fn write_owner_sidecar(staging: &Path, owner: &TransactionOwner) -> Result<(), AppError> {
    let path = owner_sidecar_path(staging, &owner.transaction_id);
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(AppError::Engine(
                "事务 owner sidecar 已存在，拒绝覆盖".to_string(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AppError::Engine(error.to_string())),
    }
    let bytes = serde_json::to_vec(owner)
        .map_err(|error| AppError::Engine(format!("序列化事务 owner sidecar 失败：{error}")))?;
    if bytes.len() as u64 > MAX_TRANSACTION_STATE_BYTES {
        return Err(AppError::Engine(
            "事务 owner sidecar 超过安全大小上限".to_string(),
        ));
    }
    crate::app::atomic_file::write_atomic(&path, &bytes)
        .map_err(|error| AppError::Engine(format!("写入事务 owner sidecar 失败：{error}")))?;
    let readback = read_bounded_transaction_state_file(&path)?
        .ok_or_else(|| AppError::Engine("事务 owner sidecar 写入后缺失".to_string()))?;
    let verified: TransactionOwner = serde_json::from_slice(&readback)
        .map_err(|error| AppError::Engine(format!("事务 owner sidecar 回读失败：{error}")))?;
    if verified != *owner {
        return Err(AppError::Engine(
            "事务 owner sidecar 回读身份不匹配".to_string(),
        ));
    }
    let backup = crate::app::atomic_file::backup_path(&path);
    if backup.exists() {
        reject_reparse_chain(&backup)?;
        fs::remove_file(backup).map_err(|error| {
            AppError::Engine(format!("清理事务 owner sidecar 备份失败：{error}"))
        })?;
    }
    Ok(())
}

fn remove_owner_sidecar_after_transaction(
    bundle_root: &Path,
    transaction_id: &str,
    expected_owner: Option<&TransactionOwner>,
) -> Result<(), AppError> {
    let staging = bundle_root.join(STAGING_DIR);
    let transaction = staging.join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
    if transaction.exists() {
        return Err(AppError::Engine(
            "Codex 更新事务目录仍存在；保留 owner sidecar 供重试".to_string(),
        ));
    }
    let sidecar = owner_sidecar_path(&staging, transaction_id);
    let Some(bytes) = read_bounded_transaction_state_file(&sidecar)? else {
        return Ok(());
    };
    let actual: TransactionOwner = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Engine(format!("事务 owner sidecar 无效：{error}")))?;
    if actual.transaction_id != transaction_id
        || expected_owner.is_some_and(|expected| expected != &actual)
    {
        return Err(AppError::Engine(
            "事务 owner sidecar 与已完成事务身份不匹配；保留供重试".to_string(),
        ));
    }
    reject_reparse_chain(&sidecar)?;
    fs::remove_file(&sidecar)
        .map_err(|error| AppError::Engine(format!("清理事务 owner sidecar 失败：{error}")))?;
    Ok(())
}

/// Return only update transactions whose bounded owner marker is bound to the
/// currently validated USB license and fingerprint. Revocation cleanup uses
/// this stricter inventory; ordinary updater recovery keeps the legacy v1
/// behavior above.
pub(crate) fn revocation_transaction_dirs(
    bundle_root: &Path,
    license: &crate::app::usb_guard::UsbLicense,
    fingerprint: &crate::app::usb_guard::UsbFingerprint,
    pinned_root: &crate::app::uninstall_cleanup::RevocationCleanupRoot,
) -> Result<Vec<crate::app::uninstall_cleanup::PinnedRevocationTransaction>, AppError> {
    pinned_root
        .validate_identity(bundle_root, fingerprint)
        .map_err(AppError::Engine)?;
    if !codex_usb_license::fingerprints_match(fingerprint, &license.fingerprint) {
        return Err(AppError::Engine(
            "Codex 更新事务的 USB 指纹与许可证不一致".to_string(),
        ));
    }
    let expected_license_sha = codex_usb_license::license_signature_sha256(license)
        .map_err(|error| AppError::Engine(format!("计算许可证签名摘要失败：{error}")))?;
    let expected_fingerprint_sha = codex_usb_license::fingerprint_digest(fingerprint)
        .map_err(|error| AppError::Engine(format!("计算 USB 指纹摘要失败：{error}")))?;
    let Some(staging) = pinned_root
        .open_pinned_entry(STAGING_DIR)
        .map_err(AppError::Engine)?
    else {
        return Ok(Vec::new());
    };
    if !staging.is_directory().map_err(AppError::Engine)? {
        return Err(AppError::Engine("U 盘事务路径不是目录".to_string()));
    }
    let names = staging.list_names().map_err(AppError::Engine)?;
    if names.len() > 512 {
        return Err(AppError::Engine("U 盘事务项目超过数量上限".to_string()));
    }
    let (transaction_ids, sidecar_ids) = inventory_revocation_staging_names(&names)?;

    let mut result = Vec::new();
    for transaction_id in transaction_ids.iter() {
        let dir_name = format!("{TRANSACTION_PREFIX}{transaction_id}");
        let sidecar_name = owner_sidecar_name(transaction_id);
        let sidecar = staging
            .open_child(&sidecar_name)
            .map_err(AppError::Engine)?
            .ok_or_else(|| AppError::Engine("事务 owner sidecar 在核验期间消失".to_string()))?;
        if sidecar.is_directory().map_err(AppError::Engine)? {
            return Err(AppError::Engine(
                "事务 owner sidecar 不能是目录".to_string(),
            ));
        }
        let owner_bytes = sidecar
            .read_bounded(MAX_TRANSACTION_STATE_BYTES as usize)
            .map_err(AppError::Engine)?;
        let owner: TransactionOwner = serde_json::from_slice(&owner_bytes)
            .map_err(|error| AppError::Engine(format!("事务 owner sidecar 无效：{error}")))?;
        if !transaction_owner_matches(
            &owner,
            transaction_id,
            &license.installation_id,
            &expected_license_sha,
            &expected_fingerprint_sha,
        ) {
            return Err(AppError::Engine(format!(
                "事务 owner sidecar 与当前 USB 身份不匹配：{dir_name}"
            )));
        }
        let tx_entry = staging
            .open_child(&dir_name)
            .map_err(AppError::Engine)?
            .ok_or_else(|| AppError::Engine("Codex 更新事务在核验期间消失".to_string()))?;
        if !tx_entry.is_directory().map_err(AppError::Engine)? {
            return Err(AppError::Engine("Codex 更新事务不是目录".to_string()));
        }
        let tree = tx_entry.prepare_delete_tree().map_err(AppError::Engine)?;
        validate_pinned_transaction_state(&tree, &owner)?;
        let relative = format!("{STAGING_DIR}/{dir_name}");
        let staging_ancestor = staging.duplicate().map_err(AppError::Engine)?;
        result.push(
            crate::app::uninstall_cleanup::RevocationCleanupRoot::prepare_pinned_transaction(
                relative,
                Some(tree),
                sidecar,
                staging_ancestor,
            ),
        );
    }

    for transaction_id in sidecar_ids.difference(&transaction_ids) {
        let sidecar_name = owner_sidecar_name(transaction_id);
        let sidecar = staging
            .open_child(&sidecar_name)
            .map_err(AppError::Engine)?
            .ok_or_else(|| AppError::Engine("孤儿事务 sidecar 在核验期间消失".to_string()))?;
        if sidecar.is_directory().map_err(AppError::Engine)? {
            return Err(AppError::Engine(
                "事务 owner sidecar 不能是目录".to_string(),
            ));
        }
        let owner_bytes = sidecar
            .read_bounded(MAX_TRANSACTION_STATE_BYTES as usize)
            .map_err(AppError::Engine)?;
        let owner: TransactionOwner = serde_json::from_slice(&owner_bytes)
            .map_err(|error| AppError::Engine(format!("孤儿事务 owner sidecar 无效：{error}")))?;
        if !transaction_owner_matches(
            &owner,
            transaction_id,
            &license.installation_id,
            &expected_license_sha,
            &expected_fingerprint_sha,
        ) {
            return Err(AppError::Engine(format!(
                "孤儿事务 owner sidecar 与当前 USB 身份不匹配：{sidecar_name}"
            )));
        }
        let relative = format!("{STAGING_DIR}/{TRANSACTION_PREFIX}{transaction_id}");
        if pinned_root
            .exists_relative(&relative)
            .map_err(AppError::Engine)?
        {
            return Err(AppError::Engine(
                "事务目录与 sidecar inventory 不一致，拒绝远程清理".to_string(),
            ));
        }
        let staging_ancestor = staging.duplicate().map_err(AppError::Engine)?;
        result.push(
            crate::app::uninstall_cleanup::RevocationCleanupRoot::prepare_pinned_transaction(
                relative,
                None,
                sidecar,
                staging_ancestor,
            ),
        );
    }
    Ok(result)
}

fn parse_transaction_dir_name(name: &str) -> Option<String> {
    let transaction_id = name.strip_prefix(TRANSACTION_PREFIX)?;
    let parsed = uuid::Uuid::parse_str(transaction_id).ok()?;
    (parsed.to_string() == transaction_id).then(|| transaction_id.to_string())
}

fn partition_revocation_staging_names(
    names: &[String],
) -> Result<(HashSet<String>, HashSet<String>), AppError> {
    let mut transaction_ids = HashSet::new();
    let mut sidecar_ids = HashSet::new();
    for name in names {
        if let Some(id) = parse_transaction_dir_name(name) {
            transaction_ids.insert(id);
        } else if let Some(id) = parse_owner_sidecar_name(name) {
            sidecar_ids.insert(id);
        } else {
            return Err(AppError::Engine(format!(
                "U 盘事务目录包含未知项，拒绝作废清理：{name}"
            )));
        }
    }
    Ok((transaction_ids, sidecar_ids))
}

fn inventory_revocation_staging_names(
    names: &[String],
) -> Result<(HashSet<String>, HashSet<String>), AppError> {
    let (transaction_ids, sidecar_ids) = partition_revocation_staging_names(names)?;
    validate_revocation_staging_owner_pairs(&transaction_ids, &sidecar_ids)?;
    Ok((transaction_ids, sidecar_ids))
}

fn validate_revocation_staging_owner_pairs(
    transaction_ids: &HashSet<String>,
    sidecar_ids: &HashSet<String>,
) -> Result<(), AppError> {
    for transaction_id in transaction_ids {
        if !sidecar_ids.contains(transaction_id) {
            return Err(AppError::Engine(format!(
                "Codex 更新事务缺少 owner sidecar，拒绝远程清理：{TRANSACTION_PREFIX}{transaction_id}"
            )));
        }
    }
    Ok(())
}

fn validate_pinned_transaction_state(
    tree: &crate::app::uninstall_cleanup::PreparedRevocationTree,
    owner: &TransactionOwner,
) -> Result<(), AppError> {
    let main = tree
        .read_relative_bounded("state.json", MAX_TRANSACTION_STATE_BYTES as usize)
        .map_err(AppError::Engine)?;
    let backup = tree
        .read_relative_bounded("state.json.bak", MAX_TRANSACTION_STATE_BYTES as usize)
        .map_err(AppError::Engine)?;
    validate_transaction_state_owner_markers(main.as_deref(), backup.as_deref(), owner)
}

fn validate_transaction_state_owner_markers(
    main: Option<&[u8]>,
    backup: Option<&[u8]>,
    owner: &TransactionOwner,
) -> Result<(), AppError> {
    for bytes in [main, backup].into_iter().flatten() {
        let state: TransactionState = serde_json::from_slice(bytes).map_err(|error| {
            AppError::Engine(format!(
                "事务 state marker JSON 无效，拒绝作废清理：{error}"
            ))
        })?;
        if state.version != TRANSACTION_VERSION || state.owner.as_ref() != Some(owner) {
            return Err(AppError::Engine(
                "事务 state marker 与 owner sidecar 不一致".to_string(),
            ));
        }
    }
    // Both absent is a valid partially-created transaction: the sidecar was
    // atomically written before the transaction directory contents.
    Ok(())
}

#[cfg(test)]
fn validate_transaction_owner_inventory(
    canonical_root: &Path,
    license: &crate::app::usb_guard::UsbLicense,
    expected_license_sha: &str,
    expected_fingerprint_sha: &str,
    transactions: &[PathBuf],
    mut read_state: impl FnMut(&Path, &str) -> Result<TransactionState, AppError>,
) -> Result<(), AppError> {
    for tx in transactions {
        let name = tx
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::Engine("Codex 更新事务目录名无效".to_string()))?;
        let transaction_id = name
            .strip_prefix(TRANSACTION_PREFIX)
            .ok_or_else(|| AppError::Engine("Codex 更新事务目录前缀无效".to_string()))?;
        let parsed = uuid::Uuid::parse_str(transaction_id)
            .map_err(|_| AppError::Engine("Codex 更新事务目录 ID 无效".to_string()))?;
        if parsed.to_string() != transaction_id {
            return Err(AppError::Engine(
                "Codex 更新事务目录 ID 不是规范 UUID".to_string(),
            ));
        }
        let relative = tx
            .strip_prefix(canonical_root)
            .map_err(|_| AppError::Engine("Codex 更新事务目录逃逸 Bundle 根目录".to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let state = read_state(tx, &relative)?;
        let owner = state.owner.ok_or_else(|| {
            AppError::Engine(format!(
                "Codex 更新事务缺少 owner marker，拒绝远程清理：{}",
                tx.display()
            ))
        })?;
        if !transaction_owner_matches(
            &owner,
            transaction_id,
            &license.installation_id,
            expected_license_sha,
            expected_fingerprint_sha,
        ) {
            return Err(AppError::Engine(format!(
                "Codex 更新事务 owner marker 与当前 USB 身份不匹配，拒绝远程清理：{}",
                tx.display()
            )));
        }
    }
    Ok(())
}

fn transaction_owner_matches(
    owner: &TransactionOwner,
    transaction_id: &str,
    installation_id: &str,
    license_signature_sha256: &str,
    fingerprint_sha256: &str,
) -> bool {
    owner.transaction_id == transaction_id
        && owner.installation_id == installation_id
        && owner
            .license_signature_sha256
            .eq_ignore_ascii_case(license_signature_sha256)
        && owner
            .fingerprint_sha256
            .eq_ignore_ascii_case(fingerprint_sha256)
}

fn recovery_owner_matches(recovery: &RecoveryTransactionState, owner: &TransactionOwner) -> bool {
    let Ok(fingerprint_sha256) = codex_usb_license::fingerprint_digest(&recovery.fingerprint)
    else {
        return false;
    };
    recovery.transaction_id == owner.transaction_id
        && recovery.installation_id == owner.installation_id
        && recovery
            .license_signature_sha256
            .eq_ignore_ascii_case(&owner.license_signature_sha256)
        && recovery
            .fingerprint_sha256
            .eq_ignore_ascii_case(&fingerprint_sha256)
        && recovery.fingerprint_sha256 == owner.fingerprint_sha256
        && transaction_owner_matches(
            owner,
            &recovery.transaction_id,
            &recovery.installation_id,
            &recovery.license_signature_sha256,
            &recovery.fingerprint_sha256,
        )
}

fn transaction_owner_for_authorization(
    bundle_root: &Path,
    tx: &Path,
    authorization: &crate::app::usb_guard::SignedUsbAuthorization,
) -> Result<TransactionOwner, AppError> {
    let canonical_root = fs::canonicalize(bundle_root)
        .map_err(|error| AppError::Engine(format!("解析事务 Bundle 根目录失败：{error}")))?;
    let tx_name = tx
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Engine("Codex 更新事务目录名无效".to_string()))?;
    let transaction_id = tx_name
        .strip_prefix(TRANSACTION_PREFIX)
        .ok_or_else(|| AppError::Engine("Codex 更新事务目录前缀无效".to_string()))?;
    let parsed = uuid::Uuid::parse_str(transaction_id)
        .map_err(|_| AppError::Engine("Codex 更新事务目录 ID 无效".to_string()))?;
    if parsed.to_string() != transaction_id {
        return Err(AppError::Engine(
            "Codex 更新事务目录 ID 不是规范 UUID".to_string(),
        ));
    }
    let authorized_root = fs::canonicalize(&authorization.bundle_root)
        .map_err(|error| AppError::Engine(format!("解析授权 Bundle 根目录失败：{error}")))?;
    if canonical_root != authorized_root {
        return Err(AppError::Engine(
            "Codex 更新事务身份在创建前发生变化".to_string(),
        ));
    }
    let license_path = canonical_root.join(USB_LICENSE_NAME);
    let license_bytes = read_bounded_transaction_state_file(&license_path)?
        .ok_or_else(|| AppError::Engine("签名许可证缺失，不能登记更新事务 owner".to_string()))?;
    let license: crate::app::usb_guard::UsbLicense = serde_json::from_slice(&license_bytes)
        .map_err(|error| AppError::Engine(format!("签名许可证 JSON 无效：{error}")))?;
    if license.installation_id != authorization.installation_id
        || !codex_usb_license::fingerprints_match(&license.fingerprint, &authorization.fingerprint)
        || !crate::app::usb_guard::validate_license(&license, &authorization.fingerprint)?
    {
        return Err(AppError::Engine(
            "签名许可证与更新事务授权身份不匹配".to_string(),
        ));
    }
    Ok(TransactionOwner {
        transaction_id: transaction_id.to_string(),
        installation_id: authorization.installation_id.clone(),
        license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
            .map_err(|error| AppError::Engine(format!("计算许可证签名摘要失败：{error}")))?,
        fingerprint_sha256: codex_usb_license::fingerprint_digest(&authorization.fingerprint)
            .map_err(|error| AppError::Engine(format!("计算 USB 指纹摘要失败：{error}")))?,
        legacy_bundle_root: None,
    })
}

fn current_license_signature_sha256(
    bundle_root: &Path,
    authorization: &crate::app::usb_guard::SignedUsbAuthorization,
) -> Result<String, AppError> {
    let license_path = bundle_root.join(USB_LICENSE_NAME);
    let license_bytes = read_bounded_transaction_state_file(&license_path)?
        .ok_or_else(|| AppError::Engine("签名许可证缺失，不能登记恢复事务身份".to_string()))?;
    let license: crate::app::usb_guard::UsbLicense = serde_json::from_slice(&license_bytes)
        .map_err(|error| AppError::Engine(format!("签名许可证 JSON 无效：{error}")))?;
    if license.installation_id != authorization.installation_id
        || !codex_usb_license::fingerprints_match(&license.fingerprint, &authorization.fingerprint)
        || !crate::app::usb_guard::validate_license(&license, &authorization.fingerprint)?
    {
        return Err(AppError::Engine(
            "签名许可证与恢复事务授权身份不匹配".to_string(),
        ));
    }
    codex_usb_license::license_signature_sha256(&license)
        .map_err(|error| AppError::Engine(format!("计算许可证签名摘要失败：{error}")))
}

fn write_transaction_state(tx: &Path, phase: &str) -> Result<(), AppError> {
    write_transaction_state_with_recovery(tx, phase, None)
}

fn write_transaction_state_with_recovery(
    tx: &Path,
    phase: &str,
    recovery: Option<RecoveryTransactionState>,
) -> Result<(), AppError> {
    let state_path = tx.join("state.json");
    let owner = if state_path.exists() {
        read_transaction_state(tx)?.owner
    } else {
        None
    };
    write_transaction_state_with_owner(tx, phase, recovery, owner)
}

fn write_transaction_state_with_owner(
    tx: &Path,
    phase: &str,
    recovery: Option<RecoveryTransactionState>,
    owner: Option<TransactionOwner>,
) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(&TransactionState {
        version: TRANSACTION_VERSION,
        phase: phase.to_string(),
        recovery,
        owner: owner.clone(),
    })
    .map_err(|error| AppError::Engine(format!("序列化事务状态失败: {error}")))?;
    reject_reparse_chain(tx)?;
    crate::app::atomic_file::write_atomic(&tx.join("state.json"), &bytes)
        .map_err(|error| AppError::Engine(format!("原子写入事务状态失败: {error}")))?;
    if owner.is_some() {
        let written = read_transaction_state(tx)?;
        if written.owner != owner || written.phase != phase {
            return Err(AppError::Engine(
                "USB 安装事务 owner marker 回读不匹配".to_string(),
            ));
        }
    }
    Ok(())
}

fn write_commit_transaction_state(
    tx: &Path,
    phase: &str,
    reactivation_reinstall: bool,
    staged_payload_sha256: &str,
) -> Result<(), AppError> {
    if reactivation_reinstall {
        let mut recovery = read_transaction_state(tx)?
            .recovery
            .filter(|state| state.prior_payload_missing)
            .ok_or_else(|| AppError::Engine("恢复事务缺少原始授权身份记录".to_string()))?;
        recovery.staged_payload_sha256 = Some(staged_payload_sha256.to_ascii_lowercase());
        write_transaction_state_with_recovery(tx, phase, Some(recovery))
    } else {
        write_transaction_state(tx, phase)
    }
}

fn read_transaction_state(tx: &Path) -> Result<TransactionState, AppError> {
    let state_path = tx.join("state.json");
    reject_reparse_chain(&state_path)?;
    let backup_path = crate::app::atomic_file::backup_path(&state_path);
    let main = match read_bounded_transaction_state_file(&state_path)? {
        Some(bytes) => serde_json::from_slice::<TransactionState>(&bytes).ok(),
        None => None,
    };
    let state = match main {
        Some(state) => Some(state),
        None => read_bounded_transaction_state_file(&backup_path)?
            .and_then(|bytes| serde_json::from_slice::<TransactionState>(&bytes).ok()),
    }
    .ok_or_else(|| {
        AppError::Engine("事务状态缺失、损坏或超过安全读取上限，保留事务供恢复".to_string())
    })?;
    if state.version != TRANSACTION_VERSION {
        return Err(AppError::Engine("事务状态版本不受支持".to_string()));
    }
    Ok(state)
}

fn read_bounded_transaction_state_file(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    let preflight = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_reparse(path)?;
            if !metadata.file_type().is_file() {
                return Err(AppError::Engine(format!(
                    "事务状态必须是普通文件：{}",
                    path.display()
                )));
            }
            Some(metadata)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::Engine(format!(
                "读取事务状态属性失败 ({}): {error}",
                path.display()
            )))
        }
    };
    if preflight.is_none() {
        return Ok(None);
    }

    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself so the opened handle metadata can be
        // checked before reading, instead of following a swapped junction.
        options.custom_flags(0x0020_0000);
    }
    let mut file = options.open(path).map_err(|error| {
        AppError::Engine(format!("打开事务状态失败 ({}): {error}", path.display()))
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::Engine(format!(
            "读取事务状态句柄属性失败 ({}): {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_TRANSACTION_STATE_BYTES {
        return Err(AppError::Engine(format!(
            "事务状态不是普通文件或超过 {} 字节上限：{}",
            MAX_TRANSACTION_STATE_BYTES,
            path.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(AppError::Engine(format!(
                "事务状态不能是重解析点或硬链接：{}",
                path.display()
            )));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(AppError::Engine(format!(
                "事务状态不能是硬链接：{}",
                path.display()
            )));
        }
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_TRANSACTION_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::Engine(format!("读取事务状态失败 ({}): {error}", path.display()))
        })?;
    if bytes.len() as u64 > MAX_TRANSACTION_STATE_BYTES {
        return Err(AppError::Engine(format!(
            "事务状态超过 {} 字节读取上限：{}",
            MAX_TRANSACTION_STATE_BYTES,
            path.display()
        )));
    }
    Ok(Some(bytes))
}

fn replace_file(staged: &Path, destination: &Path) -> Result<(), AppError> {
    reject_reparse_chain(staged)?;
    reject_reparse_chain(destination)?;
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| {
            AppError::Engine(format!(
                "替换文件前删除旧文件失败 ({}): {error}",
                destination.display()
            ))
        })?;
    }
    fs::rename(staged, destination).map_err(|error| {
        AppError::Engine(format!("提交文件失败 ({}): {error}", destination.display()))
    })
}

fn snapshot_metadata(root: &Path, tx: &Path) -> Result<(), AppError> {
    for name in [ROOT_MANIFEST_NAME, USB_MANIFEST_NAME, SHA256SUMS_NAME] {
        fs::copy(root.join(name), tx.join("old").join(name))
            .map_err(|error| AppError::Engine(format!("保存旧 {name} 快照失败: {error}")))?;
    }
    Ok(())
}

fn restore_file(snapshot: &Path, destination: &Path) -> Result<(), AppError> {
    if !regular_file_exists(snapshot)? {
        return Ok(());
    }
    reject_reparse_chain(snapshot)?;
    reject_reparse_chain(destination)?;
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| AppError::Engine(error.to_string()))?;
    }
    fs::copy(snapshot, destination).map_err(|error| AppError::Engine(error.to_string()))?;
    Ok(())
}

fn verify_reactivation_transaction_authorization(
    bundle_root: &Path,
    recovery: &RecoveryTransactionState,
    owner: &TransactionOwner,
    manifest_payload_relative: &str,
) -> Result<(), AppError> {
    let target_relative = normalize_relative_path(&recovery.target_relative_path)?;
    let manifest_payload_relative = normalize_relative_path(manifest_payload_relative)?;
    if target_relative != manifest_payload_relative {
        return Err(AppError::Engine(
            "恢复事务目标与原始 root manifest 不一致".to_string(),
        ));
    }
    #[cfg(test)]
    if !crate::app::usb_guard::guard_required() {
        // Synthetic unit fixtures have no removable volume, signed license,
        // or status service. Production never takes this branch.
        return Ok(());
    }
    let authorization = crate::app::usb_guard::verify_signed_codex_reinstall_transaction(
        "codex-payload-reinstall-rollback",
        bundle_root,
        &recovery.installation_id,
        &recovery.fingerprint,
    )?;
    let license_bytes =
        read_bounded_transaction_state_file(&bundle_root.join(USB_LICENSE_NAME))?
            .ok_or_else(|| AppError::Engine("恢复事务当前签名许可证缺失".to_string()))?;
    let license: crate::app::usb_guard::UsbLicense = serde_json::from_slice(&license_bytes)
        .map_err(|error| AppError::Engine(format!("恢复事务当前许可证 JSON 无效：{error}")))?;
    let current_license_sha = codex_usb_license::license_signature_sha256(&license)
        .map_err(|error| AppError::Engine(format!("计算当前许可证签名摘要失败：{error}")))?;
    let current_fingerprint_sha = codex_usb_license::fingerprint_digest(&authorization.fingerprint)
        .map_err(|error| AppError::Engine(format!("计算当前 USB 指纹摘要失败：{error}")))?;
    if authorization.installation_id != recovery.installation_id
        || license.installation_id != recovery.installation_id
        || !codex_usb_license::fingerprints_match(&authorization.fingerprint, &recovery.fingerprint)
        || !recovery
            .fingerprint_sha256
            .eq_ignore_ascii_case(&current_fingerprint_sha)
        || !recovery
            .license_signature_sha256
            .eq_ignore_ascii_case(&current_license_sha)
        || !recovery_owner_matches(recovery, owner)
    {
        return Err(AppError::Engine(
            "恢复事务的 installation_id、许可证摘要、事务 ID 或完整 USB 指纹已变化".to_string(),
        ));
    }
    let current_manifest = read_json(&bundle_root.join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let current_target = normalize_relative_path(
        current_manifest
            .pointer("/payload/path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Engine("当前 root manifest 缺少 payload.path".to_string()))?,
    )?;
    if current_target != target_relative {
        return Err(AppError::Engine(
            "当前 root manifest 改变了恢复事务的受管目标路径".to_string(),
        ));
    }
    Ok(())
}

fn verify_reactivation_transaction_boundary(root: &Path, tx: &Path) -> Result<(), AppError> {
    let state = read_transaction_state(tx)?;
    let recovery = state
        .recovery
        .as_ref()
        .filter(|recovery| recovery.prior_payload_missing)
        .ok_or_else(|| AppError::Engine("恢复事务缺少身份记录".to_string()))?;
    let owner = state
        .owner
        .as_ref()
        .ok_or_else(|| AppError::Engine("恢复事务缺少 owner 身份记录".to_string()))?;
    let old_manifest = read_json(&tx.join("old").join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let old_target = old_manifest
        .pointer("/payload/path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Engine("恢复事务快照缺少 payload.path".to_string()))?;
    verify_reactivation_transaction_authorization(root, recovery, owner, old_target)
}

fn rollback_transaction(bundle_root: &Path, tx: &Path) -> Result<(), AppError> {
    let transaction_state = read_transaction_state(tx)?;
    let recovery_install = transaction_state
        .recovery
        .as_ref()
        .filter(|state| state.prior_payload_missing);
    let transaction_owner = transaction_state.owner.as_ref();
    let old = tx.join("old");
    reject_reparse_chain(&old)?;
    let old_manifest = read_json(&old.join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let local_relative = normalize_relative_path(
        old_manifest
            .pointer("/payload/path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Engine("旧 manifest 缺少 payload.path".to_string()))?,
    )?;
    let local = bundle_root.join(local_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(recovery) = recovery_install {
        let owner = transaction_owner
            .ok_or_else(|| AppError::Engine("恢复事务缺少 owner 身份记录".to_string()))?;
        verify_reactivation_transaction_authorization(
            bundle_root,
            recovery,
            owner,
            &local_relative,
        )?;
    }
    let old_usb_manifest = read_json(&old.join(USB_MANIFEST_NAME), USB_MANIFEST_NAME)?;
    let final_backup_relative = normalize_relative_path(
        old_usb_manifest
            .pointer("/updateCapability/backupDirectory")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Engine("旧 usb-manifest 缺少 backupDirectory".to_string()))?,
    )?;
    let final_backup_dir = bundle_root.join(final_backup_relative);
    reject_reparse_chain(&local)?;
    reject_reparse_chain(&final_backup_dir)?;
    let old_architecture = old_manifest
        .get("architecture")
        .and_then(serde_json::Value::as_str)
        .filter(|value| matches!(*value, "x64" | "arm64"))
        .ok_or_else(|| AppError::Engine("旧 manifest 缺少有效 architecture".to_string()))?;
    let old_version = old_manifest
        .get("packageVersion")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            old_manifest
                .get("codexDesktopVersion")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| AppError::Engine("旧 manifest 缺少可恢复版本".to_string()))?;
    let expected_backup = final_backup_dir.join(format!(
        "OpenAI.Codex_{}_{}.msix",
        old_architecture,
        sanitize_version(old_version)
    ));
    if let Some(recovery) = recovery_install {
        // A recovery transaction starts with this exact manifest-declared
        // payload absent. Remove only the file this transaction installed,
        // and only after its hash matches the SHA verified before the rename.
        // Any unexpected replacement is left untouched and stops rollback.
        reject_reparse_chain(&local)?;
        if local.exists() {
            let expected_sha = recovery.staged_payload_sha256.as_deref().ok_or_else(|| {
                AppError::Engine(
                    "恢复事务目标意外出现，但事务未记录 staged SHA；拒绝删除".to_string(),
                )
            })?;
            crate::app::uninstall_cleanup::remove_revocation_staged_payload_if_sha256(
                bundle_root,
                &recovery.fingerprint,
                &local_relative,
                expected_sha,
            )
            .map_err(|error| {
                AppError::Engine(format!("移除未提交恢复载荷失败；目标已保留：{error}"))
            })?;
        }
    }
    // Restore only the exact backup name generated by this transaction. A
    // pre-existing backup must never be mistaken for the payload being
    // rolled back.
    if recovery_install.is_none() && regular_file_exists(&expected_backup)? {
        let old_payload_sha = require_sha256(
            old_manifest.pointer("/payload/sha256"),
            "旧 manifest.payload.sha256",
        )?;
        let backup_is_old = sha256_file(&expected_backup)
            .map(|hash| hash.eq_ignore_ascii_case(&old_payload_sha))
            .unwrap_or(false);
        if !backup_is_old {
            return Err(AppError::Engine(
                "恢复 backup 与旧 manifest.payload.sha256 不一致".to_string(),
            ));
        }
        let local_is_old = regular_file_exists(&local)?
            && sha256_file(&local)
                .map(|hash| hash.eq_ignore_ascii_case(&old_payload_sha))
                .unwrap_or(false);
        if !local_is_old {
            if local.exists() {
                fs::remove_file(&local).map_err(|error| AppError::Engine(error.to_string()))?;
            }
            fs::rename(&expected_backup, &local)
                .map_err(|error| AppError::Engine(error.to_string()))?;
        }
    }
    restore_file(
        &old.join(ROOT_MANIFEST_NAME),
        &bundle_root.join(ROOT_MANIFEST_NAME),
    )?;
    restore_file(
        &old.join(USB_MANIFEST_NAME),
        &bundle_root.join(USB_MANIFEST_NAME),
    )?;
    restore_file(
        &old.join(SHA256SUMS_NAME),
        &bundle_root.join(SHA256SUMS_NAME),
    )?;
    let old_backup_dir = tx.join("backup-dir").join("payload-backup");
    if old_backup_dir.exists() {
        reject_reparse_chain(&old_backup_dir)?;
        reject_reparse_chain(&final_backup_dir)?;
        fs::create_dir_all(&final_backup_dir)
            .map_err(|error| AppError::Engine(error.to_string()))?;
        for entry in
            fs::read_dir(&old_backup_dir).map_err(|error| AppError::Engine(error.to_string()))?
        {
            let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
            reject_reparse_chain(&entry.path())?;
            if !entry
                .file_type()
                .map_err(|error| AppError::Engine(error.to_string()))?
                .is_file()
            {
                return Err(AppError::Engine("事务 backup 目录包含非文件项".to_string()));
            }
            let destination = final_backup_dir.join(entry.file_name());
            if destination.exists() {
                fs::remove_file(&destination)
                    .map_err(|error| AppError::Engine(error.to_string()))?;
            }
            fs::rename(entry.path(), destination)
                .map_err(|error| AppError::Engine(error.to_string()))?;
        }
    }
    if tx.exists() {
        reject_reparse_chain(tx)?;
        fs::remove_dir_all(tx)
            .map_err(|error| AppError::Engine(format!("清理已回滚事务失败: {error}")))?;
    }
    let transaction_id = transaction_state
        .owner
        .as_ref()
        .map(|owner| owner.transaction_id.as_str())
        .unwrap_or_else(|| transaction_id_from_path(tx));
    remove_owner_sidecar_after_transaction(
        bundle_root,
        transaction_id,
        transaction_state.owner.as_ref(),
    )?;
    Ok(())
}

fn verify_committed_transaction(bundle_root: &Path, tx: &Path) -> Result<(), AppError> {
    let next_root_path = tx.join("next").join(ROOT_MANIFEST_NAME);
    let next_usb_path = tx.join("next").join(USB_MANIFEST_NAME);
    let next_sums_path = tx.join("next").join(SHA256SUMS_NAME);
    reject_reparse_chain(&tx.join("next"))?;
    for path in [&next_root_path, &next_usb_path, &next_sums_path] {
        reject_reparse_chain(path)?;
    }
    let next_root_bytes = fs::read(&next_root_path)
        .map_err(|error| AppError::Engine(format!("读取 next root manifest 失败: {error}")))?;
    let next_usb_bytes = fs::read(&next_usb_path)
        .map_err(|error| AppError::Engine(format!("读取 next usb-manifest 失败: {error}")))?;
    let next_sums = fs::read(&next_sums_path)
        .map_err(|error| AppError::Engine(format!("读取 next SHA256SUMS 失败: {error}")))?;

    for (name, expected) in [
        (ROOT_MANIFEST_NAME, &next_root_bytes),
        (USB_MANIFEST_NAME, &next_usb_bytes),
        (SHA256SUMS_NAME, &next_sums),
    ] {
        let current = fs::read(bundle_root.join(name))
            .map_err(|error| AppError::Engine(format!("读取已提交 {name} 失败: {error}")))?;
        if current != *expected {
            return Err(AppError::Engine(format!(
                "已提交事务的 {name} 与 next 状态不一致"
            )));
        }
    }

    let next_root = read_json(&next_root_path, ROOT_MANIFEST_NAME)?;
    let next_usb = read_json(&next_usb_path, USB_MANIFEST_NAME)?;
    let payload_relative = normalize_relative_path(
        next_root
            .pointer("/payload/path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Engine("next root manifest 缺少 payload.path".to_string()))?,
    )?;
    if next_usb
        .pointer("/codexInstaller/payload")
        .and_then(serde_json::Value::as_str)
        != Some(payload_relative.as_str())
    {
        return Err(AppError::Engine(
            "已提交事务的 next payload 路径不一致".to_string(),
        ));
    }
    let payload_path =
        bundle_root.join(payload_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    reject_reparse_chain(&payload_path)?;
    let payload_size = fs::metadata(&payload_path)
        .map_err(|error| AppError::Engine(format!("读取已提交 payload 大小失败: {error}")))?
        .len();
    let payload_sha =
        sha256_file(&payload_path).map_err(|error| AppError::Engine(error.to_string()))?;

    // Capability validation is intentionally performed on the already matched
    // current files. It is a local consistency check here, not the apply
    // entry-point gate that runs after pending recovery.
    let _ = require_capability(bundle_root)?;
    verify_metadata_values(bundle_root, &payload_sha, payload_size)?;
    let mut overrides = BTreeMap::new();
    overrides.insert(ROOT_MANIFEST_NAME.to_string(), next_root_bytes);
    overrides.insert(USB_MANIFEST_NAME.to_string(), next_usb_bytes);
    verify_sha256_sums(bundle_root, &next_sums, &overrides)
}

fn recover_pending_transactions(bundle_root: &Path) -> Result<(), AppError> {
    reject_reparse(bundle_root)?;
    for tx in pending_transaction_dirs(bundle_root)? {
        let state = read_transaction_state(&tx)?;
        if state.phase == "owner-registered" {
            discard_unprepared_owner_transaction(&tx, &state)?;
            remove_owner_sidecar_after_transaction(
                bundle_root,
                &state
                    .owner
                    .as_ref()
                    .ok_or_else(|| AppError::Engine("未准备事务缺少 owner".to_string()))?
                    .transaction_id,
                state.owner.as_ref(),
            )?;
            continue;
        }
        if state.phase == "committed" {
            if verify_committed_transaction(bundle_root, &tx).is_ok() {
                fs::remove_dir_all(&tx).map_err(|error| AppError::Engine(error.to_string()))?;
                remove_owner_sidecar_after_transaction(
                    bundle_root,
                    state
                        .owner
                        .as_ref()
                        .map(|owner| owner.transaction_id.as_str())
                        .unwrap_or_else(|| transaction_id_from_path(&tx)),
                    state.owner.as_ref(),
                )?;
            } else {
                rollback_transaction(bundle_root, &tx)?;
            }
        } else {
            rollback_transaction(bundle_root, &tx)?;
        }
    }
    cleanup_orphan_owner_sidecars(bundle_root)?;
    let staging = bundle_root.join(STAGING_DIR);
    if staging.exists()
        && fs::read_dir(&staging)
            .map_err(|error| AppError::Engine(error.to_string()))?
            .next()
            .is_none()
    {
        reject_reparse_chain(&staging)?;
        fs::remove_dir(staging)
            .map_err(|error| AppError::Engine(format!("清理空事务目录失败: {error}")))?;
    }
    Ok(())
}

fn transaction_id_from_path(tx: &Path) -> &str {
    tx.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(TRANSACTION_PREFIX))
        .unwrap_or_default()
}

fn cleanup_orphan_owner_sidecars(bundle_root: &Path) -> Result<(), AppError> {
    let staging = bundle_root.join(STAGING_DIR);
    if !staging.exists() {
        return Ok(());
    }
    let has_owner_sidecar = fs::read_dir(&staging)
        .map_err(|error| AppError::Engine(format!("读取事务 sidecar 失败：{error}")))?
        .try_fold(false, |found, entry| {
            let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
            Ok::<_, AppError>(
                found || parse_owner_sidecar_name(&entry.file_name().to_string_lossy()).is_some(),
            )
        })?;
    // Legacy v1/v2 transactions never had owner sidecars. Their recovery
    // semantics must not require a signed v3 license just to finish an update.
    if !has_owner_sidecar {
        return Ok(());
    }
    let license_bytes =
        read_bounded_transaction_state_file(&bundle_root.join(USB_LICENSE_NAME))?
            .ok_or_else(|| AppError::Engine("事务 sidecar 清理缺少签名许可证".to_string()))?;
    let license: crate::app::usb_guard::UsbLicense = serde_json::from_slice(&license_bytes)
        .map_err(|error| AppError::Engine(format!("事务 sidecar 清理许可证无效：{error}")))?;
    if !crate::app::usb_guard::validate_license(&license, &license.fingerprint)? {
        return Err(AppError::Engine(
            "事务 sidecar 清理需要有效签名许可证".to_string(),
        ));
    }
    let license_sha = codex_usb_license::license_signature_sha256(&license)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    let fingerprint_sha = codex_usb_license::fingerprint_digest(&license.fingerprint)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    let transactions = pending_transaction_dirs(bundle_root)?;
    let transaction_ids = transactions
        .iter()
        .filter_map(|tx| parse_transaction_dir_name(tx.file_name()?.to_str()?))
        .collect::<HashSet<_>>();
    for entry in fs::read_dir(&staging)
        .map_err(|error| AppError::Engine(format!("读取事务 sidecar 失败：{error}")))?
    {
        let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
        let name = entry.file_name();
        let Some(transaction_id) = parse_owner_sidecar_name(&name.to_string_lossy()) else {
            continue;
        };
        if transaction_ids.contains(&transaction_id) {
            continue;
        }
        let sidecar_path = entry.path();
        reject_reparse_chain(&sidecar_path)?;
        let bytes = read_bounded_transaction_state_file(&sidecar_path)?
            .ok_or_else(|| AppError::Engine("孤儿 owner sidecar 在读取期间消失".to_string()))?;
        let owner: TransactionOwner = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::Engine(format!("孤儿 owner sidecar 无效：{error}")))?;
        if !transaction_owner_matches(
            &owner,
            &transaction_id,
            &license.installation_id,
            &license_sha,
            &fingerprint_sha,
        ) {
            return Err(AppError::Engine(
                "孤儿 owner sidecar 与当前签名许可证身份不匹配，保留文件".to_string(),
            ));
        }
        remove_owner_sidecar_after_transaction(bundle_root, &transaction_id, Some(&owner))?;
    }
    Ok(())
}

/// An owner-registered transaction is written before any snapshot or payload
/// data. If the process stops during preparation, discard it only when its
/// tree contains the exact state/metadata shape that can exist before commit.
fn discard_unprepared_owner_transaction(
    tx: &Path,
    state: &TransactionState,
) -> Result<(), AppError> {
    if state.owner.is_none() {
        return Err(AppError::Engine(
            "未准备事务缺少 owner marker，保留目录供人工检查".to_string(),
        ));
    }
    reject_reparse_chain(tx)?;
    for entry in fs::read_dir(tx)
        .map_err(|error| AppError::Engine(format!("读取未准备事务失败：{error}")))?
    {
        let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        reject_reparse_chain(&path)?;
        match name.as_ref() {
            "state.json" | "state.json.bak" => {
                if !entry
                    .file_type()
                    .map_err(|error| AppError::Engine(error.to_string()))?
                    .is_file()
                {
                    return Err(AppError::Engine(
                        "未准备事务状态不是普通文件，保留目录".to_string(),
                    ));
                }
            }
            "old" => {
                let metadata = entry
                    .metadata()
                    .map_err(|error| AppError::Engine(error.to_string()))?;
                if !metadata.is_dir() {
                    return Err(AppError::Engine(
                        "未准备事务 old 路径不是目录，保留目录".to_string(),
                    ));
                }
                for snapshot in
                    fs::read_dir(&path).map_err(|error| AppError::Engine(error.to_string()))?
                {
                    let snapshot = snapshot.map_err(|error| AppError::Engine(error.to_string()))?;
                    if !matches!(
                        snapshot.file_name().to_str(),
                        Some(ROOT_MANIFEST_NAME | USB_MANIFEST_NAME | SHA256SUMS_NAME)
                    ) || !snapshot
                        .file_type()
                        .map_err(|error| AppError::Engine(error.to_string()))?
                        .is_file()
                    {
                        return Err(AppError::Engine(
                            "未准备事务 old 目录包含未知项，保留目录".to_string(),
                        ));
                    }
                    reject_reparse_chain(&snapshot.path())?;
                }
            }
            "next" => {
                let metadata = entry
                    .metadata()
                    .map_err(|error| AppError::Engine(error.to_string()))?;
                if !metadata.is_dir()
                    || fs::read_dir(&path)
                        .map_err(|error| AppError::Engine(error.to_string()))?
                        .next()
                        .is_some()
                {
                    return Err(AppError::Engine(
                        "未准备事务 next 目录非空或类型无效，保留目录".to_string(),
                    ));
                }
            }
            _ => {
                return Err(AppError::Engine(format!(
                    "未准备事务包含未知项，保留目录：{}",
                    path.display()
                )))
            }
        }
    }
    fs::remove_dir_all(tx).map_err(|error| AppError::Engine(format!("清理未准备事务失败：{error}")))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn collect_files(
    root: &Path,
    current_relative: &str,
    out: &mut Vec<PathBuf>,
) -> Result<(), AppError> {
    let directory = root.join(current_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    reject_reparse_chain(&directory)?;
    for entry in fs::read_dir(&directory).map_err(|error| AppError::Engine(error.to_string()))? {
        let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let relative = if current_relative.is_empty() {
            name.clone()
        } else {
            format!("{current_relative}/{name}")
        };
        let path = entry.path();
        reject_reparse(&path)?;
        let relative_lower = relative.to_ascii_lowercase();
        let file_type = entry
            .file_type()
            .map_err(|error| AppError::Engine(error.to_string()))?;
        // Revocation state is mutable after bundle signing. Use the bundle
        // validator's exact shared predicate so new update checksum sets and
        // startup validation agree on the same runtime files.
        if crate::app::usb_bundle::is_mutable_revocation_file(&relative) {
            if !file_type.is_file() {
                return Err(AppError::Engine(format!(
                    "USB 撤销运行状态必须是普通文件：{relative}"
                )));
            }
            if crate::app::usb_revocation::is_atomic_state_tmp_relative(&relative) {
                crate::app::usb_bundle::validate_atomic_state_temp(&path)?;
            } else {
                crate::app::usb_bundle::validate_mutable_revocation_sidecar(&path)?;
            }
            continue;
        }
        if relative_lower == "codexusbstate" {
            if !file_type.is_dir() {
                return Err(AppError::Engine("CodexUsbState 必须是目录".to_string()));
            }
            collect_files(root, &relative, out)?;
            continue;
        }
        if relative_lower.starts_with("codexusbstate/") {
            return Err(AppError::Engine(format!(
                "CodexUsbState 包含未支持项：{relative}"
            )));
        }
        // `apps/Codex` is created and updated after the USB bundle is built;
        // it is deliberately outside SHA256SUMS just like the transaction
        // directory. Keep checking its reparse boundary above, but never
        // mistake its managed runtime files for immutable release files.
        if relative_lower == STAGING_DIR.to_ascii_lowercase()
            || relative_lower.starts_with(&(STAGING_DIR.to_ascii_lowercase() + "/"))
            || relative_lower == "apps/codex"
            || relative_lower.starts_with("apps/codex/")
        {
            continue;
        }
        if file_type.is_dir() {
            collect_files(root, &relative, out)?;
        } else if file_type.is_file()
            && !relative.eq_ignore_ascii_case(SHA256SUMS_NAME)
            && !relative.eq_ignore_ascii_case(USB_LICENSE_NAME)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn build_sha256_sums(
    root: &Path,
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, AppError> {
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    let mut rows = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| AppError::Engine(error.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let hash = if let Some(bytes) = overrides.get(&relative) {
            sha256_bytes(bytes)
        } else {
            sha256_file(&path).map_err(|error| AppError::Engine(error.to_string()))?
        };
        rows.push((relative, hash));
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut text = String::new();
    for (relative, hash) in rows {
        text.push_str(&hash);
        text.push_str("  ");
        text.push_str(&relative);
        text.push('\n');
    }
    Ok(text.into_bytes())
}

fn verify_sha256_sums(
    root: &Path,
    sums: &[u8],
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<(), AppError> {
    verify_sha256_sums_with_digests(root, sums, overrides).map(|_| ())
}

fn verify_sha256_sums_with_digests(
    root: &Path,
    sums: &[u8],
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, String>, AppError> {
    let text = std::str::from_utf8(sums)
        .map_err(|error| AppError::Engine(format!("SHA256SUMS 不是 UTF-8: {error}")))?;
    let mut listed = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (hash, raw_path) = line
            .split_once(|character: char| character.is_ascii_whitespace())
            .ok_or_else(|| AppError::Engine("SHA256SUMS 行格式错误".to_string()))?;
        let hash = hash.trim();
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::Engine("SHA256SUMS 哈希格式错误".to_string()));
        }
        let relative = normalize_relative_path(raw_path.trim())?;
        if relative == SHA256SUMS_NAME
            || crate::app::usb_bundle::is_mutable_revocation_file(&relative)
            || listed.contains_key(&relative)
        {
            return Err(AppError::Engine(
                "SHA256SUMS 包含重复或非法条目".to_string(),
            ));
        }
        listed.insert(relative, hash.to_ascii_lowercase());
    }
    let mut verified = BTreeMap::new();
    for (relative, expected_hash) in &listed {
        let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        reject_reparse_chain(&path)?;
        if !regular_file_exists(&path)? {
            return Err(AppError::Engine(format!("SHA256SUMS 缺少文件：{relative}")));
        }
        let actual = if let Some(bytes) = overrides.get(relative) {
            sha256_bytes(bytes)
        } else {
            sha256_file(&path).map_err(|error| AppError::Engine(error.to_string()))?
        };
        if !actual.eq_ignore_ascii_case(expected_hash) {
            return Err(AppError::Engine(format!(
                "SHA256SUMS 哈希不匹配：{relative}"
            )));
        }
        // The value is known to match the normalized expected hash. Keeping
        // the actual value here lets the caller construct a snapshot without
        // reading the same large payload a second time.
        verified.insert(relative.clone(), actual);
    }
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| AppError::Engine(error.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !listed.contains_key(&relative) {
            return Err(AppError::Engine(format!(
                "文件未列入 SHA256SUMS：{relative}"
            )));
        }
    }
    Ok(verified)
}

fn verify_metadata_values(
    root: &Path,
    expected_sha: &str,
    expected_size: u64,
) -> Result<(), AppError> {
    let manifest = read_json(&root.join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let usb = read_json(&root.join(USB_MANIFEST_NAME), USB_MANIFEST_NAME)?;
    let root_sha = manifest
        .pointer("/payload/sha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let root_size = exact_u64(manifest.pointer("/payload/size"), "manifest.payload.size")?;
    let usb_sha = usb
        .pointer("/codexInstaller/payloadSha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let usb_size = exact_u64(
        usb.pointer("/codexInstaller/payloadSize"),
        "usb-manifest.codexInstaller.payloadSize",
    )?;
    if !root_sha.eq_ignore_ascii_case(expected_sha)
        || !usb_sha.eq_ignore_ascii_case(expected_sha)
        || root_size != expected_size
        || usb_size != expected_size
    {
        return Err(AppError::Engine("更新后 payload 元数据不一致".to_string()));
    }
    Ok(())
}

fn update_metadata(
    local: &LocalBundle,
    fresh: &CodexPayloadIdentity,
    app_version: &str,
    payload_sha: &str,
    payload_size: u64,
) -> Result<(Vec<u8>, Vec<u8>), AppError> {
    let mut manifest = local.manifest.clone();
    manifest["codexDesktopVersion"] = serde_json::Value::String(app_version.to_string());
    manifest["packageVersion"] = serde_json::Value::String(fresh.msix_version.clone());
    manifest["releaseTag"] = serde_json::Value::String(fresh.release_tag.clone());
    let payload = manifest
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::Engine("root manifest 缺少 payload 对象".to_string()))?;
    payload.insert(
        "sha256".to_string(),
        serde_json::Value::String(payload_sha.to_string()),
    );
    payload.insert(
        "size".to_string(),
        serde_json::Value::Number(serde_json::Number::from(payload_size)),
    );

    let mut usb = local.usb_manifest.clone();
    let installer = usb
        .get_mut("codexInstaller")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::Engine("usb-manifest 缺少 codexInstaller".to_string()))?;
    installer.insert(
        "payloadSha256".to_string(),
        serde_json::Value::String(payload_sha.to_string()),
    );
    installer.insert(
        "payloadSize".to_string(),
        serde_json::Value::Number(serde_json::Number::from(payload_size)),
    );
    if installer.get("payload").and_then(serde_json::Value::as_str)
        != Some(local.payload_relative.as_str())
    {
        return Err(AppError::StaleExpectation(
            "USB 清单中的 payload 路径已变化，请重新检查。".to_string(),
        ));
    }
    let root_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| AppError::Engine(format!("序列化 root manifest 失败: {error}")))?;
    let usb_bytes = serde_json::to_vec_pretty(&usb)
        .map_err(|error| AppError::Engine(format!("序列化 usb-manifest 失败: {error}")))?;
    Ok((root_bytes, usb_bytes))
}

fn identity_mismatch_reason(
    expected: &CodexPayloadIdentity,
    fresh: &CodexPayloadIdentity,
) -> Option<&'static str> {
    if expected.source_kind != fresh.source_kind || expected.source_locator != fresh.source_locator
    {
        return Some("更新源已变化");
    }
    if expected.release_id != fresh.release_id || expected.release_tag != fresh.release_tag {
        return Some("发布版本身份已变化");
    }
    if expected.package_moniker != fresh.package_moniker
        || expected.architecture != fresh.architecture
        || expected.msix_version != fresh.msix_version
        || expected.app_version != fresh.app_version
    {
        return Some("MSIX 包身份或版本已变化");
    }
    if expected.msix_size != fresh.msix_size
        || expected.msix_sha256 != fresh.msix_sha256
        || expected.github_digest != fresh.github_digest
    {
        return Some("MSIX 摘要或大小已变化");
    }
    if expected.checksums_sha256 != fresh.checksums_sha256
        || expected.manifest_sha256 != fresh.manifest_sha256
    {
        return Some("发布清单或校验文件已变化");
    }
    if expected.msix_asset_id != fresh.msix_asset_id
        || expected.msix_asset_name != fresh.msix_asset_name
        || expected.sha256_asset_id != fresh.sha256_asset_id
        || expected.sha256_asset_name != fresh.sha256_asset_name
        || expected.manifest_asset_id != fresh.manifest_asset_id
        || expected.manifest_asset_name != fresh.manifest_asset_name
        || expected.repo != fresh.repo
    {
        return Some("更新资产身份已变化");
    }
    None
}

#[cfg(test)]
fn same_identity(expected: &CodexPayloadIdentity, fresh: &CodexPayloadIdentity) -> bool {
    identity_mismatch_reason(expected, fresh).is_none()
}

fn require_newer_version(current: &str, latest: &str) -> Result<(), AppError> {
    if !is_numeric_version(current) || !is_numeric_version(latest) {
        return Err(AppError::StaleExpectation(
            "远端或本地版本格式无效，拒绝更新。".to_string(),
        ));
    }
    if compare_versions(current, latest) == std::cmp::Ordering::Less {
        Ok(())
    } else {
        Err(AppError::StaleExpectation(
            "远端版本不高于当前版本，拒绝降级更新。".to_string(),
        ))
    }
}

fn require_not_older_version(current: &str, latest: &str) -> Result<(), AppError> {
    if !is_numeric_version(current) || !is_numeric_version(latest) {
        return Err(AppError::StaleExpectation(
            "远端或本地版本格式无效，拒绝恢复。".to_string(),
        ));
    }
    if compare_versions(current, latest) == std::cmp::Ordering::Greater {
        return Err(AppError::StaleExpectation(
            "远端版本早于当前版本，拒绝降级恢复。".to_string(),
        ));
    }
    Ok(())
}

fn clear_writer_artifacts(bundle_root: &Path) -> Result<(), AppError> {
    for name in [
        ROOT_MANIFEST_NAME,
        USB_MANIFEST_NAME,
        SHA256SUMS_NAME,
        USB_LICENSE_NAME,
    ] {
        let path = bundle_root.join(name);
        let backup = crate::app::atomic_file::backup_path(&path);
        if backup.exists() {
            fs::remove_file(&backup).map_err(|error| {
                AppError::Engine(format!("清理写入残留失败 ({}): {error}", backup.display()))
            })?;
        }
    }
    Ok(())
}

pub fn check(ctx: &UpdateContext) -> Result<Option<UpdateInfo>, AppError> {
    check_inner(ctx, false)
}

/// Read-only update check for the single missing-payload reactivation state.
/// It is not exposed as a renderer flag: both this function and its command
/// caller independently require the signed USB recovery gate.
pub fn check_reinstall(ctx: &UpdateContext) -> Result<Option<UpdateInfo>, AppError> {
    check_inner(ctx, true)
}

fn check_inner(
    ctx: &UpdateContext,
    reactivation_reinstall: bool,
) -> Result<Option<UpdateInfo>, AppError> {
    let initial_reinstall_authorization = if reactivation_reinstall {
        Some(
            crate::app::usb_guard::require_signed_codex_reinstall_bundle(
                "codex-payload-reinstall-check-inner",
            )?,
        )
    } else {
        None
    };
    require_windows()?;
    let mut deferred_cleanup = None;
    {
        let _operation = CODEX_UPDATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let initial_authorization = if reactivation_reinstall {
            Some(
                initial_reinstall_authorization
                    .expect("reactivation check produced an authorization"),
            )
        } else {
            crate::app::usb_guard::require_signed_codex_bundle(
                "codex-payload-check-transaction-lock",
            )?
        };
        let _transaction_guard = if let Some(initial) = initial_authorization.as_ref() {
            let guard = crate::app::usb_revocation::acquire_codex_update_transaction_lock(
                &initial.installation_id,
            )
            .map_err(|error| AppError::Engine(format!("无法锁定 USB 安装事务：{error}")))?;
            let current = if reactivation_reinstall {
                match crate::app::usb_guard::require_signed_codex_reinstall_bundle_deferred(
                    "codex-payload-reinstall-check-lock-recheck",
                )? {
                    crate::app::usb_guard::ReinstallGateOutcome::Authorized(current) => {
                        Some(current)
                    }
                    crate::app::usb_guard::ReinstallGateOutcome::DeferredCleanup(request) => {
                        deferred_cleanup = Some(*request);
                        None
                    }
                }
            } else {
                Some(
                    crate::app::usb_guard::require_signed_codex_bundle(
                        "codex-payload-check-lock-recheck",
                    )?
                    .ok_or_else(|| AppError::Engine("当前进程不是受管 USB bundle".to_string()))?,
                )
            };
            if let Some(current) = current {
                if current.installation_id != initial.installation_id
                    || current.bundle_root != initial.bundle_root
                    || current.fingerprint != initial.fingerprint
                    || current.license_digest != initial.license_digest
                {
                    return Err(AppError::StaleExpectation(
                        "授权 U 盘或许可证在等待更新事务锁时发生变化，请重新检查。".to_string(),
                    ));
                }
            }
            Some(guard)
        } else {
            None
        };
        if deferred_cleanup.is_none() {
            // A previous apply may have been interrupted after a rename. Recovery
            // is serialized with revoked cleanup before it reads or mutates the
            // staging tree. The update and cleanup paths revalidate after this lock.
            recover_pending_transactions(&ctx.bundle_root)?;
        }
    }
    if let Some(request) = deferred_cleanup {
        return request.cleanup_and_reject().map(|_| None);
    }
    // Read only the capability metadata first. The full removable-drive
    // integrity pass is intentionally delayed until after remote resolution,
    // so an unavailable source is not hidden behind a long local hash.
    let local_metadata = if reactivation_reinstall {
        require_capability_metadata_for_reinstall(&ctx.bundle_root)
    } else {
        require_capability_metadata(&ctx.bundle_root)
    };
    let local = local_metadata.map_err(|error| {
        log::error!(
            "codex payload update check failed stage=capability code={} error={}",
            error.code(),
            error
        );
        error
    })?;
    log::info!(
        "codex payload update check start stage=capability architecture={} current={} source={} proxy={}",
        local.architecture,
        local.current_version,
        ctx.settings.codex_payload_source_kind.as_str(),
        ctx.settings.proxy_mode.as_str()
    );
    let network = network_from_settings(&ctx.settings);
    let source_version = if reactivation_reinstall {
        "0.0.0"
    } else {
        &local.current_version
    };
    let fresh = match resolve_preferred_source(
        &ctx.settings,
        &local.architecture,
        &network,
        source_version,
    ) {
        Ok(fresh) => fresh,
        Err(error) => {
            log::error!(
                "codex payload update check failed stage=metadata architecture={} current={} code={} error={}",
                local.architecture,
                local.current_version,
                error.code(),
                error
            );
            return Err(error);
        }
    };
    log::info!(
        "codex payload update check stage=integrity start architecture={} current={}",
        local.architecture,
        local.current_version
    );
    let verified_digests = if reactivation_reinstall {
        None
    } else {
        let verified_digests = match verify_bundle_integrity_with_digests(&ctx.bundle_root) {
            Ok(digests) => digests,
            Err(error) => {
                log::error!(
                    "codex payload update check failed stage=integrity current={} code={} error={}",
                    local.current_version,
                    error.code(),
                    error
                );
                return Err(error);
            }
        };
        let payload_sha = verified_digest(&verified_digests, &local.payload_relative, "payload")?;
        let payload_size = fs::metadata(&local.payload_path)
            .map_err(|error| AppError::Engine(format!("读取 payload 大小失败: {error}")))?
            .len();
        verify_metadata_values(&ctx.bundle_root, &payload_sha, payload_size)?;
        log::info!(
            "codex payload update check stage=integrity completed files={}",
            verified_digests.len()
        );
        Some(verified_digests)
    };
    let Some(fresh) = fresh else {
        log::info!(
            "codex payload update check completed stage=compare result=no-update reason=source_no_newer current={}",
            local.current_version
        );
        return Ok(None);
    };
    let version_order = compare_versions(&local.current_version, &fresh.app_version);
    if if reactivation_reinstall {
        version_order == std::cmp::Ordering::Greater
    } else {
        version_order != std::cmp::Ordering::Less
    } {
        log::info!(
            "codex payload update check completed stage=compare result=up-to-date current={} latest={} source={}",
            local.current_version,
            fresh.app_version,
            fresh.identity.source_kind.as_str()
        );
        return Ok(None);
    }
    log::info!(
        "codex payload update available stage=compare current={} latest={} source={} bytes={}",
        local.current_version,
        fresh.app_version,
        fresh.identity.source_kind.as_str(),
        fresh.msix.size
    );
    let payload = match verified_digests.as_ref() {
        Some(digests) => {
            codex_update_payload_with_digests(&ctx.bundle_root, &local, &fresh.identity, digests)?
        }
        None => serde_json::to_value(CodexUpdatePayload {
            identity: fresh.identity.clone(),
            local_snapshot: None,
        })
        .map_err(|error| AppError::Engine(format!("序列化重装更新对象失败: {error}")))?,
    };
    Ok(Some(UpdateInfo {
        channel: ChannelKind::Codex,
        current_version: local.current_version,
        latest_version: fresh.app_version,
        release_notes: None,
        size_bytes: Some(fresh.msix.size),
        requires_restart: false,
        payload,
    }))
}

pub fn apply(
    ctx: &UpdateContext,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
) -> Result<(), AppError> {
    apply_inner(ctx, expected, progress, false)
}

/// Apply a Codex payload repair only while the internal USB revocation gate
/// confirms an active receipt newer than the retained revoked history. This
/// is not a renderer-selectable mode and does not establish a launch lease.
pub fn apply_reinstall(
    ctx: &UpdateContext,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
) -> Result<(), AppError> {
    apply_inner(ctx, expected, progress, true)
}

fn require_reinstall_authorization_while_locked(
    phase: &str,
    deferred_cleanup: &mut Option<crate::app::usb_guard::DeferredRevocationCleanup>,
) -> Result<crate::app::usb_guard::SignedUsbAuthorization, AppError> {
    match crate::app::usb_guard::require_signed_codex_reinstall_bundle_deferred(phase)? {
        crate::app::usb_guard::ReinstallGateOutcome::Authorized(authorization) => Ok(authorization),
        crate::app::usb_guard::ReinstallGateOutcome::DeferredCleanup(request) => {
            *deferred_cleanup = Some(*request);
            Err(AppError::Engine(
                "usb_revocation_cleanup_deferred".to_string(),
            ))
        }
    }
}

fn after_update_locks_release<T>(
    transaction_guard: crate::app::usb_revocation::InstallationTransactionGuard,
    operation_guard: std::sync::MutexGuard<'static, ()>,
    action: impl FnOnce() -> T,
) -> T {
    drop(transaction_guard);
    drop(operation_guard);
    action()
}

fn apply_inner(
    ctx: &UpdateContext,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    reactivation_reinstall: bool,
) -> Result<(), AppError> {
    // Security boundary: this must remain the first executable statement.
    let initial_reinstall_authorization = if reactivation_reinstall {
        Some(
            crate::app::usb_guard::require_signed_codex_reinstall_bundle(
                "codex-payload-reinstall-apply",
            )?,
        )
    } else {
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-apply")?;
        None
    };
    require_windows()?;
    log::info!(
        "codex payload update apply start current={} expected_latest={} expected_source={}",
        expected.current_version,
        expected.latest_version,
        parse_codex_update_payload(&expected.payload)
            .map(|payload| payload.identity.source_kind.as_str().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    );
    let _operation = CODEX_UPDATE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let initial_authorization = if reactivation_reinstall {
        initial_reinstall_authorization
            .as_ref()
            .cloned()
            .expect("reactivation gate produced an authorization")
    } else {
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-transaction-lock")?
            .ok_or_else(|| AppError::Engine("当前进程不是受管 USB bundle".to_string()))?
    };
    let transaction_guard = crate::app::usb_revocation::acquire_codex_update_transaction_lock(
        &initial_authorization.installation_id,
    )
    .map_err(|error| AppError::Engine(format!("无法锁定 USB 安装事务：{error}")))?;
    if transaction_guard.installation_id() != initial_authorization.installation_id {
        return Err(AppError::Engine("USB 安装事务锁身份不匹配".to_string()));
    }
    let mut deferred_cleanup = None;
    macro_rules! reinstall_gate_or_cleanup {
        ($phase:expr) => {{
            match require_reinstall_authorization_while_locked($phase, &mut deferred_cleanup) {
                Ok(authorization) => authorization,
                Err(error) => {
                    if let Some(request) = deferred_cleanup.take() {
                        return after_update_locks_release(transaction_guard, _operation, || {
                            request.cleanup_and_reject().map(|_| ())
                        });
                    }
                    return Err(error);
                }
            }
        }};
    }
    // Authorization may have changed while waiting for another update or
    // cleanup process. Re-read it only after holding the installation lock;
    // this fresh gate is the authority for recovery and all later writes.
    let mut current_authorization = if reactivation_reinstall {
        reinstall_gate_or_cleanup!("codex-payload-reinstall-transaction-lock")
    } else {
        crate::app::usb_guard::require_signed_codex_bundle(
            "codex-payload-transaction-lock-recheck",
        )?
        .ok_or_else(|| AppError::Engine("当前进程不是受管 USB bundle".to_string()))?
    };
    if current_authorization.installation_id != initial_authorization.installation_id
        || current_authorization.bundle_root != initial_authorization.bundle_root
        || current_authorization.fingerprint != initial_authorization.fingerprint
        || current_authorization.license_digest != initial_authorization.license_digest
    {
        return Err(AppError::StaleExpectation(
            "授权 U 盘或许可证在等待安装事务锁时发生变化，请重新检查。".to_string(),
        ));
    }
    let observed_root = current_authorization.bundle_root.clone();
    let expected_root =
        fs::canonicalize(&ctx.bundle_root).map_err(|error| AppError::Engine(error.to_string()))?;
    let observed_root =
        fs::canonicalize(observed_root).map_err(|error| AppError::Engine(error.to_string()))?;
    if expected_root != observed_root {
        return Err(AppError::StaleExpectation(
            "授权 U 盘已变化，请重新检查后再确认。".to_string(),
        ));
    }
    recover_pending_transactions(&ctx.bundle_root)?;
    // Re-observe after recovery as well. Recovery is the first mutating
    // operation and a drive removal/reuse during it must not be followed by
    // capability reads or a network-backed swap.
    let post_recovery_authorization = if reactivation_reinstall {
        reinstall_gate_or_cleanup!("codex-payload-reinstall-post-recovery")
    } else {
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-post-recovery")?
            .ok_or_else(|| {
                AppError::Engine(
                    "anchor_postrecovery_failed：恢复事务后未能重新取得 USB 授权".to_string(),
                )
            })?
    };
    if post_recovery_authorization.installation_id != current_authorization.installation_id
        || post_recovery_authorization.bundle_root != current_authorization.bundle_root
        || post_recovery_authorization.fingerprint != current_authorization.fingerprint
        || post_recovery_authorization.license_digest != current_authorization.license_digest
    {
        return Err(AppError::StaleExpectation(
            "授权 U 盘在恢复事务期间发生变化，请重新检查后再确认。".to_string(),
        ));
    }
    current_authorization = post_recovery_authorization;
    let post_recovery_root = current_authorization.bundle_root.clone();
    let post_recovery_root = fs::canonicalize(post_recovery_root)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    if expected_root != post_recovery_root {
        return Err(AppError::StaleExpectation(
            "授权 U 盘已变化，请重新检查后再确认。".to_string(),
        ));
    }
    if expected.channel != ChannelKind::Codex || expected.latest_version.trim().is_empty() {
        return Err(AppError::StaleExpectation(
            "更新确认对象无效，请重新检查后再确认。".to_string(),
        ));
    }
    // Capability metadata is cheap enough to establish the local channel
    // identity, but the full payload read is deliberately postponed until
    // after the fresh remote identity has been resolved.
    let local = (if reactivation_reinstall {
        require_capability_metadata_for_reinstall(&ctx.bundle_root)
    } else {
        require_capability_metadata(&ctx.bundle_root)
    })
    .map_err(|error| {
        log::error!(
            "codex payload update apply failed stage=capability code={} error={}",
            error.code(),
            error
        );
        error
    })?;
    let expected_payload = parse_codex_update_payload(&expected.payload)?;
    let expected_identity = expected_payload.identity;
    let expected_snapshot = if reactivation_reinstall {
        if expected_payload.local_snapshot.is_some() {
            return Err(AppError::StaleExpectation(
                "受限重装确认对象不得携带普通更新快照，请重新检查。".to_string(),
            ));
        }
        None
    } else {
        Some(expected_payload.local_snapshot.ok_or_else(|| {
            AppError::StaleExpectation(
                "更新确认缺少本地 USB 快照，请重新检查后再确认。".to_string(),
            )
        })?)
    };
    let source_matches = source_matches_settings(&ctx.settings, &expected_identity)?;
    if !source_matches || expected.current_version != local.current_version {
        log::warn!(
            "codex payload update apply rejected stage=source-lock reason=source_identity_changed source={}",
            expected_identity.source_kind.as_str()
        );
        return Err(AppError::StaleExpectation(
            "更新源或本地版本已变化，请重新检查后再确认。".to_string(),
        ));
    }
    let network = network_from_settings(&ctx.settings);
    let mut resolver = |settings: &AppSettings,
                        architecture: &str,
                        network: &NetworkConfig,
                        source_kind: CodexPayloadSourceKind| {
        resolve_source(settings, architecture, network, source_kind)
    };
    log::info!(
        "codex payload update apply stage=source-resolve start architecture={} current={}",
        local.architecture,
        local.current_version
    );
    let fresh = resolve_source_with_retry(
        &ctx.settings,
        &local.architecture,
        &network,
        expected_identity.source_kind,
        &mut resolver,
    )
    .map_err(|error| {
        log::error!(
            "codex payload update apply failed stage=source-resolve current={} code={} error={}",
            local.current_version,
            error.code(),
            error
        );
        error
    })?;
    log::info!(
        "codex payload update apply stage=source-resolve completed latest={} source={}",
        fresh.app_version,
        fresh.identity.source_kind.as_str()
    );
    if let Some(reason) = identity_mismatch_reason(&expected_identity, &fresh.identity) {
        log::warn!(
            "codex payload update apply rejected stage=source-resolve reason=source_identity_changed source={} detail={}",
            expected_identity.source_kind.as_str(),
            reason
        );
        return Err(AppError::StaleExpectation(format!(
            "远端 Codex 更新信息已变化（{reason}），请重新检查后再确认。"
        )));
    }
    if fresh.app_version != expected.latest_version {
        return Err(AppError::StaleExpectation(
            "远端 Codex 显示版本已变化，请重新检查后再确认。".to_string(),
        ));
    }
    if reactivation_reinstall {
        require_not_older_version(&local.current_version, &fresh.app_version)?;
    } else {
        require_newer_version(&local.current_version, &fresh.app_version)?;
    }
    require_https_url(&fresh.msix.url, "MSIX 下载地址")?;
    require_https_url(&fresh.checksums.url, "SHA256SUMS 下载地址")?;
    log::info!(
        "codex payload update apply stage=integrity start payload={}",
        local.payload_relative
    );
    let verified_digests = if reactivation_reinstall {
        reinstall_gate_or_cleanup!("codex-payload-reinstall-integrity");
        None
    } else {
        let verified_digests =
            verify_bundle_integrity_with_digests(&ctx.bundle_root).map_err(|error| {
                log::error!(
                    "codex payload update apply failed stage=integrity current={} code={} error={}",
                    local.current_version,
                    error.code(),
                    error
                );
                error
            })?;
        let payload_sha = verified_digest(&verified_digests, &local.payload_relative, "payload")?;
        let payload_size = fs::metadata(&local.payload_path)
            .map_err(|error| AppError::Engine(format!("读取 payload 大小失败: {error}")))?
            .len();
        verify_metadata_values(&ctx.bundle_root, &payload_sha, payload_size).map_err(|error| {
            log::error!(
                "codex payload update apply failed stage=integrity-metadata current={} code={} error={}",
                local.current_version,
                error.code(),
                error
            );
            error
        })?;
        require_local_snapshot_from_digests(
            &ctx.bundle_root,
            &local,
            expected_snapshot.as_ref().expect("normal update snapshot"),
            &verified_digests,
        )
        .map_err(|error| {
            log::error!(
                "codex payload update apply failed stage=snapshot current={} code={} error={}",
                local.current_version,
                error.code(),
                error
            );
            error
        })?;
        Some(verified_digests)
    };
    log::info!(
        "codex payload update apply stage=integrity completed files={}",
        verified_digests.as_ref().map_or(0, BTreeMap::len)
    );
    reject_reparse_chain(&ctx.bundle_root)?;
    let staging = ctx.bundle_root.join(STAGING_DIR);
    if staging.exists() {
        reject_reparse(&staging)?;
        if !staging.is_dir() {
            return Err(AppError::Engine("U 盘事务路径不是目录".to_string()));
        }
    } else {
        fs::create_dir(&staging)
            .map_err(|error| AppError::Engine(format!("创建事务目录失败: {error}")))?;
        reject_reparse(&staging)?;
    }
    let transaction_id = uuid::Uuid::new_v4().to_string();
    let tx = staging.join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
    let owner = transaction_owner_for_authorization(&ctx.bundle_root, &tx, &current_authorization)?;
    write_owner_sidecar(&staging, &owner)?;
    fs::create_dir(&tx).map_err(|error| AppError::Engine(error.to_string()))?;
    reject_reparse_chain(&tx)?;
    let recovery = if reactivation_reinstall {
        Some(RecoveryTransactionState {
            prior_payload_missing: true,
            transaction_id: transaction_id.clone(),
            installation_id: current_authorization.installation_id.clone(),
            license_signature_sha256: current_license_signature_sha256(
                &ctx.bundle_root,
                &current_authorization,
            )?,
            fingerprint_sha256: codex_usb_license::fingerprint_digest(
                &current_authorization.fingerprint,
            )
            .map_err(|error| AppError::Engine(format!("计算 USB 指纹摘要失败：{error}")))?,
            fingerprint: current_authorization.fingerprint.clone(),
            target_relative_path: local.payload_relative.clone(),
            staged_payload_sha256: None,
            legacy_bundle_root: None,
        })
    } else {
        None
    };
    let marker_result = write_transaction_state_with_owner(
        &tx,
        "owner-registered",
        recovery.clone(),
        Some(owner.clone()),
    );
    if let Err(error) = marker_result {
        if fs::remove_dir_all(&tx).is_ok() {
            let _ = remove_owner_sidecar_after_transaction(
                &ctx.bundle_root,
                &transaction_id,
                Some(&owner),
            );
        }
        return Err(error);
    }
    // The verified owner marker is durable and has been read back before any
    // snapshot or downloaded/update content is created in this directory.
    let transaction_state_result = (|| {
        fs::create_dir(tx.join("old"))
            .map_err(|error| AppError::Engine(format!("创建事务旧快照目录失败: {error}")))?;
        fs::create_dir(tx.join("next"))
            .map_err(|error| AppError::Engine(format!("创建事务新快照目录失败: {error}")))?;
        snapshot_metadata(&ctx.bundle_root, &tx)?;
        write_transaction_state_with_recovery(&tx, "prepared", recovery)
    })();
    if let Err(error) = transaction_state_result {
        if fs::remove_dir_all(&tx).is_ok() {
            let _ = remove_owner_sidecar_after_transaction(
                &ctx.bundle_root,
                &transaction_id,
                Some(&owner),
            );
        }
        return Err(error);
    }
    let result = apply_transaction(
        &ctx.bundle_root,
        &tx,
        &local,
        &fresh,
        expected,
        progress,
        &network,
        reactivation_reinstall,
        &mut deferred_cleanup,
    );
    if let Err(error) = result {
        log::error!(
            "codex payload update failed stage=download-verify-commit current={} expected_latest={} code={} error={}",
            expected.current_version,
            expected.latest_version,
            error.code(),
            error
        );
        if matches!(error, AppError::Committed(_)) {
            return Err(error);
        }
        let rollback = rollback_transaction(&ctx.bundle_root, &tx);
        if let Some(request) = deferred_cleanup.take() {
            if let Err(rollback_error) = rollback {
                log::error!("更新被作废状态阻止，事务回滚仍需重试：{rollback_error}");
            }
            return after_update_locks_release(transaction_guard, _operation, || {
                request.cleanup_and_reject().map(|_| ())
            });
        }
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(AppError::Engine(format!(
                "Codex 更新失败且恢复失败：{error}; {rollback_error}"
            ))),
        };
    }
    log::info!(
        "codex payload update completed stage=commit current={} latest={}",
        expected.current_version,
        expected.latest_version
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_transaction(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    network: &NetworkConfig,
    reactivation_reinstall: bool,
    deferred_cleanup: &mut Option<crate::app::usb_guard::DeferredRevocationCleanup>,
) -> Result<(), AppError> {
    let allow_redirects = fresh.identity.source_kind == CodexPayloadSourceKind::Github
        || (fresh.identity.source_kind == CodexPayloadSourceKind::Mirror
            && is_builtin_codex_mirror_base(&fresh.identity.source_locator));
    let transport = production_apply_transport(network, allow_redirects);
    apply_transaction_with_transport_mode_and_deferred(
        root,
        tx,
        local,
        fresh,
        expected,
        progress,
        &transport,
        reactivation_reinstall,
        deferred_cleanup,
    )
}

fn production_apply_transport(network: &NetworkConfig, allow_redirects: bool) -> ApplyTransport {
    let download_network = network.clone();
    let text_network = network.clone();
    ApplyTransport {
        download: Box::new(move |url, dest, max_bytes, on_progress| {
            if allow_redirects {
                download_to_with_progress_bounded_with_network(
                    url,
                    dest,
                    max_bytes,
                    on_progress,
                    &download_network,
                )
            } else {
                download_to_with_progress_bounded_with_network_no_redirect(
                    url,
                    dest,
                    max_bytes,
                    on_progress,
                    &download_network,
                )
            }
            .map_err(|error| error.to_string())
        }),
        fetch_text: Box::new(move |url| {
            if allow_redirects {
                fetch_text_with_network(url, &text_network)
            } else {
                fetch_text_with_network_no_redirect(url, &text_network)
            }
            .map_err(|error| error.to_string())
        }),
        verify_authenticode: Box::new(|path| {
            #[cfg(windows)]
            {
                let report = codex_win_engine::verify_openai_authenticode(path)
                    .map_err(|error| error.to_string())?;
                if report.is_valid_openai() {
                    Ok(())
                } else {
                    Err(format!(
                        "status={} subject={}",
                        report.status, report.subject
                    ))
                }
            }
            #[cfg(not(windows))]
            {
                let _ = path;
                Ok(())
            }
        }),
        read_identity: Box::new(|path| read_msix_identity(path).map_err(|error| error.to_string())),
    }
}

#[cfg(test)]
fn apply_transaction_with_transport(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    transport: &ApplyTransport,
) -> Result<(), AppError> {
    apply_transaction_with_transport_mode(
        root, tx, local, fresh, expected, progress, transport, false,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn apply_transaction_with_transport_mode(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    transport: &ApplyTransport,
    reactivation_reinstall: bool,
) -> Result<(), AppError> {
    let mut deferred_cleanup = None;
    apply_transaction_with_transport_mode_and_deferred(
        root,
        tx,
        local,
        fresh,
        expected,
        progress,
        transport,
        reactivation_reinstall,
        &mut deferred_cleanup,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_transaction_with_transport_mode_and_deferred(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    transport: &ApplyTransport,
    reactivation_reinstall: bool,
    deferred_cleanup: &mut Option<crate::app::usb_guard::DeferredRevocationCleanup>,
) -> Result<(), AppError> {
    let staged = tx.join(format!("codex-{}.Msix", fresh.identity.msix_version));
    reject_reparse_chain(tx)?;
    if staged.exists() {
        reject_reparse(&staged)?;
        return Err(AppError::Engine(
            "事务目录中已存在 staged payload，拒绝覆盖".to_string(),
        ));
    }
    let total = fresh.msix.size;
    log::info!(
        "codex payload update stage=download start source={} host={} bytes={}",
        fresh.identity.source_kind.as_str(),
        Url::parse(&fresh.msix.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        total
    );
    let progress_adapter = |done: u64| progress(done, total);
    (transport.download)(
        &fresh.msix.url,
        &staged,
        MAX_PACKAGE_BYTES,
        &progress_adapter,
    )
    .map_err(|error| AppError::Engine(format!("下载 MSIX 失败: {error}")))?;
    reject_reparse_chain(&staged)?;
    let actual_size = fs::metadata(&staged)
        .map_err(|error| AppError::Engine(format!("读取 staged MSIX 大小失败: {error}")))?
        .len();
    if actual_size == 0 || actual_size > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine("staged MSIX 大小无效".to_string()));
    }
    if fresh.msix.size > 0 && actual_size != fresh.msix.size {
        return Err(AppError::Engine(format!(
            "staged MSIX 大小与远端描述不一致：期望 {}，实际 {actual_size}",
            fresh.msix.size
        )));
    }
    log::info!("codex payload update stage=download completed bytes={actual_size}");
    let checksums_text = (transport.fetch_text)(&fresh.checksums.url)
        .map_err(|error| AppError::Engine(format!("获取 SHA256SUMS 失败: {error}")))?;
    if fresh.checksums.size > 0 && checksums_text.len() as u64 != fresh.checksums.size {
        return Err(AppError::Engine(
            "SHA256SUMS 大小与远端描述不一致".to_string(),
        ));
    }
    if let Some(expected_sha) = &fresh.checksums.expected_sha256 {
        let actual_checksums_sha = sha256_bytes(checksums_text.as_bytes());
        if !actual_checksums_sha.eq_ignore_ascii_case(expected_sha) {
            return Err(AppError::Engine(
                "Codex relay SHA256SUMS 文件摘要不匹配".to_string(),
            ));
        }
    }
    let expected_sha = find_msix_sha256(&checksums_text, &fresh.identity.package_moniker)
        .map_err(|error| AppError::Engine(format!("解析 SHA256SUMS 失败: {error}")))?;
    let actual_sha = sha256_file(&staged)
        .map_err(|error| AppError::Engine(format!("计算 MSIX SHA256 失败: {error}")))?;
    if let Some(expected_sha) = &fresh.msix.expected_sha256 {
        if !actual_sha.eq_ignore_ascii_case(expected_sha) {
            return Err(AppError::Engine(
                "Codex relay MSIX 文件摘要不匹配".to_string(),
            ));
        }
    }
    if !actual_sha.eq_ignore_ascii_case(&expected_sha) {
        return Err(AppError::Engine(format!(
            "MSIX SHA256 不匹配：期望 {expected_sha}，实际 {actual_sha}"
        )));
    }
    log::info!(
        "codex payload update stage=checksum verified sha256={} source={}",
        actual_sha,
        fresh.identity.source_kind.as_str()
    );
    if let Some(digest) = &fresh.identity.github_digest {
        if !actual_sha.eq_ignore_ascii_case(digest) {
            return Err(AppError::Engine(
                "GitHub digest 与本地 SHA256 不一致".to_string(),
            ));
        }
    }
    (transport.verify_authenticode)(&staged)
        .map_err(|error| AppError::Engine(format!("Authenticode 校验失败: {error}")))?;
    let identity = (transport.read_identity)(&staged)
        .map_err(|error| AppError::Engine(format!("读取 MSIX identity 失败: {error}")))?;
    validate_codex_identity(
        &identity,
        &fresh.identity.msix_version,
        Some(&local.architecture),
    )
    .map_err(|error| AppError::Engine(format!("MSIX identity 校验失败: {error}")))?;
    if !is_expected_codex_manifest_publisher(&identity.publisher) {
        return Err(AppError::Engine("MSIX publisher 不是 OpenAI".to_string()));
    }
    let signed_moniker = codex_win_engine::canonical_codex_package_moniker(&identity)
        .map_err(|error| AppError::Engine(format!("读取 MSIX package moniker 失败: {error}")))?;
    if signed_moniker != fresh.identity.package_moniker {
        return Err(AppError::Engine(
            "MSIX package moniker 与 release manifest 不一致".to_string(),
        ));
    }

    log::info!(
        "codex payload update stage=authenticode-identity verified package={} version={}",
        signed_moniker,
        fresh.identity.msix_version
    );

    commit_staged_payload(StagedCommit {
        root,
        tx,
        local,
        fresh,
        expected,
        staged: &staged,
        actual_sha: &actual_sha,
        total,
        progress,
        reactivation_reinstall,
        deferred_cleanup,
    })
}

/// The signed Store MSIX uses Microsoft's marketplace publisher subject in
/// the manifest rather than the human-readable `OpenAI` organization name.
/// Authenticode verification above remains the trust anchor; this check only
/// makes sure the manifest agrees with one of the two known OpenAI publisher
/// forms before the package moniker is compared with the release manifest.
fn is_expected_codex_manifest_publisher(publisher: &str) -> bool {
    let normalized = publisher.trim().to_ascii_lowercase();
    normalized.contains("openai")
        || normalized == codex_win_engine::OPENAI_MARKETPLACE_PUBLISHER_SUBJECT
        || normalized.starts_with(&format!(
            "{},",
            codex_win_engine::OPENAI_MARKETPLACE_PUBLISHER_SUBJECT
        ))
}

/// Commit a fully verified staged payload to the USB bundle.  Keeping this
/// phase separate from network/download verification makes the destructive
/// boundary deterministic and directly testable without a live GitHub asset.
fn commit_staged_payload(input: StagedCommit<'_>) -> Result<(), AppError> {
    let StagedCommit {
        root,
        tx,
        local,
        fresh,
        expected,
        staged,
        actual_sha,
        total,
        progress,
        reactivation_reinstall,
        deferred_cleanup,
    } = input;
    reject_reparse_chain(staged)?;
    let root_manifest_path = root.join(ROOT_MANIFEST_NAME);
    let usb_manifest_path = root.join(USB_MANIFEST_NAME);
    let sums_path = root.join(SHA256SUMS_NAME);
    // The full SHA256SUMS pass below is the authoritative pre-commit payload
    // read. Keep the final standalone payload hash immediately before the
    // rename as the independent TOCTOU boundary check.
    let latest_local = if reactivation_reinstall {
        require_capability_metadata_for_reinstall(root)?
    } else {
        require_capability_metadata(root)?
    };
    log::info!(
        "codex payload update stage=commit-preflight start payload={}",
        latest_local.payload_relative
    );
    let verified_digests = if reactivation_reinstall {
        require_reinstall_authorization_while_locked(
            "codex-payload-reinstall-commit-preflight",
            deferred_cleanup,
        )?;
        if regular_file_exists(&latest_local.payload_path)? {
            return Err(AppError::StaleExpectation(
                "恢复期间受管 payload 已由其他操作填入，请重新检查后再试。".to_string(),
            ));
        }
        None
    } else {
        let verified_digests = verify_bundle_integrity_with_digests(root)?;
        let verified_payload_sha =
            verified_digest(&verified_digests, &latest_local.payload_relative, "payload")?;
        let verified_payload_size = fs::metadata(&latest_local.payload_path)
            .map_err(|error| AppError::Engine(format!("读取 payload 大小失败: {error}")))?
            .len();
        verify_metadata_values(root, &verified_payload_sha, verified_payload_size)?;
        if let Some(expected_snapshot) =
            parse_codex_update_payload(&expected.payload)?.local_snapshot
        {
            require_local_snapshot_from_digests(
                root,
                &latest_local,
                &expected_snapshot,
                &verified_digests,
            )?;
        }
        Some(verified_digests)
    };
    if latest_local.current_version != expected.current_version
        || latest_local.payload_relative != local.payload_relative
        || latest_local.architecture != local.architecture
        || (reactivation_reinstall && regular_file_exists(&latest_local.payload_path)?)
        || (!reactivation_reinstall && !latest_local.payload_path.exists())
    {
        return Err(AppError::StaleExpectation(
            "本地 USB payload 在更新期间发生变化，请重新检查后再确认。".to_string(),
        ));
    }
    log::info!(
        "codex payload update stage=commit-preflight integrity-completed files={}",
        verified_digests.as_ref().map_or(0, BTreeMap::len)
    );
    if let Some(digests) = &verified_digests {
        let old_payload_sha = sha256_file(&latest_local.payload_path)
            .map_err(|error| AppError::Engine(error.to_string()))?;
        let manifest_sha = latest_local
            .manifest
            .pointer("/payload/sha256")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !manifest_sha.is_empty() && !manifest_sha.eq_ignore_ascii_case(&old_payload_sha) {
            return Err(AppError::StaleExpectation(
                "当前 USB payload 与 manifest 不一致，请先修复或重建安装包。".to_string(),
            ));
        }
        if !digests.contains_key(&latest_local.payload_relative) {
            return Err(AppError::StaleExpectation(
                "完整性校验未包含当前 payload，拒绝提交。".to_string(),
            ));
        }
    }
    let backup_relative = latest_local
        .usb_manifest
        .pointer("/updateCapability/backupDirectory")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::Engine("缺少 backupDirectory".to_string()))?;
    let backup_relative = normalize_relative_path(backup_relative)?;
    let backup_dir = root.join(backup_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if backup_dir.exists() {
        reject_reparse_chain(&backup_dir)?;
    } else {
        reject_reparse_chain(backup_dir.parent().unwrap_or(root))?;
    }
    crate::app::usb_bundle::validate_backup_layout(
        root,
        &latest_local.payload_relative,
        &latest_local.architecture,
    )?;
    reject_reparse_chain(tx)?;
    let staged_size = fs::metadata(staged)
        .map_err(|error| AppError::Engine(format!("读取 staged MSIX 大小失败: {error}")))?
        .len();
    if staged_size == 0 || staged_size > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine("staged MSIX 大小无效".to_string()));
    }
    if let Some(expected_sha) = fresh.msix.expected_sha256.as_deref() {
        if !actual_sha.eq_ignore_ascii_case(expected_sha) {
            return Err(AppError::StaleExpectation(
                "staged MSIX 摘要与确认结果不一致，请重新检查后再试。".to_string(),
            ));
        }
    }
    let (next_root, next_usb) = update_metadata(
        &latest_local,
        &fresh.identity,
        &fresh.app_version,
        actual_sha,
        staged_size,
    )?;
    let current_sums = fs::read(&sums_path)
        .map_err(|error| AppError::Engine(format!("读取当前 {SHA256SUMS_NAME} 失败: {error}")))?;
    let precommit_authorization = if reactivation_reinstall {
        Some(require_reinstall_authorization_while_locked(
            "codex-payload-reinstall-precommit",
            deferred_cleanup,
        )?)
    } else {
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-precommit")?
    };
    if let Some(authorization) = &precommit_authorization {
        if authorization.fingerprint.anchor_schema != crate::app::usb_guard::ANCHOR_SCHEMA_V3 {
            return Err(AppError::StaleExpectation(
                "当前 U 盘包使用旧更新锚点，必须重新打包并重新授权后才能更新。".to_string(),
            ));
        }
        let projected_anchor = crate::app::usb_guard::stable_anchor_sha256_from_bytes(
            &next_root,
            &next_usb,
            &current_sums,
        )?;
        if !projected_anchor.eq_ignore_ascii_case(&authorization.fingerprint.anchor_sha256) {
            return Err(AppError::StaleExpectation(format!(
                "anchor_precommit_failed：预提交锚点 {} 与授权锚点 {} 不一致；未修改 U 盘。",
                &projected_anchor[..12],
                &authorization.fingerprint.anchor_sha256[..12]
            )));
        }
        log::info!(
            "codex payload update stage=anchor-precommit verified schema={} anchor={}",
            authorization.fingerprint.anchor_schema,
            authorization.fingerprint.anchor_sha256
        );
    }
    // The authorization snapshot must be re-observed before any transaction
    // directory mutation or backup move.  The later check immediately before
    // replacing the live payload remains necessary because the preparation
    // work itself can take long enough for the removable volume to change.
    if let Some(authorization) = &precommit_authorization {
        let expected_root = fs::canonicalize(root)
            .map_err(|error| AppError::Engine(format!("解析 Codex bundle 根目录失败：{error}")))?;
        let authorized_root = fs::canonicalize(&authorization.bundle_root).map_err(|error| {
            AppError::Engine(format!("解析授权 USB bundle 根目录失败：{error}"))
        })?;
        if expected_root != authorized_root {
            return Err(AppError::StaleExpectation(
                "授权 U 盘在 payload 提交前发生变化，请重新检查后再试。".to_string(),
            ));
        }
    }
    if !reactivation_reinstall {
        fs::create_dir_all(tx.join("backup-dir").join("payload-backup"))
            .map_err(|error| AppError::Engine(error.to_string()))?;
        reject_reparse_chain(&tx.join("backup-dir"))?;
        reject_reparse_chain(&tx.join("backup-dir").join("payload-backup"))?;
        if backup_dir.exists() {
            for entry in
                fs::read_dir(&backup_dir).map_err(|error| AppError::Engine(error.to_string()))?
            {
                let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
                reject_reparse(&entry.path())?;
                if !entry
                    .file_type()
                    .map_err(|error| AppError::Engine(error.to_string()))?
                    .is_file()
                {
                    return Err(AppError::Engine("backup 目录包含非文件项".to_string()));
                }
                fs::rename(
                    entry.path(),
                    tx.join("backup-dir")
                        .join("payload-backup")
                        .join(entry.file_name()),
                )
                .map_err(|error| AppError::Engine(format!("暂存旧 backup 失败: {error}")))?;
            }
        } else {
            fs::create_dir_all(&backup_dir).map_err(|error| AppError::Engine(error.to_string()))?;
            reject_reparse_chain(&backup_dir)?;
        }
    }
    let old_version = latest_local
        .manifest
        .get("packageVersion")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&latest_local.current_version);
    let backup_path = backup_dir.join(format!(
        "OpenAI.Codex_{}_{}.msix",
        latest_local.architecture,
        sanitize_version(old_version)
    ));
    // Re-observe immediately before the first destructive rename. The earlier
    // check protects the read/verify phase; this one closes the final
    // drive-letter/replacement window after download and metadata preparation.
    if let Some(authorization) = &precommit_authorization {
        let expected_root = fs::canonicalize(root)
            .map_err(|error| AppError::Engine(format!("解析 Codex bundle 根目录失败：{error}")))?;
        let authorized_root = fs::canonicalize(&authorization.bundle_root).map_err(|error| {
            AppError::Engine(format!("解析授权 USB bundle 根目录失败：{error}"))
        })?;
        if expected_root != authorized_root {
            return Err(AppError::StaleExpectation(
                "授权 U 盘在 payload 提交前发生变化，请重新检查后再试。".to_string(),
            ));
        }
    }
    if reactivation_reinstall {
        reject_reparse_chain(&latest_local.payload_path)?;
        if latest_local.payload_path.exists() {
            return Err(AppError::StaleExpectation(
                "恢复目标在提交前已出现，拒绝覆盖。".to_string(),
            ));
        }
        write_commit_transaction_state(tx, "payload-swap-pending", true, actual_sha)?;
        verify_reactivation_transaction_boundary(root, tx)?;
    } else {
        fs::rename(&latest_local.payload_path, &backup_path)
            .map_err(|error| AppError::Engine(format!("备份旧 MSIX 失败: {error}")))?;
        write_transaction_state(tx, "payload-backup")?;
    }
    fs::rename(staged, &latest_local.payload_path)
        .map_err(|error| AppError::Engine(format!("替换 MSIX 失败: {error}")))?;
    write_commit_transaction_state(tx, "payload-swapped", reactivation_reinstall, actual_sha)?;

    let payload_size = fs::metadata(&latest_local.payload_path)
        .map_err(|error| AppError::Engine(error.to_string()))?
        .len();
    if payload_size != staged_size {
        return Err(AppError::Engine(
            "替换后的 payload 大小与预提交 staged 大小不一致".to_string(),
        ));
    }
    let mut overrides = BTreeMap::new();
    overrides.insert(ROOT_MANIFEST_NAME.to_string(), next_root.clone());
    overrides.insert(USB_MANIFEST_NAME.to_string(), next_usb.clone());
    // Remove atomic-writer leftovers before constructing the next complete
    // checksum set; otherwise the set would list files that are deleted just
    // after commit.
    clear_writer_artifacts(root)?;
    let next_sums = build_sha256_sums(root, &overrides)?;
    verify_sha256_sums(root, &next_sums, &overrides)?;
    fs::write(tx.join("next").join(ROOT_MANIFEST_NAME), &next_root)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    fs::write(tx.join("next").join(USB_MANIFEST_NAME), &next_usb)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    fs::write(tx.join("next").join(SHA256SUMS_NAME), &next_sums)
        .map_err(|error| AppError::Engine(error.to_string()))?;
    write_commit_transaction_state(tx, "metadata-ready", reactivation_reinstall, actual_sha)?;
    if reactivation_reinstall {
        verify_reactivation_transaction_boundary(root, tx)?;
    }
    replace_file(
        &tx.join("next").join(ROOT_MANIFEST_NAME),
        &root_manifest_path,
    )?;
    write_commit_transaction_state(tx, "manifest-committed", reactivation_reinstall, actual_sha)?;
    if reactivation_reinstall {
        verify_reactivation_transaction_boundary(root, tx)?;
    }
    replace_file(&tx.join("next").join(USB_MANIFEST_NAME), &usb_manifest_path)?;
    write_commit_transaction_state(
        tx,
        "usb-manifest-committed",
        reactivation_reinstall,
        actual_sha,
    )?;
    if reactivation_reinstall {
        verify_reactivation_transaction_boundary(root, tx)?;
    }
    replace_file(&tx.join("next").join(SHA256SUMS_NAME), &sums_path)?;
    verify_sha256_sums(root, &next_sums, &BTreeMap::new())?;
    verify_metadata_values(root, actual_sha, payload_size)?;
    if let Some(precommit) = &precommit_authorization {
        let postcommit = if reactivation_reinstall {
            require_reinstall_authorization_while_locked(
                "codex-payload-reinstall-postcommit",
                deferred_cleanup,
            )?
        } else {
            crate::app::usb_guard::require_signed_codex_bundle("codex-payload-postcommit")?
                .ok_or_else(|| {
                    AppError::Engine(
                        "anchor_postcommit_failed：正式更新完成后未能重新取得 USB 授权".to_string(),
                    )
                })?
        };
        if postcommit.fingerprint.anchor_schema != precommit.fingerprint.anchor_schema
            || !postcommit
                .fingerprint
                .anchor_sha256
                .eq_ignore_ascii_case(&precommit.fingerprint.anchor_sha256)
            || postcommit.license_digest != precommit.license_digest
            || postcommit.installation_id != precommit.installation_id
            || postcommit.key_id != precommit.key_id
        {
            return Err(AppError::Engine(
                "anchor_postcommit_failed：更新后授权锚点或许可证快照发生变化".to_string(),
            ));
        }
        log::info!(
            "codex payload update stage=anchor-postcommit verified schema={} anchor={}",
            postcommit.fingerprint.anchor_schema,
            postcommit.fingerprint.anchor_sha256
        );
    }
    write_commit_transaction_state(tx, "committed", reactivation_reinstall, actual_sha)?;
    let committed_state = read_transaction_state(tx)?;
    let transaction_id = committed_state
        .owner
        .as_ref()
        .map(|owner| owner.transaction_id.as_str())
        .unwrap_or_else(|| transaction_id_from_path(tx))
        .to_string();
    fs::remove_dir_all(tx).map_err(|error| {
        AppError::Committed(format!("无法清理已提交事务目录 {}：{error}", tx.display()))
    })?;
    remove_owner_sidecar_after_transaction(root, &transaction_id, committed_state.owner.as_ref())
        .map_err(|error| AppError::Committed(error.to_string()))?;
    let staging = root.join(STAGING_DIR);
    if staging.exists()
        && fs::read_dir(&staging)
            .map_err(|error| AppError::Engine(error.to_string()))?
            .next()
            .is_none()
    {
        reject_reparse_chain(&staging)?;
        if let Err(error) = fs::remove_dir(&staging) {
            return Err(AppError::Committed(format!(
                "无法删除空事务目录 {}：{error}",
                staging.display()
            )));
        }
    }
    progress(payload_size, total);
    Ok(())
}

fn sanitize_version(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '.' | '-' | '_') {
                char
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::settings_store::DEFAULT_CODEX_PAYLOAD_BASE_URL;
    use base64::Engine as _;

    fn v2_fixture() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("codex-update-fixture-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("payload")).unwrap();
        let payload = b"old payload";
        fs::write(root.join("payload/OpenAI.Codex_x64.msix"), payload).unwrap();
        let payload_hash = sha256_bytes(payload);
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "codexDesktopVersion": "1.0.0",
            "packageVersion": "1.0.0.0",
            "architecture": "x64",
            "updateCapability": {"codexPayload": true, "schemaVersion": 3, "anchorSchema": 3},
            "payload": {"path": "payload/OpenAI.Codex_x64.msix", "sha256": payload_hash, "size": payload.len()}
        });
        let usb = serde_json::json!({
            "schemaVersion": 2,
            "updateCapability": {
                "codexPayload": true, "schemaVersion": 3, "anchorSchema": 3,
                "checksumFile": "SHA256SUMS.txt", "transactionDirectory": ".staging",
                "backupDirectory": "payload/_backup",
                "backupPolicy": "single-msix-listed-in-SHA256SUMS", "successState": "clean"
            },
            "codexInstaller": {
                "payload": "payload/OpenAI.Codex_x64.msix",
                "payloadSha256": payload_hash,
                "payloadSize": payload.len()
            }
        });
        fs::write(
            root.join(ROOT_MANIFEST_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(USB_MANIFEST_NAME),
            serde_json::to_vec_pretty(&usb).unwrap(),
        )
        .unwrap();
        let sums = build_sha256_sums(&root, &BTreeMap::new()).unwrap();
        fs::write(root.join(SHA256SUMS_NAME), sums).unwrap();
        root
    }

    #[test]
    fn checksum_update_excludes_only_exact_revocation_runtime_state() {
        let root = v2_fixture();
        let original_sums = fs::read(root.join(SHA256SUMS_NAME)).unwrap();
        let state_dir = root.join("CodexUsbState");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("revocation.json"), b"{\"state\":\"active\"}").unwrap();
        fs::write(state_dir.join("revocation.json.tmp-321-7"), b"pending").unwrap();
        let registration_name = codex_usb_license::REVOCATION_REGISTRATION_FILE_NAME;
        fs::write(root.join(registration_name), b"registration").unwrap();

        verify_sha256_sums(&root, &original_sums, &BTreeMap::new()).unwrap();
        let updated_sums = build_sha256_sums(&root, &BTreeMap::new()).unwrap();
        let updated_text = String::from_utf8(updated_sums.clone()).unwrap();
        assert!(!updated_text.contains("CodexUsbState/"));
        assert!(!updated_text.contains(registration_name));
        verify_sha256_sums(&root, &updated_sums, &BTreeMap::new()).unwrap();

        for mutable_path in [
            registration_name,
            "CodexUsbState/revocation.json",
            "CodexUsbState/revocation.json.tmp-321-7",
        ] {
            let file = root.join(mutable_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let hash = sha256_file(&file).unwrap();
            let listed_runtime_state = format!("{updated_text}{hash}  {mutable_path}\n");
            assert!(
                verify_sha256_sums(&root, listed_runtime_state.as_bytes(), &BTreeMap::new())
                    .is_err()
            );
        }

        let linked_temp = state_dir.join("revocation.json.tmp-321-8");
        fs::hard_link(root.join(ROOT_MANIFEST_NAME), &linked_temp).unwrap();
        assert!(verify_sha256_sums(&root, &updated_sums, &BTreeMap::new()).is_err());
        assert!(build_sha256_sums(&root, &BTreeMap::new()).is_err());
        fs::remove_file(&linked_temp).unwrap();

        for mutable_path in [registration_name, "CodexUsbState/revocation.json"] {
            let sidecar = root.join(mutable_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::remove_file(&sidecar).unwrap();
            fs::hard_link(root.join(ROOT_MANIFEST_NAME), &sidecar).unwrap();
            assert!(verify_sha256_sums(&root, &updated_sums, &BTreeMap::new()).is_err());
            assert!(build_sha256_sums(&root, &BTreeMap::new()).is_err());
            fs::remove_file(sidecar).unwrap();
        }

        let near_match = state_dir.join("revocation.json.tmp-0321-7");
        fs::write(&near_match, b"not a recognized atomic temp").unwrap();
        assert!(verify_sha256_sums(&root, &updated_sums, &BTreeMap::new()).is_err());
        assert!(build_sha256_sums(&root, &BTreeMap::new()).is_err());
        fs::remove_file(&near_match).unwrap();

        let unknown = state_dir.join("operator-note.json");
        fs::write(&unknown, b"unlisted state").unwrap();
        assert!(verify_sha256_sums(&root, &updated_sums, &BTreeMap::new()).is_err());
        assert!(build_sha256_sums(&root, &BTreeMap::new()).is_err());
        let _ = fs::remove_dir_all(root);
    }

    fn revocation_fixture_identity() -> (
        crate::app::usb_guard::UsbFingerprint,
        crate::app::usb_guard::UsbLicense,
    ) {
        let fingerprint = crate::app::usb_guard::UsbFingerprint {
            volume_serial: 77,
            anchor_sha256: "a".repeat(64),
            bundle_relative_path: "CodexUSB".to_string(),
            anchor_schema: 3,
        };
        let license = crate::app::usb_guard::UsbLicense {
            schema_version: codex_usb_license::LICENSE_SCHEMA_VERSION,
            product: codex_usb_license::PRODUCT_ID.to_string(),
            installation_id: "fixture-installation".to_string(),
            fingerprint: fingerprint.clone(),
            issued_at: "1".to_string(),
            key_id: Some("fixture-key".to_string()),
            account_binding: None,
            signature: Some(base64::engine::general_purpose::STANDARD.encode([0u8; 64])),
        };
        (fingerprint, license)
    }

    #[test]
    fn deferred_cleanup_runs_only_after_update_locks_are_released() {
        let installation_id = format!("deferred-cleanup-test-{}", uuid::Uuid::new_v4());
        let operation_guard = CODEX_UPDATE_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let transaction_guard =
            crate::app::usb_revocation::acquire_codex_update_transaction_lock(&installation_id)
                .unwrap();
        let acquired = after_update_locks_release(transaction_guard, operation_guard, || {
            let id = installation_id.clone();
            std::thread::spawn(move || {
                crate::app::usb_revocation::acquire_revocation_cleanup_transaction_lock(&id).is_ok()
            })
            .join()
            .unwrap()
        });
        assert!(
            acquired,
            "cleanup must reacquire the production lock after deferred release"
        );
    }

    #[test]
    fn transaction_and_recovery_markers_bind_portable_usb_identity_not_drive_letter() {
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let license_sha = codex_usb_license::license_signature_sha256(&license).unwrap();
        let fingerprint_sha = codex_usb_license::fingerprint_digest(&fingerprint).unwrap();
        let owner = TransactionOwner {
            transaction_id: transaction_id.clone(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: license_sha.clone(),
            fingerprint_sha256: fingerprint_sha.clone(),
            legacy_bundle_root: None,
        };
        let recovery = RecoveryTransactionState {
            prior_payload_missing: true,
            transaction_id: transaction_id.clone(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: license_sha.clone(),
            fingerprint_sha256: fingerprint_sha.clone(),
            fingerprint: fingerprint.clone(),
            target_relative_path: "payload/OpenAI.Codex_Test.msix".to_string(),
            staged_payload_sha256: None,
            legacy_bundle_root: None,
        };

        // `D:\\One` and `F:\\One` are only current-session locators. Neither
        // marker persists those strings; the volume/path binding comes from
        // the stable fingerprint digest and remains identical after relabel.
        let serialized_owner = serde_json::to_string(&owner).unwrap();
        let serialized_recovery = serde_json::to_string(&recovery).unwrap();
        let current_path_d = PathBuf::from(r"D:\One");
        let current_path_f = PathBuf::from(r"F:\One");
        assert_ne!(current_path_d, current_path_f);
        assert_eq!(
            codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            "the stable USB identity is derived from fingerprint fields, not the current drive letter"
        );
        assert!(!serialized_owner.contains("bundleRoot"));
        assert!(!serialized_recovery.contains("bundleRoot"));
        assert!(!serialized_owner.contains("D:\\\\One"));
        assert!(!serialized_owner.contains("F:\\\\One"));
        assert!(!serialized_recovery.contains("D:\\\\One"));
        assert!(!serialized_recovery.contains("F:\\\\One"));
        assert!(transaction_owner_matches(
            &owner,
            &transaction_id,
            &license.installation_id,
            &license_sha,
            &fingerprint_sha,
        ));
        assert!(recovery_owner_matches(&recovery, &owner));

        for mismatch in [
            (
                "other-tx".to_string(),
                license.installation_id.clone(),
                license_sha.clone(),
                fingerprint_sha.clone(),
            ),
            (
                transaction_id.clone(),
                "other-installation".to_string(),
                license_sha.clone(),
                fingerprint_sha.clone(),
            ),
            (
                transaction_id.clone(),
                license.installation_id.clone(),
                "f".repeat(64),
                fingerprint_sha.clone(),
            ),
            (
                transaction_id.clone(),
                license.installation_id.clone(),
                license_sha.clone(),
                "e".repeat(64),
            ),
        ] {
            assert!(!transaction_owner_matches(
                &owner,
                &mismatch.0,
                &mismatch.1,
                &mismatch.2,
                &mismatch.3,
            ));
        }

        for changed_fingerprint in [
            crate::app::usb_guard::UsbFingerprint {
                volume_serial: fingerprint.volume_serial + 1,
                ..fingerprint.clone()
            },
            crate::app::usb_guard::UsbFingerprint {
                anchor_sha256: "b".repeat(64),
                ..fingerprint.clone()
            },
            crate::app::usb_guard::UsbFingerprint {
                bundle_relative_path: "OtherBundle".to_string(),
                ..fingerprint.clone()
            },
            crate::app::usb_guard::UsbFingerprint {
                anchor_schema: fingerprint.anchor_schema - 1,
                ..fingerprint.clone()
            },
        ] {
            let changed_digest =
                codex_usb_license::fingerprint_digest(&changed_fingerprint).unwrap();
            assert!(!transaction_owner_matches(
                &owner,
                &transaction_id,
                &license.installation_id,
                &license_sha,
                &changed_digest,
            ));
            let mut changed_recovery = recovery.clone();
            changed_recovery.fingerprint = changed_fingerprint;
            assert!(!recovery_owner_matches(&changed_recovery, &owner));
        }
        let mut changed_recovery = recovery.clone();
        changed_recovery.license_signature_sha256 = "c".repeat(64);
        assert!(!recovery_owner_matches(&changed_recovery, &owner));
        let mut changed_recovery = recovery.clone();
        changed_recovery.transaction_id = uuid::Uuid::new_v4().to_string();
        assert!(!recovery_owner_matches(&changed_recovery, &owner));
        let mut changed_recovery = recovery;
        changed_recovery.installation_id.push_str("-other");
        assert!(!recovery_owner_matches(&changed_recovery, &owner));
    }

    #[test]
    fn revocation_inventory_accepts_sidecar_only_partial_transaction() {
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let owner = TransactionOwner {
            transaction_id: transaction_id.clone(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            legacy_bundle_root: None,
        };
        inventory_revocation_staging_names(&[
            format!("{TRANSACTION_PREFIX}{transaction_id}"),
            owner_sidecar_name(&transaction_id),
        ])
        .unwrap();
        // The sidecar is created and verified before transaction contents, so
        // an empty transaction may have neither state file after interruption.
        validate_transaction_state_owner_markers(None, None, &owner).unwrap();
    }

    #[test]
    fn revocation_inventory_rejects_unknown_or_mismatched_staging_before_first_delete() {
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let owner = TransactionOwner {
            transaction_id: transaction_id.clone(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            legacy_bundle_root: None,
        };
        let mut mismatched_owner = owner.clone();
        mismatched_owner.fingerprint_sha256 = "f".repeat(64);
        let matching_state = serde_json::to_vec(&TransactionState {
            version: TRANSACTION_VERSION,
            phase: "prepared".to_string(),
            recovery: None,
            owner: Some(owner.clone()),
        })
        .unwrap();
        let mismatched_state = serde_json::to_vec(&TransactionState {
            version: TRANSACTION_VERSION,
            phase: "prepared".to_string(),
            recovery: None,
            owner: Some(mismatched_owner),
        })
        .unwrap();
        let sentinel_root = std::env::temp_dir().join(format!(
            "codex-revocation-inventory-preflight-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&sentinel_root).unwrap();
        let sentinel = sentinel_root.join("first-target-must-remain");
        fs::write(&sentinel, b"untouched").unwrap();

        let mut first_delete_started = false;
        let inventory_result = (|| {
            let names = vec![
                format!("{TRANSACTION_PREFIX}{transaction_id}"),
                owner_sidecar_name(&transaction_id),
            ];
            inventory_revocation_staging_names(&names)?;
            validate_transaction_state_owner_markers(Some(&mismatched_state), None, &owner)?;
            Ok::<_, AppError>(())
        })();
        if inventory_result.is_ok() {
            first_delete_started = true;
            fs::remove_file(&sentinel).unwrap();
        }
        assert!(inventory_result.is_err());
        assert!(!first_delete_started);
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

        let unknown_names = vec!["codex-update-notes".to_string()];
        assert!(inventory_revocation_staging_names(&unknown_names).is_err());
        assert!(!first_delete_started);
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

        let missing_sidecar = HashSet::new();
        let tx_ids = HashSet::from([uuid::Uuid::new_v4().to_string()]);
        assert!(validate_revocation_staging_owner_pairs(&tx_ids, &missing_sidecar).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

        // A matching state remains accepted; the rejection above is specific
        // to marker mismatch, not a blanket prohibition on prepared states.
        validate_transaction_state_owner_markers(Some(&matching_state), None, &owner).unwrap();
        fs::remove_dir_all(sentinel_root).unwrap();
    }

    #[test]
    fn owner_sidecar_is_exact_idempotent_and_retained_until_transaction_disappears() {
        let root =
            std::env::temp_dir().join(format!("codex-owner-sidecar-{}", uuid::Uuid::new_v4()));
        let staging = root.join(STAGING_DIR);
        fs::create_dir_all(&staging).unwrap();
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let owner = TransactionOwner {
            transaction_id: transaction_id.clone(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            legacy_bundle_root: None,
        };

        assert_eq!(
            parse_owner_sidecar_name(&owner_sidecar_name(&transaction_id)),
            Some(transaction_id.clone())
        );
        assert!(parse_owner_sidecar_name(&format!(
            "{OWNER_SIDECAR_PREFIX}{transaction_id}-extra{OWNER_SIDECAR_SUFFIX}"
        ))
        .is_none());
        assert!(parse_owner_sidecar_name(&format!(
            "{OWNER_SIDECAR_PREFIX}{}{}",
            transaction_id.to_uppercase(),
            OWNER_SIDECAR_SUFFIX
        ))
        .is_none());

        write_owner_sidecar(&staging, &owner).unwrap();
        assert!(write_owner_sidecar(&staging, &owner).is_err());
        let sidecar = owner_sidecar_path(&staging, &transaction_id);
        let bytes = read_bounded_transaction_state_file(&sidecar)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<TransactionOwner>(&bytes).unwrap(),
            owner
        );

        let tx = staging.join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
        fs::create_dir(&tx).unwrap();
        assert!(
            remove_owner_sidecar_after_transaction(&root, &transaction_id, Some(&owner),).is_err()
        );
        assert!(
            sidecar.exists(),
            "a partial transaction keeps its owner proof"
        );
        fs::remove_dir(&tx).unwrap();
        remove_owner_sidecar_after_transaction(&root, &transaction_id, Some(&owner)).unwrap();
        assert!(!sidecar.exists());
        remove_owner_sidecar_after_transaction(&root, &transaction_id, Some(&owner)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    fn write_test_transaction_state(
        root: &Path,
        transaction_id: &str,
        license: &crate::app::usb_guard::UsbLicense,
        fingerprint: &crate::app::usb_guard::UsbFingerprint,
        owner_override: Option<TransactionOwner>,
    ) -> PathBuf {
        let tx = root
            .join(STAGING_DIR)
            .join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
        fs::create_dir_all(&tx).unwrap();
        let _canonical_root = fs::canonicalize(root).unwrap();
        let owner = owner_override.or_else(|| {
            Some(TransactionOwner {
                transaction_id: transaction_id.to_string(),
                installation_id: license.installation_id.clone(),
                license_signature_sha256: codex_usb_license::license_signature_sha256(license)
                    .unwrap(),
                fingerprint_sha256: codex_usb_license::fingerprint_digest(fingerprint).unwrap(),
                legacy_bundle_root: None,
            })
        });
        write_transaction_state_with_owner(&tx, "prepared", None, owner).unwrap();
        tx
    }

    fn validate_test_transaction_inventory(
        root: &Path,
        license: &crate::app::usb_guard::UsbLicense,
        fingerprint: &crate::app::usb_guard::UsbFingerprint,
    ) -> Result<Vec<PathBuf>, AppError> {
        let root = fs::canonicalize(root).unwrap();
        let license_sha = codex_usb_license::license_signature_sha256(license).unwrap();
        let fingerprint_sha = codex_usb_license::fingerprint_digest(fingerprint).unwrap();
        let transactions = pending_transaction_dirs(&root)?;
        validate_transaction_owner_inventory(
            &root,
            license,
            &license_sha,
            &fingerprint_sha,
            &transactions,
            |tx, _relative| read_transaction_state(tx),
        )?;
        Ok(transactions)
    }

    #[test]
    fn revocation_inventory_requires_exact_bounded_owner_marker_before_deletion() {
        let root = v2_fixture();
        let sentinel = root.join("apps/Codex/keep.txt");
        fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        fs::write(&sentinel, b"preserve").unwrap();
        let (fingerprint, license) = revocation_fixture_identity();

        let transaction_id = uuid::Uuid::new_v4().to_string();
        let tx = write_test_transaction_state(&root, &transaction_id, &license, &fingerprint, None);
        assert_eq!(
            validate_test_transaction_inventory(&root, &license, &fingerprint).unwrap(),
            vec![fs::canonicalize(&tx).unwrap()]
        );
        let phase_update = write_transaction_state(&tx, "download-complete");
        assert!(phase_update.is_ok());
        assert!(read_transaction_state(&tx).unwrap().owner.is_some());

        let mut replaced_owner = read_transaction_state(&tx).unwrap().owner.unwrap();
        replaced_owner.fingerprint_sha256 = "f".repeat(64);
        let replaced = serde_json::to_vec(&TransactionState {
            version: TRANSACTION_VERSION,
            phase: "prepared".to_string(),
            recovery: None,
            owner: Some(replaced_owner),
        })
        .unwrap();
        crate::app::atomic_file::write_atomic(&tx.join("state.json"), &replaced).unwrap();
        assert!(validate_test_transaction_inventory(&root, &license, &fingerprint).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
        let _ = fs::remove_dir_all(&root);

        // Legacy v1 transaction state and forged prefix-only entries are not
        // authorization for remote cleanup, and no runtime target is touched.
        for kind in ["legacy", "notes"] {
            let root = v2_fixture();
            let sentinel = root.join("apps/Codex/keep.txt");
            fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
            fs::write(&sentinel, b"preserve").unwrap();
            let (fingerprint, license) = revocation_fixture_identity();
            if kind == "legacy" {
                let transaction_id = uuid::Uuid::new_v4().to_string();
                let tx = root
                    .join(STAGING_DIR)
                    .join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
                fs::create_dir_all(&tx).unwrap();
                write_transaction_state(&tx, "prepared").unwrap();
            } else {
                fs::create_dir_all(root.join(STAGING_DIR).join("codex-update-notes")).unwrap();
            }
            assert!(validate_test_transaction_inventory(&root, &license, &fingerprint).is_err());
            assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
            let _ = fs::remove_dir_all(root);
        }

        // A validly shaped marker tied to a different identity is preserved
        // and cannot let cleanup inventory advance.
        let root = v2_fixture();
        let sentinel = root.join("apps/Codex/keep.txt");
        fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        fs::write(&sentinel, b"preserve").unwrap();
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let mismatched = TransactionOwner {
            transaction_id: transaction_id.clone(),
            installation_id: "other-installation".to_string(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            legacy_bundle_root: None,
        };
        let _tx = write_test_transaction_state(
            &root,
            &transaction_id,
            &license,
            &fingerprint,
            Some(mismatched),
        );
        assert!(validate_test_transaction_inventory(&root, &license, &fingerprint).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn owner_registered_prepare_interrupt_is_discarded_only_for_known_shape() {
        let root = v2_fixture();
        let (fingerprint, license) = revocation_fixture_identity();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let tx = write_test_transaction_state(&root, &transaction_id, &license, &fingerprint, None);
        write_transaction_state(&tx, "owner-registered").unwrap();
        fs::create_dir(tx.join("old")).unwrap();
        fs::write(tx.join("old").join(ROOT_MANIFEST_NAME), b"snapshot").unwrap();
        fs::create_dir(tx.join("next")).unwrap();
        let state = read_transaction_state(&tx).unwrap();
        discard_unprepared_owner_transaction(&tx, &state).unwrap();
        assert!(!tx.exists());

        let forged_tx = root
            .join(STAGING_DIR)
            .join(format!("{TRANSACTION_PREFIX}{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&forged_tx).unwrap();
        let owner = TransactionOwner {
            transaction_id: forged_tx
                .file_name()
                .unwrap()
                .to_string_lossy()
                .strip_prefix(TRANSACTION_PREFIX)
                .unwrap()
                .to_string(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            legacy_bundle_root: None,
        };
        write_transaction_state_with_owner(&forged_tx, "owner-registered", None, Some(owner))
            .unwrap();
        fs::write(forged_tx.join("unexpected.bin"), b"keep").unwrap();
        let state = read_transaction_state(&forged_tx).unwrap();
        assert!(discard_unprepared_owner_transaction(&forged_tx, &state).is_err());
        assert!(forged_tx.join("unexpected.bin").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transaction_state_and_backup_are_bounded_and_regular_files() {
        let root = v2_fixture();
        let tx = root.join(STAGING_DIR).join("codex-update-state-bound");
        fs::create_dir_all(&tx).unwrap();
        let state_path = tx.join("state.json");
        fs::write(
            &state_path,
            vec![b' '; MAX_TRANSACTION_STATE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_transaction_state(&tx).is_err());

        let backup = crate::app::atomic_file::backup_path(&state_path);
        fs::write(&state_path, b"invalid json").unwrap();
        fs::write(
            &backup,
            vec![b' '; MAX_TRANSACTION_STATE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_transaction_state(&tx).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn transaction_state_rejects_reparse_marker() {
        use std::os::unix::fs::symlink;

        let root = v2_fixture();
        let tx = root.join(STAGING_DIR).join("codex-update-reparse-state");
        fs::create_dir_all(&tx).unwrap();
        let outside = root.join("outside-state.json");
        fs::write(
            &outside,
            serde_json::to_vec(&TransactionState {
                version: TRANSACTION_VERSION,
                phase: "download-complete".to_string(),
                recovery: None,
                owner: None,
            })
            .unwrap(),
        )
        .unwrap();
        symlink(&outside, tx.join("state.json")).unwrap();
        assert!(read_transaction_state(&tx).is_err());
        let _ = fs::remove_dir_all(root);
    }

    fn reactivation_recovery_state(
        _root: &Path,
        local: &LocalBundle,
        staged_payload_sha256: Option<String>,
    ) -> RecoveryTransactionState {
        let (fingerprint, license) = revocation_fixture_identity();
        RecoveryTransactionState {
            prior_payload_missing: true,
            transaction_id: uuid::Uuid::new_v4().to_string(),
            installation_id: license.installation_id.clone(),
            license_signature_sha256: codex_usb_license::license_signature_sha256(&license)
                .unwrap(),
            fingerprint_sha256: codex_usb_license::fingerprint_digest(&fingerprint).unwrap(),
            fingerprint,
            target_relative_path: local.payload_relative.clone(),
            staged_payload_sha256,
            legacy_bundle_root: None,
        }
    }

    fn owner_for_recovery(recovery: &RecoveryTransactionState) -> TransactionOwner {
        TransactionOwner {
            transaction_id: recovery.transaction_id.clone(),
            installation_id: recovery.installation_id.clone(),
            license_signature_sha256: recovery.license_signature_sha256.clone(),
            fingerprint_sha256: recovery.fingerprint_sha256.clone(),
            legacy_bundle_root: None,
        }
    }

    #[test]
    fn manifest_schema_consumer_preserves_v2_and_requires_matching_v3_policy() {
        let usb_v2 = serde_json::json!({"schemaVersion": 2});
        assert_eq!(
            require_supported_manifest_schemas(&serde_json::json!({"schemaVersion": 2}), &usb_v2)
                .unwrap(),
            2
        );
        let usb_v3 = serde_json::json!({"schemaVersion": 3});
        assert_eq!(
            require_supported_manifest_schemas(
                &serde_json::json!({
                    "schemaVersion": 3,
                    "revocationPolicy": {"registrationRequired": true}
                }),
                &usb_v3
            )
            .unwrap(),
            3
        );
        assert!(require_supported_manifest_schemas(
            &serde_json::json!({"schemaVersion": 3}),
            &usb_v3
        )
        .is_err());
        assert!(require_supported_manifest_schemas(
            &serde_json::json!({
                "schemaVersion": 3,
                "revocationPolicy": {"registrationRequired": false}
            }),
            &usb_v3
        )
        .is_err());
        assert!(require_supported_manifest_schemas(
            &serde_json::json!({"schemaVersion": 3}),
            &usb_v2
        )
        .is_err());
    }

    #[test]
    fn repo_validation_is_strict() {
        assert_eq!(
            validate_repo_name("Wangnov/codex-app-mirror").unwrap(),
            "Wangnov/codex-app-mirror"
        );
        for invalid in [
            "",
            "owner",
            "owner/repo/extra",
            "https://github.com/o/r",
            "Owner With Space/r",
        ] {
            assert!(validate_repo_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn digest_normalization_rejects_bad_values() {
        let hex = "A".repeat(64);
        assert_eq!(
            normalize_digest(Some(&format!("sha256:{hex}"))).unwrap(),
            Some(hex.to_ascii_lowercase())
        );
        assert!(normalize_digest(Some("sha256:not-a-hash")).is_err());
    }

    #[test]
    fn relay_base_url_is_confined_to_a_safe_https_origin() {
        assert_eq!(
            validate_relay_base_url("https://UPDATE.404TOKEN.XYZ/__codex_update__").unwrap(),
            "https://update.404token.xyz/__codex_update__/"
        );
        for invalid in [
            "http://update.404token.xyz/__codex_update__",
            "https://update.404token.xyz/__codex_update__?x=1",
            "https://user@update.404token.xyz/__codex_update__",
            "https://127.0.0.1/__codex_update__",
        ] {
            assert!(validate_relay_base_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn default_mirror_uses_builtin_relay_for_fallback() {
        let settings = AppSettings::default();
        assert_eq!(relay_base_url_for(&settings), LEGACY_CODEX_PAYLOAD_BASE_URL);

        let custom = AppSettings {
            codex_payload_base_url: "https://relay.example.test/codex/".to_string(),
            ..settings
        };
        assert_eq!(
            relay_base_url_for(&custom),
            "https://relay.example.test/codex/"
        );
    }

    #[test]
    fn relative_paths_are_confined() {
        assert_eq!(
            normalize_relative_path("payload/OpenAI.Codex_x64.msix").unwrap(),
            "payload/OpenAI.Codex_x64.msix"
        );
        for invalid in ["../x.msix", "/x.msix", "C:/x.msix", "payload//x.msix"] {
            assert!(normalize_relative_path(invalid).is_err());
        }
    }

    #[test]
    fn checksum_generation_is_deterministic_and_excludes_mutable_license() {
        let root = std::env::temp_dir().join(format!("codex-update-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("payload")).unwrap();
        fs::write(root.join("b.txt"), b"b").unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join(USB_LICENSE_NAME), b"mutable").unwrap();
        fs::create_dir_all(root.join(STAGING_DIR)).unwrap();
        fs::write(root.join(STAGING_DIR).join("ignored"), b"ignored").unwrap();
        fs::create_dir_all(root.join("apps").join("Codex")).unwrap();
        fs::write(
            root.join("apps").join("Codex").join("installed.exe"),
            b"runtime",
        )
        .unwrap();
        let sums = build_sha256_sums(&root, &BTreeMap::new()).unwrap();
        let text = String::from_utf8(sums).unwrap();
        assert!(text.contains("a.txt"));
        assert!(text.contains("b.txt"));
        assert!(!text.contains(USB_LICENSE_NAME));
        assert!(!text.contains(STAGING_DIR));
        assert!(!text.contains("apps/Codex"));
        assert!(text.find("a.txt") < text.find("b.txt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn identity_contains_all_stale_fields() {
        let identity = CodexPayloadIdentity {
            source_kind: CodexPayloadSourceKind::Github,
            source_locator: repo_api_url("o/r"),
            repo: "o/r".into(),
            release_id: 1,
            release_tag: "v".into(),
            msix_asset_id: Some(2),
            msix_asset_name: "a.msix".into(),
            sha256_asset_id: Some(3),
            sha256_asset_name: SHA256SUMS_NAME.into(),
            manifest_asset_id: Some(4),
            manifest_asset_name: "release-manifest.json".into(),
            package_moniker: "m".into(),
            architecture: "x64".into(),
            msix_version: "1.0.0.0".into(),
            app_version: "1.0".into(),
            msix_size: 1,
            msix_sha256: None,
            checksums_sha256: None,
            manifest_sha256: None,
            github_digest: None,
        };
        let mut changed = identity.clone();
        changed.sha256_asset_id = Some(4);
        assert!(!same_identity(&identity, &changed));
        changed = identity.clone();
        changed.manifest_asset_id = Some(5);
        assert!(!same_identity(&identity, &changed));
        changed = identity.clone();
        changed.source_kind = CodexPayloadSourceKind::Relay;
        changed.source_locator = DEFAULT_CODEX_PAYLOAD_BASE_URL.to_string();
        assert!(!same_identity(&identity, &changed));
    }

    #[test]
    fn capability_rejects_v1_and_unknown_before_network() {
        let root = v2_fixture();
        let usb_path = root.join(USB_MANIFEST_NAME);
        let mut usb: serde_json::Value =
            serde_json::from_slice(&fs::read(&usb_path).unwrap()).unwrap();
        usb["schemaVersion"] = serde_json::json!(1);
        fs::write(&usb_path, serde_json::to_vec(&usb).unwrap()).unwrap();
        assert!(require_capability(&root).is_err());
        usb["schemaVersion"] = serde_json::json!(99);
        fs::write(&usb_path, serde_json::to_vec(&usb).unwrap()).unwrap();
        assert!(require_capability(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn capability_rejects_payload_metadata_mismatch() {
        let root = v2_fixture();
        fs::write(root.join("payload/OpenAI.Codex_x64.msix"), b"tampered").unwrap();
        assert!(require_capability(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_snapshot_rejects_a_coordinated_manifest_change() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        let snapshot = local_bundle_snapshot(&root, &local).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap()).unwrap();
        manifest["unrelated"] = serde_json::json!({"changed": true});
        fs::write(
            root.join(ROOT_MANIFEST_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(require_local_snapshot(&root, &local, &snapshot).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verified_digest_table_builds_the_same_snapshot_without_a_second_payload_read() {
        let root = v2_fixture();
        let local = require_capability_metadata(&root).unwrap();
        let digests = verify_bundle_integrity_with_digests(&root).unwrap();
        let from_verified = local_bundle_snapshot_from_digests(&root, &local, &digests).unwrap();
        let independently_read =
            local_bundle_snapshot(&root, &require_capability(&root).unwrap()).unwrap();
        assert_eq!(from_verified, independently_read);

        let mut incomplete = digests;
        incomplete.remove(&local.payload_relative);
        assert!(local_bundle_snapshot_from_digests(&root, &local, &incomplete).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_update_preserves_unrelated_fields_and_real_size() {
        let root = v2_fixture();
        let mut local = require_capability(&root).unwrap();
        local.manifest["unrelated"] = serde_json::json!({"keep": true});
        let identity = CodexPayloadIdentity {
            source_kind: CodexPayloadSourceKind::Github,
            source_locator: repo_api_url("o/r"),
            repo: "o/r".into(),
            release_id: 1,
            release_tag: "release".into(),
            msix_asset_id: Some(2),
            msix_asset_name: "new.Msix".into(),
            sha256_asset_id: Some(3),
            sha256_asset_name: SHA256SUMS_NAME.into(),
            manifest_asset_id: Some(4),
            manifest_asset_name: "release-manifest.json".into(),
            package_moniker: "m".into(),
            architecture: "x64".into(),
            msix_version: "2.0.0.0".into(),
            app_version: "2.0.0".into(),
            msix_size: 123,
            msix_sha256: None,
            checksums_sha256: None,
            manifest_sha256: None,
            github_digest: None,
        };
        let (manifest, usb) =
            update_metadata(&local, &identity, "2.0.0", "a".repeat(64).as_str(), 123).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        let usb: serde_json::Value = serde_json::from_slice(&usb).unwrap();
        assert_eq!(manifest["unrelated"]["keep"], true);
        assert_eq!(manifest["payload"]["size"], 123);
        assert_eq!(usb["codexInstaller"]["payloadSize"], 123);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checksum_verification_covers_backup_and_rejects_unlisted_files() {
        let root = v2_fixture();
        fs::create_dir_all(root.join("payload/_backup")).unwrap();
        fs::write(
            root.join("payload/_backup/OpenAI.Codex_x64_1.0.0.0.msix"),
            b"backup",
        )
        .unwrap();
        let sums = build_sha256_sums(&root, &BTreeMap::new()).unwrap();
        verify_sha256_sums(&root, &sums, &BTreeMap::new()).unwrap();
        fs::write(root.join("unlisted.bin"), b"unlisted").unwrap();
        assert!(verify_sha256_sums(&root, &sums, &BTreeMap::new()).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_does_not_promote_an_unrelated_existing_backup() {
        let root = v2_fixture();
        let backup_dir = root.join("payload/_backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let unrelated = backup_dir.join("OpenAI.Codex_x64_previous.msix");
        fs::write(&unrelated, b"unrelated backup").unwrap();

        let tx = root.join(STAGING_DIR).join("codex-update-test");
        fs::create_dir_all(tx.join("old")).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        write_transaction_state(&tx, "download-complete").unwrap();
        rollback_transaction(&root, &tx).unwrap();

        assert_eq!(
            fs::read(root.join("payload/OpenAI.Codex_x64.msix")).unwrap(),
            b"old payload"
        );
        assert!(unrelated.is_file());
        assert!(!tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reactivation_rollback_removes_only_the_recorded_staged_payload_and_restores_metadata() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        let original_root = fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap();
        let original_usb = fs::read(root.join(USB_MANIFEST_NAME)).unwrap();
        let original_sums = fs::read(root.join(SHA256SUMS_NAME)).unwrap();
        fs::remove_file(&local.payload_path).unwrap();

        let staged_payload = b"verified replacement payload";
        let staged_sha = sha256_bytes(staged_payload);
        let recovery = reactivation_recovery_state(&root, &local, Some(staged_sha.clone()));
        let tx = root
            .join(STAGING_DIR)
            .join(format!("{TRANSACTION_PREFIX}{}", recovery.transaction_id));
        fs::create_dir_all(tx.join("old")).unwrap();
        fs::create_dir_all(tx.join("next")).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        let owner = owner_for_recovery(&recovery);
        write_transaction_state_with_owner(
            &tx,
            "payload-swap-pending",
            Some(recovery),
            Some(owner),
        )
        .unwrap();
        fs::write(tx.join("codex-new.Msix"), staged_payload).unwrap();
        fs::rename(tx.join("codex-new.Msix"), &local.payload_path).unwrap();

        // Model a crash after the first manifest replacement but before the
        // checksum commit. The target path remains the same and is bound to
        // the transaction's original signed identity snapshot.
        let release = fake_resolved_release();
        let (next_root, _next_usb) = update_metadata(
            &local,
            &release.identity,
            &release.app_version,
            &staged_sha,
            staged_payload.len() as u64,
        )
        .unwrap();
        fs::write(root.join(ROOT_MANIFEST_NAME), next_root).unwrap();

        rollback_transaction(&root, &tx).unwrap();

        assert!(!local.payload_path.exists());
        assert_eq!(
            fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap(),
            original_root
        );
        assert_eq!(
            fs::read(root.join(USB_MANIFEST_NAME)).unwrap(),
            original_usb
        );
        assert_eq!(fs::read(root.join(SHA256SUMS_NAME)).unwrap(), original_sums);
        assert!(!tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reactivation_rollback_fails_closed_for_unowned_target_bytes_or_path() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        fs::remove_file(&local.payload_path).unwrap();
        let expected_sha = sha256_bytes(b"our staged payload");
        let recovery = reactivation_recovery_state(&root, &local, Some(expected_sha.clone()));
        let tx = root
            .join(STAGING_DIR)
            .join(format!("{TRANSACTION_PREFIX}{}", recovery.transaction_id));
        fs::create_dir_all(tx.join("old")).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        let mut state = recovery;
        state.target_relative_path = "payload/other.msix".to_string();
        let owner = owner_for_recovery(&state);
        write_transaction_state_with_owner(&tx, "payload-swapped", Some(state), Some(owner))
            .unwrap();
        fs::write(&local.payload_path, b"unowned replacement bytes").unwrap();

        assert!(rollback_transaction(&root, &tx).is_err());
        assert_eq!(
            fs::read(&local.payload_path).unwrap(),
            b"unowned replacement bytes"
        );
        assert!(tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn committed_recovery_keeps_verified_next_state_and_cleans_transaction() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        let payload = b"new payload";
        fs::write(&local.payload_path, payload).unwrap();
        let payload_sha = sha256_bytes(payload);
        let identity = CodexPayloadIdentity {
            source_kind: CodexPayloadSourceKind::Github,
            source_locator: repo_api_url("Wangnov/codex-app-mirror"),
            repo: "Wangnov/codex-app-mirror".into(),
            release_id: 1,
            release_tag: "codex-app-2.0.0".into(),
            msix_asset_id: Some(2),
            msix_asset_name: "OpenAI.Codex_2.0.0.0_x64.Msix".into(),
            sha256_asset_id: Some(3),
            sha256_asset_name: SHA256SUMS_NAME.into(),
            manifest_asset_id: Some(4),
            manifest_asset_name: "release-manifest.json".into(),
            package_moniker: "OpenAI.Codex".into(),
            architecture: "x64".into(),
            msix_version: "2.0.0.0".into(),
            app_version: "2.0.0".into(),
            msix_size: 1,
            msix_sha256: None,
            checksums_sha256: None,
            manifest_sha256: None,
            github_digest: None,
        };
        let (next_root, next_usb) = update_metadata(
            &local,
            &identity,
            "2.0.0",
            &payload_sha,
            payload.len() as u64,
        )
        .unwrap();
        let mut overrides = BTreeMap::new();
        overrides.insert(ROOT_MANIFEST_NAME.to_string(), next_root.clone());
        overrides.insert(USB_MANIFEST_NAME.to_string(), next_usb.clone());
        let next_sums = build_sha256_sums(&root, &overrides).unwrap();

        let tx = root.join(STAGING_DIR).join("codex-update-committed");
        fs::create_dir_all(tx.join("next")).unwrap();
        fs::write(tx.join("next").join(ROOT_MANIFEST_NAME), &next_root).unwrap();
        fs::write(tx.join("next").join(USB_MANIFEST_NAME), &next_usb).unwrap();
        fs::write(tx.join("next").join(SHA256SUMS_NAME), &next_sums).unwrap();
        fs::write(root.join(ROOT_MANIFEST_NAME), &next_root).unwrap();
        fs::write(root.join(USB_MANIFEST_NAME), &next_usb).unwrap();
        fs::write(root.join(SHA256SUMS_NAME), &next_sums).unwrap();
        write_transaction_state(&tx, "committed").unwrap();

        recover_pending_transactions(&root).unwrap();

        assert!(!tx.exists());
        let recovered = require_capability(&root).unwrap();
        assert_eq!(recovered.current_version, "2.0.0");
        assert_eq!(fs::read(&recovered.payload_path).unwrap(), payload);
        verify_sha256_sums(&root, &next_sums, &BTreeMap::new()).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn committed_recovery_rolls_back_when_next_state_does_not_match() {
        let root = v2_fixture();
        let old_manifest = fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap();
        let tx = root.join(STAGING_DIR).join("codex-update-inconsistent");
        fs::create_dir_all(tx.join("old")).unwrap();
        fs::create_dir_all(tx.join("next")).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        fs::write(tx.join("next").join(ROOT_MANIFEST_NAME), b"not current").unwrap();
        write_transaction_state(&tx, "committed").unwrap();

        recover_pending_transactions(&root).unwrap();

        assert_eq!(
            fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap(),
            old_manifest
        );
        assert_eq!(
            fs::read(root.join("payload/OpenAI.Codex_x64.msix")).unwrap(),
            b"old payload"
        );
        assert!(!tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn fake_resolved_release() -> ResolvedRelease {
        ResolvedRelease {
            identity: CodexPayloadIdentity {
                source_kind: CodexPayloadSourceKind::Github,
                source_locator: repo_api_url("Wangnov/codex-app-mirror"),
                repo: "Wangnov/codex-app-mirror".into(),
                release_id: 1,
                release_tag: "codex-app-2.0.0".into(),
                msix_asset_id: Some(2),
                msix_asset_name: "OpenAI.Codex_2.0.0.0_x64.Msix".into(),
                sha256_asset_id: Some(3),
                sha256_asset_name: SHA256SUMS_NAME.into(),
                manifest_asset_id: Some(4),
                manifest_asset_name: "release-manifest.json".into(),
                package_moniker: "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0".into(),
                architecture: "x64".into(),
                msix_version: "2.0.0.0".into(),
                app_version: "2.0.0".into(),
                msix_size: 0,
                msix_sha256: None,
                checksums_sha256: None,
                manifest_sha256: None,
                github_digest: None,
            },
            app_version: "2.0.0".into(),
            msix: ResolvedAsset {
                size: 0,
                url: "https://example.invalid/codex.msix".into(),
                expected_sha256: None,
            },
            checksums: ResolvedAsset {
                size: 0,
                url: "https://example.invalid/SHA256SUMS-windows.txt".into(),
                expected_sha256: None,
            },
        }
    }

    fn fake_source_release(
        source_kind: CodexPayloadSourceKind,
        app_version: &str,
    ) -> ResolvedRelease {
        let mut release = fake_resolved_release();
        release.identity.source_kind = source_kind;
        release.identity.source_locator = match source_kind {
            CodexPayloadSourceKind::Mirror => DEFAULT_CODEX_PAYLOAD_BASE_URL.to_string(),
            CodexPayloadSourceKind::Relay => LEGACY_CODEX_PAYLOAD_BASE_URL.to_string(),
            CodexPayloadSourceKind::Github => repo_api_url(DEFAULT_CODEX_PAYLOAD_REPO),
            CodexPayloadSourceKind::Auto => "auto".to_string(),
        };
        release.identity.app_version = app_version.to_string();
        release.app_version = app_version.to_string();
        release
    }

    fn fake_update_info(root: &Path, local: &LocalBundle) -> UpdateInfo {
        let release = fake_resolved_release();
        UpdateInfo {
            channel: ChannelKind::Codex,
            current_version: "1.0.0".into(),
            latest_version: "2.0.0".into(),
            release_notes: None,
            size_bytes: None,
            requires_restart: false,
            payload: codex_update_payload(root, local, &release.identity).unwrap(),
        }
    }

    fn fake_identity() -> MsixIdentity {
        MsixIdentity {
            name: "OpenAI.Codex".into(),
            publisher: "CN=OpenAI, O=OpenAI".into(),
            version: "2.0.0.0".into(),
            processor_architecture: "x64".into(),
            resource_id: None,
        }
    }

    #[test]
    fn accepts_openai_store_marketplace_manifest_publisher() {
        assert!(is_expected_codex_manifest_publisher(
            "CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B"
        ));
        assert!(is_expected_codex_manifest_publisher("CN=OpenAI, O=OpenAI"));
        assert!(!is_expected_codex_manifest_publisher(
            "CN=Contoso, O=Contoso"
        ));
    }

    fn fake_transport(
        download: Result<Vec<u8>, String>,
        checksums: Result<String, String>,
        authenticode: Result<(), String>,
        identity: Result<MsixIdentity, String>,
    ) -> ApplyTransport {
        ApplyTransport {
            download: Box::new(
                move |_url, destination, _max_bytes, on_progress| match &download {
                    Ok(payload) => {
                        fs::write(destination, payload).map_err(|error| error.to_string())?;
                        on_progress(payload.len() as u64);
                        Ok(())
                    }
                    Err(error) => Err(error.clone()),
                },
            ),
            fetch_text: Box::new(move |_url| checksums.clone()),
            verify_authenticode: Box::new(move |_path| authenticode.clone()),
            read_identity: Box::new(move |_path| identity.clone()),
        }
    }

    fn prepare_transaction(root: &Path) -> (LocalBundle, PathBuf) {
        let local = require_capability(root).unwrap();
        let tx = root.join(STAGING_DIR).join("codex-update-transport");
        fs::create_dir_all(tx.join("old")).unwrap();
        fs::create_dir_all(tx.join("next")).unwrap();
        snapshot_metadata(root, &tx).unwrap();
        write_transaction_state(&tx, "prepared").unwrap();
        (local, tx)
    }

    fn assert_old_bundle_is_intact(root: &Path) {
        let current = require_capability(root).unwrap();
        assert_eq!(current.current_version, "1.0.0");
        assert_eq!(fs::read(&current.payload_path).unwrap(), b"old payload");
        verify_sha256_sums(
            root,
            &fs::read(root.join(SHA256SUMS_NAME)).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
    }

    fn release_json(
        msix_url: &str,
        checksums_url: &str,
        manifest_url: &str,
        digest: Option<&str>,
    ) -> String {
        serde_json::json!({
            "id": 1,
            "tag_name": "codex-app-2.0.0",
            "assets": [
                {
                    "id": 2,
                    "name": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix",
                    "size": 4,
                    "digest": digest,
                    "browser_download_url": msix_url
                },
                {
                    "id": 3,
                    "name": "SHA256SUMS-windows.txt",
                    "size": 1,
                    "browser_download_url": checksums_url
                },
                {
                    "id": 4,
                    "name": "release-manifest.json",
                    "size": 1,
                    "browser_download_url": manifest_url
                }
            ]
        })
        .to_string()
    }

    fn release_manifest_json(architecture: &str) -> String {
        let mut architectures = serde_json::Map::new();
        architectures.insert(
            architecture.to_string(),
            serde_json::json!({
                "appVersion": "2.0.0",
                "packageMoniker": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0",
                "version": "2.0.0.0",
                "architecture": architecture
            }),
        );
        serde_json::json!({
            "sources": {"windows": {"architectures": architectures}}
        })
        .to_string()
    }

    fn mirror_manifest_json(architecture: &str) -> String {
        let moniker = format!("OpenAI.Codex_2.0.0.0_{architecture}__2p2nqsd0c76g0");
        let mut architectures = serde_json::Map::new();
        architectures.insert(
            architecture.to_string(),
            serde_json::json!({
                "architecture": architecture,
                "version": "2.0.0.0",
                "appVersion": "2.0.0",
                "packageMoniker": moniker.clone(),
                "contentLength": 123,
                "downloadable": true
            }),
        );
        serde_json::json!({
            "schemaVersion": 5,
            "codexVersion": "2.0.0",
            "sources": {
                "windows": {
                    "architecture": architecture,
                    "version": "2.0.0.0",
                    "appVersion": "2.0.0",
                    "packageMoniker": moniker,
                    "architectures": architectures
                }
            }
        })
        .to_string()
    }

    #[test]
    fn resolve_mirror_latest_uses_manifest_checksums_and_architecture_aliases() {
        let base = DEFAULT_CODEX_PAYLOAD_BASE_URL;
        let manifest = mirror_manifest_json("x64");
        let package_moniker = "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0";
        let checksums = format!("{}  {package_moniker}.Msix\n", "a".repeat(64));
        let fetched = |url: &str| {
            if url.ends_with("latest/manifest") {
                Ok(manifest.clone())
            } else if url.ends_with("latest/checksums") {
                Ok(checksums.clone())
            } else {
                Err(AppError::Engine(format!("unexpected URL: {url}")))
            }
        };
        let resolved = resolve_mirror_latest(&fetched, base, "x64").unwrap();
        assert_eq!(
            resolved.identity.source_kind,
            CodexPayloadSourceKind::Mirror
        );
        assert_eq!(resolved.identity.app_version, "2.0.0");
        assert_eq!(resolved.identity.package_moniker, package_moniker);
        assert_eq!(resolved.msix.size, 123);
        assert!(resolved.msix.url.ends_with("/latest/win-x64"));
        assert!(resolved.checksums.url.ends_with("/latest/checksums"));
        assert_eq!(resolved.msix.expected_sha256, Some("a".repeat(64)));
    }

    #[test]
    fn mirror_identity_ignores_volatile_manifest_metadata_but_binds_payload_facts() {
        let base = DEFAULT_CODEX_PAYLOAD_BASE_URL;
        let resolve = |manifest: String, checksums: String| {
            let fetched = |url: &str| {
                if url.ends_with("latest/manifest") {
                    Ok(manifest.clone())
                } else if url.ends_with("latest/checksums") {
                    Ok(checksums.clone())
                } else {
                    Err(AppError::Engine(format!("unexpected URL: {url}")))
                }
            };
            resolve_mirror_latest(&fetched, base, "x64").unwrap()
        };

        let original = mirror_manifest_json("x64");
        let first_checksums = format!(
            "{}  OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix\n",
            "a".repeat(64)
        );
        let first = resolve(original.clone(), first_checksums.clone());
        let mut volatile: serde_json::Value = serde_json::from_str(&original).unwrap();
        volatile["generatedAt"] = serde_json::json!("2026-09-17T03:01:00Z");
        volatile["sources"]["windows"]["lastModified"] =
            serde_json::json!("Thu, 17 Sep 2026 03:01:46 GMT");
        let second = resolve(volatile.to_string(), first_checksums.clone());
        assert_eq!(first.identity, second.identity);

        let mut changed: serde_json::Value = serde_json::from_str(&original).unwrap();
        changed["sources"]["windows"]["architectures"]["x64"]["contentLength"] =
            serde_json::json!(124);
        let third = resolve(changed.to_string(), first_checksums.clone());
        assert_ne!(first.identity, third.identity);

        let irrelevant_asset = format!(
            "{}  OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix\n{}  unrelated.delta\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        let fourth = resolve(original, irrelevant_asset);
        assert_eq!(first.identity, fourth.identity);
    }

    #[test]
    fn source_metadata_retry_retries_only_transient_transport_failures() {
        let settings = AppSettings::default();
        let network = NetworkConfig::direct();
        let attempts = std::cell::Cell::new(0);
        let mut resolver = |_: &AppSettings,
                            _: &str,
                            _: &NetworkConfig,
                            _: CodexPayloadSourceKind| {
            let attempt = attempts.get() + 1;
            attempts.set(attempt);
            if attempt == 1 {
                Err(AppError::Engine(
                    "curl failed for host=codexapp.agentsmirror.com exit=7: stderr='Failed to connect'"
                        .to_string(),
                ))
            } else {
                Ok(fake_resolved_release())
            }
        };
        let resolved = resolve_source_with_retry(
            &settings,
            "x64",
            &network,
            CodexPayloadSourceKind::Mirror,
            &mut resolver,
        )
        .unwrap();
        assert_eq!(attempts.get(), 2);
        assert_eq!(resolved.app_version, "2.0.0");
    }

    #[test]
    fn source_metadata_rejects_invalid_app_and_package_versions() {
        let settings = AppSettings::default();
        let network = NetworkConfig::direct();
        for (invalid_app, invalid_package) in [("bad", "2.0.0.0"), ("2.0.0", "bad")] {
            let mut release = fake_resolved_release();
            release.app_version = invalid_app.to_string();
            release.identity.app_version = invalid_app.to_string();
            release.identity.msix_version = invalid_package.to_string();
            let mut resolver =
                |_: &AppSettings, _: &str, _: &NetworkConfig, _: CodexPayloadSourceKind| {
                    Ok(release.clone())
                };
            let error = resolve_source_with_retry(
                &settings,
                "x64",
                &network,
                CodexPayloadSourceKind::Mirror,
                &mut resolver,
            )
            .unwrap_err();
            assert!(error.to_string().contains("source_invalid"));
        }
    }

    #[test]
    fn resolve_latest_accepts_valid_manifest_without_network() {
        let release = release_json(
            "https://example.invalid/codex.msix",
            "https://example.invalid/SHA256SUMS-windows.txt",
            "https://example.invalid/release-manifest.json",
            Some(&format!("sha256:{}", "a".repeat(64))),
        );
        let result = resolve_latest(
            &|_| Ok(release.clone()),
            &|_| Ok(release_manifest_json("x64")),
            "Wangnov/codex-app-mirror",
            "x64",
        )
        .unwrap();
        assert_eq!(result.identity.architecture, "x64");
        assert_eq!(result.identity.app_version, "2.0.0");
    }

    #[test]
    fn resolve_relay_latest_validates_descriptor_and_derives_same_origin_assets() {
        let base = "https://update.404token.xyz/__codex_update__/";
        let manifest = release_manifest_json("x64");
        let checksums = b"test checksum file";
        let descriptor = serde_json::json!({
            "protocol": 1,
            "architecture": "x64",
            "releaseId": 77,
            "repository": "Wangnov/codex-app-mirror",
            "tag": "codex-app-2.0.0",
            "assets": {
                "checksums": {
                    "path": "SHA256SUMS-windows.txt",
                    "sha256": sha256_bytes(checksums),
                    "size": checksums.len()
                },
                "release-manifest": {
                    "path": "release-manifest.json",
                    "sha256": sha256_bytes(manifest.as_bytes()),
                    "size": manifest.len()
                },
                "windows-msix": {
                    "path": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix",
                    "sha256": "a".repeat(64),
                    "size": 123,
                    "githubDigest": format!("sha256:{}", "a".repeat(64))
                }
            }
        })
        .to_string();
        let fetched = |url: &str| {
            if url.ends_with("latest.json") {
                Ok(descriptor.clone())
            } else if url.ends_with("release-manifest.json") {
                Ok(manifest.clone())
            } else {
                Err(AppError::Engine(format!("unexpected URL: {url}")))
            }
        };
        let resolved = resolve_relay_latest(&fetched, base, "x64").unwrap();
        assert_eq!(resolved.identity.source_kind, CodexPayloadSourceKind::Relay);
        assert_eq!(resolved.identity.release_id, 77);
        assert_eq!(resolved.identity.app_version, "2.0.0");
        assert_eq!(resolved.msix.size, 123);
        assert!(resolved
            .msix
            .url
            .starts_with("https://update.404token.xyz/__codex_update__/"));
        assert_eq!(
            resolved.checksums.expected_sha256,
            Some(sha256_bytes(checksums))
        );
    }

    #[test]
    fn resolve_relay_latest_accepts_production_protocol_2_shape() {
        let base = "https://update.404token.xyz/__codex_update__/";
        let manifest = release_manifest_json("x64");
        let manifest_sha = sha256_bytes(manifest.as_bytes());
        let assets = serde_json::json!({
            "checksums": {
                "id": 11,
                "path": "SHA256SUMS-windows.txt",
                "sha256": "a".repeat(64),
                "size": 12,
                "githubDigest": format!("sha256:{}", "a".repeat(64))
            },
            "release-manifest": {
                "id": 12,
                "path": "release-manifest.json",
                "sha256": manifest_sha,
                "size": manifest.len()
            },
            "windows-msix": {
                "id": 13,
                "path": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix",
                "sha256": "c".repeat(64),
                "size": 56,
                "githubDigest": format!("sha256:{}", "c".repeat(64))
            }
        });
        let descriptor = serde_json::json!({
            "protocol": 2,
            "architecture": "x64",
            "releaseId": 77,
            "repository": "Wangnov/codex-app-mirror",
            "tag": "codex-app-2.0.0",
            "assets": assets,
            "identity": {
                "releaseId": 77,
                "tag": "codex-app-2.0.0",
                "assets": serde_json::json!({
                    "checksums": {
                        "id": 11,
                        "path": "SHA256SUMS-windows.txt",
                        "sha256": "a".repeat(64),
                        "size": 12,
                        "githubDigest": format!("sha256:{}", "a".repeat(64))
                    },
                    "release-manifest": {
                        "id": 12,
                        "path": "release-manifest.json",
                        "sha256": manifest_sha,
                        "size": manifest.len()
                    },
                    "windows-msix": {
                        "id": 13,
                        "path": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix",
                        "sha256": "c".repeat(64),
                        "size": 56,
                        "githubDigest": format!("sha256:{}", "c".repeat(64))
                    }
                }),
                "publishedAt": "2026-09-16T00:00:00Z"
            }
        })
        .to_string();
        let fetched = |url: &str| {
            if url.ends_with("latest.json") {
                Ok(descriptor.clone())
            } else if url.ends_with("release-manifest.json") {
                Ok(manifest.clone())
            } else {
                Err(AppError::Engine(format!("unexpected URL: {url}")))
            }
        };

        let resolved = resolve_relay_latest(&fetched, base, "x64").unwrap();
        assert_eq!(resolved.identity.release_id, 77);
        assert_eq!(resolved.identity.release_tag, "codex-app-2.0.0");
        assert_eq!(resolved.identity.app_version, "2.0.0");
        assert_eq!(resolved.msix.size, 56);
    }

    #[test]
    fn relay_protocol_2_requires_a_matching_nested_identity() {
        let assets = serde_json::json!({
            "checksums": {
                "id": 11,
                "path": "SHA256SUMS-windows.txt",
                "sha256": "a".repeat(64),
                "size": 12
            },
            "release-manifest": {
                "id": 12,
                "path": "release-manifest.json",
                "sha256": "b".repeat(64),
                "size": 34
            },
            "windows-msix": {
                "id": 13,
                "path": "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix",
                "sha256": "c".repeat(64),
                "size": 56,
                "githubDigest": format!("sha256:{}", "c".repeat(64))
            }
        });
        let mut descriptor: RelayLatest = serde_json::from_value(serde_json::json!({
            "protocol": 2,
            "architecture": "x64",
            "releaseId": 77,
            "repository": "Wangnov/codex-app-mirror",
            "tag": "codex-app-2.0.0",
            "assets": assets.clone(),
            "identity": {
                "releaseId": 77,
                "tag": "codex-app-2.0.0",
                "assets": assets
            }
        }))
        .unwrap();

        assert!(validate_relay_descriptor(&descriptor).is_ok());

        descriptor.identity.as_mut().unwrap().tag = "codex-app-other".into();
        assert!(validate_relay_descriptor(&descriptor).is_err());

        descriptor.identity.as_mut().unwrap().tag = descriptor.tag.clone();
        descriptor
            .identity
            .as_mut()
            .unwrap()
            .assets
            .windows_msix
            .size += 1;
        assert!(validate_relay_descriptor(&descriptor).is_err());

        descriptor.identity = None;
        assert!(validate_relay_descriptor(&descriptor).is_err());
    }

    #[test]
    fn auto_source_falls_back_to_github_without_mixing_identity() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Auto,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "1.0.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Mirror => {
                        panic!("auto source must not resolve the legacy mirror")
                    }
                    CodexPayloadSourceKind::Relay => Ok(fake_source_release(source_kind, "1.0.0")),
                    CodexPayloadSourceKind::Github => Ok(fake_resolved_release()),
                    CodexPayloadSourceKind::Auto => {
                        Err(AppError::Internal("unexpected auto source".to_string()))
                    }
                }
            },
        )
        .unwrap();
        assert_eq!(
            attempts,
            vec![
                CodexPayloadSourceKind::Relay,
                CodexPayloadSourceKind::Github
            ]
        );
        assert_eq!(
            resolved.unwrap().identity.source_kind,
            CodexPayloadSourceKind::Github
        );
    }

    #[test]
    fn auto_source_skips_stale_relay_and_selects_a_newer_later_candidate() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Auto,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "26.908.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Mirror => {
                        panic!("auto source must not resolve the legacy mirror")
                    }
                    CodexPayloadSourceKind::Relay => {
                        Ok(fake_source_release(source_kind, "26.903.0"))
                    }
                    CodexPayloadSourceKind::Github => {
                        Ok(fake_source_release(source_kind, "26.909.0"))
                    }
                    CodexPayloadSourceKind::Auto => {
                        Err(AppError::Internal("unexpected auto source".to_string()))
                    }
                }
            },
        )
        .unwrap()
        .expect("a later newer candidate should be selected");

        assert_eq!(
            attempts,
            vec![
                CodexPayloadSourceKind::Relay,
                CodexPayloadSourceKind::Github
            ]
        );
        assert_eq!(
            resolved.identity.source_kind,
            CodexPayloadSourceKind::Github
        );
        assert_eq!(resolved.app_version, "26.909.0");
    }

    #[test]
    fn auto_source_with_all_stale_candidates_returns_no_update() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Auto,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "26.908.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                let version = match source_kind {
                    CodexPayloadSourceKind::Mirror => {
                        panic!("auto source must not resolve the legacy mirror")
                    }
                    CodexPayloadSourceKind::Github => "26.908.0",
                    CodexPayloadSourceKind::Relay => "26.903.0",
                    CodexPayloadSourceKind::Auto => "0.0.0",
                };
                Ok(fake_source_release(source_kind, version))
            },
        )
        .unwrap();

        assert!(resolved.is_none());
        assert_eq!(
            attempts,
            vec![
                CodexPayloadSourceKind::Relay,
                CodexPayloadSourceKind::Github
            ]
        );
    }

    #[test]
    fn auto_source_keeps_no_update_when_later_fallbacks_fail() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Auto,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "26.908.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Mirror => {
                        panic!("auto source must not resolve the legacy mirror")
                    }
                    CodexPayloadSourceKind::Relay => {
                        Ok(fake_source_release(source_kind, "26.903.0"))
                    }
                    CodexPayloadSourceKind::Github => {
                        Err(AppError::Engine("github unavailable".to_string()))
                    }
                    CodexPayloadSourceKind::Auto => {
                        Err(AppError::Internal("unexpected auto source".to_string()))
                    }
                }
            },
        )
        .unwrap();

        assert!(resolved.is_none());
        assert_eq!(
            attempts,
            vec![
                CodexPayloadSourceKind::Relay,
                CodexPayloadSourceKind::Github
            ]
        );
    }

    #[test]
    fn explicit_relay_stale_does_not_fallback_to_another_source() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Relay,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let result = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "26.908.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Relay => {
                        Ok(fake_source_release(source_kind, "26.903.0"))
                    }
                    _ => panic!("explicit Relay must not resolve another source"),
                }
            },
        );

        let error = result.expect_err("explicit stale source must be reported");
        assert!(
            error.to_string().contains("source_stale:"),
            "actual explicit-source error: {error}"
        );
        assert_eq!(attempts, vec![CodexPayloadSourceKind::Relay]);
    }

    #[test]
    fn explicit_github_does_not_fallback_to_another_source() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Github,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved = resolve_preferred_source_with(
            &settings,
            "x64",
            &network,
            "1.0.0",
            |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Github => Ok(fake_source_release(source_kind, "2.0.0")),
                    _ => panic!("explicit GitHub must not resolve another source"),
                }
            },
        )
        .unwrap()
        .expect("explicit GitHub should resolve its own candidate");

        assert_eq!(attempts, vec![CodexPayloadSourceKind::Github]);
        assert_eq!(
            resolved.identity.source_kind,
            CodexPayloadSourceKind::Github
        );
        assert_eq!(resolved.app_version, "2.0.0");
    }

    #[test]
    fn source_error_classification_and_logs_do_not_expose_tokens() {
        let token = "relay-secret-token";
        let error = AppError::Engine(format!(
            "curl failed for https://relay.example.test/{token}?token={token} exit=7"
        ));
        assert_eq!(source_error_category(&error), "source_unreachable");
        let detail = source_error_detail(CodexPayloadSourceKind::Relay, &error);
        assert!(detail.contains("reason=source_unreachable"));
        assert!(!detail.contains(token));
        assert_eq!(
            redacted_source_locator(&format!("https://relay.example.test/{token}")),
            "<redacted>"
        );

        let invalid = AppError::Engine("response JSON is invalid".to_string());
        assert_eq!(source_error_category(&invalid), "source_invalid");
        let safe_error = source_error_for_caller(CodexPayloadSourceKind::Relay, &error);
        assert_eq!(source_error_category(&safe_error), "source_unreachable");
        assert!(!safe_error.to_string().contains(token));
    }

    #[test]
    fn resolve_latest_rejects_missing_assets_before_asset_fetch() {
        let mut release: serde_json::Value = serde_json::from_str(&release_json(
            "https://example.invalid/codex.msix",
            "https://example.invalid/SHA256SUMS-windows.txt",
            "https://example.invalid/release-manifest.json",
            None,
        ))
        .unwrap();
        release["assets"] = serde_json::json!([]);
        let asset_fetches = std::cell::Cell::new(0);
        let result = resolve_latest(
            &|_| Ok(release.to_string()),
            &|_| {
                asset_fetches.set(asset_fetches.get() + 1);
                Ok(release_manifest_json("x64"))
            },
            "Wangnov/codex-app-mirror",
            "x64",
        );
        assert!(result.is_err());
        assert_eq!(asset_fetches.get(), 0);
    }

    #[test]
    fn resolve_latest_rejects_insecure_urls_before_fetching_manifest() {
        let release = release_json(
            "http://example.invalid/codex.msix",
            "https://example.invalid/SHA256SUMS-windows.txt",
            "https://example.invalid/release-manifest.json",
            None,
        );
        let asset_fetches = std::cell::Cell::new(0);
        let result = resolve_latest(
            &|_| Ok(release.clone()),
            &|_| {
                asset_fetches.set(asset_fetches.get() + 1);
                Ok(release_manifest_json("x64"))
            },
            "Wangnov/codex-app-mirror",
            "x64",
        );
        assert!(result.is_err());
        assert_eq!(asset_fetches.get(), 0);
    }

    #[test]
    fn resolve_latest_rejects_invalid_asset_sizes_before_fetching_manifest() {
        let mut release: serde_json::Value = serde_json::from_str(&release_json(
            "https://example.invalid/codex.msix",
            "https://example.invalid/SHA256SUMS-windows.txt",
            "https://example.invalid/release-manifest.json",
            None,
        ))
        .unwrap();
        release["assets"][0]["size"] = serde_json::json!(0);
        let asset_fetches = std::cell::Cell::new(0);
        let result = resolve_latest(
            &|_| Ok(release.to_string()),
            &|_| {
                asset_fetches.set(asset_fetches.get() + 1);
                Ok(release_manifest_json("x64"))
            },
            "Wangnov/codex-app-mirror",
            "x64",
        );
        assert!(result.is_err());
        assert_eq!(asset_fetches.get(), 0);
    }

    #[test]
    fn apply_version_gate_rejects_equal_or_older_remote_versions() {
        assert!(require_newer_version("1.0.0", "1.0.1").is_ok());
        assert!(require_newer_version("1.0.0", "1.0.0").is_err());
        assert!(require_newer_version("2.0.0", "1.9.9").is_err());
    }

    #[test]
    fn resolve_latest_rejects_malformed_or_mismatched_manifest() {
        for manifest in [
            "not-json".to_string(),
            release_manifest_json("arm64"),
            serde_json::json!({"sources": {"windows": {"architectures": {
                "x64": {"appVersion": "2.0.0"}
            }}}})
            .to_string(),
        ] {
            let release = release_json(
                "https://example.invalid/codex.msix",
                "https://example.invalid/SHA256SUMS-windows.txt",
                "https://example.invalid/release-manifest.json",
                None,
            );
            assert!(resolve_latest(
                &|_| Ok(release.clone()),
                &|_| Ok(manifest.clone()),
                "Wangnov/codex-app-mirror",
                "x64",
            )
            .is_err());
        }
    }

    #[test]
    fn resolve_latest_rejects_malformed_github_digest() {
        let release = release_json(
            "https://example.invalid/codex.msix",
            "https://example.invalid/SHA256SUMS-windows.txt",
            "https://example.invalid/release-manifest.json",
            Some("sha256:not-a-digest"),
        );
        assert!(resolve_latest(
            &|_| Ok(release.clone()),
            &|_| Ok(release_manifest_json("x64")),
            "Wangnov/codex-app-mirror",
            "x64",
        )
        .is_err());
    }

    #[test]
    fn apply_transport_success_covers_download_verification_and_commit() {
        let root = v2_fixture();
        let (local, tx) = prepare_transaction(&root);
        let payload = b"new transport payload".to_vec();
        let payload_sha = sha256_bytes(&payload);
        let mut release = fake_resolved_release();
        release.msix.size = payload.len() as u64;
        let checksums = format!("{payload_sha}  {}.Msix\n", release.identity.package_moniker);
        let mut store_identity = fake_identity();
        store_identity.publisher = "CN=50BDFD77-8903-4850-9FFE-6E8522F64D5B".into();
        let transport = fake_transport(
            Ok(payload.clone()),
            Ok(checksums),
            Ok(()),
            Ok(store_identity),
        );
        let progress_events = std::cell::RefCell::new(Vec::new());

        apply_transaction_with_transport(
            &root,
            &tx,
            &local,
            &release,
            &fake_update_info(&root, &local),
            &|done, total| progress_events.borrow_mut().push((done, total)),
            &transport,
        )
        .unwrap();

        let current = require_capability(&root).unwrap();
        assert_eq!(fs::read(&current.payload_path).unwrap(), payload);
        assert_eq!(current.current_version, "2.0.0");
        assert!(!tx.exists());
        verify_sha256_sums(
            &root,
            &fs::read(root.join(SHA256SUMS_NAME)).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(progress_events
            .into_inner()
            .iter()
            .any(|(done, total)| done == total && *total == payload.len() as u64));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_transport_download_error_rolls_back_without_promoting_staged_file() {
        let root = v2_fixture();
        let (local, tx) = prepare_transaction(&root);
        let release = fake_resolved_release();
        let result = apply_transaction_with_transport(
            &root,
            &tx,
            &local,
            &release,
            &fake_update_info(&root, &local),
            &|_, _| {},
            &fake_transport(
                Err("offline".into()),
                Ok(String::new()),
                Ok(()),
                Ok(fake_identity()),
            ),
        );
        assert!(result.is_err());
        rollback_transaction(&root, &tx).unwrap();
        assert_old_bundle_is_intact(&root);
        assert!(!tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_transport_rejects_checksum_fetch_error_and_invalid_staged_size() {
        let cases = [
            (
                Ok(b"new transport payload".to_vec()),
                Err("checksum offline".to_string()),
            ),
            (Ok(Vec::new()), Ok(String::new())),
        ];
        for (download, checksums) in cases {
            let root = v2_fixture();
            let (local, tx) = prepare_transaction(&root);
            let mut release = fake_resolved_release();
            release.msix.size = 1;
            let result = apply_transaction_with_transport(
                &root,
                &tx,
                &local,
                &release,
                &fake_update_info(&root, &local),
                &|_, _| {},
                &fake_transport(download, checksums, Ok(()), Ok(fake_identity())),
            );
            assert!(result.is_err());
            rollback_transaction(&root, &tx).unwrap();
            assert_old_bundle_is_intact(&root);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn apply_transport_rejects_checksum_and_digest_mismatch_before_commit() {
        let payload = b"new transport payload".to_vec();
        let payload_sha = sha256_bytes(&payload);
        let moniker = "OpenAI.Codex_2.0.0.0_x64__2p2nqsd0c76g0.Msix";
        let cases = [
            ("0".repeat(64) + "  " + moniker + "\n", payload_sha.clone()),
            (payload_sha.clone() + "  " + moniker + "\n", "a".repeat(64)),
        ];
        for (checksums, digest) in cases {
            let root = v2_fixture();
            let (local, tx) = prepare_transaction(&root);
            let mut release = fake_resolved_release();
            release.msix.size = payload.len() as u64;
            release.identity.github_digest = Some(digest);
            let result = apply_transaction_with_transport(
                &root,
                &tx,
                &local,
                &release,
                &fake_update_info(&root, &local),
                &|_, _| {},
                &fake_transport(
                    Ok(payload.clone()),
                    Ok(checksums),
                    Ok(()),
                    Ok(fake_identity()),
                ),
            );
            assert!(result.is_err());
            rollback_transaction(&root, &tx).unwrap();
            assert_old_bundle_is_intact(&root);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn apply_transport_rejects_authenticode_and_identity_errors_before_commit() {
        let mut wrong_identity = fake_identity();
        wrong_identity.version = "9.0.0.0".into();
        for (authenticode, identity) in [
            (Err("not signed".into()), Ok(fake_identity())),
            (Ok(()), Err("malformed manifest".into())),
            (Ok(()), Ok(wrong_identity)),
        ] {
            let root = v2_fixture();
            let (local, tx) = prepare_transaction(&root);
            let payload = b"new transport payload".to_vec();
            let payload_sha = sha256_bytes(&payload);
            let mut release = fake_resolved_release();
            release.msix.size = payload.len() as u64;
            release.identity.github_digest = Some(payload_sha.clone());
            let checksums = format!("{payload_sha}  {}.Msix\n", release.identity.package_moniker);
            let result = apply_transaction_with_transport(
                &root,
                &tx,
                &local,
                &release,
                &fake_update_info(&root, &local),
                &|_, _| {},
                &fake_transport(Ok(payload), Ok(checksums), authenticode, identity),
            );
            assert!(result.is_err());
            rollback_transaction(&root, &tx).unwrap();
            assert_old_bundle_is_intact(&root);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn staged_commit_updates_payload_metadata_and_complete_checksum_set() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        let tx = root.join(STAGING_DIR).join("codex-update-commit");
        fs::create_dir_all(tx.join("old")).unwrap();
        fs::create_dir_all(tx.join("next")).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        let staged = tx.join("codex-new.Msix");
        let payload = b"new committed payload";
        fs::write(&staged, payload).unwrap();
        let release = fake_resolved_release();
        let expected = fake_update_info(&root, &local);
        let payload_sha = sha256_bytes(payload);
        let progress_events = std::cell::RefCell::new(Vec::new());
        let mut deferred_cleanup = None;

        commit_staged_payload(StagedCommit {
            root: &root,
            tx: &tx,
            local: &local,
            fresh: &release,
            expected: &expected,
            staged: &staged,
            actual_sha: &payload_sha,
            total: payload.len() as u64,
            progress: &|done, total| progress_events.borrow_mut().push((done, total)),
            reactivation_reinstall: false,
            deferred_cleanup: &mut deferred_cleanup,
        })
        .unwrap();

        let current = require_capability(&root).unwrap();
        assert_eq!(fs::read(&current.payload_path).unwrap(), payload);
        assert_eq!(current.current_version, "2.0.0");
        assert_eq!(
            fs::read(root.join("payload/_backup/OpenAI.Codex_x64_1.0.0.0.msix")).unwrap(),
            b"old payload"
        );
        let sums = fs::read(root.join(SHA256SUMS_NAME)).unwrap();
        verify_sha256_sums(&root, &sums, &BTreeMap::new()).unwrap();
        assert!(!tx.exists());
        assert_eq!(
            progress_events.into_inner(),
            vec![(payload.len() as u64, payload.len() as u64)]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_commit_failure_after_payload_swap_rolls_back_cleanly() {
        let root = v2_fixture();
        let local = require_capability(&root).unwrap();
        let tx = root.join(STAGING_DIR).join("codex-update-fault");
        fs::create_dir_all(tx.join("old")).unwrap();
        fs::create_dir_all(tx.join("next").join(USB_MANIFEST_NAME)).unwrap();
        snapshot_metadata(&root, &tx).unwrap();
        let staged = tx.join("codex-new.Msix");
        fs::write(&staged, b"new but rolled back payload").unwrap();
        let release = fake_resolved_release();
        let expected = fake_update_info(&root, &local);
        let payload_sha = sha256_file(&staged).unwrap();
        let mut deferred_cleanup = None;

        let result = commit_staged_payload(StagedCommit {
            root: &root,
            tx: &tx,
            local: &local,
            fresh: &release,
            expected: &expected,
            staged: &staged,
            actual_sha: &payload_sha,
            total: 1,
            progress: &|_, _| {},
            reactivation_reinstall: false,
            deferred_cleanup: &mut deferred_cleanup,
        });
        assert!(result.is_err());
        rollback_transaction(&root, &tx).unwrap();

        let current = require_capability(&root).unwrap();
        assert_eq!(current.current_version, "1.0.0");
        assert_eq!(fs::read(&current.payload_path).unwrap(), b"old payload");
        verify_sha256_sums(
            &root,
            &fs::read(root.join(SHA256SUMS_NAME)).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(!tx.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn checksum_generation_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = v2_fixture();
        let outside = root.join("outside.txt");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, root.join("escaped.txt")).unwrap();
        assert!(build_sha256_sums(&root, &BTreeMap::new()).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
