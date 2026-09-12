//! Codex payload update channel.
//!
//! This module updates the authenticated USB bundle only. The deployed
//! Windows AppX is deliberately handled by a separate explicit action.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
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
    compare_versions, read_usb_codex_desktop_version, ChannelKind, UpdateContext, UpdateInfo,
};
use crate::errors::AppError;

pub const DEFAULT_CODEX_PAYLOAD_REPO: &str = "Wangnov/codex-app-mirror";

const STAGING_DIR: &str = ".staging";
const TRANSACTION_PREFIX: &str = "codex-update-";
const SHA256SUMS_NAME: &str = "SHA256SUMS.txt";
const ROOT_MANIFEST_NAME: &str = "manifest.json";
const USB_MANIFEST_NAME: &str = "usb-manifest.json";
const USB_LICENSE_NAME: &str = "usb-license.json";
const TRANSACTION_VERSION: u32 = 1;
const SOURCE_METADATA_ATTEMPTS: usize = 2;
const SOURCE_METADATA_RETRY_DELAY_MS: u64 = 450;
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
}

#[derive(Debug, Clone, Deserialize)]
struct RelayAssets {
    checksums: RelayAsset,
    #[serde(rename = "release-manifest")]
    release_manifest: RelayAsset,
    #[serde(rename = "windows-msix")]
    windows_msix: RelayAsset,
}

#[derive(Debug, Clone, Deserialize)]
struct RelayAsset {
    path: String,
    sha256: String,
    size: u64,
    #[serde(rename = "githubDigest", default)]
    github_digest: Option<String>,
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
    let manifest_sha = sha256_bytes(manifest_text.as_bytes());
    let checksums_sha = sha256_bytes(checksums_text.as_bytes());
    // The mirror manifest has no GitHub release id. A digest-derived stable
    // id still gives check/apply a strong stale-identity comparison without
    // inventing a mutable numeric release identifier.
    let release_id = u64::from_str_radix(&manifest_sha[..16], 16).unwrap_or(0);
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
            checksums_sha256: Some(checksums_sha.clone()),
            manifest_sha256: Some(manifest_sha),
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
    if descriptor.protocol != 1 {
        return Err(AppError::Engine(format!(
            "Codex relay 协议版本不受支持：{}",
            descriptor.protocol
        )));
    }
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
    let locator = source_locator_for(settings, source_kind);
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
                    "codex payload source failed stage=metadata source={} attempt={}/{} retryable={} locator={} error={}",
                    source_kind.as_str(),
                    attempt,
                    SOURCE_METADATA_ATTEMPTS,
                    retryable,
                    locator,
                    error
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
    Err(last_error.unwrap_or_else(|| {
        AppError::Internal(format!("Codex 更新源未执行：{}", source_kind.as_str()))
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
) -> Result<ResolvedRelease, AppError> {
    resolve_preferred_source_with(
        settings,
        architecture,
        network,
        |settings, architecture, network, source_kind| {
            resolve_source(settings, architecture, network, source_kind)
        },
    )
}

fn resolve_preferred_source_with<F>(
    settings: &AppSettings,
    architecture: &str,
    network: &NetworkConfig,
    mut resolve: F,
) -> Result<ResolvedRelease, AppError>
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
            let mirror_result = resolve_source_with_retry(
                settings,
                architecture,
                network,
                CodexPayloadSourceKind::Mirror,
                &mut resolve,
            );
            match mirror_result {
                Ok(release) => Ok(release),
                Err(mirror_error) => {
                    log::warn!("Codex 国内镜像不可用，准备回退：{mirror_error}");
                    let mut fallback_errors = vec![format!("镜像：{mirror_error}")];
                    let relay_result = resolve_source_with_retry(
                        settings,
                        architecture,
                        network,
                        CodexPayloadSourceKind::Relay,
                        &mut resolve,
                    );
                    match relay_result {
                        Ok(release) => return Ok(release),
                        Err(relay_error) => {
                            log::warn!("Codex 兼容 Relay 不可用，回退 GitHub：{relay_error}");
                            fallback_errors.push(format!("兼容 Relay：{relay_error}"));
                        }
                    }
                    let github_result = resolve_source_with_retry(
                        settings,
                        architecture,
                        network,
                        CodexPayloadSourceKind::Github,
                        &mut resolve,
                    );
                    match github_result {
                        Ok(release) => Ok(release),
                        Err(github_error) => Err(AppError::Engine(format!(
                            "Codex 更新源均不可用；{}；GitHub：{github_error}",
                            fallback_errors.join("；")
                        ))),
                    }
                }
            }
        }
        source_kind => {
            resolve_source_with_retry(settings, architecture, network, source_kind, &mut resolve)
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
    reject_reparse(bundle_root)?;
    for name in [ROOT_MANIFEST_NAME, USB_MANIFEST_NAME, SHA256SUMS_NAME] {
        reject_reparse_chain(&bundle_root.join(name))?;
    }
    let manifest = read_json(&bundle_root.join(ROOT_MANIFEST_NAME), ROOT_MANIFEST_NAME)?;
    let usb_manifest = read_json(&bundle_root.join(USB_MANIFEST_NAME), USB_MANIFEST_NAME)?;
    if exact_u64(manifest.get("schemaVersion"), "root manifest.schemaVersion")? != 2
        || exact_u64(
            usb_manifest.get("schemaVersion"),
            "usb-manifest.schemaVersion",
        )? != 2
    {
        return Err(AppError::Engine(
            "此 U 盘包不是支持 Codex 更新的 v2 包，请重建并重新授权".to_string(),
        ));
    }
    let root_capability = manifest
        .get("updateCapability")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::Engine("root manifest 缺少 updateCapability".to_string()))?;
    if root_capability.get("codexPayload") != Some(&serde_json::Value::Bool(true))
        || exact_u64(
            root_capability.get("schemaVersion"),
            "root updateCapability.schemaVersion",
        )? != 2
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
        )? != 2
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
    if !regular_file_exists(&payload_path)? {
        return Err(AppError::Engine(
            "Codex payload 不存在或不是普通文件".to_string(),
        ));
    }
    let actual_payload_size = fs::metadata(&payload_path)
        .map_err(|error| AppError::Engine(format!("读取 payload 大小失败: {error}")))?
        .len();
    if actual_payload_size != manifest_payload_size {
        return Err(AppError::Engine(
            "当前 payload 大小与 manifest 不一致".to_string(),
        ));
    }
    let actual_payload_sha = sha256_file(&payload_path)
        .map_err(|error| AppError::Engine(format!("计算当前 payload SHA256 失败: {error}")))?;
    if !actual_payload_sha.eq_ignore_ascii_case(&manifest_payload_sha) {
        return Err(AppError::Engine(
            "当前 payload SHA256 与 manifest 不一致".to_string(),
        ));
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

fn parse_codex_update_payload(value: &serde_json::Value) -> Result<CodexUpdatePayload, AppError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        AppError::StaleExpectation(format!("更新确认对象无效，请重新检查：{error}"))
    })
}

fn verify_bundle_integrity(bundle_root: &Path) -> Result<(), AppError> {
    let sums_path = bundle_root.join(SHA256SUMS_NAME);
    reject_reparse_chain(&sums_path)?;
    let sums = fs::read(&sums_path)
        .map_err(|error| AppError::Engine(format!("读取 {SHA256SUMS_NAME} 失败: {error}")))?;
    verify_sha256_sums(bundle_root, &sums, &BTreeMap::new())
}

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

fn pending_transaction_dirs(bundle_root: &Path) -> Result<Vec<PathBuf>, AppError> {
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
        } else {
            return Err(AppError::Engine(
                "事务目录包含未知文件，拒绝继续".to_string(),
            ));
        }
    }
    Ok(result)
}

fn write_transaction_state(tx: &Path, phase: &str) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(&TransactionState {
        version: TRANSACTION_VERSION,
        phase: phase.to_string(),
    })
    .map_err(|error| AppError::Engine(format!("序列化事务状态失败: {error}")))?;
    reject_reparse_chain(tx)?;
    crate::app::atomic_file::write_atomic(&tx.join("state.json"), &bytes)
        .map_err(|error| AppError::Engine(format!("原子写入事务状态失败: {error}")))
}

fn read_transaction_state(tx: &Path) -> Result<TransactionState, AppError> {
    let state_path = tx.join("state.json");
    reject_reparse_chain(&state_path)?;
    let backup_path = crate::app::atomic_file::backup_path(&state_path);
    if backup_path.exists() {
        reject_reparse_chain(&backup_path)?;
    }
    let (state, outcome) =
        crate::app::atomic_file::read_with_recovery::<TransactionState>(&state_path);
    let state = state.ok_or_else(|| {
        AppError::Engine(format!(
            "事务状态缺失或损坏，保留事务供人工恢复（状态={outcome:?}）"
        ))
    })?;
    if state.version != TRANSACTION_VERSION {
        return Err(AppError::Engine("事务状态版本不受支持".to_string()));
    }
    Ok(state)
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

fn rollback_transaction(bundle_root: &Path, tx: &Path) -> Result<(), AppError> {
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
    // Restore only the exact backup name generated by this transaction. A
    // pre-existing backup must never be mistaken for the payload being
    // rolled back.
    if regular_file_exists(&expected_backup)? {
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
        if state.phase == "committed" {
            if verify_committed_transaction(bundle_root, &tx).is_ok() {
                fs::remove_dir_all(&tx).map_err(|error| AppError::Engine(error.to_string()))?;
            } else {
                rollback_transaction(bundle_root, &tx)?;
            }
        } else {
            rollback_transaction(bundle_root, &tx)?;
        }
    }
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
        let file_type = entry
            .file_type()
            .map_err(|error| AppError::Engine(error.to_string()))?;
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
        if relative == SHA256SUMS_NAME || listed.contains_key(&relative) {
            return Err(AppError::Engine(
                "SHA256SUMS 包含重复或非法条目".to_string(),
            ));
        }
        listed.insert(relative, hash.to_ascii_lowercase());
    }
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
    Ok(())
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

fn same_identity(expected: &CodexPayloadIdentity, fresh: &CodexPayloadIdentity) -> bool {
    expected == fresh
}

fn require_newer_version(current: &str, latest: &str) -> Result<(), AppError> {
    if compare_versions(current, latest) == std::cmp::Ordering::Less {
        Ok(())
    } else {
        Err(AppError::StaleExpectation(
            "远端版本不高于当前版本，拒绝降级更新。".to_string(),
        ))
    }
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
    require_windows()?;
    // A previous apply may have been interrupted after a rename. Recover the
    // durable transaction before observing the version; refusing to check
    // would strand the user in a state that the next check cannot repair.
    recover_pending_transactions(&ctx.bundle_root)?;
    let local = require_capability(&ctx.bundle_root)?;
    verify_bundle_integrity(&ctx.bundle_root)?;
    log::info!(
        "codex payload update check start architecture={} current={} source={} proxy={}",
        local.architecture,
        local.current_version,
        ctx.settings.codex_payload_source_kind.as_str(),
        ctx.settings.proxy_mode.as_str()
    );
    let network = network_from_settings(&ctx.settings);
    let fresh = match resolve_preferred_source(&ctx.settings, &local.architecture, &network) {
        Ok(fresh) => fresh,
        Err(error) => {
            log::error!(
                "codex payload update check failed stage=metadata architecture={} current={} error={}",
                local.architecture,
                local.current_version,
                error
            );
            return Err(error);
        }
    };
    if compare_versions(&local.current_version, &fresh.app_version) != std::cmp::Ordering::Less {
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
    let payload = codex_update_payload(&ctx.bundle_root, &local, &fresh.identity)?;
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
    // Security boundary: this must remain the first executable statement.
    crate::app::usb_guard::require_signed_codex_bundle("codex-payload-apply")?;
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
    let (observed_root, _) = crate::app::usb_guard::authorized_signed_bundle_identity()?;
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
    let (post_recovery_root, _) = crate::app::usb_guard::authorized_signed_bundle_identity()?;
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
    let local = require_capability(&ctx.bundle_root)?;
    verify_bundle_integrity(&ctx.bundle_root)?;
    let expected_payload = parse_codex_update_payload(&expected.payload)?;
    let expected_identity = expected_payload.identity;
    let expected_snapshot = expected_payload.local_snapshot.ok_or_else(|| {
        AppError::StaleExpectation("更新确认缺少本地 USB 快照，请重新检查后再确认。".to_string())
    })?;
    require_local_snapshot(&ctx.bundle_root, &local, &expected_snapshot)?;
    if !source_matches_settings(&ctx.settings, &expected_identity)?
        || expected.current_version != local.current_version
    {
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
    let fresh = resolve_source_with_retry(
        &ctx.settings,
        &local.architecture,
        &network,
        expected_identity.source_kind,
        &mut resolver,
    )?;
    if !same_identity(&expected_identity, &fresh.identity)
        || fresh.app_version != expected.latest_version
    {
        return Err(AppError::StaleExpectation(
            "远端 Codex 版本或资产身份已变化，请重新检查后再确认。".to_string(),
        ));
    }
    require_newer_version(&local.current_version, &fresh.app_version)?;
    require_https_url(&fresh.msix.url, "MSIX 下载地址")?;
    require_https_url(&fresh.checksums.url, "SHA256SUMS 下载地址")?;
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
    let tx = staging.join(format!("{TRANSACTION_PREFIX}{}", uuid::Uuid::new_v4()));
    fs::create_dir(&tx).map_err(|error| AppError::Engine(error.to_string()))?;
    reject_reparse_chain(&tx)?;
    fs::create_dir(tx.join("old")).map_err(|error| AppError::Engine(error.to_string()))?;
    fs::create_dir(tx.join("next")).map_err(|error| AppError::Engine(error.to_string()))?;
    if let Err(error) = snapshot_metadata(&ctx.bundle_root, &tx)
        .and_then(|()| write_transaction_state(&tx, "prepared"))
    {
        fs::remove_dir_all(&tx).ok();
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
    );
    if let Err(error) = result {
        log::error!(
            "codex payload update failed stage=download-verify-commit current={} expected_latest={} error={}",
            expected.current_version,
            expected.latest_version,
            error
        );
        if matches!(error, AppError::Committed(_)) {
            return Err(error);
        }
        let rollback = rollback_transaction(&ctx.bundle_root, &tx);
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

fn apply_transaction(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    network: &NetworkConfig,
) -> Result<(), AppError> {
    let allow_redirects = fresh.identity.source_kind == CodexPayloadSourceKind::Github
        || (fresh.identity.source_kind == CodexPayloadSourceKind::Mirror
            && is_builtin_codex_mirror_base(&fresh.identity.source_locator));
    let transport = production_apply_transport(network, allow_redirects);
    apply_transaction_with_transport(root, tx, local, fresh, expected, progress, &transport)
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

fn apply_transaction_with_transport(
    root: &Path,
    tx: &Path,
    local: &LocalBundle,
    fresh: &ResolvedRelease,
    expected: &UpdateInfo,
    progress: &dyn Fn(u64, u64),
    transport: &ApplyTransport,
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
    if !identity.publisher.to_ascii_lowercase().contains("openai") {
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
    })
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
    } = input;
    reject_reparse_chain(staged)?;
    let root_manifest_path = root.join(ROOT_MANIFEST_NAME);
    let usb_manifest_path = root.join(USB_MANIFEST_NAME);
    let sums_path = root.join(SHA256SUMS_NAME);
    let latest_local = require_capability(root)?;
    verify_bundle_integrity(root)?;
    if let Some(expected_snapshot) = parse_codex_update_payload(&expected.payload)?.local_snapshot {
        require_local_snapshot(root, &latest_local, &expected_snapshot)?;
    }
    if latest_local.current_version != expected.current_version
        || latest_local.payload_relative != local.payload_relative
        || latest_local.architecture != local.architecture
        || !latest_local.payload_path.exists()
    {
        return Err(AppError::StaleExpectation(
            "本地 USB payload 在更新期间发生变化，请重新检查后再确认。".to_string(),
        ));
    }
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
    reject_reparse_chain(tx)?;
    // The authorization snapshot must be re-observed before any transaction
    // directory mutation or backup move.  The later check immediately before
    // replacing the live payload remains necessary because the preparation
    // work itself can take long enough for the removable volume to change.
    if let Some(authorization) =
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-precommit")?
    {
        let expected_root = fs::canonicalize(root)
            .map_err(|error| AppError::Engine(format!("解析 Codex bundle 根目录失败：{error}")))?;
        let authorized_root = fs::canonicalize(authorization.bundle_root).map_err(|error| {
            AppError::Engine(format!("解析授权 USB bundle 根目录失败：{error}"))
        })?;
        if expected_root != authorized_root {
            return Err(AppError::StaleExpectation(
                "授权 U 盘在 payload 提交前发生变化，请重新检查后再试。".to_string(),
            ));
        }
    }
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
    if let Some(authorization) =
        crate::app::usb_guard::require_signed_codex_bundle("codex-payload-precommit")?
    {
        let expected_root = fs::canonicalize(root)
            .map_err(|error| AppError::Engine(format!("解析 Codex bundle 根目录失败：{error}")))?;
        let authorized_root = fs::canonicalize(authorization.bundle_root).map_err(|error| {
            AppError::Engine(format!("解析授权 USB bundle 根目录失败：{error}"))
        })?;
        if expected_root != authorized_root {
            return Err(AppError::StaleExpectation(
                "授权 U 盘在 payload 提交前发生变化，请重新检查后再试。".to_string(),
            ));
        }
    }
    fs::rename(&latest_local.payload_path, &backup_path)
        .map_err(|error| AppError::Engine(format!("备份旧 MSIX 失败: {error}")))?;
    write_transaction_state(tx, "payload-backup")?;
    fs::rename(staged, &latest_local.payload_path)
        .map_err(|error| AppError::Engine(format!("替换 MSIX 失败: {error}")))?;
    write_transaction_state(tx, "payload-swapped")?;

    let payload_size = fs::metadata(&latest_local.payload_path)
        .map_err(|error| AppError::Engine(error.to_string()))?
        .len();
    let (next_root, next_usb) = update_metadata(
        &latest_local,
        &fresh.identity,
        &fresh.app_version,
        actual_sha,
        payload_size,
    )?;
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
    write_transaction_state(tx, "metadata-ready")?;
    replace_file(
        &tx.join("next").join(ROOT_MANIFEST_NAME),
        &root_manifest_path,
    )?;
    write_transaction_state(tx, "manifest-committed")?;
    replace_file(&tx.join("next").join(USB_MANIFEST_NAME), &usb_manifest_path)?;
    write_transaction_state(tx, "usb-manifest-committed")?;
    replace_file(&tx.join("next").join(SHA256SUMS_NAME), &sums_path)?;
    write_transaction_state(tx, "committed")?;
    verify_sha256_sums(root, &next_sums, &BTreeMap::new())?;
    verify_metadata_values(root, actual_sha, payload_size)?;
    fs::remove_dir_all(tx).map_err(|error| {
        AppError::Committed(format!("无法清理已提交事务目录 {}：{error}", tx.display()))
    })?;
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
            "updateCapability": {"codexPayload": true, "schemaVersion": 2},
            "payload": {"path": "payload/OpenAI.Codex_x64.msix", "sha256": payload_hash, "size": payload.len()}
        });
        let usb = serde_json::json!({
            "schemaVersion": 2,
            "updateCapability": {
                "codexPayload": true, "schemaVersion": 2,
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
    fn auto_source_falls_back_to_github_without_mixing_identity() {
        let settings = AppSettings {
            codex_payload_source_kind: CodexPayloadSourceKind::Auto,
            ..AppSettings::default()
        };
        let network = NetworkConfig::direct();
        let mut attempts = Vec::new();
        let resolved =
            resolve_preferred_source_with(&settings, "x64", &network, |_, _, _, source_kind| {
                attempts.push(source_kind);
                match source_kind {
                    CodexPayloadSourceKind::Mirror => {
                        Err(AppError::Engine("mirror unavailable".to_string()))
                    }
                    CodexPayloadSourceKind::Relay => {
                        Err(AppError::Engine("relay unavailable".to_string()))
                    }
                    CodexPayloadSourceKind::Github => Ok(fake_resolved_release()),
                    CodexPayloadSourceKind::Auto => {
                        Err(AppError::Internal("unexpected auto source".to_string()))
                    }
                }
            })
            .unwrap();
        assert_eq!(
            attempts,
            vec![
                CodexPayloadSourceKind::Mirror,
                CodexPayloadSourceKind::Relay,
                CodexPayloadSourceKind::Github
            ]
        );
        assert_eq!(
            resolved.identity.source_kind,
            CodexPayloadSourceKind::Github
        );
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
        let transport = fake_transport(
            Ok(payload.clone()),
            Ok(checksums),
            Ok(()),
            Ok(fake_identity()),
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
