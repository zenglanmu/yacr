//! Drawing entities and their import-time display attributes.

use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub struct DbObject {
    pub id: ObjectId,
    pub type_key: String,
    pub revision: Revision,
    pub source_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbEntity {
    pub object: DbObject,
    pub id: EntityId,
    pub layer: LayerId,
    pub space: SpaceId,
    pub geometry: SemanticGeometry,
    pub draw_order: i64,
}

impl DbEntity {
    pub fn entity_style_key(&self) -> &str {
        &self.object.type_key
    }
}

/// How an entity's display opacity (`alpha`) is determined before batching.
///
/// Alpha is *opacity* in `[0, 1]`: `1.0` is fully opaque and `0.0` fully
/// transparent. This is the inverse of acadrust's `Transparency` byte value
/// (`0` opaque, `255` transparent); the importer performs that conversion when
/// it records the attribute.
///
/// The importer resolves `ByObject` (an explicit entity value) and `ByLayer`
/// before storing, because the representation layer has no access to the layer
/// table. `ByBlock` is kept symbolic so INSERT expansion can substitute the
/// containing block reference's opacity (audit B21 / F14).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntityTransparency {
    /// A resolved opacity in `[0, 1]`.
    Explicit(f32),
    /// Inherit the opacity of the containing block reference (`ByBlock`). At
    /// the model root, or with no enclosing INSERT, this resolves to opaque.
    ByBlock,
}

impl Default for EntityTransparency {
    fn default() -> Self {
        EntityTransparency::Explicit(1.0)
    }
}

/// How an entity's display colour is determined before batching.
///
/// Channels are sRGB `0..=255`, exactly the values acadrust exposes through
/// `Color::Rgb` / its canonical ACI table, so nothing is invented on the way in.
/// The importer resolves an explicit entity colour (`ByObject`) and `ByLayer`
/// before storing, because the representation layer has no access to the layer
/// table. `ByBlock` stays symbolic so INSERT expansion can substitute the
/// containing block reference's colour (the same mechanism `EntityTransparency`
/// uses; audit B21 / F14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityColor {
    /// A resolved sRGB colour.
    Explicit([u8; 3]),
    /// Inherit the containing block reference's colour (`ByBlock`). At the
    /// model root, or with no enclosing INSERT, this falls back to the
    /// documented default render colour.
    ByBlock,
    /// No concrete colour was resolved (`ByLayer` with no reachable layer, or a
    /// missing attribute). The representation applies the default render colour.
    #[default]
    ByLayer,
}

/// How an entity's display lineweight is determined before batching.
///
/// Values are millimetres, matching acadrust's `LineWeight::millimeters()`; the
/// source stores 1/100 mm integers. As with [`EntityColor`], `ByObject` and
/// `ByLayer` are resolved by the importer and `ByBlock` stays symbolic for
/// INSERT expansion.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EntityLineWeight {
    /// A resolved lineweight in millimetres.
    Explicit(f32),
    /// acadrust's `LineWeight::Default`: the renderer's default weight.
    Default,
    /// Inherit the containing block reference's lineweight (`ByBlock`). At the
    /// model root this falls back to the documented default.
    ByBlock,
    /// No concrete lineweight was resolved (`ByLayer` with no reachable layer,
    /// or a missing attribute). The representation applies the default.
    #[default]
    ByLayer,
}

/// A resolved dash pattern in drawing units, mirroring the DWG/DXF element
/// convention: positive = dash, negative = gap, zero = dot.
///
/// The importer builds this from acadrust's `LineType::elements`; nothing is
/// fabricated. `cycle` is the sum of element magnitudes (one full repetition)
/// and is stored so the representation layer does not have to recompute and so
/// a degenerate pattern can be rejected during sanitising.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinetypePattern {
    /// Element lengths in drawing units. Empty means continuous.
    pub elements: Vec<f64>,
    /// Total repetition length; `0.0` when `elements` is empty.
    pub cycle: f64,
}

impl LinetypePattern {
    /// The continuous linetype: no elements, no dashes.
    pub fn continuous() -> Self {
        LinetypePattern {
            elements: Vec::new(),
            cycle: 0.0,
        }
    }

    /// Build a pattern from raw element lengths, computing the cycle.
    ///
    /// Non-finite elements are dropped (a single bad entry must not make the
    /// whole line undrawable) and a pattern that loses every element degrades
    /// to continuous.
    pub fn from_elements(elements: impl IntoIterator<Item = f64>) -> Self {
        let elements: Vec<f64> = elements.into_iter().filter(|e| e.is_finite()).collect();
        let cycle: f64 = elements.iter().map(|e| e.abs()).sum();
        if elements.is_empty() || !cycle.is_finite() || cycle <= 0.0 {
            return LinetypePattern::continuous();
        }
        LinetypePattern { elements, cycle }
    }

    /// Whether this pattern draws a solid line (no gaps at all).
    pub fn is_continuous(&self) -> bool {
        self.elements.is_empty()
    }

    /// Whether the pattern has at least one visible marker (dash or dot).
    pub fn has_visible_element(&self) -> bool {
        self.elements.iter().any(|e| *e >= 0.0)
    }

    /// Whether any element is a gap, i.e. the line is not solid.
    pub fn has_gap(&self) -> bool {
        self.elements.iter().any(|e| *e < 0.0)
    }
}

/// How an entity's display linetype is determined before batching.
///
/// The importer resolves an explicit entity linetype and a reachable layer
/// linetype before storing, because the representation layer has no access to
/// the layer table. `ByBlock` stays symbolic so INSERT expansion can substitute
/// the containing block reference's linetype, exactly like [`EntityColor`] and
/// [`EntityLineWeight`]. An unresolvable or unknown linetype becomes
/// `Continuous` rather than an invented pattern.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum EntityLineType {
    /// A named linetype with a concrete dash pattern. `scale` is the entity's
    /// own `linetype_scale` (LTSCALE/global scale is applied later).
    Explicit {
        /// Source linetype name, kept for diagnostics.
        name: String,
        /// Resolved dash pattern.
        pattern: LinetypePattern,
        /// Entity linetype scale factor (`EntityCommon::linetype_scale`).
        scale: f64,
    },
    /// Inherit the containing block reference's linetype (`ByBlock`). At the
    /// model root this falls back to continuous.
    ByBlock,
    /// The source linetype is non-continuous (or unknown) but no concrete
    /// pattern could be resolved. The representation falls back to continuous
    /// and reports it, never fabricating dashes.
    #[default]
    ByLayer,
}

impl EntityLineType {
    /// A concrete named pattern is present.
    pub fn is_explicit(&self) -> bool {
        matches!(self, EntityLineType::Explicit { .. })
    }
}

/// Import-time display attributes that are not part of the semantic geometry.
///
/// These live beside the entity in the database rather than in `DbEntity` so
/// that adding a new attribute does not ripple into every entity constructor
/// (and because the geometry itself remains the authoritative data).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRenderAttributes {
    pub transparency: EntityTransparency,
    /// Resolved display colour (or a symbolic `ByBlock`).
    pub color: EntityColor,
    /// Resolved display lineweight (or a symbolic `ByBlock`).
    pub lineweight: EntityLineWeight,
    /// Resolved display linetype (or a symbolic `ByBlock`).
    pub linetype: EntityLineType,
    /// Which producer supplied the display geometry (proxy cache vs analytic).
    pub geometry_source: GeometrySource,
}

impl Default for EntityRenderAttributes {
    fn default() -> Self {
        EntityRenderAttributes {
            transparency: EntityTransparency::default(),
            color: EntityColor::default(),
            lineweight: EntityLineWeight::default(),
            linetype: EntityLineType::default(),
            geometry_source: GeometrySource::Analytic,
        }
    }
}
