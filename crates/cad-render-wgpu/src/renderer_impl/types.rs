/// Scalar facts the fast-path admission checks (see
/// [`Renderer::checkpoint_scalar_admits`]). Pure so every failure mode is
/// unit-testable without a GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckpointFacts {
    pub(crate) device_generation: u64,
    pub(crate) cache_epoch: u64,
    pub(crate) accelerated: bool,
    pub(crate) instance_streams: usize,
    pub(crate) mesh_chunks: usize,
    pub(crate) arc_streams: usize,
    pub(crate) line_segments: usize,
    pub(crate) mesh_segments: usize,
}

/// What the rebuild tail should do (pure): count a **successful** reuse, establish
/// a checkpoint, or invalidate it. A failed reuse must never count as a reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefixOutcome {
    CountReuse,
    Establish,
    Invalidate,
}

pub(crate) fn prefix_outcome(
    is_reuse: bool,
    reuse_failed: bool,
    establish: Option<usize>,
) -> PrefixOutcome {
    if reuse_failed {
        return PrefixOutcome::Invalidate;
    }
    if is_reuse {
        return PrefixOutcome::CountReuse;
    }
    match establish {
        Some(_) => PrefixOutcome::Establish,
        None => PrefixOutcome::Invalidate,
    }
}

/// The **new (incoming)** suffix's rebased resource references (the prepared
/// upload), used by the fast-path ownership proof so a suffix that references a
/// retained resource is rejected before a checkpoint is published or reused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IncomingResources {
    pub(crate) streams: Vec<u32>,
    pub(crate) meshes: Vec<u32>,
    pub(crate) arcs: Vec<u32>,
}

impl IncomingResources {
    /// Whether every incoming reference lies outside the retained prefix `[0, n)`
    /// (i.e. the replaced suffix shares no retained resource).
    pub(crate) fn disjoint_from(&self, streams: usize, meshes: usize, arcs: usize) -> bool {
        self.streams.iter().all(|&i| i as usize >= streams)
            && self.meshes.iter().all(|&i| i as usize >= meshes)
            && self.arcs.iter().all(|&i| i as usize >= arcs)
    }
}
