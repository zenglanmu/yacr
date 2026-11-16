//! Wave 9 command-line alias contracts: `-BLOCK`, `-INSERT`, `-BOUNDARY`,
//! `-HATCH`, `-HATCHEDIT`, `EXOFFSET`.
//!
//! An alias is not a second implementation. These tests prove each alias is
//! Work-only and answers a malformed request with exactly the owning handler's
//! answer, so there is one planner per feature.

use super::*;

const ALIASES: [(ExtendedCommandId, CommandId); 6] = [
    (ExtendedCommandId::DashBlock, CommandId::DefineBlock),
    (ExtendedCommandId::DashInsert, CommandId::InsertBlock),
    (ExtendedCommandId::DashBoundary, CommandId::Boundary),
    (ExtendedCommandId::DashHatch, CommandId::Hatch),
    (ExtendedCommandId::DashHatchEdit, CommandId::HatchEdit),
    (ExtendedCommandId::ExOffset, CommandId::OffsetEntity),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave9_alias_is_work_only() {
    for (ext, _) in ALIASES {
        assert!(ext.requires_work_mode(), "{ext:?} writes through its owner");
    }
}

#[test]
fn aliases_answer_exactly_like_the_owning_command_for_a_missing_payload() {
    for (ext, parent) in ALIASES {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let alias_error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a missing payload is refused");
        let parent_error = app
            .execute(&mut session, command(parent, CommandPayload::None))
            .expect_err("the parent refuses the same payload");
        assert_eq!(
            format!("{alias_error:?}"),
            format!("{parent_error:?}"),
            "{ext:?} must not invent its own validation"
        );
        assert_eq!(revision(&app), before, "{ext:?} wrote nothing");
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{ext:?} recorded no undo step"
        );
    }
}

#[test]
fn aliases_are_refused_in_viewer_mode_before_any_write() {
    for (ext, _) in ALIASES {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("Viewer cannot run a write alias");
        assert!(matches!(error, CadError::PermissionDenied));
    }
}

// ---------------------------------------------------------------------------
// EXOFFSET: the Express name is the OFFSET planner, so it inherits its whole
// target set (`LINE`/`CIRCLE`/`ARC`/`LWPOLYLINE`/`ELLIPSE`/`SPLINE`, the
// ellipse→spline and spline→spline approximations), its refusals and its write
// discipline (one transaction / one undo step, including `Multiple`).
// ---------------------------------------------------------------------------

use crate::command_payload::{OffsetGapType, OffsetLayer, OffsetMode, OffsetOptions};
use cad_db::{DbEntity, DbObject};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn exoffset_ref(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A one-layer model-space document holding exactly `entities`.
fn exoffset_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (index, (id, geometry)) in entities.into_iter().enumerate() {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbEntity".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    let drawing = builder.finish().unwrap();
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

/// `EXOFFSET` with a fixed distance and index 1 (the Express name inherits the
/// `OFFSET` option payload).
fn exoffset(
    app: &mut Application,
    session: &mut SessionState,
    refs: &[u128],
    side_points: Vec<Point3>,
    distance: f64,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::ExOffset),
        CommandPayload::OffsetOptions(Box::new(OffsetOptions {
            refs: refs.iter().map(|id| exoffset_ref(*id)).collect(),
            side_points,
            mode: OffsetMode::Distance(distance),
            erase_source: false,
            layer: OffsetLayer::Source,
            gap: OffsetGapType::Sharp,
        })),
    )
}

fn created_geometry(app: &Application, outcome: &CommandOutcome) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the offset exists")
        .geometry
        .clone()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn exoffset_offsets_the_analytic_offset_kinds() {
    // LINE: pick above the source -> parallel line one unit up.
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(5.0, 3.0)], 1.0).unwrap();
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    let SemanticGeometry::Line { start, end } = created_geometry(&app, &outcome) else {
        panic!("EXOFFSET must inherit the LINE offset");
    };
    assert!((start.y - 1.0).abs() < 1e-9 && (end.y - 1.0).abs() < 1e-9);

    // CIRCLE: pick outside -> new radius `r + d`.
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(6.0, 0.0)], 1.0).unwrap();
    let SemanticGeometry::Circle { radius, .. } = created_geometry(&app, &outcome) else {
        panic!("EXOFFSET must inherit the CIRCLE offset");
    };
    assert!((radius - 6.0).abs() < 1e-9, "radius {radius}");
    assert_eq!(undo_depth(&app), 1);

    // ARC: the same radial model, on the arc's own circle.
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(6.0, 0.0)], 1.0).unwrap();
    let SemanticGeometry::Arc { radius, .. } = created_geometry(&app, &outcome) else {
        panic!("EXOFFSET must inherit the ARC offset");
    };
    assert!((radius - 6.0).abs() < 1e-9, "radius {radius}");
    assert_eq!(undo_depth(&app), 1);

    // Through: the legacy `Offset` payload (no fixed distance) is the same
    // handler, so the pick itself defines the distance.
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
    )]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ExOffset),
        CommandPayload::Offset {
            refs: vec![exoffset_ref(1)],
            side_point: point(5.0, 3.0),
        },
    )
    .expect("EXOFFSET inherits the Through arm");
    let SemanticGeometry::Line { start, .. } = created_geometry(&app, &outcome) else {
        panic!("expected a LINE");
    };
    assert!((start.y - 3.0).abs() < 1e-9, "through the pick: {start:?}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn exoffset_offsets_a_polyline_like_offset() {
    // A closed 4x4 square, picked inside: the inward offset is the 2x2 square
    // from (1,1) to (3,3), with the same closed flag and no new entity kind.
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(4.0, 0.0),
                point(4.0, 4.0),
                point(0.0, 4.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
            width: 0.0,
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(2.0, 2.0)], 1.0).unwrap();
    let SemanticGeometry::Polyline { points, closed, .. } = created_geometry(&app, &outcome) else {
        panic!("EXOFFSET must inherit the LWPOLYLINE offset");
    };
    assert!(closed, "a closed source stays closed");
    assert_eq!(points.len(), 4);
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for vertex in &points {
        min_x = min_x.min(vertex.x);
        max_x = max_x.max(vertex.x);
        min_y = min_y.min(vertex.y);
        max_y = max_y.max(vertex.y);
    }
    assert!(
        (min_x - 1.0).abs() < 1e-9
            && (max_x - 3.0).abs() < 1e-9
            && (min_y - 1.0).abs() < 1e-9
            && (max_y - 3.0).abs() < 1e-9,
        "the inward offset is the 2x2 square: {points:?}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn exoffset_approximates_an_ellipse_and_a_spline_as_splines() {
    // ELLIPSE -> SPLINE (the offset of an ellipse is not an ellipse).
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(0.0, 8.0)], 1.0).unwrap();
    assert!(
        matches!(
            created_geometry(&app, &outcome),
            SemanticGeometry::Spline { .. }
        ),
        "the ellipse offset must stay the spline approximation"
    );
    assert_eq!(undo_depth(&app), 1);

    // SPLINE -> SPLINE (the sampled offset curve).
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Spline {
            degree: 3,
            knots: Vec::new(),
            control_points: vec![
                p3(0.0, 0.0, 0.0),
                p3(2.0, 2.0, 0.0),
                p3(4.0, 0.0, 0.0),
                p3(6.0, 2.0, 0.0),
            ],
            weights: Vec::new(),
            closed: false,
        },
    )]);
    let outcome = exoffset(&mut app, &mut session, &[1], vec![point(3.0, 10.0)], 1.0).unwrap();
    assert!(
        matches!(
            created_geometry(&app, &outcome),
            SemanticGeometry::Spline { .. }
        ),
        "a spline source offsets to a spline"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn exoffset_multiple_is_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = exoffset_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
    )]);
    let outcome = exoffset(
        &mut app,
        &mut session,
        &[1],
        vec![point(5.0, 3.0), point(5.0, -3.0)],
        2.0,
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2, "one offset per side point");
    assert_eq!(undo_depth(&app), 1, "Multiple is still one undo step");

    let mut ys: Vec<f64> = outcome
        .objects
        .iter()
        .map(|object| {
            let SemanticGeometry::Line { start, .. } = app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(object.0))
                .expect("offset exists")
                .geometry
                .clone()
            else {
                panic!("expected a LINE");
            };
            start.y
        })
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (ys[0] + 2.0).abs() < 1e-9 && (ys[1] - 2.0).abs() < 1e-9,
        "both sides: {ys:?}"
    );

    let mut undo = SessionState::new(DocumentId(1), AppMode::Work);
    execute(&mut app, &mut undo, CommandId::Undo, CommandPayload::None).expect("undo");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "one undo removes both offsets"
    );
}

#[test]
fn exoffset_refuses_an_unsupported_target_without_writing() {
    // OFFSET has no HATCH arm, and EXOFFSET inherits that refusal verbatim (one
    // planner, so the message and the no-write behaviour are the same).
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            vec![0.0; 4],
            HatchLoopFlags::default(),
        )],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch");
    let (mut app, mut session) = exoffset_app(vec![(1, SemanticGeometry::Hatch(Box::new(hatch)))]);
    let before = revision(&app);
    let error = exoffset(&mut app, &mut session, &[1], vec![point(5.0, 2.0)], 1.0).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("OFFSET supports LINE"),
        "the owning planner's own reason: {error:?}"
    );
    assert_eq!(revision(&app), before, "nothing was written");
    assert!(!app.can_undo(&DocumentId(1)), "no undo step was recorded");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1
    );
}
