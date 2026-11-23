//! Wavefront OBJ writer.
//!
//! One `o` group per source entity, `v` per vertex, an optional `vn` per vertex
//! and one `f a//b` triple per triangle. OBJ has no vertex-colour channel here
//! and deliberately writes no `mtllib`, so source colours are reported as a
//! conversion instead of being invented as a material file.

use std::fmt::Write as _;

use crate::export::collect::Collected;
use crate::export::report::ReportBuilder;

/// Stable marker: source vertex colours have no OBJ channel in this writer.
const COLORS_DROPPED: &str = "export.obj.colors_dropped";
/// Stable marker: declared normals could not be kept for every vertex.
const NORMALS_DROPPED: &str = "export.obj.normals_dropped";
/// Stable marker: a written mesh's base-colour texture has no OBJ channel here.
const TEXTURE_NOT_WRITTEN: &str = "export.obj.texture_not_written";
/// Stable marker: a written body's stored display edges have no OBJ channel
/// here (no `l` line is written).
pub(crate) const STORED_EDGES_NOT_WRITTEN: &str = "export.obj.stored_edges_not_written";

pub(crate) fn write(collected: &Collected, report: &mut ReportBuilder) -> Vec<u8> {
    if collected.has_any_colors() {
        report.converted(
            COLORS_DROPPED.to_string(),
            "OBJ has no inline vertex-colour channel and no mtllib is written; source vertex colours were dropped"
                .to_string(),
        );
    }
    let write_normals = collected.normals_complete();
    if !write_normals && collected.has_any_normals() {
        report.converted(
            NORMALS_DROPPED.to_string(),
            "only some source meshes had a complete, finite normal per vertex; OBJ normals were dropped for the whole file instead of inventing the missing ones"
                .to_string(),
        );
    }
    let mut out = String::from("# yacr\n");
    // OBJ indices are 1-based and global across the file.
    let mut base = 1u32;
    for mesh in &collected.meshes {
        report.texture_not_written(TEXTURE_NOT_WRITTEN, "OBJ", mesh);
        let name = if mesh.type_key.is_empty() {
            "mesh"
        } else {
            mesh.type_key.as_str()
        };
        writeln!(out, "o {name}").expect("writing to a String cannot fail");
        for vertex in &mesh.vertices {
            writeln!(out, "v {} {} {}", vertex.x, vertex.y, vertex.z)
                .expect("writing to a String cannot fail");
        }
        if write_normals {
            for normal in &mesh.normals {
                writeln!(out, "vn {} {} {}", normal.x, normal.y, normal.z)
                    .expect("writing to a String cannot fail");
            }
        }
        for triangle in &mesh.triangles {
            let a = base + triangle[0];
            let b = base + triangle[1];
            let c = base + triangle[2];
            if write_normals {
                writeln!(out, "f {a}//{a} {b}//{b} {c}//{c}")
                    .expect("writing to a String cannot fail");
            } else {
                writeln!(out, "f {a} {b} {c}").expect("writing to a String cannot fail");
            }
        }
        base += mesh.vertices.len() as u32;
    }
    out.into_bytes()
}
