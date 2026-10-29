#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(feature = "usb-cdp-diagnostic"))]
compile_error!("codex-usb-cdp-diagnostic must be built with usb-cdp-diagnostic");

fn main() {
    codex_app_manager_lib::run()
}
