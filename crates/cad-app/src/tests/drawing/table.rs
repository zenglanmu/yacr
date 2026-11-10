//! First-class `ACAD_TABLE` command contracts: transform, bounds, FIND,
//! selection/filter, properties, EXPLODE and grip refusals.
//!
//! The import/export round trips and the display-equivalence contract live in
//! `cad-import-acadrust`; this module exercises the application surfaces a user
//! actually reaches.

use super::*;

use cad_domain::{
    TableCellBorders, TableCellGeometry, TableColumnGeometry, TableFrame, TableGeometry,
    TableParts, TableRowGeometry,
};

fn cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
    }
}

/// A 1x2 table at the origin: 4 + 6 wide, 3 high, cells `id` and `name`.
fn table_parts() -> TableParts {
    TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 6.0,
                name: "B".into(),
            },
        ],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell("id"), cell("name")],
        }],
        style_name: "Standard".into(),
        name: "Data".into(),
        description: String::new(),
    }
}

fn table_geometry() -> TableGeometry {
    TableGeometry::new(table_parts()).expect("valid table")
}

fn table_app() -> (Application, SessionState) {
    table_app_with(table_geometry())
}

fn table_app_with(geometry: TableGeometry) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbTable".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Table(Box::new(geometry)),
        draw_order: 0,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn stored_table(app: &Application) -> TableGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .expect("the table entity");
    match &entity.geometry {
        SemanticGeometry::Table(table) => table.as_ref().clone(),
        other => panic!("expected a table, got {other:?}"),
    }
}

#[test]
fn move_table_maps_the_frame_and_rebuilds_the_display() {
    let (mut app, mut session) = table_app();
    let before = stored_table(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: p3(10.0, 5.0, 0.0),
        },
    )
    .unwrap();
    let moved = stored_table(&app);
    let origin = moved.frame().origin;
    assert!((origin.x - 10.0).abs() < 1e-9 && (origin.y - 5.0).abs() < 1e-9);
    assert_eq!(moved.display().len(), before.display().len());
    assert!((moved.columns()[0].width - 4.0).abs() < 1e-9);
    assert_eq!(moved.texts(), vec!["id".to_string(), "name".to_string()]);

    // One undo restores the original frame and display.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_table(&app), before);
}

#[test]
fn scale_and_rotate_table_through_commands_map_sizes_and_frame() {
    let (mut app, mut session) = table_app();
    execute(
        &mut app,
        &mut session,
        CommandId::ScaleEntities,
        CommandPayload::Scale {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            factor: 2.0,
        },
    )
    .unwrap();
    let scaled = stored_table(&app);
    assert!((scaled.columns()[0].width - 8.0).abs() < 1e-9);
    assert!((scaled.columns()[1].width - 12.0).abs() < 1e-9);
    assert!((scaled.rows()[0].height - 6.0).abs() < 1e-9);
    assert!((scaled.cell(0, 0).unwrap().text_height - 5.0).abs() < 1e-9);

    execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            angle: std::f64::consts::FRAC_PI_2,
        },
    )
    .unwrap();
    let rotated = stored_table(&app);
    let horizontal = rotated.frame().horizontal;
    assert!(horizontal.x.abs() < 1e-9, "{horizontal:?}");
    assert!((horizontal.y - 1.0).abs() < 1e-9, "{horizontal:?}");
    assert!((rotated.columns()[0].width - 8.0).abs() < 1e-9);
}

#[test]
fn table_transform_refuses_a_non_uniform_or_mirrored_placement_by_name() {
    let geometry = table_geometry();
    let mut non_uniform = Transform3::identity();
    non_uniform.matrix[1][1] = 2.0;
    let error = cad_db::transform_geometry(
        &SemanticGeometry::Table(Box::new(geometry.clone())),
        &non_uniform,
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref message) if message.contains("table transform")),
        "{error:?}"
    );
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    let error = cad_db::transform_geometry(&SemanticGeometry::Table(Box::new(geometry)), &mirror)
        .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref message) if message.contains("mirror")),
        "{error:?}"
    );
}

#[test]
fn table_contributes_its_display_extent_to_drawing_bounds() {
    let (app, _session) = table_app();
    let (min, max) = app.workspace.documents[&DocumentId(1)]
        .drawing
        .bounds()
        .expect("the table has an extent");
    // The grid itself is 10 x 3. A `Text` child's bounds are conservatively
    // expanded by the pre-existing text rule (height x character count), so the
    // table's box must *contain* that grid rather than equal it exactly.
    assert!(min.x <= 1e-9 && min.y <= 1e-9, "{min:?}");
    assert!(max.x >= 10.0 - 1e-9, "{max:?}");
    assert!(max.y >= 3.0 - 1e-9, "{max:?}");
}

#[test]
fn find_reports_and_replaces_table_cell_text() {
    let (mut app, mut session) = table_app();
    let report = execute(
        &mut app,
        &mut session,
        CommandId::FindReport,
        CommandPayload::TextFind {
            needle: "NAME".into(),
            replacement: None,
        },
    )
    .unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "find.result"
                && diagnostic.message.contains("1 match(es)")),
        "{report:?}"
    );

    let replaced = execute(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        CommandPayload::TextFind {
            needle: "id".into(),
            replacement: Some("code".into()),
        },
    )
    .unwrap();
    assert_eq!(replaced.objects.len(), 1);
    assert_eq!(
        stored_table(&app).texts(),
        vec!["code".to_string(), "name".to_string()]
    );
    // One undo restores the cell text (and therefore the rebuilt display).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_table(&app).texts(),
        vec!["id".to_string(), "name".to_string()]
    );
}

#[test]
fn selection_filter_and_properties_expose_the_table() {
    let (app, _session) = table_app();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entity = drawing.entity(EntityId(1)).expect("the table");
    assert_eq!(
        crate::select_filter::geometry_key(&entity.geometry),
        "TABLE"
    );
    assert!(crate::select_filter::type_matches(entity, "AcDbTable"));
    assert!(crate::select_filter::type_matches(entity, "TABLE"));

    let rows = crate::selection::entity_property_rows(drawing, &ref_of(1), entity);
    for key in ["rows", "columns", "cells", "style_name", "name"] {
        assert!(
            rows.iter().any(|row| row.key == key),
            "missing property row {key}: {rows:?}"
        );
    }
}

#[test]
fn explode_refuses_a_real_table_by_name_without_writing() {
    let (mut app, mut session) = table_app();
    session.selection.insert(ref_of(1));
    let revision_before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref message)
            if message.contains("EXPLODE refuses AcDbTable")),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
}

#[test]
fn grip_drag_refuses_a_table_by_name() {
    let geometry = SemanticGeometry::Table(Box::new(table_geometry()));
    let grip = crate::grips::GripId {
        entity: EntityId(1),
        kind: crate::grips::GripKind::Point,
        index: 0,
    };
    let error = crate::grips::apply_grip(&geometry, &grip, p3(1.0, 1.0, 0.0)).unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref message)
            if message.contains("grip.unsupported_table")),
        "{error:?}"
    );
}

#[test]
fn find_never_matches_across_two_table_cells() {
    let (mut app, mut session) = table_app();
    // The old implementation joined cell texts with one space, so "id name"
    // matched a table whose cells are only "id" and "name".
    let report = execute(
        &mut app,
        &mut session,
        CommandId::FindReport,
        CommandPayload::TextFind {
            needle: "id name".into(),
            replacement: None,
        },
    )
    .unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "find.result"
                && diagnostic.message.contains("0 match(es)")),
        "{report:?}"
    );
}

#[test]
fn find_replaces_the_same_needle_in_two_cells_with_one_undo_step() {
    let mut geometry = table_parts();
    geometry.rows[0].cells = vec![cell("same"), cell("same")];
    let rotated = TableGeometry::from_import(geometry);
    let (mut app, mut session) = table_app_with(rotated);
    let undo_before = app
        .history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        CommandPayload::TextFind {
            needle: "same".into(),
            replacement: Some("done".into()),
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one table entity is updated");
    assert_eq!(
        stored_table(&app).texts(),
        vec!["done".to_string(), "done".to_string()]
    );
    let undo_after = app
        .history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0);
    assert_eq!(
        undo_after,
        undo_before + 1,
        "one transaction / one undo step"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_table(&app).texts(),
        vec!["same".to_string(), "same".to_string()]
    );
}

#[test]
fn a_rotated_table_is_picked_boxed_and_bounded_in_world_space() {
    let rotate = Transform3::from_basis(p3(0.0, 1.0, 0.0), p3(-1.0, 0.0, 0.0), p3(0.0, 0.0, 0.0));
    let rotated = table_geometry().transformed(&rotate).expect("rotation");
    let geometry = SemanticGeometry::Table(Box::new(rotated.clone()));
    let identity = Transform3::identity();

    // Picking uses the conservative world AABB of the derived display. Cell
    // text expands it beyond the bare grid, so assert containment of the
    // rotated grid rather than equality.
    let (min, max) = cad_spatial::pick::pick_world_bounds(&geometry, &identity)
        .expect("the rotated table has bounds");
    assert!(min.x <= -3.0 + 1e-9, "{min:?} {max:?}");
    assert!(max.x >= 0.0 - 1e-9, "{min:?} {max:?}");
    assert!(min.y <= 1e-9, "{min:?} {max:?}");
    assert!(max.y >= 10.0 - 1e-9, "{min:?} {max:?}");

    // Box selection classifies the rotated display, not the local grid.
    let hit = crate::select_box::Rect::from_corners(p3(-3.5, -1.0, 0.0), p3(0.5, 11.0, 0.0));
    assert!(crate::select_box::geometry_hits_rect(
        &geometry, &identity, hit
    ));
    let miss = crate::select_box::Rect::from_corners(p3(20.0, 20.0, 0.0), p3(21.0, 21.0, 0.0));
    assert!(!crate::select_box::geometry_hits_rect(
        &geometry, &identity, miss
    ));

    // Drawing bounds are the rotated extent (text conservatively inflated).
    let (app, _session) = table_app_with(rotated);
    let (bounds_min, bounds_max) = app.workspace.documents[&DocumentId(1)]
        .drawing
        .bounds()
        .expect("the rotated table has an extent");
    assert!(bounds_min.x <= -3.0 + 1e-9, "{bounds_min:?}");
    assert!(bounds_max.x >= 0.0 - 1e-9, "{bounds_max:?}");
    assert!(bounds_min.y <= 1e-9, "{bounds_min:?}");
    assert!(bounds_max.y >= 10.0 - 1e-9, "{bounds_max:?}");
}
