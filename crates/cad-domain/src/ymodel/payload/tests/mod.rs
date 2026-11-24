//! Payload round-trip and field-completeness tests (ADR-0037 D8, D10).
//!
//! Two contracts are asserted here:
//!
//! 1. **The fixed point.** For every one of the 20 semantic geometry variants,
//!    `encode → JSON text → parse → decode → encode` produces an equal JSON
//!    value. That is the L1 round-trip at the payload level: a decoded payload
//!    re-encodes to the same bytes, including the derived display caches Phase 1
//!    stores verbatim.
//! 2. **The descriptor is the field set.** The encoded object's keys (minus
//!    `class`) equal the class's descriptor fields exactly, so a field added to
//!    the codec without the registry (or the reverse) fails.
//!
//! The samples live in [`samples`] (split out for the code-size ceiling); they
//! are built through the restore constructors wherever the validating
//! constructors would refuse synthetic data.

mod samples;

use std::collections::BTreeSet;

use serde_json::Value;

use super::{decode, encode, id_ref_value};
use crate::dimension::DisplaySource;
use crate::ymodel::descriptor::{by_type_key, ClassDescriptor};
use crate::{
    BlockId, Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, DimensionDefinition,
    DimensionGeometry, DimensionKind, DimensionPoints, HatchFill, HatchGeometry, HatchPattern,
    HatchPatternLine, HatchPatternType, HatchPlane, HatchStyle, ImageAdjust, MLeaderContent,
    MLeaderToleranceContent, Mesh, MultiLeaderGeometry, Point3, Precision, SemanticGeometry,
    TextAlignH, Transform3,
};
use samples::*;

/// The canonical texture key the mesh sample references (ADR-0038 D4).
pub(super) const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn origin() -> Point3 {
    p(0.0, 0.0, 0.0)
}

fn z_axis() -> Point3 {
    p(0.0, 0.0, 1.0)
}

/// `encode → text → parse → decode → encode` must be a fixed point.
fn assert_round_trips(geometry: &SemanticGeometry) {
    let first = encode(geometry);
    let type_key = first
        .get("class")
        .and_then(Value::as_str)
        .expect("class key")
        .to_string();
    assert_keys_match(&type_key, &first);
    let text = serde_json::to_string(&first).expect("serialize");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let decoded = decode(&parsed).unwrap_or_else(|error| panic!("{type_key}: {error}"));
    let second = encode(&decoded);
    assert_eq!(first, second, "{type_key} is not a payload fixed point");
    assert_eq!(
        decoded, *geometry,
        "{type_key} decoded to a different value"
    );
}

/// The encoded keys (minus `class`) are exactly the descriptor's field names.
fn assert_keys_match(type_key: &str, value: &Value) {
    let class: &ClassDescriptor =
        by_type_key(type_key).unwrap_or_else(|| panic!("no descriptor for {type_key}"));
    let mut expected: BTreeSet<&str> = class.fields.iter().map(|field| field.name).collect();
    let object = value.as_object().expect("payload object");
    let actual: BTreeSet<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| *key != "class")
        .collect();
    // ADR-0055: `cad:Insert.attributes` is omitted when empty (a minor-13 row),
    // so the descriptor's field is allowed to be absent on an ordinary insert.
    if type_key == "cad:Insert" && !object.contains_key("attributes") {
        expected.remove("attributes");
    }
    // Minor 15: `cad:Text.mirrored` is omitted when false (the default), so an
    // ordinary run is the eleven-field minor-14 object.
    if type_key == "cad:Text" && !object.contains_key("mirrored") {
        expected.remove("mirrored");
    }
    assert_eq!(
        actual, expected,
        "{type_key} payload fields disagree with its descriptor"
    );
    assert!(object.contains_key("class"), "{type_key} has no class key");
}

// ---------------------------------------------------------------------------
// The one-sample-per-variant contract.
// ---------------------------------------------------------------------------

/// Every `SemanticGeometry` variant, with one representative sample.
fn samples() -> Vec<SemanticGeometry> {
    vec![
        SemanticGeometry::Line {
            start: origin(),
            end: p(1.0, 2.0, 3.0),
        },
        SemanticGeometry::Polyline {
            points: vec![origin(), p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)],
            bulges: vec![0.0, 0.5, 0.0],
            closed: true,
            width: 0.25,
        },
        SemanticGeometry::Circle {
            center: origin(),
            normal: z_axis(),
            radius: 2.0,
        },
        SemanticGeometry::Arc {
            center: origin(),
            normal: z_axis(),
            radius: 2.0,
            start: 0.25,
            sweep: 1.75,
        },
        SemanticGeometry::Ellipse {
            center: origin(),
            normal: z_axis(),
            major_axis: p(3.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: 6.0,
        },
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                origin(),
                p(1.0, 1.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(3.0, 0.0, 0.0),
            ],
            weights: vec![1.0, 2.0, 2.0, 1.0],
            closed: false,
        },
        SemanticGeometry::Point(p(4.0, 5.0, 6.0)),
        SemanticGeometry::Mesh(mesh_sample()),
        SemanticGeometry::Body3d(Box::new(body_sample())),
        SemanticGeometry::Insert {
            block: BlockId(3),
            transform: Transform3 {
                matrix: [
                    [2.0, 0.0, 0.0, 10.0],
                    [0.0, 2.0, 0.0, 20.0],
                    [0.0, 0.0, 2.0, 30.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            },
            attributes: Vec::new(),
        },
        text_sample(),
        SemanticGeometry::Shape {
            shape_name: "BOX".to_string(),
            code: 7,
            position: p(1.0, 1.0, 0.0),
            size: 2.5,
            rotation: 0.5,
            font: Some("ltypeshp.shx".to_string()),
        },
        SemanticGeometry::Opaque {
            type_key: "AcDbUnknown".to_string(),
            version: 2,
            payload: vec![0x00, 0x01, 0x7f, 0x80, 0xff],
        },
        SemanticGeometry::Compound(vec![
            SemanticGeometry::Line {
                start: origin(),
                end: p(1.0, 1.0, 1.0),
            },
            SemanticGeometry::Mask {
                boundary: vec![origin(), p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)],
                inverted: true,
            },
        ]),
        SemanticGeometry::Dimension(Box::new(dimension_sample())),
        SemanticGeometry::Hatch(Box::new(hatch_sample())),
        SemanticGeometry::MultiLeader(Box::new(mleader_sample())),
        SemanticGeometry::Table(Box::new(table_sample())),
        image_sample(),
        SemanticGeometry::Mask {
            boundary: vec![
                origin(),
                p(2.0, 0.0, 0.0),
                p(2.0, 2.0, 0.0),
                p(0.0, 2.0, 0.0),
            ],
            inverted: false,
        },
        attribute_definition_sample(),
    ]
}

#[test]
fn all_twenty_one_variants_are_a_payload_fixed_point() {
    let samples = samples();
    assert_eq!(samples.len(), 21, "one sample per SemanticGeometry variant");
    let mut seen = BTreeSet::new();
    for geometry in &samples {
        assert_round_trips(geometry);
        let type_key = encode(geometry)
            .get("class")
            .and_then(Value::as_str)
            .expect("class")
            .to_string();
        assert!(seen.insert(type_key), "duplicate class in the sample list");
    }
    assert_eq!(seen.len(), 21);
    // Every geometry class in the registry is covered.
    let registered: BTreeSet<&str> = crate::ymodel::descriptor::CLASSES
        .iter()
        .filter(|class| class.id.schema() == crate::ymodel::descriptor::GEOMETRY_SCHEMA)
        .map(|class| class.type_key)
        .collect();
    let seen_refs: BTreeSet<&str> = seen.iter().map(String::as_str).collect();
    assert_eq!(seen_refs, registered);
}

// ---------------------------------------------------------------------------
// Nested alternatives the one-per-class samples do not reach.
// ---------------------------------------------------------------------------

#[test]
fn nested_alternatives_round_trip() {
    // A solid-hatch pattern fill with definition rows.
    let pattern = HatchGeometry::from_persisted(
        vec![hatch_loop_sample()],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Pattern(HatchPattern {
            name: "ANSI31".to_string(),
            angle: 0.5,
            scale: 1.5,
            double: true,
            lines: vec![HatchPatternLine {
                angle: 0.25,
                base_x: 1.0,
                base_y: 2.0,
                offset_x: 3.0,
                offset_y: 4.0,
                dashes: vec![1.0, -0.5, 0.25],
            }],
        }),
        Vec::new(),
        None,
        HatchPatternType::Predefined,
        [0.0, 0.0],
        Vec::new(),
    );
    assert_round_trips(&SemanticGeometry::Hatch(Box::new(pattern)));

    // A tolerance-content multileader with the `Text` content kind.
    let tolerance = mleader_with_content(MLeaderContent::Tolerance(MLeaderToleranceContent {
        text: "|0.05|A|B".to_string(),
        height: 2.5,
    }));
    assert_round_trips(&SemanticGeometry::MultiLeader(Box::new(tolerance)));

    // Radial dimension points and the synthesized display provenance.
    let radial = DimensionGeometry::from_persisted(
        DimensionDefinition {
            kind: DimensionKind::Radius,
            points: DimensionPoints::Radial {
                center: origin(),
                point_on_arc: p(2.0, 0.0, 0.0),
                leader_length: 3.0,
            },
            text_middle: origin(),
            text_rotation: 0.0,
            normal: z_axis(),
        },
        "Standard".to_string(),
        dimension_style_sample(),
        2.0,
        None,
        Vec::new(),
        TextAlignH::Left,
        DisplaySource::Synthesized,
    );
    assert_round_trips(&SemanticGeometry::Dimension(Box::new(radial)));

    // A body with missing exact geometry keeps its reason.
    let unavailable = Body3d::from_persisted(
        BodyKind::Unknown,
        BodyExact::Unavailable {
            reason: "unparsed SAT payload".to_string(),
        },
        BodyDisplay {
            mesh: Mesh::default(),
            edges: Vec::new(),
            tolerance: None,
            precision: Precision::Analytic,
            completeness: Completeness::Missing(vec!["exact geometry".to_string()]),
        },
    );
    assert_round_trips(&SemanticGeometry::Body3d(Box::new(unavailable)));

    // An image with no clip and no file.
    let bare_image = SemanticGeometry::Image {
        origin: origin(),
        u: p(1.0, 0.0, 0.0),
        v: p(0.0, 1.0, 0.0),
        pixels: [1.0, 2.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    };
    assert_round_trips(&bare_image);
}

/// ADR-0055: the value array is a fixed point; an absent key is an empty list
/// (a minor-13 insert) and an explicit `null` is refused, never guessed.
#[test]
fn insert_attributes_round_trip_and_absent_means_empty() {
    let with_values = insert_with_attributes_sample();
    assert_round_trips(&with_values);

    let encoded = encode(&with_values);
    let object = encoded.as_object().expect("insert object");
    assert!(object.contains_key("attributes"), "{object:?}");

    let mut bare = object.clone();
    bare.remove("attributes");
    let decoded = decode(&Value::Object(bare)).expect("a minor-13 insert decodes");
    let SemanticGeometry::Insert { attributes, .. } = decoded else {
        panic!("expected an insert");
    };
    assert!(attributes.is_empty(), "absent attributes mean no values");

    let mut nulled = object.clone();
    nulled.insert("attributes".to_string(), Value::Null);
    let error = decode(&Value::Object(nulled)).expect_err("null attributes are refused");
    assert!(error.message().contains("attributes"), "{error}");
}

fn mleader_with_content(content: MLeaderContent) -> MultiLeaderGeometry {
    let geometry = mleader_sample();
    // Rebuild with another content by going through the same restore path.
    MultiLeaderGeometry::from_persisted(
        geometry.style().clone(),
        geometry.style_name().to_string(),
        geometry.override_mask(),
        content,
        geometry.text().clone(),
        geometry.roots().to_vec(),
        geometry.lines().to_vec(),
        geometry.reserved().clone(),
        geometry.plane(),
        geometry.display().to_vec(),
    )
}

// ---------------------------------------------------------------------------
// Strictness.
// ---------------------------------------------------------------------------

#[test]
fn unknown_class_is_refused() {
    let error = decode(&serde_json::json!({ "class": "cad:Nope" })).expect_err("unknown class");
    assert!(error.message().contains("cad:Nope"), "{error}");
}

#[test]
fn missing_and_extra_fields_are_refused() {
    let line = encode(&SemanticGeometry::Line {
        start: origin(),
        end: p(1.0, 0.0, 0.0),
    });

    let mut missing = line.as_object().expect("object").clone();
    missing.remove("end");
    let error = decode(&Value::Object(missing)).expect_err("missing field");
    assert!(error.message().contains("end"), "{error}");

    let mut extra = line.as_object().expect("object").clone();
    extra.insert("future_field".to_string(), Value::Bool(true));
    let error = decode(&Value::Object(extra)).expect_err("extra field");
    assert!(error.message().contains("future_field"), "{error}");
}

#[test]
fn a_mesh_without_uvs_texture_or_pbr_decodes_to_the_bare_state() {
    // A minor-2 archive (ADR-0038 D4): the two fields this build added in minor 3
    // are simply absent; a minor-3 archive additionally has no `pbr` (ADR-0039
    // D2). That is the format's ONLY defaulting exception, so the next test also
    // proves it does not extend to a required field.
    let mut mesh = encode(&SemanticGeometry::Mesh(mesh_sample()));
    let object = mesh.as_object_mut().expect("mesh payload object");
    object.remove("uvs");
    object.remove("texture");
    object.remove("pbr");
    assert!(
        !object.contains_key("uvs")
            && !object.contains_key("texture")
            && !object.contains_key("pbr"),
        "{object:?}"
    );

    let decoded = decode(&Value::Object(object.clone())).expect("a minor-2 mesh decodes");
    let SemanticGeometry::Mesh(decoded) = decoded else {
        panic!("expected a mesh");
    };
    assert!(decoded.uvs.is_empty(), "absent uvs means no UV");
    assert_eq!(decoded.texture, None, "absent texture means none");
    assert_eq!(
        decoded.pbr, None,
        "absent pbr means the source declared none"
    );
    assert_eq!(decoded.vertices.len(), 3);
}

#[test]
fn a_hand_written_minor_three_mesh_payload_decodes_with_no_pbr() {
    // ADR-0039 D2 backward compatibility, spelled out instead of derived from
    // `encode`: this is the exact shape a minor-3 writer emits for a textured
    // mesh — every ordinal field plus `uvs`/`texture`, and no `pbr` key at all.
    let minor_three = serde_json::json!({
        "class": "cad:Mesh",
        "vertices": [
            {"x": 0.0, "y": 0.0, "z": 0.0},
            {"x": 1.0, "y": 0.0, "z": 0.0},
            {"x": 0.0, "y": 1.0, "z": 0.0}
        ],
        "triangles": [[0, 1, 2]],
        "normals": [
            {"x": 0.0, "y": 0.0, "z": 1.0},
            {"x": 0.0, "y": 0.0, "z": 1.0},
            {"x": 0.0, "y": 0.0, "z": 1.0}
        ],
        "face_sources": [null, null, null],
        "colors": [],
        "uvs": [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        "texture": TEXTURE_KEY,
    });
    let decoded = decode(&minor_three).expect("a minor-3 mesh decodes");
    let SemanticGeometry::Mesh(decoded) = decoded else {
        panic!("expected a mesh");
    };
    assert_eq!(decoded.pbr, None, "minor 3 never wrote pbr");
    assert_eq!(decoded.texture.as_deref(), Some(TEXTURE_KEY));
    assert_eq!(decoded.uvs.len(), 3);

    // Re-saving writes the new key explicitly, so the upgrade is visible.
    let re_encoded = encode(&SemanticGeometry::Mesh(decoded));
    assert_eq!(re_encoded["pbr"], Value::Null);
}

#[test]
fn only_the_three_texture_fields_may_be_absent() {
    // The negative half of ADR-0038 D4 / ADR-0039 D2: every other `cad:Mesh`
    // field is still required, so the exception cannot spread by accident.
    for required in ["vertices", "triangles", "normals", "face_sources", "colors"] {
        let mut mesh = encode(&SemanticGeometry::Mesh(mesh_sample()));
        let object = mesh.as_object_mut().expect("mesh payload object");
        object.remove(required).expect("the field exists");
        let error = decode(&Value::Object(object.clone())).expect_err(required);
        assert!(
            error.message().contains(required),
            "{required}: {error} (the refusal must name the field)"
        );
    }
}

#[test]
fn uv_texture_and_pbr_round_trip_through_the_payload() {
    let mesh = mesh_sample();
    assert_eq!(mesh.uvs.len(), mesh.vertices.len());
    let pbr = mesh.pbr.clone().expect("the sample declares PBR");
    let encoded = encode(&SemanticGeometry::Mesh(mesh.clone()));
    assert_eq!(encoded["texture"], Value::String(TEXTURE_KEY.to_string()));
    assert!(encoded["uvs"].is_array());
    let object = encoded["pbr"].as_object().expect("the pbr object");
    assert_eq!(object.len(), 8, "every PBR field is written: {object:?}");
    assert_eq!(object["metallic"], Value::from(0.75));
    assert_eq!(object["roughness"], Value::from(0.25));
    assert_eq!(object["emissive"], serde_json::json!([0.0, 0.125, 0.5]));
    assert_eq!(
        object["normal_texture"],
        Value::String(pbr.normal_texture.clone().expect("normal map"))
    );
    let text = serde_json::to_string(&encoded).expect("serialize");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let decoded = decode(&parsed).expect("decode");
    assert_eq!(decoded, SemanticGeometry::Mesh(mesh));
    let SemanticGeometry::Mesh(decoded) = decoded else {
        panic!("expected a mesh");
    };
    assert_eq!(decoded.pbr, Some(pbr));
}

#[test]
fn a_pbr_object_with_an_unknown_field_is_refused() {
    // The absent-key exception applies to `pbr` itself, never to the fields of
    // the nested object: a future minor that added a channel must be refused
    // rather than silently dropped on the next save.
    let mut mesh = encode(&SemanticGeometry::Mesh(mesh_sample()));
    let object = mesh.as_object_mut().expect("mesh payload object");
    let mut pbr = object["pbr"].as_object().expect("pbr object").clone();
    pbr.insert("clearcoat".to_string(), Value::from(1.0));
    object.insert("pbr".to_string(), Value::Object(pbr));
    let error = decode(&Value::Object(object.clone())).expect_err("unknown pbr field");
    assert!(error.message().contains("clearcoat"), "{error}");

    // A present object missing a required field is refused too (the exception is
    // about the *key* `pbr`, not about its contents).
    let object = mesh.as_object_mut().expect("mesh payload object");
    let mut pbr = object["pbr"].as_object().expect("pbr object").clone();
    pbr.remove("roughness").expect("the field exists");
    object.insert("pbr".to_string(), Value::Object(pbr));
    let error = decode(&Value::Object(object.clone())).expect_err("missing pbr field");
    assert!(error.message().contains("roughness"), "{error}");
}

#[test]
fn id_refs_must_belong_to_the_declared_family() {
    let insert = encode(&SemanticGeometry::Insert {
        block: BlockId(1),
        transform: Transform3::identity(),
        attributes: Vec::new(),
    });
    let mut object = insert.as_object().expect("object").clone();
    // A Style id where the descriptor declares a Block reference.
    object.insert(
        "block".to_string(),
        id_ref_value(crate::ymodel::IdKind::Style, 1),
    );
    let error = decode(&Value::Object(object)).expect_err("wrong family");
    assert!(error.message().contains("block"), "{error}");
}

#[test]
fn byte_payloads_are_lowercase_hex_only() {
    let opaque = encode(&SemanticGeometry::Opaque {
        type_key: "AcDbUnknown".to_string(),
        version: 1,
        payload: vec![0xab, 0xcd],
    });
    assert_eq!(
        opaque.get("payload").and_then(Value::as_str),
        Some("abcd"),
        "bytes are written as lowercase hex"
    );

    let mut object = opaque.as_object().expect("object").clone();
    object.insert("payload".to_string(), Value::String("ABCD".to_string()));
    let error = decode(&Value::Object(object)).expect_err("uppercase hex");
    assert!(error.message().contains("payload"), "{error}");
}

#[test]
fn non_finite_scalars_use_the_escape_inside_a_payload() {
    let line = encode(&SemanticGeometry::Line {
        start: Point3 {
            x: f64::INFINITY,
            y: f64::NAN,
            z: -0.0,
        },
        end: origin(),
    });
    assert_eq!(
        line["start"]["x"],
        serde_json::json!({ "$f64": "inf" }),
        "a non-finite coordinate is escaped, never dropped"
    );
    let text = serde_json::to_string(&line).expect("serialize");
    assert!(text.contains("\"$f64\":\"nan\""), "{text}");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let decoded = decode(&parsed).expect("decode");
    let SemanticGeometry::Line { start, .. } = decoded else {
        panic!("expected a line");
    };
    assert!(start.x.is_infinite() && start.y.is_nan());
    assert_eq!(start.z.to_bits(), (-0.0f64).to_bits());
}

#[test]
fn a_non_finite_stored_measurement_is_persisted_verbatim() {
    // Phase 1 must save a legacy row's `NaN` through the escape rather than
    // refuse the file or write `0.0` (ADR-0037 D9). A structural equality
    // assertion cannot express this (`NaN != NaN`), so the contract checked
    // here is the encoded form and its fixed point.
    let mut dimension = dimension_sample();
    dimension = DimensionGeometry::from_persisted(
        dimension.definition().clone(),
        dimension.style_name().to_string(),
        dimension.style().clone(),
        f64::NAN,
        dimension.text_override().map(str::to_string),
        dimension.display().to_vec(),
        dimension.text_h_align(),
        dimension.display_source(),
    );
    let geometry = SemanticGeometry::Dimension(Box::new(dimension));
    let encoded = encode(&geometry);
    assert_eq!(
        encoded["measurement"],
        serde_json::json!({ "$f64": "nan" }),
        "a non-finite measurement round-trips through the escape"
    );
    let text = serde_json::to_string(&encoded).expect("serialize");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let decoded = decode(&parsed).expect("decode");
    assert_eq!(encode(&decoded), encoded);
    let SemanticGeometry::Dimension(decoded) = decoded else {
        panic!("expected a dimension");
    };
    assert!(decoded.measurement().is_nan());
}

#[test]
fn table_display_appearance_must_match_geometry_len() {
    let value = encode(&SemanticGeometry::Table(Box::new(table_sample())));
    let mut object = value.as_object().expect("object").clone();
    let mut display = object
        .get("display")
        .and_then(Value::as_object)
        .expect("display")
        .clone();
    let appearance = display
        .get_mut("appearance")
        .and_then(Value::as_array_mut)
        .expect("appearance");
    appearance.pop();
    object.insert("display".to_string(), Value::Object(display));
    let error = decode(&Value::Object(object)).expect_err("length mismatch");
    assert!(error.message().contains("lockstep"), "{error}");
}

#[test]
fn text_mirror_flag_is_omitted_when_false_and_round_trips_when_true() {
    // Minor 15 contract: a readable run is byte-compatible with the minor-14
    // payload (no `mirrored` key); a mirrored run writes the key and decodes
    // back to the same value.
    let mut text = samples()
        .into_iter()
        .find(|geometry| matches!(geometry, SemanticGeometry::Text { .. }))
        .expect("the sample set has a Text");
    let off = encode(&text);
    assert!(
        off.get("mirrored").is_none(),
        "a readable run must not write the key: {off}"
    );
    let SemanticGeometry::Text { mirrored, .. } = &mut text else {
        unreachable!()
    };
    *mirrored = true;
    let on = encode(&text);
    assert_eq!(on.get("mirrored"), Some(&Value::Bool(true)));
    assert_keys_match("cad:Text", &on);
    let decoded = decode(&on).expect("mirrored text decodes");
    assert_eq!(decoded, text);
    let again = encode(&decoded);
    assert_eq!(on, again, "the mirrored payload is a fixed point");
}
