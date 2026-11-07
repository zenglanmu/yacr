//! `PEDIT` v1 (AutoCAD 2026) contract tests: the declared `CLOSE` / `OPEN` /
//! `REVERSE` option subset.
//!
//! Pins the selection rule (exactly one directly addressable model-space
//! LWPOLYLINE), the exact writes (only `closed` or the REVERSE geometry change;
//! points/bulges/layer/space/draw order preserved otherwise), the named no-ops
//! (`pedit.already_closed` / `pedit.already_open`) and every refusal: none of
//! them bumps the revision or records an undo entry. A real change is exactly one
//! transaction / one undo step.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_geometry::{arbitrary_axis, bulge_arc_geometry};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(7),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
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
    // The paper-space refusal fixture needs a real layout row to attach to.
    builder
        .insert_layout(cad_db::Layout {
            id: LayoutId(2),
            name: "Paper".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, entities: &[u128]) {
    session
        .selection
        .replace(entities.iter().map(|id| ref_of(*id)).collect::<Vec<_>>());
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

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .expect("entity must exist")
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    entity(app, id).geometry
}

fn pedit(
    app: &mut Application,
    session: &mut SessionState,
    option: PeditOption,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Pedit, CommandPayload::PeditOption(option)),
    )
}

/// Three vertices, two bulged segments; the per-vertex storage shape includes
/// the trailing bulge that describes no segment.
fn open_bulged() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
        bulges: vec![0.75, -0.5, 0.25],
        closed: false,
    }
}

fn closed_bulged() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(4.0, 0.0, 0.0),
            p3(4.0, 4.0, 0.0),
            p3(0.0, 4.0, 0.0),
        ],
        bulges: vec![0.5, -0.25, 0.75, -0.125],
        closed: true,
    }
}

fn open_two_vertices() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)],
        bulges: vec![0.0, 0.0],
        closed: false,
    }
}

fn line() -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(0.0, 0.0, 0.0),
        end: p3(10.0, 0.0, 0.0),
    }
}

/// The arc midpoint of every real segment of a bulged polyline.
fn segment_midpoints(geometry: &SemanticGeometry) -> Vec<Point3> {
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = geometry
    else {
        panic!("expected a polyline");
    };
    let n = points.len();
    let segments = if *closed { n } else { n - 1 };
    (0..segments)
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % n];
            let arc = bulge_arc_geometry(a, b, bulges[i], p3(0.0, 0.0, 1.0))
                .expect("bulged segment must produce an arc");
            let SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } = arc
            else {
                panic!("expected an arc");
            };
            let (ax, ay, _) = arbitrary_axis(normal);
            let angle = start + sweep * 0.5;
            Point3 {
                x: center.x + ax.x * radius * angle.cos() + ay.x * radius * angle.sin(),
                y: center.y + ax.y * radius * angle.cos() + ay.y * radius * angle.sin(),
                z: center.z + ax.z * radius * angle.cos() + ay.z * radius * angle.sin(),
            }
        })
        .collect()
}

fn assert_point_close(actual: Point3, expected: Point3, context: &str) {
    let drift = ((actual.x - expected.x).powi(2)
        + (actual.y - expected.y).powi(2)
        + (actual.z - expected.z).powi(2))
    .sqrt();
    assert!(drift <= 1e-9, "{context}: {actual:?} vs {expected:?}");
}

fn assert_invalid(error: CadError, code: &str) {
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with(code),
            "expected refusal starting with {code}, got {message}"
        ),
        other => panic!("expected InvalidInput {code}, got {other:?}"),
    }
}

fn assert_unsupported(error: CadError, code: &str) {
    match error {
        CadError::Unsupported(message) => assert!(
            message.starts_with(code),
            "expected Unsupported starting with {code}, got {message}"
        ),
        other => panic!("expected Unsupported {code}, got {other:?}"),
    }
}

/// Every refusal must leave the drawing and its history untouched.
fn assert_wrote_nothing(app: &Application, before_revision: Revision) {
    assert_eq!(
        revision(app),
        before_revision,
        "a refusal bumped the revision"
    );
    assert_eq!(undo_depth(app), 0, "a refusal recorded an undo entry");
}

// --- CLOSE -------------------------------------------------------------------

#[test]
fn close_flips_an_open_polyline_in_one_undo_and_preserves_everything_else() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 42)], AppMode::Work);
    select(&mut session, &[1]);
    let before = entity(&app, 1);
    let rev = revision(&app);

    let outcome = pedit(&mut app, &mut session, PeditOption::Close).expect("CLOSE must succeed");
    assert_eq!(outcome.diagnostics[0].code, "pedit.applied");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);

    let after = entity(&app, 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &after.geometry
    else {
        panic!("the entity must stay a polyline");
    };
    assert!(*closed, "CLOSE must set closed = true");
    assert_eq!(
        points,
        &match &before.geometry {
            SemanticGeometry::Polyline { points, .. } => points.clone(),
            _ => unreachable!(),
        }
    );
    assert_eq!(bulges, &vec![0.75, -0.5, 0.25]);
    // The entity keeps its id, layer, space and draw order.
    assert_eq!(after.id, before.id);
    assert_eq!(after.layer, before.layer);
    assert_eq!(after.space, before.space);
    assert_eq!(after.draw_order, before.draw_order);
    assert_ne!(revision(&app), rev, "CLOSE must bump the revision");
    assert_eq!(undo_depth(&app), 1, "CLOSE is one undo step");

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(
        geometry(&app, 1),
        before.geometry,
        "UNDO restores the open ring"
    );
}

#[test]
fn close_on_an_already_closed_polyline_is_a_named_noop() {
    let (mut app, mut session) = build(vec![ent(1, closed_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let outcome =
        pedit(&mut app, &mut session, PeditOption::Close).expect("already closed is not an error");
    assert_eq!(outcome.diagnostics[0].code, "pedit.already_closed");
    assert!(outcome.changes.is_none(), "a no-op publishes no ChangeSet");
    assert!(outcome.objects.is_empty());
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0, "a no-op opens no transaction");
}

#[test]
fn close_refuses_an_open_polyline_with_fewer_than_three_vertices() {
    let (mut app, mut session) = build(vec![ent(1, open_two_vertices(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = pedit(&mut app, &mut session, PeditOption::Close).unwrap_err();
    assert_invalid(error, "pedit.too_few_vertices");
    assert_wrote_nothing(&app, rev);
    assert_eq!(geometry(&app, 1), open_two_vertices());
}

// --- OPEN --------------------------------------------------------------------

#[test]
fn open_flips_a_closed_polyline_in_one_undo_and_preserves_everything_else() {
    let (mut app, mut session) = build(vec![ent(1, closed_bulged(), 7)], AppMode::Work);
    select(&mut session, &[1]);
    let before = entity(&app, 1);
    let rev = revision(&app);

    pedit(&mut app, &mut session, PeditOption::Open).expect("OPEN must succeed");
    let after = entity(&app, 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &after.geometry
    else {
        panic!("the entity must stay a polyline");
    };
    assert!(!*closed, "OPEN must set closed = false");
    assert_eq!(points.len(), 4);
    assert_eq!(bulges, &vec![0.5, -0.25, 0.75, -0.125]);
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(after.space, SpaceId::Model);
    assert_eq!(after.draw_order, 7);
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(
        geometry(&app, 1),
        before.geometry,
        "UNDO restores the closed ring"
    );
}

#[test]
fn open_on_an_already_open_polyline_is_a_named_noop() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let outcome =
        pedit(&mut app, &mut session, PeditOption::Open).expect("already open is not an error");
    assert_eq!(outcome.diagnostics[0].code, "pedit.already_open");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}

// --- REVERSE -----------------------------------------------------------------

#[test]
fn reverse_reuses_the_reverse_rule_on_an_open_bulged_polyline() {
    let fixture = open_bulged();
    let (mut app, mut session) = build(vec![ent(1, fixture.clone(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let before_midpoints = segment_midpoints(&fixture);
    let rev = revision(&app);

    pedit(&mut app, &mut session, PeditOption::Reverse).expect("REVERSE must succeed");
    let reversed = geometry(&app, 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &reversed
    else {
        panic!("the entity must stay a polyline");
    };
    assert!(!*closed, "REVERSE keeps an open polyline open");
    assert_eq!(
        points,
        &vec![p3(4.0, 4.0, 0.0), p3(4.0, 0.0, 0.0), p3(0.0, 0.0, 0.0)]
    );
    // Open per-vertex bulges: b'[i] = -b[segments - 1 - i], and the trailing
    // entry (no real segment) is carried through unchanged — the exact REVERSE
    // rule, not a second implementation.
    assert_eq!(bulges, &vec![0.5, -0.75, 0.25]);

    // Every segment keeps its arc midpoint (the same arc, traversed backwards).
    let after_midpoints = segment_midpoints(&reversed);
    for (i, midpoint) in after_midpoints.iter().enumerate() {
        assert_point_close(
            *midpoint,
            before_midpoints[before_midpoints.len() - 1 - i],
            &format!("open segment {i}"),
        );
    }
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1, "REVERSE is one undo step");
}

#[test]
fn reverse_supports_a_closed_bulged_polyline() {
    let fixture = closed_bulged();
    let (mut app, mut session) = build(vec![ent(1, fixture.clone(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let before_midpoints = segment_midpoints(&fixture);

    pedit(&mut app, &mut session, PeditOption::Reverse).expect("REVERSE must succeed");
    let reversed = geometry(&app, 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &reversed
    else {
        panic!("the entity must stay a polyline");
    };
    assert!(*closed, "REVERSE keeps a closed polyline closed");
    assert_eq!(
        points,
        &vec![
            p3(0.0, 4.0, 0.0),
            p3(4.0, 4.0, 0.0),
            p3(4.0, 0.0, 0.0),
            p3(0.0, 0.0, 0.0),
        ]
    );
    // Closed: b'[i] = -b[(n - 2 - i) mod n].
    assert_eq!(bulges, &vec![-0.75, 0.25, -0.5, 0.125]);

    let after_midpoints = segment_midpoints(&reversed);
    let n = 4;
    for (i, midpoint) in after_midpoints.iter().enumerate() {
        let source = (n + n - 2 - i) % n;
        assert_point_close(
            *midpoint,
            before_midpoints[source],
            &format!("closed segment {i}"),
        );
    }
    assert_eq!(undo_depth(&app), 1);
}

// --- refusals ----------------------------------------------------------------

#[test]
fn bare_pedit_is_refused_with_the_implemented_option_list() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = app
        .execute(
            &mut session,
            command(CommandId::Pedit, CommandPayload::None),
        )
        .unwrap_err();
    assert_invalid(error, "pedit.needs_option");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn empty_or_multiple_selection_is_refused_and_writes_nothing() {
    let (mut app, mut session) = build(
        vec![ent(1, open_bulged(), 0), ent(2, closed_bulged(), 1)],
        AppMode::Work,
    );
    let rev = revision(&app);

    let error = pedit(&mut app, &mut session, PeditOption::Close).unwrap_err();
    assert_invalid(error, "pedit.selection");
    assert_wrote_nothing(&app, rev);

    select(&mut session, &[1, 2]);
    let error = pedit(&mut app, &mut session, PeditOption::Close).unwrap_err();
    assert_invalid(error, "pedit.selection");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn non_polyline_geometry_is_refused_by_name() {
    let (mut app, mut session) = build(vec![ent(1, line(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = pedit(&mut app, &mut session, PeditOption::Reverse).unwrap_err();
    assert_unsupported(error, "pedit.not_polyline");
    assert_wrote_nothing(&app, rev);
    assert_eq!(geometry(&app, 1), line());
}

#[test]
fn non_addressable_and_paper_space_refs_are_refused() {
    let (mut app, mut session) = build(vec![ent(1, closed_bulged(), 0)], AppMode::Work);
    let rev = revision(&app);

    // A block-instance member is not directly addressable.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    }]);
    let error = pedit(&mut app, &mut session, PeditOption::Open).unwrap_err();
    assert_unsupported(error, "pedit.not_addressable");
    assert_wrote_nothing(&app, rev);

    // A ref from another document is refused before it is resolved.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(2),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    let error = pedit(&mut app, &mut session, PeditOption::Open).unwrap_err();
    assert_invalid(error, "pedit.not_addressable");
    assert_wrote_nothing(&app, rev);

    // A missing entity is not addressable either.
    select(&mut session, &[99]);
    let error = pedit(&mut app, &mut session, PeditOption::Open).unwrap_err();
    assert_invalid(error, "pedit.not_addressable");
    assert_wrote_nothing(&app, rev);

    // Paper space is out of scope for v1.
    let mut paper = ent(1, closed_bulged(), 0);
    paper.space = SpaceId::Paper(LayoutId(2));
    let (mut app, mut session) = build(vec![paper], AppMode::Work);
    let rev = revision(&app);
    select(&mut session, &[1]);
    let error = pedit(&mut app, &mut session, PeditOption::Open).unwrap_err();
    assert_unsupported(error, "pedit.not_addressable");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn a_foreign_payload_is_refused() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = app
        .execute(
            &mut session,
            command(CommandId::Pedit, CommandPayload::Selection(Vec::new())),
        )
        .unwrap_err();
    assert_invalid(error, "pedit.payload");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn viewer_mode_is_refused_before_anything_is_read_or_written() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Viewer);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = pedit(&mut app, &mut session, PeditOption::Close).unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_wrote_nothing(&app, rev);
    assert_eq!(geometry(&app, 1), open_bulged());
}
