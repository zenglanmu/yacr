//! `MIRROR` of the curved kinds (`Arc`/`Ellipse`) contract tests.
//!
//! MIRROR reflects Arc/Ellipse **explicitly** in `cad-app`
//! (`app_drawing::mirror_curves`) instead of loosening `cad_db`'s proper-similarity
//! transform surface. These tests pin the declared rule: the plane normal is
//! negated, the arc's start angle is re-derived from its **mirrored start point**
//! with the sweep preserved, and an ellipse (full turn **and** elliptical arc)
//! reflects centre/axis with its parameter preserved. The assertions are
//! point-based (start / mid / end equal the reflections of the source points), so
//! they hold for the geometry, not for one hand-computed angle table.
//!
//! Split out of `transform.rs` (which keeps ROTATE/SCALE and the general MIRROR
//! contracts) so both files stay well inside the 1000-line code-file cap.

use super::*;

/// One point on an arc, using the display tessellator's own frame
/// (`cad_geometry::arbitrary_axis` + `cos/sin`), so an "endpoint" here is the
/// vertex the entity is actually drawn with.
fn arc_point(center: Point3, radius: f64, normal: Point3, angle: f64) -> Point3 {
    let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
    Point3 {
        x: center.x + radius * (angle.cos() * ax.x + angle.sin() * ay.x),
        y: center.y + radius * (angle.cos() * ax.y + angle.sin() * ay.y),
        z: center.z + radius * (angle.cos() * ax.z + angle.sin() * ay.z),
    }
}

/// One point on an ellipse, `c + cos t·A + sin t·B` with
/// `B = cross(normal, Â)·|A|·|ratio|` (the tessellator's formula).
fn ellipse_point(center: Point3, normal: Point3, major_axis: Point3, ratio: f64, t: f64) -> Point3 {
    let major_len = cad_geometry::length(major_axis);
    let unit = cad_geometry::scale(major_axis, 1.0 / major_len);
    let minor = cad_geometry::scale(
        cad_geometry::normalize(cad_geometry::cross(normal, unit)),
        major_len * ratio.abs(),
    );
    Point3 {
        x: center.x + t.cos() * major_axis.x + t.sin() * minor.x,
        y: center.y + t.cos() * major_axis.y + t.sin() * minor.y,
        z: center.z + t.cos() * major_axis.z + t.sin() * minor.z,
    }
}

/// One of the three mirror axes used below, with its own point reflection.
#[derive(Clone, Copy)]
enum MirrorAxis {
    X,
    Y,
    Diagonal,
}

impl MirrorAxis {
    fn endpoints(self) -> (Point3, Point3) {
        match self {
            MirrorAxis::X => (point(0.0, 0.0), point(1.0, 0.0)),
            MirrorAxis::Y => (point(0.0, 0.0), point(0.0, 1.0)),
            MirrorAxis::Diagonal => (point(0.0, 0.0), point(1.0, 1.0)),
        }
    }

    /// The reflection this axis defines, in closed form (independent of the
    /// command's own transform construction).
    fn reflect(self, p: Point3) -> Point3 {
        match self {
            MirrorAxis::X => point(p.x, -p.y),
            MirrorAxis::Y => point(-p.x, p.y),
            MirrorAxis::Diagonal => point(p.y, p.x),
        }
    }
}

/// Mirror one entity (id 2) and return the mirrored copy's geometry.
fn mirror_one(
    app: &mut Application,
    session: &mut SessionState,
    axis: MirrorAxis,
) -> SemanticGeometry {
    let (start, end) = axis.endpoints();
    let outcome = execute(
        app,
        session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(2)],
            axis_start: start,
            axis_end: end,
        },
    )
    .expect("MIRROR runs");
    let copy = EntityId(outcome.objects[0].0);
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy)
        .expect("the mirrored copy exists")
        .geometry
        .clone()
}

fn assert_point_eq(actual: Point3, expected: Point3, what: &str) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "{what}: expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn mirror_reflects_an_arc_across_the_x_y_and_diagonal_axes() {
    // A quarter-ish arc on a +Z plane; the three axes cover a horizontal, a
    // vertical and a diagonal reflection.
    let center = point(3.0, 1.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 2.0;
    let start = 0.4;
    let sweep = 1.2;
    let source = SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    };
    let source_start = arc_point(center, radius, normal, start);
    let source_end = arc_point(center, radius, normal, start + sweep);

    for axis in [MirrorAxis::X, MirrorAxis::Y, MirrorAxis::Diagonal] {
        let mut app = line_and_boundary_app(ent(2, source.clone(), 1), None);
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let mirrored = mirror_one(&mut app, &mut session, axis);
        let SemanticGeometry::Arc {
            center: new_center,
            normal: new_normal,
            radius: new_radius,
            start: new_start,
            sweep: new_sweep,
        } = mirrored
        else {
            panic!("expected an arc");
        };
        // The centre is the mirrored centre and the radius is unchanged.
        assert_point_eq(new_center, axis.reflect(center), "arc centre");
        assert!((new_radius - radius).abs() < 1e-9, "{new_radius}");
        // The plane normal is negated (a reflection reverses orientation).
        assert_point_eq(
            new_normal,
            p3(-0.0, -0.0, -1.0),
            "the mirrored plane normal is -normal",
        );
        // The directed mirror image: mirror(start) -> mirror(end), exactly.
        assert_point_eq(
            arc_point(new_center, new_radius, new_normal, new_start),
            axis.reflect(source_start),
            "mirrored start point",
        );
        assert_point_eq(
            arc_point(new_center, new_radius, new_normal, new_start + new_sweep),
            axis.reflect(source_end),
            "mirrored end point",
        );
        // The sweep keeps its magnitude **and** sign (this build's negated-normal
        // frame; see `app_drawing::mirror_curves`).
        assert!(
            (new_sweep - sweep).abs() < 1e-9,
            "the sweep must be preserved, got {new_sweep}"
        );
        // The mid-parameter point is the reflection of the source's.
        let mid = start + sweep * 0.5;
        assert_point_eq(
            arc_point(
                new_center,
                new_radius,
                new_normal,
                new_start + new_sweep * 0.5,
            ),
            axis.reflect(arc_point(center, radius, normal, mid)),
            "mirrored mid-arc point",
        );
    }
}

#[test]
fn mirror_reflects_a_semicircle_and_a_major_arc() {
    let center = point(-2.0, 4.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 3.0;
    for sweep in [std::f64::consts::PI, 4.0_f64] {
        let start = 2.5_f64;
        let source = SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        };
        let source_start = arc_point(center, radius, normal, start);
        let source_end = arc_point(center, radius, normal, start + sweep);
        let mut app = line_and_boundary_app(ent(2, source, 1), None);
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let mirrored = mirror_one(&mut app, &mut session, MirrorAxis::Y);
        let SemanticGeometry::Arc {
            center: new_center,
            normal: new_normal,
            radius: new_radius,
            start: new_start,
            sweep: new_sweep,
        } = mirrored
        else {
            panic!("expected an arc");
        };
        assert!(
            (new_sweep.abs() - sweep).abs() < 1e-9,
            "|sweep| {new_sweep}"
        );
        assert!((new_sweep - sweep).abs() < 1e-9, "sweep {new_sweep}");
        assert_point_eq(
            arc_point(new_center, new_radius, new_normal, new_start),
            MirrorAxis::Y.reflect(source_start),
            "start",
        );
        assert_point_eq(
            arc_point(new_center, new_radius, new_normal, new_start + new_sweep),
            MirrorAxis::Y.reflect(source_end),
            "end",
        );
    }
}

#[test]
fn mirror_reflects_a_full_ellipse_without_reversing_its_axis() {
    let center = point(1.0, 2.0);
    let normal = p3(0.0, 0.0, 1.0);
    let major_axis = p3(4.0, 0.0, 0.0);
    let ratio = 0.5;
    let source = SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let mut app = line_and_boundary_app(ent(2, source, 1), None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let mirrored = mirror_one(&mut app, &mut session, MirrorAxis::Diagonal);
    let SemanticGeometry::Ellipse {
        center: new_center,
        normal: new_normal,
        major_axis: new_major,
        ratio: new_ratio,
        start: new_start,
        sweep: new_sweep,
    } = mirrored
    else {
        panic!("expected an ellipse");
    };
    assert_point_eq(new_center, MirrorAxis::Diagonal.reflect(center), "centre");
    assert_point_eq(
        new_major,
        MirrorAxis::Diagonal.reflect(major_axis),
        "major axis",
    );
    assert!((new_ratio - ratio).abs() < 1e-9);
    assert_point_eq(new_normal, p3(-0.0, -0.0, -1.0), "negated normal");
    // A full ellipse keeps a full turn, and its four quadrant points are the
    // reflections of the source's (the axis did not silently reverse).
    assert!(
        (new_sweep - std::f64::consts::TAU).abs() < 1e-9,
        "{new_sweep}"
    );
    for t in [0.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI] {
        assert_point_eq(
            ellipse_point(new_center, new_normal, new_major, new_ratio, new_start + t),
            MirrorAxis::Diagonal.reflect(ellipse_point(center, normal, major_axis, ratio, t)),
            "ellipse point",
        );
    }
}

#[test]
fn mirror_reflects_an_elliptical_arc_end_to_end_and_at_mid_parameter() {
    // An **elliptical arc** (a real start/sweep, not a full turn): the reflection
    // keeps the parameter, so the mirrored start, end and mid-parameter points are
    // exactly the reflections of the source's. This is the proof that the
    // parameter mapping is right (no `θ' = -θ` inversion is needed under the
    // negated-normal convention).
    let center = point(-1.0, 0.5);
    let normal = p3(0.0, 0.0, 1.0);
    let major_axis = p3(3.0, 1.0, 0.0);
    let ratio = 0.4;
    let start = 0.3;
    let sweep = 2.0;
    let source = SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start,
        sweep,
    };
    for axis in [MirrorAxis::X, MirrorAxis::Diagonal] {
        let mut app = line_and_boundary_app(ent(2, source.clone(), 1), None);
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let mirrored = mirror_one(&mut app, &mut session, axis);
        let SemanticGeometry::Ellipse {
            center: new_center,
            normal: new_normal,
            major_axis: new_major,
            ratio: new_ratio,
            start: new_start,
            sweep: new_sweep,
        } = mirrored
        else {
            panic!("expected an ellipse");
        };
        assert!((new_sweep - sweep).abs() < 1e-9, "sweep {new_sweep}");
        for (offset, what) in [(0.0, "start"), (sweep * 0.5, "mid"), (sweep, "end")] {
            assert_point_eq(
                ellipse_point(
                    new_center,
                    new_normal,
                    new_major,
                    new_ratio,
                    new_start + offset,
                ),
                axis.reflect(ellipse_point(
                    center,
                    normal,
                    major_axis,
                    ratio,
                    start + offset,
                )),
                what,
            );
        }
    }
}

#[test]
fn mirror_of_a_mixed_line_and_arc_selection_is_one_undo_step() {
    // Entity 1 is a LINE, entity 2 an ARC: both must mirror in **one** transaction
    // and one undo step, and one undo must remove both copies.
    let arc = SemanticGeometry::Arc {
        center: point(0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 5.0,
        start: 0.0,
        sweep: 1.0,
    };
    let mut app = line_and_boundary_app(ent(2, arc, 1), None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1), ref_of(2)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .expect("MIRROR accepts a LINE + ARC selection");
    assert_eq!(outcome.objects.len(), 2, "one copy per source");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        1,
        "one transaction, one undo step"
    );
    // The originals are kept (AutoCAD's default MIRROR behaviour).
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(2))
        .is_some());

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before,
        "one undo removes both mirrored copies"
    );
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(2))
        .is_some());
}

#[test]
fn mirror_still_refuses_kinds_it_cannot_reflect_honestly() {
    // Arc/Ellipse are mirrored now, but the flat-text domain has no reflection and
    // `Opaque` has no geometry this build understands: both still refuse the whole
    // command with the generic transform surface's named reason (and nothing is
    // written for the supported arc that shares the selection).
    let arc = SemanticGeometry::Arc {
        center: point(0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 5.0,
        start: 0.0,
        sweep: 1.0,
    };
    let mut app = line_and_boundary_app(ent(2, arc, 1), None);
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1), ref_of(2)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .expect("a LINE + ARC selection is supported now");
    // A kind the flat-text domain cannot reflect still refuses the whole command;
    // `Text` is the case the generic transform surface still rejects.
    let text = SemanticGeometry::Text {
        text: "A".into(),
        position: point(1.0, 1.0),
        style: cad_domain::StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
    };
    let mut app = line_and_boundary_app(ent(2, text, 1), None);
    let count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let rev = revision(&app);
    let error = execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(2)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .expect_err("TEXT cannot be mirrored");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("would mirror it"),
        "the generic refusal is kept, and it no longer names Arc/Ellipse: {error}"
    );
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        count,
        "nothing may be added when one kind is refused"
    );
}
