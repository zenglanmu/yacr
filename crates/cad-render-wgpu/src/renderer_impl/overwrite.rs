//! In-place resident-batch overwrite (phase P4: transform-only updates).
//!
//! A transform-only edit of one entity must move its GPU geometry in place: the
//! same batch slot, the same pick key, the same batch order and draw plan, and
//! no scene re-upload. [`Renderer::overwrite_batch`] does exactly that after
//! proving the rebuilt [`cad_scene::RenderBatch`] is interchangeable with the
//! resident [`GpuBatch`] — see [`BatchOverwriteRefusal`] for every refusal.
//!
//! # Never a partial write
//!
//! [`Renderer::overwrite_batch`] validates the **whole** gate first and only
//! then writes. [`Renderer::check_overwrite_batch`] runs the same gate without
//! writing, so the engine can prove a multi-batch entity update is all-or-nothing
//! before the first byte moves.
//!
//! # What is deliberately not touched
//!
//! The flags table (the pick key is unchanged), the pick table, the batch order,
//! the cached draw plan (the paint order is gated equal; only the retained
//! transparent-sort centroid is refreshed), the accelerated pulled segment
//! organisation (stream/segment counts and identities are gated equal) and the
//! per-batch GPU buffers' allocations (the writes target the existing buffers).
//!
//! # Classes that cannot be proven equivalent are refused by name
//!
//! * merged mesh ([`BatchOverwriteRefusal::MeshNotSupported`]): a `MeshRun`
//!   records its triangle/index range but not the member's vertex offset inside
//!   the chunk's interleaved vertex buffer, so no exact in-place rewrite exists
//!   today.
//! * SDF conics ([`BatchOverwriteRefusal::ConicNotUpdatable`]): an `ArcStream`
//!   keeps no CPU mirror and an `ArcRun` records no conic-index range.
//! * the accelerated pulled line storage when its record mapping cannot be
//!   resolved exactly ([`BatchOverwriteRefusal::PulledNotUpdatable`]).
use super::upload::batch_world_z_range;
use super::*;
use crate::overwrite::BatchOverwriteRefusal;

/// The validated, still-unwritten effect of one in-place overwrite.
///
/// Built entirely before the first write; nothing in it can fail afterwards, so
/// a refusal never leaves a half-updated batch.
struct OverwritePlan {
    /// `(target buffer, byte offset, payload)` in write order.
    writes: Vec<(wgpu::Buffer, u64, Vec<u8>)>,
    /// Accelerated-tier instance-mirror updates: `(stream, first instance,
    /// instances)`. Kept in lockstep with the pulled records written above so the
    /// two never disagree.
    mirrors: Vec<(u32, u32, Vec<LineInstance>)>,
    /// Streams whose world AABB must be widened to cover the rebuilt geometry.
    /// Widening only (never shrinking) keeps the frame's chunk cull conservative:
    /// a moved run is never culled away, and other batches sharing the stream are
    /// never exposed to a too-small box.
    aabbs: Vec<(u32, WorldAabb)>,
}

impl Renderer {
    /// Validate an in-place overwrite **without writing anything**.
    ///
    /// Same fit gate as [`Renderer::overwrite_batch`]; a caller that updates
    /// several batches of one entity uses it to guarantee the group is
    /// all-or-nothing (see the engine's overwrite path).
    pub fn check_overwrite_batch(
        &self,
        index: usize,
        batch: &cad_scene::RenderBatch,
    ) -> Result<(), BatchOverwriteRefusal> {
        self.plan_overwrite(index, batch).map(|_| ())
    }

    /// Overwrite resident batch `index` in place with `batch`.
    ///
    /// On `Ok` the batch's GPU geometry has been rewritten into its **existing**
    /// buffers and its non-geometry facts (`origin`, `centroid`, `z_range`,
    /// `color`, `alpha`, `lineweight`, `sources`) are refreshed. `pick_key`,
    /// batch order, draw order, the pick table and the flags table are unchanged,
    /// so a live selection survives a move.
    ///
    /// On `Err` **nothing** was written: every gate in [`BatchOverwriteRefusal`]
    /// is checked before the first buffer write. The caller must fall back to a
    /// full base re-stage rather than keep the old geometry.
    pub fn overwrite_batch(
        &mut self,
        index: usize,
        batch: &cad_scene::RenderBatch,
    ) -> Result<(), BatchOverwriteRefusal> {
        let plan = self.plan_overwrite(index, batch)?;
        let queue = self
            .queue
            .clone()
            .ok_or(BatchOverwriteRefusal::NotInitialized)?;
        // Everything is validated; execute the writes.
        for (buffer, offset, bytes) in &plan.writes {
            queue.write_buffer(buffer, *offset, bytes);
        }
        for (stream, first, instances) in plan.mirrors {
            if let Some(target) = self.instance_streams.get_mut(stream as usize) {
                let start = first as usize;
                let end = start + instances.len();
                target.pulled_instances[start..end].clone_from_slice(&instances);
            }
        }
        for (stream, aabb) in plan.aabbs {
            if let Some(target) = self.instance_streams.get_mut(stream as usize) {
                target.aabb = widen_aabb(target.aabb, aabb);
            }
        }
        let centroid = batch.centroid();
        let z_range = batch_world_z_range(batch);
        let lineweight_changed = self
            .batches
            .get(index)
            .is_some_and(|existing| existing.lineweight != batch.lineweight);
        if let Some(existing) = self.batches.get_mut(index) {
            // Geometry-moving facts only. `pick_key`, `topology`, `mirrored`, the
            // counts, the run lists and `upload_bytes` are gated equal, so they
            // are not rewritten; `upload_bytes` in particular has exactly the
            // same inputs (counts + run shape + tier), so its value is unchanged.
            existing.origin = [
                batch.local_origin.x as f32,
                batch.local_origin.y as f32,
                batch.local_origin.z as f32,
            ];
            existing.centroid = centroid;
            existing.z_range = z_range;
            existing.color = batch.color;
            existing.alpha = batch.alpha;
            existing.lineweight = batch.lineweight;
            existing.sources = batch.sources.clone();
        }
        // The retained transparent-sort centroid is a mirror of the field just
        // updated. Refreshing it is what keeps the back-to-front order correct
        // without rebuilding the plan (the paint order and class are gated equal).
        if let Some(plan) = self.draw_plan.as_mut() {
            plan.set_centroid(index, centroid);
        }
        // A carried-but-not-drawn lineweight report is baked from the batch set at
        // commit. A move does not change it; a lineweight change (outside the
        // transform-only contract, but gated fit) rebuilds the small report.
        if lineweight_changed {
            self.rebuild_lineweight_cache();
        }
        // Geometry changed: the progressive page walk must redraw from page 0 and
        // cannot preserve the retained colour (unlike a flags/tint restart).
        self.preserve_color_on_restart = false;
        if let Some(frame) = self.progressive_frame.as_mut() {
            frame.cursor = 0;
        }
        self.batch_overwrites += 1;
        Ok(())
    }

    /// Cumulative successful in-place batch overwrites (diagnostic).
    pub fn batch_overwrites(&self) -> u64 {
        self.batch_overwrites
    }

    /// The fit gate: everything that must hold for a safe in-place write.
    fn plan_overwrite(
        &self,
        index: usize,
        batch: &cad_scene::RenderBatch,
    ) -> Result<OverwritePlan, BatchOverwriteRefusal> {
        if self.device.is_none() || self.queue.is_none() || self.device_lost {
            return Err(BatchOverwriteRefusal::NotInitialized);
        }
        let existing = self
            .batches
            .get(index)
            .ok_or(BatchOverwriteRefusal::OutOfRange {
                index,
                batches: self.batches.len(),
            })?;
        if existing.topology != batch.topology {
            return Err(BatchOverwriteRefusal::TopologyChanged);
        }
        if classify_alpha(existing.alpha) != classify_alpha(batch.alpha) {
            return Err(BatchOverwriteRefusal::AlphaClassChanged);
        }
        if existing.mirrored != batch.mirrored {
            return Err(BatchOverwriteRefusal::MirroredChanged);
        }
        if existing.draw_order != batch.draw_order {
            return Err(BatchOverwriteRefusal::DrawOrderChanged);
        }
        // A merged mesh chunk's member vertex offset is not recorded on its run,
        // so no exact in-place rewrite is provable. Refuse by name.
        if !existing.mesh_runs.is_empty() {
            return Err(BatchOverwriteRefusal::MeshNotSupported);
        }
        // A conic stream keeps no CPU mirror and the run has no conic-index
        // range, so it is refused by name rather than special-cased.
        if !existing.arc_runs.is_empty() {
            return Err(BatchOverwriteRefusal::ConicNotUpdatable);
        }
        if existing.instanced.is_empty() {
            self.plan_legacy_overwrite(existing, batch)
        } else {
            self.plan_instanced_overwrite(existing, batch)
        }
    }

    /// The legacy per-batch path: positions / normals / colors / triangle
    /// indices / anti-aliased line quads live in the batch's own buffers.
    fn plan_legacy_overwrite(
        &self,
        existing: &GpuBatch,
        batch: &cad_scene::RenderBatch,
    ) -> Result<OverwritePlan, BatchOverwriteRefusal> {
        if batch
            .vertices
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(BatchOverwriteRefusal::InvalidGeometry);
        }
        if existing.vertex_count as usize != batch.vertices.len() {
            return Err(BatchOverwriteRefusal::VertexCountChanged {
                existing: existing.vertex_count,
                rebuilt: batch.vertices.len(),
            });
        }
        // The exact buffers the staging path would produce for this batch (see
        // `prepare_upload_batches_inner`'s non-converted arm).
        let (positions, indices, normals, colors, quads) = match batch.topology {
            RenderTopology::Mesh => {
                let positions = pack_positions(&batch.vertices);
                let indices: Vec<u32> = batch
                    .indices
                    .iter()
                    .flat_map(|triangle| [triangle[0], triangle[1], triangle[2]])
                    .collect();
                let normals = pack_positions(&geometry::repaired_normals(batch));
                let colors = pack_positions(&geometry::repaired_colors(batch));
                let edges = geometry::mesh_edge_indices(batch);
                let mut quads = Vec::new();
                if !edges.is_empty() {
                    expand_line_quads(&mut quads, batch, Some(&edges));
                }
                (positions, indices, normals, colors, line_quad_bytes(&quads))
            }
            RenderTopology::MeshEdges | RenderTopology::Lines => {
                let mut quads = Vec::new();
                expand_line_quads(&mut quads, batch, None);
                (
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    line_quad_bytes(&quads),
                )
            }
            // An invisible conic batch owns no draw path and no per-batch buffer.
            RenderTopology::Conics => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        };
        if existing.index_count as usize != indices.len() {
            return Err(BatchOverwriteRefusal::IndexCountChanged {
                existing: existing.index_count,
                rebuilt: indices.len(),
            });
        }
        let quad_count = quads.len() / LINE_QUAD_STRIDE as usize;
        if existing.line_quad_count as usize != quad_count {
            return Err(BatchOverwriteRefusal::LineQuadCountChanged {
                existing: existing.line_quad_count,
                rebuilt: quad_count,
            });
        }
        let mut writes = Vec::new();
        push_shape_write(&mut writes, &existing.vertices, &positions, "positions")?;
        push_shape_write(&mut writes, &existing.normals, &normals, "normals")?;
        push_shape_write(&mut writes, &existing.colors, &colors, "colors")?;
        push_shape_write(&mut writes, &existing.line_quads, &quads, "line_quads")?;
        push_index_write(&mut writes, &existing.indices, &indices, "indices")?;
        Ok(OverwritePlan {
            writes,
            mirrors: Vec::new(),
            aabbs: Vec::new(),
        })
    }

    /// The instanced line path: re-pack this batch's instances against the
    /// **stream's stored `f64` origin** and write them at the run's instance
    /// offset in the existing stream buffer (and, on the accelerated tier, at the
    /// exact whole-scene pulled record range).
    fn plan_instanced_overwrite(
        &self,
        existing: &GpuBatch,
        batch: &cad_scene::RenderBatch,
    ) -> Result<OverwritePlan, BatchOverwriteRefusal> {
        if !matches!(
            existing.topology,
            RenderTopology::Lines | RenderTopology::MeshEdges
        ) {
            // A mesh batch with instanced runs is a merged mesh (its edge
            // overlay); its triangle range cannot be rewritten in place.
            return Err(BatchOverwriteRefusal::MeshNotSupported);
        }
        if existing.vertex_count as usize != batch.vertices.len() {
            return Err(BatchOverwriteRefusal::VertexCountChanged {
                existing: existing.vertex_count,
                rebuilt: batch.vertices.len(),
            });
        }
        // The rebuilt batch must still convert to the instanced path with the same
        // segment derivation (consecutive vertex pairs), or a rebuild would place
        // it somewhere else entirely.
        if !line_batch_is_instanced(batch) {
            return Err(BatchOverwriteRefusal::RunListChanged);
        }
        let segments = (batch.vertices.len() / 2) as u64;
        let resident: u64 = existing
            .instanced
            .iter()
            .map(|run| u64::from(run.count))
            .sum();
        if resident != segments {
            return Err(BatchOverwriteRefusal::RunListChanged);
        }
        let accelerated = self.render_tier == RenderTier::Accelerated;
        let stride = if accelerated {
            PULLED_LINE_STRIDE
        } else {
            LINE_INSTANCE_STRIDE
        };
        let mut writes = Vec::new();
        let mut mirrors = Vec::new();
        let mut aabbs = Vec::new();
        let mut first_segment = 0u32;
        for run in &existing.instanced {
            let stream = self
                .instance_streams
                .get(run.stream as usize)
                .ok_or(BatchOverwriteRefusal::RunListChanged)?;
            if run.offset.checked_add(run.count).is_none()
                || run.offset + run.count > stream.instance_count
            {
                return Err(BatchOverwriteRefusal::RunListChanged);
            }
            let mut packed = Vec::with_capacity(run.count as usize);
            push_line_instances(
                &mut packed,
                batch,
                None,
                stream.origin,
                first_segment,
                run.count,
                existing.pick_key,
            );
            if packed.len() != run.count as usize {
                return Err(BatchOverwriteRefusal::RunListChanged);
            }
            if packed
                .iter()
                .flat_map(|instance| instance.p0_width[..3].iter().chain(&instance.p1_pad[..3]))
                .any(|value| !value.is_finite())
            {
                return Err(BatchOverwriteRefusal::InvalidGeometry);
            }
            // Fixed-point gate: every stored coordinate must stay inside the
            // documented stream radius. Otherwise a rebuild would have chosen a
            // different origin (and possibly a different split), so this origin is
            // no longer a valid reference.
            if !instances_within_stream_radius(&packed) {
                return Err(BatchOverwriteRefusal::StreamOriginChanged);
            }
            first_segment += run.count;
            let (payload, offset) = if accelerated {
                // On this tier the stream's buffer **is** its pulled segment's
                // geometry buffer (64-byte `PulledLine` records), so the write
                // offset is the stream-relative record index. The whole-scene
                // mapping still has to resolve exactly: it proves which segment
                // covers this stream and that the segment's record count agrees
                // with the streams it spans, so the mirror and the pulled records
                // can never disagree.
                let chunk = match self.pulled_lines.as_ref() {
                    Some(pulled) => {
                        let (segment, _) = self
                            .pulled_stream_record_base(run.stream)
                            .ok_or(BatchOverwriteRefusal::PulledNotUpdatable)?;
                        let covered = stream
                            .base
                            .checked_add(run.offset)
                            .and_then(|first| first.checked_add(run.count))
                            .ok_or(BatchOverwriteRefusal::PulledNotUpdatable)?;
                        if covered > pulled.segments[segment].instance_count {
                            return Err(BatchOverwriteRefusal::PulledNotUpdatable);
                        }
                        run.stream
                    }
                    // The class fell back to real per-stream storage; its records
                    // were built with chunk base 0 (see `build_line_fallback`).
                    None => 0,
                };
                let first_record = stream
                    .base
                    .checked_add(run.offset)
                    .ok_or(BatchOverwriteRefusal::PulledNotUpdatable)?;
                let records = build_pulled_lines(&[packed.as_slice()], chunk);
                (
                    pulled_line_bytes(&records),
                    u64::from(first_record) * u64::from(PULLED_LINE_STRIDE),
                )
            } else {
                (
                    line_instance_bytes(&packed),
                    u64::from(stream.base + run.offset) * u64::from(LINE_INSTANCE_STRIDE),
                )
            };
            // The payload must be exactly one record per instance and fit the
            // existing buffer at that offset; anything else is a mapping we cannot
            // prove, so it is refused rather than written.
            if payload.len() != run.count as usize * stride as usize {
                return Err(BatchOverwriteRefusal::PulledNotUpdatable);
            }
            let needed = offset
                .checked_add(payload.len() as u64)
                .ok_or(BatchOverwriteRefusal::PulledNotUpdatable)?;
            if stream.buffer.size() < needed {
                return Err(BatchOverwriteRefusal::PulledNotUpdatable);
            }
            writes.push((stream.buffer.clone(), offset, payload));
            if accelerated {
                let start = run.offset as usize;
                let end = start
                    .checked_add(packed.len())
                    .ok_or(BatchOverwriteRefusal::PulledNotUpdatable)?;
                if end > stream.pulled_instances.len() {
                    return Err(BatchOverwriteRefusal::PulledNotUpdatable);
                }
                mirrors.push((run.stream, run.offset, packed));
            }
            if let Some(aabb) = batch_world_line_aabb(batch) {
                aabbs.push((run.stream, aabb));
            }
        }
        Ok(OverwritePlan {
            writes,
            mirrors,
            aabbs,
        })
    }

    /// The resident pulled segment that owns `stream` and the whole-scene record
    /// index of its first instance, resolved by walking the pulled segments.
    ///
    /// The pulled payload is the concatenation of the resident instance streams
    /// in stream-index order, split into binding-sized segments at **whole-stream**
    /// boundaries (see `rebuild_pulled`). Each segment records its own record
    /// count, so the segment boundaries are exactly the prefix sums of
    /// `PulledLineSegment::instance_count`; a segment owns the streams whose
    /// `instance_count`s add up to that count. Every step is verified — a segment
    /// whose count does not match the streams it would cover, an out-of-range
    /// stream, or a walk that ends before the target returns `None`, and the
    /// caller refuses rather than writing a mismatched pulled record.
    ///
    /// The returned segment index is used for the containment check. The write
    /// offset itself is segment-relative (`stream.base + run.offset`): on this
    /// tier the segment's geometry buffer **is** the stream's per-run instance
    /// buffer, so there is no second whole-scene copy to address.
    fn pulled_stream_record_base(&self, stream: u32) -> Option<(usize, u32)> {
        let pulled = self.pulled_lines.as_ref()?;
        let target = stream as usize;
        if target >= self.instance_streams.len() {
            return None;
        }
        let mut cursor = 0usize;
        let mut base = 0u32;
        for (index, segment) in pulled.segments.iter().enumerate() {
            let mut covered = 0u32;
            while cursor < self.instance_streams.len() && covered < segment.instance_count {
                covered = covered.checked_add(self.instance_streams[cursor].instance_count)?;
                cursor += 1;
            }
            if covered != segment.instance_count {
                return None;
            }
            if target < cursor {
                return Some((index, base));
            }
            base = base.checked_add(segment.instance_count)?;
        }
        None
    }
}

/// Append a whole-buffer vertex/attribute write, refusing a presence or size
/// mismatch instead of writing a short (or over-long) buffer.
fn push_shape_write(
    writes: &mut Vec<(wgpu::Buffer, u64, Vec<u8>)>,
    buffer: &Option<wgpu::Buffer>,
    data: &[u8],
    attribute: &'static str,
) -> Result<(), BatchOverwriteRefusal> {
    match (buffer.as_ref(), data.is_empty()) {
        (None, true) => Ok(()),
        (Some(buffer), false) if buffer.size() >= data.len() as u64 => {
            writes.push((buffer.clone(), 0, data.to_vec()));
            Ok(())
        }
        (existing, rebuilt) => Err(BatchOverwriteRefusal::AttributeShapeChanged {
            attribute,
            existing: existing.is_some(),
            rebuilt: !rebuilt,
        }),
    }
}

/// Append the triangle-index buffer write (a `u32` per index).
fn push_index_write(
    writes: &mut Vec<(wgpu::Buffer, u64, Vec<u8>)>,
    buffer: &Option<wgpu::Buffer>,
    indices: &[u32],
    attribute: &'static str,
) -> Result<(), BatchOverwriteRefusal> {
    let bytes: Vec<u8> = indices
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect();
    push_shape_write(writes, buffer, &bytes, attribute)
}

/// Whether every packed instance coordinate is inside `MAX_STREAM_RADIUS` of its
/// stream origin. The stored value **is** the already-reduced `world - origin`.
fn instances_within_stream_radius(instances: &[LineInstance]) -> bool {
    instances.iter().all(|instance| {
        instance.p0_width[..3]
            .iter()
            .chain(&instance.p1_pad[..3])
            .all(|value| f64::from(*value).abs() <= MAX_STREAM_RADIUS)
    })
}

/// World-space AABB of an instanced line batch's vertices.
fn batch_world_line_aabb(batch: &cad_scene::RenderBatch) -> Option<WorldAabb> {
    let origin = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in &batch.vertices {
        for axis in 0..3 {
            let world = origin[axis] + f64::from(vertex[axis]);
            if !world.is_finite() {
                continue;
            }
            min[axis] = min[axis].min(world);
            max[axis] = max[axis].max(world);
        }
    }
    (0..3)
        .all(|axis| min[axis] <= max[axis])
        .then_some(WorldAabb { min, max })
}

/// Union of two world AABBs; non-finite boxes are ignored, and the result is
/// never allowed to shrink (the frame cull must stay conservative).
fn widen_aabb(current: WorldAabb, widened: WorldAabb) -> WorldAabb {
    let finite = |aabb: &WorldAabb| {
        aabb.min
            .iter()
            .chain(&aabb.max)
            .all(|value| value.is_finite())
    };
    if !finite(&current) {
        return widened;
    }
    if !finite(&widened) {
        return current;
    }
    WorldAabb {
        min: [
            current.min[0].min(widened.min[0]),
            current.min[1].min(widened.min[1]),
            current.min[2].min(widened.min[2]),
        ],
        max: [
            current.max[0].max(widened.max[0]),
            current.max[1].max(widened.max[1]),
            current.max[2].max(widened.max[2]),
        ],
    }
}
