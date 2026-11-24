//! Document header variables (`document.json` → `variables`).
//!
//! These are the document-level scalars that are neither element rows nor table
//! rows: linetype scale, point display mode/size, insertion base, model limits,
//! code page and the active annotation scale. They live in the header rather
//! than in a table so a reader finds every document-level scalar in one place.
//!
//! `cad_domain::ymodel::DocumentHeader::variables` is an opaque object on the
//! domain side; this module owns its shape, so adding a variable does not touch
//! `cad-domain` and does not need a format bump as long as the key is new.
//!
//! # Keys (part of the format)
//!
//! ```text
//! linetype_scale          f64            $LTSCALE
//! point_display_mode      i16            $PDMODE, raw
//! point_display_size      f64            $PDSIZE, raw (may be non-finite)
//! insertion_base          null | {x,y,z} $INSBASE; null is a real "not stored"
//! limits_min              null | [x,y]   $LIMMIN
//! limits_max              null | [x,y]   $LIMMAX
//! code_page               null | string  $DWGCODEPAGE, raw
//! active_annotation_scale null | {name, value, named, scale}
//! metadata                absent | {title?, author?, subject?, keywords?,
//!                         comments?, custom?: {name: value}}
//! ```
//!
//! `metadata` is the `DWGPROPS` summary block. It is **absent** when nothing is
//! stored, so a document with no metadata writes exactly the pre-metadata
//! header; it needs no format bump. Fields inside the block are optional
//! (absent = `None`), and `custom` is a string→string object. Unknown keys are
//! refused, like every other header variable.
//!
//! Every `f64` goes through [`cad_domain::ymodel::json`], so a legacy non-finite
//! `$PDSIZE` survives bit-exactly as `{"$f64":"nan"}` instead of forcing the
//! whole header to be refused. Decoding is strict: an unknown key, a missing key
//! and a wrong JSON type are all errors naming the variable ([`SnapshotError`]),
//! because the writer re-serializes the whole header and would otherwise drop a
//! field it does not model (ADR-0037 D11).
//!
//! `active_annotation_scale.scale` repeats the Scale table row the name resolves
//! to (id, name, paper units, drawing units) so the header entry is readable on
//! its own; it must agree with `tables/scales.json`, and a row that does not is a
//! malformed document, not a silent overwrite.

use cad_domain::ymodel::id::{IdKind, PersistentId};
use cad_domain::ymodel::json::{
    decode_f64, decode_f64x2, decode_point3, encode_f64, encode_f64x2, encode_point3,
};
use cad_domain::ymodel::PersistentIdOf;
use cad_domain::{Point3, ScaleId};
use serde_json::{Map, Value};

use super::SnapshotError;
use crate::drawing::{DocumentMetadata, DrawingDatabase};
use crate::tables::{ActiveAnnotationScale, Scale};

/// The header variable object, exactly as stored in `document.json`.
pub(crate) fn encode(db: &DrawingDatabase) -> Result<Value, SnapshotError> {
    let mut map = Map::new();
    map.insert("linetype_scale".to_string(), encode_f64(db.linetype_scale));
    map.insert(
        "point_display_mode".to_string(),
        Value::from(db.point_display_mode),
    );
    map.insert(
        "point_display_size".to_string(),
        encode_f64(db.point_display_size),
    );
    map.insert(
        "insertion_base".to_string(),
        db.insertion_base.map_or(Value::Null, encode_point3),
    );
    map.insert(
        "limits_min".to_string(),
        db.limits_min.map_or(Value::Null, encode_f64x2),
    );
    map.insert(
        "limits_max".to_string(),
        db.limits_max.map_or(Value::Null, encode_f64x2),
    );
    map.insert(
        "code_page".to_string(),
        db.code_page.as_deref().map_or(Value::Null, Value::from),
    );
    map.insert(
        "active_annotation_scale".to_string(),
        db.active_annotation_scale
            .as_ref()
            .map_or(Value::Null, |active| encode_active(db, active)),
    );
    // DWGPROPS metadata: only a document that stores some writes the key, so
    // every other archive stays byte-identical to a pre-metadata header.
    if !db.document_metadata.is_empty() {
        map.insert("metadata".to_string(), encode_metadata(db));
    }
    Ok(Value::Object(map))
}

/// The `DWGPROPS` summary/metadata block.
fn encode_metadata(db: &DrawingDatabase) -> Value {
    let metadata = &db.document_metadata;
    let mut map = Map::new();
    for (key, value) in [
        ("title", &metadata.title),
        ("author", &metadata.author),
        ("subject", &metadata.subject),
        ("keywords", &metadata.keywords),
        ("comments", &metadata.comments),
    ] {
        if let Some(value) = value {
            map.insert(key.to_string(), Value::from(value.as_str()));
        }
    }
    if !metadata.custom.is_empty() {
        let custom = metadata
            .custom
            .iter()
            .map(|(name, value)| (name.clone(), Value::from(value.as_str())))
            .collect();
        map.insert("custom".to_string(), Value::Object(custom));
    }
    Value::Object(map)
}

/// One active annotation scale plus the Scale row its name resolves to.
fn encode_active(db: &DrawingDatabase, active: &ActiveAnnotationScale) -> Value {
    let mut map = Map::new();
    map.insert("name".to_string(), Value::from(active.name.as_str()));
    map.insert("value".to_string(), encode_f64(active.value));
    // `named` is stored state, not a derived value: it records whether the name
    // resolved when the importer set it, and a later table edit does not rewrite
    // it. Deriving it from the table on load would change the value.
    map.insert("named".to_string(), Value::Bool(active.named));
    map.insert(
        "scale".to_string(),
        db.scale_by_name(&active.name)
            .map_or(Value::Null, encode_scale),
    );
    Value::Object(map)
}

/// The Scale row the active name resolves to, as a header-owned copy.
fn encode_scale(scale: &Scale) -> Value {
    let mut map = Map::new();
    map.insert("id".to_string(), Value::from(scale.id.persistent().raw()));
    map.insert("name".to_string(), Value::from(scale.name.as_str()));
    map.insert("paper_units".to_string(), encode_f64(scale.paper_units));
    map.insert("drawing_units".to_string(), encode_f64(scale.drawing_units));
    Value::Object(map)
}

/// Write the header variables into a freshly hydrated database.
///
/// Called after `DrawingDatabaseBuilder::finish` (the database is not shared
/// yet), but the revision-tagged bounds memo is still cleared: a caller can
/// compute bounds, apply variables through a database it kept alive, and would
/// otherwise serve an extent tagged with the same revision.
pub(crate) fn apply_to(
    db: &mut DrawingDatabase,
    value: &serde_json::Value,
) -> Result<(), SnapshotError> {
    let mut reader = Reader::new(value, ROOT)?;
    let linetype_scale = reader.f64("linetype_scale")?;
    let point_display_mode = reader.i16("point_display_mode")?;
    let point_display_size = reader.f64("point_display_size")?;
    let insertion_base = reader.opt_point3("insertion_base")?;
    let limits_min = reader.opt_f64x2("limits_min")?;
    let limits_max = reader.opt_f64x2("limits_max")?;
    let code_page = reader.opt_string("code_page")?;
    let active = reader.take_active_annotation_scale("active_annotation_scale", db)?;
    let metadata = reader.take_document_metadata("metadata")?;
    reader.finish()?;

    db.linetype_scale = linetype_scale;
    db.point_display_mode = point_display_mode;
    db.point_display_size = point_display_size;
    db.insertion_base = insertion_base;
    db.limits_min = limits_min;
    db.limits_max = limits_max;
    db.code_page = code_page;
    db.active_annotation_scale = active;
    db.document_metadata = metadata;

    // `DrawingDatabase::invalidate_bounds_cache` is `pub(super)` to the
    // `drawing` module, so a sibling module cannot call it. Replacing the memo
    // with the default is the same effect (`revision` and `bounds` both cleared)
    // and does not need access to the private field.
    *db.bounds_cache
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Default::default();
    Ok(())
}

const ROOT: &str = "document.json variables";

/// A strict reader for one header-variable object.
struct Reader {
    map: Map<String, Value>,
    context: String,
}

impl Reader {
    fn new(value: &Value, context: &str) -> Result<Reader, SnapshotError> {
        match value {
            Value::Object(map) => Ok(Reader {
                map: map.clone(),
                context: context.to_string(),
            }),
            _ => Err(SnapshotError::invalid(format!(
                "{context}: expected a JSON object"
            ))),
        }
    }

    fn error(&self, detail: impl std::fmt::Display) -> SnapshotError {
        SnapshotError::invalid(format!("{}: {detail}", self.context))
    }

    fn take(&mut self, variable: &str) -> Result<Value, SnapshotError> {
        self.map
            .remove(variable)
            .ok_or_else(|| self.error(format!("missing required variable {variable:?}")))
    }

    fn finish(self) -> Result<(), SnapshotError> {
        // `Map` is a `BTreeMap`, so this is the first key in sorted order.
        match self.map.keys().next() {
            Some(variable) => Err(self.error(format!("unknown variable {variable:?}"))),
            None => Ok(()),
        }
    }

    fn f64(&mut self, variable: &str) -> Result<f64, SnapshotError> {
        let value = self.take(variable)?;
        decode_f64(&value).map_err(|error| self.error(format!("{variable}: {error}")))
    }

    fn opt_f64x2(&mut self, variable: &str) -> Result<Option<[f64; 2]>, SnapshotError> {
        let value = self.take(variable)?;
        if value.is_null() {
            return Ok(None);
        }
        decode_f64x2(&value)
            .map(Some)
            .map_err(|error| self.error(format!("{variable}: {error}")))
    }

    fn opt_point3(&mut self, variable: &str) -> Result<Option<Point3>, SnapshotError> {
        let value = self.take(variable)?;
        if value.is_null() {
            return Ok(None);
        }
        decode_point3(&value)
            .map(Some)
            .map_err(|error| self.error(format!("{variable}: {error}")))
    }

    fn opt_string(&mut self, variable: &str) -> Result<Option<String>, SnapshotError> {
        match self.take(variable)? {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            _ => Err(self.error(format!("{variable}: expected a string or null"))),
        }
    }

    fn i16(&mut self, variable: &str) -> Result<i16, SnapshotError> {
        let raw = self
            .take(variable)?
            .as_i64()
            .ok_or_else(|| self.error(format!("{variable}: expected a JSON integer")))?;
        i16::try_from(raw)
            .map_err(|_| self.error(format!("{variable}: {raw} is out of range for an i16")))
    }

    fn take_active_annotation_scale(
        &mut self,
        variable: &str,
        db: &DrawingDatabase,
    ) -> Result<Option<ActiveAnnotationScale>, SnapshotError> {
        let value = self.take(variable)?;
        if value.is_null() {
            return Ok(None);
        }
        let context = format!("{} {variable}", self.context);
        let mut reader = Reader::new(&value, &context)?;
        let name = reader.string("name")?;
        let value = reader.f64("value")?;
        let named = match reader.take("named")? {
            Value::Bool(named) => named,
            _ => return Err(reader.error("named: expected a boolean")),
        };
        // The Scale table is the authority and its row is already inserted; the
        // header copy only has to agree with it, so it is decoded for validation
        // and not stored a second time.
        let scale = reader.take("scale")?;
        if !scale.is_null() {
            decode_scale(&scale, &context, db)?;
        }
        reader.finish()?;
        Ok(Some(ActiveAnnotationScale { name, value, named }))
    }

    fn string(&mut self, field: &str) -> Result<String, SnapshotError> {
        match self.take(field)? {
            Value::String(text) => Ok(text),
            _ => Err(self.error(format!("{field}: expected a string"))),
        }
    }

    /// Read a string field only when the key is present; a present `null` is
    /// the same as absent (a tolerant read of a hand-written archive), while a
    /// present non-string is refused.
    fn opt_string_if_present(&mut self, field: &str) -> Result<Option<String>, SnapshotError> {
        match self.map.remove(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(self.error(format!("{field}: expected a string"))),
        }
    }

    /// Read the `DWGPROPS` metadata block. A missing key is "no metadata"; a
    /// present `null` is treated the same way. Inside the block every field is
    /// optional and `custom` must be a string→string object.
    fn take_document_metadata(
        &mut self,
        variable: &str,
    ) -> Result<DocumentMetadata, SnapshotError> {
        let Some(value) = self.map.remove(variable) else {
            return Ok(DocumentMetadata::default());
        };
        if value.is_null() {
            return Ok(DocumentMetadata::default());
        }
        let context = format!("{} {variable}", self.context);
        let mut reader = Reader::new(&value, &context)?;
        let metadata = DocumentMetadata {
            title: reader.opt_string_if_present("title")?,
            author: reader.opt_string_if_present("author")?,
            subject: reader.opt_string_if_present("subject")?,
            keywords: reader.opt_string_if_present("keywords")?,
            comments: reader.opt_string_if_present("comments")?,
            custom: reader.take_custom("custom")?,
        };
        reader.finish()?;
        Ok(metadata)
    }

    fn take_custom(
        &mut self,
        field: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, SnapshotError> {
        let Some(value) = self.map.remove(field) else {
            return Ok(std::collections::BTreeMap::new());
        };
        if value.is_null() {
            return Ok(std::collections::BTreeMap::new());
        }
        let Value::Object(map) = value else {
            return Err(self.error(format!("{field}: expected an object")));
        };
        let mut custom = std::collections::BTreeMap::new();
        for (name, value) in map {
            if name.is_empty() {
                return Err(self.error(format!("{field}: a property name is empty")));
            }
            let Value::String(text) = value else {
                return Err(self.error(format!("{field}.{name}: expected a string value")));
            };
            custom.insert(name, text);
        }
        Ok(custom)
    }
}

/// Decode the header's copy of the active Scale row and prove it is the row the
/// Scale table already holds.
///
/// The table is the authority (`DrawingDatabase::annotation_scale` resolves
/// through it); a header copy that disagrees, or that names a row the table does
/// not contain, is a malformed document. Nothing is inserted here: a row that
/// only the header knows about would also break the adopted id counters.
fn decode_scale(value: &Value, parent: &str, db: &DrawingDatabase) -> Result<Scale, SnapshotError> {
    let context = format!("{parent}.scale");
    let mut reader = Reader::new(value, &context)?;
    let raw = reader
        .take("id")?
        .as_i64()
        .ok_or_else(|| reader.error("id: expected a JSON integer identifier"))?;
    let id = PersistentId::from_raw(raw)
        .filter(|id| id.kind() == IdKind::Scale)
        .map(|id| ScaleId::from_seq(id.seq()))
        .ok_or_else(|| reader.error(format!("id: {raw} is not a Scale identifier")))?;
    let name = reader.string("name")?;
    let paper_units = reader.f64("paper_units")?;
    let drawing_units = reader.f64("drawing_units")?;
    reader.finish()?;
    let scale = Scale {
        id,
        name,
        paper_units,
        drawing_units,
    };
    match db.scales.get(&scale.id) {
        Some(stored) if stored == &scale => Ok(scale),
        Some(stored) => Err(SnapshotError::invalid(format!(
            "{context}: header copy of scale {} disagrees with tables/scales.json: \
             header {scale:?}, table {stored:?}",
            scale.id.0
        ))),
        None => Err(SnapshotError::invalid(format!(
            "{context}: scale {} is not in tables/scales.json; the header copy has no row to \
             resolve against",
            scale.id.0
        ))),
    }
}
