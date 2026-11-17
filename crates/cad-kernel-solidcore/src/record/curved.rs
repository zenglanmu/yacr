//! The curved half of `BrepData → record`: a labelled analytic face's loop
//! geometry, its seam, and the resolution of a full circle's traversal.
//!
//! The reader normalises two shapes the record cannot express as they are:
//!
//! - a cylinder's or a cone's lateral face comes back as **one loop per full
//!   circle** (the seam lines dropped), which is what the display tessellator
//!   understands;
//! - a full sphere or torus comes back **loop-less** (its seam dropped).
//!
//! The record needs the opposite: every edge used exactly twice by chained
//! loops. So the seam is reconstructed here from the surface and its boundary —
//! the band `[lower circle, seam, upper circle, seam]` for a cylinder/cone, the
//! meridian for a sphere, the tube circle plus the outer equator for a torus.
//! Nothing is invented about the *geometry*: the seam points are the circles' own
//! reference points, and the seam of a band must be a straight generator of the
//! surface or the face is refused by name.
//!
//! A closed circle edge has one vertex, so its two uses cannot be told apart by
//! their ends; [`resolve_closed_circles`] settles them from the planar face on
//! the other side (whose loop runs counter-clockwise about its own normal) and
//! refuses a circle shared by two non-planar faces instead of guessing.
//!
//! # Two invariants of the reconstructed loops (read before touching the order)
//!
//! **The torus seam is an oriented square.** `tube(−), equator(+), tube(+),
//! equator(−)` traces the `(u, v)` square counter-clockwise, with area exactly
//! `4π²` — it is what makes the torus's outward orientation the shell's. Flip one
//! `forward` and the signed area flips with it: the kernel then reports a
//! disagreeing topology, and the failure looks like a kernel bug rather than an
//! ordering mistake. The four flags are not interchangeable.
//!
//! **The sphere seam is a zero-area bigon.** Both uses are the same meridian, so
//! the loop encloses nothing; what makes it valid is the **pole jump**: the two
//! uses' `u` values differ by exactly one period. That period is only placeable
//! because `super::surfaces::label_box` gives a sphere the full turn `[0, 2π]` —
//! `pcurves_within` places the jump by the box, so a "tightened" box would put
//! both uses in the same place and the kernel would refuse the face. Never narrow
//! that box; it is a **bounding** box (wider than the patch by construction), not
//! a tight one.

use std::collections::HashMap;

use cad_domain::{BrepCurve, BrepData, BrepFace, BrepSurface, Point3};
use solidcore_record::brep::{Brep as Record, Curve, Frame, Surface};

use super::arc::{authored_segment, authored_with_seam};
use super::loops::{
    domain, write_loops, ArcEdge, CircleEdge, PlaneFrame, RawUse, Segment, APEX_WITHOUT_CURVE,
};
use super::surfaces::{self, CurvedKind, Written};
use super::units::point;
use super::{add, cross, dot, is_axis, length, mul, normalize, sub};
use crate::error::LiftRefusal;

pub(super) struct Pending {
    pub(super) surface: Surface,
    pub(super) nurbs: Option<Surface>,
    pub(super) domain: [[f64; 2]; 2],
    /// The plane frame the `(u, v)` pcurves are projected into; `None` for a
    /// non-planar face, whose pcurves the kernel computes.
    pub(super) plane: Option<PlaneFrame>,
    pub(super) raw: Vec<Vec<RawUse>>,
}

impl Pending {
    /// The surface whose parameters this face's `(u, v)` are in: its form when it
    /// has one, else its label (`record::Face::parameters`).
    pub(super) fn parameters(&self) -> &Surface {
        self.nurbs.as_ref().unwrap_or(&self.surface)
    }
}

/// Write one face's surface, `(u, v)` box and loop **geometry** (no pcurves).
#[allow(clippy::too_many_arguments)] // the two edge/figure maps and the two dedup lists are all mutable borrows of the record walk
pub(super) fn write_face_geometry(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    written: Written,
) -> Result<Pending, LiftRefusal> {
    match written {
        Written::Plane { surface, frame } => {
            let (raw, corners) = write_loops(record, vertex_of, edge_of, arcs, circles, face)?;
            let domain = domain(frame.origin, frame.u_axis, frame.v_axis, &corners);
            Ok(Pending {
                surface,
                nurbs: None,
                domain,
                plane: Some(frame),
                raw,
            })
        }
        Written::Labelled { label, kind } => {
            let raw = labelled_loops(
                record, vertex_of, edge_of, arcs, circles, face, &label, kind,
            )?;
            let box_ = surfaces::label_box(face, &label).ok_or_else(|| {
                LiftRefusal::unsupported(format!(
                    "face {} is a {} surface whose boundary does not pin a positive (u, v) box, \
                     so no exact form can be written for it",
                    face.id,
                    kind.name()
                ))
            })?;
            let (nurbs, domain) = surfaces::form_of(face, &label, box_)?;
            Ok(Pending {
                surface: label,
                nurbs: Some(nurbs),
                domain,
                plane: None,
                raw,
            })
        }
        Written::Form { label } => {
            let domain = face.domain.ok_or_else(|| {
                LiftRefusal::unsupported(format!(
                    "face {} is a NURBS surface with no (u, v) domain",
                    face.id
                ))
            })?;
            let (raw, _) = write_loops(record, vertex_of, edge_of, arcs, circles, face)?;
            Ok(Pending {
                surface: label,
                nurbs: None,
                domain,
                plane: None,
                raw,
            })
        }
    }
}

/// The loop geometry of a labelled curved face.
///
/// A **ring** face (a cylinder's or a cone's side) is written as the record's
/// band: one loop of `[lower circle, seam, upper circle, seam]`, the seam being
/// the generator at the circles' own reference direction, traversed up one side
/// and down the other. The reader normalises exactly this shape back into one
/// full-circle loop per boundary (which is what the display tessellator needs),
/// so the neutral body never carries the seam; the seam is reconstructed here
/// from the two circles. Any other boundary (an arc-bounded patch) is written
/// literally, one record edge per authored curve.
///
/// An untrimmed **sphere** or **torus** has no loops at all (the reader drops
/// its seam), so its seam is reconstructed: the sphere's meridian from the south
/// pole to the north pole, used twice; the torus's tube circle at longitude zero
/// and its outer equator, each used twice.
#[allow(clippy::too_many_arguments)]
fn labelled_loops(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    label: &Surface,
    kind: CurvedKind,
) -> Result<Vec<Vec<RawUse>>, LiftRefusal> {
    match kind {
        CurvedKind::Cylinder | CurvedKind::Cone => {
            if let Some(raw) =
                ring_loops(record, vertex_of, edge_of, arcs, circles, face, label, kind)?
            {
                return Ok(vec![raw]);
            }
            let (raw, _) = write_loops(record, vertex_of, edge_of, arcs, circles, face)?;
            Ok(raw)
        }
        CurvedKind::Sphere => {
            if !face.loops.is_empty() {
                let (raw, _) = write_loops(record, vertex_of, edge_of, arcs, circles, face)?;
                return Ok(raw);
            }
            sphere_seam(record, vertex_of, edge_of, arcs, circles, face, label)
        }
        CurvedKind::Torus => {
            if !face.loops.is_empty() {
                let (raw, _) = write_loops(record, vertex_of, edge_of, arcs, circles, face)?;
                return Ok(raw);
            }
            torus_seam(record, vertex_of, edge_of, arcs, circles, face, label)
        }
    }
}

/// The record band loop of a cylinder/cone side face read in its normalised
/// two-circle form, or `None` when the face is not that shape.
#[allow(clippy::too_many_arguments)]
fn ring_loops(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    label: &Surface,
    kind: CurvedKind,
) -> Result<Option<Vec<RawUse>>, LiftRefusal> {
    let (origin, axis) = match label {
        Surface::Cylinder { frame, .. } | Surface::Cone { frame, .. } => {
            let (origin, _, z) = frame_fields(frame);
            (origin, z)
        }
        _ => return Ok(None),
    };
    // A band is one or two full circles, plus (for a full cone) the reader's
    // degenerate apex marker, which is a loop with no curve at all.
    let mut ring: Vec<BrepCurve> = Vec::with_capacity(2);
    let mut has_apex = false;
    for loop_ in &face.loops {
        match loop_.edges.as_slice() {
            [only] if matches!(only, BrepCurve::Circle { .. }) => ring.push(only.clone()),
            [BrepCurve::Unsupported { type_key }] if type_key == APEX_WITHOUT_CURVE => {
                has_apex = true
            }
            _ => return Ok(None),
        }
    }
    match ring.as_slice() {
        [first, second] if !has_apex => band_loops(
            record, vertex_of, edge_of, arcs, circles, face, kind, origin, axis, first, second,
        ),
        [base] if has_apex => apex_band(
            record, vertex_of, edge_of, arcs, circles, face, kind, label, origin, axis, base,
        ),
        _ => Ok(None),
    }
}

/// The record band of a two-circle cylinder/cone side face.
#[allow(clippy::too_many_arguments)]
fn band_loops(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    kind: CurvedKind,
    origin: Point3,
    axis: Point3,
    first: &BrepCurve,
    second: &BrepCurve,
) -> Result<Option<Vec<RawUse>>, LiftRefusal> {
    let level = |curve: &BrepCurve| match curve {
        BrepCurve::Circle { center, .. } => dot(sub(*center, origin), axis),
        _ => f64::NAN,
    };
    let (lower, upper) = if level(first) <= level(second) {
        (first, second)
    } else {
        (second, first)
    };

    // The seam points come from the neutral circles, so the record edges can be
    // created in the order the loop first names them (circle, seam, circle): the
    // record's canonical order is "edges walking faces → loops → uses".
    let mut seams = Vec::with_capacity(2);
    for curve in [lower, upper] {
        let (center, radius) = match curve {
            BrepCurve::Circle { center, radius, .. } => (*center, *radius),
            _ => unreachable!("the ring is two circles"),
        };
        seams.push(seam_point(center, radius, curve).ok_or_else(|| {
            LiftRefusal::placement(format!(
                "face {} has a circular boundary with a degenerate frame",
                face.id
            ))
        })?);
    }
    let [lower_seam, upper_seam] = seams.as_slice() else {
        return Ok(None);
    };
    let radial_of = |p: Point3| {
        let offset = sub(p, origin);
        sub(offset, mul(axis, dot(offset, axis)))
    };
    let (lower_radial, upper_radial) = (radial_of(*lower_seam), radial_of(*upper_seam));
    let aligned = if length(lower_radial) > 1e-12 && length(upper_radial) > 1e-12 {
        length(sub(normalize(lower_radial), normalize(upper_radial))) <= 1e-6
    } else {
        length(sub(lower_radial, upper_radial)) <= 1e-6 * (1.0 + length(sub(*lower_seam, origin)))
    };
    if !aligned {
        return Err(LiftRefusal::unsupported(format!(
            "face {} is a {} surprise: its two boundary circles' reference points are not on one \
             generator, so the seam the record needs would not be a straight edge on the surface",
            face.id,
            kind.name()
        )));
    }
    let lower_edge = authored_segment(record, vertex_of, edge_of, arcs, circles, lower, face.id)?;
    let up = authored_segment(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &BrepCurve::Line {
            start: *lower_seam,
            end: *upper_seam,
        },
        face.id,
    )?;
    let upper_edge = authored_segment(record, vertex_of, edge_of, arcs, circles, upper, face.id)?;
    let down = authored_segment(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &BrepCurve::Line {
            start: *upper_seam,
            end: *lower_seam,
        },
        face.id,
    )?;
    // Counter-clockwise in `(u, v)` seen along the surface's natural normal:
    // the lower circle forwards, up at `u = 2π`, the upper circle backwards, down
    // at `u = 0`.
    Ok(Some(vec![
        RawUse {
            edge: lower_edge.edge,
            forward: true,
            segment: Segment::Circle,
        },
        RawUse {
            edge: up.edge,
            forward: true,
            segment: Segment::Line {
                start: *lower_seam,
                end: *upper_seam,
            },
        },
        RawUse {
            edge: upper_edge.edge,
            forward: false,
            segment: Segment::Circle,
        },
        RawUse {
            edge: down.edge,
            forward: down.forward,
            segment: Segment::Line {
                start: *upper_seam,
                end: *lower_seam,
            },
        },
    ]))
}

/// The record band of a **full cone**: one base circle and the apex.
///
/// The apex is not a loop of its own in a record this build can write (`Loop` has
/// `Vertex` but the kernel's own cone record uses two uses of one generator whose
/// far end is the apex, and a seam loop must be edges). So the band is
/// `[base circle, generator up, generator down]`: a triangle in `(u, v)` whose
/// area is `2πh/2`, counter-clockwise about the surface's natural normal. The
/// generator is a straight ruling of the cone, so its pcurve is exact
/// (`u` constant, `v` from `0` to the apex's height).
#[allow(clippy::too_many_arguments)]
fn apex_band(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    kind: CurvedKind,
    label: &Surface,
    origin: Point3,
    axis: Point3,
    base: &BrepCurve,
) -> Result<Option<Vec<RawUse>>, LiftRefusal> {
    let Surface::Cone { half_angle, .. } = label else {
        return Ok(None);
    };
    let Some(apex) = apex_point(origin, axis, *half_angle, label) else {
        return Ok(None);
    };
    let (center, radius) = match base {
        BrepCurve::Circle { center, radius, .. } => (*center, *radius),
        _ => return Ok(None),
    };
    let seam = seam_point(center, radius, base).ok_or_else(|| {
        LiftRefusal::placement(format!(
            "face {} has a circular boundary with a degenerate frame",
            face.id
        ))
    })?;
    // The generator runs from the base rim to the apex and back. Both endpoints
    // must be one ruling: the apex is on the axis, so its radial part is zero.
    let base_edge = authored_segment(record, vertex_of, edge_of, arcs, circles, base, face.id)?;
    let up = authored_segment(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &BrepCurve::Line {
            start: seam,
            end: apex,
        },
        face.id,
    )?;
    let down = authored_segment(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &BrepCurve::Line {
            start: apex,
            end: seam,
        },
        face.id,
    )?;
    let _ = kind;
    Ok(Some(vec![
        RawUse {
            edge: base_edge.edge,
            forward: true,
            segment: Segment::Circle,
        },
        RawUse {
            edge: up.edge,
            forward: true,
            segment: Segment::Line {
                start: seam,
                end: apex,
            },
        },
        RawUse {
            edge: down.edge,
            forward: down.forward,
            segment: Segment::Line {
                start: apex,
                end: seam,
            },
        },
    ]))
}

/// A cone's apex, from its label: `origin + axis·(−r₀/tan(half angle))`.
///
/// `None` when the label has no slope (a cylinder, or a half angle of zero) or the
/// point is not finite.
pub(super) fn apex_point(
    origin: Point3,
    axis: Point3,
    half_angle: f64,
    label: &Surface,
) -> Option<Point3> {
    let Surface::Cone {
        radius_at_origin, ..
    } = label
    else {
        return None;
    };
    let slope = half_angle.tan();
    if !slope.is_finite() || slope.abs() < 1e-12 {
        return None;
    }
    let apex = add(origin, mul(axis, -radius_at_origin / slope));
    if super::finite_point(apex) {
        Some(apex)
    } else {
        None
    }
}

/// The point a full circle's single vertex sits at: `u_dir`'s radius direction.
fn seam_point(center: Point3, radius: f64, curve: &BrepCurve) -> Option<Point3> {
    let BrepCurve::Circle { normal, u_dir, .. } = curve else {
        return None;
    };
    let normal = normalize(*normal);
    let x = normalize(sub(*u_dir, mul(normal, dot(*u_dir, normal))));
    if !is_axis(normal) || !is_axis(x) {
        return None;
    }
    Some(add(center, mul(x, radius)))
}

/// A full sphere's seam: the meridian at longitude zero, south pole to north
/// pole, used once in each direction (the reader maps the loop-less face back
/// to a full sphere).
fn sphere_seam(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    label: &Surface,
) -> Result<Vec<Vec<RawUse>>, LiftRefusal> {
    let Surface::Sphere { frame, radius } = label else {
        return Err(LiftRefusal::unsupported(
            "an internal sphere record lost its kind",
        ));
    };
    let (origin, x, z) = frame_fields(frame);
    let y = normalize(cross(z, x));
    // From the north pole to the south pole through the frame's `x`: the half
    // meridian at longitude zero, which is the one a sphere face's `(u, v)` can
    // trim (the other half is the longitude `pi` meridian).
    let seam = BrepCurve::Arc {
        center: origin,
        normal: y,
        u_dir: z,
        radius: *radius,
        start: add(origin, mul(z, *radius)),
        end: sub(origin, mul(z, *radius)),
    };
    let authored = authored_segment(record, vertex_of, edge_of, arcs, circles, &seam, face.id)?;
    Ok(vec![vec![
        RawUse {
            edge: authored.edge,
            forward: true,
            segment: Segment::Arc,
        },
        RawUse {
            edge: authored.edge,
            forward: false,
            segment: Segment::Arc,
        },
    ]])
}

/// A full torus's seam: its tube circle at longitude zero and its outer equator,
/// each used once in each direction (the reader maps the loop-less face back to
/// a full torus).
fn torus_seam(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    face: &BrepFace,
    label: &Surface,
) -> Result<Vec<Vec<RawUse>>, LiftRefusal> {
    let Surface::Torus {
        frame,
        major_radius,
        minor_radius,
        ..
    } = label
    else {
        return Err(LiftRefusal::unsupported(
            "an internal torus record lost its kind",
        ));
    };
    let (origin, x, z) = frame_fields(frame);
    let y = normalize(cross(z, x));
    // The one point both circles pass through, computed **once**: the tube
    // circle's seam is `(origin + x·R) + x·r` and the equator's is
    // `origin + x·(R + r)`, and those two expressions differ by a rounding step
    // for most inputs. Vertices dedup on exact bits, so computing them
    // separately would leave the loop unchained (measured: 32 % of axis-aligned
    // and 70 % of general unit frames disagree on the last bit).
    let seam = add(origin, mul(x, major_radius + minor_radius));
    let tube = BrepCurve::Circle {
        center: add(origin, mul(x, *major_radius)),
        // The tube's own normal at longitude zero, so its seam point is the
        // equator point both circles share.
        normal: mul(y, -1.0),
        u_dir: x,
        radius: *minor_radius,
    };
    let equator = BrepCurve::Circle {
        center: origin,
        normal: z,
        u_dir: x,
        radius: major_radius + minor_radius,
    };
    let tube = authored_with_seam(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &tube,
        face.id,
        Some(seam),
    )?;
    let equator = authored_with_seam(
        record,
        vertex_of,
        edge_of,
        arcs,
        circles,
        &equator,
        face.id,
        Some(seam),
    )?;
    Ok(vec![vec![
        RawUse {
            edge: tube.edge,
            forward: false,
            segment: Segment::Circle,
        },
        RawUse {
            edge: equator.edge,
            forward: true,
            segment: Segment::Circle,
        },
        RawUse {
            edge: tube.edge,
            forward: true,
            segment: Segment::Circle,
        },
        RawUse {
            edge: equator.edge,
            forward: false,
            segment: Segment::Circle,
        },
    ]])
}

/// A frame's `(origin, x, z)` back in the neutral model (see
/// [`surfaces`](super::surfaces)).
fn frame_fields(f: &Frame) -> (Point3, Point3, Point3) {
    let origin = point(f.origin);
    let x = normalize(point(f.x_axis));
    let z = normalize(cross(point(f.x_axis), point(f.y_axis)));
    (origin, x, z)
}

/// Rewrite every boundary circle/arc that lies on a labelled curved face so its
/// label's `x` is that face's `x`.
///
/// # Why this is load-bearing
///
/// `solidcore`'s exact-pcurve call writes a circle-about-the-axis pcurve as
/// `u = u0 + sign·t`, where `u0` is the angle from the **surface's** `x` axis to
/// the **circle's** own `x` axis — a **radian** offset applied to the face's form
/// parameter. That is exact only when the two axes coincide (`u0 = 0`), which
/// holds for the shapes this build constructs but not for a body whose rim circles
/// are labelled on another frame (a SAT import keeps ACIS's `u_dir` while the
/// surface's `ref_dir` is, say, the major axis). Such a body used to be refused
/// with a record-level "geometry disagrees with its topology", which names nothing
/// a caller can act on.
///
/// Only the **label** moves: `BrepCurve::Circle`'s `u_dir` is "the radius
/// direction at parameter zero" and `BrepCurve::Arc`'s is the frame's `x`
/// ([`cad_domain::BrepCurve`]), so replacing it changes no point of the boundary —
/// it picks the same points with the same traversal, parameterised from the face's
/// axis.
///
/// The relabel is done here, on the neutral body, rather than with
/// `solidcore::eval::form::on_frame` after the record exists: `on_frame` shifts an
/// **angle** interval, and the record requires a use's pcurve to span exactly the
/// edge's `interval`, so a shifted circle could not be written without also moving
/// its seam vertex. Aligning the label first gives the same circle on the face's
/// frame with the interval the record expects.
///
/// Only a parallel of a **cylinder or a cone** is aligned: their `u` is the only
/// angle and their `v` is a length, so `u0 = 0` is what makes the closed form
/// exact. A sphere's or torus's angular `v` speaks in radians in that same code,
/// so aligning a parallel there would not help; this build never writes such a face
/// (the reader maps only untrimmed spheres and tori, whose seams we build
/// ourselves, already on the face's frame).
pub(super) fn align_curved_frames(brep: &mut BrepData) -> Result<(), LiftRefusal> {
    let mut axes: Vec<Axis> = Vec::new();
    for shell in &brep.shells {
        for face in &shell.faces {
            if let Some(axis) = Axis::of(&face.surface) {
                axes.push(axis);
            }
        }
    }
    if axes.is_empty() {
        return Ok(());
    }
    for shell in &mut brep.shells {
        for face in &mut shell.faces {
            for loop_ in &mut face.loops {
                for curve in &mut loop_.edges {
                    align_curve(curve, &axes);
                }
            }
        }
    }
    Ok(())
}

/// A labelled curved surface's axis, as [`align_curved_frames`] needs it.
enum Axis {
    Cylinder {
        origin: Point3,
        x: Point3,
        z: Point3,
        radius: f64,
    },
    Cone {
        origin: Point3,
        x: Point3,
        z: Point3,
        radius_at_origin: f64,
        slope: f64,
    },
}

impl Axis {
    fn of(surface: &BrepSurface) -> Option<Self> {
        match surface {
            BrepSurface::Cylinder {
                origin,
                axis,
                ref_dir,
                radius,
            } => Some(Axis::Cylinder {
                origin: *origin,
                x: normalize(*ref_dir),
                z: normalize(*axis),
                radius: *radius,
            }),
            BrepSurface::Cone {
                origin,
                axis,
                ref_dir,
                radius,
                sin_half_angle,
                cos_half_angle,
            } => {
                if cos_half_angle.abs() < 1e-12 {
                    return None;
                }
                Some(Axis::Cone {
                    origin: *origin,
                    x: normalize(*ref_dir),
                    z: normalize(*axis),
                    radius_at_origin: *radius,
                    slope: sin_half_angle / cos_half_angle,
                })
            }
            _ => None,
        }
    }

    fn frame(&self) -> (Point3, Point3, Point3) {
        match self {
            Axis::Cylinder { origin, x, z, .. } | Axis::Cone { origin, x, z, .. } => {
                (*origin, *x, *z)
            }
        }
    }

    /// Whether `(center, radius)` is a **parallel** of this axis: the circle's
    /// plane is square to it, its centre is on it and its radius is the surface's
    /// radius there. Anything else keeps its own label.
    fn carries(&self, center: Point3, radius: f64, normal: Point3, tol: f64) -> bool {
        let (origin, _, z) = self.frame();
        if !is_axis(z) || dot(normalize(normal), z).abs() < 1.0 - 1e-9 {
            return false;
        }
        let offset = sub(center, origin);
        let height = dot(offset, z);
        if length(sub(offset, mul(z, height))) > tol {
            return false;
        }
        let expected = match self {
            Axis::Cylinder { radius: r, .. } => *r,
            Axis::Cone {
                radius_at_origin,
                slope,
                ..
            } => radius_at_origin + height * slope,
        };
        (expected - radius).abs() <= tol.max(1e-9 * expected.abs().max(radius))
    }
}

/// Relabel one curve onto the first axis that carries it (see
/// [`align_curved_frames`]).
fn align_curve(curve: &mut BrepCurve, axes: &[Axis]) {
    let (center, normal, radius) = match curve {
        BrepCurve::Circle {
            center,
            normal,
            radius,
            ..
        }
        | BrepCurve::Arc {
            center,
            normal,
            radius,
            ..
        } => (*center, *normal, *radius),
        _ => return,
    };
    let tol = 1e-6 * (1.0 + radius + length(center));
    for axis in axes {
        if !axis.carries(center, radius, normal, tol) {
            continue;
        }
        let (_, x, _) = axis.frame();
        if !is_axis(x) {
            return;
        }
        match curve {
            BrepCurve::Circle { u_dir, .. } | BrepCurve::Arc { u_dir, .. } => *u_dir = x,
            _ => {}
        }
        return;
    }
}

/// Settle every full circle's traversal from the **planar** face that shares it.
///
/// A closed circle edge has one vertex, so the loop direction cannot be read off
/// its ends the way a line's or an arc's is. It can be read off the face on the
/// other side: a plane's loop runs counter-clockwise about the plane's own
/// normal, which fixes the circle's direction there in terms of the circle's
/// label; the other use of an edge in a consistently oriented shell necessarily
/// runs the other way. A circle shared by two non-planar faces (no plane to read
/// it from) is refused by name rather than guessed.
///
/// It runs on the pending loops because a pcurve's placement may depend on the
/// loop's winding, and the pcurves are computed after it.
pub(super) fn resolve_closed_circles(
    record: &Record,
    pending: &mut [Pending],
) -> Result<(), LiftRefusal> {
    let closed: Vec<usize> = record
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| {
            edge.vertices[0] == edge.vertices[1] && matches!(edge.curve, Curve::Circle { .. })
        })
        .map(|(index, _)| index)
        .collect();
    for index in closed {
        let circle_normal = match &record.edges[index].curve {
            Curve::Circle { frame, .. } => {
                normalize(cross(point(frame.x_axis), point(frame.y_axis)))
            }
            _ => unreachable!("filtered to circles"),
        };
        // Every use of this edge, by (face, loop, position).
        let mut uses: Vec<(usize, usize, usize)> = Vec::new();
        for (face_index, face) in pending.iter().enumerate() {
            for (loop_index, loop_) in face.raw.iter().enumerate() {
                for (position, use_) in loop_.iter().enumerate() {
                    if use_.edge as usize == index {
                        uses.push((face_index, loop_index, position));
                    }
                }
            }
        }
        if uses.len() != 2 {
            return Err(LiftRefusal::unsupported(format!(
                "the closed circular edge {index} is used {} time(s); a closed solid's edge is \
                 used exactly twice",
                uses.len()
            )));
        }
        let mut resolved: Option<((usize, usize, usize), bool)> = None;
        for &use_ in &uses {
            let Surface::Plane { u_axis, v_axis, .. } = &pending[use_.0].surface else {
                continue;
            };
            let plane_normal = normalize(cross(point(*u_axis), point(*v_axis)));
            if !is_axis(plane_normal) {
                return Err(LiftRefusal::unsupported(
                    "a planar face sharing a circular edge has a degenerate frame",
                ));
            }
            // A closed circle shared with a plane must **lie in** that plane, so
            // the two normals are parallel. A perpendicular (or otherwise skew)
            // pair means the record is inconsistent, and `> 0.0` would silently
            // read "perpendicular" as "traversed the other way" — a winding the
            // body never had.
            let alignment = dot(circle_normal, plane_normal);
            if alignment.abs() < 1.0 - 1e-9 {
                return Err(LiftRefusal::unsupported(format!(
                    "a closed circular edge sharing a face with a plane does not lie in that \
                     plane (normals {alignment} apart); its traversal cannot be resolved"
                )));
            }
            let stored = alignment > 0.0;
            if resolved.is_some() {
                return Err(LiftRefusal::unsupported(
                    "a closed circular edge is shared by two planar faces; its traversal is \
                     ambiguous",
                ));
            }
            resolved = Some((use_, stored));
        }
        let Some((plane_use, plane_stored)) = resolved else {
            // No planar face to read the circle from (a sphere's or torus's own
            // seam loop): the two uses are the face's own two sides and were
            // written in opposite directions, which is the invariant.
            let a = pending[uses[0].0].raw[uses[0].1][uses[0].2].forward;
            let b = pending[uses[1].0].raw[uses[1].1][uses[1].2].forward;
            if a == b {
                return Err(LiftRefusal::unsupported(
                    "a closed circular edge is shared by two non-planar faces with the same \
                     traversal; this build resolves a circle's direction from the planar face on \
                     the other side",
                ));
            }
            continue;
        };
        for &use_ in &uses {
            let forward = if use_ == plane_use {
                plane_stored
            } else {
                !plane_stored
            };
            pending[use_.0].raw[use_.1][use_.2].forward = forward;
        }
    }
    Ok(())
}
