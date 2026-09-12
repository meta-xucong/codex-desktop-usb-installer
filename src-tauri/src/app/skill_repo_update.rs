//! Independent, unsigned Skill Hub update layer.
//!
//! This module deliberately does not depend on USB guards, capability-pack
//! installation, or the Codex payload updater.  The only shared primitive is
//! the manager's OperationManager lock, supplied by the command boundary.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use zip::ZipArchive;

use crate::app::oplock::OperationManager;
use crate::errors::CommandError;

const REPO: &str = "meta-xucong/skill_hub";
const BRANCH: &str = "master";
const RELAY_META: &str = "https://update.404token.xyz/__codex_update__/skill-hub/latest.json";
const RELAY_ARCHIVE: &str = "https://update.404token.xyz/__codex_update__/skill-hub/archive.zip";
const GITHUB_COMMITS: &str = "https://api.github.com/repos/meta-xucong/skill_hub/commits/master";
const GITHUB_ARCHIVE: &str = "https://github.com/meta-xucong/skill_hub/archive/";
const MAX_ARCHIVE: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_UNPACKED: usize = 256 * 1024 * 1024;
const MAX_FILE: usize = 32 * 1024 * 1024;
const NETWORK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const NETWORK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const NETWORK_OPERATION_TIMEOUT: Duration = Duration::from_secs(45);
const BASE_MANIFEST_SHA256: &str =
    "e6c6a610b4bab5dc9001b3846c087bdaba83fcd8c26908fd6eabc135f4ca2a9c";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Relay,
    Github,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoExpected {
    pub schema_version: u8,
    pub repo: String,
    pub branch: String,
    pub commit: String,
    pub source_kind: SourceKind,
    pub archive_size: u64,
    pub archive_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoUpdateInfo {
    pub schema_version: u8,
    pub current_commit: Option<String>,
    pub latest_commit: String,
    pub source_kind: SourceKind,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub expected: SkillRepoExpected,
    pub update_available: bool,
    pub installable_count: u32,
    pub conflict_count: u32,
    pub skipped_count: u32,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoItem {
    pub id: String,
    pub status: String,
    pub code: Option<String>,
    pub file_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoApplyReport {
    pub schema_version: u8,
    pub status: String,
    pub identity: SkillRepoExpected,
    pub applied: Vec<SkillRepoItem>,
    pub conflicts: Vec<SkillRepoItem>,
    pub skipped: Vec<SkillRepoItem>,
    pub failed: Vec<SkillRepoItem>,
    pub last_good_generation: Option<String>,
    pub reloaded_required: bool,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoRollbackReport {
    pub schema_version: u8,
    pub status: String,
    pub restored_generation: Option<String>,
    pub restored_skills: Vec<String>,
    pub reloaded_required: bool,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileRecord {
    path: String,
    size: u64,
    sha256: String,
    volume_id: Option<String>,
    file_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillState {
    installed_commit: String,
    directory_volume_id: Option<String>,
    directory_file_id: Option<String>,
    files: Vec<FileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFile {
    schema_version: u8,
    repo: String,
    branch: String,
    last_seen_commit: Option<String>,
    last_apply_status: String,
    last_good_generation: Option<String>,
    skills: BTreeMap<String, SkillState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BaseManifest {
    schema_version: u8,
    source: String,
    source_sha256: String,
    skills: Vec<BaseSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseSkill {
    id: String,
}

#[derive(Debug, Clone)]
struct CandidateSkill {
    id: String,
    files: BTreeMap<String, Vec<u8>>,
}

type GenerationFile = FileRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerationSkill {
    id: String,
    installed_commit: String,
    backup_relative: Option<String>,
    directory_volume_id: Option<String>,
    directory_file_id: Option<String>,
    files: Vec<GenerationFile>,
    new_files: Vec<GenerationFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Generation {
    schema_version: u8,
    generation_id: String,
    commit: String,
    complete: bool,
    previous_state_file: Option<String>,
    previous_state_sha256: Option<String>,
    skills: Vec<GenerationSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalSkill {
    id: String,
    action: String,
    target_relative: String,
    backup_relative: Option<String>,
    rollback_relative: Option<String>,
    old_installed_commit: Option<String>,
    new_installed_commit: String,
    old_directory_volume_id: Option<String>,
    old_directory_file_id: Option<String>,
    new_directory_volume_id: Option<String>,
    new_directory_file_id: Option<String>,
    old_files: Vec<FileRecord>,
    new_files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u8,
    transaction_id: String,
    phase: String,
    expected_identity: SkillRepoExpected,
    previous_state_sha256: Option<String>,
    previous_state_file: Option<String>,
    backup_generation: String,
    rollback_root: String,
    skills: Vec<JournalSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackJournalSkill {
    id: String,
    old_backup_relative: Option<String>,
    old_directory_volume_id: Option<String>,
    old_directory_file_id: Option<String>,
    old_files: Vec<FileRecord>,
    new_directory_volume_id: Option<String>,
    new_directory_file_id: Option<String>,
    new_files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackJournal {
    schema_version: u8,
    transaction_id: String,
    phase: String,
    generation_id: String,
    current_state_file: String,
    current_state_sha256: String,
    skills: Vec<RollbackJournalSkill>,
}

#[derive(Debug, Clone)]
struct HotError {
    code: &'static str,
    message: String,
    detail: Option<String>,
}

impl HotError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl From<HotError> for CommandError {
    fn from(value: HotError) -> Self {
        Self {
            code: value.code.to_string(),
            message: value.message,
            detail: value.detail,
        }
    }
}

fn invalid(message: impl Into<String>) -> HotError {
    HotError::new("SKILL_REPO_ARCHIVE_INVALID", message)
}
fn io_error(context: &str, error: impl std::fmt::Display) -> HotError {
    HotError::new(
        "SKILL_REPO_LEDGER_INVALID",
        format!("{context}，请打开日志查看具体失败项"),
    )
    .with_detail(format!("{context}: {error}"))
}

fn home_dir() -> Result<PathBuf, HotError> {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| io_error("无法定位用户目录", "UserDirs unavailable"))
}

fn state_dir() -> Result<PathBuf, HotError> {
    Ok(home_dir()?.join(".codex").join("skill-hub-updater"))
}
fn target_root() -> Result<PathBuf, HotError> {
    Ok(home_dir()?.join(".agents").join("skills"))
}

fn sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex_lower(&h.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn empty_state() -> StateFile {
    StateFile {
        schema_version: 1,
        repo: REPO.into(),
        branch: BRANCH.into(),
        last_seen_commit: None,
        last_apply_status: "never".into(),
        last_good_generation: None,
        skills: BTreeMap::new(),
    }
}

fn path_syntax_is_safe(path: &Path) -> Result<(), HotError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(format!(
            "路径必须是绝对且无跳转: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
struct ParentPathGuards(Vec<windows_sys::Win32::Foundation::HANDLE>);

#[cfg(windows)]
impl Drop for ParentPathGuards {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        for handle in self.0.drain(..) {
            unsafe {
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(windows)]
fn guard_parent_chain(path: &Path) -> Result<ParentPathGuards, HotError> {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    path_syntax_is_safe(path)?;
    let mut existing = Vec::new();
    let mut current = path.parent();
    while let Some(candidate) = current {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) => {
                if metadata_is_reparse(&metadata) || !metadata.is_dir() {
                    return Err(invalid(format!(
                        "写入路径父链不是安全普通目录: {}",
                        candidate.display()
                    )));
                }
                existing.push(candidate.to_path_buf());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("读取写入路径父链失败", error)),
        }
        current = candidate.parent();
    }
    existing.reverse();
    let mut handles = Vec::with_capacity(existing.len());
    for candidate in existing {
        let wide = wide_path(&candidate);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            for handle in handles {
                unsafe {
                    CloseHandle(handle);
                }
            }
            return Err(io_error(
                "打开写入路径父链句柄失败",
                std::io::Error::last_os_error(),
            ));
        }
        handles.push(handle);
    }
    Ok(ParentPathGuards(handles))
}

#[cfg(not(windows))]
fn guard_parent_chain(_path: &Path) -> Result<(), HotError> {
    Ok(())
}

fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn metadata_optional(path: &Path) -> Result<Option<fs::Metadata>, HotError> {
    path_syntax_is_safe(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_reparse(&metadata) {
                return Err(invalid(format!("reparse path: {}", path.display())));
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("读取路径属性失败", error)),
    }
}

fn metadata_required(path: &Path, context: &str) -> Result<fs::Metadata, HotError> {
    metadata_optional(path)?
        .ok_or_else(|| io_error(context, format!("路径不存在: {}", path.display())))
}

fn ensure_directory(path: &Path, context: &str) -> Result<(), HotError> {
    let metadata = metadata_required(path, context)?;
    if !metadata.is_dir() {
        return Err(io_error(
            context,
            format!("路径不是目录: {}", path.display()),
        ));
    }
    Ok(())
}

/// Check every existing ancestor, including the leaf, before any path-based
/// filesystem operation.  This is intentionally conservative: an inaccessible
/// path is an error, not evidence that the path is absent.
fn ensure_safe_parent(path: &Path) -> Result<(), HotError> {
    path_syntax_is_safe(path)?;
    let mut current = Some(path);
    while let Some(candidate) = current {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) => {
                if metadata_is_reparse(&metadata) {
                    return Err(invalid(format!("reparse path: {}", candidate.display())));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("读取父链属性失败", error)),
        }
        current = candidate.parent();
    }
    Ok(())
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

fn sync_directory(path: &Path) -> Result<(), HotError> {
    ensure_safe_parent(path)?;
    ensure_directory(path, "flush 目录失败")?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FlushFileBuffers, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };
        let wide = wide_path(path);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_error("打开目录 flush 句柄失败", path.display()));
        }
        let flushed = unsafe { FlushFileBuffers(handle) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        if !flushed {
            return Err(io_error("FlushFileBuffers 失败", path.display()));
        }
        ensure_safe_parent(path)?;
        Ok(())
    }
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error("flush 目录失败", error))?;
        ensure_safe_parent(path)?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(())
    }
}

fn safe_create_dir_all(path: &Path, context: &str) -> Result<(), HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    if let Some(metadata) = metadata_optional(path)? {
        if !metadata.is_dir() {
            return Err(io_error(
                context,
                format!("路径不是目录: {}", path.display()),
            ));
        }
        sync_directory(path)?;
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|error| io_error(context, error))?;
    ensure_safe_parent(path)?;
    ensure_directory(path, context)?;
    if let Some(parent) = path.parent() {
        if let Some(metadata) = metadata_optional(parent)? {
            if metadata.is_dir() {
                sync_directory(parent)?;
            }
        }
    }
    sync_directory(path)?;
    Ok(())
}

fn safe_read_file(path: &Path, context: &str) -> Result<Vec<u8>, HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    let metadata = metadata_required(path, context)?;
    if !metadata.is_file() {
        return Err(io_error(
            context,
            format!("路径不是普通文件: {}", path.display()),
        ));
    }
    let bytes = fs::read(path).map_err(|error| io_error(context, error))?;
    ensure_safe_parent(path)?;
    let after = metadata_required(path, context)?;
    if !after.is_file() {
        return Err(io_error(
            context,
            format!("文件类型在读取后变化: {}", path.display()),
        ));
    }
    Ok(bytes)
}

fn safe_write_new_file(path: &Path, bytes: &[u8], context: &str) -> Result<(), HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io_error(context, format!("文件路径无父目录: {}", path.display())))?;
    ensure_directory(parent, context)?;
    if metadata_optional(path)?.is_some() {
        return Err(io_error(
            context,
            format!("目标文件已存在: {}", path.display()),
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(context, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error(context, error))?;
    file.sync_all().map_err(|error| io_error(context, error))?;
    drop(file);
    ensure_safe_parent(path)?;
    let metadata = metadata_required(path, context)?;
    if !metadata.is_file() {
        return Err(io_error(
            context,
            format!("新建路径不是普通文件: {}", path.display()),
        ));
    }
    sync_directory(parent)?;
    Ok(())
}

fn same_volume(left: &Path, right: &Path) -> Result<(), HotError> {
    let (left_volume, _) = file_identity(left)?;
    let (right_volume, _) = file_identity(right)?;
    if left_volume != right_volume {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!(
                "rename 跨卷被拒绝: {} -> {}",
                left.display(),
                right.display()
            ),
        ));
    }
    Ok(())
}

fn safe_rename(source: &Path, destination: &Path, context: &str) -> Result<(), HotError> {
    ensure_safe_parent(source)?;
    ensure_safe_parent(destination)?;
    let _source_guards = guard_parent_chain(source)?;
    let _destination_guards = guard_parent_chain(destination)?;
    let source_parent = source
        .parent()
        .ok_or_else(|| io_error(context, "源路径无父目录"))?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| io_error(context, "目标路径无父目录"))?;
    ensure_directory(source_parent, context)?;
    ensure_directory(destination_parent, context)?;
    if metadata_optional(destination)?.is_some() {
        return Err(io_error(
            context,
            format!("rename 目标已存在: {}", destination.display()),
        ));
    }
    let source_identity = file_identity(source)?;
    same_volume(source_parent, destination_parent)?;
    fs::rename(source, destination).map_err(|error| io_error(context, error))?;
    ensure_safe_parent(source)?;
    ensure_safe_parent(destination)?;
    if metadata_optional(source)?.is_some() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("rename 源路径仍存在: {}", source.display()),
        ));
    }
    if file_identity(destination)? != source_identity {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("rename 后身份变化: {}", destination.display()),
        ));
    }
    sync_directory(source_parent)?;
    if destination_parent != source_parent {
        sync_directory(destination_parent)?;
    }
    Ok(())
}

fn safe_replace_file(source: &Path, destination: &Path, context: &str) -> Result<(), HotError> {
    ensure_safe_parent(source)?;
    ensure_safe_parent(destination)?;
    let _source_guards = guard_parent_chain(source)?;
    let _destination_guards = guard_parent_chain(destination)?;
    let source_parent = source
        .parent()
        .ok_or_else(|| io_error(context, "源路径无父目录"))?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| io_error(context, "目标路径无父目录"))?;
    ensure_directory(source_parent, context)?;
    ensure_directory(destination_parent, context)?;
    let source_identity = file_identity(source)?;
    if let Some(metadata) = metadata_optional(destination)? {
        if !metadata.is_file() {
            return Err(io_error(
                context,
                format!("替换目标不是普通文件: {}", destination.display()),
            ));
        }
    }
    same_volume(source_parent, destination_parent)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let source_wide = wide_path(source);
        let destination_wide = wide_path(destination);
        let ok = unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } != 0;
        if !ok {
            return Err(io_error(context, "MoveFileExW failed"));
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination).map_err(|error| io_error(context, error))?;
    }
    ensure_safe_parent(source)?;
    ensure_safe_parent(destination)?;
    if metadata_optional(source)?.is_some() || file_identity(destination)? != source_identity {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("原子替换后身份核对失败: {}", destination.display()),
        ));
    }
    sync_directory(destination_parent)?;
    Ok(())
}

fn safe_remove_file(path: &Path, context: &str) -> Result<(), HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    let Some(metadata) = metadata_optional(path)? else {
        return Ok(());
    };
    if !metadata.is_file() {
        return Err(io_error(
            context,
            format!("删除目标不是普通文件: {}", path.display()),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io_error(context, "删除路径无父目录"))?;
    ensure_directory(parent, context)?;
    fs::remove_file(path).map_err(|error| io_error(context, error))?;
    ensure_safe_parent(path)?;
    if metadata_optional(path)?.is_some() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("删除后文件仍存在: {}", path.display()),
        ));
    }
    sync_directory(parent)?;
    Ok(())
}

fn safe_remove_dir_all(path: &Path, context: &str) -> Result<(), HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    let Some(metadata) = metadata_optional(path)? else {
        return Ok(());
    };
    if !metadata.is_dir() {
        return Err(io_error(
            context,
            format!("删除目标不是目录: {}", path.display()),
        ));
    }
    let entries = fs::read_dir(path).map_err(|error| io_error(context, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(context, error))?;
        let child = entry.path();
        ensure_safe_parent(&child)?;
        let child_metadata = metadata_required(&child, context)?;
        if child_metadata.is_dir() {
            safe_remove_dir_all(&child, context)?;
        } else if child_metadata.is_file() {
            safe_remove_file(&child, context)?;
        } else {
            return Err(io_error(
                context,
                format!("删除目标包含特殊文件: {}", child.display()),
            ));
        }
    }
    ensure_safe_parent(path)?;
    fs::remove_dir(path).map_err(|error| io_error(context, error))?;
    ensure_safe_parent(path)?;
    if metadata_optional(path)?.is_some() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("删除后目录仍存在: {}", path.display()),
        ));
    }
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn safe_copy_file(source: &Path, destination: &Path, context: &str) -> Result<(), HotError> {
    let _source_guards = guard_parent_chain(source)?;
    let _destination_guards = guard_parent_chain(destination)?;
    let bytes = safe_read_file(source, context)?;
    safe_write_new_file(destination, &bytes, context)
}

fn read_state() -> Result<StateFile, HotError> {
    let path = state_dir()?.join("state.json");
    let Some(_) = metadata_optional(&path)? else {
        return Ok(empty_state());
    };
    let bytes = safe_read_file(&path, "读取热更新账本失败")?;
    let state: StateFile =
        serde_json::from_slice(&bytes).map_err(|e| io_error("热更新账本格式无效", e))?;
    validate_state_file(&state)?;
    Ok(state)
}

fn ensure_state_initialized() -> Result<(), HotError> {
    let path = state_path()?;
    if metadata_optional(&path)?.is_none() {
        let state = empty_state();
        write_atomic_json(&path, &state)?;
        let bytes = safe_read_file(&path, "读取初始热更新账本失败")?;
        let expected = b"{\"schemaVersion\":1,\"repo\":\"meta-xucong/skill_hub\",\"branch\":\"master\",\"lastSeenCommit\":null,\"lastApplyStatus\":\"never\",\"lastGoodGeneration\":null,\"skills\":{}}";
        if bytes != expected {
            return Err(HotError::new(
                "SKILL_REPO_LEDGER_INVALID",
                "初始热更新账本字节内容不匹配",
            ));
        }
    }
    Ok(())
}

fn write_atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), HotError> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("账本路径无父目录", path.display()))?;
    safe_create_dir_all(parent, "创建账本目录失败")?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(value).map_err(|e| io_error("序列化账本失败", e))?;
    safe_write_new_file(&tmp, &bytes, "创建账本临时文件失败")?;
    safe_replace_file(&tmp, path, "原子提交账本失败")?;
    let committed = safe_read_file(path, "核对原子提交账本失败")?;
    if committed != bytes {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("账本提交后字节内容变化: {}", path.display()),
        ));
    }
    Ok(())
}

fn base_manifest(app: &AppHandle) -> Result<BaseManifest, HotError> {
    let path = app
        .path()
        .resource_dir()
        .ok()
        .map(|p| p.join("skill_repo_base_manifest.json"))
        .ok_or_else(|| io_error("基础 Skill ID 资源缺失", "skill_repo_base_manifest.json"))?;
    let metadata = metadata_required(&path, "基础 Skill ID 资源缺失")?;
    if !metadata.is_file() {
        return Err(io_error(
            "基础 Skill ID 资源缺失",
            "skill_repo_base_manifest.json 不是普通文件",
        ));
    }
    let bytes = safe_read_file(&path, "读取基础 Skill ID 资源失败")?;
    let manifest: BaseManifest =
        serde_json::from_slice(&bytes).map_err(|e| io_error("基础 Skill ID 资源格式无效", e))?;
    if manifest.schema_version != 1
        || manifest.source != "installer-pack/manifest/skill-audit.json"
        || manifest.source_sha256 != BASE_MANIFEST_SHA256
    {
        return Err(io_error("基础 Skill ID 资源身份无效", "schema/source"));
    }
    let mut ids = HashSet::new();
    for item in &manifest.skills {
        if !valid_id(&item.id) || !ids.insert(item.id.clone()) {
            return Err(io_error("基础 Skill ID 资源包含重复或非法 ID", &item.id));
        }
    }
    Ok(manifest)
}

fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn file_identity(path: &Path) -> Result<(String, String), HotError> {
    ensure_safe_parent(path)?;
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::GENERIC_READ;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        let wide = wide_path(path);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_error("打开文件身份失败", path.display()));
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(handle, &mut info) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        if !ok {
            return Err(io_error("读取文件身份失败", path.display()));
        }
        if info.dwFileAttributes & 0x400 != 0 {
            return Err(invalid(format!("reparse path: {}", path.display())));
        }
        Ok((
            format!("{:08x}", info.dwVolumeSerialNumber),
            format!("{:08x}{:08x}", info.nFileIndexHigh, info.nFileIndexLow),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = metadata_required(path, "读取文件身份失败")?;
        Ok((meta.dev().to_string(), meta.ino().to_string()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let meta = metadata_required(path, "读取文件身份失败")?;
        Ok(("other".into(), format!("{}:{}", path.display(), meta.len())))
    }
}

fn record_file(root: &Path, rel: &str) -> Result<FileRecord, HotError> {
    let path = root.join(rel);
    let _parent_guards = guard_parent_chain(&path)?;
    let metadata = metadata_required(&path, "读取 Skill 文件失败")?;
    if !metadata.is_file() {
        return Err(invalid(format!("Skill 文件不是普通文件: {rel}")));
    }
    let bytes = safe_read_file(&path, "读取 Skill 文件失败")?;
    let (volume_id, file_id) = file_identity(&path)?;
    Ok(FileRecord {
        path: rel.into(),
        size: bytes.len() as u64,
        sha256: sha256(&bytes),
        volume_id: Some(volume_id),
        file_id: Some(file_id),
    })
}

fn list_files(root: &Path) -> Result<Vec<FileRecord>, HotError> {
    ensure_directory(root, "枚举 Skill 目录失败")?;
    let _parent_guards = guard_parent_chain(root)?;
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<FileRecord>) -> Result<(), HotError> {
        ensure_directory(dir, "枚举 Skill 目录失败")?;
        let entries = fs::read_dir(dir).map_err(|e| io_error("枚举 Skill 目录失败", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error("读取 Skill 目录项失败", e))?;
            let path = entry.path();
            ensure_safe_parent(&path)?;
            let rel = path
                .strip_prefix(root)
                .map_err(|e| io_error("计算 Skill 相对路径失败", e))?
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = metadata_required(&path, "读取 Skill 目录项失败")?;
            if metadata.is_dir() {
                walk(root, &path, out)?;
            } else if metadata.is_file() {
                out.push(record_file(root, &rel)?);
            } else {
                return Err(invalid(format!("Skill 目录包含特殊文件: {rel}")));
            }
        }
        ensure_safe_parent(dir)?;
        Ok(())
    }
    walk(root, root, &mut out)?;
    ensure_safe_parent(root)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn file_records_equal(root: &Path, records: &[FileRecord]) -> bool {
    file_records_match(root, records, true)
}

fn file_records_content_equal(root: &Path, records: &[FileRecord]) -> bool {
    file_records_match(root, records, false)
}

fn file_records_match(root: &Path, records: &[FileRecord], compare_identity: bool) -> bool {
    let Ok(actual) = list_files(root) else {
        return false;
    };
    actual.len() == records.len()
        && actual.iter().zip(records).all(|(a, b)| {
            a.path == b.path
                && a.size == b.size
                && a.sha256 == b.sha256
                && (!compare_identity || (a.volume_id == b.volume_id && a.file_id == b.file_id))
        })
}

fn normalize_zip_path(raw: &str, directory: bool) -> Result<String, HotError> {
    // The on-disk contract is deliberately ASCII-only: this makes archive
    // collision checks deterministic across Windows Unicode normalization and
    // case-folding behavior without adding a platform-specific normalizer.
    if !raw.is_ascii() {
        return Err(invalid(format!("归档路径必须是 ASCII: {raw}")));
    }
    let mut value = raw.replace('\\', "/");
    if directory {
        if !value.ends_with('/') {
            return Err(invalid("目录条目缺少末尾斜杠"));
        }
        value.pop();
    } else if value.ends_with('/') {
        return Err(invalid("文件条目带末尾斜杠"));
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains(':')
            || part.ends_with(' ')
            || part.ends_with('.')
            || part.contains('\0')
        {
            return Err(invalid(format!("非法归档路径: {raw}")));
        }
        let upper = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper[3..].parse::<u8>().is_ok()
        {
            return Err(invalid(format!("Windows 保留名: {raw}")));
        }
        parts.push(part.to_string());
    }
    if parts.is_empty() {
        return Err(invalid("空归档路径"));
    }
    Ok(parts.join("/"))
}

fn validate_frontmatter(id: &str, bytes: &[u8]) -> Result<(), HotError> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(invalid("SKILL.md 不得含 BOM"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("SKILL.md 不是 UTF-8"))?;
    if text.len() > 64 * 1024 {
        return Err(invalid("SKILL.md frontmatter 超限"));
    }
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    if lines.next() != Some("---") {
        return Err(invalid("SKILL.md 缺少 frontmatter 起始线"));
    }
    let mut seen = HashSet::new();
    let mut name = None;
    let mut desc = None;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.contains('\t') || line.is_empty() {
            if !line.is_empty() {
                return Err(invalid("frontmatter 不得含 tab"));
            }
            continue;
        }
        let Some(colon) = line.find(':') else {
            return Err(invalid("frontmatter 行格式无效"));
        };
        let key = &line[..colon];
        let Some(value_with_space) = line.get(colon + 1..) else {
            return Err(invalid("frontmatter 行格式无效"));
        };
        if !value_with_space.starts_with(' ') || value_with_space.starts_with("  ") {
            return Err(invalid("frontmatter 冒号后必须恰好一个空格"));
        }
        let value = &value_with_space[1..];
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || !key.as_bytes()[0].is_ascii_alphabetic()
            || !seen.insert(key.to_string())
            || value.contains('\r')
            || value.contains('\n')
            || value.contains('\t')
        {
            return Err(invalid("frontmatter key/value 无效"));
        }
        if key == "name" {
            name = Some(value);
        }
        if key == "description" {
            desc = Some(value.trim());
        }
    }
    if !closed || name != Some(id) || desc.map(str::is_empty).unwrap_or(true) {
        return Err(invalid(format!("frontmatter 与 Skill ID 不匹配: {id}")));
    }
    Ok(())
}

fn validate_payload_file(path: &str, bytes: &[u8]) -> Result<(), HotError> {
    let lower = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    for suffix in [".exe", ".dll", ".msi", ".sys", ".ocx", ".scr"] {
        if lower.ends_with(suffix) {
            return Err(invalid(format!("禁止的可执行文件: {path}")));
        }
    }
    if bytes.starts_with(b"MZ")
        || bytes.starts_with(&[0x7f, b'E', b'L', b'F'])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
    {
        return Err(invalid(format!("禁止的二进制文件: {path}")));
    }
    Ok(())
}

fn parse_archive(bytes: &[u8]) -> Result<Vec<CandidateSkill>, HotError> {
    if bytes.len() > MAX_ARCHIVE {
        return Err(invalid("归档超过 64 MiB"));
    }
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| invalid(format!("ZIP 无法读取: {e}")))?;
    if archive.len() > MAX_ENTRIES {
        return Err(invalid("ZIP 条目超过 4096"));
    }
    let mut prefix: Option<String> = None;
    let mut entries = Vec::new();
    let mut total = 0usize;
    let mut keys = HashSet::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| invalid(format!("ZIP 条目失败: {e}")))?;
        if file
            .unix_mode()
            .map(|m| m & 0o170000 == 0o120000)
            .unwrap_or(false)
        {
            return Err(invalid("ZIP 不得含符号链接"));
        }
        let normalized = normalize_zip_path(file.name(), file.is_dir())?;
        let root = normalized.split('/').next().unwrap_or_default().to_string();
        if prefix.is_none() {
            prefix = Some(root.clone());
        }
        if prefix.as_deref() != Some(root.as_str()) {
            return Err(invalid("ZIP 顶层前缀不一致"));
        }
        let Some((_, rest)) = normalized.split_once('/') else {
            if file.is_dir() {
                continue;
            }
            return Err(invalid("ZIP 缺少仓库前缀"));
        };
        let rest = rest.to_string();
        let key = rest.to_lowercase();
        if !keys.insert(key) {
            return Err(invalid("ZIP 路径大小写/Unicode 碰撞"));
        }
        if !file.is_dir() {
            let size = file.size();
            if size > MAX_FILE as u64 {
                return Err(invalid("Skill 文件超过 32 MiB"));
            }
            total = total.saturating_add(size as usize);
            if total > MAX_UNPACKED {
                return Err(invalid("ZIP 解压总量超过 256 MiB"));
            }
            let mut data = Vec::with_capacity(size as usize);
            file.read_to_end(&mut data)
                .map_err(|e| invalid(format!("读取 ZIP 条目失败: {e}")))?;
            validate_payload_file(&rest, &data)?;
            entries.push((rest, data));
        }
    }
    let prefix = prefix.ok_or_else(|| invalid("空 ZIP"))?;
    if !prefix.starts_with("skill_hub-")
        || prefix.len() != 50
        || !prefix[10..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid("GitHub ZIP 前缀无效"));
    }
    let mut skills: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    for (path, data) in entries {
        if matches!(
            path.as_str(),
            "README.md" | "LICENSE" | "LICENSE.txt" | ".gitignore"
        ) {
            continue;
        }
        let mut it = path.splitn(2, '/');
        let id = it.next().unwrap_or_default();
        let Some(rel) = it.next() else {
            return Err(invalid("仓库根存在未允许文件"));
        };
        if !valid_id(id) || rel.is_empty() || rel.starts_with('/') {
            return Err(invalid("Skill 根目录无效"));
        }
        skills
            .entry(id.to_string())
            .or_default()
            .insert(rel.to_string(), data);
    }
    if skills.is_empty() {
        return Err(invalid("归档没有 Skill"));
    }
    let mut out = Vec::new();
    for (id, files) in skills {
        if !files.contains_key("SKILL.md") {
            return Err(invalid(format!("{id} 缺少 SKILL.md")));
        }
        if let Some(md) = files.get("SKILL.md") {
            validate_frontmatter(&id, md)?;
        }
        out.push(CandidateSkill { id, files });
    }
    Ok(out)
}

fn client() -> Result<reqwest::Client, HotError> {
    reqwest::Client::builder()
        .user_agent("codex-app-manager-skill-hub/1")
        .redirect(reqwest::redirect::Policy::limited(3))
        // Do not let a mainland DNS/TCP black hole keep the UI spinner alive
        // while the relay fallback is waiting for a full request timeout.
        .connect_timeout(NETWORK_CONNECT_TIMEOUT)
        .timeout(NETWORK_REQUEST_TIMEOUT)
        .build()
        .map_err(|_| HotError::new("SKILL_REPO_UNAVAILABLE", "Skill Hub 网络客户端初始化失败"))
}

fn network_timeout() -> HotError {
    HotError::new(
        "SKILL_REPO_TIMEOUT",
        "Skill Hub 请求超过 45 秒；已记录 relay/GitHub 来源尝试，请检查网络后重试",
    )
}

async fn get_bytes(
    client: &reqwest::Client,
    url: &str,
    max: usize,
    github: bool,
    phase: &'static str,
    attempt: u8,
) -> Result<Vec<u8>, HotError> {
    let source = if github { "github" } else { "relay" };
    let started = Instant::now();
    log::info!(
        "skill_hub_attempt phase={phase} source={source} attempt={attempt} operation=download"
    );
    let response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            log::warn!(
                "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=timeout elapsed_ms={}",
                started.elapsed().as_millis()
            );
            HotError::new(
                "SKILL_REPO_TIMEOUT",
                "Skill Hub 网络请求超时，请检查网络后重试",
            )
            .with_detail(format!("phase={phase}; source={source}; attempt={attempt}; transport=timeout"))
        } else {
            log::warn!(
                "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=transport_error elapsed_ms={}",
                started.elapsed().as_millis()
            );
            HotError::new(
                "SKILL_REPO_UNAVAILABLE",
                "Skill Hub 暂时无法连接，请检查网络后重试",
            )
            .with_detail(format!("phase={phase}; source={source}; attempt={attempt}; transport=unavailable"))
        }
    })?;
    if !response.status().is_success() {
        log::warn!(
            "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=http_error status={} elapsed_ms={}",
            response.status().as_u16(),
            started.elapsed().as_millis()
        );
        return Err(HotError::new(
            "SKILL_REPO_UNAVAILABLE",
            format!("HTTP {}", response.status()),
        )
        .with_detail(format!(
            "phase={phase}; source={source}; attempt={attempt}; http_status={}",
            response.status().as_u16()
        )));
    }
    if !github && response.url().as_str() != url {
        log::warn!(
            "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=unexpected_redirect elapsed_ms={}",
            started.elapsed().as_millis()
        );
        return Err(
            HotError::new("SKILL_REPO_SOURCE_INVALID", "relay 文件禁止 HTTP 重定向").with_detail(
                format!("phase={phase}; source={source}; attempt={attempt}; redirect=rejected"),
            ),
        );
    }
    if github
        && !matches!(
            response.url().host_str(),
            Some("api.github.com") | Some("github.com") | Some("codeload.github.com")
        )
    {
        log::warn!(
            "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=redirect_rejected elapsed_ms={}",
            started.elapsed().as_millis()
        );
        return Err(
            HotError::new("SKILL_REPO_SOURCE_INVALID", "GitHub 重定向目标不允许").with_detail(
                format!("phase={phase}; source={source}; attempt={attempt}; redirect=rejected"),
            ),
        );
    }
    let bytes = response.bytes().await.map_err(|error| {
        if error.is_timeout() {
            log::warn!(
                "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=read_timeout elapsed_ms={}",
                started.elapsed().as_millis()
            );
            HotError::new("SKILL_REPO_TIMEOUT", "Skill Hub 下载超时，请检查网络后重试")
                .with_detail(format!("phase={phase}; source={source}; attempt={attempt}; read=timeout"))
        } else {
            log::warn!(
                "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=read_error elapsed_ms={}",
                started.elapsed().as_millis()
            );
            HotError::new(
                "SKILL_REPO_UNAVAILABLE",
                "Skill Hub 响应读取失败，请稍后重试",
            )
            .with_detail(format!("phase={phase}; source={source}; attempt={attempt}; read=error"))
        }
    })?;
    if bytes.len() > max {
        log::warn!(
            "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=response_too_large bytes={} elapsed_ms={}",
            bytes.len(),
            started.elapsed().as_millis()
        );
        return Err(invalid("远端响应超限"));
    }
    log::info!(
        "skill_hub_attempt phase={phase} source={source} attempt={attempt} result=ok bytes={} elapsed_ms={}",
        bytes.len(),
        started.elapsed().as_millis()
    );
    Ok(bytes.to_vec())
}

fn expected_from_meta(meta: &[u8], source: SourceKind) -> Result<SkillRepoExpected, HotError> {
    #[derive(Deserialize)]
    struct Meta {
        #[serde(rename = "schemaVersion")]
        schema_version: u8,
        repo: String,
        branch: String,
        commit: String,
        archive: String,
        size: u64,
        sha256: String,
    }
    let m: Meta = serde_json::from_slice(meta)
        .map_err(|e| HotError::new("SKILL_REPO_SOURCE_INVALID", e.to_string()))?;
    if m.schema_version != 1
        || m.repo != REPO
        || m.branch != BRANCH
        || m.archive != "archive.zip"
        || m.commit.len() != 40
        || !m.commit.chars().all(|c| c.is_ascii_hexdigit())
        || m.sha256.len() != 64
    {
        return Err(HotError::new(
            "SKILL_REPO_SOURCE_INVALID",
            "relay metadata 身份无效",
        ));
    }
    Ok(SkillRepoExpected {
        schema_version: 1,
        repo: m.repo,
        branch: m.branch,
        commit: m.commit,
        source_kind: source,
        archive_size: m.size,
        archive_sha256: m.sha256.to_lowercase(),
    })
}

async fn github_commit(client: &reqwest::Client) -> Result<String, HotError> {
    let bytes = get_bytes(client, GITHUB_COMMITS, 64 * 1024, true, "metadata", 1).await?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| HotError::new("SKILL_REPO_SOURCE_INVALID", e.to_string()))?;
    let sha = v
        .get("sha")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_lowercase();
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HotError::new(
            "SKILL_REPO_SOURCE_INVALID",
            "GitHub commit 无效",
        ));
    }
    Ok(sha)
}

async fn resolve_latest(
    client: &reqwest::Client,
) -> Result<(SkillRepoExpected, Vec<u8>), HotError> {
    log::info!("skill_hub_event phase=resolve source_order=relay,github retry=0");
    if let Ok(meta) = get_bytes(client, RELAY_META, 64 * 1024, false, "metadata", 1).await {
        if let Ok(exp) = expected_from_meta(&meta, SourceKind::Relay) {
            if let Ok(archive) =
                get_bytes(client, RELAY_ARCHIVE, MAX_ARCHIVE, false, "archive", 1).await
            {
                if archive.len() as u64 == exp.archive_size
                    && sha256(&archive) == exp.archive_sha256
                {
                    let parsed =
                        get_bytes(client, RELAY_META, 64 * 1024, false, "metadata-confirm", 2)
                            .await
                            .and_then(|m| expected_from_meta(&m, SourceKind::Relay));
                    if parsed.as_ref().map(|x| x == &exp).unwrap_or(false) {
                        log::info!("skill_hub_event phase=resolve source=relay attempt=1 result=ok retry=0");
                        return Ok((exp, archive));
                    }
                    log::warn!("skill_hub_event phase=resolve source=relay attempt=1 result=metadata_changed retry=0");
                } else {
                    log::warn!("skill_hub_event phase=resolve source=relay attempt=1 result=archive_hash_mismatch retry=0");
                }
            } else {
                log::warn!("skill_hub_event phase=resolve source=relay attempt=1 result=archive_unavailable retry=0");
            }
        } else {
            log::warn!("skill_hub_event phase=resolve source=relay attempt=1 result=metadata_invalid retry=0");
        }
    } else {
        log::warn!("skill_hub_event phase=resolve source=relay attempt=1 result=metadata_unavailable retry=0");
    }
    log::info!("skill_hub_event phase=resolve source=github attempt=1 result=fallback retry=1");
    let commit = github_commit(client).await?;
    let url = format!("{GITHUB_ARCHIVE}{commit}.zip");
    let archive = get_bytes(client, &url, MAX_ARCHIVE, true, "archive", 1).await?;
    let exp = SkillRepoExpected {
        schema_version: 1,
        repo: REPO.into(),
        branch: BRANCH.into(),
        commit: commit.clone(),
        source_kind: SourceKind::Github,
        archive_size: archive.len() as u64,
        archive_sha256: sha256(&archive),
    };
    let after = github_commit(client).await?;
    if after != commit {
        return Err(HotError::new(
            "SKILL_REPO_STALE",
            "GitHub commit 在下载期间变化",
        ));
    }
    log::info!("skill_hub_event phase=resolve source=github attempt=1 result=ok retry=1");
    Ok((exp, archive))
}

async fn resolve_latest_with_deadline(
    client: &reqwest::Client,
) -> Result<(SkillRepoExpected, Vec<u8>), HotError> {
    match tokio::time::timeout(NETWORK_OPERATION_TIMEOUT, resolve_latest(client)).await {
        Ok(result) => result,
        Err(_) => {
            log::warn!(
                "skill_hub_event phase=resolve source=all attempt=0 result=operation_timeout retry=1 budget_ms={}",
                NETWORK_OPERATION_TIMEOUT.as_millis()
            );
            Err(network_timeout())
        }
    }
}

async fn download_expected(
    client: &reqwest::Client,
    expected: &SkillRepoExpected,
) -> Result<Vec<u8>, HotError> {
    if expected.repo != REPO || expected.branch != BRANCH || expected.commit.len() != 40 {
        return Err(HotError::new(
            "SKILL_REPO_SOURCE_INVALID",
            "expected identity 无效",
        ));
    }
    let bytes = match expected.source_kind {
        SourceKind::Relay => {
            let before = get_bytes(client, RELAY_META, 64 * 1024, false, "metadata-confirm", 1)
                .await
                .and_then(|m| expected_from_meta(&m, SourceKind::Relay))?;
            if before != *expected {
                return Err(HotError::new("SKILL_REPO_STALE", "relay metadata 已变化"));
            }
            get_bytes(client, RELAY_ARCHIVE, MAX_ARCHIVE, false, "archive", 2).await?
        }
        SourceKind::Github => {
            let before = github_commit(client).await?;
            if before != expected.commit {
                return Err(HotError::new("SKILL_REPO_STALE", "GitHub commit 已变化"));
            }
            get_bytes(
                client,
                &format!("{GITHUB_ARCHIVE}{}.zip", expected.commit),
                MAX_ARCHIVE,
                true,
                "archive",
                2,
            )
            .await?
        }
    };
    if bytes.len() as u64 != expected.archive_size || sha256(&bytes) != expected.archive_sha256 {
        return Err(HotError::new(
            "SKILL_REPO_HASH_MISMATCH",
            "归档 size/SHA-256 不匹配",
        ));
    }
    match expected.source_kind {
        SourceKind::Relay => {
            let after = get_bytes(client, RELAY_META, 64 * 1024, false, "metadata-confirm", 3)
                .await
                .and_then(|m| expected_from_meta(&m, SourceKind::Relay))?;
            if after != *expected {
                return Err(HotError::new(
                    "SKILL_REPO_STALE",
                    "relay metadata 在下载后变化",
                ));
            }
        }
        SourceKind::Github => {
            if github_commit(client).await? != expected.commit {
                return Err(HotError::new(
                    "SKILL_REPO_STALE",
                    "GitHub commit 在下载后变化",
                ));
            }
        }
    }
    Ok(bytes)
}

async fn download_expected_with_deadline(
    client: &reqwest::Client,
    expected: &SkillRepoExpected,
) -> Result<Vec<u8>, HotError> {
    tokio::time::timeout(
        NETWORK_OPERATION_TIMEOUT,
        download_expected(client, expected),
    )
    .await
    .map_err(|_| network_timeout())?
}

fn plan_item(
    id: &str,
    state: &StateFile,
    base: &BTreeSet<String>,
    root: &Path,
) -> Result<(String, u32), HotError> {
    let dir = root.join(id);
    let count = state.skills.get(id).map(|s| s.files.len()).unwrap_or(0) as u32;
    if base.contains(id) {
        return Ok(("SKILL_REPO_BASE_CONFLICT".into(), count));
    }
    if let Some(owned) = state.skills.get(id) {
        if !matches!(metadata_optional(&dir)?, Some(metadata) if metadata.is_dir()) {
            return Ok(("SKILL_REPO_LEDGER_INVALID".into(), count));
        }
        if !file_records_equal(&dir, &owned.files) {
            return Ok(("SKILL_REPO_HOT_MODIFIED".into(), count));
        }
        return Ok(("apply".into(), count));
    }
    if metadata_optional(&dir)?.is_some() {
        return Ok(("SKILL_REPO_USER_CONFLICT".into(), count));
    }
    Ok(("apply".into(), count))
}

fn write_skill(dir: &Path, skill: &CandidateSkill) -> Result<Vec<FileRecord>, HotError> {
    safe_create_dir_all(dir, "创建 staging Skill 失败")?;
    for (rel, data) in &skill.files {
        if normalize_zip_path(rel, false)? != *rel {
            return Err(invalid(format!("staging 相对路径无效: {rel}")));
        }
        let path = dir.join(rel);
        if let Some(p) = path.parent() {
            safe_create_dir_all(p, "创建 Skill 子目录失败")?;
        }
        safe_write_new_file(&path, data, "写入 staging Skill 失败")?;
    }
    sync_directory(dir)?;
    list_files(dir)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), HotError> {
    ensure_directory(src, "复制备份源失败")?;
    ensure_safe_parent(dst)?;
    if metadata_optional(dst)?.is_some() {
        return Err(io_error(
            "创建备份目录失败",
            format!("目标已存在: {}", dst.display()),
        ));
    }
    safe_create_dir_all(dst, "创建备份目录失败")?;
    let entries = fs::read_dir(src).map_err(|e| io_error("枚举备份源失败", e))?;
    for e in entries {
        let e = e.map_err(|e| io_error("读取备份源失败", e))?;
        let s = e.path();
        let d = dst.join(e.file_name());
        ensure_safe_parent(&s)?;
        let metadata = metadata_required(&s, "读取备份源失败")?;
        if metadata.is_dir() {
            copy_dir(&s, &d)?;
        } else if metadata.is_file() {
            safe_copy_file(&s, &d, "复制备份文件失败")?;
        } else {
            return Err(invalid("copy source contains special file"));
        }
    }
    ensure_safe_parent(src)?;
    ensure_safe_parent(dst)?;
    sync_directory(dst)?;
    Ok(())
}

fn commit_valid(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn scoped_id_valid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

fn validate_record(record: &FileRecord) -> Result<(), HotError> {
    let normalized = normalize_zip_path(&record.path, false)?;
    if normalized != record.path
        || record.path.starts_with('/')
        || !digest_valid(&record.sha256)
        || record.volume_id.as_deref().is_none_or(str::is_empty)
        || record.file_id.as_deref().is_none_or(str::is_empty)
    {
        return Err(io_error("热更新文件记录无效", &record.path));
    }
    if record.size > MAX_FILE as u64 {
        return Err(io_error("热更新文件记录超限", &record.path));
    }
    Ok(())
}

fn validate_records(records: &[FileRecord], allow_empty: bool) -> Result<(), HotError> {
    if !allow_empty && records.is_empty() {
        return Err(io_error("热更新文件清单为空", "records"));
    }
    let mut paths = HashSet::new();
    for record in records {
        validate_record(record)?;
        if !paths.insert(record.path.to_lowercase()) {
            return Err(io_error("热更新文件重复", &record.path));
        }
    }
    Ok(())
}

fn expected_identity_valid(expected: &SkillRepoExpected) -> bool {
    expected.schema_version == 1
        && expected.repo == REPO
        && expected.branch == BRANCH
        && commit_valid(&expected.commit)
        && expected.archive_size <= MAX_ARCHIVE as u64
        && digest_valid(&expected.archive_sha256)
}

fn validate_state_file(state: &StateFile) -> Result<(), HotError> {
    if state.schema_version != 1 || state.repo != REPO || state.branch != BRANCH {
        return Err(io_error("热更新账本身份无效", "schema/repo/branch"));
    }
    if state
        .last_seen_commit
        .as_deref()
        .is_some_and(|v| !commit_valid(v))
    {
        return Err(io_error("热更新账本 commit 无效", "lastSeenCommit"));
    }
    if !matches!(
        state.last_apply_status.as_str(),
        "never" | "success" | "partial" | "failed" | "blocked"
    ) {
        return Err(io_error("热更新账本状态无效", &state.last_apply_status));
    }
    if state
        .last_good_generation
        .as_deref()
        .is_some_and(|value| !scoped_id_valid(value))
    {
        return Err(io_error("热更新账本 generation 无效", "lastGoodGeneration"));
    }
    for (id, skill) in &state.skills {
        if !valid_id(id) || !commit_valid(&skill.installed_commit) || skill.files.is_empty() {
            return Err(io_error("热更新账本 Skill 记录无效", id));
        }
        if skill.directory_volume_id.is_none() || skill.directory_file_id.is_none() {
            return Err(io_error("热更新账本目录身份缺失", id));
        }
        validate_records(&skill.files, false)?;
    }
    Ok(())
}

fn validate_journal(journal: &Journal) -> Result<(), HotError> {
    if journal.schema_version != 1
        || !scoped_id_valid(&journal.transaction_id)
        || !scoped_id_valid(&journal.backup_generation)
        || !matches!(
            journal.phase.as_str(),
            "prepared" | "backed-up" | "swapped" | "state-committed" | "rollback-required" | "done"
        )
        || journal.rollback_root != "rollback"
        || !expected_identity_valid(&journal.expected_identity)
    {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "事务 journal 身份无效",
        ));
    }
    match (&journal.previous_state_file, &journal.previous_state_sha256) {
        (Some(name), Some(digest)) if name == "previous-state.json" && digest_valid(digest) => {}
        (None, None) => {}
        _ => {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                "事务 previous state 引用无效",
            ))
        }
    }
    let mut ids = HashSet::new();
    for item in &journal.skills {
        if !valid_id(&item.id)
            || item.target_relative != item.id
            || !ids.insert(item.id.clone())
            || !matches!(item.action.as_str(), "add" | "replace")
            || item.new_installed_commit != journal.expected_identity.commit
            || item
                .new_directory_volume_id
                .as_deref()
                .is_none_or(|value| value.is_empty())
            || item
                .new_directory_file_id
                .as_deref()
                .is_none_or(|value| value.is_empty())
        {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("事务 Skill 记录无效: {}", item.id),
            ));
        }
        validate_records(&item.new_files, false)?;
        match item.action.as_str() {
            "add" => {
                if item.old_installed_commit.is_some()
                    || item.old_directory_volume_id.is_some()
                    || item.old_directory_file_id.is_some()
                    || !item.old_files.is_empty()
                    || item.backup_relative.is_some()
                    || item.rollback_relative.is_some()
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("新增 Skill 的旧目录引用无效: {}", item.id),
                    ));
                }
            }
            "replace" => {
                if !item
                    .old_installed_commit
                    .as_deref()
                    .is_some_and(commit_valid)
                    || item
                        .old_directory_volume_id
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || item
                        .old_directory_file_id
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || item.backup_relative.as_deref()
                        != Some(format!("{}/{}", journal.backup_generation, item.id).as_str())
                    || item.rollback_relative.as_deref() != Some(item.id.as_str())
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("替换 Skill 的旧目录引用无效: {}", item.id),
                    ));
                }
                validate_records(&item.old_files, false)?;
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn validate_rollback_journal(journal: &RollbackJournal) -> Result<(), HotError> {
    if journal.schema_version != 1
        || !scoped_id_valid(&journal.transaction_id)
        || !scoped_id_valid(&journal.generation_id)
        || !matches!(
            journal.phase.as_str(),
            "prepared" | "quarantined" | "swapped" | "state-committed" | "done"
        )
        || journal.current_state_file != "current-state.json"
        || !digest_valid(&journal.current_state_sha256)
    {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "rollback journal 身份无效",
        ));
    }
    let mut ids = HashSet::new();
    for item in &journal.skills {
        if !valid_id(&item.id)
            || !ids.insert(item.id.clone())
            || item.new_files.is_empty()
            || item
                .new_directory_volume_id
                .as_deref()
                .is_none_or(|value| value.is_empty())
            || item
                .new_directory_file_id
                .as_deref()
                .is_none_or(|value| value.is_empty())
        {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("rollback journal Skill 记录无效: {}", item.id),
            ));
        }
        validate_records(&item.new_files, false)?;
        match &item.old_backup_relative {
            Some(relative) if relative == &item.id => {
                if item.old_files.is_empty()
                    || item
                        .old_directory_volume_id
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || item
                        .old_directory_file_id
                        .as_deref()
                        .is_none_or(str::is_empty)
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("rollback journal 旧目录记录无效: {}", item.id),
                    ));
                }
                validate_records(&item.old_files, false)?;
            }
            None => {
                if !item.old_files.is_empty()
                    || item.old_directory_volume_id.is_some()
                    || item.old_directory_file_id.is_some()
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("rollback journal 新增 Skill 记录无效: {}", item.id),
                    ));
                }
            }
            Some(_) => {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("rollback journal 旧目录路径无效: {}", item.id),
                ));
            }
        }
    }
    Ok(())
}

fn validate_generation_value(generation: &Generation, generation_id: &str) -> Result<(), HotError> {
    if generation.schema_version != 1
        || generation.generation_id != generation_id
        || !scoped_id_valid(&generation.generation_id)
        || !commit_valid(&generation.commit)
        || !generation.complete
    {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "generation 身份无效",
        ));
    }
    match (
        &generation.previous_state_file,
        &generation.previous_state_sha256,
    ) {
        (Some(name), Some(digest)) if name == "previous-state.json" && digest_valid(digest) => {}
        (None, None) => {}
        _ => {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                "generation previous state 引用无效",
            ))
        }
    }
    let mut ids = HashSet::new();
    for item in &generation.skills {
        if !valid_id(&item.id) || !ids.insert(item.id.clone()) {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("generation Skill ID 无效: {}", item.id),
            ));
        }
        validate_records(&item.new_files, false)?;
        match &item.backup_relative {
            Some(relative) => {
                if relative != &item.id
                    || !commit_valid(&item.installed_commit)
                    || item
                        .directory_volume_id
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || item.directory_file_id.as_deref().is_none_or(str::is_empty)
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("generation 旧目录引用无效: {}", item.id),
                    ));
                }
                validate_records(&item.files, false)?;
            }
            None => {
                if item.installed_commit != generation.commit
                    || item.directory_volume_id.is_some()
                    || item.directory_file_id.is_some()
                    || !item.files.is_empty()
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("generation 新增 Skill 记录无效: {}", item.id),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn read_generation_from_disk(
    generation_root: &Path,
    generation_id: &str,
) -> Result<Generation, HotError> {
    ensure_directory(generation_root, "读取 generation 失败")?;
    let generation_path = generation_root.join("generation.json");
    let generation_bytes = safe_read_file(&generation_path, "读取 generation 失败")?;
    let generation: Generation = serde_json::from_slice(&generation_bytes)
        .map_err(|e| io_error("generation 格式无效", e))?;
    validate_generation_value(&generation, generation_id)?;
    let marker = safe_read_file(
        &generation_root.join("complete.marker"),
        "读取 generation marker 失败",
    )?;
    if marker != sha256(&generation_bytes).as_bytes() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "generation marker 无效",
        ));
    }
    match (
        &generation.previous_state_file,
        &generation.previous_state_sha256,
    ) {
        (Some(name), Some(digest)) => {
            let bytes = safe_read_file(
                &generation_root.join(name),
                "读取 generation previous state 失败",
            )?;
            if sha256(&bytes) != *digest {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "generation previous state SHA-256 不匹配",
                ));
            }
        }
        (None, None) => {}
        _ => unreachable!(),
    }
    let mut allowed = HashSet::from(["generation.json".to_string(), "complete.marker".to_string()]);
    if generation.previous_state_file.is_some() {
        allowed.insert("previous-state.json".into());
    }
    for item in &generation.skills {
        if item.backup_relative.is_some() {
            let path = generation_root.join(&item.id);
            allowed.insert(item.id.clone());
            if !current_files_match(&path, &item.files)
                || !directory_identity_matches(
                    &path,
                    item.directory_volume_id.as_deref(),
                    item.directory_file_id.as_deref(),
                )
            {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("generation 旧 Skill 核对失败: {}", item.id),
                ));
            }
        }
    }
    let entries = fs::read_dir(generation_root).map_err(|e| io_error("枚举 generation 失败", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_error("读取 generation 目录项失败", e))?;
        ensure_safe_parent(&entry.path())?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(|| {
            HotError::new("SKILL_REPO_ROLLBACK_FAILED", "generation 文件名不是 UTF-8")
        })?;
        if !allowed.contains(name) {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("generation 含额外文件: {name}"),
            ));
        }
    }
    Ok(generation)
}

fn state_path() -> Result<PathBuf, HotError> {
    Ok(state_dir()?.join("state.json"))
}
fn journal_path() -> Result<PathBuf, HotError> {
    Ok(state_dir()?.join("journal.json"))
}
fn rollback_journal_path() -> Result<PathBuf, HotError> {
    Ok(state_dir()?.join("rollback-journal.json"))
}
fn transaction_root(id: &str) -> Result<PathBuf, HotError> {
    if !scoped_id_valid(id) {
        return Err(HotError::new("SKILL_REPO_ROLLBACK_FAILED", "事务 ID 无效"));
    }
    Ok(home_dir()?
        .join(".agents")
        .join(".skill-hub-staging")
        .join(id))
}
fn generation_root(id: &str) -> Result<PathBuf, HotError> {
    if !scoped_id_valid(id) {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "generation ID 无效",
        ));
    }
    Ok(home_dir()?
        .join(".agents")
        .join(".skill-hub-backups")
        .join(id))
}

fn persist_journal(journal: &Journal) -> Result<(), HotError> {
    write_atomic_json(&journal_path()?, journal)
}

fn persist_rollback_journal(journal: &RollbackJournal) -> Result<(), HotError> {
    write_atomic_json(&rollback_journal_path()?, journal)
}

fn write_previous_state(txroot: &Path, bytes: Option<&[u8]>) -> Result<Option<String>, HotError> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let path = txroot.join("previous-state.json");
    safe_write_new_file(&path, bytes, "创建 previous state 失败")?;
    if safe_read_file(&path, "核对 previous state 失败")? != bytes {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "previous state 写入后字节内容变化",
        ));
    }
    Ok(Some(sha256(bytes)))
}

fn current_files_match(root: &Path, expected: &[FileRecord]) -> bool {
    file_records_equal(root, expected)
}

fn current_content_matches(root: &Path, expected: &[FileRecord]) -> bool {
    file_records_content_equal(root, expected)
}

fn directory_identity_matches(
    path: &Path,
    expected_volume_id: Option<&str>,
    expected_file_id: Option<&str>,
) -> bool {
    let Ok((volume_id, file_id)) = file_identity(path) else {
        return false;
    };
    expected_volume_id == Some(volume_id.as_str()) && expected_file_id == Some(file_id.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionTargetKind {
    Missing,
    New,
    Old,
}

fn classify_transaction_target(
    path: &Path,
    item: &JournalSkill,
) -> Result<TransactionTargetKind, HotError> {
    let Some(metadata) = metadata_optional(path)? else {
        return Ok(TransactionTargetKind::Missing);
    };
    if !metadata.is_dir() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("事务目标不是目录: {}", item.id),
        ));
    }
    if current_content_matches(path, &item.new_files) {
        if current_files_match(path, &item.new_files)
            && directory_identity_matches(
                path,
                item.new_directory_volume_id.as_deref(),
                item.new_directory_file_id.as_deref(),
            )
        {
            return Ok(TransactionTargetKind::New);
        }
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("事务新目录 file-id 或目录身份变化: {}", item.id),
        ));
    }
    if item.action == "replace" && current_content_matches(path, &item.old_files) {
        if current_files_match(path, &item.old_files)
            && directory_identity_matches(
                path,
                item.old_directory_volume_id.as_deref(),
                item.old_directory_file_id.as_deref(),
            )
        {
            return Ok(TransactionTargetKind::Old);
        }
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("事务旧目录 file-id 或目录身份变化: {}", item.id),
        ));
    }
    Err(HotError::new(
        "SKILL_REPO_HOT_MODIFIED",
        format!("rollback 检测到用户修改: {}", item.id),
    ))
}

fn backup_shadow_path(generation: &Path, id: &str) -> PathBuf {
    generation.join(format!(".{id}.copy"))
}

fn verified_old_source(
    txroot: &Path,
    generation: &Path,
    item: &JournalSkill,
) -> Result<Option<PathBuf>, HotError> {
    if item.action != "replace" {
        return Ok(None);
    }
    let rollback_dir = txroot.join("rollback").join(&item.id);
    let generation_dir = generation.join(&item.id);
    let shadow = backup_shadow_path(generation, &item.id);
    let mut source = None;
    if metadata_optional(&rollback_dir)?.is_some() {
        if !current_files_match(&rollback_dir, &item.old_files)
            || !directory_identity_matches(
                &rollback_dir,
                item.old_directory_volume_id.as_deref(),
                item.old_directory_file_id.as_deref(),
            )
        {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("rollback 旧目录身份或清单无效: {}", item.id),
            ));
        }
        source = Some(rollback_dir);
    }
    if metadata_optional(&generation_dir)?.is_some() {
        if current_files_match(&generation_dir, &item.old_files)
            && directory_identity_matches(
                &generation_dir,
                item.old_directory_volume_id.as_deref(),
                item.old_directory_file_id.as_deref(),
            )
        {
            if source.is_some() {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("旧目录证据重复: {}", item.id),
                ));
            }
            source = Some(generation_dir.clone());
        } else if !current_content_matches(&generation_dir, &item.old_files) {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("generation 备份内容变化: {}", item.id),
            ));
        }
    }
    if metadata_optional(&shadow)?.is_some() && !current_content_matches(&shadow, &item.old_files) {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("generation copy 证据变化: {}", item.id),
        ));
    }
    Ok(source)
}

fn read_previous_state_for_transaction(
    txroot: &Path,
    journal: &Journal,
) -> Result<Option<(Vec<u8>, StateFile)>, HotError> {
    match (&journal.previous_state_file, &journal.previous_state_sha256) {
        (Some(name), Some(digest)) if name == "previous-state.json" => {
            let bytes = safe_read_file(&txroot.join(name), "读取 previous state 失败")?;
            if sha256(&bytes) != *digest {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "previous state SHA-256 不匹配",
                ));
            }
            let state: StateFile = serde_json::from_slice(&bytes)
                .map_err(|e| HotError::new("SKILL_REPO_ROLLBACK_FAILED", e.to_string()))?;
            validate_state_file(&state)?;
            Ok(Some((bytes, state)))
        }
        (None, None) => Ok(None),
        _ => Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "previous state 引用不完整",
        )),
    }
}

fn state_matches_transaction(
    state: &StateFile,
    journal: &Journal,
    previous: Option<&StateFile>,
) -> bool {
    if validate_state_file(state).is_err()
        || state.last_good_generation.as_deref() != Some(journal.backup_generation.as_str())
    {
        return false;
    }
    let selected = journal
        .skills
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    for item in &journal.skills {
        let Some(skill) = state.skills.get(&item.id) else {
            return false;
        };
        if skill.installed_commit != item.new_installed_commit
            || skill.directory_volume_id.as_deref() != item.new_directory_volume_id.as_deref()
            || skill.directory_file_id.as_deref() != item.new_directory_file_id.as_deref()
            || skill.files != item.new_files
        {
            return false;
        }
    }
    match previous {
        Some(previous) => {
            for (id, skill) in &previous.skills {
                if !selected.contains(id.as_str()) && state.skills.get(id) != Some(skill) {
                    return false;
                }
            }
            if state
                .skills
                .keys()
                .any(|id| !previous.skills.contains_key(id) && !selected.contains(id.as_str()))
            {
                return false;
            }
            let last_seen_allowed = state.last_seen_commit == previous.last_seen_commit
                || state.last_seen_commit.as_deref()
                    == Some(journal.expected_identity.commit.as_str());
            let status_allowed = state.last_apply_status == previous.last_apply_status
                || matches!(state.last_apply_status.as_str(), "success" | "partial");
            last_seen_allowed && status_allowed
        }
        None => {
            state.skills.len() == selected.len()
                && state.skills.keys().all(|id| selected.contains(id.as_str()))
                && (state.last_seen_commit.is_none()
                    || state.last_seen_commit.as_deref()
                        == Some(journal.expected_identity.commit.as_str()))
                && matches!(state.last_apply_status.as_str(), "success" | "partial")
        }
    }
}

fn restore_transaction(txroot: &Path, journal: &Journal, target: &Path) -> Result<(), HotError> {
    validate_journal(journal)?;
    if journal.rollback_root != "rollback" {
        return Err(io_error("事务 rollback 根无效", &journal.rollback_root));
    }
    ensure_directory(target, "恢复目标根失败")?;
    let generation = generation_root(&journal.backup_generation)?;
    ensure_directory(&generation, "读取 generation 失败")?;
    let previous = read_previous_state_for_transaction(txroot, journal)?;
    let state_path = state_path()?;
    let current_state = if metadata_optional(&state_path)?.is_some() {
        Some(safe_read_file(&state_path, "读取待恢复 state 失败")?)
    } else {
        None
    };
    if let Some(bytes) = &current_state {
        let unchanged = previous
            .as_ref()
            .map(|(old_bytes, _)| old_bytes == bytes)
            .unwrap_or(false);
        if !unchanged {
            let state: StateFile = serde_json::from_slice(bytes)
                .map_err(|e| HotError::new("SKILL_REPO_ROLLBACK_FAILED", e.to_string()))?;
            let old_state = previous.as_ref().map(|(_, state)| state);
            if !state_matches_transaction(&state, journal, old_state) {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "待恢复 state 不是本事务提交内容",
                ));
            }
        }
    } else if previous.is_some() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "待恢复 state 缺失",
        ));
    }

    let mut plans = Vec::new();
    for item in &journal.skills {
        let target_dir = target.join(&item.id);
        let kind = classify_transaction_target(&target_dir, item)?;
        let source = verified_old_source(txroot, &generation, item)?;
        match (item.action.as_str(), kind, source.is_some()) {
            ("add", TransactionTargetKind::New, false)
            | ("add", TransactionTargetKind::Missing, false)
            | ("replace", TransactionTargetKind::Missing, true)
            | ("replace", TransactionTargetKind::New, true)
            | ("replace", TransactionTargetKind::Old, false) => {}
            ("replace", TransactionTargetKind::Old, true) => {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("目标与旧目录证据同时存在: {}", item.id),
                ));
            }
            (_, _, _) => {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("事务旧目录证据不足: {}", item.id),
                ));
            }
        }
        plans.push((item, kind, source));
    }

    let quarantine = txroot.join("quarantine");
    if plans
        .iter()
        .any(|(_, kind, _)| *kind == TransactionTargetKind::New)
    {
        safe_create_dir_all(&quarantine, "创建恢复隔离目录失败")?;
    }
    for (item, kind, _) in &plans {
        if *kind == TransactionTargetKind::New {
            safe_rename(
                &target.join(&item.id),
                &quarantine.join(&item.id),
                "隔离事务新目录失败",
            )?;
        }
    }
    for (item, kind, source) in &plans {
        if item.action == "replace"
            && matches!(
                kind,
                TransactionTargetKind::Missing | TransactionTargetKind::New
            )
        {
            let source = source.as_ref().ok_or_else(|| {
                HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("旧目录来源缺失: {}", item.id),
                )
            })?;
            safe_rename(source, &target.join(&item.id), "恢复旧 Skill 目录失败")?;
        }
    }
    for (item, kind, _) in &plans {
        let target_dir = target.join(&item.id);
        match (item.action.as_str(), kind) {
            ("replace", TransactionTargetKind::Missing | TransactionTargetKind::New) => {
                if !current_files_match(&target_dir, &item.old_files)
                    || !directory_identity_matches(
                        &target_dir,
                        item.old_directory_volume_id.as_deref(),
                        item.old_directory_file_id.as_deref(),
                    )
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("恢复旧 Skill 后核对失败: {}", item.id),
                    ));
                }
            }
            ("add", TransactionTargetKind::New) => {
                safe_remove_dir_all(&quarantine.join(&item.id), "清理事务新目录隔离证据失败")?;
            }
            _ => {}
        }
    }
    if let Some((bytes, _)) = &previous {
        if current_state.as_ref() != Some(bytes) {
            write_atomic_json_bytes(&state_path, bytes)?;
            if safe_read_file(&state_path, "核对恢复 state 失败")? != *bytes {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "恢复 state 后字节内容变化",
                ));
            }
        }
    } else if current_state.is_some() {
        safe_remove_file(&state_path, "清理初始 state 失败")?;
    }
    for item in journal
        .skills
        .iter()
        .filter(|item| item.action == "replace")
    {
        let shadow = backup_shadow_path(&generation, &item.id);
        safe_remove_dir_all(&shadow, "清理 generation copy 证据失败")?;
        let generation_dir = generation.join(&item.id);
        if metadata_optional(&generation_dir)?.is_some()
            && !current_files_match(&generation_dir, &item.old_files)
        {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("generation 旧目录残留不一致: {}", item.id),
            ));
        }
    }
    Ok(())
}

fn write_atomic_json_bytes(path: &Path, bytes: &[u8]) -> Result<(), HotError> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("账本路径无父目录", path.display()))?;
    safe_create_dir_all(parent, "创建账本目录失败")?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    safe_write_new_file(&tmp, bytes, "创建账本临时文件失败")?;
    safe_replace_file(&tmp, path, "原子提交账本失败")?;
    if safe_read_file(path, "核对原子提交账本失败")? != bytes {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            format!("账本提交后字节内容变化: {}", path.display()),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryDecision {
    Rollback,
    Finalize,
}

fn recovery_decision(phase: &str, committed: bool) -> Option<RecoveryDecision> {
    match phase {
        "prepared" | "backed-up" | "swapped" | "rollback-required" => {
            Some(RecoveryDecision::Rollback)
        }
        "state-committed" | "done" if committed => Some(RecoveryDecision::Finalize),
        "state-committed" | "done" => Some(RecoveryDecision::Rollback),
        _ => None,
    }
}

fn rollback_directory_matches(
    path: &Path,
    records: &[FileRecord],
    volume_id: Option<&str>,
    file_id: Option<&str>,
) -> bool {
    metadata_optional(path)
        .ok()
        .flatten()
        .is_some_and(|metadata| {
            metadata.is_dir()
                && current_files_match(path, records)
                && directory_identity_matches(path, volume_id, file_id)
        })
}

fn resume_explicit_rollback(journal: &mut RollbackJournal) -> Result<(), HotError> {
    validate_rollback_journal(journal)?;
    let journal_path = rollback_journal_path()?;
    let txroot = transaction_root(&journal.transaction_id)?;
    let generation_root = generation_root(&journal.generation_id)?;
    if journal.phase == "done" {
        safe_remove_dir_all(&txroot, "清理已完成 rollback 事务失败")?;
        safe_remove_dir_all(&generation_root, "清理已完成 generation 失败")?;
        safe_remove_file(&journal_path, "清理已完成 rollback journal 失败")?;
        return Ok(());
    }

    let current_state_path = txroot.join(&journal.current_state_file);
    let current_state = safe_read_file(&current_state_path, "读取 rollback 当前 state 失败")?;
    if sha256(&current_state) != journal.current_state_sha256 {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "rollback journal 当前 state SHA-256 不匹配",
        ));
    }
    let generation = read_generation_from_disk(&generation_root, &journal.generation_id)?;
    let target = target_root()?;
    ensure_directory(&target, "读取 rollback 目标根失败")?;
    let previous_state = match (
        &generation.previous_state_file,
        &generation.previous_state_sha256,
    ) {
        (Some(name), Some(digest)) => {
            let bytes = safe_read_file(&generation_root.join(name), "读取 rollback 旧 state 失败")?;
            if sha256(&bytes) != *digest {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "rollback 旧 state SHA-256 不匹配",
                ));
            }
            Some(bytes)
        }
        (None, None) => None,
        _ => unreachable!(),
    };
    let quarantine = txroot.join("quarantine");

    for item in &journal.skills {
        let target_dir = target.join(&item.id);
        let quarantine_dir = quarantine.join(&item.id);
        let target_is_new = rollback_directory_matches(
            &target_dir,
            &item.new_files,
            item.new_directory_volume_id.as_deref(),
            item.new_directory_file_id.as_deref(),
        );
        let quarantine_is_new = rollback_directory_matches(
            &quarantine_dir,
            &item.new_files,
            item.new_directory_volume_id.as_deref(),
            item.new_directory_file_id.as_deref(),
        );
        if target_is_new {
            if quarantine_is_new || metadata_optional(&quarantine_dir)?.is_some() {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("rollback 新目录隔离证据重复: {}", item.id),
                ));
            }
            safe_create_dir_all(&quarantine, "创建 rollback 隔离目录失败")?;
            safe_rename(&target_dir, &quarantine_dir, "隔离 rollback 新目录失败")?;
        } else if !quarantine_is_new
            && !rollback_directory_matches(
                &target_dir,
                &item.old_files,
                item.old_directory_volume_id.as_deref(),
                item.old_directory_file_id.as_deref(),
            )
        {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("rollback 新目录状态无法恢复: {}", item.id),
            ));
        }
    }
    journal.phase = "quarantined".into();
    persist_rollback_journal(journal)?;

    for item in &journal.skills {
        let target_dir = target.join(&item.id);
        let old_target = rollback_directory_matches(
            &target_dir,
            &item.old_files,
            item.old_directory_volume_id.as_deref(),
            item.old_directory_file_id.as_deref(),
        );
        if let Some(relative) = &item.old_backup_relative {
            if !old_target {
                if metadata_optional(&target_dir)?.is_some() {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("rollback 目标存在但身份不匹配: {}", item.id),
                    ));
                }
                let old = generation_root.join(relative);
                if !rollback_directory_matches(
                    &old,
                    &item.old_files,
                    item.old_directory_volume_id.as_deref(),
                    item.old_directory_file_id.as_deref(),
                ) {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("rollback 旧目录来源无效: {}", item.id),
                    ));
                }
                safe_rename(&old, &target_dir, "恢复 rollback 旧 Skill 目录失败")?;
            }
            if !rollback_directory_matches(
                &target_dir,
                &item.old_files,
                item.old_directory_volume_id.as_deref(),
                item.old_directory_file_id.as_deref(),
            ) {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("rollback 恢复旧 Skill 后核对失败: {}", item.id),
                ));
            }
        } else if metadata_optional(&target_dir)?.is_some() {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("rollback 新增 Skill 未能移除: {}", item.id),
            ));
        }
    }
    journal.phase = "swapped".into();
    persist_rollback_journal(journal)?;

    let state_path = state_path()?;
    let state_now = metadata_optional(&state_path)?
        .map(|_| safe_read_file(&state_path, "读取 rollback state 失败"))
        .transpose()?;
    match &previous_state {
        Some(previous) => {
            if state_now.as_deref() != Some(previous.as_slice()) {
                if state_now.as_deref() != Some(current_state.as_slice()) {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        "rollback state 不是事务开始时的 state",
                    ));
                }
                write_atomic_json_bytes(&state_path, previous)?;
            }
        }
        None => {
            if state_now.is_some() && state_now.as_deref() != Some(current_state.as_slice()) {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "rollback 初始 state 不是事务开始时的 state",
                ));
            }
            if state_now.is_some() {
                safe_remove_file(&state_path, "清理 rollback 初始 state 失败")?;
            }
        }
    }
    journal.phase = "state-committed".into();
    persist_rollback_journal(journal)?;

    journal.phase = "done".into();
    persist_rollback_journal(journal)?;
    safe_remove_dir_all(&quarantine, "清理 rollback 隔离目录失败")?;
    safe_remove_dir_all(&generation_root, "清理已回退 generation 失败")?;
    safe_remove_dir_all(&txroot, "清理 rollback 事务目录失败")?;
    safe_remove_file(&journal_path, "清理 rollback journal 失败")?;
    Ok(())
}

fn recover_explicit_rollback_pending() -> Result<(), HotError> {
    let path = rollback_journal_path()?;
    if metadata_optional(&path)?.is_none() {
        return Ok(());
    }
    let bytes = safe_read_file(&path, "读取 rollback journal 失败")?;
    let mut journal: RollbackJournal =
        serde_json::from_slice(&bytes).map_err(|e| io_error("rollback journal 格式无效", e))?;
    resume_explicit_rollback(&mut journal)
}

fn cleanup_transaction(
    txroot: &Path,
    generation: &Path,
    remove_generation: bool,
) -> Result<(), HotError> {
    safe_remove_dir_all(txroot, "清理事务目录失败")?;
    if remove_generation {
        safe_remove_dir_all(generation, "清理 generation 失败")?;
    }
    Ok(())
}

fn transaction_is_committed(
    txroot: &Path,
    journal: &Journal,
    target: &Path,
) -> Result<bool, HotError> {
    validate_journal(journal)?;
    ensure_directory(target, "核对事务目标根失败")?;
    let generation_root = generation_root(&journal.backup_generation)?;
    let generation = match read_generation_from_disk(&generation_root, &journal.backup_generation) {
        Ok(generation) => generation,
        Err(error) if error.code == "SKILL_REPO_ROLLBACK_FAILED" => return Ok(false),
        Err(error) => return Err(error),
    };
    if generation.commit != journal.expected_identity.commit {
        return Ok(false);
    }
    let previous_state = read_previous_state_for_transaction(txroot, journal)?;
    let state_path = state_path()?;
    let state_bytes = match metadata_optional(&state_path)? {
        Some(_) => safe_read_file(&state_path, "读取已提交 state 失败")?,
        None => return Ok(false),
    };
    let state: StateFile = match serde_json::from_slice(&state_bytes) {
        Ok(state) => state,
        Err(_) => return Ok(false),
    };
    if !state_matches_transaction(
        &state,
        journal,
        previous_state.as_ref().map(|(_, state)| state),
    ) {
        return Ok(false);
    }
    for item in &journal.skills {
        let target_dir = target.join(&item.id);
        if !current_files_match(&target_dir, &item.new_files)
            || !directory_identity_matches(
                &target_dir,
                item.new_directory_volume_id.as_deref(),
                item.new_directory_file_id.as_deref(),
            )
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn recover_pending() -> Result<(), HotError> {
    let path = journal_path()?;
    if metadata_optional(&path)?.is_some() {
        let bytes = safe_read_file(&path, "读取事务 journal 失败")?;
        let mut journal: Journal =
            serde_json::from_slice(&bytes).map_err(|e| io_error("事务 journal 格式无效", e))?;
        validate_journal(&journal)?;
        let txroot = transaction_root(&journal.transaction_id)?;
        let generation = generation_root(&journal.backup_generation)?;
        let target = target_root()?;
        let committed = matches!(journal.phase.as_str(), "state-committed" | "done")
            && transaction_is_committed(&txroot, &journal, &target)?;
        let decision = recovery_decision(&journal.phase, committed).ok_or_else(|| {
            HotError::new("SKILL_REPO_ROLLBACK_FAILED", "事务 journal phase 不受支持")
        })?;
        match decision {
            RecoveryDecision::Finalize => {
                cleanup_transaction(&txroot, &generation, false)?;
                journal.phase = "done".into();
                persist_journal(&journal)?;
                safe_remove_file(&path, "清理已完成 journal 失败")?;
            }
            RecoveryDecision::Rollback => {
                journal.phase = "rollback-required".into();
                persist_journal(&journal)?;
                restore_transaction(&txroot, &journal, &target)?;
                cleanup_transaction(&txroot, &generation, true)?;
                safe_remove_file(&path, "清理恢复 journal 失败")?;
            }
        }
    }
    recover_explicit_rollback_pending()
}

fn apply_archive(
    expected: SkillRepoExpected,
    skills: Vec<CandidateSkill>,
    app: &AppHandle,
) -> Result<SkillRepoApplyReport, HotError> {
    if !expected_identity_valid(&expected) {
        return Err(HotError::new(
            "SKILL_REPO_SOURCE_INVALID",
            "expected identity 无效",
        ));
    }
    let mut state = read_state()?;
    let base = base_manifest(app)?;
    ensure_state_initialized()?;
    let ids = base
        .skills
        .into_iter()
        .map(|x| x.id)
        .collect::<BTreeSet<_>>();
    let root = target_root()?;
    safe_create_dir_all(&root, "创建目标 Skill 根失败")?;
    let mut conflicts = Vec::new();
    let mut selected = Vec::new();
    for skill in skills {
        let (status, count) = plan_item(&skill.id, &state, &ids, &root)?;
        if status == "apply" {
            selected.push((skill, count));
        } else {
            conflicts.push(SkillRepoItem {
                id: skill.id,
                status: "conflict".into(),
                code: Some(status),
                file_count: count,
            });
        }
    }
    if selected.is_empty() {
        state.last_apply_status = if conflicts.is_empty() {
            "success".into()
        } else {
            "partial".into()
        };
        write_atomic_json(&state_path()?, &state)?;
        return Ok(SkillRepoApplyReport {
            schema_version: 1,
            status: state.last_apply_status,
            identity: expected,
            applied: Vec::new(),
            conflicts,
            skipped: Vec::new(),
            failed: Vec::new(),
            last_good_generation: state.last_good_generation,
            reloaded_required: false,
            log_path: None,
        });
    }

    let txid = Uuid::new_v4().to_string();
    let generation_id = Uuid::new_v4().to_string();
    let txroot = transaction_root(&txid)?;
    let stage = txroot.join("skills");
    let rollback = txroot.join("rollback");
    let generation = generation_root(&generation_id)?;
    if metadata_optional(&txroot)?.is_some() || metadata_optional(&generation)?.is_some() {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "事务或 generation 目录已存在",
        ));
    }
    safe_create_dir_all(&stage, "创建事务 staging 失败")?;
    safe_create_dir_all(&rollback, "创建事务 rollback 失败")?;
    safe_create_dir_all(&generation, "创建 generation 失败")?;
    let state_file_path = state_path()?;
    let old_state_bytes = if metadata_optional(&state_file_path)?.is_some() {
        Some(safe_read_file(&state_file_path, "读取旧 state 失败")?)
    } else {
        None
    };
    let previous_sha = write_previous_state(&txroot, old_state_bytes.as_deref())?;
    let mut journal = Journal {
        schema_version: 1,
        transaction_id: txid.clone(),
        phase: "prepared".into(),
        expected_identity: expected.clone(),
        previous_state_sha256: previous_sha.clone(),
        previous_state_file: previous_sha.as_ref().map(|_| "previous-state.json".into()),
        backup_generation: generation_id.clone(),
        rollback_root: "rollback".into(),
        skills: Vec::new(),
    };
    let stage_result: Result<(), HotError> = (|| {
        for (skill, _) in &selected {
            let staged = stage.join(&skill.id);
            let new_files = write_skill(&staged, skill)?;
            if !current_files_match(&staged, &new_files) {
                return Err(HotError::new(
                    "SKILL_REPO_ARCHIVE_INVALID",
                    "staging 文件核对失败",
                ));
            }
            let (new_directory_volume_id, new_directory_file_id) = file_identity(&staged)?;
            let old = state.skills.get(&skill.id).cloned();
            journal.skills.push(JournalSkill {
                id: skill.id.clone(),
                action: if old.is_some() {
                    "replace".into()
                } else {
                    "add".into()
                },
                target_relative: skill.id.clone(),
                backup_relative: old
                    .as_ref()
                    .map(|_| format!("{generation_id}/{}", skill.id)),
                rollback_relative: old.as_ref().map(|_| skill.id.clone()),
                old_installed_commit: old.as_ref().map(|v| v.installed_commit.clone()),
                new_installed_commit: expected.commit.clone(),
                old_directory_volume_id: old.as_ref().and_then(|v| v.directory_volume_id.clone()),
                old_directory_file_id: old.as_ref().and_then(|v| v.directory_file_id.clone()),
                new_directory_volume_id: Some(new_directory_volume_id),
                new_directory_file_id: Some(new_directory_file_id),
                old_files: old.map(|v| v.files).unwrap_or_default(),
                new_files,
            });
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        return match cleanup_transaction(&txroot, &generation, true) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    validate_journal(&journal)?;
    persist_journal(&journal)?;
    let result: Result<SkillRepoApplyReport, HotError> = (|| {
        for item in &journal.skills {
            if item.action == "replace" {
                let target_dir = root.join(&item.id);
                let backup_dir = generation.join(&item.id);
                if !current_files_match(&target_dir, &item.old_files)
                    || !directory_identity_matches(
                        &target_dir,
                        item.old_directory_volume_id.as_deref(),
                        item.old_directory_file_id.as_deref(),
                    )
                {
                    return Err(HotError::new(
                        "SKILL_REPO_HOT_MODIFIED",
                        format!("备份前目标 Skill 已变化: {}", item.id),
                    ));
                }
                copy_dir(&target_dir, &backup_dir)?;
                if !current_content_matches(&backup_dir, &item.old_files) {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("备份内容清单核对失败: {}", item.id),
                    ));
                }
                let shadow = backup_shadow_path(&generation, &item.id);
                safe_rename(&backup_dir, &shadow, "保存备份 copy 证据失败")?;
                let rollback_dir = rollback.join(&item.id);
                safe_rename(&target_dir, &rollback_dir, "移动旧 Skill 目录失败")?;
                if !current_files_match(&rollback_dir, &item.old_files)
                    || !directory_identity_matches(
                        &rollback_dir,
                        item.old_directory_volume_id.as_deref(),
                        item.old_directory_file_id.as_deref(),
                    )
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("移动后原目录身份核对失败: {}", item.id),
                    ));
                }
                safe_rename(&rollback_dir, &backup_dir, "保存原目录身份备份失败")?;
                if !current_files_match(&backup_dir, &item.old_files)
                    || !directory_identity_matches(
                        &backup_dir,
                        item.old_directory_volume_id.as_deref(),
                        item.old_directory_file_id.as_deref(),
                    )
                {
                    return Err(HotError::new(
                        "SKILL_REPO_ROLLBACK_FAILED",
                        format!("generation 原目录身份核对失败: {}", item.id),
                    ));
                }
                safe_remove_dir_all(&shadow, "清理备份 copy 证据失败")?;
            }
        }
        journal.phase = "backed-up".into();
        validate_journal(&journal)?;
        persist_journal(&journal)?;
        let mut applied = Vec::new();
        for item in &mut journal.skills {
            let staged = stage.join(&item.id);
            let target_dir = root.join(&item.id);
            safe_rename(&staged, &target_dir, "提交 Skill 目录失败")?;
            let (volume_id, file_id) = file_identity(&target_dir)?;
            if item.new_directory_volume_id.as_deref() != Some(volume_id.as_str())
                || item.new_directory_file_id.as_deref() != Some(file_id.as_str())
            {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("新目录 rename 后身份变化: {}", item.id),
                ));
            }
            if !current_files_match(&target_dir, &item.new_files) {
                return Err(HotError::new(
                    "SKILL_REPO_ARCHIVE_INVALID",
                    format!("目标 Skill 核对失败: {}", item.id),
                ));
            }
            state.skills.insert(
                item.id.clone(),
                SkillState {
                    installed_commit: expected.commit.clone(),
                    directory_volume_id: Some(volume_id),
                    directory_file_id: Some(file_id),
                    files: item.new_files.clone(),
                },
            );
            applied.push(SkillRepoItem {
                id: item.id.clone(),
                status: "applied".into(),
                code: None,
                file_count: item.new_files.len() as u32,
            });
        }
        journal.phase = "swapped".into();
        validate_journal(&journal)?;
        persist_journal(&journal)?;
        let generation_skills = journal
            .skills
            .iter()
            .map(|item| GenerationSkill {
                id: item.id.clone(),
                installed_commit: item
                    .old_installed_commit
                    .clone()
                    .unwrap_or_else(|| expected.commit.clone()),
                backup_relative: item.rollback_relative.clone(),
                directory_volume_id: item.old_directory_volume_id.clone(),
                directory_file_id: item.old_directory_file_id.clone(),
                files: item.old_files.clone(),
                new_files: item.new_files.clone(),
            })
            .collect::<Vec<_>>();
        let generation_value = Generation {
            schema_version: 1,
            generation_id: generation_id.clone(),
            commit: expected.commit.clone(),
            complete: true,
            previous_state_file: journal.previous_state_file.clone(),
            previous_state_sha256: journal.previous_state_sha256.clone(),
            skills: generation_skills,
        };
        validate_generation_value(&generation_value, &generation_id)?;
        write_atomic_json(&generation.join("generation.json"), &generation_value)?;
        if let Some(previous) = old_state_bytes.as_deref() {
            write_atomic_json_bytes(&generation.join("previous-state.json"), previous)?;
        }
        let generation_bytes =
            safe_read_file(&generation.join("generation.json"), "读取 generation 失败")?;
        safe_write_new_file(
            &generation.join("complete.marker"),
            sha256(&generation_bytes).as_bytes(),
            "写入 generation marker 失败",
        )?;
        let _ = read_generation_from_disk(&generation, &generation_id)?;
        state.last_apply_status = if conflicts.is_empty() {
            "success".into()
        } else {
            "partial".into()
        };
        if conflicts.is_empty() {
            state.last_seen_commit = Some(expected.commit.clone());
        }
        state.last_good_generation = Some(generation_id.clone());
        validate_state_file(&state)?;
        write_atomic_json(&state_path()?, &state)?;
        if read_state()? != state {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                "state 提交后内容核对失败",
            ));
        }
        journal.phase = "state-committed".into();
        validate_journal(&journal)?;
        persist_journal(&journal)?;
        cleanup_transaction(&txroot, &generation, false)?;
        journal.phase = "done".into();
        validate_journal(&journal)?;
        persist_journal(&journal)?;
        safe_remove_file(&journal_path()?, "清理已完成 journal 失败")?;
        Ok(SkillRepoApplyReport {
            schema_version: 1,
            status: state.last_apply_status.clone(),
            identity: expected.clone(),
            applied,
            conflicts: conflicts.clone(),
            skipped: Vec::new(),
            failed: Vec::new(),
            last_good_generation: state.last_good_generation.clone(),
            reloaded_required: true,
            log_path: None,
        })
    })();
    match result {
        Ok(report) => Ok(report),
        Err(error) => {
            if matches!(journal.phase.as_str(), "state-committed" | "done") {
                return Err(error);
            }
            journal.phase = "rollback-required".into();
            persist_journal(&journal)?;
            restore_transaction(&txroot, &journal, &root)?;
            cleanup_transaction(&txroot, &generation, true)?;
            safe_remove_file(&journal_path()?, "清理恢复 journal 失败")?;
            Err(error)
        }
    }
}

pub async fn check_update(
    app: &AppHandle,
    ops: &OperationManager,
) -> Result<SkillRepoUpdateInfo, CommandError> {
    let _guard = ops
        .begin(crate::app::oplock::OperationKind::Update)
        .map_err(|e| CommandError::from(crate::errors::AppError::from(e)))?;
    #[cfg(not(windows))]
    {
        return Err(
            HotError::new("SKILL_REPO_UNSUPPORTED", "Skill Hub 热更新仅支持 Windows").into(),
        );
    }
    recover_pending().map_err(CommandError::from)?;
    let client = client().map_err(CommandError::from)?;
    let (expected, archive) = resolve_latest_with_deadline(&client)
        .await
        .map_err(CommandError::from)?;
    let staging = state_dir()
        .map_err(CommandError::from)?
        .join("staging")
        .join(Uuid::new_v4().to_string());
    safe_create_dir_all(&staging, "创建 check staging 失败").map_err(CommandError::from)?;
    let result = (|| -> Result<SkillRepoUpdateInfo, CommandError> {
        safe_write_new_file(
            &staging.join("archive.zip"),
            &archive,
            "写入 check staging 失败",
        )
        .map_err(CommandError::from)?;
        let skills = parse_archive(&archive).map_err(CommandError::from)?;
        let state = read_state().map_err(CommandError::from)?;
        let base = base_manifest(app).map_err(CommandError::from)?;
        let ids = base
            .skills
            .into_iter()
            .map(|x| x.id)
            .collect::<BTreeSet<_>>();
        let root = target_root().map_err(CommandError::from)?;
        let mut installable = 0;
        let mut conflicts = 0;
        for s in &skills {
            match plan_item(&s.id, &state, &ids, &root) {
                Ok((status, _)) if status == "apply" => installable += 1,
                Ok(_) => conflicts += 1,
                Err(_) => conflicts += 1,
            }
        }
        let current = state.last_seen_commit.clone();
        Ok(SkillRepoUpdateInfo {
            schema_version: 1,
            current_commit: current.clone(),
            latest_commit: expected.commit.clone(),
            source_kind: expected.source_kind,
            archive_size: expected.archive_size,
            archive_sha256: expected.archive_sha256.clone(),
            expected: expected.clone(),
            update_available: current.as_deref() != Some(expected.commit.as_str()),
            installable_count: installable,
            conflict_count: conflicts,
            skipped_count: 0,
            // Skill Hub is an intentionally independent capability overlay. It
            // does not change the signed Codex bundle or USB authorization, so
            // its normal update result is not a warning state.
            warning: None,
        })
    })();
    let cleanup = safe_remove_dir_all(&staging, "清理 check staging 失败");
    match (result, cleanup) {
        (Ok(info), Ok(())) => Ok(info),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error.into()),
    }
}

pub async fn apply_update(
    app: &AppHandle,
    ops: &OperationManager,
    expected: SkillRepoExpected,
) -> Result<SkillRepoApplyReport, CommandError> {
    let _guard = ops
        .begin(crate::app::oplock::OperationKind::Update)
        .map_err(|e| CommandError::from(crate::errors::AppError::from(e)))?;
    #[cfg(not(windows))]
    {
        return Err(
            HotError::new("SKILL_REPO_UNSUPPORTED", "Skill Hub 热更新仅支持 Windows").into(),
        );
    }
    recover_pending().map_err(CommandError::from)?;
    let client = client().map_err(CommandError::from)?;
    let archive = download_expected_with_deadline(&client, &expected)
        .await
        .map_err(CommandError::from)?;
    let skills = parse_archive(&archive).map_err(CommandError::from)?;
    apply_archive(expected, skills, app).map_err(CommandError::from)
}

pub async fn rollback(
    app: &AppHandle,
    ops: &OperationManager,
) -> Result<SkillRepoRollbackReport, CommandError> {
    let _guard = ops
        .begin(crate::app::oplock::OperationKind::Update)
        .map_err(|e| CommandError::from(crate::errors::AppError::from(e)))?;
    #[cfg(not(windows))]
    {
        return Err(
            HotError::new("SKILL_REPO_UNSUPPORTED", "Skill Hub 热更新仅支持 Windows").into(),
        );
    }
    let _ = base_manifest(app).map_err(CommandError::from)?;
    recover_pending().map_err(CommandError::from)?;
    let state = read_state().map_err(CommandError::from)?;
    let Some(id) = state.last_good_generation.clone() else {
        return Err(HotError::new("SKILL_REPO_ROLLBACK_FAILED", "没有可回退 generation").into());
    };
    let root = target_root().map_err(CommandError::from)?;
    ensure_directory(&root, "读取目标 Skill 根失败").map_err(CommandError::from)?;
    let backup = generation_root(&id).map_err(CommandError::from)?;
    let gen = read_generation_from_disk(&backup, &id).map_err(CommandError::from)?;
    let state_path = state_path().map_err(CommandError::from)?;
    let current_state_bytes =
        safe_read_file(&state_path, "读取当前 state 失败").map_err(CommandError::from)?;
    let rollback_txid = Uuid::new_v4().to_string();
    let rollback_txroot = transaction_root(&rollback_txid).map_err(CommandError::from)?;
    let mut plans = Vec::new();
    for g in &gen.skills {
        let target_dir = root.join(&g.id);
        let Some(current) = state.skills.get(&g.id) else {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("当前 state 缺少 Skill: {}", g.id),
            )
            .into());
        };
        if current.installed_commit != gen.commit || current.files != g.new_files {
            return Err(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("当前 state 与 generation 新清单不一致: {}", g.id),
            )
            .into());
        }
        let metadata = metadata_optional(&target_dir).map_err(CommandError::from)?;
        if metadata.is_none()
            || !current_files_match(&target_dir, &g.new_files)
            || !directory_identity_matches(
                &target_dir,
                current.directory_volume_id.as_deref(),
                current.directory_file_id.as_deref(),
            )
        {
            if metadata.is_some() && current_content_matches(&target_dir, &g.new_files) {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("rollback 新目录 file-id 或目录身份变化: {}", g.id),
                )
                .into());
            }
            return Err(HotError::new(
                "SKILL_REPO_HOT_MODIFIED",
                format!("rollback 检测到用户修改或目录缺失: {}", g.id),
            )
            .into());
        }
        let old = if let Some(rel) = &g.backup_relative {
            let old = backup.join(rel);
            if !current_files_match(&old, &g.files)
                || !directory_identity_matches(
                    &old,
                    g.directory_volume_id.as_deref(),
                    g.directory_file_id.as_deref(),
                )
            {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    format!("旧 Skill 备份身份或清单无效: {}", g.id),
                )
                .into());
            }
            Some(old)
        } else {
            None
        };
        plans.push((g, target_dir, old));
    }
    let _previous_state = match (&gen.previous_state_file, &gen.previous_state_sha256) {
        (Some(name), Some(digest)) => {
            let previous =
                safe_read_file(&backup.join(name), "读取 generation previous state 失败")
                    .map_err(CommandError::from)?;
            if sha256(&previous) != *digest {
                return Err(HotError::new(
                    "SKILL_REPO_ROLLBACK_FAILED",
                    "generation previous state SHA-256 不匹配",
                )
                .into());
            }
            let parsed: StateFile = serde_json::from_slice(&previous)
                .map_err(|e| io_error("generation previous state 格式无效", e))
                .map_err(CommandError::from)?;
            validate_state_file(&parsed).map_err(CommandError::from)?;
            Some(previous)
        }
        (None, None) => None,
        _ => unreachable!(),
    };
    let rollback_path = rollback_journal_path().map_err(CommandError::from)?;
    if metadata_optional(&rollback_path)
        .map_err(CommandError::from)?
        .is_some()
    {
        return Err(HotError::new(
            "SKILL_REPO_ROLLBACK_FAILED",
            "已有 rollback journal，必须先恢复未完成事务",
        )
        .into());
    }
    safe_create_dir_all(&rollback_txroot, "创建 rollback 事务目录失败")
        .map_err(CommandError::from)?;
    let current_state_file = rollback_txroot.join("current-state.json");
    safe_write_new_file(
        &current_state_file,
        &current_state_bytes,
        "保存 rollback 当前 state 失败",
    )
    .map_err(CommandError::from)?;
    let mut journal_skills = Vec::with_capacity(plans.len());
    for (g, _, _) in &plans {
        let current = state.skills.get(&g.id).ok_or_else(|| {
            CommandError::from(HotError::new(
                "SKILL_REPO_ROLLBACK_FAILED",
                format!("rollback state 缺少技能: {}", g.id),
            ))
        })?;
        journal_skills.push(RollbackJournalSkill {
            id: g.id.clone(),
            old_backup_relative: g.backup_relative.clone(),
            old_directory_volume_id: g.directory_volume_id.clone(),
            old_directory_file_id: g.directory_file_id.clone(),
            old_files: g.files.clone(),
            new_directory_volume_id: current.directory_volume_id.clone(),
            new_directory_file_id: current.directory_file_id.clone(),
            new_files: g.new_files.clone(),
        });
    }
    let mut journal = RollbackJournal {
        schema_version: 1,
        transaction_id: rollback_txid,
        phase: "prepared".into(),
        generation_id: id.clone(),
        current_state_file: "current-state.json".into(),
        current_state_sha256: sha256(&current_state_bytes),
        skills: journal_skills,
    };
    validate_rollback_journal(&journal).map_err(CommandError::from)?;
    persist_rollback_journal(&journal).map_err(CommandError::from)?;
    resume_explicit_rollback(&mut journal).map_err(CommandError::from)?;
    let restored = gen.skills.iter().map(|g| g.id.clone()).collect();
    Ok(SkillRepoRollbackReport {
        schema_version: 1,
        status: "success".into(),
        restored_generation: Some(id),
        restored_skills: restored,
        reloaded_required: true,
        log_path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_fixture(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default();
        for (name, bytes, directory) in entries {
            if *directory {
                writer.add_directory(*name, options).expect("add directory");
            } else {
                writer.start_file(*name, options).expect("add file");
                writer.write_all(bytes).expect("write file");
            }
        }
        writer.finish().expect("finish zip");
        output.into_inner()
    }

    fn ledger_record(file_id: &str) -> FileRecord {
        FileRecord {
            path: "skill.md".into(),
            size: 1,
            sha256: "a".repeat(64),
            volume_id: Some("volume-1".into()),
            file_id: Some(file_id.into()),
        }
    }

    #[test]
    fn ids_are_strict_and_lowercase() {
        assert!(valid_id("skill-a"));
        assert!(valid_id("a1._-"));
        assert!(!valid_id("Skill"));
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
    }

    #[test]
    fn rejects_zip_paths() {
        assert!(normalize_zip_path("../x", false).is_err());
        assert!(normalize_zip_path("a/", false).is_err());
        assert!(normalize_zip_path("a/", true).is_ok());
        assert!(normalize_zip_path("CON.txt", false).is_err());
        assert!(normalize_zip_path("a/evil.exe", false).is_ok());
        assert!(normalize_zip_path("技能/skill.md", false).is_err());
    }

    #[test]
    fn archive_fixture_rejects_collision_and_native_binary() {
        let prefix = "skill_hub-0123456789012345678901234567890123456789/";
        let collision_skill = format!("{prefix}demo/SKILL.md");
        let collision_duplicate = format!("{prefix}demo/skill.md");
        let collision = zip_fixture(&[
            (
                collision_skill.as_str(),
                b"---\nname: demo\ndescription: x\n---\n",
                false,
            ),
            (collision_duplicate.as_str(), b"duplicate", false),
        ]);
        assert!(parse_archive(&collision).is_err());

        let binary_skill = format!("{prefix}demo/SKILL.md");
        let binary_payload = format!("{prefix}demo/payload.bin");
        let binary = zip_fixture(&[
            (
                binary_skill.as_str(),
                b"---\nname: demo\ndescription: x\n---\n",
                false,
            ),
            (binary_payload.as_str(), b"MZ\0\0", false),
        ]);
        assert!(parse_archive(&binary).is_err());
    }

    #[test]
    fn frontmatter_is_minimal_and_exact() {
        let b = b"---\nname: demo\ndescription: x\n---\nbody";
        assert!(validate_frontmatter("demo", b).is_ok());
        assert!(validate_frontmatter("other", b).is_err());
        assert!(validate_frontmatter("demo", b"---\nname:demo\ndescription: x\n---\n").is_err());
        assert!(validate_frontmatter(
            "demo",
            b"\xef\xbb\xbf---\nname: demo\ndescription: x\n---\n"
        )
        .is_err());
    }

    #[test]
    fn rejects_dangerous_extensions_and_magic() {
        assert!(validate_payload_file("tool.EXE", b"text").is_err());
        assert!(validate_payload_file("driver.sys", b"text").is_err());
        assert!(validate_payload_file("payload.bin", b"MZ\x90\0").is_err());
        assert!(validate_payload_file("payload.bin", b"\x7fELF\0").is_err());
        assert!(validate_payload_file("payload.bin", b"\xfe\xed\xfa\xce").is_err());
        assert!(validate_payload_file("notes.txt", b"plain text").is_ok());
    }

    #[test]
    fn ledger_records_require_content_and_identity() {
        let record = ledger_record("file-1");
        assert!(validate_records(std::slice::from_ref(&record), false).is_ok());

        let mut missing_identity = record.clone();
        missing_identity.file_id = None;
        assert!(validate_record(&missing_identity).is_err());

        let mut invalid_path = record.clone();
        invalid_path.path = "../skill.md".into();
        assert!(validate_record(&invalid_path).is_err());

        let mut duplicate = record.clone();
        duplicate.file_id = Some("file-2".into());
        assert!(validate_records(&[record, duplicate], false).is_err());
    }

    #[test]
    fn journal_records_and_recovery_phases_are_fail_closed() {
        let expected = SkillRepoExpected {
            schema_version: 1,
            repo: REPO.into(),
            branch: BRANCH.into(),
            commit: "a".repeat(40),
            source_kind: SourceKind::Github,
            archive_size: 0,
            archive_sha256: "b".repeat(64),
        };
        let journal = Journal {
            schema_version: 1,
            transaction_id: Uuid::from_u128(1).to_string(),
            phase: "prepared".into(),
            expected_identity: expected.clone(),
            previous_state_sha256: None,
            previous_state_file: None,
            backup_generation: Uuid::from_u128(2).to_string(),
            rollback_root: "rollback".into(),
            skills: vec![JournalSkill {
                id: "demo".into(),
                action: "add".into(),
                target_relative: "demo".into(),
                backup_relative: None,
                rollback_relative: None,
                old_installed_commit: None,
                new_installed_commit: expected.commit.clone(),
                old_directory_volume_id: None,
                old_directory_file_id: None,
                new_directory_volume_id: Some("volume-new".into()),
                new_directory_file_id: Some("directory-new".into()),
                old_files: Vec::new(),
                new_files: vec![ledger_record("file-new")],
            }],
        };
        assert!(validate_journal(&journal).is_ok());

        let mut malformed = journal.clone();
        malformed.skills[0].new_files[0].sha256 = "bad".into();
        assert!(validate_journal(&malformed).is_err());

        assert_eq!(
            recovery_decision("prepared", false),
            Some(RecoveryDecision::Rollback)
        );
        assert_eq!(
            recovery_decision("backed-up", false),
            Some(RecoveryDecision::Rollback)
        );
        assert_eq!(
            recovery_decision("swapped", false),
            Some(RecoveryDecision::Rollback)
        );
        assert_eq!(
            recovery_decision("state-committed", true),
            Some(RecoveryDecision::Finalize)
        );
        assert_eq!(
            recovery_decision("state-committed", false),
            Some(RecoveryDecision::Rollback)
        );
        assert!(recovery_decision("unknown", true).is_none());
    }

    #[test]
    fn rollback_journal_rejects_ambiguous_or_incomplete_records() {
        let mut journal = RollbackJournal {
            schema_version: 1,
            transaction_id: Uuid::from_u128(3).to_string(),
            phase: "prepared".into(),
            generation_id: Uuid::from_u128(4).to_string(),
            current_state_file: "current-state.json".into(),
            current_state_sha256: "c".repeat(64),
            skills: vec![RollbackJournalSkill {
                id: "demo".into(),
                old_backup_relative: None,
                old_directory_volume_id: None,
                old_directory_file_id: None,
                old_files: Vec::new(),
                new_directory_volume_id: Some("volume-new".into()),
                new_directory_file_id: Some("file-new".into()),
                new_files: vec![ledger_record("file-new")],
            }],
        };
        assert!(validate_rollback_journal(&journal).is_ok());

        journal.phase = "unknown".into();
        assert!(validate_rollback_journal(&journal).is_err());
        journal.phase = "prepared".into();
        journal.current_state_file = "../current-state.json".into();
        assert!(validate_rollback_journal(&journal).is_err());
        journal.current_state_file = "current-state.json".into();
        journal.skills[0].old_backup_relative = Some("other".into());
        assert!(validate_rollback_journal(&journal).is_err());
        journal.skills[0].old_backup_relative = None;
        let duplicate = journal.skills[0].clone();
        journal.skills.push(duplicate);
        assert!(validate_rollback_journal(&journal).is_err());
    }

    #[test]
    fn initial_state_serialization_is_byte_stable() {
        let bytes = serde_json::to_vec(&empty_state()).expect("state serializes");
        assert_eq!(
            bytes,
            br#"{"schemaVersion":1,"repo":"meta-xucong/skill_hub","branch":"master","lastSeenCommit":null,"lastApplyStatus":"never","lastGoodGeneration":null,"skills":{}}"#
        );
    }
}
