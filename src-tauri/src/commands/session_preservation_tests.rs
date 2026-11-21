//! Windows launch-regression fixtures. Each case runs in a fresh manager process
//! with a private application root and a real top-level HWND owned by a sibling.
use super::saved_theme_notice;
use crate::app::{codex_theme, paths, settings_store::AppSettings, win_update::ThemeOnOpen};
use codex_win_engine::{InstalledWindowsCodex, LaunchOptions, PortableLaunchPolicy};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn subprocess(name: &str, case: &str, image: &Path) -> Command {
    let mut cmd = Command::new(image);
    cmd.args([
        "--exact",
        &format!("commands::session_preservation_tests::{name}"),
        "--nocapture",
    ])
    .env("USB_SESSION_REGRESSION", case)
    .creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[test]
fn window_child() {
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok("window") {
        return;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            240,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        assert!(!hwnd.is_null());
        println!("session-window-ready");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        let mut msg = std::mem::zeroed();
        assert_eq!(
            PeekMessageW(&mut msg, hwnd, WM_CLOSE, WM_CLOSE, PM_REMOVE),
            0,
            "the previous session received WM_CLOSE"
        );
        assert_eq!(input.trim(), "check");
        DestroyWindow(hwnd);
    }
}

fn start_window(dir: &Path) -> Child {
    let image = dir.join("ChatGPT.exe");
    fs::copy(std::env::current_exe().unwrap(), &image).unwrap();
    let mut child = subprocess("window_child", "window", &image)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stdout.as_mut().unwrap()).lines();
    assert!(lines
        .map_while(Result::ok)
        .any(|line| line.contains("session-window-ready")));
    child
}

fn run_in_new_manager(name: &str, with_existing_window: bool) {
    let run = format!("session-{}", uuid::Uuid::new_v4());
    let root = std::env::temp_dir().join(format!("codex-app-manager-smoke-{run}"));
    fs::create_dir(&root).unwrap();
    let window = with_existing_window.then(|| start_window(&root));
    let status = subprocess(name, name, &std::env::current_exe().unwrap())
        .env("CAM_PACKAGED_SMOKE_RUN", &run)
        .env("CAM_PACKAGED_SMOKE_DATA_DIR", &root)
        .env("CODEX_HOME", root.join("home"))
        .status()
        .unwrap();
    if let Some(mut window) = window {
        assert!(
            window.try_wait().unwrap().is_none(),
            "window PID {} exited",
            window.id()
        );
        window.stdin.take().unwrap().write_all(b"check\n").unwrap();
        assert!(window.wait().unwrap().success());
    } else if !status.success() {
        // Let even a failed test release its own executable before cleanup.
        fs::write(root.join("stop-all"), b"").unwrap();
        wait_until(|| !codex_win_engine::codex_running_for_root(&root).unwrap());
    }
    fs::remove_dir_all(root).unwrap();
    assert!(status.success(), "manager={status}");
}

#[test]
fn reopened_manager_preserves_existing_window() {
    let name = "reopened_manager_preserves_existing_window";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, true);
    }
    let root = paths::data_dir().unwrap();
    let installed = InstalledWindowsCodex {
        app_version_unavailable: false,
        path: root.to_str().unwrap().into(),
        source: "portable".into(),
        version: "fixture".into(),
        arch: None,
        package_family_name: None,
        package_volume: None,
        installed_at: None,
    };
    // A pending model reload must also survive an unsuccessful ordinary Open.
    use crate::app::{model_catalog, usb_guard, win_update};
    let home = root.join("home");
    assert!(!usb_guard::guard_required());
    fs::create_dir(&home).unwrap();
    assert_eq!(
        fs::canonicalize(std::env::var_os("CODEX_HOME").unwrap()).unwrap(),
        home
    );
    model_catalog::mark_restart_required().unwrap();
    let marker = model_catalog::restart_marker_snapshot_checked().unwrap();
    assert!(marker.is_some());
    let options = LaunchOptions {
        codex_home: Some(home),
        portable_launch_policy: PortableLaunchPolicy::UsbCompatible,
        ..Default::default()
    };
    let error = win_update::open_with_catalog_snapshot(
        &installed,
        options,
        marker.as_deref(),
        false,
        || panic!("launched over an existing session"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("已保留现有进程"), "{error}");
    assert_eq!(
        model_catalog::restart_marker_snapshot_checked().unwrap(),
        marker
    );
}

#[test]
fn saved_theme_renderer_failure_keeps_open_window() {
    let name = "saved_theme_renderer_failure_keeps_open_window";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, true);
    }
    let root = paths::data_dir().unwrap();
    assert!(paths::packaged_smoke_run_id().is_some());
    // An existing backup makes the real hot-apply path skip reading the user config.
    fs::write(root.join("codex-theme-native-backup.json"), b"{}").unwrap();
    let theme = root.join("test-skin");
    fs::create_dir(&theme).unwrap();
    let palette = serde_json::json!({"accent":"#556677","contrast":60,"ink":"#112233","opaqueWindows":true,
        "surface":"#ffffff","fonts":{"code":null,"ui":null},
        "semanticColors":{"diffAdded":"#117733","diffRemoved":"#ff2233","skill":"#556677"}});
    let json = serde_json::json!({"schemaVersion":2,"id":"test-skin","codexTheme":{"appearanceTheme":"dark",
        "codeThemeIds":{"dark":"absolutely","light":"absolutely"},"dark":palette,"light":palette}});
    fs::write(theme.join("theme.json"), json.to_string()).unwrap();
    fs::write(theme.join("theme.css"), "").unwrap();

    // Real readiness succeeds; renderer selection fails at the same hot-apply
    // boundary which previously led to a stop-and-rewrite fallback.
    let socket =
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, codex_theme::THEME_CDP_PORT)).unwrap();
    std::thread::spawn(move || {
        for stream in socket.incoming() {
            let mut stream = stream.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut header = [0; 2048];
            let n = stream.read(&mut header).unwrap_or(0);
            let body = if header[..n].starts_with(b"GET /json/version ") {
                r#"{"Browser":"fixture"}"#
            } else {
                "[]"
            };
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let settings = AppSettings {
        codex_theme: Some("test-skin".into()),
        codex_theme_store_dir: Some(root.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let service = codex_theme::ThemeService::default();
    let notice = tauri::async_runtime::block_on(saved_theme_notice(ThemeOnOpen::Restore, async {
        let err = service
            .restore_on_open(&settings, "test-skin")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no verified Codex renderer"),
            "{err}"
        );
        Err(err)
    }))
    .unwrap();
    assert!(notice.contains("本次未能应用已保存的主题"));
    assert!(!root.join("codex-theme-native-tx").exists());
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            std::time::Instant::now() < until,
            "test window did not exit"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn managed_window_fixture() -> (InstalledWindowsCodex, LaunchOptions) {
    use crate::app::usb_guard;
    let root = paths::data_dir().unwrap();
    let home = root.join("home");
    assert!(paths::packaged_smoke_run_id().is_some() && !usb_guard::guard_required());
    fs::create_dir(&home).unwrap();
    assert_eq!(
        fs::canonicalize(std::env::var_os("CODEX_HOME").unwrap()).unwrap(),
        home
    );
    let source = root.join("LaunchWindow.cs");
    fs::write(&source, include_str!("fixtures/LaunchWindow.cs")).unwrap();
    let csc = Path::new(&std::env::var_os("SystemRoot").unwrap())
        .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
    let status = Command::new(csc)
        .current_dir(&root)
        .args([
            "/nologo",
            "/target:winexe",
            "/r:System.Drawing.dll",
            "/r:System.Windows.Forms.dll",
            "/out:ChatGPT.exe",
            "LaunchWindow.cs",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let app = InstalledWindowsCodex {
        app_version_unavailable: false,
        path: root.to_str().unwrap().into(),
        source: "portable".into(),
        version: "fixture".into(),
        arch: None,
        package_family_name: None,
        package_volume: None,
        installed_at: None,
    };
    let options = LaunchOptions {
        codex_home: Some(home),
        portable_launch_policy: PortableLaunchPolicy::UsbCompatible,
        ..Default::default()
    };
    (app, options)
}

fn fixture_pids(root: &Path, file: &str) -> Vec<u32> {
    fs::read_to_string(root.join(file))
        .unwrap()
        .lines()
        .map(|pid| pid.parse().unwrap())
        .collect()
}

fn finish_window(root: &Path, pid: u32) {
    fs::write(root.join(format!("stop-{pid}")), b"").unwrap();
    wait_until(|| !codex_win_engine::codex_running_for_root(root).unwrap());
}

#[test]
fn transient_window_during_first_launch_keeps_its_pid() {
    let name = "transient_window_during_first_launch_keeps_its_pid";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, false);
    }
    let (app, options) = managed_window_fixture();
    let root = Path::new(&app.path);
    fs::write(root.join("transient"), b"").unwrap();
    let report = codex_win_engine::launch_codex_with_options(&app, options).unwrap();
    assert_eq!(report.launch_mode, "portable");
    assert!(report.usable_window_confirmed);
    let pids = fixture_pids(root, "pids");
    assert_eq!(pids.len(), 1);
    assert_eq!(fixture_pids(root, "temporarily-small"), pids);
    assert_eq!(fixture_pids(root, "restored"), pids);
    assert!(!root.join("unexpected-close").exists());
    finish_window(root, pids[0]);
}

#[test]
fn ordinary_open_keeps_catalog_marker_until_next_natural_launch() {
    use crate::app::{model_catalog, win_update};
    let name = "ordinary_open_keeps_catalog_marker_until_next_natural_launch";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, false);
    }
    let (app, options) = managed_window_fixture();
    let root = Path::new(&app.path);
    // The existing session may have been opened with a saved theme.
    let first = LaunchOptions {
        remote_debugging_port: Some(codex_theme::THEME_CDP_PORT),
        ..options.clone()
    };
    win_update::open_with_catalog_snapshot(&app, first, None, true, || {})
        .unwrap()
        .unwrap();
    let original = fixture_pids(root, "pids");
    assert_eq!(original.len(), 1);

    model_catalog::mark_restart_required().unwrap();
    let marker = win_update::pending_usb_catalog_reload().unwrap().unwrap();
    let home = options.codex_home.as_deref().unwrap();
    let denied = codex_win_engine::sys::try_focus_live_usb_session(root, home, || {
        Err(crate::errors::AppError::Engine("test: authorization denied".into()))
    });
    assert!(denied.is_err());
    assert!(codex_win_engine::sys::try_focus_live_usb_session(root, home, || Ok::<_, crate::errors::AppError>(())).unwrap());
    assert!(!codex_win_engine::sys::try_focus_live_usb_session(root, &root.join("other-home"), || -> Result<(), crate::errors::AppError> {
        panic!("foreign home is ineligible for fast focus")
    }).unwrap());
    let reused =
        win_update::open_with_catalog_snapshot(&app, options.clone(), Some(&marker), false, || {
            panic!("ordinary Open attempted a second spawn")
        })
        .unwrap();
    assert!(reused.is_none());
    assert_eq!(fixture_pids(root, "pids"), original);
    assert!(!root.join("unexpected-close").exists());
    assert_eq!(
        model_catalog::restart_marker_snapshot_checked()
            .unwrap()
            .as_deref(),
        Some(marker.as_slice())
    );

    finish_window(root, original[0]);
    let next_marker = win_update::pending_usb_catalog_reload().unwrap().unwrap();
    assert_eq!(next_marker, marker);
    let mut spawned = false;
    let next =
        win_update::open_with_catalog_snapshot(&app, options, Some(&next_marker), false, || {
            spawned = true;
        })
        .unwrap()
        .unwrap();
    assert!(spawned && next.usable_window_confirmed);
    assert!(model_catalog::restart_marker_snapshot_checked()
        .unwrap()
        .is_none());
    let pids = fixture_pids(root, "pids");
    assert_eq!(pids.len(), 2);
    assert_ne!(pids[0], pids[1]);
    assert!(!root.join("unexpected-close").exists());
    finish_window(root, pids[1]);
}

// Keep a handle to the exact fixture process across exit and possible PID reuse.
fn observe_fixture_pid(pid: u32) -> std::os::windows::io::OwnedHandle {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null());
    unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) }
}

fn fixture_exited(handle: &std::os::windows::io::OwnedHandle) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::WaitForSingleObject,
    };
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => true,
        WAIT_TIMEOUT => false,
        other => panic!("fixture process wait failed: {other}"),
    }
}

#[test]
fn clean_close_add_model_and_relaunch_reaps_verified_helper() {
    use crate::app::{model_catalog, win_update};
    let name = "clean_close_add_model_and_relaunch_reaps_verified_helper";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, false);
    }
    let (app, mut options) = managed_window_fixture();
    // The production writer and the launch preflight use the same CODEX_HOME spelling.
    let home = std::path::PathBuf::from(std::env::var_os("CODEX_HOME").unwrap());
    options.codex_home = Some(home.clone());
    let root = Path::new(&app.path);
    fs::write(root.join("spawn-helper"), b"").unwrap();
    let first = win_update::open_with_catalog_snapshot(&app, options.clone(), None, false, || {})
        .unwrap()
        .unwrap();
    assert!(first.usable_window_confirmed);
    let pid = fixture_pids(root, "pids")[0];
    let original = observe_fixture_pid(pid);
    let helper = observe_fixture_pid(fixture_pids(root, "helpers")[0]);
    fs::write(root.join(format!("stop-{pid}")), b"").unwrap();
    wait_until(|| fixture_exited(&original));
    assert!(!fixture_exited(&helper));

    let model = "close-relaunch-fixture";
    let result = model_catalog::add_tested_model_files(
        &home,
        model,
        None,
        &false.into(),
        || Ok(true),
        || {},
        model_catalog::mark_restart_required,
    )
    .unwrap();
    assert!(result.model_ids.iter().any(|id| id == model));
    let marker = win_update::pending_usb_catalog_reload().unwrap().unwrap();
    fs::write(root.join("observe-relaunch-home"), b"").unwrap();
    let second = win_update::open_with_catalog_snapshot(&app, options, Some(&marker), false, || {})
        .unwrap()
        .unwrap();
    assert!(second.usable_window_confirmed && fixture_exited(&helper));
    assert!(win_update::pending_usb_catalog_reload().unwrap().is_none());
    let pids = fixture_pids(root, "pids");
    assert_eq!(pids.len(), 2);
    assert_ne!(pid, pids[1]);
    let effective_home =
        fs::read_to_string(root.join(format!("relaunch-home-{}", pids[1]))).unwrap();
    assert_eq!(Path::new(&effective_home), home);
    let config: toml::Value = fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap())
            .unwrap();
    assert!(catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["slug"] == model));
    if let Some(cli) = std::env::var_os("USB_RELAUNCH_TEST_CLI") {
        let output = Command::new(cli)
            .args(["debug", "models"])
            .env("CODEX_HOME", effective_home)
            .current_dir(root)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "bundled catalog reader failed: {:?}",
            output.status.code()
        );
        let models: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(models
            .as_array()
            .or_else(|| models["models"].as_array())
            .unwrap()
            .iter()
            .any(|row| row["slug"] == model));
        println!("Bundled Codex catalog reader returned the added model in the relaunched home.");
    }
    finish_window(root, pids[1]);
}

#[test]
fn clean_close_reaps_helper_but_preserves_headless_successor() {
    use crate::app::{model_catalog, win_update};
    let name = "clean_close_reaps_helper_but_preserves_headless_successor";
    if std::env::var("USB_SESSION_REGRESSION").as_deref() != Ok(name) {
        return run_in_new_manager(name, false);
    }
    let (app, options) = managed_window_fixture();
    let root = Path::new(&app.path);
    fs::write(root.join("spawn-helper"), b"").unwrap();
    fs::write(root.join("spawn-successor"), b"").unwrap();
    win_update::open_with_catalog_snapshot(&app, options.clone(), None, false, || {})
        .unwrap()
        .unwrap();
    let pid = fixture_pids(root, "pids")[0];
    let original = observe_fixture_pid(pid);
    let helper = observe_fixture_pid(fixture_pids(root, "helpers")[0]);
    let successor_pid = fixture_pids(root, "successors")[0];
    let successor = observe_fixture_pid(successor_pid);
    fs::write(root.join(format!("stop-{pid}")), b"").unwrap();
    wait_until(|| fixture_exited(&original));
    model_catalog::mark_restart_required().unwrap();
    let marker = win_update::pending_usb_catalog_reload().unwrap().unwrap();
    assert!(!codex_win_engine::sys::try_focus_live_usb_session(root, options.codex_home.as_deref().unwrap(), || -> Result<(), crate::errors::AppError> {
        panic!("a dead main process is ineligible for fast focus")
    }).unwrap());
    assert!(!fixture_exited(&helper) && !fixture_exited(&successor));
    let result =
        win_update::open_with_catalog_snapshot(&app, options, Some(&marker), false, || {
            panic!("duplicate launch")
        });
    assert!(
        result.is_err() && fixture_exited(&helper) && !fixture_exited(&successor),
        "{result:?}"
    );
    assert_eq!(
        win_update::pending_usb_catalog_reload().unwrap().unwrap(),
        marker
    );
    fs::write(root.join("show-successor"), b"").unwrap();
    wait_until(|| fixture_pids(root, "pids").contains(&successor_pid));
    assert_eq!(fixture_pids(root, "pids"), [pid, successor_pid]);
    finish_window(root, successor_pid);
}

/// The release smoke runner supplies a disposable copy of the installed desktop
/// under the validated smoke root. It uses the desktop Quit action in each run.
#[test]
#[ignore = "requires isolated installed desktop and a Windows window controller"]
fn real_desktop_close_add_and_relaunch() {
    use crate::app::{model_catalog, usb_guard, win_update};
    assert!(paths::packaged_smoke_run_id().is_some() && !usb_guard::guard_required());
    let root = paths::data_dir().unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("CODEX_HOME").unwrap());
    assert_eq!(home.canonicalize().unwrap(), root.join("home"));
    let app_root = root.join("apps/Codex");
    assert!(!codex_win_engine::codex_running_for_root(&app_root).unwrap());
    let app = InstalledWindowsCodex {
        app_version_unavailable: false,
        path: app_root.to_str().unwrap().into(),
        source: "portable".into(),
        version: "26.1111.21308".into(),
        arch: None,
        package_family_name: None,
        package_volume: None,
        installed_at: None,
    };
    let mut options = LaunchOptions {
        codex_home: Some(home.clone()),
        require_usb_entry_overlay: true,
        usb_integrity_transaction: true,
        portable_launch_policy: PortableLaunchPolicy::UsbCompatible,
        ..Default::default()
    };
    let mut marker = None;
    for stage in ["first", "reopened"] {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        options.remote_debugging_port = Some(port);
        fs::write(root.join(format!("{stage}.port")), port.to_string()).unwrap();
        let report = win_update::open_with_catalog_snapshot(
            &app,
            options.clone(),
            marker.as_deref(),
            false,
            || {},
        )
        .unwrap()
        .unwrap();
        assert!(report.usable_window_confirmed);
        let window = codex_win_engine::wait_for_usable_window_for_root(&app_root, Duration::ZERO)
            .unwrap()
            .unwrap();
        let main = observe_fixture_pid(window.pid);
        println!("desktop {stage} pid={}", window.pid);
        fs::write(root.join(format!("{stage}.pid")), window.pid.to_string()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(100);
        while !fixture_exited(&main) {
            assert!(
                std::time::Instant::now() < deadline,
                "desktop did not close"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        if stage == "first" {
            let result = model_catalog::add_tested_model_files(
                &home,
                "close-relaunch-fixture",
                None,
                &false.into(),
                || Ok(true),
                || {},
                model_catalog::mark_restart_required,
            )
            .unwrap();
            assert!(result
                .model_ids
                .iter()
                .any(|id| id == "close-relaunch-fixture"));
            marker = Some(win_update::pending_usb_catalog_reload().unwrap().unwrap());
        } else {
            assert!(win_update::pending_usb_catalog_reload().unwrap().is_none());
        }
    }
}
