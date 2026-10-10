//! cache module.

use super::*;

/// Bounds and caching for the CPU-side scene.
pub struct SceneCache {
    pub budget: SceneBudget,
    chunks: BTreeMap<u64, RenderBatch>,
    next_chunk: u64,
    used_bytes: usize,
    /// Image draw items, in insertion order (oldest first for eviction).
    images: Vec<ImageBatch>,
    images_used_bytes: usize,
    /// Count of image draw items dropped by [`SceneCache::evict_images`] to stay
    /// under [`SceneBudget::image_bytes`]. This makes the eviction observable
    /// instead of a silent drop; it only ever increases.
    image_evictions: u64,
}

impl Default for SceneCache {
    fn default() -> Self {
        SceneCache::new(SceneBudget::default())
    }
}

impl SceneCache {
    pub fn new(budget: SceneBudget) -> Self {
        SceneCache {
            budget,
            chunks: BTreeMap::new(),
            next_chunk: 1,
            used_bytes: 0,
            images: Vec::new(),
            images_used_bytes: 0,
            image_evictions: 0,
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn chunks(&self) -> impl Iterator<Item = &RenderBatch> {
        self.chunks.values()
    }

    /// The cached image draw items, in insertion order.
    pub fn image_batches(&self) -> &[ImageBatch] {
        &self.images
    }

    /// Number of cached image draw items.
    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// CPU bytes the cached image draw items count toward
    /// [`SceneBudget::image_bytes`].
    pub fn image_used_bytes(&self) -> usize {
        self.images_used_bytes
    }

    /// Total image draw items evicted from the cache since construction to stay
    /// under [`SceneBudget::image_bytes`].
    ///
    /// Image batches dropped by [`SceneCache::evict_images`] (including the
    /// maintenance call at the end of [`SceneCache::publish`]) are counted here
    /// so a host can surface them instead of losing the draw silently. The
    /// counter is monotonic; it is not reset by eviction.
    pub fn image_evictions(&self) -> u64 {
        self.image_evictions
    }

    /// Remove batches that reference objects touched by `changes`.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> CadResult<()> {
        if changes.changes.is_empty() {
            return Ok(());
        }
        let mut dirty: std::collections::BTreeSet<EntityId> = std::collections::BTreeSet::new();
        for change in &changes.changes {
            match change {
                ObjectChange::Insert(id)
                | ObjectChange::Update(id, _)
                | ObjectChange::Delete(id) => {
                    // ObjectId and EntityId share the same numeric space during
                    // import; a finer mapping is a documented follow-up.
                    dirty.insert(EntityId(id.0));
                }
            }
        }
        let to_remove: Vec<u64> = self
            .chunks
            .iter()
            .filter(|(_, batch)| batch.sources.iter().any(|s| dirty.contains(&s.entity)))
            .map(|(k, _)| *k)
            .collect();
        for key in to_remove {
            self.remove_chunk(key);
        }
        // Image draw items are invalidated by the same dirty set as chunks.
        let mut image_bytes_removed = 0usize;
        self.images.retain(|image| {
            let keep = !image.sources.iter().any(|s| dirty.contains(&s.entity));
            if !keep {
                image_bytes_removed += image.approx_bytes();
            }
            keep
        });
        self.images_used_bytes = self.images_used_bytes.saturating_sub(image_bytes_removed);
        Ok(())
    }

    fn remove_chunk(&mut self, key: u64) {
        if let Some(batch) = self.chunks.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(batch.approx_bytes());
        }
    }

    /// Convert a display representation into render batches.
    ///
    /// Every fragment's `alpha` (effective entity opacity in `[0, 1]`) is
    /// sanitised and carried onto [`RenderBatch::alpha`]; the renderer then
    /// clamps/classifies it into the opaque, transparent or invisible pass
    /// (`cad-render-wgpu::geometry::classify_alpha`).
    pub fn build(
        &mut self,
        representation: &DisplayRepresentation,
        stamp: TaskStamp,
    ) -> CadResult<SceneDelta> {
        self.build_internal(representation, stamp, false)
    }

    /// Pack adjacent opaque polylines with identical style into line-list
    /// batches, retaining their selection sources without introducing connectors.
    /// Merges stop at 65,536 vertices and a local extent of 8,192 world units
    /// (f32 rebasing roundoff below 0.0005). Transparent fragments stay separate
    /// rather than sharing a combined centroid-based depth key.
    pub fn build_compact(
        &mut self,
        representation: &DisplayRepresentation,
        stamp: TaskStamp,
    ) -> CadResult<SceneDelta> {
        self.build_internal(representation, stamp, true)
    }

    fn build_internal(
        &mut self,
        representation: &DisplayRepresentation,
        stamp: TaskStamp,
        compact: bool,
    ) -> CadResult<SceneDelta> {
        let mut delta = SceneDelta {
            stamp,
            added: Vec::new(),
            removed_chunks: Vec::new(),
            images: Vec::new(),
        };
        let mut previous_was_lines = false;
        for fragment in &representation.fragments {
            let paired = matches!(fragment.primitive, DisplayPrimitive::LineSegments(_));
            if let DisplayPrimitive::LineSegments(points) = &fragment.primitive {
                if points.len() % 2 != 0 {
                    return Err(CadError::InvalidInput(
                        "line segments require endpoint pairs".into(),
                    ));
                }
            }
            let line_points = match &fragment.primitive {
                DisplayPrimitive::Lines(p) | DisplayPrimitive::LineSegments(p) if p.len() >= 2 => {
                    Some(p.as_ref())
                }
                _ => None,
            };
            if let Some(points) = line_points.filter(|_| compact && previous_was_lines) {
                let batch = delta.added.last_mut().expect("previous line batch");
                if batch.vertices.len().saturating_add(if paired {
                    points.len()
                } else {
                    points.len().saturating_sub(1).saturating_mul(2)
                }) <= 65_536
                    && batch.alpha == 1.0
                    && batch.alpha == sanitize_alpha(fragment.alpha)
                    && batch.color == sanitize_color(fragment.color)
                    && batch.color_unresolved == fragment.color_unresolved
                    && batch.lineweight == sanitize_lineweight(fragment.lineweight)
                    && batch.lineweight_unresolved == fragment.lineweight_unresolved
                    && batch.linetype == fragment.linetype
                    && batch.linetype_unresolved == fragment.linetype_unresolved
                    && points.iter().all(|p| {
                        [
                            p.x - batch.local_origin.x,
                            p.y - batch.local_origin.y,
                            p.z - batch.local_origin.z,
                        ]
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 8192.0)
                    })
                {
                    let relative = |p: &Point3| {
                        [
                            (p.x - batch.local_origin.x) as f32,
                            (p.y - batch.local_origin.y) as f32,
                            (p.z - batch.local_origin.z) as f32,
                        ]
                    };
                    if paired {
                        batch.vertices.extend(points.iter().map(relative));
                    } else {
                        batch
                            .vertices
                            .extend(points.windows(2).flatten().map(relative));
                    }
                    if batch.sources.last() != Some(&fragment.source) {
                        batch.sources.push(fragment.source.clone());
                    }
                    continue;
                }
            }
            if let DisplayPrimitive::Image {
                resource,
                transform,
                clip,
            } = &fragment.primitive
            {
                // A real image fragment becomes its own draw item; the renderer
                // binds the texture and maps `transform` (or `clip`) next lane.
                // Text/instance fragments below still produce no batch.
                delta.images.push(ImageBatch {
                    resource: resource.clone(),
                    transform: *transform,
                    clip: clip.clone(),
                    alpha: sanitize_alpha(fragment.alpha),
                    draw_order: 0,
                    sources: vec![fragment.source.clone()],
                });
                continue;
            }
            previous_was_lines = line_points.is_some();
            let (topology, vertices, normals, colors, indices, edges, origin, mirrored) =
                match &fragment.primitive {
                    DisplayPrimitive::Lines(points) | DisplayPrimitive::LineSegments(points) => {
                        let origin = points.first().copied().unwrap_or(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                        let relative = |p: &Point3| {
                            [
                                (p.x - origin.x) as f32,
                                (p.y - origin.y) as f32,
                                (p.z - origin.z) as f32,
                            ]
                        };
                        let verts: Vec<[f32; 3]> = if compact && !paired {
                            points.windows(2).flatten().map(relative).collect()
                        } else {
                            points.iter().map(relative).collect()
                        };
                        (
                            RenderTopology::Lines,
                            verts,
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            origin,
                            false,
                        )
                    }
                    DisplayPrimitive::Mesh(mesh) => {
                        let origin = mesh.vertices.first().copied().unwrap_or(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                        // De-duplicate the triangle stream: one vertex per mesh
                        // vertex, indexed by topology, instead of repeating each
                        // shared corner (audit F14: linear indexing, not ideal).
                        let verts: Vec<[f32; 3]> = mesh
                            .vertices
                            .iter()
                            .map(|p| {
                                [
                                    (p.x - origin.x) as f32,
                                    (p.y - origin.y) as f32,
                                    (p.z - origin.z) as f32,
                                ]
                            })
                            .collect();
                        // Keep only triangles whose indices are in range, and
                        // carry their normals. A triangle dropped here is a
                        // malformed topology, reported by the caller.
                        let mut kept = Vec::with_capacity(mesh.triangles.len());
                        for tri in &mesh.triangles {
                            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                            if a < verts.len() && b < verts.len() && c < verts.len() {
                                kept.push(*tri);
                            }
                        }
                        let normals = mesh
                            .normals
                            .iter()
                            .map(|n| [n.x as f32, n.y as f32, n.z as f32])
                            .collect();
                        // Per-vertex colours are optional and only meaningful
                        // when exactly one entry per vertex exists; otherwise
                        // the batch falls back to its uniform colour.
                        let colors: Vec<[f32; 3]> = if mesh.colors.len() == mesh.vertices.len() {
                            mesh.colors
                                .iter()
                                .map(|c| sanitize_color([c[0] as f32, c[1] as f32, c[2] as f32]))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        // Edge line list built from the kept topology, one
                        // segment per triangle edge. Duplicated edges are not
                        // removed yet; that is a documented follow-up (edges are
                        // only uploaded when a caller asks for the wireframe).
                        let mut edge_positions = Vec::with_capacity(kept.len() * 6);
                        for tri in &kept {
                            for &(i, j) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                                edge_positions.push(verts[i as usize]);
                                edge_positions.push(verts[j as usize]);
                            }
                        }
                        (
                            RenderTopology::Mesh,
                            verts,
                            normals,
                            colors,
                            kept,
                            edge_positions,
                            origin,
                            false,
                        )
                    }
                    // Text/instance batching is handled by their own subsystems;
                    // a scene batch that pretends to draw them would be a fake
                    // success. Images are carried as `ImageBatch` above, so this
                    // arm is unreachable for them but keeps the match exhaustive.
                    DisplayPrimitive::Text { .. }
                    | DisplayPrimitive::Instance { .. }
                    | DisplayPrimitive::Image { .. } => {
                        continue;
                    }
                };
            if vertices.is_empty() {
                continue;
            }
            let _ = mirrored; // filled in by the caller when a transform is known
            delta.added.push(RenderBatch {
                local_origin: origin,
                topology,
                vertices,
                normals,
                colors,
                indices,
                edges,
                mirrored: false,
                alpha: sanitize_alpha(fragment.alpha),
                color: sanitize_color(fragment.color),
                color_unresolved: fragment.color_unresolved,
                lineweight: sanitize_lineweight(fragment.lineweight),
                lineweight_unresolved: fragment.lineweight_unresolved,
                linetype: fragment.linetype.clone(),
                linetype_unresolved: fragment.linetype_unresolved,
                sources: vec![fragment.source.clone()],
                draw_order: 0,
            });
        }
        Ok(delta)
    }

    /// Commit a delta if its stamp still matches the current task context.
    pub fn publish(&mut self, delta: SceneDelta, current: &TaskStamp) -> CadResult<()> {
        delta
            .stamp
            .validate(current)
            .map_err(|_| CadError::StaleResult)?;
        for key in delta.removed_chunks {
            self.remove_chunk(key);
        }
        for batch in delta.added {
            let key = self.next_chunk;
            self.next_chunk += 1;
            self.used_bytes += batch.approx_bytes();
            self.chunks.insert(key, batch);
        }
        for image in delta.images {
            self.images_used_bytes += image.approx_bytes();
            self.images.push(image);
        }
        // Maintenance eviction after publishing: keep the CPU/image caches under
        // budget. Image evictions are recorded in `image_evictions()` so the
        // drop is observable rather than silent.
        let _ = self.evict(0)?;
        let _ = self.evict_images(0)?;
        Ok(())
    }

    /// Drop oldest chunks until the cache is under budget.
    ///
    /// Returns the number of chunks evicted. `evict(0)` is the maintenance call
    /// after publishing; a caller that needs room for a known batch passes its
    /// byte size so the cache is made to fit first. A request larger than the
    /// entire budget is rejected without evicting any chunks. Eviction is what makes
    /// [`SceneBudget::cpu_bytes`] a hard ceiling: the live cache never holds
    /// more than `used_bytes()` bytes counting toward the budget.
    pub fn evict(&mut self, required_bytes: usize) -> CadResult<usize> {
        if required_bytes > self.budget.cpu_bytes {
            return Err(CadError::InvalidInput(format!(
                "scene cache reservation of {required_bytes} bytes exceeds CPU budget of {} bytes",
                self.budget.cpu_bytes
            )));
        }
        let available_bytes = self.budget.cpu_bytes - required_bytes;
        let mut evicted = 0usize;
        while self.used_bytes > available_bytes {
            let Some(oldest) = self.chunks.keys().next().copied() else {
                break;
            };
            self.remove_chunk(oldest);
            evicted += 1;
        }
        Ok(evicted)
    }

    /// Drop oldest image draw items until the image cache is under budget.
    ///
    /// Mirrors [`SceneCache::evict`] for the image list: it keeps
    /// [`SceneBudget::image_bytes`] a hard ceiling so image batches never grow
    /// unbounded. A reservation larger than the whole image budget is rejected
    /// without evicting anything. Each evicted draw item is counted in
    /// [`SceneCache::image_evictions`], so the drop is observable rather than
    /// silent.
    pub fn evict_images(&mut self, required_bytes: usize) -> CadResult<usize> {
        if required_bytes > self.budget.image_bytes {
            return Err(CadError::InvalidInput(format!(
                "image cache reservation of {required_bytes} bytes exceeds image budget of {} bytes",
                self.budget.image_bytes
            )));
        }
        let available_bytes = self.budget.image_bytes - required_bytes;
        let mut evicted = 0usize;
        while self.images_used_bytes > available_bytes {
            if self.images.is_empty() {
                break;
            }
            let oldest = self.images.remove(0);
            self.images_used_bytes = self.images_used_bytes.saturating_sub(oldest.approx_bytes());
            evicted += 1;
        }
        self.image_evictions = self.image_evictions.saturating_add(evicted as u64);
        Ok(evicted)
    }

    /// CPU bytes currently held by the live cache, counted toward
    /// [`SceneBudget::cpu_bytes`].
    ///
    /// This is the cache's own accounting (sum of [`RenderBatch::approx_bytes`]
    /// over live chunks). It is the CPU-cache slice of `MemoryBudget`, not the
    /// whole-process geometry footprint: the per-chunk bytes are counted here
    /// and the source representation/domain data lives elsewhere.
    pub fn total_cpu_bytes(&self) -> usize {
        self.used_bytes
    }
}
