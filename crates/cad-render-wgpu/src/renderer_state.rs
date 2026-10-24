use super::*;

/// The GPU objects a measurement-only timestamp bracket needs, created once and
/// reused across frames (no per-frame allocation of the measurement resources).
///
/// Created only by [`Renderer::set_gpu_time_measurement`] when the opt-in is
/// enabled and the device granted the feature set; otherwise no such resources
/// exist. The readback buffer is `MAP_READ`; it is unmapped before every frame so
/// the next frame's `copy_buffer_to_buffer` is valid.
pub(crate) struct GpuTimeResources {
    /// Two timestamp queries: index 0 before the frame's GPU work, index 1 after.
    pub(crate) query_set: wgpu::QuerySet,
    /// `QUERY_RESOLVE` target for `resolve_query_set` (256-byte aligned, ≥ 16 B).
    pub(crate) resolve: wgpu::Buffer,
    /// `MAP_READ` staging buffer the two resolved `u64` ticks are copied into.
    pub(crate) readback: wgpu::Buffer,
    /// GPU memory charges of `resolve` and `readback` (released with the
    /// resources when measurement is disabled or the device is lost).
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) charges: [GpuCharge; 2],
}

/// Where a converted batch's segments live in the instanced line streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstanceRun {
    /// Index into [`Renderer::instance_streams`].
    pub(crate) stream: u32,
    /// First instance of this batch within its stream.
    pub(crate) offset: u32,
    /// Number of instances (segments) this batch contributes.
    pub(crate) count: u32,
}

/// Where a conic batch's SDF instances live in the conic streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArcRun {
    /// Index into [`Renderer::arc_streams`].
    pub(crate) stream: u32,
    /// First instance of this batch within its stream.
    pub(crate) offset: u32,
    /// Number of conic instances this batch contributes.
    pub(crate) count: u32,
}

/// Staged GPU resources, owned by the renderer that prepared them. Publishing
/// is separate so failed preparation leaves the active batches untouched.
pub struct PreparedUpload {
    pub(crate) batches: Vec<GpuBatch>,
    /// Instanced line streams created for the staged batches; their run indices
    /// are re-based when the upload is published.
    pub(crate) instances: Vec<InstanceStream>,
    /// SDF conic streams created for the staged batches; their run indices are
    /// re-based when the upload is published.
    pub(crate) arcs: Vec<ArcStream>,
    /// Degenerate conics skipped at pack in this upload (counted, never silent).
    pub(crate) arc_degenerate_skips: u64,
    /// Trailing unpaired vertices dropped from odd-length `Lines`/`MeshEdges`
    /// batches in this upload (counted, never silent).
    pub(crate) line_odd_vertex_skips: u64,
    /// Merged mesh chunks created for the staged batches; their run indices are
    /// re-based when the upload is published.
    pub(crate) mesh_chunks: Vec<GpuMeshChunk>,
    pub(crate) device_generation: u64,
    pub(crate) device: wgpu::Device,
    pub(crate) bytes: u64,
    pub(crate) elapsed_ms: f64,
}

pub(crate) struct ProgressiveFrame {
    /// O(1) camera/attachment key the cached `ordered` plan is valid for.
    pub(crate) key: FrameKey,
    pub(crate) ordered: Vec<usize>,
    pub(crate) opaque_count: usize,
    pub(crate) invisible: usize,
    pub(crate) cursor: usize,
    /// Per-`ordered` index (batch→position inverse, byte prefix sums) for the
    /// segmented page planner. Built in the same restart sweep that rebuilds
    /// `ordered`, so it is derived from that one truth, never a second one.
    pub(crate) page_index: plan::OrderedPageIndex,
}

/// O(1)-comparable identity of the camera + attachments a cached frame plan was
/// built for.
///
/// The per-batch transform is a pure function of the camera and the batch
/// origins, and batch origins only change on upload/clear/device-loss — all of
/// which reset `progressive_frame`. So comparing this key is equivalent to the
/// previous full-matrix `transforms` comparison, without materialising or
/// comparing a per-batch matrix.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct FrameKey {
    pub(crate) camera_matrix: [f32; 16],
    pub(crate) camera_position: [f32; 3],
    pub(crate) target_size: (u32, u32),
    pub(crate) texture_revision: u64,
}

impl PreparedUpload {
    /// Append another staged slice from the same device generation. No active
    /// scene is mutated, and stale-device pieces cannot be mixed.
    pub fn append(&mut self, mut piece: Self) -> CadResult<()> {
        if self.device != piece.device || self.device_generation != piece.device_generation {
            return Err(CadError::StaleResult);
        }
        // Re-base the appended piece's stream indices onto this piece's streams.
        let base = self.instances.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.instanced {
                run.stream += base;
            }
        }
        // Re-base the appended piece's merged chunk indices the same way.
        let mesh_base = self.mesh_chunks.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.mesh_runs {
                run.chunk += mesh_base;
            }
        }
        // Re-base the appended piece's conic stream indices.
        let arc_base = self.arcs.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.arc_runs {
                run.stream += arc_base;
            }
        }
        self.batches.append(&mut piece.batches);
        self.instances.append(&mut piece.instances);
        self.arcs.append(&mut piece.arcs);
        self.arc_degenerate_skips += piece.arc_degenerate_skips;
        self.mesh_chunks.append(&mut piece.mesh_chunks);
        self.bytes += piece.bytes;
        self.elapsed_ms += piece.elapsed_ms;
        Ok(())
    }
}
