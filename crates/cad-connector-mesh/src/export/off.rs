//! ASCII OFF/COFF writer.
//!
//! `OFF` when no vertex carries a colour, `COFF` when every vertex does. All
//! source meshes are merged into one vertex/face table (OFF is one mesh per
//! file), and face indices are rebased per source mesh. Colours are 0-255
//! integers, the representation the OFF reader accepts without ambiguity.

use std::fmt::Write as _;

use crate::export::collect::Collected;
use crate::export::report::ReportBuilder;

/// Stable marker: source colours could not be kept for every vertex.
const COLORS_DROPPED: &str = "export.off.colors_dropped";
/// Stable marker: a written mesh's base-colour texture has no OFF channel here.
const TEXTURE_NOT_WRITTEN: &str = "export.off.texture_not_written";

pub(crate) fn write(collected: &Collected, report: &mut ReportBuilder) -> Vec<u8> {
    let write_colors = collected.colors_complete();
    if !write_colors && collected.has_any_colors() {
        report.converted(
            COLORS_DROPPED.to_string(),
            "not every vertex had a colour; OFF vertex colours were dropped instead of inventing the missing ones"
                .to_string(),
        );
    }

    let vertices: usize = collected
        .meshes
        .iter()
        .map(|mesh| mesh.vertices.len())
        .sum();
    let triangles = collected.triangle_count();

    let mut out = String::new();
    out.push_str(if write_colors { "COFF\n" } else { "OFF\n" });
    writeln!(out, "{vertices} {triangles} 0").expect("writing to a String cannot fail");
    for mesh in &collected.meshes {
        report.texture_not_written(TEXTURE_NOT_WRITTEN, "OFF", mesh);
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            if write_colors {
                let [red, green, blue] = mesh.colors[index];
                writeln!(
                    out,
                    "{} {} {} {red} {green} {blue}",
                    vertex.x, vertex.y, vertex.z
                )
                .expect("writing to a String cannot fail");
            } else {
                writeln!(out, "{} {} {}", vertex.x, vertex.y, vertex.z)
                    .expect("writing to a String cannot fail");
            }
        }
    }
    let mut base = 0u32;
    for mesh in &collected.meshes {
        for triangle in &mesh.triangles {
            writeln!(
                out,
                "3 {} {} {}",
                base + triangle[0],
                base + triangle[1],
                base + triangle[2]
            )
            .expect("writing to a String cannot fail");
        }
        base += mesh.vertices.len() as u32;
    }
    out.into_bytes()
}
