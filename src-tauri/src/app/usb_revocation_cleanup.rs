//! Narrow, idempotent cleanup used only after a verified signed `revoked`
//! response. This module never calls the general USB uninstall workflow.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::app::usb_guard::{UsbFingerprint, UsbLicense};
use crate::app::usb_revocation::{self, CleanupProgress, RevocationStateFile};
use crate::errors::AppError;

const APP_DIR: &str = "apps/Codex";
const ROOT_STAGING_DIR: &str = ".staging";
const APP_STAGING_DIR: &str = "apps/.codex-app-manager-staging";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupReport {
    pub complete: bool,
    pub removed: Vec<String>,
    pub failed: Vec<String>,
}

pub fn cleanup_revoked_bundle(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Result<CleanupReport, AppError> {
    cleanup_revoked_bundle_with_observer(
        root,
        license,
        fingerprint,
        crate::app::usb_guard::observe_bundle,
    )
}

fn cleanup_revoked_bundle_with_observer(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    observe: impl Fn(&Path) -> Result<crate::app::usb_guard::UsbObservation, AppError>,
) -> Result<CleanupReport, AppError> {
    cleanup_revoked_bundle_with_observer_and_process_probe(
        root,
        license,
        fingerprint,
        observe,
        strict_process_probe,
        crate::app::usb_guard::close_codex_entry_processes_for_revocation,
    )
}

fn strict_process_probe(root: &Path) -> Result<bool, String> {
    codex_win_engine::processes_running_for_root_strict(&codex_process_scope(root))
        .map_err(|error| error.to_string())
}

fn codex_process_scope(bundle_root: &Path) -> std::path::PathBuf {
    bundle_root.join(APP_DIR)
}

fn cleanup_revoked_bundle_with_observer_and_process_probe(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    observe: impl Fn(&Path) -> Result<crate::app::usb_guard::UsbObservation, AppError>,
    process_probe: impl Fn(&Path) -> Result<bool, String>,
    close_codex: impl Fn(&Path) -> Result<(), AppError>,
) -> Result<CleanupReport, AppError> {
    reject_reparse_path_chain(root)?;
    if !root.is_dir() {
        return Err(engine("U 盘安装目录不可用"));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| engine(error.to_string()))?;
    reject_reparse_path_chain(root)?;
    if fs::canonicalize(root).map_err(|error| engine(error.to_string()))? != canonical_root {
        return Err(engine("U 盘安装目录在路径核验期间发生变化，拒绝清理"));
    }
    reject_reparse(&canonical_root)?;
    let actual_fingerprint = observe_and_validate_identity_with_observer(
        &canonical_root,
        license,
        fingerprint,
        &observe,
    )?;
    // The long installation transaction guard serializes update staging,
    // commit/rollback, and revoked cleanup across processes. It is acquired
    // before the shorter state lock, and remains held through deletion.
    let transaction_guard =
        usb_revocation::acquire_revocation_cleanup_transaction_lock(&license.installation_id)
            .map_err(|error| engine(format!("无法锁定 USB 安装事务：{error}")))?;
    if transaction_guard.installation_id() != license.installation_id {
        return Err(engine("USB 安装事务锁身份不匹配"));
    }
    let current_fingerprint = observe_and_validate_identity_with_observer(
        &canonical_root,
        license,
        &actual_fingerprint,
        &observe,
    )?;
    // Serialize the verified revoked decision through process close, every
    // deletion, and the final progress write. A newer active check cannot
    // become authoritative halfway through this cleanup transaction.
    let state_lock = usb_revocation::acquire_cleanup_state_lock(license)
        .map_err(|error| engine(format!("无法锁定授权状态清理事务：{error}")))?;
    if !codex_usb_license::fingerprints_match(&current_fingerprint, &actual_fingerprint) {
        return Err(engine("USB 指纹在清理事务锁定期间发生变化"));
    }
    let cleanup_root = crate::app::uninstall_cleanup::pin_revocation_cleanup_root(
        &canonical_root,
        &actual_fingerprint,
    )
    .map_err(|error| engine(format!("固定并校验 Bundle 根句柄失败：{error}")))?;
    let state = usb_revocation::merged_state(&canonical_root, license, &actual_fingerprint)
        .ok_or_else(|| engine("缺少可验证的远程作废回执"))?;
    let highest = usb_revocation::highest_state(&state, license, &actual_fingerprint)
        .ok_or_else(|| engine("远程作废回执无效"))?;
    if highest.payload.status != "revoked" {
        return Err(engine("当前最高签名状态不是已作废"));
    }
    // The pinned cleanup root already validated schema 3 and the anchored
    // registration policy from its bounded, handle-relative manifest reads.
    let payload_relative = cleanup_root.payload_relative().to_string();
    crate::app::uninstall_cleanup::validate_revocation_backup(&cleanup_root).map_err(engine)?;
    let backup_relative = cleanup_root.backup_relative().to_string();
    reject_reparse_ancestors(&canonical_root, &canonical_root.join(ROOT_STAGING_DIR))?;
    let transaction_dirs = crate::app::channels::codex::revocation_transaction_dirs(
        &canonical_root,
        license,
        &actual_fingerprint,
        &cleanup_root,
    )
    .map_err(|error| engine(format!("核验 Codex 更新事务目录失败：{error}")))?;
    let mut targets = vec![
        CleanupTarget {
            label: "MSIX 载荷",
            relative: payload_relative.clone(),
        },
        CleanupTarget {
            label: "Codex 运行目录",
            relative: APP_DIR.to_string(),
        },
        CleanupTarget {
            label: "Codex 便携安装暂存",
            relative: APP_STAGING_DIR.to_string(),
        },
        CleanupTarget {
            label: "Codex 载荷备份",
            relative: backup_relative.clone(),
        },
    ];
    let state_tmp_targets = collect_state_tmp_targets(&canonical_root)?;
    for target in &state_tmp_targets {
        targets.push(CleanupTarget {
            label: "授权状态临时文件",
            relative: target.clone(),
        });
    }

    // Inventory every authorized tree before deleting the first one. Unknown
    // staging entries, hard links, reparses, and unreadable descendants stop
    // the whole cleanup before any target is removed.
    for target in &targets {
        crate::app::uninstall_cleanup::check_revocation_target(&cleanup_root, &target.relative)
            .map_err(|error| engine(format!("{} 预检查失败：{error}", target.label)))?;
    }

    // Close only the original MSIX entry processes under this Bundle's
    // apps/Codex. The helper validates and acts through the same process
    // handle. Manager/helper and other Bundle processes are outside its scope.
    // Missing/invalid AppxManifest fails closed instead of guessing a name.
    close_codex(&canonical_root)
        .map_err(|error| engine(format!("关闭当前 U 盘 Codex 失败，未清理文件：{error}")))?;
    let probe = || process_probe(&canonical_root);
    ensure_no_codex_process(probe)?;

    let mut removed = Vec::new();
    let mut failed = Vec::new();
    for target in &targets {
        let probe = || process_probe(&canonical_root);
        if let Err(error) = ensure_no_codex_process(probe).and_then(|()| {
            crate::app::uninstall_cleanup::remove_revocation_target(&cleanup_root, &target.relative)
                .map_err(engine)
        }) {
            failed.push(format!("{}：{error}", target.label));
        } else {
            removed.push(target.label.to_string());
        }
        if !failed.is_empty() {
            break;
        }
    }
    if failed.is_empty() {
        for transaction in transaction_dirs {
            let probe = || process_probe(&canonical_root);
            let label = format!("Codex 更新事务 {}", transaction.relative());
            if let Err(error) = ensure_no_codex_process(probe)
                .and_then(|()| transaction.remove(&cleanup_root).map_err(engine))
            {
                failed.push(format!("{label}：{error}"));
                break;
            }
            removed.push(label);
        }
    }
    let remaining_transactions = crate::app::channels::codex::revocation_transaction_dirs(
        &canonical_root,
        license,
        &actual_fingerprint,
        &cleanup_root,
    )
    .map_err(|error| engine(format!("复核 Codex 更新事务目录失败：{error}")))?;
    let payload_exists =
        crate::app::uninstall_cleanup::exists_revocation_target(&cleanup_root, &payload_relative)
            .map_err(engine)?;
    let app_exists =
        crate::app::uninstall_cleanup::exists_revocation_target(&cleanup_root, APP_DIR)
            .map_err(engine)?;
    let app_staging_exists =
        crate::app::uninstall_cleanup::exists_revocation_target(&cleanup_root, APP_STAGING_DIR)
            .map_err(engine)?;
    let backup_exists =
        crate::app::uninstall_cleanup::exists_revocation_target(&cleanup_root, &backup_relative)
            .map_err(engine)?;
    let state_temps_exist = state_tmp_targets
        .iter()
        .try_fold(false, |found, relative| {
            crate::app::uninstall_cleanup::exists_revocation_target(&cleanup_root, relative)
                .map(|exists| found || exists)
                .map_err(engine)
        })?;
    let transaction_exists = !remaining_transactions.is_empty();
    let complete = failed.is_empty()
        && !payload_exists
        && !app_exists
        && remaining_transactions.is_empty()
        && !transaction_exists
        && !app_staging_exists
        && !backup_exists
        && !state_temps_exist;
    let progress = CleanupProgress {
        payload_removed: !payload_exists,
        runtime_removed: !app_exists,
        staging_removed: remaining_transactions.is_empty()
            && !transaction_exists
            && !app_staging_exists
            && !backup_exists,
        last_error: (!failed.is_empty()).then(|| failed.join("；")),
        updated_at: Some(crate::app::usb_guard::now_ms().to_string()),
    };
    usb_revocation::write_cleanup_progress_locked(
        &canonical_root,
        license,
        &actual_fingerprint,
        progress,
        &state_lock,
    )
    .map_err(engine)?;
    Ok(CleanupReport {
        complete,
        removed,
        failed,
    })
}

#[cfg(test)]
pub(crate) fn cleanup_revoked_bundle_for_test(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
    observe: impl Fn(&Path) -> Result<crate::app::usb_guard::UsbObservation, AppError>,
    process_probe: impl Fn(&Path) -> Result<bool, String>,
) -> Result<CleanupReport, AppError> {
    cleanup_revoked_bundle_with_observer_and_process_probe(
        root,
        license,
        fingerprint,
        observe,
        process_probe,
        |_| Ok(()),
    )
}

pub fn validate_revoked_receipt(
    root: &Path,
    license: &UsbLicense,
    fingerprint: &UsbFingerprint,
) -> Option<RevocationStateFile> {
    let actual_fingerprint = observe_and_validate_identity(root, license, fingerprint).ok()?;
    let state = usb_revocation::merged_state(root, license, &actual_fingerprint)?;
    let highest = usb_revocation::highest_state(&state, license, &actual_fingerprint)?;
    (highest.payload.status == "revoked").then_some(state)
}

fn observe_and_validate_identity(
    root: &Path,
    license: &UsbLicense,
    supplied: &UsbFingerprint,
) -> Result<UsbFingerprint, AppError> {
    observe_and_validate_identity_with_observer(
        root,
        license,
        supplied,
        crate::app::usb_guard::observe_bundle,
    )
}

fn observe_and_validate_identity_with_observer(
    root: &Path,
    license: &UsbLicense,
    supplied: &UsbFingerprint,
    observe: impl Fn(&Path) -> Result<crate::app::usb_guard::UsbObservation, AppError>,
) -> Result<UsbFingerprint, AppError> {
    let observation =
        observe(root).map_err(|error| engine(format!("重新读取 U 盘身份失败：{error}")))?;
    if !observation.removable {
        return Err(engine("当前路径不是已连接的可移动 U 盘"));
    }
    let actual = observation
        .fingerprint
        .ok_or_else(|| engine("无法从当前清理目录读取完整 U 盘指纹"))?;
    validate_fingerprint_binding(license, supplied, &actual)?;
    match crate::app::usb_guard::validate_license(license, &actual) {
        Ok(true) => Ok(actual),
        Ok(false) => Err(engine("清理作废 Bundle 需要有效的签名许可证")),
        Err(error) => Err(engine(format!("清理许可证校验失败：{error}"))),
    }
}

fn validate_fingerprint_binding(
    license: &UsbLicense,
    supplied: &UsbFingerprint,
    observed: &UsbFingerprint,
) -> Result<(), AppError> {
    if !codex_usb_license::fingerprints_match(observed, supplied)
        || !codex_usb_license::fingerprints_match(observed, &license.fingerprint)
    {
        return Err(engine(
            "当前清理目录的卷序列号、锚点或相对路径与许可证和请求不一致",
        ));
    }
    Ok(())
}

fn required_relative(value: &Value, pointer: &str) -> Result<String, AppError> {
    let raw = value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| engine(format!("根清单缺少路径字段：{pointer}")))?;
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(engine("根清单目标路径不安全"));
    }
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanupTarget {
    label: &'static str,
    relative: String,
}

fn ensure_no_codex_process(
    process_probe: impl Fn() -> Result<bool, String>,
) -> Result<(), AppError> {
    match process_probe() {
        Ok(false) => Ok(()),
        Ok(true) => Err(engine(
            "检测到仍从当前 U 盘目录运行的 Codex，未删除任何目标；请关闭 Codex 后重试",
        )),
        Err(error) => Err(engine(format!(
            "无法确认 Codex 是否仍在运行，按安全策略停止清理：{error}"
        ))),
    }
}

fn collect_state_tmp_targets(root: &Path) -> Result<Vec<String>, AppError> {
    let state_dir = root.join("CodexUsbState");
    reject_reparse_ancestors(root, &state_dir)?;
    match fs::symlink_metadata(&state_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(engine(format!("检查授权状态目录失败：{error}"))),
        Ok(metadata) if is_reparse_metadata(&metadata) => {
            return Err(engine("授权状态目录是重解析点，拒绝清理"))
        }
        Ok(metadata) if !metadata.is_dir() => return Err(engine("授权状态路径不是目录，拒绝清理")),
        Ok(_) => {}
    }
    let mut targets = Vec::new();
    for entry in fs::read_dir(&state_dir).map_err(|error| engine(error.to_string()))? {
        let entry = entry.map_err(|error| engine(error.to_string()))?;
        let path = entry.path();
        reject_reparse_ancestors(root, &path)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if usb_revocation::is_atomic_state_tmp_name(name) {
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| engine(error.to_string()))?;
            if !metadata.is_file() || is_reparse_metadata(&metadata) || !is_single_link_file(&path)
            {
                return Err(engine("授权状态原子临时项不是单链接普通文件，拒绝清理"));
            }
            if targets.len() >= usb_revocation::MAX_ATOMIC_STATE_TEMPS {
                return Err(engine("授权状态原子临时项数量超过安全上限，拒绝清理"));
            }
            targets.push(format!("CodexUsbState/{name}"));
        }
    }
    Ok(targets)
}

#[cfg(windows)]
fn is_single_link_file(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    else {
        return false;
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    (unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) != 0 })
        && info.nNumberOfLinks == 1
        && info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0
}

#[cfg(unix)]
fn is_single_link_file(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1
    })
}

#[cfg(not(any(windows, unix)))]
fn is_single_link_file(_path: &Path) -> bool {
    false
}

fn reject_reparse_ancestors(root: &Path, path: &Path) -> Result<(), AppError> {
    if !path.starts_with(root) {
        return Err(engine("清理路径超出 Bundle"));
    }
    let mut current = path.to_path_buf();
    loop {
        reject_reparse(&current)?;
        if current == root {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| engine("清理路径缺少 Bundle 根目录祖先"))?
            .to_path_buf();
    }
    Ok(())
}

fn reject_reparse_path_chain(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(engine("U 盘安装目录必须是绝对路径"));
    }
    let mut current = path.to_path_buf();
    loop {
        reject_reparse(&current)?;
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_metadata(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn reject_reparse(path: &Path) -> Result<(), AppError> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
            Err(engine(format!("拒绝穿过重解析点：{}", path.display())))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(engine(error.to_string())),
    }
}

#[cfg(not(windows))]
fn reject_reparse(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(engine(format!("拒绝穿过符号链接：{}", path.display())))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(engine(error.to_string())),
    }
}

fn engine(message: impl Into<String>) -> AppError {
    AppError::Engine(format!("U 盘授权清理：{}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_target_must_be_a_safe_relative_path() {
        for value in [
            "../outside.msix",
            "payload/../../outside.msix",
            "/absolute/path.msix",
            "C:/outside.msix",
            "payload//file.msix",
            "payload/./file.msix",
            "payload/file.msix/..",
            "",
        ] {
            assert!(
                required_relative(&serde_json::json!({"path": value}), "/path").is_err(),
                "accepted unsafe path {value:?}"
            );
        }
        assert_eq!(
            required_relative(
                &serde_json::json!({"path": "payload/OpenAI.Codex_x64.msix"}),
                "/path"
            )
            .unwrap(),
            "payload/OpenAI.Codex_x64.msix"
        );
    }

    #[test]
    fn revocation_delete_allowlist_is_narrow_and_uses_pinned_manifest_payload() {
        let payload = "payload/OpenAI.Codex_x64.msix";
        for allowed in [
            payload,
            "apps/Codex",
            "apps/.codex-app-manager-staging",
            "payload/_backup",
            ".staging/codex-update-123-abc",
            "CodexUsbState/revocation.json.tmp-123-4",
        ] {
            assert!(
                crate::app::uninstall_cleanup::authorized_revocation_target(payload, allowed)
                    .is_ok(),
                "{allowed}"
            );
        }
        for rejected in [
            "..",
            "payload/other.msix",
            "payload/OpenAI.Codex_x64.msix/child",
            ".staging",
            ".staging/user-data",
            "apps",
            "CodexUsbState/revocation.json",
            "CodexUsbState/revocation.json.tmp-000123-4",
            "C:/outside.msix",
        ] {
            assert!(
                crate::app::uninstall_cleanup::authorized_revocation_target(payload, rejected)
                    .is_err(),
                "accepted {rejected}"
            );
        }
    }

    #[test]
    fn cleanup_requires_full_observed_usb_fingerprint_match() {
        let observed = UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: "a".repeat(64),
            bundle_relative_path: "products/codex".to_string(),
            anchor_schema: 3,
        };
        let license = UsbLicense {
            schema_version: codex_usb_license::LICENSE_SCHEMA_VERSION,
            product: codex_usb_license::PRODUCT_ID.to_string(),
            installation_id: "cleanup-fingerprint-test".to_string(),
            fingerprint: observed.clone(),
            issued_at: "1760000000000".to_string(),
            key_id: Some("test-key".to_string()),
            account_binding: None,
            signature: Some("test-signature".to_string()),
        };
        assert!(validate_fingerprint_binding(&license, &observed, &observed).is_ok());

        for mismatch in [
            UsbFingerprint {
                volume_serial: observed.volume_serial + 1,
                ..observed.clone()
            },
            UsbFingerprint {
                anchor_schema: 2,
                ..observed.clone()
            },
            UsbFingerprint {
                anchor_sha256: "b".repeat(64),
                ..observed.clone()
            },
            UsbFingerprint {
                bundle_relative_path: "products/other".to_string(),
                ..observed.clone()
            },
        ] {
            assert!(validate_fingerprint_binding(&license, &mismatch, &observed).is_err());
            assert!(validate_fingerprint_binding(&license, &observed, &mismatch).is_err());
        }

        let root = std::env::temp_dir().join(format!(
            "usb-revocation-observer-mismatch-{}",
            uuid::Uuid::new_v4()
        ));
        let keep_file = root.join("apps/Codex/keep.txt");
        fs::create_dir_all(keep_file.parent().unwrap()).unwrap();
        fs::write(&keep_file, b"cleanup must not start").unwrap();
        for mismatch in [
            UsbFingerprint {
                volume_serial: observed.volume_serial + 1,
                ..observed.clone()
            },
            UsbFingerprint {
                anchor_schema: 2,
                ..observed.clone()
            },
            UsbFingerprint {
                anchor_sha256: "b".repeat(64),
                ..observed.clone()
            },
            UsbFingerprint {
                bundle_relative_path: "products/other".to_string(),
                ..observed.clone()
            },
        ] {
            let observation = crate::app::usb_guard::UsbObservation {
                removable: true,
                drive_root: Some(root.to_string_lossy().into_owned()),
                bundle_path: root.to_string_lossy().into_owned(),
                fingerprint: Some(mismatch),
            };
            let error = cleanup_revoked_bundle_with_observer(&root, &license, &observed, |_| {
                Ok(observation.clone())
            })
            .unwrap_err();
            assert!(error.to_string().contains("卷序列号、锚点或相对路径"));
            assert_eq!(fs::read(&keep_file).unwrap(), b"cleanup must not start");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn running_codex_probe_blocks_delete_and_preserves_target() {
        let result = ensure_no_codex_process(|| Ok(true));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("仍从当前 U 盘目录运行"));
        let probe_error = ensure_no_codex_process(|| Err("process image query failed".to_string()));
        assert!(probe_error.unwrap_err().to_string().contains("停止清理"));
        assert!(ensure_no_codex_process(|| Ok(false)).is_ok());

        let mut target_deleted = false;
        let attempt = ensure_no_codex_process(|| Err("live PID image path is unknown".to_string()))
            .and_then(|()| {
                target_deleted = true;
                Ok(())
            });
        assert!(attempt.is_err());
        assert!(
            !target_deleted,
            "unknown live process must stop before deletion"
        );
    }

    #[test]
    fn revocation_process_scope_is_only_the_current_bundle_codex_directory() {
        let bundle = std::env::temp_dir().join("codex-revocation-scope-fixture");
        assert_eq!(codex_process_scope(&bundle), bundle.join("apps/Codex"));
        assert_ne!(codex_process_scope(&bundle), bundle);
    }

    #[test]
    fn cleanup_recognizes_only_atomic_state_temp_siblings() {
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-tmp-cleanup-{}",
            uuid::Uuid::new_v4()
        ));
        let state_dir = root.join("CodexUsbState");
        fs::create_dir_all(&state_dir).unwrap();
        let exact = state_dir.join("revocation.json.tmp-123-4");
        let near_miss = state_dir.join("revocation.json.tmp-123-four");
        fs::write(&exact, b"interrupted").unwrap();
        fs::write(&near_miss, b"preserve").unwrap();
        let targets = collect_state_tmp_targets(&root).unwrap();
        assert_eq!(targets, vec!["CodexUsbState/revocation.json.tmp-123-4"]);
        fs::write(state_dir.join("revocation.json.tmp-123-5"), b"more temp").unwrap();
        assert_eq!(collect_state_tmp_targets(&root).unwrap().len(), 2);
        assert_eq!(fs::read(&near_miss).unwrap(), b"preserve");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleanup_rejects_hardlinked_or_excess_atomic_state_temps() {
        let root = std::env::temp_dir().join(format!(
            "usb-revocation-tmp-bounds-{}",
            uuid::Uuid::new_v4()
        ));
        let state_dir = root.join("CodexUsbState");
        fs::create_dir_all(&state_dir).unwrap();
        let outside = root.join("keep.txt");
        let linked_temp = state_dir.join("revocation.json.tmp-123-1");
        fs::write(&outside, b"must remain").unwrap();
        fs::hard_link(&outside, &linked_temp).unwrap();
        assert!(collect_state_tmp_targets(&root)
            .unwrap_err()
            .to_string()
            .contains("单链接"));
        fs::remove_file(&linked_temp).unwrap();

        for index in 1..=usb_revocation::MAX_ATOMIC_STATE_TEMPS + 1 {
            fs::write(
                state_dir.join(format!("revocation.json.tmp-123-{index}")),
                b"temp",
            )
            .unwrap();
        }
        assert!(collect_state_tmp_targets(&root)
            .unwrap_err()
            .to_string()
            .contains("数量超过安全上限"));
        assert_eq!(fs::read(outside).unwrap(), b"must remain");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn raw_junction_root_is_rejected_before_canonicalization() {
        use std::os::windows::process::CommandExt;

        let fixture = std::env::temp_dir().join(format!(
            "usb-revocation-root-junction-{}",
            uuid::Uuid::new_v4()
        ));
        let outside = fixture.join("outside");
        let junction = fixture.join("usb");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let result = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(reject_reparse_path_chain(&junction).is_err());
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
        fs::remove_dir(&junction).unwrap();
        fs::remove_dir_all(&fixture).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn raw_symlink_root_is_rejected_before_canonicalization() {
        let fixture =
            std::env::temp_dir().join(format!("usb-revocation-root-link-{}", uuid::Uuid::new_v4()));
        let outside = fixture.join("outside");
        let link = fixture.join("usb");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(reject_reparse_path_chain(&link).is_err());
        fs::remove_file(&link).unwrap();
        fs::remove_dir_all(&fixture).unwrap();
    }
}
