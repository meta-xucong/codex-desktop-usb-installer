//! Unified update-channel abstraction shared by every updatable artifact
//! family (manager, codex payload, and future ccswitch / skill-repo).
//!
//! Lifecycle contract (see docs/UPDATE-ARCHITECTURE.md):
//!
//! ```text
//! check ──► notify ──► user confirms ──► apply ──► report
//!    │                                       │
//!    └─ never applies by itself              └─ re-validates before any
//!                                              destructive action
//! ```
//!
//! Dispatch is **enum-based**, not trait-object-based: the channel set is a
//! small, closed enum, native `async fn` in traits is not object-safe, and
//! the project does not depend on `async-trait`. Adding a channel means one
//! new enum variant plus one new `match` arm pair.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::app::settings_store::AppSettings;
use crate::errors::AppError;

/// Which update channel an [`UpdateInfo`] belongs to. Serialized as kebab-case
/// (`"manager"`, `"codex"`, `"ccswitch"`, `"skill-repo"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelKind {
    /// The manager app itself (Tauri updater, minisign-signed).
    Manager,
    /// The codex desktop payload (OpenAI-signed MSIX) hosted on the USB drive.
    Codex,
    /// Future: productized CodexSwitch executable.
    Ccswitch,
    /// Future: skills repository snapshot on the USB drive.
    SkillRepo,
}

impl ChannelKind {
    /// Parse the channel field of an [`UpdateInfo`] returned by (or sent to)
    /// the frontend. Unknown names are rejected instead of guessed.
    pub fn from_name(raw: &str) -> Result<Self, AppError> {
        match raw.trim() {
            "manager" => Ok(Self::Manager),
            "codex" => Ok(Self::Codex),
            "ccswitch" => Ok(Self::Ccswitch),
            "skill-repo" => Ok(Self::SkillRepo),
            other => Err(AppError::Engine(format!("unknown update channel: {other}"))),
        }
    }

    pub fn as_name(self) -> &'static str {
        match self {
            Self::Manager => "manager",
            Self::Codex => "codex",
            Self::Ccswitch => "ccswitch",
            Self::SkillRepo => "skill-repo",
        }
    }
}

/// Everything a channel's `check`/`apply` needs. Built once per command call
/// by the command bridge; channels must not re-resolve USB paths or proxy
/// settings themselves. Networking reuses the synchronous curl-based
/// primitives from `codex_win_engine` (see `channels::codex`).
#[derive(Debug, Clone)]
pub struct UpdateContext {
    /// Authorized USB bundle root, from `usb_guard::authorized_bundle_identity`.
    pub bundle_root: PathBuf,
    /// Persisted user settings snapshot.
    pub settings: AppSettings,
}

/// One available update, produced by a channel's `check` and (after the user
/// clicks) round-tripped back into `updates_apply`.
///
/// `payload` carries **channel-specific artifact identity** (repo, release
/// id/tag, asset names, package moniker, expected versions). It is display +
/// round-trip data only: `apply` MUST re-resolve everything against the
/// current persisted settings and a fresh remote API response. URLs and repo
/// names supplied by the frontend are never trusted as download instructions
/// (docs/UPDATE-CHANNELS.md § "Apply-time trust rule").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub channel: ChannelKind,
    pub current_version: String,
    pub latest_version: String,
    #[serde(default)]
    pub release_notes: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    /// True only for channels whose apply replaces the running manager
    /// (restart required). Codex payload refresh does not require a restart;
    /// a subsequent *deployment* on this machine is a separate user action.
    pub requires_restart: bool,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Result of `updates_check_all`: per-channel outcome so a single channel
/// outage never hides the others.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelCheckOutcome {
    Available(UpdateInfo),
    UpToDate {
        channel: ChannelKind,
    },
    Unavailable {
        channel: ChannelKind,
        message: String,
    },
}

fn parse_numeric_version(version: &str) -> Option<Vec<u64>> {
    version
        .split('.')
        .map(|segment| segment.parse::<u64>().ok())
        .collect()
}

/// Return whether a version is safe to compare in the Codex update channel.
/// Opaque or partially parsed remote values must never become an update
/// candidate.
pub fn is_numeric_version(version: &str) -> bool {
    parse_numeric_version(version).is_some()
}

/// Compare dotted numeric versions segment by segment.
///
/// `Less` is reserved for two parseable versions where `latest` is newer.
/// Existing callers use that result as the only permission to update, so an
/// unparseable current or latest version must fail closed instead of being
/// treated as a newer release.  The `Greater` fallback is intentionally only
/// a compatibility sentinel for this ordering API; callers with an error
/// boundary should report the invalid source explicitly.
pub fn compare_versions(current: &str, latest: &str) -> std::cmp::Ordering {
    match (
        parse_numeric_version(current),
        parse_numeric_version(latest),
    ) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => std::cmp::Ordering::Greater,
    }
}

/// Shared helper: read the current codex version from the USB root's
/// `manifest.json` (`codexDesktopVersion` field). Existing USB layouts
/// produced by `tools/collect-payload.ps1` always carry this file;
/// `payload/version.txt` is an optional future cache and MUST NOT be the
/// only source of truth (docs/UPDATE-CHANNELS.md § "Version comparison").
pub fn read_usb_codex_desktop_version(usb_root: &Path) -> Result<String, AppError> {
    let manifest_path = usb_root.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|error| {
        AppError::Engine(format!(
            "读取 U 盘 manifest.json 失败 ({}): {error}",
            manifest_path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        AppError::Engine(format!("source_invalid: 解析 manifest.json 失败: {error}"))
    })?;
    let version = value
        .get("codexDesktopVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::Engine("source_invalid: manifest.json 缺少 codexDesktopVersion".to_string())
        })?;
    if parse_numeric_version(version).is_none() {
        return Err(AppError::Engine(format!(
            "source_invalid: manifest.json codexDesktopVersion 无法解析: {version}"
        )));
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Dependency-free per-test temp directory (no `tempfile` crate in this
    /// project; hard constraint C2). Unique per call via a process counter.
    fn unique_test_dir(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cam-uc-test-{}-{tag}-{id}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Removes the directory on drop so failing tests don't leak temp state.
    struct TempDirGuard(std::path::PathBuf);

    impl TempDirGuard {
        fn new(path: std::path::PathBuf) -> Self {
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn channel_kind_names_round_trip() {
        for kind in [
            ChannelKind::Manager,
            ChannelKind::Codex,
            ChannelKind::Ccswitch,
            ChannelKind::SkillRepo,
        ] {
            assert_eq!(ChannelKind::from_name(kind.as_name()).unwrap(), kind);
        }
        assert!(ChannelKind::from_name("bogus").is_err());
        assert!(ChannelKind::from_name("  codex  ").is_ok());
    }

    #[test]
    fn numeric_versions_compare_segmentwise() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("26.602.40724", "26.602.40724"), Equal);
        assert_eq!(compare_versions("26.602.40724", "26.610.51200"), Less);
        assert_eq!(compare_versions("26.610.51200", "26.602.40724"), Greater);
        assert_eq!(compare_versions("26.602.40724", "26.602.40724.1"), Less);
        // Any parse failure is fail-closed: only Less may authorize an
        // update, so invalid current/latest versions must never be newer.
        assert_eq!(compare_versions("bad", "26.602.40724"), Greater);
        assert_eq!(compare_versions("26.602.40724", "bad"), Greater);
        assert_eq!(compare_versions("bad", "bad"), Greater);
        assert_eq!(compare_versions("26..602", "26.603.1"), Greater);
    }

    #[test]
    fn usb_codex_desktop_version_reads_manifest() {
        let dir_path = unique_test_dir("uc-read");
        let dir = TempDirGuard::new(dir_path);
        let manifest = dir.path().join("manifest.json");
        std::fs::write(
            &manifest,
            r#"{ "codexDesktopVersion": "26.825.51511", "packageName": "OpenAI.Codex" }"#,
        )
        .unwrap();
        assert_eq!(
            read_usb_codex_desktop_version(dir.path()).unwrap(),
            "26.825.51511"
        );
    }

    #[test]
    fn usb_codex_desktop_version_rejects_missing_or_empty() {
        let dir_path = unique_test_dir("uc-reject");
        let dir = TempDirGuard::new(dir_path);
        // Missing file
        assert!(read_usb_codex_desktop_version(dir.path()).is_err());
        // Missing field
        std::fs::write(dir.path().join("manifest.json"), r#"{ "a": 1 }"#).unwrap();
        assert!(read_usb_codex_desktop_version(dir.path()).is_err());
        // Empty string field
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{ "codexDesktopVersion": "  " }"#,
        )
        .unwrap();
        assert!(read_usb_codex_desktop_version(dir.path()).is_err());
    }

    #[test]
    fn usb_codex_desktop_version_reports_unparseable_version_as_source_invalid() {
        let dir_path = unique_test_dir("uc-invalid-version");
        let dir = TempDirGuard::new(dir_path);
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{ "codexDesktopVersion": "26.bad.51511" }"#,
        )
        .unwrap();

        let error = read_usb_codex_desktop_version(dir.path()).unwrap_err();
        assert!(error.to_string().contains("source_invalid:"));
    }

    #[test]
    fn update_info_serializes_kebab_channel() {
        let info = UpdateInfo {
            channel: ChannelKind::Codex,
            current_version: "26.825.51511".into(),
            latest_version: "26.900.1".into(),
            release_notes: None,
            size_bytes: Some(802_277_986),
            requires_restart: false,
            payload: serde_json::json!({}),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"channel\":\"codex\""));
        assert!(json.contains("\"requiresRestart\":false"));
    }
}
