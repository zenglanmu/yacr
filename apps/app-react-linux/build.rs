//! Tauri build helper for the React native Linux probe host.
//!
//! See `docs/adr/0006-react-native-linux-experiment.md`.
//!
//! Build scripts are compiled for the **host**, and `tauri-build` is a
//! Linux-host build-dependency, so the reference below is host-cfg-gated: on a
//! non-Linux host the crate is not linked and this becomes a no-op. Within a
//! Linux host, `tauri_build::build()` is additionally gated on the **target** os
//! (`CARGO_CFG_TARGET_OS`), so a wasm (or other-target) check never generates a
//! Tauri context.

#[cfg(target_os = "linux")]
fn run_tauri_build() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        tauri_build::build();
    }
}

#[cfg(not(target_os = "linux"))]
fn run_tauri_build() {}

fn main() {
    run_tauri_build();
}
