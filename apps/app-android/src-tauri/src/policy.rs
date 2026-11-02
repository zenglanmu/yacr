//! Target-independent policy of the Android `native-below` carrier (P5).
//!
//! Everything here is **pure** (no `tauri`, no `jni`/`ndk`, no GPU): the stable
//! refusal markers, the software-adapter rule, the advertised capability set and
//! the frozen JNI symbol names. Keeping it out of the Android-only modules is what
//! lets the contract tests run on the development host — the device assertions
//! (surface creation, Vulkan adapter, pixels) are **NOT RUN** and must not be
//! inferred from these tests.
//!
//! # What this carrier is
//!
//! `surfaceMode = "native-below"`, `documentOwner = "native"`: the native Rust
//! [`cad_ui_core::session::CadSession`] owns `cad-db` and the renderer presents to
//! the `SurfaceView` layered **beneath** the Tauri WebView. The wasm session must
//! not run on this carrier, and the native shell must never instantiate a second
//! document authority (AGENTS.md iron rule 8).
//!
//! # Why the markers live here
//!
//! Android has no shell exit status to judge a run by; the `logcat` marker line is
//! the primary evidence. The marker text therefore must not be re-spelled at a
//! call site: [`SOFTWARE_VULKAN_MARKER`] is the one string evidence tooling reads.

/// Prefix of every explicit GPU refusal on this carrier.
pub const UNSUPPORTED_GPU_PREFIX: &str = "yacr: unsupported-gpu: ";

/// Stable marker line for a software Vulkan adapter on Android.
///
/// Android v1 requires a hardware Vulkan adapter. SwiftShader/lavapipe (an
/// emulator's software Vulkan) is **refused explicitly**, never accepted as a
/// result and never silently replaced by GLES or a CPU path.
pub const SOFTWARE_VULKAN_MARKER: &str = "yacr: unsupported-gpu: software-vulkan";

/// The `surfaceMode` this host reports (already modelled by the chrome).
pub const SURFACE_MODE: &str = "native-below";

/// Who owns the document on this carrier (`HostProfile::native_below`).
pub const DOCUMENT_OWNER: &str = "native";

/// The stable marker prefix of the `dom-canvas` runtime refusal (shared
/// vocabulary with the X11/Web carriers; unchanged by P5).
pub const UNSUPPORTED_RUNTIME_PREFIX: &str = "yacr: unsupported-runtime: ";

/// Open a drawing through the Storage Access Framework (`documentLaunchOpen` +
/// `documentReadUri`). The bytes are read by the Kotlin lane and delivered as a
/// platform event; the tick never opens a document itself.
pub const CAN_OPEN: bool = true;
/// No document can be created (no New funnel, no second document authority).
pub const CAN_NEW: bool = false;
/// DXF Save As through `documentLaunchSave` + `documentWriteUri` (lossy export).
pub const CAN_SAVE: bool = true;
/// No plot/vector export (SVG/PDF) on this carrier yet.
pub const CAN_PLOT: bool = false;
/// No background import worker: the platform read completes before delivery.
pub const IMPORT_WORKER: bool = false;

/// The four frozen JNI symbols the Kotlin lane links against.
///
/// The **names are the contract**: `dev.yacr.app.MainActivity` declares
/// `private external fun <name>` for each one, and the Kotlin lane is a parallel
/// implementation. The unit test in this module pins that the crate defines
/// exactly these symbols (a rename breaks the contract, not just a build).
pub const JNI_SYMBOLS: [&str; 4] = [
    "Java_dev_yacr_app_MainActivity_nativeSurfaceCreated",
    "Java_dev_yacr_app_MainActivity_nativeSurfaceResized",
    "Java_dev_yacr_app_MainActivity_nativeSurfaceDestroyed",
    "Java_dev_yacr_app_MainActivity_nativeAssetsReady",
];

/// The explicit reason a file operation is refused by a **non-Android build**.
///
/// Named by the operation so a refusal is self-describing in the session's error
/// (`cad_status.error`), never a generic "failed". The real Android build routes
/// these operations through `crate::saf`; this wording is the honest fallback
/// for the development-host build of the crate, which does not compile the JNI
/// bridge.
pub fn file_refusal(operation: &str) -> String {
    format!(
        "{operation} is not supported by this build of the Android host: the SAF bridge \
         (Storage Access Framework) is only compiled for the Android target"
    )
}

/// The explicit reason preference persistence is refused.
///
/// The host has no durable preference path on Android v1; the session falls back
/// to in-memory defaults and must not claim the value was stored.
pub fn config_persist_refusal() -> String {
    "preference persistence is not supported by the Android host: no durable preference path \
     is configured"
        .to_string()
}

/// Whether a Vulkan adapter is software rendering and must be refused.
///
/// Two independent signals, both required to be absent: wgpu's own device type
/// (`cpu`) and the vendor-reported name of the two software implementations that
/// can appear on an Android emulator. A `virtual_gpu` is *not* software (a
/// virtualised hardware GPU is a real GPU) and stays eligible.
pub fn is_software_adapter(device_type: &str, name: &str) -> bool {
    if device_type.eq_ignore_ascii_case("cpu") {
        return true;
    }
    let name = name.to_ascii_lowercase();
    name.contains("swiftshader") || name.contains("lavapipe")
}

/// Stable class of one explicit present failure (evidence vocabulary).
///
/// Derived from the renderer's own error discriminants (never from free text), so
/// an evidence run can group failures mechanically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentClass {
    /// Surface acquire failed (outdated/validation; transient timeout/occluded).
    Acquire,
    /// `CadSession::render` failed.
    Render,
    /// The texture could not be presented.
    Present,
    /// The swapchain could not take the platform size.
    Reconfigure,
    /// The device/surface was lost; the presenter must be rebuilt.
    DeviceLost,
    /// The GPU memory budget refused the frame (device kept).
    GpuMemory,
}

impl PresentClass {
    /// Stable lowercase token for the `[probe] cad-present-error class=…` line.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "acquire",
            Self::Render => "render",
            Self::Present => "present",
            Self::Reconfigure => "reconfigure",
            Self::DeviceLost => "device_lost",
            Self::GpuMemory => "gpu_memory",
        }
    }
}

/// Build the explicit GPU-refusal marker for one reason token.
pub fn unsupported_gpu_marker(reason: &str) -> String {
    format!("{UNSUPPORTED_GPU_PREFIX}{reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_software_adapter_marker_is_the_registered_line() {
        // Byte-for-byte stable: evidence collection classifies on this line.
        assert_eq!(
            SOFTWARE_VULKAN_MARKER,
            "yacr: unsupported-gpu: software-vulkan"
        );
        assert!(SOFTWARE_VULKAN_MARKER.starts_with(UNSUPPORTED_GPU_PREFIX));
        assert_eq!(
            unsupported_gpu_marker("software-vulkan"),
            SOFTWARE_VULKAN_MARKER
        );
        // The Android GPU vocabulary is distinct from the dom-canvas one.
        assert!(!SOFTWARE_VULKAN_MARKER.starts_with(UNSUPPORTED_RUNTIME_PREFIX));
    }

    #[test]
    fn software_adapters_are_refused_by_type_or_known_name() {
        assert!(is_software_adapter("cpu", "llvmpipe"));
        assert!(is_software_adapter("other", "SwiftShader Device (Subzero)"));
        assert!(is_software_adapter("cpu", "lavapipe (Mesa)"));
        // A real GPU is eligible, including a virtualised one.
        assert!(!is_software_adapter("integrated_gpu", "Mali-G78"));
        assert!(!is_software_adapter("discrete_gpu", "Adreno 740"));
        assert!(!is_software_adapter("virtual_gpu", "virtio-gpu Vulkan"));
    }

    #[test]
    fn open_and_dxf_save_are_advertised_new_and_plot_are_not() {
        // The mirror of the `HostCapabilities` the tick passes to `CadSession::new`:
        // a fabricated capability would make the chrome offer a dead action, and a
        // missing one would hide a real SAF path. Values are read through a binding
        // pair so the assertion checks behaviour, not a literal.
        let advertised = [
            ("open", CAN_OPEN, true),
            ("save", CAN_SAVE, true),
            ("new", CAN_NEW, false),
            ("plot", CAN_PLOT, false),
            ("import_worker", IMPORT_WORKER, false),
        ];
        for (name, available, expected) in advertised {
            assert_eq!(available, expected, "{name} capability");
        }
    }

    #[test]
    fn file_refusals_name_the_operation_and_say_unsupported() {
        // This wording is the non-Android build fallback; the Android target
        // routes the same operations through `saf::launch_open`/`launch_save`.
        for operation in ["open", "open target", "save", "atomic write"] {
            let reason = file_refusal(operation);
            assert!(reason.starts_with(operation), "{reason}");
            assert!(reason.contains("not supported"), "{reason}");
            assert!(reason.contains("SAF bridge"), "{reason}");
        }
        assert!(config_persist_refusal().contains("not supported"));
    }

    #[test]
    fn the_carrier_names_are_the_wire_spellings_the_chrome_reads() {
        // The chrome routes on `hostProfile.surfaceMode`/`documentOwner`; a typo
        // here would send the React tree down the `dom-canvas` branch. The values
        // are the kebab-case/lowercase serde spellings of
        // `cad_ui_core::session::HostProfile::native_below()`.
        assert_eq!(SURFACE_MODE, "native-below");
        assert_eq!(DOCUMENT_OWNER, "native");
        // The tick must flush the shared profile, not a locally built envelope:
        // that is what keeps `cad_status` and the pushed `status-changed` bytes
        // identical on this carrier (and the carrier `native-below`).
        let tick = include_str!("host/tick.rs");
        assert!(
            tick.contains("HostProfile::native_below()"),
            "the tick must flush with the native-below profile"
        );
        assert!(
            tick.contains("flush_events_current"),
            "the tick must push status events through the shared flush"
        );
    }

    #[test]
    fn the_four_frozen_jni_symbols_are_defined_in_this_crate() {
        // The Kotlin lane (parallel implementation) declares exactly these
        // `external fun`s; a symbol rename links to nothing at runtime.
        let surface = include_str!("surface.rs");
        let platform = include_str!("platform.rs");
        for symbol in JNI_SYMBOLS {
            let definition = format!("fn {symbol}");
            assert!(
                surface.contains(&definition) || platform.contains(&definition),
                "JNI symbol is not defined in the crate: {symbol}"
            );
        }
        // Exactly these four: a fifth surface symbol would mean the frozen Kotlin
        // contract grew without the Kotlin lane.
        let surface_symbols = surface
            .matches("fn Java_dev_yacr_app_MainActivity_")
            .count();
        assert_eq!(
            surface_symbols, 3,
            "surface.rs must define exactly the three surface symbols"
        );
        let platform_symbols = platform
            .matches("fn Java_dev_yacr_app_MainActivity_")
            .count();
        assert_eq!(
            platform_symbols, 1,
            "platform.rs must define exactly nativeAssetsReady"
        );
    }
}
