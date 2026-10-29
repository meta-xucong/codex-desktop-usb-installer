//! Windows update planning + staging service.
//!
//! Mirrors the macOS command shape while keeping the Windows-specific logic in
//! `codex-win-engine`:
//!   - `plan_windows_update`  — read-only capability + manifest/checksum plan.
//!   - `stage_windows_update` — download MSIX + SHA256 + Authenticode + identity
//!     verification into staging. Non-destructive; it does not install yet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use codex_win_engine::{
    cancel_active_download, canonical_codex_package_moniker, cleanup_portable_metadata,
    close_codex_gracefully_for_root, close_msix_codex_processes, codex_running_for_root,
    detect_installed_codex, detect_portable_install,
    download_to_with_progress_bounded_with_network, fetch_text_with_network, find_msix_sha256,
    install_msix_sideload_with_observer, install_msix_to_volume_with_observer,
    install_portable_from_msix_with_observer_options_and_progress, limits::MAX_PACKAGE_BYTES,
    parse_manifest, pause_active_download, plan_update, precheck_msix_dependencies,
    probe_capabilities, purge_codex_user_data, read_codex_app_version_from_msix,
    read_msix_identity, sha256_file, uninstall_portable, validate_codex_identity,
    verify_msix_health_with_options, verify_openai_authenticode, version_key, AuthenticodeReport,
    CapabilityState, EngineError, InstalledWindowsCodex, MsixHealthReport, MsixIdentity,
    MsixMachineStateReport, MsixRemoveReport, MsixSideloadReport, NetworkConfig, PortableBoundary,
    PortableExtractionProgress, PortableInstallOptions, PortableInstallReport,
    PortableUninstallReport, WinCapabilityReport, WinInstallRoute, WindowsRelease,
    WindowsUpdatePlan,
};

use crate::app::appx_volume;
use crate::app::install_tx::{ActiveInstallTx, InstallTxKind, SelfUpdatePolicyTransition};
use crate::app::msix_policy_tx::ActiveMsixPolicyTransition;
use crate::app::op_phase::OperationPhase;
use crate::app::operation_outcome::{
    recovery, AncillaryRetryReport, OperationOutcome, StepOutcome,
};
use crate::app::provenance::ProvenanceStore;
use crate::app::release_install::{
    resolved_local_asset, verify_file_digest, ReleaseArchitecture, ReleasePackageFormat,
    ResolvedReleaseAsset,
};
use crate::app::staging;
use crate::app::usb_guard;
use crate::domain::manifest::MirrorEndpoints;
use crate::domain::settings::AppSettings;
use crate::errors::AppError;

/// Optional hook for the command layer to publish operation phases (quit policy).
pub type PhaseHook<'a> = dyn Fn(OperationPhase) + Send + Sync + 'a;

/// Completion evidence is intentionally separate from `OperationPhase`.
/// `Committing` blocks app exit before the first rename/platform call, while
/// these events describe whether a rejected command can still prove that the
/// installed app was left unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEvidence {
    MutationStarted,
    MutationRolledBack,
    OutcomeAmbiguous,
}

pub type EvidenceHook<'a> = dyn Fn(OperationEvidence) + Send + Sync + 'a;
pub type BeforeWinCommitHook<'a> = dyn Fn() -> Result<(), AppError> + Send + Sync + 'a;

/// Install mode used by the legacy fixed USB path. It remains distinct from
/// the user-selectable `msix` mode so ordinary offline callers cannot inherit
/// the manager's portable preference. `USB_APPX_INSTALL_MODE` is retained only
/// to identify and safely migrate/clean historical USB AppX installations.
pub const FIXED_MSIX_INSTALL_MODE: &str = "msix-fixed";
/// Historical USB edition mode. No new v0.5.20 install selects this route.
pub const USB_APPX_INSTALL_MODE: &str = "msix-usb-volume";
/// Current USB edition mode. The signed MSIX is used only as a verified archive;
/// its program tree is placed below the authenticated bundle while mutable data
/// remains in the ordinary local user profile. This avoids AppXVolume/WpSystem.
pub const USB_PORTABLE_INSTALL_MODE: &str = "portable-usb";

fn is_usb_portable_install_mode(install_mode: &str) -> bool {
    install_mode.eq_ignore_ascii_case(USB_PORTABLE_INSTALL_MODE)
}

fn allows_portable_fallback(install_mode: &str) -> bool {
    install_mode != FIXED_MSIX_INSTALL_MODE && install_mode != USB_APPX_INSTALL_MODE
}

/// A local, already-verified MSIX is itself a complete ZIP payload. The legacy
/// fixed mode may use same-bytes extraction recovery; USB AppXVolume mode does
/// not, because a C: copy would defeat the removable-program boundary.
fn allows_historical_portable_recovery(install_mode: &str, has_local_package: bool) -> bool {
    if install_mode == USB_APPX_INSTALL_MODE {
        return false;
    }
    allows_portable_fallback(install_mode) || has_local_package
}

fn installed_msix_matches_volume(installed: &InstalledWindowsCodex, volume_root: &Path) -> bool {
    if installed.source != "msix" {
        return false;
    }
    let normalize = |value: &str| {
        value
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    installed
        .package_volume
        .as_deref()
        .is_some_and(|actual| normalize(actual) == normalize(&volume_root.to_string_lossy()))
}

#[derive(Debug, PartialEq, Eq)]
struct FailedMsixPolicyState {
    target_landed_or_unknown: bool,
    keep_usb_data_policy: bool,
}

/// A killed PowerShell process does not prove the AppX service stopped. Even
/// an empty registration query at that instant cannot authorize policy rollback.
/// Keep that case distinct from a completed, structured deployment failure.
fn classify_failed_msix_policy(
    target: &str,
    usb_root: Option<&Path>,
    interrupted: bool,
    registration: Result<Option<String>, String>,
    installed: Option<&InstalledWindowsCodex>,
) -> FailedMsixPolicyState {
    if interrupted || registration.is_err() {
        return FailedMsixPolicyState {
            target_landed_or_unknown: true,
            keep_usb_data_policy: usb_root.is_some(),
        };
    }
    let Some(current) = registration.ok().flatten() else {
        return FailedMsixPolicyState {
            target_landed_or_unknown: false,
            keep_usb_data_policy: false,
        };
    };
    let identity_matches = current.eq_ignore_ascii_case(target);
    let Some(root) = usb_root else {
        return FailedMsixPolicyState {
            target_landed_or_unknown: identity_matches,
            keep_usb_data_policy: false,
        };
    };
    // The second query must describe the very same registration, not a stale
    // provenance record or a portable installation observed during a race.
    let consistent = installed.filter(|candidate| {
        candidate.source == "msix"
            && candidate.package_family_name.as_deref() == Some("OpenAI.Codex_2p2nqsd0c76g0")
            && candidate
                .path
                .replace('/', "\\")
                .trim_end_matches('\\')
                .rsplit('\\')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(&current))
            && candidate.package_volume.is_some()
    });
    let Some(installed) = consistent else {
        return FailedMsixPolicyState {
            target_landed_or_unknown: identity_matches,
            keep_usb_data_policy: true,
        };
    };
    let on_usb = installed_msix_matches_volume(installed, root);
    FailedMsixPolicyState {
        target_landed_or_unknown: identity_matches && on_usb,
        // An older retained USB registration also still needs local data.
        keep_usb_data_policy: on_usb,
    }
}

fn probe_failed_msix_policy(
    target: &str,
    usb_root: Option<&Path>,
    install_root: &Path,
    interrupted: bool,
) -> FailedMsixPolicyState {
    let registration =
        codex_win_engine::registered_msix_package_full_name().map_err(|err| err.to_string());
    let installed = if matches!(&registration, Ok(Some(_))) && usb_root.is_some() {
        detect_installed_codex(install_root)
    } else {
        None
    };
    let result = classify_failed_msix_policy(
        target,
        usb_root,
        interrupted,
        registration,
        installed.as_ref(),
    );
    log::warn!("MSIX failure policy disposition interrupted={interrupted} state={result:?}");
    result
}

fn interrupted_msix_error(
    error: EngineError,
    target: &str,
    usb_root: Option<&Path>,
    install_root: &Path,
    data_policy_changed: bool,
) -> AppError {
    let state = probe_failed_msix_policy(target, usb_root, install_root, true);
    // Leave the self-update journal pending too: a late AppX completion must
    // not lose its recovery evidence merely because the caller timed out.
    if data_policy_changed && state.keep_usb_data_policy {
        AppError::Engine(format!(
            "{error}；部署进程中断，Windows AppX 最终状态尚不确定。已保留本地数据策略及恢复记录；请勿手工删除 WpSystem 或包数据，重新打开管理器后检查安装状态。"
        ))
    } else {
        engine_err(error)
    }
}

/// The AppX package itself is installed by the ordinary (unelevated) path
/// whenever possible.  Only the machine-level removable-volume registration
/// needs a UAC retry.  Keep this classifier tied to the structured operation
/// marker emitted by `install_msix_script_with_volume`; a generic `Access is
/// denied` from Add-AppxPackage must not unexpectedly elevate or retry.
fn appx_volume_registration_needs_elevation(report: &MsixSideloadReport) -> bool {
    if report.success {
        return false;
    }
    let detail = [
        report.message.as_str(),
        report.raw_error.as_deref().unwrap_or_default(),
        report.deployment_log.as_deref().unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let registration_failure = detail.contains("add-appxvolume")
        || detail.contains("register removable appx volume")
        || detail.contains("注册可移动 appx 卷");
    let access_denied = detail.contains("access is denied")
        || detail.contains("0x80070005")
        || detail.contains("拒绝访问")
        || detail.contains("权限不足");
    registration_failure && access_denied
}

/// The combined USB prerequisite operation includes the elevated
/// `Add-AppxVolume` call.  Only a successful combined operation may suppress
/// the later, targeted retry.  If the combined UAC child was declined,
/// timed out, or failed for any other reason, the unelevated package attempt
/// must still be allowed to request the one-shot volume-registration UAC
/// retry when Windows reports the characteristic access-denied marker.
fn should_retry_usb_volume_registration(
    report: &MsixSideloadReport,
    combined_prerequisites_succeeded: bool,
) -> bool {
    appx_volume_registration_needs_elevation(report) && !combined_prerequisites_succeeded
}

/// USB installation must never silently take ownership of an existing Codex.
/// The renderer presents the same policy, but the backend repeats it here so a
/// stale page or a time-of-check/time-of-use race cannot trigger an uninstall,
/// migration, or overwrite of an external installation.
fn usb_install_blocks_existing_install(status: &WinInstallStatus, usb_codex_root: &Path) -> bool {
    if status.status == "external" {
        return true;
    }
    let Some(installed) = status.installed.as_ref() else {
        return false;
    };
    // A USB install is only allowed to refresh the portable tree owned by the
    // currently authenticated bundle. A managed record on C:, a stale MSIX,
    // or any other path is still an existing installation and must remain
    // untouched; the caller should ask the user to remove it explicitly or use
    // the ordinary Manager takeover flow.
    installed.source != "portable"
        || !codex_win_engine::same_windows_path(Path::new(&installed.path), usb_codex_root)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinUpdateReport {
    pub manifest_url: String,
    pub checksums_url: String,
    pub package_url: String,
    pub release: WindowsRelease,
    pub installed: Option<InstalledWindowsCodex>,
    pub capabilities: WinCapabilityReport,
    pub plan: WindowsUpdatePlan,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinStageReport {
    pub up_to_date: bool,
    pub route: String,
    pub latest_version: String,
    pub package_moniker: String,
    pub download_size: u64,
    pub staged_path: Option<String>,
    pub sha256: String,
    pub hash_verified: bool,
    pub authenticode: Option<AuthenticodeReport>,
    pub identity: Option<MsixIdentity>,
    pub identity_verified: bool,
    pub install_ready: bool,
    pub portable_fallback_ready: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinAutoStageReport {
    pub enabled: bool,
    pub allow_metered: bool,
    pub attempted: bool,
    pub skipped: bool,
    /// "disabled" | "up-to-date" | "metered-network" | "metered-unknown" | "staged"
    pub reason: String,
    pub stage: Option<WinStageReport>,
    pub capabilities: Option<WinCapabilityReport>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinPerformReport {
    pub success: bool,
    pub action: WinPerformAction,
    pub message: String,
    pub stage: WinStageReport,
    pub sideload: Option<MsixSideloadReport>,
    pub portable: Option<PortableInstallReport>,
    pub msix_health: Option<MsixHealthReport>,
    pub installed: Option<InstalledWindowsCodex>,
    pub fallback_available: bool,
    pub fallback_attempted: bool,
    pub notes: Vec<String>,
    /// Structured primary/ancillary result (provenance save, residual cleanup).
    #[serde(default)]
    pub outcome: OperationOutcome,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WinPerformAction {
    None,
    MsixSideload,
    PortableFallback,
    PortableFallbackAfterMsixFailure,
    PortableFallbackAfterMsixUnhealthy,
    PortableFallbackMissingFramework,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinPerformExpectation {
    pub current_version: Option<String>,
    pub latest_version: String,
    pub package_moniker: String,
    pub route: String,
}

impl WinPerformAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MsixSideload => "msix-sideload",
            Self::PortableFallback => "portable-fallback",
            Self::PortableFallbackAfterMsixFailure => "portable-fallback-after-msix-failure",
            Self::PortableFallbackAfterMsixUnhealthy => "portable-fallback-after-msix-unhealthy",
            Self::PortableFallbackMissingFramework => "portable-fallback-missing-framework",
        }
    }
}

impl std::fmt::Display for WinPerformAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinUninstallReport {
    pub success: bool,
    pub action: String,
    pub message: String,
    pub installed_before: Option<InstalledWindowsCodex>,
    pub msix: Option<MsixRemoveReport>,
    pub portable: Option<PortableUninstallReport>,
    pub purged_user_data: bool,
    pub notes: Vec<String>,
    /// Structured primary/ancillary result for partial-success recovery.
    #[serde(default)]
    pub outcome: OperationOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinInstallStatus {
    pub installed: Option<InstalledWindowsCodex>,
    /// "managed" | "external" | "none"
    pub status: String,
    /// A registered package can still have unfinished data restoration.
    pub recovery_pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    /// Host the bytes are coming from, e.g. `codexapp.agentsmirror.com`.
    pub source: String,
}

fn engine_err(err: impl ToString) -> AppError {
    AppError::Engine(err.to_string())
}

/// Record a managed install without failing the primary install/update when the
/// provenance store cannot be written. Surfaces the failure via [`OperationOutcome`].
fn record_managed_install(
    previous: Option<&InstalledWindowsCodex>,
    installed: &InstalledWindowsCodex,
    source: &str,
) -> OperationOutcome {
    let mut outcome = OperationOutcome::full_success("present", Some("managed"))
        .with_path(installed.path.clone());
    outcome.cleanup = StepOutcome::not_applicable();
    let mut store = ProvenanceStore::load();
    if let Some(previous) = previous {
        store.remove(&previous.path);
    }
    store.record(
        installed.path.clone(),
        version_key(&installed.version),
        source,
    );
    match store.save() {
        Ok(()) => outcome,
        Err(e) => {
            let detail = format!("托管记录保存失败（{e}）");
            outcome.provenance = StepOutcome::failed(detail.clone());
            outcome.install_class = Some("external".to_string());
            outcome.push_warning(format!(
                "{detail}；应用已安装，请用「开始管理」重试写入托管记录，勿重复安装"
            ));
            outcome.push_recovery(recovery::RECORD_PROVENANCE);
            outcome
        }
    }
}

/// Build a structured outcome after portable uninstall.
///
/// Primary success means the install tree is gone (removed this run **or**
/// already absent). Cleanup/metadata/provenance failures are partial, not hard
/// failures — the UI can offer ancillary-only recovery CTAs.
fn outcome_from_portable_uninstall(
    portable: &PortableUninstallReport,
    provenance: StepOutcome,
    path: &str,
) -> OperationOutcome {
    let provenance_failed = provenance.is_failed();
    // `removed_files == false` with `success` means the tree was already gone —
    // that is still a successful primary uninstall (nothing left to delete).
    let primary_ok = portable.success;
    let mut outcome = OperationOutcome {
        primary_ok,
        app_state: if primary_ok {
            "absent".to_string()
        } else {
            "present".to_string()
        },
        install_class: if primary_ok {
            Some("none".to_string())
        } else {
            None
        },
        path: Some(path.to_string()),
        provenance,
        cleanup: if portable.partial {
            StepOutcome::failed(
                portable
                    .notes
                    .iter()
                    .find(|n| n.contains("cleanup failed"))
                    .cloned()
                    .unwrap_or_else(|| "metadata cleanup failed".to_string()),
            )
        } else {
            StepOutcome::ok()
        },
        warnings: portable.notes.clone(),
        recovery_actions: Vec::new(),
    };
    if !portable.removed_files && primary_ok {
        outcome.push_warning(
            "Install tree was already absent; performed ancillary cleanup only.".to_string(),
        );
    }
    if outcome.cleanup.is_failed() {
        // Shortcut / uninstall-entry failures.
        if portable.notes.iter().any(|n| {
            n.contains("Start Menu")
                || n.contains("Apps & Features")
                || n.contains("uninstall entry")
        }) {
            outcome.push_recovery(recovery::CLEANUP_METADATA);
        }
        // User-data purge failures (non-fatal).
        if portable.notes.iter().any(|n| {
            let lower = n.to_ascii_lowercase();
            lower.contains("user data") && lower.contains("failed")
        }) {
            outcome.push_recovery(recovery::PURGE_USER_DATA);
        }
        // Fallback: any partial cleanup gets a metadata retry.
        if outcome.recovery_actions.is_empty() {
            outcome.push_recovery(recovery::CLEANUP_METADATA);
        }
    }
    if provenance_failed {
        outcome.push_recovery(recovery::CLEAR_PROVENANCE);
    }
    outcome
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
        .to_string()
}

fn no_progress(_p: DownloadProgress) {}

fn portable_fallback_ready(_endpoints: &MirrorEndpoints) -> bool {
    true
}

fn msix_stem(file_name: &str) -> Option<&str> {
    let base = file_name.rsplit('/').find(|segment| !segment.is_empty())?;
    let suffix_start = base.len().checked_sub(5)?;
    let suffix = base.get(suffix_start..)?;
    if suffix.eq_ignore_ascii_case(".msix") {
        base.get(..suffix_start)
    } else {
        None
    }
}

fn package_file_name_from_url(package_url: &str) -> Result<String, AppError> {
    let parsed = url::Url::parse(package_url)
        .map_err(|e| AppError::Engine(format!("invalid Windows package URL: {e}")))?;
    parsed
        .path()
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .ok_or_else(|| AppError::Engine("Windows package URL has no file name".to_string()))
}

fn bind_manifest_checksums(
    release: &WindowsRelease,
    checksums_text: &str,
    package_url: &str,
) -> Result<String, AppError> {
    let package_file = package_file_name_from_url(package_url)?;
    if let Some(url_moniker) = msix_stem(&package_file) {
        if !url_moniker.eq_ignore_ascii_case(&release.package_moniker) {
            let err = AppError::Engine(format!(
                "Windows manifest package moniker {} does not match URL artifact {}",
                release.package_moniker, package_file
            ));
            log::error!("Windows manifest checksums binding mismatch error={err}");
            return Err(err);
        }
    } else {
        log::debug!(
            "Windows package URL has no direct MSIX artifact name; relying on manifest/checksums binding package_file={package_file}"
        );
    }
    if let Some(identity) = release.package_identity.as_deref() {
        if identity != codex_win_engine::OPENAI_PACKAGE_IDENTITY {
            let err = AppError::Engine(format!(
                "Windows manifest package identity {identity} does not match {}",
                codex_win_engine::OPENAI_PACKAGE_IDENTITY
            ));
            log::error!("Windows manifest checksums binding mismatch error={err}");
            return Err(err);
        }
    }
    find_msix_sha256(checksums_text, &release.package_moniker).map_err(|err| {
        log::error!("Windows manifest checksums binding mismatch error={err}");
        engine_err(err)
    })
}

fn read_windows_release(
    endpoints: &MirrorEndpoints,
    network: &NetworkConfig,
) -> Result<(WindowsRelease, String, String), AppError> {
    let manifest_text =
        fetch_text_with_network(&endpoints.manifest_url, network).map_err(engine_err)?;
    let checksums_text =
        fetch_text_with_network(&endpoints.checksums_url, network).map_err(engine_err)?;
    let release = parse_manifest(&manifest_text).map_err(engine_err)?;
    let package_url = endpoints
        .windows_msix_url_for_arch(release.download_architecture.as_deref())
        .to_string();
    let sha256 = bind_manifest_checksums(&release, &checksums_text, &package_url)?;
    Ok((release, sha256, package_url))
}

fn route_label(plan: &WindowsUpdatePlan) -> String {
    match plan.route {
        codex_win_engine::WinInstallRoute::MsixSideload => "msix-sideload",
        codex_win_engine::WinInstallRoute::PortableFallback => "portable-fallback",
    }
    .to_string()
}

fn validate_perform_expectation(
    expected: &WinPerformExpectation,
    previous_installed: Option<&InstalledWindowsCodex>,
    stage: &WinStageReport,
) -> Result<(), AppError> {
    let actual_current = previous_installed.map(|installed| installed.version.as_str());
    if actual_current != expected.current_version.as_deref() {
        return Err(AppError::StaleExpectation(format!(
            "Windows Codex changed before install (expected current {:?}, found {:?}); please re-check and confirm again.",
            expected.current_version, actual_current
        )));
    }
    if stage.latest_version != expected.latest_version {
        return Err(AppError::StaleExpectation(format!(
            "Windows update target changed from {} to {}; please re-check and confirm again.",
            expected.latest_version, stage.latest_version
        )));
    }
    if stage.package_moniker != expected.package_moniker {
        return Err(AppError::StaleExpectation(format!(
            "Windows package changed from {} to {}; please re-check and confirm again.",
            expected.package_moniker, stage.package_moniker
        )));
    }
    if stage.route != expected.route {
        return Err(AppError::StaleExpectation(format!(
            "Windows install route changed from {} to {}; please re-check and confirm again.",
            expected.route, stage.route
        )));
    }
    Ok(())
}

fn close_existing_codex_before_portable_fallback(
    settings: &AppSettings,
    previous_installed: Option<&InstalledWindowsCodex>,
) -> Result<(), AppError> {
    log::info!("Windows portable fallback close existing source=portable-fallback");
    if let Some(installed) = detect_installed_codex(PathBuf::from(&settings.install_root).as_path())
    {
        if installed.source == "msix" {
            close_codex_gracefully_for_root(30, PathBuf::from(&installed.path).as_path())
                .map_err(engine_err)?;
        }
    }
    if let Some(previous) = previous_installed {
        if previous.source == "portable" {
            close_codex_gracefully_for_root(30, PathBuf::from(&previous.path).as_path())
                .map_err(engine_err)?;
        }
    }
    Ok(())
}

fn existing_codex_was_running(
    settings: &AppSettings,
    previous_installed: Option<&InstalledWindowsCodex>,
) -> Result<bool, AppError> {
    let mut roots = Vec::new();
    if let Some(installed) = detect_installed_codex(PathBuf::from(&settings.install_root).as_path())
    {
        roots.push(PathBuf::from(installed.path));
    }
    if let Some(previous) = previous_installed {
        let root = PathBuf::from(&previous.path);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }

    for root in roots {
        if codex_running_for_root(&root).map_err(engine_err)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Detect the installed Codex, preferring a manager-managed PORTABLE build over
/// a still-present (possibly stale) MSIX package.
///
/// `detect_installed_codex` is MSIX-first, which is correct for a clean machine.
/// But after a portable fallback an older MSIX can linger — e.g. sideload was
/// blocked by policy and the package couldn't be removed — and it would shadow
/// the portable build we just installed and recorded, leaving status, planning
/// and uninstall all resolving to the stale package (shown as external, planned
/// against the old version, and impossible to uninstall). When a managed portable
/// build is present it wins; otherwise fall back to normal MSIX-first detection.
fn detect_managed_codex(
    settings: &AppSettings,
    store: &ProvenanceStore,
) -> Option<InstalledWindowsCodex> {
    let root = PathBuf::from(&settings.install_root);
    if let Some(portable) = detect_portable_install(root.as_path()) {
        // In the fixed USB edition the authenticated bundle path itself is a
        // stronger ownership boundary than a C:-stored absolute path. This
        // keeps the install managed when Windows assigns the same USB a new
        // drive letter on a later machine/session.
        if store.is_managed(&portable.path) || is_current_usb_portable(&portable) {
            return Some(portable);
        }
    }
    for record in store.managed.iter().rev() {
        if let Some(portable) = detect_portable_install(PathBuf::from(&record.path).as_path()) {
            if store.is_managed(&portable.path) {
                return Some(portable);
            }
        }
    }
    detect_installed_codex(root.as_path())
}

fn is_current_usb_portable(installed: &InstalledWindowsCodex) -> bool {
    if installed.source != "portable" || !usb_guard::guard_required() {
        return false;
    }
    usb_guard::current_usb_codex_root().is_some_and(|expected| {
        codex_win_engine::same_windows_path(Path::new(&installed.path), &expected)
    })
}

/// A local provenance record does not travel with a USB drive. Only the
/// confirmed bundle-uninstall route may substitute live signed USB ownership
/// for that missing record. The resolver must verify the bundle, lease and
/// non-reparse destination; a renderer path or cached status is not authority.
fn authorize_portable_bundle_uninstall<F>(
    installed: &InstalledWindowsCodex,
    bundle_uninstall: bool,
    authorize: F,
) -> Result<bool, AppError>
where
    F: FnOnce() -> Result<Option<PathBuf>, AppError>,
{
    if !bundle_uninstall || installed.source != "portable" {
        return Ok(false);
    }
    Ok(authorize()?
        .is_some_and(|root| codex_win_engine::same_windows_path(Path::new(&installed.path), &root)))
}

fn validate_usb_launch_target(
    installed: &InstalledWindowsCodex,
    authorized_codex_root: &Path,
) -> Result<(), AppError> {
    if installed.source == "portable"
        && codex_win_engine::same_windows_path(Path::new(&installed.path), authorized_codex_root)
    {
        return Ok(());
    }
    Err(AppError::Engine(
        "当前 U 盘中的 Codex 程序不存在或未通过路径校验，请先修复 U 盘程序再启动。".to_string(),
    ))
}

/// Resolve USB launch authority and bind the selected Codex tree to the
/// signed bundle. Detection may fall back to another manager-registered
/// portable install when the USB payload is missing; that fallback is useful
/// for ordinary status/planning but must never enter a USB launch transaction.
pub(crate) fn authorize_usb_codex_launch(
    installed: &InstalledWindowsCodex,
    phase: &str,
) -> Result<Option<usb_guard::SignedUsbAuthorization>, AppError> {
    if !usb_guard::guard_required() {
        return Ok(None);
    }
    let authorization = usb_guard::require_signed_codex_bundle(phase)?
        .ok_or_else(|| AppError::Engine("U 盘 Codex 启动缺少有效的签名授权".to_string()))?;
    if let Err(error) = validate_usb_launch_target(installed, &authorization.codex_root) {
        log::error!(
            "USB Codex launch target rejected phase={phase} source={} selected={} authorized_root={}",
            installed.source,
            installed.path,
            authorization.codex_root.display()
        );
        return Err(error);
    }
    Ok(Some(authorization))
}

fn restart_codex_for_catalog_reload<F, C>(
    root: &Path,
    reload_pending: bool,
    is_running: F,
    close: C,
) -> Result<(), AppError>
where
    F: FnMut(&Path) -> Result<bool, String>,
    C: FnOnce(&Path) -> Result<(), String>,
{
    if !reload_pending {
        return Ok(());
    }
    let mut is_running = is_running;
    match is_running(root) {
        Ok(false) => Ok(()),
        Ok(true) => {
            close(root).map_err(|error| {
                AppError::Engine(format!("关闭正在运行的 Codex 以加载新模型失败：{error}"))
            })?;
            match is_running(root) {
                Ok(false) => Ok(()),
                Ok(true) => Err(AppError::Engine(
                    "关闭 Codex 后仍检测到旧进程，未启动以避免继续使用旧模型目录".into(),
                )),
                Err(error) => Err(AppError::Engine(format!(
                    "关闭 Codex 后无法确认旧进程是否退出，未启动以避免继续使用旧模型目录：{error}"
                ))),
            }
        }
        Err(error) => Err(AppError::Engine(format!(
            "无法确认 Codex 是否仍在运行，未尝试加载新模型；请手动关闭 Codex 后重试：{error}"
        ))),
    }
}

fn prepare_model_catalog_launch(
    installed: &InstalledWindowsCodex,
    usb_mode: bool,
) -> Result<Option<Vec<u8>>, AppError> {
    if !usb_mode {
        return Ok(None);
    }
    crate::app::model_catalog::preflight_usb_catalog()?;
    let reload_marker = crate::app::model_catalog::restart_marker_snapshot_checked()?;
    let root = Path::new(&installed.path);
    if reload_marker.is_some() {
        log::info!(
            "model catalog reload pending before Codex launch; checking managed root={}",
            root.display()
        );
    }
    restart_codex_for_catalog_reload(
        root,
        reload_marker.is_some(),
        |root| {
            codex_win_engine::processes_running_for_executables_under_root_strict(
                root,
                &[
                    std::ffi::OsStr::new("Codex.exe"),
                    std::ffi::OsStr::new("CodexCore.dat"),
                    std::ffi::OsStr::new("ChatGPT.exe"),
                ],
            )
            .map_err(|error| error.to_string())
        },
        |root| close_codex_gracefully_for_root(30, root).map_err(|error| error.to_string()),
    )?;
    Ok(reload_marker)
}

/// Prepare the same catalog reload boundary for the persisted-theme launch
/// path, which otherwise bypasses `launch_codex` and may hot-apply onto an old
/// single-instance Codex process.
pub(crate) fn prepare_usb_model_catalog_for_theme_launch(
    settings: &AppSettings,
) -> Result<Option<Vec<u8>>, AppError> {
    if !usb_guard::guard_required() {
        return Ok(None);
    }
    let store = ProvenanceStore::load();
    let installed = detect_managed_codex(settings, &store)
        .ok_or_else(|| AppError::Engine("没有可打开的 Codex".to_string()))?;
    let authorization = authorize_usb_codex_launch(&installed, "win-launch-theme-target")?;
    prepare_model_catalog_launch(&installed, authorization.is_some())
}

pub(crate) fn complete_usb_model_catalog_launch(reload_marker: Option<&[u8]>) {
    if let Some(marker) = reload_marker {
        if let Err(error) = crate::app::model_catalog::clear_restart_marker_if_matches(marker) {
            log::warn!("could not clear applied model catalog reload marker: {error}");
        }
    }
}

/// Make the USB-managed runtime wrappers the first commands resolved by the
/// Codex process and its shell children. This is process-local: it does not
/// change the user's persistent PATH or affect ordinary Codex launches.
fn usb_runtime_environment(codex_home: &Path) -> Vec<(String, String)> {
    let mut environment = vec![
        ("PYTHONUTF8".to_string(), "1".to_string()),
        ("PYTHONIOENCODING".to_string(), "utf-8".to_string()),
    ];

    let runtime_bin = codex_home.join("codex-agent-kit").join("bin");
    if runtime_bin.is_dir() {
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(runtime_bin)
            .chain(std::env::split_paths(&current_path))
            .collect::<Vec<_>>();
        if let Ok(path) = std::env::join_paths(paths) {
            environment.push(("PATH".to_string(), path.to_string_lossy().into_owned()));
        }
    }

    environment
}

/// Build the process-local environment shared by every USB Codex entry point.
/// Non-USB launches intentionally receive no USB runtime or media overrides.
pub(crate) fn codex_launch_environment(
    usb_mode: bool,
    codex_home: Option<&Path>,
) -> Vec<(String, String)> {
    if !usb_mode {
        return Vec::new();
    }

    let mut environment = codex_home.map(usb_runtime_environment).unwrap_or_default();
    environment.extend(crate::app::media_generation::usb_launch_environment());
    environment
}

pub fn plan_windows_update(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
) -> Result<WinUpdateReport, AppError> {
    plan_windows_update_with_install_mode(endpoints, settings, "msix")
}

pub fn plan_windows_update_with_install_mode(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    install_mode: &str,
) -> Result<WinUpdateReport, AppError> {
    plan_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        &NetworkConfig::system(),
    )
}

pub fn plan_windows_update_with_install_mode_and_network(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    install_mode: &str,
    network: &NetworkConfig,
) -> Result<WinUpdateReport, AppError> {
    log::info!("Windows plan start install_mode={install_mode}");
    let (release, sha256, package_url) = read_windows_release(endpoints, network)?;
    let installed = detect_managed_codex(settings, &ProvenanceStore::load());
    let capabilities = probe_capabilities();
    let mut plan = plan_update(
        &release,
        &sha256,
        &package_url,
        &installed,
        &capabilities,
        portable_fallback_ready(endpoints),
    );
    if install_mode == "portable" || is_usb_portable_install_mode(install_mode) {
        log::info!("Windows user selected portable; skipping MSIX sideload");
        plan.route = WinInstallRoute::PortableFallback;
        plan.warnings
            .push(if is_usb_portable_install_mode(install_mode) {
                "The USB bundle selected its signed portable route; MSIX sideload will be skipped."
                    .to_string()
            } else {
                "User selected the portable Windows install mode; MSIX sideload will be skipped."
                    .to_string()
            });
    }

    let report = WinUpdateReport {
        manifest_url: endpoints.manifest_url.clone(),
        checksums_url: endpoints.checksums_url.clone(),
        package_url,
        release,
        installed,
        capabilities,
        plan,
    };
    let route = route_label(&report.plan);
    let recommendation = match report.capabilities.recommendation {
        codex_win_engine::SideloadRecommendation::MsixPreferred => "msix-preferred",
        codex_win_engine::SideloadRecommendation::PortableFallback => "portable-fallback",
    };
    log::info!("Windows plan complete route={route} capabilities={recommendation}");
    Ok(report)
}

pub fn stage_windows_update(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
) -> Result<WinStageReport, AppError> {
    stage_windows_update_with_install_mode(endpoints, settings, "msix", &no_progress)
}

pub fn stage_windows_update_with_install_mode(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    install_mode: &str,
    progress: &dyn Fn(DownloadProgress),
) -> Result<WinStageReport, AppError> {
    stage_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        progress,
        &NetworkConfig::system(),
    )
}

pub fn stage_windows_update_with_install_mode_and_network(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    install_mode: &str,
    progress: &dyn Fn(DownloadProgress),
    network: &NetworkConfig,
) -> Result<WinStageReport, AppError> {
    log::info!("Windows stage start install_mode={install_mode}");
    // Own a guard so EVERY stage caller — `perform` (nested, harmless), background
    // `auto_stage`, and the standalone `win_stage_update` command — resets the
    // latch on exit. Without it, a cancelled background/standalone stage would
    // leave the latch set and make the next user perform abort at its first check.
    // (Nesting under perform's guard can't lose a reachable cancel: once the
    // download completes the UI is in the "finishing" state with cancel disabled,
    // so no cancel lands during stage's post-download verify.)
    let _abort_guard = WinAbortGuard::new();
    let report = plan_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        network,
    )?;
    // The plan above did the manifest/checksums fetch (the Windows "正在准备"
    // phase). Honor a cancel here — before the up-to-date early-return and the
    // download below; once curl runs, its own cancel flag takes over.
    check_win_update_abort()?;
    let route = route_label(&report.plan);
    if report.plan.up_to_date {
        log::info!(
            "Windows stage complete route={route} verified=false portable_fallback_ready={}",
            report.plan.portable_fallback_ready
        );
        return Ok(WinStageReport {
            up_to_date: true,
            route,
            latest_version: report.plan.latest_version,
            package_moniker: report.plan.package_moniker,
            download_size: 0,
            staged_path: None,
            sha256: report.plan.sha256,
            hash_verified: false,
            authenticode: None,
            identity: None,
            identity_verified: false,
            install_ready: false,
            portable_fallback_ready: report.plan.portable_fallback_ready,
            notes: vec!["Installed Windows Codex is already current.".to_string()],
        });
    }

    let stage_result = (|| -> Result<WinStageReport, AppError> {
        // Downloads into the PERSISTENT cache so a paused `.part` survives for the
        // next resume instead of dying with a per-run staging dir. (A preparing-
        // phase cancel was already checked right after the plan above; the
        // transfer itself is interruptible via the download loop's cancel flag.)
        let dest = staging::download_cache_path(
            &report.package_url,
            &format!("{}.msix", report.release.package_moniker),
        )?;
        let expected_size = report.release.content_length.unwrap_or(0);
        if expected_size > MAX_PACKAGE_BYTES {
            return Err(AppError::Engine(format!(
                "MSIX content length {expected_size} exceeds {MAX_PACKAGE_BYTES} byte limit"
            )));
        }
        let expected_sha = report.plan.sha256.clone();
        let source = host_of(&report.package_url);

        let cached_ok = dest.exists()
            && sha256_file(&dest)
                .map(|actual| actual.eq_ignore_ascii_case(&expected_sha))
                .unwrap_or(false);
        if !cached_ok {
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
            }
            download_to_with_progress_bounded_with_network(
                &report.package_url,
                &dest,
                MAX_PACKAGE_BYTES,
                &|downloaded| {
                    progress(DownloadProgress {
                        downloaded,
                        total: expected_size,
                        source: source.clone(),
                    });
                },
                network,
            )
            .map_err(engine_err)?;
        }

        // A fully-cached MSIX is hash-verified above WITHOUT firing a progress
        // event, so the UI is still in "正在准备" (cancel enabled) during that
        // hash — unlike the download path, which fires progress and flips the UI
        // to the cancel-disabled "finishing" state. Honor a cancel that landed
        // during the cache hash here, before we commit to the artifact: otherwise
        // the stage guard (nested under perform) would clear the latch on success
        // and perform's later checkpoint would never see it.
        check_win_update_abort()?;

        let actual_size = std::fs::metadata(&dest)
            .map_err(|e| AppError::Engine(format!("read staged MSIX metadata: {e}")))?
            .len();
        if expected_size > 0 && actual_size != expected_size {
            return Err(AppError::Engine(format!(
                "MSIX size mismatch: {actual_size} != {expected_size}"
            )));
        }
        if actual_size > MAX_PACKAGE_BYTES {
            return Err(AppError::Engine(format!(
                "MSIX size {actual_size} exceeds {MAX_PACKAGE_BYTES} byte limit"
            )));
        }
        progress(DownloadProgress {
            downloaded: actual_size,
            total: if expected_size > 0 {
                expected_size
            } else {
                actual_size
            },
            source,
        });

        let actual_sha = sha256_file(&dest).map_err(engine_err)?;
        if !actual_sha.eq_ignore_ascii_case(&expected_sha) {
            return Err(AppError::Engine(format!(
                "MSIX sha256 mismatch: {actual_sha} != {expected_sha}"
            )));
        }

        let authenticode = verify_openai_authenticode(&dest).map_err(engine_err)?;
        if !authenticode.is_valid_openai() {
            let err = AppError::Engine(format!(
                "MSIX Authenticode verification failed: status={}, subject={}",
                authenticode.status, authenticode.subject
            ));
            log::error!("Windows stage failed error={err}");
            return Err(err);
        }

        let identity = read_msix_identity(&dest).map_err(engine_err)?;
        validate_codex_identity(
            &identity,
            &report.release.package_version,
            report.release.architecture.as_deref(),
        )
        .map_err(engine_err)?;
        if let Some(expected_identity) = report.release.package_identity.as_deref() {
            if identity.name != expected_identity {
                let err = AppError::Engine(format!(
                    "MSIX identity {} does not match manifest package identity {}",
                    identity.name, expected_identity
                ));
                log::error!("Windows stage failed error={err}");
                return Err(err);
            }
        }

        let mut notes = report.plan.warnings.clone();
        notes.push(
            "MSIX is staged and verified; install execution will sideload first and fall back transparently to the portable path if sideloading fails."
                .to_string(),
        );

        Ok(WinStageReport {
            up_to_date: false,
            route,
            latest_version: report.plan.latest_version,
            package_moniker: report.plan.package_moniker,
            download_size: actual_size,
            staged_path: Some(dest.to_string_lossy().into_owned()),
            sha256: actual_sha,
            hash_verified: true,
            authenticode: Some(authenticode),
            identity: Some(identity),
            identity_verified: true,
            install_ready: true,
            portable_fallback_ready: report.plan.portable_fallback_ready,
            notes,
        })
    })();
    match stage_result {
        Ok(report) => {
            let route = &report.route;
            let verified = report.hash_verified && report.identity_verified;
            let portable_fallback_ready = report.portable_fallback_ready;
            log::info!(
                "Windows stage complete route={route} verified={verified} portable_fallback_ready={portable_fallback_ready}"
            );
            Ok(report)
        }
        Err(err) => {
            log::error!("Windows stage failed error={err}");
            Err(err)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalWinExpectation {
    pub current_path: Option<String>,
    pub current_version: Option<String>,
    pub current_source: Option<String>,
}

fn validate_historical_windows_expectation(
    expected: &HistoricalWinExpectation,
    current: Option<&InstalledWindowsCodex>,
) -> Result<(), AppError> {
    let actual_path = current.map(|installed| installed.path.as_str());
    let actual_version = current.map(|installed| installed.version.as_str());
    let actual_source = current.map(|installed| installed.source.as_str());
    if actual_path != expected.current_path.as_deref()
        || actual_version != expected.current_version.as_deref()
        || actual_source != expected.current_source.as_deref()
    {
        return Err(AppError::StaleExpectation(format!(
            "Windows Codex 在确认后发生变化（确认时 {:?} {:?} {:?}，现在 {:?} {:?} {:?}），请重新检查后再试",
            expected.current_path,
            expected.current_version,
            expected.current_source,
            actual_path,
            actual_version,
            actual_source
        )));
    }
    Ok(())
}

/// Inspect a local MSIX without consulting GitHub. Authenticode is the trust
/// anchor; package identity, publisher, architecture and internal Electron app
/// version are read directly from the signed package.
pub fn resolve_windows_local_package(
    path: &Path,
    architecture: ReleaseArchitecture,
) -> Result<ResolvedReleaseAsset, AppError> {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("msix"))
    {
        return Err(AppError::Internal(
            "请选择从 GitHub Release 下载的 MSIX 文件".to_string(),
        ));
    }
    let authenticode = verify_openai_authenticode(path).map_err(engine_err)?;
    if !authenticode.is_valid_openai() {
        return Err(AppError::Engine(format!(
            "本地 MSIX Authenticode 验证失败：status={}，subject={}",
            authenticode.status, authenticode.subject
        )));
    }
    let identity = read_msix_identity(path).map_err(engine_err)?;
    validate_codex_identity(&identity, &identity.version, Some(architecture.as_str()))
        .map_err(engine_err)?;
    let version = read_codex_app_version_from_msix(path)
        .ok_or_else(|| AppError::Engine("无法读取 MSIX 内 Codex 应用版本，拒绝安装".to_string()))?;
    resolved_local_asset(
        path,
        &version,
        architecture,
        ReleasePackageFormat::Msix,
        Some(identity.version),
    )
}

#[allow(clippy::too_many_arguments)]
fn stage_windows_historical_release(
    resolved: &ResolvedReleaseAsset,
    local_path: Option<PathBuf>,
    settings: &AppSettings,
    install_mode: &str,
    progress: &dyn Fn(DownloadProgress),
    network: &NetworkConfig,
) -> Result<WinStageReport, AppError> {
    if resolved.format != ReleasePackageFormat::Msix {
        return Err(AppError::Internal(
            "Windows 历史安装仅支持 GitHub Release 的 MSIX".to_string(),
        ));
    }
    if resolved.size == 0 || resolved.size > MAX_PACKAGE_BYTES {
        return Err(AppError::Engine(format!(
            "MSIX 大小 {} 超出允许范围",
            resolved.size
        )));
    }
    check_win_update_abort()?;
    let is_local = local_path.is_some();
    let package = if let Some(path) = local_path {
        let actual_size = std::fs::metadata(&path)
            .map_err(|e| AppError::Engine(format!("读取本地 MSIX 信息失败: {e}")))?
            .len();
        if actual_size != resolved.size {
            return Err(AppError::StaleExpectation(format!(
                "本地 MSIX 在确认后发生变化（大小 {} → {}），请重新选择",
                resolved.size, actual_size
            )));
        }
        verify_file_digest(&path, &resolved.digest)?;
        path
    } else {
        let dest = staging::download_cache_path(&resolved.url, &resolved.name)?;
        let cached_ok = std::fs::metadata(&dest)
            .map(|metadata| metadata.len() == resolved.size)
            .unwrap_or(false)
            && verify_file_digest(&dest, &resolved.digest).is_ok();
        let source = host_of(&resolved.url);
        if !cached_ok {
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
            }
            download_to_with_progress_bounded_with_network(
                &resolved.url,
                &dest,
                MAX_PACKAGE_BYTES,
                &|downloaded| {
                    progress(DownloadProgress {
                        downloaded,
                        total: resolved.size,
                        source: source.clone(),
                    });
                },
                network,
            )
            .map_err(engine_err)?;
        } else {
            progress(DownloadProgress {
                downloaded: resolved.size,
                total: resolved.size,
                source,
            });
        }
        let actual_size = std::fs::metadata(&dest)
            .map_err(|e| AppError::Engine(format!("读取下载 MSIX 信息失败: {e}")))?
            .len();
        if actual_size != resolved.size {
            return Err(AppError::Engine(format!(
                "MSIX 大小不匹配（{actual_size} != {}）",
                resolved.size
            )));
        }
        verify_file_digest(&dest, &resolved.digest)?;
        dest
    };
    check_win_update_abort()?;

    let authenticode = verify_openai_authenticode(&package).map_err(engine_err)?;
    if !authenticode.is_valid_openai() {
        return Err(AppError::Engine(format!(
            "MSIX Authenticode 验证失败：status={}，subject={}",
            authenticode.status, authenticode.subject
        )));
    }
    let identity = read_msix_identity(&package).map_err(engine_err)?;
    let package_version = resolved.package_version.as_deref().ok_or_else(|| {
        AppError::Engine("GitHub Release MSIX 名称缺少 package version".to_string())
    })?;
    validate_codex_identity(
        &identity,
        package_version,
        Some(resolved.architecture.as_str()),
    )
    .map_err(engine_err)?;
    validate_historical_windows_app_version(
        read_codex_app_version_from_msix(&package),
        &resolved.version,
    )?;

    // Deployment events identify the package by its signed manifest identity,
    // not by the on-disk filename (which may be `... (1).Msix` offline).
    let package_moniker = canonical_codex_package_moniker(&identity).map_err(engine_err)?;
    let release = WindowsRelease {
        version: resolved.version.clone(),
        package_version: package_version.to_string(),
        released_at: resolved.published_at.clone(),
        package_moniker: package_moniker.clone(),
        architecture: Some(resolved.architecture.as_str().to_string()),
        download_architecture: Some(resolved.architecture.as_str().to_string()),
        content_length: Some(resolved.size),
        etag: None,
        store_product_id: None,
        package_identity: Some(codex_win_engine::OPENAI_PACKAGE_IDENTITY.to_string()),
    };
    let capabilities = probe_capabilities();
    let mut plan = plan_update(
        &release,
        &resolved.digest,
        &resolved.url,
        &None,
        &capabilities,
        true,
    );
    // Preserve a managed portable install in-place for the normal manager. The
    // legacy fixed mode can still use the same verified local MSIX bytes as an
    // emergency extraction fallback. USB AppXVolume mode deliberately cannot:
    // creating a C: portable copy would defeat the removable-program boundary.
    let managed = detect_managed_codex(settings, &ProvenanceStore::load());
    let fixed_msix = !allows_portable_fallback(install_mode);
    if install_mode == "portable"
        || is_usb_portable_install_mode(install_mode)
        || (!fixed_msix
            && managed
                .as_ref()
                .is_some_and(|installed| installed.source == "portable"))
    {
        plan.route = WinInstallRoute::PortableFallback;
    }
    let route = route_label(&plan);
    let mut notes = plan.warnings;
    notes.push(if is_local {
        format!(
            "Verified local OpenAI-signed MSIX {} entirely on this PC; no GitHub or Sparkle request is required for offline installation.",
            resolved.name
        )
    } else {
        format!(
            "Verified GitHub Release {} asset {}; Sparkle is not used for historical installation.",
            resolved.release_tag, resolved.name
        )
    });
    notes.push(
        "MSIX Authenticode signature and package identity were verified before installation."
            .to_string(),
    );
    if is_usb_portable_install_mode(install_mode) {
        notes.push(
            "USB edition will extract the verified Codex program tree into the current bundle; no AppX volume or WpSystem package-data volume will be created."
                .to_string(),
        );
    }
    if is_local && install_mode == FIXED_MSIX_INSTALL_MODE {
        notes.push(
            "If Windows blocks AppX deployment in the legacy fixed mode, the verified local MSIX can be extracted as an emergency portable install; no network or alternate package is used."
                .to_string(),
        );
    }
    Ok(WinStageReport {
        up_to_date: false,
        route,
        latest_version: resolved.version.clone(),
        package_moniker,
        download_size: resolved.size,
        staged_path: Some(package.to_string_lossy().into_owned()),
        sha256: resolved.digest.clone(),
        hash_verified: true,
        authenticode: Some(authenticode),
        identity: Some(identity),
        identity_verified: true,
        install_ready: true,
        // USB AppXVolume mode never reports a portable fallback, even when the
        // staged package came from a local file. The legacy fixed mode retains
        // its same-bytes recovery behavior for backward compatibility.
        portable_fallback_ready: allows_historical_portable_recovery(install_mode, is_local),
        notes,
    })
}

fn validate_historical_windows_app_version(
    actual: Option<String>,
    expected: &str,
) -> Result<(), AppError> {
    let actual = actual
        .ok_or_else(|| AppError::Engine("无法读取 MSIX 内 Codex 应用版本，拒绝安装".to_string()))?;
    if actual != expected {
        return Err(AppError::Engine(format!(
            "MSIX 内 Codex 应用版本 {actual} 与所选 GitHub Release {expected} 不一致"
        )));
    }
    Ok(())
}

/// Verify and install a historical/local GitHub Release MSIX. The normal
/// Windows tail is reused verbatim, including ForceUpdateFromAnyVersion,
/// dependency pre-check, health probe, durable rollback and provenance
/// handling. Current USB mode treats the verified MSIX as an archive and writes
/// only the program tree to the removable bundle; legacy AppX-volume support is
/// retained solely for migration and cleanup of earlier builds.
#[allow(clippy::too_many_arguments)]
pub fn install_windows_historical_release(
    resolved: ResolvedReleaseAsset,
    local_path: Option<PathBuf>,
    settings: &AppSettings,
    install_mode: &str,
    expected: HistoricalWinExpectation,
    progress: &dyn Fn(DownloadProgress),
    network: &NetworkConfig,
    phase: Option<&PhaseHook<'_>>,
    evidence: Option<&EvidenceHook<'_>>,
    policy_transition: Option<SelfUpdatePolicyTransition>,
    before_commit: Option<&BeforeWinCommitHook<'_>>,
    appx_volume_root: Option<PathBuf>,
) -> Result<WinPerformReport, AppError> {
    let set_phase = |value: OperationPhase| {
        if let Some(hook) = phase {
            hook(value);
        }
    };
    let _abort_guard = WinAbortGuard::new();
    set_phase(OperationPhase::Preparing);
    let initial_status = win_install_status(settings);
    if is_usb_portable_install_mode(install_mode)
        && usb_install_blocks_existing_install(&initial_status, Path::new(&settings.install_root))
    {
        return Err(AppError::StaleExpectation(
            "检测到其他位置或其他来源已安装的 Codex；为保护现有安装，U 盘一键安装不会卸载、接管或覆盖它"
                .to_string(),
        ));
    }
    validate_historical_windows_expectation(&expected, initial_status.installed.as_ref())?;
    check_win_update_abort()?;
    set_phase(OperationPhase::Downloading);
    let local_package_supports_portable_recovery = local_path.is_some();
    let stage = stage_windows_historical_release(
        &resolved,
        local_path,
        settings,
        install_mode,
        progress,
        network,
    )?;
    let allow_portable_fallback =
        allows_historical_portable_recovery(install_mode, local_package_supports_portable_recovery);
    let appx_volume_root = appx_volume_root
        .as_deref()
        .map(appx_volume::validate_volume_root)
        .transpose()?;
    set_phase(OperationPhase::Verifying);
    let current_status = win_install_status(settings);
    if is_usb_portable_install_mode(install_mode)
        && usb_install_blocks_existing_install(&current_status, Path::new(&settings.install_root))
    {
        return Err(AppError::StaleExpectation(
            "确认后检测到其他位置或其他来源已安装的 Codex；为保护现有安装，U 盘一键安装已停止"
                .to_string(),
        ));
    }
    validate_historical_windows_expectation(&expected, current_status.installed.as_ref())?;
    perform_verified_windows_stage_tail(
        settings,
        stage,
        current_status.installed,
        Some(progress),
        phase,
        evidence,
        policy_transition,
        before_commit,
        allow_portable_fallback,
        is_usb_portable_install_mode(install_mode),
        appx_volume_root,
    )
}

pub fn auto_stage_windows_update(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    enabled: bool,
    allow_metered: bool,
) -> Result<WinAutoStageReport, AppError> {
    auto_stage_windows_update_with_install_mode(endpoints, settings, enabled, allow_metered, "msix")
}

pub fn auto_stage_windows_update_with_install_mode(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    enabled: bool,
    allow_metered: bool,
    install_mode: &str,
) -> Result<WinAutoStageReport, AppError> {
    auto_stage_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        enabled,
        allow_metered,
        install_mode,
        &NetworkConfig::system(),
    )
}

pub fn auto_stage_windows_update_with_install_mode_and_network(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    enabled: bool,
    allow_metered: bool,
    install_mode: &str,
    network: &NetworkConfig,
) -> Result<WinAutoStageReport, AppError> {
    log::info!("Windows auto-stage decision enabled={enabled} allow_metered={allow_metered}");
    if !enabled {
        return Ok(WinAutoStageReport {
            enabled,
            allow_metered,
            attempted: false,
            skipped: true,
            reason: "disabled".to_string(),
            stage: None,
            capabilities: None,
            notes: vec!["Automatic Windows pre-download is disabled.".to_string()],
        });
    }

    let report = plan_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        network,
    )?;
    let capabilities = report.capabilities.clone();
    if report.plan.up_to_date {
        return Ok(WinAutoStageReport {
            enabled,
            allow_metered,
            attempted: false,
            skipped: true,
            reason: "up-to-date".to_string(),
            stage: None,
            capabilities: Some(capabilities),
            notes: vec![
                "Windows Codex is already current; no background download needed.".to_string(),
            ],
        });
    }

    if !allow_metered {
        match report.capabilities.metered_network.state {
            CapabilityState::Available => {}
            CapabilityState::Unavailable => {
                let metered_state = report.capabilities.metered_network.state.as_str();
                log::warn!(
                    "Windows auto-stage skipped metered network metered_state={metered_state}"
                );
                return Ok(WinAutoStageReport {
                    enabled,
                    allow_metered,
                    attempted: false,
                    skipped: true,
                    reason: "metered-network".to_string(),
                    stage: None,
                    capabilities: Some(capabilities),
                    notes: vec![
                        "Automatic Windows pre-download was skipped because the current network is metered."
                            .to_string(),
                    ],
                });
            }
            CapabilityState::Unknown => {
                let metered_state = report.capabilities.metered_network.state.as_str();
                log::warn!(
                    "Windows auto-stage skipped metered network metered_state={metered_state}"
                );
                return Ok(WinAutoStageReport {
                    enabled,
                    allow_metered,
                    attempted: false,
                    skipped: true,
                    reason: "metered-unknown".to_string(),
                    stage: None,
                    capabilities: Some(capabilities),
                    notes: vec![
                        "Automatic Windows pre-download was skipped because metered-network status could not be determined."
                            .to_string(),
                    ],
                });
            }
        }
    }

    let stage = stage_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        &no_progress,
        network,
    )?;
    let notes = if stage.install_ready {
        vec!["Windows package is staged and ready for user-confirmed installation.".to_string()]
    } else {
        stage.notes.clone()
    };

    Ok(WinAutoStageReport {
        enabled,
        allow_metered,
        attempted: true,
        skipped: false,
        reason: "staged".to_string(),
        stage: Some(stage),
        capabilities: Some(capabilities),
        notes,
    })
}

/// Preparing-phase abort latch (mirrors the macOS one). Covers the gap before
/// the first byte — manifest/checksums fetch, planning — that the curl-level
/// cancel flag can't reach. Reset on op end via `WinAbortGuard`, not at entry.
static WIN_UPDATE_ABORT: AtomicBool = AtomicBool::new(false);

pub(crate) fn clear_win_update_abort() {
    WIN_UPDATE_ABORT.store(false, Ordering::SeqCst);
}

/// Resets the latch when the owning operation ends — on every path. Clearing on
/// DROP (not at entry) keeps the cancel race-free: a cancel landing between the
/// UI showing its button and the op reaching its first checkpoint isn't wiped, so
/// the checkpoint observes it; the next op still starts clean. The cancel command
/// now validates the owning token under the op lock, but the worker can still be
/// between entry and its first checkpoint. Owned by both
/// `perform` and `stage` (so background `auto_stage` and the standalone
/// `win_stage_update` can't leak a set latch into the next op). Nested guards are
/// reference-counted: an inner stage return cannot clear a quit/cancel owned by
/// the still-running outer perform operation.
pub(crate) struct WinAbortGuard;

static WIN_ABORT_GUARD_DEPTH: AtomicUsize = AtomicUsize::new(0);

impl WinAbortGuard {
    pub(crate) fn new() -> Self {
        WIN_ABORT_GUARD_DEPTH.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for WinAbortGuard {
    fn drop(&mut self) {
        let previous = WIN_ABORT_GUARD_DEPTH.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "WinAbortGuard depth underflow");
        if previous == 1 {
            clear_win_update_abort();
        }
    }
}

/// Bail out of the Windows preparing phase on a user cancel. Surfaces the same
/// "download cancelled" marker the curl-cancel path uses so the UI treats it as
/// a cancel uniformly.
fn check_win_update_abort() -> Result<(), AppError> {
    if WIN_UPDATE_ABORT.load(Ordering::SeqCst) {
        Err(AppError::Engine("download cancelled".to_string()))
    } else {
        Ok(())
    }
}

pub fn cancel_windows_download() -> bool {
    // Latch the preparing-phase abort too, so a cancel pressed before the first
    // byte (mid manifest-fetch) is honored at the next checkpoint. Report
    // actionable unconditionally — during preparing the latch IS the cancel.
    WIN_UPDATE_ABORT.store(true, Ordering::SeqCst);
    let requested = cancel_active_download();
    log::info!("Windows cancel download requested={requested}");
    true
}

pub fn pause_windows_download() -> bool {
    // Pause is only offered once bytes flow (UI disables it during preparing),
    // so it stays a pure download-loop operation — keep the `.part`.
    let requested = pause_active_download();
    log::info!("Windows pause download requested={requested}");
    requested
}

/// Paused-state cancel: the download already stopped and its `.part` is on disk.
/// Clear the cache so "继续" can't resume, and drop the abort latch. Surfaces a
/// removal failure rather than silently reporting a cancel that left the partial
/// behind (which a later run would resume).
pub fn discard_windows_download() -> Result<(), AppError> {
    clear_win_update_abort();
    staging::clear_download_cache()
        .map_err(|e| AppError::Engine(format!("清理下载缓存失败: {e}")))?;
    log::info!("Windows discard download cache");
    Ok(())
}

pub fn perform_windows_update(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    confirm: bool,
) -> Result<WinPerformReport, AppError> {
    perform_windows_update_with_install_mode(
        endpoints,
        settings,
        confirm,
        "msix",
        None,
        &no_progress,
    )
}

pub fn perform_windows_update_with_install_mode(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    confirm: bool,
    install_mode: &str,
    expected: Option<WinPerformExpectation>,
    progress: &dyn Fn(DownloadProgress),
) -> Result<WinPerformReport, AppError> {
    perform_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        confirm,
        install_mode,
        expected,
        progress,
        &NetworkConfig::system(),
    )
}

pub fn perform_windows_update_with_install_mode_and_network(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    confirm: bool,
    install_mode: &str,
    expected: Option<WinPerformExpectation>,
    progress: &dyn Fn(DownloadProgress),
    network: &NetworkConfig,
) -> Result<WinPerformReport, AppError> {
    perform_windows_update_with_install_mode_network_and_phase(
        endpoints,
        settings,
        confirm,
        install_mode,
        expected,
        progress,
        network,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn perform_windows_update_with_install_mode_network_and_phase(
    endpoints: &MirrorEndpoints,
    settings: &AppSettings,
    confirm: bool,
    install_mode: &str,
    expected: Option<WinPerformExpectation>,
    progress: &dyn Fn(DownloadProgress),
    network: &NetworkConfig,
    phase: Option<&PhaseHook<'_>>,
    evidence: Option<&EvidenceHook<'_>>,
) -> Result<WinPerformReport, AppError> {
    let set_phase = |p: OperationPhase| {
        if let Some(hook) = phase {
            hook(p);
        }
    };
    log::info!("Windows perform start install_mode={install_mode}");
    // Reset the latch when THIS perform ends (not at stage entry) so a cancel
    // racing the op's startup isn't wiped, and `auto_stage` never clears it. See
    // WinAbortGuard.
    let _abort_guard = WinAbortGuard::new();
    set_phase(OperationPhase::Preparing);
    if !confirm {
        return Err(AppError::Internal(
            "explicit confirmation is required before installing Windows Codex".to_string(),
        ));
    }

    set_phase(OperationPhase::Downloading);
    let stage = stage_windows_update_with_install_mode_and_network(
        endpoints,
        settings,
        install_mode,
        progress,
        network,
    )?;
    // Staging can take long enough for Codex to self-update, be uninstalled, or
    // move between the user's confirmation and our destructive work. Re-detect
    // after staging and use this fresh snapshot for both consent validation and
    // every close/provenance/install-root decision below.
    let store = ProvenanceStore::load();
    let current_installed = detect_managed_codex(settings, &store)
        .or_else(|| detect_installed_codex(PathBuf::from(&settings.install_root).as_path()));
    if let Some(expected) = &expected {
        validate_perform_expectation(expected, current_installed.as_ref(), &stage)?;
    }
    perform_verified_windows_stage_tail(
        settings,
        stage,
        current_installed,
        Some(progress),
        phase,
        evidence,
        None,
        None,
        true,
        is_usb_portable_install_mode(install_mode),
        None,
    )
}

// This is the single commit boundary for a staged Windows update. Keeping the
// policy hooks and USB placement evidence explicit makes the transaction order
// auditable at each call site.
#[allow(clippy::too_many_arguments)]
fn perform_verified_windows_stage_tail(
    settings: &AppSettings,
    stage: WinStageReport,
    current_installed: Option<InstalledWindowsCodex>,
    progress: Option<&dyn Fn(DownloadProgress)>,
    phase: Option<&PhaseHook<'_>>,
    evidence: Option<&EvidenceHook<'_>>,
    policy_transition: Option<SelfUpdatePolicyTransition>,
    before_commit: Option<&BeforeWinCommitHook<'_>>,
    allow_portable_fallback: bool,
    usb_portable: bool,
    appx_volume_root: Option<PathBuf>,
) -> Result<WinPerformReport, AppError> {
    let set_phase = |p: OperationPhase| {
        if let Some(hook) = phase {
            hook(p);
        }
    };
    if stage.up_to_date {
        let mut notes = stage.notes.clone();
        if appx_volume_root.is_some() {
            match appx_volume::ensure_system_app_data_policy() {
                Ok(policy) => notes.push(format!("USB AppX 模式：{}", policy.message)),
                Err(error) => notes.push(format!(
                    "USB AppX 数据策略未能启用，仍保留 USB 程序边界；应用数据可能随包卷保存：{}",
                    error
                )),
            }
        }
        return Ok(WinPerformReport {
            success: true,
            action: WinPerformAction::None,
            message: "Windows Codex is already current.".to_string(),
            installed: win_install_status(settings).installed,
            sideload: None,
            portable: None,
            msix_health: None,
            fallback_available: stage.portable_fallback_ready,
            fallback_attempted: false,
            notes,
            stage,
            outcome: OperationOutcome::full_success(
                "present",
                Some(if win_install_status(settings).status == "managed" {
                    "managed"
                } else {
                    "external"
                }),
            ),
        });
    }

    // Capture the user's app state before any update path closes or probes
    // Codex. Both MSIX and portable installs perform a real launch health check;
    // this decides whether that checked process remains open afterward.
    let was_running = existing_codex_was_running(settings, current_installed.as_ref())?;

    if stage.route == "portable-fallback" {
        if !allow_portable_fallback {
            return Err(AppError::Engine(
                "离线固定 MSIX 模式不允许切换到便携版".to_string(),
            ));
        }
        // Portable fallback still has to unpack and copy the complete MSIX
        // before the first install-root rename. Keep that work in the
        // interruptible Applying phase; entering Committing here made a slow
        // USB extraction look like a frozen "replacing files" operation and
        // blocked every normal close request for the whole copy.
        set_phase(OperationPhase::Applying);
        // Linearized with every native/window quit path by OperationManager.
        check_win_update_abort()?;
        log::warn!("Windows route changed to portable fallback from_route=msix-sideload to_route=portable-fallback");
        close_existing_codex_before_portable_fallback(settings, current_installed.as_ref())?;
        return install_portable_after_stage(
            settings,
            stage,
            None,
            None,
            was_running,
            current_installed,
            usb_portable,
            progress,
            phase,
            evidence,
            policy_transition,
            before_commit,
        );
    }

    // The MSIX sideload path reaches its point of no return only immediately
    // before the deployment call can mutate package registration.  Keep the
    // original hard close boundary for that path, but do not share it with the
    // portable pre-commit extraction above.
    set_phase(OperationPhase::Committing);
    // Linearized with every native/window quit path by OperationManager.
    check_win_update_abort()?;

    let staged_path = stage
        .staged_path
        .as_ref()
        .ok_or_else(|| AppError::Engine("staged MSIX path missing".to_string()))?
        .clone();

    // PRE-check the staged MSIX's declared framework dependencies BEFORE touching
    // the running install or attempting the sideload. On a stripped / China /
    // Store-disabled Windows, `Add-AppxPackage` cannot auto-acquire a missing
    // framework (VCLibs / WindowsAppRuntime / UI.Xaml / NET.Native), so the
    // sideload is doomed — it either errors or registers a package that won't
    // launch. When a required framework is positively missing we route straight
    // to the portable build instead of burning a failed attempt. The probe is
    // conservative: if the manifest can't be read or the check can't run it
    // returns `checked = false` and we proceed to the sideload as before, where
    // the post-install health check + transparent fallback remain the backstop.
    let precheck = precheck_msix_dependencies(PathBuf::from(&staged_path).as_path());
    validate_fixed_msix_dependencies(&precheck, allow_portable_fallback)?;
    if precheck.should_route_portable() && allow_portable_fallback {
        log::warn!("Windows route changed to portable fallback from_route=msix-sideload to_route=portable-fallback");
        // Dependency inspection proved that no AppX mutation is needed. The
        // following portable extraction is still pre-rename work, so expose it
        // as Applying rather than leaving the UI/quit policy in Committing.
        set_phase(OperationPhase::Applying);
        check_win_update_abort()?;
        // We're switching to portable, but the running build must be stopped first.
        close_existing_codex_before_portable_fallback(settings, current_installed.as_ref())?;
        let mut stage = stage;
        stage.notes.push(format!(
            "Skipped MSIX sideload before attempting it: {}. Routed to the portable build, which carries its own runtime and does not need these framework packages.",
            precheck.reason
        ));
        let mut report = install_portable_after_stage(
            settings,
            stage,
            None,
            None,
            was_running,
            current_installed,
            usb_portable,
            progress,
            phase,
            evidence,
            policy_transition,
            before_commit,
        )?;
        report.action = WinPerformAction::PortableFallbackMissingFramework;
        return Ok(report);
    }

    let installed_before_sideload =
        detect_installed_codex(PathBuf::from(&settings.install_root).as_path());
    let mut msix_policy_tx: Option<ActiveMsixPolicyTransition> = None;
    let mut usb_data_policy_changed = false;
    // This is deliberately a success marker, not an "attempted" marker.  A
    // failed/declined combined UAC operation must not suppress the targeted
    // Add-AppxVolume retry after the normal installer reports Access denied.
    let mut usb_combined_prerequisites_succeeded = false;
    let mut mark_recovery_ambiguous = || {
        set_phase(OperationPhase::Committing);
        if let Some(hook) = evidence {
            hook(OperationEvidence::OutcomeAmbiguous);
        }
    };
    // Keep the user's current Codex running while conflict detection and its
    // optional UAC recovery execute. The engine calls this fallible boundary
    // only after recovery succeeds and immediately before Add-AppxPackage can
    // mutate registration. A declined UAC prompt therefore leaves Codex open.
    let mut close_codex_and_mark_install_started = || -> Result<(), EngineError> {
        if let Some(volume_root) = appx_volume_root.as_deref() {
            // Staging can take minutes. Revalidate the live lease immediately
            // before closing the current app or invoking Add-AppxPackage so a
            // drive-letter reuse cannot redirect the first attempt.
            usb_guard::verify_authorized_volume_root(volume_root)
                .map_err(|error| EngineError::Install(error.to_string()))?;
            if !usb_appx_recovery_pending() {
                crate::app::offline_payload::validate_usb_deployment_space(
                    Path::new(&staged_path),
                    volume_root,
                )
                .map_err(|error| EngineError::Install(error.to_string()))?;
            } else {
                // A completed USB Stage already occupies the package's disk
                // budget. Let the authenticated recovery state machine decide
                // whether another Stage/removal is required. It checks space
                // again BEFORE either operation, but not before a register-only
                // or data-restore resume. A journal alone never proves Stage.
                log::info!("USB recovery pending; deployment space gate delegated to verified recovery phase");
            }
            // AppX volume migration and package registration are per-user
            // operations that must run in the same full-administrator manager
            // session which passed the USB identity check. A helper UAC prompt
            // cannot repair a medium-integrity manager after this point: the
            // package could be registered for the wrong account or silently
            // land on C:. Fail before closing Codex and make the missing token
            // explicit in the UI/log instead of allowing a misleading partial
            // install.
            #[cfg(target_os = "windows")]
            match crate::app::win_elevation::current_process_is_elevated() {
                Ok(true) => {
                    log::info!("USB AppX install boundary has a full administrator manager token");
                }
                Ok(false) => {
                    return Err(EngineError::Install(
                        "U 盘安装器当前进程未获得管理员令牌；请从 CodexDesktopSetup.exe 启动并在 UAC 对话框中选择“是”后重试"
                            .to_string(),
                    ));
                }
                Err(error) => {
                    return Err(EngineError::Install(format!(
                        "无法确认 U 盘安装器的管理员令牌，已停止安装：{error}"
                    )));
                }
            }
        }
        // The USB product promises local user data AND USB program files.
        // Verify both prerequisites before closing applications or deploying.
        // A denied/malformed policy is not an absent value, and must not turn
        // into silent data-on-USB or a system-volume bootstrap installation.
        if let Some(volume_root) = appx_volume_root.as_deref() {
            let policy = appx_volume::ensure_usb_appx_prerequisites(volume_root)
                .map_err(|error| EngineError::Install(format!(
                    "USB 安装前置检查失败，尚未部署或迁移 Codex；必须确认本地数据策略和目标 U 盘卷均可用：{error}"
                )))?;
            usb_data_policy_changed = policy.changed;
            usb_combined_prerequisites_succeeded = true;
            log::info!(
                "USB AppX prerequisites verified target={} data_policy=RestrictAppDataToSystemVolume policy_changed={} policy_value={:?}",
                volume_root.display(),
                policy.changed,
                policy.current_value
            );
        }
        if let Some(installed) = &installed_before_sideload {
            if installed.source == "msix" {
                close_codex_gracefully_for_root(30, PathBuf::from(&installed.path).as_path())?;
            }
        }
        // A managed portable build (possibly under a previous install root) is
        // not stopped by an MSIX sideload, so close it at the same last safe
        // boundary. `current_installed` is provenance-aware.
        if let Some(previous) = &current_installed {
            if previous.source == "portable" {
                close_codex_gracefully_for_root(30, PathBuf::from(&previous.path).as_path())?;
            }
        }
        if let Some(volume_root) = appx_volume_root.as_deref() {
            // The combined prerequisite path may have waited on a UAC dialog.
            // Check the lease once more after that wait and before opening the
            // durable install transaction; if the disk changed, restore the
            // policy we just applied and leave Codex untouched.
            if let Err(error) = usb_guard::verify_authorized_volume_root(volume_root) {
                if usb_data_policy_changed {
                    if let Err(restore) = appx_volume::restore_system_app_data_policy() {
                        log::warn!(
                            "restore AppX data policy after USB identity change error={restore}"
                        );
                    }
                    usb_data_policy_changed = false;
                }
                return Err(EngineError::Install(format!(
                    "U 盘身份在安装前发生变化，已停止安装：{error}"
                )));
            }
        }
        // The exact signed PackageFullName and the requested/previous policy are
        // durable before the child that can call Add-AppxPackage starts. Startup
        // can therefore resolve a crash after deployment from registered identity.
        if let Some(transition) = policy_transition {
            msix_policy_tx = Some(
                ActiveMsixPolicyTransition::begin(&stage.package_moniker, transition)
                    .map_err(|e| EngineError::Io(e.to_string()))?,
            );
        }
        if let Some(hook) = before_commit {
            if let Err(err) = hook() {
                if usb_data_policy_changed {
                    if let Err(restore) = appx_volume::restore_system_app_data_policy() {
                        log::warn!(
                            "restore AppX data policy after pre-commit failure error={restore}"
                        );
                    }
                }
                let rollback_warnings = msix_policy_tx
                    .take()
                    .map(ActiveMsixPolicyTransition::rollback_not_landed)
                    .unwrap_or_default();
                let suffix = if rollback_warnings.is_empty() {
                    String::new()
                } else {
                    format!("；{}", rollback_warnings.join("；"))
                };
                return Err(EngineError::Io(format!(
                    "persist historical self-update policy before MSIX deployment: {err}{suffix}"
                )));
            }
        }
        set_phase(OperationPhase::Committing);
        if let Some(hook) = evidence {
            hook(OperationEvidence::OutcomeAmbiguous);
        }
        Ok(())
    };
    let sideload_result = if let Some(volume_root) = appx_volume_root.as_deref() {
        install_msix_to_volume_with_observer(
            PathBuf::from(&staged_path).as_path(),
            &stage.package_moniker,
            volume_root,
            &mut mark_recovery_ambiguous,
            &mut close_codex_and_mark_install_started,
        )
    } else {
        install_msix_sideload_with_observer(
            PathBuf::from(&staged_path).as_path(),
            &stage.package_moniker,
            &mut mark_recovery_ambiguous,
            &mut close_codex_and_mark_install_started,
        )
    };
    let mut sideload = match sideload_result {
        Ok(report) => report,
        Err(error) => {
            return Err(interrupted_msix_error(
                error,
                &stage.package_moniker,
                appx_volume_root.as_deref(),
                Path::new(&settings.install_root),
                usb_data_policy_changed,
            ));
        }
    };

    // `Add-AppxVolume` writes machine-level AppX metadata.  On some Windows
    // builds the initial package script can read/create the USB path as the
    // current user but still returns `Access is denied` when it registers the
    // volume.  Request elevation only for that exact operation, then retry the
    // same verified MSIX once.  We intentionally do not rerun the full
    // pre-install closure: it already closed Codex and opened the durable
    // policy transaction before the first attempt, and starting a second
    // transaction would leave a spurious pending journal behind.
    if let Some(volume_root) = appx_volume_root.as_deref() {
        // A few down-level AppX implementations return success while silently
        // placing the package on the system volume.  Treat that as a retryable
        // target-volume failure as well as the traditional Add-AppxVolume
        // Access-denied result.  The generated script now rejects this itself,
        // but keeping the Rust-side recovery makes already-staged/older scripts
        // safe too.
        let sideload_reports_wrong_usb_volume = sideload.success
            && !sideload
                .installed
                .as_ref()
                .is_some_and(|installed| installed_msix_matches_volume(installed, volume_root));
        let needs_usb_volume_retry =
            should_retry_usb_volume_registration(&sideload, usb_combined_prerequisites_succeeded)
                || sideload_reports_wrong_usb_volume;
        if needs_usb_volume_retry {
            log::warn!(
                "USB AppX target volume requires one-shot registration/retry root={} report_success={} wrong_volume={} ",
                volume_root.display(),
                sideload.success,
                sideload_reports_wrong_usb_volume
            );
            // Re-check the lease fingerprint immediately before the UAC
            // boundary.  A drive-letter reuse while the consent dialog is
            // open must not turn the elevated write into an operation on a
            // different removable disk.
            let target_still_authorized =
                match usb_guard::verify_authorized_volume_root(volume_root) {
                    Ok(()) => true,
                    Err(error) => {
                        let detail = format!(
                            "{}；提权前 U 盘身份复核失败，未执行管理员卷操作：{}",
                            sideload.message, error
                        );
                        sideload.message = detail.clone();
                        sideload.raw_error = Some(match sideload.raw_error.take() {
                            Some(raw) => format!("{raw}\n{detail}"),
                            None => detail,
                        });
                        log::warn!("USB volume identity changed before elevation error={error}");
                        false
                    }
                };
            // The helper above is the only guard needed here.  In
            // particular, a failed combined prerequisite prompt must not be
            // treated as a successful volume registration.
            if target_still_authorized {
                match appx_volume::ensure_appx_volume_registered(volume_root) {
                    Ok(()) => {
                        // The elevated script validates the drive root and
                        // filesystem, while this check validates the product's
                        // serial/anchor fingerprint after the UAC boundary.
                        let target_still_authorized =
                            match usb_guard::verify_authorized_volume_root(volume_root) {
                                Ok(()) => true,
                                Err(error) => {
                                    let detail = format!(
                                        "{}；提权后 U 盘身份复核失败，未重试 MSIX 安装：{}",
                                        sideload.message, error
                                    );
                                    sideload.message = detail.clone();
                                    sideload.raw_error = Some(match sideload.raw_error.take() {
                                        Some(raw) => format!("{raw}\n{detail}"),
                                        None => detail,
                                    });
                                    log::warn!(
                                        "USB volume identity changed after elevation error={error}"
                                    );
                                    false
                                }
                            };
                        if target_still_authorized {
                            let mut mark_retry_install_started = || -> Result<(), EngineError> {
                                set_phase(OperationPhase::Committing);
                                if let Some(hook) = evidence {
                                    hook(OperationEvidence::OutcomeAmbiguous);
                                }
                                Ok(())
                            };
                            let retry_result = install_msix_to_volume_with_observer(
                                PathBuf::from(&staged_path).as_path(),
                                &stage.package_moniker,
                                volume_root,
                                &mut mark_recovery_ambiguous,
                                &mut mark_retry_install_started,
                            );
                            sideload = match retry_result {
                                Ok(report) => report,
                                Err(error) => {
                                    return Err(interrupted_msix_error(
                                        error,
                                        &stage.package_moniker,
                                        appx_volume_root.as_deref(),
                                        Path::new(&settings.install_root),
                                        usb_data_policy_changed,
                                    ));
                                }
                            };
                        }
                    }
                    Err(error) => {
                        let detail = format!(
                            "{}；USB AppX 卷注册需要一次管理员确认，但未完成：{}",
                            sideload.message, error
                        );
                        sideload.message = detail.clone();
                        sideload.raw_error = Some(match sideload.raw_error.take() {
                            Some(raw) => format!("{raw}\n{detail}"),
                            None => detail,
                        });
                        log::warn!("elevated USB AppX volume registration failed error={error}");
                    }
                }
            }
        }
    }

    // A structured failure can still follow a partially successful deployment.
    // Re-query exact PackageFullName before deciding whether the durable policy
    // intent commits or rolls back. If the probe itself is unavailable, retain
    // the safer requested policy: the package outcome is genuinely ambiguous.
    let usb_volume_mismatch = if sideload.success {
        appx_volume_root.as_deref().is_some_and(|root| {
            !sideload
                .installed
                .as_ref()
                .is_some_and(|installed| installed_msix_matches_volume(installed, root))
        })
    } else {
        false
    };
    let failure_policy = if sideload.success && !usb_volume_mismatch {
        FailedMsixPolicyState {
            target_landed_or_unknown: true,
            keep_usb_data_policy: appx_volume_root.is_some(),
        }
    } else {
        probe_failed_msix_policy(
            &stage.package_moniker,
            appx_volume_root.as_deref(),
            Path::new(&settings.install_root),
            false,
        )
    };
    if (!sideload.success || usb_volume_mismatch)
        && !failure_policy.keep_usb_data_policy
        && usb_data_policy_changed
    {
        if let Err(restore) = appx_volume::restore_system_app_data_policy() {
            log::warn!("restore AppX data policy after failed USB deployment error={restore}");
        }
    }
    let msix_policy_warnings = msix_policy_tx
        .take()
        .map(|active| {
            if failure_policy.target_landed_or_unknown {
                active.commit_landed()
            } else {
                active.rollback_not_landed()
            }
        })
        .unwrap_or_default();

    if usb_volume_mismatch {
        let expected = appx_volume_root
            .as_deref()
            .map(|root| root.display().to_string())
            .unwrap_or_else(|| "USB 卷".to_string());
        let actual = sideload
            .installed
            .as_ref()
            .and_then(|installed| installed.package_volume.as_deref())
            .unwrap_or("未知");
        return Err(AppError::Engine(format!(
            "Add-AppxPackage 已返回成功，但无法确认 Codex 位于目标 USB AppX 卷（目标 {expected}，实际 {actual}）；未创建 portable 副本，请检查 Windows AppX 卷状态后重试或卸载"
        )));
    }

    if sideload.success {
        // Add-AppxPackage returning success only means the cmdlet didn't throw.
        // Verify the package is actually runnable before committing to it — on a
        // stripped Windows it can register yet fail to launch. If it's unhealthy,
        // first install the portable fallback so the user is never left without
        // a runnable build, then clean up the bad MSIX best-effort.
        let health = verify_msix_health_with_options(was_running);
        if !health.healthy && allow_portable_fallback {
            log::warn!("Windows route changed to portable fallback from_route=msix-sideload to_route=portable-fallback");
            // Activation probe may have started Codex (or left a half-started
            // process). Close it before portable install and Remove-AppxPackage
            // so package files unlock cleanly. Prefer the sideload install path
            // when known; always also sweep via the registered package location.
            if let Some(installed) = sideload.installed.as_ref() {
                if let Err(err) =
                    close_codex_gracefully_for_root(20, PathBuf::from(&installed.path).as_path())
                {
                    log::warn!(
                        "close MSIX Codex after unhealthy health check (install path) error={err}"
                    );
                }
            }
            if let Err(err) = close_msix_codex_processes(20) {
                log::warn!(
                    "close MSIX Codex after unhealthy health check (package sweep) error={err}"
                );
            }
            let mut report = install_portable_after_stage(
                settings,
                stage,
                Some(sideload),
                Some(health),
                was_running,
                current_installed,
                usb_portable,
                progress,
                phase,
                evidence,
                None,
                None,
            )?;
            report.notes.extend(msix_policy_warnings.iter().cloned());
            for warning in msix_policy_warnings {
                report.outcome.push_warning(warning);
            }
            return Ok(report);
        }
        if !health.healthy {
            return Err(AppError::Engine(format!(
                "MSIX 离线安装后的健康检查失败，且固定 MSIX 模式不允许便携版回退：{}",
                health.reason
            )));
        }

        let installed = sideload
            .installed
            .clone()
            .or_else(|| win_install_status(settings).installed);
        let mut notes = stage.notes.clone();
        let mut outcome = match &installed {
            Some(installed) => {
                let outcome = record_managed_install(
                    current_installed.as_ref(),
                    installed,
                    "manager-installed-msix",
                );
                notes.extend(outcome.warnings.iter().cloned());
                outcome
            }
            None => {
                // Sideload claimed success but we cannot see an install — do not
                // pretend managed/ok provenance.
                let mut outcome = OperationOutcome {
                    primary_ok: true,
                    app_state: "unknown".to_string(),
                    install_class: None,
                    path: None,
                    provenance: StepOutcome::failed("安装完成但未检测到可记录的安装，托管状态未知"),
                    cleanup: StepOutcome::not_applicable(),
                    warnings: vec![
                        "MSIX sideload reported success but no install was detected afterward."
                            .to_string(),
                    ],
                    recovery_actions: vec![recovery::RECORD_PROVENANCE.to_string()],
                };
                notes.extend(outcome.warnings.iter().cloned());
                outcome.push_warning(
                    "请重新检查状态；若 Codex 已可用，用「开始管理」写入托管记录，勿盲目重装"
                        .to_string(),
                );
                outcome
            }
        };
        for warning in &msix_policy_warnings {
            outcome.push_warning(warning.clone());
        }
        notes.extend(msix_policy_warnings);

        let report = WinPerformReport {
            success: true,
            action: WinPerformAction::MsixSideload,
            message: if outcome.is_partial() || outcome.provenance.is_failed() {
                format!(
                    "{}（已安装，但托管记录未写入 — 请「开始管理」重试，勿重复安装）",
                    sideload.message
                )
            } else {
                sideload.message.clone()
            },
            installed,
            sideload: Some(sideload),
            portable: None,
            msix_health: Some(health),
            fallback_available: stage.portable_fallback_ready,
            fallback_attempted: false,
            notes,
            stage,
            outcome,
        };
        let action = report.action.as_str();
        let installed_version = report
            .installed
            .as_ref()
            .map(|installed| installed.version.as_str())
            .unwrap_or("none");
        log::info!("Windows perform success action={action} installed_version={installed_version}");
        return Ok(report);
    }

    if !allow_portable_fallback {
        return Err(AppError::Engine(format!(
            "MSIX 离线安装失败，且固定 MSIX 模式不允许便携版回退：{}",
            sideload.message
        )));
    }
    // Preserve the original AppX HRESULT/ActivityId if extraction itself also
    // fails. Without this wrapper the user would only see a filesystem or
    // process error and lose the actual Windows deployment diagnosis.
    let sideload_failure = sideload.message.clone();
    let mut report = install_portable_after_stage(
        settings,
        stage,
        Some(sideload),
        None,
        was_running,
        current_installed,
        usb_portable,
        progress,
        phase,
        evidence,
        policy_transition,
        before_commit,
    )
    .map_err(|err| {
        AppError::Engine(format!(
            "MSIX 离线安装失败（{}）；同一份 MSIX 的本地便携回退也失败：{}",
            sideload_failure, err
        ))
    })?;
    report.notes.extend(msix_policy_warnings.iter().cloned());
    for warning in msix_policy_warnings {
        report.outcome.push_warning(warning);
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn install_portable_after_stage(
    settings: &AppSettings,
    stage: WinStageReport,
    sideload: Option<MsixSideloadReport>,
    health: Option<MsixHealthReport>,
    relaunch: bool,
    previous_installed: Option<InstalledWindowsCodex>,
    usb_portable: bool,
    progress: Option<&dyn Fn(DownloadProgress)>,
    phase: Option<&PhaseHook<'_>>,
    evidence: Option<&EvidenceHook<'_>>,
    policy_transition: Option<SelfUpdatePolicyTransition>,
    before_commit: Option<&BeforeWinCommitHook<'_>>,
) -> Result<WinPerformReport, AppError> {
    let cleanup_registered_msix = portable_fallback_needs_msix_cleanup(
        previous_installed
            .as_ref()
            .map(|installed| installed.source.as_str()),
        sideload.is_some(),
    );
    let staged_path = stage
        .staged_path
        .as_ref()
        .ok_or_else(|| AppError::Engine("staged MSIX path missing".to_string()))?;
    let install_root = if usb_portable {
        // Never inherit a C: portable path from an older manager. The command
        // layer has already derived and validated this root from the live USB.
        settings.install_root.clone()
    } else {
        previous_installed
            .as_ref()
            .filter(|installed| installed.source == "portable")
            .map(|installed| installed.path.clone())
            .unwrap_or_else(|| settings.install_root.clone())
    };
    let install_root_path = PathBuf::from(&install_root);
    let msix_path = PathBuf::from(staged_path);
    // Persist the transaction log before the first possible rename using the
    // real paths chosen by the engine. Once that journal is durable, publish
    // `Committing` before the first rename so the app cannot exit in the narrow
    // gap between the boundary callback and the filesystem call. Completion
    // remains pre-write until a later boundary proves a rename succeeded.
    let mut tx: Option<ActiveInstallTx> = None;
    let mut observer = |boundary: PortableBoundary| -> Result<(), codex_win_engine::EngineError> {
        if matches!(&boundary, PortableBoundary::BeforeRollback { .. }) {
            if let Some(active) = tx.as_mut() {
                active
                    .mark_rollback_started()
                    .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
            }
            return Ok(());
        }
        if matches!(&boundary, PortableBoundary::RollbackCompleted { .. }) {
            // Disk rollback is already complete. Persist and clear the durable
            // journal first; only then publish evidence that permits a retry.
            if let Some(active) = tx.take() {
                active
                    .mark_rolled_back()
                    .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
            }
            if let Some(hook) = evidence {
                hook(OperationEvidence::MutationRolledBack);
            }
            return Ok(());
        }
        if portable_boundary_mutated_install_root(&boundary) {
            if let Some(hook) = evidence {
                hook(OperationEvidence::MutationStarted);
            }
        }
        match boundary {
            PortableBoundary::BeforeMoveOld {
                install_root,
                payload,
                backup,
                had_previous,
            } => {
                if usb_portable {
                    let authorization =
                        usb_guard::require_signed_codex_bundle("portable-usb-precommit")
                            .map_err(|error| EngineError::Io(error.to_string()))?
                            .ok_or_else(|| {
                                EngineError::Io(
                                    "USB portable 提交前未获得当前 bundle 授权".to_string(),
                                )
                            })?;
                    if !codex_win_engine::same_windows_path(
                        Path::new(&install_root),
                        &authorization.codex_root,
                    ) {
                        return Err(codex_win_engine::EngineError::Io(
                            "USB portable 提交目标已偏离当前授权 bundle".to_string(),
                        ));
                    }
                }
                let mut started = ActiveInstallTx::begin(
                    InstallTxKind::WindowsPortable,
                    &install_root,
                    &payload,
                    &backup,
                    had_previous,
                    None,
                )
                .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
                if let Some(transition) = policy_transition {
                    started
                        .set_self_update_policy_transition(transition)
                        .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
                }
                tx = Some(started);
                if let Some(hook) = phase {
                    hook(OperationPhase::Committing);
                }
                // The journal now durably owns both the filesystem swap and the
                // policy transition. A crash before the first rename restores
                // the old policy; a landed payload finalizes the requested one.
                if let Some(hook) = before_commit {
                    hook().map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
                }
                Ok(())
            }
            PortableBoundary::AfterMoveOld { .. } => {
                if let Some(active) = tx.as_mut() {
                    active
                        .mark_old_moved()
                        .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
                }
                Ok(())
            }
            PortableBoundary::BeforeMoveNew { .. } => Ok(()),
            PortableBoundary::AfterMoveNew { .. } => {
                if let Some(active) = tx.as_mut() {
                    active
                        .mark_new_installed()
                        .map_err(|e| codex_win_engine::EngineError::Io(e.to_string()))?;
                }
                Ok(())
            }
            PortableBoundary::BeforeRollback { .. }
            | PortableBoundary::RollbackCompleted { .. } => unreachable!("handled above"),
        }
    };
    let extraction_progress = |update: PortableExtractionProgress| {
        if let Some(hook) = progress {
            hook(DownloadProgress {
                downloaded: update.completed_bytes,
                total: update.total_bytes,
                source: "U盘文件解压".to_string(),
            });
        }
        Ok(())
    };
    let portable_options = if usb_portable {
        let bundle_root = usb_guard::current_bundle_root().ok_or_else(|| {
            AppError::Engine("无法从当前 U 盘管理器解析 direct-entry shim 来源".to_string())
        })?;
        let shim = bundle_root.join("CodexEntryShim.exe");
        if !shim.is_file() {
            return Err(AppError::Engine(format!(
                "U 盘缺少 Codex 直接入口组件：{}",
                shim.display()
            )));
        }
        PortableInstallOptions {
            usb_entry_shim: Some(shim),
            ..PortableInstallOptions::usb_offline()
        }
    } else {
        PortableInstallOptions {
            close_running: true,
            launch_health_check: true,
            relaunch,
            create_shell_metadata: true,
            usb_entry_shim: None,
        }
    };
    let portable_result = install_portable_from_msix_with_observer_options_and_progress(
        msix_path.as_path(),
        install_root_path.as_path(),
        portable_options,
        &mut observer,
        Some(&extraction_progress),
    );
    let portable = portable_result.map_err(|err| {
        log::error!(
            "Windows portable install failed install_root={} error={err}",
            install_root_path.display()
        );
        engine_err(err)
    })?;
    let tx_finalize_warnings = tx.take().map(ActiveInstallTx::succeed).unwrap_or_default();
    if let Some(hook) = phase {
        hook(OperationPhase::Finishing);
    }

    // Detect the PORTABLE install we just wrote — not detect_installed_codex,
    // which prefers MSIX and would return a still-present older MSIX package
    // (e.g. when sideload was blocked by policy), recording the wrong target so
    // the user keeps seeing the same update and the portable build goes unmanaged.
    let installed = detect_portable_install(PathBuf::from(&install_root).as_path());
    let mut outcome = match &installed {
        Some(installed) => record_managed_install(
            previous_installed.as_ref(),
            installed,
            if usb_portable {
                "manager-installed-usb-portable"
            } else {
                "manager-installed-portable"
            },
        ),
        None => OperationOutcome {
            primary_ok: portable.success,
            app_state: if portable.success {
                "unknown".to_string()
            } else {
                "absent".to_string()
            },
            install_class: None,
            path: Some(install_root.clone()),
            provenance: StepOutcome::failed("便携安装完成但未检测到可记录的安装，托管状态未知"),
            cleanup: StepOutcome::not_applicable(),
            warnings: vec![
                "Portable install finished but no install was detected for provenance.".to_string(),
            ],
            recovery_actions: vec![recovery::RECORD_PROVENANCE.to_string()],
        },
    };
    for warning in &tx_finalize_warnings {
        outcome.push_warning(warning.clone());
    }

    let mut notes = stage.notes.clone();
    let msix_unhealthy = health.as_ref().is_some_and(|h| !h.healthy);
    if let Some(health) = &health {
        if !health.healthy {
            notes.push(format!(
                "MSIX installed but failed its post-install health check ({}); switched to the portable build.",
                health.reason
            ));
        }
    } else if let Some(sideload) = &sideload {
        notes.push(format!(
            "MSIX sideload failed without elevation or policy changes: {}",
            sideload.message
        ));
        notes.push(
            "已改用同一份通过 SHA-256 与 Authenticode 校验的本地 MSIX 解包安装；未下载或执行其他版本。"
                .to_string(),
        );
    }
    notes.extend(portable.notes.clone());
    if usb_portable {
        let local = usb_guard::local_data_paths()?;
        notes.push(format!(
            "USB program boundary: {}; local CODEX_HOME: {}; roaming app data: {}; local app data: {}",
            install_root_path.display(),
            local.codex_home.display(),
            local.roaming_app_data.display(),
            local.local_app_data.display()
        ));
    }

    // A portable fallback is not a complete downgrade while Windows still has
    // the superseded MSIX registered: Start Menu activation would continue to
    // launch that package, and uninstalling the portable tree would reveal it
    // again. Wait until the portable payload has passed its real launch health
    // check above, then remove any previous/attempted MSIX registration. Failure
    // is ancillary (the requested portable build is runnable) but must be shown
    // as a partial outcome instead of silently claiming a clean success. USB
    // mode relies on static manifest/entry/copy-length integrity here and does
    // not launch Codex before endpoint configuration.
    if cleanup_registered_msix {
        if let Err(err) = close_msix_codex_processes(20) {
            notes.push(format!(
                "Portable fallback succeeded, but closing the superseded MSIX before cleanup reported: {err}"
            ));
        }
        // Portable fallback must not erase a user's existing AppX data while
        // cleaning the superseded registration.  The explicit uninstall path
        // below passes the user's purge choice instead.
        match codex_win_engine::remove_msix_package_with_options(true) {
            Ok(remove) if remove.success => {
                outcome.cleanup = StepOutcome::ok_detail(if usb_portable {
                    "superseded MSIX package removed after USB payload integrity checks"
                } else {
                    "superseded MSIX package removed after portable health check"
                });
                notes.push(
                    if usb_portable {
                        "Superseded MSIX package was removed after the USB program tree passed static integrity checks."
                            .to_string()
                    } else {
                        "Superseded MSIX package was removed after portable fallback passed its health check."
                            .to_string()
                    },
                );
                notes.extend(remove.notes);
            }
            Ok(remove) => {
                let detail = format!(
                    "便携版已可运行，但移除仍注册的 MSIX 失败：{}",
                    remove.message
                );
                outcome.cleanup = StepOutcome::failed(detail.clone());
                outcome.push_warning(format!(
                    "{detail}；开始菜单仍可能启动旧 MSIX，请在 Windows 应用设置中移除 Codex MSIX"
                ));
                notes.extend(remove.notes);
            }
            Err(err) => {
                let detail = format!("便携版已可运行，但无法执行 MSIX 清理：{err}");
                outcome.cleanup = StepOutcome::failed(detail.clone());
                outcome.push_warning(format!(
                    "{detail}；开始菜单仍可能启动旧 MSIX，请在 Windows 应用设置中移除 Codex MSIX"
                ));
            }
        }
    }
    notes.extend(outcome.warnings.iter().cloned());

    let action = if msix_unhealthy {
        WinPerformAction::PortableFallbackAfterMsixUnhealthy
    } else if sideload.is_some() {
        WinPerformAction::PortableFallbackAfterMsixFailure
    } else {
        WinPerformAction::PortableFallback
    };

    let base_message = if sideload.is_some() {
        "Windows 拒绝了 MSIX 部署，已使用同一份已验证 MSIX 完成本地便携安装。"
    } else {
        &portable.message
    };
    let message = portable_install_result_message(base_message, &outcome);
    let report = WinPerformReport {
        success: true,
        action,
        message,
        installed,
        sideload,
        portable: Some(portable),
        msix_health: health,
        fallback_available: true,
        fallback_attempted: true,
        notes,
        stage,
        outcome,
    };
    let action = report.action.as_str();
    let installed_version = report
        .installed
        .as_ref()
        .map(|installed| installed.version.as_str())
        .unwrap_or("none");
    log::info!("Windows perform success action={action} installed_version={installed_version}");
    Ok(report)
}

fn portable_install_result_message(base: &str, outcome: &OperationOutcome) -> String {
    if outcome.provenance.is_failed() {
        return format!("{base}（已安装，但托管记录未写入 — 请「开始管理」重试，勿重复安装）");
    }
    if outcome.cleanup.is_failed() {
        return format!(
            "{base}（便携版已安装，但旧 MSIX 未能移除；开始菜单仍可能启动旧版本，请在 Windows“已安装的应用”中移除 Codex MSIX）"
        );
    }
    base.to_string()
}

fn portable_fallback_needs_msix_cleanup(
    previous_source: Option<&str>,
    sideload_attempted: bool,
) -> bool {
    previous_source == Some("msix") || sideload_attempted
}

/// Whether a completed portable-engine boundary proves the install root was
/// mutated. An upgrade crosses this boundary after moving the old tree; a fresh
/// install has no old tree, so it crosses only after moving the new payload in.
fn portable_boundary_mutated_install_root(boundary: &PortableBoundary) -> bool {
    matches!(
        boundary,
        PortableBoundary::AfterMoveOld {
            had_previous: true,
            ..
        } | PortableBoundary::AfterMoveNew {
            had_previous: false,
            ..
        }
    )
}

fn validate_fixed_msix_dependencies(
    precheck: &codex_win_engine::MsixDependencyPrecheck,
    allow_portable_fallback: bool,
) -> Result<(), AppError> {
    if !allow_portable_fallback && precheck.should_route_portable() {
        return Err(AppError::Engine(format!(
            "离线 MSIX 缺少必要依赖：{}。{}；请补齐匹配的离线依赖组件后重试，尚未修改策略、关闭 Codex 或开始部署",
            precheck.missing_frameworks.join("、"), precheck.reason
        )));
    }
    Ok(())
}

fn appx_recovery_pending_at(backup_root: &Path) -> bool {
    ["recovery.json", "recovery.json.bak"].iter().any(|name| {
        // Absence must be positively known. Permission errors and unexpected
        // filesystem objects cannot mean that data recovery is complete.
        match std::fs::symlink_metadata(backup_root.join(name)) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    })
}

fn usb_appx_recovery_pending() -> bool {
    if !usb_guard::guard_required() {
        return false;
    }
    // v0.5.20+ portable USB installs do not use AppX data recovery. A stale
    // v0.5.x journal must not block launching a fully present, verified program
    // tree after migration away from WpSystem.
    if usb_guard::current_usb_codex_root()
        .as_deref()
        .and_then(detect_portable_install)
        .is_some()
    {
        return false;
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|local| {
            appx_recovery_pending_at(
                &PathBuf::from(local)
                    .join("CodexAppxDataBackups")
                    .join("OpenAI.Codex_2p2nqsd0c76g0"),
            )
        })
        .unwrap_or(true)
}

pub fn win_install_status(settings: &AppSettings) -> WinInstallStatus {
    let store = ProvenanceStore::load();
    let installed = detect_managed_codex(settings, &store);
    let status = match &installed {
        None => "none",
        // Build-aware (matching the macOS path): a self-updated or path-reused
        // install no longer matches its record and reads as "external" so the
        // user is prompted to re-adopt rather than silently treated as managed.
        Some(codex)
            if store.is_managed_build(&codex.path, version_key(&codex.version))
                || is_current_usb_portable(codex) =>
        {
            "managed"
        }
        Some(_) => "external",
    }
    .to_string();
    WinInstallStatus {
        installed,
        status,
        recovery_pending: usb_appx_recovery_pending(),
    }
}

pub fn win_adopt(settings: &AppSettings) -> Result<WinInstallStatus, AppError> {
    let installed = detect_installed_codex(PathBuf::from(&settings.install_root).as_path())
        .ok_or_else(|| AppError::Internal("no Windows Codex detected to adopt".to_string()))?;
    let path = &installed.path;
    log::info!("Windows adopt external install path={path}");
    let mut store = ProvenanceStore::load();
    store.record(
        installed.path.clone(),
        version_key(&installed.version),
        "adopted-external",
    );
    store.save()?;
    Ok(win_install_status(settings))
}

pub fn detect_existing_windows_install_at_path(
    path: &Path,
) -> Result<InstalledWindowsCodex, AppError> {
    if !path.exists() {
        return Err(AppError::Internal(
            "所选位置不存在，请选择已安装的 Codex 文件夹".to_string(),
        ));
    }
    if !path.is_dir() {
        return Err(AppError::Internal(
            "所选位置必须是 Codex 安装文件夹".to_string(),
        ));
    }
    let installed = detect_portable_install(path).ok_or_else(|| {
        AppError::Internal(
            "未在所选位置识别到 Codex 安装：需要 ChatGPT.exe / Codex.exe 入口，且 AppxManifest.xml \
             声明的包身份必须是 OpenAI.Codex（其他产品如 ChatGPT Classic 不受本工具管理）"
                .to_string(),
        )
    })?;
    if installed.version.trim().is_empty() || installed.version == "0.0.0.0" {
        return Err(AppError::Internal(
            "无法读取所选 Codex 的版本信息，请确认这是完整的 Codex 安装目录".to_string(),
        ));
    }
    Ok(installed)
}

pub fn win_adopt_path(settings: &AppSettings, path: &Path) -> Result<WinInstallStatus, AppError> {
    let installed = detect_existing_windows_install_at_path(path)?;
    let install_path = &installed.path;
    log::info!("Windows adopt selected install path={install_path}");
    let mut store = ProvenanceStore::load();
    store.record(
        installed.path.clone(),
        version_key(&installed.version),
        "adopted-external",
    );
    store.save()?;
    Ok(win_install_status(settings))
}

const USB_CODEX_LAUNCH_COMPATIBILITY_SCHEMA: u32 = 1;
const USB_CODEX_LAUNCH_COMPATIBILITY_FILE: &str = "codex-launch-compatibility.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsbCodexLaunchCompatibility {
    schema_version: u32,
    codex_version: String,
    no_sandbox_required: bool,
}

fn usb_launch_compatibility_path() -> Option<PathBuf> {
    crate::app::paths::data_dir().map(|dir| dir.join(USB_CODEX_LAUNCH_COMPATIBILITY_FILE))
}

fn compatibility_requires_no_sandbox(
    state: &UsbCodexLaunchCompatibility,
    codex_version: &str,
) -> bool {
    state.schema_version == USB_CODEX_LAUNCH_COMPATIBILITY_SCHEMA
        && state.no_sandbox_required
        && state.codex_version == codex_version
}

pub(crate) fn usb_no_sandbox_compatibility_known(codex_version: &str) -> bool {
    let Some(path) = usb_launch_compatibility_path() else {
        return false;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    match serde_json::from_slice::<UsbCodexLaunchCompatibility>(&bytes) {
        Ok(state) => compatibility_requires_no_sandbox(&state, codex_version),
        Err(error) => {
            log::warn!(
                "ignoring invalid USB Codex launch compatibility state path={} error={error}",
                path.display()
            );
            false
        }
    }
}

pub(crate) fn remember_usb_no_sandbox_compatibility(codex_version: &str) {
    let Some(path) = usb_launch_compatibility_path() else {
        return;
    };
    let state = UsbCodexLaunchCompatibility {
        schema_version: USB_CODEX_LAUNCH_COMPATIBILITY_SCHEMA,
        codex_version: codex_version.to_string(),
        no_sandbox_required: true,
    };
    let result = serde_json::to_vec_pretty(&state)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            crate::app::atomic_file::write_atomic(&path, &bytes).map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        // The successful launch remains successful. This state is only a
        // performance/security optimization for the next launch.
        log::warn!(
            "could not persist USB Codex launch compatibility path={} error={error}",
            path.display()
        );
    }
}

/// Open the installed Codex (MSIX or portable). Uses the SAME managed-aware
/// detection as status/planning (`detect_managed_codex`) — not raw MSIX-first
/// `detect_installed_codex` — so we launch exactly the build the UI is showing,
/// never a stale MSIX that lingers behind a managed portable install.
/// Fully-qualified engine call to avoid shadowing this function's name.
pub fn launch_codex(settings: &AppSettings, codex_locale: Option<&str>) -> Result<(), AppError> {
    if usb_appx_recovery_pending() {
        return Err(AppError::Engine(
            "上次安装或数据恢复尚未完成，请先在 U 盘安装器中继续完成安装；恢复期间暂不启动 Codex，以免改写待恢复数据".into(),
        ));
    }
    let store = ProvenanceStore::load();
    let installed = detect_managed_codex(settings, &store)
        .ok_or_else(|| AppError::Engine("没有可打开的 Codex".to_string()))?;
    // Bind the selected tree to the signed USB root before applying any
    // startup compatibility changes. The detection fallback may otherwise
    // select a previously registered portable install on the local disk.
    let usb_authorization = authorize_usb_codex_launch(&installed, "win-launch-target")?;
    let usb_mode = usb_authorization.is_some();
    let catalog_reload_marker = if usb_mode {
        // Validate/recover the manager-owned model catalog before either launch
        // path can close an existing window or run a Codex compatibility patch.
        prepare_model_catalog_launch(&installed, true)?
    } else {
        None
    };
    if usb_mode {
        if let Err(error) = crate::app::codex_agents_hint::ensure_portable_usb_hint() {
            log::warn!(
                "could not add the USB Codex mojibake hint to AGENTS.md before launch: {error}"
            );
        }
    }
    if settings.disable_codex_self_updates {
        crate::app::codex_self_update::sync_setting(true)?;
    }
    let path = &installed.path;
    log::info!("Windows launch Codex path={path}");
    let codex_home = if usb_mode {
        Some(usb_guard::local_data_paths()?.codex_home)
    } else {
        None
    };
    let launch_policy = if usb_mode {
        if usb_no_sandbox_compatibility_known(&installed.version) {
            codex_win_engine::PortableLaunchPolicy::UsbCompatibleNoSandbox
        } else {
            codex_win_engine::PortableLaunchPolicy::UsbCompatible
        }
    } else {
        codex_win_engine::PortableLaunchPolicy::Standard
    };
    let direct_entry_required = usb_authorization
        .as_ref()
        .is_some_and(|authorization| authorization.direct_entry_required);
    let launch_environment = codex_launch_environment(usb_mode, codex_home.as_deref());
    let report = codex_win_engine::launch_codex_with_options(
        &installed,
        codex_win_engine::LaunchOptions {
            // A program tree on removable media must never replace itself in
            // place. Scope this to the launched child instead of changing the
            // user's global setting for an unrelated official installation.
            disable_codex_self_updates: settings.disable_codex_self_updates || usb_mode,
            environment: launch_environment,
            usb_integrity_transaction: usb_mode,
            require_usb_entry_overlay: direct_entry_required,
            remote_debugging_port: codex_remote_debugging_port(cfg!(
                feature = "usb-cdp-diagnostic"
            )),
            codex_home,
            locale_override: codex_locale.map(str::to_string),
            portable_launch_policy: launch_policy,
        },
    )
    .map_err(|e| AppError::Engine(e.to_string()))?;
    complete_usb_model_catalog_launch(catalog_reload_marker.as_deref());
    if usb_mode && report.no_sandbox {
        remember_usb_no_sandbox_compatibility(&installed.version);
    }
    log::info!(
        "Windows Codex launch completed mode={} usable_window_confirmed={} no_sandbox={}",
        report.launch_mode,
        report.usable_window_confirmed,
        report.no_sandbox
    );
    Ok(())
}

fn codex_remote_debugging_port(diagnostic_launch: bool) -> Option<u16> {
    diagnostic_launch.then_some(9222)
}

pub fn uninstall_windows_codex(
    settings: &AppSettings,
    confirm: bool,
    purge_user_data: bool,
) -> Result<WinUninstallReport, AppError> {
    // The normal manager deliberately stays conservative: an external Store
    // install must be explicitly adopted before this command can remove it.
    finish_windows_uninstall(
        uninstall_windows_codex_impl(settings, confirm, purge_user_data, false),
        "manager",
        purge_user_data,
    )
}

// `success` retains its public meaning (the primary uninstall succeeded).
// Ancillary failures must still have a distinct final message and log phase.
fn windows_uninstall_phase(report: &WinUninstallReport) -> &'static str {
    if !report.outcome.primary_ok {
        "primary_failed"
    } else if !report.success
        || report.outcome.is_partial()
        || !report.outcome.recovery_actions.is_empty()
    {
        "partial"
    } else {
        "completed"
    }
}

fn finish_windows_uninstall(
    result: Result<WinUninstallReport, AppError>,
    scope: &str,
    purge_user_data: bool,
) -> Result<WinUninstallReport, AppError> {
    match result {
        Ok(mut report) => {
            let phase = windows_uninstall_phase(&report);
            if phase == "partial" {
                report.message = format!(
                    "{}（主卸载已完成，但附属清理或核验未完成；请查看详情，仅重试失败步骤）",
                    report.message
                );
            }
            if phase == "completed" {
                log::info!(
                    "Windows uninstall phase={phase} scope={scope} purge_user_data={purge_user_data} action={} detail={}",
                    report.action, report.message
                );
            } else {
                log::error!(
                    "Windows uninstall phase={phase} scope={scope} purge_user_data={purge_user_data} action={} detail={} notes={:?} outcome={:?}",
                    report.action, report.message, report.notes, report.outcome
                );
            }
            Ok(report)
        }
        Err(error) => {
            // No report is available, so do not assert absence or completed
            // cleanup. Preserve the actual error propagated by the failing step.
            log::error!(
                "Windows uninstall phase=primary_failed scope={scope} purge_user_data={purge_user_data} detail={error}"
            );
            Err(error)
        }
    }
}

fn fail_uninstall_postcondition(report: &mut WinUninstallReport, app_state: &str, detail: String) {
    report.success = false;
    report.outcome.primary_ok = false;
    report.outcome.app_state = app_state.to_string();
    report.outcome.install_class = None;
    // Do not leave a success sentence as the final message after a later
    // current-user query disproves it. The earlier result remains diagnostic.
    report.message = format!("卸载未完成：{detail}；前一步结果：{}", report.message);
    report.notes.push(detail.clone());
    report.outcome.cleanup = StepOutcome::failed(detail.clone());
    report.outcome.push_warning(detail);
    report.outcome.push_recovery(recovery::CLEANUP_METADATA);
}

fn external_msix_uninstall_note(removed: bool) -> String {
    let result = if removed {
        "已移除当前用户的 OpenAI.Codex AppX 注册"
    } else {
        "已尝试移除当前用户的 OpenAI.Codex AppX 注册，但移除未完成"
    };
    format!("USB 一键卸载按用户确认{result}；本机没有对应的托管记录，无法证明它来自本安装器。")
}

fn merge_extra_portable_uninstall(
    report: &mut WinUninstallReport,
    root: &Path,
    extra: PortableUninstallReport,
    purge_user_data: bool,
) {
    if extra.success {
        report.notes.push(if extra.removed_files {
            format!("已移除旧版 Codex 便携程序目录：{}", root.display())
        } else {
            format!("旧版 Codex 便携程序目录已不存在：{}", root.display())
        });
        if purge_user_data {
            report.purged_user_data |= extra.purged_user_data;
        }
    }
    if !extra.success || extra.partial {
        let detail = format!(
            "旧版 Codex 便携目录或附属清理未完成：{}；{}",
            root.display(),
            extra.message
        );
        if !extra.success && !extra.removed_files {
            fail_uninstall_postcondition(report, "present", detail);
        } else {
            report.notes.push(detail.clone());
            report.outcome.cleanup = StepOutcome::failed(detail);
            report.outcome.push_recovery(recovery::CLEANUP_METADATA);
        }
    }
    report.notes.extend(extra.notes);
}

/// USB bundle cleanup is an explicit, user-confirmed "remove Codex + CCSwitch"
/// action.  The first USB installer versions could fail before writing a
/// provenance record, and an official Store install has the same
/// `OpenAI.Codex` identity as our signed MSIX.  In this narrow bundle path we
/// therefore allow removal of the current user's external MSIX registration,
/// while keeping the normal manager uninstall conservative.
pub fn uninstall_windows_codex_for_usb_bundle(
    settings: &AppSettings,
    confirm: bool,
    purge_user_data: bool,
) -> Result<WinUninstallReport, AppError> {
    finish_windows_uninstall(
        uninstall_windows_codex_for_usb_bundle_impl(settings, confirm, purge_user_data),
        "usb_bundle_codex_primary",
        purge_user_data,
    )
}

fn uninstall_windows_codex_for_usb_bundle_impl(
    settings: &AppSettings,
    confirm: bool,
    purge_user_data: bool,
) -> Result<WinUninstallReport, AppError> {
    // Snapshot managed portable roots before the primary uninstall clears its
    // provenance record.  Older builds could leave a portable fallback beside
    // a later MSIX registration; the USB button is the explicit, combined
    // cleanup path and therefore removes both variants in one run.
    let managed_portable_roots = managed_portable_roots_for_bundle(settings);
    let mut report = uninstall_windows_codex_impl(settings, confirm, purge_user_data, true)?;

    for root in managed_portable_roots {
        let primary_path = report
            .installed_before
            .as_ref()
            .map(|installed| installed.path.as_str());
        if primary_path.is_some_and(|path| same_windows_path_text(path, &root)) {
            continue;
        }
        let Some(installed) = detect_portable_install(&root) else {
            continue;
        };
        if installed.source != "portable" {
            continue;
        }
        match uninstall_portable(&root, purge_user_data) {
            Ok(extra) => {
                merge_extra_portable_uninstall(&mut report, &root, extra, purge_user_data);
            }
            Err(error) => {
                let detail = format!("清理旧版 Codex 便携目录失败：{}；{error}", root.display());
                fail_uninstall_postcondition(&mut report, "unknown", detail);
            }
        }
    }

    // If the primary detection chose a managed portable build, an older MSIX
    // can still be registered for the current user.  Remove it explicitly so
    // the next install cannot be redirected back to C: by stale AppX state.
    let primary_was_msix = report
        .installed_before
        .as_ref()
        .is_some_and(|installed| installed.source == "msix");
    if !primary_was_msix {
        if let Some(msix) = detect_installed_codex(PathBuf::from(&settings.install_root).as_path())
            .filter(|installed| installed.source == "msix")
        {
            let close_result = close_codex_gracefully_for_root(30, Path::new(&msix.path));
            let removal = close_result
                .map_err(engine_err)
                .and_then(|_| remove_msix_with_authorized_data_volume(!purge_user_data));
            match removal {
                Ok(extra) if extra.success => {
                    report
                        .notes
                        .push("已清理残留的 Codex AppX 注册。".to_string());
                    report.notes.extend(extra.notes.clone());
                    report.msix = Some(extra);
                }
                Ok(extra) => {
                    let detail = format!("残留 Codex AppX 未确认移除：{}", extra.message);
                    fail_uninstall_postcondition(&mut report, "unknown", detail);
                    report.notes.extend(extra.notes.clone());
                    if let Some(raw_error) = &extra.raw_error {
                        report.notes.push(raw_error.clone());
                    }
                    report.msix = Some(extra);
                }
                Err(error) => {
                    let detail = format!("清理残留 Codex AppX 失败：{error}");
                    fail_uninstall_postcondition(&mut report, "unknown", detail);
                }
            }
        }
    }

    // A portable install creates metadata that an MSIX install does not own.
    // The combined USB action is deliberately the one place that cleans these
    // known Codex records even when the package itself was already absent.
    match cleanup_portable_metadata(false) {
        Ok(metadata) => {
            report.notes.extend(metadata.notes);
            if metadata.partial {
                report.success = false;
                report.outcome.cleanup = StepOutcome::failed(metadata.message.clone());
                report.outcome.push_recovery(recovery::CLEANUP_METADATA);
            }
        }
        Err(error) => {
            report.success = false;
            let detail = format!("清理 Codex 快捷方式/卸载项失败：{error}");
            report.notes.push(detail.clone());
            report.outcome.cleanup = StepOutcome::failed(detail);
            report.outcome.push_recovery(recovery::CLEANUP_METADATA);
        }
    }

    // An already-absent package still permits explicit data purge, but a
    // reported primary removal failure must not erase the still-used profile.
    if purge_user_data && report.success {
        let mut purge_notes = Vec::new();
        match purge_codex_user_data(&mut purge_notes) {
            Ok(purged) => {
                report.purged_user_data |= purged;
                report.notes.extend(purge_notes);
            }
            Err(error) => {
                report.success = false;
                let detail = format!("清理 Codex 用户数据失败：{error}");
                report.notes.push(detail.clone());
                report.outcome.cleanup = StepOutcome::failed(detail);
                report.outcome.push_recovery(recovery::PURGE_USER_DATA);
            }
        }
    }

    // Final current-user postcondition.  Do not report success if either an
    // AppX registration or a recognizable managed portable tree remains.
    let registration = codex_win_engine::registered_msix_package_full_name();
    let registration_state = if registration.is_err() {
        "unknown"
    } else {
        "present"
    };
    if let Err(error) = confirm_no_current_user_msix(registration) {
        fail_uninstall_postcondition(&mut report, registration_state, error.to_string());
    }
    if detect_installed_codex(PathBuf::from(&settings.install_root).as_path()).is_some() {
        let detail = "卸载后仍检测到 Codex 安装；请关闭相关进程后重试".to_string();
        fail_uninstall_postcondition(&mut report, "present", detail);
    }

    if report.success && report.message.contains("not installed") {
        report.message = "Windows Codex 未安装；已完成残留元数据与数据检查。".to_string();
    }
    Ok(report)
}

fn same_windows_path_text(left: &str, right: &Path) -> bool {
    let normalize = |value: &str| {
        value
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    normalize(left) == normalize(&right.to_string_lossy())
}

// Optional install detection deliberately tolerates unavailable probes for UI
// status, but an uninstall postcondition cannot equate that with absence.
fn confirm_no_current_user_msix(
    probe: Result<Option<String>, EngineError>,
) -> Result<(), AppError> {
    match probe {
        Ok(None) => Ok(()),
        Ok(Some(package)) => Err(AppError::Engine(format!(
            "当前用户仍有 Codex AppX 注册：{package}；未确认卸载完成"
        ))),
        Err(error) => Err(AppError::Engine(format!(
            "无法核验当前用户的 Codex AppX 注册状态，不能当作已卸载：{error}"
        ))),
    }
}

fn remove_msix_with_authorized_data_volume(
    preserve_data: bool,
) -> Result<MsixRemoveReport, AppError> {
    let Some(root) = usb_guard::current_bundle_volume_root() else {
        return codex_win_engine::remove_msix_package_with_options(preserve_data)
            .map_err(engine_err);
    };
    // Do not derive backup authority from the AC link or an installation path.
    // Both this lease and the live GUID must name the present product USB.
    usb_guard::verify_authorized_volume_root(&root)?;
    let guid = appx_volume::filesystem_volume_guid(&root)?;
    usb_guard::verify_authorized_volume_root(&root)?;
    codex_win_engine::remove_msix_package_with_volume(preserve_data, Some(&root), Some(&guid))
        .map_err(engine_err)
}

/// Read-only all-user/provisioned/path classification used after the ordinary
/// current-user uninstall. Failure is not treated as absence by the combined
/// USB command.
pub fn probe_windows_codex_machine_state() -> Result<MsixMachineStateReport, AppError> {
    codex_win_engine::probe_msix_machine_state().map_err(engine_err)
}

/// Retire one exact machine-level orphan through supported AppX APIs. The
/// caller supplies a FullName derived from the Authenticode-verified USB MSIX;
/// the bound-volume lease is independently revalidated here before package
/// data links on that volume can be backed up.
pub fn cleanup_windows_codex_machine_orphan(
    expected_full_name: &str,
    authorized_package_path: &Path,
    authorized_package_sha256: &str,
    preserve_data: bool,
) -> Result<MsixRemoveReport, AppError> {
    let root = usb_guard::current_bundle_volume_root().ok_or_else(|| {
        AppError::Engine(
            "无法确认当前产品 U 盘，未授权机器级 AppX 孤儿清理；请保持 U 盘连接后重试".to_string(),
        )
    })?;
    usb_guard::verify_authorized_volume_root(&root)?;
    let guid = appx_volume::filesystem_volume_guid(&root)?;
    usb_guard::verify_authorized_volume_root(&root)?;
    codex_win_engine::remove_exact_msix_machine_orphan_with_volume(
        expected_full_name,
        authorized_package_path,
        authorized_package_sha256,
        preserve_data,
        Some(&root),
        Some(&guid),
    )
    .map_err(engine_err)
}

fn managed_portable_roots_for_bundle(settings: &AppSettings) -> Vec<PathBuf> {
    let store = ProvenanceStore::load();
    let mut roots = store
        .managed
        .iter()
        .filter_map(|record| {
            let path = PathBuf::from(record.path.trim());
            detect_portable_install(&path)
                .filter(|installed| installed.source == "portable")
                .map(|_| path)
        })
        .collect::<Vec<_>>();
    // The persisted install root is included only if its provenance record is
    // present; an arbitrary external portable directory must remain outside
    // the combined cleanup scope.
    let configured = PathBuf::from(settings.install_root.trim());
    if store.is_managed(&configured.to_string_lossy())
        && detect_portable_install(&configured)
            .is_some_and(|installed| installed.source == "portable")
    {
        roots.push(configured);
    }
    roots.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    roots.dedup_by(|left, right| same_windows_path_text(&left.to_string_lossy(), right));
    roots
}

fn uninstall_windows_codex_impl(
    settings: &AppSettings,
    confirm: bool,
    purge_user_data: bool,
    allow_external_msix: bool,
) -> Result<WinUninstallReport, AppError> {
    log::info!(
        "Windows uninstall start purge_user_data={purge_user_data} allow_external_msix={allow_external_msix}"
    );
    if !confirm {
        return Err(AppError::Internal(
            "explicit confirmation is required before uninstalling Windows Codex".to_string(),
        ));
    }

    let mut store = ProvenanceStore::load();
    let installed = detect_managed_codex(settings, &store).or_else(|| {
        // Only broaden the USB bundle path to an external MSIX.  Do not use
        // this escape hatch for an arbitrary external portable directory.
        if allow_external_msix {
            detect_installed_codex(PathBuf::from(&settings.install_root).as_path())
                .filter(|candidate| candidate.source == "msix")
        } else {
            None
        }
    });
    let Some(installed_before) = installed else {
        confirm_no_current_user_msix(codex_win_engine::registered_msix_package_full_name())?;
        let mut notes = Vec::new();
        let mut outcome = OperationOutcome::full_success("absent", Some("none"));
        match appx_volume::restore_system_app_data_policy() {
            Ok(Some(policy)) => notes.push(policy.message),
            Ok(None) => {}
            Err(error) => {
                let detail = format!("恢复 AppX 用户数据策略失败：{error}");
                notes.push(detail.clone());
                outcome.cleanup = StepOutcome::failed(detail.clone());
                outcome.push_warning(detail);
                outcome.push_recovery(recovery::CLEANUP_METADATA);
            }
        }
        return Ok(WinUninstallReport {
            success: true,
            action: "none".to_string(),
            message: "Windows Codex is not installed.".to_string(),
            installed_before: None,
            msix: None,
            portable: None,
            purged_user_data: false,
            notes,
            outcome,
        });
    };

    // Keep exact-build provenance for ordinary installs. A confirmed USB
    // uninstall also accepts this drive's portable tree after LIVE signed
    // authorization: another computer has no local provenance for that tree.
    // The existing current-user MSIX exception remains unchanged.
    let managed_build = store.is_managed_build(
        &installed_before.path,
        version_key(&installed_before.version),
    );
    let current_usb_portable = !managed_build
        && authorize_portable_bundle_uninstall(&installed_before, allow_external_msix, || {
            if !is_current_usb_portable(&installed_before) {
                return Ok(None);
            }
            Ok(
                usb_guard::require_signed_codex_bundle("codex-uninstall-destination")?
                    .map(|authorization| authorization.codex_root),
            )
        })?;
    if !(managed_build
        || current_usb_portable
        || allow_external_msix && installed_before.source == "msix")
    {
        log::warn!("Windows uninstall rejected external install");
        return Ok(WinUninstallReport {
            success: false,
            action: "external-not-managed".to_string(),
            message:
                "Detected Windows Codex is external. Adopt it before uninstalling via manager."
                    .to_string(),
            installed_before: Some(installed_before),
            msix: None,
            portable: None,
            purged_user_data: false,
            notes: vec!["No files or packages were removed.".to_string()],
            outcome: OperationOutcome::primary_failed(
                "present",
                "external install — refuse to uninstall",
            ),
        });
    }
    if current_usb_portable {
        log::info!(
            "USB bundle uninstall authorized current portable without local provenance path={} version={}",
            installed_before.path,
            installed_before.version
        );
    } else if !managed_build {
        log::warn!(
            "USB bundle uninstall explicitly removing current-user external MSIX path={} version={}",
            installed_before.path,
            installed_before.version
        );
    }

    if installed_before.source == "msix" {
        close_codex_gracefully_for_root(30, PathBuf::from(&installed_before.path).as_path())
            .map_err(engine_err)?;
        let msix = remove_msix_with_authorized_data_volume(!purge_user_data)?;
        // Backup locations and native diagnostics also matter on failure.
        let mut notes = msix.notes.clone();
        if let Some(raw_error) = msix.raw_error.as_ref().filter(|raw| **raw != msix.message) {
            notes.push(raw_error.clone());
        }
        if !managed_build {
            notes.push(external_msix_uninstall_note(msix.success));
        }
        let mut purged_user_data = false;
        let mut outcome = if msix.success {
            OperationOutcome::full_success("absent", Some("none"))
        } else {
            OperationOutcome::primary_failed("present", msix.message.clone())
        };
        if msix.success {
            store.remove(&installed_before.path);
            if let Err(e) = store.save() {
                let detail = format!("托管记录清除失败（{e}）");
                outcome.provenance = StepOutcome::failed(detail.clone());
                outcome.push_warning(detail);
                outcome.push_recovery(recovery::CLEAR_PROVENANCE);
                outcome.path = Some(installed_before.path.clone());
            }
            // A previous portable fallback may have left a Start Menu link or
            // an Apps & Features entry behind even though the current install
            // is now MSIX.  Remove those manager-owned metadata records on the
            // MSIX path too; otherwise a reinstall can appear to be an old
            // C:-drive installation.  This cleanup never removes user data.
            match cleanup_portable_metadata(false) {
                Ok(metadata) => {
                    notes.extend(metadata.notes.clone());
                    if metadata.partial {
                        let detail = metadata.message.clone();
                        outcome.cleanup = StepOutcome::failed(detail.clone());
                        outcome.push_warning(detail);
                        outcome.push_recovery(recovery::CLEANUP_METADATA);
                    }
                }
                Err(error) => {
                    let detail = format!("清理 Codex 快捷方式/卸载项失败：{error}");
                    notes.push(detail.clone());
                    outcome.cleanup = StepOutcome::failed(detail.clone());
                    outcome.push_warning(detail);
                    outcome.push_recovery(recovery::CLEANUP_METADATA);
                }
            }
            // Honor the user's "don't keep my data" choice on the MSIX path too,
            // exactly like the portable path — remove ~/.codex when asked.
            if purge_user_data {
                match purge_codex_user_data(&mut notes) {
                    Ok(purged) => {
                        purged_user_data = purged;
                        if purged {
                            notes.push("User data was removed.".to_string());
                        }
                        if !outcome.cleanup.is_failed() {
                            outcome.cleanup = StepOutcome::ok();
                        }
                    }
                    Err(err) => {
                        let detail = format!("user data purge failed: {err}");
                        notes.push(detail.clone());
                        outcome.cleanup = StepOutcome::failed(detail);
                        outcome.push_recovery(recovery::PURGE_USER_DATA);
                    }
                }
            } else {
                notes.push(
                    "未清除独立的 .codex 用户目录；AppX 包内数据的保留或备份结果见卸载详细记录。"
                        .to_string(),
                );
                if !outcome.cleanup.is_failed() {
                    outcome.cleanup = StepOutcome::skipped("user data preserved");
                }
            }
            match appx_volume::restore_system_app_data_policy() {
                Ok(Some(policy)) => notes.push(policy.message),
                Ok(None) => {}
                Err(error) => {
                    let detail = format!("恢复 AppX 用户数据策略失败：{error}");
                    notes.push(detail.clone());
                    outcome.cleanup = StepOutcome::failed(detail.clone());
                    outcome.push_warning(detail);
                    outcome.push_recovery(recovery::CLEANUP_METADATA);
                }
            }
        }
        notes.extend(outcome.warnings.iter().cloned());
        let report = WinUninstallReport {
            success: msix.success,
            action: "remove-msix".to_string(),
            message: if msix.success && !managed_build {
                format!("{}（已清理当前用户的外部 Codex AppX 注册）", msix.message)
            } else {
                msix.message.clone()
            },
            installed_before: Some(installed_before),
            msix: Some(msix),
            portable: None,
            purged_user_data,
            notes,
            outcome,
        };
        return Ok(report);
    }

    let portable = uninstall_portable(
        PathBuf::from(&installed_before.path).as_path(),
        purge_user_data,
    )
    .map_err(engine_err)?;
    let mut provenance = StepOutcome::ok();
    if portable.success {
        store.remove(&installed_before.path);
        if let Err(e) = store.save() {
            provenance = StepOutcome::failed(format!("托管记录清除失败（{e}）"));
        }
    }
    let mut outcome =
        outcome_from_portable_uninstall(&portable, provenance, &installed_before.path);
    let mut notes = portable.notes.clone();
    notes.extend(outcome.warnings.iter().cloned());
    if portable.success {
        match appx_volume::restore_system_app_data_policy() {
            Ok(Some(policy)) => notes.push(policy.message),
            Ok(None) => {}
            Err(error) => {
                let detail = format!("恢复 AppX 用户数据策略失败：{error}");
                notes.push(detail.clone());
                outcome.cleanup = StepOutcome::failed(detail.clone());
                outcome.push_warning(detail);
                outcome.push_recovery(recovery::CLEANUP_METADATA);
            }
        }
    }
    let report = WinUninstallReport {
        success: portable.success,
        action: "remove-portable".to_string(),
        message: portable.message.clone(),
        installed_before: Some(installed_before),
        msix: None,
        purged_user_data: portable.purged_user_data,
        notes,
        portable: Some(portable),
        outcome,
    };
    Ok(report)
}

/// Retry only failed ancillary steps after a Windows install/uninstall.
/// Never re-runs full install or package removal.
pub fn retry_windows_ancillary(
    settings: &AppSettings,
    actions: &[String],
    path: Option<&str>,
    purge_user_data: bool,
) -> Result<AncillaryRetryReport, AppError> {
    retry_windows_ancillary_with_detector(actions, path, purge_user_data, || {
        win_install_status(settings)
    })
}

fn retry_windows_ancillary_with_detector<F>(
    actions: &[String],
    path: Option<&str>,
    purge_user_data: bool,
    detect_status: F,
) -> Result<AncillaryRetryReport, AppError>
where
    F: FnOnce() -> WinInstallStatus,
{
    let mut outcome = OperationOutcome {
        primary_ok: true,
        app_state: "unknown".to_string(),
        install_class: None,
        path: path.map(str::to_string),
        provenance: StepOutcome::not_applicable(),
        cleanup: StepOutcome::not_applicable(),
        warnings: Vec::new(),
        recovery_actions: Vec::new(),
    };
    let mut messages = Vec::new();

    if actions.iter().any(|a| a == recovery::RECORD_PROVENANCE) {
        let status = detect_status();
        match status.installed {
            Some(installed) => {
                let recorded =
                    record_managed_install(None, &installed, "manager-installed-recovery");
                outcome.provenance = recorded.provenance.clone();
                outcome.app_state = recorded.app_state;
                outcome.install_class = recorded.install_class;
                if recorded.provenance.is_failed() {
                    outcome.push_recovery(recovery::RECORD_PROVENANCE);
                    messages.push(
                        recorded
                            .warnings
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "托管记录写入失败".to_string()),
                    );
                } else {
                    messages.push("托管记录已重新写入".to_string());
                }
            }
            None => {
                outcome.provenance = StepOutcome::failed("未检测到可记录的 Codex 安装");
                outcome.app_state = "absent".to_string();
                messages.push("未检测到 Codex，无法写入托管记录".to_string());
            }
        }
        // A failed record attempt is always retryable. In particular, the
        // no-install-detected branch must not strand the UI without its CTA.
        outcome.retain_failed_provenance_recovery(recovery::RECORD_PROVENANCE);
    }

    if actions.iter().any(|a| a == recovery::CLEAR_PROVENANCE) {
        let mut store = ProvenanceStore::load();
        if let Some(path) = path {
            store.remove(path);
        } else {
            store.managed.retain(|r| Path::new(&r.path).exists());
        }
        match store.save() {
            Ok(()) => {
                outcome.provenance = StepOutcome::ok();
                messages.push("陈旧托管记录已清除".to_string());
            }
            Err(e) => {
                outcome.provenance = StepOutcome::failed(e.to_string());
                outcome.push_recovery(recovery::CLEAR_PROVENANCE);
                messages.push(format!("清除托管记录仍失败: {e}"));
            }
        }
    }

    if actions.iter().any(|a| a == recovery::CLEANUP_METADATA) {
        match cleanup_portable_metadata(false).map_err(engine_err) {
            Ok(report) => {
                if report.partial {
                    outcome.cleanup = StepOutcome::failed(report.message.clone());
                    outcome.push_recovery(recovery::CLEANUP_METADATA);
                } else {
                    outcome.cleanup = StepOutcome::ok();
                }
                messages.push(report.message);
                outcome.warnings.extend(report.notes);
            }
            Err(err) => {
                outcome.cleanup = StepOutcome::failed(err.to_string());
                outcome.push_recovery(recovery::CLEANUP_METADATA);
                messages.push(format!("元数据清理失败: {err}"));
            }
        }
        // The USB AppX-volume install also owns a machine policy journal. A
        // previous uninstall may have removed the package but failed while
        // restoring that policy; retry the same ancillary action here so the
        // manager cannot silently leave a system-wide setting behind.
        match appx_volume::restore_system_app_data_policy() {
            Ok(Some(policy)) => messages.push(policy.message),
            Ok(None) => {}
            Err(error) => {
                let detail = format!("恢复 AppX 用户数据策略仍失败：{error}");
                outcome.cleanup = StepOutcome::failed(detail.clone());
                outcome.push_recovery(recovery::CLEANUP_METADATA);
                messages.push(detail);
            }
        }
    }

    if actions.iter().any(|a| a == recovery::PURGE_USER_DATA) && purge_user_data {
        let mut notes = Vec::new();
        match purge_codex_user_data(&mut notes).map_err(engine_err) {
            Ok(_) => {
                outcome.cleanup = StepOutcome::ok_detail("user data purged");
                messages.push("用户数据已清除".to_string());
            }
            Err(err) => {
                outcome.cleanup = StepOutcome::failed(err.to_string());
                outcome.push_recovery(recovery::PURGE_USER_DATA);
                messages.push(format!("清除用户数据仍失败: {err}"));
            }
        }
        outcome.warnings.extend(notes);
    }

    if messages.is_empty() {
        return Err(AppError::Internal(
            "没有可执行的恢复步骤（请传入 record_provenance / clear_provenance / cleanup_metadata / purge_user_data）"
                .to_string(),
        ));
    }

    Ok(AncillaryRetryReport {
        message: messages.join("; "),
        outcome,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_debugging_is_only_enabled_for_an_explicit_diagnostic_codex_launch() {
        assert_eq!(super::codex_remote_debugging_port(false), None);
        assert_eq!(super::codex_remote_debugging_port(true), Some(9222));
    }

    #[test]
    fn usb_runtime_environment_prefers_managed_bin_and_enables_utf8_python() {
        let root = temp_test_dir("usb-runtime-environment");
        let codex_home = root.join("codex-home");
        let runtime_bin = codex_home.join("codex-agent-kit").join("bin");
        std::fs::create_dir_all(&runtime_bin).unwrap();
        let original_path = std::env::var_os("PATH").unwrap_or_default();

        let environment = super::usb_runtime_environment(&codex_home);
        let configured_path = environment
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value)
            .expect("USB launch should include managed runtime PATH");
        let path_entries =
            std::env::split_paths(std::ffi::OsStr::new(configured_path)).collect::<Vec<_>>();
        assert_eq!(path_entries.first(), Some(&runtime_bin));
        assert_eq!(
            &path_entries[1..],
            std::env::split_paths(&original_path).collect::<Vec<_>>()
        );
        assert!(environment
            .iter()
            .any(|(name, value)| { name == "PYTHONUTF8" && value == "1" }));
        assert!(environment
            .iter()
            .any(|(name, value)| { name == "PYTHONIOENCODING" && value == "utf-8" }));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_codex_launch_environment_keeps_usb_runtime_and_media_overrides_together() {
        let root = temp_test_dir("shared-codex-launch-environment");
        let codex_home = root.join("codex-home");
        let runtime_bin = codex_home.join("codex-agent-kit").join("bin");
        std::fs::create_dir_all(&runtime_bin).unwrap();

        let environment = super::codex_launch_environment(true, Some(&codex_home));
        let path = environment
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value)
            .expect("USB launch should include the managed runtime PATH");
        assert_eq!(
            std::env::split_paths(std::ffi::OsStr::new(path)).next(),
            Some(runtime_bin)
        );
        assert!(environment
            .iter()
            .any(|(name, value)| name == "PYTHONUTF8" && value == "1"));
        assert!(environment
            .iter()
            .any(|(name, value)| name == "PYTHONIOENCODING" && value == "utf-8"));
        for (name, value) in crate::app::media_generation::usb_launch_environment() {
            assert!(environment.iter().any(|(actual_name, actual_value)| {
                actual_name == &name && actual_value == &value
            }));
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_codex_launch_environment_omits_managed_path_when_runtime_is_missing() {
        let root = temp_test_dir("shared-codex-launch-environment-no-runtime");
        let codex_home = root.join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();

        let environment = super::codex_launch_environment(true, Some(&codex_home));
        assert!(!environment
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("PATH")));
        assert!(environment
            .iter()
            .any(|(name, value)| name == "PYTHONUTF8" && value == "1"));
        assert!(environment
            .iter()
            .any(|(name, value)| name == "PYTHONIOENCODING" && value == "utf-8"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_codex_launch_environment_does_not_change_non_usb_launches() {
        assert!(super::codex_launch_environment(false, None).is_empty());
    }

    #[test]
    fn model_catalog_reload_skips_process_inspection_when_no_marker_is_pending() {
        let inspected = std::cell::Cell::new(false);
        let closed = std::cell::Cell::new(false);
        super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            false,
            |_| {
                inspected.set(true);
                Ok(true)
            },
            |_| {
                closed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(!inspected.get());
        assert!(!closed.get());
    }

    #[test]
    fn model_catalog_reload_closes_only_a_running_managed_root() {
        let closed_roots = std::cell::RefCell::new(Vec::new());
        let checks = std::cell::Cell::new(0);
        super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            true,
            |_| {
                let count = checks.get();
                checks.set(count + 1);
                Ok(count == 0)
            },
            |root| {
                closed_roots
                    .borrow_mut()
                    .push(root.to_string_lossy().into_owned());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(&*closed_roots.borrow(), &[r"X:\Codex"]);

        let closed = std::cell::Cell::new(false);
        super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            true,
            |_| Ok(false),
            |_| {
                closed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(!closed.get());
    }

    #[test]
    fn model_catalog_reload_fails_closed_when_process_state_or_close_is_unknown() {
        let inspect_error = super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            true,
            |_| Err("process query failed".to_string()),
            |_| Ok(()),
        )
        .unwrap_err()
        .to_string();
        assert!(inspect_error.contains("无法确认 Codex 是否仍在运行"));
        assert!(inspect_error.contains("process query failed"));

        let close_error = super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            true,
            |_| Ok(true),
            |_| Err("graceful close timed out".to_string()),
        )
        .unwrap_err()
        .to_string();
        assert!(close_error.contains("关闭正在运行的 Codex 以加载新模型失败"));
        assert!(close_error.contains("graceful close timed out"));
    }

    #[test]
    fn model_catalog_reload_keeps_pending_marker_when_close_does_not_complete() {
        let marker_pending = std::cell::Cell::new(true);
        let result = super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            marker_pending.get(),
            |_| Ok(true),
            |_| Err("graceful close failed".to_string()),
        );
        if result.is_ok() {
            super::complete_usb_model_catalog_launch(Some(b"pending"));
            marker_pending.set(false);
        }
        assert!(result.is_err());
        assert!(marker_pending.get());
    }

    #[test]
    fn model_catalog_reload_requires_old_process_to_be_confirmed_exited() {
        let checks = std::cell::Cell::new(0);
        let error = super::restart_codex_for_catalog_reload(
            std::path::Path::new(r"X:\Codex"),
            true,
            |_| {
                checks.set(checks.get() + 1);
                Ok(true)
            },
            |_| Ok(()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("仍检测到旧进程"));
        assert_eq!(checks.get(), 2);
    }

    fn policy_msix_fixture(drive: &str, full_name: &str) -> InstalledWindowsCodex {
        InstalledWindowsCodex {
            path: format!(r"{drive}\WindowsApps\{full_name}"),
            version: "1.0.0.0".into(),
            arch: None,
            source: "msix".into(),
            package_family_name: Some("OpenAI.Codex_2p2nqsd0c76g0".into()),
            package_volume: Some(format!(r"{drive}\")),
            installed_at: None,
        }
    }

    #[test]
    fn no_sandbox_compatibility_is_scoped_to_the_exact_codex_version() {
        let state = super::UsbCodexLaunchCompatibility {
            schema_version: super::USB_CODEX_LAUNCH_COMPATIBILITY_SCHEMA,
            codex_version: "26.825.5331.0".to_string(),
            no_sandbox_required: true,
        };
        assert!(super::compatibility_requires_no_sandbox(
            &state,
            "26.825.5331.0"
        ));
        assert!(!super::compatibility_requires_no_sandbox(
            &state,
            "26.900.1.0"
        ));
        let disabled = super::UsbCodexLaunchCompatibility {
            no_sandbox_required: false,
            ..state
        };
        assert!(!super::compatibility_requires_no_sandbox(
            &disabled,
            "26.825.5331.0"
        ));
    }

    #[test]
    fn interrupted_deployment_retains_policy_even_before_late_registration() {
        for registration in [
            Ok(None),
            Err("query timed out".into()),
            Ok(Some("target".into())),
        ] {
            let result = super::classify_failed_msix_policy(
                "target",
                Some(std::path::Path::new(r"D:\")),
                true,
                registration,
                None,
            );
            assert!(result.keep_usb_data_policy);
            assert!(result.target_landed_or_unknown);
        }
    }

    #[test]
    fn completed_failure_with_confirmed_absence_can_restore_policy() {
        let result = super::classify_failed_msix_policy(
            "target",
            Some(std::path::Path::new(r"D:\")),
            false,
            Ok(None),
            None,
        );
        assert!(!result.keep_usb_data_policy);
        assert!(!result.target_landed_or_unknown);
    }

    #[test]
    fn usb_package_and_unavailable_detection_retain_data_policy() {
        let installed = policy_msix_fixture("D:", "target");
        for candidate in [Some(&installed), None] {
            let result = super::classify_failed_msix_policy(
                "target",
                Some(std::path::Path::new(r"D:\")),
                false,
                Ok(Some("target".into())),
                candidate,
            );
            assert!(result.keep_usb_data_policy);
            assert!(result.target_landed_or_unknown);
        }
    }

    #[test]
    fn older_usb_package_retains_data_policy_without_committing_new_version() {
        let installed = policy_msix_fixture("D:", "older");
        let result = super::classify_failed_msix_policy(
            "target",
            Some(std::path::Path::new(r"D:\")),
            false,
            Ok(Some("older".into())),
            Some(&installed),
        );
        assert!(result.keep_usb_data_policy);
        assert!(!result.target_landed_or_unknown);
    }

    #[test]
    fn coherent_other_volume_is_not_a_landed_usb_target() {
        let installed = policy_msix_fixture("C:", "target");
        let result = super::classify_failed_msix_policy(
            "target",
            Some(std::path::Path::new(r"D:\")),
            false,
            Ok(Some("target".into())),
            Some(&installed),
        );
        assert!(!result.keep_usb_data_policy);
        assert!(!result.target_landed_or_unknown);
    }

    #[test]
    fn mismatched_snapshot_or_identity_must_not_authorize_policy_restoration() {
        let mut installed = policy_msix_fixture("C:", "different");
        for mode in 0..3 {
            if mode == 1 {
                installed = policy_msix_fixture("C:", "target");
                installed.package_family_name = Some("untrusted".into());
            }
            if mode == 2 {
                installed = policy_msix_fixture("C:", "target");
                installed.source = "portable".into();
            }
            let result = super::classify_failed_msix_policy(
                "target",
                Some(std::path::Path::new(r"D:\")),
                false,
                Ok(Some("target".into())),
                Some(&installed),
            );
            assert!(result.keep_usb_data_policy, "mode {mode}");
        }
    }

    #[test]
    fn unavailable_registration_is_unknown_not_absent() {
        let result = super::classify_failed_msix_policy(
            "target",
            Some(std::path::Path::new(r"D:\")),
            false,
            Err("Access is denied".into()),
            None,
        );
        assert!(result.keep_usb_data_policy);
        assert!(result.target_landed_or_unknown);
    }

    #[test]
    fn uninstall_does_not_confuse_query_failure_with_absence() {
        assert!(super::confirm_no_current_user_msix(Ok(None)).is_ok());
        assert!(
            super::confirm_no_current_user_msix(Ok(Some("OpenAI.Codex_fixture".into()))).is_err()
        );
        assert!(
            super::confirm_no_current_user_msix(Err(codex_win_engine::EngineError::Io(
                "Get-AppxPackage Access is denied".into()
            )))
            .is_err()
        );
    }

    fn uninstall_result_fixture(primary_ok: bool) -> super::WinUninstallReport {
        let message = if primary_ok {
            "Remove-AppxPackage succeeded"
        } else {
            "Cannot safely preserve linked package data: AC; removal was not started"
        };
        super::WinUninstallReport {
            success: primary_ok,
            action: "remove-msix".into(),
            message: message.into(),
            installed_before: None,
            msix: None,
            portable: None,
            purged_user_data: false,
            notes: vec!["original diagnostic / backup path".into()],
            outcome: if primary_ok {
                OperationOutcome::full_success("absent", Some("none"))
            } else {
                OperationOutcome::primary_failed("present", message)
            },
        }
    }

    #[test]
    fn uninstall_failure_keeps_original_error_and_never_claims_completed() {
        let report = uninstall_result_fixture(false);
        let original = report.message.clone();
        let finished = super::finish_windows_uninstall(Ok(report), "test", false).unwrap();
        assert_eq!(super::windows_uninstall_phase(&finished), "primary_failed");
        assert!(!finished.success);
        assert_eq!(finished.message, original);
        assert!(!finished.message.contains("已完成"));
        assert!(!finished.message.contains("已清理"));
        assert!(!super::external_msix_uninstall_note(false).contains("已移除"));
        assert!(super::external_msix_uninstall_note(false).contains("移除未完成"));
        assert!(super::external_msix_uninstall_note(true).contains("已移除"));
        let error = crate::errors::AppError::Engine("actual Remove-AppxPackage timeout".into());
        let original = error.to_string();
        assert_eq!(
            super::finish_windows_uninstall(Err(error), "test", false)
                .unwrap_err()
                .to_string(),
            original
        );
    }

    #[test]
    fn uninstall_ancillary_failure_is_partial_without_changing_primary_contract() {
        for mode in [
            "provenance",
            "cleanup",
            "already-absent",
            "pending-recovery",
        ] {
            let mut report = uninstall_result_fixture(true);
            if mode == "provenance" {
                report.outcome.provenance = StepOutcome::failed("record could not be removed");
            } else if mode == "pending-recovery" {
                report.outcome.push_recovery(recovery::CLEANUP_METADATA);
            } else {
                report.outcome.cleanup = StepOutcome::failed("policy restore access denied");
            }
            if mode == "already-absent" {
                report.action = "none".into();
                report.message = "Windows Codex is not installed.".into();
            }
            let original = report.message.clone();
            let finished = super::finish_windows_uninstall(Ok(report), "test", false).unwrap();
            assert_eq!(super::windows_uninstall_phase(&finished), "partial");
            assert!(
                finished.success,
                "primary success supports targeted cleanup retry"
            );
            assert!(finished.outcome.primary_ok);
            assert!(finished.message.starts_with(&original));
            assert!(finished.message.contains("附属清理或核验未完成"));
            assert!(finished
                .notes
                .iter()
                .any(|n| n == "original diagnostic / backup path"));
        }
    }

    #[test]
    fn uninstall_completed_only_after_primary_and_ancillary_steps_succeed() {
        let report = uninstall_result_fixture(true);
        let original = report.message.clone();
        let finished = super::finish_windows_uninstall(Ok(report), "test", false).unwrap();
        assert_eq!(super::windows_uninstall_phase(&finished), "completed");
        assert_eq!(finished.message, original);
        assert!(finished.success);
    }

    #[test]
    fn uninstall_bundle_late_postcondition_failure_replaces_success_headline() {
        for state in ["present", "unknown"] {
            let mut report = uninstall_result_fixture(true);
            super::fail_uninstall_postcondition(
                &mut report,
                state,
                "original registration probe detail".into(),
            );
            let finished =
                super::finish_windows_uninstall(Ok(report), "usb_bundle", false).unwrap();
            assert_eq!(super::windows_uninstall_phase(&finished), "primary_failed");
            assert_eq!(finished.outcome.app_state, state);
            assert!(!finished.success);
            assert!(finished
                .message
                .starts_with("卸载未完成：original registration probe detail"));
            assert!(finished
                .message
                .contains("前一步结果：Remove-AppxPackage succeeded"));
            assert!(!finished.message.contains("主卸载已完成"));
        }
    }

    #[test]
    fn uninstall_bundle_extra_portable_partial_is_not_a_complete_cleanup() {
        for success in [true, false] {
            let mut report = uninstall_result_fixture(true);
            let extra = PortableUninstallReport {
                success,
                partial: true,
                install_root: r"X:\fixture-only".into(),
                removed_files: success,
                removed_shortcut: false,
                removed_uninstall_entry: false,
                purged_user_data: false,
                message: "actual metadata failure".into(),
                notes: vec!["native cleanup diagnostic".into()],
            };
            super::merge_extra_portable_uninstall(
                &mut report,
                std::path::Path::new(r"X:\fixture-only"),
                extra,
                false,
            );
            let finished =
                super::finish_windows_uninstall(Ok(report), "usb_bundle", false).unwrap();
            assert_eq!(
                super::windows_uninstall_phase(&finished),
                if success { "partial" } else { "primary_failed" }
            );
            assert_eq!(finished.success, success);
            assert_eq!(finished.outcome.primary_ok, success);
            assert!(finished
                .notes
                .iter()
                .any(|n| n.contains("actual metadata failure")));
            assert!(finished
                .notes
                .iter()
                .any(|n| n == "native cleanup diagnostic"));
            assert!(finished
                .outcome
                .recovery_actions
                .iter()
                .any(|n| n == recovery::CLEANUP_METADATA));
            if !success {
                assert!(!finished.notes.iter().any(|n| n.contains("已移除旧版")));
            }
        }
    }
    use super::{
        allows_historical_portable_recovery, allows_portable_fallback,
        appx_volume_registration_needs_elevation, bind_manifest_checksums, check_win_update_abort,
        detect_existing_windows_install_at_path, detect_managed_codex,
        installed_msix_matches_volume, is_usb_portable_install_mode,
        outcome_from_portable_uninstall, portable_boundary_mutated_install_root,
        portable_fallback_needs_msix_cleanup, portable_install_result_message,
        retry_windows_ancillary_with_detector, should_retry_usb_volume_registration,
        usb_install_blocks_existing_install, validate_historical_windows_app_version,
        validate_usb_launch_target, WinAbortGuard, WinInstallStatus, WinPerformAction,
        FIXED_MSIX_INSTALL_MODE, USB_APPX_INSTALL_MODE, USB_PORTABLE_INSTALL_MODE,
        WIN_UPDATE_ABORT,
    };
    use crate::app::operation_outcome::{recovery, OperationOutcome, StepOutcome};
    use crate::app::provenance::ProvenanceStore;
    use crate::domain::settings::AppSettings;
    use codex_win_engine::{
        InstalledWindowsCodex, MsixSideloadReport, PortableBoundary, PortableUninstallReport,
        WindowsRelease,
    };
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-manager-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_portable_install(dir: &std::path::Path, version: &str) {
        std::fs::write(dir.join("Codex.exe"), b"").unwrap();
        std::fs::write(
            dir.join("AppxManifest.xml"),
            format!(
                r#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI OpCo, LLC" Version="{version}" ProcessorArchitecture="x64" />
</Package>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn cross_machine_usb_uninstall_accepts_live_bundle_without_local_record() {
        let root = temp_test_dir("cross-machine-usb-uninstall");
        write_fake_portable_install(&root, "26.1002.7124.0");
        let installed = detect_existing_windows_install_at_path(&root).unwrap();
        let store = ProvenanceStore::default();
        assert!(!store.is_managed_build(
            &installed.path,
            codex_win_engine::version_key(&installed.version)
        ));
        assert!(
            super::authorize_portable_bundle_uninstall(&installed, true, || {
                Ok(Some(root.clone()))
            })
            .unwrap()
        );
        // Authorization alone performs no uninstall and creates no provenance.
        assert!(root.join("Codex.exe").is_file());
        assert!(store.managed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_machine_usb_uninstall_requires_exact_current_root_after_drive_change() {
        let installed = InstalledWindowsCodex {
            path: r"G:\kit\apps\Codex".into(),
            version: "26.1002.7124.0".into(),
            arch: Some("x64".into()),
            source: "portable".into(),
            package_family_name: None,
            package_volume: None,
            installed_at: None,
        };
        for (root, accepted) in [
            (r"g:\kit\apps\codex", true),
            (r"F:\kit\apps\Codex", false),
            (r"G:\other-kit\apps\Codex", false),
            (r"G:\kit\apps\Codex-old", false),
            (r"G:\kit\apps", false),
        ] {
            assert_eq!(
                super::authorize_portable_bundle_uninstall(&installed, true, || {
                    Ok(Some(PathBuf::from(root)))
                })
                .unwrap(),
                accepted,
                "root={root}"
            );
        }
    }

    #[test]
    fn cross_machine_usb_uninstall_does_not_broaden_normal_manager_or_msix() {
        let mut installed = InstalledWindowsCodex {
            path: r"F:\kit\apps\Codex".into(),
            version: "26.1002.7124.0".into(),
            arch: Some("x64".into()),
            source: "portable".into(),
            package_family_name: None,
            package_volume: None,
            installed_at: None,
        };
        assert!(
            !super::authorize_portable_bundle_uninstall(&installed, false, || {
                panic!("ordinary manager must not use USB exception")
            })
            .unwrap()
        );
        for source in ["msix", "unknown"] {
            installed.source = source.into();
            assert!(
                !super::authorize_portable_bundle_uninstall(&installed, true, || {
                    panic!("non-portable installs must retain their own policy")
                })
                .unwrap()
            );
        }
    }

    #[test]
    fn cross_machine_usb_uninstall_rejects_missing_or_failed_live_authorization() {
        let installed = InstalledWindowsCodex {
            path: r"F:\kit\apps\Codex".into(),
            version: "26.1002.7124.0".into(),
            arch: Some("x64".into()),
            source: "portable".into(),
            package_family_name: None,
            package_volume: None,
            installed_at: None,
        };
        assert!(
            !super::authorize_portable_bundle_uninstall(&installed, true, || Ok(None)).unwrap()
        );
        for reason in [
            "license invalid",
            "watchdog stopped",
            "USB removed",
            "reparse point",
        ] {
            let error = super::authorize_portable_bundle_uninstall(&installed, true, || {
                Err(crate::errors::AppError::Engine(reason.into()))
            })
            .unwrap_err();
            assert!(error.to_string().contains(reason));
        }
    }

    #[test]
    fn historical_msix_app_version_must_match_selected_release() {
        assert!(validate_historical_windows_app_version(
            Some("26.707.31428".to_string()),
            "26.707.31428"
        )
        .is_ok());

        let mismatch = validate_historical_windows_app_version(
            Some("26.707.30751".to_string()),
            "26.707.31428",
        )
        .unwrap_err();
        assert!(mismatch.to_string().contains("不一致"));

        let unreadable = validate_historical_windows_app_version(None, "26.707.31428").unwrap_err();
        assert!(unreadable.to_string().contains("无法读取"));
    }

    #[test]
    fn fixed_msix_mode_disables_portable_fallback() {
        assert!(!allows_portable_fallback(FIXED_MSIX_INSTALL_MODE));
        assert!(allows_portable_fallback("msix"));
        assert!(allows_portable_fallback("portable"));
    }

    #[test]
    fn usb_appx_volume_mode_never_falls_back_to_c_drive() {
        assert!(!allows_portable_fallback(USB_APPX_INSTALL_MODE));
        assert!(!allows_historical_portable_recovery(
            USB_APPX_INSTALL_MODE,
            true
        ));
        assert!(!allows_historical_portable_recovery(
            USB_APPX_INSTALL_MODE,
            false
        ));
    }

    #[test]
    fn only_add_appxvolume_access_denied_requests_elevation() {
        let denied = MsixSideloadReport {
            success: false,
            message: r"Could not register removable AppX volume D:\: Access is denied.".to_string(),
            installed: None,
            fallback_recommended: true,
            raw_error: Some("operation=Add-AppxVolume".to_string()),
            hresult: Some("0x80070005".to_string()),
            activity_id: None,
            deployment_log: None,
        };
        assert!(appx_volume_registration_needs_elevation(&denied));

        let package_denied = MsixSideloadReport {
            message: "Add-AppxPackage failed: Access is denied".to_string(),
            raw_error: None,
            ..denied.clone()
        };
        assert!(!appx_volume_registration_needs_elevation(&package_denied));

        let successful = MsixSideloadReport {
            success: true,
            ..denied
        };
        assert!(!appx_volume_registration_needs_elevation(&successful));
    }

    #[test]
    fn failed_combined_prerequisites_do_not_suppress_targeted_volume_retry() {
        let denied = MsixSideloadReport {
            success: false,
            message: r"Could not register removable AppX volume D:\: Access is denied.".to_string(),
            installed: None,
            fallback_recommended: true,
            raw_error: Some("operation=Add-AppxVolume".to_string()),
            hresult: Some("0x80070005".to_string()),
            activity_id: None,
            deployment_log: None,
        };
        assert!(should_retry_usb_volume_registration(&denied, false));
        assert!(!should_retry_usb_volume_registration(&denied, true));
    }

    #[test]
    fn usb_mode_blocks_external_and_non_current_managed_installations() {
        let usb_root = std::path::Path::new(r"F:\bundle\apps\codex");
        let external = WinInstallStatus {
            installed: Some(InstalledWindowsCodex {
                path: r"C:\Program Files\Codex".to_string(),
                version: "1.0.0".to_string(),
                arch: Some("x64".to_string()),
                source: "msix".to_string(),
                package_family_name: None,
                package_volume: None,
                installed_at: None,
            }),
            status: "external".to_string(),
            recovery_pending: false,
        };
        assert!(usb_install_blocks_existing_install(&external, usb_root));

        let current_usb = WinInstallStatus {
            installed: Some(InstalledWindowsCodex {
                path: usb_root.to_string_lossy().into_owned(),
                version: "1.0.0".to_string(),
                arch: Some("x64".to_string()),
                source: "portable".to_string(),
                package_family_name: None,
                package_volume: None,
                installed_at: None,
            }),
            status: "managed".to_string(),
            recovery_pending: false,
        };
        assert!(!usb_install_blocks_existing_install(&current_usb, usb_root));

        let managed_elsewhere = WinInstallStatus {
            installed: Some(InstalledWindowsCodex {
                path: r"C:\Codex\apps\codex".to_string(),
                version: "1.0.0".to_string(),
                arch: Some("x64".to_string()),
                source: "portable".to_string(),
                package_family_name: None,
                package_volume: None,
                installed_at: None,
            }),
            status: "managed".to_string(),
            recovery_pending: false,
        };
        assert!(usb_install_blocks_existing_install(
            &managed_elsewhere,
            usb_root
        ));

        let absent = WinInstallStatus {
            installed: None,
            status: "none".to_string(),
            recovery_pending: false,
        };
        assert!(!usb_install_blocks_existing_install(&absent, usb_root));
    }

    #[test]
    fn usb_volume_verification_requires_the_registered_package_volume() {
        let installed = InstalledWindowsCodex {
            path: r"F:\WindowsApps\OpenAI.Codex".to_string(),
            version: "26.825.5331.0".to_string(),
            arch: Some("x64".to_string()),
            source: "msix".to_string(),
            package_family_name: Some("OpenAI.Codex_2p2nqsd0c76g0".to_string()),
            package_volume: Some(r"f:\".to_string()),
            installed_at: None,
        };
        assert!(installed_msix_matches_volume(
            &installed,
            std::path::Path::new(r"F:\")
        ));
        assert!(!installed_msix_matches_volume(
            &installed,
            std::path::Path::new(r"C:\")
        ));
        let mut missing = installed;
        missing.package_volume = None;
        assert!(!installed_msix_matches_volume(
            &missing,
            std::path::Path::new(r"F:\")
        ));
    }

    #[test]
    fn local_fixed_msix_allows_same_bytes_portable_recovery() {
        assert!(allows_historical_portable_recovery(
            FIXED_MSIX_INSTALL_MODE,
            true
        ));
        assert!(!allows_historical_portable_recovery(
            FIXED_MSIX_INSTALL_MODE,
            false
        ));
        assert!(allows_historical_portable_recovery("msix", false));
    }

    #[test]
    fn portable_fallback_cleans_previous_or_attempted_msix_registration() {
        assert!(portable_fallback_needs_msix_cleanup(Some("msix"), false));
        assert!(portable_fallback_needs_msix_cleanup(None, true));
        assert!(portable_fallback_needs_msix_cleanup(Some("portable"), true));
        assert!(!portable_fallback_needs_msix_cleanup(
            Some("portable"),
            false
        ));
        assert!(!portable_fallback_needs_msix_cleanup(None, false));
    }

    #[test]
    fn portable_result_distinguishes_provenance_from_msix_cleanup_failure() {
        let mut outcome = OperationOutcome::full_success("present", Some("managed"));
        outcome.cleanup = StepOutcome::failed("old MSIX remains");
        let cleanup_message = portable_install_result_message("便携版安装完成", &outcome);
        assert!(cleanup_message.contains("旧 MSIX 未能移除"));
        assert!(cleanup_message.contains("已安装的应用"));
        assert!(!cleanup_message.contains("开始管理"));

        outcome.provenance = StepOutcome::failed("provenance write failed");
        let provenance_message = portable_install_result_message("便携版安装完成", &outcome);
        assert!(provenance_message.contains("开始管理"));
    }

    #[test]
    fn win_abort_guard_preserves_a_startup_race_cancel_and_resets_on_drop() {
        let _serial = crate::app::oplock::CANCEL_LATCH_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Mirrors the macOS guard test: a cancel landing before `perform` reaches
        // its first checkpoint must survive the guard's creation (no entry-clear)
        // and still be observed; the guard resets the latch on drop so the next
        // op — and background auto_stage — start clean.
        WIN_UPDATE_ABORT.store(true, Ordering::SeqCst);
        {
            let outer = WinAbortGuard::new();
            let inner = WinAbortGuard::new();
            assert!(
                check_win_update_abort().is_err(),
                "guard creation must not wipe a pending cancel"
            );
            drop(inner);
            assert!(
                check_win_update_abort().is_err(),
                "a nested guard must not clear the owning operation's cancel"
            );
            drop(outer);
        }
        assert!(
            check_win_update_abort().is_ok(),
            "guard drop must reset the latch for the next op"
        );
    }

    #[test]
    fn portable_mutation_evidence_requires_a_successful_install_root_rename() {
        let install_root = PathBuf::from(r"C:\Codex");
        let payload = PathBuf::from(r"C:\staging\Codex");
        let backup = PathBuf::from(r"C:\Codex.rollback");

        assert!(!portable_boundary_mutated_install_root(
            &PortableBoundary::BeforeMoveOld {
                install_root: install_root.clone(),
                payload: payload.clone(),
                backup: backup.clone(),
                had_previous: false,
            }
        ));
        assert!(!portable_boundary_mutated_install_root(
            &PortableBoundary::AfterMoveOld {
                install_root: install_root.clone(),
                payload: payload.clone(),
                backup: backup.clone(),
                had_previous: false,
            }
        ));
        assert!(portable_boundary_mutated_install_root(
            &PortableBoundary::AfterMoveOld {
                install_root: install_root.clone(),
                payload: payload.clone(),
                backup: backup.clone(),
                had_previous: true,
            }
        ));
        assert!(!portable_boundary_mutated_install_root(
            &PortableBoundary::BeforeMoveNew {
                install_root: install_root.clone(),
                payload: payload.clone(),
                backup: backup.clone(),
                had_previous: false,
            }
        ));
        assert!(portable_boundary_mutated_install_root(
            &PortableBoundary::AfterMoveNew {
                install_root: install_root.clone(),
                backup: backup.clone(),
                had_previous: false,
            }
        ));
        assert!(!portable_boundary_mutated_install_root(
            &PortableBoundary::RollbackCompleted {
                install_root,
                backup,
                had_previous: false,
            }
        ));
    }

    #[test]
    fn serializes_win_perform_actions_as_frontend_contract() {
        let cases = [
            (WinPerformAction::None, "\"none\""),
            (WinPerformAction::MsixSideload, "\"msix-sideload\""),
            (WinPerformAction::PortableFallback, "\"portable-fallback\""),
            (
                WinPerformAction::PortableFallbackAfterMsixFailure,
                "\"portable-fallback-after-msix-failure\"",
            ),
            (
                WinPerformAction::PortableFallbackAfterMsixUnhealthy,
                "\"portable-fallback-after-msix-unhealthy\"",
            ),
            (
                WinPerformAction::PortableFallbackMissingFramework,
                "\"portable-fallback-missing-framework\"",
            ),
        ];

        for (action, expected) in cases {
            assert_eq!(serde_json::to_string(&action).unwrap(), expected);
        }
    }

    #[test]
    fn detects_user_selected_portable_install() {
        let dir = temp_test_dir("manual-existing-portable");
        write_fake_portable_install(&dir, "26.623.31921.0");

        let installed = detect_existing_windows_install_at_path(&dir).unwrap();
        assert_eq!(installed.path, dir.to_string_lossy());
        assert_eq!(installed.version, "26.623.31921.0");
        assert_eq!(installed.arch.as_deref(), Some("x64"));
        assert_eq!(installed.source, "portable");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usb_install_mode_is_the_only_mode_that_enables_usb_commit_guard() {
        assert!(is_usb_portable_install_mode(USB_PORTABLE_INSTALL_MODE));
        assert!(is_usb_portable_install_mode("PORTABLE-USB"));
        assert!(!is_usb_portable_install_mode("portable"));
        assert!(!is_usb_portable_install_mode("msix"));
    }

    #[test]
    fn rejects_user_selected_non_codex_folder() {
        let dir = temp_test_dir("manual-existing-empty");
        let err = detect_existing_windows_install_at_path(&dir).unwrap_err();
        assert!(err.to_string().contains("Codex.exe"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_detection_prefers_latest_provenance_record() {
        let old_dir = temp_test_dir("managed-old-portable");
        let new_dir = temp_test_dir("managed-new-portable");
        write_fake_portable_install(&old_dir, "26.623.31921.0");
        write_fake_portable_install(&new_dir, "26.623.42026.0");

        let mut store = ProvenanceStore::default();
        store.record(
            old_dir.to_string_lossy().into_owned(),
            codex_win_engine::version_key("26.623.31921.0"),
            "adopted-external",
        );
        store.record(
            new_dir.to_string_lossy().into_owned(),
            codex_win_engine::version_key("26.623.42026.0"),
            "adopted-external",
        );
        let settings = AppSettings::new(
            "https://codexapp.agentsmirror.com".to_string(),
            temp_test_dir("managed-missing-root")
                .join("missing")
                .to_string_lossy()
                .into_owned(),
        );

        let installed = detect_managed_codex(&settings, &store).unwrap();
        assert_eq!(installed.path, new_dir.to_string_lossy());
        assert_eq!(installed.version, "26.623.42026.0");

        let _ = std::fs::remove_dir_all(&old_dir);
        let _ = std::fs::remove_dir_all(&new_dir);
    }

    #[test]
    fn usb_launch_rejects_registered_external_portable_when_usb_payload_is_missing() {
        let external = temp_test_dir("usb-launch-external-portable");
        let missing_usb_root = temp_test_dir("usb-launch-missing-usb-root").join("Codex");
        write_fake_portable_install(&external, "26.1002.52244.0");

        let protected_files = [
            "Codex.exe",
            "CodexCore.dat",
            "codex-usb-entry-overlay.json",
            "codex-usb-integrity-recovery.json",
            "CodexCore.dat.codex-usb-recovery.bak",
            "codex-usb-entry-overlay.json.codex-usb-recovery.bak",
        ];
        for name in protected_files {
            std::fs::write(external.join(name), format!("protected:{name}")).unwrap();
        }
        let before = protected_files
            .iter()
            .map(|name| (*name, std::fs::read(external.join(name)).unwrap()))
            .collect::<Vec<_>>();

        let mut store = ProvenanceStore::default();
        store.record(
            external.to_string_lossy().into_owned(),
            codex_win_engine::version_key("26.1002.52244.0"),
            "adopted-external",
        );
        let settings = AppSettings::new(
            "https://codexapp.agentsmirror.com".to_string(),
            missing_usb_root.to_string_lossy().into_owned(),
        );
        let selected = detect_managed_codex(&settings, &store)
            .expect("manager detection reproduces the external-portable fallback");
        assert_eq!(selected.path, external.to_string_lossy());

        let rejected = validate_usb_launch_target(&selected, &missing_usb_root);
        assert!(rejected.is_err());
        for (name, expected) in before {
            assert_eq!(std::fs::read(external.join(name)).unwrap(), expected);
        }

        let _ = std::fs::remove_dir_all(&external);
        let _ = std::fs::remove_dir_all(missing_usb_root.parent().unwrap());
    }

    fn release() -> WindowsRelease {
        WindowsRelease {
            version: "26.602.3474.0".to_string(),
            package_version: "26.602.3474.0".to_string(),
            released_at: None,
            package_moniker: "OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0".to_string(),
            architecture: Some("x64".to_string()),
            download_architecture: None,
            content_length: Some(566_504_666),
            etag: None,
            store_product_id: Some("9PLM9XGG6VKS".to_string()),
            package_identity: Some("OpenAI.Codex".to_string()),
        }
    }

    #[test]
    fn binds_manifest_checksums_to_url_moniker() {
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix
";
        let sha = bind_manifest_checksums(
            &release(),
            checksums,
            "https://example.com/OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix",
        )
        .unwrap();
        assert_eq!(
            sha,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn accepts_stable_windows_short_url_without_msix_suffix() {
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix
";
        let sha = bind_manifest_checksums(&release(), checksums, "https://example.com/latest/win")
            .unwrap();
        assert_eq!(
            sha,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn rejects_manifest_url_moniker_mismatch() {
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix
";
        let err = bind_manifest_checksums(
            &release(),
            checksums,
            "https://example.com/OpenAI.Codex_26.602.3474.0_arm64__2p2nqsd0c76g0.Msix",
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not match URL artifact"));
    }

    #[test]
    fn rejects_manifest_identity_mismatch() {
        let mut release = release();
        release.package_identity = Some("OpenAI.Other".to_string());
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix
";
        assert!(bind_manifest_checksums(
            &release,
            checksums,
            "https://example.com/OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix",
        )
        .is_err());
    }

    #[test]
    fn portable_uninstall_primary_ok_when_tree_already_absent() {
        let portable = PortableUninstallReport {
            success: true,
            partial: true,
            install_root: r"C:\Codex".into(),
            removed_files: false, // already gone
            removed_shortcut: false,
            removed_uninstall_entry: false,
            purged_user_data: false,
            message: "cleanup warnings".into(),
            notes: vec!["Start Menu shortcut cleanup failed: access denied".into()],
        };
        let outcome = outcome_from_portable_uninstall(&portable, StepOutcome::ok(), r"C:\Codex");
        assert!(outcome.primary_ok, "absent tree is still primary success");
        assert_eq!(outcome.app_state, "absent");
        assert_eq!(outcome.path.as_deref(), Some(r"C:\Codex"));
        assert!(outcome.is_partial());
        assert!(outcome
            .recovery_actions
            .iter()
            .any(|a| a == recovery::CLEANUP_METADATA));
        // Path lives in the field, not smuggled through warnings.
        assert!(!outcome.warnings.iter().any(|w| w.starts_with("path:")));
    }

    #[test]
    fn portable_uninstall_surfaces_user_data_purge_failure_as_partial() {
        let portable = PortableUninstallReport {
            success: true,
            partial: true,
            install_root: r"C:\Codex".into(),
            removed_files: true,
            removed_shortcut: true,
            removed_uninstall_entry: true,
            purged_user_data: false,
            message: "cleanup warnings".into(),
            notes: vec!["User data cleanup failed: access denied".into()],
        };
        let outcome = outcome_from_portable_uninstall(&portable, StepOutcome::ok(), r"C:\Codex");
        assert!(outcome.primary_ok);
        assert!(outcome.is_partial());
        assert!(outcome
            .recovery_actions
            .iter()
            .any(|a| a == recovery::PURGE_USER_DATA));
    }

    #[test]
    fn portable_uninstall_hard_failure_is_not_partial() {
        let portable = PortableUninstallReport {
            success: false,
            partial: false,
            install_root: r"C:\Codex".into(),
            removed_files: false,
            removed_shortcut: false,
            removed_uninstall_entry: false,
            purged_user_data: false,
            message: "remove failed".into(),
            notes: vec![],
        };
        let outcome = outcome_from_portable_uninstall(&portable, StepOutcome::ok(), r"C:\Codex");
        assert!(!outcome.primary_ok);
        assert!(!outcome.is_partial());
        assert_eq!(outcome.app_state, "present");
    }

    #[test]
    fn retry_record_provenance_without_detected_install_keeps_retry_action() {
        let report = retry_windows_ancillary_with_detector(
            &[recovery::RECORD_PROVENANCE.to_string()],
            None,
            false,
            || WinInstallStatus {
                installed: None,
                status: "none".to_string(),
                recovery_pending: false,
            },
        )
        .expect("a missing install is a retryable ancillary result");

        assert!(report.outcome.provenance.is_failed());
        assert!(report
            .outcome
            .recovery_actions
            .iter()
            .any(|action| action == recovery::RECORD_PROVENANCE));
    }

    #[test]
    fn fixed_msix_missing_dependency_is_rejected_before_deployment() {
        let mut check = codex_win_engine::MsixDependencyPrecheck {
            checked: true,
            frameworks_ok: false,
            missing_frameworks: vec!["Microsoft.VCLibs.140.00 x64 >= 14.0".into()],
            reason: "required framework absent".into(),
        };
        let error = super::validate_fixed_msix_dependencies(&check, false).unwrap_err();
        assert!(error.to_string().contains("Microsoft.VCLibs.140.00"));
        assert!(super::validate_fixed_msix_dependencies(&check, true).is_ok());
        check.checked = false;
        assert!(super::validate_fixed_msix_dependencies(&check, false).is_ok());
        check.checked = true;
        check.frameworks_ok = true;
        check.missing_frameworks.clear();
        assert!(super::validate_fixed_msix_dependencies(&check, false).is_ok());
    }

    #[test]
    fn recovery_status_distinguishes_pending_from_archived_evidence() {
        let root = temp_test_dir("pending-recovery");
        assert!(!super::appx_recovery_pending_at(&root));
        std::fs::write(root.join("recovery-123.completed.json"), "archived").unwrap();
        assert!(!super::appx_recovery_pending_at(&root));
        let primary = root.join("recovery.json");
        std::fs::write(&primary, "{invalid or interrupted}").unwrap();
        assert!(super::appx_recovery_pending_at(&root));
        std::fs::rename(&primary, root.join("recovery.json.bak")).unwrap();
        assert!(super::appx_recovery_pending_at(&root));
        std::fs::remove_file(root.join("recovery.json.bak")).unwrap();
        std::fs::create_dir(&primary).unwrap();
        assert!(super::appx_recovery_pending_at(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
}
