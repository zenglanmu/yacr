use super::*;

#[test]
fn pulled_strides_match_their_wgsl_vec4_layouts() {
    // The WGSL structs are four `vec4`s (16 B each); the CPU structs must be
    // exactly that stride or the storage array indexing is wrong.
    assert_eq!(
        std::mem::size_of::<PulledLine>(),
        PULLED_LINE_STRIDE as usize
    );
    assert_eq!(
        std::mem::size_of::<PulledMeshVertex>(),
        PULLED_MESH_VERTEX_STRIDE as usize
    );
    assert_eq!(PULLED_CAMERA_BYTES, 64);
    // The layout is alignment-clean for the storage array stride.
    assert_eq!(std::mem::align_of::<PulledLine>(), 4);
    assert_eq!(std::mem::align_of::<PulledMeshVertex>(), 4);
}

#[test]
fn tier_gate_requires_storage_buffers_and_a_usable_binding() {
    let limits = wgpu::Limits::default();
    // A capable device (storage buffers, defaults) selects accelerated.
    assert_eq!(
        tier_for(true, &limits, None),
        RenderTier::Accelerated,
        "a storage-buffer device must select the accelerated tier"
    );
    // WebGL2 (no storage buffers) downgrades silently.
    assert_eq!(
        tier_for(false, &limits, None),
        RenderTier::Universal,
        "a device without storage buffers must downgrade"
    );
    // The test hook can strip support on a capable device.
    assert_eq!(tier_for(true, &limits, Some(false)), RenderTier::Universal);
    // ...and force it on a device that reports none, so the tier is testable
    // on any adapter.
    assert_eq!(
        tier_for(false, &limits, Some(true)),
        RenderTier::Accelerated
    );
}

#[test]
fn tier_gate_rejects_zero_storage_buffers_and_tiny_bindings() {
    // The gate must require the *true* number of vertex-stage storage
    // bindings (three: `mesh_pulled.wgsl` binds vertices, indices and the
    // chunk table), not one. A device reporting 0, 1 or 2 must downgrade
    // rather than select a tier whose pipelines it cannot create.
    for reported in 0..MIN_PULLED_STORAGE_BINDINGS {
        let limits = wgpu::Limits {
            max_storage_buffers_per_shader_stage: reported,
            ..wgpu::Limits::default()
        };
        assert_eq!(
            tier_for(true, &limits, None),
            RenderTier::Universal,
            "{reported} storage bindings must downgrade"
        );
    }
    let limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: MIN_PULLED_STORAGE_BINDINGS,
        ..wgpu::Limits::default()
    };
    assert_eq!(
        tier_for(true, &limits, None),
        RenderTier::Accelerated,
        "exactly the required binding count must still select accelerated"
    );
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: MIN_PULLED_BINDING_BYTES - 1,
        ..wgpu::Limits::default()
    };
    assert_eq!(tier_for(true, &limits, None), RenderTier::Universal);
}

#[test]
fn finalize_tier_downgrades_when_pulled_resources_are_missing() {
    // A device that selected accelerated but could not build its pulled
    // pipelines (a missing flags layout) must downgrade rather than stay
    // labelled Accelerated with a missing pulled path.
    assert_eq!(
        finalize_tier(RenderTier::Accelerated, true),
        RenderTier::Accelerated
    );
    assert_eq!(
        finalize_tier(RenderTier::Accelerated, false),
        RenderTier::Universal
    );
    assert_eq!(
        finalize_tier(RenderTier::Universal, true),
        RenderTier::Universal
    );
    assert_eq!(
        finalize_tier(RenderTier::Universal, false),
        RenderTier::Universal
    );
}

fn instance(p0: [f32; 3], p1: [f32; 3], key: u32, flags: u32) -> LineInstance {
    LineInstance {
        p0_width: [p0[0], p0[1], p0[2], 0.0],
        p1_pad: [p1[0], p1[1], p1[2], 0.0],
        rgba: [0.25, 0.5, 0.75, 1.0],
        id_flags: [key, flags],
    }
}

#[test]
fn pulled_lines_concatenate_streams_in_order_and_stamp_the_chunk() {
    let s0 = vec![
        instance([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, 5),
        instance([0.0, 1.0, 0.0], [1.0, 1.0, 0.0], 2, 5),
    ];
    let s1 = vec![instance([2.0, 0.0, 0.0], [3.0, 0.0, 0.0], 3, 1)];
    let streams: Vec<&[LineInstance]> = vec![&s0, &s1];
    let pulled = build_pulled_lines(&streams, 0);
    assert_eq!(pulled.len(), 3);
    // Stream order preserved, each record stamped with its stream index.
    assert_eq!(pulled[0].meta, [1, 5, 0, 0]);
    assert_eq!(pulled[1].meta, [2, 5, 0, 0]);
    assert_eq!(pulled[2].meta, [3, 1, 1, 0]);
    // Geometry fields are carried verbatim.
    assert_eq!(pulled[2].p0_width, [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(pulled[2].p1_pad, [3.0, 0.0, 0.0, 0.0]);
    assert_eq!(pulled[2].rgba, [0.25, 0.5, 0.75, 1.0]);
}

fn mesh_vertex(position: [f32; 3], key: u32) -> MeshVertex {
    MeshVertex {
        position,
        normal: [0.0, 0.0, 1.0],
        color: [0.5, 0.5, 0.5],
        entity_flags: [key, 0],
    }
}

#[test]
fn pulled_mesh_vertices_are_stamped_with_their_chunk() {
    let c0 = vec![
        mesh_vertex([0.0, 0.0, 0.0], 1),
        mesh_vertex([1.0, 0.0, 0.0], 1),
    ];
    let c1 = vec![mesh_vertex([2.0, 0.0, 0.0], 2)];
    let chunks: Vec<&[MeshVertex]> = vec![&c0, &c1];
    let pulled = build_pulled_mesh_vertices(&chunks, 0);
    assert_eq!(pulled.len(), 3);
    assert_eq!(pulled[0].meta, [1, 0, 0, 0]);
    assert_eq!(pulled[1].meta, [1, 0, 0, 0]);
    assert_eq!(pulled[2].meta, [2, 0, 1, 0]);
    assert_eq!(pulled[2].position, [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(pulled[2].color, [0.5, 0.5, 0.5, 0.0]);
}

#[test]
fn pulled_mesh_indices_rebase_and_split_mirrored_chunks_last() {
    // Chunk 0: 4 vertices, non-mirrored. Chunk 1: 3 vertices, mirrored.
    let i0: Vec<u32> = vec![0, 2, 1, 0, 3, 2];
    let i1: Vec<u32> = vec![0, 1, 2];
    let chunks: Vec<(&[u32], u32, bool)> =
        vec![(i0.as_slice(), 4, false), (i1.as_slice(), 3, true)];
    let (indices, mirror_split) = build_pulled_mesh_indices_with_bases(&chunks);
    // Non-mirrored first (unchanged, base 0), then mirrored rebased by +4.
    assert_eq!(indices, vec![0, 2, 1, 0, 3, 2, 4, 5, 6]);
    assert_eq!(mirror_split, 6);
}

#[test]
fn pulled_mesh_indices_with_no_mirrored_chunks_have_a_full_split() {
    let i0: Vec<u32> = vec![0, 1, 2];
    let chunks: Vec<(&[u32], u32, bool)> = vec![(i0.as_slice(), 3, false)];
    let (indices, mirror_split) = build_pulled_mesh_indices_with_bases(&chunks);
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(mirror_split, 3);
}

#[test]
fn pulled_camera_columns_are_origin_independent_and_match_the_batch_matrix() {
    let projection = FrameProjection::Ortho {
        sx: 0.03125,
        sy: 0.03125,
        center: [1.0e6, 2.0e6, 0.0],
        z_plane: 0.0,
        depth: OrthoDepth::default(),
        world_per_px: 0.0,
    };
    let columns = pulled_camera_columns(projection);
    // The first three columns of a per-chunk matrix must not depend on the
    // chunk origin: one camera uniform serves every chunk.
    let m = projection.batch_matrix_f64([1.0e6, 2.0e6, 0.0]);
    assert_eq!(columns[0], [m[0], m[1], m[2], m[3]]);
    assert_eq!(columns[1], [m[4], m[5], m[6], m[7]]);
    assert_eq!(columns[2], [m[8], m[9], m[10], m[11]]);
    // The fourth column carries the world units per pixel for the AA line SDF
    // in its x (zero here, which selects the `fwidth` fallback).
    assert_eq!(columns[3], [0.0, 0.0, 0.0, 0.0]);
    // And the chunk column is exactly the per-chunk matrix's last column, so
    // `mat4x4(c0, c1, c2, t)` reproduces `batch_matrix_f64(origin)`.
    let origin = [1.0e6 + 123.5, 2.0e6 - 77.25, 0.0];
    let t = chunk_transform_column(projection, origin);
    let mo = projection.batch_matrix_f64(origin);
    assert_eq!(t, [mo[12], mo[13], mo[14], mo[15]]);
}

#[test]
fn chunk_transform_column_keeps_the_f64_camera_reduction() {
    // A large drawing coordinate: the translation column must be the f64
    // reduction `sx * (origin - center)`, not a large-magnitude f32 origin.
    let projection = FrameProjection::Ortho {
        sx: 0.001,
        sy: 0.001,
        center: [1.0e7, 0.0, 0.0],
        z_plane: 0.0,
        depth: OrthoDepth::default(),
        world_per_px: 0.0,
    };
    let origin = [1.0e7 + 1000.0, 0.0, 0.0];
    let t = chunk_transform_column(projection, origin);
    assert!((t[0] - 1.0).abs() < 1e-4, "expected ~1.0, got {t:?}");
}

#[test]
fn pulled_camera_bytes_are_four_columns_little_endian() {
    let columns = [
        [1.0f32, 2.0, 3.0, 4.0],
        [5.0, 6.0, 7.0, 8.0],
        [9.0, 10.0, 11.0, 12.0],
        [13.0, 14.0, 15.0, 16.0],
    ];
    let bytes = pulled_camera_bytes(&columns);
    assert_eq!(bytes.len(), PULLED_CAMERA_BYTES as usize);
    for (i, value) in [
        1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
    ]
    .iter()
    .enumerate()
    {
        let at = i * 4;
        assert_eq!(
            &bytes[at..at + 4],
            &value.to_le_bytes(),
            "column word {i} must be little-endian"
        );
    }
}
