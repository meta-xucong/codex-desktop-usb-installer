//! Native Windows process discovery and shutdown for Codex installs.
//!
//! The pre-replacement close gate must not add a PowerShell dependency: policy
//! can block a later PowerShell launch even after an artifact was staged and
//! verified. Keep the close path in process and pin every target to its
//! executable path. Every process loaded from the managed install root is a
//! target, regardless of filename, while a separate ChatGPT product remains
//! out of scope because its executable lives outside that root.

use std::path::Path;

use crate::EngineError;

/// Result of finding and presenting a real application window for a process
/// whose image lives below one exact managed program root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsableWindowReport {
    pub pid: u32,
    pub width: i32,
    pub height: i32,
    pub was_visible: bool,
    /// Windows may reject `SetForegroundWindow` because of its foreground-lock
    /// policy.  The window is still restored and moved to the top of the normal
    /// z-order, so this flag is diagnostic rather than a success condition.
    pub foreground_accepted: bool,
}

#[cfg(any(windows, test))]
fn window_dimensions_are_usable(width: i32, height: i32) -> bool {
    // Chromium/Tao creates tiny bookkeeping HWNDs (the target report showed
    // 14x14).  They are not the product window and must never be resized or
    // treated as proof that the UI is ready.
    width >= 320 && height >= 200
}

#[cfg(any(windows, test))]
fn is_codex_startup_failure_dialog(class_name: &str, title: &str) -> bool {
    // Electron's `dialog.showMessageBox({type: "error"})` is a native
    // `#32770` dialog.  The Codex bootstrap uses the product name as its
    // title, so the title is a reliable signal even when the detail text is
    // rendered in a child STATIC control that EnumWindows cannot see.
    if !class_name.eq_ignore_ascii_case("#32770") {
        return false;
    }
    let title = title.trim();
    if title.eq_ignore_ascii_case("chatgpt") || title.eq_ignore_ascii_case("codex") {
        return true;
    }
    let normalized = title.to_lowercase();
    [
        "failed to start",
        "startup failed",
        "package identity",
        "no package identity",
        "程序包标识符",
        "没有程序包",
        "启动失败",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(any(windows, test))]
fn is_codex_auxiliary_window(class_name: &str, title: &str) -> bool {
    // Electron creates a title-less Chrome_WidgetWin_0 companion surface on
    // some portable launches.  It can be a full-size black renderer while
    // Chrome_WidgetWin_1 is the actual user-operable Codex window.  Keep this
    // predicate narrow: only the title-less auxiliary class is hidden by the
    // startup watcher, so a user-created browser window is untouched.
    class_name.eq_ignore_ascii_case("Chrome_WidgetWin_0") && title.trim().is_empty()
}

#[cfg(any(windows, test))]
fn is_user_operable_window(class_name: &str, title: &str) -> bool {
    // Never restore the auxiliary Chromium surface as the launch result. A
    // real Codex shell normally uses Chrome_WidgetWin_1; keep its title check
    // permissive because Chromium can publish the title asynchronously. Other
    // app classes (including CCSwitch's Tauri window) retain generic sizing.
    let _ = title;
    !class_name.eq_ignore_ascii_case("Chrome_WidgetWin_0")
}

#[cfg(any(windows, test))]
fn is_lifecycle_window_class(class_name: &str) -> bool {
    // The WinEvent hook is process-global.  Filter the overwhelmingly common
    // unrelated window classes before taking a managed-process snapshot; the
    // two classes below are the only surfaces this watcher can suppress.
    class_name.eq_ignore_ascii_case("ConsoleWindowClass")
        || class_name.eq_ignore_ascii_case("Chrome_WidgetWin_0")
}

fn normalize_windows_path_text(value: &str) -> String {
    let mut normalized = value.replace('/', "\\").to_lowercase();
    if let Some(rest) = normalized.strip_prefix(r"\\?\unc\") {
        normalized = format!(r"\\{rest}");
    } else if let Some(rest) = normalized.strip_prefix(r"\\?\") {
        normalized = rest.to_string();
    }
    normalized.trim_end_matches('\\').to_string()
}

fn normalized_windows_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_windows_path_text(&resolved.to_string_lossy())
}

/// Compare Windows paths after resolving what can be resolved, removing the
/// extended-path prefix, normalizing separators and folding case.
pub fn same_windows_path(left: &Path, right: &Path) -> bool {
    let left = normalized_windows_path(left);
    let right = normalized_windows_path(right);
    !left.is_empty() && left == right
}

#[cfg(any(windows, test))]
fn path_is_within_root(candidate: &Path, root: &Path) -> bool {
    let candidate = normalized_windows_path(candidate);
    let root = normalized_windows_path(root);
    if candidate.is_empty() || root.is_empty() {
        return false;
    }
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

#[cfg(windows)]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStringExt;
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
        ERROR_NO_MORE_FILES, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, RECT, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW, SetForegroundWindow, SetWindowPos,
        ShowWindow, TranslateMessage, EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW, HWND_TOP,
        MWMO_INPUTAVAILABLE, OBJID_WINDOW, PM_REMOVE, QS_ALLINPUT, SWP_NOSIZE, SWP_SHOWWINDOW,
        SW_HIDE, SW_RESTORE, WM_CLOSE,
    };

    use super::{
        is_codex_auxiliary_window, is_codex_startup_failure_dialog, is_lifecycle_window_class,
        is_user_operable_window, path_is_within_root, window_dimensions_are_usable, EngineError,
        UsableWindowReport,
    };

    const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;
    const FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(250);
    const MAX_PROCESS_PATH_UTF16: usize = 32_768;
    const CONSOLE_WATCH_POLL: Duration = Duration::from_millis(250);
    const CONSOLE_WATCH_STARTUP_LIMIT: Duration = Duration::from_secs(45);
    const CONSOLE_WATCH_LIFETIME: Duration = Duration::from_secs(60);
    const CONSOLE_WATCH_EXIT_GRACE: Duration = Duration::from_secs(3);
    const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
    static CONSOLE_WATCH_ROOTS: OnceLock<Mutex<HashMap<u32, PathBuf>>> = OnceLock::new();

    unsafe extern "system" fn on_window_lifecycle_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        object_id: i32,
        child_id: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if !matches!(event, EVENT_OBJECT_CREATE | EVENT_OBJECT_SHOW)
            || hwnd.is_null()
            || object_id != OBJID_WINDOW
            || child_id != 0
        {
            return;
        }
        let mut class_name = [0_u16; 128];
        let class_length =
            unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
        if class_length <= 0 {
            return;
        }
        let class_name = String::from_utf16_lossy(&class_name[..class_length as usize]);
        if !is_lifecycle_window_class(&class_name) {
            return;
        }
        let thread_id = unsafe { GetCurrentThreadId() };
        let Some(registry) = CONSOLE_WATCH_ROOTS.get() else {
            return;
        };
        let root = registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&thread_id)
            .cloned();
        if let Some(root) = root {
            if let Ok(hidden) = hide_console_windows_for_root(&root) {
                if hidden > 0 {
                    log::debug!("managed Codex console window suppression event={event} hidden_candidates={hidden}");
                }
            }
            if let Ok(hidden) = hide_codex_auxiliary_windows_for_root(&root) {
                if hidden > 0 {
                    log::debug!(
                        "managed Codex auxiliary window suppression event={event} hidden_candidates={hidden}"
                    );
                }
            }
        }
    }

    /// Arm an out-of-context create/show hook before launching Codex. The
    /// callback applies the managed process-tree filter to both console hosts
    /// and the title-less Chromium auxiliary surface; it never hides an
    /// unrelated console or normal application window. A bounded polling pass
    /// is retained inside the watcher for missed WinEvents.
    pub fn start_console_window_watcher(root: &Path) -> Result<(), EngineError> {
        let root = root.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("codex-console-window-watch".into())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let roots = CONSOLE_WATCH_ROOTS.get_or_init(|| Mutex::new(HashMap::new()));
                roots.lock().unwrap_or_else(|e| e.into_inner()).insert(thread_id, root.clone());
                // Create this thread's message queue before installing the hook.
                let mut message = unsafe { std::mem::zeroed() };
                unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_SHOW,
                        std::ptr::null_mut(),
                        Some(on_window_lifecycle_event),
                        0,
                        0,
                        WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.is_null() {
                    roots.lock().unwrap_or_else(|e| e.into_inner()).remove(&thread_id);
                    let error = std::io::Error::last_os_error();
                    let _ = ready_tx.send(Err(format!("SetWinEventHook failed: {error}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                log::info!("managed Codex console event hook armed before launch root={}", root.display());
                let started = Instant::now();
                let mut root_seen = false;
                let mut absent_since = None;
                let mut last_fallback = Instant::now();
                loop {
                    // The hook is a startup guard, not a permanent global
                    // window monitor.  PE/ASAR fixes cover later launches;
                    // always release this thread and its WinEvent hook after
                    // a bounded post-launch lifetime even if Codex remains
                    // open for hours.
                    if started.elapsed() >= CONSOLE_WATCH_LIFETIME {
                        break;
                    }
                    unsafe {
                        MsgWaitForMultipleObjectsEx(
                            0,
                            std::ptr::null(),
                            CONSOLE_WATCH_POLL.as_millis() as u32,
                            QS_ALLINPUT,
                            MWMO_INPUTAVAILABLE,
                        );
                    }
                    while unsafe {
                        PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE)
                    } != 0
                    {
                        unsafe {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    if last_fallback.elapsed() >= CONSOLE_WATCH_POLL {
                        match hide_console_windows_for_root(&root) {
                            Ok(hidden) if hidden > 0 => log::debug!(
                                "managed Codex console fallback hidden_candidates={hidden}"
                            ),
                            Err(error) => log::warn!("managed Codex console fallback failed: {error}"),
                            _ => {}
                        }
                        match hide_codex_auxiliary_windows_for_root(&root) {
                            Ok(hidden) if hidden > 0 => log::debug!(
                                "managed Codex auxiliary window fallback hidden_candidates={hidden}"
                            ),
                            Err(error) => log::warn!(
                                "managed Codex auxiliary window fallback failed: {error}"
                            ),
                            _ => {}
                        }
                        last_fallback = Instant::now();
                    }
                    match processes_running_for_root(&root) {
                        Ok(true) => {
                            root_seen = true;
                            absent_since = None;
                        }
                        Ok(false) if root_seen => {
                            let absent = absent_since.get_or_insert_with(Instant::now);
                            if absent.elapsed() >= CONSOLE_WATCH_EXIT_GRACE {
                                break;
                            }
                        }
                        Ok(false) if started.elapsed() >= CONSOLE_WATCH_STARTUP_LIMIT => break,
                        Err(error) => log::debug!("managed Codex console process scan unavailable: {error}"),
                        _ => {}
                    }
                }
                let unhooked = unsafe { UnhookWinEvent(hook) } != 0;
                roots.lock().unwrap_or_else(|e| e.into_inner()).remove(&thread_id);
                log::info!("managed Codex console event hook stopped root_seen={root_seen} unhooked={unhooked}");
            })
            .map_err(|error| EngineError::Io(format!("start Codex console window watcher: {error}")))?;
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| EngineError::Io(format!("wait for console watcher: {error}")))?
            .map_err(|error| EngineError::Io(error))
    }

    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE) -> Option<Self> {
            (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle))
        }

        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: `OwnedHandle` is only constructed from a successful Win32
            // handle-returning call and owns that handle exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct TargetProcess {
        pid: u32,
        handle: OwnedHandle,
        can_terminate: bool,
    }

    impl TargetProcess {
        fn is_running(&self) -> bool {
            // SAFETY: the process handle remains owned for the lifetime of self.
            match unsafe { WaitForSingleObject(self.handle.raw(), 0) } {
                WAIT_OBJECT_0 => false,
                WAIT_TIMEOUT => true,
                WAIT_FAILED => {
                    log::warn!("wait for managed install process failed pid={}", self.pid);
                    true
                }
                status => {
                    log::warn!(
                        "wait for managed install process returned unexpected status pid={} status={status}",
                        self.pid
                    );
                    true
                }
            }
        }
    }

    fn last_error(context: &str) -> EngineError {
        EngineError::Install(format!("{context}: {}", std::io::Error::last_os_error()))
    }

    fn open_process_under_root(pid: u32, root: &Path) -> Option<TargetProcess> {
        let query_access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        // Query every snapshot entry without requesting termination rights on
        // unrelated system processes. Escalate access only after the image path
        // has matched the managed root.
        // Access-denied and already-exited processes are routine while walking
        // the system-wide snapshot, so an unavailable query handle is skipped.
        let query_handle = OwnedHandle::new(unsafe { OpenProcess(query_access, 0, pid) })?;

        let mut buffer = vec![0u16; MAX_PROCESS_PATH_UTF16];
        let mut length = buffer.len() as u32;
        // SAFETY: buffer is writable for `length` UTF-16 units and the process
        // handle has PROCESS_QUERY_LIMITED_INFORMATION access.
        if unsafe {
            QueryFullProcessImageNameW(query_handle.raw(), 0, buffer.as_mut_ptr(), &mut length)
        } == 0
        {
            return None;
        }
        let image_path = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
        if !path_is_within_root(&image_path, root) {
            return None;
        }

        // Process snapshots can visit the same managed process many times while
        // an install/uninstall is in flight. Keep this diagnostic available for
        // troubleshooting without flooding the user-facing log at INFO level.
        log::debug!(
            "managed install process discovered pid={pid} image={}",
            image_path.display()
        );

        let full_access = query_access | PROCESS_TERMINATE;
        // SAFETY: access flags and PID come from the process snapshot. If
        // policy denies termination, keep the query/synchronize handle so a
        // graceful close can still succeed.
        let terminate_handle = OwnedHandle::new(unsafe { OpenProcess(full_access, 0, pid) });
        let can_terminate = terminate_handle.is_some();
        let handle = terminate_handle.unwrap_or(query_handle);

        Some(TargetProcess {
            pid,
            handle,
            can_terminate,
        })
    }

    fn target_processes_under_root(root: &Path) -> Result<Vec<TargetProcess>, EngineError> {
        // SAFETY: TH32CS_SNAPPROCESS ignores the process-id argument.
        let snapshot = OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
            .ok_or_else(|| last_error("create process snapshot"))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        // SAFETY: `entry` has the documented size and remains writable.
        if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(Vec::new());
            }
            return Err(EngineError::Install(format!(
                "read first process snapshot entry: {err}"
            )));
        }

        let mut targets = Vec::new();
        let manager_pid = std::process::id();
        loop {
            // The Manager itself can be installed under a user-selected tree;
            // never include the process performing the replacement.
            let target = if entry.th32ProcessID != manager_pid {
                open_process_under_root(entry.th32ProcessID, root)
            } else {
                None
            };
            if let Some(target) = target {
                targets.push(target);
            }

            // SAFETY: same valid snapshot and entry buffer as Process32FirstW.
            if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(EngineError::Install(format!(
                    "read next process snapshot entry: {err}"
                )));
            }
        }
        Ok(targets)
    }

    /// Return the PIDs that may own a window for a managed install.
    ///
    /// The executable-path check above deliberately remains the ownership
    /// boundary: only a process whose image is below the exact USB root can
    /// establish the managed tree.  Chromium/WebView2 and `conhost.exe` are
    /// then allowed to create child windows outside that directory, so window
    /// operations must follow the point-in-time parent-PID chain from those
    /// exact roots.  This keeps an unrelated CCSwitch/Codex installation out
    /// of the hide/show scope while covering the current multi-process shell.
    fn managed_window_pids(root: &Path) -> Result<Vec<u32>, EngineError> {
        // Use one point-in-time process snapshot for both the exact image
        // ownership check and the parent map.  This function is called every
        // 100 ms during a cold start; taking a second system-wide snapshot on
        // each poll needlessly amplified removable-drive startup cost.
        // SAFETY: TH32CS_SNAPPROCESS ignores the process-id argument.
        let snapshot = OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
            .ok_or_else(|| last_error("create process snapshot for managed window scope"))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut entries = Vec::<(u32, u32)>::new();
        // A process can disappear while the snapshot is being enumerated. In
        // that case the roots already collected remain valid and the missing
        // descendant simply cannot own a later window.
        if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0 {
            loop {
                entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
                    break;
                }
            }
        }

        let manager_pid = std::process::id();
        let mut roots = HashSet::<u32>::new();
        for (pid, _) in &entries {
            if *pid == manager_pid {
                continue;
            }
            if let Some(target) = open_process_under_root(*pid, root) {
                if target.is_running() {
                    roots.insert(target.pid);
                }
            }
        }
        if roots.is_empty() {
            return Ok(Vec::new());
        }

        let parents = entries.into_iter().collect::<HashMap<_, _>>();
        let mut managed = roots.clone();
        for pid in parents.keys().copied() {
            let mut current = pid;
            // A broken/cyclic process tree must never make a launch probe
            // spin. Windows process ancestry is normally much shorter than
            // this bound; the cap is a defensive upper limit.
            for _ in 0..64 {
                if roots.contains(&current) {
                    managed.insert(pid);
                    break;
                }
                let Some(parent) = parents.get(&current).copied() else {
                    break;
                };
                if parent == 0 || parent == current {
                    break;
                }
                current = parent;
            }
        }
        let mut pids = managed.into_iter().collect::<Vec<_>>();
        pids.sort_unstable();
        Ok(pids)
    }

    #[derive(Clone, Copy)]
    struct WindowCandidate {
        hwnd: HWND,
        pid: u32,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
        visible: bool,
        score: i64,
    }

    struct WindowSearchContext {
        pids: *const u32,
        pid_count: usize,
        candidate: Option<WindowCandidate>,
        startup_failure_dialog: Option<String>,
    }

    struct ConsoleHideContext {
        pids: *const u32,
        pid_count: usize,
        hidden: usize,
    }

    struct WindowHideContext {
        pids: *const u32,
        pid_count: usize,
        hidden: usize,
    }

    struct AuxiliaryHideContext {
        pids: *const u32,
        pid_count: usize,
        hidden: usize,
    }

    struct WindowShowContext {
        pids: *const u32,
        pid_count: usize,
        shown: usize,
    }

    fn window_text(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return String::new();
        }
        // Error dialogs are short, but cap the allocation in case a hostile
        // process exposes an unbounded window title to the readiness probe.
        let length = (length as usize).min(4096);
        let mut buffer = vec![0_u16; length + 1];
        let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        if copied <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buffer[..copied as usize])
        }
    }

    unsafe extern "system" fn hide_console_window(hwnd: HWND, raw_context: LPARAM) -> i32 {
        if raw_context == 0 {
            return 1;
        }
        // SAFETY: the caller passes a live stack context to synchronous
        // EnumWindows; the PID slice remains valid for the callback duration.
        let context = unsafe { &mut *(raw_context as *mut ConsoleHideContext) };
        let pids = unsafe { std::slice::from_raw_parts(context.pids, context.pid_count) };
        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if !pids.contains(&pid) {
            return 1;
        }
        let mut class_name = [0u16; 64];
        let length =
            unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
        if length <= 0 {
            return 1;
        }
        let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
        if class_name.eq_ignore_ascii_case("ConsoleWindowClass")
            && unsafe { IsWindowVisible(hwnd) } != 0
        {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
            context.hidden += 1;
        }
        1
    }

    fn hide_console_windows_for_pids(pids: &[u32]) -> usize {
        if pids.is_empty() {
            return 0;
        }
        let mut context = ConsoleHideContext {
            pids: pids.as_ptr(),
            pid_count: pids.len(),
            hidden: 0,
        };
        // SAFETY: callback and context are synchronous and no pointer escapes.
        unsafe {
            EnumWindows(
                Some(hide_console_window),
                (&mut context as *mut ConsoleHideContext) as LPARAM,
            );
        }
        context.hidden
    }

    unsafe extern "system" fn hide_codex_auxiliary_window(hwnd: HWND, raw_context: LPARAM) -> i32 {
        if raw_context == 0 {
            return 1;
        }
        // SAFETY: the caller passes a live stack context to synchronous
        // EnumWindows; the PID slice remains valid for the callback duration.
        let context = unsafe { &mut *(raw_context as *mut AuxiliaryHideContext) };
        let pids = unsafe { std::slice::from_raw_parts(context.pids, context.pid_count) };
        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if !pids.contains(&pid) || unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let mut class_name = [0u16; 128];
        let class_length =
            unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
        if class_length <= 0 {
            return 1;
        }
        let class_name = String::from_utf16_lossy(&class_name[..class_length as usize]);
        let title = window_text(hwnd);
        if is_codex_auxiliary_window(&class_name, &title) {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
            context.hidden += 1;
        }
        1
    }

    fn hide_codex_auxiliary_windows_for_pids(pids: &[u32]) -> usize {
        if pids.is_empty() {
            return 0;
        }
        let mut context = AuxiliaryHideContext {
            pids: pids.as_ptr(),
            pid_count: pids.len(),
            hidden: 0,
        };
        // SAFETY: callback and context are synchronous and no pointer escapes.
        unsafe {
            EnumWindows(
                Some(hide_codex_auxiliary_window),
                (&mut context as *mut AuxiliaryHideContext) as LPARAM,
            );
        }
        context.hidden
    }

    /// Hide only Win32 console windows belonging to the managed process tree.
    ///
    /// Chromium can start a console-backed helper after the main window has
    /// already passed the stable-window check. A short post-launch sweep
    /// closes that race without hiding the real Codex application window.
    /// Console HWNDs are owned by `conhost.exe` on modern Windows, so include
    /// descendants of the managed processes by parent-PID ancestry instead of
    /// matching only the executable path.
    pub fn hide_console_windows_for_root(root: &Path) -> Result<usize, EngineError> {
        let pids = managed_window_pids(root)?;
        Ok(hide_console_windows_for_pids(&pids))
    }

    /// Hide the title-less Chromium companion surface observed on portable
    /// Codex launches.  This is scoped to the managed process tree and never
    /// touches a normal Chrome_WidgetWin_1 application window.
    pub fn hide_codex_auxiliary_windows_for_root(root: &Path) -> Result<usize, EngineError> {
        let pids = managed_window_pids(root)?;
        Ok(hide_codex_auxiliary_windows_for_pids(&pids))
    }

    unsafe extern "system" fn hide_application_window(hwnd: HWND, raw_context: LPARAM) -> i32 {
        if raw_context == 0 {
            return 1;
        }
        // SAFETY: the caller passes a live stack context to synchronous
        // EnumWindows; the PID slice remains valid for the callback duration.
        let context = unsafe { &mut *(raw_context as *mut WindowHideContext) };
        let pids = unsafe { std::slice::from_raw_parts(context.pids, context.pid_count) };
        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if !pids.contains(&pid) || unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        context.hidden += 1;
        1
    }

    /// Hide all visible top-level windows belonging to processes loaded from
    /// `root`. This is used after a successful CCSwitch bootstrap: its tray
    /// process remains alive while the configuration window stays out of the
    /// user's way.
    pub fn hide_windows_for_root(root: &Path) -> Result<usize, EngineError> {
        let pids = managed_window_pids(root)?;
        if pids.is_empty() {
            return Ok(0);
        }
        let mut context = WindowHideContext {
            pids: pids.as_ptr(),
            pid_count: pids.len(),
            hidden: 0,
        };
        // SAFETY: callback and context are synchronous and no pointer escapes.
        unsafe {
            EnumWindows(
                Some(hide_application_window),
                (&mut context as *mut WindowHideContext) as LPARAM,
            );
        }
        Ok(context.hidden)
    }

    unsafe extern "system" fn show_application_window(hwnd: HWND, raw_context: LPARAM) -> i32 {
        if raw_context == 0 {
            return 1;
        }
        let context = unsafe { &mut *(raw_context as *mut WindowShowContext) };
        let pids = unsafe { std::slice::from_raw_parts(context.pids, context.pid_count) };
        let mut pid = 0_u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if !pids.contains(&pid) {
            return 1;
        }
        let mut class_name = [0u16; 128];
        let class_length =
            unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
        if class_length > 0 {
            let class_name = String::from_utf16_lossy(&class_name[..class_length as usize]);
            let title = window_text(hwnd);
            if class_name.eq_ignore_ascii_case("ConsoleWindowClass")
                || !is_user_operable_window(&class_name, &title)
            {
                return 1;
            }
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return 1;
        }
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if !window_dimensions_are_usable(width, height) {
            return 1;
        }
        let (x, y) =
            visible_origin_on_nearest_monitor(hwnd, width, height).unwrap_or((rect.left, rect.top));
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetWindowPos(hwnd, HWND_TOP, x, y, 0, 0, SWP_NOSIZE | SWP_SHOWWINDOW);
            SetForegroundWindow(hwnd);
        }
        if unsafe { IsWindowVisible(hwnd) } != 0 {
            context.shown += 1;
        }
        1
    }

    /// Restore visible top-level windows belonging to the managed root. This
    /// is intentionally the inverse of `hide_windows_for_root` and is used by
    /// the explicit CCSwitch button after the one-time bootstrap hide.
    pub fn show_windows_for_root(root: &Path) -> Result<usize, EngineError> {
        let pids = managed_window_pids(root)?;
        if pids.is_empty() {
            return Ok(0);
        }
        let mut context = WindowShowContext {
            pids: pids.as_ptr(),
            pid_count: pids.len(),
            shown: 0,
        };
        unsafe {
            EnumWindows(
                Some(show_application_window),
                (&mut context as *mut WindowShowContext) as LPARAM,
            );
        }
        Ok(context.shown)
    }

    unsafe extern "system" fn find_usable_window(hwnd: HWND, raw_context: LPARAM) -> i32 {
        if raw_context == 0 {
            return 1;
        }
        // SAFETY: `wait_for_usable_window_for_root` passes a live stack context
        // to synchronous EnumWindows. The PID slice outlives that call.
        let context = unsafe { &mut *(raw_context as *mut WindowSearchContext) };
        let pids = unsafe { std::slice::from_raw_parts(context.pids, context.pid_count) };
        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if !pids.contains(&pid) {
            return 1;
        }
        let mut class_name = [0_u16; 128];
        let class_length =
            unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
        let class_name = if class_length > 0 {
            String::from_utf16_lossy(&class_name[..class_length as usize])
        } else {
            String::new()
        };
        let title = window_text(hwnd);
        // A sizeable console host can be mistaken for the GUI while Electron
        // is still starting. Never treat it as a usable Codex window.
        if class_name.eq_ignore_ascii_case("ConsoleWindowClass")
            || !is_user_operable_window(&class_name, &title)
        {
            return 1;
        }
        if is_codex_startup_failure_dialog(&class_name, &title) {
            if context.startup_failure_dialog.is_none() {
                context.startup_failure_dialog = Some(title);
            }
            return 1;
        }
        if unsafe { IsIconic(hwnd) } != 0 {
            // Minimized windows are often reported at -32000 with a tiny icon
            // rectangle. Restore first so the real normal dimensions can be
            // assessed instead of misclassifying a healthy product window.
            unsafe {
                ShowWindow(hwnd, SW_RESTORE);
            }
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return 1;
        }
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if !window_dimensions_are_usable(width, height) {
            return 1;
        }
        let visible = unsafe { IsWindowVisible(hwnd) } != 0;
        let area = i64::from(width).saturating_mul(i64::from(height));
        // Prefer an already-visible application window, then the largest
        // hidden one. This avoids selecting a sizeable hidden helper over the
        // real main window when both exist.
        let score = area.saturating_add(if visible { 1_i64 << 60 } else { 0 });
        if context
            .candidate
            .as_ref()
            .map_or(true, |candidate| score > candidate.score)
        {
            context.candidate = Some(WindowCandidate {
                hwnd,
                pid,
                left: rect.left,
                top: rect.top,
                width,
                height,
                visible,
                score,
            });
        }
        1
    }

    fn visible_origin_on_nearest_monitor(
        hwnd: HWND,
        width: i32,
        height: i32,
    ) -> Option<(i32, i32)> {
        // SAFETY: `hwnd` came from EnumWindows and MONITOR_DEFAULTTONEAREST
        // guarantees a monitor for an otherwise valid top-level window.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return None;
        }
        let work = info.rcWork;
        let max_x = work.right.saturating_sub(width).max(work.left);
        let max_y = work.bottom.saturating_sub(height).max(work.top);
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return None;
        }
        Some((
            rect.left.clamp(work.left, max_x),
            rect.top.clamp(work.top, max_y),
        ))
    }

    /// Wait for a sizeable top-level window owned by a process physically
    /// loaded from `root`, restore it and request foreground activation.
    ///
    /// Returning `None` is an explicit failed postcondition: a process may
    /// still be alive, but no user-operable UI appeared before the deadline.
    pub fn wait_for_usable_window_for_root(
        root: &Path,
        timeout: Duration,
    ) -> Result<Option<UsableWindowReport>, EngineError> {
        let deadline = Instant::now() + timeout;
        loop {
            let pids = managed_window_pids(root)?;
            if !pids.is_empty() {
                let hidden = hide_console_windows_for_pids(&pids);
                if hidden > 0 {
                    log::info!(
                        "hidden console window(s) created by managed Codex process tree count={hidden}"
                    );
                }
                let hidden_auxiliary = hide_codex_auxiliary_windows_for_pids(&pids);
                if hidden_auxiliary > 0 {
                    log::info!(
                        "hidden auxiliary Chromium window(s) created by managed Codex process tree count={hidden_auxiliary}"
                    );
                }
                let mut context = WindowSearchContext {
                    pids: pids.as_ptr(),
                    pid_count: pids.len(),
                    candidate: None,
                    startup_failure_dialog: None,
                };
                // SAFETY: callback and context remain valid for this synchronous
                // call; no pointer escapes EnumWindows.
                unsafe {
                    EnumWindows(
                        Some(find_usable_window),
                        (&mut context as *mut WindowSearchContext) as LPARAM,
                    );
                }
                if let Some(title) = context.startup_failure_dialog {
                    return Err(EngineError::Io(format!(
                        "Codex 启动失败：检测到错误对话框（标题={title}，可能是便携版缺少 MSIX 程序包标识符）"
                    )));
                }
                if let Some(candidate) = context.candidate {
                    // SW_RESTORE also makes a hidden/minimized explicit-launch
                    // window visible. HWND_TOP avoids a successfully-created
                    // configurator remaining completely behind the manager.
                    let (x, y) = visible_origin_on_nearest_monitor(
                        candidate.hwnd,
                        candidate.width,
                        candidate.height,
                    )
                    .unwrap_or((candidate.left, candidate.top));
                    unsafe {
                        ShowWindow(candidate.hwnd, SW_RESTORE);
                        SetWindowPos(
                            candidate.hwnd,
                            HWND_TOP,
                            x,
                            y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_SHOWWINDOW,
                        );
                    }
                    if unsafe { IsWindowVisible(candidate.hwnd) } == 0 {
                        // The HWND existed but did not become user-visible;
                        // keep polling instead of reporting a false success
                        // that leaves the caller staring at a black/hidden
                        // startup surface.
                        continue;
                    }
                    let foreground_accepted = unsafe { SetForegroundWindow(candidate.hwnd) } != 0;
                    return Ok(Some(UsableWindowReport {
                        pid: candidate.pid,
                        width: candidate.width,
                        height: candidate.height,
                        was_visible: candidate.visible,
                        foreground_accepted,
                    }));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn processes_running_for_root(root: &Path) -> Result<bool, EngineError> {
        Ok(target_processes_under_root(root)?
            .iter()
            .any(TargetProcess::is_running))
    }

    fn strict_process_open_failure<F>(
        pid: u32,
        snapshot_image_name: &std::ffi::OsStr,
        error_code: u32,
        fresh_snapshot_contains_pid: F,
    ) -> Result<bool, EngineError>
    where
        F: FnOnce(u32) -> Result<bool, EngineError>,
    {
        if pid == 0 || pid == 4 {
            // Kernel idle/System pseudo-processes have no queryable user image.
            return Ok(false);
        }
        if error_code == ERROR_INVALID_PARAMETER {
            // A PID may be skipped only if a second, fresh Toolhelp snapshot
            // proves it disappeared after the original process snapshot.
            // If it is still present, or the fresh snapshot is unreadable,
            // keep the scan fail-closed.
            return match fresh_snapshot_contains_pid(pid) {
                Ok(false) => Ok(false),
                Ok(true) => Err(EngineError::Install(format!(
                    "process {pid} is still present in a fresh snapshot after OpenProcess returned ERROR_INVALID_PARAMETER"
                ))),
                Err(error) => Err(EngineError::Install(format!(
                    "cannot confirm process {pid} exited after ERROR_INVALID_PARAMETER: {error}"
                ))),
            };
        }
        if matches!(
            snapshot_image_name,
            name if name == std::ffi::OsStr::new("Secure System")
                || name == std::ffi::OsStr::new("Registry")
        ) && error_code == ERROR_INVALID_HANDLE
        {
            // VBS secure-kernel processes are protected OS processes with no
            // ordinary query handle. Require an exact snapshot name and the
            // exact expected error; all other inaccessible PIDs fail closed.
            return Ok(false);
        }
        Err(EngineError::Install(format!(
            "cannot determine image path for process {pid}: {}",
            std::io::Error::from_raw_os_error(error_code as i32)
        )))
    }

    fn fresh_snapshot_contains_pid(pid: u32) -> Result<bool, EngineError> {
        let snapshot = OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
            .ok_or_else(|| last_error("create fresh process snapshot after invalid parameter"))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(false);
            }
            return Err(EngineError::Install(format!(
                "read first fresh process snapshot entry: {error}"
            )));
        }
        loop {
            if entry.th32ProcessID == pid {
                return Ok(true);
            }
            if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    return Ok(false);
                }
                return Err(EngineError::Install(format!(
                    "read next fresh process snapshot entry: {error}"
                )));
            }
        }
    }

    fn strict_process_image_is_under_root(
        pid: u32,
        root: &Path,
        snapshot_image_name: &std::ffi::OsStr,
    ) -> Result<bool, EngineError> {
        let query_access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        let Some(handle) = OwnedHandle::new(unsafe { OpenProcess(query_access, 0, pid) }) else {
            return strict_process_open_failure(
                pid,
                snapshot_image_name,
                unsafe { GetLastError() },
                fresh_snapshot_contains_pid,
            );
        };

        let mut buffer = vec![0u16; MAX_PROCESS_PATH_UTF16];
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle.raw(), 0, buffer.as_mut_ptr(), &mut length) }
            == 0
        {
            let code = unsafe { GetLastError() } as i32;
            match unsafe { WaitForSingleObject(handle.raw(), 0) } {
                WAIT_OBJECT_0 => return Ok(false),
                WAIT_TIMEOUT => {
                    return Err(EngineError::Install(format!(
                        "cannot query image path for live process {pid}: {}",
                        std::io::Error::from_raw_os_error(code)
                    )))
                }
                WAIT_FAILED => {
                    return Err(EngineError::Install(format!(
                        "cannot query image path or process state for pid {pid}: image error {}; wait error {}",
                        std::io::Error::from_raw_os_error(code),
                        std::io::Error::last_os_error()
                    )))
                }
                status => {
                    return Err(EngineError::Install(format!(
                        "cannot query image path for process {pid}; unexpected wait status {status}"
                    )))
                }
            }
        }
        if length == 0 || length as usize > buffer.len() {
            return Err(EngineError::Install(format!(
                "process {pid} returned an invalid image path length"
            )));
        }
        let image_path = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
        if !path_is_within_root(&image_path, root) {
            return Ok(false);
        }
        match unsafe { WaitForSingleObject(handle.raw(), 0) } {
            WAIT_OBJECT_0 => Ok(false),
            WAIT_TIMEOUT => Ok(true),
            WAIT_FAILED => Err(EngineError::Install(format!(
                "cannot confirm whether managed process {pid} is still running: {}",
                std::io::Error::last_os_error()
            ))),
            status => Err(EngineError::Install(format!(
                "managed process {pid} returned unexpected wait status {status}"
            ))),
        }
    }

    fn strict_probe_process<F>(
        pid: u32,
        snapshot_image_name: &std::ffi::OsStr,
        root: &Path,
        probe: F,
    ) -> Result<Option<bool>, EngineError>
    where
        F: FnOnce(u32, &Path, &std::ffi::OsStr) -> Result<bool, EngineError>,
    {
        // PID 0 is the kernel idle pseudo-process; PID 4 is the protected
        // Windows System process. Neither can be a user-launched Codex image,
        // and Windows does not permit querying PID 4's ordinary process image
        // handle. Do not broaden this exception to other protected processes.
        if pid == 0 || pid == 4 {
            return Ok(None);
        }
        probe(pid, root, snapshot_image_name).map(Some)
    }

    /// Strict cleanup probe. Unlike the ordinary process query, it does not
    /// silently skip live processes whose executable path cannot be queried.
    pub fn processes_running_for_root_strict(root: &Path) -> Result<bool, EngineError> {
        let snapshot = OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
            .ok_or_else(|| last_error("create strict process snapshot"))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(false);
            }
            return Err(EngineError::Install(format!(
                "read first strict process snapshot entry: {error}"
            )));
        }

        let manager_pid = std::process::id();
        let mut unresolved = None;
        loop {
            let pid = entry.th32ProcessID;
            if pid != manager_pid {
                let image_name_length = entry
                    .szExeFile
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(entry.szExeFile.len());
                let snapshot_image_name =
                    OsString::from_wide(&entry.szExeFile[..image_name_length]);
                match strict_probe_process(
                    pid,
                    &snapshot_image_name,
                    root,
                    strict_process_image_is_under_root,
                ) {
                    Ok(Some(true)) => return Ok(true),
                    Ok(Some(false)) | Ok(None) => {}
                    Err(error) if unresolved.is_none() => unresolved = Some(error),
                    Err(_) => {}
                }
            }
            if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(EngineError::Install(format!(
                    "read next strict process snapshot entry: {error}"
                )));
            }
        }
        if let Some(error) = unresolved {
            return Err(error);
        }
        Ok(false)
    }

    pub(crate) fn codex_processes_running_for_root(root: &Path) -> Result<bool, EngineError> {
        processes_running_for_root(root)
    }

    unsafe extern "system" fn post_close_to_pid(hwnd: HWND, target_pid: LPARAM) -> i32 {
        let mut window_pid = 0u32;
        // SAFETY: EnumWindows supplied a live top-level HWND; `window_pid` is a
        // valid out pointer for the duration of the call.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut window_pid);
        }
        if window_pid == target_pid as u32 {
            // Best-effort graceful close. A process without a responsive window
            // is handled by the bounded force-close phase below.
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        1
    }

    fn request_graceful_close(targets: &[TargetProcess]) {
        for target in targets.iter().filter(|target| target.is_running()) {
            // SAFETY: callback is valid for the synchronous enumeration call and
            // the PID fits losslessly in LPARAM on supported Windows targets.
            unsafe {
                EnumWindows(Some(post_close_to_pid), target.pid as LPARAM);
            }
        }
    }

    fn wait_until_exited(targets: &[TargetProcess], timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if targets.iter().all(|target| !target.is_running()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    pub fn close_processes_for_root(timeout_secs: u64, root: &Path) -> Result<(), EngineError> {
        let targets = target_processes_under_root(root)?;
        if targets.is_empty() {
            return Ok(());
        }

        request_graceful_close(&targets);
        if wait_until_exited(&targets, Duration::from_secs(timeout_secs)) {
            return Ok(());
        }

        let force_ids: Vec<u32> = targets
            .iter()
            .filter(|target| target.is_running())
            .map(|target| target.pid)
            .collect();
        for target in targets.iter().filter(|target| target.is_running()) {
            if !target.can_terminate {
                log::warn!(
                    "managed install process cannot be force-closed without PROCESS_TERMINATE access pid={}",
                    target.pid
                );
                continue;
            }
            // SAFETY: handle was opened with PROCESS_TERMINATE and is still owned.
            if unsafe { TerminateProcess(target.handle.raw(), 1) } == 0 {
                log::warn!(
                    "force-close managed install process failed pid={} error={}",
                    target.pid,
                    std::io::Error::last_os_error()
                );
            }
        }

        if wait_until_exited(&targets, FORCE_CLOSE_TIMEOUT) {
            log::warn!(
                "managed install processes required native force-close pids={:?}",
                force_ids
            );
            return Ok(());
        }

        let remaining: Vec<u32> = targets
            .iter()
            .filter(|target| target.is_running())
            .map(|target| target.pid)
            .collect();
        Err(EngineError::Install(format!(
            "managed install process is still running after native close request (pids={remaining:?}); no files were replaced"
        )))
    }

    pub(crate) fn close_codex_processes_for_root(
        timeout_secs: u64,
        root: &Path,
    ) -> Result<(), EngineError> {
        close_processes_for_root(timeout_secs, root)
    }

    #[cfg(test)]
    mod windows_tests {
        use std::process::Command;

        use super::*;

        const HELPER_ENV: &str = "CODEX_APP_MANAGER_PROCESS_HELPER";
        const STRICT_HELPER_ENV: &str = "CODEX_USB_REVOCATION_PROCESS_HELPER";

        #[test]
        fn strict_probe_skips_only_known_system_pids_and_preserves_other_errors() {
            let root = Path::new("C:\\fixture");
            for pid in [0, 4] {
                assert_eq!(
                    strict_probe_process(pid, std::ffi::OsStr::new("System"), root, |_, _, _| {
                        panic!("known Windows system PID {pid} must not be queried")
                    })
                    .unwrap(),
                    None
                );
            }
            let ordinary_error = EngineError::Install("image query denied".to_string());
            let error = strict_probe_process(
                1234,
                std::ffi::OsStr::new("helper.exe"),
                root,
                |pid, _, _| {
                    assert_eq!(pid, 1234);
                    Err(ordinary_error)
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("image query denied"));
        }

        #[test]
        fn protected_system_exception_requires_exact_name_and_invalid_handle_error() {
            let invalid_handle = ERROR_INVALID_HANDLE;
            let other_error = windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
            for (pid, name) in [(236, "Secure System"), (276, "Registry")] {
                assert!(!strict_process_open_failure(
                    pid,
                    std::ffi::OsStr::new(name),
                    invalid_handle,
                    |_| panic!("protected process exception must not request a fresh snapshot"),
                )
                .unwrap());
            }
            assert!(strict_process_open_failure(
                237,
                std::ffi::OsStr::new("Other System"),
                invalid_handle,
                |_| panic!("non-exit error must not request a fresh snapshot"),
            )
            .is_err());
            assert!(strict_process_open_failure(
                238,
                std::ffi::OsStr::new("Secure System"),
                other_error,
                |_| panic!("non-exit error must not request a fresh snapshot"),
            )
            .is_err());
            assert!(strict_process_open_failure(
                239,
                std::ffi::OsStr::new("Registry"),
                other_error,
                |_| panic!("non-exit error must not request a fresh snapshot"),
            )
            .is_err());
        }

        #[test]
        fn invalid_parameter_is_skipped_only_after_fresh_snapshot_proves_pid_vanished() {
            assert!(!strict_process_open_failure(
                1234,
                std::ffi::OsStr::new("fixture.exe"),
                ERROR_INVALID_PARAMETER,
                |pid| {
                    assert_eq!(pid, 1234);
                    Ok(false)
                },
            )
            .unwrap());
        }

        #[test]
        fn invalid_parameter_fails_closed_when_pid_remains_in_fresh_snapshot() {
            let error = strict_process_open_failure(
                1234,
                std::ffi::OsStr::new("fixture.exe"),
                ERROR_INVALID_PARAMETER,
                |pid| {
                    assert_eq!(pid, 1234);
                    Ok(true)
                },
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("still present in a fresh snapshot"));
        }

        #[test]
        fn invalid_parameter_fails_closed_when_fresh_snapshot_cannot_be_read() {
            let error = strict_process_open_failure(
                1234,
                std::ffi::OsStr::new("fixture.exe"),
                ERROR_INVALID_PARAMETER,
                |_| Err(EngineError::Install("snapshot fixture failure".to_string())),
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("cannot confirm process 1234 exited"));
            assert!(error.to_string().contains("snapshot fixture failure"));
        }

        #[test]
        fn strict_probe_helper_child() {
            if std::env::var_os(STRICT_HELPER_ENV).is_some() {
                thread::sleep(Duration::from_secs(30));
            }
        }

        #[test]
        fn strict_probe_finds_a_live_process_under_the_managed_root() {
            if std::env::var_os(STRICT_HELPER_ENV).is_some() {
                return;
            }
            let root = std::env::temp_dir().join(format!(
                "codex-strict-process-probe-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let helper = root.join("Managed.Child.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
            let mut child = Command::new(&helper)
                .args([
                    "--exact",
                    "windows_process::imp::windows_tests::strict_probe_helper_child",
                    "--nocapture",
                ])
                .env(STRICT_HELPER_ENV, "1")
                .spawn()
                .unwrap();

            let deadline = Instant::now() + Duration::from_secs(10);
            let found = loop {
                match processes_running_for_root_strict(&root) {
                    Ok(true) => break true,
                    Ok(false) | Err(_) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Ok(false) | Err(_) => break false,
                }
            };
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&root);
            assert!(found, "strict process probe missed a live managed process");
        }

        #[test]
        fn closes_unlisted_helper_process_without_powershell() {
            if std::env::var_os(HELPER_ENV).is_some() {
                thread::sleep(Duration::from_secs(30));
                return;
            }

            let root = std::env::temp_dir().join(format!(
                "codex-native-close-test-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            // The scanner must not depend on the two known entrypoint names:
            // Chromium/Electron helpers can hold payload files open too.
            let helper = root.join("Codex.Helper.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
            // Arm the out-of-context listener before child creation, matching
            // the production launch ordering and exercising hook setup/teardown.
            start_console_window_watcher(&root).unwrap();
            let mut child = Command::new(&helper)
                .args([
                    "--exact",
                    "windows_process::imp::windows_tests::closes_unlisted_helper_process_without_powershell",
                    "--nocapture",
                ])
                .env(HELPER_ENV, "1")
                .spawn()
                .unwrap();

            let discover_deadline = Instant::now() + Duration::from_secs(10);
            while target_processes_under_root(&root).unwrap().is_empty() {
                if Instant::now() >= discover_deadline {
                    let _ = child.kill();
                    panic!("helper process was not discovered under its install root");
                }
                thread::sleep(Duration::from_millis(50));
            }
            assert!(codex_processes_running_for_root(&root).unwrap());

            let result = close_codex_processes_for_root(0, &root);
            if result.is_err() {
                let _ = child.kill();
            }
            result.unwrap();
            assert!(child.wait().unwrap().code().is_some());
            assert!(!codex_processes_running_for_root(&root).unwrap());
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[cfg(windows)]
pub use imp::{
    close_processes_for_root, hide_codex_auxiliary_windows_for_root, hide_console_windows_for_root,
    hide_windows_for_root, processes_running_for_root, processes_running_for_root_strict,
    show_windows_for_root, start_console_window_watcher, wait_for_usable_window_for_root,
};

#[cfg(windows)]
pub(crate) use imp::{close_codex_processes_for_root, codex_processes_running_for_root};

#[cfg(not(windows))]
pub(crate) fn close_codex_processes_for_root(
    _timeout_secs: u64,
    _root: &Path,
) -> Result<(), EngineError> {
    Ok(())
}

#[cfg(not(windows))]
pub fn close_processes_for_root(_timeout_secs: u64, _root: &Path) -> Result<(), EngineError> {
    Ok(())
}

#[cfg(not(windows))]
pub fn hide_windows_for_root(_root: &Path) -> Result<usize, EngineError> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn show_windows_for_root(_root: &Path) -> Result<usize, EngineError> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn hide_console_windows_for_root(_root: &Path) -> Result<usize, EngineError> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn hide_codex_auxiliary_windows_for_root(_root: &Path) -> Result<usize, EngineError> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn start_console_window_watcher(_root: &Path) -> Result<(), EngineError> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn codex_processes_running_for_root(_root: &Path) -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn processes_running_for_root(_root: &Path) -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn processes_running_for_root_strict(_root: &Path) -> Result<bool, EngineError> {
    Err(EngineError::Install(
        "strict process image query is supported only on Windows".to_string(),
    ))
}

#[cfg(not(windows))]
pub fn wait_for_usable_window_for_root(
    _root: &Path,
    _timeout: std::time::Duration,
) -> Result<Option<UsableWindowReport>, EngineError> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_windows_paths_case_insensitively_and_on_component_boundaries() {
        let root = Path::new(r"C:\Users\Alice\Apps\Codex");
        assert!(path_is_within_root(
            Path::new(r"c:/users/ALICE/apps/codex/ChatGPT.exe"),
            root
        ));
        assert!(path_is_within_root(root, root));
        assert!(!path_is_within_root(
            Path::new(r"C:\Users\Alice\Apps\Codex-old\ChatGPT.exe"),
            root
        ));
    }

    #[test]
    fn normalizes_extended_drive_and_unc_prefixes() {
        assert_eq!(
            normalize_windows_path_text(r"\\?\C:\Users\Alice\Codex\"),
            r"c:\users\alice\codex"
        );
        assert_eq!(
            normalize_windows_path_text(r"\\?\UNC\server\share\Codex\"),
            r"\\server\share\codex"
        );
    }

    #[test]
    fn same_path_ignores_case_separators_and_extended_prefix() {
        assert!(same_windows_path(
            Path::new(r"\\?\C:\Users\Alice\Codex\"),
            Path::new(r"c:/users/alice/codex")
        ));
        assert!(!same_windows_path(
            Path::new(r"C:\Users\Alice\Codex"),
            Path::new(r"C:\Users\Alice\Other")
        ));
    }

    #[test]
    fn rejects_tiny_chromium_bookkeeping_windows() {
        assert!(!window_dimensions_are_usable(14, 14));
        assert!(!window_dimensions_are_usable(319, 600));
        assert!(window_dimensions_are_usable(914, 638));
    }

    #[test]
    fn recognizes_codex_startup_error_dialog_without_rejecting_normal_windows() {
        assert!(is_codex_startup_failure_dialog("#32770", "ChatGPT"));
        assert!(is_codex_startup_failure_dialog("#32770", "程序包标识符"));
        assert!(!is_codex_startup_failure_dialog(
            "Chrome_WidgetWin_1",
            "ChatGPT"
        ));
        assert!(!is_codex_startup_failure_dialog("#32770", "另一个应用"));
    }

    #[test]
    fn rejects_titleless_chromium_auxiliary_window_from_launch_restore() {
        assert!(is_codex_auxiliary_window("Chrome_WidgetWin_0", ""));
        assert!(!is_codex_auxiliary_window("Chrome_WidgetWin_0", "ChatGPT"));
        assert!(!is_codex_auxiliary_window("Chrome_WidgetWin_1", "ChatGPT"));

        assert!(!is_user_operable_window("Chrome_WidgetWin_0", ""));
        assert!(!is_user_operable_window("Chrome_WidgetWin_0", "ChatGPT"));
        assert!(is_user_operable_window("Chrome_WidgetWin_1", ""));
        assert!(is_user_operable_window("Chrome_WidgetWin_1", "ChatGPT"));
        assert!(is_user_operable_window("Tauri Window", ""));
    }

    #[test]
    fn lifecycle_hook_only_considers_managed_window_classes() {
        assert!(is_lifecycle_window_class("ConsoleWindowClass"));
        assert!(is_lifecycle_window_class("Chrome_WidgetWin_0"));
        assert!(!is_lifecycle_window_class("Chrome_WidgetWin_1"));
        assert!(!is_lifecycle_window_class("Tauri Window"));
    }
}
