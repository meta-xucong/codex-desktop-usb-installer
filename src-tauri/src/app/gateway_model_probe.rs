//! Explicit, read-only diagnostics for the signed 404token USB binding.
//! Probe results are transient observations and never alter Codex/CCSwitch state.

use crate::app::model_capability::{CapabilityState, ModelCapabilityReport};
use crate::app::model_diagnostics::{digest_hex, safe_identifier, ProbeError};
use crate::app::model_probe::{ProbeClient, REFRESH_BUDGET};
use crate::app::usb_guard;
use crate::errors::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TARGET_PROVIDER_ID: &str = "404token-sub2api";
const MAX_MODELS: usize = 3;
const MAX_CATALOG_MODELS: usize = 512;
const MODEL_MATRIX_BUDGET: Duration = Duration::from_secs(10 * 60);
const PROOF_TTL: Duration = Duration::from_secs(5 * 60);
const PROOF_LIMIT: usize = 128;
const RUN_HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayModelProbeRequest {
    pub request_id: String,
    #[serde(default)]
    pub selected_model_ids: Vec<String>,
    #[serde(default)]
    pub cost_consent: bool,
    #[serde(default)]
    pub connectivity_model_id: Option<String>,
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
    /// One-way fingerprint of the account, provider, and endpoint binding.
    pub connection_fingerprint: String,
    pub checked_at: u64,
    pub models: Vec<GatewayProbeModel>,
    pub results: Vec<GatewayProbeModelResult>,
    pub generated_requests: usize,
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof_expires_at: Option<u64>,
}

#[derive(Default)]
struct ActiveState {
    active: HashMap<String, ActiveRequest>,
    cancelled_before_start: VecDeque<String>,
    finished: VecDeque<String>,
}

struct ActiveRequest {
    cancelled: Arc<AtomicBool>,
    commit_started: bool,
}

static ACTIVE: OnceLock<Mutex<ActiveState>> = OnceLock::new();

fn active_runs() -> &'static Mutex<ActiveState> {
    ACTIVE.get_or_init(|| Mutex::new(ActiveState::default()))
}

pub(crate) struct ActiveRun {
    request_id: String,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        let mut runs = active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if runs
            .active
            .get(&self.request_id)
            .is_some_and(|current| Arc::ptr_eq(&current.cancelled, &self.cancelled))
        {
            runs.active.remove(&self.request_id);
            remember_finished(&mut runs, &self.request_id);
        }
    }
}

fn remember_finished(state: &mut ActiveState, request_id: &str) {
    if !state.finished.iter().any(|id| id == request_id) {
        state.finished.push_back(request_id.to_owned());
        if state.finished.len() > RUN_HISTORY_LIMIT {
            state.finished.pop_front();
        }
    }
}

pub(crate) fn begin_management_run(request_id: &str) -> Result<ActiveRun, AppError> {
    if !safe_identifier(request_id) {
        return Err(engine("探测请求标识无效"));
    }
    let mut runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if runs.finished.iter().any(|id| id == request_id) {
        return Err(engine("该请求标识已使用；请重新发起操作"));
    }
    if let Some(position) = runs
        .cancelled_before_start
        .iter()
        .position(|id| id == request_id)
    {
        runs.cancelled_before_start.remove(position);
        remember_finished(&mut runs, request_id);
        return Err(engine("请求已取消；未发送模型请求"));
    }
    if !runs.active.is_empty() {
        return Err(AppError::Busy("已有 404token 探测正在运行".into()));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    runs.active.insert(
        request_id.to_owned(),
        ActiveRequest {
            cancelled: cancelled.clone(),
            commit_started: false,
        },
    );
    Ok(ActiveRun {
        request_id: request_id.to_owned(),
        cancelled,
    })
}

pub(crate) fn mark_catalog_commit_started(request_id: &str) {
    let mut runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(active) = runs.active.get_mut(request_id) {
        if !active.cancelled.load(Ordering::Acquire) {
            // The checked file lock is held by the caller. From this point a
            // cancellation request reports that the operation crossed its
            // commit boundary instead of pretending it can roll it back.
            active.commit_started = true;
        }
    }
}

impl ActiveRun {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancellation_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }
}

#[derive(Clone)]
struct ConnectivityProof {
    request_id: String,
    model_id: String,
    provider_id: String,
    base: String,
    binding_digest: String,
    credential_digest: String,
    catalog_digest: String,
    epoch: u64,
    expires_at: u64,
}

#[derive(Default)]
struct ProofState {
    epoch: u64,
    reset_in_progress: bool,
    tickets: HashMap<String, ConnectivityProof>,
}

static PROOFS: OnceLock<Mutex<ProofState>> = OnceLock::new();

fn proofs() -> &'static Mutex<ProofState> {
    PROOFS.get_or_init(|| Mutex::new(ProofState::default()))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn catalog_digest(models: &[GatewayProbeModel]) -> String {
    let mut models = models.to_vec();
    models.sort_by(|left, right| {
        (&left.id, &left.owned_by, left.selectable).cmp(&(
            &right.id,
            &right.owned_by,
            right.selectable,
        ))
    });
    let canonical = serde_json::to_vec(&models).unwrap_or_default();
    digest_hex(&canonical)
}

pub(crate) fn binding_digest(binding: &codex_usb_license::UsbAccountBinding, base: &str) -> String {
    let identity = serde_json::to_vec(&(
        binding.ccswitch.provider_id.as_str(),
        base,
        binding.usb_asset_id.as_str(),
        binding.account_ref.as_str(),
        binding.ccswitch.credential_id.as_str(),
        binding.ccswitch.group_id,
        binding.sub2api_user_id,
    ))
    .unwrap_or_default();
    digest_hex(&identity)
}

fn proof_epoch_snapshot() -> Result<u64, AppError> {
    let state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    if state.reset_in_progress {
        return Err(engine("模型目录正在复位；请稍后重新测试"));
    }
    Ok(state.epoch)
}

pub(crate) fn begin_catalog_reset() {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    state.epoch = state.epoch.wrapping_add(1);
    state.reset_in_progress = true;
    state.tickets.clear();
}

pub(crate) fn finish_catalog_reset() {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    state.reset_in_progress = false;
}

fn revoke_request_proofs(request_id: &str) -> bool {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    let before = state.tickets.len();
    state
        .tickets
        .retain(|_, ticket| ticket.request_id != request_id);
    before != state.tickets.len()
}

#[allow(clippy::too_many_arguments)]
fn issue_proof(
    request_id: &str,
    model_id: &str,
    provider_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    catalog_digest: String,
    expected_epoch: u64,
    cancelled: &AtomicBool,
) -> Result<(String, u64), AppError> {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    state
        .tickets
        .retain(|_, ticket| ticket.expires_at > unix_seconds());
    if state.reset_in_progress || state.epoch != expected_epoch || cancelled.load(Ordering::Acquire)
    {
        return Err(engine("目录状态已变化或请求已取消；请重新测试"));
    }
    if state.tickets.len() >= PROOF_LIMIT {
        return Err(engine("待确认的测试票据已满；请关闭并重新打开模型管理"));
    }
    let id = format!("proof-{}", uuid::Uuid::new_v4().simple());
    let expires_at = unix_seconds().saturating_add(PROOF_TTL.as_secs());
    state.tickets.insert(
        id.clone(),
        ConnectivityProof {
            request_id: request_id.to_owned(),
            model_id: model_id.to_owned(),
            provider_id: provider_id.to_owned(),
            base: base.to_owned(),
            binding_digest: binding_digest(binding, base),
            credential_digest: digest_hex(api_key.as_bytes()),
            catalog_digest,
            epoch: expected_epoch,
            expires_at,
        },
    );
    Ok((id, expires_at))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn consume_connectivity_proof(
    proof_id: &str,
    request_id: &str,
    model_id: &str,
    provider_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    catalog_digest: &str,
) -> Result<(), AppError> {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    let Some(ticket) = state.tickets.remove(proof_id) else {
        return Err(engine("测试凭证已失效、已使用或需要重新测试"));
    };
    let valid = !state.reset_in_progress
        && ticket.expires_at > unix_seconds()
        && ticket.epoch == state.epoch
        && ticket.request_id != request_id
        && ticket.model_id == model_id
        && ticket.provider_id == provider_id
        && ticket.base == base
        && ticket.binding_digest == binding_digest(binding, base)
        && ticket.credential_digest == digest_hex(api_key.as_bytes())
        && ticket.catalog_digest == catalog_digest;
    if !valid {
        return Err(engine("上游目录、账户或测试状态已变化；请重新测试"));
    }
    Ok(())
}

pub fn cancel(request_id: &str) -> bool {
    if !safe_identifier(request_id) {
        return false;
    }
    // Linearize cancellation with catalog commits; once a commit owns this
    // lock, cancel cannot claim it rolled the write back.
    let _files = crate::app::model_catalog::FILE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let active_or_pending = {
        let mut runs = active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(active) = runs.active.get(request_id) {
            if active.commit_started {
                false
            } else {
                active.cancelled.store(true, Ordering::Release);
                true
            }
        } else if runs.finished.iter().any(|id| id == request_id) {
            false
        } else if runs
            .cancelled_before_start
            .iter()
            .any(|id| id == request_id)
        {
            true
        } else if runs.cancelled_before_start.len() < RUN_HISTORY_LIMIT {
            runs.cancelled_before_start.push_back(request_id.to_owned());
            true
        } else {
            false
        }
    };
    let revoked = revoke_request_proofs(request_id);
    active_or_pending || revoked
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
    if request
        .connectivity_model_id
        .as_deref()
        .is_some_and(|id| !safe_identifier(id))
    {
        return Err(engine("连通性测试的模型标识无效"));
    }
    if request.connectivity_model_id.is_some() && !request.selected_model_ids.is_empty() {
        return Err(engine("单模型连通性测试不能与完整协议矩阵同时运行"));
    }
    if request.connectivity_model_id.is_some() && request.cost_consent {
        return Err(engine("单模型连通性测试不能与完整协议矩阵费用授权混用"));
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
    let active = begin_management_run(&request.request_id)?;
    let proof_epoch = if request.connectivity_model_id.is_some() {
        Some(proof_epoch_snapshot()?)
    } else {
        None
    };
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
        connection_fingerprint: binding_digest(&binding, &base),
        checked_at: unix_seconds(),
        models,
        results: Vec::new(),
        generated_requests: 0,
        error_code: None,
        connectivity_model_id: None,
        connectivity_proof_id: None,
        connectivity_proof_expires_at: None,
    };

    if let Some(model_id) = request.connectivity_model_id.as_deref() {
        resolve_selected_models(&report.models, &[model_id.to_owned()])?;
        if active.is_cancelled() {
            report.status = "cancelled".into();
            report.error_code = Some(ProbeError::Cancelled.code().into());
            return Ok(report);
        }
        let client = ProbeClient::new(
            &base,
            api_key,
            active.cancellation_flag(),
            Instant::now() + REFRESH_BUDGET,
        )
        .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
        .with_run_id(&request.request_id);
        let token = format!("codexcheck{}", uuid::Uuid::new_v4().simple());
        let result = client.quick_text(model_id, &token).await;
        report.generated_requests = client.requests_started();
        let passed = result.is_ok() && !active.is_cancelled();
        let error_code = result.err().map(|error| error.code().to_owned());
        report.connectivity_model_id = Some(model_id.to_owned());
        report.results.push(GatewayProbeModelResult {
            model_id: model_id.to_owned(),
            passed,
            generated_requests: report.generated_requests,
            checks: vec![GatewayProbeCheck {
                name: "text",
                passed,
            }],
            error_code: error_code.clone(),
        });
        if !passed {
            report.status = if active.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            report.error_code =
                Some(error_code.unwrap_or_else(|| ProbeError::Cancelled.code().to_owned()));
            return Ok(report);
        }
        let (proof_id, proof_expires_at) = issue_proof(
            &request.request_id,
            model_id,
            &binding.ccswitch.provider_id,
            &base,
            &binding,
            api_key,
            catalog_digest(&report.models),
            proof_epoch.expect("quick test captured a proof epoch"),
            &active.cancelled,
        )?;
        report.status = "passed".into();
        report.connectivity_proof_id = Some(proof_id);
        report.connectivity_proof_expires_at = Some(proof_expires_at);
        return Ok(report);
    }

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

pub(crate) async fn add_tested_model(
    request_id: &str,
    model_id: &str,
    proof_id: &str,
) -> Result<crate::app::model_catalog::NativeModelCatalogActionReport, AppError> {
    if !safe_identifier(request_id) || !safe_identifier(model_id) || !safe_identifier(proof_id) {
        return Err(engine("模型添加请求无效；请重新测试"));
    }
    let active = begin_management_run(request_id)?;
    let binding = usb_guard::authorized_account_binding()?
        .ok_or_else(|| engine("未取得有效的签名 U 盘账号绑定"))?;
    if binding.ccswitch.provider_id != TARGET_PROVIDER_ID {
        return Err(engine("当前 U 盘绑定不是 404token；未写入模型目录"));
    }
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        return Err(engine("签名 U 盘绑定缺少设备凭证"));
    }
    let base = crate::app::model_catalog::provider_base(&binding.ccswitch.provider_id)?;
    let client = ProbeClient::new(
        &base,
        api_key,
        active.cancellation_flag(),
        Instant::now() + REFRESH_BUDGET,
    )
    .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
    .with_run_id(request_id);
    let payload = client
        .models()
        .await
        .map_err(|error| engine(format!("添加前无法重新确认模型目录：{}", error.code())))?;
    let models = parse_model_catalog(&payload, api_key)
        .map_err(|error| engine(format!("添加前模型目录无效：{}", error.code())))?;
    resolve_selected_models(&models, &[model_id.to_owned()])?;
    if active.is_cancelled() {
        return Err(engine("模型添加已取消；未修改 Codex 目录"));
    }
    consume_connectivity_proof(
        proof_id,
        request_id,
        model_id,
        &binding.ccswitch.provider_id,
        &base,
        &binding,
        api_key,
        &catalog_digest(&models),
    )?;
    crate::app::model_catalog::add_tested_model_to_catalog(
        request_id,
        model_id,
        models
            .iter()
            .find(|candidate| candidate.id == model_id)
            .and_then(|candidate| candidate.owned_by.as_deref()),
        &binding,
        &active.cancelled,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    static PROOF_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_binding() -> codex_usb_license::UsbAccountBinding {
        codex_usb_license::UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "usb-fixture".into(),
            account_ref: "account-fixture".into(),
            account_name: "Fixture".into(),
            login_email: "fixture@example.test".into(),
            sub2api_user_id: 42,
            ccswitch: codex_usb_license::UsbCcswitchBinding {
                provider_id: TARGET_PROVIDER_ID.into(),
                credential_id: "credential-fixture".into(),
                device_api_key: "fixture-key-never-serialized".into(),
                group_id: 7,
            },
        }
    }

    fn request(selected_model_ids: Vec<String>, cost_consent: bool) -> GatewayModelProbeRequest {
        GatewayModelProbeRequest {
            request_id: "gateway-probe-fixture-1".into(),
            selected_model_ids,
            cost_consent,
            connectivity_model_id: None,
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

    #[test]
    fn quick_test_mode_is_single_model_and_separate_from_matrix_consent() {
        let mut quick = request(Vec::new(), false);
        quick.connectivity_model_id = Some("vendor/model-latest".into());
        assert!(validate_request(&quick).is_ok());

        quick.selected_model_ids = vec!["vendor/model-latest".into()];
        assert!(validate_request(&quick).is_err());
        quick.selected_model_ids.clear();
        quick.cost_consent = true;
        assert!(validate_request(&quick).is_err());

        quick.cost_consent = false;
        quick.connectivity_model_id = Some("sk-secret".into());
        assert!(validate_request(&quick).is_err());
    }

    #[test]
    fn proof_is_bound_to_model_provider_account_credential_catalog_and_single_use() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let models = parse_model_catalog(
            &serde_json::json!({"data":[{"id":"vendor/model-latest","owned_by":"fixture"}]}),
            "fixture-key-never-serialized",
        )
        .unwrap();
        let catalog = catalog_digest(&models);
        let cancelled = AtomicBool::new(false);
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 123;
            state.epoch
        };
        let (proof_id, expires_at) = issue_proof(
            "quick-test-fixture",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            "fixture-key-never-serialized",
            catalog.clone(),
            epoch,
            &cancelled,
        )
        .unwrap();
        assert!(expires_at > unix_seconds());
        assert!(!proof_id.contains("fixture-key"));
        consume_connectivity_proof(
            &proof_id,
            "catalog-add-fixture",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            "fixture-key-never-serialized",
            &catalog,
        )
        .unwrap();
        assert!(consume_connectivity_proof(
            &proof_id,
            "catalog-add-again",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            "fixture-key-never-serialized",
            &catalog,
        )
        .is_err());
    }

    #[test]
    fn reset_epoch_prevents_late_quick_test_ticket_issuance() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let cancelled = AtomicBool::new(false);
        {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
        }
        let old_epoch = proof_epoch_snapshot().unwrap();
        begin_catalog_reset();
        let result = issue_proof(
            "late-quick-test",
            "vendor/model",
            TARGET_PROVIDER_ID,
            "https://404token.xyz/v1/",
            &binding,
            "fixture-key",
            "fixture-catalog-digest".into(),
            old_epoch,
            &cancelled,
        );
        finish_catalog_reset();
        assert!(result.is_err());
    }

    #[test]
    fn reset_revokes_existing_ticket_before_queued_add_can_consume_it() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let models = parse_model_catalog(
            &serde_json::json!({"data":[{"id":"vendor/model-latest","owned_by":"fixture"}]}),
            api_key,
        )
        .unwrap();
        let catalog = catalog_digest(&models);
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 456;
            state.epoch
        };
        let cancelled = AtomicBool::new(false);
        let (proof_id, _) = issue_proof(
            "quick-test-before-reset",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            catalog.clone(),
            epoch,
            &cancelled,
        )
        .unwrap();

        begin_catalog_reset();
        let consume = consume_connectivity_proof(
            &proof_id,
            "queued-add-after-reset",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            &catalog,
        );
        finish_catalog_reset();

        assert!(consume.is_err());
        assert!(proofs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tickets
            .is_empty());
    }
}
