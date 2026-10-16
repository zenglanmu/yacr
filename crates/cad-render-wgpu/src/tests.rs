//! Unit tests.

use super::*;

#[test]
fn uninitialized_renderer_reports_no_active_backend() {
    // A preference is not an activated backend. Before the host supplies a
    // device there is no actual backend and no capabilities (audit B02).
    for preference in [
        BackendPreference::Auto,
        BackendPreference::WebGpu,
        BackendPreference::WebGl2,
    ] {
        let renderer = Renderer::new(preference);
        assert_eq!(renderer.active_backend(), None);
        assert!(
            renderer.capabilities().is_none(),
            "no device yet for {preference:?}"
        );
    }
}

#[test]
fn preference_and_active_backend_are_distinct_concepts() {
    // The requested preference is retained even while activation is pending.
    let renderer = Renderer::new(BackendPreference::WebGl2);
    assert_eq!(renderer.preference, BackendPreference::WebGl2);
    assert_eq!(renderer.active_backend(), None);
}

#[test]
fn backend_names_are_stable() {
    assert_eq!(ActiveBackend::WebGpu.as_str(), "webgpu");
    assert_eq!(ActiveBackend::WebGl2.as_str(), "webgl2");
    assert_eq!(ActiveBackend::Native.as_str(), "native");
}

#[test]
fn mesh_pipeline_declares_position_and_normal_attributes() {
    // Regression: `mesh.wgsl` reads `@location(0)` position and
    // `@location(1)` normal from two separate vertex buffers. The pipeline
    // must declare both layouts or wgpu rejects `cad-mesh-pipeline` at
    // creation ("Location[1] ... is not provided by the previous stage
    // outputs").
    let layouts = [mesh_vertex_layout(), mesh_normal_layout()];
    let locations: Vec<u32> = layouts
        .iter()
        .flat_map(|layout| layout.attributes.iter().map(|attr| attr.shader_location))
        .collect();
    assert_eq!(locations, vec![0, 1]);
    assert!(layouts.iter().all(|layout| layout.array_stride == 12));
}

#[test]
fn render_errors_distinguish_device_loss_from_a_bad_frame() {
    assert!(RenderError::DeviceLost("reset".into()).is_device_loss());
    assert!(!RenderError::Frame("validation".into()).is_device_loss());
    assert!(!RenderError::NotInitialized("no device".into()).is_device_loss());
}

#[test]
fn upload_time_is_absent_before_any_upload() {
    let renderer = Renderer::default();
    assert!(renderer.last_upload_ms().is_none());
}

#[test]
fn commit_timing_is_absent_before_any_commit_and_defaults_to_zero() {
    let renderer = Renderer::default();
    assert!(renderer.last_commit_timing().is_none());
    let timing = crate::CommitTiming::default();
    assert_eq!(timing.total_us, 0);
    for stage in [
        timing.rebase_us,
        timing.append_us,
        timing.gc_us,
        timing.pulled_us,
        timing.pick_us,
        timing.lineweight_us,
    ] {
        assert_eq!(stage, 0);
    }
}

#[test]
fn render_before_init_is_not_a_device_loss() {
    let mut renderer = Renderer::default();
    let target = RenderTarget::new(64, 64);
    let frame = renderer.render(Camera2d::default(), &target);
    assert!(matches!(frame, Err(RenderError::NotInitialized(_))));
    assert!(!renderer.is_device_lost());
}

#[test]
fn noted_device_loss_is_explicit_and_clears_batches() {
    let mut renderer = Renderer::default();
    // Simulate a batch having been uploaded, then a host-observed loss.
    let loss = renderer.note_device_lost("driver reset");
    assert!(loss.is_device_loss());
    assert!(renderer.is_device_lost());
    assert_eq!(renderer.batch_count(), 0);
    let target = RenderTarget::new(64, 64);
    let frame = renderer.render(Camera2d::default(), &target);
    assert!(matches!(frame, Err(RenderError::DeviceLost(_))));
}

#[test]
fn over_budget_diagnostic_carries_structured_parameters() {
    let report = OverBudget {
        category: "vertices",
        requested: 100,
        limit: 50,
        skipped_batches: 3,
    };
    let reason = Renderer::over_budget_reason(&report);
    assert_eq!(reason.code, codes::RENDER_FRAME_OVER_BUDGET);
    assert!(reason.parameters.contains(&DiagnosticParameter::Limit(50)));
}

#[test]
fn device_lost_diagnostic_has_stable_code() {
    let reason = Renderer::device_lost_reason("test");
    assert_eq!(reason.code, codes::RENDER_DEVICE_LOST);
}

#[test]
fn draw_passes_remap_accepted_positions_to_global_indices() {
    // The budget accepted global batches 10, 11, 12 (upload order). Batch 10
    // is transparent and farthest, 11 opaque with the higher draw order, and
    // 12 opaque with a lower draw order.
    let accepted = vec![10usize, 11, 12];
    let entries = vec![
        BatchOrderEntry {
            draw_order: 0,
            alpha: 0.5,
            centroid: [100.0, 0.0, 0.0],
        },
        BatchOrderEntry {
            draw_order: 5,
            alpha: 1.0,
            centroid: [1.0, 0.0, 0.0],
        },
        BatchOrderEntry {
            draw_order: 1,
            alpha: 1.0,
            centroid: [2.0, 0.0, 0.0],
        },
    ];
    let passes = draw_passes(&accepted, &entries, Some([0.0, 0.0, 0.0]));
    // Opaque first, ascending draw order: global 12 (order 1), then 11 (5).
    assert_eq!(passes.opaque, vec![12, 11]);
    // Transparent after, single element global 10.
    assert_eq!(passes.transparent, vec![10]);
    assert_eq!(passes.invisible, 0);
}

#[test]
fn draw_passes_report_invisible_batches() {
    let accepted = vec![0usize, 1];
    let entries = vec![
        BatchOrderEntry {
            draw_order: 0,
            alpha: 0.0,
            centroid: [0.0; 3],
        },
        BatchOrderEntry {
            draw_order: 0,
            alpha: 1.0,
            centroid: [0.0; 3],
        },
    ];
    let passes = draw_passes(&accepted, &entries, Some([0.0; 3]));
    assert_eq!(passes.opaque, vec![1]);
    assert!(passes.transparent.is_empty());
    assert_eq!(passes.invisible, 1);
}

#[test]
fn image_quad_maps_the_unit_square_through_the_transform() {
    let batch = ImageBatch {
        resource: ResourceKey("texture".into()),
        transform: Transform3::translation(Point3 {
            x: 2.0,
            y: 3.0,
            z: 0.0,
        }),
        clip: None,
        alpha: 1.0,
        draw_order: 0,
        sources: Vec::new(),
    };
    let (vertices, indices) = image_quad_vertices(&batch);
    // Four vertices of 20 bytes (position vec3 + uv vec2), two triangles.
    assert_eq!(vertices.len(), 4 * 20);
    assert_eq!(indices, vec![0, 1, 2, 0, 2, 3]);
    let read = |i: usize| {
        let mut out = [0.0f32; 5];
        for (j, slot) in out.iter_mut().enumerate() {
            let start = i * 20 + j * 4;
            *slot = f32::from_le_bytes(vertices[start..start + 4].try_into().unwrap());
        }
        out
    };
    // The CAD origin is the lower-left with v pointing up, while decoded image
    // rows are top-down, so `v` is flipped: corner (0,0) gets UV (0,1) and
    // corner (1,1) gets UV (1,0).
    assert_eq!(read(0), [2.0, 3.0, 0.0, 0.0, 1.0]);
    assert_eq!(read(2), [3.0, 4.0, 0.0, 1.0, 0.0]);
}

#[test]
fn image_quad_uses_the_clip_polygon_instead_of_the_unit_square() {
    use cad_representation::ImageVertex;
    use std::sync::Arc;
    // A world-space left half of the unit square with matching UVs; the batch
    // transform is a large translation that must NOT be applied to the clip.
    let clip: Arc<[ImageVertex]> = Arc::from(
        vec![
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.5, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.5, 1.0],
            },
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.0, 1.0],
            },
        ]
        .into_boxed_slice(),
    );
    let batch = ImageBatch {
        resource: ResourceKey("texture".into()),
        transform: Transform3::translation(Point3 {
            x: 100.0,
            y: 100.0,
            z: 0.0,
        }),
        clip: Some(clip),
        alpha: 1.0,
        draw_order: 0,
        sources: Vec::new(),
    };
    let (vertices, indices) = image_quad_vertices(&batch);
    // Four clip vertices, fan-triangulated (0,1,2),(0,2,3).
    assert_eq!(vertices.len(), 4 * 20);
    assert_eq!(indices, vec![0, 1, 2, 0, 2, 3]);
    let read = |i: usize| {
        let mut out = [0.0f32; 5];
        for (j, slot) in out.iter_mut().enumerate() {
            let start = i * 20 + j * 4;
            *slot = f32::from_le_bytes(vertices[start..start + 4].try_into().unwrap());
        }
        out
    };
    // Clip positions are used verbatim (no transform), and `v` is flipped.
    assert_eq!(read(0), [-0.8, -0.8, 0.0, 0.0, 1.0]);
    assert_eq!(read(3), [-0.8, 0.8, 0.0, 0.0, 0.0]);
}

#[test]
fn cad_clear_color_is_pure_black_opaque() {
    // The React host requires the CAD render target to clear to actual pure
    // black (not a DOM overlay); this is the global default for every space.
    let color = CAD_CLEAR_COLOR;
    assert_eq!(
        (color.r, color.g, color.b, color.a),
        (0.0, 0.0, 0.0, 1.0),
        "the CAD render target clears to pure opaque black"
    );
}

#[test]
fn uniform_page_stays_within_the_default_binding_limit() {
    // WebGPU defaults: 64 KiB uniform binding limit, 256-byte offset alignment.
    let limits = wgpu::Limits::default();
    let (slot_size, slots) = uniform_layout_params(&limits);
    assert_eq!(
        slot_size, 256,
        "the struct is padded to the offset alignment"
    );
    assert_eq!(slots, 256, "64 KiB / 256 B = 256 slots per page");
    assert!(
        slots as u64 * u64::from(slot_size) <= limits.max_uniform_buffer_binding_size,
        "a whole page must remain a legal uniform binding"
    );
}

#[test]
fn uniform_slot_size_follows_the_offset_alignment() {
    // A backend with a larger offset alignment must still get aligned slots.
    let limits = wgpu::Limits {
        min_uniform_buffer_offset_alignment: 512,
        ..Default::default()
    };
    let (slot_size, _) = uniform_layout_params(&limits);
    assert_eq!(slot_size, 512);
    assert_eq!(slot_size % limits.min_uniform_buffer_offset_alignment, 0);
    // An adapter reporting a very large binding size is still capped.
    let huge = wgpu::Limits {
        max_uniform_buffer_binding_size: u64::MAX,
        ..Default::default()
    };
    let (_, slots) = uniform_layout_params(&huge);
    assert_eq!(slots, MAX_UNIFORM_SLOTS_PER_PAGE);
}

#[test]
fn slot_location_picks_page_and_offset_across_buffers() {
    // 3 slots per page at a 256-byte stride.
    assert_eq!(slot_location(0, 3, 256), (0, 0));
    assert_eq!(slot_location(2, 3, 256), (0, 512));
    // Slot 3 is the first slot of the *second* shared buffer.
    assert_eq!(slot_location(3, 3, 256), (1, 0));
    assert_eq!(slot_location(259, 256, 256), (1, 3 * 256));
}

#[test]
fn the_id_target_probe_prefers_r32uint_when_the_spec_guarantees_it_renderable() {
    // The WebGPU spec's guaranteed format features list `R32Uint` as a render
    // attachment, so a device that enabled no optional feature still gets the
    // single-channel id target. A device that did not would take the packed
    // fallback, which the headless tests exercise through the override hook.
    assert_eq!(
        Renderer::id_format_for_features(wgpu::Features::empty()),
        IdTargetFormat::R32Uint
    );
    assert_eq!(
        IdTargetFormat::R32Uint.texture_format(),
        wgpu::TextureFormat::R32Uint
    );
    assert_eq!(
        IdTargetFormat::PackedRgba8.texture_format(),
        wgpu::TextureFormat::Rgba8Unorm
    );
    assert_eq!(IdTargetFormat::R32Uint.as_str(), "r32uint");
    assert_eq!(IdTargetFormat::PackedRgba8.as_str(), "packed-rgba8");
    // A renderer with no device has no id target format.
    assert!(Renderer::default().id_target_format().is_none());
}

#[test]
fn the_uniform_struct_grew_for_the_pick_key_without_moving_the_slot_stride() {
    // `mat4x4` (64) + tint `vec4<f32>` (16) + key_flags `vec4<u32>` (16) = 96.
    assert_eq!(UNIFORM_STRUCT_BYTES, 96);
    // The dynamic-offset slot stride stays at the 256-byte alignment, so the
    // per-page slot count is unchanged by the extra field.
    let (slot_size, slots) = uniform_layout_params(&wgpu::Limits::default());
    assert_eq!(slot_size, 256);
    assert_eq!(slots, 256);
    let uniform = uniform_bytes(
        &[1.0; 16],
        [0.25, 0.5, 0.75, 1.0],
        [7, INSTANCE_FLAG_OPAQUE, 0, 0],
    );
    assert_eq!(uniform.len(), UNIFORM_STRUCT_BYTES as usize);
    // Word 16..20 = tint, 20..24 = key/flags (little-endian words).
    assert_eq!(
        f32::from_le_bytes(uniform[64..68].try_into().unwrap()),
        0.25
    );
    assert_eq!(u32::from_le_bytes(uniform[80..84].try_into().unwrap()), 7);
    assert_eq!(
        u32::from_le_bytes(uniform[84..88].try_into().unwrap()),
        INSTANCE_FLAG_OPAQUE
    );
}

/// Numeric proof of the SDF conic **ramp-width** semantics — the root cause of
/// the reported "arc stroke is √2 too wide".
///
/// `shaders/arc.wgsl` shades `coverage = saturate(1 - d/w)` where
/// `d = length(vec2(dot(q, u), dot(q, normalize(v)))) - r` is evaluated in the
/// conic's packed `u`/`v` frame and `w = world_per_px` (one screen pixel in
/// world units, the `key_flags.z` uniform word). Two independent facts pin the
/// ink to exactly one pixel, so no scale factor (√2 or otherwise) can hide:
///
/// 1. the **packed frame is orthonormal** — `u` is unit, `normalize(v)` is unit
///    and `u ⟂ normalize(v)` — so `q -> (x, y)` is an isometry and `d` is a
///    **unit-gradient** distance field: one world unit of `d` is one world unit
///    on screen. (A non-unit `u`, or a `v` that is not perpendicular, would
///    make the gradient differ from 1 — the only way the on-screen ramp width
///    could disagree with `w`.)
/// 2. the triangle filter integrates to exactly `w`: `∫ saturate(1 - |d|/w) dd
///    = w`. With `w = world_per_px` the ink is therefore exactly
///    `w / world_per_px = 1.0 px`.
///
/// Integrating the **sRGB-encoded** channel instead of linear coverage is what
/// makes a soft triangle stroke *look* √2 wide in a raw pixel-value integral
/// (up to `1.47` vs a hard 1 px line's `1.00`); the render itself is correct.
/// `tests/sdf_arcs.rs` fences the linear-coverage ink on the real GPU.
#[test]
fn sdf_conic_field_is_unit_gradient_and_triangle_ink_is_one_pixel() {
    let r = 7.0f64;
    let center = Point3 {
        x: 100_000.0,
        y: -250_000.0,
        z: 0.0,
    };
    let conic = cad_scene::RenderConic {
        center,
        major: Point3 {
            x: r,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: r,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    };
    let batch = cad_scene::RenderBatch {
        local_origin: center,
        topology: cad_scene::RenderTopology::Conics,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics: vec![conic],
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: Vec::new(),
        draw_order: 0,
    };
    let mut instances = Vec::new();
    assert!(crate::plan::push_arc_instance(
        &mut instances,
        &batch,
        0,
        [center.x, center.y, center.z],
        1,
    ));
    assert_eq!(instances.len(), 1);
    let inst = instances[0];

    // (1) The packed frame is orthonormal.
    let u = [inst.uvec[0], inst.uvec[1], inst.uvec[2]];
    let v = [inst.vvec[0], inst.vvec[1], inst.vvec[2]];
    let u_len = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    let v_len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    assert!(
        (u_len - 1.0).abs() < 1e-6,
        "the major direction must be unit, got |u|={u_len}"
    );
    assert!(
        (v_len as f64 - r).abs() < 1e-3,
        "the minor is carried as its full semi-diameter, got |v|={v_len} vs r={r}"
    );
    let vhat = [v[0] / v_len, v[1] / v_len, v[2] / v_len];
    let uv = u[0] * vhat[0] + u[1] * vhat[1] + u[2] * vhat[2];
    assert!(
        uv.abs() < 1e-6,
        "the u/v frame must be orthogonal for a circle, got u·v̂={uv}"
    );

    // (2) The exact shader field is a unit-gradient distance field.
    let c = [
        inst.center_r[0] as f64,
        inst.center_r[1] as f64,
        inst.center_r[2] as f64,
    ];
    let rr = inst.center_r[3] as f64;
    let field = |p: [f64; 3]| -> f64 {
        let q = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        let x = q[0] * u[0] as f64 + q[1] * u[1] as f64 + q[2] * u[2] as f64;
        let y = q[0] * vhat[0] as f64 + q[1] * vhat[1] as f64 + q[2] * vhat[2] as f64;
        (x * x + y * y).sqrt() - rr
    };
    let h = 1e-4;
    for k in 0..64 {
        let theta = k as f64 / 64.0 * std::f64::consts::TAU;
        let radius = rr * 1.7;
        let p = [
            c[0] + radius * theta.cos(),
            c[1] + radius * theta.sin(),
            c[2],
        ];
        let gx = (field([p[0] + h, p[1], p[2]]) - field([p[0] - h, p[1], p[2]])) / (2.0 * h);
        let gy = (field([p[0], p[1] + h, p[2]]) - field([p[0], p[1] - h, p[2]])) / (2.0 * h);
        let g = (gx * gx + gy * gy).sqrt();
        assert!(
            (g - 1.0).abs() < 1e-6,
            "gradient magnitude at theta={theta:.4} is {g}; the field must be a true \
             distance field (unit gradient) or the on-screen ramp width would differ from w"
        );
    }

    // (3) The triangle filter's ink is exactly `w` world units, i.e. exactly
    // one pixel for `w = world_per_px`.
    for &world_per_px in &[0.0078125f64, 0.2, 0.5, 3.0] {
        let w = world_per_px;
        let steps = 100_000usize;
        let span = 2.0 * w;
        let dd = span / steps as f64;
        let mut ink = 0.0;
        for i in 0..steps {
            let d = -w + (i as f64 + 0.5) * dd;
            ink += (1.0 - d.abs() / w).max(0.0) * dd;
        }
        assert!(
            (ink - w).abs() < 1e-9 * w.max(1.0),
            "triangle-filter ink is {ink} world units, expected exactly w={w}"
        );
        let ink_px = ink / world_per_px;
        assert!(
            (ink_px - 1.0).abs() < 1e-9,
            "triangle-filter ink is {ink_px} px, expected exactly 1.0"
        );
    }
}
