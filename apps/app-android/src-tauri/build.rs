//! Tauri build helper for the Android host (`apps/app-android/src-tauri`).
//!
//! For an **Android target** this generates the Tauri context from
//! `tauri.conf.json` and, when the Tauri CLI runs the build
//! (`TAURI_ANDROID_PROJECT_PATH` is set), re-generates the Gradle glue in
//! `gen/android`. It also exports the `TAURI_ANDROID_PACKAGE_NAME_PREFIX` /
//! `TAURI_ANDROID_PACKAGE_NAME_APP_NAME` pair that
//! `#[tauri::mobile_entry_point]` (in `src/lib.rs`) expands with; without them the
//! macro is a compile error, which is the intended loud failure.
//!
//! Build scripts are compiled for the **host**, so `tauri-build` is an ordinary
//! build-dependency and the reference always resolves. The real work is gated on
//! the **target** os (`CARGO_CFG_TARGET_OS`): a host or wasm check never generates
//! a Tauri context for this crate, which is what keeps
//! `cargo check --workspace --all-targets` and the `wasm32-unknown-unknown`
//! `--lib` gate compiling.

fn main() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=icons/icon.png");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // `tauri_build::build()` panics on a real configuration error; there is
        // deliberately no silent fallback to a context-less library.
        tauri_build::build();
    }
}
