//! Neutral surfaces → the record's surface, and the exact NURBS form a labelled
//! curved face must carry.
//!
//! Two jobs, both about the *form* rather than the label:
//!
//! 1. [`write`] turns a neutral [`BrepSurface`] into what the record holds: a
//!    `Plane` label (with the frame the loop writer projects into), a **labelled
//!    curved** surface (`Cylinder`/`Cone`/`Sphere`/`Torus`, which the record
//!    requires to carry a NURBS form), or a **form** surface (a neutral
//!    [`BrepSurface::Nurbs`] is the record's `Spline`/`RationalSpline` label
//!    itself).
//! 2. [`form_of`] builds that form with `solidcore`'s own `eval::form::surface`:
//!    the exact rational tensor-product NURBS of the analytic surface over a
//!    parameter box, and the box in the form's parameters. The kernel proves the
//!    label against it when the record is built, so the two are the same
//!    geometry by construction rather than by our arithmetic.
//!
//! # Why the box is derived from the loops
//!
//! A record face's `domain` is in its **`parameters()`** surface: for a face that
//! carries a NURBS form, that is the *form's* parameters, and every producer
//! chooses its own (the pinned kernel writes a cylinder's u in radians, a
//! sphere's in quarters of a turn over `[0, 1]`). [`cad_domain::BrepFace::domain`]
//! carries the producer's box verbatim, so its numbers cannot be handed to
//! `form::surface` — that call reads *label* parameters (an angle and a height).
//! The box is therefore derived from the face's own boundary geometry
//! ([`label_box`]), which is exact and parameterisation-free, and the written
//! domain is the form's box that `form::surface` returns for it. The neutral
//! domain is still **required** (a curved face that states none is refused by
//! name): its presence is the producer's statement that the face is a trimmed
//! patch of the label rather than something this build may invent.
//!
//! # Frames
//!
//! A record frame is right-handed with `z = x_axis × y_axis`; `x_axis` is the
//! angle's reference direction and `z` the axis (a sphere's pole, a torus's
//! normal). The neutral fields are the same frame, so `y = z × x` reproduces it
//! exactly. A degenerate frame is refused by name, never repaired.

use cad_domain::{BrepCurve, BrepFace, BrepSurface, Point3};
use solidcore_record::brep::{Curve, Frame, Surface};

use super::loops::PlaneFrame;
use super::units::{point, record_point};
use super::{add, cross, dot, finite_point, is_axis, mul, normalize, sub};
use crate::error::LiftRefusal;

/// What a face needs from the surface writer.
#[derive(Debug)]
pub(super) enum Written {
    /// A plane: the loop writer projects into `frame`, and the `(u, v)` box comes
    /// from the boundary corners.
    Plane { surface: Surface, frame: PlaneFrame },
    /// A labelled curved surface (`Cylinder`/`Cone`/`Sphere`/`Torus`): the record
    /// label, whose `nurbs` form the caller builds over [`label_box`].
    Labelled {
        label: Surface,
        /// Which family, for the box and the refusal text.
        kind: CurvedKind,
    },
    /// A **form** surface: the record's `Spline`/`RationalSpline` label, whose own
    /// parameters are the face's `(u, v)` (so no separate `nurbs` is written).
    Form { label: Surface },
}

/// The four analytic families a labelled curved face can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CurvedKind {
    Cylinder,
    Cone,
    Sphere,
    Torus,
}

impl CurvedKind {
    /// The name used in refusals.
    pub(super) fn name(self) -> &'static str {
        match self {
            CurvedKind::Cylinder => "cylinder",
            CurvedKind::Cone => "cone",
            CurvedKind::Sphere => "sphere",
            CurvedKind::Torus => "torus",
        }
    }
}

/// The right-handed record frame `(origin, x, z)` with `y = z × x`.
fn frame(origin: Point3, x_axis: Point3, z_axis: Point3) -> Option<Frame> {
    let z = normalize(z_axis);
    let x = normalize(sub(x_axis, mul(z, dot(x_axis, z))));
    if !is_axis(x) || !is_axis(z) || !finite_point(origin) {
        return None;
    }
    let y = normalize(cross(z, x));
    if !is_axis(y) {
        return None;
    }
    Some(Frame {
        origin: record_point(origin),
        x_axis: record_point(x),
        y_axis: record_point(y),
    })
}

/// A record frame's `(origin, x, z)` back in the neutral model.
fn frame_fields(f: &Frame) -> (Point3, Point3, Point3) {
    let origin = super::units::point(f.origin);
    let x = normalize(super::units::point(f.x_axis));
    let z = normalize(cross(
        super::units::point(f.x_axis),
        super::units::point(f.y_axis),
    ));
    (origin, x, z)
}

/// One neutral surface as the record holds it, or the named refusal.
pub(super) fn write(face: &BrepFace) -> Result<Written, LiftRefusal> {
    match &face.surface {
        BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } => {
            let normal = normalize(*normal);
            if !is_axis(normal) || !finite_point(*origin) {
                return Err(LiftRefusal::placement(format!(
                    "face {} has a degenerate plane frame",
                    face.id
                )));
            }
            let surface = super::surface::record_plane(*origin, normal, *u_dir)?;
            let (u_axis, v_axis) = match &surface {
                Surface::Plane { u_axis, v_axis, .. } => (*u_axis, *v_axis),
                _ => {
                    return Err(LiftRefusal::unsupported(
                        "an internal plane record lost its kind",
                    ))
                }
            };
            Ok(Written::Plane {
                surface,
                frame: PlaneFrame {
                    origin: *origin,
                    u_axis: super::units::point(u_axis),
                    v_axis: super::units::point(v_axis),
                },
            })
        }
        BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } => {
            require_domain(face, CurvedKind::Cylinder)?;
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a cylinder with radius {radius}; it must be finite and positive",
                    face.id
                )));
            }
            let frame = frame(*origin, *ref_dir, *axis).ok_or_else(|| {
                LiftRefusal::placement(format!("face {} has a degenerate cylinder frame", face.id))
            })?;
            Ok(Written::Labelled {
                label: Surface::Cylinder {
                    frame,
                    radius: *radius,
                },
                kind: CurvedKind::Cylinder,
            })
        }
        BrepSurface::Cone {
            origin,
            axis,
            ref_dir,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => {
            require_domain(face, CurvedKind::Cone)?;
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a cone with radius {radius}; it must be finite and positive",
                    face.id
                )));
            }
            let half_angle = sin_half_angle.atan2(*cos_half_angle);
            if !half_angle.is_finite() || half_angle.abs() >= std::f64::consts::FRAC_PI_2 {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a cone with half angle {half_angle}; it must be finite and below \
                     a right angle",
                    face.id
                )));
            }
            let frame = frame(*origin, *ref_dir, *axis).ok_or_else(|| {
                LiftRefusal::placement(format!("face {} has a degenerate cone frame", face.id))
            })?;
            Ok(Written::Labelled {
                label: Surface::Cone {
                    frame,
                    radius_at_origin: *radius,
                    half_angle,
                },
                kind: CurvedKind::Cone,
            })
        }
        BrepSurface::Sphere {
            center,
            radius,
            u_dir,
            pole,
        } => {
            require_domain(face, CurvedKind::Sphere)?;
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a sphere with radius {radius}; it must be finite and positive",
                    face.id
                )));
            }
            let frame = frame(*center, *u_dir, *pole).ok_or_else(|| {
                LiftRefusal::placement(format!("face {} has a degenerate sphere frame", face.id))
            })?;
            Ok(Written::Labelled {
                label: Surface::Sphere {
                    frame,
                    radius: *radius,
                },
                kind: CurvedKind::Sphere,
            })
        }
        BrepSurface::Torus {
            center,
            normal,
            major_radius,
            minor_radius,
            u_dir,
        } => {
            require_domain(face, CurvedKind::Torus)?;
            if !major_radius.is_finite()
                || !minor_radius.is_finite()
                || *minor_radius <= 0.0
                || *major_radius <= *minor_radius
            {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a torus with major radius {major_radius} and minor radius \
                     {minor_radius}; the record's torus label needs a ring torus",
                    face.id
                )));
            }
            let frame = frame(*center, *u_dir, *normal).ok_or_else(|| {
                LiftRefusal::placement(format!("face {} has a degenerate torus frame", face.id))
            })?;
            Ok(Written::Labelled {
                label: Surface::Torus {
                    frame,
                    major_radius: *major_radius,
                    minor_radius: *minor_radius,
                    tube_start: 0.0,
                },
                kind: CurvedKind::Torus,
            })
        }
        BrepSurface::Nurbs {
            u_knots,
            v_knots,
            points,
        } => {
            // A form surface has no analytic label to fall back on, so its box is
            // the producer's and cannot be derived from a label.
            if face.domain.is_none() {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} is a NURBS surface with no (u, v) domain; the record writer does not \
                     invent a parameter box for a form surface",
                    face.id
                )));
            }
            Ok(Written::Form {
                label: form_surface(face, u_knots, v_knots, points)?,
            })
        }
        BrepSurface::Unsupported { type_key } => Err(LiftRefusal::unsupported(format!(
            "face {} is an unsupported surface '{type_key}', which this build cannot write",
            face.id
        ))),
    }
}

/// A curved face must state its `(u, v)` box: without it the face is not
/// distinguishable from an untrimmed primitive, and this writer will not invent
/// one.
fn require_domain(face: &BrepFace, kind: CurvedKind) -> Result<(), LiftRefusal> {
    let Some(domain) = face.domain else {
        return Err(LiftRefusal::unsupported(format!(
            "face {} is a {} surface without its (u, v) domain; the record writer does not invent \
             a parameter box for a curved face",
            face.id,
            kind.name()
        )));
    };
    // The numbers are the producer's parameterisation and are not read (see the
    // module docs), but a box that is not a box is not a statement about a patch
    // either, so it is refused rather than ignored.
    if domain.iter().flatten().any(|value| !value.is_finite())
        || domain[0][1] <= domain[0][0]
        || domain[1][1] <= domain[1][0]
    {
        return Err(LiftRefusal::unsupported(format!(
            "face {} states a {} (u, v) domain {domain:?} that is not a positive finite box",
            face.id,
            kind.name()
        )));
    }
    Ok(())
}

/// A neutral NURBS net as the record's `Spline` (all weights exactly `1.0`) or
/// `RationalSpline`.
///
/// The net is copied bit-for-bit: knots, weights and the degree (which stays
/// implicit as `knots.len() - points.len() - 1`). A weight that is not finite and
/// positive is refused by name — `Spline` cannot carry it and `RationalSpline`
/// would divide by it.
fn form_surface(
    face: &BrepFace,
    u_knots: &[f64],
    v_knots: &[f64],
    points: &[Vec<[f64; 4]>],
) -> Result<Surface, LiftRefusal> {
    let mut unit_weights = true;
    for (row_index, row) in points.iter().enumerate() {
        for (column_index, point) in row.iter().enumerate() {
            let weight = point[3];
            if !weight.is_finite() || weight < 1e-12 {
                return Err(LiftRefusal::unsupported(format!(
                    "face {} has a NURBS weight {weight} at ({row_index}, {column_index}); it must \
                     be finite and at least 1e-12",
                    face.id
                )));
            }
            if weight != 1.0 {
                unit_weights = false;
            }
        }
    }
    if unit_weights {
        let points = points
            .iter()
            .map(|row| row.iter().map(|p| [p[0], p[1], p[2]]).collect())
            .collect();
        Ok(Surface::Spline {
            u_knots: u_knots.to_vec(),
            v_knots: v_knots.to_vec(),
            points,
        })
    } else {
        Ok(Surface::RationalSpline {
            u_knots: u_knots.to_vec(),
            v_knots: v_knots.to_vec(),
            homogeneous: points.to_vec(),
        })
    }
}

/// The `(u, v)` box a labelled curved face spans, **in the label's parameters**
/// (an angle in radians, a height in lengths) — the input `eval::form::surface`
/// takes, not the box the record stores.
///
/// The **u** direction is the full turn `[0, 2π]` for every labelled curved
/// family: their `u` is an angle about the axis, and a face's boundary is either a
/// closed circle (the whole turn) or a set of arcs inside it. Taking the whole
/// turn gives a **bounding** box — wider than the patch by construction, not a
/// claim about how much of the surface the face uses: it contains every trim, it
/// needs no unwrapping of angles that can sit anywhere on the circle, and it is
/// what lets a seam use sit a whole period from its twin. **Never tighten it**: a
/// sphere's seam loop is a zero-area bigon only because the box is the full turn
/// (the pole jump places its second use exactly one period away).
///
/// The **v** direction is the boundary's own extent: the axial height of every
/// boundary point for a cylinder or cone. A sphere's latitude and a torus's tube
/// angle are whole ranges too (the reader maps only untrimmed sphere/torus faces,
/// so their v extent *is* the full surface). `None` when there is no boundary to
/// measure (an untrimmed cylinder/cone, whose face is not a body) or the extent is
/// not a positive finite interval.
pub(super) fn label_box(face: &BrepFace, label: &Surface) -> Option<[[f64; 2]; 2]> {
    const TURN: f64 = std::f64::consts::TAU;
    match label {
        Surface::Cylinder { frame, .. } | Surface::Cone { frame, .. } => {
            let (origin, _, z) = frame_fields(frame);
            let (mut v_min, mut v_max) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut sampled = false;
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    for point in samples(curve) {
                        let axial = dot(sub(point, origin), z);
                        v_min = v_min.min(axial);
                        v_max = v_max.max(axial);
                        sampled = true;
                    }
                    if let BrepCurve::Circle { center, .. } = curve {
                        let axial = dot(sub(*center, origin), z);
                        v_min = v_min.min(axial);
                        v_max = v_max.max(axial);
                        sampled = true;
                    }
                    // A full cone's apex is a boundary point with no curve: the
                    // v extent must reach it, or the form stops short of the face.
                    if matches!(curve, BrepCurve::Unsupported { type_key } if type_key == super::loops::APEX_WITHOUT_CURVE)
                    {
                        if let Surface::Cone { half_angle, .. } = label {
                            if let Some(apex) =
                                super::curved::apex_point(origin, z, *half_angle, label)
                            {
                                let axial = dot(sub(apex, origin), z);
                                v_min = v_min.min(axial);
                                v_max = v_max.max(axial);
                                sampled = true;
                            }
                        }
                    }
                }
            }
            if !sampled || !v_min.is_finite() || !v_max.is_finite() || v_max <= v_min {
                return None;
            }
            Some([[0.0, TURN], [v_min, v_max]])
        }
        Surface::Sphere { .. } => Some([
            [0.0, TURN],
            [-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2],
        ]),
        Surface::Torus { .. } => Some([[0.0, TURN], [0.0, TURN]]),
        _ => None,
    }
}

/// Points that pin one boundary curve's parameter extent on a surface of
/// revolution: its ends, plus the arc's own bulge (a chord would cut it).
fn samples(curve: &BrepCurve) -> Vec<Point3> {
    match curve {
        BrepCurve::Line { start, end } => vec![*start, *end],
        BrepCurve::Circle { .. } => Vec::new(),
        BrepCurve::Arc {
            center,
            normal,
            u_dir,
            radius,
            start,
            end,
        } => {
            let normal = normalize(*normal);
            let x = normalize(*u_dir);
            let y = cross(normal, x);
            if !is_axis(normal) || !is_axis(x) {
                return vec![*start, *end];
            }
            let angle = |p: Point3| {
                let d = sub(p, *center);
                dot(d, y).atan2(dot(d, x))
            };
            let a0 = angle(*start);
            let sweep = {
                let delta = angle(*end) - a0;
                if delta > 0.0 {
                    delta
                } else {
                    delta + std::f64::consts::TAU
                }
            };
            let mut out = Vec::with_capacity(9);
            for step in 0..=8 {
                let t = a0 + sweep * f64::from(step) / 8.0;
                out.push(add(
                    *center,
                    add(mul(x, radius * t.cos()), mul(y, radius * t.sin())),
                ));
            }
            out
        }
        BrepCurve::Unsupported { .. } => Vec::new(),
    }
}

/// The exact NURBS form of `label` over `box` (in the label's parameters) and the
/// box in the form's parameters.
///
/// `solidcore`'s `eval::form` is the authority here: it lays the form on the
/// surface's own quarter grid, so a circle edge written by
/// [`circle_form`](super::arc::circle_form) shares this parameterisation and the
/// pcurves are exact by construction. `None` (an empty or over-long angle range,
/// a non-ring torus) is a named refusal, never a fallback.
pub(super) fn form_of(
    face: &BrepFace,
    label: &Surface,
    box_: [[f64; 2]; 2],
) -> Result<(Surface, [[f64; 2]; 2]), LiftRefusal> {
    let (su, sv) = parameter_scale(face, label)?;
    let kernel = solidcore_convert::surface(label).map_err(|error| {
        LiftRefusal::unsupported(format!(
            "face {} has a surface the record conversion refuses: {error}",
            face.id
        ))
    })?;
    let (form, domain) = solidcore::eval::form::surface(&kernel, box_).ok_or_else(|| {
        LiftRefusal::unsupported(format!(
            "face {} cannot be written as an exact NURBS form over {box_:?} (an empty or \
             over-long parameter range, or a surface `eval::form` does not carry)",
            face.id
        ))
    })?;
    let form = solidcore_convert::record_surface(form).map_err(|error| {
        LiftRefusal::unsupported(format!(
            "face {}'s NURBS form cannot be written as a record surface: {error}",
            face.id
        ))
    })?;
    Ok((
        rescaled(face, form, su, sv)?,
        [
            [domain[0][0] * su, domain[0][1] * su],
            [domain[1][0] * sv, domain[1][1] * sv],
        ],
    ))
}

/// How much `eval::form`'s parameter is scaled per direction: `π/2` (one quarter
/// turn in radians) for each direction that is an **angle**, `1` for a length.
///
/// # What the scale buys — and what it does not
///
/// `eval::form` lays its arcs on the surface's quarter grid: one unit per quarter
/// turn, its parameter `q + quarter_u(ψ)` (see `solidcore::eval::form::param`).
/// That map is **not** the angle — inside a quarter, parameter `0.25` is
/// `0.357…` rad, not `0.393…` rad — and scaling its knots does not make it the
/// angle either; the interior stays the conic's parameter.
///
/// What the scale does buy is **units**: after it, one turn is `2π`, a quarter
/// turn is `π/2`, and the box ends are real quarter-turn angles. That is the unit
/// the rest of the seam uses — `solidcore`'s analytic `Domain::period` for these
/// surfaces is `2π`, and a seam use sits exactly one period from its twin — so the
/// box, the period and the seam jump agree, and a boundary at a quarter line lands
/// on a knot. (Measured without it: a cylinder's seam pcurve came out at `u = 2π`
/// while its rims were at `u = 4`, and a sphere's meridian started 0.65 mm off its
/// edge.)
///
/// Exactness of every pcurve emitted here rests on something else: each one is
/// constant-`u` or constant-`v`, or a **boundary circle relabelled onto the face's
/// own frame** (`curved::align_curved_frames`), which is what makes the kernel's
/// closed forms (`u0 + t` with `u0 = 0`) agree with this parameter term by term.
/// The form parameter is **not** the label's angle; nothing may treat it as one.
fn parameter_scale(face: &BrepFace, label: &Surface) -> Result<(f64, f64), LiftRefusal> {
    const QUARTER: f64 = std::f64::consts::FRAC_PI_2;
    Ok(match label {
        Surface::Cylinder { .. } => (QUARTER, 1.0),
        Surface::Cone { .. } => (QUARTER, 1.0),
        Surface::Sphere { .. } => (QUARTER, QUARTER),
        Surface::Torus { .. } => (QUARTER, QUARTER),
        _ => {
            return Err(LiftRefusal::unsupported(format!(
                "face {} carries a form on a label this build has no angle/length \
                 parameterisation for: {:?}",
                face.id, label
            )))
        }
    })
}

/// A record surface with its knot vectors (and the form's domain) multiplied by
/// `(su, sv)`.
///
/// Only a NURBS can be rescaled; anything else would pair an unscaled form with a
/// scaled domain, so it is a named refusal instead.
fn rescaled(face: &BrepFace, surface: Surface, su: f64, sv: f64) -> Result<Surface, LiftRefusal> {
    let scale = |knots: &mut Vec<f64>, factor: f64| {
        if factor != 1.0 {
            for knot in knots.iter_mut() {
                *knot *= factor;
            }
        }
    };
    Ok(match surface {
        Surface::Spline {
            mut u_knots,
            mut v_knots,
            points,
        } => {
            scale(&mut u_knots, su);
            scale(&mut v_knots, sv);
            Surface::Spline {
                u_knots,
                v_knots,
                points,
            }
        }
        Surface::RationalSpline {
            mut u_knots,
            mut v_knots,
            homogeneous,
        } => {
            scale(&mut u_knots, su);
            scale(&mut v_knots, sv);
            Surface::RationalSpline {
                u_knots,
                v_knots,
                homogeneous,
            }
        }
        other => {
            return Err(LiftRefusal::unsupported(format!(
                "face {}'s form is not a NURBS surface, so its parameters cannot be \
                 rescaled: {other:?}",
                face.id
            )))
        }
    })
}

/// A circle's or arc's exact NURBS form (a record `Curve`) over the angular span
/// `[t0, t1]`, with its interval **in the form's parameters**.
///
/// The record treats `Curve::Circle` as a label and keeps the geometry in the
/// edge's `nurbs`, so an arc is "circle label + form + interval + two vertices".
/// The form is `solidcore`'s own (the same quarter-grid parameterisation
/// [`form_of`] uses for the face it bounds), which is what makes the use's
/// pcurve exact instead of fitted.
pub(super) fn circle_form(
    face: u32,
    center: Point3,
    normal: Point3,
    u_dir: Point3,
    radius: f64,
    span: [f64; 2],
) -> Result<(Curve, [f64; 2]), LiftRefusal> {
    let normal = normalize(normal);
    let x = normalize(sub(u_dir, mul(normal, dot(u_dir, normal))));
    if !is_axis(normal) || !is_axis(x) {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular boundary with a degenerate frame"
        )));
    }
    let y = normalize(cross(normal, x));
    if !is_axis(y) || !radius.is_finite() || radius <= 0.0 {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary with radius {radius}; it must be finite and \
             positive"
        )));
    }
    let label = solidcore::eval::Curve::Circle {
        frame: solidcore::eval::Frame {
            origin: record_point(center),
            x_axis: record_point(x),
            y_axis: record_point(y),
        },
        radius,
    };
    circular_form(face, &label, span)
}

/// The exact record form (and its interval) of a circular boundary described by
/// the kernel's own `eval::Curve`, over an angular span in radians.
///
/// One quarter turn becomes `π/2`, so one turn is `2π` — the unit the face's box
/// and the analytic period use (see `parameter_scale`). The interior parameter
/// stays the conic's; the pcurve is exact because the label shares this grid
/// **and** the face's frame (see `curved::align_curved_frames`).
pub(super) fn circular_form(
    face: u32,
    label: &solidcore::eval::Curve,
    span: [f64; 2],
) -> Result<(Curve, [f64; 2]), LiftRefusal> {
    let (form, interval) = solidcore::eval::form::curve(label, span).ok_or_else(|| {
        LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary over {span:?} that has no exact NURBS form \
             (a non-positive span or more than one turn)"
        ))
    })?;
    let form = solidcore_convert::record_curve(form).map_err(|error| {
        LiftRefusal::unsupported(format!(
            "face {face}'s circular boundary form cannot be written as a record curve: {error}"
        ))
    })?;
    let Curve::RationalSpline { knots, homogeneous } = form else {
        // `eval::form::curve` builds a rational form for a circle or an arc; a
        // plain `Spline` cannot carry the conic, so writing one would be a
        // different curve.
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary whose exact form is not a rational spline"
        )));
    };
    const QUARTER: f64 = std::f64::consts::FRAC_PI_2;
    let span_in_form = [interval[0] * QUARTER, interval[1] * QUARTER];
    let knots = knots.into_iter().map(|k| k * QUARTER).collect::<Vec<f64>>();
    // The record requires a use's pcurve to span exactly the edge's `interval`, and
    // `arc_pcurve` gets it by projecting the edge's form. So the form's own knot
    // span must **be** the trim. On the shared quarter grid that holds only when
    // both ends of the arc are on the grid (a whole turn always is). Off the grid
    // the form is still exact, but it would cover part of a quarter the edge does
    // not, so it is rebuilt on the arc's **own** ends below (exact on a plane,
    // where the pcurve is the affine image; a curved face then has no exact pcurve
    // and says so).
    let (first, last) = match (knots.first(), knots.last()) {
        (Some(first), Some(last)) => (*first, *last),
        _ => {
            return Err(LiftRefusal::unsupported(format!(
                "face {face} has a circular boundary whose form has no knot span"
            )))
        }
    };
    let slack = 1e-9 * (1.0 + first.abs().max(last.abs()));
    if (span_in_form[0] - first).abs() <= slack && (span_in_form[1] - last).abs() <= slack {
        return Ok((Curve::RationalSpline { knots, homogeneous }, span_in_form));
    }
    let (center, x, y, radius) = conic_frame(face, label)?;
    arc_on_its_own_arcs(face, center, x, y, radius, span)
}

/// A circle label's centre, in-plane axes and radius.
fn conic_frame(
    face: u32,
    label: &solidcore::eval::Curve,
) -> Result<(Point3, Point3, Point3, f64), LiftRefusal> {
    let solidcore::eval::Curve::Circle { frame, radius } = label else {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a non-circular boundary where a circle was expected"
        )));
    };
    let center = point(frame.origin);
    let x = normalize(point(frame.x_axis));
    let y = normalize(point(frame.y_axis));
    if !is_axis(x) || !is_axis(y) || !radius.is_finite() || *radius <= 0.0 {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular boundary with a degenerate frame"
        )));
    }
    Ok((center, x, y, *radius))
}

/// The exact rational quadratic of an arc **on its own knots**, one clamped
/// segment per quarter turn or less (the form the record's own kernel writes).
///
/// Used when the arc's ends are off the shared grid: its knot span is then the
/// trim itself, which is what a use's pcurve must span. The parameter between two
/// knots is the conic's, not the angle — that is what keeps it exact.
fn arc_on_its_own_arcs(
    face: u32,
    center: Point3,
    x: Point3,
    y: Point3,
    radius: f64,
    [start, end]: [f64; 2],
) -> Result<(Curve, [f64; 2]), LiftRefusal> {
    let sweep = end - start;
    if !sweep.is_finite() || sweep <= 1e-12 || sweep > std::f64::consts::TAU {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary sweeping {sweep} rad"
        )));
    }
    let at = |angle: f64| {
        add(
            center,
            add(mul(x, radius * angle.cos()), mul(y, radius * angle.sin())),
        )
    };
    let segments = ((sweep / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
    let step = sweep / segments as f64;
    let weight = (0.5 * step).cos().max(1e-9);
    let mut knots = vec![start, start, start];
    let mut homogeneous = vec![[at(start).x, at(start).y, at(start).z, 1.0]];
    for index in 0..segments {
        let from = start + step * index as f64;
        let mid = from + 0.5 * step;
        let to = from + step;
        let control = add(
            center,
            add(
                mul(x, radius / weight * mid.cos()),
                mul(y, radius / weight * mid.sin()),
            ),
        );
        homogeneous.push([
            control.x * weight,
            control.y * weight,
            control.z * weight,
            weight,
        ]);
        let p = at(to);
        homogeneous.push([p.x, p.y, p.z, 1.0]);
        let repeats = if index + 1 < segments { 2 } else { 3 };
        for _ in 0..repeats {
            knots.push(to);
        }
    }
    Ok((Curve::RationalSpline { knots, homogeneous }, [start, end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cylinder_surface() -> BrepSurface {
        BrepSurface::Cylinder {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            axis: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            ref_dir: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            radius: 1.0,
        }
    }

    fn make_face(surface: BrepSurface, domain: Option<[[f64; 2]; 2]>) -> BrepFace {
        BrepFace {
            id: 7,
            surface,
            reversed: false,
            loops: Vec::new(),
            domain,
            tolerance: None,
        }
    }

    fn net(weights: [f64; 3]) -> Vec<Vec<[f64; 4]>> {
        vec![
            vec![[0.0, 0.0, 0.0, weights[0]], [0.0, 1.0, 0.0, 1.0]],
            vec![[1.0, 0.0, 0.0, 1.0], [1.0, 1.0, 0.0, weights[1]]],
            vec![[2.0, 0.0, 0.0, 1.0], [2.0, 1.0, 0.0, weights[2]]],
        ]
    }

    fn form(weights: [f64; 3]) -> Surface {
        let face = make_face(
            BrepSurface::Nurbs {
                u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                v_knots: vec![0.0, 0.0, 1.0, 1.0],
                points: net(weights),
            },
            Some([[0.0, 1.0], [0.0, 1.0]]),
        );
        match write(&face).expect("a well-formed net writes") {
            Written::Form { label } => label,
            _ => panic!("a form face writes as a form label"),
        }
    }

    /// Every weight exactly `1.0` is the record's `Spline` (no weights at all);
    /// any other weight is its `RationalSpline`. Neither is rounded into the
    /// other: a weight of `1.0000000001` is a different surface.
    #[test]
    fn a_unit_weight_net_is_a_spline_and_any_other_weight_a_rational_spline() {
        assert!(matches!(form([1.0, 1.0, 1.0]), Surface::Spline { .. }));
        for weight in [1.0000000001, 0.5, 2.0] {
            assert!(
                matches!(form([1.0, weight, 1.0]), Surface::RationalSpline { .. }),
                "weight {weight}"
            );
        }
    }

    /// A weight that cannot be a point of a surface is refused by name, never
    /// nudged to `1.0` and never divided by.
    #[test]
    fn a_non_positive_or_non_finite_weight_is_refused_by_name() {
        for weight in [0.0, -1.0, 1e-13, f64::NAN] {
            let face = make_face(
                BrepSurface::Nurbs {
                    u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    v_knots: vec![0.0, 0.0, 1.0, 1.0],
                    points: net([1.0, weight, 1.0]),
                },
                Some([[0.0, 1.0], [0.0, 1.0]]),
            );
            let refusal = write(&face).expect_err("a weight below 1e-12 is refused");
            assert_eq!(
                refusal.code,
                crate::error::LIFT_UNSUPPORTED,
                "weight {weight}"
            );
            assert!(
                refusal.detail.contains("weight") && refusal.detail.contains("(1, 1)"),
                "the refusal names the coefficient: {}",
                refusal.detail
            );
        }
    }

    /// A curved face must state its box; a plane must not need one.
    #[test]
    fn a_curved_face_without_a_domain_is_refused_by_name() {
        let face = make_face(cylinder_surface(), None);
        let refusal = write(&face).expect_err("a curved face without a box is refused");
        assert_eq!(refusal.code, crate::error::LIFT_UNSUPPORTED);
        assert!(
            refusal.detail.contains("(u, v) domain") && refusal.detail.contains("cylinder"),
            "the refusal names the face and the gap: {}",
            refusal.detail
        );

        // A stated box that is not a box is refused too, not ignored.
        for domain in [
            [[0.0, f64::NAN], [0.0, 1.0]],
            [[1.0, 0.0], [0.0, 1.0]],
            [[0.0, 1.0], [2.0, 2.0]],
        ] {
            let face = make_face(cylinder_surface(), Some(domain));
            let refusal = write(&face).expect_err("a degenerate box is refused");
            assert_eq!(refusal.code, crate::error::LIFT_UNSUPPORTED, "{domain:?}");
            assert!(
                refusal.detail.contains("not a positive finite box"),
                "{domain:?}"
            );
        }
    }
}
