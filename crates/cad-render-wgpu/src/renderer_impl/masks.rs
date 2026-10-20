//! Wipeout mask upload and draw (ADR 0022 C1).
//!
//! A mask is not a geometry batch. It must hide what was painted before it, and
//! in a depth-tested single pass a later filled quad at the same world z is
//! rejected by the depth test (meshes compare `Less`, lines `LessEqual`, both
//! write depth). The renderer therefore owns a **final pass**:
//!
//! * `depth_compare: Always`, `depth_write_enabled: false` (see
//!   `Renderer::initialize_with_device`);
//! * the fragment colour is the renderer's clear colour
//!   ([`crate::CAD_CLEAR_COLOR`]), never a colour from the representation layer;
//! * the polygon is tessellated here with the same even-odd fill solid HATCH
//!   uses, so a concave boundary is filled exactly and the notch is not wiped.
//!
//! Draw order between masks and geometry is deliberately "masks last" for this
//! slice (ADR 0022 D2): `draw_order` is not threaded to the representation layer,
//! so no interleaving is claimed.

use super::*;

impl Renderer {
    /// Replace the resident WIPEOUT mask set from `masks`.
    ///
    /// Each boundary polygon is tessellated into a triangle list with
    /// [`cad_domain::hatch::fill_rings`] (even-odd, y-band trapezoid
    /// decomposition), which is the same fill solid HATCH uses: a **concave**
    /// boundary is filled exactly, so its notch is not wiped by a naive fan. A
    /// boundary that cannot be filled is **skipped and named** in
    /// [`Renderer::mask_skips`], never drawn as an approximate fill and never
    /// silently dropped.
    ///
    /// The fill is projected onto the world XY plane (a WIPEOUT is planar) and
    /// every emitted vertex takes the boundary's mean `z`, which only affects the
    /// clip-space position: the mask pass compares depth `Always` and writes no
    /// depth.
    ///
    /// Masks carry no pick key: they are not pickable geometry, so this does not
    /// touch the pick table or the key space.
    pub fn upload_masks(&mut self, masks: &[MaskBatch]) -> CadResult<()> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let mut staged = Vec::with_capacity(masks.len());
        let mut skips = Vec::new();
        for mask in masks {
            match mask_fill(&mask.polygon) {
                Ok(triangles) if !triangles.is_empty() => {
                    let origin = triangles[0];
                    let mut bytes = Vec::with_capacity(triangles.len() * 12);
                    for point in &triangles {
                        for component in [
                            (point[0] - origin[0]) as f32,
                            (point[1] - origin[1]) as f32,
                            (point[2] - origin[2]) as f32,
                        ] {
                            bytes.extend_from_slice(&component.to_le_bytes());
                        }
                    }
                    let vertices = Self::vertex_buffer(&device, Some(&queue), &bytes);
                    staged.push(GpuMask {
                        vertices,
                        vertex_count: triangles.len() as u32,
                        origin,
                    });
                }
                // Empty or refused: report the named reason instead of drawing an
                // approximate fill.
                Ok(_) => skips.push("wipeout mask boundary has no fillable area".to_string()),
                Err(reason) => skips.push(reason),
            }
        }
        self.masks = staged;
        self.mask_skips = skips;
        // Masks are scene data that change the frame's draw cost; the page-cost
        // knowledge is view-independent but scene-specific, like images.
        self.reset_pacing();
        Ok(())
    }

    /// Number of resident WIPEOUT masks.
    pub fn mask_count(&self) -> usize {
        self.masks.len()
    }

    /// Named reasons for mask boundaries the last [`Renderer::upload_masks`] call
    /// could not fill. Empty means every requested mask is resident.
    pub fn mask_skips(&self) -> &[String] {
        &self.mask_skips
    }

    /// Total vertices the resident masks submit (diagnostics).
    pub fn mask_vertex_count(&self) -> u64 {
        self.masks
            .iter()
            .map(|mask| u64::from(mask.vertex_count))
            .sum()
    }
}

/// Submit one mask triangle list: bind its uniform slot, then draw.
///
/// The caller has already bound the mask pipeline (the final pass binds it once).
pub(super) fn draw_mask(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    mask: &GpuMask,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
) {
    state.set_bind_group0(pass, bind_group, uniform_offset);
    state.set_vertex_buffer(pass, 0, &mask.vertices);
    pass.draw(0..mask.vertex_count, 0..1);
}

/// Tessellate one mask boundary into a world-space triangle list.
///
/// Uses the shared even-odd fill (`fill_rings`): rings are projected to XY, the
/// band decomposition handles concave contours and holes without a fan, and the
/// area cross-check refuses a self-intersecting boundary instead of filling it
/// wrongly. The error strings are mask-specific (the shared `FillError::reason`
/// talks about hatch boundaries).
pub(crate) fn mask_fill(polygon: &[Point3]) -> Result<Vec<[f64; 3]>, String> {
    use cad_domain::hatch::{fill_rings, FillError};
    if polygon.len() < 3 {
        return Err("wipeout mask boundary has fewer than three vertices".to_string());
    }
    let ring: Vec<[f64; 2]> = polygon.iter().map(|point| [point.x, point.y]).collect();
    let fill = fill_rings(std::slice::from_ref(&ring)).map_err(|error| match error {
        FillError::Empty => "wipeout mask boundary has no usable ring".to_string(),
        FillError::Budget { points, limit } => {
            format!("wipeout mask boundary has {points} points, over the {limit} point budget")
        }
        FillError::TooComplex => {
            "wipeout mask fill needs too many triangles to triangulate safely".to_string()
        }
        FillError::Degenerate => {
            "wipeout mask boundary is degenerate or self-intersecting; the fill is not drawn"
                .to_string()
        }
    })?;
    let count = polygon.len() as f64;
    let z = polygon.iter().map(|point| point.z).sum::<f64>() / count;
    let mut triangles = Vec::with_capacity(fill.triangles.len() * 3);
    for triangle in &fill.triangles {
        for &index in triangle {
            let vertex = fill.vertices[index as usize];
            triangles.push([vertex[0], vertex[1], z]);
        }
    }
    Ok(triangles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The L outline used by the headless concave test: bottom bar plus left bar.
    fn l_shape() -> Vec<Point3> {
        vec![
            point(-0.8, -0.8),
            point(0.2, -0.8),
            point(0.2, -0.1),
            point(-0.1, -0.1),
            point(-0.1, 0.2),
            point(-0.8, 0.2),
        ]
    }

    fn covers(triangles: &[[f64; 3]], x: f64, y: f64) -> bool {
        triangles.chunks_exact(3).any(|triangle| {
            let sign =
                |a: [f64; 3], b: [f64; 3]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
            let d1 = sign(triangle[0], triangle[1]);
            let d2 = sign(triangle[1], triangle[2]);
            let d3 = sign(triangle[2], triangle[0]);
            let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            !(has_neg && has_pos)
        })
    }

    #[test]
    fn concave_fill_covers_the_bars_and_not_the_notch() {
        let triangles = mask_fill(&l_shape()).expect("the L fills");
        assert!(
            covers(&triangles, -0.5, 0.0),
            "the left bar is inside the polygon"
        );
        assert!(
            covers(&triangles, 0.0, -0.5),
            "the bottom bar is inside the polygon"
        );
        assert!(
            !covers(&triangles, 0.05, 0.05),
            "the notch is outside the polygon and a fan would fill it"
        );
    }

    #[test]
    fn fill_uses_the_mean_boundary_z() {
        let mut polygon = l_shape();
        for (index, vertex) in polygon.iter_mut().enumerate() {
            vertex.z = index as f64;
        }
        let expected = (0..polygon.len()).map(|i| i as f64).sum::<f64>() / polygon.len() as f64;
        let triangles = mask_fill(&polygon).expect("the L fills");
        assert!(triangles.iter().all(|vertex| vertex[2] == expected));
    }

    #[test]
    fn degenerate_boundaries_are_refused_with_a_named_reason() {
        assert!(mask_fill(&[point(0.0, 0.0), point(1.0, 0.0)])
            .expect_err("two vertices have no area")
            .contains("fewer than three"));
        // A self-intersecting bowtie: the shared fill refuses it rather than
        // returning an approximate region.
        let bowtie = vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(1.0, 0.0),
            point(0.0, 1.0),
        ];
        let reason = mask_fill(&bowtie).expect_err("bowtie is self-intersecting");
        assert!(
            reason.contains("no usable ring")
                || reason.contains("degenerate")
                || reason.contains("self-intersecting"),
            "unexpected reason: {reason}"
        );
    }
}
