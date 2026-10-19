//! Shared fixtures for the `pick` tests.
//!
//! Every helper here is used by at least one theme module; no theme module
//! defines its own copy.

use cad_domain::*;

use super::super::*;

pub(super) fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

pub(super) fn ray(origin: Point3, direction: Point3) -> Ray3 {
    Ray3 { origin, direction }
}

pub(super) fn source(id: u128, instance: Vec<u128>) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath(instance.into_iter().map(EntityId).collect()),
        sub_element: None,
    }
}

pub(super) fn item(id: u128, geometry: SemanticGeometry) -> PickItem {
    PickItem {
        source: source(id, Vec::new()),
        geometry,
        transform: Transform3::identity(),
        geometry_source: GeometrySource::Analytic,
    }
}

pub(super) fn face(key: &str) -> SubElementId {
    SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    }
}

/// Two coplanar triangles at z = 2, side by side along x, each carrying a
/// distinct stable face source.
pub(super) fn two_face_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p(-2.0, -1.0, 2.0),
            p(-1.0, 1.0, 2.0),
            p(0.0, -1.0, 2.0),
            p(0.0, -1.0, 2.0),
            p(1.0, 1.0, 2.0),
            p(2.0, -1.0, 2.0),
        ],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: Vec::new(),
        face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
        colors: Vec::new(),
    }
}
