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
}

/// A frame's accumulated usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameUsage {
    pub vertices: usize,
    pub triangles: usize,
}

/// What (if anything) crossed a [`FrameBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded {
    /// `"vertices"` or `"triangles"`.
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneBudget {
    pub cpu_bytes: usize,
    pub queued_tasks: usize,
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
