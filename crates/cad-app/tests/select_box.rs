//! Synthetic window/crossing selection contracts (ADR 0014 B); not UI, GPU or
//! vendor validation.
use std::sync::Arc;

use cad_app::interaction::SelectionMode;
use cad_app::{
    AppMode, Application, Command, CommandId, CommandOutcome, CommandPayload, Document,
    SelectionSet, SessionState, ToolState, Viewport,
};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::*;

const DOC: DocumentId = DocumentId(1);
const VIEWPORT: ViewportId = ViewportId(1);
const HIDDEN_LAYER: LayerId = LayerId(1);

const INSIDE: EntityId = EntityId(1);
const STRADDLE: EntityId = EntityId(2);
const OUTSIDE: EntityId = EntityId(3);
const HIDDEN: EntityId = EntityId(4);

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn line(start: (f64, f64), end: (f64, f64)) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(start.0, start.1),
        end: point(end.0, end.1),
    }
}

fn entity(id: EntityId, layer: LayerId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer,
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn fixture(mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: HIDDEN_LAYER,
            name: "HIDDEN".into(),
            visible: false,
        })
        .unwrap();
    builder
        .insert_entity(entity(INSIDE, LayerId(0), line((1.0, 1.0), (2.0, 2.0))))
        .unwrap();
    builder
        .insert_entity(entity(STRADDLE, LayerId(0), line((5.0, 5.0), (15.0, 5.0))))
        .unwrap();
    builder
        .insert_entity(entity(
            OUTSIDE,
            LayerId(0),
            line((100.0, 100.0), (110.0, 110.0)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(HIDDEN, HIDDEN_LAYER, line((3.0, 3.0), (4.0, 4.0))))
        .unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(builder.finish().unwrap()),
            identity: DocumentIdentity::Temporary(1),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace
        .viewports
        .insert(VIEWPORT, Viewport::new(VIEWPORT, DOC, [800.0, 600.0]));
    (app, SessionState::new(DOC, mode))
}

/// Canvas logical corners for a world-space rectangle under the default plan
/// camera (target origin, scale 1.0, 800x600 canvas).
fn logical_rect(min: (f64, f64), max: (f64, f64)) -> [[f64; 2]; 2] {
    [
        [400.0 + min.0, 300.0 - min.1],
        [400.0 + max.0, 300.0 - max.1],
    ]
}

fn box_command(rect_logical: [[f64; 2]; 2], crossing: bool, mode: SelectionMode) -> Command {
    Command {
        schema_version: 1,
        id: CommandId::SelectInBox,
        document: DOC,
        viewport: VIEWPORT,
        payload: CommandPayload::BoxSelect {
            rect_logical,
            crossing,
            mode,
        },
    }
}

fn reference(entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: DOC,
        entity,
        instance: InstancePath(Vec::new()),
        sub_element: None,
    }
}

fn count(outcome: &CommandOutcome) -> usize {
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "select.box.count")
        .expect("select.box.count diagnostic");
    diagnostic
        .message
        .split(';')
        .find_map(|field| field.strip_prefix("count="))
        .expect("count field")
        .parse()
        .expect("numeric count")
}

fn precision(outcome: &CommandOutcome) -> String {
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "select.box.count")
        .expect("select.box.count diagnostic");
    diagnostic
        .message
        .split(';')
        .find_map(|field| field.strip_prefix("precision="))
        .expect("precision field")
        .to_string()
}

#[test]
fn window_selects_only_fully_inside_visible_items() {
    let (mut app, mut session) = fixture(AppMode::Viewer);
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 1);
    assert_eq!(precision(&outcome), "window");
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(INSIDE)])
    );
    assert!(outcome.changes.is_none());
    assert!(app.history.is_empty());
}

#[test]
fn crossing_selects_straddling_and_fully_inside_items() {
    let (mut app, mut session) = fixture(AppMode::Work);
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                true,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 2);
    assert_eq!(precision(&outcome), "exact");
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(INSIDE), reference(STRADDLE)])
    );
}

#[test]
fn rectangle_touching_nothing_is_an_explicit_zero_result() {
    let (mut app, mut session) = fixture(AppMode::Work);
    let before = session.selection.clone();
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((40.0, 40.0), (50.0, 50.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 0);
    assert_eq!(precision(&outcome), "window");
    assert_eq!(session.selection, before);
    assert!(outcome.changes.is_none());
}

#[test]
fn degenerate_and_non_finite_rectangles_are_refused() {
    let (mut app, mut session) = fixture(AppMode::Work);
    let selection = session.selection.clone();
    for rect in [
        [[400.0, 300.0], [400.0, 300.0]],
        [[400.0, 300.0], [400.0, 290.0]],
        [[f64::NAN, 300.0], [410.0, 290.0]],
        [[400.0, f64::INFINITY], [410.0, 290.0]],
    ] {
        let error = app
            .execute(
                &mut session,
                box_command(rect, false, SelectionMode::Replace),
            )
            .expect_err("invalid rectangle must be refused");
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
        assert_eq!(session.selection, selection);
    }
}

#[test]
fn paper_space_is_unsupported_and_a_missing_document_is_invalid_input() {
    let (mut app, mut session) = fixture(AppMode::Work);
    session.active_space = SpaceId::Paper(LayoutId(0));
    let error = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .expect_err("paper space is not supported");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");

    let mut empty_app = Application::new();
    let mut empty_session = SessionState::new(DOC, AppMode::Work);
    let error = empty_app
        .execute(
            &mut empty_session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .expect_err("no document");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
}

#[test]
fn hidden_layer_items_are_filtered_and_overrides_reveal_them() {
    let (mut app, mut session) = fixture(AppMode::Work);
    app.execute(
        &mut session,
        box_command(
            logical_rect((0.0, 0.0), (10.0, 10.0)),
            false,
            SelectionMode::Replace,
        ),
    )
    .unwrap();
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(INSIDE)])
    );

    session.layer_overrides.set(HIDDEN_LAYER, true);
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 2);
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(INSIDE), reference(HIDDEN)])
    );
}

#[test]
fn merge_modes_apply_to_box_matches() {
    let rect = logical_rect((0.0, 0.0), (10.0, 10.0));
    for (mode, expected) in [
        (SelectionMode::Replace, vec![reference(INSIDE)]),
        (
            SelectionMode::Add,
            vec![reference(OUTSIDE), reference(INSIDE)],
        ),
    ] {
        let (mut app, mut session) = fixture(AppMode::Work);
        session.selection = SelectionSet::from_refs([reference(OUTSIDE)]);
        app.execute(&mut session, box_command(rect, false, mode))
            .unwrap();
        assert_eq!(
            session.selection,
            SelectionSet::from_refs(expected),
            "mode {mode:?}"
        );
    }

    let (mut app, mut session) = fixture(AppMode::Work);
    session.selection = SelectionSet::from_refs([reference(OUTSIDE), reference(INSIDE)]);
    app.execute(
        &mut session,
        box_command(rect, false, SelectionMode::Toggle),
    )
    .unwrap();
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(OUTSIDE)])
    );

    let (mut app, mut session) = fixture(AppMode::Work);
    session.selection = SelectionSet::from_refs([reference(OUTSIDE), reference(INSIDE)]);
    app.execute(
        &mut session,
        box_command(rect, false, SelectionMode::Remove),
    )
    .unwrap();
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(OUTSIDE)])
    );
}

#[test]
fn viewer_mode_is_allowed_for_box_selection() {
    assert!(!CommandId::SelectInBox.requires_work_mode());
    let (mut app, mut session) = fixture(AppMode::Viewer);
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 1);
    assert_eq!(session.selection.len(), 1);
}

#[test]
fn opaque_geometry_is_refused_by_preflight_without_changing_selection() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            EntityId(9),
            LayerId(0),
            SemanticGeometry::Opaque {
                type_key: "AcDbUnknown".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        ))
        .unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(builder.finish().unwrap()),
            identity: DocumentIdentity::Temporary(2),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace
        .viewports
        .insert(VIEWPORT, Viewport::new(VIEWPORT, DOC, [800.0, 600.0]));
    let mut session = SessionState::new(DOC, AppMode::Work);
    session.selection = SelectionSet::from_refs([reference(OUTSIDE)]);
    let error = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                true,
                SelectionMode::Replace,
            ),
        )
        .expect_err("opaque geometry must be refused, never faked");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(
        session.selection,
        SelectionSet::from_refs([reference(OUTSIDE)])
    );
}

#[test]
fn active_non_selecting_tool_refuses_visibly_with_a_zero_count() {
    let (mut app, mut session) = fixture(AppMode::Work);
    session.selection = SelectionSet::from_refs([reference(OUTSIDE)]);
    session.begin_draw(cad_app::DrawToolKind::Line).unwrap();
    let before = session.selection.clone();
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                false,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(session.selection, before);
    assert!(matches!(session.tool, ToolState::Drawing(_)));
    // The refusal must be visible, not a silent empty success: the same stable
    // diagnostic with `count=0` (ADR 0014 B.8).
    assert_eq!(count(&outcome), 0);
    assert_eq!(precision(&outcome), "none");
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
}

/// A one-entity document whose only geometry is `geometry`, on a visible layer,
/// under the default plan camera. Keeps the reported `precision` unambiguous
/// (a single class, never `mixed`).
fn single_geometry_fixture(geometry: SemanticGeometry) -> (Application, SessionState) {
    const ONLY: EntityId = EntityId(7);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(entity(ONLY, LayerId(0), geometry))
        .unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(builder.finish().unwrap()),
            identity: DocumentIdentity::Temporary(3),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace
        .viewports
        .insert(VIEWPORT, Viewport::new(VIEWPORT, DOC, [800.0, 600.0]));
    (app, SessionState::new(DOC, AppMode::Work))
}

/// Run one CROSSING box over `geometry` and return the reported precision.
fn crossing_precision(geometry: SemanticGeometry) -> String {
    let (mut app, mut session) = single_geometry_fixture(geometry);
    let outcome = app
        .execute(
            &mut session,
            box_command(
                logical_rect((0.0, 0.0), (10.0, 10.0)),
                true,
                SelectionMode::Replace,
            ),
        )
        .unwrap();
    assert_eq!(count(&outcome), 1, "the geometry intersects the rectangle");
    precision(&outcome)
}

#[test]
fn crossing_a_straight_segment_is_reported_exact() {
    // A straight segment has an exact rectangle clip: never approximate.
    assert_eq!(
        crossing_precision(line((5.0, 5.0), (15.0, 5.0))),
        "exact",
        "a straight segment crossing must be reported exact"
    );
}

#[test]
fn crossing_a_circle_or_arc_is_reported_approximate() {
    let circle = SemanticGeometry::Circle {
        center: point(5.0, 5.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius: 2.0,
    };
    assert_eq!(
        crossing_precision(circle),
        "approximate",
        "a chord-tessellated circle crossing must be reported approximate"
    );

    let arc = SemanticGeometry::Arc {
        center: point(5.0, 5.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius: 2.0,
        start: 0.0,
        sweep: std::f64::consts::PI,
    };
    assert_eq!(
        crossing_precision(arc),
        "approximate",
        "a chord-tessellated arc crossing must be reported approximate"
    );
}

#[test]
fn crossing_a_bulge_polyline_or_spline_falls_back_to_aabb() {
    // A bulge segment is a circular arc, not its chord, and no public
    // segment-wise bulge tessellator is reachable here: the conservative AABB is
    // used and must be reported as such, never as exact/approximate.
    let bulged = SemanticGeometry::Polyline {
        points: vec![point(5.0, 0.0), point(5.0, 10.0)],
        bulges: vec![1.0],
        closed: false,
    };
    assert_eq!(
        crossing_precision(bulged),
        "aabb",
        "a bulge polyline crossing must be reported aabb"
    );

    // Splines likewise have no rectangle tessellator reachable here.
    let spline = SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        control_points: vec![point(2.0, 2.0), point(5.0, 8.0), point(8.0, 2.0)],
        weights: vec![1.0, 1.0, 1.0],
    };
    assert_eq!(
        crossing_precision(spline),
        "aabb",
        "a spline crossing must be reported aabb"
    );
}

fn hatch_geometry() -> SemanticGeometry {
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
    SemanticGeometry::Hatch(Box::new(hatch))
}

/// An app/session with one model-space entity (id 1) holding `geometry`.
fn single_entity_app(geometry: SemanticGeometry) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbHatch".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: 0,
        })
        .unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(builder.finish().unwrap()),
            identity: DocumentIdentity::Temporary(1),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace
        .viewports
        .insert(VIEWPORT, Viewport::new(VIEWPORT, DOC, [800.0, 600.0]));
    (app, SessionState::new(DOC, AppMode::Work))
}

#[test]
fn hatch_box_select_matches_an_equivalent_compound() {
    // ADR 0018 E: a first-class hatch classifies exactly like the compound of
    // its display children (the previous wildcard degraded it to the item AABB).
    let hatch = hatch_geometry();
    let SemanticGeometry::Hatch(geometry) = &hatch else {
        unreachable!("helper returns a hatch");
    };
    let compound = SemanticGeometry::Compound(geometry.display().to_vec());
    let mut results = Vec::new();
    for geometry in [hatch, compound] {
        let (mut app, mut session) = single_entity_app(geometry);
        for crossing in [false, true] {
            let outcome = app
                .execute(
                    &mut session,
                    box_command(
                        logical_rect((-1.0, -1.0), (5.0, 5.0)),
                        crossing,
                        SelectionMode::Replace,
                    ),
                )
                .unwrap();
            results.push((
                count(&outcome),
                precision(&outcome),
                session.selection.clone(),
            ));
        }
    }
    assert_eq!(results[0], results[2], "window: hatch vs compound differ");
    assert_eq!(results[1], results[3], "crossing: hatch vs compound differ");
    assert_eq!(results[0].0, 1, "the hatch window-selects");
    assert_eq!(
        results[0].2,
        SelectionSet::from_refs([reference(EntityId(1))])
    );
}
