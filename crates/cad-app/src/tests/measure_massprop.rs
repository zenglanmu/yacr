//! `MASSPROP` contract tests (AutoCAD 2026 inquiry).
//!
//! Split out of `measure_extra.rs` to keep both files inside the code-size
//! limit. The shared database fixtures, the `app_with_objects` builder and the
//! `execute`/`select`/diagnostic helpers live in the sibling
//! [`super::measure_extra`] module; this module owns the `MASSPROP` contracts
//! only (the `MEASUREGEOM` ones stay there).
//!
//! Pinned here: the exact per-kind area/perimeter/bbox/centroid values, the exact
//! 2D second moments (`Ix`, `Iy`, `Ixy`, `J`, the radii of gyration and the
//! principal axes), the parallel-axis combination of a compound selection, the
//! sampled kinds' explicit approximation labels, every named refusal with zero
//! state change, and the fact that a region-only run still names the un-modelled
//! 3D properties. The exact `Body3d` solid path (ADR-0036 C3a) is the sibling
//! `tests::drawing::massprop_solid` module.

use super::measure_extra::*;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

#[test]
fn massprop_reports_a_circle_exactly_and_viewer_legal() {
    let (mut app, _) = app_with_objects();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    assert!(!CommandId::MassProp.requires_work_mode());
    let before = revision(&app);
    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("MASSPROP is Viewer-legal");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=12.566371"), "{object}");
    assert!(object.contains("perimeter=12.566371"), "{object}");
    assert!(
        object.contains("bbox min=(-2.000000, -2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(2.000000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("centroid=(0.000000, 0.000000, 0.000000)"),
        "{object}"
    );
    // The disc's second moments are exact: Ix = Iy = pi r^4 / 4 = 4pi, J = 8pi
    // and kx = ky = r/2 = 1.
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    assert!(
        inertia.contains("Ix=12.566371 Iy=12.566371 Ixy=0.000000 J=25.132741"),
        "{inertia}"
    );
    assert!(inertia.contains("kx=1.000000 ky=1.000000"), "{inertia}");
    assert!(inertia.contains("exact closed form"), "{inertia}");
    // A region run still names the 3D properties it does not model; an exact
    // body is measured by its own selection path (see
    // `tests::drawing::massprop_solid`), never by this 2D summary.
    let solid = diagnostic_text(&outcome, "massprop.solid_unsupported");
    for named in ["volume", "mass", "density", "3D moments of inertia"] {
        assert!(solid.contains(named), "{named} in {solid}");
    }
    let not_implemented = diagnostic_text(&outcome, "massprop.not_implemented");
    assert!(
        not_implemented.contains("Add/Subtract region composition"),
        "{not_implemented}"
    );
    assert!(
        not_implemented.contains("NOT IMPLEMENTED"),
        "{not_implemented}"
    );
    assert!(
        !not_implemented.contains("moments/products of inertia"),
        "the 2D moments are implemented now: {not_implemented}"
    );
    let summary = diagnostic_text(&outcome, "massprop.summary");
    assert!(summary.contains("1 region(s)"), "{summary}");
    let total = diagnostic_text(&outcome, "massprop.summary_inertia");
    assert!(
        total.contains("Ix=12.566371 Iy=12.566371 Ixy=0.000000 J=25.132741"),
        "{total}"
    );
    assert!(total.contains("principal I1=12.566371"), "{total}");
    assert!(total.contains("I2=12.566371"), "{total}");
    // The reported frame is the selection plane's OCS basis, printed with it.
    assert!(
        total.contains("u=(1.000000, 0.000000, 0.000000)"),
        "{total}"
    );
    assert!(
        total.contains("v=(0.000000, 1.000000, 0.000000)"),
        "{total}"
    );
    assert!(
        outcome.measurement.is_none(),
        "no scalar measurement record"
    );
    assert!(outcome.point_report.is_none());
    assert!(outcome.changes.is_none());
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_ellipse_perimeter_is_labelled_approximate() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[ELLIPSE_3X1]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a full ellipse is supported");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=9.424778"), "{object}");
    assert!(
        object.contains("perimeter=~"),
        "the ellipse perimeter is approximate: {object}"
    );
    let approx = diagnostic_text(&outcome, "massprop.approx_ellipse_perimeter");
    assert!(approx.contains("Ramanujan"), "{approx}");
    // The second moments are exact even though the perimeter is not:
    // Ix = pi*a*b^3/4 = 3pi/4 (about the minor-axis-parallel one, i.e. ∫y²) and
    // Iy = pi*a^3*b/4 = 27pi/4, with Ixy = 0 in the ellipse's own frame.
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    let (ix, iy) = (
        std::f64::consts::PI * 3.0 / 4.0,
        std::f64::consts::PI * 27.0 / 4.0,
    );
    assert!(
        inertia.contains(&format!("Ix={ix:.6} Iy={iy:.6} Ixy=0.000000")),
        "{inertia}"
    );
    assert!(inertia.contains("exact closed form"), "{inertia}");
    assert!(
        object.contains("bbox min=(7.000000, -1.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(13.000000, 1.000000, 0.000000)"),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_closed_polyline_area_perimeter_and_centroid_are_exact() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a closed straight polyline is supported");
    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=12.000000"), "{object}");
    assert!(object.contains("perimeter=14.000000"), "{object}");
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_bulged_polyline_uses_the_exact_arc_area_length_and_centroid() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[BULGED_RECT]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a bulged closed polyline is supported");
    let object = diagnostic_text(&outcome, "massprop.object");
    // 3x4 chord ring + a semicircular cap (r = 1.5): 12 + pi*1.5^2/2.
    let area = 12.0 + std::f64::consts::PI * 1.5 * 1.5 / 2.0;
    assert!(object.contains(&format!("area={area:.6}")), "{object}");
    // Straight edges 4 + 3 + 4 = 11 plus the semicircle's pi*1.5.
    let perimeter = 11.0 + std::f64::consts::PI * 1.5;
    assert!(
        object.contains(&format!("perimeter={perimeter:.6}")),
        "{object}"
    );
    // The cap bows outside the bottom edge, so the bounding box reaches y = -1.5
    // (a vertex-only box would stop at y = 0).
    assert!(
        object.contains("bbox min=(0.000000, -1.500000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(3.000000, 4.000000, 0.000000)"),
        "{object}"
    );
    // Area-weighted centroid: (12*(1.5,2) + 1.5^2*pi/2*(1.5,-4r/3pi)) / area.
    let cap_area = std::f64::consts::PI * 1.5 * 1.5 / 2.0;
    let y = (12.0 * 2.0 + cap_area * (-4.0 * 1.5 / (3.0 * std::f64::consts::PI))) / area;
    assert!(
        object.contains(&format!("centroid=(1.500000, {y:.6}, 0.000000)")),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_summary_combines_coplanar_regions_and_refuses_non_coplanar_ones() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2, RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("both regions are on z = 0");
    let summary = diagnostic_text(&outcome, "massprop.summary");
    let total = std::f64::consts::PI * 4.0 + 12.0;
    assert!(summary.contains("2 region(s)"), "{summary}");
    assert!(summary.contains(&format!("area={total:.6}")), "{summary}");

    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2, CIRCLE_HIGH]).unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("z = 0 and z = 5 are not coplanar");
    let text = format!("{error:?}");
    assert!(text.contains("massprop.not_coplanar"), "{text}");
}

#[test]
fn massprop_names_every_unsupported_input() {
    let (mut app, mut session) = app_with_objects();
    let empty = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an empty selection is refused");
    assert!(
        format!("{empty:?}").contains("massprop.selection"),
        "{empty:?}"
    );

    select(&mut app, &mut session, &[OPEN_POLYLINE]).unwrap();
    let open = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an open polyline encloses no area");
    assert!(
        format!("{open:?}").contains("massprop.open_polyline"),
        "{open:?}"
    );

    select(&mut app, &mut session, &[ARC]).unwrap();
    let arc = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an arc is an open curve");
    assert!(format!("{arc:?}").contains("massprop.arc"), "{arc:?}");

    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let payload = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .expect_err("MASSPROP takes no payload");
    assert!(
        format!("{payload:?}").contains("massprop.invalid_payload"),
        "{payload:?}"
    );

    // A block-instance member has no addressable entity identity.
    let refs = vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(CIRCLE_R2),
        instance: InstancePath(vec![EntityId(99)]),
        sub_element: None,
    }];
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(refs),
    )
    .unwrap();
    let not_addressable = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("a block member is not addressable");
    assert!(
        format!("{not_addressable:?}").contains("massprop.not_addressable"),
        "{not_addressable:?}"
    );
}

// --- MASSPROP region extension (2026-11-12) ------------------------------

#[test]
fn massprop_closed_spline_is_a_labelled_sample_ring() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[SPLINE_SQUARE]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a closed spline is measured through its sample ring");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("(closed spline (sampled))"), "{object}");
    // The degree-1 control net closes on itself, so the sample ring is exactly the
    // 3x4 rectangle: 12 area, 14 perimeter, centroid in its centre.
    assert!(object.contains("area=12.000000"), "{object}");
    assert!(
        object.contains("perimeter=~14.000000"),
        "the sampled perimeter is labelled approximate: {object}"
    );
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("bbox min=(0.000000, 0.000000, 0.000000)"),
        "{object}"
    );
    let note = diagnostic_text(&outcome, "massprop.approx_spline_region");
    assert!(note.contains("sample ring"), "{note}");
    assert!(note.contains("not closed-form"), "{note}");
    assert!(
        note.contains("second moments"),
        "the note covers the second moments too: {note}"
    );
    // The degree-1 sample ring is exactly the 3x4 rectangle, so its second
    // moments are the rectangle's — but they are labelled as the sample ring's,
    // because the curve itself has no closed form here.
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    assert!(
        inertia.contains("Ix=16.000000 Iy=9.000000 Ixy=0.000000"),
        "{inertia}"
    );
    assert!(
        inertia.contains("the sample polygon's values, not closed-form"),
        "{inertia}"
    );
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.approx_ellipse_perimeter"),
        "a spline must not borrow the ellipse's approximation note"
    );
    // The summary counts it as approximate because its perimeter is sampled.
    let summary = diagnostic_text(&outcome, "massprop.summary");
    assert!(summary.contains("perimeter=~"), "{summary}");
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_refuses_an_open_or_unevaluable_spline_by_name() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);

    select(&mut app, &mut session, &[SPLINE_OPEN]).unwrap();
    let open = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an open spline encloses no region");
    assert!(
        format!("{open:?}").contains("massprop.open_spline"),
        "{open:?}"
    );
    assert_no_write(&app, &session, before);

    // A degree-0 spline cannot be evaluated; MASSPROP refuses instead of falling
    // back to a clamped-uniform curve (the display tessellator's fallback).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(spline(64, vec![point(0.0, 0.0), point(1.0, 0.0)], 0))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([72u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    select(&mut app, &mut session, &[64]).unwrap();
    let invalid = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("a degree-0 spline has no evaluable curve");
    assert!(
        format!("{invalid:?}").contains("massprop.spline_invalid"),
        "{invalid:?}"
    );
}

#[test]
fn massprop_hatch_region_is_the_exact_net_loop_area() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[HATCH_CIRCLE]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a hatch region is measured from its own loops");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("(hatch region)"), "{object}");
    // Four exact quarter arcs of r = 1: area pi, perimeter 2*pi.
    assert!(
        object.contains(&format!("area={:.6}", std::f64::consts::PI)),
        "{object}"
    );
    assert!(
        object.contains(&format!("perimeter={:.6}", std::f64::consts::TAU)),
        "{object}"
    );
    assert!(
        object.contains("bbox min=(4.000000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(6.000000, 4.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("centroid=(5.000000, 3.000000, 0.000000)"),
        "{object}"
    );
    // Four exact quarter arcs of r = 1 about (5, 3): a disc's second moments are
    // Ix = Iy = pi r^4 / 4 = pi/4 about every centroidal axis, so the loop data is
    // exact and no approximation note may appear.
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    let quarter = std::f64::consts::PI / 4.0;
    assert!(
        inertia.contains(&format!("Ix={quarter:.6} Iy={quarter:.6}")),
        "{inertia}"
    );
    assert!(inertia.contains("exact closed form"), "{inertia}");
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.approx_hatch_boundary"),
        "{:?}",
        outcome.diagnostics
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_hatch_subtracts_islands_by_nesting_parity_not_winding() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[HATCH_ISLAND]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("the two-loop hatch is a region");

    let object = diagnostic_text(&outcome, "massprop.object");
    // Both loops are written counter-clockwise; nesting parity still subtracts
    // the 2x2 island from the 3x4 rectangle: 12 - 2 = 10.
    assert!(object.contains("area=10.000000"), "{object}");
    // The island subtracts its own second moment about its own centroid:
    // Ix = (3*4^3 - 1*2^3)/12 = 16 - 2/3 and Iy = (4*3^3 - 2*1^3)/12 = 9 - 1/6,
    // which is not the same as ignoring the hole or as subtracting it about the
    // region's centroid.
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    assert!(
        inertia.contains(&format!(
            "Ix={:.6} Iy={:.6}",
            16.0 - 2.0 / 3.0,
            9.0 - 1.0 / 6.0
        )),
        "{inertia}"
    );
    // Only the loop carrying the external flag contributes to the perimeter.
    assert!(object.contains("perimeter=14.000000"), "{object}");
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.hatch_perimeter_rule"),
        "{:?}",
        outcome.diagnostics
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_hatch_names_a_sampled_import_boundary_as_approximate() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[HATCH_SAMPLED]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("the imported hatch is still a region");
    let note = diagnostic_text(&outcome, "massprop.approx_hatch_boundary");
    assert!(note.contains("sampled"), "{note}");
    assert!(note.contains("not exact"), "{note}");
    // The fixture's loops carry no external flag bit, so the outer-boundary rule
    // is stated instead of guessed silently.
    let rule = diagnostic_text(&outcome, "massprop.hatch_perimeter_rule");
    assert!(rule.contains("largest-area loop"), "{rule}");
}

// --- MASSPROP 2D second moments (2026-11-15) ------------------------------

#[test]
fn massprop_rectangle_second_moments_are_exact() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a closed polyline is a region");

    // 3 (x) by 4 (y), so Ix = b*h^3/12 = 16, Iy = h*b^3/12 = 9, Ixy = 0,
    // J = 25, kx = sqrt(16/12), ky = sqrt(9/12).
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    assert!(
        inertia.contains("Ix=16.000000 Iy=9.000000 Ixy=0.000000 J=25.000000"),
        "{inertia}"
    );
    assert!(inertia.contains("kx=1.154701 ky=0.866025"), "{inertia}");
    assert!(inertia.contains("exact closed form"), "{inertia}");

    let total = diagnostic_text(&outcome, "massprop.summary_inertia");
    assert!(
        total.contains("Ix=16.000000 Iy=9.000000 Ixy=0.000000 J=25.000000"),
        "{total}"
    );
    // The centroidal axes of a 3x4 rectangle are the frame axes: the larger
    // principal moment (16) belongs to the axis parallel to u.
    assert!(
        total.contains("principal I1=16.000000 axis1=(1.000000, 0.000000, 0.000000)"),
        "{total}"
    );
    assert!(
        total.contains("I2=9.000000 axis2=(0.000000, 1.000000, 0.000000)"),
        "{total}"
    );
    assert!(total.contains("principal_angle_deg=0.000000"), "{total}");
    assert!(total.contains("kz=1.443376"), "{total}");
}

#[test]
fn massprop_rotated_rectangle_reports_the_exact_principal_axes() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[ROTATED_RECT]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("the rotated rectangle is a region");

    // Body-frame moments 9 and 16 rotate into Ix = Iy = 12.5 and
    // Ixy = (9 - 16)/2 = -3.5 (the product changes sign with the +45 rotation).
    let inertia = diagnostic_text(&outcome, "massprop.inertia");
    assert!(
        inertia.contains("Ix=12.500000 Iy=12.500000 Ixy=-3.500000 J=25.000000"),
        "{inertia}"
    );

    let total = diagnostic_text(&outcome, "massprop.summary_inertia");
    // The principal moments are rotation invariants; the principal axis of the
    // larger one is a quarter turn from the long side, i.e. along +45 degrees.
    assert!(total.contains("principal I1=16.000000"), "{total}");
    assert!(total.contains("I2=9.000000"), "{total}");
    assert!(
        total.contains("axis1=(0.707107, 0.707107, 0.000000)"),
        "{total}"
    );
    assert!(total.contains("principal_angle_deg=45.000000"), "{total}");
}

#[test]
fn massprop_compound_selection_uses_the_parallel_axis_theorem() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2, RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a disc and a rectangle on z = 0 are coplanar regions");

    // Explicit area-weighted combination: the disc (centroid (0,0), Ix = Iy =
    // 4*pi) and the rectangle (centroid (1.5, 2), Ix = 16, Iy = 9), each shifted
    // onto the combined centroid by the parallel-axis theorem.
    let circle_area = std::f64::consts::PI * 4.0;
    let rect_area = 12.0;
    let total_area = circle_area + rect_area;
    let cx = rect_area * 1.5 / total_area;
    let cy = rect_area * 2.0 / total_area;
    let disc = std::f64::consts::PI * 16.0 / 4.0;
    let ix = disc + circle_area * cy * cy + 16.0 + rect_area * (2.0 - cy).powi(2);
    let iy = disc + circle_area * cx * cx + 9.0 + rect_area * (1.5 - cx).powi(2);
    let ixy = circle_area * cx * cy + rect_area * (1.5 - cx) * (2.0 - cy);

    let total = diagnostic_text(&outcome, "massprop.summary_inertia");
    assert!(
        total.contains(&format!(
            "Ix={ix:.6} Iy={iy:.6} Ixy={ixy:.6} J={:.6}",
            ix + iy
        )),
        "{total}"
    );
    assert!(
        total.contains(&format!(
            "about the combined centroid ({cx:.6}, {cy:.6}, 0.000000)"
        )),
        "{total}"
    );
    // The per-object lines are in the same frame, so their values are the ones
    // the combination above used.
    let per_object: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "massprop.inertia")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(per_object.len(), 2, "{per_object:?}");
    assert!(
        per_object[0].contains("entity 1 (circle)"),
        "{per_object:?}"
    );
    assert!(
        per_object[1].contains("Ix=16.000000 Iy=9.000000 Ixy=0.000000"),
        "{per_object:?}"
    );
}

#[test]
fn massprop_refuses_a_zero_area_region_by_name() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[TINY_CIRCLE]).unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("a radius whose square underflows encloses no area");
    let text = format!("{error:?}");
    assert!(text.contains("massprop.degenerate"), "{text}");
    assert!(!text.contains("NaN"), "no NaN is ever reported: {text}");
}
