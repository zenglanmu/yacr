//! ARRAYPATH (2D path array v1) contracts.
//!
//! The first selected entity is the path and the remaining entities are the
//! items; every item is copied to `count` equal arc-length positions along the
//! path. Split out of `drawing/array.rs` so both suites stay inside the
//! 1000-line code gate.

use super::array::{all_lines, array_app, entity_count, one_line_app, undo_depth};
use super::*;

use cad_db::{BlockDefinition, DrawingDatabaseBuilder, Layout};
use cad_domain::{BlockId, InstancePath, LayoutId, SpaceId};

// ---------------------------------------------------------------------------
// ARRAYPATH (2D path array v1): the first payload ref is the path, the rest are
// items placed at equal arc-length positions. `count` is the number of copies.
// ---------------------------------------------------------------------------

/// The typed payload with an explicit selection snapshot.
pub(super) fn arraypath_payload(
    refs: Vec<SelectionRef>,
    count: usize,
    align: bool,
) -> CommandPayload {
    CommandPayload::ArrayPath { refs, count, align }
}

/// Build the standard test application around a finished database builder.
fn finish_test_app(b: DrawingDatabaseBuilder) -> (Application, SessionState) {
    let drawing = b.finish().unwrap();
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
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// Every model-space INSERT's `(world insertion base point, block)`.
pub(super) fn all_inserts(app: &Application) -> Vec<(Point3, BlockId)> {
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.as_ref();
    let mut inserts: Vec<(Point3, BlockId)> = drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Insert { block, transform } => {
                let base = drawing
                    .block_base_point(*block)
                    .expect("test block metadata");
                Some((transform.apply_point(base), *block))
            }
            _ => None,
        })
        .collect();
    inserts.sort_by(|a, b| a.0.x.partial_cmp(&b.0.x).expect("finite insertions"));
    inserts
}

/// A drawing with a path (id 1) and one INSERT item (id 2) of a block whose base
/// point is the world origin.
pub(super) fn insert_item_app(
    path: SemanticGeometry,
    insertion: Point3,
    block_base: Point3,
) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(0), "SQUARE", block_base)
        .unwrap();
    b.insert_entity(ent(1, path, 0)).unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Insert {
            block: BlockId(0),
            // The importer/INSERT convention: the block base point lands on
            // the insertion point.
            transform: Transform3::translation(insertion).matrix_mul(&Transform3::translation(
                Point3 {
                    x: -block_base.x,
                    y: -block_base.y,
                    z: -block_base.z,
                },
            )),
        },
        1,
    ))
    .unwrap();
    finish_test_app(b)
}

pub(super) fn z_axis() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

fn all_circles(app: &Application) -> Vec<(Point3, f64)> {
    let mut circles: Vec<(Point3, f64)> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Circle { center, radius, .. } => Some((*center, *radius)),
            _ => None,
        })
        .collect();
    circles.sort_by(|a, b| a.0.x.partial_cmp(&b.0.x).expect("finite centres"));
    circles
}

pub(super) fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn arraypath_straight_path_places_every_item_at_equal_arc_length_positions() {
    // Path: a straight LINE (0,0)-(9,0). Items: a LINE and a CIRCLE. The declared
    // group anchor is the componentwise `BoundsAccumulator` minimum of the item
    // anchors (BoundsAccumulator's circular bound is conservative, so the anchor
    // is below the circle's tight box).
    let item_line = SemanticGeometry::Line {
        start: point(0.0, 1.0),
        end: point(1.0, 1.0),
    };
    let item_circle = SemanticGeometry::Circle {
        center: point(2.0, 1.0),
        normal: z_axis(),
        radius: 1.0,
    };
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (2, item_line.clone()),
        (3, item_circle.clone()),
    ]);
    session
        .selection
        .replace(vec![ref_of(1), ref_of(2), ref_of(3)]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2), ref_of(3)], 4, false),
    )
    .unwrap();

    // 2 items x 4 copies = 8 independent copies; the path is not duplicated.
    assert_eq!(outcome.objects.len(), 8);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(entity_count(&app), 11, "path + 2 items + 8 copies");

    // The declared group anchor, computed from the same `BoundsAccumulator` rule.
    let mut bounds = cad_db::BoundsAccumulator::new();
    bounds.add_geometry(&item_line);
    bounds.add_geometry(&item_circle);
    let anchor = bounds.finish().expect("finite item bounds").0;

    // Every item LINE copy starts at its path position plus its own (0,1) offset,
    // translated by `−anchor`.
    let copy_y = 1.0 - anchor.y;
    let mut starts: Vec<f64> = all_lines(&app)
        .into_iter()
        .filter(|(start, _)| near(start.y, 1.0) || near(start.y, copy_y))
        .map(|(start, _)| start.x)
        .collect();
    starts.sort_by(|a, b| a.partial_cmp(b).expect("finite x"));
    // Five lines share the copy y: the retained source item (x=0) plus the four
    // copies at 0/3/6/9 minus the anchor x.
    let expected = [
        0.0 - anchor.x,
        0.0 - anchor.x,
        3.0 - anchor.x,
        6.0 - anchor.x,
        9.0 - anchor.x,
    ];
    assert_eq!(starts.len(), expected.len());
    for (actual, expected) in starts.iter().zip(expected) {
        assert!(near(*actual, expected), "{actual} != {expected}");
    }
    // The CIRCLE copies sit at the same four translations from their own geometry.
    let circles = all_circles(&app);
    assert_eq!(circles.len(), 5, "source + four CIRCLE copies");
    let expected_x = [
        2.0,
        2.0 - anchor.x,
        5.0 - anchor.x,
        8.0 - anchor.x,
        11.0 - anchor.x,
    ];
    let expected_y = [
        1.0,
        1.0 - anchor.y,
        1.0 - anchor.y,
        1.0 - anchor.y,
        1.0 - anchor.y,
    ];
    for ((center, radius), (x, y)) in circles.iter().zip(expected_x.into_iter().zip(expected_y)) {
        assert!(near(center.x, x) && near(center.y, y), "{center:?}");
        assert!(near(*radius, 1.0));
    }

    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "arraypath.created")
        .expect("ARRAYPATH reports its placement summary");
    assert!(
        diagnostic.message.contains("4 copies"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("2 sources retained"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("path 1 (AcDbLine)"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("align=false"),
        "{}",
        diagnostic.message
    );

    // UNDO removes every copy and restores the exact prior entity count.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 3, "undo removes every copy");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(all_circles(&app).len(), 1, "only the source CIRCLE remains");

    // REDO reapplies the whole array.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 11);
    assert_eq!(
        all_lines(&app).len(),
        6,
        "path + source item + 4 item LINE copies"
    );
}

#[test]
fn arraypath_align_rotates_each_copy_by_the_local_tangent_delta() {
    // A quarter ARC path: at u = 1 the tangent is PI while the path start tangent
    // is PI/2, so the second copy is rotated by +PI/2 about its own position.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Arc {
                center: point(0.0, 0.0),
                normal: z_axis(),
                radius: 10.0,
                start: 0.0,
                sweep: std::f64::consts::FRAC_PI_2,
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, true),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(entity_count(&app), 4, "arc + item + 2 copies");

    // The copy at the path end is anchored at (0,10) and its local direction is the
    // rotated original +X, i.e. +Y (angle PI/2).
    let end_copy = all_lines(&app)
        .into_iter()
        .find(|(start, _)| near(start.x, 0.0) && near(start.y, 10.0))
        .expect("a copy is placed at the quarter-arc end");
    let angle = (end_copy.1.y - end_copy.0.y).atan2(end_copy.1.x - end_copy.0.x);
    assert!(
        near(angle, std::f64::consts::FRAC_PI_2),
        "copy angle {angle} != PI/2"
    );

    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "arraypath.created")
        .expect("ARRAYPATH reports its placement summary");
    assert!(
        diagnostic.message.contains("align=true"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn arraypath_a_single_position_lands_at_the_path_start() {
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 1, false),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(entity_count(&app), 3);
    // The item's bounding-box lower-left (0,1) is translated to the path start.
    let copy = all_lines(&app)
        .into_iter()
        .find(|(start, _)| near(start.y, 0.0))
        .expect("the single copy sits at the path start");
    assert!(near(copy.0.x, 0.0) && near(copy.0.y, 0.0));
    assert!(near(copy.1.x, 1.0) && near(copy.1.y, 0.0));
}

#[test]
fn arraypath_refusals_write_nothing() {
    // No refs at all (a path needs at least one item).
    let (mut app, mut session) = one_line_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(Vec::new(), 2, false),
    )
    .expect_err("an empty refs payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);

    // Only the path (no item).
    let (mut app, mut session) = one_line_app();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1)], 2, false),
    )
    .expect_err("a path without an item is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 1);

    // A block-instance member ref is refused before the drawing is read.
    let (mut app, mut session) = one_line_app();
    let mut member = ref_of(2);
    member.instance = InstancePath(vec![EntityId(1)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), member], 2, false),
    )
    .expect_err("a block-instance member is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.contains("arraypath.block_member")),
        "{error:?}"
    );

    // A sub-element ref is refused the same way.
    let mut sub_element = ref_of(2);
    sub_element.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge".into(),
        topology_revision: Revision(0),
    });
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), sub_element], 2, false),
    )
    .expect_err("a sub-element is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.contains("arraypath.block_member")),
        "{error:?}"
    );

    // An unsupported path kind (OPAQUE) is refused as Unsupported.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, false),
    )
    .expect_err("an unsupported path is refused");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);

    // A zero-length path is refused without a fabricated position.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(5.0, 5.0),
                end: point(5.0, 5.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, false),
    )
    .expect_err("a zero-length path is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 2);

    // A path and item in different spaces are refused.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(0),
        name: "Layout1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(9.0, 0.0),
        },
        0,
    ))
    .unwrap();
    let mut paper_item = ent(
        2,
        SemanticGeometry::Line {
            start: point(0.0, 1.0),
            end: point(1.0, 1.0),
        },
        1,
    );
    paper_item.space = SpaceId::Paper(LayoutId(0));
    b.insert_entity(paper_item).unwrap();
    let (mut app, mut session) = finish_test_app(b);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, false),
    )
    .expect_err("a space mismatch is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.contains("arraypath.space_mismatch")),
        "{error:?}"
    );
    assert_eq!(entity_count(&app), 2);

    // Count 0.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 0, false),
    )
    .expect_err("count 0 is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 2);

    // Cap overflow (1 item x (cap + 1) copies).
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(
            vec![ref_of(1), ref_of(2)],
            crate::app_drawing::MAX_ARRAY_ENTITIES + 1,
            false,
        ),
    )
    .expect_err("above the array cap is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
