//! budget module.

use super::*;

/// A publishable update to the scene cache.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDelta {
    pub stamp: TaskStamp,
    pub added: Vec<RenderBatch>,
    pub removed_chunks: Vec<u64>,
}

impl SceneDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed_chunks.is_empty()
    }
}

/// Budget bookkeeping shared by the CPU scene and the GPU upload path.
///
/// The audit (F14, cross-cutting §8) found uploads had no per-frame budget. This
/// type accumulates the vertex/triangle counts a frame would submit and reports
/// the exact category and amount that crossed a limit, so the caller can emit a
/// diagnostic instead of silently dropping a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBudget {
    pub max_vertices: usize,
    pub max_triangles: usize,
    /// Maximum upload bytes this frame may submit. `usize::MAX` means the byte
    /// limit is not enforced (used by callers that only budget vertices and
    /// triangles, e.g. the pure draw-order tests).
    pub max_bytes: usize,
}

/// A frame's accumulated usage.
///
/// `bytes` is the upload size the frame would submit: position/normal/edge
/// vertex bytes plus index bytes (edges counted at their actual index width,
/// see [`FrameBudget::charge_bytes`]). It is only meaningful when the caller
/// charges bytes; the vertex/triangle-only path leaves it at `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameUsage {
    pub vertices: usize,
    pub triangles: usize,
    pub bytes: usize,
}

/// What (if anything) crossed a [`FrameBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded {
    /// `"vertices"`, `"triangles"` or `"bytes"`.
    pub category: &'static str,
    /// Amount the frame would have used, including the offending charge.
    pub requested: usize,
    pub limit: usize,
}

impl FrameBudget {
    pub fn from_scene(budget: &SceneBudget) -> Self {
        FrameBudget {
            max_vertices: budget.max_vertices_per_frame,
            max_triangles: budget.max_triangles_per_frame,
            max_bytes: budget.upload_bytes_per_frame,
        }
    }

    /// Charge one batch. `vertices` is its vertex count and `triangles` its
    /// triangle count (0 for line batches). Returns the first limit crossed, if
    /// any; on success the usage is updated in place.
    pub fn charge(
        &self,
        usage: &mut FrameUsage,
        vertices: usize,
        triangles: usize,
    ) -> Result<(), BudgetExceeded> {
        let next_vertices = usage.vertices.saturating_add(vertices);
        if next_vertices > self.max_vertices {
            return Err(BudgetExceeded {
                category: "vertices",
                requested: next_vertices,
                limit: self.max_vertices,
            });
        }
        let next_triangles = usage.triangles.saturating_add(triangles);
        if next_triangles > self.max_triangles {
            return Err(BudgetExceeded {
                category: "triangles",
                requested: next_triangles,
                limit: self.max_triangles,
            });
        }
        usage.vertices = next_vertices;
        usage.triangles = next_triangles;
        Ok(())
    }

    /// Charge the exact GPU upload size of one batch, in bytes.
    ///
    /// Callers pass [`RenderBatch::upload_size_bytes`] (or the cached
    /// `GpuBatch::upload_size_bytes`), so the charged amount is the packed buffer
    /// size, not an estimate: every vertex attribute is three `f32` (12 bytes),
    /// mesh triangle indices are `[u32; 3]` (12 bytes) and wireframe edge indices
    /// are `u32` (the mesh index buffer is `IndexFormat::Uint32`, see
    /// `cad-render-wgpu::plan::draw_batch`). Positions, normals and edge
    /// positions all count, because all three are uploaded.
    ///
    /// Deriving the charge from the batch itself means a caller cannot claim a
    /// smaller upload than it submits. When the limit is crossed the usage is
    /// left unchanged and a `"bytes"` [`BudgetExceeded`] is returned; nothing is
    /// dropped silently.
    pub fn charge_bytes(&self, usage: &mut FrameUsage, bytes: usize) -> Result<(), BudgetExceeded> {
        let next = usage.bytes.saturating_add(bytes);
        if next > self.max_bytes {
            return Err(BudgetExceeded {
                category: "bytes",
                requested: next,
                limit: self.max_bytes,
            });
        }
        usage.bytes = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneBudget {
    pub cpu_bytes: usize,
    /// Maximum tasks that may be in flight at once.
    ///
    /// Enforced by [`TaskQueue`]: submitting past this limit is rejected with an
    /// explicit [`BudgetError`], never silently dropped.
    pub queued_tasks: usize,
    /// Maximum GPU upload bytes a single frame may submit.
    ///
    /// Enforced by [`FrameBudget::charge_bytes`]; the amount is the packed
    /// buffer size, not an estimate.
    pub upload_bytes_per_frame: usize,
    /// Maximum vertices a single frame may submit across every batch.
    pub max_vertices_per_frame: usize,
    /// Maximum triangles a single frame may submit across every mesh batch.
    pub max_triangles_per_frame: usize,
}

impl Default for SceneBudget {
    fn default() -> Self {
        SceneBudget {
            cpu_bytes: 128 * 1024 * 1024,
            queued_tasks: 8,
            upload_bytes_per_frame: 4 * 1024 * 1024,
            max_vertices_per_frame: 8_000_000,
            max_triangles_per_frame: 2_000_000,
        }
    }
}

/// A budget category that was crossed, with the amount and the limit.
///
/// Unlike a decision that just returns a bool, this always carries the numbers
/// so the caller can emit a diagnostic instead of silently dropping work. It
/// covers the CPU-side categories (`cpu_bytes`, `queued_tasks`) that the GPU
/// frame plan does not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetError {
    /// `"cpu_bytes"` or `"queued_tasks"`.
    pub category: &'static str,
    /// Amount that would have been used, including the offending charge.
    pub requested: usize,
    pub limit: usize,
}

/// A back-pressured queue of in-flight tasks.
///
/// `submit` refuses to exceed `max_queued` and returns an explicit
/// [`BudgetError`]; it never drops a submission and never accepts over capacity.
/// `complete` releases a slot. This is the enforcement of
/// [`SceneBudget::queued_tasks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskQueue {
    max_queued: usize,
    in_flight: usize,
}

impl TaskQueue {
    pub fn new(max_queued: usize) -> Self {
        TaskQueue {
            max_queued,
            in_flight: 0,
        }
    }

    pub fn from_scene(budget: &SceneBudget) -> Self {
        TaskQueue::new(budget.queued_tasks)
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub fn max_queued(&self) -> usize {
        self.max_queued
    }

    /// Reserve one in-flight slot, or report the queue as over budget.
    ///
    /// All-or-nothing: a rejected submission does not consume a slot.
    pub fn submit(&mut self) -> Result<(), BudgetError> {
        let requested = self.in_flight.saturating_add(1);
        if requested > self.max_queued {
            return Err(BudgetError {
                category: "queued_tasks",
                requested,
                limit: self.max_queued,
            });
        }
        self.in_flight = requested;
        Ok(())
    }

    /// Release a slot for a task that finished (successfully or not).
    ///
    /// Saturating: a spurious `complete` cannot underflow the counter.
    pub fn complete(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }
}
