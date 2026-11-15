#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "usb-cdp-diagnostic")]
compile_error!("the ordinary codex-app-manager entry point cannot be built with usb-cdp-diagnostic; use codex-usb-cdp-diagnostic");

fn main() {
    codex_app_manager_lib::run()
}
