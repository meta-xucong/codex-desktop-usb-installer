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
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use usb_license_signer::{
    run_with_args, run_with_portable_password, run_with_portable_password_and_account,
    sub2api::{self, Sub2ApiClient, Sub2ApiConfig},
    windows::acquire_gui_instance_mutex,
};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::UpdateWindow;
use windows_sys::Win32::Graphics::Gdi::{
    GetStockObject, GetSysColorBrush, COLOR_WINDOW, DEFAULT_GUI_FONT,
};
use windows_sys::Win32::Security::Credentials::{
    CredUIPromptForCredentialsW, CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
    CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_FLAGS_PASSWORD_ONLY_OK, CREDUI_INFOW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, LoadCursorW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, SendMessageW, SetForegroundWindow, ShowWindow, TranslateMessage,
    CBS_DROPDOWNLIST, CBS_HASSTRINGS, CB_ADDSTRING, CB_RESETCONTENT, CB_SETCURSEL, CW_USEDEFAULT,
    ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, ES_PASSWORD, ES_READONLY, HMENU, IDC_ARROW,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_DESTROY, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_CLIENTEDGE, WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use zeroize::Zeroizing;

const ID_TARGETS: i32 = 1001;
const ID_REFRESH: i32 = 1002;
const ID_SUMMARY: i32 = 1007;
const ID_STATUS: i32 = 1008;
const ID_PROVISION_SIGN: i32 = 1009;
const ID_BASE_URL: i32 = 1010;
const ID_ADMIN_KEY: i32 = 1011;
const ID_EMAIL_DOMAIN: i32 = 1013;
const ID_PRELOAD: i32 = 1014;
const ID_RETRY_PRELOAD: i32 = 1015;
const ID_ACCOUNT_PASSWORD: i32 = 1016;

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
    mode: String,
    error: Option<String>,
}

#[derive(Debug)]
struct ProvisionSignOutcome {
    account: Result<sub2api::ProvisionedAccount, String>,
    sign: Option<Result<Value, String>>,
    verify: Option<Result<Value, String>>,
    preload: Option<Result<(), String>>,
    preload_applied: bool,
    recovery_cleared: bool,
    recovery_available: bool,
    report_path: String,
}

struct RetryPreloadRequest {
    config: Sub2ApiConfig,
    binding: codex_usb_license::UsbAccountBinding,
    license_digest: String,
}

#[derive(Debug)]
enum WorkerEvent {
    Scan(Result<Vec<GuiCandidate>, String>),
    Inspect { index: usize, result: InspectView },
    ProvisionSign(Box<ProvisionSignOutcome>),
    RetryPreload(Result<sub2api::RetryPreloadResult, String>),
}

#[derive(Debug)]
struct GuiState {
    window: HwndValue,
    targets: HwndValue,
    refresh: HwndValue,
    provision_sign: HwndValue,
    base_url: HwndValue,
    admin_key: HwndValue,
    email_domain: HwndValue,
    preload: HwndValue,
    account_password: HwndValue,
    retry_preload: HwndValue,
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
            provision_sign: 0,
            base_url: 0,
            admin_key: 0,
            email_domain: 0,
            preload: 0,
            account_password: 0,
            retry_preload: 0,
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

/// Run the exact key-resolution path used by the desktop GUI without opening
/// a window. Release tooling uses this diagnostic to validate the published
/// GUI binary instead of treating a separately launched CLI as proof that
/// the desktop signer can sign.
pub fn write_key_diagnostic(output: &std::path::Path) -> Result<(), String> {
    let value = run_with_args([OsString::from("init-key")]).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    fs::write(output, bytes).map_err(|error| error.to_string())
}

unsafe fn run_windows() {
    startup_log("run_windows:start");
    let Some(_gui_instance_mutex) = (match acquire_gui_instance_mutex() {
        Ok(value) => value,
        Err(error) => {
            startup_log(&format!("run_windows:gui-instance-mutex:failed: {error}"));
            show_message(
                null_mut(),
                "无法锁定签名工具窗口，未开始扫描。",
                "Codex U 盘签名授权",
                MB_ICONERROR | MB_OK,
            );
            return;
        }
    }) else {
        startup_log("run_windows:gui-instance-mutex:already-running");
        show_message(
            null_mut(),
            "签名工具已经打开。请使用已有窗口；如果已有窗口显示旧状态，请先关闭旧窗口，再从最新发布目录重新打开。",
            "Codex U 盘签名授权",
            MB_ICONINFORMATION | MB_OK,
        );
        return;
    };
    startup_log("run_windows:gui-instance-mutex:acquired");

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
    let existing_window = FindWindowW(class_name.as_ptr(), null());
    if !existing_window.is_null() {
        startup_log("run_windows:existing-window-found");
        ShowWindow(
            existing_window,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );
        SetForegroundWindow(existing_window);
        show_message(
            null_mut(),
            "签名工具已经打开。请使用已有窗口；如果已有窗口显示旧状态，请先关闭旧窗口，再从最新发布目录重新打开。",
            "Codex U 盘签名授权",
            MB_ICONINFORMATION | MB_OK,
        );
        return;
    }

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
        760,
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
    let config = sub2api::load_config().unwrap_or_default();
    let status_text = {
        let Some(lock) = STATE.get() else { return };
        let Ok(mut state) = lock.lock() else { return };
        state.window = hwnd_value(hwnd);
        if config.admin_api_key.trim().is_empty() {
            state.status_text = format!(
                "未找到内置管理员 Key；请由受信任部署设置 {}。",
                sub2api::ADMIN_API_KEY_ENV
            );
        }
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
        "404token 注册（管理员 Key 自动加载；目标分组固定为统一 API）",
        20,
        298,
        740,
        22,
        0,
    );
    create_label(hwnd, instance, "API 地址", 20, 330, 90, 22, 0);
    let base_url = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        &config.base_url,
        WS_TABSTOP | ES_LEFT as u32 | WS_BORDER,
        WS_EX_CLIENTEDGE,
        105,
        326,
        270,
        26,
        ID_BASE_URL,
    ));
    create_label(
        hwnd,
        instance,
        "管理员 Key（自动加载）",
        395,
        330,
        95,
        22,
        0,
    );
    let admin_key = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        &config.admin_api_key,
        WS_TABSTOP | ES_LEFT as u32 | ES_PASSWORD as u32 | ES_READONLY as u32 | WS_BORDER,
        WS_EX_CLIENTEDGE,
        490,
        326,
        300,
        26,
        ID_ADMIN_KEY,
    ));
    create_label(hwnd, instance, "目标分组", 20, 365, 80, 22, 0);
    create_label(
        hwnd,
        instance,
        sub2api::TARGET_GROUP_NAME,
        105,
        365,
        205,
        22,
        0,
    );
    create_label(hwnd, instance, "模拟邮箱域名", 330, 365, 100, 22, 0);
    let email_domain = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        &config.fake_email_domain,
        WS_TABSTOP | ES_LEFT as u32 | WS_BORDER,
        WS_EX_CLIENTEDGE,
        435,
        361,
        220,
        26,
        ID_EMAIL_DOMAIN,
    ));
    create_label(hwnd, instance, "初始密码（固定）", 20, 400, 120, 22, 0);
    let account_password = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        sub2api::DEFAULT_INITIAL_PASSWORD,
        WS_TABSTOP | ES_LEFT as u32 | ES_READONLY as u32 | WS_BORDER,
        WS_EX_CLIENTEDGE,
        145,
        396,
        220,
        26,
        ID_ACCOUNT_PASSWORD,
    ));
    create_label(hwnd, instance, "预充值", 350, 400, 60, 22, 0);
    let preload = hwnd_value(create_control(
        hwnd,
        instance,
        "EDIT",
        "0",
        WS_TABSTOP | ES_LEFT as u32 | WS_BORDER,
        WS_EX_CLIENTEDGE,
        415,
        396,
        175,
        26,
        ID_PRELOAD,
    ));

    create_label(
        hwnd,
        instance,
        "签名前检查摘要（密钥自动管理）",
        20,
        430,
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
        456,
        770,
        145,
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
        610,
        770,
        30,
        ID_STATUS,
    ));
    let provision_sign = create_button(
        hwnd,
        instance,
        "注册账号并签名",
        20,
        650,
        510,
        38,
        ID_PROVISION_SIGN,
    );
    let retry_preload = create_button(
        hwnd,
        instance,
        "重试预充值",
        540,
        650,
        230,
        38,
        ID_RETRY_PRELOAD,
    );
    let Some(lock) = STATE.get() else { return };
    let Ok(mut state) = lock.lock() else { return };
    state.targets = targets;
    state.refresh = refresh;
    state.summary = summary;
    state.status = status;
    state.provision_sign = provision_sign;
    state.base_url = base_url;
    state.admin_key = admin_key;
    state.email_domain = email_domain;
    state.preload = preload;
    state.account_password = account_password;
    state.retry_preload = retry_preload;
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

fn start_provision_sign(request: ProvisionSignRequest) {
    let Some((token, window)) = begin_operation(
        "正在注册账号（U盘编号将在注册响应后显示）、签名并回读验证，请勿拔出 U 盘…",
    ) else {
        return;
    };
    let report_path = request.sign.report_path.clone();
    thread::spawn(move || {
        let result = (|| {
            let client =
                Sub2ApiClient::new(request.config.clone()).map_err(|error| error.to_string())?;
            // Validate the encrypted portable vault before any remote account
            // mutation. A wrong password or a damaged signer directory must
            // never create an orphan account.
            run_with_portable_password(
                request
                    .sign
                    .portable_inspect_args()
                    .into_iter()
                    .map(OsString::from),
                request.sign.password.as_str(),
            )
            .map_err(|error| format!("签名库预检失败：{error}"))?;
            sub2api::save_config(&request.config).map_err(|error| error.to_string())?;
            let account = client
                .provision_account_with_recovery(
                    request.existing_binding.as_ref(),
                    None,
                    Some(&request.recovery_key),
                )
                .map_err(|error| error.to_string())?;
            let sign = run_with_portable_password_and_account(
                request.sign.sign_args().into_iter().map(OsString::from),
                request.sign.password.as_str(),
                account.binding.clone(),
            )
            .map_err(|error| error.to_string());
            let verify = if sign.is_ok() {
                Some(
                    run_command_with_password(
                        &request.sign.verify_args(),
                        Some(request.sign.password.as_str()),
                    )
                    .map_err(|error| error.to_string()),
                )
            } else {
                None
            };
            let mut preload_applied = false;
            let preload = if sign.is_ok() && verify.as_ref().is_some_and(Result::is_ok) {
                if sub2api::should_precharge(request.preload_amount, account.reused) {
                    let digest = sign
                        .as_ref()
                        .ok()
                        .and_then(|value| value.get("licenseDigest"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| "签名结果缺少许可证摘要，未执行预充值".to_string())?;
                    let result = client
                        .precharge_with_pending(
                            account.binding.sub2api_user_id,
                            request.preload_amount,
                            digest,
                            &account.binding.usb_asset_id,
                        )
                        .map(|_| ())
                        .map_err(|error| error.to_string());
                    if result.is_ok() {
                        preload_applied = true;
                    }
                    Some(result)
                } else {
                    Some(Ok(()))
                }
            } else {
                None
            };
            let recovery_cleared = if sign.is_ok() && verify.as_ref().is_some_and(Result::is_ok) {
                client
                    .clear_provision_recovery(&request.recovery_key)
                    .is_ok()
            } else {
                false
            };
            let recovery_available =
                sub2api::has_provision_recovery(&request.recovery_key).unwrap_or(false);
            Ok::<_, String>(ProvisionSignOutcome {
                account: Ok(account),
                sign: Some(sign),
                verify,
                preload,
                preload_applied,
                recovery_cleared,
                recovery_available,
                report_path: report_path.clone(),
            })
        })();
        let recovery_available =
            sub2api::has_provision_recovery(&request.recovery_key).unwrap_or(false);
        let outcome = match result {
            Ok(value) => value,
            Err(error) => ProvisionSignOutcome {
                account: Err(error),
                sign: None,
                verify: None,
                preload: None,
                preload_applied: false,
                recovery_cleared: false,
                recovery_available,
                report_path,
            },
        };
        publish(token, WorkerEvent::ProvisionSign(Box::new(outcome)), window);
    });
}

fn start_retry_preload(request: RetryPreloadRequest) {
    let Some((token, window)) = begin_operation("正在按原幂等记录重试预充值，请勿关闭工具…")
    else {
        return;
    };
    thread::spawn(move || {
        let result = (|| {
            let client = Sub2ApiClient::new(request.config).map_err(|error| error.to_string())?;
            client
                .retry_pending_precharge(&request.binding, &request.license_digest)
                .map_err(|error| error.to_string())
        })();
        publish(token, WorkerEvent::RetryPreload(result), window);
    });
}

struct SignRequest {
    drive: String,
    relative: String,
    portable_vault: String,
    password: Zeroizing<String>,
    confirm: String,
    report_path: String,
}

struct ProvisionSignRequest {
    sign: SignRequest,
    config: Sub2ApiConfig,
    preload_amount: f64,
    existing_binding: Option<codex_usb_license::UsbAccountBinding>,
    recovery_key: String,
}

impl SignRequest {
    fn sign_args(&self) -> Vec<String> {
        vec![
            "sign".to_string(),
            "--drive".to_string(),
            self.drive.clone(),
            "--bundle-relative-path".to_string(),
            self.relative.clone(),
            "--portable-vault".to_string(),
            self.portable_vault.clone(),
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
            "--portable-vault".to_string(),
            self.portable_vault.clone(),
            "--report".to_string(),
            verify_report.display().to_string(),
            "--skip-authenticode".to_string(),
        ]
    }

    fn portable_inspect_args(&self) -> Vec<String> {
        vec![
            "inspect-portable".to_string(),
            "--portable-vault".to_string(),
            self.portable_vault.clone(),
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
                let key_matched = state
                    .key_status
                    .as_ref()
                    .map(|status| status.matched)
                    .unwrap_or(false);
                state.status_text = inspection_status_text(result, key_matched);
                if state.selected != Some(*index) {
                    state.status_text = "目标已变化，请重新扫描。".to_string();
                    state.sign_blocked = true;
                }
            }
            WorkerEvent::ProvisionSign(outcome) => {
                state.busy = false;
                state.sign_blocked = true;
                match (
                    &outcome.account,
                    &outcome.sign,
                    &outcome.verify,
                    &outcome.preload,
                ) {
                    (Ok(account), Some(Ok(_sign)), Some(Ok(_verify)), Some(preload)) => {
                        match preload {
                            Err(error) => {
                                state.status_text = if outcome.recovery_cleared {
                                    "账号和签名已完成，但预充值未确认；请勿重复点击。"
                                } else {
                                    "账号和签名已完成，但预充值未确认；本机恢复记录清理失败，请保留此签名器处理。"
                                }
                                .to_string();
                                state.last_result = Some(format!(
                                    "U盘编号：{}\n账号：{}\n用户 ID：{}\n预充值错误：{}\n报告：{}",
                                    account.binding.usb_asset_id,
                                    account.binding.account_name,
                                    account.binding.sub2api_user_id,
                                    error,
                                    outcome.report_path
                                ));
                            }
                            Ok(()) => {
                                state.status_text = if outcome.recovery_cleared {
                                    "注册、签名和回读验证完成。"
                                } else {
                                    "注册、签名和回读验证完成，但本机恢复记录清理失败。"
                                }
                                .to_string();
                                let password_text = if account.initial_password.is_empty() {
                                    "（沿用已有绑定，未生成新密码）".to_string()
                                } else {
                                    format!(
                                        "\n初始密码（固定）：{}",
                                        account.initial_password.as_str()
                                    )
                                };
                                let preload_text = if outcome.preload_applied {
                                    "预充值已完成"
                                } else if account.reused {
                                    "已有绑定，本次未重复预充值"
                                } else {
                                    "本次未预充值"
                                };
                                state.last_result = Some(format!(
                                    "U盘编号：{}\n账号：{}\n用户 ID：{}\n{}{}\n报告：{}",
                                    account.binding.usb_asset_id,
                                    account.binding.account_name,
                                    account.binding.sub2api_user_id,
                                    preload_text,
                                    password_text,
                                    outcome.report_path
                                ));
                            }
                        }
                        inspect_after_sign = true;
                    }
                    (Ok(account), Some(Ok(_)), Some(Err(error)), _) => {
                        state.status_text = if outcome.recovery_available {
                            "账号已创建，自动复核失败；已保留加密恢复记录，请重新扫描后继续同一账号。"
                        } else {
                            "账号和签名已完成，但自动复核失败；预充值未执行。"
                        }
                        .to_string();
                        state.last_result = Some(format!(
                            "U盘编号：{}\n账号：{}\n用户 ID：{}\n复核错误：{}\n报告：{}",
                            account.binding.usb_asset_id,
                            account.binding.account_name,
                            account.binding.sub2api_user_id,
                            error,
                            outcome.report_path
                        ));
                    }
                    (Ok(account), Some(Err(error)), _, _) => {
                        state.status_text = if outcome.recovery_available {
                            "账号已创建，但签名未确认成功；已保留加密恢复记录，重新扫描后会继续同一账号。"
                        } else {
                            "账号已创建，但签名未确认成功；未执行预充值。"
                        }
                        .to_string();
                        state.last_result = Some(format!(
                            "U盘编号：{}\n账号：{}\n用户 ID：{}\n签名错误：{}\n报告：{}",
                            account.binding.usb_asset_id,
                            account.binding.account_name,
                            account.binding.sub2api_user_id,
                            error,
                            outcome.report_path
                        ));
                    }
                    (_, _, _, _) => {
                        state.status_text = if outcome.recovery_available {
                            "注册未完成；检测到加密恢复记录，重新扫描后会继续原账号。"
                        } else {
                            "注册流程失败，未报告成功。"
                        }
                        .to_string();
                        state.last_result = Some(format!(
                            "注册错误：{}\n报告：{}",
                            outcome
                                .account
                                .as_ref()
                                .err()
                                .map(String::as_str)
                                .unwrap_or("签名流程未完成"),
                            outcome.report_path
                        ));
                    }
                }
            }
            WorkerEvent::RetryPreload(result) => {
                state.busy = false;
                match result {
                    Ok(value) => {
                        state.status_text = "预充值重试完成，原待处理记录已清除。".to_string();
                        state.last_result = Some(format!(
                            "账号：{}\n用户 ID：{}\n预充值：{}\n幂等记录已完成",
                            value.pending.usb_asset_id,
                            value.pending.sub2api_user_id,
                            value.pending.amount
                        ));
                    }
                    Err(error) => {
                        state.status_text =
                            "预充值仍未完成；待处理记录已保留，可在确认后台状态后重试。"
                                .to_string();
                        state.last_result = Some(format!("预充值重试错误：{error}"));
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
            can_provision_sign_locked(&state),
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
        (ID_PROVISION_SIGN, windows_sys::Win32::UI::WindowsAndMessaging::BN_CLICKED) => {
            prepare_provision_sign_from_ui(hwnd);
        }
        (ID_RETRY_PRELOAD, windows_sys::Win32::UI::WindowsAndMessaging::BN_CLICKED) => {
            prepare_retry_preload_from_ui();
        }
        _ => {}
    }
}

fn prepare_provision_sign_from_ui(hwnd: HWND) {
    let preparation = {
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
            state.status_text = "目标深度检查失败，不能注册。".to_string();
            update_controls_locked(&mut state);
            return;
        };
        if json_bool(value, &["deepVerify", "ok"]) != Some(true) {
            state.status_text = "Bundle 完整性检查未通过，不能注册。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        if json_string(value, &["license", "state"]).eq_ignore_ascii_case("signed") {
            state.status_text = "该 Bundle 已经签名，不会覆盖已有签名。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        if json_bool(value, &["signing", "ok"]) != Some(true)
            || json_bool(value, &["signingReady", "ok"]) != Some(true)
        {
            state.status_text = "签名环境或安装包绑定不满足，已停止。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        if state.sign_blocked {
            state.status_text = "本次操作已结束，请重新扫描后再操作。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        let mut config = Sub2ApiConfig {
            base_url: window_text(state.base_url),
            admin_api_key: window_text(state.admin_key),
            fake_email_domain: window_text(state.email_domain),
        };
        config.base_url = config.base_url.trim().to_string();
        config.fake_email_domain = config.fake_email_domain.trim().to_string();
        if let Err(error) = sub2api::validate_config(&config) {
            state.status_text = error.to_string();
            update_controls_locked(&mut state);
            return;
        }
        let preload_text = window_text(state.preload);
        let preload_amount = match sub2api::parse_preload_amount(&preload_text) {
            Ok(value) => value,
            Err(error) => {
                state.status_text = error.to_string();
                update_controls_locked(&mut state);
                return;
            }
        };
        let expected_confirmation = format!(
            "SIGN {}-{}",
            candidate.volume_serial,
            candidate.anchor_sha256.chars().take(12).collect::<String>()
        );
        let report = report_path(candidate.volume_serial);
        let existing_binding = match usb_license_signer::transaction::load_existing_account_binding(
            std::path::Path::new(&candidate.bundle_path),
        ) {
            Ok(binding) => binding,
            Err(error) => {
                state.status_text = format!("读取已有账号绑定失败：{error}");
                update_controls_locked(&mut state);
                return;
            }
        };
        let recovery_key = sub2api::make_provision_recovery_key(
            candidate.volume_serial,
            &candidate.anchor_sha256,
            &candidate.bundle_relative_path,
        );
        Some((
            candidate,
            config,
            preload_amount,
            existing_binding,
            expected_confirmation,
            report,
            recovery_key,
        ))
    };

    let Some((candidate, config, preload_amount, existing_binding, confirm, report, recovery_key)) =
        preparation
    else {
        return;
    };
    let vault_path = match usb_license_signer::key_blob::default_portable_vault_path() {
        Ok(path) => path,
        Err(error) => {
            set_status_text(format!("便携签名库路径准备失败：{error}"));
            return;
        }
    };
    if !vault_path.is_file() {
        set_status_text(
            "签名器缺少同目录的加密签名库，请复制完整签名器目录后重试；未修改 U 盘。".to_string(),
        );
        return;
    }
    let Some(password) = (unsafe { prompt_signing_password(hwnd, false) }) else {
        set_status_text("已取消注册，未修改 U 盘。".to_string());
        return;
    };
    if let Err(error) = usb_license_signer::key_blob::portable_password_is_valid(&password) {
        set_status_text(format!("签名密码不符合要求：{error}"));
        return;
    }
    start_provision_sign(ProvisionSignRequest {
        sign: SignRequest {
            drive: candidate.drive_root,
            relative: candidate.bundle_relative_path,
            portable_vault: vault_path.display().to_string(),
            password,
            confirm,
            report_path: report,
        },
        config,
        preload_amount,
        existing_binding,
        recovery_key,
    });
}

fn prepare_retry_preload_from_ui() {
    let preparation = {
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
            state.status_text = "目标深度检查失败，不能重试预充值。".to_string();
            update_controls_locked(&mut state);
            return;
        };
        if json_bool(value, &["deepVerify", "ok"]) != Some(true) {
            state.status_text = "Bundle 完整性检查未通过，不能重试预充值。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        if !json_string(value, &["license", "state"]).eq_ignore_ascii_case("signed") {
            state.status_text = "只有已签名的 U 盘才能重试预充值。".to_string();
            update_controls_locked(&mut state);
            return;
        }
        let binding = match usb_license_signer::transaction::load_existing_account_binding(
            std::path::Path::new(&candidate.bundle_path),
        ) {
            Ok(Some(binding)) => binding,
            Ok(None) => {
                state.status_text = "当前许可证没有账号绑定，无法重试预充值。".to_string();
                update_controls_locked(&mut state);
                return;
            }
            Err(error) => {
                state.status_text = format!("读取账号绑定失败：{error}");
                update_controls_locked(&mut state);
                return;
            }
        };
        let current_license_digest = match usb_license_signer::transaction::inspect_license(
            std::path::Path::new(&candidate.bundle_path),
            None,
        ) {
            Ok(result) => match result.license_digest {
                Some(digest) => digest,
                None => {
                    state.status_text = "当前许可证没有可用摘要，无法重试预充值。".to_string();
                    update_controls_locked(&mut state);
                    return;
                }
            },
            Err(error) => {
                state.status_text = format!("读取当前许可证摘要失败：{error}");
                update_controls_locked(&mut state);
                return;
            }
        };
        match sub2api::load_pending_preload(&binding.usb_asset_id) {
            Ok(Some(pending)) if pending.license_digest == current_license_digest => {}
            Ok(Some(_)) => {
                state.status_text = "待处理记录与当前许可证不一致，已停止。".to_string();
                update_controls_locked(&mut state);
                return;
            }
            Ok(None) => {
                state.status_text = "没有找到该 U 盘的待处理预充值记录。".to_string();
                update_controls_locked(&mut state);
                return;
            }
            Err(error) => {
                state.status_text = error.to_string();
                update_controls_locked(&mut state);
                return;
            }
        }
        let mut config = Sub2ApiConfig {
            base_url: window_text(state.base_url),
            admin_api_key: window_text(state.admin_key),
            fake_email_domain: window_text(state.email_domain),
        };
        config.base_url = config.base_url.trim().to_string();
        config.fake_email_domain = config.fake_email_domain.trim().to_string();
        if let Err(error) = sub2api::validate_config(&config) {
            state.status_text = error.to_string();
            update_controls_locked(&mut state);
            return;
        }
        if let Err(error) = sub2api::save_config(&config) {
            state.status_text = error.to_string();
            update_controls_locked(&mut state);
            return;
        }
        Some((config, binding, current_license_digest))
    };

    let Some((config, binding, license_digest)) = preparation else {
        return;
    };
    start_retry_preload(RetryPreloadRequest {
        config,
        binding,
        license_digest,
    });
}

fn window_text(hwnd: HwndValue) -> String {
    if hwnd == 0 {
        return String::new();
    }
    unsafe {
        let target = to_hwnd(hwnd);
        let length = GetWindowTextLengthW(target);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = GetWindowTextW(target, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..written.max(0) as usize])
    }
}

fn set_status_text(text: String) {
    let Some(lock) = STATE.get() else { return };
    let Ok(mut state) = lock.lock() else { return };
    state.status_text = text;
    update_controls_locked(&mut state);
    drop(state);
    refresh_controls();
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
        let portable_vault = usb_license_signer::key_blob::default_portable_vault_path()
            .ok()
            .filter(|path| path.is_file());
        if let Some(path) = portable_vault {
            json!({
                "ok": true,
                "mode": "portable-password",
                "vaultPath": path.display().to_string(),
                "keyId": json_string(&value, &["signing", "keyId"]),
                "publicKeySha256": json_string(&value, &["signing", "publicKeySha256"]),
            })
        } else {
            json!({
                "ok": false,
                "mode": "portable-missing",
                "error": "签名器缺少同目录的加密便携签名库；请复制完整签名器目录后重试"
            })
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
    run_command_with_password(parts, None)
}

fn run_command_with_password(parts: &[String], password: Option<&str>) -> Result<Value, String> {
    let args = parts.iter().map(OsString::from).collect::<Vec<_>>();
    match password {
        Some(password) => run_with_portable_password(args, password),
        None => run_with_args(args),
    }
    .map_err(|error| error.to_string())
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

fn can_provision_sign_locked(state: &GuiState) -> bool {
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

fn can_retry_preload_locked(state: &GuiState) -> bool {
    if state.busy || state.candidates.len() != 1 || state.selected != Some(0) {
        return false;
    }
    let Some(inspection) = state.inspection.as_ref() else {
        return false;
    };
    let Some(value) = inspection.value.as_ref() else {
        return false;
    };
    if json_bool(value, &["deepVerify", "ok"]) != Some(true)
        || !json_string(value, &["license", "state"]).eq_ignore_ascii_case("signed")
    {
        return false;
    }
    let Some(candidate) = state.candidates.first() else {
        return false;
    };
    let Ok(Some(binding)) = usb_license_signer::transaction::load_existing_account_binding(
        std::path::Path::new(&candidate.bundle_path),
    ) else {
        return false;
    };
    sub2api::has_pending_preload(&binding.usb_asset_id).unwrap_or(false)
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
    provision_sign: HwndValue,
    retry_preload: HwndValue,
    summary: HwndValue,
    status: HwndValue,
    candidate_texts: Vec<String>,
    selected: Option<usize>,
    summary_text: String,
    status_text: String,
    busy: bool,
    can_provision_sign: bool,
    can_retry_preload: bool,
}

fn refresh_controls() {
    let snapshot = STATE.get().and_then(|lock| {
        lock.lock().ok().map(|state| ControlsSnapshot {
            targets: state.targets,
            refresh: state.refresh,
            provision_sign: state.provision_sign,
            retry_preload: state.retry_preload,
            summary: state.summary,
            status: state.status,
            candidate_texts: state.candidates.iter().map(candidate_display).collect(),
            selected: state.selected,
            summary_text: summary_text(&state),
            status_text: state.status_text.clone(),
            busy: state.busy,
            can_provision_sign: can_provision_sign_locked(&state),
            can_retry_preload: can_retry_preload_locked(&state),
        })
    });
    if let Some(snapshot) = snapshot {
        startup_log(&format!(
            "refresh-controls: targets={} summary={} status={} provisionSign={} retryPreload={} busy={} canProvisionSign={} candidates={} summaryLen={} statusLen={}",
            snapshot.targets,
            snapshot.summary,
            snapshot.status,
            snapshot.provision_sign,
            snapshot.can_retry_preload,
            snapshot.busy,
            snapshot.can_provision_sign,
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
    if snapshot.provision_sign != 0 {
        EnableWindow(
            to_hwnd(snapshot.provision_sign),
            snapshot.can_provision_sign as i32,
        );
    }
    if snapshot.retry_preload != 0 {
        EnableWindow(
            to_hwnd(snapshot.retry_preload),
            snapshot.can_retry_preload as i32,
        );
    }
}

fn worker_event_name(event: &WorkerEvent) -> &'static str {
    match event {
        WorkerEvent::Scan(_) => "scan",
        WorkerEvent::Inspect { .. } => "inspect",
        WorkerEvent::ProvisionSign(_) => "provision-sign",
        WorkerEvent::RetryPreload(_) => "retry-preload",
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
            lines.push(if status.mode == "portable-password" {
                "签名环境：便携加密签名库已准备（签名时输入密码）".to_string()
            } else if status.generated {
                "签名环境：已自动初始化并匹配当前安装包".to_string()
            } else {
                "签名环境：已自动找到并匹配当前安装包".to_string()
            });
        }
        Some(status) => {
            lines.push(if status.mode == "portable-missing" {
                "签名环境：缺少便携加密签名库，已停止".to_string()
            } else {
                "签名环境：未匹配，已停止".to_string()
            });
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
        mode: ready
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("dpapi-local")
            .to_string(),
        error: ready
            .get("error")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn inspection_status_text(result: &InspectView, key_matched: bool) -> String {
    if let Some(error) = &result.error {
        return format!("目标不可签名：{error}");
    }
    if result
        .value
        .as_ref()
        .map(|value| json_string(value, &["license", "state"]))
        .is_some_and(|state| state.eq_ignore_ascii_case("signed"))
    {
        return "该 Bundle 已完成签名，签名按钮已禁用；如需重新签名，请使用未签名的干净发行包。"
            .to_string();
    }
    if key_matched {
        "检查完成；已自动准备签名环境，可以注册账号并签名。".to_string()
    } else {
        "检查完成；签名环境或安装包绑定不满足，已停止。".to_string()
    }
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

/// Ask for the vault password using the native Windows credential dialog.
/// `CredUIPromptForCredentialsW` owns the visual prompt; its password buffer
/// is zeroized immediately after conversion and is never persisted by the
/// dialog.
unsafe fn prompt_signing_password(owner: HWND, first_use: bool) -> Option<Zeroizing<String>> {
    let caption = wide("Codex U 盘签名授权");
    let message = if first_use {
        wide("首次使用：请设置一个至少 12 字节的签名密码。该密码用于保护便携签名库，之后可把整个签名工具目录复制到其他 Windows 电脑使用。")
    } else {
        wide("请输入便携签名库密码。密码只在本次签名期间使用，不会保存到系统或日志。")
    };
    let target = wide("CodexUsbLicenseSignerPortableVault");
    let mut username = wide("Codex USB signer");
    let mut password = Zeroizing::new(vec![0u16; 1024]);
    let mut save = 0;
    let info = CREDUI_INFOW {
        cbSize: size_of::<CREDUI_INFOW>() as u32,
        hwndParent: owner,
        pszMessageText: message.as_ptr(),
        pszCaptionText: caption.as_ptr(),
        hbmBanner: null_mut(),
    };
    let flags = CREDUI_FLAGS_ALWAYS_SHOW_UI
        | CREDUI_FLAGS_DO_NOT_PERSIST
        | CREDUI_FLAGS_GENERIC_CREDENTIALS
        | CREDUI_FLAGS_PASSWORD_ONLY_OK;
    let result = CredUIPromptForCredentialsW(
        &info,
        target.as_ptr(),
        null(),
        0,
        username.as_mut_ptr(),
        username.len() as u32,
        password.as_mut_ptr(),
        password.len() as u32,
        &mut save,
        flags,
    );
    if result != 0 {
        return None;
    }
    let length = password
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(password.len());
    String::from_utf16(&password[..length])
        .ok()
        .map(Zeroizing::new)
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
    fn signed_bundle_status_does_not_claim_that_it_can_be_signed_again() {
        let result = InspectView {
            value: Some(serde_json::json!({
                "license": {"state": "signed"}
            })),
            error: None,
        };
        let status = inspection_status_text(&result, true);
        assert!(status.contains("已完成签名"));
        assert!(!status.contains("可以注册账号并签名"));
    }

    #[test]
    fn wide_text_is_nul_terminated() {
        let text = wide("签名授权");
        assert_eq!(text.last(), Some(&0));
        assert!(text.len() > 1);
    }
}
