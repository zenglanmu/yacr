//! Android host for the CAD viewer/measure/annotation app.
//!
//! The host is a **Tauri 2 WebView carrier with a native CAD surface below it**
//! (P5, `surfaceMode = "native-below"`, `documentOwner = "native"`): the shared
//! React chrome (`apps/desktop-ui`, built with `YACR_RUNTIME=android` into
//! `../dist`) runs in the Android WebView and the **native Rust
//! [`cad_ui_core::session::CadSession`] owns `cad-db`**, presented by
//! `cad-render-wgpu` on the `SurfaceView` the Kotlin lane layers beneath the
//! WebView. The wasm session must not run on this carrier and the native shell
//! never instantiates a second document authority (`AGENTS.md` iron rule 8).
//!
//! What the host owns:
//!
//! * [`host`] — the Tauri entry point, the shared command handler plus the two
//!   shell-only reports, `Shared`/telemetry, and the `MainEventsCleared` tick
//!   drive;
//! * [`platform`] — the SAF-backed file contracts (open + DXF-export save;
//!   Plot still refuses by capability), the DOM surface source, the durable
//!   `filesDir` config/preference store and the `nativeAssetsReady` handshake;
//! * [`popup`] — the in-DOM popup seam (`PopupOps` payload + the `ShellPopupHost`
//!   that marshals the shared lifecycle onto the loop thread);
//! * [`surface`] — the frozen JNI surface callbacks and the
//!   `SurfaceProbePresenter` lifecycle (Vulkan only, no GLES/software fallback);
//! * [`saf`] — the Storage Access Framework JNI bridge (Android only): the
//!   Activity `document*` methods behind the platform file contracts;
//! * [`policy`] — the target-independent, unit-tested refusal markers, capability
//!   set and JNI symbol names;
//! * [`input`] — the partition-local → canvas coordinate mapping for pointer/wheel
//!   input (pure, unit-tested on the development host);
//! * [`refusal`]/[`report`] — the `dom-canvas` refusal/report vocabulary, kept
//!   because the chrome's shell reports remain valid on every carrier.
//!
//! On every other target this crate is a documented empty shell: `tauri` and the
//! CAD crates are Android-only dependencies, so a host `--all-targets` check and
//! the `wasm32-unknown-unknown --lib` gate compile the modules below instead of
//! pretending an Android app can run there. [`start`] is the explicit
//! NOT IMPLEMENTED answer for those targets, not an empty success.

pub mod policy;
pub mod refusal;
pub mod report;

#[cfg(target_os = "android")]
pub mod host;
#[cfg(target_os = "android")]
pub mod saf;
#[cfg(target_os = "android")]
pub mod surface;

/// The explicit `cad-platform` contract set of this carrier.
///
/// Compiled on every non-wasm target so its refusal tests run on the development
/// host (no device/emulator involved); it has no `tauri`/GPU dependency.
#[cfg(not(target_arch = "wasm32"))]
pub mod platform;

/// Durable plugin storage that resolves `filesDir` after the asset handshake.
#[cfg(not(target_arch = "wasm32"))]
pub mod plugin_store;

/// Partition-local DOM input → presented-canvas coordinates (see the module docs).
///
/// Pure, so it is compiled on every non-wasm target and its unit tests run on the
/// development host: the offset arithmetic must not be validated only on a device.
#[cfg(not(target_arch = "wasm32"))]
pub mod input;

/// The popup seam (`ShellPopupHost` + the `textEdit` payload derivation).
///
/// Popups are in-DOM overlays and the seam is toolkit-neutral, so the `PopupOps`
/// half is compiled on every non-wasm target (its refusal contract runs on the
/// development host). The Tauri marshalling half is Android-only, like [`host`].
#[cfg(not(target_arch = "wasm32"))]
pub mod popup;

/// Source-structure contracts of the Android-only modules (`host/tick.rs`,
/// `surface.rs`), compiled and run on the **development host**.
///
/// `host`/`surface` are `cfg(target_os = "android")`, so tests declared inside them
/// never run on any gate (`cargo test -p app-android` builds for the host). These
/// assertions read those files with `include_str!` instead, and the behavioural
/// tests stay next to the code they exercise.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod contract;

#[cfg(target_os = "android")]
pub use host::run;

/// What this crate reports when it is asked to run somewhere it cannot: an
/// Android host is only buildable for Android targets.
pub const NON_ANDROID_DIAGNOSTIC: &str =
    "app-android: an Android host; build with `tauri android build` (NOT IMPLEMENTED for this target)";

/// Explicit refusal for non-Android targets.
///
/// There is no binary here, so nothing user-facing calls this; it exists so a
/// future caller gets a modeled failure instead of an empty `Ok(())`.
#[cfg(not(target_os = "android"))]
pub fn start() -> Result<(), String> {
    Err(NON_ANDROID_DIAGNOSTIC.to_string())
}
