//! The class registry: which element classes exist and which fields their
//! payload carries (ADR-0037 D4, D10).
//!
//! The format rules this module enforces:
//!
//! 1. **The descriptor decides the on-disk field set.** An element payload in
//!    `elements/<model>-<shard>.json` is a JSON object with a `class` key and
//!    exactly the fields named here. [`super::payload`] is the Rust view of the
//!    same field set, and its tests assert the two agree field by field, so a
//!    field added to one without the other fails.
//! 2. **A class id and a type key are file-format constants.** They are never
//!    renumbered and never reused: an ordinal is assigned once, in
//!    [`crate::SemanticGeometry`] declaration order, and a retired class leaves
//!    its ordinal unused. `type_key` is the human-facing name written into the
//!    payload (`"cad:Line"`, …).
//! 3. **Nested shapes are owned by their codecs, not by this table.** A field of
//!    kind [`FieldKind::Point3`] or [`FieldKind::DimensionDefinition`] is one
//!    JSON value whose internal shape `super::payload` defines; the registry
//!    names the value, it does not expand it. That is why the kind list contains
//!    no `I16`/`I32`/`U64`: no *top-level* element field is one.
//!
//! Two schema ids are in use:
//!
//! * [`GEOMETRY_SCHEMA`] — the semantic geometry classes of
//!   [`crate::SemanticGeometry`] (one class per variant, 20 of them).
//! * [`ELEMENT_SCHEMA`] — the element envelope the database owns
//!   ([`CadElement`]). `cad-db` encodes it; this registry is only the shared
//!   contract, so its field list is exactly what `cad-db`'s entity row plus its
//!   render-attribute carrier hold.
//!
//! `cad:Opaque` is a regular geometry class (it is a [`crate::SemanticGeometry`]
//! variant). It is how a source object this build does not model is preserved
//! verbatim (D10: undeclared classes are a Phase-4 concern, never a silent
//! drop).

use crate::ymodel::id::IdKind;

/// A schema generation. A class id embeds its schema so two schemas can use the
/// same ordinal without colliding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaId(pub u32);

/// A class inside one schema: `(schema << 32) | ordinal`.
///
/// The composite is a `u64` because ordinals are small and the schema is a
/// generation counter, not a namespace string. Accessors decode the parts;
/// nothing compares a raw id against a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClassId(u64);

impl ClassId {
    /// Build a class id from a schema and its ordinal.
    pub const fn new(schema: SchemaId, ordinal: u32) -> ClassId {
        ClassId(((schema.0 as u64) << 32) | ordinal as u64)
    }

    /// The schema this class belongs to.
    pub const fn schema(self) -> SchemaId {
        SchemaId((self.0 >> 32) as u32)
    }

    /// The ordinal inside the schema.
    pub const fn ordinal(self) -> u32 {
        self.0 as u32
    }

    /// The raw composite, for diagnostics and tests.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The declared type of one payload field.
///
/// Every kind listed here is used by at least one [`CLASSES`] field (a test
/// asserts it), so the enum cannot grow a speculative variant. The payload
/// codec in [`super::payload`] is the implementation of each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// A finite-or-escaped `f64` (see [`super::json`]).
    F64,
    /// An optional `f64` (`null` or the number codec).
    OptF64,
    /// A `u32` JSON number.
    U32,
    /// A JSON boolean.
    Bool,
    /// A JSON string.
    String,
    /// A string, or `null`.
    OptString,
    /// A `{"x","y","z"}` object.
    Point3,
    /// An array of [`Self::Point3`] objects.
    VecPoint3,
    /// An array of numbers.
    VecF64,
    /// An array of two numbers.
    F64x2,
    /// An array of two-number arrays (in-plane 2D points).
    VecF64x2,
    /// An array of `[u32; 3]` arrays (triangle indices).
    VecU32x3,
    /// An array of `[r, g, b]` byte triples.
    U8x3List,
    /// An array of per-face source records (`null` when the face has none).
    VecOptSubElement,
    /// A composite identifier of the named family (`PersistentId` number).
    IdRef(IdKind),
    /// `Option<MeshPbr>` (`null`, absent, or the PBR channel object; ADR-0039
    /// D1/D2). The object's shape is owned by [`super::payload`], like the other
    /// nested kinds.
    MeshPbr,
    /// One nested semantic geometry object (recursive).
    Geometry,
    /// An array of nested semantic geometry objects (recursive).
    VecGeometry,
    /// A byte string, written as lowercase hexadecimal (two chars per byte).
    ///
    /// Hexadecimal rather than a JSON array of numbers: one character per
    /// nibble keeps an opaque payload roughly half the size of `[0,12,255,…]`
    /// and stays diffable/`jq`-readable. Case is part of the format: the writer
    /// emits lowercase and the reader accepts only lowercase.
    Bytes,
    /// `Transform3` (`{"matrix": [[f64; 4]; 4]}`).
    Transform3,
    /// `TextAlignH` (`"left" | "center" | "right"`).
    TextAlignH,
    /// `TextAlignV` (`"baseline" | "bottom" | "middle" | "top"`).
    TextAlignV,
    /// `TextLineSpacing` (`{"factor", "style"}`).
    TextLineSpacing,
    /// `Body3d::kind` (`"wire" | "sheet" | "solid" | "unknown"`).
    BodyKind,
    /// `BodyExact` (`{"state":"exact","brep":…}` or
    /// `{"state":"unavailable","reason":…}`).
    BodyExact,
    /// `BodyDisplay` (mesh, edges, tolerance, precision, completeness).
    BodyDisplay,
    /// `DimensionDefinition` (subtype, points, text fields, normal).
    DimensionDefinition,
    /// `DimensionStyleValues` (the resolved DIMSTYLE snapshot).
    DimensionStyleValues,
    /// `DisplaySource` (`"synthesized" | "picture"`), the dimension display's
    /// provenance.
    DisplaySource,
    /// `Vec<HatchLoop>` (boundary loops in hatch-plane 2D).
    HatchLoops,
    /// `HatchPlane` (OCS normal + elevation).
    HatchPlane,
    /// `HatchStyle` (`"normal" | "outer" | "ignore"`).
    HatchStyle,
    /// `HatchFill` (solid / pattern / gradient).
    HatchFill,
    /// `HatchPatternType` (`"user" | "predefined" | "custom"`).
    HatchPatternType,
    /// `MLeaderStyleValues` (the resolved MLEADERSTYLE snapshot).
    MLeaderStyleValues,
    /// `MLeaderContent` (none / block / text / tolerance, with its payload).
    MLeaderContent,
    /// `MLeaderText` (the annotation text and its raw attachment codes).
    MLeaderText,
    /// `Vec<MLeaderRoot>`.
    MLeaderRoots,
    /// `Vec<MLeaderLine>`.
    MLeaderLines,
    /// `MLeaderReserved` (the verbatim source bag).
    MLeaderReserved,
    /// `PlaneFrame` (origin, u, v, normal-reversed).
    PlaneFrame,
    /// `TableFrame` (origin, horizontal, normal).
    TableFrame,
    /// `Vec<TableColumnGeometry>`.
    TableColumns,
    /// `Vec<TableRowGeometry>`.
    TableRows,
    /// `TableStyleValues` (the `TABLESTYLE` snapshot).
    TableStyleValues,
    /// The sparse per-cell margin overrides.
    TableMargins,
    /// The derived display cache (primitives + per-primitive appearance).
    TableDisplay,
    /// `Option<ImageClip>` (`null` or the polygon).
    OptImageClip,
    /// `ImageAdjust` (brightness/contrast/fade/monochrome).
    ImageAdjust,
    /// `SpaceId` (`{"kind":"model"}` / `{"kind":"paper","layout":…}` /
    /// `{"kind":"block","block":…}`).
    Space,
    /// The element's render attributes (colour/lineweight/linetype/opacity,
    /// annotative state, clip). Encoded by `cad-db`, not by this crate.
    RenderAttributes,
    /// The element's optional BIM block (ADR-0042 D2), or **absent** when the
    /// element carries none. Encoded by `cad-db` (`bim.rs`), not by this crate:
    /// the nested shape (source, level/type/material keys, psets, evidence) is
    /// that module's contract, like [`Self::RenderAttributes`].
    ///
    /// This is the second documented absent-means-`None` exception in an
    /// element payload (after the mesh `uvs`/`texture`/`pbr` fields): a
    /// minor-5 row is the same object minus this key. It is never written as
    /// `null`, because a strict minor-5 reader would refuse the extra key.
    Bim,
}

/// One field of a class payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// The JSON key, which is also the Rust field name, so the two never drift.
    pub name: &'static str,
    /// The field's declared JSON shape.
    pub kind: FieldKind,
}

/// One element class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassDescriptor {
    /// The stable `(schema, ordinal)` id.
    pub id: ClassId,
    /// The name written into the payload's `class` key.
    pub type_key: &'static str,
    /// The payload fields, in declaration order. The JSON object's keys are
    /// sorted by the map, so this order is documentation, not byte order.
    pub fields: &'static [FieldDescriptor],
}

/// The schema of the semantic geometry classes.
pub const GEOMETRY_SCHEMA: SchemaId = SchemaId(1);

/// The schema of the database-owned element envelope.
pub const ELEMENT_SCHEMA: SchemaId = SchemaId(2);

/// Shorthand for a field row.
const fn field(name: &'static str, kind: FieldKind) -> FieldDescriptor {
    FieldDescriptor { name, kind }
}

/// The class id of the n-th semantic geometry variant (1-based).
const fn geometry(ordinal: u32) -> ClassId {
    ClassId::new(GEOMETRY_SCHEMA, ordinal)
}

/// The database-owned element envelope (`cad:Element`).
///
/// The fields are exactly what `cad-db`'s entity row (`type_key`, `space`,
/// `layer`, `source_handle`) plus its render-attribute carrier and the baked
/// hidden flag hold; `geometry` is the semantic payload of the element's own
/// class. The element id is the row's key in the shard, not a field.
pub const CAD_ELEMENT: ClassDescriptor = ClassDescriptor {
    id: ClassId::new(ELEMENT_SCHEMA, 1),
    type_key: "cad:Element",
    fields: &[
        // The element's own persisted identity. It is inside the payload so an
        // entry read with `unzip -p ... | jq` names its rows without consulting
        // the shard filename; `persistent` ids keep it exact in f64 tooling.
        field("id", FieldKind::IdRef(IdKind::Entity)),
        field("type_key", FieldKind::String),
        field("space", FieldKind::Space),
        field("layer", FieldKind::IdRef(IdKind::Layer)),
        field("source_handle", FieldKind::OptString),
        field("invisible", FieldKind::Bool),
        field("render_attributes", FieldKind::RenderAttributes),
        field("geometry", FieldKind::Geometry),
        // ADR-0042 D2: appended last and **omitted** when the element has no
        // BIM data, so a minor-5 row is the same object minus this key. The
        // writer never emits `"bim": null`.
        field("bim", FieldKind::Bim),
    ],
};

/// Every element class this build understands, in format order.
///
/// Ordinals `1..=20` are the [`crate::SemanticGeometry`] variants **in
/// declaration order**; [`CAD_ELEMENT`] belongs to [`ELEMENT_SCHEMA`] and is
/// last because it is a different generation, not a 21st geometry.
pub const CLASSES: &[ClassDescriptor] = &[
    ClassDescriptor {
        id: geometry(1),
        type_key: "cad:Line",
        fields: &[
            field("start", FieldKind::Point3),
            field("end", FieldKind::Point3),
        ],
    },
    ClassDescriptor {
        id: geometry(2),
        type_key: "cad:Polyline",
        fields: &[
            field("points", FieldKind::VecPoint3),
            field("bulges", FieldKind::VecF64),
            field("closed", FieldKind::Bool),
            field("width", FieldKind::F64),
        ],
    },
    ClassDescriptor {
        id: geometry(3),
        type_key: "cad:Circle",
        fields: &[
            field("center", FieldKind::Point3),
            field("normal", FieldKind::Point3),
            field("radius", FieldKind::F64),
        ],
    },
    ClassDescriptor {
        id: geometry(4),
        type_key: "cad:Arc",
        fields: &[
            field("center", FieldKind::Point3),
            field("normal", FieldKind::Point3),
            field("radius", FieldKind::F64),
            field("start", FieldKind::F64),
            field("sweep", FieldKind::F64),
        ],
    },
    ClassDescriptor {
        id: geometry(5),
        type_key: "cad:Ellipse",
        fields: &[
            field("center", FieldKind::Point3),
            field("normal", FieldKind::Point3),
            field("major_axis", FieldKind::Point3),
            field("ratio", FieldKind::F64),
            field("start", FieldKind::F64),
            field("sweep", FieldKind::F64),
        ],
    },
    ClassDescriptor {
        id: geometry(6),
        type_key: "cad:Spline",
        fields: &[
            field("degree", FieldKind::U32),
            field("knots", FieldKind::VecF64),
            field("control_points", FieldKind::VecPoint3),
            field("weights", FieldKind::VecF64),
            field("closed", FieldKind::Bool),
        ],
    },
    ClassDescriptor {
        id: geometry(7),
        type_key: "cad:Point",
        fields: &[field("point", FieldKind::Point3)],
    },
    ClassDescriptor {
        id: geometry(8),
        type_key: "cad:Mesh",
        fields: &[
            field("vertices", FieldKind::VecPoint3),
            field("triangles", FieldKind::VecU32x3),
            field("normals", FieldKind::VecPoint3),
            field("face_sources", FieldKind::VecOptSubElement),
            field("colors", FieldKind::U8x3List),
            // ADR-0038 D4: appended last so a minor-2 payload is the same object
            // minus these two keys, and the reader may treat them as absent.
            field("uvs", FieldKind::VecF64x2),
            field("texture", FieldKind::OptString),
            // ADR-0039 D2: `null` or the PBR channel object. A minor-3 payload
            // is the same object minus this key, so the reader treats it as
            // absent (`pbr: None`).
            field("pbr", FieldKind::MeshPbr),
        ],
    },
    ClassDescriptor {
        id: geometry(9),
        type_key: "cad:Body3d",
        fields: &[
            field("kind", FieldKind::BodyKind),
            field("exact", FieldKind::BodyExact),
            field("display", FieldKind::BodyDisplay),
        ],
    },
    ClassDescriptor {
        id: geometry(10),
        type_key: "cad:Insert",
        fields: &[
            field("block", FieldKind::IdRef(IdKind::Block)),
            field("transform", FieldKind::Transform3),
        ],
    },
    ClassDescriptor {
        id: geometry(11),
        type_key: "cad:Text",
        fields: &[
            field("text", FieldKind::String),
            field("position", FieldKind::Point3),
            field("style", FieldKind::IdRef(IdKind::Style)),
            field("height", FieldKind::F64),
            field("rotation", FieldKind::F64),
            field("font", FieldKind::OptString),
            field("h_align", FieldKind::TextAlignH),
            field("v_align", FieldKind::TextAlignV),
            field("wrap_width", FieldKind::OptF64),
            field("width_factor", FieldKind::F64),
            field("line_spacing", FieldKind::TextLineSpacing),
        ],
    },
    ClassDescriptor {
        id: geometry(12),
        type_key: "cad:Shape",
        fields: &[
            field("shape_name", FieldKind::String),
            field("code", FieldKind::U32),
            field("position", FieldKind::Point3),
            field("size", FieldKind::F64),
            field("rotation", FieldKind::F64),
            field("font", FieldKind::OptString),
        ],
    },
    ClassDescriptor {
        id: geometry(13),
        type_key: "cad:Opaque",
        fields: &[
            field("type_key", FieldKind::String),
            field("version", FieldKind::U32),
            field("payload", FieldKind::Bytes),
        ],
    },
    ClassDescriptor {
        id: geometry(14),
        type_key: "cad:Compound",
        fields: &[field("children", FieldKind::VecGeometry)],
    },
    ClassDescriptor {
        id: geometry(15),
        type_key: "cad:Dimension",
        fields: &[
            field("definition", FieldKind::DimensionDefinition),
            field("style_name", FieldKind::String),
            field("style", FieldKind::DimensionStyleValues),
            field("measurement", FieldKind::F64),
            field("text_override", FieldKind::OptString),
            field("display", FieldKind::VecGeometry),
            field("text_h_align", FieldKind::TextAlignH),
            field("source", FieldKind::DisplaySource),
        ],
    },
    ClassDescriptor {
        id: geometry(16),
        type_key: "cad:Hatch",
        fields: &[
            field("loops", FieldKind::HatchLoops),
            field("plane", FieldKind::HatchPlane),
            field("style", FieldKind::HatchStyle),
            field("fill", FieldKind::HatchFill),
            field("seeds", FieldKind::VecF64x2),
            field("pixel_size", FieldKind::OptF64),
            field("pattern_type", FieldKind::HatchPatternType),
            field("pattern_origin", FieldKind::F64x2),
            field("display", FieldKind::VecGeometry),
        ],
    },
    ClassDescriptor {
        id: geometry(17),
        type_key: "cad:MultiLeader",
        fields: &[
            field("style", FieldKind::MLeaderStyleValues),
            field("style_name", FieldKind::String),
            field("override_mask", FieldKind::U32),
            field("content", FieldKind::MLeaderContent),
            field("text", FieldKind::MLeaderText),
            field("roots", FieldKind::MLeaderRoots),
            field("lines", FieldKind::MLeaderLines),
            field("reserved", FieldKind::MLeaderReserved),
            field("plane", FieldKind::PlaneFrame),
            field("display", FieldKind::VecGeometry),
        ],
    },
    ClassDescriptor {
        id: geometry(18),
        type_key: "cad:Table",
        fields: &[
            field("frame", FieldKind::TableFrame),
            field("columns", FieldKind::TableColumns),
            field("rows", FieldKind::TableRows),
            field("style", FieldKind::TableStyleValues),
            field("margins", FieldKind::TableMargins),
            field("name", FieldKind::String),
            field("description", FieldKind::String),
            field("display", FieldKind::TableDisplay),
        ],
    },
    ClassDescriptor {
        id: geometry(19),
        type_key: "cad:Image",
        fields: &[
            field("origin", FieldKind::Point3),
            field("u", FieldKind::Point3),
            field("v", FieldKind::Point3),
            field("pixels", FieldKind::F64x2),
            field("file", FieldKind::OptString),
            field("clip", FieldKind::OptImageClip),
            field("visible", FieldKind::Bool),
            field("adjust", FieldKind::ImageAdjust),
        ],
    },
    ClassDescriptor {
        id: geometry(20),
        type_key: "cad:Mask",
        fields: &[
            field("boundary", FieldKind::VecPoint3),
            field("inverted", FieldKind::Bool),
        ],
    },
    CAD_ELEMENT,
];

/// The class with this id, if this build knows it.
pub fn by_class_id(id: ClassId) -> Option<&'static ClassDescriptor> {
    CLASSES.iter().find(|class| class.id == id)
}

/// The class with this type key, if this build knows it.
pub fn by_type_key(type_key: &str) -> Option<&'static ClassDescriptor> {
    CLASSES.iter().find(|class| class.type_key == type_key)
}

/// Every class, in format order.
pub fn all() -> &'static [ClassDescriptor] {
    CLASSES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticGeometry;
    use std::collections::BTreeSet;

    /// The class each variant persists as. The exhaustive `match` is the
    /// compile-time guard: adding a `SemanticGeometry` variant without a
    /// descriptor stops this test module from building.
    fn type_key_of(geometry: &SemanticGeometry) -> &'static str {
        match geometry {
            SemanticGeometry::Line { .. } => "cad:Line",
            SemanticGeometry::Polyline { .. } => "cad:Polyline",
            SemanticGeometry::Circle { .. } => "cad:Circle",
            SemanticGeometry::Arc { .. } => "cad:Arc",
            SemanticGeometry::Ellipse { .. } => "cad:Ellipse",
            SemanticGeometry::Spline { .. } => "cad:Spline",
            SemanticGeometry::Point(_) => "cad:Point",
            SemanticGeometry::Mesh(_) => "cad:Mesh",
            SemanticGeometry::Body3d(_) => "cad:Body3d",
            SemanticGeometry::Insert { .. } => "cad:Insert",
            SemanticGeometry::Text { .. } => "cad:Text",
            SemanticGeometry::Shape { .. } => "cad:Shape",
            SemanticGeometry::Opaque { .. } => "cad:Opaque",
            SemanticGeometry::Compound(_) => "cad:Compound",
            SemanticGeometry::Dimension(_) => "cad:Dimension",
            SemanticGeometry::Hatch(_) => "cad:Hatch",
            SemanticGeometry::MultiLeader(_) => "cad:MultiLeader",
            SemanticGeometry::Table(_) => "cad:Table",
            SemanticGeometry::Image { .. } => "cad:Image",
            SemanticGeometry::Mask { .. } => "cad:Mask",
        }
    }

    fn sample_for(type_key: &'static str) -> SemanticGeometry {
        use crate::{Body3d, BodyDisplay, BodyExact, BodyKind, Mesh, Point3, Precision};
        let p = |x: f64| Point3 { x, y: 0.0, z: 0.0 };
        let mesh = Mesh::default();
        match type_key {
            "cad:Line" => SemanticGeometry::Line {
                start: p(0.0),
                end: p(1.0),
            },
            "cad:Polyline" => SemanticGeometry::Polyline {
                points: vec![p(0.0), p(1.0)],
                bulges: vec![0.0],
                closed: false,
                width: 0.0,
            },
            "cad:Circle" => SemanticGeometry::Circle {
                center: p(0.0),
                normal: p(0.0),
                radius: 1.0,
            },
            "cad:Arc" => SemanticGeometry::Arc {
                center: p(0.0),
                normal: p(0.0),
                radius: 1.0,
                start: 0.0,
                sweep: 1.0,
            },
            "cad:Ellipse" => SemanticGeometry::Ellipse {
                center: p(0.0),
                normal: p(0.0),
                major_axis: p(1.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
            "cad:Spline" => SemanticGeometry::Spline {
                degree: 3,
                knots: Vec::new(),
                control_points: vec![p(0.0)],
                weights: Vec::new(),
                closed: false,
            },
            "cad:Point" => SemanticGeometry::Point(p(0.0)),
            "cad:Mesh" => SemanticGeometry::Mesh(mesh),
            "cad:Body3d" => SemanticGeometry::Body3d(Box::new(Body3d {
                kind: BodyKind::Unknown,
                exact: BodyExact::Unavailable {
                    reason: "not modelled".into(),
                },
                display: BodyDisplay {
                    mesh,
                    edges: Vec::new(),
                    tolerance: None,
                    precision: Precision::Unknown,
                    completeness: crate::Completeness::Unverified,
                },
            })),
            "cad:Insert" => SemanticGeometry::Insert {
                block: crate::BlockId(1),
                transform: crate::Transform3::identity(),
            },
            "cad:Text" => SemanticGeometry::Text {
                text: String::new(),
                position: p(0.0),
                style: crate::StyleId(1),
                height: 1.0,
                rotation: 0.0,
                font: None,
                h_align: crate::TextAlignH::Left,
                v_align: crate::TextAlignV::Baseline,
                wrap_width: None,
                width_factor: 1.0,
                line_spacing: crate::TextLineSpacing::default(),
            },
            "cad:Shape" => SemanticGeometry::Shape {
                shape_name: "BOX".into(),
                code: 1,
                position: p(0.0),
                size: 1.0,
                rotation: 0.0,
                font: None,
            },
            "cad:Opaque" => SemanticGeometry::Opaque {
                type_key: "AcDbOther".into(),
                version: 1,
                payload: vec![0, 1, 255],
            },
            "cad:Compound" => SemanticGeometry::Compound(Vec::new()),
            "cad:Image" => SemanticGeometry::Image {
                origin: p(0.0),
                u: p(1.0),
                v: p(1.0),
                pixels: [1.0, 1.0],
                file: None,
                clip: None,
                visible: true,
                adjust: crate::ImageAdjust::default(),
            },
            "cad:Mask" => SemanticGeometry::Mask {
                boundary: vec![p(0.0), p(1.0), p(2.0)],
                inverted: false,
            },
            other => panic!("no sample for {other}"),
        }
    }

    /// One sample per non-recursive variant; the recursive families build their
    /// own samples in `payload::tests`.
    const SAMPLE_KEYS: [&str; 16] = [
        "cad:Line",
        "cad:Polyline",
        "cad:Circle",
        "cad:Arc",
        "cad:Ellipse",
        "cad:Spline",
        "cad:Point",
        "cad:Mesh",
        "cad:Body3d",
        "cad:Insert",
        "cad:Text",
        "cad:Shape",
        "cad:Opaque",
        "cad:Compound",
        "cad:Image",
        "cad:Mask",
    ];

    #[test]
    fn every_variant_has_a_descriptor_in_declaration_order() {
        let geometry_classes: Vec<&ClassDescriptor> = CLASSES
            .iter()
            .filter(|class| class.id.schema() == GEOMETRY_SCHEMA)
            .collect();
        assert_eq!(geometry_classes.len(), 20);
        for (index, class) in geometry_classes.iter().enumerate() {
            assert_eq!(
                class.id.ordinal(),
                index as u32 + 1,
                "{} is out of declaration order",
                class.type_key
            );
        }
        for key in SAMPLE_KEYS {
            let class = by_type_key(key).unwrap_or_else(|| panic!("no descriptor for {key}"));
            assert_eq!(class.id.schema(), GEOMETRY_SCHEMA);
            assert_eq!(type_key_of(&sample_for(key)), key);
        }
        // The four recursive families resolve too (their samples live in
        // `payload::tests`, which needs the restore constructors).
        for key in ["cad:Dimension", "cad:Hatch", "cad:MultiLeader", "cad:Table"] {
            assert!(by_type_key(key).is_some(), "{key}");
        }
    }

    #[test]
    fn class_ids_and_type_keys_are_unique() {
        let ids: BTreeSet<u64> = CLASSES.iter().map(|class| class.id.raw()).collect();
        assert_eq!(ids.len(), CLASSES.len(), "duplicate class id");
        let keys: BTreeSet<&str> = CLASSES.iter().map(|class| class.type_key).collect();
        assert_eq!(keys.len(), CLASSES.len(), "duplicate type key");
    }

    #[test]
    fn field_names_are_unique_per_class() {
        for class in CLASSES {
            let names: BTreeSet<&str> = class.fields.iter().map(|field| field.name).collect();
            assert_eq!(
                names.len(),
                class.fields.len(),
                "{} repeats a field name",
                class.type_key
            );
            assert!(!class.fields.is_empty(), "{} has no fields", class.type_key);
        }
    }

    #[test]
    fn lookups_are_consistent() {
        assert_eq!(all().len(), CLASSES.len());
        for class in CLASSES {
            assert_eq!(by_class_id(class.id), Some(class));
            assert_eq!(by_type_key(class.type_key), Some(class));
        }
        assert_eq!(by_type_key("cad:Nope"), None);
        assert_eq!(by_class_id(ClassId::new(GEOMETRY_SCHEMA, 21)), None);
        assert_eq!(CAD_ELEMENT.id.schema(), ELEMENT_SCHEMA);
        assert_eq!(CAD_ELEMENT.id.ordinal(), 1);
    }

    #[test]
    fn class_ids_encode_schema_and_ordinal() {
        let id = ClassId::new(SchemaId(3), 7);
        assert_eq!(id.schema(), SchemaId(3));
        assert_eq!(id.ordinal(), 7);
        assert_eq!(id.raw(), (3 << 32) | 7);
        // The same ordinal in two schemas is two distinct classes.
        assert_ne!(
            ClassId::new(GEOMETRY_SCHEMA, 1),
            ClassId::new(ELEMENT_SCHEMA, 1)
        );
    }

    #[test]
    fn every_declared_kind_is_used_by_a_field() {
        // Naming every variant here is the exhaustiveness guard: a new kind
        // without a used field fails this test, and a removed kind breaks the
        // compile.
        let all_kinds = |kind: FieldKind| -> FieldKind {
            match kind {
                FieldKind::F64
                | FieldKind::OptF64
                | FieldKind::U32
                | FieldKind::Bool
                | FieldKind::String
                | FieldKind::OptString
                | FieldKind::Point3
                | FieldKind::VecPoint3
                | FieldKind::VecF64
                | FieldKind::F64x2
                | FieldKind::VecF64x2
                | FieldKind::VecU32x3
                | FieldKind::U8x3List
                | FieldKind::VecOptSubElement
                | FieldKind::IdRef(_)
                | FieldKind::MeshPbr
                | FieldKind::Geometry
                | FieldKind::VecGeometry
                | FieldKind::Bytes
                | FieldKind::Transform3
                | FieldKind::TextAlignH
                | FieldKind::TextAlignV
                | FieldKind::TextLineSpacing
                | FieldKind::BodyKind
                | FieldKind::BodyExact
                | FieldKind::BodyDisplay
                | FieldKind::DimensionDefinition
                | FieldKind::DimensionStyleValues
                | FieldKind::DisplaySource
                | FieldKind::HatchLoops
                | FieldKind::HatchPlane
                | FieldKind::HatchStyle
                | FieldKind::HatchFill
                | FieldKind::HatchPatternType
                | FieldKind::MLeaderStyleValues
                | FieldKind::MLeaderContent
                | FieldKind::MLeaderText
                | FieldKind::MLeaderRoots
                | FieldKind::MLeaderLines
                | FieldKind::MLeaderReserved
                | FieldKind::PlaneFrame
                | FieldKind::TableFrame
                | FieldKind::TableColumns
                | FieldKind::TableRows
                | FieldKind::TableStyleValues
                | FieldKind::TableMargins
                | FieldKind::TableDisplay
                | FieldKind::OptImageClip
                | FieldKind::ImageAdjust
                | FieldKind::Space
                | FieldKind::RenderAttributes
                | FieldKind::Bim => kind,
            }
        };
        let probes: Vec<FieldKind> = vec![
            FieldKind::F64,
            FieldKind::OptF64,
            FieldKind::U32,
            FieldKind::Bool,
            FieldKind::String,
            FieldKind::OptString,
            FieldKind::Point3,
            FieldKind::VecPoint3,
            FieldKind::VecF64,
            FieldKind::F64x2,
            FieldKind::VecF64x2,
            FieldKind::VecU32x3,
            FieldKind::U8x3List,
            FieldKind::VecOptSubElement,
            FieldKind::IdRef(IdKind::Block),
            FieldKind::MeshPbr,
            FieldKind::Geometry,
            FieldKind::VecGeometry,
            FieldKind::Bytes,
            FieldKind::Transform3,
            FieldKind::TextAlignH,
            FieldKind::TextAlignV,
            FieldKind::TextLineSpacing,
            FieldKind::BodyKind,
            FieldKind::BodyExact,
            FieldKind::BodyDisplay,
            FieldKind::DimensionDefinition,
            FieldKind::DimensionStyleValues,
            FieldKind::DisplaySource,
            FieldKind::HatchLoops,
            FieldKind::HatchPlane,
            FieldKind::HatchStyle,
            FieldKind::HatchFill,
            FieldKind::HatchPatternType,
            FieldKind::MLeaderStyleValues,
            FieldKind::MLeaderContent,
            FieldKind::MLeaderText,
            FieldKind::MLeaderRoots,
            FieldKind::MLeaderLines,
            FieldKind::MLeaderReserved,
            FieldKind::PlaneFrame,
            FieldKind::TableFrame,
            FieldKind::TableColumns,
            FieldKind::TableRows,
            FieldKind::TableStyleValues,
            FieldKind::TableMargins,
            FieldKind::TableDisplay,
            FieldKind::OptImageClip,
            FieldKind::ImageAdjust,
            FieldKind::Space,
            FieldKind::RenderAttributes,
            FieldKind::Bim,
        ];
        for kind in probes {
            let kind = all_kinds(kind);
            let used = CLASSES
                .iter()
                .flat_map(|class| class.fields.iter())
                .any(|field| dict_has_kind(field.kind, kind));
            assert!(used, "FieldKind::{kind:?} is declared but never used");
        }
    }

    /// `IdRef(_)` compares equal only for the same family, so the coverage check
    /// treats it as one kind.
    fn dict_has_kind(field: FieldKind, probe: FieldKind) -> bool {
        match (field, probe) {
            (FieldKind::IdRef(_), FieldKind::IdRef(_)) => true,
            (field, probe) => field == probe,
        }
    }
}
