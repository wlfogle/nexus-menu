//! Declares the commands the page may call. Tauri turns each into a
//! permission (`allow-<name>`) that a capability file has to grant, so a
//! command added in Rust but not listed here and in `capabilities/` cannot be
//! reached from the page.

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "scan",
            "install",
            "list_generated",
            "remove",
            "prefixes",
            "check_update",
            "install_update",
        ]),
    ))
    .expect("failed to run tauri-build");
}
