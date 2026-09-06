use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=usb-launcher.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let _manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("usb-launcher.manifest")
            .canonicalize()
            .expect("usb-launcher.manifest must exist");
        // The MSVC linker supplies its default asInvoker fragment. Override
        // only that UAC level for this standalone USB entry. The ordinary
        // manager build remains asInvoker; the release script patches its
        // copied USB artifact separately and verifies the final PE resource.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTUAC:level='requireAdministrator'");
        println!("cargo:rustc-link-lib=user32");
    }
}
