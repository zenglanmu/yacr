//! Elements row codecs: `serde_json::Value` ⇄ rows.
//!
//! Crate-private on purpose: only [`super::DocumentSnapshot`] crosses the store
//! boundary, so a store can never invent a payload shape the database does not
//! understand.
//!
//! One row is one [`super::ElementSnapshot::payload`]: the `cad:Element` envelope
//! of [`cad_domain::ymodel::descriptor::CAD_ELEMENT`] plus the element's geometry
//! payload (ADR-0037 D4/D8/D10). The field set is exactly the descriptor's:
//!
//! ```text
//! {"class":"cad:Element", "type_key":…, "space":…, "layer":…,
//!  "source_handle":…, "invisible":…, "render_attributes":…, "geometry":…}
//!
//! space: {"kind":"model"} | {"kind":"paper","id":<LayoutId>}
//!      | {"kind":"block","id":<BlockId>}     ids are persistent composites
//! ```
//!
//! Contract rules: (1) every identifier is a `(kind << 45) | seq` persistent id
//! ([`cad_domain::ymodel::PersistentId`]) and decoding checks the family, so an
//! entity id in `layer` is refused — a bare sequence would collide on disk;
//! (2) `space` is tagged because a Model id alone cannot tell paper from block,
//! and `"model"` carries no id (one there is an unknown field); (3) enum variants
//! are tagged objects with stable string tags (`{"kind":"explicit","value":…}`),
//! never `Debug` output; (4) numbers go through [`cad_domain::ymodel::json`], so
//! a legacy non-finite value survives as `{"$f64":"nan"}`; (5) decoding refuses
//! a missing field, an extra field, an unknown class or tag and a wrong JSON
//! type, because the writer re-serializes whole shards (ADR-0037 D11).
//!
//! # What a row does *not* carry
//!
//! The element's identity is the shard key ([`super::ElementSnapshot::id`]), not
//! a payload field; [`decode`] leaves sequence 0 for the hydrate path to replace.
//! `draw_order` and `DbObject::revision` are runtime values too: decode uses the
//! defaults `cad-db` builds rows with (`0`, `Revision(0)`).

use cad_domain::ymodel::descriptor::CAD_ELEMENT;
use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::ymodel::{json, payload};
use cad_domain::{
    BlockClip, BlockId, EntityId, GeometrySource, LayerId, LayoutId, ObjectId, Point3, Revision,
    SpaceId,
};
use serde_json::{Map, Number, Value};

use super::SnapshotError;
use crate::bim::BimElement;
use crate::entity::{
    AnnotativeAttributes, AnnotativePlacement, AnnotativeScaleOverride, DbEntity, DbObject,
    EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes, EntityTransparency,
    LinetypePattern,
};

/// The class key of this row. Part of the format.
const CLASS: &str = CAD_ELEMENT.type_key;

/// The optional per-element BIM block (ADR-0042 D2).
pub(crate) mod bim;

/// One decoded element row.
#[derive(Debug)]
pub(crate) struct DecodedElement {
    pub entity: DbEntity,
    pub attributes: Option<EntityRenderAttributes>,
    pub invisible: bool,
    /// `None` for a minor ≤ 5 row and for an element without BIM data.
    pub bim: Option<BimElement>,
}

/// Encode one element row.
///
/// The element's own persistent id is written into the payload: an entry must be
/// readable on its own (`unzip -p ... | jq`) without decoding a filename. Keys
/// are sorted by `serde_json`'s `BTreeMap`, so the output is deterministic.
///
/// `bim` is written **only** when the element has BIM data (ADR-0042 D2): an
/// absent block leaves the key out entirely, so the row keeps the shape a
/// minor-5 reader accepts.
pub(crate) fn encode(
    entity: &DbEntity,
    attributes: Option<&EntityRenderAttributes>,
    invisible: bool,
    bim: Option<&BimElement>,
) -> Result<Value, SnapshotError> {
    let type_key = Value::String(entity.object.type_key.clone());
    let space = encode_space(&entity.space);
    let layer = encode_id(entity.layer.persistent());
    let source_handle = entity
        .object
        .source_handle
        .as_deref()
        .map_or(Value::Null, |handle| Value::String(handle.to_string()));
    let render_attributes = attributes.map_or(Value::Null, encode_render_attributes);
    let mut row = fields(vec![
        ("class", Value::String(CLASS.to_string())),
        ("id", encode_id(entity.id.persistent())),
        ("type_key", type_key),
        ("space", space),
        ("layer", layer),
        ("source_handle", source_handle),
        ("invisible", Value::Bool(invisible)),
        ("render_attributes", render_attributes),
        ("geometry", payload::encode(&entity.geometry)),
    ]);
    if let Some(bim) = bim {
        match &mut row {
            Value::Object(map) => {
                map.insert("bim".to_string(), bim::encode(bim)?);
            }
            _ => unreachable!("`fields` builds an object"),
        }
    }
    Ok(row)
}

/// Decode one element row.
///
/// Strict: an absent required field, an unknown extra field, an unknown `class`
/// or tag, a wrong JSON type and an id of the wrong family are all errors named
/// after the field.
pub(crate) fn decode(value: &Value) -> Result<DecodedElement, SnapshotError> {
    let mut reader = Reader::new(value, CLASS)?;
    let class = reader.class()?.to_string();
    if class != CLASS {
        return Err(SnapshotError::invalid(format!(
            "{CLASS}: unknown class {class:?} (the row must be {CLASS:?})"
        )));
    }
    let _ = reader.take("class")?;

    let id = EntityId::from_seq(reader.id("id", IdKind::Entity)?);
    let type_key = reader.string("type_key")?;
    let space = reader.take_space("space")?;
    let layer = LayerId::from_seq(reader.id("layer", IdKind::Layer)?);
    let source_handle = reader.opt_string("source_handle")?;
    let invisible = reader.bool("invisible")?;
    let attributes = reader.take_render_attributes("render_attributes")?;

    let geometry_value = reader.take("geometry")?;
    let geometry = payload::decode(&geometry_value)
        .map_err(|error| SnapshotError::invalid(format!("{CLASS}.geometry: {error}")))?;

    // ADR-0042 D2/D6: the block is optional. An absent key is `None` (a minor
    // ≤ 5 row); a present key must be a complete `bim` object — `"bim": null` is
    // refused rather than treated as "no data", so this writer never produces a
    // shape the reader would have to guess about.
    let bim = match reader.opt_take("bim") {
        None => None,
        Some(value) => Some(bim::decode(&value).map_err(super::bim::prefixed)?),
    };

    reader.finish()?;

    Ok(DecodedElement {
        entity: DbEntity {
            object: DbObject {
                // `ObjectId` shares the entity sequence space, so it mirrors the
                // element id instead of being stored twice.
                id: ObjectId::from_seq(id.0),
                type_key,
                // Runtime only: seeded from the snapshot revision on load.
                revision: Revision(0),
                source_handle,
            },
            id,
            layer,
            space,
            geometry,
            // Runtime only: draw order is a display cache rebuilt from id order.
            draw_order: 0,
        },
        attributes,
        invisible,
        bim,
    })
}

fn encode_id(id: PersistentId) -> Value {
    Value::Number(Number::from(id.raw()))
}

fn encode_space(space: &SpaceId) -> Value {
    match space {
        SpaceId::Model => variant("model", Vec::new()),
        SpaceId::Paper(layout) => variant("paper", vec![("id", encode_id(layout.persistent()))]),
        SpaceId::Block(block) => variant("block", vec![("id", encode_id(block.persistent()))]),
    }
}

fn encode_render_attributes(attributes: &EntityRenderAttributes) -> Value {
    let linetype_scale = json::encode_f64(attributes.linetype_scale);
    let geometry_source = encode_geometry_source(&attributes.geometry_source);
    let clip = attributes.clip.as_ref().map_or(Value::Null, encode_clip);
    // `linetype_scale` uses the shared codec, so NaN survives as `$f64:"nan"`.
    fields(vec![
        ("transparency", encode_transparency(attributes.transparency)),
        ("color", encode_color(attributes.color)),
        ("lineweight", encode_lineweight(attributes.lineweight)),
        ("linetype", encode_linetype(&attributes.linetype)),
        ("linetype_scale", linetype_scale),
        ("geometry_source", geometry_source),
        ("annotative", encode_annotative(&attributes.annotative)),
        ("clip", clip),
    ])
}

fn encode_transparency(value: EntityTransparency) -> Value {
    match value {
        EntityTransparency::Explicit(opacity) => variant(
            "explicit",
            vec![("value", json::encode_f64(opacity as f64))],
        ),
        EntityTransparency::ByBlock => variant("byblock", Vec::new()),
        EntityTransparency::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn encode_color(value: EntityColor) -> Value {
    match value {
        EntityColor::Explicit([r, g, b]) => {
            let channels = vec![Value::from(r), Value::from(g), Value::from(b)];
            variant("explicit", vec![("value", Value::Array(channels))])
        }
        EntityColor::ByBlock => variant("byblock", Vec::new()),
        EntityColor::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn encode_lineweight(value: EntityLineWeight) -> Value {
    match value {
        EntityLineWeight::Explicit(millimeters) => variant(
            "explicit",
            vec![("value", json::encode_f64(millimeters as f64))],
        ),
        EntityLineWeight::Default => variant("default", Vec::new()),
        EntityLineWeight::ByBlock => variant("byblock", Vec::new()),
        EntityLineWeight::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn encode_linetype(value: &EntityLineType) -> Value {
    match value {
        EntityLineType::Explicit {
            name,
            pattern,
            scale,
        } => {
            let elements = pattern.elements.iter().copied().map(json::encode_f64);
            let pattern = fields(vec![
                ("elements", Value::Array(elements.collect())),
                ("cycle", json::encode_f64(pattern.cycle)),
            ]);
            variant(
                "explicit",
                vec![
                    ("name", Value::String(name.clone())),
                    ("pattern", pattern),
                    ("scale", json::encode_f64(*scale)),
                ],
            )
        }
        EntityLineType::ByBlock => variant("byblock", Vec::new()),
        EntityLineType::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn encode_geometry_source(source: &GeometrySource) -> Value {
    let kind = match source {
        GeometrySource::Analytic => "analytic",
        GeometrySource::DirectMesh => "direct_mesh",
        GeometrySource::ProxyCache => "proxy_cache",
        GeometrySource::KernelMesh => "kernel_mesh",
        GeometrySource::UserPoints => "user_points",
    };
    variant(kind, Vec::new())
}

fn encode_annotative(value: &AnnotativeAttributes) -> Value {
    let overrides = value.overrides.iter().map(encode_annotative_override);
    fields(vec![
        ("annotative", Value::Bool(value.annotative)),
        ("overrides", Value::Array(overrides.collect())),
    ])
}

fn encode_annotative_override(value: &AnnotativeScaleOverride) -> Value {
    let height = value.placement.height.map_or(Value::Null, json::encode_f64);
    let placement = fields(vec![
        ("position", json::encode_point3(value.placement.position)),
        ("rotation", json::encode_f64(value.placement.rotation)),
        ("height", height),
    ]);
    fields(vec![
        ("scale", Value::String(value.scale.clone())),
        ("placement", placement),
    ])
}

fn encode_clip(clip: &BlockClip) -> Value {
    let vertices = Value::Array(
        clip.vertices
            .iter()
            .map(|v| json::encode_f64x2(*v))
            .collect(),
    );
    fields(vec![
        ("vertices", vertices),
        ("inside", Value::Bool(clip.inside)),
        ("visible", Value::Bool(clip.visible)),
        (
            "front_clip",
            clip.front_clip.map_or(Value::Null, json::encode_f64),
        ),
        (
            "back_clip",
            clip.back_clip.map_or(Value::Null, json::encode_f64),
        ),
    ])
}

/// A plain JSON object with exactly the given fields (no `class` key).
fn fields(entries: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (name, value) in entries {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate object field {name}");
    }
    Value::Object(map)
}

/// A tagged enum variant: `{"kind": <kind>, <fields…>}`.
fn variant(kind: &str, entries: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    map.insert("kind".to_string(), Value::String(kind.to_string()));
    for (name, value) in entries {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate variant field {name}");
    }
    Value::Object(map)
}

/// A strict reader for one JSON object: fields are taken by name and
/// [`Reader::finish`] refuses anything left over.
///
/// `pub(crate)` because the two BIM codecs (`snapshot/bim.rs` and
/// `snapshot/elements/bim.rs`) reuse it: one strict-reader implementation, so
/// the element block and the definition rows cannot disagree about what "extra
/// field" or "wrong type" means.
pub(crate) struct Reader {
    map: Map<String, Value>,
    context: String,
}

impl Reader {
    pub(crate) fn new(value: &Value, context: impl Into<String>) -> Result<Reader, SnapshotError> {
        let context = context.into();
        match value {
            Value::Object(map) => Ok(Reader {
                map: map.clone(),
                context,
            }),
            _ => Err(SnapshotError::invalid(format!(
                "{context}: expected a JSON object"
            ))),
        }
    }

    /// The path this reader names its refusals after.
    pub(crate) fn context(&self) -> &str {
        &self.context
    }

    pub(crate) fn error(&self, detail: impl std::fmt::Display) -> SnapshotError {
        SnapshotError::invalid(format!("{}: {detail}", self.context))
    }

    fn unknown_kind(&self, found: &str, expected: &str) -> SnapshotError {
        self.error(format!("unknown kind {found:?} (expected {expected})"))
    }

    /// The `class` discriminator, without consuming it.
    fn class(&self) -> Result<&str, SnapshotError> {
        match self.map.get("class") {
            Some(Value::String(class)) => Ok(class),
            Some(_) => Err(self.error("class: expected a string")),
            None => Err(self.error("missing required field \"class\"")),
        }
    }

    /// Take a field that must be present.
    pub(crate) fn take(&mut self, field: &'static str) -> Result<Value, SnapshotError> {
        let value = self.map.remove(field);
        value.ok_or_else(|| self.error(format!("missing required field {field:?}")))
    }

    /// Take a field that may be absent: `None` means the key was not there.
    ///
    /// This is the only optional read in an element payload (ADR-0042 D2's
    /// `bim` block). It does **not** treat `null` as absent — a caller that
    /// accepts null checks the value itself — so an existing key is always
    /// returned and the caller decides whether its shape is acceptable.
    pub(crate) fn opt_take(&mut self, field: &'static str) -> Option<Value> {
        self.map.remove(field)
    }

    pub(crate) fn finish(self) -> Result<(), SnapshotError> {
        // `Map` is a `BTreeMap`, so this is the first key in sorted order.
        match self.map.keys().next() {
            Some(key) => Err(SnapshotError::invalid(format!(
                "{}: unknown field {key:?}",
                self.context
            ))),
            None => Ok(()),
        }
    }

    pub(crate) fn string(&mut self, field: &'static str) -> Result<String, SnapshotError> {
        match self.take(field)? {
            Value::String(text) => Ok(text),
            _ => Err(self.error(format!("{field}: expected a string"))),
        }
    }

    fn opt_string(&mut self, field: &'static str) -> Result<Option<String>, SnapshotError> {
        match self.take(field)? {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            _ => Err(self.error(format!("{field}: expected a string or null"))),
        }
    }

    pub(crate) fn bool(&mut self, field: &'static str) -> Result<bool, SnapshotError> {
        match self.take(field)? {
            Value::Bool(value) => Ok(value),
            _ => Err(self.error(format!("{field}: expected a boolean"))),
        }
    }

    /// A JSON integer (`i64`, the range the format's property values use).
    pub(crate) fn integer(&mut self, field: &'static str) -> Result<i64, SnapshotError> {
        match self.take(field)? {
            Value::Number(value) => value
                .as_i64()
                .ok_or_else(|| self.error(format!("{field}: expected a JSON integer"))),
            _ => Err(self.error(format!("{field}: expected a JSON integer"))),
        }
    }

    pub(crate) fn f64(&mut self, field: &'static str) -> Result<f64, SnapshotError> {
        let value = self.take(field)?;
        json::decode_f64(&value).map_err(|error| self.error(format!("{field}: {error}")))
    }

    pub(crate) fn opt_f64(&mut self, field: &'static str) -> Result<Option<f64>, SnapshotError> {
        let value = self.take(field)?;
        json::decode_opt_f64(&value).map_err(|error| self.error(format!("{field}: {error}")))
    }

    /// An `f32` written by the encoder, which stores the exact `f64` widening.
    /// A value that no `f32` could have produced is refused.
    fn f32(&mut self, field: &'static str) -> Result<f32, SnapshotError> {
        let value = self.f64(field)?;
        if value.is_finite() {
            let narrowed = value as f32;
            if (narrowed as f64).to_bits() == value.to_bits() {
                return Ok(narrowed);
            }
            return Err(self.error(format!("{field}: not exactly representable as an f32")));
        }
        Ok(value as f32)
    }

    fn point3(&mut self, field: &'static str) -> Result<Point3, SnapshotError> {
        let value = self.take(field)?;
        json::decode_point3(&value).map_err(|error| self.error(format!("{field}: {error}")))
    }

    pub(crate) fn array(&mut self, field: &'static str) -> Result<Vec<Value>, SnapshotError> {
        match self.take(field)? {
            Value::Array(items) => Ok(items),
            _ => Err(self.error(format!("{field}: expected an array"))),
        }
    }

    fn u8_triple(&mut self, field: &'static str) -> Result<[u8; 3], SnapshotError> {
        let items = self.array(field)?;
        if items.len() != 3 {
            return Err(self.error(format!("{field}: expected an array of three bytes")));
        }
        let mut bytes = [0u8; 3];
        for (slot, item) in bytes.iter_mut().zip(&items) {
            let byte = item
                .as_u64()
                .and_then(|raw| u8::try_from(raw).ok())
                .ok_or_else(|| self.error(format!("{field}: expected a byte (0..=255)")))?;
            *slot = byte;
        }
        Ok(bytes)
    }

    fn f64_array(&mut self, field: &'static str) -> Result<Vec<f64>, SnapshotError> {
        self.array(field)?
            .iter()
            .map(|item| {
                json::decode_f64(item).map_err(|error| self.error(format!("{field}: {error}")))
            })
            .collect()
    }

    fn f64x2_array(&mut self, field: &'static str) -> Result<Vec<[f64; 2]>, SnapshotError> {
        self.array(field)?
            .iter()
            .map(|item| {
                json::decode_f64x2(item).map_err(|error| self.error(format!("{field}: {error}")))
            })
            .collect()
    }

    /// A required persistent identifier of the given family.
    ///
    /// The JSON number is decoded as a composite and its kind is checked, so an
    /// `EntityId` written into `layer` is an error instead of a silent `LayerId`.
    fn id(&mut self, field: &'static str, kind: IdKind) -> Result<u128, SnapshotError> {
        let value = self.take(field)?;
        let raw = value
            .as_i64()
            .ok_or_else(|| self.error(format!("{field}: expected a JSON integer identifier")))?;
        let id = PersistentId::from_raw(raw).ok_or_else(|| {
            self.error(format!(
                "{field}: {raw} is not a persistent identifier this format knows"
            ))
        })?;
        if id.kind() != kind {
            return Err(self.error(format!(
                "{field}: identifier {raw} is a {:?}, not a {kind:?}",
                id.kind()
            )));
        }
        Ok(id.seq())
    }

    fn take_space(&mut self, field: &'static str) -> Result<SpaceId, SnapshotError> {
        let value = self.take(field)?;
        let mut reader = Reader::new(&value, format!("{}.{field}", self.context))?;
        let kind = reader.string("kind")?;
        let space = match kind.as_str() {
            "model" => SpaceId::Model,
            "paper" => SpaceId::Paper(LayoutId::from_seq(reader.id("id", IdKind::Layout)?)),
            "block" => SpaceId::Block(BlockId::from_seq(reader.id("id", IdKind::Block)?)),
            _ => return Err(reader.unknown_kind(&kind, "model, paper or block")),
        };
        // `finish` is what refuses an `id` on a model space: it is a leftover.
        reader.finish()?;
        Ok(space)
    }

    fn take_render_attributes(
        &mut self,
        field: &'static str,
    ) -> Result<Option<EntityRenderAttributes>, SnapshotError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        let mut reader = Reader::new(&value, format!("{}.{field}", self.context))?;
        let attributes = EntityRenderAttributes {
            transparency: reader.take_transparency("transparency")?,
            color: reader.take_color("color")?,
            lineweight: reader.take_lineweight("lineweight")?,
            linetype: reader.take_linetype("linetype")?,
            linetype_scale: reader.f64("linetype_scale")?,
            geometry_source: reader.take_geometry_source("geometry_source")?,
            annotative: reader.take_annotative("annotative")?,
            clip: reader.take_clip("clip")?,
        };
        reader.finish()?;
        Ok(Some(attributes))
    }

    fn take_transparency(
        &mut self,
        field: &'static str,
    ) -> Result<EntityTransparency, SnapshotError> {
        let value = self.take(field)?;
        variant_object(value, format!("{}.{field}", self.context), |reader| {
            let kind = reader.string("kind")?;
            match kind.as_str() {
                "explicit" => Ok(EntityTransparency::Explicit(reader.f32("value")?)),
                "byblock" => Ok(EntityTransparency::ByBlock),
                "bylayer" => Ok(EntityTransparency::ByLayer),
                _ => Err(reader.unknown_kind(&kind, "explicit, byblock or bylayer")),
            }
        })
    }

    fn take_color(&mut self, field: &'static str) -> Result<EntityColor, SnapshotError> {
        let value = self.take(field)?;
        variant_object(value, format!("{}.{field}", self.context), |reader| {
            let kind = reader.string("kind")?;
            match kind.as_str() {
                "explicit" => Ok(EntityColor::Explicit(reader.u8_triple("value")?)),
                "byblock" => Ok(EntityColor::ByBlock),
                "bylayer" => Ok(EntityColor::ByLayer),
                _ => Err(reader.unknown_kind(&kind, "explicit, byblock or bylayer")),
            }
        })
    }

    fn take_lineweight(&mut self, field: &'static str) -> Result<EntityLineWeight, SnapshotError> {
        let value = self.take(field)?;
        variant_object(value, format!("{}.{field}", self.context), |reader| {
            let kind = reader.string("kind")?;
            match kind.as_str() {
                "explicit" => Ok(EntityLineWeight::Explicit(reader.f32("value")?)),
                "default" => Ok(EntityLineWeight::Default),
                "byblock" => Ok(EntityLineWeight::ByBlock),
                "bylayer" => Ok(EntityLineWeight::ByLayer),
                _ => Err(reader.unknown_kind(&kind, "explicit, default, byblock or bylayer")),
            }
        })
    }

    fn take_linetype(&mut self, field: &'static str) -> Result<EntityLineType, SnapshotError> {
        let value = self.take(field)?;
        variant_object(value, format!("{}.{field}", self.context), |reader| {
            let kind = reader.string("kind")?;
            match kind.as_str() {
                "explicit" => {
                    let name = reader.string("name")?;
                    let pattern_value = reader.take("pattern")?;
                    let pattern = {
                        let mut pattern_reader = reader.child(&pattern_value, "pattern")?;
                        let elements = pattern_reader.f64_array("elements")?;
                        let cycle = pattern_reader.f64("cycle")?;
                        pattern_reader.finish()?;
                        LinetypePattern { elements, cycle }
                    };
                    let scale = reader.f64("scale")?;
                    Ok(EntityLineType::Explicit {
                        name,
                        pattern,
                        scale,
                    })
                }
                "byblock" => Ok(EntityLineType::ByBlock),
                "bylayer" => Ok(EntityLineType::ByLayer),
                _ => Err(reader.unknown_kind(&kind, "explicit, byblock or bylayer")),
            }
        })
    }

    fn take_geometry_source(
        &mut self,
        field: &'static str,
    ) -> Result<GeometrySource, SnapshotError> {
        let value = self.take(field)?;
        variant_object(value, format!("{}.{field}", self.context), |reader| {
            let kind = reader.string("kind")?;
            match kind.as_str() {
                "analytic" => Ok(GeometrySource::Analytic),
                "direct_mesh" => Ok(GeometrySource::DirectMesh),
                "proxy_cache" => Ok(GeometrySource::ProxyCache),
                "kernel_mesh" => Ok(GeometrySource::KernelMesh),
                "user_points" => Ok(GeometrySource::UserPoints),
                _ => Err(reader.unknown_kind(
                    &kind,
                    "analytic, direct_mesh, proxy_cache, kernel_mesh or user_points",
                )),
            }
        })
    }

    fn take_annotative(
        &mut self,
        field: &'static str,
    ) -> Result<AnnotativeAttributes, SnapshotError> {
        let value = self.take(field)?;
        let path = format!("{}.{field}", self.context);
        let mut reader = Reader::new(&value, path.clone())?;
        let annotative = reader.bool("annotative")?;
        let overrides = reader
            .array("overrides")?
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                decode_annotative_override(value, format!("{path}.overrides[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        reader.finish()?;
        Ok(AnnotativeAttributes {
            annotative,
            overrides,
        })
    }

    fn take_clip(&mut self, field: &'static str) -> Result<Option<BlockClip>, SnapshotError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        let mut reader = Reader::new(&value, format!("{}.{field}", self.context))?;
        let clip = BlockClip {
            vertices: reader.f64x2_array("vertices")?,
            inside: reader.bool("inside")?,
            visible: reader.bool("visible")?,
            front_clip: reader.opt_f64("front_clip")?,
            back_clip: reader.opt_f64("back_clip")?,
        };
        reader.finish()?;
        Ok(Some(clip))
    }

    /// A nested object, with its path joined onto this reader's context.
    pub(crate) fn child(
        &self,
        value: &Value,
        field: &'static str,
    ) -> Result<Reader, SnapshotError> {
        Reader::new(value, format!("{}.{field}", self.context))
    }
}

/// Read one tagged variant object, then refuse leftovers.
fn variant_object<T>(
    value: Value,
    path: String,
    read: impl FnOnce(&mut Reader) -> Result<T, SnapshotError>,
) -> Result<T, SnapshotError> {
    let mut reader = Reader::new(&value, path)?;
    let decoded = read(&mut reader)?;
    reader.finish()?;
    Ok(decoded)
}

fn decode_annotative_override(
    value: Value,
    path: String,
) -> Result<AnnotativeScaleOverride, SnapshotError> {
    let mut reader = Reader::new(&value, path)?;
    let scale = reader.string("scale")?;
    let placement_value = reader.take("placement")?;
    let placement = {
        let mut placement_reader = reader.child(&placement_value, "placement")?;
        let placement = AnnotativePlacement {
            position: placement_reader.point3("position")?,
            rotation: placement_reader.f64("rotation")?,
            height: placement_reader.opt_f64("height")?,
        };
        placement_reader.finish()?;
        placement
    };
    reader.finish()?;
    Ok(AnnotativeScaleOverride { scale, placement })
}

#[cfg(test)]
mod tests;
