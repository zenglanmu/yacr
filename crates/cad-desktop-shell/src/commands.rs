//! Tauri IPC commands shared by every desktop host.
//!
//! These are thin `#[tauri::command]` wrappers. The **semantics** — wire argument
//! parsing, the modal/probe-mode gates, the queue bounds, every refusal string and
//! the `{accepted:true}` acknowledgement — live in `cad_ui_core::session::wire`
//! (one function per command) on top of `cad_ui_core::session::admission` and
//! `cad_ui_core::shell`; this module only moves the Tauri arguments into those
//! functions and hands them [`Shared::queue_sink`] (the bounded-queue sink) or
//! [`Shared::queue_reads`] (the owner-tick mirror). The same `wire::*` functions
//! are what the `dom-canvas` (direct wasm) host calls, so the two hosts cannot
//! answer differently for the same request.
//!
//! The renderer telemetry that `cad_status`/`probe_input` used to read directly
//! is behind [`ShellTelemetry`]; no `cad-render-wgpu`/`cad-engine` type is named
//! here.
//!
//! ## The queued shape
//!
//! `CadSession` is `!Send` and owned by the host main thread, so nothing here
//! touches it: an admitted request is pushed onto a bounded queue and the owner
//! tick applies it. `{accepted:true}` therefore means **queued**, never
//! completed; the real outcome (including a session refusal) surfaces through
//! `cad_status.error`. The reads (`cad_settings`, `cad_layers`, `cad_fonts`)
//! answer from the tick's mirror for the same reason.
//!
//! ## Registration
//!
//! [`register_commands`] is the single `generate_handler!` list and
//! [`COMMAND_NAMES`] is the same set, both generated from
//! `cad_ui_core::session::registry`. The three desktop-only commands
//! (`probe_status`, `probe_input`, `cad_status`) stay handwritten because their
//! bodies read host state that the shared registry deliberately does not name.

use std::sync::{Arc, Mutex};

use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{
    wire, CadFonts, CadLayers, CadRecentFiles, CadRecoveryFiles, CadSettings, CadYmodelResources,
    HostProfile, WireStatus,
};
use cad_ui_core::shell::{modal_gate_open, queue_cad_input, POPUP_MODAL_OPEN};

use crate::popup::DesktopPopupTarget;
use crate::state::{lock, ProbeShared, Shared};

// Re-exported for the evidence driver, which must queue through the **same**
// selected-text admission `cad_set_selected_text` runs.
pub use cad_ui_core::session::admit_selected_text;
// Re-exported for the owner tick, which drains the queue with the shared rule.
pub use cad_ui_core::session::drain_standard_views;
// Re-exported for the owner tick, which drains the ViewCube orbit queue with the
// shared rule.
pub use cad_ui_core::session::drain_orbits;
// Re-exported for the owner tick, which drains the layer-write queue with the
// shared rule (the React layer panel's three mutations).
pub use cad_ui_core::session::drain_layer_writes;
// The wire shapes moved into `cad_ui_core::session::wire` so both hosts answer
// the identical JSON; these aliases keep every existing import path working.
pub use cad_ui_core::session::wire::{parse_setting_request, CadCommandAck, CadCommandAlias};

// ---------------------------------------------------------------------------
// Generated transport: the shared registry rows become the typed wrappers,
// the `COMMAND_NAMES` constant and the one `generate_handler!` list.
// ---------------------------------------------------------------------------

/// Emit one typed `#[tauri::command]` wrapper per shared registry row.
macro_rules! tauri_wire_wrappers {
    ($sink:ident, $reads:ident, $popups:ident; $($rows:tt)*) => {
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rows)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;) => {};
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, sink, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let mut state = lock(&shared.0);
            let mut queue = state.queue_sink();
            let $sink = &mut queue;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, reads, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let state = lock(&shared.0);
            let queue_reads = state.queue_reads();
            let $reads = &queue_reads;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, popups, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let mut host_popups = DesktopPopupTarget::new(&shared.0)?;
            let $popups = &mut host_popups;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, pure, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            _shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row($($other:tt)*) $($rest:tt)*) => {
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
}

/// Emit `COMMAND_NAMES` from every registry row, in registration order.
macro_rules! tauri_wire_command_names {
    (@munch [$($names:tt)*]) => {
        /// The registered command set, in the order `generate_handler!` lists it.
        ///
        /// Both this constant and the handler list are generated from
        /// `cad_ui_core::session::registry`; the `registration` test pins the set
        /// against the pre-migration fixture.
        pub const COMMAND_NAMES: &[&str] = &[$($names)*];
    };
    (@munch [$($names:tt)*] @row($kind:ident $($extra:ident)*, $lane:ident, $args:ident, $row_name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        tauri_wire_command_names!(@munch [$($names)* stringify!($row_name),] $($rest)*);
    };
    ($($rows:tt)*) => {
        tauri_wire_command_names!(@munch [] $($rows)*);
    };
}

/// Emit the one `generate_handler!` list from every registry row.
macro_rules! tauri_wire_register {
    (@munch [$($names:ident,)*]) => {
        /// Build the `invoke_handler` for the desktop hosts.
        ///
        /// This is the only `generate_handler!` invocation; the list and the
        /// typed wrappers are both generated from the shared registry.
        pub fn register_commands<R: tauri::Runtime>(
        ) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($names),*]
        }
    };
    (@munch [$($names:ident,)*] @row($kind:ident $($extra:ident)*, $lane:ident, $args:ident, $row_name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        tauri_wire_register!(@munch [$($names,)* $row_name,] $($rest)*);
    };
    ($($rows:tt)*) => {
        tauri_wire_register!(@munch [] $($rows)*);
    };
}

/// The desktop emitter over the shared registry.
macro_rules! emit_tauri_wire_registry {
    (@context($sink:ident, $reads:ident, $popups:ident, $name:ident, $args_json:ident) $($rows:tt)*) => {
        tauri_wire_wrappers!($sink, $reads, $popups; $($rows)*);
        tauri_wire_command_names!($($rows)*);
        tauri_wire_register!($($rows)*);
    };
}

// ---------------------------------------------------------------------------
// Desktop-only commands: status and probe use host state the registry does not
// name, so their wrappers (and bodies) are handwritten.
// ---------------------------------------------------------------------------

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    probe_status_impl(&shared.0)
}

/// Command body of [`probe_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn probe_status_impl(shared: &Arc<Mutex<Shared>>) -> ProbeStatus {
    lock(shared).shell.status.clone()
}

/// Command body of `cad_settings`, without the Tauri state wrapper.
///
/// Split out so the wire-shape tests read the **same** snapshot the IPC path
/// serves (no duplicated projection in tests).
pub fn cad_settings_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadSettings, String> {
    wire::settings(&lock(shared).queue_reads())
}

/// Command body of `cad_set_setting`, without the Tauri state wrapper.
///
/// Split out so the wire-shape tests drive the **same** parse-and-admit path the
/// IPC command runs (never a re-implemented copy in the test).
pub fn cad_set_setting_impl(
    shared: &Arc<Mutex<Shared>>,
    key: &str,
    value: &serde_json::Value,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::set_setting(&mut state.queue_sink(), key, value)
}

/// Command body of `cad_layers`, without the Tauri state wrapper.
///
/// Split out so the wire-shape/golden tests can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_layers_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadLayers, String> {
    wire::layers(&lock(shared).queue_reads())
}

/// Command body of `cad_fonts`, without the Tauri state wrapper.
///
/// Split out so the wire test can call the **same** implementation the IPC path
/// runs; the answer is the owner-tick mirror, never a read of the `!Send`
/// session.
pub fn cad_fonts_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadFonts, String> {
    wire::fonts(&lock(shared).queue_reads())
}

/// Command body of `cad_bim_query`, without the Tauri state wrapper.
///
/// A **request-carrying** read (ADR-0042 D11): the filter and page window come
/// from the caller, so the answer is served by the host's
/// [`cad_ui_core::bim::BimHost`] seam (the session is `!Send` and owned by the
/// owner thread). A host that installs no seam keeps the shared
/// `unsupported-runtime: bimQuery` refusal — never an empty page.
pub fn cad_bim_query_impl(
    shared: &Arc<Mutex<Shared>>,
    filter: cad_ui_core::bim::BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<cad_ui_core::bim::BimQueryResponse, String> {
    cad_ui_core::session::bim_reads::bim_query(&lock(shared).queue_reads(), filter, offset, limit)
}

/// Command body of `cad_bim_summary`, without the Tauri state wrapper.
///
/// Same seam and same refusal as [`cad_bim_query_impl`]; a summary is one
/// logical row, so the host answers the whole count.
pub fn cad_bim_summary_impl(
    shared: &Arc<Mutex<Shared>>,
    filter: cad_ui_core::bim::BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<cad_ui_core::bim::BimSummaryResponse, String> {
    cad_ui_core::session::bim_reads::bim_summary(&lock(shared).queue_reads(), filter, offset, limit)
}

/// `cad_recent_files` command: the host's real recent-drawings list.
///
/// A host file capability, not CAD document data, so the body lives here and is
/// served by the host's [`crate::state::ShellRecentHost`] seam (the session is
/// `!Send` and owned by the owner thread). A host without the seam refuses with
/// the same stable marker the browser adapter uses
/// (`unsupported-runtime: cad_recent_files`); it never answers an empty list
/// that would read as "nothing was ever opened".
#[tauri::command]
pub fn cad_recent_files(shared: tauri::State<'_, ProbeShared>) -> Result<CadRecentFiles, String> {
    cad_recent_files_impl(&shared.0)
}

/// Command body of [`cad_recent_files`], without the Tauri state wrapper.
pub fn cad_recent_files_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadRecentFiles, String> {
    // Clone the seam and release the shell lock before the (possibly blocking)
    // main-thread hop, per the `Shared` lock-order contract.
    let host = lock(shared).recent_host.clone();
    match host {
        Some(host) => host.list(),
        None => Err(format!(
            "unsupported-runtime: {}",
            cad_ui_core::session::RECENT_FILES_COMMAND
        )),
    }
}

/// `cad_open_recent` command: start reopening one recorded recent drawing.
///
/// Arguments are exactly `{ opaque, displayName }`. The host revalidates the
/// opaque handle against its own durable list (a crafted handle is refused, not
/// opened) and starts the ordinary platform open; a real failure (missing file,
/// revoked grant) rejects unchanged. Resolving means the platform open was
/// **started**; the document outcome still arrives through `cad_status`.
#[tauri::command]
pub fn cad_open_recent(
    shared: tauri::State<'_, ProbeShared>,
    opaque: String,
    display_name: String,
) -> Result<CadCommandAck, String> {
    cad_open_recent_impl(&shared.0, &opaque, &display_name)
}

/// Command body of [`cad_open_recent`], without the Tauri state wrapper.
pub fn cad_open_recent_impl(
    shared: &Arc<Mutex<Shared>>,
    opaque: &str,
    display_name: &str,
) -> Result<CadCommandAck, String> {
    let host = lock(shared).recent_host.clone();
    let Some(host) = host else {
        return Err(format!(
            "unsupported-runtime: {}",
            cad_ui_core::session::OPEN_RECENT_COMMAND
        ));
    };
    if opaque.trim().is_empty() {
        return Err(
            "recent.invalid_payload: the recent row has no file handle to reopen".to_string(),
        );
    }
    host.open_recent(opaque, display_name)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_recovery_files` command: the recovery snapshots this carrier can open.
///
/// A host file capability like the recent list, served by the host's
/// [`crate::state::ShellRecoveryHost`] seam because the store lives on the
/// `!Send` owner-thread session. A host without the seam refuses with the stable
/// marker (`unsupported-runtime: cad_recovery_files`); it never answers an empty
/// list that would read as "nothing to recover".
#[tauri::command]
pub fn cad_recovery_files(
    shared: tauri::State<'_, ProbeShared>,
) -> Result<CadRecoveryFiles, String> {
    cad_recovery_files_impl(&shared.0)
}

/// Command body of [`cad_recovery_files`], without the Tauri state wrapper.
pub fn cad_recovery_files_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadRecoveryFiles, String> {
    // Clone the seam and release the shell lock before the (possibly blocking)
    // main-thread hop, per the `Shared` lock-order contract.
    let host = lock(shared).recovery_host.clone();
    match host {
        Some(host) => host.list(),
        None => Err(format!(
            "unsupported-runtime: {}",
            cad_ui_core::session::RECOVERY_FILES_COMMAND
        )),
    }
}

/// `cad_open_recovery` command: start opening one listed recovery snapshot.
///
/// The argument is exactly `{ key }` — the store's hex key, never a path. The
/// host validates it and resolves the location from its own store, so a crafted
/// key is refused rather than opened; a real failure (missing archive) rejects
/// unchanged. Resolving means the platform open was **started**; the document
/// outcome still arrives through `cad_status`.
#[tauri::command]
pub fn cad_open_recovery(
    shared: tauri::State<'_, ProbeShared>,
    key: String,
) -> Result<CadCommandAck, String> {
    cad_open_recovery_impl(&shared.0, &key)
}

/// Command body of [`cad_open_recovery`], without the Tauri state wrapper.
pub fn cad_open_recovery_impl(
    shared: &Arc<Mutex<Shared>>,
    key: &str,
) -> Result<CadCommandAck, String> {
    let host = lock(shared).recovery_host.clone();
    let Some(host) = host else {
        return Err(format!(
            "unsupported-runtime: {}",
            cad_ui_core::session::OPEN_RECOVERY_COMMAND
        ));
    };
    if key.trim().is_empty() {
        return Err("recovery.invalid_payload: the recovery row has no key to open".to_string());
    }
    host.open_recovery(key)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_ymodel_resource_preview` command: what a `.ymodel` save would embed.
///
/// Read-only (ADR-0040 D17): it lists the resources the open document references
/// with their size and whether the host can read them, plus the document's
/// current self-containment flag. Nothing is written, so a cancelled save dialog
/// leaves no trace. The session is `!Send` and owned by the owner thread, so the
/// read is marshalled through the host's [`crate::state::ShellYmodelHost`] seam.
///
/// A carrier that installs no `.ymodel` archive has no seam: the answer is the
/// stable `unsupported-runtime: ymodel`, never an empty preview that would read as
/// "nothing to embed".
#[tauri::command]
pub fn cad_ymodel_resource_preview(
    shared: tauri::State<'_, ProbeShared>,
) -> Result<CadYmodelResources, String> {
    cad_ymodel_resource_preview_impl(&shared.0)
}

/// Command body of [`cad_ymodel_resource_preview`], without the Tauri wrapper.
pub fn cad_ymodel_resource_preview_impl(
    shared: &Arc<Mutex<Shared>>,
) -> Result<CadYmodelResources, String> {
    // Clone the seam and release the shell lock before the (possibly blocking)
    // main-thread hop, per the `Shared` lock-order contract.
    let host = lock(shared).ymodel_host.clone();
    match host {
        Some(host) => host.preview(),
        None => Err(cad_ui_core::session::UNSUPPORTED_YMODEL.to_string()),
    }
}

/// `cad_set_embed_resources` command: store the save dialog's choice on the
/// document (ADR-0040 D3).
///
/// Admission only: `{accepted:true}` means **queued**, exactly like every other
/// typed write on this transport. The owner tick drains it into
/// `CadSession::set_embed_resources`, so a refusal (a document write that cannot
/// land) surfaces through `cad_status.error` rather than a fabricated success.
///
/// A carrier with no `.ymodel` archive has no seam and refuses here, before the
/// queue: accepting the choice would store a flag no file on that carrier can
/// carry.
#[tauri::command]
pub fn cad_set_embed_resources(
    shared: tauri::State<'_, ProbeShared>,
    embed: bool,
) -> Result<CadCommandAck, String> {
    cad_set_embed_resources_impl(&shared.0, embed)
}

/// Command body of [`cad_set_embed_resources`], without the Tauri wrapper.
pub fn cad_set_embed_resources_impl(
    shared: &Arc<Mutex<Shared>>,
    embed: bool,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    if state.ymodel_host.is_none() {
        return Err(cad_ui_core::session::UNSUPPORTED_EMBED_RESOURCES.to_string());
    }
    if !state.shell.cad_mode {
        return Err("CAD is disabled in probe mode".to_string());
    }
    // Last wins: the tick applies the newest choice once (`Option`, not a queue).
    state.cad_embed_resources = Some(embed);
    Ok(CadCommandAck::accepted())
}

/// `cad_pick_directory` command: open the carrier's folder chooser.
///
/// The Options > Files page asks for one local absolute directory. The reply is
/// the chosen path, or `null` when the user closed the dialog without choosing.
///
/// The command does **not** run the chooser on the calling thread: Tauri commands
/// run on the GTK main thread here, and an open portal dialog would freeze the
/// frame loop. [`DirectoryPicker::begin`](crate::directory::DirectoryPicker::begin)
/// admits one pick and starts its worker; the owner tick drains the pending slot
/// (`state::drain_directory_pick`), and this async body waits for that answer on a
/// blocking-pool thread. A chooser failure is the explicit
/// `pick_directory.failed: …` error, never a `null` that reads as a cancellation.
///
/// A carrier that installs no picker (`dom-canvas`, and the native carriers that
/// have not wired one yet) refuses with `unsupported-runtime: pick_directory`.
#[tauri::command]
pub async fn cad_pick_directory(
    shared: tauri::State<'_, ProbeShared>,
) -> Result<Option<String>, String> {
    let pending = cad_pick_directory_impl(&shared.0)?;
    tauri::async_runtime::spawn_blocking(move || match pending.recv() {
        Ok(answer) => answer,
        Err(_) => Err(cad_ui_core::session::PICK_DIRECTORY_LOST.to_string()),
    })
    .await
    .map_err(|error| cad_ui_core::session::pick_directory_failed(&error.to_string()))?
}

/// Command body of [`cad_pick_directory`], without the Tauri wrapper or the await.
///
/// Split out so a test can assert the admission and the no-picker refusal without
/// running an async runtime.
pub fn cad_pick_directory_impl(
    shared: &Arc<Mutex<Shared>>,
) -> Result<crate::directory::DirectoryPickReceiver, String> {
    // Clone the seam and release the shell lock before starting the worker, per
    // the `Shared` lock-order contract.
    let picker = lock(shared).directory_picker.clone();
    match picker {
        Some(picker) => picker.begin(),
        None => Err(cad_ui_core::session::UNSUPPORTED_PICK_DIRECTORY.to_string()),
    }
}

/// Command body of `cad_create_layer`, without the Tauri state wrapper.
pub fn cad_create_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::create_layer(&mut state.queue_sink(), &name)
}

/// Command body of `cad_rename_layer`, without the Tauri state wrapper.
pub fn cad_rename_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::rename_layer(&mut state.queue_sink(), &layer, &name)
}

/// Command body of `cad_delete_layer`, without the Tauri state wrapper.
pub fn cad_delete_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::delete_layer(&mut state.queue_sink(), &layer)
}

/// Command body of `cad_set_layer_property`, without the Tauri state wrapper.
pub fn cad_set_layer_property_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::set_layer_property(&mut state.queue_sink(), &layer, &key, &value)
}

/// Command body of `cad_command_aliases`, without the Tauri command wrapper.
pub fn cad_command_aliases_impl() -> Vec<CadCommandAlias> {
    wire::aliases()
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in and the carrier described by one typed
/// `hostProfile` object.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
///
/// The envelope is [`WireStatus`]: the frozen [`cad_ui_core::session::CadStatus`]
/// fields (unchanged, same order) followed by the always-present `hostProfile`.
/// On this carrier the profile is [`HostProfile::native_below`] (native document
/// owner, palettes served, standard views undeclared) and there is no
/// `canvasSession` key. The old "carrier signalled by the presence/absence of
/// `surfaceMode`" story is dead: every carrier now names itself.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> WireStatus<'static> {
    cad_status_wire_impl(&shared.0)
}

/// Command body of [`cad_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_status_impl(shared: &Arc<Mutex<Shared>>) -> cad_ui_core::session::CadStatus {
    // One projection for both paths: the host error and the live popup are
    // applied by `events::project_current_status`, the same function the per-tick
    // event flush calls, so `cad_status` and `status-changed` cannot diverge.
    let (status, telemetry) = {
        let state = lock(shared);
        (
            crate::events::project_current_status(&state),
            state.telemetry.clone(),
        )
    };
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    // The pulled organisation state is appended as a separate token (the JSON is
    // unchanged and stays first) so a skipped accelerated organisation is visible
    // next to the status payload.
    //
    // Lock order: the shell guard is released before the telemetry seam runs, so
    // the app-side implementation only ever locks the telemetry mutex (it never
    // re-enters `Shared`).
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        let pulled = telemetry
            .as_ref()
            .map(|telemetry| telemetry.cad_status_evidence())
            .unwrap_or_else(|| "-".to_string());
        match serde_json::to_string(&status) {
            Ok(json) => eprintln!("[probe] cad_status {json} pulled={pulled}"),
            Err(error) => eprintln!("[probe] cad_status serialize failed: {error} pulled={pulled}"),
        }
    }
    status
}

/// The exact `cad_status` wire object: the shared projection plus this carrier's
/// typed [`HostProfile`], with no `canvasSession` (native sessions report their
/// document through the shared snapshot itself).
///
/// Split out so the wire goldens serialize the **same** envelope the IPC command
/// returns; a native host cannot return references to its locally built values,
/// so the envelope owns them ([`WireStatus::owned`]).
pub fn cad_status_wire_impl(shared: &Arc<Mutex<Shared>>) -> WireStatus<'static> {
    WireStatus::owned(
        cad_status_impl(shared),
        HostProfile::native_below(),
        None::<serde_json::Value>,
    )
}

/// `probe_input` command: sequence-gate one frontend input and route it.
///
/// In CAD mode a pointer/wheel input is queued for the main thread only when the
/// CAD session is ready; before that it is refused with an explicit `cad_waiting`
/// reason (never a pretend success), and a full queue is refused explicitly.
/// `resize` is applied by the tick through the CAD rect path.
#[tauri::command]
pub fn probe_input(shared: tauri::State<'_, ProbeShared>, input: ProbeInput) -> ProbeInputAck {
    probe_input_impl(&shared.0, input)
}

/// Whether the diagnostic input line is enabled.
///
/// Shares the host's existing evidence gate (`YACR_PROBE_CAD_LOG`) rather than
/// adding a second switch: evidence drivers already set it, and normal sessions
/// keep their stderr free of one line per pointer move.
fn input_trace_enabled() -> bool {
    crate::probe_log::evidence_log_enabled()
}

/// Command body of [`probe_input`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated gate/queue logic in tests).
pub fn probe_input_impl(shared: &Arc<Mutex<Shared>>, input: ProbeInput) -> ProbeInputAck {
    let kind = input.kind;
    let sequence = input.sequence;
    // Modal popup gate (ADR-0021 D2): while a popup is open, every interaction
    // input is refused by name before the tracker sees it. `resize` still passes
    // because it carries no CAD intent and keeps the layout truthful while the
    // modal is up. This gate is host-owned, so the command line's blur path can
    // never release it.
    if !matches!(kind, InputKind::Resize) {
        let refused = {
            let state = lock(shared);
            modal_gate_open(&state.shell).then(|| state.shell.input.marker_offset())
        };
        if let Some(marker_offset) = refused {
            if input_trace_enabled() {
                eprintln!(
                    "[probe] input kind={} seq={sequence} accepted=false reason={POPUP_MODAL_OPEN}",
                    kind.as_str()
                );
            }
            return ProbeInputAck {
                accepted: false,
                sequence,
                marker_offset,
                terminal: kind.is_terminal(),
                reason: Some(POPUP_MODAL_OPEN.to_string()),
            };
        }
    }
    let ack = lock(shared).shell.input.apply(&input);
    // Evidence line: shows the real IPC path (invoke -> command -> tracker).
    // x/y/dx/dy/width are logged so the accepted input can be tied to real OS
    // coordinates (diagnosis only; identifiers stay English). Gated exactly like
    // the other probe evidence: a pointer move arrives at up to display rate, so
    // an always-on `eprintln!` is both log spam and avoidable main/IPC-thread work
    // in normal use. Evidence runs already set `YACR_PROBE_CAD_LOG=1`.
    if input_trace_enabled() {
        eprintln!(
            "[probe] input kind={} seq={} accepted={} marker={:.3} terminal={} reason={:?} x={:?} y={:?} dx={:?} dy={:?} width={:?}",
            kind.as_str(),
            sequence,
            ack.accepted,
            ack.marker_offset,
            ack.terminal,
            ack.reason,
            input.x,
            input.y,
            input.dx,
            input.dy,
            input.width,
        );
    }
    if !ack.accepted {
        return ack;
    }
    // Post-load pan driver accounting: count host-admitted gesture inputs, and
    // flag a concurrent resize (which would make any view change unattributable
    // to the pan). The app-side implementation locks only the telemetry mutex.
    let telemetry = lock(shared).telemetry.clone();
    if let Some(telemetry) = telemetry {
        telemetry.note_input_accepted(kind);
    }
    if matches!(kind, InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(shared);
        queue_cad_input(&mut state.shell, &input)
    };
    match queued {
        Ok(()) => ack,
        Err(reason) => ProbeInputAck {
            accepted: false,
            sequence,
            marker_offset: ack.marker_offset,
            terminal: ack.terminal,
            reason: Some(reason),
        },
    }
}

// The shared wrappers, `COMMAND_NAMES` and `register_commands` are generated
// from the one registry table. This is the only desktop emitter invocation.
cad_ui_core::cad_wire_registry!(emit_tauri_wire_registry);

#[cfg(test)]
mod tests {
    use super::*;
    use cad_ui_core::session::{CadRecoveryFiles, RecoveryEntry};
    use std::sync::Mutex;

    fn shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared::new(
            cad_ui_core::launch::LaunchOptions::default(),
        )))
    }

    /// A seam that answers one empty supported list and records the keys it is
    /// asked to open.
    #[derive(Default)]
    struct RecordingRecovery {
        opened: Mutex<Vec<String>>,
    }

    impl crate::state::ShellRecoveryHost for RecordingRecovery {
        fn list(&self) -> Result<CadRecoveryFiles, String> {
            Ok(CadRecoveryFiles::supported(Vec::new()))
        }

        fn open_recovery(&self, key: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(key.to_string());
            Ok(())
        }
    }

    #[test]
    fn the_recovery_commands_refuse_by_name_without_a_seam() {
        // A host with no seam has no recovery capability. Both commands keep the
        // stable marker instead of an empty list / a silent acceptance.
        let shared = shared();
        assert_eq!(
            cad_recovery_files_impl(&shared).expect_err("no seam must refuse"),
            "unsupported-runtime: cad_recovery_files"
        );
        for key in ["0123456789abcdef", ""] {
            assert_eq!(
                cad_open_recovery_impl(&shared, key).expect_err("no seam must refuse"),
                "unsupported-runtime: cad_open_recovery",
                "{key:?}"
            );
        }
        // The names come from the shared wire module, so the refusal and the
        // registered command can never drift apart.
        assert_eq!(
            cad_ui_core::session::RECOVERY_FILES_COMMAND,
            "cad_recovery_files"
        );
        assert_eq!(
            cad_ui_core::session::OPEN_RECOVERY_COMMAND,
            "cad_open_recovery"
        );
    }

    #[test]
    fn a_configured_recovery_seam_serves_the_list_and_the_key_only() {
        let shared = shared();
        let seam = Arc::new(RecordingRecovery::default());
        crate::state::configure_recovery_host(&shared, seam.clone());

        let answer = cad_recovery_files_impl(&shared).expect("the seam answers");
        assert_eq!(
            answer,
            CadRecoveryFiles::supported(Vec::new()),
            "empty is the seam's real answer"
        );

        // An empty key is refused before the seam is called.
        let error = cad_open_recovery_impl(&shared, "   ").expect_err("empty key");
        assert!(error.starts_with("recovery.invalid_payload:"), "{error}");
        assert!(seam.opened.lock().unwrap().is_empty());

        let ack = cad_open_recovery_impl(&shared, "0123456789abcdef").expect("admitted");
        assert!(ack.accepted, "admission only; the open is in flight");
        assert_eq!(
            seam.opened.lock().unwrap().clone(),
            vec!["0123456789abcdef".to_string()],
            "the seam receives the key, not a path"
        );

        // Pin the row shape the seam returns so the UI parser's contract is
        // explicit here too.
        let row = RecoveryEntry {
            key: "0123456789abcdef".to_string(),
            display_name: None,
            revision: 3,
            saved_at_ms: 0,
        };
        assert_eq!(
            serde_json::to_value(&row).expect("json"),
            serde_json::json!({
                "key": "0123456789abcdef",
                "displayName": null,
                "revision": 3,
                "savedAtMs": 0,
            })
        );
    }

    /// A seam that answers one fixed preview and counts the reads.
    #[derive(Default)]
    struct RecordingYmodel {
        previews: Mutex<usize>,
    }

    impl crate::state::ShellYmodelHost for RecordingYmodel {
        fn preview(&self) -> Result<CadYmodelResources, String> {
            *self.previews.lock().unwrap() += 1;
            Ok(CadYmodelResources {
                policy: true,
                entries: vec![cad_ui_core::session::CadYmodelResource {
                    kind: "font".to_string(),
                    name: "arial.ttf".to_string(),
                    source: Some("/fonts/arial.ttf".to_string()),
                    bytes: 12,
                    found: true,
                }],
            })
        }
    }

    #[test]
    fn the_ymodel_commands_refuse_by_marker_without_a_seam() {
        // A carrier with no `.ymodel` archive installs no seam: neither command
        // may pretend a preview exists or queue a choice no file can carry.
        let shared = shared();
        assert_eq!(
            cad_ymodel_resource_preview_impl(&shared).expect_err("no seam must refuse"),
            cad_ui_core::session::UNSUPPORTED_YMODEL
        );
        assert_eq!(
            cad_set_embed_resources_impl(&shared, true).expect_err("no seam must refuse"),
            cad_ui_core::session::UNSUPPORTED_EMBED_RESOURCES
        );
        assert!(
            lock(&shared).cad_embed_resources.is_none(),
            "a refused choice is never queued"
        );
        assert_eq!(
            cad_ui_core::session::YMODEL_PREVIEW_COMMAND,
            "cad_ymodel_resource_preview"
        );
        assert_eq!(
            cad_ui_core::session::EMBED_RESOURCES_COMMAND,
            "cad_set_embed_resources"
        );
    }

    #[test]
    fn a_configured_ymodel_seam_serves_the_preview_and_queues_the_choice() {
        let shared = shared();
        let seam = Arc::new(RecordingYmodel::default());
        crate::state::configure_ymodel_host(&shared, seam.clone());

        let preview = cad_ymodel_resource_preview_impl(&shared).expect("the seam answers");
        assert!(preview.policy);
        assert_eq!(preview.entries.len(), 1);
        assert_eq!(preview.entries[0].kind, "font");
        assert_eq!(preview.entries[0].bytes, 12);
        assert!(preview.entries[0].found);
        assert_eq!(*seam.previews.lock().unwrap(), 1);

        // The setter is admission only: the choice is queued for the owner tick,
        // and the newest choice wins (a scalar, not a queue).
        let ack = cad_set_embed_resources_impl(&shared, true).expect("admitted");
        assert!(ack.accepted);
        assert_eq!(lock(&shared).cad_embed_resources, Some(true));
        cad_set_embed_resources_impl(&shared, false).expect("admitted");
        assert_eq!(lock(&shared).cad_embed_resources, Some(false));
    }

    #[test]
    fn the_embed_choice_is_never_queued_in_probe_mode() {
        let mut state = Shared::new(cad_ui_core::launch::LaunchOptions {
            probe_only: true,
            ..cad_ui_core::launch::LaunchOptions::default()
        });
        state.ymodel_host = Some(Arc::new(RecordingYmodel::default()));
        assert!(!state.shell.cad_mode);
        let shared = Arc::new(Mutex::new(state));
        assert_eq!(
            cad_set_embed_resources_impl(&shared, true).expect_err("probe mode refuses"),
            "CAD is disabled in probe mode"
        );
        assert!(lock(&shared).cad_embed_resources.is_none());
    }

    #[test]
    fn the_directory_picker_refuses_by_marker_without_a_picker() {
        // Windows/macOS/Android install no picker in this pass, and `dom-canvas`
        // never registers one: the command must say so by name, never answer
        // `null` (which the Options footer would read as "the user cancelled").
        let shared = shared();
        assert_eq!(
            cad_pick_directory_impl(&shared).expect_err("no picker must refuse"),
            "unsupported-runtime: pick_directory"
        );
        assert_eq!(
            cad_ui_core::session::PICK_DIRECTORY_COMMAND,
            "cad_pick_directory"
        );
    }

    #[test]
    fn a_configured_picker_admits_one_pick_and_the_tick_delivers_it() {
        let shared = shared();
        let picker = crate::directory::DirectoryPicker::new(Arc::new(|| {
            crate::directory::DirectoryOutcome::Cancelled
        }));
        crate::state::configure_directory_picker(&shared, picker);

        let answer = cad_pick_directory_impl(&shared).expect("admitted");
        // The pending slot is drained by the owner tick's helper, exactly like the
        // app-linux tick calls it; only then does the waiting call resolve.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut delivered = false;
        while std::time::Instant::now() < deadline {
            crate::state::drain_directory_pick(&shared);
            if let Ok(value) = answer.try_recv() {
                assert_eq!(value, Ok(None), "a cancelled dialog is a real null");
                delivered = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(delivered, "the tick must deliver the terminal");
    }
}
