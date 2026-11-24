//! Block attribute semantics: `ATTDEF` definitions and per-`INSERT` values.
//!
//! A block definition owns [`AttributeDefinition`]s as ordinary member entities
//! (an `ATTDEF` is geometry of class [`crate::SemanticGeometry::AttributeDefinition`]).
//! Each `INSERT` of that block carries the concrete [`AttributeValue`]s in
//! block-local coordinates — the same frame as the definition they mirror — so a
//! block reference keeps its own text without rewriting the definition.
//!
//! This module is the platform-independent contract only (ADR-0055 M1). Import,
//! export, rendering, commands and extraction are later lanes; nothing here draws
//! or edits.
//!
//! The appearance of a value reuses the [`EntityColor`]/[`EntityLineType`]/
//! [`EntityLineWeight`]/[`EntityTransparency`] value types an entity row already
//! uses, so a block attribute cannot drift into a parallel styling model.

use crate::{
    EntityColor, EntityLineType, EntityLineWeight, EntityTransparency, LayerId, Point3,
    SemanticGeometry, StyleId, TextAlignH, TextAlignV, TextLineSpacing, DEFAULT_TEXT_WIDTH_FACTOR,
};

/// The `ATTDEF` flag bits that change how a definition behaves.
///
/// The flags are modelled as independent booleans rather than a raw group-70
/// integer: the DXF reader (a later lane) is the one place that decodes bits, and
/// every consumer reads named fields. `invisible` hides the value until
/// `ATTDISP`; `constant` means the value is fixed at definition time; `verify`
/// asks for confirmation on insertion; `preset` inserts without prompting;
/// `locked_position` pins the value's placement; `annotative` ties its text
/// height to the annotation scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttributeFlags {
    /// The value is not displayed unless `ATTDISP` turns attributes on.
    pub invisible: bool,
    /// The value is fixed; insertion does not prompt for it.
    pub constant: bool,
    /// The insertion path asks the user to verify the value.
    pub verify: bool,
    /// The insertion path uses the default without prompting.
    pub preset: bool,
    /// The value's placement is locked against editing.
    pub locked_position: bool,
    /// The value's text scales with the annotation scale.
    pub annotative: bool,
}

/// The text placement of an `ATTDEF`/`ATTRIB`.
///
/// Every field mirrors [`SemanticGeometry::Text`] except the run's content, which
/// belongs to the definition's `default_value` / the value's `value`; the extra
/// [`Self::oblique`] is the DXF text oblique angle (group 51), which a plain
/// `TEXT` entity does not carry at the semantic level.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeText {
    /// Anchor position in the owning block's local frame.
    pub position: Point3,
    /// Glyph height in drawing units; must be finite and positive.
    pub height: f64,
    /// Rotation in radians.
    pub rotation: f64,
    /// Text style the run resolves through.
    pub style: StyleId,
    /// Horizontal width scale; must be finite and positive.
    pub width_factor: f64,
    /// Horizontal placement relative to `position`.
    pub h_align: TextAlignH,
    /// Vertical placement relative to `position`.
    pub v_align: TextAlignV,
    /// Optional wrap boundary in block-local units.
    pub wrap_width: Option<f64>,
    /// Baseline spacing of a multi-line value.
    pub line_spacing: TextLineSpacing,
    /// Font resource key, when the style names one.
    pub font: Option<String>,
    /// DXF group 51 oblique angle in radians (`0.0` is upright).
    pub oblique: f64,
}

impl Default for AttributeText {
    fn default() -> Self {
        AttributeText {
            position: Point3::default(),
            height: 1.0,
            rotation: 0.0,
            style: StyleId(0),
            width_factor: DEFAULT_TEXT_WIDTH_FACTOR,
            h_align: TextAlignH::default(),
            v_align: TextAlignV::default(),
            wrap_width: None,
            line_spacing: TextLineSpacing::default(),
            font: None,
            oblique: 0.0,
        }
    }
}

impl AttributeText {
    /// The flat-text geometry for a run with `text`, in the same placement.
    ///
    /// `oblique` has no field on [`SemanticGeometry::Text`]: it is kept on the
    /// attribute and intentionally **not** folded into the synthesized text, so a
    /// display consumer that only knows `TEXT` renders an upright run instead of
    /// silently approximating a slant. Bounds and plain-text extraction can use
    /// this image; slanted rendering is the representation lane's decision.
    pub fn as_text(&self, text: impl Into<String>) -> SemanticGeometry {
        SemanticGeometry::Text {
            text: text.into(),
            position: self.position,
            style: self.style,
            height: self.height,
            rotation: self.rotation,
            font: self.font.clone(),
            h_align: self.h_align,
            v_align: self.v_align,
            wrap_width: self.wrap_width,
            width_factor: self.width_factor,
            line_spacing: self.line_spacing,
            mirrored: false,
        }
    }
}

/// How one attribute value is drawn.
///
/// The field set is the appearance subset of the entity render-attribute
/// carrier in `cad-db` minus its entity-only members (`geometry_source`,
/// annotative overrides, block clip): those describe how the *entity* was
/// produced, not how its attribute text is styled. `linetype_scale` is kept
/// because every linetype provenance stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeAppearance {
    /// Display opacity (`Explicit` concrete, `ByLayer`/`ByBlock` symbolic).
    pub transparency: EntityTransparency,
    /// Display colour (`Explicit` sRGB, `ByLayer`/`ByBlock` symbolic).
    pub color: EntityColor,
    /// Display lineweight.
    pub lineweight: EntityLineWeight,
    /// Display linetype.
    pub linetype: EntityLineType,
    /// The entity-level linetype scale factor stored beside every provenance.
    pub linetype_scale: f64,
}

impl Default for AttributeAppearance {
    fn default() -> Self {
        AttributeAppearance {
            transparency: EntityTransparency::default(),
            color: EntityColor::default(),
            lineweight: EntityLineWeight::default(),
            linetype: EntityLineType::default(),
            linetype_scale: 1.0,
        }
    }
}

/// An `ATTDEF`: the block definition's template for one attribute.
///
/// The definition is a block **member entity**, so its [`Self::text`] is in the
/// owning block's local frame and moves with the block definition.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDefinition {
    /// The attribute tag (DXF group 2), unique per block in AutoCAD; the model
    /// allows duplicates on import and does not refuse them.
    pub tag: String,
    /// The prompt shown during insertion (DXF group 3).
    pub prompt: String,
    /// The value inserted when the block reference does not override it
    /// (DXF group 1).
    pub default_value: String,
    /// The behaviour flags (DXF group 70).
    pub flags: AttributeFlags,
    /// Ordering key among the block's definitions (draw order / `BATTMAN`
    /// display order). Not an identity: tags are the identity within a block.
    pub order: u32,
    /// Text placement in the block's local frame.
    pub text: AttributeText,
    /// Source handle, for diagnostics and re-export.
    pub source_handle: Option<String>,
}

/// A concrete `ATTRIB` value carried by one `INSERT`.
///
/// The value is in the **same block-local frame** as the `ATTDEF` it mirrors, so
/// it follows the instance transform and is never stored in world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeValue {
    /// The tag this value fills (matches an `ATTDEF` in the referenced block).
    pub tag: String,
    /// The text content.
    pub value: String,
    /// The definition's flags, copied so a detached value stays self-describing.
    pub flags: AttributeFlags,
    /// Text placement in block-local coordinates.
    pub text: AttributeText,
    /// The value's own layer (DXF group 8).
    pub layer: LayerId,
    /// The value's draw appearance.
    pub appearance: AttributeAppearance,
    /// Source handle, for diagnostics and re-export.
    pub source_handle: Option<String>,
}

impl AttributeValue {
    /// Materialize the value a definition contributes to a fresh `INSERT`.
    ///
    /// The tag, flags and text placement are copied; the value starts as the
    /// definition's default, on `layer` (the definition entity's layer) with the
    /// default appearance.
    pub fn from_definition(definition: &AttributeDefinition, layer: LayerId) -> AttributeValue {
        AttributeValue {
            tag: definition.tag.clone(),
            value: definition.default_value.clone(),
            flags: definition.flags,
            text: definition.text.clone(),
            layer,
            appearance: AttributeAppearance::default(),
            // A fresh value has no source handle of its own; the definition's
            // handle identifies the ATTDEF, not the ATTRIB.
            source_handle: None,
        }
    }
}
