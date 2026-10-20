use super::*;

impl Renderer {
    // Upload CPU batches as static GPU buffers (once per change).
    //
    // Mesh batches get an index buffer and a repaired normal buffer; line
    // batches keep the position-only layout.
    pub fn upload(&mut self, delta: &SceneDelta) -> CadResult<()> {
        let prepared = self.prepare_upload(delta)?;
        self.commit_upload(prepared, self.batches.len())?;
        // Wipeout masks are a separate resident set, exactly like images: a full
        // scene upload replaces them wholesale (ADR 0022). Hosts that stage and
        // commit geometry themselves call `upload_masks` on their base change.
        self.upload_masks(&delta.masks)
    }

    /// Build resources without changing the active scene. Synchronous input and
    /// device-limit validation precedes allocation; async driver failures remain
    /// covered by the render error scope/device lifecycle, not this return value.
    pub fn prepare_upload(&self, delta: &SceneDelta) -> CadResult<PreparedUpload> {
        self.prepare_upload_batches(&delta.added)
    }

    /// Stage a bounded borrowed batch slice without cloning a full scene delta.
    /// Active GPU batches are unchanged until explicit atomic commit.
    pub fn prepare_upload_batches(
        &self,
        batches: &[cad_scene::RenderBatch],
    ) -> CadResult<PreparedUpload> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        let started = Instant::now();
        let mut staged = Vec::with_capacity(batches.len());
        let mut uploaded_bytes = 0;
        // Which batches convert to the instanced line path and which to the
        // flat-merge mesh path this upload. Computed before validation so the
        // device-limit check uses the true upload stride per path.
        let line_converted = convert_line_batches(batches, self.line_instancing, self.mesh_merging);
        let mesh_converted: Vec<usize> = if self.mesh_merging {
            batches
                .iter()
                .enumerate()
                .filter(|(_, batch)| mesh_batch_is_mergeable(batch))
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };
        let mut converted_mask = vec![false; batches.len()];
        for source in &line_converted {
            converted_mask[source.batch] = true;
        }
        for &index in &mesh_converted {
            converted_mask[index] = true;
        }
        // SDF conic batches (slice B): every opaque `Conics` batch converts.
        let arc_converted = convert_arc_batches(batches);
        for source in &arc_converted {
            converted_mask[source.batch] = true;
        }
        // Allocate one renderer pick key per staged batch (see `PickTable`). The
        // keys are monotonic within a device generation and never reused, so
        // independently staged pieces (see `PreparedUpload::append`) cannot
        // collide even though neither knows the other's ordinal. Exhaustion is
        // reported as an invalid-input error rather than silently wrapping onto a
        // key that would resolve to the wrong entity.
        let mut keys = Vec::with_capacity(batches.len());
        for _ in 0..batches.len() {
            let key = self.pick_key_counter.fetch_add(1, Ordering::Relaxed);
            if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
                return Err(CadError::InvalidInput(
                    "renderer pick key space exhausted (too many batches uploaded \
                     without a full scene clear)"
                        .into(),
                ));
            }
            keys.push(key);
        }
        let cap = self.max_instances_per_stream.max(1);
        let mesh_cap = self.max_vertices_per_mesh_stream.max(3);
        let max_buffer_size = device.limits().max_buffer_size;
        for (index, batch) in batches.iter().enumerate() {
            // Conservative *estimates* of the largest single buffer this batch can
            // produce, used only as an early gate. A converted batch uploads
            // 56-byte instances (one per segment) or 44-byte merged vertices (one
            // per stored triangle vertex) and 4-byte merged indices, never the
            // legacy 12 B/vertex; a batch over a cap is split, so the largest
            // single vertex/instance buffer stays within the cap. These bounds are
            // deliberately loose (an overlay's segment count uses the
            // `3·triangles/2` edge bound, not `vertices/2`); the *authoritative*
            // size checks are the per-stream/per-chunk guards at upload below
            // (`bytes.len() > max_buffer_size`), which see the real buffer
            // payloads after the cap split.
            let vertex_bytes: u64 = if converted_mask[index] {
                let instance_bytes = if self.mesh_merging && mesh_batch_is_mergeable(batch) {
                    batch
                        .vertices
                        .len()
                        .min(mesh_cap)
                        .saturating_mul(MESH_VERTEX_STRIDE as usize) as u64
                } else {
                    0
                };
                // A `Lines` batch has `vertices/2` segments; a mesh edge overlay
                // has one segment per edge and `edges <= 3·triangles` before
                // dedup, so `3·triangles/2` upper-bounds its segments. Take the
                // larger bound.
                let segments =
                    (batch.vertices.len() / 2).max(batch.indices.len().saturating_mul(3) / 2);
                let line_bytes = segments
                    .min(cap)
                    .saturating_mul(LINE_INSTANCE_STRIDE as usize)
                    as u64;
                instance_bytes + line_bytes
            } else {
                (batch.vertices.len() as u64).saturating_mul(12)
            };
            if batch.topology == RenderTopology::Conics
                && classify_alpha(batch.alpha) == AlphaClass::Transparent
            {
                // The SDF conic pipeline is opaque-only and `draw_batch` has no
                // conic arm, so a transparent conic batch would be drawn by the
                // transparent pass and silently rasterise nothing. Reject it
                // loudly (iron rule #2: never a silent drop); the provider never
                // emits one today. (An `alpha <= 0` conic is already reported as
                // invisible and is not drawn at all.)
                return Err(CadError::InvalidInput(
                    "transparent conic batch has no supported draw path".into(),
                ));
            }
            if batch.vertices.len() > u32::MAX as usize
                || batch.indices.len() > u32::MAX as usize / 3
                || vertex_bytes > max_buffer_size
                || batch.indices.len().saturating_mul(24) as u64 > max_buffer_size
                || batch
                    .indices
                    .iter()
                    .flatten()
                    .any(|i| *i as usize >= batch.vertices.len())
                || batch.vertices.iter().flatten().any(|v| !v.is_finite())
                // A conic batch carries f64 semantic records, not vertices; guard
                // its own buffer size and finiteness.
                || (batch.topology == RenderTopology::Conics
                    && ((batch.conics.len() as u64)
                        .saturating_mul(ARC_INSTANCE_STRIDE as u64)
                        > max_buffer_size
                        || batch.conics.iter().any(|conic| {
                            !(conic.center.x.is_finite()
                                && conic.center.y.is_finite()
                                && conic.center.z.is_finite()
                                && conic.major.x.is_finite()
                                && conic.major.y.is_finite()
                                && conic.major.z.is_finite()
                                && conic.minor.x.is_finite()
                                && conic.minor.y.is_finite()
                                && conic.minor.z.is_finite()
                                && conic.start.is_finite()
                                && conic.sweep.is_finite())
                        })))
            {
                return Err(CadError::InvalidInput(
                    "invalid or over-limit GPU batch".into(),
                ));
            }
        }

        // --- instanced line streams -----------------------------------------
        //
        // Convert eligible opaque `LineList` batches (and the edge overlays of
        // merged meshes) into static instance streams here, at upload. Each
        // stream is a spatially bounded chunk with its own `f64` origin (see
        // `build_instance_chunks`); segments are packed in original batch order,
        // so no re-sorting happens here. The per-instance pick key is
        // first-class (see `LineInstance`); this slice does not yet consume the
        // flags.
        let mut instance_streams: Vec<InstanceStream> = Vec::new();
        let mut batch_runs: Vec<Option<Vec<InstanceRun>>> = vec![None; batches.len()];
        let accelerated = self.render_tier == RenderTier::Accelerated;
        if !line_converted.is_empty() {
            let (chunks, runs) = build_instance_chunks(
                batches,
                &line_converted,
                &keys,
                self.max_instances_per_stream,
                MAX_STREAM_RADIUS,
            );
            batch_runs = runs;
            for chunk in chunks {
                // Each chunk is non-empty and within the cap, so its buffer is
                // non-zero and legal; guard the device limit defensively.
                let bytes = line_instance_bytes(&chunk.instances);
                if bytes.len() as u64 > max_buffer_size {
                    return Err(CadError::InvalidInput(
                        "invalid or over-limit GPU batch".into(),
                    ));
                }
                // On the accelerated tier the instance buffers are the
                // whole-scene `VERTEX | STORAGE` segments, rebuilt by
                // `rebuild_pulled` at commit (they must span every resident
                // stream, including a kept prefix). A placeholder keeps the
                // `InstanceStream` shape valid until then; nothing draws between
                // prepare and commit.
                let buffer = if accelerated {
                    Self::vertex_buffer(device, self.queue.as_ref(), &[0u8; 4])
                } else {
                    Self::vertex_buffer(device, self.queue.as_ref(), &bytes)
                };
                instance_streams.push(InstanceStream {
                    buffer,
                    instance_count: chunk.instances.len() as u32,
                    origin: chunk.origin,
                    aabb: chunk.aabb,
                    base: 0,
                    // Keep the packed instances so the accelerated tier's
                    // whole-scene pulled buffer can be rebuilt on every commit
                    // (including an overlay append that keeps a prefix). The
                    // universal tier keeps no CPU copy.
                    pulled_instances: if accelerated {
                        chunk.instances
                    } else {
                        Vec::new()
                    },
                });
            }
        }

        // --- merged mesh chunks ---------------------------------------------
        //
        // Concatenate adjacent same-pipeline opaque `Mesh` batches into one
        // interleaved vertex buffer per chunk and draw each contiguous run as one
        // indexed draw (see `build_mesh_chunks`). The per-batch tint is baked
        // into the per-vertex colour (bit-identical to the legacy per-pixel
        // product only for a constant-colour mesh or a white tint) and a
        // per-vertex `entity_flags` carries the first source's `EntityId` low
        // bits for a later picking phase (the legacy per-batch draw carried no
        // explicit pick key).
        let mut mesh_chunk_buffers: Vec<GpuMeshChunk> = Vec::new();
        let mut batch_mesh_runs: Vec<Option<Vec<MeshRun>>> = vec![None; batches.len()];
        if !mesh_converted.is_empty() {
            let (chunks, runs) = build_mesh_chunks(
                batches,
                &mesh_converted,
                &keys,
                self.max_vertices_per_mesh_stream,
                MAX_STREAM_RADIUS,
            );
            batch_mesh_runs = runs;
            for chunk in chunks {
                let vertex_bytes = mesh_vertex_bytes(&chunk.vertices);
                let index_bytes: Vec<u8> =
                    chunk.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                if vertex_bytes.len() as u64 > max_buffer_size
                    || index_bytes.len() as u64 > max_buffer_size
                {
                    return Err(CadError::InvalidInput(
                        "invalid or over-limit GPU batch".into(),
                    ));
                }
                let vertex_buffer = Self::vertex_buffer(device, self.queue.as_ref(), &vertex_bytes);
                let index_buffer = Self::index_buffer(device, self.queue.as_ref(), &chunk.indices);
                mesh_chunk_buffers.push(GpuMeshChunk {
                    vertex_buffer,
                    index_buffer,
                    vertex_count: chunk.vertices.len() as u32,
                    index_count: chunk.indices.len() as u32,
                    origin: chunk.origin,
                    aabb: chunk.aabb,
                    mirrored: chunk.mirrored,
                    // CPU copies for the accelerated whole-scene rebuild (see
                    // `InstanceStream::pulled_instances`).
                    pulled_vertices: if accelerated {
                        chunk.vertices
                    } else {
                        Vec::new()
                    },
                    pulled_indices: if accelerated {
                        chunk.indices
                    } else {
                        Vec::new()
                    },
                });
            }
        }

        // --- SDF conic streams (slice B) ------------------------------------
        //
        // Pack eligible circle/arc batches into static SDF instance streams here,
        // at upload (see `build_arc_chunks`). Each stream is a spatially bounded
        // chunk with its own `f64` origin; conics are packed in original batch
        // order, so no re-sorting happens. The per-instance pick key is
        // first-class (see `ArcInstance`). Universal-tier only this round.
        let mut arc_streams: Vec<ArcStream> = Vec::new();
        let mut batch_arc_runs: Vec<Option<Vec<ArcRun>>> = vec![None; batches.len()];
        let mut arc_degenerate_skips = 0u64;
        if !arc_converted.is_empty() {
            let (chunks, runs, skipped) = build_arc_chunks(
                batches,
                &arc_converted,
                &keys,
                self.max_instances_per_stream,
                MAX_STREAM_RADIUS,
            );
            batch_arc_runs = runs;
            arc_degenerate_skips = skipped;
            for chunk in chunks {
                let bytes = arc_instance_bytes(&chunk.instances);
                if bytes.len() as u64 > max_buffer_size {
                    return Err(CadError::InvalidInput(
                        "invalid or over-limit GPU batch".into(),
                    ));
                }
                arc_streams.push(ArcStream {
                    buffer: Self::vertex_buffer(device, self.queue.as_ref(), &bytes),
                    instance_count: chunk.instances.len() as u32,
                    origin: chunk.origin,
                    aabb: chunk.aabb,
                    base: 0,
                });
            }
        }

        let mut line_odd_vertex_skips = 0u64;
        for (index, batch) in batches.iter().enumerate() {
            let run = batch_runs[index].take();
            let mesh_run = batch_mesh_runs[index].take();
            let arc_run = batch_arc_runs[index].take();
            let instanced = run.is_some();
            let merged = mesh_run.is_some();
            let arc_instanced = arc_run.is_some();
            // A trailing unpaired vertex of a `Lines`/`MeshEdges` batch is dropped
            // by every path (a `LineList` cannot pair it). Count it so the drop is
            // reported, never silent.
            if matches!(
                batch.topology,
                RenderTopology::Lines | RenderTopology::MeshEdges
            ) {
                line_odd_vertex_skips += (batch.vertices.len() % 2) as u64;
            }
            // A converted batch owns no per-batch buffers: skip packing its
            // geometry entirely (its bytes live in the shared stream/chunk).
            // `line_quads` holds the pre-expanded anti-aliased line quads for a
            // legacy `Lines`/`MeshEdges` batch or a non-merged mesh's wireframe
            // overlay (empty for a converted batch).
            let (vertices, indices, normals, colors, line_quads) =
                if instanced || merged || arc_instanced {
                    Default::default()
                } else {
                    match batch.topology {
                        RenderTopology::Mesh => {
                            let verts = pack_positions(&batch.vertices);
                            let idx: Vec<u32> = batch
                                .indices
                                .iter()
                                .flat_map(|t| [t[0], t[1], t[2]])
                                .collect();
                            let normals = geometry::repaired_normals(batch);
                            // Always bound (white when the batch has no per-vertex
                            // colours), so the mesh pipeline's `@location(2)` is valid.
                            let colors = geometry::repaired_colors(batch);
                            let edges = geometry::mesh_edge_indices(batch);
                            let mut quads = Vec::new();
                            if !edges.is_empty() {
                                expand_line_quads(&mut quads, batch, Some(&edges));
                            }
                            (
                                verts,
                                idx,
                                pack_positions(&normals),
                                pack_positions(&colors),
                                line_quad_bytes(&quads),
                            )
                        }
                        // Line geometry is expanded into anti-aliased quads; no
                        // legacy position/index buffer is kept.
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
                        // A conic batch's geometry lives in the SDF instance streams
                        // (`arc_instanced` is always true here); it owns no per-batch
                        // buffers.
                        RenderTopology::Conics => Default::default(),
                    }
                };

            // A converted batch owns no per-batch vertex buffer; its exact upload
            // charge is the instance/merged/conic bytes it contributed (never the
            // legacy vertex bytes).
            let vertex_buffer = if instanced || merged || arc_instanced || vertices.is_empty() {
                None
            } else {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &vertices))
            };
            let is_mesh = batch.topology == RenderTopology::Mesh;
            // A merged mesh batch owns no per-batch index buffer, but its
            // triangle-index count must still drive the frame triangle budget,
            // so it is summed from the batch's merged runs.
            let triangle_index_count = if merged {
                mesh_run
                    .as_ref()
                    .map_or(0, |run| run.iter().map(|piece| piece.index_count).sum())
            } else {
                indices.len() as u32
            };
            let index_buffer = if indices.is_empty() {
                None
            } else {
                Some(Self::index_buffer(device, self.queue.as_ref(), &indices))
            };
            let line_quad_count = (line_quads.len() / LINE_QUAD_STRIDE as usize) as u32;
            let line_quad_buffer = if line_quads.is_empty() {
                None
            } else {
                Some(Self::vertex_buffer(
                    device,
                    self.queue.as_ref(),
                    &line_quads,
                ))
            };
            let normal_buffer = if is_mesh && !merged {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &normals))
            } else {
                None
            };
            let color_buffer = if is_mesh && !merged {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &colors))
            } else {
                None
            };

            // The exact bytes this batch uploads: its instance-stream share
            // (56 B per segment; 64 B on the accelerated tier, whose segment
            // buffer is also the pulled storage) and/or its merged-chunk share
            // (44 B per stored vertex + 4 B per triangle index), otherwise
            // positions/normals/colours 12 B per vertex, triangle indices 4 B each
            // and anti-aliased line quads 32 B per vertex. Kept in lockstep with
            // `GpuBatch::upload_size_bytes`, which the frame budget charges.
            //
            // On the accelerated tier the merged mesh also contributes to the
            // whole-scene pulled storage buffers (64 B per merged vertex, 4 B per
            // rebased index), charged here so the budget contract stays honest
            // rather than hiding the second copy.
            let line_stride = if accelerated {
                PULLED_LINE_STRIDE as usize
            } else {
                LINE_INSTANCE_STRIDE as usize
            };
            let line_bytes = run.as_ref().map_or(0, |run| {
                run.iter()
                    .map(|piece| piece.count as usize * line_stride)
                    .sum()
            });
            let mesh_bytes = mesh_run.as_ref().map_or(0, |run| {
                run.iter()
                    .map(|piece| {
                        piece.vertex_count as usize * MESH_VERTEX_STRIDE as usize
                            + piece.index_count as usize * 4
                    })
                    .sum()
            });
            let pulled_bytes = if accelerated && merged {
                mesh_run.as_ref().map_or(0, |run| {
                    run.iter()
                        .map(|piece| {
                            piece.vertex_count as usize * PULLED_MESH_VERTEX_STRIDE as usize
                                + piece.index_count as usize * 4
                        })
                        .sum::<usize>()
                })
            } else {
                0
            };
            let arc_bytes = arc_run.as_ref().map_or(0, |run| {
                run.iter()
                    .map(|piece| piece.count as usize * ARC_INSTANCE_STRIDE as usize)
                    .sum()
            });
            let bytes = if instanced || merged || arc_instanced {
                line_bytes + mesh_bytes + pulled_bytes + arc_bytes
            } else {
                normals.len() + vertices.len() + colors.len() + indices.len() * 4 + line_quads.len()
            };
            uploaded_bytes += bytes as u64;
            staged.push(GpuBatch {
                vertices: vertex_buffer,
                vertex_count: batch.vertices.len() as u32,
                normals: normal_buffer,
                colors: color_buffer,
                topology: batch.topology,
                indices: index_buffer,
                index_count: triangle_index_count,
                line_quads: line_quad_buffer,
                line_quad_count,
                mirrored: batch.mirrored,
                alpha: batch.alpha,
                color: batch.color,
                lineweight: batch.lineweight,
                draw_order: batch.draw_order,
                centroid: batch.centroid(),
                origin: [
                    batch.local_origin.x as f32,
                    batch.local_origin.y as f32,
                    batch.local_origin.z as f32,
                ],
                instanced: run.unwrap_or_default(),
                mesh_runs: mesh_run.unwrap_or_default(),
                arc_runs: arc_run.unwrap_or_default(),
                upload_bytes: bytes,
                pick_key: keys[index],
                sources: batch.sources.clone(),
            });
        }
        Ok(PreparedUpload {
            batches: staged,
            instances: instance_streams,
            arcs: arc_streams,
            arc_degenerate_skips,
            line_odd_vertex_skips,
            mesh_chunks: mesh_chunk_buffers,
            device_generation: self.device_generation,
            device: device.clone(),
            bytes: uploaded_bytes,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Atomically replace the suffix after `keep_prefix` (0 replaces the scene).
    /// This supports independently updating an annotation overlay while keeping
    /// base drawing buffers. A stale-device upload cannot be published.
    pub fn commit_upload(
        &mut self,
        mut prepared: PreparedUpload,
        keep_prefix: usize,
    ) -> CadResult<()> {
        if self.device.as_ref() != Some(&prepared.device)
            || prepared.device_generation != self.device_generation
            || self.device_lost
            || keep_prefix > self.batches.len()
        {
            return Err(CadError::StaleResult);
        }
        // `commit_started` covers the incoming reference collection and the
        // admission (prefix scans/allocs) below plus the staging/append/commit, so
        // `CommitTiming::total_us >= the sum of the substages` (it additionally
        // covers those prefix scans/allocs and the append). It begins **after** the
        // stale-validation guard above, which is therefore not attributed to any
        // substage or to `total_us` (negligible, and deliberately unclaimed).
        let commit_started = std::time::Instant::now();
        // The **new (incoming)** suffix's rebased resource references (for the
        // ownership proof): the prepared runs are rebased onto the current
        // resource counts.
        let incoming = IncomingResources {
            streams: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .instanced
                        .iter()
                        .map(|run| run.stream + self.instance_streams.len() as u32)
                })
                .collect(),
            meshes: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .mesh_runs
                        .iter()
                        .map(|run| run.chunk + self.mesh_chunks.len() as u32)
                })
                .collect(),
            arcs: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .arc_runs
                        .iter()
                        .map(|run| run.stream + self.arc_streams.len() as u32)
                })
                .collect(),
        };
        // Admission is computed **before** any mutation, from the renderer's own
        // retained state (never from the caller's `base_changed` alone).
        let fast = self.pulled_fastpath_plan(keep_prefix, &incoming);
        // Re-base the prepared streams' run indices onto the existing streams so
        // a kept prefix's converted batches keep pointing at valid buffers.
        let rebase_started = std::time::Instant::now();
        let base = self.instance_streams.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.instanced {
                run.stream += base;
            }
        }
        let mesh_base = self.mesh_chunks.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.mesh_runs {
                run.chunk += mesh_base;
            }
        }
        // Re-base the prepared conic runs onto the existing conic streams.
        let arc_base = self.arc_streams.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.arc_runs {
                run.stream += arc_base;
            }
        }
        let rebase_us = rebase_started.elapsed().as_micros() as u64;
        let append_started = std::time::Instant::now();
        self.instance_stream_uploads += prepared.instances.len() as u64;
        self.arc_stream_uploads += prepared.arcs.len() as u64;
        self.arc_degenerate_skips += prepared.arc_degenerate_skips;
        self.line_odd_vertex_skips += prepared.line_odd_vertex_skips;
        self.mesh_chunk_uploads += prepared.mesh_chunks.len() as u64;
        self.instance_streams.append(&mut prepared.instances);
        self.arc_streams.append(&mut prepared.arcs);
        self.mesh_chunks.append(&mut prepared.mesh_chunks);
        // The pick keys of the batches this commit drops (the replaced suffix):
        // the pick table retires exactly these entries incrementally instead of
        // rebuilding over the whole resident scene.
        let retired_pick_keys: Vec<u32> = self.batches[keep_prefix..]
            .iter()
            .map(|batch| batch.pick_key)
            .collect();
        self.batches.truncate(keep_prefix);
        self.batches.extend(prepared.batches);
        let append_us = append_started.elapsed().as_micros() as u64;
        let gc_started = std::time::Instant::now();
        self.gc_instance_streams();
        self.gc_arc_streams();
        self.gc_mesh_chunks();
        let gc_us = gc_started.elapsed().as_micros() as u64;
        let pulled_started = std::time::Instant::now();
        self.rebuild_pulled(fast);
        let pulled_us = pulled_started.elapsed().as_micros() as u64;
        let pick_started = std::time::Instant::now();
        self.rebuild_pick_table(Some((keep_prefix, retired_pick_keys)))?;
        let pick_us = pick_started.elapsed().as_micros() as u64;
        self.progressive_frame = None;
        self.draw_plan = None;
        // Page-cost knowledge is scene-specific, so a commit that replaces the
        // scene resets the controller. A **suffix-only** commit (a true prefix is
        // kept: `keep_prefix > 0`, the same discriminator the pick table's
        // incremental path uses) only re-uploads the transient overlay range and
        // does not change the base draw-cost profile, so it must NOT reset:
        // otherwise a periodic overlay refresh (e.g. the deferred reference-grid
        // settle, once per camera settle) knocks the learned scale back to 1.0
        // every settle, the controller never converges, and pages stay
        // vertex-heavy. A base change (`keep_prefix == 0`) still resets, as do
        // `clear_batches` and a device generation change / device loss.
        //
        // This is a heuristic cost model, **not** correctness state: it is
        // deliberately not modelled as a dirty/invalidation flag, and nothing
        // else is coupled to the reset (the restart-pending flag is left as-is
        // here — a suffix commit does not reset the knowledge, so a pending
        // restart from an earlier real reset correctly stays pending).
        if keep_prefix == 0 {
            self.reset_pacing();
        }
        let lineweight_started = std::time::Instant::now();
        self.rebuild_lineweight_cache();
        let lineweight_us = lineweight_started.elapsed().as_micros() as u64;
        self.uploaded_bytes += prepared.bytes;
        self.last_upload_ms = Some(prepared.elapsed_ms);
        self.last_commit_timing = Some(CommitTiming {
            rebase_us,
            append_us,
            gc_us,
            pulled_us,
            pick_us,
            lineweight_us,
            total_us: commit_started.elapsed().as_micros() as u64,
        });
        Ok(())
    }

    /// Drop instance streams no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_instance_streams(&mut self) {
        if self.instance_streams.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.instance_streams.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.instanced {
                let slot = &mut map[run.stream as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.stream = new_index as u32;
            }
        }
        if next == self.instance_streams.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<InstanceStream>> = (0..next).map(|_| None).collect();
        for (old_index, stream) in std::mem::take(&mut self.instance_streams)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(stream);
            }
        }
        self.instance_streams = result
            .into_iter()
            .map(|stream| stream.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Drop SDF conic streams no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_arc_streams(&mut self) {
        if self.arc_streams.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.arc_streams.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.arc_runs {
                let slot = &mut map[run.stream as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.stream = new_index as u32;
            }
        }
        if next == self.arc_streams.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            return;
        }
        let mut result: Vec<Option<ArcStream>> = (0..next).map(|_| None).collect();
        for (old_index, stream) in std::mem::take(&mut self.arc_streams)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(stream);
            }
        }
        self.arc_streams = result
            .into_iter()
            .map(|stream| stream.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Drop merged mesh chunks no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_mesh_chunks(&mut self) {
        if self.mesh_chunks.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.mesh_chunks.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.mesh_runs {
                let slot = &mut map[run.chunk as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.chunk = new_index as u32;
            }
        }
        if next == self.mesh_chunks.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<GpuMeshChunk>> = (0..next).map(|_| None).collect();
        for (old_index, chunk) in std::mem::take(&mut self.mesh_chunks)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(chunk);
            }
        }
        self.mesh_chunks = result
            .into_iter()
            .map(|chunk| chunk.expect("referenced by a surviving batch"))
            .collect();
    }
}
