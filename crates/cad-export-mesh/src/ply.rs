//! Binary little-endian PLY 1.0 writer.
//!
//! The header declares `x y z`, optional `nx ny nz` (kept only when every vertex
//! has a finite normal) and optional `red green blue` (kept only when every
//! vertex has a colour), followed by one `property list uchar int
//! vertex_indices` face element. The payload is fixed-width little-endian, so
//! the output is deterministic.

use std::fmt::Write as _;

use crate::collect::Collected;
use crate::report::ReportBuilder;

/// Stable marker: declared normals could not be kept for every vertex.
const NORMALS_DROPPED: &str = "export.ply.normals_dropped";
/// Stable marker: source colours could not be kept for every vertex.
const COLORS_DROPPED: &str = "export.ply.colors_dropped";
/// Stable marker: a written mesh's base-colour texture has no PLY channel here.
const TEXTURE_NOT_WRITTEN: &str = "export.ply.texture_not_written";

pub(crate) fn write(collected: &Collected, report: &mut ReportBuilder) -> Vec<u8> {
    let write_colors = collected.colors_complete();
    if !write_colors && collected.has_any_colors() {
        report.converted(
            COLORS_DROPPED.to_string(),
            "not every vertex had a colour; PLY vertex colours were dropped instead of inventing the missing ones"
                .to_string(),
        );
    }
    let write_normals = collected.normals_complete();
    if !write_normals && collected.has_any_normals() {
        report.converted(
            NORMALS_DROPPED.to_string(),
            "not every vertex had a finite normal; PLY vertex normals were dropped instead of inventing the missing ones"
                .to_string(),
        );
    }

    let vertices: usize = collected
        .meshes
        .iter()
        .map(|mesh| mesh.vertices.len())
        .sum();
    let triangles = collected.triangle_count();

    let mut header = String::from("ply\nformat binary_little_endian 1.0\ncomment yacr\n");
    writeln!(header, "element vertex {vertices}").expect("writing to a String cannot fail");
    header.push_str("property float x\nproperty float y\nproperty float z\n");
    if write_normals {
        header.push_str("property float nx\nproperty float ny\nproperty float nz\n");
    }
    if write_colors {
        header.push_str("property uchar red\nproperty uchar green\nproperty uchar blue\n");
    }
    writeln!(header, "element face {triangles}").expect("writing to a String cannot fail");
    header.push_str("property list uchar int vertex_indices\nend_header\n");

    let mut out = header.into_bytes();
    for mesh in &collected.meshes {
        report.texture_not_written(TEXTURE_NOT_WRITTEN, "PLY", mesh);
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            push_f32(&mut out, vertex.x);
            push_f32(&mut out, vertex.y);
            push_f32(&mut out, vertex.z);
            if write_normals {
                let normal = mesh.normals[index];
                push_f32(&mut out, normal.x);
                push_f32(&mut out, normal.y);
                push_f32(&mut out, normal.z);
            }
            if write_colors {
                out.extend_from_slice(&mesh.colors[index]);
            }
        }
    }
    let mut base = 0u32;
    for mesh in &collected.meshes {
        for triangle in &mesh.triangles {
            out.push(3u8);
            for index in triangle {
                out.extend_from_slice(&((base + *index) as i32).to_le_bytes());
            }
        }
        base += mesh.vertices.len() as u32;
    }
    out
}

fn push_f32(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&(value as f32).to_le_bytes());
}
