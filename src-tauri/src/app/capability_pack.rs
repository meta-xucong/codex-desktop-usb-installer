//! Offline Codex capability-pack installation for the USB manager.
//!
//! The pack is a separate, signed-by-build-input directory shipped beside the
//! USB manager.  This module deliberately invokes only the pack's local
//! PowerShell installer: it never downloads a skill, MCP package, Python
//! runtime, Node runtime, or credential from the network.

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
const READINESS_RELATIVE: &str = ".codex/codex-agent-kit/readiness.json";
const CONNECTION_OWNERSHIP_RELATIVE: &str = ".codex/codex-agent-kit/connection-ownership.json";
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
pub struct CapabilityPackInstallOptions {
    pub enable_feishu: bool,
    pub persist_connections: bool,
    pub feishu_app_id: String,
    pub feishu_app_secret: String,
    pub feishu_user_access_token: String,
    pub feishu_domain: String,
    pub feishu_auth_mode: String,
    pub enabled_api_services: Vec<String>,
    pub dashscope_api_key: String,
    pub dashscope_base_url: String,
    pub dashscope_model: String,
    pub image2_api_key: String,
    pub image2_base_url: String,
    pub image2_model: String,
    pub image2_agent_id: String,
    pub seedance_api_key: String,
    pub seedance_base_url: String,
    pub seedance_model: String,
    pub seedance_agent_id: String,
    pub seedance_precharge_points: String,
    pub seedance_official_link_ttl_hours: String,
    pub seedance_refund_rule: String,
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

fn readiness_summary(profile: &Path) -> ReadinessSummary {
    let path = readiness_path(profile);
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
        core_ready_from_readiness, persist_connection_ownership, readiness_summary, strip_utf8_bom,
        ConnectionOwnershipRecord, CONNECTION_OWNERSHIP_RELATIVE, PACK_ID,
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
        let readiness_path = profile.join(".codex/codex-agent-kit/readiness.json");
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

        let summary = readiness_summary(&profile);
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
        fs::create_dir_all(&profile).unwrap();
        persist_connection_ownership(&profile, &["FEISHU_APP_SECRET"]).unwrap();
        persist_connection_ownership(&profile, &["DASHSCOPE_API_KEY"]).unwrap();
        let path = profile.join(CONNECTION_OWNERSHIP_RELATIVE);
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
        &options.image2_api_key,
        &options.seedance_api_key,
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
    let readiness = readiness_summary(&profile);
    let installed = readiness.pack_identity_valid
        && readiness.core_ready
        && readiness.pack_version.as_deref() == Some(version.as_str());
    let runtime_ready = installed && readiness.runtime_status == "ready";
    let status = overall_status(
        installed,
        &readiness.runtime_status,
        &readiness.config_status,
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
    for service in &options.enabled_api_services {
        if !matches!(
            service.trim(),
            "dashscope-web-search" | "image-2" | "seedance"
        ) {
            return Err(AppError::Engine(format!(
                "不支持的能力包外部服务: {service}"
            )));
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
    if selected_service(options, "image-2")
        && (non_empty(&options.image2_api_key).is_none()
            || non_empty(&options.image2_base_url).is_none()
            || non_empty(&options.image2_model).is_none()
            || non_empty(&options.image2_agent_id).is_none())
    {
        return Err(AppError::Engine(
            "启用图像服务时必须填写 API Key、Base URL、模型和路由 ID".to_string(),
        ));
    }
    if selected_service(options, "seedance")
        && [
            &options.seedance_api_key,
            &options.seedance_base_url,
            &options.seedance_model,
            &options.seedance_agent_id,
            &options.seedance_precharge_points,
            &options.seedance_official_link_ttl_hours,
            &options.seedance_refund_rule,
        ]
        .iter()
        .any(|value| non_empty(value).is_none())
    {
        return Err(AppError::Engine(
            "启用视频服务时必须补全 API Key、端点、模型、路由和费用策略".to_string(),
        ));
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
    if selected_service(options, "image-2") {
        values.extend([
            ("IMAGE_2_API_KEY", options.image2_api_key.as_str()),
            ("IMAGE_2_API_BASE_URL", options.image2_base_url.as_str()),
            ("IMAGE_2_MODEL", options.image2_model.as_str()),
            ("IMAGE_2_AGENT_ID", options.image2_agent_id.as_str()),
        ]);
    }
    if selected_service(options, "seedance") {
        values.extend([
            ("SEEDANCE_API_KEY", options.seedance_api_key.as_str()),
            ("SEEDANCE_API_BASE_URL", options.seedance_base_url.as_str()),
            ("SEEDANCE_MODEL", options.seedance_model.as_str()),
            ("SEEDANCE_AGENT_ID", options.seedance_agent_id.as_str()),
            (
                "SEEDANCE_PRECHARGE_POINTS",
                options.seedance_precharge_points.as_str(),
            ),
            (
                "SEEDANCE_OFFICIAL_LINK_TTL_HOURS",
                options.seedance_official_link_ttl_hours.as_str(),
            ),
            (
                "SEEDANCE_REFUND_RULE",
                options.seedance_refund_rule.as_str(),
            ),
        ]);
    }
    let values = values
        .into_iter()
        .filter(|(_, value)| non_empty(value).is_some())
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
    let profile = user_profile()?;
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
    add_env(&mut command, "IMAGE_2_API_KEY", &options.image2_api_key);
    add_env(
        &mut command,
        "IMAGE_2_API_BASE_URL",
        &options.image2_base_url,
    );
    add_env(&mut command, "IMAGE_2_MODEL", &options.image2_model);
    add_env(&mut command, "IMAGE_2_AGENT_ID", &options.image2_agent_id);
    add_env(&mut command, "SEEDANCE_API_KEY", &options.seedance_api_key);
    add_env(
        &mut command,
        "SEEDANCE_API_BASE_URL",
        &options.seedance_base_url,
    );
    add_env(&mut command, "SEEDANCE_MODEL", &options.seedance_model);
    add_env(
        &mut command,
        "SEEDANCE_AGENT_ID",
        &options.seedance_agent_id,
    );
    add_env(
        &mut command,
        "SEEDANCE_PRECHARGE_POINTS",
        &options.seedance_precharge_points,
    );
    add_env(
        &mut command,
        "SEEDANCE_OFFICIAL_LINK_TTL_HOURS",
        &options.seedance_official_link_ttl_hours,
    );
    add_env(
        &mut command,
        "SEEDANCE_REFUND_RULE",
        &options.seedance_refund_rule,
    );

    let output = command
        .output()
        .map_err(|error| AppError::Engine(format!("启动能力包安装器失败: {error}")))?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let detail = sanitized_child_output(&output, &options);
        return Err(AppError::Engine(format!(
            "能力包安装失败（PowerShell 退出码 {code}）{}；请打开组件修复入口重试",
            if detail.is_empty() {
                String::new()
            } else {
                format!("：{detail}")
            }
        )));
    }
    persist_configured_environment(&options, &profile)?;
    let readiness = readiness_summary(&profile);
    let installed = readiness.pack_identity_valid
        && readiness.core_ready
        && readiness.pack_version.as_deref() == Some(pack_version.as_str());
    if !installed {
        let readiness_file = readiness_path(&profile);
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
    let runtime_ready = readiness.runtime_status == "ready";
    let status = overall_status(
        installed,
        &readiness.runtime_status,
        &readiness.config_status,
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
