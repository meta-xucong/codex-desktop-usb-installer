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
const MODEL_CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const MODEL_CATALOG_MAX_ATTEMPTS: u32 = 3;
const PROOF_TTL: Duration = Duration::from_secs(20 * 60);
const PROOF_LIMIT: usize = 128;
const RUN_HISTORY_LIMIT: usize = 256;
const CODEX_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

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
    #[serde(default)]
    pub reasoning_model_id: Option<String>,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning_cost_consent: bool,
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
pub struct GatewayReasoningEffortResult {
    pub effort: String,
    /// accepted_by_route, rejected_by_route, or unknown
    pub status: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_results: Option<Vec<GatewayReasoningEffortResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_proof_id: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagementRunStartError {
    InvalidRequest,
    DuplicateRequest,
    CancelledBeforeStart,
    Busy,
}

fn begin_management_run_detailed(request_id: &str) -> Result<ActiveRun, ManagementRunStartError> {
    if !safe_identifier(request_id) {
        return Err(ManagementRunStartError::InvalidRequest);
    }
    let mut runs = active_runs()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if runs.finished.iter().any(|id| id == request_id) {
        return Err(ManagementRunStartError::DuplicateRequest);
    }
    if let Some(position) = runs
        .cancelled_before_start
        .iter()
        .position(|id| id == request_id)
    {
        runs.cancelled_before_start.remove(position);
        remember_finished(&mut runs, request_id);
        return Err(ManagementRunStartError::CancelledBeforeStart);
    }
    if !runs.active.is_empty() {
        return Err(ManagementRunStartError::Busy);
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

pub(crate) fn begin_management_run(request_id: &str) -> Result<ActiveRun, AppError> {
    begin_management_run_detailed(request_id).map_err(|error| match error {
        ManagementRunStartError::InvalidRequest => engine("探测请求标识无效"),
        ManagementRunStartError::DuplicateRequest => engine("该请求标识已使用；请重新发起操作"),
        ManagementRunStartError::CancelledBeforeStart => engine("请求已取消；未发送模型请求"),
        ManagementRunStartError::Busy => AppError::Busy("已有 404token 探测正在运行".into()),
    })
}

#[cfg(test)]
fn mock_add_commit_boundary(
    active: &ActiveRun,
    proof_id: &str,
    model_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    digest: &str,
    writes: &std::sync::atomic::AtomicUsize,
) -> bool {
    let _files = crate::app::model_catalog::FILE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if active.is_cancelled() {
        return false;
    }
    if consume_connectivity_proof(
        proof_id,
        &active.request_id,
        model_id,
        TARGET_PROVIDER_ID,
        base,
        binding,
        api_key,
        Some(digest),
    )
    .is_err()
    {
        return false;
    }
    mark_catalog_commit_started(&active.request_id);
    writes.fetch_add(1, Ordering::AcqRel);
    true
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
    owned_by: Option<String>,
    epoch: u64,
    expires_at: u64,
    reasoning: Option<ReasoningEvidence>,
}

#[derive(Clone)]
pub(crate) struct ReasoningEvidence {
    pub efforts: Vec<String>,
    pub default_effort: String,
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

fn revoke_proof(proof_id: &str) -> bool {
    proofs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .tickets
        .remove(proof_id)
        .is_some()
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
    reasoning: Option<ReasoningEvidence>,
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
            owned_by: None,
            epoch: expected_epoch,
            expires_at,
            reasoning,
        },
    );
    Ok((id, expires_at))
}

fn set_connectivity_proof_owner(proof_id: &str, owned_by: Option<String>) -> Result<(), AppError> {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    let ticket = state
        .tickets
        .get_mut(proof_id)
        .ok_or_else(|| engine("测试凭证已失效；请重新检测模型"))?;
    ticket.owned_by = owned_by;
    Ok(())
}

fn connectivity_proof_owner(proof_id: &str) -> Option<String> {
    proofs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .tickets
        .get(proof_id)
        .and_then(|ticket| ticket.owned_by.clone())
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
    catalog_digest: Option<&str>,
) -> Result<Option<ReasoningEvidence>, AppError> {
    let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    let Some(ticket) = state.tickets.get(proof_id).cloned() else {
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
        && catalog_digest.map_or(true, |digest| ticket.catalog_digest == digest);
    if !valid {
        return Err(engine("账户绑定或测试凭证已变化；请重新测试"));
    }
    state.tickets.remove(proof_id);
    Ok(ticket.reasoning)
}

/// Validate an Add proof without consuming it. The proof is consumed only by
/// the catalog transaction immediately before its first atomic write.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_connectivity_proof(
    proof_id: &str,
    request_id: &str,
    model_id: &str,
    provider_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    catalog_digest: Option<&str>,
    minimum_remaining: Duration,
) -> Result<ReasoningEvidence, ProofValidationError> {
    let state = proofs().lock().unwrap_or_else(|error| error.into_inner());
    let Some(ticket) = state.tickets.get(proof_id) else {
        return Err(ProofValidationError::Invalid);
    };
    let valid_binding = !state.reset_in_progress
        && ticket.epoch == state.epoch
        && ticket.request_id != request_id
        && ticket.model_id == model_id
        && ticket.provider_id == provider_id
        && ticket.base == base
        && ticket.binding_digest == binding_digest(binding, base)
        && ticket.credential_digest == digest_hex(api_key.as_bytes());
    if !valid_binding {
        return Err(ProofValidationError::Invalid);
    }
    let remaining = ticket.expires_at.saturating_sub(unix_seconds());
    if remaining == 0 || remaining < minimum_remaining.as_secs() {
        return Err(ProofValidationError::Expired);
    }
    if catalog_digest.is_some_and(|digest| ticket.catalog_digest != digest) {
        return Err(ProofValidationError::BindingChanged);
    }
    ticket
        .reasoning
        .clone()
        .ok_or(ProofValidationError::Invalid)
}

fn retryable_model_catalog_error(error: ProbeError) -> bool {
    matches!(
        error,
        ProbeError::Connect | ProbeError::Network | ProbeError::Timeout | ProbeError::Http
    )
}

fn model_catalog_retry_delay(attempt: u32, error: ProbeError) -> Option<Duration> {
    (attempt < MODEL_CATALOG_MAX_ATTEMPTS && retryable_model_catalog_error(error))
        .then(|| Duration::from_millis(250 * u64::from(attempt)))
}

/// `/models` is an idempotent GET. Retry brief transport/server failures only;
/// never repeat a generation request or hide auth/input/response errors.
async fn fetch_model_catalog_with_retry(client: &ProbeClient<'_>) -> Result<Value, ProbeError> {
    for attempt in 1..=MODEL_CATALOG_MAX_ATTEMPTS {
        match client.models().await {
            Ok(payload) => return Ok(payload),
            Err(error) => {
                let Some(delay) = model_catalog_retry_delay(attempt, error) else {
                    return Err(error);
                };
                log::warn!(
                    "codex_gateway_catalog_retry attempt={} code={}",
                    attempt,
                    error.code()
                );
                client.check()?;
                tokio::time::sleep(delay).await;
                client.check()?;
            }
        }
    }
    Err(ProbeError::BudgetExhausted)
}

fn request_needs_model_catalog(request: &GatewayModelProbeRequest) -> bool {
    request.reasoning_model_id.is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProofValidationError {
    Invalid,
    Expired,
    BindingChanged,
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
    let reasoning_mode =
        request.reasoning_model_id.is_some() || !request.reasoning_efforts.is_empty();
    if reasoning_mode {
        let id = request
            .reasoning_model_id
            .as_deref()
            .ok_or_else(|| engine("思考档位测试缺少模型"))?;
        if !safe_identifier(id)
            || request.connectivity_model_id.is_some()
            || !request.selected_model_ids.is_empty()
        {
            return Err(engine("思考档位测试请求无效"));
        }
        if !request.reasoning_cost_consent {
            return Err(engine("思考档位测试需要单独确认可能产生的费用"));
        }
        if request.cost_consent {
            return Err(engine("思考档位测试费用确认不能与完整协议矩阵授权混用"));
        }
        if request.reasoning_efforts.is_empty()
            || request.reasoning_efforts.len() > CODEX_EFFORTS.len()
        {
            return Err(engine("请明确选择要检测的思考档位"));
        }
        let allowed = reasoning_candidates(id);
        let mut unique = BTreeSet::new();
        if request
            .reasoning_efforts
            .iter()
            .any(|effort| !allowed.contains(&effort.as_str()) || !unique.insert(effort))
        {
            return Err(engine("思考档位无效、重复或不适用于该精确型号"));
        }
        if canonical_reasoning_efforts(id, &request.reasoning_efforts) != request.reasoning_efforts
        {
            return Err(engine("思考档位必须按 Codex 顺序提交"));
        }
    } else if request.reasoning_cost_consent {
        return Err(engine("没有思考档位测试时不能提交费用确认"));
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

fn reasoning_candidates(model_id: &str) -> Vec<&'static str> {
    let normalized = model_id.to_ascii_lowercase();
    let omit_none = normalized == "claude-opus-5-5" || normalized.ends_with("/claude-opus-5-5");
    CODEX_EFFORTS
        .iter()
        .copied()
        .filter(|effort| !omit_none || *effort != "none")
        .collect()
}

pub(crate) fn canonical_reasoning_efforts(model_id: &str, selected: &[String]) -> Vec<String> {
    let selected = selected.iter().map(String::as_str).collect::<BTreeSet<_>>();
    reasoning_candidates(model_id)
        .into_iter()
        .filter(|effort| selected.contains(effort))
        .map(str::to_owned)
        .collect()
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
    let proof_epoch =
        if request.connectivity_model_id.is_some() || request.reasoning_model_id.is_some() {
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
    let catalog_client = ProbeClient::with_timeouts(
        &base,
        api_key,
        active.cancelled.clone(),
        Instant::now() + REFRESH_BUDGET,
        crate::app::model_probe::ProbeTimeouts::new(
            MODEL_CATALOG_TIMEOUT,
            crate::app::model_probe::GENERATION_REQUEST_TIMEOUT,
        ),
    )
    .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
    .with_run_id(&request.request_id);

    // Discovery and bulk checks need a fresh model catalog. A single-model
    // reasoning probe already carries the selected ID from discovery; the
    // exact generation request is the connectivity check, so don't gate it on
    // a second `/models` round trip.
    let models = if !request_needs_model_catalog(&request) {
        Vec::new()
    } else {
        let catalog_payload = fetch_model_catalog_with_retry(&catalog_client)
            .await
            .map_err(|error| engine(format!("404token 模型目录请求失败：{}", error.code())))?;
        parse_model_catalog(&catalog_payload, api_key)
            .map_err(|error| engine(format!("404token 模型目录无效：{}", error.code())))?
    };
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
        reasoning_model_id: None,
        reasoning_results: None,
        reasoning_proof_id: None,
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
            None,
        )?;
        let owner = report
            .models
            .iter()
            .find(|candidate| candidate.id == model_id)
            .and_then(|candidate| candidate.owned_by.clone());
        set_connectivity_proof_owner(&proof_id, owner)?;
        report.status = "passed".into();
        report.connectivity_proof_id = Some(proof_id);
        report.connectivity_proof_expires_at = Some(proof_expires_at);
        return Ok(report);
    }

    if let Some(model_id) = request.reasoning_model_id.as_deref() {
        let client = ProbeClient::new(
            &base,
            api_key,
            active.cancellation_flag(),
            Instant::now() + MODEL_MATRIX_BUDGET,
        )
        .map_err(|_| engine("无法初始化安全的 404token 探测客户端"))?
        .with_run_id(&request.request_id);
        // Probe the complete bounded candidate set. A successful level must
        // not hide other levels that the route may also accept.
        let efforts = request.reasoning_efforts.clone();
        let mut results = Vec::with_capacity(efforts.len());
        let mut terminal_error = None;
        for effort in &efforts {
            if active.is_cancelled() {
                report.status = "cancelled".into();
                report.error_code = Some(ProbeError::Cancelled.code().into());
                break;
            }
            let token = format!("codexeffort{}", uuid::Uuid::new_v4().simple());
            match client
                .quick_text_with_effort(model_id, &token, effort)
                .await
            {
                Ok(()) => {
                    results.push(GatewayReasoningEffortResult {
                        effort: effort.clone(),
                        status: "accepted_by_route".into(),
                        error_code: None,
                    });
                }
                Err(error) => {
                    let unsupported = error == ProbeError::UnsupportedEffort;
                    results.push(GatewayReasoningEffortResult {
                        effort: effort.clone(),
                        status: if unsupported {
                            "rejected_by_route"
                        } else {
                            "unknown"
                        }
                        .into(),
                        error_code: Some(error.code().into()),
                    });
                    if !unsupported {
                        terminal_error = Some(error);
                        break;
                    }
                }
            }
        }
        report.generated_requests = client.requests_started();
        report.reasoning_model_id = Some(model_id.to_owned());
        let accepted = accepted_reasoning_efforts(&results);
        report.reasoning_results = Some(results);
        if !accepted.is_empty() && !active.is_cancelled() {
            let default_effort = accepted
                .iter()
                .find(|effort| effort.as_str() == "medium")
                .or_else(|| accepted.iter().find(|effort| effort.as_str() == "low"))
                .cloned()
                .unwrap_or_else(|| accepted[0].clone());
            let (proof_id, proof_expires_at) = issue_proof(
                &request.request_id,
                model_id,
                &binding.ccswitch.provider_id,
                &base,
                &binding,
                api_key,
                catalog_digest(&report.models),
                proof_epoch.expect("reasoning probe captured proof epoch"),
                &active.cancelled,
                Some(ReasoningEvidence {
                    efforts: accepted,
                    default_effort,
                }),
            )?;
            let owner = report
                .models
                .iter()
                .find(|candidate| candidate.id == model_id)
                .and_then(|candidate| candidate.owned_by.clone());
            set_connectivity_proof_owner(&proof_id, owner)?;
            report.status = "passed".into();
            report.connectivity_model_id = Some(model_id.to_owned());
            report.connectivity_proof_id = Some(proof_id.clone());
            report.connectivity_proof_expires_at = Some(proof_expires_at);
            report.reasoning_proof_id = Some(proof_id);
        } else if report.status != "cancelled" {
            report.status = "failed".into();
            report.error_code = Some(
                terminal_error
                    .map(|error| error.code())
                    .unwrap_or("no-effort-accepted")
                    .into(),
            );
        }
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
    let binding = match usb_guard::authorized_account_binding() {
        Ok(Some(binding)) => binding,
        _ => {
            let _ = revoke_proof(proof_id);
            return Ok(add_failure(
                "proof_invalid",
                "binding-changed",
                "未取得有效的签名 U 盘账号绑定，请重新检测",
            ));
        }
    };
    if binding.ccswitch.provider_id != TARGET_PROVIDER_ID {
        let _ = revoke_proof(proof_id);
        return Ok(add_failure(
            "proof_invalid",
            "binding-changed",
            "当前签名 U 盘账号已变化，请重新检测",
        ));
    }
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        let _ = revoke_proof(proof_id);
        return Ok(add_failure(
            "proof_invalid",
            "binding-changed",
            "签名 U 盘授权信息已变化，请重新检测",
        ));
    }
    let base = match crate::app::model_catalog::provider_base(&binding.ccswitch.provider_id) {
        Ok(base) => base,
        Err(_) => {
            let _ = revoke_proof(proof_id);
            return Ok(add_failure(
                "proof_invalid",
                "binding-changed",
                "签名 U 盘服务设置已变化，请重新检测",
            ));
        }
    };
    let early_evidence = match validate_connectivity_proof(
        proof_id,
        request_id,
        model_id,
        &binding.ccswitch.provider_id,
        &base,
        &binding,
        api_key,
        None,
        Duration::ZERO,
    ) {
        Ok(evidence) => evidence,
        Err(ProofValidationError::Expired) => {
            let mut report =
                add_failure("proof_invalid", "proof-expired", "测试已过期，请重新检测");
            report.proof_state = "expired".into();
            let _ = revoke_proof(proof_id);
            return Ok(report);
        }
        Err(_) => {
            let _ = revoke_proof(proof_id);
            return Ok(add_failure(
                "proof_invalid",
                "proof-invalid",
                "测试凭证已失效，请重新检测",
            ));
        }
    };
    let owned_by = connectivity_proof_owner(proof_id);
    let active = match begin_management_run_detailed(request_id) {
        Ok(active) => active,
        Err(ManagementRunStartError::Busy) => {
            // A real catalogue operation is busy; retain the proof only while
            // its static binding and expiry remain valid.
            return Ok(pending_retryable_report(
                proof_id,
                request_id,
                model_id,
                &binding.ccswitch.provider_id,
                &base,
                &binding,
                api_key,
                "catalog-busy",
                "模型管理正在处理另一项操作；测试凭证保留，稍后可直接重试添加",
            ));
        }
        Err(error) => {
            let (code, message) = match error {
                ManagementRunStartError::CancelledBeforeStart => {
                    ("cancelled", "模型添加已取消；请重新检测")
                }
                ManagementRunStartError::DuplicateRequest => {
                    ("request-id-reused", "添加请求已使用；请重新检测后再试")
                }
                ManagementRunStartError::InvalidRequest => {
                    ("request-invalid", "添加请求无效；请重新检测")
                }
                ManagementRunStartError::Busy => unreachable!(),
            };
            let _ = revoke_proof(proof_id);
            let mut report = add_failure("proof_invalid", code, message);
            report.proof_state = "invalid".into();
            return Ok(report);
        }
    };
    if active.is_cancelled() {
        let _ = revoke_proof(proof_id);
        return Ok(add_failure(
            "proof_invalid",
            "cancelled",
            "模型添加已取消；请重新检测",
        ));
    }
    // The detected model and its reasoning evidence are covered by the
    // one-time proof. Add performs local compatibility and atomic-write checks
    // without refetching the upstream catalogue.
    let evidence = early_evidence;
    let proof_id = proof_id.to_owned();
    let proof_id_for_consume = proof_id.clone();
    let request_id_for_consume = request_id.to_owned();
    let model_id_for_consume = model_id.to_owned();
    let provider_id = binding.ccswitch.provider_id.clone();
    let base_for_consume = base.clone();
    let binding_for_consume = binding.clone();
    let api_key_for_consume = api_key.to_owned();
    let stage = std::sync::atomic::AtomicU8::new(0);
    let stage_ref = &stage;
    let added = crate::app::model_catalog::add_tested_model_to_catalog(
        request_id,
        model_id,
        owned_by.as_deref(),
        &active.cancelled,
        &evidence,
        move || {
            stage_ref.store(3, Ordering::Release);
            consume_connectivity_proof(
                &proof_id_for_consume,
                &request_id_for_consume,
                &model_id_for_consume,
                &provider_id,
                &base_for_consume,
                &binding_for_consume,
                &api_key_for_consume,
                None,
            )
            .map(|_| {
                stage_ref.store(4, Ordering::Release);
            })
        },
        &stage,
    );
    match added {
        Ok(mut report) => {
            report.commit_state = match report.status.as_str() {
                "cleanup_pending" | "outcome_unconfirmed" => "uncertain",
                "applied_restart_pending" => "started",
                _ => "not_started",
            }
            .into();
            report.proof_state = "consumed".into();
            report.retryable = false;
            Ok(report)
        }
        Err(error) => {
            let phase = stage.load(Ordering::Acquire);
            if phase == 3 {
                let mut report = add_failure(
                    "proof_invalid",
                    "proof-invalid",
                    "测试凭证已失效或已使用，请重新检测",
                );
                report.proof_state = "invalid".into();
                let _ = revoke_proof(&proof_id);
                let _ = error;
                return Ok(report);
            }
            if phase == 5 {
                let _ = revoke_proof(&proof_id);
                return Ok(add_failure(
                    "proof_invalid",
                    "binding-changed",
                    "签名 U 盘账号或配置已变化，请重新检测",
                ));
            }
            if phase == 0 && !matches!(&error, AppError::Busy(_)) {
                let _ = revoke_proof(&proof_id);
                let mut report = add_failure(
                    "proof_invalid",
                    "catalog-precommit-rejected",
                    "本地模型目录校验未通过；请重新检测后再试",
                );
                report.proof_state = "invalid".into();
                return Ok(report);
            }
            if phase == 4 {
                let _ = error;
                let mut report = add_failure(
                    "commit_started",
                    "catalog-write-failed",
                    "模型目录写入已开始但未能确认完成；请刷新状态后再操作",
                );
                report.commit_state = "uncertain".into();
                report.proof_state = "consumed".into();
                return Ok(report);
            }
            let (code, message) = precommit_failure_details(phase, &error.to_string());
            match validate_connectivity_proof(
                &proof_id,
                request_id,
                model_id,
                &binding.ccswitch.provider_id,
                &base,
                &binding,
                api_key,
                None,
                Duration::ZERO,
            ) {
                Ok(_) => {
                    log::warn!("model add precommit failed: request_id={request_id} phase={phase} code={code}");
                    let mut report = add_failure("precommit_retryable", code, message);
                    report.proof_state = "valid".into();
                    report.retryable = true;
                    Ok(report)
                }
                Err(ProofValidationError::Expired) => {
                    let _ = revoke_proof(&proof_id);
                    let mut report =
                        add_failure("proof_invalid", "proof-expired", "测试已过期，请重新检测");
                    report.proof_state = "expired".into();
                    Ok(report)
                }
                Err(_) => {
                    let _ = revoke_proof(&proof_id);
                    Ok(add_failure(
                        "proof_invalid",
                        "proof-invalid",
                        "测试凭证已失效，请重新检测",
                    ))
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn pending_retryable_report(
    proof_id: &str,
    request_id: &str,
    model_id: &str,
    provider_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    error_code: &str,
    message: &str,
) -> crate::app::model_catalog::NativeModelCatalogActionReport {
    match validate_connectivity_proof(
        proof_id,
        request_id,
        model_id,
        provider_id,
        base,
        binding,
        api_key,
        None,
        Duration::ZERO,
    ) {
        Ok(_) => {
            let mut report = add_failure("precommit_retryable", error_code, message);
            report.proof_state = "valid".into();
            report.retryable = true;
            report
        }
        Err(ProofValidationError::Expired) => {
            let _ = revoke_proof(proof_id);
            let mut report =
                add_failure("proof_invalid", "proof-expired", "测试已过期，请重新检测");
            report.proof_state = "expired".into();
            report
        }
        Err(_) => {
            let _ = revoke_proof(proof_id);
            add_failure(
                "proof_invalid",
                "proof-invalid",
                "测试凭证已失效，请重新检测",
            )
        }
    }
}

#[cfg(test)]
fn revalidate_connectivity_proof(
    proof_id: &str,
    request_id: &str,
    model_id: &str,
    provider_id: &str,
    base: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    api_key: &str,
    catalog_digest: &str,
) -> Result<ReasoningEvidence, ProofValidationError> {
    let result = validate_connectivity_proof(
        proof_id,
        request_id,
        model_id,
        provider_id,
        base,
        binding,
        api_key,
        Some(catalog_digest),
        Duration::ZERO,
    );
    if result.is_err() {
        let _ = revoke_proof(proof_id);
    }
    result
}
fn accepted_reasoning_efforts(results: &[GatewayReasoningEffortResult]) -> Vec<String> {
    results
        .iter()
        .filter(|result| result.status == "accepted_by_route")
        .map(|result| result.effort.clone())
        .collect()
}

fn add_failure(
    status: &str,
    error_code: &str,
    message: &str,
) -> crate::app::model_catalog::NativeModelCatalogActionReport {
    crate::app::model_catalog::NativeModelCatalogActionReport {
        status: status.into(),
        model_ids: Vec::new(),
        restart_required: false,
        message: message.into(),
        error_code: Some(error_code.into()),
        commit_state: "not_started".into(),
        proof_state: "invalid".into(),
        retryable: status == "precommit_retryable",
        connection_fingerprint: None,
        credential_fingerprint: None,
    }
}

fn precommit_failure_details(phase: u8, error: &str) -> (&'static str, &'static str) {
    if phase == 2 && error.contains("native-probe-timeout") {
        return (
            "native-probe-timeout",
            "本地正在核对随包 Codex，耗时超过本次校验时限；未写入模型，可直接重试添加",
        );
    }
    match phase {
        0 => (
            "catalog-busy",
            "另一个本地配置操作刚完成；未写入模型，可使用同一测试结果重试",
        ),
        1 => (
            "native-defaults-rejected",
            "读取随包 Codex 默认模型清单失败；未写入模型，可重试添加",
        ),
        2 => (
            "native-cli-rejected",
            "随包 Codex 未通过模型目录校验；未写入模型，可重试添加",
        ),
        _ => (
            "catalog-precommit-failed",
            "添加前的本地校验未通过；未写入模型，可重试添加",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_add_has_no_extra_proof_remaining_budget() {
        assert_eq!(PROOF_TTL, Duration::from_secs(1200));
        assert_eq!(MODEL_CATALOG_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn model_catalog_retry_is_limited_to_transient_safe_get_failures() {
        assert!(retryable_model_catalog_error(ProbeError::Connect));
        assert!(retryable_model_catalog_error(ProbeError::Network));
        assert!(retryable_model_catalog_error(ProbeError::Timeout));
        assert!(retryable_model_catalog_error(ProbeError::Http));
        assert!(!retryable_model_catalog_error(ProbeError::Unauthorized));
        assert!(!retryable_model_catalog_error(ProbeError::Http400));
        assert!(!retryable_model_catalog_error(ProbeError::InvalidResponse));
        assert!(!retryable_model_catalog_error(ProbeError::Cancelled));
        assert_eq!(MODEL_CATALOG_MAX_ATTEMPTS, 3);
        assert_eq!(
            model_catalog_retry_delay(1, ProbeError::Connect),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            model_catalog_retry_delay(2, ProbeError::Timeout),
            Some(Duration::from_millis(500))
        );
        assert_eq!(model_catalog_retry_delay(3, ProbeError::Connect), None);
        assert_eq!(model_catalog_retry_delay(1, ProbeError::Unauthorized), None);
    }

    #[tokio::test]
    async fn model_catalog_get_retries_after_transient_server_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut statuses = Vec::new();
            for (status, body) in [
                ("503 Service Unavailable", "".to_owned()),
                ("200 OK", r#"{"data":[{"id":"vendor/model"}]}"#.to_owned()),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                statuses.push(String::from_utf8_lossy(&request).to_string());
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body.as_bytes()).unwrap();
            }
            statuses
        });

        let client = ProbeClient::with_timeouts(
            &format!("http://{address}/v1/"),
            "fixture-key",
            Arc::new(AtomicBool::new(false)),
            Instant::now() + Duration::from_secs(5),
            crate::app::model_probe::ProbeTimeouts::new(
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
        )
        .unwrap();
        let payload = fetch_model_catalog_with_retry(&client).await.unwrap();
        assert_eq!(payload["data"][0]["id"], "vendor/model");
        assert_eq!(client.requests_started(), 2);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| request.starts_with("GET /v1/models HTTP/1.1")));
    }

    #[test]
    fn native_catalog_timeout_is_reported_as_retryable_local_validation() {
        assert_eq!(
            precommit_failure_details(2, "native-probe-timeout"),
            (
                "native-probe-timeout",
                "本地正在核对随包 Codex，耗时超过本次校验时限；未写入模型，可直接重试添加",
            )
        );
        assert_eq!(
            precommit_failure_details(2, "native-catalog-unsupported"),
            (
                "native-cli-rejected",
                "随包 Codex 未通过模型目录校验；未写入模型，可重试添加",
            )
        );
    }
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
            reasoning_model_id: None,
            reasoning_efforts: Vec::new(),
            reasoning_cost_consent: false,
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
        assert!(request_needs_model_catalog(&quick));
        quick.connectivity_model_id = Some("vendor/model-latest".into());
        assert!(request_needs_model_catalog(&quick));
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
    fn reasoning_effort_probe_requires_explicit_cost_consent_and_exact_model_candidates() {
        let mut request = request(Vec::new(), false);
        request.reasoning_model_id = Some("claude-opus-5-5".into());
        assert!(!request_needs_model_catalog(&request));
        request.reasoning_efforts = vec!["medium".into(), "high".into()];
        assert!(validate_request(&request).is_err());
        request.reasoning_cost_consent = true;
        request.reasoning_efforts = vec!["medium".into(), "low".into()];
        assert!(validate_request(&request).is_err());
        request.reasoning_efforts = vec!["low".into(), "medium".into()];
        assert!(validate_request(&request).is_ok());
        request.reasoning_efforts = reasoning_candidates("claude-opus-5-5")
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(request.reasoning_efforts.len(), 6);
        assert!(validate_request(&request).is_ok());
        request.reasoning_efforts = vec!["low".into()];
        assert!(validate_request(&request).is_ok());
        request.reasoning_efforts = vec!["medium".into()];
        assert!(validate_request(&request).is_ok());
        request.reasoning_efforts = vec!["medium".into(), "low".into()];
        request.reasoning_efforts.push("none".into());
        assert!(validate_request(&request).is_err());
        request.reasoning_efforts = vec!["medium".into(), "medium".into()];
        assert!(validate_request(&request).is_err());
        request.reasoning_efforts = vec!["arbitrary".into()];
        assert!(validate_request(&request).is_err());
        assert!(!reasoning_candidates("claude-opus-5-5").contains(&"none"));
        assert!(reasoning_candidates("vendor/model-latest").contains(&"none"));
        request.reasoning_model_id = Some("vendor/model-latest".into());
        assert!(!request_needs_model_catalog(&request));
        request.reasoning_efforts = reasoning_candidates("vendor/model-latest")
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(request.reasoning_efforts.len(), 7);
        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn accepted_reasoning_subset_excludes_unknown_and_rejected_efforts() {
        let results = vec![
            GatewayReasoningEffortResult {
                effort: "medium".into(),
                status: "accepted_by_route".into(),
                error_code: None,
            },
            GatewayReasoningEffortResult {
                effort: "high".into(),
                status: "unknown".into(),
                error_code: Some("upstream-unavailable".into()),
            },
            GatewayReasoningEffortResult {
                effort: "low".into(),
                status: "rejected_by_route".into(),
                error_code: Some("unsupported-effort".into()),
            },
        ];
        assert_eq!(accepted_reasoning_efforts(&results), vec!["medium"]);
    }

    #[test]
    fn management_run_start_errors_distinguish_busy_cancelled_duplicate_and_invalid() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let busy_owner = begin_management_run("manager-start-busy-owner").unwrap();
        assert!(matches!(
            begin_management_run_detailed("manager-start-busy-request"),
            Err(ManagementRunStartError::Busy)
        ));
        drop(busy_owner);

        let duplicate = begin_management_run("manager-start-duplicate").unwrap();
        drop(duplicate);
        assert!(matches!(
            begin_management_run_detailed("manager-start-duplicate"),
            Err(ManagementRunStartError::DuplicateRequest)
        ));

        assert!(cancel("manager-start-cancelled"));
        assert!(matches!(
            begin_management_run_detailed("manager-start-cancelled"),
            Err(ManagementRunStartError::CancelledBeforeStart)
        ));
        assert!(matches!(
            begin_management_run_detailed("unsafe request id"),
            Err(ManagementRunStartError::InvalidRequest)
        ));
        assert!(!active_runs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .contains_key("manager-start-duplicate"));
    }

    #[test]
    fn reasoning_efforts_are_canonicalized_independently_of_request_order() {
        let reversed = vec!["high".into(), "medium".into(), "low".into()];
        assert_eq!(
            canonical_reasoning_efforts("vendor/model", &reversed),
            vec!["low", "medium", "high"]
        );
        let claude = vec!["high".into(), "low".into(), "medium".into()];
        assert_eq!(
            canonical_reasoning_efforts("claude-opus-5-5", &claude),
            vec!["low", "medium", "high"]
        );
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
            None,
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
            Some(&catalog),
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
            Some(&catalog),
        )
        .is_err());
    }

    #[test]
    fn add_preflight_validation_is_non_consuming_and_rejects_insufficient_ttl() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let cancelled = AtomicBool::new(false);
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 124;
            state.epoch
        };
        let reasoning = ReasoningEvidence {
            efforts: vec!["low".into(), "medium".into()],
            default_effort: "medium".into(),
        };
        let (proof_id, _) = issue_proof(
            "reasoning-run",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "fixture-catalog-digest".into(),
            epoch,
            &cancelled,
            Some(reasoning.clone()),
        )
        .unwrap();

        for _ in 0..2 {
            let validated = validate_connectivity_proof(
                &proof_id,
                "add-run",
                "vendor/model-latest",
                TARGET_PROVIDER_ID,
                base,
                &binding,
                api_key,
                None,
                Duration::ZERO,
            )
            .unwrap();
            assert_eq!(validated.efforts, reasoning.efforts);
        }
        assert!(proofs().lock().unwrap().tickets.contains_key(&proof_id));
        assert!(matches!(
            validate_connectivity_proof(
                &proof_id,
                "add-run",
                "vendor/model-latest",
                TARGET_PROVIDER_ID,
                base,
                &binding,
                api_key,
                None,
                PROOF_TTL + Duration::from_secs(1),
            ),
            Err(ProofValidationError::Expired)
        ));
        assert!(proofs().lock().unwrap().tickets.contains_key(&proof_id));
    }

    #[test]
    fn optional_catalog_snapshot_check_is_non_consuming_and_add_busy_is_retryable() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let cancelled = AtomicBool::new(false);
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 126;
            state.epoch
        };
        let reasoning = Some(ReasoningEvidence {
            efforts: vec!["medium".into()],
            default_effort: "medium".into(),
        });
        let (matching_proof, _) = issue_proof(
            "reasoning-run-revalidate-match",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "catalog-before-refresh".into(),
            epoch,
            &cancelled,
            reasoning.clone(),
        )
        .unwrap();
        // Add busy remains retryable without a second upstream read. This
        // test-only helper retains coverage for explicit snapshot comparisons.
        let pending = pending_retryable_report(
            &matching_proof,
            "add-run-revalidate",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "upstream-unavailable",
            "temporarily unavailable",
        );
        assert_eq!(pending.status, "precommit_retryable");
        assert_eq!(pending.proof_state, "valid");
        assert!(proofs()
            .lock()
            .unwrap()
            .tickets
            .contains_key(&matching_proof));
        revalidate_connectivity_proof(
            &matching_proof,
            "add-run-revalidate",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "catalog-before-refresh",
        )
        .unwrap();
        assert!(proofs()
            .lock()
            .unwrap()
            .tickets
            .contains_key(&matching_proof));

        let (changed_proof, _) = issue_proof(
            "reasoning-run-revalidate-changed",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "catalog-before-refresh".into(),
            epoch,
            &cancelled,
            reasoning,
        )
        .unwrap();
        assert!(matches!(
            revalidate_connectivity_proof(
                &changed_proof,
                "add-run-revalidate",
                "vendor/model-latest",
                TARGET_PROVIDER_ID,
                base,
                &binding,
                api_key,
                "catalog-after-change",
            ),
            Err(ProofValidationError::BindingChanged)
        ));
        assert!(!proofs()
            .lock()
            .unwrap()
            .tickets
            .contains_key(&changed_proof));
        assert!(proofs()
            .lock()
            .unwrap()
            .tickets
            .contains_key(&matching_proof));
    }

    #[test]
    fn expired_add_proof_is_revoked_before_any_mock_catalog_write() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 129;
            state.epoch
        };
        let cancelled = AtomicBool::new(false);
        let (proof_id, _) = issue_proof(
            "expired-add-origin",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "fixture-catalog-digest".into(),
            epoch,
            &cancelled,
            Some(ReasoningEvidence {
                efforts: vec!["medium".into()],
                default_effort: "medium".into(),
            }),
        )
        .unwrap();
        proofs()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tickets
            .get_mut(&proof_id)
            .unwrap()
            .expires_at = unix_seconds().saturating_sub(1);

        let report = pending_retryable_report(
            &proof_id,
            "expired-add-request",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "upstream-unavailable",
            "temporarily unavailable",
        );
        assert_eq!(report.status, "proof_invalid");
        assert_eq!(report.proof_state, "expired");
        assert!(!proofs().lock().unwrap().tickets.contains_key(&proof_id));

        let active = begin_management_run("expired-add-request").unwrap();
        let writes = std::sync::atomic::AtomicUsize::new(0);
        assert!(!mock_add_commit_boundary(
            &active,
            &proof_id,
            "vendor/model-latest",
            base,
            &binding,
            api_key,
            "fixture-catalog-digest",
            &writes,
        ));
        assert_eq!(writes.load(Ordering::Acquire), 0);
        drop(active);
    }
    #[test]
    fn only_one_concurrent_add_can_consume_a_shared_proof() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let cancelled = AtomicBool::new(false);
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 125;
            state.epoch
        };
        let (proof_id, _) = issue_proof(
            "reasoning-run-race",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "fixture-catalog-digest".into(),
            epoch,
            &cancelled,
            Some(ReasoningEvidence {
                efforts: vec!["medium".into()],
                default_effort: "medium".into(),
            }),
        )
        .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for suffix in ["a", "b"] {
            let barrier = barrier.clone();
            let binding = binding.clone();
            let proof_id = proof_id.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                consume_connectivity_proof(
                    &proof_id,
                    &format!("add-race-{suffix}"),
                    "vendor/model-latest",
                    TARGET_PROVIDER_ID,
                    base,
                    &binding,
                    api_key,
                    Some("fixture-catalog-digest"),
                )
                .is_ok()
            }));
        }
        barrier.wait();
        let successes = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
    }

    #[test]
    fn add_cancel_barrier_has_one_linearized_outcome_and_revokes_pending_proof() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 127;
            state.epoch
        };
        let cancelled_flag = AtomicBool::new(false);
        let origin_id = "reasoning-run-cancel-race";
        drop(begin_management_run(origin_id).unwrap());
        let (proof_id, _) = issue_proof(
            origin_id,
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "fixture-catalog-digest".into(),
            epoch,
            &cancelled_flag,
            Some(ReasoningEvidence {
                efforts: vec!["medium".into()],
                default_effort: "medium".into(),
            }),
        )
        .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let add_barrier = barrier.clone();
        let add_binding = binding.clone();
        let add_proof = proof_id.clone();
        let add_writes = writes.clone();
        let add = std::thread::spawn(move || {
            let active = begin_management_run("catalog-add-cancel-race").unwrap();
            add_barrier.wait();
            let committed = mock_add_commit_boundary(
                &active,
                &add_proof,
                "vendor/model-latest",
                base,
                &add_binding,
                api_key,
                "fixture-catalog-digest",
                &add_writes,
            );
            drop(active);
            committed
        });
        let cancel_barrier = barrier.clone();
        let cancel = std::thread::spawn(move || {
            cancel_barrier.wait();
            super::cancel(origin_id)
        });
        barrier.wait();
        let committed = add.join().unwrap();
        let cancellation_result = cancel.join().unwrap();
        let write_count = writes.load(Ordering::Acquire);
        assert_eq!(write_count, usize::from(committed));
        assert_eq!(cancellation_result, !committed);

        assert!(!proofs().lock().unwrap().tickets.contains_key(&proof_id));
        assert!(!active_runs()
            .lock()
            .unwrap()
            .active
            .contains_key("catalog-add-cancel-race"));
    }
    #[test]
    fn add_reset_barrier_never_leaves_a_consumable_old_epoch_proof() {
        let _serial = PROOF_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let binding = fixture_binding();
        let base = "https://404token.xyz/v1/";
        let api_key = "fixture-key-never-serialized";
        let epoch = {
            let mut state = proofs().lock().unwrap_or_else(|error| error.into_inner());
            *state = ProofState::default();
            state.epoch = 128;
            state.epoch
        };
        let cancelled_flag = AtomicBool::new(false);
        let origin_id = "reasoning-run-reset-race";
        drop(begin_management_run(origin_id).unwrap());
        let (proof_id, _) = issue_proof(
            origin_id,
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            "fixture-catalog-digest".into(),
            epoch,
            &cancelled_flag,
            Some(ReasoningEvidence {
                efforts: vec!["medium".into()],
                default_effort: "medium".into(),
            }),
        )
        .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let add_barrier = barrier.clone();
        let add_binding = binding.clone();
        let add_proof = proof_id.clone();
        let add_writes = writes.clone();
        let add = std::thread::spawn(move || {
            let active = begin_management_run("catalog-add-reset-race").unwrap();
            add_barrier.wait();
            let committed = mock_add_commit_boundary(
                &active,
                &add_proof,
                "vendor/model-latest",
                base,
                &add_binding,
                api_key,
                "fixture-catalog-digest",
                &add_writes,
            );
            drop(active);
            committed
        });
        let reset_barrier = barrier.clone();
        let reset = std::thread::spawn(move || {
            reset_barrier.wait();
            let _files = crate::app::model_catalog::FILE_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            begin_catalog_reset();
            finish_catalog_reset();
        });
        barrier.wait();
        let committed = add.join().unwrap();
        reset.join().unwrap();
        let write_count = writes.load(Ordering::Acquire);
        assert_eq!(write_count, usize::from(committed));
        assert!(!proofs().lock().unwrap().tickets.contains_key(&proof_id));
        assert!(!active_runs()
            .lock()
            .unwrap()
            .active
            .contains_key("catalog-add-reset-race"));
        assert!(validate_connectivity_proof(
            &proof_id,
            "catalog-add-after-reset",
            "vendor/model-latest",
            TARGET_PROVIDER_ID,
            base,
            &binding,
            api_key,
            Some("fixture-catalog-digest"),
            Duration::ZERO,
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
            None,
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
            None,
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
            None,
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
