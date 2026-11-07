//! Typed input for the 2026-11-07 layout / viewport batch.
//!
//! Kept out of `command_payload.rs` (which is already at the file-size ceiling)
//! and out of the command enum for the same reason. Every variant is an
//! **input**, never behaviour: `crate::app_layout` validates it and either
//! reads the authoritative layout table / session state or returns the named
//! refusal documented in [`crate::command_id::layout`].

use cad_domain::Point3;

/// One typed layout / viewport command input.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutPayload {
    /// `LAYOUT LIST`: read the paper layouts in `LayoutId` order.
    List,
    /// `LAYOUT NEW <name>`: the name is validated, then the missing write API
    /// is reported (`layout.write_unsupported`).
    New { name: String },
    /// `LAYOUT RENAME <old> <new>`: both names are validated/resolved, then the
    /// missing write API is reported.
    Rename { old: String, new: String },
    /// `LAYOUT DELETE <name>`: unknown and last-layout refusals come first,
    /// then the missing write API is reported.
    Delete { name: String },
    /// `LAYOUT SET <name>`: resolve the paper layout and switch the session's
    /// active space (session state only).
    Set { name: String },
    /// `LAYOUTMERGE <source> <target>`: resolve both, refuse same/missing, then
    /// report `layoutmerge.write_unsupported`.
    Merge { source: String, target: String },
    /// `LAYOUTWIZARD <name> [<paper> <width> <height>]`: validate the name and
    /// paper numbers, then report `layoutwizard.write_unsupported` because no
    /// layout/page-setup write API exists.
    Wizard {
        name: String,
        paper: Option<PaperSetup>,
    },
    /// `VPORTS <rows> <cols>`: both counts are validated `>= 1`; a valid grid
    /// is then refused because no `PaperViewport` creation API exists.
    Viewports { rows: u32, cols: u32 },
    /// `VPCLIP RECT <minx> <miny> <maxx> <maxy>`: the rectangle is validated
    /// (`vpclip.degenerate_rect`) and then refused because there is no
    /// selected-viewport/clip write.
    ClipRect { min: Point3, max: Point3 },
    /// `VPSCALE <scale>`: a standard scale name or ratio; resolved against the
    /// drawing's scale table, then refused because there is no viewport scale
    /// write.
    Scale { name: String },
    /// `LAYVPI <layer>[,<layer>...]`: the names are parsed and validated, then
    /// refused because there is no per-viewport layer state.
    FreezeViewportLayers { layers: Vec<String> },
}

/// The optional page setup of `LAYOUTWIZARD <name> <paper> <width> <height>`.
///
/// `name` is the paper-size name the caller typed (for example `A4`); the
/// numeric fields are **positive finite** paper dimensions. This build can
/// **read** the imported [`cad_db::PlotSettingsRecord`] but has no page-setup
/// write API, so the parsed setup is only used to name the boundary in the
/// refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperSetup {
    pub name: String,
    pub width: f64,
    pub height: f64,
}
