//! Explicit, read-only diagnostics for the signed 404token USB binding.
//! Probe results are transient observations and never alter Codex/CCSwitch state.

use crate::app::model_capability::{CapabilityState, ModelCapabilityReport};
use crate::app::model_diagnostics::{digest_hex, safe_identifier, ProbeError};
use crate::app::model_probe::{ProbeClient, REFRESH_BUDGET};
use crate::app::usb_guard;
use crate::errors::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TARGET_PROVIDER_ID: &str = "404token-sub2api";
const MAX_MODELS: usize = 3;
const MAX_CATALOG_MODELS: usize = 512;
const MODEL_MATRIX_BUDGET: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayModelProbeRequest {
    pub request_id: String,
    #[serde(default)]
    pub selected_model_ids: Vec<String>,
    #[serde(default)]
    pub cost_consent: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProbeModel {
    pub id: String,
    pub owned_by: Option<String>,
    pub selectable: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProbeModelResult {
    pub model_id: String,
    pub passed: bool,
    pub generated_requests: usize,
    pub checks: Vec<GatewayProbeCheck>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProbeCheck {
    pub name: &'static str,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayModelProbeReport {
    /// connected, passed, failed, or cancelled
    pub status: String,
    pub endpoint: String,
    /// A shortened one-way fingerprint; never the bearer credential.
    pub credential_fingerprint: String,
    pub checked_at: u64,
    pub models: Vec<GatewayProbeModel>,
    pub results: Vec<GatewayProbeModelResult>,
    pub generated_requests: usize,
    pub error_code: Option<String>,
}

static ACTIVE: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn active_runs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ActiveRun {
    request_id: String,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        let mut runs = active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if runs
            .get(&self.request_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.cancelled))
        {
            runs.remove(&self.request_id);
        }
    }
}

fn begin_run(request_id: &str) -> Result<ActiveRun, AppError> {
    if !safe_identifier(request_id) {
        return Err(engine("探测请求标识无效"));
    }
    let mut runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !runs.is_empty() {
        return Err(AppError::Busy("已有 404token 探测正在运行".into()));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    runs.insert(request_id.to_owned(), cancelled.clone());
    Ok(ActiveRun {
        request_id: request_id.to_owned(),
        cancelled,
    })
}

pub fn cancel(request_id: &str) -> bool {
    let runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(cancelled) = runs.get(request_id) {
        cancelled.store(true, Ordering::Release);
        true
    } else {
        false
    }
}

fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(message.into())
}

fn validate_request(request: &GatewayModelProbeRequest) -> Result<(), AppError> {
    if !safe_identifier(&request.request_id) {
        return Err(engine("探测请求标识无效"));
    }
    if request.selected_model_ids.len() > MAX_MODELS {
        return Err(engine("一次最多选择三个模型"));
    }
    let mut selected = BTreeSet::new();
    for model_id in &request.selected_model_ids {
        if !safe_identifier(model_id) || !selected.insert(model_id) {
            return Err(engine("所选模型标识无效或重复"));
        }
    }
    if !request.selected_model_ids.is_empty() && !request.cost_consent {
        return Err(engine("完整网关探测需要本次单独费用确认"));
    }
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_model_catalog(
    payload: &Value,
    api_key: &str,
) -> Result<Vec<GatewayProbeModel>, ProbeError> {
    let rows = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= MAX_CATALOG_MODELS)
        .ok_or(ProbeError::InvalidResponse)?;

    let mut parsed = Vec::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let Some(id) = object
            .get("id")
            .or_else(|| object.get("slug"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| safe_identifier(id) && !id.contains(api_key))
        else {
            continue;
        };
        let owned_by = object
            .get("owned_by")
            .or_else(|| object.get("ownedBy"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|owner| safe_identifier(owner) && !owner.contains(api_key))
            .map(ToOwned::to_owned);
        parsed.push((id.to_owned(), owned_by));
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for (id, _) in &parsed {
        *counts.entry(id.clone()).or_default() += 1;
    }
    Ok(parsed
        .into_iter()
        .map(|(id, owned_by)| GatewayProbeModel {
            selectable: counts.get(&id) == Some(&1),
            id,
            owned_by,
        })
        .collect())
}

fn resolve_selected_models(
    catalog: &[GatewayProbeModel],
    selected: &[String],
) -> Result<Vec<GatewayProbeModel>, AppError> {
    selected
        .iter()
        .map(|id| {
            let mut matches = catalog
                .iter()
                .filter(|model| model.id == *id && model.selectable);
            let model = matches.next().cloned();
            if model.is_none() || matches.next().is_some() {
                return Err(engine(
                    "所选模型不在当前唯一模型目录中；请刷新目录后重新选择",
                ));
            }
            model.ok_or_else(|| engine("所选模型不在当前唯一模型目录中"))
        })
        .collect()
}

fn gateway_checks(model: &ModelCapabilityReport) -> Vec<GatewayProbeCheck> {
    [
        ("text", model.checks.text),
        ("singleTool", model.checks.single_tool),
        ("toolResult", model.checks.tool_result),
        ("multipleTools", model.checks.multiple_tools),
        ("toolError", model.checks.tool_error),
        ("stream", model.checks.stream),
        ("multiTurn", model.checks.multi_turn),
    ]
    .into_iter()
    .map(|(name, state)| GatewayProbeCheck {
        name,
        passed: state == CapabilityState::Verified,
    })
    .collect()
}

pub async fn run(request: GatewayModelProbeRequest) -> Result<GatewayModelProbeReport, AppError> {
    validate_request(&request)?;
    let active = begin_run(&request.request_id)?;
    let binding = usb_guard::authorized_account_binding()?
        .ok_or_else(|| engine("未取得有效的签名 U 盘账号绑定"))?;
    if binding.ccswitch.provider_id != TARGET_PROVIDER_ID {
        return Err(engine("当前 U 盘绑定不是 404token；未发送探测请求"));
    }
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        return Err(engine("签名 U 盘绑定缺少设备凭证"));
    }
    let base = crate::app::model_catalog::provider_base(&binding.ccswitch.provider_id)?;
    let catalog_client = ProbeClient::new(
        &base,
        api_key,
        active.cancelled.clone(),
        Instant::now() + REFRESH_BUDGET,
    )
    .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
    .with_run_id(&request.request_id);

    // Both UI modes use exactly one authenticated /v1/models request. The
    // lightweight mode returns here and cannot emit a Responses generation.
    let catalog_payload = catalog_client
        .models()
        .await
        .map_err(|error| engine(format!("404token 模型目录请求失败：{}", error.code())))?;
    let models = parse_model_catalog(&catalog_payload, api_key)
        .map_err(|error| engine(format!("404token 模型目录无效：{}", error.code())))?;
    let mut report = GatewayModelProbeReport {
        status: "connected".into(),
        endpoint: base.clone(),
        credential_fingerprint: format!("sha256:{}", &digest_hex(api_key.as_bytes())[..12]),
        checked_at: unix_seconds(),
        models,
        results: Vec::new(),
        generated_requests: 0,
        error_code: None,
    };

    if request.selected_model_ids.is_empty() {
        return Ok(report);
    }
    let selected = resolve_selected_models(&report.models, &request.selected_model_ids)?;
    for selected_model in selected {
        if active.cancelled.load(Ordering::Acquire) {
            report.status = "cancelled".into();
            report.error_code = Some(ProbeError::Cancelled.code().into());
            break;
        }
        let matrix_client = ProbeClient::new(
            &base,
            api_key,
            active.cancelled.clone(),
            Instant::now() + MODEL_MATRIX_BUDGET,
        )
        .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
        .with_run_id(&request.request_id);
        let mut capability = ModelCapabilityReport::discovered(
            selected_model.id.clone(),
            selected_model.owned_by.clone(),
        );
        matrix_client
            .probe_model(&mut capability, unix_seconds())
            .await;
        let generated_requests = matrix_client.requests_started();
        report.generated_requests = report.generated_requests.saturating_add(generated_requests);
        let checks = gateway_checks(&capability);
        let passed = checks.iter().all(|check| check.passed);
        report.results.push(GatewayProbeModelResult {
            model_id: selected_model.id,
            passed,
            generated_requests,
            checks,
            error_code: capability.error_code,
        });
        if active.cancelled.load(Ordering::Acquire) {
            report.status = "cancelled".into();
            report.error_code = Some(ProbeError::Cancelled.code().into());
            break;
        }
    }
    if report.status != "cancelled" {
        report.status = if report.results.iter().all(|result| result.passed) {
            "passed".into()
        } else {
            "failed".into()
        };
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(selected_model_ids: Vec<String>, cost_consent: bool) -> GatewayModelProbeRequest {
        GatewayModelProbeRequest {
            request_id: "gateway-probe-fixture-1".into(),
            selected_model_ids,
            cost_consent,
        }
    }

    #[test]
    fn full_matrix_is_rejected_without_consent_before_any_client_is_created() {
        let error = validate_request(&request(vec!["glm-5.2".into()], false)).unwrap_err();
        assert!(error.to_string().contains("费用确认"));
    }

    #[test]
    fn selection_is_bounded_unique_and_restricted_to_directory_rows() {
        let catalog = parse_model_catalog(
            &serde_json::json!({"data":[
                {"id":"glm-5.2","owned_by":"fixture"},
                {"id":"duplicate","owned_by":"fixture"},
                {"id":"duplicate","owned_by":"fixture"},
                {"id":"sk-secret-value","owned_by":"fixture"}
            ]}),
            "fixture-device-key",
        )
        .unwrap();
        assert_eq!(catalog.len(), 3);
        assert!(catalog[0].selectable);
        assert!(!catalog[1].selectable);
        assert!(!catalog[2].selectable);
        assert!(resolve_selected_models(&catalog, &["glm-5.2".into()]).is_ok());
        assert!(resolve_selected_models(&catalog, &["duplicate".into()]).is_err());
        assert!(resolve_selected_models(&catalog, &["not-listed".into()]).is_err());
    }

    #[test]
    fn requests_reject_too_many_duplicate_or_unsafe_ids() {
        assert!(validate_request(&request(vec!["a".into(); 4], true)).is_err());
        assert!(validate_request(&request(vec!["a".into(), "a".into()], true)).is_err());
        assert!(validate_request(&request(vec!["sk-secret".into()], true)).is_err());
    }
}
