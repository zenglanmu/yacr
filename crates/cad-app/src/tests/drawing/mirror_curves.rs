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
    // A kind this build cannot reflect still refuses the whole command (Text is
    // now mirrored readably; `Shape` keeps the generic transform refusal).
    let shape = SemanticGeometry::Shape {
        shape_name: "BOX".into(),
        code: 1,
        position: point(1.0, 1.0),
        size: 1.0,
        rotation: 0.0,
        font: None,
    };
    let mut app = line_and_boundary_app(ent(2, shape, 1), None);
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
    .expect_err("SHAPE cannot be mirrored");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("Shape"),
        "the refusal names the kind: {error}"
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

// ---------------------------------------------------------------------------
// MIRRTEXT, Erase-source, and the dimension arm. These exercise the option
// entry point (`CommandPayload` has no variant for the options while it sits at
// its 1000-line ceiling).
// ---------------------------------------------------------------------------

use crate::command_payload::MirrorOptions;

fn mirror_with_options(
    app: &mut Application,
    options: &MirrorOptions,
) -> CadResult<CommandOutcome> {
    let command = command(
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: options.refs.clone(),
            axis_start: options.axis_start,
            axis_end: options.axis_end,
        },
    );
    app.mirror_entities_with_options(&command, options)
}

fn text_geometry(position: Point3, rotation: f64) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "ABC".into(),
        position,
        style: cad_domain::StyleId(0),
        height: 1.0,
        rotation,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
    }
}

#[test]
fn mirror_options_payload_reaches_the_option_entry_point_through_dispatch() {
    // `Erase source objects? Yes` through the **command dispatch** (the variant,
    // not the crate entry point) deletes the source in one undo step.
    let mut app = line_and_boundary_app(ent(2, text_geometry(point(2.0, 0.0), 0.3), 1), None);
    let options = MirrorOptions {
        refs: vec![ref_of(2)],
        axis_start: point(0.0, 0.0),
        axis_end: point(0.0, 1.0),
        mirrortext: false,
        erase_source: true,
    };
    let outcome = execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::MirrorEntities,
        CommandPayload::MirrorOptions(Box::new(options)),
    )
    .expect("MirrorOptions routes");
    assert_eq!(outcome.objects.len(), 1);
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(2))
            .is_none(),
        "erase_source reached the entry point"
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        1
    );

    // `MIRRTEXT = 1` is a named refusal through the same routed variant.
    let mut app = line_and_boundary_app(ent(2, text_geometry(point(2.0, 0.0), 0.3), 1), None);
    let options = MirrorOptions {
        refs: vec![ref_of(2)],
        axis_start: point(0.0, 0.0),
        axis_end: point(0.0, 1.0),
        mirrortext: true,
        erase_source: false,
    };
    let error = execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::MirrorEntities,
        CommandPayload::MirrorOptions(Box::new(options)),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("mirror.mirrortext_glyphs"),
        "{error:?}"
    );
}

#[test]
fn mirror_text_position_mirrors_and_the_run_stays_readable_by_default() {
    let mut app = line_and_boundary_app(ent(2, text_geometry(point(2.0, 0.0), 0.3), 1), None);
    let options = MirrorOptions {
        refs: vec![ref_of(2)],
        axis_start: point(0.0, 0.0),
        axis_end: point(0.0, 1.0),
        mirrortext: false,
        erase_source: false,
    };
    let outcome = mirror_with_options(&mut app, &options).unwrap();
    let copy = EntityId(outcome.objects[0].0);
    let SemanticGeometry::Text {
        text,
        position,
        rotation,
        ..
    } = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy)
        .unwrap()
        .geometry
        .clone()
    else {
        panic!("expected a mirrored TEXT");
    };
    assert_eq!(text, "ABC", "the string is never reversed at MIRRTEXT=0");
    assert_point_eq(position, point(-2.0, 0.0), "mirrored insertion point");
    // Readable: the reflected baseline (π − 0.3) is turned back by π, so the run
    // still faces the same way as the source.
    assert!(rotation.cos() * 0.3_f64.cos() > 0.0, "rotation {rotation}");
    assert!((rotation + 0.3).abs() < 1e-9, "rotation {rotation}");
}

#[test]
fn mirror_mirrortext_glyphs_is_a_named_refusal() {
    let mut app = line_and_boundary_app(ent(2, text_geometry(point(2.0, 0.0), 0.0), 1), None);
    let count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let rev = revision(&app);
    let options = MirrorOptions {
        refs: vec![ref_of(2)],
        axis_start: point(0.0, 0.0),
        axis_end: point(0.0, 1.0),
        mirrortext: true,
        erase_source: false,
    };
    let error = mirror_with_options(&mut app, &options).unwrap_err();
    match error {
        CadError::Unsupported(reason) => {
            assert!(reason.contains("mirror.mirrortext_glyphs"), "{reason}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        count
    );
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn mirror_erase_source_deletes_the_original_in_the_same_undo_step() {
    let mut app = line_and_boundary_app(ent(2, text_geometry(point(2.0, 0.0), 0.0), 1), None);
    let count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let options = MirrorOptions {
        refs: vec![ref_of(1), ref_of(2)],
        axis_start: point(0.0, 0.0),
        axis_end: point(0.0, 1.0),
        mirrortext: false,
        erase_source: true,
    };
    let outcome = mirror_with_options(&mut app, &options).unwrap();
    assert_eq!(outcome.objects.len(), 2, "two mirrored copies");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_count(),
        count,
        "erase source keeps the count"
    );
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_none());
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        1
    );
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), count);
    assert!(drawing.entity(EntityId(1)).is_some());
    assert!(drawing.entity(EntityId(2)).is_some());
}

fn linear_dimension_geometry() -> SemanticGeometry {
    let style = DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
        dim_line_increment: 3.75,
        text_vertical: 1,
        text_vertical_position: 0.0,
        suppress_ext1: false,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: false,
        linear_factor: 1.0,
        decimals: 4,
        rounding: 0.0,
        postfix: String::new(),
        zero_suppression: 0,
        linear_unit_format: 2,
        decimal_separator: 46,
        text_style_name: "Standard".into(),
        text_style: cad_domain::StyleId(0),
        text_font: None,
    };
    let definition = DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: point(1.0, 0.0),
            second: point(3.0, 0.0),
            dimension_line: point(2.0, -4.0),
        },
        text_middle: point(2.0, -4.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    };
    let dimension =
        DimensionGeometry::synthesized(definition, "Standard".into(), style, None).unwrap();
    SemanticGeometry::Dimension(Box::new(dimension))
}

#[test]
fn mirror_reflects_a_dimension_definition_and_rebuilds_its_display() {
    let mut app = line_and_boundary_app(ent(2, linear_dimension_geometry(), 1), None);
    let mirrored = mirror_one(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        MirrorAxis::Y,
    );
    let SemanticGeometry::Dimension(dimension) = mirrored else {
        panic!("expected a mirrored DIMENSION");
    };
    let DimensionPoints::Linear {
        first,
        second,
        dimension_line,
    } = &dimension.definition().points
    else {
        panic!("expected a linear definition");
    };
    assert_point_eq(*first, point(-1.0, 0.0), "first");
    assert_point_eq(*second, point(-3.0, 0.0), "second");
    assert_point_eq(*dimension_line, point(-2.0, -4.0), "dimension_line");
    assert!(!dimension.display().is_empty(), "display rebuilt");
}
