fn main() {
    println!("cargo:rerun-if-env-changed=CODEX_404TOKEN_ADMIN_API_KEY");
    println!("cargo:rerun-if-env-changed=SUB2API_ADMIN_API_KEY");
}
