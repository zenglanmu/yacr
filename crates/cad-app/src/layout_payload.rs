//! Typed input for the 2026-11-07 layout / viewport batch.
//!
//! Kept out of `command_payload.rs` (which is already at the file-size ceiling)
//! and out of the command enum for the same reason. Every variant is an
//! **input**, never behaviour: `crate::app_layout` validates it and then reads,
//! switches space, or performs the one-transaction layout/viewport write
//! documented in [`crate::command_id::layout`].

use cad_db::PlotRotation;
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
    /// paper numbers, create the layout, and (when a paper setup is supplied)
    /// write one authored `PLOTSETTINGS` row in the same revision and undo step.
    Wizard {
        name: String,
        paper: Option<PaperSetup>,
    },
    /// `PAGESETUP <layout> <paper> <width> <height> [<num>:<den>] [0|90|180|270]`:
    /// write one authored page setup for an existing layout as one revision and
    /// one undo step.
    ///
    /// Grammar boundary: `<layout>` is the whole first token (so a layout name
    /// with a space is not addressable through this typed form); `<paper>` is
    /// one token; the width/height are millimetres. `scale` is `None` for the
    /// default `1:1`; rotation is one of the four typed quarter turns.
    PageSetup {
        layout: String,
        paper: PaperSetup,
        scale: Option<(f64, f64)>,
        rotation: PlotRotation,
    },
    /// `PSETUPIN <source-layout> <target-layout>`: copy a **stored** page setup
    /// from one layout of this drawing to another (same-drawing import; there is
    /// no cross-drawing transfer in this build).
    ///
    /// The source must carry a stored `PLOTSETTINGS` row: the A4 fallback is
    /// never synthesized into a real record (`psetupin.no_source`). The copied
    /// record is re-keyed to the target and marked
    /// [`cad_db::PlotProvenance::Authored`].
    PageSetupIn { source: String, target: String },
    /// `VPORTS <rows> <cols>`: build a real `rows * cols` viewport grid in the
    /// active paper layout as one reversible write.
    Viewports { rows: u32, cols: u32 },
    /// `VPCLIP RECT <minx> <miny> <maxx> <maxy>`: validate the rectangle and
    /// replace the **first** viewport's clip (this build has no viewport
    /// selection).
    ClipRect { min: Point3, max: Point3 },
    /// `VPCLIP` with a **polygonal** boundary: validate the 3+ point coplanar,
    /// simple (non self-intersecting) polygon and replace the **first**
    /// viewport's clip with it.
    ///
    /// The boundary is stored verbatim in
    /// [`cad_db::PaperViewport::clip`], which is a `Vec<Point3>` polygon and
    /// therefore needs no new database field. The representation lane
    /// (`cad-representation`) still resolves only three- and four-point clips,
    /// so the written viewport renders as `viewport_unsupported: clip_arity`
    /// until that lane learns N-gons; the command reports that residual as
    /// `vpclip.polygon_not_renderable` instead of claiming a draw.
    ClipPolygon { points: Vec<Point3> },
    /// `VPSCALE <scale>`: a standard scale name or ratio, resolved against the
    /// drawing's scale table and applied to the **first** viewport.
    Scale { name: String },
    /// `LAYVPI <layer>[,<layer>...]`: resolve every name, then freeze those
    /// layers in **every** viewport of the active paper layout
    /// ([`cad_db::PaperViewport::layer_overrides`]) as one revision and one
    /// undo step. The global layer table is never written.
    ///
    /// AutoCAD's `LAYVPI` freezes in "all viewports except the current one";
    /// this build has no viewport selection, so "all viewports of the active
    /// layout" is the declared scope.
    FreezeViewportLayers { layers: Vec<String> },
    /// `LAYVPI` targeted at **one** viewport index of the active layout.
    ///
    /// This is the viewport-targeting form: `viewport` indexes
    /// [`cad_db::Layout::viewports`] of the active paper layout, so a caller can
    /// express AutoCAD's "every viewport except the current one" by issuing the
    /// per-index form for each index it wants. The index is validated against
    /// the stored viewport count (`layvpi.unknown_viewport`).
    FreezeViewportLayersIn {
        layers: Vec<String>,
        viewport: usize,
    },
}

/// The page setup of `PAGESETUP` / `LAYOUTWIZARD`.
///
/// `name` is the paper-size name the caller typed (for example `A4`); the
/// numeric fields are **positive finite** paper dimensions in millimetres. The
/// parsed setup is written to the drawing as an authored
/// [`cad_db::PlotSettingsRecord`] ([`cad_db::PlotProvenance::Authored`]), never
/// as an imported or fallback record.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperSetup {
    pub name: String,
    pub width: f64,
    pub height: f64,
}
