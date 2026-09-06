#[cfg(windows)]
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::app::settings_store::AppSettings as PersistedAppSettings;
use crate::errors::AppError;

pub const DISABLE_ENV_KEY: &str = "CODEX_SPARKLE_ENABLED";
pub const DISABLE_ENV_VALUE: &str = "false";

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "io.github.wangnov.codex-app-manager.codex-self-update-env";

pub fn apply_to_command(command: &mut Command, disabled: bool) {
    if disabled {
        command.env(DISABLE_ENV_KEY, DISABLE_ENV_VALUE);
    }
}

pub fn sync_setting(disabled: bool) -> Result<(), AppError> {
    sync_current_process(disabled);
    sync_platform(disabled)
}

/// Apply the platform environment and durably record the same value.
///
/// Historical installs use this from both the live commit path and startup
/// transaction recovery. The platform sync is intentionally unconditional:
/// a previous process may have crashed after changing only one side, so the
/// settings file alone is not proof that launchctl/the Windows user environment
/// already matches it.
pub fn sync_and_persist_setting(disabled: bool) -> Result<(), AppError> {
    sync_setting(disabled)?;
    let mut settings = PersistedAppSettings::load();
    if settings.disable_codex_self_updates != disabled {
        settings.disable_codex_self_updates = disabled;
        settings.normalize();
        settings.save()?;
    }
    Ok(())
}

fn sync_current_process(disabled: bool) {
    if disabled {
        std::env::set_var(DISABLE_ENV_KEY, DISABLE_ENV_VALUE);
    } else {
        std::env::remove_var(DISABLE_ENV_KEY);
    }
}

#[cfg(target_os = "macos")]
fn sync_platform(disabled: bool) -> Result<(), AppError> {
    if disabled {
        launchctl_required(&["setenv", DISABLE_ENV_KEY, DISABLE_ENV_VALUE])?;
        install_macos_launch_agent()?;
    } else {
        launchctl_required(&["unsetenv", DISABLE_ENV_KEY])?;
        remove_macos_launch_agent()?;
    }
    Ok(())
}

#[cfg(windows)]
fn sync_platform(disabled: bool) -> Result<(), AppError> {
    let _guard = ENV_JOURNAL_LOCK
        .lock()
        .map_err(|_| AppError::Internal("自更新策略锁异常".into()))?;
    let requested = disabled.then(|| EnvRegistryValue::plain(DISABLE_ENV_VALUE));
    let current = read_windows_user_env()?;
    let existing = load_env_journal()?;
    // No mutation means no ownership claim over a pre-existing user value.
    if current != requested {
        let mut journal = existing.unwrap_or(EnvJournal {
            schema: 1,
            previous: current.clone(),
            written: requested.clone(),
        });
        if current != journal.written && current != journal.previous {
            journal.previous = current;
        }
        journal.written = requested.clone();
        save_env_journal(&journal)?; // durable intent precedes registry mutation
        write_windows_user_env(requested.as_ref())?;
    }
    broadcast_windows_environment_change();
    Ok(())
}

/// Restore only a recorded manager-owned environment transition. Old builds
/// have no prior-value snapshot, so a current value is retained and reported,
/// not guessed away. Call before deleting manager data/journals.
pub fn cleanup_for_uninstall() -> Result<Vec<String>, AppError> {
    #[cfg(windows)]
    {
        let _guard = ENV_JOURNAL_LOCK
            .lock()
            .map_err(|_| AppError::Internal("自更新策略锁异常".into()))?;
        let current = read_windows_user_env()?;
        let Some(journal) = load_env_journal()? else {
            return Ok(if current.is_some() {
                vec!["保留 CODEX_SPARKLE_ENABLED 用户环境变量：旧版没有安装前快照，无法证明该值由本安装器创建。".into()]
            } else {
                vec![]
            });
        };
        let restore = env_restore_action(&current, &journal.previous, &journal.written);
        let notes = match restore {
            EnvRestore::Restore => {
                write_windows_user_env(journal.previous.as_ref())?;
                broadcast_windows_environment_change();
                vec!["已按安装前快照恢复 CODEX_SPARKLE_ENABLED 用户环境变量。".into()]
            }
            EnvRestore::AlreadyOriginal => vec![],
            EnvRestore::PreserveExternal => {
                vec!["CODEX_SPARKLE_ENABLED 已被其他程序/用户修改，已保留外部现值。".into()]
            }
        };
        let path = env_journal_path()?;
        for item in [path.with_extension("json.bak"), path.clone()] {
            codex_win_engine::validate_cleanup_path(&item)
                .map_err(|e| AppError::Engine(e.to_string()))?;
            match std::fs::remove_file(&item) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::Engine(format!(
                        "环境变量已处理但快照清理失败：{error}"
                    )))
                }
            }
        }
        Ok(notes)
    }
    #[cfg(not(windows))]
    {
        Ok(vec![])
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg(any(windows, test))]
enum EnvRestore {
    Restore,
    AlreadyOriginal,
    PreserveExternal,
}

#[cfg(any(windows, test))]
fn env_restore_action<T: PartialEq>(
    current: &Option<T>,
    previous: &Option<T>,
    written: &Option<T>,
) -> EnvRestore {
    if current == previous {
        EnvRestore::AlreadyOriginal
    } else if current == written {
        EnvRestore::Restore
    } else {
        EnvRestore::PreserveExternal
    }
}

#[cfg(windows)]
static ENV_JOURNAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct EnvRegistryValue {
    kind: u32,
    bytes: Vec<u8>,
}

#[cfg(windows)]
impl EnvRegistryValue {
    fn plain(text: &str) -> Self {
        Self {
            kind: windows_sys::Win32::System::Registry::REG_SZ,
            bytes: text
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        }
    }
}

#[cfg(windows)]
#[derive(Serialize, Deserialize)]
struct EnvJournal {
    schema: u32,
    previous: Option<EnvRegistryValue>,
    written: Option<EnvRegistryValue>,
}

#[cfg(windows)]
fn env_journal_path() -> Result<std::path::PathBuf, AppError> {
    let path = crate::app::paths::data_dir()
        .ok_or_else(|| AppError::Engine("无法定位自更新环境变量快照".into()))?
        .join("codex-self-update-env.json");
    codex_win_engine::validate_cleanup_path(&path).map_err(|e| AppError::Engine(e.to_string()))?;
    Ok(path)
}

#[cfg(windows)]
fn load_env_journal() -> Result<Option<EnvJournal>, AppError> {
    let path = env_journal_path()?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path.with_extension("json.bak")) {
                Ok(_) => {
                    return Err(AppError::Engine(
                        "环境变量快照主文件缺失但备份仍在；保留快照，拒绝猜测原值".into(),
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::Engine(format!(
                        "无法检查环境变量备份快照：{error}"
                    )))
                }
            }
            return Ok(None);
        }
        Err(error) => return Err(AppError::Engine(format!("读取环境变量快照失败：{error}"))),
    };
    let journal: EnvJournal = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Engine(format!("环境变量快照无效：{e}")))?;
    if journal.schema != 1
        || journal
            .previous
            .iter()
            .chain(journal.written.iter())
            .any(|value| {
                !matches!(value.kind, 1 | 2)
                    || value.bytes.len() > 65536
                    || value.bytes.len() % 2 != 0
            })
    {
        return Err(AppError::Engine("环境变量快照版本/类型无效，已保留".into()));
    }
    Ok(Some(journal))
}

#[cfg(windows)]
fn save_env_journal(journal: &EnvJournal) -> Result<(), AppError> {
    let path = env_journal_path()?;
    codex_win_engine::validate_cleanup_path(&path.with_extension("json.bak"))
        .map_err(|e| AppError::Engine(e.to_string()))?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| AppError::Engine(e.to_string()))?;
    let bytes = serde_json::to_vec(journal).map_err(|e| AppError::Engine(e.to_string()))?;
    crate::app::atomic_file::write_atomic(&path, &bytes)
        .map_err(|e| AppError::Engine(format!("保存环境变量快照失败：{e}")))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn sync_platform(_disabled: bool) -> Result<(), AppError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_agent_path() -> Result<std::path::PathBuf, AppError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| AppError::Internal("无法定位用户主目录".to_string()))?;
    Ok(std::path::PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn launchctl_required(args: &[&str]) -> Result<(), AppError> {
    let output = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| AppError::Engine(format!("launchctl {}: {e}", args.join(" "))))?;
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::Engine(format!(
        "launchctl {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(target_os = "macos")]
fn launchctl_best_effort(args: &[String]) {
    if let Ok(output) = Command::new("/bin/launchctl").args(args).output() {
        if !output.status.success() {
            log::debug!(
                "launchctl best-effort command failed args={} stderr={}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn launchctl_gui_target() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn install_macos_launch_agent() -> Result<(), AppError> {
    let path = launch_agent_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Internal("无法定位 LaunchAgents 目录".to_string()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| AppError::Internal(format!("create LaunchAgents dir: {e}")))?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/launchctl</string>
    <string>setenv</string>
    <string>{DISABLE_ENV_KEY}</string>
    <string>{DISABLE_ENV_VALUE}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    );
    std::fs::write(&path, plist)
        .map_err(|e| AppError::Internal(format!("write LaunchAgent: {e}")))?;

    // The current GUI session is already updated by `launchctl setenv` before
    // this function runs. Leave the LaunchAgent on disk for the next login
    // instead of bootstrapping it immediately: immediate bootstrap/kickstart is
    // noticeably slow and makes macOS show a "launchctl can run in background"
    // notification for a one-shot helper.
    Ok(())
}

#[cfg(target_os = "macos")]
fn remove_macos_launch_agent() -> Result<(), AppError> {
    let path = launch_agent_path()?;
    if path.exists() {
        let target = launchctl_gui_target();
        launchctl_best_effort(&[
            "bootout".to_string(),
            target,
            path.to_string_lossy().to_string(),
        ]);
        std::fs::remove_file(&path)
            .map_err(|e| AppError::Internal(format!("remove LaunchAgent: {e}")))?;
    }
    Ok(())
}

#[cfg(windows)]
fn env_registry_error(operation: &str, status: u32) -> AppError {
    AppError::Engine(format!(
        "{operation} HKCU\\Environment\\{DISABLE_ENV_KEY} 失败（Win32 {status}）"
    ))
}

#[cfg(windows)]
struct EnvKey(windows_sys::Win32::System::Registry::HKEY);
#[cfg(windows)]
impl Drop for EnvKey {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(windows)]
fn open_env_key(access: u32, create: bool) -> Result<Option<EnvKey>, AppError> {
    use windows_sys::Win32::System::Registry::*;
    let name: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    let mut status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut handle) };
    if status == 2 && create {
        status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
    }
    if status == 2 {
        return Ok(None);
    }
    if status != 0 {
        return Err(env_registry_error("Open", status));
    }
    Ok(Some(EnvKey(handle)))
}

#[cfg(windows)]
fn read_windows_user_env() -> Result<Option<EnvRegistryValue>, AppError> {
    use windows_sys::Win32::System::Registry::*;
    let Some(key) = open_env_key(KEY_QUERY_VALUE, false)? else {
        return Ok(None);
    };
    let name: Vec<u16> = DISABLE_ENV_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let (mut kind, mut size) = (0, 0);
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status == 2 {
        return Ok(None);
    }
    if status != 0 {
        return Err(env_registry_error("Read", status));
    }
    if !matches!(kind, REG_SZ | REG_EXPAND_SZ) || size > 65536 || size % 2 != 0 {
        return Err(AppError::Engine(
            "CODEX_SPARKLE_ENABLED 不是正常字符串；保留原值".into(),
        ));
    }
    let mut bytes = vec![0; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            bytes.as_mut_ptr(),
            &mut size,
        )
    };
    if status != 0 {
        return Err(env_registry_error("Read", status));
    }
    bytes.truncate(size as usize);
    if !matches!(kind, REG_SZ | REG_EXPAND_SZ) || size % 2 != 0 {
        return Err(AppError::Engine(
            "读取期间环境变量类型发生变化，保留原值".into(),
        ));
    }
    Ok(Some(EnvRegistryValue { kind, bytes }))
}

#[cfg(windows)]
fn write_windows_user_env(value: Option<&EnvRegistryValue>) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;
    if let Some(key) = open_env_key(KEY_SET_VALUE, value.is_some())? {
        let name: Vec<u16> = DISABLE_ENV_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let status = unsafe {
            match value {
                Some(value) => RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    value.kind,
                    value.bytes.as_ptr(),
                    value.bytes.len() as u32,
                ),
                None => RegDeleteValueW(key.0, name.as_ptr()),
            }
        };
        if status != 0 && !(status == 2 && value.is_none()) {
            return Err(env_registry_error("Write", status));
        }
    }
    if read_windows_user_env()?.as_ref() != value {
        return Err(AppError::Engine(
            "CODEX_SPARKLE_ENABLED 写后回读不符；保留恢复快照".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn broadcast_windows_environment_change() {
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let value = "Environment\0".encode_utf16().collect::<Vec<_>>();
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            value.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        )
    };
    if sent == 0 {
        log::warn!("broadcast Windows environment change failed");
    }
}

#[cfg(test)]
mod tests {
    use super::{env_restore_action, EnvRestore};
    #[test]
    fn uninstall_environment_restore_compares_owned_value() {
        assert_eq!(
            env_restore_action(&Some("false"), &None, &Some("false")),
            EnvRestore::Restore
        );
        assert_eq!(
            env_restore_action(&None::<&str>, &None, &Some("false")),
            EnvRestore::AlreadyOriginal
        );
        assert_eq!(
            env_restore_action(&Some("custom"), &None, &Some("false")),
            EnvRestore::PreserveExternal
        );
        assert_eq!(
            env_restore_action(&None, &Some("before"), &None),
            EnvRestore::Restore
        );
    }
    #[cfg(windows)]
    #[test]
    fn environment_snapshot_preserves_raw_type_and_utf16() {
        let original = super::EnvRegistryValue {
            kind: 2,
            bytes: vec![37, 0, 65, 0, 37, 0, 0, 0],
        };
        let journal = super::EnvJournal {
            schema: 1,
            previous: Some(original.clone()),
            written: Some(super::EnvRegistryValue::plain("false")),
        };
        let decoded: super::EnvJournal =
            serde_json::from_slice(&serde_json::to_vec(&journal).unwrap()).unwrap();
        assert_eq!(decoded.previous, Some(original));
    }
}
