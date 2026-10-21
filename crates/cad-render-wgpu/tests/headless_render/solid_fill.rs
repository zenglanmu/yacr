//! A tessellated closed solid must render as a **filled** face, not a wireframe.
//!
//! Regression: `Renderer::render` (the orthographic 2D camera) mapped world z
//! one-to-one onto clip z. WebGPU clip space is `0 <= z <= w`, so only the
//! 1-unit slab above the drawing plane was inside the frustum, and the visible
//! +Z face of a solid spanning `z ∈ [-1, 1]` landed at clip z = 1 — exactly the
//! cleared depth (1.0) — where the mesh pass's `Less` compare discards every
//! fill fragment. The `LessEqual` wireframe overlay of the same triangles kept
//! drawing, so a closed solid looked like its own wireframe. These tests read
//! the pixels back from a real (software-Vulkan) frame.

use super::*;

use cad_render_wgpu::headless::RgbaImage;
use cad_render_wgpu::RenderTier;

/// The `+Z` face of a `[-1, 1]³` cube, counter-clockwise seen from `+Z`, as two
/// triangles over four shared corners.
const TOP_FACE: [[f32; 3]; 4] = [
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0],
];

/// One quad of the cube: four corners wound CCW seen from outside, plus the
/// outward normal of its plane.
type Quad = ([f32; 3], [f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// A cube batch carrying **exactly** the tessellation `cad-kernel-adapter`
/// produces for the `3DSOLID` fixture: 24 vertices (four per face), 12 outward
/// wound triangles and one outward per-face normal per vertex. The winding of
/// the `+Z` face is stated explicitly so the renderer's front-face convention is
/// the one under test.
fn acis_cube(with_edge_overlay: bool) -> RenderBatch {
    // (corner a, b, c, d, outward normal) with the quad wound CCW seen from
    // outside, matching the adapter's facet output.
    let faces: [Quad; 6] = [
        (
            [1.0, -1.0, -1.0],
            [-1.0, -1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [1.0, 1.0, -1.0],
            [0.0, 0.0, -1.0],
        ),
        (
            TOP_FACE[0],
            TOP_FACE[1],
            TOP_FACE[2],
            TOP_FACE[3],
            [0.0, 0.0, 1.0],
        ),
        (
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
            [0.0, -1.0, 0.0],
        ),
        (
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ),
        (
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 0.0, 0.0],
        ),
        (
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [-1.0, 0.0, 0.0],
        ),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut normals = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(12);
    for (a, b, c, d, normal) in faces {
        let base = vertices.len() as u32;
        for corner in [a, b, c, d] {
            vertices.push(corner);
            normals.push(normal);
        }
        indices.push([base, base + 1, base + 2]);
        indices.push([base, base + 2, base + 3]);
    }
    let mut edges = Vec::new();
    if with_edge_overlay {
        // `DisplayPrimitive::Mesh` carries the wireframe overlay; the scene
        // expands it into one segment per triangle edge.
        for tri in &indices {
            for (i, j) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                edges.push(vertices[i as usize]);
                edges.push(vertices[j as usize]);
            }
        }
    }
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices,
        normals,
        colors: Vec::new(),
        indices,
        edges,
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source()],
        draw_order: 0,
    }
}

/// A camera whose 64 px span is four world units, so the `[-1, 1]³` cube covers
/// the centre half of the target and the corners stay background.
fn zoomed_out_camera_2d() -> Camera2d {
    let mut camera = camera_2d();
    camera.world_per_px = 4.0 / 64.0;
    camera
}

/// Render the cube on `tier` and read the frame back. `None` when no adapter (or
/// no accelerated tier) is available; a downgrade to the universal tier is
/// reported rather than silently accepted.
fn render_cube(tier: RenderTier, with_edge_overlay: bool) -> Option<RgbaImage> {
    let gpu = gpu()?;
    if tier == RenderTier::Accelerated {
        let limits = gpu.device.limits();
        if limits.max_storage_buffers_per_shader_stage < 3
            || limits.max_storage_buffer_binding_size < 64 * 1024
        {
            eprintln!("skipping accelerated-tier cube fill: adapter has no vertex-stage storage");
            return None;
        }
    }
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(tier == RenderTier::Accelerated));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    assert_eq!(
        renderer.render_tier(),
        tier,
        "the requested render tier must be the one under test"
    );
    renderer
        .upload(&scene(vec![acis_cube(with_edge_overlay)]))
        .expect("upload the cube mesh");
    let stats = renderer
        .render(zoomed_out_camera_2d(), &RenderTarget::new(64, 64))
        .expect("render the cube");
    if with_edge_overlay {
        assert!(
            stats.triangles > 12,
            "the wireframe overlay must add draw work on top of the 12 fill triangles, got {}",
            stats.triangles
        );
    } else {
        assert_eq!(stats.triangles, 12, "the cube must submit its 12 triangles");
    }
    Some(renderer.read_target_rgba().expect("read the frame"))
}

/// The centre pixel of the shape is the mesh's **fill**, not the clear colour.
///
/// The `+Z` (top) face is the face a top-down orthographic view sees. Its
/// per-vertex normals are `+Z` and the headlight is `-Z`, so the lit fill is the
/// batch colour at full diffuse (white); the clear colour is opaque black.
#[test]
fn tessellated_solid_face_is_filled_not_clear() {
    for tier in [RenderTier::Universal, RenderTier::Accelerated] {
        let Some(image) = render_cube(tier, false) else {
            return;
        };
        let clear = image.pixel(0, 0);
        assert_eq!(clear, [0, 0, 0, 255], "{tier:?}: the clear colour is black");
        let centre = image.pixel(32, 32);
        assert_ne!(
            centre, clear,
            "{tier:?}: the top face did not rasterize the centre pixel (got the clear colour)"
        );
        assert!(
            centre[0] > 240 && centre[1] > 240 && centre[2] > 240,
            "{tier:?}: the centre must be the lit white fill, got {centre:?}"
        );
    }
}

/// The same solid **with** the edge overlay `DisplayPrimitive::Mesh` asks for
/// (the app's path): the fill stays visible next to its wireframe, which is the
/// only thing that used to render.
#[test]
fn tessellated_solid_with_edge_overlay_is_still_filled() {
    for tier in [RenderTier::Universal, RenderTier::Accelerated] {
        let Some(image) = render_cube(tier, true) else {
            return;
        };
        let clear = image.pixel(0, 0);
        // The top face is split corner to corner, so the true centre sits on the
        // diagonal. Probe half-way between the centre and the left silhouette,
        // which is strictly inside one triangle.
        let inside = image.pixel(24, 32);
        assert_ne!(
            inside, clear,
            "{tier:?}: the overlay batch's fill did not rasterize (got the clear colour)"
        );
        assert!(
            inside[0] > 240 && inside[1] > 240 && inside[2] > 240,
            "{tier:?}: the interior must be the lit white fill, got {inside:?}"
        );
    }
}
