//! Gateway discovery and capability diagnostics, separate from Codex's catalog.
//! No discovered model is advertised as a native Code Agent without evidence.
use crate::app::model_capability::{
    select_ready_default, AutoSetupCandidate, AutoSetupCandidateStatus, AutoSetupCheckpoint,
    AutoSetupPhase, CapabilityContext, CapabilitySidecar, ModelCapabilityReport, ModelState,
    EVIDENCE_TTL_SECONDS, SIDECAR_IDENTITY,
};
use crate::app::model_diagnostics::{digest_hex, safe_identifier, ProbeError};
use crate::app::model_probe::{ProbeClient, BODY_LIMIT, REFRESH_BUDGET};
use crate::app::{atomic_file, paths, usb_guard};
use crate::errors::AppError;
use serde::{Deserialize, Serialize};
#[path = "model_catalog_activation.rs"]
mod activation;
pub use activation::{NativeRuntimeTarget, NativeVerificationOptions};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering as AtomicOrdering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

const CATALOG_FILE_NAME: &str = "codex-usb-native-model-catalog.json";
const SIDECAR_FILE_NAME: &str = "codex-usb-model-capabilities.json";
const CONFIG_FILE_NAME: &str = "config.toml";
const RELOAD_MARKER_FILE_NAME: &str = "codex-usb-native-model-catalog.reload";
const CATALOG_IDENTITY: &str = "404token-unified-api-internal";
const CATALOG_CLIENT_VERSION: &str = "codex-usb-gateway-2";
const CONFIG_BEGIN: &str = "# BEGIN CODEX-USB-NATIVE-MODEL-CATALOG v1";
const CONFIG_END: &str = "# END CODEX-USB-NATIVE-MODEL-CATALOG v1";
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CATALOG_MUTATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(crate) static FILE_LOCK: Mutex<()> = Mutex::new(());
const RUN_HISTORY_LIMIT: usize = 256;
static RUNS: Mutex<RunRegistry> = Mutex::new(RunRegistry {
    active: None,
    cancelled_before_start: BTreeSet::new(),
    finished: VecDeque::new(),
});

async fn with_catalog_mutation_lock<T>(
    lock: &tokio::sync::Mutex<()>,
    operation: impl Future<Output = T>,
) -> T {
    let _guard = lock.lock().await;
    operation.await
}

struct RegisteredRun {
    id: String,
    cancelled: Arc<AtomicBool>,
}
struct RunRegistry {
    active: Option<RegisteredRun>,
    cancelled_before_start: BTreeSet<String>,
    finished: VecDeque<String>,
}
#[derive(Debug, PartialEq, Eq)]
enum RunAdmission {
    Busy,
    Cancelled,
    Finished,
}
impl RunRegistry {
    fn remember_finished(&mut self, id: &str) {
        if !self.finished.iter().any(|entry| entry == id) {
            self.finished.push_back(id.into());
            if self.finished.len() > RUN_HISTORY_LIMIT {
                self.finished.pop_front();
            }
        }
    }
    fn begin(&mut self, id: &str) -> Result<Arc<AtomicBool>, RunAdmission> {
        if self.finished.iter().any(|entry| entry == id) {
            return Err(RunAdmission::Finished);
        }
        if self.cancelled_before_start.remove(id) {
            self.remember_finished(id);
            return Err(RunAdmission::Cancelled);
        }
        // Never evict an acknowledged cancellation just to admit more work.
        if self.active.is_some() || self.cancelled_before_start.len() >= RUN_HISTORY_LIMIT {
            return Err(RunAdmission::Busy);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        self.active = Some(RegisteredRun {
            id: id.into(),
            cancelled: cancelled.clone(),
        });
        Ok(cancelled)
    }
    fn cancel(&mut self, id: &str) -> bool {
        if let Some(active) = self.active.as_ref().filter(|run| run.id == id) {
            active.cancelled.store(true, AtomicOrdering::Release);
            return true;
        }
        if self.finished.iter().any(|entry| entry == id) {
            return false;
        }
        if self.cancelled_before_start.contains(id) {
            return true;
        }
        if self.cancelled_before_start.len() >= RUN_HISTORY_LIMIT {
            return false;
        }
        self.cancelled_before_start.insert(id.into());
        true
    }
    fn finish(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|run| run.id == id) {
            self.active = None;
            self.remember_finished(id);
        }
    }
}
fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

const AUTO_SETUP_CLOCK_SOURCE: &str = "windows:GetTickCount64-ms";
const AUTO_SETUP_BOOT_SOURCE: &str = "Win32_OperatingSystem.LastBootUpTime";
const AUTO_SETUP_CHECKPOINT_RESULT_CODES: &[&str] = &[
    "probe-inflight",
    "gateway-complete",
    "gateway-incomplete",
    "native-complete",
    "native-incomplete",
    "previous-process-result-untrusted",
    "deadline-exhausted",
    "cancelled",
    "apply-complete",
    "apply-incomplete",
    "operation-incomplete",
];

#[derive(Debug, Clone)]
struct SetupClock {
    boot_identity: String,
    monotonic_ms: u64,
    utc_now: String,
    unix_now: u64,
}

fn format_unix_utc(seconds: u64) -> Option<String> {
    let seconds = i64::try_from(seconds).ok()?;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

#[cfg(windows)]
fn read_setup_clock() -> Option<SetupClock> {
    use std::os::windows::process::CommandExt;
    let system_root = env::var_os("SystemRoot")?;
    let powershell = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut child = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference='Stop'; $v=(Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop).LastBootUpTime; [Console]::Write($v.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ',[Globalization.CultureInfo]::InvariantCulture))",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .ok()?;
    let process_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() >= process_deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let boot_identity = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if !valid_rfc3339_utc(&boot_identity) {
        return None;
    }
    let monotonic_ms = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
    let unix_now = unix_seconds();
    Some(SetupClock {
        boot_identity,
        monotonic_ms,
        utc_now: format_unix_utc(unix_now)?,
        unix_now,
    })
}

#[cfg(not(windows))]
fn read_setup_clock() -> Option<SetupClock> {
    None
}

#[cfg(windows)]
fn windows_tick_now() -> Option<u64> {
    Some(unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() })
}

#[cfg(not(windows))]
fn windows_tick_now() -> Option<u64> {
    None
}

fn valid_auto_setup_checkpoint_shape(checkpoint: &AutoSetupCheckpoint) -> bool {
    let mut seen = BTreeSet::new();
    valid_run_id(&checkpoint.operation_id)
        && valid_git_revision(&checkpoint.service_revision)
        && checkpoint.authorization_scope_sha256.len() == 64
        && checkpoint
            .authorization_scope_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && valid_rfc3339_utc(&checkpoint.started_at_utc)
        && valid_rfc3339_utc(&checkpoint.deadline_utc)
        && checkpoint.started_at_utc < checkpoint.deadline_utc
        && checkpoint.monotonic_clock_source == AUTO_SETUP_CLOCK_SOURCE
        && checkpoint.boot_identity_source == AUTO_SETUP_BOOT_SOURCE
        && valid_rfc3339_utc(&checkpoint.boot_identity)
        && checkpoint
            .deadline_monotonic_ms
            .checked_sub(checkpoint.started_monotonic_ms)
            == Some(activation::AUTO_SETUP_BUDGET.as_millis() as u64)
        && (1..=3).contains(&checkpoint.candidates.len())
        && checkpoint.candidates.iter().all(|candidate| {
            safe_identifier(&candidate.model_id)
                && seen.insert(candidate.model_id.as_str())
                && candidate
                    .result_code
                    .as_deref()
                    .is_none_or(|code| AUTO_SETUP_CHECKPOINT_RESULT_CODES.contains(&code))
        })
}

fn read_auto_setup_checkpoint(
    bytes: Option<&[u8]>,
) -> Result<Option<AutoSetupCheckpoint>, AppError> {
    let Some(bytes) = bytes else { return Ok(None) };
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| engine("能力 sidecar 损坏，保留原文件"))?;
    if value.get("identity").and_then(Value::as_str) != Some(SIDECAR_IDENTITY) {
        return Err(engine("能力 sidecar 非本管理器所有，未覆盖"));
    }
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if !matches!(version, 1 | 2 | 3) {
        return Err(engine("能力 sidecar schema 不支持，未覆盖"));
    }
    let Some(checkpoint) = value.get("autoSetup").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if version != 3 {
        return Err(engine("能力 sidecar 自动配置检查点版本不匹配，未覆盖"));
    }
    let checkpoint: AutoSetupCheckpoint = serde_json::from_value(checkpoint.clone())
        .map_err(|_| engine("能力 sidecar 自动配置检查点损坏，未覆盖"))?;
    if !valid_auto_setup_checkpoint_shape(&checkpoint) {
        return Err(engine("能力 sidecar 自动配置检查点无效，未覆盖"));
    }
    Ok(Some(checkpoint))
}

fn new_auto_setup_checkpoint(
    operation_id: &str,
    candidates: &[String],
    service_revision: &str,
    authorization_scope_sha256: &str,
    clock: &SetupClock,
) -> Result<AutoSetupCheckpoint, AppError> {
    let budget_ms = activation::AUTO_SETUP_BUDGET.as_millis() as u64;
    let deadline_monotonic_ms = clock
        .monotonic_ms
        .checked_add(budget_ms)
        .ok_or_else(|| engine("单调时钟检查点溢出；未发送模型请求"))?;
    let deadline_unix = clock
        .unix_now
        .checked_add(activation::AUTO_SETUP_BUDGET.as_secs())
        .ok_or_else(|| engine("检查点时间溢出；未发送模型请求"))?;
    let checkpoint = AutoSetupCheckpoint {
        operation_id: operation_id.to_owned(),
        phase: AutoSetupPhase::GatewayVerification,
        service_revision: service_revision.to_owned(),
        authorization_scope_sha256: authorization_scope_sha256.to_owned(),
        started_at_utc: clock.utc_now.clone(),
        deadline_utc: format_unix_utc(deadline_unix)
            .ok_or_else(|| engine("无法格式化检查点时间；未发送模型请求"))?,
        monotonic_clock_source: AUTO_SETUP_CLOCK_SOURCE.into(),
        boot_identity_source: AUTO_SETUP_BOOT_SOURCE.into(),
        boot_identity: clock.boot_identity.clone(),
        started_monotonic_ms: clock.monotonic_ms,
        deadline_monotonic_ms,
        candidates: candidates
            .iter()
            .map(|model_id| AutoSetupCandidate {
                model_id: model_id.clone(),
                status: AutoSetupCandidateStatus::Untouched,
                result_code: None,
            })
            .collect(),
    };
    if !valid_auto_setup_checkpoint_shape(&checkpoint) {
        return Err(engine("自动配置检查点结构无效；未发送模型请求"));
    }
    Ok(checkpoint)
}

fn auto_setup_checkpoint_matches_resume(
    checkpoint: &AutoSetupCheckpoint,
    now: &SetupClock,
    service_revision: &str,
    authorization_scope_sha256: &str,
    candidate_ids: &[String],
) -> bool {
    valid_auto_setup_checkpoint_shape(checkpoint)
        && checkpoint.boot_identity == now.boot_identity
        && checkpoint.phase != AutoSetupPhase::ApplyPending
        && checkpoint.phase != AutoSetupPhase::Complete
        && now.monotonic_ms < checkpoint.deadline_monotonic_ms
        && checkpoint.service_revision == service_revision
        && checkpoint.authorization_scope_sha256 == authorization_scope_sha256
        && checkpoint
            .candidates
            .iter()
            .map(|candidate| candidate.model_id.clone())
            .collect::<Vec<_>>()
            == candidate_ids
}

fn mark_prior_candidate_results_untrusted(checkpoint: &mut AutoSetupCheckpoint) {
    for candidate in &mut checkpoint.candidates {
        if candidate.status != AutoSetupCandidateStatus::Untouched {
            candidate.status = AutoSetupCandidateStatus::Uncertain;
            candidate.result_code = Some("previous-process-result-untrusted".into());
        }
    }
}

fn has_untouched_auto_candidates(checkpoint: &AutoSetupCheckpoint) -> bool {
    checkpoint
        .candidates
        .iter()
        .any(|candidate| candidate.status == AutoSetupCandidateStatus::Untouched)
}

fn persist_auto_setup_checkpoint(
    path: &Path,
    expected_bytes: &mut Option<Vec<u8>>,
    context: &CapabilityContext,
    models: &[ModelCapabilityReport],
    checkpoint: &AutoSetupCheckpoint,
    binding: &codex_usb_license::UsbAccountBinding,
    home: &Path,
) -> Result<(), AppError> {
    if !valid_auto_setup_checkpoint_shape(checkpoint) {
        return Err(engine("自动配置检查点无效；未发送后续模型请求"));
    }
    let _files = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if usb_guard::authorized_account_binding()?.as_ref() != Some(binding)
        || codex_home_dir().as_deref() != Some(home)
    {
        return Err(engine("U 盘绑定或 Codex 用户目录已变化；未继续模型请求"));
    }
    if read_optional(path)? != *expected_bytes {
        return Err(engine(
            "能力 sidecar 已被其他进程修改；未覆盖，未继续模型请求",
        ));
    }
    let backup = atomic_file::backup_path(path);
    regular_or_absent(&backup)?;
    if backup.exists() && !owned_json(&backup, SIDECAR_IDENTITY) {
        return Err(engine("能力备份不归本管理器所有，未覆盖"));
    }
    let sidecar = CapabilitySidecar {
        schema_version: 3,
        identity: SIDECAR_IDENTITY.into(),
        context: context.clone(),
        fetched_at: unix_seconds(),
        models: models.to_vec(),
        auto_setup: Some(checkpoint.clone()),
    };
    let bytes = serde_json::to_vec_pretty(&sidecar).map_err(|_| engine("能力检查点序列化失败"))?;
    if bytes.len() > BODY_LIMIT {
        return Err(engine("能力检查点超过本地大小限制"));
    }
    atomic_file::write_atomic(path, &bytes)
        .map_err(|_| engine("能力检查点原子写入失败，未发送后续模型请求"))?;
    *expected_bytes = Some(bytes);
    Ok(())
}

fn commit_completed_auto_setup_checkpoint<R, W, C>(
    expected_sidecar_bytes: &Option<Vec<u8>>,
    bytes: &[u8],
    read_current: R,
    write_atomic: W,
    confirm_setup_complete: C,
) -> Result<(), AppError>
where
    R: FnMut() -> Result<Option<Vec<u8>>, AppError>,
    W: FnOnce(&[u8]) -> Result<(), AppError>,
    C: FnOnce() -> Result<(), &'static str>,
{
    let mut read_current = read_current;
    if bytes.len() > BODY_LIMIT {
        return Err(engine("最终检查点超过本地大小限制"));
    }
    if read_current()? != *expected_sidecar_bytes {
        return Err(engine("配置应用后能力文件被其他进程修改；结果未确认为完成"));
    }
    write_atomic(bytes).map_err(|_| engine("最终检查点写入失败；配置完成状态未确认"))?;
    if read_current()?.as_deref() != Some(bytes) {
        return Err(engine("最终检查点回读失败；配置完成状态未确认"));
    }
    confirm_setup_complete()
        .map_err(|_| engine("最终检查点已写入，但当前模型配置 proof 未能最终确认"))?;
    Ok(())
}

fn complete_auto_setup_after_apply<R, W, C>(
    applied: bool,
    checkpoint: Option<&mut AutoSetupCheckpoint>,
    default_model: Option<&str>,
    context: &CapabilityContext,
    fetched_at: u64,
    models: &[ModelCapabilityReport],
    expected_sidecar_bytes: &mut Option<Vec<u8>>,
    read_current: R,
    write_atomic: W,
    confirm_setup_complete: C,
) -> Result<(), AppError>
where
    R: FnMut() -> Result<Option<Vec<u8>>, AppError>,
    W: FnOnce(&[u8]) -> Result<(), AppError>,
    C: FnOnce() -> Result<(), &'static str>,
{
    if !applied {
        return Ok(());
    }
    let checkpoint = checkpoint.ok_or_else(|| engine("模型已应用但缺少自动配置完成检查点"))?;
    if checkpoint.phase != AutoSetupPhase::ApplyPending {
        return Err(engine("自动配置阶段不是待应用状态；未确认完成"));
    }
    let default_model =
        default_model.ok_or_else(|| engine("已应用模型缺少默认模型 ID；未确认完成"))?;
    let candidate = checkpoint
        .candidates
        .iter_mut()
        .find(|candidate| candidate.model_id == default_model)
        .ok_or_else(|| engine("默认模型不在认证候选检查点中；未确认完成"))?;
    if candidate.status != AutoSetupCandidateStatus::NativeComplete {
        return Err(engine("默认模型没有当前进程原生验证结果；未确认完成"));
    }
    candidate.result_code = Some("apply-complete".into());
    checkpoint.phase = AutoSetupPhase::Complete;
    if !valid_auto_setup_checkpoint_shape(checkpoint) {
        return Err(engine("自动配置完成检查点无效；未确认完成"));
    }
    let completed = CapabilitySidecar {
        schema_version: 3,
        identity: SIDECAR_IDENTITY.into(),
        context: context.clone(),
        fetched_at,
        models: models.to_vec(),
        auto_setup: Some(checkpoint.clone()),
    };
    let completed_bytes =
        serde_json::to_vec_pretty(&completed).map_err(|_| engine("最终检查点序列化失败"))?;
    commit_completed_auto_setup_checkpoint(
        expected_sidecar_bytes,
        &completed_bytes,
        read_current,
        write_atomic,
        confirm_setup_complete,
    )?;
    *expected_sidecar_bytes = Some(completed_bytes);
    Ok(())
}

fn should_probe_gateway_model(
    native_requested: bool,
    certified_model_ids: &[String],
    model_id: &str,
) -> bool {
    native_requested
        && certified_model_ids
            .iter()
            .any(|candidate| candidate == model_id)
}

fn gateway_matrix_complete(model: &ModelCapabilityReport) -> bool {
    use crate::app::model_capability::CapabilityState;
    model.evidence_scope == crate::app::model_capability::EvidenceScope::Gateway
        && model.error_code.is_none()
        && [
            model.checks.text,
            model.checks.single_tool,
            model.checks.tool_result,
            model.checks.multiple_tools,
            model.checks.tool_error,
            model.checks.stream,
            model.checks.multi_turn,
        ]
        .iter()
        .all(|state| *state == CapabilityState::Verified)
}

const CODEX_COMPATIBILITY_PROFILE: &str = "codex-responses-tool-use-v1";
const CODEX_CERTIFICATION_PREFIX: &str = "t0-final-c4d48cf75-404token-";
const CODEX_EVIDENCE_TIME_SOURCE: &str = "delivery_artifact_last_write_utc";
const CODEX_CERTIFICATION_EVIDENCE_SHA256: &str =
    "5EBAA126CB23033766027B81B14CA0FC10D11221BE90C379B79AA15998EFB7F5";
const REQUIRED_CODEX_CAPABILITIES: [&str; 8] = [
    "responses",
    "streaming",
    "tool_calls",
    "tool_results",
    "continuation",
    "parallel_tools",
    "error_recovery",
    "compact",
];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompatibilityModel {
    id: String,
    certification_id: String,
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompatibilityManifest {
    schema_version: u32,
    service_revision: String,
    profile_id: String,
    evidence_recorded_at: String,
    evidence_time_source: String,
    model_priority: Vec<String>,
    default_model_id: String,
    models: Vec<CompatibilityModel>,
}

fn validate_compatibility_manifest(
    manifest_payload: &Value,
    models_payload: &Value,
    api_key: &str,
) -> Result<CompatibilityManifest, &'static str> {
    let mut manifest: CompatibilityManifest = serde_json::from_value(manifest_payload.clone())
        .map_err(|_| "compatibility-manifest-invalid")?;
    if manifest.schema_version != 1
        || manifest.profile_id != CODEX_COMPATIBILITY_PROFILE
        || !valid_git_revision(&manifest.service_revision)
        || !valid_rfc3339_utc(&manifest.evidence_recorded_at)
        || manifest.evidence_time_source != CODEX_EVIDENCE_TIME_SOURCE
        || manifest.model_priority.is_empty()
        || manifest.model_priority.len() > 3
        || manifest.models.len() != manifest.model_priority.len()
        || manifest.default_model_id != manifest.model_priority[0]
    {
        return Err("compatibility-manifest-invalid");
    }
    let mut seen_ids = BTreeSet::new();
    let mut seen_certifications = BTreeSet::new();
    for (index, model) in manifest.models.iter().enumerate() {
        let id = model.id.trim();
        if !safe_identifier(id)
            || id != model.id
            || manifest.model_priority.get(index).map(String::as_str) != Some(id)
            || !seen_ids.insert(id.to_owned())
            || !seen_certifications.insert(model.certification_id.clone())
            || !valid_certification_id(&model.certification_id, id)
            || !valid_required_capabilities(&model.capabilities)
        {
            return Err("compatibility-manifest-invalid");
        }
    }

    let rows = models_payload
        .get("data")
        .or_else(|| models_payload.get("models"))
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty() && rows.len() <= 512)
        .ok_or("models-catalog-invalid")?;
    let mut visible_counts = BTreeMap::<String, usize>::new();
    for row in rows {
        let Some(id) = row
            .get("id")
            .or_else(|| row.get("slug"))
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        if !safe_identifier(id) || (!api_key.is_empty() && id.contains(api_key)) {
            continue;
        }
        *visible_counts.entry(id.to_owned()).or_default() += 1;
    }
    if visible_counts.get(&manifest.default_model_id).copied() != Some(1) {
        return Err("certified-models-catalog-mismatch");
    }
    manifest
        .models
        .retain(|model| visible_counts.get(&model.id).copied() == Some(1));
    manifest.model_priority = manifest
        .models
        .iter()
        .map(|model| model.id.clone())
        .collect();
    if manifest.models.is_empty()
        || manifest.model_priority.first() != Some(&manifest.default_model_id)
    {
        return Err("no-certified-models");
    }
    Ok(manifest)
}

fn valid_git_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_certification_id(value: &str, model_id: &str) -> bool {
    let prefix = format!("{CODEX_CERTIFICATION_PREFIX}{model_id}-sha256-");
    value
        .strip_prefix(&prefix)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(CODEX_CERTIFICATION_EVIDENCE_SHA256))
}

fn valid_required_capabilities(capabilities: &[String]) -> bool {
    let set = capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    set.len() == capabilities.len()
        && REQUIRED_CODEX_CAPABILITIES
            .iter()
            .all(|required| set.contains(required))
        && capabilities.len() == REQUIRED_CODEX_CAPABILITIES.len()
}

fn valid_rfc3339_utc(value: &str) -> bool {
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let (whole, fraction) = value
        .split_once('.')
        .map_or((value, None), |(w, f)| (w, Some(f)));
    if !whole.is_ascii()
        || whole.len() != 19
        || whole.as_bytes()[4] != b'-'
        || whole.as_bytes()[7] != b'-'
        || whole.as_bytes()[10] != b'T'
        || whole.as_bytes()[13] != b':'
        || whole.as_bytes()[16] != b':'
        || [0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
            .iter()
            .any(|range| {
                !whole[range.clone()]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
            })
        || fraction.is_some_and(|fraction| {
            fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return false;
    }
    let year = whole[0..4].parse::<u32>().ok();
    let month = whole[5..7].parse::<u32>().ok();
    let day = whole[8..10].parse::<u32>().ok();
    let hour = whole[11..13].parse::<u32>().ok();
    let minute = whole[14..16].parse::<u32>().ok();
    let second = whole[17..19].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
        (year, month, day, hour, minute, second)
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year > 0 && (1..=days_in_month).contains(&day) && hour < 24 && minute < 60 && second < 60
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeModelCatalogReport {
    pub status: String,
    pub model_count: usize,
    pub catalog_path: Option<String>,
    pub fetched_at: Option<String>,
    pub message: String,
    pub attempts: u32,
    pub error_kind: Option<String>,
    pub run_id: String,
    pub gateway_status: String,
    pub ready_count: usize,
    pub quarantined_count: usize,
    pub default_model: Option<String>,
    pub sidecar_path: Option<String>,
    pub native_status: String,
    pub activation_status: String,
    pub service_revision: Option<String>,
    pub models: Vec<ModelCapabilityReport>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeModelCatalogActionRequest {
    pub operation: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub connectivity_proof_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeModelCatalogActionReport {
    pub status: String,
    pub model_ids: Vec<String>,
    pub restart_required: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_fingerprint: Option<String>,
}
fn report(run_id: &str, status: &str, message: &str) -> NativeModelCatalogReport {
    NativeModelCatalogReport {
        status: status.into(),
        model_count: 0,
        catalog_path: None,
        fetched_at: None,
        message: message.into(),
        attempts: 0,
        error_kind: None,
        run_id: run_id.into(),
        gateway_status: "unknown".into(),
        ready_count: 0,
        quarantined_count: 0,
        default_model: None,
        sidecar_path: None,
        native_status: "not_requested".into(),
        activation_status: "not_requested".into(),
        service_revision: None,
        models: Vec::new(),
    }
}
fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(message.into())
}
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

fn config_path() -> Result<PathBuf, AppError> {
    codex_home_dir()
        .map(|home| config_path_for(&home))
        .ok_or_else(|| engine("无法确定 Codex 配置目录，模型目录未刷新"))
}

fn config_path_for(home: &Path) -> PathBuf {
    home.join(CONFIG_FILE_NAME)
}

fn catalog_path_for(home: &Path) -> PathBuf {
    home.join(CATALOG_FILE_NAME)
}

fn reload_marker_path() -> Result<PathBuf, AppError> {
    codex_home_dir()
        .map(|home| home.join(RELOAD_MARKER_FILE_NAME))
        .ok_or_else(|| engine("无法确定 Codex 用户目录，模型目录重载标记未写入"))
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
                .filter(|id| safe_identifier(id))?
                .to_string();
            let owned_by = object
                .get("owned_by")
                .or_else(|| object.get("ownedBy"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|owner| safe_identifier(owner))
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

fn conservative_tested_model_entry(model_id: &str, owner: Option<&str>, priority: usize) -> Value {
    let display_name = owner
        .map(|owner| format!("{owner} / {model_id}"))
        .unwrap_or_else(|| model_id.to_owned());
    json!({
        "slug": model_id,
        "display_name": display_name,
        "description": "Text model supplied by the configured gateway; only one text response was verified.",
        "shell_type": "unified_exec",
        "visibility": "list",
        "supported_in_api": true,
        "priority": priority,
        "support_verbosity": false,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "availability_nux": null,
        "upgrade": null,
        "include_skills_usage_instructions": false,
        "include_plugin_usage_instructions": false,
        "include_apps_usage_instructions": false,
        "default_reasoning_summary": null,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "web_search_tool_type": null,
        "truncation_policy": null,
        "supports_image_detail_original": false,
        "comp_hash": null,
        "effective_context_window_percent": null,
        "experimental_supported_tools": [],
        "input_modalities": ["text"],
        "supports_search_tool": false,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": true,
        "default_reasoning_level": null,
        "supported_reasoning_levels": []
    })
}

fn expected_catalog_reference(config: &Path) -> Result<String, AppError> {
    let catalog = config
        .parent()
        .ok_or_else(|| engine("模型配置没有父目录"))?
        .join(CATALOG_FILE_NAME);
    Ok(format!(
        "model_catalog_json = \"{}\"",
        toml_string(&catalog)
    ))
}

fn validate_managed_catalog_reference(text: &str, config: &Path) -> Result<bool, AppError> {
    let expected = expected_catalog_reference(config)?;
    let mut managed = false;
    let mut seen = false;
    let mut count = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == CONFIG_BEGIN {
            if managed || seen {
                return Err(engine("模型目录托管块重复，保留原配置"));
            }
            managed = true;
            seen = true;
        } else if trimmed == CONFIG_END {
            if !managed {
                return Err(engine("模型目录托管块损坏，保留原配置"));
            }
            managed = false;
        } else if managed && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if trimmed != expected {
                return Err(engine("模型目录托管块中的路径已被修改，保留原配置"));
            }
            count += 1;
        }
    }
    if managed {
        return Err(engine("模型目录托管块未闭合，保留原配置"));
    }
    if seen && count != 1 {
        return Err(engine("模型目录托管路径不明确，保留原配置"));
    }
    Ok(seen)
}

fn cleaned_config_without_external_catalog(
    text: &str,
    config: &Path,
) -> Result<(String, bool), AppError> {
    let managed = validate_managed_catalog_reference(text, config)?;
    let cleaned = remove_managed_config(text)?;
    let parsed: toml::Value = toml::from_str(cleaned.trim_start_matches('\u{feff}'))
        .map_err(|_| engine("Codex 配置无法安全解析，未修改模型目录"))?;
    if parsed.get("model_catalog_json").is_some() {
        return Err(engine(
            "检测到外部模型目录配置；已保留原配置，请先处理该配置后重试",
        ));
    }
    Ok((cleaned, managed))
}

fn has_unmanaged_catalog_key(text: &str) -> bool {
    toml::from_str::<toml::Value>(text.trim_start_matches('\u{feff}'))
        .ok()
        .is_none_or(|value| value.get("model_catalog_json").is_some())
}

fn catalog_model_ids(value: &Value) -> Result<Vec<String>, AppError> {
    if value.get("identity").and_then(Value::as_str) != Some(CATALOG_IDENTITY) {
        return Err(engine("模型目录文件不归本管理器所有，已保留原文件"));
    }
    let rows = value
        .get("models")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 512)
        .ok_or_else(|| engine("本地模型目录格式无效，已保留原文件"))?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row
            .get("slug")
            .and_then(Value::as_str)
            .filter(|id| safe_identifier(id))
            .ok_or_else(|| engine("本地模型目录包含无效模型标识，已保留原文件"))?;
        ids.push(id.to_owned());
    }
    Ok(ids)
}

fn action_report(
    status: &str,
    model_ids: Vec<String>,
    message: &str,
) -> NativeModelCatalogActionReport {
    NativeModelCatalogActionReport {
        status: status.to_owned(),
        model_ids,
        restart_required: restart_required(),
        message: message.to_owned(),
        connection_fingerprint: None,
        credential_fingerprint: None,
    }
}

fn catalog_state_for_home(home: &Path) -> Result<NativeModelCatalogActionReport, AppError> {
    let _files = FILE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let config = config_path_for(home);
    let catalog = catalog_path_for(home);
    let config_bytes = read_optional(&config)?;
    let text = config_bytes
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| engine("Codex 配置不是 UTF-8"))?
        .unwrap_or("");
    let (_, managed) = cleaned_config_without_external_catalog(text, &config)?;
    let catalog_bytes = read_optional(&catalog)?;
    let Some(bytes) = catalog_bytes else {
        if managed {
            return Err(engine("模型目录配置存在但文件缺失；请先复位模型目录"));
        }
        return Ok(action_report(
            "ok",
            Vec::new(),
            "当前没有启用由管理器添加的上游模型",
        ));
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| engine("本地模型目录格式无效；已保留原文件"))?;
    let ids = catalog_model_ids(&value)?;
    if managed {
        Ok(action_report("ok", ids, "已读取当前启用的上游模型目录"))
    } else {
        Ok(action_report(
            "orphaned",
            Vec::new(),
            "检测到未启用的管理器目录文件；请复位后再添加模型",
        ))
    }
}

fn catalog_state() -> Result<NativeModelCatalogActionReport, AppError> {
    let home = codex_home_dir().ok_or_else(|| engine("无法确定 Codex 用户目录"))?;
    let mut report = catalog_state_for_home(&home)?;
    if let Some(binding) = usb_guard::authorized_account_binding().ok().flatten() {
        if let Ok(base) = provider_base(&binding.ccswitch.provider_id) {
            report.connection_fingerprint = Some(crate::app::gateway_model_probe::binding_digest(
                &binding, &base,
            ));
            report.credential_fingerprint = Some(format!(
                "sha256:{}",
                &digest_hex(binding.ccswitch.device_api_key.trim().as_bytes())[..12]
            ));
        }
    }
    Ok(report)
}

fn empty_native_catalog() -> Value {
    json!({
        "schema_version": 1,
        "fetched_at": format!("unix:{}", unix_seconds()),
        "client_version": CATALOG_CLIENT_VERSION,
        "identity": CATALOG_IDENTITY,
        "models": []
    })
}

pub(crate) fn add_tested_model_to_catalog(
    request_id: &str,
    model_id: &str,
    owner: Option<&str>,
    expected_binding: &codex_usb_license::UsbAccountBinding,
    cancelled: &AtomicBool,
) -> Result<NativeModelCatalogActionReport, AppError> {
    let home = codex_home_dir().ok_or_else(|| engine("无法确定 Codex 用户目录"))?;
    let expected_home = home.clone();
    add_tested_model_files(
        &home,
        model_id,
        owner,
        cancelled,
        || {
            Ok(
                usb_guard::authorized_account_binding()?.as_ref() == Some(expected_binding)
                    && codex_home_dir().as_deref() == Some(expected_home.as_path()),
            )
        },
        || crate::app::gateway_model_probe::mark_catalog_commit_started(request_id),
        mark_restart_required,
    )
}

fn add_tested_model_files<F, C, M>(
    home: &Path,
    model_id: &str,
    owner: Option<&str>,
    cancelled: &AtomicBool,
    is_current: F,
    mark_commit: C,
    mark_restart: M,
) -> Result<NativeModelCatalogActionReport, AppError>
where
    F: Fn() -> Result<bool, AppError>,
    C: Fn(),
    M: Fn() -> Result<(), AppError>,
{
    if !safe_identifier(model_id) || owner.is_some_and(|value| !safe_identifier(value)) {
        return Err(engine("已测试模型的标识无效，未写入目录"));
    }
    let _files = FILE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    if cancelled.load(AtomicOrdering::Acquire) {
        return Err(engine("模型添加已取消；未修改 Codex 目录"));
    }
    if !is_current()? {
        return Err(engine("USB 账户绑定已变化；请重新发现并测试模型"));
    }
    let config = config_path_for(home);
    let catalog = catalog_path_for(home);
    regular_or_absent(&catalog)?;
    regular_or_absent(&config)?;
    let catalog_backup = atomic_file::backup_path(&catalog);
    let config_backup = atomic_file::backup_path(&config);
    regular_or_absent(&catalog_backup)?;
    regular_or_absent(&config_backup)?;
    if catalog_backup.exists() && !owned_json(&catalog_backup, CATALOG_IDENTITY) {
        return Err(engine("模型目录备份不归本管理器所有，未覆盖"));
    }

    let config_snapshot = read_optional(&config)?;
    let config_backup_snapshot = read_optional(&config_backup)?;
    let config_text = config_snapshot
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| engine("Codex 配置不是 UTF-8，模型未添加"))?
        .unwrap_or("");
    let (cleaned_config, managed) = cleaned_config_without_external_catalog(config_text, &config)?;
    let catalog_snapshot = read_optional(&catalog)?;
    let mut catalog_value = match catalog_snapshot.as_deref() {
        Some(bytes) => {
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| engine("本地模型目录格式无效；请先复位模型目录"))?;
            let _ = catalog_model_ids(&value)?;
            if !managed {
                return Err(engine("发现未启用的管理器目录文件；请先复位模型目录"));
            }
            value
        }
        None => empty_native_catalog(),
    };
    let existing_ids = catalog_model_ids(&catalog_value)?;
    if existing_ids.iter().any(|id| id == model_id) {
        return Ok(action_report(
            "already_added",
            existing_ids,
            "该模型已在 Codex 目录中，无需重复添加",
        ));
    }
    let models = catalog_value
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| engine("本地模型目录格式无效；请先复位模型目录"))?;
    models.push(conservative_tested_model_entry(
        model_id,
        owner,
        models.len() + 1,
    ));
    let resulting_ids = catalog_model_ids(&catalog_value)?;
    let catalog_bytes =
        serde_json::to_vec_pretty(&catalog_value).map_err(|_| engine("模型目录序列化失败"))?;
    if catalog_bytes.len() > BODY_LIMIT {
        return Err(engine("模型目录超过本地大小限制"));
    }
    let merged = if managed {
        None
    } else {
        Some(merged_config(&cleaned_config, &catalog)?)
    };
    if !is_current()?
        || read_optional(&config)? != config_snapshot
        || read_optional(&config_backup)? != config_backup_snapshot
        || read_optional(&catalog)? != catalog_snapshot
        || cancelled.load(AtomicOrdering::Acquire)
    {
        return Err(engine("配置或账户在写入前已变化；未覆盖其他修改"));
    }
    if cancelled.load(AtomicOrdering::Acquire) {
        return Err(engine("模型添加已取消；未修改 Codex 目录"));
    }
    mark_commit();
    atomic_file::write_atomic(&catalog, &catalog_bytes)
        .map_err(|_| engine("模型目录写入未完成；请重新读取状态后再试"))?;
    if let Some(merged) = merged {
        if let Err(_) = atomic_file::write_atomic_preserving_backup(&config, merged.as_bytes()) {
            let current = read_optional(&catalog).ok().flatten();
            let safe_to_remove = current.as_deref() == Some(catalog_bytes.as_slice());
            let removed = safe_to_remove && fs::remove_file(&catalog).is_ok();
            if removed {
                return Err(engine(
                    "Codex 配置写入失败；未启用新模型，已清理临时目录文件",
                ));
            }
            return Ok(action_report(
                "cleanup_pending",
                Vec::new(),
                "Codex 配置写入失败；新目录文件清理待重试，请先复位目录",
            ));
        }
    }
    let readback_catalog = read_optional(&catalog)?
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| catalog_model_ids(&value).ok());
    let expected_config_active = if managed {
        true
    } else {
        read_optional(&config)?
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| validate_managed_catalog_reference(text, &config).ok())
            .unwrap_or(false)
    };
    if readback_catalog.as_deref() != Some(resulting_ids.as_slice()) || !expected_config_active {
        return Ok(action_report(
            "outcome_unconfirmed",
            readback_catalog.unwrap_or_default(),
            "目录写入后的回读与预期不一致；请重新打开管理器确认实际状态",
        ));
    }
    if mark_restart().is_err() {
        return Ok(action_report(
            "applied_restart_pending",
            resulting_ids,
            "模型已加入；重载标记未写入，请手动重启 Codex 后检查",
        ));
    }
    Ok(action_report(
        "applied_restart_pending",
        resulting_ids,
        "模型已加入 Codex 菜单；重启 Codex 后生效",
    ))
}

fn reset_owned_catalog() -> Result<NativeModelCatalogActionReport, AppError> {
    activation::invalidate_live();
    let home = codex_home_dir().ok_or_else(|| engine("无法确定 Codex 用户目录"))?;
    reset_owned_catalog_at(&home, mark_restart_required)
}

fn reset_owned_catalog_at<M>(
    home: &Path,
    mark_restart: M,
) -> Result<NativeModelCatalogActionReport, AppError>
where
    M: Fn() -> Result<(), AppError>,
{
    let _files = FILE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let config = config_path_for(home);
    let catalog = catalog_path_for(home);
    let sidecar = home.join(SIDECAR_FILE_NAME);
    let config_backup = atomic_file::backup_path(&config);
    let catalog_backup = atomic_file::backup_path(&catalog);
    let sidecar_backup = atomic_file::backup_path(&sidecar);
    for path in [
        &config,
        &config_backup,
        &catalog,
        &sidecar,
        &catalog_backup,
        &sidecar_backup,
    ] {
        regular_or_absent(path)?;
    }
    let config_snapshot = read_optional(&config)?;
    let config_text = config_snapshot
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| engine("Codex 配置无法安全解析；原配置和模型目录均未改动"))?
        .unwrap_or("");
    let (cleaned_config, managed) = cleaned_config_without_external_catalog(config_text, &config)
        .map_err(|_| {
        engine("检测到外部模型目录配置或配置无法安全解析，已保留原配置；未恢复 Codex 原生目录")
    })?;
    let config_backup_snapshot = read_optional(&config_backup)?;
    let catalog_snapshot = read_optional(&catalog)?;
    let sidecar_snapshot = read_optional(&sidecar)?;
    let catalog_backup_snapshot = read_optional(&catalog_backup)?;
    let sidecar_backup_snapshot = read_optional(&sidecar_backup)?;
    for (bytes, identity, label) in [
        (catalog_snapshot.as_deref(), CATALOG_IDENTITY, "模型目录"),
        (
            catalog_backup_snapshot.as_deref(),
            CATALOG_IDENTITY,
            "模型目录备份",
        ),
        (sidecar_snapshot.as_deref(), SIDECAR_IDENTITY, "能力文件"),
        (
            sidecar_backup_snapshot.as_deref(),
            SIDECAR_IDENTITY,
            "能力文件备份",
        ),
    ] {
        if let Some(bytes) = bytes {
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| engine(format!("{label}无法确认所有权；原文件均未改动")))?;
            if value.get("identity").and_then(Value::as_str) != Some(identity) {
                return Err(engine(format!("{label}不归本管理器所有；原文件均未改动")));
            }
        }
    }
    if managed && catalog_snapshot.is_some() && !owned_json(&catalog, CATALOG_IDENTITY) {
        return Err(engine(
            "模型目录文件所有权无法确认；原配置和模型目录均未改动",
        ));
    }
    if read_optional(&config)? != config_snapshot
        || read_optional(&config_backup)? != config_backup_snapshot
        || read_optional(&catalog)? != catalog_snapshot
        || read_optional(&sidecar)? != sidecar_snapshot
        || read_optional(&catalog_backup)? != catalog_backup_snapshot
        || read_optional(&sidecar_backup)? != sidecar_backup_snapshot
    {
        return Err(engine("模型目录或配置已被其他进程修改；本次复位未执行"));
    }
    if managed {
        atomic_file::write_atomic_preserving_backup(&config, cleaned_config.as_bytes())
            .map_err(|_| engine("复位时无法安全移除管理器配置块；模型目录未清理"))?;
    }
    let mut cleanup_failed = false;
    for path in [&catalog, &catalog_backup, &sidecar, &sidecar_backup] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => cleanup_failed = true,
        }
    }
    let changed = managed
        || catalog_snapshot.is_some()
        || catalog_backup_snapshot.is_some()
        || sidecar_snapshot.is_some()
        || sidecar_backup_snapshot.is_some();
    let marker_ok = !changed || mark_restart().is_ok();
    if cleanup_failed {
        return Ok(action_report(
            "cleanup_pending",
            Vec::new(),
            "Codex 已切回原生模型目录；管理器文件清理待重试",
        ));
    }
    if !marker_ok {
        return Ok(action_report(
            "applied_restart_pending",
            Vec::new(),
            "已恢复 Codex 原生模型目录，但未能写入重载标记；请手动重启 Codex",
        ));
    }
    Ok(action_report(
        if changed {
            "applied_restart_pending"
        } else {
            "already_native"
        },
        Vec::new(),
        if changed {
            "已恢复 Codex 原生模型目录；重启 Codex 后生效"
        } else {
            "当前已使用 Codex 原生模型目录"
        },
    ))
}

pub async fn manage_native_model_catalog(
    request: NativeModelCatalogActionRequest,
) -> Result<NativeModelCatalogActionReport, AppError> {
    match request.operation.as_str() {
        "state" => catalog_state(),
        "add" => {
            let request_id = request
                .request_id
                .as_deref()
                .ok_or_else(|| engine("模型添加请求缺少 requestId"))?;
            let model_id = request
                .model_id
                .as_deref()
                .ok_or_else(|| engine("模型添加请求缺少 modelId"))?;
            let proof_id = request
                .connectivity_proof_id
                .as_deref()
                .ok_or_else(|| engine("模型添加请求缺少测试凭证"))?;
            with_catalog_mutation_lock(
                &CATALOG_MUTATION_LOCK,
                crate::app::gateway_model_probe::add_tested_model(request_id, model_id, proof_id),
            )
            .await
        }
        "reset" => {
            with_catalog_mutation_lock(&CATALOG_MUTATION_LOCK, async {
                crate::app::gateway_model_probe::begin_catalog_reset();
                let result = reset_owned_catalog();
                crate::app::gateway_model_probe::finish_catalog_reset();
                result
            })
            .await
        }
        _ => Err(engine("未知的模型目录操作")),
    }
}

fn merged_config(text: &str, catalog: &Path) -> Result<String, AppError> {
    let without_managed = remove_managed_config(text)?;
    if has_unmanaged_catalog_key(&without_managed) {
        return Err(engine(
            "Codex 已存在未由本管理器管理的 model_catalog_json，已保留原配置",
        ));
    }
    let block = format!(
        "{CONFIG_BEGIN}\nmodel_catalog_json = \"{}\"\n{CONFIG_END}",
        toml_string(catalog)
    );
    let body = without_managed.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        return Ok(format!("{block}\n"));
    }
    Ok(format!("{block}\n\n{body}\n"))
}

struct ActiveRun {
    id: String,
}
impl Drop for ActiveRun {
    fn drop(&mut self) {
        RUNS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .finish(&self.id);
    }
}
pub fn cancel(request_id: &str) -> bool {
    if !valid_run_id(request_id) {
        return false;
    }
    // Share the commit boundary: once true is returned, this run cannot write.
    let _files = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    RUNS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .cancel(request_id)
}
pub fn restart_required() -> bool {
    reload_marker_path()
        .ok()
        .is_some_and(|p| fs::metadata(p).is_ok_and(|m| m.is_file()))
}
pub fn mark_restart_required() -> Result<(), AppError> {
    atomic_file::write_atomic(&reload_marker_path()?, b"catalog-updated\n")
        .map_err(|_| engine("写入模型目录重载标记失败"))
}
pub fn clear_restart_required() {
    if let Ok(p) = reload_marker_path() {
        let _ = fs::remove_file(p);
    }
}
fn regular_or_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(engine("模型配置路径包含重解析点"));
                }
            }
            if !meta.file_type().is_file() {
                return Err(engine("模型配置路径不是普通文件"));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(engine("无法读取模型配置路径")),
    }
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    regular_or_absent(path)?;
    if fs::metadata(path).is_ok_and(|m| m.len() > BODY_LIMIT as u64) {
        return Err(engine("模型配置文件超过大小限制"));
    }
    match fs::File::open(path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take((BODY_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| engine("无法读取模型配置文件"))?;
            if bytes.len() > BODY_LIMIT {
                return Err(engine("模型配置文件超过大小限制"));
            }
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(engine("无法读取模型配置文件")),
    }
}
fn remove_managed_config(text: &str) -> Result<String, AppError> {
    let mut output = String::new();
    let mut managed = false;
    let mut seen = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == CONFIG_BEGIN {
            if managed || seen {
                return Err(engine("模型目录托管块重复，保留原配置"));
            }
            managed = true;
            seen = true;
            continue;
        }
        if trimmed == CONFIG_END {
            if !managed {
                return Err(engine("模型目录托管块损坏，保留原配置"));
            }
            managed = false;
            continue;
        }
        if managed {
            // Never delete a user setting that was inserted inside our block.
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed
                    .strip_prefix("model_catalog_json")
                    .is_some_and(|s| s.trim_start().starts_with('='))
            {
                return Err(engine("模型目录托管块含用户配置，保留原配置"));
            }
        } else {
            output.push_str(line);
        }
    }
    if managed {
        return Err(engine("模型目录托管块未闭合，保留原配置"));
    }
    Ok(output)
}
fn toml_string(value: &Path) -> String {
    value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn deactivate_owned_catalog(config: &Path) -> Result<bool, AppError> {
    let Some(bytes) = read_optional(config)? else {
        return Ok(false);
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| engine("Codex 配置不是 UTF-8，未修改"))?;
    let cleaned = remove_managed_config(text)?;
    if cleaned.as_bytes() == bytes {
        return Ok(false);
    }
    // A user-edited managed block may point to an unrelated catalog. Refuse
    // to detach it merely because its surrounding comment looks familiar.
    let expected_path = config
        .parent()
        .ok_or_else(|| engine("模型配置没有父目录"))?
        .join(CATALOG_FILE_NAME);
    let expected = format!("model_catalog_json = \"{}\"", toml_string(&expected_path));
    let mut managed = false;
    let mut matched = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == CONFIG_BEGIN {
            managed = true;
            continue;
        }
        if trimmed == CONFIG_END {
            managed = false;
            continue;
        }
        if managed && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if trimmed != expected {
                return Err(engine("模型目录引用已由用户修改，未自动移除"));
            }
            matched += 1;
        }
    }
    if matched != 1 {
        return Err(engine("模型目录引用不明确，未自动移除"));
    }
    if expected_path.exists() && !owned_json(&expected_path, CATALOG_IDENTITY) {
        return Err(engine("模型目录文件不是本管理器所有，未自动移除引用"));
    }
    regular_or_absent(&atomic_file::backup_path(config))?;
    if read_optional(config)?.as_deref() != Some(bytes.as_slice()) {
        return Err(engine("Codex 配置已被其他进程修改，请重试"));
    }
    atomic_file::write_atomic_preserving_backup(config, cleaned.as_bytes())
        .map_err(|_| engine("停用旧模型目录失败"))?;
    mark_restart_required()?;
    Ok(true)
}
fn owned_json(path: &Path, identity: &str) -> bool {
    read_optional(path)
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v.get("identity").and_then(Value::as_str) == Some(identity))
}
pub fn cleanup() -> Result<bool, AppError> {
    activation::invalidate_live();
    let _files = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active
        .as_ref()
    {
        active.cancelled.store(true, AtomicOrdering::Release);
    }
    let config = config_path()?;
    let mut changed = deactivate_owned_catalog(&config)?;
    let home = codex_home_dir().ok_or_else(|| engine("无法确定 Codex 用户目录"))?;
    for (name, identity) in [
        (CATALOG_FILE_NAME, CATALOG_IDENTITY),
        (SIDECAR_FILE_NAME, SIDECAR_IDENTITY),
    ] {
        let path = home.join(name);
        for p in [&path, &atomic_file::backup_path(&path)] {
            if owned_json(p, identity) {
                fs::remove_file(p).map_err(|_| engine("删除托管模型文件失败"))?;
                changed = true;
            }
        }
    }
    clear_restart_required();
    Ok(changed)
}
pub(crate) fn provider_base(provider_id: &str) -> Result<String, AppError> {
    let value: Value =
        serde_json::from_str(include_str!("../../resources/ccswitch/providers.json"))
            .map_err(|_| engine("内置 provider 资源无效"))?;
    let provider = value
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|v| v.get("id").and_then(Value::as_str) == Some(provider_id))
        })
        .ok_or_else(|| engine("当前绑定没有本地 Responses provider"))?;
    if provider.get("protocol").and_then(Value::as_str) != Some("responses") {
        return Err(engine("当前 provider 不是 Responses 协议"));
    }
    let raw = provider
        .get("baseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| engine("provider 缺少地址"))?;
    let mut url = url::Url::parse(raw).map_err(|_| engine("provider 地址无效"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(engine("provider 地址不安全"));
    }
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url.to_string())
}
fn binding_context(
    binding: &codex_usb_license::UsbAccountBinding,
    base: &str,
    version: Option<String>,
) -> CapabilityContext {
    // Only public binding identifiers, never the credential or account password.
    let identity = serde_json::to_vec(&(
        binding.ccswitch.provider_id.as_str(),
        base,
        binding.account_ref.as_str(),
        binding.ccswitch.credential_id.as_str(),
        binding.ccswitch.group_id,
        binding.sub2api_user_id,
    ))
    .unwrap_or_default();
    CapabilityContext {
        binding_fingerprint: digest_hex(&identity),
        codex_version: version,
        native_runtime_version: None,
        adapter_version: None,
    }
}
fn read_sidecar(
    path: &Path,
    context: &CapabilityContext,
) -> Result<Option<CapabilitySidecar>, AppError> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| engine("能力 sidecar 损坏，保留原文件"))?;
    if value.get("identity").and_then(Value::as_str) != Some(SIDECAR_IDENTITY) {
        return Err(engine("能力 sidecar 非本管理器所有，未覆盖"));
    }
    // The scheduler checkpoint is parsed separately from cached diagnostics.
    // A checkpoint can never affect model authority or cache eligibility.
    let mut cache_value = value;
    if let Some(object) = cache_value.as_object_mut() {
        object.remove("autoSetup");
    }
    let mut sidecar: CapabilitySidecar = serde_json::from_value(cache_value)
        .map_err(|_| engine("能力 sidecar schema 不支持，未覆盖"))?;
    if !matches!(sidecar.schema_version, 1 | 2 | 3) || sidecar.models.len() > 512 {
        return Err(engine("能力 sidecar schema 不支持，未覆盖"));
    }
    // Persisted runtime information is display/scheduling data, not proof.
    let mut observed_context = sidecar.context.clone();
    observed_context.native_runtime_version = None;
    let mut expected_context = context.clone();
    expected_context.native_runtime_version = None;
    if observed_context != expected_context {
        return Ok(None);
    }
    sidecar.models.retain(|m| {
        safe_identifier(&m.model_id) && m.owned_by.as_deref().is_none_or(safe_identifier)
    });
    for model in &mut sidecar.models {
        model.discard_cached_authority();
        // Do not reflect arbitrary error strings from disk into the UI/log.
        model.error_code = None;
        if model.checked_at.is_some_and(|at| at > unix_seconds()) {
            model.checked_at = None;
        }
    }
    Ok(Some(sidecar))
}
fn finalize_report(
    report: &mut NativeModelCatalogReport,
    context: &CapabilityContext,
    native: &[crate::app::model_capability::NativeRuntimeEvidence],
) {
    for model in &mut report.models {
        let proof = native
            .iter()
            .find(|p| p.model_id == model.model_id && p.owned_by == model.owned_by);
        model.evaluate(context, proof, unix_seconds());
    }
    report.model_count = report.models.len();
    report.ready_count = report
        .models
        .iter()
        .filter(|m| m.state == ModelState::Ready)
        .count();
    report.quarantined_count = report
        .models
        .iter()
        .filter(|m| m.state == ModelState::Quarantined)
        .count();
    // Eligibility and application are different outcomes. publish sets this
    // only after the existing configuration writer acknowledges the change.
    report.default_model = None;
}

fn activation_failure_status(report: &NativeModelCatalogReport) -> &'static str {
    if report.default_model.is_some() {
        "applied_restart_pending"
    } else if report.activation_status == "outcome_unconfirmed" {
        "outcome_unconfirmed"
    } else {
        "incomplete"
    }
}

fn mark_new_auto_operation_required(report: &mut NativeModelCatalogReport) {
    report.status = "failed".into();
    report.native_status = "incomplete".into();
    report.activation_status = "not_eligible".into();
    report.error_kind = Some("auto-setup-new-operation-required".into());
    report.message =
        "上次自动配置无法在原启动周期和截止时间内安全续跑；需重新确认费用并显式开始新操作".into();
}

/// Legacy internal entrypoint; interactive callers use a cancellable run ID.
pub async fn refresh(force: bool) -> Result<NativeModelCatalogReport, AppError> {
    refresh_requested(force, None, std::future::ready(None), None).await
}
pub async fn refresh_requested<F>(
    force: bool,
    request_id: Option<String>,
    runtime_target: F,
    native_verification: Option<NativeVerificationOptions>,
) -> Result<NativeModelCatalogReport, AppError>
where
    F: std::future::Future<Output = Option<NativeRuntimeTarget>>,
{
    if native_verification.is_some() {
        return Err(engine("legacy-auto-activation-disabled"));
    }
    let native_requested = false;
    let id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !valid_run_id(&id) {
        return Err(engine("无效的模型探测请求 ID"));
    }
    let Ok(_run) = REFRESH_LOCK.try_lock() else {
        return Ok(report(&id, "busy", "上一次模型探测仍在收尾，请稍后重试"));
    };
    let cancelled = match RUNS.lock().unwrap_or_else(|e| e.into_inner()).begin(&id) {
        Ok(flag) => flag,
        Err(RunAdmission::Busy) => {
            return Ok(report(&id, "busy", "探测请求仍在收尾，或取消登记已达上限"))
        }
        Err(RunAdmission::Cancelled) => {
            return Ok(report(&id, "cancelled", "探测在开始前已取消；未修改目录"))
        }
        Err(RunAdmission::Finished) => {
            return Ok(report(
                &id,
                "cancelled",
                "该请求已结束；重试须使用新请求 ID",
            ))
        }
    };
    let _active = ActiveRun { id: id.clone() };
    let home = codex_home_dir().ok_or_else(|| engine("无法确定 Codex 用户目录"))?;
    let config = config_path()?;
    {
        let _files = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if cancelled.load(AtomicOrdering::Acquire) {
            return Ok(report(&id, "cancelled", "探测已取消；未修改目录"));
        }
        if !activation::current_catalog_is_verified(&home) {
            activation::invalidate_live();
            deactivate_owned_catalog(&config)?;
        }
    }
    // Authorization and identity checks precede every cache fast path.
    let Some(binding) = usb_guard::authorized_account_binding()? else {
        return Ok(report(
            &id,
            "skipped",
            "未取得有效 U 盘绑定；未复用旧模型能力",
        ));
    };
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        return Ok(report(&id, "skipped", "绑定缺少设备凭据；模型未验证"));
    }
    let base = match provider_base(&binding.ccswitch.provider_id) {
        Ok(base) => base,
        Err(_) => {
            return Ok(report(
                &id,
                "skipped",
                "当前 provider 未配置本地 Responses 探测",
            ))
        }
    };
    let target = runtime_target.await;
    let version = target
        .as_ref()
        .map(|t| t.desktop_version.clone())
        .filter(|v| safe_identifier(v));
    if cancelled.load(AtomicOrdering::Acquire) {
        return Ok(report(&id, "cancelled", "探测已取消；未读取旧能力缓存"));
    }
    let mut context = binding_context(&binding, &base, version);
    let sidecar_path = home.join(SIDECAR_FILE_NAME);
    let mut expected_sidecar_bytes = read_optional(&sidecar_path)?;
    let cached = read_sidecar(&sidecar_path, &context)?;
    let mut result = report(&id, "discovered", "模型发现不等于 Code Agent 已验证");
    result.sidecar_path = Some(sidecar_path.to_string_lossy().into_owned());
    if native_requested {
        result.native_status = "pending".into();
        result.activation_status = "pending".into();
    }
    if let Some(sidecar) = cached.as_ref() {
        result.models = sidecar.models.clone();
        // Even a same-binding cached model ID must not echo the active secret.
        result.models.retain(|m| {
            !m.model_id.contains(api_key)
                && m.owned_by.as_ref().is_none_or(|v| !v.contains(api_key))
        });
        result.fetched_at = Some(format!("unix:{}", sidecar.fetched_at));
        if !force
            && !native_requested
            && !result.models.is_empty()
            && sidecar.fetched_at <= unix_seconds()
            && unix_seconds() - sidecar.fetched_at <= EVIDENCE_TTL_SECONDS
        {
            finalize_report(&mut result, &context, &[]);
            result.message =
                "使用同绑定的发现缓存；未验证原生 Shell、Skills 或 Code Agent 兼容性".into();
            return Ok(result);
        }
    }
    let start_new_auto_operation = native_verification
        .as_ref()
        .is_some_and(|options| options.start_new_operation);
    let persisted_auto_setup = match read_auto_setup_checkpoint(expected_sidecar_bytes.as_deref()) {
        Ok(checkpoint) => checkpoint,
        Err(_) if native_requested && start_new_auto_operation => None,
        Err(_) if native_requested => {
            mark_new_auto_operation_required(&mut result);
            return Ok(result);
        }
        Err(error) => return Err(error),
    };
    let setup_start_clock = if native_requested {
        match read_setup_clock() {
            Some(clock) => Some(clock),
            None => {
                result.status = "failed".into();
                result.native_status = "incomplete".into();
                result.activation_status = "not_eligible".into();
                result.error_kind = Some("auto-setup-clock-unavailable".into());
                result.message = "无法读取 Windows 单调时钟和启动标识；未发送模型生成请求".into();
                return Ok(result);
            }
        }
    } else {
        None
    };
    let started = Instant::now();
    let mut deadline = started
        + if native_requested {
            activation::AUTO_SETUP_BUDGET
        } else {
            REFRESH_BUDGET
        };
    if native_requested && !start_new_auto_operation {
        if let Some(checkpoint) = persisted_auto_setup.as_ref() {
            let clock = setup_start_clock
                .as_ref()
                .expect("auto setup clock was read");
            let now_ms = windows_tick_now().unwrap_or(clock.monotonic_ms);
            if !valid_auto_setup_checkpoint_shape(checkpoint)
                || checkpoint.boot_identity != clock.boot_identity
                || checkpoint.phase == AutoSetupPhase::ApplyPending
                || matches!(checkpoint.phase, AutoSetupPhase::Complete)
                || now_ms >= checkpoint.deadline_monotonic_ms
            {
                mark_new_auto_operation_required(&mut result);
                return Ok(result);
            }
            let remaining = checkpoint.deadline_monotonic_ms - now_ms;
            if remaining <= activation::AUTO_SETUP_APPLY_RESERVE.as_millis() as u64 {
                mark_new_auto_operation_required(&mut result);
                return Ok(result);
            }
            deadline = Instant::now() + Duration::from_millis(remaining);
        }
    }
    let discovery_deadline = if native_requested {
        deadline - activation::AUTO_SETUP_APPLY_RESERVE
    } else {
        started + REFRESH_BUDGET
    };
    let client = ProbeClient::new(&base, api_key, cancelled.clone(), discovery_deadline)
        .map_err(|_| engine("无法初始化模型探测客户端"))?
        .with_run_id(&id);
    let mut discovered = None;
    let mut discovery_error = ProbeError::Network;
    for attempt in 1..=3_u32 {
        result.attempts = attempt;
        let fetched = match client.health().await {
            Ok(()) => client.models().await,
            Err(e) => Err(e),
        };
        match fetched {
            Ok(value) => {
                discovered = Some(value);
                break;
            }
            Err(e) => {
                discovery_error = e;
                if !e.retryable()
                    || matches!(
                        e,
                        ProbeError::Cancelled
                            | ProbeError::Unauthorized
                            | ProbeError::BudgetExhausted
                    )
                {
                    break;
                }
            }
        }
        if attempt < 3 {
            tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
        }
    }
    let Some(payload) = discovered else {
        result.status = if discovery_error == ProbeError::Cancelled {
            "cancelled"
        } else if result.models.is_empty() {
            "failed"
        } else {
            "stale"
        }
        .into();
        result.gateway_status = if discovery_error == ProbeError::Unauthorized {
            "unauthorized"
        } else {
            "unavailable"
        }
        .into();
        result.error_kind = Some(discovery_error.code().into());
        result.message = "网关探测未完成；旧发现信息仅供诊断，可手动重试，安装状态不受影响".into();
        finalize_report(&mut result, &context, &[]);
        return Ok(result);
    };
    let source = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array);
    if source.is_none_or(|items| items.len() > 512) {
        return Ok(report(&id, "failed", "网关模型列表格式无效或超过数量限制"));
    }
    let certified_model_ids = if native_requested {
        let compatibility_payload = match client.codex_compatibility().await {
            Ok(value) => value,
            Err(error) => {
                result.status = "failed".into();
                result.gateway_status = if error == ProbeError::Unauthorized {
                    "unauthorized"
                } else {
                    "unavailable"
                }
                .into();
                result.activation_status = "not_eligible".into();
                result.error_kind = Some(error.code().into());
                result.message = "服务端认证目录不可用或未通过校验；未发送模型生成请求".into();
                finalize_report(&mut result, &context, &[]);
                return Ok(result);
            }
        };
        let manifest =
            match validate_compatibility_manifest(&compatibility_payload, &payload, api_key) {
                Ok(value) => value,
                Err(code) => {
                    result.status = "failed".into();
                    result.activation_status = "not_eligible".into();
                    result.error_kind = Some(code.into());
                    result.message =
                        "服务端认证目录与当前模型目录不一致；未发送模型生成请求".into();
                    finalize_report(&mut result, &context, &[]);
                    return Ok(result);
                }
            };
        let certified_ids = manifest.model_priority.clone();
        let binding_data = serde_json::to_vec(&(
            manifest.service_revision.as_str(),
            manifest.profile_id.as_str(),
            manifest.model_priority.as_slice(),
            manifest
                .models
                .iter()
                .map(|model| model.certification_id.as_str())
                .collect::<Vec<_>>(),
        ))
        .unwrap_or_default();
        context.adapter_version = Some(format!(
            "codex-compatibility:{}:{}",
            manifest.service_revision,
            digest_hex(&binding_data),
        ));
        result.service_revision = Some(manifest.service_revision);
        certified_ids
    } else {
        Vec::new()
    };
    let normalized = normalize_models(parse_gateway_models(&payload));
    result.models = normalized
        .into_iter()
        .filter(|m| {
            !m.raw_id.contains(api_key) && m.owned_by.as_ref().is_none_or(|v| !v.contains(api_key))
        })
        .map(|m| {
            let mut model = ModelCapabilityReport::discovered(m.raw_id, m.owned_by);
            if let Some(old) = cached.as_ref().and_then(|cache| {
                cache
                    .models
                    .iter()
                    .find(|old| old.model_id == model.model_id && old.owned_by == model.owned_by)
            }) {
                model.checked_at = old.checked_at;
            }
            model
        })
        .collect();
    if native_requested {
        result
            .models
            .retain(|model| certified_model_ids.iter().any(|id| id == &model.model_id));
        result.models.sort_by_key(|model| {
            certified_model_ids
                .iter()
                .position(|id| id == &model.model_id)
                .unwrap_or(3)
        });
        if result.models.iter().any(|model| {
            result
                .models
                .iter()
                .filter(|candidate| candidate.model_id == model.model_id)
                .count()
                != 1
        }) || certified_model_ids
            .iter()
            .any(|id| !result.models.iter().any(|model| &model.model_id == id))
        {
            result.status = "failed".into();
            result.activation_status = "not_eligible".into();
            result.error_kind = Some("certified-model-catalog-mismatch".into());
            result.message =
                "认证模型目录未与当前 group 模型 ID 唯一匹配；未发送模型生成请求".into();
            finalize_report(&mut result, &context, &[]);
            return Ok(result);
        }
    } else {
        // Retry older or previously unvisited entries first; a large catalog
        // cannot starve later models by restarting at index zero every time.
        result.models.sort_by(|a, b| {
            a.checked_at
                .cmp(&b.checked_at)
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
    }
    let mut auto_checkpoint = persisted_auto_setup.clone();
    if native_requested {
        let start_clock = setup_start_clock
            .as_ref()
            .expect("auto setup clock was read");
        let now_clock = read_setup_clock()
            .ok_or_else(|| engine("无法复核 Windows 单调时钟和启动标识；未发送模型生成请求"))?;
        let service_revision = result
            .service_revision
            .as_deref()
            .ok_or_else(|| engine("认证目录缺少服务 revision；未发送模型生成请求"))?;
        let scope_material = serde_json::to_vec(&(
            context.binding_fingerprint.as_str(),
            context.adapter_version.as_deref(),
            service_revision,
            certified_model_ids.as_slice(),
        ))
        .unwrap_or_default();
        let authorization_scope_sha256 = digest_hex(&scope_material);
        let mut checkpoint = if start_new_auto_operation || persisted_auto_setup.is_none() {
            new_auto_setup_checkpoint(
                &id,
                &certified_model_ids,
                service_revision,
                &authorization_scope_sha256,
                start_clock,
            )?
        } else {
            let mut previous = persisted_auto_setup
                .clone()
                .expect("checkpoint was checked");
            let can_resume = auto_setup_checkpoint_matches_resume(
                &previous,
                &now_clock,
                service_revision,
                &authorization_scope_sha256,
                &certified_model_ids,
            );
            if !can_resume {
                mark_new_auto_operation_required(&mut result);
                return Ok(result);
            }
            mark_prior_candidate_results_untrusted(&mut previous);
            let remaining_ms = previous.deadline_monotonic_ms - now_clock.monotonic_ms;
            if !has_untouched_auto_candidates(&previous)
                || remaining_ms <= activation::AUTO_SETUP_APPLY_RESERVE.as_millis() as u64
            {
                mark_new_auto_operation_required(&mut result);
                return Ok(result);
            }
            deadline = Instant::now() + Duration::from_millis(remaining_ms);
            previous
        };
        checkpoint.phase = AutoSetupPhase::GatewayVerification;
        persist_auto_setup_checkpoint(
            &sidecar_path,
            &mut expected_sidecar_bytes,
            &context,
            &result.models,
            &checkpoint,
            &binding,
            &home,
        )?;
        auto_checkpoint = Some(checkpoint);
    }
    result.gateway_status = "connected".into();
    let mut raw_id_counts = BTreeMap::new();
    for model in &result.models {
        *raw_id_counts
            .entry(model.model_id.clone())
            .or_insert(0usize) += 1;
    }
    let mut candidate_deadlines = BTreeMap::new();
    for index in 0..result.models.len() {
        let model_id = result.models[index].model_id.clone();
        if raw_id_counts.get(&model_id).copied().unwrap_or_default() != 1 {
            result.models[index].error_code = Some("ambiguous-model-owner".into());
            continue;
        }
        // Ordinary refresh is metadata-only. One-click setup probes only the
        // exact candidates independently certified by the server manifest.
        if !should_probe_gateway_model(native_requested, &certified_model_ids, &model_id) {
            continue;
        }
        if native_requested
            && auto_checkpoint.as_ref().is_some_and(|checkpoint| {
                checkpoint.candidates.iter().any(|candidate| {
                    candidate.model_id == model_id
                        && candidate.status != AutoSetupCandidateStatus::Untouched
                })
            })
        {
            continue;
        }
        if cancelled.load(AtomicOrdering::Acquire) {
            break;
        }
        let candidate_started = Instant::now();
        let candidate_deadline = (candidate_started + activation::NATIVE_REFRESH_BUDGET)
            .min(deadline - activation::AUTO_SETUP_APPLY_RESERVE);
        candidate_deadlines.insert(model_id.clone(), candidate_deadline);
        if let Some(checkpoint) = auto_checkpoint.as_mut() {
            if let Some(candidate) = checkpoint
                .candidates
                .iter_mut()
                .find(|candidate| candidate.model_id == model_id)
            {
                candidate.status = AutoSetupCandidateStatus::Inflight;
                candidate.result_code = Some("probe-inflight".into());
            }
            persist_auto_setup_checkpoint(
                &sidecar_path,
                &mut expected_sidecar_bytes,
                &context,
                &result.models,
                checkpoint,
                &binding,
                &home,
            )?;
        }
        if Instant::now() >= candidate_deadline {
            result.models[index].error_code = Some(ProbeError::BudgetExhausted.code().into());
            if let Some(checkpoint) = auto_checkpoint.as_mut() {
                if let Some(candidate) = checkpoint
                    .candidates
                    .iter_mut()
                    .find(|candidate| candidate.model_id == model_id)
                {
                    candidate.status = AutoSetupCandidateStatus::Failed;
                    candidate.result_code = Some("deadline-exhausted".into());
                }
                persist_auto_setup_checkpoint(
                    &sidecar_path,
                    &mut expected_sidecar_bytes,
                    &context,
                    &result.models,
                    checkpoint,
                    &binding,
                    &home,
                )?;
            }
            continue;
        }
        let candidate_client =
            match ProbeClient::new(&base, api_key, cancelled.clone(), candidate_deadline) {
                Ok(client) => client.with_run_id(&id),
                Err(error) => {
                    result.models[index].error_code = Some(error.code().into());
                    if let Some(checkpoint) = auto_checkpoint.as_mut() {
                        if let Some(candidate) = checkpoint
                            .candidates
                            .iter_mut()
                            .find(|candidate| candidate.model_id == model_id)
                        {
                            candidate.status = if error == ProbeError::Cancelled {
                                AutoSetupCandidateStatus::Uncertain
                            } else {
                                AutoSetupCandidateStatus::Failed
                            };
                            candidate.result_code = Some(
                                if error == ProbeError::Cancelled {
                                    "cancelled"
                                } else {
                                    "gateway-incomplete"
                                }
                                .into(),
                            );
                        }
                        persist_auto_setup_checkpoint(
                            &sidecar_path,
                            &mut expected_sidecar_bytes,
                            &context,
                            &result.models,
                            checkpoint,
                            &binding,
                            &home,
                        )?;
                    }
                    if error == ProbeError::Cancelled {
                        break;
                    }
                    continue;
                }
            };
        if let Err(error) = candidate_client.check() {
            result.models[index].error_code = Some(error.code().into());
            if let Some(checkpoint) = auto_checkpoint.as_mut() {
                if let Some(candidate) = checkpoint
                    .candidates
                    .iter_mut()
                    .find(|candidate| candidate.model_id == model_id)
                {
                    candidate.status = if error == ProbeError::Cancelled {
                        AutoSetupCandidateStatus::Uncertain
                    } else {
                        AutoSetupCandidateStatus::Failed
                    };
                    candidate.result_code = Some(
                        if error == ProbeError::Cancelled {
                            "cancelled"
                        } else {
                            "gateway-incomplete"
                        }
                        .into(),
                    );
                }
                persist_auto_setup_checkpoint(
                    &sidecar_path,
                    &mut expected_sidecar_bytes,
                    &context,
                    &result.models,
                    checkpoint,
                    &binding,
                    &home,
                )?;
            }
            if error == ProbeError::Cancelled {
                break;
            }
            continue;
        }
        candidate_client
            .probe_model(&mut result.models[index], unix_seconds())
            .await;
        let complete = gateway_matrix_complete(&result.models[index]);
        if let Some(checkpoint) = auto_checkpoint.as_mut() {
            if let Some(candidate) = checkpoint
                .candidates
                .iter_mut()
                .find(|candidate| candidate.model_id == model_id)
            {
                candidate.status = if complete {
                    AutoSetupCandidateStatus::GatewayComplete
                } else {
                    AutoSetupCandidateStatus::Failed
                };
                candidate.result_code = Some(
                    if complete {
                        "gateway-complete"
                    } else {
                        "gateway-incomplete"
                    }
                    .into(),
                );
            }
            persist_auto_setup_checkpoint(
                &sidecar_path,
                &mut expected_sidecar_bytes,
                &context,
                &result.models,
                checkpoint,
                &binding,
                &home,
            )?;
        }
        if cancelled.load(AtomicOrdering::Acquire) {
            break;
        }
    }
    drop(client);
    if let Some(checkpoint) = auto_checkpoint.as_mut() {
        checkpoint.phase = AutoSetupPhase::NativeVerification;
        persist_auto_setup_checkpoint(
            &sidecar_path,
            &mut expected_sidecar_bytes,
            &context,
            &result.models,
            checkpoint,
            &binding,
            &home,
        )?;
    }
    let native = if native_requested {
        activation::collect(
            &mut result.models,
            &context,
            target.as_ref(),
            &binding,
            &base,
            &home,
            &id,
            &candidate_deadlines,
            cancelled.clone(),
            deadline,
        )
        .await
    } else {
        activation::NativeBatch::empty()
    };
    if let Some(first) = native.results.first() {
        context.native_runtime_version = Some(first.runtime_version.clone());
    }
    let proofs = native.proofs();
    finalize_report(&mut result, &context, &proofs);
    if native_requested {
        result.native_status = if result.ready_count > 0 {
            "verified"
        } else {
            "incomplete"
        }
        .into();
        if let Some(checkpoint) = auto_checkpoint.as_mut() {
            for candidate in &mut checkpoint.candidates {
                if candidate.status != AutoSetupCandidateStatus::GatewayComplete {
                    continue;
                }
                let ready = result.models.iter().any(|model| {
                    model.model_id == candidate.model_id && model.state == ModelState::Ready
                });
                candidate.status = if ready {
                    AutoSetupCandidateStatus::NativeComplete
                } else {
                    AutoSetupCandidateStatus::Failed
                };
                candidate.result_code = Some(
                    if ready {
                        "native-complete"
                    } else {
                        "native-incomplete"
                    }
                    .into(),
                );
            }
            let is_cancelled = cancelled.load(AtomicOrdering::Acquire);
            checkpoint.phase = if result.ready_count > 0 && !is_cancelled {
                AutoSetupPhase::ApplyPending
            } else {
                AutoSetupPhase::Incomplete
            };
            persist_auto_setup_checkpoint(
                &sidecar_path,
                &mut expected_sidecar_bytes,
                &context,
                &result.models,
                checkpoint,
                &binding,
                &home,
            )?;
        }
    }
    if cancelled.load(AtomicOrdering::Acquire) {
        result.status = "cancelled".into();
        result.error_kind = Some("probe-cancelled".into());
        result.message = "探测已取消；未写入迟到结果".into();
        return Ok(result);
    }
    // All final writes run on one blocking worker. Cancellation waits for the
    // same commit boundary and cannot claim to undo an acknowledged commit.
    let mut auto_setup_state = if native_requested {
        auto_checkpoint
    } else {
        persisted_auto_setup
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _files = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let outcome = (|| -> Result<NativeModelCatalogReport, AppError> {
            let current = usb_guard::authorized_account_binding()?;
            if current.as_ref() != Some(&binding)
                || codex_home_dir().as_ref() != Some(&home)
                || cancelled.load(AtomicOrdering::Acquire) {
                return Ok(report(&id, "cancelled", "授权或用户目录已变化；未写入旧请求结果"));
            }
            if read_optional(&sidecar_path)? != expected_sidecar_bytes {
                return Err(engine("能力文件已被其他进程修改，本次结果未覆盖；请重试"));
            }
            let backup = atomic_file::backup_path(&sidecar_path);
            regular_or_absent(&backup)?;
            if backup.exists() && !owned_json(&backup, SIDECAR_IDENTITY) {
                return Err(engine("能力备份不归本管理器所有，未覆盖"));
            }
            let observed = unix_seconds();
            let sidecar = CapabilitySidecar {
                schema_version: if auto_setup_state.is_some() { 3 } else { 2 },
                identity: SIDECAR_IDENTITY.into(),
                context: context.clone(),
                fetched_at: observed,
                models: result.models.clone(),
                auto_setup: auto_setup_state.clone(),
            };
            let bytes = serde_json::to_vec_pretty(&sidecar).map_err(|_| engine("能力结果序列化失败"))?;
            if bytes.len() > BODY_LIMIT { return Err(engine("能力结果超过本地大小限制")); }
            atomic_file::write_atomic(&sidecar_path, &bytes).map_err(|_| engine("能力结果写入失败，可重试"))?;
            expected_sidecar_bytes = Some(bytes);
            result.fetched_at = Some(format!("unix:{observed}"));
            result.status = if result.models.iter().any(|m| m.error_code.is_some()) { "partial" } else { "discovered" }.into();
            if native_requested {
                result.activation_status = "not_eligible".into();
                result.error_kind = Some(native.error.unwrap_or("native-verification-incomplete").into());
            } else {
                result.error_kind = Some("native_verification_not_requested".into());
            }
            complete_auto_setup_after_apply(
                result.activation_status == "applied",
                auto_setup_state.as_mut(),
                result.default_model.as_deref(),
                &context,
                observed,
                &result.models,
                &mut expected_sidecar_bytes,
                || read_optional(&sidecar_path),
                |bytes| atomic_file::write_atomic(&sidecar_path, bytes)
                    .map_err(|_| engine("最终检查点原子写入失败")),
                || activation::confirm_setup_complete(&home),
            )?;
            result.message = if native_requested {
                format!("已对服务端认证候选执行原生验证；本次目录发现 {} 个，原生验证通过 {} 个，测试未通过 {} 个；应用状态：{}。",
                    result.model_count, result.ready_count, result.quarantined_count, result.activation_status)
            } else {
                format!("网关已连接，发现 {} 个模型；尚未进行 Codex 原生验证。", result.model_count)
            };
            Ok(result)
        })();
        RUNS.lock().unwrap_or_else(|e| e.into_inner()).finish(&id);
        outcome
    }).await.map_err(|_| engine("模型结果提交任务未完成"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tested_catalog_entry_preserves_exact_id_and_claims_only_text_input() {
        let entry = conservative_tested_model_entry("vendor/model-latest", Some("vendor"), 3);
        assert_eq!(entry["slug"], "vendor/model-latest");
        assert_eq!(entry["input_modalities"], json!(["text"]));
        assert_eq!(entry["supports_search_tool"], false);
        assert_eq!(entry["supported_reasoning_levels"], json!([]));
        assert!(entry.get("context_window").is_none());
        assert!(entry.get("tool_mode").is_none());
        assert!(!entry.to_string().contains("image\""));
    }

    #[test]
    fn external_model_catalog_override_and_invalid_toml_fail_closed_semantically() {
        let config = std::env::temp_dir()
            .join("codex-selective-test")
            .join("config.toml");
        let external = "model = \"keep-me\"\n\"model_catalog_json\" = \"user.json\"\n";
        assert!(cleaned_config_without_external_catalog(external, &config).is_err());
        assert!(cleaned_config_without_external_catalog("model = [\n", &config).is_err());
        let unrelated_table = "[other]\nmodel_catalog_json = \"not-the-root-setting\"\n";
        assert!(cleaned_config_without_external_catalog(unrelated_table, &config).is_ok());
    }

    #[test]
    fn owned_catalog_reference_is_removed_without_changing_other_toml() {
        let config = std::env::temp_dir()
            .join("codex-selective-test")
            .join("config.toml");
        let expected = expected_catalog_reference(&config).unwrap();
        let original = format!(
            "model = \"keep-me\"\n\n{CONFIG_BEGIN}\n{expected}\n{CONFIG_END}\n\n[other]\nvalue = 7\n"
        );
        let (cleaned, managed) =
            cleaned_config_without_external_catalog(&original, &config).unwrap();
        assert!(managed);
        assert_eq!(cleaned, "model = \"keep-me\"\n\n\n[other]\nvalue = 7\n");
    }

    struct CatalogTempHome(std::path::PathBuf);

    impl CatalogTempHome {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "codex-selective-catalog-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for CatalogTempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn explicit_add_and_reset_preserve_user_config_and_its_backup() {
        let home = CatalogTempHome::new();
        let config = config_path_for(home.path());
        let config_backup = atomic_file::backup_path(&config);
        let original = b"model = \"keep-me\"\n[other]\nvalue = 7\n";
        let user_backup = b"older user recovery point\n";
        std::fs::write(&config, original).unwrap();
        std::fs::write(&config_backup, user_backup).unwrap();
        let cancelled = AtomicBool::new(false);

        let added = add_tested_model_files(
            home.path(),
            "vendor/model-latest",
            Some("vendor"),
            &cancelled,
            || Ok(true),
            || {},
            || Ok(()),
        )
        .unwrap();
        assert_eq!(added.status, "applied_restart_pending");
        assert!(std::fs::read_to_string(&config)
            .unwrap()
            .contains("model = \"keep-me\""));
        assert_eq!(std::fs::read(&config_backup).unwrap(), user_backup);
        assert_eq!(
            catalog_state_for_home(home.path()).unwrap().model_ids,
            ["vendor/model-latest"]
        );

        let reset = reset_owned_catalog_at(home.path(), || Ok(())).unwrap();
        assert_eq!(reset.status, "applied_restart_pending");
        let reset_config = std::fs::read_to_string(&config).unwrap();
        assert_eq!(
            toml::from_str::<toml::Value>(&reset_config).unwrap(),
            toml::from_str::<toml::Value>(std::str::from_utf8(original).unwrap()).unwrap()
        );
        assert!(!reset_config.contains("model_catalog_json"));
        assert_eq!(std::fs::read(&config_backup).unwrap(), user_backup);
        assert!(!catalog_path_for(home.path()).exists());
        assert!(catalog_state_for_home(home.path())
            .unwrap()
            .model_ids
            .is_empty());
    }

    #[test]
    fn catalog_state_marks_unreferenced_owned_json_as_orphaned() {
        let home = CatalogTempHome::new();
        let catalog = json!({
            "identity": CATALOG_IDENTITY,
            "models": [conservative_tested_model_entry("vendor/model", None, 1)]
        });
        std::fs::write(
            catalog_path_for(home.path()),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();

        let state = catalog_state_for_home(home.path()).unwrap();
        assert_eq!(state.status, "orphaned");
        assert!(state.model_ids.is_empty());
        assert!(state.message.contains("请复位"));
    }

    #[test]
    fn add_and_reset_fail_closed_on_external_catalog_override_without_mutation() {
        let home = CatalogTempHome::new();
        let config = config_path_for(home.path());
        let original = b"model = \"keep-me\"\nmodel_catalog_json = \"user-catalog.json\"\n";
        std::fs::write(&config, original).unwrap();
        let cancelled = AtomicBool::new(false);

        assert!(add_tested_model_files(
            home.path(),
            "vendor/model",
            None,
            &cancelled,
            || Ok(true),
            || {},
            || Ok(()),
        )
        .is_err());
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert!(!catalog_path_for(home.path()).exists());

        let owned_catalog = json!({
            "identity": CATALOG_IDENTITY,
            "models": [conservative_tested_model_entry("vendor/model", None, 1)]
        });
        std::fs::write(
            catalog_path_for(home.path()),
            serde_json::to_vec(&owned_catalog).unwrap(),
        )
        .unwrap();
        assert!(reset_owned_catalog_at(home.path(), || Ok(())).is_err());
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert!(catalog_path_for(home.path()).exists());
    }

    #[test]
    fn cancelled_add_does_not_cross_the_catalog_commit_boundary() {
        let home = CatalogTempHome::new();
        let config = config_path_for(home.path());
        let original = b"model = \"unchanged\"\n";
        std::fs::write(&config, original).unwrap();
        let cancelled = AtomicBool::new(true);
        let committed = AtomicBool::new(false);

        let result = add_tested_model_files(
            home.path(),
            "vendor/model",
            None,
            &cancelled,
            || Ok(true),
            || committed.store(true, AtomicOrdering::Release),
            || Ok(()),
        );
        assert!(result.is_err());
        assert!(!committed.load(AtomicOrdering::Acquire));
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert!(!catalog_path_for(home.path()).exists());
    }

    #[test]
    fn cancellation_after_final_precommit_check_leaves_files_untouched() {
        use std::cell::Cell;

        let home = CatalogTempHome::new();
        let config = config_path_for(home.path());
        let original = b"model = \"unchanged\"\n";
        std::fs::write(&config, original).unwrap();
        let cancelled = AtomicBool::new(false);
        let checks = Cell::new(0);
        let committed = AtomicBool::new(false);

        let result = add_tested_model_files(
            home.path(),
            "vendor/model",
            None,
            &cancelled,
            || {
                let count = checks.get() + 1;
                checks.set(count);
                if count == 2 {
                    cancelled.store(true, AtomicOrdering::Release);
                }
                Ok(true)
            },
            || committed.store(true, AtomicOrdering::Release),
            || Ok(()),
        );

        assert!(result.is_err());
        assert!(!committed.load(AtomicOrdering::Acquire));
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert!(!catalog_path_for(home.path()).exists());
    }

    #[test]
    fn cancellation_after_commit_boundary_keeps_applied_catalog_state() {
        let home = CatalogTempHome::new();
        let config = config_path_for(home.path());
        std::fs::write(&config, b"model = \"unchanged\"\n").unwrap();
        let cancelled = AtomicBool::new(false);

        let result = add_tested_model_files(
            home.path(),
            "vendor/model",
            None,
            &cancelled,
            || Ok(true),
            || cancelled.store(true, AtomicOrdering::Release),
            || Ok(()),
        )
        .unwrap();

        assert!(cancelled.load(AtomicOrdering::Acquire));
        assert_eq!(result.status, "applied_restart_pending");
        assert_eq!(result.model_ids, ["vendor/model"]);
        assert_eq!(
            catalog_state_for_home(home.path()).unwrap().model_ids,
            ["vendor/model"]
        );
    }

    #[tokio::test]
    async fn catalog_mutation_lock_serializes_add_before_queued_reset() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let held = lock.lock().await;
        let order = Arc::new(Mutex::new(Vec::new()));

        let (add_ready_tx, add_ready_rx) = tokio::sync::oneshot::channel();
        let add_lock = lock.clone();
        let add_order = order.clone();
        let add = tokio::spawn(async move {
            add_ready_tx.send(()).unwrap();
            with_catalog_mutation_lock(&add_lock, async {
                add_order.lock().unwrap().push("add");
                tokio::task::yield_now().await;
            })
            .await;
        });
        add_ready_rx.await.unwrap();

        let (reset_ready_tx, reset_ready_rx) = tokio::sync::oneshot::channel();
        let reset_lock = lock.clone();
        let reset_order = order.clone();
        let reset = tokio::spawn(async move {
            reset_ready_tx.send(()).unwrap();
            with_catalog_mutation_lock(&reset_lock, async {
                reset_order.lock().unwrap().push("reset");
            })
            .await;
        });
        reset_ready_rx.await.unwrap();
        drop(held);

        add.await.unwrap();
        reset.await.unwrap();
        assert_eq!(*order.lock().unwrap(), ["add", "reset"]);
    }

    #[tokio::test]
    async fn catalog_mutation_lock_reset_first_rejects_queued_add_without_writes() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let held = lock.lock().await;
        let reset_finished = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(AtomicBool::new(false));
        let order = Arc::new(Mutex::new(Vec::new()));

        let (reset_ready_tx, reset_ready_rx) = tokio::sync::oneshot::channel();
        let reset_lock = lock.clone();
        let reset_done = reset_finished.clone();
        let reset_order = order.clone();
        let reset = tokio::spawn(async move {
            reset_ready_tx.send(()).unwrap();
            with_catalog_mutation_lock(&reset_lock, async {
                reset_order.lock().unwrap().push("reset");
                reset_done.store(true, AtomicOrdering::Release);
            })
            .await;
        });
        reset_ready_rx.await.unwrap();

        let (add_ready_tx, add_ready_rx) = tokio::sync::oneshot::channel();
        let add_lock = lock.clone();
        let add_reset_done = reset_finished.clone();
        let add_writes = writes.clone();
        let add_order = order.clone();
        let add = tokio::spawn(async move {
            add_ready_tx.send(()).unwrap();
            with_catalog_mutation_lock(&add_lock, async {
                if add_reset_done.load(AtomicOrdering::Acquire) {
                    add_order.lock().unwrap().push("add-rejected");
                } else {
                    add_writes.store(true, AtomicOrdering::Release);
                    add_order.lock().unwrap().push("add-write");
                }
            })
            .await;
        });
        add_ready_rx.await.unwrap();
        drop(held);

        reset.await.unwrap();
        add.await.unwrap();
        assert_eq!(*order.lock().unwrap(), ["reset", "add-rejected"]);
        assert!(!writes.load(AtomicOrdering::Acquire));
    }

    #[test]
    fn generation_probes_are_limited_to_server_certified_models() {
        let certified = vec!["glm-5.2".into(), "deepseek-v4-flash".into()];
        assert!(!should_probe_gateway_model(false, &certified, "glm-5.2"));
        assert!(!should_probe_gateway_model(
            true,
            &certified,
            "unverified-model"
        ));
        assert!(should_probe_gateway_model(true, &certified, "glm-5.2"));
        assert!(should_probe_gateway_model(
            true,
            &certified,
            "deepseek-v4-flash"
        ));
    }

    #[test]
    fn complete_setup_proof_is_confirmed_only_after_checkpoint_write() {
        use std::cell::Cell;
        use std::cell::RefCell;

        let bytes = br#"{"autoSetup":{"phase":"complete"}}"#;
        let persisted = RefCell::new(Some(b"inflight-checkpoint".to_vec()));
        let confirmed = Cell::new(false);
        commit_completed_auto_setup_checkpoint(
            &Some(b"inflight-checkpoint".to_vec()),
            bytes,
            || Ok(persisted.borrow().clone()),
            |written_bytes| {
                assert_eq!(written_bytes, bytes);
                *persisted.borrow_mut() = Some(written_bytes.to_vec());
                Ok(())
            },
            || {
                assert_eq!(persisted.borrow().as_deref(), Some(bytes.as_slice()));
                confirmed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(confirmed.get());

        let confirmed_after_failed_write = Cell::new(false);
        let failed = commit_completed_auto_setup_checkpoint(
            &Some(b"inflight-checkpoint".to_vec()),
            bytes,
            || Ok(Some(b"inflight-checkpoint".to_vec())),
            |_| Err(engine("injected checkpoint write failure")),
            || {
                confirmed_after_failed_write.set(true);
                Ok(())
            },
        );
        assert!(failed.is_err());
        assert!(!confirmed_after_failed_write.get());

        let confirmed_after_oversized_checkpoint = Cell::new(false);
        let oversized = vec![0; BODY_LIMIT + 1];
        assert!(commit_completed_auto_setup_checkpoint(
            &Some(b"inflight-checkpoint".to_vec()),
            &oversized,
            || panic!("oversized checkpoint must not read or write"),
            |_| panic!("oversized checkpoint must not be written"),
            || {
                confirmed_after_oversized_checkpoint.set(true);
                Ok(())
            },
        )
        .is_err());
        assert!(!confirmed_after_oversized_checkpoint.get());

        let wrote_conflict = Cell::new(false);
        let confirmed_after_conflict = Cell::new(false);
        let conflict = commit_completed_auto_setup_checkpoint(
            &Some(b"inflight-checkpoint".to_vec()),
            bytes,
            || Ok(Some(b"externally-modified".to_vec())),
            |_| {
                wrote_conflict.set(true);
                Ok(())
            },
            || {
                confirmed_after_conflict.set(true);
                Ok(())
            },
        );
        assert!(conflict.is_err());
        assert!(!wrote_conflict.get());
        assert!(!confirmed_after_conflict.get());

        let readback_count = Cell::new(0);
        let confirmed_after_bad_readback = Cell::new(false);
        let bad_readback = commit_completed_auto_setup_checkpoint(
            &Some(b"inflight-checkpoint".to_vec()),
            bytes,
            || {
                let count = readback_count.get() + 1;
                readback_count.set(count);
                Ok(Some(if count == 1 {
                    b"inflight-checkpoint".to_vec()
                } else {
                    b"external-rewrite".to_vec()
                }))
            },
            |_| Ok(()),
            || {
                confirmed_after_bad_readback.set(true);
                Ok(())
            },
        );
        assert!(bad_readback.is_err());
        assert_eq!(readback_count.get(), 2);
        assert!(!confirmed_after_bad_readback.get());
    }

    #[test]
    fn publish_completion_requires_applied_native_candidate_and_complete_checkpoint_readback() {
        use std::cell::{Cell, RefCell};

        let mut checkpoint = checkpoint_fixture();
        checkpoint.phase = AutoSetupPhase::ApplyPending;
        checkpoint.candidates[0].status = AutoSetupCandidateStatus::NativeComplete;
        checkpoint.candidates[0].result_code = Some("native-complete".into());
        let context = CapabilityContext {
            binding_fingerprint: "fixture-binding".into(),
            codex_version: Some("fixture-codex".into()),
            native_runtime_version: Some("fixture-runtime".into()),
            adapter_version: Some("fixture-adapter".into()),
        };
        let models = vec![ModelCapabilityReport::discovered(
            "glm-5.2".into(),
            Some("zhipu".into()),
        )];
        let expected_bytes = b"apply-pending-checkpoint".to_vec();
        let mut expected = Some(expected_bytes.clone());
        let persisted = RefCell::new(Some(expected_bytes.clone()));
        let confirmed = Cell::new(false);

        complete_auto_setup_after_apply(
            true,
            Some(&mut checkpoint),
            Some("glm-5.2"),
            &context,
            1_790_352_001,
            &models,
            &mut expected,
            || Ok(persisted.borrow().clone()),
            |bytes| {
                let sidecar: Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(sidecar["autoSetup"]["phase"], "complete");
                assert_eq!(
                    sidecar["autoSetup"]["candidates"][0]["resultCode"],
                    "apply-complete"
                );
                *persisted.borrow_mut() = Some(bytes.to_vec());
                Ok(())
            },
            || {
                let sidecar: Value = serde_json::from_slice(
                    persisted
                        .borrow()
                        .as_deref()
                        .expect("complete sidecar must be persisted"),
                )
                .unwrap();
                assert_eq!(sidecar["autoSetup"]["phase"], "complete");
                confirmed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(confirmed.get());
        assert_eq!(checkpoint.phase, AutoSetupPhase::Complete);
        assert_eq!(expected, *persisted.borrow());

        let mut not_applied = checkpoint_fixture();
        let mut not_applied_expected = Some(expected_bytes.clone());
        let wrote_when_not_applied = Cell::new(false);
        let confirmed_when_not_applied = Cell::new(false);
        complete_auto_setup_after_apply(
            false,
            Some(&mut not_applied),
            None,
            &context,
            1_790_352_001,
            &models,
            &mut not_applied_expected,
            || panic!("an unapplied result must not inspect a completion checkpoint"),
            |_| {
                wrote_when_not_applied.set(true);
                Ok(())
            },
            || {
                confirmed_when_not_applied.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(not_applied.phase, AutoSetupPhase::GatewayVerification);
        assert!(!wrote_when_not_applied.get());
        assert!(!confirmed_when_not_applied.get());

        let mut missing_candidate = checkpoint_fixture();
        missing_candidate.phase = AutoSetupPhase::ApplyPending;
        missing_candidate.candidates[0].status = AutoSetupCandidateStatus::NativeComplete;
        let missing_result = complete_auto_setup_after_apply(
            true,
            Some(&mut missing_candidate),
            Some("unlisted-model"),
            &context,
            1_790_352_001,
            &models,
            &mut not_applied_expected,
            || panic!("a missing candidate must fail before checkpoint I/O"),
            |_| panic!("a missing candidate must not be written"),
            || panic!("a missing candidate must not confirm activation"),
        );
        assert!(missing_result.is_err());
        assert_eq!(missing_candidate.phase, AutoSetupPhase::ApplyPending);

        let missing_checkpoint = complete_auto_setup_after_apply(
            true,
            None,
            Some("glm-5.2"),
            &context,
            1_790_352_001,
            &models,
            &mut not_applied_expected,
            || panic!("a missing checkpoint must fail before I/O"),
            |_| panic!("a missing checkpoint must not be written"),
            || panic!("a missing checkpoint must not confirm activation"),
        );
        assert!(missing_checkpoint.is_err());

        let mut proof_rejected = checkpoint_fixture();
        proof_rejected.phase = AutoSetupPhase::ApplyPending;
        proof_rejected.candidates[0].status = AutoSetupCandidateStatus::NativeComplete;
        let mut proof_rejected_expected = Some(expected_bytes.clone());
        let proof_rejected_persisted = RefCell::new(Some(expected_bytes));
        let proof_rejected_result = complete_auto_setup_after_apply(
            true,
            Some(&mut proof_rejected),
            Some("glm-5.2"),
            &context,
            1_790_352_001,
            &models,
            &mut proof_rejected_expected,
            || Ok(proof_rejected_persisted.borrow().clone()),
            |bytes| {
                *proof_rejected_persisted.borrow_mut() = Some(bytes.to_vec());
                Ok(())
            },
            || Err("synthetic-live-proof-drift"),
        );
        assert!(proof_rejected_result.is_err());
        assert_eq!(
            proof_rejected_expected.as_deref(),
            Some(b"apply-pending-checkpoint".as_slice())
        );
        let persisted_sidecar: Value =
            serde_json::from_slice(proof_rejected_persisted.borrow().as_deref().unwrap()).unwrap();
        assert_eq!(persisted_sidecar["autoSetup"]["phase"], "complete");
    }

    fn checkpoint_fixture() -> AutoSetupCheckpoint {
        AutoSetupCheckpoint {
            operation_id: "operation-fixture-1".into(),
            phase: AutoSetupPhase::GatewayVerification,
            service_revision: "c4d48cf753c62f166d848c06c1fc25a23430316d".into(),
            authorization_scope_sha256: "a".repeat(64),
            started_at_utc: "2026-09-26T00:00:00Z".into(),
            deadline_utc: "2026-09-26T00:32:00Z".into(),
            monotonic_clock_source: AUTO_SETUP_CLOCK_SOURCE.into(),
            boot_identity_source: AUTO_SETUP_BOOT_SOURCE.into(),
            boot_identity: "2026-09-25T12:00:00.0000000Z".into(),
            started_monotonic_ms: 1_000,
            deadline_monotonic_ms: 1_000 + activation::AUTO_SETUP_BUDGET.as_millis() as u64,
            candidates: ["glm-5.2", "deepseek-v4-flash", "claude-fable-5"]
                .into_iter()
                .enumerate()
                .map(|(index, model_id)| AutoSetupCandidate {
                    model_id: model_id.into(),
                    status: if index == 0 {
                        AutoSetupCandidateStatus::Inflight
                    } else {
                        AutoSetupCandidateStatus::Untouched
                    },
                    result_code: if index == 0 {
                        Some("probe-inflight".into())
                    } else {
                        None
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn resume_checkpoint_keeps_only_untouched_candidates_and_never_imports_old_results() {
        let mut checkpoint = checkpoint_fixture();
        let now = SetupClock {
            boot_identity: checkpoint.boot_identity.clone(),
            monotonic_ms: 2_000,
            utc_now: "2026-09-26T00:00:01Z".into(),
            unix_now: 1_790_352_001,
        };
        let candidates = checkpoint
            .candidates
            .iter()
            .map(|candidate| candidate.model_id.clone())
            .collect::<Vec<_>>();
        assert!(valid_auto_setup_checkpoint_shape(&checkpoint));
        assert!(auto_setup_checkpoint_matches_resume(
            &checkpoint,
            &now,
            &checkpoint.service_revision,
            &checkpoint.authorization_scope_sha256,
            &candidates,
        ));
        mark_prior_candidate_results_untrusted(&mut checkpoint);
        assert_eq!(
            checkpoint.candidates[0].status,
            AutoSetupCandidateStatus::Uncertain
        );
        assert_eq!(
            checkpoint.candidates[0].result_code.as_deref(),
            Some("previous-process-result-untrusted")
        );
        assert!(checkpoint.candidates[1..]
            .iter()
            .all(|candidate| candidate.status == AutoSetupCandidateStatus::Untouched));
        assert!(has_untouched_auto_candidates(&checkpoint));
        let serialized = serde_json::to_string(&checkpoint).unwrap();
        assert!(!serialized.contains("device_api_key"));
        assert!(!serialized.contains("request_body"));
    }

    #[test]
    fn checkpoint_resume_rejects_boot_deadline_revision_scope_and_apply_mismatches() {
        let checkpoint = checkpoint_fixture();
        let now = SetupClock {
            boot_identity: checkpoint.boot_identity.clone(),
            monotonic_ms: 2_000,
            utc_now: "2026-09-26T00:00:01Z".into(),
            unix_now: 1_790_352_001,
        };
        let candidates = checkpoint
            .candidates
            .iter()
            .map(|candidate| candidate.model_id.clone())
            .collect::<Vec<_>>();
        assert!(!auto_setup_checkpoint_matches_resume(
            &checkpoint,
            &SetupClock {
                boot_identity: "2026-09-24T00:00:00Z".into(),
                ..now.clone()
            },
            &checkpoint.service_revision,
            &checkpoint.authorization_scope_sha256,
            &candidates,
        ));
        assert!(!auto_setup_checkpoint_matches_resume(
            &checkpoint,
            &SetupClock {
                monotonic_ms: checkpoint.deadline_monotonic_ms,
                ..now.clone()
            },
            &checkpoint.service_revision,
            &checkpoint.authorization_scope_sha256,
            &candidates,
        ));
        assert!(!auto_setup_checkpoint_matches_resume(
            &checkpoint,
            &now,
            "0000000000000000000000000000000000000000",
            &checkpoint.authorization_scope_sha256,
            &candidates,
        ));
        assert!(!auto_setup_checkpoint_matches_resume(
            &checkpoint,
            &now,
            &checkpoint.service_revision,
            &"b".repeat(64),
            &candidates,
        ));
        let mut applying = checkpoint.clone();
        applying.phase = AutoSetupPhase::ApplyPending;
        assert!(!auto_setup_checkpoint_matches_resume(
            &applying,
            &now,
            &applying.service_revision,
            &applying.authorization_scope_sha256,
            &candidates,
        ));
    }

    #[test]
    fn malformed_checkpoint_fields_are_rejected() {
        let mut checkpoint = checkpoint_fixture();
        checkpoint.deadline_monotonic_ms += 1;
        assert!(!valid_auto_setup_checkpoint_shape(&checkpoint));
        checkpoint = checkpoint_fixture();
        checkpoint.candidates[1].model_id = checkpoint.candidates[0].model_id.clone();
        assert!(!valid_auto_setup_checkpoint_shape(&checkpoint));
        checkpoint = checkpoint_fixture();
        checkpoint.candidates[0].result_code = Some("secret-key-value".into());
        assert!(!valid_auto_setup_checkpoint_shape(&checkpoint));
    }

    fn compatibility_fixture() -> (Value, Value) {
        let models = ["glm-5.2", "deepseek-v4-flash", "claude-fable-5"];
        let capabilities = REQUIRED_CODEX_CAPABILITIES;
        let manifest_models = models.iter().map(|model| json!({
            "id": model,
            "certification_id": format!("{CODEX_CERTIFICATION_PREFIX}{model}-sha256-{CODEX_CERTIFICATION_EVIDENCE_SHA256}"),
            "capabilities": capabilities,
        })).collect::<Vec<_>>();
        let manifest = json!({
            "schema_version": 1,
            "service_revision": "c4d48cf753c62f166d848c06c1fc25a23430316d",
            "profile_id": CODEX_COMPATIBILITY_PROFILE,
            "evidence_recorded_at": "2026-09-25T17:50:06Z",
            "evidence_time_source": CODEX_EVIDENCE_TIME_SOURCE,
            "model_priority": models,
            "default_model_id": models[0],
            "models": manifest_models,
        });
        let catalog = json!({ "object": "list", "data": [
            {"id":"claude-fable-5"}, {"id":"unverified-model"},
            {"id":"deepseek-v4-flash"}, {"id":"glm-5.2"}
        ]});
        (manifest, catalog)
    }

    #[test]
    fn compatibility_manifest_is_strict_and_intersected_with_exact_current_ids() {
        let (manifest, catalog) = compatibility_fixture();
        let accepted = validate_compatibility_manifest(&manifest, &catalog, "fixture-key")
            .expect("the matching certified model list should be accepted");
        assert_eq!(accepted.default_model_id, "glm-5.2");
        assert_eq!(
            accepted.model_priority,
            ["glm-5.2", "deepseek-v4-flash", "claude-fable-5"]
        );

        let filtered_catalog = json!({"data":[{"id":"glm-5.2"},{"id":"claude-fable-5"}]});
        let filtered = validate_compatibility_manifest(&manifest, &filtered_catalog, "fixture-key")
            .expect("lower priority IDs may be filtered by the local catalog");
        assert_eq!(filtered.model_priority, ["glm-5.2", "claude-fable-5"]);

        let no_default = json!({"data":[{"id":"deepseek-v4-flash"}]});
        assert_eq!(
            validate_compatibility_manifest(&manifest, &no_default, "fixture-key"),
            Err("certified-models-catalog-mismatch")
        );
    }

    #[test]
    fn compatibility_manifest_rejects_invalid_revision_priority_fields_and_certification() {
        let (valid, catalog) = compatibility_fixture();
        let cases = [
            {
                let mut value = valid.clone();
                value["service_revision"] = json!("unknown");
                value
            },
            {
                let mut value = valid.clone();
                value["default_model_id"] = json!("deepseek-v4-flash");
                value
            },
            {
                let mut value = valid.clone();
                value["models"][0]["certification_id"] = json!("untrusted");
                value
            },
            {
                let mut value = valid.clone();
                value["models"][0]["capabilities"] = json!(["responses"]);
                value
            },
            {
                let mut value = valid.clone();
                value["unexpected"] = json!(true);
                value
            },
            {
                let mut value = valid.clone();
                value["evidence_recorded_at"] = json!("2026-02-30T17:50:06Z");
                value
            },
        ];
        for invalid in cases {
            assert_eq!(
                validate_compatibility_manifest(&invalid, &catalog, "fixture-key"),
                Err("compatibility-manifest-invalid")
            );
        }
    }
    #[test]
    fn post_acknowledgement_failure_never_claims_no_application() {
        let mut result = report("fixture", "partial", "fixture");
        assert_eq!(activation_failure_status(&result), "incomplete");
        result.activation_status = "outcome_unconfirmed".into();
        assert_eq!(activation_failure_status(&result), "outcome_unconfirmed");
        result.default_model = Some("verified-fixture-model".into());
        assert_eq!(
            activation_failure_status(&result),
            "applied_restart_pending"
        );
    }

    #[test]
    fn discovery_does_not_grant_native_capabilities_or_read_extra_fields() {
        let payload = json!({"data":[{"id":"vendor/arbitrary","owned_by":"vendor","api_key":"fixture-only"}]});
        let models = normalize_models(parse_gateway_models(&payload));
        let model =
            ModelCapabilityReport::discovered(models[0].raw_id.clone(), models[0].owned_by.clone());
        assert_eq!(model.state, ModelState::Unverified);
        let encoded = serde_json::to_string(&model).unwrap();
        assert!(!encoded.contains("fixture-only"));
        assert!(!encoded.contains("unified_exec"));
        assert!(!encoded.contains("128000"));
        assert!(!encoded.contains("responses_lite"));
    }
    #[test]
    fn alias_normalization_preserves_vendor_and_raw_request_id() {
        let models = normalize_models(parse_gateway_models(&json!({"data":[
            {"id":"model-latest","owned_by":"a"},{"id":"model","owned_by":"a"},
            {"id":"model","owned_by":"b"},{"id":"rare-20260901","owned_by":"c"}]})));
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.raw_id == "rare-20260901"));
        assert_eq!(parse_date_suffix("2026-02-29"), None);
        assert_eq!(parse_date_suffix("2024-02-29"), Some(20240229));
    }
    #[test]
    fn managed_removal_preserves_user_settings_and_rejects_corruption() {
        let text=format!("model = \"user-selected\"\r\n{CONFIG_BEGIN}\r\nmodel_catalog_json = \"owned.json\"\r\n{CONFIG_END}\r\n[other]\r\nx = 1\r\n");
        assert_eq!(
            remove_managed_config(&text).unwrap(),
            "model = \"user-selected\"\r\n[other]\r\nx = 1\r\n"
        );
        assert!(remove_managed_config(&format!(
            "{CONFIG_BEGIN}\nmodel = \"user\"\n{CONFIG_END}\n"
        ))
        .is_err());
        assert!(remove_managed_config(&format!("{CONFIG_BEGIN}\n")).is_err());
        assert!(remove_managed_config(&format!("{CONFIG_END}\n")).is_err());
        assert!(remove_managed_config(&format!(
            "{CONFIG_BEGIN}\n{CONFIG_END}\n{CONFIG_BEGIN}\n{CONFIG_END}\n"
        ))
        .is_err());
    }
    fn run_registry() -> RunRegistry {
        RunRegistry {
            active: None,
            cancelled_before_start: BTreeSet::new(),
            finished: VecDeque::new(),
        }
    }
    #[test]
    fn cancellation_before_start_is_retained_and_never_runs() {
        let mut registry = run_registry();
        assert!(registry.cancel("early"));
        assert!(registry.cancel("early"));
        assert_eq!(
            registry.begin("early").unwrap_err(),
            RunAdmission::Cancelled
        );
        assert!(!registry.cancel("early"));
        assert_eq!(registry.begin("early").unwrap_err(), RunAdmission::Finished);
    }
    #[test]
    fn cancellation_only_marks_its_own_active_run_and_cannot_undo_commit() {
        let mut registry = run_registry();
        let flag = registry.begin("owner").unwrap();
        assert!(registry.cancel("other"));
        assert!(!flag.load(AtomicOrdering::Acquire));
        assert!(registry.cancel("owner"));
        assert!(flag.load(AtomicOrdering::Acquire));
        registry.finish("owner");
        assert!(!registry.cancel("owner"));
        assert_eq!(
            registry.begin("other").unwrap_err(),
            RunAdmission::Cancelled
        );
    }
    #[test]
    fn full_cancellation_registry_does_not_silently_drop_an_acknowledgement() {
        let mut registry = run_registry();
        for index in 0..RUN_HISTORY_LIMIT {
            assert!(registry.cancel(&format!("pending-{index}")));
        }
        assert!(!registry.cancel("overflow"));
        assert_eq!(registry.begin("new").unwrap_err(), RunAdmission::Busy);
        assert_eq!(
            registry.begin("pending-0").unwrap_err(),
            RunAdmission::Cancelled
        );
        assert!(registry.begin("new").is_ok());
        assert_eq!(registry.cancelled_before_start.len(), RUN_HISTORY_LIMIT - 1);
    }
    #[test]
    fn completed_history_is_bounded_and_request_ids_are_validated() {
        let mut registry = run_registry();
        for index in 0..RUN_HISTORY_LIMIT + 10 {
            let id = format!("run-{index}");
            registry.begin(&id).unwrap();
            registry.finish(&id);
        }
        assert_eq!(registry.finished.len(), RUN_HISTORY_LIMIT);
        assert!(valid_run_id("probe-abc_123"));
        for invalid in ["", "bad\nrequest", "bad/request"] {
            assert!(!valid_run_id(invalid));
        }
        assert!(!valid_run_id(&"a".repeat(65)));
    }
}

#[cfg(test)]
#[path = "model_catalog_storage_tests.rs"]
mod storage_tests;
