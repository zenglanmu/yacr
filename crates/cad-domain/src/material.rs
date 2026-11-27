//! Document materials (SketchUp Paint Bucket), `.ymodel` minor 19.
//!
//! A document owns a small table of named **flat** materials — a name, an sRGB
//! colour and a transparency — plus one assignment per painted entity. A
//! material is a look, not geometry: it never changes an entity's type, layer or
//! shape, and an unpainted entity keeps its own `ByLayer`/explicit colour.
//!
//! # Scope
//!
//! Paint targets a **whole entity** or, when the selection carries a
//! `SubElementId`, one **mesh face** (`Mesh.face_sources`). A face assignment is
//! keyed by `(entity, face source key)` and wins over the entity's assignment for
//! that face; the renderer splits the mesh per material. A face key that no
//! longer exists in the mesh is inert (mesh edits can drop faces); the report
//! names that state instead of pretending the paint is visible. Textures are not
//! modelled: the built-in catalogue and every document material are flat
//! colours, and the mesh texture/PBR channels that importers fill stay untouched
//! by this table.
//!
//! # Bounds and names
//!
//! [`MATERIAL_CAP`] = **256** rows. Adding a row at the cap is refused by name
//! rather than evicting one, so an undo never has to resurrect a silently
//! dropped material. Names are trimmed, non-empty, at most
//! [`MAX_MATERIAL_NAME_CHARS`] characters, without control characters, and
//! unique case-insensitively; resolving a name is exact-first then a unique
//! case-insensitive match, exactly like the named-view table.
//!
//! # Assignments
//!
//! The map is keyed by [`EntityId`]; deleting an entity drops its assignment in
//! the database's one prune point, and the archive codec refuses an assignment
//! whose entity or material does not exist. An assignment to a missing material
//! is a malformed document, never a silently ignored row.

use std::collections::BTreeMap;
use std::fmt;

use crate::EntityId;

/// How many material rows one document retains.
pub const MATERIAL_CAP: usize = 256;

/// Longest accepted material name, in characters.
pub const MAX_MATERIAL_NAME_CHARS: usize = 64;

/// Longest accepted mesh face-source key, in characters.
pub const MAX_FACE_KEY_CHARS: usize = 256;

/// How many per-face paints one document retains.
pub const MATERIAL_FACE_CAP: usize = 65_536;

/// Stable refusal markers.
pub mod markers {
    /// The name is empty, too long or contains a control character.
    pub const MATERIAL_NAME_INVALID: &str = "material.name_invalid";
    /// The transparency is not finite or outside `0..=1`.
    pub const MATERIAL_TRANSPARENCY_INVALID: &str = "material.transparency_invalid";
    /// The serialized table holds more rows than [`super::MATERIAL_CAP`].
    pub const MATERIAL_TABLE_TOO_LARGE: &str = "material.table_too_large";
    /// Adding a new name would exceed [`super::MATERIAL_CAP`].
    pub const MATERIAL_CAP_EXCEEDED: &str = "material.cap_exceeded";
    /// Two rows share a name (case-insensitively).
    pub const MATERIAL_DUPLICATE_NAME: &str = "material.duplicate_name";
    /// Two rows share a sequence.
    pub const MATERIAL_DUPLICATE_SEQ: &str = "material.duplicate_seq";
    /// The table has no row of that name / sequence.
    pub const MATERIAL_UNKNOWN: &str = "material.unknown";
    /// An assignment names a material the table does not hold.
    pub const MATERIAL_ASSIGNMENT_MISSING: &str = "material.assignment_missing";
    /// A face assignment names an empty/untrimmed/over-long/control key.
    pub const MATERIAL_FACE_KEY_INVALID: &str = "material.face_key_invalid";
    /// A new face paint would exceed [`super::MATERIAL_FACE_CAP`].
    pub const MATERIAL_FACE_CAP_EXCEEDED: &str = "material.face_cap_exceeded";
    /// A paint write would leave the document exactly as it was.
    pub const MATERIAL_NO_CHANGE: &str = "material.no_change";
}

/// Table-local identity of one material.
///
/// There is no `IdKind` and no header counter: like a parameter or a group, the
/// sequence is table-local (ADR-0043 D4/D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialSeq(pub u64);

/// One named flat material.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// Table-local identity; stable across renames.
    pub seq: MaterialSeq,
    /// Stored name, trimmed and non-empty.
    pub name: String,
    /// sRGB colour, one byte per channel.
    pub color: [u8; 3],
    /// `0.0` opaque … `1.0` fully transparent.
    pub transparency: f64,
}

impl Material {
    /// A new opaque material.
    pub fn new(seq: MaterialSeq, name: impl Into<String>, color: [u8; 3]) -> Self {
        Material {
            seq,
            name: name.into(),
            color,
            transparency: 0.0,
        }
    }

    /// Whether this row answers `name` (trimmed, case-insensitive).
    pub fn has_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name.trim())
    }

    /// The row's single validation point.
    pub fn validate(&self) -> Result<(), MaterialError> {
        if self.name.trim() != self.name {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_NAME_INVALID,
                "a material name is stored trimmed".into(),
            ));
        }
        if self.name.is_empty() {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_NAME_INVALID,
                "a material needs a non-empty name".into(),
            ));
        }
        if self.name.chars().count() > MAX_MATERIAL_NAME_CHARS {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_NAME_INVALID,
                format!(
                    "`{}` is longer than {MAX_MATERIAL_NAME_CHARS} characters",
                    self.name
                ),
            ));
        }
        if self.name.chars().any(char::is_control) {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_NAME_INVALID,
                "a material name cannot contain a control character".into(),
            ));
        }
        if !self.transparency.is_finite() || !(0.0..=1.0).contains(&self.transparency) {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_TRANSPARENCY_INVALID,
                format!(
                    "material `{}` transparency must be finite and in 0..=1, got {}",
                    self.name, self.transparency
                ),
            ));
        }
        Ok(())
    }
}

/// One material-table failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialError {
    /// A stable marker plus a detail, ready for a `CadError`/snapshot error.
    Invalid(&'static str, String),
}

impl MaterialError {
    /// The stable marker.
    pub fn marker(&self) -> &'static str {
        match self {
            MaterialError::Invalid(marker, _) => marker,
        }
    }
}

impl fmt::Display for MaterialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaterialError::Invalid(marker, detail) => write!(formatter, "{marker}: {detail}"),
        }
    }
}

impl std::error::Error for MaterialError {}

/// The document's material table and its one assignment per painted entity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialDocument {
    materials: Vec<Material>,
    assignments: BTreeMap<EntityId, MaterialSeq>,
    /// Per-face paints, keyed by `(entity, Mesh.face_sources source key)`.
    face_assignments: BTreeMap<(EntityId, String), MaterialSeq>,
}

impl MaterialDocument {
    /// An empty document (no material, no assignment).
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a document from decoded rows and assignments.
    ///
    /// Validation is the caller's: this is the snapshot/undo constructor, and a
    /// decoder validates before it hands rows over.
    pub fn from_parts(
        materials: Vec<Material>,
        assignments: BTreeMap<EntityId, MaterialSeq>,
        face_assignments: BTreeMap<(EntityId, String), MaterialSeq>,
    ) -> Self {
        MaterialDocument {
            materials,
            assignments,
            face_assignments,
        }
    }

    /// The material rows in sequence order.
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    /// How many materials the document holds.
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Whether the document holds no material.
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    /// Resolve a name: exact first, then a unique case-insensitive match.
    pub fn resolve(&self, name: &str) -> Option<&Material> {
        let name = name.trim();
        if let Some(found) = self.materials.iter().find(|row| row.name == name) {
            return Some(found);
        }
        let mut matches = self
            .materials
            .iter()
            .filter(|row| row.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// The row with `seq`, when it exists.
    pub fn get(&self, seq: MaterialSeq) -> Option<&Material> {
        self.materials.iter().find(|row| row.seq == seq)
    }

    /// The material painted on `entity`, when there is one.
    pub fn assignment(&self, entity: EntityId) -> Option<&Material> {
        let seq = *self.assignments.get(&entity)?;
        self.get(seq)
    }

    /// The assigned sequence for `entity`.
    pub fn assigned(&self, entity: EntityId) -> Option<MaterialSeq> {
        self.assignments.get(&entity).copied()
    }

    /// Every assignment, keyed by entity (sorted).
    pub fn assignments(&self) -> &BTreeMap<EntityId, MaterialSeq> {
        &self.assignments
    }

    /// The material painted on one face of `entity`, when there is one.
    pub fn face_assignment(&self, entity: EntityId, face: &str) -> Option<&Material> {
        let seq = *self.face_assignments.get(&(entity, face.to_string()))?;
        self.get(seq)
    }

    /// Every face assignment, keyed by `(entity, face source key)`.
    pub fn face_assignments(&self) -> &BTreeMap<(EntityId, String), MaterialSeq> {
        &self.face_assignments
    }

    /// Find `name` or add it (opaque/transparency as given) with the next free
    /// sequence. Returns the row's sequence.
    ///
    /// A new row at [`MATERIAL_CAP`] is [`markers::MATERIAL_CAP_EXCEEDED`]; an
    /// existing name is returned as-is (a repaint with a different colour does not
    /// silently edit the shared material).
    pub fn ensure(
        &mut self,
        name: &str,
        color: [u8; 3],
        transparency: f64,
    ) -> Result<MaterialSeq, MaterialError> {
        let name = name.trim();
        if let Some(existing) = self.resolve(name) {
            return Ok(existing.seq);
        }
        if self.materials.len() >= MATERIAL_CAP {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_CAP_EXCEEDED,
                format!(
                    "`{name}` would be material {}, but the table holds at most {MATERIAL_CAP} rows",
                    self.materials.len() + 1
                ),
            ));
        }
        let next = self
            .materials
            .iter()
            .map(|row| row.seq.0)
            .max()
            .unwrap_or(0)
            + 1;
        let material = Material {
            seq: MaterialSeq(next),
            name: name.to_string(),
            color,
            transparency,
        };
        material.validate()?;
        self.materials.push(material);
        self.materials.sort_by_key(|row| row.seq);
        Ok(MaterialSeq(next))
    }

    /// Paint `entity` with `seq` (the material must exist).
    pub fn assign(&mut self, entity: EntityId, seq: MaterialSeq) -> Result<(), MaterialError> {
        if self.get(seq).is_none() {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_UNKNOWN,
                format!("no material has sequence {}", seq.0),
            ));
        }
        self.assignments.insert(entity, seq);
        Ok(())
    }

    /// Paint one mesh face with `seq` (the material must exist).
    ///
    /// The key is the persisted `Mesh.face_sources` source key (the same key a
    /// pick stores in `SelectionRef.sub_element`). A trimmed, non-empty,
    /// at-most-[`MAX_FACE_KEY_CHARS`] key without control characters is
    /// required; the command layer verifies the key exists in the mesh.
    pub fn assign_face(
        &mut self,
        entity: EntityId,
        face: &str,
        seq: MaterialSeq,
    ) -> Result<(), MaterialError> {
        if self.get(seq).is_none() {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_UNKNOWN,
                format!("no material has sequence {}", seq.0),
            ));
        }
        validate_face_key(face)?;
        let key = (entity, face.to_string());
        if !self.face_assignments.contains_key(&key)
            && self.face_assignments.len() >= MATERIAL_FACE_CAP
        {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_FACE_CAP_EXCEEDED,
                format!("the document holds at most {MATERIAL_FACE_CAP} face paints"),
            ));
        }
        self.face_assignments.insert(key, seq);
        Ok(())
    }

    /// Remove `entity`'s assignment and every face assignment of that entity
    /// (an unpainted entity keeps its own colour).
    pub fn clear(&mut self, entity: EntityId) {
        self.assignments.remove(&entity);
        self.face_assignments
            .retain(|(owner, _), _| *owner != entity);
    }

    /// Drop every assignment whose entity no longer exists in `alive`.
    pub fn retain_entities(&mut self, alive: impl Fn(EntityId) -> bool) {
        self.assignments.retain(|entity, _| alive(*entity));
        self.face_assignments
            .retain(|(entity, _), _| alive(*entity));
    }

    /// The whole document's validation.
    pub fn validate(&self) -> Result<(), MaterialError> {
        if self.materials.len() > MATERIAL_CAP {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_TABLE_TOO_LARGE,
                format!(
                    "the table holds {} materials, more than {MATERIAL_CAP}",
                    self.materials.len()
                ),
            ));
        }
        let mut names: Vec<String> = Vec::with_capacity(self.materials.len());
        let mut sequences: Vec<u64> = Vec::with_capacity(self.materials.len());
        for material in &self.materials {
            material.validate()?;
            if sequences.contains(&material.seq.0) {
                return Err(MaterialError::Invalid(
                    markers::MATERIAL_DUPLICATE_SEQ,
                    format!("sequence {} appears twice", material.seq.0),
                ));
            }
            sequences.push(material.seq.0);
            let name = material.name.to_ascii_lowercase();
            if names.contains(&name) {
                return Err(MaterialError::Invalid(
                    markers::MATERIAL_DUPLICATE_NAME,
                    format!("material `{}` appears twice", material.name),
                ));
            }
            names.push(name);
        }
        for (entity, seq) in &self.assignments {
            if !sequences.contains(&seq.0) {
                return Err(MaterialError::Invalid(
                    markers::MATERIAL_ASSIGNMENT_MISSING,
                    format!(
                        "entity {} is painted with material {}, which the table does not hold",
                        entity.0, seq.0
                    ),
                ));
            }
        }
        if self.face_assignments.len() > MATERIAL_FACE_CAP {
            return Err(MaterialError::Invalid(
                markers::MATERIAL_FACE_CAP_EXCEEDED,
                format!(
                    "the table holds {} face paints, more than {MATERIAL_FACE_CAP}",
                    self.face_assignments.len()
                ),
            ));
        }
        for ((entity, face), seq) in &self.face_assignments {
            validate_face_key(face)?;
            if !sequences.contains(&seq.0) {
                return Err(MaterialError::Invalid(
                    markers::MATERIAL_ASSIGNMENT_MISSING,
                    format!(
                        "face `{face}` of entity {} is painted with material {}, which the table \
                         does not hold",
                        entity.0, seq.0
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// One validated face-source key.
fn validate_face_key(face: &str) -> Result<(), MaterialError> {
    if face.trim().is_empty() || face != face.trim() {
        return Err(MaterialError::Invalid(
            markers::MATERIAL_FACE_KEY_INVALID,
            "a face key must be trimmed and non-empty".into(),
        ));
    }
    if face.chars().count() > MAX_FACE_KEY_CHARS {
        return Err(MaterialError::Invalid(
            markers::MATERIAL_FACE_KEY_INVALID,
            format!("`{face}` is longer than {MAX_FACE_KEY_CHARS} characters"),
        ));
    }
    if face.chars().any(char::is_control) {
        return Err(MaterialError::Invalid(
            markers::MATERIAL_FACE_KEY_INVALID,
            "a face key cannot contain a control character".into(),
        ));
    }
    Ok(())
}

/// A short, locale-neutral point the command layer can echo in a diagnostic.
pub fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// One entry of the built-in palette: a name, sRGB colour and transparency.
///
/// This is our own small flat palette (the classic colour-wheel names plus a
/// translucent `Glass`), not a copy of any vendor's material library: no icon,
/// texture or vendor name. The document table is the authority; painting a
/// built-in copies it into the document on first use.
pub const BUILTIN_MATERIALS: &[(&str, [u8; 3], f64)] = &[
    ("Red", [178, 34, 34], 0.0),
    ("Orange", [230, 126, 34], 0.0),
    ("Yellow", [241, 196, 15], 0.0),
    ("Green", [39, 174, 96], 0.0),
    ("Cyan", [26, 188, 156], 0.0),
    ("Blue", [41, 128, 185], 0.0),
    ("Purple", [142, 68, 173], 0.0),
    ("Gray", [127, 140, 141], 0.0),
    ("White", [236, 240, 241], 0.0),
    ("Black", [44, 62, 80], 0.0),
    ("Wood", [160, 110, 60], 0.0),
    ("Glass", [170, 220, 240], 0.6),
];

/// The built-in entry named `name` (case-insensitive), when there is one.
pub fn builtin_material(name: &str) -> Option<(&'static str, [u8; 3], f64)> {
    let name = name.trim();
    BUILTIN_MATERIALS
        .iter()
        .find(|(candidate, _, _)| candidate.eq_ignore_ascii_case(name))
        .copied()
}

/// The renderer-facing look of a material: sRGB bytes and opacity.
///
/// Kept as a tiny value type so `cad-representation` does not depend on the
/// material table itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialLook {
    /// sRGB colour, one byte per channel.
    pub color: [u8; 3],
    /// `1.0` opaque … `0.0` fully transparent (the renderer's alpha).
    pub alpha: f64,
}

impl Material {
    /// The look this material paints.
    pub fn look(&self) -> MaterialLook {
        MaterialLook {
            color: self.color,
            alpha: (1.0 - self.transparency).clamp(0.0, 1.0),
        }
    }
}
