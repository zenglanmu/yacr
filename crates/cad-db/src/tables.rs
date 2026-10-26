//! Layer, block, layout and style tables.

use crate::entity::LinetypePattern;
use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}

/// A layer's colour: the AutoCAD Color Index identity **and** its sRGB value.
///
/// Both are carried because they answer different questions. `aci` is the
/// identity the source drawing used (DXF `62`, sign stripped; the sign itself
/// is the off flag, see [`LayerAttributes::off`]) and is what a re-export must
/// write to keep the drawing's colour *index* stable — for example a
/// `ByLayer`-coloured entity on an ACI 1 layer must stay red, not become some
/// nearest-RGB approximation. `rgb` is the resolved display value.
///
/// The pair can disagree by construction: `cad-db` may not depend on `cad-app`
/// (architecture gate), so its ACI→sRGB table is unavailable here; a true-colour
/// source layer (DXF `62` plus `420`) has `aci = None` and only `rgb`, and an
/// ACI source has both. `cad-db` never derives one from the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerColor {
    /// The source ACI index, when the layer had one. `None` for a true-colour
    /// layer.
    pub aci: Option<u8>,
    /// Resolved sRGB display value.
    pub rgb: [u8; 3],
}

impl Default for LayerColor {
    fn default() -> Self {
        // ACI 7 is AutoCAD's nominal white/black entity default.
        LayerColor {
            aci: Some(7),
            rgb: [255, 255, 255],
        }
    }
}

/// A layer's lineweight, kept in the source's own scale and vocabulary.
///
/// The concrete variant stores **1/100 mm** — the DXF `370` / DWG unit — and not
/// millimetres, so a re-export writes back the exact integer it read instead of
/// a rounded float. `ByLayer`/`ByBlock` are degenerate on a layer record; they
/// are modelled rather than collapsed so a source that carries them is not
/// rewritten as an invented concrete weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerLineWeight {
    /// DXF `370` value, in 1/100 mm.
    Hundredths(u16),
    /// The linetype's "default" weight (DXF `370` = `-3`).
    Default,
    /// Symbolic inheritance (DXF `370` = `-1`).
    #[default]
    ByLayer,
}

/// Import-time attributes of a layer table row.
///
/// These live **beside** [`Layer`] in the database (map keyed by [`LayerId`]),
/// exactly like [`EntityRenderAttributes`](crate::EntityRenderAttributes) live
/// beside `DbEntity`, so adding an attribute never ripples into the hundreds of
/// `Layer { .. }` literals across the workspace, and [`Layer`] itself stays the
/// three-field row every existing consumer already reads.
///
/// # Invariant
///
/// `Layer::visible == !(off || frozen)` for every row the importer produces and
/// for every row [`DrawingDatabase::create_layer`](crate::DrawingDatabase::create_layer)
/// creates. `visible` remains the single answered question the rest of the
/// build consumes; `off` and `frozen` are recorded **separately** because the
/// drawing distinguishes them (a frozen layer is not an off layer) even though
/// this phase treats both as "not visible".
///
/// # Nothing reads these yet
///
/// `locked` is enforced on the entity write path (`crate::drawing::lock`), and
/// `off`/`frozen` drive the derived `Layer::visible`. `frozen_in_new_viewport`,
/// `plottable` and `plot_style` are storage plus round-trip only: freeze-specific
/// regeneration, plotting and named plot styles are **not implemented**.
///
/// `color` **is** read: the representation resolves a `ByLayer` entity colour
/// from this row, so editing a layer colour recolours its `ByLayer` entities
/// without writing any entity. `linetype` and `lineweight` are still flattened
/// onto entities at import time and are therefore storage-only here.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerAttributes {
    /// Layer is off (DXF: the **sign** of group `62`; there is no off bit in
    /// group `70`).
    pub off: bool,
    /// Layer is frozen (DXF group `70` bit `1`).
    pub frozen: bool,
    /// Frozen in newly created viewports (DXF group `70` bit `2`).
    ///
    /// This is **not** the off state; the DXF reader explicitly warns against
    /// conflating them.
    pub frozen_in_new_viewport: bool,
    /// Layer is locked (DXF group `70` bit `4`).
    pub locked: bool,
    /// Layer came from an external reference (DXF group `70` bit `0x10`).
    pub xref_dependent: bool,
    /// Whether the layer plots (DXF group `290`). `true` by default.
    pub plottable: bool,
    /// Layer colour (DXF group `62` for the ACI, `420` for true colour).
    pub color: LayerColor,
    /// Linetype **name** (DXF group `6`). Resolved through the drawing's
    /// linetype table, never a dangling reference.
    pub linetype: String,
    /// Lineweight (DXF group `370`).
    pub lineweight: LayerLineWeight,
    /// Opacity in `[0, 1]`, `1.0` fully opaque.
    ///
    /// The source stores transparency as an `AcCmTransparency` XDATA pair where
    /// `0` is opaque and `255` is fully transparent; the sign of the meaning is
    /// flipped here so every other opacity in the build compares the same way.
    pub opacity: f32,
    /// Plot style **name** (DXF group `390` is a handle, not a name).
    ///
    /// Stored because the importer can read it, but a name **cannot survive
    /// export**: the DXF/DWG writers emit the plot-style *handle* only, so an
    /// export writes the default and reports the dropped name. `None` means the
    /// source declared no name.
    pub plot_style: Option<String>,
}

impl Default for LayerAttributes {
    fn default() -> Self {
        // The state of a freshly created layer: on, thawed, unlocked, not
        // xref-dependent, plottable, ACI 7 white, solid, ByLayer weight, fully
        // opaque, no plot style. Every flag default is `false` **except**
        // `plottable`.
        LayerAttributes {
            off: false,
            frozen: false,
            frozen_in_new_viewport: false,
            locked: false,
            xref_dependent: false,
            plottable: true,
            color: LayerColor::default(),
            linetype: "Continuous".to_string(),
            lineweight: LayerLineWeight::ByLayer,
            opacity: 1.0,
            plot_style: None,
        }
    }
}

impl LayerAttributes {
    /// Whether this attribute set makes the layer visible.
    ///
    /// The same predicate the importer uses for [`Layer::visible`]: a frozen
    /// layer is as invisible as an off one.
    pub fn is_visible(&self) -> bool {
        !self.off && !self.frozen
    }
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
