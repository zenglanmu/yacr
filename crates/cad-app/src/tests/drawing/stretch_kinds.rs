//! STRETCH contract tests: the move-only 2D kinds (HATCH / IMAGE / MASK / TABLE /
//! ELLIPSE) added in the 2026-11-11 completion batch.
//!
//! Split out of `stretch.rs` so both test files stay inside the code-size limit.
//! A partial enclosure of a move-only kind never moves it and always reports the
//! kind-specific `drawing.stretch.<reason>` diagnostic; a full enclosure still
//! moves the whole entity, exactly like every other fully enclosed entity.

use super::stretch::{
    build, entity_count, geometry_of, has_code, line, line_of, near, stretch, undo_depth, window,
};
use super::*;
use cad_domain::{
    TableCellBorders, TableCellGeometry, TableColumnGeometry, TableFrame, TableGeometry,
    TableParts, TableRowGeometry,
};

// ---------------------------------------------------------------------------
// The 2026-11-11 completion batch: the move-only 2D kinds (HATCH / IMAGE / MASK /
// TABLE / ELLIPSE). A partial enclosure never moves them and always says so; a
// full enclosure still moves the whole entity.
// ---------------------------------------------------------------------------

/// A first-class solid HATCH over a 4x4 square (boundary loop + fill mesh).
fn solid_hatch_geometry() -> SemanticGeometry {
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

/// A raster image whose world box is the 4x4 square at the origin.
fn image_geometry() -> SemanticGeometry {
    SemanticGeometry::Image {
        origin: point(0.0, 0.0),
        u: point(4.0, 0.0),
        v: point(0.0, 4.0),
        pixels: [1.0, 1.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    }
}

/// A mask over the same 4x4 square.
fn mask_geometry() -> SemanticGeometry {
    SemanticGeometry::Mask {
        boundary: vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 4.0),
            point(0.0, 4.0),
        ],
        inverted: false,
    }
}

/// A 1x1 TABLE at the origin: 4 wide, 3 high, one cell.
fn table_geometry() -> SemanticGeometry {
    let cell = TableCellGeometry {
        text: "cell".into(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
    };
    let table = TableGeometry::new(TableParts {
        frame: TableFrame::default(),
        columns: vec![TableColumnGeometry {
            width: 4.0,
            name: "A".into(),
        }],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell],
        }],
        style_name: "Standard".into(),
        name: "Data".into(),
        description: String::new(),
    })
    .expect("valid table");
    SemanticGeometry::Table(Box::new(table))
}

/// A flat ellipse at the origin: major radius 4 along +x, ratio 0.5.
fn ellipse_geometry() -> SemanticGeometry {
    SemanticGeometry::Ellipse {
        center: point(0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(4.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    }
}

/// Run one STRETCH whose crossing window **partially** encloses `geometry` and
/// assert the kind-specific "left unchanged" contract: nothing is written, no
/// transaction is opened, and the stable `drawing.stretch.<reason>` diagnostic is
/// reported exactly once.
fn assert_partial_enclosure_is_move_only(
    geometry: SemanticGeometry,
    window_corners: (Point3, Point3),
    reason: &str,
) {
    let (mut app, mut session) = build(vec![ent(1, geometry.clone(), 0)], Vec::new());
    let outcome = stretch(
        &mut app,
        &mut session,
        window(window_corners.0, window_corners.1),
        point(0.0, 0.0),
        point(10.0, 10.0),
    )
    .expect("a move-only kind never fails the command on a partial enclosure");
    assert!(
        outcome.changes.is_none(),
        "no transaction is opened when nothing changes"
    );
    assert_eq!(outcome.objects.len(), 0, "no entity is reported as changed");
    assert_eq!(undo_depth(&app), 0, "no history entry");
    assert_eq!(entity_count(&app), 1, "the entity is retained");
    assert_eq!(
        geometry_of(&app, 1),
        geometry,
        "the geometry is bit-identically unchanged (it never silently moves)"
    );
    let code = format!("drawing.stretch.{reason}");
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == code)
            .count(),
        1,
        "exactly one named diagnostic for {reason}: {:?}",
        outcome.diagnostics
    );
    assert!(
        has_code(&outcome, "drawing.stretch"),
        "the summary diagnostic is still present"
    );
    assert!(
        !has_code(&outcome, "drawing.stretch.not_stretchable"),
        "only the kind-specific reason is reported: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_hatch_partial_enclosure_never_moves_it_and_reports_the_reason_once() {
    // A window over the top-right corner of the 4x4 hatch: a crossing hit that is
    // not a full enclosure. A HATCH is not stretchable, so it must stay put — and
    // the command must say so exactly once (never a silent move, never the old
    // duplicate `hatch` + `not_stretchable` pair).
    assert_partial_enclosure_is_move_only(
        solid_hatch_geometry(),
        (point(3.0, 3.0), point(6.0, 6.0)),
        "hatch",
    );
}

#[test]
fn stretch_fully_enclosed_hatch_moves_as_a_whole() {
    // A full enclosure is the ordinary whole-entity move (ADR 0018 E), so the
    // boundary loop and the fill move together by exactly the displacement.
    let (mut app, mut session) = build(vec![ent(1, solid_hatch_geometry(), 0)], Vec::new());
    let before = match geometry_of(&app, 1) {
        SemanticGeometry::Hatch(hatch) => hatch.display().to_vec(),
        other => panic!("expected a hatch, got {other:?}"),
    };
    execute(
        &mut app,
        &mut session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: window(point(-5.0, -5.0), point(10.0, 10.0)),
            base: point(0.0, 0.0),
            delta: point(3.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(undo_depth(&app), 1, "one undo step");
    let SemanticGeometry::Hatch(after) = geometry_of(&app, 1) else {
        panic!("expected a hatch");
    };
    let SemanticGeometry::Polyline { points, .. } = &before[0] else {
        panic!(
            "the first display child is the boundary loop: {:?}",
            before[0]
        );
    };
    let SemanticGeometry::Polyline {
        points: moved_points,
        ..
    } = &after.display()[0]
    else {
        panic!("the first display child is still the boundary loop");
    };
    assert_eq!(points.len(), moved_points.len());
    for (original, moved) in points.iter().zip(moved_points) {
        assert!(
            near(*moved, p3(original.x + 3.0, original.y, original.z)),
            "the boundary loop moves as a whole: {original:?} -> {moved:?}"
        );
    }
}

#[test]
fn stretch_partial_image_and_mask_never_move_and_are_named() {
    // A raster image and a mask each partially covered by a crossing window: neither
    // has a per-vertex rule in this build, so each stays put with its own reason
    // instead of failing the whole command (the pre-2026-11-11 behaviour) or being
    // half-moved.
    assert_partial_enclosure_is_move_only(
        image_geometry(),
        (point(3.0, 3.0), point(6.0, 6.0)),
        "image",
    );
    assert_partial_enclosure_is_move_only(
        mask_geometry(),
        (point(3.0, 3.0), point(6.0, 6.0)),
        "mask",
    );
}

#[test]
fn stretch_partial_table_never_moves_and_is_named() {
    // The window is derived from the table's own display box (the frame's row
    // direction is the domain's, not the test's guess): lower-left corner plus one
    // unit of margin up to the box centre is a crossing hit that cannot enclose it.
    let geometry = table_geometry();
    let (min, max) = cad_spatial::pick_world_bounds(&geometry, &Transform3::identity())
        .expect("a table has a computable box");
    let window_corners = (
        p3(min.x - 1.0, min.y - 1.0, min.z),
        p3((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, min.z),
    );
    assert_partial_enclosure_is_move_only(geometry, window_corners, "table");
}

#[test]
fn stretch_fully_enclosed_image_moves_as_a_whole() {
    let (mut app, mut session) = build(vec![ent(1, image_geometry(), 0)], Vec::new());
    execute(
        &mut app,
        &mut session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: window(point(-5.0, -5.0), point(10.0, 10.0)),
            base: point(0.0, 0.0),
            delta: point(0.0, 7.0),
        },
    )
    .unwrap();
    let SemanticGeometry::Image { origin, u, v, .. } = geometry_of(&app, 1) else {
        panic!("expected an image");
    };
    assert!(near(origin, p3(0.0, 7.0, 0.0)), "origin moved: {origin:?}");
    assert_eq!(u, point(4.0, 0.0), "the image frame is not rescaled");
    assert_eq!(v, point(0.0, 4.0), "the image frame is not rescaled");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn stretch_partial_ellipse_enclosure_is_move_only_with_a_named_reason() {
    // One in-window quadrant point cannot determine new axes or a ratio, so an
    // ellipse is move-only: a partial enclosure leaves it untouched and says so.
    assert_partial_enclosure_is_move_only(
        ellipse_geometry(),
        (point(2.0, 1.0), point(8.0, 8.0)),
        "ellipse",
    );
}

#[test]
fn stretch_partial_enclosure_of_a_mixed_selection_changes_only_the_stretchable_kind() {
    // A LINE plus an IMAGE in the same crossing window: the LINE's in-window
    // endpoint stretches, the IMAGE stays put with its named reason, and the whole
    // edit is still one transaction / one undo step.
    let (mut app, mut session) = build(
        vec![
            line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
            ent(2, image_geometry(), 1),
        ],
        Vec::new(),
    );
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(1.0, 7.0)),
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "only the LINE changes");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    assert_eq!(line_of(&app, 1), (point(0.0, 5.0), point(10.0, 0.0)));
    assert_eq!(
        geometry_of(&app, 2),
        image_geometry(),
        "the image never moves"
    );
    assert!(
        has_code(&outcome, "drawing.stretch.image"),
        "the image reason is reported: {:?}",
        outcome.diagnostics
    );
}
