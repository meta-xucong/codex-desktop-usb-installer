use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::app_version::read_codex_app_version_from_install_root;
use crate::capability::WinCapabilityReport;
#[cfg(windows)]
use crate::capability::{CapabilityCheck, CapabilityState};
use crate::limits::MAX_TEXT_BYTES;
use crate::msix::parse_appx_manifest_xml;
use crate::network::{is_schannel_revocation_offline, NetworkConfig, SchannelRevocationCheck};
use crate::process::{
    curl_exe, hidden_command, run_capturing, spawn_and_require_liveness, LivenessResult, RunLimits,
    PORTABLE_LIVENESS_WINDOW,
};
#[cfg(windows)]
use crate::process::{
    RunError, TimeoutKind, MSIX_ACTIVATION_WINDOW_SECS, MSIX_LIVENESS_WINDOW_SECS,
};
use crate::EngineError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledWindowsCodex {
    pub path: String,
    /// Human-facing Codex app version read from app.asar/package.json when available.
    /// Falls back to the Windows package version if the app payload is unreadable.
    pub version: String,
    pub arch: Option<String>,
    /// "msix" | "portable"
    pub source: String,
    pub package_family_name: Option<String>,
    /// Drive root of the registered AppX package (for example `F:\`).
    /// This is descriptive metadata; authorization still belongs to the USB
    /// guard. Older provenance records omit it and remain readable.
    #[serde(default)]
    pub package_volume: Option<String>,
    /// Install-dir / executable mtime as Unix seconds — when this build landed
    /// on disk. Surfaced as the "installed" date (Windows has no Sparkle feed).
    #[serde(default)]
    pub installed_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PortableLaunchPolicy {
    /// Preserve the ordinary desktop-manager behaviour.
    #[default]
    Standard,
    /// USB edition: start with the normal hardware-rendered launch and require
    /// a stable user-operable window. Retry with software-rendering/no-sandbox
    /// compatibility flags only when diagnostics succeeded but no usable
    /// window appeared.
    UsbCompatible,
    /// A prior launch on this machine already proved that Chromium's sandbox
    /// must be disabled. Keep the normal GPU path but apply that exception to
    /// the first USB attempt without showing a renderer auxiliary window.
    UsbCompatibleNoSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLaunchReport {
    /// `msix`, `standard`, `portable`, `portable-no-sandbox`,
    /// `software-rendering`, or `software-rendering-no-sandbox`.
    pub launch_mode: String,
    pub usable_window_confirmed: bool,
    pub no_sandbox: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub disable_codex_self_updates: bool,
    /// Signed schema 3 USB bundles require the direct-entry shim overlay.
    /// Legacy schema 2 and standard portable installs retain their manifest
    /// entry behavior.
    pub require_usb_entry_overlay: bool,
    /// Start Electron's loopback DevTools endpoint for renderer theme injection.
    /// `None` preserves the ordinary launch path.
    pub remote_debugging_port: Option<u16>,
    /// Optional explicit Codex state root. USB builds set this to the normal
    /// per-user profile path so an inherited environment cannot redirect
    /// account/session/config state onto the removable program volume.
    /// Managed Skill/MCP payloads are a separate USB content concern.
    pub codex_home: Option<PathBuf>,
    /// Optional Electron locale override passed to portable Codex launches.
    /// The USB manager currently uses this for `zh-CN`/`zh-TW`, where the
    /// bundled webview translations are enabled before the process starts.
    pub locale_override: Option<String>,
    pub portable_launch_policy: PortableLaunchPolicy,
}

const CODEX_SELF_UPDATE_ENV_KEY: &str = "CODEX_SPARKLE_ENABLED";
const CODEX_SELF_UPDATE_ENV_DISABLED: &str = "false";
const CODEX_LOCALE_OVERRIDE_ENV_KEY: &str = "ELECTRON_LOCALE_OVERRIDE";

fn normalized_locale_override(locale: Option<&str>) -> Option<&str> {
    locale.map(str::trim).filter(|locale| {
        matches!(
            locale.to_ascii_lowercase().as_str(),
            "zh-cn" | "zh-tw" | "zh-hans" | "zh-hant"
        )
    })
}

fn remote_debugging_arguments(port: u16) -> [String; 2] {
    [
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--remote-debugging-port={port}"),
    ]
}

#[cfg(windows)]
fn portable_launch_arguments(
    options: &LaunchOptions,
    software_rendering: bool,
    no_sandbox: bool,
) -> Vec<std::ffi::OsString> {
    let mut arguments = portable_compatibility_arguments(software_rendering, no_sandbox)
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    if let Some(port) = options.remote_debugging_port {
        arguments.extend(
            remote_debugging_arguments(port)
                .into_iter()
                .map(std::ffi::OsString::from),
        );
    }
    arguments
}

#[cfg(windows)]
fn portable_launch_environment_overrides(
    options: &LaunchOptions,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut overrides = Vec::new();
    if options.disable_codex_self_updates {
        overrides.push((
            CODEX_SELF_UPDATE_ENV_KEY.into(),
            CODEX_SELF_UPDATE_ENV_DISABLED.into(),
        ));
    }
    if let Some(codex_home) = options.codex_home.as_ref() {
        overrides.push(("CODEX_HOME".into(), codex_home.as_os_str().to_os_string()));
    }
    if let Some(locale) = normalized_locale_override(options.locale_override.as_deref()) {
        overrides.push((CODEX_LOCALE_OVERRIDE_ENV_KEY.into(), locale.into()));
    }
    overrides
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(any(windows, test))]
struct RegisteredMsixApp {
    package_family_name: String,
    app_id: String,
}

#[cfg(any(windows, test))]
fn registered_msix_aumid(json: &str, expected_family: &str) -> Result<String, EngineError> {
    let entry: RegisteredMsixApp = serde_json::from_str(json)
        .map_err(|e| EngineError::Io(format!("parse registered MSIX app entry: {e}")))?;
    if !entry
        .package_family_name
        .eq_ignore_ascii_case(expected_family)
    {
        return Err(EngineError::Io(format!(
            "registered MSIX package family changed: expected {expected_family}, got {}",
            entry.package_family_name
        )));
    }
    let app_id = entry.app_id.trim();
    if app_id.is_empty() {
        return Err(EngineError::Io(
            "registered MSIX app entry has no application id".to_string(),
        ));
    }
    Ok(format!("{}!{app_id}", entry.package_family_name))
}

/// Filesystem mtime of `path` as Unix seconds, best-effort (None if unreadable).
fn path_mtime_secs(path: &str) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

fn prefer_codex_app_version(mut codex: InstalledWindowsCodex) -> InstalledWindowsCodex {
    if let Some(version) = read_codex_app_version_from_install_root(Path::new(&codex.path)) {
        codex.version = version;
    }
    codex
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsixSideloadReport {
    pub success: bool,
    pub message: String,
    pub installed: Option<InstalledWindowsCodex>,
    pub fallback_recommended: bool,
    pub raw_error: Option<String>,
    /// HRESULT parsed from the AppX deployment exception, when Windows
    /// included one in the error text.  Keeping this separate from `message`
    /// lets the UI/logging layer classify the failure without scraping prose.
    #[serde(default)]
    pub hresult: Option<String>,
    /// ActivityId emitted by AppX deployment.  `Get-AppxLog -ActivityId` can
    /// use it to retrieve the server-side reason on the target machine.
    #[serde(default)]
    pub activity_id: Option<String>,
    /// Bounded `Get-AppxLog` output captured at the failure boundary.  This is
    /// deliberately optional because older/stripped Windows images may not
    /// expose the cmdlet at all.
    #[serde(default)]
    pub deployment_log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsixRemoveReport {
    pub success: bool,
    pub message: String,
    pub raw_error: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Read-only machine-wide view used by the USB uninstaller after the current
/// user's registration has been removed. A package is only a repair candidate
/// when Windows reports one exact official Codex identity, no provisioned
/// package, no installed owner, and either no usable path or one uniquely
/// proven public AppX-volume staging path. The observation is
/// never itself removal authority; the caller must separately derive the exact
/// FullName from the Authenticode-verified offline MSIX.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MsixMachineStateReport {
    /// `none` | `current-user-present` | `orphan-candidate` |
    /// `staged-candidate` | `blocked`.
    pub classification: String,
    pub reason: String,
    pub package_full_name: Option<String>,
    pub install_location: Option<String>,
    #[serde(default)]
    pub owner_states: Vec<String>,
    #[serde(default)]
    pub provisioned_packages: Vec<String>,
    #[serde(default)]
    pub path_state: Option<String>,
    /// AppX volume that owns the exact staging record, when the public
    /// `Get-AppxPackage -Volume` projection can prove one unique volume.
    #[serde(default)]
    pub record_volume_name: Option<String>,
    #[serde(default)]
    pub record_volume_store: Option<String>,
}

pub mod msix_machine_state {
    pub const NONE: &str = "none";
    pub const CURRENT_USER_PRESENT: &str = "current-user-present";
    pub const ORPHAN_CANDIDATE: &str = "orphan-candidate";
    pub const STAGED_CANDIDATE: &str = "staged-candidate";
    pub const BLOCKED: &str = "blocked";
}

impl MsixMachineStateReport {
    pub fn is_none(&self) -> bool {
        self.classification == msix_machine_state::NONE
    }

    pub fn is_orphan_candidate(&self) -> bool {
        matches!(
            self.classification.as_str(),
            msix_machine_state::ORPHAN_CANDIDATE | msix_machine_state::STAGED_CANDIDATE
        )
    }
}

/// Post-install sanity check for a sideloaded MSIX. `Add-AppxPackage` returning
/// success only means the cmdlet did not throw — on a stripped Windows (no
/// Store / App Installer, missing framework packages) the package can register
/// yet fail to launch, which is exactly the failure users hit. We verify the
/// package is registered, its Status is Ok, the app entry (AUMID) resolves,
/// every declared framework dependency is present, **and** a real shell
/// activation leaves a process under the install location. When any of these
/// fail the caller removes the package and falls back to the portable build.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsixHealthReport {
    pub healthy: bool,
    /// Whether the health probe actually ran and the `healthy` verdict reflects
    /// real checks. `false` means the probe could not run (e.g. PowerShell
    /// missing) — not a clean bill of health that was observed.
    pub verified: bool,
    pub package_registered: bool,
    /// Raw `Get-AppxPackage` Status string (e.g. "Ok", "Modified").
    pub status: String,
    pub status_ok: bool,
    /// The app entry (AUMID) could be resolved from the package manifest.
    pub aumid_resolved: bool,
    /// Declared framework dependencies that are NOT installed on this machine —
    /// the usual reason an MSIX installs but won't launch on a stripped Windows.
    pub missing_dependencies: Vec<String>,
    /// Shell activation succeeded and a process under the package install
    /// location stayed alive for the liveness window.
    #[serde(default)]
    pub activation_ok: bool,
    /// Machine-stable failure class for UI / fallback routing. Empty when healthy.
    /// Values: `not-registered` | `status-bad` | `aumid-unresolved` |
    /// `missing-dependencies` | `activation-failed` | `immediate-exit` |
    /// `timeout` | `probe-failed` | `policy` | `cleanup-failed`.
    #[serde(default)]
    pub failure_kind: String,
    /// Human-facing reason when unhealthy; empty when healthy.
    pub reason: String,
}

/// Stable failure-kind strings shared with the frontend and notes.
pub mod msix_failure {
    pub const NOT_REGISTERED: &str = "not-registered";
    pub const STATUS_BAD: &str = "status-bad";
    pub const AUMID_UNRESOLVED: &str = "aumid-unresolved";
    pub const MISSING_DEPENDENCIES: &str = "missing-dependencies";
    pub const ACTIVATION_FAILED: &str = "activation-failed";
    pub const IMMEDIATE_EXIT: &str = "immediate-exit";
    pub const TIMEOUT: &str = "timeout";
    pub const PROBE_FAILED: &str = "probe-failed";
    pub const POLICY: &str = "policy";
    pub const CLEANUP_FAILED: &str = "cleanup-failed";
}

/// Result of the framework-dependency PRE-check run BEFORE attempting an MSIX
/// sideload. On a stripped / China / Store-disabled Windows, `Add-AppxPackage`
/// cannot auto-acquire missing framework packages (VCLibs, WindowsAppRuntime,
/// UI.Xaml, NET.Native), so a sideload that needs an absent framework is doomed:
/// it either errors outright or registers a package that won't launch. When a
/// required framework is missing we proactively route to the portable build
/// instead of burning a failed install attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsixDependencyPrecheck {
    /// Whether the pre-check could actually evaluate framework presence. False
    /// when the manifest could not be read or the `Get-AppxPackage` probe could
    /// not run — in that case we do NOT block the sideload on an unknown signal.
    pub checked: bool,
    /// True when every required framework dependency is present (or none are
    /// declared). When false, `missing_frameworks` lists what is absent.
    pub frameworks_ok: bool,
    /// Framework packages the manifest requires that are NOT installed.
    pub missing_frameworks: Vec<String>,
    /// Human-facing reason; empty when `frameworks_ok` and there is nothing to say.
    pub reason: String,
}

impl MsixDependencyPrecheck {
    /// The pre-check has positively determined a required framework is missing,
    /// so the sideload should be skipped in favor of the portable build.
    pub fn should_route_portable(&self) -> bool {
        self.checked && !self.frameworks_ok && !self.missing_frameworks.is_empty()
    }
}

pub fn fetch_text(url: &str) -> Result<String, EngineError> {
    fetch_text_with_network_headers(url, &NetworkConfig::system(), &[])
}

pub fn fetch_text_with_network(url: &str, network: &NetworkConfig) -> Result<String, EngineError> {
    fetch_text_with_network_headers(url, network, &[])
}

/// Fetch a small HTTPS text resource without following redirects. Relay
/// metadata is served from a fixed same-origin path, so accepting a redirect
/// would allow an otherwise valid relay configuration to move the request to
/// an unrelated host.
pub fn fetch_text_with_network_no_redirect(
    url: &str,
    network: &NetworkConfig,
) -> Result<String, EngineError> {
    let source = url_host(url);
    let mut output = fetch_text_output(url, network, SchannelRevocationCheck::Strict, &[], false)?;
    if is_schannel_revocation_offline(
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).as_ref(),
    ) {
        output = fetch_text_output(url, network, SchannelRevocationCheck::Disabled, &[], false)?;
    }
    if !output.status.success() {
        return Err(EngineError::Io(curl_failure_message(
            url,
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        )));
    }
    if output.stdout.len() > MAX_TEXT_BYTES as usize {
        return Err(EngineError::Io(format!(
            "text response exceeded {MAX_TEXT_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| EngineError::Io(e.to_string()))?;
    log::debug!(
        "fetch Windows text completed without redirects source={source} bytes={}",
        text.len()
    );
    Ok(text)
}

/// Stable headers for callers that fetch GitHub's REST API.
pub const GITHUB_API_HEADERS: &[&'static str] = &[
    "Accept: application/vnd.github+json",
    "User-Agent: Codex-App-Manager",
];

/// Fetch a small HTTPS text resource with compile-time-trusted curl headers.
///
/// Header strings are deliberately required to be `static` and are passed as
/// separate process arguments, never through a shell. Do not pass user- or
/// network-provided header values to this API.
pub fn fetch_text_with_network_headers(
    url: &str,
    network: &NetworkConfig,
    headers: &[&'static str],
) -> Result<String, EngineError> {
    let source = url_host(url);
    log::debug!("fetch Windows text source={source}");
    let mut output =
        fetch_text_output(url, network, SchannelRevocationCheck::Strict, headers, true)?;
    let should_retry_without_revocation = {
        let stderr = String::from_utf8_lossy(&output.stderr);
        !output.status.success()
            && is_schannel_revocation_offline(output.status.code(), stderr.as_ref())
    };
    if should_retry_without_revocation {
        log::warn!(
            "Windows curl Schannel revocation check failed; retrying with --ssl-no-revoke source={source}"
        );
        output = fetch_text_output(
            url,
            network,
            SchannelRevocationCheck::Disabled,
            headers,
            true,
        )?;
    }

    if !output.status.success() {
        let err = EngineError::Io(curl_failure_message(
            url,
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ));
        log::warn!("fetch Windows text failed source={source} error={err}");
        return Err(err);
    }

    if output.stdout.len() > MAX_TEXT_BYTES as usize {
        let err = EngineError::Io(format!("text response exceeded {MAX_TEXT_BYTES} bytes"));
        log::warn!("fetch Windows text failed source={source} error={err}");
        return Err(err);
    }

    let text = String::from_utf8(output.stdout).map_err(|e| EngineError::Io(e.to_string()))?;
    let bytes = text.len();
    log::debug!("fetch Windows text completed source={source} bytes={bytes}");
    Ok(text)
}

fn fetch_text_output(
    url: &str,
    network: &NetworkConfig,
    revocation_check: SchannelRevocationCheck,
    headers: &[&'static str],
    follow_redirects: bool,
) -> Result<std::process::Output, EngineError> {
    // Metadata checks must fail fast enough for the UI to remain responsive
    // when a machine is offline or a regional route is temporarily blocked.
    // Package downloads use the separate long-running download limits below.
    const CONNECT_TIMEOUT_SECONDS: &str = "10";
    const MAX_TIME_SECONDS: &str = "15";
    const OUTER_TIMEOUT_SECONDS: u64 = 20;
    let max_text = MAX_TEXT_BYTES.to_string();
    let mut command = hidden_command(curl_exe());
    network.apply_to_command_with_schannel_revocation(&mut command, revocation_check);
    append_curl_headers(&mut command, headers);
    command.args([
        "-fsS",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        CONNECT_TIMEOUT_SECONDS,
        "--max-time",
        MAX_TIME_SECONDS,
        "--max-filesize",
        &max_text,
    ]);
    if follow_redirects {
        command.arg("-L");
    }
    command.arg(url);
    // curl's own --max-time is the primary budget; the outer deadline is a
    // backstop that also kills a hung curl that ignored max-time.
    run_capturing(
        command,
        RunLimits::total(std::time::Duration::from_secs(OUTER_TIMEOUT_SECONDS)),
        None,
    )
    .map_err(|e| EngineError::Io(format!("curl: {}", e.message())))
}

fn append_curl_headers(command: &mut std::process::Command, headers: &[&'static str]) {
    for header in headers {
        command.arg("-H").arg(*header);
    }
}

fn curl_failure_message(url: &str, exit_code: Option<i32>, stderr: &str) -> String {
    let base = format!(
        "curl failed for host={} exit={}: stderr='{}'",
        url_host(url),
        exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr.trim(),
    );
    // Append the proxy diagnostic only for connectivity failures — pasting it
    // onto write / disk / HTTP errors (e.g. exit 23) only misleads.
    if is_connectivity_exit(exit_code) {
        format!("{base}; {}", proxy_env_summary())
    } else {
        base
    }
}

fn is_connectivity_exit(exit_code: Option<i32>) -> bool {
    matches!(
        exit_code,
        Some(
            5 | 6
                | 7
                | 28
                | 35
                | 52
                | 53
                | 54
                | 55
                | 56
                | 58
                | 59
                | 60
                | 67
                | 77
                | 80
                | 82
                | 83
                | 91
        )
    )
}

fn url_host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
}

fn proxy_env_summary() -> String {
    let vars = ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"];
    let configured = vars
        .iter()
        .filter(|name| std::env::var_os(name).is_some())
        .copied()
        .collect::<Vec<_>>();
    if configured.is_empty() {
        "no curl proxy environment variables are set; Windows system proxy/PAC may not be used automatically".to_string()
    } else {
        format!(
            "curl proxy environment variables set: {}",
            configured.join(", ")
        )
    }
}

pub fn detect_installed_codex(portable_root: &Path) -> Option<InstalledWindowsCodex> {
    detect_msix_install().or_else(|| detect_portable_install(portable_root))
}

fn parse_registered_msix_package_full_name(output: &str) -> Result<Option<String>, EngineError> {
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(None);
    }
    let value: String = serde_json::from_str(trimmed)
        .map_err(|e| EngineError::Io(format!("parse registered MSIX package full name: {e}")))?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_string()))
}

/// Exact PackageFullName of the currently registered Codex MSIX. Historical
/// policy recovery compares this signed-package identity with the durable target
/// recorded immediately before Add-AppxPackage, rather than guessing from the
/// human-facing app version inside app.asar.
#[cfg(windows)]
pub fn registered_msix_package_full_name() -> Result<Option<String>, EngineError> {
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$p = Get-AppxPackage -Name {name} -ErrorAction Stop |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if ($null -eq $p) {{
  'null'
}} else {{
  [string]$p.PackageFullName | ConvertTo-Json -Compress
}}
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
    );
    let output = run_powershell_json(&script)?;
    parse_registered_msix_package_full_name(&output)
}

#[cfg(not(windows))]
pub fn registered_msix_package_full_name() -> Result<Option<String>, EngineError> {
    Ok(None)
}

#[cfg(windows)]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn normalize_win32_path_text(value: &str) -> String {
    // std::fs::canonicalize() returns an extended Win32 path on Windows. The
    // AppX PowerShell binder and the WinRT Uri constructor reject that spelling
    // on several supported Windows/.NET combinations, reporting an unrelated
    // "Invalid URI: hostname could not be parsed". Normalize the path before
    // it crosses the PowerShell boundary; ordinary drive and UNC paths remain
    // unchanged.
    const EXTENDED_UNC_PREFIX: &str = "\\\\?\\UNC\\";
    const EXTENDED_PREFIX: &str = "\\\\?\\";
    const NT_PREFIX: &str = "\\??\\";
    if value.len() >= EXTENDED_UNC_PREFIX.len()
        && value[..EXTENDED_UNC_PREFIX.len()].eq_ignore_ascii_case(EXTENDED_UNC_PREFIX)
    {
        return format!("\\\\{}", &value[EXTENDED_UNC_PREFIX.len()..]);
    }
    if value.len() >= EXTENDED_PREFIX.len()
        && value[..EXTENDED_PREFIX.len()].eq_ignore_ascii_case(EXTENDED_PREFIX)
    {
        return value[EXTENDED_PREFIX.len()..].to_string();
    }
    if value.len() >= NT_PREFIX.len() && value[..NT_PREFIX.len()].eq_ignore_ascii_case(NT_PREFIX) {
        return value[NT_PREFIX.len()..].to_string();
    }
    value.to_string()
}

#[cfg(windows)]
fn ps_nullable(value: Option<&str>) -> String {
    value.map(ps_quote).unwrap_or_else(|| "$null".to_string())
}

#[cfg(windows)]
fn powershell_exe() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|windir| {
            windir
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

/// Windows' CreateProcess command-line limit is about 32K UTF-16 code units.
/// USB AppX deployment scripts contain enough diagnostics and compatibility
/// branches to exceed that limit on some builds.  Keep short probes inline,
/// but transport larger generated scripts through a private, create-new temp
/// file so they cannot be truncated into a different PowerShell program.
#[cfg(windows)]
const MAX_INLINE_POWERSHELL_CHARS: usize = 12_000;

#[cfg(windows)]
fn write_temporary_powershell_script(script: &str) -> Result<PathBuf, String> {
    use std::io::Write;

    // Windows wall-clock resolution is not unique across concurrent probes.
    // Keep the existing PID-numeric filename contract used by owned-temp
    // cleanup, but generate the numeric suffix independently of the clock.
    let stamp = uuid::Uuid::new_v4().as_u128();
    let path = std::env::temp_dir().join(format!(
        "codex-win-engine-ps-{}-{stamp}.ps1",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("create temporary PowerShell script: {error}"))?;
    // Windows PowerShell 5.1 otherwise interprets UTF-8 as the active ANSI
    // code page.  The BOM keeps Chinese diagnostics and non-ASCII profile paths
    // intact on the target machines that exposed the original install error.
    file.write_all(&[0xEF, 0xBB, 0xBF])
        .and_then(|_| file.write_all(script.as_bytes()))
        .and_then(|_| file.flush())
        .map_err(|error| {
            let _ = std::fs::remove_file(&path);
            format!("write temporary PowerShell script: {error}")
        })?;
    Ok(path)
}

/// Typed PowerShell runner failure so callers can branch on timeout without
/// string-matching human messages.
#[cfg(windows)]
#[derive(Debug)]
enum PowerShellRunError {
    Timeout(TimeoutKind),
    Other(String),
}

#[cfg(windows)]
impl PowerShellRunError {
    fn message(&self) -> String {
        match self {
            Self::Timeout(TimeoutKind::Total) => {
                "powershell: process exceeded total deadline".to_string()
            }
            Self::Timeout(TimeoutKind::Stall) => {
                "powershell: process made no progress within stall timeout".to_string()
            }
            Self::Other(msg) => msg.clone(),
        }
    }

    fn into_capability(self) -> EngineError {
        EngineError::Capability(self.message())
    }

    fn into_install(self) -> EngineError {
        EngineError::Install(self.message())
    }
}

#[cfg(windows)]
fn run_powershell_json(script: &str) -> Result<String, EngineError> {
    run_powershell_json_with_limits(script, RunLimits::probe()).map_err(|e| e.into_capability())
}

#[cfg(windows)]
fn run_powershell_json_with_limits(
    script: &str,
    limits: RunLimits,
) -> Result<String, PowerShellRunError> {
    let mut command = hidden_command(powershell_exe());
    let script_path = if script.chars().count() > MAX_INLINE_POWERSHELL_CHARS {
        Some(write_temporary_powershell_script(script).map_err(PowerShellRunError::Other)?)
    } else {
        None
    };
    if let Some(path) = &script_path {
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        command.arg(path);
    } else {
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    }
    let output = run_capturing(command, limits, None);
    if let Some(path) = script_path {
        let _ = std::fs::remove_file(path);
    }
    let output = output.map_err(|e| match e {
        RunError::Timeout { kind, .. } => PowerShellRunError::Timeout(kind),
        other => PowerShellRunError::Other(format!("powershell: {}", other.message())),
    })?;
    if !output.status.success() {
        return Err(PowerShellRunError::Other(format!(
            "powershell failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// Plain hashtables keep the same JSON contract as PSCustomObject without using
// a type conversion that PowerShell ConstrainedLanguage rejects.
#[cfg(windows)]
const INSTALLED_MSIX_OBJECT: &str = r#"@{
  path = [string]$p.InstallLocation
  version = [string]$p.Version
  arch = $null
  source = 'msix'
  packageFamilyName = [string]$p.PackageFamilyName
  packageVolume = & {
    $locationInfo = Get-CodexPackagePathInfo ([string]$p.InstallLocation) ([string]$p.PackageFullName)
    if ([string]$locationInfo.effectivePath -match '^[A-Za-z]:\\') { ([string]$locationInfo.effectivePath).Substring(0, 3) } else { $null }
  }
}"#;

pub const APPX_PACKAGE_PATH_SCRIPT: &str = include_str!("appx_package_path.ps1");

/// Shared read-only classifier for the install recovery and USB uninstall
/// boundaries.  The helper intentionally observes only documented AppX
/// projections and filesystem metadata.  It never reads or edits the private
/// PackageRepository registry/database.
#[cfg(any(windows, test))]
const APPX_MACHINE_CLEANUP_STATE_SCRIPT: &str = r#"
function Get-CodexAppxOwnerSid($Owner) {
  if ($null -eq $Owner) { throw 'AppX owner record is null' }
  $value = $Owner.UserSecurityId
  $candidates = @()
  if ($null -ne $value) {
    if ($value -is [string]) {
      $candidates += $value
    } else {
      # Windows PowerShell has projected AppxUserSecurityId differently across
      # builds: some expose Sid, SecurityIdentifier exposes Value, and a few
      # stringify directly. Accept only a value that still parses as a SID.
      if ($value.PSObject.Properties.Name -contains 'Sid') { $candidates += $value.Sid }
      if ($value.PSObject.Properties.Name -contains 'Value') { $candidates += $value.Value }
      $candidates += [string]$value
    }
  }
  foreach ($candidate in $candidates) {
    $sid = ([string]$candidate).Trim()
    if ($sid -match '^S-\d+(?:-\d+)+$') { return $sid }
  }
  throw 'AppX owner SID is unavailable or malformed'
}

function Get-CodexMachineRecordVolumes([string]$FullName) {
  $command = Get-Command Get-AppxPackage -ErrorAction Stop
  if ($command.Parameters.Keys -notcontains 'Volume') {
    throw 'Get-AppxPackage -Volume is unavailable; staging-record volume cannot be proven'
  }
  $volumeCommand = Get-Command Get-AppxVolume -ErrorAction Stop
  $volumes = if ($volumeCommand.Parameters.Keys -contains 'Online') {
    @(Get-AppxVolume -Online -ErrorAction Stop)
  } else {
    @(Get-AppxVolume -ErrorAction Stop | Where-Object { -not [bool]$_.IsOffline })
  }
  $matches = @()
  foreach ($volume in $volumes) {
    if ($null -eq $volume -or [bool]$volume.IsOffline) { continue }
    $onVolume = @(Get-AppxPackage -Name 'OpenAI.Codex' -AllUsers -Volume $volume -ErrorAction Stop |
      Where-Object { [string]$_.PackageFullName -ieq $FullName })
    if ($onVolume.Count -gt 1) { throw 'One AppX volume returned duplicate exact Codex records' }
    if ($onVolume.Count -eq 1) { $matches += $volume }
  }
  return @($matches)
}

function Get-CodexMachineCleanupState([string]$ExpectedFullName = '') {
  $result = [ordered]@{
    classification = 'blocked'
    reason = $null
    packageFullName = $null
    installLocation = $null
    ownerStates = @()
    provisionedPackages = @()
    pathState = $null
    recordVolumeName = $null
    recordVolumeStore = $null
  }
  $current = @(Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop)
  $machine = @(Get-AppxPackage -Name 'OpenAI.Codex' -AllUsers -ErrorAction Stop)
  $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop |
    Where-Object {
      [string]$_.DisplayName -ieq 'OpenAI.Codex' -or
      (-not [string]::IsNullOrWhiteSpace($ExpectedFullName) -and
       [string]$_.PackageName -ieq $ExpectedFullName)
    })
  $result.provisionedPackages = @($provisioned | ForEach-Object {
    if (-not [string]::IsNullOrWhiteSpace([string]$_.PackageName)) {
      [string]$_.PackageName
    } else {
      [string]$_.DisplayName
    }
  })
  if ($current.Count -gt 0) {
    $result.classification = 'current-user-present'
    $result.packageFullName = [string]$current[0].PackageFullName
    $result.installLocation = [string]$current[0].InstallLocation
    $result.reason = if ($current.Count -eq 1) {
      'The current user still owns a Codex registration'
    } else {
      'Multiple current-user Codex registrations exist'
    }
    return $result
  }
  if ($machine.Count -eq 0) {
    if ($provisioned.Count -gt 0) {
      $result.reason = 'A provisioned Codex package exists without a current-user registration'
      return $result
    }
    $result.classification = 'none'
    $result.reason = 'No AppX-projected machine Codex package exists'
    return $result
  }
  if ($machine.Count -ne 1) {
    $result.reason = 'Multiple machine Codex package records exist'
    return $result
  }
  $package = $machine[0]
  $result.packageFullName = [string]$package.PackageFullName
  $result.installLocation = [string]$package.InstallLocation
  if ([string]$package.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0' -or
      [string]$package.PackageFullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$') {
    $result.reason = 'The machine query returned an unexpected Codex package identity'
    return $result
  }
  if (-not [string]::IsNullOrWhiteSpace($ExpectedFullName) -and
      [string]$package.PackageFullName -ine $ExpectedFullName) {
    $result.reason = 'The machine package differs from the Authenticode-authorized payload identity'
    return $result
  }
  if ($provisioned.Count -gt 0) {
    $result.reason = 'A provisioned Codex package exists'
    return $result
  }
  $owners = @($package.PackageUserInformation)
  foreach ($owner in $owners) {
    $state = [string]$owner.InstallState
    try { $ownerSid = Get-CodexAppxOwnerSid $owner }
    catch { $result.reason = [string]$_.Exception.Message; return $result }
    $result.ownerStates += ('{0}:{1}' -f $ownerSid, $state)
    # Staged is a machine staging record, not an installed user. Empty and
    # unknown states fail closed because another usable registration cannot be
    # disproved.
    if ($state -ne 'Staged') {
      $result.reason = 'The machine package has an installed or unknown owner'
      return $result
    }
  }
  $recordVolumeError = $null
  try {
    $recordVolumes = @(Get-CodexMachineRecordVolumes ([string]$package.PackageFullName))
    if ($recordVolumes.Count -eq 1) {
      $result.recordVolumeName = [string]$recordVolumes[0].Name
      $result.recordVolumeStore = [string]$recordVolumes[0].PackageStorePath
    } elseif ($recordVolumes.Count -gt 1) {
      $result.reason = 'The exact staged record appears on multiple AppX volumes'
      return $result
    }
  } catch {
    $recordVolumes = @()
    $recordVolumeError = [string]$_.Exception.Message
  }
  $rawLocation = ([string]$package.InstallLocation).Trim()
  if ([string]::IsNullOrWhiteSpace($rawLocation)) {
    $result.pathState = 'empty'
    $result.classification = 'orphan-candidate'
    $result.reason = 'The exact unowned package has an empty InstallLocation' +
      $(if ($recordVolumeError) { '; record-volume query=' + $recordVolumeError } else { '' })
    return $result
  }
  $pathInfo = Get-CodexPackagePathInfo $rawLocation ([string]$package.PackageFullName)
  $result.pathState = [string]$pathInfo.state
  if ([string]$pathInfo.state -in @('path-absent', 'dangling-system-junction')) {
    $result.classification = 'orphan-candidate'
    $result.reason = 'The exact unowned package has no usable package path' +
      $(if ($recordVolumeError) { '; record-volume query=' + $recordVolumeError } else { '' })
    return $result
  }
  if ($recordVolumes.Count -eq 1 -and [string]$pathInfo.state -in @('direct','verified-system-junction')) {
    $result.classification = 'staged-candidate'
    $result.reason = 'The exact unowned package is staged on one proven AppX volume'
    return $result
  }
  $result.reason = 'An unowned staged package still has an unprovable path; it was retained'
  return $result
}
"#;

/// Supported recovery for a machine staging record whose private deployment
/// metadata survived but whose package files did not.  The only bytes allowed
/// to repair the record are the caller's already Authenticode-authorized exact
/// MSIX.  Recovery uses public PackageManager Stage/Move APIs and revalidates
/// the official identity, owner state and volume after every transition.
#[cfg(any(windows, test))]
const APPX_MACHINE_REHYDRATION_SCRIPT: &str = r#"
function Get-CodexMachineRepairRecordVolume([string]$FullName, $Context) {
  $queryError = $null
  try { $matches = @(Get-CodexMachineRecordVolumes $FullName) }
  catch { $matches = @(); $queryError = [string]$_.Exception.Message }
  if ($matches.Count -gt 1) { throw 'The exact Codex staging record belongs to multiple AppX volumes' }
  if ($matches.Count -eq 1) {
    return @{ volume=$matches[0]; source='Get-AppxPackage -AllUsers -Volume' }
  }
  # A sufficiently damaged record can disappear from every per-volume
  # projection while remaining visible in the global all-user projection. In
  # that exact no-owner/no-provisioned case, rehydrate on the one verified
  # system AppX volume, then move/remove it immediately under the journal.
  $systems = @(Get-AppxVolume -Online -ErrorAction Stop | Where-Object {
    [bool]$_.IsSystemVolume -and -not [bool]$_.IsOffline -and
    [string]$_.Name -ieq [string]$Context.systemName
  })
  if ($systems.Count -ne 1) { throw 'Staging-record volume is unavailable and one verified system fallback volume cannot be proven' }
  return @{ volume=$systems[0]; source='verified-system-volume-fallback'; queryError=$queryError }
}

function Invoke-CodexRehydrateMachinePackage([string]$PackagePath, [string]$ExpectedSha256, [string]$FullName, $Context) {
  Assert-CodexRecoveryContextUnchanged $Context
  $payload = ConvertTo-CodexRecoveryLocalPath $PackagePath
  if ($ExpectedSha256 -notmatch '^[a-fA-F0-9]{64}$') {
    throw 'The Authenticode-authorized recovery MSIX has no valid SHA-256 binding'
  }
  $payloadPins = Lock-CodexRecoveryDirectory ([IO.Path]::GetDirectoryName($payload))
  $payloadStream = $null
  try {
    # Hold a no-write/no-delete file handle from hash verification until the
    # WinRT operation and its postcondition complete. This closes the local
    # staging-path replacement window during standalone uninstall cleanup.
    $payloadStream = [CodexAppxRecovery.NativeFiles]::OpenRead($payload)
    $actualSha256 = [CodexAppxRecovery.NativeFiles]::Hash($payloadStream)
    if ($actualSha256 -ine $ExpectedSha256) {
      throw "The pinned recovery MSIX changed after authorization: expectedSha256=$ExpectedSha256 actualSha256=$actualSha256"
    }
    $selection = Get-CodexMachineRepairRecordVolume $FullName $Context
    $record = $selection.volume
    if ($null -eq $record -or [bool]$record.IsOffline -or
        [string]$record.Name -notmatch '(?i)^\\\\\?\\Volume\{[0-9a-f-]{36}\}\\?$') {
      throw 'The selected AppX repair volume is invalid or offline'
    }
    $store = (ConvertTo-CodexRecoveryLocalPath ([string]$record.PackageStorePath)).TrimEnd('\')
    $root = [IO.Path]::GetPathRoot($store)
    if ([string]::IsNullOrWhiteSpace($root)) { throw 'The selected AppX repair volume has no local package-store root' }
    [uint64]$required = Get-CodexRecoveryDeploymentSpaceBudget $payload
    [uint64]$available = [uint64]([IO.DriveInfo]::new($root).AvailableFreeSpace)
    if ($available -lt $required) {
      throw "Insufficient temporary repair-volume space: recordVolume=$([string]$record.Name) requiredBytes=$required availableBytes=$available"
    }
    Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
    $manager = [Windows.Management.Deployment.PackageManager,Windows,ContentType=WindowsRuntime]::new()
    $nativeVolume = $manager.FindPackageVolume([string]$record.Name)
    if ($null -eq $nativeVolume -or [bool]$nativeVolume.IsOffline) {
      throw 'PackageManager could not resolve the selected online repair volume'
    }
    $uri = [Uri]::new($payload, [UriKind]::Absolute)
    if (-not $uri.IsFile) { throw 'Machine repair MSIX URI is not a local file URI' }
    $options = [Windows.Management.Deployment.DeploymentOptions]::ForceUpdateFromAnyVersion
    $operation = $manager.StagePackageAsync($uri, $null, $options, $nativeVolume)
    $result = Wait-CodexRecoveryDeployment $operation

    $packages = @($nativeVolume.FindPackages('OpenAI.Codex_2p2nqsd0c76g0') |
      Where-Object { [string]$_.Id.FullName -ieq $FullName })
    if ($packages.Count -ne 1 -or -not [bool]$packages[0].Status.VerifyIsOK()) {
      throw 'Exact Codex payload was not restored as one healthy staged package on the repair volume'
    }
    $actual = ConvertTo-CodexRecoveryLocalPath ([string]$packages[0].InstalledLocation.Path)
    $expected = Join-Path $store $FullName
    if ($actual.TrimEnd('\') -ine $expected.TrimEnd('\') -or
        -not [CodexAppxRecovery.NativeFiles]::Exists($actual, $true)) {
      throw "Rehydrated package path is not the selected AppX store: expected=$expected actual=$actual"
    }
    return @{
      activityId=[string]$result.ActivityId
      recordVolumeName=[string]$record.Name
      recordVolumeStore=$store
      recordVolumeSource=[string]$selection.source
      payloadSha256=$actualSha256
      operation='StagePackageAsync exact-payload rehydration'
    }
  } finally {
    if ($payloadStream) { $payloadStream.Dispose() }
    Close-CodexRecoveryHandles $payloadPins
  }
}

function Invoke-CodexMoveRehydratedPackageToUsb([string]$FullName, $Context) {
  Assert-CodexRecoveryContextUnchanged $Context
  $already = Get-CodexRecoveryStagedPackage $FullName $Context
  if ($null -ne $already) {
    return @{activityId=$null;operation='already staged on bound USB'}
  }
  $state = Get-CodexMachineCleanupState $FullName
  if ([string]$state.classification -ne 'staged-candidate' -or
      [string]$state.packageFullName -ine $FullName) {
    throw ('Rehydrated package is not an exact exclusive staged candidate: ' + [string]$state.reason)
  }
  $native = Get-CodexRecoveryNativeVolumes $Context
  $operation = $native.manager.MovePackageToVolumeAsync(
    $FullName,
    [Windows.Management.Deployment.DeploymentOptions]::ForceApplicationShutdown,
    $native.usb)
  $result = Wait-CodexRecoveryDeployment $operation
  $moved = Get-CodexRecoveryStagedPackage $FullName $Context
  Assert-CodexRecoveryPackage $moved $FullName $Context
  return @{activityId=[string]$result.ActivityId;operation='MovePackageToVolumeAsync bound USB'}
}
"#;

#[cfg(any(windows, test))]
// 0x80073CFA is a generic uninstall failure, NOT a preserve-switch diagnosis.
const PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN: &str = r"(?is)PreserveApplicationData.{0,120}(development mode|deployment mode|开发模式|部署模式)|(development mode|deployment mode|开发模式|部署模式).{0,120}PreserveApplicationData";

/// PreserveApplicationData only works for development-mode registrations.
/// Before omitting it for an ordinary signed package, preserve a verified
/// recoverable copy; never turn "keep data" into silent permanent deletion.
#[cfg(any(windows, test))]
const APPX_DATA_BACKUP_SCRIPT: &str = concat!(
    include_str!("appx_package_path.ps1"),
    include_str!("appx_data_recovery.ps1"),
    r#"
function Assert-CodexPackageDataAbsent {
  $source = Join-Path (Join-Path $env:LOCALAPPDATA 'Packages') 'OpenAI.Codex_2p2nqsd0c76g0'
  Assert-CodexBackupNoLinkAncestors $source
  try { $null = Get-Item -LiteralPath $source -Force -ErrorAction Stop }
  catch {
    if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) { return }
    throw
  }
  throw 'Package data appeared after the absent-data snapshot; removal was not started'
}
function Get-CodexBackupHash([string]$path) {
  Initialize-CodexRecoveryFileHandles
  $stream = [CodexAppxRecovery.NativeFiles]::OpenRead($path)
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
  finally { $sha.Dispose(); $stream.Dispose() }
}
function Copy-CodexBackupFile([string]$source, [string]$destination) {
  Initialize-CodexRecoveryFileHandles
  $read = $null; $write = $null
  try {
    # Both calls open each directory component relative to an ordinary parent
    # handle with OBJ_DONT_REPARSE. A junction created after inventory cannot
    # redirect either the read or the write to another directory.
    $read = [CodexAppxRecovery.NativeFiles]::OpenRead($source)
    $write = [CodexAppxRecovery.NativeFiles]::CreateNew($destination)
    $read.CopyTo($write); $write.Flush($true)
  } finally { if ($write) { $write.Dispose() }; if ($read) { $read.Dispose() } }
}
function New-CodexBackupDirectory([string]$path) {
  Initialize-CodexRecoveryFileHandles
  $pins = [CodexAppxRecovery.NativeFiles]::LockDirectories($path, $true)
  Close-CodexRecoveryHandles $pins
}
function Get-CodexBackupEntries([string]$path) {
  Initialize-CodexRecoveryFileHandles
  foreach ($native in @([CodexAppxRecovery.NativeFiles]::Entries($path))) {
    $child = [IO.Path]::Combine($path, [string]$native.Name)
    $entry = @{ FullName = $child; Name = [string]$native.Name; PSIsContainer = $native.PSIsContainer; Attributes = $native.Attributes; LinkType = ''; Target = @() }
    if (($native.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      # Link metadata is classification only. No file contents are read through
      # this path; the exact accepted target is opened independently below.
      $link = Get-Item -LiteralPath $child -Force -ErrorAction Stop
      if (($link.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) { throw 'Package-data link changed during inventory' }
      $entry.LinkType = [string]$link.LinkType; $entry.Target = @($link.Target)
    }
    $entry
  }
}
function Assert-CodexBackupNoLinkAncestors([string]$path) {
  $cursor = [IO.Path]::GetFullPath($path)
  while (-not [string]::IsNullOrWhiteSpace($cursor)) {
    try {
      $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
      if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Backup path contains a link/junction: $cursor; removal was not started"
      }
    } catch {
      if ($_.CategoryInfo.Category -ne [Management.Automation.ErrorCategory]::ObjectNotFound) { throw }
    }
    $parent = [IO.Path]::GetDirectoryName($cursor.TrimEnd('\'))
    if ($parent -eq $cursor) { break }
    $cursor = $parent
  }
}
function Get-CodexContainedDirectoryLinkTarget($entry, [string]$root) {
  # ReparsePoint is a category, not evidence that this is an empty AC cache.
  # Only ordinary Windows directory junctions/symlinks with one absolute,
  # non-linked target inside this exact package-data root are understood here.
  if (-not $entry.PSIsContainer -or [string]$entry.LinkType -notin @('Junction', 'SymbolicLink')) {
    throw "Cannot safely preserve unknown linked package data: $($entry.FullName); linkType=$([string]$entry.LinkType) target=$(@($entry.Target) -join '|') attributes=$([string]$entry.Attributes); removal was not started"
  }
  $targets = @($entry.Target)
  if ($targets.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$targets[0])) {
    throw "Cannot resolve package-data link: $($entry.FullName); removal was not started"
  }
  $target = [string]$targets[0]
  if ($target.StartsWith('\??\')) { $target = $target.Substring(4) }
  if ($target.StartsWith('\\?\')) { $target = $target.Substring(4) }
  if ($target -notmatch '^[A-Za-z]:\\') { throw 'Package-data link target is not an absolute local path; removal was not started' }
  $target = [IO.Path]::GetFullPath($target).TrimEnd('\')
  $rootPrefix = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
  $entryPath = ([string]$entry.FullName).TrimEnd('\')
  if (-not $target.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
      ($entryPath + '\').StartsWith($target + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw "Package-data link escapes the package root or forms a cycle: $entryPath -> $target; removal was not started"
  }
  Assert-CodexBackupNoLinkAncestors $target
  $targetItem = Get-Item -LiteralPath $target -Force -ErrorAction Stop
  if (-not $targetItem.PSIsContainer) { throw 'Package-data directory link target is not an available directory' }
  return $target
}
function Get-CodexAppxVolumeId([string]$name) {
  if ($name -notmatch '(?i)^\\\\\?\\Volume\{(?<id>[0-9a-f-]{36})\}\\?$') {
    throw 'AppX volume does not expose a canonical volume GUID; removal was not started'
  }
  return ([guid]$Matches.id).ToString('D')
}
function Get-CodexReparseTag([string]$path) {
  $fsutil = Join-Path ([Environment]::GetFolderPath('System')) 'fsutil.exe'
  $output = @(& $fsutil reparsepoint query $path 2>&1) -join "`n"
  if ($LASTEXITCODE -ne 0) { throw "Cannot read package data reparse tag: $path; removal was not started" }
  $tag = [regex]::Match($output, '(?i)\b0x[0-9a-f]{8}\b').Value
  if ([string]::IsNullOrWhiteSpace($tag)) { throw 'Package data reparse tag could not be parsed; removal was not started' }
  return $tag
}
function Get-CodexWpSystemBackupTarget($entry, [string]$root, [string]$authorizedRoot, [string]$authorizedId) {
  if ([string]::IsNullOrWhiteSpace($authorizedRoot) -or [string]::IsNullOrWhiteSpace($authorizedId)) {
    throw 'Cross-volume package data needs the verified USB volume identity; removal was not started'
  }
  $authorizedRoot = [IO.Path]::GetFullPath($authorizedRoot).TrimEnd('\') + '\'
  if ($authorizedRoot -notmatch '^[A-Za-z]:\\$') { throw 'Authorized AppX volume is not a drive root' }
  if ($authorizedRoot.TrimEnd('\') -ieq ([string]$env:SystemDrive).TrimEnd('\')) { throw 'WpSystem USB authorization cannot name the Windows system volume' }
  $identity = Get-CodexAppxVolumeId $authorizedId
  $family = 'OpenAI.Codex_2p2nqsd0c76g0'
  $expectedRoot = Join-Path (Join-Path $env:LOCALAPPDATA 'Packages') $family
  if ([IO.Path]::GetFullPath($root).TrimEnd('\') -ine [IO.Path]::GetFullPath($expectedRoot).TrimEnd('\')) {
    throw 'WpSystem backup source is not this user exact package root'
  }
  $leaf = [IO.Path]::GetFileName(([string]$entry.FullName).TrimEnd('\'))
  if ([IO.Path]::GetDirectoryName(([string]$entry.FullName).TrimEnd('\')) -ine $root.TrimEnd('\') -or
      $leaf -notin @('AC', 'AppData', 'LocalCache', 'LocalState', 'RoamingState', 'Settings', 'SystemAppData', 'TempState') -or
      -not $entry.PSIsContainer -or [string]$entry.LinkType -ne 'Junction') {
    throw 'Only direct standard package-state junctions can use WpSystem backup'
  }
  if ((Get-CodexReparseTag ([string]$entry.FullName)) -ine '0xa0000003') {
    throw 'WpSystem package-state link is not an NTFS mount-point junction'
  }
  $targets = @($entry.Target)
  if ($targets.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$targets[0])) { throw 'WpSystem junction has no unique target' }
  $target = [string]$targets[0]
  if ($target.StartsWith('\??\') -or $target.StartsWith('\\?\')) { $target = $target.Substring(4) }
  if ($target -notmatch '^[A-Za-z]:\\') { throw 'WpSystem junction target is not a local drive path' }
  $target = [IO.Path]::GetFullPath($target).TrimEnd('\')
  $sid = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $expectedTarget = [IO.Path]::Combine($authorizedRoot, "WpSystem\$sid\AppData\Local\Packages\$family\$leaf")
  if ($target -ine $expectedTarget) {
    throw "WpSystem junction does not match the authorized volume/current SID/package/state directory: $target; removal was not started"
  }
  Assert-CodexBackupNoLinkAncestors $target
  $targetItem = Get-Item -LiteralPath $target -Force -ErrorAction Stop
  if (-not $targetItem.PSIsContainer) { throw 'WpSystem state directory is unavailable' }
  $volumes = @(Get-AppxVolume -Path $authorizedRoot -Online -ErrorAction Stop)
  if ($volumes.Count -ne 1 -or [bool]$volumes[0].IsOffline -or
      ([string]$volumes[0].PackageStorePath).TrimEnd('\') -ine ([IO.Path]::Combine($authorizedRoot, 'WindowsApps'))) {
    throw 'Authorized WpSystem AppX volume is offline, ambiguous or has the wrong package store'
  }
  $actualId = Get-CodexAppxVolumeId ([string]$volumes[0].Name)
  $physical = @(Get-Volume -DriveLetter $authorizedRoot.Substring(0, 1) -ErrorAction Stop)
  if ($physical.Count -ne 1 -or [string]$physical[0].FileSystem -ine 'NTFS' -or
      $actualId -ine $identity -or
      (Get-CodexAppxVolumeId ([string]$physical[0].UniqueId)) -ine $identity -or
      (Get-CodexAppxVolumeId ([string]$physical[0].Path)) -ine $identity) {
    throw 'WpSystem volume identity changed or cannot be verified; removal was not started'
  }
  return @{ target = $target; identity = $identity; sid = $sid }
}
function Get-CodexDataInventory([string]$root, [string]$authorizedRoot = '', [string]$authorizedId = '') {
  Assert-CodexBackupNoLinkAncestors $root
  $items = New-Object 'System.Collections.Generic.List[object]'
  $seenLinkTargets = @{}
  $pending = New-Object 'System.Collections.Generic.Queue[object]'
  $pending.Enqueue(@{ path = $root; relative = '' })
  while ($pending.Count -gt 0) {
    $directory = $pending.Dequeue()
    foreach ($entry in @(Get-CodexBackupEntries ([string]$directory.path))) {
      if ([IO.Path]::GetDirectoryName(([string]$entry.FullName).TrimEnd('\')) -ine ([string]$directory.path).TrimEnd('\')) {
        throw 'Package-data enumeration escaped the verified read directory'
      }
      $leaf = [IO.Path]::GetFileName(([string]$entry.FullName).TrimEnd('\'))
      $relative = if ($directory.relative) { Join-Path $directory.relative $leaf } else { $leaf }
      $entry | Add-Member -NotePropertyName CodexBackupRelativePath -NotePropertyValue $relative -Force
      $readPath = [string]$entry.FullName
      if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        try { $target = Get-CodexContainedDirectoryLinkTarget $entry $root }
        catch {
          # This additional classification is BACKUP ONLY. It must never be
          # used by the local-data placement gate to call USB data local.
          if ([string]::IsNullOrWhiteSpace($authorizedRoot)) { throw }
          $wp = Get-CodexWpSystemBackupTarget $entry $root $authorizedRoot $authorizedId
          $target = [string]$wp.target
          $entry | Add-Member -NotePropertyName CodexVerifiedWpSystemIdentity -NotePropertyValue ([string]$wp.identity) -Force
        }
        if ($seenLinkTargets.ContainsKey($target)) { throw 'Package-data link cycle/duplicate target; removal was not started' }
        $seenLinkTargets[$target] = $true
        $entry | Add-Member -NotePropertyName CodexVerifiedLinkTarget -NotePropertyValue $target -Force
        # Traverse the already verified target directly, never the original
        # junction path that could be retargeted between inspection and read.
        $readPath = $target
      }
      $items.Add($entry)
      if ($entry.PSIsContainer) { $pending.Enqueue(@{ path = $readPath; relative = $relative }) }
    }
  }
  return $items
}
function Backup-CodexPackageData($package, [string]$authorizedRoot = '', [string]$authorizedId = '') {
  $family = [string]$package.PackageFamilyName
  if ($family -ine 'OpenAI.Codex_2p2nqsd0c76g0') {
    throw 'Cannot back up package data: unexpected package family identity'
  }
  if ([string]::IsNullOrWhiteSpace([string]$env:LOCALAPPDATA)) {
    throw 'Cannot back up package data: LOCALAPPDATA is missing'
  }
  $source = Join-Path (Join-Path $env:LOCALAPPDATA 'Packages') $family
  try { $sourceItem = Get-Item -LiteralPath $source -Force -ErrorAction Stop }
  catch {
    if ($_.CategoryInfo.Category -ne [Management.Automation.ErrorCategory]::ObjectNotFound) { throw }
    # An absent target below a disconnected USB junction is not absent data.
    Assert-CodexBackupNoLinkAncestors $source
    return $null
  }
  if (-not $sourceItem.PSIsContainer) { throw 'Package data root is not a directory' }
  # An arbitrary root junction is not proof of Windows-owned WpSystem data.
  # Reject unresolved/redirected roots instead of reading another user's tree.
  Assert-CodexBackupNoLinkAncestors $source
  $items = @(Get-CodexDataInventory ([string]$sourceItem.FullName) $authorizedRoot $authorizedId)
  $backupBase = Join-Path $env:LOCALAPPDATA 'CodexAppxDataBackups'
  if ([IO.Path]::GetPathRoot([IO.Path]::GetFullPath($backupBase)).TrimEnd('\') -ine ([string]$env:SystemDrive).TrimEnd('\')) {
    throw 'Recoverable package data backup must be on the local Windows system volume; removal was not started'
  }
  $backup = Join-Path (Join-Path $backupBase $family) ([guid]::NewGuid().ToString('N'))
  Assert-CodexBackupNoLinkAncestors $backup
  New-CodexBackupDirectory $backup
  $manifest = @()
  $directoryManifest = @{}
  $volumeManifest = @{}
  foreach ($entry in $items) {
    $entryPath = [string]$entry.FullName
    $relative = [string]$entry.CodexBackupRelativePath
    if ([string]::IsNullOrWhiteSpace($relative) -or [IO.Path]::IsPathRooted($relative) -or
        ($relative.Split('\') | Where-Object { $_ -in @('.', '..', '') })) { throw 'Unsafe package-data relative path' }
    $destination = Join-Path $backup $relative
    Assert-CodexBackupNoLinkAncestors $destination
    if ($entry.PSIsContainer) {
      New-CodexBackupDirectory $destination
      $directoryManifest[$relative] = [string]$entry.CodexVerifiedLinkTarget
      $volumeManifest[$relative] = [string]$entry.CodexVerifiedWpSystemIdentity
    } else {
      $parent = Split-Path -Parent $destination
      if (-not (Test-Path -LiteralPath $parent)) {
        New-CodexBackupDirectory $parent
      }
      Copy-CodexBackupFile $entryPath $destination
      $sourceHash = Get-CodexBackupHash $entryPath
      $copyHash = Get-CodexBackupHash $destination
      if ($sourceHash -ne $copyHash) { throw "Package data changed during backup: $entryPath" }
      $manifest += @{ path = $relative; sha256 = $copyHash }
    }
  }
  # Check again after copying: new/deleted files and concurrent edits must not
  # turn a partial copy into permission to discard the live data directory.
  $finalItems = @(Get-CodexDataInventory ([string]$sourceItem.FullName) $authorizedRoot $authorizedId)
  if ($finalItems.Count -ne $items.Count) { throw 'Package data inventory changed during backup; removal was not started' }
  $expectedFiles = @{}
  foreach ($record in $manifest) { $expectedFiles[[string]$record.path] = [string]$record.sha256 }
  foreach ($entry in $finalItems) {
    $relative = [string]$entry.CodexBackupRelativePath
    if ($entry.PSIsContainer) {
      if (-not $directoryManifest.ContainsKey($relative) -or
          [string]$entry.CodexVerifiedLinkTarget -cne $directoryManifest[$relative] -or
          [string]$entry.CodexVerifiedWpSystemIdentity -cne $volumeManifest[$relative]) {
        throw 'Package data directory/link changed during backup; removal was not started'
      }
    } else {
      if (-not $expectedFiles.ContainsKey($relative) -or
          (Get-CodexBackupHash ([string]$entry.FullName)) -ne $expectedFiles[$relative]) {
        throw 'Package data content changed during backup; removal was not started'
      }
    }
  }
  # Preserve the inventory next to, not inside, the recovered data tree.
  Assert-CodexBackupNoLinkAncestors ($backup + '.manifest.json')
  $manifestJson = @{ source = $source; packageFamilyName = $family; files = @($manifest); directories = $directoryManifest; appxVolumes = $volumeManifest } | ConvertTo-Json -Depth 6
  $manifestStream = [CodexAppxRecovery.NativeFiles]::CreateNew($backup + '.manifest.json')
  try { $bytes = [Text.Encoding]::UTF8.GetBytes($manifestJson); $manifestStream.Write($bytes, 0, $bytes.Length); $manifestStream.Flush($true) }
  finally { $manifestStream.Dispose() }
  return $backup
}
function Assert-CodexBackupStillMatches($package, [string]$backup, [string]$authorizedRoot = '', [string]$authorizedId = '') {
  if ([string]$package.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0') { throw 'Backup recheck package identity mismatch' }
  $family = [string]$package.PackageFamilyName
  $base = Join-Path (Join-Path $env:LOCALAPPDATA 'CodexAppxDataBackups') $family
  $fullBackup = [IO.Path]::GetFullPath($backup).TrimEnd('\')
  if ([IO.Path]::GetDirectoryName($fullBackup) -ine $base -or
      [IO.Path]::GetFileName($fullBackup) -notmatch '^[0-9a-f]{32}$') { throw 'Backup recheck path is outside the exact package backup store' }
  Assert-CodexBackupNoLinkAncestors $fullBackup
  Assert-CodexBackupNoLinkAncestors ($fullBackup + '.manifest.json')
  Initialize-CodexRecoveryFileHandles
  $manifestStream = [CodexAppxRecovery.NativeFiles]::OpenRead($fullBackup + '.manifest.json')
  $reader = $null
  try {
    if ($manifestStream.Length -gt 32MB) { throw 'Package backup manifest is too large' }
    $reader = New-Object IO.StreamReader($manifestStream, [Text.Encoding]::UTF8, $true)
    $manifest = $reader.ReadToEnd() | ConvertFrom-Json -ErrorAction Stop
  } finally { if ($reader) { $reader.Dispose() } else { $manifestStream.Dispose() } }
  $source = Join-Path (Join-Path $env:LOCALAPPDATA 'Packages') $family
  if ([string]$manifest.packageFamilyName -ine $family -or [string]$manifest.source -ine $source) { throw 'Backup manifest source identity mismatch' }
  $items = @(Get-CodexDataInventory $source $authorizedRoot $authorizedId)
  $files = @{}
  foreach ($record in @($manifest.files)) {
    if ($null -eq $record) { continue }
    $relative = [string]$record.path
    if ([string]::IsNullOrWhiteSpace($relative) -or [IO.Path]::IsPathRooted($relative) -or
        ($relative.Split('\') | Where-Object { $_ -in @('.', '..', '') }) -or $files.ContainsKey($relative)) {
      throw 'Backup manifest contains an unsafe/duplicate file path'
    }
    $files[$relative] = [string]$record.sha256
    $copy = Join-Path $fullBackup $relative
    Assert-CodexBackupNoLinkAncestors $copy
    if ((Get-CodexBackupHash $copy) -cne [string]$record.sha256) { throw 'Verified backup content changed before removal' }
  }
  $directories = @($manifest.directories.PSObject.Properties | Where-Object { $_.MemberType -eq 'NoteProperty' })
  if ($items.Count -ne ($files.Count + $directories.Count)) { throw 'Package data inventory changed before removal' }
  foreach ($entry in $items) {
    $relative = [string]$entry.CodexBackupRelativePath
    if ($entry.PSIsContainer) {
      $directory = $manifest.directories.PSObject.Properties[$relative]
      $volume = $manifest.appxVolumes.PSObject.Properties[$relative]
      if ($null -eq $directory -or $null -eq $volume -or
          [string]$directory.Value -cne [string]$entry.CodexVerifiedLinkTarget -or
          [string]$volume.Value -cne [string]$entry.CodexVerifiedWpSystemIdentity) {
        throw 'Package-data link or volume identity changed before removal'
      }
    } elseif (-not $files.ContainsKey($relative) -or (Get-CodexBackupHash ([string]$entry.FullName)) -cne $files[$relative]) {
      throw 'Package data content changed before removal'
    }
  }
}
function Get-CodexPackageDataPlacement([string]$usbVolume) {
  $source = Join-Path (Join-Path $env:LOCALAPPDATA 'Packages') 'OpenAI.Codex_2p2nqsd0c76g0'
  $systemRoot = ([string]$env:SystemDrive).TrimEnd('\') + '\'
  $sourceRoot = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($source))
  if ($systemRoot -notmatch '^[A-Za-z]:\\$' -or $sourceRoot -ine $systemRoot -or
      $sourceRoot.TrimEnd('\') -ieq $usbVolume.TrimEnd('\')) {
    return @{ state = 'volume-unverified'; source = $source; targets = @(); detail = 'Package data root is not on the distinct Windows system volume' }
  }
  try { $entry = Get-Item -LiteralPath $source -Force -ErrorAction Stop }
  catch {
    if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
      Assert-CodexBackupNoLinkAncestors $source
      return @{ state = 'not-created'; source = $source; targets = @() }
    }
    throw
  }
  if (-not $entry.PSIsContainer) { throw 'Package data root is not a directory' }
  $observations = @()
  # Inspect only the package root, its ancestors and direct state directories.
  # Do not enumerate user files or mistake a registry policy readback for the
  # actual data location Windows chose on this edition/build.
  $cursor = $source
  while (-not [string]::IsNullOrWhiteSpace($cursor)) {
    $observations += Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
    $parent = [IO.Path]::GetDirectoryName($cursor.TrimEnd('\'))
    if ($cursor -eq $parent) { break }; $cursor = $parent
  }
  $links = @()
  foreach ($item in $observations) {
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      $links += @{ path = [string]$item.FullName; type = [string]$item.LinkType; target = @($item.Target) }
    }
  }
  if ($links.Count) {
    # Do not follow a redirected root/ancestor, even when its first hop spells
    # C:. It may be a GUID volume mount or a chain ending on removable media.
    return @{ state = 'redirected-unverified'; source = $source; targets = $links }
  }
  $unknownLink = $false
  foreach ($item in @(Get-ChildItem -LiteralPath $source -Force -ErrorAction Stop | Where-Object { $_.PSIsContainer })) {
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      $link = @{ path = [string]$item.FullName; type = [string]$item.LinkType; target = @($item.Target) }
      try {
        # The same strict resolver used for backups proves a one-hop target
        # inside this ordinary system-volume package root with no linked target
        # ancestors. Unknown tags, GUID targets and multi-hop redirects fail.
        $link.resolvedTarget = Get-CodexContainedDirectoryLinkTarget $item $source
      } catch {
        $unknownLink = $true
        $link.detail = [string]$_.Exception.Message
      }
      $links += $link
    }
  }
  if (-not $unknownLink) {
    try {
      # The direct state directories are only the first boundary. A nested
      # junction or file link can still put data on USB. Inspect names/link
      # structure recursively with the safe reader, but do not hash/log user
      # contents and do NOT grant the backup-only WpSystem authorization.
      $null = @(Get-CodexDataInventory $source)
    } catch {
      $unknownLink = $true
      $links += @{ path = $source; type = 'nested-inventory-unverified'; detail = [string]$_.Exception.Message }
    }
  }
  return @{ state = if ($unknownLink) { 'redirected-unverified' } elseif ($links.Count) { 'contained-links-verified' } else { 'system-volume-observed' }; source = $source; targets = $links }
}
"#
);

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfflineAppxConflictReport {
    active: bool,
    activity_id: Option<String>,
    file_id: Option<String>,
    message: String,
}

#[cfg(windows)]
fn parse_offline_appx_conflict_probe(
    result: Result<String, PowerShellRunError>,
) -> Option<OfflineAppxConflictReport> {
    let json = match result {
        Ok(json) => json,
        Err(err) => {
            log::warn!(
                "offline AppX conflict probe unavailable; proceeding with verified local MSIX error={}",
                err.message()
            );
            return None;
        }
    };
    match serde_json::from_str(&json) {
        Ok(report) => Some(report),
        Err(err) => {
            log::warn!(
                "offline AppX conflict probe returned invalid JSON; proceeding with verified local MSIX error={err}"
            );
            None
        }
    }
}

#[cfg(windows)]
fn validated_delivery_optimization_file_id(value: Option<String>) -> String {
    let value = value.unwrap_or_default();
    if value.is_empty()
        || ((40..=128).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return value;
    }
    // The AppX event names this field `Path`; builds may emit an actual path or
    // URI instead of a Delivery Optimization FileId. Cache cleanup is optional,
    // so ignore an untrusted shape without blocking the verified local MSIX.
    log::warn!(
        "AppX conflict event had a non-FileId Path payload; skipping Delivery Optimization cache cleanup"
    );
    String::new()
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevatedRecoveryLaunchReport {
    started: bool,
    exit_code: i32,
    error: String,
}

#[cfg(windows)]
struct OfflineAppxRestoreGuard {
    signal: Option<PathBuf>,
}

#[cfg(windows)]
const OFFLINE_APPX_RESTORE_WATCHDOG_SECS: u64 = 10 * 60;

/// Leave enough time before the unelevated launcher's hard deadline for the
/// elevated watchdog to terminate a wedged recovery helper and restart the
/// services it paused. This deadline is absolute, so time spent on the UAC
/// prompt cannot create a detached five-minute recovery window of its own.
#[cfg(windows)]
const OFFLINE_APPX_RECOVERY_CLEANUP_MARGIN_SECS: u64 = 60;

#[cfg(windows)]
impl Drop for OfflineAppxRestoreGuard {
    fn drop(&mut self) {
        if let Some(path) = self.signal.take() {
            if let Err(err) = std::fs::write(&path, b"restore") {
                log::error!(
                    "signal Windows update service restoration failed path={} error={err}",
                    path.display()
                );
            } else {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    let _ = std::fs::remove_file(path);
                });
            }
        }
    }
}

#[cfg(windows)]
fn detect_offline_appx_conflict_script(package_moniker: &str) -> String {
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$TargetMoniker = {package_moniker}
$DeploymentLog = 'Microsoft-Windows-AppXDeploymentServer/Operational'

function Get-ActiveWindowsUpdateDeployment {{
  try {{
    $starts = @(Get-WinEvent -FilterHashtable @{{
      LogName = $DeploymentLog
      Id = 10002
      StartTime = (Get-Date).AddDays(-7)
    }} -ErrorAction Stop)
    foreach ($event in $starts) {{
      $xml = [xml]$event.ToXml()
      $moniker = @($xml.Event.EventData.Data |
        Where-Object {{ $_.Name -eq 'MainPackageMoniker' }} |
        Select-Object -First 1)
      if ($moniker.Count -eq 0 -or [string]$moniker[0].'#text' -ne $TargetMoniker) {{ continue }}
      $activityId = $event.ActivityId.ToString('B')
      $related = @(Get-WinEvent -LogName $DeploymentLog -FilterXPath "*[System[Correlation[@ActivityID='$activityId']]]" -ErrorAction Stop)
      # 400 with Opcode=2 (Stop) is the provider's successful-completion
      # record. 401/404 are failed-completion records; 603 is the start record.
      $terminal = @($related | Where-Object {{
        ([int]$_.Id -eq 400 -and [int]$_.Opcode -eq 2) -or
        (401, 404 -contains [int]$_.Id)
      }})
      if ($terminal.Count -gt 0) {{ continue }}
      $wuStart = @($related | Where-Object {{ [int]$_.Id -eq 603 }} | Select-Object -First 1)
      if ($wuStart.Count -eq 0) {{ continue }}
      $wuXml = [xml]$wuStart[0].ToXml()
      $operation = [string](@($wuXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'DeploymentOperation' }} | Select-Object -First 1)[0].'#text')
      $caller = [string](@($wuXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'CallingProcess' }} | Select-Object -First 1)[0].'#text')
      $owner = [string]$wuXml.Event.System.Security.UserID
      if ($operation -ne '4' -or $caller -notmatch '(^|,)wuauserv($|,)' -or $owner -ne 'S-1-5-18') {{ continue }}
      $pathData = @($wuXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'Path' }} | Select-Object -First 1)
      $fileId = if ($pathData.Count -gt 0) {{ [string]$pathData[0].'#text' }} else {{ '' }}
      return @{{ activityId = $activityId; fileId = $fileId }}
    }}
  }} catch {{
    return $null
  }}
  return $null
}}

$activeDeployment = Get-ActiveWindowsUpdateDeployment
if ($null -eq $activeDeployment) {{
  @{{ active = $false; activityId = $null; fileId = $null; message = 'No conflicting SYSTEM Windows Update deployment' }} |
    ConvertTo-Json -Compress -Depth 3
  exit 0
}}
@{{
  active = $true
  activityId = [string]$activeDeployment.activityId
  fileId = [string]$activeDeployment.fileId
  message = 'SYSTEM Windows Update owns an active stage operation for the exact target package'
}} | ConvertTo-Json -Compress -Depth 3
"#,
        package_moniker = ps_quote(package_moniker),
    )
}

#[cfg(windows)]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(windows)]
fn encode_powershell_command(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect::<Vec<_>>();
    base64_encode(&bytes)
}

/// Builds the narrowly scoped elevated recovery as an inline command. No
/// executable script is written to a user-writable directory; temporary paths
/// are one-way state markers shared with the elevated watchdogs.
#[cfg(windows)]
fn offline_appx_recovery_script(
    package_moniker: &str,
    activity_id: &str,
    file_id: &str,
    restore_signal: &Path,
    recovery_started_signal: &Path,
    recovery_done_signal: &Path,
    recovery_timed_out_signal: &Path,
    recovery_deadline_unix_ms: u64,
) -> String {
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$DeploymentLog = 'Microsoft-Windows-AppXDeploymentServer/Operational'
$TargetMoniker = {package_moniker}
$ActivityId = {activity_id}
$FileId = {file_id}
$RestoreSignal = {restore_signal}
$RecoveryStartedSignal = {recovery_started_signal}
$RecoveryDoneSignal = {recovery_done_signal}
$RecoveryTimedOutSignal = {recovery_timed_out_signal}
$RecoveryDeadlineUnixMs = [int64]{recovery_deadline_unix_ms}
$stopped = @()

# This marker is written before any service/process mutation. The unelevated
# caller uses it to distinguish a harmless UAC/launcher timeout from an
# elevated helper whose outcome must be treated as ambiguous.
Set-Content -LiteralPath $RecoveryStartedSignal -Value 'started' -Force -ErrorAction Stop

function Restore-PausedServices([string]$Names) {{
  foreach ($name in @($Names -split ',' | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }})) {{
    try {{ Start-Service -Name $name -ErrorAction Stop }} catch {{}}
  }}
}}

function Signal-RecoveryFinished {{
  Set-Content -LiteralPath $RecoveryDoneSignal -Value 'done' -Force -ErrorAction SilentlyContinue
}}

function Start-RecoveryDeadlineWatchdog([string]$Names) {{
  $helperPid = [uint32]$PID
  $helperStarted = (Get-Process -Id $helperPid -ErrorAction Stop).StartTime.ToFileTimeUtc()
  $escapedServices = $Names.Replace("'", "''")
  $escapedSignal = $RecoveryDoneSignal.Replace("'", "''")
  $escapedTimedOutSignal = $RecoveryTimedOutSignal.Replace("'", "''")
  $deadlineTemplate = @'
$ErrorActionPreference = 'SilentlyContinue'
$deadline = [DateTimeOffset]::FromUnixTimeMilliseconds(__DEADLINE__).UtcDateTime
$signal = '__SIGNAL__'
while (-not (Test-Path -LiteralPath $signal) -and [DateTime]::UtcNow -lt $deadline) {{ Start-Sleep -Seconds 1 }}
if (Test-Path -LiteralPath $signal) {{ Remove-Item -LiteralPath $signal -Force -ErrorAction SilentlyContinue; exit 0 }}
$timedOutSignal = '__TIMED_OUT_SIGNAL__'
$null = New-Item -ItemType File -Path $timedOutSignal -Force -ErrorAction SilentlyContinue
$helper = Get-Process -Id __HELPER_PID__ -ErrorAction SilentlyContinue
if ($null -ne $helper -and $helper.StartTime.ToFileTimeUtc() -eq [int64]__HELPER_STARTED__) {{ Stop-Process -Id __HELPER_PID__ -Force -ErrorAction SilentlyContinue }}
foreach ($name in @('__SERVICES__' -split ',' | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }})) {{ Start-Service -Name $name -ErrorAction SilentlyContinue }}
'@
  $deadlineScript = $deadlineTemplate.Replace('__DEADLINE__', [string]$RecoveryDeadlineUnixMs).Replace('__SIGNAL__', $escapedSignal).Replace('__TIMED_OUT_SIGNAL__', $escapedTimedOutSignal).Replace('__HELPER_PID__', [string]$helperPid).Replace('__HELPER_STARTED__', [string]$helperStarted).Replace('__SERVICES__', $escapedServices)
  $encodedDeadline = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($deadlineScript))
  Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList ('-NoProfile -NonInteractive -EncodedCommand ' + $encodedDeadline) -WindowStyle Hidden -ErrorAction Stop | Out-Null
}}

function Start-RestoreWatchdog([string]$Names) {{
  $escapedServices = $Names.Replace("'", "''")
  $escapedSignal = $RestoreSignal.Replace("'", "''")
  $restoreTemplate = @'
$ErrorActionPreference = 'SilentlyContinue'
$deadline = (Get-Date).AddSeconds({restore_watchdog_secs})
$signal = '__SIGNAL__'
while (-not (Test-Path -LiteralPath $signal) -and (Get-Date) -lt $deadline) {{ Start-Sleep -Seconds 1 }}
foreach ($name in @('__SERVICES__' -split ',' | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }})) {{ Start-Service -Name $name -ErrorAction SilentlyContinue }}
'@
  $restoreScript = $restoreTemplate.Replace('__SIGNAL__', $escapedSignal).Replace('__SERVICES__', $escapedServices)
  $encodedRestore = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($restoreScript))
  Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList ('-NoProfile -NonInteractive -EncodedCommand ' + $encodedRestore) -WindowStyle Hidden -ErrorAction Stop | Out-Null
}}

try {{
  # Arm restoration before the first service/process mutation. If this elevated
  # helper itself hangs, the independent elevated watchdog still restores every
  # service that was running when recovery began.
  foreach ($name in @('InstallService', 'wuauserv', 'DoSvc', 'AppXSvc', 'ClipSVC')) {{
    $service = Get-Service -Name $name -ErrorAction SilentlyContinue
    if ($null -ne $service -and $service.Status -eq 'Running' -and $stopped -notcontains $name) {{
      $stopped += $name
    }}
  }}
  if ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() -ge $RecoveryDeadlineUnixMs) {{
    throw 'Offline AppX recovery deadline expired before mutation began'
  }}
  Start-RecoveryDeadlineWatchdog ($stopped -join ',')
  Start-RestoreWatchdog ($stopped -join ',')

  $related = @(Get-WinEvent -LogName $DeploymentLog -FilterXPath "*[System[Correlation[@ActivityID='$ActivityId']]]" -ErrorAction Stop)
  # Accept event 400 only when its ETW opcode is Stop. This avoids treating a
  # differently shaped provider event as completion while preserving the
  # documented success record emitted by AppXDeploymentServer.
  $terminal = @($related | Where-Object {{
    ([int]$_.Id -eq 400 -and [int]$_.Opcode -eq 2) -or
    (401, 404 -contains [int]$_.Id)
  }})
  if ($terminal.Count -gt 0) {{
    Signal-RecoveryFinished
    exit 0
  }}
  $packageStart = @($related | Where-Object {{ [int]$_.Id -eq 10002 }} | Select-Object -First 1)
  $wuStart = @($related | Where-Object {{ [int]$_.Id -eq 603 }} | Select-Object -First 1)
  if ($packageStart.Count -eq 0 -or $wuStart.Count -eq 0) {{ throw 'AppX activity no longer has the expected start events' }}
  $packageXml = [xml]$packageStart[0].ToXml()
  $wuXml = [xml]$wuStart[0].ToXml()
  $moniker = [string](@($packageXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'MainPackageMoniker' }} | Select-Object -First 1)[0].'#text')
  $operation = [string](@($wuXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'DeploymentOperation' }} | Select-Object -First 1)[0].'#text')
  $caller = [string](@($wuXml.Event.EventData.Data | Where-Object {{ $_.Name -eq 'CallingProcess' }} | Select-Object -First 1)[0].'#text')
  $owner = [string]$wuXml.Event.System.Security.UserID
  if ($moniker -ne $TargetMoniker -or $operation -ne '4' -or $caller -notmatch '(^|,)wuauserv($|,)' -or $owner -ne 'S-1-5-18') {{
    throw 'Refusing to reset an AppX activity that is not the exact SYSTEM Windows Update stage operation'
  }}

  foreach ($name in @('InstallService', 'wuauserv')) {{
    $service = Get-Service -Name $name -ErrorAction SilentlyContinue
    if ($null -ne $service -and $service.Status -eq 'Running') {{
      Stop-Service -Name $name -Force -ErrorAction Stop
    }}
  }}

  if (-not [string]::IsNullOrWhiteSpace($FileId)) {{
    $delivery = @()
    if ($null -ne (Get-Command Get-DeliveryOptimizationStatus -ErrorAction SilentlyContinue)) {{
      $delivery = @(Get-DeliveryOptimizationStatus -ErrorAction SilentlyContinue |
        Where-Object {{ $_.FileId -eq $FileId }} | Select-Object -First 1)
    }}
    if ($delivery.Count -gt 0 -and [string]$delivery[0].PredefinedCallerApplication -ne 'WU Client Download') {{
      throw 'Refusing to clear a Delivery Optimization entry not owned by Windows Update'
    }}
    if ($delivery.Count -gt 0 -and $null -ne (Get-Command Delete-DeliveryOptimizationCache -ErrorAction SilentlyContinue)) {{
      try {{ Delete-DeliveryOptimizationCache -FileId $FileId -Force -ErrorAction Stop }} catch {{}}
    }}
  }}

  $deliveryService = Get-Service -Name DoSvc -ErrorAction SilentlyContinue
  if ($null -ne $deliveryService -and $deliveryService.Status -eq 'Running') {{
    Stop-Service -Name DoSvc -Force -ErrorAction Stop
  }}

  $appx = Get-CimInstance Win32_Service -Filter "Name='AppXSvc'" -ErrorAction Stop
  if ($null -ne $appx -and [uint32]$appx.ProcessId -ne 0) {{
    $hosted = @(Get-CimInstance Win32_Service -Filter ("ProcessId=" + [uint32]$appx.ProcessId) -ErrorAction Stop |
      Where-Object {{ $_.State -eq 'Running' }})
    $unexpected = @($hosted | Where-Object {{ @('AppXSvc', 'ClipSVC') -notcontains [string]$_.Name }})
    if ($unexpected.Count -gt 0 -or @($hosted | Where-Object {{ [string]$_.Name -eq 'AppXSvc' }}).Count -ne 1) {{
      throw 'Refusing to terminate an AppX service host containing unrelated services'
    }}
    $cohostedServices = @($hosted | Where-Object {{ [string]$_.Name -ne 'AppXSvc' }})
    foreach ($cohosted in $cohostedServices) {{
      if ($stopped -notcontains [string]$cohosted.Name) {{ $stopped += [string]$cohosted.Name }}
    }}
    if ($cohostedServices.Count -gt 0) {{
      Start-RestoreWatchdog ((@($cohostedServices | ForEach-Object {{ [string]$_.Name }})) -join ',')
    }}
    Stop-Process -Id ([uint32]$appx.ProcessId) -Force -ErrorAction Stop
    Start-Sleep -Milliseconds 500
  }}
  Start-Service -Name AppXSvc -ErrorAction SilentlyContinue
  Signal-RecoveryFinished
  exit 0
}} catch {{
  Restore-PausedServices ($stopped -join ',')
  Signal-RecoveryFinished
  exit 1
}}
"#,
        package_moniker = ps_quote(package_moniker),
        activity_id = ps_quote(activity_id),
        file_id = ps_quote(file_id),
        restore_signal = ps_quote(&restore_signal.to_string_lossy()),
        recovery_started_signal = ps_quote(&recovery_started_signal.to_string_lossy()),
        recovery_done_signal = ps_quote(&recovery_done_signal.to_string_lossy()),
        recovery_timed_out_signal = ps_quote(&recovery_timed_out_signal.to_string_lossy()),
        recovery_deadline_unix_ms = recovery_deadline_unix_ms,
        restore_watchdog_secs = OFFLINE_APPX_RESTORE_WATCHDOG_SECS,
    )
}

#[cfg(windows)]
fn launch_elevated_recovery(script: &str) -> Result<ElevatedRecoveryLaunchReport, EngineError> {
    let arguments = format!(
        "-NoProfile -NonInteractive -EncodedCommand {}",
        encode_powershell_command(script)
    );
    let launcher = format!(
        r#"
 $ErrorActionPreference = 'Stop'
 try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}
  try {{
  $process = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -Verb RunAs -ArgumentList {arguments} -WindowStyle Hidden -Wait -PassThru -ErrorAction Stop
  @{{ started = $true; exitCode = [int]$process.ExitCode; error = '' }} | ConvertTo-Json -Compress
}} catch {{
  @{{ started = $false; exitCode = -1; error = [string]$_.Exception.Message }} | ConvertTo-Json -Compress
}}
"#,
        arguments = ps_quote(&arguments),
    );
    let json =
        run_powershell_json_with_limits(&launcher, RunLimits::appx_recovery()).map_err(|e| {
            EngineError::Install(format!(
                "run administrator-approved offline AppX recovery: {}",
                e.message()
            ))
        })?;
    serde_json::from_str(&json)
        .map_err(|e| EngineError::Install(format!("parse elevated AppX recovery result: {e}")))
}

#[cfg(windows)]
fn prepare_offline_appx_install(
    package_moniker: &str,
    recovery_outcome_ambiguous: &mut dyn FnMut(),
) -> Result<OfflineAppxRestoreGuard, EngineError> {
    let detection = detect_offline_appx_conflict_script(package_moniker);
    let Some(conflict) = parse_offline_appx_conflict_probe(run_powershell_json_with_limits(
        &detection,
        RunLimits::probe(),
    )) else {
        // This probe only decides whether an exact SYSTEM transaction needs to
        // be released. It must never become a prerequisite for trying the
        // already-downloaded, already-verified local package.
        return Ok(OfflineAppxRestoreGuard { signal: None });
    };
    log::info!(
        "offline AppX conflict active={} message={}",
        conflict.active,
        conflict.message
    );
    if !conflict.active {
        return Ok(OfflineAppxRestoreGuard { signal: None });
    }

    let activity_id = conflict.activity_id.ok_or_else(|| {
        EngineError::Install("SYSTEM AppX conflict had no activity id".to_string())
    })?;
    let activity_uuid = activity_id.trim_matches(['{', '}']);
    uuid::Uuid::parse_str(activity_uuid).map_err(|_| {
        EngineError::Install("SYSTEM AppX conflict had an invalid activity id".to_string())
    })?;
    let file_id = validated_delivery_optimization_file_id(conflict.file_id);

    let base = std::env::temp_dir().join(format!(
        "codex-offline-appx-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let restore_signal = base.with_extension("restore");
    let recovery_started_signal = base.with_extension("recovery-started");
    let recovery_done_signal = base.with_extension("recovery-done");
    let recovery_timed_out_signal = base.with_extension("recovery-timed-out");
    let recovery_deadline = std::time::SystemTime::now()
        + (crate::process::APPX_RECOVERY_TIMEOUT
            - std::time::Duration::from_secs(OFFLINE_APPX_RECOVERY_CLEANUP_MARGIN_SECS));
    let recovery_deadline_unix_ms = recovery_deadline
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| EngineError::Install(format!("compute AppX recovery deadline: {e}")))?
        .as_millis() as u64;
    let recovery = offline_appx_recovery_script(
        package_moniker,
        &activity_id,
        &file_id,
        &restore_signal,
        &recovery_started_signal,
        &recovery_done_signal,
        &recovery_timed_out_signal,
        recovery_deadline_unix_ms,
    );
    let launched = match launch_elevated_recovery(&recovery) {
        Ok(launched) => launched,
        Err(err) => {
            // The unelevated launcher cannot directly terminate a process that
            // has crossed the UAC boundary. The elevated absolute-deadline
            // watchdog will stop it, while this marker prevents the operation
            // from being classified as a clean, immediately retryable failure.
            if recovery_started_signal.exists() {
                recovery_outcome_ambiguous();
            }
            let _ = std::fs::remove_file(&recovery_started_signal);
            let _ = std::fs::remove_file(&recovery_timed_out_signal);
            return Err(err);
        }
    };
    if recovery_timed_out_signal.exists() {
        recovery_outcome_ambiguous();
    }
    let _ = std::fs::remove_file(&recovery_started_signal);
    let _ = std::fs::remove_file(&recovery_timed_out_signal);
    if !launched.started {
        let _ = std::fs::remove_file(&recovery_done_signal);
        // An exact active SYSTEM transaction is not evidence that MSIX is
        // unsupported. Returning a failed sideload report here would make the
        // app route to portable, violating the mirror-first contract. Keep the
        // verified staged package and expose a retryable permission failure.
        return Err(EngineError::Install(format!(
            "Windows Update owns this Codex deployment; administrator approval is required to release only that transaction. The verified local MSIX remains staged; retry and approve the prompt: {}",
            launched.error
        )));
    }
    if launched.exit_code != 0 {
        let _ = std::fs::remove_file(&restore_signal);
        return Err(EngineError::Install(format!(
            "could not release the exact SYSTEM Windows Update deployment (exit {})",
            launched.exit_code
        )));
    }
    log::info!(
        "SYSTEM Windows Update deployment was released or had already completed; proceeding with verified local MSIX"
    );
    Ok(OfflineAppxRestoreGuard {
        signal: Some(restore_signal),
    })
}

#[cfg(windows)]
fn install_msix_script_with_volume_and_identity(
    path: &Path,
    volume_root: Option<&Path>,
    expected_package_full_name: &str,
) -> String {
    let volume_setup = volume_root
        .map(|root| {
            format!(
                r#"
{data_backup_script}
{machine_state_script}
{machine_rehydration_script}
  function Test-PreserveApplicationDataUnsupported([string]$detail) {{
    if ([string]::IsNullOrWhiteSpace($detail)) {{ return $false }}
    # Some older sideload registrations expose the preserve switch but reject
    # it at runtime because the package was not deployed through deployment mode.
    # Keep this classifier narrow; ordinary permission/policy failures must not
    # be hidden by a destructive retry.
    return $detail -match {preserve_compatibility_pattern}
  }}
  function Invoke-CodexRemoveAppxPackage([hashtable]$removeArgs, [scriptblock]$beforeUnpreservedRemove) {{
    $preserveAttempted = $removeArgs.ContainsKey('PreserveApplicationData')
    $preserveFallbackUsed = $false
    $attempt = 0
    while ($attempt -lt 8) {{
      # A busy/error response can give the app time to produce new data. The
      # original verified snapshot must still authorize EVERY destructive try.
      if (-not $removeArgs.ContainsKey('PreserveApplicationData')) {{
        & $beforeUnpreservedRemove
      }}
      try {{
        Remove-AppxPackage @removeArgs | Out-Null
        return @{{
          preserveFallbackUsed = $preserveFallbackUsed
          warning = if ($preserveFallbackUsed) {{
            'Windows rejected PreserveApplicationData for this sideload package (0x80073CFA/non-deployment); cleanup retried without the flag after checking/backing up package-local data.'
          }} else {{ $null }}
        }}
      }} catch {{
        $detail = [string]$_
        if ($preserveAttempted -and (Test-PreserveApplicationDataUnsupported $detail)) {{
          # Retry the exact package without the incompatible switch. This is
          # deliberately limited to the documented deployment-mode error; all
          # other permission, policy and identity errors remain failures.
          $removeArgs.Remove('PreserveApplicationData') | Out-Null
          $preserveAttempted = $false
          $preserveFallbackUsed = $true
          continue
        }}
        if ($attempt -eq 7) {{ throw }}
        $attempt++
        Start-Sleep -Milliseconds 250
      }}
    }}
    throw 'Remove-AppxPackage did not complete'
  }}
  if ($cmd.Parameters.Keys -notcontains 'Volume') {{
    throw 'Add-AppxPackage on this Windows build does not expose -Volume; USB AppX installation is unavailable'
  }}
  if (-not (Get-Command Get-AppxVolume -ErrorAction SilentlyContinue)) {{
    throw 'Get-AppxVolume is unavailable; USB AppX installation is unavailable'
  }}
  if (-not (Get-Command Add-AppxVolume -ErrorAction SilentlyContinue)) {{
    throw 'Add-AppxVolume is unavailable; USB AppX installation is unavailable'
  }}
  $mountPoint = {root}
  $usbVolumeMode = $true
  $operation = 'Validate USB AppX volume'
  # AppX volume registration only works with a local Windows volume. A
  # mapped/WebDAV drive can look like a USB path to the caller, but the AppX
  # service cannot resolve it and commonly surfaces the misleading
  # "Invalid URI"/hostname error. Probe the volume before touching
  # WindowsApps so the user gets an actionable diagnosis.
  if (Get-Command Get-Volume -ErrorAction SilentlyContinue) {{
    $volumeProbe = @(Get-Volume -DriveLetter $mountPoint.Substring(0, 1) -ErrorAction SilentlyContinue |
      Select-Object -First 1)[0]
    if ($null -eq $volumeProbe) {{
      throw "USB AppX volume $mountPoint is not a local Windows volume; network or remapped drives are unsupported"
    }}
    $fileSystem = [string]$volumeProbe.FileSystem
    if (-not [string]::IsNullOrWhiteSpace($fileSystem) -and $fileSystem -notmatch '(?i)^NTFS$') {{
      throw "USB AppX volume $mountPoint uses filesystem '$fileSystem'; NTFS is required for MSIX AppX volumes"
    }}
  }}
  # Keep this as a lexical path combine. Join-Path resolves the provider and
  # rejects a not-yet-mounted drive letter before Add-AppxVolume gets a chance
  # to register it; the latter is a normal state for a fresh removable volume
  # and is also how the PowerShell fixture models the USB target.
  $packageStorePath = [System.IO.Path]::Combine($mountPoint, 'WindowsApps')
  if (-not (Test-Path -LiteralPath $packageStorePath)) {{
    New-Item -ItemType Directory -Path $packageStorePath -Force | Out-Null
  }}
  # Resolve the volume through the path-aware/online query first. Enumerating
  # all AppX volumes can return an old offline registration for the same drive
  # letter after a removable disk was reformatted or reassigned. Passing that
  # stale object to Add-AppxPackage is known to surface the misleading
  # "Invalid URI: The hostname could not be parsed" error.
  $getAppxVolumeCmd = Get-Command Get-AppxVolume -ErrorAction Stop
  $volume = $null
  $volumeCandidates = @()
  try {{
    if ($getAppxVolumeCmd.Parameters.Keys -contains 'Online') {{
      $volumeCandidates = @(Get-AppxVolume -Path $mountPoint -Online -ErrorAction SilentlyContinue)
    }} else {{
      $volumeCandidates = @(Get-AppxVolume -Path $mountPoint -ErrorAction SilentlyContinue)
    }}
  }} catch {{
    $volumeCandidates = @()
  }}
  $volume = @($volumeCandidates | Where-Object {{ -not [bool]$_.IsOffline }})[0]
  $offlineVolume = $null
  if ($null -eq $volume) {{
    $allAppxVolumes = @(Get-AppxVolume -ErrorAction SilentlyContinue)
    $volumeCandidates = @($allAppxVolumes | Where-Object {{
      ([string]$_.MountPoint).TrimEnd('\\') -ieq $mountPoint.TrimEnd('\\') -or
      ([string]$_.PackageStorePath).TrimEnd('\\') -ieq $packageStorePath.TrimEnd('\\')
    }})
    $volume = @($volumeCandidates | Where-Object {{ -not [bool]$_.IsOffline }})[0]
    $offlineVolume = @($volumeCandidates | Where-Object {{ [bool]$_.IsOffline }})[0]
    if ($null -eq $volume -and $null -ne $offlineVolume -and
        (Get-Command Mount-AppxVolume -ErrorAction SilentlyContinue)) {{
      $operation = 'Mount-AppxVolume'
      try {{
        Mount-AppxVolume -Volume $offlineVolume -ErrorAction Stop | Out-Null
      }} catch {{
        $mountError = [string]$_.Exception.Message
      }}
      try {{
        if ($getAppxVolumeCmd.Parameters.Keys -contains 'Online') {{
          $volume = @(Get-AppxVolume -Path $mountPoint -Online -ErrorAction SilentlyContinue)[0]
        }} else {{
          $volume = @(Get-AppxVolume -Path $mountPoint -ErrorAction SilentlyContinue |
            Where-Object {{ -not [bool]$_.IsOffline }})[0]
        }}
      }} catch {{
        $volume = $null
      }}
    }}
  }}
  if ($null -eq $volume) {{
    $operation = 'Add-AppxVolume'
    $addedVolume = $null
    $addVolumeError = $null
    try {{
      # Keep the native AppxVolume object returned by the cmdlet. Do not
      # convert it to a string or a Select-Object projection before passing it
      # to the typed -Volume parameter.
      $addedVolumes = @(Add-AppxVolume -Path $packageStorePath -ErrorAction Stop)
      if ($addedVolumes.Count -gt 0) {{ $addedVolume = $addedVolumes[0] }}
    }} catch {{
      $addVolumeError = [string]$_.Exception.Message
    }}
    # Re-read by path after registration. This gives the AppX service time to
    # publish the online/native object on slower USB media and replaces a stale
    # object that happened to share the same drive letter.
    try {{
      if ($getAppxVolumeCmd.Parameters.Keys -contains 'Online') {{
        $volume = @(Get-AppxVolume -Path $mountPoint -Online -ErrorAction SilentlyContinue)[0]
      }} else {{
        $volume = @(Get-AppxVolume -Path $mountPoint -ErrorAction SilentlyContinue |
          Where-Object {{ -not [bool]$_.IsOffline }})[0]
      }}
    }} catch {{
      $volume = $null
    }}
    if ($null -eq $volume) {{
      $allAppxVolumes = @(Get-AppxVolume -ErrorAction SilentlyContinue)
      $volume = @($allAppxVolumes | Where-Object {{
        -not [bool]$_.IsOffline -and
        (([string]$_.MountPoint).TrimEnd('\\') -ieq $mountPoint.TrimEnd('\\') -or
         ([string]$_.PackageStorePath).TrimEnd('\\') -ieq $packageStorePath.TrimEnd('\\'))
      }})[0]
    }}
    if ($null -eq $volume -and $null -ne $addedVolume -and -not [bool]$addedVolume.IsOffline) {{
      $volume = $addedVolume
    }}
    if ($null -eq $volume -and $addVolumeError) {{
      throw ('Could not register removable AppX volume ' + $mountPoint + ': ' + $addVolumeError)
    }}
  }}
  if ($null -eq $volume) {{
    throw "Could not register removable AppX volume $mountPoint"
  }}
  if ([bool]$volume.IsOffline) {{
    throw "Removable AppX volume $mountPoint is offline; reconnect the drive and retry"
  }}
  $resolvedStore = ([string]$volume.PackageStorePath).TrimEnd('\')
  if ([string]::IsNullOrWhiteSpace($resolvedStore) -or
      $resolvedStore -ine $packageStorePath.TrimEnd('\')) {{
    throw "Resolved AppX volume does not match the USB package store: expected=$packageStorePath actual=$resolvedStore"
  }}
  function Invoke-NativeAppxAdd {{
    param(
      [Parameter(Mandatory=$true)][string]$packagePath,
      [Parameter(Mandatory=$true)]$targetVolume
    )
    # The PowerShell Appx wrapper has a Windows-version-specific URI bug for
    # removable volumes. Call the documented WinRT PackageManager overload
    # directly as a second path; it takes a file:// Uri and a native
    # PackageVolume, so no string/projection conversion of the volume occurs.
    Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
    $pm = [Windows.Management.Deployment.PackageManager,Windows,ContentType=WindowsRuntime]::new()
    $volumeName = [string]$targetVolume.Name
    if ([string]::IsNullOrWhiteSpace($volumeName)) {{ throw 'USB AppX volume has no native volume name' }}
    $packageVolume = $pm.FindPackageVolume($volumeName)
    if ($null -eq $packageVolume) {{ throw "PackageManager could not resolve volume $volumeName" }}
    if ([bool]$packageVolume.IsOffline) {{ throw "PackageManager volume $volumeName is offline" }}
     $fullPath = (Get-Item -LiteralPath $packagePath -Force -ErrorAction Stop).FullName
     # DOS-path URI construction escapes spaces, # and literal % correctly.
     $packageUri = [System.Uri]::new($fullPath, [System.UriKind]::Absolute)
     if (-not $packageUri.IsFile) {{ throw 'MSIX source must be an absolute local file path' }}
    $options = [Windows.Management.Deployment.DeploymentOptions]::ForceApplicationShutdown
    # This FOUR-parameter WinRT overload supports only None and
    # ForceApplicationShutdown. ForceUpdateFromAnyVersion belongs to later
    # overloads; passing it here breaks recovery on supported older Windows.
    if ($null -ne $existing -and [version]$existing.Version -gt [version]$expectedPackageFullName.Split('_')[1]) {{
      throw 'Native USB recovery does not support downgrading an existing registration; it was retained'
    }}
    $async = $pm.AddPackageAsync($packageUri, $null, $options, $packageVolume)
    $asTaskMethod = [System.WindowsRuntimeSystemExtensions].GetMethods() |
      Where-Object {{
        $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and
          $_.GetGenericArguments().Count -eq 2 -and $_.GetParameters().Count -eq 1
      }} |
      Select-Object -First 1
    if ($null -eq $asTaskMethod) {{ throw 'WindowsRuntimeSystemExtensions.AsTask is unavailable' }}
    $closedAsTask = $asTaskMethod.MakeGenericMethod(
      [Windows.Management.Deployment.DeploymentResult],
      [Windows.Management.Deployment.DeploymentProgress])
    $task = $closedAsTask.Invoke($null, @($async))
    $result = $task.GetAwaiter().GetResult()
    $extendedError = $null
    try {{
      if ($result.ExtendedErrorCode) {{
        $hr = [int]$result.ExtendedErrorCode.HResult
        if ($hr -ne 0) {{ $extendedError = ('0x{{0:X8}}' -f $hr) }}
      }}
    }} catch {{}}
    if ($extendedError) {{
      throw "PackageManager AddPackageAsync failed hresult=$extendedError activityId=$([string]$result.ActivityId) errorText=$([string]$result.ErrorText)"
    }}
    @{{
      # Diagnostic hint only. Windows 10 can report false while its successful
      # OnDemandRegister PackageUser projection is still converging.
      isRegisteredHint = [bool]$result.IsRegistered
      activityId = [string]$result.ActivityId
      errorText = [string]$result.ErrorText
      extendedError = $extendedError
      volumeName = $volumeName
      packageUri = [string]$packageUri.AbsoluteUri
    }}
  }}
  function Wait-CodexCurrentUserPackageProjection {{
    param([Parameter(Mandatory=$true)][string]$fullName, $nativeResult = $null)
    $lastObservation = 'current-user package is not visible yet'
    $observed = @()
    for ($attempt = 1; $attempt -le 240; $attempt++) {{
      $packages = $null
      try {{
        $packages = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
      }} catch {{
        $lastObservation = 'current-user package query failed: ' + [string]$_.Exception.Message
      }}
      if ($null -ne $packages) {{
        $observed = @($packages | ForEach-Object {{ [string]$_.PackageFullName }})
        $exact = @($packages | Where-Object {{ [string]$_.PackageFullName -ieq $fullName }})
        if ($exact.Count -gt 1) {{ throw 'Current-user AppX projection returned duplicate exact Codex packages' }}
        if ($exact.Count -eq 1) {{
          $candidate = $exact[0]
          if ($candidate.PSObject.Properties.Name -contains 'Status' -and
              -not [string]::IsNullOrWhiteSpace([string]$candidate.Status) -and
              [string]$candidate.Status -ine 'Ok') {{
            $lastObservation = 'exact package status is ' + [string]$candidate.Status
          }} elseif ($candidate.PSObject.Properties.Name -contains 'IsPartiallyStaged' -and
                    [bool]$candidate.IsPartiallyStaged) {{
            $lastObservation = 'exact package is still partially staged'
          }} else {{
            return $candidate
          }}
        }} elseif ($observed.Count -gt 0) {{
          throw "An unexpected current-user Codex package appeared during registration convergence: expected=$fullName observed=$($observed -join ',')"
        }} else {{
          $lastObservation = 'current-user package is not visible yet'
        }}
      }}
      if ($attempt -lt 240) {{ Start-Sleep -Milliseconds 250 }}
    }}
    $hint = if ($nativeResult -and $nativeResult.ContainsKey('isRegisteredHint')) {{
      [string]$nativeResult.isRegisteredHint
    }} else {{ '<unavailable>' }}
    $activity = if ($nativeResult -and $nativeResult.ContainsKey('activityId')) {{
      [string]$nativeResult.activityId
    }} else {{ '<PowerShell-or-controlled-recovery>' }}
    throw "USB registration API returned, but the exact healthy current-user package did not become observable within 60 seconds; expected=$fullName observed=$($observed -join ',') IsRegisteredHint=$hint ActivityId=$activity lastObservation=$lastObservation"
  }}
  $currentSid = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  function Get-CodexMachinePackageLocationEvidence($package) {{
    $raw = ([string]$package.InstallLocation).Trim()
    if ([string]::IsNullOrWhiteSpace($raw)) {{
      return @{{state='empty';raw=$raw;effective=$null;detail='InstallLocation is empty'}}
    }}
    $info = Get-CodexPackagePathInfo $raw ([string]$package.PackageFullName) $packageStorePath
    $expected = [System.IO.Path]::Combine($packageStorePath, [string]$package.PackageFullName)
    if (-not [string]::IsNullOrWhiteSpace([string]$info.effectivePath) -and
        ([string]$info.effectivePath).TrimEnd('\') -ieq $expected.TrimEnd('\')) {{
      return @{{state='target-usb';raw=$raw;effective=[string]$info.effectivePath;detail=[string]$info.state}}
    }}
    if ([string]$info.state -in @('path-absent','dangling-system-junction')) {{
      return @{{state='orphan-path';raw=$raw;effective=$null;detail=($info | ConvertTo-Json -Compress)}}
    }}
    return @{{state='other-or-unknown';raw=$raw;effective=[string]$info.effectivePath;detail=($info | ConvertTo-Json -Compress)}}
  }}
  function Get-CodexMachineOwnershipSnapshot {{
    $current = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
    $all = @(Get-AppxPackage -Name {name} -AllUsers -ErrorAction Stop)
    $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop |
      Where-Object {{ [string]$_.DisplayName -ieq {name} -or [string]$_.PackageName -ieq $expectedPackageFullName }})
    $snapshot = [ordered]@{{
      classification='blocked'; reason=$null; current=$null; packages=@($all); provisioned=@($provisioned)
      currentFullName=$null; machineFullNames=@($all | ForEach-Object {{[string]$_.PackageFullName}})
      ownerStates=@(); locationState=$null; location=$null
    }}
    if ($current.Count -gt 1) {{ $snapshot.reason='Multiple current-user Codex registrations were found'; return $snapshot }}
    if ($current.Count -eq 1) {{
      $snapshot.current = $current[0]; $snapshot.currentFullName = [string]$current[0].PackageFullName
      if ([string]$current[0].PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0') {{ $snapshot.reason='A different publisher owns the current-user registration'; return $snapshot }}
    }}
    $unexpected = @($all | Where-Object {{ [string]$_.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0' }})
    if ($unexpected.Count) {{ $snapshot.reason='The machine query returned an unexpected package family'; return $snapshot }}
    if ($provisioned.Count) {{ $snapshot.reason='A provisioned Codex package exists'; return $snapshot }}
    $currentSid = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    foreach ($entry in $all) {{
      foreach ($owner in @($entry.PackageUserInformation)) {{
        $snapshot.ownerStates += ('{{0}}:{{1}}' -f (Get-CodexAppxOwnerSid $owner),[string]$owner.InstallState)
      }}
    }}
    if ($null -ne $snapshot.current) {{
      if ($all.Count -ne 1 -or [string]$all[0].PackageFullName -ine [string]$snapshot.currentFullName) {{
        $snapshot.reason='Current-user and machine package snapshots disagree'; return $snapshot
      }}
      $owners = @($all[0].PackageUserInformation)
      if ($owners.Count -ne 1 -or (Get-CodexAppxOwnerSid $owners[0]) -ine $currentSid -or [string]$owners[0].InstallState -ne 'Installed') {{
        $snapshot.reason='Codex is installed/staged for another user or has ambiguous ownership'; return $snapshot
      }}
      $snapshot.classification='owned-current'; $snapshot.reason='Only the current user owns this package'; return $snapshot
    }}
    if ($all.Count -eq 0) {{ $snapshot.classification='none'; $snapshot.reason='No machine package exists'; return $snapshot }}
    if ($all.Count -ne 1) {{ $snapshot.reason='Multiple machine Codex package records exist'; return $snapshot }}
    $entry = $all[0]
    if ([string]$entry.PackageFullName -ine $expectedPackageFullName) {{
      $snapshot.reason='An unowned machine package has a different version than the verified payload'; return $snapshot
    }}
    $owners = @($entry.PackageUserInformation)
    # PackageUserInformation may attribute the machine staging record to
    # SYSTEM or another SID. `Staged` is not a user registration; any Installed
    # or unknown state still blocks the repair so another user's usable app is
    # never removed.
    $unsafeOwners = @($owners | Where-Object {{ [string]$_.InstallState -ne 'Staged' }})
    if ($unsafeOwners.Count) {{ $snapshot.reason='A machine package has an installed or unknown owner'; return $snapshot }}
    $location = Get-CodexMachinePackageLocationEvidence $entry
    $snapshot.locationState=[string]$location.state; $snapshot.location=[string]$location.raw
    if ([string]$location.state -eq 'target-usb') {{
      $snapshot.classification='target-staged'; $snapshot.reason='The exact unregistered package is already staged on the bound USB volume'; return $snapshot
    }}
    if ([string]$location.state -in @('empty','orphan-path')) {{
      $snapshot.classification='orphan-exact'; $snapshot.reason='The exact package has no installed owner and no usable package path'; return $snapshot
    }}
    if ([string]$location.state -eq 'other-or-unknown' -and
        -not [string]::IsNullOrWhiteSpace([string]$location.effective)) {{
      $recordVolumes = @(Get-CodexMachineRecordVolumes $expectedPackageFullName)
      if ($recordVolumes.Count -eq 1) {{
        $snapshot.classification='source-staged';
        $snapshot.reason='The exact package is exclusively staged on one non-target AppX volume';
        return $snapshot
      }}
    }}
    $snapshot.reason='An unowned staged package exists outside the target USB volume or its location cannot be proven'; return $snapshot
  }}
  # A failed query is not absence. Classify every machine-level observation
  # before any cleanup or deployment; only one exact, unowned and unlocated
  # package may enter the journaled repair path below.
  $operation = 'Snapshot current-user and machine Codex ownership'
  $script:machineOwnershipSnapshot = Get-CodexMachineOwnershipSnapshot
  $existing = $script:machineOwnershipSnapshot.current
  $allUserPackages = @($script:machineOwnershipSnapshot.packages)
  $provisionedPackages = @($script:machineOwnershipSnapshot.provisioned)
  if ($null -ne $existing) {{ $initialPackageFullName = [string]$existing.PackageFullName }}
  $script:machineOrphanRepairRequired = [string]$script:machineOwnershipSnapshot.classification -in @('orphan-exact','source-staged')
  $sharedPackagePresent = [string]$script:machineOwnershipSnapshot.classification -eq 'blocked'
  if ($sharedPackagePresent) {{
    throw ('Codex machine package ownership blocks USB deployment: ' + [string]$script:machineOwnershipSnapshot.reason)
  }}
  $initialPackageSnapshotComplete = $true
  function Get-CodexPackageVolumeRoot($package) {{
    if ($null -eq $package) {{ return $null }}
    $script:packagePathEvidence = Get-CodexPackagePathInfo ([string]$package.InstallLocation) ([string]$package.PackageFullName) $packageStorePath
    $location = ([string]$script:packagePathEvidence.effectivePath).Trim()
    if ([string]::IsNullOrWhiteSpace($location)) {{ return $null }}
    # InstallLocation is normally a drive path. Normalize an extended Win32
    # prefix as a defensive measure because Get-AppxPackage has returned that
    # spelling on a few down-level builds.
    if ($location.Length -ge 4 -and
        $location.Substring(0, 4) -ieq '\\?\') {{
      $location = $location.Substring(4)
    }}
    if ($location -match '^[A-Za-z]:\\') {{ return $location.Substring(0, 3) }}
    return $null
  }}
  function Assert-CodexMoveOwnership([string]$fullName) {{
    $machine = @(Get-AppxPackage -Name {name} -AllUsers -ErrorAction Stop |
      Where-Object {{ [string]$_.PackageFullName -ieq $fullName }})
    if ($machine.Count -ne 1) {{ throw 'Cannot prove exclusive package ownership immediately before Move; no move was started' }}
    $owners = @($machine[0].PackageUserInformation)
    if ($owners.Count -ne 1 -or (Get-CodexAppxOwnerSid $owners[0]) -ine $currentSid -or
        [string]$owners[0].InstallState -ne 'Installed') {{
      throw 'Package is now staged/shared with another user; no move was started'
    }}
    $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop |
      Where-Object {{ [string]$_.DisplayName -ieq {name} -or [string]$_.PackageName -ieq $fullName }})
    if ($provisioned.Count -gt 0) {{ throw 'Package is now provisioned; no move was started' }}
  }}
  function Get-CodexRecoveryCurrentPackage {{
    $packages = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
    if ($packages.Count -gt 1) {{ throw 'Recovery current-user package query is ambiguous' }}
    if ($packages.Count -eq 0) {{ return $null }}
    $package = $packages[0]
    if ([string]$package.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0' -or
        [string]$package.PackageFullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$') {{
      throw 'Recovery current-user package identity is not the verified Codex family/architecture'
    }}
    return $package
  }}
  function Assert-CodexRecoveryInstallOwnership($package) {{
    $snapshot = Get-CodexMachineOwnershipSnapshot
    $script:machineOwnershipSnapshot = $snapshot
    $classification = [string]$snapshot.classification
    if ($null -ne $package) {{
      if ($classification -eq 'owned-current' -and
          [string]$snapshot.currentFullName -ieq [string]$package.PackageFullName) {{ return }}
      throw ('Current-user package is no longer exclusively owned by this recovery: ' + [string]$snapshot.reason)
    }}
    if ($classification -in @('none','target-staged')) {{ return }}
    if ($classification -in @('orphan-exact','source-staged') -and $script:machineOrphanRepairRequired) {{ return }}
    throw ('Machine package ownership changed before recovery: ' + [string]$snapshot.reason)
  }}
  function Invoke-CodexNativeOrphanRemoval([string]$fullName) {{
    # Supported WinRT fallback for machines whose Appx PowerShell projection
    # cannot retire a damaged staging record. This deliberately uses the same
    # public deployment API as Windows package management and never edits the
    # private PackageRepository registry/database.
    Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
    $pm = [Windows.Management.Deployment.PackageManager,Windows,ContentType=WindowsRuntime]::new()
    $options = [Windows.Management.Deployment.RemovalOptions]::RemoveForAllUsers
    $async = $pm.RemovePackageAsync($fullName, $options)
    $asTaskMethod = [System.WindowsRuntimeSystemExtensions].GetMethods() |
      Where-Object {{
        $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and
          $_.GetGenericArguments().Count -eq 2 -and $_.GetParameters().Count -eq 1
      }} |
      Select-Object -First 1
    if ($null -eq $asTaskMethod) {{ throw 'WindowsRuntimeSystemExtensions.AsTask is unavailable for package removal' }}
    $closedAsTask = $asTaskMethod.MakeGenericMethod(
      [Windows.Management.Deployment.DeploymentResult],
      [Windows.Management.Deployment.DeploymentProgress])
    $task = $closedAsTask.Invoke($null, @($async))
    $result = $task.GetAwaiter().GetResult()
    $extendedError = $null
    try {{
      if ($result.ExtendedErrorCode) {{
        $hr = [int]$result.ExtendedErrorCode.HResult
        if ($hr -ne 0) {{ $extendedError = ('0x{{0:X8}}' -f $hr) }}
      }}
    }} catch {{}}
    if ($extendedError) {{
      throw "PackageManager RemovePackageAsync failed hresult=$extendedError activityId=$([string]$result.ActivityId) errorText=$([string]$result.ErrorText)"
    }}
    return @{{activityId=[string]$result.ActivityId;errorText=[string]$result.ErrorText}}
  }}
  function Repair-CodexOrphanMachineState([string]$backupPath, [string]$packagePath, $journal, $context) {{
    $snapshot = Get-CodexMachineOwnershipSnapshot
    $script:machineOwnershipSnapshot = $snapshot
    if ([string]$snapshot.classification -in @('none','target-staged')) {{
      $script:machineOrphanRepairApi = 'journal-resume-observed-machine-repair-complete'
      $script:machineOrphanRepairCompleted = $true
      return
    }}
    if ([string]$snapshot.classification -notin @('orphan-exact','source-staged') -or
        [string](@($snapshot.machineFullNames)[0]) -ine $expectedPackageFullName) {{
      throw ('Machine repair precondition changed; no package was moved or removed: ' + [string]$snapshot.reason)
    }}
    $backupPackage = @{{PackageFullName=$expectedPackageFullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'}}
    function Assert-CodexMachineRepairDataGate {{
      if (-not [string]::IsNullOrWhiteSpace($backupPath)) {{
        Assert-CodexBackupStillMatches $backupPackage $backupPath $mountPoint ([string]$volume.Name)
      }} else {{
        Assert-CodexPackageDataAbsent
      }}
    }}
    function Set-CodexMachineRepairStep([string]$step, $detail = $null) {{
      if ($null -eq $journal -or $null -eq $journal.machineRepair) {{ return }}
      Set-CodexRecoveryProperty $journal.machineRepair 'step' $step
      if ($null -ne $detail) {{
        Set-CodexRecoveryProperty $journal.machineRepair 'detail' $detail
      }}
      Write-CodexRecoveryJournal $journal $context
    }}
    function Wait-CodexMachineRepairState {{
      $state = $null
      for ($attempt=0; $attempt -lt 20; $attempt++) {{
        $state = Get-CodexMachineOwnershipSnapshot
        if ([string]$state.classification -notin @('orphan-exact','source-staged')) {{ break }}
        Start-Sleep -Milliseconds 250
      }}
      $script:machineOwnershipSnapshot = $state
      return $state
    }}
    function Invoke-CodexSupportedRepairRemoval {{
      Assert-CodexMachineRepairDataGate
      $beforeRemove = Get-CodexMachineOwnershipSnapshot
      if ([string]$beforeRemove.classification -notin @('orphan-exact','source-staged') -or
          [string](@($beforeRemove.machineFullNames)[0]) -ine $expectedPackageFullName) {{
        throw ('Machine ownership changed immediately before supported removal: ' + [string]$beforeRemove.reason)
      }}
      $removeCmd = Get-Command Remove-AppxPackage -ErrorAction Stop
      if ($removeCmd.Parameters.Keys -notcontains 'AllUsers') {{
        throw 'This Windows AppX module cannot remove an unowned machine package through the supported AllUsers API'
      }}
      $localCmdletError = $null
      $localNativeError = $null
      try {{ Remove-AppxPackage -Package $expectedPackageFullName -AllUsers -ErrorAction Stop | Out-Null }}
      catch {{ $localCmdletError = [string]$_ }}
      $afterRemove = Wait-CodexMachineRepairState
      if ([string]$afterRemove.classification -eq 'none') {{
        return @{{completed=$true;api='Remove-AppxPackage -AllUsers';cmdletError=$localCmdletError;nativeError=$null}}
      }}
      if ([string]$afterRemove.classification -notin @('orphan-exact','source-staged') -or
          [string](@($afterRemove.machineFullNames)[0]) -ine $expectedPackageFullName) {{
        throw ('Machine ownership changed after supported removal: ' + [string]$afterRemove.reason)
      }}
      Assert-CodexMachineRepairDataGate
      try {{ $null = Invoke-CodexNativeOrphanRemoval $expectedPackageFullName }}
      catch {{ $localNativeError = [string]$_ }}
      $afterRemove = Wait-CodexMachineRepairState
      return @{{
        completed=([string]$afterRemove.classification -eq 'none')
        api='PackageManager.RemovePackageAsync RemoveForAllUsers'
        cmdletError=$localCmdletError
        nativeError=$localNativeError
        finalState=$afterRemove
      }}
    }}

    Assert-CodexMachineRepairDataGate
    $script:machineOrphanRepairAttempted = $true

    # A pathless record is first offered to both supported removal APIs. This
    # keeps the common case fast and avoids writing temporary bytes to C:.
    $removal = $null
    if ([string]$snapshot.classification -eq 'orphan-exact') {{
      Set-CodexMachineRepairStep 'initial-supported-removal'
      $removal = Invoke-CodexSupportedRepairRemoval
      if ([bool]$removal.completed) {{
        $script:machineOrphanRepairApi = [string]$removal.api
        $script:machineOrphanRepairCompleted = $true
        return
      }}
    }}

    # If public removal cannot open the missing package files, restore the
    # exact Authenticode-authorized payload on the record volume. This repairs
    # the public deployment model; no private registry/database/ACL is edited.
    $snapshot = Get-CodexMachineOwnershipSnapshot
    if ([string]$snapshot.classification -eq 'orphan-exact') {{
      Set-CodexMachineRepairStep 'rehydrating-exact-payload'
      $rehydrated = Invoke-CodexRehydrateMachinePackage $packagePath ([string]$journal.payloadSha256) $expectedPackageFullName $context
      Set-CodexMachineRepairStep 'rehydrated-exact-payload' $rehydrated
      $snapshot = Get-CodexMachineOwnershipSnapshot
    }}
    if ([string]$snapshot.classification -eq 'target-staged') {{
      $script:machineOrphanRepairApi = 'StagePackageAsync exact-payload rehydration on bound USB'
      $script:machineOrphanRepairCompleted = $true
      return
    }}
    if ([string]$snapshot.classification -ne 'source-staged') {{
      throw ('Exact payload rehydration produced an unsafe machine state: ' + [string]$snapshot.reason)
    }}

    $moveError = $null
    try {{
      Set-CodexMachineRepairStep 'moving-rehydrated-package-to-usb'
      $moved = Invoke-CodexMoveRehydratedPackageToUsb $expectedPackageFullName $context
      Set-CodexMachineRepairStep 'moved-rehydrated-package-to-usb' $moved
    }} catch {{ $moveError = [string]$_ }}
    $afterMove = Get-CodexMachineOwnershipSnapshot
    $script:machineOwnershipSnapshot = $afterMove
    if ([string]$afterMove.classification -eq 'target-staged') {{
      $script:machineOrphanRepairApi = 'StagePackageAsync rehydrate + MovePackageToVolumeAsync USB'
      $script:machineOrphanRepairCompleted = $true
      return
    }}
    if ([string]$afterMove.classification -notin @('source-staged','orphan-exact')) {{
      throw ('Package move ended in an unsafe state; no cleanup retry was made: ' + [string]$afterMove.reason + '; moveError=' + $moveError)
    }}

    # Down-level Windows can reject MovePackageToVolumeAsync for a staged-only
    # package. With its exact files now restored, the documented removal APIs
    # can retire it and the normal journaled USB Stage follows.
    Set-CodexMachineRepairStep 'removing-rehydrated-package'
    $removal = Invoke-CodexSupportedRepairRemoval
    if (-not [bool]$removal.completed) {{
      $script:machineOrphanRepairError =
        'initialRemove=' + ($removal | ConvertTo-Json -Depth 5 -Compress) +
        '; moveError=' + $moveError
      throw ('Supported AppX repair failed after exact-payload rehydration; private PackageRepository state was not edited: ' +
        [string]$script:machineOrphanRepairError)
    }}
    Set-CodexMachineRepairStep 'removed-rehydrated-package' $removal
    $script:machineOrphanRepairApi = 'StagePackageAsync exact-payload rehydrate + ' + [string]$removal.api
    $script:machineOrphanRepairCompleted = $true
  }}
  function Assert-CodexRecoveryPackageStopped($package) {{
    if ($null -eq $package) {{ return }}
    $roots = @(Get-CodexPackageProcessRoots ([string]$package.InstallLocation) ([string]$package.PackageFullName) $packageStorePath)
    # The manager already asks this exact installed root to shut down. Never
    # kill an unrelated Codex process or assume inaccessible paths are absent.
    $running = @(Get-CimInstance -ClassName Win32_Process -Filter "Name='Codex.exe' OR Name='ChatGPT.exe'" -ErrorAction Stop)
    foreach ($process in $running) {{
      $exe = [string]$process.ExecutablePath
      if ([string]::IsNullOrWhiteSpace($exe)) {{ throw 'Cannot verify a Codex process path; close Codex before data recovery' }}
      if (Test-CodexProcessPathUnderRoots $exe $roots) {{
        throw 'The exact recovery package is still running; close it and retry. Existing package data was retained'
      }}
    }}
  }}
  function Remove-NewWrongVolumeRegistration($package) {{
    if (-not $usbVolumeMode -or -not $initialPackageSnapshotComplete -or
        -not $deploymentAttempted -or $null -eq $package -or
        -not [string]::IsNullOrWhiteSpace([string]$initialPackageFullName)) {{ return }}
    $script:wrongVolumeCleanupAttempted = $true
    if ([string]$package.PackageFullName -ine $expectedPackageFullName) {{
      $script:wrongVolumeCleanupError = 'Observed package does not match this verified payload; another registration was retained'
      return
    }}
    try {{
      # The Move/query step may have raced with a completed deployment. Never
      # authorize removal from a stale InstallLocation captured before it.
      $current = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
      if ($current.Count -eq 0) {{ return }}
      if ($current.Count -ne 1 -or [string]$current[0].PackageFullName -ine $expectedPackageFullName) {{
        throw 'Wrong-volume cleanup identity changed; removal was not started'
      }}
      $currentRoot = Get-CodexPackageVolumeRoot $current[0]
      if ($null -eq $currentRoot -or $currentRoot.TrimEnd('\') -ieq $mountPoint.TrimEnd('\')) {{
        throw 'Wrong-volume cleanup cannot prove a remaining off-USB registration; removal was not started'
      }}
      $package = $current[0]
      # A newly-created registration is not proof of empty data: a user may
      # launch it while deployment is finishing. Preserve actual content too.
      $backupPath = Backup-CodexPackageData $package $mountPoint ([string]$volume.Name)
      if ($backupPath) {{ $script:wrongVolumeCleanupWarning = "Package-local data backed up and SHA256 verified at $backupPath" }}
      $removeCmd = Get-Command Remove-AppxPackage -ErrorAction Stop
      $removeArgs = @{{
        Package = [string]$package.PackageFullName
        ErrorAction = 'Stop'
      }}
      # Preserve package data. The registration was created by this attempt;
      # keeping data is safer if a later retry needs the same C: profile.
      if ([bool]$package.IsDevelopmentMode -and
          $removeCmd.Parameters.Keys -contains 'PreserveApplicationData') {{
        $removeArgs['PreserveApplicationData'] = $true
      }} else {{
        # A few down-level AppX modules do not expose the switch at all. The
        # registration was created by this invocation, so remove it without
        # the optional flag rather than leaving a false C: registration behind.
        # The warning is surfaced in the final diagnostic because package-local
        # data cannot be guaranteed on this compatibility path.
        $script:wrongVolumeCleanupWarning = ([string]$script:wrongVolumeCleanupWarning + '; PreserveApplicationData is not supported for this ordinary signed registration; cleanup uses the exact package after checking/backing up its local data.')
      }}
      if ($removeCmd.Parameters.Keys -contains 'ForceApplicationShutdown') {{
        $removeArgs['ForceApplicationShutdown'] = $true
      }}
      if ($backupPath) {{ Assert-CodexBackupStillMatches $package $backupPath $mountPoint ([string]$volume.Name) }}
      else {{ Assert-CodexPackageDataAbsent }}
      $removeResult = Invoke-CodexRemoveAppxPackage $removeArgs {{
        if ($backupPath) {{ Assert-CodexBackupStillMatches $package $backupPath $mountPoint ([string]$volume.Name) }}
        else {{ Assert-CodexPackageDataAbsent }}
      }}
      if ([bool]$removeResult.preserveFallbackUsed) {{
        $script:wrongVolumeCleanupWarning = ([string]$script:wrongVolumeCleanupWarning + '; ' + [string]$removeResult.warning)
      }}
      $remaining = @()
      for ($attempt = 0; $attempt -lt 20; $attempt++) {{
        $remaining = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
        if ($remaining.Count -eq 0) {{ break }}
        Start-Sleep -Milliseconds 250
      }}
      if ($remaining.Count -gt 0) {{
        throw 'wrong-volume Codex registration remains after cleanup'
      }}
    }} catch {{
      $script:wrongVolumeCleanupError = [string]$_.Exception.Message
    }}
  }}
  $movedExisting = $false
  $skipAdd = $false
  $useExplicitAppDataVolume = $true
  if ($useExplicitAppDataVolume) {{
    $operation = 'StagePackageAsync USB + RegisterPackagesByFullNameAsync system AppDataVolume'
    $journalPath = Join-Path (Join-Path (Join-Path $env:LOCALAPPDATA 'CodexAppxDataBackups') 'OpenAI.Codex_2p2nqsd0c76g0') 'recovery.json'
    Assert-CodexBackupNoLinkAncestors $journalPath
    $journalPresent = $false
    foreach ($candidate in @($journalPath, ($journalPath + '.bak'))) {{
      try {{ $null = Get-Item -LiteralPath $candidate -Force -ErrorAction Stop; $journalPresent = $true }}
      catch {{ if ($_.CategoryInfo.Category -ne [Management.Automation.ErrorCategory]::ObjectNotFound) {{ throw }} }}
    }}
    $existingRoot = Get-CodexPackageVolumeRoot $existing
    $existingData = Get-CodexPackageDataPlacement $mountPoint
    $alreadyComplete = $null -ne $existing -and
      [string]$existing.PackageFullName -ieq $expectedPackageFullName -and
      $null -ne $existingRoot -and $existingRoot.TrimEnd('\') -ieq $mountPoint.TrimEnd('\') -and
      $existingData.state -in @('system-volume-observed', 'contained-links-verified', 'not-created')
    if ($alreadyComplete -and -not $journalPresent) {{
      $controlledRecoveryNote = 'USB program and system-volume data already satisfy placement checks; no reinstall or data removal was performed'
    }} else {{
      $callbacks = @{{
        GetPackage = {{ Get-CodexRecoveryCurrentPackage }}
        AssertOwnership = {{ param($package) Assert-CodexRecoveryInstallOwnership $package }}
        RepairMachineState = {{
          param($backupPath, $packagePath, $journal, $context)
          Repair-CodexOrphanMachineState $backupPath $packagePath $journal $context
        }}
        StopPackage = {{ param($package) Assert-CodexRecoveryPackageStopped $package }}
        BackupPackage = {{ param($package, $usbRoot, $usbName) Backup-CodexPackageData $package $usbRoot $usbName }}
        RemovePackage = {{
          param($package, $backupPath)
          $current = Get-CodexRecoveryCurrentPackage
          if ($null -eq $current -or [string]$current.PackageFullName -ine [string]$package.PackageFullName) {{
            throw 'Recovery registration changed before removal; no removal was started'
          }}
          Assert-CodexRecoveryInstallOwnership $current
          Assert-CodexRecoveryPackageStopped $current
          if ($backupPath) {{ Assert-CodexBackupStillMatches $current $backupPath $mountPoint ([string]$volume.Name) }}
          else {{ Assert-CodexPackageDataAbsent }}
          # The module persists its removal journal and verified backup before
          # invoking this exact current-user, ordinary signed-package removal.
          Remove-AppxPackage -Package ([string]$current.PackageFullName) -ErrorAction Stop | Out-Null
        }}
        AssertDataPlacement = {{ param($usbRoot) Get-CodexPackageDataPlacement $usbRoot }}
      }}
      $controlledRecoveryStarted = $true
      $updateExisting = $null -ne $existing -and [string]$existing.PackageFullName -ine $expectedPackageFullName
      if ($updateExisting) {{
        # This gate is BEFORE Move. The recovery module also rejects downgrade,
        # but that later check cannot undo an already-moved existing package.
        if ([string]$existing.PackageFullName -notmatch '^OpenAI\.Codex_(?<version>[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+)_x64__2p2nqsd0c76g0$') {{
          throw 'Existing USB update package version/identity cannot be verified; no package was moved'
        }}
        $existingVersion = [version]$Matches.version
        $requestedVersion = [version]$expectedPackageFullName.Split('_')[1]
        if ([version]$existing.Version -ne $existingVersion) {{ throw 'Existing package version metadata is inconsistent; no package was moved' }}
        if ($existingVersion -gt $requestedVersion) {{
          throw 'USB explicit-data deployment does not downgrade a newer existing package; no package was moved or removed'
        }}
        if ([string]::IsNullOrWhiteSpace([string]$existingRoot)) {{
          throw 'Existing package location is unknown; no package was moved or removed'
        }}
        Assert-CodexRecoveryPackageStopped $existing
      }}
      if ($updateExisting -and $existingRoot.TrimEnd('\') -ine $mountPoint.TrimEnd('\')) {{
        $operation = 'Move-AppxPackage (existing version before explicit-data update)'
        Assert-CodexMoveOwnership ([string]$existing.PackageFullName)
        Move-AppxPackage -Package ([string]$existing.PackageFullName) -Volume $volume -ErrorAction Stop | Out-Null
        $moved = Get-CodexRecoveryCurrentPackage
        $movedRoot = Get-CodexPackageVolumeRoot $moved
        if ($null -eq $moved -or [string]$moved.PackageFullName -ine [string]$existing.PackageFullName -or
            $null -eq $movedRoot -or $movedRoot.TrimEnd('\') -ine $mountPoint.TrimEnd('\')) {{
          throw 'Existing-version move did not retain its exact identity on USB; package and data were retained'
        }}
        $existing = $moved
      }}
      $controlledResult = Invoke-CodexDataRecovery -PackagePath ([string]$installArgs['Path']) -ExpectedFullName $expectedPackageFullName -UsbRoot $mountPoint -UsbVolume $volume -Callbacks $callbacks -NonDestructiveUpdate:$updateExisting -MachineRepairRequired:$script:machineOrphanRepairRequired
      if (-not [bool]$controlledResult.success) {{ throw 'Controlled USB deployment did not complete; recovery evidence was retained' }}
      $controlledRecoveryNote = [string]$controlledResult.message + '; recoveryJournal=' + [string]$controlledResult.journalPath + '; originalBackup=' + [string]$controlledResult.originalBackup + '; preUpdateBackup=' + [string]$controlledResult.preUpdateBackup + '; baselineBackup=' + [string]$controlledResult.baselineBackup
    }}
    $skipAdd = $true
  }} else {{
  # A failed move NEVER authorizes deleting a pre-existing registration/data.
  if ($null -ne $existing) {{
    $existingRoot = Get-CodexPackageVolumeRoot $existing
    if ([string]::IsNullOrWhiteSpace($existingRoot)) {{
      throw 'Existing Codex package location could not be resolved; automatic migration is unsafe'
    }}
    if ($existingRoot.TrimEnd('\') -ine $mountPoint.TrimEnd('\')) {{
      if ($sharedPackagePresent) {{ throw 'Existing Codex package is shared/staged/provisioned outside this operation; automatic cross-volume move is not authorized' }}
      if (-not (Get-Command Move-AppxPackage -ErrorAction SilentlyContinue)) {{
        throw 'Codex exists on another volume, but Move-AppxPackage is unavailable; existing package was retained'
      }}
      $operation = 'Move-AppxPackage (existing registration to USB)'
      Assert-CodexMoveOwnership ([string]$existing.PackageFullName)
      Move-AppxPackage -Package ([string]$existing.PackageFullName) -Volume $volume -ErrorAction Stop | Out-Null
      $movedExisting = $true
    }}
  }}
  # Strict USB mode never bootstraps on C: and never omits the target volume.
  $installArgs['Volume'] = $volume
  }}

"#,
                name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
                root = ps_quote(&root.to_string_lossy()),
                preserve_compatibility_pattern = ps_quote(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN),
                data_backup_script = APPX_DATA_BACKUP_SCRIPT,
                machine_state_script = APPX_MACHINE_CLEANUP_STATE_SCRIPT,
                machine_rehydration_script = APPX_MACHINE_REHYDRATION_SCRIPT,
            )
        })
        .unwrap_or_default();
    format!(
        r#"
$ErrorActionPreference = 'Stop'
{package_path_script}
 try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}
 try {{
  $cmd = Get-Command Add-AppxPackage -ErrorAction Stop
 $installArgs = @{{ ErrorAction = 'Stop' }}
  $movedExisting = $false
  $skipAdd = $false
 $nativeRecoveryNote = $null
 $usbVolumeMode = $false
 $directError = $null
 $nativeAppxSucceeded = $false
 $nativeAppxError = $null
 $nativeAppxResult = $null
  # USB placement is a hard postcondition. A successful Add-AppxPackage
  # return only means the cmdlet did not throw; older AppX implementations have
  # been observed to ignore a removable -Volume object and register the package
  # on the system volume instead. Keep enough state to diagnose and, when this
  # invocation created the registration, remove that accidental registration.
  $initialPackageFullName = $null
  $expectedPackageFullName = {expected_package_full_name}
  $initialPackageSnapshotComplete = $false
  $deploymentAttempted = $false
  $controlledRecoveryStarted = $false
  $controlledRecoveryNote = $null
  $actualInstallLocation = $null
  $actualPackageVolume = $null
  $script:wrongVolumeCleanupAttempted = $false
  $script:wrongVolumeCleanupError = $null
  $script:wrongVolumeCleanupWarning = $null
  $preserveDataFallbackWarning = $null
  $nativePlacementError = $null
  $wrongVolumeMoveError = $null
  $wrongVolumeMoveRecovered = $false
  $sharedPackagePresent = $false
  $script:machineOwnershipSnapshot = $null
  $script:machineOrphanRepairRequired = $false
  $script:machineOrphanRepairAttempted = $false
  $script:machineOrphanRepairCompleted = $false
  $script:machineOrphanRepairApi = $null
  $script:machineOrphanRepairError = $null
   $operation = 'initializing'
  # Add-AppxPackage documents the typed -Path parameter. Always use it for
  # the package install, including the USB -Volume parameter set; the legacy
  # LiteralPath alias is not present on every AppX module and can select a
  # different binder path on older Windows builds.
  if ($cmd.Parameters.Keys -notcontains 'Path') {{
    throw 'Add-AppxPackage on this Windows build does not expose -Path'
  }}
   $installArgs['Path'] = {path}
  if ($cmd.Parameters.Keys -contains 'ForceUpdateFromAnyVersion') {{
    $installArgs['ForceUpdateFromAnyVersion'] = $true
  }}
  if ($cmd.Parameters.Keys -contains 'ForceTargetApplicationShutdown') {{
    $installArgs['ForceTargetApplicationShutdown'] = $true
  }} elseif ($cmd.Parameters.Keys -contains 'ForceApplicationShutdown') {{
    $installArgs['ForceApplicationShutdown'] = $true
  }}
{volume_setup}
   if (-not $skipAdd) {{
     try {{
       $operation = if ($usbVolumeMode) {{ 'Add-AppxPackage (USB AppX volume)' }} else {{ 'Add-AppxPackage' }}
       $deploymentAttempted = $true
       Add-AppxPackage @installArgs
     }} catch {{
       $directError = [string]$_.Exception.Message
       $isUsbVolumeUriError = $usbVolumeMode -and
         ($directError -match '(?i)Invalid URI|hostname could not be parsed|无效的 URI|未能分析主机名')
       if (-not $isUsbVolumeUriError) {{ throw }}
       # Only bypass the URI binder; the alternative targets the SAME USB volume.
       $operation = 'PackageManager.AddPackageAsync (USB AppX volume after URI error)'
       try {{
         $nativeAppxResult = Invoke-NativeAppxAdd -packagePath ([string]$installArgs['Path']) -targetVolume $volume
         $nativeAppxSucceeded = $true
         $nativeRecoveryNote = "USB AppX 卷 URI 兼容重试已改用 WinRT PackageManager 直接安装：$directError"
       }} catch {{
         $nativeAppxError = [string]$_.Exception.Message
         throw "USB deployment failed; system-volume fallback is disabled. Direct error: $directError; native error: $nativeAppxError"
       }}
     }}
   }}
   # A successful async/cmdlet return is not the final contract. Windows 10 can
   # publish PackageUser slightly later and DeploymentResult.IsRegistered can be
   # false even when registration committed. Converge on the independent exact
   # current-user health projection before any placement/success decision.
   $p = if ($usbVolumeMode) {{
     Wait-CodexCurrentUserPackageProjection $expectedPackageFullName $nativeAppxResult
   }} else {{
     @(Get-AppxPackage -Name {name} -ErrorAction Stop)[0]
   }}
   if ($usbVolumeMode) {{
     if ($null -eq $p) {{
       throw "USB AppX placement verification failed: expectedPackageVolume=$mountPoint actualPackageVolume=none package registration was not observable after deployment"
     }}
     if ([string]$p.PackageFullName -ine $expectedPackageFullName) {{
       throw "USB deployed registration does not match this verified payload: expected=$expectedPackageFullName actual=$($p.PackageFullName)"
     }}
     $actualInstallLocation = ([string]$p.InstallLocation).Trim()
     $actualPackageVolume = Get-CodexPackageVolumeRoot $p
     $expectedPackageVolume = $mountPoint.TrimEnd('\\') + '\\'
     $actualPackageVolumeNormalized = if ($actualPackageVolume) {{
       $actualPackageVolume.TrimEnd('\\') + '\\'
     }} else {{ $null }}
     if ($null -eq $actualPackageVolumeNormalized -or
         $actualPackageVolumeNormalized -ine $expectedPackageVolume) {{
       # Some Windows builds reuse an already-staged package on C: even when
       # Add was explicitly targeted at USB. Try the documented move operation
       # on only the exact registration created by this attempt, preserving its
       # data and avoiding unnecessary uninstall/reinstall cycles. Never use an
       # intentional system-volume Add as a bootstrap.
       if ([string]::IsNullOrWhiteSpace([string]$initialPackageFullName) -and
           $initialPackageSnapshotComplete -and $deploymentAttempted -and
           -not $sharedPackagePresent -and
           (Get-Command Move-AppxPackage -ErrorAction SilentlyContinue)) {{
         try {{
           $operation = 'Move-AppxPackage (new verified registration to USB)'
           Assert-CodexMoveOwnership $expectedPackageFullName
           Move-AppxPackage -Package $expectedPackageFullName -Volume $volume -ErrorAction Stop | Out-Null
           $moved = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
           if ($moved.Count -ne 1 -or [string]$moved[0].PackageFullName -ine $expectedPackageFullName) {{
             throw 'Move result did not retain the exact verified package identity'
           }}
           $movedRoot = Get-CodexPackageVolumeRoot $moved[0]
           if ($null -eq $movedRoot -or $movedRoot.TrimEnd('\') -ine $mountPoint.TrimEnd('\')) {{
             throw 'Move returned success but package location is still outside the USB volume'
           }}
           $p = $moved[0]
           $actualInstallLocation = ([string]$p.InstallLocation).Trim()
           $actualPackageVolume = $movedRoot
           $wrongVolumeMoveRecovered = $true
         }} catch {{
           $wrongVolumeMoveError = [string]$_.Exception.Message
         }}
       }}
       if (-not $wrongVolumeMoveRecovered) {{
       Remove-NewWrongVolumeRegistration $p
       # If this attempt created the wrong-volume registration, make one native
       # WinRT placement retry after removing it. This covers Windows builds
       # where the PowerShell -Volume binder returns success but silently uses
       # the default C: volume. The retry is still tied to the already-resolved
       # native AppxVolume object; it never falls back to a portable copy.
       $nativePlacementRecovered = $false
       if ($wrongVolumeCleanupAttempted -and -not $wrongVolumeCleanupError) {{
         try {{
           $operation = 'PackageManager.AddPackageAsync (USB AppX volume after placement mismatch)'
           $nativeAppxResult = Invoke-NativeAppxAdd -packagePath ([string]$installArgs['Path']) -targetVolume $volume
           $p = Wait-CodexCurrentUserPackageProjection $expectedPackageFullName $nativeAppxResult
           if ($null -eq $p -or [string]$p.PackageFullName -ine $expectedPackageFullName) {{
             throw 'Native USB retry did not register the verified payload identity'
           }}
           $actualInstallLocation = if ($null -ne $p) {{ ([string]$p.InstallLocation).Trim() }} else {{ '' }}
           $actualPackageVolume = Get-CodexPackageVolumeRoot $p
           $actualPackageVolumeNormalized = if ($actualPackageVolume) {{
             $actualPackageVolume.TrimEnd('\\') + '\\'
           }} else {{ $null }}
           if ($null -eq $actualPackageVolumeNormalized -or
               $actualPackageVolumeNormalized -ine $expectedPackageVolume) {{
             throw "native placement retry landed at $(if ($actualPackageVolume) {{ $actualPackageVolume }} else {{ 'unknown' }})"
           }}
           $nativePlacementRecovered = $true
           $nativeAppxSucceeded = $true
           $nativeRecoveryNote = 'PowerShell -Volume 返回成功但未落到目标卷，已改用 WinRT PackageManager 重新部署'
         }} catch {{
           $nativePlacementError = [string]$_.Exception.Message
         }}
       }}
       if (-not $nativePlacementRecovered) {{
          $cleanupDetail = if ($wrongVolumeCleanupAttempted) {{
            if ($wrongVolumeCleanupError) {{ "wrongVolumeCleanupError=$wrongVolumeCleanupError" }} else {{ 'wrongVolumeCleanup=completed' }}
          }} else {{ 'wrongVolumeCleanup=skipped-existing-registration' }}
          $cleanupWarning = if ($wrongVolumeCleanupWarning) {{ " wrongVolumeCleanupWarning=$wrongVolumeCleanupWarning" }} else {{ '' }}
          $dataConflictWarning = if ($preserveDataFallbackWarning) {{ " preserveDataFallbackWarning=$preserveDataFallbackWarning" }} else {{ '' }}
          $nativeDetail = if ($nativePlacementError) {{ " nativePlacementRetryError=$nativePlacementError" }} else {{ '' }}
          throw "USB AppX placement verification failed: expectedPackageVolume=$expectedPackageVolume actualPackageVolume=$(if ($actualPackageVolume) {{ $actualPackageVolume }} else {{ 'unknown' }}) actualInstallLocation=$actualInstallLocation $cleanupDetail$cleanupWarning$dataConflictWarning$nativeDetail"
       }}
       }}
     }}
   }}
   if ($usbVolumeMode) {{
     $dataPlacementFailure = 'Codex 程序已在 U 盘，已保留；但尚不能确认数据保存在本机，配置未完成。请勿反复卸载或清除数据，请保留下方目录链接诊断。USB package installation was retained, but local data placement could not be verified; installation is incomplete.'
     try {{ $dataPlacement = Get-CodexPackageDataPlacement $mountPoint }}
     catch {{ throw "$dataPlacementFailure $([string]$_.Exception.Message)" }}
     $dataPlacementNote = 'Package data placement: ' + ($dataPlacement | ConvertTo-Json -Depth 5 -Compress)
     if ($dataPlacement.state -eq 'not-created') {{
       $dataPlacementNote = 'Package data has not been created; local data placement must still be verified after first app launch. ' + $dataPlacementNote
     }}
     if ($dataPlacement.state -notin @('system-volume-observed', 'contained-links-verified', 'not-created')) {{
       throw "$dataPlacementFailure $dataPlacementNote"
     }}
   }}
   $machineRepairNote = if ($script:machineOrphanRepairCompleted) {{
     '已通过 Windows AppX 正式接口清理无用户且无有效路径的精确孤儿包状态；repairApi=' + [string]$script:machineOrphanRepairApi + '；未编辑 PackageRepository 注册表'
   }} else {{ $null }}
   $dataRetentionWarnings = @($machineRepairNote, $wrongVolumeCleanupWarning, $preserveDataFallbackWarning, $dataPlacementNote) |
     Where-Object {{ -not [string]::IsNullOrWhiteSpace([string]$_) }}
   $dataRetentionWarningSuffix = if ($dataRetentionWarnings.Count -gt 0) {{
     '；' + ($dataRetentionWarnings -join '；')
   }} else {{ '' }}
   @{{
     success = $true
    message = if ($controlledRecoveryNote) {{
      "$controlledRecoveryNote$dataRetentionWarningSuffix"
    }} elseif ($wrongVolumeMoveRecovered) {{
      "Windows reused a system-volume registration; the exact new package was moved to USB and its location verified$dataRetentionWarningSuffix"
    }} elseif ($nativeAppxSucceeded -and $nativeRecoveryNote) {{
      "$nativeRecoveryNote$dataRetentionWarningSuffix；USB AppX 安装成功"
    }} elseif ($movedExisting) {{
      "Existing Codex package moved to the USB AppX volume; Add-AppxPackage succeeded$dataRetentionWarningSuffix"
    }} else {{
      "Add-AppxPackage succeeded$dataRetentionWarningSuffix"
    }}
    fallbackRecommended = $false
    rawError = $null
    installed = if ($null -ne $p) {{
      {installed_msix_object}
    }} else {{ $null }}
  }} | ConvertTo-Json -Compress -Depth 4
 }} catch {{
  $deploymentFailure = $_
  # Add-AppxPackage often reports only a generic message while the actionable
  # deployment reason is stored under an ActivityId.  Capture both without
  # making the install depend on Get-AppxLog (it is absent on some stripped
  # Windows images and on our constrained test host).
  # If Add/Move failed after creating a new registration, do not leave a
  # package on C: that looks installed while the USB operation is reported as
  # failed. Existing registrations are never removed by this safety cleanup.
  if (-not $controlledRecoveryStarted -and $usbVolumeMode -and $initialPackageSnapshotComplete -and $deploymentAttempted -and
      [string]::IsNullOrWhiteSpace([string]$initialPackageFullName)) {{
    try {{
      $leftover = @(Get-AppxPackage -Name {name} -ErrorAction Stop |
        Sort-Object -Property Version -Descending |
        Select-Object -First 1)[0]
      if ($null -ne $leftover) {{
        $leftoverRoot = Get-CodexPackageVolumeRoot $leftover
        $targetRoot = $mountPoint.TrimEnd('\\') + '\\'
        if ($null -eq $leftoverRoot -or
            (($leftoverRoot.TrimEnd('\\') + '\\') -ine $targetRoot)) {{
          Remove-NewWrongVolumeRegistration $leftover
          $actualInstallLocation = ([string]$leftover.InstallLocation).Trim()
          $actualPackageVolume = $leftoverRoot
        }}
      }}
    }} catch {{
      if (-not $wrongVolumeCleanupError) {{
        $wrongVolumeCleanupError = [string]$_.Exception.Message
      }}
    }}
  }}
  $rawMessage = [string]$deploymentFailure.Exception.Message
  $rawError = [string]$deploymentFailure
  $hresult = $null
  if ($rawError -match '(?i)(0x[0-9a-f]{{8}})') {{
    $hresult = $Matches[1].ToUpperInvariant()
  }}
  $activityId = $null
  if ($rawError -match '(?i)(?:Activity(?:Id| ID)|ActivityID)[^0-9a-f]*([0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}})') {{
    $activityId = $Matches[1]
  }}
  $deploymentLog = $null
  try {{
    if ($activityId -and (Get-Command Get-AppxLog -ErrorAction SilentlyContinue)) {{
      $events = @(Get-AppxLog -ActivityId $activityId -ErrorAction SilentlyContinue | Select-Object -Last 8)
      $lines = @()
      foreach ($event in $events) {{
        $line = [string]$event.Message
        if ($event.Id) {{ $line = "id=$($event.Id) $line" }}
        if ($event.TimeCreated) {{ $line = "$($event.TimeCreated) $line" }}
        if ($line) {{ $lines += $line }}
      }}
      $deploymentLog = $lines -join "`n"
      if ($deploymentLog.Length -gt 6000) {{
        $deploymentLog = $deploymentLog.Substring(0, 6000)
      }}
    }}
  }} catch {{
    # Diagnostics must never turn a deployment failure into a script failure.
  }}
  $details = @($rawMessage)
  if ($operation) {{ $details += "operation=$operation" }}
  if ($usbVolumeMode -and $mountPoint) {{
    $details += "targetVolume=$mountPoint"
    if ($fileSystem) {{ $details += "volumeFileSystem=$fileSystem" }}
    if ($packageStorePath) {{ $details += "packageStore=$packageStorePath" }}
    if ($volume) {{
      $details += "appxVolumeType=$($volume.GetType().FullName)"
      $details += "appxVolumeIsOffline=$([bool]$volume.IsOffline)"
      $details += "appxVolumeName=$([string]$volume.Name)"
      $details += "appxVolumeMountPoint=$([string]$volume.MountPoint)"
      $details += "appxVolumePackageStore=$([string]$volume.PackageStorePath)"
    }}
     if ($mountError) {{ $details += "appxVolumeMountError=$mountError" }}
     if ($addVolumeError) {{ $details += "appxVolumeAddError=$addVolumeError" }}
     if ($directError) {{ $details += "usbDirectError=$directError" }}
     if ($nativeAppxError) {{ $details += "nativePackageManagerError=$nativeAppxError" }}
     if ($nativePlacementError) {{ $details += "nativePlacementRetryError=$nativePlacementError" }}
     if ($wrongVolumeMoveError) {{ $details += "wrongVolumeMoveError=$wrongVolumeMoveError" }}
     $details += "preexistingSharedOrProvisionedPackage=$sharedPackagePresent"
     if ($script:machineOwnershipSnapshot) {{
       $details += "machineOwnershipClassification=$([string]$script:machineOwnershipSnapshot.classification)"
       $details += "machineOwnershipReason=$([string]$script:machineOwnershipSnapshot.reason)"
       $details += "machineOwnershipLocationState=$([string]$script:machineOwnershipSnapshot.locationState)"
       $details += "machineOwnershipOwners=$(@($script:machineOwnershipSnapshot.ownerStates) -join ',')"
     }}
     $details += "machineOrphanRepairRequired=$([bool]$script:machineOrphanRepairRequired)"
     $details += "machineOrphanRepairAttempted=$([bool]$script:machineOrphanRepairAttempted)"
     $details += "machineOrphanRepairCompleted=$([bool]$script:machineOrphanRepairCompleted)"
     if ($script:machineOrphanRepairApi) {{ $details += "machineOrphanRepairApi=$([string]$script:machineOrphanRepairApi)" }}
     if ($script:machineOrphanRepairError) {{ $details += "machineOrphanRepairError=$([string]$script:machineOrphanRepairError)" }}
     foreach ($machinePackage in @($allUserPackages)) {{
       $details += "preexistingMachinePackage=$([string]$machinePackage.PackageFullName) location=$([string]$machinePackage.InstallLocation)"
     }}
     foreach ($provisionedPackage in @($provisionedPackages)) {{
       $details += "preexistingProvisionedPackage=$([string]$provisionedPackage.PackageName)"
     }}
     if ($nativeAppxResult) {{
       if ($nativeAppxResult.activityId) {{ $details += "nativeActivityId=$($nativeAppxResult.activityId)" }}
       if ($nativeAppxResult.extendedError) {{ $details += "nativeExtendedError=$($nativeAppxResult.extendedError)" }}
       if ($nativeAppxResult.volumeName) {{ $details += "nativeVolumeName=$($nativeAppxResult.volumeName)" }}
       if ($nativeAppxResult.packageUri) {{ $details += "nativePackageUri=$($nativeAppxResult.packageUri)" }}
       if ($nativeAppxResult.errorText) {{ $details += "nativeErrorText=$($nativeAppxResult.errorText)" }}
     }}
     if ($installArgs -and $installArgs['Path']) {{ $details += "packagePath=$($installArgs['Path'])" }}
     if ($actualInstallLocation) {{ $details += "actualInstallLocation=$actualInstallLocation" }}
     if ($actualPackageVolume) {{ $details += "actualPackageVolume=$actualPackageVolume" }}
     if ($script:packagePathEvidence) {{ $details += "packagePathEvidence=$($script:packagePathEvidence | ConvertTo-Json -Compress)" }}
     if ($controlledRecoveryStarted) {{
       $details += '受控安装/数据恢复尚未完成；已验证的备份和恢复记录保留。请勿手动卸载或清除这些目录，先按下方具体错误处理。'
       $details += "recoveryJournal=$journalPath"
       $details += 'The recovery journal, not a generic installed/uninstalled label, records the last durable deployment phase.'
     }}
      if ($wrongVolumeCleanupAttempted) {{
        if ($wrongVolumeCleanupError) {{ $details += "wrongVolumeCleanupError=$wrongVolumeCleanupError" }}
        else {{ $details += 'wrongVolumeCleanup=completed' }}
      }}
      if ($wrongVolumeCleanupWarning) {{ $details += "wrongVolumeCleanupWarning=$wrongVolumeCleanupWarning" }}
      if ($preserveDataFallbackWarning) {{ $details += "preserveDataFallbackWarning=$preserveDataFallbackWarning" }}
    }}
  if ($hresult) {{ $details += "HRESULT=$hresult" }}
  if ($activityId) {{ $details += "ActivityId=$activityId" }}
  if ($deploymentLog) {{ $details += "AppX deployment log:`n$deploymentLog" }}
  @{{
    success = $false
    message = ($details -join "`n")
    fallbackRecommended = -not $usbVolumeMode
    rawError = $rawError
    hresult = $hresult
    activityId = $activityId
    deploymentLog = $deploymentLog
    installed = $null
  }} | ConvertTo-Json -Compress -Depth 4
}}
"#,
        path = ps_quote(&normalize_win32_path_text(&path.to_string_lossy())),
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        installed_msix_object = INSTALLED_MSIX_OBJECT,
        package_path_script = if volume_root.is_none() {
            APPX_PACKAGE_PATH_SCRIPT
        } else {
            ""
        },
        volume_setup = volume_setup,
        expected_package_full_name = ps_quote(expected_package_full_name),
    )
}

#[cfg(all(windows, test))]
fn install_msix_script_with_volume(path: &Path, volume_root: Option<&Path>) -> String {
    let production = install_msix_script_with_volume_and_identity(
        path,
        volume_root,
        "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
    )
    .replace(
        "$useExplicitAppDataVolume = $true",
        "$useExplicitAppDataVolume = $false",
    );
    // Newly added recovery paths must never escape older mocks and call a real
    // deployment cmdlet or inspect/back up this developer's actual AppX data.
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let fixture = std::env::temp_dir().join(format!("codex-sys-isolated-{}", uuid::Uuid::new_v4()));
    format!(
        r#"
if ($env:LOCALAPPDATA -eq {local}) {{ $env:LOCALAPPDATA = {fixture} }}
if ((Get-Command Move-AppxPackage -ErrorAction SilentlyContinue).CommandType -ne 'Function') {{
  function Move-AppxPackage {{ param($Package, $Volume, $ErrorAction); throw 'Move disabled in isolated fixture' }}
}}
if ((Get-Command Get-AppxProvisionedPackage -ErrorAction SilentlyContinue).CommandType -ne 'Function') {{
  function Get-AppxProvisionedPackage {{ param([switch]$Online, $ErrorAction); @() }}
}}
{production}"#,
        local = ps_quote(&local),
        fixture = ps_quote(&fixture.to_string_lossy())
    )
}

#[cfg(all(windows, test))]
fn install_msix_script(path: &Path) -> String {
    install_msix_script_with_volume(path, None)
}

#[cfg(windows)]
pub fn install_msix_sideload(
    path: &Path,
    package_moniker: &str,
) -> Result<MsixSideloadReport, EngineError> {
    let mut recovery_outcome_ambiguous = || {};
    let mut before_install = || Ok(());
    install_msix_sideload_with_observer(
        path,
        package_moniker,
        &mut recovery_outcome_ambiguous,
        &mut before_install,
    )
}

#[cfg(windows)]
pub fn install_msix_sideload_with_observer(
    path: &Path,
    package_moniker: &str,
    recovery_outcome_ambiguous: &mut dyn FnMut(),
    before_install: &mut dyn FnMut() -> Result<(), EngineError>,
) -> Result<MsixSideloadReport, EngineError> {
    install_msix_sideload_to_volume_with_observer(
        path,
        package_moniker,
        None,
        recovery_outcome_ambiguous,
        before_install,
    )
}

#[cfg(windows)]
pub fn install_msix_to_volume_with_observer(
    path: &Path,
    package_moniker: &str,
    volume_root: &Path,
    recovery_outcome_ambiguous: &mut dyn FnMut(),
    before_install: &mut dyn FnMut() -> Result<(), EngineError>,
) -> Result<MsixSideloadReport, EngineError> {
    install_msix_sideload_to_volume_with_observer(
        path,
        package_moniker,
        Some(volume_root),
        recovery_outcome_ambiguous,
        before_install,
    )
}

#[cfg(windows)]
fn install_msix_sideload_to_volume_with_observer(
    path: &Path,
    package_moniker: &str,
    volume_root: Option<&Path>,
    recovery_outcome_ambiguous: &mut dyn FnMut(),
    before_install: &mut dyn FnMut() -> Result<(), EngineError>,
) -> Result<MsixSideloadReport, EngineError> {
    let path_display = path.display();
    if let Some(root) = volume_root {
        log::info!(
            "MSIX sideload start path={path_display} appx_volume={}",
            root.display()
        );
    } else {
        log::info!("MSIX sideload start path={path_display}");
    }
    // USB installation must not pause machine-wide Windows Update/DO/AppX
    // services or clear update caches to force placement. Concurrent system
    // deployment conflicts are reported with their HRESULT/ActivityId instead.
    // Retain the historical recovery path only for ordinary local installs.
    let _restore_guard = if volume_root.is_none() {
        Some(prepare_offline_appx_install(
            package_moniker,
            recovery_outcome_ambiguous,
        )?)
    } else {
        None
    };
    let script = install_msix_script_with_volume_and_identity(path, volume_root, package_moniker);
    // Everything above is pre-package-mutation. An optional elevated recovery
    // may have paused services, but its own watchdog restores them. The app
    // records an ambiguous package outcome from this boundary onward,
    // immediately before the child that can invoke Add-AppxPackage is started.
    before_install()?;
    // USB AppX deployment writes the package through the removable volume and
    // may be serialized behind Defender/AppX service work. Use the longer
    // bounded budget only for that explicit route; normal installs retain the
    // shorter deadline so a broken local AppX service is still surfaced
    // promptly.
    let limits = if volume_root.is_some() {
        RunLimits::usb_install()
    } else {
        RunLimits::install()
    };
    let json = run_powershell_json_with_limits(&script, limits)
        .map_err(|e| EngineError::Install(format!("run MSIX deployment: {}", e.message())))?;
    let mut report: MsixSideloadReport = serde_json::from_str(&json)
        .map_err(|e| EngineError::Install(format!("parse MSIX deployment result: {e}")))?;
    if let Some(installed) = report.installed.take() {
        report.installed = Some(prefer_codex_app_version(installed));
    }
    if report.success {
        let path_display = path.display();
        log::info!("MSIX sideload completed path={path_display}");
    } else {
        let path_display = path.display();
        let error = &report.message;
        log::error!("MSIX sideload failed path={path_display} error={error}");
    }
    Ok(report)
}

#[cfg(not(windows))]
pub fn install_msix_sideload(
    _path: &Path,
    _package_moniker: &str,
) -> Result<MsixSideloadReport, EngineError> {
    log::info!("MSIX sideload start path=unsupported-platform");
    log::error!("MSIX sideload failed error=unsupported-platform");
    Ok(MsixSideloadReport {
        success: false,
        message: "MSIX sideloading is only available on Windows".to_string(),
        installed: None,
        fallback_recommended: true,
        raw_error: None,
        hresult: None,
        activity_id: None,
        deployment_log: None,
    })
}

#[cfg(not(windows))]
pub fn install_msix_sideload_with_observer(
    path: &Path,
    package_moniker: &str,
    _recovery_outcome_ambiguous: &mut dyn FnMut(),
    _before_install: &mut dyn FnMut() -> Result<(), EngineError>,
) -> Result<MsixSideloadReport, EngineError> {
    install_msix_sideload(path, package_moniker)
}

#[cfg(not(windows))]
pub fn install_msix_to_volume_with_observer(
    path: &Path,
    package_moniker: &str,
    _volume_root: &Path,
    recovery_outcome_ambiguous: &mut dyn FnMut(),
    before_install: &mut dyn FnMut() -> Result<(), EngineError>,
) -> Result<MsixSideloadReport, EngineError> {
    install_msix_sideload_with_observer(
        path,
        package_moniker,
        recovery_outcome_ambiguous,
        before_install,
    )
}

/// PRE-check, before sideloading, that every redistributable framework the
/// staged MSIX declares as a `PackageDependency` is already installed. Reads the
/// staged manifest's `PackageDependency` entries (same source the post-install
/// `verify_msix_health` inspects), filters to the framework packages, and asks
/// `Get-AppxPackage` whether each is present. A missing framework means the
/// sideload cannot succeed on this machine (no Store / App Installer to acquire
/// it), so the caller routes straight to the portable build.
///
/// This is intentionally conservative: if the manifest cannot be read or the
/// PowerShell probe cannot run, it returns `checked = false` and does NOT block
/// the sideload — the existing post-install health check + portable fallback
/// remain the backstop.
#[cfg(windows)]
pub fn precheck_msix_dependencies(path: &Path) -> MsixDependencyPrecheck {
    let frameworks = match crate::msix::read_msix_dependencies(path) {
        Ok(deps) => crate::msix::framework_dependencies(&deps),
        Err(err) => {
            log::warn!("MSIX dependency precheck could not read manifest error={err}");
            return MsixDependencyPrecheck {
                checked: false,
                frameworks_ok: true,
                missing_frameworks: vec![],
                reason: format!("could not read staged MSIX dependencies: {err}"),
            };
        }
    };

    if frameworks.is_empty() {
        log::info!("MSIX dependency precheck completed missing=[]");
        return MsixDependencyPrecheck {
            checked: true,
            frameworks_ok: true,
            missing_frameworks: vec![],
            reason: String::new(),
        };
    }

    let package_architecture = crate::msix::read_msix_identity(path)
        .ok()
        .map(|identity| identity.processor_architecture);
    let deps_literal = frameworks
        .iter()
        .map(|dep| {
            format!(
                "@{{ Name = {name}; Publisher = {publisher}; MinVersion = {min_version}; ProcessorArchitecture = {arch} }}",
                name = ps_quote(&dep.name),
                publisher = ps_nullable(dep.publisher.as_deref()),
                min_version = ps_nullable(dep.min_version.as_deref()),
                arch = ps_nullable(dep.processor_architecture.as_deref()),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let package_arch_literal = ps_nullable(package_architecture.as_deref());
    let script = format!(
        r#"
	$ErrorActionPreference = 'SilentlyContinue'
	$deps = @({deps_literal})
	$mainArch = {package_arch_literal}
	$missing = @()
	function Convert-ToVersion($value) {{
	  try {{
	    $text = [string]$value
	    if ([string]::IsNullOrWhiteSpace($text)) {{ return $null }}
	    return [version]$text
	  }} catch {{
	    return $null
	  }}
	}}
	function Same-Publisher($package, [string]$publisher) {{
	  if ([string]::IsNullOrWhiteSpace($publisher)) {{ return $true }}
	  return [string]$package.Publisher -eq $publisher
	}}
	function Same-Architecture($package, [string]$required) {{
	  if ([string]::IsNullOrWhiteSpace($required) -or $required -eq 'neutral') {{ return $true }}
	  $arch = [string]$package.Architecture
	  return [string]::IsNullOrWhiteSpace($arch) -or $arch -eq 'Neutral' -or $arch -eq $required
	}}
	foreach ($d in $deps) {{
	  $name = [string]$d.Name
	  if ([string]::IsNullOrWhiteSpace($name)) {{ continue }}
	  $publisher = [string]$d.Publisher
	  $requiredArch = [string]$d.ProcessorArchitecture
	  if ([string]::IsNullOrWhiteSpace($requiredArch)) {{ $requiredArch = $mainArch }}
	  $candidates = @(Get-AppxPackage -Name $name -ErrorAction SilentlyContinue)
	  if ($candidates.Count -eq 0) {{
	    $missing += "$name not installed"
	    continue
	  }}
	  $publisherCandidates = @($candidates | Where-Object {{ Same-Publisher $_ $publisher }})
	  if ($candidates.Count -gt 0 -and $publisherCandidates.Count -eq 0) {{
	    $missing += "$name publisher $publisher not installed"
	    continue
	  }}
	  $archCandidates = @($publisherCandidates | Where-Object {{ Same-Architecture $_ $requiredArch }})
	  if ($publisherCandidates.Count -gt 0 -and $archCandidates.Count -eq 0) {{
	    $missing += "$name architecture $requiredArch not installed"
	    continue
	  }}
	  $depPkg = $archCandidates |
	    Sort-Object -Property @{{ Expression = {{ Convert-ToVersion $_.Version }}; Descending = $true }} |
	    Select-Object -First 1
	  if ($null -eq $depPkg) {{
	    $missing += "$name not installed"
	    continue
	  }}
	  $minText = [string]$d.MinVersion
	  $min = Convert-ToVersion $minText
	  $installedVersion = Convert-ToVersion $depPkg.Version
	  if ($null -ne $min -and $null -ne $installedVersion -and $installedVersion -lt $min) {{
	    $missing += "$name >= $minText required (installed $($depPkg.Version))"
	  }}
	}}
	@{{
	  missing = ($missing -join ', ')
	}} | ConvertTo-Json -Compress
	"#,
    );

    let parsed = run_powershell_json(&script)
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());

    let Some(value) = parsed else {
        // Probe could not run — don't overturn the sideload on an unknown signal.
        return MsixDependencyPrecheck {
            checked: false,
            frameworks_ok: true,
            missing_frameworks: vec![],
            reason: "framework dependency probe could not run; proceeding with sideload"
                .to_string(),
        };
    };

    let missing_frameworks: Vec<String> = value
        .get("missing")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let frameworks_ok = missing_frameworks.is_empty();
    if !frameworks_ok {
        let missing = missing_frameworks.join(",");
        log::warn!("MSIX dependency precheck missing frameworks missing={missing}");
    }
    let reason = if frameworks_ok {
        String::new()
    } else {
        format!(
            "required framework packages are not installed: {}",
            missing_frameworks.join(", ")
        )
    };

    MsixDependencyPrecheck {
        checked: true,
        frameworks_ok,
        missing_frameworks,
        reason,
    }
}

#[cfg(not(windows))]
pub fn precheck_msix_dependencies(_path: &Path) -> MsixDependencyPrecheck {
    // Non-Windows builds never sideload, so there is nothing to pre-check. Report
    // checked = false / frameworks_ok = true so this can never block a path that
    // does not exist off Windows.
    MsixDependencyPrecheck {
        checked: false,
        frameworks_ok: true,
        missing_frameworks: vec![],
        reason: "MSIX dependency pre-checks are only meaningful on Windows".to_string(),
    }
}

#[cfg(windows)]
pub fn remove_msix_package() -> Result<MsixRemoveReport, EngineError> {
    remove_msix_package_with_options(false)
}

/// Remove every current-user registration of the Codex AppX package.  When
/// `preserve_application_data` is true, ask Windows to retain the package's
/// per-user data; the caller can then report a clean program uninstall while
/// honoring the UI's "keep my data" choice.  The postcondition check remains
/// mandatory in both modes so a delayed AppX transaction is never presented as
/// a successful uninstall.
#[cfg(windows)]
fn remove_msix_package_script(preserve_application_data: bool) -> String {
    format!(
        r#"
	$ErrorActionPreference = 'Stop'
	try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}
	$script:notes = @()
{data_backup_script}
	$preserveApplicationData = {preserve_application_data}
	$script:preserveApplicationDataFallbackUsed = $false
	function Test-PreserveApplicationDataUnsupported([string]$detail) {{
	  if ([string]::IsNullOrWhiteSpace($detail)) {{ return $false }}
	  # Windows can expose -PreserveApplicationData but reject it at runtime for
	  # a package registered by a plain sideload rather than deployment mode.
	  # Keep the classifier narrow so unrelated policy/identity failures remain
	  # hard failures instead of being hidden by a second removal attempt.
	  return $detail -match {preserve_compatibility_pattern}
	}}
	function Add-ResidualNotes {{
	  try {{
	    $allUsers = @(Get-AppxPackage -AllUsers -Name {name} -ErrorAction Stop)
	    if ($allUsers.Count -gt 0) {{
	      $script:notes += 'MSIX package still exists for another user or elevated context: ' + (($allUsers | ForEach-Object {{ $_.PackageFullName }}) -join ', ')
	    }}
	  }} catch {{
	    $script:notes += 'Could not query all-user MSIX registrations: ' + [string]$_.Exception.Message
	  }}
	  try {{
	    $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop | Where-Object {{ $_.DisplayName -eq {name} }})
	    if ($provisioned.Count -gt 0) {{
	      $script:notes += 'Provisioned MSIX package remains and may be reinstalled for new users; remove it from an elevated shell with Remove-AppxProvisionedPackage if desired.'
	    }}
	  }} catch {{
	    $script:notes += 'Could not query provisioned MSIX packages: ' + [string]$_.Exception.Message
	  }}
	}}
	try {{
	  $packages = Get-AppxPackage -Name {name} -ErrorAction Stop
	  if (-not $packages) {{
	    Add-ResidualNotes
	    @{{
	      success = $true
	      message = 'MSIX package was not installed'
	      rawError = $null
	      notes = $script:notes
	    }} | ConvertTo-Json -Compress
	    exit 0
	  }}
	  $removeCmd = Get-Command Remove-AppxPackage -ErrorAction Stop
	  foreach ($p in $packages) {{
	    if ([string]$p.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0') {{
	      throw 'Unexpected Codex publisher identity; no matching-name packages were removed'
	    }}
	  }}
	  foreach ($p in $packages) {{
	    $removeArgs = @{{ Package = [string]$p.PackageFullName; ErrorAction = 'Stop' }}
	    $preserveAttempted = $false
	    $backupPath = $null
	    if ($preserveApplicationData) {{
	      if ([bool]$p.IsDevelopmentMode -and $removeCmd.Parameters.Keys -contains 'PreserveApplicationData') {{
	        $removeArgs['PreserveApplicationData'] = $true
	        $preserveAttempted = $true
	      }} else {{
	        $backupPath = Backup-CodexPackageData $p
	        if ($backupPath) {{ Assert-CodexBackupStillMatches $p $backupPath }}
	        else {{ Assert-CodexPackageDataAbsent }}
	        $script:preserveApplicationDataFallbackUsed = $true
	        $script:notes += if ($backupPath) {{
	          "Package-local data backed up and SHA256 verified at $backupPath; retain this directory for recovery."
	        }} else {{ 'No package-local data directory exists; external Codex data is not removed.' }}
	      }}
	    }}
	    $removed = $false
	    $attempt = 0
	    while ($attempt -lt 8 -and -not $removed) {{
	      if ($preserveApplicationData -and -not $preserveAttempted) {{
	        if ($backupPath) {{ Assert-CodexBackupStillMatches $p $backupPath }}
	        else {{ Assert-CodexPackageDataAbsent }}
	      }}
	      try {{
	        Remove-AppxPackage @removeArgs
	        $removed = $true
	      }} catch {{
	        $detail = [string]$_
	        if ($preserveAttempted -and (Test-PreserveApplicationDataUnsupported $detail)) {{
	          # Flag incompatibility is not authorization to discard data.
	          $backupPath = Backup-CodexPackageData $p
	          if ($backupPath) {{ Assert-CodexBackupStillMatches $p $backupPath }}
	          else {{ Assert-CodexPackageDataAbsent }}
	          if ($backupPath) {{
	            $script:notes += "Package-local data backed up and SHA256 verified at $backupPath; retain this directory for recovery."
	          }}
	          $removeArgs.Remove('PreserveApplicationData') | Out-Null
	          $preserveAttempted = $false
	          $script:preserveApplicationDataFallbackUsed = $true
	          $script:notes += 'Windows rejected PreserveApplicationData; retry uses the exact package after checking/backing up its local data.'
	          continue
	        }}
	        if ($attempt -eq 7) {{ throw }}
	        # AppX deployment can release a package lock shortly after the
	        # process close gate returns. Retry this exact package a few times;
	        # permanent policy/identity errors still surface on the final attempt.
	        $attempt++
	        Start-Sleep -Milliseconds 250
	      }}
	    }}
	    if (-not $removed) {{ throw \"Remove-AppxPackage did not complete for $($p.PackageFullName)\" }}
	  }}
	  # Remove-AppxPackage can return before the current-user registration is
	  # observable as absent on slower Windows images.  Verify the postcondition
	  # instead of reporting a false-clean uninstall that will later trigger the
	  # USB install's cross-volume 0x80073CF9 conflict again.
	  $remaining = @()
	  for ($attempt = 0; $attempt -lt 20; $attempt++) {{
	    $remaining = @(Get-AppxPackage -Name {name} -ErrorAction Stop)
	    if ($remaining.Count -eq 0) {{ break }}
	    Start-Sleep -Milliseconds 250
	  }}
	  if ($remaining.Count -gt 0) {{
	    $script:notes += 'Current-user MSIX registration remains after Remove-AppxPackage: ' + (($remaining | ForEach-Object {{ $_.PackageFullName }}) -join ', ')
	    @{{
	      success = $false
	      message = 'Remove-AppxPackage returned, but the current-user MSIX registration remains'
	      rawError = $null
	      notes = $script:notes
	    }} | ConvertTo-Json -Compress
	  }} else {{
	    Add-ResidualNotes
	    $successMessage = if ($script:preserveApplicationDataFallbackUsed) {{
	      'Remove-AppxPackage succeeded; package-local data was checked/backed up before removal (see notes)'
	    }} else {{
	      'Remove-AppxPackage succeeded'
	    }}
	    @{{
	      success = $true
	      message = $successMessage
	      rawError = $null
	      notes = $script:notes
	    }} | ConvertTo-Json -Compress
	  }}
	}} catch {{
	  $removalFailure = $_
	  # PowerShell ErrorDetails can contain the native HRESULT / ActivityId that
	  # Exception.Message alone omits. Preserve supplied diagnostics verbatim;
	  # a backup guard is not a Windows deployment error and has no invented code.
	  $rawDetails = @([string]$removalFailure, [string]$removalFailure.Exception.Message)
	  if ($removalFailure.ErrorDetails -and $removalFailure.ErrorDetails.Message) {{
	    $rawDetails += [string]$removalFailure.ErrorDetails.Message
	  }}
	  if ($removalFailure.FullyQualifiedErrorId) {{
	    $rawDetails += 'FullyQualifiedErrorId=' + [string]$removalFailure.FullyQualifiedErrorId
	  }}
	  if ($removalFailure.CategoryInfo) {{
	    $rawDetails += 'CategoryInfo=' + [string]$removalFailure.CategoryInfo
	  }}
	  @{{
	    success = $false
	    message = [string]$removalFailure.Exception.Message
	    rawError = (@($rawDetails | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }} | Select-Object -Unique) -join "`n")
	    notes = $script:notes
	  }} | ConvertTo-Json -Compress
	}}
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        preserve_compatibility_pattern = ps_quote(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN),
        data_backup_script = APPX_DATA_BACKUP_SCRIPT,
        preserve_application_data = if preserve_application_data {
            "$true"
        } else {
            "$false"
        },
    )
}

#[cfg(windows)]
pub fn remove_msix_package_with_options(
    preserve_application_data: bool,
) -> Result<MsixRemoveReport, EngineError> {
    remove_msix_package_with_volume(preserve_application_data, None, None)
}

#[cfg(windows)]
fn remove_msix_package_script_with_volume(
    preserve_application_data: bool,
    authorized_volume_root: Option<&Path>,
    authorized_volume_id: Option<&str>,
) -> String {
    let script = remove_msix_package_script(preserve_application_data);
    let root = authorized_volume_root
        .map(|root| normalize_win32_path_text(&root.to_string_lossy()))
        .unwrap_or_default();
    script
        .replace(
            "Backup-CodexPackageData $p",
            &format!(
                "Backup-CodexPackageData $p {} {}",
                ps_quote(&root),
                ps_quote(authorized_volume_id.unwrap_or(""))
            ),
        )
        .replace(
            "Assert-CodexBackupStillMatches $p $backupPath",
            &format!(
                "Assert-CodexBackupStillMatches $p $backupPath {} {}",
                ps_quote(&root),
                ps_quote(authorized_volume_id.unwrap_or(""))
            ),
        )
}

/// The optional USB identity is supplied only after the caller verifies the
/// bound device. Without both fields, external WpSystem links remain protected.
#[cfg(windows)]
pub fn remove_msix_package_with_volume(
    preserve_application_data: bool,
    authorized_volume_root: Option<&Path>,
    authorized_volume_id: Option<&str>,
) -> Result<MsixRemoveReport, EngineError> {
    if authorized_volume_root.is_some() != authorized_volume_id.is_some() {
        return Err(EngineError::Install(
            "USB data backup needs both the authorized root and volume GUID".into(),
        ));
    }
    let package = crate::OPENAI_PACKAGE_IDENTITY;
    log::info!(
        "remove MSIX package package={package} preserve_application_data={preserve_application_data}"
    );
    let script = remove_msix_package_script_with_volume(
        preserve_application_data,
        authorized_volume_root,
        authorized_volume_id,
    );
    // Hashing a recoverable data backup can exceed the ordinary deployment
    // budget. Still bounded; a timeout never deletes the partial backup.
    let limits = if preserve_application_data {
        RunLimits::usb_install()
    } else {
        RunLimits::install()
    };
    let json = run_powershell_json_with_limits(&script, limits).map_err(|e| {
        let backup_note = if preserve_application_data {
            "; backup files are retained under %LOCALAPPDATA%\\CodexAppxDataBackups. Registration outcome is unknown until re-queried"
        } else {
            ""
        };
        EngineError::Install(format!("run Remove-AppxPackage: {}{backup_note}", e.message()))
    })?;
    let report: MsixRemoveReport = serde_json::from_str(&json)
        .map_err(|e| EngineError::Install(format!("parse Remove-AppxPackage result: {e}")))?;
    if report.success {
        let package = crate::OPENAI_PACKAGE_IDENTITY;
        log::info!("remove MSIX package completed package={package}");
    } else {
        let package = crate::OPENAI_PACKAGE_IDENTITY;
        let error = &report.message;
        log::error!(
            "remove MSIX package failed package={package} error={error} raw_error={}",
            report.raw_error.as_deref().unwrap_or("")
        );
    }
    Ok(report)
}

#[cfg(any(windows, test))]
fn valid_exact_codex_package_full_name(value: &str) -> bool {
    let parts = value.split('_').collect::<Vec<_>>();
    parts.len() == 5
        && parts[0] == crate::OPENAI_PACKAGE_IDENTITY
        && parts[2].eq_ignore_ascii_case("x64")
        && parts[3].is_empty()
        && parts[4] == "2p2nqsd0c76g0"
        && parts[1].split('.').count() == 4
        && parts[1].split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u16>().is_ok()
        })
}

#[cfg(windows)]
fn msix_machine_state_script(expected_full_name: Option<&str>) -> Result<String, EngineError> {
    if let Some(expected) = expected_full_name {
        if !valid_exact_codex_package_full_name(expected) {
            return Err(EngineError::Install(format!(
                "invalid exact Codex package FullName for machine-state query: {expected}"
            )));
        }
    }
    let expected = expected_full_name
        .map(ps_quote)
        .unwrap_or_else(|| "''".into());
    let mut script = String::new();
    script.push_str("$ErrorActionPreference = 'Stop'\n");
    script.push_str(APPX_PACKAGE_PATH_SCRIPT);
    script.push('\n');
    script.push_str(APPX_MACHINE_CLEANUP_STATE_SCRIPT);
    script.push_str("\n$state = Get-CodexMachineCleanupState ");
    script.push_str(&expected);
    script.push_str("\n$state | ConvertTo-Json -Depth 6 -Compress\n");
    Ok(script)
}

/// Query current-user, all-user, provisioned, owner-state and package-path
/// projections without mutating AppX state. `orphan-candidate` is deliberately
/// only a signal to authenticate the offline payload; it is not authorization
/// to remove the observed FullName.
#[cfg(windows)]
pub fn probe_msix_machine_state() -> Result<MsixMachineStateReport, EngineError> {
    let script = msix_machine_state_script(None)?;
    let json = run_powershell_json_with_limits(&script, RunLimits::probe())
        .map_err(|error| error.into_install())?;
    serde_json::from_str(&json)
        .map_err(|error| EngineError::Install(format!("parse machine AppX state: {error}")))
}

#[cfg(windows)]
fn remove_exact_msix_machine_orphan_script(
    expected_full_name: &str,
    authorized_package_path: &Path,
    authorized_package_sha256: &str,
    preserve_application_data: bool,
    authorized_volume_root: Option<&Path>,
    authorized_volume_id: Option<&str>,
) -> Result<String, EngineError> {
    if !valid_exact_codex_package_full_name(expected_full_name) {
        return Err(EngineError::Install(format!(
            "invalid Authenticode-authorized Codex package FullName: {expected_full_name}"
        )));
    }
    if authorized_package_sha256.len() != 64
        || !authorized_package_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(EngineError::Install(
            "invalid Authenticode-authorized Codex payload SHA-256".into(),
        ));
    }
    if authorized_volume_root.is_some() != authorized_volume_id.is_some() {
        return Err(EngineError::Install(
            "machine AppX data backup needs both the authorized root and volume GUID".into(),
        ));
    }
    let root = authorized_volume_root
        .map(|root| normalize_win32_path_text(&root.to_string_lossy()))
        .unwrap_or_default();
    let package_path = normalize_win32_path_text(&authorized_package_path.to_string_lossy());
    let template = r#"
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
$expectedFullName = __EXPECTED_FULL_NAME__
$packagePath = __PACKAGE_PATH__
$packageSha256 = __PACKAGE_SHA256__
$preserveApplicationData = __PRESERVE_APPLICATION_DATA__
$authorizedVolumeRoot = __AUTHORIZED_VOLUME_ROOT__
$authorizedVolumeId = __AUTHORIZED_VOLUME_ID__
$script:notes = @()
$script:cmdletError = $null
$script:nativeError = $null
$script:cleanupApi = $null

function Invoke-CodexNativeMachineRemoval([string]$fullName) {
  Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
  $pm = [Windows.Management.Deployment.PackageManager,Windows,ContentType=WindowsRuntime]::new()
  $options = [Windows.Management.Deployment.RemovalOptions]::RemoveForAllUsers
  $async = $pm.RemovePackageAsync($fullName, $options)
  $asTaskMethod = [System.WindowsRuntimeSystemExtensions].GetMethods() |
    Where-Object {
      $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and
        $_.GetGenericArguments().Count -eq 2 -and $_.GetParameters().Count -eq 1
    } | Select-Object -First 1
  if ($null -eq $asTaskMethod) {
    throw 'WindowsRuntimeSystemExtensions.AsTask is unavailable for package removal'
  }
  $closedAsTask = $asTaskMethod.MakeGenericMethod(
    [Windows.Management.Deployment.DeploymentResult],
    [Windows.Management.Deployment.DeploymentProgress])
  $task = $closedAsTask.Invoke($null, @($async))
  $result = $task.GetAwaiter().GetResult()
  $extendedError = $null
  try {
    if ($result.ExtendedErrorCode) {
      $hr = [int]$result.ExtendedErrorCode.HResult
      if ($hr -ne 0) { $extendedError = ('0x{0:X8}' -f $hr) }
    }
  } catch {}
  if ($extendedError) {
    throw "PackageManager RemovePackageAsync failed hresult=$extendedError activityId=$([string]$result.ActivityId) errorText=$([string]$result.ErrorText)"
  }
  return @{activityId=[string]$result.ActivityId;errorText=[string]$result.ErrorText}
}

function Assert-CodexOrphanBackup($package, [string]$backupPath) {
  if (-not $preserveApplicationData) { return }
  if (-not [string]::IsNullOrWhiteSpace($backupPath)) {
    Assert-CodexBackupStillMatches $package $backupPath $authorizedVolumeRoot $authorizedVolumeId
  } else {
    Assert-CodexPackageDataAbsent
  }
}

function Wait-CodexMachineState([int]$attempts) {
  $state = $null
  for ($attempt = 0; $attempt -lt $attempts; $attempt++) {
    $state = Get-CodexMachineCleanupState $expectedFullName
    if ([string]$state.classification -notin @('orphan-candidate','staged-candidate')) { break }
    Start-Sleep -Milliseconds 250
  }
  return $state
}

function Assert-CodexSystemEntryGone {
  $audit = $null
  # AppX registration can disappear slightly before the deployment service
  # retires its Windows-managed system-store entry, especially on slow USB
  # media. Poll a bounded interval instead of manufacturing a false failure.
  for ($attempt = 0; $attempt -lt 40; $attempt++) {
    $audit = Get-CodexSystemPackageEntryAudit $expectedFullName
    if ([string]$audit.status -eq 'absent') { break }
    Start-Sleep -Milliseconds 250
  }
  if ([string]$audit.status -ne 'absent') {
    throw ([string]$audit.message + '; Windows-managed entry was retained and no ACL or private registry change was attempted')
  }
  $script:notes += [string]$audit.message
}

try {
  $before = Get-CodexMachineCleanupState $expectedFullName
  if ([string]$before.classification -eq 'current-user-present') {
    throw ('Current-user Codex registration still exists; ordinary per-user removal must finish first: ' + [string]$before.reason)
  }
  if ([string]$before.classification -eq 'blocked') {
    throw ('Machine Codex state is not an exclusive orphan and was retained: ' + [string]$before.reason)
  }
  if ([string]$before.classification -eq 'none') {
    # When a volume/recovery journal requested reconciliation, make one exact
    # supported WinRT call. It can retire a deployment-service record that the
    # PowerShell projection already considers absent. "package not found" is
    # harmless only if the fresh postcondition and system-entry audit are clean.
    try {
      $null = Invoke-CodexNativeMachineRemoval $expectedFullName
      $script:cleanupApi = 'PackageManager.RemovePackageAsync RemoveForAllUsers (absent-state reconciliation)'
    } catch {
      $script:nativeError = [string]$_
      $script:notes += 'Supported absent-state reconciliation reported: ' + $script:nativeError
    }
    $afterAbsent = Get-CodexMachineCleanupState $expectedFullName
    if ([string]$afterAbsent.classification -ne 'none') {
      throw ('Machine AppX state changed during absent-state reconciliation: ' + [string]$afterAbsent.reason)
    }
    Assert-CodexSystemEntryGone
    @{
      success = $true
      message = 'No machine Codex orphan remains; supported AppX reconciliation and postconditions completed'
      rawError = $null
      notes = $script:notes
    } | ConvertTo-Json -Depth 6 -Compress
    exit 0
  }
  if ([string]$before.classification -notin @('orphan-candidate','staged-candidate') -or
      [string]$before.packageFullName -ine $expectedFullName) {
    throw ('Machine Codex state is not the exact authorized orphan: ' + [string]$before.reason)
  }

  $packageDataIdentity = @{
    PackageFullName = $expectedFullName
    PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
  }
  $backupPath = $null
  if ($preserveApplicationData) {
    $backupPath = Backup-CodexPackageData $packageDataIdentity $authorizedVolumeRoot $authorizedVolumeId
    Assert-CodexOrphanBackup $packageDataIdentity $backupPath
    $script:notes += if ($backupPath) {
      "Orphan package data backed up and SHA256 verified at $backupPath; retain this directory for recovery."
    } else {
      'No package-local data exists for the exact orphan.'
    }
  }

  # Re-query after the data gate. No stale observation can authorize removing a
  # package that gained an installed owner or provisioned record meanwhile.
  $again = Get-CodexMachineCleanupState $expectedFullName
  if ([string]$again.classification -notin @('orphan-candidate','staged-candidate') -or
      [string]$again.packageFullName -ine $expectedFullName) {
    throw ('Machine orphan ownership changed before removal; nothing was removed: ' + [string]$again.reason)
  }
  Assert-CodexOrphanBackup $packageDataIdentity $backupPath
  $removeCmd = Get-Command Remove-AppxPackage -ErrorAction Stop
  if ($removeCmd.Parameters.Keys -notcontains 'AllUsers') {
    throw 'This Windows AppX module cannot remove an unowned package through the supported AllUsers API'
  }
  try {
    Remove-AppxPackage -Package $expectedFullName -AllUsers -ErrorAction Stop | Out-Null
    $script:cleanupApi = 'Remove-AppxPackage -AllUsers'
  } catch {
    $script:cmdletError = [string]$_
  }
  $after = Wait-CodexMachineState 8
  if ([string]$after.classification -in @('orphan-candidate','staged-candidate')) {
    Assert-CodexOrphanBackup $packageDataIdentity $backupPath
    $last = Get-CodexMachineCleanupState $expectedFullName
    if ([string]$last.classification -notin @('orphan-candidate','staged-candidate') -or
        [string]$last.packageFullName -ine $expectedFullName) {
      throw ('Machine orphan ownership changed before WinRT fallback; nothing else was removed: ' + [string]$last.reason)
    }
    try {
      $null = Invoke-CodexNativeMachineRemoval $expectedFullName
      $script:cleanupApi = 'PackageManager.RemovePackageAsync RemoveForAllUsers'
    } catch {
      $script:nativeError = [string]$_
    }
    $after = Wait-CodexMachineState 20
  }
  if ([string]$after.classification -in @('orphan-candidate','staged-candidate')) {
    # Both removal projections can fail with 0x80070002 when the deployment
    # record survived but its files did not. Rehydrate only from the exact
    # Authenticode-authorized MSIX, then retry the supported removal APIs.
    if ([string]::IsNullOrWhiteSpace($authorizedVolumeRoot) -or
        [string]::IsNullOrWhiteSpace($authorizedVolumeId)) {
      throw 'Exact-payload machine repair requires the verified product USB volume identity'
    }
    Assert-CodexOrphanBackup $packageDataIdentity $backupPath
    $volumeCommand = Get-Command Get-AppxVolume -ErrorAction Stop
    $onlineVolumes = if ($volumeCommand.Parameters.Keys -contains 'Online') {
      @(Get-AppxVolume -Online -ErrorAction Stop)
    } else {
      @(Get-AppxVolume -ErrorAction Stop | Where-Object { -not [bool]$_.IsOffline })
    }
    $authorizedStore = (Join-Path $authorizedVolumeRoot 'WindowsApps').TrimEnd('\')
    $usbVolumes = @($onlineVolumes | Where-Object {
      -not [bool]$_.IsOffline -and -not [bool]$_.IsSystemVolume -and
      ([string]$_.Name).TrimEnd('\') -ieq $authorizedVolumeId.TrimEnd('\') -and
      ([string]$_.PackageStorePath).TrimEnd('\') -ieq $authorizedStore
    })
    if ($usbVolumes.Count -ne 1) {
      throw 'Verified product USB AppX volume is absent or ambiguous during exact-payload repair'
    }
    $context = Get-CodexRecoveryContext $authorizedVolumeRoot $usbVolumes[0]
    $rehydrated = Invoke-CodexRehydrateMachinePackage $packagePath $packageSha256 $expectedFullName $context
    $script:notes += 'Restored missing package bytes through the public StagePackageAsync API: ' +
      ($rehydrated | ConvertTo-Json -Depth 5 -Compress)

    $restored = Get-CodexMachineCleanupState $expectedFullName
    if ([string]$restored.classification -notin @('staged-candidate','orphan-candidate') -or
        [string]$restored.packageFullName -ine $expectedFullName) {
      throw ('Exact-payload rehydration produced an unsafe machine state: ' + [string]$restored.reason)
    }
    Assert-CodexOrphanBackup $packageDataIdentity $backupPath
    try {
      Remove-AppxPackage -Package $expectedFullName -AllUsers -ErrorAction Stop | Out-Null
      $script:cleanupApi = 'StagePackageAsync rehydrate + Remove-AppxPackage -AllUsers'
    } catch { $script:cmdletError = ([string]$script:cmdletError + '; afterRehydrate=' + [string]$_) }
    $after = Wait-CodexMachineState 20
    if ([string]$after.classification -in @('orphan-candidate','staged-candidate')) {
      Assert-CodexOrphanBackup $packageDataIdentity $backupPath
      try {
        $null = Invoke-CodexNativeMachineRemoval $expectedFullName
        $script:cleanupApi = 'StagePackageAsync rehydrate + PackageManager.RemovePackageAsync RemoveForAllUsers'
      } catch { $script:nativeError = ([string]$script:nativeError + '; afterRehydrate=' + [string]$_) }
      $after = Wait-CodexMachineState 20
    }
    if ([string]$after.classification -in @('orphan-candidate','staged-candidate')) {
      # Some down-level builds only reconcile the staging record after it is
      # moved to the currently online product volume. Move remains a public,
      # exact-FullName operation; revalidate and remove once more afterwards.
      $moveError = $null
      try { $null = Invoke-CodexMoveRehydratedPackageToUsb $expectedFullName $context }
      catch { $moveError = [string]$_ }
      $afterMove = Get-CodexMachineCleanupState $expectedFullName
      if ([string]$afterMove.classification -eq 'staged-candidate' -and
          [string]$afterMove.packageFullName -ieq $expectedFullName) {
        Assert-CodexOrphanBackup $packageDataIdentity $backupPath
        try {
          Remove-AppxPackage -Package $expectedFullName -AllUsers -ErrorAction Stop | Out-Null
          $script:cleanupApi = 'StagePackageAsync rehydrate + MovePackageToVolumeAsync + Remove-AppxPackage -AllUsers'
        } catch { $script:cmdletError = ([string]$script:cmdletError + '; afterMove=' + [string]$_) }
        $after = Wait-CodexMachineState 20
      }
      if ([string]$after.classification -in @('orphan-candidate','staged-candidate')) {
        $script:notes += 'MovePackageToVolumeAsync recovery result: ' + [string]$moveError
      }
    }
  }
  if ([string]$after.classification -ne 'none') {
    throw ('Supported AppX orphan cleanup failed after exact-payload rehydration; private PackageRepository state was not edited; ' +
      'Remove-AppxPackage=' + [string]$script:cmdletError + '; PackageManager=' + [string]$script:nativeError +
      '; finalState=' + [string]$after.reason)
  }
  Assert-CodexSystemEntryGone
  $script:notes += 'Exact machine orphan removed with ' + [string]$script:cleanupApi
  @{
    success = $true
    message = 'Exact unowned Codex machine package state was removed through supported AppX APIs'
    rawError = $null
    notes = $script:notes
  } | ConvertTo-Json -Depth 6 -Compress
} catch {
  $failure = $_
  $raw = @([string]$failure, [string]$failure.Exception.Message)
  if ($failure.ErrorDetails -and $failure.ErrorDetails.Message) { $raw += [string]$failure.ErrorDetails.Message }
  if ($failure.FullyQualifiedErrorId) { $raw += 'FullyQualifiedErrorId=' + [string]$failure.FullyQualifiedErrorId }
  if ($script:cmdletError) { $raw += 'Remove-AppxPackage=' + [string]$script:cmdletError }
  if ($script:nativeError) { $raw += 'PackageManager=' + [string]$script:nativeError }
  @{
    success = $false
    message = [string]$failure.Exception.Message
    rawError = (@($raw | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique) -join "`n")
    notes = $script:notes
  } | ConvertTo-Json -Depth 6 -Compress
}
"#;
    let mut script = String::new();
    script.push_str(APPX_DATA_BACKUP_SCRIPT);
    script.push('\n');
    script.push_str(APPX_MACHINE_CLEANUP_STATE_SCRIPT);
    script.push('\n');
    script.push_str(APPX_MACHINE_REHYDRATION_SCRIPT);
    script.push_str(
        &template
            .replace("__EXPECTED_FULL_NAME__", &ps_quote(expected_full_name))
            .replace("__PACKAGE_PATH__", &ps_quote(&package_path))
            .replace(
                "__PACKAGE_SHA256__",
                &ps_quote(&authorized_package_sha256.to_ascii_lowercase()),
            )
            .replace(
                "__PRESERVE_APPLICATION_DATA__",
                if preserve_application_data {
                    "$true"
                } else {
                    "$false"
                },
            )
            .replace("__AUTHORIZED_VOLUME_ROOT__", &ps_quote(&root))
            .replace(
                "__AUTHORIZED_VOLUME_ID__",
                &ps_quote(authorized_volume_id.unwrap_or("")),
            ),
    );
    Ok(script)
}

/// Remove one exact, unowned and unprovisioned machine-level Codex orphan. The
/// exact FullName must come from the separately Authenticode-verified offline
/// payload. Both public AppX removal APIs are revalidated around every attempt;
/// WindowsApps ACLs and the private PackageRepository are never modified.
#[cfg(windows)]
pub fn remove_exact_msix_machine_orphan_with_volume(
    expected_full_name: &str,
    authorized_package_path: &Path,
    authorized_package_sha256: &str,
    preserve_application_data: bool,
    authorized_volume_root: Option<&Path>,
    authorized_volume_id: Option<&str>,
) -> Result<MsixRemoveReport, EngineError> {
    let script = remove_exact_msix_machine_orphan_script(
        expected_full_name,
        authorized_package_path,
        authorized_package_sha256,
        preserve_application_data,
        authorized_volume_root,
        authorized_volume_id,
    )?;
    log::info!(
        "machine AppX orphan cleanup start package={} preserve_application_data={}",
        expected_full_name,
        preserve_application_data
    );
    let json =
        run_powershell_json_with_limits(&script, RunLimits::usb_install()).map_err(|error| {
            EngineError::Install(format!("run machine AppX cleanup: {}", error.message()))
        })?;
    let report: MsixRemoveReport = serde_json::from_str(&json)
        .map_err(|error| EngineError::Install(format!("parse machine AppX cleanup: {error}")))?;
    if report.success {
        log::info!("machine AppX orphan cleanup completed package={expected_full_name}");
    } else {
        log::error!(
            "machine AppX orphan cleanup failed package={} error={} raw_error={}",
            expected_full_name,
            report.message,
            report.raw_error.as_deref().unwrap_or("")
        );
    }
    Ok(report)
}

#[cfg(not(windows))]
pub fn remove_msix_package() -> Result<MsixRemoveReport, EngineError> {
    remove_msix_package_with_options(false)
}

#[cfg(not(windows))]
pub fn remove_msix_package_with_options(
    _preserve_application_data: bool,
) -> Result<MsixRemoveReport, EngineError> {
    let package = crate::OPENAI_PACKAGE_IDENTITY;
    log::info!("remove MSIX package package={package}");
    Ok(MsixRemoveReport {
        success: false,
        message: "MSIX removal is only available on Windows".to_string(),
        raw_error: None,
        notes: vec![],
    })
}

#[cfg(not(windows))]
pub fn remove_msix_package_with_volume(
    preserve_application_data: bool,
    _authorized_volume_root: Option<&Path>,
    _authorized_volume_id: Option<&str>,
) -> Result<MsixRemoveReport, EngineError> {
    remove_msix_package_with_options(preserve_application_data)
}

#[cfg(not(windows))]
pub fn probe_msix_machine_state() -> Result<MsixMachineStateReport, EngineError> {
    Ok(MsixMachineStateReport {
        classification: msix_machine_state::NONE.to_string(),
        reason: "machine AppX state is only available on Windows".to_string(),
        package_full_name: None,
        install_location: None,
        owner_states: vec![],
        provisioned_packages: vec![],
        path_state: None,
        record_volume_name: None,
        record_volume_store: None,
    })
}

#[cfg(not(windows))]
pub fn remove_exact_msix_machine_orphan_with_volume(
    _expected_full_name: &str,
    _authorized_package_path: &Path,
    _authorized_package_sha256: &str,
    _preserve_application_data: bool,
    _authorized_volume_root: Option<&Path>,
    _authorized_volume_id: Option<&str>,
) -> Result<MsixRemoveReport, EngineError> {
    Ok(MsixRemoveReport {
        success: false,
        message: "machine AppX cleanup is only available on Windows".to_string(),
        raw_error: None,
        notes: vec![],
    })
}

/// Best-effort: close Codex processes belonging to the registered OpenAI.Codex
/// MSIX package (by InstallLocation). Used after an activation probe that may
/// have started the app, and before portable fallback / Remove-AppxPackage.
/// Failures are logged and swallowed — cleanup must not block fallback.
#[cfg(windows)]
pub fn close_msix_codex_processes(timeout_secs: u64) -> Result<(), EngineError> {
    let script = format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
$pkg = Get-AppxPackage -Name {name} |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if ($null -eq $pkg) {{ 'no-package'; exit 0 }}
$loc = [string]$pkg.InstallLocation
if ([string]::IsNullOrWhiteSpace($loc)) {{ 'no-location'; exit 0 }}
try {{ $loc = ([string](Convert-Path -LiteralPath $loc -ErrorAction Stop)).TrimEnd('\') }} catch {{}}
Write-Output $loc
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY)
    );
    let loc = match run_powershell_json_with_limits(&script, RunLimits::probe()) {
        Ok(s) => s.trim().to_string(),
        Err(err) => {
            log::warn!(
                "close MSIX Codex processes: could not resolve install location error={}",
                err.message()
            );
            return Ok(());
        }
    };
    if loc.is_empty() || loc == "no-package" || loc == "no-location" {
        return Ok(());
    }
    crate::portable::close_codex_gracefully_for_root(timeout_secs, Path::new(&loc))
}

#[cfg(not(windows))]
pub fn close_msix_codex_processes(_timeout_secs: u64) -> Result<(), EngineError> {
    Ok(())
}

#[cfg(windows)]
fn best_effort_close_msix_after_probe() {
    // Start-Process detaches from the PowerShell tree, so killing the probe on
    // timeout leaves Codex running. Always try to reap it before the caller
    // falls back / removes the package.
    if let Err(err) = close_msix_codex_processes(15) {
        log::warn!("best-effort MSIX process cleanup after probe failed error={err}");
    }
}

#[cfg(windows)]
/// Build the PowerShell activation fragment used by both the post-install
/// health probe and the normal launch path.
///
/// `Start-Process 'shell:AppsFolder\\...'` treats the shell target as the
/// process executable. On some Windows PowerShell/.NET combinations that
/// invokes URI parsing and fails with "Invalid URI: The hostname could not be
/// parsed". Passing the target as an argument to the real Explorer executable
/// keeps shell URI resolution in Explorer and avoids that parser altogether.
fn powershell_explorer_activation(target_expression: &str) -> String {
    format!(
        r#"$shellTarget = "shell:AppsFolder\" + {target_expression}
$explorer = "$env:WINDIR\explorer.exe"
if (-not (Test-Path -LiteralPath $explorer -PathType Leaf)) {{
  $explorer = 'explorer.exe'
}}
Start-Process -FilePath $explorer -ArgumentList @($shellTarget) -ErrorAction Stop | Out-Null"#,
        target_expression = target_expression
    )
}

pub fn verify_msix_health() -> MsixHealthReport {
    verify_msix_health_with_options(false)
}

#[cfg(windows)]
fn msix_health_script(keep_running: bool) -> String {
    format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
{package_path_script}
$activationWindowSecs = {activation_window}
$livenessWindowSecs = {liveness_window}
$pkg = Get-AppxPackage -Name {name} |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if ($null -eq $pkg) {{
  @{{
    packageRegistered = $false
    statusOk = $false
    status = 'not-registered'
    aumidResolved = $false
    missingDependencies = ''
    activationOk = $false
    failureKind = 'not-registered'
    activationDetail = ''
  }} | ConvertTo-Json -Compress
  exit 0
}}
$statusStr = [string]$pkg.Status
$statusOk = ([string]::IsNullOrEmpty($statusStr) -or $statusStr -eq 'Ok')
$aumidResolved = $false
$missing = @()
$activationOk = $false
$failureKind = ''
$activationDetail = ''
$appId = ''
$installLoc = ''
$targetIds = @()
$activationAttempted = $false
$keepRunning = {keep_running}
$script:packageProcessRoots = @()
function Convert-ToVersion($value) {{
  try {{
    $text = [string]$value
    if ([string]::IsNullOrWhiteSpace($text)) {{ return $null }}
    return [version]$text
  }} catch {{
    return $null
  }}
}}
function Same-Publisher($package, [string]$publisher) {{
  if ([string]::IsNullOrWhiteSpace($publisher)) {{ return $true }}
  return [string]$package.Publisher -eq $publisher
}}
function Same-Architecture($package, [string]$required) {{
  if ([string]::IsNullOrWhiteSpace($required) -or $required -eq 'neutral') {{ return $true }}
  $arch = [string]$package.Architecture
  return [string]::IsNullOrWhiteSpace($arch) -or $arch -eq 'Neutral' -or $arch -eq $required
}}
# AppX / protected processes often leave Get-Process.Path empty. Fall through
# MainModule and CIM so a live Codex is not treated as activation-failed.
function Get-ProcessExePath($p) {{
  try {{
    $path = [string]$p.Path
    if (-not [string]::IsNullOrWhiteSpace($path)) {{ return $path }}
  }} catch {{}}
  try {{
    $path = [string]$p.MainModule.FileName
    if (-not [string]::IsNullOrWhiteSpace($path)) {{ return $path }}
  }} catch {{}}
  try {{
    $cim = Get-CimInstance -ClassName Win32_Process -Filter ("ProcessId=" + $p.Id) -ErrorAction SilentlyContinue
    if ($null -ne $cim -and -not [string]::IsNullOrWhiteSpace([string]$cim.ExecutablePath)) {{
      return [string]$cim.ExecutablePath
    }}
  }} catch {{}}
  return $null
}}
function Test-UnderInstall($p, [string]$root) {{
  if ([string]::IsNullOrWhiteSpace($root)) {{ return $false }}
  $path = Get-ProcessExePath $p
  if ([string]::IsNullOrWhiteSpace($path)) {{ return $false }}
  $full = [string]$path
  try {{
    $resolved = [string](Convert-Path -LiteralPath $path -ErrorAction Stop)
    if (-not [string]::IsNullOrWhiteSpace($resolved)) {{ $full = $resolved }}
  }} catch {{}}
  return Test-CodexProcessPathUnderRoots $full $script:packageProcessRoots
}}
function Get-PackageProcesses([string]$root) {{
  $found = @()
  foreach ($p in @(Get-Process -Name Codex,ChatGPT -ErrorAction SilentlyContinue)) {{
    if (Test-UnderInstall $p $root) {{ $found += $p }}
  }}
  return $found
}}
function Stop-PackageProcesses([string]$root, $ids) {{
  $pendingIds = @($ids)
  $deadline = (Get-Date).AddSeconds(5)
  do {{
    foreach ($id in $pendingIds) {{
      try {{ Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }} catch {{}}
    }}
    foreach ($p in @(Get-PackageProcesses $root)) {{
      try {{ Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }} catch {{}}
    }}
    Start-Sleep -Milliseconds 200
    $remaining = @(Get-PackageProcesses $root)
    if ($remaining.Count -eq 0) {{ return $true }}
    # Electron may replace a process while shutting down. Follow the replacement
    # PIDs and retry until the bounded cleanup deadline.
    $pendingIds = @($remaining | ForEach-Object {{ $_.Id }} | Select-Object -Unique)
  }} while ((Get-Date) -lt $deadline)
  return (@(Get-PackageProcesses $root).Count -eq 0)
}}
try {{
  $manifest = Get-AppxPackageManifest $pkg -ErrorAction Stop
  $app = $manifest.Package.Applications.Application
  if ($app -is [array]) {{ $app = $app[0] }}
  $appId = [string]$app.Id
  if (-not [string]::IsNullOrEmpty($appId)) {{ $aumidResolved = $true }}
  $mainArch = [string]$pkg.Architecture
  $deps = $manifest.Package.Dependencies.PackageDependency
  foreach ($d in @($deps)) {{
    if ($null -ne $d) {{
      $dn = [string]$d.Name
      if (-not [string]::IsNullOrEmpty($dn)) {{
        $depPublisher = [string]$d.Publisher
        $depMinText = [string]$d.MinVersion
        $depMin = Convert-ToVersion $depMinText
        $depArch = [string]$d.ProcessorArchitecture
        if ([string]::IsNullOrWhiteSpace($depArch)) {{ $depArch = $mainArch }}
        $candidates = @(Get-AppxPackage -Name $dn -ErrorAction SilentlyContinue |
          Where-Object {{ Same-Publisher $_ $depPublisher }})
        $archCandidates = @($candidates | Where-Object {{ Same-Architecture $_ $depArch }})
        if ($candidates.Count -gt 0 -and $archCandidates.Count -eq 0) {{
          $missing += "$dn architecture $depArch not installed"
          continue
        }}
        $depPkg = $archCandidates |
          Sort-Object -Property @{{ Expression = {{ Convert-ToVersion $_.Version }}; Descending = $true }} |
          Select-Object -First 1
        if ($null -eq $depPkg) {{
          $missing += "$dn not installed"
        }} else {{
          $installedVersion = Convert-ToVersion $depPkg.Version
          if ($null -ne $depMin -and $null -ne $installedVersion -and $installedVersion -lt $depMin) {{
            $missing += "$dn >= $depMinText required (installed $($depPkg.Version))"
          }}
        }}
      }}
    }}
  }}
}} catch {{}}

# Real activation: ask Explorer to shell-start the AUMID and require a process
# under InstallLocation for a continuous liveness window aligned with portable.
# Do not pass the `shell:AppsFolder\...` value as Start-Process' FilePath: on
# some Windows PowerShell/.NET builds that is parsed as a URI and raises
# "Invalid URI: The hostname could not be parsed" before Explorer gets a chance
# to resolve the AppX identity. Keeping the URI as an argument to the real
# Explorer executable works on both the desktop shell and stripped machines.
if ($statusOk -and $aumidResolved -and $missing.Count -eq 0) {{
  if ([string]::IsNullOrEmpty($appId)) {{ $appId = 'App' }}
  $aumid = [string]$pkg.PackageFamilyName + '!' + $appId
  $installLoc = [string]$pkg.InstallLocation
  try {{
    $script:packageProcessRoots = @(Get-CodexPackageProcessRoots $installLoc ([string]$pkg.PackageFullName))
    $activationAttempted = $true
    {explorer_activation}
    $deadline = (Get-Date).AddSeconds($activationWindowSecs)
    $sawProcess = $false
    $stillAlive = $false
    while ((Get-Date) -lt $deadline) {{
      Start-Sleep -Milliseconds 300
      $found = @(Get-PackageProcesses $installLoc)
      if ($found.Count -eq 0) {{ continue }}
      $sawProcess = $true
      $targetIds = @($found | ForEach-Object {{ $_.Id }} | Select-Object -Unique)
      # Continuous survival for $livenessWindowSecs (same bar as portable).
      $liveDeadline = (Get-Date).AddSeconds($livenessWindowSecs)
      $continuous = $true
      while ((Get-Date) -lt $liveDeadline) {{
        Start-Sleep -Milliseconds 250
        $alive = @()
        foreach ($id in $targetIds) {{
          $p = Get-Process -Id $id -ErrorAction SilentlyContinue
          if ($null -ne $p) {{ $alive += $p }}
        }}
        # Also accept replacements under install root (Electron restarts).
        if ($alive.Count -eq 0) {{
          $alive = @(Get-PackageProcesses $installLoc)
          $targetIds = @($alive | ForEach-Object {{ $_.Id }} | Select-Object -Unique)
        }}
        if ($alive.Count -eq 0) {{
          $continuous = $false
          break
        }}
      }}
      if ($continuous) {{
        $stillAlive = $true
        break
      }}
    }}
    if ($stillAlive) {{
      $activationOk = $true
      if (-not $keepRunning) {{
        if (Stop-PackageProcesses $installLoc $targetIds) {{
          $targetIds = @()
        }} else {{
          # A health check is not successful if it changes a previously-closed
          # app into a running one. Fail closed so the caller can report/fallback.
          $activationOk = $false
          $failureKind = 'cleanup-failed'
          $activationDetail = 'package process remained running after health-check cleanup'
        }}
      }}
    }} elseif ($sawProcess) {{
      $failureKind = 'immediate-exit'
      $activationDetail = 'package process started then exited during the liveness window'
    }} else {{
      $failureKind = 'activation-failed'
      $activationDetail = 'no process under install location after shell activation'
    }}
  }} catch {{
    $msg = [string]$_.Exception.Message
    $activationDetail = $msg
    if ($msg -match '0x80073|policy|denied|Access is denied|0x80070005|blocked') {{
      $failureKind = 'policy'
    }} else {{
      $failureKind = 'activation-failed'
    }}
  }}
  # Unhealthy activation must not leave Codex holding package files open for
  # the subsequent portable fallback / Remove-AppxPackage.
  if (-not $activationOk -and $activationAttempted) {{
    $null = Stop-PackageProcesses $installLoc $targetIds
  }}
}}

@{{
  packageRegistered = $true
  statusOk = $statusOk
  status = $statusStr
  aumidResolved = $aumidResolved
  missingDependencies = ($missing -join ', ')
  activationOk = $activationOk
  failureKind = $failureKind
  activationDetail = $activationDetail
}} | ConvertTo-Json -Compress
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        activation_window = MSIX_ACTIVATION_WINDOW_SECS,
        liveness_window = MSIX_LIVENESS_WINDOW_SECS,
        keep_running = if keep_running { "$true" } else { "$false" },
        package_path_script = APPX_PACKAGE_PATH_SCRIPT,
        explorer_activation = powershell_explorer_activation("$aumid")
    )
}

#[cfg(windows)]
pub fn verify_msix_health_with_options(keep_running: bool) -> MsixHealthReport {
    log::info!("MSIX health check start");
    // Budget deps probe + cold-start window + continuous liveness + cleanup.
    let limits = RunLimits::total(std::time::Duration::from_secs(
        60 + MSIX_ACTIVATION_WINDOW_SECS + MSIX_LIVENESS_WINDOW_SECS + 30,
    ));
    let script = msix_health_script(keep_running);
    let run_result = run_powershell_json_with_limits(&script, limits);
    let parsed = match &run_result {
        Ok(json) => serde_json::from_str::<serde_json::Value>(json).ok(),
        Err(PowerShellRunError::Timeout(kind)) => {
            log::info!("MSIX health check result healthy=false status=timeout kind={kind:?}");
            // Start-Process detaches; kill any process the timed-out probe left running.
            best_effort_close_msix_after_probe();
            return MsixHealthReport {
                healthy: false,
                verified: true,
                package_registered: true,
                status: "timeout".to_string(),
                status_ok: false,
                aumid_resolved: false,
                missing_dependencies: vec![],
                activation_ok: false,
                failure_kind: msix_failure::TIMEOUT.to_string(),
                reason: "health probe timed out; routed to portable fallback".to_string(),
            };
        }
        Err(_) => None,
    };

    let Some(value) = parsed else {
        // The health probe itself could not run. On managed/stripped Windows this
        // is exactly the situation where an MSIX can register but fail to launch,
        // so treat the verdict as degraded and let the caller fall back to the
        // portable build instead of silently keeping an unverifiable package.
        log::info!("MSIX health check result healthy=false status=probe-failed");
        best_effort_close_msix_after_probe();
        return MsixHealthReport {
            healthy: false,
            verified: false,
            package_registered: true,
            status: "probe-failed".to_string(),
            status_ok: false,
            aumid_resolved: false,
            missing_dependencies: vec![],
            activation_ok: false,
            failure_kind: msix_failure::PROBE_FAILED.to_string(),
            reason: "health probe could not run; routed to portable fallback".to_string(),
        };
    };

    let package_registered = value
        .get("packageRegistered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let status_ok = value
        .get("statusOk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let aumid_resolved = value
        .get("aumidResolved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let missing_text = value
        .get("missingDependencies")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let missing_dependencies: Vec<String> = missing_text
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let activation_ok = value
        .get("activationOk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let activation_detail = value
        .get("activationDetail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reported_kind = value
        .get("failureKind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let (healthy, failure_kind, reason) = if !package_registered {
        (
            false,
            msix_failure::NOT_REGISTERED.to_string(),
            "the package is not registered after install".to_string(),
        )
    } else if !status_ok {
        (
            false,
            msix_failure::STATUS_BAD.to_string(),
            format!("package status is {status}"),
        )
    } else if !aumid_resolved {
        (
            false,
            msix_failure::AUMID_UNRESOLVED.to_string(),
            "could not resolve the app entry (AUMID)".to_string(),
        )
    } else if !missing_dependencies.is_empty() {
        (
            false,
            msix_failure::MISSING_DEPENDENCIES.to_string(),
            format!(
                "missing framework dependencies: {}",
                missing_dependencies.join(", ")
            ),
        )
    } else if !activation_ok {
        let kind = if reported_kind.is_empty() {
            msix_failure::ACTIVATION_FAILED.to_string()
        } else {
            reported_kind
        };
        let reason = if activation_detail.is_empty() {
            "package failed real activation / liveness check".to_string()
        } else {
            format!("package activation failed: {activation_detail}")
        };
        (false, kind, reason)
    } else {
        (true, String::new(), String::new())
    };

    let report = MsixHealthReport {
        healthy,
        // The probe ran and produced a real verdict.
        verified: true,
        package_registered,
        status,
        status_ok,
        aumid_resolved,
        missing_dependencies,
        activation_ok,
        failure_kind,
        reason,
    };
    let healthy = report.healthy;
    let status = &report.status;
    let kind = &report.failure_kind;
    log::info!("MSIX health check result healthy={healthy} status={status} failure_kind={kind}");
    report
}

#[cfg(not(windows))]
pub fn verify_msix_health_with_options(_keep_running: bool) -> MsixHealthReport {
    // Non-Windows builds never sideload, so there is nothing to verify; report
    // healthy so this can never be the thing that blocks a (non-existent) path,
    // but leave verified = false since no real check was performed.
    log::info!("MSIX health check start");
    log::info!("MSIX health check result healthy=true status=not-windows");
    MsixHealthReport {
        healthy: true,
        verified: false,
        package_registered: false,
        status: "not-windows".to_string(),
        status_ok: true,
        aumid_resolved: true,
        missing_dependencies: vec![],
        activation_ok: true,
        failure_kind: String::new(),
        reason: "MSIX health checks are only meaningful on Windows".to_string(),
    }
}

#[cfg(windows)]
fn detect_msix_script() -> String {
    format!(
        r#"
{package_path_script}
$p = Get-AppxPackage -Name {name} -ErrorAction SilentlyContinue |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if ($null -ne $p) {{
  {installed_msix_object} | ConvertTo-Json -Compress
}}
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        package_path_script = APPX_PACKAGE_PATH_SCRIPT,
        installed_msix_object = INSTALLED_MSIX_OBJECT,
    )
}

#[cfg(windows)]
fn detect_msix_install() -> Option<InstalledWindowsCodex> {
    let script = detect_msix_script();
    let json = run_powershell_json(&script).ok()?;
    if json.trim().is_empty() {
        return None;
    }
    let mut codex: InstalledWindowsCodex = serde_json::from_str(&json).ok()?;
    codex.installed_at = path_mtime_secs(&codex.path);
    Some(prefer_codex_app_version(codex))
}

#[cfg(not(windows))]
fn detect_msix_install() -> Option<InstalledWindowsCodex> {
    None
}

pub fn detect_portable_install(portable_root: &Path) -> Option<InstalledWindowsCodex> {
    // Entry-exe aware (manifest-declared, ChatGPT.exe/Codex.exe fallback) so
    // both pre- and post-rebrand portable payloads are recognized.
    let exe = crate::portable::installed_app_exe(portable_root)?;
    // Identity gate: an entry executable alone is not product identity — a
    // ChatGPT Classic payload also ships a root-level ChatGPT.exe.
    //   - With a manifest present (our installer always writes one), it must
    //     declare exactly the Codex package identity; a foreign or unparseable
    //     manifest is never a Codex install.
    //   - Without a manifest (a user-unpacked `app/` dir or a legacy adopted
    //     root), fall back to the payload's package-level identity: the
    //     `name` in app.asar's package.json. MSIX-internal executables carry
    //     no embedded Authenticode (integrity lives in the package-level
    //     AppxSignature.p7x, which does not survive unpacking), so signature
    //     checks cannot recognize these roots — the asar marker can.
    let identity = match std::fs::read_to_string(portable_root.join("AppxManifest.xml")) {
        Ok(xml) => match parse_appx_manifest_xml(&xml) {
            Ok(identity) if identity.name == crate::OPENAI_PACKAGE_IDENTITY => Some(identity),
            Ok(identity) => {
                log::debug!(
                    "portable root at {} declares identity {} (expected {}); not a Codex install",
                    portable_root.display(),
                    identity.name,
                    crate::OPENAI_PACKAGE_IDENTITY
                );
                return None;
            }
            Err(err) => {
                log::debug!(
                    "portable root at {} has an unparseable AppxManifest.xml ({err}); not a Codex install",
                    portable_root.display()
                );
                return None;
            }
        },
        Err(_) => {
            let asar_name =
                crate::app_version::read_asar_package_name_from_install_root(portable_root);
            if asar_name.as_deref() != Some(crate::app_version::CODEX_ASAR_PACKAGE_NAME) {
                log::debug!(
                    "portable root at {} has no manifest and its app payload name is {:?} (expected {}); not a Codex install",
                    portable_root.display(),
                    asar_name,
                    crate::app_version::CODEX_ASAR_PACKAGE_NAME
                );
                return None;
            }
            None
        }
    };

    let installed = InstalledWindowsCodex {
        path: portable_root.to_string_lossy().into_owned(),
        version: identity
            .as_ref()
            .map(|identity| identity.version.clone())
            .unwrap_or_else(|| "0.0.0.0".to_string()),
        arch: identity
            .as_ref()
            .map(|identity| identity.processor_architecture.clone()),
        source: "portable".to_string(),
        package_family_name: None,
        package_volume: None,
        installed_at: path_mtime_secs(&exe.to_string_lossy()),
    };
    let installed = prefer_codex_app_version(installed);
    let path = &installed.path;
    log::debug!("detected portable install path={path}");
    Some(installed)
}

/// Open the installed Codex: run the portable `Codex.exe`, or hand the MSIX
/// app's resolved AUMID to the Windows shell.
pub fn launch_codex(installed: &InstalledWindowsCodex) -> Result<(), EngineError> {
    launch_codex_with_options(installed, LaunchOptions::default()).map(|_| ())
}

#[cfg(windows)]
fn portable_compatibility_arguments(
    software_rendering: bool,
    no_sandbox: bool,
) -> Vec<&'static str> {
    let mut arguments = Vec::with_capacity(2);
    if software_rendering {
        arguments.push("--disable-gpu");
    }
    if no_sandbox {
        arguments.push("--no-sandbox");
    }
    arguments
}

#[cfg(windows)]
fn portable_launch_mode(software_rendering: bool, no_sandbox: bool) -> &'static str {
    match (software_rendering, no_sandbox) {
        (false, false) => "portable",
        (false, true) => "portable-no-sandbox",
        (true, false) => "software-rendering",
        (true, true) => "software-rendering-no-sandbox",
    }
}

#[cfg(windows)]
fn configure_portable_launch_command(
    exe: &Path,
    options: &LaunchOptions,
    software_rendering: bool,
    no_sandbox: bool,
) -> std::process::Command {
    let mut command = hidden_command(exe);
    for (name, value) in portable_launch_environment_overrides(options) {
        command.env(name, value);
    }
    command.args(portable_launch_arguments(
        options,
        software_rendering,
        no_sandbox,
    ));
    command
}

#[cfg(windows)]
fn schedule_console_window_suppression(root: &Path) {
    // Fallback for systems where the WinEvent hook cannot be installed. Start
    // before CreateProcess and keep watching through startup and process exit.
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let lifetime = std::time::Duration::from_secs(60);
        let mut root_seen = false;
        let mut absent_since = None;
        loop {
            // This is a bounded startup guard.  The portable PE/ASAR edits
            // handle future child creation, so do not retain a polling thread
            // for the full lifetime of a healthy Codex process.
            if started.elapsed() >= lifetime {
                break;
            }
            match crate::windows_process::hide_console_windows_for_root(&root) {
                Ok(hidden) if hidden > 0 => {
                    log::debug!("managed Codex console fallback hidden_candidates={hidden}")
                }
                Err(error) => log::warn!("managed Codex console fallback failed: {error}"),
                _ => {}
            }
            match crate::windows_process::hide_codex_auxiliary_windows_for_root(&root) {
                Ok(hidden) if hidden > 0 => {
                    log::debug!(
                        "managed Codex auxiliary window fallback hidden_candidates={hidden}"
                    )
                }
                Err(error) => log::warn!("managed Codex auxiliary window fallback failed: {error}"),
                _ => {}
            }
            match crate::windows_process::processes_running_for_root(&root) {
                Ok(true) => {
                    root_seen = true;
                    absent_since = None;
                }
                Ok(false) if root_seen => {
                    let absent = absent_since.get_or_insert_with(std::time::Instant::now);
                    if absent.elapsed() >= std::time::Duration::from_secs(3) {
                        break;
                    }
                }
                Ok(false) if started.elapsed() >= std::time::Duration::from_secs(45) => break,
                Err(error) => {
                    log::debug!("managed Codex console fallback process scan unavailable: {error}")
                }
                _ => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        log::info!("managed Codex console fallback stopped root_seen={root_seen}");
    });
}

#[cfg(windows)]
fn cleanup_failed_portable_launch(root: &Path, launch_error: EngineError) -> EngineError {
    // A startup failure dialog is reported from inside the readiness probe.
    // That probe returns early, so keep the same bounded process-tree cleanup
    // used by the timeout path before a no-sandbox retry can begin. Otherwise
    // the first broken Electron tree would make the retry observe the stale
    // dialog and fail for the wrong reason.
    match crate::windows_process::processes_running_for_root(root) {
        Ok(true) => match crate::windows_process::close_processes_for_root(5, root) {
            Ok(()) => launch_error,
            Err(cleanup_error) => EngineError::Io(format!(
                "{launch_error}; 清理失败启动进程时出错：{cleanup_error}"
            )),
        },
        Ok(false) => launch_error,
        Err(check_error) => EngineError::Io(format!(
            "{launch_error}; 无法确认失败启动进程是否已退出：{check_error}"
        )),
    }
}

#[cfg(any(windows, test))]
#[derive(Debug)]
enum PortableLaunchFailure {
    /// The process started and diagnostics succeeded, but no stable usable
    /// Codex window appeared. This is the only condition eligible for a
    /// software-rendering compatibility retry.
    NoUsableWindow(String),
    /// Setup, process creation, or diagnostics failed. Retrying with renderer
    /// flags cannot repair these failures and may obscure the original cause.
    Fatal(EngineError),
}

#[cfg(any(windows, test))]
impl PortableLaunchFailure {
    fn should_retry_compatibility(&self) -> bool {
        matches!(self, Self::NoUsableWindow(_))
    }

    fn into_engine_error(self) -> EngineError {
        match self {
            Self::NoUsableWindow(message) => EngineError::Io(message),
            Self::Fatal(error) => error,
        }
    }
}

#[cfg(any(windows, test))]
impl std::fmt::Display for PortableLaunchFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoUsableWindow(message) => formatter.write_str(message),
            Self::Fatal(error) => write!(formatter, "{error}"),
        }
    }
}

#[cfg(windows)]
fn launch_portable_with_usable_window(
    root: &Path,
    exe: &Path,
    options: &LaunchOptions,
    software_rendering: bool,
    no_sandbox: bool,
) -> Result<CodexLaunchReport, PortableLaunchFailure> {
    const WINDOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    const STABILITY_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

    if let Err(error) = crate::windows_process::start_console_window_watcher(root) {
        log::warn!(
            "prelaunch Codex console event hook unavailable; using process-tree polling: {error}"
        );
        schedule_console_window_suppression(root);
    }
    let mut command =
        configure_portable_launch_command(exe, options, software_rendering, no_sandbox);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|error| {
        PortableLaunchFailure::Fatal(EngineError::Io(format!("launch Codex: {error}")))
    })?;

    let first = match crate::windows_process::wait_for_usable_window_for_root(root, WINDOW_TIMEOUT)
    {
        Ok(report) => report,
        Err(error) => {
            return Err(PortableLaunchFailure::Fatal(
                cleanup_failed_portable_launch(root, error),
            ))
        }
    };
    if let Some(window) = first {
        // A transient HWND is not a successful launch. Re-check after a short
        // stability window before reporting that Codex is ready.
        std::thread::sleep(STABILITY_WINDOW);
        let stable = match crate::windows_process::wait_for_usable_window_for_root(
            root,
            std::time::Duration::ZERO,
        ) {
            Ok(report) => report,
            Err(error) => {
                return Err(PortableLaunchFailure::Fatal(
                    cleanup_failed_portable_launch(root, error),
                ))
            }
        };
        if let Some(stable) = stable {
            if child.try_wait().ok().flatten().is_none() {
                std::mem::forget(child);
            }
            let launch_mode = portable_launch_mode(software_rendering, no_sandbox);
            log::info!(
                "portable Codex usable window confirmed mode={launch_mode} pid={} size={}x{} initially_visible={} foreground_accepted={}",
                stable.pid,
                stable.width,
                stable.height,
                window.was_visible,
                stable.foreground_accepted
            );
            return Ok(CodexLaunchReport {
                launch_mode: launch_mode.to_string(),
                usable_window_confirmed: true,
                no_sandbox,
                message: match (software_rendering, no_sandbox) {
                    (false, false) => "Codex 已按常规渲染模式启动。".to_string(),
                    (false, true) => {
                        "Codex 已按常规渲染模式启动；此设备需要关闭 Chromium 沙箱。".to_string()
                    }
                    (true, false) => "Codex 已用软件渲染兼容模式启动。".to_string(),
                    (true, true) => {
                        "Codex 已用软件渲染兼容模式启动；此设备需要关闭 Chromium 沙箱。".to_string()
                    }
                },
            });
        }
    }

    let exit = child
        .try_wait()
        .ok()
        .flatten()
        .and_then(|status| status.code())
        .map(|code| code.to_string())
        .unwrap_or_else(|| "仍有后台进程但没有可用窗口".to_string());
    // The retry must not attach to a broken first attempt or leave its GPU
    // cache locks alive. Scope shutdown to this exact portable program root.
    if crate::windows_process::processes_running_for_root(root)
        .map_err(PortableLaunchFailure::Fatal)?
    {
        crate::windows_process::close_processes_for_root(5, root)
            .map_err(PortableLaunchFailure::Fatal)?;
    }
    Err(PortableLaunchFailure::NoUsableWindow(format!(
        "未显示稳定的 Codex 可操作窗口（模式={}，状态={exit}）",
        portable_launch_mode(software_rendering, no_sandbox)
    )))
}

pub fn launch_codex_with_options(
    installed: &InstalledWindowsCodex,
    options: LaunchOptions,
) -> Result<CodexLaunchReport, EngineError> {
    if installed.source == "portable" {
        let root = Path::new(&installed.path);
        let exe = if options.require_usb_entry_overlay {
            crate::portable::installed_usb_codex_launch_exe(root)?
        } else {
            crate::portable::installed_codex_launch_exe(root)?
        };
        // Keep older non-USB/standard portable roots safe as well.  The patch
        // is no-op for a package whose bootstrap does not use the MSIX
        // updater, and is applied before either launch strategy below.
        crate::portable::ensure_portable_codex_startup_compatibility(root)?;
        // The locale patch is deliberately launch-time: the manager's
        // language can change without reinstalling the payload, and the
        // fixed-width edit is idempotent for existing USB installs.
        let _ = crate::portable::ensure_portable_codex_locale_compatibility(
            root,
            options.locale_override.as_deref(),
        )?;
        #[cfg(windows)]
        if options.portable_launch_policy != PortableLaunchPolicy::Standard {
            let first_no_sandbox =
                options.portable_launch_policy == PortableLaunchPolicy::UsbCompatibleNoSandbox;
            // Match a direct ChatGPT.exe launch first.  The old USB path
            // forced --disable-gpu here, which could surface Electron's
            // title-less Chrome_WidgetWin_0 renderer as a full-size black
            // window.  Keep software rendering as a bounded fallback.
            match launch_portable_with_usable_window(root, &exe, &options, false, first_no_sandbox)
            {
                Ok(report) => return Ok(report),
                Err(first_failure) if first_failure.should_retry_compatibility() => {
                    let first_error = first_failure.to_string();
                    log::warn!(
                        "portable Codex did not show a stable usable window; retrying software-rendering compatibility mode: {first_error}"
                    );
                    return launch_portable_with_usable_window(
                        root,
                        &exe,
                        &options,
                        true,
                        true,
                    )
                    .map_err(|fallback_error| {
                        EngineError::Io(format!(
                            "Codex 启动失败；常规渲染尝试未显示稳定窗口：{first_error}；软件渲染兼容重试：{fallback_error}"
                        ))
                    });
                }
                Err(fatal) => return Err(fatal.into_engine_error()),
            }
        }

        // CREATE_NO_WINDOW only suppresses a console flash; the GUI still shows.
        // The ordinary non-USB manager retains its historical liveness check.
        let mut command = hidden_command(exe);
        if options.disable_codex_self_updates {
            command.env(CODEX_SELF_UPDATE_ENV_KEY, CODEX_SELF_UPDATE_ENV_DISABLED);
        }
        if let Some(codex_home) = options.codex_home.as_ref() {
            command.env("CODEX_HOME", codex_home);
        }
        if let Some(locale) = normalized_locale_override(options.locale_override.as_deref()) {
            command.env(CODEX_LOCALE_OVERRIDE_ENV_KEY, locale);
        }
        if let Some(port) = options.remote_debugging_port {
            command.args(remote_debugging_arguments(port));
        }
        match spawn_and_require_liveness(command, PORTABLE_LIVENESS_WINDOW) {
            Ok(LivenessResult::Survived { child }) => {
                std::mem::forget(child);
                Ok(CodexLaunchReport {
                    launch_mode: "standard".to_string(),
                    usable_window_confirmed: false,
                    no_sandbox: false,
                    message: "Codex 进程已启动。".to_string(),
                })
            }
            Ok(LivenessResult::ExitedEarly { code }) => Err(EngineError::Io(format!(
                "Codex exited immediately after launch (exit={})",
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ))),
            Err(err) => Err(EngineError::Io(format!("launch Codex: {}", err.message()))),
        }
    } else {
        if options.disable_codex_self_updates {
            log::debug!(
                "launching MSIX Codex with updater disabled via persisted user environment"
            );
        }
        let mut arguments = options
            .remote_debugging_port
            .map(remote_debugging_arguments)
            .map(|args| args.to_vec())
            .unwrap_or_default();
        if let Some(locale) = normalized_locale_override(options.locale_override.as_deref()) {
            arguments.push(format!("--lang={locale}"));
        }
        if arguments.is_empty() {
            launch_msix_app()?;
        } else {
            launch_msix_app_with_arguments(installed, &arguments.join(" "))?;
        }
        Ok(CodexLaunchReport {
            launch_mode: "msix".to_string(),
            usable_window_confirmed: false,
            no_sandbox: false,
            message: "Codex MSIX 激活请求已提交。".to_string(),
        })
    }
}

#[cfg(windows)]
fn launch_msix_app_with_arguments(
    installed: &InstalledWindowsCodex,
    arguments: &str,
) -> Result<(), EngineError> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, AO_NONE,
    };

    let family = installed.package_family_name.as_deref().ok_or_else(|| {
        EngineError::Io("MSIX Codex has no package family name for activation".to_string())
    })?;
    // Resolve through package registration instead of reading WindowsApps
    // directly; its ACL is not guaranteed to grant ordinary desktop apps
    // access to AppxManifest.xml.
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$pkg = Get-AppxPackage -Name {name} |
  Where-Object {{ $_.PackageFamilyName -eq {family} }} |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if ($null -eq $pkg) {{ throw 'registered Codex package not found' }}
$app = (Get-AppxPackageManifest $pkg -ErrorAction Stop).Package.Applications.Application
if ($app -is [array]) {{ $app = $app[0] }}
@{{
  packageFamilyName = [string]$pkg.PackageFamilyName
  appId = [string]$app.Id
}} | ConvertTo-Json -Compress
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        family = ps_quote(family),
    );
    let registered = run_powershell_json_with_limits(&script, RunLimits::probe())
        .map_err(|e| e.into_install())?;
    let aumid = HSTRING::from(registered_msix_aumid(&registered, family)?);
    let arguments = HSTRING::from(arguments);

    // Blocking launch calls run on a pool thread whose COM apartment is
    // normally uninitialized. If the host initialized it differently already,
    // RPC_E_CHANGED_MODE is safe to tolerate: COM is still available there.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(EngineError::Io(format!(
            "initialize COM for MSIX activation: {initialized:?}"
        )));
    }
    let uninitialize = initialized.is_ok();
    let outcome = (|| unsafe {
        let manager: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER).map_err(
                |e| EngineError::Io(format!("create application activation manager: {e}")),
            )?;
        let pid = manager
            .ActivateApplication(&aumid, &arguments, AO_NONE)
            .map_err(|e| EngineError::Io(format!("activate MSIX Codex with arguments: {e}")))?;
        log::debug!("activated MSIX Codex with arguments pid={pid}");
        Ok(())
    })();
    if uninitialize {
        unsafe { CoUninitialize() };
    }
    outcome
}

#[cfg(not(windows))]
fn launch_msix_app_with_arguments(
    _installed: &InstalledWindowsCodex,
    _arguments: &str,
) -> Result<(), EngineError> {
    Err(EngineError::Io(
        "MSIX launch with arguments is only available on Windows".to_string(),
    ))
}

#[cfg(windows)]
fn launch_msix_app() -> Result<(), EngineError> {
    // Resolve the real AUMID (PackageFamilyName!AppId) from the manifest so the
    // Explorer can activate the package; fall back to the conventional "App" id.
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$pkg = Get-AppxPackage -Name {name} | Sort-Object -Property Version -Descending | Select-Object -First 1
if ($null -eq $pkg) {{ throw 'Codex is not installed' }}
$app = (Get-AppxPackageManifest $pkg).Package.Applications.Application
if ($app -is [array]) {{ $app = $app[0] }}
$id = $app.Id
if (-not $id) {{ $id = 'App' }}
{activation}
"#,
        name = ps_quote(crate::OPENAI_PACKAGE_IDENTITY),
        activation = powershell_explorer_activation("$pkg.PackageFamilyName + \"!\" + $id",)
    );
    // Activation is fire-and-forget from the shell; bound the AUMID resolve + Start-Process.
    run_powershell_json_with_limits(&script, RunLimits::probe())
        .map(|_| ())
        .map_err(|e| e.into_install())
}

#[cfg(not(windows))]
fn launch_msix_app() -> Result<(), EngineError> {
    Err(EngineError::Io(
        "MSIX launch is only available on Windows".to_string(),
    ))
}

#[cfg(windows)]
const CAPABILITY_PROBE_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$languageMode = [string]$ExecutionContext.SessionState.LanguageMode
$isConstrained = $languageMode -eq 'ConstrainedLanguage'
$add = Get-Command Add-AppxPackage -ErrorAction SilentlyContinue
$svc = Get-Service AppXSvc -ErrorAction SilentlyContinue
$appInstaller = Get-AppxPackage -Name Microsoft.DesktopAppInstaller -ErrorAction SilentlyContinue |
  Select-Object -First 1

$policy = $null
$policySource = ''
$policyState = 'notConfigured'
$policyError = ''
function Read-CodexCapabilityRegistryValue([string]$path, [string]$name) {
  try {
    $properties = Get-ItemProperty -LiteralPath $path -ErrorAction Stop
    $property = $properties.PSObject.Properties[$name]
    if ($null -eq $property) { return @{ state = 'notConfigured' } }
    if ($property.Value -isnot [int] -or $property.Value -notin @(0, 1)) {
      return @{ state = 'invalid'; detail = "Unexpected DWORD value for $name at $path" }
    }
    return @{ state = 'value'; value = [int]$property.Value }
  } catch {
    if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
      return @{ state = 'notConfigured' }
    }
    return @{ state = 'readError'; detail = [string]$_.Exception.Message }
  }
}
foreach ($p in @(
  'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Appx',
  'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
)) {
  $entry = Read-CodexCapabilityRegistryValue $p 'AllowAllTrustedApps'
  if ($entry.state -ne 'notConfigured') {
    $policyState = [string]$entry.state
    $policy = $entry.value
    $policySource = $p
    $policyError = [string]$entry.detail
    break
  }
}
$developer = Read-CodexCapabilityRegistryValue 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' 'AllowDevelopmentWithoutDevLicense'

$meteredKnown = $false
$metered = $null
$costType = ''
if (-not $isConstrained) {
  try {
    $profile = [Windows.Networking.Connectivity.NetworkInformation,Windows.Networking.Connectivity,ContentType=WindowsRuntime]::GetInternetConnectionProfile()
    if ($null -ne $profile) {
      $cost = $profile.GetConnectionCost()
      $meteredKnown = $true
      $costType = [string]$cost.NetworkCostType
      $metered = [bool]($cost.NetworkCostType -ne 'Unrestricted' -or $cost.Roaming -or $cost.OverDataLimit)
    }
  } catch {}
}

$msixDeploymentKnown = $false
$msixDeploymentOk = $false
$msixDeploymentError = ''
if ($isConstrained) {
  $msixDeploymentError = 'PackageManager probe skipped in ConstrainedLanguage'
} else {
  try {
    $pm = New-Object -TypeName Windows.Management.Deployment.PackageManager -ErrorAction Stop
    $msixDeploymentKnown = $true
    $msixDeploymentOk = ($null -ne $pm)
  } catch {
    $msixDeploymentKnown = $true
    $msixDeploymentOk = $false
    $hr = 0
    try { $hr = [int]$_.Exception.HResult } catch {}
    if ($hr -ne 0) {
      $msixDeploymentError = ('{0} (HRESULT=0x{1:X8})' -f $_.Exception.Message, $hr)
    } else {
      $msixDeploymentError = [string]$_.Exception.Message
    }
  }
}

@{
  languageMode = $languageMode
  addAppxPackage = [bool]$add
  appxSvcExists = [bool]$svc
  appxSvcStatus = if ($svc) { [string]$svc.Status } else { '' }
  appxSvcStartType = if ($svc) { [string]$svc.StartType } else { '' }
  appInstallerInstalled = [bool]$appInstaller
  appInstallerVersion = if ($appInstaller) { [string]$appInstaller.Version } else { '' }
  allowAllTrustedApps = $policy
  allowAllTrustedAppsSource = $policySource
  allowAllTrustedAppsState = $policyState
  allowAllTrustedAppsError = $policyError
  developerModeState = [string]$developer.state
  developerModeValue = $developer.value
  developerModeError = [string]$developer.detail
  meteredKnown = $meteredKnown
  metered = $metered
  networkCostType = $costType
  msixDeploymentKnown = $msixDeploymentKnown
  msixDeploymentOk = $msixDeploymentOk
  msixDeploymentError = $msixDeploymentError
} | ConvertTo-Json -Compress
"#;

#[cfg(windows)]
pub fn probe_capabilities() -> WinCapabilityReport {
    match run_powershell_json(CAPABILITY_PROBE_SCRIPT)
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
    {
        Some(value) => capabilities_from_probe_json(&value),
        None => WinCapabilityReport::from_checks(
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            CapabilityCheck::unknown("PowerShell capability probe failed"),
            vec!["Could not complete Windows capability probe.".to_string()],
        ),
    }
}

#[cfg(not(windows))]
pub fn probe_capabilities() -> WinCapabilityReport {
    WinCapabilityReport::unknown_for_non_windows()
}

#[cfg(windows)]
fn capabilities_from_probe_json(value: &serde_json::Value) -> WinCapabilityReport {
    let add = if value
        .get("addAppxPackage")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        CapabilityCheck::available("Add-AppxPackage command is present")
    } else {
        CapabilityCheck::unavailable("Add-AppxPackage command is not present")
    };

    let svc_exists = value
        .get("appxSvcExists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let svc_status = value
        .get("appxSvcStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let svc_start = value
        .get("appxSvcStartType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let appx_service = if !svc_exists {
        CapabilityCheck::unavailable("AppXSvc service is missing")
    } else if svc_start.eq_ignore_ascii_case("Disabled") {
        CapabilityCheck::unavailable("AppXSvc service is disabled")
    } else {
        CapabilityCheck::available(format!("AppXSvc is {svc_status}, start type {svc_start}"))
    };

    let policy = value.get("allowAllTrustedApps").and_then(|v| v.as_i64());
    let policy_source = value
        .get("allowAllTrustedAppsSource")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sideload_policy = match policy {
        Some(0) => {
            CapabilityCheck::unavailable(format!("AllowAllTrustedApps=0 at {policy_source}"))
        }
        Some(1) => CapabilityCheck::available(format!("AllowAllTrustedApps=1 at {policy_source}")),
        Some(other) => {
            CapabilityCheck::unknown(format!("AllowAllTrustedApps={other} at {policy_source}"))
        }
        None => match value.get("allowAllTrustedAppsState").and_then(|v| v.as_str()) {
            Some("readError" | "invalid") => CapabilityCheck::unknown(format!(
                "AllowAllTrustedApps could not be determined at {policy_source}: {}",
                value.get("allowAllTrustedAppsError").and_then(|v| v.as_str()).unwrap_or("unknown registry error")
            )),
            _ => CapabilityCheck::unknown("AllowAllTrustedApps is not explicitly configured; Windows defaults apply, this is not an explicit policy denial"),
        },
    };

    let app_installer = if value
        .get("appInstallerInstalled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let version = value
            .get("appInstallerVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        CapabilityCheck::available(format!("Desktop App Installer {version}"))
    } else {
        CapabilityCheck::unknown("Desktop App Installer package was not detected")
    };

    let metered_network = if !value
        .get("meteredKnown")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        CapabilityCheck::unknown("WinRT network cost could not be read")
    } else if value
        .get("metered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let cost = value
            .get("networkCostType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        CapabilityCheck {
            state: CapabilityState::Unavailable,
            detail: format!("current connection is metered ({cost})"),
        }
    } else {
        let cost = value
            .get("networkCostType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        CapabilityCheck::available(format!("current connection is not metered ({cost})"))
    };

    // The functional probe: did the WinRT PackageManager actually activate? Only
    // an explicit activation failure (known && !ok) flips this to Unavailable —
    // an unrun probe stays Unknown so a good machine is never steered to portable
    // on a signal we couldn't read. This is the check that catches 0x80040154.
    let msix_deployment = if !value
        .get("msixDeploymentKnown")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        CapabilityCheck::unknown("MSIX deployment (PackageManager) could not be probed")
    } else if value
        .get("msixDeploymentOk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        CapabilityCheck::available("PackageManager (MSIX deployment runtime) activates")
    } else {
        let err = value
            .get("msixDeploymentError")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        CapabilityCheck::unavailable(format!(
            "PackageManager (MSIX deployment runtime) cannot activate: {err}"
        ))
    };

    let mut notes = vec!["Certificate trust is verified after the MSIX is staged.".to_string()];
    if let Some(state) = value.get("developerModeState").and_then(|v| v.as_str()) {
        notes.push(format!(
            "Developer mode probe: state={state}, value={}, detail={}; developer mode is not enabled by this installer.",
            value.get("developerModeValue").filter(|v| !v.is_null()).map(|v| v.to_string()).unwrap_or_else(|| "not configured".into()),
            value.get("developerModeError").and_then(|v| v.as_str()).unwrap_or("")
        ));
    }
    if value
        .get("languageMode")
        .and_then(|v| v.as_str())
        .is_some_and(|mode| mode.eq_ignore_ascii_case("ConstrainedLanguage"))
    {
        notes.push(
            "PowerShell is running in ConstrainedLanguage; restricted WinRT probes remain unknown."
                .to_string(),
        );
    }

    WinCapabilityReport::from_checks(
        add,
        appx_service,
        sideload_policy,
        app_installer,
        msix_deployment,
        metered_network,
        notes,
    )
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;
    use super::{
        parse_registered_msix_package_full_name, registered_msix_aumid, remote_debugging_arguments,
    };

    #[cfg(windows)]
    const TEST_AUTHORIZED_SHA256: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn builds_loopback_remote_debugging_arguments() {
        assert_eq!(
            remote_debugging_arguments(9345),
            [
                "--remote-debugging-address=127.0.0.1".to_string(),
                "--remote-debugging-port=9345".to_string(),
            ]
        );
    }

    #[test]
    fn github_api_headers_are_separate_curl_arguments() {
        let mut command = std::process::Command::new("curl");
        super::append_curl_headers(&mut command, super::GITHUB_API_HEADERS);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "-H",
                "Accept: application/vnd.github+json",
                "-H",
                "User-Agent: Codex-App-Manager",
            ]
        );
    }

    #[test]
    fn portable_launch_only_retries_after_a_successful_probe_without_a_usable_window() {
        let spawn_error = super::PortableLaunchFailure::Fatal(super::EngineError::Io(
            "launch Codex: access denied".to_string(),
        ));
        let diagnostic_error = super::PortableLaunchFailure::Fatal(super::EngineError::Io(
            "window probe returned an error dialog".to_string(),
        ));
        let compatibility_error = super::PortableLaunchFailure::Fatal(super::EngineError::Io(
            "startup compatibility patch failed".to_string(),
        ));
        let cleanup_error = super::PortableLaunchFailure::Fatal(super::EngineError::Io(
            "process-tree cleanup failed".to_string(),
        ));
        let no_usable_window = super::PortableLaunchFailure::NoUsableWindow(
            "未显示稳定的 Codex 可操作窗口".to_string(),
        );

        assert!(!spawn_error.should_retry_compatibility());
        assert!(!diagnostic_error.should_retry_compatibility());
        assert!(!compatibility_error.should_retry_compatibility());
        assert!(!cleanup_error.should_retry_compatibility());
        assert!(no_usable_window.should_retry_compatibility());
    }

    #[cfg(windows)]
    #[test]
    fn usb_launch_prefers_normal_gpu_and_keeps_compatibility_fallback_args() {
        assert!(portable_compatibility_arguments(false, false).is_empty());
        assert_eq!(
            portable_compatibility_arguments(false, true),
            vec!["--no-sandbox"]
        );
        assert_eq!(
            portable_compatibility_arguments(true, false),
            vec!["--disable-gpu"]
        );
        assert_eq!(
            portable_compatibility_arguments(true, true),
            vec!["--disable-gpu", "--no-sandbox"]
        );
        assert_eq!(portable_launch_mode(false, false), "portable");
        assert_eq!(portable_launch_mode(false, true), "portable-no-sandbox");
        assert_eq!(portable_launch_mode(true, false), "software-rendering");
        assert_eq!(
            portable_launch_mode(true, true),
            "software-rendering-no-sandbox"
        );
    }

    #[test]
    fn registered_app_entry_must_match_the_detected_package_family() {
        let json = r#"{"packageFamilyName":"OpenAI.Codex_abc","appId":"CodexApp"}"#;
        assert_eq!(
            registered_msix_aumid(json, "openai.codex_ABC").unwrap(),
            "OpenAI.Codex_abc!CodexApp"
        );
        assert!(registered_msix_aumid(json, "OpenAI.Codex_other").is_err());
        assert!(registered_msix_aumid(
            r#"{"packageFamilyName":"OpenAI.Codex_abc","appId":""}"#,
            "OpenAI.Codex_abc"
        )
        .is_err());
    }

    #[test]
    fn exact_machine_cleanup_identity_rejects_broad_or_malformed_names() {
        assert!(super::valid_exact_codex_package_full_name(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0"
        ));
        for invalid in [
            "OpenAI.Codex_26.825.5331.0_x64__other",
            "OpenAI.Codex_26.825.5331_x64__2p2nqsd0c76g0",
            "OpenAI.Codex_26.825.5331.0_arm64__2p2nqsd0c76g0",
            "OpenAI.Codex_*_x64__2p2nqsd0c76g0",
            "Other.App_26.825.5331.0_x64__2p2nqsd0c76g0",
        ] {
            assert!(
                !super::valid_exact_codex_package_full_name(invalid),
                "{invalid} must not authorize machine cleanup"
            );
        }
    }

    #[test]
    fn parses_exact_registered_msix_package_full_name() {
        assert_eq!(
            parse_registered_msix_package_full_name(
                r#""OpenAI.Codex_26.803.10989.0_x64__2p2nqsd0c76g0""#
            )
            .unwrap()
            .as_deref(),
            Some("OpenAI.Codex_26.803.10989.0_x64__2p2nqsd0c76g0")
        );
        assert_eq!(
            parse_registered_msix_package_full_name("null").unwrap(),
            None
        );
        assert!(parse_registered_msix_package_full_name("not-json").is_err());
    }

    #[cfg(windows)]
    const STRICT_USB_MOCKS: &str = r#"
$script:registeredLocation = $null
$script:addCalls = 0
$script:removeCalls = 0
$script:moveCalls = 0
$script:addError = $null
$script:queryError = $false
$script:moveError = $null
$script:observedFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
function Test-Path { param($LiteralPath, $PathType, $ErrorAction); $LiteralPath -eq 'E:\WindowsApps' }
function Get-Volume { param($DriveLetter, $ErrorAction); @{ FileSystem = 'NTFS' } }
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  @{ Name = 'fixture-usb'; PackageStorePath = 'E:\WindowsApps'; IsOffline = $false }
}
function Add-AppxVolume { param($Path, $ErrorAction); throw 'unexpected volume mutation' }
function Get-AppxPackage {
  param($Name, [switch]$AllUsers, $Volume, $ErrorAction)
  if ($script:queryError) { throw 'registration query denied' }
  if ($script:registeredLocation) {
    @{
      PackageFullName = $script:observedFullName
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      InstallLocation = $script:registeredLocation
      Version = '26.825.5331.0'
      PackageUserInformation = @(@{ UserSecurityId = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value; InstallState = 'Installed' })
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ErrorAction)
  $script:addCalls++
  if ($null -eq $Volume -or $Volume.Name -ne 'fixture-usb') { throw 'NON-USB TARGET' }
  if ($script:addError) { throw $script:addError }
  $script:registeredLocation = 'E:\WindowsApps\OpenAI.Codex'
}
function Move-AppxPackage {
  param($Package, $Volume, $ErrorAction)
  $script:moveCalls++
  if ($script:moveError) { throw $script:moveError }
  $script:registeredLocation = 'E:\WindowsApps\OpenAI.Codex'
}
function Remove-AppxPackage {
  param($Package, $PreserveApplicationData, $ErrorAction)
  $script:removeCalls++
  $script:registeredLocation = $null
}
function Get-AppxLog { param($ActivityId, $ErrorAction); @() }
"#;

    #[cfg(windows)]
    fn strict_usb_mock_report(setup: &str) -> serde_json::Value {
        let production = install_msix_script_with_volume(
            Path::new(r"C:\staging\Codex.msix"),
            Some(Path::new("E:\\")),
        );
        // Stub every native invocation; behavioral tests never deploy real AppX.
        let shim = "function Invoke-NativeAppxAdd { param($packagePath, $targetVolume); throw 'native unavailable in fixture' }\n";
        let production = production.replace(
            "$nativeAppxResult = Invoke-NativeAppxAdd",
            &format!("{shim}$nativeAppxResult = Invoke-NativeAppxAdd"),
        );
        let script = format!(
            "{STRICT_USB_MOCKS}\n{setup}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{ report = $report; adds = $script:addCalls; removes = $script:removeCalls; moves = $script:moveCalls; location = $script:registeredLocation }} | ConvertTo-Json -Depth 8 -Compress"
        );
        serde_json::from_str(&run_powershell_json(&script).expect("run strict USB mocks")).unwrap()
    }

    #[cfg(windows)]
    const CONSTRAINED_APPX_MOCKS: &str = r#"
$ErrorActionPreference = 'Stop'
$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'
if ($ExecutionContext.SessionState.LanguageMode -ne 'ConstrainedLanguage') {
  throw 'failed to enter ConstrainedLanguage'
}
function Add-AppxPackage {
  param($LiteralPath, $Path, $ForceUpdateFromAnyVersion, $ErrorAction)
  if ($Path -ne 'C:\staging\Codex.msix' -or $null -ne $LiteralPath -or -not $ForceUpdateFromAnyVersion) {
    throw 'unexpected Add-AppxPackage arguments'
  }
}
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($Name -ne 'OpenAI.Codex') { throw 'unexpected Get-AppxPackage name' }
  '{"InstallLocation":"C:\\Program Files\\WindowsApps\\OpenAI.Codex","Version":"26.707.3748.0","PackageFamilyName":"OpenAI.Codex_2p2nqsd0c76g0"}' |
    ConvertFrom-Json
}
"#;

    #[cfg(windows)]
    #[test]
    fn parses_probe_json_into_portable_recommendation_when_policy_blocks() {
        let value = serde_json::json!({
            "addAppxPackage": true,
            "appxSvcExists": true,
            "appxSvcStatus": "Running",
            "appxSvcStartType": "Manual",
            "appInstallerInstalled": true,
            "appInstallerVersion": "1.0.0.0",
            "allowAllTrustedApps": 0,
            "allowAllTrustedAppsSource": "HKLM:\\SOFTWARE\\Policies\\Microsoft\\Windows\\Appx",
            "meteredKnown": true,
            "metered": false,
            "networkCostType": "Unrestricted",
            "msixDeploymentKnown": true,
            "msixDeploymentOk": true,
            "msixDeploymentError": ""
        });
        let report = capabilities_from_probe_json(&value);
        assert_eq!(
            report.recommendation,
            crate::capability::SideloadRecommendation::PortableFallback
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_probe_json_into_portable_when_deployment_broken() {
        // Every existence check looks fine, but the functional PackageManager
        // probe failed (0x80040154). The recommendation must be portable and the
        // deployment check must read Unavailable — this is the issue #13 machine.
        let value = serde_json::json!({
            "addAppxPackage": true,
            "appxSvcExists": true,
            "appxSvcStatus": "Running",
            "appxSvcStartType": "Manual",
            "appInstallerInstalled": true,
            "appInstallerVersion": "1.0.0.0",
            "allowAllTrustedApps": 1,
            "allowAllTrustedAppsSource": "HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock",
            "meteredKnown": true,
            "metered": false,
            "networkCostType": "Unrestricted",
            "msixDeploymentKnown": true,
            "msixDeploymentOk": false,
            "msixDeploymentError": "没有注册类 (HRESULT=0x80040154)"
        });
        let report = capabilities_from_probe_json(&value);
        assert_eq!(
            report.recommendation,
            crate::capability::SideloadRecommendation::PortableFallback
        );
        assert_eq!(
            report.msix_deployment.state,
            crate::capability::CapabilityState::Unavailable
        );
    }

    #[cfg(windows)]
    #[test]
    fn constrained_language_capability_probe_keeps_restricted_checks_unknown() {
        let script = format!(
            "$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'; if ($ExecutionContext.SessionState.LanguageMode -ne 'ConstrainedLanguage') {{ throw 'CLM was not enabled' }}; {}",
            super::CAPABILITY_PROBE_SCRIPT
        );
        let json = run_powershell_json(&script).expect("run constrained-language capability probe");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("parse constrained-language capability JSON");
        assert_eq!(
            value.get("languageMode").and_then(|v| v.as_str()),
            Some("ConstrainedLanguage")
        );
        assert_eq!(
            value.get("msixDeploymentKnown").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value.get("msixDeploymentOk").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(value
            .get("msixDeploymentError")
            .and_then(|v| v.as_str())
            .is_some_and(|error| error.contains("skipped")));
        assert_eq!(
            value.get("meteredKnown").and_then(|v| v.as_bool()),
            Some(false)
        );
        let report = capabilities_from_probe_json(&value);
        assert_eq!(report.msix_deployment.state, CapabilityState::Unknown);
        assert_eq!(report.metered_network.state, CapabilityState::Unknown);
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("ConstrainedLanguage")));
    }

    #[cfg(windows)]
    #[test]
    fn constrained_language_msix_install_script_preserves_json_contract() {
        let production = install_msix_script(std::path::Path::new(r"C:\staging\Codex.msix"));
        assert!(production.contains("Get-AppxLog"));
        assert!(production.contains("ActivityId"));
        let script = format!("{}\n{}", CONSTRAINED_APPX_MOCKS, production);
        let json = run_powershell_json(&script).expect("run constrained-language MSIX install");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse constrained-language install report");
        assert!(report.success);
        let installed = report.installed.expect("mock install should be detected");
        assert_eq!(installed.version, "26.707.3748.0");
        assert_eq!(installed.source, "msix");
        assert_eq!(
            installed.package_family_name.as_deref(),
            Some("OpenAI.Codex_2p2nqsd0c76g0")
        );
    }

    #[cfg(windows)]
    #[test]
    fn preserve_application_data_compatibility_classifier_is_narrow() {
        // A sideload registration can expose the PowerShell switch yet reject
        // it at runtime with this HRESULT/wording. The generated uninstall
        // script retries only this compatibility class; ordinary access,
        // identity and policy failures must still remain hard failures.
        assert!(!PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("0x80073CFA"));
        assert!(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("PreserveApplicationData"));
        assert!(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("deployment mode"));
        assert!(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("部署模式"));
        assert!(!PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("PreserveApplicationData|"));
        assert!(!PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN.contains("Access is denied"));

        // Exercise the same .NET regex engine used by PowerShell, rather than
        // only checking the source text. This guards against a future pattern
        // change that accidentally broadens the fallback to ordinary errors.
        let script = format!(
            r#"$pattern = {pattern}; @(
  ('部署失败，只能在以部署模式部署的程序包上使用 PreserveApplicationData 标记。' -match $pattern)
  ('Remove-AppxPackage: Access is denied. PreserveApplicationData was requested but the package is locked.' -match $pattern)
  ('0x80073CFA' -match $pattern)
  ('deployment policy rejected the package' -match $pattern)
) | ConvertTo-Json -Compress"#,
            pattern = ps_quote(PRESERVE_APPLICATION_DATA_UNSUPPORTED_PATTERN),
        );
        let json =
            run_powershell_json(&script).expect("evaluate PreserveApplicationData classifier");
        let matches: Vec<bool> = serde_json::from_str(&json).expect("parse classifier result");
        assert_eq!(matches, vec![true, false, false, false]);
    }

    #[cfg(windows)]
    #[test]
    fn msix_install_script_normalizes_extended_package_paths() {
        // Rust's canonicalize() returns this spelling on Windows.  It is valid
        // for filesystem APIs but is rejected by the AppX PowerShell binder on
        // affected Windows/.NET builds as an invalid URI.
        let production = install_msix_script(std::path::Path::new(r"\\?\C:\staging\Codex.msix"));
        let script = format!("{}\n{}", CONSTRAINED_APPX_MOCKS, production);
        let json = run_powershell_json(&script).expect("run extended-path normalization mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse extended-path install report");
        assert!(
            report.success,
            "extended package path should be normalized before Add-AppxPackage: {report:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn msix_install_script_exposes_hresult_and_activity_id_on_failure() {
        let production = install_msix_script(std::path::Path::new(r"C:\staging\Codex.msix"));
        let mocks = r#"
function Add-AppxPackage {
  param($LiteralPath, $Path, $ForceUpdateFromAnyVersion, $ErrorAction)
  throw 'Deployment failed with HRESULT: 0x80073CF0 ActivityId {9afe3240-101d-0002-37b9-8aa11d10dd01}'
}
function Get-AppxLog {
  param($ActivityId, $ErrorAction)
  @{ Id = 42; Message = 'fixture deployment event'; TimeCreated = '2026-09-05T00:00:00Z' }
}
# Avoid module autoload through the legacy helper's capability checks: loading
# Appx exports its own Get-AppxLog function and would replace our mock.
function Move-AppxPackage { param($Package, $Volume, $ErrorAction); throw 'unexpected fixture move' }
function Get-AppxProvisionedPackage { param([switch]$Online, $ErrorAction); @() }
"#;
        let json = run_powershell_json(&format!("{}\n{}", mocks, production))
            .expect("run failing MSIX mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse failing MSIX report");
        assert!(!report.success);
        assert_eq!(report.hresult.as_deref(), Some("0X80073CF0"));
        assert_eq!(
            report.activity_id.as_deref(),
            Some("9afe3240-101d-0002-37b9-8aa11d10dd01")
        );
        assert!(report.message.contains("HRESULT=0X80073CF0"));
        assert!(
            report.message.contains("fixture deployment event"),
            "{report:?}"
        );
        assert!(report
            .message
            .contains("ActivityId=9afe3240-101d-0002-37b9-8aa11d10dd01"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_script_targets_registered_volume() {
        let script = install_msix_script_with_volume(
            std::path::Path::new(r"F:\staging\Codex.msix"),
            Some(std::path::Path::new("F:\\")),
        );
        assert!(script.contains("Add-AppxVolume"));
        assert!(script.contains("Get-AppxVolume"));
        assert!(script.contains("$installArgs['Volume'] = $volume"));
        assert!(script.contains("Move-AppxPackage"));
        assert!(!script.contains("$systemArgs"));
        assert!(script.contains("$movedExisting"));
        assert!(script.contains("F:\\"));
        assert!(script.contains("does not expose -Volume"));
        assert!(script.contains("$installArgs['Path']"));
        assert!(script.contains("Get-AppxVolume -Path $mountPoint -Online"));
        assert!(script.contains("Mount-AppxVolume"));
        assert!(script.contains("system-volume fallback is disabled"));
        assert!(script.contains("Invoke-NativeAppxAdd"));
        assert!(script.contains("Invoke-CodexRemoveAppxPackage"));
        assert!(script.contains("wrongVolumeCleanupWarning"));
        assert!(script.contains("preserveDataFallbackWarning"));
        assert!(script.contains("PackageManager.AddPackageAsync"));
        assert!(script.contains("Wait-CodexCurrentUserPackageProjection"));
        assert!(script.contains("exact healthy current-user package"));
        assert!(script.contains("isRegisteredHint"));
        assert!(
            !script.contains("PackageManager returned IsRegistered=false"),
            "DeploymentResult.IsRegistered is diagnostic-only; exact current-user health/placement is the completion contract"
        );
        assert!(script.contains("System.Runtime.WindowsRuntime"));
        assert!(script.contains("FindPackageVolume"));
        assert!(script.contains("nativePackageManagerError="));
        assert!(script.contains("Get-Volume -DriveLetter"));
        assert!(script.contains("NTFS is required for MSIX AppX volumes"));
        assert!(
            script.contains("operation="),
            "failure diagnostics include the failing AppX operation"
        );
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_rejects_non_ntfs_volume_before_appx_mutation() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"F:\staging\Codex.msix"),
            Some(std::path::Path::new("F:\\")),
        );
        let mocks = r#"
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'exFAT'; DriveType = 'Removable' }
}
function Get-AppxVolume { param($ErrorAction); @() }
function Add-AppxVolume { param($Path, $ErrorAction); throw 'must not mutate a non-NTFS volume' }
function Add-AppxPackage {
  param($LiteralPath, $Path, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $Volume, $ErrorAction)
}
"#;
        let json = run_powershell_json(&format!("{}\n{}", mocks, production))
            .expect("run non-NTFS USB preflight mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse non-NTFS USB preflight report");
        assert!(!report.success);
        assert!(report
            .message
            .contains("NTFS is required for MSIX AppX volumes"));
        assert!(report
            .message
            .contains("operation=Validate USB AppX volume"));
        assert!(report.message.contains("volumeFileSystem=exFAT"));
        assert!(report.message.contains("targetVolume=F:\\"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_data_conflict_never_bootstraps_system_volume() {
        let value = strict_usb_mock_report(
            "$script:addError = '0x80073CF9: app data is on another volume'",
        );
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["adds"], 1);
        assert_eq!(value["removes"], 0);
        assert_eq!(value["moves"], 0);
        assert!(value["location"].is_null());
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_refreshes_online_volume_and_uses_documented_path() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // The first query sees no registered volume. Add-AppxVolume returns an
        // object, but the script must re-read the online object by path before
        // handing it to the typed -Volume parameter. The mock rejects the
        // legacy LiteralPath route and accepts only the documented Path key.
        let mocks = r#"
$script:volumeAdded = $false
$script:packageVolume = $null
$script:onlineQueryCount = 0
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Path) {
    $script:onlineQueryCount++
    if ($script:volumeAdded -and $Online) {
      @{
        Name = '\\?\Volume{usb-online}'
        MountPoint = ''
        PackageStorePath = 'E:\WindowsApps'
        IsOffline = $false
        Source = 'online-refresh'
      }
    }
    return
  }
  if ($script:volumeAdded) {
    @{
      Name = '\\?\Volume{usb-online}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $false
      Source = 'enumeration'
    }
  }
}
function Add-AppxVolume {
  param($Path, $ErrorAction)
  $script:volumeAdded = $true
  @{
    Name = '\\?\Volume{usb-added}'
    MountPoint = ''
    PackageStorePath = 'E:\WindowsApps'
    IsOffline = $false
    Source = 'add-result'
  }
}
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($LiteralPath, $Path, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $Volume, $ErrorAction)
  if ($null -ne $LiteralPath) { throw 'legacy LiteralPath must not be used' }
  if ($Path -ne 'C:\staging\Codex.msix') { throw "unexpected package path: $Path" }
  if ($null -eq $Volume -or $Volume.Source -ne 'online-refresh') {
    throw 'Add-AppxPackage did not receive the refreshed online AppxVolume'
  }
  $script:packageVolume = 'E:\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
}
"#;
        let json = run_powershell_json(&format!("{}\n{}", mocks, production))
            .expect("run online-volume/path compatibility mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse online-volume/path report");
        assert!(report.success, "USB direct Add should succeed: {report:?}");
        assert_eq!(
            report
                .installed
                .as_ref()
                .and_then(|installed| installed.package_volume.as_deref()),
            Some("E:\\")
        );
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_native_failure_never_bootstraps_system_volume() {
        let value = strict_usb_mock_report(
            "$script:addError = 'Invalid URI: The hostname could not be parsed'",
        );
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["adds"], 1);
        assert_eq!(value["removes"], 0);
        assert_eq!(value["moves"], 0);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("system-volume fallback is disabled"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_failed_move_preserves_existing_package() {
        let value = strict_usb_mock_report(
            r"$script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'; $script:moveError = '0x80073CF9 data conflict'",
        );
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["adds"], 0);
        assert_eq!(value["removes"], 0);
        assert_eq!(value["moves"], 1);
        assert_eq!(
            value["location"],
            r"C:\Program Files\WindowsApps\OpenAI.Codex"
        );
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_query_failure_never_authorizes_install_or_cleanup() {
        let value = strict_usb_mock_report("$script:queryError = $true");
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["adds"], 0);
        assert_eq!(value["removes"], 0);
        assert_eq!(value["moves"], 0);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("registration query denied"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_wrong_volume_cleanup_requires_verified_payload_identity() {
        let value = strict_usb_mock_report(
            r#"
$script:observedFullName = 'OpenAI.Codex_99.0.0.0_x64__2p2nqsd0c76g0'
function Add-AppxPackage {
  param($Path, $Volume, $ErrorAction)
  $script:addCalls++
  $script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'
}
"#,
        );
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["removes"], 0);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("does not match this verified payload"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_recovers_uri_failure_with_native_package_manager() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // The PowerShell -Volume parameter set fails with the URI error, but
        // the native WinRT call succeeds.  Inject the shim immediately before
        // the generated script invokes the helper so this test exercises the
        // branch without mutating the host's AppX registration.
        let mocks = r#"
$script:packageVolume = $null
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Path -and $Online) {
    @{
      Name = if ($Path -like 'C:*') { '\\?\Volume{system-online}' } else { '\\?\Volume{usb-online}' }
      MountPoint = ''
      PackageStorePath = if ($Path -like 'C:*') { 'C:\Program Files\WindowsApps' } else { 'E:\WindowsApps' }
      IsOffline = $false
      IsSystemVolume = ($Path -like 'C:*')
      Source = if ($Path -like 'C:*') { 'system-online' } else { 'usb-online' }
    }
  }
}
function Add-AppxVolume { param($Path, $ErrorAction); throw 'volume should already be registered' }
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $ErrorAction)
  if ($Volume -and $Volume.Source -eq 'usb-online') { throw 'Invalid URI: The hostname could not be parsed' }
  throw 'unexpected Add-AppxPackage target'
}
"#;
        let native_shim = r#"
function Invoke-NativeAppxAdd {
  param([string]$packagePath, $targetVolume)
  $script:packageVolume = 'E:\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
  @{
    # Reproduce the Windows 10 report: the native hint is false even though
    # the independent current-user package projection is already healthy.
    isRegisteredHint = $false
    activityId = '00000000-0000-0000-0000-000000000001'
    errorText = ''
    extendedError = $null
    volumeName = [string]$targetVolume.Name
    packageUri = 'file:///C:/staging/Codex.msix'
  }
}
"#;
        let needle = "$nativeAppxResult = Invoke-NativeAppxAdd";
        let insert_at = production
            .find(needle)
            .expect("generated script should have a native recovery call");
        let production = format!(
            "{}{}\n{}",
            &production[..insert_at],
            native_shim,
            &production[insert_at..]
        );
        let json = run_powershell_json(&format!("{}\n{}", mocks, production))
            .expect("run native USB URI recovery mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse native USB URI recovery report");
        assert!(
            report.success,
            "native PackageManager recovery should complete: {report:?}"
        );
        assert!(report.message.contains("WinRT PackageManager"));
        assert!(!report.message.contains("system-volume bootstrap"));
        assert_eq!(
            report
                .installed
                .as_ref()
                .and_then(|installed| installed.package_volume.as_deref()),
            Some("E:\\")
        );
    }

    #[cfg(windows)]
    const PLAIN_SYSTEM_PACKAGE_MOCK: &str = r#"
function Get-Item {
  [CmdletBinding()]param($LiteralPath,[switch]$Force)
  if ($LiteralPath -eq 'C:\Program Files\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0') {
    return @{PSIsContainer=$true; Attributes=[IO.FileAttributes]::Directory}
  }
  Microsoft.PowerShell.Management\Get-Item -LiteralPath $LiteralPath -Force:$Force -ErrorAction Stop
}
"#;

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_rejects_and_cleans_a_silent_system_volume_success() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // Isolate optional backup/native recovery from the real host.
        let production = production
            .replace("$backupPath = Backup-CodexPackageData $package $mountPoint ([string]$volume.Name)", "$backupPath = $null")
            .replace("$nativeAppxResult = Invoke-NativeAppxAdd", "function Invoke-NativeAppxAdd { param($packagePath, $targetVolume); throw 'native unavailable in fixture' }\n$nativeAppxResult = Invoke-NativeAppxAdd");

        // Reproduce the affected Windows behavior: Add-AppxPackage accepts the
        // typed USB volume argument and returns normally, but the registration
        // is observable on C:\ instead. The script-level postcondition must
        // turn that false success into a failure and remove only the
        // registration created by this invocation.
        let mocks = r#"
$script:packageVolume = $null
$script:removed = $false
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Path -and $Path -like 'E:*') {
    @{
      Name = '\\?\Volume{usb-online}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $false
      Source = 'usb-online'
    }
  }
}
function Add-AppxVolume {
  param($Path, $ErrorAction)
  @{
    Name = '\\?\Volume{usb-online}'
    MountPoint = ''
    PackageStorePath = 'E:\WindowsApps'
    IsOffline = $false
    Source = 'usb-online'
  }
}
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $ErrorAction)
  if ($null -eq $Volume -or $Volume.Source -ne 'usb-online') { throw 'unexpected Add-AppxPackage target' }
  # The regression: the cmdlet reports success but registers on C:.
  $script:packageVolume = 'C:\Program Files\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
}
function Remove-AppxPackage {
  param($Package, $PreserveApplicationData, $ForceApplicationShutdown, $ErrorAction)
  $script:packageVolume = $null
  $script:removed = $true
}
"#;
        let native_shim = r#"
function Invoke-NativeAppxAdd {
  param([string]$packagePath, $targetVolume)
  throw 'native placement recovery intentionally unavailable in this fixture'
}
"#;
        let json = run_powershell_json(&format!(
            "{PLAIN_SYSTEM_PACKAGE_MOCK}\n{}\n{}\n{}",
            mocks, production, native_shim
        ))
        .expect("run silent wrong-volume success mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse silent wrong-volume report");
        assert!(
            !report.success,
            "wrong-volume success must fail closed: {report:?}"
        );
        assert!(report
            .message
            .contains("USB AppX placement verification failed"));
        assert!(report.message.contains("expectedPackageVolume=E:"));
        assert!(report.message.contains("actualPackageVolume=C:"));
        assert!(report.message.contains("wrongVolumeCleanup=completed"));
        assert!(report.installed.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_retries_wrong_volume_cleanup_without_preserve_flag() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // Isolate optional backup/native recovery from the real host.
        let production = production
            .replace("$backupPath = Backup-CodexPackageData $package $mountPoint ([string]$volume.Name)", "$backupPath = $null")
            .replace("$nativeAppxResult = Invoke-NativeAppxAdd", "function Invoke-NativeAppxAdd { param($packagePath, $targetVolume); throw 'native unavailable in fixture' }\n$nativeAppxResult = Invoke-NativeAppxAdd");

        // Reproduce the v0.5.5 failure chain: the first Add-AppxPackage call
        // silently registers on C:, and Remove-AppxPackage exposes the
        // PreserveApplicationData switch but rejects it at runtime with the
        // non-deployment 0x80073CFA error. The compatibility helper must retry
        // the exact package without the switch, then allow the native target-
        // volume retry to complete on E:.
        let mocks = r#"
$script:packageVolume = $null
$script:removedWithPreserve = $false
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Path -and $Path -like 'E:*') {
    @{
      Name = '\\?\Volume{usb-online}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $false
      Source = 'usb-online'
    }
  }
}
function Add-AppxVolume {
  param($Path, $ErrorAction)
  throw 'volume should already be registered'
}
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $ErrorAction)
  if ($null -eq $Volume -or $Volume.Source -ne 'usb-online') { throw 'unexpected Add-AppxPackage target' }
  # The regression: the cmdlet reports success but registers on C:.
  $script:packageVolume = 'C:\Program Files\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
}
function Remove-AppxPackage {
  param($Package, $PreserveApplicationData, $ForceApplicationShutdown, $ErrorAction)
  if ($PreserveApplicationData) {
    $script:removedWithPreserve = $true
    throw 'Deployment failed: 0x80073CFA; PreserveApplicationData is valid only for deployment mode packages'
  }
  $script:packageVolume = $null
}
"#;
        let json = run_powershell_json(&format!(
            "{PLAIN_SYSTEM_PACKAGE_MOCK}\n{}\n{}",
            mocks, production
        ))
        .expect("run PreserveApplicationData compatibility mock");
        let report: MsixSideloadReport = serde_json::from_str(&json)
            .expect("parse PreserveApplicationData compatibility report");
        assert!(
            !report.success,
            "placement must still fail closed when native recovery is unavailable: {report:?}"
        );
        assert!(report.message.contains("PreserveApplicationData"));
        assert!(report.message.contains("wrongVolumeCleanup=completed"));
        assert!(report.message.contains("nativePlacementRetryError"));
        assert!(report.installed.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_cleans_wrong_volume_when_preserve_parameter_is_missing() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // Isolate optional backup/native recovery from the real host.
        let production = production
            .replace("$backupPath = Backup-CodexPackageData $package $mountPoint ([string]$volume.Name)", "$backupPath = $null")
            .replace("$nativeAppxResult = Invoke-NativeAppxAdd", "function Invoke-NativeAppxAdd { param($packagePath, $targetVolume); throw 'native unavailable in fixture' }\n$nativeAppxResult = Invoke-NativeAppxAdd");

        // Down-level AppX modules may not expose PreserveApplicationData at
        // all. A registration created by this invocation must still be
        // removed without the optional flag, with an explicit warning.
        let mocks = r#"
$script:packageVolume = $null
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Path -and $Path -like 'E:*') {
    @{
      Name = '\\?\Volume{usb-online}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $false
      Source = 'usb-online'
    }
  }
}
function Add-AppxVolume { param($Path, $ErrorAction); throw 'volume should already be registered' }
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $ErrorAction)
  if ($null -eq $Volume -or $Volume.Source -ne 'usb-online') { throw 'unexpected Add-AppxPackage target' }
  $script:packageVolume = 'C:\Program Files\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
}
function Remove-AppxPackage {
  param($Package, $ErrorAction)
  $script:packageVolume = $null
}
"#;
        let native_shim = r#"
function Invoke-NativeAppxAdd {
  param([string]$packagePath, $targetVolume)
  throw 'native placement recovery intentionally unavailable in this fixture'
}
"#;
        let json = run_powershell_json(&format!(
            "{PLAIN_SYSTEM_PACKAGE_MOCK}\n{}\n{}\n{}",
            mocks, production, native_shim
        ))
        .expect("run missing-Preserve compatibility mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse missing-Preserve compatibility report");
        assert!(
            !report.success,
            "placement must remain fail-closed: {report:?}"
        );
        assert!(report.message.contains("PreserveApplicationData"));
        assert!(report.message.contains("wrongVolumeCleanup=completed"));
        assert!(report.installed.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn temporary_powershell_scripts_are_unique_for_parallel_calls_and_keep_cleanup_contract() {
        let barrier = std::sync::Barrier::new(32);
        let results = std::thread::scope(|scope| {
            let threads = (0..32)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        (0..4)
                            .map(|_| {
                                write_temporary_powershell_script("# isolated concurrent fixture")
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .flat_map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        let mut unique = std::collections::HashSet::new();
        let mut failures = Vec::new();
        let prefix = format!("codex-win-engine-ps-{}-", std::process::id());
        for result in results {
            match result {
                Ok(path) => {
                    if !unique.insert(path.clone()) {
                        failures.push(format!("duplicate path: {}", path.display()));
                    }
                    let name = path.file_name().unwrap().to_string_lossy();
                    let suffix = name
                        .strip_prefix(&prefix)
                        .unwrap()
                        .strip_suffix(".ps1")
                        .unwrap();
                    if suffix.is_empty() || !suffix.bytes().all(|c| c.is_ascii_digit()) {
                        failures.push(format!("cleanup filename contract broken: {name}"));
                    }
                    if std::fs::read(&path).unwrap() != b"\xef\xbb\xbf# isolated concurrent fixture"
                    {
                        failures.push("script content/BOM mismatch".into());
                    }
                    // Delete only paths returned from this test's create-new calls.
                    std::fs::remove_file(path).unwrap();
                }
                Err(error) => failures.push(error),
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(unique.len(), 128);
    }

    #[cfg(windows)]
    const UNINSTALL_MOCKS: &str = r#"
# Every uninstall fixture defaults to an absent disposable profile. Tests with
# real temporary data explicitly override it after these mocks. New guards
# must never inspect this developer's active package-data tree.
$env:LOCALAPPDATA = Join-Path ([IO.Path]::GetTempPath()) ('codex-uninstall-isolated-' + [guid]::NewGuid().ToString('N'))
$script:removed = $false
$script:removeCalls = 0
$script:forceError = $null
$script:developmentMode = $false
$script:failQuery = $false
$script:failPostQuery = $false
function Get-AppxPackage {
  param([switch]$AllUsers, $Name, $ErrorAction)
  if ($script:failQuery -or ($script:removed -and $script:failPostQuery)) { throw 'registration query denied' }
  if (-not $script:removed -and -not $AllUsers) {
    @{
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      IsDevelopmentMode = $script:developmentMode
    }
  }
}
function Get-AppxProvisionedPackage { param([switch]$Online, $ErrorAction); @() }
function Remove-AppxPackage {
  param($Package, [switch]$PreserveApplicationData, $ErrorAction)
  $script:removeCalls++
  if ($script:forceError) {
    if (-not $PreserveApplicationData) { throw 'UNSAFE no-preserve retry' }
    throw $script:forceError
  }
  if ($PreserveApplicationData) {
    throw '0x80073CFA: PreserveApplicationData is only supported in development mode'
  }
  $script:removed = $true
}
"#;

    #[cfg(windows)]
    const MACHINE_ORPHAN_MOCKS: &str = r#"
$env:LOCALAPPDATA = Join-Path ([IO.Path]::GetTempPath()) ('codex-machine-cleanup-local-' + [guid]::NewGuid().ToString('N'))
$env:ProgramW6432 = Join-Path ([IO.Path]::GetTempPath()) ('codex-machine-cleanup-programfiles-' + [guid]::NewGuid().ToString('N'))
$env:ProgramFiles = $env:ProgramW6432
$script:mode = 'orphan'
$script:machinePresent = $true
$script:rehydrated = $false
$script:removeCalls = 0
function Get-AppxPackage {
  param($Name, [switch]$AllUsers, $Volume, $ErrorAction)
  if (-not $AllUsers) { return }
  if (-not $script:machinePresent) { return }
  if ($null -ne $Volume -and -not [bool]$Volume.IsSystemVolume) { return }
  $fullName = if ($script:mode -eq 'mismatch') {
    'OpenAI.Codex_26.825.5330.0_x64__2p2nqsd0c76g0'
  } else {
    'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
  }
  $ownerState = if ($script:mode -eq 'installed-owner') { 'Installed' } else { 'Staged' }
  $ownerId = if ($script:mode -eq 'wrapped-sid') {
    New-Object -TypeName PSObject -Property @{Sid='S-1-5-18';Username='NT AUTHORITY\SYSTEM'}
  } elseif ($script:mode -eq 'wrapped-value') {
    New-Object -TypeName PSObject -Property @{Value='S-1-5-18'}
  } else { 'S-1-5-18' }
  @{
    PackageFullName = $fullName
    PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
    InstallLocation = if ($script:rehydrated) { Join-Path (Join-Path $env:ProgramW6432 'WindowsApps') $fullName } else { '' }
    PackageUserInformation = @(@{UserSecurityId=$ownerId;InstallState=$ownerState})
  }
}
function Get-AppxVolume {
  param([switch]$Online, $ErrorAction)
  @{ Name='\\?\Volume{11111111-1111-4111-8111-111111111111}\'; PackageStorePath=(Join-Path $env:ProgramW6432 'WindowsApps'); IsOffline=$false; IsSystemVolume=$true }
}
function Get-AppxProvisionedPackage { param([switch]$Online, $ErrorAction); @() }
function Remove-AppxPackage {
  param($Package, [switch]$AllUsers, $ErrorAction)
  $script:removeCalls++
  if (-not $AllUsers -or $Package -ne 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0') {
    throw 'fixture rejected broad or wrong-identity removal'
  }
  $script:machinePresent = $false
}
"#;

    #[cfg(windows)]
    #[test]
    fn machine_state_probe_distinguishes_exact_empty_location_orphan() {
        let production = msix_machine_state_script(None).unwrap();
        let json = run_powershell_json(&format!("{MACHINE_ORPHAN_MOCKS}\n{production}"))
            .expect("run machine orphan probe fixture");
        let state: MsixMachineStateReport = serde_json::from_str(&json).unwrap();
        assert!(state.is_orphan_candidate(), "{state:?}");
        assert_eq!(state.path_state.as_deref(), Some("empty"));
        assert_eq!(
            state.package_full_name.as_deref(),
            Some("OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0")
        );
    }

    #[cfg(windows)]
    #[test]
    fn machine_state_extracts_sid_from_appx_user_security_id_object() {
        let production = msix_machine_state_script(None).unwrap();
        let script = format!("{MACHINE_ORPHAN_MOCKS}\n$script:mode='wrapped-sid'\n{production}");
        let json = run_powershell_json(&script).expect("run wrapped AppX SID fixture");
        let state: MsixMachineStateReport = serde_json::from_str(&json).unwrap();
        assert!(state.is_orphan_candidate(), "{state:?}");
        assert_eq!(state.owner_states, vec!["S-1-5-18:Staged"]);
    }

    #[cfg(windows)]
    #[test]
    fn machine_state_extracts_sid_from_security_identifier_projection() {
        let production = msix_machine_state_script(None).unwrap();
        let script = format!("{MACHINE_ORPHAN_MOCKS}\n$script:mode='wrapped-value'\n{production}");
        let json = run_powershell_json(&script).expect("run SecurityIdentifier fixture");
        let state: MsixMachineStateReport = serde_json::from_str(&json).unwrap();
        assert!(state.is_orphan_candidate(), "{state:?}");
        assert_eq!(state.owner_states, vec!["S-1-5-18:Staged"]);
    }

    #[cfg(windows)]
    #[test]
    fn machine_state_accepts_one_publicly_proven_staged_volume() {
        let production = msix_machine_state_script(None).unwrap();
        let script = format!(
            r#"{MACHINE_ORPHAN_MOCKS}
$script:rehydrated=$true
$path=Join-Path (Join-Path $env:ProgramW6432 'WindowsApps') 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
[IO.Directory]::CreateDirectory($path) | Out-Null
{production}"#
        );
        let json = run_powershell_json(&script).expect("run proven staged-volume fixture");
        let state: MsixMachineStateReport = serde_json::from_str(&json).unwrap();
        assert_eq!(
            state.classification,
            msix_machine_state::STAGED_CANDIDATE,
            "{state:?}"
        );
        assert_eq!(
            state.record_volume_name.as_deref(),
            Some(r"\\?\Volume{11111111-1111-4111-8111-111111111111}\")
        );
        assert!(state
            .record_volume_store
            .as_deref()
            .is_some_and(|path| path.ends_with("WindowsApps")));
    }

    #[cfg(windows)]
    #[test]
    fn machine_orphan_cleanup_uses_exact_all_users_api_and_proves_absence() {
        let production = remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            TEST_AUTHORIZED_SHA256,
            false,
            None,
            None,
        )
        .unwrap();
        let script = format!(
            "{MACHINE_ORPHAN_MOCKS}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{report=$report;calls=$script:removeCalls;present=$script:machinePresent}} | ConvertTo-Json -Depth 8 -Compress"
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], true, "{value:?}");
        assert_eq!(value["calls"], 1);
        assert_eq!(value["present"], false);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("supported AppX APIs"));
    }

    #[cfg(windows)]
    #[test]
    fn machine_orphan_cleanup_rehydrates_missing_bytes_then_retries_supported_remove() {
        let production = remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            TEST_AUTHORIZED_SHA256,
            false,
            Some(Path::new("C:\\")),
            Some(r"\\?\Volume{22222222-2222-4222-8222-222222222222}\"),
        )
        .unwrap()
        .replace(
            "$null = Invoke-CodexNativeMachineRemoval $expectedFullName",
            "throw 'fixture native remove reports missing package files'",
        )
        .replace(
            "try {\n  $before = Get-CodexMachineCleanupState $expectedFullName",
            r#"
function Start-Sleep { param($Milliseconds) }
function Get-AppxVolume {
  param($Path,[switch]$Online,$ErrorAction)
  @(
    @{Name='\\?\Volume{11111111-1111-4111-8111-111111111111}\';PackageStorePath=(Join-Path $env:ProgramW6432 'WindowsApps');IsOffline=$false;IsSystemVolume=$true},
    @{Name='\\?\Volume{22222222-2222-4222-8222-222222222222}\';PackageStorePath='C:\WindowsApps';IsOffline=$false;IsSystemVolume=$false}
  )
}
function Get-CodexRecoveryContext {
  param($UsbRoot,$UsbVolume)
  @{usbRoot='C:\';usbName=$UsbVolume.Name;usbId='22222222-2222-4222-8222-222222222222';systemName='\\?\Volume{11111111-1111-4111-8111-111111111111}\';systemId='11111111-1111-4111-8111-111111111111';systemRoot='C:\';sid='S-1-5-21-1-2-3-1001'}
}
function Invoke-CodexRehydrateMachinePackage {
  param($PackagePath,$ExpectedSha256,$FullName,$Context)
  if ($ExpectedSha256 -ne $packageSha256) { throw 'fixture rehydration hash authorization changed' }
  $script:rehydrated=$true
  $location=Join-Path (Join-Path $env:ProgramW6432 'WindowsApps') $FullName
  [IO.Directory]::CreateDirectory($location) | Out-Null
  @{activityId='fixture-rehydrate';recordVolumeName='system';recordVolumeStore=(Join-Path $env:ProgramW6432 'WindowsApps');recordVolumeSource='fixture'}
}
function Remove-AppxPackage {
  param($Package,[switch]$AllUsers,$ErrorAction)
  $script:removeCalls++
  if (-not $AllUsers -or $Package -ne 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0') { throw 'fixture rejected removal identity' }
  if (-not $script:rehydrated) { throw '0x80073CFA fixture linked package data' }
  $location=Join-Path (Join-Path $env:ProgramW6432 'WindowsApps') $Package
  if ([IO.Directory]::Exists($location)) { [IO.Directory]::Delete($location,$true) }
  $script:machinePresent=$false
}
try {
  $before = Get-CodexMachineCleanupState $expectedFullName"#,
        );
        let script = format!(
            "{MACHINE_ORPHAN_MOCKS}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{report=$report;calls=$script:removeCalls;rehydrated=$script:rehydrated;present=$script:machinePresent}} | ConvertTo-Json -Depth 10 -Compress"
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], true, "{value}");
        assert_eq!(value["rehydrated"], true, "{value}");
        assert_eq!(value["present"], false, "{value}");
        assert!(value["calls"].as_i64().unwrap() >= 2, "{value}");
        assert!(
            value["report"]["notes"]
                .to_string()
                .contains("StagePackageAsync"),
            "{value}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn machine_orphan_cleanup_blocks_installed_owner_and_payload_mismatch() {
        for mode in ["installed-owner", "mismatch"] {
            let production = remove_exact_msix_machine_orphan_script(
                "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
                Path::new(r"C:\verified.msix"),
                TEST_AUTHORIZED_SHA256,
                false,
                None,
                None,
            )
            .unwrap();
            let script = format!(
                "{MACHINE_ORPHAN_MOCKS}\n$script:mode={mode}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{report=$report;calls=$script:removeCalls}} | ConvertTo-Json -Depth 8 -Compress",
                mode = ps_quote(mode)
            );
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert_eq!(value["report"]["success"], false, "{mode}: {value:?}");
            assert_eq!(value["calls"], 0, "{mode}: {value:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn machine_orphan_cleanup_never_claims_success_while_system_entry_remains() {
        let production = remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            TEST_AUTHORIZED_SHA256,
            false,
            None,
            None,
        )
        .unwrap()
        .replace("$attempt -lt 40", "$attempt -lt 1");
        let mock_system_entry = r#"
function Get-Item {
  param($LiteralPath, [switch]$Force, $ErrorAction)
  @{PSIsContainer=$true;Attributes=[IO.FileAttributes]::Directory;LinkType='';Target=@()}
}
"#;
        let script = format!(
            "{MACHINE_ORPHAN_MOCKS}\n{mock_system_entry}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{report=$report;calls=$script:removeCalls}} | ConvertTo-Json -Depth 8 -Compress"
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value:?}");
        assert_eq!(value["calls"], 1);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("Windows-managed package entry remains"));
    }

    #[cfg(windows)]
    #[test]
    fn pending_volume_can_reconcile_an_already_absent_machine_projection() {
        let production = remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            TEST_AUTHORIZED_SHA256,
            false,
            None,
            None,
        )
        .unwrap()
        .replace(
            "$null = Invoke-CodexNativeMachineRemoval $expectedFullName",
            "$script:nativeCalls++; $null = $null",
        )
        .replace("exit 0", "return");
        let script = format!(
            "{MACHINE_ORPHAN_MOCKS}\n$script:machinePresent=$false\n$script:nativeCalls=0\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{report=$report;nativeCalls=$script:nativeCalls;removeCalls=$script:removeCalls}} | ConvertTo-Json -Depth 8 -Compress"
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], true, "{value:?}");
        assert_eq!(value["nativeCalls"], 1, "{value:?}");
        assert_eq!(value["removeCalls"], 0, "{value:?}");
    }

    #[cfg(windows)]
    #[test]
    fn machine_orphan_cleanup_contains_only_supported_package_mutations() {
        let script = remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            TEST_AUTHORIZED_SHA256,
            true,
            Some(Path::new("E:\\")),
            Some(r"\\?\Volume{fixture}"),
        )
        .unwrap();
        assert!(remove_exact_msix_machine_orphan_script(
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
            Path::new(r"C:\verified.msix"),
            "not-a-sha256",
            true,
            Some(Path::new("E:\\")),
            Some(r"\\?\Volume{fixture}"),
        )
        .is_err());
        assert!(script.contains("Remove-AppxPackage -Package $expectedFullName -AllUsers"));
        assert!(script.contains("RemovePackageAsync($fullName, $options)"));
        assert!(script.contains("RemovalOptions]::RemoveForAllUsers"));
        assert!(script.contains("StagePackageAsync($uri, $null, $options, $nativeVolume)"));
        assert!(script.contains("MovePackageToVolumeAsync("));
        assert!(script.contains("NativeFiles]::Hash($payloadStream)"));
        assert!(script.contains(TEST_AUTHORIZED_SHA256));
        assert!(script.contains(
            "Backup-CodexPackageData $packageDataIdentity $authorizedVolumeRoot $authorizedVolumeId"
        ));
        assert!(script.contains(
            "Assert-CodexBackupStillMatches $package $backupPath $authorizedVolumeRoot $authorizedVolumeId"
        ));
        for forbidden in [
            r"AppModel\PackageRepository",
            "Remove-ItemProperty",
            "takeown.exe",
            "icacls.exe",
        ] {
            assert!(
                !script.contains(forbidden),
                "machine cleanup must not contain {forbidden}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_signed_package_avoids_unsupported_preserve_switch() {
        let production = remove_msix_package_script(true).replace(
            "$backupPath = Backup-CodexPackageData $p",
            "$backupPath = $null",
        );
        let script = format!("{UNINSTALL_MOCKS}\nfunction Test-Path {{ param($LiteralPath, $ErrorAction); $false }}\n{production}");
        let report: MsixRemoveReport =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert!(report.success, "{report:?}");
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("No package-local data")));
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_handles_missing_switch_and_explicit_development_flag_rejection() {
        for missing_switch in [false, true] {
            let production = remove_msix_package_script(true).replace(
                "$backupPath = Backup-CodexPackageData $p",
                "$backupPath = $null",
            );
            let setup = if missing_switch {
                "function Remove-AppxPackage { param($Package, $ErrorAction); $script:removeCalls++; $script:removed = $true }"
            } else {
                "$script:developmentMode = $true"
            };
            let script = format!("{UNINSTALL_MOCKS}\n{setup}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{ report = $report; calls = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress");
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert_eq!(value["report"]["success"], true, "{value:?}");
            assert_eq!(value["calls"], if missing_switch { 1 } else { 2 });
        }
    }

    #[cfg(windows)]
    #[test]
    fn backup_destination_junction_blocks_removal() {
        let root =
            std::env::temp_dir().join(format!("codex-backup-link-test-{}", uuid::Uuid::new_v4()));
        let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        std::fs::create_dir_all(&source).unwrap();
        let production = remove_msix_package_script(true);
        let setup = r#"
function Get-Item {
  param($LiteralPath, [switch]$Force, $ErrorAction)
  if ($LiteralPath -like '*\CodexAppxDataBackups') {
    return @{ Attributes = [IO.FileAttributes]::ReparsePoint; PSIsContainer = $true }
  }
  Microsoft.PowerShell.Management\Get-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop
}
"#;
        let script = format!("{UNINSTALL_MOCKS}\n$env:LOCALAPPDATA = {}\n{setup}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{ report = $report; removes = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress", ps_quote(&root.to_string_lossy()));
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["removes"], 0);
        assert!(
            value["report"]["message"]
                .as_str()
                .unwrap()
                .contains("link/junction"),
            "{value:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn package_backup_copies_nonempty_contained_ac_junction_as_plain_data() {
        let root =
            std::env::temp_dir().join(format!("codex-contained-ac-{}", uuid::Uuid::new_v4()));
        let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        let target = source.join("LocalState");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("valuable #100%.txt"),
            b"preserve even cache content",
        )
        .unwrap();
        let link = source.join("AC");
        let script = format!(
            r#"
$env:LOCALAPPDATA = {root}
New-Item -ItemType Junction -Path {link} -Target {target} -ErrorAction Stop | Out-Null
try {{
{backup_script}
  $backup = Backup-CodexPackageData @{{ PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0' }}
  @{{ backup = $backup; placement = Get-CodexPackageDataPlacement 'E:\' }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete({link}) }}
"#,
            root = ps_quote(&root.to_string_lossy()),
            link = ps_quote(&link.to_string_lossy()),
            target = ps_quote(&target.to_string_lossy()),
            backup_script = super::APPX_DATA_BACKUP_SCRIPT
        );
        let json = run_powershell_json(&script).unwrap();
        let result: serde_json::Value = serde_json::from_str(&json).unwrap();
        let backup = Path::new(result["backup"].as_str().unwrap());
        assert_eq!(
            result["placement"]["state"], "contained-links-verified",
            "{result}"
        );
        assert_eq!(
            std::fs::read(backup.join("AC").join("valuable #100%.txt")).unwrap(),
            b"preserve even cache content"
        );
        assert_eq!(
            std::fs::read(target.join("valuable #100%.txt")).unwrap(),
            b"preserve even cache content"
        );
        assert!(!std::fs::symlink_metadata(backup.join("AC"))
            .unwrap()
            .file_type()
            .is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn package_backup_rejects_external_and_cyclic_ac_links_without_touching_targets() {
        for cycle in [false, true] {
            let root =
                std::env::temp_dir().join(format!("codex-ac-boundary-{}", uuid::Uuid::new_v4()));
            let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
            let target = if cycle {
                source.clone()
            } else {
                root.join("unrelated-user-data")
            };
            std::fs::create_dir_all(&source).unwrap();
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("important.txt"), b"do not alter").unwrap();
            let link = source.join("AC");
            let production = remove_msix_package_script(true);
            let script = format!(
                r#"
{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {root}
New-Item -ItemType Junction -Path {link} -Target {target} -ErrorAction Stop | Out-Null
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{ report = $report; removes = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete({link}) }}
"#,
                root = ps_quote(&root.to_string_lossy()),
                link = ps_quote(&link.to_string_lossy()),
                target = ps_quote(&target.to_string_lossy())
            );
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert_eq!(value["report"]["success"], false, "{value}");
            assert_eq!(value["removes"], 0, "{value}");
            assert!(
                value["report"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("escapes the package root or forms a cycle"),
                "{value}"
            );
            assert_eq!(
                std::fs::read(target.join("important.txt")).unwrap(),
                b"do not alter"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_new_wrong_volume_registration_moves_once_and_is_verified() {
        let value = strict_usb_mock_report(
            r#"
function Add-AppxPackage {
 param($Path, $Volume, $ErrorAction)
 $script:addCalls++
 $script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'
}
"#,
        );
        assert_eq!(value["report"]["success"], true, "{value}");
        assert_eq!(value["adds"], 1);
        assert_eq!(value["moves"], 1);
        assert_eq!(value["removes"], 0);
        assert_eq!(value["location"], r"E:\WindowsApps\OpenAI.Codex");
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_orphan_repair_uses_only_supported_package_apis() {
        let script = super::install_msix_script_with_volume_and_identity(
            Path::new(r"C:\staging\OpenAI.Codex_x64.msix"),
            Some(Path::new(r"E:\")),
            "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
        );
        assert!(script.contains(
            "Remove-AppxPackage -Package $expectedPackageFullName -AllUsers -ErrorAction Stop"
        ));
        assert!(script.contains("RemovePackageAsync($fullName, $options)"));
        assert!(
            script.contains("[Windows.Management.Deployment.RemovalOptions]::RemoveForAllUsers")
        );
        assert!(script.contains("Assert-CodexBackupStillMatches"));
        assert!(script.contains("MachineRepairRequired"));
        assert!(script.contains("StagePackageAsync($uri, $null, $options, $nativeVolume)"));
        assert!(script.contains("MovePackageToVolumeAsync("));
        assert!(script.contains("NativeFiles]::Hash($payloadStream)"));
        assert!(!script.contains(r"AppModel\PackageRepository"));
        assert!(!script.contains("Remove-ItemProperty"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_provisioned_or_other_user_package_is_never_moved() {
        for setup in [
            r#"
function Get-AppxProvisionedPackage {
 param([switch]$Online, $ErrorAction)
 @{ DisplayName = 'OpenAI.Codex'; PackageName = $script:observedFullName }
}
"#,
            r#"
function Get-AppxPackage {
 param($Name, [switch]$AllUsers, $ErrorAction)
 if ($AllUsers -or $script:registeredLocation) {
  @{ PackageFullName = $script:observedFullName; PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'; InstallLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'; Version = '26.825.5331.0'; PackageUserInformation = @(@{ UserSecurityId = 'S-1-5-21-123-456-789-1002'; InstallState = 'Installed' }) }
 }
}
"#,
        ] {
            let value = strict_usb_mock_report(&format!(
                r#"
{setup}
function Add-AppxPackage {{
 param($Path, $Volume, $ErrorAction)
 $script:addCalls++
 $script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'
}}
"#
            ));
            assert_eq!(value["report"]["success"], false, "{value}");
            assert_eq!(value["moves"], 0, "{value}");
            assert!(
                value["report"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("preexistingSharedOrProvisionedPackage=True"),
                "{value}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_shared_snapshot_failure_is_not_absence() {
        let value = strict_usb_mock_report(
            r#"
function Get-AppxProvisionedPackage { param([switch]$Online, $ErrorAction); throw 'provisioned query denied' }
"#,
        );
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["adds"], 0);
        assert_eq!(value["moves"], 0);
        assert_eq!(value["removes"], 0);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("provisioned query denied"));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_existing_current_user_package_moves_without_system_add() {
        let value = strict_usb_mock_report(
            r"$script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'",
        );
        assert_eq!(value["report"]["success"], true, "{value}");
        assert_eq!(value["moves"], 1);
        assert_eq!(value["removes"], 0);
        // The one Add updates/verifies the exact payload on the USB volume;
        // the mock rejects any non-USB target.
        assert_eq!(value["adds"], 1);
        assert_eq!(value["location"], r"E:\WindowsApps\OpenAI.Codex");
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_fresh_registration_still_backs_up_concurrent_user_content() {
        let root =
            std::env::temp_dir().join(format!("codex-concurrent-data-{}", uuid::Uuid::new_v4()));
        let source = root
            .join("Packages")
            .join("OpenAI.Codex_2p2nqsd0c76g0")
            .join("LocalState");
        // The root is absent until the mocked deployment creates and writes it.
        assert!(!source.exists());
        let setup = format!(
            r#"
$env:LOCALAPPDATA = {root}
$script:moveError = 'move unsupported in this fixture'
function Add-AppxPackage {{
 param($Path, $Volume, $ErrorAction)
 $script:addCalls++
 [IO.Directory]::CreateDirectory({source}) | Out-Null
 [IO.File]::WriteAllText((Join-Path {source} 'new-session.txt'), 'user content written during install')
 $script:registeredLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'
}}
"#,
            root = ps_quote(&root.to_string_lossy()),
            source = ps_quote(&source.to_string_lossy())
        );
        let value = strict_usb_mock_report(&setup);
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["removes"], 1, "{value}");
        let backups = root
            .join("CodexAppxDataBackups")
            .join("OpenAI.Codex_2p2nqsd0c76g0");
        let backup = std::fs::read_dir(backups)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(backup.join("LocalState").join("new-session.txt")).unwrap(),
            "user content written during install"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("new-session.txt")).unwrap(),
            "user content written during install"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn package_backup_rejects_alias_target_changes_during_copy() {
        let root =
            std::env::temp_dir().join(format!("codex-alias-change-{}", uuid::Uuid::new_v4()));
        let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        for dir in ["LocalState", "LocalCache"] {
            std::fs::create_dir_all(source.join(dir)).unwrap();
            std::fs::write(
                source.join(dir).join("content.txt"),
                b"same bytes, different identity",
            )
            .unwrap();
        }
        let link = source.join("AC");
        let production = remove_msix_package_script(true).replace(
            "Copy-CodexBackupFile $entryPath $destination",
            "Copy-CodexBackupFile $entryPath $destination; Invoke-FixtureAfterCopy",
        );
        let script = format!(
            r#"
{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {root}
New-Item -ItemType Junction -Path {link} -Target {first} -ErrorAction Stop | Out-Null
$script:changed = $false
function Invoke-FixtureAfterCopy {{
 if (-not $script:changed) {{
   $script:changed = $true
   [IO.Directory]::Delete({link})
   New-Item -ItemType Junction -Path {link} -Target {second} -ErrorAction Stop | Out-Null
 }}
}}
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{ report = $report; removes = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete({link}) }}
"#,
            root = ps_quote(&root.to_string_lossy()),
            link = ps_quote(&link.to_string_lossy()),
            first = ps_quote(&source.join("LocalState").to_string_lossy()),
            second = ps_quote(&source.join("LocalCache").to_string_lossy())
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["removes"], 0);
        assert!(
            value["report"]["message"]
                .as_str()
                .unwrap()
                .contains("changed during backup"),
            "{value}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn capability_registry_errors_differ_from_missing_policy() {
        let missing = capabilities_from_probe_json(
            &serde_json::json!({ "allowAllTrustedAppsState": "notConfigured", "developerModeState": "value", "developerModeValue": 0 }),
        );
        let denied = capabilities_from_probe_json(
            &serde_json::json!({ "allowAllTrustedAppsState": "readError", "allowAllTrustedAppsError": "access denied" }),
        );
        assert!(missing
            .sideload_policy
            .detail
            .contains("not an explicit policy denial"));
        assert!(denied.sideload_policy.detail.contains("access denied"));
        assert!(missing
            .notes
            .iter()
            .any(|note| note.contains("Developer mode probe: state=value, value=0")));
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_unverified_guid_multihop_and_unknown_data_links_keep_package_but_fail_install() {
        for kind in ["guid", "multihop", "unknown"] {
            let root =
                std::env::temp_dir().join(format!("codex-data-placement-{}", uuid::Uuid::new_v4()));
            let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
            std::fs::create_dir_all(&source).unwrap();
            let setup = format!(
                r#"
$env:LOCALAPPDATA = {root}
$script:dataSource = {source}
$script:dataLinkKind = {kind}
function Get-ChildItem {{
 param($LiteralPath, [switch]$Force, $ErrorAction)
 if ($LiteralPath -eq $script:dataSource) {{
   @{{ FullName = (Join-Path $script:dataSource 'AC'); PSIsContainer = $true; Attributes = [IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint; LinkType = $(if ($script:dataLinkKind -eq 'unknown') {{ 'Unknown' }} else {{ 'Junction' }}); Target = @($(if ($script:dataLinkKind -eq 'guid') {{ '\\?\Volume{{usb-volume-guid}}\WpSystem\data' }} else {{ Join-Path $script:dataSource 'middle' }})) }}
 }} else {{ Microsoft.PowerShell.Management\Get-ChildItem -LiteralPath $LiteralPath -Force -ErrorAction Stop }}
}}
function Get-Item {{
 param($LiteralPath, [switch]$Force, $ErrorAction)
 if ($LiteralPath -eq (Join-Path $script:dataSource 'middle')) {{
   @{{ FullName = $LiteralPath; PSIsContainer = $true; Attributes = [IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint; LinkType = 'Junction'; Target = @('E:\WpSystem\external-data') }}
 }} else {{ Microsoft.PowerShell.Management\Get-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop }}
}}
"#,
                root = ps_quote(&root.to_string_lossy()),
                source = ps_quote(&source.to_string_lossy()),
                kind = ps_quote(kind)
            );
            let value = strict_usb_mock_report(&setup);
            assert_eq!(value["report"]["success"], false, "{kind}: {value}");
            assert_eq!(value["adds"], 1);
            assert_eq!(value["removes"], 0, "USB-placed package must be retained");
            assert_eq!(value["location"], r"E:\WindowsApps\OpenAI.Codex");
            assert!(
                value["report"]["message"].as_str().unwrap().contains(
                    "local data placement could not be verified; installation is incomplete"
                ),
                "{kind}: {value}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn package_data_on_usb_without_links_is_not_local_success() {
        let script = format!(
            r#"
$env:LOCALAPPDATA = 'E:\fixture-user-data'
$env:SystemDrive = 'C:'
{backup_script}
Get-CodexPackageDataPlacement 'E:\' | ConvertTo-Json -Depth 8 -Compress
"#,
            backup_script = super::APPX_DATA_BACKUP_SCRIPT
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["state"], "volume-unverified", "{value}");
    }

    #[cfg(windows)]
    fn wpsystem_fixture_script(mode: &str) -> (std::path::PathBuf, String) {
        let root =
            std::env::temp_dir().join(format!("codex-wpsystem-backup-{}", uuid::Uuid::new_v4()));
        let local = root.join("local");
        let source = local.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        std::fs::create_dir_all(&source).unwrap();
        let script = format!(
            r#"
{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {local}
$script:fixtureRoot = {root}
$script:fixtureMode = {mode}
$script:sourceRoot = {source}
$script:sourceLink = Join-Path $script:sourceRoot 'AC'
$script:volumeId = '\\?\Volume{{12345678-1111-2222-3333-444444444444}}\'
$script:physicalVolumeRoot = Join-Path $script:fixtureRoot 'volume'
$sid = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$script:logicalTarget = "R:\WpSystem\$sid\AppData\Local\Packages\OpenAI.Codex_2p2nqsd0c76g0\AC"
function Convert-FixturePath([string]$path) {{
  if ($path.StartsWith('R:\', [StringComparison]::OrdinalIgnoreCase)) {{ return Join-Path $script:physicalVolumeRoot $path.Substring(3) }}
  return $path
}}
function Join-Path {{
  param($Path, $ChildPath)
  if ([string]$Path -like 'R:\*') {{ return [IO.Path]::Combine([string]$Path, [string]$ChildPath) }}
  Microsoft.PowerShell.Management\Join-Path -Path $Path -ChildPath $ChildPath
}}
$script:physicalTarget = Convert-FixturePath $script:logicalTarget
[IO.Directory]::CreateDirectory((Join-Path $script:physicalTarget 'Temp')) | Out-Null
[IO.File]::WriteAllText((Join-Path $script:physicalTarget 'Temp\valuable.txt'), 'valuable AC state must survive')
New-Item -ItemType Junction -Path $script:sourceLink -Target $script:physicalTarget -ErrorAction Stop | Out-Null
if ($script:fixtureMode -eq 'wrongSid') {{ $script:logicalTarget = $script:logicalTarget.Replace($sid, 'S-1-5-21-1-2-3-9999') }}
if ($script:fixtureMode -eq 'wrongPfn') {{ $script:logicalTarget = $script:logicalTarget.Replace('2p2nqsd0c76g0', 'otherpublisher') }}
if ($script:fixtureMode -eq 'wrongLeaf') {{ $script:logicalTarget = $script:logicalTarget.TrimEnd('C') + 'ppData' }}
function Get-Item {{
  param($LiteralPath, [switch]$Force, $ErrorAction)
  if ($script:fixtureMode -eq 'disconnectedTarget' -and $LiteralPath -eq $script:logicalTarget) {{ throw 'fixture WpSystem target unavailable' }}
  if ($script:fixtureMode -eq 'secondHop' -and $LiteralPath -eq 'R:\WpSystem') {{
    return @{{ Attributes = [IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint; PSIsContainer = $true }}
  }}
  $entry = Microsoft.PowerShell.Management\Get-Item -LiteralPath (Convert-FixturePath $LiteralPath) -Force -ErrorAction Stop
  if ($LiteralPath -eq $script:sourceLink) {{
    return @{{FullName=$script:sourceLink;Name='AC';PSIsContainer=$entry.PSIsContainer;Attributes=$entry.Attributes;LinkType=$entry.LinkType;Target=@($script:logicalTarget)}}
  }}
  $entry
}}
function Get-ChildItem {{
  param($LiteralPath, [switch]$Force, $ErrorAction)
  foreach ($item in @(Microsoft.PowerShell.Management\Get-ChildItem -LiteralPath (Convert-FixturePath $LiteralPath) -Force -ErrorAction Stop)) {{
    $fullName = [string]$item.FullName
    if ($LiteralPath.StartsWith('R:\', [StringComparison]::OrdinalIgnoreCase)) {{ $fullName = [IO.Path]::Combine($LiteralPath, $item.Name) }}
    $targets = @($item.Target)
    if ($fullName -eq $script:sourceLink) {{ $targets = @($script:logicalTarget) }}
    @{{ FullName = $fullName; Name = $item.Name; PSIsContainer = $item.PSIsContainer; Attributes = $item.Attributes; LinkType = [string]$item.LinkType; Target = $targets }}
  }}
}}
function Get-AppxVolume {{
  param($Path, [switch]$Online, $ErrorAction)
  if ($script:fixtureMode -eq 'offline') {{ throw 'fixture USB is disconnected' }}
  if ($Path -ne 'R:\') {{ throw 'unexpected AppX volume query' }}
  @{{ Name = $script:volumeId; PackageStorePath = 'R:\WindowsApps'; IsOffline = $false }}
}}
function Get-Volume {{
  param($DriveLetter, $ErrorAction)
  $script:volumeReads++
  if ($DriveLetter -ne 'R') {{ throw 'unexpected physical volume query' }}
  $id = if ($script:fixtureMode -eq 'wrongVolume' -or ($script:fixtureMode -eq 'lateVolumeChange' -and $script:volumeReads -ge 3)) {{ '\\?\Volume{{aaaaaaaa-1111-2222-3333-444444444444}}\' }} else {{ $script:volumeId }}
  @{{ FileSystem = 'NTFS'; UniqueId = $id; Path = $id }}
}}
$script:copied = $false
function Invoke-FixtureAfterCopy {{
  if (-not $script:copied) {{
    $script:copied = $true
    if ($script:fixtureMode -eq 'swappedVolume') {{ $script:volumeId = '\\?\Volume{{bbbbbbbb-1111-2222-3333-444444444444}}\' }}
    if ($script:fixtureMode -eq 'swappedTarget') {{ $script:logicalTarget = $script:logicalTarget.Replace('\AC', '\LocalState') }}
  }}
}}
"#,
            local = ps_quote(&local.to_string_lossy()),
            root = ps_quote(&root.to_string_lossy()),
            source = ps_quote(&source.to_string_lossy()),
            mode = ps_quote(mode)
        );
        (root, script)
    }

    #[cfg(windows)]
    fn project_wpsystem_fixture_io(production: String) -> String {
        production
            .replace(
                "[CodexAppxRecovery.NativeFiles]::OpenRead($path)",
                "[CodexAppxRecovery.NativeFiles]::OpenRead((Convert-FixturePath $path))",
            )
            .replace(
                "[CodexAppxRecovery.NativeFiles]::OpenRead($source)",
                "[CodexAppxRecovery.NativeFiles]::OpenRead((Convert-FixturePath $source))",
            )
            .replace(
                "[CodexAppxRecovery.NativeFiles]::Entries($path)",
                "[CodexAppxRecovery.NativeFiles]::Entries((Convert-FixturePath $path))",
            )
            .replace(
                "Copy-CodexBackupFile $entryPath $destination",
                "Copy-CodexBackupFile $entryPath $destination; Invoke-FixtureAfterCopy",
            )
    }

    #[cfg(windows)]
    #[test]
    fn explicit_data_volume_production_flow_repairs_wpsystem_and_is_idempotent() {
        use std::io::Write;
        for mode in [
            "fresh",
            "orphan",
            "orphan-native-fallback",
            "orphan-remove-fails",
            "orphan-rehydrate-fails",
            "orphan-removal-interrupted",
            "late-orphan-prepared-resume",
            "repair",
            "already-local",
            "update-local",
            "update-from-system",
            "update-wpsystem",
            "update-downgrade-from-system",
            "update-unknown-location",
            "update-running-from-system",
        ] {
            let (root, setup) = wpsystem_fixture_script("valid");
            let payload = root.join("Codex verified.msix");
            let mut archive = zip::ZipWriter::new(std::fs::File::create(&payload).unwrap());
            archive
                .start_file("app.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(b"authenticated payload boundary mocked by this fixture")
                .unwrap();
            archive.finish().unwrap();
            // Keep the real production dispatcher, backup, volume checks,
            // journal, native file pins and restoration. Only deployment and
            // machine observation are mocked; no real AppX/HKLM mutations.
            let native_mocks = r#"
function Get-CodexRecoveryUsbFreeBytes { param($Context); return [uint64]100GB }
function Invoke-CodexRecoveryStage {
 param($PackagePath, $Context)
 if ($Context.usbRoot -ne 'R:\') { throw 'fixture stage escaped USB' }
 $script:stageCalls++; $script:staged = $true; $script:stagedFullName = $script:fullName
 @{ activityId='fixture-stage'; operation='Stage USB mock' }
}
function Get-CodexRecoveryStagedPackage {
 param($FullName, $Context)
 if ($script:staged -and $script:stagedFullName -eq $FullName) { New-FixtureRegisteredPackage $FullName 'R:' }
}
function Invoke-CodexRecoveryRegister {
 param($FullName, $Context)
 if ($Context.systemRoot -ne ($env:SystemDrive + '\')) { throw 'fixture register did not select system data volume' }
 $script:registerCalls++; $script:registered = $true; $script:registeredFullName = $FullName; $script:registeredRoot = 'R:'
 [IO.Directory]::CreateDirectory((Join-Path $script:sourceRoot 'LocalState')) | Out-Null
 [IO.File]::WriteAllText((Join-Path $script:sourceRoot 'LocalState\bootstrap.txt'), 'fresh Windows-created state')
 @{ activityId='fixture-register'; operation='Register system data mock' }
}
function Invoke-CodexRehydrateMachinePackage {
 param($PackagePath,$ExpectedSha256,$FullName,$Context)
 if ($ExpectedSha256 -notmatch '^[a-f0-9]{64}$' -or $FullName -ne $script:fullName -or $Context.usbRoot -ne 'R:\') { throw 'fixture rehydration escaped its exact authorization' }
 if ($script:mode -eq 'orphan-rehydrate-fails') { throw 'fixture exact-payload rehydration failed' }
 $script:rehydrateCalls++; $script:ghost=$false; $script:staged=$true; $script:stagedFullName=$FullName
 @{activityId='fixture-rehydrate';recordVolumeName='fixture-system';recordVolumeStore='C:\Program Files\WindowsApps';recordVolumeSource='fixture';operation='fixture exact rehydrate'}
}
function Invoke-CodexMoveRehydratedPackageToUsb {
 param($FullName,$Context)
 if ($FullName -ne $script:fullName -or $Context.usbRoot -ne 'R:\') { throw 'fixture move escaped its exact authorization' }
 $script:rehydrateMoveCalls++; $script:ghost=$false; $script:staged=$true; $script:stagedFullName=$FullName
 @{activityId='fixture-rehydrate-move';operation='fixture move USB'}
}
"#;
            let production =
                project_wpsystem_fixture_io(super::install_msix_script_with_volume_and_identity(
                    &payload,
                    Some(Path::new("R:\\")),
                    "OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
                ))
                .replace(
                    "  $useExplicitAppDataVolume = $true",
                    &format!("{native_mocks}\n  $useExplicitAppDataVolume = $true"),
                )
                .replace(
                    "$null = Invoke-CodexNativeOrphanRemoval $expectedPackageFullName",
                    "$null = Invoke-FixtureNativeOrphanRemoval $expectedPackageFullName",
                )
                .replace(
                    "      catch { $localCmdletError = [string]$_ }\n      $afterRemove = Wait-CodexMachineRepairState",
                    "      catch { $localCmdletError = [string]$_ }\n      if ($script:mode -eq 'orphan-removal-interrupted' -and $script:removeCalls -eq 1) { throw 'fixture process terminated before AppX removal requery' }\n      $afterRemove = Wait-CodexMachineRepairState",
                )
                // APPX_DATA_BACKUP_SCRIPT is included verbatim from a CRLF
                // PowerShell file. Anchor on the assertion line itself so the
                // interruption hook remains effective if the source line
                // ending changes again; a LF-only multiline replacement would
                // silently leave this recovery-resume case untested.
                .replace(
                    "    Assert-CodexRecoveryJournal $journal $context $ExpectedFullName $payloadHash",
                    "    Invoke-FixtureAfterInitialJournal\r\n    Assert-CodexRecoveryJournal $journal $context $ExpectedFullName $payloadHash",
                );
            assert!(
                production.contains(
                    "    Invoke-FixtureAfterInitialJournal\r\n    Assert-CodexRecoveryJournal"
                ),
                "late-orphan interruption hook was not injected into the production fixture"
            );
            let script = format!(
                r#"{setup}
$script:mode = {mode}
$script:fullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
$script:registeredFullName = if ($script:mode.StartsWith('update-')) {{'OpenAI.Codex_26.800.1000.0_x64__2p2nqsd0c76g0'}} else {{$script:fullName}}
if ($script:mode -eq 'update-downgrade-from-system') {{$script:registeredFullName='OpenAI.Codex_27.0.0.0_x64__2p2nqsd0c76g0'}}
$script:registeredRoot = if ($script:mode.EndsWith('from-system')) {{$env:SystemDrive}} elseif ($script:mode -eq 'update-unknown-location') {{'not-a-drive'}} else {{'R:'}}
$script:stagedFullName = $script:registeredFullName
 $script:stageCalls = 0; $script:registerCalls = 0; $script:removeCalls = 0; $script:nativeRemoveCalls = 0; $script:moveCalls = 0; $script:rehydrateCalls = 0; $script:rehydrateMoveCalls = 0
$script:isOrphanMode = $script:mode.StartsWith('orphan') -or $script:mode -eq 'late-orphan-prepared-resume'
$script:ghost = $script:mode.StartsWith('orphan')
$script:initialJournalInterrupted = $false
 $script:staged = $script:mode -notin @('fresh','orphan','orphan-native-fallback','orphan-remove-fails','orphan-rehydrate-fails','orphan-removal-interrupted','late-orphan-prepared-resume'); $script:registered = $script:mode -notin @('fresh','orphan','orphan-native-fallback','orphan-remove-fails','orphan-rehydrate-fails','orphan-removal-interrupted','late-orphan-prepared-resume')
$script:retiredSource = Join-Path $script:fixtureRoot 'retired-data'
 if ($script:mode -in @('fresh', 'orphan', 'orphan-native-fallback', 'orphan-remove-fails', 'orphan-rehydrate-fails', 'orphan-removal-interrupted', 'late-orphan-prepared-resume', 'already-local', 'update-local', 'update-from-system','update-downgrade-from-system','update-unknown-location','update-running-from-system')) {{
 [IO.Directory]::Delete($script:sourceLink)
 if ($script:mode -eq 'fresh') {{ [IO.Directory]::Delete($script:sourceRoot) }}
 else {{
  [IO.Directory]::CreateDirectory((Join-Path $script:sourceRoot 'LocalState')) | Out-Null
  [IO.File]::WriteAllText((Join-Path $script:sourceRoot 'LocalState\existing.txt'), 'keep existing local data')
 }}
}}
function New-FixtureRegisteredPackage {{
 param([string]$FullName=$script:registeredFullName,[string]$Root=$script:registeredRoot)
 @{{ PackageFullName=$FullName; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'; Version=$FullName.Split('_')[1]; InstallLocation=($Root + '\WindowsApps\' + $FullName); PackageUserInformation=@(@{{UserSecurityId=[string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value; InstallState='Installed'}}) }}
}}
function Get-AppxPackage {{
 param($Name,[switch]$AllUsers,$Volume,$ErrorAction)
 if ($script:registered) {{ New-FixtureRegisteredPackage }}
 elseif ($AllUsers -and $script:ghost) {{
  @{{ PackageFullName=$script:fullName; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'; Version='26.825.5331.0'; InstallLocation=''; PackageUserInformation=@() }}
 }}
 elseif ($AllUsers -and $script:staged) {{
  $p = New-FixtureRegisteredPackage $script:stagedFullName 'R:'
  $p.PackageUserInformation = @(@{{UserSecurityId='S-1-5-18';InstallState='Staged'}})
  $p
 }}
}}
function Get-AppxVolume {{
 param($Path,[switch]$Online,$ErrorAction)
 if (-not $Path -or $Path -eq 'R:\') {{ @{{Name=$script:volumeId;PackageStorePath='R:\WindowsApps';IsOffline=$false;IsSystemVolume=$false}} }}
 if (-not $Path) {{ @{{Name='\\?\Volume{{99999999-1111-2222-3333-444444444444}}\';PackageStorePath=($env:SystemDrive + '\Program Files\WindowsApps');IsOffline=$false;IsSystemVolume=$true}} }}
}}
function Get-AppxProvisionedPackage {{param([switch]$Online,$ErrorAction);@()}}
function Invoke-FixtureAfterInitialJournal {{
 if ($script:mode -eq 'late-orphan-prepared-resume' -and -not $script:initialJournalInterrupted) {{
  $script:initialJournalInterrupted=$true; $script:ghost=$true
  throw 'fixture orphan appeared after the null repair record was journaled'
 }}
}}
function Get-CimInstance {{
 param($ClassName,$Filter,$ErrorAction)
 if ($script:mode -eq 'update-running-from-system') {{
  @{{ExecutablePath=($script:registeredRoot + '\WindowsApps\' + $script:registeredFullName + '\Codex.exe')}}
 }} else {{@()}}
}}
function Add-AppxVolume {{param($Path,$ErrorAction);throw 'fixture forbids real volume mutation'}}
function Add-AppxPackage {{param($Path,$Volume,$ErrorAction);throw 'new flow must not call legacy Add'}}
function Move-AppxPackage {{
 param($Package,$Volume,$ErrorAction)
 if ($script:mode -ne 'update-from-system' -or $Package -ne $script:registeredFullName) {{throw 'unexpected legacy Move'}}
 $script:moveCalls++; $script:registeredRoot='R:'
}}
function Remove-AppxPackage {{
 param($Package,[switch]$AllUsers,$ErrorAction)
 if ($Package -ne $script:fullName) {{throw 'unauthorized fixture package removal'}}
 if ($script:isOrphanMode) {{
  if (-not $AllUsers -or -not $script:ghost) {{throw 'orphan repair did not use the supported all-users API'}}
 }} elseif ($AllUsers -or $script:mode -notin @('repair','update-wpsystem')) {{throw 'unauthorized fixture removal'}}
 $script:removeCalls++
  if ($script:mode -in @('orphan-native-fallback','orphan-remove-fails','orphan-rehydrate-fails')) {{throw 'fixture Appx PowerShell projection rejected orphan removal'}}
 if ($script:mode -eq 'orphan-removal-interrupted') {{
  [IO.Directory]::Move($script:sourceRoot,$script:retiredSource)
  $script:registered=$false; $script:staged=$false; $script:ghost=$false
  return
 }}
 [IO.Directory]::Move($script:sourceRoot,$script:retiredSource)
 $script:registered=$false; $script:staged=$false; $script:ghost=$false
}}
function Invoke-FixtureNativeOrphanRemoval {{
 param($FullName)
 $script:nativeRemoveCalls++
 if ($script:mode -ne 'orphan-native-fallback' -or $FullName -ne $script:fullName) {{throw 'fixture native AppX API rejected orphan removal'}}
 [IO.Directory]::Move($script:sourceRoot,$script:retiredSource)
 $script:registered=$false; $script:staged=$false; $script:ghost=$false
 @{{activityId='fixture-native-remove';errorText=''}}
}}
function Test-Path {{
 param($LiteralPath,$PathType,$ErrorAction)
 if ($LiteralPath -eq 'R:\WindowsApps') {{return $true}}
 Microsoft.PowerShell.Management\Test-Path -LiteralPath $LiteralPath -ErrorAction Stop
}}
try {{
 $install = {{ {production} }}
 $firstReport = if ($script:mode -in @('orphan-removal-interrupted','late-orphan-prepared-resume')) {{ & $install | ConvertFrom-Json }} else {{ $null }}
 $report = & $install | ConvertFrom-Json
 @{{report=$report;firstReport=$firstReport;stage=$script:stageCalls;register=$script:registerCalls;remove=$script:removeCalls;nativeRemove=$script:nativeRemoveCalls;move=$script:moveCalls;rehydrate=$script:rehydrateCalls;rehydrateMove=$script:rehydrateMoveCalls;registered=$script:registered;ghost=$script:ghost}} | ConvertTo-Json -Depth 12 -Compress
}} finally {{
 foreach ($link in @($script:sourceLink,(Join-Path $script:retiredSource 'AC'))) {{
  $entry = Microsoft.PowerShell.Management\Get-Item -LiteralPath $link -Force -ErrorAction SilentlyContinue
  if ($entry -and ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{[IO.Directory]::Delete($link)}}
 }}
}}
"#,
                mode = ps_quote(mode)
            );
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            if mode == "orphan-rehydrate-fails" {
                assert_eq!(value["report"]["success"], false, "{value}");
                assert_eq!(value["remove"], 1, "{value}");
                assert_eq!(value["nativeRemove"], 1, "{value}");
                assert_eq!(value["rehydrate"], 0, "{value}");
                let message = value["report"]["message"].as_str().unwrap();
                assert!(
                    message.contains("fixture exact-payload rehydration failed"),
                    "{value}"
                );
                assert!(message.contains("\"state\":\"failed\""), "{value}");
                assert!(message.contains("\"attempts\":1"), "{value}");
                assert!(
                    message.contains("\"lastError\":\"fixture exact-payload rehydration failed"),
                    "{value}"
                );
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            if mode == "orphan-remove-fails" {
                assert_eq!(value["report"]["success"], true, "{value}");
                assert_eq!(value["remove"], 1, "{value}");
                assert_eq!(value["nativeRemove"], 1, "{value}");
                assert_eq!(value["stage"], 0, "{value}");
                assert_eq!(value["rehydrate"], 1, "{value}");
                assert_eq!(value["register"], 1, "{value}");
                assert_eq!(value["ghost"], false, "{value}");
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
                let message = value["report"]["message"].as_str().unwrap();
                assert!(message.contains("exact-payload rehydration"), "{value}");
                assert!(message.contains("未编辑 PackageRepository"), "{value}");
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            if mode == "orphan-native-fallback" {
                assert_eq!(value["report"]["success"], true, "{value}");
                assert_eq!(value["remove"], 1, "{value}");
                assert_eq!(value["nativeRemove"], 1, "{value}");
                assert_eq!(value["stage"], 1, "{value}");
                assert_eq!(value["register"], 1, "{value}");
                assert_eq!(value["ghost"], false, "{value}");
                assert!(
                    value["report"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("PackageManager.RemovePackageAsync"),
                    "{value}"
                );
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            if mode == "orphan-removal-interrupted" {
                assert_eq!(value["firstReport"]["success"], false, "{value}");
                let first_message = value["firstReport"]["message"].as_str().unwrap();
                assert!(
                    first_message.contains("process terminated before AppX removal requery"),
                    "{value}"
                );
                assert!(first_message.contains("\"state\":\"failed\""), "{value}");
                assert_eq!(value["report"]["success"], true, "{value}");
                assert_eq!(value["remove"], 1, "{value}");
                assert_eq!(value["stage"], 1, "{value}");
                assert_eq!(value["register"], 1, "{value}");
                assert_eq!(value["ghost"], false, "{value}");
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            if mode == "late-orphan-prepared-resume" {
                assert_eq!(value["firstReport"]["success"], false, "{value}");
                assert!(
                    value["firstReport"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("orphan appeared after the null repair record was journaled"),
                    "{value}"
                );
                assert_eq!(value["report"]["success"], true, "{value}");
                assert_eq!(value["remove"], 1, "{value}");
                assert_eq!(value["nativeRemove"], 0, "{value}");
                assert_eq!(value["stage"], 1, "{value}");
                assert_eq!(value["register"], 1, "{value}");
                assert_eq!(value["ghost"], false, "{value}");
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            if mode == "update-downgrade-from-system"
                || mode == "update-unknown-location"
                || mode == "update-running-from-system"
            {
                assert_eq!(value["report"]["success"], false, "{mode}: {value}");
                for operation in ["move", "remove", "stage", "register"] {
                    assert_eq!(value[operation], 0, "{mode}: {value}");
                }
                assert_eq!(value["registered"], true, "{mode}: {value}");
                let expected_reason = if mode == "update-downgrade-from-system" {
                    "does not downgrade"
                } else if mode == "update-unknown-location" {
                    "location is unknown"
                } else {
                    "still running"
                };
                assert!(
                    value["report"]["message"]
                        .as_str()
                        .unwrap()
                        .contains(expected_reason),
                    "{mode}: {value}"
                );
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
                std::fs::remove_dir_all(root).unwrap();
                continue;
            }
            assert_eq!(value["report"]["success"], true, "{mode}: {value}");
            assert_eq!(value["registered"], true, "{mode}: {value}");
            assert_eq!(
                value["remove"],
                if mode == "orphan" || mode == "repair" || mode == "update-wpsystem" {
                    1
                } else {
                    0
                },
                "{mode}: {value}"
            );
            let expected_deployments = if mode == "already-local" {
                0
            } else if mode == "update-wpsystem" {
                2
            } else {
                1
            };
            assert_eq!(value["stage"], expected_deployments, "{mode}: {value}");
            assert_eq!(value["register"], expected_deployments, "{mode}: {value}");
            assert_eq!(
                value["move"],
                if mode == "update-from-system" { 1 } else { 0 },
                "{mode}: {value}"
            );
            if mode == "repair" || mode == "update-wpsystem" {
                assert_eq!(
                    std::fs::read_to_string(
                        root.join("local/Packages/OpenAI.Codex_2p2nqsd0c76g0/AC/Temp/valuable.txt")
                    )
                    .unwrap(),
                    "valuable AC state must survive"
                );
                assert!(value["report"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("originalBackup="));
                assert!(value["report"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("baselineBackup="));
            } else if mode != "fresh" {
                assert_eq!(
                    std::fs::read_to_string(
                        root.join(
                            "local/Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalState/existing.txt"
                        )
                    )
                    .unwrap(),
                    "keep existing local data"
                );
            }
            if mode == "orphan" {
                assert!(
                    value["report"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("Windows AppX 正式接口"),
                    "{value}"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn wpsystem_nonempty_exact_authorized_ac_is_backed_up_before_uninstall() {
        let (root, setup) = wpsystem_fixture_script("valid");
        let production = project_wpsystem_fixture_io(remove_msix_package_script_with_volume(
            true,
            Some(Path::new("R:\\")),
            Some(r"\\?\Volume{12345678-1111-2222-3333-444444444444}\"),
        ));
        let script = format!(
            r#"{setup}
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{ report = $report; removes = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete($script:sourceLink) }}
"#
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], true, "{value}");
        assert_eq!(value["removes"], 1);
        let backups = root
            .join("local")
            .join("CodexAppxDataBackups")
            .join("OpenAI.Codex_2p2nqsd0c76g0");
        let backup = std::fs::read_dir(backups)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(backup.join("AC").join("Temp").join("valuable.txt")).unwrap(),
            "valuable AC state must survive"
        );
        let manifest = std::fs::read_to_string(backup.with_extension("manifest.json")).unwrap();
        assert!(manifest.contains("12345678-1111-2222-3333-444444444444"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn wpsystem_backup_rejects_identity_changes_and_disconnected_targets() {
        for mode in [
            "wrongSid",
            "wrongPfn",
            "wrongLeaf",
            "wrongVolume",
            "offline",
            "swappedVolume",
            "swappedTarget",
            "noAuth",
            "badTag",
            "secondHop",
            "disconnectedTarget",
            "lateVolumeChange",
        ] {
            let (root, setup) = wpsystem_fixture_script(mode);
            let mut production =
                project_wpsystem_fixture_io(remove_msix_package_script_with_volume(
                    true,
                    Some(Path::new("R:\\")),
                    Some(r"\\?\Volume{12345678-1111-2222-3333-444444444444}\"),
                ));
            if mode == "noAuth" {
                production = project_wpsystem_fixture_io(remove_msix_package_script(true));
            }
            if mode == "badTag" {
                production = production.replace(
                    "Get-CodexReparseTag ([string]$entry.FullName)",
                    "'0xa000000c'",
                );
            }
            let script = format!(
                r#"{setup}
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{ report = $report; removes = $script:removeCalls; content = [IO.File]::ReadAllText((Join-Path $script:physicalTarget 'Temp\valuable.txt')) }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete($script:sourceLink) }}
"#
            );
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert_eq!(value["report"]["success"], false, "{mode}: {value}");
            assert_eq!(value["removes"], 0, "{mode}: {value}");
            assert_eq!(value["content"], "valuable AC state must survive");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn wpsystem_wrong_volume_rollback_preserves_ac_before_removing_new_registration() {
        let (root, setup) = wpsystem_fixture_script("valid");
        let production = project_wpsystem_fixture_io(install_msix_script_with_volume(Path::new(r"C:\staging\Codex.msix"), Some(Path::new("R:\\"))))
            .replace("$nativeAppxResult = Invoke-NativeAppxAdd", "function Invoke-NativeAppxAdd { param($packagePath, $targetVolume); throw 'native disabled in fixture' }\n$nativeAppxResult = Invoke-NativeAppxAdd");
        let script = format!(
            r#"{setup}
$script:registered = $false
$script:removeCalls = 0
function Test-Path {{ param($LiteralPath, $PathType, $ErrorAction); $LiteralPath -eq 'R:\WindowsApps' }}
function Add-AppxVolume {{ param($Path, $ErrorAction); throw 'unexpected volume mutation' }}
function Get-AppxProvisionedPackage {{ param([switch]$Online, $ErrorAction); @() }}
function Get-AppxPackage {{
 param($Name, [switch]$AllUsers, $ErrorAction)
 if ($script:registered) {{
  @{{ PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'; PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'; Version = '26.825.5331.0'; InstallLocation = 'C:\Program Files\WindowsApps\OpenAI.Codex'; PackageUserInformation = @(@{{UserSecurityId = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value; InstallState = 'Installed'}}) }}
 }}
}}
function Add-AppxPackage {{ param($Path, $Volume, $ErrorAction); $script:registered = $true }}
function Move-AppxPackage {{ param($Package, $Volume, $ErrorAction); throw 'move disabled in fixture' }}
function Remove-AppxPackage {{ param($Package, $ErrorAction); $script:registered = $false; $script:removeCalls++ }}
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{ report = $report; removes = $script:removeCalls; registered = $script:registered }} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete($script:sourceLink) }}
"#
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["removes"], 1, "{value}");
        assert_eq!(value["registered"], false);
        assert!(
            value["report"]["message"]
                .as_str()
                .unwrap()
                .contains("wrongVolumeCleanup=completed"),
            "{value}"
        );
        let backups = root
            .join("local")
            .join("CodexAppxDataBackups")
            .join("OpenAI.Codex_2p2nqsd0c76g0");
        let backup = std::fs::read_dir(backups)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(backup.join("AC").join("Temp").join("valuable.txt")).unwrap(),
            "valuable AC state must survive"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_does_not_downgrade_unrelated_access_denied_or_generic_hresult() {
        for error in [
            "Access is denied. The package is in use.",
            "0x80073CFA: generic uninstall failed",
        ] {
            let production = remove_msix_package_script(true);
            let script = format!("{UNINSTALL_MOCKS}\n$script:developmentMode = $true\n$script:forceError = {}\n{production}", ps_quote(error));
            let report: MsixRemoveReport =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert!(!report.success, "{report:?}");
            assert_eq!(report.message, error);
        }
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_query_errors_are_not_clean_uninstalls() {
        for setup in ["$script:failQuery = $true", "$script:failPostQuery = $true"] {
            let production = remove_msix_package_script(false);
            let script = format!("{UNINSTALL_MOCKS}\n{setup}\n{production}");
            let report: MsixRemoveReport =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert!(!report.success, "{report:?}");
            assert!(report.message.contains("registration query denied"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_preserves_native_error_record_without_inventing_backup_hresult() {
        let production = remove_msix_package_script(false);
        let script = format!(
            r#"{UNINSTALL_MOCKS}
function Remove-AppxPackage {{
 [CmdletBinding()] param($Package)
 $script:removeCalls++
 $exception = [InvalidOperationException]::new('Native uninstall failed without detail in Exception.Message')
 $errorRecord = [Management.Automation.ErrorRecord]::new($exception, 'NativeRemovalFixture', [Management.Automation.ErrorCategory]::PermissionDenied, $Package)
 $errorRecord.ErrorDetails = [Management.Automation.ErrorDetails]::new('Deployment failed with HRESULT: 0x80073CFA; ActivityId {{9afe3240-101d-0002-37b9-8aa11d10dd01}}; original native explanation')
 $PSCmdlet.ThrowTerminatingError($errorRecord)
}}
function Start-Sleep {{ param($Milliseconds) }}
$report = & {{ {production} }} | ConvertFrom-Json
@{{ report = $report; calls = $script:removeCalls; registered = -not $script:removed }} | ConvertTo-Json -Depth 8 -Compress
"#
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["calls"], 8);
        assert_eq!(value["registered"], true);
        assert_eq!(
            value["report"]["message"],
            "Native uninstall failed without detail in Exception.Message"
        );
        let raw = value["report"]["rawError"].as_str().unwrap();
        for evidence in [
            "0x80073CFA",
            "9afe3240-101d-0002-37b9-8aa11d10dd01",
            "original native explanation",
            "FullyQualifiedErrorId=NativeRemovalFixture",
            "PermissionDenied",
        ] {
            assert!(raw.contains(evidence), "missing {evidence}: {raw}");
        }

        // This is our pre-removal safety guard, not a native deployment error.
        let production = remove_msix_package_script(true).replace(
            "$backupPath = Backup-CodexPackageData $p",
            "throw 'Cannot safely preserve linked package data: AC; removal was not started'",
        );
        let script = format!("{UNINSTALL_MOCKS}\n$report = & {{ {production} }} | ConvertFrom-Json\n@{{ report = $report; calls = $script:removeCalls; registered = -not $script:removed }} | ConvertTo-Json -Depth 8 -Compress");
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["calls"], 0);
        assert_eq!(value["registered"], true);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("removal was not started"));
        assert!(!value["report"]["rawError"]
            .as_str()
            .unwrap()
            .contains("0x80073CFA"));
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_preserves_verified_recoverable_package_data() {
        let root =
            std::env::temp_dir().join(format!("codex-data-backup-test-{}", uuid::Uuid::new_v4()));
        let source = root
            .join("Packages")
            .join("OpenAI.Codex_2p2nqsd0c76g0")
            .join("LocalState");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("profile #100%.json"), b"important user content").unwrap();
        let production = remove_msix_package_script(true);
        let script = format!(
            "{UNINSTALL_MOCKS}\n$env:LOCALAPPDATA = {}\n{production}",
            ps_quote(&root.to_string_lossy())
        );
        let report: MsixRemoveReport =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert!(report.success, "{report:?}");
        let backup_base = root
            .join("CodexAppxDataBackups")
            .join("OpenAI.Codex_2p2nqsd0c76g0");
        let backup = std::fs::read_dir(&backup_base)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .unwrap();
        assert_eq!(
            std::fs::read(backup.join("LocalState").join("profile #100%.json")).unwrap(),
            b"important user content"
        );
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("SHA256 verified") && note.contains("CodexAppxDataBackups")));
        assert!(std::fs::read(backup.with_extension("manifest.json")).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn local_data_gate_rejects_nested_external_redirect_without_reading_target() {
        let root =
            std::env::temp_dir().join(format!("codex-nested-placement-{}", uuid::Uuid::new_v4()));
        let package_root = root.join("Packages/OpenAI.Codex_2p2nqsd0c76g0");
        let link = package_root.join("LocalState/nested-cache");
        let target = root.join("unrelated-external");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("important.txt"),
            b"do not touch this external target",
        )
        .unwrap();
        let script = format!(
            r#"
$ErrorActionPreference = 'Stop'
$env:LOCALAPPDATA = {root}
New-Item -ItemType Junction -Path {link} -Target {target} -ErrorAction Stop | Out-Null
{backup}
try {{ Get-CodexPackageDataPlacement 'R:\' | ConvertTo-Json -Depth 8 -Compress }}
finally {{ [IO.Directory]::Delete({link}) }}
"#,
            root = ps_quote(&root.to_string_lossy()),
            link = ps_quote(&link.to_string_lossy()),
            target = ps_quote(&target.to_string_lossy()),
            backup = super::APPX_DATA_BACKUP_SCRIPT
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["state"], "redirected-unverified", "{value}");
        assert!(value.to_string().contains("nested-inventory-unverified"));
        assert_eq!(
            std::fs::read(target.join("important.txt")).unwrap(),
            b"do not touch this external target"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn backup_refuses_ancestor_redirect_created_after_inventory() {
        let root = std::env::temp_dir().join(format!(
            "codex-backup-reparse-race-{}",
            uuid::Uuid::new_v4()
        ));
        let source = root.join("Packages/OpenAI.Codex_2p2nqsd0c76g0");
        let retired = root.join("original-retained");
        let outside = root.join("unrelated-data");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(source.join("important.txt"), b"original-owned-data").unwrap();
        std::fs::write(outside.join("important.txt"), b"must-not-read-or-change").unwrap();
        let production = remove_msix_package_script(true).replace(
            "Copy-CodexBackupFile $entryPath $destination",
            "Invoke-FixtureBeforeCopy; Copy-CodexBackupFile $entryPath $destination",
        );
        let script = format!(
            r#"{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {root}
function Invoke-FixtureBeforeCopy {{
 [IO.Directory]::Move({source}, {retired})
 New-Item -ItemType Junction -Path {source} -Target {outside} -ErrorAction Stop | Out-Null
}}
try {{
 $report = & {{ {production} }} | ConvertFrom-Json
 @{{report=$report;removes=$script:removeCalls}} | ConvertTo-Json -Depth 8 -Compress
}} finally {{ [IO.Directory]::Delete({source}) }}
"#,
            root = ps_quote(&root.to_string_lossy()),
            source = ps_quote(&source.to_string_lossy()),
            retired = ps_quote(&retired.to_string_lossy()),
            outside = ps_quote(&outside.to_string_lossy())
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["removes"], 0, "{value}");
        assert_eq!(
            std::fs::read(retired.join("important.txt")).unwrap(),
            b"original-owned-data"
        );
        assert_eq!(
            std::fs::read(outside.join("important.txt")).unwrap(),
            b"must-not-read-or-change"
        );
        let store = root.join("CodexAppxDataBackups/OpenAI.Codex_2p2nqsd0c76g0");
        for entry in std::fs::read_dir(store).unwrap() {
            if entry.as_ref().unwrap().path().is_dir() {
                assert!(!entry.unwrap().path().join("important.txt").exists());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn uninstall_does_not_discard_data_created_after_absent_snapshot() {
        let root =
            std::env::temp_dir().join(format!("codex-absent-data-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("Packages/OpenAI.Codex_2p2nqsd0c76g0");
        let production = remove_msix_package_script(true).replace(
            "$backupPath = Backup-CodexPackageData $p",
            "$backupPath = Backup-CodexPackageData $p; Invoke-FixtureAfterAbsentBackup",
        );
        let script = format!(
            r#"{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {root}
function Invoke-FixtureAfterAbsentBackup {{
 [IO.Directory]::CreateDirectory({source}) | Out-Null
 [IO.File]::WriteAllText((Join-Path {source} 'new-session.txt'), 'concurrent user data')
}}
$report = & {{ {production} }} | ConvertFrom-Json
@{{report=$report;removes=$script:removeCalls}} | ConvertTo-Json -Depth 8 -Compress
"#,
            root = ps_quote(&root.to_string_lossy()),
            source = ps_quote(&source.to_string_lossy())
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false, "{value}");
        assert_eq!(value["removes"], 0, "{value}");
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("appeared after"));
        assert_eq!(
            std::fs::read_to_string(source.join("new-session.txt")).unwrap(),
            "concurrent user data"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn uninstall_busy_retry_rechecks_backup_and_absent_data_before_next_remove() {
        for mode in ["existing", "absent"] {
            let root = std::env::temp_dir()
                .join(format!("codex-remove-retry-data-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let source = root.join("Packages/OpenAI.Codex_2p2nqsd0c76g0");
            if mode == "existing" {
                std::fs::create_dir_all(&source).unwrap();
                std::fs::write(source.join("session.txt"), b"original backed-up data").unwrap();
            }
            let production = remove_msix_package_script(true);
            let script = format!(
                r#"{UNINSTALL_MOCKS}
$env:LOCALAPPDATA = {root}
function Remove-AppxPackage {{
 param($Package,$ErrorAction)
 $script:removeCalls++
 if ($script:removeCalls -eq 1) {{
  [IO.Directory]::CreateDirectory({source}) | Out-Null
  [IO.File]::WriteAllText((Join-Path {source} 'session.txt'), 'new user data after busy response')
  throw 'AppX deployment is busy; fixture writes user state while waiting'
 }}
 $script:removed = $true
}}
$report = & {{ {production} }} | ConvertFrom-Json
@{{report=$report;calls=$script:removeCalls;removed=$script:removed}} | ConvertTo-Json -Depth 8 -Compress
"#,
                root = ps_quote(&root.to_string_lossy()),
                source = ps_quote(&source.to_string_lossy())
            );
            let value: serde_json::Value =
                serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
            assert_eq!(value["report"]["success"], false, "{mode}: {value}");
            assert_eq!(
                value["calls"], 1,
                "No second Remove may use a stale backup: {mode}: {value}"
            );
            assert_eq!(value["removed"], false, "{mode}: {value}");
            assert_eq!(
                std::fs::read_to_string(source.join("session.txt")).unwrap(),
                "new user data after busy response"
            );
            if mode == "existing" {
                let store = root.join("CodexAppxDataBackups/OpenAI.Codex_2p2nqsd0c76g0");
                let backup = std::fs::read_dir(store)
                    .unwrap()
                    .filter_map(Result::ok)
                    .find(|e| e.path().is_dir())
                    .unwrap()
                    .path();
                assert_eq!(
                    std::fs::read(backup.join("session.txt")).unwrap(),
                    b"original backed-up data"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn msix_uninstall_backup_failure_prevents_any_removal() {
        let root = std::env::temp_dir().join(format!(
            "codex-data-backup-fail-test-{}",
            uuid::Uuid::new_v4()
        ));
        let source = root.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("important.txt"), b"must survive").unwrap();
        let production = remove_msix_package_script(true).replace(
            "Copy-CodexBackupFile $entryPath $destination",
            "throw 'backup copy denied'",
        );
        let script = format!(
            "{UNINSTALL_MOCKS}\n$env:LOCALAPPDATA = {}\nfunction Copy-Item {{ param($LiteralPath, $Destination, $ErrorAction); throw 'backup copy denied' }}\n$report = & {{\n{production}\n}} | ConvertFrom-Json\n@{{ report = $report; removes = $script:removeCalls }} | ConvertTo-Json -Depth 8 -Compress",
            ps_quote(&root.to_string_lossy())
        );
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["report"]["success"], false);
        assert_eq!(value["removes"], 0);
        assert!(value["report"]["message"]
            .as_str()
            .unwrap()
            .contains("backup copy denied"));
        assert_eq!(
            std::fs::read(source.join("important.txt")).unwrap(),
            b"must survive"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn native_msix_file_uri_roundtrips_reserved_characters() {
        let path = r"C:\安装 100% #测试\Codex.msix";
        let script = format!("$uri = [Uri]::new({}, [UriKind]::Absolute); @{{ path = $uri.LocalPath; fragment = $uri.Fragment; isFile = $uri.IsFile }} | ConvertTo-Json -Compress", ps_quote(path));
        let value: serde_json::Value =
            serde_json::from_str(&run_powershell_json(&script).unwrap()).unwrap();
        assert_eq!(value["path"], path);
        assert_eq!(value["fragment"], "");
        assert_eq!(value["isFile"], true);
    }

    #[cfg(windows)]
    #[test]
    fn usb_appx_install_mounts_stale_offline_volume_before_add() {
        let production = install_msix_script_with_volume(
            std::path::Path::new(r"C:\staging\Codex.msix"),
            Some(std::path::Path::new("E:\\")),
        );
        // A prior failed attempt can leave an offline E: registration. The
        // installer must mount and refresh that registration instead of
        // passing the stale object to Add-AppxPackage or registering a second
        // volume with the same package store path.
        let mocks = r#"
$script:mounted = $false
$script:packageVolume = $null
function Test-Path {
  param($LiteralPath, $PathType)
  $LiteralPath -eq 'E:\WindowsApps'
}
function Get-Volume {
  param($DriveLetter, $ErrorAction)
  @{ FileSystem = 'NTFS'; DriveType = 'Removable' }
}
function Get-AppxVolume {
  param($Path, [switch]$Online, $ErrorAction)
  if ($Online -and $script:mounted) {
    @{
      Name = '\\?\Volume{usb-mounted}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $false
      Source = 'mounted-online'
    }
    return
  }
  if (-not $Path -and -not $script:mounted) {
    @{
      Name = '\\?\Volume{usb-stale}'
      MountPoint = ''
      PackageStorePath = 'E:\WindowsApps'
      IsOffline = $true
      Source = 'stale-offline'
    }
  }
}
function Mount-AppxVolume {
  param($Volume, $ErrorAction)
  if ($Volume.Source -ne 'stale-offline') { throw 'unexpected volume to mount' }
  $script:mounted = $true
}
function Add-AppxVolume {
  param($Path, $ErrorAction)
  throw 'stale volume should be mounted, not re-added'
}
function Get-AppxPackage {
  param($Name, $ErrorAction)
  if ($script:packageVolume) {
    @{
      InstallLocation = $script:packageVolume
      Version = '26.825.5331.0'
      PackageFamilyName = 'OpenAI.Codex_2p2nqsd0c76g0'
      PackageFullName = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
    }
  }
}
function Add-AppxPackage {
  param($Path, $Volume, $ForceUpdateFromAnyVersion, $ForceTargetApplicationShutdown, $ForceApplicationShutdown, $ErrorAction)
  if ($Path -ne 'C:\staging\Codex.msix') { throw "unexpected package path: $Path" }
  if ($null -eq $Volume -or $Volume.Source -ne 'mounted-online') {
    throw 'stale offline volume was passed to Add-AppxPackage'
  }
  $script:packageVolume = 'E:\WindowsApps\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
}
"#;
        let json = run_powershell_json(&format!("{}\n{}", mocks, production))
            .expect("run stale-offline AppxVolume mock");
        let report: MsixSideloadReport =
            serde_json::from_str(&json).expect("parse stale-offline report");
        assert!(
            report.success,
            "stale offline volume should be recovered: {report:?}"
        );
        assert_eq!(
            report
                .installed
                .as_ref()
                .and_then(|installed| installed.package_volume.as_deref()),
            Some("E:\\")
        );
    }

    #[test]
    fn msix_sideload_report_defaults_optional_diagnostics_for_old_json() {
        let report: super::MsixSideloadReport = serde_json::from_str(
            r#"{
                "success": false,
                "message": "deployment failed",
                "installed": null,
                "fallbackRecommended": true,
                "rawError": "deployment failed"
            }"#,
        )
        .expect("old MSIX report shape remains readable");
        assert!(report.hresult.is_none());
        assert!(report.activity_id.is_none());
        assert!(report.deployment_log.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn constrained_language_msix_detection_preserves_json_contract() {
        let script = format!("{}\n{}", CONSTRAINED_APPX_MOCKS, detect_msix_script());
        let json = run_powershell_json(&script).expect("run constrained-language MSIX detection");
        let installed: InstalledWindowsCodex =
            serde_json::from_str(&json).expect("parse constrained-language detected install");
        assert_eq!(installed.version, "26.707.3748.0");
        assert_eq!(installed.source, "msix");
    }

    #[cfg(windows)]
    const SYSTEM_PACKAGE_ALIAS_MOCKS: &str = r#"
$env:ProgramW6432='C:\Program Files'
$script:fullName='OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
function Get-AppxPackage { [CmdletBinding()]param($Name)
  @{PackageFullName=$script:fullName; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'; Version='26.825.5331.0'; Status='Ok'; InstallLocation=('C:\Program Files\WindowsApps\'+$script:fullName)}
}
function Get-Item { [CmdletBinding()]param($LiteralPath,[switch]$Force)
  if ($script:denied) { throw 'mock package path access denied' }
  if ($LiteralPath -eq ('C:\Program Files\WindowsApps\'+$script:fullName) -and -not $script:plain) {
    return @{PSIsContainer=$true; Attributes=[IO.FileAttributes]::ReparsePoint; LinkType='Junction'; Target=@('D:\WindowsApps\'+$script:fullName)}
  }
  return @{PSIsContainer=$true; Attributes=[IO.FileAttributes]::Directory}
}
"#;
    #[cfg(windows)]
    #[test]
    fn msix_detection_resolves_only_verified_system_junctions_to_physical_volume() {
        for (setup, expected) in [
            ("", Some(r"D:\")),
            (
                "$ExecutionContext.SessionState.LanguageMode='ConstrainedLanguage'",
                Some(r"D:\"),
            ),
            ("$script:plain=$true", Some(r"C:\")),
            ("$script:denied=$true", None),
        ] {
            let json = run_powershell_json(&format!(
                "{SYSTEM_PACKAGE_ALIAS_MOCKS}\n{setup}\n{}",
                detect_msix_script()
            ))
            .unwrap();
            let installed: InstalledWindowsCodex = serde_json::from_str(&json).unwrap();
            assert_eq!(
                installed.package_volume.as_deref(),
                expected,
                "{setup}: {json}"
            );
            assert!(
                installed.path.starts_with(r"C:\Program Files\WindowsApps\"),
                "provenance path must not be silently rewritten"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn health_probe_recognizes_alias_and_physical_process_and_refuses_unknown_alias() {
        // Shadow every process/deployment observation. No real app is started
        // or stopped; the production activation/liveness path still executes.
        let mocks = r#"
$script:active=$false; $script:starts=0; $script:stops=0
function Get-AppxPackageManifest { [CmdletBinding()]param($Package)
  @{Package=@{Applications=@{Application=@{Id='App'}};Dependencies=@{PackageDependency=@()}}}
}
function Get-Process { [CmdletBinding()]param($Name,$Id)
  if ($script:active) { @{Id=543210; Path=$script:processPath} }
}
function Start-Process { [CmdletBinding()]param($FilePath,$ArgumentList)
  $script:active=$true; $script:starts++
}
function Stop-Process { [CmdletBinding()]param($Id,[switch]$Force)
  if ($Id -ne 543210) { throw 'unexpected mock PID' }
  $script:active=$false; $script:stops++
}
function Convert-Path { [CmdletBinding()]param($LiteralPath) $LiteralPath }
function Get-CimInstance { [CmdletBinding()]param($ClassName,$Filter) throw 'unexpected fallback in process fixture' }
"#;
        let production = msix_health_script(false)
            .replace(
                &format!("$activationWindowSecs = {MSIX_ACTIVATION_WINDOW_SECS}"),
                "$activationWindowSecs = 2",
            )
            .replace(
                &format!("$livenessWindowSecs = {MSIX_LIVENESS_WINDOW_SECS}"),
                "$livenessWindowSecs = 1",
            );
        for (root, denied) in [
            (r"D:\WindowsApps", false),
            (r"C:\Program Files\WindowsApps", false),
            (r"D:\WindowsApps", true),
        ] {
            let script = format!("{SYSTEM_PACKAGE_ALIAS_MOCKS}\n{mocks}\n$script:processPath='{root}\\'+$script:fullName+'\\app\\ChatGPT.exe'; $script:denied=${denied}\n$result=& {{ {production} }} | ConvertFrom-Json\n@{{report=$result;starts=$script:starts;stops=$script:stops}} | ConvertTo-Json -Compress -Depth 5");
            let json = run_powershell_json(&script).unwrap();
            let data: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(data["report"]["activationOk"], !denied, "{json}");
            assert_eq!(data["starts"], if denied { 0 } else { 1 }, "{json}");
            assert_eq!(data["stops"], if denied { 0 } else { 1 }, "{json}");
        }
    }

    #[test]
    fn msix_health_failure_kinds_are_stable_strings() {
        // Keep these stable: frontend / notes may switch on them.
        assert_eq!(super::msix_failure::NOT_REGISTERED, "not-registered");
        assert_eq!(super::msix_failure::ACTIVATION_FAILED, "activation-failed");
        assert_eq!(super::msix_failure::IMMEDIATE_EXIT, "immediate-exit");
        assert_eq!(super::msix_failure::TIMEOUT, "timeout");
        assert_eq!(super::msix_failure::POLICY, "policy");
        assert_eq!(super::msix_failure::CLEANUP_FAILED, "cleanup-failed");
    }

    #[test]
    fn msix_liveness_window_matches_portable() {
        assert_eq!(
            crate::process::MSIX_LIVENESS_WINDOW_SECS,
            crate::process::PORTABLE_LIVENESS_WINDOW.as_secs()
        );
        assert!(crate::process::MSIX_ACTIVATION_WINDOW_SECS >= 20);
    }

    #[cfg(windows)]
    #[test]
    fn appx_shell_activation_passes_target_to_explorer_not_uri_as_file_path() {
        let snippet = powershell_explorer_activation("$aumid");
        assert!(snippet.contains("$shellTarget = \"shell:AppsFolder\\\" + $aumid"));
        assert!(snippet.contains(
            "Start-Process -FilePath $explorer -ArgumentList @($shellTarget) -ErrorAction Stop"
        ));
        assert!(!snippet.contains("Start-Process (\"shell:AppsFolder\\\""));
    }

    #[cfg(windows)]
    #[test]
    fn offline_appx_recovery_targets_only_the_exact_system_update_activity() {
        let detection =
            detect_offline_appx_conflict_script("OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0");
        let recovery = offline_appx_recovery_script(
            "OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0",
            "{9afe3240-101d-0002-37b9-8aa11d10dd01}",
            "4cecd7e61f209785343c5808b459e94818b9627f",
            Path::new(r"C:\Temp\codex.restore"),
            Path::new(r"C:\Temp\codex.recovery-started"),
            Path::new(r"C:\Temp\codex.recovery-done"),
            Path::new(r"C:\Temp\codex.recovery-timed-out"),
            1_800_000_000_000,
        );
        for script in [&detection, &recovery] {
            assert!(script.contains("MainPackageMoniker"));
            assert!(script.contains("ActivityID"));
            assert!(script.contains("[int]$_.Id -eq 400 -and [int]$_.Opcode -eq 2"));
            assert!(script.contains("401, 404 -contains [int]$_.Id"));
            assert!(script.contains("DeploymentOperation"));
            assert!(script.contains("wuauserv"));
            assert!(script.contains("S-1-5-18"));
            assert!(!script.contains("QueueDeadline"));
            assert!(!script.contains("FileSizeInCache"));
            assert!(!script.contains("$_.Message"));
        }
        assert!(recovery.contains("Delete-DeliveryOptimizationCache -FileId"));
        assert!(recovery.contains("PredefinedCallerApplication"));
        assert!(recovery.contains("Stop-Process -Id"));
        assert!(recovery.contains("EncodedCommand"));
        assert!(recovery.contains("AddSeconds(600)"));
        assert!(recovery.contains("$RecoveryDeadlineUnixMs = [int64]1800000000000"));
        assert!(recovery.contains("codex.recovery-timed-out"));
        assert!(!recovery.contains("-File \""));
        assert!(
            encode_powershell_command(&recovery).len() < 30_000,
            "elevated recovery must leave room below the Windows command-line limit"
        );
        assert!(
            recovery
                .find("Start-RecoveryDeadlineWatchdog ($stopped -join ',')")
                .expect("deadline watchdog must be armed")
                < recovery
                    .find("Stop-Service -Name $name")
                    .expect("service stop must follow watchdog")
        );
        assert!(
            recovery
                .find("Start-RestoreWatchdog ($stopped -join ',')")
                .expect("watchdog must be armed")
                < recovery
                    .find("Stop-Service -Name $name")
                    .expect("service stop must follow watchdog")
        );
        assert!(recovery.contains("@('InstallService', 'wuauserv', 'DoSvc', 'AppXSvc', 'ClipSVC')"));
        assert!(
            std::time::Duration::from_secs(OFFLINE_APPX_RESTORE_WATCHDOG_SECS)
                > crate::process::APPX_RECOVERY_TIMEOUT + crate::process::INSTALL_TIMEOUT
        );
        assert!(
            std::time::Duration::from_secs(OFFLINE_APPX_RECOVERY_CLEANUP_MARGIN_SECS)
                < crate::process::APPX_RECOVERY_TIMEOUT
        );
    }

    #[cfg(windows)]
    #[test]
    fn offline_appx_prepare_skips_elevation_without_an_exact_conflict() {
        let production =
            detect_offline_appx_conflict_script("OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0");
        let script = format!(
            "function Get-WinEvent {{ param($FilterHashtable, $LogName, $FilterXPath, $ErrorAction); @() }}\n{production}"
        );
        let json = run_powershell_json(&script).expect("run no-conflict AppX preparation");
        let report: OfflineAppxConflictReport =
            serde_json::from_str(&json).expect("parse no-conflict AppX preparation");
        assert!(!report.active);
        assert!(report.activity_id.is_none());
        assert!(report.file_id.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn offline_appx_probe_failures_do_not_block_the_verified_local_msix() {
        assert!(
            parse_offline_appx_conflict_probe(Err(PowerShellRunError::Timeout(
                crate::process::TimeoutKind::Total
            )))
            .is_none()
        );
        assert!(parse_offline_appx_conflict_probe(Ok("not-json".to_string())).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn appx_path_payload_only_enables_cache_cleanup_for_a_valid_file_id() {
        let valid = "4cecd7e61f209785343c5808b459e94818b9627f";
        assert_eq!(
            validated_delivery_optimization_file_id(Some(valid.to_string())),
            valid
        );
        assert_eq!(
            validated_delivery_optimization_file_id(Some(
                r"C:\Windows\SoftwareDistribution\Download\Codex.msix".to_string()
            )),
            ""
        );
        assert_eq!(
            validated_delivery_optimization_file_id(Some("https://example.invalid".to_string())),
            ""
        );
    }

    #[cfg(windows)]
    #[test]
    fn offline_appx_elevated_helper_is_valid_powershell() {
        let recovery = offline_appx_recovery_script(
            "OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0",
            "{9afe3240-101d-0002-37b9-8aa11d10dd01}",
            "4cecd7e61f209785343c5808b459e94818b9627f",
            Path::new(r"C:\Temp\codex.restore"),
            Path::new(r"C:\Temp\codex.recovery-started"),
            Path::new(r"C:\Temp\codex.recovery-done"),
            Path::new(r"C:\Temp\codex.recovery-timed-out"),
            1_800_000_000_000,
        );
        let script = format!(
            "$null = [scriptblock]::Create({}); @{{ ok = $true }} | ConvertTo-Json -Compress",
            ps_quote(&recovery)
        );
        // Exercise the same encoded transport used in production. Passing this
        // large nested script through `-Command` adds a second quoting layer
        // that can hang Windows PowerShell near its command-line limit.
        let encoded = encode_powershell_command(&script);
        let mut command = hidden_command(powershell_exe());
        command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]);
        let output = run_capturing(command, RunLimits::probe(), None)
            .expect("parse elevated AppX recovery helper");
        assert!(
            output.status.success(),
            "PowerShell syntax validation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .expect("parse helper syntax result")
                .get("ok")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[cfg(windows)]
    #[test]
    fn encoded_powershell_transport_executes_without_a_script_file() {
        let script = "@{ ok = $true } | ConvertTo-Json -Compress";
        let encoded = encode_powershell_command(script);
        let mut command = hidden_command(powershell_exe());
        command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]);
        let output =
            run_capturing(command, RunLimits::probe(), None).expect("run encoded PowerShell");
        assert!(output.status.success());
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse encoded PowerShell output");
        assert_eq!(
            value.get("ok").and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_timeout_is_typed_not_string_matched() {
        let err = super::PowerShellRunError::Timeout(crate::process::TimeoutKind::Total);
        assert!(matches!(
            err,
            super::PowerShellRunError::Timeout(crate::process::TimeoutKind::Total)
        ));
        // Callers classify via the enum arm, not by scraping message text.
        let msg = err.message();
        assert!(msg.contains("deadline"));
    }

    #[test]
    fn msix_health_report_defaults_new_fields_on_deserialize() {
        // Older fixtures without activationOk / failureKind still parse.
        let report: super::MsixHealthReport = serde_json::from_value(serde_json::json!({
            "healthy": true,
            "verified": true,
            "packageRegistered": true,
            "status": "Ok",
            "statusOk": true,
            "aumidResolved": true,
            "missingDependencies": [],
            "reason": ""
        }))
        .unwrap();
        assert!(report.healthy);
        assert!(!report.activation_ok); // default false when absent
        assert!(report.failure_kind.is_empty());
    }

    // Pure routing logic for the framework pre-check — verified on every host so
    // the steer-to-portable decision is covered even though the probe that fills
    // the struct only runs on Windows.
    #[test]
    fn precheck_routes_portable_only_when_a_framework_is_positively_missing() {
        // Missing framework, positively determined -> route to portable.
        let missing = super::MsixDependencyPrecheck {
            checked: true,
            frameworks_ok: false,
            missing_frameworks: vec!["Microsoft.VCLibs.140.00".to_string()],
            reason: "required framework packages are not installed: Microsoft.VCLibs.140.00"
                .to_string(),
        };
        assert!(missing.should_route_portable());

        // All frameworks present -> proceed with the sideload.
        let ok = super::MsixDependencyPrecheck {
            checked: true,
            frameworks_ok: true,
            missing_frameworks: vec![],
            reason: String::new(),
        };
        assert!(!ok.should_route_portable());

        // Probe could not run (manifest unreadable / PowerShell failed) -> do NOT
        // block on an unknown signal; let the sideload + health check decide.
        let unknown = super::MsixDependencyPrecheck {
            checked: false,
            frameworks_ok: true,
            missing_frameworks: vec![],
            reason: "framework dependency probe could not run; proceeding with sideload"
                .to_string(),
        };
        assert!(!unknown.should_route_portable());

        // Defensive: even if flags say not-ok, an empty list must not route (we
        // only steer when we can name the missing framework).
        let inconsistent = super::MsixDependencyPrecheck {
            checked: true,
            frameworks_ok: false,
            missing_frameworks: vec![],
            reason: String::new(),
        };
        assert!(!inconsistent.should_route_portable());
    }
}

#[cfg(test)]
mod portable_identity_tests {
    use super::detect_portable_install;
    use std::path::Path;

    fn write_root(name: &str, exe: &str, manifest: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("codex-sys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(exe), b"fake exe").unwrap();
        if let Some(identity_name) = manifest {
            std::fs::write(
                root.join("AppxManifest.xml"),
                format!(
                    r#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="{identity_name}" Publisher="CN=OpenAI OpCo, LLC" Version="26.707.3748.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app/{exe}" /></Applications>
</Package>"#
                ),
            )
            .unwrap();
        }
        root
    }

    fn cleanup(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_rebranded_portable_with_codex_identity() {
        let root = write_root("rebrand", "ChatGPT.exe", Some("OpenAI.Codex"));
        let installed = detect_portable_install(&root).expect("codex identity accepted");
        assert_eq!(installed.version, "26.707.3748.0");
        assert_eq!(installed.source, "portable");
        cleanup(&root);
    }

    #[test]
    fn rejects_portable_with_foreign_identity() {
        // Same file shape as an unpacked ChatGPT Classic: entry exe + manifest,
        // but the identity is not OpenAI.Codex — never a Codex install.
        let root = write_root("classic", "ChatGPT.exe", Some("OpenAI.ChatGPT"));
        assert!(detect_portable_install(&root).is_none());
        cleanup(&root);
    }

    #[test]
    fn rejects_portable_without_manifest_identity() {
        // Without a manifest the gate falls back to the app.asar package-name
        // marker; this root has no asar at all, so it is not recognized.
        let root = write_root("no-manifest", "Codex.exe", None);
        assert!(detect_portable_install(&root).is_none());
        cleanup(&root);
    }

    #[test]
    fn accepts_manifestless_portable_with_codex_asar_marker() {
        // A user-unpacked official `app/` dir: no package-root manifest, but
        // the payload's asar carries the Codex package name — the supported
        // self-extracted layout must stay detectable/adoptable.
        let root = write_root("selfextracted", "ChatGPT.exe", None);
        let resources = root.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        crate::app_version::write_test_asar(
            &resources.join("app.asar"),
            br#"{"version":"26.707.31428","name":"openai-codex-electron"}"#,
        );
        let installed = detect_portable_install(&root).expect("asar marker accepted");
        assert_eq!(installed.version, "26.707.31428");
        cleanup(&root);
    }

    #[test]
    fn rejects_manifestless_portable_with_foreign_asar_name() {
        // Same shape but a non-Codex Electron payload (e.g. an unpacked
        // Classic): the asar package name differs, so it is never adopted.
        let root = write_root("foreign-asar", "ChatGPT.exe", None);
        let resources = root.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        crate::app_version::write_test_asar(
            &resources.join("app.asar"),
            br#"{"version":"1.2026.160","name":"chatgpt-desktop"}"#,
        );
        assert!(detect_portable_install(&root).is_none());
        cleanup(&root);
    }
}
