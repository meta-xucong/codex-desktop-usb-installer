//! Product configurator hand-off for the Codex USB installer.
//!
//! The Codex installer itself is intentionally kept separate from the
//! CCSwitch product binary.  Once Codex has been verified, this module copies
//! the CCSwitch payload from the USB bundle (or a Tauri resource bundle) into
//! a per-user, non-removable location and starts it.  The copy is best-effort
//! from the installer's point of view: a missing CCSwitch payload must never
//! turn an otherwise successful Codex installation into a failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::errors::AppError;

pub const CCSWITCH_EXECUTABLE: &str = "CodexSwitch.exe";
const CCSWITCH_SOURCE_DIR: &str = "ccswitch";
pub const CCSWITCH_INSTALL_DIR: &str = "CodexSwitch";
const CCSWITCH_DATA_DIR: &str = ".cc-switch";
const CCSWITCH_IDENTIFIER: &str = "com.ccswitch.desktop";
const CCSWITCH_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const CCSWITCH_RUN_VALUE: &str = "CC Switch";

/// Files copied when they exist beside the product executable.  Only the EXE
/// is required at runtime; the catalog and notices make the local install
/// self-describing and preserve the open-source attribution.
const CCSWITCH_PAYLOAD_FILES: &[&str] = &[
    "CodexSwitch.exe",
    "providers.json",
    "manifest.json",
    "LICENSE",
    "SOURCE-NOTICE.txt",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CcswitchLaunchReport {
    /// Whether a CCSwitch executable was found in the installer bundle.
    pub available: bool,
    /// Whether the payload was copied to the local per-user directory.
    pub installed: bool,
    /// Whether the local executable was successfully spawned.
    pub launched: bool,
    pub source_path: Option<String>,
    pub installed_path: Option<String>,
    pub copied_files: usize,
    pub message: String,
}

/// Result of removing the per-user CCSwitch copy installed by this manager.
/// The operation is deliberately idempotent: a missing directory is treated
/// as already removed so a second cleanup attempt can finish after a locked
/// process has exited.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CcswitchUninstallReport {
    pub success: bool,
    pub installed_before: bool,
    pub removed_install: bool,
    pub removed_autostart: bool,
    pub purged_user_data: bool,
    pub install_dir: String,
    pub data_dirs: Vec<String>,
    pub message: String,
    pub notes: Vec<String>,
}

fn report_missing(message: impl Into<String>) -> CcswitchLaunchReport {
    CcswitchLaunchReport {
        available: false,
        installed: false,
        launched: false,
        source_path: None,
        installed_path: None,
        copied_files: 0,
        message: message.into(),
    }
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> AppError {
    AppError::Engine(format!("写入 {} 失败: {error}", path.display()))
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Return the candidate locations in deterministic priority order.  A
/// sibling of the running installer is the USB layout; resource_dir covers a
/// future NSIS/MSIX bundle that embeds `ccswitch/*` as Tauri resources.
fn source_candidates(executable: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = executable.parent() {
        candidates.push(parent.join(CCSWITCH_SOURCE_DIR).join(CCSWITCH_EXECUTABLE));
        candidates.push(
            parent
                .join("resources")
                .join(CCSWITCH_SOURCE_DIR)
                .join(CCSWITCH_EXECUTABLE),
        );
    }
    if let Some(resource_dir) = resource_dir {
        candidates.push(
            resource_dir
                .join(CCSWITCH_SOURCE_DIR)
                .join(CCSWITCH_EXECUTABLE),
        );
        candidates.push(resource_dir.join(CCSWITCH_EXECUTABLE));
    }
    candidates
}

fn find_source(executable: &Path, resource_dir: Option<&Path>) -> Option<PathBuf> {
    source_candidates(executable, resource_dir)
        .into_iter()
        .find(|candidate| is_regular_file(candidate))
}

pub fn local_install_dir() -> Result<PathBuf, AppError> {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local_app_data)
            .join("Programs")
            .join(CCSWITCH_INSTALL_DIR));
    }

    // This fallback keeps the command testable on non-Windows hosts while
    // still following the per-user data convention when LOCALAPPDATA is not
    // present (for example in a restricted Windows shell).
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join(CCSWITCH_INSTALL_DIR))
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch 的本地安装目录".to_string()))
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(base_dirs) = directories::BaseDirs::new() {
        dirs.push(base_dirs.home_dir().join(CCSWITCH_DATA_DIR));
    }

    // Tauri's WebView profile is separate from CCSwitch's SQLite/config
    // directory.  Include both Windows profile roots so a full purge also
    // removes the first-run marker and cached UI state, regardless of the
    // WebView runtime version used on the target machine.
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            dirs.push(PathBuf::from(root).join(CCSWITCH_IDENTIFIER));
        }
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

fn remove_dir_all_retry(path: &Path) -> Result<bool, AppError> {
    if !path.exists() {
        return Ok(false);
    }

    let mut last_error = None;
    for attempt in 0..3 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(true),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }

    Err(io_error(
        path,
        last_error.expect("remove_dir_all must record its last error"),
    ))
}

#[cfg(target_os = "windows")]
fn terminate_running_process() -> Result<bool, AppError> {
    // Query first so a missing process is not reported as a failure.  The
    // image name is unique to the bundled product, and taskkill /T also closes
    // any renderer child processes that would otherwise keep files locked.
    let listed = Command::new("tasklist")
        .args([
            "/FI",
            &format!("IMAGENAME eq {CCSWITCH_EXECUTABLE}"),
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .map_err(|error| AppError::Engine(format!("查询 CCSwitch 进程失败: {error}")))?;
    let listing = String::from_utf8_lossy(&listed.stdout).to_ascii_lowercase();
    if !listing.contains(&CCSWITCH_EXECUTABLE.to_ascii_lowercase()) {
        return Ok(false);
    }

    let killed = Command::new("taskkill")
        .args(["/IM", CCSWITCH_EXECUTABLE, "/T", "/F"])
        .output()
        .map_err(|error| AppError::Engine(format!("关闭 CCSwitch 进程失败: {error}")))?;
    if !killed.status.success() {
        let detail = String::from_utf8_lossy(&killed.stderr).trim().to_string();
        return Err(AppError::Engine(if detail.is_empty() {
            "关闭 CCSwitch 进程失败".to_string()
        } else {
            format!("关闭 CCSwitch 进程失败: {detail}")
        }));
    }
    thread::sleep(Duration::from_millis(150));
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
fn terminate_running_process() -> Result<bool, AppError> {
    Ok(false)
}

#[cfg(target_os = "windows")]
fn remove_autostart_entry(install_dir: &Path) -> Result<bool, AppError> {
    let queried = Command::new("reg")
        .args(["query", CCSWITCH_RUN_KEY, "/v", CCSWITCH_RUN_VALUE])
        .output()
        .map_err(|error| AppError::Engine(format!("查询 CCSwitch 开机启动项失败: {error}")))?;
    if !queried.status.success() {
        return Ok(false);
    }

    // Only remove the value when it points at the copy managed by this
    // installer.  A separately installed CCSwitch at another path is outside
    // this product's scope and must not be touched.
    let value = String::from_utf8_lossy(&queried.stdout)
        .to_ascii_lowercase()
        .replace('/', "\\");
    let expected = install_dir
        .join(CCSWITCH_EXECUTABLE)
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    if !value.contains(&expected) {
        return Ok(false);
    }

    let deleted = Command::new("reg")
        .args(["delete", CCSWITCH_RUN_KEY, "/v", CCSWITCH_RUN_VALUE, "/f"])
        .output()
        .map_err(|error| AppError::Engine(format!("删除 CCSwitch 开机启动项失败: {error}")))?;
    if !deleted.status.success() {
        let detail = String::from_utf8_lossy(&deleted.stderr).trim().to_string();
        return Err(AppError::Engine(if detail.is_empty() {
            "删除 CCSwitch 开机启动项失败".to_string()
        } else {
            format!("删除 CCSwitch 开机启动项失败: {detail}")
        }));
    }
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
fn remove_autostart_entry(_install_dir: &Path) -> Result<bool, AppError> {
    Ok(false)
}

/// Remove the local CCSwitch program copy and, when requested, its per-user
/// configuration/WebView data.  No Codex data is touched here; that remains
/// the responsibility of the Codex uninstall engine.
pub fn uninstall(purge_user_data: bool) -> Result<CcswitchUninstallReport, AppError> {
    let install_dir = local_install_dir()?;
    let installed_before = install_dir.exists();
    let data_paths = data_dirs();
    let mut notes = Vec::new();

    let terminated = terminate_running_process()?;
    if terminated {
        notes.push("已关闭正在运行的 CCSwitch 进程。".to_string());
    }

    let removed_autostart = remove_autostart_entry(&install_dir)?;
    if removed_autostart {
        notes.push("已移除 CCSwitch 开机启动项。".to_string());
    }

    let removed_install = remove_dir_all_retry(&install_dir)?;
    if removed_install {
        notes.push(format!(
            "已删除 CCSwitch 程序目录：{}",
            install_dir.display()
        ));
    }

    let mut purged_user_data = false;
    if purge_user_data {
        let mut removed_any = false;
        for data_path in &data_paths {
            if remove_dir_all_retry(data_path)? {
                removed_any = true;
                notes.push(format!("已删除 CCSwitch 数据目录：{}", data_path.display()));
            }
        }
        purged_user_data = removed_any || data_paths.iter().all(|path| !path.exists());
    } else {
        notes.push("已保留 CCSwitch 配置与用户数据。".to_string());
    }

    let success =
        !install_dir.exists() && (!purge_user_data || data_paths.iter().all(|path| !path.exists()));
    let message = if !success {
        "CCSwitch 程序已部分清理，请关闭占用文件的进程后重试。".to_string()
    } else if installed_before || removed_autostart || purged_user_data {
        if purge_user_data {
            "CCSwitch 已卸载并清除配置。".to_string()
        } else {
            "CCSwitch 已卸载，配置已保留。".to_string()
        }
    } else {
        "未检测到 CCSwitch 安装。".to_string()
    };

    Ok(CcswitchUninstallReport {
        success,
        installed_before,
        removed_install,
        removed_autostart,
        purged_user_data,
        install_dir: install_dir.to_string_lossy().into_owned(),
        data_dirs: data_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        message,
        notes,
    })
}

fn copy_payload(source_executable: &Path, target_dir: &Path) -> Result<usize, AppError> {
    fs::create_dir_all(target_dir).map_err(|error| io_error(target_dir, error))?;
    let source_dir = source_executable
        .parent()
        .ok_or_else(|| AppError::Engine("无法确定 CCSwitch 载荷目录".to_string()))?;
    let target_executable = target_dir.join(CCSWITCH_EXECUTABLE);
    let temp_executable = target_dir.join(format!(".{CCSWITCH_EXECUTABLE}.new"));

    // Copy the executable through a sibling temporary file.  This avoids a
    // partially written EXE if the USB read is interrupted.  Replacing a
    // currently running executable can fail on Windows; in that case the
    // existing installation remains intact and the error is surfaced to the
    // caller rather than deleting it.
    let mut copied = 0usize;
    for file_name in CCSWITCH_PAYLOAD_FILES {
        let source = source_dir.join(file_name);
        if !is_regular_file(&source) {
            continue;
        }
        let target = target_dir.join(file_name);
        if *file_name == CCSWITCH_EXECUTABLE {
            fs::copy(&source, &temp_executable)
                .map_err(|error| io_error(&temp_executable, error))?;
            if target.exists() {
                fs::remove_file(&target).map_err(|error| io_error(&target, error))?;
            }
            if let Err(error) = fs::rename(&temp_executable, &target) {
                let _ = fs::remove_file(&temp_executable);
                return Err(io_error(&target, error));
            }
        } else {
            fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        }
        copied += 1;
    }

    if !is_regular_file(&target_executable) {
        return Err(AppError::Engine(
            "CCSwitch 载荷中缺少 CodexSwitch.exe".to_string(),
        ));
    }
    Ok(copied)
}

/// Copy CCSwitch locally and launch it.  A missing source is represented as a
/// non-fatal report so the caller can still mark Codex installation complete.
pub fn install_and_launch(app: &AppHandle) -> Result<CcswitchLaunchReport, AppError> {
    if !cfg!(target_os = "windows") {
        return Ok(report_missing("当前平台不支持自动启动 Windows CCSwitch"));
    }

    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("无法确定安装器路径: {error}")))?;
    let resource_dir = app.path().resource_dir().ok();
    let Some(source) = find_source(&executable, resource_dir.as_deref()) else {
        return Ok(report_missing(
            "未找到 ccswitch\\CodexSwitch.exe；Codex 已安装，但 CCSwitch 未自动启动",
        ));
    };
    let target_dir = local_install_dir()?;
    let copied_files = copy_payload(&source, &target_dir)?;
    let target = target_dir.join(CCSWITCH_EXECUTABLE);
    Command::new(&target)
        .current_dir(&target_dir)
        .spawn()
        .map_err(|error| io_error(&target, error))?;

    Ok(CcswitchLaunchReport {
        available: true,
        installed: true,
        launched: true,
        source_path: Some(source.to_string_lossy().into_owned()),
        installed_path: Some(target.to_string_lossy().into_owned()),
        copied_files,
        message: "Codex Switch 已复制到本机并启动，请输入 API Key 完成配置".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{source_candidates, CCSWITCH_EXECUTABLE, CCSWITCH_SOURCE_DIR};
    use std::path::Path;

    #[test]
    fn sibling_usb_payload_is_checked_before_embedded_resources() {
        let candidates = source_candidates(
            Path::new(r"F:\usb\CodexDesktopSetup.exe"),
            Some(Path::new(r"C:\Program Files\CodexManager\resources")),
        );
        assert_eq!(
            candidates[0],
            Path::new(r"F:\usb")
                .join(CCSWITCH_SOURCE_DIR)
                .join(CCSWITCH_EXECUTABLE)
        );
        assert!(candidates.iter().any(|candidate| {
            candidate
                == &Path::new(r"C:\Program Files\CodexManager\resources")
                    .join(CCSWITCH_SOURCE_DIR)
                    .join(CCSWITCH_EXECUTABLE)
        }));
    }
}
