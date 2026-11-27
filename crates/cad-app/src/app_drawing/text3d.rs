//! `TEXT3D` geometry: extrude the **filled glyph faces** the display already
//! builds into one `AcDbSubDMesh`.
//!
//! The shaping path is the same engine `TXTEXP`/the display use
//! (`FontEngine::shape_unstyled`), and it already returns one filled
//! [`cad_domain::Mesh`] per glyph: the even-odd fill with counters removed
//! (`o`, `e`, holes). This module owns only the **solid construction**:
//!
//! * each glyph's cap is its own fill triangulation, copied to `z` (normal
//!   −Z, reversed) and to `z + extrusion` (normal +Z);
//! * each **boundary** edge of a glyph fill (an undirected edge used by exactly
//!   one triangle) becomes one quad side wall between the two caps, facing
//!   outward (the fill is consistently CCW, so the wall is the right side of the
//!   edge).
//!
//! No re-triangulation happens here: the caps are the display's own triangles, so
//! a glyph's hole can never be filled by a second, weaker fill rule.
//!
//! An SHX face draws open pen strokes and contributes no filled glyph, so it is
//! refused by name instead of being extruded into a ribbon. The triangle/vertex
//! bounds are the only size limits: a string over them is refused, never
//! truncated.

use std::collections::HashMap;

use cad_domain::{CadError, Mesh, Point3};

/// Upper bound on the extruded triangles one `TEXT3D` writes.
pub(crate) const MAX_TEXT3D_TRIANGLES: usize = 131_072;

/// Upper bound on the extruded vertices one `TEXT3D` writes.
pub(crate) const MAX_TEXT3D_VERTICES: usize = 100_000;

/// A stable, named refusal: the build cannot represent this input.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// One built mesh plus the facts the outcome reports.
#[derive(Debug)]
pub(crate) struct Text3dMesh {
    pub(crate) mesh: Mesh,
    /// Filled glyphs that contributed geometry.
    pub(crate) glyphs: usize,
}

/// Extrude the filled glyph `meshes` from `z_base` by `extrusion` along `+Z`.
///
/// Refusals (nothing is written by this function): `text3d.not_solid` (no filled
/// glyph at all — an SHX outline-only face or a fill failure) and
/// `text3d.budget` (the assembled mesh exceeds
/// [`MAX_TEXT3D_TRIANGLES`]/[`MAX_TEXT3D_VERTICES`]).
pub(crate) fn build_text3d(
    glyphs: &[Mesh],
    z_base: f64,
    extrusion: f64,
) -> Result<Text3dMesh, CadError> {
    if !(z_base.is_finite() && extrusion.is_finite() && extrusion > 0.0) {
        return Err(refusal(
            "text3d.invalid_size",
            "the base z and extrusion must be finite, and the extrusion strictly positive",
        ));
    }
    let z_top = z_base + extrusion;
    if !z_top.is_finite() {
        return Err(refusal(
            "text3d.invalid_size",
            "the extrusion overflows to a non-finite top z",
        ));
    }
    let mut mesh = Mesh::default();
    let mut filled = 0usize;
    for glyph in glyphs {
        if glyph.triangles.is_empty() {
            continue;
        }
        if !glyph
            .vertices
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        {
            return Err(refusal(
                "text3d.degenerate",
                "the font produced a non-finite glyph vertex",
            ));
        }
        filled += 1;
        let cap = glyph.vertices.len() as u32;
        let base = mesh.vertices.len() as u32;
        // Bottom cap (−Z) then top cap (+Z), one vertex each per glyph vertex.
        for vertex in &glyph.vertices {
            mesh.vertices.push(Point3 {
                x: vertex.x,
                y: vertex.y,
                z: z_base,
            });
            mesh.normals.push(Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            });
        }
        for vertex in &glyph.vertices {
            mesh.vertices.push(Point3 {
                x: vertex.x,
                y: vertex.y,
                z: z_top,
            });
            mesh.normals.push(Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            });
        }
        for triangle in &glyph.triangles {
            let [a, b, c] = *triangle;
            // The fill is CCW seen from +Z: the top keeps it, the bottom reverses.
            mesh.triangles.push([base + a, base + c, base + b]);
            mesh.triangles
                .push([base + a + cap, base + b + cap, base + c + cap]);
        }
        // A boundary edge is used by exactly one triangle; its directed form in
        // that triangle puts the material on the left, so the outward wall is the
        // right side of the edge.
        let mut edges: HashMap<(u32, u32), (u32, u32, usize)> = HashMap::new();
        for triangle in &glyph.triangles {
            let corners = [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ];
            for (from, to) in corners {
                let key = if from < to { (from, to) } else { (to, from) };
                edges
                    .entry(key)
                    .and_modify(|entry| entry.2 += 1)
                    .or_insert((from, to, 1));
            }
        }
        for (_, (from, to, count)) in edges {
            if count != 1 {
                continue;
            }
            let p = glyph.vertices[from as usize];
            let q = glyph.vertices[to as usize];
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            let length = dx.hypot(dy);
            if length <= 1e-12 {
                continue;
            }
            let normal = Point3 {
                x: dy / length,
                y: -dx / length,
                z: 0.0,
            };
            let wall = mesh.vertices.len() as u32;
            for _ in 0..4 {
                mesh.normals.push(normal);
            }
            // p0 = p@z0, p1 = q@z0, p2 = q@z1, p3 = p@z1.
            mesh.vertices.push(Point3 {
                x: p.x,
                y: p.y,
                z: z_base,
            });
            mesh.vertices.push(Point3 {
                x: q.x,
                y: q.y,
                z: z_base,
            });
            mesh.vertices.push(Point3 {
                x: q.x,
                y: q.y,
                z: z_top,
            });
            mesh.vertices.push(Point3 {
                x: p.x,
                y: p.y,
                z: z_top,
            });
            // (p0,p1,p2) has normal (dy,−dx), the outward wall side.
            mesh.triangles.push([wall, wall + 1, wall + 2]);
            mesh.triangles.push([wall, wall + 2, wall + 3]);
        }
        if mesh.vertices.len() > MAX_TEXT3D_VERTICES || mesh.triangles.len() > MAX_TEXT3D_TRIANGLES
        {
            return Err(refusal(
                "text3d.budget",
                format!(
                    "the extruded text needs {} vertices / {} triangles, over the {} / {} bound",
                    mesh.vertices.len(),
                    mesh.triangles.len(),
                    MAX_TEXT3D_VERTICES,
                    MAX_TEXT3D_TRIANGLES
                ),
            ));
        }
    }
    if filled == 0 {
        return Err(refusal(
            "text3d.not_solid",
            "the resolved face produced no filled glyph (an SHX outline-only face or a fill \
             failure); TEXT3D needs a TrueType/OpenType outline face",
        ));
    }
    Ok(Text3dMesh {
        mesh,
        glyphs: filled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_glyph() -> Mesh {
        Mesh {
            vertices: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 10.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2]],
            normals: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                };
                3
            ],
            ..Mesh::default()
        }
    }

    #[test]
    fn one_filled_glyph_extrudes_into_two_caps_and_three_walls() {
        let built = build_text3d(&[triangle_glyph()], 0.0, 2.0).expect("extrude");
        assert_eq!(built.glyphs, 1);
        // 3 cap vertices × 2 caps + 3 boundary edges × 4 wall vertices.
        assert_eq!(built.mesh.vertices.len(), 18);
        assert_eq!(built.mesh.normals.len(), built.mesh.vertices.len());
        assert_eq!(built.mesh.triangles.len(), 2 + 6);
        // Both cap planes exist.
        assert!(built.mesh.vertices.iter().any(|p| p.z == 0.0));
        assert!(built.mesh.vertices.iter().any(|p| p.z == 2.0));
        // The caps face out, the walls are side-facing.
        assert!(built.mesh.normals.iter().any(|n| n.z == -1.0));
        assert!(built.mesh.normals.iter().any(|n| n.z == 1.0));
        assert!(built.mesh.normals.iter().any(|n| n.z == 0.0));
    }

    #[test]
    fn a_glyph_without_a_fill_is_refused_by_name() {
        let error = build_text3d(&[Mesh::default()], 0.0, 2.0).expect_err("no fill");
        assert!(
            format!("{error:?}").contains("text3d.not_solid"),
            "{error:?}"
        );
        let error = build_text3d(&[], 0.0, 2.0).expect_err("no glyphs");
        assert!(
            format!("{error:?}").contains("text3d.not_solid"),
            "{error:?}"
        );
    }

    #[test]
    fn a_mesh_over_the_triangle_bound_is_refused_not_truncated() {
        let mut glyph = triangle_glyph();
        glyph.triangles = vec![[0, 1, 2]; MAX_TEXT3D_TRIANGLES + 1];
        let error = build_text3d(&[glyph], 0.0, 1.0).expect_err("over budget");
        assert!(format!("{error:?}").contains("text3d.budget"), "{error:?}");
    }

    #[test]
    fn a_non_positive_or_non_finite_extrusion_is_refused() {
        for extrusion in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error =
                build_text3d(&[triangle_glyph()], 0.0, extrusion).expect_err("invalid extrusion");
            assert!(
                format!("{error:?}").contains("text3d.invalid_size"),
                "{extrusion}: {error:?}"
            );
        }
    }
}
