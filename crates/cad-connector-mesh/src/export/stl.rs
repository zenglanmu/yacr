//! STL writers (binary and ASCII).
//!
//! STL is a triangle soup with a per-facet normal and no colour channel. Every
//! facet normal is derived from its triangle (the neutral mesh's per-vertex
//! normals are not representable), a zero-area triangle is dropped and reported,
//! and any source vertex colour is reported as a conversion because the format
//! cannot store it.

use std::fmt::Write as _;

use cad_domain::Point3;

use crate::export::collect::Collected;
use crate::export::report::ReportBuilder;

/// Stable marker: a zero-area facet was dropped.
const DEGENERATE_TRIANGLE: &str = "export.stl.degenerate_triangle";
/// Stable marker: source vertex colours have no STL channel.
const COLORS_DROPPED: &str = "export.stl.colors_dropped";
/// Stable marker: a written mesh's base-colour texture has no STL channel.
const TEXTURE_NOT_WRITTEN: &str = "export.stl.texture_not_written";

struct Facet {
    normal: Point3,
    vertices: [Point3; 3],
}

/// Write the binary container (`u32` facet count, 50 bytes per facet).
pub(crate) fn write_binary(collected: &Collected, report: &mut ReportBuilder) -> Vec<u8> {
    let facets = collect_facets(collected, report);
    let mut out = Vec::with_capacity(84 + facets.len() * 50);
    let mut header = [0u8; 80];
    let label = b"yacr STL binary";
    header[..label.len()].copy_from_slice(label);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for facet in &facets {
        push_point(&mut out, facet.normal);
        for vertex in &facet.vertices {
            push_point(&mut out, *vertex);
        }
        // Attribute-byte count: always zero, STL has no per-facet colour here.
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// Write the ASCII container (`solid yacr` … `endsolid yacr`).
pub(crate) fn write_ascii(collected: &Collected, report: &mut ReportBuilder) -> Vec<u8> {
    let facets = collect_facets(collected, report);
    let mut out = String::from("solid yacr\n");
    for facet in &facets {
        writeln!(
            out,
            "  facet normal {} {} {}",
            facet.normal.x, facet.normal.y, facet.normal.z
        )
        .expect("writing to a String cannot fail");
        out.push_str("    outer loop\n");
        for vertex in &facet.vertices {
            writeln!(out, "      vertex {} {} {}", vertex.x, vertex.y, vertex.z)
                .expect("writing to a String cannot fail");
        }
        out.push_str("    endloop\n");
        out.push_str("  endfacet\n");
    }
    out.push_str("endsolid yacr\n");
    out.into_bytes()
}

/// Flatten every source mesh into facets, dropping and reporting zero-area ones.
fn collect_facets(collected: &Collected, report: &mut ReportBuilder) -> Vec<Facet> {
    if collected.has_any_colors() {
        report.converted(
            COLORS_DROPPED.to_string(),
            "STL has no colour channel; source vertex colours were dropped".to_string(),
        );
    }
    let mut facets = Vec::with_capacity(collected.triangle_count());
    for mesh in &collected.meshes {
        let mut degenerate = 0usize;
        for triangle in &mesh.triangles {
            let points = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            match triangle_normal(points) {
                Some(normal) => facets.push(Facet {
                    normal,
                    vertices: points,
                }),
                None => degenerate += 1,
            }
        }
        if degenerate > 0 {
            report.lost_triangles(
                mesh.object,
                &mesh.type_key,
                degenerate,
                DEGENERATE_TRIANGLE,
                format!(
                    "{degenerate} zero-area STL facet(s) were dropped; the stored triangle is degenerate"
                ),
            );
        }
        // Only a mesh with at least one written facet has geometry in the file;
        // an all-degenerate mesh wrote nothing and is already reported by name.
        if degenerate < mesh.triangles.len() {
            report.texture_not_written(TEXTURE_NOT_WRITTEN, "STL", mesh);
        }
    }
    facets
}

fn push_point(out: &mut Vec<u8>, point: Point3) {
    out.extend_from_slice(&(point.x as f32).to_le_bytes());
    out.extend_from_slice(&(point.y as f32).to_le_bytes());
    out.extend_from_slice(&(point.z as f32).to_le_bytes());
}

/// Normalised triangle normal, or `None` for a zero-area triangle.
fn triangle_normal(points: [Point3; 3]) -> Option<Point3> {
    let ux = points[1].x - points[0].x;
    let uy = points[1].y - points[0].y;
    let uz = points[1].z - points[0].z;
    let vx = points[2].x - points[0].x;
    let vy = points[2].y - points[0].y;
    let vz = points[2].z - points[0].z;
    let x = uy * vz - uz * vy;
    let y = uz * vx - ux * vz;
    let z = ux * vy - uy * vx;
    let length = (x * x + y * y + z * z).sqrt();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    Some(Point3 {
        x: x / length,
        y: y / length,
        z: z / length,
    })
}
