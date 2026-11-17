//! Chain evidence for `BrepCurve::Arc`: the lift gate, and what the pinned kernel
//! does with an arc-bounded body.
//!
//! Evidence boundary: Linux x86-64, this repository's test profile, the real
//! pinned kernel (no mock). No GPU, no host, no other platform (ADR-0036).
//!
//! An arc lies in exactly one plane, so an all-planar shell containing an arc is
//! necessarily a **zero-thickness** double-sided face. The kernel refuses such a
//! shell (status 1001, its generic `SM_FAILURE`), and the control below shows an
//! all-line zero-thickness shell is refused identically — so the refusal is about
//! the shell, not about the arc conversion. P2.1b (curved faces) is the first
//! body that can exercise a real solid.

mod common;

use cad_domain::{BrepCurve, BrepSurface};
use cad_kernel_adapter::{
    BooleanOp, BrepTessellator, GeometryHandle, PrimitiveSpec, SolidExchange, SolidOperation,
    SolidOutcome, SolidTessellator, TessellationBudget, TessellationRequest, TessellationTolerance,
};
use cad_kernel_solidcore::SolidCoreModeler;

use common::*;

/// The disc in `z = 0` as two coplanar faces sharing two half-arc edges.
///
/// An arc lies in exactly one plane, so **every** face on either side of it lies
/// in that plane too: an all-planar shell with arc edges is necessarily
/// zero-thickness. A real solid with an arc always has a curved neighbour, which
/// is what P2.1b lifts. This body is nevertheless the exact shape the arc
/// conversion must carry, so the chain tests below pin what the kernel does with
/// it.
fn arc_disc() -> cad_domain::BrepData {
    let half = |start: [f64; 2], end: [f64; 2]| BrepCurve::Arc {
        center: p(0.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius: 1.0,
        start: p(start[0], start[1], 0.0),
        end: p(end[0], end[1], 0.0),
    };
    let loops = vec![cad_domain::BrepLoop {
        edges: vec![half([1.0, 0.0], [-1.0, 0.0]), half([-1.0, 0.0], [1.0, 0.0])],
    }];
    let face = |id: u32, reversed: bool| cad_domain::BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed,
        loops: loops.clone(),
        domain: None,
        tolerance: None,
    };
    cad_domain::BrepData {
        shells: vec![cad_domain::BrepShell {
            id: 0,
            faces: vec![face(0, false), face(1, true)],
        }],
        placement: None,
        tolerance: None,
    }
}

#[test]
fn an_arc_bounded_body_clears_the_lift_gate_and_the_kernel_refuses_only_the_degenerate_shell() {
    // The arc conversion is what was missing: mass properties and a boolean must
    // now get *past* the lift and be refused (if at all) by the kernel, with a
    // kernel code — never `solid.lift_*`. Status 1001 is the kernel's generic
    // `SM_FAILURE`, so this test *does not* establish why the body was refused;
    // `a_zero_thickness_linear_shell_fails_the_same_way_as_the_arc_one` supplies
    // the control that isolates the shell's degeneracy from the arc.
    let modeler = SolidCoreModeler::new();
    let brep = arc_disc();
    let mass = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &brep,
            density: 1.0,
        },
    );
    assert!(mass.mass().is_none(), "a degenerate shell is not a mass");
    assert!(
        mass.unsupported().is_none(),
        "the arc must not be a conversion refusal: {mass:?}"
    );
    let SolidOutcome::Failed { diagnostics } = &mass else {
        panic!("expected the kernel's own failure, got {mass:?}");
    };
    assert_eq!(diagnostics[0].code, cad_kernel_solidcore::KERNEL_FAILED);

    let other = arc_disc();
    let union = execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Union,
            left: &brep,
            right: &other,
        },
    );
    assert!(union.body().is_none());
    assert!(
        union.unsupported().is_none(),
        "the boolean must reach the kernel, not refuse the arc: {union:?}"
    );
    let SolidOutcome::Failed { diagnostics } = &union else {
        panic!("expected the kernel's own failure, got {union:?}");
    };
    assert_eq!(diagnostics[0].code, cad_kernel_solidcore::KERNEL_FAILED);
}

/// Inverted from `an_arc_bounded_cylinder_side_face_still_refuses_the_curved_lift`.
///
/// A **trimmed** cylinder is the body that used to be impossible: its lateral
/// face is bounded by two half-arcs and two generators, not by two full circles.
/// It now lifts (P2.1b part 2), so it is measurable against a closed form — and it
/// still **displays** as `Partial`, because the neutral tessellator draws a
/// cylinder strip only from two full circles and this build meshes no NURBS face.
/// Both halves of that statement are asserted here; neither is inferred from the
/// other.
#[test]
fn an_arc_bounded_cylinder_side_face_lifts_and_measures_a_half_cylinder() {
    let modeler = SolidCoreModeler::new();
    let cylinder = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        },
    ));
    // Remove everything below `y = 0`: the material left is the half `y >= 0`.
    let cutter = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(-2.0, -2.0, -1.0), 4.0, 2.0, 4.0),
        },
    ));
    let half = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &cylinder,
            right: &cutter,
        },
    ));

    // The lateral face really is arc-bounded: an open circle on the cylinder.
    let lateral = faces(&half)
        .find(|face| matches!(face.surface, BrepSurface::Cylinder { .. }))
        .expect("the half cylinder keeps its lateral face");
    assert!(
        lateral
            .loops
            .iter()
            .flat_map(|loop_| &loop_.edges)
            .any(|curve| matches!(curve, BrepCurve::Arc { .. })),
        "the lateral face is bounded by arcs: {:?}",
        lateral.loops
    );
    // Inverted from the old reader: a trimmed side face is *not* normalised into
    // one full-circle loop per boundary, because its boundary is not two full
    // circles. It maps literally — one loop of arcs and generators — which is what
    // lifts (and what the display tessellator then refuses, below).
    assert_eq!(
        lateral.loops.len(),
        1,
        "the trimmed band maps as one literal loop: {:?}",
        lateral.loops
    );
    assert!(
        lateral.loops[0].edges.len() > 2,
        "arcs and generators, not a ring: {:?}",
        lateral.loops[0]
    );

    // Closed forms of a half cylinder: V = pi r^2 h / 2 with r = 1, h = 2; the
    // area is the curved half (pi r h) + the flat rectangle (2 r h) + two half
    // discs (pi r^2); the centroid sits at y = 4r/3pi, z = h/2.
    let measured = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &half,
            density: 1.0,
        },
    );
    let mass = measured
        .mass()
        .unwrap_or_else(|| panic!("a trimmed cylinder must lift and measure"));
    let pi = std::f64::consts::PI;
    assert!(
        (mass.volume - pi).abs() <= 1e-6 * pi,
        "half-cylinder volume {} != {pi}",
        mass.volume
    );
    let area = 3.0 * pi + 4.0;
    assert!(
        (mass.surface_area - area).abs() <= 1e-6 * area,
        "half-cylinder area {} != {area}",
        mass.surface_area
    );
    assert!(
        (mass.centroid.y - 4.0 / (3.0 * pi)).abs() <= 1e-6,
        "centroid {:?}",
        mass.centroid
    );
    assert!(
        (mass.centroid.z - 1.0).abs() <= 1e-6 && mass.centroid.x.abs() <= 1e-6,
        "centroid {:?}",
        mass.centroid
    );

    // Display stays honest: the lateral face cannot be meshed as a strip, and the
    // result says which face is missing and why.
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(cad_domain::ObjectId(1)),
        exchange: SolidExchange::Brep(half.clone()),
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: stamp(),
    };
    let result = BrepTessellator
        .tessellate(&request, &never())
        .expect("tessellation answers");
    let cad_kernel_adapter::TessellationOutcome::Partial { degradation, .. } = &result.outcome
    else {
        panic!(
            "a trimmed lateral face must be a partial display, got {:?}",
            result.outcome
        );
    };
    let missing = degradation
        .missing_faces
        .iter()
        .find(|face| face.id == lateral.id)
        .unwrap_or_else(|| panic!("the lateral face is named missing: {degradation:?}"));
    assert!(
        missing.reason.contains("circle") || missing.reason.contains("cylinder"),
        "the reason names the gap: {}",
        missing.reason
    );
}

/// The **control** for the refusal above: the same zero-thickness sandwich built
/// from **lines only** (two coplanar square faces sharing four edges).
///
/// It has no arc anywhere, yet the pinned kernel refuses it exactly like the arc
/// disc. So status 1001 is the kernel's generic `SM_FAILURE` for a degenerate
/// (zero-volume) shell, and the arc-bearing body's refusal says nothing about the
/// arc conversion — which is the point the arc test had over-claimed.
fn flat_square() -> cad_domain::BrepData {
    // Counter-clockwise in the face plane, which the record's loop rule requires.
    let corners = [
        p(1.0, 0.0, 0.0),
        p(1.0, 2.0, 0.0),
        p(-1.0, 2.0, 0.0),
        p(-1.0, 0.0, 0.0),
    ];
    let loop_ = cad_domain::BrepLoop {
        edges: (0..4)
            .map(|index| BrepCurve::Line {
                start: corners[index],
                end: corners[(index + 1) % 4],
            })
            .collect(),
    };
    let face = |id: u32, reversed: bool| cad_domain::BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed,
        loops: vec![loop_.clone()],
        domain: None,
        tolerance: None,
    };
    cad_domain::BrepData {
        shells: vec![cad_domain::BrepShell {
            id: 0,
            faces: vec![face(0, false), face(1, true)],
        }],
        placement: None,
        tolerance: None,
    }
}

#[test]
fn a_zero_thickness_linear_shell_fails_the_same_way_as_the_arc_one() {
    let modeler = SolidCoreModeler::new();
    let control = flat_square();
    let mass = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &control,
            density: 1.0,
        },
    );
    assert!(mass.mass().is_none(), "a zero-thickness shell has no mass");
    assert!(
        mass.unsupported().is_none(),
        "the lift gate accepts an all-line planar shell: {mass:?}"
    );
    let SolidOutcome::Failed { diagnostics } = &mass else {
        panic!("expected the kernel's own failure, got {mass:?}");
    };
    // The very same code and upstream status as the arc disc: the refusal is about
    // the zero-volume shell, not about the arc.
    assert_eq!(diagnostics[0].code, cad_kernel_solidcore::KERNEL_FAILED);
    assert!(
        diagnostics[0].message.contains("1001"),
        "the control must reproduce the kernel's generic status: {}",
        diagnostics[0].message
    );

    // And the arc disc carries the identical diagnostic, which is what makes the
    // control conclusive.
    let arc = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &arc_disc(),
            density: 1.0,
        },
    );
    let SolidOutcome::Failed {
        diagnostics: arc_diagnostics,
    } = &arc
    else {
        panic!("expected the kernel's own failure, got {arc:?}");
    };
    assert_eq!(arc_diagnostics[0].code, diagnostics[0].code);
    assert_eq!(arc_diagnostics[0].message, diagnostics[0].message);
}
