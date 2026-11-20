//! Affine and frame-conversion helpers for the glTF importer.
//!
//! glTF node matrices are **column-major** (`gltf::scene::Transform::matrix`), a
//! child's world transform is `parent_world * local`, and positions are
//! transformed in the glTF frame first (metres, +Y up) before being mapped to the
//! CAD frame (metres, +Z up) as `(x, y, z) -> (x, -z, y)`.
//!
//! Splitting these out of the scene walk keeps the traversal readable and each
//! file inside the repository's line ceiling; they are also the only place the
//! column-major/row-major conversion appears, so a frame bug has one home.

use cad_domain::{CadError, CadResult, Point3};
use gltf::Node;

use crate::DIAG_INVALID_TRANSFORM;

/// Whether a triangle has zero area after its node transform.
pub(crate) fn is_zero_area(triangle: [u32; 3], vertices: &[Point3]) -> bool {
    let p0 = vertices[triangle[0] as usize];
    let p1 = vertices[triangle[1] as usize];
    let p2 = vertices[triangle[2] as usize];
    let ux = p1.x - p0.x;
    let uy = p1.y - p0.y;
    let uz = p1.z - p0.z;
    let vx = p2.x - p0.x;
    let vy = p2.y - p0.y;
    let vz = p2.z - p0.z;
    let cx = uy * vz - uz * vy;
    let cy = uz * vx - ux * vz;
    let cz = ux * vy - uy * vx;
    cx == 0.0 && cy == 0.0 && cz == 0.0
}

/// `glTF [col][row]` -> row-major affine, in f64.
pub(crate) fn local_matrix(node: &Node<'_>) -> CadResult<[[f64; 4]; 4]> {
    let matrix = node.transform().matrix();
    let mut out = [[0.0f64; 4]; 4];
    for (row, output) in out.iter_mut().enumerate() {
        for (column, value) in output.iter_mut().enumerate() {
            *value = matrix[column][row] as f64;
        }
    }
    if !out.iter().flatten().all(|value| value.is_finite()) {
        return Err(CadError::CorruptData(format!(
            "{DIAG_INVALID_TRANSFORM}: glTF node {} has a non-finite transform",
            node.index()
        )));
    }
    Ok(out)
}

pub(crate) fn multiply(left: &[[f64; 4]; 4], right: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0f64; 4]; 4];
    for (row, output) in out.iter_mut().enumerate() {
        for (column, value) in output.iter_mut().enumerate() {
            *value = (0..4).map(|k| left[row][k] * right[k][column]).sum();
        }
    }
    out
}

pub(crate) fn apply(matrix: &[[f64; 4]; 4], point: Point3) -> Point3 {
    Point3 {
        x: matrix[0][0] * point.x + matrix[0][1] * point.y + matrix[0][2] * point.z + matrix[0][3],
        y: matrix[1][0] * point.x + matrix[1][1] * point.y + matrix[1][2] * point.z + matrix[1][3],
        z: matrix[2][0] * point.x + matrix[2][1] * point.y + matrix[2][2] * point.z + matrix[2][3],
    }
}

/// glTF (+Y up) -> CAD (+Z up).
pub(crate) fn to_cad_frame(point: Point3) -> Point3 {
    Point3 {
        x: point.x,
        y: -point.z,
        z: point.y,
    }
}

/// The inverse-transpose of the affine's linear part, or `None` when singular.
pub(crate) fn normal_matrix(matrix: &[[f64; 4]; 4]) -> Option<[[f64; 3]; 3]> {
    let a = [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if !det.is_finite() || det.abs() <= f64::EPSILON {
        return None;
    }
    let inverse = 1.0 / det;
    let cofactor = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inverse,
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inverse,
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inverse,
        ],
        [
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inverse,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inverse,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inverse,
        ],
        [
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inverse,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inverse,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inverse,
        ],
    ];
    cofactor
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(cofactor)
}

/// Transform every normal by `matrix`, then into the CAD frame and normalize.
///
/// `None` when any result is zero-length or non-finite: the caller drops the
/// whole set rather than storing a partial or zeroed list.
pub(crate) fn transform_normals(normals: &[Point3], matrix: &[[f64; 3]; 3]) -> Option<Vec<Point3>> {
    let mut out = Vec::with_capacity(normals.len());
    for normal in normals {
        let transformed = Point3 {
            x: matrix[0][0] * normal.x + matrix[0][1] * normal.y + matrix[0][2] * normal.z,
            y: matrix[1][0] * normal.x + matrix[1][1] * normal.y + matrix[1][2] * normal.z,
            z: matrix[2][0] * normal.x + matrix[2][1] * normal.y + matrix[2][2] * normal.z,
        };
        let cad = to_cad_frame(transformed);
        let length = (cad.x * cad.x + cad.y * cad.y + cad.z * cad.z).sqrt();
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        out.push(Point3 {
            x: cad.x / length,
            y: cad.y / length,
            z: cad.z / length,
        });
    }
    Some(out)
}
