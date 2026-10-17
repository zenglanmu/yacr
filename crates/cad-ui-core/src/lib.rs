//! UI-framework-independent shared layer for the CAD hosts.
//!
//! `cad-ui-core` holds the pieces a UI host needs that carry **no** windowing
//! toolkit and **no** GPU: the frontend wire protocol, the logical viewport
//! layout tracker, the owner-thread CAD session wrapper over [`cad_host`], the
//! generic host admission/error state and the bilingual message catalogs. A
//! concrete UI host (the React/Tauri desktop host, …) composes this crate
//! instead of re-implementing the same state machines.
//!
//! Boundary (enforced by `scripts/check-architecture.py`): no `slint`, no `wgpu`,
//! no `tauri`/`gtk`/`gdk`/`ashpd`, no platform bindings (`web-sys`,
//! `android-activity`, `jni`, `ndk`). The whole crate is UI-framework-free.

pub mod i18n;
pub mod layout;
pub mod protocol;
pub mod session;
pub mod shell;

pub use i18n::{
    format_message, placeholders, resolve_locale, FallbackReason, Locale, LocaleResolution,
    Message, MessageCatalog, MessageSource,
};
pub use layout::{
    insets_from, surface_metrics, validate_metrics, CadMetrics, LayoutError, LayoutTracker,
    ViewportInsets,
};
pub use protocol::{
    InputKind, InputTracker, ProbeInput, ProbeInputAck, ProbeStatus, BUTTON_MIDDLE, BUTTON_PRIMARY,
    BUTTON_SECONDARY,
};
pub use session::{
    canonical_command_name, normalize_command, unsupported_command_reason, unsupported_status,
    CadCamera, CadSelection, CadSelectionRow, CadSession, CadStatus, COMMAND_ALIASES,
    COMMAND_TABLE,
};
pub use shell::{
    admit_command, bind_allowed, clear_host_error, note_host_error, projected_status,
    queue_cad_input, HostError, HostErrorKind, ShellState, CAD_COMMAND_CAP, CAD_INPUT_CAP,
};
