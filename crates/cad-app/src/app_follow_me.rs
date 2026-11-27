//! `FOLLOWME`: sweep one closed planar profile along an open straight path.
//!
//! SketchUp's Follow Me: select a closed profile loop **and** an open path, run
//! the tool, and one new mesh body appears. The profile is placed at the path's
//! start perpendicular to the first segment, projected into the angle-bisecting
//! plane at every bend (the mitre a SketchUp user sees), and both ends capped.
//!
//! # Why a mesh, not an exact B-rep
//!
//! The result is an `AcDbSubDMesh` with per-face identity, not an exact
//! `AcDb3dSolid`: the pinned kernel cannot skin a sheared mitre section (it
//! returns an invalid shell) and a rigid section pair pinches a 90° bend to
//! volume 6 instead of 8, so an exact mitred elbow is not expressible through
//! this seam. SketchUp's own result is a face soup, and `face_sources` keeps
//! `PRESSPULL`/`MOVE` working on each wall and cap. The command needs no kernel,
//! so it works on every carrier; the mesh is watertight and outward-oriented.
//!
//! # Inputs
//!
//! * **Profile**: one closed straight `LWPOLYLINE`, a `LINE` ring, or a `CIRCLE`
//!   (sampled as a 48-gon). It must be planar, non-degenerate and
//!   non-self-intersecting; arcs/bulged segments are refused.
//! * **Path**: one open straight `LWPOLYLINE` or an open `LINE` chain, at least
//!   two distinct points, no zero-length segment, not closed.
//!
//! Both rows stay exactly as they were: one insert, one transaction, one undo.
//!
//! # Refusals
//!
//! `followme.requires_payload`, `followme.selection_required`,
//! `followme.not_addressable`, `followme.not_model_space`,
//! `followme.unsupported_entity`, `followme.profile_missing`,
//! `followme.profile_multiple`, `followme.profile_unsupported`,
//! `followme.profile_degenerate`, `followme.profile_not_planar`,
//! `followme.profile_self_intersecting`, `followme.path_missing`,
//! `followme.path_multiple`, `followme.path_unsupported`,
//! `followme.path_degenerate`, `followme.path_closed`, `followme.path_reversal`,
//! `followme.ambiguous`, `followme.mesh_invalid`.

use super::*;
use crate::app_extrude::{geometry_name, invalid};
use cad_domain::{DocumentId, Mesh, SelectionRef};

/// The per-face identity prefix of a swept wall quad.
const WALL_PREFIX: &str = "followme.wall";
/// The per-face identity of the two end caps.
const CAP_START: &str = "followme.cap.start";
const CAP_END: &str = "followme.cap.end";

/// How far a profile point may leave the profile's plane, relative to its size.
const PLANAR_TOLERANCE: f64 = 1e-7;

/// A profile/path pair extracted from the selection.
struct SweepInput {
    profile: Vec<Point3>,
    path: Vec<Point3>,
}

impl Application {
    /// `FOLLOWME`: sweep the selected profile along the selected path.
    ///
    /// Every refusal and every validation happens before the insert, so a failed
    /// run bumps no revision and records no undo entry.
    pub(crate) fn follow_me(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let refs = match &command.payload {
            CommandPayload::FollowMeProfile(refs) => refs.as_slice(),
            _ => return Err(invalid(
                "followme.requires_payload",
                "FOLLOWME takes the host-built FollowMeProfile { refs } payload; it is started \
                     by its pre-selection tool, never by a bare typed line",
            )),
        };
        let input = self.sweep_input(command.document, refs)?;
        let mesh = sweep_mesh(&input.profile, &input.path)?;
        let summary = MeshSummary::of(&mesh);
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Mesh(mesh),
            crate::app_drawing::mesh_solids::MESH_TYPE_KEY,
            "follow me",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "followme.applied".into(),
            message: format!(
                "FOLLOWME: swept a {}-vertex profile along a {}-segment path into a {} triangle \
                 mesh",
                input.profile.len(),
                input.path.len() - 1,
                summary.triangles
            ),
        });
        Ok(outcome)
    }

    /// Partition the selected top-level rows into one profile loop and one path.
    fn sweep_input(&self, document: DocumentId, refs: &[SelectionRef]) -> CadResult<SweepInput> {
        if refs.is_empty() {
            return Err(invalid(
                "followme.selection_required",
                "FOLLOWME needs a closed profile and an open path",
            ));
        }
        let drawing = self
            .workspace
            .documents
            .get(&document)
            .ok_or_else(|| invalid("followme.selection_changed", "document not open"))?
            .drawing
            .as_ref();
        let mut seen: Vec<EntityId> = Vec::new();
        let mut closed_polylines: Vec<Vec<Point3>> = Vec::new();
        let mut circles: Vec<Vec<Point3>> = Vec::new();
        let mut open_polylines: Vec<Vec<Point3>> = Vec::new();
        let mut lines: Vec<(Point3, Point3)> = Vec::new();
        for reference in refs {
            if reference.document != document {
                return Err(invalid(
                    "followme.not_addressable",
                    "the selected row belongs to another document",
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "followme.not_addressable",
                    "FOLLOWME reads whole top-level entities, never a block member or a \
                     sub-element",
                ));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                invalid(
                    "followme.selection_changed",
                    format!("entity {} does not exist any more", reference.entity.0),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "followme.not_model_space",
                    format!("entity {} is not in model space", entity.id.0),
                ));
            }
            match &entity.geometry {
                SemanticGeometry::Line { start, end } => lines.push((*start, *end)),
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                    ..
                } => {
                    if points.len() < 2 {
                        return Err(invalid(
                            "followme.profile_degenerate",
                            format!("polyline {} has fewer than two vertices", entity.id.0),
                        ));
                    }
                    if bulges.iter().any(|bulge| bulge.abs() > 1e-12) {
                        return Err(invalid(
                            if *closed {
                                "followme.profile_unsupported"
                            } else {
                                "followme.path_unsupported"
                            },
                            format!(
                                "polyline {} has arc segments; FOLLOWME accepts straight segments \
                                 only",
                                entity.id.0
                            ),
                        ));
                    }
                    if *closed {
                        closed_polylines.push(points.clone());
                    } else {
                        open_polylines.push(points.clone());
                    }
                }
                // A CIRCLE is a profile, sampled as a polygonal ring: a projected
                // circle is an ellipse, which no `BrepCurve` carries, and the
                // mesh wall is faceted anyway (a SketchUp circle is a polygon
                // too). `CIRCLE_SEGMENTS` is the documented tessellation.
                SemanticGeometry::Circle {
                    center,
                    normal,
                    radius,
                } => {
                    if !radius.is_finite() || *radius <= 0.0 {
                        return Err(invalid(
                            "followme.profile_degenerate",
                            format!("circle {} has a non-positive radius", entity.id.0),
                        ));
                    }
                    let axis = normalize(*normal).ok_or_else(|| {
                        invalid(
                            "followme.profile_degenerate",
                            format!("circle {} has no plane", entity.id.0),
                        )
                    })?;
                    circles.push(sample_circle(*center, axis, *radius));
                }
                other => {
                    return Err(invalid(
                        "followme.unsupported_entity",
                        format!(
                            "entity {} is {} ({}); FOLLOWME accepts LINE, closed straight \
                             LWPOLYLINE and open straight LWPOLYLINE rows",
                            entity.id.0,
                            entity.object.type_key,
                            geometry_name(other)
                        ),
                    ))
                }
            }
        }

        if closed_polylines.len() + circles.len() > 1 {
            return Err(invalid(
                "followme.profile_multiple",
                "FOLLOWME sweeps one closed profile at a time; more than one closed profile is \
                 selected",
            ));
        }
        if open_polylines.len() > 1 {
            return Err(invalid(
                "followme.path_multiple",
                "FOLLOWME follows one open path at a time; more than one open polyline is selected",
            ));
        }

        let profile = match closed_polylines.pop().or_else(|| circles.pop()) {
            Some(points) => points,
            None => {
                // No closed polyline: the LINE rows must be the profile ring and an
                // open polyline must be the path (otherwise both roles would be
                // LINE rows and the attribution would be a guess).
                let Some(path) = open_polylines.pop() else {
                    return Err(invalid(
                        "followme.profile_missing",
                        "FOLLOWME needs one closed profile (a closed straight LWPOLYLINE or a LINE \
                         ring) plus an open path",
                    ));
                };
                let ring = close_line_ring(&lines)?;
                return Ok(SweepInput {
                    profile: ring,
                    path,
                });
            }
        };
        let path = match open_polylines.pop() {
            Some(path) => {
                if !lines.is_empty() {
                    return Err(invalid(
                        "followme.ambiguous",
                        "the selection mixes an open path polyline with LINE rows, so the path \
                         cannot be told apart; select one path",
                    ));
                }
                path
            }
            None => open_line_chain(&lines)?,
        };
        Ok(SweepInput { profile, path })
    }
}

/// How many segments one CIRCLE profile is sampled into (a SketchUp circle is
/// itself a polygon; 48 is finer than its default 24).
const CIRCLE_SEGMENTS: usize = 48;

/// One CIRCLE as a polygonal ring in its own plane, wound counter-clockwise about
/// `axis`.
fn sample_circle(center: Point3, axis: Point3, radius: f64) -> Vec<Point3> {
    let (u, v) = plane_basis(axis);
    (0..CIRCLE_SEGMENTS)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / CIRCLE_SEGMENTS as f64;
            add(
                center,
                add(mul(u, radius * angle.cos()), mul(v, radius * angle.sin())),
            )
        })
        .collect()
}

/// Order LINE rows into one closed ring.
fn close_line_ring(lines: &[(Point3, Point3)]) -> CadResult<Vec<Point3>> {
    let ordered = order_chain(lines).ok_or_else(|| {
        invalid(
            "followme.profile_degenerate",
            "the selected LINE rows do not form one closed ring",
        )
    })?;
    if !same_point(ordered[0], ordered[ordered.len() - 1]) {
        return Err(invalid(
            "followme.profile_degenerate",
            "the selected LINE rows form an open chain, not a closed profile",
        ));
    }
    Ok(ordered[..ordered.len() - 1].to_vec())
}

/// Order LINE rows into one open path.
fn open_line_chain(lines: &[(Point3, Point3)]) -> CadResult<Vec<Point3>> {
    let ordered = order_chain(lines).ok_or_else(|| {
        invalid(
            "followme.path_missing",
            "FOLLOWME needs one open path (an open straight LWPOLYLINE or an open LINE chain)",
        )
    })?;
    if same_point(ordered[0], ordered[ordered.len() - 1]) {
        return Err(invalid(
            "followme.path_closed",
            "the selected LINE rows form a closed loop; this cut follows open paths only",
        ));
    }
    Ok(ordered)
}

/// Walk a set of segments into one chain, reversing segments as needed.
///
/// Returns `None` for a branching set, a disjoint set, or duplicate segments: the
/// caller names the role-specific refusal.
fn order_chain(lines: &[(Point3, Point3)]) -> Option<Vec<Point3>> {
    if lines.is_empty() {
        return None;
    }
    let mut remaining: Vec<(Point3, Point3)> = lines.to_vec();
    let (first, second) = remaining.remove(0);
    let mut chain = vec![first, second];
    // Each step appends the far end of the one segment that touches the chain's
    // current tail; a segment touching it twice (a zero-length row) or a branch
    // makes the walk fail instead of guessing.
    while !remaining.is_empty() {
        let tail = *chain.last()?;
        let index = remaining
            .iter()
            .position(|(a, b)| same_point(*a, tail) || same_point(*b, tail))?;
        let (a, b) = remaining.remove(index);
        if same_point(a, tail) {
            chain.push(b);
        } else {
            chain.push(a);
        }
    }
    Some(chain)
}

/// The closed, outward-oriented triangle mesh of one sweep.
fn sweep_mesh(profile: &[Point3], path: &[Point3]) -> CadResult<Mesh> {
    // A self-crossing loop has a cancelling Newell normal, so the crossing check
    // runs first on the loop's dominant-axis projection.
    check_simple(profile)?;
    let (normal, origin) = profile_plane(profile)?;
    check_planar(profile, normal, origin)?;
    let (u, v) = plane_basis(normal);
    check_path(path)?;

    let local: Vec<[f64; 2]> = profile
        .iter()
        .map(|point| {
            let d = sub(*point, origin);
            [dot(d, u), dot(d, v)]
        })
        .collect();

    let tangents = path_tangents(path)?;
    // Orientation of the profile at each segment, transported along the path so a
    // 3D bend never flips the section.
    let mut orientations: Vec<[[f64; 3]; 3]> = Vec::with_capacity(tangents.len());
    let mut rotation = align(normal, tangents[0]);
    orientations.push(rotation);
    for pair in tangents.windows(2) {
        rotation = mul3(&align(pair[0], pair[1]), &rotation);
        orientations.push(rotation);
    }

    // Ring 0 is the profile at the path start; every interior vertex gets the
    // mitre ring (the incoming ring projected into the bisecting plane); the last
    // ring is the profile at the path end.
    let mut rings: Vec<Vec<Point3>> = Vec::with_capacity(path.len());
    rings.push(place_ring(
        path[0],
        &rotation_of(&orientations, 0),
        u,
        v,
        &local,
    ));
    for index in 1..path.len() - 1 {
        let incoming = &tangents[index - 1];
        let outgoing = &tangents[index];
        let bisector = normalize(add(*incoming, *outgoing)).ok_or_else(|| {
            invalid(
                "followme.path_reversal",
                format!("path vertex {index} reverses the path direction"),
            )
        })?;
        let incoming_ring = place_ring(
            path[index],
            &rotation_of(&orientations, index - 1),
            u,
            v,
            &local,
        );
        let projected = incoming_ring
            .into_iter()
            .map(|point| project_along(point, path[index], *incoming, bisector))
            .collect();
        rings.push(projected);
    }
    rings.push(place_ring(
        path[path.len() - 1],
        &rotation_of(&orientations, path.len() - 2),
        u,
        v,
        &local,
    ));

    let mut builder = MeshBuilder::default();
    let count = profile.len();
    for (segment, pair) in rings.windows(2).enumerate() {
        for edge in 0..count {
            let next = (edge + 1) % count;
            let source = format!("{WALL_PREFIX}.{segment}.{edge}");
            builder.quad(
                [pair[0][edge], pair[0][next], pair[1][next], pair[1][edge]],
                &source,
            );
        }
    }
    // Caps: the ring loop in its own plane, triangulated by ear clipping (the
    // profile may be concave).
    // The start cap faces back along the first segment, the end cap forward
    // along the last one; both are wound to match the walls so the shell stays
    // consistently oriented (the global sign fix below then only decides which
    // side is out, never how the faces relate to each other).
    let start_source = CAP_START.to_string();
    builder.cap(&rings[0], mul(tangents[0], -1.0), &start_source)?;
    let end_source = CAP_END.to_string();
    builder.cap(
        &rings[rings.len() - 1],
        tangents[tangents.len() - 1],
        &end_source,
    )?;
    let mesh = builder.finish();
    if signed_volume(&mesh) < 0.0 {
        return Ok(flipped(mesh));
    }
    Ok(mesh)
}

fn rotation_of(orientations: &[[[f64; 3]; 3]], index: usize) -> [[f64; 3]; 3] {
    orientations[index.min(orientations.len() - 1)]
}

/// Place the profile's local `(u, v)` coordinates at `origin` under `rotation`.
fn place_ring(
    origin: Point3,
    rotation: &[[f64; 3]; 3],
    u: Point3,
    v: Point3,
    local: &[[f64; 2]],
) -> Vec<Point3> {
    let ru = apply3(rotation, u);
    let rv = apply3(rotation, v);
    local
        .iter()
        .map(|[s, t]| add(origin, add(mul(ru, *s), mul(rv, *t))))
        .collect()
}

/// Project `point` along `direction` onto the plane through `origin` with
/// `normal` (the mitre section of a bend).
fn project_along(point: Point3, origin: Point3, direction: Point3, normal: Point3) -> Point3 {
    let denominator = dot(direction, normal);
    if denominator.abs() <= 1e-12 {
        return point;
    }
    let distance = dot(sub(point, origin), normal) / denominator;
    sub(point, mul(direction, distance))
}

/// The unit profile normal (Newell's method) and a point on its plane.
fn profile_plane(profile: &[Point3]) -> CadResult<(Point3, Point3)> {
    if profile.len() < 3 {
        return Err(invalid(
            "followme.profile_degenerate",
            "a profile needs at least three vertices",
        ));
    }
    let mut normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let mut centroid = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for index in 0..profile.len() {
        let current = profile[index];
        let next = profile[(index + 1) % profile.len()];
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
        centroid = add(centroid, current);
    }
    let normal = normalize(normal).ok_or_else(|| {
        invalid(
            "followme.profile_degenerate",
            "the profile has zero area, so it has no plane",
        )
    })?;
    Ok((normal, mul(centroid, 1.0 / profile.len() as f64)))
}

/// Every profile point on the profile plane, within [`PLANAR_TOLERANCE`].
fn check_planar(profile: &[Point3], normal: Point3, origin: Point3) -> CadResult<()> {
    let extent = profile
        .iter()
        .map(|point| length(sub(*point, origin)))
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let tolerance = PLANAR_TOLERANCE * extent;
    for (index, point) in profile.iter().enumerate() {
        if dot(sub(*point, origin), normal).abs() > tolerance {
            return Err(invalid(
                "followme.profile_not_planar",
                format!("profile vertex {index} leaves the profile plane"),
            ));
        }
    }
    Ok(())
}

/// No two non-adjacent profile edges cross, and no vertex repeats.
fn check_simple(profile: &[Point3]) -> CadResult<()> {
    let count = profile.len();
    for index in 0..count {
        for other in index + 1..count {
            if same_point(profile[index], profile[other]) {
                return Err(invalid(
                    "followme.profile_degenerate",
                    format!("profile vertices {index} and {other} coincide"),
                ));
            }
        }
    }
    for first in 0..count {
        let a = profile[first];
        let b = profile[(first + 1) % count];
        for second in first + 1..count {
            // Adjacent edges share a vertex by construction.
            if second == first + 1 || (first == 0 && second == count - 1) {
                continue;
            }
            let c = profile[second];
            let d = profile[(second + 1) % count];
            if segments_cross((a, b), (c, d), flat_axis(profile)) {
                return Err(invalid(
                    "followme.profile_self_intersecting",
                    format!("profile edges {first} and {second} cross"),
                ));
            }
        }
    }
    Ok(())
}

/// The path is open, has at least two distinct points, and no zero-length step.
fn check_path(path: &[Point3]) -> CadResult<()> {
    if path.len() < 2 {
        return Err(invalid(
            "followme.path_degenerate",
            "a path needs at least two vertices",
        ));
    }
    if same_point(path[0], path[path.len() - 1]) {
        return Err(invalid(
            "followme.path_closed",
            "this cut follows open paths only; a closed path would need a joined sweep",
        ));
    }
    for index in 0..path.len() - 1 {
        if same_point(path[index], path[index + 1]) {
            return Err(invalid(
                "followme.path_degenerate",
                format!("path vertices {index} and {} coincide", index + 1),
            ));
        }
    }
    Ok(())
}

/// The unit tangent of every path segment.
fn path_tangents(path: &[Point3]) -> CadResult<Vec<Point3>> {
    let mut tangents = Vec::with_capacity(path.len() - 1);
    for index in 0..path.len() - 1 {
        let tangent = normalize(sub(path[index + 1], path[index])).ok_or_else(|| {
            invalid(
                "followme.path_degenerate",
                format!("path segment {index} has zero length"),
            )
        })?;
        tangents.push(tangent);
    }
    Ok(tangents)
}

/// A right-handed orthonormal basis with `normal` as its third axis.
fn plane_basis(normal: Point3) -> (Point3, Point3) {
    let reference = if normal.z.abs() < 0.9 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    };
    let u = normalize(cross(reference, normal)).unwrap_or(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    let v = cross(normal, u);
    (u, v)
}

/// The minimal rotation taking `from` to `to`; the antiparallel case turns a
/// half turn about a stable perpendicular.
fn align(from: Point3, to: Point3) -> [[f64; 3]; 3] {
    let axis = cross(from, to);
    let sine = length(axis);
    let cosine = dot(from, to).clamp(-1.0, 1.0);
    if sine <= 1e-12 {
        if cosine >= 0.0 {
            return identity3();
        }
        let (u, _) = plane_basis(from);
        return rodrigues(u, std::f64::consts::PI);
    }
    rodrigues(mul(axis, 1.0 / sine), sine.atan2(cosine))
}

/// Rodrigues' rotation matrix about a unit axis.
fn rodrigues(axis: Point3, angle: f64) -> [[f64; 3]; 3] {
    let (s, c) = angle.sin_cos();
    let skew = [
        [0.0, -axis.z, axis.y],
        [axis.z, 0.0, -axis.x],
        [-axis.y, axis.x, 0.0],
    ];
    let mut out = identity3();
    for (row, line) in out.iter_mut().enumerate() {
        for (column, cell) in line.iter_mut().enumerate() {
            let squared: f64 = (0..3)
                .map(|index| skew[row][index] * skew[index][column])
                .sum();
            *cell = *cell + s * skew[row][column] + (1.0 - c) * squared;
        }
    }
    out
}

fn identity3() -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn mul3(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (row, line) in out.iter_mut().enumerate() {
        for (column, cell) in line.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    out
}

fn apply3(m: &[[f64; 3]; 3], v: Point3) -> Point3 {
    Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    }
}

/// Accumulates one mesh, duplicating each face's vertices so flat normals are
/// exact and every wall quad carries one face identity.
#[derive(Default)]
struct MeshBuilder {
    vertices: Vec<Point3>,
    normals: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
    sources: Vec<Option<SubElementId>>,
}

impl MeshBuilder {
    fn vertex(&mut self, point: Point3, normal: Point3) -> u32 {
        self.vertices.push(point);
        self.normals.push(normal);
        (self.vertices.len() - 1) as u32
    }

    /// One planar quad as two triangles, wound `a → b → c → d`.
    fn quad(&mut self, corners: [Point3; 4], source: &str) {
        let normal = face_normal(corners[0], corners[1], corners[2])
            .or_else(|| face_normal(corners[0], corners[2], corners[3]));
        let Some(normal) = normal else { return };
        let indices = [
            self.vertex(corners[0], normal),
            self.vertex(corners[1], normal),
            self.vertex(corners[2], normal),
            self.vertex(corners[3], normal),
        ];
        let sub = SubElementId {
            source_key: source.to_string(),
            topology_revision: Revision(0),
        };
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            self.triangles.push([
                indices[triangle[0]],
                indices[triangle[1]],
                indices[triangle[2]],
            ]);
            self.sources.push(Some(sub.clone()));
        }
    }

    /// Triangulate one planar ring as a cap.
    fn cap(&mut self, ring: &[Point3], outward: Point3, source: &str) -> CadResult<()> {
        let (normal, origin) = profile_plane(ring)?;
        let (u, v) = plane_basis(normal);
        let flat: Vec<[f64; 2]> = ring
            .iter()
            .map(|point| {
                let d = sub(*point, origin);
                [dot(d, u), dot(d, v)]
            })
            .collect();
        let triangles = ear_clip(&flat).ok_or_else(|| {
            invalid(
                "followme.mesh_invalid",
                "a cap ring could not be triangulated",
            )
        })?;
        let sub = SubElementId {
            source_key: source.to_string(),
            topology_revision: Revision(0),
        };
        // The ear clip is counter-clockwise about the ring's Newell normal; flip
        // the winding when that normal faces into the tube.
        let reverse = dot(normal, outward) < 0.0;
        for [a, b, c] in triangles {
            let (a, b, c) = if reverse { (a, c, b) } else { (a, b, c) };
            let normal = face_normal(ring[a], ring[b], ring[c]).unwrap_or(normal);
            let indices = [
                self.vertex(ring[a], normal),
                self.vertex(ring[b], normal),
                self.vertex(ring[c], normal),
            ];
            self.triangles.push(indices);
            self.sources.push(Some(sub.clone()));
        }
        Ok(())
    }

    fn finish(self) -> Mesh {
        Mesh {
            vertices: self.vertices,
            triangles: self.triangles,
            normals: self.normals,
            face_sources: self.sources,
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
        }
    }
}

/// Ear-clipping triangulation of a simple polygon, preserving its winding.
fn ear_clip(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
    if points.len() < 3 {
        return None;
    }
    let mut order: Vec<usize> = (0..points.len()).collect();
    if signed_area_2d(points) < 0.0 {
        order.reverse();
    }
    let mut triangles = Vec::with_capacity(points.len() - 2);
    let mut guard = 0;
    while order.len() > 3 {
        let count = order.len();
        let mut clipped = false;
        for position in 0..count {
            let previous = order[(position + count - 1) % count];
            let current = order[position];
            let next = order[(position + 1) % count];
            let (a, b, c) = (points[previous], points[current], points[next]);
            if cross_2d(sub2(b, a), sub2(c, b)) <= 1e-18 {
                continue;
            }
            let inside = order.iter().any(|candidate| {
                *candidate != previous
                    && *candidate != current
                    && *candidate != next
                    && point_in_triangle(points[*candidate], a, b, c)
            });
            if inside {
                continue;
            }
            triangles.push([previous, current, next]);
            order.remove(position);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
        guard += 1;
        if guard > points.len() * points.len() {
            return None;
        }
    }
    triangles.push([order[0], order[1], order[2]]);
    Some(triangles)
}

fn signed_area_2d(points: &[[f64; 2]]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area * 0.5
}

fn cross_2d(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn sub2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let d1 = cross_2d(sub2(b, a), sub2(p, a));
    let d2 = cross_2d(sub2(c, b), sub2(p, b));
    let d3 = cross_2d(sub2(a, c), sub2(p, c));
    let negative = d1 < -1e-18 || d2 < -1e-18 || d3 < -1e-18;
    let positive = d1 > 1e-18 || d2 > 1e-18 || d3 > 1e-18;
    !(negative && positive)
}

/// The two axes a loop is projected on (the widest extent pair).
fn flat_axis(profile: &[Point3]) -> (usize, usize) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in profile {
        let coords = [point.x, point.y, point.z];
        for axis in 0..3 {
            min[axis] = min[axis].min(coords[axis]);
            max[axis] = max[axis].max(coords[axis]);
        }
    }
    let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let drop = if extent[0] <= extent[1] && extent[0] <= extent[2] {
        0
    } else if extent[1] <= extent[2] {
        1
    } else {
        2
    };
    let keep: Vec<usize> = (0..3).filter(|axis| *axis != drop).collect();
    (keep[0], keep[1])
}

/// Proper intersection of two profile segments, projected on the two axes that
/// span the loop most (a planar loop projects injectively there).
fn segments_cross(
    (a, b): (Point3, Point3),
    (c, d): (Point3, Point3),
    (first, second): (usize, usize),
) -> bool {
    let flat = |p: Point3| {
        let coords = [p.x, p.y, p.z];
        [coords[first], coords[second]]
    };
    let (a2, b2, c2, d2) = (flat(a), flat(b), flat(c), flat(d));
    let d1 = cross_2d(sub2(b2, a2), sub2(c2, a2));
    let d2v = cross_2d(sub2(b2, a2), sub2(d2, a2));
    let d3 = cross_2d(sub2(d2, c2), sub2(a2, c2));
    let d4 = cross_2d(sub2(d2, c2), sub2(b2, c2));
    ((d1 > 0.0) != (d2v > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// The signed volume of a closed triangle mesh (divergence theorem).
fn signed_volume(mesh: &Mesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|triangle| {
            let a = mesh.vertices[triangle[0] as usize];
            let b = mesh.vertices[triangle[1] as usize];
            let c = mesh.vertices[triangle[2] as usize];
            dot(a, cross(b, c)) / 6.0
        })
        .sum()
}

/// Reverse every triangle, keeping vertices/normals/sources consistent.
fn flipped(mut mesh: Mesh) -> Mesh {
    for triangle in &mut mesh.triangles {
        triangle.swap(1, 2);
    }
    for normal in &mut mesh.normals {
        *normal = mul(*normal, -1.0);
    }
    mesh
}

/// What a diagnostic states about the result.
struct MeshSummary {
    triangles: usize,
}

impl MeshSummary {
    fn of(mesh: &Mesh) -> Self {
        MeshSummary {
            triangles: mesh.triangles.len(),
        }
    }
}

fn face_normal(a: Point3, b: Point3, c: Point3) -> Option<Point3> {
    normalize(cross(sub(b, a), sub(c, a)))
}

fn same_point(a: Point3, b: Point3) -> bool {
    let scale = 1.0_f64.max(length(a)).max(length(b));
    length(sub(a, b)) <= 1e-9 * scale
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn mul(v: Point3, scale: f64) -> Point3 {
    Point3 {
        x: v.x * scale,
        y: v.y * scale,
        z: v.z * scale,
    }
}

fn length(v: Point3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

fn normalize(v: Point3) -> Option<Point3> {
    let length = length(v);
    (length.is_finite() && length > 1e-12).then(|| mul(v, 1.0 / length))
}
