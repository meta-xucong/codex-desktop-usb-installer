#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
#[path = "../gui.rs"]
mod gui;

#[cfg(target_os = "windows")]
fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|value| value == "--diagnose-key") {
        let Some(output) = args.get(1) else {
            std::process::exit(2);
        };
        if args.len() != 2 {
            std::process::exit(2);
        }
        if gui::write_key_diagnostic(std::path::Path::new(output)).is_err() {
            std::process::exit(1);
        }
        return;
    }
    gui::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("usb-license-signer-gui requires Windows");
}
