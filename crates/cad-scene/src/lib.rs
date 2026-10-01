//! CPU scene cache, separate from authoritative data and device resources.
//!
//! Spec v2.0 §4.8, §8.1, §8.4: display representations become chunked render
//! batches. Batches are keyed so a change invalidates only the affected chunks,
//! and vertices are stored relative to a per-batch local origin so `f32` keeps
//! precision at large coordinates.

use cad_db::{ChangeSet, ObjectChange};
use cad_domain::*;
use cad_representation::{DisplayPrimitive, DisplayRepresentation};
use std::collections::BTreeMap;

/// Identity of a cached chunk; any component change invalidates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub identity: DocumentIdentity,
    pub entity: EntityId,
    pub revision: Revision,
    pub style_version: u64,
    pub font_version: u64,
    pub lod: u32,
    pub space: SpaceId,
}

/// A GPU-ready batch. `vertices` are relative to `local_origin`.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderBatch {
    pub local_origin: Point3,
    pub vertices: Vec<[f32; 3]>,
    pub sources: Vec<SelectionRef>,
    pub draw_order: i64,
}

impl RenderBatch {
    pub fn triangle_count(&self) -> usize {
        self.vertices.len() / 3
    }

    pub fn approx_bytes(&self) -> usize {
        self.vertices.len() * 12
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneBudget {
    pub cpu_bytes: usize,
    pub queued_tasks: usize,
    pub upload_bytes_per_frame: usize,
}

impl Default for SceneBudget {
    fn default() -> Self {
        SceneBudget {
            cpu_bytes: 128 * 1024 * 1024,
            queued_tasks: 8,
            upload_bytes_per_frame: 4 * 1024 * 1024,
        }
    }
}

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
            let (vertices, local_origin) = match &fragment.primitive {
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
                    (verts, origin)
                }
                DisplayPrimitive::Mesh(mesh) => {
                    let origin = mesh.vertices.first().copied().unwrap_or(Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    });
                    let mut verts = Vec::with_capacity(mesh.triangles.len() * 3);
                    for tri in &mesh.triangles {
                        for &idx in tri.iter() {
                            if let Some(p) = mesh.vertices.get(idx as usize) {
                                verts.push([
                                    (p.x - origin.x) as f32,
                                    (p.y - origin.y) as f32,
                                    (p.z - origin.z) as f32,
                                ]);
                            }
                        }
                    }
                    (verts, origin)
                }
                // Text/instance/image batching is handled by their own
                // subsystems; a scene batch that pretends to draw them would be
                // a fake success.
                DisplayPrimitive::Text { .. }
                | DisplayPrimitive::Instance { .. }
                | DisplayPrimitive::Image { .. } => {
                    continue;
                }
            };
            if vertices.is_empty() {
                continue;
            }
            delta.added.push(RenderBatch {
                local_origin,
                vertices,
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

#[cfg(test)]
mod tests {
    use super::*;
    use cad_representation::DisplayFragment;
    use std::sync::Arc;

    fn stamp() -> TaskStamp {
        TaskStamp::new(DocumentId(1), 0)
    }

    fn line_representation(entity: u128, points: Vec<Point3>) -> DisplayRepresentation {
        DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: SelectionRef {
                    document: DocumentId(1),
                    entity: EntityId(entity),
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                geometry_source: GeometrySource::Analytic,
                primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn build_produces_relative_vertices() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
                Point3 {
                    x: 1_000_000.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1_000_010.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added.len(), 1);
        let batch = &delta.added[0];
        assert_eq!(batch.local_origin.x, 1_000_000.0);
        // Relative coordinates stay small and precise.
        assert_eq!(batch.vertices[1][0], 10.0);
    }

    #[test]
    fn publish_rejects_stale_stamp() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, TaskStamp::new(DocumentId(1), 5)).unwrap();
        let current = TaskStamp::new(DocumentId(1), 6);
        assert_eq!(cache.publish(delta, &current), Err(CadError::StaleResult));
        assert_eq!(cache.chunk_count(), 0);
    }

    #[test]
    fn change_set_removes_affected_chunks() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
        assert_eq!(cache.chunk_count(), 1);
        let changes = ChangeSet {
            database: DatabaseId(1),
            before: Revision(0),
            after: Revision(1),
            transaction: TransactionId(1),
            reason: "edit".into(),
            changes: vec![ObjectChange::Update(
                ObjectId(1),
                cad_db::ChangeMask::GEOMETRY,
            )],
        };
        cache.apply_changes(&changes).unwrap();
        assert_eq!(cache.chunk_count(), 0);
    }

    #[test]
    fn eviction_respects_budget() {
        let mut cache = SceneCache::new(SceneBudget {
            cpu_bytes: 40,
            ..Default::default()
        });
        for i in 0..4 {
            let rep = line_representation(
                i,
                vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 2.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ],
            );
            let delta = cache.build(&rep, stamp()).unwrap();
            cache.publish(delta, &stamp()).unwrap();
        }
        assert!(cache.used_bytes() <= 40);
        assert!(cache.chunk_count() < 4);
    }
}
