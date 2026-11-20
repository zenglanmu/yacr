//! cache module.

use super::*;
use cad_representation::DisplayFragment;

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
    /// WIPEOUT mask draw items, in insertion order. Masks are painted last by the
    /// renderer (ADR 0022), so the order here is the order among masks only.
    masks: Vec<MaskBatch>,
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
            masks: Vec::new(),
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

    /// The cached WIPEOUT mask draw items, in insertion order.
    ///
    /// A host re-uploads these to the renderer exactly like images: they are
    /// resident scene data, not part of the ordered geometry batch set.
    pub fn mask_batches(&self) -> &[MaskBatch] {
        &self.masks
    }

    /// Every distinct mesh texture key held by this cache — base-colour keys and
    /// the four PBR channel slots — deduplicated and sorted (see
    /// [`mesh_texture_keys`]).
    ///
    /// The host calls this on a base change to resolve exactly the textures the
    /// resident batches reference; the renderer prunes any key outside the set it
    /// is given, so this is also the "live set" contract.
    pub fn mesh_texture_keys(&self) -> Vec<ResourceKey> {
        mesh_texture_keys(self.chunks())
    }

    /// Number of cached mask draw items.
    pub fn mask_count(&self) -> usize {
        self.masks.len()
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

    /// Drop one cached chunk and its accounted bytes.
    ///
    /// Only [`SceneCache::evict`] calls this; there is no change-set or
    /// scene-delta removal channel (a published delta only adds content).
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
            images: Vec::new(),
            masks: Vec::new(),
        };
        let mut previous_was_lines = false;
        let mut previous_was_conic = false;
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
                if line_fragment_merges_into(batch, fragment, paired, points, fragment.draw_order) {
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
                adjust,
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
                    // Images are drawn in their own pass after geometry. Their
                    // order is a separate segmentation follow-up (like masks
                    // before this change), so the image item stays at the
                    // neutral `0` rather than silently pretending the pass is
                    // already interleaved. The fragment does carry the real
                    // order for that follow-up.
                    draw_order: 0,
                    sources: vec![fragment.source.clone()],
                    adjust: *adjust,
                });
                previous_was_lines = false;
                previous_was_conic = false;
                continue;
            }
            if let DisplayPrimitive::Mask {
                boundary,
                inverted,
                frame,
            } = &fragment.primitive
            {
                // A WIPEOUT mask is not geometry the frame plan orders: it paints
                // last, with `depth_compare: Always`, in the renderer's clear
                // colour (ADR 0022). It is carried with its **world boundary**
                // polygon so the renderer can tessellate it concave-safely, and
                // with `inverted` so the complement is filled there.
                if boundary.len() < 3 {
                    return Err(CadError::InvalidInput(
                        "mask boundary requires at least three vertices".into(),
                    ));
                }
                delta.masks.push(MaskBatch {
                    polygon: boundary.clone(),
                    inverted: *inverted,
                    frame: frame.then_some(sanitize_color(fragment.color)),
                    draw_order: fragment.draw_order,
                    sources: vec![fragment.source.clone()],
                });
                previous_was_lines = false;
                previous_was_conic = false;
                continue;
            }
            // Slice B: an eligible circle/arc becomes an SDF conic batch (see
            // `docs/adr/0008-sdf-conic-rendering.md`). An ineligible conic — an
            // ellipse (slice 2), a dashed/transparent conic or a degenerate span
            // — keeps the chord draw below, so the scene never silently changes
            // it. The eligibility gate is recomputed here from the fragment's own
            // fields, so a hand-built `build()` (no provider) is still correct.
            if let DisplayPrimitive::Conic(conic) = &fragment.primitive {
                let eligible = fragment.alpha == 1.0
                    && fragment.linetype.is_continuous()
                    && conic.is_sdf_circle_arc();
                if eligible {
                    let record = RenderConic {
                        center: conic.center,
                        major: conic.major,
                        minor: conic.minor,
                        start: conic.start,
                        sweep: if conic.full {
                            std::f64::consts::TAU
                        } else {
                            conic.sweep
                        },
                        full: conic.full,
                    };
                    if compact && previous_was_conic {
                        let batch = delta.added.last_mut().expect("previous conic batch");
                        if conic_fragment_merges_into(batch, fragment, record, fragment.draw_order)
                        {
                            batch.conics.push(record);
                            if batch.sources.last() != Some(&fragment.source) {
                                batch.sources.push(fragment.source.clone());
                            }
                            previous_was_lines = false;
                            continue;
                        }
                    }
                    delta.added.push(RenderBatch {
                        local_origin: record.center,
                        topology: RenderTopology::Conics,
                        vertices: Vec::new(),
                        normals: Vec::new(),
                        colors: Vec::new(),
                        // A conic batch has no surface, hence no UV0, texture or
                        // PBR channel set.
                        uvs: Vec::new(),
                        texture: None,
                        pbr: None,
                        indices: Vec::new(),
                        edges: Vec::new(),
                        conics: vec![record],
                        mirrored: false,
                        alpha: sanitize_alpha(fragment.alpha),
                        color: sanitize_color(fragment.color),
                        color_unresolved: fragment.color_unresolved,
                        lineweight: sanitize_lineweight(fragment.lineweight),
                        lineweight_unresolved: fragment.lineweight_unresolved,
                        linetype: fragment.linetype.clone(),
                        linetype_unresolved: fragment.linetype_unresolved,
                        sources: vec![fragment.source.clone()],
                        draw_order: fragment.draw_order,
                    });
                    previous_was_lines = false;
                    previous_was_conic = true;
                    continue;
                }
            }
            previous_was_lines = line_points.is_some();
            previous_was_conic = false;
            let (
                topology,
                vertices,
                normals,
                colors,
                uvs,
                texture,
                pbr,
                indices,
                edges,
                origin,
                mirrored,
            ) = match &fragment.primitive {
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
                        None,
                        // A polyline has no surface material, hence no PBR set.
                        None,
                        Vec::new(),
                        Vec::new(),
                        origin,
                        false,
                    )
                }
                // An ineligible conic (ellipse, dashed or degenerate) is
                // chord-expanded exactly like a continuous `Lines` polyline.
                DisplayPrimitive::Conic(conic) => {
                    let points = conic.chord_points();
                    let points: &[Point3] = &points;
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
                    let verts: Vec<[f32; 3]> = if compact {
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
                        None,
                        // A polyline has no surface material, hence no PBR set.
                        None,
                        Vec::new(),
                        Vec::new(),
                        origin,
                        false,
                    )
                }
                DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                    let include_edge_overlay =
                        matches!(&fragment.primitive, DisplayPrimitive::Mesh(_));
                    let MeshStreams {
                        origin,
                        verts,
                        kept,
                        normals,
                        colors,
                        uvs,
                        texture,
                        pbr,
                    } = mesh_streams(mesh);
                    // A plain `Mesh` keeps the legacy triangle-derived
                    // overlay (one segment per unique triangle edge;
                    // shared edges de-duplicated). `MeshWithoutEdgeOverlay`
                    // (a text glyph) carries none.
                    let edge_positions = if include_edge_overlay {
                        triangle_edge_positions(&kept, &verts)
                    } else {
                        Vec::new()
                    };
                    (
                        RenderTopology::Mesh,
                        verts,
                        normals,
                        colors,
                        uvs,
                        texture,
                        pbr,
                        kept,
                        edge_positions,
                        origin,
                        false,
                    )
                }
                // A body's stored edge set is the wireframe source: the
                // overlay positions come from those independent endpoint
                // pairs, never from the triangle topology. De-duplication
                // and the defensive empty fallback live in
                // [`stored_edge_positions`].
                DisplayPrimitive::MeshWithStoredEdges { mesh, edges } => {
                    let MeshStreams {
                        origin,
                        verts,
                        kept,
                        normals,
                        colors,
                        uvs,
                        texture,
                        pbr,
                    } = mesh_streams(mesh);
                    let mut edge_positions = stored_edge_positions(edges, origin);
                    if edge_positions.is_empty() {
                        // Defensive only: the provider emits this variant
                        // with at least one stored segment. If a hand-built
                        // fragment carries none, keep the legacy
                        // triangle-derived overlay (which also keeps
                        // `is_text_glyph` false) rather than drawing a
                        // body with no wireframe.
                        edge_positions = triangle_edge_positions(&kept, &verts);
                    }
                    (
                        RenderTopology::Mesh,
                        verts,
                        normals,
                        colors,
                        uvs,
                        texture,
                        pbr,
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
                | DisplayPrimitive::Image { .. }
                | DisplayPrimitive::Mask { .. } => {
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
                // Texture state, sanitised by `mesh_streams`: both are empty/None
                // unless the mesh carried a valid key and one UV per vertex.
                uvs,
                texture,
                // PBR channels, sanitised by `mesh_streams`: `None` unless the
                // source declared a material with finite in-range factors.
                pbr,
                indices,
                edges,
                conics: Vec::new(),
                mirrored: false,
                alpha: sanitize_alpha(fragment.alpha),
                color: sanitize_color(fragment.color),
                color_unresolved: fragment.color_unresolved,
                lineweight: sanitize_lineweight(fragment.lineweight),
                lineweight_unresolved: fragment.lineweight_unresolved,
                linetype: fragment.linetype.clone(),
                linetype_unresolved: fragment.linetype_unresolved,
                sources: vec![fragment.source.clone()],
                draw_order: fragment.draw_order,
            });
        }
        Ok(delta)
    }

    /// Commit a delta if its stamp still matches the current task context.
    ///
    /// A committed delta only appends: batches get the next chunk key, images
    /// and masks are pushed onto their own lists. Nothing is removed here —
    /// removals are the host's retire path (suppress by flag, then compact on a
    /// full rebuild) plus this cache's own budget eviction.
    pub fn publish(&mut self, delta: SceneDelta, current: &TaskStamp) -> CadResult<()> {
        delta
            .stamp
            .validate(current)
            .map_err(|_| CadError::StaleResult)?;
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
        self.masks.extend(delta.masks);
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

/// The shared GPU streams of a [`RenderTopology::Mesh`] batch.
struct MeshStreams {
    /// World-space reference the vertex and edge positions are reduced against.
    origin: Point3,
    /// De-duplicated vertex stream (one entry per mesh vertex).
    verts: Vec<[f32; 3]>,
    /// The triangle topology whose indices are all in range.
    kept: Vec<[u32; 3]>,
    /// Per-vertex normals, carried verbatim.
    normals: Vec<[f32; 3]>,
    /// Per-vertex colours, empty unless exactly one entry per vertex exists.
    colors: Vec<[f32; 3]>,
    /// Per-vertex UV0 in texture space, empty unless the mesh is textured.
    uvs: Vec<[f32; 2]>,
    /// The document-owned texture key (already converted to a [`ResourceKey`]).
    texture: Option<ResourceKey>,
    /// The sanitised PBR channel set, or `None` when the source declared none.
    pbr: Option<RenderPbr>,
}

/// Build a mesh batch's shared streams.
///
/// De-duplication means one vertex per mesh vertex, indexed by topology, instead
/// of repeating each shared corner (audit F14: linear indexing, not ideal). A
/// triangle with an out-of-range index is dropped; normals are carried verbatim
/// and per-vertex colours are only meaningful when exactly one entry per vertex
/// exists.
///
/// Texture state is carried under the ADR-0038 D8 invariant, sanitised rather
/// than trusted: a mesh only becomes a textured batch when it has a **valid**
/// texture key and exactly one finite UV per vertex. Anything else is sanitised
/// into the untextured form (both fields empty/`None`) instead of panicking — a
/// half-declared texture can never reach the GPU as a fabricated sampling state.
/// `cad-db` validation is the authority that refuses such a mesh at import; this
/// is the last-line defensive filter for a hand-built scene.
fn mesh_streams(mesh: &Mesh) -> MeshStreams {
    let origin = mesh.vertices.first().copied().unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
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
    let colors: Vec<[f32; 3]> = if mesh.colors.len() == mesh.vertices.len() {
        mesh.colors
            .iter()
            .map(|c| sanitize_color([c[0] as f32, c[1] as f32, c[2] as f32]))
            .collect()
    } else {
        Vec::new()
    };
    let textured = mesh
        .texture
        .as_deref()
        .filter(|key| is_valid_texture_key(key))
        .filter(|_| mesh.uvs.len() == mesh.vertices.len() && !mesh.uvs.is_empty())
        .filter(|_| {
            mesh.uvs
                .iter()
                .flatten()
                .all(|component| component.is_finite())
        });
    let texture = textured.map(|key| Some(ResourceKey(key.to_owned())));
    // PBR channels (ADR-0039 D6), sanitised rather than trusted:
    //
    // * every scalar must be finite and inside `[0, 1]` — a violating value drops
    //   the whole `pbr` (there is no honest partial material), matching the
    //   `cad-db` rule that refuses such a mesh at import;
    // * a texture slot needs a valid document key **and** a non-empty, finite,
    //   vertex-aligned UV0 stream, because every PBR map samples UV0. A slot
    //   without a parameterisation is dropped (channel disabled) and the scalar
    //   factors are kept — sampling it would need invented UVs.
    let uvs_usable = !mesh.uvs.is_empty()
        && mesh.uvs.len() == mesh.vertices.len()
        && mesh
            .uvs
            .iter()
            .flatten()
            .all(|component| component.is_finite());
    let pbr = mesh.pbr.as_ref().and_then(|pbr| {
        let unit = |value: f64| value.is_finite() && (0.0..=1.0).contains(&value);
        if !unit(pbr.metallic)
            || !unit(pbr.roughness)
            || !unit(pbr.occlusion_strength)
            || !pbr.emissive.iter().copied().all(unit)
        {
            return None;
        }
        let slot = |key: &Option<String>| {
            key.as_deref()
                .filter(|key| is_valid_texture_key(key))
                .filter(|_| uvs_usable)
                .map(|key| ResourceKey(key.to_owned()))
        };
        Some(RenderPbr {
            metallic: pbr.metallic as f32,
            roughness: pbr.roughness as f32,
            emissive: [
                pbr.emissive[0] as f32,
                pbr.emissive[1] as f32,
                pbr.emissive[2] as f32,
            ],
            occlusion_strength: pbr.occlusion_strength as f32,
            metallic_roughness: slot(&pbr.metallic_roughness_texture),
            normal: slot(&pbr.normal_texture),
            emissive_texture: slot(&pbr.emissive_texture),
            occlusion: slot(&pbr.occlusion_texture),
        })
    });
    // UV0 is uploaded when any channel that samples it is active: the base colour
    // texture or any surviving PBR map.
    let needs_uvs = texture.is_some() || pbr.as_ref().is_some_and(RenderPbr::has_textures);
    let uvs = if needs_uvs {
        mesh.uvs
            .iter()
            .map(|uv| [uv[0] as f32, uv[1] as f32])
            .collect()
    } else {
        Vec::new()
    };
    MeshStreams {
        origin,
        verts,
        kept,
        normals,
        colors,
        uvs,
        texture: texture.flatten(),
        pbr,
    }
}

/// The legacy triangle-derived edge overlay: one segment per **unique**
/// triangle edge.
///
/// The directed pairs of every kept triangle are de-duplicated through
/// [`dedup_pairs`] (an unordered endpoint pair is one segment), so the shared
/// diagonal of two triangles is emitted once — the same five-segment quad
/// wireframe as before the P0.1 change, with no doubled AA fringe on shared
/// edges. It is also the defensive fallback for a `MeshWithStoredEdges` that
/// carries no usable stored segment.
fn triangle_edge_positions(kept: &[[u32; 3]], verts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut pairs = Vec::with_capacity(kept.len() * 6);
    for tri in kept {
        for &(i, j) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            pairs.push(verts[i as usize]);
            pairs.push(verts[j as usize]);
        }
    }
    dedup_pairs(pairs)
}

/// Stored body edge positions relative to `origin`, reduced to `f32`, with
/// duplicate segments removed.
///
/// The input is a flat list of independent endpoint pairs (the
/// [`DisplayPrimitive::MeshWithStoredEdges`] contract). A non-finite endpoint is
/// dropped, then [`dedup_pairs`] removes a repeated unordered pair (`-0.0`
/// normalized to `0.0`) while preserving first-occurrence order and dropping
/// zero-length segments.
///
/// Defensive only: `cad-db` validates a body's edge polylines (at least two
/// points, finite), so a release-path cache cannot contain a bad endpoint. There
/// is no diagnostics channel in this scene build, so the drop is not reported
/// here; the provider's `representation.body_empty` is the missing-geometry
/// signal.
fn stored_edge_positions(points: &[Point3], origin: Point3) -> Vec<[f32; 3]> {
    let mut pairs = Vec::with_capacity(points.len());
    for pair in points.as_chunks::<2>().0 {
        let (Some(a), Some(b)) = (relative_f32(pair[0], origin), relative_f32(pair[1], origin))
        else {
            continue;
        };
        pairs.push(a);
        pairs.push(b);
    }
    dedup_pairs(pairs)
}

/// Drop duplicate and degenerate segments from a flat endpoint-pair list.
///
/// A segment is an **unordered** endpoint pair compared bitwise
/// ([`canonical_segment`]), so the same edge emitted in either direction is one
/// segment. The first occurrence keeps its order; a zero-length pair is dropped.
/// Both wireframe sources (stored body edges and the triangle-derived overlay)
/// go through this one rule.
fn dedup_pairs(pairs: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    let mut seen: std::collections::BTreeSet<[[u32; 3]; 2]> = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs.as_chunks::<2>().0 {
        let key = canonical_segment(pair[0], pair[1]);
        if key[0] == key[1] {
            // Degenerate: both endpoints are the same reduced position.
            continue;
        }
        if seen.insert(key) {
            out.push(pair[0]);
            out.push(pair[1]);
        }
    }
    out
}

/// One world point reduced against `origin` and narrowed to `f32`, `None` when a
/// component is not finite.
fn relative_f32(point: Point3, origin: Point3) -> Option<[f32; 3]> {
    let values = [
        (point.x - origin.x) as f32,
        (point.y - origin.y) as f32,
        (point.z - origin.z) as f32,
    ];
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(values)
}

/// A bitwise key for one `f32` position, with `-0.0` folded onto `0.0`.
fn f32_key(value: f32) -> u32 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

/// The unordered bitwise key of one segment: endpoints are sorted so both
/// directions produce the same key.
fn canonical_segment(a: [f32; 3], b: [f32; 3]) -> [[u32; 3]; 2] {
    let a = a.map(f32_key);
    let b = b.map(f32_key);
    if a <= b {
        [a, b]
    } else {
        [b, a]
    }
}

/// Whether a compactable line `fragment` may be absorbed into the adjacent
/// `batch` (the predicate behind [`SceneCache::build_compact`]).
///
/// The merge is deliberately conservative: it only joins when every attribute
/// the renderer reads matches, **including the paint order**.
/// `incoming_draw_order` is the order of the fragment being appended
/// (`DisplayFragment::draw_order`); the batch's own order is the key it was
/// built with. Requiring equality is what stops a merged batch from silently
/// losing the order of some of its geometry: fragments at different display
/// orders must stay in separate batches so the renderer can sort them
/// individually. It also keeps a batch carrying a distinct order — e.g. a
/// selection-highlight batch ([`crate::HIGHLIGHT_DRAW_ORDER`]) — from absorbing
/// a normal scene fragment.
pub(crate) fn line_fragment_merges_into(
    batch: &RenderBatch,
    fragment: &DisplayFragment,
    paired: bool,
    points: &[Point3],
    incoming_draw_order: i64,
) -> bool {
    let vertex_cost = if paired {
        points.len()
    } else {
        points.len().saturating_sub(1).saturating_mul(2)
    };
    batch.draw_order == incoming_draw_order
        && batch.vertices.len().saturating_add(vertex_cost) <= 65_536
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
}

/// Whether an eligible conic `fragment` may be absorbed into the adjacent conic
/// `batch` (the conic half of [`SceneCache::build_compact`]).
///
/// Mirrors [`line_fragment_merges_into`]: it joins only when every attribute the
/// renderer reads matches (including [`cad_representation::DisplayFragment::draw_order`]),
/// the batch is a conic batch and
/// the conic's whole extent stays within the 8,192-unit stream radius of the
/// batch origin. A conic never merges into a line batch and vice versa.
pub(crate) fn conic_fragment_merges_into(
    batch: &RenderBatch,
    fragment: &DisplayFragment,
    record: RenderConic,
    incoming_draw_order: i64,
) -> bool {
    batch.topology == RenderTopology::Conics
        && batch.draw_order == incoming_draw_order
        && batch.alpha == 1.0
        && batch.alpha == sanitize_alpha(fragment.alpha)
        && batch.color == sanitize_color(fragment.color)
        && batch.color_unresolved == fragment.color_unresolved
        && batch.lineweight == sanitize_lineweight(fragment.lineweight)
        && batch.lineweight_unresolved == fragment.lineweight_unresolved
        && batch.linetype == fragment.linetype
        && batch.linetype_unresolved == fragment.linetype_unresolved
        && record.world_aabb().is_some_and(|(min, max)| {
            let origin = batch.local_origin;
            [min, max].iter().all(|corner| {
                [
                    corner[0] - origin.x,
                    corner[1] - origin.y,
                    corner[2] - origin.z,
                ]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 8192.0)
            })
        })
}
