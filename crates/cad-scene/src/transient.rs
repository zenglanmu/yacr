//! Carrier-neutral transient cursor geometry (ADR 0028).
//!
//! The pointer crosshair + pickbox is per-frame, O(1) screen feedback: it is
//! **not** scene content and must not go through scene preparation/publication/
//! commit. `cad-app` owns the pure geometry (`cad_app::render_scene::overlay::
//! cursor_geometry`) and hands this type to the renderer, which draws it in the
//! transient overlay pass. It never becomes a `RenderBatch` (no sources, no pick
//! key, no commit semantics).

use cad_domain::Point3;

/// Maximum number of line segments a transient cursor may carry: two crosshair
/// arms plus the four pickbox sides.
pub const MAX_TRANSIENT_CURSOR_SEGMENTS: usize = 6;

/// One renderer-owned transient cursor: bounded world-space line segments plus
/// the colour and paint order they draw with.
///
/// `segments` holds explicit `[start, end]` pairs in **world space** (the
/// renderer picks a fixed-point origin when packing). The geometry producer is
/// `cad_app::render_scene::overlay::cursor_geometry`; the renderer never
/// fabricates a cursor of its own (an absent/invalid cursor is `None`, an
/// explicit empty).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientCursor {
    /// World-space `[start, end]` line segments, at most
    /// [`MAX_TRANSIENT_CURSOR_SEGMENTS`].
    pub segments: Vec<[Point3; 2]>,
    /// Linear sRGB colour channels and alpha, in the same convention as a scene
    /// batch colour. The renderer sanitises it like every other colour.
    pub color: [f32; 4],
    /// Paint order among the transient overlay batches (preview < cursor < snap).
    pub draw_order: i64,
}
