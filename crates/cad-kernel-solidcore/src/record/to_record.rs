//! Neutral B-rep → record: the writer every boolean, mass-properties and chamfer
//! request passes through.
//!
//! `cad-db` is the authority, so the kernel only ever sees a record; a body this
//! writer cannot express **exactly** is refused with a stable code rather than
//! approximated.
//!
//! # Faces
//!
//! - A **plane** is written from its frame with no form, and its `(u, v)` box is
//!   derived from its boundary (a plane has no bounded natural domain).
//! - A **labelled curved** face (`Cylinder`, `Cone`, `Sphere`, `Torus`) is written
//!   as the analytic label **plus** the exact rational NURBS form the record
//!   requires of it (`solidcore`'s `eval::form::surface`, rescaled so **one turn is
//!   `2π`** — the unit the analytic period and the seam jump use; the form's
//!   interior parameter is *not* the label's angle, see `record::surfaces`). Its
//!   boundary circles are relabelled onto the face's frame first
//!   (`record::curved::align_curved_frames`), which is what makes the kernel's
//!   closed-form pcurves exact. It must state its `(u, v)` domain: a curved face
//!   without one is refused **by name**
//!   ([`LIFT_UNSUPPORTED`](crate::LIFT_UNSUPPORTED)), because this writer will not
//!   invent a parameter box. The written box is the form's, not the producer's
//!   numbers — see `record::surfaces`.
//! - A **form** face ([`BrepSurface::Nurbs`]) is the record's own `Spline`/
//!   `RationalSpline` label, copied with its stated box.
//!
//! Whatever cannot be written — a spline or `Unsupported` boundary curve, an
//! `Unsupported` surface kind — is refused with
//! [`LIFT_REQUIRES_PLANAR`](crate::LIFT_REQUIRES_PLANAR) (a curve/surface family
//! this build does not carry) or
//! [`LIFT_UNSUPPORTED`](crate::LIFT_UNSUPPORTED) (the body is structurally
//! unusable).
//!
//! # pcurves
//!
//! `solidcore` stores every edge use's trim curve in the face's `(u, v)` and
//! integrates over that trim for area, centroid and inertia. A **plane's** uses
//! get the exact `(u, v)` image of their line segment as a degree-1 spline, or the
//! exact projection of an arc's (or circle's) own NURBS form. A **non-planar**
//! face's uses come from `solidcore_ops::pcurve::pcurves_within`, which has exact
//! closed forms only and fits nothing: a use with no exact pcurve is refused by
//! name ([`PCURVE_NOT_EXACT`](crate::PCURVE_NOT_EXACT)) and never replaced by a
//! fitted trim or a plane assumption. Volume alone tolerates a placeholder
//! pcurve; the mass-properties area does not, and this was measured, not assumed.
//!
//! # Circles and arcs
//!
//! A [`BrepCurve::Arc`] is written as the record's `Curve::Circle` **label plus
//! its NURBS form** (the record requires both) with the edge's `interval` and both
//! endpoint vertices — that is how the record carries an open circle, because its
//! curves are unbounded and the edge's interval is the trim. A [`BrepCurve::Circle`]
//! is the same label with a whole-turn form and **one** vertex (its seam point).
//! The form is `solidcore`'s own quarter-grid arc rescaled to radians, so it
//! shares the face's parameterisation and the use's pcurve is exact by
//! construction.
//!
//! Two uses of one arc (the two faces sharing it) collapse to one record edge,
//! matched by the arc's **point set** (centre, radius and the arc's own midpoint),
//! which a reversed traversal of the same arc reproduces exactly; a full circle is
//! matched by its circle. The details live in [`arc`](super::arc).
//!
//! A `BrepFace::tolerance` cannot be written as such: the pinned record carries
//! one tolerance per body (plus per edge and vertex), not per face. It is
//! therefore *resolved* into the record's single tolerance — the largest stated
//! face tolerance when the body states none, or the body's own when it agrees
//! with every stated face within `1e-6` relative; a materially different face
//! tolerance is refused by name (`LIFT_UNSUPPORTED`) instead of being silently
//! dropped. `BrepData::tolerance` is what `record::lift` hands to `to_record`
//! (`None` there means our own `DEFAULT_TOLERANCE`, never the producer's number).
//!
//! # Canonical order
//!
//! The record requires entities in first-appearance order (upstream
//! `docs/native-brep.md`): shells, then faces walking regions → shells, then
//! edges walking faces → loops → oriented edges (backwards in a face whose first
//! side is reversed), then vertices walking edges start-then-end. This module
//! builds each list in exactly that walk, so the record validates without a
//! re-sorting pass — including the seam a cylinder/cone/sphere/torus face needs
//! (`record::curved`). Edges are deduplicated by their **unordered** endpoint pair
//! (arcs and circles by their point sets) and take their direction from the first
//! use that created them; vertices are deduplicated by exact bits.

use std::collections::HashMap;

use cad_domain::BrepData;
use solidcore_record::brep::{
    Brep as Record, EdgeUseRef, Face, Loop, OrientedFace, Region, Shell, VERSION,
};

use super::curved::{align_curved_frames, resolve_closed_circles, write_face_geometry, Pending};
use super::loops::{list_uses, plane_loops, surface_loops, ArcEdge, CircleEdge};
use super::surfaces;
use super::units::bits;
use crate::error::LiftRefusal;

/// Record tolerance (mm) used when a caller does not carry one: the value the
/// upstream record tests use and comfortably tighter than SMLib's default
/// intersection tolerance (`1e-5` mm).
pub(crate) const DEFAULT_TOLERANCE: f64 = 1e-7;

/// Neutral B-rep → record. Refuses a body this cut cannot express exactly.
pub(crate) fn to_record(brep: &BrepData, tolerance: f64) -> Result<Record, LiftRefusal> {
    if !tolerance.is_finite() || tolerance <= 0.0 || tolerance > 1e3 {
        return Err(LiftRefusal::unsupported(format!(
            "record tolerance must be finite and in (0, 1e3], got {tolerance}"
        )));
    }
    let placement = brep
        .placement
        .unwrap_or_else(cad_domain::BrepPlacement::identity);
    let mut baked = super::bake(brep, placement)?;
    // Boundary circles of a labelled curved face are relabelled onto that face's
    // frame **before** anything is written (see `curved::align_curved_frames`):
    // it is a parameterisation choice, not a geometric one, and it is what makes
    // the kernel's closed-form pcurve exact instead of offset by an angle.
    align_curved_frames(&mut baked)?;
    let brep = &baked;
    // A tolerance is a length, so baking a placed body's geometry into the record
    // scales it by the placement's uniform factor.
    let tolerance = tolerance * placement.scale;
    let tolerance = record_tolerance(brep, tolerance)?;

    if brep.shells.len() != 1 {
        return Err(LiftRefusal::unsupported(format!(
            "a body with {} shells cannot be lifted; this build lifts exactly one closed face \
             shell",
            brep.shells.len()
        )));
    }
    let shell = &brep.shells[0];
    if shell.faces.is_empty() {
        return Err(LiftRefusal::unsupported(
            "a shell without a face cannot be lifted",
        ));
    }

    let mut record = Record {
        version: VERSION,
        tolerance,
        regions: Vec::new(),
        shells: Vec::new(),
        faces: Vec::new(),
        edges: Vec::new(),
        vertices: Vec::new(),
    };
    let mut vertex_of: HashMap<[u64; 3], u32> = HashMap::new();
    let mut edge_of: HashMap<([u64; 3], [u64; 3]), u32> = HashMap::new();
    let mut arcs: Vec<ArcEdge> = Vec::new();
    let mut circles: Vec<CircleEdge> = Vec::new();
    let mut pending: Vec<Pending> = Vec::with_capacity(shell.faces.len());
    // A record index is a `u32`; the face count is bounded by the record's budget.
    let face_index = |index: usize| {
        u32::try_from(index)
            .map_err(|_| LiftRefusal::unsupported("more faces than a record index can name"))
    };

    for (index, face) in shell.faces.iter().enumerate() {
        face_index(index)?;
        let written = surfaces::write(face)?;
        let pending_face = write_face_geometry(
            &mut record,
            &mut vertex_of,
            &mut edge_of,
            &mut arcs,
            &mut circles,
            face,
            written,
        )?;
        // The face is pushed now (with no loops yet) so a later face's shared
        // circle can read its surface; the loops are filled in below.
        record.faces.push(Face {
            surface: pending_face.surface.clone(),
            nurbs: pending_face.nurbs.clone(),
            domain: pending_face.domain,
            loops: Vec::new(),
        });
        pending.push(pending_face);
    }

    // A full circle's traversal (which its one vertex cannot record) is resolved
    // from the planar face that shares it, before any pcurve is computed.
    resolve_closed_circles(&record, &mut pending)?;

    for (index, pending_face) in pending.into_iter().enumerate() {
        let loops = match pending_face.plane {
            Some(frame) => plane_loops(&record, &pending_face.raw, frame)?,
            None => surface_loops(
                &record,
                &pending_face.raw,
                shell.faces[index].id,
                &pending_face.surface,
                pending_face.parameters(),
                pending_face.domain,
                record.tolerance,
            )?,
        };
        if loops.is_empty() {
            return Err(LiftRefusal::unsupported(format!(
                "face {} has no boundary loop",
                shell.faces[index].id
            )));
        }
        record.faces[index].loops = loops;
    }

    let mut oriented = Vec::with_capacity(shell.faces.len());
    for (index, face) in shell.faces.iter().enumerate() {
        oriented.push(OrientedFace {
            face: face_index(index)?,
            forward: !face.reversed,
        });
    }
    record.shells.push(Shell::Faces {
        faces: oriented,
        wires: Vec::new(),
    });
    // A closed face shell separates two regions: it is the outer shell of the
    // bounded region inside it and listed by the infinite region around it.
    record.regions = vec![
        Region {
            shells: vec![0],
            void: false,
        },
        Region {
            shells: vec![0],
            void: false,
        },
    ];
    list_uses(&mut record);
    record.validate().map_err(|error| {
        LiftRefusal::unsupported(format!(
            "the lifted record is structurally invalid: {error}"
        ))
    })?;
    check_closed_shell(&record)?;
    check_loop_chain(&record)?;
    // Last: the kernel's own geometric agreement check (edge/vertex, pcurve
    // lifting, loop turn), which assumes a structurally valid record. This
    // catches a body whose topology passes `validate` but whose geometry does
    // not agree with it, before the kernel is asked to build a body from it.
    solidcore_ops::record_check::agreement(&record).map_err(|mismatch| {
        LiftRefusal::unsupported(format!(
            "the lifted record's geometry disagrees with its topology: {mismatch}"
        ))
    })?;
    Ok(record)
}

/// The one record tolerance a body's stated tolerances imply.
///
/// The pinned record carries **one** tolerance per body (plus per edge/vertex)
/// and none per face, so a stated face number can only be honoured by becoming
/// the body's, or it would vanish silently:
///
/// - **no body tolerance**: the largest stated face tolerance wins (that is the
///   coarsest number a producer asked for, not one we invented); when no face
///   states one either, `fallback` stands — which is the caller's own default
///   (`DEFAULT_TOLERANCE`), never attributed to the producer;
/// - **a body tolerance**: every stated face tolerance must agree with it within
///   `1e-6` relative. A face that differs materially is refused by name, because
///   the record cannot carry both numbers and silently widening or narrowing a
///   face's geometry check is not ours to decide.
fn record_tolerance(brep: &BrepData, fallback: f64) -> Result<f64, LiftRefusal> {
    let stated: Vec<(u32, f64)> = brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .filter_map(|face| face.tolerance.map(|tolerance| (face.id, tolerance)))
        .collect();
    for (id, tolerance) in &stated {
        if !tolerance.is_finite() || *tolerance <= 0.0 {
            return Err(LiftRefusal::unsupported(format!(
                "face {id} states tolerance {tolerance}; a tolerance must be finite and positive"
            )));
        }
    }
    let Some(body) = brep.tolerance else {
        let coarsest = stated.iter().map(|(_, tolerance)| *tolerance).fold(
            None::<f64>,
            |current, tolerance| {
                Some(current.map_or(tolerance, |current: f64| current.max(tolerance)))
            },
        );
        return Ok(coarsest.unwrap_or(fallback));
    };
    for (id, face) in &stated {
        let slack = 1e-6 * face.abs().max(body.abs());
        if (face - body).abs() > slack {
            return Err(LiftRefusal::unsupported(format!(
                "face {id} states tolerance {face} but the body states {body}; the record holds \
                 one tolerance per body and none per face, so one of the two would be lost"
            )));
        }
    }
    Ok(body)
}

/// Every edge is used exactly twice, by the two faces traversing it in opposite
/// directions **in the shell's outward orientation**: the signature of a closed,
/// consistently oriented shell.
///
/// `Brep::validate` proves the `uses` list matches the walk; it cannot prove the
/// walk visits each edge twice. A sheet mapped as a solid (one use per edge) or a
/// face whose orientation was flipped (both uses traversing the same way) passes
/// structural validation and then produces a *plausible but wrong* body — the
/// measured symptom was an area of 40 instead of 52 — so it is refused here.
///
/// The direction compared is the shell's, not the stored loop's: a face used
/// backwards (`OrientedFace::forward == false`) stores its loop relative to its
/// natural normal, so its stored traverse is the *reverse* of the shell's. The
/// kernel's own record of a box minus a corner has exactly such faces, and their
/// shared edges carry equal `upper` flags while the body is perfectly valid — so
/// `upper` equality alone is **not** the invariant here. Traversal in the shell's
/// orientation is, and it is also what a negative volume would reflect.
fn check_closed_shell(record: &Record) -> Result<(), LiftRefusal> {
    let uses_of_shell = match record.shells.first() {
        Some(Shell::Faces { faces, .. }) => faces.as_slice(),
        _ => {
            return Err(LiftRefusal::unsupported(
                "the lifted record has no single face shell",
            ));
        }
    };
    for (index, edge) in record.edges.iter().enumerate() {
        let [first, second] = edge.uses.as_slice() else {
            return Err(LiftRefusal::unsupported(format!(
                "edge {index} is used {} time(s); a closed solid's edge is used exactly twice \
                 (an open sheet, or a face listed twice)",
                edge.uses.len()
            )));
        };
        match (
            shell_traverse(record, uses_of_shell, first),
            shell_traverse(record, uses_of_shell, second),
        ) {
            (Some(a), Some(b)) if a != b => {}
            (Some(_), Some(_)) => {
                return Err(LiftRefusal::unsupported(format!(
                    "edge {index} is traversed the same way by both faces: the shell is not \
                     consistently oriented (a reversed face?)"
                )));
            }
            _ => {
                return Err(LiftRefusal::unsupported(format!(
                    "edge {index} has a use whose face is missing from the shell or listed \
                     twice, so its shell orientation cannot be resolved"
                )));
            }
        }
    }
    Ok(())
}

/// Whether a use walks its edge from interval min to max **in the shell's
/// orientation**.
///
/// The stored traversal is flipped when the shell uses the face backwards, i.e.
/// the result is `stored XNOR facing` (`stored == facing`); the invariant checked
/// by [`check_closed_shell`] is that the two uses of an edge disagree (`a != b`).
fn shell_traverse(
    record: &Record,
    uses_of_shell: &[OrientedFace],
    use_: &EdgeUseRef,
) -> Option<bool> {
    let facing = shell_facing(uses_of_shell, use_.face)?;
    let stored = match record
        .faces
        .get(use_.face as usize)?
        .loops
        .get(use_.r#loop as usize)?
    {
        Loop::Edges { edges } => edges.get(use_.position as usize)?.forward,
        Loop::Vertex { .. } => return None,
    };
    Some(stored == facing)
}

/// The shell's facing for a face, looked up by the face's **record index**.
///
/// This module pushes one oriented face per body face in order, so the face index
/// also works as a position — but a second producer must not be able to rely on
/// that and silently read another face's facing, so the lookup matches the index
/// and refuses (returns `None`) when the face is missing or listed twice.
fn shell_facing(uses_of_shell: &[OrientedFace], face: u32) -> Option<bool> {
    let mut facing = None;
    for oriented in uses_of_shell
        .iter()
        .filter(|oriented| oriented.face == face)
    {
        if facing.replace(oriented.forward).is_some() {
            // Listed twice in one shell: a use cannot say which side it is on.
            return None;
        }
    }
    facing
}

/// Every loop chains end to end: edge `k`'s traversed end is edge `k + 1`'s
/// traversed start, and the last end is the first start — compared by **exact
/// bits**, so the loop is one closed cycle of the same points the curves carry.
///
/// The record's own validation already chains each loop by vertex index; this
/// repeats the requirement on the record this module just built, so a builder
/// mistake is refused with a geometric detail (which vertex ends where) instead
/// of only the structural message.
fn check_loop_chain(record: &Record) -> Result<(), LiftRefusal> {
    for (face_index, face) in record.faces.iter().enumerate() {
        for (loop_index, loop_) in face.loops.iter().enumerate() {
            let Loop::Edges { edges } = loop_ else {
                continue;
            };
            let mut previous: Option<u32> = None;
            let mut first_start: Option<u32> = None;
            for use_ in edges {
                let Some(edge) = record.edges.get(use_.edge as usize) else {
                    return Err(LiftRefusal::unsupported(format!(
                        "face {face_index} loop {loop_index} references missing edge {}",
                        use_.edge
                    )));
                };
                let (start, end) = if use_.forward {
                    (edge.vertices[0], edge.vertices[1])
                } else {
                    (edge.vertices[1], edge.vertices[0])
                };
                if let Some(previous) = previous {
                    if !same_vertex_point(record, previous, start) {
                        return Err(LiftRefusal::unsupported(format!(
                            "face {face_index} loop {loop_index} is not chained: vertex \
                             {previous} ends a use but the next starts at vertex {start}"
                        )));
                    }
                } else {
                    first_start = Some(start);
                }
                previous = Some(end);
            }
            match (previous, first_start) {
                (Some(last), Some(first)) if same_vertex_point(record, last, first) => {}
                (Some(last), Some(first)) => {
                    return Err(LiftRefusal::unsupported(format!(
                        "face {face_index} loop {loop_index} is open: it ends at vertex {last} \
                         and starts at vertex {first}"
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Two vertices carry bit-identical geometry (the record's exact-equality rule).
fn same_vertex_point(record: &Record, a: u32, b: u32) -> bool {
    match (
        record.vertices.get(a as usize),
        record.vertices.get(b as usize),
    ) {
        (Some(a), Some(b)) => bits(a.point) == bits(b.point),
        _ => false,
    }
}
