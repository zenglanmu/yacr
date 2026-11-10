//! Resident WIPEOUT mask state (ADR 0022 C1/C2).
//!
//! Split out of `lib.rs` (which is at the file-size ceiling) so the fill and the
//! `WIPEOUTFRAME` boundary frame keep their full contracts next to the mask
//! renderer.

use super::*;

/// One resident WIPEOUT mask: a tessellated fill plus the `f64` reference origin
/// its stored positions are measured from.
///
/// A mask is **not** a [`GpuBatch`]: it is painted by the frame's final pass with
/// `depth_compare: Always`, no depth write, and the clear colour (ADR 0022 D1),
/// so it must never enter the ordered geometry plan. The fill is tessellated with
/// the same even-odd fill solid HATCH uses (`cad_domain::hatch::fill_rings`), so a
/// concave boundary does not leak.
///
/// A non-`inverted` mask is filled at upload from its boundary alone. An
/// `inverted` mask fills the **complement**, which needs a ring covering the
/// frame's visible world rect: its geometry is therefore built by the first frame
/// that draws it (see `ensure_inverted_mask_fill`) and rebuilt only when the view
/// leaves the resident ring.
pub(crate) struct GpuMask {
    /// Triangle list positions, relative to [`GpuMask::origin`]. `None` until an
    /// inverted mask's first frame has built its complement fill.
    pub(crate) vertices: Option<wgpu::Buffer>,
    pub(crate) vertex_count: u32,
    pub(crate) origin: [f64; 3],
    /// The untransformed world boundary, retained for the inverted fill (the ring
    /// depends on the view, so it cannot be baked once).
    pub(crate) polygon: Arc<[Point3]>,
    /// `true` for clip mode Inside: mask everything outside `polygon`.
    pub(crate) inverted: bool,
    /// World-XY rectangle the resident inverted fill guarantees to cover. `None`
    /// for a non-inverted mask or before the first inverted fill.
    pub(crate) covered: Option<MaskCoverage>,
    /// GPU memory charge of the resident fill geometry, or `None` while an
    /// inverted mask has no ring yet.
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) vertex_charge: Option<GpuCharge>,
    /// Closed-boundary frame as pre-expanded anti-aliased line quads, relative to
    /// [`GpuMask::frame_origin`]. `None` when `WIPEOUTFRAME` is `0`/plot-only.
    pub(crate) frame_vertices: Option<wgpu::Buffer>,
    pub(crate) frame_vertex_count: u32,
    /// Origin of the frame's local coordinates. The frame has its own uniform
    /// slot, so it is independent of the fill's `origin`.
    pub(crate) frame_origin: [f64; 3],
    /// Resolved linear sRGB colour the frame strokes with. `None` exactly when
    /// `frame_vertices` is `None`.
    pub(crate) frame_color: Option<[f32; 3]>,
    /// GPU memory charge of the frame geometry (released on drop).
    #[allow(dead_code)]
    pub(crate) frame_charge: Option<GpuCharge>,
}
