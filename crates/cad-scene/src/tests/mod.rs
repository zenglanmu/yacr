//! Unit tests.

use super::*;
use cad_representation::DisplayFragment;
use std::sync::Arc;

mod budget;
mod cache;
mod centroid;
mod fragments;
mod image;
mod lines;
mod mesh;
mod style;

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 0)
}

fn line_representation(entity: u128, points: Vec<Point3>) -> DisplayRepresentation {
    DisplayRepresentation {
        fragments: vec![DisplayFragment {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(entity),
                instance: InstancePath::default(),
                sub_element: None,
            },
            geometry_source: GeometrySource::Analytic,
            precision: Precision::Analytic,
            alpha: 1.0,
            color: cad_representation::DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: cad_db::LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
        }],
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    }
}

fn mesh_representation(entity: u128, mesh: Mesh) -> DisplayRepresentation {
    DisplayRepresentation {
        fragments: vec![DisplayFragment {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(entity),
                instance: InstancePath::default(),
                sub_element: None,
            },
            geometry_source: GeometrySource::DirectMesh,
            precision: Precision::Analytic,
            alpha: 1.0,
            color: cad_representation::DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: cad_db::LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Mesh(std::sync::Arc::new(mesh)),
        }],
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    }
}

fn quad() -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            };
            4
        ],
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

fn unit_points() -> Vec<Point3> {
    vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ]
}
