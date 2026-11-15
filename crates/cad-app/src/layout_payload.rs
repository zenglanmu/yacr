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
    /// `LAYOUT COPY <source> <new-name>`: duplicate one paper layout into a new
    /// row — its **viewports with their clips, `model_to_paper` scales and
    /// `layer_overrides`**, its stored `PLOTSETTINGS` row (re-keyed to the new
    /// id) and its paper-space entities (new entity ids, same draw order) — in
    /// one revision and one undo step.
    ///
    /// The geometry is copied verbatim: no viewport, clip or scale is
    /// re-derived, so a copied sheet frames exactly what the source framed. The
    /// copy never renames or re-points the source, and the source layout keeps
    /// every entity it had.
    ///
    /// `LAYOUT TEMPLATE` (import a layout from a DWT/DXF template) is **not**
    /// modelled: it is refused by name before anything is written.
    Copy { source: String, name: String },
    /// `LAYOUTMERGE <source> <target>`: move every source paper entity into the
    /// target and delete the source layout in one combined revision/undo step.
    Merge { source: String, target: String },
    /// `LAYOUTWIZARD <name> [<paper> <width> <height>] [ARRANGE <arrangement>]`:
    /// validate the name, paper numbers and arrangement, then create the layout
    /// row, its optional authored `PLOTSETTINGS` row **and** its optional
    /// arrangement viewports in one revision and one undo step.
    ///
    /// `arrangement` is the **spelling** the caller typed, exactly like
    /// `VPSCALE`'s `name`: the application resolves it against the bounded
    /// [`WizardArrangement`] set and refuses a miss by name
    /// (`layoutwizard.unknown_arrangement`), so an arrangement outside that set
    /// is never silently replaced by a default. `unavailable` carries the wizard
    /// steps this build cannot perform (plot device, plot style table, title
    /// block). They are **not** applied and are not silently dropped: the
    /// outcome reports each one with its own stable code
    /// ([`UnavailableWizardStep::code`]) while the layout is still created with
    /// the authored paper settings.
    Wizard {
        name: String,
        paper: Option<PaperSetup>,
        arrangement: Option<String>,
        unavailable: Vec<UnavailableWizardStep>,
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
    /// replace the clip of the targeted viewport.
    ///
    /// `viewport` is the **zero-based** `cad_db::Layout::viewports` index of the
    /// explicit-index form; `None` means "the session `CVPORT` current viewport
    /// of the active paper layout". A payload index always wins over the marker,
    /// and an index (or a stale marker) outside the viewport count is the named
    /// `vpclip.unknown_viewport` / `vpclip.current_viewport_out_of_range`
    /// refusal, never a silent redirect to the first viewport.
    ClipRect {
        min: Point3,
        max: Point3,
        viewport: Option<usize>,
    },
    /// `VPCLIP` with a **polygonal** boundary: validate the 3+ point coplanar,
    /// simple (non self-intersecting) polygon and replace the targeted
    /// viewport's clip with it.
    ///
    /// The boundary is stored verbatim in
    /// [`cad_db::PaperViewport::clip`], which is a `Vec<Point3>` polygon and
    /// therefore needs no new database field. The representation lane
    /// (`cad-representation`) resolves the same polygon as its viewport clip and
    /// limits every primitive to it, concave boundaries included, so the write is
    /// a drawable viewport and not stored-only data.
    ///
    /// `viewport` carries the same targeting contract as
    /// [`Self::ClipRect::viewport`]: `None` is the session `CVPORT` current
    /// viewport, `Some(index)` is the explicit zero-based index.
    ClipPolygon {
        points: Vec<Point3>,
        viewport: Option<usize>,
    },
    /// `VPSCALE <scale>`: a standard scale name or ratio, resolved against the
    /// drawing's scale table and applied to the targeted viewport.
    ///
    /// `viewport` is the zero-based `cad_db::Layout::viewports` index; `None`
    /// means the session `CVPORT` current viewport. The explicit index always
    /// wins; both store the documented `1 / factor` paper→model diagonal and keep
    /// the model anchor of the clip centre.
    Scale {
        name: String,
        viewport: Option<usize>,
    },
    /// `LAYVPI <layer>[,<layer>...]`: resolve every name, then freeze those
    /// layers in **every viewport of the active paper layout except the current
    /// one** ([`cad_db::PaperViewport::layer_overrides`]) as one revision and
    /// one undo step. The global layer table is never written.
    ///
    /// That is AutoCAD's own `LAYVPI` default: the current viewport comes from
    /// the session `CVPORT` marker. A layout whose only viewport *is* the current
    /// one has no viewport left to freeze and refuses by name
    /// (`layvpi.no_other_viewport`) instead of freezing everywhere.
    FreezeViewportLayers { layers: Vec<String> },
    /// `LAYVPI` targeted at **one** viewport index of the active layout.
    ///
    /// This is the explicit-index form and it always wins over the `CVPORT`
    /// marker: `viewport` indexes [`cad_db::Layout::viewports`] of the active
    /// paper layout (zero-based), so a caller can still freeze the current
    /// viewport itself. The index is validated against the stored viewport count
    /// (`layvpi.unknown_viewport`).
    FreezeViewportLayersIn {
        layers: Vec<String>,
        viewport: usize,
    },
}

/// The bounded set of viewport arrangements `LAYOUTWIZARD` may create.
///
/// Every member is an **equal-cell grid** built by the same helper as `VPORTS`
/// (`grid_viewports_on_sheet` in `crate::app_layout_geometry`), i.e. the same
/// declared simple-layout geometry; this build has no per-cell sizing, so
/// AutoCAD's asymmetric Standard configurations (`Three:Right`, `Three:Above`,
/// …) are deliberately absent instead of being approximated. `(rows, cols)` in
/// [`Self::grid`] is the `VPORTS` grid that produces the same cells.
///
/// `TwoVertical` / `TwoHorizontal` name the **divider**: a vertical divider
/// produces two side-by-side cells (`1 x 2`), a horizontal divider two stacked
/// cells (`2 x 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardArrangement {
    /// One full-sheet viewport (`1 x 1`).
    Single,
    /// Two side-by-side cells (`1 x 2`).
    TwoVertical,
    /// Two stacked cells (`2 x 1`).
    TwoHorizontal,
    /// Three side-by-side cells (`1 x 3`).
    ThreeVertical,
    /// Three stacked cells (`3 x 1`).
    ThreeHorizontal,
    /// A `2 x 2` grid of four cells.
    Four,
}

impl WizardArrangement {
    /// Every supported arrangement, in the order a refusal message lists them.
    pub const ALL: [Self; 6] = [
        Self::Single,
        Self::TwoVertical,
        Self::TwoHorizontal,
        Self::ThreeVertical,
        Self::ThreeHorizontal,
        Self::Four,
    ];

    /// The `(rows, cols)` grid of this arrangement (the `VPORTS` equivalent).
    pub fn grid(self) -> (u32, u32) {
        match self {
            Self::Single => (1, 1),
            Self::TwoVertical => (1, 2),
            Self::TwoHorizontal => (2, 1),
            Self::ThreeVertical => (1, 3),
            Self::ThreeHorizontal => (3, 1),
            Self::Four => (2, 2),
        }
    }

    /// The stable spelling used by diagnostics and by the command grammar.
    pub fn name(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::TwoVertical => "TwoVertical",
            Self::TwoHorizontal => "TwoHorizontal",
            Self::ThreeVertical => "ThreeVertical",
            Self::ThreeHorizontal => "ThreeHorizontal",
            Self::Four => "Four",
        }
    }

    /// Resolve one arrangement spelling, case-insensitively.
    ///
    /// The separator is normalized away, so `TwoVertical`, `two-vertical` and
    /// `two_vertical` are the same member. Anything else is `None`; the caller
    /// refuses it by name rather than falling back to a default.
    pub fn from_name(text: &str) -> Option<Self> {
        let normalized: String = text
            .trim()
            .chars()
            .filter(|character| *character != '-' && *character != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|arrangement| arrangement.name().to_ascii_lowercase() == normalized)
    }

    /// The refusal sentence for a name outside [`Self::ALL`].
    pub fn unknown_name_refusal(text: &str) -> String {
        let supported: Vec<&str> = Self::ALL
            .iter()
            .map(|arrangement| arrangement.name())
            .collect();
        format!(
            "layoutwizard.unknown_arrangement: {:?} is not a LAYOUTWIZARD arrangement; this build \
             supports the equal-cell arrangements {} and nothing else",
            text,
            supported.join(", ")
        )
    }
}

/// One `LAYOUTWIZARD` step this build cannot perform.
///
/// The wizard's printer/plotter, plot-style-table and title-block pages have no
/// implementation (and no plot device is ever opened), so the payload can only
/// name the request: the handler creates the layout with the authored paper
/// settings and reports the step with [`Self::code`], which is the refusal
/// record. Nothing about the requested device, style or block is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableWizardStep {
    /// `PLOTDEVICE <name>`: printers and plot devices are not modelled.
    PlotDevice(String),
    /// `PLOTSTYLE <name>`: plot-style tables are not modelled.
    PlotStyle(String),
    /// `TITLEBLOCK <name>`: the wizard does not insert title blocks.
    TitleBlock(String),
}

impl UnavailableWizardStep {
    /// The stable diagnostic code that names this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PlotDevice(_) => "layoutwizard.plot_device_not_implemented",
            Self::PlotStyle(_) => "layoutwizard.plot_style_not_implemented",
            Self::TitleBlock(_) => "layoutwizard.title_block_not_implemented",
        }
    }

    /// The spelling the caller used for the step.
    pub fn step(&self) -> &'static str {
        match self {
            Self::PlotDevice(_) => "PLOTDEVICE",
            Self::PlotStyle(_) => "PLOTSTYLE",
            Self::TitleBlock(_) => "TITLEBLOCK",
        }
    }

    /// The value the caller attached to the step.
    pub fn value(&self) -> &str {
        match self {
            Self::PlotDevice(value) | Self::PlotStyle(value) | Self::TitleBlock(value) => value,
        }
    }
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
