fn main() {
    // Keep the build script sensitive to the USB build flag so Cargo rebuilds
    // when the release pipeline switches between the ordinary and USB
    // variants.  Tauri embeds its own RT_MANIFEST resource, so a linker
    // `/MANIFESTUAC` flag cannot override it (and `/MANIFEST:EMBED` would
    // create a duplicate resource).  The release pipeline patches only the
    // copied USB manager with the Windows SDK's mt.exe and verifies the final
    // PE resource.  The ordinary installed manager therefore remains
    // asInvoker, while the USB copy is explicitly requireAdministrator.
    println!("cargo:rerun-if-env-changed=CODEX_USB_MANAGER");
    tauri_build::build();
}
