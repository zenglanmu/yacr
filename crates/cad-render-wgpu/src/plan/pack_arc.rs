use super::*;

/// Byte stride of one [`ArcInstance`]: four `vec4<f32>` (`center_r`, `uvec`,
/// `vvec`, `rgba`) plus one `vec2<u32>` (`id_flags`).
pub(crate) const ARC_INSTANCE_STRIDE: u32 = 72;

// The scene's public `CONIC_INSTANCE_BYTES` and this stride must never drift.
const _: () = assert!(ARC_INSTANCE_STRIDE == cad_scene::CONIC_INSTANCE_BYTES);

/// Reused instance-flag bit: this conic is a closed full circle. Lives in the
/// second component of [`ArcInstance::id_flags`], alongside the
/// [`INSTANCE_FLAG_*`] bits.
pub(crate) const ARC_FLAG_FULL_CIRCLE: u32 = 1 << 3;

/// One SDF conic instance (a slice-1 circle/arc).
///
/// `#[repr(C)]` + `Pod` so the packed bytes are exactly the GPU layout; a unit
/// test fences the stride against `size_of`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ArcInstance {
    /// `xyz` = center in the stream's fixed-point-relative world space,
    /// `w` = the major radius (a slice-1 circle's radius).
    pub(crate) center_r: [f32; 4],
    /// `xyz` = unit major direction, `w` = start angle in `[0, τ)`.
    pub(crate) uvec: [f32; 4],
    /// `xyz` = minor semi-diameter vector (conjugate), `w` = signed sweep.
    pub(crate) vvec: [f32; 4],
    /// Sanitised linear sRGB colour (`rgb`) and clamped alpha (`a`).
    pub(crate) rgba: [f32; 4],
    /// `[0]` = renderer pick key, `[1]` = `INSTANCE_FLAG_*` |
    /// [`ARC_FLAG_FULL_CIRCLE`].
    pub(crate) id_flags: [u32; 2],
}

/// The exact packed bytes of `instances` (one contiguous GPU buffer payload).
pub(crate) fn arc_instance_bytes(instances: &[ArcInstance]) -> Vec<u8> {
    bytemuck::cast_slice(instances).to_vec()
}

/// Per-instance vertex-buffer layout (slot 1 of the SDF conic pipeline).
///
/// `shader_location` 1..=5 so the shared unit quad keeps `@location(0)`.
pub(crate) fn arc_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Uint32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(ARC_INSTANCE_STRIDE),
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// Whether a scene batch's conics are converted to the SDF instance path.
///
/// Only an opaque `Conics` batch with at least one conic is converted. A
/// **transparent** conic batch has no supported draw path (the SDF conic
/// pipeline is opaque-only and `draw_batch` has no conic arm), so it is
/// **rejected at upload validation** rather than silently dropped. An invisible
/// (`alpha <= 0`) conic is reported as invisible and never drawn.
pub(crate) fn arc_batch_is_instanced(batch: &cad_scene::RenderBatch) -> bool {
    batch.topology == RenderTopology::Conics
        && !batch.conics.is_empty()
        && classify_alpha(batch.alpha) == AlphaClass::Opaque
}

/// One batch converted to the SDF conic path.
pub(crate) struct ConvertedArc {
    /// Index into the full batch slice.
    pub(crate) batch: usize,
}

/// Build the SDF conic conversion list for a batch set.
pub(crate) fn convert_arc_batches(batches: &[cad_scene::RenderBatch]) -> Vec<ConvertedArc> {
    batches
        .iter()
        .enumerate()
        .filter(|(_, batch)| arc_batch_is_instanced(batch))
        .map(|(index, _)| ConvertedArc { batch: index })
        .collect()
}

/// One memory- and precision-bounded static SDF conic stream, ready to upload.
pub(crate) struct ArcChunk {
    /// The stream's fixed-point reference.
    pub(crate) origin: [f64; 3],
    /// Packed instances, in original batch order.
    pub(crate) instances: Vec<ArcInstance>,
    /// World-space AABB of the chunk's conic full extents, retained from the
    /// builder for per-frame view culling (see [`cull`]). An oversized source
    /// split across several chunks carries the whole-batch AABB in every piece
    /// (conservative).
    pub(crate) aabb: WorldAabb,
}

/// One conic's contribution to a chunk: its batch, index within the batch, and
/// the world-space AABB of its full extent (used for the radius split).
pub(super) struct ArcItem {
    batch: usize,
    conic: usize,
    min: [f64; 3],
    max: [f64; 3],
    batch_origin: [f64; 3],
}

pub(super) struct ArcChunkBuilder {
    item_start: usize,
    count: usize,
    shared_origin: Option<[f64; 3]>,
    min: [f64; 3],
    max: [f64; 3],
}

impl ArcChunkBuilder {
    pub(super) fn accepts(&self, item: &ArcItem, cap: usize, max_radius: f64) -> bool {
        if self.count + 1 > cap {
            return false;
        }
        if self.shared_origin == Some(item.batch_origin) {
            return true;
        }
        let union_min = [
            self.min[0].min(item.min[0]),
            self.min[1].min(item.min[1]),
            self.min[2].min(item.min[2]),
        ];
        let union_max = [
            self.max[0].max(item.max[0]),
            self.max[1].max(item.max[1]),
            self.max[2].max(item.max[2]),
        ];
        aabb_half_diagonal(union_min, union_max) <= max_radius
    }

    pub(super) fn push(&mut self, item: &ArcItem) {
        if self.shared_origin != Some(item.batch_origin) {
            self.shared_origin = None;
        }
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(item.min[axis]);
            self.max[axis] = self.max[axis].max(item.max[axis]);
        }
        self.count += 1;
    }
}

/// Partition the converted conic sources into instance-stream chunks and pack
/// them.
///
/// Mirrors [`build_instance_chunks`] with the conic's full-extent AABB: chunks
/// are built in converted order (original batch order), each conic joins the
/// current chunk only while that keeps it within `max_instances` and — for a
/// mixed-origin chunk — within `max_radius` of its AABB midpoint, otherwise it
/// starts a new chunk. An oversized batch is split across several chunks (one run
/// per piece). No empty chunk is ever produced.
///
/// Returns the chunks, per input batch its run list (`None` for a batch that was
/// not converted), and the number of conics skipped as degenerate at pack (a
/// defensive guard the provider already excludes; counted, never silent).
/// `keys[i]` is `batches[i]`'s renderer pick key.
pub(crate) fn build_arc_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[ConvertedArc],
    keys: &[u32],
    max_instances: usize,
    max_radius: f64,
) -> (Vec<ArcChunk>, Vec<Option<Vec<ArcRun>>>, u64) {
    debug_assert_eq!(batches.len(), keys.len());
    let cap = max_instances.max(1);
    let mut items: Vec<ArcItem> = Vec::new();
    for source in converted {
        let batch = &batches[source.batch];
        let batch_origin = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        for (conic_index, conic) in batch.conics.iter().enumerate() {
            let Some((min, max)) = conic.world_aabb() else {
                continue;
            };
            items.push(ArcItem {
                batch: source.batch,
                conic: conic_index,
                min,
                max,
                batch_origin,
            });
        }
    }

    let mut builders: Vec<ArcChunkBuilder> = Vec::new();
    let mut current: Option<ArcChunkBuilder> = None;
    for (pos, item) in items.iter().enumerate() {
        if let Some(chunk) = current.as_mut() {
            if chunk.accepts(item, cap, max_radius) {
                chunk.push(item);
                continue;
            }
        }
        if let Some(chunk) = current.take() {
            builders.push(chunk);
        }
        current = Some(ArcChunkBuilder {
            item_start: pos,
            count: 1,
            shared_origin: Some(item.batch_origin),
            min: item.min,
            max: item.max,
        });
    }
    if let Some(chunk) = current.take() {
        builders.push(chunk);
    }

    let mut runs: Vec<Option<Vec<ArcRun>>> = vec![None; batches.len()];
    let mut chunks = Vec::with_capacity(builders.len());
    let mut degenerate_skips = 0u64;
    for (stream, builder) in builders.iter().enumerate() {
        let origin = match builder.shared_origin {
            Some(origin) => origin,
            None => [
                (builder.min[0] + builder.max[0]) * 0.5,
                (builder.min[1] + builder.max[1]) * 0.5,
                (builder.min[2] + builder.max[2]) * 0.5,
            ],
        };
        let mut instances = Vec::with_capacity(builder.count);
        let mut previous_batch: Option<usize> = None;
        for item in &items[builder.item_start..builder.item_start + builder.count] {
            let offset = instances.len() as u32;
            let pushed_ok = push_arc_instance(
                &mut instances,
                &batches[item.batch],
                item.conic,
                origin,
                keys[item.batch],
            );
            if !pushed_ok {
                degenerate_skips += 1;
            }
            let pushed = instances.len() as u32 - offset;
            if pushed == 0 {
                continue;
            }
            if previous_batch == Some(item.batch) {
                if let Some(run) = runs[item.batch].as_mut().and_then(|list| list.last_mut()) {
                    if run.stream == stream as u32 && run.offset + run.count == offset {
                        run.count += pushed;
                        previous_batch = Some(item.batch);
                        continue;
                    }
                }
            }
            runs[item.batch].get_or_insert_with(Vec::new).push(ArcRun {
                stream: stream as u32,
                offset,
                count: pushed,
            });
            previous_batch = Some(item.batch);
        }
        chunks.push(ArcChunk {
            origin,
            instances,
            aabb: WorldAabb {
                min: builder.min,
                max: builder.max,
            },
        });
    }
    (chunks, runs, degenerate_skips)
}

/// Normalize an angle into `[0, τ)`.
pub(super) fn normalize_tau(angle: f64) -> f64 {
    let t = angle.rem_euclid(std::f64::consts::TAU);
    if t >= std::f64::consts::TAU {
        0.0
    } else {
        t
    }
}

/// Append one [`ArcInstance`] for `batch.conics[conic_index]`.
///
/// The center is stored relative to the chunk `origin` (reduced in `f64` before
/// narrowing); the major direction is a unit vector and the minor is carried as
/// its full semi-diameter vector, so the parameterization
/// `p(t) = center + (r·u)·cos t + v·sin t` is exact. `pick_key` is the batch's
/// renderer pick key. Returns `false` (pushing nothing) for a degenerate conic
/// (`sweep == 0 && !full`, or a zero-length major axis); the caller counts the
/// skip so it is never silent, though the scene/provider already exclude it.
pub(crate) fn push_arc_instance(
    out: &mut Vec<ArcInstance>,
    batch: &cad_scene::RenderBatch,
    conic_index: usize,
    origin: [f64; 3],
    pick_key: u32,
) -> bool {
    let Some(conic) = batch.conics.get(conic_index) else {
        return false;
    };
    if !conic.full && conic.sweep == 0.0 {
        return false;
    }
    let major_len = (conic.major.x * conic.major.x
        + conic.major.y * conic.major.y
        + conic.major.z * conic.major.z)
        .sqrt();
    if !(major_len.is_finite() && major_len > 0.0) {
        return false;
    }
    let u = [
        conic.major.x / major_len,
        conic.major.y / major_len,
        conic.major.z / major_len,
    ];
    let center = [
        (conic.center.x - origin[0]) as f32,
        (conic.center.y - origin[1]) as f32,
        (conic.center.z - origin[2]) as f32,
    ];
    let sweep = if conic.full {
        std::f64::consts::TAU
    } else {
        conic
            .sweep
            .clamp(-std::f64::consts::TAU, std::f64::consts::TAU)
    };
    let full = conic.full || sweep.abs() >= std::f64::consts::TAU - 1e-9;
    let mut flags = batch_instance_flags(batch);
    if full {
        flags |= ARC_FLAG_FULL_CIRCLE;
    }
    let [r, g, b] = renderer_color(batch.color);
    out.push(ArcInstance {
        center_r: [center[0], center[1], center[2], major_len as f32],
        uvec: [
            u[0] as f32,
            u[1] as f32,
            u[2] as f32,
            normalize_tau(conic.start) as f32,
        ],
        vvec: [
            conic.minor.x as f32,
            conic.minor.y as f32,
            conic.minor.z as f32,
            sweep as f32,
        ],
        rgba: [r, g, b, clamp_alpha(batch.alpha)],
        id_flags: [pick_key, flags],
    });
    true
}

// ---------------------------------------------------------------------------
// Accelerated tier: vertex pulling (capability-gated)
// ---------------------------------------------------------------------------
//
// The universal path coalesces draws per *chunk* (a per-chunk `f64` origin
// bounds every stored coordinate to `MAX_STREAM_RADIUS`, so a stream buffer is
// never oversized and the f32 ulp stays under ~1 µm). That still leaves one
// draw per contiguous chunk run. The accelerated tier collapses the converted
// opaque classes to **one draw per class** by concatenating every chunk into a
// single storage buffer and pulling each primitive with
// `@builtin(instance_index)` / `@builtin(vertex_index)`.
//
// What is *not* relaxed: the per-chunk `f64` reduction still happens on the CPU
// at upload (stored coordinate = `(world - origin) as f32`), and the per-chunk
// translation column is still `FrameProjection::batch_matrix_f64(origin)`'s last
// column. The shader only adds a radius-bounded `f32` position to that column,
// so the precision contract is byte-identical to the universal path.
//
// The tier requires a read-only storage buffer in the vertex stage, which
// WebGL2 does not provide. It is chosen once per device in
// `Renderer::initialize_with_device` from the device's own limits/backend and
// never requests a feature from a host-provided device.
