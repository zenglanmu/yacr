//! Drawing entities and their import-time display attributes.

use cad_domain::*;

// The appearance value types moved to `cad-domain` (ADR-0055: block attribute
// values carry the same representation). They are re-exported here so every
// historical `cad_db::Entity*` path keeps working unchanged.
pub use cad_domain::{
    sanitize_linetype_scale, EntityColor, EntityLineType, EntityLineWeight, EntityTransparency,
    LinetypePattern,
};

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

/// Per-scale placement override for an annotative entity.
///
/// Mirrors the subset of `AcDb*ObjectContextData` this build understands: the
/// anchor position and (for text) rotation that apply at one named scale. A
/// missing override means "use the base placement, scaled".
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotativePlacement {
    /// Anchor position for this scale, in world coordinates.
    pub position: Point3,
    /// Rotation in radians for this scale.
    pub rotation: f64,
    /// Explicit glyph height for this scale, when the source context carries
    /// one. `None` means the base height is scaled by the factor.
    pub height: Option<f64>,
}

/// The named scale an [`AnnotativePlacement`] override applies to.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotativeScaleOverride {
    /// Source scale name (for example `"1:100"`), matched case-sensitively
    /// against the drawing's Scale table / active annotation scale.
    pub scale: String,
    pub placement: AnnotativePlacement,
}

/// Annotative scaling state resolved at import time for one entity.
///
/// Hand-built databases keep [`AnnotativeAttributes::default`] (not annotative,
/// no overrides), so nothing here changes the rendering of non-imported data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnnotativeAttributes {
    /// `true` when the source entity (or its style/block) is annotative.
    pub annotative: bool,
    /// Per-scale placement overrides, one per imported context leaf.
    pub overrides: Vec<AnnotativeScaleOverride>,
}

impl AnnotativeAttributes {
    /// A placement override for `scale`, if one was imported.
    pub fn override_for(&self, scale: &str) -> Option<&AnnotativePlacement> {
        self.overrides
            .iter()
            .find(|o| o.scale == scale)
            .map(|o| &o.placement)
    }
}

/// Import-time display attributes that are not part of the semantic geometry.
///
/// These live beside the entity in the database rather than in `DbEntity` so
/// that adding a new attribute does not ripple into every entity constructor
/// (and because the geometry itself remains the authoritative data).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRenderAttributes {
    /// Display opacity: `Explicit` is concrete, `ByLayer` and `ByBlock` stay
    /// symbolic until expansion resolves them.
    pub transparency: EntityTransparency,
    /// Display colour: `Explicit` is concrete sRGB, while `ByLayer` and
    /// `ByBlock` stay symbolic until expansion resolves them.
    pub color: EntityColor,
    /// Display lineweight: `Explicit`/`Default` are concrete, `ByLayer` and
    /// `ByBlock` stay symbolic until expansion resolves them.
    pub lineweight: EntityLineWeight,
    /// Display linetype: `Explicit` is concrete, `ByLayer` and `ByBlock` stay
    /// symbolic until expansion resolves them.
    pub linetype: EntityLineType,
    /// The entity's own linetype scale (DXF group `48`), stored for **every**
    /// linetype provenance — explicit and symbolic alike — so a `ByLayer` or
    /// `ByBlock` entity whose source carried a scale keeps that factor instead
    /// of losing it with the symbolic variant.
    ///
    /// `EntityLineType::Explicit` also carries the factor inside its variant and
    /// that copy stays authoritative for the explicit path (its behaviour is
    /// unchanged); the importer writes both to the same sanitised value. Consume
    /// this field through [`sanitize_linetype_scale`]: a non-finite or
    /// non-positive value means the identity `1.0`.
    pub linetype_scale: f64,
    /// Which producer supplied the display geometry (proxy cache vs analytic).
    pub geometry_source: GeometrySource,
    /// Annotative scaling state (§3.2). Defaults to non-annotative.
    pub annotative: AnnotativeAttributes,
    /// Display-only block-reference clip boundary (`XCLIP`).
    ///
    /// Stored in the **block definition's local space** (see
    /// [`cad_domain::BlockClip`]) so the clip follows the reference through
    /// move/rotate/scale; the representation layer maps it into world space at
    /// expansion time. `None` means "no clip boundary", which is also what an
    /// `XCLIP DELETE` writes. This carrier is transaction/undo aware: a write
    /// stages it through `update_entity_display`, so it bumps one revision and
    /// carries `ChangeMask::STYLE` exactly like the style attributes above.
    pub clip: Option<BlockClip>,
}

impl Default for EntityRenderAttributes {
    fn default() -> Self {
        EntityRenderAttributes {
            transparency: EntityTransparency::default(),
            color: EntityColor::default(),
            lineweight: EntityLineWeight::default(),
            linetype: EntityLineType::default(),
            linetype_scale: 1.0,
            geometry_source: GeometrySource::Analytic,
            annotative: AnnotativeAttributes::default(),
            clip: None,
        }
    }
}
