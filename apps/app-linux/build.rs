//! Tauri build helper for the React/Tauri native Linux host (`apps/app-linux`).
//!
//! See `docs/adr/0010-react-linux-host-replaces-slint.md` (the current host
//! decision) and `docs/adr/0006-react-native-linux-experiment.md` (its origin).
//!
//! This script does two things, both gated to a **Linux target**:
//!
//! 1. Generates the Tauri context (`tauri_build::build()`).
//! 2. Compiles the private Wayland protocol shim (`native/wayland_shim.c`) with
//!    `cc` and links the system `wayland-client` (the shim's protocol wrappers are
//!    header-only inline functions that resolve to `libwayland-client` at link
//!    time, so the link flag comes from `wayland-client.pc`).
//!
//! Build scripts are compiled for the **host**, and `tauri-build`/`cc`/`pkg-config`
//! are Linux-host build-dependencies, so the references below are host-cfg-gated:
//! on a non-Linux host they are not linked and the block becomes a no-op. Within a
//! Linux host, the real work is additionally gated on the **target** os
//! (`CARGO_CFG_TARGET_OS`), so a wasm (or other-target) check never compiles the
//! shim and never generates a Tauri context.

#[cfg(target_os = "linux")]
fn run_tauri_build() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        tauri_build::build();
    }
}

#[cfg(not(target_os = "linux"))]
fn run_tauri_build() {}

/// Compile the app-linux private Wayland protocol shim for a Linux target.
#[cfg(target_os = "linux")]
fn build_wayland_shim() -> Result<(), String> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return Ok(());
    }

    println!("cargo:rerun-if-changed=native/wayland_shim.c");
    println!("cargo:rerun-if-changed=native/wayland_shim.h");

    // Probe without emitting Cargo metadata: the include dirs go to the C build and
    // the link flags are emitted below in an explicit order.
    let wayland = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("wayland-client")
        .map_err(|error| {
            format!(
                "app-linux requires the Wayland client development files \
                 (`wayland-client.pc`, e.g. libwayland-dev): {error}"
            )
        })?;

    // Compile the shim first so its static archive is emitted before the dynamic
    // `wayland-client` link.
    let mut build = cc::Build::new();
    build.file("native/wayland_shim.c");
    for include in &wayland.include_paths {
        build.include(include);
    }
    build.compile("yacr_wayland_shim");

    for path in &wayland.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    // Keep libwayland-client even under a distro default `--as-needed`, so the
    // static shim archive's undefined `wl_*` symbols always resolve regardless of
    // link order.
    println!("cargo:rustc-link-arg=-Wl,--push-state");
    println!("cargo:rustc-link-arg=-Wl,--no-as-needed");
    for lib in &wayland.libs {
        println!("cargo:rustc-link-arg=-l{lib}");
    }
    println!("cargo:rustc-link-arg=-Wl,--pop-state");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn build_wayland_shim() -> Result<(), String> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // The native Wayland shim is compiled by the Linux-host build path only.
        // A Linux target from a non-Linux host is NOT IMPLEMENTED for the native
        // Wayland shim (and app-linux itself needs the Linux GTK stack); surface
        // that explicitly instead of a silent omission — no cross toolchain is
        // added here.
        println!(
            "cargo:warning=app-linux: the native Wayland shim is NOT compiled on a non-Linux \
             host; Linux cross-builds of the native Wayland path are unsupported"
        );
    }
    Ok(())
}

fn main() {
    run_tauri_build();
    if let Err(message) = build_wayland_shim() {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}
