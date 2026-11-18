//! `CHSPACE` contract tests: moving objects between model space and the active
//! layout's paper space.
//!
//! The command must rewrite each selected entity's geometry **and** space in one
//! transaction / one undo step, using the active viewport's resolved transform
//! (`to_paper` for model → paper, `to_model` for paper → model). Every refusal is
//! asserted to write nothing: revision, undo depth, geometry and space are read
//! before and after.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

const DOC: DocumentId = DocumentId(1);

fn chspace() -> Command {
    command(
        CommandId::Extended(ExtendedCommandId::ChSpace),
        CommandPayload::None,
    )
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn rect_viewport(scale: f64) -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(scale),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

fn entity(
    id: u128,
    space: SpaceId,
    type_key: &str,
    geometry: SemanticGeometry,
) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn line(id: u128, space: SpaceId) -> cad_db::DbEntity {
    entity(
        id,
        space,
        "AcDbLine",
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(100.0, 0.0),
        },
    )
}

/// One linear DIMENSION entity (the build's dimensions are non-associative).
fn dimension(id: u128, space: SpaceId) -> cad_db::DbEntity {
    let style = cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    );
    let geometry = DimensionGeometry::synthesized(
        DimensionDefinition {
            kind: DimensionKind::Linear { rotation: 0.0 },
            points: DimensionPoints::Linear {
                first: point(0.0, 0.0),
                second: point(10.0, 0.0),
                dimension_line: point(5.0, 5.0),
            },
            text_middle: point(5.0, 5.0),
            text_rotation: 0.0,
            normal: p3(0.0, 0.0, 1.0),
        },
        "Standard".to_string(),
        style,
        None,
    )
    .expect("the fixture dimension synthesizes");
    entity(
        id,
        space,
        "AcDbDimension",
        SemanticGeometry::Dimension(Box::new(geometry)),
    )
}

/// The fixture: layout 1 (`Sheet1`, one 1:100 viewport), layout 2 (`Broken`, a
/// `Partial` viewport), layout 3 (`Empty`, no viewport) and the given entities.
fn fixture(entities: Vec<cad_db::DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: vec![rect_viewport(100.0)],
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(2),
            name: "Broken".into(),
            viewports: vec![PaperViewport {
                completeness: Completeness::Partial(
                    vec!["importer dropped the view height".into()],
                ),
                ..rect_viewport(100.0)
            }],
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(3),
            name: "Empty".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    let mut session = SessionState::new(DOC, AppMode::Work);
    session.active_space = SpaceId::Paper(LayoutId(1));
    (app, session)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection.replace(ids.iter().map(|id| SelectionRef {
        document: DOC,
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn refusal(result: CadResult<CommandOutcome>) -> String {
    match result {
        Err(CadError::Unsupported(message)) | Err(CadError::InvalidInput(message)) => message,
        Err(other) => panic!("expected a named refusal, got {other:?}"),
        Ok(outcome) => panic!("expected a refusal, got {outcome:?}"),
    }
}

fn stored(app: &Application, id: u128) -> (SpaceId, SemanticGeometry) {
    let entity = drawing(app)
        .entity(EntityId(id))
        .expect("the entity exists");
    (entity.space.clone(), entity.geometry.clone())
}

#[test]
fn chspace_moves_model_objects_into_paper_with_the_viewport_transform() {
    let (mut app, mut session) = fixture(vec![line(10, SpaceId::Model)]);
    select(&mut session, &[10]);
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = app
        .execute(&mut session, chspace())
        .expect("CHSPACE moves the model line to paper space");
    assert_eq!(codes(&outcome), vec!["chspace.moved"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");

    let (space, geometry) = stored(&app, 10);
    assert_eq!(space, SpaceId::Paper(LayoutId(1)));
    let SemanticGeometry::Line { start, end } = geometry else {
        panic!("the entity stays a LINE");
    };
    // 1:100 means one paper unit is 100 model units: `to_paper` scales by 0.01.
    assert!((start.x - 0.0).abs() < 1e-12);
    assert!(
        (end.x - 1.0).abs() < 1e-12,
        "100 model units = 1 paper unit"
    );

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo replays the migration");
    let (space, geometry) = stored(&app, 10);
    assert_eq!(space, SpaceId::Model, "undo restores the space");
    let SemanticGeometry::Line { end, .. } = geometry else {
        panic!("the entity stays a LINE");
    };
    assert_eq!(end.x, 100.0, "undo restores the model coordinates");
}

#[test]
fn chspace_moves_paper_objects_into_model_with_the_inverse_transform() {
    let (mut app, mut session) = fixture(vec![line(11, SpaceId::Paper(LayoutId(1)))]);
    select(&mut session, &[11]);
    let before = revision(&app);

    let outcome = app
        .execute(&mut session, chspace())
        .expect("CHSPACE moves the paper line to model space");
    assert_eq!(codes(&outcome), vec!["chspace.moved"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));

    let (space, geometry) = stored(&app, 11);
    assert_eq!(space, SpaceId::Model);
    let SemanticGeometry::Line { end, .. } = geometry else {
        panic!("the entity stays a LINE");
    };
    assert!(
        (end.x - 10_000.0).abs() < 1e-9,
        "1 paper unit = 100 model units"
    );
}

#[test]
fn chspace_refusals_write_nothing() {
    let mut entities = vec![
        line(10, SpaceId::Model),
        line(11, SpaceId::Paper(LayoutId(1))),
        line(12, SpaceId::Paper(LayoutId(2))),
        entity(
            13,
            SpaceId::Paper(LayoutId(1)),
            "AcDbViewport",
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 1.0),
            },
        ),
        dimension(14, SpaceId::Paper(LayoutId(1))),
    ];
    // A block-instance member reference is not directly addressable.
    entities.push(line(15, SpaceId::Paper(LayoutId(1))));
    let (mut app, mut session) = fixture(entities);
    let before = revision(&app);

    // Empty selection.
    let empty = refusal(app.execute(&mut session, chspace()));
    assert!(empty.contains("chspace.needs_selection"), "{empty}");

    // Model space is not a layout, so there is no viewport to transform through.
    session.active_space = SpaceId::Model;
    select(&mut session, &[11]);
    let model = refusal(app.execute(&mut session, chspace()));
    assert!(model.contains("chspace.paper_space_required"), "{model}");
    session.active_space = SpaceId::Paper(LayoutId(1));

    // A selection in another layout is not addressable here.
    select(&mut session, &[12]);
    let other_layout = refusal(app.execute(&mut session, chspace()));
    assert!(
        other_layout.contains("chspace.not_addressable"),
        "{other_layout}"
    );

    // A mixed selection cannot be one transaction in one direction.
    select(&mut session, &[10, 11]);
    let mixed = refusal(app.execute(&mut session, chspace()));
    assert!(mixed.contains("chspace.mixed_space"), "{mixed}");

    // An AcDbViewport frame belongs to its layout row, not to the entity table.
    select(&mut session, &[13]);
    let viewport = refusal(app.execute(&mut session, chspace()));
    assert!(
        viewport.contains("chspace.viewport_not_movable"),
        "{viewport}"
    );

    // Dimensions carry no association model, so the move cannot be proven safe.
    select(&mut session, &[14]);
    let dimension = refusal(app.execute(&mut session, chspace()));
    assert!(
        dimension.contains("chspace.dimension_not_associative"),
        "{dimension}"
    );

    // A block-instance member is refused, never silently reinterpreted.
    session.selection.replace(vec![SelectionRef {
        document: DOC,
        entity: EntityId(15),
        instance: InstancePath(vec![EntityId(99)]),
        sub_element: None,
    }]);
    let member = refusal(app.execute(&mut session, chspace()));
    assert!(member.contains("chspace.not_addressable"), "{member}");

    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), 0, "a refusal records no history");
    for (id, expected) in [(10u128, SpaceId::Model), (11, SpaceId::Paper(LayoutId(1)))] {
        assert_eq!(stored(&app, id).0, expected, "entity {id} kept its space");
    }
}

#[test]
fn chspace_refuses_a_layout_without_a_usable_viewport() {
    let (mut app, mut session) = fixture(vec![line(10, SpaceId::Model)]);
    select(&mut session, &[10]);
    let before = revision(&app);

    session.active_space = SpaceId::Paper(LayoutId(3));
    let no_viewport = refusal(app.execute(&mut session, chspace()));
    assert!(no_viewport.contains("chspace.no_viewport"), "{no_viewport}");

    session.active_space = SpaceId::Paper(LayoutId(2));
    let unsupported = refusal(app.execute(&mut session, chspace()));
    assert!(
        unsupported.contains("chspace.unsupported_viewport"),
        "{unsupported}"
    );

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn chspace_is_work_only() {
    let (mut app, mut session) = fixture(vec![line(10, SpaceId::Model)]);
    select(&mut session, &[10]);
    let before = revision(&app);
    session.switch_mode(AppMode::Viewer).unwrap();
    let viewer = app
        .execute(&mut session, chspace())
        .expect_err("a Viewer session cannot move entities");
    assert_eq!(viewer, CadError::PermissionDenied);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DOC));
}
