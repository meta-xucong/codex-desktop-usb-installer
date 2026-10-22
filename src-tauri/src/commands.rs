use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use codex_win_engine::InstalledWindowsCodex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

use crate::app::account::AccountStatus;
use crate::app::atomic_file;
use crate::app::capability_pack::{
    install as install_capability_pack_impl, status as capability_pack_status_impl,
    CapabilityPackInstallOptions, CapabilityPackReport,
};
use crate::app::ccswitch::{
    install_and_launch as install_and_launch_ccswitch_impl, uninstall as uninstall_ccswitch_impl,
    CcswitchLaunchReport, CcswitchUninstallReport,
};
use crate::app::channels::codex::normalize_relay_base_url;
use crate::app::config_health::ConfigHealth;
use crate::app::diagnostics::Diagnostics;
use crate::app::disk::available_space;
use crate::app::install_tx::SelfUpdatePolicyTransition;
use crate::app::logging::redact_url;
use crate::app::mac_update::{
    cancel_macos_download, detect_existing_install_at_path as detect_macos_install_at_path,
    discard_macos_download, install_macos_historical_release,
    install_macos_with_network_and_phase_for_target, mac_adopt_path as adopt_macos_path,
    pause_macos_download, perform_macos_update_with_network_and_phase,
    plan_macos_update_with_network, resolve_macos_local_package, retry_macos_ancillary,
    stage_macos_update_with_network, uninstall_macos, AbortGuard as MacAbortGuard,
    HistoricalMacEvidence, HistoricalMacExpectation, InstalledCodex, MacInstallStatus,
    MacPerformReport, MacStageReport, MacUninstallReport, MacUpdateReport, PerformExpectation,
};
use crate::app::offline_payload::{
    cleanup_staged_payload, probe_offline_payload, stage_offline_payload, OfflinePayloadReport,
    OfflineStageReport,
};
use crate::app::op_phase::{OperationPhase, QuitPolicy};
use crate::app::operation_outcome::{
    recovery, AncillaryRetryReport, AncillaryRetryRequest, OperationOutcome, StepOutcome,
};
use crate::app::oplock::{
    HistoricalResumeContext, HistoricalResumeExpectation, HistoricalResumeSelection,
    OperationCompletion, OperationError, OperationGuard, OperationKind, OperationManager,
    OperationProgress, OperationResumeContext, OperationResumeExpectation, OperationResumeKind,
    OperationSnapshot, OperationToken,
};
use crate::app::paths;
use crate::app::provenance::ProvenanceStore;
use crate::app::release_install::{
    fetch_catalog, local_package_from_resolved, resolve_confirmed_macos_local_asset,
    resolve_release_asset, verify_release_checksum, HistoricalReleaseCatalog, LocalReleasePackage,
    ReleaseArchitecture, ReleasePlatform, ResolvedReleaseAsset,
};
use crate::app::settings_store::AppSettings as PersistedAppSettings;
use crate::app::settings_store::{CodexPayloadSourceKind, ProxyMode, UpdateSource};
use crate::app::skill_repo_update::{
    apply_update as apply_skill_repo_update, check_update as check_skill_repo_update,
    rollback as rollback_skill_repo_update, SkillRepoApplyReport, SkillRepoExpected,
    SkillRepoRollbackReport, SkillRepoUpdateInfo,
};
use crate::app::url_guard::{validate_custom_proxy, validate_custom_source};
use crate::app::usb_guard::UsbGuardStatus;
use crate::app::win_update::{
    auto_stage_windows_update_with_install_mode_and_network, cancel_windows_download,
    cleanup_windows_codex_machine_orphan,
    detect_existing_windows_install_at_path as detect_windows_install_at_path,
    discard_windows_download, install_windows_historical_release, pause_windows_download,
    perform_windows_update_with_install_mode_network_and_phase,
    plan_windows_update_with_install_mode_and_network, probe_windows_codex_machine_state,
    resolve_windows_local_package, retry_windows_ancillary,
    stage_windows_update_with_install_mode_and_network, uninstall_windows_codex,
    uninstall_windows_codex_for_usb_bundle, win_adopt as adopt_windows_install,
    win_adopt_path as adopt_windows_path, win_install_status,
    DownloadProgress as WinDownloadProgress, HistoricalWinExpectation, OperationEvidence,
    WinAbortGuard, WinAutoStageReport, WinInstallStatus, WinPerformExpectation, WinPerformReport,
    WinStageReport, WinUninstallReport, WinUpdateReport, FIXED_MSIX_INSTALL_MODE,
};
use crate::app::window_mode::{self, WindowMode, WindowModeReport};
use crate::domain::settings::AppSettings as DomainAppSettings;
use crate::domain::target::{Architecture, OperatingSystem, Target};
use crate::errors::{classify, AppError, CommandError, ErrorKind};
use crate::state::{ManagerState, UninstallHandoff};

fn normalize_windows_source_base(raw: &str) -> Option<String> {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return None;
    }
    for suffix in [
        "/latest/manifest",
        "/latest/checksums",
        "/latest/win-unpacked",
        "/latest/win-arm64",
        "/latest/win-x64",
        "/latest/win",
        "/latest",
    ] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/').to_string();
            break;
        }
    }
    (!base.is_empty()).then_some(base)
}

fn windows_endpoints_for_settings(
    state: &ManagerState,
) -> Result<crate::domain::manifest::MirrorEndpoints, AppError> {
    let mut saved = PersistedAppSettings::load();
    normalize_settings_for_target(&mut saved, &state.target);
    match saved.source {
        UpdateSource::Custom => {
            let base = if saved.custom_url.trim().is_empty() {
                state.settings.mirror_base_url.clone()
            } else {
                let normalized = validate_custom_source(&saved.custom_url).map_err(|e| {
                    let host = redact_url(&saved.custom_url);
                    log::warn!("url_guard rejected custom Windows source reason={e} host={host}");
                    AppError::Engine(e.to_string())
                })?;
                normalize_windows_source_base(&normalized)
                    .unwrap_or_else(|| state.settings.mirror_base_url.clone())
            };
            Ok(crate::domain::manifest::MirrorEndpoints::from_base_url(&base))
        }
        UpdateSource::Official => Err(AppError::Engine(
            "Windows official update source is not available yet; choose mirror, auto, or a custom source that serves latest/manifest, latest/checksums, and latest/win.".to_string(),
        )),
        // Windows currently depends on the mirror-style manifest/checksum/MSIX
        // endpoints. `auto` therefore resolves to the known-good mirror until an
        // official source exposes the same contract.
        UpdateSource::Auto | UpdateSource::Mirror => Ok(state.endpoints.clone()),
    }
}

fn normalize_settings_for_target(
    settings: &mut PersistedAppSettings,
    target: &crate::domain::target::Target,
) {
    if matches!(target.os, OperatingSystem::Windows) && settings.source == UpdateSource::Official {
        settings.source = UpdateSource::Auto;
    }
}

fn windows_install_mode_for_settings() -> String {
    // A running USB bundle owns the program tree below the authenticated
    // removable bundle. Never let a persisted local-manager preference select
    // MSIX/AppX or an ordinary C: portable fallback for this command path.
    if crate::app::usb_guard::guard_required() {
        return crate::app::win_update::USB_PORTABLE_INSTALL_MODE.to_string();
    }
    let saved = PersistedAppSettings::load();
    if saved.windows_install_mode == "portable" {
        "portable".to_string()
    } else {
        "msix".to_string()
    }
}

fn is_usb_install_path(requested_by_caller: bool, running_from_usb_bundle: bool) -> bool {
    // The renderer flag is a hint for ordinary manager callers, not a security
    // boundary. A signed USB build must remain on the USB path even if a stale
    // or direct IPC caller omits the flag.
    requested_by_caller || running_from_usb_bundle
}

fn validated_custom_proxy_for_settings(raw: &str, context: &str) -> Result<String, AppError> {
    validate_custom_proxy(raw).map_err(|e| {
        log::warn!("url_guard rejected {context} proxy reason={e}");
        AppError::Engine(e.to_string())
    })
}

fn mac_network_config_for_settings() -> Result<codex_mac_engine::NetworkConfig, AppError> {
    let saved = PersistedAppSettings::load();
    match saved.proxy_mode {
        ProxyMode::System => Ok(codex_mac_engine::NetworkConfig::system()),
        ProxyMode::Direct => Ok(codex_mac_engine::NetworkConfig::direct()),
        ProxyMode::Custom => {
            let proxy = validated_custom_proxy_for_settings(&saved.custom_proxy_url, "mac update")?;
            Ok(codex_mac_engine::NetworkConfig::custom(proxy))
        }
    }
}

fn win_network_config_for_settings() -> Result<codex_win_engine::NetworkConfig, AppError> {
    let saved = PersistedAppSettings::load();
    match saved.proxy_mode {
        ProxyMode::System => Ok(codex_win_engine::NetworkConfig::system()),
        ProxyMode::Direct => Ok(codex_win_engine::NetworkConfig::direct()),
        ProxyMode::Custom => {
            let proxy =
                validated_custom_proxy_for_settings(&saved.custom_proxy_url, "Windows update")?;
            Ok(codex_win_engine::NetworkConfig::custom(proxy))
        }
    }
}

async fn require_usb_install_network() -> Result<(), AppError> {
    let network = win_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        codex_win_engine::fetch_text_with_network("https://404token.xyz/", &network)
            .map(|_| ())
            .map_err(|error| {
                AppError::Engine(format!(
                    "USB installation requires internet access to 404token; connect to the internet and retry. No installation changes were started. ({error})"
                ))
            })
    })
    .await
    .map_err(|error| AppError::Internal(format!("join: {error}")))?
}

fn require_usb_install_account_binding() -> Result<(), AppError> {
    if !crate::app::usb_guard::guard_required() {
        return Ok(());
    }
    if !crate::app::usb_guard::signed_install_account_binding_ready()? {
        return Err(AppError::Engine(
            "签名 U 盘未包含可用的 404token API Key，无法完成自动配置；请先重新签名。".to_string(),
        ));
    }
    Ok(())
}

async fn require_usb_install_preflight() -> Result<(), AppError> {
    require_usb_install_network().await?;
    require_usb_install_account_binding()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInstallPreflightReport {
    pub internet_available: bool,
    pub api_key_ready: bool,
}

enum HistoricalReleaseNetwork {
    Mac(codex_mac_engine::NetworkConfig),
    Windows(codex_win_engine::NetworkConfig),
}

impl HistoricalReleaseNetwork {
    fn fetch_text(&self, url: &str) -> Result<String, AppError> {
        match self {
            Self::Mac(network) => codex_mac_engine::sys::fetch_text_with_network(url, network)
                .map_err(|e| AppError::Engine(e.to_string())),
            Self::Windows(network) => codex_win_engine::fetch_text_with_network(url, network)
                .map_err(|e| AppError::Engine(e.to_string())),
        }
    }
}

fn historical_release_platform(state: &ManagerState) -> Result<ReleasePlatform, AppError> {
    match state.target.os {
        OperatingSystem::Macos => Ok(ReleasePlatform::Macos),
        OperatingSystem::Windows => Ok(ReleasePlatform::Windows),
        _ => Err(AppError::UnsupportedPlatform),
    }
}

fn historical_release_network(
    platform: ReleasePlatform,
) -> Result<HistoricalReleaseNetwork, AppError> {
    match platform {
        ReleasePlatform::Macos => Ok(HistoricalReleaseNetwork::Mac(
            mac_network_config_for_settings()?,
        )),
        ReleasePlatform::Windows => Ok(HistoricalReleaseNetwork::Windows(
            win_network_config_for_settings()?,
        )),
    }
}

fn parse_release_architecture(raw: &str) -> Result<ReleaseArchitecture, AppError> {
    ReleaseArchitecture::from_runtime(raw).ok_or_else(|| {
        AppError::Internal(format!("不支持的安装包架构：{raw}（仅支持 arm64 / x64）"))
    })
}

/// Reject a package architecture the current machine cannot execute. ARM64
/// hosts may intentionally install an x64 build through the platform's
/// compatibility layer, but an x64 host can never execute an ARM64 binary.
/// This is enforced in the command layer so a forged renderer cannot bypass
/// the picker UI and leave a signed-but-unlaunchable installation behind.
fn validate_historical_architecture(
    target: &Target,
    selected: ReleaseArchitecture,
) -> Result<(), AppError> {
    let x64_compatible = match (&target.os, target.arch, selected) {
        (OperatingSystem::Macos, Architecture::Arm64, ReleaseArchitecture::X64) => {
            crate::domain::target::macos_x64_compatibility_available()
        }
        (OperatingSystem::Windows, Architecture::Arm64, ReleaseArchitecture::X64) => {
            crate::domain::target::windows_x64_compatibility_available()
        }
        _ => true,
    };
    validate_historical_architecture_with_compatibility(target, selected, x64_compatible)
}

fn validate_historical_architecture_with_compatibility(
    target: &Target,
    selected: ReleaseArchitecture,
    x64_compatible: bool,
) -> Result<(), AppError> {
    let supported = match (target.arch, selected) {
        (Architecture::X64, ReleaseArchitecture::X64) => true,
        (Architecture::Arm64, ReleaseArchitecture::Arm64) => true,
        (Architecture::Arm64, ReleaseArchitecture::X64) => x64_compatible,
        _ => false,
    };
    if supported {
        Ok(())
    } else if matches!(
        (target.os.clone(), target.arch, selected),
        (
            OperatingSystem::Macos,
            Architecture::Arm64,
            ReleaseArchitecture::X64
        )
    ) {
        Err(AppError::Engine(
            "当前 Apple Silicon 设备未安装或无法使用 Rosetta 2，拒绝替换为无法运行的 x64 Codex；请改选 arm64，或先安装 Rosetta 2"
                .to_string(),
        ))
    } else if matches!(
        (target.os.clone(), target.arch, selected),
        (
            OperatingSystem::Windows,
            Architecture::Arm64,
            ReleaseArchitecture::X64
        )
    ) {
        Err(AppError::Engine(
            "当前 Windows ARM64 设备不支持 x64 仿真，拒绝替换为无法运行的 x64 Codex；请改选 arm64，或升级到支持 x64 仿真的 Windows 版本"
                .to_string(),
        ))
    } else {
        Err(AppError::Engine(format!(
            "安装包架构 {} 无法在当前设备架构 {:?} 上运行",
            selected.as_str(),
            target.arch
        )))
    }
}

fn historical_package_start_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Apply the explicit choice from the historical-install confirmation before
/// the native installer can launch the selected Codex build. Mirrors the
/// Settings command's ordering: platform state first, then durable settings.
fn persist_historical_self_update_policy(disabled: bool) -> Result<(), AppError> {
    crate::app::codex_self_update::sync_and_persist_setting(disabled)
}

/// Undo a historical-install policy change after the install itself fails. The
/// platform sync is unconditional: the forward sync may have changed the current
/// process or user environment before a later platform/save step returned an
/// error while the settings file still contains the old value.
fn restore_historical_self_update_policy(previous: bool) -> Result<(), AppError> {
    crate::app::codex_self_update::sync_and_persist_setting(previous)
}

fn rollback_historical_policy_after_failure(previous: bool, primary: AppError) -> AppError {
    match restore_historical_self_update_policy(previous) {
        Ok(()) => primary,
        Err(rollback) => AppError::Internal(format!(
            "{primary}；同时恢复安装前的 Codex 自更新策略失败：{rollback}"
        )),
    }
}

/// A stale expectation proves that the retained resume context can no longer be
/// authorized. Clear it in the backend as well as the renderer so a reload
/// cannot resurrect an unrecoverable pause forever.
fn invalidate_retained_pause_if_stale(operations: &OperationManager, error: &AppError) {
    if matches!(error, AppError::StaleExpectation(_)) {
        operations.clear_paused_snapshot();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum HistoricalPolicyInstallState {
    PreCommit = 0,
    Mutated = 1,
    RolledBack = 2,
    OutcomeUnknown = 3,
}

impl HistoricalPolicyInstallState {
    fn load(state: &AtomicU8) -> Self {
        match state.load(Ordering::SeqCst) {
            1 => Self::Mutated,
            2 => Self::RolledBack,
            3 => Self::OutcomeUnknown,
            _ => Self::PreCommit,
        }
    }

    fn store(self, state: &AtomicU8) {
        state.store(self as u8, Ordering::SeqCst);
    }

    fn policy_may_be_restored(self) -> bool {
        matches!(self, Self::PreCommit | Self::RolledBack)
    }
}

fn record_macos_policy_install_state(state: &AtomicU8, evidence: HistoricalMacEvidence) {
    match evidence {
        HistoricalMacEvidence::MutationStarted => {
            HistoricalPolicyInstallState::Mutated.store(state)
        }
        // macOS atomic-swap rollback is the only platform mutation in this path,
        // so its explicit recovery evidence makes restoring the old policy safe.
        HistoricalMacEvidence::MutationRolledBack => {
            HistoricalPolicyInstallState::RolledBack.store(state)
        }
    }
}

fn record_windows_policy_install_state(state: &AtomicU8, evidence: OperationEvidence) {
    let current = HistoricalPolicyInstallState::load(state);
    let next = match evidence {
        OperationEvidence::OutcomeAmbiguous => HistoricalPolicyInstallState::OutcomeUnknown,
        // Add-AppxPackage may already be outcome-unknown before a later portable
        // fallback starts. Portable evidence cannot erase that system ambiguity.
        OperationEvidence::MutationStarted
            if current != HistoricalPolicyInstallState::OutcomeUnknown =>
        {
            HistoricalPolicyInstallState::Mutated
        }
        OperationEvidence::MutationRolledBack
            if current == HistoricalPolicyInstallState::Mutated =>
        {
            HistoricalPolicyInstallState::RolledBack
        }
        _ => current,
    };
    next.store(state);
}

fn historical_resume_context(
    resolved: &ResolvedReleaseAsset,
    local_path: Option<&Path>,
    block_updates: bool,
    expectation: HistoricalResumeExpectation,
    install_root: Option<&str>,
) -> HistoricalResumeContext {
    HistoricalResumeContext {
        selection: HistoricalResumeSelection {
            release_tag: resolved.release_tag.clone(),
            version: resolved.version.clone(),
            asset_name: resolved.name.clone(),
            architecture: resolved.architecture,
            format: resolved.format,
            package_version: resolved.package_version.clone(),
            local_path: local_path.map(|path| path.to_string_lossy().into_owned()),
            local_file_name: local_path.and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            }),
        },
        block_updates,
        expectation,
        install_root: install_root.map(str::to_string),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerUpdateMetadata {
    pub version: String,
    pub current_version: String,
    pub body: Option<String>,
}

fn manager_updater_builder(
    app: &AppHandle,
) -> Result<tauri_plugin_updater::UpdaterBuilder, AppError> {
    let saved = PersistedAppSettings::load();
    let mut builder = app.updater_builder();
    match saved.proxy_mode {
        ProxyMode::System => {}
        ProxyMode::Direct => {
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            let normalized =
                validated_custom_proxy_for_settings(&saved.custom_proxy_url, "manager updater")?;
            let proxy = url::Url::parse(&normalized)
                .map_err(|e| AppError::Engine(format!("invalid proxy URL: {e}")))?;
            builder = builder.proxy(proxy);
        }
    }
    Ok(builder)
}

fn manager_update_matches_confirmation(
    latest_version: &str,
    current_version: &str,
    expected_version: &str,
    expected_current_version: &str,
) -> bool {
    latest_version == expected_version.trim() && current_version == expected_current_version.trim()
}

#[tauri::command]
pub async fn manager_check_update(
    app: AppHandle,
) -> Result<Option<ManagerUpdateMetadata>, CommandError> {
    log::info!("manager update check start channel=self");
    let updater = manager_updater_builder(&app)?
        .build()
        .map_err(|e| AppError::Engine(format!("build manager updater: {e}")))?;
    let update =
        match tokio::time::timeout(std::time::Duration::from_secs(30), updater.check()).await {
            Ok(Ok(update)) => update,
            Ok(Err(error)) => {
                log::warn!("manager update check failed stage=metadata error={error}");
                return Err(AppError::Engine(format!("check manager update: {error}")).into());
            }
            Err(_) => {
                log::warn!("manager update check failed stage=metadata reason=timeout");
                return Err(AppError::Engine(
                    "manager update check timed out after 30 seconds".to_string(),
                )
                .into());
            }
        };
    log::info!(
        "manager update check completed available={}",
        update.is_some()
    );
    Ok(update.map(|update| ManagerUpdateMetadata {
        version: update.version,
        current_version: update.current_version,
        body: update.body,
    }))
}

#[tauri::command]
pub async fn manager_install_update(
    app: AppHandle,
    state: State<'_, ManagerState>,
    expected_version: String,
    expected_current_version: String,
) -> Result<(), CommandError> {
    reject_uninstall_handoff(&state)?;
    log::info!(
        "manager update install start expected_version={} expected_current_version={}",
        expected_version,
        expected_current_version
    );
    let updater = manager_updater_builder(&app)?
        .build()
        .map_err(|e| AppError::Engine(format!("build manager updater: {e}")))?;
    let update = updater
        .check()
        .await
        .map_err(|e| AppError::Engine(format!("check manager update before install: {e}")))?
        .ok_or_else(|| AppError::Engine("No manager update is available.".to_string()))?;
    if !manager_update_matches_confirmation(
        &update.version,
        &update.current_version,
        &expected_version,
        &expected_current_version,
    ) {
        return Err(AppError::StaleExpectation(
            "管理器更新内容已变化，请重新检查后再确认。".to_string(),
        )
        .into());
    }
    if let Err(error) = update.download_and_install(|_, _| {}, || {}).await {
        log::error!("manager update install failed stage=download-install error={error}");
        return Err(AppError::Engine(format!("install manager update: {error}")).into());
    }
    log::info!("manager update install completed");
    Ok(())
}

// ── Codex payload update channel (docs/UPDATE-CHANNELS.md § "Channel: codex")

/// Resolve the payload update mode only from the backend USB authorization
/// state. The renderer cannot select a recovery mode or provide a bundle path.
fn authorized_codex_payload_root() -> Result<(PathBuf, bool), AppError> {
    match crate::app::usb_guard::authorized_signed_bundle_identity() {
        Ok((root, _)) => Ok((root, false)),
        Err(normal_error) if crate::app::usb_guard::guard_required() => {
            crate::app::usb_guard::require_signed_codex_reinstall_bundle(
                "codex-payload-command-route",
            )
            .map(|authorization| (authorization.bundle_root, true))
            .map_err(|reinstall_error| {
                log::debug!(
                    "restricted Codex reinstall route denied after normal gate: {reinstall_error}"
                );
                normal_error
            })
        }
        Err(error) => Err(error),
    }
}

/// Read-only check of the USB payload against the configured codex payload
/// repo. Returns `None` when up to date. Network failures surface as errors;
/// the frontend decides how softly to present them.
#[tauri::command]
pub async fn payload_check_codex(
    state: State<'_, ManagerState>,
) -> Result<Option<crate::app::update_channel::UpdateInfo>, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let (bundle_root, reactivation_reinstall) = authorized_codex_payload_root()?;
    let settings = PersistedAppSettings::load();
    let ctx = crate::app::update_channel::UpdateContext {
        bundle_root,
        settings,
    };
    tauri::async_runtime::spawn_blocking(move || {
        if reactivation_reinstall {
            crate::app::channels::codex::check_reinstall(&ctx)
        } else {
            crate::app::channels::codex::check(&ctx)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Phase-1 apply: refresh the USB payload after the user clicks. Re-validates
/// everything against fresh remote metadata (never trusts the round-tripped
/// `info`). Deployment to this machine's Windows installation is a separate
/// user action reusing the existing win_* update commands.
#[tauri::command]
pub async fn payload_apply_codex(
    app: AppHandle,
    state: State<'_, ManagerState>,
    info: crate::app::update_channel::UpdateInfo,
) -> Result<(), CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    let (bundle_root, reactivation_reinstall) = authorized_codex_payload_root()?;
    let settings = PersistedAppSettings::load();
    let ctx = crate::app::update_channel::UpdateContext {
        bundle_root,
        settings,
    };
    let progress_app = app.clone();
    let progress = move |done: u64, total: u64| {
        let _ = progress_app.emit(
            "payload-update-progress",
            serde_json::json!({
                "channel": "codex",
                "phase": "download",
                "bytesDone": done,
                "bytesTotal": total,
            }),
        );
    };
    let _ = app.emit(
        "payload-update-progress",
        serde_json::json!({
            "channel": "codex",
            "phase": "preparing",
        }),
    );
    let update_result = tauri::async_runtime::spawn_blocking(move || {
        if reactivation_reinstall {
            crate::app::channels::codex::apply_reinstall(&ctx, &info, &progress)
        } else {
            crate::app::channels::codex::apply(&ctx, &info, &progress)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?;
    if let Err(error) = update_result {
        log::error!(
            "codex payload update command failed stage=command code={} error={error}",
            error.code()
        );
        return Err(error.into());
    }
    if reactivation_reinstall {
        // The updater never creates a lease. Re-run manager startup so it can
        // retain the restricted reinstall state or authorize only after the
        // complete USB bundle has been verified again.
        let status = crate::app::usb_guard::start_for_manager()?;
        let _ = app.emit("usb-guard-status-changed", status);
    }
    let _ = app.emit(
        "payload-update-progress",
        serde_json::json!({
            "channel": "codex",
            "phase": "done",
        }),
    );
    Ok(())
}

fn windows_domain_settings_for_persisted(state: &ManagerState) -> DomainAppSettings {
    let saved = PersistedAppSettings::load();
    let mut settings = state.settings.clone();
    // A removable drive letter can change between sessions. In the fixed USB
    // product the executable itself is the authority, so always derive the
    // program root from the currently running, guarded bundle instead of a
    // stale absolute path persisted on C:.
    settings.install_root = crate::app::usb_guard::current_usb_codex_root()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or(saved.install_root);
    settings.disable_codex_self_updates = saved.disable_codex_self_updates;
    settings
}

fn dialog_start_dir(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_dir() {
        return path;
    }
    path.parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(PersistedAppSettings::load().install_root))
}

fn mac_existing_install_start_dir() -> PathBuf {
    let system = PathBuf::from("/Applications");
    if system.is_dir() {
        return system;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let user_apps = PathBuf::from(home).join("Applications");
        if user_apps.is_dir() {
            return user_apps;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

const MIN_PORTABLE_FREE_SPACE_BYTES: u64 = 1_073_741_824;
const INSTALL_LOCATION_PROBE_PREFIX: &str = ".codex-manager-install-test-";

fn begin_guard(state: &ManagerState, kind: OperationKind) -> Result<OperationGuard, CommandError> {
    reject_uninstall_handoff(state)?;
    state
        .operations
        .begin(kind)
        .map_err(|err| {
            log::warn!(
                "failed to acquire operation guard kind={} error={err}",
                kind.as_str()
            );
            AppError::from(err)
        })
        .map_err(Into::into)
}

fn reject_uninstall_handoff(state: &ManagerState) -> Result<(), CommandError> {
    if state.uninstall_handoff_active() {
        return Err(
            AppError::Busy("卸载清理正在等待管理器退出，请先完成并退出".to_string()).into(),
        );
    }
    Ok(())
}

struct DetachedGuard {
    completion_tracking: bool,
    operations: OperationManager,
    succeeded: bool,
    token: Option<OperationToken>,
}

impl DetachedGuard {
    fn validate_tracked(state: &ManagerState, token: OperationToken) -> Result<Self, CommandError> {
        Self::validate_inner(state, token, true)
    }

    fn validate_inner(
        state: &ManagerState,
        token: OperationToken,
        completion_tracking: bool,
    ) -> Result<Self, CommandError> {
        reject_uninstall_handoff(state)?;
        let operations = state.operations.clone();
        operations
            .validate(&token)
            .map_err(destructive_token_error)?;
        Ok(Self {
            completion_tracking,
            operations,
            succeeded: false,
            token: Some(token),
        })
    }

    fn validate_with_phase(
        state: &ManagerState,
        token: OperationToken,
        phase: OperationPhase,
    ) -> Result<Self, CommandError> {
        reject_uninstall_handoff(state)?;
        let operations = state.operations.clone();
        operations
            .validate_with_phase(&token, phase)
            .map_err(destructive_token_error)?;
        Ok(Self {
            completion_tracking: false,
            operations,
            succeeded: false,
            token: Some(token),
        })
    }

    fn mark_succeeded(&mut self) {
        self.succeeded = true;
    }

    fn set_phase(&self, phase: OperationPhase) {
        if let Some(token) = self.token.as_ref() {
            let _ = self.operations.set_phase(token, phase);
        }
    }

    fn token_clone(&self) -> Option<OperationToken> {
        self.token.clone()
    }

    fn operations(&self) -> OperationManager {
        self.operations.clone()
    }

    fn confirm_pause_if_cancelled(&self, error: &AppError) {
        if classify(&error.to_string()) != ErrorKind::Cancelled {
            return;
        }
        if let Some(token) = self.token.as_ref() {
            if let Err(confirm_error) = self.operations.confirm_pause(token) {
                log::error!("failed to confirm terminal paused download: {confirm_error}");
            }
        }
    }

    fn mark_outcome_ambiguous_if_noninterruptible(&self) {
        if !self.operations.phase().is_point_of_no_return() {
            return;
        }
        if let Some(token) = self.token.as_ref() {
            if let Err(error) = self.operations.mark_outcome_ambiguous(token) {
                log::error!("failed to mark terminal operation outcome ambiguous: {error}");
            }
        }
    }
}

/// Progress payload emitted on the download event bus. Includes the operation
/// id so a reloaded frontend can reject late events from a previous op.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressEvent {
    downloaded: u64,
    total: u64,
    source: String,
    operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<OperationPhase>,
}

fn emit_op_phase(
    app: &AppHandle,
    operations: &OperationManager,
    token: &OperationToken,
    channel: &str,
    phase: OperationPhase,
) {
    let _ = operations.set_phase(token, phase);
    let snapshot = operations.snapshot();
    let progress = snapshot
        .filter(|snapshot| snapshot.id == token.0)
        .and_then(|snapshot| snapshot.progress);
    let _ = app.emit(
        channel,
        DownloadProgressEvent {
            downloaded: progress.as_ref().map(|p| p.downloaded).unwrap_or(0),
            total: progress.as_ref().map(|p| p.total).unwrap_or(0),
            source: progress.map(|p| p.source).unwrap_or_default(),
            operation_id: token.0.clone(),
            phase: Some(phase),
        },
    );
}

fn emit_op_download_progress(
    app: &AppHandle,
    operations: &OperationManager,
    token: &OperationToken,
    channel: &str,
    downloaded: u64,
    total: u64,
    source: String,
) {
    let progress = OperationProgress {
        downloaded,
        total,
        source: source.clone(),
    };
    let _ = operations.set_progress(token, progress);
    // Bytes in flight normally mean Downloading, but portable extraction also
    // uses this event channel while the operation is already Applying.  Do
    // not regress the durable phase or the UI will lose its install wording
    // and re-enable download controls during a non-interruptible write.
    let current_phase = operations
        .snapshot()
        .filter(|snapshot| snapshot.id == token.0)
        .map(|snapshot| snapshot.phase);
    if total > 0
        && downloaded < total
        && matches!(
            current_phase,
            None | Some(
                OperationPhase::Idle | OperationPhase::Preparing | OperationPhase::Downloading
            )
        )
    {
        let _ = operations.set_phase(token, OperationPhase::Downloading);
    }
    let phase = operations
        .snapshot()
        .filter(|snapshot| snapshot.id == token.0)
        .map(|snapshot| snapshot.phase);
    let _ = app.emit(
        channel,
        DownloadProgressEvent {
            downloaded,
            total,
            source,
            operation_id: token.0.clone(),
            phase,
        },
    );
}

impl Drop for DetachedGuard {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if self.completion_tracking {
                if let Err(error) = self.operations.record_completion(&token, self.succeeded) {
                    log::error!("failed to record terminal operation outcome: {error}");
                }
            }
            let _ = self.operations.end(token);
        }
    }
}

fn destructive_token_error(err: OperationError) -> CommandError {
    match err {
        OperationError::InvalidToken => {
            log::warn!("destructive token validation failed");
            AppError::StaleExpectation("操作令牌无效或已过期，请重新检查后再确认".to_string())
                .into()
        }
        other => {
            log::warn!("destructive token rejected error={other}");
            AppError::from(other).into()
        }
    }
}

fn refresh_config_health(state: &ManagerState) -> ConfigHealth {
    let (_, settings_health) = PersistedAppSettings::load_with_health();
    let (_, provenance_health) = ProvenanceStore::load_with_health();
    let health =
        ConfigHealth::from_parts(settings_health, provenance_health).with_live_backup_flags();
    let mut slot = state
        .config_health
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    *slot = health.clone();
    health
}

fn config_path(which: &str) -> Result<PathBuf, AppError> {
    match which {
        "settings" => paths::settings_path()
            .ok_or_else(|| AppError::Internal("无法定位 settings.json 数据目录".to_string())),
        "provenance" => paths::provenance_path()
            .ok_or_else(|| AppError::Internal("无法定位 provenance.json 数据目录".to_string())),
        _ => Err(AppError::Internal(
            "配置类型必须是 settings 或 provenance".to_string(),
        )),
    }
}

fn auto_stage_busy_report(enabled: bool, allow_metered: bool) -> WinAutoStageReport {
    WinAutoStageReport {
        enabled,
        allow_metered,
        attempted: false,
        skipped: true,
        reason: "operation-busy".to_string(),
        stage: None,
        capabilities: None,
        notes: vec![
            "Automatic Windows pre-download was skipped because another operation is running."
                .to_string(),
        ],
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn path_is_equal_or_child(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    path == root || path.starts_with(&format!("{root}\\"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendErrorPayload {
    kind: String,
    message: String,
    stack: Option<String>,
    component_stack: Option<String>,
}

#[cfg(windows)]
fn protected_windows_roots() -> Vec<PathBuf> {
    [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
        "SystemRoot",
        "WINDIR",
    ]
    .into_iter()
    .filter_map(std::env::var_os)
    .filter(|value| !value.is_empty())
    .map(PathBuf::from)
    .collect()
}

#[cfg(not(windows))]
fn protected_windows_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none() || (path.has_root() && path.components().count() <= 2)
}

fn is_protected_install_root(path: &Path) -> bool {
    is_filesystem_root(path)
        || protected_windows_roots()
            .iter()
            .any(|root| path_is_equal_or_child(path, root))
}

fn is_existing_codex_portable_root(path: &Path) -> bool {
    // Identity-gated (entry exe + AppxManifest Identity == OpenAI.Codex): a
    // directory that merely contains a ChatGPT.exe (e.g. an unpacked ChatGPT
    // Classic) must not be treated as a replaceable Codex install.
    codex_win_engine::detect_portable_install(path).is_some()
}

fn directory_is_empty(path: &Path) -> Result<bool, AppError> {
    let mut entries = std::fs::read_dir(path)
        .map_err(|e| AppError::Internal(format!("读取安装位置失败: {e}")))?;
    Ok(entries.next().is_none())
}

/// Probe the operation the portable installer ultimately needs: create a
/// non-empty child directory and atomically rename it beside the install root.
/// A plain file write inside an existing install only proves file creation; it
/// does not prove the parent grants directory replacement/delete-child rights.
fn probe_install_parent_replace(path: &Path) -> Result<PathBuf, AppError> {
    let requested_parent = path.parent().unwrap_or(path);
    let probe_dir = nearest_existing_dir(requested_parent);
    let probe_id = uuid::Uuid::new_v4();
    let source = probe_dir.join(format!("{INSTALL_LOCATION_PROBE_PREFIX}{probe_id}-source"));
    let destination = probe_dir.join(format!(
        "{INSTALL_LOCATION_PROBE_PREFIX}{probe_id}-destination"
    ));

    let probe_result = (|| -> std::io::Result<()> {
        std::fs::create_dir(&source)?;
        std::fs::write(source.join("probe"), b"ok")?;
        codex_win_engine::rename_directory_with_retry(
            "probe install parent directory rename",
            &source,
            &destination,
        )?;
        Ok(())
    })();

    // Validation must remain side-effect free. Clean both names because the
    // probe can fail before or after the rename boundary.
    let source_cleanup = if source.exists() {
        codex_win_engine::remove_directory_all_with_retry(
            "clean install parent source probe",
            &source,
        )
    } else {
        Ok(())
    };
    let destination_cleanup = if destination.exists() {
        codex_win_engine::remove_directory_all_with_retry(
            "clean install parent destination probe",
            &destination,
        )
    } else {
        Ok(())
    };

    let cleanup_result = source_cleanup.and(destination_cleanup);
    match (probe_result, cleanup_result) {
        (Ok(()), Ok(())) => {}
        (Err(probe_err), Ok(())) => {
            return Err(AppError::Internal(format!(
                "安装位置父目录不可写或不支持目录替换: {probe_err}"
            )));
        }
        (Ok(()), Err(cleanup_err)) => {
            return Err(AppError::Internal(format!(
                "安装位置探测目录清理失败: {cleanup_err}"
            )));
        }
        (Err(probe_err), Err(cleanup_err)) => {
            return Err(AppError::Internal(format!(
                "安装位置父目录不可写或不支持目录替换: {probe_err}; 探测目录清理也失败: {cleanup_err}"
            )));
        }
    }
    Ok(probe_dir)
}

fn validate_install_root_path(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Internal("安装位置不能为空".to_string()));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(AppError::Internal("安装位置必须是绝对路径".to_string()));
    }
    if path.exists() && !path.is_dir() {
        return Err(AppError::Internal("安装位置必须是文件夹".to_string()));
    }
    if is_protected_install_root(&path) {
        return Err(AppError::Internal(
            "安装位置不能放在系统目录、管理员目录或磁盘根目录".to_string(),
        ));
    }
    if path.exists() && !directory_is_empty(&path)? && !is_existing_codex_portable_root(&path) {
        return Err(AppError::Internal(
            "安装位置必须是空文件夹，或已有的 Codex 免安装版目录".to_string(),
        ));
    }
    // Probe parent-level directory creation and rename WITHOUT creating the
    // requested target itself. This mirrors the install swap and also gives us
    // the same-volume ancestor for the free-space check.
    let probe_dir = probe_install_parent_replace(&path)?;
    if let Some(free) = available_space(&probe_dir)? {
        if free < MIN_PORTABLE_FREE_SPACE_BYTES {
            return Err(AppError::Internal(
                "安装位置所在磁盘剩余空间不足".to_string(),
            ));
        }
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Nearest existing directory at or above `path`. Lets us probe writability and
/// free space for a not-yet-created install root without creating it.
fn nearest_existing_dir(path: &Path) -> PathBuf {
    let mut cur = path;
    loop {
        if cur.is_dir() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return cur.to_path_buf(),
        }
    }
}

fn install_root_from_picked_dir(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Internal("安装位置不能为空".to_string()));
    }
    let selected = PathBuf::from(trimmed);
    if !selected.is_absolute() {
        return Err(AppError::Internal("安装位置必须是绝对路径".to_string()));
    }
    if selected.exists() && !selected.is_dir() {
        return Err(AppError::Internal("安装位置必须是文件夹".to_string()));
    }
    if is_filesystem_root(&selected) || is_protected_install_root(&selected) {
        return validate_install_root_path(trimmed);
    }
    if selected.exists()
        && selected.is_dir()
        && !directory_is_empty(&selected)?
        && !is_existing_codex_portable_root(&selected)
    {
        return validate_install_root_path(&selected.join("Codex").to_string_lossy());
    }
    validate_install_root_path(trimmed)
}

/// Read-only historical package catalog. Only assets with a GitHub SHA-256
/// digest and a pinned, expected repository URL are returned to the frontend.
#[tauri::command]
pub async fn historical_release_catalog(
    state: State<'_, ManagerState>,
    architecture: String,
) -> Result<HistoricalReleaseCatalog, CommandError> {
    let architecture = parse_release_architecture(&architecture)?;
    validate_historical_architecture(&state.target, architecture)?;
    let platform = historical_release_platform(&state)?;
    let network = historical_release_network(platform)?;
    tauri::async_runtime::spawn_blocking(move || {
        fetch_catalog(platform, architecture, &|url| network.fetch_text(url))
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Pick a local DMG/ZIP/MSIX and verify it entirely through the native platform
/// trust chain. This path intentionally performs no GitHub or Sparkle request.
#[tauri::command]
pub async fn historical_pick_local_package(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    architecture: String,
) -> Result<Option<LocalReleasePackage>, CommandError> {
    let architecture = parse_release_architecture(&architecture)?;
    validate_historical_architecture(&state.target, architecture)?;
    let platform = historical_release_platform(&state)?;
    let start_dir = historical_package_start_dir();
    tauri::async_runtime::spawn_blocking(
        move || -> Result<Option<LocalReleasePackage>, AppError> {
            let builder = app
                .dialog()
                .file()
                .set_title("选择从 GitHub Release 下载的 Codex 安装包")
                .set_directory(start_dir);
            let selected = match platform {
                ReleasePlatform::Macos => builder.add_filter("Codex DMG / ZIP", &["dmg", "zip"]),
                ReleasePlatform::Windows => builder.add_filter("Codex MSIX", &["msix"]),
            }
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok());
            let Some(path) = selected else {
                return Ok(None);
            };
            let resolved = match platform {
                ReleasePlatform::Macos => resolve_macos_local_package(&path, architecture),
                ReleasePlatform::Windows => resolve_windows_local_package(&path, architecture),
            }?;
            Ok(Some(local_package_from_resolved(&path, &resolved)))
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// First-version USB entry point: inspect the fixed manifest and MSIX beside
/// the running setup executable.  No catalog, GitHub, Store or mirror request
/// is made.  The quick preflight intentionally defers the expensive
/// Authenticode scan until `offline_payload_stage` has copied the MSIX to the
/// local temporary directory.  `package_path` exists only for developer
/// diagnostics and must still point inside the discovered bundle.
#[tauri::command]
pub async fn offline_payload_probe(
    state: State<'_, ManagerState>,
    package_path: Option<String>,
) -> Result<OfflinePayloadReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let expected_architecture = state.target.arch.as_str().to_string();
    let package_path = package_path.map(PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        probe_offline_payload(
            &executable,
            package_path.as_deref(),
            Some(expected_architecture.as_str()),
            false,
        )
    })
    .await
    .map_err(|error| AppError::Internal(format!("join: {error}")))?
    .map_err(Into::into)
}

/// Copy a hash/identity-verified USB MSIX to the local temporary directory,
/// verify Authenticode there, and return the path to the guarded installer.
/// The subsequent historical-install command receives this staged path, so
/// the removable drive is no longer held open during AppX deployment.
#[tauri::command]
pub async fn offline_payload_stage(
    state: State<'_, ManagerState>,
    package_path: Option<String>,
) -> Result<OfflineStageReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_signed_codex_install_bundle("offline-payload-stage")?;
    require_usb_install_preflight().await?;
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let expected_architecture = state.target.arch.as_str().to_string();
    let package_path = package_path.map(PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        stage_offline_payload(
            &executable,
            package_path.as_deref(),
            Some(expected_architecture.as_str()),
            true,
        )
    })
    .await
    .map_err(|error| AppError::Internal(format!("join: {error}")))?
    .map_err(Into::into)
}

/// Check network access and the signed API-key binding before the UI stages or
/// installs any local payload. The mutating commands repeat both checks.
#[tauri::command]
pub async fn win_check_install_preflight(
    state: State<'_, ManagerState>,
) -> Result<UsbInstallPreflightReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    let internet_available = require_usb_install_network().await.is_ok();
    let api_key_ready = require_usb_install_account_binding().is_ok();
    Ok(UsbInstallPreflightReport {
        internet_available,
        api_key_ready,
    })
}

/// Best-effort cleanup for a staged MSIX after a successful installation.  The
/// backend validates the path is below `%TEMP%\\CodexDesktopInstaller` before
/// deleting anything.
#[tauri::command]
pub async fn offline_payload_cleanup(
    state: State<'_, ManagerState>,
    staged_path: String,
) -> Result<(), CommandError> {
    if !cfg!(target_os = "windows") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    tauri::async_runtime::spawn_blocking(move || cleanup_staged_payload(Path::new(&staged_path)))
        .await
        .map_err(|error| AppError::Internal(format!("join: {error}")))?
        .map_err(Into::into)
}

/// macOS-only: detect the installed Codex build, read the Sparkle appcast, and
/// return an update plan (delta vs full). Read-only — performs no install.
#[tauri::command]
pub async fn mac_plan_update(
    simulated_build: Option<u64>,
) -> Result<MacUpdateReport, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    // Off the main thread: the appcast fetch (plus the auto-source official
    // probe) is network IO — running it inline froze the webview, so the
    // re-check spinner never animated ("卡一下没动画").
    let network = mac_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        plan_macos_update_with_network(simulated_build, &network)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// macOS-only: plan + download + size/EdDSA verify into staging. Non-destructive
/// (no apply/swap). Runs the blocking download off the main thread.
#[tauri::command]
pub async fn mac_stage_update(
    state: State<'_, ManagerState>,
    simulated_build: Option<u64>,
) -> Result<MacStageReport, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let _op = begin_guard(&state, OperationKind::Update)?;
    let network = mac_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        stage_macos_update_with_network(simulated_build, &network)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Locate the vendored Sparkle `BinaryDelta` tool, if present: an explicit
/// `CODEX_BINARY_DELTA` override first (testing / a system Sparkle), then the app
/// bundle's resources. Returns `None` when it isn't found — a *full*-package
/// update doesn't need it, so resolution is best-effort and only the delta path
/// errors on a genuine miss.
fn resolve_binary_delta(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CODEX_BINARY_DELTA") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for rel in ["resources/BinaryDelta", "BinaryDelta"] {
        if let Ok(res) = app
            .path()
            .resolve(rel, tauri::path::BaseDirectory::Resource)
        {
            if res.exists() {
                return Some(res);
            }
        }
    }
    None
}

/// macOS-only **destructive** update: download+verify → reconstruct → codesign
/// gate → graceful quit → atomic same-volume swap → health-check → relaunch (or
/// rollback). Requires an explicit `confirm: true` from a UI second confirmation;
/// runs the blocking work off the main thread.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacPerformResumeExpectation {
    from_build: u64,
    to_build: u64,
    path: String,
    from_version: String,
    to_version: String,
}

#[tauri::command]
pub async fn mac_perform_update(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    expected: MacPerformResumeExpectation,
) -> Result<MacPerformReport, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    if !confirm {
        return Err(
            AppError::Internal("拒绝执行：破坏性更新必须带显式 confirm".to_string()).into(),
        );
    }
    let mut op = DetachedGuard::validate_tracked(&state, token)?;
    op.set_phase(OperationPhase::Preparing);
    // Best-effort: a full-package update needs no delta tool, so don't reject the
    // whole operation when it's absent — only the delta branch requires it.
    let binary_delta = resolve_binary_delta(&app);
    // The user confirmed a specific target; the backend re-verifies reality still
    // matches before the destructive swap (guards a TOCTOU vs appcast refresh /
    // Codex self-update between confirm and execute).
    let engine_expected = PerformExpectation {
        from_build: expected.from_build,
        to_build: expected.to_build,
        install_path: expected.path.clone(),
    };
    if let Some(token) = op.token_clone() {
        state
            .operations
            .set_resume_context(
                &token,
                OperationResumeContext {
                    kind: OperationResumeKind::Perform,
                    install_root: None,
                    expectation: OperationResumeExpectation::Macos {
                        current_build: Some(expected.from_build),
                        target_build: expected.to_build,
                        install_path: Some(expected.path),
                        current_version: Some(expected.from_version),
                        target_version: expected.to_version,
                    },
                },
            )
            .map_err(AppError::from)?;
    }
    let progress_app = app.clone();
    let network = mac_network_config_for_settings()?;
    let ops = op.operations();
    let phase_token = op.token_clone();
    let progress_token = phase_token.clone();
    let progress_ops = ops.clone();
    let phase_app = progress_app.clone();
    let phase_ops = ops.clone();
    let phase_event_token = phase_token.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let report = move |p: crate::app::mac_update::DownloadProgress| {
            if let Some(token) = progress_token.as_ref() {
                emit_op_download_progress(
                    &progress_app,
                    &progress_ops,
                    token,
                    "mac://download-progress",
                    p.downloaded,
                    p.total,
                    p.source,
                );
            } else {
                let _ = progress_app.emit("mac://download-progress", p);
            }
        };
        let phase_hook = move |phase: OperationPhase| {
            if let Some(token) = phase_event_token.as_ref() {
                emit_op_phase(
                    &phase_app,
                    &phase_ops,
                    token,
                    "mac://download-progress",
                    phase,
                );
            }
        };
        perform_macos_update_with_network_and_phase(
            binary_delta,
            engine_expected,
            &report,
            &network,
            Some(&phase_hook),
        )
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(AppError::Internal(format!("join: {error}"))),
    };
    match result {
        Ok(report) => {
            op.mark_succeeded();
            Ok(report)
        }
        Err(error) => {
            op.confirm_pause_if_cancelled(&error);
            op.mark_outcome_ambiguous_if_noninterruptible();
            invalidate_retained_pause_if_stale(&state.operations, &error);
            Err(error.into())
        }
    }
}

/// macOS-only: classify the installed Codex (managed / external / none).
#[tauri::command]
pub fn mac_status(state: State<'_, ManagerState>) -> Result<MacInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    Ok(crate::app::mac_update::mac_install_status())
}

/// macOS-only: adopt the detected external install (after explicit user consent).
#[tauri::command]
pub fn mac_adopt(state: State<'_, ManagerState>) -> Result<MacInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let _op = begin_guard(&state, OperationKind::Adopt)?;
    crate::app::mac_update::mac_adopt().map_err(Into::into)
}

/// macOS-only: let the user pick an existing Codex install and validate it.
/// The bundle may be named Codex.app or (post-rebrand) ChatGPT.app — validation
/// is by CFBundleIdentifier, so ChatGPT Classic is rejected with a clear error.
#[tauri::command]
pub async fn mac_pick_existing_install(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<Option<InstalledCodex>, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let start_dir = mac_existing_install_start_dir();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择 Codex 应用（Codex.app 或 ChatGPT.app）")
            .set_directory(start_dir)
            .blocking_pick_file()
            .map(|path| {
                path.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| AppError::Internal(format!("读取选择的应用失败: {e}")))
            })
            .transpose()
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))??;
    selected
        .as_deref()
        .map(|path| detect_macos_install_at_path(Path::new(path)))
        .transpose()
        .map_err(Into::into)
}

/// macOS-only: adopt the user-selected Codex.app path.
#[tauri::command]
pub fn mac_adopt_path(
    state: State<'_, ManagerState>,
    path: String,
) -> Result<MacInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let _op = begin_guard(&state, OperationKind::Adopt)?;
    adopt_macos_path(Path::new(&path)).map_err(Into::into)
}

/// macOS-only: open the installed Codex.app (explicit 〔打开 Codex〕 action).
/// With a persisted theme selection this transparently becomes "launch
/// debuggable + inject" — unless an operation holds the lock, in which case
/// the launch stays plain rather than racing the updater for the process.
#[tauri::command]
pub async fn mac_launch_codex(state: State<'_, ManagerState>) -> Result<(), CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let settings = PersistedAppSettings::load();
    if settings.codex_theme.is_some() && state.operations.snapshot().is_none() {
        let themed =
            crate::app::codex_theme::launch_with_active_theme(&state.codex_theme, &settings, None)
                .await?;
        if themed {
            return Ok(());
        }
    }
    crate::app::mac_update::launch_codex().map_err(Into::into)
}

/// macOS-only: fresh-install the latest Codex (full package) into /Applications.
/// Runs the blocking download/verify/install off the main thread.
#[tauri::command]
pub async fn mac_install(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    expected_target_build: u64,
    expected_target_version: String,
) -> Result<MacInstallStatus, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let op = begin_guard(&state, OperationKind::Install)?;
    let token = op.token().clone();
    let ops = state.operations.clone();
    let _ = ops.set_phase(&token, OperationPhase::Preparing);
    ops.set_resume_context(
        &token,
        OperationResumeContext {
            kind: OperationResumeKind::Install,
            install_root: None,
            expectation: OperationResumeExpectation::Macos {
                current_build: None,
                target_build: expected_target_build,
                install_path: None,
                current_version: None,
                target_version: expected_target_version,
            },
        },
    )
    .map_err(AppError::from)?;
    let network = match mac_network_config_for_settings() {
        Ok(network) => network,
        Err(error) => {
            if let Err(record_error) = ops.record_completion(&token, false) {
                log::error!("failed to record terminal macOS install outcome: {record_error}");
            }
            return Err(error.into());
        }
    };
    let progress_token = token.clone();
    let progress_ops = ops.clone();
    let phase_token = token.clone();
    let phase_ops = ops.clone();
    let phase_app = app.clone();
    let phase_event_token = phase_token.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let report = move |p: crate::app::mac_update::DownloadProgress| {
            emit_op_download_progress(
                &app,
                &progress_ops,
                &progress_token,
                "mac://download-progress",
                p.downloaded,
                p.total,
                p.source,
            );
        };
        let phase_hook = move |phase: OperationPhase| {
            emit_op_phase(
                &phase_app,
                &phase_ops,
                &phase_event_token,
                "mac://download-progress",
                phase,
            );
        };
        install_macos_with_network_and_phase_for_target(
            &report,
            &network,
            Some(&phase_hook),
            Some(expected_target_build),
        )
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(AppError::Internal(format!("join: {error}"))),
    };
    match result {
        Ok(status) => {
            if let Err(error) = ops.record_completion(&token, true) {
                log::error!("failed to record terminal macOS install outcome: {error}");
            }
            Ok(status)
        }
        Err(error) => {
            if classify(&error.to_string()) == ErrorKind::Cancelled {
                if let Err(confirm_error) = ops.confirm_pause(&token) {
                    log::error!("failed to confirm terminal paused download: {confirm_error}");
                }
            }
            if ops.phase().is_point_of_no_return() {
                if let Err(mark_error) = ops.mark_outcome_ambiguous(&token) {
                    log::error!(
                        "failed to mark terminal macOS install outcome ambiguous: {mark_error}"
                    );
                }
            }
            if let Err(record_error) = ops.record_completion(&token, false) {
                log::error!("failed to record terminal macOS install outcome: {record_error}");
            }
            invalidate_retained_pause_if_stale(&ops, &error);
            Err(error.into())
        }
    }
}

/// macOS-only historical/offline install. The selected tag/asset is rebound to
/// a fresh GitHub API response, GitHub digest must agree with SHA256SUMS when
/// that older release publishes one, and the existing atomic replacement +
/// rollback model handles both upgrade/downgrade.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn mac_install_historical_release(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    release_tag: String,
    asset_name: String,
    local_path: Option<String>,
    architecture: String,
    block_updates: bool,
    expected_current_path: Option<String>,
    expected_current_build: Option<u64>,
) -> Result<MacInstallStatus, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    if !confirm {
        return Err(
            AppError::Internal("拒绝执行：历史版本安装必须带显式 confirm".to_string()).into(),
        );
    }
    if expected_current_path.is_some() != expected_current_build.is_some() {
        return Err(AppError::StaleExpectation(
            "当前 macOS 安装快照不完整，请重新检查后再确认".to_string(),
        )
        .into());
    }
    let architecture = parse_release_architecture(&architecture)?;
    validate_historical_architecture(&state.target, architecture)?;
    let mut op = DetachedGuard::validate_tracked(&state, token)?;
    op.set_phase(OperationPhase::Preparing);
    // Own the preparing-phase cancel latch before any GitHub/tag/checksum or
    // local-package resolution can fail. The installer creates a nested guard;
    // reference counting keeps that inner return from clearing this command's
    // cancellation, and this outer scope always resets it on every early exit.
    let _abort_scope = MacAbortGuard::new();
    let local_path = local_path.map(PathBuf::from);
    let network = if local_path.is_some() {
        codex_mac_engine::NetworkConfig::system()
    } else {
        mac_network_config_for_settings()?
    };
    let progress_app = app.clone();
    let ops = op.operations();
    let phase_token = op.token_clone();
    let progress_token = phase_token.clone();
    let progress_ops = ops.clone();
    let phase_app = progress_app.clone();
    let phase_ops = ops.clone();
    let phase_event_token = phase_token.clone();
    let expected = HistoricalMacExpectation {
        current_path: expected_current_path,
        current_build: expected_current_build,
    };
    let previous_block_updates = PersistedAppSettings::load().disable_codex_self_updates;
    let policy_transition =
        (previous_block_updates != block_updates).then_some(SelfUpdatePolicyTransition {
            previous_disabled: previous_block_updates,
            requested_disabled: block_updates,
        });
    let policy_attempted = Arc::new(AtomicBool::new(false));
    let policy_attempted_worker = Arc::clone(&policy_attempted);
    let policy_attempted_rollback = Arc::clone(&policy_attempted);
    let policy_restored_after_rollback = Arc::new(AtomicBool::new(false));
    let policy_restored_worker = Arc::clone(&policy_restored_after_rollback);
    let policy_install_state =
        Arc::new(AtomicU8::new(HistoricalPolicyInstallState::PreCommit as u8));
    let policy_install_state_worker = Arc::clone(&policy_install_state);
    let report = tauri::async_runtime::spawn_blocking(move || {
        let fetch = |url: &str| {
            codex_mac_engine::sys::fetch_text_with_network(url, &network)
                .map_err(|e| AppError::Engine(e.to_string()))
        };
        let resolved = if let Some(path) = local_path.as_ref() {
            resolve_confirmed_macos_local_asset(path, &release_tag, &asset_name, architecture)?
        } else {
            let resolved = resolve_release_asset(
                &release_tag,
                &asset_name,
                ReleasePlatform::Macos,
                architecture,
                &fetch,
            )?;
            verify_release_checksum(&resolved, &fetch)?;
            resolved
        };
        if let Some(token) = phase_token.as_ref() {
            ops.set_historical_context(
                token,
                historical_resume_context(
                    &resolved,
                    local_path.as_deref(),
                    block_updates,
                    HistoricalResumeExpectation::Macos {
                        current_path: expected.current_path.clone(),
                        current_build: expected.current_build,
                    },
                    None,
                ),
            )
            .map_err(AppError::from)?;
        }
        let progress = move |p: crate::app::mac_update::DownloadProgress| {
            if let Some(token) = progress_token.as_ref() {
                emit_op_download_progress(
                    &progress_app,
                    &progress_ops,
                    token,
                    "mac://download-progress",
                    p.downloaded,
                    p.total,
                    p.source,
                );
            }
        };
        let phase_hook = move |phase: OperationPhase| {
            if let Some(token) = phase_event_token.as_ref() {
                emit_op_phase(
                    &phase_app,
                    &phase_ops,
                    token,
                    "mac://download-progress",
                    phase,
                );
            }
        };
        let evidence_hook = |evidence: HistoricalMacEvidence| {
            record_macos_policy_install_state(&policy_install_state_worker, evidence);
            if let Some(token) = phase_token.as_ref() {
                let result = match evidence {
                    HistoricalMacEvidence::MutationStarted => ops.mark_mutation_started(token),
                    HistoricalMacEvidence::MutationRolledBack => {
                        ops.mark_mutation_rolled_back(token)
                    }
                };
                if let Err(error) = result {
                    log::error!("failed to record macOS historical install evidence: {error}");
                }
            }
        };
        let before_commit = || {
            policy_attempted_worker.store(true, Ordering::SeqCst);
            persist_historical_self_update_policy(block_updates)
        };
        let after_rollback = || {
            if previous_block_updates != block_updates
                && policy_attempted_rollback.load(Ordering::SeqCst)
            {
                restore_historical_self_update_policy(previous_block_updates)?;
            }
            policy_restored_worker.store(true, Ordering::SeqCst);
            Ok(())
        };
        install_macos_historical_release(
            resolved,
            local_path,
            expected,
            &progress,
            &network,
            Some(&phase_hook),
            policy_transition,
            Some(&before_commit),
            Some(&evidence_hook),
            Some(&after_rollback),
        )
    })
    .await;
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            // A worker panic can happen after the platform mutation crossed its
            // commit boundary but before the evidence callback ran. Treat that
            // as outcome-unknown and keep the policy aligned with the requested
            // release instead of guessing that the old app survived.
            HistoricalPolicyInstallState::OutcomeUnknown.store(&policy_install_state);
            Err(AppError::Internal(format!("join: {error}")))
        }
    };
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            op.confirm_pause_if_cancelled(&error);
            invalidate_retained_pause_if_stale(&state.operations, &error);
            let error = if previous_block_updates != block_updates
                && policy_attempted.load(Ordering::SeqCst)
                && !policy_restored_after_rollback.load(Ordering::SeqCst)
                && HistoricalPolicyInstallState::load(&policy_install_state)
                    .policy_may_be_restored()
            {
                rollback_historical_policy_after_failure(previous_block_updates, error)
            } else {
                error
            };
            return Err(error.into());
        }
    };
    op.mark_succeeded();
    Ok(report)
}

/// macOS-only: request pausing an active package download.
/// Partial bytes are left in place for the next resume-capable run.
#[tauri::command]
pub fn mac_pause_download(
    state: State<'_, ManagerState>,
    operation_id: String,
) -> Result<bool, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    Ok(state
        .operations
        .request_pause(&OperationToken(operation_id), pause_macos_download))
}

/// macOS-only: request cancellation of an active package download.
/// Partial bytes are discarded.
#[tauri::command]
pub fn mac_cancel_download(
    state: State<'_, ManagerState>,
    operation_id: String,
) -> Result<bool, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    Ok(state
        .operations
        .request_cancellation(&OperationToken(operation_id), cancel_macos_download))
}

/// macOS-only: discard a PAUSED download. After a pause the curl process is gone
/// but its `.part` is still cached for resume; this drops it when the user
/// cancels from the paused state instead of resuming.
#[tauri::command]
pub fn mac_discard_download(state: State<'_, ManagerState>) -> Result<(), CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    discard_macos_download()?;
    state.operations.clear_paused_snapshot();
    Ok(())
}

/// Windows-only: detect installed Codex, read mirror manifest/checksums, probe
/// sideload capabilities, and return the preferred update path. Read-only.
#[tauri::command]
pub async fn win_plan_update(
    state: State<'_, ManagerState>,
) -> Result<WinUpdateReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let endpoints = windows_endpoints_for_settings(&state)?;
    let settings = windows_domain_settings_for_persisted(&state);
    let install_mode = windows_install_mode_for_settings();
    let network = win_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        plan_windows_update_with_install_mode_and_network(
            &endpoints,
            &settings,
            &install_mode,
            &network,
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Windows-only: plan + download + size/SHA256/AuthentiCode/AppxManifest gates
/// into staging. Non-destructive (no install yet).
#[tauri::command]
pub async fn win_stage_update(
    state: State<'_, ManagerState>,
) -> Result<WinStageReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    crate::app::usb_guard::require_signed_codex_bundle("win-stage-update")?;
    let _op = begin_guard(&state, OperationKind::Update)?;
    let endpoints = windows_endpoints_for_settings(&state)?;
    let settings = windows_domain_settings_for_persisted(&state);
    let install_mode = windows_install_mode_for_settings();
    let network = win_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        stage_windows_update_with_install_mode_and_network(
            &endpoints,
            &settings,
            &install_mode,
            &|_| {},
            &network,
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Read persisted app settings (update source + general).
#[tauri::command]
pub fn get_settings(state: State<'_, ManagerState>) -> Result<PersistedAppSettings, CommandError> {
    let mut settings = PersistedAppSettings::load();
    normalize_settings_for_target(&mut settings, &state.target);
    Ok(settings)
}

/// Persist app settings. `signed_only` is forced on regardless of input.
#[tauri::command]
pub fn set_settings(
    state: State<'_, ManagerState>,
    settings: PersistedAppSettings,
) -> Result<PersistedAppSettings, CommandError> {
    let mut s = settings;
    s.normalize();
    normalize_settings_for_target(&mut s, &state.target);
    // After normalize(), empty custom source/proxy is coerced away. Any remaining
    // Custom mode must carry a non-empty, validated URL so disk matches runtime.
    if s.source == UpdateSource::Custom {
        s.custom_url = validate_custom_source(&s.custom_url).map_err(|e| {
            let host = redact_url(&s.custom_url);
            log::warn!("url_guard rejected custom source reason={e} host={host}");
            AppError::Engine(e.to_string())
        })?;
    }
    if s.proxy_mode == ProxyMode::Custom {
        s.custom_proxy_url = validated_custom_proxy_for_settings(&s.custom_proxy_url, "settings")?;
    }
    if matches!(
        s.codex_payload_source_kind,
        CodexPayloadSourceKind::Auto
            | CodexPayloadSourceKind::Mirror
            | CodexPayloadSourceKind::Relay
    ) {
        s.codex_payload_base_url = normalize_relay_base_url(&s.codex_payload_base_url)?;
    }
    let previous = PersistedAppSettings::load();
    let _op = begin_guard(&state, OperationKind::SetInstallRoot)?;
    if previous.disable_codex_self_updates != s.disable_codex_self_updates {
        crate::app::codex_self_update::sync_setting(s.disable_codex_self_updates)?;
    }
    s.save()?;
    refresh_config_health(&state);
    log::info!(
        "saved settings source={} windows_install_mode={} proxy_mode={} disable_codex_self_updates={}",
        s.source.as_str(),
        s.windows_install_mode,
        s.proxy_mode.as_str(),
        s.disable_codex_self_updates
    );
    Ok(s)
}

#[tauri::command]
pub fn get_config_health(state: State<'_, ManagerState>) -> ConfigHealth {
    // Always re-read from disk so the UI sees post-restore/reset truth, not a
    // stale snapshot taken at process start.
    refresh_config_health(&state)
}

#[tauri::command]
pub fn restore_config_backup(
    state: State<'_, ManagerState>,
    which: String,
) -> Result<ConfigHealth, CommandError> {
    let _op = begin_guard(&state, OperationKind::SetInstallRoot)?;
    let path = config_path(which.as_str())?;
    let backup = atomic_file::backup_path(&path);
    if !backup.exists() {
        return Err(AppError::Internal(format!("找不到 {which} 的 .bak 备份")).into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("create data dir: {e}")))?;
    }
    let current_tmp = path.with_extension(format!("restore-current-{}", std::process::id()));
    if path.exists() {
        std::fs::rename(&path, &current_tmp)
            .map_err(|e| AppError::Internal(format!("move current config aside: {e}")))?;
    }
    if let Err(err) = std::fs::rename(&backup, &path) {
        if current_tmp.exists() {
            let _ = std::fs::rename(&current_tmp, &path);
        }
        return Err(AppError::Internal(format!("restore config backup: {err}")).into());
    }
    if current_tmp.exists() {
        let _ = std::fs::remove_file(current_tmp);
    }
    log::info!("restored config backup which={which}");
    // Re-read + re-verify; never claim success without a fresh health probe.
    let health = refresh_config_health(&state);
    let status = match which.as_str() {
        "settings" => health.settings_status.as_str(),
        "provenance" => health.provenance_status.as_str(),
        _ => "ok",
    };
    if status == "corrupt" {
        return Err(
            AppError::Internal(format!("已从 .bak 还原 {which}，但重新读取仍判定为损坏")).into(),
        );
    }
    Ok(health)
}

#[tauri::command]
pub fn reset_config(
    state: State<'_, ManagerState>,
    which: String,
) -> Result<ConfigHealth, CommandError> {
    let _op = begin_guard(&state, OperationKind::SetInstallRoot)?;
    match which.as_str() {
        "settings" => {
            let mut settings = PersistedAppSettings::default();
            normalize_settings_for_target(&mut settings, &state.target);
            crate::app::codex_self_update::sync_setting(settings.disable_codex_self_updates)?;
            settings.save()?;
        }
        "provenance" => {
            ProvenanceStore::default().save()?;
        }
        _ => {
            return Err(
                AppError::Internal("配置类型必须是 settings 或 provenance".to_string()).into(),
            )
        }
    }
    log::info!("reset config which={which}");
    let health = refresh_config_health(&state);
    let status = match which.as_str() {
        "settings" => health.settings_status.as_str(),
        "provenance" => health.provenance_status.as_str(),
        _ => "ok",
    };
    if status == "corrupt" {
        return Err(AppError::Internal(format!("已重置 {which}，但重新读取仍判定为损坏")).into());
    }
    Ok(health)
}

/// Retry only failed ancillary steps after a partial install/uninstall.
/// Never re-runs full install or uninstall of the app itself.
///
/// `purge_user_data` is destructive (deletes `~/.codex`): it requires the same
/// explicit confirm + armed uninstall token as a full uninstall, so it cannot
/// be one-clicked from a recovery CTA.
#[tauri::command]
pub fn retry_ancillary(
    state: State<'_, ManagerState>,
    request: AncillaryRetryRequest,
    confirm: Option<bool>,
    token: Option<OperationToken>,
) -> Result<AncillaryRetryReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    let actions = &request.actions;
    let path = request.path.as_deref();
    let purge = request.purge_user_data
        && actions
            .iter()
            .any(|a| a == crate::app::operation_outcome::recovery::PURGE_USER_DATA);
    // Hold either a scoped adopt lock or a validated destructive uninstall token
    // for the duration of the retry. Drop ends the token/lock (fields unread on purpose).
    #[allow(dead_code)]
    enum RetryGuard {
        Scoped(OperationGuard),
        Detached(DetachedGuard),
    }
    let _guard: RetryGuard = if purge {
        if confirm != Some(true) {
            return Err(
                AppError::Internal("清除用户数据需要二次确认（confirm=true）".to_string()).into(),
            );
        }
        let token = token.ok_or_else(|| {
            AppError::Internal(
                "清除用户数据需要破坏性令牌（先 arm_destructive uninstall）".to_string(),
            )
        })?;
        let guard = DetachedGuard::validate_with_phase(&state, token, OperationPhase::Committing)?;
        RetryGuard::Detached(guard)
    } else {
        RetryGuard::Scoped(begin_guard(&state, OperationKind::Adopt)?)
    };
    match state.target.os {
        OperatingSystem::Macos => retry_macos_ancillary(actions, path, purge).map_err(Into::into),
        OperatingSystem::Windows => {
            let settings = windows_domain_settings_for_persisted(&state);
            retry_windows_ancillary(&settings, actions, path, purge).map_err(Into::into)
        }
        _ => Err(AppError::UnsupportedPlatform.into()),
    }
}

#[tauri::command]
pub fn begin_operation(
    state: State<'_, ManagerState>,
    kind: OperationKind,
) -> Result<OperationToken, CommandError> {
    reject_uninstall_handoff(&state)?;
    state
        .operations
        .begin_detached(kind)
        .map_err(|err| {
            log::warn!(
                "begin_operation rejected kind={} error={err}",
                kind.as_str()
            );
            AppError::from(err)
        })
        .map_err(Into::into)
}

#[tauri::command]
pub fn arm_destructive(
    state: State<'_, ManagerState>,
    kind: OperationKind,
) -> Result<OperationToken, CommandError> {
    reject_uninstall_handoff(&state)?;
    if !matches!(kind, OperationKind::Update | OperationKind::Uninstall) {
        return Err(
            AppError::Internal("仅 update/uninstall 操作可使用破坏性令牌".to_string()).into(),
        );
    }
    state
        .operations
        .begin_detached(kind)
        .map_err(|err| {
            log::warn!(
                "arm_destructive rejected kind={} error={err}",
                kind.as_str()
            );
            AppError::from(err)
        })
        .map_err(Into::into)
}

#[tauri::command]
pub fn end_operation(
    state: State<'_, ManagerState>,
    token: OperationToken,
) -> Result<(), CommandError> {
    state
        .operations
        .end(token)
        .map_err(AppError::from)
        .map_err(Into::into)
}

/// Current same-process operation lease, if any. Used by the frontend on mount
/// to reattach progress/phase UI after a renderer reload without re-arming work.
#[tauri::command]
pub fn get_operation_snapshot(
    state: State<'_, ManagerState>,
) -> Result<Option<OperationSnapshot>, CommandError> {
    Ok(state.operations.snapshot())
}

/// Last transfer that reached a confirmed paused terminal state. The active
/// lease may already be gone by the time a replacement renderer polls.
#[tauri::command]
pub fn get_paused_operation_snapshot(
    state: State<'_, ManagerState>,
) -> Result<Option<OperationSnapshot>, CommandError> {
    Ok(state.operations.paused_snapshot())
}

/// Token-keyed terminal evidence for a renderer that lost the original invoke
/// promise. `failed-before-commit` and `rolled-back` prove that retrying a fresh
/// install is safe; unresolved committing/finishing failures remain outcome-unknown.
#[tauri::command]
pub fn get_operation_completion(
    state: State<'_, ManagerState>,
    token: OperationToken,
) -> Result<Option<OperationCompletion>, CommandError> {
    Ok(state.operations.completion(&token))
}

/// The user confirmed quitting from the close dialog — flag it and exit so the
/// CloseRequested / ExitRequested guards stop intercepting and let it go.
/// Still refuses when the backend is in a non-interruptible install phase.
#[tauri::command]
pub fn confirm_quit(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<(), CommandError> {
    let confirm_close = crate::app::settings_store::AppSettings::load().confirm_close;
    // Decide and arm exit under the SAME operation mutex used by phase changes.
    // If the worker already reached commit this returns Block. Otherwise both app
    // abort latches and force_quit are set before it can cross that boundary, so
    // the final pre-commit checkpoint observes the cancellation.
    let policy = state.operations.prepare_quit(confirm_close, true, || {
        let _ = cancel_macos_download();
        let _ = cancel_windows_download();
        state
            .force_quit
            .store(true, std::sync::atomic::Ordering::SeqCst);
    });
    if let QuitPolicy::Block {
        phase,
        reason_code,
        reason,
        kind,
    } = &policy
    {
        log::warn!(
            "confirm_quit blocked phase={} reason_code={reason_code} kind={:?} reason={reason}",
            phase.as_str(),
            kind
        );
        crate::dispatch_shell_event(
            &app,
            crate::app::shell::ShellEvent::QuitBlocked(policy.clone()),
        );
        return Err(AppError::Busy(reason.clone()).into());
    }
    crate::exit_after_confirm(&app);
    Ok(())
}

/// The Windows USB uninstaller returns a truthful `cleanup_pending` handoff
/// before the manager can exit.  The renderer calls this only after showing
/// that result to the user.  Do not turn this into a general-purpose exit
/// command: an armed in-process handoff is the proof that a helper is waiting
/// for this exact manager lifetime.
#[tauri::command]
pub fn get_uninstall_handoff(state: State<'_, ManagerState>) -> Option<UninstallHandoffStatus> {
    state
        .uninstall_handoff()
        .map(|handoff| UninstallHandoffStatus {
            id: handoff.id,
            report_path: handoff.report_path,
            message: handoff.message,
        })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallHandoffStatus {
    pub id: String,
    pub report_path: String,
    pub message: String,
}

/// Acknowledge the exact pending helper handoff and request a controlled exit.
/// The ID check is deliberately separate from the OS exit request: stale or
/// duplicated renderer events must not consume a newer handoff.
#[tauri::command]
pub fn complete_uninstall_handoff(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    handoff_id: String,
) -> Result<(), CommandError> {
    if handoff_id.trim().is_empty() {
        return Err(AppError::Busy("卸载清理交接标识无效，请重新打开卸载页面".to_string()).into());
    }
    if !state.uninstall_handoff_matches(&handoff_id) {
        return Err(AppError::Busy("卸载清理交接不存在、已过期或已完成".to_string()).into());
    }
    let policy = crate::confirmed_quit_policy_for(&app);
    if let QuitPolicy::Block {
        phase,
        reason_code,
        reason,
        kind,
    } = &policy
    {
        log::warn!(
            "uninstall handoff exit blocked phase={} reason_code={reason_code} kind={kind:?} reason={reason}",
            phase.as_str()
        );
        crate::dispatch_shell_event(
            &app,
            crate::app::shell::ShellEvent::QuitBlocked(policy.clone()),
        );
        return Err(AppError::Busy(reason.clone()).into());
    }
    if !state.authorize_uninstall_handoff(&handoff_id) {
        return Err(AppError::Busy("卸载清理交接不存在、已过期或已完成".to_string()).into());
    }
    log::info!("Windows uninstall handoff acknowledged by user");
    crate::exit_after_uninstall_handoff(&app);
    Ok(())
}

/// Windows-only: return the current user's default portable install root.
#[tauri::command]
pub fn win_default_install_root(state: State<'_, ManagerState>) -> Result<String, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    Ok(PersistedAppSettings::default().install_root)
}

/// Windows-only: open a system folder picker for the portable install root.
#[tauri::command]
pub async fn win_pick_install_dir(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<Option<String>, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let start_dir = dialog_start_dir(&PersistedAppSettings::load().install_root);
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择 Codex 安装位置")
            .set_directory(start_dir)
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| AppError::Internal(format!("读取选择的文件夹失败: {e}")))
            })
            .transpose()
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))??;
    match selected {
        Some(path) => tauri::async_runtime::spawn_blocking(move || {
            install_root_from_picked_dir(&path).map(Some)
        })
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into),
        None => Ok(None),
    }
}

/// Windows-only: let the user pick an existing portable/self-extracted Codex directory.
#[tauri::command]
pub async fn win_pick_existing_install(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<Option<InstalledWindowsCodex>, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let start_dir = dialog_start_dir(&PersistedAppSettings::load().install_root);
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择已安装的 Codex 位置")
            .set_directory(start_dir)
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| AppError::Internal(format!("读取选择的文件夹失败: {e}")))
            })
            .transpose()
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))??;
    selected
        .as_deref()
        .map(|path| detect_windows_install_at_path(Path::new(path)))
        .transpose()
        .map_err(Into::into)
}

/// Windows-only: adopt the user-selected Codex directory.
#[tauri::command]
pub fn win_adopt_path(
    state: State<'_, ManagerState>,
    path: String,
) -> Result<WinInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    crate::app::usb_guard::require_signed_codex_bundle("win-adopt-path")?;
    let _op = begin_guard(&state, OperationKind::Adopt)?;
    let settings = windows_domain_settings_for_persisted(&state);
    adopt_windows_path(&settings, Path::new(&path)).map_err(Into::into)
}

/// Windows-only: persist a validated portable install root.
#[tauri::command]
pub async fn win_set_install_root(
    state: State<'_, ManagerState>,
    path: String,
) -> Result<PersistedAppSettings, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let _op = begin_guard(&state, OperationKind::SetInstallRoot)?;
    let install_root =
        tauri::async_runtime::spawn_blocking(move || validate_install_root_path(&path))
            .await
            .map_err(|e| AppError::Internal(format!("join: {e}")))??;
    let mut settings = PersistedAppSettings::load();
    settings.install_root = install_root;
    settings.normalize();
    settings.save()?;
    let path = &settings.install_root;
    log::info!("set Windows install root path={path}");
    Ok(settings)
}

/// Windows-only: reset the remembered portable install root to the per-user default.
#[tauri::command]
pub async fn win_reset_install_root(
    state: State<'_, ManagerState>,
) -> Result<PersistedAppSettings, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let _op = begin_guard(&state, OperationKind::SetInstallRoot)?;
    let default_install_root = PersistedAppSettings::default().install_root;
    let install_root = tauri::async_runtime::spawn_blocking(move || {
        validate_install_root_path(&default_install_root)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))??;
    let mut settings = PersistedAppSettings::load();
    settings.install_root = install_root;
    settings.normalize();
    settings.save()?;
    let path = &settings.install_root;
    log::info!("reset Windows install root path={path}");
    Ok(settings)
}

/// macOS-only **destructive**: uninstall Codex. Requires explicit `confirm`.
/// `keep_codex_home` defaults true at the UI — `~/.codex` survives unless the
/// user opts out. Runs the blocking work off the main thread.
#[tauri::command]
pub async fn mac_uninstall(
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    keep_codex_home: bool,
) -> Result<MacUninstallReport, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    if !confirm {
        return Err(AppError::Internal("拒绝执行：卸载必须带显式 confirm".to_string()).into());
    }
    let _op = DetachedGuard::validate_with_phase(&state, token, OperationPhase::Committing)?;
    // Uninstall has no resumable cancellation protocol. Treat the whole worker
    // as point-of-no-return so every native/window quit path blocks until the
    // removal and ancillary bookkeeping have settled.
    tauri::async_runtime::spawn_blocking(move || uninstall_macos(keep_codex_home))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into)
}

/// Windows-only: background pre-download guard. It stages only when the user
/// enabled auto download and the current network passes the metered policy.
#[tauri::command]
pub async fn win_auto_stage_update(
    state: State<'_, ManagerState>,
    enabled: bool,
    allow_metered: bool,
) -> Result<WinAutoStageReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    if enabled {
        crate::app::usb_guard::require_signed_codex_bundle("win-auto-stage-update")?;
    }
    let _op = if enabled {
        match state.operations.begin(OperationKind::Update) {
            Ok(guard) => Some(guard),
            Err(OperationError::BusySameProcess(_) | OperationError::BusyOtherProcess) => {
                return Ok(auto_stage_busy_report(enabled, allow_metered));
            }
            Err(err) => return Err(AppError::from(err).into()),
        }
    } else {
        None
    };
    let endpoints = windows_endpoints_for_settings(&state)?;
    let settings = windows_domain_settings_for_persisted(&state);
    let install_mode = windows_install_mode_for_settings();
    let network = win_network_config_for_settings()?;
    tauri::async_runtime::spawn_blocking(move || {
        auto_stage_windows_update_with_install_mode_and_network(
            &endpoints,
            &settings,
            enabled,
            allow_metered,
            &install_mode,
            &network,
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Windows-only: request pausing an active background/manual download.
/// Partial bytes are left in place for the next resume-capable staging run.
#[tauri::command]
pub fn win_pause_download(
    state: State<'_, ManagerState>,
    operation_id: String,
) -> Result<bool, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    Ok(state
        .operations
        .request_pause(&OperationToken(operation_id), pause_windows_download))
}

/// Windows-only: request cancellation of an active background/manual download.
/// Partial bytes are discarded.
#[tauri::command]
pub fn win_cancel_download(
    state: State<'_, ManagerState>,
    operation_id: String,
) -> Result<bool, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    Ok(state
        .operations
        .request_cancellation(&OperationToken(operation_id), cancel_windows_download))
}

/// Windows-only: discard a PAUSED download. Drops the cached `.part` left for
/// resume when the user cancels from the paused state instead of resuming.
#[tauri::command]
pub fn win_discard_download(state: State<'_, ManagerState>) -> Result<(), CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    reject_uninstall_handoff(&state)?;
    discard_windows_download()?;
    state.operations.clear_paused_snapshot();
    Ok(())
}

/// Whether "launch at login" is currently enabled (off by default).
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, CommandError> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| AppError::Internal(format!("autostart: {e}")).into())
}

/// Enable/disable launch at login. The user opts in explicitly from Settings.
#[tauri::command]
pub fn set_autostart(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    enabled: bool,
) -> Result<(), CommandError> {
    reject_uninstall_handoff(&state)?;
    let mgr = app.autolaunch();
    let result = if enabled { mgr.enable() } else { mgr.disable() };
    result.map_err(|e| AppError::Internal(format!("autostart: {e}")).into())
}

// ── Codex UI themes ──────────────────────────────────────────────────────────
// CDP-injected theme packages (see crates/codex-theme-engine). Live try-on
// needs a debuggable Codex; the apply path restarts Codex with the loopback
// debug port and writes the native config.toml appearance sections.

/// Gate theme restarts with the same backend lease as ordinary Codex launch.
/// The lease closes the snapshot-check race and stays held through the relaunch.
fn begin_theme_restart_guard(state: &ManagerState) -> Result<OperationGuard, CommandError> {
    reject_uninstall_handoff(state)?;
    crate::app::usb_guard::require_signed_codex_bundle("codex-theme-restart")?;
    let operation = begin_guard(state, OperationKind::Launch)?;
    // Theme apply/off/try-on can close and relaunch Codex without going through
    // win_launch_codex. Validate the shared user config at this common restart
    // boundary before any theme flow mutates it or quits the running process.
    if matches!(state.target.os, OperatingSystem::Windows) {
        crate::app::codex_config_guard::ensure_user_config_ready()?;
    }
    Ok(operation)
}

#[tauri::command]
pub fn codex_theme_list() -> Vec<crate::app::codex_theme::ThemeListEntry> {
    let settings = PersistedAppSettings::load();
    crate::app::codex_theme::merged_theme_list(&settings)
}

#[tauri::command]
pub async fn codex_theme_status(
    state: State<'_, ManagerState>,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    let settings = PersistedAppSettings::load();
    Ok(state.codex_theme.status(&settings).await)
}

/// Live try-on against an already-debuggable Codex. Does not persist.
#[tauri::command]
pub async fn codex_theme_try_on(
    state: State<'_, ManagerState>,
    theme_ref: String,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_signed_codex_bundle("codex-theme-try-on")?;
    let settings = PersistedAppSettings::load();
    state.codex_theme.try_on(&settings, &theme_ref).await?;
    Ok(state.codex_theme.status(&settings).await)
}

/// Try-on that restarts Codex into debug mode first. Persists nothing —
/// the 保留 action on the try-on banner is what makes it stick.
#[tauri::command]
pub async fn codex_theme_try_on_restart(
    state: State<'_, ManagerState>,
    theme_ref: String,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    let _op = begin_theme_restart_guard(&state)?;
    let settings = PersistedAppSettings::load();
    state
        .codex_theme
        .try_on_with_restart(&settings, &theme_ref)
        .await?;
    Ok(state.codex_theme.status(&settings).await)
}

/// Persist the selection (the daemon keeps whatever is currently injected;
/// future launches through the manager apply it automatically).
#[tauri::command]
pub fn codex_theme_keep(
    state: State<'_, ManagerState>,
    theme_ref: String,
) -> Result<(), CommandError> {
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_signed_codex_bundle("codex-theme-keep")?;
    let mut settings = PersistedAppSettings::load();
    let dir = crate::app::codex_theme::resolve_theme_for_keep(&settings, &theme_ref)?;
    settings.codex_theme = Some(dir);
    settings.save()?;
    // The pre-try-on stash is consumed by keeping: the hot-imported native
    // theme is now the wanted state, and the long-term baseline (captured
    // before the first hot write) remains the path back to the user's own.
    crate::app::codex_theme::consume_try_on_stash();
    Ok(())
}

/// Delete a managed skin from the store directory. Refuses the active
/// selection (turn it off first — never silently un-theme a running Codex);
/// dev-checkout copies are never touched (delete only the store copy).
#[tauri::command]
pub fn codex_theme_delete(
    state: State<'_, ManagerState>,
    theme_ref: String,
) -> Result<(), CommandError> {
    reject_uninstall_handoff(&state)?;
    let settings = PersistedAppSettings::load();
    if settings.codex_theme.as_deref() == Some(theme_ref.as_str()) {
        return Err(AppError::Engine("请先关闭该主题再删除".to_string()).into());
    }
    crate::app::codex_theme::delete_store_skin(&settings, &theme_ref)?;
    Ok(())
}

/// Full apply: quiesce Codex → native appearance sections → relaunch with the
/// debug port → inject → persist the selection.
#[tauri::command]
pub async fn codex_theme_apply(
    state: State<'_, ManagerState>,
    theme_ref: String,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    let _op = begin_theme_restart_guard(&state)?;
    let settings = PersistedAppSettings::load();
    state
        .codex_theme
        .apply_with_restart(&settings, &theme_ref)
        .await?;
    let id = crate::app::codex_theme::resolve_theme_for_keep(&settings, &theme_ref)?;
    let mut updated = PersistedAppSettings::load();
    updated.codex_theme = Some(id);
    updated.save()?;
    Ok(state.codex_theme.status(&updated).await)
}

/// Pick a `.codexskin` archive and install it into the managed theme root.
/// Returns None when the user cancels the picker.
#[tauri::command]
pub async fn codex_theme_import(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<Option<codex_theme_engine::theme::ThemeSummary>, CommandError> {
    reject_uninstall_handoff(&state)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("导入 Codex 主题包")
            .add_filter("Codex Skin", &["codexskin", "zip"])
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok())
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?;
    let Some(archive) = picked else {
        return Ok(None);
    };
    import_codexskin_at(&archive).map(Some)
}

/// Install a `.codexskin` from an explicit path (drag-and-drop delivery).
#[tauri::command]
pub async fn codex_theme_import_path(
    state: State<'_, ManagerState>,
    path: String,
) -> Result<codex_theme_engine::theme::ThemeSummary, CommandError> {
    reject_uninstall_handoff(&state)?;
    tauri::async_runtime::spawn_blocking(move || import_codexskin_at(Path::new(&path)))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
}

fn import_codexskin_at(
    archive: &Path,
) -> Result<codex_theme_engine::theme::ThemeSummary, CommandError> {
    let themes_root = crate::app::codex_theme::store_dir(&PersistedAppSettings::load())?;
    let summary = codex_theme_engine::import::import_codexskin(archive, &themes_root)
        .map_err(|e| AppError::Engine(e.to_string()))?;
    log::info!(
        "imported codexskin id={} version={:?} from={}",
        summary.id,
        summary.meta.version,
        archive.display()
    );
    Ok(summary)
}

/// Cover preview of a theme as a data URL (None when the package ships no
/// preview). Served through invoke rather than the asset protocol so dev-dir
/// themes — whose location is user-chosen — need no protocol scope.
#[tauri::command]
pub fn codex_theme_preview(theme_ref: String) -> Result<Option<String>, CommandError> {
    let settings = PersistedAppSettings::load();
    Ok(crate::app::codex_theme::preview_data_url(
        &settings, &theme_ref,
    ))
}

/// The online skin catalog (skins.agentsmirror.com, published by
/// awesome-codex-skins CI).
#[tauri::command]
pub async fn codex_theme_catalog() -> Result<Vec<crate::app::codex_theme::CatalogSkin>, CommandError>
{
    tauri::async_runtime::spawn_blocking(crate::app::codex_theme::fetch_catalog)
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into)
}

/// Catalog cover preview as a data URL (relative path resolved only against
/// the pinned catalog origin).
#[tauri::command]
pub async fn codex_theme_catalog_preview(
    preview: String,
    version: String,
) -> Result<String, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::app::codex_theme::catalog_preview_data_url(&preview, &version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Download, sha256-verify and install one catalog skin.
#[tauri::command]
pub async fn codex_theme_install_online(
    state: State<'_, ManagerState>,
    skin_id: String,
) -> Result<codex_theme_engine::theme::ThemeSummary, CommandError> {
    reject_uninstall_handoff(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::app::codex_theme::install_from_catalog(&skin_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// Pick a new skin-store directory and migrate existing skins into it.
/// Returns None when the user cancels the picker. Conflicting ids stay at
/// the destination (never clobbered); a live daemon directive pointing into
/// the old store is re-based onto the new one.
#[tauri::command]
pub async fn codex_theme_pick_store_dir(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
) -> Result<Option<crate::app::codex_theme::StoreMigrationReport>, CommandError> {
    reject_uninstall_handoff(&state)?;
    let settings = PersistedAppSettings::load();
    let current = crate::app::codex_theme::store_dir(&settings)?;
    let start_dir = current.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择主题存储目录")
            .set_directory(start_dir)
            .blocking_pick_folder()
            .and_then(|path| path.into_path().ok())
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?;
    let Some(new_dir) = picked else {
        return Ok(None);
    };
    if new_dir == current {
        return Ok(None);
    }
    if new_dir.starts_with(&current) || current.starts_with(&new_dir) {
        return Err(AppError::Engine("新目录不能嵌套在当前主题目录内".to_string()).into());
    }

    let report = {
        let current = current.clone();
        let new_dir = new_dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::app::codex_theme::migrate_store(&current, &new_dir)
        })
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))??
    };

    let mut updated = PersistedAppSettings::load();
    updated.codex_theme_store_dir = Some(new_dir.to_string_lossy().into_owned());
    updated.save()?;
    state.codex_theme.rebase_directive(&current, &new_dir).await;
    log::info!(
        "skin store migrated from={} to={} moved={} skipped={}",
        current.display(),
        new_dir.display(),
        report.moved.len(),
        report.skipped.len()
    );
    Ok(Some(report))
}

/// Reveal the current skin store in Finder/Explorer.
#[tauri::command]
pub fn codex_theme_open_store(state: State<'_, ManagerState>) -> Result<(), CommandError> {
    reject_uninstall_handoff(&state)?;
    let dir = crate::app::codex_theme::store_dir(&PersistedAppSettings::load())?;
    let _ = std::fs::create_dir_all(&dir);
    open_dir_platform(&dir).map_err(|e| AppError::Internal(format!("打开主题目录失败: {e}")).into())
}

/// Turn the theme off. `full` additionally restores the original config.toml
/// appearance sections (restarting Codex plainly if it was running).
#[tauri::command]
pub async fn codex_theme_off(
    state: State<'_, ManagerState>,
    full: bool,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    if full {
        let _op = begin_theme_restart_guard(&state)?;
        state.codex_theme.off_full().await?;
        // Only the full restore clears the selection — off_live is a
        // session-scoped pause (SPEC §11): the native palette stays enabled
        // and the next themed launch re-injects.
        let mut settings = PersistedAppSettings::load();
        settings.codex_theme = None;
        settings.save()?;
        return Ok(state.codex_theme.status(&settings).await);
    }
    crate::app::usb_guard::require_signed_codex_bundle("codex-theme-off-live")?;
    state.codex_theme.turn_off_live().await?;
    let settings = PersistedAppSettings::load();
    Ok(state.codex_theme.status(&settings).await)
}

/// Cancel an unkept try-on: CSS off + hot-restore the stashed pre-try-on
/// native settings (or write them back to config.toml if Codex is stopped).
#[tauri::command]
pub async fn codex_theme_cancel(
    state: State<'_, ManagerState>,
) -> Result<crate::app::codex_theme::ThemeStatusReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_signed_codex_bundle("codex-theme-cancel")?;
    state.codex_theme.cancel_try_on().await?;
    let settings = PersistedAppSettings::load();
    Ok(state.codex_theme.status(&settings).await)
}

/// Switch the main window between the compact dashboard and the expanded
/// workbench. `width`/`height` are the frontend-remembered expanded size
/// (logical px); the echoed report carries what was actually applied after
/// clamping, so the frontend persists reality rather than the request.
#[tauri::command]
pub fn set_window_mode(
    app: tauri::AppHandle,
    mode: WindowMode,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<WindowModeReport, CommandError> {
    window_mode::apply_window_mode(&app, mode, width, height).map_err(Into::into)
}

/// Open an external http(s) URL in the user's default browser. Restricted to
/// http(s) so it can't be coerced into launching arbitrary local handlers.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), CommandError> {
    validate_external_http_url(&url)?;
    let host = redact_url(&url);
    log::info!("open external URL host={host}");
    open_external_url(&url).map_err(|e| AppError::Internal(format!("打开链接失败: {e}")).into())
}

#[tauri::command]
pub fn get_diagnostics(app: tauri::AppHandle, state: State<'_, ManagerState>) -> Diagnostics {
    log::info!("collecting diagnostics");
    crate::app::diagnostics::collect_diagnostics(&app, &state)
}

/// Lightweight native host architecture for package selection. Unlike full
/// diagnostics this performs no install, PowerShell, config, or log probes.
#[tauri::command]
pub fn get_host_architecture(state: State<'_, ManagerState>) -> String {
    state.target.arch.as_str().to_string()
}

#[tauri::command]
pub fn open_logs_dir(app: tauri::AppHandle) -> Result<(), CommandError> {
    let preferred = crate::app::logging::logs_dir(&app);
    let dir = preferred
        .filter(|candidate| candidate.is_dir())
        .unwrap_or_else(crate::app::logging::stable_uninstall_reports_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|error| AppError::Internal(format!("创建日志目录失败: {error}")))?;
    let path = dir.display();
    log::info!("open logs dir path={path}");
    open_dir_platform(&dir).map_err(|e| AppError::Internal(format!("打开日志目录失败: {e}")).into())
}

#[cfg(target_os = "macos")]
fn open_dir_platform(dir: &Path) -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn open_dir_platform(dir: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation: Vec<u16> = OsStr::new("open").encode_wide().chain([0]).collect();
    let target: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(format!("ShellExecuteW failed with code {result}"))
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_dir_platform(dir: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_codex_home() -> Result<(), CommandError> {
    let dir = paths::codex_home_dir()
        .ok_or_else(|| AppError::Internal("无法定位 Codex 数据目录".to_string()))?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.display();
    log::info!("open Codex home path={path}");
    open_dir_platform(&dir)
        .map_err(|e| AppError::Internal(format!("打开 Codex 数据目录失败: {e}")).into())
}

#[tauri::command]
pub fn log_frontend_error(payload: FrontendErrorPayload) {
    let kind = single_line(&payload.kind);
    let message = single_line(&payload.message);
    let stack = payload
        .stack
        .as_deref()
        .map(single_line)
        .unwrap_or_else(|| "none".to_string());
    let component_stack = payload
        .component_stack
        .as_deref()
        .map(single_line)
        .unwrap_or_else(|| "none".to_string());
    log::error!(
        "frontend error kind={kind} message={message} stack={stack} component_stack={component_stack}"
    );
}

fn single_line(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}

fn validate_external_http_url(url: &str) -> Result<(), AppError> {
    if url
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\\')
    {
        return Err(AppError::Internal("链接包含非法字符".to_string()));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(AppError::Internal("仅支持 http(s) 链接".to_string()));
    };
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err(AppError::Internal("仅支持 http(s) 链接".to_string()));
    }
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_matches('.');
    if host.is_empty() || host.contains('@') {
        return Err(AppError::Internal("链接缺少有效主机名".to_string()));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn open_external_url(url: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation: Vec<u16> = OsStr::new("open").encode_wide().chain([0]).collect();
    let target: Vec<u16> = OsStr::new(url).encode_wide().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(format!("ShellExecuteW failed with code {result}"))
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod open_url_tests {
    use super::validate_external_http_url;

    #[test]
    fn accepts_http_urls_with_query_delimiters() {
        assert!(validate_external_http_url("https://example.com/a?x=1&y=2").is_ok());
    }

    #[test]
    fn rejects_non_http_and_shell_sensitive_url_shapes() {
        for url in [
            "file:///C:/Windows/notepad.exe",
            "https://example.com/a b",
            "https://example.com\\evil",
            "https://user@example.com/",
            "https://",
        ] {
            assert!(
                validate_external_http_url(url).is_err(),
                "{url} should be rejected"
            );
        }
    }
}

/// Windows-only: classify the installed Codex (managed / external / none).
///
/// Async + `spawn_blocking` so AppX / PowerShell status probes cannot freeze the
/// WebView when enterprise policy or a hung AppX service stalls detection.
#[tauri::command]
pub async fn win_status(state: State<'_, ManagerState>) -> Result<WinInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let settings = windows_domain_settings_for_persisted(&state);
    tauri::async_runtime::spawn_blocking(move || win_install_status(&settings))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))
        .map_err(Into::into)
}

/// Windows-only: adopt the detected external install (after explicit consent).
///
/// Async + `spawn_blocking` so filesystem / provenance work stays off the UI thread.
#[tauri::command]
pub async fn win_adopt(state: State<'_, ManagerState>) -> Result<WinInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    crate::app::usb_guard::require_signed_codex_bundle("win-adopt")?;
    let _op = begin_guard(&state, OperationKind::Adopt)?;
    let settings = windows_domain_settings_for_persisted(&state);
    tauri::async_runtime::spawn_blocking(move || adopt_windows_install(&settings))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into)
}

/// Windows-only: open the installed Codex. A persisted theme uses the same
/// debuggable launch + injection path as macOS before falling back to a plain
/// launch when no theme is selected (or another operation owns the app).
///
/// Async + `spawn_blocking` so PowerShell AUMID activation (or portable spawn)
/// cannot freeze the WebView while the OS is cold-starting Codex. Errors still
/// surface to the UI after the blocking work finishes.
#[tauri::command]
pub async fn win_launch_codex(
    state: State<'_, ManagerState>,
    locale: Option<String>,
) -> Result<(), CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    // Acquire the shared backend operation lock before resolving the USB core
    // or applying startup compatibility changes. The guard remains live until
    // the selected launch path returns.
    let _op = begin_guard(&state, OperationKind::Launch)?;
    if crate::app::usb_guard::guard_required() {
        // Manager startup already performs the online revocation decision and
        // starts the USB watchdog. Re-running start_for_manager here tears
        // down and respawns that helper for every launch click, then the
        // launch implementation performs the full signed-bundle check again.
        // Keep this command boundary cheap and live; win_update::launch_codex
        // repeats the signed bundle verification immediately before spawn.
        crate::app::usb_guard::require_authorized()?;
        let authorization = crate::app::usb_guard::current_status();
        if !authorization.codex_allowed {
            return Err(AppError::Engine(
                authorization
                    .codex_block_reason
                    .unwrap_or(authorization.message),
            )
            .into());
        }
    }
    // Validate the user-owned Codex config before theme setup, catalog reload,
    // process handling, compatibility patches, or the final process spawn.
    // A valid BOM-only file is backed up and repaired; all other parse errors
    // stay untouched and are returned to the UI instead of becoming a silent
    // no-window startup failure.
    crate::app::codex_config_guard::ensure_user_config_ready()?;
    let persisted = PersistedAppSettings::load();
    let settings = windows_domain_settings_for_persisted(&state);
    let codex_locale = locale
        .as_deref()
        .map(str::trim)
        .filter(|locale| matches!(*locale, "zh-CN" | "zh-TW"))
        .map(str::to_owned);
    let catalog_reload_marker = if persisted.codex_theme.is_some() {
        crate::app::win_update::prepare_usb_model_catalog_for_theme_launch(&settings)?
    } else {
        None
    };
    let mut before_codex = crate::app::usb_guard::snapshot_processes("Codex.exe");
    if crate::app::usb_guard::guard_required() {
        before_codex.extend(crate::app::usb_guard::snapshot_usb_codex_entry_processes());
    }
    let before_chatgpt = crate::app::usb_guard::snapshot_processes("ChatGPT.exe");
    // A pending catalog reload must use the normal USB launch transaction.
    // The theme route can treat any listener on its fixed CDP port as success,
    // even when that listener belongs to an unrelated local Codex instance.
    // The normal route guarantees the authorized USB executable is launched;
    // the saved theme resumes on the next launch.
    if should_attempt_theme_codex_launch(
        persisted.codex_theme.is_some(),
        catalog_reload_marker.is_some(),
    ) {
        let themed = crate::app::codex_theme::launch_with_active_theme(
            &state.codex_theme,
            &persisted,
            codex_locale.as_deref(),
        )
        .await?;
        if themed {
            crate::app::win_update::complete_usb_model_catalog_launch(
                catalog_reload_marker.as_deref(),
            );
            tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                crate::app::usb_guard::register_new_codex_processes(&before_codex, &before_chatgpt);
            });
            return Ok(());
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = crate::app::win_update::launch_codex(&settings, codex_locale.as_deref());
        if result.is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            crate::app::usb_guard::register_new_codex_processes(&before_codex, &before_chatgpt);
        }
        result
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

fn should_attempt_theme_codex_launch(has_theme: bool, catalog_reload_pending: bool) -> bool {
    has_theme && !catalog_reload_pending
}

/// Return the latest local USB guard snapshot.  The command is read-only and
/// intentionally does not attempt to re-enroll or change the current lease.
#[tauri::command]
pub fn usb_guard_status() -> UsbGuardStatus {
    crate::app::usb_guard::current_status()
}

/// Read the signed 404token binding and perform an explicitly requested,
/// read-only model-directory check or gateway capability probe.
#[tauri::command]
pub async fn gateway_model_probe(
    request: crate::app::gateway_model_probe::GatewayModelProbeRequest,
) -> Result<crate::app::gateway_model_probe::GatewayModelProbeReport, CommandError> {
    crate::app::gateway_model_probe::run(request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub fn cancel_gateway_model_probe(request_id: String) -> bool {
    crate::app::gateway_model_probe::cancel(&request_id)
}

/// Read the currently authorized upstream model directory for automatic
/// media-model selection. The backend issues only GET /v1/models.
#[tauri::command]
pub async fn media_model_catalog(
) -> Result<crate::app::media_generation::MediaModelCatalog, CommandError> {
    crate::app::media_generation::catalog()
        .await
        .map_err(Into::into)
}

/// Read this user's saved, non-secret media model routing values.
#[tauri::command]
pub fn media_model_settings() -> crate::app::media_generation::LocalMediaModelSettings {
    crate::app::media_generation::local_model_settings()
}

/// Read, selectively extend, or reset the manager-owned native model catalog.
#[tauri::command]
pub async fn manage_native_model_catalog(
    request: crate::app::model_catalog::NativeModelCatalogActionRequest,
) -> Result<crate::app::model_catalog::NativeModelCatalogActionReport, CommandError> {
    crate::app::model_catalog::manage_native_model_catalog(request)
        .await
        .map_err(Into::into)
}

/// Windows-only: install the bundled Codex Switch configurator and start it.
///
/// The USB build keeps `ccswitch/CodexSwitch.exe` beside the setup executable;
/// a packaged Tauri build may instead expose the same directory through its
/// resource directory.  In USB mode the verified sibling payload is launched
/// directly from the removable volume (there is no hidden C: copy); normal
/// non-USB builds may use a per-user local copy.  A missing configurator is
/// returned as a non-fatal report and never rolls back a successful Codex
/// installation.
#[tauri::command]
pub async fn install_and_launch_ccswitch(
    app: AppHandle,
    state: State<'_, ManagerState>,
) -> Result<CcswitchLaunchReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_usb_content_access("ccswitch-launch")?;
    tauri::async_runtime::spawn_blocking(move || install_and_launch_ccswitch_impl(&app))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into)
}

/// Restore or open the managed CCSwitch window after the one-time bootstrap
/// hand-off. This command never applies the automatic background hide policy.
#[tauri::command]
pub async fn open_ccswitch(
    app: AppHandle,
    state: State<'_, ManagerState>,
) -> Result<CcswitchLaunchReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    crate::app::usb_guard::require_usb_content_access("ccswitch-open")?;
    tauri::async_runtime::spawn_blocking(move || crate::app::ccswitch::open(&app))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(Into::into)
}

/// Read the signed USB account binding and the in-memory 404token session.
#[tauri::command]
pub fn account_status() -> Result<AccountStatus, CommandError> {
    crate::app::account::status().map_err(Into::into)
}

/// Login with the password belonging to the signed USB account. The password
/// is consumed by this call and is never persisted or returned to the UI.
#[tauri::command]
pub async fn account_login(password: String) -> Result<AccountStatus, CommandError> {
    crate::app::account::login(password)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn account_refresh() -> Result<AccountStatus, CommandError> {
    crate::app::account::refresh().await.map_err(Into::into)
}

#[tauri::command]
pub async fn account_redeem(code: String) -> Result<AccountStatus, CommandError> {
    crate::app::account::redeem(code).await.map_err(Into::into)
}

/// Inspect the bundled Codex capability pack without changing the local
/// profile.  The pack is optional in development builds, but a release USB
/// bundle is expected to ship it under content/codex-starter-kit.
#[tauri::command]
pub async fn capability_pack_status(app: AppHandle) -> Result<CapabilityPackReport, CommandError> {
    crate::app::usb_guard::require_usb_content_access("capability-pack-status")?;
    tauri::async_runtime::spawn_blocking(move || capability_pack_status_impl(&app))
        .await
        .map_err(|error| AppError::Internal(format!("join: {error}")))?
        .map_err(Into::into)
}

/// Install or repair the complete local capability pack.  All connection
/// fields are opt-in; empty fields never enable Feishu, API adapters, or paid
/// requests.  Secrets are passed to the child only for this invocation and,
/// after a successful install, persisted to the current user's environment
/// rather than to the USB package or Codex TOML.
#[tauri::command]
pub async fn install_capability_pack(
    app: AppHandle,
    state: State<'_, ManagerState>,
    options: CapabilityPackInstallOptions,
) -> Result<CapabilityPackReport, CommandError> {
    let _op = begin_guard(&state, OperationKind::Install)?;
    crate::app::usb_guard::require_usb_content_access("capability-pack-install")?;
    tauri::async_runtime::spawn_blocking(move || install_capability_pack_impl(&app, options))
        .await
        .map_err(|error| AppError::Internal(format!("join: {error}")))?
        .map_err(Into::into)
}

/// Inspect the independent, unsigned Skill Hub overlay. This deliberately
/// does not require USB presence or USB signature authorization.
#[tauri::command]
pub async fn skill_repo_check_update(
    app: AppHandle,
    state: State<'_, ManagerState>,
) -> Result<SkillRepoUpdateInfo, CommandError> {
    reject_uninstall_handoff(&state)?;
    let operations = state.operations.clone();
    check_skill_repo_update(&app, &operations).await
}

/// Read the installed Product Design plugin state from the isolated USB
/// CODEX_HOME without performing network access or modifying files.
#[tauri::command]
pub async fn skill_repo_product_design_status(
    state: State<'_, ManagerState>,
) -> Result<Option<u32>, CommandError> {
    reject_uninstall_handoff(&state)?;
    tauri::async_runtime::spawn_blocking(
        crate::app::capability_pack::product_design_plugin_status,
    )
    .await
    .map_err(|error| AppError::Internal(format!("join: {error}")))
    .map_err(CommandError::from)
}

/// Apply the exact identity returned by a preceding Skill Hub check.
#[tauri::command]
pub async fn skill_repo_apply_update(
    app: AppHandle,
    state: State<'_, ManagerState>,
    expected: SkillRepoExpected,
) -> Result<SkillRepoApplyReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    let product_design_skill_count = if crate::app::usb_guard::guard_required() {
        let plugin_app = app.clone();
        Some(
            tauri::async_runtime::spawn_blocking(move || {
                crate::app::capability_pack::install_product_design_plugin(&plugin_app)
            })
            .await
            .map_err(|error| AppError::Internal(format!("join: {error}")))?
            .map_err(CommandError::from)?,
        )
    } else {
        None
    };
    let operations = state.operations.clone();
    let mut report = apply_skill_repo_update(&app, &operations, expected).await?;
    report.product_design_skill_count = product_design_skill_count;
    Ok(report)
}

/// Roll back the last complete independent Skill Hub generation.
#[tauri::command]
pub async fn skill_repo_rollback(
    app: AppHandle,
    state: State<'_, ManagerState>,
) -> Result<SkillRepoRollbackReport, CommandError> {
    reject_uninstall_handoff(&state)?;
    let operations = state.operations.clone();
    rollback_skill_repo_update(&app, &operations).await
}

/// Windows-only: guarded execution. Requires explicit confirmation, stages and
/// verifies the MSIX first, then attempts Add-AppxPackage without elevation or
/// policy changes. Reports portable fallback need transparently.
#[tauri::command]
pub async fn win_perform_update(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    install_root: Option<String>,
    expected: WinPerformExpectation,
    resume_kind: OperationResumeKind,
) -> Result<WinPerformReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    crate::app::usb_guard::require_signed_codex_bundle("win-perform-update")?;
    if !confirm {
        return Err(
            AppError::Internal("拒绝执行：Windows 更新必须带显式 confirm".to_string()).into(),
        );
    }
    if crate::app::usb_guard::guard_required() {
        // Recheck immediately before the install path: the UI preflight and
        // background staging may have happened earlier or in another process.
        require_usb_install_preflight().await?;
    }
    let mut op = DetachedGuard::validate_tracked(&state, token)?;
    op.set_phase(OperationPhase::Preparing);
    let endpoints = windows_endpoints_for_settings(&state)?;
    let mut settings = windows_domain_settings_for_persisted(&state);
    // Validate (but don't yet persist) an explicitly chosen install root: it
    // only becomes the remembered default after the install actually succeeds,
    // so a failed or cancelled attempt never changes the user's saved location.
    let pending_install_root = match install_root {
        Some(raw) => {
            let validated =
                tauri::async_runtime::spawn_blocking(move || validate_install_root_path(&raw))
                    .await
                    .map_err(|e| AppError::Internal(format!("join: {e}")))??;
            settings.install_root = validated.clone();
            Some(validated)
        }
        None => None,
    };
    if let Some(token) = op.token_clone() {
        state
            .operations
            .set_resume_context(
                &token,
                OperationResumeContext {
                    kind: resume_kind,
                    install_root: pending_install_root.clone(),
                    expectation: OperationResumeExpectation::Windows {
                        current_version: expected.current_version.clone(),
                        target_version: expected.latest_version.clone(),
                        package_moniker: expected.package_moniker.clone(),
                        route: expected.route.clone(),
                    },
                },
            )
            .map_err(AppError::from)?;
    }
    let install_mode = windows_install_mode_for_settings();
    let network = win_network_config_for_settings()?;
    let progress_app = app.clone();
    let ops = op.operations();
    let phase_token = op.token_clone();
    let progress_token = phase_token.clone();
    let progress_ops = ops.clone();
    let phase_app = progress_app.clone();
    let phase_ops = ops.clone();
    let phase_event_token = phase_token.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let report = move |p: WinDownloadProgress| {
            if let Some(token) = progress_token.as_ref() {
                emit_op_download_progress(
                    &progress_app,
                    &progress_ops,
                    token,
                    "win://download-progress",
                    p.downloaded,
                    p.total,
                    p.source,
                );
            } else {
                let _ = progress_app.emit("win://download-progress", p);
            }
        };
        let phase_hook = move |phase: OperationPhase| {
            if let Some(token) = phase_event_token.as_ref() {
                emit_op_phase(
                    &phase_app,
                    &phase_ops,
                    token,
                    "win://download-progress",
                    phase,
                );
            }
        };
        let evidence_hook = |evidence: OperationEvidence| {
            if let Some(token) = phase_token.as_ref() {
                let result = match evidence {
                    OperationEvidence::MutationStarted => ops.mark_mutation_started(token),
                    OperationEvidence::MutationRolledBack => ops.mark_mutation_rolled_back(token),
                    OperationEvidence::OutcomeAmbiguous => ops.mark_outcome_ambiguous(token),
                };
                if let Err(error) = result {
                    log::error!("failed to record Windows operation evidence: {error}");
                }
            }
        };
        perform_windows_update_with_install_mode_network_and_phase(
            &endpoints,
            &settings,
            confirm,
            &install_mode,
            Some(expected),
            &report,
            &network,
            Some(&phase_hook),
            Some(&evidence_hook),
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?;
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            op.confirm_pause_if_cancelled(&error);
            invalidate_retained_pause_if_stale(&state.operations, &error);
            return Err(error.into());
        }
    };
    if let Some(root) = pending_install_root {
        if report.success {
            let mut saved = PersistedAppSettings::load();
            saved.install_root = root;
            saved.normalize();
            saved.save()?;
        }
    }
    if report.success {
        if let Err(error) = crate::app::codex_agents_hint::ensure_portable_usb_hint() {
            log::warn!("could not add the USB Codex mojibake hint to AGENTS.md: {error}");
        }
        // The staged MSIX was consumed by the install — drop the cache. A failed
        // or cancelled perform leaves it so the next run (or a resume) reuses the
        // partial/full artifact instead of re-downloading. `stage`/`auto_stage`
        // never clear it: they're pre-downloads whose whole point is to be reused.
        // Best-effort: the stale sweep reclaims a leftover, so a cleanup failure
        // must not turn a successful install into an error.
        let _ = crate::app::staging::clear_download_cache();
    }
    op.mark_succeeded();
    Ok(report)
}

/// Windows-only historical/offline MSIX install. The package is always hashed,
/// Authenticode-verified and identity-checked first. `install_to_usb` selects
/// the fixed USB edition: verified program files are transactionally extracted
/// to `<bundle>\apps\Codex`, while mutable user data stays in the normal local
/// Windows profile. No AppX volume, WpSystem or StateRepository mutation is used.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn win_install_historical_release(
    app: tauri::AppHandle,
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    release_tag: String,
    asset_name: String,
    local_path: Option<String>,
    architecture: String,
    block_updates: bool,
    install_root: Option<String>,
    expected: HistoricalWinExpectation,
    force_msix: Option<bool>,
    install_to_usb: Option<bool>,
) -> Result<WinPerformReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    let install_to_usb = is_usb_install_path(
        install_to_usb.unwrap_or(false),
        crate::app::usb_guard::guard_required(),
    );
    if install_to_usb {
        crate::app::usb_guard::require_signed_codex_install_bundle("win-install-historical")?;
        require_usb_install_preflight().await?;
    } else {
        crate::app::usb_guard::require_signed_codex_bundle("win-install-historical")?;
    }
    if !confirm {
        return Err(AppError::Internal(
            "拒绝执行：Windows 历史版本安装必须带显式 confirm".to_string(),
        )
        .into());
    }
    let architecture = parse_release_architecture(&architecture)?;
    validate_historical_architecture(&state.target, architecture)?;
    let mut op = DetachedGuard::validate_tracked(&state, token)?;
    op.set_phase(OperationPhase::Preparing);
    // Cover Release resolution as well as the installer itself so an error
    // before the inner WinAbortGuard is created cannot leak a cancel latch into
    // the next operation.
    let _abort_scope = WinAbortGuard::new();
    let mut settings = windows_domain_settings_for_persisted(&state);
    // Match the normal fresh-install flow: validate the one-shot destination
    // before use, but remember it only after the historical install succeeds.
    let pending_install_root = if install_to_usb {
        // Ignore renderer-selected destinations in USB mode. The exact target
        // is derived from the authenticated bundle and rejects reparse points.
        let usb_root = crate::app::usb_guard::authorized_usb_codex_root()?;
        // Fail before staging a 700+ MB package if this machine redirects the
        // normal Codex profile back onto the product USB.
        crate::app::usb_guard::local_data_paths()?;
        let raw = usb_root.to_string_lossy().into_owned();
        let validated =
            tauri::async_runtime::spawn_blocking(move || validate_install_root_path(&raw))
                .await
                .map_err(|e| AppError::Internal(format!("join: {e}")))??;
        settings.install_root = validated.clone();
        Some(validated)
    } else {
        match install_root {
            Some(raw) => {
                let validated =
                    tauri::async_runtime::spawn_blocking(move || validate_install_root_path(&raw))
                        .await
                        .map_err(|e| AppError::Internal(format!("join: {e}")))??;
                settings.install_root = validated.clone();
                Some(validated)
            }
            None => None,
        }
    };
    let resume_install_root = pending_install_root.clone();
    // The offline USB flow must never inherit a previous portable preference
    // from the full manager. Keep the optional flag at the command boundary
    // so existing online/historical callers retain their configured route.
    let install_mode = if install_to_usb {
        crate::app::win_update::USB_PORTABLE_INSTALL_MODE.to_string()
    } else if force_msix.unwrap_or(false) {
        FIXED_MSIX_INSTALL_MODE.to_string()
    } else {
        windows_install_mode_for_settings()
    };
    let local_path = local_path.map(PathBuf::from);
    let was_local = local_path.is_some();
    let network = if was_local {
        codex_win_engine::NetworkConfig::system()
    } else {
        win_network_config_for_settings()?
    };
    // The v0.5.20 USB architecture is an ordinary removable program tree.
    // This must stay `None`; a non-None value re-enters the fragile AppXVolume
    // and WpSystem path retained only for legacy cleanup compatibility.
    let appx_volume_root = None;
    let progress_app = app.clone();
    let ops = op.operations();
    let phase_token = op.token_clone();
    let progress_token = phase_token.clone();
    let progress_ops = ops.clone();
    let phase_app = progress_app.clone();
    let phase_ops = ops.clone();
    let phase_event_token = phase_token.clone();
    let previous_block_updates = PersistedAppSettings::load().disable_codex_self_updates;
    let policy_transition =
        (previous_block_updates != block_updates).then_some(SelfUpdatePolicyTransition {
            previous_disabled: previous_block_updates,
            requested_disabled: block_updates,
        });
    let policy_attempted = Arc::new(AtomicBool::new(false));
    let policy_attempted_worker = Arc::clone(&policy_attempted);
    let policy_install_state =
        Arc::new(AtomicU8::new(HistoricalPolicyInstallState::PreCommit as u8));
    let policy_install_state_worker = Arc::clone(&policy_install_state);
    let report = tauri::async_runtime::spawn_blocking(move || {
        let fetch = |url: &str| {
            codex_win_engine::fetch_text_with_network(url, &network)
                .map_err(|e| AppError::Engine(e.to_string()))
        };
        let resolved = if let Some(path) = local_path.as_ref() {
            let resolved = resolve_windows_local_package(path, architecture)?;
            if resolved.name != asset_name {
                return Err(AppError::StaleExpectation(format!(
                    "本地 MSIX 文件名在确认后发生变化（确认时 {}，现在 {}），请重新选择并确认",
                    asset_name, resolved.name
                )));
            }
            resolved
        } else {
            let resolved = resolve_release_asset(
                &release_tag,
                &asset_name,
                ReleasePlatform::Windows,
                architecture,
                &fetch,
            )?;
            verify_release_checksum(&resolved, &fetch)?;
            resolved
        };
        if let Some(token) = phase_token.as_ref() {
            ops.set_historical_context(
                token,
                historical_resume_context(
                    &resolved,
                    local_path.as_deref(),
                    block_updates,
                    HistoricalResumeExpectation::Windows {
                        current_path: expected.current_path.clone(),
                        current_version: expected.current_version.clone(),
                        current_source: expected.current_source.clone(),
                    },
                    resume_install_root.as_deref(),
                ),
            )
            .map_err(AppError::from)?;
        }
        let progress = move |p: WinDownloadProgress| {
            if let Some(token) = progress_token.as_ref() {
                emit_op_download_progress(
                    &progress_app,
                    &progress_ops,
                    token,
                    "win://download-progress",
                    p.downloaded,
                    p.total,
                    p.source,
                );
            }
        };
        let phase_hook = move |phase: OperationPhase| {
            if let Some(token) = phase_event_token.as_ref() {
                emit_op_phase(
                    &phase_app,
                    &phase_ops,
                    token,
                    "win://download-progress",
                    phase,
                );
            }
        };
        let evidence_hook = |evidence: OperationEvidence| {
            record_windows_policy_install_state(&policy_install_state_worker, evidence);
            if let Some(token) = phase_token.as_ref() {
                let result = match evidence {
                    OperationEvidence::MutationStarted => ops.mark_mutation_started(token),
                    OperationEvidence::MutationRolledBack => ops.mark_mutation_rolled_back(token),
                    OperationEvidence::OutcomeAmbiguous => ops.mark_outcome_ambiguous(token),
                };
                if let Err(error) = result {
                    log::error!("failed to record Windows historical install evidence: {error}");
                }
            }
        };
        let before_commit = || {
            if install_to_usb {
                let authorization = crate::app::usb_guard::require_signed_codex_install_bundle(
                    "historical-usb-precommit",
                )?
                .ok_or_else(|| {
                    AppError::Engine("USB 历史安装提交前未获得当前 bundle 授权".to_string())
                })?;
                if !codex_win_engine::same_windows_path(
                    Path::new(&settings.install_root),
                    &authorization.codex_root,
                ) {
                    return Err(AppError::StaleExpectation(
                        "USB 历史安装提交前目标目录已变化，请重新检查后再试".to_string(),
                    ));
                }
            }
            policy_attempted_worker.store(true, Ordering::SeqCst);
            persist_historical_self_update_policy(block_updates)
        };
        install_windows_historical_release(
            resolved,
            local_path,
            &settings,
            &install_mode,
            expected,
            &progress,
            &network,
            Some(&phase_hook),
            Some(&evidence_hook),
            policy_transition,
            Some(&before_commit),
            appx_volume_root,
        )
    })
    .await;
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            // Add-AppxPackage and the portable swap are both external mutation
            // boundaries. A panicked worker cannot prove that neither landed.
            HistoricalPolicyInstallState::OutcomeUnknown.store(&policy_install_state);
            Err(AppError::Internal(format!("join: {error}")))
        }
    };
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            op.confirm_pause_if_cancelled(&error);
            invalidate_retained_pause_if_stale(&state.operations, &error);
            let error = if previous_block_updates != block_updates
                && policy_attempted.load(Ordering::SeqCst)
                && HistoricalPolicyInstallState::load(&policy_install_state)
                    .policy_may_be_restored()
            {
                rollback_historical_policy_after_failure(previous_block_updates, error)
            } else {
                error
            };
            return Err(error.into());
        }
    };
    if let Some(root) = pending_install_root {
        if report.success && report.portable.is_some() && !install_to_usb {
            let mut saved = PersistedAppSettings::load();
            saved.install_root = root;
            saved.normalize();
            saved.save()?;
        }
    }
    if report.success && install_to_usb {
        if let Err(error) = crate::app::codex_agents_hint::ensure_portable_usb_hint() {
            log::warn!("could not add the USB Codex mojibake hint to AGENTS.md: {error}");
        }
        // A reactivated USB stays without a lease throughout the reinstall.
        // Only after the install has succeeded does startup repeat the signed
        // status check and full bundle validation before publishing an active
        // guard lease.
        if let Err(error) = crate::app::usb_guard::start_for_manager() {
            log::error!("USB authorization refresh after reactivation install failed: {error}");
        }
    }
    if report.success && !was_local {
        let _ = crate::app::staging::clear_download_cache();
    }
    op.mark_succeeded();
    Ok(report)
}

/// Windows-only: guarded uninstall. Only removes installs recorded as managed
/// by this app. User data is preserved unless `purge_user_data` is true.
#[tauri::command]
pub async fn win_uninstall(
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    purge_user_data: bool,
) -> Result<WinUninstallReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    if !confirm {
        return Err(
            AppError::Internal("拒绝执行：Windows 卸载必须带显式 confirm".to_string()).into(),
        );
    }
    let _op = DetachedGuard::validate_with_phase(&state, token, OperationPhase::Committing)?;
    // Remove-AppxPackage / portable tree removal must not be killed mid-call.
    let settings = windows_domain_settings_for_persisted(&state);
    let transaction_scope = PathBuf::from(settings.install_root.clone());
    let mut report = tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) =
            crate::app::install_tx::reconcile_for_uninstall_scoped(Some(&transaction_scope))
        {
            log::warn!("pre-uninstall install transaction reconciliation skipped: {error}");
        }
        // A user can have a stale journal from an older C: install while the
        // active install is on a removable drive.  The scoped pass above is
        // correct for ordinary cleanup, but the detached cleanup gate later
        // checks the shared manager journal directory as a whole.  Reconcile
        // only the bookkeeping-only, no-recovery-path shape across all roots
        // as well; this never renames or deletes an install tree.
        if let Err(error) = crate::app::install_tx::reconcile_for_uninstall() {
            log::warn!("pre-uninstall global install transaction reconciliation skipped: {error}");
        }
        uninstall_windows_codex(&settings, confirm, purge_user_data)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(CommandError::from)?;
    if report.outcome.primary_ok {
        append_native_model_catalog_cleanup(&mut report);
    }
    Ok(report)
}

/// Result of the fixed USB build's combined cleanup.  Codex and CCSwitch are
/// reported separately so a failed CCSwitch file lock never obscures the fact
/// that Codex itself was already removed (and vice versa).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinBundleUninstallReport {
    pub success: bool,
    pub codex: Option<WinUninstallReport>,
    pub codex_error: Option<String>,
    pub ccswitch: Option<CcswitchUninstallReport>,
    pub ccswitch_error: Option<String>,
    pub message: String,
    pub notes: Vec<String>,
    pub purged_user_data: bool,
    pub purged_ccswitch_data: bool,
    /// Primary uninstall is complete; the detached helper still has to remove
    /// the manager's open files after process exit. Never label this "all clean".
    pub cleanup_pending: bool,
    /// The USB applications were removed, but shared Manager/recovery data
    /// remains because another installation still owns or needs it.
    #[serde(default)]
    pub cleanup_deferred: bool,
    pub cleanup_report_path: Option<String>,
    /// Opaque helper nonce required by the explicit exit acknowledgement.
    #[serde(default)]
    pub cleanup_handoff_id: Option<String>,
    #[serde(default)]
    pub outcome: OperationOutcome,
}

fn append_native_model_catalog_cleanup(report: &mut WinUninstallReport) {
    match crate::app::model_catalog::cleanup() {
        Ok(true) => report
            .notes
            .push("已清理本管理器生成的 Codex 原生模型目录".to_string()),
        Ok(false) => {}
        Err(error) => {
            let detail = format!("Codex 原生模型目录清理未完成：{error}");
            report.notes.push(detail.clone());
            report.outcome.cleanup = StepOutcome::failed(detail.clone());
            report.outcome.push_warning(detail.clone());
            report.outcome.push_recovery(recovery::CLEANUP_METADATA);
            report.message = format!(
                "{}（主卸载已完成，但模型目录清理未完成；请查看详情）",
                report.message
            );
        }
    }
}

fn combine_bundle_uninstall(
    codex_result: Result<WinUninstallReport, AppError>,
    ccswitch_result: Result<CcswitchUninstallReport, AppError>,
    purge_user_data: bool,
    purge_ccswitch_data: bool,
) -> WinBundleUninstallReport {
    let (codex, codex_error) = match codex_result {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let (ccswitch, ccswitch_error) = match ccswitch_result {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(error.to_string())),
    };

    let mut notes = Vec::new();
    let mut messages = Vec::new();
    let mut outcome = if let Some(report) = codex.as_ref() {
        report.outcome.clone()
    } else {
        OperationOutcome::primary_failed(
            "unknown",
            codex_error
                .as_deref()
                .unwrap_or("Codex 卸载失败，无法确认磁盘状态"),
        )
    };

    if let Some(report) = codex.as_ref() {
        messages.push(format!("Codex：{}", report.message));
        if let Some(msix) = report.msix.as_ref().filter(|msix| !msix.success) {
            if let Some(raw) = msix
                .raw_error
                .as_deref()
                .filter(|raw| !raw.trim().is_empty())
            {
                if !report.message.contains(raw) {
                    messages.push(format!("AppX 原始错误：{raw}"));
                }
            }
        }
        notes.extend(report.notes.iter().map(|note| format!("Codex：{note}")));
    }
    if let Some(error) = codex_error.as_deref() {
        messages.push(format!("Codex 卸载失败：{error}"));
        notes.push(format!("Codex：{error}"));
    }

    if let Some(report) = ccswitch.as_ref() {
        messages.push(format!("CCSwitch：{}", report.message));
        notes.extend(report.notes.iter().map(|note| format!("CCSwitch：{note}")));
        if !report.success {
            let detail = report.message.clone();
            if outcome.primary_ok {
                outcome.primary_ok = false;
                outcome.app_state = "unknown".to_string();
                outcome.cleanup = StepOutcome::failed(detail.clone());
            }
            outcome.push_warning(detail);
        }
    }
    if let Some(error) = ccswitch_error.as_deref() {
        messages.push(format!("CCSwitch 卸载失败：{error}"));
        notes.push(format!("CCSwitch：{error}"));
        if outcome.primary_ok {
            outcome.primary_ok = false;
            outcome.app_state = "unknown".to_string();
            outcome.cleanup = StepOutcome::failed(error);
        } else {
            outcome.push_warning(error);
        }
    }

    let codex_success = codex.as_ref().is_some_and(|report| report.success);
    let ccswitch_success = ccswitch.as_ref().is_some_and(|report| report.success);
    let success = codex_success && ccswitch_success;
    if messages.is_empty() {
        messages.push("未检测到 Codex 或 CCSwitch 安装。".to_string());
    }
    if !success {
        messages.push("部分清理未完成；关闭相关进程后可再次执行本操作。".to_string());
    }

    let purged_codex_data =
        purge_user_data && codex.as_ref().is_some_and(|report| report.purged_user_data);
    let purged_ccswitch_data = purge_ccswitch_data
        && ccswitch
            .as_ref()
            .is_some_and(|report| report.purged_user_data);

    WinBundleUninstallReport {
        success,
        codex,
        codex_error,
        ccswitch,
        ccswitch_error,
        message: messages.join("；"),
        notes,
        purged_user_data: purged_codex_data,
        purged_ccswitch_data,
        cleanup_pending: false,
        cleanup_deferred: false,
        cleanup_report_path: None,
        cleanup_handoff_id: None,
        outcome,
    }
}

fn fail_usb_codex_machine_cleanup(report: &mut WinUninstallReport, detail: String) {
    report.success = false;
    report.message = format!(
        "{}；机器级 AppX 残留未确认清理完成",
        report.message.trim_end_matches('。')
    );
    report.notes.push(detail.clone());
    report.outcome.cleanup = StepOutcome::failed(detail.clone());
    report.outcome.push_warning(detail);
    report.outcome.push_recovery(recovery::CLEANUP_METADATA);
}

fn merge_usb_machine_cleanup(
    report: &mut WinUninstallReport,
    cleanup: codex_win_engine::MsixRemoveReport,
) {
    report
        .notes
        .push(format!("机器级 AppX 清理：{}", cleanup.message));
    report.notes.extend(cleanup.notes.iter().cloned());
    if cleanup.success {
        // Keep the ordinary per-user removal report when one exists; otherwise
        // expose the machine cleanup result through the same public field.
        if report.msix.is_none() {
            report.msix = Some(cleanup);
        }
        return;
    }
    if let Some(raw) = cleanup
        .raw_error
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        report.notes.push(format!("机器级 AppX 原始错误：{raw}"));
    }
    let detail = format!("机器级 AppX 清理失败：{}", cleanup.message);
    report.msix = Some(cleanup);
    fail_usb_codex_machine_cleanup(report, detail);
}

fn bundle_can_start_final_cleanup(report: &WinBundleUninstallReport) -> bool {
    report.success
        && !report.cleanup_deferred
        && report.outcome.primary_ok
        && !report.outcome.is_partial()
        && report.outcome.steps_are_terminal()
        && report.outcome.recovery_actions.is_empty()
}

fn defer_shared_bundle_cleanup(report: &mut WinBundleUninstallReport, detail: &str) {
    report.cleanup_deferred = true;
    let note = format!(
        "U 盘版 Codex 与 CCSwitch 已完成卸载；检测到其他安装或共享恢复记录，未删除共享管理器数据。请保留该恢复材料并在对应安装中处理。详情：{detail}"
    );
    report.notes.push(note.clone());
    report.message = format!("{}；{}", report.message, note);
    report.outcome.cleanup =
        StepOutcome::skipped("共享管理器数据和其他安装的恢复记录已保留，最终清理延后");
}

/// Keep the original primary errors even when later policy/helper steps add
/// notes. `success` describes primary work; only the detached finalizer can
/// report that the manager's own files have actually been removed.
fn finish_bundle_uninstall_report(report: &mut WinBundleUninstallReport) {
    // An unknown/pending ancillary state is not a successful uninstall. Keep
    // the raw state in the report, but turn the aggregate result into an
    // actionable failure instead of allowing it to reach the pending-exit
    // branch or be mislabeled as primaryCompleted.
    if report.success && !report.cleanup_pending && !report.outcome.steps_are_terminal() {
        report.success = false;
        report
            .outcome
            .push_warning("卸载收尾状态尚未完成，未启动最终清理");
    }
    let phase = if !report.outcome.primary_ok {
        "primaryFailed"
    } else if !report.success
        || report.outcome.is_partial()
        || !report.outcome.steps_are_terminal()
        || !report.outcome.recovery_actions.is_empty()
    {
        "partialFailure"
    } else if report.cleanup_pending {
        "awaitingFinalCleanup"
    } else {
        "primaryCompleted"
    };
    if !report.success
        || report.outcome.is_partial()
        || !report.outcome.steps_are_terminal()
        || !report.outcome.recovery_actions.is_empty()
    {
        let mut details = vec![report.message.clone()];
        if !report.outcome.recovery_actions.is_empty() {
            details.push(format!(
                "尚待恢复步骤：{}",
                report.outcome.recovery_actions.join(", ")
            ));
        }
        for detail in report
            .notes
            .iter()
            .chain(report.outcome.warnings.iter())
            .chain(report.outcome.provenance.detail.iter())
            .chain(report.outcome.cleanup.detail.iter())
        {
            if !detail.trim().is_empty() && !details.iter().any(|entry| entry.contains(detail)) {
                details.push(detail.clone());
            }
        }
        let summary = if report.outcome.primary_ok {
            "Codex 主卸载已完成，但联合卸载或附属清理未全部完成；仍存在的诊断/恢复记录已保留，已经成功删除的所选数据不会恢复。"
        } else {
            "Codex 主卸载未完成；仍存在的诊断/恢复记录已保留，已经成功删除的所选数据不会恢复。"
        };
        report.message = format!("{summary}\n{}", details.join("\n"));
        log::warn!(
            "USB uninstall phase={phase} primary_ok={} success={} detail={}",
            report.outcome.primary_ok,
            report.success,
            report.message
        );
    } else {
        log::info!(
            "USB uninstall phase={phase} cleanup_pending={}",
            report.cleanup_pending
        );
    }
}

/// Windows USB build: remove the manager-owned Codex installation and the
/// locally copied CCSwitch program in one guarded operation. Both operations
/// are idempotent; if one side is already absent, the other side is still
/// attempted. User data is retained unless the corresponding purge flag is
/// explicitly enabled by the UI. The offline USB installation kit is always
/// preserved so the drive remains reusable for a future reinstall.
#[tauri::command]
pub async fn win_uninstall_bundle(
    state: State<'_, ManagerState>,
    confirm: bool,
    token: OperationToken,
    purge_user_data: bool,
    purge_ccswitch_data: bool,
    force_cleanup: Option<bool>,
    remove_usb_media: Option<bool>,
) -> Result<WinBundleUninstallReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Windows) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    if !confirm {
        return Err(
            AppError::Internal("拒绝执行：Windows 卸载必须带显式 confirm".to_string()).into(),
        );
    }
    let _op = DetachedGuard::validate_with_phase(&state, token, OperationPhase::Committing)?;
    let settings = windows_domain_settings_for_persisted(&state);
    let transaction_scope = PathBuf::from(settings.install_root.clone());
    let force_cleanup = force_cleanup.unwrap_or(false);
    let remove_usb_media = remove_usb_media.unwrap_or(false);
    if remove_usb_media {
        // Older renderers exposed a destructive "erase USB kit" toggle. Keep
        // accepting the optional field for protocol compatibility, but never
        // honor it: deleting the kit would violate the product guarantee that
        // an intact USB drive can always perform a fresh reinstall.
        return Err(AppError::Engine(
            "为保证 U 盘可随时重装，当前版本不支持删除 U 盘安装包；安装包已保留。".to_string(),
        )
        .into());
    }
    // Source-media cleanup is intentionally disabled. The USB kit is the
    // reinstall path and must remain intact for the lifetime of the drive.
    let source_license: Option<crate::app::usb_guard::UsbFingerprint> = None;
    let report = tauri::async_runtime::spawn_blocking(move || {
        // Validate and close only this USB install's terminal bookkeeping
        // before touching either product. A foreign or malformed recovery
        // record must never be reconciled or deleted on behalf of this one.
        let preflight = crate::app::install_tx::reconcile_for_uninstall_scoped(Some(
            &transaction_scope,
        ))
        .and_then(|_| {
            crate::app::install_tx::clear_completed_for_uninstall_scoped(Some(
                &transaction_scope,
            ))
            .map(|_| ())
        })
        .and_then(|_| {
            if force_cleanup {
                Ok(())
            } else {
                crate::app::install_tx::validate_foreign_recovery_volume(&transaction_scope)
            }
        });
        if let Err(error) = preflight {
            let mut report = combine_bundle_uninstall(
                Err(AppError::Engine(format!(
                    "U 盘安装事务无法安全核对，未开始卸载：{error}"
                ))),
                Err(AppError::Engine("由于安装恢复记录未通过安全核验，未卸载 CCSwitch".into())),
                purge_user_data,
                purge_ccswitch_data,
            );
            finish_bundle_uninstall_report(&mut report);
            return report;
        }
        let mut codex =
            uninstall_windows_codex_for_usb_bundle(&settings, confirm, purge_user_data);
        if let Ok(codex_report) = &mut codex {
            if codex_report.outcome.primary_ok {
                append_native_model_catalog_cleanup(codex_report);
            }
        }
        let mut cleanup_claim = None;
        let mut cleanup_claim_note = None;

        // Current-user removal is not the complete AppX postcondition. Failed
        // deployments can leave one machine-level Staged record with an empty
        // or dangling package path. Classify it before destroying the guard
        // lease or the AppX-volume journal, then authenticate the exact cleanup
        // identity from the signed USB MSIX. Never edit PackageRepository or a
        // WindowsApps ACL/link directly.
        if let Ok(codex_report) = &mut codex {
            if codex_report.success {
                let machine_state = probe_windows_codex_machine_state();
                let volume_cleanup_pending =
                    match crate::app::appx_volume::has_pending_volume_cleanup() {
                        Ok(pending) => pending,
                        Err(error) => {
                            cleanup_claim_note = Some(format!(
                                "无法预读 AppX 卷恢复记录：{error}；未授权删除任何未知或共享机器包。"
                            ));
                            false
                        }
                    };
                let orphan_candidate = machine_state
                    .as_ref()
                    .is_ok_and(|state| state.is_orphan_candidate());
                let machine_reconciliation = machine_state
                    .as_ref()
                    .is_ok_and(|state| state.is_none() && volume_cleanup_pending);
                if orphan_candidate || volume_cleanup_pending {
                    match std::env::current_exe()
                        .map_err(|error| AppError::Internal(error.to_string()))
                        .and_then(|exe| crate::app::offline_payload::cleanup_package_claim(&exe))
                    {
                        Ok(claim) => cleanup_claim = Some(claim),
                        Err(error) => {
                            log::warn!("USB machine/staged cleanup claim unavailable: {error}");
                            cleanup_claim_note = Some(format!(
                                "未建立精确机器包清理授权：{error}；未知、共享或未签名身份只会列出并保留。"
                            ));
                        }
                    }
                }

                match machine_state {
                    Err(error) => fail_usb_codex_machine_cleanup(
                        codex_report,
                        format!(
                            "无法核验所有用户/预配/机器级 Codex AppX 状态，不能把当前用户包已删除当作彻底卸载：{error}"
                        ),
                    ),
                    Ok(state) if state.is_orphan_candidate() => {
                        let observed = state.package_full_name.as_deref().unwrap_or("");
                        match cleanup_claim.as_ref() {
                            Some(claim)
                                if claim.package_full_name.eq_ignore_ascii_case(observed) => {
                                match cleanup_windows_codex_machine_orphan(
                                    &claim.package_full_name,
                                    &claim.payload_path,
                                    &claim.payload_sha256,
                                    !purge_user_data,
                                ) {
                                    Ok(cleanup) => {
                                        merge_usb_machine_cleanup(codex_report, cleanup)
                                    }
                                    Err(error) => fail_usb_codex_machine_cleanup(
                                        codex_report,
                                        format!("执行精确机器级 AppX 孤儿清理失败：{error}"),
                                    ),
                                }
                            }
                            Some(claim) => fail_usb_codex_machine_cleanup(
                                codex_report,
                                format!(
                                    "机器级孤儿包 {observed} 与已签名离线载荷 {} 不一致；已保留现场",
                                    claim.package_full_name
                                ),
                            ),
                            None => fail_usb_codex_machine_cleanup(
                                codex_report,
                                cleanup_claim_note.clone().unwrap_or_else(|| {
                                    "没有经过 Authenticode 校验的精确 FullName，未清理机器级孤儿包"
                                        .to_string()
                                }),
                            ),
                        }
                    }
                    Ok(state) if machine_reconciliation && state.is_none() => {
                        // A durable AppX-volume journal can outlive the
                        // PowerShell projection. Give the documented WinRT API
                        // one exact reconciliation attempt before removing the
                        // journal and verify the Windows-managed system entry.
                        if let Some(claim) = cleanup_claim.as_ref() {
                            match cleanup_windows_codex_machine_orphan(
                                &claim.package_full_name,
                                &claim.payload_path,
                                &claim.payload_sha256,
                                !purge_user_data,
                            ) {
                                Ok(cleanup) => merge_usb_machine_cleanup(codex_report, cleanup),
                                Err(error) => fail_usb_codex_machine_cleanup(
                                    codex_report,
                                    format!("核对机器级 AppX 清理状态失败：{error}"),
                                ),
                            }
                        }
                    }
                    Ok(state) if !state.is_none() => fail_usb_codex_machine_cleanup(
                        codex_report,
                        format!(
                            "检测到不可自动删除的机器级 Codex 状态：classification={}；reason={}；package={}；owners={}；provisioned={}",
                            state.classification,
                            state.reason,
                            state.package_full_name.as_deref().unwrap_or("none"),
                            state.owner_states.join(","),
                            state.provisioned_packages.join(",")
                        ),
                    ),
                    Ok(_) => {}
                }
            }
        }

        let ccswitch = uninstall_ccswitch_impl(purge_ccswitch_data);
        // CCSwitch termination uses the active lease's PID list.  Stop and
        // remove the helper only after that step and machine AppX data backup
        // have had a chance to use the verified bound-volume lease.
        let guard_notes = crate::app::usb_guard::stop_and_uninstall();
        let mut report =
            combine_bundle_uninstall(codex, ccswitch, purge_user_data, purge_ccswitch_data);
        report.notes.extend(guard_notes);
        // Local installer staging is neither AppX package data nor recovery
        // evidence. Clean it even when an owned/shared AppX volume must remain,
        // so a partial system cleanup cannot strand the ~800 MB MSIX copy.
        match crate::app::uninstall_cleanup::cleanup_install_staging_now() {
            Ok(items) => {
                let failures = items
                    .iter()
                    .filter(|item| item.status == "failed")
                    .map(|item| format!("{}：{}", item.path, item.detail))
                    .collect::<Vec<_>>();
                report.notes.extend(items.into_iter().map(|item| {
                    format!("安装暂存 {}：{}（{}）", item.status, item.path, item.detail)
                }));
                if !failures.is_empty() {
                    let detail = format!("安装暂存清理未完成：{}", failures.join("；"));
                    report.success = false;
                    report.outcome.cleanup = StepOutcome::failed(detail);
                }
            }
            Err(error) => {
                let detail = format!("无法枚举安装暂存目录：{error}");
                report.success = false;
                report.notes.push(detail.clone());
                report.outcome.cleanup = StepOutcome::failed(detail);
            }
        }
        // Do not discard recovery evidence while the program or its policies
        // remain unresolved. Each failure leaves the manager available to retry.
        if report.codex.as_ref().is_some_and(|r| r.success) {
            let system = crate::app::appx_volume::cleanup_system_after_uninstall(
                cleanup_claim
                    .as_ref()
                    .map(|claim| claim.package_full_name.as_str()),
            );
            report.notes.extend(system.messages);
            report.notes.extend(system.warnings);
            if system.pending {
                if let Some(note) = cleanup_claim_note {
                    report.notes.push(note);
                }
                report.success = false;
                report.outcome.cleanup =
                    StepOutcome::failed("AppX 卷或策略尚未清理，已保留恢复记录");
            }
            match crate::app::codex_self_update::cleanup_for_uninstall() {
                Ok(notes) => report.notes.extend(notes),
                Err(error) => {
                    report.success = false;
                    report
                        .notes
                        .push(format!("自动更新策略恢复未完成：{error}"));
                    report.outcome.cleanup = StepOutcome::failed(error.to_string());
                }
            }
            if report.success {
                if let Err(error) = crate::app::msix_policy_tx::clear_pending_after_uninstall() {
                    report.success = false;
                    report.notes.push(format!("安装策略事务仍待处理：{error}"));
                    report.outcome.cleanup = StepOutcome::failed(error.to_string());
                }
                // The primary uninstall may remove the install root that was
                // still present during the preflight reconciliation.  Retry
                // this bookkeeping-only pass now, so a landed transaction
                // whose recovery paths are both gone can be closed before the
                // completed-journal cleanup runs.
                if report.success {
                    match crate::app::install_tx::reconcile_for_uninstall_scoped(Some(
                        &transaction_scope,
                    )) {
                        Ok(reconciled) if reconciled > 0 => report.notes.push(format!(
                            "已在卸载后核对并关闭 {reconciled} 个已落地安装事务"
                        )),
                        Ok(_) => {}
                        Err(error) => {
                            report.success = false;
                            report
                                .notes
                                .push(format!("卸载后安装文件事务核对未完成：{error}"));
                            report.outcome.cleanup = StepOutcome::failed(error.to_string());
                        }
                    }
                }
                if report.success {
                    if let Err(error) = crate::app::install_tx::clear_completed_for_uninstall_scoped(
                        Some(&transaction_scope),
                    ) {
                        report.success = false;
                        report.notes.push(format!("U 盘安装文件事务仍待处理：{error}"));
                        report.outcome.cleanup = StepOutcome::failed(error.to_string());
                    }
                }
                if report.success {
                    if let Err(error) = crate::app::uninstall_cleanup::deferred_cleanup_preflight(force_cleanup) {
                        defer_shared_bundle_cleanup(&mut report, &error);
                    }
                }
            }
        }
        if bundle_can_start_final_cleanup(&report) {
            match crate::app::uninstall_cleanup::launch_deferred_cleanup(
                purge_user_data,
                purge_ccswitch_data,
                force_cleanup,
                remove_usb_media,
                source_license,
                report.notes.clone(),
            ) {
                Ok(handoff) => {
                    crate::app::usb_guard::suppress_status_for_uninstall();
                    report.cleanup_pending = true;
                    report.cleanup_handoff_id = Some(handoff.handoff_id.clone());
                    report.cleanup_report_path = Some(handoff.report_path);
                    report.message = handoff.message;
                    report.outcome.cleanup =
                        StepOutcome::skipped("等待管理器退出后进行最终清理与复核");
                }
                Err(error) => {
                    report.success = false;
                    report.notes.push(format!("最终清理进程未启动：{error}"));
                    report.outcome.cleanup = StepOutcome::failed(error);
                }
            }
        }
        finish_bundle_uninstall_report(&mut report);
        report
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?;
    if report.cleanup_pending {
        // Return the truthful handoff state and keep the manager alive until
        // the renderer has shown it. The helper already holds the session
        // mutex and waits on this exact process handle; a fixed-delay exit
        // would turn this intermediate state into a silent close.
        let handoff_id = report.cleanup_handoff_id.clone().ok_or_else(|| {
            AppError::Internal("清理交接缺少唯一标识，已保留清理报告".to_string())
        })?;
        let report_path = report.cleanup_report_path.clone().unwrap_or_default();
        if !state.arm_uninstall_handoff(UninstallHandoff {
            id: handoff_id,
            report_path,
            message: report.message.clone(),
        }) {
            return Err(AppError::Busy(
                "已有卸载清理交接正在等待退出，请先完成当前交接".to_string(),
            )
            .into());
        }
        log::info!("Windows uninstall handoff ready; awaiting user acknowledgement");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{
        install_root_from_picked_dir, invalidate_retained_pause_if_stale, is_usb_install_path,
        manager_update_matches_confirmation, normalize_windows_source_base,
        record_macos_policy_install_state, record_windows_policy_install_state,
        should_attempt_theme_codex_launch, validate_historical_architecture_with_compatibility,
        validate_install_root_path, validated_custom_proxy_for_settings,
        HistoricalPolicyInstallState, INSTALL_LOCATION_PROBE_PREFIX,
    };

    #[test]
    fn signed_usb_context_cannot_be_downgraded_by_omitting_renderer_flag() {
        assert!(is_usb_install_path(false, true));
        assert!(is_usb_install_path(true, false));
        assert!(is_usb_install_path(true, true));
        assert!(!is_usb_install_path(false, false));
    }

    #[test]
    fn catalog_reload_uses_normal_usb_launch_instead_of_fixed_port_theme_hot_attach() {
        assert!(!should_attempt_theme_codex_launch(true, true));
        assert!(should_attempt_theme_codex_launch(true, false));
        assert!(!should_attempt_theme_codex_launch(false, false));
    }
    use crate::app::mac_update::HistoricalMacEvidence;
    use crate::app::op_phase::OperationPhase;
    use crate::app::oplock::{OperationKind, OperationManager};
    use crate::app::release_install::ReleaseArchitecture;
    use crate::app::win_update::OperationEvidence;
    use crate::domain::target::{Architecture, OperatingSystem, Target};
    use crate::errors::AppError;
    use crate::state::{ManagerState, UninstallHandoff};
    use std::fs;
    use std::sync::atomic::AtomicU8;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
    }

    fn uninstall_fixture(success: bool) -> super::WinUninstallReport {
        super::WinUninstallReport {
            success,
            action: "remove-msix".into(),
            message: if success {
                "package absent"
            } else {
                "linked package data blocked"
            }
            .into(),
            installed_before: None,
            msix: Some(codex_win_engine::MsixRemoveReport {
                success,
                message: "package result".into(),
                raw_error: (!success)
                    .then(|| "HRESULT=0x80073CFA ActivityId=fixture-activity".into()),
                notes: vec![],
            }),
            portable: None,
            purged_user_data: false,
            notes: vec!["fixture backup retained".into()],
            outcome: if success {
                super::OperationOutcome::full_success("absent", Some("none"))
            } else {
                super::OperationOutcome::primary_failed("present", "linked package data blocked")
            },
        }
    }

    fn switch_uninstall_fixture(success: bool) -> super::CcswitchUninstallReport {
        super::CcswitchUninstallReport {
            success,
            installed_before: true,
            removed_install: success,
            removed_autostart: success,
            purged_user_data: false,
            install_dir: "fixture".into(),
            data_dirs: vec![],
            message: if success {
                "switch absent"
            } else {
                "switch file locked"
            }
            .into(),
            notes: vec![],
        }
    }

    #[test]
    fn pending_uninstall_handoff_blocks_new_write_commands() {
        let state = ManagerState::new();
        assert!(super::reject_uninstall_handoff(&state).is_ok());
        assert!(!state.arm_uninstall_handoff(UninstallHandoff {
            id: "  ".into(),
            report_path: "report.json".into(),
            message: "invalid".into(),
        }));
        assert!(state.arm_uninstall_handoff(UninstallHandoff {
            id: "handoff".into(),
            report_path: "report.json".into(),
            message: "waiting".into(),
        }));
        assert!(super::reject_uninstall_handoff(&state).is_err());
        assert_eq!(
            state.uninstall_handoff().map(|handoff| handoff.id),
            Some("handoff".into())
        );
    }

    #[test]
    fn nonterminal_bundle_steps_cannot_start_final_cleanup() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        report.outcome.cleanup = super::StepOutcome {
            state: "pending".into(),
            detail: Some("helper has not committed".into()),
        };
        assert!(!super::bundle_can_start_final_cleanup(&report));
        super::finish_bundle_uninstall_report(&mut report);
        assert!(!report.success);
        assert!(report.message.contains("尚未完成"));
        assert!(report.message.contains("helper has not committed"));
    }

    #[test]
    fn foreign_shared_recovery_record_defers_only_shared_cleanup() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        super::defer_shared_bundle_cleanup(&mut report, "foreign transaction fixture");
        assert!(report.success);
        assert!(report.outcome.primary_ok);
        assert!(report.cleanup_deferred);
        assert_eq!(report.outcome.cleanup.state, "skipped");
        assert!(!super::bundle_can_start_final_cleanup(&report));
        super::finish_bundle_uninstall_report(&mut report);
        assert!(report.message.contains("共享管理器数据"));
    }

    #[test]
    fn machine_orphan_cleanup_failure_blocks_finalizer_but_keeps_primary_evidence() {
        let mut codex = uninstall_fixture(true);
        super::merge_usb_machine_cleanup(
            &mut codex,
            codex_win_engine::MsixRemoveReport {
                success: false,
                message: "Windows-managed package entry remains".into(),
                raw_error: Some("fixture machine cleanup error".into()),
                notes: vec!["recovery evidence retained".into()],
            },
        );
        assert!(!codex.success);
        assert!(codex.outcome.primary_ok);
        assert!(codex
            .outcome
            .recovery_actions
            .iter()
            .any(|item| item == crate::app::operation_outcome::recovery::CLEANUP_METADATA));
        let report = super::combine_bundle_uninstall(
            Ok(codex),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        assert!(!report.success);
        assert!(!super::bundle_can_start_final_cleanup(&report));
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("fixture machine cleanup error")));
    }

    #[test]
    fn successful_machine_orphan_cleanup_can_continue_to_finalizer() {
        let mut codex = uninstall_fixture(true);
        codex.msix = None;
        super::merge_usb_machine_cleanup(
            &mut codex,
            codex_win_engine::MsixRemoveReport {
                success: true,
                message: "exact orphan absent".into(),
                raw_error: None,
                notes: vec!["supported AppX API used".into()],
            },
        );
        assert!(codex.success);
        assert!(codex.msix.as_ref().is_some_and(|item| item.success));
        let report = super::combine_bundle_uninstall(
            Ok(codex),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        assert!(super::bundle_can_start_final_cleanup(&report));
    }

    #[test]
    fn bundle_uninstall_keeps_primary_and_raw_errors_when_notes_are_added() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(false)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        report.notes.push("later policy note".into());
        super::finish_bundle_uninstall_report(&mut report);
        assert!(!report.success);
        assert!(!super::bundle_can_start_final_cleanup(&report));
        for detail in [
            "linked package data blocked",
            "HRESULT=0x80073CFA",
            "ActivityId=fixture-activity",
            "fixture backup retained",
            "later policy note",
        ] {
            assert!(
                report.message.contains(detail),
                "missing {detail}: {}",
                report.message
            );
        }
        assert!(report.message.starts_with("Codex 主卸载未完成"));
    }

    #[test]
    fn bundle_uninstall_keeps_both_thrown_errors_without_a_nested_report() {
        let mut report = super::combine_bundle_uninstall(
            Err(AppError::Engine("AppX query failed".into())),
            Err(AppError::Engine("CCSwitch access denied".into())),
            false,
            false,
        );
        super::finish_bundle_uninstall_report(&mut report);
        assert!(!report.success);
        assert_eq!(report.outcome.app_state, "unknown");
        assert!(report.message.contains("AppX query failed"));
        assert!(report.message.contains("CCSwitch access denied"));
        assert!(!super::bundle_can_start_final_cleanup(&report));
    }

    #[test]
    fn bundle_uninstall_preserves_switch_failure_after_codex_success() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(false)),
            false,
            false,
        );
        super::finish_bundle_uninstall_report(&mut report);
        assert!(!report.success);
        assert!(report.message.contains("switch file locked"));
        assert!(report.message.starts_with("Codex 主卸载未完成"));
        assert!(!super::bundle_can_start_final_cleanup(&report));
    }

    #[test]
    fn bundle_uninstall_does_not_handoff_after_provenance_or_cleanup_failure() {
        for provenance_failure in [true, false] {
            let mut codex = uninstall_fixture(true);
            if provenance_failure {
                codex.outcome.provenance = super::StepOutcome::failed("record save denied");
            } else {
                codex.outcome.cleanup = super::StepOutcome::failed("metadata busy");
            }
            let mut report = super::combine_bundle_uninstall(
                Ok(codex),
                Ok(switch_uninstall_fixture(true)),
                false,
                false,
            );
            assert!(
                report.success,
                "primary result stays successful for targeted retry"
            );
            assert!(!super::bundle_can_start_final_cleanup(&report));
            super::finish_bundle_uninstall_report(&mut report);
            assert!(report
                .message
                .starts_with("Codex 主卸载已完成，但联合卸载或附属清理未全部完成"));
            assert!(report.message.contains(if provenance_failure {
                "record save denied"
            } else {
                "metadata busy"
            }));
        }
    }

    #[test]
    fn bundle_uninstall_keeps_late_helper_or_policy_error_and_primary_result() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        report.success = false;
        report.outcome.cleanup = super::StepOutcome::failed("helper handshake timed out");
        super::finish_bundle_uninstall_report(&mut report);
        assert!(report.message.contains("package absent"));
        assert!(report.message.contains("helper handshake timed out"));
        assert!(!super::bundle_can_start_final_cleanup(&report));
    }

    #[test]
    fn bundle_uninstall_pending_never_replaces_handoff_with_all_clean() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        assert!(super::bundle_can_start_final_cleanup(&report));
        report.cleanup_pending = true;
        report.cleanup_report_path = Some("fixture-report.json".into());
        report.message = "waiting for manager exit; pending final cleanup".into();
        report.outcome.cleanup = super::StepOutcome::skipped("pending");
        super::finish_bundle_uninstall_report(&mut report);
        assert_eq!(
            report.message,
            "waiting for manager exit; pending final cleanup"
        );
        assert!(report.cleanup_pending);
        assert_eq!(
            report.cleanup_report_path.as_deref(),
            Some("fixture-report.json")
        );
    }

    #[test]
    fn bundle_uninstall_pending_recovery_action_blocks_helper_even_without_failed_flag() {
        let mut report = super::combine_bundle_uninstall(
            Ok(uninstall_fixture(true)),
            Ok(switch_uninstall_fixture(true)),
            false,
            false,
        );
        report
            .outcome
            .recovery_actions
            .push("clear_provenance".into());
        assert!(!super::bundle_can_start_final_cleanup(&report));
        super::finish_bundle_uninstall_report(&mut report);
        assert!(report.success);
        assert!(report.message.contains("尚待恢复步骤：clear_provenance"));
        assert!(report
            .message
            .starts_with("Codex 主卸载已完成，但联合卸载或附属清理未全部完成"));
    }

    #[test]
    fn bundle_uninstall_shared_data_flags_are_independent() {
        for (purge_codex, purge_switch) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let mut codex = uninstall_fixture(true);
            codex.purged_user_data = true;
            let mut switch = switch_uninstall_fixture(true);
            switch.purged_user_data = true;
            let report =
                super::combine_bundle_uninstall(Ok(codex), Ok(switch), purge_codex, purge_switch);
            assert_eq!(report.purged_user_data, purge_codex);
            assert_eq!(report.purged_ccswitch_data, purge_switch);
        }
    }

    #[test]
    fn normalizes_windows_source_base_urls() {
        assert_eq!(
            normalize_windows_source_base("https://example.test/latest/manifest").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            normalize_windows_source_base("https://example.test/latest/win/").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            normalize_windows_source_base("https://example.test/latest/win-arm64").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            normalize_windows_source_base("https://example.test/latest/win-x64").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            normalize_windows_source_base("https://example.test/custom").as_deref(),
            Some("https://example.test/custom")
        );
        assert!(normalize_windows_source_base("   ").is_none());
    }

    #[test]
    fn historical_architecture_rejects_unrunnable_host_combinations() {
        let target = |os, arch| Target {
            os,
            arch,
            label: "test".to_string(),
        };
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::X64),
            ReleaseArchitecture::X64,
            false,
        )
        .is_ok());
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::X64),
            ReleaseArchitecture::Arm64,
            true,
        )
        .is_err());
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::Arm64),
            ReleaseArchitecture::Arm64,
            false,
        )
        .is_ok());
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::Arm64),
            ReleaseArchitecture::X64,
            true,
        )
        .is_ok());
        let missing_rosetta = validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::Arm64),
            ReleaseArchitecture::X64,
            false,
        )
        .unwrap_err();
        assert!(missing_rosetta.to_string().contains("Rosetta 2"));
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Windows, Architecture::Arm64),
            ReleaseArchitecture::X64,
            true,
        )
        .is_ok());
        let missing_windows_emulation = validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Windows, Architecture::Arm64),
            ReleaseArchitecture::X64,
            false,
        )
        .unwrap_err();
        assert!(missing_windows_emulation
            .to_string()
            .contains("不支持 x64 仿真"));
        assert!(validate_historical_architecture_with_compatibility(
            &target(OperatingSystem::Macos, Architecture::Unknown),
            ReleaseArchitecture::X64,
            true,
        )
        .is_err());
    }

    #[test]
    fn macos_policy_is_restorable_only_before_mutation_or_after_rollback() {
        let state = AtomicU8::new(HistoricalPolicyInstallState::PreCommit as u8);
        assert!(HistoricalPolicyInstallState::load(&state).policy_may_be_restored());

        record_macos_policy_install_state(&state, HistoricalMacEvidence::MutationStarted);
        assert_eq!(
            HistoricalPolicyInstallState::load(&state),
            HistoricalPolicyInstallState::Mutated
        );
        assert!(!HistoricalPolicyInstallState::load(&state).policy_may_be_restored());

        record_macos_policy_install_state(&state, HistoricalMacEvidence::MutationRolledBack);
        assert_eq!(
            HistoricalPolicyInstallState::load(&state),
            HistoricalPolicyInstallState::RolledBack
        );
        assert!(HistoricalPolicyInstallState::load(&state).policy_may_be_restored());
    }

    #[test]
    fn windows_policy_keeps_add_appx_ambiguity_sticky_across_fallback() {
        let state = AtomicU8::new(HistoricalPolicyInstallState::PreCommit as u8);
        record_windows_policy_install_state(&state, OperationEvidence::OutcomeAmbiguous);
        record_windows_policy_install_state(&state, OperationEvidence::MutationStarted);
        record_windows_policy_install_state(&state, OperationEvidence::MutationRolledBack);
        assert_eq!(
            HistoricalPolicyInstallState::load(&state),
            HistoricalPolicyInstallState::OutcomeUnknown
        );
        assert!(!HistoricalPolicyInstallState::load(&state).policy_may_be_restored());
    }

    #[test]
    fn windows_policy_is_restorable_after_a_confirmed_portable_rollback() {
        let state = AtomicU8::new(HistoricalPolicyInstallState::PreCommit as u8);
        record_windows_policy_install_state(&state, OperationEvidence::MutationStarted);
        record_windows_policy_install_state(&state, OperationEvidence::MutationRolledBack);
        assert_eq!(
            HistoricalPolicyInstallState::load(&state),
            HistoricalPolicyInstallState::RolledBack
        );
        assert!(HistoricalPolicyInstallState::load(&state).policy_may_be_restored());
    }

    #[test]
    fn stale_resume_invalidates_the_retained_backend_pause() {
        let path = temp_path("cam-stale-historical-pause.lock");
        let _ = fs::remove_file(&path);
        let manager = OperationManager::new(path.clone());

        let paused = manager.begin_detached(OperationKind::Install).unwrap();
        manager.validate(&paused).unwrap();
        manager
            .set_phase(&paused, OperationPhase::Downloading)
            .unwrap();
        manager.set_paused(&paused, true).unwrap();
        manager.record_completion(&paused, false).unwrap();
        manager.end(paused).unwrap();
        assert!(manager.paused_snapshot().is_some());

        // Ordinary transient failures keep the resumable bytes/context.
        invalidate_retained_pause_if_stale(
            &manager,
            &AppError::Engine("network unavailable".to_string()),
        );
        assert!(manager.paused_snapshot().is_some());

        // A new resume can fail before emitting progress. Clearing while that
        // lease is active must survive its failed completion and lease release.
        let resume = manager.begin_detached(OperationKind::Install).unwrap();
        manager.validate(&resume).unwrap();
        invalidate_retained_pause_if_stale(
            &manager,
            &AppError::StaleExpectation("install changed".to_string()),
        );
        manager.record_completion(&resume, false).unwrap();
        manager.end(resume).unwrap();
        assert!(manager.paused_snapshot().is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn settings_custom_proxy_requires_a_url() {
        let err = validated_custom_proxy_for_settings("  ", "settings").unwrap_err();
        assert!(err.to_string().contains("代理不能为空"));
        assert_eq!(
            validated_custom_proxy_for_settings("socks5h://127.0.0.1:1080", "settings").unwrap(),
            "socks5h://127.0.0.1:1080"
        );
    }

    #[test]
    fn manager_self_update_confirmation_must_match_latest_check() {
        assert!(manager_update_matches_confirmation(
            "0.2.1", "0.2.0", "0.2.1", "0.2.0"
        ));
        assert!(!manager_update_matches_confirmation(
            "0.2.2", "0.2.0", "0.2.1", "0.2.0"
        ));
        assert!(!manager_update_matches_confirmation(
            "0.2.1", "0.2.1", "0.2.1", "0.2.0"
        ));
    }

    #[test]
    fn rejects_non_empty_non_codex_install_root() {
        let root = temp_path("codex-manager-non-empty-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), b"not codex").unwrap();

        let err = validate_install_root_path(&root.to_string_lossy()).unwrap_err();
        assert!(err
            .to_string()
            .contains("安装位置必须是空文件夹，或已有的 Codex 免安装版目录"));

        let _ = fs::remove_dir_all(root);
    }

    fn write_portable_manifest(root: &std::path::Path, identity_name: &str) {
        fs::write(
            root.join("AppxManifest.xml"),
            format!(
                r#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="{identity_name}" Publisher="CN=OpenAI OpCo, LLC" Version="26.707.3748.0" ProcessorArchitecture="x64" />
</Package>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn accepts_existing_codex_portable_install_root() {
        let root = temp_path("codex-manager-existing-portable-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Codex.exe"), b"codex").unwrap();
        write_portable_manifest(&root, "OpenAI.Codex");

        let validated = validate_install_root_path(&root.to_string_lossy()).unwrap();
        assert_eq!(validated, root.to_string_lossy());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_portable_root_with_foreign_identity() {
        // An unpacked non-Codex payload (e.g. ChatGPT Classic) also carries a
        // root-level ChatGPT.exe + AppxManifest.xml. Its identity fails the
        // gate, so the non-empty directory must NOT be treated as replaceable.
        let root = temp_path("codex-manager-foreign-portable-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ChatGPT.exe"), b"classic").unwrap();
        write_portable_manifest(&root, "OpenAI.ChatGPT");

        let err = validate_install_root_path(&root.to_string_lossy()).unwrap_err();
        assert!(err
            .to_string()
            .contains("安装位置必须是空文件夹，或已有的 Codex 免安装版目录"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn picked_non_empty_folder_installs_into_codex_child() {
        let parent = temp_path("codex-manager-picked-parent");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("existing.txt"), b"user file").unwrap();

        let validated = install_root_from_picked_dir(&parent.to_string_lossy()).unwrap();
        assert_eq!(validated, parent.join("Codex").to_string_lossy());
        // Validation maps to the Codex child but must not create it — the
        // directory only appears at install time.
        assert!(!parent.join("Codex").exists());

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn validation_does_not_create_the_target_directory() {
        let root = temp_path("codex-manager-validate-no-create").join("Codex");
        let _ = fs::remove_dir_all(root.parent().unwrap());

        let validated = validate_install_root_path(&root.to_string_lossy()).unwrap();
        assert_eq!(validated, root.to_string_lossy());
        // Validating a fresh location must leave the filesystem untouched.
        assert!(
            !root.exists(),
            "validation must not create the install root"
        );

        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn validation_removes_parent_replace_probe() {
        let parent = temp_path("codex-manager-validate-probe-cleanup");
        let root = parent.join("Codex");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&parent).unwrap();

        validate_install_root_path(&root.to_string_lossy()).unwrap();

        assert!(!fs::read_dir(&parent).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(INSTALL_LOCATION_PROBE_PREFIX)
        }));
        assert!(!root.exists());

        let _ = fs::remove_dir_all(parent);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_protected_install_root() {
        let Some(program_files) = std::env::var_os("ProgramFiles") else {
            return;
        };
        let root = std::path::PathBuf::from(program_files).join("Codex");

        let err = validate_install_root_path(&root.to_string_lossy()).unwrap_err();
        assert!(err
            .to_string()
            .contains("安装位置不能放在系统目录、管理员目录或磁盘根目录"));
    }
}
