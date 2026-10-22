//! Offline Codex capability-pack installation for the USB manager.
//!
//! The pack is a separate, signed-by-build-input directory shipped beside the
//! USB manager. This module invokes the pack's local PowerShell installer.
//! Skills, Python/Node base runtimes, Feishu MCP, FFmpeg, and Manim are bundled;
//! the HyperFrames CLI and its browser may need network access during setup.
//! A missing network-dependent component is reported as requires-network and
//! does not prevent the remaining capability pack from being installed.

use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct CapabilityPackInstallOptions {
    pub enable_feishu: bool,
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
            enable_feishu: false,
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
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| AppError::Engine("无法确定当前 Windows 用户目录".to_string()))
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
        "pack.json",
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
    let pack_path = root.join("pack.json");
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
    Ok((version, root.join("install-all.ps1")))
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
        media_install_switch, persist_connection_ownership, readiness_path, readiness_summary,
        strip_utf8_bom, validate_options, write_media_authorization_status,
        CapabilityPackInstallOptions, ConnectionOwnershipRecord, CONNECTION_OWNERSHIP_RELATIVE,
        KIT_HOME_RELATIVE, PACK_ID,
    };
    use serde_json::json;
    use std::fs;

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

fn read_media_authorization_status(profile: &Path) -> String {
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
        .arg(&codex_home);
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

/// Return the Product Design skill count only when the USB Codex Home has a
/// verified, enabled offline installation. This is read-only and is used by
/// Skill Hub status so the plugin remains visible after navigating away from
/// the one-time apply result.
pub fn product_design_plugin_status() -> Option<u32> {
    if !cfg!(target_os = "windows") || !crate::app::usb_guard::guard_required() {
        return None;
    }
    let codex_home = crate::app::usb_guard::local_data_paths().ok()?.codex_home;
    let config_text = fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let config: toml::Value = toml::from_str(&config_text).ok()?;
    let marketplace_name = "usb-product-design-offline";
    let plugin_name = "product-design";
    let plugin_id = format!("{plugin_name}@{marketplace_name}");
    if config
        .get("plugins")?
        .get(&plugin_id)?
        .get("enabled")?
        .as_bool()
        != Some(true)
    {
        return None;
    }
    let marketplace_root = codex_home
        .join("plugins")
        .join("marketplaces")
        .join(marketplace_name);
    let marketplace_source = config
        .get("marketplaces")?
        .get(marketplace_name)?
        .get("source")?
        .as_str()?;
    if PathBuf::from(marketplace_source) != marketplace_root {
        return None;
    }
    let cache_plugin = codex_home
        .join("plugins")
        .join("cache")
        .join(marketplace_name)
        .join(plugin_name);
    let manifest_path = cache_plugin.join(".codex-plugin").join("plugin.json");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(manifest_path).ok()?).ok()?;
    let version = manifest.get("version")?.as_str()?;
    if manifest.get("name")?.as_str()? != plugin_name || !cache_plugin.is_dir() {
        return None;
    }
    let skill_count = fs::read_dir(cache_plugin.join("skills"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir() && entry.path().join("SKILL.md").is_file())
        .count() as u32;
    if skill_count != 10 {
        return None;
    }
    let runtime_root = codex_home.join("product-design-runtime");
    let runtime_ready = fs::read_dir(runtime_root).ok()?.filter_map(Result::ok).any(|entry| {
        let marker = entry.path().join(".installed-manifest.json");
        let Ok(bytes) = fs::read(marker) else { return false };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false };
        value.get("pluginVersion").and_then(serde_json::Value::as_str) == Some(version)
            && value.get("platform").and_then(serde_json::Value::as_str) == Some("win32-x64")
    });
    runtime_ready.then_some(skill_count)
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
    let readiness = readiness_summary(&codex_home);
    let installed = readiness.pack_identity_valid
        && readiness.core_ready
        && readiness.pack_version.as_deref() == Some(version.as_str());
    let runtime_ready = installed && readiness.runtime_status == "ready";
    let media_needs_auth = readiness.media_enabled || readiness.media_registered;
    let media_auth_live = !readiness.media_enabled
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
    Ok(CapabilityPackReport {
        available: true,
        installed,
        success: status == "ready",
        core_ready: installed,
        overall_status: status.to_string(),
        source_path: Some(source.to_string_lossy().into_owned()),
        pack_version: Some(readiness.pack_version.unwrap_or(version)),
        skills: readiness.skills,
        agents: readiness.agents,
        runtime_ready,
        runtime_status: readiness.runtime_status.clone(),
        config_status: readiness.config_status.clone(),
        diagnostics_path: readiness.diagnostics_path,
        feishu_enabled: false,
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
                readiness.runtime_status, readiness.config_status
            )
        } else {
            "能力包已就绪，可安装或修复全部 Skills、Agents、运行时和本地 MCP 资源".to_string()
        },
    })
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
    if options.enable_feishu {
        if non_empty(&options.feishu_app_id).is_none()
            || non_empty(&options.feishu_app_secret).is_none()
        {
            return Err(AppError::Engine(
                "启用飞书 MCP 时必须填写 App ID 和 App Secret".to_string(),
            ));
        }
        if options.feishu_auth_mode.eq_ignore_ascii_case("user")
            && non_empty(&options.feishu_user_access_token).is_none()
        {
            return Err(AppError::Engine(
                "飞书 user 认证模式必须填写 User Access Token".to_string(),
            ));
        }
    }
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
fn update_user_environment(values: &[(&str, &str)]) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
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
            "写入当前用户环境变量失败（Win32 {status}）"
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
fn update_user_environment(_values: &[(&str, &str)]) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn remove_owned_user_environment(names: &[&str]) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut handle,
        )
    };
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
fn remove_owned_user_environment(_names: &[&str]) -> Result<(), AppError> {
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
    remove_owned_user_environment(&owned)?;
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
    if options.enable_feishu {
        values.extend([
            ("FEISHU_APP_ID", options.feishu_app_id.as_str()),
            ("FEISHU_APP_SECRET", options.feishu_app_secret.as_str()),
            ("FEISHU_DOMAIN", options.feishu_domain.as_str()),
            ("FEISHU_AUTH_MODE", options.feishu_auth_mode.as_str()),
            (
                "FEISHU_USER_ACCESS_TOKEN",
                options.feishu_user_access_token.as_str(),
            ),
        ]);
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
    let values = values
        .into_iter()
        .filter(|(name, value)| name.starts_with("MEDIA_") || non_empty(value).is_some())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let names = values.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    // Persist ownership before writing values.  If a registry write is
    // interrupted after a partial update, the later purge still has a safe,
    // exact allowlist to remove; the record never stores the secret values.
    persist_connection_ownership(profile, &names)?;
    update_user_environment(&values)
}

pub fn install(
    app: &AppHandle,
    options: CapabilityPackInstallOptions,
) -> Result<CapabilityPackReport, AppError> {
    if !cfg!(target_os = "windows") {
        return Ok(report_missing("当前平台不支持 Windows 能力包安装"));
    }
    validate_options(&options)?;
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
        command.arg("-TargetCodexHome").arg(usb_codex_home);
        command.arg("-EnableFullAccess");
    }
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

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

    let output = command.output().map_err(|error| {
        log::error!(
            "capability pack installer process could not start source={} error={error}",
            source.display()
        );
        AppError::Engine(format!("启动能力包安装器失败: {error}"))
    })?;
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
    let authorization_status = if !options.enable_media_generation {
        "not-required"
    } else if authorization_result.is_ok() {
        "verified"
    } else {
        "failed"
    };
    if let Err(error) = write_media_authorization_status(&codex_home, authorization_status) {
        log::error!("media MCP authorization readiness update failed: {error}");
        return Err(error);
    }
    authorization_result?;
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
            "统一媒体 MCP 未完成安装：部署文件、Codex 注册或协议自检未通过；请查看 {} 并从组件修复入口重试",
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
        feishu_enabled: options.enable_feishu,
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
