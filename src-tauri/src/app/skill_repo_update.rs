//! Independent, unsigned Skill Hub update layer.
//!
//! This module deliberately does not depend on USB guards, capability-pack
//! installation, or the Codex payload updater.  It shares the manager's
//! persisted proxy setting and OperationManager lock, supplied/read at the
//! command boundary.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tokio::task::JoinSet;
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

use crate::app::oplock::OperationManager;
use crate::app::settings_store::{AppSettings, ProxyMode};
use crate::app::url_guard::validate_custom_proxy;
use crate::errors::CommandError;

const REPO: &str = "meta-xucong/skill_hub";
const BRANCH: &str = "master";
const SOURCE_RESOURCE: &str = "skill_hub_sources.json";
const BASE_MANIFEST_RESOURCE: &str = "skill_repo_base_manifest.json";
const EMBEDDED_SOURCE_DIRECTORY: &[u8] = include_bytes!("../../resources/skill_hub_sources.json");
const EMBEDDED_BASE_MANIFEST: &[u8] =
    include_bytes!("../../resources/skill_repo_base_manifest.json");
const RELAY_BASE: &str = "https://update.404token.xyz/__codex_update__/skill-hub/";
const GITHUB_COMMITS: &str = "https://api.github.com/repos/meta-xucong/skill_hub/commits/master";
const GITHUB_ARCHIVE: &str = "https://github.com/meta-xucong/skill_hub/archive/";
const MAX_ARCHIVE: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_UNPACKED: usize = 256 * 1024 * 1024;
const MAX_FILE: usize = 32 * 1024 * 1024;
const NETWORK_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const NETWORK_METADATA_TIMEOUT: Duration = Duration::from_secs(8);
const NETWORK_ARCHIVE_TIMEOUT: Duration = Duration::from_secs(20);
const NETWORK_OPERATION_TIMEOUT: Duration = Duration::from_secs(45);
const NETWORK_RETRY_BACKOFF: Duration = Duration::from_millis(250);
const BASE_MANIFEST_SHA256: &str =
    "6effd655adbfc6f3be80b1c8348c2da2c428ef9a3d1b77b0c9da05da055e9953";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Domestic,
    Relay,
    Github,
}

impl SourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Domestic => "domestic",
            Self::Relay => "relay",
            Self::Github => "github",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillRepoAttemptPhase {
    Metadata,
    Archive,
    Verify,
}

impl SkillRepoAttemptPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Archive => "archive",
            Self::Verify => "verify",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillRepoAttemptResult {
    Ok,
    Timeout,
    Unavailable,
    Http,
    Invalid,
    HashMismatch,
    Cancelled,
}

impl SkillRepoAttemptResult {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::Http => "http",
            Self::Invalid => "invalid",
            Self::HashMismatch => "hash-mismatch",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoSourceAttempt {
    pub source_id: String,
    pub source_kind: SourceKind,
    pub phase: SkillRepoAttemptPhase,
    pub result: SkillRepoAttemptResult,
    pub http_status: Option<u16>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillHubSourceEntry {
    id: String,
    kind: SourceKind,
    base: Option<String>,
    priority: u32,
    enabled: bool,
    #[serde(default)]
    test_fixture: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillHubSourceDirectory {
    schema_version: u8,
    channel: String,
    sources: Vec<SkillHubSourceEntry>,
}

impl SkillHubSourceDirectory {
    fn source(&self, source_id: &str) -> Option<&SkillHubSourceEntry> {
        self.sources.iter().find(|source| source.id == source_id)
    }

    fn enabled_domestic(&self) -> Vec<SkillHubSourceEntry> {
        let mut sources = self
            .sources
            .iter()
            .filter(|source| source.enabled && source.kind == SourceKind::Domestic)
            .cloned()
            .collect::<Vec<_>>();
        sources.sort_by_key(|source| (source.priority, source.id.clone()));
        sources
    }

    fn enabled_source(&self, kind: SourceKind) -> Option<SkillHubSourceEntry> {
        self.sources
            .iter()
            .find(|source| source.enabled && source.kind == kind)
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoExpected {
    pub schema_version: u8,
    pub repo: String,
    pub branch: String,
    pub commit: String,
    pub source_kind: SourceKind,
    pub source_id: String,
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
    pub source_label: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub expected: SkillRepoExpected,
    pub update_available: bool,
    pub installable_count: u32,
    pub conflict_count: u32,
    pub skipped_count: u32,
    pub attempts: Vec<SkillRepoSourceAttempt>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_design_skill_count: Option<u32>,
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
    attempts: Vec<SkillRepoSourceAttempt>,
}

impl HotError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            attempts: Vec::new(),
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn with_attempts(mut self, attempts: &[SkillRepoSourceAttempt]) -> Self {
        self.attempts = attempts.to_vec();
        self
    }
}

impl From<HotError> for CommandError {
    fn from(value: HotError) -> Self {
        let detail = if value.attempts.is_empty() {
            value.detail
        } else {
            serde_json::to_string(&serde_json::json!({
                "attempts": value.attempts,
                "detail": value.detail,
            }))
            .ok()
        };
        Self {
            code: value.code.to_string(),
            message: value.message,
            detail,
        }
    }
}

fn invalid(message: impl Into<String>) -> HotError {
    HotError::new("SKILL_REPO_ARCHIVE_INVALID", message)
}

fn local_io_error(context: &str, error: impl std::fmt::Display) -> HotError {
    HotError::new(
        "SKILL_REPO_LOCAL_IO",
        "本地 Skill Hub 数据写入失败，请检查权限和磁盘空间后重试。",
    )
    .with_detail(format!("{context}: {error}"))
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

fn source_invalid(message: impl Into<String>) -> HotError {
    HotError::new("SKILL_REPO_SOURCE_INVALID", message)
}

fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn reserved_fixture_host(host: &str) -> bool {
    host == "example"
        || host.ends_with(".example")
        || host.ends_with(".invalid")
        || host.ends_with(".test")
}

fn validate_source_base(source: &SkillHubSourceEntry) -> Result<(), HotError> {
    let Some(base) = source.base.as_deref() else {
        return Err(source_invalid("静态 Skill Hub 来源缺少 base"));
    };
    if base.len() > 512 || !base.ends_with("/skill-hub/") {
        return Err(source_invalid("Skill Hub 来源路径边界无效"));
    }
    let url = Url::parse(base).map_err(|_| source_invalid("Skill Hub 来源 URL 无效"))?;
    let host = url
        .host_str()
        .ok_or_else(|| source_invalid("Skill Hub 来源缺少公网域名"))?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || host.contains("..")
        || host.parse::<std::net::IpAddr>().is_ok()
        || !host.contains('.')
    {
        return Err(source_invalid(
            "Skill Hub 来源必须是无凭据的 HTTPS 公网域名",
        ));
    }
    if reserved_fixture_host(host) {
        if source.kind != SourceKind::Domestic || !source.test_fixture {
            return Err(source_invalid("保留测试域名只能用于显式 domestic fixture"));
        }
    } else if source.test_fixture {
        return Err(source_invalid("测试 fixture 必须使用保留域名"));
    }
    Ok(())
}

fn validate_source_directory(directory: &SkillHubSourceDirectory) -> Result<(), HotError> {
    if directory.schema_version != 1
        || directory.channel != "skill-hub"
        || directory.sources.is_empty()
    {
        return Err(source_invalid("Skill Hub 来源目录 schema 无效"));
    }
    let mut ids = HashSet::new();
    let mut priorities = HashSet::new();
    let mut relay_count = 0usize;
    let mut github_count = 0usize;
    let mut static_bases = HashSet::new();

    for source in &directory.sources {
        if !valid_source_id(&source.id)
            || !ids.insert(source.id.clone())
            || source.priority == 0
            || !priorities.insert(source.priority)
        {
            return Err(source_invalid("Skill Hub 来源 ID 或 priority 重复/无效"));
        }
        match source.kind {
            SourceKind::Domestic => {
                let Some(suffix) = source.id.strip_prefix("cn-mirror-") else {
                    return Err(source_invalid("国内来源 ID 必须是 cn-mirror-N"));
                };
                if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
                    return Err(source_invalid("国内来源 ID 序号无效"));
                }
                validate_source_base(source)?;
                if source.enabled && source.test_fixture {
                    return Err(source_invalid("测试 fixture 不能启用为生产来源"));
                }
                if let Some(base) = &source.base {
                    if !static_bases.insert(base.clone()) {
                        return Err(source_invalid("Skill Hub 静态来源 base 重复"));
                    }
                }
            }
            SourceKind::Relay => {
                relay_count += 1;
                if source.id != "ph-relay"
                    || !source.enabled
                    || source.test_fixture
                    || source.base.as_deref() != Some(RELAY_BASE)
                {
                    return Err(source_invalid("菲律宾 Relay 来源目录身份无效"));
                }
                validate_source_base(source)?;
                if let Some(base) = &source.base {
                    if !static_bases.insert(base.clone()) {
                        return Err(source_invalid("Skill Hub 静态来源 base 重复"));
                    }
                }
            }
            SourceKind::Github => {
                github_count += 1;
                if source.id != "github"
                    || !source.enabled
                    || source.test_fixture
                    || source.base.is_some()
                {
                    return Err(source_invalid("GitHub 来源目录身份无效"));
                }
            }
        }
    }

    if relay_count != 1 || github_count != 1 {
        return Err(source_invalid(
            "Skill Hub 来源目录必须包含一个合法启用的 Relay 和一个合法启用的 GitHub",
        ));
    }
    Ok(())
}

fn parse_source_directory(bytes: &[u8]) -> Result<SkillHubSourceDirectory, HotError> {
    let directory: SkillHubSourceDirectory = serde_json::from_slice(bytes)
        .map_err(|_| source_invalid("Skill Hub 来源目录 JSON 无效"))?;
    validate_source_directory(&directory)?;
    Ok(directory)
}

fn source_resource_candidates(app: &AppHandle) -> Vec<(String, PathBuf)> {
    let mut candidates = Vec::new();
    let mut add = |label: &str, path: PathBuf| {
        if !candidates.iter().any(|(_, existing)| existing == &path) {
            candidates.push((label.to_string(), path));
        }
    };

    // The USB build is a portable layout. Prefer the resources directory next
    // to the actual running manager so a stale Cargo/Tauri resource directory
    // cannot win over the release files the user launched.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            add(
                "portable-resources",
                parent.join("resources").join(SOURCE_RESOURCE),
            );
            add("portable-sibling", parent.join(SOURCE_RESOURCE));
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        add("tauri-resource-dir", resource_dir.join(SOURCE_RESOURCE));
    }
    candidates
}

fn source_resource_error(
    message: impl Into<String>,
    stage: &str,
    label: &str,
    path: Option<&Path>,
) -> HotError {
    let path_text = path
        .map(|value| value.display().to_string())
        .unwrap_or_else(|| "<embedded>".into());
    source_invalid(message).with_detail(format!(
        "stage={stage}; candidate={label}; path={path_text}"
    ))
}

fn source_directory(app: &AppHandle) -> Result<SkillHubSourceDirectory, HotError> {
    let candidates = source_resource_candidates(app);
    for (label, path) in &candidates {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(source_resource_error(
                    "读取 Skill Hub 来源目录属性失败",
                    "source-resource-stat",
                    label,
                    Some(path),
                )
                .with_detail(format!(
                    "stage=source-resource-stat; candidate={label}; path={}; error={error}",
                    path.display()
                )))
            }
        };
        if metadata_is_reparse(&metadata) {
            return Err(source_resource_error(
                "Skill Hub 来源目录不能是符号链接或重解析点",
                "source-resource-reparse",
                label,
                Some(path),
            ));
        }
        if !metadata.is_file() {
            return Err(source_resource_error(
                "Skill Hub 来源目录不是普通文件",
                "source-resource-type",
                label,
                Some(path),
            ));
        }
        let bytes = safe_read_file(path, "读取 Skill Hub 来源目录失败").map_err(|error| {
            source_resource_error(
                "读取 Skill Hub 来源目录失败",
                "source-resource-read",
                label,
                Some(path),
            )
            .with_detail(format!(
                "stage=source-resource-read; candidate={label}; path={}; error={}",
                path.display(),
                error.message
            ))
        })?;
        log::info!(
            "skill_hub_source_resource_loaded candidate={} bytes={}",
            label,
            bytes.len()
        );
        return parse_source_directory(&bytes).map_err(|error| {
            let reason = error.message.clone();
            log::warn!(
                "skill_hub_source_resource_invalid candidate={} reason={}",
                label,
                reason
            );
            error.with_detail(format!(
                "stage=source-resource-validate; candidate={label}; path={}; reason={reason}",
                path.display()
            ))
        });
    }

    // The source directory is release-owned and compiled from the same file
    // validated by build.rs. A missing copied resource is therefore a package
    // staging defect, not a reason to make the whole Skill Hub unavailable.
    log::warn!(
        "skill_hub_source_resource_missing candidates={} fallback=embedded",
        candidates
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    parse_source_directory(EMBEDDED_SOURCE_DIRECTORY).map_err(|error| {
        let reason = error.message.clone();
        error.with_detail(format!(
            "stage=embedded-source-validate; candidate=embedded; reason={reason}"
        ))
    })
}

fn base_manifest_resource_candidates_for(
    executable: Option<&Path>,
    resource_dir: Option<&Path>,
) -> Vec<(String, PathBuf)> {
    let mut candidates = Vec::new();
    let mut add = |label: &str, path: PathBuf| {
        if !candidates.iter().any(|(_, existing)| existing == &path) {
            candidates.push((label.to_string(), path));
        }
    };

    // A USB release is launched directly from its root. Prefer its own
    // resources over a stale Tauri resource directory left by an installed
    // copy of the manager on the same machine.
    if let Some(parent) = executable.and_then(Path::parent) {
        add(
            "portable-resources",
            parent.join("resources").join(BASE_MANIFEST_RESOURCE),
        );
        add("portable-sibling", parent.join(BASE_MANIFEST_RESOURCE));
    }
    if let Some(resource_dir) = resource_dir {
        add(
            "tauri-resource-dir",
            resource_dir.join(BASE_MANIFEST_RESOURCE),
        );
    }
    candidates
}

fn base_manifest_resource_candidates(app: &AppHandle) -> Vec<(String, PathBuf)> {
    let executable = std::env::current_exe().ok();
    let resource_dir = app.path().resource_dir().ok();
    base_manifest_resource_candidates_for(executable.as_deref(), resource_dir.as_deref())
}

fn source_label(source: &SkillHubSourceEntry) -> String {
    match source.kind {
        SourceKind::Domestic => source
            .id
            .strip_prefix("cn-mirror-")
            .map(|suffix| format!("domestic-{suffix}"))
            .unwrap_or_else(|| "domestic".into()),
        SourceKind::Relay => "relay".into(),
        SourceKind::Github => "github".into(),
    }
}

fn record_attempt(
    attempts: &mut Vec<SkillRepoSourceAttempt>,
    source: &SkillHubSourceEntry,
    phase: SkillRepoAttemptPhase,
    result: SkillRepoAttemptResult,
    http_status: Option<u16>,
    started: Instant,
) {
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    log::info!(
        "skill_hub_attempt source_id={} kind={} phase={} result={} http_status={} elapsed_ms={elapsed_ms}",
        source.id,
        source.kind.as_str(),
        phase.as_str(),
        result.as_str(),
        http_status.map_or_else(|| "none".into(), |status| status.to_string())
    );
    attempts.push(SkillRepoSourceAttempt {
        source_id: source.id.clone(),
        source_kind: source.kind,
        phase,
        result,
        http_status,
        elapsed_ms,
    });
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
        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, GENERIC_WRITE, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FlushFileBuffers, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };
        let wide = wide_path(path);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                // FlushFileBuffers requires GENERIC_WRITE. Opening a
                // directory read-only makes normal NTFS directory flushes
                // fail with ERROR_ACCESS_DENIED (5).
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            return Err(local_io_error(
                "打开目录 flush 句柄失败",
                format!("win32_error={error}; path={}", path.display()),
            ));
        }
        let flushed = unsafe { FlushFileBuffers(handle) } != 0;
        let error = if flushed {
            0
        } else {
            unsafe { GetLastError() }
        };
        unsafe {
            CloseHandle(handle);
        }
        if !flushed {
            return Err(local_io_error(
                "FlushFileBuffers 失败",
                format!("win32_error={error}; path={}", path.display()),
            ));
        }
        ensure_safe_parent(path)?;
        Ok(())
    }
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| local_io_error("flush 目录失败", error))?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _source_guards = guard_parent_chain(source)?;
    #[cfg(not(windows))]
    guard_parent_chain(source)?;
    #[cfg(windows)]
    let _destination_guards = guard_parent_chain(destination)?;
    #[cfg(not(windows))]
    guard_parent_chain(destination)?;
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
    #[cfg(windows)]
    let _source_guards = guard_parent_chain(source)?;
    #[cfg(not(windows))]
    guard_parent_chain(source)?;
    #[cfg(windows)]
    let _destination_guards = guard_parent_chain(destination)?;
    #[cfg(not(windows))]
    guard_parent_chain(destination)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _source_guards = guard_parent_chain(source)?;
    #[cfg(not(windows))]
    guard_parent_chain(source)?;
    #[cfg(windows)]
    let _destination_guards = guard_parent_chain(destination)?;
    #[cfg(not(windows))]
    guard_parent_chain(destination)?;
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

fn base_resource_error(context: &str, error: impl std::fmt::Display) -> HotError {
    HotError::new(
        "SKILL_REPO_BASE_RESOURCE_INVALID",
        "安装器基础资源不可用，请重新复制完整安装包后重试。",
    )
    .with_detail(format!("{context}: {error}"))
}

fn parse_base_manifest_bytes(bytes: &[u8]) -> Result<BaseManifest, HotError> {
    let manifest: BaseManifest = serde_json::from_slice(bytes)
        .map_err(|error| base_resource_error("基础 Skill ID 资源格式无效", error))?;
    if manifest.schema_version != 1
        || manifest.source != "installer-pack/manifest/skill-audit.json"
        || manifest.source_sha256 != BASE_MANIFEST_SHA256
    {
        return Err(base_resource_error(
            "基础 Skill ID 资源身份无效",
            "schema/source/hash",
        ));
    }
    let mut ids = HashSet::new();
    for item in &manifest.skills {
        if !valid_id(&item.id) || !ids.insert(item.id.clone()) {
            return Err(base_resource_error(
                "基础 Skill ID 资源包含重复或非法 ID",
                &item.id,
            ));
        }
    }
    Ok(manifest)
}

fn base_manifest(app: &AppHandle) -> Result<BaseManifest, HotError> {
    let candidates = base_manifest_resource_candidates(app);
    for (label, path) in &candidates {
        let metadata = match metadata_optional(path) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => continue,
            Err(error) => {
                let reason = error
                    .detail
                    .clone()
                    .unwrap_or_else(|| error.message.clone());
                return Err(
                    base_resource_error("读取基础 Skill ID 资源属性失败", reason).with_detail(
                        format!(
                            "stage=base-resource-stat; candidate={label}; path={}; error={}",
                            path.display(),
                            error.message
                        ),
                    ),
                );
            }
        };
        if !metadata.is_file() {
            return Err(base_resource_error(
                "基础 Skill ID 资源不是普通文件",
                format!("candidate={label}; path={}", path.display()),
            ));
        }
        let bytes = safe_read_file(path, "读取基础 Skill ID 资源失败").map_err(|error| {
            base_resource_error(
                "读取基础 Skill ID 资源失败",
                format!(
                    "candidate={label}; path={}; error={}",
                    path.display(),
                    error.message
                ),
            )
        })?;
        log::info!(
            "skill_hub_base_manifest_loaded candidate={} path={} bytes={}",
            label,
            path.display(),
            bytes.len()
        );
        return parse_base_manifest_bytes(&bytes).map_err(|error| {
            let reason = error.message.clone();
            error.with_detail(format!(
                "stage=base-resource-validate; candidate={label}; path={}; reason={}",
                path.display(),
                reason
            ))
        });
    }

    log::warn!(
        "skill_hub_base_manifest_missing candidates={} fallback=embedded",
        candidates
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    parse_base_manifest_bytes(EMBEDDED_BASE_MANIFEST).map_err(|error| {
        let reason = error.message.clone();
        error.with_detail(format!(
            "stage=embedded-base-resource-validate; candidate=embedded; reason={}",
            reason
        ))
    })
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(path)?;
    #[cfg(not(windows))]
    guard_parent_chain(path)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(&path)?;
    #[cfg(not(windows))]
    guard_parent_chain(&path)?;
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
    #[cfg(windows)]
    let _parent_guards = guard_parent_chain(root)?;
    #[cfg(not(windows))]
    guard_parent_chain(root)?;
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
    let mut desc: Option<String> = None;
    let mut description_block = false;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if description_block {
            // The Skill Hub uses the standard YAML folded scalar form for
            // longer descriptions (`description: >-` followed by indented
            // lines).  Keep this parser deliberately small and deterministic:
            // only an indented continuation of the already-seen description
            // key is accepted; arbitrary YAML is still not evaluated.
            if line.is_empty() {
                continue;
            }
            if line.contains('\t') || !line.starts_with(' ') {
                description_block = false;
            } else {
                if let Some(value) = desc.as_mut() {
                    if !value.is_empty() {
                        value.push('\n');
                    }
                    value.push_str(line.trim());
                }
                continue;
            }
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
            name = Some(value.to_string());
        }
        if key == "description" {
            if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
                desc = Some(String::new());
                description_block = true;
            } else {
                desc = Some(value.trim().to_string());
            }
        }
    }
    if !closed
        || name.as_deref() != Some(id)
        || desc
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
    {
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

fn parse_archive(
    bytes: &[u8],
    expected_commit: Option<&str>,
) -> Result<Vec<CandidateSkill>, HotError> {
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
    if let Some(expected_commit) = expected_commit {
        if prefix[10..] != *expected_commit {
            return Err(invalid("GitHub ZIP 前缀 commit 与 metadata 不一致"));
        }
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

fn proxy_mode_label(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::System => "system",
        ProxyMode::Direct => "direct",
        ProxyMode::Custom => "custom",
    }
}

fn ensure_rustls_crypto_provider() -> Result<(), HotError> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }

    // reqwest 0.13's rustls-no-provider feature leaves provider selection to
    // the application.  Install the bundled ring provider before building a
    // client; otherwise Client::build() panics and the UI only sees its
    // watchdog timeout.  The second check handles a concurrent initializer
    // that won the install race.
    let _ = rustls::crypto::ring::default_provider().install_default();
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }

    log::error!("skill_hub_tls_provider_unavailable");
    Err(HotError::new(
        "SKILL_REPO_UNAVAILABLE",
        "Skill Hub 网络加密组件初始化失败，请重试",
    )
    .with_detail("stage=tls-provider; reason=unavailable"))
}

fn client(settings: &AppSettings) -> Result<reqwest::Client, HotError> {
    ensure_rustls_crypto_provider()?;
    let mut builder = reqwest::Client::builder();
    match settings.proxy_mode {
        // reqwest's system-proxy feature reads the Windows/macOS system
        // proxy for this mode.  This is the same default used by the Codex
        // payload updater.
        ProxyMode::System => {}
        ProxyMode::Direct => {
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            let proxy_url = validate_custom_proxy(&settings.custom_proxy_url).map_err(|error| {
                log::warn!("skill_hub_network_config_invalid proxy_mode=custom reason={error}");
                HotError::new(
                    "SKILL_REPO_UNAVAILABLE",
                    "Skill Hub 网络代理设置无效，请检查设置后重试",
                )
                .with_detail("stage=network-config; reason=invalid-custom-proxy")
            })?;
            let proxy = reqwest::Proxy::all(&proxy_url).map_err(|_| {
                log::warn!("skill_hub_network_config_invalid proxy_mode=custom reason=builder");
                HotError::new(
                    "SKILL_REPO_UNAVAILABLE",
                    "Skill Hub 网络代理设置无效，请检查设置后重试",
                )
                .with_detail("stage=network-config; reason=proxy-builder")
            })?;
            builder = builder.proxy(proxy);
        }
    }
    log::info!(
        "skill_hub_network_config proxy_mode={}",
        proxy_mode_label(settings.proxy_mode)
    );
    builder
        .user_agent("codex-app-manager-skill-hub/1")
        .redirect(reqwest::redirect::Policy::limited(3))
        .connect_timeout(NETWORK_CONNECT_TIMEOUT)
        // Keep the request itself bounded even if a transport path does not
        // surface its stall through the phase timeout. The caller also has a
        // resolve deadline, so this is a second independent safety net.
        .timeout(NETWORK_OPERATION_TIMEOUT)
        .build()
        .map_err(|_| HotError::new("SKILL_REPO_UNAVAILABLE", "Skill Hub 网络客户端初始化失败"))
}

fn network_timeout() -> HotError {
    HotError::new(
        "SKILL_REPO_TIMEOUT",
        "Skill Hub 网络阶段超时，请检查网络后重试",
    )
}

fn attempt_result_for_error(error: &HotError) -> SkillRepoAttemptResult {
    match error.code {
        "SKILL_REPO_TIMEOUT" => SkillRepoAttemptResult::Timeout,
        "SKILL_REPO_SOURCE_INVALID" | "SKILL_REPO_ARCHIVE_INVALID" => {
            SkillRepoAttemptResult::Invalid
        }
        "SKILL_REPO_SOURCE_UNAVAILABLE"
            if error
                .detail
                .as_deref()
                .and_then(|detail| detail.strip_prefix("http_status="))
                .and_then(|status| status.parse::<u16>().ok())
                .is_some() =>
        {
            SkillRepoAttemptResult::Http
        }
        _ => SkillRepoAttemptResult::Unavailable,
    }
}

fn phase_timeout(phase: SkillRepoAttemptPhase) -> Duration {
    match phase {
        SkillRepoAttemptPhase::Metadata | SkillRepoAttemptPhase::Verify => NETWORK_METADATA_TIMEOUT,
        SkillRepoAttemptPhase::Archive => NETWORK_ARCHIVE_TIMEOUT,
    }
}

fn retryable_get_error(error: &HotError, phase: SkillRepoAttemptPhase) -> bool {
    if error.code == "SKILL_REPO_TIMEOUT" {
        // Metadata and verification responses are small; an archive timeout
        // should immediately yield the remaining deadline to source fallback.
        return phase != SkillRepoAttemptPhase::Archive;
    }

    if error.code != "SKILL_REPO_SOURCE_UNAVAILABLE" {
        return false;
    }

    let Some(detail) = error.detail.as_deref() else {
        return false;
    };
    if detail.contains("reason=transport") {
        return true;
    }

    let Some(status) = detail
        .strip_prefix("http_status=")
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return false;
    };

    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

fn static_source_url(source: &SkillHubSourceEntry, file: &str) -> Result<String, HotError> {
    source
        .base
        .as_ref()
        .map(|base| format!("{base}{file}"))
        .ok_or_else(|| source_invalid("静态 Skill Hub 来源缺少 base"))
}

async fn get_bytes(
    client: &reqwest::Client,
    source: &SkillHubSourceEntry,
    url: &str,
    max: usize,
    phase: SkillRepoAttemptPhase,
    deadline: tokio::time::Instant,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<Vec<u8>, HotError> {
    for request_index in 0..2 {
        let started = Instant::now();
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            record_attempt(
                attempts,
                source,
                phase,
                SkillRepoAttemptResult::Timeout,
                None,
                started,
            );
            return Err(network_timeout());
        }
        let budget = remaining.min(phase_timeout(phase));
        let result = tokio::time::timeout(budget, async {
            let response = client.get(url).send().await.map_err(|error| {
                if error.is_timeout() {
                    HotError::new("SKILL_REPO_TIMEOUT", "Skill Hub 网络请求超时").with_detail(
                        format!(
                            "stage=network-request; source_id={}; reason=timeout",
                            source.id
                        ),
                    )
                } else if error.is_redirect() {
                    source_invalid("Skill Hub 来源重定向失败").with_detail(format!(
                        "stage=network-request; source_id={}; reason=redirect",
                        source.id
                    ))
                } else {
                    HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "Skill Hub 来源不可用")
                        .with_detail(format!(
                            "stage=network-request; source_id={}; reason=transport",
                            source.id
                        ))
                }
            })?;
            if source.kind != SourceKind::Github && response.url().as_str() != url {
                return Err(
                    source_invalid("静态 Skill Hub 来源禁止重定向").with_detail(format!(
                        "stage=network-response; source_id={}; reason=redirect",
                        source.id
                    )),
                );
            }
            if source.kind == SourceKind::Github
                && !matches!(
                    response.url().host_str(),
                    Some("api.github.com") | Some("github.com") | Some("codeload.github.com")
                )
            {
                return Err(source_invalid("GitHub 重定向目标不允许").with_detail(
                    "stage=network-response; source_id=github; reason=redirect-host",
                ));
            }
            let status = response.status();
            if !status.is_success() {
                return Err(HotError::new(
                    "SKILL_REPO_SOURCE_UNAVAILABLE",
                    "Skill Hub 来源返回 HTTP 错误",
                )
                .with_detail(format!("http_status={}", status.as_u16())));
            }
            if response
                .content_length()
                .is_some_and(|length| length > max as u64)
            {
                return Err(invalid("远端响应超限"));
            }
            let bytes = response.bytes().await.map_err(|error| {
                if error.is_timeout() {
                    HotError::new("SKILL_REPO_TIMEOUT", "Skill Hub 响应读取超时").with_detail(
                        format!(
                            "stage=network-response; source_id={}; reason=timeout",
                            source.id
                        ),
                    )
                } else {
                    HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "Skill Hub 响应读取失败")
                        .with_detail(format!(
                            "stage=network-response; source_id={}; reason=transport",
                            source.id
                        ))
                }
            })?;
            if bytes.len() > max {
                return Err(invalid("远端响应超限"));
            }
            Ok((bytes.to_vec(), Some(status.as_u16())))
        })
        .await;

        let error = match result {
            Ok(Ok((bytes, status))) => {
                record_attempt(
                    attempts,
                    source,
                    phase,
                    SkillRepoAttemptResult::Ok,
                    status,
                    started,
                );
                return Ok(bytes);
            }
            Ok(Err(error)) => {
                let result = attempt_result_for_error(&error);
                let http_status = error
                    .detail
                    .as_deref()
                    .and_then(|detail| detail.strip_prefix("http_status="))
                    .and_then(|status| status.parse::<u16>().ok());
                record_attempt(attempts, source, phase, result, http_status, started);
                error
            }
            Err(_) => {
                record_attempt(
                    attempts,
                    source,
                    phase,
                    SkillRepoAttemptResult::Timeout,
                    None,
                    started,
                );
                network_timeout()
            }
        };

        if request_index == 0
            && retryable_get_error(&error, phase)
            && deadline.saturating_duration_since(tokio::time::Instant::now())
                > NETWORK_RETRY_BACKOFF
        {
            tokio::time::sleep(NETWORK_RETRY_BACKOFF).await;
            continue;
        }
        return Err(error);
    }
    unreachable!("bounded GET loop always returns after at most two requests")
}

fn expected_from_meta(
    meta: &[u8],
    source: &SkillHubSourceEntry,
) -> Result<SkillRepoExpected, HotError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
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
    let m: Meta = serde_json::from_slice(meta).map_err(|_| {
        source_invalid("Skill Hub metadata JSON 无效").with_detail(format!(
            "stage=remote-metadata; source_id={}; reason=json",
            source.id
        ))
    })?;
    if m.schema_version != 1
        || m.repo != REPO
        || m.branch != BRANCH
        || m.archive != "archive.zip"
        || !commit_valid(&m.commit)
        || !digest_valid(&m.sha256)
        || m.size == 0
        || m.size > MAX_ARCHIVE as u64
    {
        return Err(
            source_invalid("Skill Hub metadata 身份无效").with_detail(format!(
                "stage=remote-metadata; source_id={}; reason=identity",
                source.id
            )),
        );
    }
    Ok(SkillRepoExpected {
        schema_version: 1,
        repo: m.repo,
        branch: m.branch,
        commit: m.commit.to_lowercase(),
        source_kind: source.kind,
        source_id: source.id.clone(),
        archive_size: m.size,
        archive_sha256: m.sha256.to_lowercase(),
    })
}

async fn source_metadata(
    client: &reqwest::Client,
    source: &SkillHubSourceEntry,
    phase: SkillRepoAttemptPhase,
    deadline: tokio::time::Instant,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<SkillRepoExpected, HotError> {
    let started = Instant::now();
    let url = static_source_url(source, "latest.json")?;
    let bytes = get_bytes(client, source, &url, 64 * 1024, phase, deadline, attempts).await?;
    match expected_from_meta(&bytes, source) {
        Ok(expected) => Ok(expected),
        Err(error) => {
            record_attempt(
                attempts,
                source,
                phase,
                SkillRepoAttemptResult::Invalid,
                None,
                started,
            );
            Err(error)
        }
    }
}

async fn github_commit(
    client: &reqwest::Client,
    source: &SkillHubSourceEntry,
    phase: SkillRepoAttemptPhase,
    deadline: tokio::time::Instant,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<String, HotError> {
    let started = Instant::now();
    let bytes = get_bytes(
        client,
        source,
        GITHUB_COMMITS,
        64 * 1024,
        phase,
        deadline,
        attempts,
    )
    .await?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        source_invalid("GitHub metadata JSON 无效")
            .with_detail("stage=github-commit; source_id=github; reason=json")
    })?;
    let sha = v
        .get("sha")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_lowercase();
    if !commit_valid(&sha) {
        record_attempt(
            attempts,
            source,
            phase,
            SkillRepoAttemptResult::Invalid,
            None,
            started,
        );
        return Err(source_invalid("GitHub commit 无效")
            .with_detail("stage=github-commit; source_id=github; reason=sha"));
    }
    Ok(sha)
}

#[derive(Debug, Clone)]
struct StaticCandidate {
    source: SkillHubSourceEntry,
    expected: SkillRepoExpected,
}

#[derive(Debug)]
struct MetadataProbe {
    source: SkillHubSourceEntry,
    expected: Result<SkillRepoExpected, HotError>,
    attempts: Vec<SkillRepoSourceAttempt>,
}

#[derive(Debug)]
struct ResolvedSkillRepo {
    expected: SkillRepoExpected,
    archive: Vec<u8>,
    attempts: Vec<SkillRepoSourceAttempt>,
}

#[derive(Debug)]
struct DownloadedSkillRepo {
    expected: SkillRepoExpected,
    archive: Vec<u8>,
    attempts: Vec<SkillRepoSourceAttempt>,
}

fn retain_consistent_domestic_candidates(
    candidates: Vec<StaticCandidate>,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Vec<StaticCandidate> {
    let Some(reference) = candidates.first().map(|candidate| {
        (
            candidate.expected.commit.clone(),
            candidate.expected.archive_size,
            candidate.expected.archive_sha256.clone(),
        )
    }) else {
        return Vec::new();
    };

    candidates
        .into_iter()
        .filter_map(|candidate| {
            let identity = (
                candidate.expected.commit.clone(),
                candidate.expected.archive_size,
                candidate.expected.archive_sha256.clone(),
            );
            if identity == reference {
                return Some(candidate);
            }

            record_attempt(
                attempts,
                &candidate.source,
                SkillRepoAttemptPhase::Verify,
                SkillRepoAttemptResult::Invalid,
                None,
                Instant::now(),
            );
            log::warn!(
                "skill_hub_event phase=resolve source_id={} result=metadata_mismatch",
                candidate.source.id
            );
            None
        })
        .collect()
}

async fn probe_domestic_source(
    client: reqwest::Client,
    source: SkillHubSourceEntry,
    deadline: tokio::time::Instant,
) -> MetadataProbe {
    let mut attempts = Vec::new();
    let expected = source_metadata(
        &client,
        &source,
        SkillRepoAttemptPhase::Metadata,
        deadline,
        &mut attempts,
    )
    .await;
    MetadataProbe {
        source,
        expected,
        attempts,
    }
}

fn verify_archive(
    source: &SkillHubSourceEntry,
    expected: &SkillRepoExpected,
    archive: &[u8],
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<(), HotError> {
    let started = Instant::now();
    if archive.len() as u64 != expected.archive_size || sha256(archive) != expected.archive_sha256 {
        record_attempt(
            attempts,
            source,
            SkillRepoAttemptPhase::Verify,
            SkillRepoAttemptResult::HashMismatch,
            None,
            started,
        );
        return Err(HotError::new(
            "SKILL_REPO_HASH_MISMATCH",
            "归档 size/SHA-256 不匹配",
        ));
    }
    record_attempt(
        attempts,
        source,
        SkillRepoAttemptPhase::Verify,
        SkillRepoAttemptResult::Ok,
        None,
        started,
    );
    Ok(())
}

async fn fetch_static_candidate(
    client: &reqwest::Client,
    source: &SkillHubSourceEntry,
    expected: &SkillRepoExpected,
    deadline: tokio::time::Instant,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<Vec<u8>, HotError> {
    let archive_url = static_source_url(source, "archive.zip")?;
    let archive = get_bytes(
        client,
        source,
        &archive_url,
        MAX_ARCHIVE,
        SkillRepoAttemptPhase::Archive,
        deadline,
        attempts,
    )
    .await?;
    verify_archive(source, expected, &archive, attempts)?;
    let confirmed = source_metadata(
        client,
        source,
        SkillRepoAttemptPhase::Verify,
        deadline,
        attempts,
    )
    .await?;
    if confirmed != *expected {
        record_attempt(
            attempts,
            source,
            SkillRepoAttemptPhase::Verify,
            SkillRepoAttemptResult::Invalid,
            None,
            Instant::now(),
        );
        return Err(HotError::new(
            "SKILL_REPO_STALE",
            "Skill Hub 来源 metadata 在下载期间变化",
        ));
    }
    Ok(archive)
}

async fn resolve_github(
    client: &reqwest::Client,
    source: &SkillHubSourceEntry,
    deadline: tokio::time::Instant,
    attempts: &mut Vec<SkillRepoSourceAttempt>,
) -> Result<(SkillRepoExpected, Vec<u8>), HotError> {
    let commit = github_commit(
        client,
        source,
        SkillRepoAttemptPhase::Metadata,
        deadline,
        attempts,
    )
    .await?;
    let url = format!("{GITHUB_ARCHIVE}{commit}.zip");
    let archive = get_bytes(
        client,
        source,
        &url,
        MAX_ARCHIVE,
        SkillRepoAttemptPhase::Archive,
        deadline,
        attempts,
    )
    .await?;
    let expected = SkillRepoExpected {
        schema_version: 1,
        repo: REPO.into(),
        branch: BRANCH.into(),
        commit: commit.clone(),
        source_kind: SourceKind::Github,
        source_id: source.id.clone(),
        archive_size: archive.len() as u64,
        archive_sha256: sha256(&archive),
    };
    verify_archive(source, &expected, &archive, attempts)?;
    let after = github_commit(
        client,
        source,
        SkillRepoAttemptPhase::Verify,
        deadline,
        attempts,
    )
    .await?;
    if after != commit {
        record_attempt(
            attempts,
            source,
            SkillRepoAttemptPhase::Verify,
            SkillRepoAttemptResult::Invalid,
            None,
            Instant::now(),
        );
        return Err(HotError::new(
            "SKILL_REPO_STALE",
            "GitHub commit 在下载期间变化",
        ));
    }
    Ok((expected, archive))
}

async fn resolve_latest(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
    deadline: tokio::time::Instant,
) -> Result<ResolvedSkillRepo, HotError> {
    let mut source_order = directory
        .enabled_domestic()
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    source_order.extend(
        [SourceKind::Relay, SourceKind::Github]
            .into_iter()
            .filter_map(|kind| directory.enabled_source(kind).map(|source| source.id)),
    );
    log::info!(
        "skill_hub_event phase=resolve source_order={} retry=0",
        source_order.join(",")
    );

    let mut attempts = Vec::new();
    let mut probes = JoinSet::new();
    for source in directory.enabled_domestic() {
        let client = client.clone();
        probes.spawn(probe_domestic_source(client, source, deadline));
    }
    let mut candidates = Vec::new();
    while let Some(result) = probes.join_next().await {
        if let Ok(probe) = result {
            attempts.extend(probe.attempts);
            if let Ok(expected) = probe.expected {
                candidates.push(StaticCandidate {
                    source: probe.source,
                    expected,
                });
            }
        }
    }
    candidates.sort_by_key(|candidate| (candidate.source.priority, candidate.source.id.clone()));

    candidates = retain_consistent_domestic_candidates(candidates, &mut attempts);

    for candidate in candidates {
        match fetch_static_candidate(
            client,
            &candidate.source,
            &candidate.expected,
            deadline,
            &mut attempts,
        )
        .await
        {
            Ok(archive) => {
                log::info!(
                    "skill_hub_selected source_id={} kind={} commit={}",
                    candidate.source.id,
                    candidate.source.kind.as_str(),
                    candidate.expected.commit
                );
                return Ok(ResolvedSkillRepo {
                    expected: candidate.expected,
                    archive,
                    attempts,
                });
            }
            Err(error) => log::warn!(
                "skill_hub_event phase=resolve source_id={} result=source_failed code={}",
                candidate.source.id,
                error.code
            ),
        }
    }

    if let Some(source) = directory.enabled_source(SourceKind::Relay) {
        if let Ok(expected) = source_metadata(
            client,
            &source,
            SkillRepoAttemptPhase::Metadata,
            deadline,
            &mut attempts,
        )
        .await
        {
            if let Ok(archive) =
                fetch_static_candidate(client, &source, &expected, deadline, &mut attempts).await
            {
                log::info!(
                    "skill_hub_selected source_id={} kind={} commit={}",
                    source.id,
                    source.kind.as_str(),
                    expected.commit
                );
                return Ok(ResolvedSkillRepo {
                    expected,
                    archive,
                    attempts,
                });
            }
        }
    }

    if let Some(source) = directory.enabled_source(SourceKind::Github) {
        log::info!("skill_hub_event phase=resolve source_id=github result=fallback");
        if let Ok((expected, archive)) =
            resolve_github(client, &source, deadline, &mut attempts).await
        {
            log::info!(
                "skill_hub_selected source_id={} kind={} commit={}",
                source.id,
                source.kind.as_str(),
                expected.commit
            );
            return Ok(ResolvedSkillRepo {
                expected,
                archive,
                attempts,
            });
        }
    }

    Err(HotError::new(
        "SKILL_REPO_ALL_SOURCES_FAILED",
        "所有 Skill Hub 更新来源均不可用，未修改本地能力包",
    )
    .with_attempts(&attempts))
}

async fn resolve_latest_with_deadline(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
) -> Result<ResolvedSkillRepo, HotError> {
    let deadline = tokio::time::Instant::now() + NETWORK_OPERATION_TIMEOUT;
    resolve_latest(client, directory, deadline).await
}

fn same_archive_identity(left: &SkillRepoExpected, right: &SkillRepoExpected) -> bool {
    left.schema_version == right.schema_version
        && left.repo == right.repo
        && left.branch == right.branch
        && left.commit == right.commit
        && left.archive_size == right.archive_size
        && left.archive_sha256 == right.archive_sha256
}

fn expected_for_fallback_source(
    expected: &SkillRepoExpected,
    source: &SkillHubSourceEntry,
) -> SkillRepoExpected {
    let mut rebased = expected.clone();
    rebased.source_kind = source.kind;
    rebased.source_id = source.id.clone();
    rebased
}

fn pinned_timeout_fallback_allowed(error: &HotError, deadline: tokio::time::Instant) -> bool {
    error.code == "SKILL_REPO_TIMEOUT" && tokio::time::Instant::now() < deadline
}

fn fallback_archive_matches_expected(
    expected: &SkillRepoExpected,
    actual_identity: &SkillRepoExpected,
    archive: &[u8],
) -> bool {
    same_archive_identity(expected, actual_identity)
        && archive.len() as u64 == expected.archive_size
        && sha256(archive) == expected.archive_sha256
}

async fn download_expected_from_source(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
    expected: &SkillRepoExpected,
    deadline: tokio::time::Instant,
) -> Result<DownloadedSkillRepo, HotError> {
    let mut attempts = Vec::new();
    let result = async {
        if !expected_identity_valid(expected) {
            return Err(invalid_expected_identity());
        }
        let source = directory
            .source(&expected.source_id)
            .filter(|source| source.enabled)
            .cloned()
            .ok_or_else(|| source_invalid("expected sourceId 不在当前来源目录"))?;
        if source.kind != expected.source_kind {
            return Err(source_invalid("expected sourceId/sourceKind 不一致"));
        }
        let archive = match source.kind {
            SourceKind::Domestic | SourceKind::Relay => {
                let before = source_metadata(
                    client,
                    &source,
                    SkillRepoAttemptPhase::Metadata,
                    deadline,
                    &mut attempts,
                )
                .await?;
                if before != *expected {
                    record_attempt(
                        &mut attempts,
                        &source,
                        SkillRepoAttemptPhase::Verify,
                        SkillRepoAttemptResult::Invalid,
                        None,
                        Instant::now(),
                    );
                    return Err(HotError::new(
                        "SKILL_REPO_STALE",
                        "确认的来源 metadata 已变化，请重新检查",
                    ));
                }
                let url = static_source_url(&source, "archive.zip")?;
                let archive = get_bytes(
                    client,
                    &source,
                    &url,
                    MAX_ARCHIVE,
                    SkillRepoAttemptPhase::Archive,
                    deadline,
                    &mut attempts,
                )
                .await?;
                verify_archive(&source, expected, &archive, &mut attempts)?;
                let after = source_metadata(
                    client,
                    &source,
                    SkillRepoAttemptPhase::Verify,
                    deadline,
                    &mut attempts,
                )
                .await?;
                if after != *expected {
                    record_attempt(
                        &mut attempts,
                        &source,
                        SkillRepoAttemptPhase::Verify,
                        SkillRepoAttemptResult::Invalid,
                        None,
                        Instant::now(),
                    );
                    return Err(HotError::new(
                        "SKILL_REPO_STALE",
                        "同源 metadata 在下载后变化，请重新检查",
                    ));
                }
                archive
            }
            SourceKind::Github => {
                let before = github_commit(
                    client,
                    &source,
                    SkillRepoAttemptPhase::Metadata,
                    deadline,
                    &mut attempts,
                )
                .await?;
                if before != expected.commit {
                    return Err(HotError::new(
                        "SKILL_REPO_STALE",
                        "确认的 GitHub commit 已变化，请重新检查",
                    ));
                }
                let url = format!("{GITHUB_ARCHIVE}{}.zip", expected.commit);
                let archive = get_bytes(
                    client,
                    &source,
                    &url,
                    MAX_ARCHIVE,
                    SkillRepoAttemptPhase::Archive,
                    deadline,
                    &mut attempts,
                )
                .await?;
                verify_archive(&source, expected, &archive, &mut attempts)?;
                let after = github_commit(
                    client,
                    &source,
                    SkillRepoAttemptPhase::Verify,
                    deadline,
                    &mut attempts,
                )
                .await?;
                if after != expected.commit {
                    return Err(HotError::new(
                        "SKILL_REPO_STALE",
                        "GitHub commit 在下载后变化，请重新检查",
                    ));
                }
                archive
            }
        };
        Ok(DownloadedSkillRepo {
            expected: expected.clone(),
            archive,
            attempts: attempts.clone(),
        })
    }
    .await;
    result.map_err(|error: HotError| error.with_attempts(&attempts))
}

async fn download_from_fallback_source(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
    expected: &SkillRepoExpected,
    source: &SkillHubSourceEntry,
    deadline: tokio::time::Instant,
) -> Result<DownloadedSkillRepo, HotError> {
    // Rebase only the source coordinates here. download_expected_from_source
    // performs the source's before/after metadata (or commit) checks and
    // verifies the archive against this exact pinned content identity. A
    // separate preflight would consume the shared deadline and duplicate the
    // same metadata request.
    let candidate = expected_for_fallback_source(expected, source);
    if !same_archive_identity(expected, &candidate) {
        return Err(HotError::new(
            "SKILL_REPO_STALE",
            "备用来源身份与已确认内容不一致",
        ));
    }

    match download_expected_from_source(client, directory, &candidate, deadline).await {
        Ok(downloaded) => {
            if !fallback_archive_matches_expected(
                expected,
                &downloaded.expected,
                &downloaded.archive,
            ) {
                return Err(HotError::new(
                    "SKILL_REPO_HASH_MISMATCH",
                    "备用来源下载内容与已确认的 size/SHA-256 不一致",
                )
                .with_attempts(&downloaded.attempts));
            }
            Ok(downloaded)
        }
        Err(error) => Err(error),
    }
}

async fn download_expected(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
    expected: &SkillRepoExpected,
    deadline: tokio::time::Instant,
) -> Result<DownloadedSkillRepo, HotError> {
    match download_expected_from_source(client, directory, expected, deadline).await {
        Ok(downloaded) => Ok(downloaded),
        Err(primary_error) if pinned_timeout_fallback_allowed(&primary_error, deadline) => {
            let mut attempts = primary_error.attempts.clone();
            let mut fallbacks = directory.enabled_domestic();
            fallbacks.extend(
                [SourceKind::Relay, SourceKind::Github]
                    .into_iter()
                    .filter_map(|kind| directory.enabled_source(kind)),
            );
            fallbacks.retain(|source| source.enabled && source.id != expected.source_id);
            fallbacks.sort_by_key(|source| (source.priority, source.id.clone()));
            for source in fallbacks {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                match download_from_fallback_source(client, directory, expected, &source, deadline)
                    .await
                {
                    Ok(mut downloaded) => {
                        attempts.append(&mut downloaded.attempts);
                        downloaded.attempts = attempts;
                        log::warn!(
                            "skill_hub_apply_fallback source_id={} kind={} commit={}",
                            source.id,
                            source.kind.as_str(),
                            expected.commit
                        );
                        return Ok(downloaded);
                    }
                    Err(error) => attempts.extend(error.attempts),
                }
            }
            Err(primary_error.with_attempts(&attempts))
        }
        Err(error) => Err(error),
    }
}

async fn download_expected_with_deadline(
    client: &reqwest::Client,
    directory: &SkillHubSourceDirectory,
    expected: &SkillRepoExpected,
) -> Result<DownloadedSkillRepo, HotError> {
    let deadline = tokio::time::Instant::now() + NETWORK_OPERATION_TIMEOUT;
    download_expected(client, directory, expected, deadline).await
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
    value.len() == 40
        && value == value.to_ascii_lowercase()
        && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.chars().all(|c| c.is_ascii_hexdigit())
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
        && source_identity_matches_kind(&expected.source_id, expected.source_kind)
        && expected.archive_size > 0
        && expected.archive_size <= MAX_ARCHIVE as u64
        && digest_valid(&expected.archive_sha256)
}

fn source_identity_matches_kind(source_id: &str, kind: SourceKind) -> bool {
    valid_source_id(source_id)
        && match kind {
            SourceKind::Domestic => source_id.strip_prefix("cn-mirror-").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|value| value.is_ascii_digit())
            }),
            SourceKind::Relay => source_id == "ph-relay",
            SourceKind::Github => source_id == "github",
        }
}

fn invalid_expected_identity() -> HotError {
    source_invalid("Skill Hub 更新确认身份无效或已过期").with_detail(
        "stage=expected-identity; expected 必须来自当前 check 结果，且 sourceId/sourceKind、仓库、分支、commit、size、SHA-256 必须完整一致",
    )
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
        return Err(invalid_expected_identity());
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
            product_design_skill_count: None,
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
            product_design_skill_count: None,
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

#[cfg_attr(not(windows), allow(unreachable_code, unused_variables))]
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
    let directory = source_directory(app).map_err(CommandError::from)?;
    let settings = AppSettings::load();
    let client = client(&settings).map_err(CommandError::from)?;
    log::info!(
        "skill_hub_check_start network_timeout_ms={} proxy_mode={}",
        NETWORK_OPERATION_TIMEOUT.as_millis(),
        proxy_mode_label(settings.proxy_mode)
    );
    // `resolve_latest_with_deadline` bounds every source phase. This outer
    // deadline covers an unexpected future that fails to honor one of those
    // phase limits, so the command can never leave the renderer awaiting it
    // indefinitely. The two-second grace is only for joining already-finished
    // source tasks and does not extend any individual network request.
    let resolved = tokio::time::timeout(
        NETWORK_OPERATION_TIMEOUT + Duration::from_secs(2),
        resolve_latest_with_deadline(&client, &directory),
    )
    .await
    .map_err(|_| {
        log::error!(
            "skill_hub_check_timeout stage=resolve timeout_ms={}",
            (NETWORK_OPERATION_TIMEOUT + Duration::from_secs(2)).as_millis()
        );
        CommandError::from(network_timeout().with_detail(format!(
            "stage=resolve-deadline; timeout_ms={}",
            (NETWORK_OPERATION_TIMEOUT + Duration::from_secs(2)).as_millis()
        )))
    })?
    .map_err(CommandError::from)?;
    let ResolvedSkillRepo {
        expected,
        archive,
        attempts,
    } = resolved;
    let staging = state_dir()
        .map_err(CommandError::from)?
        .join("staging")
        .join(Uuid::new_v4().to_string());
    if let Err(error) = safe_create_dir_all(&staging, "创建 check staging 失败") {
        // Directory creation can succeed before the durability check fails.
        // Best-effort cleanup prevents an empty UUID staging directory from
        // accumulating after a local I/O error; the original error remains
        // the user-visible failure.
        let _ = safe_remove_dir_all(&staging, "清理失败的 check staging 失败");
        return Err(error.into());
    }
    let result = (|| -> Result<SkillRepoUpdateInfo, CommandError> {
        safe_write_new_file(
            &staging.join("archive.zip"),
            &archive,
            "写入 check staging 失败",
        )
        .map_err(CommandError::from)?;
        let skills = parse_archive(&archive, Some(&expected.commit))
            .map_err(|error| CommandError::from(error.with_attempts(&attempts)))?;
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
            source_label: directory
                .source(&expected.source_id)
                .map(source_label)
                .unwrap_or_else(|| expected.source_id.clone()),
            archive_size: expected.archive_size,
            archive_sha256: expected.archive_sha256.clone(),
            expected: expected.clone(),
            update_available: current.as_deref() != Some(expected.commit.as_str()),
            installable_count: installable,
            conflict_count: conflicts,
            skipped_count: 0,
            attempts,
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

#[cfg_attr(not(windows), allow(unreachable_code, unused_variables))]
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
    let directory = source_directory(app).map_err(CommandError::from)?;
    let settings = AppSettings::load();
    let client = client(&settings).map_err(CommandError::from)?;
    log::info!(
        "skill_hub_apply_start network_timeout_ms={} proxy_mode={}",
        NETWORK_OPERATION_TIMEOUT.as_millis(),
        proxy_mode_label(settings.proxy_mode)
    );
    let downloaded = download_expected_with_deadline(&client, &directory, &expected)
        .await
        .map_err(CommandError::from)?;
    let attempts = downloaded.attempts;
    let skills = parse_archive(&downloaded.archive, Some(&expected.commit))
        .map_err(|error| CommandError::from(error.with_attempts(&attempts)))?;
    apply_archive(downloaded.expected, skills, app)
        .map_err(|error| CommandError::from(error.with_attempts(&attempts)))
}

#[cfg_attr(not(windows), allow(unreachable_code, unused_variables))]
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

    fn retry_fixture_server(
        statuses: Vec<u16>,
        body: Vec<u8>,
        redirect_path: Option<&str>,
    ) -> (
        String,
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        retry_fixture_server_with_startup_timeout(
            statuses,
            body,
            redirect_path,
            Duration::from_secs(10),
        )
    }

    fn retry_fixture_server_with_startup_timeout(
        statuses: Vec<u16>,
        body: Vec<u8>,
        redirect_path: Option<&str>,
        first_request_timeout: Duration,
    ) -> (
        String,
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        retry_fixture_server_with_timeouts(
            statuses,
            body,
            redirect_path,
            first_request_timeout,
            Duration::from_secs(5),
        )
    }

    fn retry_fixture_server_with_timeouts(
        statuses: Vec<u16>,
        body: Vec<u8>,
        redirect_path: Option<&str>,
        first_request_timeout: Duration,
        request_timeout: Duration,
    ) -> (
        String,
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind retry fixture");
        listener
            .set_nonblocking(true)
            .expect("set retry fixture nonblocking");
        let address = listener.local_addr().expect("retry fixture address");
        let redirect_url = format!(
            "http://{address}{}",
            redirect_path.unwrap_or("/latest.json")
        );
        let request_count = Arc::new(AtomicUsize::new(0));
        let server_request_count = Arc::clone(&request_count);
        let worker = std::thread::spawn(move || {
            let started_at = Instant::now();
            let mut next_status = 0usize;
            let mut last_request_at = None;
            while next_status < statuses.len() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("set retry fixture stream blocking");
                        last_request_at = Some(Instant::now());
                        server_request_count.fetch_add(1, Ordering::SeqCst);
                        let mut request = Vec::new();
                        let mut chunk = [0u8; 1024];
                        let request_deadline = Instant::now() + request_timeout;
                        loop {
                            let remaining =
                                request_deadline.saturating_duration_since(Instant::now());
                            if remaining.is_zero() {
                                break;
                            }
                            stream
                                .set_read_timeout(Some(remaining))
                                .expect("set retry fixture request deadline");
                            let Ok(read) = stream.read(&mut chunk) else {
                                break;
                            };
                            if read == 0 {
                                break;
                            }
                            request.extend_from_slice(&chunk[..read]);
                            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                        let status = statuses[next_status];
                        next_status += 1;
                        let reason = match status {
                            200 => "OK",
                            302 => "Found",
                            404 => "Not Found",
                            503 => "Service Unavailable",
                            _ => "Fixture Status",
                        };
                        let redirect = if status == 302 {
                            format!("Location: {redirect_url}\r\n")
                        } else {
                            String::new()
                        };
                        let response = format!(
                            "HTTP/1.1 {status} {reason}\r\n{redirect}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len(),
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                        if next_status == statuses.len() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        let idle_timeout_reached = match last_request_at {
                            Some(last) => last.elapsed() >= Duration::from_secs(2),
                            None => started_at.elapsed() >= first_request_timeout,
                        };
                        if idle_timeout_reached {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (
            format!("http://{address}/latest.json"),
            worker,
            request_count,
        )
    }

    #[test]
    fn retry_fixture_server_stops_when_no_first_request_arrives() {
        let (_, worker, request_count) =
            retry_fixture_server_with_startup_timeout(vec![200], Vec::new(), None, Duration::ZERO);

        worker
            .join()
            .expect("fixture server should stop without a request");
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn retry_fixture_server_bounds_incomplete_request_headers() {
        use std::io::Write;

        let (base, worker, request_count) = retry_fixture_server_with_timeouts(
            vec![200],
            Vec::new(),
            None,
            Duration::from_secs(1),
            Duration::from_millis(25),
        );
        let address = base
            .strip_prefix("http://")
            .and_then(|url| url.split('/').next())
            .expect("fixture server URL should have an address");
        let mut client = std::net::TcpStream::connect(address).expect("connect fixture server");
        client
            .write_all(b"GET /latest.json HTTP/1.1\r\n")
            .expect("send incomplete fixture request");
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = finished_tx.send(worker.join().is_ok());
        });

        assert!(
            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("fixture server should stop reading an incomplete header"),
            "fixture server thread should not panic"
        );
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    fn skill_hub_fixture_server(
        request_count: usize,
        metadata: Vec<u8>,
        archive: Vec<u8>,
        delayed_archive_path: String,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture server address");
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::with_capacity(request_count);
            for _ in 0..request_count {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let metadata = metadata.clone();
                let archive = archive.clone();
                let delayed_archive_path = delayed_archive_path.clone();
                requests.push(std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        let Ok(read) = stream.read(&mut chunk) else {
                            break;
                        };
                        if read == 0 {
                            break;
                        }
                        request.extend_from_slice(&chunk[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request_line = String::from_utf8_lossy(&request);
                    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                    if path == delayed_archive_path {
                        // Keep this far above the fixture client timeout so the
                        // primary source's archive reliably times out even under
                        // parallel `cargo test` load; the relay path answers
                        // immediately. 180ms sat too close to the old 60ms client
                        // timeout and made the relay-fallback test flaky.
                        std::thread::sleep(Duration::from_millis(900));
                    }
                    let (content_type, body) = if path.ends_with("latest.json") {
                        ("application/json", metadata.as_slice())
                    } else {
                        ("application/zip", archive.as_slice())
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                }));
            }
            for request in requests {
                let _ = request.join();
            }
        });
        (format!("http://{address}/"), worker)
    }

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

    fn source_entry(
        id: &str,
        kind: SourceKind,
        base: Option<&str>,
        priority: u32,
        enabled: bool,
        test_fixture: bool,
    ) -> SkillHubSourceEntry {
        SkillHubSourceEntry {
            id: id.into(),
            kind,
            base: base.map(str::to_string),
            priority,
            enabled,
            test_fixture,
        }
    }

    fn source_directory_fixture(enabled_domestic: bool) -> SkillHubSourceDirectory {
        SkillHubSourceDirectory {
            schema_version: 1,
            channel: "skill-hub".into(),
            sources: vec![
                source_entry(
                    "cn-mirror-1",
                    SourceKind::Domestic,
                    Some("https://cn-mirror-1.example/skill-hub/"),
                    10,
                    enabled_domestic,
                    true,
                ),
                source_entry(
                    "cn-mirror-2",
                    SourceKind::Domestic,
                    Some("https://cn-mirror-2.example/skill-hub/"),
                    20,
                    enabled_domestic,
                    true,
                ),
                source_entry(
                    "cn-mirror-3",
                    SourceKind::Domestic,
                    Some("https://cn-mirror-3.example/skill-hub/"),
                    30,
                    enabled_domestic,
                    true,
                ),
                source_entry(
                    "ph-relay",
                    SourceKind::Relay,
                    Some(RELAY_BASE),
                    40,
                    true,
                    false,
                ),
                source_entry("github", SourceKind::Github, None, 50, true, false),
            ],
        }
    }

    #[test]
    fn source_directory_accepts_relay_and_github_without_domestic_sources() {
        let directory =
            parse_source_directory(include_bytes!("../../resources/skill_hub_sources.json"))
                .expect("relay and GitHub source directory");
        assert_eq!(directory.sources.len(), 2);
        assert!(directory.enabled_domestic().is_empty());
        assert_eq!(
            directory.source("ph-relay").map(|source| source.kind),
            Some(SourceKind::Relay)
        );
        assert_eq!(
            directory.source("github").map(|source| source.kind),
            Some(SourceKind::Github)
        );
    }

    #[test]
    fn embedded_source_directory_is_the_same_validated_release_resource() {
        let directory = parse_source_directory(EMBEDDED_SOURCE_DIRECTORY)
            .expect("embedded source directory must satisfy the release contract");
        assert_eq!(directory.channel, "skill-hub");
        assert_eq!(
            directory.source("ph-relay").map(|source| source.kind),
            Some(SourceKind::Relay)
        );
        assert_eq!(
            directory.source("github").map(|source| source.kind),
            Some(SourceKind::Github)
        );
    }

    #[test]
    fn portable_base_manifest_candidates_precede_tauri_resource_directory() {
        let candidates = base_manifest_resource_candidates_for(
            Some(Path::new("usb-bundle/CodexAppManager.exe")),
            Some(Path::new("installed-manager/resources")),
        );
        assert_eq!(
            candidates[0],
            (
                "portable-resources".into(),
                PathBuf::from("usb-bundle/resources/skill_repo_base_manifest.json")
            )
        );
        assert_eq!(
            candidates[1],
            (
                "portable-sibling".into(),
                PathBuf::from("usb-bundle/skill_repo_base_manifest.json")
            )
        );
        assert_eq!(
            candidates[2],
            (
                "tauri-resource-dir".into(),
                PathBuf::from("installed-manager/resources/skill_repo_base_manifest.json")
            )
        );
    }

    #[test]
    fn embedded_base_manifest_is_a_valid_release_resource() {
        let manifest = parse_base_manifest_bytes(EMBEDDED_BASE_MANIFEST)
            .expect("embedded base manifest must satisfy the release contract");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.source_sha256, BASE_MANIFEST_SHA256);
        assert!(!manifest.skills.is_empty());
    }

    #[test]
    fn invalid_base_manifest_uses_a_distinct_resource_error() {
        let error = parse_base_manifest_bytes(b"{}").expect_err("invalid base manifest");
        assert_eq!(error.code, "SKILL_REPO_BASE_RESOURCE_INVALID");
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_flush_uses_a_writable_handle() {
        let path = std::env::temp_dir().join(format!(
            "codex-skill-hub-directory-flush-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("temporary directory");
        let result = sync_directory(&path);
        let _ = fs::remove_dir_all(&path);
        assert!(result.is_ok(), "directory flush failed: {result:?}");
    }

    #[test]
    fn expected_identity_requires_fixed_source_id_and_kind_pair() {
        let mut expected = SkillRepoExpected {
            schema_version: 1,
            repo: REPO.into(),
            branch: BRANCH.into(),
            commit: "a".repeat(40),
            source_kind: SourceKind::Relay,
            source_id: "ph-relay".into(),
            archive_size: 1,
            archive_sha256: "b".repeat(64),
        };
        assert!(expected_identity_valid(&expected));

        expected.source_id = "github".into();
        assert!(!expected_identity_valid(&expected));
        let error = invalid_expected_identity();
        assert_eq!(error.code, "SKILL_REPO_SOURCE_INVALID");
        assert!(error
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("expected-identity")));
    }

    #[test]
    fn source_directory_accepts_one_optional_domestic_source() {
        let directory = SkillHubSourceDirectory {
            schema_version: 1,
            channel: "skill-hub".into(),
            sources: vec![
                source_entry(
                    "cn-mirror-1",
                    SourceKind::Domestic,
                    Some("https://mirror-one.example-cn.com/skill-hub/"),
                    10,
                    true,
                    false,
                ),
                source_entry(
                    "ph-relay",
                    SourceKind::Relay,
                    Some(RELAY_BASE),
                    40,
                    true,
                    false,
                ),
                source_entry("github", SourceKind::Github, None, 50, true, false),
            ],
        };

        assert!(validate_source_directory(&directory).is_ok());
        assert_eq!(directory.enabled_domestic().len(), 1);
    }

    #[test]
    fn source_directory_accepts_two_real_domestic_sources_with_optional_third() {
        let mut directory = source_directory_fixture(true);
        directory.sources[0].base = Some("https://mirror-one.example-cn.com/skill-hub/".into());
        directory.sources[1].base = Some("https://mirror-two.example-cn.com/skill-hub/".into());
        directory.sources[0].test_fixture = false;
        directory.sources[1].test_fixture = false;
        directory.sources[0].enabled = true;
        directory.sources[1].enabled = true;
        directory.sources[2].enabled = false;
        assert!(validate_source_directory(&directory).is_ok());
    }

    #[test]
    fn source_directory_rejects_duplicate_or_unapproved_entries() {
        let mut directory = source_directory_fixture(true);
        directory.sources[1].id = directory.sources[0].id.clone();
        assert!(validate_source_directory(&directory).is_err());

        let mut directory = source_directory_fixture(false);
        directory.sources[0].test_fixture = false;
        assert!(validate_source_directory(&directory).is_err());

        let mut directory = source_directory_fixture(true);
        directory.sources[0].enabled = true;
        assert!(validate_source_directory(&directory).is_err());
    }

    #[test]
    fn source_directory_rejects_enabled_domestic_fixture() {
        let mut directory = source_directory_fixture(true);
        directory
            .sources
            .retain(|source| !matches!(source.id.as_str(), "cn-mirror-2" | "cn-mirror-3"));
        directory.sources[0].enabled = true;
        assert!(validate_source_directory(&directory).is_err());
    }

    #[test]
    fn mismatched_domestic_candidates_are_dropped_before_archive_fetch() {
        let directory = source_directory_fixture(true);
        let expected_for = |source_id: &str, commit: char, digest: char| {
            let source = directory.source(source_id).unwrap().clone();
            StaticCandidate {
                source,
                expected: SkillRepoExpected {
                    schema_version: 1,
                    repo: REPO.into(),
                    branch: BRANCH.into(),
                    commit: commit.to_string().repeat(40),
                    source_kind: SourceKind::Domestic,
                    source_id: source_id.into(),
                    archive_size: 1,
                    archive_sha256: digest.to_string().repeat(64),
                },
            }
        };
        let mut attempts = Vec::new();
        let retained = retain_consistent_domestic_candidates(
            vec![
                expected_for("cn-mirror-1", 'a', 'b'),
                expected_for("cn-mirror-2", 'c', 'd'),
                expected_for("cn-mirror-3", 'a', 'b'),
            ],
            &mut attempts,
        );

        assert_eq!(
            retained
                .iter()
                .map(|candidate| candidate.source.id.as_str())
                .collect::<Vec<_>>(),
            vec!["cn-mirror-1", "cn-mirror-3"]
        );
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].source_id, "cn-mirror-2");
        assert_eq!(attempts[0].result, SkillRepoAttemptResult::Invalid);
    }

    #[test]
    fn metadata_fixes_source_identity_without_accepting_download_url() {
        let directory = source_directory_fixture(true);
        let source = directory.source("cn-mirror-1").unwrap();
        let meta = format!(
            r#"{{"schemaVersion":1,"repo":"{REPO}","branch":"{BRANCH}","commit":"{}","archive":"archive.zip","size":1,"sha256":"{}"}}"#,
            "a".repeat(40),
            "b".repeat(64)
        );
        let expected = expected_from_meta(meta.as_bytes(), source).expect("metadata");
        assert_eq!(expected.source_id, "cn-mirror-1");
        assert_eq!(expected.source_kind, SourceKind::Domestic);
        assert!(expected_identity_valid(&expected));
        assert!(!serde_json::to_string(&expected)
            .unwrap()
            .contains("archive.zip"));

        let with_extra = format!(
            r#"{{"schemaVersion":1,"repo":"{REPO}","branch":"{BRANCH}","commit":"{}","archive":"archive.zip","size":1,"sha256":"{}","extra":true}}"#,
            "a".repeat(40),
            "b".repeat(64)
        );
        assert!(expected_from_meta(with_extra.as_bytes(), source).is_err());

        let invalid = expected_from_meta(b"{}", source).expect_err("invalid metadata");
        assert!(invalid
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("remote-metadata")));

        let uppercase_commit = format!(
            r#"{{"schemaVersion":1,"repo":"{REPO}","branch":"{BRANCH}","commit":"{}","archive":"archive.zip","size":1,"sha256":"{}"}}"#,
            "A".repeat(40),
            "b".repeat(64)
        );
        assert!(expected_from_meta(uppercase_commit.as_bytes(), source).is_err());
    }

    #[test]
    fn pinned_timeout_can_fallback_to_relay_only_for_the_exact_confirmed_archive() {
        let directory = source_directory_fixture(true);
        let primary = SkillRepoExpected {
            schema_version: 1,
            repo: REPO.into(),
            branch: BRANCH.into(),
            commit: "a".repeat(40),
            source_kind: SourceKind::Domestic,
            source_id: "cn-mirror-1".into(),
            archive_size: 4,
            archive_sha256: sha256(b"same"),
        };
        let relay = directory.source("ph-relay").unwrap();
        let relay_identity = expected_for_fallback_source(&primary, relay);
        let exact_archive = b"same";
        assert!(pinned_timeout_fallback_allowed(
            &network_timeout(),
            tokio::time::Instant::now() + Duration::from_secs(1),
        ));
        assert!(fallback_archive_matches_expected(
            &primary,
            &relay_identity,
            exact_archive
        ));
        assert_eq!(relay_identity.source_id, "ph-relay");
        assert_eq!(relay_identity.source_kind, SourceKind::Relay);

        let mut changed_commit = relay_identity.clone();
        changed_commit.commit = "b".repeat(40);
        let mut changed_digest = relay_identity.clone();
        changed_digest.archive_sha256 = "c".repeat(64);
        assert!(!fallback_archive_matches_expected(
            &primary,
            &changed_commit,
            exact_archive
        ));
        assert!(!fallback_archive_matches_expected(
            &primary,
            &changed_digest,
            exact_archive
        ));
        assert!(!fallback_archive_matches_expected(
            &primary,
            &relay_identity,
            b"diff"
        ));
    }

    #[test]
    fn pinned_timeout_fallback_refuses_permanent_errors_and_expired_deadlines() {
        let deadline = tokio::time::Instant::now() - Duration::from_millis(1);
        assert!(!pinned_timeout_fallback_allowed(
            &network_timeout(),
            deadline
        ));
        assert!(!pinned_timeout_fallback_allowed(
            &HotError::new("SKILL_REPO_SOURCE_INVALID", "identity mismatch"),
            tokio::time::Instant::now() + Duration::from_secs(1),
        ));
    }

    #[test]
    fn apply_download_timeout_falls_back_to_relay_for_the_same_archive() {
        let archive = b"fixture archive bytes".to_vec();
        let commit = "a".repeat(40);
        let metadata = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "repo": REPO,
            "branch": BRANCH,
            "commit": commit.clone(),
            "archive": "archive.zip",
            "size": archive.len(),
            "sha256": sha256(&archive),
        }))
        .expect("metadata fixture");
        let (base, server) = skill_hub_fixture_server(
            5,
            metadata,
            archive.clone(),
            "/cn-mirror-1/archive.zip".into(),
        );
        let mut directory = source_directory_fixture(true);
        for source in &mut directory.sources {
            source.enabled = source.id == "cn-mirror-1" || source.id == "ph-relay";
            source.base = Some(format!("{}{}/", base, source.id));
        }
        let expected = SkillRepoExpected {
            schema_version: 1,
            repo: REPO.into(),
            branch: BRANCH.into(),
            commit,
            source_kind: SourceKind::Domestic,
            source_id: "cn-mirror-1".into(),
            archive_size: archive.len() as u64,
            archive_sha256: sha256(&archive),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            // 300ms leaves a wide margin over the immediate relay responses
            // (observed peak ~74ms under full-suite parallel load) while staying
            // far below the 900ms delayed primary-archive fixture, so the timeout
            // that drives the relay fallback remains deterministic.
            .timeout(Duration::from_millis(300))
            .build()
            .expect("short-timeout fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let downloaded = runtime
            .block_on(download_expected(
                &client,
                &directory,
                &expected,
                tokio::time::Instant::now() + Duration::from_secs(5),
            ))
            .expect("matching relay should recover the pinned archive download");
        server.join().expect("fixture server thread");
        assert_eq!(downloaded.expected.source_id, "ph-relay");
        assert_eq!(downloaded.archive, archive);
        assert!(downloaded.attempts.iter().any(|attempt| {
            attempt.source_id == "cn-mirror-1" && attempt.result == SkillRepoAttemptResult::Timeout
        }));
        assert!(downloaded.attempts.iter().any(|attempt| {
            attempt.source_id == "ph-relay" && attempt.result == SkillRepoAttemptResult::Ok
        }));
        assert_eq!(
            downloaded
                .attempts
                .iter()
                .filter(|attempt| {
                    attempt.source_id == "cn-mirror-1"
                        && attempt.phase == SkillRepoAttemptPhase::Archive
                })
                .count(),
            1,
            "an archive timeout must yield directly to the existing fallback source"
        );
    }

    #[test]
    fn apply_download_does_not_fallback_after_deadline_or_accept_changed_identity() {
        let archive = b"fixture archive bytes";
        let mut expected = SkillRepoExpected {
            schema_version: 1,
            repo: REPO.into(),
            branch: BRANCH.into(),
            commit: "a".repeat(40),
            source_kind: SourceKind::Domestic,
            source_id: "cn-mirror-1".into(),
            archive_size: archive.len() as u64,
            archive_sha256: sha256(archive),
        };
        let directory = source_directory_fixture(true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let expired = runtime.block_on(download_expected(
            &client,
            &directory,
            &expected,
            tokio::time::Instant::now() - Duration::from_millis(1),
        ));
        assert!(
            expired.is_err(),
            "expired deadline must not produce bytes to apply"
        );

        let relay = directory.source("ph-relay").unwrap();
        let relay_identity = expected_for_fallback_source(&expected, relay);
        assert!(fallback_archive_matches_expected(
            &expected,
            &relay_identity,
            archive
        ));
        expected.archive_sha256 = "b".repeat(64);
        assert!(!fallback_archive_matches_expected(
            &expected,
            &relay_identity,
            archive
        ));
    }

    #[test]
    fn structured_attempts_are_redacted() {
        let source = source_directory_fixture(true)
            .source("cn-mirror-1")
            .unwrap()
            .clone();
        let mut attempts = Vec::new();
        record_attempt(
            &mut attempts,
            &source,
            SkillRepoAttemptPhase::Metadata,
            SkillRepoAttemptResult::Timeout,
            None,
            Instant::now(),
        );
        let json = serde_json::to_string(&attempts).unwrap();
        assert!(json.contains("cn-mirror-1"));
        assert!(json.contains("metadata"));
        assert!(!json.contains("https://"));
        assert!(!json.contains("archive.zip"));
    }

    #[test]
    fn http_source_errors_are_distinguished_from_transport_failures() {
        let http = HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "source returned an error")
            .with_detail("http_status=503");
        assert_eq!(
            attempt_result_for_error(&http),
            SkillRepoAttemptResult::Http
        );

        let transport = HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "source unavailable")
            .with_detail("dns lookup failed");
        assert_eq!(
            attempt_result_for_error(&transport),
            SkillRepoAttemptResult::Unavailable
        );
    }

    #[test]
    fn get_retries_transient_http_once_and_records_each_request() {
        use std::sync::atomic::Ordering;

        let (url, server, request_count) =
            retry_fixture_server(vec![503, 200], b"metadata-ok".to_vec(), None);
        let source = source_entry("cn-mirror-1", SourceKind::Domestic, None, 1, true, true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let mut attempts = Vec::new();
        let result = runtime.block_on(get_bytes(
            &client,
            &source,
            &url,
            64 * 1024,
            SkillRepoAttemptPhase::Metadata,
            tokio::time::Instant::now() + Duration::from_secs(5),
            &mut attempts,
        ));
        server.join().expect("fixture server thread");

        assert_eq!(
            result.expect("second request should recover"),
            b"metadata-ok"
        );
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].result, SkillRepoAttemptResult::Http);
        assert_eq!(attempts[0].http_status, Some(503));
        assert_eq!(attempts[1].result, SkillRepoAttemptResult::Ok);
        assert_eq!(attempts[1].http_status, Some(200));
    }

    #[test]
    fn get_does_not_retry_permanent_http_or_archive_timeout() {
        use std::sync::atomic::Ordering;

        let (url, server, request_count) =
            retry_fixture_server(vec![404], b"must-not-be-used".to_vec(), None);
        let source = source_entry("cn-mirror-1", SourceKind::Domestic, None, 1, true, true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let mut attempts = Vec::new();
        let result = runtime.block_on(get_bytes(
            &client,
            &source,
            &url,
            64 * 1024,
            SkillRepoAttemptPhase::Metadata,
            tokio::time::Instant::now() + Duration::from_secs(5),
            &mut attempts,
        ));
        server.join().expect("fixture server thread");

        assert!(result.is_err());
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].result, SkillRepoAttemptResult::Http);
        assert_eq!(attempts[0].http_status, Some(404));

        let timeout = network_timeout();
        assert!(retryable_get_error(
            &timeout,
            SkillRepoAttemptPhase::Metadata
        ));
        assert!(!retryable_get_error(
            &timeout,
            SkillRepoAttemptPhase::Archive
        ));
        for status in [408, 425, 429, 500, 503, 599] {
            let error = HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "HTTP error")
                .with_detail(format!("http_status={status}"));
            assert!(retryable_get_error(&error, SkillRepoAttemptPhase::Metadata));
        }
        for status in [400, 401, 403, 404] {
            let error = HotError::new("SKILL_REPO_SOURCE_UNAVAILABLE", "HTTP error")
                .with_detail(format!("http_status={status}"));
            assert!(!retryable_get_error(
                &error,
                SkillRepoAttemptPhase::Metadata
            ));
        }
    }

    #[test]
    fn get_does_not_retry_a_reqwest_redirect_failure() {
        use std::sync::atomic::Ordering;

        let (url, server, request_count) = retry_fixture_server(vec![302, 302], Vec::new(), None);
        let source = source_entry("cn-mirror-1", SourceKind::Domestic, None, 1, true, true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::limited(1))
            .timeout(Duration::from_secs(3))
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let mut attempts = Vec::new();
        let result = runtime.block_on(get_bytes(
            &client,
            &source,
            &url,
            64 * 1024,
            SkillRepoAttemptPhase::Metadata,
            tokio::time::Instant::now() + Duration::from_secs(5),
            &mut attempts,
        ));
        server.join().expect("fixture server thread");

        let error = result.expect_err("redirect loop must be rejected");
        assert_eq!(error.code, "SKILL_REPO_SOURCE_INVALID");
        assert!(error
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("reason=redirect")));
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            attempts.len(),
            1,
            "redirect failure must not trigger a second logical GET"
        );
        assert_eq!(attempts[0].result, SkillRepoAttemptResult::Invalid);
    }

    #[test]
    fn static_source_redirect_is_rejected_before_retryable_target_status() {
        use std::sync::atomic::Ordering;

        let (url, server, request_count) = retry_fixture_server(
            vec![302, 503],
            b"temporary target failure".to_vec(),
            Some("/elsewhere.json"),
        );
        let source = source_entry("cn-mirror-1", SourceKind::Domestic, None, 1, true, true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let mut attempts = Vec::new();
        let result = runtime.block_on(get_bytes(
            &client,
            &source,
            &url,
            64 * 1024,
            SkillRepoAttemptPhase::Metadata,
            tokio::time::Instant::now() + Duration::from_secs(5),
            &mut attempts,
        ));
        server.join().expect("fixture server thread");

        let error = result.expect_err("static source redirect must be rejected");
        assert_eq!(error.code, "SKILL_REPO_SOURCE_INVALID");
        assert!(error.detail.as_deref().is_some_and(|detail| {
            detail.contains("stage=network-response") && detail.contains("reason=redirect")
        }));
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            attempts.len(),
            1,
            "redirected 503 must not trigger another logical GET"
        );
        assert_eq!(attempts[0].result, SkillRepoAttemptResult::Invalid);
        assert_eq!(attempts[0].http_status, None);
    }

    #[test]
    fn one_logical_get_never_sends_more_than_one_retry() {
        use std::sync::atomic::Ordering;

        let (url, server, request_count) =
            retry_fixture_server(vec![503, 503], b"temporary".to_vec(), None);
        let source = source_entry("cn-mirror-1", SourceKind::Domestic, None, 1, true, true);
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("fixture client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let mut attempts = Vec::new();
        let result = runtime.block_on(get_bytes(
            &client,
            &source,
            &url,
            64 * 1024,
            SkillRepoAttemptPhase::Metadata,
            tokio::time::Instant::now() + Duration::from_secs(5),
            &mut attempts,
        ));
        server.join().expect("fixture server thread");

        assert!(result.is_err());
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert_eq!(attempts.len(), 2);
        assert!(attempts
            .iter()
            .all(|attempt| attempt.http_status == Some(503)));
    }

    #[test]
    fn skill_hub_client_uses_the_persisted_proxy_mode_without_leaking_url() {
        let mut settings = AppSettings {
            proxy_mode: ProxyMode::System,
            ..AppSettings::default()
        };
        assert!(client(&settings).is_ok());

        settings.proxy_mode = ProxyMode::Direct;
        assert!(client(&settings).is_ok());

        settings.proxy_mode = ProxyMode::Custom;
        settings.custom_proxy_url = "socks5h://127.0.0.1:7890".into();
        assert!(client(&settings).is_ok());

        settings.custom_proxy_url = "not a proxy".into();
        let error = client(&settings).expect_err("invalid custom proxy must be rejected");
        assert_eq!(error.code, "SKILL_REPO_UNAVAILABLE");
        assert_eq!(
            error.detail.as_deref(),
            Some("stage=network-config; reason=invalid-custom-proxy")
        );
        assert!(!error
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("not a proxy"));
    }

    #[test]
    #[ignore = "requires live Skill Hub relay connectivity"]
    fn live_skill_hub_relay_round_trip_uses_current_native_client() {
        let directory: SkillHubSourceDirectory =
            serde_json::from_slice(EMBEDDED_SOURCE_DIRECTORY).expect("embedded source directory");
        validate_source_directory(&directory).expect("embedded source directory is valid");
        let source = directory
            .source("ph-relay")
            .expect("embedded source directory contains the relay")
            .clone();
        let http = client(&AppSettings::default()).expect("native Skill Hub client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let (expected, archive, attempts) = runtime
            .block_on(async {
                let deadline = tokio::time::Instant::now() + NETWORK_OPERATION_TIMEOUT;
                let mut attempts = Vec::new();
                let expected = source_metadata(
                    &http,
                    &source,
                    SkillRepoAttemptPhase::Metadata,
                    deadline,
                    &mut attempts,
                )
                .await?;
                let archive =
                    fetch_static_candidate(&http, &source, &expected, deadline, &mut attempts)
                        .await?;
                Ok::<_, HotError>((expected, archive, attempts))
            })
            .expect("relay metadata, archive and revalidation");

        assert_eq!(expected.source_id, "ph-relay");
        assert_eq!(expected.source_kind, SourceKind::Relay);
        assert!(!expected.commit.is_empty());
        assert!(!parse_archive(&archive, Some(&expected.commit))
            .expect("relay archive is a valid Skill archive")
            .is_empty());
        assert!(attempts.iter().any(|attempt| {
            attempt.source_id == "ph-relay" && attempt.result == SkillRepoAttemptResult::Ok
        }));
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
        assert!(parse_archive(&collision, None).is_err());

        let valid = zip_fixture(&[(
            collision_skill.as_str(),
            b"---\nname: demo\ndescription: x\n---\n",
            false,
        )]);
        assert!(parse_archive(&valid, Some(&"f".repeat(40))).is_err());

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
        assert!(parse_archive(&binary, None).is_err());
    }

    #[test]
    fn frontmatter_is_minimal_and_exact() {
        let b = b"---\nname: demo\ndescription: x\n---\nbody";
        assert!(validate_frontmatter("demo", b).is_ok());
        assert!(validate_frontmatter("other", b).is_err());
        let multiline = b"---\nname: demo\ndescription: >-\n  first line\n  second line\nversion: 1.0.0\n---\nbody";
        assert!(validate_frontmatter("demo", multiline).is_ok());
        let empty_multiline = b"---\nname: demo\ndescription: >-\n---\nbody";
        assert!(validate_frontmatter("demo", empty_multiline).is_err());
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
            source_id: "github".into(),
            archive_size: 1,
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
