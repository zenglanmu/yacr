//! The writable property surface of the properties panel: one accepted grammar
//! per key, plus the canonical projection a reader sees.
//!
//! Every value the write path accepts is a **string with a documented grammar**
//! (never a loosely parsed number), and every value it writes has a **canonical
//! form** this module also produces. A caller can therefore read a value,
//! present it, and write it back unchanged: the same write applied twice is the
//! same state (idempotent), and no input is silently clamped or rounded.
//!
//! ## Accepted grammar (case-sensitive canonical form; symbols and the `aci:`
//! ## / hex prefixes are matched case-insensitively on input)
//!
//! | key | accepted value | stored |
//! |---|---|---|
//! | `layer` | an existing layer **name** (exact first, then a unique ASCII case-insensitive match) | [`DbEntity::layer`] |
//! | `color` | `ByLayer`, `ByBlock`, `aci:N` (`N` in `0..=256`), `#RRGGBB` | [`EntityColor`] |
//! | `linetype` | `ByLayer`, `ByBlock`, or an existing linetype **name** | [`EntityLineType`] |
//! | `lineweight` | `ByLayer`, `ByBlock`, `Default`, or a canonical millimetre value (below) | [`EntityLineWeight`] |
//! | `transparency` | a decimal integer `0..=90` (transparency **percent**) | [`EntityTransparency`] |
//!
//! `text` is deliberately **not** accepted here: it has its own command
//! (`TEXTEDIT` / `set_selected_text`) with its own newline policy.
//!
//! Names are canonicalised to the drawing table's own spelling: an exact match
//! wins, a case-insensitive fallback is accepted only when it is unambiguous, and
//! the stored/printed name is the table entry's. `ByLayer` / `ByBlock` stay
//! symbolic for every attribute — a linetype the importer resolved to a pattern
//! while its source said `ByLayer` reads as `ByLayer` and writes back as the
//! symbolic value.
//!
//! ## Storage conversions (done exactly once, in one place)
//!
//! * **color** — ACI `0` is ByBlock and ACI `256` is ByLayer (both symbolic).
//!   ACI `1..=255` maps to the canonical sRGB triple in [`aci`]; `#RRGGBB` is
//!   stored verbatim. The canonical projection prints an ACI index only when the
//!   stored sRGB triple is an exact palette entry, using the **lowest** such
//!   index (the palette repeats some colours, so `aci:10` canonicalises to
//!   `aci:1`); every other true colour prints as `#RRGGBB`.
//! * **lineweight** — the database stores millimetres in `f32` (the importer
//!   divides the source's 1/100 mm integer by 100). Input is matched against the
//!   canonical AutoCAD list (`0.00, 0.05, 0.09, 0.13, 0.15, 0.18, 0.20, 0.25,
//!   0.30, 0.35, 0.40, 0.50, 0.53, 0.60, 0.70, 0.80, 0.90, 1.00, 1.06, 1.20,
//!   1.40, 1.58, 2.00, 2.11` mm); the stored value is that constant, not a
//!   re-parse of the string. Anything else is refused, never snapped.
//! * **transparency** — the input is the AutoCAD **percent transparent**
//!   (`0` = opaque, `90` = most transparent). The database stores **opacity**
//!   (`1.0` = opaque), so the conversion is `opacity = 1.0 − percent / 100`; the
//!   canonical projection is `round((1.0 − opacity) · 100)`. The no-op rule
//!   compares the projected **percentage**, so re-writing the value the panel
//!   displays never creates an undo entry even when the stored float was
//!   quantised from an imported transparency byte.
//!
//! ## Why `layer` prints a name but the row prints `name (id)`
//!
//! [`PropertyRow`](crate::PropertyRow) is display text (`"WALLS (3)"`); a write
//! target must be the **name**. [`property_value`] returns the round-trippable
//! name, so a consumer reads it from here and never from the display row.
//!
//! ## How the panel reads these values
//!
//! The `color`, `linetype`, `lineweight` and `transparency` rows emitted by
//! `crate::selection` carry **this module's projection verbatim**, so the string
//! a row shows can be sent straight back to `SETENTITYPROPERTY` unchanged
//! (a re-write of the same value is a no-op). `layer` is the documented
//! exception above: its row keeps the display text and a writer takes the bare
//! name from [`property_value`].

mod aci;

use cad_db::{
    DbEntity, DrawingDatabase, EntityColor, EntityLineType, EntityLineWeight, EntityTransparency,
    LinetypePattern,
};
use cad_domain::{CadError, CadResult, LayerId};

/// The property keys this write path accepts, in panel order.
pub const SUPPORTED_KEYS: [&str; 5] = ["layer", "color", "linetype", "lineweight", "transparency"];

/// The canonical AutoCAD lineweights in millimetres (source values are 1/100 mm
/// integers `0, 5, 9, …, 211`). Nothing outside this list is writable.
pub const LINEWEIGHTS_MM: [f32; 24] = [
    0.00, 0.05, 0.09, 0.13, 0.15, 0.18, 0.20, 0.25, 0.30, 0.35, 0.40, 0.50, 0.53, 0.60, 0.70, 0.80,
    0.90, 1.00, 1.06, 1.20, 1.40, 1.58, 2.00, 2.11,
];

/// The symbolic values the `linetype` grammar accepts besides a named table
/// entry. They are also part of the panel's pickable set, because a row whose
/// stored value is symbolic must remain representable.
pub const LINETYPE_SYMBOLS: [&str; 2] = ["ByLayer", "ByBlock"];

/// The symbolic values the `lineweight` grammar accepts besides the canonical
/// millimetre list.
pub const LINEWEIGHT_SYMBOLS: [&str; 3] = ["ByLayer", "ByBlock", "Default"];

/// Every value the `lineweight` grammar accepts, printed exactly like
/// [`lineweight_value`] prints it: the symbolic values plus [`LINEWEIGHTS_MM`].
///
/// This is the **single shared source** for the row's `enum` options and the
/// parser's accepted set, so a listed weight can never be refused and an
/// accepted weight can never be missing from the picker.
pub fn lineweight_options() -> Vec<String> {
    LINEWEIGHT_SYMBOLS
        .into_iter()
        .map(str::to_string)
        .chain(LINEWEIGHTS_MM.into_iter().map(|mm| format!("{mm:.2}")))
        .collect()
}

/// The largest accepted transparency percentage (AutoCAD's own bound).
pub const MAX_TRANSPARENCY_PERCENT: u8 = 90;

/// Whether `key` names a property this write path accepts.
pub fn is_supported_key(key: &str) -> bool {
    SUPPORTED_KEYS.contains(&key)
}

/// Whether the write path accepts `value` for `key` against `database`.
///
/// This is the parser's own verdict, exposed for the read side: a row may
/// project a stored value that no grammar can express (a symbolic `ByBlock`
/// transparency, or an out-of-range stored opacity reported as
/// `opacity:<raw>`), and such a row is **not** editable even though the key
/// itself is writable.
pub fn accepts(database: &DrawingDatabase, key: &str, value: &str) -> bool {
    parse(database, key, value).is_ok()
}

/// A validated write target, already resolved against the drawing's tables.
///
/// Resolution happens **once** per command (before any transaction) so a
/// refusal cannot depend on which entity is processed first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PropertyValue {
    Layer(LayerId),
    Color(EntityColor),
    Linetype(LinetypeTarget),
    Lineweight(EntityLineWeight),
    Transparency(EntityTransparency),
}

/// A linetype write target: symbolic, or a named table entry with its resolved
/// dash pattern. The per-entity scale is preserved by the caller, because it is
/// an entity value, not a table value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LinetypeTarget {
    ByLayer,
    ByBlock,
    Named {
        name: String,
        pattern: LinetypePattern,
    },
}

/// Parse and resolve one `(key, value)` pair against `database`.
///
/// Refusals carry a stable code:
///
/// * `property.unsupported_key` — `text` (its own path) or any other key;
/// * `property.unknown_layer` / `property.unknown_linetype` — the name is not in
///   the drawing's table;
/// * `property.ambiguous_name` — a case-insensitive fallback that matches more
///   than one table entry (the exact name is always tried first);
/// * `property.invalid_value` — the grammar/range does not match the key.
pub(crate) fn parse(
    database: &DrawingDatabase,
    key: &str,
    value: &str,
) -> CadResult<PropertyValue> {
    match key {
        "layer" => Ok(PropertyValue::Layer(layer_by_name(database, value)?)),
        "color" => Ok(PropertyValue::Color(parse_color(value)?)),
        "linetype" => Ok(PropertyValue::Linetype(parse_linetype(database, value)?)),
        "lineweight" => Ok(PropertyValue::Lineweight(parse_lineweight(value)?)),
        "transparency" => Ok(PropertyValue::Transparency(parse_transparency(value)?)),
        "text" => Err(CadError::Unsupported(
            "property.unsupported_key: text content has its own command (TEXTEDIT); \
             the property write path does not change it"
                .into(),
        )),
        other => Err(CadError::Unsupported(format!(
            "property.unsupported_key: '{other}' is not a writable property"
        ))),
    }
}

/// The canonical, round-trippable string of `key` for one entity.
///
/// `None` when the key is not writable or the value cannot be expressed in the
/// accepted grammar (an unresolved layer, for example). The projection never
/// clamps: a stored opacity outside `0..=1` is reported as `opacity:<raw>` and
/// is not accepted back, so a caller can see the invariant violation instead of
/// a silently rounded percentage.
pub fn property_value(database: &DrawingDatabase, entity: &DbEntity, key: &str) -> Option<String> {
    match key {
        "layer" => database.layer(entity.layer).map(|layer| layer.name.clone()),
        "color" => {
            let attributes = database.entity_render_attributes(entity.id);
            Some(color_value(attributes.color))
        }
        "linetype" => {
            let attributes = database.entity_render_attributes(entity.id);
            match &attributes.linetype {
                // An unnamed explicit pattern (an imported name-less definition)
                // has no name the write grammar accepts: report no writable value
                // rather than an empty string.
                EntityLineType::Explicit { name, .. } if name.trim().is_empty() => None,
                other => Some(linetype_value(other)),
            }
        }
        "lineweight" => {
            let attributes = database.entity_render_attributes(entity.id);
            Some(lineweight_value(attributes.lineweight))
        }
        "transparency" => {
            let attributes = database.entity_render_attributes(entity.id);
            Some(transparency_value(attributes.transparency))
        }
        _ => None,
    }
}

/// Canonical projection of a stored colour.
pub fn color_value(color: EntityColor) -> String {
    match color {
        EntityColor::ByLayer => "ByLayer".to_string(),
        EntityColor::ByBlock => "ByBlock".to_string(),
        EntityColor::Explicit([r, g, b]) => match aci::aci_for_rgb([r, g, b]) {
            Some(index) => format!("aci:{index}"),
            None => format!("#{r:02X}{g:02X}{b:02X}"),
        },
    }
}

/// Canonical projection of a stored linetype.
pub fn linetype_value(linetype: &EntityLineType) -> String {
    match linetype {
        EntityLineType::ByLayer => "ByLayer".to_string(),
        EntityLineType::ByBlock => "ByBlock".to_string(),
        EntityLineType::Explicit { name, .. } => name.clone(),
    }
}

/// Canonical projection of a stored lineweight.
pub fn lineweight_value(lineweight: EntityLineWeight) -> String {
    match lineweight {
        EntityLineWeight::ByLayer => "ByLayer".to_string(),
        EntityLineWeight::ByBlock => "ByBlock".to_string(),
        EntityLineWeight::Default => "Default".to_string(),
        EntityLineWeight::Explicit(millimetres) => {
            match canonical_lineweight(millimetres) {
                Some(canonical) => format!("{canonical:.2}"),
                // Not a canonical weight: report the stored value honestly
                // rather than snapping it to a neighbour.
                None => format!("{millimetres:.4}"),
            }
        }
    }
}

/// Canonical projection of a stored transparency.
pub fn transparency_value(transparency: EntityTransparency) -> String {
    match transparency {
        EntityTransparency::ByBlock => "ByBlock".to_string(),
        EntityTransparency::Explicit(opacity) => {
            if opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
                let percent = ((1.0 - opacity) * 100.0).round();
                format!("{percent:.0}")
            } else {
                format!("opacity:{opacity}")
            }
        }
    }
}

fn parse_color(value: &str) -> CadResult<EntityColor> {
    if value.eq_ignore_ascii_case("ByLayer") {
        return Ok(EntityColor::ByLayer);
    }
    if value.eq_ignore_ascii_case("ByBlock") {
        return Ok(EntityColor::ByBlock);
    }
    // `aci:N` with N in 0..=256; 0/256 are the symbolic variants, 1..=255 the
    // canonical palette entry.
    if let Some(raw) = strip_prefix_ignore_ascii_case(value, "aci:") {
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(CadError::InvalidInput(format!(
                "property.invalid_value: color needs an ACI index in 0..=256, got '{value}'"
            )));
        }
        let index: u16 = raw.parse().map_err(|_| {
            CadError::InvalidInput(format!(
                "property.invalid_value: color needs an ACI index in 0..=256, got '{value}'"
            ))
        })?;
        if index > 256 {
            return Err(CadError::InvalidInput(format!(
                "property.invalid_value: ACI index {index} is outside 0..=256"
            )));
        }
        return match index {
            0 => Ok(EntityColor::ByBlock),
            256 => Ok(EntityColor::ByLayer),
            index => aci::rgb_for_aci(index)
                .map(EntityColor::Explicit)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "property.invalid_value: ACI index {index} has no palette entry"
                    ))
                }),
        };
    }
    if let Some(raw) = value.strip_prefix('#') {
        if raw.len() != 6 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CadError::InvalidInput(format!(
                "property.invalid_value: color needs #RRGGBB, got '{value}'"
            )));
        }
        let channel = |from: usize| {
            u8::from_str_radix(&raw[from..from + 2], 16)
                .expect("six ASCII hex digits are a valid u8 pair")
        };
        return Ok(EntityColor::Explicit([channel(0), channel(2), channel(4)]));
    }
    Err(CadError::InvalidInput(format!(
        "property.invalid_value: color needs ByLayer, ByBlock, aci:0..=256 or #RRGGBB, \
         got '{value}'"
    )))
}

fn parse_linetype(database: &DrawingDatabase, value: &str) -> CadResult<LinetypeTarget> {
    if value.eq_ignore_ascii_case("ByLayer") {
        return Ok(LinetypeTarget::ByLayer);
    }
    if value.eq_ignore_ascii_case("ByBlock") {
        return Ok(LinetypeTarget::ByBlock);
    }
    let entry = linetype_by_name(database, value)?;
    Ok(LinetypeTarget::Named {
        name: entry.name.clone(),
        pattern: entry.pattern.clone(),
    })
}

fn parse_lineweight(value: &str) -> CadResult<EntityLineWeight> {
    if value.eq_ignore_ascii_case("ByLayer") {
        return Ok(EntityLineWeight::ByLayer);
    }
    if value.eq_ignore_ascii_case("ByBlock") {
        return Ok(EntityLineWeight::ByBlock);
    }
    if value.eq_ignore_ascii_case("Default") {
        return Ok(EntityLineWeight::Default);
    }
    let millimetres: f32 = value.parse().map_err(|_| invalid_lineweight(value))?;
    canonical_lineweight(millimetres)
        .map(EntityLineWeight::Explicit)
        .ok_or_else(|| invalid_lineweight(value))
}

fn invalid_lineweight(value: &str) -> CadError {
    CadError::InvalidInput(format!(
        "property.invalid_value: lineweight needs ByLayer, ByBlock, Default or one of the \
         canonical millimetre values (0.00 ..= 2.11), got '{value}'"
    ))
}

/// The canonical constant equal to `millimetres`, if the value is exactly one
/// of [`LINEWEIGHTS_MM`]. The comparison is a tolerance on the printed value
/// (`1e-4` mm), far below the 0.01 mm quantisation of the source, so a string
/// such as `"0.35"` always resolves to the canonical constant.
fn canonical_lineweight(millimetres: f32) -> Option<f32> {
    if !millimetres.is_finite() {
        return None;
    }
    LINEWEIGHTS_MM
        .into_iter()
        .find(|canonical| (canonical - millimetres).abs() < 1e-4)
}

fn parse_transparency(value: &str) -> CadResult<EntityTransparency> {
    // Digits only: no sign, no decimal point, no whitespace. `-1`/`91`/`abc`
    // are refusals, never clamped.
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_transparency(value));
    }
    let percent: u8 = value.parse().map_err(|_| invalid_transparency(value))?;
    if percent > MAX_TRANSPARENCY_PERCENT {
        return Err(invalid_transparency(value));
    }
    Ok(EntityTransparency::Explicit(
        1.0 - f32::from(percent) / 100.0,
    ))
}

fn invalid_transparency(value: &str) -> CadError {
    CadError::InvalidInput(format!(
        "property.invalid_value: transparency needs an integer 0..=90 (percent), got '{value}'"
    ))
}

fn layer_by_name(database: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    if let Some(layer) = database.layers().find(|layer| layer.name == name) {
        return Ok(layer.id);
    }
    let matches: Vec<LayerId> = database
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name))
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [] => Err(CadError::InvalidInput(format!(
            "property.unknown_layer: this drawing has no layer named '{name}'"
        ))),
        [only] => Ok(*only),
        _ => Err(CadError::InvalidInput(format!(
            "property.ambiguous_name: '{name}' matches {} layers case-insensitively; \
             use the exact name",
            matches.len()
        ))),
    }
}

fn linetype_by_name<'a>(
    database: &'a DrawingDatabase,
    name: &str,
) -> CadResult<&'a cad_db::LineType> {
    if let Some(entry) = database.linetypes().find(|entry| entry.name == name) {
        return Ok(entry);
    }
    let matches: Vec<&cad_db::LineType> = database
        .linetypes()
        .filter(|entry| entry.name.eq_ignore_ascii_case(name))
        .collect();
    match matches.as_slice() {
        [] => Err(CadError::InvalidInput(format!(
            "property.unknown_linetype: this drawing has no linetype named '{name}'"
        ))),
        [only] => Ok(only),
        _ => Err(CadError::InvalidInput(format!(
            "property.ambiguous_name: '{name}' matches {} linetypes case-insensitively; \
             use the exact name",
            matches.len()
        ))),
    }
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}
