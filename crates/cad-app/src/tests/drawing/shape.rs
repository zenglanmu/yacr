//! `SHAPE` creation and `SHP2BLK` conversion contracts.
//!
//! `SHAPE` writes one `SemanticGeometry::Shape` row through the shared create
//! funnel (one transaction / one undo step) with the font resolved from the
//! explicit payload key or the drawing's `Standard` style. `SHP2BLK` converts the
//! selected rows into per-glyph outline block definitions plus one INSERT at each
//! original position, deleting the sources in one transaction.
//!
//! The outline engine is the process-wide font hook `TXTEXP` uses. The committed
//! `osifont.ttf` is always registered; `complex.shx` is third-party and not
//! committed (see `docs/fonts.md`), so it is read at run time through
//! `tests::font_fixture` and a checkout that never staged it skips the SHX cases
//! with an explicit message. Both modules install the same face set because the
//! first installation wins, and the SHX cases address a code `complex.shx`
//! really contains.

use super::*;
use crate::command_payload::ShapeCommand;
use cad_db::{BlockDefinition, BlockMetadata, BlockRow, Style};
use cad_representation::FontEngine;
use std::sync::Arc;

/// Install the same engine the TXTEXP tests install (first installation wins, so
/// the two test modules must register the identical face set).
fn install_outline_engine() {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    match crate::tests::font_fixture::shx_face("complex.shx") {
        Some(face) => {
            engine
                .register("complex.shx", Arc::from(face.as_slice()))
                .unwrap();
        }
        None => {
            // Never silent: the osifont-only engine is installed and the skip is
            // printed with the staging command.
            eprintln!(
                "{}",
                crate::tests::font_fixture::missing_face_hint("complex.shx")
            );
        }
    }
    let _ = crate::app_extended_w9_reports::text_explode::install_outline_fonts(Arc::new(engine));
}

/// The installed engine must actually contain the SHX face the tests address.
fn shape_font_available() -> bool {
    let Some(fonts) = crate::app_extended_w9_reports::text_explode::outline_fonts() else {
        return false;
    };
    fonts.contains("complex.shx")
}

/// Layer `0` + `WALLS`, an optional `Standard` style font, and no entities.
fn shape_app(style_font: Option<&str>) -> (Application, SessionState) {
    use cad_domain::StyleId;
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_style(Style {
        id: StyleId(0),
        name: "Standard".into(),
        resource_keys: style_font
            .map(|font| vec![font.to_string()])
            .unwrap_or_default(),
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([31u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.active_layer = LayerId(7);
    (app, session)
}

fn drawing(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn shape_payload(
    name: &str,
    code: u32,
    position: Point3,
    size: f64,
    rotation: f64,
) -> CommandPayload {
    CommandPayload::Shape(Box::new(ShapeCommand {
        name: name.into(),
        code,
        position,
        size,
        rotation,
        font: None,
    }))
}

/// Put one SHAPE row in the drawing and return its id.
fn insert_shape(
    app: &mut Application,
    code: u32,
    position: Point3,
    size: f64,
    rotation: f64,
    font: Option<&str>,
    space: SpaceId,
) -> EntityId {
    let document = app
        .workspace
        .documents
        .get_mut(&DocumentId(1))
        .expect("document");
    let mut candidate = document.drawing.as_ref().clone();
    let id = candidate.allocate_entity_id();
    let row = DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbShape".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: LayerId(7),
        space,
        geometry: SemanticGeometry::Shape {
            shape_name: String::new(),
            code,
            position,
            size,
            rotation,
            font: font.map(str::to_string),
        },
        draw_order: 0,
    };
    let transaction = crate::app_drawing::next_transaction_id();
    let change_set = {
        let mut staged = candidate
            .begin_drawing_transaction("fixture shape", transaction)
            .unwrap();
        staged.insert_entity(row).unwrap();
        staged.commit().unwrap()
    };
    document.drawing = Arc::new(candidate);
    let _ = change_set;
    id
}

fn select(session: &mut SessionState, ids: &[EntityId]) {
    session.selection = SelectionSet::new();
    for id in ids {
        session.selection.insert(SelectionRef {
            document: DocumentId(1),
            entity: *id,
            instance: InstancePath::default(),
            sub_element: None,
        });
    }
}

// ---------------------------------------------------------------------------
// SHAPE
// ---------------------------------------------------------------------------

#[test]
fn shape_creates_the_exact_glyph_entity_in_one_undo_step() {
    let (mut app, mut session) = shape_app(Some("complex.shx"));
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateShape,
        shape_payload("", 65, p3(10.0, 20.0, 0.0), 2.5, 0.0),
    )
    .expect("SHAPE creates one entity");
    assert_eq!(outcome.objects.len(), 1);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let entity = drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .expect("stored shape");
    assert_eq!(entity.object.type_key, "AcDbShape");
    assert_eq!(entity.layer, LayerId(7), "active layer");
    assert_eq!(entity.space, SpaceId::Model);
    match &entity.geometry {
        SemanticGeometry::Shape {
            code,
            position,
            size,
            rotation,
            font,
            ..
        } => {
            assert_eq!(*code, 65);
            assert_eq!(*position, p3(10.0, 20.0, 0.0));
            assert_eq!(*size, 2.5);
            assert_eq!(*rotation, 0.0);
            assert_eq!(font.as_deref(), Some("complex.shx"));
        }
        other => panic!("expected a SHAPE, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn shape_by_name_is_created_honestly_with_code_zero() {
    let (mut app, mut session) = shape_app(Some("complex.shx"));
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateShape,
        shape_payload("ARROW", 0, p3(0.0, 0.0, 0.0), 1.0, 30.0f64.to_radians()),
    )
    .expect("a name-referenced SHAPE is a real row");
    let entity = drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .expect("stored shape");
    match &entity.geometry {
        SemanticGeometry::Shape {
            shape_name,
            code,
            rotation,
            ..
        } => {
            assert_eq!(shape_name, "ARROW");
            assert_eq!(*code, 0);
            assert!((rotation - 30.0f64.to_radians()).abs() < 1e-12);
        }
        other => panic!("expected a SHAPE, got {other:?}"),
    }
}

#[test]
fn shape_refusals_write_nothing() {
    let (mut app, mut session) = shape_app(Some("complex.shx"));
    let cases: Vec<(CommandPayload, &str)> = vec![
        (
            shape_payload("", 0, p3(0.0, 0.0, 0.0), 1.0, 0.0),
            "shape.invalid_code",
        ),
        (
            shape_payload("", 65, p3(f64::NAN, 0.0, 0.0), 1.0, 0.0),
            "shape.invalid_point",
        ),
        (
            shape_payload("", 65, p3(0.0, 0.0, 0.0), 0.0, 0.0),
            "shape.invalid_size",
        ),
        (
            shape_payload("", 65, p3(0.0, 0.0, 0.0), -2.0, 0.0),
            "shape.invalid_size",
        ),
        (
            shape_payload("", 65, p3(0.0, 0.0, 0.0), 1.0, f64::INFINITY),
            "shape.invalid_rotation",
        ),
        (
            shape_payload("bad\nname", 0, p3(0.0, 0.0, 0.0), 1.0, 0.0),
            "shape.invalid_name",
        ),
        (CommandPayload::None, "shape.payload"),
    ];
    for (payload, code) in cases {
        let before = revision(&app);
        let error =
            execute(&mut app, &mut session, CommandId::CreateShape, payload).expect_err("refused");
        assert!(error.to_string().contains(code), "{error}");
        assert_eq!(revision(&app), before, "{code} writes nothing");
        assert!(!app.can_undo(&DocumentId(1)), "{code} records no history");
    }
}

#[test]
fn shape_without_any_font_is_refused_rather_than_inventing_one() {
    let (mut app, mut session) = shape_app(None);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateShape,
        shape_payload("", 65, p3(0.0, 0.0, 0.0), 1.0, 0.0),
    )
    .expect_err("no style font and no explicit key");
    assert!(error.to_string().contains("shape.font_missing"), "{error}");
    assert!(!app.can_undo(&DocumentId(1)));
}

// ---------------------------------------------------------------------------
// SHP2BLK
// ---------------------------------------------------------------------------

#[test]
fn shp2blk_defines_one_outline_block_and_places_every_insert() {
    install_outline_engine();
    if !shape_font_available() {
        // No SHX face in the process-wide hook: the command's honest
        // `shp2blk.font_unavailable` path is the only reachable one.
        eprintln!(
            "shp2blk_defines_one_outline_block_and_places_every_insert: {}",
            crate::tests::font_fixture::missing_face_hint("complex.shx")
        );
        let (mut app, mut session) = shape_app(Some("complex.shx"));
        let id = insert_shape(
            &mut app,
            65,
            p3(10.0, 5.0, 0.0),
            2.0,
            0.0,
            Some("complex.shx"),
            SpaceId::Model,
        );
        select(&mut session, &[id]);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Shp2Blk),
            CommandPayload::None,
        )
        .expect_err("no outline engine");
        assert!(
            error.to_string().contains("shp2blk.font_unavailable"),
            "{error}"
        );
        return;
    }

    let (mut app, mut session) = shape_app(Some("complex.shx"));
    let first = insert_shape(
        &mut app,
        65,
        p3(10.0, 5.0, 0.0),
        2.0,
        0.0,
        Some("complex.shx"),
        SpaceId::Model,
    );
    let second = insert_shape(
        &mut app,
        65,
        p3(40.0, 5.0, 0.0),
        4.0,
        0.0,
        Some("complex.shx"),
        SpaceId::Model,
    );
    select(&mut session, &[first, second]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Shp2Blk),
        CommandPayload::None,
    )
    .expect("SHP2BLK converts both shapes");
    assert_eq!(outcome.objects.len(), 2, "one INSERT per source");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    assert!(
        session.selection.refs().is_empty(),
        "the deleted sources no longer stay selected"
    );

    // Exactly one definition for the shared (font, code) glyph.
    assert_eq!(drawing(&app).blocks().count(), 1);
    let definition = drawing(&app).blocks().next().expect("the defined block");
    assert!(
        !definition.entities.is_empty(),
        "outline polylines are members"
    );
    let base = drawing(&app)
        .block_base_point(definition.id)
        .expect("the generated base point");
    assert_eq!(base, Point3::default());

    // Each INSERT is `T(position) · S(size)` in this fixture (rotation 0), so the
    // origin maps to the source position and a unit x vector scales by `size`.
    for (object, position, size) in [
        (outcome.objects[0], p3(10.0, 5.0, 0.0), 2.0),
        (outcome.objects[1], p3(40.0, 5.0, 0.0), 4.0),
    ] {
        let entity = drawing(&app)
            .entity(EntityId(object.0))
            .expect("stored reference");
        match &entity.geometry {
            SemanticGeometry::Insert { block, transform } => {
                assert_eq!(*block, definition.id);
                assert_eq!(transform.apply_point(Point3::default()), position);
                let x = transform.apply_vector(Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                });
                assert!((x.x.hypot(x.y) - size).abs() < 1e-9);
            }
            other => panic!("expected an INSERT, got {other:?}"),
        }
    }
    // Both sources are gone.
    assert!(drawing(&app).entity(first).is_none());
    assert!(drawing(&app).entity(second).is_none());
}

#[test]
fn shp2blk_generated_names_do_not_overwrite_an_existing_block() {
    install_outline_engine();
    if !shape_font_available() {
        eprintln!(
            "shp2blk_generated_names_do_not_overwrite_an_existing_block: {}",
            crate::tests::font_fixture::missing_face_hint("complex.shx")
        );
        return;
    }
    let (mut app, mut session) = shape_app(Some("complex.shx"));
    // Pre-create the exact stem `SHP2BLK` would generate for complex.shx/65, so
    // the command must take the first free numeric suffix instead of a collision.
    {
        let document = app.workspace.documents.get(&DocumentId(1)).unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let transaction = crate::app_drawing::next_transaction_id();
        {
            let mut staged = candidate
                .begin_drawing_transaction("collision", transaction)
                .unwrap();
            staged
                .stage_create_block(BlockRow {
                    definition: BlockDefinition {
                        id: BlockId(99),
                        entities: Vec::new(),
                        dynamic_visibility: None,
                    },
                    metadata: Some(BlockMetadata {
                        name: "complex_SHAPE65".into(),
                        base_point: Point3::default(),
                    }),
                })
                .unwrap();
            staged.commit().unwrap();
        }
        app.workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = Arc::new(candidate);
    }
    let id = insert_shape(
        &mut app,
        65,
        p3(10.0, 5.0, 0.0),
        2.0,
        0.0,
        Some("complex.shx"),
        SpaceId::Model,
    );
    select(&mut session, &[id]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Shp2Blk),
        CommandPayload::None,
    )
    .expect("a collision gets a suffix, not an overwrite");
    assert_eq!(drawing(&app).blocks().count(), 2, "the old block survived");
    assert!(
        drawing(&app).block_by_name("complex_SHAPE65_1").is_some(),
        "the generated name took the first free suffix"
    );
}

#[test]
fn shp2blk_refusals_write_nothing() {
    let (mut app, mut session) = shape_app(Some("complex.shx"));
    let expect_refusal =
        |app: &mut Application, session: &mut SessionState, payload: CommandPayload, code: &str| {
            let before = revision(app);
            let error = execute(
                app,
                session,
                CommandId::Extended(ExtendedCommandId::Shp2Blk),
                payload,
            )
            .expect_err("refused");
            assert!(error.to_string().contains(code), "{error}");
            assert_eq!(revision(app), before, "{code} writes nothing");
        };

    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "shp2blk.no_selection",
    );

    // A non-shape row is refused by name.
    let line = {
        let document = app.workspace.documents.get_mut(&DocumentId(1)).unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let id = candidate.allocate_entity_id();
        let transaction = crate::app_drawing::next_transaction_id();
        {
            let mut staged = candidate
                .begin_drawing_transaction("fixture line", transaction)
                .unwrap();
            staged
                .insert_entity(DbEntity {
                    object: DbObject {
                        id: ObjectId(id.0),
                        type_key: "AcDbLine".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id,
                    layer: LayerId(7),
                    space: SpaceId::Model,
                    geometry: SemanticGeometry::Line {
                        start: point(0.0, 0.0),
                        end: point(1.0, 0.0),
                    },
                    draw_order: 0,
                })
                .unwrap();
            staged.commit().unwrap();
        }
        document.drawing = Arc::new(candidate);
        id
    };
    select(&mut session, &[line]);
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "shp2blk.not_shape",
    );

    // A name-referenced shape cannot be outlined.
    let named = insert_shape(
        &mut app,
        0,
        p3(0.0, 0.0, 0.0),
        1.0,
        0.0,
        Some("complex.shx"),
        SpaceId::Model,
    );
    select(&mut session, &[named]);
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "shp2blk.shape_by_name",
    );

    // A shape without a font key is refused by name.
    let fontless = insert_shape(
        &mut app,
        65,
        p3(0.0, 0.0, 0.0),
        1.0,
        0.0,
        None,
        SpaceId::Model,
    );
    select(&mut session, &[fontless]);
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "shp2blk.font_missing",
    );

    // A payload is refused before anything else.
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::Mode(AppMode::Work),
        "shp2blk.payload",
    );

    // A block-member shape is refused by name (it cannot become a model INSERT).
    let member = {
        let document = app.workspace.documents.get_mut(&DocumentId(1)).unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let block = candidate.allocate_block_id();
        let id = candidate.allocate_entity_id();
        let transaction = crate::app_drawing::next_transaction_id();
        {
            let mut staged = candidate
                .begin_drawing_transaction("fixture member shape", transaction)
                .unwrap();
            staged
                .stage_create_block(BlockRow {
                    definition: BlockDefinition {
                        id: block,
                        entities: vec![id],
                        dynamic_visibility: None,
                    },
                    metadata: Some(BlockMetadata {
                        name: "MEMBER".into(),
                        base_point: Point3::default(),
                    }),
                })
                .unwrap();
            staged
                .insert_entity(DbEntity {
                    object: DbObject {
                        id: ObjectId(id.0),
                        type_key: "AcDbShape".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id,
                    layer: LayerId(7),
                    space: SpaceId::Block(block),
                    geometry: SemanticGeometry::Shape {
                        shape_name: String::new(),
                        code: 65,
                        position: p3(0.0, 0.0, 0.0),
                        size: 1.0,
                        rotation: 0.0,
                        font: Some("complex.shx".into()),
                    },
                    draw_order: 0,
                })
                .unwrap();
            staged.commit().unwrap();
        }
        document.drawing = Arc::new(candidate);
        id
    };
    select(&mut session, &[member]);
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "shp2blk.not_model_space",
    );
}
