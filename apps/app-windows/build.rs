//! Tauri build helper for the native Windows host (`apps/app-windows`).
//!
//! For a **Windows target** this generates the Tauri context and embeds the PE
//! icon/version resource from `assets/yacr-icon.ico` (via `tauri-build`'s
//! `tauri_winres`). The icon is the shared single source of truth; a missing
//! icon fails the build loudly instead of shipping an icon-less executable.
//!
//! Build scripts are compiled for the **host**, but `tauri-build` is an ordinary
//! build-dependency here, so the reference always resolves. The real work is
//! gated on the **target** os (`CARGO_CFG_TARGET_OS`), so a workspace wasm/other
//! target check never generates a Tauri context and never runs the resource
//! compiler.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/yacr-icon.ico");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=icons/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // `tauri_build::build()` panics on a real configuration/resource error;
        // there is deliberately no silent fallback to an icon-less binary.
        tauri_build::build();
    }
}
