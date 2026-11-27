//! `tables/materials.json` row codec (document materials, minor 19).
//!
//! One row per material, in sequence order, with the entities and mesh faces
//! painted by that material inline:
//!
//! ```json
//! [{"seq":1,"name":"Red","color":[178,34,34],"entities":[2,3],
//!   "faces":[{"entity":2,"face":"skp:face:0"}]},
//!  {"seq":2,"name":"Glass","color":[170,220,240],"transparency":0.6,"entities":[4]}]
//! ```
//!
//! `transparency` is omitted when zero (never written as a `0.0` placeholder);
//! `entities` is always present, possibly empty; `faces` is omitted when empty
//! and lists `(entity, Mesh.face_sources key)` pairs otherwise. An entity appears
//! in at most one row and one face key at most once. The decoder refuses a
//! duplicate name/sequence/entity/face key, an out-of-range colour or
//! transparency, an unknown key and an assignment to a material the table does
//! not hold; the builder's `finish()` then refuses an assignment whose entity
//! does not exist in the drawing.
//!
//! Strict in both directions, like the group codec.

use std::collections::BTreeMap;

use cad_domain::material::{
    Material, MaterialDocument, MaterialError, MaterialSeq, MATERIAL_CAP, MATERIAL_FACE_CAP,
};
use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::Value;

use super::bim::object;
use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Materials)
}

fn invalid(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("material.invalid_value: {detail}"))
}

impl From<MaterialError> for SnapshotError {
    fn from(error: MaterialError) -> Self {
        SnapshotError::invalid(error.to_string())
    }
}

/// Encode every material row, in sequence order, with its painted entities.
pub(crate) fn encode_materials(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    let document = db.materials();
    let mut by_material: BTreeMap<MaterialSeq, Vec<EntityId>> = BTreeMap::new();
    for (entity, seq) in document.assignments() {
        by_material.entry(*seq).or_default().push(*entity);
    }
    let mut faces_by_material: BTreeMap<MaterialSeq, Vec<(EntityId, String)>> = BTreeMap::new();
    for ((entity, face), seq) in document.face_assignments() {
        faces_by_material
            .entry(*seq)
            .or_default()
            .push((*entity, face.clone()));
    }
    let mut rows = Vec::with_capacity(document.len());
    for material in document.materials() {
        let mut entries = vec![
            ("seq", Value::from(material.seq.0)),
            ("name", Value::from(material.name.as_str())),
            (
                "color",
                Value::Array(
                    material
                        .color
                        .iter()
                        .map(|channel| Value::from(*channel))
                        .collect(),
                ),
            ),
        ];
        if material.transparency != 0.0 {
            entries.push(("transparency", Value::from(material.transparency)));
        }
        let entities = by_material.remove(&material.seq).unwrap_or_default();
        entries.push((
            "entities",
            Value::Array(
                entities
                    .iter()
                    .map(|entity| Value::from(entity.persistent().raw()))
                    .collect(),
            ),
        ));
        let faces = faces_by_material.remove(&material.seq).unwrap_or_default();
        if !faces.is_empty() {
            entries.push((
                "faces",
                Value::Array(
                    faces
                        .into_iter()
                        .map(|(entity, face)| {
                            object(vec![
                                ("entity", Value::from(entity.persistent().raw())),
                                ("face", Value::from(face)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        rows.push(object(entries));
    }
    Ok(rows)
}

/// Decode the material table. An absent table is an empty document.
pub(crate) fn decode_materials(tables: &TableSnapshot) -> Result<MaterialDocument, SnapshotError> {
    let rows = tables.get(TableKind::Materials).unwrap_or(&[]);
    if rows.len() > MATERIAL_CAP {
        return Err(invalid(format!(
            "{}: the table holds {} materials, more than {MATERIAL_CAP}",
            table(),
            rows.len()
        )));
    }
    let mut materials = Vec::with_capacity(rows.len());
    let mut assignments: BTreeMap<EntityId, MaterialSeq> = BTreeMap::new();
    let mut face_assignments: BTreeMap<(EntityId, String), MaterialSeq> = BTreeMap::new();
    let mut names: Vec<String> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let (material, entities, faces) = decode_row(row, context.clone())?;
        let name = material.name.to_ascii_lowercase();
        if names.contains(&name) {
            return Err(SnapshotError::invalid(format!(
                "{}: duplicate material name {:?}",
                context, material.name
            )));
        }
        names.push(name);
        for entity in entities {
            if assignments.insert(entity, material.seq).is_some() {
                return Err(SnapshotError::invalid(format!(
                    "{context}: entity {} is painted more than once",
                    entity.0
                )));
            }
        }
        for (entity, face) in faces {
            if face_assignments
                .insert((entity, face.clone()), material.seq)
                .is_some()
            {
                return Err(SnapshotError::invalid(format!(
                    "{context}: face `{face}` of entity {} is painted more than once",
                    entity.0
                )));
            }
        }
        materials.push(material);
    }
    if face_assignments.len() > MATERIAL_FACE_CAP {
        return Err(invalid(format!(
            "{}: {} face paints exceed the {MATERIAL_FACE_CAP} budget",
            table(),
            face_assignments.len()
        )));
    }
    let document = MaterialDocument::from_parts(materials, assignments, face_assignments);
    document.validate()?;
    Ok(document)
}

type DecodedRow = (Material, Vec<EntityId>, Vec<(EntityId, String)>);

fn decode_row(value: &Value, context: String) -> Result<DecodedRow, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let raw = reader.integer("seq")?;
    let seq = u64::try_from(raw)
        .ok()
        .filter(|seq| *seq > 0)
        .map(MaterialSeq)
        .ok_or_else(|| reader.error(format!("seq: {raw} is not a positive material sequence")))?;
    let name = reader.string("name")?;
    if name.trim().is_empty() {
        return Err(reader.error("name: a material name cannot be empty"));
    }
    if name != name.trim() {
        return Err(reader.error(format!(
            "name: {name:?} is not trimmed (the writer stores trimmed names)"
        )));
    }
    let channels = reader.array("color")?;
    if channels.len() != 3 {
        return Err(reader.error("color: expected three sRGB channels"));
    }
    let mut color = [0_u8; 3];
    for (index, channel) in channels.iter().enumerate() {
        let value = channel
            .as_i64()
            .filter(|value| (0..=255).contains(value))
            .ok_or_else(|| {
                reader.error(format!(
                    "color: channel {index} must be an integer in 0..=255"
                ))
            })?;
        color[index] = value as u8;
    }
    let transparency = match reader.opt_take("transparency") {
        Some(Value::Number(number)) => number
            .as_f64()
            .ok_or_else(|| reader.error("transparency: expected a finite number"))?,
        Some(_) => return Err(reader.error("transparency: expected a number")),
        None => 0.0,
    };
    let entities = reader
        .array("entities")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| decode_entity(&value, format!("{context}.entities[{index}]")))
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    let faces = match reader.opt_take("faces") {
        Some(Value::Array(rows)) => rows
            .into_iter()
            .enumerate()
            .map(|(index, value)| decode_face(&value, format!("{context}.faces[{index}]")))
            .collect::<Result<Vec<_>, SnapshotError>>()?,
        Some(_) => return Err(reader.error("faces: expected an array")),
        None => Vec::new(),
    };
    reader.finish()?;
    let material = Material {
        seq,
        name,
        color,
        transparency,
    };
    material.validate()?;
    Ok((material, entities, faces))
}

fn decode_face(value: &Value, context: String) -> Result<(EntityId, String), SnapshotError> {
    let object = value
        .as_object()
        .ok_or_else(|| SnapshotError::invalid(format!("{context}: expected a JSON object")))?;
    if object.len() != 2 {
        return Err(SnapshotError::invalid(format!(
            "{context}: expected exactly `entity` and `face`"
        )));
    }
    let entity = object
        .get("entity")
        .ok_or_else(|| SnapshotError::invalid(format!("{context}.entity: missing")))
        .and_then(|value| decode_entity(value, format!("{context}.entity")))?;
    let face = object
        .get("face")
        .and_then(Value::as_str)
        .ok_or_else(|| SnapshotError::invalid(format!("{context}.face: expected a string")))?;
    if face.trim().is_empty() || face != face.trim() {
        return Err(SnapshotError::invalid(format!(
            "{context}.face: a face key must be trimmed and non-empty"
        )));
    }
    Ok((entity, face.to_string()))
}

fn decode_entity(value: &Value, context: String) -> Result<EntityId, SnapshotError> {
    let raw = value
        .as_i64()
        .ok_or_else(|| SnapshotError::invalid(format!("{context}: expected a JSON integer")))?;
    let persistent = PersistentId::from_raw(raw)
        .ok_or_else(|| SnapshotError::invalid(format!("{context}: {raw} is not an identifier")))?;
    if persistent.kind() != IdKind::Entity {
        return Err(SnapshotError::invalid(format!(
            "{context}: {raw} is not an entity identifier"
        )));
    }
    Ok(EntityId::from_seq(persistent.seq()))
}

/// The paint-table validation the builder's `finish()` runs.
///
/// The document itself must be valid **and** every painted entity must exist:
/// the codec cannot check the second rule (it decodes before the elements are
/// installed), so the one construction gate does. A delete prunes assignments in
/// its own path, so a dangling row means a malformed archive, not a deleted
/// object.
pub(crate) fn validate_materials(db: &DrawingDatabase) -> Result<(), SnapshotError> {
    let document = db.materials();
    document.validate()?;
    for entity in document.assignments().keys() {
        if !db.entities.contains_key(entity) {
            return Err(invalid(format!(
                "{}: entity {} is painted but does not exist",
                table(),
                entity.0
            )));
        }
    }
    for (entity, face) in document.face_assignments().keys() {
        if !db.entities.contains_key(entity) {
            return Err(invalid(format!(
                "{}: face `{face}` is painted on entity {}, which does not exist",
                table(),
                entity.0
            )));
        }
    }
    Ok(())
}
