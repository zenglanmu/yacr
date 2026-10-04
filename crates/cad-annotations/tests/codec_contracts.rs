//! F09 sidecar codec contracts: lossless versioned round-trip, deterministic
//! fingerprint/mapping policy, and atomic export (audit B09/B10/B11/B07).
//!
//! These are integration tests over the public surface only, so they pin the
//! behaviour a host depends on rather than internal helper shapes.

use std::collections::BTreeMap;

use cad_annotations::{
    AnnotationCommand, AnnotationFile, AnnotationService, FingerprintPolicy, ViewBookmark,
    SCHEMA_VERSION,
};
use cad_db::{
    AnchorStatus, Annotation, AnnotationDatabase, AnnotationGeometry, AnnotationStyle,
    EntityAnchor, MeasurementAlgorithm, MeasurementRecord,
};
use cad_domain::*;

fn text_note(id: u128, text: &str) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        space: SpaceId::Model,
        geometry: AnnotationGeometry::Text(Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }),
        text: text.to_string(),
        style: AnnotationStyle {
            rgba: [1, 2, 3, 4],
            logical_width: 2.5,
            text_height: 1.75,
        },
        created_unix_ms: 10,
        modified_unix_ms: 20,
        anchor: None,
        precision: Precision::Analytic,
    }
}

fn empty_file(identity: DocumentIdentity) -> AnnotationFile {
    AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "test".into(),
        document_fingerprint: identity,
        document_name_hint: "sample.dwg".into(),
        unit_context: UnitContext::drawing_units(),
        annotations: Vec::new(),
        view_bookmarks: Vec::new(),
        extensions_json: BTreeMap::new(),
        nested_extensions: Default::default(),
    }
}

/// parse -> serialize -> parse must be a structural identity for every field the
/// codec claims to support (audit B09).
#[test]
fn parse_serialise_parse_is_lossless_for_every_field() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0x5au8; 32]);
    let mut unit_context = UnitContext {
        source: Unit::Millimeter,
        display: Unit::Meter,
        display_per_source: Some(0.001),
        decimal_places: 6,
    };
    unit_context.decimal_places = 6;

    let mut annotations = Vec::new();
    let geoms = vec![
        AnnotationGeometry::Text(Point3 {
            x: 1.25,
            y: -2.5,
            z: 0.0,
        }),
        AnnotationGeometry::Leader(vec![
            Point3::default(),
            Point3 {
                x: 4.0,
                y: 4.0,
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
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            axis_v: Point3 {
                x: 0.0,
                y: 1.5,
                z: 0.0,
            },
        },
        AnnotationGeometry::Freehand(vec![
            Point3::default(),
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        ]),
        AnnotationGeometry::Cloud(vec![
            Point3::default(),
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
        ]),
        AnnotationGeometry::Measurement(MeasurementRecord {
            algorithm: MeasurementAlgorithm::Angle3Points,
            inputs: vec![
                Point3::default(),
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 1.0,
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
            value: 90.0,
            units: unit_context.clone(),
            source: GeometrySource::ProxyCache,
            precision: Precision::Approximate {
                error_bound: Some(0.25),
            },
        }),
    ];
    for (i, geometry) in geoms.into_iter().enumerate() {
        let mut a = text_note(0x1000 + i as u128, "note");
        a.space = SpaceId::Paper(LayoutId(9));
        a.geometry = geometry;
        a.precision = Precision::Unknown;
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
        annotations.push(a);
    }

    let mut extensions = BTreeMap::new();
    extensions.insert("future_field".to_string(), "{\"a\":[1,2,3]}".to_string());

    let file = AnnotationFile {
        schema_version: SCHEMA_VERSION,
        application_version: "9.9.9".into(),
        document_fingerprint: identity.clone(),
        document_name_hint: "sample.dwg".into(),
        unit_context: unit_context.clone(),
        annotations,
        view_bookmarks: vec![ViewBookmark {
            name: "view-1".into(),
            viewport: ViewportId(42),
            space: SpaceId::Paper(LayoutId(3)),
            camera_transform: Transform3::translation(Point3 {
                x: 8.0,
                y: 9.0,
                z: 0.0,
            }),
        }],
        extensions_json: extensions,
        nested_extensions: Default::default(),
    };

    // First parse.
    let first_bytes = service.encode(&file).unwrap();
    let first = service
        .decode(&first_bytes, &identity, FingerprintPolicy::RejectMismatch)
        .unwrap();
    assert_eq!(first, file, "first parse altered the file");

    // Serialize the parsed value and parse again.
    let second_bytes = service.encode(&first).unwrap();
    let second = service
        .decode(&second_bytes, &identity, FingerprintPolicy::RejectMismatch)
        .unwrap();
    assert_eq!(second, first, "second parse altered the file");
    assert_eq!(first.unit_context, unit_context);
    assert_eq!(first.annotations[0].style.logical_width, 2.5);
    assert_eq!(first.annotations[0].style.text_height, 1.75);
    assert_eq!(first.view_bookmarks[0].name, "view-1");
    assert_eq!(first.schema_version, SCHEMA_VERSION);
}

/// An out-of-range `decimal_places` must be refused, not truncated by a cast.
#[test]
fn out_of_range_decimal_places_is_rejected_not_truncated() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "unit_context":{"source":"Millimeter","display":"Millimeter","decimal_places":300},
        "annotations":[]}"#;
    let err = service
        .decode(json, &identity, FingerprintPolicy::ImportUnanchored)
        .unwrap_err();
    assert!(matches!(err, CadError::CorruptData(_)), "got {err:?}");
}

/// An unrecognized unit string is corruption, not a silent `DrawingUnits`.
#[test]
fn unknown_unit_string_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "unit_context":{"source":"Parsec","display":"Meter"},"annotations":[]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

/// A malformed or missing fingerprint is corruption, not `Temporary(0)`, even
/// when the open drawing happens to be `Temporary(0)` (audit B11).
#[test]
fn malformed_or_missing_fingerprint_is_rejected() {
    let service = AnnotationService;
    let temporary = DocumentIdentity::Temporary(0);

    let garbage = br#"{"schema_version":1,"document_fingerprint":"not-a-uuid","annotations":[]}"#;
    assert!(matches!(
        service.decode(garbage, &temporary, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));

    let missing = br#"{"schema_version":1,"annotations":[]}"#;
    assert!(matches!(
        service.decode(missing, &temporary, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));

    let wrong_type = br#"{"schema_version":1,"document_fingerprint":7,"annotations":[]}"#;
    assert!(matches!(
        service.decode(wrong_type, &temporary, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

/// A malformed space object must not silently become `Paper(0)`.
#[test]
fn malformed_space_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "annotations":[{"id":"00000000-0000-0000-0000-000000000001","space":{"Other":"x"},
        "geometry":{"kind":"text","position":[0,0,0]},"text":"t","style":{"rgba":[1,2,3,4],"logical_width":1.0,"text_height":1.0},
        "created_unix_ms":0,"modified_unix_ms":0,"precision":{"kind":"analytic"}}]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

/// A missing anchor instance path must be refused, not silently empty.
#[test]
fn missing_anchor_instance_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "annotations":[{"id":"00000000-0000-0000-0000-000000000001","space":"Model",
        "geometry":{"kind":"text","position":[0,0,0]},"text":"t","style":{"rgba":[1,2,3,4],"logical_width":1.0,"text_height":1.0},
        "created_unix_ms":0,"modified_unix_ms":0,"precision":{"kind":"analytic"},
        "anchor":{"source_handle":"A","fallback":[0,0,0],"status":"Valid"}}]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

/// Missing required annotation fields must be refused, not defaulted.
#[test]
fn missing_required_annotation_fields_are_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    // No `text`, no `precision`, no `space`.
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "annotations":[{"id":"00000000-0000-0000-0000-000000000001",
        "geometry":{"kind":"text","position":[0,0,0]},"style":{"rgba":[1,2,3,4],"logical_width":1.0,"text_height":1.0},
        "created_unix_ms":0,"modified_unix_ms":0}]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

/// Duplicate ids would collapse on import; the codec must refuse them.
#[test]
fn duplicate_annotation_ids_are_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let one = r#"{"id":"00000000-0000-0000-0000-000000000001","space":"Model",
        "geometry":{"kind":"text","position":[0,0,0]},"text":"t","style":{"rgba":[1,2,3,4],"logical_width":1.0,"text_height":1.0},
        "created_unix_ms":0,"modified_unix_ms":0,"precision":{"kind":"analytic"}}"#;
    let json = format!(
        r#"{{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"annotations":[{one},{one}]}}"#
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

/// Extension fields must not be silently dropped on encode: a collision with a
/// schema field is refused so an export cannot claim success while losing data.
#[test]
fn extension_field_colliding_with_schema_is_refused_on_encode() {
    let service = AnnotationService;
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.extensions_json
        .insert("schema_version".to_string(), "99".to_string());
    let err = service.encode(&file).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "got {err:?}");
}

/// Exporting a file that claims a newer schema must be refused, not silently
/// downgraded to the current schema.
#[test]
fn encoding_a_newer_schema_is_refused() {
    let service = AnnotationService;
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.schema_version = SCHEMA_VERSION + 5;
    assert!(matches!(
        service.encode(&file),
        Err(CadError::Unsupported(_))
    ));
}

/// The fingerprint report must explain a mismatch and the rebinding rather than
/// dropping it silently (audit B10).
#[test]
fn fingerprint_report_explains_mismatch_and_rebinding() {
    let service = AnnotationService;
    let open = DocumentIdentity::Sha256([2u8; 32]);
    let mut a = text_note(1, "note");
    a.anchor = Some(EntityAnchor {
        source_handle: "A".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: Point3::default(),
        status: AnchorStatus::Valid,
    });
    let mut file = empty_file(DocumentIdentity::Sha256([9u8; 32]));
    file.annotations = vec![a];
    let bytes = service.encode(&file).unwrap();

    let outcome = service
        .decode_with_report(&bytes, &open, FingerprintPolicy::ImportUnanchored)
        .unwrap();
    assert!(!outcome.fingerprint.matched);
    assert!(outcome.fingerprint.rebound);
    assert_eq!(outcome.fingerprint.anchors_detached, 1);
    // The returned file is rebound to the open drawing and carries no anchors.
    assert_eq!(outcome.file.document_fingerprint, open);
    assert!(outcome.file.annotations[0].anchor.is_none());
}

/// An explicit mapping moves bookmarks too, not just annotation geometry.
#[test]
fn explicit_mapping_transforms_bookmarks_and_rebinds_identity() {
    let service = AnnotationService;
    let open = DocumentIdentity::Sha256([0u8; 32]);
    let mut file = empty_file(DocumentIdentity::Sha256([9u8; 32]));
    file.annotations = vec![text_note(1, "note")];
    file.view_bookmarks = vec![ViewBookmark {
        name: "view".into(),
        viewport: ViewportId(1),
        space: SpaceId::Model,
        camera_transform: Transform3::identity(),
    }];
    let bytes = service.encode(&file).unwrap();

    let mapping = Transform3::translation(Point3 {
        x: 5.0,
        y: 0.0,
        z: 0.0,
    });
    let outcome = service
        .decode_with_report(
            &bytes,
            &open,
            FingerprintPolicy::ExplicitCoordinateMapping(mapping),
        )
        .unwrap();
    assert!(outcome.fingerprint.geometry_transformed);
    assert_eq!(outcome.fingerprint.bookmarks_transformed, 1);
    assert_eq!(outcome.file.document_fingerprint, open);
    let moved = outcome.file.view_bookmarks[0]
        .camera_transform
        .apply_point(Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        });
    assert_eq!(moved.x, 6.0);
    assert_eq!(moved.y, 2.0);
}

/// A failed export must not mark the document saved or advance its revision,
/// and must leave the prior content intact (audit B07/F09).
#[test]
fn failed_export_does_not_mark_saved_or_corrupt_state() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    service
        .apply(&mut db, AnnotationCommand::Create(text_note(1, "kept")))
        .unwrap();
    let revision_before = db.revision();
    assert!(db.is_dirty());

    // An extension that collides with a schema field makes encode fail.
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = db.annotations().cloned().collect();
    file.extensions_json
        .insert("schema_version".to_string(), "99".to_string());

    let err = service.export_sidecar(&mut db, &file).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "got {err:?}");
    assert!(db.is_dirty(), "a failed export marked the document saved");
    assert_eq!(db.revision(), revision_before);
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "kept");
}

/// A successful export marks exactly the exported revision saved, and a later
/// edit flips dirty again (audit B07).
#[test]
fn successful_export_marks_saved_and_later_edit_is_dirty() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    service
        .apply(&mut db, AnnotationCommand::Create(text_note(1, "first")))
        .unwrap();

    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = db.annotations().cloned().collect();
    let bytes = service.export_sidecar(&mut db, &file).unwrap();
    assert!(!db.is_dirty());
    assert!(!bytes.is_empty());

    service
        .apply(&mut db, AnnotationCommand::Create(text_note(2, "second")))
        .unwrap();
    assert!(db.is_dirty());
}

/// A malformed bookmark is reported (the old codec silently emptied the list).
#[test]
fn malformed_view_bookmark_is_rejected() {
    let service = AnnotationService;
    let identity = DocumentIdentity::Sha256([0u8; 32]);
    let json = br#"{"schema_version":1,"document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "annotations":[],"view_bookmarks":[{"name":"v"}]}"#;
    assert!(matches!(
        service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
        Err(CadError::CorruptData(_))
    ));
}

#[test]
fn duplicate_ids_cannot_be_encoded_or_mark_export_saved() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let note = text_note(1, "kept");
    service
        .apply(&mut db, AnnotationCommand::Create(note.clone()))
        .unwrap();
    let revision = db.revision();
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = vec![note.clone(), note.clone()];
    assert!(matches!(
        service.encode(&file),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        service.export_sidecar(&mut db, &file),
        Err(CadError::InvalidInput(_))
    ));
    assert!(db.is_dirty());
    assert_eq!(db.revision(), revision);
    assert_eq!(db.get(note.id), Some(&note));
}

#[test]
fn stale_or_incomplete_exports_preserve_dirty_revision_and_contents() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let original = text_note(1, "original");
    service
        .apply(&mut db, AnnotationCommand::Create(original.clone()))
        .unwrap();
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = vec![original.clone()];
    service.export_sidecar(&mut db, &file).unwrap();
    let current = text_note(1, "current");
    service
        .apply(&mut db, AnnotationCommand::Update(current.clone()))
        .unwrap();
    let revision = db.revision();
    let variants = [
        vec![original],
        Vec::new(),
        vec![text_note(2, "foreign")],
        vec![current.clone(), text_note(2, "extra")],
    ];
    for annotations in variants {
        file.annotations = annotations;
        // A standalone sidecar is valid; it just cannot confirm this revision.
        assert!(service.encode(&file).is_ok());
        assert!(matches!(
            service.export_sidecar(&mut db, &file),
            Err(CadError::InvalidInput(_))
        ));
        assert!(db.is_dirty());
        assert_eq!(db.revision(), revision);
        assert_eq!(db.len(), 1);
        assert_eq!(db.get(current.id), Some(&current));
    }
    file.annotations = vec![current];
    service.export_sidecar(&mut db, &file).unwrap();
    assert!(!db.is_dirty());
    assert_eq!(db.revision(), revision);
}

#[test]
fn matching_export_snapshot_does_not_require_database_order() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let first = text_note(1, "first");
    let second = text_note(2, "second");
    for note in [&first, &second] {
        service
            .apply(&mut db, AnnotationCommand::Create(note.clone()))
            .unwrap();
    }
    let revision = db.revision();
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = vec![second, first];
    service.export_sidecar(&mut db, &file).unwrap();
    assert!(!db.is_dirty());
    assert_eq!(db.revision(), revision);
}

#[test]
fn raw_reserved_sideband_is_rejected_instead_of_silently_discarded() {
    let service = AnnotationService;
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let note = text_note(1, "kept");
    service
        .apply(&mut db, AnnotationCommand::Create(note.clone()))
        .unwrap();
    let revision = db.revision();
    let mut file = empty_file(DocumentIdentity::Sha256([0u8; 32]));
    file.annotations = vec![note.clone()];
    for raw in [r#"{"version":1,"annotations":[]}"#, "not JSON"] {
        file.extensions_json
            .insert(cad_annotations::NESTED_EXTENSIONS_KEY.into(), raw.into());
        assert!(matches!(
            service.encode(&file),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            service.export_sidecar(&mut db, &file),
            Err(CadError::InvalidInput(_))
        ));
        assert!(db.is_dirty());
        assert_eq!(db.revision(), revision);
        assert_eq!(db.get(note.id), Some(&note));
        assert_eq!(
            file.extensions_json[cad_annotations::NESTED_EXTENSIONS_KEY],
            raw
        );
    }
}
