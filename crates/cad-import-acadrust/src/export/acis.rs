//! Exact ACIS writer for the planar subset of a neutral B-rep.
//!
//! This is the **only** place the exporter builds an ACIS document. It is
//! deliberately narrow and refuses everything it cannot represent exactly:
//!
//! * exactly **one** shell of faces (a void would be a second shell);
//! * every face supported by a plane (`BrepSurface::Plane` only);
//! * every loop segment a straight line (`BrepCurve::Line` only);
//! * a **proper** similarity placement (no shear, no mirror), baked into the
//!   points and surface frames rather than written as a `transform` record.
//!
//! Anything else is an [`AcisWriteRefusal`] with a stable code, so the caller
//! falls back to the POLYFACE display cache and names the reason instead of
//! inventing a payload.
//!
//! A **reflection** (`det < 0`) is refused rather than written: mirroring the
//! points alone would invert the shell (its outward normals would point into the
//! material), and negating the plane normals instead is a second convention the
//! reader would have to agree with. A negative scale is a `Placement` refusal,
//! like shear.
//!
//! ## Topology built here
//!
//! `body → lump → shell → face → loop → coedge → edge → vertex → point`, with
//! one plane surface per face, one straight curve per geometric edge, and one
//! vertex per geometric point. Geometric edges are welded across the two faces
//! that share them (each edge has exactly two coedges, one partner each) and a
//! closed two-manifold shell is required: an edge used once (an open sheet) or
//! more than twice (non-manifold) is refused, never written half-linked.
//!
//! The face sense is the B-rep's own `reversed` flag, the coedge sense is the
//! loop's direction relative to its (canonical, first-use) edge direction, and
//! sidedness is `single` because only a closed shell is written.
//!
//! Every loop is also checked against its face's **baked natural normal**: the
//! loop's Newell vector must lie along the plane normal, with the sign the face
//! sense requires (a `forward` face winds counter-clockwise about the normal, a
//! `reversed` one about its negative). A single inverted face — one lone loop
//! wound the wrong way — is refused here instead of being written as an
//! inside-out shell.
//!
//! **Evidence boundary**: the payload is read back by this crate's own SAT/SAB
//! reader only. No third-party CAD application was used to verify it.
//!
//! The result is a [`SatDocument`]. The DXF export serialises it as SAT text and
//! carries it inline; the DWG export serialises the *same* document with
//! `SabWriter` and attaches it as a binary SAB blob. Both encodings come from
//! this one document, so the reachable subset is identical for the two formats.

use std::collections::HashMap;

use acadrust::entities::acis::{SatDocument, SatPointer, Sense, Sidedness};
use cad_domain::{BrepCurve, BrepData, BrepFace, BrepPlacement, BrepSurface, Point3};

/// Why a B-rep is not written as ACIS.
///
/// Each variant maps to one stable [`AcisWriteRefusal::code`], which the export
/// report records verbatim; the message names the geometry that caused it.
/// These refusals are format-independent: the same B-rep is refused, with the
/// same code, in a DXF and in a DWG export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AcisWriteRefusal {
    /// A face is supported by something other than a plane.
    CurvedFace { face_id: u32 },
    /// A planar face has a boundary segment that is not a straight line.
    NonLineCurve { face_id: u32 },
    /// The body has no shell, or a shell without faces.
    NoShell,
    /// The body has more than one shell (a void needs an inner shell).
    MultipleShells { n: usize },
    /// The placement is not a similarity (shear / non-uniform scale), or is
    /// otherwise unusable.
    Placement,
    /// Anything else the writer will not emit (degenerate or non-manifold
    /// topology, non-finite geometry, ...).
    Invalid(String),
}

impl AcisWriteRefusal {
    /// Stable machine code recorded in the export report.
    pub(super) fn code(&self) -> &'static str {
        match self {
            AcisWriteRefusal::CurvedFace { .. } => "acis.curved_face_not_written",
            AcisWriteRefusal::NonLineCurve { .. } => "acis.non_line_curve_not_written",
            AcisWriteRefusal::NoShell => "acis.no_shell_not_written",
            AcisWriteRefusal::MultipleShells { .. } => "acis.multi_shell_not_written",
            AcisWriteRefusal::Placement => "acis.placement_not_written",
            AcisWriteRefusal::Invalid(_) => "acis.invalid_topology_not_written",
        }
    }

    /// Human detail for the report, naming the offending face/edge.
    pub(super) fn message(&self) -> String {
        match self {
            AcisWriteRefusal::CurvedFace { face_id } => format!(
                "face {face_id} is not supported by a plane; this writer emits ACIS for \
                 planar-faced solids only (acis.planar_only)"
            ),
            AcisWriteRefusal::NonLineCurve { face_id } => format!(
                "face {face_id} has a boundary loop segment that is not a straight line; this \
                 writer emits ACIS for line-bounded planar faces only (acis.planar_only)"
            ),
            AcisWriteRefusal::NoShell => {
                "the body has no shell of faces to write as a solid".to_string()
            }
            AcisWriteRefusal::MultipleShells { n } => format!(
                "the body has {n} shells (a void or a multi-lump body); this writer emits one \
                 closed shell per 3DSOLID (acis.single_shell_only)"
            ),
            AcisWriteRefusal::Placement => {
                "the body placement is not a proper similarity (it has shear, a non-uniform \
                 scale or a reflection with a negative determinant) or is not finite; baking it \
                 into the SAT points and surface frames would invert the shell or skew it"
                    .to_string()
            }
            AcisWriteRefusal::Invalid(detail) => detail.clone(),
        }
    }
}

/// Build a SAT document for a single-shell, planar, line-bounded solid.
pub(super) fn brep_to_sat(brep: &BrepData) -> Result<SatDocument, AcisWriteRefusal> {
    let shell = match brep.shells.len() {
        0 => return Err(AcisWriteRefusal::NoShell),
        1 => &brep.shells[0],
        n => return Err(AcisWriteRefusal::MultipleShells { n }),
    };
    if shell.faces.is_empty() {
        return Err(AcisWriteRefusal::NoShell);
    }

    let placement = brep.placement.filter(|value| !value.is_identity());
    if let Some(placement) = placement {
        check_placement(&placement)?;
    }
    let bake_point = |p: Point3| match placement {
        Some(placement) => placement.apply(p),
        None => p,
    };
    let bake_direction = |d: Point3| match placement {
        Some(placement) => placement.apply_direction(d),
        None => d,
    };

    // Pass 1: validate the subset and measure the baked geometry (the welding
    // tolerance is relative to the body, not to the drawing).
    let mut min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for face in &shell.faces {
        match &face.surface {
            BrepSurface::Plane {
                origin,
                normal,
                u_dir,
            } => {
                if !finite(*origin) || !finite(*normal) || !finite(*u_dir) {
                    return Err(invalid(format!(
                        "face {} has a non-finite plane frame",
                        face.id
                    )));
                }
                let normal_length = length(*normal);
                let u_length = length(*u_dir);
                if normal_length <= 1e-12 || u_length <= 1e-12 {
                    return Err(invalid(format!(
                        "face {} has a degenerate plane normal or u direction",
                        face.id
                    )));
                }
                // A planar face frame is an orthonormal (normal, u_dir) pair. A
                // skew frame would write a plane whose SAT parameterization does
                // not match the boundary, so it is refused, not silently
                // orthogonalised into a different plane.
                let cosine = dot(*normal, *u_dir) / (normal_length * u_length);
                if cosine.abs() > 1e-9 {
                    return Err(invalid(format!(
                        "face {} has a plane frame whose u direction is not perpendicular to its \
                         normal (cosine {cosine:.3e}); a skewed plane frame cannot be written",
                        face.id
                    )));
                }
            }
            _ => return Err(AcisWriteRefusal::CurvedFace { face_id: face.id }),
        }
        if face.loops.is_empty() {
            return Err(invalid(format!(
                "face {} has no boundary loop; an unbounded planar face is not a solid boundary",
                face.id
            )));
        }
        for loop_ in &face.loops {
            if loop_.edges.is_empty() {
                return Err(invalid(format!(
                    "face {} has an empty boundary loop",
                    face.id
                )));
            }
            for curve in &loop_.edges {
                let BrepCurve::Line { start, end } = curve else {
                    return Err(AcisWriteRefusal::NonLineCurve { face_id: face.id });
                };
                let (start, end) = (bake_point(*start), bake_point(*end));
                if !finite(start) || !finite(end) {
                    return Err(invalid(format!(
                        "face {} has a boundary point that is not finite",
                        face.id
                    )));
                }
                for point in [start, end] {
                    min.x = min.x.min(point.x);
                    min.y = min.y.min(point.y);
                    min.z = min.z.min(point.z);
                    max.x = max.x.max(point.x);
                    max.y = max.y.max(point.y);
                    max.z = max.z.max(point.z);
                }
            }
        }
    }
    let diagonal = length(Point3 {
        x: max.x - min.x,
        y: max.y - min.y,
        z: max.z - min.z,
    });
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return Err(invalid(
            "the body's boundary points span no distance; nothing can be written".to_string(),
        ));
    }
    let tolerance = 1e-9 * diagonal;

    // Pass 2: weld points and geometric edges, and record each coedge use.
    let mut welder = PointWelder::new(tolerance);
    let mut edges: Vec<EdgeRecord> = Vec::new();
    let mut edge_index: HashMap<(usize, usize), usize> = HashMap::new();
    // Per geometric edge, the global coedge ordinals that use it.
    let mut edge_uses: Vec<Vec<usize>> = Vec::new();
    // Per coedge ordinal, its geometric edge and whether it runs along the
    // canonical edge direction.
    let mut coedge_edge: Vec<usize> = Vec::new();
    let mut coedge_forward: Vec<bool> = Vec::new();
    let mut faces: Vec<BuiltFace> = Vec::new();

    for face in &shell.faces {
        let BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } = &face.surface
        else {
            unreachable!("pass 1 refused every non-planar face");
        };
        let surface = SurfaceRecord {
            origin: bake_point(*origin),
            // Baked and re-normalised: the printed frame is an orthonormal pair
            // even when the source vectors were only approximately unit length.
            normal: unit(bake_direction(*normal)),
            u_dir: unit(bake_direction(*u_dir)),
        };
        let mut loops = Vec::with_capacity(face.loops.len());
        let mut signed_areas: Vec<f64> = Vec::with_capacity(face.loops.len());
        for loop_ in &face.loops {
            let first_use = coedge_edge.len();
            let mut edges_of_loop: Vec<(usize, bool)> = Vec::with_capacity(loop_.edges.len());
            let mut ring: Vec<Point3> = Vec::with_capacity(loop_.edges.len());
            for curve in &loop_.edges {
                let BrepCurve::Line { start, end } = curve else {
                    unreachable!("pass 1 refused every non-line curve");
                };
                let start = bake_point(*start);
                let end = bake_point(*end);
                let Some(start_id) = welder.weld(start) else {
                    return Err(invalid(
                        "a boundary point is outside the representable range".to_string(),
                    ));
                };
                let Some(end_id) = welder.weld(end) else {
                    return Err(invalid(
                        "a boundary point is outside the representable range".to_string(),
                    ));
                };
                if start_id == end_id {
                    return Err(invalid(format!(
                        "face {} has a boundary segment shorter than the welding tolerance \
                         ({tolerance:.3e}); it cannot become an edge",
                        face.id
                    )));
                }
                // Canonical direction: the first face that uses the edge decides
                // it, and the other face traverses it backwards (Reversed).
                let key = if start_id < end_id {
                    (start_id, end_id)
                } else {
                    (end_id, start_id)
                };
                let edge_id = *edge_index.entry(key).or_insert_with(|| {
                    let id = edges.len();
                    edges.push(EdgeRecord {
                        start: key.0,
                        end: key.1,
                    });
                    edge_uses.push(Vec::new());
                    id
                });
                let forward = start_id == key.0 && end_id == key.1;
                let ordinal = coedge_edge.len();
                if edges_of_loop
                    .iter()
                    .any(|(other, _)| coedge_edge.get(*other).copied() == Some(edge_id))
                {
                    return Err(invalid(format!(
                        "face {} traverses one geometric edge twice in the same loop; a \
                         zero-width spike is not a writable boundary",
                        face.id
                    )));
                }
                coedge_edge.push(edge_id);
                coedge_forward.push(forward);
                edge_uses[edge_id].push(ordinal);
                edges_of_loop.push((ordinal, forward));
                ring.push(start);
            }
            signed_areas.push(loop_signed_area(face, &ring, surface.normal)?);
            loops.push(BuiltLoop {
                record: SatPointer::NULL,
                next: SatPointer::NULL,
                first_use,
                coedges: edges_of_loop,
            });
        }
        check_face_orientation(face, &signed_areas)?;
        faces.push(BuiltFace {
            surface,
            reversed: face.reversed,
            first_loop: SatPointer::NULL,
            loops,
        });
    }

    for (edge_id, uses) in edge_uses.iter().enumerate() {
        if uses.len() != 2 {
            return Err(invalid(format!(
                "the edge between welded points ({}, {}, {}) and ({}, {}, {}) is used by {} \
                 coedge(s); only a closed two-manifold shell (exactly two uses per edge) can be \
                 written",
                welder.points[edges[edge_id].start].x,
                welder.points[edges[edge_id].start].y,
                welder.points[edges[edge_id].start].z,
                welder.points[edges[edge_id].end].x,
                welder.points[edges[edge_id].end].y,
                welder.points[edges[edge_id].end].z,
                uses.len()
            )));
        }
        if coedge_forward[uses[0]] == coedge_forward[uses[1]] {
            return Err(invalid(format!(
                "the two faces sharing the edge between welded points ({}, {}, {}) and \
                 ({}, {}, {}) traverse it in the same direction; the shell is not consistently \
                 oriented",
                welder.points[edges[edge_id].start].x,
                welder.points[edges[edge_id].start].y,
                welder.points[edges[edge_id].start].z,
                welder.points[edges[edge_id].end].x,
                welder.points[edges[edge_id].end].y,
                welder.points[edges[edge_id].end].z
            )));
        }
    }

    // Record indices are position-indexed: create the records group by group so
    // every forward pointer (coedge → loop, loop → face, face → shell) is known.
    let mut doc = SatDocument::new_body();
    let points_start = doc.records.len() as i32;
    let vertices_start = points_start + welder.points.len() as i32;
    let curves_start = vertices_start + welder.points.len() as i32;
    let edges_start = curves_start + edges.len() as i32;
    let surfaces_start = edges_start + edges.len() as i32;
    let coedges_start = surfaces_start + faces.len() as i32;
    let loops_start = coedges_start + coedge_edge.len() as i32;
    let loops_total: usize = faces.iter().map(|face| face.loops.len()).sum();
    let faces_start = loops_start + loops_total as i32;
    let shell_index = faces_start + faces.len() as i32;
    let lump_index = shell_index + 1;

    for point in &welder.points {
        doc.add_point(point.x, point.y, point.z);
    }
    for index in 0..welder.points.len() {
        doc.add_vertex(
            SatPointer::NULL,
            SatPointer::new(points_start + index as i32),
        );
    }
    for edge in &edges {
        let start = welder.points[edge.start];
        let end = welder.points[edge.end];
        let direction = normalized(sub(end, start));
        doc.add_straight_curve(
            [start.x, start.y, start.z],
            [direction.x, direction.y, direction.z],
        );
    }
    for (index, edge) in edges.iter().enumerate() {
        let start = welder.points[edge.start];
        let end = welder.points[edge.end];
        doc.add_edge(
            SatPointer::new(vertices_start + edge.start as i32),
            0.0,
            SatPointer::new(vertices_start + edge.end as i32),
            length(sub(end, start)),
            SatPointer::NULL,
            SatPointer::new(curves_start + index as i32),
            Sense::Forward,
        );
    }
    for face in &faces {
        doc.add_plane_surface(
            [
                face.surface.origin.x,
                face.surface.origin.y,
                face.surface.origin.z,
            ],
            [
                face.surface.normal.x,
                face.surface.normal.y,
                face.surface.normal.z,
            ],
            [
                face.surface.u_dir.x,
                face.surface.u_dir.y,
                face.surface.u_dir.z,
            ],
        );
    }

    // Loop layout: assign every loop its record index and its next-loop pointer
    // from the known position, so no later lookup can silently fall back to a
    // wrong record. The loops of one face form their own chain, ending at NULL.
    let mut cursor = loops_start;
    for face in &mut faces {
        let count = face.loops.len();
        face.first_loop = SatPointer::new(cursor);
        for (position, loop_) in face.loops.iter_mut().enumerate() {
            loop_.record = SatPointer::new(cursor);
            loop_.next = if position + 1 < count {
                SatPointer::new(cursor + 1)
            } else {
                SatPointer::NULL
            };
            cursor += 1;
        }
    }

    // Coedges: the ring is the loop's own coedge run, and the partner is the
    // other (and only other) use of the same geometric edge — both validated
    // above, so this is a direct lookup with no fallback.
    for face in &faces {
        for loop_ in &face.loops {
            let count = loop_.coedges.len();
            for (position, (ordinal, forward)) in loop_.coedges.iter().enumerate() {
                let next = if position + 1 < count {
                    *ordinal + 1
                } else {
                    loop_.first_use
                };
                let prev = if position == 0 {
                    loop_.first_use + count - 1
                } else {
                    *ordinal - 1
                };
                let uses = &edge_uses[coedge_edge[*ordinal]];
                let partner = if uses[0] == *ordinal {
                    uses[1]
                } else {
                    uses[0]
                };
                doc.add_coedge(
                    SatPointer::new(coedges_start + next as i32),
                    SatPointer::new(coedges_start + prev as i32),
                    SatPointer::new(coedges_start + partner as i32),
                    SatPointer::new(edges_start + coedge_edge[*ordinal] as i32),
                    if *forward {
                        Sense::Forward
                    } else {
                        Sense::Reversed
                    },
                    loop_.record,
                );
            }
        }
    }
    for (face_index, face) in faces.iter().enumerate() {
        let owner = SatPointer::new(faces_start + face_index as i32);
        for loop_ in &face.loops {
            doc.add_loop(
                loop_.next,
                SatPointer::new(coedges_start + loop_.first_use as i32),
                owner,
            );
        }
    }
    for (face_index, face) in faces.iter().enumerate() {
        let next_face = if face_index + 1 < faces.len() {
            SatPointer::new(faces_start + face_index as i32 + 1)
        } else {
            SatPointer::NULL
        };
        doc.add_face(
            next_face,
            face.first_loop,
            SatPointer::new(shell_index),
            SatPointer::new(surfaces_start + face_index as i32),
            if face.reversed {
                Sense::Reversed
            } else {
                Sense::Forward
            },
            Sidedness::Single,
        );
    }
    doc.add_shell(SatPointer::new(faces_start), SatPointer::new(lump_index));
    doc.add_lump(SatPointer::new(shell_index), SatPointer::new(0));
    if let Some(body) = doc.record_mut(0) {
        body.tokens[1] = acadrust::entities::acis::SatToken::Pointer(SatPointer::new(lump_index));
    }
    doc.complete_brep_links();

    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(invalid(format!(
            "the built SAT document failed its own pointer validation ({} error(s))",
            errors.len()
        )));
    }
    Ok(doc)
}

/// One welded geometric edge, as point indices in the welder.
struct EdgeRecord {
    start: usize,
    end: usize,
}

/// A baked plane surface frame.
struct SurfaceRecord {
    origin: Point3,
    normal: Point3,
    u_dir: Point3,
}

struct BuiltLoop {
    /// Record index of this loop (assigned when the record layout is known).
    record: SatPointer,
    /// Next loop of the same face, or [`SatPointer::NULL`] for the last one.
    next: SatPointer,
    /// Ordinal of this loop's first coedge use.
    first_use: usize,
    /// `(coedge ordinal, forward relative to the canonical edge direction)`.
    coedges: Vec<(usize, bool)>,
}

struct BuiltFace {
    surface: SurfaceRecord,
    reversed: bool,
    /// Record index of the face's first loop (assigned with the layout).
    first_loop: SatPointer,
    loops: Vec<BuiltLoop>,
}

fn invalid(detail: String) -> AcisWriteRefusal {
    AcisWriteRefusal::Invalid(detail)
}

fn check_placement(placement: &BrepPlacement) -> Result<(), AcisWriteRefusal> {
    if !placement.scale.is_finite() || placement.scale <= 0.0 {
        return Err(AcisWriteRefusal::Placement);
    }
    if !finite(placement.translation) {
        return Err(AcisWriteRefusal::Placement);
    }
    // `M·Mᵀ = I` is the test for a rotation or reflection (no shear, uniform
    // scale). A non-similarity cannot be baked into points and unit directions.
    for (row, values) in placement.matrix.iter().enumerate() {
        for (column, other) in placement.matrix.iter().enumerate() {
            let dot: f64 = (0..3).map(|k| values[k] * other[k]).sum();
            let expected = if row == column { 1.0 } else { 0.0 };
            if (dot - expected).abs() > 1e-9 {
                return Err(AcisWriteRefusal::Placement);
            }
        }
    }
    // A reflection (`det < 0`) is a similarity by that test, but baking it would
    // turn the shell inside out: the same face loop would then wind clockwise
    // about its outward normal. Refuse it rather than pick a second normal
    // convention the reader would have to agree with.
    let m = &placement.matrix;
    let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if determinant.is_nan() || determinant <= 0.0 {
        return Err(AcisWriteRefusal::Placement);
    }
    Ok(())
}

/// Signed double area of one loop about the face plane normal.
///
/// The loop's Newell vector must lie in the face plane (a loop that does not is
/// not a boundary of this face) and must enclose area. The **sign** is checked
/// per face in [`check_face_orientation`], because an inner (hole) loop winds
/// the opposite way to its face's outer loop by construction.
fn loop_signed_area(
    face: &BrepFace,
    ring: &[Point3],
    normal: Point3,
) -> Result<f64, AcisWriteRefusal> {
    let area = newell(ring);
    let area_length = length(area);
    if !area_length.is_finite() || area_length <= 0.0 {
        return Err(invalid(format!(
            "face {} has a boundary loop that encloses no area; a degenerate loop is not a \
             writable boundary",
            face.id
        )));
    }
    let cosine = dot(area, normal) / area_length;
    if 1.0 - cosine.abs() > 1e-9 {
        return Err(invalid(format!(
            "face {} has a boundary loop that does not lie in its plane (Newell direction is \
             {cosine:.6} off the plane normal)",
            face.id
        )));
    }
    Ok(dot(area, normal))
}

/// The face's loops must enclose material on the side its sense claims.
///
/// The **outer** loop (the largest by absolute area) winds counter-clockwise
/// about the plane normal for a `forward` face and clockwise for a `reversed`
/// one. Inner loops wind the other way (a hole), and a face's net signed area is
/// therefore positive in its outward sense. Checking the outer loop is what
/// catches a single face written inside-out; a plain "every loop has the same
/// sign" rule would refuse every face with a hole.
fn check_face_orientation(face: &BrepFace, signed_areas: &[f64]) -> Result<(), AcisWriteRefusal> {
    if signed_areas.is_empty() {
        return Err(invalid(format!("face {} has no loop", face.id)));
    }
    let required = if face.reversed { -1.0 } else { 1.0 };
    let outer = signed_areas
        .iter()
        .enumerate()
        .max_by(|left, right| {
            left.1
                .abs()
                .partial_cmp(&right.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);
    if signed_areas[outer] * required <= 0.0 {
        return Err(invalid(format!(
            "face {} has its outer boundary loop wound against the face's outward normal; the \
             face is inverted relative to the shell",
            face.id
        )));
    }
    let net: f64 = signed_areas.iter().sum();
    if net * required <= 0.0 {
        return Err(invalid(format!(
            "face {} has loops that enclose no net area in the face's outward sense; the hole \
             loops do not oppose the outer loop",
            face.id
        )));
    }
    Ok(())
}

/// Newell's area vector of a closed polygon ring; its direction is the ring's
/// normal and its length is twice the ring's area.
fn newell(ring: &[Point3]) -> Point3 {
    let mut normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    normal
}

/// Weld coincident points so one geometric point becomes one `point`/`vertex`.
///
/// A grid of cells one tolerance wide is searched in the 3×3×3 neighbourhood, so
/// a point that lands on the far side of a cell boundary still meets its
/// neighbour; the search stays O(1) per point instead of O(points²).
struct PointWelder {
    tolerance: f64,
    cell: f64,
    points: Vec<Point3>,
    grid: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl PointWelder {
    fn new(tolerance: f64) -> Self {
        PointWelder {
            tolerance,
            cell: tolerance.max(f64::MIN_POSITIVE),
            points: Vec::new(),
            grid: HashMap::new(),
        }
    }

    fn weld(&mut self, point: Point3) -> Option<usize> {
        if !finite(point) {
            return None;
        }
        let cell = self.cell;
        let key = (
            (point.x / cell).floor(),
            (point.y / cell).floor(),
            (point.z / cell).floor(),
        );
        if !key.0.is_finite() || !key.1.is_finite() || !key.2.is_finite() {
            return None;
        }
        let key = (key.0 as i64, key.1 as i64, key.2 as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let neighbor = (key.0 + dx, key.1 + dy, key.2 + dz);
                    if let Some(indices) = self.grid.get(&neighbor) {
                        for &index in indices {
                            if length(sub(self.points[index], point)) <= self.tolerance {
                                return Some(index);
                            }
                        }
                    }
                }
            }
        }
        let index = self.points.len();
        self.points.push(point);
        self.grid.entry(key).or_default().push(index);
        Some(index)
    }
}

fn finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn length(vector: Point3) -> f64 {
    (vector.x * vector.x + vector.y * vector.y + vector.z * vector.z).sqrt()
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Unit vector; a vanished vector stays zero (callers check the length first).
fn unit(vector: Point3) -> Point3 {
    normalized(vector)
}

fn normalized(vector: Point3) -> Point3 {
    let length = length(vector);
    if length > 1e-12 {
        Point3 {
            x: vector.x / length,
            y: vector.y / length,
            z: vector.z / length,
        }
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}
