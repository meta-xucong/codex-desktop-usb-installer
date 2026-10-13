//! Runtime credential handoff for the USB-bundled media-generation MCP.
//!
//! The device API key already lives in the signed USB license. This module
//! derives the trusted 404token endpoint from the bundled CCSwitch provider
//! record and places the credential in the USB Codex launch environment. The
//! MCP's env_vars allowlist forwards it to the local media server at runtime;
//! it is not persisted in HKCU or Codex config.

use crate::app::model_diagnostics::safe_identifier;
use crate::app::model_probe::{ProbeClient, ProbeTimeouts};
use crate::app::usb_guard;
use crate::errors::AppError;
use codex_usb_license::UsbAccountBinding;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};

const DEVICE_PROVIDER_ID: &str = "404token-sub2api";
const MEDIA_API_BASE_URL: &str = "MEDIA_GENERATION_API_BASE_URL";
const MEDIA_API_PROFILE: &str = "MEDIA_GENERATION_API_PROFILE";
const MEDIA_API_KEY: &str = "MEDIA_GENERATION_API_KEY";
const MEDIA_SIGNED_LAUNCH: &str = "MEDIA_GENERATION_SIGNED_LAUNCH";
const CREDENTIAL_FILE: &str = "media-generation.credential.dpapi";
const CONNECTION_FILE: &str = "media-generation.connection.json";
const CREDENTIAL_MAGIC: &[u8; 8] = b"CDXMGDP1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalConnection {
    schema_version: u32,
    api_base_url: String,
    api_profile: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaModelCatalog {
    pub models: Vec<String>,
    /// Existing local Seedance route settings are returned only so unattended
    /// installation can preserve a still-listed user-configured route.
    pub configured_seedance_model: Option<String>,
    pub configured_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalMediaModelSettings {
    pub licensed_credential_available: bool,
    pub media_generation_agent_id: String,
    pub image2_model: String,
    pub grok_image_model: String,
    pub grok_video_model: String,
    pub seedance_model: String,
    pub minimax_video_model: String,
}

/// Read only the non-secret model routing values saved for this Windows user.
pub fn local_model_settings() -> LocalMediaModelSettings {
    let values = current_user_model_settings()
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    let setting = |name: &str| {
        values
            .get(name)
            .filter(|value| safe_identifier(value))
            .cloned()
            .unwrap_or_default()
    };
    LocalMediaModelSettings {
        licensed_credential_available: usb_guard::authorized_account_binding()
            .ok()
            .flatten()
            .is_some_and(|binding| {
                binding.ccswitch.provider_id == DEVICE_PROVIDER_ID
                    && !binding.ccswitch.device_api_key.trim().is_empty()
            }),
        media_generation_agent_id: setting("MEDIA_GENERATION_AGENT_ID"),
        image2_model: setting("MEDIA_IMAGE2_MODEL"),
        grok_image_model: setting("MEDIA_GROK_IMAGE_MODEL"),
        grok_video_model: setting("MEDIA_GROK_VIDEO_MODEL"),
        seedance_model: setting("MEDIA_SEEDANCE_MODEL"),
        minimax_video_model: setting("MEDIA_MINIMAX_VIDEO_MODEL"),
    }
}

/// Read the signed USB account's upstream model directory. This makes one
/// authenticated GET /v1/models request and never invokes a generation API.
pub async fn catalog() -> Result<MediaModelCatalog, AppError> {
    let binding = usb_guard::authorized_account_binding()?
        .ok_or_else(|| AppError::Engine("未取得有效的签名 U 盘账号绑定".into()))?;
    if binding.ccswitch.provider_id != DEVICE_PROVIDER_ID {
        return Err(AppError::Engine(
            "当前 U 盘绑定不是 404token；未请求模型目录".into(),
        ));
    }
    let api_key = binding.ccswitch.device_api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::Engine("签名 U 盘绑定缺少设备凭证".into()));
    }
    let base = crate::app::model_catalog::provider_base(DEVICE_PROVIDER_ID)?;
    let client = ProbeClient::with_timeouts(
        &base,
        api_key,
        Arc::new(AtomicBool::new(false)),
        Instant::now() + Duration::from_secs(25),
        ProbeTimeouts::new(Duration::from_secs(20), Duration::from_secs(20)),
    )
    .map_err(|_| AppError::Engine("无法初始化安全的媒体模型目录客户端".into()))?;
    let payload = client.models().await.map_err(|error| {
        AppError::Engine(format!("404token 模型目录请求失败：{}", error.code()))
    })?;
    let rows = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 512)
        .ok_or_else(|| AppError::Engine("404token 模型目录格式无效".into()))?;
    let mut models = rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .or_else(|| row.get("slug"))
                .and_then(Value::as_str)?
                .trim();
            (safe_identifier(id) && !id.contains(api_key)).then(|| id.to_owned())
        })
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    let local = local_model_settings();
    let configured_seedance_model = (!local.seedance_model.is_empty()
        && models.iter().any(|model| model == &local.seedance_model))
    .then_some(local.seedance_model);
    let configured_agent_id = (!local.media_generation_agent_id.is_empty())
        .then_some(local.media_generation_agent_id);
    Ok(MediaModelCatalog {
        models,
        configured_seedance_model,
        configured_agent_id,
    })
}

/// Build a child-only environment contract. Empty credential/base values are
/// intentional when the signed binding is missing: they prevent stale values
/// inherited from a previous installer version from activating the MCP.
pub fn launch_environment(binding: Option<&UsbAccountBinding>) -> Vec<(String, String)> {
    let mut base_url = String::new();
    let mut api_key = String::new();

    if let Some(binding) = binding.filter(|binding| {
        binding.ccswitch.provider_id == DEVICE_PROVIDER_ID
            && !binding.ccswitch.device_api_key.trim().is_empty()
    }) {
        if let Ok(provider_base) = crate::app::model_catalog::provider_base(DEVICE_PROVIDER_ID) {
            let normalized = provider_base.trim_end_matches('/');
            if let Some(root) = normalized.strip_suffix("/v1") {
                base_url = root.to_string();
                api_key = binding.ccswitch.device_api_key.trim().to_string();
            }
        }
    }

    let signed_launch = if api_key.is_empty() { "" } else { "1" };
    vec![
        (MEDIA_API_BASE_URL.to_string(), base_url),
        (MEDIA_API_PROFILE.to_string(), "responses".to_string()),
        (MEDIA_API_KEY.to_string(), api_key),
        (MEDIA_SIGNED_LAUNCH.to_string(), signed_launch.to_string()),
    ]
}

pub fn usb_launch_environment() -> Vec<(String, String)> {
    let mut values = match crate::app::usb_guard::authorized_account_binding() {
        Ok(binding) => {
            if let Err(error) = sync_local_connection(binding.as_ref()) {
                log::error!("media MCP local credential sync failed: {error}");
                if let Err(clear_error) = clear_local_connection() {
                    log::error!("could not clear stale media MCP credential: {clear_error}");
                }
            }
            launch_environment(binding.as_ref())
        }
        Err(error) => {
            log::warn!(
                "media MCP credential handoff unavailable; MCP will remain unconfigured: {error}"
            );
            if let Err(clear_error) = clear_local_connection() {
                log::error!("could not clear stale media MCP credential: {clear_error}");
            }
            launch_environment(None)
        }
    };
    values.extend(current_user_model_settings());
    values
}

/// Refresh the user-scoped, DPAPI-protected credential from the currently
/// authorized USB license. The MCP wrapper can then start independently of
/// the Codex process environment; the plaintext key is never written to a
/// configuration file or command line.
pub fn sync_authorized_local_connection() -> Result<(), AppError> {
    if !usb_guard::guard_required() {
        return Ok(());
    }
    let binding = usb_guard::authorized_account_binding()?;
    let Some(binding) = binding.as_ref().filter(|binding| {
        binding.ccswitch.provider_id == DEVICE_PROVIDER_ID
            && !binding.ccswitch.device_api_key.trim().is_empty()
    }) else {
        return Err(AppError::Engine(
            "没有有效的 404token 签名 U 盘凭据，媒体 MCP 无法启用".into(),
        ));
    };
    sync_local_connection(Some(binding))?;
    let (credential_path, connection_path) = local_connection_paths()?;
    if !credential_path.is_file() || !connection_path.is_file() {
        return Err(AppError::Engine(
            "媒体 MCP 授权文件未完整写入本机".into(),
        ));
    }
    Ok(())
}

/// Verify the same DPAPI path the Codex MCP wrapper uses, without exposing the
/// decrypted API key in process arguments, output, or diagnostics.
pub fn verify_authorized_local_connection() -> Result<(), AppError> {
    if !usb_guard::guard_required() {
        return Ok(());
    }
    let (credential_path, connection_path) = local_connection_paths()?;
    if !credential_path.is_file() || !connection_path.is_file() {
        return Err(AppError::Engine(
            "媒体 MCP 授权文件未完整写入本机".into(),
        ));
    }
    let wrapper = usb_guard::local_data_paths()?
        .codex_home
        .join("codex-agent-kit")
        .join("bin")
        .join("start-media-generation-mcp.ps1");
    if !wrapper.is_file() {
        return Err(AppError::Engine(
            "媒体 MCP 本机启动脚本不存在，无法验证授权".into(),
        ));
    }
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(wrapper)
        .arg("-CredentialCheck")
        .env_remove(MEDIA_API_BASE_URL)
        .env_remove(MEDIA_API_PROFILE)
        .env_remove(MEDIA_API_KEY)
        .env_remove(MEDIA_SIGNED_LAUNCH);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);
    let output = command
        .output()
        .map_err(|_| AppError::Engine("无法启动媒体 MCP 授权自检".into()))?;
    if !output.status.success() {
        return Err(AppError::Engine("媒体 MCP DPAPI 凭据自检失败".into()));
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| AppError::Engine("媒体 MCP 授权自检返回格式无效".into()))?;
    if result.get("credentialSource").and_then(Value::as_str) != Some("dpapi-current-user")
        || result.get("apiKeyConfigured").and_then(Value::as_bool) != Some(true)
        || result.get("baseUrlConfigured").and_then(Value::as_bool) != Some(true)
        || result
            .get("configuredModelCount")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
    {
        return Err(AppError::Engine(
            "媒体 MCP DPAPI 凭据、网关地址或模型配置未就绪".into(),
        ));
    }
    Ok(())
}

fn local_connection_paths() -> Result<(PathBuf, PathBuf), AppError> {
    let codex_home = usb_guard::local_data_paths()?.codex_home;
    let connections = codex_home.join("codex-agent-kit").join("connections");
    Ok((
        connections.join(CREDENTIAL_FILE),
        connections.join(CONNECTION_FILE),
    ))
}

fn sync_local_connection(binding: Option<&UsbAccountBinding>) -> Result<(), AppError> {
    let (credential_path, connection_path) = local_connection_paths()?;
    let Some(binding) = binding.filter(|binding| {
        binding.ccswitch.provider_id == DEVICE_PROVIDER_ID
            && !binding.ccswitch.device_api_key.trim().is_empty()
    }) else {
        remove_if_present(&credential_path)?;
        remove_if_present(&connection_path)?;
        return Ok(());
    };

    let provider_base = crate::app::model_catalog::provider_base(DEVICE_PROVIDER_ID)?;
    let normalized = provider_base.trim_end_matches('/');
    let api_base_url = normalized
        .strip_suffix("/v1")
        .ok_or_else(|| AppError::Engine("404token 地址不是预期的 /v1 provider 地址".into()))?;
    let encrypted = protect_for_current_user(binding.ccswitch.device_api_key.trim().as_bytes())?;
    let mut credential = Vec::with_capacity(CREDENTIAL_MAGIC.len() + encrypted.len());
    credential.extend_from_slice(CREDENTIAL_MAGIC);
    credential.extend_from_slice(&encrypted);
    let connection = LocalConnection {
        schema_version: 1,
        api_base_url: api_base_url.to_string(),
        api_profile: "responses".to_string(),
    };
    let connection_bytes = serde_json::to_vec(&connection)
        .map_err(|_| AppError::Engine("媒体 MCP 连接配置编码失败".into()))?;
    let parent = credential_path
        .parent()
        .ok_or_else(|| AppError::Engine("媒体 MCP 凭据目录无效".into()))?;
    fs::create_dir_all(parent).map_err(|_| AppError::Engine("无法创建媒体 MCP 凭据目录".into()))?;
    crate::app::atomic_file::write_atomic(&credential_path, &credential)
        .map_err(|_| AppError::Engine("无法安全保存媒体 MCP 凭据".into()))?;
    crate::app::atomic_file::write_atomic(&connection_path, &connection_bytes)
        .map_err(|_| AppError::Engine("无法保存媒体 MCP 连接配置".into()))?;
    Ok(())
}

fn clear_local_connection() -> Result<(), AppError> {
    let (credential_path, connection_path) = local_connection_paths()?;
    remove_if_present(&credential_path)?;
    remove_if_present(&connection_path)
}

fn remove_if_present(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AppError::Engine("无法清除旧的媒体 MCP 本地凭据".into())),
    }
}

#[cfg(target_os = "windows")]
fn protect_for_current_user(plain: &[u8]) -> Result<Vec<u8>, AppError> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut input = CRYPT_INTEGER_BLOB {
        cbData: plain
            .len()
            .try_into()
            .map_err(|_| AppError::Engine("媒体 MCP 凭据长度无效".into()))?,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &mut input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() || output.cbData == 0 {
        return Err(AppError::Engine(
            "Windows DPAPI 无法保护媒体 MCP 凭据".into(),
        ));
    }
    let encrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(encrypted)
}

#[cfg(not(target_os = "windows"))]
fn protect_for_current_user(_plain: &[u8]) -> Result<Vec<u8>, AppError> {
    Err(AppError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn current_user_model_settings() -> Vec<(String, String)> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_SZ,
    };

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut raw_key = std::ptr::null_mut();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut raw_key,
        )
    } != 0
    {
        return media_model_environment_names()
            .into_iter()
            .map(|name| (name.to_owned(), String::new()))
            .collect();
    }
    let key = Key(raw_key);
    media_model_environment_names()
        .into_iter()
        .map(|name| {
            let value_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut value_type = 0;
            let mut byte_len = 0;
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    value_name.as_ptr(),
                    std::ptr::null(),
                    &mut value_type,
                    std::ptr::null_mut(),
                    &mut byte_len,
                )
            };
            if status != 0 || value_type != REG_SZ || byte_len < 2 || byte_len % 2 != 0 {
                return (name.to_owned(), String::new());
            }
            let mut value = vec![0u16; byte_len as usize / 2];
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    value_name.as_ptr(),
                    std::ptr::null(),
                    &mut value_type,
                    value.as_mut_ptr().cast(),
                    &mut byte_len,
                )
            };
            if status != 0 || value_type != REG_SZ {
                return (name.to_owned(), String::new());
            }
            let end = value
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(value.len());
            (name.to_owned(), String::from_utf16_lossy(&value[..end]))
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn current_user_model_settings() -> Vec<(String, String)> {
    media_model_environment_names()
        .into_iter()
        .map(|name| (name.to_owned(), String::new()))
        .collect()
}

fn media_model_environment_names() -> [&'static str; 6] {
    [
        "MEDIA_IMAGE2_MODEL",
        "MEDIA_GROK_IMAGE_MODEL",
        "MEDIA_GROK_VIDEO_MODEL",
        "MEDIA_SEEDANCE_MODEL",
        "MEDIA_MINIMAX_VIDEO_MODEL",
        "MEDIA_GENERATION_AGENT_ID",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_usb_license::UsbCcswitchBinding;

    fn binding(provider_id: &str, device_api_key: &str) -> UsbAccountBinding {
        UsbAccountBinding {
            schema_version: 1,
            usb_asset_id: "USB-TEST".into(),
            account_ref: "acct-test".into(),
            account_name: "test".into(),
            login_email: "test@example.invalid".into(),
            sub2api_user_id: 1,
            ccswitch: UsbCcswitchBinding {
                provider_id: provider_id.into(),
                credential_id: "credential-test".into(),
                device_api_key: device_api_key.into(),
                group_id: 1,
            },
        }
    }

    fn values(binding: Option<&UsbAccountBinding>) -> std::collections::HashMap<String, String> {
        launch_environment(binding).into_iter().collect()
    }

    #[test]
    fn licensed_404token_key_is_added_to_runtime_launch_environment() {
        let binding = binding(DEVICE_PROVIDER_ID, "sk-test-license-key");
        let values = values(Some(&binding));
        assert_eq!(values[MEDIA_API_BASE_URL], "https://404token.xyz");
        assert_eq!(values[MEDIA_API_PROFILE], "responses");
        assert_eq!(values[MEDIA_API_KEY], "sk-test-license-key");
        assert_eq!(values[MEDIA_SIGNED_LAUNCH], "1");
        assert!(!values.contains_key("MEDIA_IMAGE2_MODEL"));
    }

    #[test]
    fn missing_or_other_provider_clears_stale_media_credentials() {
        for binding in [None, Some(&binding("other-provider", "sk-not-404token"))] {
            let values = values(binding);
            assert_eq!(values[MEDIA_API_BASE_URL], "");
            assert_eq!(values[MEDIA_API_KEY], "");
            assert_eq!(values[MEDIA_API_PROFILE], "responses");
            assert_eq!(values[MEDIA_SIGNED_LAUNCH], "");
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_credential_blob_does_not_contain_the_plaintext_key() {
        let secret = b"synthetic-media-test-key";
        let protected = protect_for_current_user(secret).expect("DPAPI protects test credential");
        assert!(!protected.is_empty());
        assert!(!protected
            .windows(secret.len())
            .any(|window| window == secret));
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: protected.len() as u32,
            pbData: protected.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                null_mut(),
                null(),
                null_mut(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        assert_ne!(ok, 0, "Windows must decrypt the local credential blob");
        let unprotected =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
        assert_eq!(unprotected, secret);
        unsafe { LocalFree(output.pbData.cast()) };
    }
}
