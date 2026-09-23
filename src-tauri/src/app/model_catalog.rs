//! Dynamic native Codex model catalog bridge.
//!
//! Codex exposes an official `model_catalog_json` configuration field.  The
//! bridge writes only the public model metadata required by that catalog; the
//! 404token device key is used for the request and is never written to the
//! catalog, config, logs, or the USB bundle.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::app::atomic_file;
use crate::app::paths;
use crate::app::usb_guard;
use crate::errors::AppError;

const GATEWAY_MODELS_URL: &str = "https://404token.xyz/v1/models";
const CATALOG_FILE_NAME: &str = "codex-usb-native-model-catalog.json";
const RELOAD_MARKER_FILE_NAME: &str = "codex-usb-native-model-catalog.reload";
const CATALOG_IDENTITY: &str = "404token-unified-api-internal";
const CATALOG_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const GATEWAY_MAX_ATTEMPTS: u32 = 3;
const GATEWAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const GATEWAY_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONFIG_BEGIN: &str = "# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1";
const CONFIG_END: &str = "# END CODEX-USB-NATIVE-MODEL-CATALOG v1";
const GENERIC_BASE_INSTRUCTIONS: &str = "You are Codex, a coding assistant.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeModelCatalogReport {
    pub status: String,
    pub model_count: usize,
    pub catalog_path: Option<String>,
    pub fetched_at: Option<String>,
    pub message: String,
    /// Number of requests made to the upstream gateway for this refresh.
    /// This is exposed so the UI can distinguish a skipped refresh from a
    /// refresh which exhausted its bounded retry budget.
    #[serde(default)]
    pub attempts: u32,
    /// Stable error category for callers that need to distinguish an upstream
    /// outage/authentication problem from a local file/configuration failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayModel {
    raw_id: String,
    owned_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedModel {
    raw_id: String,
    canonical_id: String,
    owned_by: Option<String>,
    alias_kind: AliasKind,
    date_key: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasKind {
    Exact,
    Stable,
    Latest,
    Date,
}

impl AliasKind {
    fn rank(self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::Stable => 1,
            Self::Latest => 2,
            Self::Date => 3,
        }
    }
}

fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(message.into())
}

fn codex_home_dir() -> Option<PathBuf> {
    // The portable launcher passes the exact same profile path to Codex via
    // `CODEX_HOME`.  Resolve the USB profile first so a machine with a stale
    // process-level CODEX_HOME (or a different `directories` home resolver)
    // cannot receive a false "refreshed" result written to a directory that
    // the launched Codex never reads.
    if usb_guard::guard_required() {
        if let Ok(paths) = usb_guard::local_data_paths() {
            return Some(paths.codex_home);
        }
    }
    if let Some(value) = env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        return path.is_absolute().then_some(path);
    }
    paths::codex_home_dir()
}

fn catalog_path() -> Result<PathBuf, AppError> {
    codex_home_dir()
        .map(|home| home.join(CATALOG_FILE_NAME))
        .ok_or_else(|| engine("无法确定 Codex 用户目录，模型目录未刷新"))
}

fn config_path() -> Result<PathBuf, AppError> {
    codex_home_dir()
        .map(|home| home.join("config.toml"))
        .ok_or_else(|| engine("无法确定 Codex 配置目录，模型目录未刷新"))
}

fn reload_marker_path() -> Result<PathBuf, AppError> {
    codex_home_dir()
        .map(|home| home.join(RELOAD_MARKER_FILE_NAME))
        .ok_or_else(|| engine("无法确定 Codex 用户目录，模型目录重载标记未写入"))
}

/// Whether a successful catalog write happened after the currently running
/// Codex process loaded its model list.  The next manager launch consumes this
/// marker and restarts only the managed portable Codex tree when necessary.
pub fn restart_required() -> bool {
    reload_marker_path()
        .ok()
        .is_some_and(|path| matches!(fs::metadata(path), Ok(metadata) if metadata.is_file()))
}

pub fn mark_restart_required() -> Result<(), AppError> {
    let path = reload_marker_path()?;
    atomic_file::write_atomic(&path, b"catalog-updated\n")
        .map_err(|error| engine(format!("写入模型目录重载标记失败：{error}")))
}

/// Best-effort terminal cleanup.  The catalog itself remains the source of
/// truth; a stale marker must never turn a successful launch into an error.
pub fn clear_restart_required() {
    let Ok(path) = reload_marker_path() else {
        return;
    };
    if let Err(error) = fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!(
                "native model catalog reload marker retained path={} error={error}",
                path.display()
            );
        }
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn catalog_is_fresh(path: &Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    modified
        .elapsed()
        .map(|age| age <= CATALOG_TTL)
        .unwrap_or(false)
}

fn existing_catalog_count(path: &Path) -> usize {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            (value.get("identity").and_then(Value::as_str) == Some(CATALOG_IDENTITY))
                .then(|| value.get("models").and_then(Value::as_array).cloned())
                .flatten()
        })
        .map(|models| models.len())
        .unwrap_or(0)
}

fn report(
    status: &str,
    count: usize,
    path: Option<&Path>,
    fetched_at: Option<String>,
    message: impl Into<String>,
) -> NativeModelCatalogReport {
    report_with_details(status, count, path, fetched_at, message, 0, None)
}

fn report_with_details(
    status: &str,
    count: usize,
    path: Option<&Path>,
    fetched_at: Option<String>,
    message: impl Into<String>,
    attempts: u32,
    error_kind: Option<&str>,
) -> NativeModelCatalogReport {
    NativeModelCatalogReport {
        status: status.to_string(),
        model_count: count,
        catalog_path: path.map(|value| value.to_string_lossy().into_owned()),
        fetched_at,
        message: message.into(),
        attempts,
        error_kind: error_kind.map(ToOwned::to_owned),
    }
}

fn parse_gateway_models(payload: &Value) -> Vec<GatewayModel> {
    let Some(items) = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = object
                .get("id")
                .or_else(|| object.get("slug"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| {
                    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
                })?
                .to_string();
            let owned_by = object
                .get("owned_by")
                .or_else(|| object.get("ownedBy"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|owner| {
                    !owner.is_empty() && owner.len() <= 128 && !owner.chars().any(char::is_control)
                })
                .map(ToOwned::to_owned);
            Some(GatewayModel {
                raw_id: id,
                owned_by,
            })
        })
        .collect()
}

fn parse_date_suffix(value: &str) -> Option<u32> {
    let digits_owned;
    let digits = if value.len() == 8 {
        value
    } else if value.len() == 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
    {
        digits_owned = value.replace('-', "");
        digits_owned.as_str()
    } else {
        return None;
    };
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = digits[0..4].parse::<u32>().ok()?;
    let month = digits[4..6].parse::<u32>().ok()?;
    let day = digits[6..8].parse::<u32>().ok()?;
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (day <= max_day).then_some(year * 10_000 + month * 100 + day)
}

fn split_alias(raw_id: &str) -> (String, AliasKind, u32) {
    for (suffix, kind) in [
        ("-stable", AliasKind::Stable),
        ("-latest", AliasKind::Latest),
    ] {
        if raw_id.len() > suffix.len() && raw_id.to_ascii_lowercase().ends_with(suffix) {
            return (raw_id[..raw_id.len() - suffix.len()].to_string(), kind, 0);
        }
    }
    if raw_id.len() > 11 {
        let date_start = raw_id.len() - 10;
        if raw_id.as_bytes().get(date_start - 1) == Some(&b'-') {
            let suffix = &raw_id[date_start..];
            if let Some(date_key) = parse_date_suffix(suffix) {
                return (
                    raw_id[..date_start - 1].to_string(),
                    AliasKind::Date,
                    date_key,
                );
            }
        }
    }
    if let Some((base, suffix)) = raw_id.rsplit_once('-') {
        if let Some(date_key) = parse_date_suffix(suffix) {
            return (base.to_string(), AliasKind::Date, date_key);
        }
    }
    (raw_id.to_string(), AliasKind::Exact, 0)
}

fn compare_model_choice(left: &NormalizedModel, right: &NormalizedModel) -> Ordering {
    left.alias_kind
        .rank()
        .cmp(&right.alias_kind.rank())
        .then_with(|| {
            if left.alias_kind == AliasKind::Date && right.alias_kind == AliasKind::Date {
                right.date_key.cmp(&left.date_key)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            left.raw_id
                .to_ascii_lowercase()
                .cmp(&right.raw_id.to_ascii_lowercase())
        })
}

fn normalize_models(models: Vec<GatewayModel>) -> Vec<NormalizedModel> {
    let mut groups: BTreeMap<String, Vec<NormalizedModel>> = BTreeMap::new();
    for model in models {
        let (canonical_id, alias_kind, date_key) = split_alias(&model.raw_id);
        let owner_key = model
            .owned_by
            .as_deref()
            .unwrap_or("other")
            .trim()
            .to_ascii_lowercase();
        let key = format!("{owner_key}\u{1f}{}", canonical_id.to_ascii_lowercase());
        groups.entry(key).or_default().push(NormalizedModel {
            raw_id: model.raw_id,
            canonical_id,
            owned_by: model.owned_by,
            alias_kind,
            date_key,
        });
    }

    groups
        .into_values()
        .filter_map(|mut group| {
            group.sort_by(compare_model_choice);
            group.into_iter().next()
        })
        .collect()
}

fn reasoning_levels() -> Value {
    json!([
        { "effort": "low", "description": "Fast responses with lighter reasoning" },
        { "effort": "medium", "description": "Balanced gateway reasoning" },
        { "effort": "high", "description": "Greater reasoning depth for complex tasks" },
        { "effort": "xhigh", "description": "Extra high reasoning depth for difficult tasks" },
        { "effort": "max", "description": "Maximum reasoning depth" }
    ])
}

fn native_model_entry(model: &NormalizedModel, priority: usize) -> Value {
    let display_name = match model.owned_by.as_deref() {
        Some(owner) => format!("{owner} / {}", model.canonical_id),
        None => model.canonical_id.clone(),
    };
    json!({
        "slug": model.raw_id,
        "display_name": display_name,
        "description": "Model supplied by the configured unified gateway.",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": reasoning_levels(),
        "shell_type": "unified_exec",
        "visibility": "list",
        "supported_in_api": true,
        "priority": priority,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "availability_nux": null,
        "upgrade": null,
        "include_skills_usage_instructions": false,
        "include_plugin_usage_instructions": false,
        "include_apps_usage_instructions": false,
        "default_reasoning_summary": "none",
        "support_verbosity": true,
        "default_verbosity": "low",
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": "text_and_image",
        "truncation_policy": { "mode": "tokens", "limit": 10000 },
        "supports_image_detail_original": false,
        "context_window": 128000,
        "max_context_window": 128000,
        "comp_hash": "gateway",
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text"],
        "supports_search_tool": false,
        "use_responses_lite": true,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": false,
        "tool_mode": "code_mode_only",
        "multi_agent_version": "v1",
        "base_instructions": GENERIC_BASE_INSTRUCTIONS
    })
}

fn native_catalog(models: &[NormalizedModel]) -> Value {
    json!({
        "schema_version": 1,
        "fetched_at": format!("unix:{}", unix_seconds()),
        "client_version": "codex-usb-gateway-1",
        "identity": CATALOG_IDENTITY,
        "models": models.iter().enumerate().map(|(index, model)| native_model_entry(model, index + 1)).collect::<Vec<_>>()
    })
}

fn toml_string(value: &Path) -> String {
    value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn has_managed_config(text: &str) -> bool {
    text.lines().any(|line| line.trim() == CONFIG_BEGIN)
}

fn managed_config_points_to_catalog(text: &str, path: &Path) -> bool {
    let expected = format!("model_catalog_json = \"{}\"", toml_string(path));
    let mut managed = false;
    let mut found = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == CONFIG_BEGIN {
            if managed {
                return false;
            }
            managed = true;
            continue;
        }
        if trimmed == CONFIG_END {
            return managed && found;
        }
        if managed && trimmed == expected {
            found = true;
        }
    }
    false
}

fn remove_managed_config(text: &str) -> Result<String, AppError> {
    let mut kept = Vec::new();
    let mut managed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == CONFIG_BEGIN {
            if managed {
                return Err(engine(
                    "Codex 模型目录配置块重复或损坏，已保留原配置；请手动检查 config.toml",
                ));
            }
            managed = true;
            continue;
        }
        if managed {
            if trimmed == CONFIG_END {
                managed = false;
            }
            continue;
        }
        kept.push(line);
    }
    if managed {
        return Err(engine(
            "Codex 模型目录配置块未闭合，已保留原配置；请手动检查 config.toml",
        ));
    }
    let mut result = kept.join("\n");
    if text.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    Ok(result)
}

fn has_unmanaged_catalog_key(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && trimmed.starts_with("model_catalog_json")
            && trimmed["model_catalog_json".len()..]
                .trim_start()
                .starts_with('=')
    })
}

fn merged_config(text: &str, path: &Path) -> Result<String, AppError> {
    let without_managed = remove_managed_config(text)?;
    if has_unmanaged_catalog_key(&without_managed) {
        return Err(engine(
            "Codex 已存在未由本管理器管理的 model_catalog_json，已保留原配置；请先确认后再刷新模型目录",
        ));
    }
    let block = format!(
        "{CONFIG_BEGIN}\nmodel_catalog_json = \"{}\"\n{CONFIG_END}",
        toml_string(path)
    );
    let body = without_managed.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        return Ok(format!("{block}\n"));
    }
    Ok(format!("{block}\n\n{body}\n"))
}

fn owned_catalog_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("identity")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|identity| identity == CATALOG_IDENTITY)
}

/// Remove only the catalog and managed config block created by this bridge.
/// User model-provider settings and an unrelated catalog are retained.
pub fn cleanup() -> Result<bool, AppError> {
    let catalog = catalog_path()?;
    let config = config_path()?;
    let config_text = match fs::read_to_string(&config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(engine(format!(
                "读取 Codex 配置失败，未执行模型目录清理：{error}"
            )))
        }
    };

    let mut changed = false;
    if has_managed_config(&config_text) {
        let cleaned = remove_managed_config(&config_text)?;
        atomic_file::write_atomic(&config, cleaned.as_bytes())
            .map_err(|error| engine(format!("移除 Codex 模型目录配置失败：{error}")))?;
        changed = true;
    }

    for path in [&catalog, &atomic_file::backup_path(&catalog)] {
        if !owned_catalog_file(path) {
            continue;
        }
        fs::remove_file(path).map_err(|error| {
            engine(format!(
                "删除 Codex 模型目录文件 {} 失败：{error}",
                path.display()
            ))
        })?;
        changed = true;
    }
    if let Ok(marker) = reload_marker_path() {
        if fs::remove_file(&marker).is_ok() {
            changed = true;
        }
    }
    Ok(changed)
}

fn write_catalog_and_config(
    catalog: &Value,
    catalog_path: &Path,
    config_path: &Path,
) -> Result<(), AppError> {
    let existing_config = fs::read_to_string(config_path).unwrap_or_default();
    let new_config = merged_config(&existing_config, catalog_path)?;
    let catalog_bytes = serde_json::to_vec_pretty(catalog)
        .map_err(|error| engine(format!("模型目录序列化失败：{error}")))?;
    atomic_file::write_atomic(catalog_path, &catalog_bytes)
        .map_err(|error| engine(format!("写入 Codex 模型目录失败：{error}")))?;
    atomic_file::write_atomic(config_path, new_config.as_bytes())
        .map_err(|error| engine(format!("写入 Codex 模型目录配置失败：{error}")))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayFailureKind {
    Unavailable,
    Unauthorized,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayFetchFailure {
    attempts: u32,
    kind: GatewayFailureKind,
    status: Option<u16>,
}

fn gateway_failure_message(failure: &GatewayFetchFailure, has_previous_catalog: bool) -> String {
    let suffix = if has_previous_catalog {
        "继续使用上次成功的模型目录。"
    } else {
        "暂未覆盖本地模型目录，将继续使用 Codex 默认目录。"
    };
    match failure.kind {
        GatewayFailureKind::Unauthorized => format!(
            "404token 上游拒绝了当前 U 盘授权（HTTP {}）；这不是本地安装问题，请联系管理员检查授权。{}",
            failure.status.unwrap_or(401),
            suffix
        ),
        GatewayFailureKind::InvalidResponse => format!(
            "404token 上游返回了无效的模型目录（已尝试 {} 次）；这不是本地安装问题。{}",
            failure.attempts, suffix
        ),
        GatewayFailureKind::Unavailable => format!(
            "404token 上游服务器/网络暂时不可达（已重试 {} 次）；这不是本地安装问题。{}",
            failure.attempts, suffix
        ),
    }
}

fn gateway_client() -> Result<reqwest::Client, GatewayFetchFailure> {
    // The application intentionally enables reqwest's `rustls-no-provider`
    // feature (the updater owns the provider choice).  A fresh manager can
    // reach this path before the account module has created its client, so
    // installing the process provider here is mandatory.  Without it,
    // reqwest panics while building the client and the request never reaches
    // 404token; a retry loop cannot recover from that failure.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        // Keep each attempt bounded. The caller applies a small backoff and a
        // hard attempt limit so a sleeping 404token upstream cannot leave the
        // manager's refresh action spinning forever.
        .connect_timeout(GATEWAY_CONNECT_TIMEOUT)
        .timeout(GATEWAY_REQUEST_TIMEOUT)
        .user_agent("Codex-App-Manager/native-model-catalog")
        .build()
        .map_err(|error| {
            log::warn!("native model catalog network client initialization failed: {error}");
            GatewayFetchFailure {
                attempts: 0,
                kind: GatewayFailureKind::Unavailable,
                status: None,
            }
        })
}

async fn fetch_gateway_models_from(
    api_key: &str,
    endpoint: &str,
) -> Result<Vec<GatewayModel>, GatewayFetchFailure> {
    let client = gateway_client()?;

    let mut last_failure = GatewayFetchFailure {
        attempts: 0,
        kind: GatewayFailureKind::Unavailable,
        status: None,
    };
    for attempt in 1..=GATEWAY_MAX_ATTEMPTS {
        last_failure.attempts = attempt;
        match client.get(endpoint).bearer_auth(api_key).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(payload) => {
                        let models = parse_gateway_models(&payload);
                        if !models.is_empty() {
                            return Ok(models);
                        }
                        last_failure.kind = GatewayFailureKind::InvalidResponse;
                        last_failure.status = None;
                    }
                    Err(error) => {
                        log::warn!(
                            "native model catalog upstream response parse failed on attempt {attempt}: {error}"
                        );
                        last_failure.kind = GatewayFailureKind::InvalidResponse;
                        last_failure.status = None;
                    }
                }
            }
            Ok(response) => {
                let status = response.status().as_u16();
                log::warn!(
                    "native model catalog upstream returned HTTP {status} on attempt {attempt}"
                );
                last_failure.status = Some(status);
                last_failure.kind = if matches!(status, 401 | 403) {
                    GatewayFailureKind::Unauthorized
                } else {
                    GatewayFailureKind::Unavailable
                };
                // A credential rejection is deterministic; retrying it only
                // delays the useful administrator-facing diagnosis.
                if matches!(status, 401 | 403) {
                    break;
                }
            }
            Err(error) => {
                log::warn!(
                    "native model catalog upstream request failed on attempt {attempt}: {error}"
                );
                last_failure.kind = GatewayFailureKind::Unavailable;
                last_failure.status = None;
            }
        }
        if attempt < GATEWAY_MAX_ATTEMPTS {
            let backoff = Duration::from_millis(500_u64.saturating_mul(1_u64 << (attempt - 1)));
            tokio::time::sleep(backoff).await;
        }
    }
    Err(last_failure)
}

async fn fetch_gateway_models(api_key: &str) -> Result<Vec<GatewayModel>, GatewayFetchFailure> {
    fetch_gateway_models_from(api_key, GATEWAY_MODELS_URL).await
}

/// Refresh the native Codex model menu.  A failed refresh never removes the
/// last good file; callers may display the returned status and continue.
pub async fn refresh(force: bool) -> Result<NativeModelCatalogReport, AppError> {
    let catalog = catalog_path()?;
    let config = config_path()?;
    let old_count = existing_catalog_count(&catalog);
    let config_is_current = fs::read_to_string(&config)
        .map(|text| managed_config_points_to_catalog(&text, &catalog))
        .unwrap_or(false);
    if !force && catalog_is_fresh(&catalog) && old_count > 0 && config_is_current {
        return Ok(report(
            "fresh",
            old_count,
            Some(&catalog),
            None,
            "Codex 模型目录仍在有效期内",
        ));
    }

    let Some(binding) = usb_guard::authorized_account_binding()? else {
        return Ok(report(
            "skipped",
            old_count,
            Some(&catalog),
            None,
            "当前没有可用于刷新统一网关目录的 U 盘绑定",
        ));
    };
    if binding.ccswitch.provider_id != "404token-sub2api" {
        return Ok(report(
            "skipped",
            old_count,
            Some(&catalog),
            None,
            "当前 U 盘绑定不是 404token 统一网关，未刷新模型目录",
        ));
    }
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        return Ok(report(
            "skipped",
            old_count,
            Some(&catalog),
            None,
            "当前 U 盘绑定没有可用的统一网关 Key",
        ));
    }

    let fetched = match fetch_gateway_models(api_key).await {
        Ok(models) => models,
        Err(failure) => {
            let message = gateway_failure_message(&failure, old_count > 0);
            log::warn!(
                "native model catalog refresh failed after {} attempt(s): {}",
                failure.attempts,
                message
            );
            return Ok(report_with_details(
                if old_count > 0 { "stale" } else { "failed" },
                old_count,
                Some(&catalog),
                None,
                message,
                failure.attempts,
                Some(match failure.kind {
                    GatewayFailureKind::Unauthorized => "upstream-auth",
                    GatewayFailureKind::InvalidResponse => "upstream-response",
                    GatewayFailureKind::Unavailable => "upstream-unavailable",
                }),
            ));
        }
    };
    let normalized = normalize_models(fetched);
    if normalized.is_empty() {
        return Ok(report_with_details(
            if old_count > 0 { "stale" } else { "failed" },
            old_count,
            Some(&catalog),
            None,
            gateway_failure_message(
                &GatewayFetchFailure {
                    attempts: GATEWAY_MAX_ATTEMPTS,
                    kind: GatewayFailureKind::InvalidResponse,
                    status: None,
                },
                old_count > 0,
            ),
            GATEWAY_MAX_ATTEMPTS,
            Some("upstream-response"),
        ));
    }

    let native = native_catalog(&normalized);
    let fetched_at = native
        .get("fetched_at")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Err(error) = write_catalog_and_config(&native, &catalog, &config) {
        log::warn!("native model catalog commit failed: {error}");
        return Ok(report(
            if old_count > 0 { "stale" } else { "failed" },
            old_count,
            Some(&catalog),
            None,
            "模型目录已获取，但未能安全写入 Codex 配置",
        ));
    }
    if let Err(error) = mark_restart_required() {
        log::warn!("native model catalog refreshed but reload marker was not written: {error}");
    }
    log::info!(
        "native model catalog refreshed models={} path={}",
        normalized.len(),
        catalog.display()
    );
    Ok(report(
        "ready",
        normalized.len(),
        Some(&catalog),
        fetched_at,
        "Codex 原生模型目录已刷新",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        managed_config_points_to_catalog, merged_config, native_catalog, normalize_models,
        parse_date_suffix, parse_gateway_models, remove_managed_config, write_catalog_and_config,
        AliasKind, GatewayModel, CATALOG_IDENTITY,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn gateway_client_initializes_rustls_provider_before_network_io() {
        let client = super::gateway_client()
            .expect("model catalog HTTPS client should build without an account request");
        drop(client);
    }

    #[test]
    fn simulated_gateway_http_response_reaches_parser_with_bearer_auth() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local gateway fixture");
        let address = listener.local_addr().expect("local gateway address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept gateway fixture request");
            let mut request = [0_u8; 4096];
            let read = stream
                .read(&mut request)
                .expect("read gateway fixture request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("Authorization: Bearer sk-simulated"));
            let body = r#"{"object":"list","data":[{"id":"gpt-6","owned_by":"openai"},{"id":"qwen-next","owned_by":"qwen"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write gateway fixture response");
        });
        let endpoint = format!("http://{address}/v1/models");
        let models = tauri::async_runtime::block_on(super::fetch_gateway_models_from(
            "sk-simulated",
            &endpoint,
        ))
        .expect("simulated gateway response should parse");
        server.join().expect("gateway fixture thread");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].raw_id, "gpt-6");
        assert_eq!(models[1].owned_by.as_deref(), Some("qwen"));
    }

    #[test]
    fn simulated_gateway_payload_round_trips_through_catalog_and_config() {
        let root = std::env::temp_dir().join(format!(
            "codex-native-model-catalog-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temporary Codex home");
        let catalog_path = root.join("codex-usb-native-model-catalog.json");
        let config_path = root.join("config.toml");
        let payload = json!({
            "object": "list",
            "data": [
                { "id": "deepseek-chat-2026-09-01", "owned_by": "deepseek" },
                { "id": "deepseek-chat", "owned_by": "deepseek" },
                { "id": "glm-5-pro", "owned_by": "zhipu" },
                { "id": "qwen-next", "owned_by": "qwen" }
            ]
        });
        let models = normalize_models(parse_gateway_models(&payload));
        assert_eq!(models.len(), 3);
        let catalog = native_catalog(&models);
        write_catalog_and_config(&catalog, &catalog_path, &config_path)
            .expect("simulated model refresh should commit both files");

        let written: Value =
            serde_json::from_slice(&fs::read(&catalog_path).expect("catalog file should exist"))
                .expect("catalog JSON should be valid");
        assert_eq!(written["identity"], CATALOG_IDENTITY);
        assert_eq!(written["models"].as_array().map(Vec::len), Some(3));
        assert!(written.to_string().contains("deepseek-chat"));
        let config = fs::read_to_string(&config_path).expect("config file should exist");
        assert!(managed_config_points_to_catalog(&config, &catalog_path));
        assert!(config.contains("# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1"));
        assert!(!config.contains("api_key"));

        fs::remove_dir_all(&root).expect("remove temporary Codex home");
    }

    #[test]
    fn preserves_vendors_and_merges_only_release_aliases() {
        let models = normalize_models(vec![
            GatewayModel {
                raw_id: "gpt-6-20260901".into(),
                owned_by: Some("openai".into()),
            },
            GatewayModel {
                raw_id: "gpt-6".into(),
                owned_by: Some("openai".into()),
            },
            GatewayModel {
                raw_id: "qwen-next-2026-09-01".into(),
                owned_by: Some("qwen".into()),
            },
            GatewayModel {
                raw_id: "qwen-next".into(),
                owned_by: Some("qwen".into()),
            },
            GatewayModel {
                raw_id: "deepseek-chat-latest".into(),
                owned_by: Some("deepseek".into()),
            },
            GatewayModel {
                raw_id: "deepseek-chat".into(),
                owned_by: Some("deepseek".into()),
            },
            GatewayModel {
                raw_id: "deepseek-reasoner".into(),
                owned_by: Some("deepseek".into()),
            },
            GatewayModel {
                raw_id: "glm-5".into(),
                owned_by: Some("zhipu".into()),
            },
            GatewayModel {
                raw_id: "glm-5-pro".into(),
                owned_by: Some("zhipu".into()),
            },
            GatewayModel {
                raw_id: "rare-vendor-20260101".into(),
                owned_by: Some("rare".into()),
            },
        ]);
        assert_eq!(models.len(), 7);
        assert!(models.iter().any(|model| model.raw_id == "gpt-6"));
        assert!(models.iter().any(|model| model.raw_id == "deepseek-chat"));
        assert!(models
            .iter()
            .any(|model| model.raw_id == "deepseek-reasoner"));
        assert!(models.iter().any(|model| model.raw_id == "glm-5-pro"));
        assert!(models
            .iter()
            .any(|model| model.raw_id == "rare-vendor-20260101"));
        assert!(models.iter().any(|model| model.raw_id == "qwen-next"));
        assert!(models
            .iter()
            .all(|model| model.alias_kind == AliasKind::Exact
                || model.raw_id == "rare-vendor-20260101"));
    }

    #[test]
    fn date_alias_validation_rejects_impossible_dates() {
        assert_eq!(parse_date_suffix("20260901"), Some(20260901));
        assert_eq!(parse_date_suffix("2026-09-01"), Some(20260901));
        assert_eq!(parse_date_suffix("2026-02-29"), None);
        assert_eq!(parse_date_suffix("2024-02-29"), Some(20240229));
        assert_eq!(parse_date_suffix("20261301"), None);
    }

    #[test]
    fn generated_catalog_keeps_raw_ids_and_contains_no_credentials() {
        let models = normalize_models(vec![GatewayModel {
            raw_id: "deepseek-chat-latest".into(),
            owned_by: Some("deepseek".into()),
        }]);
        let catalog = native_catalog(&models).to_string();
        assert!(catalog.contains("\"slug\":\"deepseek-chat-latest\""));
        assert!(catalog.contains("deepseek / deepseek-chat"));
        assert!(!catalog.contains("api_key"));
        assert!(!catalog.contains("Authorization"));
    }

    #[test]
    fn truncated_managed_config_is_rejected_without_discarding_user_text() {
        let text = "model = \"gpt-6\"\n# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1\nmodel_catalog_json = \"x\"\n";
        assert!(remove_managed_config(text).is_err());
    }

    #[test]
    fn managed_config_removal_keeps_unmanaged_settings() {
        let text = "model = \"gpt-6\"\n# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1\nmodel_catalog_json = \"x\"\n# END CODEX-USB-NATIVE-MODEL-CATALOG v1\nprofile = \"default\"\n";
        let cleaned = remove_managed_config(text).unwrap();
        assert_eq!(cleaned, "model = \"gpt-6\"\nprofile = \"default\"\n");
    }

    #[test]
    fn parses_openai_compatible_data_and_never_reads_credentials() {
        let payload = json!({
            "data": [
                { "id": "gpt-6", "owned_by": "openai", "api_key": "must-not-be-read" },
                { "id": "glm-5", "ownedBy": "zhipu" }
            ]
        });
        let models = parse_gateway_models(&payload);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].raw_id, "gpt-6");
        assert_eq!(models[1].owned_by.as_deref(), Some("zhipu"));
    }

    #[test]
    fn managed_config_is_at_root_and_unmanaged_key_is_preserved() {
        let path = Path::new(r"C:\Users\tester\.codex\codex-usb-native-model-catalog.json");
        let merged = merged_config(
            "[model_providers.gateway]\nbase_url = \"https://example.test/v1\"\n",
            path,
        )
        .unwrap();
        assert!(merged.starts_with("# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1\n"));
        assert!(merged.contains("[model_providers.gateway]"));
        assert!(merged.contains("model_catalog_json = \"C:\\\\Users\\\\tester"));
        assert!(managed_config_points_to_catalog(&merged, path));
        assert!(!managed_config_points_to_catalog(
            &merged.replace("codex-usb-native-model-catalog.json", "other.json"),
            path
        ));

        let conflict = merged_config("model_catalog_json = \"other.json\"\n", path);
        assert!(conflict.is_err());
    }
}
