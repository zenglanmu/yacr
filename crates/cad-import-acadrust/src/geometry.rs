//! Entity geometry helpers (OCS, splines, paper viewports, transforms).

use super::*;

// ---- small helpers ----

pub(crate) fn p3(v: acadrust::Vector3) -> Point3 {
    Point3 {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

/// The world-Z direction, for the common `(0, 0, 1)` extrusion.
#[cfg(test)]
pub(crate) fn world_z() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

/// The axis-aligned paper rectangle enclosed by a clip-boundary LWPOLYLINE.
///
/// VPCLIP stores its boundary as a closed polyline, so a viewport whose clip
/// handle is set is not necessarily non-rectangular. Only a closed, bulge-free,
/// world-Z polyline whose unique vertices are exactly the four corners of an
/// axis-aligned rectangle is provably rectangular; a curve, a rotated quad or
/// extra corners returns `None` rather than being squared off.
///
/// Consecutive duplicate vertices are collapsed and a closing duplicate is
/// dropped (real DWG files store both). The returned corners use the boundary's
/// min/max extents on the paper plane, in counter-clockwise order.
fn rectangular_clip_rectangle(p: &acadrust::entities::LwPolyline) -> Option<[Point3; 4]> {
    if !p.is_closed || p.vertices.len() < 3 {
        return None;
    }
    // Any bulge is an arc segment: the boundary is not a straight-edged
    // rectangle, even if its vertices happen to line up.
    if p.vertices.iter().any(|v| v.bulge.abs() > 1e-12) {
        return None;
    }
    let normal = polyline_normal(p3(p.normal));
    if !is_world_z(normal) {
        // A tilted OCS projects onto paper as some other shape; do not guess.
        return None;
    }
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(p.vertices.len());
    for v in &p.vertices {
        let w = ocs_to_wcs(normal, p.elevation, v.location.x, v.location.y);
        points.push((w.x, w.y));
    }
    if !points.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        return None;
    }
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let scale = (max_x - min_x).max(max_y - min_y);
    // Relative tolerance: a real paper rectangle is many units across, so
    // 1e-9 of its extent separates true corners from duplicate vertices
    // without accepting a visibly non-rectangular outline.
    let tol = 1e-9 * scale;
    let mut unique: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for (x, y) in points {
        if unique
            .last()
            .is_some_and(|(qx, qy)| (*qx - x).abs() <= tol && (*qy - y).abs() <= tol)
        {
            continue;
        }
        unique.push((x, y));
    }
    if unique.len() > 1 {
        let (first_x, first_y) = unique[0];
        let (last_x, last_y) = *unique.last().expect("non-empty");
        if (first_x - last_x).abs() <= tol && (first_y - last_y).abs() <= tol {
            unique.pop();
        }
    }
    if unique.len() != 4 || max_x - min_x <= tol || max_y - min_y <= tol {
        return None;
    }
    for (x, y) in &unique {
        let on_x = (*x - min_x).abs() <= tol || (*x - max_x).abs() <= tol;
        let on_y = (*y - min_y).abs() <= tol || (*y - max_y).abs() <= tol;
        if !on_x || !on_y {
            return None;
        }
    }
    Some([
        Point3 {
            x: min_x,
            y: min_y,
            z: 0.0,
        },
        Point3 {
            x: max_x,
            y: min_y,
            z: 0.0,
        },
        Point3 {
            x: max_x,
            y: max_y,
            z: 0.0,
        },
        Point3 {
            x: min_x,
            y: max_y,
            z: 0.0,
        },
    ])
}

/// Convert a paper-space VIEWPORT entity into a database [`PaperViewport`].
///
/// Returns `None` for the sheet viewport (`id == 1`), which frames the paper
/// sheet itself rather than a model-space window, and for a viewport switched
/// off: neither has drawable content.
///
/// The clip is written as four axis-aligned paper corners. Without a clip
/// boundary handle they are `center ± (width/2, height/2)`; with one, the
/// boundary entity is resolved and used when it is provably a closed,
/// bulge-free, axis-aligned rectangle (VPCLIP's own geometry). `model_to_paper`
/// is the real paper→model map
/// `model = (paper - paper_center) * model_per_paper + anchor`,
/// where `anchor` is the model point shown at the viewport centre, so the
/// representation layer recovers the view centre from the stored transform
/// (audit B22) even when the clip rectangle differs from the viewport
/// rectangle.
///
/// The anchor comes from the DXF *view centre* (group 12/22, DCS), not the
/// 3D `view_target` (group 17/27/37, WCS): QCAD and other writers put the real
/// 2D centre in 12/22 and leave 17/27/37 at `(0, 0, 0)`, so anchoring on
/// `view_target` degenerates every such viewport to "model origin at the
/// viewport centre". Only the anchor's z stays on `view_target.z`. Files that
/// populate just group 17 are kept working by a compatibility fallback: when
/// the view centre is exactly `(0, 0)` and the target's x/y is not, the
/// target's x/y become the anchor.
///
/// Exact only for a top/plan view: a tilted view direction, a view twist, a
/// perspective projection or a clip that cannot be proven rectangular is
/// reported `Partial` with the specific reason. The representation layer then
/// refuses it with a stable code instead of drawing model space wrong.
pub(crate) fn paper_viewport(
    v: &acadrust::entities::Viewport,
    acad: &acadrust::CadDocument,
) -> Option<PaperViewport> {
    if v.id == 1 || !v.status.is_on {
        return None;
    }
    let mut reasons: Vec<String> = Vec::new();
    // VPCLIP stores its boundary as a separate entity. Resolve it and accept
    // only a shape that can be represented by four corners; anything else (a
    // curve, a rotated quad, extra corners, a dangling handle) stays Partial.
    let has_clip_boundary = !v.clip_boundary_handle.null_or_value_zero();
    let boundary_clip = if has_clip_boundary {
        acad.get_entity(v.clip_boundary_handle)
            .and_then(|entity| match entity {
                EntityType::LwPolyline(polyline) => rectangular_clip_rectangle(polyline),
                _ => None,
            })
    } else {
        None
    };
    if has_clip_boundary && boundary_clip.is_none() {
        reasons.push("non-rectangular viewport clip is not applied".into());
    }
    if !is_world_z(p3(v.view_direction)) {
        reasons.push("viewport view direction is not perpendicular to the paper plane".into());
    }
    if v.twist_angle.abs() > 1e-12 {
        reasons.push("viewport has a view twist".into());
    }
    if v.status.perspective {
        reasons.push("perspective viewport is not supported".into());
    }
    let view_height = v.view_height;
    let model_per_paper = if view_height.is_finite() && view_height.abs() > 1e-12 {
        view_height / v.height
    } else {
        reasons.push("viewport view_height is missing, zero or non-finite".into());
        0.0
    };
    let paper_center = p3(v.center);
    let view_center = p3(v.view_center);
    let view_target = p3(v.view_target);
    // DXF group 12/22 carries the 2D view centre (DCS); group 17/27/37 the 3D
    // WCS target. QCAD leaves the target at the origin, so anchor on the view
    // centre and only fall back to the target for files that populate just 17.
    let anchor = if view_center.x == 0.0
        && view_center.y == 0.0
        && (view_target.x != 0.0 || view_target.y != 0.0)
    {
        Point3 {
            x: view_target.x,
            y: view_target.y,
            z: view_target.z,
        }
    } else {
        Point3 {
            x: view_center.x,
            y: view_center.y,
            z: view_target.z,
        }
    };
    let finite = [paper_center, view_center, view_target]
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        && v.width.is_finite()
        && v.height.is_finite();
    if !finite {
        reasons.push("viewport geometry is non-finite".into());
    }
    let half_w = v.width / 2.0;
    let half_h = v.height / 2.0;
    let clip = match boundary_clip {
        Some(corners) => corners.to_vec(),
        None => vec![
            Point3 {
                x: paper_center.x - half_w,
                y: paper_center.y - half_h,
                z: 0.0,
            },
            Point3 {
                x: paper_center.x + half_w,
                y: paper_center.y - half_h,
                z: 0.0,
            },
            Point3 {
                x: paper_center.x + half_w,
                y: paper_center.y + half_h,
                z: 0.0,
            },
            Point3 {
                x: paper_center.x - half_w,
                y: paper_center.y + half_h,
                z: 0.0,
            },
        ],
    };
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = model_per_paper;
    matrix[1][1] = model_per_paper;
    matrix[0][3] = anchor.x - model_per_paper * paper_center.x;
    matrix[1][3] = anchor.y - model_per_paper * paper_center.y;
    matrix[2][3] = anchor.z;
    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    Some(PaperViewport {
        clip,
        model_to_paper: Transform3 { matrix },
        completeness,
        // VPLAYER parsing is NOT IMPLEMENTED: an imported viewport starts with
        // no per-viewport layer overrides, so every layer keeps its global
        // state. A source with VPLAYER data stays `Partial` through
        // `completeness`; this build never invents overrides it did not read.
        layer_overrides: std::collections::BTreeMap::new(),
    })
}

/// True when an extrusion/direction is parallel to the world Z axis.
///
/// A zero vector is treated as world Z: acadrust defaults an absent normal to
/// `UNIT_Z`, and a degenerate normal has no other meaningful plane.
pub(crate) fn is_world_z(normal: Point3) -> bool {
    let n = cad_geometry::normalize(normal);
    n.x.abs() <= 1e-9 && n.y.abs() <= 1e-9
}

/// True when the extrusion is the world Z axis pointing *down*.
///
/// The OCS that `(0, 0, -1)` defines is the x-mirror of world XY: the
/// arbitrary-axis frame is `ax = -X`, `ay = +Y`, `az = -Z`, so an OCS point
/// `(x, y)` at elevation `e` lands at `(-x, y, -e)`. The identity fast path is
/// therefore only valid for `+Z`.
pub(crate) fn is_negative_world_z(normal: Point3) -> bool {
    is_world_z(normal) && cad_geometry::normalize(normal).z < 0.0
}

/// Map a 2D OCS (object coordinate system) point to WCS.
///
/// A 2D polyline's vertices live in the plane defined by its extrusion
/// (`normal`); `elevation` offsets the plane along that normal. The AutoCAD
/// arbitrary-axis algorithm supplies the in-plane axes. The previous importer
/// ignored the extrusion and used `(x, y, elevation)` for every polyline,
/// treating a tilted OCS entity as if it were flat on the WCS XY plane.
pub(crate) fn ocs_to_wcs(normal: Point3, elevation: f64, x: f64, y: f64) -> Point3 {
    let (ax, ay, az) = arbitrary_axis(normal);
    cad_geometry::add(
        cad_geometry::scale(az, elevation),
        cad_geometry::add(cad_geometry::scale(ax, x), cad_geometry::scale(ay, y)),
    )
}

/// Normalise a 2D polyline's OCS vertices to WCS.
pub(crate) fn polyline_ocs_points(
    normal: Point3,
    elevation: f64,
    vertices: impl IntoIterator<Item = (f64, f64)>,
) -> Vec<Point3> {
    if is_world_z(normal) && !is_negative_world_z(normal) {
        // Fast/identity path: the common drawing keeps `z = elevation` exactly.
        return vertices
            .into_iter()
            .map(|(x, y)| Point3 { x, y, z: elevation })
            .collect();
    }
    vertices
        .into_iter()
        .map(|(x, y)| ocs_to_wcs(normal, elevation, x, y))
        .collect()
}

/// Bulges matching the vertices returned by [`polyline_ocs_points`].
///
/// A `-Z` extrusion mirrors the OCS plane, and a mirror reverses the direction
/// of every arc: with the vertices already mirrored, each bulge's sign must
/// flip as well, or the arc bows to the side the unmirrored file used.
pub(crate) fn polyline_ocs_bulges(normal: Point3, bulges: &[f64]) -> Vec<f64> {
    if is_negative_world_z(normal) {
        bulges.iter().map(|b| -b).collect()
    } else {
        bulges.to_vec()
    }
}

/// Map a TEXT-family insertion point and rotation from its OCS to world XY.
///
/// A single-line TEXT stores its alignment point in OCS (DXF group 10) and its
/// rotation in the OCS plane; `+Z` is already world XY. A `-Z` extrusion
/// mirrors that plane, so the world point mirrors `x`/`z` and the baseline
/// angle becomes `PI - rotation` (the mirror of a direction).
///
/// The planar text domain stores no extrusion normal, so a tilted (non-Z)
/// extrusion is left untouched here: representing it would need a domain
/// change, not an import-only mapping.
pub(crate) fn text_ocs_placement(normal: Point3, position: Point3, rotation: f64) -> (Point3, f64) {
    if is_negative_world_z(normal) {
        (
            ocs_to_wcs(normal, position.z, position.x, position.y),
            std::f64::consts::PI - rotation,
        )
    } else {
        (position, rotation)
    }
}

/// Completeness of a 2D polyline whose vertices were mapped to WCS.
///
/// A tilted extrusion is exact for straight segments (the vertices carry the
/// plane). A bulge arc is exact too once the polyline has at least three
/// points to fix its plane; a two-point tilted bulge has no unique plane and is
/// reported Partial rather than silently drawn flat.
pub(crate) fn polyline_completeness(
    normal: Point3,
    point_count: usize,
    bulges: &[f64],
) -> Completeness {
    let has_bulge = bulges.iter().any(|b| b.abs() > 1e-12);
    if !is_world_z(normal) && has_bulge && point_count < 3 {
        Completeness::Partial(vec![
            "tilted extrusion with a two-vertex bulge: the arc plane is ambiguous".into(),
        ])
    } else {
        Completeness::Complete
    }
}

/// Convert an ELLIPSE, preserving its extrusion normal.
///
/// An ELLIPSE stores its centre and major axis in world coordinates, but the
/// minor axis direction is defined by the extrusion normal
/// (`minor = cross(normal, major)`). Carrying the normal lets an ellipse on an
/// arbitrary OCS plane stay in that plane instead of being folded onto world XY
/// (audit B23/B31).
pub(crate) fn ellipse_semantics(
    e: &acadrust::entities::Ellipse,
) -> (SemanticGeometry, Completeness) {
    let normal = p3(e.normal);
    // Preserve a `-Z` extrusion: it is the x-mirror of `+Z`, and coercing it
    // to `+Z` would flip the minor axis (and the parameter sweep) instead.
    // Only a degenerate (absent) normal defaults to world Z.
    let normal = if cad_geometry::length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        cad_geometry::normalize(normal)
    };
    let geometry = SemanticGeometry::Ellipse {
        center: p3(e.center),
        normal,
        major_axis: p3(e.major_axis),
        ratio: e.minor_axis_ratio,
        start: e.start_parameter,
        sweep: normalize_sweep(e.end_parameter - e.start_parameter),
    };
    let complete = is_finite_point(e.center)
        && is_finite_point(e.major_axis)
        && cad_geometry::length(p3(e.major_axis)) >= 1e-12
        && e.minor_axis_ratio.is_finite();
    let completeness = if complete {
        Completeness::Complete
    } else {
        Completeness::Partial(vec!["ellipse has a zero or non-finite axis/ratio".into()])
    };
    (geometry, completeness)
}

pub(crate) fn is_finite_point(v: acadrust::types::Vector3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Convert a SPLINE, preserving the source's own degree, knots and weights.
///
/// A rational spline is no longer downgraded: the geometry engine evaluates the
/// source knot vector and weights exactly (audit B23). The status is `Partial`
/// only when the source record cannot be represented faithfully — missing
/// control points (a fit-point-only spline), or a knot/weight vector that does
/// not match the control polygon length.
pub(crate) fn spline_semantics(s: &acadrust::entities::Spline) -> (SemanticGeometry, Completeness) {
    let degree = s.degree.max(1) as u32;
    let control_points: Vec<Point3> = s.control_points.iter().map(|p| p3(*p)).collect();
    let geometry = SemanticGeometry::Spline {
        degree,
        knots: s.knots.clone(),
        control_points: control_points.clone(),
        weights: s.weights.clone(),
    };
    let n = control_points.len();
    let mut reasons: Vec<String> = Vec::new();
    if n == 0 && !s.fit_points.is_empty() {
        reasons.push("fit-point spline is not interpolated to a NURBS control polygon".into());
    }
    if n > 0 {
        let expected = n + degree as usize + 1;
        if s.knots.len() != expected {
            reasons.push(format!(
                "knot vector has {} entries, expected {expected}",
                s.knots.len()
            ));
        }
        if !s.weights.is_empty() && s.weights.len() != n {
            reasons.push(format!(
                "weight vector has {} entries, expected {n}",
                s.weights.len()
            ));
        }
    }
    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    (geometry, completeness)
}

pub(crate) fn normalize_sweep(sweep: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut s = sweep % tau;
    if s <= 0.0 {
        s += tau;
    }
    s
}

pub(crate) fn quad_mesh(corners: [Point3; 4]) -> Mesh {
    let mut vertices = corners.to_vec();
    // A SOLID/TRACE/FACE with repeated corners is a triangle.
    let degenerate = corners[2] == corners[3];
    let triangles = if degenerate {
        vec![[0, 1, 2]]
    } else {
        vec![[0, 1, 2], [0, 2, 3]]
    };
    let normals = cad_geometry::compute_vertex_normals(&Mesh {
        vertices: vertices.clone(),
        triangles: triangles.clone(),
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
    Mesh {
        vertices: std::mem::take(&mut vertices),
        triangles,
        normals,
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

/// A LWPOLYLINE/POLYLINE's OCS plane normal, defaulting a degenerate or absent
/// normal to world Z.
pub(crate) fn polyline_normal(normal: Point3) -> Point3 {
    if cad_geometry::length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        cad_geometry::normalize(normal)
    }
}

/// Build the block-local shift that moves the block's insertion base point to
/// the local origin, so the INSERT's insertion point lands on it.
pub(crate) fn block_placement(base_point: Point3) -> Transform3 {
    Transform3::translation(Point3 {
        x: -base_point.x,
        y: -base_point.y,
        z: -base_point.z,
    })
}

/// OCS (extrusion-direction) to WCS transform, the AutoCAD arbitrary-axis one.
pub(crate) fn ocs_to_wcs_transform(normal: Point3) -> Transform3 {
    let (ax, ay, az) = arbitrary_axis(polyline_normal(normal));
    Transform3 {
        matrix: [
            [ax.x, ay.x, az.x, 0.0],
            [ax.y, ay.y, az.y, 0.0],
            [ax.z, ay.z, az.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// Build one INSERT array cell's placement matrix, including the block's base
/// point and rotation.
///
/// The block's base point and rotation are folded in first (the same order as
/// the stored `INSERT`): a block-space point *p* lands at
/// `insert(OCS · scale · R · (p - base))`. `offset_x`/`offset_y` are the cell's
/// pre-scale displacement, so the array spacing is not scaled by the INSERT's
/// own scale factors.
pub(crate) fn insert_array_transform(
    i: &acadrust::entities::Insert,
    base_point: Point3,
    offset_x: f64,
    offset_y: f64,
) -> Transform3 {
    let ocs = ocs_to_wcs_transform(p3(i.normal));
    let (s, c) = i.rotation.sin_cos();
    let scale = Transform3 {
        matrix: [
            [i.x_scale(), 0.0, 0.0, offset_x],
            [0.0, i.y_scale(), 0.0, offset_y],
            [0.0, 0.0, i.z_scale(), 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rotate = Transform3 {
        matrix: [
            [c, -s, 0.0, 0.0],
            [s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let translate = Transform3::translation(p3(i.insert_point));
    ocs.matrix_mul(&translate)
        .matrix_mul(&rotate)
        .matrix_mul(&scale)
        .matrix_mul(&block_placement(base_point))
}

/// Every block referenced by expanded geometry (one per array cell), so render
/// status nesting resolves through MINSERTs as well.
pub(crate) fn referenced_blocks(geometry: &SemanticGeometry) -> Vec<BlockId> {
    match geometry {
        SemanticGeometry::Insert { block, .. } => vec![*block],
        SemanticGeometry::Compound(children) => {
            children.iter().flat_map(referenced_blocks).collect()
        }
        // A dimension's display children may reference blocks (for example a
        // picture-block dimension), so register them (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => dimension
            .display()
            .iter()
            .flat_map(referenced_blocks)
            .collect(),
        // A hatch renders its derived display children, exactly like a compound
        // (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            hatch.display().iter().flat_map(referenced_blocks).collect()
        }
        // A multileader is traversed like the compound it used to be
        // (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => multileader
            .display()
            .iter()
            .flat_map(referenced_blocks)
            .collect(),
        // A table's display is grid lines and cell text; it can never reference
        // a block, so this arm is explicitly empty.
        SemanticGeometry::Table(_) => Vec::new(),
        _ => Vec::new(),
    }
}

/// Expand a SOLID/TRACE into a mesh.
///
/// Unlike 3DFACE, a SOLID/TRACE stores its corners out of boundary order: the
/// visible quadrilateral runs first, second, fourth, third. The previous
/// importer emitted the stored order, which crossed the quad. Corners are also
/// lifted from the entity's OCS to WCS, so a tilted SOLID is not flattened onto
/// world XY (audit B31).
///
/// Returns `Partial` for the states this build cannot draw exactly: a
/// non-finite corner, or a thickness extrusion (a prism, not a flat fill) — the
/// flat face is still emitted.
pub(crate) fn solid_mesh_semantics(
    s: &acadrust::entities::Solid,
) -> (SemanticGeometry, Completeness) {
    let normal = polyline_normal(p3(s.normal));
    let finite = [
        s.first_corner,
        s.second_corner,
        s.third_corner,
        s.fourth_corner,
    ]
    .iter()
    .all(|v| is_finite_point(*v));
    if !finite {
        return (
            SemanticGeometry::Opaque {
                type_key: "AcDbTrace".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec!["SOLID/TRACE has a non-finite corner".into()]),
        );
    }
    let (ax, ay, az) = arbitrary_axis(normal);
    let w = |v: acadrust::types::Vector3| {
        cad_geometry::add(
            cad_geometry::add(cad_geometry::scale(ax, v.x), cad_geometry::scale(ay, v.y)),
            cad_geometry::scale(az, v.z),
        )
    };
    let first = w(s.first_corner);
    let second = w(s.second_corner);
    let third = w(s.third_corner);
    let fourth = w(s.fourth_corner);
    // Stored order is first, second, third, fourth; the visible boundary is
    // first, second, fourth, third.
    let boundary = [first, second, fourth, third];
    let mut completeness = Completeness::Complete;
    if s.thickness.abs() > 1e-12 {
        completeness = Completeness::Partial(vec![
            "thickness extrusion is not drawn (flat face only)".into(),
        ]);
    }
    (SemanticGeometry::Mesh(quad_mesh(boundary)), completeness)
}

pub(crate) fn dimension_transform(base: &DimensionBase) -> Transform3 {
    placement_transform(
        base.insertion_point,
        base.insertion_rotation,
        base.insertion_scale,
    )
}

/// Scale, then rotate about Z, then translate. Used by dimension blocks.
pub(crate) fn placement_transform(
    origin: acadrust::types::Vector3,
    rotation: f64,
    scale: acadrust::types::Vector3,
) -> Transform3 {
    let (s, c) = rotation.sin_cos();
    let mut m = [[0.0f64; 4]; 4];
    m[0][0] = c * scale.x;
    m[0][1] = -s * scale.y;
    m[1][0] = s * scale.x;
    m[1][1] = c * scale.y;
    m[2][2] = scale.z;
    m[3][3] = 1.0;
    m[0][3] = origin.x;
    m[1][3] = origin.y;
    m[2][3] = origin.z;
    Transform3 { matrix: m }
}
