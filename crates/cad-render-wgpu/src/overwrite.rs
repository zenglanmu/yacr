//! Named refusals for an in-place resident-batch overwrite.
//!
//! A transform-only update of one entity must move the entity's GPU geometry
//! **in place**: the same batch slot, the same pick key, the same batch/draw
//! order, no re-upload of the scene. [`Renderer::overwrite_batch`] therefore fits
//! the rebuilt [`cad_scene::RenderBatch`] against the resident
//! [`GpuBatch`](crate) and refuses — by name, never partially — whenever the two
//! are not provably interchangeable.
//!
//! Every variant below is a distinct, documented reason. A refusal is not an
//! error to swallow: the engine turns it into a full base re-stage, which is
//! always correct (it only costs the rebuild).
//!
//! [`Renderer::overwrite_batch`]: crate::Renderer::overwrite_batch

use std::fmt;

/// Why an in-place batch overwrite was refused.
///
/// The fit gate is checked in full **before** any buffer is touched, so a
/// refusal always leaves the resident scene exactly as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOverwriteRefusal {
    /// The renderer has no live device/queue (not initialized, or the device was
    /// lost). Nothing was written.
    NotInitialized,
    /// `index` is not a resident batch. Nothing was written.
    OutOfRange {
        /// The requested resident batch index.
        index: usize,
        /// The number of resident batches at the time of the refusal.
        batches: usize,
    },
    /// The rebuilt batch's [`cad_scene::RenderTopology`] differs from the
    /// resident one (a line batch cannot become a mesh in place).
    TopologyChanged,
    /// The rebuilt batch's alpha class (`opaque`/`transparent`/`invisible`)
    /// differs from the resident one. The class decides the draw pass, the
    /// cached draw plan and the flags path, so a class change is a re-stage.
    AlphaClassChanged,
    /// The rebuilt batch's `mirrored` winding flag differs; the resident batch is
    /// drawn by a pipeline whose cull state was chosen for the old value.
    MirroredChanged,
    /// The rebuilt batch's paint order (`draw_order`) differs. The cached draw
    /// plan orders the base once per base change, and an in-place overwrite must
    /// not reorder or rebuild it.
    DrawOrderChanged,
    /// The legacy path: the rebuilt batch's vertex count differs from the
    /// resident batch's, so it cannot fit the existing position/vertex buffer.
    VertexCountChanged {
        /// The resident batch's vertex count.
        existing: u32,
        /// The rebuilt batch's vertex count.
        rebuilt: usize,
    },
    /// The legacy path: the rebuilt batch's triangle-index count differs from the
    /// resident batch's index buffer.
    IndexCountChanged {
        /// The resident batch's index count.
        existing: u32,
        /// The rebuilt batch's index count.
        rebuilt: usize,
    },
    /// The legacy path: the rebuilt batch's anti-aliased line-quad count differs
    /// from the resident quad buffer.
    LineQuadCountChanged {
        /// The resident batch's quad-vertex count.
        existing: u32,
        /// The rebuilt batch's quad-vertex count.
        rebuilt: usize,
    },
    /// The legacy path: a per-batch attribute (`positions`, `normals`, `colors`,
    /// `line_quads`, `indices`) is present on one side and absent on the other,
    /// or the resident buffer is smaller than the rebuilt attribute. All four
    /// shape facts are checked before anything is written.
    AttributeShapeChanged {
        /// The attribute whose shape differs.
        attribute: &'static str,
        /// Whether the resident batch owns that buffer.
        existing: bool,
        /// Whether the rebuilt batch produces that attribute.
        rebuilt: bool,
    },
    /// The rebuilt geometry carries a non-finite coordinate. Writing it would
    /// silently drop the batch's pixels; the staging path rejects such a batch,
    /// so the overwrite refuses it too instead of publishing garbage.
    InvalidGeometry,
    /// The instanced line path: the rebuilt batch does not produce exactly the
    /// resident run list. A run is `(stream, offset, count)`; a different count,
    /// a different split or a different stream would move other batches'
    /// instances, so the overwrite is refused.
    RunListChanged,
    /// The instanced line path: the stream's stored `f64` origin is no longer a
    /// valid fixed-point reference for the rebuilt local coordinates (a stored
    /// coordinate would exceed `MAX_STREAM_RADIUS`, so a rebuild would have
    /// chosen a different origin and a different stream split). The stream origin
    /// is never rewritten by an overwrite: that would move every other batch
    /// sharing the stream.
    StreamOriginChanged,
    /// The batch's triangles live in a merged mesh chunk. [`MeshRun`] records a
    /// triangle/index range but **not** the member's vertex offset inside the
    /// chunk's interleaved vertex buffer, so an exact in-place rewrite is not
    /// provable today. A full rebuild is the honest fallback.
    ///
    /// [`MeshRun`]: crate
    MeshNotSupported,
    /// The batch's conics live in an SDF conic stream (`ArcStream`) that keeps no
    /// CPU mirror, and an [`ArcRun`] records no conic-index range, so the exact
    /// instance range cannot be re-packed. Refused by name rather than
    /// special-cased.
    ///
    /// [`ArcRun`]: crate
    ConicNotUpdatable,
    /// The accelerated tier's whole-scene pulled line storage could not be mapped
    /// exactly to the run's records (the resident segment/stream arithmetic does
    /// not resolve). Writing the instance mirror without the matching pulled
    /// records would leave the two views disagreeing, so the overwrite is refused
    /// and the caller re-stages.
    PulledNotUpdatable,
}

impl BatchOverwriteRefusal {
    /// The stable diagnostic code for every refusal of this class.
    pub fn diagnostic_code(&self) -> &'static str {
        cad_diagnostics::codes::RENDER_BATCH_OVERWRITE_REFUSED
    }

    /// The stable machine reason (`snake_case`), one per distinct gate.
    ///
    /// This is the identifier a diagnostic/host log keys off; the `Display` text
    /// is human prose and may change.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::OutOfRange { .. } => "out_of_range",
            Self::TopologyChanged => "topology_changed",
            Self::AlphaClassChanged => "alpha_class_changed",
            Self::MirroredChanged => "mirrored_changed",
            Self::DrawOrderChanged => "draw_order_changed",
            Self::VertexCountChanged { .. } => "vertex_count_changed",
            Self::IndexCountChanged { .. } => "index_count_changed",
            Self::LineQuadCountChanged { .. } => "line_quad_count_changed",
            Self::AttributeShapeChanged { .. } => "attribute_shape_changed",
            Self::InvalidGeometry => "invalid_geometry",
            Self::RunListChanged => "run_list_changed",
            Self::StreamOriginChanged => "stream_origin_changed",
            Self::MeshNotSupported => "mesh_not_supported",
            Self::ConicNotUpdatable => "conic_not_updatable",
            Self::PulledNotUpdatable => "pulled_not_updatable",
        }
    }
}

impl fmt::Display for BatchOverwriteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => {
                write!(f, "batch overwrite refused: the renderer has no live device")
            }
            Self::OutOfRange { index, batches } => write!(
                f,
                "batch overwrite refused: index {index} is outside the {batches} resident batches"
            ),
            Self::TopologyChanged => write!(
                f,
                "batch overwrite refused: the rebuilt topology differs from the resident batch"
            ),
            Self::AlphaClassChanged => write!(
                f,
                "batch overwrite refused: the rebuilt alpha class differs from the resident batch"
            ),
            Self::MirroredChanged => write!(
                f,
                "batch overwrite refused: the rebuilt winding (mirrored) flag differs"
            ),
            Self::DrawOrderChanged => write!(
                f,
                "batch overwrite refused: the rebuilt paint order differs (the draw plan is not rebuilt)"
            ),
            Self::VertexCountChanged { existing, rebuilt } => write!(
                f,
                "batch overwrite refused: vertex count changed from {existing} to {rebuilt}"
            ),
            Self::IndexCountChanged { existing, rebuilt } => write!(
                f,
                "batch overwrite refused: index count changed from {existing} to {rebuilt}"
            ),
            Self::LineQuadCountChanged { existing, rebuilt } => write!(
                f,
                "batch overwrite refused: line-quad count changed from {existing} to {rebuilt}"
            ),
            Self::AttributeShapeChanged {
                attribute,
                existing,
                rebuilt,
            } => write!(
                f,
                "batch overwrite refused: the `{attribute}` attribute shape changed \
                 (resident buffer: {existing}, rebuilt attribute: {rebuilt})"
            ),
            Self::InvalidGeometry => write!(
                f,
                "batch overwrite refused: the rebuilt geometry carries a non-finite coordinate"
            ),
            Self::RunListChanged => write!(
                f,
                "batch overwrite refused: the rebuilt instanced run list differs"
            ),
            Self::StreamOriginChanged => write!(
                f,
                "batch overwrite refused: the stream's stored f64 origin is no longer a valid \
                 reference for the rebuilt coordinates"
            ),
            Self::MeshNotSupported => write!(
                f,
                "batch overwrite refused: merged-mesh batches carry no member vertex offset, so \
                 an exact in-place rewrite is not provable (full rebuild required)"
            ),
            Self::ConicNotUpdatable => write!(
                f,
                "batch overwrite refused: the conic stream has no CPU mirror and the run records \
                 no conic-index range (full rebuild required)"
            ),
            Self::PulledNotUpdatable => write!(
                f,
                "batch overwrite refused: the accelerated pulled line records for this run cannot \
                 be resolved exactly (full rebuild required)"
            ),
        }
    }
}

impl std::error::Error for BatchOverwriteRefusal {}
