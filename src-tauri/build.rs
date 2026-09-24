fn main() {
    // tauri-build reads STATIC_VCRUNTIME on Windows but does not ask Cargo to
    // rerun when it changes, so a plain `cargo build` could keep stale CRT
    // link arguments from a previous `cargo tauri build`.
    println!("cargo:rerun-if-env-changed=STATIC_VCRUNTIME");
    tauri_build::build()
}
