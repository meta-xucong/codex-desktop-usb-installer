fn main() {
    println!("cargo:rerun-if-env-changed=CODEX_404TOKEN_ADMIN_API_KEY");
    println!("cargo:rerun-if-env-changed=SUB2API_ADMIN_API_KEY");

    // Generate a Rust constant from the provisioned release credential rather
    // than relying on option_env! to observe a value through every Cargo
    // wrapper. Never print the credential; it exists only in this generated
    // build file and the resulting executable.
    let key = ["CODEX_404TOKEN_ADMIN_API_KEY", "SUB2API_ADMIN_API_KEY"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                // API keys are opaque tokens; line wrapping from copied text
                // files is transport whitespace, not part of the credential.
                .map(|value| value.split_whitespace().collect::<String>())
                .filter(|value| !value.is_empty())
        });
    let generated = match key {
        Some(key) => format!(
            "pub const EMBEDDED_ADMIN_API_KEY: Option<&str> = Some({key:?});\npub const EMBEDDED_ADMIN_API_KEY_FALLBACK: Option<&str> = None;\n"
        ),
        None => "pub const EMBEDDED_ADMIN_API_KEY: Option<&str> = None;\npub const EMBEDDED_ADMIN_API_KEY_FALLBACK: Option<&str> = None;\n".to_string(),
    };
    let out_dir = std::env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR");
    std::fs::write(
        std::path::PathBuf::from(out_dir).join("embedded_admin_key.rs"),
        generated,
    )
    .expect("failed to generate embedded administrator credential constant");
}
