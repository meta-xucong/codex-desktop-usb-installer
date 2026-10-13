//! The only producer of in-process native-runtime capability evidence.
//!
//! This module deliberately has no disk authority path.  A successful result is
//! constructed only after the current gateway, the current native catalog
//! parser, and one isolated app-server turn have all been observed by this
//! process.

#[cfg(windows)]
use crate::app::model_capability::CapabilityState;
use crate::app::model_capability::{
    CapabilityContext, ModelCapabilityReport, NativeRuntimeEvidence,
};
use crate::app::model_catalog::MODEL_BASE_INSTRUCTIONS;
use crate::app::model_diagnostics::safe_identifier;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PROVIDER_ID: &str = "codex-native-probe";
const CATALOG_IDENTITY: &str = "404token-unified-api-internal";
const CLIENT_NAME: &str = "codex-app-manager-native-probe";
const CLIENT_PROTOCOL_VERSION: &str = "1";
const OPENAI_API_KEY: &str = "OPENAI_API_KEY";

const MAX_PROTOCOL_LINE_BYTES: usize = 256 * 1024;
const MAX_PROTOCOL_STDOUT_BYTES: usize = 8 * 1024 * 1024;
// `debug models` serializes a normalized catalog and may add runtime defaults
// and derived fields. Keep the capture bounded while leaving room for the
// largest accepted 1 MiB candidate plus JSON formatting/normalization.
const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROTOCOL_MESSAGES: usize = 512;
const MAX_PROTOCOL_REQUESTS: usize = 128;
const MAX_STREAM_BYTES: usize = 256 * 1024;
const MAX_CATALOG_BYTES: usize = crate::app::model_probe::BODY_LIMIT;
const MAX_VERSION_BYTES: usize = 16 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;

pub(crate) struct NativeProbeInput {
    pub executable: std::path::PathBuf,
    /// Hash of the authorized USB CLI. This binds evidence to the installation
    /// being configured, but this mutable path is never executed.
    pub expected_executable_sha256: String,
    /// Verified MSIX source and member identity for the executable that may
    /// be run. This is separate from the USB CLI because the installer may
    /// make the documented PE subsystem adjustment to its portable copy.
    pub trusted_payload_path: PathBuf,
    /// Keeps the staged MSIX snapshot pinned against later write/delete opens
    /// from verification through extraction of the private CLI copy.
    pub _trusted_payload_pin: std::fs::File,
    pub trusted_cli_entry_path: String,
    pub trusted_cli_sha256: String,
    pub context: CapabilityContext,
    pub model: ModelCapabilityReport,
    pub gateway_base: String,
    pub device_key: String,
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub deadline: std::time::Instant,
}

pub(crate) struct NativeProbeResult {
    pub proof: NativeRuntimeEvidence,
    #[cfg(test)]
    pub catalog_entry: serde_json::Value,
    pub runtime_version: String,
    pub executable_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeProbeError {
    #[cfg_attr(windows, allow(dead_code))]
    UnsupportedPlatform,
    Cancelled,
    DeadlineExceeded,
    InvalidInput,
    ExecutableUnavailable,
    ExecutableChanged,
    RuntimeVersionUnavailable,
    TemporaryStorage,
    CatalogUnsupported,
    GatewayUnavailable,
    GatewayIncomplete,
    RuntimeUnavailable,
    ProtocolInvalid,
    CorrelationInvalid,
    ApprovalRejected,
    UnexpectedExecution,
    NonceMismatch,
    TurnFailed,
    OutputLimit,
    RequestLimit,
    ProcessFailed,
    CleanupFailed,
}

impl NativeProbeError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "native-platform-unsupported",
            Self::Cancelled => "native-probe-cancelled",
            Self::DeadlineExceeded => "native-probe-timeout",
            Self::InvalidInput => "native-input-invalid",
            Self::ExecutableUnavailable => "native-executable-unavailable",
            Self::ExecutableChanged => "native-executable-changed",
            Self::RuntimeVersionUnavailable => "native-runtime-version-unavailable",
            Self::TemporaryStorage => "native-temporary-storage",
            Self::CatalogUnsupported => "native-catalog-unsupported",
            Self::GatewayUnavailable => "native-gateway-unavailable",
            Self::GatewayIncomplete => "native-gateway-incomplete",
            Self::RuntimeUnavailable => "native-runtime-unavailable",
            Self::ProtocolInvalid => "native-protocol-invalid",
            Self::CorrelationInvalid => "native-correlation-invalid",
            Self::ApprovalRejected => "native-approval-rejected",
            Self::UnexpectedExecution => "native-unexpected-execution",
            Self::NonceMismatch => "native-nonce-mismatch",
            Self::TurnFailed => "native-turn-failed",
            Self::OutputLimit => "native-output-limit",
            Self::RequestLimit => "native-request-limit",
            Self::ProcessFailed => "native-process-failed",
            Self::CleanupFailed => "native-cleanup-failed",
        }
    }
}

pub(crate) fn probe(input: NativeProbeInput) -> Result<NativeProbeResult, NativeProbeError> {
    #[cfg(windows)]
    {
        probe_windows(input)
    }
    #[cfg(not(windows))]
    {
        let _ = input;
        Err(NativeProbeError::UnsupportedPlatform)
    }
}

/// Read the unmodified model list compiled into the bundled Codex runtime.
/// This is used only when the manager creates its first overlay, so adding a
/// gateway model does not hide the client's existing native models.
#[cfg(windows)]
pub(crate) fn native_default_catalog(
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<Value, NativeProbeError> {
    check_limits(cancelled, deadline)?;
    let mut area = TemporaryArea::create(cancelled, deadline)?;
    let result = run_after_runtime_verification(
        || verified_catalog_runtime(&area, cancelled, deadline),
        |executable| {
            win_process::capture(
                executable,
                &["debug", "models"],
                &area,
                None,
                cancelled,
                deadline,
            )
        },
    )?;
    if !result.status.success() {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    let models = native_models_from_debug_output(&result.stdout)?;
    let catalog = json!({
        "schema_version": 1,
        "fetched_at": format!("unix:{}", unix_seconds()),
        "client_version": crate::app::model_catalog::CATALOG_CLIENT_VERSION,
        "identity": CATALOG_IDENTITY,
        "models": models,
    });
    let bytes = serde_json::to_vec(&catalog).map_err(|_| NativeProbeError::CatalogUnsupported)?;
    if !within_catalog_limit(bytes.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    area.cleanup()?;
    Ok(catalog)
}

#[cfg(not(windows))]
pub(crate) fn native_default_catalog(
    _: &AtomicBool,
    _: std::time::Instant,
) -> Result<Value, NativeProbeError> {
    Err(NativeProbeError::UnsupportedPlatform)
}

fn native_models_from_debug_output(bytes: &[u8]) -> Result<Vec<Value>, NativeProbeError> {
    if !within_capture_limit(bytes.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    let output: Value =
        serde_json::from_slice(bytes).map_err(|_| NativeProbeError::CatalogUnsupported)?;
    let object = output
        .as_object()
        .ok_or(NativeProbeError::CatalogUnsupported)?;
    let models = output
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty() && models.len() < 512)
        .ok_or(NativeProbeError::CatalogUnsupported)?;
    if object.len() != 1 {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    let mut slugs = std::collections::BTreeSet::new();
    for model in models {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .filter(|slug| safe_identifier(slug))
            .ok_or(NativeProbeError::CatalogUnsupported)?;
        if !slugs.insert(slug) {
            return Err(NativeProbeError::CatalogUnsupported);
        }
    }
    let models = models.clone();
    let encoded = serde_json::to_vec(&models).map_err(|_| NativeProbeError::CatalogUnsupported)?;
    if !within_catalog_limit(encoded.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    Ok(models)
}

fn check_limits(
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<(), NativeProbeError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(NativeProbeError::Cancelled);
    }
    if std::time::Instant::now() >= deadline {
        return Err(NativeProbeError::DeadlineExceeded);
    }
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn valid_metadata(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && !value
            .bytes()
            .any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn neutral_catalog_entry(model: &ModelCapabilityReport) -> Value {
    json!({
        "slug": model.model_id,
        "display_name": model.model_id,
        "description": "Temporary native-probe candidate",
        "shell_type": "unified_exec",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "support_verbosity": false,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "availability_nux": null,
        "upgrade": null,
        "include_skills_usage_instructions": true,
        "include_plugin_usage_instructions": false,
        "include_apps_usage_instructions": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "base_instructions": MODEL_BASE_INSTRUCTIONS,
        "supports_image_detail_original": false,
        "comp_hash": null,
        "experimental_supported_tools": [],
        "input_modalities": ["text"],
        "supports_search_tool": false,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": true,
        "default_reasoning_level": null,
        "supported_reasoning_levels": []
    })
}

fn neutral_catalog_root(entry: &Value, runtime_version: &str) -> Value {
    json!({
        "schema_version": 1,
        "fetched_at": format!("unix:{}", unix_seconds()),
        "client_version": runtime_version,
        "identity": CATALOG_IDENTITY,
        "models": [entry]
    })
}

fn catalog_is_neutral(entry: &Value) -> bool {
    entry.get("shell_type").and_then(Value::as_str) == Some("unified_exec")
        && entry
            .get("include_skills_usage_instructions")
            .and_then(Value::as_bool)
            == Some(true)
        && entry
            .get("default_reasoning_level")
            .is_some_and(Value::is_null)
        && entry
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        && entry.get("default_verbosity").is_some_and(Value::is_null)
        && entry
            .get("truncation_policy")
            .is_some_and(|value| value == &json!({"mode":"bytes","limit":10000}))
        && entry
            .get("base_instructions")
            .and_then(Value::as_str)
            .is_some()
        && entry.get("default_reasoning_summary").is_none()
        && entry.get("web_search_tool_type").is_none()
        && entry.get("effective_context_window_percent").is_none()
        && entry.get("comp_hash").is_some_and(Value::is_null)
        && entry
            .get("input_modalities")
            .and_then(Value::as_array)
            .is_some()
        && entry.get("supported_in_api").and_then(Value::as_bool) == Some(true)
        && entry.get("context_window").is_none()
        && entry.get("max_context_window").is_none()
        && entry.get("tool_mode").is_none()
        && entry.get("use_responses_lite").is_none()
}

fn gateway_model_matches(item: &Value, model: &ModelCapabilityReport, key: &str) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    let id = object
        .get("id")
        .or_else(|| object.get("slug"))
        .and_then(Value::as_str);
    let owner = object
        .get("owned_by")
        .or_else(|| object.get("ownedBy"))
        .and_then(Value::as_str);
    id == Some(model.model_id.as_str())
        && match (model.owned_by.as_deref(), owner) {
            (Some(expected), Some(actual)) => expected == actual,
            (None, None) => true,
            _ => false,
        }
        && !id.is_some_and(|value| value.contains(key))
        && !owner.is_some_and(|value| value.contains(key))
}

fn catalog_output_matches(bytes: &[u8], expected: &Value) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return false;
    };
    let (Some(actual_root), Some(expected_root)) = (value.as_object(), expected.as_object()) else {
        return false;
    };
    let root_fields = [
        "schema_version",
        "fetched_at",
        "client_version",
        "identity",
        "models",
    ];
    if expected_root.len() != root_fields.len()
        || expected_root
            .keys()
            .any(|key| !root_fields.contains(&key.as_str()))
        || expected.get("schema_version").and_then(Value::as_u64) != Some(1)
        || expected.get("identity").and_then(Value::as_str) != Some(CATALOG_IDENTITY)
        || !expected
            .get("client_version")
            .and_then(Value::as_str)
            .is_some_and(|v| valid_metadata(v, 256))
        || !expected
            .get("fetched_at")
            .and_then(Value::as_str)
            .is_some_and(|v| {
                v.strip_prefix("unix:")
                    .is_some_and(|t| t.parse::<u64>().is_ok())
            })
        || actual_root.len() != 1
        || !actual_root.contains_key("models")
    {
        return false;
    }
    let (Some(models), Some(entries)) = (
        value.get("models").and_then(Value::as_array),
        expected.get("models").and_then(Value::as_array),
    ) else {
        return false;
    };
    if entries.is_empty() || entries.len() > 512 || models.len() != entries.len() {
        return false;
    }
    let mut expected_slugs = std::collections::BTreeSet::new();
    let mut actual_slugs = std::collections::BTreeSet::new();
    if !entries.iter().all(|entry| {
        entry
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| safe_identifier(slug) && expected_slugs.insert(slug))
    }) || !models.iter().all(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| safe_identifier(slug) && actual_slugs.insert(slug))
    }) || expected_slugs != actual_slugs
    {
        return false;
    }

    entries.iter().all(|expected_entry| {
        let Some(slug) = expected_entry.get("slug").and_then(Value::as_str) else {
            return false;
        };
        let Some(actual_entry) = models
            .iter()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some(slug))
        else {
            return false;
        };
        catalog_model_entry_matches(actual_entry, expected_entry)
    })
}

/// `codex debug models` prints only its normalized `models` array, not the
/// input catalog wrapper. It also materializes known runtime defaults and
/// `model_messages` from `base_instructions`. Match every submitted field,
/// allowing the two verified null fields to be omitted, documented runtime
/// defaults, and false-only legacy flags that this runtime ignores.
fn catalog_model_entry_matches(actual: &Value, expected: &Value) -> bool {
    let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) else {
        return false;
    };
    for (key, expected_value) in expected {
        let Some(actual_value) = actual.get(key) else {
            // This runtime omits these two verified optional null fields from
            // `debug models`; no other expected null field may be omitted.
            if expected_value.is_null()
                && matches!(key.as_str(), "comp_hash" | "default_reasoning_level")
            {
                continue;
            }
            let ignored_false_flag = matches!(
                key.as_str(),
                "supports_parallel_tool_calls" | "supports_reasoning_summaries"
            ) && expected_value.as_bool() == Some(false);
            if ignored_false_flag {
                continue;
            }
            return false;
        };
        if actual_value == expected_value {
            continue;
        }
        let known_normalization = match key.as_str() {
            "default_reasoning_summary" => {
                expected_value.is_null() && actual_value.as_str() == Some("auto")
            }
            "web_search_tool_type" => {
                expected_value.is_null() && actual_value.as_str() == Some("text")
            }
            "effective_context_window_percent" => {
                expected_value.is_null() && actual_value.as_u64() == Some(95)
            }
            _ => false,
        };
        if !known_normalization {
            return false;
        }
    }
    for (key, actual_value) in actual {
        if expected.contains_key(key) {
            continue;
        }
        let runtime_default = match key.as_str() {
            "model_messages" => {
                let instructions = expected
                    .get("base_instructions")
                    .cloned()
                    .unwrap_or(Value::Null);
                actual_value
                    == &json!({
                        "instructions_template": instructions,
                        "instructions_variables": null,
                        "approvals": null,
                        "collaboration_modes": null,
                        "auto_review": null,
                        "permissions": null,
                        "multi_agent": null
                    })
            }
            "default_reasoning_summary" => actual_value.as_str() == Some("auto"),
            "web_search_tool_type" => actual_value.as_str() == Some("text"),
            "effective_context_window_percent" => actual_value.as_u64() == Some(95),
            "supports_experimental_context"
            | "use_responses_lite"
            | "supports_reasoning_effort_updates" => actual_value.as_bool() == Some(false),
            _ => false,
        };
        if !runtime_default {
            return false;
        }
    }
    true
}

fn within_catalog_limit(length: usize) -> bool {
    length <= MAX_CATALOG_BYTES
}

fn within_capture_limit(length: usize) -> bool {
    length <= MAX_CAPTURE_BYTES
}

/// Method-specific negative responses. Unknown methods use a JSON-RPC
/// error envelope, never a successful result containing an error object.
fn server_request_rejection(id: &Value, method: &str) -> Value {
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({"decision":"cancel"})
        }
        "applyPatchApproval" | "execCommandApproval" => json!({"decision":"abort"}),
        "item/permissions/requestApproval" => json!({
            "permissions":{"fileSystem":null,"network":null},"scope":"turn","strictAutoReview":false
        }),
        "mcpServer/elicitation/request" => json!({"action":"cancel","content":null}),
        "item/tool/requestUserInput" => json!({"answers":{}}),
        "item/tool/call" => json!({"success":false,"contentItems":[]}),
        _ => return json!({"id":id,"error":{"code":-32601,"message":"unsupported"}}),
    };
    json!({"id":id,"result":result})
}

fn same_id(left: &Value, right: &Value) -> bool {
    left == right
}

struct ExactStream {
    offset: usize,
    valid: bool,
    used: bool,
}

impl ExactStream {
    fn new() -> Self {
        Self {
            offset: 0,
            valid: true,
            used: false,
        }
    }

    fn push(&mut self, expected: &str, chunk: &str) -> Result<(), NativeProbeError> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.used = true;
        if !self.valid || self.offset.saturating_add(chunk.len()) > MAX_STREAM_BYTES {
            self.valid = false;
            return Err(NativeProbeError::OutputLimit);
        }
        let expected_bytes = expected.as_bytes();
        let chunk_bytes = chunk.as_bytes();
        let end = self.offset + chunk_bytes.len();
        if expected_bytes.get(self.offset..end) != Some(chunk_bytes) {
            self.valid = false;
            return Err(NativeProbeError::NonceMismatch);
        }
        self.offset = end;
        Ok(())
    }

    fn complete(&self, expected: &str) -> bool {
        !self.used || (self.valid && self.offset == expected.len())
    }
}

struct TurnObservation {
    thread_id: String,
    turn_id: String,
    cwd: String,
    command: String,
    nonce: String,
    command_item_id: Option<String>,
    command_started: bool,
    command_completed: bool,
    command_output: ExactStream,
    agent_item_id: Option<String>,
    agent_completed: bool,
    agent_text: ExactStream,
    approval_item_id: Option<String>,
    turn_completed: bool,
}

impl TurnObservation {
    fn new(thread_id: &str, turn_id: &str, cwd: &str, command: &str, nonce: &str) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
            cwd: cwd.to_owned(),
            command: command.to_owned(),
            nonce: nonce.to_owned(),
            command_item_id: None,
            command_started: false,
            command_completed: false,
            command_output: ExactStream::new(),
            agent_item_id: None,
            agent_completed: false,
            agent_text: ExactStream::new(),
            approval_item_id: None,
            turn_completed: false,
        }
    }

    fn check_identity(&self, thread_id: &str, turn_id: &str) -> Result<(), NativeProbeError> {
        if thread_id == self.thread_id && turn_id == self.turn_id {
            Ok(())
        } else {
            Err(NativeProbeError::CorrelationInvalid)
        }
    }

    fn record_command_item(
        &mut self,
        item: &Value,
        completed_event: bool,
    ) -> Result<(), NativeProbeError> {
        let object = item.as_object().ok_or(NativeProbeError::ProtocolInvalid)?;
        let item_id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| safe_identifier(value))
            .ok_or(NativeProbeError::CorrelationInvalid)?;
        if self
            .command_item_id
            .as_deref()
            .is_some_and(|known| known != item_id)
        {
            return Err(NativeProbeError::UnexpectedExecution);
        }
        self.command_item_id = Some(item_id.to_owned());
        if self
            .approval_item_id
            .as_deref()
            .is_some_and(|known| known != item_id)
        {
            return Err(NativeProbeError::ApprovalRejected);
        }
        if object.get("source").and_then(Value::as_str) != Some("agent") {
            return Err(NativeProbeError::UnexpectedExecution);
        }
        if object.get("command").and_then(Value::as_str) != Some(self.command.as_str())
            || object.get("cwd").and_then(Value::as_str) != Some(self.cwd.as_str())
        {
            return Err(NativeProbeError::CorrelationInvalid);
        }
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or(NativeProbeError::ProtocolInvalid)?;
        if matches!(status, "failed" | "declined") {
            return Err(NativeProbeError::TurnFailed);
        }
        if !matches!(status, "inProgress" | "completed") {
            return Err(NativeProbeError::ProtocolInvalid);
        }
        self.command_started = true;

        if completed_event || status == "completed" {
            if status != "completed"
                || object.get("exitCode").and_then(Value::as_i64) != Some(0)
                || object.get("aggregatedOutput").and_then(Value::as_str)
                    != Some(self.nonce.as_str())
                || !self.command_output.complete(&self.nonce)
            {
                return Err(NativeProbeError::NonceMismatch);
            }
            self.command_completed = true;
        }
        Ok(())
    }

    fn record_agent_item(
        &mut self,
        item: &Value,
        completed_event: bool,
    ) -> Result<(), NativeProbeError> {
        let object = item.as_object().ok_or(NativeProbeError::ProtocolInvalid)?;
        let item_id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| safe_identifier(value))
            .ok_or(NativeProbeError::CorrelationInvalid)?;
        if self
            .agent_item_id
            .as_deref()
            .is_some_and(|known| known != item_id)
        {
            return Err(NativeProbeError::CorrelationInvalid);
        }
        self.agent_item_id = Some(item_id.to_owned());
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or(NativeProbeError::ProtocolInvalid)?;
        if completed_event {
            if text != self.nonce || !self.agent_text.complete(&self.nonce) {
                return Err(NativeProbeError::NonceMismatch);
            }
            self.agent_completed = true;
        } else if !text.is_empty() && text != self.nonce {
            return Err(NativeProbeError::NonceMismatch);
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), NativeProbeError> {
        if self.turn_completed
            && self.command_started
            && self.command_completed
            && self.agent_completed
            && self.command_item_id.is_some()
            && self.agent_item_id.is_some()
            && self.command_output.complete(&self.nonce)
            && self.agent_text.complete(&self.nonce)
        {
            Ok(())
        } else {
            Err(NativeProbeError::CorrelationInvalid)
        }
    }
}

#[cfg(windows)]
struct TemporaryArea {
    root: PathBuf,
    home: PathBuf,
    workspace: PathBuf,
    appdata: PathBuf,
    localappdata: PathBuf,
    cleaned: bool,
}

#[cfg(windows)]
impl TemporaryArea {
    fn create(
        cancelled: &AtomicBool,
        deadline: std::time::Instant,
    ) -> Result<Self, NativeProbeError> {
        check_limits(cancelled, deadline)?;
        let base = std::fs::canonicalize(std::env::temp_dir())
            .map_err(|_| NativeProbeError::TemporaryStorage)?;
        let root = base.join(format!(
            "codex-native-probe-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).map_err(|_| NativeProbeError::TemporaryStorage)?;
        let home = root.join("home");
        let workspace = root.join("workspace");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        for path in [&home, &workspace, &appdata, &localappdata] {
            if std::fs::create_dir(path).is_err() {
                let _ = std::fs::remove_dir_all(&root);
                return Err(NativeProbeError::TemporaryStorage);
            }
        }
        let canonical_root = match std::fs::canonicalize(&root) {
            Ok(path) => path,
            Err(_) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(NativeProbeError::TemporaryStorage);
            }
        };
        if !canonical_root.starts_with(&base)
            || !canonical_root.is_dir()
            || [&home, &workspace, &appdata, &localappdata]
                .iter()
                .any(|path| {
                    std::fs::canonicalize(path)
                        .ok()
                        .is_none_or(|p| !p.starts_with(&canonical_root))
                })
        {
            let _ = std::fs::remove_dir_all(&root);
            return Err(NativeProbeError::TemporaryStorage);
        }
        Ok(Self {
            root: canonical_root,
            home,
            workspace,
            appdata,
            localappdata,
            cleaned: false,
        })
    }

    fn text(path: &Path) -> Result<String, NativeProbeError> {
        let value = path
            .to_str()
            .filter(|value| valid_metadata(value, 4096))
            .ok_or(NativeProbeError::TemporaryStorage)?;
        if value.contains('\'') {
            return Err(NativeProbeError::TemporaryStorage);
        }
        Ok(value.to_owned())
    }

    fn cleanup(&mut self) -> Result<(), NativeProbeError> {
        if self.cleaned {
            return Ok(());
        }
        let root =
            std::fs::canonicalize(&self.root).map_err(|_| NativeProbeError::CleanupFailed)?;
        let parent = root.parent().ok_or(NativeProbeError::CleanupFailed)?;
        if !root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("codex-native-probe-"))
            || std::fs::canonicalize(std::env::temp_dir())
                .ok()
                .is_none_or(|base| parent != base)
        {
            return Err(NativeProbeError::CleanupFailed);
        }
        std::fs::remove_dir_all(&root).map_err(|_| NativeProbeError::CleanupFailed)?;
        self.cleaned = true;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for TemporaryArea {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(windows)]
struct PreparedProbe {
    area: TemporaryArea,
    nonce: String,
    command: String,
    skill_name: String,
    skill_root: PathBuf,
    #[cfg(test)]
    catalog_entry: Value,
    cwd_text: String,
}

#[cfg(windows)]
fn write_owned_file(path: &Path, bytes: &[u8]) -> Result<(), NativeProbeError> {
    if !within_catalog_limit(bytes.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| NativeProbeError::TemporaryStorage)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| NativeProbeError::TemporaryStorage)?;
    drop(file);
    // Exclusive creation confines ownership. OS file attributes are not a
    // sandbox, and must not prevent bounded cleanup of these temporary files.
    Ok(())
}

#[cfg(windows)]
fn toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(windows)]
fn prepare_probe(
    area: TemporaryArea,
    model: &ModelCapabilityReport,
    runtime_version: &str,
    gateway_base: &str,
) -> Result<PreparedProbe, NativeProbeError> {
    let cwd_text = TemporaryArea::text(&area.workspace)?;
    let skill_name = format!("native_probe_{}", uuid::Uuid::new_v4().simple());
    let skill_root = area
        .workspace
        .join(".agents")
        .join("skills")
        .join(&skill_name);
    std::fs::create_dir_all(&skill_root).map_err(|_| NativeProbeError::TemporaryStorage)?;
    let payload_name = format!("payload-{}.txt", uuid::Uuid::new_v4().simple());
    let payload_path = area.workspace.join(payload_name);
    let payload_text = TemporaryArea::text(&payload_path)?;
    let command = format!(
        "[Console]::Write([IO.File]::ReadAllText('{}'))",
        payload_text
    );
    let nonce = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let skill_text = format!(
        "---\nname: {skill_name}\ndescription: Read the isolated native probe file.\n---\n\nRun exactly this one read-only PowerShell expression in the current working directory:\n{command}\nDo not alter its spelling, add another command, or describe the file. After it succeeds, reply with exactly the command stdout and nothing else.\n"
    );
    if skill_text.contains(&nonce) || command.contains(&nonce) {
        return Err(NativeProbeError::TemporaryStorage);
    }
    let skill_file = skill_root.join("SKILL.md");
    write_owned_file(&payload_path, nonce.as_bytes())?;
    write_owned_file(&skill_file, skill_text.as_bytes())?;

    let catalog_entry = neutral_catalog_entry(model);
    if !catalog_is_neutral(&catalog_entry) {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    let catalog_root = neutral_catalog_root(&catalog_entry, runtime_version);
    let catalog_bytes =
        serde_json::to_vec(&catalog_root).map_err(|_| NativeProbeError::CatalogUnsupported)?;
    let catalog_path = area.home.join("catalog.json");
    write_owned_file(&catalog_path, &catalog_bytes)?;
    let catalog_text = TemporaryArea::text(&catalog_path)?;
    let system_root =
        std::env::var("SystemRoot").map_err(|_| NativeProbeError::RuntimeUnavailable)?;
    let shell_path = std::env::join_paths([
        PathBuf::from(&system_root).join("System32"),
        PathBuf::from(&system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0"),
    ])
    .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
    let config_text = format!(
        "model_provider = \"{PROVIDER_ID}\"\nmodel_catalog_json = \"{}\"\nweb_search = \"disabled\"\n\n[shell_environment_policy]\ninherit = \"none\"\nset = {{ SystemRoot = \"{}\", WINDIR = \"{}\", PATH = \"{}\", TEMP = \"{}\", TMP = \"{}\" }}\n\n[model_providers.{PROVIDER_ID}]\nbase_url = \"{}\"\nwire_api = \"responses\"\nenv_key = \"{OPENAI_API_KEY}\"\n",
        toml_string(&catalog_text), toml_string(&system_root), toml_string(&system_root),
        toml_string(&shell_path.to_string_lossy()), toml_string(&TemporaryArea::text(&area.root)?),
        toml_string(&TemporaryArea::text(&area.root)?), toml_string(gateway_base),
    );
    write_owned_file(&area.home.join("config.toml"), config_text.as_bytes())?;
    Ok(PreparedProbe {
        area,
        nonce,
        command,
        skill_name,
        skill_root,
        #[cfg(test)]
        catalog_entry,
        cwd_text,
    })
}

#[cfg(windows)]
fn validate_input(input: &NativeProbeInput) -> Result<(), NativeProbeError> {
    if !input.executable.is_absolute()
        || !input.trusted_payload_path.is_absolute()
        || !valid_msix_cli_entry(&input.trusted_cli_entry_path)
        || !input
            .executable
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
        || !valid_metadata(&input.gateway_base, 4096)
        || !valid_metadata(&input.device_key, 4096)
        || input.gateway_base.contains(input.device_key.as_str())
        || !valid_sha256(&input.expected_executable_sha256)
        || !valid_sha256(&input.trusted_cli_sha256)
        || !valid_sha256(&input.context.binding_fingerprint)
        || !input
            .context
            .codex_version
            .as_deref()
            .is_some_and(|value| valid_metadata(value, 256))
        || !input
            .context
            .adapter_version
            .as_deref()
            .is_some_and(|value| valid_metadata(value, 256))
        || !safe_identifier(&input.model.model_id)
        || !input.model.owned_by.as_deref().is_none_or(safe_identifier)
    {
        return Err(NativeProbeError::InvalidInput);
    }
    if input.model.model_id.contains(&input.device_key)
        || input
            .model
            .owned_by
            .as_deref()
            .is_some_and(|owner| owner.contains(&input.device_key))
    {
        return Err(NativeProbeError::InvalidInput);
    }
    let metadata = std::fs::symlink_metadata(&input.executable)
        .map_err(|_| NativeProbeError::ExecutableUnavailable)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(NativeProbeError::ExecutableUnavailable);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(NativeProbeError::ExecutableUnavailable);
        }
    }
    let url = url::Url::parse(&input.gateway_base).map_err(|_| NativeProbeError::InvalidInput)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
        || input.gateway_base.contains('"')
    {
        return Err(NativeProbeError::InvalidInput);
    }
    check_limits(&input.cancelled, input.deadline)
}

#[cfg(windows)]
fn hash_executable(
    executable: &Path,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<String, NativeProbeError> {
    check_limits(cancelled, deadline)?;
    let mut file =
        std::fs::File::open(executable).map_err(|_| NativeProbeError::ExecutableUnavailable)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        check_limits(cancelled, deadline)?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| NativeProbeError::ExecutableUnavailable)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > MAX_EXECUTABLE_BYTES {
            return Err(NativeProbeError::ExecutableUnavailable);
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(windows)]
fn configure_command(
    command: &mut std::process::Command,
    area: &TemporaryArea,
    device_key: Option<&str>,
) -> Result<(), NativeProbeError> {
    use std::ffi::OsString;

    let system_root = std::env::var_os("SystemRoot").ok_or(NativeProbeError::RuntimeUnavailable)?;
    let system_root_path = PathBuf::from(&system_root);
    let system32 = system_root_path.join("System32");
    let powershell = system32.join("WindowsPowerShell").join("v1.0");
    let path = std::env::join_paths([
        system32.as_os_str(),
        powershell.as_os_str(),
        system_root_path.as_os_str(),
    ])
    .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
    let comspec = system32.join("cmd.exe");
    command.env_clear();
    command
        .env("SystemRoot", &system_root)
        .env("ComSpec", comspec)
        .env("PATH", path)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("CODEX_HOME", &area.home)
        .env("HOME", &area.home)
        .env("USERPROFILE", &area.home)
        .env("APPDATA", &area.appdata)
        .env("LOCALAPPDATA", &area.localappdata)
        .env("TEMP", &area.root)
        .env("TMP", &area.root)
        .env("NO_COLOR", "1");
    if let Some(device_key) = device_key {
        if device_key
            .bytes()
            .any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
        {
            return Err(NativeProbeError::InvalidInput);
        }
        let key = OsString::from(device_key);
        command.env(OPENAI_API_KEY, key);
    }
    Ok(())
}

#[cfg(windows)]
mod win_process {
    use super::{
        check_limits, same_id, server_request_rejection, within_capture_limit, NativeProbeError,
        MAX_CAPTURE_BYTES, MAX_PROTOCOL_LINE_BYTES, MAX_PROTOCOL_MESSAGES, MAX_PROTOCOL_REQUESTS,
        MAX_PROTOCOL_STDOUT_BYTES,
    };
    use serde_json::{json, Value};
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    };
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000 | 0x0000_0004; // CREATE_SUSPENDED

    struct Job(HANDLE);

    impl Job {
        fn attach(child: &Child) -> Result<Self, NativeProbeError> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(NativeProbeError::RuntimeUnavailable);
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                    || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
                {
                    return Err(NativeProbeError::RuntimeUnavailable);
                }
                crate::app::model_process::resume_suspended(child)
                    .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
                Ok(job)
            }
        }
        fn close(mut self) -> Result<(), NativeProbeError> {
            unsafe {
                let terminated = TerminateJobObject(self.0, 1) != 0;
                let closed = CloseHandle(self.0) != 0;
                self.0 = std::ptr::null_mut();
                if terminated && closed {
                    Ok(())
                } else {
                    Err(NativeProbeError::CleanupFailed)
                }
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = TerminateJobObject(self.0, 1);
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn stop_child(child: &mut Child, job: &mut Option<Job>) -> Result<(), NativeProbeError> {
        let tree = job.take().map(Job::close).transpose();
        let _ = child.kill();
        let waited = child.wait().map_err(|_| NativeProbeError::CleanupFailed);
        tree?;
        waited.map(|_| ())
    }

    fn spawn_command(
        executable: &Path,
        args: &[&str],
        area: &super::TemporaryArea,
        device_key: Option<&str>,
    ) -> Result<(Child, Option<Job>), NativeProbeError> {
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(&area.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);
        super::configure_command(&mut command, area, device_key)?;
        let mut child = command
            .spawn()
            .map_err(|_| NativeProbeError::ProcessFailed)?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let mut no_job = None;
                let _ = stop_child(&mut child, &mut no_job);
                return Err(error);
            }
        };
        Ok((child, Some(job)))
    }

    enum CaptureEvent {
        Stdout(Result<Vec<u8>, NativeProbeError>),
        Stderr(Result<(), NativeProbeError>),
    }

    pub(crate) struct CaptureResult {
        pub(crate) status: ExitStatus,
        pub(crate) stdout: Vec<u8>,
    }

    pub(crate) fn capture(
        executable: &Path,
        args: &[&str],
        area: &super::TemporaryArea,
        device_key: Option<&str>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> Result<CaptureResult, NativeProbeError> {
        check_limits(cancelled, deadline)?;
        let (mut child, mut job) = spawn_command(executable, args, area, device_key)?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let cleanup = stop_child(&mut child, &mut job);
                return Err(if cleanup.is_err() {
                    NativeProbeError::CleanupFailed
                } else {
                    NativeProbeError::ProcessFailed
                });
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let cleanup = stop_child(&mut child, &mut job);
                return Err(if cleanup.is_err() {
                    NativeProbeError::CleanupFailed
                } else {
                    NativeProbeError::ProcessFailed
                });
            }
        };
        let (tx, rx) = mpsc::channel();
        let stdout_reader = thread::spawn({
            let tx = tx.clone();
            move || {
                let mut bytes = Vec::new();
                let result = stdout
                    .take((MAX_CAPTURE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|_| NativeProbeError::ProcessFailed)
                    .and_then(|_| {
                        if !within_capture_limit(bytes.len()) {
                            Err(NativeProbeError::OutputLimit)
                        } else {
                            Ok(bytes)
                        }
                    });
                let _ = tx.send(CaptureEvent::Stdout(result));
            }
        });
        let stderr_reader = thread::spawn({
            let tx = tx.clone();
            move || {
                let result =
                    drain(stderr, MAX_CAPTURE_BYTES).map_err(|_| NativeProbeError::OutputLimit);
                let _ = tx.send(CaptureEvent::Stderr(result));
            }
        });
        drop(tx);

        let mut stdout_bytes = None;
        let mut stderr_done = false;
        let mut status = None;
        let mut failure = None;
        while stdout_bytes.is_none() || !stderr_done || status.is_none() {
            if let Err(error) = check_limits(cancelled, deadline) {
                failure = Some(error);
                break;
            }
            while let Ok(event) = rx.try_recv() {
                match event {
                    CaptureEvent::Stdout(result) => match result {
                        Ok(bytes) => stdout_bytes = Some(bytes),
                        Err(error) => failure = Some(error),
                    },
                    CaptureEvent::Stderr(result) => {
                        stderr_done = true;
                        if let Err(error) = result {
                            failure = Some(error);
                        }
                    }
                }
            }
            if failure.is_some() {
                break;
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(value) => status = value,
                    Err(_) => {
                        failure = Some(NativeProbeError::ProcessFailed);
                        break;
                    }
                }
                if status.is_some() && job.take().map(Job::close).transpose().is_err() {
                    failure = Some(NativeProbeError::CleanupFailed);
                    break;
                }
            }
            if stdout_bytes.is_some() && stderr_done && status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if failure.is_some() || status.is_none() {
            let cleanup = stop_child(&mut child, &mut job);
            if failure.is_none() {
                failure = Some(NativeProbeError::ProcessFailed);
            }
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            if cleanup.is_err() {
                return Err(NativeProbeError::CleanupFailed);
            }
            return Err(failure.unwrap_or(NativeProbeError::ProcessFailed));
        }
        let stdout_join_failed = stdout_reader.join().is_err();
        let stderr_join_failed = stderr_reader.join().is_err();
        if stdout_join_failed || stderr_join_failed {
            return Err(NativeProbeError::CleanupFailed);
        }
        let status = status.ok_or(NativeProbeError::ProcessFailed)?;
        let stdout_bytes = stdout_bytes.ok_or(NativeProbeError::ProcessFailed)?;
        if !status.success() {
            return Err(NativeProbeError::ProcessFailed);
        }
        Ok(CaptureResult {
            status,
            stdout: stdout_bytes,
        })
    }

    fn drain(mut reader: impl Read, limit: usize) -> io::Result<()> {
        let mut total = 0_usize;
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            total = total.saturating_add(count);
            if total > limit {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bounded output"));
            }
        }
    }

    enum ProtocolEvent {
        Message(Value),
        Eof,
        Failure(NativeProbeError),
    }

    pub(crate) struct Session {
        child: Child,
        stdin: Option<ChildStdin>,
        receiver: Receiver<ProtocolEvent>,
        readers: Vec<JoinHandle<()>>,
        watchdog: Option<JoinHandle<()>>,
        stop: Arc<AtomicBool>,
        job: Option<Job>,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
        sent: usize,
        received: usize,
        next_id: u64,
        stopped: bool,
    }

    impl Session {
        pub(crate) fn spawn(
            executable: &Path,
            area: &super::TemporaryArea,
            device_key: &str,
            cancelled: Arc<AtomicBool>,
            deadline: Instant,
        ) -> Result<Self, NativeProbeError> {
            check_limits(&cancelled, deadline)?;
            let mut command = Command::new(executable);
            command
                .args(["app-server", "--stdio"])
                .current_dir(&area.workspace)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(CREATE_NO_WINDOW);
            super::configure_command(&mut command, area, Some(device_key))?;
            let mut child = command
                .spawn()
                .map_err(|_| NativeProbeError::ProcessFailed)?;
            let job = match Job::attach(&child) {
                Ok(job) => job,
                Err(error) => {
                    let mut no_job = None;
                    let _ = stop_child(&mut child, &mut no_job);
                    return Err(error);
                }
            };
            let stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => {
                    let mut job = Some(job);
                    let cleanup = stop_child(&mut child, &mut job);
                    return Err(if cleanup.is_err() {
                        NativeProbeError::CleanupFailed
                    } else {
                        NativeProbeError::ProcessFailed
                    });
                }
            };
            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    let mut job = Some(job);
                    let cleanup = stop_child(&mut child, &mut job);
                    return Err(if cleanup.is_err() {
                        NativeProbeError::CleanupFailed
                    } else {
                        NativeProbeError::ProcessFailed
                    });
                }
            };
            let stderr = match child.stderr.take() {
                Some(stderr) => stderr,
                None => {
                    let mut job = Some(job);
                    let cleanup = stop_child(&mut child, &mut job);
                    return Err(if cleanup.is_err() {
                        NativeProbeError::CleanupFailed
                    } else {
                        NativeProbeError::ProcessFailed
                    });
                }
            };
            let (sender, receiver) = mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let stdout_stop = stop.clone();
            let stdout_sender = sender.clone();
            let stdout_reader = thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut total = 0_usize;
                loop {
                    let mut line = Vec::new();
                    match read_line_bounded(&mut reader, &mut line) {
                        Ok(Some(true)) => {
                            total = total.saturating_add(line.len());
                            if total > MAX_PROTOCOL_STDOUT_BYTES {
                                let _ = stdout_sender
                                    .send(ProtocolEvent::Failure(NativeProbeError::OutputLimit));
                                return;
                            }
                            while line.last() == Some(&b'\n') || line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            if line.is_empty() {
                                let _ = stdout_sender.send(ProtocolEvent::Failure(
                                    NativeProbeError::ProtocolInvalid,
                                ));
                                return;
                            }
                            let message = match serde_json::from_slice::<Value>(&line) {
                                Ok(message) => message,
                                Err(_) => {
                                    let _ = stdout_sender.send(ProtocolEvent::Failure(
                                        NativeProbeError::ProtocolInvalid,
                                    ));
                                    return;
                                }
                            };
                            if stdout_sender.send(ProtocolEvent::Message(message)).is_err() {
                                return;
                            }
                        }
                        Ok(Some(false)) | Ok(None) => {
                            if !stdout_stop.load(Ordering::Acquire) {
                                let _ = stdout_sender.send(ProtocolEvent::Eof);
                            }
                            return;
                        }
                        Err(_) => {
                            if !stdout_stop.load(Ordering::Acquire) {
                                let _ = stdout_sender.send(ProtocolEvent::Failure(
                                    NativeProbeError::ProtocolInvalid,
                                ));
                            }
                            return;
                        }
                    }
                }
            });
            let stderr_stop = stop.clone();
            let stderr_sender = sender.clone();
            let stderr_reader = thread::spawn(move || {
                if drain(stderr, MAX_PROTOCOL_STDOUT_BYTES).is_err()
                    && !stderr_stop.load(Ordering::Acquire)
                {
                    let _ =
                        stderr_sender.send(ProtocolEvent::Failure(NativeProbeError::OutputLimit));
                }
            });
            let watchdog_stop = stop.clone();
            let watchdog_cancelled = cancelled.clone();
            let watchdog_deadline = deadline;
            let watchdog_handle = job.0 as usize;
            let watchdog = thread::spawn(move || {
                let job_handle = watchdog_handle as HANDLE;
                while !watchdog_stop.load(Ordering::Acquire) {
                    if watchdog_cancelled.load(Ordering::Acquire)
                        || Instant::now() >= watchdog_deadline
                    {
                        unsafe {
                            let _ = TerminateJobObject(job_handle, 1);
                        }
                        return;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            });
            drop(sender);
            Ok(Self {
                child,
                stdin: Some(stdin),
                receiver,
                readers: vec![stdout_reader, stderr_reader],
                watchdog: Some(watchdog),
                stop,
                job: Some(job),
                cancelled,
                deadline,
                sent: 0,
                received: 0,
                next_id: 1,
                stopped: false,
            })
        }

        pub(crate) fn request(
            &mut self,
            method: &str,
            params: Value,
        ) -> Result<Value, NativeProbeError> {
            let id = self.send_request(method, params)?;
            loop {
                let message = self.next_message()?;
                if let Some(message_method) = message.get("method").and_then(Value::as_str) {
                    if message.get("id").is_some() {
                        self.cancel_server_request(&message, message_method)?;
                        return Err(NativeProbeError::ApprovalRejected);
                    }
                    super::preflight_notification(message_method, &message)?;
                    continue;
                }
                let Some(message_id) = message.get("id") else {
                    return Err(NativeProbeError::ProtocolInvalid);
                };
                if !same_id(message_id, &json!(id)) {
                    return Err(NativeProbeError::CorrelationInvalid);
                }
                if message.get("error").is_some() {
                    return Err(NativeProbeError::RuntimeUnavailable);
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or(NativeProbeError::ProtocolInvalid);
            }
        }

        pub(crate) fn notify(
            &mut self,
            method: &str,
            params: Option<Value>,
        ) -> Result<(), NativeProbeError> {
            let mut message = json!({"method": method});
            if let Some(params) = params {
                message["params"] = params;
            }
            self.send_message(message)
        }

        pub(crate) fn pump_turn(
            &mut self,
            observation: &mut super::TurnObservation,
        ) -> Result<(), NativeProbeError> {
            while !observation.turn_completed {
                let message = self.next_message()?;
                if let Some(method) = message.get("method").and_then(Value::as_str) {
                    if message.get("id").is_some() {
                        self.handle_server_request(&message, method, observation)?;
                    } else {
                        super::observe_notification(method, &message, observation)?;
                    }
                    continue;
                }
                return Err(NativeProbeError::ProtocolInvalid);
            }
            observation.finish()
        }

        fn send_request(&mut self, method: &str, params: Value) -> Result<u64, NativeProbeError> {
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            self.send_message(json!({"id": id, "method": method, "params": params}))?;
            Ok(id)
        }

        fn send_message(&mut self, message: Value) -> Result<(), NativeProbeError> {
            check_limits(&self.cancelled, self.deadline)?;
            self.sent = self.sent.saturating_add(1);
            if self.sent > MAX_PROTOCOL_REQUESTS {
                return Err(NativeProbeError::RequestLimit);
            }
            let bytes =
                serde_json::to_vec(&message).map_err(|_| NativeProbeError::ProtocolInvalid)?;
            if bytes.len() > MAX_PROTOCOL_LINE_BYTES {
                return Err(NativeProbeError::OutputLimit);
            }
            let result = {
                let stdin = self.stdin.as_mut().ok_or(NativeProbeError::ProcessFailed)?;
                stdin
                    .write_all(&bytes)
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
            };
            match result {
                Ok(()) => Ok(()),
                Err(_) => match check_limits(&self.cancelled, self.deadline) {
                    Err(error) => Err(error),
                    Ok(()) => Err(NativeProbeError::ProcessFailed),
                },
            }
        }

        fn next_message(&mut self) -> Result<Value, NativeProbeError> {
            loop {
                check_limits(&self.cancelled, self.deadline)?;
                match self.receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(ProtocolEvent::Message(message)) => {
                        self.received = self.received.saturating_add(1);
                        if self.received > MAX_PROTOCOL_MESSAGES {
                            return Err(NativeProbeError::RequestLimit);
                        }
                        if !message.is_object() {
                            return Err(NativeProbeError::ProtocolInvalid);
                        }
                        return Ok(message);
                    }
                    Ok(ProtocolEvent::Eof) => return Err(NativeProbeError::ProcessFailed),
                    Ok(ProtocolEvent::Failure(error)) => return Err(error),
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(NativeProbeError::ProcessFailed)
                    }
                }
            }
        }

        fn handle_server_request(
            &mut self,
            message: &Value,
            method: &str,
            observation: &mut super::TurnObservation,
        ) -> Result<(), NativeProbeError> {
            let id = message
                .get("id")
                .cloned()
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            if method == "item/commandExecution/requestApproval" {
                let matches = super::matching_command_approval(message.get("params"), observation);
                self.send_message(json!({
                    "id": id,
                    "result": {"decision": if matches {"accept"} else {"cancel"}}
                }))?;
                if matches {
                    Ok(())
                } else {
                    Err(NativeProbeError::ApprovalRejected)
                }
            } else {
                self.cancel_server_request(message, method)?;
                Err(NativeProbeError::ApprovalRejected)
            }
        }

        fn cancel_server_request(
            &mut self,
            message: &Value,
            method: &str,
        ) -> Result<(), NativeProbeError> {
            let id = message
                .get("id")
                .cloned()
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            self.send_message(server_request_rejection(&id, method))
        }

        pub(crate) fn shutdown(&mut self) -> Result<(), NativeProbeError> {
            if self.stopped {
                return Ok(());
            }
            self.stopped = true;
            self.stop.store(true, Ordering::Release);
            self.stdin.take();
            let mut join_failed = self
                .watchdog
                .take()
                .is_some_and(|watchdog| watchdog.join().is_err());
            let cleanup = stop_child(&mut self.child, &mut self.job);
            join_failed |= cleanup.is_err();
            for reader in self.readers.drain(..) {
                if reader.join().is_err() {
                    join_failed = true;
                }
            }
            if join_failed {
                Err(NativeProbeError::CleanupFailed)
            } else {
                Ok(())
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            let _ = self.shutdown();
        }
    }

    fn read_line_bounded(
        reader: &mut BufReader<impl Read>,
        line: &mut Vec<u8>,
    ) -> io::Result<Option<bool>> {
        loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                return Ok(if line.is_empty() { None } else { Some(false) });
            }
            let newline = buffer.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(buffer.len(), |index| index + 1);
            if line.len().saturating_add(count) > MAX_PROTOCOL_LINE_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bounded line"));
            }
            line.extend_from_slice(&buffer[..count]);
            reader.consume(count);
            if newline.is_some() {
                return Ok(Some(true));
            }
        }
    }
}

#[cfg(windows)]
fn read_runtime_version(
    executable: &Path,
    area: &TemporaryArea,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<String, NativeProbeError> {
    let result = win_process::capture(executable, &["--version"], area, None, cancelled, deadline)
        .map_err(|error| match error {
            NativeProbeError::Cancelled | NativeProbeError::DeadlineExceeded => error,
            _ => NativeProbeError::RuntimeVersionUnavailable,
        })?;
    if !result.status.success() || result.stdout.len() > MAX_VERSION_BYTES {
        return Err(NativeProbeError::RuntimeVersionUnavailable);
    }
    let first_line = result
        .stdout
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| valid_metadata(line, 256))
        .ok_or(NativeProbeError::RuntimeVersionUnavailable)?;
    Ok(first_line.to_owned())
}

#[cfg(windows)]
fn run_catalog_gate(
    executable: &Path,
    area: &TemporaryArea,
    model: &ModelCapabilityReport,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<(), NativeProbeError> {
    let mut candidate_bytes = Vec::new();
    std::fs::File::open(area.home.join("catalog.json"))
        .map_err(|_| NativeProbeError::CatalogUnsupported)?
        .take((MAX_CATALOG_BYTES + 1) as u64)
        .read_to_end(&mut candidate_bytes)
        .map_err(|_| NativeProbeError::CatalogUnsupported)?;
    if !within_catalog_limit(candidate_bytes.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    let expected: Value = serde_json::from_slice(&candidate_bytes)
        .map_err(|_| NativeProbeError::CatalogUnsupported)?;
    if expected.get("models") != Some(&json!([neutral_catalog_entry(model)])) {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    let result = win_process::capture(
        executable,
        &["debug", "models"],
        area,
        None,
        cancelled,
        deadline,
    )
    .map_err(|error| match error {
        NativeProbeError::Cancelled | NativeProbeError::DeadlineExceeded => error,
        _ => NativeProbeError::CatalogUnsupported,
    })?;
    if !result.status.success() || !catalog_output_matches(&result.stdout, &expected) {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    Ok(())
}

/// Validate a candidate against the bundled Codex parser before an explicit
/// model Add/setup commit. Startup uses bounded local validation instead of
/// spawning a second Codex process on every launch.
#[cfg(windows)]
pub(crate) fn validate_final_catalog(
    catalog: &[u8],
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<(), NativeProbeError> {
    check_limits(cancelled, deadline)?;
    if !within_catalog_limit(catalog.len()) {
        return Err(NativeProbeError::OutputLimit);
    }
    let value: Value =
        serde_json::from_slice(catalog).map_err(|_| NativeProbeError::CatalogUnsupported)?;
    let entries = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or(NativeProbeError::CatalogUnsupported)?;
    if entries.is_empty() || entries.len() > 512 {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    let mut area = TemporaryArea::create(cancelled, deadline)?;
    let path = area.home.join("catalog.json");
    write_owned_file(&path, catalog)?;
    let config = format!(
        "model_catalog_json = \"{}\"\n",
        toml_string(&TemporaryArea::text(&path)?)
    );
    write_owned_file(&area.home.join("config.toml"), config.as_bytes())?;
    let parsed = run_after_runtime_verification(
        || verified_catalog_runtime(&area, cancelled, deadline),
        |executable| {
            win_process::capture(
                executable,
                &["debug", "models"],
                &area,
                None,
                cancelled,
                deadline,
            )
        },
    )?;
    if !parsed.status.success() || !catalog_output_matches(&parsed.stdout, &value) {
        return Err(NativeProbeError::CatalogUnsupported);
    }
    area.cleanup()?;
    Ok(())
}

#[cfg(windows)]
fn verified_catalog_runtime(
    area: &TemporaryArea,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<PathBuf, NativeProbeError> {
    let verified =
        crate::app::model_catalog::verified_codex_runtime_for_catalog(cancelled, deadline)
            .map_err(|code| match code {
                "native-executable-changed" => NativeProbeError::ExecutableChanged,
                "native-probe-cancelled" => NativeProbeError::Cancelled,
                "native-probe-timeout" => NativeProbeError::DeadlineExceeded,
                _ => NativeProbeError::ExecutableUnavailable,
            })?;
    let copy = area.root.join("codex-verified.exe");
    extract_verified_cli_from_msix(
        &verified.trusted_payload_path,
        &verified.trusted_cli_entry_path,
        &verified.trusted_cli_sha256,
        &copy,
        cancelled,
        deadline,
    )?;
    Ok(copy)
}

#[cfg(windows)]
fn verified_probe_executable(
    input: &NativeProbeInput,
    area: &TemporaryArea,
) -> Result<PathBuf, NativeProbeError> {
    verified_probe_executable_path(
        &input.trusted_payload_path,
        &input.trusted_cli_entry_path,
        &input.trusted_cli_sha256,
        &area.root,
        &input.cancelled,
        input.deadline,
    )
}

fn verified_probe_executable_path(
    payload_path: &Path,
    entry_path: &str,
    expected_sha256: &str,
    temporary_root: &Path,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<PathBuf, NativeProbeError> {
    let copy = temporary_root.join("codex-native-probe-verified.exe");
    extract_verified_cli_from_msix(
        payload_path,
        entry_path,
        expected_sha256,
        &copy,
        cancelled,
        deadline,
    )?;
    Ok(copy)
}

fn valid_msix_cli_entry(value: &str) -> bool {
    value.len() <= 4096
        && !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
        && value.to_ascii_lowercase().ends_with("/resources/codex.exe")
}

fn extract_verified_cli_from_msix(
    payload_path: &Path,
    entry_path: &str,
    expected_sha256: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    deadline: std::time::Instant,
) -> Result<(), NativeProbeError> {
    if !payload_path.is_absolute()
        || !valid_msix_cli_entry(entry_path)
        || !valid_sha256(expected_sha256)
    {
        return Err(NativeProbeError::InvalidInput);
    }
    let result = (|| {
        check_limits(cancelled, deadline)?;
        let payload = std::fs::File::open(payload_path)
            .map_err(|_| NativeProbeError::ExecutableUnavailable)?;
        let mut archive =
            zip::ZipArchive::new(payload).map_err(|_| NativeProbeError::ExecutableUnavailable)?;
        let mut entry = archive
            .by_name(entry_path)
            .map_err(|_| NativeProbeError::ExecutableUnavailable)?;
        let declared_size = entry.size();
        if declared_size == 0 || declared_size > MAX_EXECUTABLE_BYTES {
            return Err(NativeProbeError::ExecutableUnavailable);
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|_| NativeProbeError::TemporaryStorage)?;
        let mut hasher = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            check_limits(cancelled, deadline)?;
            let count = entry
                .read(&mut buffer)
                .map_err(|_| NativeProbeError::ExecutableUnavailable)?;
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(count as u64)
                .filter(|size| *size <= MAX_EXECUTABLE_BYTES)
                .ok_or(NativeProbeError::OutputLimit)?;
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|_| NativeProbeError::TemporaryStorage)?;
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|_| NativeProbeError::TemporaryStorage)?;
        if copied != declared_size {
            return Err(NativeProbeError::ExecutableChanged);
        }
        let copied_sha256: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if !copied_sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err(NativeProbeError::ExecutableChanged);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn run_after_runtime_verification<T>(
    verify: impl FnOnce() -> Result<PathBuf, NativeProbeError>,
    run: impl FnOnce(&Path) -> Result<T, NativeProbeError>,
) -> Result<T, NativeProbeError> {
    let executable = verify()?;
    run(&executable)
}
#[cfg(not(windows))]
pub(crate) fn validate_final_catalog(
    _: &[u8],
    _: &AtomicBool,
    _: std::time::Instant,
) -> Result<(), NativeProbeError> {
    Err(NativeProbeError::UnsupportedPlatform)
}

#[cfg(windows)]
fn fresh_gateway(input: &NativeProbeInput) -> Result<ModelCapabilityReport, NativeProbeError> {
    use crate::app::model_capability::EvidenceScope;
    use crate::app::model_diagnostics::ProbeError;
    use crate::app::model_probe::ProbeClient;

    let model = input.model.clone();
    let gateway_complete = model.evidence_scope == EvidenceScope::Gateway
        && model.error_code.is_none()
        && model.checked_at.is_some()
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
        .all(|state| *state == CapabilityState::Verified);
    if !gateway_complete {
        return Err(NativeProbeError::GatewayIncomplete);
    }
    let client = ProbeClient::new(
        &input.gateway_base,
        &input.device_key,
        input.cancelled.clone(),
        input.deadline,
    )
    .map_err(|_| NativeProbeError::GatewayUnavailable)?;
    let result = tauri::async_runtime::block_on(async {
        client.health().await?;
        let payload = client.models().await?;
        let Some(items) = payload
            .get("data")
            .or_else(|| payload.get("models"))
            .and_then(Value::as_array)
        else {
            return Err(ProbeError::InvalidResponse);
        };
        if items.is_empty()
            || items.len() > 512
            || items
                .iter()
                .filter(|item| gateway_model_matches(item, &input.model, &input.device_key))
                .count()
                != 1
        {
            return Err(ProbeError::InvalidResponse);
        }
        Ok(())
    });
    match result {
        Ok(()) => {}
        Err(ProbeError::Cancelled) => return Err(NativeProbeError::Cancelled),
        Err(ProbeError::BudgetExhausted) => return Err(NativeProbeError::DeadlineExceeded),
        Err(_) => return Err(NativeProbeError::GatewayUnavailable),
    }
    let all_gateway_checks = [
        model.checks.text,
        model.checks.single_tool,
        model.checks.tool_result,
        model.checks.multiple_tools,
        model.checks.tool_error,
        model.checks.stream,
        model.checks.multi_turn,
    ]
    .iter()
    .all(|state| *state == CapabilityState::Verified);
    if model.evidence_scope != EvidenceScope::Gateway
        || model.checked_at.is_none()
        || !all_gateway_checks
    {
        return Err(NativeProbeError::GatewayIncomplete);
    }
    Ok(model)
}

#[cfg(windows)]
fn validate_initialize(result: &Value, area: &TemporaryArea) -> Result<(), NativeProbeError> {
    let home_text = TemporaryArea::text(&area.home)?;
    if result.get("codexHome").and_then(Value::as_str) != Some(home_text.as_str())
        || result.get("platformFamily").and_then(Value::as_str) != Some("windows")
        || result.get("platformOs").and_then(Value::as_str) != Some("windows")
        || !result
            .get("userAgent")
            .and_then(Value::as_str)
            .is_some_and(|value| valid_metadata(value, 512))
    {
        return Err(NativeProbeError::RuntimeUnavailable);
    }
    Ok(())
}

// Private wire-shape admission for the two retained list responses. These
// values validate structure only; none is a native capability certificate.
#[allow(dead_code)]
mod native_schema {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ModelList {
        pub(super) data: Vec<Model>,
        pub(super) next_cursor: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Model {
        pub(super) id: String,
        pub(super) model: String,
        description: String,
        display_name: String,
        hidden: bool,
        is_default: bool,
        pub(super) default_reasoning_effort: String,
        pub(super) supported_reasoning_efforts: Vec<ReasoningOption>,
        #[serde(default)]
        additional_speed_tiers: Vec<String>,
        availability_nux: Option<AvailabilityNux>,
        default_service_tier: Option<String>,
        #[serde(default)]
        input_modalities: Vec<InputModality>,
        model_specialty: Option<String>,
        multi_agent_version: Option<MultiAgentVersion>,
        #[serde(default)]
        service_tiers: Vec<ServiceTier>,
        #[serde(default)]
        supports_personality: bool,
        upgrade: Option<String>,
        upgrade_info: Option<UpgradeInfo>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ReasoningOption {
        description: String,
        pub(super) reasoning_effort: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AvailabilityNux {
        message: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ServiceTier {
        id: String,
        name: String,
        description: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct UpgradeInfo {
        model: String,
        migration_markdown: Option<String>,
        model_link: Option<String>,
        retirement_at: Option<i64>,
        upgrade_copy: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum InputModality {
        Text,
        Image,
        Audio,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum MultiAgentVersion {
        Disabled,
        V1,
        V2,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct SkillsList {
        pub(super) data: Vec<SkillsEntry>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct SkillsEntry {
        cwd: String,
        pub(super) errors: Vec<SkillError>,
        pub(super) skills: Vec<Skill>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct SkillError {
        message: String,
        path: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Skill {
        description: String,
        enabled: bool,
        name: String,
        path: String,
        scope: SkillScope,
        dependencies: Option<SkillDependencies>,
        interface: Option<SkillInterface>,
        plugin_id: Option<String>,
        short_description: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum SkillScope {
        User,
        Repo,
        System,
        Admin,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SkillDependencies {
        tools: Vec<SkillDependency>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SkillDependency {
        #[serde(rename = "type")]
        kind: String,
        value: String,
        command: Option<String>,
        description: Option<String>,
        transport: Option<String>,
        url: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct SkillInterface {
        brand_color: Option<String>,
        default_prompt: Option<String>,
        display_name: Option<String>,
        icon_large: Option<String>,
        icon_large_url: Option<String>,
        icon_small: Option<String>,
        icon_small_url: Option<String>,
        short_description: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
    pub(super) enum CommandAction {
        Read {
            command: String,
            name: String,
            path: String,
        },
        ListFiles {
            command: String,
            path: Option<String>,
        },
        Search {
            command: String,
            path: Option<String>,
            query: Option<String>,
        },
        Unknown {
            command: String,
        },
    }
}

fn validate_model_list(
    result: &Value,
    model: &ModelCapabilityReport,
) -> Result<(), NativeProbeError> {
    let parsed: native_schema::ModelList =
        serde_json::from_value(result.clone()).map_err(|_| NativeProbeError::ProtocolInvalid)?;
    if parsed.next_cursor.is_some()
        || parsed.data.len() > 512
        || parsed.data.iter().any(|item| {
            item.default_reasoning_effort.is_empty()
                || item.supported_reasoning_efforts.len() > 512
                || item
                    .supported_reasoning_efforts
                    .iter()
                    .any(|option| option.reasoning_effort.is_empty())
        })
        || parsed
            .data
            .iter()
            .filter(|item| item.id == model.model_id && item.model == model.model_id)
            .count()
            != 1
    {
        return Err(NativeProbeError::RuntimeUnavailable);
    }
    Ok(())
}

fn validate_skills_list_shape(result: &Value) -> Result<(), NativeProbeError> {
    let parsed: native_schema::SkillsList =
        serde_json::from_value(result.clone()).map_err(|_| NativeProbeError::ProtocolInvalid)?;
    if parsed.data.len() > 512
        || parsed
            .data
            .iter()
            .any(|entry| entry.skills.len() > 512 || entry.errors.len() > 512)
    {
        return Err(NativeProbeError::OutputLimit);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_skills_list(
    result: &Value,
    prepared: &PreparedProbe,
) -> Result<String, NativeProbeError> {
    validate_skills_list_shape(result)?;
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or(NativeProbeError::RuntimeUnavailable)?;
    let mut found = None;
    for entry in entries {
        if entry.get("cwd").and_then(Value::as_str) != Some(prepared.cwd_text.as_str()) {
            continue;
        }
        if entry
            .get("errors")
            .and_then(Value::as_array)
            .is_none_or(|errors| !errors.is_empty())
        {
            return Err(NativeProbeError::RuntimeUnavailable);
        }
        let skills = entry
            .get("skills")
            .and_then(Value::as_array)
            .ok_or(NativeProbeError::RuntimeUnavailable)?;
        for skill in skills {
            if skill.get("name").and_then(Value::as_str) != Some(prepared.skill_name.as_str())
                || skill.get("enabled").and_then(Value::as_bool) != Some(true)
            {
                continue;
            }
            let path = skill
                .get("path")
                .and_then(Value::as_str)
                .filter(|value| valid_metadata(value, 4096))
                .ok_or(NativeProbeError::RuntimeUnavailable)?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(NativeProbeError::RuntimeUnavailable);
            }
            let area_root = std::fs::canonicalize(&prepared.area.root)
                .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
            let workspace = std::fs::canonicalize(&prepared.area.workspace)
                .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
            let root = std::fs::canonicalize(&prepared.skill_root)
                .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
            let expected = std::fs::canonicalize(prepared.skill_root.join("SKILL.md"))
                .map_err(|_| NativeProbeError::RuntimeUnavailable)?;
            let actual =
                std::fs::canonicalize(&path).map_err(|_| NativeProbeError::RuntimeUnavailable)?;
            if !workspace.starts_with(&area_root)
                || !root.starts_with(&workspace)
                || !actual.starts_with(&root)
                || actual != expected
            {
                return Err(NativeProbeError::RuntimeUnavailable);
            }
            if found.replace(path.to_string_lossy().into_owned()).is_some() {
                return Err(NativeProbeError::CorrelationInvalid);
            }
        }
    }
    found.ok_or(NativeProbeError::RuntimeUnavailable)
}

fn validate_thread_start(
    result: &Value,
    model: &ModelCapabilityReport,
    cwd_text: &str,
) -> Result<String, NativeProbeError> {
    let sandbox = result
        .get("sandbox")
        .and_then(Value::as_object)
        .ok_or(NativeProbeError::RuntimeUnavailable)?;
    let roots_ok = result
        .get("runtimeWorkspaceRoots")
        .and_then(Value::as_array)
        .is_some_and(|roots| roots.len() == 1 && roots[0].as_str() == Some(cwd_text));
    let network_ok = sandbox
        .get("networkAccess")
        .is_some_and(|value| value.as_bool() == Some(false));
    if result.get("model").and_then(Value::as_str) != Some(model.model_id.as_str())
        || result.get("modelProvider").and_then(Value::as_str) != Some(PROVIDER_ID)
        || result.get("cwd").and_then(Value::as_str) != Some(cwd_text)
        || sandbox.get("type").and_then(Value::as_str) != Some("readOnly")
        || !network_ok
        || result.get("approvalPolicy").and_then(Value::as_str) != Some("on-request")
        || result.get("approvalsReviewer").and_then(Value::as_str) != Some("user")
        || !roots_ok
    {
        return Err(NativeProbeError::RuntimeUnavailable);
    }
    let thread = result
        .get("thread")
        .and_then(Value::as_object)
        .ok_or(NativeProbeError::ProtocolInvalid)?;
    let id = thread
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| safe_identifier(value))
        .ok_or(NativeProbeError::CorrelationInvalid)?;
    Ok(id.to_owned())
}

#[cfg(windows)]
fn validate_turn_start(result: &Value, thread_id: &str) -> Result<String, NativeProbeError> {
    // The supported TurnStartResponse contains only `turn`. Session::request
    // binds this response to its unique JSON-RPC request ID for this thread;
    // all subsequent item/turn events must repeat the same thread and turn IDs.
    // A newer runtime's explicit association must also agree when present.
    if result
        .get("threadId")
        .is_some_and(|v| v.as_str() != Some(thread_id))
    {
        return Err(NativeProbeError::CorrelationInvalid);
    }
    let turn = result
        .get("turn")
        .and_then(Value::as_object)
        .ok_or(NativeProbeError::ProtocolInvalid)?;
    let id = turn
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| safe_identifier(value))
        .ok_or(NativeProbeError::CorrelationInvalid)?;
    if !matches!(
        turn.get("status").and_then(Value::as_str),
        Some("inProgress" | "completed")
    ) || turn.get("error").is_some_and(|value| !value.is_null())
        || thread_id.is_empty()
        || turn
            .get("threadId")
            .is_some_and(|v| v.as_str() != Some(thread_id))
    {
        return Err(NativeProbeError::TurnFailed);
    }
    Ok(id.to_owned())
}

fn valid_event_identity(
    params: &Value,
    observation: &TurnObservation,
) -> Result<(), NativeProbeError> {
    let object = params
        .as_object()
        .ok_or(NativeProbeError::ProtocolInvalid)?;
    let thread_id = object
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or(NativeProbeError::CorrelationInvalid)?;
    let turn_id = object
        .get("turnId")
        .and_then(Value::as_str)
        .ok_or(NativeProbeError::CorrelationInvalid)?;
    observation.check_identity(thread_id, turn_id)
}

fn observe_turn_item(
    item: &Value,
    observation: &mut TurnObservation,
    completed_event: bool,
) -> Result<(), NativeProbeError> {
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or(NativeProbeError::ProtocolInvalid)?;
    match item_type {
        "commandExecution" => observation.record_command_item(item, completed_event),
        "agentMessage" => observation.record_agent_item(item, completed_event),
        "userMessage" | "reasoning" | "plan" | "contextCompaction" => Ok(()),
        "dynamicToolCall"
        | "mcpToolCall"
        | "fileChange"
        | "collabAgentToolCall"
        | "webSearch"
        | "imageGeneration"
        | "terminalInteraction" => Err(NativeProbeError::UnexpectedExecution),
        _ => Err(NativeProbeError::ProtocolInvalid),
    }
}

fn observe_notification(
    method: &str,
    message: &Value,
    observation: &mut TurnObservation,
) -> Result<(), NativeProbeError> {
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "thread/started" => {
            let id = params
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
                .ok_or(NativeProbeError::CorrelationInvalid)?;
            if id != observation.thread_id {
                return Err(NativeProbeError::CorrelationInvalid);
            }
        }
        "turn/started" => {
            valid_event_identity(params, observation)?;
            let turn = params
                .get("turn")
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            if turn.get("id").and_then(Value::as_str) != Some(observation.turn_id.as_str())
                || !matches!(
                    turn.get("status").and_then(Value::as_str),
                    Some("inProgress")
                )
            {
                return Err(NativeProbeError::CorrelationInvalid);
            }
        }
        "item/started" | "item/completed" => {
            valid_event_identity(params, observation)?;
            let item = params
                .get("item")
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            observe_turn_item(item, observation, method == "item/completed")?;
        }
        "item/agentMessage/delta" => {
            valid_event_identity(params, observation)?;
            let item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .filter(|value| safe_identifier(value))
                .ok_or(NativeProbeError::CorrelationInvalid)?;
            if observation
                .agent_item_id
                .as_deref()
                .is_some_and(|known| known != item_id)
            {
                return Err(NativeProbeError::CorrelationInvalid);
            }
            observation.agent_item_id = Some(item_id.to_owned());
            let nonce = observation.nonce.clone();
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            observation.agent_text.push(&nonce, delta)?;
        }
        "item/commandExecution/outputDelta" => {
            valid_event_identity(params, observation)?;
            let item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .filter(|value| safe_identifier(value))
                .ok_or(NativeProbeError::CorrelationInvalid)?;
            if observation.command_item_id.as_deref() != Some(item_id) {
                return Err(NativeProbeError::UnexpectedExecution);
            }
            let nonce = observation.nonce.clone();
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            observation.command_output.push(&nonce, delta)?;
        }
        "turn/completed" => {
            let thread_id = params
                .get("threadId")
                .and_then(Value::as_str)
                .ok_or(NativeProbeError::CorrelationInvalid)?;
            if thread_id != observation.thread_id {
                return Err(NativeProbeError::CorrelationInvalid);
            }
            let turn = params
                .get("turn")
                .ok_or(NativeProbeError::ProtocolInvalid)?;
            if turn.get("id").and_then(Value::as_str) != Some(observation.turn_id.as_str()) {
                return Err(NativeProbeError::CorrelationInvalid);
            }
            match turn.get("status").and_then(Value::as_str) {
                Some("completed") if turn.get("error").is_none_or(Value::is_null) => {}
                Some("failed" | "interrupted") => return Err(NativeProbeError::TurnFailed),
                Some("completed") => return Err(NativeProbeError::TurnFailed),
                _ => return Err(NativeProbeError::ProtocolInvalid),
            }
            if turn
                .get("itemsView")
                .is_some_and(|value| value.as_str() != Some("full"))
            {
                return Err(NativeProbeError::CorrelationInvalid);
            }
            for item in turn
                .get("items")
                .and_then(Value::as_array)
                .ok_or(NativeProbeError::ProtocolInvalid)?
            {
                observe_turn_item(item, observation, true)?;
            }
            observation.turn_completed = true;
        }
        "thread/status/changed"
        | "turn/plan/updated"
        | "item/plan/delta"
        | "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "item/reasoning/textDelta" => {
            if params.get("threadId").and_then(Value::as_str)
                != Some(observation.thread_id.as_str())
                || params
                    .get("turnId")
                    .is_some_and(|value| value.as_str() != Some(observation.turn_id.as_str()))
            {
                return Err(NativeProbeError::CorrelationInvalid);
            }
        }
        "item/commandExecution/terminalInteraction"
        | "command/exec/outputDelta"
        | "item/fileChange/outputDelta"
        | "process/outputDelta"
        | "process/exited" => return Err(NativeProbeError::UnexpectedExecution),
        "error" => return Err(NativeProbeError::RuntimeUnavailable),
        "serverRequest/resolved"
        | "thread/tokenUsage/updated"
        | "turn/diff/updated"
        | "model/verification"
        | "model/rerouted"
        | "model/safetyBuffering/updated"
        | "warning"
        | "guardianWarning"
        | "deprecationNotice"
        | "configWarning"
        | "skills/changed"
        | "hook/started"
        | "hook/completed"
        | "item/autoApprovalReview/started"
        | "item/autoApprovalReview/completed"
        | "autoApprovalReview/strictReviewRequired"
        | "remoteControl/status/changed"
        | "account/updated"
        | "account/rateLimits/updated"
        | "app/list/updated"
        | "mcpServer/startupStatus/updated"
        | "fs/changed" => {}
        _ => return Err(NativeProbeError::ProtocolInvalid),
    }
    Ok(())
}

fn preflight_notification(method: &str, message: &Value) -> Result<(), NativeProbeError> {
    match method {
        "item/started"
        | "item/completed"
        | "item/agentMessage/delta"
        | "item/commandExecution/outputDelta"
        | "item/commandExecution/terminalInteraction"
        | "command/exec/outputDelta"
        | "process/outputDelta"
        | "process/exited"
        | "turn/completed" => Err(NativeProbeError::UnexpectedExecution),
        "error" => Err(NativeProbeError::RuntimeUnavailable),
        "thread/started"
        | "thread/status/changed"
        | "turn/started"
        | "serverRequest/resolved"
        | "thread/tokenUsage/updated"
        | "skills/changed"
        | "remoteControl/status/changed"
        | "warning"
        | "guardianWarning"
        | "deprecationNotice"
        | "configWarning"
        | "account/updated"
        | "account/rateLimits/updated"
        | "app/list/updated"
        | "mcpServer/startupStatus/updated" => Ok(()),
        _ => {
            let _ = message;
            Err(NativeProbeError::ProtocolInvalid)
        }
    }
}

fn matching_command_approval(params: Option<&Value>, observation: &mut TurnObservation) -> bool {
    let Some(object) = params.and_then(Value::as_object) else {
        return false;
    };
    let thread_id = object.get("threadId").and_then(Value::as_str);
    let turn_id = object.get("turnId").and_then(Value::as_str);
    let item_id = object
        .get("itemId")
        .and_then(Value::as_str)
        .filter(|value| safe_identifier(value));
    let cwd = object.get("cwd").and_then(Value::as_str);
    let command = object.get("command").and_then(Value::as_str);
    let known_fields = [
        "threadId",
        "turnId",
        "itemId",
        "command",
        "cwd",
        "kind",
        "approvalId",
        "commandActions",
        "environmentId",
        "reason",
        "startedAtMs",
        "networkApprovalContext",
        "proposedExecpolicyAmendment",
        "proposedNetworkPolicyAmendments",
    ];
    if object
        .keys()
        .any(|key| !known_fields.contains(&key.as_str()))
    {
        return false;
    }
    let no_additional_permissions = [
        "approvalId",
        "environmentId",
        "networkApprovalContext",
        "proposedExecpolicyAmendment",
        "proposedNetworkPolicyAmendments",
    ]
    .iter()
    .all(|key| object.get(*key).is_none_or(Value::is_null));
    // The supported request schema defaults omitted kind to command.
    // It has no availableDecisions field: unknown fields are rejected above.
    // Only its schema-defined one-command accept response is ever sent.
    let kind_ok = object
        .get("kind")
        .is_none_or(|value| value.as_str() == Some("command"));
    let timestamp_present = object
        .get("startedAtMs")
        .and_then(Value::as_i64)
        .is_some_and(|value| value >= 0);
    let metadata_valid = object
        .get("reason")
        .is_none_or(|value| value.is_null() || value.is_string())
        && object.get("commandActions").is_none_or(|value| {
            value.is_null()
                || serde_json::from_value::<Vec<native_schema::CommandAction>>(value.clone())
                    .is_ok()
        });
    let matches = thread_id == Some(observation.thread_id.as_str())
        && turn_id == Some(observation.turn_id.as_str())
        && item_id.is_some()
        && cwd == Some(observation.cwd.as_str())
        && command == Some(observation.command.as_str())
        && no_additional_permissions
        && kind_ok
        && timestamp_present
        && metadata_valid;
    if matches {
        let item_id = item_id.expect("checked above");
        if observation
            .approval_item_id
            .as_deref()
            .is_some_and(|known| known != item_id)
        {
            return false;
        }
        observation.approval_item_id = Some(item_id.to_owned());
    }
    matches
}

#[cfg(windows)]
fn probe_windows(input: NativeProbeInput) -> Result<NativeProbeResult, NativeProbeError> {
    validate_input(&input)?;
    let area = TemporaryArea::create(&input.cancelled, input.deadline)?;
    // The removable source is never passed to a process API. Rebuild the
    // trusted executable from the verified MSIX member so the documented PE
    // subsystem patch on the USB copy cannot alter or invalidate execution.
    let verified_executable = verified_probe_executable(&input, &area)?;
    let hash_before = input.trusted_cli_sha256.clone();
    let runtime_version = read_runtime_version(
        &verified_executable,
        &area,
        &input.cancelled,
        input.deadline,
    )?;
    // The context is the verified desktop distribution version. The embedded
    // CLI reports a different version namespace; its identity is bound by the
    // same-executable hash and this observed version, not string equality.
    let mut prepared = prepare_probe(area, &input.model, &runtime_version, &input.gateway_base)?;
    run_catalog_gate(
        &verified_executable,
        &prepared.area,
        &input.model,
        &input.cancelled,
        input.deadline,
    )?;
    let gateway_model = fresh_gateway(&input)?;
    check_limits(&input.cancelled, input.deadline)?;

    let mut session = win_process::Session::spawn(
        &verified_executable,
        &prepared.area,
        &input.device_key,
        input.cancelled.clone(),
        input.deadline,
    )?;
    let initialize = session.request(
        "initialize",
        json!({
            "clientInfo": {"name": CLIENT_NAME, "version": CLIENT_PROTOCOL_VERSION},
            // Workspace-root/policy fields in the retained protocol are
            // negotiated explicitly; an unsupported runtime must reject them.
            "capabilities": {"experimentalApi": true}
        }),
    )?;
    validate_initialize(&initialize, &prepared.area)?;
    session.notify("initialized", None)?;
    let models = session.request(
        "model/list",
        json!({"includeHidden": true, "cursor": null, "limit": 512}),
    )?;
    validate_model_list(&models, &input.model)?;
    let skills = session.request(
        "skills/list",
        json!({"cwds": [prepared.cwd_text], "forceReload": true}),
    )?;
    let skill_path = validate_skills_list(&skills, &prepared)?;
    let thread = session.request(
        "thread/start",
        json!({
            "model": input.model.model_id,
            "modelProvider": PROVIDER_ID,
            "cwd": prepared.cwd_text,
            "runtimeWorkspaceRoots": [prepared.cwd_text],
            "sandbox": "read-only",
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "ephemeral": true
        }),
    )?;
    let thread_id = validate_thread_start(&thread, &input.model, &prepared.cwd_text)?;
    let turn = session.request(
        "turn/start",
        json!({
            "threadId": thread_id,
            // Inherit the exact acknowledged thread policy. No turn-level
            // model/cwd/permission overrides exist to require a second echo.
            "input": [{"type": "skill", "name": prepared.skill_name, "path": skill_path}]
        }),
    )?;
    let turn_id = validate_turn_start(&turn, &thread_id)?;
    let mut observation = TurnObservation::new(
        &thread_id,
        &turn_id,
        &prepared.cwd_text,
        &prepared.command,
        &prepared.nonce,
    );
    let initial_turn_completed = turn
        .get("turn")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("completed");
    if let Some(turn_snapshot) = turn.get("turn") {
        if initial_turn_completed
            && turn_snapshot
                .get("itemsView")
                .is_some_and(|value| value.as_str() != Some("full"))
        {
            return Err(NativeProbeError::CorrelationInvalid);
        }
        for item in turn_snapshot
            .get("items")
            .and_then(Value::as_array)
            .ok_or(NativeProbeError::ProtocolInvalid)?
        {
            observe_turn_item(item, &mut observation, initial_turn_completed)?;
        }
    }
    if initial_turn_completed {
        observation.turn_completed = true;
        observation.finish()?;
    } else {
        session.pump_turn(&mut observation)?;
    }
    session.shutdown()?;
    let hash_after = hash_executable(&verified_executable, &input.cancelled, input.deadline)?;
    if hash_before != hash_after {
        return Err(NativeProbeError::ExecutableChanged);
    }
    #[cfg(test)]
    let catalog_entry = prepared.catalog_entry.clone();
    prepared.area.cleanup()?;
    check_limits(&input.cancelled, input.deadline)?;
    let mut checks = gateway_model.checks.clone();
    checks.shell = CapabilityState::Verified;
    checks.skills = CapabilityState::Verified;
    checks.native_catalog = CapabilityState::Verified;
    if !checks.all_verified() {
        return Err(NativeProbeError::GatewayIncomplete);
    }
    let mut verified_context = input.context;
    if verified_context
        .native_runtime_version
        .as_ref()
        .is_some_and(|v| v != &runtime_version)
    {
        return Err(NativeProbeError::ExecutableChanged);
    }
    verified_context.native_runtime_version = Some(runtime_version.clone());
    let proof = NativeRuntimeEvidence {
        context: verified_context,
        model_id: input.model.model_id,
        owned_by: input.model.owned_by,
        checked_at: unix_seconds(),
        checks,
    };
    Ok(NativeProbeResult {
        proof,
        #[cfg(test)]
        catalog_entry,
        runtime_version,
        // Evidence and later activation remain tied to the USB executable
        // whose exact (possibly permitted PE-normalized) hash was checked by
        // verified_runtime. The process itself ran the MSIX-sourced hash.
        executable_sha256: input.expected_executable_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unverified_runtime_is_never_passed_to_codex_process_runner() {
        let launched = std::sync::atomic::AtomicBool::new(false);
        let result = run_after_runtime_verification::<()>(
            || Err(NativeProbeError::ExecutableChanged),
            |_| {
                launched.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert_eq!(result, Err(NativeProbeError::ExecutableChanged));
        assert!(!launched.load(Ordering::Acquire));
    }

    #[test]
    fn verified_runtime_path_is_the_only_path_passed_to_codex_process_runner() {
        let expected = PathBuf::from("verified-codex.exe");
        let observed = std::sync::Mutex::new(None);
        let result = run_after_runtime_verification(
            || Ok(expected.clone()),
            |path| {
                *observed.lock().unwrap() = Some(path.to_path_buf());
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(*observed.lock().unwrap(), Some(expected));
    }

    #[test]
    fn executable_copy_must_match_signed_runtime_hash_before_runner_is_called() {
        let root = std::env::temp_dir().join(format!(
            "codex-verified-copy-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("payload.msix");
        let entry = "app/resources/codex.exe";
        let trusted = b"trusted signed package bytes";
        let mut hash = Sha256::new();
        hash.update(trusted);
        let expected: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        write_test_msix_cli(&source, entry, b"changed after verification");
        let launched = std::sync::atomic::AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        let result = run_after_runtime_verification(
            || {
                verified_probe_executable_path(
                    &source,
                    entry,
                    &expected,
                    &root,
                    &cancelled,
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                )?;
                Ok(root.join("codex-native-probe-verified.exe"))
            },
            |_| {
                launched.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert_eq!(result, Err(NativeProbeError::ExecutableChanged));
        assert!(!launched.load(Ordering::Acquire));
        assert!(!root.join("codex-native-probe-verified.exe").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capability_probe_runner_receives_copy_extracted_from_signed_msix_entry() {
        let root = std::env::temp_dir().join(format!(
            "codex-native-probe-copy-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("payload.msix");
        let entry = "app/resources/codex.exe";
        let expected_bytes = b"signed MSIX CLI bytes";
        write_test_msix_cli(&source, entry, expected_bytes);
        let mut hasher = Sha256::new();
        hasher.update(expected_bytes);
        let expected: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let cancelled = AtomicBool::new(false);
        let launched_from = std::sync::Mutex::new(None);
        let result = run_after_runtime_verification(
            || {
                verified_probe_executable_path(
                    &source,
                    entry,
                    &expected,
                    &root,
                    &cancelled,
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                )
            },
            |path| {
                *launched_from.lock().unwrap() = Some(path.to_path_buf());
                assert_ne!(path, source.as_path());
                assert_eq!(std::fs::read(path).unwrap(), expected_bytes);
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(
            *launched_from.lock().unwrap(),
            Some(root.join("codex-native-probe-verified.exe"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extraction_uses_verified_snapshot_after_usb_source_path_is_replaced() {
        let root = std::env::temp_dir().join(format!(
            "codex-native-probe-snapshot-swap-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("usb-source.msix");
        let snapshot = root.join("verified-snapshot.msix");
        let entry = "app/resources/codex.exe";
        let trusted_cli = b"CLI bytes from validated package snapshot";
        write_test_msix_cli(&source, entry, trusted_cli);
        std::fs::copy(&source, &snapshot).unwrap();

        let mut digest = Sha256::new();
        digest.update(trusted_cli);
        let expected: String = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        write_test_msix_cli(&source, entry, b"replacement package after validation");

        let output = root.join("codex-verified.exe");
        extract_verified_cli_from_msix(
            &snapshot,
            entry,
            &expected,
            &output,
            &AtomicBool::new(false),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), trusted_cli);
        assert!(extract_verified_cli_from_msix(
            &source,
            entry,
            &expected,
            &root.join("must-not-exist.exe"),
            &AtomicBool::new(false),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    fn write_test_msix_cli(path: &Path, entry: &str, bytes: &[u8]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(entry, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn msix_cli_entry_path_rejects_zip_traversal() {
        assert!(valid_msix_cli_entry("app/resources/codex.exe"));
        assert!(!valid_msix_cli_entry("../app/resources/codex.exe"));
        assert!(!valid_msix_cli_entry("app/../resources/codex.exe"));
        assert!(!valid_msix_cli_entry("app\\resources\\codex.exe"));
    }

    #[test]
    fn generated_neutral_catalog_entry_uses_codex_required_fields() {
        let model = ModelCapabilityReport::discovered("vendor/model".into(), None);
        let entry = neutral_catalog_entry(&model);

        assert!(catalog_is_neutral(&entry));
        assert_eq!(
            entry["truncation_policy"],
            json!({"mode":"bytes","limit":10000})
        );
        assert_eq!(entry["base_instructions"], MODEL_BASE_INSTRUCTIONS);
        for key in [
            "default_reasoning_summary",
            "web_search_tool_type",
            "effective_context_window_percent",
        ] {
            assert!(
                entry.get(key).is_none(),
                "unexpected null/default field: {key}"
            );
        }
    }

    #[test]
    fn native_default_models_are_parsed_without_reordering_or_duplicates() {
        let output = json!({
            "models": [
                {"slug":"gpt-native-a","shell_type":"unified_exec"},
                {"slug":"gpt-native-b","shell_type":"unified_exec"}
            ]
        });
        let bytes = serde_json::to_vec(&output).unwrap();
        let models = native_models_from_debug_output(&bytes).unwrap();
        assert_eq!(models, output["models"].as_array().unwrap().clone());
        assert!(native_models_from_debug_output(b"not json").is_err());
        assert!(native_models_from_debug_output(
            br#"{"models":[{"slug":"duplicate"},{"slug":"duplicate"}]}"#
        )
        .is_err());
        assert!(native_models_from_debug_output(
            br#"{"models":[{"slug":"valid"}],"unexpected":true}"#
        )
        .is_err());
    }

    #[test]
    fn native_catalog_admission_matches_the_codex_debug_models_output() {
        let entry = json!({"slug":"arbitrary/model","shell_type":"unified_exec"});
        let expected = neutral_catalog_root(&entry, "fixture-runtime");
        let normalized = json!({
            "slug":"arbitrary/model",
            "shell_type":"unified_exec",
            "model_messages": {
                "instructions_template":null,
                "instructions_variables":null,
                "approvals":null,
                "collaboration_modes":null,
                "auto_review":null,
                "permissions":null,
                "multi_agent":null
            },
            "default_reasoning_summary":"auto",
            "web_search_tool_type":"text",
            "effective_context_window_percent":95,
            "supports_experimental_context":false,
            "use_responses_lite":false,
            "supports_reasoning_effort_updates":false
        });
        let runtime_output = json!({"models":[normalized]});
        assert!(!catalog_output_matches(
            b"warning mentions arbitrary/model",
            &expected
        ));
        assert!(catalog_output_matches(
            &serde_json::to_vec(&runtime_output).unwrap(),
            &expected
        ));
        let mut changed = runtime_output.clone();
        changed["models"][0]["shell_type"] = json!("other");
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&changed).unwrap(),
            &expected
        ));
        let mut added = runtime_output.clone();
        added["models"][0]["use_responses_lite"] = json!(true);
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&added).unwrap(),
            &expected
        ));
        let mut duplicate = runtime_output.clone();
        duplicate["models"] = json!([normalized, normalized]);
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&duplicate).unwrap(),
            &expected
        ));
        for key in ["schema_version", "identity", "fetched_at"] {
            let mut wrong = expected.clone();
            wrong[key] = json!("different");
            assert!(!catalog_output_matches(
                &serde_json::to_vec(&runtime_output).unwrap(),
                &wrong
            ));
            wrong.as_object_mut().unwrap().remove(key);
            assert!(!catalog_output_matches(
                &serde_json::to_vec(&runtime_output).unwrap(),
                &wrong
            ));
        }
        let mut invalid_client_version = expected.clone();
        invalid_client_version["client_version"] = json!("");
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&runtime_output).unwrap(),
            &invalid_client_version
        ));
        invalid_client_version
            .as_object_mut()
            .unwrap()
            .remove("client_version");
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&runtime_output).unwrap(),
            &invalid_client_version
        ));
        let mut extra = runtime_output.clone();
        extra["unknownRoot"] = json!(true);
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&extra).unwrap(),
            &expected
        ));
    }

    #[test]
    fn native_catalog_admission_accepts_only_omitted_null_optional_fields() {
        let entry = json!({
            "slug":"claude-haiku-4-5",
            "display_name":"claude-haiku-4-5",
            "description":"Text model supplied by the configured gateway.",
            "shell_type":"unified_exec",
            "visibility":"list",
            "supported_in_api":true,
            "priority":0,
            "support_verbosity":false,
            "additional_speed_tiers":[],
            "service_tiers":[],
            "availability_nux":null,
            "upgrade":null,
            "include_skills_usage_instructions":false,
            "include_plugin_usage_instructions":false,
            "include_apps_usage_instructions":false,
            "default_verbosity":null,
            "apply_patch_tool_type":null,
            "truncation_policy":{"mode":"bytes","limit":10000},
            "base_instructions":"Use this model for the current conversation.",
            "supports_image_detail_original":false,
            "comp_hash":null,
            "experimental_supported_tools":[],
            "input_modalities":["text"],
            "supports_search_tool":false,
            "node_repl_auto_review_required":false,
            "node_repl_disabled":true,
            "default_reasoning_level":null,
            "supported_reasoning_levels":[]
        });
        let expected = neutral_catalog_root(&entry, "codex-cli 0.162.0-alpha.2");
        let mut normalized = entry.clone();
        normalized.as_object_mut().unwrap().remove("comp_hash");
        normalized
            .as_object_mut()
            .unwrap()
            .remove("default_reasoning_level");
        normalized["model_messages"] = json!({
            "instructions_template":"Use this model for the current conversation.",
            "instructions_variables":null,
            "approvals":null,
            "collaboration_modes":null,
            "auto_review":null,
            "permissions":null,
            "multi_agent":null
        });
        normalized["default_reasoning_summary"] = json!("auto");
        normalized["web_search_tool_type"] = json!("text");
        normalized["effective_context_window_percent"] = json!(95);
        normalized["supports_experimental_context"] = json!(false);
        normalized["use_responses_lite"] = json!(false);
        normalized["supports_reasoning_effort_updates"] = json!(false);
        let output = json!({"models":[normalized]});
        let output_bytes = serde_json::to_vec(&output).unwrap();

        assert!(catalog_output_matches(&output_bytes, &expected));

        let mut missing_required = output.clone();
        missing_required["models"][0]
            .as_object_mut()
            .unwrap()
            .remove("display_name");
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&missing_required).unwrap(),
            &expected
        ));

        for key in ["comp_hash", "default_reasoning_level"] {
            let mut explicit_null = output.clone();
            explicit_null["models"][0][key] = Value::Null;
            assert!(catalog_output_matches(
                &serde_json::to_vec(&explicit_null).unwrap(),
                &expected
            ));

            let mut wrong_null_normalization = output.clone();
            wrong_null_normalization["models"][0][key] = json!("unexpected");
            assert!(!catalog_output_matches(
                &serde_json::to_vec(&wrong_null_normalization).unwrap(),
                &expected
            ));
        }

        let mut unknown_null = expected.clone();
        unknown_null["models"][0]["future_optional_field"] = Value::Null;
        assert!(!catalog_output_matches(&output_bytes, &unknown_null));

        let mut altered_identity = expected.clone();
        altered_identity["identity"] = json!("another-provider");
        assert!(!catalog_output_matches(&output_bytes, &altered_identity));

        let mut duplicate = output;
        duplicate["models"] = json!([
            duplicate["models"][0].clone(),
            duplicate["models"][0].clone()
        ]);
        assert!(!catalog_output_matches(
            &serde_json::to_vec(&duplicate).unwrap(),
            &expected
        ));
    }

    #[test]
    fn native_catalog_and_cli_capture_limits_cover_the_supported_catalog_size() {
        assert_eq!(MAX_CATALOG_BYTES, crate::app::model_probe::BODY_LIMIT);
        const { assert!(MAX_CAPTURE_BYTES >= MAX_CATALOG_BYTES * 2) };
        assert_eq!(MAX_CAPTURE_BYTES, 2 * 1024 * 1024);
        for (limit, allows) in [
            (MAX_CATALOG_BYTES - 1, true),
            (MAX_CATALOG_BYTES, true),
            (MAX_CATALOG_BYTES + 1, false),
        ] {
            assert_eq!(within_catalog_limit(limit), allows);
        }
        for (limit, allows) in [
            (MAX_CAPTURE_BYTES - 1, true),
            (MAX_CAPTURE_BYTES, true),
            (MAX_CAPTURE_BYTES + 1, false),
        ] {
            assert_eq!(within_capture_limit(limit), allows);
        }
    }

    #[test]
    fn approval_requires_supported_schema_and_exact_identity() {
        let mut observed = TurnObservation::new(
            "thread-fixture",
            "turn-fixture",
            "C:/fixture",
            "echo fixture",
            "nonce",
        );
        let mut request = json!({"threadId":"thread-fixture","turnId":"turn-fixture","itemId":"item-fixture",
            "cwd":"C:/fixture","command":"echo fixture"});
        assert!(!matching_command_approval(Some(&request), &mut observed));
        request["startedAtMs"] = json!(1000);
        assert!(matching_command_approval(Some(&request), &mut observed));
        for value in [json!("stdin"), Value::Null] {
            let mut bad = request.clone();
            bad["kind"] = value;
            assert!(!matching_command_approval(Some(&bad), &mut observed));
        }
        for value in [json!(["accept", "cancel"]), json!(["cancel"]), Value::Null] {
            let mut bad = request.clone();
            bad["availableDecisions"] = value;
            assert!(!matching_command_approval(Some(&bad), &mut observed));
        }
        for key in ["additionalPermissions", "unknownSecurityGrant"] {
            let mut bad = request.clone();
            bad[key] = Value::Null;
            assert!(!matching_command_approval(Some(&bad), &mut observed));
        }
        for (key, value) in [
            ("environmentId", json!("other")),
            ("startedAtMs", json!("1000")),
            ("reason", json!(true)),
            (
                "commandActions",
                json!([{"type":"read","command":"fixture"}]),
            ),
        ] {
            let mut bad = request.clone();
            bad[key] = value;
            assert!(!matching_command_approval(Some(&bad), &mut observed));
        }
        request["kind"] = json!("command");
        request["commandActions"] = json!([{"type":"unknown","command":"echo fixture"}]);
        assert!(matching_command_approval(Some(&request), &mut observed));
    }

    #[test]
    fn negative_server_replies_use_the_method_schema_and_error_envelope() {
        let id = json!("fixture-id");
        for method in ["applyPatchApproval", "execCommandApproval"] {
            assert_eq!(
                server_request_rejection(&id, method),
                json!({"id":id,"result":{"decision":"abort"}})
            );
        }
        for method in [
            "item/fileChange/requestApproval",
            "item/commandExecution/requestApproval",
        ] {
            assert_eq!(
                server_request_rejection(&id, method),
                json!({"id":id,"result":{"decision":"cancel"}})
            );
        }
        let unknown = server_request_rejection(&id, "unsupported-fixture-method");
        assert_eq!(unknown["id"], id);
        assert_eq!(unknown["error"]["code"], json!(-32601));
        assert!(unknown.get("result").is_none());
    }

    #[test]
    fn model_list_rejects_missing_required_fields_and_malformed_optional_shapes() {
        let model = ModelCapabilityReport::discovered("fixture-model".into(), None);
        let entry = json!({"id":"fixture-model","model":"fixture-model","description":"fixture",
            "displayName":"Fixture","hidden":false,"isDefault":false,"defaultReasoningEffort":"none",
            "supportedReasoningEfforts":[{"description":"fixture","reasoningEffort":"none"}]});
        let payload = |entry: Value| json!({"data":[entry],"nextCursor":null});
        assert!(validate_model_list(&payload(entry.clone()), &model).is_ok());
        for key in [
            "id",
            "model",
            "description",
            "displayName",
            "hidden",
            "isDefault",
            "defaultReasoningEffort",
            "supportedReasoningEfforts",
        ] {
            let mut bad = entry.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(validate_model_list(&payload(bad), &model).is_err());
        }
        for (key, value) in [
            ("hidden", json!("false")),
            ("isDefault", json!(1)),
            ("description", json!(true)),
            ("defaultReasoningEffort", json!("")),
            (
                "supportedReasoningEfforts",
                json!([{"reasoningEffort":"none"}]),
            ),
            ("serviceTiers", json!([{"id":"x"}])),
            ("inputModalities", json!(["other"])),
            ("upgradeInfo", json!({"retirementAt":"tomorrow"})),
            ("unknownField", json!(true)),
        ] {
            let mut bad = entry.clone();
            bad[key] = value;
            assert!(validate_model_list(&payload(bad), &model).is_err());
        }
    }

    #[test]
    fn skills_list_rejects_partial_metadata_and_invalid_nested_fields() {
        let skill = json!({"name":"fixture","path":"C:/fixture/SKILL.md","description":"fixture","enabled":true,"scope":"repo"});
        let payload =
            |skill: Value| json!({"data":[{"cwd":"C:/fixture","errors":[],"skills":[skill]}]});
        assert!(validate_skills_list_shape(&payload(skill.clone())).is_ok());
        for key in ["name", "path", "description", "enabled", "scope"] {
            let mut bad = skill.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(validate_skills_list_shape(&payload(bad)).is_err());
        }
        for (key, value) in [
            ("scope", json!("other")),
            ("enabled", json!("true")),
            ("description", json!(1)),
            ("dependencies", json!({"tools":[{"value":"x"}]})),
            ("interface", json!({"displayName":false})),
            ("unknownField", Value::Null),
        ] {
            let mut bad = skill.clone();
            bad[key] = value;
            assert!(validate_skills_list_shape(&payload(bad)).is_err());
        }
        let mut bad = payload(skill);
        bad["data"][0]["errors"] = json!([{"path":"fixture"}]);
        assert!(validate_skills_list_shape(&bad).is_err());
    }

    #[test]
    fn native_thread_requires_the_explicit_user_reviewer() {
        let model = ModelCapabilityReport::discovered("fixture-model".into(), None);
        let result = json!({"model":"fixture-model","modelProvider":PROVIDER_ID,"cwd":"C:/fixture",
            "runtimeWorkspaceRoots":["C:/fixture"],"sandbox":{"type":"readOnly","networkAccess":false},
            "approvalPolicy":"on-request","approvalsReviewer":"user","thread":{"id":"thread-fixture"}});
        assert!(validate_thread_start(&result, &model, "C:/fixture").is_ok());
        let mut missing = result.clone();
        missing.as_object_mut().unwrap().remove("approvalsReviewer");
        assert!(validate_thread_start(&missing, &model, "C:/fixture").is_err());
        for value in [
            Value::Null,
            json!("auto_review"),
            json!("guardian_subagent"),
            json!(true),
        ] {
            let mut bad = result.clone();
            bad["approvalsReviewer"] = value;
            assert!(validate_thread_start(&bad, &model, "C:/fixture").is_err());
        }
    }

    fn observation() -> TurnObservation {
        TurnObservation::new(
            "thread-fixture",
            "turn-fixture",
            r"C:\probe-workspace",
            "[Console]::Write([IO.File]::ReadAllText('C:\\probe-workspace\\payload.txt'))",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
    }

    fn command_item(id: &str, status: &str, output: &str) -> Value {
        json!({
            "id": id,
            "type": "commandExecution",
            "source": "agent",
            "command": "[Console]::Write([IO.File]::ReadAllText('C:\\probe-workspace\\payload.txt'))",
            "cwd": "C:\\probe-workspace",
            "status": status,
            "exitCode": if status == "completed" { Value::from(0) } else { Value::Null },
            "aggregatedOutput": if status == "completed" { Value::from(output) } else { Value::Null },
            "processId": null
        })
    }

    #[test]
    fn state_identity_rejects_cross_thread_or_turn_events() {
        let state = observation();
        assert!(state
            .check_identity("thread-fixture", "turn-fixture")
            .is_ok());
        assert_eq!(
            state.check_identity("other-thread", "turn-fixture"),
            Err(NativeProbeError::CorrelationInvalid)
        );
        assert_eq!(
            state.check_identity("thread-fixture", "other-turn"),
            Err(NativeProbeError::CorrelationInvalid)
        );
    }

    #[test]
    fn extra_command_item_is_rejected_even_when_its_text_matches() {
        let mut state = observation();
        assert!(state
            .record_command_item(&command_item("item-one", "inProgress", ""), false)
            .is_ok());
        assert_eq!(
            state.record_command_item(&command_item("item-two", "inProgress", ""), false),
            Err(NativeProbeError::UnexpectedExecution)
        );
    }

    #[test]
    fn nonce_mismatch_does_not_pass_aggregated_output() {
        let mut state = observation();
        assert!(state
            .record_command_item(&command_item("item-one", "inProgress", ""), false)
            .is_ok());
        assert_eq!(
            state.record_command_item(&command_item("item-one", "completed", "wrong"), true),
            Err(NativeProbeError::NonceMismatch)
        );
    }

    #[test]
    fn failed_execution_is_not_treated_as_a_partial_success() {
        let mut state = observation();
        assert_eq!(
            state.record_command_item(&command_item("item-one", "failed", ""), true),
            Err(NativeProbeError::TurnFailed)
        );
    }

    #[test]
    fn unsupported_is_static_and_does_not_include_runtime_details() {
        assert_eq!(
            NativeProbeError::UnsupportedPlatform.code(),
            "native-platform-unsupported"
        );
    }
}
