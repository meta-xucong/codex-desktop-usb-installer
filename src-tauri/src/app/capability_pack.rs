//! Offline Codex capability-pack installation for the USB manager.
//!
//! The pack is a separate, signed-by-build-input directory shipped beside the
//! USB manager.  This module deliberately invokes only the pack's local
//! PowerShell installer: it never downloads a skill, MCP package, Python
//! runtime, Node runtime, or credential from the network.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::errors::AppError;

const PACK_DIRECTORY: &str = "codex-starter-kit";
const PACK_ID: &str = "codex-starter";
const READINESS_RELATIVE: &str = ".codex/codex-agent-kit/readiness.json";

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
    pub source_path: Option<String>,
    pub pack_version: Option<String>,
    pub skills: usize,
    pub agents: usize,
    pub runtime_ready: bool,
    pub feishu_enabled: bool,
    pub enabled_api_services: Vec<String>,
    pub message: String,
}

fn report_missing(message: impl Into<String>) -> CapabilityPackReport {
    CapabilityPackReport {
        available: false,
        installed: false,
        success: false,
        source_path: None,
        pack_version: None,
        skills: 0,
        agents: 0,
        runtime_ready: false,
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
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&pack_path)
            .map_err(|error| AppError::Engine(format!("读取能力包清单失败: {error}")))?,
    )
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

fn readiness_path() -> Result<PathBuf, AppError> {
    Ok(user_profile()?.join(READINESS_RELATIVE))
}

fn readiness_summary() -> (bool, usize, usize, bool) {
    let Ok(path) = readiness_path() else {
        return (false, 0, 0, false);
    };
    let Ok(bytes) = fs::read(path) else {
        return (false, 0, 0, false);
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (false, 0, 0, false);
    };
    let installed = value.pointer("/pack/id").and_then(|v| v.as_str()) == Some(PACK_ID);
    let skills = value
        .pointer("/installed/skills")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let agents = value
        .pointer("/installed/agents")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let runtime_ready = value.pointer("/runtime/status").and_then(|v| v.as_str()) == Some("ready");
    (installed, skills, agents, runtime_ready)
}

fn pack_version_from_readiness() -> Option<String> {
    let path = readiness_path().ok()?;
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    value
        .pointer("/pack/version")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
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
    let (readiness_installed, skills, agents, runtime_ready) = readiness_summary();
    let readiness_version = pack_version_from_readiness();
    let installed = readiness_installed && readiness_version.as_deref() == Some(version.as_str());
    Ok(CapabilityPackReport {
        available: true,
        installed,
        success: installed && runtime_ready,
        source_path: Some(source.to_string_lossy().into_owned()),
        pack_version: Some(readiness_version.unwrap_or(version)),
        skills,
        agents,
        runtime_ready,
        feishu_enabled: false,
        enabled_api_services: Vec::new(),
        message: if installed {
            "能力包已安装；外部连接状态按本机配置决定".to_string()
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
fn update_user_environment(values: &[(&str, &str)], removals: &[&str]) -> Result<(), AppError> {
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
    for name in removals {
        if !valid_name(name) {
            return Err(AppError::Engine(format!("拒绝删除非法环境变量名: {name}")));
        }
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let status = unsafe { RegDeleteValueW(handle, name_wide.as_ptr()) };
        if status != 0 && status != 2 {
            return Err(AppError::Engine(format!(
                "删除当前用户环境变量 {name} 失败（Win32 {status}）"
            )));
        }
        std::env::remove_var(name);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn update_user_environment(_values: &[(&str, &str)], _removals: &[&str]) -> Result<(), AppError> {
    Err(AppError::UnsupportedPlatform)
}

fn persist_configured_environment(options: &CapabilityPackInstallOptions) -> Result<(), AppError> {
    if !options.persist_connections {
        return Ok(());
    }
    let mut values = Vec::<(&str, &str)>::new();
    let mut removals = Vec::<&str>::new();
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
        if non_empty(&options.feishu_user_access_token).is_none() {
            removals.push("FEISHU_USER_ACCESS_TOKEN");
        }
    } else {
        removals.extend([
            "FEISHU_APP_ID",
            "FEISHU_APP_SECRET",
            "FEISHU_DOMAIN",
            "FEISHU_AUTH_MODE",
            "FEISHU_USER_ACCESS_TOKEN",
        ]);
    }
    if selected_service(options, "dashscope-web-search") {
        values.extend([
            ("DASHSCOPE_API_KEY", options.dashscope_api_key.as_str()),
            ("DASHSCOPE_BASE_URL", options.dashscope_base_url.as_str()),
            ("DASHSCOPE_MODEL", options.dashscope_model.as_str()),
        ]);
    } else {
        removals.extend(["DASHSCOPE_API_KEY", "DASHSCOPE_BASE_URL", "DASHSCOPE_MODEL"]);
    }
    if selected_service(options, "image-2") {
        values.extend([
            ("IMAGE_2_API_KEY", options.image2_api_key.as_str()),
            ("IMAGE_2_API_BASE_URL", options.image2_base_url.as_str()),
            ("IMAGE_2_MODEL", options.image2_model.as_str()),
            ("IMAGE_2_AGENT_ID", options.image2_agent_id.as_str()),
        ]);
    } else {
        removals.extend([
            "IMAGE_2_API_KEY",
            "IMAGE_2_API_BASE_URL",
            "IMAGE_2_MODEL",
            "IMAGE_2_AGENT_ID",
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
    } else {
        removals.extend([
            "SEEDANCE_API_KEY",
            "SEEDANCE_API_BASE_URL",
            "SEEDANCE_MODEL",
            "SEEDANCE_AGENT_ID",
            "SEEDANCE_PRECHARGE_POINTS",
            "SEEDANCE_OFFICIAL_LINK_TTL_HOURS",
            "SEEDANCE_REFUND_RULE",
        ]);
    }
    let values = values
        .into_iter()
        .filter(|(_, value)| non_empty(value).is_some())
        .collect::<Vec<_>>();
    update_user_environment(&values, &removals)
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
        return Err(AppError::Engine(format!(
            "能力包安装失败（PowerShell 退出码 {code}）；请打开组件修复入口重试"
        )));
    }
    persist_configured_environment(&options)?;
    let (installed, skills, agents, runtime_ready) = readiness_summary();
    if !installed || !runtime_ready {
        return Err(AppError::Engine(
            "能力包安装脚本已结束，但 readiness 校验未通过；请打开组件修复入口重试".to_string(),
        ));
    }
    Ok(CapabilityPackReport {
        available: true,
        installed: true,
        success: true,
        source_path: Some(source.to_string_lossy().into_owned()),
        pack_version: Some(pack_version),
        skills,
        agents,
        runtime_ready,
        feishu_enabled: options.enable_feishu,
        enabled_api_services: options.enabled_api_services,
        message: format!("能力包安装完成：{skills} 个 Skills、{agents} 个 Agents，运行时已就绪"),
    })
}
