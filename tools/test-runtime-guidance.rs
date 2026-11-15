// Standalone harness: exercise the production merge/writer without booting Tauri.
#![allow(dead_code)]
mod app {
    pub mod usb_guard {
        pub struct Paths {
            pub codex_home: std::path::PathBuf,
        }
        pub fn guard_required() -> bool {
            false
        }
        pub fn local_data_paths() -> std::io::Result<Paths> {
            Err(std::io::Error::other(
                "This fixture requires an explicit isolated home",
            ))
        }
    }
}
mod subject {
    include!("../src-tauri/src/app/codex_agents_hint.rs");
    pub fn install_fixture(home: &std::path::Path) -> std::io::Result<bool> {
        ensure_hint_at(home)
    }
}
fn main() {
    let home = std::env::args_os()
        .nth(1)
        .expect("isolated CODEX_HOME required");
    println!(
        "updated={}",
        subject::install_fixture(std::path::Path::new(&home)).unwrap()
    );
}
