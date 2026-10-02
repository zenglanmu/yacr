//! cache module.

use super::*;

/// Bounds and caching for the CPU-side scene.
pub struct SceneCache {
    pub budget: SceneBudget,
    chunks: BTreeMap<u64, RenderBatch>,
    next_chunk: u64,
    used_bytes: usize,
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
    /// (`cad-render-wgpu::geometry::classify_alpha`). The annotation overlay
    /// path ([`annotations::annotation_batches`]) supplies its own alpha from
    /// the annotation style.
    pub fn build(
        &mut self,
        representation: &DisplayRepresentation,
        stamp: TaskStamp,
    ) -> CadResult<SceneDelta> {
        let mut delta = SceneDelta {
            stamp,
            added: Vec::new(),
            removed_chunks: Vec::new(),
        };
        for fragment in &representation.fragments {
            let (topology, vertices, normals, indices, edges, origin, mirrored) =
                match &fragment.primitive {
                    DisplayPrimitive::Lines(points) => {
                        let origin = points.first().copied().unwrap_or(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                        let verts: Vec<[f32; 3]> = points
                            .iter()
                            .map(|p| {
                                [
                                    (p.x - origin.x) as f32,
                                    (p.y - origin.y) as f32,
                                    (p.z - origin.z) as f32,
                                ]
                            })
                            .collect();
                        (
                            RenderTopology::Lines,
                            verts,
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
                            kept,
                            edge_positions,
                            origin,
                            false,
                        )
                    }
                    // Text/instance/image batching is handled by their own
                    // subsystems; a scene batch that pretends to draw them would
                    // be a fake success.
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
        self.evict(0)?;
        Ok(())
    }

    /// Drop oldest chunks until the cache is under budget.
    pub fn evict(&mut self, required_bytes: usize) -> CadResult<()> {
        while self.used_bytes + required_bytes > self.budget.cpu_bytes {
            let Some(oldest) = self.chunks.keys().next().copied() else {
                break;
            };
            self.remove_chunk(oldest);
        }
        Ok(())
    }
}
