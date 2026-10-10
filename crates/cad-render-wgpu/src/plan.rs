//! Batch planning, GPU packing and pipeline layout helpers.

use super::*;
use std::collections::HashSet;

pub(crate) struct GpuPlan {
    pub(crate) accepted: Vec<usize>,
    pub(crate) usage: cad_scene::FrameUsage,
    pub(crate) over_budget: Option<OverBudgetReason>,
}

/// The ordered, budget-accepted batch indices split into the two draw passes.
pub(crate) struct DrawPasses {
    pub(crate) opaque: Vec<usize>,
    pub(crate) transparent: Vec<usize>,
    pub(crate) invisible: usize,
}

/// Turn the pure [`DrawOrderPlan`] over the accepted sub-set back into global
/// batch indices, preserving the plan's order.
///
/// `accepted` are the global indices the budget kept (upload order); `entries`
/// are their ordering keys in the same order. Splitting this out keeps the index
/// remapping testable without a GPU.
pub(crate) fn draw_passes(
    accepted: &[usize],
    entries: &[BatchOrderEntry],
    camera_position: Option<[f32; 3]>,
) -> DrawPasses {
    debug_assert_eq!(accepted.len(), entries.len());
    let order = plan_draw_order(entries, camera_position);
    let remap = |positions: &[usize]| -> Vec<usize> {
        positions.iter().map(|&pos| accepted[pos]).collect()
    };
    DrawPasses {
        opaque: remap(&order.opaque),
        transparent: remap(&order.transparent),
        invisible: order.invisible.len(),
    }
}

/// Submit one batch, dispatching on its topology.
///
/// The caller chooses the mesh pipeline (opaque vs transparent, mirrored vs
/// normal); line and `MeshEdges` topologies always use the line pipeline. A mesh
/// batch also draws its optional wireframe overlay.
pub(crate) fn draw_batch(
    pass: &mut wgpu::RenderPass<'_>,
    batch: &GpuBatch,
    line_pipeline: &wgpu::RenderPipeline,
    mesh_pipeline: &wgpu::RenderPipeline,
) {
    match batch.topology {
        RenderTopology::Lines => {
            pass.set_pipeline(line_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            if let Some(indices) = &batch.indices {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.index_count, 0, 0..1);
            } else {
                pass.draw(0..batch.vertex_count, 0..1);
            }
        }
        RenderTopology::Mesh => {
            pass.set_pipeline(mesh_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            if let Some(normals) = &batch.normals {
                pass.set_vertex_buffer(1, normals.slice(..));
            }
            if let Some(colors) = &batch.colors {
                pass.set_vertex_buffer(2, colors.slice(..));
            }
            let Some(indices) = batch.indices.as_ref() else {
                return;
            };
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
            // Optional wireframe overlay over the shared mesh vertices.
            if let Some(edges) = &batch.edge_indices {
                pass.set_pipeline(line_pipeline);
                pass.set_vertex_buffer(0, batch.vertices.slice(..));
                pass.set_index_buffer(edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.edge_index_count, 0, 0..1);
            }
        }
        RenderTopology::MeshEdges => {
            pass.set_pipeline(line_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            let Some(indices) = batch.indices.as_ref() else {
                return;
            };
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
        }
    }
}

pub(crate) fn plan_from_gpu(batches: &[GpuBatch], budget: &FrameBudget) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for (i, batch) in batches.iter().enumerate() {
        let vertices = batch.vertex_count as usize;
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.index_count as usize / 3
        } else {
            0
        };
        // The GPU bytes actually submitted: position, normal and edge-index
        // buffers plus the triangle index buffer. Derived from the packed GpuBatch
        // so `upload_bytes_per_frame` is charged the same numbers the device sees.
        let bytes = batch.upload_size_bytes();
        let outcome = budget
            .charge_bytes(&mut usage, bytes)
            .and_then(|()| budget.charge(&mut usage, vertices, triangles));
        match outcome {
            Ok(()) => accepted.push(i),
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: batches.len() - accepted.len(),
                };
                return GpuPlan {
                    accepted,
                    usage,
                    over_budget: Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                };
            }
        }
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

/// Budget one consecutive page of a globally ordered scene. Unlike the legacy
/// one-frame planner, callers retain the cursor and never discard the suffix.
pub(crate) fn plan_ordered_page(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::new();
    for &index in &ordered[cursor..] {
        let batch = &batches[index];
        let mut next = usage;
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.index_count as usize / 3
        } else {
            0
        };
        let result = budget
            .charge_bytes(&mut next, batch.upload_size_bytes())
            .and_then(|()| budget.charge(&mut next, batch.vertex_count as usize, triangles));
        if let Err(exceeded) = result {
            let report = OverBudget {
                category: exceeded.category,
                requested: exceeded.requested,
                limit: exceeded.limit,
                skipped_batches: ordered.len() - cursor - accepted.len(),
            };
            return GpuPlan {
                accepted,
                usage,
                over_budget: Some(OverBudgetReason {
                    reason: over_budget_reason(&report),
                    report,
                }),
            };
        }
        usage = next;
        accepted.push(index);
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

pub(crate) fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
    DiagnosticReason::partial(
        codes::RENDER_FRAME_OVER_BUDGET,
        vec![
            DiagnosticParameter::Identifier(report.category.to_string()),
            DiagnosticParameter::Count(report.requested as u64),
            DiagnosticParameter::Limit(report.limit as u64),
            DiagnosticParameter::Count(report.skipped_batches as u64),
        ],
    )
}

pub(crate) fn pack_positions(vectors: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vectors.len() * 12);
    for v in vectors {
        bytes.extend_from_slice(&v[0].to_le_bytes());
        bytes.extend_from_slice(&v[1].to_le_bytes());
        bytes.extend_from_slice(&v[2].to_le_bytes());
    }
    bytes
}

/// Sanitise a batch colour for the uniform: finite `[0, 1]` channels.
///
/// Mirrors `cad_scene::sanitize_color` so a value that bypassed the scene cache
/// still cannot put a NaN into the shader.
pub(crate) fn renderer_color(color: [f32; 3]) -> [f32; 3] {
    let channel = |c: f32| {
        if c.is_finite() {
            c.clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    [channel(color[0]), channel(color[1]), channel(color[2])]
}

/// Aggregate the "lineweight carried but not drawn" batches into one stable
/// diagnostic reason. `None` when nothing asked for a non-zero weight.
pub(crate) fn lineweight_reason(not_drawn: &[LineweightNotDrawn]) -> Option<DiagnosticReason> {
    if not_drawn.is_empty() {
        return None;
    }
    let max_mm = not_drawn
        .iter()
        .map(|l| l.millimeters)
        .fold(0.0f32, f32::max);
    Some(DiagnosticReason::partial(
        codes::RENDER_LINEWEIGHT_NOT_DRAWN,
        vec![
            DiagnosticParameter::Count(not_drawn.len() as u64),
            DiagnosticParameter::Identifier(format!("{max_mm:.2}mm")),
        ],
    ))
}

pub(crate) fn translate_left(vp: &[[f32; 4]; 4], origin: [f32; 3]) -> [f32; 16] {
    // Column-major mat4x4 * translation(origin). Translation only affects the
    // last column: out[.][3] = vp[.][0]*ox + vp[.][1]*oy + vp[.][2]*oz + vp[.][3].
    let mut m = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] = vp[col][row];
        }
    }
    let ox = origin[0];
    let oy = origin[1];
    let oz = origin[2];
    for row in 0..4 {
        let base = vp[0][row] * ox + vp[1][row] * oy + vp[2][row] * oz + vp[3][row];
        m[12 + row] = base;
    }
    m
}

pub(crate) fn position_only_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

pub(crate) fn mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

/// Per-vertex normals travel in their own vertex buffer (slot 1, see
/// `draw_batch`). `mesh.wgsl` reads them at `@location(1)`, so the pipeline must
/// declare this layout too or `create_render_pipeline` fails validation with
/// "Location[1] ... is not provided by the previous stage outputs".
pub(crate) fn mesh_normal_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![1 => Float32x3],
    }
}

/// Per-vertex colours travel in their own vertex buffer (slot 2, see
/// `draw_batch`). The buffer is always present for a mesh — white when the
/// batch has no gradient — so `mesh.wgsl`'s `@location(2)` is always supplied.
pub(crate) fn mesh_color_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![2 => Float32x3],
    }
}

// Shaders are checked into `shaders/` so they can be validated as files and
// stay reviewable; `include_str!` keeps a single source of truth.
pub(crate) const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

// Mesh shader. Shading is intentionally explicit: a single fixed headlight
// direction, N·L diffuse plus a constant ambient term. There is no claim of
// physically based rendering, environment lighting or specular response.
pub(crate) const MESH_SHADER: &str = include_str!("../shaders/mesh.wgsl");

// Raster image shader. It samples a texture and modulates it by a per-image
// alpha/colour factor; there is no lighting model.
pub(crate) const IMAGE_SHADER: &str = include_str!("../shaders/image.wgsl");

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

/// Submit one uploaded image: bind the camera uniform (group 0) and the
/// texture/sampler (group 1), then draw the indexed quad.
pub(crate) fn draw_image(
    pass: &mut wgpu::RenderPass<'_>,
    image: &GpuImage,
    pipeline: &wgpu::RenderPipeline,
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &image.camera_bind_group, &[]);
    pass.set_bind_group(1, &image.texture_bind_group, &[]);
    pass.set_vertex_buffer(0, image.vertex_buffer.slice(..));
    pass.set_index_buffer(image.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
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
