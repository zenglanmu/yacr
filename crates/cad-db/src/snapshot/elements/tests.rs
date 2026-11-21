use super::*;
use cad_domain::ymodel::descriptor::FieldKind;
use cad_domain::ymodel::SEQ_BITS;
use cad_domain::{ObjectId, Revision, SemanticGeometry};
use serde_json::json;
use std::collections::BTreeSet;

fn line() -> SemanticGeometry {
    let end = Point3 {
        x: 10.0,
        ..Point3::default()
    };
    SemanticGeometry::Line {
        start: Point3::default(),
        end,
    }
}

fn sample(id: u128, layer: u128, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: Some("2F".to_string()),
        },
        id: EntityId(id),
        layer: LayerId(layer),
        space,
        geometry,
        draw_order: 0,
    }
}

/// Encode → text → decode; the shard row owns the id, so the test sets it.
fn round_trip(
    entity: &DbEntity,
    attributes: Option<&EntityRenderAttributes>,
    invisible: bool,
) -> DecodedElement {
    let encoded = encode(entity, attributes, invisible, None).expect("encode");
    let text = serde_json::to_string(&encoded).expect("serialize");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let mut decoded = decode(&parsed).expect("decode");
    decoded.entity.id = entity.id;
    decoded.entity.object.id = entity.object.id;
    decoded
}

#[test]
fn model_paper_block_and_opaque_rows_round_trip() {
    let model = sample(1, 2, SpaceId::Model, line());
    let decoded = round_trip(&model, None, false);
    assert_eq!(decoded.entity, model);
    assert_eq!(decoded.attributes, None);
    assert!(!decoded.invisible);
    assert_eq!(
        decoded.entity.object.revision,
        Revision(0),
        "a revision is runtime state, not persisted"
    );
    assert_eq!(decoded.entity.draw_order, 0, "draw order is runtime state");

    let value = encode(&model, None, false, None).expect("encode");
    assert_eq!(value["space"], json!({ "kind": "model" }));
    assert_eq!(value["layer"], json!(LayerId(2).persistent().raw()));
    assert_eq!(value["source_handle"], json!("2F"));
    assert_eq!(value["invisible"], json!(false));
    assert_eq!(
        value["geometry"]["class"],
        json!("cad:Line"),
        "geometry is delegated to the payload codec"
    );

    let paper = sample(4, 3, SpaceId::Paper(LayoutId(1)), line());
    assert_eq!(
        round_trip(&paper, None, false).entity.space,
        SpaceId::Paper(LayoutId(1))
    );
    let value = encode(&paper, None, false, None).expect("encode");
    assert_eq!(
        value["space"],
        json!({ "kind": "paper", "id": LayoutId(1).persistent().raw() }),
        "the format spells the id field \"id\""
    );

    let block = sample(5, 3, SpaceId::Block(BlockId(2)), line());
    let decoded = round_trip(&block, None, false);
    assert_eq!(decoded.entity.space, SpaceId::Block(BlockId(2)));
    assert_eq!(
        decoded.entity.layer,
        LayerId(3),
        "a block member still belongs to a layer"
    );
    let value = encode(&block, None, false, None).expect("encode");
    assert_eq!(
        value["space"],
        json!({ "kind": "block", "id": BlockId(2).persistent().raw() })
    );
    assert_eq!(value["layer"], json!(LayerId(3).persistent().raw()));

    let opaque = SemanticGeometry::Opaque {
        type_key: "AcDbOther".to_string(),
        version: 1,
        payload: vec![0, 1, 255],
    };
    let entity = sample(6, 2, SpaceId::Model, opaque);
    assert_eq!(
        round_trip(&entity, None, false).entity.geometry,
        entity.geometry
    );
}

#[test]
fn render_attributes_round_trip_with_nan_scale_and_clip() {
    let attributes = EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(0.37),
        color: EntityColor::Explicit([1, 2, 3]),
        lineweight: EntityLineWeight::Explicit(0.35),
        linetype: EntityLineType::Explicit {
            name: "DASHED".to_string(),
            pattern: LinetypePattern {
                elements: vec![0.5, -0.25],
                cycle: 0.75,
            },
            scale: 1.25,
        },
        linetype_scale: f64::NAN,
        geometry_source: GeometrySource::ProxyCache,
        annotative: AnnotativeAttributes {
            annotative: true,
            overrides: vec![AnnotativeScaleOverride {
                scale: "1:100".to_string(),
                placement: AnnotativePlacement {
                    position: Point3 {
                        x: 1.5,
                        y: -2.0,
                        z: 0.0,
                    },
                    rotation: 0.5,
                    height: Some(2.5),
                },
            }],
        },
        clip: Some(BlockClip {
            vertices: vec![[0.0, 0.0], [10.0, 5.0]],
            inside: true,
            visible: false,
            front_clip: Some(-2.0),
            back_clip: None,
        }),
    };
    let entity = sample(7, 2, SpaceId::Model, line());

    let encoded = encode(&entity, Some(&attributes), false, None).expect("encode");
    assert_eq!(
        encoded["render_attributes"]["linetype_scale"],
        json!({ "$f64": "nan" }),
        "a non-finite scale uses the documented escape"
    );
    assert_eq!(
        encoded["render_attributes"]["color"],
        json!({ "kind": "explicit", "value": [1, 2, 3] })
    );
    assert_eq!(
        encoded["render_attributes"]["geometry_source"],
        json!({ "kind": "proxy_cache" })
    );

    let decoded = round_trip(&entity, Some(&attributes), false);
    let actual = decoded.attributes.expect("attributes");
    assert_eq!(
        actual.linetype_scale.to_bits(),
        f64::NAN.to_bits(),
        "NaN survives by bits"
    );
    // Everything else is exact; compare with the NaN field neutralised.
    let mut expected = attributes.clone();
    expected.linetype_scale = 0.0;
    let mut actual_rest = actual.clone();
    actual_rest.linetype_scale = 0.0;
    assert_eq!(actual_rest, expected);

    let keys: BTreeSet<&str> = encoded["render_attributes"]
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected_keys: BTreeSet<&str> = [
        "transparency",
        "color",
        "lineweight",
        "linetype",
        "linetype_scale",
        "geometry_source",
        "annotative",
        "clip",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected_keys, "every attribute field is written");
}

#[test]
fn absent_source_handle_and_invisible_round_trip() {
    let mut entity = sample(8, 2, SpaceId::Model, line());
    entity.object.source_handle = None;
    let decoded = round_trip(&entity, None, true);
    assert_eq!(decoded.entity.object.source_handle, None);
    assert!(decoded.invisible);
    assert_eq!(decoded.entity, entity);

    let value = encode(&entity, None, true, None).expect("encode");
    assert!(value["source_handle"].is_null());
    assert_eq!(value["invisible"], json!(true));
}

#[test]
fn malformed_rows_are_refused() {
    let entity = sample(1, 2, SpaceId::Model, line());
    let base = encode(&entity, None, false, None).expect("encode");
    let refused = |value: Value, needle: &str| {
        let error = decode(&value).expect_err("must be refused");
        assert!(error.detail.contains(needle), "{error}");
    };

    let mut extra = base.clone();
    extra
        .as_object_mut()
        .expect("object")
        .insert("colour".to_string(), json!(1));
    refused(extra, "colour");

    let mut missing = base.clone();
    missing.as_object_mut().expect("object").remove("geometry");
    refused(missing, "geometry");

    let mut wrong_family = base.clone();
    wrong_family["layer"] = json!(EntityId(1).persistent().raw());
    refused(wrong_family, "layer");

    let mut model_with_id = base.clone();
    model_with_id["space"] = json!({ "kind": "model", "id": LayoutId(1).persistent().raw() });
    refused(model_with_id, "space");

    let mut wrong_class = base.clone();
    wrong_class["class"] = json!("cad:NotAnElement");
    refused(wrong_class, "class");

    let mut wrong_type = base.clone();
    wrong_type["type_key"] = json!(7);
    refused(wrong_type, "type_key");

    let mut unknown_escape = encode(
        &entity,
        Some(&EntityRenderAttributes::default()),
        false,
        None,
    )
    .expect("encode");
    unknown_escape["render_attributes"]["linetype_scale"] = json!({ "$f64": "snan" });
    refused(unknown_escape, "snan");
}

#[test]
fn the_encoded_row_matches_the_descriptor() {
    let entity = sample(9, 2, SpaceId::Model, line());
    let value = encode(
        &entity,
        Some(&EntityRenderAttributes::default()),
        true,
        None,
    )
    .expect("encode");
    let object = value.as_object().expect("object");

    let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let mut expected: BTreeSet<&str> = CAD_ELEMENT.fields.iter().map(|field| field.name).collect();
    expected.insert("class");
    // ADR-0042 D2: `bim` is the one descriptor field an element row omits when it
    // has no BIM data (a minor-5 row is this row minus the key), so the "exactly
    // the descriptor" assertion is made over every other field.
    expected.remove("bim");
    assert_eq!(
        keys, expected,
        "the row is exactly the descriptor's field set"
    );
    assert_eq!(value["class"], json!(CLASS));
    let text = serde_json::to_string(&value).expect("serialize");
    assert!(
        text.starts_with("{\"class\":\"cad:Element\""),
        "class is the first key: {text}"
    );

    // Every descriptor kind is checked, so a new one cannot slip in untested.
    let check = |ok: bool, name: &str| assert!(ok, "field {name} has the wrong shape");
    for field in CAD_ELEMENT.fields {
        let encoded = &value[field.name];
        let name = field.name;
        match field.kind {
            FieldKind::String => check(encoded.is_string(), name),
            FieldKind::Space => check(encoded.get("kind").is_some_and(Value::is_string), name),
            FieldKind::IdRef(kind) => {
                let raw = encoded.as_i64().expect("a JSON integer id");
                let id = PersistentId::from_raw(raw).expect("a persistent id");
                assert_eq!(id.kind(), kind, "{name}");
                // Never a bare sequence: a raw value below 2^45 would mean the
                // encoder wrote `EntityId(1)` instead of `(kind << 45) | 1`.
                assert!(id.raw() >= 1 << SEQ_BITS, "{name} is not a composite id");
            }
            FieldKind::OptString => check(encoded.is_string() || encoded.is_null(), name),
            FieldKind::Bool => check(encoded.is_boolean(), name),
            FieldKind::RenderAttributes => check(encoded.is_object() || encoded.is_null(), name),
            FieldKind::Bim => check(encoded.is_object() || encoded.is_null(), name),
            FieldKind::Geometry => check(encoded.get("class").is_some_and(Value::is_string), name),
            other => panic!("{name} uses {other:?}, which no test here exercises"),
        }
    }
}
