//! Contract tests for the DGN vector importer.
//!
//! The two committed fixtures are GDAL autotest files (`fixtures/dgn/`): a V7
//! design file and a V8 (OLE) model with a shared cell, a raster-less tag and an
//! unmodelled multi-line element. Synthetic `cadkit_core::Document`s cover the
//! degradation paths that the fixtures do not contain. Device rendering,
//! pixels and performance are **not** covered here.

use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{
    CadError, DatabaseId, DocumentId, Point3, SemanticGeometry, TransactionId, Unit, UnitContext,
};
use cad_import_core::{
    compute_identity, ImportLimits, ImportReport, ImportRequest, ImportedDrawing, Importer,
};
use cadkit_core::{
    Block as KitBlock, Entity, EntityKind, Layer as KitLayer, LengthUnit, Model, ModelKind,
    Point3 as KitPoint3, Units, Vec3,
};

use crate::map::Mapper;
use crate::{
    DgnImporter, BLOCK_MISSING, DEPTH_LIMIT, ENTITY_LIMIT, GEOMETRY_REJECTED,
    LEVEL_SYMBOLOGY_UNDECODED, NOT_A_DGN_FILE, NO_GEOMETRY, RASTER_NOT_LOADED, SPLINE_FIT_POINTS,
    TAGS_NOT_STORED, UNKNOWN_ELEMENT, UNKNOWN_ELEMENTS_ONLY,
};

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dgn")
        .join(name);
    std::fs::read(path).expect("fixture is committed")
}

fn import_fixture(name: &str) -> ImportedDrawing {
    DgnImporter::new()
        .import(&request(fixture(name)), &|| false)
        .expect("fixture imports")
}

fn codes(report: &ImportReport) -> Vec<&str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn close(left: Point3, right: Point3) -> bool {
    (left.x - right.x).abs() < 1e-9
        && (left.y - right.y).abs() < 1e-9
        && (left.z - right.z).abs() < 1e-9
}

fn find(
    database: &DrawingDatabase,
    predicate: impl Fn(&SemanticGeometry) -> bool,
) -> Option<&cad_db::DbEntity> {
    database
        .entities()
        .find(|entity| predicate(&entity.geometry))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[test]
fn v7_imports_text_circle_polyline_line() {
    let drawing = import_fixture("smalltest.dgn");
    assert_eq!(
        drawing.units.source,
        Unit::DrawingUnits,
        "smalltest declares no master unit size"
    );
    let mut layers: Vec<&str> = drawing
        .database
        .layers()
        .map(|layer| layer.name.as_str())
        .collect();
    layers.sort();
    assert_eq!(layers, ["Level 1", "Level 2"]);
    assert_eq!(drawing.database.entity_count(), 4);

    let text = find(&drawing.database, |geometry| {
        matches!(geometry, SemanticGeometry::Text { .. })
    })
    .expect("the V7 file has text");
    match &text.geometry {
        SemanticGeometry::Text {
            text,
            height,
            h_align,
            v_align,
            ..
        } => {
            assert_eq!(text, "Demo Text");
            assert!((height - 1.0).abs() < 1e-6);
            assert_eq!(*h_align, cad_domain::TextAlignH::Center);
            assert_eq!(*v_align, cad_domain::TextAlignV::Middle);
        }
        other => panic!("expected text, got {other:?}"),
    }

    let circle = find(&drawing.database, |geometry| {
        matches!(geometry, SemanticGeometry::Circle { .. })
    })
    .expect("the V7 file has a circle");
    match &circle.geometry {
        SemanticGeometry::Circle { radius, .. } => {
            assert!((radius - 4.679_606_583_891_428).abs() < 1e-9);
        }
        other => panic!("expected circle, got {other:?}"),
    }

    let polyline = find(&drawing.database, |geometry| {
        matches!(geometry, SemanticGeometry::Polyline { .. })
    })
    .expect("the V7 file has a shape");
    match &polyline.geometry {
        SemanticGeometry::Polyline {
            points,
            closed,
            bulges,
            width,
        } => {
            assert_eq!(points.len(), 4, "the shape has four vertices");
            assert!(*closed, "the DGN shape is closed");
            assert!(bulges.is_empty(), "DGN line strings carry no bulges");
            assert_eq!(*width, 0.0);
        }
        other => panic!("expected polyline, got {other:?}"),
    }

    assert!(
        find(&drawing.database, |geometry| matches!(
            geometry,
            SemanticGeometry::Line { .. }
        ))
        .is_some(),
        "the V7 file has a line"
    );
    assert!(codes(&drawing.report).contains(&LEVEL_SYMBOLOGY_UNDECODED));
    assert!(matches!(
        drawing.report.completeness,
        cad_domain::Completeness::Partial(_)
    ));
    // No level symbology was invented: no layer carries an attribute row.
    for layer in drawing.database.layers() {
        assert!(
            drawing.database.layer_attributes_ref(layer.id).is_none(),
            "layer '{}' must not carry fabricated attributes",
            layer.name
        );
    }
}

#[test]
fn v8_imports_metres_block_insert_and_flattens_groups() {
    let drawing = import_fixture("test_dgnv8.dgn");
    assert_eq!(drawing.units.source, Unit::Meter);
    assert_eq!(drawing.database.layers().count(), 3);

    let block_id = drawing
        .database
        .block_by_name("Named definition")
        .expect("the shared cell definition is stored")
        .id;
    let block = drawing.database.block(block_id).expect("block row");
    assert_eq!(block.entities.len(), 1, "one member circle");
    let member = drawing.database.entity(block.entities[0]).expect("member");
    match &member.geometry {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 1e-4).abs() < 1e-12),
        other => panic!("expected the block member circle, got {other:?}"),
    }

    let insert = find(&drawing.database, |geometry| {
        matches!(geometry, SemanticGeometry::Insert { .. })
    })
    .expect("the shared cell instance is stored");
    match &insert.geometry {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, block_id);
            assert!(
                close(
                    transform.apply_point(Point3::default()),
                    Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 2.0
                    }
                ),
                "the instance lands at its stored position: {transform:?}"
            );
            let scaled = transform.apply_vector(Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            });
            assert!(
                (scaled.x - 10_000.0).abs() < 1e-6,
                "scale is 10000: {scaled:?}"
            );
        }
        other => panic!("expected insert, got {other:?}"),
    }

    // Groups are flattened: no inserted row carries a group kind.
    assert!(
        drawing
            .database
            .entities()
            .all(|entity| !entity.object.type_key.starts_with("DgnGroup")),
        "no group row may be stored"
    );
    // The unmodelled multi-line (type 36) was skipped, not stored as Unknown.
    assert!(
        drawing
            .database
            .entities()
            .all(|entity| entity.object.type_key != "DgnUnknown"),
        "unknown elements are diagnostics, not rows"
    );
    let codes = codes(&drawing.report);
    assert!(
        codes.contains(&UNKNOWN_ELEMENT),
        "type 36 is reported: {codes:?}"
    );
    assert!(
        codes.contains(&TAGS_NOT_STORED),
        "the tag is reported: {codes:?}"
    );
    assert!(codes.contains(&LEVEL_SYMBOLOGY_UNDECODED));
    assert!(matches!(
        drawing.report.completeness,
        cad_domain::Completeness::Partial(_)
    ));
}

#[test]
fn v8_linetype_pattern_is_scaled_from_uor_to_metres() {
    // `test_dgnv8` stores DGN_DOT_DASH as [4.5, -1.5, 0, -1.5] UOR with
    // 10_000 UOR per metre, so the stored pattern is 0.00045 m, etc.
    let drawing = import_fixture("test_dgnv8.dgn");
    let linetype = drawing
        .database
        .linetypes()
        .find(|linetype| linetype.name == "DGN_DOT_DASH")
        .expect("the dashed linetype is stored");
    let expected = [0.000_45, -0.000_15, 0.0, -0.000_15];
    assert_eq!(linetype.pattern.elements.len(), expected.len());
    for (stored, expected) in linetype.pattern.elements.iter().zip(expected) {
        assert!(
            (stored - expected).abs() < 1e-12,
            "dash length {stored} != {expected} m"
        );
    }
}

#[test]
fn v8_level_flags_are_reported_and_never_become_layer_attributes() {
    let drawing = import_fixture("test_dgnv8.dgn");
    assert!(codes(&drawing.report).contains(&LEVEL_SYMBOLOGY_UNDECODED));
    match &drawing.report.completeness {
        cad_domain::Completeness::Partial(reasons) => assert!(
            reasons[0].contains("level"),
            "the level loss is the first reason: {reasons:?}"
        ),
        other => panic!("DGN completeness is always Partial, got {other:?}"),
    }
    // The "Default" level carries `dgn.level_flags` in the reader model, but the
    // database must not invent colour/off/frozen/locked values from it.
    for layer in drawing.database.layers() {
        let attributes = drawing.database.layer_attributes(layer.id);
        assert!(!attributes.off);
        assert!(!attributes.frozen);
        assert!(!attributes.locked);
        assert!(drawing.database.layer_attributes_ref(layer.id).is_none());
    }
}

#[test]
fn every_imported_entity_is_writable_through_the_validated_path() {
    // `finish()` does not check dangling block/style references; the first edit
    // does. Writing every imported row catches those before a user hits them.
    let drawing = import_fixture("test_dgnv8.dgn");
    let mut database = drawing.database.clone();
    let changes: Vec<_> = database
        .entities()
        .map(|entity| (entity.id, Some(entity.clone())))
        .collect();
    assert!(!changes.is_empty());
    database
        .apply_drawing_changes("dgn import write test", TransactionId(1), changes)
        .expect("every imported entity is a valid write target");
}

// ---------------------------------------------------------------------------
// Refusals and limits
// ---------------------------------------------------------------------------

#[test]
fn not_a_dgn_file_is_refused_by_name() {
    let error = DgnImporter::new()
        .import(&request(b"not a dgn file".to_vec()), &|| false)
        .err()
        .expect("random bytes are not a DGN file");
    match error {
        CadError::CorruptData(message) => assert!(message.contains(NOT_A_DGN_FILE), "{message}"),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn over_file_limit_is_invalid_input() {
    let mut request = request(fixture("smalltest.dgn"));
    request.limits.max_file_bytes = 16;
    let error = DgnImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the file is over the limit");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
}

#[test]
fn over_entity_limit_is_a_named_invalid_input() {
    let mut request = request(fixture("test_dgnv8.dgn"));
    request.limits.max_entities = 1;
    let error = DgnImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the file has more entities than the limit");
    match error {
        CadError::InvalidInput(message) => assert!(message.contains(ENTITY_LIMIT), "{message}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn cancellation_stops_before_reading() {
    let error = DgnImporter::new()
        .import(&request(fixture("smalltest.dgn")), &|| true)
        .err()
        .expect("cancelled");
    assert!(matches!(error, CadError::Cancelled), "{error:?}");
}

// ---------------------------------------------------------------------------
// Synthetic documents: degradation paths the fixtures do not contain
// ---------------------------------------------------------------------------

#[allow(clippy::field_reassign_with_default)]
fn synthetic(entities: Vec<Entity>) -> cadkit_core::Document {
    let mut document = cadkit_core::Document::default();
    document.units = Units::from_insunits(LengthUnit::Meter.to_insunits());
    let mut model = Model::default();
    model.name = "Model".into();
    model.kind = ModelKind::Model;
    model.entities = entities;
    model.props.insert(
        "dgn.uor_per_master".into(),
        cadkit_core::Value::Float(10_000.0),
    );
    document.models.push(model);
    let mut layer = KitLayer::default();
    layer.name = "0".into();
    layer.visible = true;
    document.layers.push(layer);
    document
}

fn map_document(
    document: &cadkit_core::Document,
    limits: &ImportLimits,
) -> cad_domain::CadResult<(DrawingDatabase, UnitContext, ImportReport)> {
    let cancelled = || false;
    let mut mapper = Mapper::new(DatabaseId(1), document, limits, &cancelled);
    mapper.map_tables()?;
    mapper.map_blocks()?;
    mapper.map_primary_model()?;
    mapper.finish(compute_identity(&[]), 0.0)
}

fn kit_point(x: f64, y: f64, z: f64) -> KitPoint3 {
    KitPoint3::new(x, y, z)
}

fn line(start: KitPoint3, end: KitPoint3) -> Entity {
    Entity::new(EntityKind::Line { start, end })
}

fn cell_insert(block: &str) -> Entity {
    Entity::new(EntityKind::Insert {
        block: block.into(),
        position: KitPoint3::ZERO,
        scale: Vec3::ONE,
        rotation: 0.0,
        normal: Vec3::Z,
        columns: 1,
        rows: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
    })
}

fn cell_definition(name: &str, entities: Vec<Entity>) -> KitBlock {
    let mut block = KitBlock::default();
    block.name = name.into();
    block.entities = entities;
    block
}

#[test]
fn an_unknown_only_document_is_unsupported_not_empty_success() {
    let document = synthetic(vec![Entity::new(EntityKind::Unknown {
        type_name: "dgn.type_99".into(),
    })]);
    let error = map_document(&document, &ImportLimits::default())
        .expect_err("unknown-only is not an import");
    match error {
        CadError::Unsupported(message) => {
            assert!(message.contains(UNKNOWN_ELEMENTS_ONLY), "{message}");
            assert!(
                message.contains("dgn.type_99"),
                "type names are listed: {message}"
            );
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn a_document_with_no_entity_refuses_with_no_geometry() {
    let document = synthetic(Vec::new());
    let error = map_document(&document, &ImportLimits::default()).expect_err("nothing drawable");
    match error {
        CadError::Unsupported(message) => assert!(message.contains(NO_GEOMETRY), "{message}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn a_fit_point_only_spline_becomes_a_polyline() {
    let entity = Entity::new(EntityKind::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: Vec::new(),
        weights: Vec::new(),
        fit_points: vec![
            kit_point(0.0, 0.0, 0.0),
            kit_point(1.0, 1.0, 0.0),
            kit_point(2.0, 0.0, 0.0),
            kit_point(3.0, 1.0, 0.0),
        ],
        closed: false,
    });
    let document = synthetic(vec![entity]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("fit points are drawable");
    assert_eq!(database.entity_count(), 1);
    let stored = database.entities().next().expect("one row");
    match &stored.geometry {
        SemanticGeometry::Polyline { points, closed, .. } => {
            assert_eq!(points.len(), 4);
            assert!(!closed);
        }
        other => panic!("expected a polyline fallback, got {other:?}"),
    }
    assert!(codes(&report).contains(&SPLINE_FIT_POINTS));
    assert!(matches!(
        report.completeness,
        cad_domain::Completeness::Partial(_)
    ));
}

#[test]
fn an_insert_without_a_block_definition_is_skipped_not_invented() {
    let insert = Entity::new(EntityKind::Insert {
        block: "Nope".into(),
        position: KitPoint3::ZERO,
        scale: Vec3::ONE,
        rotation: 0.0,
        normal: Vec3::Z,
        columns: 1,
        rows: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
    });
    let document = synthetic(vec![
        line(kit_point(0.0, 0.0, 0.0), kit_point(1.0, 0.0, 0.0)),
        insert,
    ]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the line imports");
    assert_eq!(
        database.entity_count(),
        1,
        "no fake block instance is stored"
    );
    assert!(codes(&report).contains(&BLOCK_MISSING));
    // No empty block row was invented either.
    assert_eq!(database.blocks().count(), 0);
}

#[test]
fn a_cyclic_cell_graph_is_not_emitted_recursively() {
    // A -> B -> A. The back edge (Insert A) must not be stored, or the
    // representation would recurse forever expanding the reference.
    let mut document = synthetic(vec![
        line(kit_point(0.0, 0.0, 0.0), kit_point(1.0, 0.0, 0.0)),
        cell_insert("A"),
    ]);
    document
        .blocks
        .push(cell_definition("A", vec![cell_insert("B")]));
    document
        .blocks
        .push(cell_definition("B", vec![cell_insert("A")]));
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the line imports");
    assert!(
        codes(&report).contains(&DEPTH_LIMIT),
        "the cycle is reported: {:?}",
        codes(&report)
    );
    let a = database.block_by_name("A").expect("A is stored").id;
    for entity in database.entities() {
        if let SemanticGeometry::Insert { block, .. } = &entity.geometry {
            assert_ne!(*block, a, "no recursive Insert back to A may be stored");
        }
    }
}

#[test]
fn a_raster_is_image_geometry_with_per_pixel_vectors() {
    let image = Entity::new(EntityKind::Image {
        path: Some("scan.tif".into()),
        position: kit_point(1.0, 2.0, 3.0),
        u_vector: Vec3::new(2.0, 0.0, 0.0),
        v_vector: Vec3::new(0.0, 4.0, 0.0),
        size_px: Some([100, 200]),
    });
    let document = synthetic(vec![image]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the raster placement imports");
    let stored = database.entities().next().expect("one row");
    match &stored.geometry {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            ..
        } => {
            assert!(close(
                *origin,
                Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0
                }
            ));
            assert!(
                close(
                    *u,
                    Point3 {
                        x: 0.02,
                        y: 0.0,
                        z: 0.0
                    }
                ),
                "u is per-pixel: {u:?}"
            );
            assert!(
                close(
                    *v,
                    Point3 {
                        x: 0.0,
                        y: 0.02,
                        z: 0.0
                    }
                ),
                "v is per-pixel: {v:?}"
            );
            assert_eq!(*pixels, [100.0, 200.0]);
            assert_eq!(file.as_deref(), Some("scan.tif"));
        }
        other => panic!("expected image geometry, got {other:?}"),
    }
    assert!(codes(&report).contains(&RASTER_NOT_LOADED));
}

#[test]
fn a_raster_without_a_pixel_size_is_an_opaque_placeholder() {
    let image = Entity::new(EntityKind::Image {
        path: Some("scan.tif".into()),
        position: kit_point(0.0, 0.0, 0.0),
        u_vector: Vec3::new(1.0, 0.0, 0.0),
        v_vector: Vec3::new(0.0, 1.0, 0.0),
        size_px: None,
    });
    let document = synthetic(vec![image]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the placeholder imports");
    let stored = database.entities().next().expect("one row");
    match &stored.geometry {
        SemanticGeometry::Opaque { type_key, .. } => {
            assert_eq!(type_key, "DgnRasterReference");
        }
        other => panic!("expected an opaque raster placeholder, got {other:?}"),
    }
    assert!(codes(&report).contains(&RASTER_NOT_LOADED));
}

#[test]
fn a_degenerate_ellipse_is_dropped_with_a_diagnostic() {
    let ellipse = Entity::new(EntityKind::Ellipse {
        center: kit_point(0.0, 0.0, 0.0),
        major_axis: Vec3::new(1.0, 0.0, 0.0),
        // Upstream bug #24 has produced ratio 0; it cannot be drawn.
        ratio: 0.0,
        start_param: 0.0,
        end_param: std::f64::consts::TAU,
        normal: Vec3::Z,
    });
    let document = synthetic(vec![
        line(kit_point(0.0, 0.0, 0.0), kit_point(1.0, 0.0, 0.0)),
        ellipse,
    ]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the line still imports");
    assert_eq!(database.entity_count(), 1);
    assert!(codes(&report).contains(&GEOMETRY_REJECTED));
}

#[test]
fn a_zero_sweep_arc_is_dropped_with_a_diagnostic() {
    let arc = Entity::new(EntityKind::Arc {
        center: kit_point(0.0, 0.0, 0.0),
        radius: 1.0,
        start_angle: 0.5,
        end_angle: 0.5,
        normal: Vec3::Z,
    });
    let document = synthetic(vec![
        line(kit_point(0.0, 0.0, 0.0), kit_point(1.0, 0.0, 0.0)),
        arc,
    ]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the line still imports");
    assert_eq!(database.entity_count(), 1);
    assert!(codes(&report).contains(&GEOMETRY_REJECTED));
}

#[test]
fn a_spline_with_no_usable_curve_data_is_dropped_with_a_diagnostic() {
    let spline = Entity::new(EntityKind::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: Vec::new(),
        weights: Vec::new(),
        fit_points: vec![kit_point(0.0, 0.0, 0.0)],
        closed: false,
    });
    let document = synthetic(vec![
        line(kit_point(0.0, 0.0, 0.0), kit_point(1.0, 0.0, 0.0)),
        spline,
    ]);
    let (database, _, report) =
        map_document(&document, &ImportLimits::default()).expect("the line still imports");
    assert_eq!(database.entity_count(), 1);
    assert!(codes(&report).contains(&GEOMETRY_REJECTED));
}

#[test]
fn a_full_ellipse_keeps_its_full_turn() {
    let ellipse = Entity::new(EntityKind::Ellipse {
        center: kit_point(1.0, 2.0, 0.0),
        major_axis: Vec3::new(2.0, 0.0, 0.0),
        ratio: 0.5,
        start_param: 0.0,
        end_param: std::f64::consts::TAU,
        normal: Vec3::Z,
    });
    let document = synthetic(vec![ellipse]);
    let (database, _, _) =
        map_document(&document, &ImportLimits::default()).expect("the ellipse imports");
    let stored = database.entities().next().expect("one row");
    match &stored.geometry {
        SemanticGeometry::Ellipse { sweep, ratio, .. } => {
            assert!((sweep - std::f64::consts::TAU).abs() < 1e-9);
            assert!((ratio - 0.5).abs() < 1e-12);
        }
        other => panic!("expected an ellipse, got {other:?}"),
    }
}
