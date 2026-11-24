//! Display appearance value types shared by entity rows and block attributes.
//!
//! These live in `cad-domain` rather than `cad-db` so a domain-level type
//! (a block attribute value, ADR-0055) can carry the same colour, linetype,
//! lineweight and transparency representation an entity row uses — one model,
//! not a parallel one. `cad-db` re-exports every item under its historical
//! `cad_db::Entity*` path, so no consumer path changes.

/// How an entity's display opacity (`alpha`) is determined before batching.
///
/// Alpha is *opacity* in `[0, 1]`: `1.0` is fully opaque and `0.0` fully
/// transparent. This is the inverse of acadrust's `Transparency` byte value
/// (`0` opaque, `255` transparent); the importer performs that conversion only
/// when it records an explicit entity value.
///
/// The importer resolves an explicit entity value (`ByObject`) and keeps both
/// symbolic forms. `ByLayer` is resolved by the representation from the owning
/// layer's [`LayerAttributes::opacity`](crate::LayerAttributes::opacity) at
/// expansion time — the same mechanism [`EntityColor`] and [`EntityLineWeight`]
/// use — so editing a layer opacity changes its `ByLayer` entities with no
/// entity write. `ByBlock` is kept symbolic so INSERT expansion can substitute
/// the containing block reference's resolved opacity (audit B21 / F14).
///
/// The default is `ByLayer`, so an entity built without an explicit
/// transparency inherits from its layer (opaque when the layer has no stored
/// attribute row) rather than baking "fully opaque" into the entity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EntityTransparency {
    /// A resolved opacity in `[0, 1]`.
    Explicit(f32),
    /// Inherit the opacity of the containing block reference (`ByBlock`). At
    /// the model root, or with no enclosing INSERT, this resolves to opaque.
    ByBlock,
    /// Use the owning layer's opacity. The representation resolves it from the
    /// layer table at expansion time; a layer with no stored attribute row falls
    /// back to `LayerAttributes::default().opacity` (fully opaque).
    #[default]
    ByLayer,
}

/// How an entity's display colour is determined before batching.
///
/// Channels are sRGB `0..=255`, exactly the values acadrust exposes through
/// `Color::Rgb` / its canonical ACI table, so nothing is invented on the way in.
/// The importer resolves an explicit entity colour (`ByObject`) and keeps
/// `ByLayer` **symbolic**: the representation reads the owning layer's colour
/// from the layer table when it expands the entity, so editing a layer colour
/// recolours its `ByLayer` entities without any entity write. `ByBlock` stays
/// symbolic so INSERT expansion can substitute the containing block reference's
/// colour (the same mechanism `EntityTransparency` uses; audit B21 / F14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityColor {
    /// A resolved sRGB colour.
    Explicit([u8; 3]),
    /// Inherit the containing block reference's colour (`ByBlock`). At the
    /// model root, or with no enclosing INSERT, this falls back to the
    /// documented default render colour.
    ByBlock,
    /// Use the owning layer's colour. The representation resolves it from the
    /// layer table at expansion time; a layer with no stored attribute row falls
    /// back to `LayerAttributes::default().color` (ACI 7 white).
    #[default]
    ByLayer,
}

/// How an entity's display lineweight is determined before batching.
///
/// Values are millimetres, matching acadrust's `LineWeight::millimeters()`; the
/// source stores 1/100 mm integers. As with [`EntityColor`], only an explicit
/// entity value is resolved by the importer: `ByLayer` and `ByBlock` stay
/// symbolic and the representation resolves them at expansion time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EntityLineWeight {
    /// A resolved lineweight in millimetres.
    Explicit(f32),
    /// acadrust's `LineWeight::Default`: the renderer's default weight.
    Default,
    /// Inherit the containing block reference's lineweight (`ByBlock`). At the
    /// model root this falls back to the documented default.
    ByBlock,
    /// Use the owning layer's lineweight. The representation resolves it from
    /// the layer table at expansion time (1/100 mm → mm); a layer row that itself
    /// carries no weight falls back to the documented default and is flagged
    /// unresolved.
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

/// The single sanitisation rule for a linetype scale factor.
///
/// A non-finite or non-positive factor is meaningless (it would erase every
/// dash), so it degrades to the identity `1.0`; anything else is kept as stored.
/// The importer, the database field and the representation all call **this**
/// function rather than each keeping a private copy of the rule.
pub fn sanitize_linetype_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// How an entity's display linetype is determined before batching.
///
/// The importer resolves only an explicit entity linetype; `ByLayer` and
/// `ByBlock` stay symbolic and the representation resolves them at expansion time
/// (a `ByLayer` name is looked up in the drawing's linetype table). This mirrors
/// [`EntityColor`] and [`EntityLineWeight`]. An unresolvable or unknown linetype
/// becomes `Continuous` rather than an invented pattern.
///
/// The entity's own scale factor is **not** part of the symbolic variants: it
/// lives on [`EntityRenderAttributes::linetype_scale`] for every provenance.
/// `Explicit` additionally carries a copy inside the variant, and that copy is
/// what the explicit path resolves (byte-identical to before the attribute
/// field existed).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum EntityLineType {
    /// A named linetype with a concrete dash pattern. `scale` is the entity's
    /// own `linetype_scale` (LTSCALE/global scale is applied later).
    Explicit {
        /// Source linetype name, kept for diagnostics.
        name: String,
        /// Resolved dash pattern.
        pattern: LinetypePattern,
        /// Entity linetype scale factor (`EntityCommon::linetype_scale`). The
        /// authoritative factor for this variant; also mirrored on
        /// [`EntityRenderAttributes::linetype_scale`] by the importer.
        scale: f64,
    },
    /// Inherit the containing block reference's linetype (`ByBlock`). At the
    /// model root this falls back to continuous.
    ByBlock,
    /// Use the owning layer's linetype. The representation looks the layer's
    /// linetype name up in the drawing's linetype table at expansion time; when
    /// the name is absent it falls back to continuous and flags the fragment
    /// unresolved, never fabricating dashes.
    #[default]
    ByLayer,
}

impl EntityLineType {
    /// A concrete named pattern is present.
    pub fn is_explicit(&self) -> bool {
        matches!(self, EntityLineType::Explicit { .. })
    }
}
