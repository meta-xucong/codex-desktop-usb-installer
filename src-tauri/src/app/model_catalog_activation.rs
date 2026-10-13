//! Native evidence orchestration and finite activation; disk state is never proof.
use super::*;
use crate::app::codex_config_readback;
use crate::app::model_capability::{CapabilityState, EvidenceScope, NativeRuntimeEvidence};
use crate::app::model_native_probe::{self, NativeProbeInput, NativeProbeResult};
use crate::app::model_selection_client::{self, SelectionReply, SelectionRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;

pub const NATIVE_REFRESH_BUDGET: Duration = Duration::from_secs(600);
pub const AUTO_SETUP_BUDGET: Duration = Duration::from_secs(1920);
pub const AUTO_SETUP_APPLY_RESERVE: Duration = Duration::from_secs(120);
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeVerificationOptions {
    pub cost_consent: bool,
    #[serde(default)]
    pub start_new_operation: bool,
}
impl NativeVerificationOptions {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.cost_consent {
            return Err(engine("须先确认真实模型探测可能产生无法预估的用量和费用"));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct NativeRuntimeTarget {
    pub installation_root: PathBuf,
    pub desktop_version: String,
}
pub(crate) struct VerifiedRuntime {
    /// Current USB CLI path, retained only for identity and mutation checks.
    pub executable: PathBuf,
    pub executable_sha256: String,
    /// The original CLI is always executed from this verified MSIX member,
    /// never from the subsystem-patched USB copy.
    pub trusted_payload_path: PathBuf,
    pub trusted_cli_entry_path: String,
    pub trusted_cli_sha256: String,
}
pub struct NativeBatch {
    pub results: Vec<NativeProbeResult>,
    pub selection: Option<SelectionReply>,
    pub executable: Option<PathBuf>,
    pub error: Option<&'static str>,
}
impl NativeBatch {
    pub fn empty() -> Self {
        Self {
            results: Vec::new(),
            selection: None,
            executable: None,
            error: None,
        }
    }
    pub fn proofs(&self) -> Vec<NativeRuntimeEvidence> {
        self.results.iter().map(|r| r.proof.clone()).collect()
    }
}
struct LiveCatalog {
    setup_complete: bool,
    binding: codex_usb_license::UsbAccountBinding,
    context: CapabilityContext,
    selected_model: String,
    selected_owner: Option<String>,
    proofs: Vec<NativeRuntimeEvidence>,
    models: Vec<ModelCapabilityReport>,
    executable: PathBuf,
    executable_sha256: String,
    catalog_sha256: String,
    config_sha256: String,
    auth_sha256: String,
}
static LIVE_CATALOG: Mutex<Option<LiveCatalog>> = Mutex::new(None);
pub fn invalidate_live() {
    *LIVE_CATALOG.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
fn check(cancelled: &AtomicBool, deadline: Instant) -> Result<(), &'static str> {
    if cancelled.load(AtomicOrdering::Acquire) {
        return Err("native-probe-cancelled");
    }
    if Instant::now() >= deadline {
        return Err("native-probe-timeout");
    }
    Ok(())
}
fn hash_runtime(
    path: &Path,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<String, &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "native-executable-unavailable")?;
    if !meta.is_file() || meta.len() > 1024 * 1024 * 1024 {
        return Err("native-executable-unavailable");
    }
    let mut input = fs::File::open(path).map_err(|_| "native-executable-unavailable")?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut hash = Sha256::new();
    let mut read = 0u64;
    loop {
        check(cancelled, deadline)?;
        let size = input
            .read(&mut buffer)
            .map_err(|_| "native-executable-unavailable")?;
        if size == 0 {
            break;
        }
        read += size as u64;
        if read > meta.len() {
            return Err("native-executable-changed");
        }
        hash.update(&buffer[..size]);
    }
    if read != meta.len() {
        return Err("native-executable-changed");
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

struct RuntimeHashes {
    raw_sha256: String,
    normalized_console_sha256: Option<String>,
}

/// Hash the original executable bytes and, when the only portable change is
/// PE Subsystem 3→2, the console-normalized bytes during the same streaming
/// observation. The raw digest returned here must be the same byte stream
/// whose normalized digest is compared with the signed MSIX member.
fn hash_runtime_with_console_normalization(
    path: &Path,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<RuntimeHashes, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "native-executable-unavailable")?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
        return Err("native-executable-unavailable");
    }
    let mut input = fs::File::open(path).map_err(|_| "native-executable-unavailable")?;
    hash_runtime_reader_with_console_normalization(&mut input, metadata.len(), cancelled, deadline)
}

fn hash_runtime_reader_with_console_normalization(
    input: &mut impl Read,
    expected_len: u64,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<RuntimeHashes, &'static str> {
    if expected_len < 64 || expected_len > 1024 * 1024 * 1024 {
        return Err("native-executable-unavailable");
    }
    let mut prefix = Vec::with_capacity(1024 * 1024 + 96);
    read_prefix_to(input, &mut prefix, 64, expected_len)?;
    let normalization = if &prefix[..2] == b"MZ" {
        let nt_offset = u32::from_le_bytes(prefix[0x3c..0x40].try_into().unwrap()) as usize;
        if nt_offset <= 1024 * 1024 && nt_offset.saturating_add(94) <= expected_len as usize {
            read_prefix_to(input, &mut prefix, nt_offset + 24, expected_len)?;
            let nt = &prefix[nt_offset..nt_offset + 24];
            let optional_size = u16::from_le_bytes([nt[20], nt[21]]) as usize;
            let subsystem_offset = nt_offset + 24 + 68;
            if &nt[..4] == b"PE\0\0"
                && optional_size >= 70
                && subsystem_offset.saturating_add(2) <= expected_len as usize
            {
                read_prefix_to(input, &mut prefix, subsystem_offset + 2, expected_len)?;
                let subsystem = [prefix[subsystem_offset], prefix[subsystem_offset + 1]];
                (u16::from_le_bytes(subsystem) == 2).then_some((subsystem_offset, subsystem))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut raw_hash = Sha256::new();
    let mut normalized_hash = normalization.map(|_| Sha256::new());
    raw_hash.update(&prefix);
    if let (Some((subsystem_offset, _)), Some(hash)) = (normalization, normalized_hash.as_mut()) {
        let mut restored_prefix = prefix.clone();
        restored_prefix[subsystem_offset..subsystem_offset + 2]
            .copy_from_slice(&3u16.to_le_bytes());
        hash.update(&restored_prefix);
    }
    let mut read = prefix.len() as u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        check(cancelled, deadline)?;
        let size = input
            .read(&mut buffer)
            .map_err(|_| "native-executable-unavailable")?;
        if size == 0 {
            break;
        }
        read = read
            .checked_add(size as u64)
            .filter(|count| *count <= expected_len)
            .ok_or("native-executable-changed")?;
        raw_hash.update(&buffer[..size]);
        if let Some(hash) = normalized_hash.as_mut() {
            hash.update(&buffer[..size]);
        }
    }
    if read != expected_len {
        return Err("native-executable-changed");
    }
    Ok(RuntimeHashes {
        raw_sha256: raw_hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        normalized_console_sha256: normalized_hash.map(|hash| {
            hash.finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }),
    })
}

fn read_prefix_to(
    input: &mut impl Read,
    prefix: &mut Vec<u8>,
    target: usize,
    expected_len: u64,
) -> Result<(), &'static str> {
    if target as u64 > expected_len || target > 1024 * 1024 + 96 {
        return Err("native-executable-changed");
    }
    while prefix.len() < target {
        let remaining = target - prefix.len();
        let mut chunk = vec![0u8; remaining.min(64 * 1024)];
        let count = input
            .read(&mut chunk)
            .map_err(|_| "native-executable-unavailable")?;
        if count == 0 {
            return Err("native-executable-changed");
        }
        prefix.extend_from_slice(&chunk[..count]);
    }
    Ok(())
}
#[cfg(windows)]
fn verified_runtime(
    target: &NativeRuntimeTarget,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedRuntime, &'static str> {
    verified_runtime_inner(
        &target.installation_root,
        Some(&target.desktop_version),
        cancelled,
        deadline,
    )
}

#[cfg(windows)]
pub(crate) fn verified_catalog_runtime(
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedRuntime, &'static str> {
    if !usb_guard::guard_required() {
        return Err("native-usb-required");
    }
    let root = usb_guard::authorized_usb_codex_root().map_err(|_| "native-binding-unavailable")?;
    verified_runtime_inner(&root, None, cancelled, deadline)
}

#[cfg(windows)]
fn verified_runtime_inner(
    installation_root: &Path,
    expected_version: Option<&str>,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedRuntime, &'static str> {
    use std::os::windows::fs::MetadataExt;
    if !usb_guard::guard_required() {
        return Err("native-usb-required");
    }
    usb_guard::require_signed_codex_bundle("native-model-verification")
        .map_err(|_| "native-binding-unavailable")?
        .ok_or("native-usb-required")?;
    let status = usb_guard::current_status();
    let bundle = status
        .bundle_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or("native-usb-required")?;
    for raw in [installation_root, &bundle.join("apps").join("Codex")] {
        for ancestor in raw.ancestors() {
            let meta =
                fs::symlink_metadata(ancestor).map_err(|_| "native-installation-unavailable")?;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("native-installation-mismatch");
            }
        }
    }
    let expected_root = fs::canonicalize(bundle.join("apps").join("Codex"))
        .map_err(|_| "native-installation-unavailable")?;
    let actual_root =
        fs::canonicalize(installation_root).map_err(|_| "native-installation-unavailable")?;
    if actual_root != expected_root {
        return Err("native-installation-mismatch");
    }
    let executable = actual_root.join("resources").join("codex.exe");
    for p in executable.ancestors() {
        let meta = fs::symlink_metadata(p).map_err(|_| "native-executable-unavailable")?;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("native-executable-unavailable");
        }
    }
    let runtime_hashes = hash_runtime_with_console_normalization(&executable, cancelled, deadline)?;
    // Packaged internal executables need not carry an individual signature.
    // Compare their bytes with the original, signature-verified MSIX instead.
    check(cancelled, deadline)?;
    let manager = env::current_exe().map_err(|_| "native-package-unavailable")?;
    // Stage first: the release-manifest hash and Authenticode check are both
    // applied to a local package snapshot. All later ZIP reads use this same
    // staged package, never the mutable removable-drive path.
    let staged = crate::app::offline_payload::stage_offline_payload(&manager, None, None, true)
        .map_err(|_| "native-package-unavailable")?;
    let trusted_payload_path = PathBuf::from(&staged.staged_path);
    let payload = staged.payload;
    if !payload.hash_verified
        || !payload
            .authenticode
            .as_ref()
            .is_some_and(|report| report.trusted && report.publisher_is_openai)
    {
        return Err("native-package-unavailable");
    }
    let staged_identity = codex_win_engine::read_msix_identity(&trusted_payload_path)
        .map_err(|_| "native-package-unavailable")?;
    codex_win_engine::validate_codex_identity(
        &staged_identity,
        &staged_identity.version,
        Some(payload.architecture.as_str()),
    )
    .map_err(|_| "native-package-unavailable")?;
    if codex_win_engine::read_codex_app_version_from_msix(&trusted_payload_path)
        .is_some_and(|version| version != payload.codex_desktop_version)
    {
        return Err("native-package-version-mismatch");
    }
    if expected_version.is_some_and(|version| payload.codex_desktop_version != version) {
        return Err("native-package-version-mismatch");
    }
    let entry_path = crate::app::offline_payload::read_app_entry(&trusted_payload_path)
        .map_err(|_| "native-package-unavailable")?
        .as_deref()
        .ok_or("native-package-unavailable")?
        .replace('\\', "/");
    let parent = entry_path
        .rsplit_once('/')
        .map(|(p, _)| p)
        .ok_or("native-package-unavailable")?;
    if parent.is_empty()
        || parent
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err("native-package-unavailable");
    }
    let trusted_cli_entry_path = format!("{parent}/resources/codex.exe");
    let file = fs::File::open(&trusted_payload_path).map_err(|_| "native-package-unavailable")?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "native-package-unavailable")?;
    let mut entry = archive
        .by_name(&trusted_cli_entry_path)
        .map_err(|_| "native-package-unavailable")?;
    if entry.size() > 1024 * 1024 * 1024 {
        return Err("native-package-unavailable");
    }
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        check(cancelled, deadline)?;
        let n = entry
            .read(&mut buffer)
            .map_err(|_| "native-package-unavailable")?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let packaged: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if packaged != runtime_hashes.raw_sha256
        && runtime_hashes.normalized_console_sha256.as_deref() != Some(packaged.as_str())
    {
        return Err("native-executable-changed");
    }
    Ok(VerifiedRuntime {
        executable,
        executable_sha256: runtime_hashes.raw_sha256,
        trusted_payload_path,
        trusted_cli_entry_path,
        trusted_cli_sha256: packaged,
    })
}
#[cfg(not(windows))]
fn verified_runtime(
    _: &NativeRuntimeTarget,
    _: &AtomicBool,
    _: Instant,
) -> Result<VerifiedRuntime, &'static str> {
    Err("native-platform-unsupported")
}

#[cfg(not(windows))]
pub(crate) fn verified_catalog_runtime(
    _: &AtomicBool,
    _: Instant,
) -> Result<VerifiedRuntime, &'static str> {
    Err("native-platform-unsupported")
}

fn complete_gateway(model: &ModelCapabilityReport) -> bool {
    model.evidence_scope == EvidenceScope::Gateway
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
pub async fn collect(
    models: &mut [ModelCapabilityReport],
    context: &CapabilityContext,
    target: Option<&NativeRuntimeTarget>,
    binding: &codex_usb_license::UsbAccountBinding,
    base: &str,
    home: &Path,
    run_id: &str,
    candidate_deadlines: &BTreeMap<String, Instant>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
) -> NativeBatch {
    let mut batch = NativeBatch::empty();
    let Some(target) = target.cloned() else {
        batch.error = Some("native-installation-unavailable");
        return batch;
    };
    let flag = cancelled.clone();
    let verified =
        tauri::async_runtime::spawn_blocking(move || verified_runtime(&target, &flag, deadline))
            .await;
    let verified_runtime = match verified {
        Ok(Ok(value)) => value,
        Ok(Err(code)) => {
            batch.error = Some(code);
            return batch;
        }
        Err(_) => {
            batch.error = Some("native-worker-failed");
            return batch;
        }
    };
    let executable = verified_runtime.executable.clone();
    let expected_hash = verified_runtime.executable_sha256.clone();
    batch.executable = Some(executable.clone());
    let snapshot_binding = binding.clone();
    let snapshot_home = home.to_path_buf();
    let snapshot_id = format!("snapshot-{}", uuid::Uuid::new_v4().simple());
    let flag = cancelled.clone();
    batch.selection = match tauri::async_runtime::spawn_blocking(move || {
        model_selection_client::request(
            SelectionRequest {
                schema_version: 3,
                request_id: &snapshot_id,
                operation: "snapshot",
                expected_revision: None,
                model: None,
                catalog_path: None,
                catalog_sha256: None,
            },
            &snapshot_binding,
            &flag,
            &snapshot_home,
        )
    })
    .await
    {
        Ok(Ok(reply)) => Some(reply),
        Ok(Err(code)) => {
            batch.error = Some(code);
            None
        }
        Err(_) => {
            batch.error = Some("selection-worker-failed");
            None
        }
    };
    // A missing configuration helper does not fabricate native capabilities.
    // Native evidence may still be collected and reported without activation.
    for model in models {
        if !complete_gateway(model) {
            log::info!("codex_native_model_probe run_id={} model={} outcome=skipped code=incomplete-gateway-checks", run_id, model.model_id);
            continue;
        }
        let candidate_deadline = candidate_deadlines
            .get(&model.model_id)
            .copied()
            .unwrap_or(deadline);
        if let Err(code) = check(&cancelled, candidate_deadline) {
            model.error_code = Some(code.into());
            continue;
        }
        let model_id = model.model_id.clone();
        let input = NativeProbeInput {
            executable: executable.clone(),
            expected_executable_sha256: expected_hash.clone(),
            trusted_payload_path: verified_runtime.trusted_payload_path.clone(),
            trusted_cli_entry_path: verified_runtime.trusted_cli_entry_path.clone(),
            trusted_cli_sha256: verified_runtime.trusted_cli_sha256.clone(),
            context: context.clone(),
            model: model.clone(),
            gateway_base: base.into(),
            device_key: binding.ccswitch.device_api_key.clone(),
            cancelled: cancelled.clone(),
            deadline: candidate_deadline,
        };
        match tauri::async_runtime::spawn_blocking(move || model_native_probe::probe(input)).await {
            Ok(Ok(result)) if result.executable_sha256 == expected_hash => {
                log::info!(
                    "codex_native_model_probe run_id={} model={} outcome=verified",
                    run_id,
                    model_id
                );
                batch.results.push(result)
            }
            Ok(Ok(_)) => {
                log::warn!("codex_native_model_probe run_id={} model={} outcome=failed code=native-executable-changed", run_id, model_id);
                model.error_code = Some("native-executable-changed".into());
            }
            Ok(Err(error)) => {
                log::warn!(
                    "codex_native_model_probe run_id={} model={} outcome=failed code={}",
                    run_id,
                    model_id,
                    error.code()
                );
                model.error_code = Some(error.code().into());
            }
            Err(_) => {
                log::warn!("codex_native_model_probe run_id={} model={} outcome=failed code=native-worker-failed", run_id, model_id);
                model.error_code = Some("native-worker-failed".into());
            }
        }
    }
    batch
}
fn no_unmanaged_catalog(text: &str) -> bool {
    !text.lines().any(|line| {
        let line = line.trim_start();
        !line.starts_with('#') && line.contains("model_catalog_json")
    })
}
fn catalog_bytes(
    batch: &NativeBatch,
    report: &NativeModelCatalogReport,
    context: &CapabilityContext,
) -> Result<Vec<u8>, &'static str> {
    let proofs = batch.proofs();
    let mut slugs = BTreeSet::new();
    let mut entries = Vec::new();
    let version = batch
        .results
        .first()
        .map(|r| r.runtime_version.as_str())
        .ok_or("native-evidence-missing")?;
    let expected_hash = &batch
        .results
        .first()
        .ok_or("native-evidence-missing")?
        .executable_sha256;
    for result in &batch.results {
        if result.runtime_version != version || &result.executable_sha256 != expected_hash {
            return Err("native-executable-changed");
        }
        let Some(source) = report
            .models
            .iter()
            .find(|m| m.model_id == result.proof.model_id && m.owned_by == result.proof.owned_by)
        else {
            return Err("native-evidence-mismatch");
        };
        let mut current = source.clone();
        current.evaluate(context, Some(&result.proof), unix_seconds());
        if current.state != ModelState::Ready {
            continue;
        }
        if result.catalog_entry.get("slug").and_then(Value::as_str)
            != Some(current.model_id.as_str())
            || !slugs.insert(current.model_id.clone())
        {
            return Err("native-catalog-ambiguous");
        }
        entries.push(result.catalog_entry.clone());
    }
    if select_ready_default(&report.models, context, &proofs, unix_seconds()).is_none() {
        return Err("native-evidence-missing");
    }
    serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,
        "fetched_at":format!("unix:{}",unix_seconds()),"client_version":version,
        "identity":CATALOG_IDENTITY,"models":entries}))
    .map_err(|_| "native-catalog-invalid")
}

struct PreparedPublish {
    selected_model: String,
    snapshot: SelectionReply,
    executable: PathBuf,
    expected_executable_sha256: String,
    catalog_bytes: Vec<u8>,
}

fn prepare_publish<R, V>(
    batch: &NativeBatch,
    result: &NativeModelCatalogReport,
    context: &CapabilityContext,
    preferred_model_id: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
    verify_runtime: R,
    validate_catalog: V,
) -> Result<PreparedPublish, &'static str>
where
    R: FnOnce(&Path, &AtomicBool, Instant) -> Result<String, &'static str>,
    V: FnOnce(&Path, &[u8], &AtomicBool, Instant) -> Result<(), &'static str>,
{
    check(cancelled, deadline)?;
    let proofs = batch.proofs();
    let selected_models = result
        .models
        .iter()
        .filter(|model| model.model_id == preferred_model_id)
        .cloned()
        .collect::<Vec<_>>();
    if selected_models.len() != 1
        || select_ready_default(&selected_models, context, &proofs, unix_seconds()).as_deref()
            != Some(preferred_model_id)
    {
        return Err("native-evidence-missing");
    }
    let snapshot = batch
        .selection
        .as_ref()
        .ok_or(batch.error.unwrap_or("selection-snapshot-missing"))?
        .clone();
    let executable = batch
        .executable
        .as_ref()
        .ok_or("native-executable-unavailable")?
        .clone();
    let expected_executable_sha256 = batch
        .results
        .first()
        .ok_or("native-evidence-missing")?
        .executable_sha256
        .clone();
    if verify_runtime(&executable, cancelled, deadline)? != expected_executable_sha256 {
        return Err("native-executable-changed");
    }
    let bytes = catalog_bytes(batch, result, context)?;
    if bytes.len() > BODY_LIMIT {
        return Err("native-catalog-too-large");
    }
    validate_catalog(&executable, &bytes, cancelled, deadline)?;
    Ok(PreparedPublish {
        selected_model: preferred_model_id.to_owned(),
        snapshot,
        executable,
        expected_executable_sha256,
        catalog_bytes: bytes,
    })
}

fn post_apply_snapshot_matches(
    applied: &SelectionReply,
    snapshot: &SelectionReply,
    preferred_model_id: &str,
) -> bool {
    applied.ok
        && applied.applied
        && snapshot.ok
        && !snapshot.applied
        && applied.selected_model.as_deref() == Some(preferred_model_id)
        && snapshot.selected_model.as_deref() == Some(preferred_model_id)
        && applied.revision.is_some()
        && applied.revision == snapshot.revision
        && applied.config_sha256.is_some()
        && applied.config_sha256 == snapshot.config_sha256
}

fn request_selection_snapshot<F>(request_snapshot: F) -> Result<SelectionReply, &'static str>
where
    F: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
{
    let request_id = format!("snapshot-{}", uuid::Uuid::new_v4().simple());
    let snapshot = request_snapshot(SelectionRequest {
        schema_version: 3,
        request_id: &request_id,
        operation: "snapshot",
        expected_revision: None,
        model: None,
        catalog_path: None,
        catalog_sha256: None,
    })?;
    if snapshot.schema_version != 3
        || snapshot.request_id != request_id
        || !snapshot.ok
        || snapshot.code != "ok"
        || snapshot.applied
        || !snapshot.revision.as_deref().is_some_and(is_sha256_hex)
        || snapshot
            .selected_model
            .as_deref()
            .is_some_and(|model| !safe_identifier(model))
        || snapshot
            .config_sha256
            .as_deref()
            .is_some_and(|hash| !is_sha256_hex(hash))
    {
        return Err("selection-response-invalid");
    }
    Ok(snapshot)
}

fn snapshot_confirms_model(snapshot: &SelectionReply, preferred_model_id: &str) -> bool {
    snapshot.selected_model.as_deref() == Some(preferred_model_id)
        && snapshot.revision.as_deref().is_some_and(is_sha256_hex)
        && snapshot.config_sha256.as_deref().is_some_and(is_sha256_hex)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn request_post_apply_snapshot<F>(
    applied: &SelectionReply,
    preferred_model_id: &str,
    request_snapshot: F,
) -> Result<SelectionReply, &'static str>
where
    F: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
{
    if !applied.applied
        || applied.selected_model.as_deref() != Some(preferred_model_id)
        || applied.revision.is_none()
        || applied.config_sha256.is_none()
    {
        return Err("selection-response-invalid");
    }
    let snapshot = request_selection_snapshot(request_snapshot)?;
    if !post_apply_snapshot_matches(applied, &snapshot, preferred_model_id) {
        return Err("selection-readback-mismatch");
    }
    Ok(snapshot)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SelectionStateFile {
    Config,
    Auth,
    Catalog,
}

fn compensate_apply_failure<R, W, D>(
    candidate_catalog: &[u8],
    previous_catalog: Option<&[u8]>,
    previous_config: Option<&[u8]>,
    previous_auth: Option<&[u8]>,
    mut read: R,
    mut write: W,
    mut remove: D,
) -> Result<(), &'static str>
where
    R: FnMut(SelectionStateFile) -> Result<Option<Vec<u8>>, &'static str>,
    W: FnMut(SelectionStateFile, &[u8]) -> Result<(), &'static str>,
    D: FnMut(SelectionStateFile) -> Result<(), &'static str>,
{
    if read(SelectionStateFile::Config)
        .map_err(|_| "selection-outcome-unconfirmed")?
        .as_deref()
        != previous_config
        || read(SelectionStateFile::Auth)
            .map_err(|_| "selection-outcome-unconfirmed")?
            .as_deref()
            != previous_auth
    {
        return Err("selection-outcome-unconfirmed");
    }
    if read(SelectionStateFile::Catalog)
        .map_err(|_| "selection-rollback-conflict")?
        .as_deref()
        != Some(candidate_catalog)
    {
        return Err("selection-rollback-conflict");
    }
    match previous_catalog {
        Some(previous) => write(SelectionStateFile::Catalog, previous)
            .map_err(|_| "selection-rollback-incomplete"),
        None => remove(SelectionStateFile::Catalog).map_err(|_| "selection-rollback-incomplete"),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SelectionApplyFailure {
    ApplyRejected(&'static str),
    CompensationFailed(&'static str),
    ReadbackFailed(&'static str),
}

fn finalize_selection_apply<A, C>(
    apply_request_id: &str,
    apply_result: Result<SelectionReply, &'static str>,
    preferred_model_id: &str,
    request_snapshot: A,
    compensate: C,
) -> Result<(SelectionReply, SelectionReply), SelectionApplyFailure>
where
    A: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
    C: FnOnce() -> Result<(), &'static str>,
{
    let applied = match apply_result {
        Ok(applied) => applied,
        Err(code) => {
            if let Ok(snapshot) = request_selection_snapshot(request_snapshot) {
                if snapshot_confirms_model(&snapshot, preferred_model_id) {
                    // The helper acknowledgement may be lost after it wrote
                    // the provider. Bind this observation to the apply request;
                    // publish still verifies local config/auth/catalog bytes.
                    let confirmed = SelectionReply {
                        schema_version: snapshot.schema_version,
                        request_id: apply_request_id.to_owned(),
                        ok: true,
                        code: "ok".into(),
                        revision: snapshot.revision.clone(),
                        selected_model: snapshot.selected_model.clone(),
                        config_sha256: snapshot.config_sha256.clone(),
                        applied: true,
                    };
                    return Ok((confirmed, snapshot));
                }
            }
            compensate().map_err(SelectionApplyFailure::CompensationFailed)?;
            return Err(SelectionApplyFailure::ApplyRejected(code));
        }
    };
    let snapshot = request_post_apply_snapshot(&applied, preferred_model_id, request_snapshot)
        .map_err(SelectionApplyFailure::ReadbackFailed)?;
    Ok((applied, snapshot))
}

fn finalize_selection_apply_with_compensation<A, R, W, D>(
    apply_request_id: &str,
    apply_result: Result<SelectionReply, &'static str>,
    preferred_model_id: &str,
    candidate_catalog: &[u8],
    previous_catalog: Option<&[u8]>,
    previous_config: Option<&[u8]>,
    previous_auth: Option<&[u8]>,
    request_snapshot: A,
    read: R,
    write: W,
    remove: D,
) -> Result<(SelectionReply, SelectionReply), SelectionApplyFailure>
where
    A: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
    R: FnMut(SelectionStateFile) -> Result<Option<Vec<u8>>, &'static str>,
    W: FnMut(SelectionStateFile, &[u8]) -> Result<(), &'static str>,
    D: FnMut(SelectionStateFile) -> Result<(), &'static str>,
{
    finalize_selection_apply(
        apply_request_id,
        apply_result,
        preferred_model_id,
        request_snapshot,
        || {
            compensate_apply_failure(
                candidate_catalog,
                previous_catalog,
                previous_config,
                previous_auth,
                read,
                write,
                remove,
            )
        },
    )
}

struct ConfirmedPublish {
    selected_model: String,
    selected_owner: Option<String>,
    proofs: Vec<NativeRuntimeEvidence>,
    models: Vec<ModelCapabilityReport>,
    executable: PathBuf,
    executable_sha256: String,
    catalog_path: PathBuf,
    catalog_sha256: String,
    config_sha256: String,
    auth_sha256: String,
}

fn commit_prepared_publish<A, S, B, R>(
    batch: &NativeBatch,
    prepared: &PreparedPublish,
    result: &mut NativeModelCatalogReport,
    binding_api_key: &str,
    binding_provider_id: &str,
    expected_base_url: &str,
    home: &Path,
    cancelled: &AtomicBool,
    deadline: Instant,
    apply_request: A,
    snapshot_request: S,
    binding_is_current: B,
    verify_runtime: R,
) -> Result<ConfirmedPublish, &'static str>
where
    A: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
    S: for<'request> FnOnce(SelectionRequest<'request>) -> Result<SelectionReply, &'static str>,
    B: FnOnce() -> bool,
    R: Fn(&Path, &AtomicBool, Instant) -> Result<String, &'static str>,
{
    let catalog = home.join(CATALOG_FILE_NAME);
    let config = home.join("config.toml");
    let auth = home.join("auth.json");
    let catalog_name = catalog.to_str().ok_or("native-catalog-path-invalid")?;
    let bytes = &prepared.catalog_bytes;
    let old_catalog;
    let old_config;
    let old_auth;
    {
        let _configuration = model_selection_client::lock_configuration()?;
        for path in [&catalog, &atomic_file::backup_path(&catalog)] {
            regular_or_absent(path).map_err(|_| "native-catalog-conflict")?;
            if path.exists() && !owned_json(path, CATALOG_IDENTITY) {
                return Err("native-catalog-conflict");
            }
        }
        old_catalog = read_optional(&catalog).map_err(|_| "native-catalog-conflict")?;
        old_config = read_optional(&config).map_err(|_| "selection-config-conflict")?;
        old_auth = read_optional(&auth).map_err(|_| "selection-auth-conflict")?;
        let text = std::str::from_utf8(old_config.as_deref().unwrap_or_default())
            .map_err(|_| "selection-config-conflict")?;
        let remaining = remove_managed_config(text).map_err(|_| "selection-catalog-conflict")?;
        if !no_unmanaged_catalog(&remaining) {
            return Err("selection-catalog-conflict");
        }
        if read_optional(&catalog).map_err(|_| "native-catalog-conflict")? != old_catalog {
            return Err("native-catalog-revision-conflict");
        }
        // The catalog remains non-authoritative until the verified provider
        // selection and its local postconditions have both been confirmed.
        atomic_file::write_atomic(&catalog, bytes).map_err(|_| "native-catalog-write-failed")?;
    }

    let request_id = format!("apply-{}", uuid::Uuid::new_v4().simple());
    let catalog_hash = digest_hex(bytes);
    result.activation_status = "outcome_unconfirmed".into();
    let apply_result = apply_request(SelectionRequest {
        schema_version: 3,
        request_id: &request_id,
        operation: "apply",
        expected_revision: prepared.snapshot.revision.as_deref(),
        model: Some(&prepared.selected_model),
        catalog_path: Some(catalog_name),
        catalog_sha256: Some(&catalog_hash),
    });

    let _configuration = model_selection_client::lock_configuration()?;
    let selection_outcome = finalize_selection_apply_with_compensation(
        &request_id,
        apply_result,
        &prepared.selected_model,
        bytes,
        old_catalog.as_deref(),
        old_config.as_deref(),
        old_auth.as_deref(),
        snapshot_request,
        |file| {
            let path = match file {
                SelectionStateFile::Config => &config,
                SelectionStateFile::Auth => &auth,
                SelectionStateFile::Catalog => &catalog,
            };
            read_optional(path).map_err(|_| "selection-state-read-failed")
        },
        |file, previous| {
            let path = match file {
                SelectionStateFile::Catalog => &catalog,
                SelectionStateFile::Config | SelectionStateFile::Auth => {
                    return Err("selection-rollback-incomplete")
                }
            };
            atomic_file::write_atomic(path, previous).map_err(|_| "selection-rollback-incomplete")
        },
        |file| {
            let path = match file {
                SelectionStateFile::Catalog => &catalog,
                SelectionStateFile::Config | SelectionStateFile::Auth => {
                    return Err("selection-rollback-incomplete")
                }
            };
            fs::remove_file(path).map_err(|_| "selection-rollback-incomplete")
        },
    );
    let (applied, readback) = match selection_outcome {
        Ok(confirmed) => confirmed,
        Err(SelectionApplyFailure::ApplyRejected(code)) => {
            result.activation_status = "incomplete".into();
            return Err(code);
        }
        Err(SelectionApplyFailure::CompensationFailed(code))
        | Err(SelectionApplyFailure::ReadbackFailed(code)) => {
            result.activation_status = "outcome_unconfirmed".into();
            return Err(code);
        }
    };
    debug_assert!(!readback.applied);
    if !binding_is_current() {
        return Err("selection-binding-changed");
    }

    let observed = read_optional(&config)
        .map_err(|_| "selection-config-conflict")?
        .ok_or("selection-config-conflict")?;
    let observed_config_sha256 = digest_hex(&observed);
    if applied.config_sha256.as_deref() != Some(observed_config_sha256.as_str())
        || read_optional(&catalog)
            .map_err(|_| "native-catalog-conflict")?
            .as_deref()
            != Some(bytes.as_slice())
    {
        return Err("selection-revision-conflict");
    }
    codex_config_readback::validate(
        &observed,
        binding_provider_id,
        &prepared.selected_model,
        expected_base_url,
    )?;
    let authentication = read_optional(&auth)
        .map_err(|_| "selection-auth-conflict")?
        .ok_or("selection-auth-conflict")?;
    let auth_sha256 = live_auth_fingerprint(&authentication, binding_api_key)?;
    if verify_runtime(&prepared.executable, cancelled, deadline)?
        != prepared.expected_executable_sha256
    {
        return Err("native-executable-changed");
    }

    let selected_owner = result
        .models
        .iter()
        .find(|model| model.model_id == prepared.selected_model && model.state == ModelState::Ready)
        .and_then(|model| model.owned_by.clone());
    Ok(ConfirmedPublish {
        selected_model: prepared.selected_model.clone(),
        selected_owner,
        proofs: batch.proofs(),
        models: result.models.clone(),
        executable: prepared.executable.clone(),
        executable_sha256: prepared.expected_executable_sha256.clone(),
        catalog_path: catalog,
        catalog_sha256: catalog_hash,
        config_sha256: observed_config_sha256,
        auth_sha256,
    })
}

/// Called inside the manager's final commit lock, never during discovery.
/// Configuration conflicts are failures, not permission to overwrite the user.
pub fn publish(
    batch: NativeBatch,
    result: &mut NativeModelCatalogReport,
    context: &CapabilityContext,
    preferred_model_id: &str,
    binding: &codex_usb_license::UsbAccountBinding,
    provider_base_url: &str,
    home: &Path,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<(), &'static str> {
    let prepared = prepare_publish(
        &batch,
        result,
        context,
        preferred_model_id,
        cancelled,
        deadline,
        hash_runtime,
        |_executable, bytes, cancelled, deadline| {
            model_native_probe::validate_final_catalog(bytes, cancelled, deadline)
                .map_err(|error| error.code())
        },
    )?;
    let confirmed = commit_prepared_publish(
        &batch,
        &prepared,
        result,
        binding.ccswitch.device_api_key.trim(),
        binding.ccswitch.provider_id.trim(),
        provider_base_url,
        home,
        cancelled,
        deadline,
        |request| model_selection_client::request(request, binding, cancelled, home),
        |request| model_selection_client::request(request, binding, cancelled, home),
        || {
            usb_guard::authorized_account_binding()
                .ok()
                .flatten()
                .as_ref()
                == Some(binding)
                && codex_home_dir().as_deref() == Some(home)
        },
        hash_runtime,
    )?;
    *LIVE_CATALOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(LiveCatalog {
        // The provider apply and snapshot are only the first half of commit.
        // model_catalog promotes this proof after the final Complete sidecar
        // checkpoint is durably written.
        setup_complete: false,
        binding: binding.clone(),
        context: context.clone(),
        selected_model: confirmed.selected_model.clone(),
        selected_owner: confirmed.selected_owner,
        proofs: confirmed.proofs,
        models: confirmed.models,
        executable: confirmed.executable,
        executable_sha256: confirmed.executable_sha256,
        catalog_sha256: confirmed.catalog_sha256,
        config_sha256: confirmed.config_sha256,
        auth_sha256: confirmed.auth_sha256,
    });
    result.default_model = Some(confirmed.selected_model);
    result.catalog_path = Some(confirmed.catalog_path.to_string_lossy().into_owned());
    mark_restart_required().map_err(|_| "selection-restart-marker-failed")?;
    Ok(())
}

pub fn confirm_setup_complete(home: &Path) -> Result<(), &'static str> {
    let mut guard = LIVE_CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    let live = guard.as_mut().ok_or("native-evidence-missing")?;
    if !live_catalog_state_matches(live, home) {
        return Err("selection-outcome-unconfirmed");
    }
    live.setup_complete = true;
    Ok(())
}

fn live_auth_fingerprint(bytes: &[u8], expected_key: &str) -> Result<String, &'static str> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "selection-auth-conflict")?;
    if expected_key.is_empty()
        || value.get("OPENAI_API_KEY").and_then(Value::as_str) != Some(expected_key)
    {
        return Err("selection-auth-conflict");
    }
    // Retain only an in-process byte fingerprint, never an authentication body.
    Ok(digest_hex(bytes))
}
pub fn current_catalog_is_verified(home: &Path) -> bool {
    let live = LIVE_CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(live) = live.as_ref() else {
        return false;
    };
    launch_proof_is_current(live.setup_complete, live_catalog_state_matches(live, home))
}

fn launch_proof_is_current(setup_complete: bool, state_matches: bool) -> bool {
    setup_complete && state_matches
}

fn live_catalog_state_matches(live: &LiveCatalog, home: &Path) -> bool {
    let selected = live
        .models
        .iter()
        .filter(|m| m.model_id == live.selected_model && m.owned_by == live.selected_owner)
        .cloned()
        .collect::<Vec<_>>();
    if selected.len() != 1
        || usb_guard::authorized_account_binding()
            .ok()
            .flatten()
            .as_ref()
            != Some(&live.binding)
        || select_ready_default(&selected, &live.context, &live.proofs, unix_seconds()).as_deref()
            != Some(live.selected_model.as_str())
    {
        return false;
    }
    let matches = |path: &Path, expected: &str| {
        read_optional(path)
            .ok()
            .flatten()
            .is_some_and(|b| digest_hex(&b) == expected)
    };
    matches(&home.join(CATALOG_FILE_NAME), &live.catalog_sha256)
        && matches(&home.join("config.toml"), &live.config_sha256)
        && matches(&home.join("auth.json"), &live.auth_sha256)
        && hash_runtime(
            &live.executable,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(30),
        )
        .is_ok_and(|hash| hash == live.executable_sha256)
}

#[cfg(test)]
#[path = "model_catalog_activation_tests.rs"]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_codex_pe(subsystem: u16) -> Vec<u8> {
        let mut bytes = vec![0x5a; 2048];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes());
        let subsystem_offset = 0x80 + 24 + 68;
        bytes[subsystem_offset..subsystem_offset + 2].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn portable_cli_hash_allows_only_console_to_gui_subsystem_patch() {
        let dir =
            std::env::temp_dir().join(format!("codex-pe-normalization-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("codex.exe");
        let signed_msix_member = synthetic_codex_pe(3);
        let mut portable_copy = synthetic_codex_pe(2);
        fs::write(&path, &portable_copy).unwrap();
        let hashes = hash_runtime_with_console_normalization(
            &path,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(hashes.raw_sha256, sha256_hex(&portable_copy));
        assert_eq!(
            hashes.normalized_console_sha256.as_deref(),
            Some(sha256_hex(&signed_msix_member).as_str())
        );

        portable_copy[0x500] ^= 0x01;
        fs::write(&path, &portable_copy).unwrap();
        let tampered = hash_runtime_with_console_normalization(
            &path,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_ne!(
            tampered.normalized_console_sha256.as_deref(),
            Some(sha256_hex(&signed_msix_member).as_str())
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn raw_and_normalized_hashes_come_from_one_streamed_observation() {
        struct SwitchAfterPrefix {
            first: Vec<u8>,
            second: Vec<u8>,
            switch_at: usize,
            read: usize,
        }
        impl Read for SwitchAfterPrefix {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.read >= self.first.len() {
                    return Ok(0);
                }
                let source = if self.read < self.switch_at {
                    &self.first
                } else {
                    &self.second
                };
                let count = buffer.len().min(source.len() - self.read);
                buffer[..count].copy_from_slice(&source[self.read..self.read + count]);
                self.read += count;
                Ok(count)
            }
        }

        let first = synthetic_codex_pe(2);
        let mut second = first.clone();
        second[0x500] ^= 0x5a;
        let switch_at = 0x80 + 24 + 70;
        let mut stream = SwitchAfterPrefix {
            first: first.clone(),
            second: second.clone(),
            switch_at,
            read: 0,
        };
        let hashes = hash_runtime_reader_with_console_normalization(
            &mut stream,
            first.len() as u64,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();

        let mut observed = first.clone();
        observed[switch_at..].copy_from_slice(&second[switch_at..]);
        let mut normalized_observed = observed.clone();
        let subsystem_offset = 0x80 + 24 + 68;
        normalized_observed[subsystem_offset..subsystem_offset + 2]
            .copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(hashes.raw_sha256, sha256_hex(&observed));
        assert_eq!(
            hashes.normalized_console_sha256.as_deref(),
            Some(sha256_hex(&normalized_observed).as_str())
        );
        assert_ne!(hashes.raw_sha256, sha256_hex(&first));
        assert_ne!(
            hashes.normalized_console_sha256.as_deref(),
            Some(sha256_hex(&second).as_str())
        );
    }

    #[test]
    fn launch_proof_requires_final_checkpoint_and_matching_live_state() {
        assert!(!launch_proof_is_current(false, true));
        assert!(!launch_proof_is_current(true, false));
        assert!(launch_proof_is_current(true, true));
    }

    #[test]
    fn live_authentication_is_bound_to_the_current_credential_and_exact_bytes() {
        let first = br#"{"OPENAI_API_KEY":"fixture-key","metadata":"one"}"#;
        let changed = br#"{"OPENAI_API_KEY":"fixture-key","metadata":"two"}"#;
        let fingerprint = live_auth_fingerprint(first, "fixture-key").unwrap();
        assert_eq!(fingerprint, digest_hex(first));
        assert_ne!(
            fingerprint,
            live_auth_fingerprint(changed, "fixture-key").unwrap()
        );
        assert!(live_auth_fingerprint(first, "another-fixture-key").is_err());
        assert!(live_auth_fingerprint(first, "").is_err());
        let invalid: &[&[u8]] = &[
            b"{",
            b"{}",
            b"[]",
            br#"{"OPENAI_API_KEY":false}"#,
            br#"{"OPENAI_API_KEY":1}"#,
            br#"{"OPENAI_API_KEY":""}"#,
        ];
        for bytes in invalid {
            assert!(live_auth_fingerprint(bytes, "fixture-key").is_err());
        }
    }
    #[test]
    fn native_opt_in_requires_explicit_cost_consent_and_rejects_extra_fields() {
        for text in [
            r#"{"costConsent":"true"}"#,
            r#"{"costConsent":1}"#,
            r#"{"costConsent":true,"selectedModelId":"unverified-model"}"#,
            r#"{}"#,
        ] {
            assert!(serde_json::from_str::<NativeVerificationOptions>(text).is_err());
        }
        assert!(NativeVerificationOptions {
            cost_consent: false,
            start_new_operation: false
        }
        .validate()
        .is_err());
        assert!(NativeVerificationOptions {
            cost_consent: true,
            start_new_operation: false
        }
        .validate()
        .is_ok());
    }
    #[test]
    fn mock_observations_cannot_enter_the_native_producer() {
        let mut model = ModelCapabilityReport::discovered("arbitrary-vendor/model".into(), None);
        model.evidence_scope = EvidenceScope::Mock;
        assert!(!complete_gateway(&model));
        model.evidence_scope = EvidenceScope::Gateway;
        assert!(!complete_gateway(&model));
    }
    #[test]
    fn unmanaged_catalog_reference_is_not_replaced() {
        assert!(!no_unmanaged_catalog(
            "model_catalog_json = \"user.json\"\n"
        ));
        assert!(no_unmanaged_catalog(
            "# user note\nmodel = \"user-model\"\n"
        ));
    }
}
