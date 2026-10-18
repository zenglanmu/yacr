use super::*;

use std::collections::HashSet;

/// One image vertex: world-space position (`@location(0)`) plus a normalized
/// texture coordinate (`@location(1)`). The two attributes share one buffer,
/// unlike the mesh pipeline's split position/normal/colour buffers.
pub(crate) fn image_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
    wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

/// Submit one uploaded image: bind the camera uniform (group 0, with this
/// image's dynamic slot offset) and the texture/sampler (group 2), then draw the
/// indexed quad.
pub(crate) fn draw_image(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    image: &GpuImage,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
    pipeline: &wgpu::RenderPipeline,
) {
    state.set_pipeline(pass, pipeline);
    state.set_bind_group0(pass, bind_group, uniform_offset);
    state.set_bind_group2(pass, &image.texture_bind_group);
    state.set_vertex_buffer(pass, 0, &image.vertex_buffer);
    state.set_index_buffer(pass, &image.index_buffer);
    pass.draw_indexed(0..image.index_count, 0, 0..1);
}

/// The world-space geometry for one image batch.
///
/// Without a `clip` this is the unit square `(0,0),(1,0),(1,1),(0,1)` mapped
/// through `transform`, with a two-triangle index list (`0,1,2, 0,2,3`). When
/// `clip` is `Some` the surviving convex polygon (already in world space, with
/// per-vertex texture coordinates) is used verbatim and fan-triangulated
/// (`0,1,2, 0,2,3, ...`); the batch `transform` is not applied again.
///
/// Texture coordinates are emitted with `uv.y = 1 - v`. Decoded image rows are
/// stored top-down and wgpu samples texel row 0 at `v = 0`, while the CAD/DXF
/// convention has the origin at the lower-left with `v` pointing up; without the
/// flip every image would render vertically mirrored.
pub(crate) fn image_quad_vertices(batch: &ImageBatch) -> (Vec<u8>, Vec<u32>) {
    if let Some(clip) = &batch.clip {
        let mut vertices = Vec::with_capacity(clip.len() * 20);
        for vertex in clip.iter() {
            let p = vertex.position;
            let uv = [vertex.uv[0] as f32, (1.0 - vertex.uv[1]) as f32];
            for component in [p.x as f32, p.y as f32, p.z as f32, uv[0], uv[1]] {
                vertices.extend_from_slice(&component.to_le_bytes());
            }
        }
        // Fan-triangulate the convex polygon: (0,1,2), (0,2,3), ...; fewer than
        // three vertices (a fully clipped-away image) yields no triangles.
        let mut indices = Vec::with_capacity(clip.len().saturating_sub(2) * 3);
        for i in 1..clip.len().saturating_sub(1) {
            indices.extend_from_slice(&[0, i as u32, (i + 1) as u32]);
        }
        return (vertices, indices);
    }
    const CORNERS: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut vertices = Vec::with_capacity(4 * 20);
    for [u, v] in CORNERS {
        let p = batch.transform.apply_point(Point3 { x: u, y: v, z: 0.0 });
        for component in [
            p.x as f32,
            p.y as f32,
            p.z as f32,
            u as f32,
            (1.0 - v) as f32,
        ] {
            vertices.extend_from_slice(&component.to_le_bytes());
        }
    }
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// Charge the frame budget for the resident image quads and return the accepted
/// indices (in upload order). The first image that crosses a limit stops the
/// scan and is reported as over budget — images are never silently dropped.
pub(crate) fn charge_images(
    usage: &mut cad_scene::FrameUsage,
    budget: &FrameBudget,
    images: &[GpuImage],
) -> (Vec<usize>, Option<OverBudgetReason>) {
    let mut accepted = Vec::with_capacity(images.len());
    // A texture is uploaded once per resource key, so its bytes are charged only
    // for the first resident quad that references it; later quads sharing the key
    // charge only their own vertex/index buffers.
    let mut charged_textures: HashSet<&ResourceKey> = HashSet::new();
    for (i, image) in images.iter().enumerate() {
        let mut next = *usage;
        let texture_bytes = if charged_textures.insert(&image.resource) {
            image.texture_bytes
        } else {
            0
        };
        let result = budget
            .charge_bytes(&mut next, image.upload_bytes + texture_bytes)
            .and_then(|()| budget.charge(&mut next, image.vertex_count as usize, 2));
        match result {
            Ok(()) => {
                *usage = next;
                accepted.push(i);
            }
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: images.len() - accepted.len(),
                };
                return (
                    accepted,
                    Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                );
            }
        }
    }
    (accepted, None)
}

// ---------------------------------------------------------------------------
// Pick keys and the CPU-side id → entity resolution table
// ---------------------------------------------------------------------------
//
// The id pass (see `pick.rs`) writes one 32-bit **pick key** per fragment. The
// key is a renderer key, `1 + <batch ordinal>`: the first resident batch is key
// 1, and 0 means "no geometry" (the background). `PickTable` is the CPU half of
// that contract: it maps a key back to the batch and to the *full* identities
// that can own a fragment of it.
//
// Why a renderer key rather than a truncated `EntityId`: a `u128` `EntityId`
// does not fit in the 32-bit id target, and packing its low 32 bits would make
// two distinct entities share a fragment value — a silent mis-pick. The table
// keeps the whole `SelectionRef`, so resolution is exact by construction. The
// truncation itself is still *detected and reported* (`PickTable::collisions`),
// because the low-32 form is what a 32-bit-only consumer would see.
//
// A batch may carry **several** sources (the scene builder concatenates adjacent
// line fragments of different entities into one batch, and the merged runs
// concatenate such batches). Fragment geometry does not record which source owns
// which range, so a hit on a multi-source batch resolves to the whole list and is
// reported as ambiguous — never silently attributed to `sources.first()`.
