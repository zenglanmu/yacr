//! The writable **layer attribute** surface: one accepted grammar per key, plus
//! the canonical projection a reader sees.
//!
//! This is the layer-side sibling of [`crate::property_value`] (the entity
//! display-property grammar) and it follows the same discipline: every accepted
//! value is a **string with a documented grammar**, every value it writes has a
//! **canonical form this module also produces**, and nothing is clamped,
//! rounded or substituted. The one difference is the unit vocabulary — a layer
//! record stores the *table's* values (DXF group numbers), not the entity
//! display values, so `lineweight` and `transparency` do **not** share the entity
//! grammar's units (see the table).
//!
//! ## Accepted grammar
//!
//! | key | accepted value | stored in [`LayerAttributes`] |
//! |---|---|---|
//! | `color` | `aci:<1..=255>`, `#RRGGBB`, `rgb:#RRGGBB`, `r,g,b` | [`LayerColor`] `{ aci, rgb }` |
//! | `linetype` | an existing linetype **name** (exact first, then a unique ASCII case-insensitive match) | `linetype` |
//! | `lineweight` | `ByLayer`, `Default`, or **1/100 mm** integer `0..=211` | [`LayerLineWeight`] |
//! | `transparency` | an integer `0..=100` (percent transparent, `0` = opaque) | `opacity = 1 − v/100` |
//! | `off` | `true`, `false`, `0`, `1` (case-insensitive) | `off` |
//! | `frozen` | `true`, `false`, `0`, `1` | `frozen` |
//! | `locked` | `true`, `false`, `0`, `1` | `locked` |
//! | `plottable` | `true`, `false`, `0`, `1` | `plottable` |
//!
//! ### `color`
//!
//! * `aci:N` for `N` in `1..=255` sets **both** halves: the ACI identity and the
//!   palette's sRGB triple for it. The palette is
//!   [`crate::property_value::rgb_for_aci`] — the **one** ACI table the build
//!   has (the entity grammar uses the same function), so `aci:30` cannot mean
//!   two different colours on an entity row and a layer row. `0` (ByBlock) and
//!   `256` (ByLayer) are symbolic and refused: a layer is the end of the
//!   inheritance chain.
//! * `#RRGGBB` / `rgb:#RRGGBB` / `r,g,b` (three decimal `0..=255` channels) store
//!   a **true colour**: `aci = None`. The ACI is deliberately **not** inferred
//!   even when the triple happens to equal a palette entry — `cad-db`'s
//!   [`LayerColor`] documents that the two halves are separate facts and are
//!   never derived from each other, and inventing an index would change what a
//!   re-export writes (`62` instead of `420`).
//! * `ByLayer` / `ByBlock` are refused **by name**: a layer has no layer above it
//!   to inherit from.
//!
//! ### `linetype`
//!
//! A non-empty name that must exist in the drawing's own linetype table, so no
//! dangling reference is ever stored. The name is canonicalised to the table
//! entry's spelling. `ByLayer` / `ByBlock` are refused by name (a layer's
//! linetype has no inheritance target); an unknown name is
//! `layer.linetype_missing`.
//!
//! ### `lineweight` — 1/100 mm, **not** millimetres
//!
//! The layer record stores the DXF `370` integer ([`LayerLineWeight::Hundredths`]),
//! so `35` means `0.35 mm`. This is the documented difference from
//! `SETENTITYPROPERTY`'s `lineweight`, whose values are millimetres: the entity
//! row's value is a display property, the layer's is the table's raw field, and
//! silently reinterpreting one as the other would write a 100× wrong weight.
//! `ByBlock` is refused (the layer model has no such variant).
//!
//! ### `transparency` — percent, `0..=100`
//!
//! The input is percent **transparent** (`0` = opaque), stored as
//! `opacity = 1 − v/100`. The layer grammar accepts the full `0..=100` range (an
//! entity is limited to `0..=90`); the projection is
//! `round((1 − opacity) · 100)`, so a value read from a row can be written back
//! unchanged.
//!
//! ## Not writable (deliberately)
//!
//! * `plotstyle` — **refused as not implemented** (`layer.property.unsupported`).
//!   The importer can read a plot-style *name*, but the writers emit only the
//!   style **handle**, so a name written here could not survive an export: it
//!   would be reported as dropped and silently become the default. Storing a
//!   value the export cannot honour is a lie, so the key is refused instead of
//!   accepted-and-dropped.
//! * `frozenInNewViewport` / `xrefDependent` — import facts with no editing
//!   semantics in this phase (there are no viewports to be "new" and no xref
//!   machinery), so there is no key for them.
//! * `name` / `visible` — the name is [`cad_app::CommandId::RenameLayer`]'s, and
//!   `visible` is **derived** (`visible == !(off || frozen)`):
//!   [`cad_db::DrawingDatabase::set_layer_attributes`] recomputes it, so it can
//!   never be written directly.
//!
//! ## What a write does *not* do
//!
//! Nothing here changes rendering. `off` / `frozen` hide a layer through the
//! existing `Layer::visible` path (and only through it); `locked`, `plottable`,
//! the colour, the linetype, the lineweight and the transparency are **stored
//! only** — see `docs/compatibility.md`.

use cad_db::{
    DrawingDatabase, LayerAttributes, LayerColor, LayerLineWeight, LineType as DbLinetype,
};
use cad_domain::{CadError, CadResult};

/// The layer attribute keys this write path accepts, in panel order.
pub const SUPPORTED_KEYS: [&str; 8] = [
    "color",
    "linetype",
    "lineweight",
    "transparency",
    "off",
    "frozen",
    "locked",
    "plottable",
];

/// A key the wire knows but the write path **refuses as not implemented**.
///
/// Kept separate from [`SUPPORTED_KEYS`] so the refusal is by name (and stable)
/// instead of "unknown key", while no caller can believe the value is stored.
pub const UNIMPLEMENTED_KEYS: [&str; 1] = ["plotstyle"];

/// Whether `key` is a layer attribute this write path can apply.
pub fn is_supported_key(key: &str) -> bool {
    SUPPORTED_KEYS.contains(&key)
}

/// Whether `key` is a layer attribute the grammar *names*, including the ones it
/// refuses as not implemented ([`UNIMPLEMENTED_KEYS`]).
///
/// The wire admission uses this so an unknown key and an unimplemented one get
/// their distinct, stable answers.
pub fn is_known_key(key: &str) -> bool {
    is_supported_key(key) || UNIMPLEMENTED_KEYS.contains(&key)
}

/// A validated write target, already resolved against the drawing's tables.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LayerPropertyValue {
    Off(bool),
    Frozen(bool),
    Locked(bool),
    Plottable(bool),
    Color(LayerColor),
    /// A linetype **table name**, canonicalised to the entry's spelling.
    Linetype(String),
    LineWeight(LayerLineWeight),
    /// Stored opacity (`0.0` = fully transparent, `1.0` = opaque).
    Transparency(f32),
}

/// Parse one `(key, value)` pair against `database`.
///
/// Refusals carry a stable code and change nothing:
///
/// * `layer.property.unsupported` — an unknown key, or `plotstyle` (not
///   implemented);
/// * `layer.property.unsupported` — a read-only attribute of the row
///   (`name` / `visible`);
/// * `layer.linetype_missing` — the name is not in the drawing's linetype table
///   (or matches more than one entry case-insensitively);
/// * `layer.property.value` — the grammar/range does not match the key.
pub(crate) fn parse(
    database: &DrawingDatabase,
    key: &str,
    value: &str,
) -> CadResult<LayerPropertyValue> {
    match key {
        "color" => Ok(LayerPropertyValue::Color(parse_color(value)?)),
        "linetype" => Ok(LayerPropertyValue::Linetype(linetype_by_name(
            database, value,
        )?)),
        "lineweight" => Ok(LayerPropertyValue::LineWeight(parse_lineweight(value)?)),
        "transparency" => Ok(LayerPropertyValue::Transparency(parse_transparency(value)?)),
        "off" => Ok(LayerPropertyValue::Off(parse_flag(key, value)?)),
        "frozen" => Ok(LayerPropertyValue::Frozen(parse_flag(key, value)?)),
        "locked" => Ok(LayerPropertyValue::Locked(parse_flag(key, value)?)),
        "plottable" => Ok(LayerPropertyValue::Plottable(parse_flag(key, value)?)),
        "plotstyle" => Err(CadError::Unsupported(
            "layer.property.unsupported: 'plotstyle' is not implemented — the writer emits only \
             the style handle, so a stored name could not survive an export"
                .into(),
        )),
        "name" => Err(CadError::Unsupported(
            "layer.property.unsupported: a layer name is changed by RENAMELAYER, not by a \
             property write"
                .into(),
        )),
        "visible" => Err(CadError::Unsupported(
            "layer.property.unsupported: 'visible' is derived from off/frozen; write those two \
             flags instead"
                .into(),
        )),
        other => Err(CadError::Unsupported(format!(
            "layer.property.unsupported: '{other}' is not a writable layer property"
        ))),
    }
}

/// Apply an already validated value to a candidate attribute set.
///
/// The caller does this on a **clone** of the stored attributes and publishes the
/// clone only after every check passed, so a partially applied write cannot
/// exist.
pub(crate) fn apply(attributes: &mut LayerAttributes, value: LayerPropertyValue) {
    match value {
        LayerPropertyValue::Off(off) => attributes.off = off,
        LayerPropertyValue::Frozen(frozen) => attributes.frozen = frozen,
        LayerPropertyValue::Locked(locked) => attributes.locked = locked,
        LayerPropertyValue::Plottable(plottable) => attributes.plottable = plottable,
        LayerPropertyValue::Color(color) => attributes.color = color,
        LayerPropertyValue::Linetype(linetype) => attributes.linetype = linetype,
        LayerPropertyValue::LineWeight(lineweight) => attributes.lineweight = lineweight,
        LayerPropertyValue::Transparency(opacity) => attributes.opacity = opacity,
    }
}

/// Canonical `#rrggbb` of a stored sRGB triple (lowercase, as the wire promises).
pub fn color_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Canonical projection of a stored layer lineweight: the write grammar's own
/// vocabulary (`ByLayer` / `Default` / the 1/100 mm integer).
///
/// A stored weight outside `0..=211` is printed as the stored integer (honest,
/// never clamped) and the grammar will refuse to write it back — the row shows a
/// value it cannot edit instead of a rounded lie.
pub fn lineweight_value(lineweight: LayerLineWeight) -> String {
    match lineweight {
        LayerLineWeight::ByLayer => "ByLayer".to_string(),
        LayerLineWeight::Default => "Default".to_string(),
        LayerLineWeight::Hundredths(hundredths) => hundredths.to_string(),
    }
}

/// Canonical projection of a stored layer opacity as **percent transparent**.
///
/// A stored opacity outside `[0, 1]` (only a corrupt import can produce one) is
/// reported as `opacity:<raw>` and is not writable back, exactly like the entity
/// grammar's `transparency` projection: the invariant violation is visible
/// instead of being rounded into a plausible percentage.
pub fn transparency_value(opacity: f32) -> String {
    if opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
        let percent = ((1.0 - opacity) * 100.0).round();
        format!("{percent:.0}")
    } else {
        format!("opacity:{opacity}")
    }
}

fn invalid_value(key: &str, expected: &str, value: &str) -> CadError {
    CadError::InvalidInput(format!(
        "layer.property.value: '{key}' needs {expected}, got '{value}'"
    ))
}

fn parse_flag(key: &str, value: &str) -> CadResult<bool> {
    if value.eq_ignore_ascii_case("true") || value == "1" {
        return Ok(true);
    }
    if value.eq_ignore_ascii_case("false") || value == "0" {
        return Ok(false);
    }
    Err(invalid_value(key, "true, false, 0 or 1", value))
}

fn parse_color(value: &str) -> CadResult<LayerColor> {
    if value.eq_ignore_ascii_case("ByLayer") || value.eq_ignore_ascii_case("ByBlock") {
        return Err(CadError::InvalidInput(format!(
            "layer.property.value: 'color' needs a concrete colour on a layer (aci:1..=255, \
             #RRGGBB, r,g,b); '{value}' is symbolic and a layer has nothing to inherit from"
        )));
    }
    if let Some(raw) = strip_prefix_ignore_ascii_case(value, "aci:") {
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid_value("color", "an ACI index in 1..=255", value));
        }
        let index: u16 = raw
            .parse()
            .map_err(|_| invalid_value("color", "an ACI index in 1..=255", value))?;
        if !(1..=255).contains(&index) {
            return Err(invalid_value(
                "color",
                "an ACI index in 1..=255 (0 and 256 are symbolic)",
                value,
            ));
        }
        // One palette for entity and layer colours: the index and the sRGB
        // triple are written together so the row never carries a stale colour.
        let rgb = crate::property_value::rgb_for_aci(index)
            .ok_or_else(|| invalid_value("color", "an ACI index with a palette entry", value))?;
        return Ok(LayerColor {
            aci: Some(u8::try_from(index).expect("1..=255 fits a u8")),
            rgb,
        });
    }
    // `rgb:#RRGGBB` is the explicit spelling; a bare `#RRGGBB` is the same value.
    let hex = strip_prefix_ignore_ascii_case(value, "rgb:").unwrap_or(value);
    if let Some(raw) = hex.strip_prefix('#') {
        if raw.len() != 6 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid_value("color", "#RRGGBB", value));
        }
        let channel = |from: usize| {
            u8::from_str_radix(&raw[from..from + 2], 16)
                .expect("six ASCII hex digits are a valid u8 pair")
        };
        return Ok(LayerColor {
            aci: None,
            rgb: [channel(0), channel(2), channel(4)],
        });
    }
    if let Some(rgb) = parse_rgb_channels(hex) {
        return Ok(LayerColor { aci: None, rgb });
    }
    Err(invalid_value(
        "color",
        "aci:1..=255, #RRGGBB, rgb:#RRGGBB or r,g,b",
        value,
    ))
}

/// `r,g,b` with three decimal channels in `0..=255` (exactly three parts, no
/// spaces beyond the trim the caller already applied).
fn parse_rgb_channels(value: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (slot, part) in rgb.iter_mut().zip(parts) {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse::<u8>().ok()?;
    }
    Some(rgb)
}

fn linetype_by_name(database: &DrawingDatabase, name: &str) -> CadResult<String> {
    if name.eq_ignore_ascii_case("ByLayer") || name.eq_ignore_ascii_case("ByBlock") {
        return Err(CadError::InvalidInput(format!(
            "layer.property.value: a layer's linetype must be a named table entry; '{name}' is \
             symbolic and a layer has nothing to inherit from"
        )));
    }
    if let Some(entry) = database.linetypes().find(|entry| entry.name == name) {
        return Ok(entry.name.clone());
    }
    let matches: Vec<&DbLinetype> = database
        .linetypes()
        .filter(|entry| entry.name.eq_ignore_ascii_case(name))
        .collect();
    match matches.as_slice() {
        [] => Err(CadError::InvalidInput(format!(
            "layer.linetype_missing: this drawing has no linetype named '{name}'; a layer may \
             only reference a linetype the table defines"
        ))),
        [only] => Ok(only.name.clone()),
        _ => Err(CadError::InvalidInput(format!(
            "layer.linetype_missing: '{name}' matches {} linetypes case-insensitively; use the \
             exact name",
            matches.len()
        ))),
    }
}

fn parse_lineweight(value: &str) -> CadResult<LayerLineWeight> {
    if value.eq_ignore_ascii_case("ByLayer") {
        return Ok(LayerLineWeight::ByLayer);
    }
    if value.eq_ignore_ascii_case("Default") {
        return Ok(LayerLineWeight::Default);
    }
    // Digits only: no sign, no decimal point. `-1` (ByLayer) and `-2` (ByBlock)
    // are the DXF symbolic codes and are not accepted as numbers; `ByBlock` has
    // no layer variant at all.
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_value(
            "lineweight",
            "ByLayer, Default or a 1/100 mm integer in 0..=211",
            value,
        ));
    }
    let hundredths: u16 = value
        .parse()
        .map_err(|_| invalid_value("lineweight", "a 1/100 mm integer in 0..=211", value))?;
    if hundredths > 211 {
        return Err(invalid_value(
            "lineweight",
            "a 1/100 mm integer in 0..=211",
            value,
        ));
    }
    Ok(LayerLineWeight::Hundredths(hundredths))
}

fn parse_transparency(value: &str) -> CadResult<f32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_value(
            "transparency",
            "an integer 0..=100 (percent transparent)",
            value,
        ));
    }
    let percent: u16 = value.parse().map_err(|_| {
        invalid_value(
            "transparency",
            "an integer 0..=100 (percent transparent)",
            value,
        )
    })?;
    if percent > 100 {
        return Err(invalid_value(
            "transparency",
            "an integer 0..=100 (percent transparent)",
            value,
        ));
    }
    Ok(1.0 - f32::from(percent) / 100.0)
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}
