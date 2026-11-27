//! UI-framework-independent shared layer for the CAD hosts.
//!
//! `cad-ui-core` holds the pieces a UI host needs that carry **no** windowing
//! toolkit and **no** GPU: the frontend wire protocol, the logical viewport
//! layout tracker, the owner-thread CAD session wrapper over [`cad_host`], the
//! generic host admission/error state, the typed CAD IPC admission rules, the
//! modal-popup state machine, launch-option parsing and the bilingual message
//! catalogs. A concrete UI host (the React/Tauri desktop host, …) composes this
//! crate instead of re-implementing the same state machines.
//!
//! Boundary (enforced by `scripts/check-architecture.py`): no UI toolkit, no
//! `wgpu`, no `tauri`/`gtk`/`gdk`/`ashpd`, no platform bindings (`web-sys`,
//! `android-activity`, `jni`, `ndk`). The whole crate is UI-framework-free.

pub mod action_recorder;
pub mod bim;
pub mod display;
pub mod host_read;
pub mod i18n;
pub mod launch;
pub mod layout;
pub mod match_photo;
pub mod mechanical;
pub mod outliner;
pub mod overlays;
pub mod param;
pub mod popup;
pub mod protocol;
pub mod scene;
pub mod session;
pub mod shell;

pub use action_recorder::{
    decode_action_macro, encode_action_macro, is_recorder_control_name, validate_action_macro,
    ActionMacro, ActionMacroSummary, ActionPlaybackSnapshot, ActionRecorderSnapshot,
    ActionRecordingSnapshot, MacroCodecError, MacroStep, ACTION_MACRO_FORMAT,
    ACTION_MACRO_MAX_STEPS, ACTION_MACRO_MAX_TEXT, ACTION_MACRO_VERSION,
    ACTION_RECORDER_UNSUPPORTED, RECORDER_CONTROL_NAMES,
};
pub use bim::{
    query_response, BimEvidenceWire, BimFacetCountWire, BimFilterWire, BimObjectWire, BimOpWire,
    BimPropertyCountWire, BimPropertyFilterWire, BimQueryRequest, BimQueryResponse, BimRowWire,
    BimSourceWire, BimSummaryWire, BimValueWire, BIM_DEFAULT_LIMIT, BIM_REAL_NOT_FINITE, BIM_STALE,
    BIM_UNSUPPORTED, BIM_UNSUPPORTED_VALUE,
};
pub use host_read::{
    HostReadAnswer, HostReadHost, HostReadRequest, HostReadSeam, HostReads, ACTION_RECORDER_KIND,
    BIM_QUERY_KIND, HOST_READ_UNKNOWN_KIND,
};
pub use mechanical::{
    MechanicalTreeResponse, MechanicalTreeRow, MECHANICAL_TREE_KIND, MECHANICAL_TREE_UNSUPPORTED,
};
pub use param::{ParamListResponse, ParamListRow, PARAM_LIST_KIND, PARAM_LIST_UNSUPPORTED};

pub use i18n::{
    format_message, locale_from_env, placeholders, resolve_locale, supported_locale_for,
    FallbackReason, Locale, LocaleResolution, Message, MessageCatalog, MessageSource,
};
pub use launch::{LaunchOptions, INITIAL_HEIGHT, INITIAL_WIDTH};
pub use layout::{
    insets_from, surface_metrics, validate_metrics, CadMetrics, LayoutError, LayoutTracker,
    ViewportInsets,
};
pub use popup::{
    PopupAck, PopupGate, PopupKind, PopupOps, PopupPayload, PopupReset, TextEditPayload,
    POPUP_LABEL,
};
pub use protocol::{
    InputKind, InputTracker, ProbeInput, ProbeInputAck, ProbeStatus, BUTTON_MIDDLE, BUTTON_PRIMARY,
    BUTTON_SECONDARY,
};
pub use session::{
    canonical_command_name, command_name, invalid_justifytext_argument_reason,
    invalid_layerpmode_argument_reason, invalid_layerstate_argument_reason,
    invalid_laywalk_argument_reason, invalid_ltscale_argument_reason,
    invalid_tcase_argument_reason, invalid_tcircle_argument_reason, invalid_tcount_argument_reason,
    invalid_textalign_argument_reason, invalid_torient_argument_reason,
    invalid_tscale_argument_reason, invalid_zoom_argument_reason, normalize_command,
    parse_justifytext_argument, parse_layerpmode_argument, parse_layerstate_argument,
    parse_laywalk_argument, parse_ltscale_argument, parse_tcase_argument, parse_tcircle_argument,
    parse_tcount_argument, parse_textalign_argument, parse_torient_argument, parse_tscale_argument,
    parse_zoom_argument, typed_command_argument, unsupported_command_reason, unsupported_status,
    BytesDocument, CadActiveSpace, CadBlockStatus, CadCamera, CadCommandLogEntry,
    CadCommandLogKind, CadGroupMember, CadGroupStatus, CadImageClipStatus, CadImageStatus,
    CadLayer, CadLayers, CadLayoutTab, CadMeasurementEcho, CadReferenceStatus, CadSelection,
    CadSelectionRow, CadSession, CadSettingKey, CadSettingValue, CadSettings, CadStatus,
    LayerStateOption, TorientArgument, ZoomArgument, ARGUMENT_SELECTOR, COMMAND_ALIASES,
    COMMAND_TABLE, SETTINGS_INVALID_PLOT_FORMAT, SETTINGS_INVALID_VALUE, SETTINGS_UNKNOWN_KEY,
};
pub use shell::{
    admit_command, bind_allowed, clear_host_error, note_host_error, projected_status,
    queue_cad_input, HostError, HostErrorKind, ShellState, CAD_COMMAND_CAP, CAD_INPUT_CAP,
};

/// The running UI crate version. Same value the About dialog reports and the
/// workspace `Cargo.toml` version (`env!("CARGO_PKG_VERSION")`).
///
/// Kept here (not in `session/mod.rs`, which is at the file-size limit) because
/// it is a property of the process serving the command, not of the wire session.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
