use super::*;

impl Renderer {
    pub(super) fn rebuild_pulled(&mut self, fast: PulledFastPath) {
        // Take the previous pulled state so an admitted reuse can retain its base
        // segments; both classes are replaced below either way.
        let previous_lines = self.pulled_lines.take();
        let previous_mesh = self.pulled_mesh.take();
        if self.render_tier != RenderTier::Accelerated {
            self.pulled_state = PulledState::Unavailable;
            return;
        }
        let (Some(device), Some(queue)) = (self.device.clone(), self.queue.clone()) else {
            self.pulled_state = PulledState::Unavailable;
            return;
        };
        let (Some(line_layout), Some(mesh_layout)) = (
            self.pulled_line_storage_layout.clone(),
            self.pulled_mesh_storage_layout.clone(),
        ) else {
            self.pulled_state = PulledState::Unavailable;
            return;
        };
        let limit = self.pulled_binding_limit_override.unwrap_or_else(|| {
            device
                .limits()
                .max_storage_buffer_binding_size
                .min(device.limits().max_buffer_size)
        });
        let mut table_bytes_uploaded = 0u64;

        // --- capacity gate (pre-flight, before any allocation) --------------
        //
        // A whole-scene pulled organisation is one extra copy of every converted
        // instance/vertex, allocated in binding-sized segments. `create_buffer` is
        // not fallible through its return value, so an over-capacity request
        // panics through wgpu's default error handler; the gate refuses it here
        // instead and lets the universal per-run path draw the class. See
        // `pulled_budget_for` for how the budget is derived and
        // `pulled_payload_bytes` for the requirement.
        //
        // The requirement counts the **whole resident** pulled set, so a
        // prefix-retaining suffix commit is checked conservatively (it would reuse
        // its base segments instead of reallocating them). Skipping is always
        // correct — the universal path draws the same pixels — so a conservative
        // verdict only costs the accelerated organisation.
        let required_bytes = self.pulled_required_bytes();
        let budget_bytes = self.pulled_budget_bytes(&device.limits());
        if !pulled_capacity_fits(required_bytes, budget_bytes) {
            // Drop the resident pulled classes (their buffers survive through the
            // streams' handles until the fallback replaces them, so nothing is
            // left dangling) and give every stream real per-run geometry.
            self.pulled_lines = None;
            self.pulled_mesh = None;
            self.invalidate_pulled_prefix();
            // The fallback allocates too, so it is fallible: a device that cannot
            // satisfy even the per-stream geometry is reported as an allocation
            // failure rather than as an over-budget pull (and never as a panic or
            // an empty success).
            self.pulled_state = match self.build_line_fallback(&device, &queue) {
                Ok(()) => PulledState::SkippedInsufficientVram {
                    required_bytes,
                    budget_bytes,
                },
                Err(error) => pulled_skip_state(
                    required_bytes,
                    budget_bytes,
                    error.with_note(
                        "the universal per-run fallback could not be rebuilt after the capacity skip",
                    ),
                ),
            };
            return;
        }

        // Resolve the reuse plan: the retained base segments, the resource
        // boundaries, and whether a checkpoint should be (re)established.
        let is_reuse_plan = matches!(fast, PulledFastPath::Reuse(_));
        // Derived retained boundaries: the checkpoint's for a reuse, the validated
        // prefix ownership for a full warm-up (so the warm-up still forces the
        // base boundary and can record the base ranges).
        let (derived_line, derived_mesh, boundaries_ok) = self.derived_boundaries(&fast);
        let mut establish = match &fast {
            PulledFastPath::Full { establish } => *establish,
            PulledFastPath::Reuse(_) => None,
        };
        if !boundaries_ok {
            // The ownership proof failed: do not establish a checkpoint.
            establish = None;
        }
        let checkpoint = match &fast {
            PulledFastPath::Reuse(checkpoint) if boundaries_ok => Some((**checkpoint).clone()),
            _ => None,
        };
        let is_reuse = is_reuse_plan && checkpoint.is_some();
        let reuse_line_boundary = derived_line;
        let reuse_mesh_boundary = derived_mesh;
        let mut reusable_line_segments = match (&checkpoint, previous_lines) {
            (Some(checkpoint), Some(pulled))
                if pulled.segments.len() >= checkpoint.line_segments =>
            {
                let mut segments = pulled.segments;
                let base = segments
                    .drain(..checkpoint.line_segments)
                    .collect::<Vec<_>>();
                base
            }
            _ => Vec::new(),
        };
        let mut reusable_mesh_segments = match (&checkpoint, previous_mesh) {
            (Some(checkpoint), Some(pulled))
                if pulled.segments.len() >= checkpoint.mesh_segments =>
            {
                let mut segments = pulled.segments;
                let base = segments
                    .drain(..checkpoint.mesh_segments)
                    .collect::<Vec<_>>();
                base
            }
            _ => Vec::new(),
        };
        // Diagnostics + the base ranges an established checkpoint would record.
        let mut line_base_ranges: Vec<(usize, usize)> = Vec::new();
        let mut mesh_base_ranges: Vec<(usize, usize)> = Vec::new();
        // A class that owns resources must have retained *all* of its base
        // segments; a class with no resources requires none. At least one pulled
        // class must be retained.
        let mut reuse_failed = checkpoint.as_ref().is_some_and(|checkpoint| {
            (checkpoint.instance_streams == 0 && checkpoint.mesh_chunks == 0)
                || (checkpoint.instance_streams > 0
                    && reusable_line_segments.len() < checkpoint.line_segments)
                || (checkpoint.mesh_chunks > 0
                    && reusable_mesh_segments.len() < checkpoint.mesh_segments)
        });
        if reuse_failed {
            reusable_line_segments.clear();
            reusable_mesh_segments.clear();
        }

        // --- accelerated resource build (error-scoped) ---------------------
        //
        // The whole build runs inside **two** error scopes, and the reason is
        // measured on this backend, not theoretical.
        //
        // `create_buffer` cannot fail through its return value: when the driver
        // refuses an allocation (out of memory) wgpu 30.0.1 hands back an
        // **invalid** buffer handle. The failure then arrives as a *validation*
        // error on the next use of that handle (`Buffer with
        // 'cad-pulled-line-storage' label is invalid`), and an
        // `ErrorFilter::OutOfMemory` scope does **not** capture it — it reached
        // wgpu's default handler and aborted the process (exit 101). So:
        //
        // * `Validation` is the filter that actually sees the failure this class
        //   of failure is reported as, plus the bind-group / binding-size
        //   validation errors of the resources built here;
        // * `OutOfMemory` still captures a genuine create-time OOM, which
        //   wgpu-core maps to `ErrorType::OutOfMemory` (wgpu-core-30.0.1
        //   `src/device/resource.rs:712-717` → `WebGpuError::webgpu_error_type`),
        //   so the two are reported as distinct verdicts instead of one generic
        //   failure.
        //
        // wgpu routes each error to the innermost scope with a **matching**
        // filter (`wgpu-30.0.1/src/backend/wgpu_core.rs:657-682`), so nesting the
        // two captures both without shadowing. Push order is Validation then
        // OutOfMemory; both are popped in reverse (LIFO) on every path — success,
        // per-resource abort, and captured error — and `ErrorScopeGuard` also
        // pops on drop, so an unexpected early return cannot leave the stack
        // unbalanced.
        //
        // The scope pair alone is not enough: the invalid handle must not be
        // *used* before the attempt is abandoned, because a write/bind on it is
        // the very thing that reports the failure. Every allocation is therefore
        // individually guarded (`allocate_storage_buffer` / `guarded_bind_group`)
        // and the build closure below aborts on the first one that fails.
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        // Largest single allocation a class needed but could not segment (see
        // `PulledState::SkippedUnsegmentable`), if any.
        let mut unsegmentable: Option<(u64, u64)> = None;
        // One attempt: `Err` aborts the pull at the first failed resource, before
        // anything binds or copies that resource, and leaves every partially
        // built class to be dropped below.
        let build: Result<(), PulledAllocationError> = (|| {
            if !self.instance_streams.is_empty() {
                let table_bytes = vec![0u8; self.instance_streams.len() * 16];
                let max_elems = (limit / u64::from(PULLED_LINE_STRIDE)).max(1) as usize;
                // Test hook: shrink the per-segment element cap so the multi-segment
                // path can be exercised on a small scene without allocating a real
                // binding-sized stream.
                let segment_elems = self
                    .pulled_line_segment_elems_override
                    .unwrap_or(max_elems)
                    .max(1);
                // A single stream larger than a binding cannot be segmented without
                // splitting one of its runs across two buffers (per-run segment
                // mapping is not implemented), so the class falls back to the
                // universal per-run path rather than emit an oversized binding that
                // would fail validation.
                let oversized = self
                    .instance_streams
                    .iter()
                    .any(|stream| stream.pulled_instances.len() > segment_elems);
                if !oversized && table_bytes.len() as u64 <= limit {
                    let chunk_table =
                        Self::try_storage_buffer(&device, Some(&queue), &table_bytes)?;
                    // Segment boundaries fall on whole streams, so a run never
                    // straddles two buffers. The segment buffer is the per-run
                    // instance vertex buffer too, so the accelerated tier keeps one
                    // copy of the line geometry.
                    let counts: Vec<usize> = self
                        .instance_streams
                        .iter()
                        .map(|stream| stream.pulled_instances.len())
                        .collect();
                    // A segment never straddles the retained base boundary, so the
                    // base ranges (and therefore the retained geometry buffers) are
                    // identical across suffix commits.
                    let line_boundary =
                        reuse_line_boundary.filter(|b| *b > 0 && *b <= counts.len());
                    let ranges =
                        Self::forced_boundary_ranges(&counts, segment_elems, line_boundary);
                    // Base ranges are exactly those ending at or before the boundary.
                    let base_range_count = Self::base_range_count(&ranges, line_boundary);
                    // The recomputed base ranges must match the checkpoint's recorded
                    // ranges exactly (boundary stability).
                    let ranges_match = checkpoint.as_ref().is_some_and(|checkpoint| {
                        checkpoint.line_base_ranges.len() == base_range_count
                            && checkpoint.line_base_ranges.as_slice() == &ranges[..base_range_count]
                    });
                    // Verify the reuse plan against the recomputed ranges; any doubt
                    // falls back to a full rebuild (never a skip).
                    if !reusable_line_segments.is_empty()
                        && (!ranges_match
                            || base_range_count != reusable_line_segments.len()
                            || reusable_line_segments.iter().zip(ranges.iter()).any(
                                |(segment, (a, b))| {
                                    let expected: u32 = self.instance_streams[*a..*b]
                                        .iter()
                                        .map(|stream| stream.instance_count)
                                        .sum();
                                    segment.instance_count != expected
                                },
                            ))
                    {
                        reusable_line_segments.clear();
                        reuse_failed = true;
                    }
                    let mut reuse_line = std::mem::take(&mut reusable_line_segments).into_iter();
                    let base_range_count = if reuse_failed { 0 } else { base_range_count };
                    let mut segments = Vec::new();
                    for (index, (a, b)) in ranges.iter().copied().enumerate() {
                        if index < base_range_count {
                            if let Some(mut segment) = reuse_line.next() {
                                // Reuse the retained base geometry buffer; only the
                                // chunk table binding changed, so just recreate the
                                // bind group.
                                segment.bind_group = Self::guarded_bind_group(
                                    &device,
                                    &wgpu::BindGroupDescriptor {
                                        label: Some("cad-pulled-line-storage-bind"),
                                        layout: &line_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: segment
                                                    .geometry_buffer
                                                    .as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: chunk_table.as_entire_binding(),
                                            },
                                        ],
                                    },
                                )?;
                                let mut base = 0u32;
                                for stream in self.instance_streams[a..b].iter_mut() {
                                    stream.base = base;
                                    base += stream.instance_count;
                                    stream.buffer = segment.geometry_buffer.clone();
                                }
                                line_base_ranges.push((a, b));
                                segments.push(segment);
                                continue;
                            }
                        }
                        let streams: Vec<&[LineInstance]> = self.instance_streams[a..b]
                            .iter()
                            .map(|stream| stream.pulled_instances.as_slice())
                            .collect();
                        let records = build_pulled_lines(&streams, a as u32);
                        // Every resident stream is non-empty by construction, so a
                        // segment always holds at least one record. Assert that
                        // instead of silently skipping a segment, which would leave
                        // its streams pointing at the prepare-time placeholder.
                        debug_assert!(
                            !records.is_empty(),
                            "a pulled line segment must hold at least one instance"
                        );
                        let bytes = pulled_line_bytes(&records);
                        let buffer =
                            Self::try_vertex_storage_buffer(&device, Some(&queue), &bytes)?;
                        let bind_group = Self::guarded_bind_group(
                            &device,
                            &wgpu::BindGroupDescriptor {
                                label: Some("cad-pulled-line-storage-bind"),
                                layout: &line_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: buffer.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: chunk_table.as_entire_binding(),
                                    },
                                ],
                            },
                        )?;
                        // Point each stream in the segment at the shared buffer.
                        let mut base = 0u32;
                        for stream in self.instance_streams[a..b].iter_mut() {
                            stream.base = base;
                            base += stream.instance_count;
                            stream.buffer = buffer.clone();
                        }
                        segments.push(PulledLineSegment {
                            bind_group,
                            instance_count: records.len() as u32,
                            geometry_buffer: buffer,
                        });
                    }
                    if !segments.is_empty() {
                        table_bytes_uploaded += table_bytes.len() as u64;
                        self.pulled_lines = Some(PulledLines {
                            segments,
                            chunk_table,
                        });
                        // A full warm-up that survives a prefix records the base
                        // ranges it just built (a reuse records them per range above).
                        if establish.is_some() {
                            line_base_ranges = ranges[..base_range_count].to_vec();
                        }
                    }
                } else {
                    // The class cannot be segmented below the binding limit (a single
                    // stream, or the chunk table itself, is larger than one binding).
                    // Reported explicitly; never built oversized.
                    let largest_stream = self
                        .instance_streams
                        .iter()
                        .filter(|stream| stream.pulled_instances.len() > segment_elems)
                        .map(|stream| {
                            stream.pulled_instances.len() as u64
                                * std::mem::size_of::<PulledLine>() as u64
                        })
                        .max()
                        .unwrap_or(0);
                    unsegmentable = Some((largest_stream.max(table_bytes.len() as u64), limit));
                }
                if self.pulled_lines.is_none() {
                    // The whole-scene pulled line class could not be built: give
                    // every stream a real `VERTEX | STORAGE` buffer (64-byte pulled
                    // layout, matching the accelerated line pipeline) so the
                    // universal per-run fallback draws actual geometry rather than
                    // the prepare-time placeholder.
                    self.build_line_fallback(&device, &queue)?;
                }
            }

            // --- whole-scene merged-mesh storage ------------------------------
            if !self.mesh_chunks.is_empty() {
                let table_bytes = vec![0u8; self.mesh_chunks.len() * 16];
                let max_verts = (limit / u64::from(PULLED_MESH_VERTEX_STRIDE)).max(1) as usize;
                let max_indices = (limit / 4).max(1) as usize;
                // A single chunk larger than a binding is not segmentable; leave the
                // class on the universal path rather than emit an invalid binding.
                let oversized = self.mesh_chunks.iter().any(|chunk| {
                    chunk.pulled_vertices.len() > max_verts
                        || chunk.pulled_indices.len() > max_indices
                });
                if !oversized && table_bytes.len() as u64 <= limit {
                    let chunk_table =
                        Self::try_storage_buffer(&device, Some(&queue), &table_bytes)?;
                    // Greedy segmentation over whole chunks (a triangle never spans
                    // chunks, so a segment's indices reference only its own
                    // vertices), forced to break at the retained base boundary so the
                    // base chunk ranges (and their retained buffers) are stable.
                    let count = self.mesh_chunks.len();
                    let mesh_boundary = reuse_mesh_boundary.filter(|b| *b > 0 && *b <= count);
                    let chunk_vertices_len: Vec<usize> = self
                        .mesh_chunks
                        .iter()
                        .map(|chunk| chunk.pulled_vertices.len())
                        .collect();
                    let chunk_indices_len: Vec<usize> = self
                        .mesh_chunks
                        .iter()
                        .map(|chunk| chunk.pulled_indices.len())
                        .collect();
                    let ranges = Self::forced_boundary_chunk_ranges(
                        &chunk_vertices_len,
                        &chunk_indices_len,
                        max_verts,
                        max_indices,
                        mesh_boundary,
                    );
                    let base_range_count = Self::base_range_count(&ranges, mesh_boundary);
                    // The recomputed base ranges must match the checkpoint's recorded
                    // ranges exactly (boundary stability).
                    let ranges_match = checkpoint.as_ref().is_some_and(|checkpoint| {
                        checkpoint.mesh_base_ranges.len() == base_range_count
                            && checkpoint.mesh_base_ranges.as_slice() == &ranges[..base_range_count]
                    });
                    // Verify the reuse plan against the recomputed ranges; any doubt
                    // falls back to a full rebuild (never a skip).
                    if !reusable_mesh_segments.is_empty()
                        && (!ranges_match
                            || base_range_count != reusable_mesh_segments.len()
                            || reusable_mesh_segments.iter().zip(ranges.iter()).any(
                                |(segment, (a, b))| {
                                    let expected_vertices: usize = self.mesh_chunks[*a..*b]
                                        .iter()
                                        .map(|chunk| chunk.pulled_vertices.len())
                                        .sum();
                                    segment.vertex_count as usize != expected_vertices
                                },
                            ))
                    {
                        reusable_mesh_segments.clear();
                        reuse_failed = true;
                    }
                    let mut reuse_mesh = std::mem::take(&mut reusable_mesh_segments).into_iter();
                    let base_range_count = if reuse_failed { 0 } else { base_range_count };
                    let mut segments = Vec::new();
                    for (index, (start, end)) in ranges.iter().copied().enumerate() {
                        if index < base_range_count {
                            if let Some(mut segment) = reuse_mesh.next() {
                                // Reuse the retained base vertex/index buffers; only
                                // the chunk table binding changed.
                                segment.bind_group = Self::guarded_bind_group(
                                    &device,
                                    &wgpu::BindGroupDescriptor {
                                        label: Some("cad-pulled-mesh-storage-bind"),
                                        layout: &mesh_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: segment.vertex_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: segment.index_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: chunk_table.as_entire_binding(),
                                            },
                                        ],
                                    },
                                )?;
                                mesh_base_ranges.push((start, end));
                                segments.push(segment);
                                continue;
                            }
                        }
                        let mut chunk_vertices: Vec<&[MeshVertex]> =
                            Vec::with_capacity(end - start);
                        let mut chunk_indices: Vec<(&[u32], u32, bool)> =
                            Vec::with_capacity(end - start);
                        for chunk in &self.mesh_chunks[start..end] {
                            chunk_vertices.push(chunk.pulled_vertices.as_slice());
                            chunk_indices.push((
                                chunk.pulled_indices.as_slice(),
                                chunk.vertex_count,
                                chunk.mirrored,
                            ));
                        }
                        let vertices = build_pulled_mesh_vertices(&chunk_vertices, start as u32);
                        let (seg_indices, mirror_split) =
                            build_pulled_mesh_indices_with_bases(&chunk_indices);
                        if !vertices.is_empty() && !seg_indices.is_empty() {
                            let vertex_bytes = pulled_mesh_vertex_bytes(&vertices);
                            let index_bytes: Vec<u8> =
                                seg_indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                            let vertex_buffer =
                                Self::try_storage_buffer(&device, Some(&queue), &vertex_bytes)?;
                            let index_buffer =
                                Self::try_storage_buffer(&device, Some(&queue), &index_bytes)?;
                            let bind_group = Self::guarded_bind_group(
                                &device,
                                &wgpu::BindGroupDescriptor {
                                    label: Some("cad-pulled-mesh-storage-bind"),
                                    layout: &mesh_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: vertex_buffer.as_entire_binding(),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: index_buffer.as_entire_binding(),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 2,
                                            resource: chunk_table.as_entire_binding(),
                                        },
                                    ],
                                },
                            )?;
                            segments.push(PulledMeshSegment {
                                bind_group,
                                vertex_count: vertices.len() as u32,
                                index_count: seg_indices.len() as u32,
                                mirror_split,
                                vertex_buffer,
                                index_buffer,
                            });
                        }
                    }
                    if !segments.is_empty() {
                        table_bytes_uploaded += table_bytes.len() as u64;
                        self.pulled_mesh = Some(PulledMesh {
                            segments,
                            chunk_table,
                        });
                        // A full warm-up that survives a prefix records the base chunk
                        // ranges it just built (a reuse records them above).
                        if establish.is_some() {
                            mesh_base_ranges = ranges[..base_range_count].to_vec();
                        }
                    }
                } else {
                    // As for the line class: not segmentable below the binding limit.
                    let largest_chunk = self
                        .mesh_chunks
                        .iter()
                        .filter(|chunk| {
                            chunk.pulled_vertices.len() > max_verts
                                || chunk.pulled_indices.len() > max_indices
                        })
                        .map(|chunk| {
                            (chunk.pulled_vertices.len() as u64
                                * std::mem::size_of::<PulledMeshVertex>() as u64)
                                .max(chunk.pulled_indices.len() as u64 * 4)
                        })
                        .max()
                        .unwrap_or(0);
                    unsegmentable = Some((largest_chunk.max(table_bytes.len() as u64), limit));
                }
            }
            Ok(())
        })();
        // Pop the scopes LIFO (OutOfMemory was pushed last). Both pops happen on
        // every path, including success, so the stack stays balanced; a captured
        // error is the device's own report of why the attempt failed.
        let oom_captured = pop_captured_error(oom_scope);
        let validation_captured = pop_captured_error(validation_scope);
        // Precedence: a real captured error names the root cause (and an OOM is
        // the root cause of a follow-on "invalid buffer" validation error), then
        // the injected test hook, then the first per-resource abort.
        let failure = match allocation_verdict(
            validation_captured,
            oom_captured,
            self.injected_pulled_oom().map(str::to_string),
        ) {
            PulledAllocationVerdict::Skip(error) => Some(error),
            PulledAllocationVerdict::Proceed => build.err(),
        };
        if let Some(mut error) = failure {
            // Discard the partial pulled state; the partial class Vecs are already
            // gone, and stream/bind-group handles from the aborted attempt die with
            // them (nothing draws between commit and the next frame).
            self.pulled_lines = None;
            self.pulled_mesh = None;
            self.invalidate_pulled_prefix();
            // The universal per-run path needs real geometry (see
            // `build_line_fallback`). It is guarded as well and *reported* rather
            // than panicked: a device that cannot allocate the fallback either is
            // part of the same explicit verdict, never an empty success.
            if let Err(fallback) = self.build_line_fallback(&device, &queue) {
                error = error.with_note(&format!(
                    "the universal per-run fallback also failed: {}",
                    fallback.message()
                ));
            }
            self.pulled_state = pulled_skip_state(required_bytes, budget_bytes, error);
            return;
        }
        self.uploaded_bytes += table_bytes_uploaded;

        // (Re)establish or invalidate the retained-prefix checkpoint.
        // Final completion proof: the resource counts can match while a class
        // still fell back (an oversized suffix stream, or a chunk table past the
        // binding limit, drops the pulled class), which must NOT count as a reuse.
        if let Some(checkpoint) = checkpoint.as_ref() {
            if !Self::reuse_completion(
                checkpoint,
                self.pulled_lines.is_some(),
                self.pulled_mesh.is_some(),
                &line_base_ranges,
                &mesh_base_ranges,
            ) {
                reuse_failed = true;
            }
        }
        if reuse_failed {
            self.invalidate_pulled_prefix();
        }
        match prefix_outcome(is_reuse, reuse_failed, establish) {
            PrefixOutcome::CountReuse => self.pulled_prefix_reuses += 1,
            PrefixOutcome::Establish => {
                if let Some(prefix) = establish {
                    self.establish_pulled_prefix(prefix, line_base_ranges, mesh_base_ranges);
                }
            }
            PrefixOutcome::Invalidate => self.invalidate_pulled_prefix(),
        }

        // Report what the rebuild actually produced. `Active` requires at least
        // one resident converted class to be pulled; a class that exists but could
        // not be segmented is reported as such, never as success (and never as
        // "no converted geometry").
        let has_line_class = !self.instance_streams.is_empty();
        let has_mesh_class = !self.mesh_chunks.is_empty();
        self.pulled_state = if !has_line_class && !has_mesh_class {
            PulledState::SkippedNoConvertedGeometry
        } else if (has_line_class && self.pulled_lines.is_none())
            || (has_mesh_class && self.pulled_mesh.is_none())
        {
            let (largest, binding) = unsegmentable.unwrap_or((required_bytes, limit));
            PulledState::SkippedUnsegmentable {
                largest_class_bytes: largest,
                binding_limit_bytes: binding,
            }
        } else {
            PulledState::Active
        };
    }

    /// Exact bytes one pulled rebuild would allocate for the resident scene.
    ///
    /// Collected from the real resident classes and reduced by
    /// [`pulled_payload_bytes`]; it is the requirement the pre-flight capacity
    /// gate compares against `pulled_budget_bytes`.
    pub(super) fn pulled_required_bytes(&self) -> u64 {
        let line_instances: Vec<usize> = self
            .instance_streams
            .iter()
            .map(|stream| stream.pulled_instances.len())
            .collect();
        let mesh_vertices: Vec<usize> = self
            .mesh_chunks
            .iter()
            .map(|chunk| chunk.pulled_vertices.len())
            .collect();
        let mesh_indices: Vec<usize> = self
            .mesh_chunks
            .iter()
            .map(|chunk| chunk.pulled_indices.len())
            .collect();
        pulled_payload_bytes(&line_instances, &mesh_vertices, &mesh_indices)
    }

    /// The memory budget the whole pulled organisation of one rebuild may claim.
    ///
    /// `pulled_memory_budget_override` (test hook) wins; otherwise the budget is
    /// [`pulled_budget_for`] of the live device limits.
    pub(super) fn pulled_budget_bytes(&self, limits: &wgpu::Limits) -> u64 {
        self.pulled_memory_budget_override
            .unwrap_or_else(|| pulled_budget_for(limits))
    }

    /// The test-only injected out-of-memory message, if any (production builds
    /// always return `None`).
    pub(super) fn injected_pulled_oom(&self) -> Option<&'static str> {
        #[cfg(test)]
        {
            self.pulled_oom_injection
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    /// Record a retained-prefix checkpoint after a full rebuild that kept a base
    /// prefix. Only established when the prefix actually references resources and
    /// both classes produced reusable base segments.
    pub(super) fn establish_pulled_prefix(
        &mut self,
        prefix: usize,
        line_base_ranges: Vec<(usize, usize)>,
        mesh_base_ranges: Vec<(usize, usize)>,
    ) {
        let prefix = prefix.min(self.batches.len());
        let Some(instance_streams) = Self::owned_prefix_count(
            &Self::instanced_sequence(&self.batches[..prefix]),
            &Self::instanced_sequence(&self.batches[prefix..]),
            self.instance_streams.len(),
        ) else {
            self.invalidate_pulled_prefix();
            return;
        };
        let Some(mesh_chunks) = Self::owned_prefix_count(
            &Self::mesh_sequence(&self.batches[..prefix]),
            &Self::mesh_sequence(&self.batches[prefix..]),
            self.mesh_chunks.len(),
        ) else {
            self.invalidate_pulled_prefix();
            return;
        };
        let Some(arc_streams) = Self::owned_prefix_count(
            &Self::arc_sequence(&self.batches[..prefix]),
            &Self::arc_sequence(&self.batches[prefix..]),
            self.arc_streams.len(),
        ) else {
            self.invalidate_pulled_prefix();
            return;
        };
        // At least one *pulled* class must own a retained prefix (arcs have no
        // pulled variant).
        if instance_streams == 0 && mesh_chunks == 0 {
            self.invalidate_pulled_prefix();
            return;
        }
        // A class that fell back to the universal path (no pulled segments) cannot
        // be reused, so a checkpoint with base content but no base segments is
        // refused outright.
        if (instance_streams > 0 && line_base_ranges.is_empty())
            || (mesh_chunks > 0 && mesh_base_ranges.is_empty())
        {
            self.invalidate_pulled_prefix();
            return;
        }
        self.pulled_prefix_rebuilds += 1;
        self.pulled_prefix_checkpoint = Some(PulledPrefixCheckpoint {
            device_generation: self.device_generation,
            cache_epoch: self.pulled_cache_epoch,
            retained_batches: prefix,
            instance_streams,
            mesh_chunks,
            arc_streams,
            line_segments: line_base_ranges.len(),
            mesh_segments: mesh_base_ranges.len(),
            line_base_ranges,
            mesh_base_ranges,
        });
    }

    /// Diagnostics: suffix commits that reused the retained pulled base.
    pub fn pulled_prefix_reuses(&self) -> u64 {
        self.pulled_prefix_reuses
    }

    /// Diagnostics: full pulled rebuilds that (re)established a checkpoint.
    pub fn pulled_prefix_rebuilds(&self) -> u64 {
        self.pulled_prefix_rebuilds
    }
}
