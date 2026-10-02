//! Layer, block, layout and style tables.

use crate::entity::LinetypePattern;
use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}

/// A named linetype table entry (spec §3.2 / §7.1).
///
/// `pattern` is the resolved dash pattern; `complex` records whether the source
/// linetype carried shape/text elements that this build cannot draw. A complex
/// linetype still stores its dash segments so the line is at least dashed, and
/// the importer reports the omitted glyphs as a `Partial` reason.
#[derive(Debug, Clone, PartialEq)]
pub struct LineType {
    pub id: LinetypeId,
    pub name: String,
    pub pattern: LinetypePattern,
    /// `true` when the source linetype had embedded shape/text content.
    pub complex: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDefinition {
    pub id: BlockId,
    pub entities: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: LayoutId,
    pub name: String,
    pub viewports: Vec<PaperViewport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperViewport {
    pub clip: Vec<Point3>,
    pub model_to_paper: Transform3,
    pub completeness: Completeness,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub id: StyleId,
    pub name: String,
    pub resource_keys: Vec<String>,
}

// ---------------------------------------------------------------------------
// Annotation scales (spec §3.2 / capability matrix)
// ---------------------------------------------------------------------------

/// A named annotation scale from the drawing's `ACAD_SCALELIST`.
///
/// The values mirror the DWG `AcDbScale` object without leaking the acadrust
/// type: `paper_units` / `drawing_units` are the raw ratio, and [`Scale::factor`]
/// is the paper/drawing ratio that scales an annotative entity's glyph height
/// (`1:100 -> 0.01`, `2:1 -> 2.0`, `1:1 -> 1.0`).
#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    pub id: ScaleId,
    pub name: String,
    pub paper_units: f64,
    pub drawing_units: f64,
}

impl Scale {
    /// The paper/drawing scale factor.
    ///
    /// A degenerate `0` drawing-units value falls back to `1.0` rather than
    /// producing an infinite factor, matching acadrust's `Scale::factor`.
    pub fn factor(&self) -> f64 {
        if self.drawing_units.abs() < 1e-10 {
            1.0
        } else {
            self.paper_units / self.drawing_units
        }
    }

    /// The drawing/paper inverse factor (`1:100 -> 100.0`).
    pub fn inverse_factor(&self) -> f64 {
        if self.paper_units.abs() < 1e-10 {
            1.0
        } else {
            self.drawing_units / self.paper_units
        }
    }

    /// Whether this is the unit scale (`1:1`) within tolerance.
    pub fn is_unit_scale(&self) -> bool {
        (self.factor() - 1.0).abs() < 1e-10
    }

    /// Whether this is a reduction (`factor < 1`, e.g. `1:100`).
    pub fn is_reduction(&self) -> bool {
        self.factor() < 1.0 - 1e-10
    }

    /// Whether this is an enlargement (`factor > 1`, e.g. `2:1`).
    pub fn is_enlargement(&self) -> bool {
        self.factor() > 1.0 + 1e-10
    }

    /// Whether both ratio components are finite.
    pub fn is_well_formed(&self) -> bool {
        self.paper_units.is_finite() && self.drawing_units.is_finite()
    }
}

/// The drawing's active annotation scale (`CANNOSCALE` + `CANNOSCALEVALUE`).
///
/// `name` is the source name (for example `"1:100"`); `factor` is the raw
/// paper/drawing value as stored in the header, kept so an unknown name still
/// carries the real ratio. [`DrawingDatabase::annotation_scale`] prefers the
/// named [`Scale`] from the table when it resolves, so the table is
/// authoritative and the header value is the documented fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAnnotationScale {
    pub name: String,
    /// Paper/drawing factor from the header (`CANNOSCALEVALUE`).
    pub value: f64,
    /// `true` when the name resolved to an entry in the drawing's Scale table.
    pub named: bool,
}

// ---------------------------------------------------------------------------
// Plot settings (paper-space output configuration)
// ---------------------------------------------------------------------------

/// Rotation applied to the sheet when plotting.
///
/// Mirrors the DWG `PlotRotation` codes without leaking the acadrust type into
/// the database. `to_degrees` is the only interpretation of a code, so the
/// importer and the planner cannot disagree about which way is 90°.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotRotation {
    #[default]
    None,
    Degrees90,
    Degrees180,
    Degrees270,
}

impl PlotRotation {
    /// Rotation angle in degrees, counter-clockwise.
    pub fn to_degrees(self) -> f64 {
        match self {
            PlotRotation::None => 0.0,
            PlotRotation::Degrees90 => 90.0,
            PlotRotation::Degrees180 => 180.0,
            PlotRotation::Degrees270 => 270.0,
        }
    }
}

/// Unit of the stored paper dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotPaperUnits {
    #[default]
    Millimeters,
    Inches,
    Pixels,
}

/// Which region of the drawing a plot settings record targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotType {
    LastScreenDisplay,
    Extents,
    Limits,
    View,
    #[default]
    Window,
    Layout,
}

/// Unprintable margins of the sheet, in the paper's own units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlotMargins {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl PlotMargins {
    pub fn uniform(margin: f64) -> Self {
        PlotMargins {
            left: margin,
            bottom: margin,
            right: margin,
            top: margin,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.left == 0.0 && self.bottom == 0.0 && self.right == 0.0 && self.top == 0.0
    }

    pub fn horizontal_total(&self) -> f64 {
        self.left + self.right
    }

    pub fn vertical_total(&self) -> f64 {
        self.top + self.bottom
    }
}

/// Where a [`PlotSettingsRecord`]'s values came from.
///
/// This is the honest provenance the UI/CLI surface: a `DefaultPage` record was
/// not read from the drawing, so no exactness is claimed for its values.
#[derive(Debug, Clone, PartialEq)]
pub enum PlotProvenance {
    /// Read from a PLOTSETTINGS object or the layout's embedded plot data.
    Imported,
    /// No plot settings were present; the page is an explicit fallback.
    DefaultPage {
        /// Machine-stable reason the fallback was used.
        reason: String,
    },
}

/// Plot configuration for one paper-space layout.
///
/// Every field is optional at the database level: an absent record is reported
/// by [`crate::DrawingDatabase::plot_settings_for`] as an explicit default page,
/// never as a guessed vendor configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotSettingsRecord {
    /// Layout this configuration belongs to.
    pub layout: LayoutId,
    /// Paper-size name as stored in the drawing (may be empty).
    pub paper_size_name: String,
    /// Physical paper width in `paper_units` (0 means unknown/not read).
    pub paper_width: f64,
    /// Physical paper height in `paper_units` (0 means unknown/not read).
    pub paper_height: f64,
    pub margins: PlotMargins,
    pub rotation: PlotRotation,
    /// Custom scale numerator (model units per paper unit for a model plot).
    pub scale_numerator: f64,
    /// Custom scale denominator.
    pub scale_denominator: f64,
    pub plot_type: PlotType,
    pub paper_units: PlotPaperUnits,
    /// Whether these values were read from the file or are a documented default.
    pub provenance: PlotProvenance,
}

impl PlotSettingsRecord {
    /// The size of the already-rotated sheet, in paper units.
    ///
    /// A 90°/270° rotation swaps the sheet's width and height, matching how the
    /// plot surface is presented to the printer.
    pub fn rotated_size(&self) -> (f64, f64) {
        match self.rotation {
            PlotRotation::Degrees90 | PlotRotation::Degrees270 => {
                (self.paper_height, self.paper_width)
            }
            _ => (self.paper_width, self.paper_height),
        }
    }
}
