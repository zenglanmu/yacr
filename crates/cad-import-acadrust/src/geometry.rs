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

/// Convert a paper-space VIEWPORT entity into a database [`PaperViewport`].
///
/// Returns `None` for the sheet viewport (`id == 1`), which frames the paper
/// sheet itself rather than a model-space window, and for a viewport switched
/// off: neither has drawable content.
///
/// The clip is written as four axis-aligned paper corners
/// (`center ± (width/2, height/2)`), and `model_to_paper` is the real
/// paper→model map
/// `model = (paper - paper_center) * model_per_paper + view_target`,
/// so the representation layer recovers the view centre from the stored
/// transform (audit B22).
///
/// Exact only for a top/plan view: a tilted view direction, a view twist, a
/// perspective projection or a non-rectangular clip is reported `Partial` with
/// the specific reason. The representation layer then refuses it with a stable
/// code instead of drawing model space wrong.
pub(crate) fn paper_viewport(v: &acadrust::entities::Viewport) -> Option<PaperViewport> {
    if v.id == 1 || !v.status.is_on {
        return None;
    }
    let mut reasons: Vec<String> = Vec::new();
    // A non-rectangular (object) clip cannot be represented by four corners.
    if !v.clip_boundary_handle.null_or_value_zero() {
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
    let view_target = p3(v.view_target);
    let finite = [paper_center, view_target]
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        && v.width.is_finite()
        && v.height.is_finite();
    if !finite {
        reasons.push("viewport geometry is non-finite".into());
    }
    let half_w = v.width / 2.0;
    let half_h = v.height / 2.0;
    let clip = vec![
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
    ];
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = model_per_paper;
    matrix[1][1] = model_per_paper;
    matrix[0][3] = view_target.x - model_per_paper * paper_center.x;
    matrix[1][3] = view_target.y - model_per_paper * paper_center.y;
    matrix[2][3] = view_target.z;
    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    Some(PaperViewport {
        clip,
        model_to_paper: Transform3 { matrix },
        completeness,
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
    if is_world_z(normal) {
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
    let normal = if is_world_z(normal) || cad_geometry::length(normal) < 1e-24 {
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
    });
    Mesh {
        vertices: std::mem::take(&mut vertices),
        triangles,
        normals,
        face_sources: Vec::new(),
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
