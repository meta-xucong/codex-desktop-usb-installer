#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
#[path = "../gui.rs"]
mod gui;

#[cfg(target_os = "windows")]
fn main() {
    gui::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("usb-license-signer-gui requires Windows");
}
