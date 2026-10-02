//! Unit tests.

use super::*;

fn ann(id: u128, text: &str) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        space: SpaceId::Model,
        geometry: AnnotationGeometry::Text(Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        }),
        text: text.to_string(),
        style: AnnotationStyle::default(),
        created_unix_ms: 10,
        modified_unix_ms: 20,
        anchor: None,
        precision: Precision::Analytic,
    }
}

#[test]
fn create_then_delete_round_trips_through_the_database() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let changes = service
        .apply(&mut db, AnnotationCommand::Create(ann(1, "hello")))
        .unwrap();
    assert_eq!(changes.after, Revision(1));
    assert_eq!(db.len(), 1);
    service
        .apply(&mut db, AnnotationCommand::Delete(AnnotationId(1)))
        .unwrap();
    assert_eq!(db.len(), 0);
}

#[test]
fn encode_decode_round_trip_preserves_annotation() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([7u8; 32]);
    let mut extensions = BTreeMap::new();
    extensions.insert("future_field".to_string(), "{\"a\":1}".to_string());
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "test".into(),
        document_fingerprint: identity.clone(),
        document_name_hint: "sample.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: vec![ann(1, "hello")],
        view_bookmarks: Vec::new(),
        extensions_json: extensions,
    };
    let bytes = service.encode(&file).unwrap();
    let decoded = service
        .decode(&bytes, &identity, FingerprintPolicy::RejectMismatch)
        .unwrap();
    assert_eq!(decoded.annotations.len(), 1);
    assert_eq!(decoded.annotations[0].text, "hello");
    assert!(decoded.extensions_json.contains_key("future_field"));
}

#[test]
fn mismatched_fingerprint_is_refused_by_default() {
    let service = AnnotationService;
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "test".into(),
        document_fingerprint: DocumentIdentity::Sha256([1u8; 32]),
        document_name_hint: "a.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: Vec::new(),
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
    };
    let bytes = service.encode(&file).unwrap();
    let other = DocumentIdentity::Sha256([2u8; 32]);
    assert!(service
        .decode(&bytes, &other, FingerprintPolicy::RejectMismatch)
        .is_err());
    assert!(service
        .decode(&bytes, &other, FingerprintPolicy::ImportUnanchored)
        .is_ok());
}

#[test]
fn newer_schema_is_rejected() {
    let json = br#"{"schema_version": 99, "annotations": []}"#;
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::Unsupported(_))
    ));
}

fn round_trip(annotation: Annotation) -> Annotation {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([7u8; 32]);
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "test".into(),
        document_fingerprint: identity.clone(),
        document_name_hint: "sample.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: vec![annotation],
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
    };
    let bytes = service.encode(&file).unwrap();
    let decoded = service
        .decode(&bytes, &identity, FingerprintPolicy::RejectMismatch)
        .unwrap();
    decoded.annotations.into_iter().next().unwrap()
}

#[test]
fn full_fidelity_round_trip_preserves_all_fields() {
    // Every geometry variant, style, precision and a multi-INSERT anchor
    // must survive the round trip byte-for-byte on the struct (audit B09).
    let style = AnnotationStyle {
        rgba: [1, 2, 3, 4],
        logical_width: 7.5,
        text_height: 3.25,
    };
    let geoms = vec![
        AnnotationGeometry::Text(Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }),
        AnnotationGeometry::Leader(vec![
            Point3::default(),
            Point3 {
                x: 9.0,
                y: 8.0,
                z: 0.0,
            },
        ]),
        AnnotationGeometry::Rectangle([
            Point3::default(),
            Point3 {
                x: 5.0,
                y: 6.0,
                z: 0.0,
            },
        ]),
        AnnotationGeometry::Ellipse {
            center: Point3::default(),
            axis_u: Point3 {
                x: 4.0,
                y: 0.0,
                z: 0.0,
            },
            axis_v: Point3 {
                x: 0.0,
                y: 2.0,
                z: 0.0,
            },
        },
        AnnotationGeometry::Freehand(vec![Point3::default()]),
        AnnotationGeometry::Cloud(vec![Point3::default()]),
        AnnotationGeometry::Measurement(MeasurementRecord {
            algorithm: MeasurementAlgorithm::PlanarPolygonArea,
            inputs: vec![
                Point3::default(),
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
            plane: Some(WorkPlane {
                origin: Point3::default(),
                u: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                v: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            }),
            value: 12.5,
            units: UnitContext {
                source: Unit::Millimeter,
                display: Unit::Meter,
                display_per_source: Some(0.001),
                decimal_places: 4,
            },
            source: GeometrySource::ProxyCache,
            precision: Precision::Approximate {
                error_bound: Some(0.01),
            },
        }),
    ];
    for geometry in geoms {
        let mut a = ann(0x1234_5678_9abc_def0_1234_5678_9abc_def0, "x");
        a.geometry = geometry;
        a.style = style.clone();
        a.space = SpaceId::Paper(LayoutId(9));
        a.precision = Precision::Approximate {
            error_bound: Some(0.5),
        };
        a.created_unix_ms = 111;
        a.modified_unix_ms = 222;
        let decoded = round_trip(a.clone());
        assert_eq!(decoded, a);
    }
}

#[test]
fn multi_insert_anchor_keeps_instance_and_status() {
    let mut a = ann(1, "anchored");
    a.anchor = Some(EntityAnchor {
        source_handle: "2F".into(),
        instance: InstancePath(vec![EntityId(10), EntityId(20)]),
        sub_element: Some(SubElementId {
            source_key: "edge-3".into(),
            topology_revision: Revision(4),
        }),
        fallback: Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        status: AnchorStatus::Stale,
    });
    assert_eq!(round_trip(a.clone()), a);
}

/// A matching all-zero SHA-256 fingerprint, so a test can reach the
/// annotation-level decoders without tripping the fingerprint check.
const ZERO_FP: &str =
    r#""document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"#;

#[test]
fn unknown_measurement_algorithm_is_rejected_not_approximated() {
    // A future algorithm must not be silently downgraded to Distance2d.
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = format!(
        r#"{{"schema_version":1,{ZERO_FP}"annotations":[{{"id":"00000000-0000-0000-0000-000000000001",
            "space":"Model","geometry":{{"kind":"measurement","algorithm":"Future4d","value":1.0,"points":[[0,0,0]],
            "units":{{"source":"Millimeter","display":"Millimeter","decimal_places":3}},
            "source":"UserPoints","precision":{{"kind":"analytic"}}}},"text":"t","style":{{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0}},"created_unix_ms":0,"modified_unix_ms":0,"precision":{{"kind":"analytic"}}}}]}}"#
    );
    assert!(matches!(
        service.decode(
            json.as_bytes(),
            &identity,
            FingerprintPolicy::ImportUnanchored
        ),
        Err(CadError::CorruptData(_))
    ));
}

#[test]
fn malformed_annotation_id_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = format!(
        r#"{{"schema_version":1,{ZERO_FP}"annotations":[{{"id":"not-a-uuid",
            "space":"Model","geometry":{{"kind":"text","position":[0,0,0]}},"text":"t","style":{{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0}},"created_unix_ms":0,"modified_unix_ms":0,"precision":{{"kind":"analytic"}}}}]}}"#
    );
    assert!(matches!(
        service.decode(
            json.as_bytes(),
            &identity,
            FingerprintPolicy::ImportUnanchored
        ),
        Err(CadError::CorruptData(_))
    ));
}

#[test]
fn short_fingerprint_array_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[1,2,3],"annotations":[]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

#[test]
fn explicit_mapping_moves_geometry_and_unresolves_anchors() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let mut a = ann(1, "note");
    a.geometry = AnnotationGeometry::Text(Point3 {
        x: 1.0,
        y: 2.0,
        z: 0.0,
    });
    a.anchor = Some(EntityAnchor {
        source_handle: "A".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        status: AnchorStatus::Valid,
    });
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "t".into(),
        document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
        document_name_hint: "a.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: vec![a],
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
    };
    let bytes = service.encode(&file).unwrap();
    let mapping = Transform3::translation(Point3 {
        x: 10.0,
        y: -3.0,
        z: 0.0,
    });
    let decoded = service
        .decode(
            &bytes,
            &identity,
            FingerprintPolicy::ExplicitCoordinateMapping(mapping),
        )
        .unwrap();
    let out = &decoded.annotations[0];
    assert_eq!(
        out.geometry,
        AnnotationGeometry::Text(Point3 {
            x: 11.0,
            y: -1.0,
            z: 0.0
        })
    );
    let anchor = out.anchor.as_ref().unwrap();
    assert_eq!(anchor.fallback.x, 11.0);
    assert_eq!(anchor.fallback.y, -1.0);
    assert_eq!(anchor.status, AnchorStatus::Unresolved);
}

#[test]
fn unanchored_import_clears_anchors() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let mut a = ann(1, "note");
    a.anchor = Some(EntityAnchor {
        source_handle: "A".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: Point3::default(),
        status: AnchorStatus::Valid,
    });
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "t".into(),
        document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
        document_name_hint: "a.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: vec![a],
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
    };
    let bytes = service.encode(&file).unwrap();
    let decoded = service
        .decode(&bytes, &identity, FingerprintPolicy::ImportUnanchored)
        .unwrap();
    assert!(decoded.annotations[0].anchor.is_none());
}

#[test]
fn singular_mapping_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "t".into(),
        document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
        document_name_hint: "a.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: vec![ann(1, "note")],
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
    };
    let bytes = service.encode(&file).unwrap();
    let mut matrix = [[0.0f64; 4]; 4];
    matrix[3][3] = 1.0; // all-zero linear part => singular
    let singular = Transform3 { matrix };
    assert!(matches!(
        service.decode(
            &bytes,
            &identity,
            FingerprintPolicy::ExplicitCoordinateMapping(singular)
        ),
        Err(CadError::InvalidInput(_))
    ));
}
