use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use crate::adapters::host;
use crate::app::config_health::ConfigHealth;
use crate::app::oplock::OperationManager;
use crate::app::provenance::ProvenanceStore;
use crate::app::settings_store::AppSettings as PersistedAppSettings;
use crate::app::shell::FrontendGate;
use crate::domain::manifest::MirrorEndpoints;
use crate::domain::settings::AppSettings;
use crate::domain::target::Target;

/// A verified Windows uninstall helper handoff that is waiting for this
/// manager process to exit. The opaque ID is the helper nonce; keeping the
/// report/message beside it lets a reloaded renderer recover the same action
/// without creating a second cleanup job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UninstallHandoff {
    pub id: String,
    pub report_path: String,
    pub message: String,
}

pub struct ManagerState {
    pub target: Target,
    pub settings: AppSettings,
    pub endpoints: MirrorEndpoints,
    /// Set once the user confirms quitting (or has the guard off) so the close /
    /// exit handlers stop intercepting and let the process go.
    pub force_quit: AtomicBool,
    /// Set only after the Windows USB uninstaller has handed a verified helper
    /// to the manager. The helper waits for this process to exit; retaining the
    /// nonce and result in process state lets a reloaded renderer recover the
    /// same handoff instead of relying on a timer that can make the result
    /// invisible. The enum also retains an exit-authorized state as a write
    /// barrier while the operating system is closing the process.
    pub(crate) uninstall_handoff: Mutex<UninstallHandoffState>,
    /// Windows release windows stay hidden until WebView2's native browser
    /// accelerators have been disabled. The single-instance focus path must not
    /// bypass that startup gate.
    #[cfg(target_os = "windows")]
    pub webview_safe_to_show: AtomicBool,
    /// Failure wins over readiness so startup recovery never begins after the
    /// release WebView safety gate has failed.
    #[cfg(target_os = "windows")]
    pub webview_gate_failed: AtomicBool,
    /// Prevent repeated native startup notices when multiple startup
    /// fallbacks observe the same stalled WebView.
    pub(crate) startup_notice_shown: AtomicBool,
    pub operations: OperationManager,
    pub config_health: Mutex<ConfigHealth>,
    pub frontend: FrontendGate,
    /// Codex UI theme orchestration (daemon handle + status).
    pub codex_theme: crate::app::codex_theme::ThemeService,
}

#[cfg(any(target_os = "windows", test))]
fn initial_webview_safe_to_show(is_dev: bool) -> bool {
    is_dev
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebviewStartupGate {
    Wait,
    Proceed,
    Abort,
}

#[cfg(any(target_os = "windows", test))]
fn webview_startup_gate(safe_to_show: bool, failed: bool) -> WebviewStartupGate {
    if failed {
        WebviewStartupGate::Abort
    } else if safe_to_show {
        WebviewStartupGate::Proceed
    } else {
        WebviewStartupGate::Wait
    }
}

impl ManagerState {
    pub fn new() -> Self {
        let target = Target::current();
        let mirror_base_url = "https://codexapp.agentsmirror.com".to_string();
        let (saved, settings_health) = PersistedAppSettings::load_with_health();
        let (_, provenance_health) = ProvenanceStore::load_with_health();
        let config_health = Mutex::new(
            ConfigHealth::from_parts(settings_health, provenance_health).with_live_backup_flags(),
        );
        let install_root = if saved.install_root.trim().is_empty() {
            host::default_install_root(&target)
        } else {
            saved.install_root
        };
        let settings = AppSettings::new(mirror_base_url.clone(), install_root);
        let endpoints = MirrorEndpoints::from_base_url(&mirror_base_url);
        let lock_path = crate::app::paths::data_dir()
            .map(|dir| dir.join("operation.lock"))
            .unwrap_or_else(|| std::env::temp_dir().join("codex-app-manager-operation.lock"));
        let operations = OperationManager::new(lock_path);

        Self {
            target,
            settings,
            endpoints,
            force_quit: AtomicBool::new(false),
            uninstall_handoff: Mutex::new(UninstallHandoffState::None),
            #[cfg(target_os = "windows")]
            webview_safe_to_show: AtomicBool::new(initial_webview_safe_to_show(cfg!(dev))),
            #[cfg(target_os = "windows")]
            webview_gate_failed: AtomicBool::new(false),
            startup_notice_shown: AtomicBool::new(false),
            operations,
            config_health,
            frontend: FrontendGate::default(),
            codex_theme: crate::app::codex_theme::ThemeService::default(),
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn webview_startup_gate(&self) -> WebviewStartupGate {
        webview_startup_gate(
            self.webview_safe_to_show
                .load(std::sync::atomic::Ordering::SeqCst),
            self.webview_gate_failed
                .load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    pub(crate) fn arm_uninstall_handoff(&self, handoff: UninstallHandoff) -> bool {
        if handoff.id.trim().is_empty() {
            return false;
        }
        let mut state = self
            .uninstall_handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !matches!(&*state, UninstallHandoffState::None) {
            return false;
        }
        *state = UninstallHandoffState::Pending(handoff);
        true
    }

    pub(crate) fn uninstall_handoff(&self) -> Option<UninstallHandoff> {
        let state = self
            .uninstall_handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match &*state {
            // Keep the handoff visible until the process is actually gone.
            // `app.exit(0)` is asynchronous on Windows; dropping the only
            // renderer-visible ID at authorization time strands a still-live
            // manager if the OS delays the exit event.
            UninstallHandoffState::Pending(handoff) | UninstallHandoffState::Exiting(handoff) => {
                Some(handoff.clone())
            }
            UninstallHandoffState::None => None,
        }
    }

    pub(crate) fn uninstall_handoff_matches(&self, id: &str) -> bool {
        let state = self
            .uninstall_handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        matches!(&*state, UninstallHandoffState::Pending(handoff) | UninstallHandoffState::Exiting(handoff) if handoff.id == id)
    }

    /// Atomically authorize the exact pending handoff for process exit. The
    /// `Exiting` state remains a write barrier even if the OS delays the actual
    /// process termination after `app.exit(0)` is requested.
    pub(crate) fn authorize_uninstall_handoff(&self, id: &str) -> bool {
        let mut state = self
            .uninstall_handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let matches = match &*state {
            UninstallHandoffState::Pending(handoff) if handoff.id == id => {
                *state = UninstallHandoffState::Exiting(handoff.clone());
                true
            }
            // A duplicate renderer invocation after the first exit request is
            // safe and should be allowed to re-issue the controlled exit.
            UninstallHandoffState::Exiting(handoff) if handoff.id == id => true,
            _ => false,
        };
        matches
    }

    pub(crate) fn uninstall_handoff_active(&self) -> bool {
        let state = self
            .uninstall_handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        !matches!(&*state, UninstallHandoffState::None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UninstallHandoffState {
    None,
    Pending(UninstallHandoff),
    Exiting(UninstallHandoff),
}

impl Default for ManagerState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{initial_webview_safe_to_show, webview_startup_gate, WebviewStartupGate};

    #[test]
    fn windows_release_starts_with_the_webview_show_gate_closed() {
        assert!(!initial_webview_safe_to_show(false));
        assert!(initial_webview_safe_to_show(true));
    }

    #[test]
    fn failure_dominates_the_windows_startup_gate_state() {
        assert_eq!(webview_startup_gate(false, false), WebviewStartupGate::Wait);
        assert_eq!(
            webview_startup_gate(true, false),
            WebviewStartupGate::Proceed
        );
        assert_eq!(webview_startup_gate(false, true), WebviewStartupGate::Abort);
        assert_eq!(webview_startup_gate(true, true), WebviewStartupGate::Abort);
    }

    #[test]
    fn uninstall_handoff_requires_the_current_id_and_is_one_shot() {
        let state = super::ManagerState::new();
        let handoff = super::UninstallHandoff {
            id: "current".into(),
            report_path: "report.json".into(),
            message: "waiting".into(),
        };
        assert!(!state.uninstall_handoff_active());
        assert!(state.arm_uninstall_handoff(handoff.clone()));
        assert_eq!(state.uninstall_handoff(), Some(handoff.clone()));
        assert!(!state.authorize_uninstall_handoff("old"));
        assert_eq!(state.uninstall_handoff(), Some(handoff.clone()));
        assert!(state.authorize_uninstall_handoff("current"));
        assert!(state.uninstall_handoff_active());
        assert_eq!(state.uninstall_handoff(), Some(handoff.clone()));
        assert!(state.authorize_uninstall_handoff("current"));
        assert!(!state.authorize_uninstall_handoff("old"));
    }

    #[test]
    fn uninstall_handoff_arm_does_not_replace_an_existing_generation() {
        let state = super::ManagerState::new();
        let first = super::UninstallHandoff {
            id: "first".into(),
            report_path: "one".into(),
            message: "one".into(),
        };
        let second = super::UninstallHandoff {
            id: "second".into(),
            report_path: "two".into(),
            message: "two".into(),
        };
        assert!(state.arm_uninstall_handoff(first.clone()));
        assert!(!state.arm_uninstall_handoff(second));
        assert_eq!(state.uninstall_handoff(), Some(first));
    }
}
