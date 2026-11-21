//! Offline Codex capability-pack installation for the USB manager.
//!
//! The pack is a separate, signed-by-build-input directory shipped beside the
//! USB manager. This module invokes the pack's local PowerShell installer.
//! Skills, Python/Node base runtimes, Feishu MCP, FFmpeg, and Manim are bundled;
//! the HyperFrames CLI may need npm access; its pinned browser is bundled.
//! A missing network-dependent component is reported as requires-network and
//! does not prevent the remaining capability pack from being installed.

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::errors::AppError;

const PACK_DIRECTORY: &str = "codex-starter-kit";
const PACK_ID: &str = "codex-starter";
const KIT_HOME_RELATIVE: &str = "codex-agent-kit";
const READINESS_RELATIVE: &str = "codex-agent-kit/readiness.json";
const MEDIA_AUTHORIZATION_RELATIVE: &str =
    "codex-agent-kit/connections/media-generation.authorization.json";
const CONNECTION_OWNERSHIP_RELATIVE: &str = "codex-agent-kit/connection-ownership.json";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CAPABILITY_INSTALL_PROGRESS_EVENT: &str = "capability-pack://progress";
const CAPABILITY_INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CAPABILITY_PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const CAPABILITY_INSTALL_STAGES: &[&str] = &[
    "pack.validate",
    "pack.skills",
    "runtime.verify",
    "runtime.python",
    "runtime.manim",
    "runtime.nodeMedia",
    "runtime.hyperframes",
    "runtime.hyperframesFfmpeg",
    "runtime.hyperframesCliCheck",
    "runtime.hyperframesBrowser",
    "runtime.hyperframesBrowserCheck",
    "runtime.feishu",
    "runtime.environment",
    "runtime.complete",
    "pack.pdf",
    "pack.media",
    "pack.config",
    "pack.models",
    "pack.finalize",
];
const OWNED_CONNECTION_ENV_NAMES: &[&str] = &[
    "FEISHU_APP_ID",
    "FEISHU_APP_SECRET",
    "FEISHU_DOMAIN",
    "FEISHU_AUTH_MODE",
    "FEISHU_USER_ACCESS_TOKEN",
    "DASHSCOPE_API_KEY",
    "DASHSCOPE_BASE_URL",
    "DASHSCOPE_MODEL",
    "MEDIA_GENERATION_API_BASE_URL",
    "MEDIA_GENERATION_API_PROFILE",
    "MEDIA_GENERATION_API_KEY",
    "MEDIA_GENERATION_AGENT_ID",
    "MEDIA_IMAGE2_MODEL",
    "MEDIA_GROK_IMAGE_MODEL",
    "MEDIA_GROK_VIDEO_MODEL",
    "MEDIA_SEEDANCE_MODEL",
    "MEDIA_MINIMAX_VIDEO_MODEL",
    // Retained only so a prior package-owned connection record can be safely
    // cleaned up. The new installer never writes or consumes these variables.
    "IMAGE_2_API_KEY",
    "IMAGE_2_API_BASE_URL",
    "IMAGE_2_MODEL",
    "IMAGE_2_AGENT_ID",
    "SEEDANCE_API_KEY",
    "SEEDANCE_API_BASE_URL",
    "SEEDANCE_MODEL",
    "SEEDANCE_AGENT_ID",
    "SEEDANCE_PRECHARGE_POINTS",
    "SEEDANCE_OFFICIAL_LINK_TTL_HOURS",
    "SEEDANCE_REFUND_RULE",
    "MINIMAX_API_KEY",
    "MINIMAX_API_BASE_URL",
];

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionOwnershipRecord {
    schema_version: u32,
    pack_id: String,
    names: Vec<String>,
    #[serde(default)]
    note: String,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FeishuConnectionAction {
    Apply,
    Preserve,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct CapabilityPackInstallOptions {
    pub enable_feishu: bool,
    pub feishu_action: Option<FeishuConnectionAction>,
    pub persist_connections: bool,
    pub feishu_app_id: String,
    pub feishu_app_secret: String,
    pub feishu_user_access_token: String,
    pub feishu_domain: String,
    pub feishu_auth_mode: String,
    pub enabled_api_services: Vec<String>,
    pub enabled_remote_mcps: Vec<String>,
    pub enable_media_generation: bool,
    pub media_gateway_catalog_status: String,
    pub media_gateway_model_count: usize,
    pub media_gateway_catalog_verified_at: Option<String>,
    pub dashscope_api_key: String,
    pub dashscope_base_url: String,
    pub dashscope_model: String,
    pub media_generation_api_profile: String,
    pub media_generation_agent_id: String,
    pub image2_model: String,
    pub grok_image_model: String,
    pub grok_video_model: String,
    pub seedance_model: String,
    pub minimax_video_model: String,
}

impl Default for CapabilityPackInstallOptions {
    fn default() -> Self {
        Self {
            enable_feishu: true,
            feishu_action: None,
            persist_connections: true,
            feishu_app_id: String::new(),
            feishu_app_secret: String::new(),
            feishu_user_access_token: String::new(),
            feishu_domain: "https://open.feishu.cn".to_string(),
            feishu_auth_mode: "tenant".to_string(),
            enabled_api_services: Vec::new(),
            enabled_remote_mcps: Vec::new(),
            enable_media_generation: true,
            media_gateway_catalog_status: "not-checked".to_string(),
            media_gateway_model_count: 0,
            media_gateway_catalog_verified_at: None,
            dashscope_api_key: String::new(),
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            dashscope_model: String::new(),
            media_generation_api_profile: "responses".to_string(),
            media_generation_agent_id: String::new(),
            image2_model: String::new(),
            grok_image_model: String::new(),
            grok_video_model: String::new(),
            seedance_model: String::new(),
            minimax_video_model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPackReport {
    pub available: bool,
    pub installed: bool,
    pub success: bool,
    pub core_ready: bool,
    pub overall_status: String,
    pub source_path: Option<String>,
    pub pack_version: Option<String>,
    pub skills: usize,
    pub agents: usize,
    pub runtime_ready: bool,
    pub runtime_status: String,
    pub config_status: String,
    pub diagnostics_path: Option<String>,
    pub feishu_enabled: bool,
    pub feishu_configured: bool,
    pub feishu_saved_live: bool,
    pub feishu_runtime_status: String,
    pub enabled_api_services: Vec<String>,
    pub media_runtime_status: String,
    pub media_protocol_status: String,
    pub media_gateway_catalog_status: String,
    pub media_gateway_model_count: usize,
    pub media_gateway_catalog_verified_at: Option<String>,
    pub media_generation_capability: String,
    pub codex_restart_required: bool,
    pub codex_running: bool,
    pub message: String,
}

fn report_missing(message: impl Into<String>) -> CapabilityPackReport {
    CapabilityPackReport {
        available: false,
        installed: false,
        success: false,
        core_ready: false,
        overall_status: "failed".to_string(),
        source_path: None,
        pack_version: None,
        skills: 0,
        agents: 0,
        runtime_ready: false,
        runtime_status: "unknown".to_string(),
        config_status: "unknown".to_string(),
        diagnostics_path: None,
        feishu_enabled: false,
        feishu_configured: false,
        feishu_saved_live: false,
        feishu_runtime_status: "unknown".to_string(),
        enabled_api_services: Vec::new(),
        media_runtime_status: "unknown".to_string(),
        media_protocol_status: "unknown".to_string(),
        media_gateway_catalog_status: "not-checked".to_string(),
        media_gateway_model_count: 0,
        media_gateway_catalog_verified_at: None,
        media_generation_capability: "not-tested".to_string(),
        codex_restart_required: false,
        codex_running: false,
        message: message.into(),
    }
}

fn user_profile() -> Result<PathBuf, AppError> {
    if crate::app::usb_guard::guard_required() {
        let profile = crate::app::usb_guard::local_data_paths()?
            .user_profile
            .filter(|path| path.is_absolute())
            .ok_or_else(|| AppError::Engine("无法确定当前桌面用户目录".to_string()))?;
        return Ok(profile);
    }
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| AppError::Engine("无法确定当前 Windows 用户目录".to_string()))
}

fn powershell_provider_path(path: &Path) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let verbatim_prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        let is_ascii_drive_letter = |value: u16| {
            (b'A' as u16..=b'Z' as u16).contains(&value)
                || (b'a' as u16..=b'z' as u16).contains(&value)
        };
        if wide.starts_with(&verbatim_prefix)
            && wide.len() >= 7
            && is_ascii_drive_letter(wide[4])
            && wide[5] == b':' as u16
            && wide[6] == b'\\' as u16
        {
            return std::ffi::OsString::from_wide(&wide[verbatim_prefix.len()..]);
        }
    }

    path.as_os_str().to_owned()
}

fn is_reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            }
            #[cfg(not(windows))]
            {
                metadata.file_type().is_symlink()
            }
        })
        .unwrap_or(true)
}

fn assert_no_reparse_points(root: &Path) -> Result<(), AppError> {
    if is_reparse_point(root) {
        return Err(AppError::Engine(format!(
            "能力包目录不能是符号链接或重解析点: {}",
            root.display()
        )));
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| {
            AppError::Engine(format!(
                "读取能力包目录 {} 失败: {error}",
                directory.display()
            ))
        })? {
            let entry = entry.map_err(|error| AppError::Engine(error.to_string()))?;
            let path = entry.path();
            if is_reparse_point(&path) {
                return Err(AppError::Engine(format!(
                    "能力包包含符号链接或重解析点: {}",
                    path.display()
                )));
            }
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

fn pack_candidates(executable: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = executable.parent() {
        candidates.push(parent.join("content").join(PACK_DIRECTORY));
        candidates.push(parent.join("resources").join(PACK_DIRECTORY));
        candidates.push(parent.join(PACK_DIRECTORY));
    }
    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join(PACK_DIRECTORY));
        candidates.push(resource_dir.join("content").join(PACK_DIRECTORY));
    }
    candidates
}

fn find_pack(executable: &Path, resource_dir: Option<&Path>) -> Option<PathBuf> {
    pack_candidates(executable, resource_dir)
        .into_iter()
        .find(|candidate| candidate.is_dir() && !is_reparse_point(candidate))
}

fn validate_pack(root: &Path) -> Result<(String, PathBuf), AppError> {
    assert_no_reparse_points(root)?;
    let required = [
        "dependencies.json",
        "file-manifest.json",
        "checksums.sha256",
        "install-all.ps1",
        "install-to-codex.ps1",
        "runtime/provision-runtime.ps1",
    ];
    for relative in required {
        let path = root.join(relative);
        if !path.is_file() || is_reparse_point(&path) {
            return Err(AppError::Engine(format!(
                "能力包缺少必要文件: {}",
                path.display()
            )));
        }
    }
    Ok((pack_version(root)?, root.join("install-all.ps1")))
}

fn pack_version(root: &Path) -> Result<String, AppError> {
    let pack_path = root.join("pack.json");
    if !pack_path.is_file() || is_reparse_point(&pack_path) {
        return Err(AppError::Engine("能力包清单不是普通文件".into()));
    }
    let pack_bytes = fs::read(&pack_path)
        .map_err(|error| AppError::Engine(format!("读取能力包清单失败: {error}")))?;
    let value: serde_json::Value = serde_json::from_slice(strip_utf8_bom(&pack_bytes))
        .map_err(|error| AppError::Engine(format!("能力包清单 JSON 无效: {error}")))?;
    if value.get("schemaVersion").and_then(|v| v.as_u64()) != Some(2)
        || value.get("id").and_then(|v| v.as_str()) != Some(PACK_ID)
    {
        return Err(AppError::Engine(
            "能力包清单版本或包 ID 不受支持".to_string(),
        ));
    }
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(version)
}

#[derive(Debug, Clone, Default)]
struct ReadinessSummary {
    pack_identity_valid: bool,
    pack_version: Option<String>,
    core_ready: bool,
    skills: usize,
    agents: usize,
    runtime_status: String,
    config_status: String,
    feishu_runtime_status: String,
    media_runtime_status: String,
    media_protocol_status: String,
    media_registered: bool,
    media_enabled: bool,
    media_authorization_status: String,
    media_gateway_catalog_status: String,
    media_gateway_model_count: usize,
    media_gateway_catalog_verified_at: Option<String>,
    media_generation_capability: String,
    codex_restart_required: bool,
    codex_running: bool,
    diagnostics_path: Option<String>,
}

fn readiness_path(profile: &Path) -> PathBuf {
    profile.join(READINESS_RELATIVE)
}

fn core_ready_from_readiness(
    value: &serde_json::Value,
    pack_identity_valid: bool,
    skills: usize,
    agents: usize,
) -> bool {
    match value.pointer("/core/status").and_then(|v| v.as_str()) {
        Some(status) => status == "ready",
        // Readiness files from the pre-core-status schema are still readable,
        // but an explicit partial/failed state must never be upgraded by the
        // legacy count fallback.
        None => pack_identity_valid && skills > 0 && agents > 0,
    }
}

fn readiness_summary(codex_home: &Path) -> ReadinessSummary {
    let path = readiness_path(codex_home);
    let Ok(bytes) = fs::read(path) else {
        return ReadinessSummary {
            runtime_status: "unknown".to_string(),
            config_status: "unknown".to_string(),
            ..ReadinessSummary::default()
        };
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) else {
        return ReadinessSummary {
            runtime_status: "unknown".to_string(),
            config_status: "unknown".to_string(),
            ..ReadinessSummary::default()
        };
    };
    let pack_identity_valid = value.pointer("/pack/id").and_then(|v| v.as_str()) == Some(PACK_ID);
    let skills = value
        .pointer("/installed/skills")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let agents = value
        .pointer("/installed/agents")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let core_ready = core_ready_from_readiness(&value, pack_identity_valid, skills, agents);
    ReadinessSummary {
        pack_identity_valid,
        pack_version: value
            .pointer("/pack/version")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        core_ready,
        skills,
        agents,
        runtime_status: value
            .pointer("/runtime/status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        feishu_runtime_status: value
            .pointer("/runtime/feishu/health")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        media_runtime_status: value
            .pointer("/connections/mediaGeneration/runtime")
            .and_then(|v| v.as_str())
            .unwrap_or("not-checked")
            .to_string(),
        media_protocol_status: value
            .pointer("/connections/mediaGeneration/protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("not-checked")
            .to_string(),
        media_registered: value
            .pointer("/connections/mediaGeneration/registered")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        media_enabled: value
            .pointer("/connections/mediaGeneration/enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        media_authorization_status: read_media_authorization_status(codex_home),
        media_gateway_catalog_status: value
            .pointer("/connections/mediaGeneration/gatewayCatalog")
            .and_then(|v| v.as_str())
            .unwrap_or("not-checked")
            .to_string(),
        media_gateway_model_count: value
            .pointer("/connections/mediaGeneration/gatewayModelCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        media_gateway_catalog_verified_at: value
            .pointer("/connections/mediaGeneration/gatewayCatalogVerifiedAt")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        media_generation_capability: value
            .pointer("/connections/mediaGeneration/generationCapability")
            .and_then(|v| v.as_str())
            .unwrap_or("not-tested")
            .to_string(),
        codex_restart_required: value
            .pointer("/codex/restartRequired")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        codex_running: value
            .pointer("/codex/running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        config_status: value
            .pointer("/config/status")
            .or_else(|| value.pointer("/connections/codexConfig"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        diagnostics_path: value
            .pointer("/diagnostics/path")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        core_ready_from_readiness, legacy_media_names_owned, media_install_ready,
        media_install_switch, persist_connection_ownership, powershell_provider_path,
        readiness_path, readiness_summary, strip_utf8_bom, validate_options,
        write_media_authorization_status, CapabilityPackInstallOptions, ConnectionOwnershipRecord,
        CONNECTION_OWNERSHIP_RELATIVE, KIT_HOME_RELATIVE, PACK_ID,
    };
    use serde_json::json;
    use std::fs;

    #[cfg(windows)]
    #[test]
    fn capability_installer_timeout_reports_latest_stage_and_captures_output() {
        use super::{run_capability_installer, CapabilityProcessError, CapabilityProgressFile};
        use std::process::Command;
        use std::time::Duration;

        let progress = CapabilityProgressFile::create().unwrap();
        fs::write(progress.path(), "pack.validate").unwrap();
        let quoted_path = progress.path().to_string_lossy().replace('\'', "''");
        let script = format!(
            "[IO.File]::WriteAllText('{quoted_path}', 'runtime.python', [Text.Encoding]::ASCII); [Console]::WriteLine('process-started'); Start-Sleep -Seconds 8"
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let mut stages = Vec::new();
        let error = run_capability_installer(
            &mut command,
            progress.path(),
            Duration::from_secs(2),
            |stage| stages.push(stage.to_string()),
        )
        .unwrap_err();

        match error {
            CapabilityProcessError::Timeout { stage, output } => {
                assert_eq!(stage, "runtime.python");
                assert!(String::from_utf8_lossy(&output.stdout).contains("process-started"));
                assert!(!output.status.success());
            }
            other => panic!("expected timeout, got {other:?}"),
        }
        assert!(stages.iter().any(|stage| stage == "runtime.python"));
    }

    #[cfg(windows)]
    #[test]
    fn capability_installer_stops_descendants_that_hold_output_pipes() {
        use super::{run_capability_installer, CapabilityProgressFile};
        use std::process::Command;
        use std::time::{Duration, Instant};

        let progress = CapabilityProgressFile::create().unwrap();
        fs::write(progress.path(), "pack.validate").unwrap();
        let pid_file = std::env::temp_dir().join(format!(
            "codex-capability-descendant-{}.txt",
            uuid::Uuid::new_v4()
        ));
        let quoted_pid_path = pid_file.to_string_lossy().replace('\'', "''");
        let script = format!(
            r#"$si = [System.Diagnostics.ProcessStartInfo]::new(); $si.FileName = Join-Path $PSHOME 'powershell.exe'; $si.Arguments = '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'; $si.UseShellExecute = $false; $si.CreateNoWindow = $true; $child = [System.Diagnostics.Process]::Start($si); [IO.File]::WriteAllText('{quoted_pid_path}', [string]$child.Id); [Console]::Out.WriteLine('parent-finished')"#
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let started = Instant::now();
        let output = run_capability_installer(
            &mut command,
            progress.path(),
            Duration::from_secs(10),
            |_| {},
        )
        .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("parent-finished"));
        assert!(started.elapsed() < Duration::from_secs(8));

        let descendant_pid = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        let descendant_pid_text = descendant_pid.to_string();
        let process_list = Command::new("tasklist.exe")
            .args([
                "/FI",
                &format!("PID eq {descendant_pid_text}"),
                "/FO",
                "CSV",
                "/NH",
            ])
            .output()
            .unwrap();
        let process_list = String::from_utf8_lossy(&process_list.stdout);
        let descendant_running = process_list.contains(&format!(",\"{descendant_pid_text}\","));
        if descendant_running {
            let _ = Command::new("taskkill.exe")
                .args(["/PID", &descendant_pid_text, "/T", "/F"])
                .output();
        }
        let _ = fs::remove_file(pid_file);
        assert!(
            !descendant_running,
            "capability installer left a process alive after its parent exited"
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_provider_path_removes_only_verbatim_local_drive_prefix() {
        use std::ffi::OsString;
        use std::path::Path;

        assert_eq!(
            powershell_provider_path(Path::new(r"\\?\C:\Users\测试\usb-codex-home")),
            OsString::from(r"C:\Users\测试\usb-codex-home")
        );
        assert_eq!(
            powershell_provider_path(Path::new(r"C:\Users\测试\usb-codex-home")),
            OsString::from(r"C:\Users\测试\usb-codex-home")
        );
        assert_eq!(
            powershell_provider_path(Path::new(r"\\?\UNC\server\share\codex-home")),
            OsString::from(r"\\?\UNC\server\share\codex-home")
        );
    }

    #[test]
    fn explicit_partial_readiness_is_not_upgraded_by_counts() {
        let value = json!({"core": {"status": "partial"}});
        assert!(!core_ready_from_readiness(&value, true, 50, 7));
    }

    #[test]
    fn explicit_ready_readiness_is_accepted() {
        let value = json!({"core": {"status": "ready"}});
        assert!(core_ready_from_readiness(&value, true, 1, 1));
    }

    #[test]
    fn host_core_ignores_optional_failure_but_requires_live_media_files() {
        let home = std::env::temp_dir().join(format!("usb-host-core-{}", uuid::Uuid::new_v4()));
        let bin = home.join("codex-agent-kit/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(home.join("agents")).unwrap();
        for file in ["configure-codex.ps1", "start-media-generation-mcp.ps1", "media-generation-mcp.py", "python.exe", "node.exe"] {
            fs::write(bin.join(file), "fixture").unwrap();
        }
        let readiness = json!({"pack":{"id":PACK_ID,"version":"fixture"}, "core":{"status":"ready"},
            "config":{"merged":true,"status":"partial"}, "overallStatus":"partial",
            "runtime":{"python":{"executable":bin.join("python.exe")},"node":{"executable":bin.join("node.exe")},"hyperframes":{"status":"install-failed"}},
            "connections":{"mediaGeneration":{"runtime":"passed","protocol":"passed","gatewayCatalog":"failed"}}});
        let wrapper = bin.join("start-media-generation-mcp.ps1");
        let config: toml::Value = format!("[mcp_servers.media_generation]\nenabled=true\ncommand='powershell.exe'\nargs=['-File', '{}']\n", wrapper.display()).parse().unwrap();
        assert!(super::host_core_files_ready(&home, &config, &readiness, "fixture"));
        assert!(!super::host_core_files_ready(&home, &config, &readiness, "new-version"));
        fs::remove_file(&wrapper).unwrap();
        assert!(!super::host_core_files_ready(&home, &config, &readiness, "fixture"));
        let disabled: toml::Value = "[mcp_servers.media_generation]\nenabled=false".parse().unwrap();
        assert!(super::host_core_files_ready(&home, &disabled, &readiness, "fixture"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn host_repair_preserves_explicit_disabled_and_remote_selections() {
        let home = std::env::temp_dir().join(format!("usb-host-options-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "[mcp_servers.feishu]\nenabled=false\n[mcp_servers.media_generation]\nenabled=false\n[mcp_servers.cloudflare]\nenabled=true\n[mcp_servers.notion]\nenabled=false\n").unwrap();
        let options = super::host_install_options_with_settings(&home, std::collections::HashMap::new()).unwrap();
        assert!(!options.persist_connections);
        assert!(!options.enable_media_generation);
        assert!(!options.enable_feishu);
        assert_eq!(options.enabled_remote_mcps, vec!["cloudflare"]);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn feishu_apply_preserves_other_connection_settings() {
        use super::{
            feishu_environment_values, plan_feishu_config, preserve_feishu_selection,
            validate_feishu_options, FeishuConnectionAction,
        };

        let current = r#"model = "existing"
# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1
[mcp_servers.media_generation]
command = "existing-media"
enabled = true
[mcp_servers.cloudflare]
enabled = false
url = "https://example.invalid/mcp"
# END CODEX-STARTER-KIT MANAGED CONFIG v1
"#;
        let mut submitted = CapabilityPackInstallOptions {
            enable_feishu: true,
            feishu_action: Some(FeishuConnectionAction::Apply),
            feishu_app_id: "cli_fixture".into(),
            feishu_app_secret: "invalid".into(),
            ..CapabilityPackInstallOptions::default()
        };
        assert!(validate_feishu_options(&submitted).is_err());
        submitted.feishu_app_secret = " fixture-secret ".into();
        assert!(validate_feishu_options(&submitted).is_ok());
        let home = std::path::Path::new(r"C:\Users\Fixture\.codex");
        let planned = plan_feishu_config(current, home, &submitted).unwrap();
        assert!(planned.contains("# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1"));
        assert!(planned.contains("# END CODEX-STARTER-KIT MANAGED CONFIG v1"));
        let before: toml::Value = current.parse().unwrap();
        assert!(!planned.contains("fixture-secret"));
        assert!(!planned.contains("cli_fixture"));
        let parsed: toml::Value = planned.parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["feishu"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(parsed["model"], before["model"]);
        for server in ["media_generation", "cloudflare"] {
            assert_eq!(parsed["mcp_servers"][server], before["mcp_servers"][server]);
        }
        assert_eq!(
            parsed["mcp_servers"]["feishu"]["tools"]["drive.v1.file.delete"]["approval_mode"]
                .as_str(),
            Some("prompt"),
        );
        assert_eq!(
            feishu_environment_values(&submitted)[1],
            ("FEISHU_APP_SECRET", "fixture-secret")
        );

        let preserved =
            preserve_feishu_selection(CapabilityPackInstallOptions::default(), submitted.clone());
        assert!(preserved.enable_feishu);
        assert!(preserved.persist_connections);
        assert_eq!(preserved.feishu_auth_mode, "tenant");
        assert_eq!(preserved.feishu_app_id, "cli_fixture");

        submitted.enable_feishu = false;
        let disabled: toml::Value = plan_feishu_config(&planned, home, &submitted)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            disabled["mcp_servers"]["feishu"]["enabled"].as_bool(),
            Some(false)
        );
        for server in ["media_generation", "cloudflare"] {
            assert_eq!(
                parsed["mcp_servers"][server],
                disabled["mcp_servers"][server]
            );
        }
    }

    #[test]
    fn feishu_first_install_and_live_save_keep_the_existing_config() {
        use super::{feishu_can_save_live, host_install_options_with_settings, FeishuConnectionAction};

        let home = std::env::temp_dir().join(format!("usb-feishu-options-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&home).unwrap();
        let mut options = host_install_options_with_settings(&home, Default::default()).unwrap();
        assert!(options.enable_feishu);
        assert!(options.feishu_app_id.is_empty() && options.feishu_app_secret.is_empty());
        assert!(validate_options(&options).is_ok());

        options.feishu_action = Some(FeishuConnectionAction::Apply);
        assert!(validate_options(&options).is_err());
        options.feishu_app_id = "cli_fixture".into();
        options.feishu_app_secret = "fixture-secret".into();
        assert!(validate_options(&options).is_ok());

        let mut config: toml::Value = r#"
            model_provider = "custom"
            [mcp_servers.feishu]
            enabled = true
            command = "powershell.exe"
            env_vars = ["FEISHU_APP_ID", "FEISHU_APP_SECRET", "CODEX_USB_FEISHU_SETUP_ENTRY"]
            args = ["-File", "wrapper", "-PackageCache", "cache", "-AuthMode", "tenant", "-Domain", "https://open.feishu.cn"]
            [mcp_servers.media_generation]
            enabled = true
            [mcp_servers.existing]
            enabled = true
        "#.parse().unwrap();
        config["mcp_servers"]["feishu"]["args"].as_array_mut().unwrap()[1] =
            toml::Value::String(home.join("codex-agent-kit/bin/start-feishu-mcp.ps1").to_string_lossy().into_owned());
        config["mcp_servers"]["feishu"]["args"].as_array_mut().unwrap()[3] =
            toml::Value::String(home.join("codex-agent-kit/runtime/feishu").to_string_lossy().into_owned());
        assert!(feishu_can_save_live(&config, &home, &options));
        options.feishu_domain = "https://open.larksuite.com".into();
        assert!(!feishu_can_save_live(&config, &home, &options));

        config["mcp_servers"]["feishu"]["enabled"] = toml::Value::Boolean(false);
        let config_path = home.join("config.toml");
        fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
        let saved = [
            ("FEISHU_APP_ID".to_string(), "cli_saved".to_string()),
            ("FEISHU_APP_SECRET".to_string(), "saved-test-secret".to_string()),
            ("FEISHU_DOMAIN".to_string(), "https://open.larksuite.com".to_string()),
        ].into_iter().collect();
        let preserved = host_install_options_with_settings(&home, saved).unwrap();
        assert!(!preserved.enable_feishu);
        assert_eq!(preserved.feishu_app_id, "cli_saved");
        assert_eq!(preserved.feishu_domain, "https://open.larksuite.com");
        fs::remove_file(config_path).unwrap();
        fs::remove_dir(home).unwrap();
    }

    #[test]
    fn utf8_bom_is_ignored_for_readiness_json() {
        let profile =
            std::env::temp_dir().join(format!("codex-readiness-bom-test-{}", uuid::Uuid::new_v4()));
        let codex_home = profile.join(".codex");
        let readiness_path = readiness_path(&codex_home);
        fs::create_dir_all(readiness_path.parent().unwrap()).unwrap();
        let json = serde_json::to_vec(&json!({
            "pack": {"id": PACK_ID, "version": "1.2.1"},
            "core": {"status": "ready"},
            "installed": {"skills": 50, "agents": 7},
            "runtime": {"status": "ready"},
            "config": {"status": "merged"},
            "diagnostics": {"path": "diagnostics/capability-pack.log"}
        }))
        .unwrap();
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend(json);
        fs::write(&readiness_path, bytes).unwrap();

        let summary = readiness_summary(&codex_home);
        assert!(summary.pack_identity_valid);
        assert_eq!(summary.pack_version.as_deref(), Some("1.2.1"));
        assert!(summary.core_ready);
        assert_eq!(summary.skills, 50);
        assert_eq!(summary.agents, 7);
        assert_eq!(summary.runtime_status, "ready");
        assert_eq!(summary.config_status, "merged");
        let _ = fs::remove_dir_all(profile);
    }

    #[test]
    fn media_authorization_failure_then_recovery_updates_the_same_record() {
        let home = std::env::temp_dir().join(format!("media-auth-recovery-{}", uuid::Uuid::new_v4()));
        let failed = super::record_media_authorization(&home, true,
            Err(crate::errors::AppError::Engine("fixture credential failure".into())));
        assert!(failed.unwrap_err().to_string().contains("fixture credential failure"));
        assert_eq!(super::read_media_authorization_status(&home), "failed");
        super::record_media_authorization(&home, true, Ok(())).unwrap();
        assert_eq!(super::read_media_authorization_status(&home), "verified");
        super::record_media_authorization(&home, false, Ok(())).unwrap();
        assert_eq!(super::read_media_authorization_status(&home), "not-required");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn media_probe_uses_a_bounded_process_tree() {
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"]);
        let start = std::time::Instant::now();
        let error = super::run_capability_probe(&mut command).unwrap_err();
        assert!(error.to_string().contains("45 秒"));
        assert!(start.elapsed() < std::time::Duration::from_secs(55));
    }

    #[test]
    fn media_mcp_requires_registered_protocol_and_both_local_files() {
        let profile = std::env::temp_dir().join(format!(
            "codex-media-readiness-test-{}",
            uuid::Uuid::new_v4()
        ));
        let codex_home = profile.join(".codex");
        let kit = codex_home.join(KIT_HOME_RELATIVE);
        let readiness_path = kit.join("readiness.json");
        let bin = kit.join("bin");
        let connections = kit.join("connections");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&connections).unwrap();
        fs::write(bin.join("start-media-generation-mcp.ps1"), "wrapper").unwrap();
        fs::write(bin.join("media-generation-mcp.py"), "server").unwrap();
        fs::write(
            connections.join("media-generation.connection.json"),
            "connection",
        )
        .unwrap();
        fs::write(
            connections.join("media-generation.credential.dpapi"),
            "credential",
        )
        .unwrap();
        fs::write(
            &readiness_path,
            serde_json::to_vec(&json!({
                "connections": {"mediaGeneration": {
                    "runtime": "passed",
                    "protocol": "passed",
                    "registered": true,
                    "enabled": true
                }}
            }))
            .unwrap(),
        )
        .unwrap();

        write_media_authorization_status(&codex_home, "verified").unwrap();
        let readiness = readiness_summary(&codex_home);
        assert!(media_install_ready(&readiness, &codex_home));
        let mut pending_authorization = readiness.clone();
        pending_authorization.media_authorization_status = "pending".to_string();
        assert!(!media_install_ready(&pending_authorization, &codex_home));
        write_media_authorization_status(&codex_home, "pending").unwrap();
        let refreshed = readiness_summary(&codex_home);
        assert_eq!(refreshed.media_authorization_status, "pending");
        assert!(!media_install_ready(&refreshed, &codex_home));
        write_media_authorization_status(&codex_home, "verified").unwrap();
        assert!(media_install_ready(
            &readiness_summary(&codex_home),
            &codex_home
        ));
        let mut stale_schema = readiness.clone();
        stale_schema.media_enabled = false;
        assert!(!media_install_ready(&stale_schema, &codex_home));
        let mut unregistered = readiness.clone();
        unregistered.media_registered = false;
        assert!(!media_install_ready(&unregistered, &codex_home));
        let mut unhealthy = readiness.clone();
        unhealthy.media_protocol_status = "failed".to_string();
        assert!(!media_install_ready(&unhealthy, &codex_home));
        fs::remove_file(bin.join("media-generation-mcp.py")).unwrap();
        assert!(!media_install_ready(&readiness, &codex_home));
        fs::write(bin.join("media-generation-mcp.py"), "server").unwrap();
        fs::remove_file(connections.join("media-generation.credential.dpapi")).unwrap();
        assert!(!media_install_ready(&readiness, &codex_home));
        let _ = fs::remove_dir_all(profile);
    }

    #[test]
    fn media_disabled_is_explicitly_forwarded_to_the_powershell_installer() {
        assert_eq!(media_install_switch(true), "-EnableMediaGeneration");
        assert_eq!(media_install_switch(false), "-DisableMediaGeneration");
    }

    #[test]
    fn strip_utf8_bom_preserves_bomless_json() {
        let bytes = br#"{"ready":true}"#;
        assert_eq!(strip_utf8_bom(bytes), bytes);
        assert_eq!(strip_utf8_bom(&[0xef, 0xbb, 0xbf, b'{'])[0], b'{');
    }

    #[test]
    fn legacy_readiness_uses_identity_and_positive_counts() {
        let value = json!({});
        assert!(core_ready_from_readiness(&value, true, 50, 7));
        assert!(!core_ready_from_readiness(&value, false, 50, 7));
        assert!(!core_ready_from_readiness(&value, true, 0, 7));
    }

    #[test]
    fn connection_ownership_records_names_without_values_and_merges_previous_names() {
        let profile = std::env::temp_dir().join(format!(
            "codex-capability-ownership-test-{}",
            uuid::Uuid::new_v4()
        ));
        let codex_home = profile.join(".codex");
        fs::create_dir_all(&codex_home).unwrap();
        persist_connection_ownership(&codex_home, &["FEISHU_APP_SECRET"]).unwrap();
        persist_connection_ownership(&codex_home, &["DASHSCOPE_API_KEY"]).unwrap();
        let path = codex_home.join(CONNECTION_OWNERSHIP_RELATIVE);
        let record: ConnectionOwnershipRecord =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(record.schema_version, 1);
        assert_eq!(record.pack_id, PACK_ID);
        assert_eq!(
            record.names,
            vec![
                "DASHSCOPE_API_KEY".to_string(),
                "FEISHU_APP_SECRET".to_string()
            ]
        );
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("secret-value"));
        let _ = fs::remove_dir_all(profile);
    }

    #[test]
    fn legacy_media_migration_only_removes_names_owned_by_this_pack() {
        let record = ConnectionOwnershipRecord {
            schema_version: 1,
            pack_id: PACK_ID.to_string(),
            names: vec![
                "MEDIA_GENERATION_API_KEY".to_string(),
                "MEDIA_GENERATION_API_PROFILE".to_string(),
                "USER_CUSTOM_VALUE".to_string(),
            ],
            note: String::new(),
        };
        assert_eq!(
            legacy_media_names_owned(&record),
            vec!["MEDIA_GENERATION_API_PROFILE", "MEDIA_GENERATION_API_KEY"]
        );
        let foreign = ConnectionOwnershipRecord {
            pack_id: "another-pack".to_string(),
            ..record
        };
        assert!(legacy_media_names_owned(&foreign).is_empty());
    }

    #[test]
    fn minimax_video_uses_the_shared_media_connection_only() {
        let options = CapabilityPackInstallOptions {
            enable_media_generation: true,
            media_generation_api_profile: "wokey".to_string(),
            minimax_video_model: "upstream-minimax-video-model".to_string(),
            ..CapabilityPackInstallOptions::default()
        };
        assert!(validate_options(&options).is_ok());

        let license_backed_image = CapabilityPackInstallOptions {
            enable_media_generation: true,
            ..CapabilityPackInstallOptions::default()
        };
        assert!(validate_options(&license_backed_image).is_ok());

        let mut legacy_direct = options;
        legacy_direct
            .enabled_api_services
            .push("minimax-h3".to_string());
        assert!(validate_options(&legacy_direct).is_err());
    }
}

fn overall_status(core_ready: bool, runtime_status: &str, config_status: &str) -> &'static str {
    if !core_ready {
        return "failed";
    }
    if runtime_status == "ready" && !matches!(config_status, "failed" | "partial" | "conflict") {
        "ready"
    } else {
        "partial"
    }
}

fn media_install_ready(readiness: &ReadinessSummary, codex_home: &Path) -> bool {
    let kit_home = codex_home.join(KIT_HOME_RELATIVE);
    readiness.media_registered
        && readiness.media_enabled
        && readiness.media_authorization_status == "verified"
        && readiness.media_runtime_status == "passed"
        && readiness.media_protocol_status == "passed"
        && kit_home
            .join("bin/start-media-generation-mcp.ps1")
            .is_file()
        && kit_home.join("bin/media-generation-mcp.py").is_file()
        && kit_home
            .join("connections/media-generation.connection.json")
            .is_file()
        && kit_home
            .join("connections/media-generation.credential.dpapi")
            .is_file()
}

fn write_media_authorization_status(profile: &Path, status: &str) -> Result<(), AppError> {
    if !matches!(status, "pending" | "verified" | "failed" | "not-required") {
        return Err(AppError::Engine("媒体授权状态无效".to_string()));
    }
    let path = profile.join(MEDIA_AUTHORIZATION_RELATIVE);
    if path.exists() && is_reparse_point(&path) {
        return Err(AppError::Engine(
            "媒体授权状态文件不能是符号链接或重解析点".to_string(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Engine("无法确定媒体授权状态目录".to_string()))?;
    fs::create_dir_all(parent)
        .map_err(|error| AppError::Engine(format!("创建媒体授权状态目录失败: {error}")))?;
    if is_reparse_point(parent) {
        return Err(AppError::Engine(
            "媒体授权状态目录不能是符号链接或重解析点".to_string(),
        ));
    }
    let value = serde_json::json!({
        "schemaVersion": 1,
        "packId": PACK_ID,
        "status": status,
    });
    let encoded = serde_json::to_vec_pretty(&value)
        .map_err(|error| AppError::Engine(format!("编码媒体授权状态失败: {error}")))?;
    crate::app::atomic_file::write_atomic(&path, &encoded)
        .map_err(|error| AppError::Engine(format!("保存媒体授权状态失败: {error}")))
}

pub(crate) fn read_media_authorization_status(profile: &Path) -> String {
    let path = profile.join(MEDIA_AUTHORIZATION_RELATIVE);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return "unknown".to_string();
    };
    if !metadata.is_file() || is_reparse_point(&path) || metadata.len() > 4096 {
        return "unknown".to_string();
    }
    let Ok(bytes) = fs::read(path) else {
        return "unknown".to_string();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) else {
        return "unknown".to_string();
    };
    if value.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1)
        || value.get("packId").and_then(|v| v.as_str()) != Some(PACK_ID)
    {
        return "unknown".to_string();
    }
    value
        .get("status")
        .and_then(|v| v.as_str())
        .filter(|status| matches!(*status, "pending" | "verified" | "failed" | "not-required"))
        .unwrap_or("unknown")
        .to_string()
}

fn record_media_authorization(
    home: &Path,
    enabled: bool,
    result: Result<(), AppError>,
) -> Result<(), AppError> {
    let status = if !enabled { "not-required" } else if result.is_ok() { "verified" } else { "failed" };
    write_media_authorization_status(home, status)?;
    result
}

pub(crate) fn refresh_media_authorization(home: &Path) -> Result<(), AppError> {
    let result = crate::app::media_generation::sync_authorized_local_connection()
        .and_then(|_| crate::app::media_generation::verify_authorized_local_connection());
    record_media_authorization(home, true, result)
}

fn media_install_switch(enabled: bool) -> &'static str {
    if enabled {
        "-EnableMediaGeneration"
    } else {
        "-DisableMediaGeneration"
    }
}

fn sanitized_child_output(output: &Output, options: &CapabilityPackInstallOptions) -> String {
    let mut text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for secret in [
        &options.feishu_app_secret,
        &options.feishu_user_access_token,
        &options.dashscope_api_key,
    ] {
        if !secret.trim().is_empty() {
            text = text.replace(secret, "<redacted>");
        }
    }
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > 12 {
        lines = lines.split_off(lines.len() - 12);
    }
    let text = lines.join(" | ");
    let mut chars = text.chars().collect::<Vec<_>>();
    if chars.len() > 1600 {
        chars = chars.split_off(chars.len() - 1600);
    }
    chars.into_iter().collect()
}

struct CapabilityProgressFile(PathBuf);

impl CapabilityProgressFile {
    fn create() -> Result<Self, AppError> {
        let temp_dir = std::env::temp_dir();
        for attempt in 0..16u32 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = temp_dir.join(format!(
                "codex-capability-progress-{}-{nonce}-{attempt}.txt",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(AppError::Engine(format!("创建能力包进度通道失败: {error}")))
                }
            }
        }
        Err(AppError::Engine(
            "创建能力包进度通道失败：无法取得唯一临时文件名".to_string(),
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CapabilityProgressFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Debug)]
enum CapabilityProcessError {
    Spawn(io::Error),
    Containment(io::Error),
    Wait(io::Error),
    Timeout { stage: String, output: Output },
}

#[cfg(windows)]
struct CapabilityInstallerJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl CapabilityInstallerJob {
    fn create() -> io::Result<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self(handle))
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

#[cfg(windows)]
impl Drop for CapabilityInstallerJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        unsafe {
            TerminateJobObject(self.0, 1);
            CloseHandle(self.0);
        }
    }
}

fn wait_for_capability_child(
    child: &mut Child,
    timeout: Duration,
) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn terminate_capability_process_tree(
    child: &mut Child,
    #[cfg(windows)] job: &CapabilityInstallerJob,
) -> io::Result<ExitStatus> {
    #[cfg(windows)]
    {
        job.terminate();
    }
    let _ = child.kill();
    wait_for_capability_child(child, Duration::from_secs(5))?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "installer process did not exit after process-tree termination",
        )
    })
}

fn receive_process_pipe(receiver: &mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default()
}

fn run_capability_installer<F>(
    command: &mut Command,
    progress_path: &Path,
    timeout: Duration,
    mut on_progress: F,
) -> Result<Output, CapabilityProcessError>
where
    F: FnMut(&str),
{
    #[cfg(windows)]
    let job = CapabilityInstallerJob::create().map_err(CapabilityProcessError::Containment)?;
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(CapabilityProcessError::Spawn)?;
    #[cfg(windows)]
    {
        if let Err(error) = job.assign(&child) {
            let _ = child.kill();
            let _ = wait_for_capability_child(&mut child, Duration::from_secs(2));
            return Err(CapabilityProcessError::Containment(error));
        }
        if crate::app::model_process::resume_suspended(&child).is_err() {
            job.terminate();
            let _ = child.kill();
            let _ = wait_for_capability_child(&mut child, Duration::from_secs(2));
            return Err(CapabilityProcessError::Containment(io::Error::other(
                "could not resume suspended installer process",
            )));
        }
    }
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let (stdout_tx, stdout_rx) = mpsc::sync_channel(1);
    let (stderr_tx, stderr_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut reader = stdout;
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        let _ = stdout_tx.send(bytes);
    });
    thread::spawn(move || {
        let mut reader = stderr;
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        let _ = stderr_tx.send(bytes);
    });

    let mut stage = "pack.validate".to_string();
    on_progress(&stage);
    let started = Instant::now();
    let mut last_progress_poll = Instant::now();
    loop {
        let status = child.try_wait().map_err(CapabilityProcessError::Wait)?;
        if let Some(status) = status {
            #[cfg(windows)]
            job.terminate();
            let stdout = receive_process_pipe(&stdout_rx);
            let stderr = receive_process_pipe(&stderr_rx);
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if started.elapsed() >= timeout {
            #[cfg(windows)]
            let status = terminate_capability_process_tree(&mut child, &job)
                .map_err(CapabilityProcessError::Wait)?;
            #[cfg(not(windows))]
            let status = terminate_capability_process_tree(&mut child)
                .map_err(CapabilityProcessError::Wait)?;
            return Err(CapabilityProcessError::Timeout {
                stage,
                output: Output {
                    status,
                    stdout: receive_process_pipe(&stdout_rx),
                    stderr: receive_process_pipe(&stderr_rx),
                },
            });
        }
        if last_progress_poll.elapsed() >= CAPABILITY_PROGRESS_POLL_INTERVAL {
            last_progress_poll = Instant::now();
            if let Ok(raw_stage) = fs::read_to_string(progress_path) {
                let next_stage = raw_stage.trim();
                if CAPABILITY_INSTALL_STAGES.contains(&next_stage) && next_stage != stage {
                    stage = next_stage.to_string();
                    on_progress(&stage);
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The local credential probe uses the installer's existing bounded process
/// tree and pipe cleanup; its output is parsed by the media caller, not logged.
pub(super) fn run_capability_probe(command: &mut Command) -> Result<Output, AppError> {
    let progress = CapabilityProgressFile::create()?;
    run_capability_installer(command, progress.path(), Duration::from_secs(45), |_| {})
        .map_err(|error| match error {
            CapabilityProcessError::Timeout { .. } => AppError::Engine(
                "媒体 MCP 授权自检超过 45 秒，已停止本次检查".into()),
            CapabilityProcessError::Spawn(e) => AppError::Engine(format!(
                "无法启动媒体 MCP 授权自检（系统错误码 {:?}）", e.raw_os_error())),
            CapabilityProcessError::Containment(e) | CapabilityProcessError::Wait(e) =>
                AppError::Engine(format!("媒体 MCP 授权自检进程未正常完成（系统错误码 {:?}）", e.raw_os_error())),
        })
}

fn capability_stage_label(stage: &str) -> &'static str {
    match stage {
        "pack.validate" => "检查能力包清单",
        "pack.skills" => "安装 Skills 与 Agents",
        "runtime.verify" => "检查随包运行时文件",
        "runtime.python" => "准备 Python 和离线依赖",
        "runtime.manim" => "准备 Manim 环境",
        "runtime.nodeMedia" => "准备 Node、FFmpeg 和本地媒体运行时",
        "runtime.hyperframes" => "从本地部署 HyperFrames CLI 和依赖",
        "runtime.hyperframesFfmpeg" => "从本地部署 HyperFrames 视频编码器",
        "runtime.hyperframesCliCheck" => "检查 HyperFrames CLI",
        "runtime.hyperframesBrowser" => "从本地部署 HyperFrames 浏览器",
        "runtime.hyperframesBrowserCheck" => "验证本地 HyperFrames 浏览器能否启动",
        "runtime.feishu" => "部署本地飞书 MCP",
        "runtime.environment" => "设置命令入口和用户环境",
        "runtime.complete" => "写入运行时状态",
        "pack.pdf" => "检查 PDF 工具",
        "pack.media" => "验证媒体 MCP 本地连接",
        "pack.config" => "生成并合并 Codex 配置",
        "pack.models" => "同步模型目录",
        "pack.finalize" => "整理能力包安装记录",
        _ => "安装能力包",
    }
}

/// Install the bundled Product Design plugin into the USB edition's isolated
/// CODEX_HOME. This is called from the Skill Hub apply action so the plugin is
/// available alongside the user's Skill Hub skills without touching the host
/// Codex profile.
pub fn install_product_design_plugin(app: &AppHandle) -> Result<u32, AppError> {
    if !cfg!(target_os = "windows") {
        return Err(AppError::Engine(
            "离线 Product Design 插件安装仅支持 Windows U 盘版".to_string(),
        ));
    }
    if !crate::app::usb_guard::guard_required() {
        return Err(AppError::Engine(
            "离线 Product Design 插件只允许安装到 U 盘版 CODEX_HOME".to_string(),
        ));
    }
    let codex_home = crate::app::usb_guard::local_data_paths()?.codex_home;
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let resource_dir = app.path().resource_dir().ok();
    let Some(pack_root) = find_pack(&executable, resource_dir.as_deref()) else {
        return Err(AppError::Engine(
            "未找到 U 盘安装包内的离线 Product Design 插件".to_string(),
        ));
    };
    validate_pack(&pack_root)?;
    let installer = pack_root.join("install-product-design-plugin.ps1");
    if !installer.is_file() || is_reparse_point(&installer) {
        return Err(AppError::Engine(
            "U 盘安装包缺少离线 Product Design 安装脚本".to_string(),
        ));
    }
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
    command
        .arg(&installer)
        .arg("-PackRoot")
        .arg(&pack_root)
        .arg("-TargetCodexHome")
        .arg(powershell_provider_path(&codex_home));
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .map_err(|error| AppError::Engine(format!("启动离线 Product Design 安装失败: {error}")))?;
    if !output.status.success() {
        return Err(AppError::Engine(format!(
            "离线 Product Design 安装失败，PowerShell 退出码: {}",
            output.status
        )));
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| AppError::Engine(format!("离线 Product Design 安装结果无效: {error}")))?;
    let ready = report.get("status").and_then(|value| value.as_str()) == Some("ready")
        && report.get("plugin").and_then(|value| value.as_str()) == Some("product-design")
        && report.get("marketplace").and_then(|value| value.as_str())
            == Some("usb-product-design-offline")
        && report.get("enabled").and_then(|value| value.as_bool()) == Some(true)
        && report.get("runtimeReady").and_then(|value| value.as_bool()) == Some(true)
        && report.get("skillCount").and_then(|value| value.as_u64()) == Some(10);
    if !ready {
        return Err(AppError::Engine(
            "离线 Product Design 安装未通过就绪校验".to_string(),
        ));
    }
    log::info!(
        "USB offline Product Design plugin ready in {}",
        codex_home.display()
    );
    Ok(report
        .get("skillCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as u32)
}

/// Core launch requirements only; optional/network readiness is not a launch gate.
pub(crate) fn host_core_ready(home: &Path, config: &toml::Value) -> Result<bool, AppError> {
    let executable = std::env::current_exe().map_err(|e| AppError::Internal(e.to_string()))?;
    let Some(source) = find_pack(&executable, None) else { return Ok(false) };
    let version = pack_version(&source)?;
    let Some(bytes) = crate::app::usb_host::read_optional(&readiness_path(home))? else { return Ok(false) };
    let Ok(readiness) = serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) else { return Ok(false) };
    Ok(host_core_files_ready(home, config, &readiness, &version))
}

fn host_core_files_ready(home: &Path, config: &toml::Value, readiness: &serde_json::Value, version: &str) -> bool {
    if readiness.pointer("/pack/id").and_then(|v| v.as_str()) != Some(PACK_ID)
        || readiness.pointer("/pack/version").and_then(|v| v.as_str()) != Some(version)
        || readiness.pointer("/core/status").and_then(|v| v.as_str()) != Some("ready")
        || readiness.pointer("/config/merged").and_then(|v| v.as_bool()) != Some(true)
        || !home.join("agents").is_dir()
        || !home.join("codex-agent-kit/bin/configure-codex.ps1").is_file() {
        return false;
    }
    for runtime in ["python", "node"] {
        if !readiness.pointer(&format!("/runtime/{runtime}/executable"))
            .and_then(|v| v.as_str()).is_some_and(|p| Path::new(p).is_file()) { return false; }
    }
    let Some(media) = config.get("mcp_servers").and_then(|v| v.get("media_generation")) else { return false };
    // A deliberate disable is a user setting, not an installation failure.
    if media.get("enabled").and_then(|v| v.as_bool()) == Some(false) { return true; }
    let wrapper = home.join("codex-agent-kit/bin/start-media-generation-mcp.ps1");
    let command_exists = media.get("command").and_then(|v| v.as_str())
        .is_some_and(|p| p.eq_ignore_ascii_case("powershell.exe") || Path::new(p).is_file());
    let wrapper_registered = media.get("args").and_then(|v| v.as_array()).is_some_and(|args|
        args.windows(2).any(|pair| pair[0].as_str() == Some("-File")
            && pair[1].as_str().is_some_and(|p| Path::new(p) == wrapper)));
    command_exists && wrapper_registered && wrapper.is_file()
        && home.join("codex-agent-kit/bin/media-generation-mcp.py").is_file()
        && readiness.pointer("/connections/mediaGeneration/runtime").and_then(|v| v.as_str()) == Some("passed")
        && readiness.pointer("/connections/mediaGeneration/protocol").and_then(|v| v.as_str()) == Some("passed")
}

/// Reinstall using this user's choices, never empty defaults that disable
/// existing integrations or erase saved routing values.
pub(crate) fn host_install_options(home: &Path) -> Result<CapabilityPackInstallOptions, AppError> {
    let saved = crate::app::media_generation::current_user_environment_settings(OWNED_CONNECTION_ENV_NAMES)?
        .into_iter().collect();
    host_install_options_with_settings(home, saved)
}

fn host_install_options_with_settings(home: &Path, saved: std::collections::HashMap<String, String>) -> Result<CapabilityPackInstallOptions, AppError> {
    let config = crate::app::usb_host::read_config(home)?;
    let enabled = |name: &str, default: bool| config.get("mcp_servers").and_then(|v| v.get(name))
        .map(|v| v.get("enabled").and_then(toml::Value::as_bool) != Some(false)).unwrap_or(default);
    let setting = |key: &str| saved.get(key).cloned().unwrap_or_default();
    let mut options = CapabilityPackInstallOptions {
        persist_connections: false,
        enable_feishu: enabled("feishu", true),
        enable_media_generation: enabled("media_generation", true),
        enabled_remote_mcps: ["cloudflare", "figma", "linear", "notion"].into_iter()
            .filter(|id| enabled(id, false)).map(str::to_owned).collect(),
        feishu_app_id: setting("FEISHU_APP_ID"),
        feishu_app_secret: setting("FEISHU_APP_SECRET"),
        feishu_user_access_token: setting("FEISHU_USER_ACCESS_TOKEN"),
        dashscope_api_key: setting("DASHSCOPE_API_KEY"),
        dashscope_model: setting("DASHSCOPE_MODEL"),
        media_generation_agent_id: setting("MEDIA_GENERATION_AGENT_ID"),
        image2_model: setting("MEDIA_IMAGE2_MODEL"),
        grok_image_model: setting("MEDIA_GROK_IMAGE_MODEL"),
        grok_video_model: setting("MEDIA_GROK_VIDEO_MODEL"),
        seedance_model: setting("MEDIA_SEEDANCE_MODEL"),
        minimax_video_model: setting("MEDIA_MINIMAX_VIDEO_MODEL"),
        ..Default::default()
    };
    for (key, target) in [("FEISHU_DOMAIN", &mut options.feishu_domain),
        ("FEISHU_AUTH_MODE", &mut options.feishu_auth_mode),
        ("DASHSCOPE_BASE_URL", &mut options.dashscope_base_url),
        ("MEDIA_GENERATION_API_PROFILE", &mut options.media_generation_api_profile)] {
        let value = setting(key);
        if !value.is_empty() { *target = value; }
    }
    if let Some(bytes) = crate::app::usb_host::read_optional(&readiness_path(home))? {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) {
            if value.pointer("/connections/apiServices/enabled/dashscope-web-search").and_then(|v| v.as_bool()) == Some(true) {
                options.enabled_api_services.push("dashscope-web-search".into());
            }
        }
    }
    Ok(options)
}

fn preserve_feishu_selection(
    mut options: CapabilityPackInstallOptions,
    feishu: CapabilityPackInstallOptions,
) -> CapabilityPackInstallOptions {
    options.feishu_action = Some(FeishuConnectionAction::Preserve);
    options.enable_feishu = feishu.enable_feishu;
    options.feishu_app_id = feishu.feishu_app_id;
    options.feishu_app_secret = feishu.feishu_app_secret;
    options.feishu_user_access_token = feishu.feishu_user_access_token;
    options.feishu_auth_mode = feishu.feishu_auth_mode;
    options.feishu_domain = feishu.feishu_domain;
    options
}

fn feishu_config_enabled(home: &Path) -> Result<bool, AppError> {
    let config = crate::app::usb_host::read_config(home)?;
    Ok(config
        .get("mcp_servers")
        .and_then(|value| value.get("feishu"))
        .is_some_and(|server| {
            server.is_table() && server.get("enabled").and_then(toml::Value::as_bool) != Some(false)
        }))
}

fn feishu_credentials_valid(app_id: &str, app_secret: &str) -> bool {
    app_id.trim().strip_prefix("cli_").is_some_and(|id| {
        !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    }) && app_secret.trim().chars().count() >= 8
}

fn feishu_configured() -> bool {
    crate::app::media_generation::current_user_environment_settings(&["FEISHU_APP_ID", "FEISHU_APP_SECRET"])
        .ok()
        .is_some_and(|values| {
            let setting = |name| values.iter().find(|(key, _)| key == name).map_or("", |(_, value)| value.as_str());
            feishu_credentials_valid(setting("FEISHU_APP_ID"), setting("FEISHU_APP_SECRET"))
        })
}

fn feishu_can_save_live(
    config: &toml::Value,
    home: &Path,
    options: &CapabilityPackInstallOptions,
) -> bool {
    if !options.enable_feishu || options.feishu_auth_mode != "tenant" {
        return false;
    }
    let Some(server) = config.get("mcp_servers").and_then(|v| v.get("feishu")) else {
        return false;
    };
    let Some(args) = server.get("args").and_then(toml::Value::as_array) else {
        return false;
    };
    let wrapper = home.join("codex-agent-kit/bin/start-feishu-mcp.ps1");
    let package_cache = home.join("codex-agent-kit/runtime/feishu");
    let argument = |flag: &str| {
        args.windows(2).find_map(|pair| {
            (pair[0].as_str() == Some(flag))
                .then(|| pair[1].as_str())
                .flatten()
        })
    };
    server.get("enabled").and_then(toml::Value::as_bool) == Some(true)
        && server.get("command").and_then(toml::Value::as_str) == Some("powershell.exe")
        && server
            .get("env_vars")
            .and_then(toml::Value::as_array)
            .is_some_and(|vars| {
                vars.iter()
                    .any(|value| value.as_str() == Some("CODEX_USB_FEISHU_SETUP_ENTRY"))
            })
        && argument("-File")
            .is_some_and(|p| codex_win_engine::same_windows_path(Path::new(p), &wrapper))
        && argument("-PackageCache")
            .is_some_and(|p| codex_win_engine::same_windows_path(Path::new(p), &package_cache))
        && argument("-AuthMode") == Some("tenant")
        && argument("-Domain").is_some_and(|domain| {
            domain.eq_ignore_ascii_case(options.feishu_domain.trim().trim_end_matches('/'))
        })
}

pub fn status(app: &AppHandle) -> Result<CapabilityPackReport, AppError> {
    if !cfg!(target_os = "windows") {
        return Ok(report_missing("当前平台不支持 Windows 能力包安装"));
    }
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let resource_dir = app.path().resource_dir().ok();
    let Some(source) = find_pack(&executable, resource_dir.as_deref()) else {
        return Ok(report_missing(
            "未找到 content\\codex-starter-kit；请使用包含能力包的完整 U 盘发行目录",
        ));
    };
    let (version, _) = validate_pack(&source)?;
    let profile = user_profile()?;
    let usb_codex_home = if crate::app::usb_guard::guard_required() {
        Some(crate::app::usb_guard::local_data_paths()?.codex_home)
    } else {
        None
    };
    let codex_home = usb_codex_home.unwrap_or_else(|| profile.join(".codex"));
    Ok(pack_status_at(&source, &version, &codex_home, true))
}

fn pack_status_at(
    source: &Path,
    version: &str,
    codex_home: &Path,
    probe_media: bool,
) -> CapabilityPackReport {
    let readiness = readiness_summary(codex_home);
    let installed = readiness.pack_identity_valid
        && readiness.core_ready
        && readiness.pack_version.as_deref() == Some(version);
    let runtime_ready = installed && readiness.runtime_status == "ready";
    let media_needs_auth = readiness.media_enabled || readiness.media_registered;
    let media_auth_live = !probe_media
        || !readiness.media_enabled
        || crate::app::media_generation::verify_authorized_local_connection().is_ok();
    let effective_config_status = if media_needs_auth
        && (readiness.media_authorization_status != "verified" || !media_auth_live)
    {
        "partial"
    } else {
        readiness.config_status.as_str()
    };
    let status = overall_status(
        installed,
        &readiness.runtime_status,
        effective_config_status,
    );
    CapabilityPackReport {
        available: true,
        installed,
        success: status == "ready",
        core_ready: installed,
        overall_status: status.to_string(),
        source_path: Some(source.to_string_lossy().into_owned()),
        pack_version: Some(readiness.pack_version.unwrap_or_else(|| version.to_owned())),
        skills: readiness.skills,
        agents: readiness.agents,
        runtime_ready,
        runtime_status: readiness.runtime_status.clone(),
        config_status: effective_config_status.to_string(),
        diagnostics_path: readiness.diagnostics_path,
        feishu_enabled: feishu_config_enabled(codex_home).unwrap_or(false),
        feishu_configured: feishu_configured(),
        feishu_saved_live: false,
        feishu_runtime_status: readiness.feishu_runtime_status,
        enabled_api_services: Vec::new(),
        media_runtime_status: readiness.media_runtime_status.clone(),
        media_protocol_status: readiness.media_protocol_status.clone(),
        media_gateway_catalog_status: readiness.media_gateway_catalog_status.clone(),
        media_gateway_model_count: readiness.media_gateway_model_count,
        media_gateway_catalog_verified_at: readiness.media_gateway_catalog_verified_at.clone(),
        media_generation_capability: readiness.media_generation_capability.clone(),
        codex_restart_required: readiness.codex_restart_required,
        codex_running: readiness.codex_running,
        message: if status == "ready" {
            "能力包已安装；外部连接状态按本机配置决定".to_string()
        } else if installed {
            format!(
                "核心 Skills/Agents 已安装；运行时状态为 {}，配置状态为 {}，可从组件修复入口继续处理",
                readiness.runtime_status, effective_config_status
            )
        } else {
            "能力包已就绪，可安装或修复全部 Skills、Agents、运行时和本地 MCP 资源".to_string()
        },
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn add_env(command: &mut Command, name: &str, value: &str) {
    if let Some(value) = non_empty(value) {
        command.env(name, value);
    }
}

fn selected_service(options: &CapabilityPackInstallOptions, id: &str) -> bool {
    options
        .enabled_api_services
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case(id))
}

fn validate_feishu_options(options: &CapabilityPackInstallOptions) -> Result<(), AppError> {
    if !options.enable_feishu {
        return Ok(());
    }
    let pending = options.feishu_auth_mode == "tenant"
        && options.feishu_action != Some(FeishuConnectionAction::Apply)
        && non_empty(&options.feishu_app_id).is_none()
        && non_empty(&options.feishu_app_secret).is_none();
    if !pending && !feishu_credentials_valid(&options.feishu_app_id, &options.feishu_app_secret) {
        return Err(AppError::Engine(
            "请填写有效的飞书 App ID 和 App Secret".into(),
        ));
    }
    if !matches!(
        options.feishu_auth_mode.as_str(),
        "tenant" | "oauth" | "user"
    ) || !["https://open.feishu.cn", "https://open.larksuite.com"]
        .iter()
        .any(|domain| {
            options
                .feishu_domain
                .trim()
                .trim_end_matches('/')
                .eq_ignore_ascii_case(domain)
        })
    {
        return Err(AppError::Engine("飞书认证方式或 API 地址不受支持".into()));
    }
    if options.feishu_auth_mode == "user" && non_empty(&options.feishu_user_access_token).is_none()
    {
        return Err(AppError::Engine(
            "飞书 user 认证模式必须填写 User Access Token".into(),
        ));
    }
    Ok(())
}

fn validate_options(options: &CapabilityPackInstallOptions) -> Result<(), AppError> {
    if !matches!(
        options.media_gateway_catalog_status.as_str(),
        "not-checked" | "verified" | "failed"
    ) || options.media_gateway_model_count > 100_000
        || (options.media_gateway_catalog_status == "verified"
            && options
                .media_gateway_catalog_verified_at
                .as_deref()
                .and_then(non_empty)
                .is_none())
    {
        return Err(AppError::Engine("媒体上游模型目录检查状态无效".to_string()));
    }
    for service in &options.enabled_api_services {
        if !matches!(service.trim(), "dashscope-web-search") {
            return Err(AppError::Engine(format!(
                "不支持的能力包外部服务: {service}"
            )));
        }
    }
    for server in &options.enabled_remote_mcps {
        if !matches!(server.trim(), "cloudflare" | "figma" | "linear" | "notion") {
            return Err(AppError::Engine(format!("不支持的远程 MCP 服务: {server}")));
        }
    }
    validate_feishu_options(options)?;
    if selected_service(options, "dashscope-web-search")
        && (non_empty(&options.dashscope_api_key).is_none()
            || non_empty(&options.dashscope_base_url).is_none()
            || non_empty(&options.dashscope_model).is_none())
    {
        return Err(AppError::Engine(
            "启用 DashScope 搜索时必须填写 API Key、Base URL 和模型".to_string(),
        ));
    }
    if options.enable_media_generation {
        if !matches!(
            options.media_generation_api_profile.trim(),
            "auto" | "legacy" | "wokey" | "responses"
        ) {
            return Err(AppError::Engine("媒体 API 兼容模式无效".to_string()));
        }
        let has_image2 = non_empty(&options.image2_model).is_some();
        let has_seedance = non_empty(&options.seedance_model).is_some();
        let has_minimax = non_empty(&options.minimax_video_model).is_some();
        let uses_wokey = options.media_generation_api_profile == "wokey";
        if has_minimax && !uses_wokey {
            return Err(AppError::Engine("MiniMax 视频模型槽位仅可用于统一上游的 Wokey 兼容模式；当前不得配置官方 MiniMax API".to_string()));
        }
        let needs_image_route_id = has_image2
            && matches!(
                options.media_generation_api_profile.as_str(),
                "auto" | "legacy"
            );
        let needs_seedance_route_id = has_seedance && !uses_wokey;
        if (needs_image_route_id || needs_seedance_route_id)
            && non_empty(&options.media_generation_agent_id).is_none()
        {
            return Err(AppError::Engine(
                "当前媒体兼容模式下使用 Image-2 或 Seedance 时必须填写服务方提供的路由 ID"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn update_user_environment(
    values: &[(&str, &str)],
    target_user_sid: Option<&str>,
) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    let mut desktop_hive = std::ptr::null_mut();
    let root = if let Some(user_sid) = target_user_sid {
        let sid_key = user_sid
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_USERS,
                sid_key.as_ptr(),
                0,
                KEY_CREATE_SUB_KEY | KEY_QUERY_VALUE,
                &mut desktop_hive,
            )
        };
        if status != 0 || desktop_hive.is_null() {
            return Err(AppError::Engine(format!(
                "打开当前桌面用户注册表配置失败（Win32 {status}）"
            )));
        }
        desktop_hive
    } else {
        HKEY_CURRENT_USER
    };
    let _desktop_hive = (!desktop_hive.is_null()).then_some(Key(desktop_hive));
    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(AppError::Engine(format!(
            "写入用户环境变量失败（Win32 {status}）"
        )));
    }
    let _key = Key(handle);
    let valid_name = |name: &str| {
        !name.is_empty()
            && name.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
    };
    for (name, value) in values {
        if !valid_name(name) {
            return Err(AppError::Engine(format!("拒绝写入非法环境变量名: {name}")));
        }
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let value_wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let status = unsafe {
            RegSetValueExW(
                handle,
                name_wide.as_ptr(),
                0,
                REG_SZ,
                value_wide.as_ptr().cast(),
                (value_wide.len() * std::mem::size_of::<u16>()) as u32,
            )
        };
        if status != 0 {
            return Err(AppError::Engine(format!(
                "写入当前用户环境变量 {name} 失败（Win32 {status}）"
            )));
        }
        std::env::set_var(name, value);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn update_user_environment(
    _values: &[(&str, &str)],
    _target_user_sid: Option<&str>,
) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn remove_owned_user_environment(
    names: &[&str],
    target_user_sid: Option<&str>,
) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    let mut desktop_hive = std::ptr::null_mut();
    let (root, subkey_text) = match target_user_sid {
        Some(user_sid) => {
            let sid_key = user_sid
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_USERS,
                    sid_key.as_ptr(),
                    0,
                    KEY_QUERY_VALUE,
                    &mut desktop_hive,
                )
            };
            if status != 0 || desktop_hive.is_null() {
                return Err(AppError::Engine(format!(
                    "打开当前桌面用户注册表配置失败（Win32 {status}）"
                )));
            }
            (desktop_hive, "Environment\0".to_string())
        }
        None => (HKEY_CURRENT_USER, "Environment\0".to_string()),
    };
    let _desktop_hive = (!desktop_hive.is_null()).then_some(Key(desktop_hive));
    let subkey: Vec<u16> = subkey_text.encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    let status = unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_SET_VALUE, &mut handle) };
    if status == 2 {
        return Ok(());
    }
    if status != 0 {
        return Err(AppError::Engine(format!(
            "读取当前用户环境变量失败（Win32 {status}）"
        )));
    }
    let _key = Key(handle);
    for name in names {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let status = unsafe { RegDeleteValueW(handle, wide.as_ptr()) };
        if status != 0 && status != 2 {
            return Err(AppError::Engine(format!(
                "清理旧媒体环境变量 {name} 失败（Win32 {status}）"
            )));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn remove_owned_user_environment(
    _names: &[&str],
    _target_user_sid: Option<&str>,
) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

fn persist_connection_ownership(profile: &Path, names: &[&str]) -> Result<(), AppError> {
    let ownership_path = profile.join(CONNECTION_OWNERSHIP_RELATIVE);
    let ownership_parent = ownership_path
        .parent()
        .ok_or_else(|| AppError::Engine("无法确定连接所有权记录目录".to_string()))?;
    if ownership_parent.exists() && is_reparse_point(ownership_parent) {
        return Err(AppError::Engine(
            "连接所有权记录目录不能是符号链接或重解析点".to_string(),
        ));
    }
    if ownership_path.exists() && is_reparse_point(&ownership_path) {
        return Err(AppError::Engine(
            "连接所有权记录不能是符号链接或重解析点".to_string(),
        ));
    }
    fs::create_dir_all(ownership_parent)
        .map_err(|error| AppError::Engine(format!("创建连接所有权记录目录失败: {error}")))?;

    let mut owned_names = Vec::<String>::new();
    if ownership_path.is_file() {
        let bytes = fs::read(&ownership_path)
            .map_err(|error| AppError::Engine(format!("读取连接所有权记录失败: {error}")))?;
        if bytes.len() > 64 * 1024 {
            return Err(AppError::Engine("连接所有权记录超过大小限制".to_string()));
        }
        let value = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let previous: ConnectionOwnershipRecord = serde_json::from_slice(value)
            .map_err(|error| AppError::Engine(format!("连接所有权记录 JSON 无效: {error}")))?;
        if previous.schema_version != 1 || previous.pack_id != PACK_ID {
            return Err(AppError::Engine(
                "连接所有权记录身份不匹配；为避免误删，未覆盖原记录".to_string(),
            ));
        }
        owned_names = previous.names;
    }
    for name in names {
        if !OWNED_CONNECTION_ENV_NAMES.contains(name) {
            return Err(AppError::Engine(format!(
                "拒绝记录未声明的连接环境变量: {name}"
            )));
        }
        if !owned_names.iter().any(|existing| existing == name) {
            owned_names.push((*name).to_string());
        }
    }
    owned_names.sort();
    owned_names.dedup();
    let record = ConnectionOwnershipRecord {
        schema_version: 1,
        pack_id: PACK_ID.to_string(),
        names: owned_names,
        note:
            "Only environment variable names are recorded; values and credentials are never stored."
                .to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&record)
        .map_err(|error| AppError::Engine(format!("生成连接所有权记录失败: {error}")))?;
    fs::write(&ownership_path, bytes)
        .map_err(|error| AppError::Engine(format!("写入连接所有权记录失败: {error}")))
}

fn migrate_legacy_media_environment(profile: &Path) -> Result<(), AppError> {
    let ownership_path = profile.join(CONNECTION_OWNERSHIP_RELATIVE);
    if !ownership_path.is_file() {
        return Ok(());
    }
    if is_reparse_point(&ownership_path) {
        return Err(AppError::Engine(
            "连接所有权记录不能是符号链接或重解析点".to_string(),
        ));
    }
    let bytes = fs::read(&ownership_path)
        .map_err(|error| AppError::Engine(format!("读取连接所有权记录失败: {error}")))?;
    if bytes.len() > 64 * 1024 {
        return Err(AppError::Engine("连接所有权记录超过大小限制".to_string()));
    }
    let value = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let mut record: ConnectionOwnershipRecord = serde_json::from_slice(value)
        .map_err(|error| AppError::Engine(format!("连接所有权记录 JSON 无效: {error}")))?;
    if record.schema_version != 1 || record.pack_id != PACK_ID {
        return Ok(());
    }
    let owned = legacy_media_names_owned(&record);
    if owned.is_empty() {
        return Ok(());
    }
    let target_user_sid = crate::app::usb_guard::user_environment_registry_sid()?;
    remove_owned_user_environment(&owned, target_user_sid.as_deref())?;
    for name in &owned {
        std::env::remove_var(name);
    }
    record
        .names
        .retain(|name| !owned.iter().any(|owned_name| name == owned_name));
    let encoded = serde_json::to_vec_pretty(&record)
        .map_err(|error| AppError::Engine(format!("编码连接所有权记录失败: {error}")))?;
    crate::app::atomic_file::write_atomic(&ownership_path, &encoded)
        .map_err(|error| AppError::Engine(format!("更新连接所有权记录失败: {error}")))
}

fn legacy_media_names_owned(record: &ConnectionOwnershipRecord) -> Vec<&'static str> {
    if record.schema_version != 1 || record.pack_id != PACK_ID {
        return Vec::new();
    }
    [
        "MEDIA_GENERATION_API_BASE_URL",
        "MEDIA_GENERATION_API_PROFILE",
        "MEDIA_GENERATION_API_KEY",
    ]
    .into_iter()
    .filter(|name| record.names.iter().any(|existing| existing == name))
    .collect()
}

fn persist_configured_environment(
    options: &CapabilityPackInstallOptions,
    profile: &Path,
) -> Result<(), AppError> {
    if !options.persist_connections {
        return Ok(());
    }
    let mut values = Vec::<(&str, &str)>::new();
    if options.enable_feishu && options.feishu_action != Some(FeishuConnectionAction::Preserve) {
        values.extend(feishu_environment_values(options));
    }
    if selected_service(options, "dashscope-web-search") {
        values.extend([
            ("DASHSCOPE_API_KEY", options.dashscope_api_key.as_str()),
            ("DASHSCOPE_BASE_URL", options.dashscope_base_url.as_str()),
            ("DASHSCOPE_MODEL", options.dashscope_model.as_str()),
        ]);
    }
    // The manager owns these model slots. Persist empty values too so a model
    // that disappeared from the live upstream catalog cannot survive from an
    // earlier install as a stale fallback.
    let media_values = if options.enable_media_generation {
        [
            options.media_generation_agent_id.as_str(),
            options.image2_model.as_str(),
            options.grok_image_model.as_str(),
            options.grok_video_model.as_str(),
            options.seedance_model.as_str(),
            options.minimax_video_model.as_str(),
        ]
    } else {
        ["", "", "", "", "", ""]
    };
    values.extend([
        ("MEDIA_GENERATION_AGENT_ID", media_values[0]),
        ("MEDIA_IMAGE2_MODEL", media_values[1]),
        ("MEDIA_GROK_IMAGE_MODEL", media_values[2]),
        ("MEDIA_GROK_VIDEO_MODEL", media_values[3]),
        ("MEDIA_SEEDANCE_MODEL", media_values[4]),
        ("MEDIA_MINIMAX_VIDEO_MODEL", media_values[5]),
    ]);
    persist_connection_values(profile, values)
}

fn feishu_environment_values(options: &CapabilityPackInstallOptions) -> [(&str, &str); 5] {
    [
        ("FEISHU_APP_ID", options.feishu_app_id.trim()),
        ("FEISHU_APP_SECRET", options.feishu_app_secret.trim()),
        ("FEISHU_DOMAIN", options.feishu_domain.trim()),
        ("FEISHU_AUTH_MODE", options.feishu_auth_mode.trim()),
        ("FEISHU_USER_ACCESS_TOKEN", options.feishu_user_access_token.trim()),
    ]
}

fn persist_connection_values(profile: &Path, values: Vec<(&str, &str)>) -> Result<(), AppError> {
    let values = values
        .into_iter()
        .filter(|(name, value)| name.starts_with("MEDIA_") || non_empty(value).is_some())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let names = values.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let target_user_sid = crate::app::usb_guard::user_environment_registry_sid()?;
    // Persist ownership before writing values.  If a registry write is
    // interrupted after a partial update, the later purge still has a safe,
    // exact allowlist to remove; the record never stores the secret values.
    persist_connection_ownership(profile, &names)?;
    update_user_environment(&values, target_user_sid.as_deref())
}

fn plan_feishu_config(
    current: &str,
    home: &Path,
    options: &CapabilityPackInstallOptions,
) -> Result<String, AppError> {
    use toml_edit::{value, Document, Item, Table};

    let invalid_config = || AppError::Engine("无法读取本机飞书 MCP 配置，原设置未更改".into());
    let mut config = current.parse::<Document>().map_err(|_| invalid_config())?;
    if !options.enable_feishu {
        if let Some(server) = config
            .get_mut("mcp_servers")
            .and_then(Item::as_table_like_mut)
            .and_then(|servers| servers.get_mut("feishu"))
        {
            server
                .as_table_like_mut()
                .ok_or_else(invalid_config)?
                .insert("enabled", value(false));
        }
        return Ok(config.to_string());
    }

    // Render the same table and tool approval rules as the first-install script.
    let quote = |text: &str| serde_json::to_string(text).expect("serialize TOML string");
    let mut environment = vec!["FEISHU_APP_ID", "FEISHU_APP_SECRET"];
    let tenant = options.feishu_auth_mode == "tenant";
    if tenant {
        environment.push("CODEX_USB_FEISHU_SETUP_ENTRY");
    }
    if options.feishu_auth_mode == "user" {
        environment.push("FEISHU_USER_ACCESS_TOKEN");
    }
    let wrapper = home.join("codex-agent-kit/bin/start-feishu-mcp.ps1");
    let cache = home.join("codex-agent-kit/runtime/feishu");
    let mut fragment = include_str!(
        "../../../installer-pack/config-fragments/feishu-official-stdio.template.toml"
    )
    .to_owned();
    for (placeholder, replacement) in [
        ("__FEISHU_ENABLED__", "true".to_owned()),
        (
            "__FEISHU_WRAPPER__",
            quote(&powershell_provider_path(&wrapper).to_string_lossy()),
        ),
        (
            "__FEISHU_MCP_CACHE__",
            quote(&powershell_provider_path(&cache).to_string_lossy()),
        ),
        (
            "__FEISHU_DOMAIN__",
            quote(options.feishu_domain.trim().trim_end_matches('/')),
        ),
        ("__FEISHU_AUTH_MODE__", quote(&options.feishu_auth_mode)),
        (
            "__FEISHU_CONNECT_TOOL__",
            if tenant { "  \"connect\"," } else { "" }.to_owned(),
        ),
        (
            "__FEISHU_ENV_VARS__",
            serde_json::to_string(&environment).expect("serialize env names"),
        ),
    ] {
        fragment = fragment.replace(placeholder, &replacement);
    }
    let fragment = fragment
        .parse::<Document>()
        .map_err(|_| AppError::Internal("随包飞书 MCP 模板无法解析".into()))?;
    let defaults = fragment["mcp_servers"]["feishu"]
        .as_table()
        .expect("Feishu template table");
    let servers = config
        .as_table_mut()
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .ok_or_else(invalid_config)?;
    if let Some(server) = servers.get_mut("feishu") {
        let server = server.as_table_like_mut().ok_or_else(invalid_config)?;
        server.remove("url");
        for (key, item) in defaults.iter() {
            if matches!(key, "enabled" | "command" | "args" | "env_vars")
                || !server.contains_key(key)
            {
                server.insert(key, item.clone());
            }
        }
    } else {
        servers.insert("feishu", Item::Table(defaults.clone()));
    }
    let planned = config.to_string();
    planned
        .parse::<toml::Value>()
        .map_err(|_| invalid_config())?;
    Ok(planned)
}

#[derive(Deserialize)]
struct FeishuRuntimeReport {
    repaired: bool,
    feishu: serde_json::Value,
}

fn prepare_feishu_runtime(
    source: &Path,
    profile: &Path,
    home: &Path,
    options: &CapabilityPackInstallOptions,
) -> Result<FeishuRuntimeReport, AppError> {
    for path in [
        "bin/start-feishu-mcp.ps1",
        "bin/feishu-tenant-server.cjs",
        "manifest/feishu-tools.json",
    ] {
        if !home.join(KIT_HOME_RELATIVE).join(path).is_file() {
            return Err(AppError::Engine(
                "飞书组件文件缺失。请在组件修复中补齐后再保存".into(),
            ));
        }
    }
    let progress = CapabilityProgressFile::create()?;
    let mut command = Command::new("powershell.exe");
    command
        .current_dir(source)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(source.join("runtime/provision-runtime.ps1"))
        .arg("-PackRoot")
        .arg(source)
        .arg("-TargetUserProfile")
        .arg(profile)
        .arg("-TargetCodexHome")
        .arg(powershell_provider_path(home))
        .arg("-FeishuOnly");
    let result = run_capability_installer(
        &mut command,
        progress.path(),
        CAPABILITY_INSTALL_TIMEOUT,
        |_| {},
    )
    .map_err(|_| AppError::Engine("飞书运行时准备进程未完成，请重试".into()))?;
    if !result.status.success() {
        log::warn!(
            "Feishu runtime preparation failed: {}",
            sanitized_child_output(&result, options)
        );
        return Err(AppError::Engine(
            "无法从安装包准备飞书运行时。请查看管理器日志".into(),
        ));
    }
    serde_json::from_slice(strip_utf8_bom(&result.stdout))
        .map_err(|_| AppError::Engine("飞书运行时准备未返回有效结果".into()))
}

fn record_feishu_runtime(home: &Path, feishu: serde_json::Value) {
    let path = readiness_path(home);
    let Ok(bytes) = fs::read(&path) else { return };
    let Ok(mut record) = serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) else {
        return;
    };
    let Some(runtime) = record
        .get_mut("runtime")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if runtime.get("feishu") == Some(&feishu) {
        return;
    }
    runtime.insert("feishu".into(), feishu);
    if let Err(error) = crate::app::atomic_file::write_atomic(&path, record.to_string().as_bytes())
    {
        log::warn!("Could not update Feishu runtime readiness: {error}");
    }
}

fn save_feishu_connection(
    source: &Path,
    version: &str,
    profile: &Path,
    home: &Path,
    options: &CapabilityPackInstallOptions,
) -> Result<CapabilityPackReport, AppError> {
    validate_feishu_options(options)?;
    let runtime = options
        .enable_feishu
        .then(|| prepare_feishu_runtime(source, profile, home, options))
        .transpose()?;
    let _config_lock = crate::app::model_selection_client::lock_configuration()
        .map_err(|_| AppError::Busy("另一个 Codex 配置操作正在进行，请稍后再保存飞书".into()))?;
    let config_path = home.join("config.toml");
    let current_bytes = crate::app::usb_host::read_optional(&config_path)?.unwrap_or_default();
    let current = std::str::from_utf8(strip_utf8_bom(&current_bytes))
        .map_err(|_| AppError::Engine("无法读取本机 Codex 配置，原设置未更改".into()))?;
    let config = current
        .parse::<toml::Value>()
        .map_err(|_| AppError::Engine("无法解析本机 Codex 配置，原设置未更改".into()))?;
    let same_registration = feishu_can_save_live(&config, home, options);
    if !same_registration {
        let planned = plan_feishu_config(current, home, options)?;
        if planned != current {
            crate::app::atomic_file::write_atomic_preserving_backup(
                &config_path,
                planned.as_bytes(),
            )
            .map_err(|_| {
                AppError::Engine("无法写入飞书 MCP 配置，请检查本机配置文件是否可写".into())
            })?;
        }
    }
    let saved_live = if let Some(runtime) = runtime {
        persist_connection_values(home, feishu_environment_values(options).to_vec())?;
        record_feishu_runtime(home, runtime.feishu);
        same_registration && !runtime.repaired
    } else {
        false
    };
    let mut report = pack_status_at(source, version, home, false);
    report.feishu_saved_live = saved_live;
    if options.enable_feishu {
        report.feishu_configured = true;
        report.feishu_runtime_status = "passed".into();
    }
    Ok(report)
}

pub fn install(
    app: &AppHandle,
    mut options: CapabilityPackInstallOptions,
) -> Result<CapabilityPackReport, AppError> {
    if !cfg!(target_os = "windows") {
        return Ok(report_missing("当前平台不支持 Windows 能力包安装"));
    }
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let resource_dir = app.path().resource_dir().ok();
    let Some(source) = find_pack(&executable, resource_dir.as_deref()) else {
        return Ok(report_missing(
            "未找到 content\\codex-starter-kit；请确认 U 盘发行包包含完整能力包",
        ));
    };
    let (pack_version, install_script) = validate_pack(&source)?;
    log::info!(
        "capability pack install requested source={} version={} profile={}",
        source.display(),
        pack_version,
        user_profile()?.display()
    );
    let profile = user_profile()?;
    let usb_codex_home = if crate::app::usb_guard::guard_required() {
        Some(crate::app::usb_guard::local_data_paths()?.codex_home)
    } else {
        None
    };
    let codex_home = usb_codex_home
        .clone()
        .unwrap_or_else(|| profile.join(".codex"));
    match options.feishu_action {
        Some(FeishuConnectionAction::Apply) => {
            return save_feishu_connection(&source, &pack_version, &profile, &codex_home, &options);
        }
        Some(FeishuConnectionAction::Preserve) => {
            options = preserve_feishu_selection(options, host_install_options(&codex_home)?);
        }
        None => {}
    }
    validate_options(&options)?;
    if options.enable_media_generation {
        migrate_legacy_media_environment(&codex_home)?;
    }
    let mut command = Command::new("powershell.exe");
    command
        .current_dir(&source)
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&install_script)
        .args([
            "-TargetUserProfile",
            profile.to_string_lossy().as_ref(),
            "-FeishuAuthMode",
            non_empty(&options.feishu_auth_mode).unwrap_or("tenant"),
            "-FeishuDomain",
            non_empty(&options.feishu_domain).unwrap_or("https://open.feishu.cn"),
        ]);
    if let Some(usb_codex_home) = usb_codex_home.as_ref() {
        command.arg("-TargetCodexHome").arg(powershell_provider_path(usb_codex_home));
        command.arg("-ReplaceExisting");
        let existing = crate::app::usb_host::read_config(usb_codex_home)?;
        let profile = crate::app::codex_config_readback::selected_profile(&existing)
            .map_err(|_| AppError::Engine("当前 Codex profile 无效，未修改权限设置".into()))?;
        if ["approval_policy", "sandbox_mode"].iter().all(|key|
            crate::app::codex_config_readback::effective_string(&existing, profile, key).is_none()) {
            command.arg("-EnableFullAccess");
        }
    }
    if options.enable_feishu {
        command.arg("-EnableFeishu");
    } else {
        command.arg("-DisableFeishu");
    }
    if !options.enabled_api_services.is_empty() {
        command.arg("-EnabledApiServices");
        for service in &options.enabled_api_services {
            command.arg(service);
        }
    }
    if !options.enabled_remote_mcps.is_empty() {
        command.arg("-EnabledRemoteMcps");
        for server in &options.enabled_remote_mcps {
            command.arg(server);
        }
    }
    command.arg(media_install_switch(options.enable_media_generation));
    if options.enable_media_generation {
        command
            .arg("-MediaGatewayCatalogStatus")
            .arg(&options.media_gateway_catalog_status)
            .arg("-MediaGatewayModelCount")
            .arg(options.media_gateway_model_count.to_string());
        if let Some(verified_at) = options
            .media_gateway_catalog_verified_at
            .as_deref()
            .and_then(non_empty)
        {
            command
                .arg("-MediaGatewayCatalogVerifiedAt")
                .arg(verified_at);
        }
    }
    add_env(&mut command, "FEISHU_APP_ID", &options.feishu_app_id);
    add_env(
        &mut command,
        "FEISHU_APP_SECRET",
        &options.feishu_app_secret,
    );
    add_env(
        &mut command,
        "FEISHU_USER_ACCESS_TOKEN",
        &options.feishu_user_access_token,
    );
    add_env(&mut command, "FEISHU_DOMAIN", &options.feishu_domain);
    add_env(&mut command, "FEISHU_AUTH_MODE", &options.feishu_auth_mode);
    add_env(
        &mut command,
        "DASHSCOPE_API_KEY",
        &options.dashscope_api_key,
    );
    add_env(
        &mut command,
        "DASHSCOPE_BASE_URL",
        &options.dashscope_base_url,
    );
    add_env(&mut command, "DASHSCOPE_MODEL", &options.dashscope_model);
    // These settings are a complete manager-owned snapshot. Explicit empty
    // values prevent an inherited route from an older install being reused.
    command
        .env(
            "MEDIA_GENERATION_API_PROFILE",
            &options.media_generation_api_profile,
        )
        .env(
            "MEDIA_GENERATION_AGENT_ID",
            &options.media_generation_agent_id,
        )
        .env("MEDIA_IMAGE2_MODEL", &options.image2_model)
        .env("MEDIA_GROK_IMAGE_MODEL", &options.grok_image_model)
        .env("MEDIA_GROK_VIDEO_MODEL", &options.grok_video_model)
        .env("MEDIA_SEEDANCE_MODEL", &options.seedance_model)
        .env("MEDIA_MINIMAX_VIDEO_MODEL", &options.minimax_video_model)
        // Credentials and endpoint are loaded only from the protected local
        // connection tuple after the installer exits.
        .env_remove("MEDIA_GENERATION_API_BASE_URL")
        .env_remove("MEDIA_GENERATION_API_KEY");
    let progress_file = CapabilityProgressFile::create()?;
    command.arg("-ProgressFile").arg(progress_file.path());
    let output = match run_capability_installer(
        &mut command,
        progress_file.path(),
        CAPABILITY_INSTALL_TIMEOUT,
        |stage| {
            let _ = app.emit(CAPABILITY_INSTALL_PROGRESS_EVENT, stage.to_string());
        },
    ) {
        Ok(output) => output,
        Err(CapabilityProcessError::Spawn(error)) => {
            log::error!(
                "capability pack installer process could not start source={} error={error}",
                source.display()
            );
            return Err(AppError::Engine(format!("启动能力包安装器失败: {error}")));
        }
        Err(CapabilityProcessError::Containment(error)) => {
            log::error!(
                "capability pack installer process containment failed source={} error={error}",
                source.display()
            );
            return Err(AppError::Engine(format!(
                "无法安全管理能力包安装进程树: {error}"
            )));
        }
        Err(CapabilityProcessError::Wait(error)) => {
            log::error!(
                "capability pack installer process wait failed source={} error={error}",
                source.display()
            );
            return Err(AppError::Engine(format!("等待能力包安装器失败: {error}")));
        }
        Err(CapabilityProcessError::Timeout { stage, output }) => {
            let detail = sanitized_child_output(&output, &options);
            log::error!(
                "capability pack install timed out source={} version={} timeout_secs={} stage={} detail={}",
                source.display(),
                pack_version,
                CAPABILITY_INSTALL_TIMEOUT.as_secs(),
                stage,
                if detail.is_empty() { "<no-output>" } else { &detail }
            );
            return Err(AppError::Engine(format!(
                "能力包安装在“{}”阶段超过 {} 分钟，已停止本次能力包安装；Codex 程序仍已安装，可在组件修复中重试",
                capability_stage_label(&stage),
                CAPABILITY_INSTALL_TIMEOUT.as_secs() / 60
            )));
        }
    };
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let detail = sanitized_child_output(&output, &options);
        log::error!(
            "capability pack install failed source={} version={} exit_code={} detail={}",
            source.display(),
            pack_version,
            code,
            if detail.is_empty() {
                "<no-output>"
            } else {
                &detail
            }
        );
        return Err(AppError::Engine(format!(
            "能力包安装失败（PowerShell 退出码 {code}）{}；请打开组件修复入口重试",
            if detail.is_empty() {
                String::new()
            } else {
                format!("：{detail}")
            }
        )));
    }
    log::info!(
        "capability pack installer exited successfully source={} version={}",
        source.display(),
        pack_version
    );
    let authorization_result = (|| {
        persist_configured_environment(&options, &codex_home)?;
        if options.enable_media_generation {
            crate::app::media_generation::sync_authorized_local_connection().map_err(|error| {
                log::error!("media MCP credential persistence failed: {error}");
                AppError::Engine(format!("媒体 MCP 授权同步失败：{error}"))
            })?;
            crate::app::media_generation::verify_authorized_local_connection().map_err(
                |error| {
                    log::error!("media MCP credential verification failed: {error}");
                    AppError::Engine(format!("媒体 MCP 授权自检失败：{error}"))
                },
            )?;
        }
        Ok::<(), AppError>(())
    })();
    record_media_authorization(&codex_home, options.enable_media_generation, authorization_result)?;
    let readiness = readiness_summary(&codex_home);
    let installed = readiness.pack_identity_valid
        && readiness.core_ready
        && readiness.pack_version.as_deref() == Some(pack_version.as_str());
    if !installed {
        let readiness_file = readiness_path(&codex_home);
        log::error!(
            "capability pack readiness failed source={} version={} skills={} agents={} runtime={} config={} readiness={}",
            source.display(),
            pack_version,
            readiness.skills,
            readiness.agents,
            readiness.runtime_status,
            readiness.config_status,
            readiness_file.display()
        );
        return Err(AppError::Engine(
            format!(
                "能力包核心文件已返回，但 readiness 校验未通过（Skills：{}，Agents：{}，路径：{}，运行时：{}，配置：{}）；请打开组件修复入口重试",
                readiness.skills,
                readiness.agents,
                readiness_file.display(),
                readiness.runtime_status,
                readiness.config_status
            ),
        ));
    }
    if options.enable_media_generation && !media_install_ready(&readiness, &codex_home) {
        log::error!(
            "media MCP readiness failed source={} version={} runtime={} protocol={} registered={} kit_home={}",
            source.display(),
            pack_version,
            readiness.media_runtime_status,
            readiness.media_protocol_status,
            readiness.media_registered,
            codex_home.join(KIT_HOME_RELATIVE).display()
        );
        return Err(AppError::Engine(format!(
            "核心能力包已安装，但统一媒体 MCP 未就绪：部署文件、Codex 注册或协议自检未通过；请查看 {} 并从组件修复入口重试",
            readiness
                .diagnostics_path
                .as_deref()
                .unwrap_or(".codex/codex-agent-kit/diagnostics/capability-pack.log")
        )));
    }
    let runtime_ready = readiness.runtime_status == "ready";
    let effective_config_status =
        if options.enable_media_generation && readiness.media_authorization_status != "verified" {
            "partial"
        } else {
            readiness.config_status.as_str()
        };
    let status = overall_status(
        installed,
        &readiness.runtime_status,
        effective_config_status,
    );
    let enabled_api_services = options.enabled_api_services.clone();
    Ok(CapabilityPackReport {
        available: true,
        installed: true,
        success: status == "ready",
        core_ready: true,
        overall_status: status.to_string(),
        source_path: Some(source.to_string_lossy().into_owned()),
        pack_version: Some(pack_version),
        skills: readiness.skills,
        agents: readiness.agents,
        runtime_ready,
        runtime_status: readiness.runtime_status.clone(),
        config_status: readiness.config_status.clone(),
        diagnostics_path: readiness.diagnostics_path,
        feishu_enabled: feishu_config_enabled(&codex_home).unwrap_or(false),
        feishu_configured: feishu_configured(),
        feishu_saved_live: false,
        feishu_runtime_status: readiness.feishu_runtime_status,
        enabled_api_services,
        media_runtime_status: readiness.media_runtime_status.clone(),
        media_protocol_status: readiness.media_protocol_status.clone(),
        media_gateway_catalog_status: readiness.media_gateway_catalog_status.clone(),
        media_gateway_model_count: readiness.media_gateway_model_count,
        media_gateway_catalog_verified_at: readiness.media_gateway_catalog_verified_at.clone(),
        media_generation_capability: readiness.media_generation_capability.clone(),
        codex_restart_required: readiness.codex_restart_required,
        codex_running: readiness.codex_running,
        message: if status == "ready" {
            format!(
                "能力包安装完成：{} 个 Skills、{} 个 Agents，运行时已就绪",
                readiness.skills, readiness.agents
            )
        } else {
            format!(
                "核心能力已安装：{} 个 Skills、{} 个 Agents；运行时状态为 {}，配置状态为 {}，可继续使用核心能力并从组件修复入口重试",
                readiness.skills, readiness.agents, readiness.runtime_status, readiness.config_status
            )
        },
    })
}
