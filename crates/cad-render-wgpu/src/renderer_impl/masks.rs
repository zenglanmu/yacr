//! Wipeout mask upload and draw (ADR 0022 C1/C2).
//!
//! A mask is not a geometry batch. It must hide what was painted before it, and
//! in a depth-tested single pass a later filled quad at the same world z is
//! rejected by the depth test (meshes compare `Less`, lines `LessEqual`, both
//! write depth). The renderer therefore owns a **final pass**:
//!
//! * `depth_compare: Always`, `depth_write_enabled: false` (see
//!   `Renderer::initialize_with_device`);
//! * the fragment colour is the renderer's clear colour for this frame
//!   ([`RenderTarget::clear_color`](crate::RenderTarget::clear_color), whose
//!   default is [`crate::CAD_CLEAR_COLOR`]), never a colour from the
//!   representation layer;
//! * the polygon is tessellated here with the same even-odd fill solid HATCH
//!   uses, so a concave boundary is filled exactly and the notch is not wiped.
//!
//! An **`inverted`** WIPEOUT (DXF clip mode Inside) masks everything *outside* its
//! boundary. The complement is expressed without a stencil by adding one more
//! ring to the same even-odd fill: a rectangle that covers the visible world rect
//! (plus a margin) and strictly contains the boundary. A point outside the
//! boundary is then crossed once (inside the ring) and is painted; a point inside
//! the boundary is crossed twice and stays visible. Because the ring depends on
//! the view, an inverted mask's geometry is built on its first frame and rebuilt
//! only when the view leaves the resident ring — never per frame on a steady
//! camera.
//!
//! Draw order between masks and geometry is deliberately "masks last" for this
//! slice (ADR 0022 D2): `draw_order` is not threaded to the representation layer,
//! so no interleaving is claimed.

use super::*;

/// Extra coverage an inverted mask's ring keeps beyond the region it must cover,
/// as a fraction of that region's largest side.
///
/// The margin lets a small pan or zoom reuse the resident triangulation instead
/// of rebuilding it; it also guarantees the ring strictly contains the boundary.
const MASK_RING_MARGIN: f64 = 0.25;

/// Perspective fallback multiplier for the visible footprint estimate.
///
/// A mask is a flat 2D entity, so the 3D path has no exact world-plane viewport
/// rect: the ring is grown to this many times the focus-plane half-extent, which
/// is deliberately generous. The limitation is named in the renderer docs rather
/// than claimed as exact.
const MASK_PERSPECTIVE_MARGIN: f64 = 64.0;

impl Renderer {
    /// Replace the resident WIPEOUT mask set from `masks`.
    ///
    /// A non-`inverted` boundary is tessellated immediately into a triangle list
    /// with [`cad_domain::hatch::fill_rings`] (even-odd, y-band trapezoid
    /// decomposition), which is the same fill solid HATCH uses: a **concave**
    /// boundary is filled exactly, so its notch is not wiped by a naive fan.
    ///
    /// An `inverted` mask keeps its world boundary here and builds its complement
    /// fill on the first frame that draws it (the ring needs the frame's visible
    /// world rect; see [`Renderer::ensure_inverted_mask_fill`]).
    ///
    /// A boundary that cannot be filled is **skipped and named** in
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
            if mask.polygon.len() < 3 {
                skips.push("wipeout mask boundary has fewer than three vertices".to_string());
                continue;
            }
            if mask.inverted {
                // The complement needs the frame's visible world rect; keep the
                // boundary and let the first frame build the ring.
                staged.push(GpuMask {
                    vertices: None,
                    vertex_count: 0,
                    origin: [0.0; 3],
                    polygon: mask.polygon.clone(),
                    inverted: true,
                    covered: None,
                });
                continue;
            }
            match mask_fill(&mask.polygon) {
                Ok(triangles) if !triangles.is_empty() => {
                    staged.push(staged_mask(
                        &device,
                        &queue,
                        &mask.polygon,
                        triangles,
                        false,
                        None,
                    ));
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

    /// Build (or rebuild) the complement fill of every resident `inverted` mask so
    /// it covers this frame's visible world rect.
    ///
    /// Called once per frame before the uniforms are written. A mask whose
    /// resident ring already contains the view is left untouched, so a steady
    /// camera or a small pan does no work. The ring is `view ∪ boundary AABB`
    /// grown by [`MASK_RING_MARGIN`], which guarantees it strictly contains the
    /// boundary: even-odd over the two rings is then exactly the complement inside
    /// the visible area.
    pub(crate) fn ensure_inverted_mask_fill(&mut self, view: MaskCoverage) {
        if !view.is_finite() || !self.masks.iter().any(|mask| mask.inverted) {
            return;
        }
        let (Some(device), Some(queue)) = (self.device.clone(), self.queue.clone()) else {
            return;
        };
        for index in 0..self.masks.len() {
            if !self.masks[index].inverted {
                continue;
            }
            if self.masks[index]
                .covered
                .is_some_and(|covered| covered.contains(view))
            {
                continue;
            }
            let boundary = self.masks[index].polygon.clone();
            let required = view.union(MaskCoverage::boundary(&boundary));
            let extent = (required.max[0] - required.min[0])
                .max(required.max[1] - required.min[1])
                .max(1.0);
            let ring = required.expanded(extent * MASK_RING_MARGIN);
            match inverted_mask_fill(&boundary, ring) {
                Ok(triangles) if !triangles.is_empty() => {
                    let built =
                        staged_mask(&device, &queue, &boundary, triangles, true, Some(ring));
                    self.masks[index] = built;
                }
                Ok(_) => {
                    self.masks[index].vertex_count = 0;
                    self.masks[index].vertices = None;
                    self.masks[index].covered = None;
                    let reason = "wipeout mask complement fill has no fillable area".to_string();
                    if !self.mask_skips.contains(&reason) {
                        self.mask_skips.push(reason);
                    }
                }
                Err(reason) => {
                    self.masks[index].vertex_count = 0;
                    self.masks[index].vertices = None;
                    self.masks[index].covered = None;
                    if !self.mask_skips.contains(&reason) {
                        self.mask_skips.push(reason);
                    }
                }
            }
        }
    }

    /// Number of resident WIPEOUT masks.
    pub fn mask_count(&self) -> usize {
        self.masks.len()
    }

    /// Named reasons for mask boundaries the last [`Renderer::upload_masks`] or
    /// [`Renderer::ensure_inverted_mask_fill`] call could not fill. Empty means
    /// every requested mask is resident and drawn.
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

/// Pack one tessellated mask into its device buffer and resident record.
fn staged_mask(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    boundary: &[Point3],
    triangles: Vec<[f64; 3]>,
    inverted: bool,
    covered: Option<MaskCoverage>,
) -> GpuMask {
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
    GpuMask {
        vertices: Some(Renderer::vertex_buffer(device, Some(queue), &bytes)),
        vertex_count: triangles.len() as u32,
        origin,
        polygon: Arc::from(boundary.to_vec().into_boxed_slice()),
        inverted,
        covered,
    }
}

/// Submit one mask triangle list: bind its uniform slot, then draw.
///
/// The caller has already bound the mask pipeline (the final pass binds it once).
/// A mask whose fill was refused (or not built yet) has no vertex buffer and draws
/// nothing; the reason is reported through [`Renderer::mask_skips`].
pub(super) fn draw_mask(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    mask: &GpuMask,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
) {
    let Some(vertices) = mask.vertices.as_ref() else {
        return;
    };
    state.set_bind_group0(pass, bind_group, uniform_offset);
    state.set_vertex_buffer(pass, 0, vertices);
    pass.draw(0..mask.vertex_count, 0..1);
}

/// The visible world rect of one frame, used to size an inverted mask's ring.
///
/// The orthographic camera gives the exact rect from its centre and world units
/// per pixel. A perspective camera has no single world-plane rect for a flat
/// entity, so this returns a deliberately generous square around the camera
/// position at [`MASK_PERSPECTIVE_MARGIN`] times the focus-plane half-extent; a
/// mask plane seen obliquely could still need more, which is a named limitation,
/// not a silently assumed exactness.
pub(super) fn mask_view_coverage(
    projection: FrameProjection,
    target_size: (u32, u32),
    camera_position: [f32; 3],
) -> MaskCoverage {
    match projection {
        FrameProjection::Ortho {
            center,
            world_per_px,
            ..
        } => {
            let half_w = world_per_px * f64::from(target_size.0.max(1)) * 0.5;
            let half_h = world_per_px * f64::from(target_size.1.max(1)) * 0.5;
            MaskCoverage::new(
                [center[0] - half_w, center[1] - half_h],
                [center[0] + half_w, center[1] + half_h],
            )
        }
        FrameProjection::Perspective { world_per_px, .. } => {
            let half = world_per_px
                * f64::from(target_size.0.max(target_size.1).max(1))
                * MASK_PERSPECTIVE_MARGIN;
            MaskCoverage::around(
                [f64::from(camera_position[0]), f64::from(camera_position[1])],
                half.max(1.0),
            )
        }
    }
}

/// Tessellate one mask boundary into a world-space triangle list.
///
/// Uses the shared even-odd fill (`fill_rings`): rings are projected to XY, the
/// band decomposition handles concave contours and holes without a fan, and the
/// area cross-check refuses a self-intersecting boundary instead of filling it
/// wrongly. The error strings are mask-specific (the shared `FillError::reason`
/// talks about hatch boundaries).
pub(crate) fn mask_fill(polygon: &[Point3]) -> Result<Vec<[f64; 3]>, String> {
    fill_mask_rings(polygon, None)
}

/// Tessellate the **complement** of `polygon` inside `ring`.
///
/// Even-odd over `[ring, polygon]` fills exactly the points that are inside the
/// ring but outside the polygon, i.e. the inverted mask region within the visible
/// area. `ring` must strictly contain the boundary (the caller builds it from the
/// union of the view and the boundary bounds).
pub(crate) fn inverted_mask_fill(
    polygon: &[Point3],
    ring: MaskCoverage,
) -> Result<Vec<[f64; 3]>, String> {
    let ring = vec![
        [ring.min[0], ring.min[1]],
        [ring.max[0], ring.min[1]],
        [ring.max[0], ring.max[1]],
        [ring.min[0], ring.max[1]],
    ];
    fill_mask_rings(polygon, Some(ring))
}

/// Shared fill: `ring` (already in XY) is prepended when present, so the even-odd
/// rule turns the boundary into a hole of that ring.
fn fill_mask_rings(
    polygon: &[Point3],
    ring: Option<Vec<[f64; 2]>>,
) -> Result<Vec<[f64; 3]>, String> {
    use cad_domain::hatch::{fill_rings, FillError};
    if polygon.len() < 3 {
        return Err("wipeout mask boundary has fewer than three vertices".to_string());
    }
    let boundary: Vec<[f64; 2]> = polygon.iter().map(|point| [point.x, point.y]).collect();
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::with_capacity(2);
    if let Some(ring) = ring {
        rings.push(ring);
    }
    rings.push(boundary);
    let fill = fill_rings(&rings).map_err(|error| match error {
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

    /// The L outline used by the headless concave tests: bottom bar plus left bar.
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

    /// The ring the frame path would build for the 64x64 test view: view
    /// `[-1, 1]^2` unioned with the boundary bounds, then grown by the margin.
    fn enclosing_ring(polygon: &[Point3]) -> MaskCoverage {
        let view = MaskCoverage::new([-1.0, -1.0], [1.0, 1.0]);
        let required = view.union(MaskCoverage::boundary(polygon));
        let extent = (required.max[0] - required.min[0])
            .max(required.max[1] - required.min[1])
            .max(1.0);
        required.expanded(extent * MASK_RING_MARGIN)
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
    fn inverted_fill_is_the_complement_inside_the_ring() {
        let polygon = l_shape();
        let ring = enclosing_ring(&polygon);
        assert!(
            ring.contains(MaskCoverage::boundary(&polygon)),
            "the ring must strictly contain the boundary for even-odd nesting"
        );
        let triangles = inverted_mask_fill(&polygon, ring).expect("the L complement fills");
        // Inside the polygon: visible (covered twice, hence not filled).
        assert!(!covers(&triangles, -0.5, 0.0), "the left bar stays visible");
        assert!(
            !covers(&triangles, 0.0, -0.5),
            "the bottom bar stays visible"
        );
        // The notch is outside the polygon: wiped.
        assert!(
            covers(&triangles, 0.05, 0.05),
            "the notch is masked by the inverted fill"
        );
        // Far outside the polygon but inside the ring: wiped.
        assert!(covers(&triangles, 0.6, 0.6), "the outside is masked");
        assert!(
            covers(&triangles, -0.95, 0.95),
            "the viewport corner is masked"
        );
    }

    /// A boundary larger than the view (an inverted mask that surrounds the
    /// screen) must mask nothing visible: the ring is grown to contain it, so the
    /// frame's interior is covered twice and stays visible.
    #[test]
    fn inverted_fill_keeps_a_boundary_larger_than_the_view_clear() {
        let big = vec![
            point(-50.0, -50.0),
            point(50.0, -50.0),
            point(50.0, 50.0),
            point(-50.0, 50.0),
        ];
        let ring = enclosing_ring(&big);
        assert!(
            ring.contains(MaskCoverage::new([-50.0, -50.0], [50.0, 50.0])),
            "the ring must contain a boundary larger than the view"
        );
        let triangles = inverted_mask_fill(&big, ring).expect("the complement fills");
        assert!(
            !covers(&triangles, 0.0, 0.0),
            "the viewport inside a huge boundary stays visible"
        );
        assert!(
            covers(&triangles, 0.0, 60.0),
            "outside the huge boundary is masked"
        );
    }

    #[test]
    fn inverted_fill_without_a_containing_ring_still_needs_three_vertices() {
        assert!(inverted_mask_fill(
            &[point(0.0, 0.0), point(1.0, 0.0)],
            MaskCoverage::new([-1.0, -1.0], [1.0, 1.0])
        )
        .expect_err("two vertices have no area")
        .contains("fewer than three"));
    }

    #[test]
    fn orthographic_view_coverage_is_the_exact_visible_rect() {
        // 64x64 target at 2 world units across, centred on (0.5, -0.25).
        let projection = FrameProjection::Ortho {
            sx: 1.0,
            sy: 1.0,
            center: [0.5, -0.25, 0.0],
            z_plane: 0.0,
            depth: OrthoDepth::default(),
            world_per_px: 2.0 / 64.0,
        };
        let view = mask_view_coverage(projection, (64, 64), [0.5, -0.25, 0.0]);
        assert_eq!(
            view,
            MaskCoverage::new([-0.5, -1.25], [1.5, 0.75]),
            "the ortho view rect must be exact, not a guess"
        );
    }

    #[test]
    fn perspective_view_coverage_is_generous_around_the_camera() {
        let projection = FrameProjection::Perspective {
            vp: [[0.0; 4]; 4],
            world_per_px: 0.01,
        };
        let view = mask_view_coverage(projection, (64, 64), [10.0, -4.0, 2.0]);
        assert_eq!(view.min, [10.0 - 40.96, -4.0 - 40.96]);
        assert_eq!(view.max, [10.0 + 40.96, -4.0 + 40.96]);
        assert!(view.contains(MaskCoverage::new([9.0, -5.0], [11.0, -3.0])));
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
        let inverted =
            inverted_mask_fill(&polygon, enclosing_ring(&polygon)).expect("the L complement fills");
        assert!(inverted.iter().all(|vertex| vertex[2] == expected));
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
