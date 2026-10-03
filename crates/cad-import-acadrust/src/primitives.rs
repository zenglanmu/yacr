//! Conversions for the remaining drawable primitive entity kinds.
//!
//! Each function returns neutral [`SemanticGeometry`] plus an honest
//! [`Completeness`]. Kinds with no analytic display path (shapes, mline,
//! multileader, table, tolerance, raster reference) stay out of this module and
//! remain explicit `Partial` at the call site.

use super::*;

/// A polyline through already-world-space points.
pub(crate) fn polyline_semantics(points: Vec<Point3>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges: Vec::new(),
        closed,
    }
}

/// Build a triangle mesh, dropping degenerate/out-of-range faces.
pub(crate) fn triangle_mesh(vertices: Vec<Point3>, faces: &[Vec<usize>]) -> SemanticGeometry {
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for face in faces {
        let mut fan = face
            .iter()
            .copied()
            .filter(|index| *index < vertices.len())
            .collect::<Vec<_>>();
        fan.dedup();
        if fan.len() < 3 {
            continue;
        }
        for i in 1..fan.len() - 1 {
            triangles.push([fan[0] as u32, fan[i] as u32, fan[i + 1] as u32]);
        }
    }
    SemanticGeometry::Mesh(Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    })
}

fn polyface_index(raw: i16, len: usize) -> Option<usize> {
    if raw == 0 {
        return None;
    }
    // Polyface indices are 1-based; a negative value marks an invisible edge.
    let index = (raw.unsigned_abs() as usize).checked_sub(1)?;
    (index < len).then_some(index)
}

pub(crate) fn polyface_mesh_semantics(m: &acadrust::entities::PolyfaceMesh) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(v.location)).collect();
    let len = vertices.len();
    let faces: Vec<Vec<usize>> = m
        .faces
        .iter()
        .map(|face| {
            [face.index1, face.index2, face.index3, face.index4]
                .into_iter()
                .filter_map(|raw| polyface_index(raw, len))
                .collect()
        })
        .collect();
    triangle_mesh(vertices, &faces)
}

pub(crate) fn polygon_mesh_semantics(
    m: &acadrust::entities::PolygonMeshEntity,
) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(v.location)).collect();
    let columns = m.m_vertex_count.max(0) as usize;
    let rows = m.n_vertex_count.max(0) as usize;
    let mut faces: Vec<Vec<usize>> = Vec::new();
    if columns >= 2 && rows >= 2 && vertices.len() >= columns * rows {
        for row in 0..rows - 1 {
            for column in 0..columns - 1 {
                let a = row * columns + column;
                let b = a + 1;
                let c = a + columns + 1;
                let d = a + columns;
                faces.push(vec![a, b, c, d]);
            }
        }
    }
    triangle_mesh(vertices, &faces)
}

pub(crate) fn subd_mesh_semantics(m: &acadrust::entities::Mesh) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(*v)).collect();
    let faces: Vec<Vec<usize>> = m.faces.iter().map(|face| face.vertices.clone()).collect();
    triangle_mesh(vertices, &faces)
}

/// A WIPEOUT's clip boundary is drawn as a closed polyline; the masking effect
/// itself is reported `Partial` because it needs a draw-order/background path.
pub(crate) fn wipeout_semantics(
    w: &acadrust::entities::Wipeout,
) -> (SemanticGeometry, Completeness) {
    let origin = p3(w.insertion_point);
    let u = p3(w.u_vector);
    let v = p3(w.v_vector);
    let mut points: Vec<Point3> = w
        .clip_boundary_vertices
        .iter()
        .map(|p| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(u, p.x), cad_geometry::scale(v, p.y)),
            )
        })
        .collect();
    if points.len() < 3 {
        points = vec![
            origin,
            cad_geometry::add(origin, cad_geometry::scale(u, w.size.x)),
            cad_geometry::add(
                origin,
                cad_geometry::add(
                    cad_geometry::scale(u, w.size.x),
                    cad_geometry::scale(v, w.size.y),
                ),
            ),
            cad_geometry::add(origin, cad_geometry::scale(v, w.size.y)),
        ];
    }
    (
        polyline_semantics(points, true),
        Completeness::Partial(vec![
            "wipeout clip boundary drawn; the masking fill is not applied".into(),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn quad_face_fans_into_two_triangles_and_degenerate_faces_are_dropped() {
        let vertices = vec![
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(1.0, 1.0),
            point(0.0, 1.0),
        ];
        let geometry = triangle_mesh(
            vertices.clone(),
            &[vec![0, 1, 2, 3], vec![0, 1], vec![0, 1, 9]],
        );
        let SemanticGeometry::Mesh(mesh) = geometry else {
            panic!("expected a mesh");
        };
        assert_eq!(mesh.triangles.len(), 2, "{:?}", mesh.triangles);
    }

    #[test]
    fn polyface_indices_are_one_based_and_tolerate_negative_edges() {
        assert_eq!(polyface_index(1, 3), Some(0));
        assert_eq!(polyface_index(-3, 3), Some(2));
        assert_eq!(polyface_index(0, 3), None);
        assert_eq!(polyface_index(4, 3), None);
    }
}
