//! Native Windows front end for the USB license signer.
//!
//! This module is intentionally an interaction layer.  Every scan, deep
//! inspection, sign and verify operation is sent through the same
//! `run_with_args` command pipeline used by the console signer.

#![cfg(target_os = "windows")]
#![allow(clippy::too_many_arguments)]

use std::collections::VecDeque;
use std::env;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use usb_license_signer::run_with_args;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::UpdateWindow;
use windows_sys::Win32::Graphics::Gdi::{
    GetStockObject, GetSysColorBrush, COLOR_WINDOW, DEFAULT_GUI_FONT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, LoadCursorW,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, ShowWindow,
    TranslateMessage, CBS_DROPDOWNLIST, CBS_HASSTRINGS, CB_ADDSTRING, CB_RESETCONTENT,
    CB_SETCURSEL, CW_USEDEFAULT, ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, ES_READONLY, HMENU,
    IDC_ARROW, MB_ICONERROR, MB_ICONWARNING, MB_OK, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_CLIENTEDGE, WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_TARGETS: i32 = 1001;
const ID_REFRESH: i32 = 1002;
const ID_SIGN: i32 = 1006;
const ID_SUMMARY: i32 = 1007;
const ID_STATUS: i32 = 1008;

const WM_APP_WORK_COMPLETE: u32 = WM_APP + 41;
const WM_APP_REFRESH_CONTROLS: u32 = WM_APP + 42;
type HwndValue = isize;

static STATE: OnceLock<Mutex<GuiState>> = OnceLock::new();

#[derive(Debug, Clone)]
struct GuiCandidate {
    drive_root: String,
    volume_serial: u32,
    file_system: String,
    bundle_relative_path: String,
    bundle_path: String,
    anchor_schema: u32,
    anchor_sha256: String,
}

#[derive(Debug, Clone)]
struct InspectView {
    value: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct AutoKeyStatus {
    matched: bool,
    generated: bool,
    error: Option<String>,
}

#[derive(Debug)]
struct SignOutcome {
    sign: Result<Value, String>,
    verify: Option<Result<Value, String>>,
    report_path: String,
}

#[derive(Debug)]
enum WorkerEvent {
    Scan(Result<Vec<GuiCandidate>, String>),
    Inspect { index: usize, result: InspectView },
    Sign(SignOutcome),
}

#[derive(Debug)]
struct GuiState {
    window: HwndValue,
    targets: HwndValue,
    refresh: HwndValue,
    sign: HwndValue,
    summary: HwndValue,
    status: HwndValue,
    candidates: Vec<GuiCandidate>,
    selected: Option<usize>,
    inspection: Option<InspectView>,
    pending: VecDeque<WorkerEvent>,
    operation_id: u64,
    busy: bool,
    sign_blocked: bool,
    key_status: Option<AutoKeyStatus>,
    status_text: String,
    last_result: Option<String>,
}

impl GuiState {
    fn new() -> Self {
        Self {
            window: 0,
            targets: 0,
            refresh: 0,
            sign: 0,
            summary: 0,
            status: 0,
            candidates: Vec::new(),
            selected: None,
            inspection: None,
            pending: VecDeque::new(),
            operation_id: 0,
            busy: false,
            sign_blocked: false,
            key_status: None,
            status_text: "正在初始化…".to_string(),
            last_result: None,
        }
    }
}

pub fn run() {
    unsafe { run_windows() }
}

unsafe fn run_windows() {
    startup_log("run_windows:start");
    if STATE.set(Mutex::new(GuiState::new())).is_err() {
        startup_log("run_windows:state-already-initialized");
        return;
    }
    startup_log("run_windows:state-ready");

    let instance = GetModuleHandleW(null());
    if instance.is_null() {
        startup_log("run_windows:get-module-handle:failed");
        show_message(
            null_mut(),
            "无法取得 Windows 应用实例。",
            "Codex U 盘签名授权",
            MB_ICONERROR | MB_OK,
        );
        return;
    }
    startup_log("run_windows:get-module-handle:ok");

    let class_name = wide("CodexUsbLicenseSignerGuiClass");
    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance as HINSTANCE,
        hIcon: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_WINDOW),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };
    if RegisterClassW(&window_class) == 0 {
        startup_log("run_windows:register-class:failed");
        show_message(
            null_mut(),
            "无法注册签名工具窗口。",
            "Codex U 盘签名授权",
            MB_ICONERROR | MB_OK,
        );
        return;
    }
    startup_log("run_windows:register-class:ok");

    let title = wide("Codex U 盘签名授权");
    startup_log("run_windows:create-window:start");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        820,
        640,
        null_mut(),
        null_mut(),
        instance as HINSTANCE,
        null(),
    );
    if hwnd.is_null() {
        startup_log("run_windows:create-window:failed");
        show_message(
            null_mut(),
            "无法创建签名工具窗口。",
            "Codex U 盘签名授权",
            MB_ICONERROR | MB_OK,
        );
        return;
    }
    startup_log("run_windows:create-window:ok");
    ShowWindow(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW);
    UpdateWindow(hwnd);

    let mut message = std::mem::zeroed();
    loop {
        let result = GetMessageW(&mut message, null_mut(), 0, 0);
        if result <= 0 {
            break;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            startup_log("window-proc:wm-create:start");
            create_controls(hwnd);
            startup_log("window-proc:wm-create:controls-done");
            start_scan(hwnd);
            startup_log("window-proc:wm-create:scan-started");
            0
        }
        WM_COMMAND => {
            handle_command(hwnd, wparam);
            0
        }
        WM_APP_WORK_COMPLETE => {
            startup_log("window-proc:wm-app-work-complete");
            handle_worker_event(hwnd);
            0
        }
        WM_APP_REFRESH_CONTROLS => {
            startup_log("window-proc:wm-app-refresh-controls");
            refresh_controls();
            0
        }
        WM_SIZE => {
            resize_summary(hwnd);
            0
        }
        WM_CLOSE => {
            let busy = STATE
                .get()
                .and_then(|state| state.lock().ok().map(|state| state.busy))
                .unwrap_or(false);
            if busy {
                show_message(
                    hwnd,
                    "当前正在执行介质检查或签名，请等待本次操作结束。",
                    "操作进行中",
                    MB_ICONWARNING | MB_OK,
                );
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn create_controls(hwnd: HWND) {
    startup_log("create-controls:start");
    let instance = GetModuleHandleW(null()) as HINSTANCE;
    let status_text = {
        let Some(lock) = STATE.get() else { return };
        let Ok(mut state) = lock.lock() else { return };
        state.window = hwnd_value(hwnd);
        state.status_text.clone()
    };

    create_label(hwnd, instance, "Codex U 盘签名授权", 20, 16, 500, 28, 18);
    create_label(
        hwnd,
        instance,
        "将完整安装包放在可移动 NTFS U 盘上；本工具只完成 U 盘内部授权签名。",
        20,
        48,
        740,
        22,
        0,
    );
    create_label(
        hwnd,
        instance,
        "系统 PE/MSIX Authenticode 不作为本流程的签名条件。",
        20,
        68,
        740,
        18,
        0,
    );
    create_label(
        hwnd,
        instance,
        "自动识别的目标 Bundle（必须唯一）",
        20,
        82,
        260,
        22,
        0,
    );
    let targets = hwnd_value(create_control(
        hwnd,
        instance,
        "COMBOBOX",
        "",
        WS_TABSTOP | CBS_DROPDOWNLIST as u32 | CBS_HASSTRINGS as u32 | WS_VSCROLL,
        WS_EX_CLIENTEDGE,
        20,
        105,
        655,
        180,
        ID_TARGETS,
    ));
    let refresh = create_button(hwnd, instance, "重新扫描", 690, 105, 100, 28, ID_REFRESH);

    create_label(
        hwnd,
        instance,
        "签名前检查摘要（密钥自动管理）",
        20,
        320,
        300,
        22,
        0,
    );
    let summary = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        "正在读取…",
        WS_TABSTOP
            | ES_LEFT as u32
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_READONLY as u32
            | WS_BORDER
            | WS_VSCROLL
            | WS_HSCROLL,
        WS_EX_CLIENTEDGE,
        20,
        344,
        770,
        190,
        ID_SUMMARY,
    ));
    let status = hwnd_value(create_control(
        hwnd,
        instance,
        "STATIC",
        &status_text,
        0,
        0,
        20,
        545,
        540,
        30,
        ID_STATUS,
    ));
    let sign = create_button(hwnd, instance, "一键签名 U 盘", 600, 540, 190, 36, ID_SIGN);
    let Some(lock) = STATE.get() else { return };
    let Ok(mut state) = lock.lock() else { return };
    state.targets = targets;
    state.refresh = refresh;
    state.summary = summary;
    state.status = status;
    state.sign = sign;
    update_controls_locked(&mut state);
    startup_log("create-controls:done");
}

unsafe fn create_label(
    parent: HWND,
    instance: HINSTANCE,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    _emphasis: i32,
) -> HWND {
    create_control(
        parent, instance, "STATIC", text, 0, 0, x, y, width, height, 0,
    )
}

unsafe fn create_button(
    parent: HWND,
    instance: HINSTANCE,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
) -> HwndValue {
    hwnd_value(create_control(
        parent,
        instance,
        "BUTTON",
        text,
        WS_TABSTOP | windows_sys::Win32::UI::WindowsAndMessaging::BS_DEFPUSHBUTTON as u32,
        0,
        x,
        y,
        width,
        height,
        id,
    ))
}

unsafe fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    class_name: &str,
    text: &str,
    style_extra: u32,
    ex_style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
) -> HWND {
    let class = wide(class_name);
    let title = wide(text);
    let child = CreateWindowExW(
        ex_style,
        class.as_ptr(),
        title.as_ptr(),
        WS_CHILD | WS_VISIBLE | style_extra,
        x,
        y,
        width,
        height,
        parent,
        id_to_menu(id),
        instance,
        null(),
    );
    if !child.is_null() {
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(child, WM_SETFONT, font as WPARAM, 1);
    }
    child
}

fn start_scan(_hwnd: HWND) {
    let Some((token, window)) = begin_operation("正在扫描可移动 NTFS U 盘…") else {
        return;
    };
    thread::spawn(move || {
        let result = scan_candidates();
        publish(token, WorkerEvent::Scan(result), window);
    });
}

fn start_inspect(index: usize) {
    let (token, window, candidate) = {
        let Some(lock) = STATE.get() else { return };
        let Ok(mut state) = lock.lock() else { return };
        let Some(candidate) = state.candidates.get(index).cloned() else {
            return;
        };
        state.selected = Some(index);
        state.inspection = None;
        let Some((token, window)) = begin_operation_locked(&mut state, "正在深度检查目标 Bundle…")
        else {
            return;
        };
        (token, window, candidate)
    };
    // The operation state is now unlocked.  Refresh synchronously on the UI
    // thread before the worker starts so the window cannot retain the
    // previous candidate's status or disabled sign button.
    refresh_controls();
    thread::spawn(move || {
        let result = match inspect_candidate(&candidate) {
            Ok(value) => InspectView {
                value: Some(value),
                error: None,
            },
            Err(error) => InspectView {
                value: None,
                error: Some(error),
            },
        };
        publish(token, WorkerEvent::Inspect { index, result }, window);
    });
}

fn start_sign(request: SignRequest) {
    let Some((token, window)) = begin_operation("正在签名并回读验证，请勿拔出 U 盘…")
    else {
        return;
    };
    thread::spawn(move || {
        let sign = run_command(&request.sign_args());
        let verify = if sign.is_ok() {
            Some(run_command(&request.verify_args()))
        } else {
            None
        };
        publish(
            token,
            WorkerEvent::Sign(SignOutcome {
                sign,
                verify,
                report_path: request.report_path,
            }),
            window,
        );
    });
}

#[derive(Debug)]
struct SignRequest {
    drive: String,
    relative: String,
    key_id: String,
    key_blob: String,
    confirm: String,
    report_path: String,
}

impl SignRequest {
    fn sign_args(&self) -> Vec<String> {
        vec![
            "sign".to_string(),
            "--drive".to_string(),
            self.drive.clone(),
            "--bundle-relative-path".to_string(),
            self.relative.clone(),
            "--key-id".to_string(),
            self.key_id.clone(),
            "--key-blob".to_string(),
            self.key_blob.clone(),
            "--confirm".to_string(),
            self.confirm.clone(),
            "--report".to_string(),
            self.report_path.clone(),
            "--skip-authenticode".to_string(),
        ]
    }

    fn verify_args(&self) -> Vec<String> {
        let verify_report = PathBuf::from(&self.report_path).with_file_name(
            PathBuf::from(&self.report_path)
                .file_name()
                .map(|name| format!("verify-{}", name.to_string_lossy()))
                .unwrap_or_else(|| "verify.json".to_string()),
        );
        vec![
            "verify".to_string(),
            "--drive".to_string(),
            self.drive.clone(),
            "--bundle-relative-path".to_string(),
            self.relative.clone(),
            "--key-id".to_string(),
            self.key_id.clone(),
            "--key-blob".to_string(),
            self.key_blob.clone(),
            "--report".to_string(),
            verify_report.display().to_string(),
            "--skip-authenticode".to_string(),
        ]
    }
}

fn begin_operation(status: &str) -> Option<(u64, HwndValue)> {
    let result = {
        let lock = STATE.get()?;
        let mut state = lock.lock().ok()?;
        begin_operation_locked(&mut state, status)
    };
    refresh_controls();
    result
}

fn begin_operation_locked(state: &mut GuiState, status: &str) -> Option<(u64, HwndValue)> {
    if state.busy || state.window == 0 {
        return None;
    }
    state.operation_id = state.operation_id.wrapping_add(1);
    state.busy = true;
    state.pending.clear();
    state.status_text = status.to_string();
    update_controls_locked(state);
    Some((state.operation_id, state.window))
}

fn publish(token: u64, event: WorkerEvent, window: HwndValue) {
    let Some(lock) = STATE.get() else { return };
    let Ok(mut state) = lock.lock() else { return };
    if state.operation_id != token || state.window != window {
        return;
    }
    let event_name = worker_event_name(&event);
    state.pending.push_back(event);
    startup_log(&format!(
        "publish: token={} window={} event={} queued={}",
        token,
        window,
        event_name,
        state.pending.len()
    ));
    let hwnd = to_hwnd(window);
    drop(state);
    unsafe {
        let posted = PostMessageW(hwnd, WM_APP_WORK_COMPLETE, 0, 0);
        startup_log(&format!("publish: post-work-complete={posted}"));
    }
}

unsafe fn handle_worker_event(_hwnd: HWND) {
    let (event, window, inspect_index, inspect_after_sign) = {
        let Some(lock) = STATE.get() else { return };
        let Ok(mut state) = lock.lock() else { return };
        let Some(event) = state.pending.pop_front() else {
            startup_log("handle-worker-event: no-pending-event");
            return;
        };
        startup_log(&format!(
            "handle-worker-event: event={} pending-after={} busy-before={} candidates-before={} selected-before={:?}",
            worker_event_name(&event),
            state.pending.len(),
            state.busy,
            state.candidates.len(),
            state.selected
        ));
        let mut inspect_index = None;
        let mut inspect_after_sign = false;
        match &event {
            WorkerEvent::Scan(result) => {
                state.busy = false;
                state.inspection = None;
                state.key_status = None;
                state.selected = None;
                state.sign_blocked = false;
                state.last_result = None;
                match result {
                    Ok(candidates) => {
                        state.candidates = candidates.clone();
                        if candidates.len() == 1 {
                            state.selected = Some(0);
                            state.status_text = "已发现 1 个候选，正在检查…".to_string();
                            inspect_index = Some(0);
                        } else if candidates.is_empty() {
                            state.candidates.clear();
                            state.status_text = "未发现可签名的可移动 NTFS Bundle。".to_string();
                        } else {
                            state.status_text = format!(
                                "发现 {} 个候选；必须只保留一个候选后才能签名。",
                                candidates.len()
                            );
                        }
                    }
                    Err(error) => {
                        state.candidates.clear();
                        state.status_text = format!("扫描失败：{error}");
                    }
                }
            }
            WorkerEvent::Inspect { index, result } => {
                state.busy = false;
                state.inspection = Some(result.clone());
                state.key_status = result.value.as_ref().and_then(parse_key_status);
                match &result.error {
                    Some(error) => {
                        state.status_text = format!("目标不可签名：{error}");
                    }
                    None => {
                        state.status_text = if state
                            .key_status
                            .as_ref()
                            .map(|status| status.matched)
                            .unwrap_or(false)
                        {
                            "检查完成；已自动准备签名环境，可以一键签名。".to_string()
                        } else {
                            "检查完成；签名环境或安装包绑定不满足，已停止。".to_string()
                        };
                    }
                }
                if state.selected != Some(*index) {
                    state.status_text = "目标已变化，请重新扫描。".to_string();
                    state.sign_blocked = true;
                }
            }
            WorkerEvent::Sign(outcome) => {
                state.busy = false;
                state.sign_blocked = true;
                match (&outcome.sign, &outcome.verify) {
                    (Ok(sign), Some(Ok(verify))) => {
                        let installation = json_string(sign, &["installationId"]);
                        let digest = json_string(sign, &["licenseDigest"]);
                        state.status_text = "签名完成，自动复核通过。".to_string();
                        state.last_result = Some(format!(
                            "签名成功\ninstallationId: {installation}\nlicenseDigest: {digest}\n报告：{}\n复核：verified=true",
                            outcome.report_path
                        ));
                        let _ = verify;
                        inspect_after_sign = true;
                    }
                    (Ok(_), Some(Err(error))) => {
                        state.status_text =
                            "已写入签名，但自动复核失败；请勿重试，先重新扫描并人工复核。"
                                .to_string();
                        state.last_result = Some(format!(
                            "签名结果需要人工复核\n报告：{}\n复核错误：{error}",
                            outcome.report_path
                        ));
                    }
                    (Ok(_), None) => {
                        state.status_text = "签名完成；请重新扫描确认授权状态。".to_string();
                        state.last_result = Some(format!(
                            "签名已完成\n报告：{}\n建议重新扫描后确认。",
                            outcome.report_path
                        ));
                        inspect_after_sign = true;
                    }
                    (Err(error), _) => {
                        state.status_text =
                            "签名未确认成功；请重新扫描，不要直接重复写入。".to_string();
                        state.last_result = Some(format!(
                            "签名失败或结果未知\n错误：{error}\n报告位置：{}",
                            outcome.report_path
                        ));
                    }
                }
            }
        }
        let window_value = state.window;
        startup_log(&format!(
            "handle-worker-event: state-after event={} busy={} candidates={} selected={:?} inspection={} keyMatched={:?} canSign={} status={}",
            worker_event_name(&event),
            state.busy,
            state.candidates.len(),
            state.selected,
            state.inspection.is_some(),
            state.key_status.as_ref().map(|item| item.matched),
            can_sign_locked(&state),
            state.status_text
        ));
        (event, window_value, inspect_index, inspect_after_sign)
    };

    let _ = event;
    if let Some(index) = inspect_index {
        start_inspect(index);
    } else if inspect_after_sign {
        if let Some(index) = STATE
            .get()
            .and_then(|lock| lock.lock().ok().and_then(|state| state.selected))
        {
            start_inspect(index);
        }
    }
    // All state mutations above happened after taking and releasing the GUI
    // mutex.  Apply the final snapshot directly now; relying only on a
    // queued WM_APP refresh here can leave the edit control and sign button
    // displaying the previous inspection result.
    refresh_controls();
    let _ = window;
}

unsafe fn handle_command(hwnd: HWND, wparam: WPARAM) {
    let id = (wparam & 0xffff) as i32;
    let notify = ((wparam >> 16) & 0xffff) as u32;
    match (id, notify) {
        (ID_REFRESH, windows_sys::Win32::UI::WindowsAndMessaging::BN_CLICKED) => {
            start_scan(hwnd);
        }
        (ID_SIGN, windows_sys::Win32::UI::WindowsAndMessaging::BN_CLICKED) => {
            prepare_sign_from_ui(hwnd);
        }
        _ => {}
    }
}

fn prepare_sign_from_ui(_hwnd: HWND) {
    let request = {
        let Some(lock) = STATE.get() else { return };
        let Ok(mut state) = lock.lock() else { return };
        if state.busy {
            return;
        }
        let Some(candidate) = state
            .candidates
            .first()
            .filter(|_| state.candidates.len() == 1)
            .cloned()
        else {
            state.status_text = "必须只保留一个可移动 NTFS Bundle。".to_string();
            update_controls_locked(&mut state);
            return;
        };
        let Some(inspection) = state.inspection.as_ref() else {
            state.status_text = "目标尚未完成深度检查。".to_string();
            update_controls_locked(&mut state);
            return;
        };
        let Some(value) = inspection.value.as_ref() else {
            state.status_text = "目标深度检查失败，不能签名。".to_string();
            update_controls_locked(&mut state);
            return;
        };
        if json_bool(value, &["deepVerify", "ok"]) != Some(true) {
            state.status_text = "Bundle 完整性检查未通过，不能签名。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        let license_state = json_string(value, &["license", "state"]);
        if license_state.eq_ignore_ascii_case("signed") {
            state.status_text = "该 Bundle 已经签名，工具不会覆盖已有签名。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        if json_bool(value, &["signing", "ok"]) != Some(true) {
            state.status_text = json_string(value, &["signing", "error"]);
            update_controls_locked(&mut state);
            return;
        }
        if json_bool(value, &["signingReady", "ok"]) != Some(true) {
            state.status_text = json_string(value, &["signingReady", "error"]);
            update_controls_locked(&mut state);
            return;
        }
        if state.sign_blocked {
            state.status_text = "本次签名尝试已结束，请重新扫描后再操作。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        let resolved = match usb_license_signer::key_blob::auto_resolve_key() {
            Ok(value) => value,
            Err(error) => {
                state.status_text = format!("签名环境准备失败：{error}");
                state.key_status = Some(AutoKeyStatus {
                    matched: false,
                    generated: false,
                    error: Some(error.to_string()),
                });
                update_controls_locked(&mut state);
                return;
            }
        };
        let key_id = resolved.key.key_id.clone();
        let key_blob = resolved.path.display().to_string();
        let expected_key_id = json_string(value, &["signing", "keyId"]);
        let expected_digest = json_string(value, &["signing", "publicKeySha256"]);
        let resolved_digest = resolved.key.public_key_sha256();
        if key_id != expected_key_id || !resolved_digest.eq_ignore_ascii_case(&expected_digest) {
            state.status_text = format!(
                "签名环境与当前安装包不匹配，未写入 U 盘；本机 {} / {}，安装包要求 {} / {}。这表示发行公钥不是同一把，不是 U 盘损坏；请使用同一发行配置重新打包，并在匹配的签名电脑上操作。",
                key_id, resolved_digest, expected_key_id, expected_digest
            );
            update_controls_locked(&mut state);
            return;
        }
        let confirm = format!(
            "SIGN {}-{}",
            candidate.volume_serial,
            candidate.anchor_sha256.chars().take(12).collect::<String>()
        );
        let report = report_path(candidate.volume_serial);
        Some(SignRequest {
            drive: candidate.drive_root.clone(),
            relative: candidate.bundle_relative_path.clone(),
            key_id,
            key_blob,
            confirm,
            report_path: report,
        })
    };

    let Some(request) = request else {
        return;
    };
    start_sign(request);
}

fn scan_candidates() -> Result<Vec<GuiCandidate>, String> {
    let value = run_command(&["scan".to_string()])?;
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| "scan 输出缺少 candidates".to_string())?;
    candidates.iter().map(parse_candidate).collect()
}

fn parse_candidate(value: &Value) -> Result<GuiCandidate, String> {
    Ok(GuiCandidate {
        drive_root: required_string(value, "driveRoot")?,
        volume_serial: required_u32(value, "volumeSerial")?,
        file_system: required_string(value, "fileSystem")?,
        bundle_relative_path: required_string(value, "bundleRelativePath")?,
        bundle_path: required_string(value, "bundlePath")?,
        anchor_schema: required_u32(value, "anchorSchema")?,
        anchor_sha256: required_string(value, "anchorSha256")?,
    })
}

fn inspect_candidate(candidate: &GuiCandidate) -> Result<Value, String> {
    let mut value = run_command(&[
        "inspect".to_string(),
        "--drive".to_string(),
        candidate.drive_root.clone(),
        "--bundle-relative-path".to_string(),
        candidate.bundle_relative_path.clone(),
        "--skip-authenticode".to_string(),
    ])?;

    let ready = if json_bool(&value, &["deepVerify", "ok"]) != Some(true) {
        json!({
            "ok": false,
            "error": "Bundle 深度检查未通过，未初始化签名环境"
        })
    } else if json_bool(&value, &["signing", "ok"]) != Some(true) {
        json!({
            "ok": false,
            "error": json_string(&value, &["signing", "error"])
        })
    } else {
        match usb_license_signer::key_blob::auto_resolve_key() {
            Ok(resolved) => {
                let expected_key_id = json_string(&value, &["signing", "keyId"]);
                let expected_digest = json_string(&value, &["signing", "publicKeySha256"]);
                let resolved_digest = resolved.key.public_key_sha256();
                let matched = resolved.key.key_id == expected_key_id
                    && resolved_digest.eq_ignore_ascii_case(&expected_digest);
                if matched {
                    json!({
                        "ok": true,
                        "generated": resolved.generated,
                        "keyId": resolved.key.key_id,
                        "publicKeySha256": resolved_digest
                    })
                } else {
                    json!({
                        "ok": false,
                        "generated": resolved.generated,
                        "keyId": resolved.key.key_id,
                        "publicKeySha256": resolved_digest,
                        "expectedKeyId": expected_key_id,
                        "expectedPublicKeySha256": expected_digest,
                        "keyBlobPath": resolved.path.display().to_string(),
                        "error": format!(
                            "签名环境与当前安装包不匹配：本机 keyId={} / 公钥摘要={}；安装包要求 keyId={} / 公钥摘要={}；这表示发行公钥不是同一把，不是 U 盘损坏；请使用同一发行配置重新打包，并在匹配的签名电脑上操作；密钥文件={}",
                            resolved.key.key_id,
                            resolved_digest,
                            expected_key_id,
                            expected_digest,
                            resolved.path.display()
                        )
                    })
                }
            }
            Err(error) => json!({
                "ok": false,
                "error": format!("签名环境准备失败：{error}")
            }),
        }
    };
    startup_log(&format!(
        "inspect-candidate: deepVerify={} signing={} signingReady={}",
        json_bool(&value, &["deepVerify", "ok"])
            .map(|item| item.to_string())
            .unwrap_or_else(|| "missing".to_string()),
        json_bool(&value, &["signing", "ok"])
            .map(|item| item.to_string())
            .unwrap_or_else(|| "missing".to_string()),
        serde_json::to_string(&ready).unwrap_or_else(|_| "<serialize-error>".to_string())
    ));
    if let Some(object) = value.as_object_mut() {
        object.insert("signingReady".to_string(), ready);
    }
    Ok(value)
}

fn run_command(parts: &[String]) -> Result<Value, String> {
    let args = parts.iter().map(OsString::from).collect::<Vec<_>>();
    run_with_args(args).map_err(|error| error.to_string())
}

fn startup_log(event: &str) {
    let path = env::temp_dir().join("CodexUsbLicenseSigner-startup.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let _ = writeln!(file, "{timestamp} {event}");
}

fn report_path(volume_serial: u32) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    local_app_data()
        .join("CodexUsbLicenseSigner")
        .join("reports")
        .join(format!("sign-{stamp}-{volume_serial}.json"))
        .display()
        .to_string()
}

fn can_sign_locked(state: &GuiState) -> bool {
    if state.busy || state.sign_blocked || state.candidates.len() != 1 || state.selected != Some(0)
    {
        return false;
    }
    let Some(inspection) = state.inspection.as_ref() else {
        return false;
    };
    let Some(value) = inspection.value.as_ref() else {
        return false;
    };
    if json_bool(value, &["deepVerify", "ok"]) != Some(true) {
        return false;
    }
    let license_state = json_string(value, &["license", "state"]);
    if license_state.eq_ignore_ascii_case("signed") {
        return false;
    }
    json_bool(value, &["signing", "ok"]) == Some(true)
        && json_bool(value, &["signingReady", "ok"]) == Some(true)
}

fn update_controls_locked(state: &mut GuiState) {
    // Win32 controls can synchronously send WM_COMMAND/WM_SIZE back to the
    // parent.  Only queue the refresh here; never SendMessage while STATE is
    // locked, or the window thread can deadlock during control creation.
    if state.window != 0 {
        unsafe {
            let posted = PostMessageW(to_hwnd(state.window), WM_APP_REFRESH_CONTROLS, 0, 0);
            startup_log(&format!(
                "update-controls-locked: window={} posted={posted}",
                state.window
            ));
        }
    }
}

struct ControlsSnapshot {
    targets: HwndValue,
    refresh: HwndValue,
    sign: HwndValue,
    summary: HwndValue,
    status: HwndValue,
    candidate_texts: Vec<String>,
    selected: Option<usize>,
    summary_text: String,
    status_text: String,
    busy: bool,
    can_sign: bool,
}

fn refresh_controls() {
    let snapshot = STATE.get().and_then(|lock| {
        lock.lock().ok().map(|state| ControlsSnapshot {
            targets: state.targets,
            refresh: state.refresh,
            sign: state.sign,
            summary: state.summary,
            status: state.status,
            candidate_texts: state.candidates.iter().map(candidate_display).collect(),
            selected: state.selected,
            summary_text: summary_text(&state),
            status_text: state.status_text.clone(),
            busy: state.busy,
            can_sign: can_sign_locked(&state),
        })
    });
    if let Some(snapshot) = snapshot {
        startup_log(&format!(
            "refresh-controls: targets={} summary={} status={} sign={} busy={} canSign={} candidates={} summaryLen={} statusLen={}",
            snapshot.targets,
            snapshot.summary,
            snapshot.status,
            snapshot.sign,
            snapshot.busy,
            snapshot.can_sign,
            snapshot.candidate_texts.len(),
            snapshot.summary_text.len(),
            snapshot.status_text.len()
        ));
        unsafe { apply_controls(snapshot) };
    } else {
        startup_log("refresh-controls: no-snapshot");
    }
}

unsafe fn apply_controls(snapshot: ControlsSnapshot) {
    if snapshot.targets != 0 {
        let combo = to_hwnd(snapshot.targets);
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        for text in snapshot.candidate_texts {
            let text_wide = wide(&text);
            SendMessageW(combo, CB_ADDSTRING, 0, text_wide.as_ptr() as LPARAM);
        }
        if let Some(index) = snapshot.selected {
            SendMessageW(combo, CB_SETCURSEL, index, 0);
        }
    }
    if snapshot.summary != 0 {
        let result = set_text(snapshot.summary, &snapshot.summary_text);
        startup_log(&format!("apply-controls: summary-set={result}"));
    }
    if snapshot.status != 0 {
        let result = set_text(snapshot.status, &snapshot.status_text);
        startup_log(&format!("apply-controls: status-set={result}"));
    }
    let enabled = (!snapshot.busy) as i32;
    if snapshot.refresh != 0 {
        EnableWindow(to_hwnd(snapshot.refresh), enabled);
    }
    if snapshot.targets != 0 {
        // Target selection is intentionally not user-editable.  The signer
        // accepts only the sole candidate returned by the removable-volume
        // scanner; this keeps the one-click flow from turning into a path
        // picker or a way to bypass the multi-candidate gate.
        EnableWindow(to_hwnd(snapshot.targets), 0);
    }
    if snapshot.sign != 0 {
        let result = EnableWindow(to_hwnd(snapshot.sign), snapshot.can_sign as i32);
        startup_log(&format!(
            "apply-controls: sign-enabled={} result={result}",
            snapshot.can_sign
        ));
    }
}

fn worker_event_name(event: &WorkerEvent) -> &'static str {
    match event {
        WorkerEvent::Scan(_) => "scan",
        WorkerEvent::Inspect { .. } => "inspect",
        WorkerEvent::Sign(_) => "sign",
    }
}

fn summary_text(state: &GuiState) -> String {
    let mut lines = Vec::new();
    if state.candidates.is_empty() {
        lines.push("未发现候选 Bundle。请插入包含完整安装包的可移动 NTFS U 盘。".to_string());
    } else if state.candidates.len() > 1 {
        lines.push(format!(
            "发现 {} 个候选 Bundle。为防止误签，必须只保留一个候选。",
            state.candidates.len()
        ));
        for candidate in &state.candidates {
            lines.push(format!("- {}", candidate_display(candidate)));
        }
    } else if let Some(candidate) = state.candidates.first() {
        lines.push(format!("目标：{}", candidate.bundle_path));
        lines.push(format!(
            "盘符 / 卷序列号：{} / {}",
            candidate.drive_root, candidate.volume_serial
        ));
        lines.push(format!("文件系统：{}", candidate.file_system));
        lines.push(format!(
            "Bundle 相对路径：{}",
            candidate.bundle_relative_path
        ));
        lines.push(format!(
            "Anchor schema / SHA256：{} / {}",
            candidate.anchor_schema, candidate.anchor_sha256
        ));
        match state.inspection.as_ref() {
            None => lines.push("深度检查：进行中或尚未开始".to_string()),
            Some(view) => match (&view.value, &view.error) {
                (Some(value), _) => {
                    let authenticode_mode = json_string(value, &["deepVerify", "authenticodeMode"]);
                    lines.push(format!(
                        "系统签名检查：{}（内部授权模式）",
                        if authenticode_mode.eq_ignore_ascii_case("skipped") {
                            "跳过"
                        } else {
                            "严格检查"
                        }
                    ));
                    lines.push(format!(
                        "License 状态：{}",
                        json_string(value, &["license", "state"])
                    ));
                    lines.push(format!(
                        "installationId：{}",
                        json_string(value, &["license", "installationId"])
                    ));
                    lines.push(format!(
                        "Payload SHA256：{}",
                        json_string(value, &["deepVerify", "summary", "payloadSha256"])
                    ));
                    lines.push(format!(
                        "Payload 大小：{} bytes",
                        json_u64(value, &["deepVerify", "summary", "payloadSize"])
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    ));
                    lines.push(format!(
                        "Bundle 深度检查：{}",
                        if json_bool(value, &["deepVerify", "ok"]) == Some(true) {
                            "通过"
                        } else {
                            "失败"
                        }
                    ));
                }
                (None, Some(error)) => lines.push(format!("Bundle 深度检查：失败\n{}", error)),
                (None, None) => lines.push("Bundle 深度检查：未知".to_string()),
            },
        }
    }
    match &state.key_status {
        Some(status) if status.matched => {
            lines.push(if status.generated {
                "签名环境：已自动初始化并匹配当前安装包".to_string()
            } else {
                "签名环境：已自动找到并匹配当前安装包".to_string()
            });
        }
        Some(status) => {
            lines.push("签名环境：未匹配，已停止".to_string());
            if let Some(error) = &status.error {
                lines.push(error.clone());
            }
        }
        None => lines.push("签名环境：等待自动检查".to_string()),
    }
    if let Some(result) = &state.last_result {
        lines.push(String::new());
        lines.push("最近一次操作：".to_string());
        lines.push(result.clone());
    }
    lines.join("\r\n")
}

fn candidate_display(candidate: &GuiCandidate) -> String {
    let anchor = candidate.anchor_sha256.chars().take(12).collect::<String>();
    format!(
        "{}  {}  serial={}  anchor={}…",
        candidate.drive_root, candidate.bundle_relative_path, candidate.volume_serial, anchor
    )
}

fn local_app_data() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

unsafe fn resize_summary(_hwnd: HWND) {
    // The first version uses a fixed, DPI-friendly layout.  Keeping this hook
    // explicit prevents accidental reparenting or path changes during resize.
}

fn required_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("字段 {key} 缺失或类型错误"))
}

fn required_u32(value: &Value, key: &str) -> Result<u32, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| format!("字段 {key} 缺失或类型错误"))
}

fn parse_key_status(value: &Value) -> Option<AutoKeyStatus> {
    let ready = value.get("signingReady")?.as_object()?;
    let matched = ready.get("ok").and_then(Value::as_bool)?;
    Some(AutoKeyStatus {
        matched,
        generated: ready
            .get("generated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        error: ready
            .get("error")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn json_string(value: &Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn json_u64(value: &Value, path: &[&str]) -> Option<u64> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_u64)
}

fn json_bool(value: &Value, path: &[&str]) -> Option<bool> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_bool)
}

fn hwnd_value(hwnd: HWND) -> HwndValue {
    hwnd as HwndValue
}

fn to_hwnd(hwnd: HwndValue) -> HWND {
    hwnd as HWND
}

fn id_to_menu(id: i32) -> HMENU {
    if id == 0 {
        null_mut()
    } else {
        id as isize as HMENU
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn set_text(hwnd: HwndValue, value: &str) -> LRESULT {
    if hwnd == 0 {
        return 0;
    }
    let text = wide(value);
    SendMessageW(
        to_hwnd(hwnd),
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SETTEXT,
        0,
        text.as_ptr() as LPARAM,
    )
}

unsafe fn show_message(owner: HWND, message: &str, title: &str, style: u32) -> i32 {
    let message_wide = wide(message);
    let title_wide = wide(title);
    MessageBoxW(owner, message_wide.as_ptr(), title_wide.as_ptr(), style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_value() -> Value {
        serde_json::json!({
            "driveRoot": "E:\\",
            "volumeSerial": 123,
            "fileSystem": "NTFS",
            "bundleRelativePath": "CodexUsb",
            "bundlePath": "E:\\CodexUsb",
            "anchorSchema": 2,
            "anchorSha256": "a".repeat(64)
        })
    }

    #[test]
    fn candidate_parser_requires_the_scan_contract() {
        let candidate = parse_candidate(&candidate_value()).expect("scan candidate should parse");
        assert_eq!(candidate.drive_root, "E:\\");
        assert_eq!(candidate.volume_serial, 123);
        assert_eq!(candidate.bundle_relative_path, "CodexUsb");

        let mut missing = candidate_value();
        missing.as_object_mut().unwrap().remove("anchorSha256");
        assert!(parse_candidate(&missing).is_err());
    }

    #[test]
    fn automatic_key_status_is_read_from_inspection_contract() {
        let value = serde_json::json!({
            "signingReady": {
                "ok": true,
                "generated": true,
                "keyId": "prod-2026-01"
            }
        });
        let status = parse_key_status(&value).expect("automatic key status should parse");
        assert!(status.matched);
        assert!(status.generated);
        assert!(status.error.is_none());
    }

    #[test]
    fn wide_text_is_nul_terminated() {
        let text = wide("签名授权");
        assert_eq!(text.last(), Some(&0));
        assert!(text.len() > 1);
    }
}
