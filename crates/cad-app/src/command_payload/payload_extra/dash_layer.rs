//! The typed `-LAYER <option> …` write line: AutoCAD's command-line layer
//! editor (`-LAYER`, not the `LAYER` dialog).
//!
//! One variant per option; the list-valued variants carry the layer names
//! **exactly as the line split them** (comma-separated, trimmed, never
//! resolved). Names are resolved against the authoritative drawing by the
//! command layer with the one `resolve_layer` rule the layer panel also uses, so
//! there is no second resolver and no second name grammar.
//!
//! `COLOR`'s bare-ACI spelling (`COLOR 3 WALLS` means `aci:3`) is normalised by
//! the command layer (`crate::layer_property::normalize_color_input`), so this
//! payload carries the token as typed and the one [`crate::layer_property`]
//! colour grammar still parses every value.
//!
//! # Why this rides `CommandId::SetLayerProperty`
//!
//! `CommandId::Extended(ExtendedCommandId::DashLayer)` is the **read-only**
//! report and must stay Viewer-legal (`-LAYER` and `-LAYER ?`). Every option
//! below writes the drawing, so it needs the Work-mode authorisation the
//! existing layer-table writes already carry; `SetLayerProperty` is that
//! Work-only carrier. Routing through it means no new `CommandId` (the command
//! count and the vocabulary stay unchanged), no new authorization rule to keep
//! in sync, and the same single layer-table write path
//! (`app_drawing::dash_layer_write` → `cad-db`'s validated setters).

/// One typed `-LAYER` write line.
///
/// The variants are the documented command-line options. `NEW`/`DELETE`/`ON`/
/// `OFF`/`FREEZE`/`THAW`/`LOCK`/`UNLOCK` take a comma-separated name list;
/// `MAKE`/`SET` take one name (the trimmed remainder, spaces allowed);
/// `RENAME` takes the first whitespace-delimited token as the old name and the
/// trimmed remainder as the new one; `COLOR`/`LTYPE`/`LWEIGHT`/`PLOT` take one
/// value token followed by the name list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashLayerOps {
    /// `NEW n1[,n2...]` / `N`: create each named layer.
    New(Vec<String>),
    /// `MAKE name` / `M`: create the layer when it is missing, then make it the
    /// current layer.
    Make(String),
    /// `SET name` / `S`: make an existing layer the current layer.
    Set(String),
    /// `RENAME old new` / `R`: rename one layer.
    Rename { old: String, new: String },
    /// `DELETE n1[,n2...]` / `D`.
    Delete(Vec<String>),
    /// `ON n1[,n2...]`.
    On(Vec<String>),
    /// `OFF n1[,n2...]`.
    Off(Vec<String>),
    /// `FREEZE n1[,n2...]` / `F`.
    Freeze(Vec<String>),
    /// `THAW n1[,n2...]` / `T`.
    Thaw(Vec<String>),
    /// `LOCK n1[,n2...]` / `LO`.
    Lock(Vec<String>),
    /// `UNLOCK n1[,n2...]` / `U`.
    Unlock(Vec<String>),
    /// `COLOR value n1[,n2...]` / `C`: `value` follows the layer colour grammar
    /// (`aci:1..=255`, `#RRGGBB`, `rgb:#RRGGBB`, `r,g,b`); AutoCAD's bare ACI
    /// integer (`3`) is normalised to `aci:3` by the command layer before that
    /// one grammar parses it.
    Color { value: String, layers: Vec<String> },
    /// `LTYPE name n1[,n2...]` / `L`: `name` must be an existing linetype table
    /// entry.
    Linetype { name: String, layers: Vec<String> },
    /// `LWEIGHT value n1[,n2...]` / `LW`: the layer lineweight grammar
    /// (`ByLayer`, `Default` or a 1/100 mm integer `0..=211`).
    Lineweight { value: String, layers: Vec<String> },
    /// `PLOT Y|N n1[,n2...]` / `P`: the plottable flag.
    Plot {
        plottable: bool,
        layers: Vec<String>,
    },
}
