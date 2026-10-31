//! Tauri build helper for the native macOS host (`apps/app-macos`).
//!
//! For a **macOS target** this generates the Tauri context from
//! `tauri.conf.json`; `tauri::generate_context!()` in `src/entry.rs` then embeds
//! `build.frontendDist` (`../desktop-ui/dist`), so the React bundle has to exist
//! before the Rust host is checked — exactly like `apps/app-linux`.
//!
//! Build scripts are compiled for the **host**, so `tauri-build` is an ordinary
//! build-dependency and the reference always resolves. The real work is gated on
//! the **target** os (`CARGO_CFG_TARGET_OS`): a Linux host check, a wasm check or
//! any other target never generates a Tauri context for this crate, which is what
//! keeps `cargo check --workspace --all-targets` and the
//! `wasm32-unknown-unknown` `--lib` gate compiling on a non-macOS machine.

fn main() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=icons/icon.png");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Building the context panics on a real configuration error; there is
        // deliberately no silent fallback to a context-less binary.
        tauri_build::build();
    }
}
