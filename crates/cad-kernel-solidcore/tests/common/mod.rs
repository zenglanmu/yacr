//! Shared helpers for the `cad-kernel-solidcore` integration tests.
//!
//! Split out so the chain tests and the probe tests can each be their own binary
//! while staying under the workspace's per-file line ceiling. Only the helpers a
//! binary happens to use are compiled into it, so unused-item warnings are
//! silenced here rather than per binary.
#![allow(dead_code)]

use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, DocumentId, Point3, TaskStamp,
};
use cad_kernel_adapter::{
    execute_solid, PrimitiveSpec, SolidModeler, SolidOperation, SolidOutcome, SolidRequest,
};

pub fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

pub fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 1)
}

pub fn request(operation: SolidOperation<'_>) -> SolidRequest<'_> {
    SolidRequest {
        stamp: stamp(),
        operation,
    }
}

pub fn never() -> impl Fn() -> bool + Sync {
    || false
}

pub fn execute(modeler: &dyn SolidModeler, operation: SolidOperation<'_>) -> SolidOutcome {
    let request = request(operation);
    execute_solid(modeler, &request, &never())
        .unwrap_or_else(|error| panic!("unexpected error: {error:?}"))
        .outcome
}

pub fn box_spec(origin: Point3, length: f64, width: f64, height: f64) -> PrimitiveSpec {
    PrimitiveSpec::Box {
        origin,
        length,
        width,
        height,
    }
}

pub fn volume(modeler: &dyn SolidModeler, brep: &BrepData) -> f64 {
    let outcome = execute(
        modeler,
        SolidOperation::MassProperties { brep, density: 1.0 },
    );
    let mass = outcome
        .mass()
        .unwrap_or_else(|| panic!("mass properties refused: {outcome:?}"));
    mass.volume
}

pub fn body(outcome: SolidOutcome) -> BrepData {
    match outcome {
        SolidOutcome::Success(product) => product
            .body()
            .cloned()
            .expect("a body product carries a B-rep"),
        other => panic!("expected a body, got {other:?}"),
    }
}

pub fn unsupported_code(outcome: &SolidOutcome) -> &'static str {
    outcome
        .unsupported()
        .unwrap_or_else(|| panic!("expected Unsupported, got {outcome:?}"))
        .code
}

/// Every face of a body, in shell order.
pub fn faces(brep: &BrepData) -> impl Iterator<Item = &BrepFace> {
    brep.shells.iter().flat_map(|shell| &shell.faces)
}

pub fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// The cap circles of a body's planar faces (a cylinder/cone's two ends).
pub fn planar_circles(brep: &BrepData) -> Vec<(Point3, f64)> {
    let mut out = Vec::new();
    for face in faces(brep) {
        if !matches!(face.surface, BrepSurface::Plane { .. }) {
            continue;
        }
        for loop_ in &face.loops {
            for curve in &loop_.edges {
                if let BrepCurve::Circle { center, radius, .. } = curve {
                    out.push((*center, *radius));
                }
            }
        }
    }
    out
}

/// A closed prism over a regular `sides`-gon: two planar caps of one loop each
/// plus `sides` planar side quads. Used to make a boolean slow enough to cancel.
pub fn prism(sides: usize, height: f64) -> BrepData {
    let radius = 1.0;
    let mut corners = Vec::with_capacity(sides * 2);
    for level in 0..2 {
        for index in 0..sides {
            let angle = std::f64::consts::TAU * (index as f64) / (sides as f64);
            corners.push(p(
                radius * angle.cos(),
                radius * angle.sin(),
                height * (level as f64),
            ));
        }
    }
    let mut face_list = Vec::with_capacity(sides + 2);
    let top: Vec<Point3> = (0..sides).map(|i| corners[sides + i]).collect();
    face_list.push(plane_face(0, &top));
    let bottom: Vec<Point3> = (0..sides).rev().map(|i| corners[i]).collect();
    face_list.push(plane_face(1, &bottom));
    for index in 0..sides {
        let next = (index + 1) % sides;
        let cycle = [
            corners[index],
            corners[next],
            corners[sides + next],
            corners[sides + index],
        ];
        face_list.push(plane_face((index + 2) as u32, &cycle));
    }
    BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: face_list,
        }],
        placement: None,
        tolerance: None,
    }
}

/// A planar face from an outward-wound cycle, with the same orthonormal frame
/// rule the crate's own polyhedra authoring uses.
pub fn plane_face(id: u32, cycle: &[Point3]) -> BrepFace {
    let origin = cycle[0];
    let edge = p(
        cycle[1].x - origin.x,
        cycle[1].y - origin.y,
        cycle[1].z - origin.z,
    );
    let across = p(
        cycle[2].x - origin.x,
        cycle[2].y - origin.y,
        cycle[2].z - origin.z,
    );
    let normal = p(
        edge.y * across.z - edge.z * across.y,
        edge.z * across.x - edge.x * across.z,
        edge.x * across.y - edge.y * across.x,
    );
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin,
            normal: p(normal.x / length, normal.y / length, normal.z / length),
            u_dir: edge,
        },
        reversed: false,
        loops: vec![BrepLoop {
            edges: (0..cycle.len())
                .map(|index| BrepCurve::Line {
                    start: cycle[index],
                    end: cycle[(index + 1) % cycle.len()],
                })
                .collect(),
        }],
        domain: None,
        tolerance: None,
    }
}
