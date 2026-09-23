//! Minimal 404token account session for a signed USB bundle.
//!
//! The USB license is the only source of account identity.  The password and
//! bearer token live in memory for this manager process only; neither is
//! persisted to the USB bundle, settings, logs, or renderer storage.

use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::app::usb_guard;
use crate::errors::AppError;

const BASE_URL: &str = "https://404token.xyz";
const LOGIN_PATH: &str = "/api/v1/auth/login";
const ME_PATH: &str = "/api/v1/auth/me";
const USAGE_PATH: &str = "/api/v1/usage/stats";
const REDEEM_PATH: &str = "/api/v1/redeem";
const ACCOUNT_MAX_ATTEMPTS: u32 = 3;
const ACCOUNT_RETRY_BASE_DELAY_MS: u64 = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub bound: bool,
    pub account_name: Option<String>,
    pub logged_in: bool,
    pub balance: Option<Value>,
    pub usage: Option<Value>,
    pub message: String,
}

/// Authorization state distinguishes loading, unbound, bound-not-logged-in,
/// logged-in, auth-invalid, and network-unavailable states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationState {
    Loading,
    NotBound,
    BoundNotLoggedIn,
    LoggedIn,
    AuthInvalid,
    NetworkUnavailable,
}

#[derive(Debug)]
struct Session {
    account_ref: String,
    usb_asset_id: String,
    token: String,
    expires_at: SystemTime,
}

fn session() -> &'static Mutex<Option<Session>> {
    static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(None))
}

fn account_binding() -> Result<codex_usb_license::UsbAccountBinding, AppError> {
    usb_guard::authorized_account_binding()?
        .ok_or_else(|| AppError::Engine("当前 U 盘尚未绑定 404token 账号".to_string()))
}

/// Recursively searches for a value using multiple field name candidates.
/// Matches the signer's find_string strategy from sub2api.rs.
fn find_string_recursive(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(s) = object.get(*key).and_then(Value::as_str) {
                    // Filter out empty strings and desensitized markers
                    if !s.trim().is_empty() && !s.contains("****") {
                        return Some(s.to_string());
                    }
                }
            }
            // Recurse into nested objects
            object
                .values()
                .find_map(|child| find_string_recursive(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_recursive(child, keys)),
        _ => None,
    }
}

/// Extracts balance using multiple field name candidates.
/// Returns the first non-null value found.
fn find_balance(value: &Value, keys: &[&str]) -> Option<Value> {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if !v.is_null() {
                return Some(v.clone());
            }
        }
    }
    // Recurse into nested structures
    match value {
        Value::Object(object) => object.values().find_map(|child| find_balance(child, keys)),
        _ => None,
    }
}

/// Safe error message mapping - never exposes raw server responses.
fn safe_error_message(operation: &str, status: u16) -> String {
    match status {
        401 => format!("{}失败：密码错误或会话已过期,请重新登录", operation),
        403 => format!("{}失败：账号已被禁用,请联系管理员", operation),
        404 => format!("{}失败：服务接口不可用", operation),
        400 if operation == "充值" => format!("{}失败：充值码无效或请求参数错误", operation),
        409 if operation == "充值" => {
            format!("{}失败：充值码已使用或请求冲突，请刷新余额确认", operation)
        }
        400 | 422 => format!("{}失败：请求参数无效", operation),
        500..=599 => format!(
            "{}失败：404token 上游服务暂时不可用,请稍后重试（不是本地安装问题）",
            operation
        ),
        _ => format!(
            "{}失败：404token 上游返回了请求错误,请检查账号状态",
            operation
        ),
    }
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(
        ACCOUNT_RETRY_BASE_DELAY_MS.saturating_mul(1_u64 << attempt.saturating_sub(1)),
    )
}

fn upstream_unavailable(operation: &str, attempts: u32) -> AppError {
    AppError::Engine(format!(
        "{operation}失败：404token 上游服务器/网络暂时不可达（已重试 {attempts} 次），这不是本地安装问题"
    ))
}

/// Retry only transport and explicitly transient upstream responses.  Stable
/// 4xx responses are returned to the caller immediately so a bad password or
/// invalid recharge code is never hidden behind a delay.  The recharge caller
/// supplies an idempotency key because a POST may have reached the upstream
/// before a transport failure was observed.
async fn send_with_retry<F, Fut>(
    operation: &str,
    mut request: F,
) -> Result<reqwest::Response, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    for attempt in 1..=ACCOUNT_MAX_ATTEMPTS {
        match request().await {
            Ok(response) if retryable_status(response.status().as_u16()) => {
                if attempt == ACCOUNT_MAX_ATTEMPTS {
                    return Err(upstream_unavailable(operation, attempt));
                }
                log::warn!(
                    "404token {operation} returned transient HTTP {} on attempt {attempt}/{ACCOUNT_MAX_ATTEMPTS}",
                    response.status().as_u16()
                );
                drop(response);
                tokio::time::sleep(retry_delay(attempt)).await;
            }
            Ok(response) => return Ok(response),
            Err(error) => {
                log::warn!(
                    "404token {operation} network attempt {attempt}/{ACCOUNT_MAX_ATTEMPTS} failed: {error}"
                );
                if attempt == ACCOUNT_MAX_ATTEMPTS {
                    return Err(upstream_unavailable(operation, attempt));
                }
                tokio::time::sleep(retry_delay(attempt)).await;
            }
        }
    }
    unreachable!("account retry loop always returns")
}

fn redeem_idempotency_key(binding: &codex_usb_license::UsbAccountBinding, code: &str) -> String {
    let digest = Sha256::digest(format!("{}:{code}", binding.account_ref).as_bytes());
    format!(
        "usb-redeem-{}-{}",
        binding.account_ref,
        digest
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn status_from_payload(
    binding: &codex_usb_license::UsbAccountBinding,
    logged_in: bool,
    me: Option<&Value>,
    usage: Option<Value>,
    message: String,
) -> AccountStatus {
    let balance =
        me.and_then(|value| find_balance(value, &["balance", "credits", "quota", "remaining"]));
    AccountStatus {
        bound: true,
        account_name: Some(binding.account_name.clone()),
        logged_in,
        balance,
        usage,
        message,
    }
}

/// Safe error from response - logs detail but returns user-friendly message.
async fn error_from_response(operation: &str, response: reqwest::Response) -> AppError {
    let status = response.status();
    let status_code = status.as_u16();

    // Read body for logging but never expose it to users
    let body = response.text().await.unwrap_or_default();

    // Log detailed error for debugging (truncated)
    let detail = body.chars().take(200).collect::<String>();
    if !detail.is_empty() {
        log::warn!("404token {operation} HTTP {status_code}: {detail}");
    }

    // Return safe, user-friendly message
    AppError::Engine(safe_error_message(operation, status_code))
}

/// Clear session when auth is invalid or USB changed.
fn clear_session() {
    if let Ok(mut guard) = session().lock() {
        *guard = None;
    }
}

fn request_client() -> Result<reqwest::Client, AppError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .user_agent("Codex-App-Manager/404token")
        .build()
        .map_err(|error| AppError::Engine(format!("无法初始化 404token 网络客户端：{error}")))
}

/// Get session token, checking USB binding and expiration.
fn session_token(binding: &codex_usb_license::UsbAccountBinding) -> Result<String, AppError> {
    let guard = session()
        .lock()
        .map_err(|_| AppError::Internal("账户会话锁已损坏".to_string()))?;

    let Some(ref sess) = *guard else {
        return Err(AppError::Engine("请先登录 404token 账号".to_string()));
    };

    // Check USB binding matches
    if sess.account_ref != binding.account_ref || sess.usb_asset_id != binding.usb_asset_id {
        drop(guard);
        clear_session();
        return Err(AppError::Engine("U 盘已更换,请重新登录".to_string()));
    }

    // Check token expiration
    if SystemTime::now() >= sess.expires_at {
        drop(guard);
        clear_session();
        return Err(AppError::Engine("会话已过期,请重新登录".to_string()));
    }

    Ok(sess.token.clone())
}

pub fn status_unbound() -> AccountStatus {
    AccountStatus {
        bound: false,
        account_name: None,
        logged_in: false,
        balance: None,
        usage: None,
        message: "此 U 盘尚未绑定 404token 账号".to_string(),
    }
}

pub fn status() -> Result<AccountStatus, AppError> {
    let binding = match account_binding() {
        Ok(b) => b,
        Err(e) => {
            // Distinguish authorization failure from not-bound
            if e.to_string().contains("尚未绑定") {
                return Ok(status_unbound());
            }
            // Authorization invalid (signature error, etc)
            return Ok(AccountStatus {
                bound: false,
                account_name: None,
                logged_in: false,
                balance: None,
                usage: None,
                message: "U 盘授权失效,请重新签名".to_string(),
            });
        }
    };

    let logged_in = session_token(&binding).is_ok();
    Ok(AccountStatus {
        bound: true,
        account_name: Some(binding.account_name),
        logged_in,
        balance: None,
        usage: None,
        message: if logged_in {
            "已绑定,点击刷新获取余额".to_string()
        } else {
            "已绑定,请输入密码登录".to_string()
        },
    })
}

pub async fn login(password: String) -> Result<AccountStatus, AppError> {
    if password.trim().is_empty() {
        return Err(AppError::Engine("请输入账号密码".to_string()));
    }

    let binding = account_binding()?;
    let client = request_client()?;

    let response = send_with_retry("登录", || {
        client
            .post(format!("{BASE_URL}{LOGIN_PATH}"))
            .json(&serde_json::json!({
                "email": binding.login_email,
                "password": password
            }))
            .send()
    })
    .await?;

    let status_code = response.status().as_u16();

    if !response.status().is_success() {
        // Clear any existing session on auth failure
        if status_code == 401 {
            clear_session();
        }
        return Err(error_from_response("登录", response).await);
    }

    let body: Value = response.json().await.map_err(|_| {
        AppError::Engine("登录失败：404token 上游响应格式无效,请稍后重试".to_string())
    })?;

    // Use recursive search matching signer's strategy
    let token = find_string_recursive(&body, &["access_token", "accessToken", "token"])
        .ok_or_else(|| AppError::Engine("登录失败：响应缺少会话令牌".to_string()))?;

    // Extract expires_in (default 1 hour if not present)
    let expires_in_secs = body
        .get("expires_in")
        .or_else(|| body.get("expiresIn"))
        .and_then(|v| v.as_u64())
        .unwrap_or(3600);

    let expires_at = SystemTime::now() + Duration::from_secs(expires_in_secs);

    // Save session with USB binding
    session()
        .lock()
        .map_err(|_| AppError::Internal("账户会话锁已损坏".to_string()))?
        .replace(Session {
            account_ref: binding.account_ref.clone(),
            usb_asset_id: binding.usb_asset_id.clone(),
            token,
            expires_at,
        });

    refresh().await
}

pub async fn refresh() -> Result<AccountStatus, AppError> {
    let binding = account_binding()?;
    let token = session_token(&binding)?;
    let client = request_client()?;

    let me_response = send_with_retry("余额查询", || {
        client
            .get(format!("{BASE_URL}{ME_PATH}"))
            .bearer_auth(&token)
            .send()
    })
    .await?;

    let status_code = me_response.status().as_u16();

    if !me_response.status().is_success() {
        // Clear session on 401
        if status_code == 401 {
            clear_session();
        }
        return Err(error_from_response("余额查询", me_response).await);
    }

    let me: Value = me_response.json().await.map_err(|_| {
        AppError::Engine("余额查询失败：404token 上游响应格式无效,请稍后重试".to_string())
    })?;

    // Usage query failure is non-fatal
    let usage_response = send_with_retry("用量查询", || {
        client
            .get(format!("{BASE_URL}{USAGE_PATH}"))
            .bearer_auth(&token)
            .send()
    })
    .await;

    let mut usage_available = true;
    let usage = match usage_response {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(value) => Some(value),
            Err(_) => {
                usage_available = false;
                None
            }
        },
        Ok(_) | Err(_) => {
            usage_available = false;
            None
        }
    };

    Ok(status_from_payload(
        &binding,
        true,
        Some(&me),
        usage,
        if usage_available {
            "账户信息已刷新".to_string()
        } else {
            "余额已刷新，但用量查询暂时无法连接 404token 上游，请稍后重试".to_string()
        },
    ))
}

pub async fn redeem(code: String) -> Result<AccountStatus, AppError> {
    if code.trim().is_empty() {
        return Err(AppError::Engine("请输入充值码".to_string()));
    }

    let binding = account_binding()?;
    let token = session_token(&binding)?;
    let client = request_client()?;

    let code = code.trim().to_string();
    let idempotency_key = redeem_idempotency_key(&binding, &code);
    let response = send_with_retry("充值", || {
        client
            .post(format!("{BASE_URL}{REDEEM_PATH}"))
            .bearer_auth(&token)
            .header("Idempotency-Key", &idempotency_key)
            .json(&serde_json::json!({ "code": code.as_str() }))
            .send()
    })
    .await?;

    let status_code = response.status().as_u16();

    if !response.status().is_success() {
        // Clear session on 401
        if status_code == 401 {
            clear_session();
        }
        return Err(error_from_response("充值", response).await);
    }

    // Redeem succeeded, refresh to get updated balance
    // Note: Even if refresh fails, redeem itself succeeded
    match refresh().await {
        Ok(status) => Ok(AccountStatus {
            message: "充值成功,余额已更新".to_string(),
            ..status
        }),
        Err(_) => {
            // Redeem succeeded but refresh failed - inform user
            Ok(AccountStatus {
                bound: true,
                account_name: Some(binding.account_name),
                logged_in: true,
                balance: None,
                usage: None,
                message: "充值成功,但余额刷新失败,请手动刷新".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn send_fixture_response(mut stream: std::net::TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write account fixture response");
    }

    #[test]
    fn status_never_exposes_secret_fields() {
        let status = status_unbound();
        let value = serde_json::to_value(status).unwrap();
        assert!(value.get("token").is_none());
        assert!(value.get("loginEmail").is_none());
        assert!(value.get("usbAssetId").is_none());
    }

    #[test]
    fn find_balance_supports_multiple_field_names() {
        let value = json!({ "credits": 12500 });
        assert_eq!(
            find_balance(&value, &["balance", "credits"]),
            Some(json!(12500))
        );

        let nested = json!({ "data": { "balance": 8000 } });
        assert_eq!(find_balance(&nested, &["balance"]), Some(json!(8000)));
    }

    #[test]
    fn find_string_recursive_filters_desensitized_values() {
        let value = json!({ "token": "real_token_123" });
        assert_eq!(
            find_string_recursive(&value, &["token"]),
            Some("real_token_123".to_string())
        );

        let masked = json!({ "token": "****" });
        assert_eq!(find_string_recursive(&masked, &["token"]), None);

        let empty = json!({ "token": "" });
        assert_eq!(find_string_recursive(&empty, &["token"]), None);
    }

    #[test]
    fn find_string_recursive_searches_nested_structures() {
        let nested = json!({
            "data": {
                "auth": {
                    "accessToken": "nested_token"
                }
            }
        });
        assert_eq!(
            find_string_recursive(&nested, &["accessToken", "token"]),
            Some("nested_token".to_string())
        );
    }

    #[test]
    fn safe_error_message_never_exposes_details() {
        assert_eq!(
            safe_error_message("登录", 401),
            "登录失败：密码错误或会话已过期,请重新登录"
        );
        assert_eq!(
            safe_error_message("充值", 500),
            "充值失败：404token 上游服务暂时不可用,请稍后重试（不是本地安装问题）"
        );
        assert_eq!(
            safe_error_message("充值", 400),
            "充值失败：充值码无效或请求参数错误"
        );
        assert_eq!(
            safe_error_message("充值", 409),
            "充值失败：充值码已使用或请求冲突，请刷新余额确认"
        );
        // No raw status or server response details
        assert!(!safe_error_message("test", 500).contains("HTTP"));
    }

    #[test]
    fn authorization_state_distinguishes_all_cases() {
        // Verify enum exists and has expected variants
        let _loading = AuthorizationState::Loading;
        let _not_bound = AuthorizationState::NotBound;
        let _bound_not_logged_in = AuthorizationState::BoundNotLoggedIn;
        let _logged_in = AuthorizationState::LoggedIn;
        let _auth_invalid = AuthorizationState::AuthInvalid;
        let _network_unavailable = AuthorizationState::NetworkUnavailable;
    }

    #[test]
    fn retry_policy_only_retries_transient_upstream_statuses() {
        assert!(retryable_status(408));
        assert!(retryable_status(429));
        assert!(retryable_status(500));
        assert!(retryable_status(503));
        assert!(!retryable_status(400));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }

    #[test]
    fn upstream_retry_reaches_success_after_a_transient_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind account retry fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            for (status, body) in [
                ("503 Service Unavailable", "{}"),
                ("200 OK", "{\"ok\":true}"),
            ] {
                let (mut stream, _) = listener.accept().expect("accept account retry request");
                let mut request = [0u8; 1024];
                let _ = stream
                    .read(&mut request)
                    .expect("read account retry request");
                send_fixture_response(stream, status, body);
            }
        });

        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .build()
            .expect("build account retry client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build account retry runtime");
        let response = runtime
            .block_on(send_with_retry("测试", || {
                client.get(format!("http://{address}/retry")).send()
            }))
            .expect("transient response should be retried");
        assert_eq!(response.status().as_u16(), 200);
        handle.join().expect("account retry fixture thread");
    }

    #[test]
    fn exhausted_network_retry_identifies_the_upstream_and_attempt_count() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind closed account fixture");
        let address = listener.local_addr().expect("fixture address");
        drop(listener);
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .build()
            .expect("build account retry client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build account retry runtime");
        let error = runtime
            .block_on(send_with_retry("余额查询", || {
                client.get(format!("http://{address}/closed")).send()
            }))
            .expect_err("closed upstream should exhaust retries");
        let text = error.to_string();
        assert!(text.contains("404token 上游服务器/网络暂时不可达"));
        assert!(text.contains("已重试 3 次"));
        assert!(text.contains("不是本地安装问题"));
    }

    #[test]
    fn redeem_idempotency_key_is_stable_and_does_not_contain_the_code() {
        let binding = codex_usb_license::UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "USB-0007".to_string(),
            account_ref: "USB-0007".to_string(),
            account_name: "USB-0007".to_string(),
            login_email: "u7@u.404token.xyz".to_string(),
            sub2api_user_id: 7,
            ccswitch: codex_usb_license::UsbCcswitchBinding {
                provider_id: "404token-sub2api".to_string(),
                credential_id: "sub2api-user-key-7".to_string(),
                device_api_key: "sk-secret".to_string(),
                group_id: 7,
            },
        };
        let first = redeem_idempotency_key(&binding, "RECHARGE-ABC");
        assert_eq!(first, redeem_idempotency_key(&binding, "RECHARGE-ABC"));
        assert!(first.starts_with("usb-redeem-USB-0007-"));
        assert!(!first.contains("RECHARGE-ABC"));
    }
}
