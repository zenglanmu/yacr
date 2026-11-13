//! **The one wire-command registry**: names, argument shapes, reply types and the
//! `wire::*` call each transport serves.
//!
//! # Why this file exists
//!
//! Before this table there were three hand-maintained mirrors of the same command
//! set: the desktop `#[tauri::command]` list + [`COMMAND_NAMES`], the `dom-canvas`
//! `dispatch` match + `WIRE_COMMANDS`, and a name list in the equivalence tests.
//! Adding or renaming a command meant editing all of them, and a miss produced a
//! runtime "command not found" instead of a compile error.
//!
//! The table below is the single authority. It is expanded by
//! [`cad_wire_registry!`] into whichever transport is asked for:
//!
//! * [`WIRE_COMMAND_NAMES`] (and its grouped slices) — the stable name set, in the
//!   order the desktop has always registered it;
//! * `apps/app-web` — one `#[derive(Deserialize)]` argument struct per command plus
//!   the `dispatch` match and `WIRE_COMMANDS`;
//! * `cad-desktop-shell` — the typed `#[tauri::command]` wrappers, the
//!   `COMMAND_NAMES` constant and the `generate_handler!` list.
//!
//! This module is deliberately **tauri-free and host-type-free**: a row names only
//! `cad-ui-core` types and a `wire::*` expression. The host emitters live in the
//! crates that own the transport (and therefore the state type, the lock and the
//! popup seam).
//!
//! # Row shape
//!
//! ```text
//! @row(<groups>, <context lane>, <ArgsStruct>, <name>, { <fields> }, <Reply>, { <body> })
//! ```
//!
//! * **groups** — `shared` (served by both transports) or `tauri_only` (desktop
//!   only: `cad_status`, `probe_status`, `probe_input`, `cad_recent_files`,
//!   `cad_open_recent`); `wasm_export` marks a row
//!   that is *also* a direct wasm export (`probe_input`, `cad_status`). The wasm
//!   export list is informational for the generated name sets; those functions are
//!   declared next to the wasm binding, because only that crate knows the export
//!   signature.
//! * **context lane** — `sink` (admission target), `reads` (settings/layers
//!   projection), `popups` (popup seam), or `pure` (no host state). The same
//!   identifier is bound by each emitter and used by the body, so the body has one
//!   spelling in both transports.
//! * **fields** — `arg(req, field, Type)` or `arg(opt, field, Option<T>)`; `opt`
//!   controls `#[serde(default)]` in the wasm argument struct.
//! * **Reply** — the *success* type. Every generated body returns
//!   `Result<Reply, String>`; the desktop wrapper keeps that typed result (no
//!   `serde_json::Value` round-trip), the wasm transport serializes exactly once
//!   into the envelope. Two pre-registry commands (`cad_commands`,
//!   `cad_command_aliases`) returned bare values before P2; the generated
//!   `Ok(...)` wrapper is wire-identical because Tauri serializes `Ok(payload)`
//!   exactly like a bare return, and the reject path is unreachable for a row
//!   whose body cannot fail.
//! * **body** — the `wire::*` call. This is the only place a shared wire command
//!   body is written for either transport. The five `tauri_only` rows carry an
//!   empty body: their implementations read host state (`Shared`, the probe
//!   tracker, the file capability) that this crate deliberately does not name, so
//!   those wrappers stay in the desktop emitter.

/// The JSON value type a generated argument struct uses.
///
/// Re-exported so the macro never requires a consuming crate to depend on
/// `serde_json` under a particular crate-root spelling.
#[doc(hidden)]
pub use serde_json::Value as RegistryJsonValue;

/// Expand the single wire-command table into an emitter macro.
///
/// `$emitter` receives the rows below as `@row(...)` token trees, in registration
/// order, and may generate any transport-specific code from them. The macro itself
/// knows nothing about Tauri, wasm or host state.
#[macro_export]
macro_rules! cad_wire_registry {
    ($emitter:ident) => {
        $emitter! {
            // The context names every emitter binds: the three host seams plus the
            // two transport locals. They live here (not in an emitter) so the row
            // bodies below and the generated bindings share one hygiene context.
            @context(sink, reads, popups, name, args_json)
            // --- desktop-only status/probe commands (not request-carrying wire
            // commands for the dom-canvas transport) ---------------------------
            @row(tauri_only, host, ProbeStatusArgs, probe_status, { }, $crate::protocol::ProbeStatus, { })
            @row(tauri_only wasm_export, host, ProbeInputArgs, probe_input, { arg(req, input, $crate::protocol::ProbeInput) }, $crate::protocol::ProbeInputAck, { })
            // --- shared commands, in the desktop's registration order ----------
            @row(shared, sink, CommandArgs, cad_command, { arg(req, command, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::command(sink, &command) })
            @row(shared, pure, NoArgs, cad_command_aliases, { }, ::std::vec::Vec<$crate::session::wire::CadCommandAlias>, { ::std::result::Result::Ok($crate::session::wire::aliases()) })
            @row(tauri_only wasm_export, host, NoArgs, cad_status, { }, $crate::session::WireStatus<'static>, { })
            // Host file capabilities: the recent-drawings list and a reopen of one
            // row. They are not drawing commands (absent from `cad_commands`) and
            // a browser carrier refuses them by name; both bodies stay in the
            // desktop emitter because they read the host's own file capability.
            @row(tauri_only, host, NoArgs, cad_recent_files, { }, $crate::session::CadRecentFiles, { })
            @row(tauri_only, host, OpenRecentArgs, cad_open_recent, { arg(req, opaque, ::std::string::String) arg(req, display_name, ::std::string::String) }, $crate::session::wire::CadCommandAck, { })
            @row(shared, pure, NoArgs, cad_commands, { }, ::std::vec::Vec<&'static str>, { ::std::result::Result::Ok($crate::session::wire::commands()) })
            @row(shared, pure, NoArgs, cad_app_version, { }, ::std::string::String, { ::std::result::Result::Ok($crate::app_version().to_string()) })
            @row(shared, pure, NoArgs, cad_api_status, { }, $crate::session::CadApiStatus, { $crate::session::wire::api_status() })
            // The plugin-API operations: reflected reads of the open document /
            // document list / database revision (or an explicit refusal).
            // `parameters` is the operation's argument object (optional).
            @row(shared, reads, ApiCallArgs, cad_api_call, { arg(req, namespace, String) arg(req, operation, String) arg(opt, parameters, ::std::option::Option<$crate::session::registry::RegistryJsonValue>) }, $crate::session::CadApiOutcome, { $crate::session::wire::api_call(reads, &namespace, &operation, parameters.as_ref()) })
            @row(shared, sink, ToolParameterArgs, cad_set_tool_parameter, { arg(req, parameter, String) arg(opt, number, ::std::option::Option<f64>) arg(opt, flag, ::std::option::Option<bool>) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_tool_parameter(sink, &parameter, number, flag) })
            @row(shared, sink, ToolTextArgs, cad_set_tool_text, { arg(req, text, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_tool_text(sink, &text) })
            @row(shared, sink, SelectedTextArgs, cad_set_selected_text, { arg(req, text, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_selected_text(sink, &text) })
            @row(shared, sink, PropertyArgs, cad_set_selected_property, { arg(req, key, String) arg(req, value, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_selected_property(sink, &key, &value) })
            @row(shared, sink, StandardViewArgs, cad_standard_view, { arg(req, view, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::standard_view(sink, &view) })
            @row(shared, popups, PopupArgs, cad_open_popup, { arg(req, kind, String) }, $crate::popup::PopupAck, { $crate::session::wire::open_popup(popups, &kind) })
            @row(shared, popups, NoArgs, cad_close_popup, { }, $crate::session::wire::CadCommandAck, { $crate::session::wire::close_popup(popups) })
            @row(shared, reads, NoArgs, cad_settings, { }, $crate::session::CadSettings, { $crate::session::wire::settings(reads) })
            @row(shared, sink, SettingArgs, cad_set_setting, { arg(req, key, String) arg(req, value, $crate::session::registry::RegistryJsonValue) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_setting(sink, &key, &value) })
            @row(shared, reads, NoArgs, cad_layers, { }, $crate::session::CadLayers, { $crate::session::wire::layers(reads) })
            @row(shared, reads, NoArgs, cad_fonts, { }, $crate::session::CadFonts, { $crate::session::wire::fonts(reads) })
            @row(shared, sink, LayerVisibleArgs, cad_set_layer_visible, { arg(req, name, String) arg(req, visible, bool) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_layer_visible(sink, &name, visible) })
            @row(shared, sink, NoArgs, cad_restore_layers, { }, $crate::session::wire::CadCommandAck, { $crate::session::wire::restore_layers(sink) })
            @row(shared, sink, ActiveLayerArgs, cad_set_active_layer, { arg(req, name, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_active_layer(sink, &name) })
            @row(shared, sink, CreateLayerArgs, cad_create_layer, { arg(req, name, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::create_layer(sink, &name) })
            @row(shared, sink, RenameLayerArgs, cad_rename_layer, { arg(req, layer, String) arg(req, name, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::rename_layer(sink, &layer, &name) })
            @row(shared, sink, DeleteLayerArgs, cad_delete_layer, { arg(req, layer, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::delete_layer(sink, &layer) })
            @row(shared, sink, LayerPropertyArgs, cad_set_layer_property, { arg(req, layer, String) arg(req, key, String) arg(req, value, String) }, $crate::session::wire::CadCommandAck, { $crate::session::wire::set_layer_property(sink, &layer, &key, &value) })
        }
    };
}

/// Collect the four generated name sets from the rows in one pass.
macro_rules! __emit_wire_lists {
    ([$($all:tt)*] [$($shared:tt)*] [$($tauri:tt)*] [$($wasm:tt)*]) => {
        /// Every registered wire command, in registration order.
        pub const WIRE_COMMAND_NAMES: &[&str] = &[$($all)*];
        /// The commands served by **both** transports (`dom-canvas` + desktop).
        pub const SHARED_WIRE_COMMAND_NAMES: &[&str] = &[$($shared)*];
        /// The commands only the desktop `#[tauri::command]` list serves.
        pub const TAURI_ONLY_WIRE_COMMAND_NAMES: &[&str] = &[$($tauri)*];
        /// The shared/desktop commands that are also direct wasm exports.
        pub const WASM_EXPORT_WIRE_COMMAND_NAMES: &[&str] = &[$($wasm)*];
    };
    ([$($all:tt)*] [$($shared:tt)*] [$($tauri:tt)*] [$($wasm:tt)*]
        @row(shared, $lane:ident, $args:ident, $name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        __emit_wire_lists!(
            [$($all)* stringify!($name),]
            [$($shared)* stringify!($name),]
            [$($tauri)*]
            [$($wasm)*]
            $($rest)*
        );
    };
    ([$($all:tt)*] [$($shared:tt)*] [$($tauri:tt)*] [$($wasm:tt)*]
        @row(tauri_only wasm_export, $lane:ident, $args:ident, $name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        __emit_wire_lists!(
            [$($all)* stringify!($name),]
            [$($shared)*]
            [$($tauri)* stringify!($name),]
            [$($wasm)* stringify!($name),]
            $($rest)*
        );
    };
    ([$($all:tt)*] [$($shared:tt)*] [$($tauri:tt)*] [$($wasm:tt)*]
        @row(tauri_only, $lane:ident, $args:ident, $name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        __emit_wire_lists!(
            [$($all)* stringify!($name),]
            [$($shared)*]
            [$($tauri)* stringify!($name),]
            [$($wasm)*]
            $($rest)*
        );
    };
}

/// Seed the accumulator and walk the rows (the context header is not a row).
macro_rules! __emit_registry_lists {
    (@context($($context:tt)*) $($rows:tt)*) => {
        __emit_wire_lists!([] [] [] [] $($rows)*);
    };
}

cad_wire_registry!(__emit_registry_lists);
