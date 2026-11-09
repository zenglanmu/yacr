//! Typed input for the 2026-11-07 layout / viewport batch.
//!
//! Kept out of `command_payload.rs` (which is already at the file-size ceiling)
//! and out of the command enum for the same reason. Every variant is an
//! **input**, never behaviour: `crate::app_layout` validates it and then reads,
//! switches space, or performs the one-transaction layout/viewport write
//! documented in [`crate::command_id::layout`].

use cad_domain::Point3;

/// One typed layout / viewport command input.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutPayload {
    /// `LAYOUT LIST`: read the paper layouts in `LayoutId` order.
    List,
    /// `LAYOUT NEW <name>`: validate the name, create the layout row and record
    /// one reversible layout patch.
    New { name: String },
    /// `LAYOUT RENAME <old> <new>`: resolve the old name, validate the new one,
    /// rename the row and record one before/after layout patch.
    Rename { old: String, new: String },
    /// `LAYOUT DELETE <name>`: unknown and last-layout refusals come first, then
    /// the layout and its plot settings are deleted in one undo step.
    Delete { name: String },
    /// `LAYOUT SET <name>`: resolve the paper layout and switch the session's
    /// active space (session state only).
    Set { name: String },
    /// `LAYOUTMERGE <source> <target>`: move every source paper entity into the
    /// target and delete the source layout in one combined revision/undo step.
    Merge { source: String, target: String },
    /// `LAYOUTWIZARD <name> [<paper> <width> <height>]`: validate the name and
    /// paper numbers, create the layout, and report that the page setup was not
    /// applied (page setup is still not modeled).
    Wizard {
        name: String,
        paper: Option<PaperSetup>,
    },
    /// `VPORTS <rows> <cols>`: build a real `rows * cols` viewport grid in the
    /// active paper layout as one reversible write.
    Viewports { rows: u32, cols: u32 },
    /// `VPCLIP RECT <minx> <miny> <maxx> <maxy>`: validate the rectangle and
    /// replace the **first** viewport's clip (this build has no viewport
    /// selection).
    ClipRect { min: Point3, max: Point3 },
    /// `VPSCALE <scale>`: a standard scale name or ratio, resolved against the
    /// drawing's scale table and applied to the **first** viewport.
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
/// write API, so the parsed setup is validated and then reported as not applied
/// (`layoutwizard.paper_not_applied`) instead of being written.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperSetup {
    pub name: String,
    pub width: f64,
    pub height: f64,
}
