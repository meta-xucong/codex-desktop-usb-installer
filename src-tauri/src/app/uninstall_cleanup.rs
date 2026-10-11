//! Deferred cleanup runs before Tauri, logging, runtime installation, and USB guard startup.
//! No caller-provided deletion paths are accepted. The plan records identity only;
//! deletion targets are reconstructed from Windows known folders in both processes.

// The public non-Windows stubs keep the manager API portable; the recovery,
// finalizer and identity-pinning implementation is deliberately Windows-only.
#![cfg_attr(not(windows), allow(dead_code))]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const HELPER_ARG: &str = "--codex-uninstall-cleanup";
const HELPER_PREFIX: &str = "codex-uninstall-helper-";
const HELPER_EXE: &str = "CodexUninstallCleanup.exe";
const RETRIES: usize = 8;
const RETRY_DELAY: Duration = Duration::from_millis(500);
const HELPER_FILES: &[&str] = &[
    HELPER_EXE,
    "plan.json",
    "ready.json",
    "ready.pending",
    "finalizer.ready",
    "finalizer.ready.pending",
    "finalizer.error.txt",
    "finalizer.error.pending",
    "commit.json",
    "commit.pending",
    "lifetime.lock",
];

const FINALIZER_ERROR_MAX_BYTES: u64 = 16 * 1024;
const DIAGNOSTIC_LOG_TAIL_BYTES: u64 = 64 * 1024;
const REPORT_ISSUE_MAX_ITEMS: usize = 8;
const REPORT_ISSUE_MAX_CHARS: usize = 720;
#[cfg(windows)]
const FINALIZER_REPORT_CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(windows)]
const FINALIZER_REPORT_CONFIRM_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(windows)]
const FINALIZER_START_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(windows)]
const FINALIZER_STDIN_BOOTSTRAP: &str = r#"$reader = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), [System.Text.UTF8Encoding]::new($false)); $source = $reader.ReadToEnd(); $reader.Dispose(); & ([ScriptBlock]::Create($source))"#;
const CONNECTION_OWNERSHIP_MAX_BYTES: u64 = 64 * 1024;
const CONNECTION_OWNERSHIP_RELATIVE: &str = ".codex/codex-agent-kit/connection-ownership.json";
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

fn is_legacy_uninstall_report_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix("Codex-Uninstall-Report-")
        .and_then(|value| value.strip_suffix(".json"))
    else {
        return false;
    };
    !stem.is_empty() && name.len() <= 160 && !name.contains('\\') && !name.contains('/')
}

struct UnstartedHelper {
    directory: PathBuf,
    armed: bool,
}
impl Drop for UnstartedHelper {
    fn drop(&mut self) {
        if self.armed && check_ancestors(&self.directory).is_ok() {
            // Exact files from this call only; unknown files prevent rmdir.
            for name in HELPER_FILES {
                let path = self.directory.join(name);
                if check_ancestors(&path).is_ok() {
                    let _ = fs::remove_file(&path);
                }
            }
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupRoots {
    home: PathBuf,
    local: PathBuf,
    roaming: PathBuf,
    temp: PathBuf,
    desktop: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupPlan {
    schema_version: u32,
    nonce: String,
    parent_pid: u32,
    parent_creation_time: u64,
    source_executable: PathBuf,
    executable_sha256: String,
    roots: CleanupRoots,
    purge_user_data: bool,
    #[serde(default)]
    purge_ccswitch_data: bool,
    /// Explicit USB-bundle "彻底清理" authorization.  When set, the
    /// detached helper is allowed to remove this product's shared recovery
    /// journals after the manager has exited; ordinary uninstall preserves
    /// those journals for another installation to recover.
    #[serde(default)]
    force_shared_cleanup: bool,
    remove_usb_media: bool,
    source_volume: Option<VolumeIdentity>,
    source_media: Option<MediaSnapshot>,
    #[serde(default)]
    source_license: Option<crate::app::usb_guard::UsbFingerprint>,
    prior_notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MediaSnapshot {
    checksums_sha256: String,
    manifest_sha256: String,
    #[serde(default)]
    license_sha256: Option<String>,
    #[serde(default)]
    license_backup_sha256: Option<String>,
}

fn optional_media_sha256(bundle: &Path, name: &str) -> Result<Option<String>, String> {
    let path = bundle.join(name);
    check_ancestors(&path)?;
    let Some(metadata) = metadata_optional(&path)? else {
        return Ok(None);
    };
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(format!("安装介质运行期文件类型无效：{}", path.display()));
    }
    sha256(&path).map(Some)
}

fn media_snapshot(bundle: &Path) -> Result<MediaSnapshot, String> {
    let checksums = bundle.join("SHA256SUMS.txt");
    let manifest = bundle.join("usb-manifest.json");
    check_ancestors(&checksums)?;
    check_ancestors(&manifest)?;
    Ok(MediaSnapshot {
        checksums_sha256: sha256(&checksums)?,
        manifest_sha256: sha256(&manifest)?,
        license_sha256: optional_media_sha256(bundle, "usb-license.json")?,
        license_backup_sha256: optional_media_sha256(bundle, "usb-license.json.bak")?,
    })
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VolumeIdentity {
    volume_guid: String,
    serial: u32,
}

#[cfg(windows)]
fn volume_identity(path: &Path) -> Result<VolumeIdentity, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut root = vec![0_u16; 32768];
    if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(format!(
            "无法核验安装介质根目录：{}",
            io::Error::last_os_error()
        ));
    }
    let mut guid = vec![0_u16; 100];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32)
    } == 0
    {
        return Err(format!(
            "无法核验安装介质卷身份：{}",
            io::Error::last_os_error()
        ));
    }
    let mut serial = 0_u32;
    if unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(format!(
            "无法读取安装介质序列号：{}",
            io::Error::last_os_error()
        ));
    }
    Ok(VolumeIdentity {
        volume_guid: String::from_utf16_lossy(
            &guid[..guid
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(guid.len())],
        ),
        serial,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub path: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub schema_version: u32,
    pub status: String,
    /// Explicit lifecycle phase.  Older reports may omit it and deserialize
    /// to the empty string for backwards compatibility.
    #[serde(default)]
    pub phase: String,
    /// Finalizer process exit code when it has terminated.
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Actionable recovery guidance for a partial or failed second stage.
    #[serde(default)]
    pub retry_suggestion: String,
    pub items: Vec<CleanupItem>,
    pub helper_finalization: String,
    pub note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionOwnershipRecord {
    schema_version: u32,
    pack_id: String,
    names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupHandoff {
    pub handoff_id: String,
    pub report_path: String,
    pub message: String,
}

/// Produce a bounded, user-visible explanation for a partial second-stage
/// cleanup. The full JSON report remains the source of truth; this summary is
/// deliberately limited so a locked log or a long path cannot make the native
/// dialog unusable or accidentally echo a whole diagnostic file.
fn report_issue_summary(report: &CleanupReport) -> Option<String> {
    let mut lines = Vec::new();
    for item in &report.items {
        if !matches!(
            item.status.as_str(),
            "failed" | "deferred" | "preserved" | "warning"
        ) {
            continue;
        }
        let detail = item
            .detail
            .chars()
            .map(|character| {
                if character == '\r' || character == '\n' {
                    ' '
                } else {
                    character
                }
            })
            .take(REPORT_ISSUE_MAX_CHARS)
            .collect::<String>();
        let suffix = if item.detail.chars().count() > REPORT_ISSUE_MAX_CHARS {
            "…"
        } else {
            ""
        };
        lines.push(format!(
            "- [{}] {}：{}{}",
            item.status, item.path, detail, suffix
        ));
        if lines.len() >= REPORT_ISSUE_MAX_ITEMS {
            break;
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!("具体原因/保留项：\n{}", lines.join("\n")))
    }
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    false
}

fn metadata_optional(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法核验 {}：{error}", path.display())),
    }
}

/// Reject traversal and ALL existing reparse ancestors, not just the leaf.
fn check_ancestors(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("拒绝非绝对/含跳转路径：{}", path.display()));
    }
    for ancestor in path.ancestors() {
        if let Some(metadata) = metadata_optional(ancestor)? {
            if is_reparse(&metadata) {
                return Err(format!("拒绝链接或重解析点：{}", ancestor.display()));
            }
        }
    }
    Ok(())
}

fn canonical_root(path: &Path) -> Result<PathBuf, String> {
    check_ancestors(path)?;
    let metadata =
        metadata_optional(path)?.ok_or_else(|| format!("必要目录不存在：{}", path.display()))?;
    if !metadata.is_dir() || path.parent().is_none() {
        return Err(format!("无效的清理根目录：{}", path.display()));
    }
    fs::canonicalize(path).map_err(|error| format!("解析目录 {}：{error}", path.display()))
}

fn canonical_or_parent(path: &Path) -> Result<PathBuf, String> {
    check_ancestors(path)?;
    if metadata_optional(path)?.is_some() {
        return fs::canonicalize(path)
            .map_err(|error| format!("解析路径 {}：{error}", path.display()));
    }

    // Optional cleanup targets may have more than one missing parent. Walk
    // back to the nearest existing directory, which is the only location
    // that can be canonicalized without creating the target path. All
    // existing ancestors were already checked above for reparse points and
    // traversal components; preserve the missing suffix literally so this
    // normalization cannot broaden the deletion target.
    let mut missing_components = Vec::new();
    let mut existing = path;
    loop {
        match metadata_optional(existing)? {
            Some(metadata) => {
                if !metadata.is_dir() {
                    return Err(format!(
                        "可选清理目标的已存在祖先不是目录：{}",
                        existing.display()
                    ));
                }
                let mut canonical = fs::canonicalize(existing)
                    .map_err(|error| format!("解析路径 {}：{error}", existing.display()))?;
                for component in missing_components.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            None => {
                let name = existing
                    .file_name()
                    .ok_or_else(|| format!("路径没有文件名：{}", path.display()))?;
                missing_components.push(name.to_os_string());
                existing = existing
                    .parent()
                    .ok_or_else(|| format!("路径没有父目录：{}", path.display()))?;
            }
        }
    }
}

fn same_or_child_path(child: &Path, parent: &Path) -> bool {
    if child.strip_prefix(parent).is_ok() {
        return true;
    }
    #[cfg(windows)]
    {
        child
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(&parent.to_string_lossy().to_ascii_lowercase())
    }
    #[cfg(not(windows))]
    false
}

impl CleanupRoots {
    fn discover() -> Result<Self, String> {
        let base = directories::BaseDirs::new().ok_or("无法读取用户目录")?;
        // The finalizer runs after the manager data root has been removed.
        // Keep reports in a stable temp root that is never part of the
        // product cleanup targets. The old local-app-data candidate created
        // the G22 self-reference failure on older packaged builds.
        let report_candidates = [std::env::temp_dir()
            .join("CodexAppManager")
            .join("uninstall-reports")];
        let mut report_dir = None;
        for candidate in report_candidates {
            if check_ancestors(&candidate).is_err() {
                continue;
            }
            if !candidate.exists() && fs::create_dir_all(&candidate).is_err() {
                continue;
            }
            let Ok(candidate) = canonical_root(&candidate) else {
                continue;
            };
            let probe = candidate.join(format!(".codex-report-probe-{}.tmp", uuid::Uuid::new_v4()));
            if let Ok(file) = OpenOptions::new().write(true).create_new(true).open(&probe) {
                drop(file);
                fs::remove_file(&probe)
                    .map_err(|error| format!("清理报告位置测试文件失败：{error}"))?;
                report_dir = Some(candidate);
                break;
            }
        }
        let roots = Self {
            home: canonical_root(base.home_dir())?,
            local: canonical_root(base.data_local_dir())?,
            roaming: canonical_root(base.data_dir())?,
            temp: canonical_root(&std::env::temp_dir())?,
            // Reports live outside every manager data target and remain
            // available after the helper deletes the application directory.
            desktop: report_dir.ok_or("无法安全创建卸载日志目录")?,
        };
        roots.ensure_report_root_safe()?;
        Ok(roots)
    }

    fn ensure_report_root_safe(&self) -> Result<(), String> {
        let report_root = canonical_root(&self.desktop)?;
        for target in self.targets(true, true)? {
            let target = canonical_or_parent(&target)?;
            if same_or_child_path(&report_root, &target) {
                return Err(format!(
                    "卸载报告目录不能位于待删除目录内：{} -> {}",
                    report_root.display(),
                    target.display()
                ));
            }
        }
        Ok(())
    }

    fn install_staging_targets(&self) -> Vec<PathBuf> {
        vec![
            self.temp.join("CodexDesktopInstaller"),
            self.temp.join("codex-app-manager"),
        ]
    }

    fn legacy_desktop_report_targets(&self) -> Result<Vec<PathBuf>, String> {
        let Some(user) = directories::UserDirs::new() else {
            return Ok(Vec::new());
        };
        let Some(desktop) = user.desktop_dir() else {
            return Ok(Vec::new());
        };
        let Some(home) =
            directories::BaseDirs::new().and_then(|base| canonical_root(base.home_dir()).ok())
        else {
            return Ok(Vec::new());
        };
        // Synthetic tests use isolated roots; never inspect the real Desktop
        // from those fixtures. In production this exact home identity enables
        // removal of only reports emitted by pre-fix builds.
        if home != self.home {
            return Ok(Vec::new());
        }
        let Ok(desktop) = canonical_root(desktop) else {
            return Ok(Vec::new());
        };
        let mut targets = Vec::new();
        for entry in fs::read_dir(desktop).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_legacy_uninstall_report_name(name) {
                continue;
            }
            let path = entry.path();
            let Some(metadata) = metadata_optional(&path)? else {
                continue;
            };
            if metadata.is_file() && !is_reparse(&metadata) {
                targets.push(path);
            }
        }
        Ok(targets)
    }

    fn targets(&self, purge_codex: bool, purge_ccswitch: bool) -> Result<Vec<PathBuf>, String> {
        // Exact product subtrees; never delete the whole `wangnov` publisher folder.
        let mut targets = vec![
            self.local.join("io.github.wangnov.codexappmanager"),
            self.roaming.join("wangnov").join("codexappmanager"),
            self.local.join("wangnov").join("codexappmanager"),
            self.local.join("CodexUsbGuard"),
            self.local.join("CodexUsbAppx"),
        ];
        targets.extend(self.install_staging_targets());
        targets.extend(self.legacy_desktop_report_targets()?);
        // Legacy UAC helpers used decimal PID + Unix-millis names. Restrict
        // extensions and numeric identifier, rather than accepting arbitrary prefix matches.
        for entry in fs::read_dir(&self.temp).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| legacy_helper_owner(name).is_some())
            {
                targets.push(entry.path());
            }
        }
        if purge_codex {
            targets.extend([
                self.home.join(".codex"),
                // The offline capability pack installs Codex Skills in the
                // user-scoped Codex discovery root outside ~/.codex. It is
                // part of the explicit shared-Codex purge, but never part of
                // ordinary cleanup. The USB copy under content/ is not a
                // target and therefore remains untouched.
                self.home.join(".agents/skills"),
                self.local.join("Packages/OpenAI.Codex_2p2nqsd0c76g0"),
                self.local.join("OpenAI/Codex"),
                self.local.join("Codex/Logs"),
                self.roaming.join("OpenAI/Codex"),
                self.roaming.join("Codex"),
            ]);
            // Family identity is the pinned official Codex publisher, not *Codex*.
            targets.push(
                self.local
                    .join("CodexAppxDataBackups")
                    .join("OpenAI.Codex_2p2nqsd0c76g0"),
            );
        }
        if purge_ccswitch {
            targets.extend([
                self.home.join(".cc-switch"),
                self.local.join("com.ccswitch.desktop"),
                self.roaming.join("com.ccswitch.desktop"),
            ]);
        }
        Ok(targets)
    }
}

#[cfg(test)]
fn cleanup_install_staging_for_roots(roots: &CleanupRoots) -> Vec<CleanupItem> {
    clean_targets_with_recheck(&roots.install_staging_targets(), || {})
}

/// Remove only the two exact installer staging directories. This is safe to
/// run as soon as the primary uninstall attempt has returned and is deliberately
/// independent of AppX volume cleanup: a retained/shared volume must not strand
/// an 800 MB local MSIX copy. Recovery journals and package data are not part of
/// this pass.
pub fn cleanup_install_staging_now() -> Result<Vec<CleanupItem>, String> {
    // Do not call `CleanupRoots::discover`: this pass neither writes the final
    // report nor touches profile data, so an unavailable/redirected Desktop or
    // Documents folder must not prevent the large temporary MSIX from being
    // removed.
    let temp = canonical_root(&std::env::temp_dir())?;
    Ok(clean_targets_with_recheck(
        &[
            temp.join("CodexDesktopInstaller"),
            temp.join("codex-app-manager"),
        ],
        || {},
    ))
}

fn is_legacy_uac_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("codex-usb-uac-") else {
        return false;
    };
    let suffixes = [
        ".ps1",
        ".json",
        ".stderr",
        ".stderr.log",
        ".stdout.log",
        ".result.json",
    ];
    suffixes.iter().any(|suffix| {
        rest.strip_suffix(suffix).is_some_and(|id| {
            let Some((pid, timestamp)) = id.split_once('-') else {
                return false;
            };
            !pid.is_empty()
                && !timestamp.is_empty()
                && pid.chars().all(|ch| ch.is_ascii_digit())
                && timestamp.chars().all(|ch| ch.is_ascii_digit())
        })
    })
}

fn legacy_helper_owner(name: &str) -> Option<u32> {
    if is_legacy_uac_file(name) {
        return name
            .strip_prefix("codex-usb-uac-")?
            .split('-')
            .next()?
            .parse()
            .ok();
    }
    let rest = name
        .strip_prefix("codex-win-engine-ps-")?
        .strip_suffix(".ps1")?;
    let (pid, stamp) = rest.split_once('-')?;
    if stamp.is_empty() || !stamp.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

#[cfg(windows)]
fn reject_live_legacy_helper(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let Some(pid) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(legacy_helper_owner)
    else {
        return Ok(());
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if !handle.is_null() {
        let _handle = ProcessHandle(handle);
        return Err(format!(
            "旧临时脚本记录的 PID {pid} 仍存在，不能确认已停止，保留 {}",
            path.display()
        ));
    }
    if unsafe { GetLastError() } != ERROR_INVALID_PARAMETER {
        return Err(format!("无法排除旧临时脚本 PID {pid} 仍运行，保留文件"));
    }
    Ok(())
}
#[cfg(not(windows))]
fn reject_live_legacy_helper(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn directory_has_entries(path: &Path) -> Result<bool, String> {
    check_ancestors(path)?;
    match metadata_optional(path)? {
        None => Ok(false),
        Some(metadata) if metadata.is_dir() => fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .next()
            .transpose()
            .map(|entry| entry.is_some())
            .map_err(|error| error.to_string()),
        Some(_) => Err(format!("预期日志目录却发现文件：{}", path.display())),
    }
}

fn ensure_journals_recovered(roots: &CleanupRoots) -> Result<(), String> {
    // A failed/unknown rollback must retain its only evidence. This applies to
    // backups too. New registry/volume journals live in the same owned directory.
    for dir in [
        roots.local.join("CodexUsbAppx"),
        roots
            .roaming
            .join("wangnov/codexappmanager/data/msix-policy-transactions"),
        roots
            .local
            .join("wangnov/codexappmanager/data/msix-policy-transactions"),
        roots
            .roaming
            .join("wangnov/codexappmanager/data/install-transactions"),
        roots
            .local
            .join("wangnov/codexappmanager/data/install-transactions"),
    ] {
        if directory_has_entries(&dir)? {
            return Err(format!(
                "系统策略/卷回滚记录尚未清空，保留数据并停止二阶段清理：{}",
                dir.display()
            ));
        }
    }
    for data in [
        roots.roaming.join("wangnov/codexappmanager/data"),
        roots.local.join("wangnov/codexappmanager/data"),
    ] {
        for name in [
            "codex-self-update-env.json",
            "codex-self-update-env.json.bak",
        ] {
            let path = data.join(name);
            check_ancestors(&path)?;
            if metadata_optional(&path)?.is_some() {
                return Err(format!(
                    "自更新环境恢复记录尚未清空，保留数据：{}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// Read-only gate used by the bundle uninstaller before it starts the detached
/// finalizer. A different manager installation may own a live transaction in
/// these shared per-user directories; in that case the USB app uninstall can
/// be complete while shared-manager cleanup is explicitly deferred.
pub fn deferred_cleanup_preflight(force_shared_cleanup: bool) -> Result<(), String> {
    let roots = CleanupRoots::discover()?;
    if force_shared_cleanup {
        Ok(())
    } else {
        ensure_journals_recovered(&roots)
    }
}

#[cfg(windows)]
#[path = "uninstall_tree.rs"]
mod handle_tree;

const REVOCATION_MANIFEST_MAX_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct RevocationCleanupRoot {
    bundle_root: std::path::PathBuf,
    fingerprint: crate::app::usb_guard::UsbFingerprint,
    payload_relative: String,
    architecture: String,
    backup_relative: String,
    #[cfg(windows)]
    pinned: handle_tree::PinnedRoot,
}

pub(crate) struct PinnedRevocationEntry {
    #[cfg(windows)]
    inner: handle_tree::PinnedEntry,
}

pub(crate) struct PreparedRevocationTree {
    #[cfg(windows)]
    inner: handle_tree::PreparedDeleteTree,
}

pub(crate) struct PinnedRevocationTransaction {
    relative: String,
    tree: Option<PreparedRevocationTree>,
    sidecar: PinnedRevocationEntry,
    // Retain the exact `.staging` directory object that anchored both the
    // sidecar and transaction entry throughout cleanup, including failures.
    staging_ancestor: PinnedRevocationEntry,
}

impl PinnedRevocationEntry {
    pub(crate) fn duplicate(&self) -> Result<PinnedRevocationEntry, String> {
        #[cfg(windows)]
        {
            self.inner
                .duplicate()
                .map(|inner| PinnedRevocationEntry { inner })
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn is_directory(&self) -> Result<bool, String> {
        #[cfg(windows)]
        {
            self.inner.is_directory()
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn read_bounded(&self, max_bytes: usize) -> Result<Vec<u8>, String> {
        #[cfg(windows)]
        {
            self.inner.read_bounded(max_bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = max_bytes;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn read_child_bounded(
        &self,
        name: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        #[cfg(windows)]
        {
            self.inner.read_child_bounded(name, max_bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = (name, max_bytes);
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn open_child(&self, name: &str) -> Result<Option<PinnedRevocationEntry>, String> {
        #[cfg(windows)]
        {
            self.inner
                .open_child(name)
                .map(|entry| entry.map(|inner| PinnedRevocationEntry { inner }))
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn list_names(&self) -> Result<Vec<String>, String> {
        #[cfg(windows)]
        {
            self.inner
                .list_names()?
                .into_iter()
                .map(|name| {
                    name.into_string()
                        .map_err(|_| "固定事务目录包含非 UTF-8 文件名".to_string())
                })
                .collect()
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn prepare_delete_tree(self) -> Result<PreparedRevocationTree, String> {
        #[cfg(windows)]
        {
            Ok(PreparedRevocationTree {
                inner: self.inner.prepare_delete_tree()?,
            })
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn remove_regular_file(self) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.inner.remove_regular_file()
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }
}

impl PreparedRevocationTree {
    pub(crate) fn read_relative_bounded(
        &self,
        relative: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        #[cfg(windows)]
        {
            self.inner.read_relative_bounded(relative, max_bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = (relative, max_bytes);
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn delete(self) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.inner.delete()
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    #[cfg(test)]
    pub(crate) fn delete_with_test_hook(
        self,
        before_item: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.inner.delete_with_test_hook(before_item)
        }
        #[cfg(not(windows))]
        {
            let _ = before_item;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }
}

impl PinnedRevocationTransaction {
    pub(crate) fn relative(&self) -> &str {
        &self.relative
    }

    pub(crate) fn remove(self, root: &RevocationCleanupRoot) -> Result<(), String> {
        if !self.staging_ancestor.is_directory()? {
            return Err("固定的 .staging ancestor 不再是目录".into());
        }
        if let Some(tree) = self.tree {
            tree.delete()?;
        }
        if root.exists_relative(&self.relative)? {
            return Err("Codex 更新事务目录删除后仍存在；保留 owner sidecar 供重试".into());
        }
        self.sidecar.remove_regular_file()
    }

    #[cfg(test)]
    fn remove_with_test_hooks(
        self,
        root: &RevocationCleanupRoot,
        before_item: impl FnMut(usize) -> Result<(), String>,
        before_sidecar: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if !self.staging_ancestor.is_directory()? {
            return Err("固定的 .staging ancestor 不再是目录".into());
        }
        if let Some(tree) = self.tree {
            tree.delete_with_test_hook(before_item)?;
        }
        if root.exists_relative(&self.relative)? {
            return Err("Codex 更新事务目录删除后仍存在；保留 owner sidecar 供重试".into());
        }
        before_sidecar()?;
        self.sidecar.remove_regular_file()
    }
}

/// Open the Bundle root once for a revocation transaction and bind that handle
/// to the observed volume serial. All subsequent reads, checks, and deletions
/// are relative to this pinned handle.
pub(crate) fn pin_revocation_cleanup_root(
    root: &Path,
    expected: &crate::app::usb_guard::UsbFingerprint,
) -> Result<RevocationCleanupRoot, String> {
    if !root.is_absolute() || fs::canonicalize(root).map_err(|error| error.to_string())? != root {
        return Err("作废清理 Bundle 根目录不是已核验的规范绝对路径".into());
    }
    if expected.anchor_schema != 3 {
        return Err("作废清理只允许 anchorSchema=3 的有效指纹".into());
    }
    #[cfg(windows)]
    {
        let pinned = handle_tree::PinnedRoot::open(root, u64::from(expected.volume_serial))?;
        if pinned.volume_serial() != u64::from(expected.volume_serial) {
            return Err("固定的 Bundle 根句柄卷序列号与已观察 USB 指纹不一致".into());
        }
        let relative_path = bundle_relative_path(root)?;
        if !relative_path.eq_ignore_ascii_case(&expected.bundle_relative_path) {
            return Err("固定的 Bundle 根相对路径与已观察 USB 指纹不一致".into());
        }
        let manifest_bytes =
            pinned.read_relative_bounded("manifest.json", REVOCATION_MANIFEST_MAX_BYTES)?;
        let usb_manifest_bytes =
            pinned.read_relative_bounded("usb-manifest.json", REVOCATION_MANIFEST_MAX_BYTES)?;
        let sums_bytes = pinned.read_relative_bounded("SHA256SUMS.txt", 64 * 1024 * 1024)?;
        let anchor = codex_usb_license::stable_anchor_sha256_from_bytes(
            &manifest_bytes,
            &usb_manifest_bytes,
            &sums_bytes,
        )
        .map_err(|error| format!("固定 Bundle 根句柄锚点校验失败：{error}"))?;
        if !anchor.eq_ignore_ascii_case(&expected.anchor_sha256) {
            return Err("固定 Bundle 根句柄锚点与已观察 USB 指纹不一致".into());
        }
        let root_manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("固定根清单格式无效：{error}"))?;
        let usb_manifest: serde_json::Value = serde_json::from_slice(&usb_manifest_bytes)
            .map_err(|error| format!("固定 USB 清单格式无效：{error}"))?;
        validate_pinned_revocation_policy(&root_manifest, &usb_manifest)?;
        let (payload_relative, architecture) = revocation_manifest_contract(&manifest_bytes)?;
        let payload_parent = payload_relative
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .ok_or_else(|| "MSIX 载荷必须位于受管 payload 目录".to_string())?;
        let backup_relative = format!("{payload_parent}/_backup");
        Ok(RevocationCleanupRoot {
            bundle_root: root.to_path_buf(),
            fingerprint: expected.clone(),
            payload_relative,
            architecture,
            backup_relative,
            pinned,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = expected;
        Err("远程作废清理只允许在 Windows 受管环境中执行".into())
    }
}

impl RevocationCleanupRoot {
    pub(crate) fn validate_identity(
        &self,
        root: &Path,
        fingerprint: &crate::app::usb_guard::UsbFingerprint,
    ) -> Result<(), String> {
        if fs::canonicalize(root).map_err(|error| error.to_string())? != self.bundle_root
            || !codex_usb_license::fingerprints_match(&self.fingerprint, fingerprint)
        {
            return Err("事务 owner marker 检查与固定 Bundle 根身份不一致".into());
        }
        #[cfg(windows)]
        if self.pinned.volume_serial() != u64::from(fingerprint.volume_serial) {
            return Err("固定 Bundle 根句柄卷序列号已变化".into());
        }
        Ok(())
    }

    pub(crate) fn read_relative_bounded(
        &self,
        relative: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        #[cfg(windows)]
        {
            match self.pinned.read_relative_bounded(relative, max_bytes) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => Ok(None),
                Err(error) => Err(error),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (relative, max_bytes);
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn list_relative_names(
        &self,
        relative: &str,
    ) -> Result<Option<Vec<String>>, String> {
        #[cfg(windows)]
        {
            self.pinned
                .list_relative_names(relative)?
                .map(|names| {
                    names
                        .into_iter()
                        .map(|name| {
                            name.into_string()
                                .map_err(|_| "固定暂存目录包含非 UTF-8 文件名".to_string())
                        })
                        .collect()
                })
                .transpose()
        }
        #[cfg(not(windows))]
        {
            let _ = relative;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn open_pinned_entry(
        &self,
        relative: &str,
    ) -> Result<Option<PinnedRevocationEntry>, String> {
        #[cfg(windows)]
        {
            self.pinned
                .open_pinned_entry(relative)
                .map(|entry| entry.map(|inner| PinnedRevocationEntry { inner }))
        }
        #[cfg(not(windows))]
        {
            let _ = relative;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn open_entry_child(
        &self,
        parent: &PinnedRevocationEntry,
        name: &str,
    ) -> Result<Option<PinnedRevocationEntry>, String> {
        parent.open_child(name)
    }

    pub(crate) fn exists_relative(&self, relative: &str) -> Result<bool, String> {
        #[cfg(windows)]
        {
            self.pinned.exists_relative(relative)
        }
        #[cfg(not(windows))]
        {
            let _ = relative;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }

    pub(crate) fn prepare_pinned_transaction(
        relative: String,
        tree: Option<PreparedRevocationTree>,
        sidecar: PinnedRevocationEntry,
        staging_ancestor: PinnedRevocationEntry,
    ) -> PinnedRevocationTransaction {
        PinnedRevocationTransaction {
            relative,
            tree,
            sidecar,
            staging_ancestor,
        }
    }

    pub(crate) fn payload_relative(&self) -> &str {
        &self.payload_relative
    }

    pub(crate) fn backup_relative(&self) -> &str {
        &self.backup_relative
    }

    pub(crate) fn validate_backup(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.pinned
                .validate_backup(&self.backup_relative, &self.architecture)
        }
        #[cfg(not(windows))]
        {
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }
}

#[cfg(windows)]
fn bundle_relative_path(root: &Path) -> Result<String, String> {
    use std::path::Component;
    let mut components = root.components();
    if !matches!(components.next(), Some(Component::Prefix(_)))
        || components.next() != Some(Component::RootDir)
    {
        return Err("作废清理 Bundle 根路径不是本地卷绝对路径".into());
    }
    let parts = components
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "Bundle 相对路径包含非 UTF-8 名称".to_string()),
            _ => Err("Bundle 相对路径包含跳转组件".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err("作废清理 Bundle 根目录不能是卷根".into());
    }
    Ok(parts.join("/"))
}

/// Strictly preflight one exact target in the signed USB revocation cleanup
/// allowlist. This is separate from the ordinary uninstall workflow.
pub(crate) fn check_revocation_target(
    root: &RevocationCleanupRoot,
    relative: &str,
) -> Result<(), String> {
    authorized_revocation_target(&root.payload_relative, relative)?;
    #[cfg(windows)]
    {
        root.pinned.check_relative(relative)
    }
    #[cfg(not(windows))]
    {
        let _ = relative;
        Err("远程作废清理只允许在 Windows 受管环境中执行".into())
    }
}

/// Remove only an exact target authorized by the revocation cleanup policy.
/// Windows recursively deletes retained handle-tree entries after full-tree
/// preflight, so path substitution cannot redirect deletion outside the tree.
pub(crate) fn remove_revocation_target(
    root: &RevocationCleanupRoot,
    relative: &str,
) -> Result<(), String> {
    authorized_revocation_target(&root.payload_relative, relative)?;
    #[cfg(windows)]
    {
        root.pinned.remove_relative(relative)
    }
    #[cfg(not(windows))]
    {
        Err("远程作废清理只允许在 Windows 受管环境中执行".to_string())
    }
}

/// Remove only the exact payload named by the currently anchored manifest,
/// after hashing and deleting it through the same single-link file handle.
/// This is reserved for recovery rollback of a payload staged by this manager.
pub(crate) fn remove_revocation_staged_payload_if_sha256(
    root: &Path,
    fingerprint: &crate::app::usb_guard::UsbFingerprint,
    relative: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || !parts[0].eq_ignore_ascii_case("payload")
        || !parts[1].starts_with("OpenAI.Codex_")
        || !parts[1].ends_with(".msix")
        || parts[1].contains(['\\', ':', '\0'])
    {
        return Err("恢复载荷路径不符合固定 MSIX 布局".into());
    }
    #[cfg(test)]
    {
        let _ = fingerprint;
        return remove_revocation_staged_payload_test(root, relative, expected_sha256);
    }
    #[cfg(not(test))]
    {
        if !root.is_absolute() || fs::canonicalize(root).map_err(|error| error.to_string())? != root
        {
            return Err("恢复载荷根目录不是已核验的规范绝对路径".into());
        }
        if fingerprint.anchor_schema != 3 {
            return Err("恢复载荷删除只允许 anchorSchema=3 的 USB 指纹".into());
        }
        #[cfg(windows)]
        {
            let relative_root = bundle_relative_path(root)?;
            if !relative_root.eq_ignore_ascii_case(&fingerprint.bundle_relative_path) {
                return Err("恢复载荷根目录与事务记录的 USB 相对路径不一致".into());
            }
            let pinned = handle_tree::PinnedRoot::open(root, u64::from(fingerprint.volume_serial))?;
            let manifest_bytes =
                pinned.read_relative_bounded("manifest.json", REVOCATION_MANIFEST_MAX_BYTES)?;
            let usb_manifest_bytes =
                pinned.read_relative_bounded("usb-manifest.json", REVOCATION_MANIFEST_MAX_BYTES)?;
            let sums_bytes = pinned.read_relative_bounded("SHA256SUMS.txt", 64 * 1024 * 1024)?;
            let anchor = codex_usb_license::stable_anchor_sha256_from_bytes(
                &manifest_bytes,
                &usb_manifest_bytes,
                &sums_bytes,
            )
            .map_err(|error| format!("恢复载荷固定根锚点校验失败：{error}"))?;
            if !anchor.eq_ignore_ascii_case(&fingerprint.anchor_sha256) {
                return Err("恢复载荷固定根锚点与事务记录不一致".into());
            }
            let usb_manifest: serde_json::Value = serde_json::from_slice(&usb_manifest_bytes)
                .map_err(|error| format!("恢复载荷 USB 清单格式无效：{error}"))?;
            let root_manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
                .map_err(|error| format!("恢复载荷根清单格式无效：{error}"))?;
            validate_pinned_revocation_policy(&root_manifest, &usb_manifest)?;
            let (declared_payload, _) = revocation_manifest_contract(&manifest_bytes)?;
            if declared_payload != relative {
                return Err("恢复载荷路径与固定根清单声明不一致".into());
            }
            pinned.remove_regular_file_relative_if_sha256(relative, expected_sha256)
        }
        #[cfg(not(windows))]
        {
            let _ = expected_sha256;
            Err("远程作废清理只允许在 Windows 受管环境中执行".into())
        }
    }
}

#[cfg(test)]
fn remove_revocation_staged_payload_test(
    root: &Path,
    relative: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        let pinned = handle_tree::PinnedRoot::open_for_test(&canonical_root)?;
        pinned.remove_regular_file_relative_if_sha256(relative, expected_sha256)
    }
    #[cfg(not(windows))]
    {
        let path = canonical_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("读取测试恢复载荷失败：{error}")),
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() =>
            {
                return Err("测试恢复载荷不是普通文件".into())
            }
            Ok(_) => {}
        }
        let actual = sha256(&path)?;
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Err("测试恢复载荷 SHA-256 不匹配；保留目标".into());
        }
        fs::remove_file(path).map_err(|error| format!("删除测试恢复载荷失败：{error}"))
    }
}

pub(crate) fn exists_revocation_target(
    root: &RevocationCleanupRoot,
    relative: &str,
) -> Result<bool, String> {
    authorized_revocation_target(&root.payload_relative, relative)?;
    #[cfg(windows)]
    {
        root.pinned.exists_relative(relative)
    }
    #[cfg(not(windows))]
    {
        let _ = relative;
        Err("远程作废清理只允许在 Windows 受管环境中执行".into())
    }
}

pub(crate) fn validate_revocation_backup(root: &RevocationCleanupRoot) -> Result<(), String> {
    authorized_revocation_target(&root.payload_relative, &root.backup_relative)?;
    root.validate_backup()
}

pub(crate) fn authorized_revocation_target(
    payload_relative: &str,
    relative: &str,
) -> Result<(), String> {
    let normalized = relative.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("作废清理目标不是安全相对路径".into());
    }
    let lower = normalized.to_ascii_lowercase();
    let payload_parent = payload_relative
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| "MSIX 载荷必须位于受管 payload 目录".to_string())?;
    let backup_relative = format!("{payload_parent}/_backup");
    let authorized = lower == "apps/codex"
        || lower == "apps/.codex-app-manager-staging"
        || lower == payload_relative.to_ascii_lowercase()
        || lower == backup_relative.to_ascii_lowercase()
        || lower.starts_with(".staging/codex-update-")
            && lower
                .strip_prefix(".staging/codex-update-")
                .is_some_and(valid_update_transaction_name)
        || lower
            .strip_prefix("codexusbstate/")
            .is_some_and(crate::app::usb_revocation::is_atomic_state_tmp_name);
    if !authorized {
        return Err("拒绝清理不在远程作废固定白名单内的目标".into());
    }
    Ok(())
}

fn valid_update_transaction_name(suffix: &str) -> bool {
    !suffix.is_empty()
        && suffix.len() <= 128
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn revocation_manifest_contract(bytes: &[u8]) -> Result<(String, String), String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("root manifest 格式无效：{error}"))?;
    let raw = value
        .pointer("/payload/path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "root manifest 缺少 MSIX 载荷路径".to_string())?;
    let relative = raw.trim().replace('\\', "/");
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || !parts[0].eq_ignore_ascii_case("payload")
        || !parts[1].starts_with("OpenAI.Codex_")
        || !parts[1].ends_with(".msix")
        || parts[1].contains(':')
        || parts[1].contains('\0')
    {
        return Err("root manifest MSIX 路径不符合固定载荷布局".into());
    }
    let architecture = value
        .get("architecture")
        .and_then(serde_json::Value::as_str)
        .filter(|value| matches!(*value, "x64" | "arm64"))
        .ok_or_else(|| "root manifest 架构无效".to_string())?
        .to_string();
    Ok((relative, architecture))
}

fn validate_pinned_revocation_policy(
    root_manifest: &serde_json::Value,
    usb_manifest: &serde_json::Value,
) -> Result<(), String> {
    let root_schema = root_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "固定根清单版本无效".to_string())?;
    let usb_schema = usb_manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "固定 USB 清单版本无效".to_string())?;
    if root_schema != 3 || usb_schema != root_schema {
        return Err("作废清理要求锚定且版本一致的 schema 3 Bundle 清单".into());
    }
    if root_manifest
        .pointer("/revocationPolicy/registrationRequired")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err("固定根清单未锚定 registrationRequired=true".into());
    }
    Ok(())
}

#[cfg(all(windows, test))]
fn pin_directory(path: &Path) -> Result<handle_tree::Entry, String> {
    handle_tree::Entry::open(path, false)
}

#[cfg(all(windows, test))]
fn clear_owned_readonly_attribute(path: &Path) -> Result<(), String> {
    handle_tree::clear_readonly(path)
}

fn check_tree(path: &Path) -> Result<(), String> {
    let Some(metadata) = metadata_optional(path)? else {
        return Ok(());
    };
    if is_reparse(&metadata) {
        return Err(format!("拒绝清理含链接的目录树：{}", path.display()));
    }
    #[cfg(windows)]
    {
        handle_tree::check(path)
    }
    #[cfg(not(windows))]
    {
        if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
                check_tree(&entry.map_err(|error| error.to_string())?.path())?;
            }
        } else if !metadata.is_file() {
            return Err(format!("拒绝未知文件类型：{}", path.display()));
        }
        Ok(())
    }
}

fn remove_tree_no_follow(path: &Path) -> Result<(), String> {
    let Some(metadata) = metadata_optional(path)? else {
        return Ok(());
    };
    if is_reparse(&metadata) {
        return Err(format!("拒绝删除链接：{}", path.display()));
    }
    #[cfg(windows)]
    {
        handle_tree::remove(path)
    }
    #[cfg(not(windows))]
    {
        if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
                remove_tree_no_follow(&entry.map_err(|error| error.to_string())?.path())?;
            }
            fs::remove_dir(path).map_err(|error| error.to_string())
        } else if metadata.is_file() {
            fs::remove_file(path).map_err(|error| error.to_string())
        } else {
            Err(format!("拒绝未知文件类型：{}", path.display()))
        }
    }
}

fn verify_absent(path: &Path) -> Result<(), String> {
    check_ancestors(path)?;
    if metadata_optional(path)?.is_some() {
        return Err("目录/文件仍存在或在退出时被重新创建".into());
    }
    Ok(())
}

fn clean_targets_with_recheck(
    targets: &[PathBuf],
    after_first_pass: impl FnOnce(),
) -> Vec<CleanupItem> {
    let mut results = targets
        .iter()
        .map(|path| clean_target(path))
        .collect::<Vec<_>>();
    after_first_pass();
    for (index, target) in targets.iter().enumerate() {
        if let Err(error) = verify_absent(target) {
            let mut retry = clean_target(target);
            retry.detail = format!("最终复核发现残留（{error}），再次清理：{}", retry.detail);
            results[index] = retry;
        }
    }
    for (index, target) in targets.iter().enumerate() {
        if let Err(error) = verify_absent(target) {
            results[index].status = "failed".into();
            results[index].detail = format!("最终复核未通过：{error}");
        }
    }
    results
}

fn clean_target(path: &Path) -> CleanupItem {
    let mut last_error = String::new();
    for attempt in 0..RETRIES {
        let result = (|| {
            check_ancestors(path)?;
            if metadata_optional(path)?.is_none() {
                return Ok("absent");
            }
            reject_live_legacy_helper(path)?;
            check_tree(path)?;
            remove_tree_no_follow(path)?;
            if metadata_optional(path)?.is_some() {
                return Err("删除后仍存在".to_string());
            }
            Ok("removed")
        })();
        match result {
            Ok(status) => {
                return CleanupItem {
                    path: path.display().to_string(),
                    status: status.into(),
                    detail: "已核验不存在".into(),
                }
            }
            Err(error) => last_error = error,
        }
        if attempt + 1 < RETRIES {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    CleanupItem {
        path: path.display().to_string(),
        status: "failed".into(),
        detail: last_error,
    }
}

fn clean_empty_container(path: &Path) -> CleanupItem {
    let result = (|| {
        check_ancestors(path)?;
        let Some(metadata) = metadata_optional(path)? else {
            return Ok(("absent", "空父目录已不存在".to_string()));
        };
        if !metadata.is_dir() || is_reparse(&metadata) {
            return Err("预期普通空目录却发现其他文件类型".to_string());
        }
        if directory_has_entries(path)? {
            return Ok((
                "preserved",
                "目录仍含非本次清理目标，未递归删除".to_string(),
            ));
        }
        fs::remove_dir(path).map_err(|error| error.to_string())?;
        verify_absent(path)?;
        Ok(("removed", "空父目录已非递归删除并复核".to_string()))
    })();
    match result {
        Ok((status, detail)) => CleanupItem {
            path: path.display().to_string(),
            status: status.into(),
            detail,
        },
        Err(error) => CleanupItem {
            path: path.display().to_string(),
            status: "failed".into(),
            detail: format!("空父目录清理失败：{error}"),
        },
    }
}

#[cfg(windows)]
fn original_parent_is_dead(plan: &CleanupPlan) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            plan.parent_pid,
        )
    };
    if handle.is_null() {
        return if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
            Ok(true)
        } else {
            Err("无法核验旧清理任务父进程是否已退出".into())
        };
    }
    let owner = ProcessHandle(handle);
    Ok(process_creation_time(owner.0)? != plan.parent_creation_time
        || unsafe { WaitForSingleObject(owner.0, 0) } == WAIT_OBJECT_0)
}

#[cfg(windows)]
fn validate_stale_helper(directory: &Path, roots: &CleanupRoots) -> Result<(), String> {
    check_ancestors(directory)?;
    if directory.parent() != Some(roots.temp.as_path()) {
        return Err("旧组件不在当前临时目录直接子目录".into());
    }
    let nonce = directory
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(HELPER_PREFIX))
        .ok_or("旧组件目录名无效")?;
    if uuid::Uuid::parse_str(nonce)
        .map_err(|_| "旧组件 UUID 无效")?
        .to_string()
        != nonce
    {
        return Err("旧组件 UUID 不规范".into());
    }
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        check_ancestors(&path)?;
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|name| HELPER_FILES.contains(&name))
            || !metadata_optional(&path)?.ok_or("旧组件条目消失")?.is_file()
        {
            return Err(format!(
                "旧组件含未知条目，保留以免误删：{}",
                path.display()
            ));
        }
    }
    let plan_path = directory.join("plan.json");
    if metadata_optional(&plan_path)?
        .ok_or("旧组件缺少身份计划")?
        .len()
        > 65536
    {
        return Err("旧组件身份计划过大".into());
    }
    let plan: CleanupPlan =
        serde_json::from_slice(&fs::read(&plan_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("旧组件身份计划无效：{error}"))?;
    if !matches!(plan.schema_version, 1..=3) || plan.nonce != nonce || &plan.roots != roots {
        return Err("旧组件身份/用户目录不匹配".into());
    }
    if !original_parent_is_dead(&plan)? {
        return Err("旧清理任务的原父进程仍运行，保留组件".into());
    }
    let executable = directory.join(HELPER_EXE);
    if metadata_optional(&executable)?.is_some() && sha256(&executable)? != plan.executable_sha256 {
        return Err("旧组件可执行文件哈希不匹配".into());
    }
    Ok(())
}

#[cfg(windows)]
fn clean_stale_helpers(roots: &CleanupRoots, current: &Path) -> Result<Vec<CleanupItem>, String> {
    let mut results = Vec::new();
    for entry in fs::read_dir(&roots.temp).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path == current {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(nonce) = name.strip_prefix(HELPER_PREFIX) else {
            continue;
        };
        if uuid::Uuid::parse_str(nonce).is_err() {
            continue;
        }
        let cleanup = (|| {
            // A previous finalizer can remove every known file and still lose
            // the last directory unlink to a short-lived scanner handle. An
            // exact empty UUID directory carries no data or identity to retain.
            if directory_has_entries(&path)? {
                validate_stale_helper(&path, roots)?;
            }
            // Executable FIRST: Windows refuses unlinking a live mapped image,
            // leaving its plan intact. Never terminate any named process.
            for name in HELPER_FILES {
                let item = clean_target(&path.join(name));
                if item.status == "failed" {
                    return Err(item.detail);
                }
            }
            check_ancestors(&path)?;
            fs::remove_dir(&path).map_err(|error| error.to_string())?;
            Ok(())
        })();
        results.push(CleanupItem {
            path: path.display().to_string(),
            status: if cleanup.is_ok() { "removed" } else { "failed" }.into(),
            detail: cleanup
                .err()
                .unwrap_or_else(|| "旧临时清理组件已核验并移除".into()),
        });
    }
    Ok(results)
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn report_path(roots: &CleanupRoots, nonce: &str) -> PathBuf {
    roots
        .desktop
        .join(format!("Codex-Uninstall-Report-{nonce}.json"))
}

fn write_report(path: &Path, report: &CleanupReport) -> Result<(), String> {
    check_ancestors(path)?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn capture_latest_log_tail(roots: &CleanupRoots) -> CleanupItem {
    let log_dir = roots
        .local
        .join("io.github.wangnov.codexappmanager")
        .join("logs");
    let result = (|| {
        check_ancestors(&log_dir)?;
        let Some(metadata) = metadata_optional(&log_dir)? else {
            return Ok(("absent", "卸载前未发现管理器日志".to_string()));
        };
        if !metadata.is_dir() || is_reparse(&metadata) {
            return Err("管理器日志位置不是可核验的普通目录".to_string());
        }
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&log_dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !name.starts_with("codex-app-manager") || !name.contains(".log") {
                continue;
            }
            let path = entry.path();
            check_ancestors(&path)?;
            let metadata = metadata_optional(&path)?.ok_or("日志条目在读取时消失")?;
            if !metadata.is_file() || is_reparse(&metadata) {
                return Err(format!("拒绝读取非普通日志文件：{}", path.display()));
            }
            candidates.push((
                metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                path,
                metadata.len(),
            ));
        }
        let Some((_, path, length)) = candidates
            .into_iter()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        else {
            return Ok(("absent", "卸载前未发现管理器日志文件".to_string()));
        };
        let mut file = File::open(&path).map_err(|error| error.to_string())?;
        let start = length.saturating_sub(DIAGNOSTIC_LOG_TAIL_BYTES);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        let mut bytes =
            Vec::with_capacity((length - start).min(DIAGNOSTIC_LOG_TAIL_BYTES) as usize);
        file.take(DIAGNOSTIC_LOG_TAIL_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        Ok((
            "archived",
            format!(
                "删除管理器缓存前已把最新日志末尾 {} 字节嵌入本报告；source={}\n{}",
                bytes.len(),
                path.display(),
                String::from_utf8_lossy(&bytes)
            ),
        ))
    })();
    match result {
        Ok((status, detail)) => CleanupItem {
            path: log_dir.display().to_string(),
            status: status.into(),
            detail,
        },
        Err(error) => CleanupItem {
            path: log_dir.display().to_string(),
            status: "warning".into(),
            detail: format!("卸载继续，但无法把原日志尾部嵌入最终报告：{error}"),
        },
    }
}

#[cfg(windows)]
struct ProcessHandle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
fn process_creation_time(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<u64, String> {
    use windows_sys::Win32::Foundation::FILETIME;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe {
        windows_sys::Win32::System::Threading::GetProcessTimes(
            handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(format!(
            "无法读取进程启动时间：{}",
            io::Error::last_os_error()
        ));
    }
    Ok(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
}

#[cfg(windows)]
fn open_parent(plan: &CleanupPlan) -> Result<ProcessHandle, String> {
    use windows_sys::Win32::System::Threading::*;
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            plan.parent_pid,
        )
    };
    if handle.is_null() {
        return Err(format!(
            "无法锁定卸载器进程：{}",
            io::Error::last_os_error()
        ));
    }
    let parent = ProcessHandle(handle);
    if process_creation_time(handle)? != plan.parent_creation_time {
        return Err("卸载器 PID 已复用，拒绝执行".into());
    }
    let mut image = vec![0_u16; 32768];
    let mut length = image.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut length) } == 0 {
        return Err("无法核验卸载器映像".into());
    }
    let image = PathBuf::from(String::from_utf16_lossy(&image[..length as usize]));
    if fs::canonicalize(image).map_err(|error| error.to_string())? != plan.source_executable {
        return Err("卸载器来源映像不匹配".into());
    }
    Ok(parent)
}

#[cfg(windows)]
fn acquire_cleanup_mutex() -> Result<ProcessHandle, String> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = "Local\\CodexUsbInstaller.FullAdmin.v1"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        SetLastError(0);
    }
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    let error = unsafe { GetLastError() };
    if handle.is_null() {
        return Err(format!("无法锁定清理会话：{error}"));
    }
    // The parent's instance object MUST remain alive across its exit. This is
    // lifetime retention, not mutex ownership. Existing object is expected;
    // any newly opened normal manager sees already-exists and stops startup.
    Ok(ProcessHandle(handle))
}

#[cfg(windows)]
fn create_lifetime_lock(helper_dir: &Path, nonce: &str) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    let path = helper_dir.join("lifetime.lock");
    check_ancestors(&path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        // The finalizer learns that every helper write is complete by waiting
        // until this exact no-share handle closes. This avoids relying on
        // .NET Process.StartTime precision or reopening an already-dead PID.
        .share_mode(0)
        .open(&path)
        .map_err(|error| format!("创建清理生命周期锁失败：{error}"))?;
    file.write_all(nonce.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("写入清理生命周期锁失败：{error}"))?;
    Ok(file)
}

fn read_finalizer_error(helper_dir: &Path) -> Result<Option<String>, String> {
    let path = helper_dir.join("finalizer.error.txt");
    check_ancestors(&path)?;
    let Some(metadata) = metadata_optional(&path)? else {
        return Ok(None);
    };
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err("收尾错误通道不是普通文件".into());
    }
    if metadata.len() > FINALIZER_ERROR_MAX_BYTES {
        return Err("收尾错误通道超过大小限制".into());
    }
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let text = text.trim_start_matches('\u{feff}').trim();
    Ok((!text.is_empty()).then(|| text.to_string()))
}

/// Read only the installer-owned environment-variable names before `.codex`
/// is removed.  The record never contains values, and an invalid/mismatched
/// record is rejected so an arbitrary file cannot turn this into a generic
/// environment-variable wipe.
fn read_owned_connection_env_names(roots: &CleanupRoots) -> Result<Option<Vec<String>>, String> {
    let path = roots.home.join(CONNECTION_OWNERSHIP_RELATIVE);
    check_ancestors(&path)?;
    let Some(metadata) = metadata_optional(&path)? else {
        return Ok(None);
    };
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err("连接所有权记录不是普通文件".into());
    }
    if metadata.len() > CONNECTION_OWNERSHIP_MAX_BYTES {
        return Err("连接所有权记录超过大小限制".into());
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    let value = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let record: ConnectionOwnershipRecord = serde_json::from_slice(value)
        .map_err(|error| format!("连接所有权记录 JSON 无效：{error}"))?;
    if record.schema_version != 1 || record.pack_id != "codex-starter" {
        return Err("连接所有权记录身份不匹配；为避免误删，未清理环境变量".into());
    }
    let mut names = Vec::new();
    for name in record.names {
        if !OWNED_CONNECTION_ENV_NAMES.contains(&name.as_str()) {
            return Err(format!(
                "连接所有权记录包含未声明的环境变量 {name}；为避免误删，未清理环境变量"
            ));
        }
        if !names.iter().any(|existing| existing == &name) {
            names.push(name);
        }
    }
    Ok(Some(names))
}

#[cfg(windows)]
fn cleanup_owned_connection_environment(names: &[String]) -> CleanupItem {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    const REPORT_PATH: &str = "HKCU\\Environment (Codex capability pack)";
    if names.is_empty() {
        return CleanupItem {
            path: REPORT_PATH.into(),
            status: "absent".into(),
            detail: "能力包没有登记需要清理的当前用户环境变量".into(),
        };
    }
    let subkey = "Environment\0".encode_utf16().collect::<Vec<_>>();
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
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return CleanupItem {
            path: REPORT_PATH.into(),
            status: "absent".into(),
            detail: "当前用户环境变量注册表项不存在".into(),
        };
    }
    if status != ERROR_SUCCESS || handle.is_null() {
        return CleanupItem {
            path: REPORT_PATH.into(),
            status: "failed".into(),
            detail: format!("打开当前用户环境变量注册表项失败（Win32 {status}）"),
        };
    }

    struct Key(windows_sys::Win32::System::Registry::HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    let _key = Key(handle);
    let mut removed = 0usize;
    let mut absent = 0usize;
    let mut failures = Vec::new();
    for name in names {
        let name_wide = name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let status = unsafe { RegDeleteValueW(handle, name_wide.as_ptr()) };
        match status {
            ERROR_SUCCESS => removed += 1,
            ERROR_FILE_NOT_FOUND => absent += 1,
            other => failures.push(format!("{name}（Win32 {other}）")),
        }
    }
    if !failures.is_empty() {
        return CleanupItem {
            path: REPORT_PATH.into(),
            status: "failed".into(),
            detail: format!(
                "已删除 {removed} 个、原本不存在 {absent} 个；删除失败：{}",
                failures.join("、")
            ),
        };
    }
    CleanupItem {
        path: REPORT_PATH.into(),
        status: if removed == 0 { "absent" } else { "removed" }.into(),
        detail: format!("已删除 {removed} 个能力包登记的当前用户环境变量；原本不存在 {absent} 个"),
    }
}

#[cfg(not(windows))]
fn cleanup_owned_connection_environment(_names: &[String]) -> CleanupItem {
    CleanupItem {
        path: "当前用户环境变量（Codex capability pack）".into(),
        status: "unsupported".into(),
        detail: "当前平台不支持 Windows 能力包环境变量清理".into(),
    }
}

/// The PowerShell finalizer publishes `finalizer.ready` only as a short-lived
/// observation marker.  It can legitimately create the marker, release the
/// lifetime lock, remove the helper directory, persist the final report, and
/// exit before this process observes the marker.  In that race the durable
/// report is the authoritative completion signal.
fn finalizer_report_completed(report: &Path) -> Result<bool, String> {
    check_ancestors(report)?;
    let Some(metadata) = metadata_optional(report)? else {
        return Ok(false);
    };
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err("卸载报告不是普通文件".into());
    }
    if metadata.len() > 1024 * 1024 {
        return Err("卸载报告超过大小限制".into());
    }
    let bytes = fs::read(report).map_err(|error| error.to_string())?;
    let value = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let report: CleanupReport =
        serde_json::from_slice(value).map_err(|error| format!("卸载报告 JSON 无效：{error}"))?;
    Ok(report.helper_finalization == "removed"
        && matches!(report.status.as_str(), "completed" | "partialFailure")
        && (report.phase.is_empty() || report.phase == "completed")
        && report.exit_code.is_none_or(|code| code == 0))
}

#[cfg(windows)]
/// Repair only the narrow state that can be produced after a successful
/// finalizer process has already removed its helper directory but the durable
/// JSON report was left at its pre-finalization state.  The helper directory
/// absence is the safety gate: without it, a report must never be upgraded to
/// `removed` merely because the child exited with code 0.
fn repair_successful_finalizer_report(
    helper_dir: &Path,
    report: &Path,
    roots: &CleanupRoots,
    nonce: &str,
) -> Result<bool, String> {
    let parsed_nonce = uuid::Uuid::parse_str(nonce)
        .map_err(|_| "最终收尾批次 UUID 无效，拒绝修复卸载报告".to_string())?;
    if parsed_nonce.to_string() != nonce
        || helper_dir.parent() != Some(roots.temp.as_path())
        || helper_dir.file_name().and_then(|name| name.to_str())
            != Some(&format!("{HELPER_PREFIX}{nonce}"))
        || report != report_path(roots, nonce)
    {
        return Err("最终收尾 helper/report 身份不匹配，拒绝修复卸载报告".into());
    }
    check_ancestors(helper_dir)?;
    if metadata_optional(helper_dir)?.is_some() {
        return Ok(false);
    }
    check_ancestors(report)?;
    let Some(metadata) = metadata_optional(report)? else {
        return Ok(false);
    };
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err("卸载报告不是普通文件".into());
    }
    if metadata.len() > 1024 * 1024 {
        return Err("卸载报告超过大小限制".into());
    }
    let bytes = fs::read(report).map_err(|error| error.to_string())?;
    let value = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let mut current: CleanupReport =
        serde_json::from_slice(value).map_err(|error| format!("卸载报告 JSON 无效：{error}"))?;

    // A terminal report may have become visible just after the confirmer's
    // last poll.  Accept that durable proof before inspecting the pre-state
    // fields used only for repair eligibility.
    if finalizer_report_completed(report)? {
        return Ok(true);
    }

    // Do not repair an explicit finalizer failure, a failed phase, or an
    // unrelated/stale report.  Only the handoff states that the real finalizer
    // writes before its terminal report are eligible for this recovery.
    if current.schema_version != 1
        || current.exit_code.is_some()
        || current.helper_finalization != "pending"
        || !matches!(
            current.status.as_str(),
            "finalizing" | "completed" | "partialFailure"
        )
        || !matches!(current.phase.as_str(), "" | "finalizing" | "completed")
    {
        return Ok(false);
    }
    if current.status != "partialFailure"
        && current
            .items
            .iter()
            .any(|item| matches!(item.status.as_str(), "failed" | "deferred"))
    {
        return Ok(false);
    }
    current.helper_finalization = "removed".into();
    current.phase = "completed".into();
    current.exit_code = Some(0);
    current.retry_suggestion.clear();
    if current.status == "finalizing" {
        current.status = "completed".into();
    }
    write_report_atomically(report, &current)?;
    finalizer_report_completed(report)
}

#[cfg(windows)]
fn write_report_atomically(path: &Path, report: &CleanupReport) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    check_ancestors(path)?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("卸载报告文件名无效")?;
    let pending = path.with_file_name(format!(".{name}.repair.pending"));
    check_ancestors(&pending)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|error| format!("创建卸载报告修复暂存失败：{error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&pending);
        return Err(format!("写入卸载报告修复暂存失败：{error}"));
    }
    drop(file);
    let from = pending
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        let error = io::Error::last_os_error();
        let _ = fs::remove_file(&pending);
        return Err(format!("原子替换卸载报告失败：{error}"));
    }
    Ok(())
}

#[cfg(windows)]
fn finalizer_completion_ready(report: &Path, helper_dir: Option<&Path>) -> Result<bool, String> {
    if !finalizer_report_completed(report)? {
        return Ok(false);
    }
    if let Some(helper_dir) = helper_dir {
        check_ancestors(helper_dir)?;
        // A terminal JSON report is not enough while the exact helper tree is
        // still present.  The finalizer's deletion of that tree is the
        // independent proof that the second stage actually finished.
        return Ok(metadata_optional(helper_dir)?.is_none());
    }
    Ok(true)
}

#[cfg(windows)]
fn confirm_finalizer_report(report: &Path, helper_dir: Option<&Path>) -> Result<(), String> {
    let deadline = Instant::now() + FINALIZER_REPORT_CONFIRM_TIMEOUT;
    loop {
        let detail = match finalizer_completion_ready(report, helper_dir) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                "卸载报告尚未确认收尾完成（helperFinalization/status 未完成，或临时 helper 尚未删除）".to_string()
            }
            Err(error) => format!("读取最终卸载报告失败：{error}"),
        };
        if Instant::now() >= deadline {
            return Err(detail);
        }
        std::thread::sleep(FINALIZER_REPORT_CONFIRM_INTERVAL);
    }
}

fn validated_helper_cleanup_targets(
    helper_dir: &Path,
    roots: &CleanupRoots,
) -> Result<Vec<PathBuf>, String> {
    check_ancestors(helper_dir)?;
    if helper_dir.parent() != Some(roots.temp.as_path()) {
        return Err("拒绝为临时目录外的路径登记重启清理".into());
    }
    let nonce = helper_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(HELPER_PREFIX))
        .ok_or("重启清理目录名无效")?;
    if uuid::Uuid::parse_str(nonce)
        .map_err(|_| "重启清理 UUID 无效")?
        .to_string()
        != nonce
    {
        return Err("重启清理 UUID 不规范".into());
    }
    let metadata = metadata_optional(helper_dir)?.ok_or("临时清理目录已不存在")?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err("拒绝登记非普通临时清理目录".into());
    }
    for entry in fs::read_dir(helper_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_owned)
            .ok_or("临时清理目录含非 Unicode 条目")?;
        let metadata = metadata_optional(&path)?.ok_or("临时清理条目在核验时消失")?;
        if !HELPER_FILES.contains(&name.as_str()) || !metadata.is_file() || is_reparse(&metadata) {
            return Err(format!(
                "临时清理目录含未知或非普通条目，拒绝登记重启删除：{}",
                path.display()
            ));
        }
    }

    let mut targets = Vec::new();
    for name in HELPER_FILES {
        let path = helper_dir.join(name);
        if metadata_optional(&path)?.is_some() {
            targets.push(path);
        }
    }
    targets.push(helper_dir.to_path_buf());
    Ok(targets)
}

#[cfg(windows)]
fn schedule_helper_cleanup_on_reboot(
    helper_dir: &Path,
    roots: &CleanupRoots,
) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};

    let targets = validated_helper_cleanup_targets(helper_dir, roots)?;

    let schedule = |path: &Path| -> Result<(), String> {
        check_ancestors(path)?;
        let metadata = metadata_optional(path)?.ok_or("重启清理目标在登记前消失")?;
        let expected_type = if path == helper_dir {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
        if !expected_type || is_reparse(&metadata) {
            return Err(format!("拒绝登记类型变化或重解析目标：{}", path.display()));
        }
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe { MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0
        {
            return Err(format!(
                "登记重启删除 {} 失败：{}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
        Ok(())
    };

    let files = targets.iter().filter(|path| path != &helper_dir);
    let mut scheduled = 0usize;
    for path in files {
        schedule(path)?;
        scheduled += 1;
    }
    Ok(format!(
        "即时收尾失败后已登记 {scheduled} 个已核验文件在下次 Windows 启动时删除；临时目录将在后续启动的空目录清理中移除"
    ))
}

fn validate_plan(
    plan: &CleanupPlan,
    helper_dir: &Path,
    current_exe: &Path,
    roots: &CleanupRoots,
) -> Result<(), String> {
    if plan.remove_usb_media {
        return Err("当前版本始终保留 U 盘安装包，拒绝执行介质删除计划".into());
    }
    let nonce = uuid::Uuid::parse_str(&plan.nonce).map_err(|_| "无效清理批次")?;
    if plan.schema_version != 3 || nonce.to_string() != plan.nonce || &plan.roots != roots {
        return Err("清理计划身份/用户目录不匹配".into());
    }
    if plan.remove_usb_media != plan.source_license.is_some() {
        return Err("清理计划缺少匹配的 U 盘许可身份".into());
    }
    if helper_dir.parent() != Some(roots.temp.as_path())
        || helper_dir.file_name().and_then(|value| value.to_str())
            != Some(&format!("{HELPER_PREFIX}{}", plan.nonce))
        || current_exe != helper_dir.join(HELPER_EXE)
    {
        return Err("清理程序不在已验证的临时子目录".into());
    }
    check_ancestors(helper_dir)?;
    check_ancestors(&plan.source_executable)?;
    if sha256(current_exe)? != plan.executable_sha256
        || sha256(&plan.source_executable)? != plan.executable_sha256
    {
        return Err("清理程序和卸载器原件的 SHA256 不一致".into());
    }
    #[cfg(windows)]
    if plan.remove_usb_media
        && plan.source_volume.as_ref() != Some(&volume_identity(&plan.source_executable)?)
    {
        return Err("安装介质已更换或盘符复用，拒绝清理".into());
    }
    if plan.remove_usb_media
        && plan.source_media.as_ref()
            != Some(&media_snapshot(
                plan.source_executable
                    .parent()
                    .ok_or("安装介质父目录缺失")?,
            )?)
    {
        return Err("安装介质校验清单已更改，拒绝清理".into());
    }
    if plan.force_shared_cleanup {
        Ok(())
    } else {
        ensure_journals_recovered(roots)
    }
}

/// Call only after AppX, process, configuration, volume and policy cleanup
/// succeeded. This handoff itself performs NO target deletion. The manager
/// must display report_path then exit normally after successful return.
pub fn launch_deferred_cleanup(
    purge_user_data: bool,
    purge_ccswitch_data: bool,
    force_shared_cleanup: bool,
    remove_usb_media: bool,
    source_license: Option<crate::app::usb_guard::UsbFingerprint>,
    prior_notes: Vec<String>,
) -> Result<CleanupHandoff, String> {
    if remove_usb_media {
        return Err("当前版本始终保留 U 盘安装包，拒绝启动介质删除计划".into());
    }
    #[cfg(not(windows))]
    {
        let _ = (
            purge_user_data,
            purge_ccswitch_data,
            force_shared_cleanup,
            remove_usb_media,
            source_license,
            prior_notes,
        );
        Err("二阶段清理仅支持 Windows".into())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, CREATE_NO_WINDOW, DETACHED_PROCESS,
        };
        if std::env::var_os("CAM_PACKAGED_SMOKE_RUN").is_some()
            || std::env::var_os("CAM_PACKAGED_SMOKE_DATA_DIR").is_some()
        {
            return Err("测试隔离环境禁止启动真实清理".into());
        }
        let roots = CleanupRoots::discover()?;
        if remove_usb_media != source_license.is_some() {
            return Err("删除安装介质需要当前进程持有的 U 盘许可身份".into());
        }
        if !force_shared_cleanup {
            ensure_journals_recovered(&roots)?;
        }
        // Resolve all planned names before detachment so unreadable roots do
        // not masquerade as a successful empty cleanup.
        let _ = roots.targets(purge_user_data, purge_ccswitch_data)?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let helper_dir = roots.temp.join(format!("{HELPER_PREFIX}{nonce}"));
        fs::create_dir(&helper_dir).map_err(|error| format!("创建独立清理目录失败：{error}"))?;
        let mut unstarted = UnstartedHelper {
            directory: helper_dir.clone(),
            armed: true,
        };
        check_ancestors(&helper_dir)?;
        let source_executable =
            fs::canonicalize(std::env::current_exe().map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        let helper_exe = helper_dir.join(HELPER_EXE);
        fs::copy(&source_executable, &helper_exe)
            .map_err(|error| format!("复制清理组件失败：{error}"))?;
        let source_volume = if remove_usb_media {
            Some(volume_identity(&source_executable)?)
        } else {
            None
        };
        let source_media = if remove_usb_media {
            Some(media_snapshot(
                source_executable.parent().ok_or("安装介质父目录缺失")?,
            )?)
        } else {
            None
        };
        let plan = CleanupPlan {
            schema_version: 3,
            nonce: nonce.clone(),
            parent_pid: std::process::id(),
            parent_creation_time: process_creation_time(unsafe { GetCurrentProcess() })?,
            executable_sha256: sha256(&source_executable)?,
            source_executable,
            roots: roots.clone(),
            purge_user_data,
            purge_ccswitch_data,
            force_shared_cleanup,
            remove_usb_media,
            source_volume,
            source_media,
            source_license,
            prior_notes,
        };
        if sha256(&helper_exe)? != plan.executable_sha256 {
            return Err("清理组件复制校验失败".into());
        }
        let plan_path = helper_dir.join("plan.json");
        let plan_bytes = serde_json::to_vec(&plan).map_err(|error| error.to_string())?;
        if plan_bytes.len() > 65536 {
            return Err("清理计划超过大小限制".into());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&plan_path)
            .map_err(|error| error.to_string())?;
        file.write_all(&plan_bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        let report = report_path(&roots, &nonce);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&report)
            .map_err(|error| format!("创建清理报告失败：{error}"))?;
        write_report(
            &report,
            &CleanupReport {
                schema_version: 1,
                status: "awaitingManagerExit".into(),
                phase: "prepare".into(),
                exit_code: None,
                retry_suggestion: "等待安装器退出后自动继续清理；失败时可根据报告重试卸载".into(),
                items: plan
                    .prior_notes
                    .iter()
                    .map(|note| CleanupItem {
                        path: "systemUninstall".into(),
                        status: "note".into(),
                        detail: note.clone(),
                    })
                    .collect(),
                helper_finalization: "pending".into(),
                note: "此报告按卸载确认保留在所示位置；应用数据尚未开始二阶段清理。".into(),
            },
        )?;
        let mut child = Command::new(&helper_exe)
            .current_dir(&helper_dir)
            .arg(HELPER_ARG)
            .arg(&nonce)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .map_err(|error| format!("启动清理组件失败：{error}；报告：{}", report.display()))?;
        unstarted.armed = false;
        let ready = helper_dir.join("ready.json");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            if let Some(metadata) = metadata_optional(&ready)? {
                if is_reparse(&metadata) {
                    return Err("清理握手文件被替换为链接".into());
                }
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&ready).map_err(|error| error.to_string())?)
                        .map_err(|error| error.to_string())?;
                if value["nonce"] == nonce
                    && value["parentPid"] == plan.parent_pid
                    && value["helperPid"] == child.id()
                {
                    // A late helper must not begin deletion merely because the
                    // manager eventually exits after a failed ready handshake.
                    let commit = helper_dir.join("commit.pending");
                    let mut file = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&commit)
                        .map_err(|error| format!("确认二阶段清理失败：{error}"))?;
                    file.write_all(&serde_json::to_vec(&serde_json::json!({"nonce":nonce,"parentPid":plan.parent_pid,"helperPid":child.id()})).map_err(|error| error.to_string())?).and_then(|_| file.sync_all()).map_err(|error| error.to_string())?;
                    drop(file);
                    fs::rename(&commit, helper_dir.join("commit.json"))
                        .map_err(|error| error.to_string())?;
                    return Ok(CleanupHandoff {
                        handoff_id: nonce.clone(),
                        report_path: report.display().to_string(),
                        message: "系统卸载已完成，清理组件已就绪。关闭安装器后清理日志、缓存、锁文件和暂存；最终结果保留在所示路径的报告。".into(),
                    });
                }
                return Err("清理组件握手身份不匹配".into());
            }
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "清理组件就绪前退出（{status}）；报告：{}",
                    report.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // No deletion occurs until the manager exits. Do not kill by name or
        // force-close the app if preparation failed; helper wait is bounded.
        Err(format!(
            "清理组件就绪超时；安装器未退出，数据保留。报告：{}",
            report.display()
        ))
    }
}

#[cfg(windows)]
fn show_summary(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let body = message.to_owned();
    let (completed, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let body = body.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let title = "Codex 卸载清理结果"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        let _ = completed.send(());
    });
    // A helper must never keep its own 20 MB image mapped forever merely
    // because nobody is present to dismiss an error dialog. The durable report
    // remains after this bounded notification closes with the process.
    let _ = wait.recv_timeout(Duration::from_secs(60));
}

#[cfg(windows)]
fn finalizer_script(
    helper_dir: &Path,
    report: &Path,
    lifetime_token: &str,
    show_result: bool,
) -> String {
    let quote = |path: &Path| {
        let raw = path.display().to_string();
        let plain = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
        };
        format!("'{}'", plain.replace('\'', "''"))
    };
    // The final OS shell removes exact known files, then the empty UUID dir.
    // No recursive delete, glob, encoded user command, task, or service. If
    // this immediate shell is blocked, the Rust helper separately registers
    // only this validated UUID directory's known files for reboot cleanup;
    // the report remains partial and never claims immediate removal.
    format!(
        r#"$ErrorActionPreference='Stop'
$report={report}
$helper={helper}
$lock=Join-Path $helper 'lifetime.lock'
$expected='{token}'
$finalizationRecorded=$false
$finalizationError=$null
$r=$null
function Assert-SafeAncestors([string]$path) {{
  $cursor=$path
  while($cursor) {{
    if(Test-Path -LiteralPath $cursor) {{ if(((Get-Item -LiteralPath $cursor -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw 'Refusing reparse ancestor' }} }}
    $parent=[IO.Directory]::GetParent($cursor)
    if($null -eq $parent) {{ break }}
    $cursor=$parent.FullName
  }}
}}
function Assert-KnownHelperEntries {{
  $allowed=@('CodexUninstallCleanup.exe','plan.json','ready.json','ready.pending','finalizer.ready','finalizer.ready.pending','finalizer.error.txt','finalizer.error.pending','commit.json','commit.pending','lifetime.lock')
  foreach($entry in @(Get-ChildItem -LiteralPath $helper -Force -ErrorAction Stop)) {{
    if($allowed -notcontains $entry.Name) {{ throw ('Unexpected cleanup helper file: '+$entry.Name) }}
    if($entry.PSIsContainer) {{ throw ('Unexpected cleanup helper directory: '+$entry.Name) }}
    if(($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw ('Refusing helper reparse point: '+$entry.Name) }}
  }}
}}
function Publish-FinalizerError([string]$problem) {{
  try {{
    Assert-SafeAncestors $helper
    $pending=Join-Path $helper 'finalizer.error.pending'
    $target=Join-Path $helper 'finalizer.error.txt'
    if((Test-Path -LiteralPath $pending) -or (Test-Path -LiteralPath $target)) {{ return }}
    [IO.File]::WriteAllText($pending,$problem)
    Move-Item -LiteralPath $pending -Destination $target -ErrorAction Stop
  }} catch {{ }}
}}
function Publish-FinalizerReport([object]$value, [bool]$allowFailure) {{
  # Never expose a partially overwritten JSON file to the Rust confirmer.
  # The staging file is in the same directory so the final move is a
  # same-volume replacement, and its name is bound to this nonce.
  Assert-SafeAncestors $report
  $pendingReport=$report+'.{token}.pending'
  Assert-SafeAncestors $pendingReport
  if(Test-Path -LiteralPath $pendingReport) {{ throw 'Finalizer report staging file already exists' }}
  $json=$value | ConvertTo-Json -Depth 10
  $json | Set-Content -LiteralPath $pendingReport -Encoding UTF8
  $staged=Get-Content -LiteralPath $pendingReport -Raw -Encoding UTF8 | ConvertFrom-Json
  if(-not $allowFailure -and [string]$staged.helperFinalization -ne 'removed') {{ throw 'Staged finalizer report did not confirm helper removal' }}
  if($allowFailure -and [string]$staged.status -ne 'partialFailure') {{ throw ('Staged finalizer failure report status is not partialFailure: '+[string]$staged.status) }}
  if(-not $allowFailure -and @('completed','partialFailure') -notcontains [string]$staged.status) {{ throw ('Staged finalizer report status is not terminal: '+[string]$staged.status) }}
  Move-Item -LiteralPath $pendingReport -Destination $report -Force -ErrorAction Stop
  return (Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json)
}}
function Get-ReportIssueSummary([object]$value) {{
  if($null -eq $value) {{ return $null }}
  $lines=@()
  foreach($item in @($value.items)) {{
    if(@('failed','deferred','preserved','warning') -notcontains [string]$item.status) {{ continue }}
    $detail=[string]$item.detail
    $detail=($detail -replace '[\r\n]+',' ')
    if($detail.Length -gt 720) {{ $detail=$detail.Substring(0,720)+'…' }}
    $lines += ('- ['+[string]$item.status+'] '+[string]$item.path+'：'+$detail)
    if($lines.Count -ge 8) {{ break }}
  }}
  if($lines.Count -eq 0) {{ return $null }}
  return ('具体原因/保留项：' + "`r`n" + ($lines -join "`r`n"))
}}
function Register-RebootDelete([string]$path) {{
  Assert-SafeAncestors $path
  $entry=Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if(($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw ('Refusing reboot cleanup reparse point: '+$path) }}
  if($path -eq $helper) {{
    if(-not $entry.PSIsContainer) {{ throw ('Refusing reboot cleanup type change: '+$path) }}
  }} elseif($entry.PSIsContainer) {{
    throw ('Refusing reboot cleanup type change: '+$path)
  }}
  if(-not ('CodexUsbUninstallRebootDelete' -as [type])) {{
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class CodexUsbUninstallRebootDelete {{
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  private static extern bool MoveFileEx(string existingFileName, string newFileName, int flags);
  public static void Delete(string path) {{
    if(!MoveFileEx(path, null, 0x00000004)) throw new Win32Exception(Marshal.GetLastWin32Error());
  }}
}}
'@
  }}
  [CodexUsbUninstallRebootDelete]::Delete($path)
}}
function Register-HelperRebootCleanup {{
  # Only the exact helper allow-list can be deferred.  An unknown entry keeps
  # the directory out of the reboot queue so user data cannot be swept.
  Assert-KnownHelperEntries
  $scheduled=@()
  foreach($name in @('CodexUninstallCleanup.exe','plan.json','ready.json','ready.pending','finalizer.ready','finalizer.ready.pending','finalizer.error.txt','finalizer.error.pending','commit.json','commit.pending','lifetime.lock')) {{
    $f=Join-Path $helper $name
    if(Test-Path -LiteralPath $f) {{ Register-RebootDelete $f; $scheduled += $f }}
  }}
  return ('已登记 '+$scheduled.Count+' 个已核验收尾文件在下次 Windows 启动时删除；临时目录将在后续启动的空目录清理中移除')
}}
try {{
  Assert-SafeAncestors $helper
  Assert-SafeAncestors $report
  Assert-SafeAncestors $lock
  if(-not (Test-Path -LiteralPath $lock -PathType Leaf)) {{ throw 'Cleanup lifetime lock is missing' }}
  if(((Get-Item -LiteralPath $lock -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw 'Refusing lifetime lock reparse point' }}
  Assert-KnownHelperEntries
  $readyPending=Join-Path $helper 'finalizer.ready.pending'
  $ready=Join-Path $helper 'finalizer.ready'
  if((Test-Path -LiteralPath $readyPending) -or (Test-Path -LiteralPath $ready)) {{ throw 'Finalizer ready marker already exists' }}
  [IO.File]::WriteAllText($readyPending,$expected)
  Move-Item -LiteralPath $readyPending -Destination $ready -ErrorAction Stop
  $observed=$null
  $lockError=$null
  for($attempt=0;$attempt -lt 240;$attempt++) {{
    try {{ $observed=[IO.File]::ReadAllText($lock); $lockError=$null; break }}
    catch {{ $lockError=$_.Exception.Message; Start-Sleep -Milliseconds 500 }}
  }}
  if($null -eq $observed) {{ throw ('Cleanup lifetime lock was not released: '+$lockError) }}
  if($observed -cne $expected) {{ throw 'Cleanup lifetime token mismatch' }}
  $last=$null
  for($attempt=0;$attempt -lt 40;$attempt++) {{
    try {{
      Assert-SafeAncestors $helper
      Assert-SafeAncestors $report
      Assert-KnownHelperEntries
      foreach($name in @('CodexUninstallCleanup.exe','ready.json','ready.pending','finalizer.ready','finalizer.ready.pending','finalizer.error.txt','finalizer.error.pending','commit.json','commit.pending','lifetime.lock')) {{
        $f=Join-Path $helper $name
        if(Test-Path -LiteralPath $f) {{ if(((Get-Item -LiteralPath $f -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw 'Refusing helper reparse point' }}; Remove-Item -LiteralPath $f -Force -ErrorAction Stop }}
      }}
      $plan=Join-Path $helper 'plan.json'
      if(Test-Path -LiteralPath $plan) {{ if(((Get-Item -LiteralPath $plan -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw 'Refusing helper plan reparse point' }}; Remove-Item -LiteralPath $plan -Force -ErrorAction Stop }}
      if(Test-Path -LiteralPath $helper) {{ if(((Get-Item -LiteralPath $helper -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {{ throw 'Refusing helper directory reparse point' }}; if(@(Get-ChildItem -LiteralPath $helper -Force).Count -ne 0) {{ throw 'Unexpected cleanup helper files retained' }}; Remove-Item -LiteralPath $helper -Force -ErrorAction Stop }}
      $last=$null; break
    }} catch {{ $last=$_.Exception.Message; Start-Sleep -Milliseconds 500 }}
  }}
  if($last) {{ throw $last }}
  $r=Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
  $r.helperFinalization='removed'
  $r.phase='completed'
  $r.exitCode=0
  $r.retrySuggestion=''
  if($r.status -eq 'finalizing') {{ $r.status='completed' }}
  $r=Publish-FinalizerReport $r $false
  $finalizationRecorded=$true
}} catch {{
  $finalizationRecorded=$false
  $problem=$_.Exception.Message
  # Publish the error before registering the reboot queue so the newly-created
  # finalizer.error.txt is included in the exact allow-list. Otherwise the
  # helper directory would remain non-empty after reboot.
  $finalizationError=$problem
  Publish-FinalizerError $problem
  $rebootDetail=$null
  try {{ $rebootDetail=Register-HelperRebootCleanup }} catch {{ $rebootDetail='重启兜底登记失败：'+$_.Exception.Message }}
  if($rebootDetail) {{ $problem=$problem+'；'+$rebootDetail }}
  $finalizationError=$problem
  try {{
    Assert-SafeAncestors $report
    $pendingReport=$report+'.{token}.pending'
    Assert-SafeAncestors $pendingReport
    if(Test-Path -LiteralPath $pendingReport) {{ Remove-Item -LiteralPath $pendingReport -Force -ErrorAction Stop }}
    $r=Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
    $r.helperFinalization='failed: '+$problem
    $r.phase='failed'
    $r.exitCode=2
    $r.retrySuggestion=if($rebootDetail -and $rebootDetail -notlike '重启兜底登记失败：*') {{ $rebootDetail }} else {{ '请根据报告中的具体失败文件处理占用或权限后重试卸载' }}
    $rebootStatus=if($rebootDetail -and $rebootDetail -notlike '重启兜底登记失败：*') {{ 'deferred' }} else {{ 'failed' }}
    $r.items=@($r.items)+[pscustomobject]@{{ path=$helper; status=$rebootStatus; detail=$rebootDetail }}
    $r.status='partialFailure'
    $null=Publish-FinalizerReport $r $true
  }} catch {{ }}
}}
if({show_result}) {{ try {{
  $summary=if($finalizationRecorded -and $r.status -eq 'completed') {{ '已完成所选范围清理，临时清理组件也已删除。' }} else {{ '部分卸载清理未完成，请查看报告中的失败与保留项目。' }}
  if($finalizationError) {{ $summary += "`r`n收尾错误："+$finalizationError }}
  $issueSummary=Get-ReportIssueSummary $r
  if($issueSummary) {{ $summary += "`r`n"+$issueSummary }}
  $shell=New-Object -ComObject WScript.Shell
  $null=$shell.Popup(($summary+"`r`n"+$report),60,'Codex 卸载清理结果',64)
}} catch {{ }} }}
if(-not $finalizationRecorded) {{ exit 2 }}
exit 0
"#,
        report = quote(report),
        helper = quote(helper_dir),
        token = lifetime_token.replace('\'', "''"),
        show_result = if show_result { "$true" } else { "$false" },
    )
}

#[cfg(windows)]
fn powershell_encoded_command(script: &str) -> String {
    use base64::Engine as _;

    // PowerShell's EncodedCommand contract is UTF-16LE. Passing the generated
    // script as a single base64 argument avoids Windows command-line quoting,
    // newline, non-ASCII path, and ampersand parsing differences that can make
    // the detached finalizer exit before it publishes its ready marker.
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
/// Start the detached finalizer and wait only for the handoff marker.
///
/// `Ok(())` means that the finalizer owns the rest of the cleanup. It does
/// not mean that the final report is complete. The caller must release the
/// lifetime lock and let this helper process exit so the finalizer can remove
/// the helper image itself.
fn schedule_self_cleanup(
    helper_dir: &Path,
    report: &Path,
    roots: &CleanupRoots,
    lifetime_token: &str,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};
    check_ancestors(helper_dir)?;
    check_ancestors(report)?;
    let script = finalizer_script(helper_dir, report, lifetime_token, true);
    let system_root = std::env::var_os("SystemRoot").ok_or("无法确定 Windows 目录")?;
    let shell = PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    check_ancestors(&shell)?;
    // Keep this bootstrap ASCII-only on the command line.  The actual
    // generated script is UTF-8 and can contain a non-ASCII Windows user or
    // USB path, so the detached PowerShell process reads it explicitly with
    // UTF-8 instead of relying on the active console code page.
    let mut child = Command::new(shell)
        .current_dir(
            PathBuf::from(std::env::var_os("SystemRoot").ok_or("无法确定 Windows 工作目录")?)
                .join("System32"),
        )
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            // Read the generated script from stdin.  The previous encoded
            // command put the entire finalizer on the Windows command line;
            // long paths and the PowerShell UTF-16/base64 expansion could
            // make the detached process exit before publishing
            // `finalizer.ready`, which surfaced as a false second-stage
            // uninstall failure.
            "-Command",
            FINALIZER_STDIN_BOOTSTRAP,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|error| format!("自清理启动失败：{error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(error) = stdin.write_all(script.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("向自清理进程写入脚本失败：{error}"));
        }
        // Dropping stdin sends EOF, allowing `powershell -Command -` to
        // parse and execute the complete script in the detached child.
    } else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("自清理进程未提供脚本输入管道".into());
    }
    let ready = helper_dir.join("finalizer.ready");
    let expected = lifetime_token;
    let start = Instant::now();
    while start.elapsed() < FINALIZER_START_TIMEOUT {
        check_ancestors(&ready)?;
        if metadata_optional(&ready)?.is_some()
            && fs::read_to_string(&ready).map_err(|error| error.to_string())? == expected
        {
            return Ok(());
        }
        if let Some(error) = read_finalizer_error(helper_dir)? {
            // The durable error is already published. Do not wait behind the
            // finalizer's bounded user notification; the helper owns the
            // authoritative report update and reboot fallback from here.
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("临时组件收尾进程启动失败：{error}"));
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            if status.success() {
                // The marker may have been created and consumed between two
                // 100 ms polls.  Confirm the durable report for a short,
                // bounded window instead of treating one transient read or
                // parse failure as a cleanup failure. The PowerShell script
                // also performs a read-back check before it can return 0.
                match confirm_finalizer_report(report, Some(helper_dir)) {
                    Ok(()) => return Ok(()),
                    Err(report_error) => {
                        // A Windows delete-pending handle can make the
                        // helper directory disappear just after PowerShell
                        // exits.  If that happened and the only missed step
                        // was durable report publication, repair the report
                        // from the independently verified directory absence.
                        if repair_successful_finalizer_report(
                            helper_dir,
                            report,
                            roots,
                            lifetime_token,
                        )? {
                            return Ok(());
                        }
                        let detail = read_finalizer_error(helper_dir)?
                            .map(|error| format!("{report_error}；收尾错误：{error}"))
                            .unwrap_or(report_error);
                        return Err(format!(
                            "临时组件收尾进程已退出但未确认最终报告：{status}；{detail}"
                        ));
                    }
                }
            }
            let detail = read_finalizer_error(helper_dir)?
                .unwrap_or_else(|| "收尾进程未写入错误详情".into());
            return Err(format!(
                "临时组件收尾进程尚未就绪即退出：{status}；{detail}"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(format!(
        "临时组件收尾进程 {} 秒内未确认就绪，已终止该精确子进程；保留报告并登记兜底清理，未宣称彻底完成",
        FINALIZER_START_TIMEOUT.as_secs()
    ))
}

#[cfg(windows)]
fn run_helper(nonce_arg: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let current_exe = fs::canonicalize(std::env::current_exe().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let helper_dir = current_exe.parent().ok_or("清理目录不存在")?;
    let roots = CleanupRoots::discover()?;
    // Reject manual invocation outside the exact temporary helper layout BEFORE
    // opening an attacker-selected plan, showing UI, or creating files.
    let nonce = uuid::Uuid::parse_str(nonce_arg)
        .map_err(|_| "无效清理批次")?
        .to_string();
    if nonce != nonce_arg
        || helper_dir != roots.temp.join(format!("{HELPER_PREFIX}{nonce}"))
        || current_exe.file_name().and_then(|name| name.to_str()) != Some(HELPER_EXE)
    {
        return Err("二阶段清理只能由安装器生成并启动".into());
    }
    check_ancestors(&helper_dir.join("plan.json"))?;
    if metadata_optional(&helper_dir.join("plan.json"))?
        .ok_or("清理计划不存在")?
        .len()
        > 65536
    {
        return Err("清理计划过大".into());
    }
    let bytes = fs::read(helper_dir.join("plan.json")).map_err(|error| error.to_string())?;
    if bytes.len() > 65536 {
        return Err("清理计划过大".into());
    }
    let plan: CleanupPlan = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    validate_plan(&plan, helper_dir, &current_exe, &roots)?;
    if plan.nonce != nonce {
        return Err("清理计划批次不匹配".into());
    }
    let parent = open_parent(&plan)?;
    let _mutex = acquire_cleanup_mutex()?;
    let lifetime_lock = create_lifetime_lock(helper_dir, &nonce)?;
    let report_file = report_path(&roots, &nonce);
    let mut report = CleanupReport { schema_version: 1, status: "awaitingManagerExit".into(), phase: "prepare".into(), exit_code: None, retry_suggestion: "等待安装器退出后自动继续清理；失败时可根据报告重试卸载".into(), items: plan.prior_notes.iter().map(|note| CleanupItem { path: "systemUninstall".into(), status: "note".into(), detail: note.clone() }).collect(), helper_finalization: "pending".into(),
        note: "本报告是卸载确认中声明保留的唯一诊断输出。共享 VC++/WebView2、非本产品文件及未知系统状态不强制删除。".into() };
    write_report(&report_file, &report)?;
    // Atomic ready publication avoids a partial JSON handshake read.
    let ready_bytes = serde_json::to_vec(&serde_json::json!({"nonce":nonce,"parentPid":plan.parent_pid,"helperPid":std::process::id()})).map_err(|error| error.to_string())?;
    let pending = helper_dir.join("ready.pending");
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending)
        .and_then(|mut file| file.write_all(&ready_bytes))
        .map_err(|error| error.to_string())?;
    fs::rename(&pending, helper_dir.join("ready.json")).map_err(|error| error.to_string())?;
    report.phase = "cleanup".into();
    let cleanup = (|| {
        // The helper has already published a verified handoff and the manager
        // owns the only explicit commit file. Waiting on the exact parent
        // process handle avoids a wall-clock race with a user reading the
        // result. If the manager never exits, no target is deleted; if it
        // exits without the commit file, the next check below still refuses
        // deletion.
        if unsafe { WaitForSingleObject(parent.0, u32::MAX) } != WAIT_OBJECT_0 {
            return Err("无法等待安装器进程退出，未执行任何数据删除".to_string());
        }
        let commit = helper_dir.join("commit.json");
        check_ancestors(&commit)?;
        if metadata_optional(&commit)?
            .ok_or("安装器未确认二阶段清理；未执行删除")?
            .len()
            > 4096
        {
            return Err("二阶段确认文件过大".into());
        }
        let confirmation: serde_json::Value =
            serde_json::from_slice(&fs::read(&commit).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if confirmation["nonce"] != plan.nonce
            || confirmation["parentPid"] != plan.parent_pid
            || confirmation["helperPid"] != std::process::id()
        {
            return Err("二阶段确认身份不匹配；未执行删除".into());
        }
        if !plan.force_shared_cleanup {
            ensure_journals_recovered(&roots)?;
        }
        // The parent has exited, so the log sink is closed. Preserve the
        // bounded newest tail inside the durable report before deleting the
        // manager-owned log/cache tree.
        report.items.push(capture_latest_log_tail(&roots));
        // Read the ownership record before `.codex` is removed.  Only names
        // explicitly recorded by this capability pack are eligible for HKCU
        // environment cleanup; an absent or invalid record never broadens
        // the deletion scope.
        let owned_connection_env_names = if plan.purge_user_data {
            Some(read_owned_connection_env_names(&roots))
        } else {
            None
        };
        let selected_targets = roots.targets(plan.purge_user_data, plan.purge_ccswitch_data)?;
        report
            .items
            .extend(clean_targets_with_recheck(&selected_targets, || {}));
        if let Some(result) = owned_connection_env_names {
            match result {
                Ok(Some(names)) => report
                    .items
                    .push(cleanup_owned_connection_environment(&names)),
                Ok(None) => report.items.push(CleanupItem {
                    path: roots
                        .home
                        .join(CONNECTION_OWNERSHIP_RELATIVE)
                        .display()
                        .to_string(),
                    status: "absent".into(),
                    detail: "未发现能力包连接所有权记录；未触碰未核验的环境变量".into(),
                }),
                Err(error) => report.items.push(CleanupItem {
                    path: roots
                        .home
                        .join(CONNECTION_OWNERSHIP_RELATIVE)
                        .display()
                        .to_string(),
                    status: "failed".into(),
                    detail: error,
                }),
            }
        }
        report
            .items
            .extend(clean_stale_helpers(&roots, helper_dir)?);
        for target in &selected_targets {
            if let Err(error) = verify_absent(target) {
                report.items.push(CleanupItem {
                    path: target.display().to_string(),
                    status: "failed".into(),
                    detail: format!("介质清理前最终复核失败：{error}"),
                });
            }
        }
        let mut empty_containers = vec![
            roots.local.join("wangnov"),
            roots.roaming.join("wangnov"),
            roots.local.join("CodexAppxDataBackups"),
        ];
        if plan.purge_user_data {
            empty_containers.extend([
                roots.home.join(".agents"),
                roots.local.join("OpenAI"),
                roots.roaming.join("OpenAI"),
                roots.local.join("Codex"),
            ]);
        }
        report.items.extend(
            empty_containers
                .iter()
                .map(|path| clean_empty_container(path)),
        );
        if plan.remove_usb_media && report.items.iter().any(|item| item.status == "failed") {
            report.items.push(CleanupItem {
                path: plan
                    .source_executable
                    .parent()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                status: "preserved".into(),
                detail: "本地清理尚有失败项，保留原安装介质以便重试；未删除卸载入口。".into(),
            });
        } else if plan.remove_usb_media {
            let bundle = plan
                .source_executable
                .parent()
                .ok_or("无法确定安装介质目录")?;
            let media_result = if plan.source_volume.as_ref() != Some(&volume_identity(bundle)?)
                || plan.source_media.as_ref() != Some(&media_snapshot(bundle)?)
            {
                Err("原 U 盘已拔出或盘符被其他卷复用，未删除替代介质".into())
            } else {
                crate::app::usb_media_cleanup::cleanup_authorized_bundle(
                    bundle,
                    plan.source_license
                        .as_ref()
                        .ok_or("清理计划缺少 U 盘许可身份")?,
                )
            };
            match media_result {
                Ok(notes) => report.items.push(CleanupItem {
                    path: bundle.display().to_string(),
                    status: "removed".into(),
                    detail: notes.join("；"),
                }),
                Err(error) => report.items.push(CleanupItem {
                    path: bundle.display().to_string(),
                    status: "failed".into(),
                    detail: error,
                }),
            }
        }
        Ok(())
    })();
    if let Err(error) = cleanup {
        report.items.push(CleanupItem {
            path: "cleanupPrecondition".into(),
            status: "failed".into(),
            detail: error,
        });
    }
    report.status = if report.items.iter().any(|item| item.status == "failed") {
        "partialFailure"
    } else {
        "finalizing"
    }
    .into();
    report.phase = "finalizing".into();
    report.exit_code = None;
    report.retry_suggestion = if report.status == "partialFailure" {
        "请查看报告中的失败/保留项，处理占用或权限后重试卸载".into()
    } else {
        "等待收尾组件完成自清理".into()
    };
    write_report(&report_file, &report)?;
    let finalizer_error = schedule_self_cleanup(helper_dir, &report_file, &roots, &nonce).err();
    // `finalizer.ready` means that the detached PowerShell process has taken
    // ownership of the handoff and is waiting for this exact lifetime lock.
    // Once it is ready, this helper must release the lock and exit immediately:
    // the finalizer cannot delete CodexUninstallCleanup.exe while this process
    // still has its own image mapped. The finalizer owns the durable report,
    // user notification, and any reboot fallback after that point.
    drop(lifetime_lock);
    if let Some(error) = finalizer_error {
        let fallback = schedule_helper_cleanup_on_reboot(helper_dir, &roots);
        let fallback_detail = match fallback {
            Ok(detail) => {
                report.items.push(CleanupItem {
                    path: helper_dir.display().to_string(),
                    status: "deferred".into(),
                    detail: detail.clone(),
                });
                detail
            }
            Err(fallback_error) => {
                let detail = format!("重启兜底登记也失败：{fallback_error}");
                report.items.push(CleanupItem {
                    path: helper_dir.display().to_string(),
                    status: "failed".into(),
                    detail: detail.clone(),
                });
                detail
            }
        };
        report.helper_finalization = format!("failed: {error}; {fallback_detail}");
        report.status = "partialFailure".into();
        report.phase = "failed".into();
        report.exit_code = Some(2);
        report.retry_suggestion = "请根据报告中的具体失败文件处理占用或权限后重试卸载".into();
        write_report(&report_file, &report)?;
        let issue_summary = report_issue_summary(&report)
            .unwrap_or_else(|| "具体原因/保留项：未能从报告读取到可展开的失败条目".into());
        return Err(format!(
            "{error}；{fallback_detail}；{issue_summary}；报告：{}",
            report_file.display()
        ));
    }
    Ok(())
}

/// FIRST startup hook. `Some(code)` means helper CLI was handled; caller must
/// terminate immediately with that code and must not initialize Tauri/logging/
/// USB guard afterwards.
pub fn maybe_run_cleanup_helper() -> Option<i32> {
    let args = std::env::args().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == HELPER_ARG) {
        return None;
    }
    #[cfg(windows)]
    {
        let result = if args.len() == 3 && args[1] == HELPER_ARG {
            run_helper(&args[2])
        } else {
            Err("无效的卸载清理参数".into())
        };
        if let Err(error) = result {
            show_summary(&format!(
                "二阶段清理未完成：{error}\n错误已写入卸载报告；窗口最迟 60 秒自动关闭，不会继续占用清理组件。"
            ));
            return Some(2);
        }
        Some(0)
    }
    #[cfg(not(windows))]
    {
        Some(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("codex-cleanup-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = remove_tree_no_follow(&self.0);
        }
    }

    #[cfg(windows)]
    fn test_revocation_root(path: &Path) -> RevocationCleanupRoot {
        let pinned = handle_tree::PinnedRoot::open_for_test(path).unwrap();
        RevocationCleanupRoot {
            bundle_root: path.to_path_buf(),
            fingerprint: crate::app::usb_guard::UsbFingerprint {
                volume_serial: pinned.volume_serial() as u32,
                anchor_sha256: "a".repeat(64),
                bundle_relative_path: "fixture/CodexUSB".to_string(),
                anchor_schema: 3,
            },
            payload_relative: "payload/OpenAI.Codex_x64.msix".to_string(),
            architecture: "x64".to_string(),
            backup_relative: "payload/_backup".to_string(),
            pinned,
        }
    }

    #[cfg(windows)]
    fn open_test_pinned_transaction(
        root: &RevocationCleanupRoot,
        transaction_id: &str,
    ) -> PinnedRevocationTransaction {
        let staging = root.open_pinned_entry(".staging").unwrap().unwrap();
        let staging_ancestor = staging.duplicate().unwrap();
        let sidecar = staging
            .open_child(&format!(".codex-update-{transaction_id}.owner.json"))
            .unwrap()
            .unwrap();
        let relative = format!(".staging/codex-update-{transaction_id}");
        let transaction = staging
            .open_child(&format!("codex-update-{transaction_id}"))
            .unwrap();
        let tree = transaction.map(|entry| entry.prepare_delete_tree().unwrap());
        RevocationCleanupRoot::prepare_pinned_transaction(relative, tree, sidecar, staging_ancestor)
    }

    #[test]
    fn pinned_revocation_policy_requires_matching_schema_three_and_registration() {
        let root = serde_json::json!({
            "schemaVersion": 3,
            "revocationPolicy": { "registrationRequired": true }
        });
        let usb = serde_json::json!({ "schemaVersion": 3 });
        assert!(validate_pinned_revocation_policy(&root, &usb).is_ok());
        assert!(validate_pinned_revocation_policy(
            &serde_json::json!({
                "schemaVersion": 2,
                "revocationPolicy": { "registrationRequired": true }
            }),
            &usb
        )
        .is_err());
        assert!(validate_pinned_revocation_policy(
            &root,
            &serde_json::json!({ "schemaVersion": 2 })
        )
        .is_err());
        assert!(validate_pinned_revocation_policy(
            &serde_json::json!({
                "schemaVersion": 3,
                "revocationPolicy": { "registrationRequired": false }
            }),
            &usb
        )
        .is_err());
    }

    #[test]
    fn exact_legacy_names_only() {
        for good in [
            "codex-usb-uac-123-1788356474731.ps1",
            "codex-usb-uac-123-123.stderr.log",
            "codex-usb-uac-9-33.json",
        ] {
            assert!(is_legacy_uac_file(good));
        }
        for bad in [
            "codex-usb-uac-important.ps1",
            "codex-usb-uac--3.ps1",
            "codex-usb-uac-3-44-directory.ps1",
            "codex-usb-uac-3-44.ps1.backup",
            "my-codex-usb-uac-3-44.ps1",
        ] {
            assert!(!is_legacy_uac_file(bad));
        }
    }

    #[cfg(windows)]
    #[test]
    fn pinned_transaction_partial_delete_retains_owner_and_retries() {
        let temp = TempRoot::new();
        let staging = temp.0.join(".staging");
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let transaction_path = staging.join(format!("codex-update-{transaction_id}"));
        let sidecar_path = staging.join(format!(".codex-update-{transaction_id}.owner.json"));
        fs::create_dir_all(&transaction_path).unwrap();
        let marker = br#"{"transactionId":"owner-bound"}"#;
        fs::write(transaction_path.join("state.json"), marker).unwrap();
        fs::write(transaction_path.join("payload.bin"), b"staged bytes").unwrap();
        fs::write(&sidecar_path, marker).unwrap();
        let outside = temp.0.join("outside-sentinel.txt");
        fs::write(&outside, b"outside must survive").unwrap();
        let root = test_revocation_root(&temp.0);

        let transaction = open_test_pinned_transaction(&root, &transaction_id);
        let staged_count = fs::read_dir(&transaction_path).unwrap().count();
        let error = transaction
            .remove_with_test_hooks(
                &root,
                |index| {
                    if index == 0 {
                        let moved = staging.join("attacker-moved");
                        assert!(
                            fs::rename(&transaction_path, moved).is_err(),
                            "the pinned marker/tree must prevent replacing the verified directory"
                        );
                        assert_eq!(fs::read(&outside).unwrap(), b"outside must survive");
                        Ok(())
                    } else {
                        Err("injected failure after first disposition".to_string())
                    }
                },
                || Ok(()),
            )
            .unwrap_err();
        assert!(error.contains("injected failure"));
        assert!(transaction_path.is_dir());
        assert!(
            sidecar_path.is_file(),
            "owner proof survives partial deletion"
        );
        assert!(fs::read_dir(&transaction_path).unwrap().count() < staged_count);

        let retry = open_test_pinned_transaction(&root, &transaction_id);
        assert!(retry
            .remove_with_test_hooks(&root, |_| Ok(()), || Ok(()))
            .is_ok());
        assert!(!transaction_path.exists());
        assert!(!sidecar_path.exists());
        assert_eq!(fs::read(&outside).unwrap(), b"outside must survive");
    }

    #[cfg(windows)]
    #[test]
    fn pinned_transaction_holds_staging_ancestor_until_remove_returns() {
        let temp = TempRoot::new();
        let staging = temp.0.join(".staging");
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let transaction_path = staging.join(format!("codex-update-{transaction_id}"));
        let sidecar_path = staging.join(format!(".codex-update-{transaction_id}.owner.json"));
        fs::create_dir_all(&transaction_path).unwrap();
        fs::write(transaction_path.join("payload.bin"), b"staged bytes").unwrap();
        fs::write(&sidecar_path, b"owner proof").unwrap();
        let root = test_revocation_root(&temp.0);

        let transaction = open_test_pinned_transaction(&root, &transaction_id);
        let renamed_staging = temp.0.join(".staging-moved");
        assert!(
            fs::rename(&staging, &renamed_staging).is_err(),
            "the transaction must retain the exact pinned .staging ancestor"
        );

        transaction.remove(&root).unwrap();
        assert!(
            !transaction_path.exists() && !sidecar_path.exists(),
            "the pinned transaction and owner sidecar should be removed"
        );
        fs::rename(&staging, &renamed_staging)
            .expect("after transaction cleanup releases the ancestor handle, rename should work");
        assert!(renamed_staging.is_dir());
        fs::rename(&renamed_staging, &staging).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn pinned_transaction_orphan_sidecar_deletion_failure_is_retryable() {
        let temp = TempRoot::new();
        let staging = temp.0.join(".staging");
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let transaction_path = staging.join(format!("codex-update-{transaction_id}"));
        let sidecar_path = staging.join(format!(".codex-update-{transaction_id}.owner.json"));
        fs::create_dir_all(&transaction_path).unwrap();
        fs::write(transaction_path.join("state.json"), b"owner marker").unwrap();
        fs::write(&sidecar_path, b"sidecar marker").unwrap();
        let root = test_revocation_root(&temp.0);

        let transaction = open_test_pinned_transaction(&root, &transaction_id);
        let error = transaction
            .remove_with_test_hooks(
                &root,
                |_| Ok(()),
                || Err("injected sidecar disposition failure".to_string()),
            )
            .unwrap_err();
        assert!(error.contains("sidecar disposition"));
        assert!(
            !transaction_path.exists(),
            "the transaction tree was fully removed"
        );
        assert!(sidecar_path.is_file(), "sidecar remains as retry proof");

        let orphan = open_test_pinned_transaction(&root, &transaction_id);
        assert!(orphan
            .remove_with_test_hooks(&root, |_| Ok(()), || Ok(()))
            .is_ok());
        assert!(!sidecar_path.exists());
    }

    #[test]
    fn exact_legacy_uninstall_report_names_only() {
        for good in [
            "Codex-Uninstall-Report-eeb6727-1980-4830-af22-1b989e2cac82.json",
            "Codex-Uninstall-Report-legacy.json",
        ] {
            assert!(is_legacy_uninstall_report_name(good));
        }
        for bad in [
            "Codex-Uninstall-Report-.json",
            "Codex-Uninstall-Report-legacy.txt",
            "prefix-Codex-Uninstall-Report-legacy.json",
            "Codex-Uninstall-Report-legacy.json.backup",
            "Codex-Uninstall-Report-..\\outside.json",
            "Codex-Uninstall-Report-../outside.json",
        ] {
            assert!(!is_legacy_uninstall_report_name(bad));
        }
    }

    #[test]
    fn clean_nested_tree_and_idempotent_absence() {
        let root = TempRoot::new();
        let target = root.0.join("owned");
        fs::create_dir_all(target.join("cache/sub")).unwrap();
        fs::write(target.join("cache/sub/item"), b"owned").unwrap();
        fs::write(root.0.join("unrelated"), b"keep").unwrap();
        assert_eq!(clean_target(&target).status, "removed");
        assert_eq!(clean_target(&target).status, "absent");
        assert_eq!(fs::read(root.0.join("unrelated")).unwrap(), b"keep");
    }

    #[test]
    fn empty_container_cleanup_is_non_recursive_and_preserves_other_data() {
        let root = TempRoot::new();
        let empty = root.0.join("empty-publisher");
        fs::create_dir(&empty).unwrap();
        assert_eq!(clean_empty_container(&empty).status, "removed");
        assert!(!empty.exists());

        let shared = root.0.join("shared-publisher");
        fs::create_dir(&shared).unwrap();
        fs::write(shared.join("other-product.txt"), b"keep").unwrap();
        let item = clean_empty_container(&shared);
        assert_eq!(item.status, "preserved", "{item:?}");
        assert_eq!(fs::read(shared.join("other-product.txt")).unwrap(), b"keep");
    }

    #[test]
    fn diagnostic_log_tail_is_embedded_before_owned_log_tree_cleanup() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let log = roots
            .local
            .join("io.github.wangnov.codexappmanager/logs/codex-app-manager.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        let mut bytes = vec![b'x'; DIAGNOSTIC_LOG_TAIL_BYTES as usize + 1024];
        bytes.extend_from_slice(b"\n[ERROR] retained-final-diagnostic\n");
        fs::write(&log, bytes).unwrap();

        let item = capture_latest_log_tail(&roots);
        assert_eq!(item.status, "archived", "{item:?}");
        assert!(item.detail.contains("retained-final-diagnostic"));
        assert!(item.detail.contains("嵌入本报告"));
        assert!(log.exists(), "capture itself must not delete the source");
    }

    #[test]
    fn finalizer_error_channel_is_bounded_and_accepts_utf8_bom() {
        let root = TempRoot::new();
        fs::write(
            root.0.join("finalizer.error.txt"),
            b"\xef\xbb\xbfprecise finalizer startup failure\r\n",
        )
        .unwrap();
        assert_eq!(
            read_finalizer_error(&root.0).unwrap().as_deref(),
            Some("precise finalizer startup failure")
        );
        fs::write(
            root.0.join("finalizer.error.txt"),
            vec![b'x'; FINALIZER_ERROR_MAX_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_finalizer_error(&root.0).is_err());
    }

    #[test]
    fn connection_ownership_record_is_bounded_to_declared_pack_names() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let path = roots.home.join(CONNECTION_OWNERSHIP_RELATIVE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::json!({
                "schemaVersion": 1,
                "packId": "codex-starter",
                "names": ["DASHSCOPE_API_KEY", "FEISHU_APP_SECRET", "DASHSCOPE_API_KEY"]
            })
            .to_string(),
        )
        .unwrap();
        let names = read_owned_connection_env_names(&roots).unwrap().unwrap();
        assert_eq!(
            names,
            vec![
                "DASHSCOPE_API_KEY".to_string(),
                "FEISHU_APP_SECRET".to_string()
            ]
        );

        fs::write(
            &path,
            serde_json::json!({
                "schemaVersion": 1,
                "packId": "codex-starter",
                "names": ["PATH"]
            })
            .to_string(),
        )
        .unwrap();
        assert!(read_owned_connection_env_names(&roots).is_err());
    }

    #[test]
    fn reboot_fallback_scope_accepts_only_known_files_in_exact_uuid_directory() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let helper = roots
            .temp
            .join(format!("{HELPER_PREFIX}{}", uuid::Uuid::new_v4()));
        fs::create_dir(&helper).unwrap();
        for name in [HELPER_EXE, "plan.json", "lifetime.lock"] {
            fs::write(helper.join(name), b"owned fixture").unwrap();
        }
        let targets = validated_helper_cleanup_targets(&helper, &roots).unwrap();
        assert_eq!(targets.last(), Some(&helper));
        assert_eq!(targets.len(), 4);
        assert!(targets.iter().all(|path| path.starts_with(&helper)));

        fs::write(helper.join("unrelated-user-file.txt"), b"keep").unwrap();
        assert!(validated_helper_cleanup_targets(&helper, &roots).is_err());
        assert_eq!(
            fs::read(helper.join("unrelated-user-file.txt")).unwrap(),
            b"keep"
        );
        assert!(validated_helper_cleanup_targets(&root.0.join("wrong"), &roots).is_err());
    }

    #[test]
    fn traversal_is_rejected() {
        assert!(check_ancestors(Path::new("relative")).is_err());
        assert!(check_ancestors(&std::env::temp_dir().join("..\\other")).is_err());
    }

    #[test]
    fn report_root_safety_allows_absent_local_and_roaming_wangnov_parents() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let local_wangnov = roots.local.join("wangnov");
        let roaming_wangnov = roots.roaming.join("wangnov");

        assert!(!local_wangnov.exists());
        assert!(!roaming_wangnov.exists());
        roots.ensure_report_root_safe().unwrap();
        assert!(!local_wangnov.exists());
        assert!(!roaming_wangnov.exists());

        for target in [
            roots.local.join("wangnov/codexappmanager"),
            roots.roaming.join("wangnov/codexappmanager"),
        ] {
            assert_eq!(clean_target(&target).status, "absent");
        }
    }

    fn fake_roots(root: &Path) -> CleanupRoots {
        let result = CleanupRoots {
            home: root.join("home"),
            local: root.join("local"),
            roaming: root.join("roaming"),
            temp: root.join("temp"),
            desktop: root.join("desktop"),
        };
        for dir in [
            &result.home,
            &result.local,
            &result.roaming,
            &result.temp,
            &result.desktop,
        ] {
            fs::create_dir(dir).unwrap();
        }
        result
    }

    #[test]
    fn immediate_staging_cleanup_is_exact_and_independent_of_recovery_evidence() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        for target in roots.install_staging_targets() {
            fs::create_dir_all(target.join("nested")).unwrap();
            fs::write(target.join("nested/payload.bin"), b"staged").unwrap();
        }
        let recovery = roots
            .local
            .join("CodexAppxDataBackups/OpenAI.Codex_2p2nqsd0c76g0/recovery.json");
        fs::create_dir_all(recovery.parent().unwrap()).unwrap();
        fs::write(&recovery, b"evidence").unwrap();
        let unrelated = roots.temp.join("unrelated-install-cache/payload.bin");
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(&unrelated, b"keep").unwrap();

        let report = cleanup_install_staging_for_roots(&roots);
        assert!(report.iter().all(|item| item.status == "removed"));
        for target in roots.install_staging_targets() {
            assert!(!target.exists());
        }
        assert_eq!(fs::read(&recovery).unwrap(), b"evidence");
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep");

        let second = cleanup_install_staging_for_roots(&roots);
        assert!(second.iter().all(|item| item.status == "absent"));
    }

    #[test]
    fn uninstall_report_path_is_independent_of_manager_data_root() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let report = report_path(&roots, "g22");
        let manager_root = roots.local.join("io.github.wangnov.codexappmanager");
        assert!(report.starts_with(&roots.desktop));
        assert!(!report.starts_with(&manager_root));
        assert!(!report.starts_with(&roots.local));
    }

    #[test]
    fn allowlist_never_selects_publisher_or_shared_user_data() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let targets = roots.targets(false, false).unwrap();
        assert!(targets.contains(&roots.roaming.join("wangnov/codexappmanager")));
        assert!(!targets.contains(&roots.roaming.join("wangnov")));
        assert!(!targets.contains(&roots.home.join(".codex")));
        assert!(!targets.contains(&roots.home.join(".agents/skills")));
        for shared in [
            roots.local.join("Packages/OpenAI.Codex_2p2nqsd0c76g0"),
            roots.local.join("OpenAI/Codex"),
            roots.local.join("Codex/Logs"),
            roots.roaming.join("OpenAI/Codex"),
            roots.roaming.join("Codex"),
            roots.home.join(".cc-switch"),
            roots.local.join("com.ccswitch.desktop"),
            roots.roaming.join("com.ccswitch.desktop"),
        ] {
            assert!(
                !targets.contains(&shared),
                "default cleanup selected {}",
                shared.display()
            );
        }
        assert!(!targets.contains(&roots.local.join("CodexAppxDataBackups")));
        assert!(roots.targets(true, false).unwrap().contains(
            &roots
                .local
                .join("CodexAppxDataBackups/OpenAI.Codex_2p2nqsd0c76g0")
        ));
        assert!(roots
            .targets(true, false)
            .unwrap()
            .contains(&roots.home.join(".agents/skills")));
    }

    #[test]
    fn default_cleanup_keeps_shared_data_without_auth_files_and_distinct_profile_roots() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let shared = [
            (
                roots.home.join(".codex/state.sqlite"),
                b"important history".as_slice(),
            ),
            (
                roots.local.join("Codex/Logs/same-name.txt"),
                b"local content".as_slice(),
            ),
            (
                roots.roaming.join("Codex/Logs/same-name.txt"),
                b"roaming content".as_slice(),
            ),
            (
                roots.home.join(".cc-switch/settings.json"),
                b"shared switch settings".as_slice(),
            ),
        ];
        for (path, bytes) in &shared {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let owned = roots.local.join("CodexUsbAppx");
        fs::create_dir_all(&owned).unwrap();
        fs::write(owned.join("fixture.txt"), b"owned cache").unwrap();
        let targets = roots.targets(false, false).unwrap();
        let report = super::clean_targets_with_recheck(&targets, || {});
        assert!(
            !report.iter().any(|item| item.status == "failed"),
            "{report:?}"
        );
        assert!(!owned.exists());
        assert!(!roots.home.join(".codex/auth.json").exists());
        for (path, bytes) in &shared {
            assert_eq!(
                &fs::read(path).unwrap(),
                bytes,
                "shared data changed: {}",
                path.display()
            );
        }
    }

    #[test]
    fn codex_and_ccswitch_purge_flags_are_independent_and_exact() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let codex = roots.targets(true, false).unwrap();
        let switch = roots.targets(false, true).unwrap();
        for expected in [
            roots.home.join(".codex"),
            roots.home.join(".agents/skills"),
            roots.local.join("Packages/OpenAI.Codex_2p2nqsd0c76g0"),
            roots.local.join("OpenAI/Codex"),
            roots.local.join("Codex/Logs"),
            roots.roaming.join("OpenAI/Codex"),
            roots.roaming.join("Codex"),
        ] {
            assert!(codex.contains(&expected));
            assert!(!switch.contains(&expected));
        }
        for expected in [
            roots.home.join(".cc-switch"),
            roots.local.join("com.ccswitch.desktop"),
            roots.roaming.join("com.ccswitch.desktop"),
        ] {
            assert!(switch.contains(&expected));
            assert!(!codex.contains(&expected));
        }
        assert!(!codex.contains(&roots.local.join("Codex")));
        assert!(!codex.contains(&roots.local.join("Packages")));
    }

    #[test]
    fn final_scope_recheck_cleans_profile_recreated_during_exit() {
        let root = TempRoot::new();
        let target = root.0.join(".codex");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("initial"), b"old").unwrap();
        let report = clean_targets_with_recheck(std::slice::from_ref(&target), || {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("recreated"), b"late writer").unwrap();
        });
        assert_eq!(report[0].status, "removed");
        assert!(report[0].detail.contains("最终复核"));
        assert!(verify_absent(&target).is_ok());
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn directory_pin_prevents_rename_and_readonly_handle_refuses_hardlinks() {
        let root = TempRoot::new();
        let directory = root.0.join("pinned");
        fs::create_dir(&directory).unwrap();
        let pin = pin_directory(&directory).unwrap();
        assert!(fs::rename(&directory, root.0.join("moved")).is_err());
        drop(pin);
        fs::rename(&directory, root.0.join("moved")).unwrap();
        let first = root.0.join("first");
        let second = root.0.join("second");
        fs::write(&first, b"same fixture").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let mut permissions = fs::metadata(&first).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&first, permissions).unwrap();
        assert!(clear_owned_readonly_attribute(&first).is_err());
        assert!(fs::metadata(&second).unwrap().permissions().readonly());
        let mut permissions = fs::metadata(&first).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&first, permissions).unwrap();
    }

    #[test]
    fn outstanding_policy_or_volume_journal_prevents_cleanup() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        assert!(ensure_journals_recovered(&roots).is_ok());
        let journals = roots.local.join("CodexUsbAppx");
        fs::create_dir(&journals).unwrap();
        fs::write(journals.join("volume-any.json.bak"), b"unrecovered").unwrap();
        assert!(ensure_journals_recovered(&roots).is_err());
        fs::remove_file(journals.join("volume-any.json.bak")).unwrap();
        assert!(ensure_journals_recovered(&roots).is_ok());
    }

    #[test]
    fn typed_plan_rejects_unknown_deletion_fields() {
        assert!(serde_json::from_value::<CleanupPlan>(
            serde_json::json!({"schemaVersion":1,"deletePaths":["C:\\"]})
        )
        .is_err());
    }

    #[test]
    fn media_snapshot_covers_manager_created_license_and_atomic_backup() {
        let root = TempRoot::new();
        let bundle = root.0.join("bundle");
        fs::create_dir(&bundle).unwrap();
        fs::write(bundle.join("SHA256SUMS.txt"), b"immutable sums").unwrap();
        fs::write(bundle.join("usb-manifest.json"), b"immutable manifest").unwrap();
        fs::write(bundle.join("usb-license.json"), b"license one").unwrap();
        fs::write(bundle.join("usb-license.json.bak"), b"license backup").unwrap();

        let first = media_snapshot(&bundle).unwrap();
        assert!(first.license_sha256.is_some());
        assert!(first.license_backup_sha256.is_some());
        fs::write(bundle.join("usb-license.json"), b"license two").unwrap();
        let changed = media_snapshot(&bundle).unwrap();
        assert_ne!(first, changed);

        fs::remove_file(bundle.join("usb-license.json")).unwrap();
        fs::remove_file(bundle.join("usb-license.json.bak")).unwrap();
        let absent = media_snapshot(&bundle).unwrap();
        assert!(absent.license_sha256.is_none());
        assert!(absent.license_backup_sha256.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn stale_helper_sweep_requires_typed_identity_and_preserves_unknown_files() {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let nonce = uuid::Uuid::new_v4().to_string();
        let known = roots.temp.join(format!("{HELPER_PREFIX}{nonce}"));
        fs::create_dir(&known).unwrap();
        let executable = known.join(HELPER_EXE);
        fs::write(&executable, b"synthetic executable").unwrap();
        let mut plan = CleanupPlan {
            schema_version: 1,
            nonce,
            parent_pid: std::process::id(),
            parent_creation_time: process_creation_time(unsafe { GetCurrentProcess() }).unwrap(),
            source_executable: root.0.join("never-executed-old-source.exe"),
            executable_sha256: sha256(&executable).unwrap(),
            roots: roots.clone(),
            purge_user_data: false,
            purge_ccswitch_data: false,
            force_shared_cleanup: false,
            remove_usb_media: false,
            source_volume: None,
            source_media: None,
            source_license: None,
            prior_notes: Vec::new(),
        };
        fs::write(known.join("plan.json"), serde_json::to_vec(&plan).unwrap()).unwrap();
        assert!(
            validate_stale_helper(&known, &roots).is_err(),
            "Live original process must block sweep"
        );
        plan.parent_creation_time = 0; // Same PID with different start time is a different process.
        fs::write(known.join("plan.json"), serde_json::to_vec(&plan).unwrap()).unwrap();
        assert!(validate_stale_helper(&known, &roots).is_ok());
        let unknown = roots
            .temp
            .join(format!("{HELPER_PREFIX}{}", uuid::Uuid::new_v4()));
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("unrelated.txt"), b"keep").unwrap();
        let empty = roots
            .temp
            .join(format!("{HELPER_PREFIX}{}", uuid::Uuid::new_v4()));
        fs::create_dir(&empty).unwrap();
        let results = clean_stale_helpers(&roots, &roots.temp.join("current")).unwrap();
        assert!(results
            .iter()
            .any(|item| item.path == known.display().to_string() && item.status == "removed"));
        assert!(results
            .iter()
            .any(|item| item.path == unknown.display().to_string() && item.status == "failed"));
        assert!(results
            .iter()
            .any(|item| item.path == empty.display().to_string() && item.status == "removed"));
        assert!(!known.exists());
        assert!(!empty.exists());
        assert_eq!(fs::read(unknown.join("unrelated.txt")).unwrap(), b"keep");
    }

    #[test]
    fn unstarted_helper_raii_cleans_only_files_created_by_handoff() {
        let root = TempRoot::new();
        let directory = root.0.join("helper");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(HELPER_EXE), b"own").unwrap();
        fs::write(directory.join("plan.json"), b"own").unwrap();
        fs::write(directory.join("unrelated"), b"keep").unwrap();
        drop(UnstartedHelper {
            directory: directory.clone(),
            armed: true,
        });
        assert!(!directory.join(HELPER_EXE).exists());
        assert!(!directory.join("plan.json").exists());
        assert_eq!(fs::read(directory.join("unrelated")).unwrap(), b"keep");
    }

    #[cfg(windows)]
    #[test]
    fn readonly_owned_file_is_removed_without_acl_changes() {
        let root = TempRoot::new();
        let target = root.0.join("readonly.bin");
        fs::write(&target, b"owned").unwrap();
        let mut permission = fs::metadata(&target).unwrap().permissions();
        permission.set_readonly(true);
        fs::set_permissions(&target, permission).unwrap();
        assert_eq!(clean_target(&target).status, "removed");
        assert!(!target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn junction_tree_and_ancestor_are_refused_without_touching_destination() {
        use std::os::windows::process::CommandExt;
        let root = TempRoot::new();
        let owned = root.0.join("owned");
        let outside = root.0.join("outside");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let link = owned.join("redirect");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(check_tree(&owned).is_err());
        assert!(check_ancestors(&link.join("keep.txt")).is_err());
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
        fs::remove_dir(&link).unwrap();
        assert_eq!(clean_target(&owned).status, "removed");
    }

    #[cfg(windows)]
    #[test]
    fn legacy_helper_active_pid_is_preserved() {
        let root = TempRoot::new();
        let path = root
            .0
            .join(format!("codex-usb-uac-{}-123.ps1", std::process::id()));
        fs::write(&path, b"active").unwrap();
        assert!(reject_live_legacy_helper(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), b"active");
        assert_eq!(
            legacy_helper_owner("codex-win-engine-ps-123-1788345678912345678.ps1"),
            Some(123)
        );
        assert_eq!(
            legacy_helper_owner("codex-win-engine-ps-123-important.ps1"),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_is_plain_bounded_and_parses_in_windows_powershell() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let script = finalizer_script(
            Path::new(r"C:\Temp\codex-uninstall-helper-test"),
            Path::new(r"C:\Users\test\Desktop\report.json"),
            "fixture-lifetime-token",
            false,
        );
        assert!(!script.contains("-Recurse"));
        assert!(!script.contains("Stop-Process"));
        assert!(!script.contains("StartTime"));
        assert!(script.contains("[IO.File]::ReadAllText($lock)"));
        assert!(script.contains("helperFinalization='removed'"));
        assert!(script.contains("Publish-FinalizerReport"));
        assert!(script.contains("Get-ReportIssueSummary"));
        assert!(script.contains("具体原因/保留项"));
        assert!(
            script.contains("Move-Item -LiteralPath $pendingReport -Destination $report -Force")
        );
        assert!(script.contains("Staged finalizer report did not confirm helper removal"));
        assert!(script.contains("finalizer.ready"));
        assert!(script.contains("finalizer.error.txt"));
        assert!(script.contains("exit 2"));
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(shell).args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::InputEncoding=[Text.UTF8Encoding]::new($false);[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$tokens=$null;$errors=$null;[void][System.Management.Automation.Language.Parser]::ParseInput([Console]::In.ReadToEnd(),[ref]$tokens,[ref]$errors);if($errors.Count -gt 0){$errors | Out-String | Write-Error;exit 1}"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).creation_flags(0x08000000).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                let mut error = Vec::new();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_end(&mut error)
                    .unwrap();
                assert!(status.success(), "{}", String::from_utf8_lossy(&error));
                break;
            }
            if start.elapsed() > Duration::from_secs(15) {
                let _ = child.kill();
                panic!("PowerShell parser test timeout");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_stdin_bootstrap_executes_utf8_paths() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        let root = TempRoot::new();
        let marker = root.0.join("中文-finalizer-路径 & ampersand.txt");
        let script = format!(
            "Set-Content -LiteralPath '{}' -Value 'stdin-utf8' -Encoding UTF8",
            marker.display().to_string().replace('\'', "''")
        );
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(shell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                FINALIZER_STDIN_BOOTSTRAP,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = fs::read_to_string(marker).unwrap();
        assert_eq!(value.trim_start_matches('\u{feff}').trim(), "stdin-utf8");
    }

    #[cfg(windows)]
    #[test]
    fn encoded_finalizer_command_round_trips_utf16le() {
        use base64::Engine as _;

        let script = "Write-Output '中文路径 & ampersand'";
        let encoded = powershell_encoded_command(script);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
        let decoded = String::from_utf16(units.collect::<Vec<_>>().as_slice()).unwrap();
        assert_eq!(decoded, script);
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_failure_has_exact_reboot_fallback_without_recursive_delete() {
        let root = TempRoot::new();
        let helper = root.0.join("helper");
        fs::create_dir(&helper).unwrap();
        let report = root.0.join("report.json");
        let script = finalizer_script(&helper, &report, &uuid::Uuid::new_v4().to_string(), false);
        assert!(script.contains("Register-HelperRebootCleanup"));
        assert!(script.contains("MoveFileEx"));
        assert!(script.contains("Assert-KnownHelperEntries"));
        assert!(script.contains("$rebootStatus"));
        assert!(script.contains("'deferred'"));
        assert!(!script.contains("Remove-Item -Recurse"));
        assert!(!script.contains("Get-ChildItem -Recurse"));
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_catch_publishes_error_before_registering_reboot_fallback() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        let root = TempRoot::new();
        let helper = root.0.join("codex-uninstall-helper");
        fs::create_dir(&helper).unwrap();
        for name in [HELPER_EXE, "plan.json", "ready.json"] {
            fs::write(helper.join(name), b"fixture").unwrap();
        }
        let report_path = root.0.join("report.json");
        fs::write(
            &report_path,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: None,
                retry_suggestion: "retry fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "temp test".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let token = uuid::Uuid::new_v4().to_string();
        let lifetime_lock = create_lifetime_lock(&helper, &token).unwrap();
        let reboot_calls = root.0.join("reboot-calls.txt");
        let reboot_calls_literal = reboot_calls.display().to_string().replace('\'', "''");
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let generated = finalizer_script(&helper, &report_path, &token, false)
            .replace("$attempt -lt 40", "$attempt -lt 1");
        let script = format!(
            r#"
Add-Type -TypeDefinition @'
using System;
using System.IO;
public static class CodexUsbUninstallRebootDelete {{
  public static string LogPath;
  public static void Delete(string path) {{ File.AppendAllText(LogPath, path + Environment.NewLine); }}
}}
'@
[CodexUsbUninstallRebootDelete]::LogPath='{reboot_calls_literal}'
function Remove-Item {{
  param([string]$LiteralPath,[switch]$Force,[string]$ErrorAction)
  if($LiteralPath -like '*CodexUninstallCleanup.exe') {{ throw 'injected finalizer delete failure' }}
  Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Force -ErrorAction $ErrorAction
}}
{generated}
"#,
            reboot_calls_literal = reboot_calls_literal,
        );
        let mut finalizer = Command::new(shell)
            .current_dir(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let ready = helper.join("finalizer.ready");
        let start = Instant::now();
        while !ready.exists() {
            if let Some(status) = finalizer.try_wait().unwrap() {
                panic!("finalizer exited before ready: {status}");
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "finalizer ready timeout"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(lifetime_lock);
        let output = finalizer.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{}", output.status);
        let bytes = fs::read(&report_path).unwrap();
        let report: CleanupReport =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .unwrap_or_else(|error| {
                    panic!(
                        "report parse failed: {error}; bytes={:?}; stderr={}",
                        bytes,
                        String::from_utf8_lossy(&output.stderr)
                    )
                });
        assert_eq!(report.status, "partialFailure");
        assert!(report
            .helper_finalization
            .contains("injected finalizer delete failure"));
        assert!(
            report
                .items
                .iter()
                .any(|item| item.path.ends_with("codex-uninstall-helper")
                    && matches!(item.status.as_str(), "deferred" | "failed")),
            "{report:?}"
        );
        assert!(helper.join("finalizer.error.txt").exists());
        let reboot_log = fs::read_to_string(&reboot_calls).unwrap();
        assert!(reboot_log.contains("finalizer.error.txt"), "{reboot_log}");
    }

    #[test]
    fn report_issue_summary_lists_bounded_failure_paths() {
        let report = CleanupReport {
            schema_version: 1,
            status: "partialFailure".into(),
            phase: "failed".into(),
            exit_code: Some(2),
            retry_suggestion: "retry fixture".into(),
            items: vec![
                CleanupItem {
                    path: r"C:\locked\usb-license.json".into(),
                    status: "preserved".into(),
                    detail: "许可证身份校验失败\n保留以便重试".into(),
                },
                CleanupItem {
                    path: "ignored-note".into(),
                    status: "note".into(),
                    detail: "not an issue".into(),
                },
            ],
            helper_finalization: "failed: fixture".into(),
            note: "fixture".into(),
        };
        let summary = report_issue_summary(&report).unwrap();
        assert!(summary.contains("具体原因/保留项"));
        assert!(summary.contains(r"C:\locked\usb-license.json"));
        assert!(summary.contains("许可证身份校验失败 保留以便重试"));
        assert!(!summary.contains("not an issue"));
    }

    #[test]
    fn durable_finalizer_report_wins_when_transient_ready_marker_was_missed() {
        let root = TempRoot::new();
        let report = root.0.join("report.json");
        fs::write(
            &report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "completed".into(),
                phase: "completed".into(),
                exit_code: Some(0),
                retry_suggestion: String::new(),
                items: Vec::new(),
                helper_finalization: "removed".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(finalizer_report_completed(&report).unwrap());

        let partial = root.0.join("partial.json");
        fs::write(
            &partial,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "partialFailure".into(),
                phase: "completed".into(),
                exit_code: Some(0),
                retry_suggestion: "retry fixture".into(),
                items: vec![CleanupItem {
                    path: "fixture".into(),
                    status: "failed".into(),
                    detail: "locked".into(),
                }],
                helper_finalization: "removed".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(finalizer_report_completed(&partial).unwrap());
    }

    #[test]
    fn incomplete_finalizer_report_is_not_accepted_as_success() {
        let root = TempRoot::new();
        let report = root.0.join("report.json");
        fs::write(
            &report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: None,
                retry_suggestion: "wait fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!finalizer_report_completed(&report).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn successful_child_exit_repairs_only_after_helper_directory_is_gone() {
        let root = TempRoot::new();
        let roots = fake_roots(&root.0);
        let nonce = uuid::Uuid::new_v4().to_string();
        let helper = roots.temp.join(format!("{HELPER_PREFIX}{nonce}"));
        let report = report_path(&roots, &nonce);
        fs::write(
            &report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: None,
                retry_suggestion: "wait fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();

        // This is the only state in which the recovery is allowed: the
        // successful finalizer process has already removed its exact helper
        // directory, but the report still contains the pre-finalization
        // state.  The report becomes a durable terminal proof after repair.
        assert!(repair_successful_finalizer_report(&helper, &report, &roots, &nonce).unwrap());
        assert!(finalizer_report_completed(&report).unwrap());
        let bytes = fs::read(&report).unwrap();
        let repaired: CleanupReport = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(repaired.helper_finalization, "removed");
        assert_eq!(repaired.status, "completed");
        assert_eq!(repaired.phase, "completed");
        assert_eq!(repaired.exit_code, Some(0));
        assert!(repair_successful_finalizer_report(&helper, &report, &roots, &nonce).unwrap());

        // A still-present helper directory is not safe to upgrade.  This
        // prevents an exit-code-only signal from masking a real residual.
        let present_nonce = uuid::Uuid::new_v4().to_string();
        let helper_present = roots.temp.join(format!("{HELPER_PREFIX}{present_nonce}"));
        fs::create_dir(&helper_present).unwrap();
        let present_report = report_path(&roots, &present_nonce);
        fs::write(
            &present_report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: None,
                retry_suggestion: "wait fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!repair_successful_finalizer_report(
            &helper_present,
            &present_report,
            &roots,
            &present_nonce
        )
        .unwrap());
        let bytes = fs::read(&present_report).unwrap();
        let untouched: CleanupReport = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(untouched.helper_finalization, "pending");

        // A non-terminal report with an existing non-zero exit code is never
        // rewritten as success, even when the helper directory is absent.
        let failed_nonce = uuid::Uuid::new_v4().to_string();
        let failed_helper = roots.temp.join(format!("{HELPER_PREFIX}{failed_nonce}"));
        let failed_report = report_path(&roots, &failed_nonce);
        fs::write(
            &failed_report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: Some(2),
                retry_suggestion: "retry fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!repair_successful_finalizer_report(
            &failed_helper,
            &failed_report,
            &roots,
            &failed_nonce
        )
        .unwrap());
        let bytes = fs::read(&failed_report).unwrap();
        let failed: CleanupReport = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(failed.exit_code, Some(2));

        let malformed_nonce = uuid::Uuid::new_v4().to_string();
        let malformed_helper = roots.temp.join(format!("{HELPER_PREFIX}{malformed_nonce}"));
        let malformed_report = report_path(&roots, &malformed_nonce);
        fs::write(
            &malformed_report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "completed".into(),
                phase: "completed".into(),
                exit_code: None,
                retry_suggestion: "inconsistent fixture".into(),
                items: vec![CleanupItem {
                    path: "locked-item".into(),
                    status: "failed".into(),
                    detail: "fixture failure".into(),
                }],
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!repair_successful_finalizer_report(
            &malformed_helper,
            &malformed_report,
            &roots,
            &malformed_nonce
        )
        .unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_report_confirmation_retries_transient_incomplete_state() {
        let root = TempRoot::new();
        let report = root.0.join("report.json");
        fs::write(
            &report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "finalizing".into(),
                phase: "finalizing".into(),
                exit_code: None,
                retry_suggestion: "wait fixture".into(),
                items: Vec::new(),
                helper_finalization: "pending".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let delayed_report = report.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            fs::write(
                delayed_report,
                serde_json::to_vec(&CleanupReport {
                    schema_version: 1,
                    status: "completed".into(),
                    phase: "completed".into(),
                    exit_code: Some(0),
                    retry_suggestion: String::new(),
                    items: Vec::new(),
                    helper_finalization: "removed".into(),
                    note: "fixture".into(),
                })
                .unwrap(),
            )
            .unwrap();
        });
        assert!(confirm_finalizer_report(&report, None).is_ok());
        writer.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn terminal_report_is_not_ready_while_helper_directory_remains() {
        let root = TempRoot::new();
        let report = root.0.join("report.json");
        let helper = root.0.join("helper");
        fs::create_dir(&helper).unwrap();
        fs::write(
            &report,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: "completed".into(),
                phase: "completed".into(),
                exit_code: Some(0),
                retry_suggestion: String::new(),
                items: Vec::new(),
                helper_finalization: "removed".into(),
                note: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!finalizer_completion_ready(&report, Some(&helper)).unwrap());
        fs::remove_dir(&helper).unwrap();
        assert!(finalizer_completion_ready(&report, Some(&helper)).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_is_failed_not_silently_reported_absent() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = TempRoot::new();
        let target = root.0.join("locked");
        fs::write(&target, b"keep").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        assert_eq!(clean_target(&target).status, "failed");
        assert!(target.exists());
        drop(held);
        assert_eq!(clean_target(&target).status, "removed");
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_removes_only_synthetic_helper_after_lifetime_lock_release() {
        run_synthetic_finalizer("finalizing", false, false, false);
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_preserves_prior_partial_failure_after_successful_self_removal() {
        run_synthetic_finalizer("partialFailure", false, false, false);
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_retains_unknown_helper_files_and_reports_failure() {
        run_synthetic_finalizer("finalizing", true, false, false);
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_rejects_lifetime_token_mismatch_without_deleting_helper_files() {
        run_synthetic_finalizer("finalizing", false, true, false);
    }

    #[cfg(windows)]
    #[test]
    fn finalizer_never_displays_success_when_report_write_and_failure_read_both_fail() {
        run_synthetic_finalizer("finalizing", false, false, true);
    }

    #[cfg(windows)]
    fn run_synthetic_finalizer(
        initial_status: &str,
        unknown_file: bool,
        token_mismatch: bool,
        report_io_failure: bool,
    ) {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let root = TempRoot::new();
        let helper = root.0.join("synthetic-helper");
        fs::create_dir(&helper).unwrap();
        for name in [HELPER_EXE, "plan.json", "ready.json"] {
            fs::write(helper.join(name), b"fixture").unwrap();
        }
        if unknown_file {
            fs::write(helper.join("unknown-user-file.txt"), b"keep unknown").unwrap();
        }
        let report_path = root.0.join("report.json");
        fs::write(
            &report_path,
            serde_json::to_vec(&CleanupReport {
                schema_version: 1,
                status: initial_status.into(),
                phase: initial_status.into(),
                exit_code: None,
                retry_suggestion: "retry fixture".into(),
                items: if initial_status == "partialFailure" {
                    vec![CleanupItem {
                        path: "fixture-owned-cache".into(),
                        status: "failed".into(),
                        detail: "file locked".into(),
                    }]
                } else {
                    Vec::new()
                },
                helper_finalization: "pending".into(),
                note: "temp test".into(),
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(root.0.join("unrelated.txt"), b"keep").unwrap();
        let actual_token = uuid::Uuid::new_v4().to_string();
        let expected_token = if token_mismatch {
            uuid::Uuid::new_v4().to_string()
        } else {
            actual_token.clone()
        };
        let lifetime_lock = create_lifetime_lock(&helper, &actual_token).unwrap();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut script =
            finalizer_script(&helper, &report_path, &expected_token, report_io_failure);
        let visible_summary = root.0.join("captured-summary.txt");
        if report_io_failure {
            // No real UI or user data: inject precisely the late report I/O
            // failure, and capture the Popup text inside this UUID fixture.
            let capture = visible_summary.display().to_string().replace('\'', "''");
            script = format!(
                r#"
$script:reportReads=0
$script:summaryCapture='{capture}'
function Get-Content {{
  param([string]$LiteralPath, [switch]$Raw, [string]$Encoding)
  $script:reportReads++
  if($script:reportReads -gt 1) {{ throw 'injected report reread failure' }}
  Microsoft.PowerShell.Management\Get-Content -LiteralPath $LiteralPath -Raw -Encoding UTF8
}}
function Set-Content {{ throw 'injected report write failure' }}
function New-Object {{
  param([string]$ComObject)
  if($ComObject -ne 'WScript.Shell') {{ throw 'Unexpected test COM request' }}
  $fake=[pscustomobject]@{{}}
  Add-Member -InputObject $fake -MemberType ScriptMethod -Name Popup -Value {{
    param($message,$seconds,$title,$flags)
    [IO.File]::WriteAllText($script:summaryCapture,[string]$message)
    return 0
  }}
  return $fake
}}
{script}
"#
            );
        }
        let mut finalizer = Command::new(shell)
            .current_dir(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        if !unknown_file {
            let ready = helper.join("finalizer.ready");
            let start = Instant::now();
            loop {
                if ready.exists() {
                    assert_eq!(fs::read_to_string(&ready).unwrap(), expected_token);
                    break;
                }
                if let Some(status) = finalizer.try_wait().unwrap() {
                    panic!("finalizer exited before ready: {status}");
                }
                if start.elapsed() > Duration::from_secs(10) {
                    let _ = finalizer.kill();
                    panic!("Synthetic finalizer ready timeout");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        drop(lifetime_lock);
        let start = Instant::now();
        loop {
            if let Some(status) = finalizer.try_wait().unwrap() {
                if unknown_file || token_mismatch || report_io_failure {
                    assert_eq!(status.code(), Some(2), "{status}");
                } else {
                    assert!(status.success(), "{status}");
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(20) {
                let _ = finalizer.kill();
                panic!("Synthetic finalizer timed out");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let report_bytes = fs::read(&report_path).unwrap();
        let report: CleanupReport = serde_json::from_slice(
            report_bytes
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(&report_bytes),
        )
        .unwrap();
        if report_io_failure {
            assert!(
                !helper.exists(),
                "helper itself was removed before report I/O failed"
            );
            assert_eq!(
                report.status, "finalizing",
                "durable report must not claim completed"
            );
            assert_eq!(report.helper_finalization, "pending");
            let displayed = fs::read_to_string(visible_summary).unwrap();
            assert!(!displayed.contains("已完成所选范围"), "{displayed}");
            assert!(displayed.contains("部分卸载清理未完成"), "{displayed}");
            assert!(
                displayed.contains("injected report write failure"),
                "{displayed}"
            );
        } else if unknown_file || token_mismatch {
            assert!(
                report.helper_finalization.starts_with("failed:"),
                "{report:?}"
            );
            assert_eq!(report.status, "partialFailure");
            assert!(helper.exists());
            if token_mismatch {
                assert!(report
                    .helper_finalization
                    .contains("lifetime token mismatch"));
                for name in [HELPER_EXE, "plan.json", "ready.json"] {
                    assert_eq!(fs::read(helper.join(name)).unwrap(), b"fixture");
                }
            } else {
                assert!(
                    report
                        .helper_finalization
                        .contains("Unexpected cleanup helper file"),
                    "{report:?}"
                );
                assert_eq!(
                    fs::read(helper.join("unknown-user-file.txt")).unwrap(),
                    b"keep unknown"
                );
            }
        } else {
            assert_eq!(report.helper_finalization, "removed", "{report:?}");
            assert_eq!(
                report.status,
                if initial_status == "partialFailure" {
                    "partialFailure"
                } else {
                    "completed"
                }
            );
            assert!(!helper.exists());
        }
        if initial_status == "partialFailure" {
            assert_eq!(report.items[0].status, "failed");
            assert_eq!(report.items[0].detail, "file locked");
        }
        assert_eq!(fs::read(root.0.join("unrelated.txt")).unwrap(), b"keep");
    }
}
