//! SDF conic (analytic circle/arc) fences — real GPU, slice B.
//!
//! These are **real-GPU** integration tests (the same headless path as
//! `headless_render.rs`: a real wgpu device on whatever adapter is visible, the
//! Quadro P620 / Vulkan on the project machine). They assert the SDF raster
//! against the **f64 analytic curve** the conic represents, so a wrong
//! parameterization, a missing `discard`, or a bad precision reduction fails.
//! See `docs/adr/0008-sdf-conic-rendering.md`.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};

use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu, RgbaImage};

use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

use cad_scene::{LinetypePattern, RenderBatch, RenderConic, RenderTopology, SceneDelta};

use std::f64::consts::TAU;

mod arc_core;
mod arc_ink;
mod unified_line;
const SIZE: u32 = 256;

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn conic_batch(conics: Vec<RenderConic>, entity: u128) -> RenderBatch {
    let origin = conics.first().map(|c| c.center).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Conics,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics,
        mirrored: false,
        alpha: 1.0,
        // White on the black clear: the encoded channel is a direct coverage read.
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

fn circle(center: Point3, radius: f64) -> RenderConic {
    RenderConic {
        center,
        major: Point3 {
            x: radius,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: radius,
            z: 0.0,
        },
        start: 0.0,
        sweep: TAU,
        full: true,
    }
}

fn arc(center: Point3, radius: f64, start: f64, sweep: f64) -> RenderConic {
    RenderConic {
        center,
        major: Point3 {
            x: radius,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: radius,
            z: 0.0,
        },
        start,
        sweep,
        full: false,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping SDF arc GPU test: {error}");
            None
        }
    }
}

/// The universal per-run tier: arcs are universal-only this round, so the tier
/// override makes the run deterministic regardless of the device.
fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    renderer
}

fn camera(center: Point3, world_per_px: f64) -> Camera2d {
    Camera2d {
        center,
        world_per_px,
        z_plane: 0.0,
    }
}

/// A camera whose `center` maps to a **pixel centre** (not a pixel corner), so a
/// conic centred there has its axis points on pixel centres and the SDF's peak
/// coverage is observable. Without this half-pixel shift the ring can fall on
/// pixel corners and no pixel centre is closer than ~0.5 px.
fn aligned_camera(center: Point3, world_per_px: f64) -> Camera2d {
    camera(
        Point3 {
            x: center.x + 0.5 * world_per_px,
            y: center.y + 0.5 * world_per_px,
            z: center.z,
        },
        world_per_px,
    )
}

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

/// World position of a pixel **center** for `camera` at the target size.
fn pixel_world(camera: &Camera2d, x: u32, y: u32) -> (f64, f64) {
    let half = SIZE as f64 / 2.0;
    let wx = camera.center.x + (x as f64 + 0.5 - half) * camera.world_per_px;
    let wy = camera.center.y - (y as f64 + 0.5 - half) * camera.world_per_px;
    (wx, wy)
}

/// The pixel whose **centre** is nearest the world point `(wx, wy)`.
fn pixel_at(camera: &Camera2d, wx: f64, wy: f64) -> (u32, u32) {
    let half = SIZE as f64 / 2.0;
    let x = half + (wx - camera.center.x) / camera.world_per_px - 0.5;
    let y = half - (wy - camera.center.y) / camera.world_per_px - 0.5;
    (
        x.round().clamp(0.0, (SIZE - 1) as f64) as u32,
        y.round().clamp(0.0, (SIZE - 1) as f64) as u32,
    )
}

/// The analytic signed distance (world units) from `p` to `conic`'s drawn set,
/// mirroring `shaders/arc.wgsl` exactly (the f64 reference the SDF is fenced to).
fn analytic_distance(conic: &RenderConic, p: (f64, f64)) -> f64 {
    let major_len = (conic.major.x * conic.major.x
        + conic.major.y * conic.major.y
        + conic.major.z * conic.major.z)
        .sqrt();
    let minor_len = (conic.minor.x * conic.minor.x
        + conic.minor.y * conic.minor.y
        + conic.minor.z * conic.minor.z)
        .sqrt();
    let u = (conic.major.x / major_len, conic.major.y / major_len);
    let v = (conic.minor.x / minor_len, conic.minor.y / minor_len);
    let q = (p.0 - conic.center.x, p.1 - conic.center.y);
    let x = q.0 * u.0 + q.1 * u.1;
    let y = q.0 * v.0 + q.1 * v.1;
    let d_ring = (x * x + y * y).sqrt() - major_len;
    let full = conic.full;
    if full {
        return d_ring.abs();
    }
    let t = y.atan2(x);
    let rel = (t - conic.start).rem_euclid(TAU);
    let inside = if conic.sweep >= 0.0 {
        rel <= conic.sweep
    } else {
        TAU - rel <= -conic.sweep
    };
    if inside {
        return d_ring.abs();
    }
    let e0 = (major_len * conic.start.cos(), major_len * conic.start.sin());
    let e1 = (
        major_len * (conic.start + conic.sweep).cos(),
        major_len * (conic.start + conic.sweep).sin(),
    );
    let d0 = ((x - e0.0).powi(2) + (y - e0.1).powi(2)).sqrt();
    let d1 = ((x - e1.0).powi(2) + (y - e1.1).powi(2)).sqrt();
    d0.min(d1)
}

/// Decode an sRGB8 channel back to linear coverage in `[0, 1]`.
fn coverage(channel: u8) -> f64 {
    let c = channel as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The rendered coverage at a pixel (the red channel; the batch is white).
fn rendered_coverage(image: &RgbaImage, x: u32, y: u32) -> f64 {
    coverage(image.pixel(x, y)[0])
}

/// Straight polyline segments as a `Lines` batch (one segment per vertex pair),
/// relative to `origin`.
fn straight_lines(origin: Point3, segments: Vec<[f64; 3]>, entity: u128) -> RenderBatch {
    let mut vertices = Vec::new();
    for pair in segments.chunks_exact(2) {
        for p in pair {
            vertices.push([
                (p[0] - origin.x) as f32,
                (p[1] - origin.y) as f32,
                (p[2] - origin.z) as f32,
            ]);
        }
    }
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// The peak rendered coverage inside a pixel band (`x_lo..=x_hi`), used to
/// isolate one class's core intensity when several classes share a frame.
fn band_peak(image: &RgbaImage, x_lo: u32, x_hi: u32) -> f64 {
    let mut peak = 0.0f64;
    for y in 0..SIZE {
        for x in x_lo..=x_hi.min(SIZE - 1) {
            peak = peak.max(rendered_coverage(image, x, y));
        }
    }
    peak
}

/// Fractional pixel coordinates of a world point, in the convention where an
/// integer value is a pixel **centre** (the inverse of [`pixel_world`]).
fn pixel_coord(camera: &Camera2d, wx: f64, wy: f64) -> (f64, f64) {
    let half = SIZE as f64 / 2.0;
    (
        half + (wx - camera.center.x) / camera.world_per_px - 0.5,
        half - (wy - camera.center.y) / camera.world_per_px - 0.5,
    )
}

/// Bilinearly interpolated **linear coverage** at a fractional pixel coordinate.
///
/// Linear coverage — not the sRGB-encoded channel — is the physical ink measure
/// (the display decodes sRGB to light before the eye integrates it). Integrating
/// the encoded channel instead inflates a soft anti-aliased stroke by up to
/// `√2` (a 1 px triangle filter reads ~1.4-1.47 while a hard 1 px line reads
/// 1.0), which is a measurement artifact, not a width defect.
fn coverage_at(image: &RgbaImage, px: f64, py: f64) -> f64 {
    let x0 = px.floor();
    let y0 = py.floor();
    let fx = px - x0;
    let fy = py - y0;
    let (x0, y0) = (x0 as i64, y0 as i64);
    let at = |x: i64, y: i64| -> f64 {
        if x < 0 || y < 0 || x >= SIZE as i64 || y >= SIZE as i64 {
            0.0
        } else {
            rendered_coverage(image, x as u32, y as u32)
        }
    };
    at(x0, y0) * (1.0 - fx) * (1.0 - fy)
        + at(x0 + 1, y0) * fx * (1.0 - fy)
        + at(x0, y0 + 1) * (1.0 - fx) * fy
        + at(x0 + 1, y0 + 1) * fx * fy
}

/// Integrated ink (in pixels) across a stroke: the linear coverage summed along
/// the ray leaving `center` at angle `theta`, over a ±2 px window around the
/// analytic `radius`. A hard 1 px line and a triangle filter of half-width 1 px
/// both integrate to exactly 1.0 (equal visual weight); the width scale is what
/// this measures, independent of the peak.
fn radial_ink(
    image: &RgbaImage,
    camera: &Camera2d,
    center: (f64, f64),
    radius: f64,
    theta: f64,
) -> f64 {
    let wpp = camera.world_per_px;
    let (ux, uy) = (theta.cos(), theta.sin());
    let steps = 512usize;
    let half_span = 2.0 * wpp;
    let step = 2.0 * half_span / steps as f64;
    let mut ink = 0.0;
    for i in 0..steps {
        let rr = radius - half_span + (i as f64 + 0.5) * step;
        let (px, py) = pixel_coord(camera, center.0 + rr * ux, center.1 + rr * uy);
        ink += coverage_at(image, px, py) * step / wpp;
    }
    ink
}

/// The same analytic circles/arcs chord-expanded into a `LineList` batch — the
/// pre-SDF draw, used as the visual-acceptance "before".
fn chord_lines_batch(conics: &[RenderConic], entity: u128) -> RenderBatch {
    let origin = conics.first().map(|c| c.center).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let mut vertices = Vec::new();
    for conic in conics {
        let segment = cad_representation::ConicSegment {
            center: conic.center,
            major: conic.major,
            minor: conic.minor,
            start: conic.start,
            sweep: conic.sweep,
            full: conic.full,
        };
        let points = segment.chord_points();
        for pair in points.windows(2) {
            for p in pair {
                vertices.push([
                    (p.x - origin.x) as f32,
                    (p.y - origin.y) as f32,
                    (p.z - origin.z) as f32,
                ]);
            }
        }
    }
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

// ===========================================================================
// Anti-aliased line-stroke unification fences (see
// `docs/adr/0009-antialiased-line-strokes.md`)
//
// Every line-like primitive is now an anti-aliased SDF quad stroke with the same
// semantics as the SDF conics: a triangle (linear) coverage filter over the
// distance-to-segment field with `w = world_per_px`, i.e. exactly 1.0 px of
// linear ink. These fences measure **linear** coverage (never the sRGB-encoded
// channel, which inflates soft strokes by ~√2).
// ===========================================================================

/// Integrated linear ink across a straight stroke: the linear coverage summed
/// along `normal` over a ±2 px window centred on `point`.
fn perpendicular_ink(
    image: &RgbaImage,
    camera: &Camera2d,
    point: (f64, f64),
    normal: (f64, f64),
) -> f64 {
    let wpp = camera.world_per_px;
    let steps = 512usize;
    let half_span = 2.0 * wpp;
    let step = 2.0 * half_span / steps as f64;
    let mut ink = 0.0;
    for i in 0..steps {
        let t = -half_span + (i as f64 + 0.5) * step;
        let (px, py) = pixel_coord(camera, point.0 + t * normal.0, point.1 + t * normal.1);
        ink += coverage_at(image, px, py) * step / wpp;
    }
    ink
}

/// A vertical `Lines` batch (one 2-vertex segment) at world `x`, spanning
/// `-half..half` in `y`, with an explicit alpha and colour.
fn vertical_line(x: f64, half: f64, alpha: f32, color: [f32; 3], entity: u128) -> RenderBatch {
    let mut batch = straight_lines(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        vec![[x, -half, 0.0], [x, half, 0.0]],
        entity,
    );
    batch.alpha = alpha;
    batch.color = color;
    batch.color_unresolved = false;
    batch
}

/// Core-ink mask of a frame: one boolean per pixel, true where the linear
/// coverage reaches the pickable/`0.5` threshold. Two captures of the same scene
/// (differing only in stroke rendering mode) must produce the same mask.
fn core_mask(image: &RgbaImage) -> Vec<bool> {
    image
        .pixels
        .chunks_exact(4)
        .map(|p| coverage(p[0]) >= 0.5)
        .collect()
}

/// The number of 8-connected components of a `SIZE x SIZE` boolean mask
/// (row-major). Used to prove two frames cover the same strokes.
fn mask_components(mask: &[bool]) -> usize {
    let n = SIZE as usize;
    let mut seen = vec![false; mask.len()];
    let mut stack = Vec::new();
    let mut components = 0usize;
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        components += 1;
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let x = i % n;
            let y = i / n;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    if nx < 0 || ny < 0 || nx >= n as i64 || ny >= n as i64 {
                        continue;
                    }
                    let j = ny as usize * n + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
    }
    components
}
