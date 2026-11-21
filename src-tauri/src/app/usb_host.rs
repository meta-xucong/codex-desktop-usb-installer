//! Per-user USB readiness. Program presence on the USB is deliberately separate.
use std::{fs, path::Path};

use serde::Serialize;
use serde_json::Value as Json;
use toml::Value;

use crate::app::{capability_pack, codex_config_readback as readback, usb_guard};
use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbHostStatus {
    pub required: bool,
    pub ready: bool,
    pub account_ready: bool,
    pub capabilities_ready: bool,
    pub conflict: bool,
    pub message: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AccountState {
    Ready,
    Missing,
    Conflict,
}

fn invalid_config() -> AppError {
    AppError::Engine(
        "本机 Codex 配置无法解析或当前 profile 无效；原设置未更改，请先在组件设置中处理。".into(),
    )
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(AppError::Engine(format!(
            "无法读取本机配置文件：{}",
            path.display()
        ))),
    }
}

pub(crate) fn read_config(home: &Path) -> Result<Value, AppError> {
    let bytes = read_optional(&home.join("config.toml"))?.unwrap_or_default();
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|_| invalid_config())?;
    text.parse().map_err(|_| invalid_config())
}

/// No model selection is required for account readiness. The active profile
/// takes precedence, just as it does in the existing model-selection readback.
pub(crate) fn account_state(
    config: &Value,
    auth: Option<&Json>,
    key: &str,
) -> Result<AccountState, AppError> {
    if auth.is_some_and(|v| !v.is_object())
        || config.get("model_providers").is_some_and(|v| !v.is_table())
        || config.get("mcp_servers").is_some_and(|v| !v.is_table())
    {
        return Err(invalid_config());
    }
    let profile = readback::selected_profile(config).map_err(|_| invalid_config())?;
    let active = readback::effective_string(config, profile, "model_provider");
    let provider = active.and_then(|id| config.get("model_providers")?.get(id));
    let endpoint = provider
        .and_then(|v| v.get("base_url"))
        .and_then(Value::as_str);
    let endpoint_matches = endpoint.and_then(readback::canonical_base_url)
        == readback::canonical_base_url("https://404token.xyz/v1");
    let auth_key = auth
        .and_then(|v| v.get("OPENAI_API_KEY"))
        .and_then(Json::as_str)
        .filter(|v| !v.is_empty());
    // CCSwitch 4.0.20's native field writer stores a custom route's key
    // here. auth.json is retained for official login, not used as the
    // credential for that route. Older USB installations used auth.json.
    let bearer = provider
        .and_then(|v| v.get("experimental_bearer_token"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty());
    let native_credential = bearer == Some(key);
    let foreign_auth = auth.is_some_and(|v| {
        v.get("tokens").is_some_and(|v| !v.is_null())
            || v.get("auth_mode")
                .and_then(Json::as_str)
                .is_some_and(|v| v != "apikey")
    });
    let auth_override = provider.is_some_and(|v| {
        [
            "env_key",
            "http_headers",
            "env_http_headers",
            "auth",
            "aws",
            "query_params",
        ]
        .iter()
        .any(|name| v.get(*name).is_some())
    }) || ["openai_base_url", "env_key", "experimental_bearer_token"].iter()
        .any(|field| config.get(*field).is_some() || profile.is_some_and(|p| p.get(*field).is_some()))
        || readback::effective_string(config, profile, "forced_login_method")
        == Some("chatgpt")
        || readback::effective_string(config, profile, "cli_auth_credentials_store")
            .is_some_and(|v| v != "file");
    let reserved_conflict = !endpoint_matches
        && config
            .get("model_providers")
            .and_then(|p| p.get("404token"))
            .and_then(|p| p.get("base_url"))
            .and_then(Value::as_str)
            .is_some_and(|v| {
                readback::canonical_base_url(v)
                    != readback::canonical_base_url("https://404token.xyz/v1")
            });
    if (foreign_auth && !native_credential)
        || auth_override
        || reserved_conflict
        || bearer.is_some_and(|v| v != key)
        || (!native_credential && auth_key.is_some_and(|v| v != key))
        || (active.is_some() && !endpoint_matches)
    {
        return Ok(AccountState::Conflict);
    }
    if endpoint_matches
        && (native_credential || auth_key == Some(key))
        && provider
            .and_then(|v| v.get("wire_api"))
            .and_then(Value::as_str)
            == Some("responses")
        && (provider
            .and_then(|v| v.get("requires_openai_auth"))
            .and_then(Value::as_bool)
            == Some(false)
            && native_credential
            || provider
                .and_then(|v| v.get("requires_openai_auth"))
                .and_then(Value::as_bool)
                == Some(true)
                && (auth_key == Some(key) || foreign_auth))
    {
        return Ok(AccountState::Ready);
    }
    Ok(AccountState::Missing)
}

pub fn status() -> Result<UsbHostStatus, AppError> {
    host_status(None, true)
}

fn host_status(config: Option<&Value>, probe_media: bool) -> Result<UsbHostStatus, AppError> {
    if !usb_guard::guard_required() {
        return Ok(UsbHostStatus {
            required: false,
            ready: true,
            account_ready: true,
            capabilities_ready: true,
            conflict: false,
            message: String::new(),
        });
    }
    let binding = usb_guard::authorized_account_binding()?
        .ok_or_else(|| AppError::Engine("未取得当前 U 盘账号绑定".into()))?;
    let home = usb_guard::local_data_paths()?.codex_home;
    if binding.ccswitch.provider_id != readback::USB_404TOKEN_PROVIDER_ID {
        return Err(AppError::Engine("当前 U 盘账号类型不支持本机初始化".into()));
    }
    let disk_config;
    let config = match config {
        Some(config) => config,
        None => { disk_config = read_config(&home)?; &disk_config }
    };
    let auth = read_optional(&home.join("auth.json"))?
        .map(|bytes| serde_json::from_slice::<Json>(&bytes).map_err(|_| invalid_config()))
        .transpose()?;
    let account = account_state(config, auth.as_ref(), &binding.ccswitch.device_api_key)?;
    let media_disabled = config
        .get("mcp_servers")
        .and_then(|v| v.get("media_generation"))
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        == Some(false);
    let core_ready = capability_pack::host_core_ready(&home, config)?;
    let media_error = if probe_media && core_ready && !media_disabled {
        match crate::app::media_generation::verify_authorized_local_connection() {
            Err(error) => Some(error.to_string()),
            Ok(()) if capability_pack::read_media_authorization_status(&home) != "verified" =>
                Some("媒体 MCP 本机授权状态尚未确认，启动时将重新初始化".into()),
            Ok(()) => None,
        }
    } else { None };
    let capabilities_ready = core_ready && media_error.is_none();
    let account_ready = account == AccountState::Ready;
    let conflict = account == AccountState::Conflict;
    let ready = account_ready && capabilities_ready;
    Ok(UsbHostStatus { required: true, ready, account_ready, capabilities_ready, conflict,
        message: if ready { "本机已就绪" } else if conflict {
            "启动时将自动使用当前 U 盘账号配置本机；模型、MCP 和其他偏好将保留。"
        } else if !account_ready { "程序已安装，本机账号尚未初始化" }
        else { media_error.as_deref().unwrap_or("程序已安装，本机核心能力尚未准备完成") }.into() })
}

pub fn ensure_ready() -> Result<(), AppError> {
    ensure_ready_for_open(None, true)
}

pub(crate) fn ensure_ready_for_open(config: Option<&Value>, probe_media: bool) -> Result<(), AppError> {
    let state = host_status(config, probe_media)?;
    if state.ready {
        Ok(())
    } else {
        Err(AppError::UsbHostNotReady(state.message))
    }
}

/// Account setup can succeed even if a capability dependency fails. Keep both
/// outcomes so the next launch only completes the remaining work.
pub(crate) fn complete_initialization(
    capabilities: impl FnOnce() -> Result<(), AppError>,
    account: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    let capability_result = capabilities();
    let account_result = account();
    let errors: Vec<String> = [capability_result, account_result].into_iter()
        .filter_map(Result::err).map(|error| error.to_string()).collect();
    if errors.is_empty() { Ok(()) } else { Err(AppError::Engine(errors.join("\n"))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_account_setup_runs_after_capability_failure_and_retains_both_errors() {
        let called = std::cell::Cell::new(false);
        let error = complete_initialization(
            || Err(AppError::Engine("capability fixture failure".into())),
            || { called.set(true); Err(AppError::Engine("account fixture failure".into())) },
        ).unwrap_err().to_string();
        assert!(called.get());
        assert!(error.contains("capability fixture failure"));
        assert!(error.contains("account fixture failure"));
    }
    #[test]
    fn independent_initialization_does_not_hide_a_partial_failure() {
        assert!(complete_initialization(|| Err(AppError::Engine("fixture".into())), || Ok(())).is_err());
        assert!(complete_initialization(|| Ok(()), || Ok(())).is_ok());
    }
    #[test]
    fn stale_root_or_profile_auth_is_not_treated_as_a_ready_usb_account() {
        let base: Value = "profile='work'\nmodel_provider='custom'\n[profiles.work]\nmodel='kept'\n[model_providers.custom]\nbase_url='https://404token.xyz/v1'\nwire_api='responses'\nrequires_openai_auth=false\nexperimental_bearer_token='fixture'\n".parse().unwrap();
        assert_eq!(account_state(&base, None, "fixture").unwrap(), AccountState::Ready);
        for field in ["openai_base_url", "env_key", "experimental_bearer_token"] {
            let mut root = base.clone();
            root.as_table_mut().unwrap().insert(field.into(), Value::String("old-fixture".into()));
            assert_eq!(account_state(&root, None, "fixture").unwrap(), AccountState::Conflict);
            let mut profile = base.clone();
            profile["profiles"]["work"].as_table_mut().unwrap().insert(field.into(), Value::String("old-fixture".into()));
            assert_eq!(account_state(&profile, None, "fixture").unwrap(), AccountState::Conflict);
        }
    }
    fn config() -> Value {
        r#"
model_provider = "custom"
model = "kept-model"
[model_providers.custom]
base_url = "https://404token.xyz/v1"
wire_api = "responses"
requires_openai_auth = true
"#
        .parse()
        .unwrap()
    }
    #[test]
    fn empty_host_is_missing_not_ready_or_conflict() {
        assert_eq!(
            account_state(&"".parse().unwrap(), None, "fixture").unwrap(),
            AccountState::Missing
        );
    }
    #[test]
    fn returning_host_keeps_alias_and_model() {
        let config = config();
        let auth = serde_json::json!({"OPENAI_API_KEY":"fixture"});
        assert_eq!(
            account_state(&config, Some(&auth), "fixture").unwrap(),
            AccountState::Ready
        );
        assert_eq!(config["model"].as_str(), Some("kept-model"));
    }
    #[test]
    fn native_field_writer_route_is_ready_without_auth_json() {
        let mut config = config();
        let route = config["model_providers"]["custom"].as_table_mut().unwrap();
        route.insert(
            "experimental_bearer_token".into(),
            Value::String("fixture".into()),
        );
        route.insert("requires_openai_auth".into(), Value::Boolean(false));
        assert_eq!(
            account_state(&config, None, "fixture").unwrap(),
            AccountState::Ready
        );
        assert_eq!(
            account_state(&config, None, "different").unwrap(),
            AccountState::Conflict
        );
        route_requirement(&mut config, true);
        assert_eq!(
            account_state(&config, None, "fixture").unwrap(),
            AccountState::Missing
        );
        let login = serde_json::json!({"tokens":{"access_token":"test-only"}});
        assert_eq!(
            account_state(&config, Some(&login), "fixture").unwrap(),
            AccountState::Ready
        );
    }
    fn route_requirement(config: &mut Value, requires: bool) {
        config["model_providers"]["custom"]
            .as_table_mut()
            .unwrap()
            .insert("requires_openai_auth".into(), Value::Boolean(requires));
    }
    #[test]
    fn profile_and_credentials_override_root() {
        let mut config = config();
        config
            .as_table_mut()
            .unwrap()
            .insert("profile".into(), Value::String("work".into()));
        config.as_table_mut().unwrap().insert(
            "profiles".into(),
            "[work]\nmodel_provider='openai'".parse().unwrap(),
        );
        let auth = serde_json::json!({"OPENAI_API_KEY":"fixture"});
        assert_eq!(
            account_state(&config, Some(&auth), "fixture").unwrap(),
            AccountState::Conflict
        );
        assert_eq!(
            account_state(&self::config(), Some(&auth), "other").unwrap(),
            AccountState::Conflict
        );
    }
    #[test]
    fn credential_overrides_and_official_tokens_are_not_ready() {
        let mut config = config();
        config["model_providers"]["custom"]
            .as_table_mut()
            .unwrap()
            .insert("env_key".into(), Value::String("OTHER_KEY".into()));
        let auth = serde_json::json!({"OPENAI_API_KEY":"fixture"});
        assert_eq!(
            account_state(&config, Some(&auth), "fixture").unwrap(),
            AccountState::Conflict
        );
        let auth = serde_json::json!({"OPENAI_API_KEY":"fixture", "tokens":{}});
        assert_eq!(
            account_state(&self::config(), Some(&auth), "fixture").unwrap(),
            AccountState::Conflict
        );
    }

    #[test]
    fn legacy_key_and_provider_alias_do_not_require_account_replacement() {
        let mut doc = config();
        let route = doc["model_providers"].as_table_mut().unwrap().remove("custom").unwrap();
        doc["model_providers"].as_table_mut().unwrap().insert("saved_alias".into(), route);
        doc["model_provider"] = Value::String("saved_alias".into());
        doc["model_providers"]["saved_alias"]["base_url"] =
            Value::String("https://404token.xyz/v1/".into());
        let auth = serde_json::json!({"auth_mode":"apikey", "OPENAI_API_KEY":"fixture", "tokens":null});
        let before = doc.clone();
        assert_eq!(account_state(&doc, Some(&auth), "fixture").unwrap(), AccountState::Ready);
        assert_eq!(doc, before);
        assert_eq!(account_state(&doc, Some(&auth), "other-usb-key").unwrap(), AccountState::Conflict);
    }

    #[test]
    fn selected_native_route_wins_over_inactive_accounts_and_saved_official_login() {
        let doc: Value = r#"
profile = "work"
model_provider = "openai"
model = "root-choice"
[model_providers.custom]
base_url = "https://404token.xyz/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "fixture"
[profiles.work]
model_provider = "custom"
model = "profile-choice"
[profiles.inactive]
model_provider = "openai"
forced_login_method = "chatgpt"
"#.parse().unwrap();
        let auth = serde_json::json!({"auth_mode":"chatgpt", "OPENAI_API_KEY":"old-key", "tokens":{"access_token":"test-only"}});
        let before = doc.clone();
        assert_eq!(account_state(&doc, Some(&auth), "fixture").unwrap(), AccountState::Ready);
        assert_eq!(doc, before);
        assert_eq!(account_state(&doc, Some(&auth), "another-usb-key").unwrap(), AccountState::Conflict);
    }

    #[test]
    fn active_auth_overrides_remain_conflicts_even_when_the_inline_key_matches() {
        for field in ["env_key", "http_headers", "env_http_headers", "auth", "aws", "query_params"] {
            let mut doc = config();
            let route = doc["model_providers"]["custom"].as_table_mut().unwrap();
            route.insert("experimental_bearer_token".into(), Value::String("fixture".into()));
            route.insert("requires_openai_auth".into(), Value::Boolean(false));
            route.insert(field.into(), Value::String("test-only".into()));
            assert_eq!(account_state(&doc, None, "fixture").unwrap(), AccountState::Conflict, "{field}");
        }
    }
}
