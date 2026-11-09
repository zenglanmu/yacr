//! `SSX` Express-Tool selection-filter contract tests.
//!
//! Pins the typed criteria API end to end against a real database: the exact id
//! set of each criterion, overwrite (one value per criterion), removal, the
//! template `Pick`/bare-`SSX` seed, current-space scoping (nested block members
//! are never selected), the named unmodelled-criterion refusals and the
//! read-only guarantee (no revision bump, no undo entry, no `ChangeSet`).

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType,
    EntityRenderAttributes, Layer, LinetypePattern, Style,
};

const LINE_PLAIN: EntityId = EntityId(10);
const LINE_RED: EntityId = EntityId(11);
const CIRCLE: EntityId = EntityId(12);
const TEXT_STANDARD: EntityId = EntityId(13);
const TEXT_NOTE: EntityId = EntityId(14);
const INSERT: EntityId = EntityId(15);
/// A member of the block definition: never a candidate in model space.
const BLOCK_MEMBER: EntityId = EntityId(20);

fn entity(
    id: EntityId,
    type_key: &str,
    layer: LayerId,
    space: SpaceId,
    geometry: SemanticGeometry,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer,
        space,
        geometry,
        draw_order: id.0 as i64,
    }
}

fn line(start: (f64, f64), end: (f64, f64)) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(start.0, start.1),
        end: point(end.0, end.1),
    }
}

fn text(value: &str, style: StyleId) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: value.into(),
        position: point(0.0, 0.0),
        style,
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
    }
}

fn app_with(extra: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for (id, name) in [(StyleId(0), "Standard"), (StyleId(1), "Note")] {
        builder
            .insert_style(Style {
                id,
                name: name.into(),
                resource_keys: Vec::new(),
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(2),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![BLOCK_MEMBER],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "CHAIR", point(0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            LINE_PLAIN,
            "AcDbLine",
            LayerId(0),
            SpaceId::Model,
            line((0.0, 0.0), (1.0, 0.0)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            LINE_RED,
            "AcDbLine",
            LayerId(7),
            SpaceId::Model,
            line((0.0, 1.0), (1.0, 1.0)),
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            LINE_RED,
            EntityRenderAttributes {
                color: EntityColor::Explicit([255, 0, 0]),
                linetype: EntityLineType::Explicit {
                    name: "DASHED".into(),
                    pattern: LinetypePattern::continuous(),
                    scale: 1.0,
                },
                ..EntityRenderAttributes::default()
            },
        )
        .unwrap();
    builder
        .insert_entity(entity(
            CIRCLE,
            "AcDbCircle",
            LayerId(7),
            SpaceId::Model,
            SemanticGeometry::Circle {
                center: point(5.0, 5.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: 2.0,
            },
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            TEXT_STANDARD,
            "AcDbText",
            LayerId(0),
            SpaceId::Model,
            text("Hello", StyleId(0)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            TEXT_NOTE,
            "AcDbText",
            LayerId(0),
            SpaceId::Model,
            text("World", StyleId(1)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            INSERT,
            "AcDbBlockReference",
            LayerId(0),
            SpaceId::Model,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: cad_domain::Transform3::identity(),
            },
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            EntityId(30),
            "AcDbLine",
            LayerId(0),
            SpaceId::Paper(LayoutId(2)),
            line((0.0, 0.0), (2.0, 0.0)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            BLOCK_MEMBER,
            "AcDbLine",
            LayerId(0),
            SpaceId::Block(BlockId(1)),
            line((0.0, 0.0), (1.0, 0.0)),
        ))
        .unwrap();
    for entity in extra {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([3u8; 32]),
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

fn model_ref(entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn ids(session: &SessionState) -> Vec<u128> {
    let mut ids: Vec<u128> = session
        .selection
        .refs()
        .iter()
        .map(|reference| reference.entity.0)
        .collect();
    ids.sort_unstable();
    ids
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    request: SsxRequest,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::SelectMatchingProperties,
            CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Ssx(request))),
        ),
    )
}

fn set(criterion: SsxCriterion) -> SsxRequest {
    SsxRequest::Set(criterion)
}

#[test]
fn ssx_each_criterion_selects_its_exact_id_set() {
    for (request, expected) in [
        (set(SsxCriterion::Block("CHAIR".into())), vec![15]),
        (set(SsxCriterion::Entity("CIRCLE".into())), vec![12]),
        (set(SsxCriterion::Entity("LINE".into())), vec![10, 11]),
        (set(SsxCriterion::Entity("TEXT".into())), vec![13, 14]),
        (set(SsxCriterion::Layer("WALLS".into())), vec![11, 12]),
        (set(SsxCriterion::Color("aci:1".into())), vec![11]),
        // A bare ACI number is the same criterion value.
        (set(SsxCriterion::Color("1".into())), vec![11]),
        (set(SsxCriterion::LType("DASHED".into())), vec![11]),
        (
            set(SsxCriterion::LType("ByLayer".into())),
            vec![10, 12, 13, 14, 15],
        ),
        (set(SsxCriterion::Style("Standard".into())), vec![13]),
        (set(SsxCriterion::Style("Note".into())), vec![14]),
    ] {
        // A fresh session per criterion: the filter accumulates, and the
        // combination/overwrite rules are pinned by their own test.
        let (mut app, mut session) = app_with(Vec::new());
        let label = format!("{request:?}");
        let outcome = run(&mut app, &mut session, request).expect(&label);
        assert_eq!(ids(&session), expected, "{label}");
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ssx.result"),
            "{label}"
        );
    }
}

#[test]
fn ssx_overwrites_a_criterion_and_clear_removes_it() {
    let (mut app, mut session) = app_with(Vec::new());

    run(&mut app, &mut session, set(SsxCriterion::Layer("0".into()))).unwrap();
    assert_eq!(ids(&session), vec![10, 13, 14, 15]);

    // Adding an existing criterion overwrites it (one value per criterion).
    run(
        &mut app,
        &mut session,
        set(SsxCriterion::Layer("WALLS".into())),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![11, 12]);

    // Two criteria combine (AND).
    run(
        &mut app,
        &mut session,
        set(SsxCriterion::Entity("LINE".into())),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![11]);

    // Clearing the layer leaves only the entity criterion active.
    let outcome = run(
        &mut app,
        &mut session,
        SsxRequest::Clear(SsxCriterionKind::Layer),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![10, 11]);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "ssx.cleared"));

    // Clearing a criterion that is not set is a reported no-op, not a failure.
    let outcome = run(
        &mut app,
        &mut session,
        SsxRequest::Clear(SsxCriterionKind::Layer),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![10, 11]);
    assert!(outcome.diagnostics.iter().any(
        |diagnostic| diagnostic.code == "ssx.cleared" && diagnostic.message.contains("not set")
    ));

    // An empty filter is refused and leaves the selection untouched.
    let before = ids(&session);
    let error = run(
        &mut app,
        &mut session,
        SsxRequest::Clear(SsxCriterionKind::Entity),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("ssx.empty_filter"))
    );
    assert_eq!(ids(&session), before);
}

#[test]
fn ssx_pick_and_bare_ssx_seed_from_the_template_object() {
    let (mut app, mut session) = app_with(Vec::new());

    // The first selected object is the template; the five modelled properties
    // are seeded and applied.
    session.selection = SelectionSet::from_refs(vec![model_ref(LINE_RED)]);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::SelectMatchingProperties, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(ids(&session), vec![11]);
    let echo = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "ssx.criteria")
        .expect("criteria echo");
    assert!(echo.message.contains("layer=WALLS"), "{}", echo.message);
    assert!(echo.message.contains("color=aci:1"), "{}", echo.message);
    assert!(echo.message.contains("ltype=DASHED"), "{}", echo.message);

    // `SSX PICK` is the explicit form of the same seed.
    session.selection = SelectionSet::from_refs(vec![model_ref(CIRCLE)]);
    run(&mut app, &mut session, SsxRequest::Pick).unwrap();
    assert_eq!(ids(&session), vec![12]);

    // An empty selection cannot seed anything.
    session.selection.clear();
    let error = run(&mut app, &mut session, SsxRequest::Pick).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("ssx.selection_required"))
    );

    // A block-instance member has no single entity row to read as a template.
    let mut reference = model_ref(LINE_RED);
    reference.instance = InstancePath(vec![INSERT]);
    session.selection = SelectionSet::from_refs(vec![reference]);
    let error = run(&mut app, &mut session, SsxRequest::Pick).unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref reason) if reason.starts_with("ssx.template_not_addressable"))
    );

    // A template ref whose entity no longer exists.
    session.selection = SelectionSet::from_refs(vec![model_ref(EntityId(999))]);
    let error = run(&mut app, &mut session, SsxRequest::Pick).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("ssx.template_missing"))
    );

    // The template must be in the current space (the paper line is not).
    session.active_space = SpaceId::Model;
    session.selection = SelectionSet::from_refs(vec![model_ref(EntityId(30))]);
    let error = run(&mut app, &mut session, SsxRequest::Pick).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("ssx.template_space"))
    );
}

#[test]
fn ssx_is_current_space_only_and_never_selects_nested_block_members() {
    let (mut app, mut session) = app_with(Vec::new());

    // Model space: the block member (id 20) and the paper-space line (id 30)
    // are never candidates.
    run(
        &mut app,
        &mut session,
        set(SsxCriterion::Entity("LINE".into())),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![10, 11]);

    // The active paper layout is queried instead of model space.
    session.active_space = SpaceId::Paper(LayoutId(2));
    run(
        &mut app,
        &mut session,
        set(SsxCriterion::Entity("LINE".into())),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![30]);

    // Block space is a named refusal.
    session.active_space = SpaceId::Block(BlockId(1));
    let error = run(
        &mut app,
        &mut session,
        set(SsxCriterion::Entity("LINE".into())),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref reason) if reason.starts_with("ssx.block_space"))
    );
}

#[test]
fn ssx_refuses_unmodelled_criteria_by_name() {
    for (request, code) in [
        (
            set(SsxCriterion::Thickness(2.5)),
            "ssx.thickness_not_modelled",
        ),
        (
            set(SsxCriterion::Vector(Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            })),
            "ssx.vector_not_modelled",
        ),
        (set(SsxCriterion::Flag), "ssx.flag_not_modelled"),
    ] {
        let (mut app, mut session) = app_with(Vec::new());
        let before = revision(&app);
        let error = run(&mut app, &mut session, request).unwrap_err();
        assert!(
            matches!(error, CadError::Unsupported(ref reason) if reason.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert!(session.selection.is_empty());
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
    }

    // The criterion can be cleared again, leaving an empty filter (refused).
    let (mut app, mut session) = app_with(Vec::new());
    run(&mut app, &mut session, set(SsxCriterion::Flag)).unwrap_err();
    let error = run(
        &mut app,
        &mut session,
        SsxRequest::Clear(SsxCriterionKind::Flag),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("ssx.empty_filter"))
    );
}

#[test]
fn ssx_is_read_only() {
    let (mut app, mut session) = app_with(Vec::new());
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let outcome = run(
        &mut app,
        &mut session,
        set(SsxCriterion::Entity("CIRCLE".into())),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_undo);
    assert_eq!(ids(&session), vec![12]);
}
