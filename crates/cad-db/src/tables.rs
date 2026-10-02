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
    /// Optional dynamic-block visibility descriptor (spec §3.2).
    ///
    /// `None` for an ordinary block, so non-dynamic definitions are unchanged.
    /// When present, the active state selects which member entities are drawn.
    pub dynamic_visibility: Option<DynamicBlockVisibility>,
}

/// One named visibility state of a dynamic block definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicBlockState {
    /// State name exactly as stored in the drawing's lookup list.
    pub name: String,
    /// Member entities visible while this state is active.
    pub entities: Vec<EntityId>,
}

/// The visibility descriptor of a dynamic block definition.
///
/// This mirrors acadrust's `BlockVisibilityParameter`: the drawing keeps the
/// geometry for every state in one anonymous block and marks the other states'
/// entities invisible. The descriptor records which member entities each state
/// makes visible and which state is currently active, so switching never has to
/// re-import or re-evaluate the base drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicBlockVisibility {
    /// Member entities the parameter governs (the union of every state). These
    /// are the only entities the active state may hide; entities outside this
    /// set (structural markers, ungoverned geometry) always remain visible.
    pub member_entities: Vec<EntityId>,
    /// Selectable states, in list order.
    pub states: Vec<DynamicBlockState>,
    /// The currently active state, when it could be determined honestly.
    ///
    /// `None` means the importer could not resolve it (reported as `Partial`
    /// with a stable reason code); the descriptor then exposes no restricted
    /// set and every governed entity is drawn rather than guessing a state.
    pub active_state: Option<String>,
}

impl DynamicBlockVisibility {
    /// Names of every selectable state, in list order.
    pub fn state_names(&self) -> Vec<&str> {
        self.states.iter().map(|s| s.name.as_str()).collect()
    }

    /// A state by exact name.
    pub fn state(&self, name: &str) -> Option<&DynamicBlockState> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Whether the block defines a state with this exact name.
    pub fn has_state(&self, name: &str) -> bool {
        self.state(name).is_some()
    }

    /// Entities the active state makes visible, or `None` when no active state
    /// was resolved (the caller then draws every governed entity).
    pub fn active_entities(&self) -> Option<&[EntityId]> {
        let active = self.active_state.as_deref()?;
        self.state(active).map(|s| s.entities.as_slice())
    }

    /// Whether a member entity is visible under the active state.
    ///
    /// An entity outside `member_entities` is ungoverned and always visible. If
    /// no active state is known, every member stays visible: an unresolved
    /// active state must not hide geometry on a guess.
    pub fn is_visible(&self, entity: EntityId) -> bool {
        if !self.member_entities.contains(&entity) {
            return true;
        }
        match self.active_entities() {
            Some(visible) => visible.contains(&entity),
            None => true,
        }
    }
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
