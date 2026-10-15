//! Sub-element selection highlight overlay (spec F05 / F14).
//!
//! **Whole-entity selection is no longer an overlay.** Since P3a-2 an
//! entity-level selection/hover/hidden state is carried by the renderer's
//! *in-stream flags* path ([`FLAG_SELECTED`] / [`FLAG_HOVERED`] / [`FLAG_HIDDEN`],
//! see `cad-render-wgpu`), which tints (or discards) the resident scene geometry
//! in the shader — no second draw, no scene rebuild. `highlight_batches`
//! therefore only builds the **sub-element** overlay: a mesh face cannot be
//! expressed as a whole-batch flag because the static merged streams do not
//! re-upload per selection change, so a face still needs its own small draw (the
//! user explicitly authorized a separate draw for the few selected sub-elements).
//!
//! This module turns the **sub-element** [`SelectionRef`]s plus the already-built
//! [`DisplayRepresentation`]s into **overlay batches** that reuse the existing
//! [`RenderBatch`] type — there is deliberately no second renderer and no
//! mutation of the authoritative scene.
//!
//! ## What is converted
//!
//! | selection | overlay |
//! |---|---|
//! | whole entity (any primitive) | **nothing** — routed to the flags path, not here |
//! | mesh **face** sub-element | only the triangles whose `face_sources` match |
//!
//! The emitted batches use a distinct [`HighlightOptions::draw_order`] (default
//! [`HIGHLIGHT_DRAW_ORDER`], above the drawing but below the annotation overlay
//! so committed markup stays on top) and a configurable
//! [`HighlightOptions::alpha`] so a host can composite them after the scene.
//!
//! ## Explicit, non-silent limitations
//!
//! - **Entity-level refs contribute no batch.** They are the flags path: a
//!   whole-entity selection tints the resident geometry in-shader, so drawing a
//!   second overlay batch for it would double-highlight. This function silently
//!   ignores an entity-level ref **by design**; the app layer routes those refs
//!   to `cad_render_wgpu`'s `set_entity_flags` instead.
//! - **Highlight tint.** Since `RenderBatch` gained a per-batch RGB channel the
//!   overlay carries [`HighlightOptions::color`] (a selection tint); ordering and
//!   alpha blending still apply. A host that installs no tint uses the documented
//!   default.
//! - **A sub-element only resolves where the geometry carries a stable id.** A
//!   face is drawn iff `Mesh::face_sources[triangle]` equals the selected id;
//!   otherwise nothing is drawn and `highlight.face-missing` is reported. An
//!   edge has no source id in the domain mesh at all, so an edge sub-element is
//!   always unresolved.
//! - **Hidden selections contribute nothing.** A ref rejected by the caller's
//!   `visible` predicate is skipped silently (a session choice, not a gap,
//!   exactly like the annotation overlay). A visible sub-element whose fragments
//!   are all fully transparent (`alpha <= 0`, which the renderer classifies
//!   invisible) is reported as `highlight.invisible` and still contributes no
//!   batch.
//! - **Text / image / instance primitives are not batched** by the scene (same
//!   as [`crate::SceneCache::build`]); a sub-element of one is an explicit
//!   `highlight.unsupported` diagnostic rather than a fake batch.

use cad_domain::{Completeness, Diagnostic, Mesh, SelectionRef, SubElementId};
use cad_representation::{DisplayFragment, DisplayPrimitive, DisplayRepresentation};

use crate::{sanitize_alpha, RenderBatch, RenderTopology};

/// Paint order of the first highlight batch.
///
/// Above the drawing geometry (`0`). A selection is a transient decoration of
/// the base drawing; tool previews use a still-higher order (see
/// `cad_app::render_scene::overlay::PREVIEW_DRAW_ORDER`).
pub const HIGHLIGHT_DRAW_ORDER: i64 = 900_000;

/// Default overlay alpha: visible but not opaque.
pub const DEFAULT_HIGHLIGHT_ALPHA: f32 = 0.55;

/// Default overlay tint: a warm selection highlight, distinct from the drawing's
/// normal per-entity colours.
///
/// This is the **sub-element overlay** tint only (via [`HighlightOptions::color`]).
/// Entity-level selection/hover is tinted by the renderer's in-stream flags path,
/// whose shader tint constants (`SELECTED_TINT`/`HOVERED_TINT` in
/// `cad-render-wgpu`) are a separate, mechanical choice — changing this constant
/// does not affect an entity-level selection.
pub const DEFAULT_HIGHLIGHT_COLOR: [f32; 3] = [1.0, 0.62, 0.19];

/// Selection/hover/visibility bits of the renderer's in-stream flags path.
///
/// These are the **canonical bit values**: the renderer (`cad-render-wgpu`)
/// re-exports them and its WGSL shaders declare the matching constants. A host
/// resolves an entity-level selection into `(SelectionRef, bits)` pairs and hands
/// them to the renderer, which maps each ref to the pick keys of the resident
/// whole-entity batches whose sources name it (see `Renderer::set_entity_flags`).
/// The *tint colours* those bits select are the renderer's shader constants, not
/// [`DEFAULT_HIGHLIGHT_COLOR`] (which is the sub-element overlay tint).
///
/// The bits are a *whole-entity* visual state, not a sub-element one: a merged
/// batch that concatenated several sources is tinted as a whole (documented
/// granularity limit), and a mesh face still needs the sub-element overlay above.
pub const FLAG_SELECTED: u32 = 1 << 0;
/// Hovered (lighter tint of the same mechanism).
pub const FLAG_HOVERED: u32 = 1 << 1;
/// Hidden: the fragment is discarded, so a flagged entity is not drawn.
pub const FLAG_HIDDEN: u32 = 1 << 2;

/// Overlay paint parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightOptions {
    /// `draw_order` of the first emitted batch; each further batch increments
    /// it so the overlay is deterministic and stable under a sort.
    pub draw_order: i64,
    /// Constant per-batch alpha in `[0, 1]` (sanitised like every other batch).
    pub alpha: f32,
    /// Overlay tint as normalized sRGB in `[0, 1]`, carried onto every emitted
    /// batch's `color` (sanitised by [`crate::sanitize_color`]).
    pub color: [f32; 3],
}

impl Default for HighlightOptions {
    fn default() -> Self {
        HighlightOptions {
            draw_order: HIGHLIGHT_DRAW_ORDER,
            alpha: DEFAULT_HIGHLIGHT_ALPHA,
            color: DEFAULT_HIGHLIGHT_COLOR,
        }
    }
}

/// The generated highlight overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct HighlightScene {
    /// Overlay batches, in deterministic selection order. Empty when nothing is
    /// highlighted; never a fabricated batch.
    pub batches: Vec<RenderBatch>,
    /// `Complete` when every visible, resolvable selection was drawn; `Partial`
    /// / `Missing` with one reason per selection that could not be.
    pub completeness: Completeness,
    /// Machine-keyed diagnostics (`highlight.*`); never translated prose.
    pub diagnostics: Vec<Diagnostic>,
    /// How many **entity-level** refs were seen and routed to the renderer's
    /// in-stream flags path instead of producing an overlay batch.
    ///
    /// An entity-level ref contributes no batch by design (it is tinted in-shader
    /// by `cad_render_wgpu`'s `set_entity_flags`), so it is not a gap and does not
    /// make the scene `Partial`. This count makes that shape explicit instead of
    /// an unexplained empty/`Complete` result; the caller is expected to route
    /// those refs to the flags path.
    pub entity_level_refs: usize,
}

impl HighlightScene {
    /// Number of overlay batches produced.
    pub fn drawn(&self) -> usize {
        self.batches.len()
    }

    /// Whether the overlay is empty.
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }
}

/// Build the highlight overlay for `selection`.
///
/// * `selection` — the exact refs (entity + instance + sub-element) to draw.
/// * `representations` — the authoritative display representations, consumed
///   read-only; the caller keeps ownership and the scene cache untouched.
/// * `visible` — session visibility predicate; a ref it rejects contributes
///   nothing and is not an error.
///
/// The result is deterministic: selections are processed in the given order,
/// representations/fragments in iterator order, and triangles in ascending
/// index order. An empty selection or a selection whose refs are all hidden is
/// an explicit empty overlay, not an error.
pub fn highlight_batches<'a, I>(
    selection: &[SelectionRef],
    representations: I,
    visible: impl Fn(&SelectionRef) -> bool,
    options: &HighlightOptions,
) -> HighlightScene
where
    I: IntoIterator<Item = &'a DisplayRepresentation>,
{
    let fragments: Vec<&DisplayFragment> = representations
        .into_iter()
        .flat_map(|representation| representation.fragments.iter())
        .collect();

    let mut scene = HighlightScene {
        batches: Vec::new(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
        entity_level_refs: 0,
    };
    let mut order = options.draw_order;

    for reference in selection {
        if reference.sub_element.is_none() {
            // Whole-entity refs are the renderer's in-stream flags path, not an
            // overlay batch: count them so the result's shape is explicit and
            // they are never mistaken for a silently-dropped selection.
            scene.entity_level_refs += 1;
        }
        if !visible(reference) {
            // Hidden is a session choice: it is not `Partial`.
            continue;
        }
        let matching: Vec<&DisplayFragment> = fragments
            .iter()
            .copied()
            .filter(|fragment| same_entity_instance(&fragment.source, reference))
            .collect();
        if matching.is_empty() {
            scene.missing(
                "highlight.unresolved",
                format!(
                    "selection {} is not present in the supplied representations",
                    SelectionLabel(reference)
                ),
            );
            continue;
        }

        let mut drawn = 0usize;
        let mut opaque = 0usize;
        let mut unsupported = false;
        for fragment in &matching {
            if sanitize_alpha(fragment.alpha) <= 0.0 {
                continue;
            }
            opaque += 1;
            // Entity-level refs are the renderer's in-stream flags path, not an
            // overlay batch: drawing one here would double-highlight the entity.
            // Only a sub-element (mesh face) still needs its own small draw.
            let batch = match &reference.sub_element {
                None => None,
                Some(id) => face_only(reference, fragment, id, order, options.alpha, options.color),
            };
            match batch {
                Some(batch) => {
                    scene.batches.push(batch);
                    order += 1;
                    drawn += 1;
                }
                None => {
                    if reference.sub_element.is_some()
                        && matches!(
                            &fragment.primitive,
                            DisplayPrimitive::Text { .. }
                                | DisplayPrimitive::Image { .. }
                                | DisplayPrimitive::Instance { .. }
                        )
                    {
                        unsupported = true;
                        scene.partial(
                            "highlight.unsupported",
                            format!(
                                "selection {} is a primitive the scene cannot batch",
                                SelectionLabel(reference)
                            ),
                        );
                    }
                }
            }
        }

        if drawn == 0 && !unsupported && reference.sub_element.is_some() {
            if opaque == 0 {
                scene.partial(
                    "highlight.invisible",
                    format!(
                        "selection {} is fully transparent and contributes no batch",
                        SelectionLabel(reference)
                    ),
                );
            } else {
                scene.missing(
                    "highlight.face-missing",
                    format!(
                        "selection {}: no mesh triangle carries that stable source id",
                        SelectionLabel(reference)
                    ),
                );
            }
        }
    }

    scene
}

/// Face-only overlay for a sub-element selection. Returns `None` when the
/// fragment is not a mesh or no triangle carries the selected source id.
fn face_only(
    reference: &SelectionRef,
    fragment: &DisplayFragment,
    id: &SubElementId,
    draw_order: i64,
    alpha: f32,
    color: [f32; 3],
) -> Option<RenderBatch> {
    match &fragment.primitive {
        DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
            mesh_batch(reference, mesh, id, draw_order, alpha, color)
        }
        _ => None,
    }
}

/// Build a face-only overlay mesh batch.
///
/// Selects only the triangles whose `face_sources` equal `id`. Vertices are
/// compacted to those referenced by the selected triangles and rebased on a
/// local origin. Normals are carried only when the mesh has one per vertex;
/// otherwise they are left empty for the renderer's repair path. Returns `None`
/// when no in-range triangle carries the id.
fn mesh_batch(
    reference: &SelectionRef,
    mesh: &Mesh,
    id: &SubElementId,
    draw_order: i64,
    alpha: f32,
    color: [f32; 3],
) -> Option<RenderBatch> {
    let selected: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .enumerate()
        .filter(|(index, tri)| {
            let in_range = tri.iter().all(|&i| (i as usize) < mesh.vertices.len());
            if !in_range {
                return false;
            }
            matches!(mesh.face_sources.get(*index), Some(Some(s)) if s == id)
        })
        .map(|(_, tri)| *tri)
        .collect();
    if selected.is_empty() {
        return None;
    }

    let origin = mesh.vertices[selected[0][0] as usize];
    let carry_normals = mesh.normals.len() == mesh.vertices.len();
    let mut remap: Vec<Option<u32>> = vec![None; mesh.vertices.len()];
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::with_capacity(selected.len());
    for tri in &selected {
        let mut out = [0u32; 3];
        for (slot, &vertex) in tri.iter().enumerate() {
            let vi = vertex as usize;
            let mapped = match remap[vi] {
                Some(mapped) => mapped,
                None => {
                    let mapped = vertices.len() as u32;
                    remap[vi] = Some(mapped);
                    let p = mesh.vertices[vi];
                    vertices.push([
                        (p.x - origin.x) as f32,
                        (p.y - origin.y) as f32,
                        (p.z - origin.z) as f32,
                    ]);
                    if carry_normals {
                        let n = mesh.normals[vi];
                        normals.push([n.x as f32, n.y as f32, n.z as f32]);
                    }
                    mapped
                }
            };
            out[slot] = mapped;
        }
        indices.push(out);
    }

    Some(RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Mesh,
        vertices,
        normals,
        // A highlight recolours the whole overlay uniformly, so any per-vertex
        // gradient colour is intentionally dropped here.
        colors: Vec::new(),
        indices,
        edges: Vec::new(),
        mirrored: false,
        alpha: sanitize_alpha(alpha),
        color: crate::sanitize_color(color),
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        // The highlight overlay is solid: the fragment's dash pattern is not
        // reapplied to the tint so the selection stays fully visible.
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![reference.clone()],
        draw_order,
    })
}

/// Whether a fragment's source and a selection ref name the same object,
/// ignoring the sub-element (which the face filter applies separately).
///
/// Identity is `document + entity + instance`; two INSERT placements stay
/// distinct because their `InstancePath`s differ.
fn same_entity_instance(fragment: &SelectionRef, reference: &SelectionRef) -> bool {
    fragment.document == reference.document
        && fragment.entity == reference.entity
        && fragment.instance == reference.instance
}

/// Minimal identity for a diagnostic message: entity, instance path and
/// sub-element key.
struct SelectionLabel<'a>(&'a SelectionRef);

impl std::fmt::Display for SelectionLabel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "entity {}", self.0.entity.0)?;
        if !self.0.instance.0.is_empty() {
            let path: Vec<String> = self.0.instance.0.iter().map(|e| e.0.to_string()).collect();
            write!(f, " @ [{}]", path.join(" > "))?;
        }
        if let Some(sub) = &self.0.sub_element {
            write!(f, " #{}", sub.source_key)?;
        }
        Ok(())
    }
}

impl HighlightScene {
    fn missing(&mut self, code: &str, message: String) {
        self.diagnostics.push(Diagnostic {
            object: None,
            code: code.into(),
            message: message.clone(),
        });
        self.completeness = self
            .completeness
            .clone()
            .combine(Completeness::Missing(vec![message]));
    }

    fn partial(&mut self, code: &str, message: String) {
        self.diagnostics.push(Diagnostic {
            object: None,
            code: code.into(),
            message: message.clone(),
        });
        self.completeness = self
            .completeness
            .clone()
            .combine(Completeness::Partial(vec![message]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{
        DocumentId, EntityId, GeometrySource, InstancePath, Point3, Precision, Revision,
    };
    use cad_representation::DisplayFragment;
    use cad_representation::{DEFAULT_LINEWEIGHT_MM, DEFAULT_RENDER_COLOR};
    use std::sync::Arc;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn reference(
        entity: u128,
        instance: Vec<u128>,
        sub_element: Option<SubElementId>,
    ) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath(instance.into_iter().map(EntityId).collect()),
            sub_element,
        }
    }

    fn face(key: &str) -> SubElementId {
        SubElementId {
            source_key: key.into(),
            topology_revision: Revision(0),
        }
    }

    fn line_rep(entity: u128, instance: Vec<u128>, points: Vec<Point3>) -> DisplayRepresentation {
        rep(
            entity,
            instance,
            DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
            1.0,
        )
    }

    fn mesh_rep(entity: u128, instance: Vec<u128>, mesh: Mesh) -> DisplayRepresentation {
        rep(
            entity,
            instance,
            DisplayPrimitive::Mesh(Arc::new(mesh)),
            1.0,
        )
    }

    fn rep(
        entity: u128,
        instance: Vec<u128>,
        primitive: DisplayPrimitive,
        alpha: f32,
    ) -> DisplayRepresentation {
        DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: reference(entity, instance, None),
                geometry_source: GeometrySource::Analytic,
                precision: Precision::Analytic,
                alpha,
                color: DEFAULT_RENDER_COLOR,
                color_unresolved: false,
                lineweight: DEFAULT_LINEWEIGHT_MM,
                lineweight_unresolved: false,
                linetype: cad_db::LinetypePattern::continuous(),
                linetype_unresolved: false,
                linetype_scale: 1.0,
                primitive,
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    /// Two coplanar triangles at z = 2 (centres x = -1 and x = 1).
    fn two_face_mesh() -> Mesh {
        Mesh {
            vertices: vec![
                p(-2.0, -1.0, 2.0),
                p(-1.0, 1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(1.0, 1.0, 2.0),
                p(2.0, -1.0, 2.0),
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
        }
    }

    fn highlight(selection: &[SelectionRef], reps: &[DisplayRepresentation]) -> HighlightScene {
        highlight_batches(
            selection,
            reps.iter(),
            |_| true,
            &HighlightOptions::default(),
        )
    }

    #[test]
    fn empty_selection_is_an_explicit_empty_overlay() {
        let reps = vec![line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        )];
        let scene = highlight(&[], &reps);
        assert!(scene.is_empty());
        assert_eq!(scene.drawn(), 0);
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.diagnostics.is_empty());
    }

    #[test]
    fn entity_level_selection_is_routed_to_the_flags_path_not_an_overlay() {
        // A whole-entity ref is the in-stream flags path (see `FLAG_SELECTED`);
        // building an overlay batch here would double-highlight it.
        let reps = vec![line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0)],
        )];
        let selection = [reference(1, Vec::new(), None)];
        let scene = highlight(&selection, &reps);
        assert!(
            scene.is_empty(),
            "entity-level must not emit an overlay batch"
        );
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(
            scene.diagnostics.is_empty(),
            "entity-level is not a gap: it is routed to flags"
        );
        // The shape is explicit: the ref was seen and routed, not dropped.
        assert_eq!(scene.entity_level_refs, 1);
    }

    #[test]
    fn mesh_face_selection_emits_only_that_face() {
        let reps = vec![mesh_rep(50, Vec::new(), two_face_mesh())];
        let selection = [reference(50, Vec::new(), Some(face("face-a")))];
        let scene = highlight(&selection, &reps);
        assert_eq!(scene.drawn(), 1);
        let batch = &scene.batches[0];
        assert_eq!(batch.topology, RenderTopology::Mesh);
        // Only triangle A, compacted to its three vertices.
        assert_eq!(batch.indices, vec![[0, 1, 2]]);
        assert_eq!(batch.vertices.len(), 3);
        // The batch reports the exact selected identity (entity + face).
        assert_eq!(
            batch.sources[0]
                .sub_element
                .as_ref()
                .map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        // The sub-element overlay keeps the distinct paint order (above the base
        // drawing at 0, below the tool preview).
        assert_eq!(batch.draw_order, HIGHLIGHT_DRAW_ORDER);
        assert!((batch.alpha - DEFAULT_HIGHLIGHT_ALPHA).abs() < 1e-6);
        assert_eq!(scene.completeness, Completeness::Complete);
    }

    #[test]
    fn two_faces_of_one_mesh_highlight_independently() {
        let reps = vec![mesh_rep(50, Vec::new(), two_face_mesh())];
        let selection = [
            reference(50, Vec::new(), Some(face("face-a"))),
            reference(50, Vec::new(), Some(face("face-b"))),
        ];
        let scene = highlight(&selection, &reps);
        assert_eq!(scene.drawn(), 2);
        // Deterministic paint order: each batch increments by one.
        assert_eq!(scene.batches[0].draw_order, HIGHLIGHT_DRAW_ORDER);
        assert_eq!(scene.batches[1].draw_order, HIGHLIGHT_DRAW_ORDER + 1);
        assert_eq!(scene.batches[0].indices, vec![[0, 1, 2]]);
        assert_eq!(scene.batches[1].indices, vec![[0, 1, 2]]);
        // Different local origins prove the two faces are distinct geometry.
        assert_ne!(scene.batches[0].local_origin, scene.batches[1].local_origin);
    }

    #[test]
    fn whole_mesh_selection_is_routed_to_the_flags_path_not_an_overlay() {
        let reps = vec![mesh_rep(50, Vec::new(), two_face_mesh())];
        let scene = highlight(&[reference(50, Vec::new(), None)], &reps);
        assert!(
            scene.is_empty(),
            "a whole mesh is flags-tinted, not overlaid"
        );
        assert!(scene.diagnostics.is_empty());
    }

    #[test]
    fn face_without_a_stable_source_is_reported_not_drawn() {
        let mut mesh = two_face_mesh();
        mesh.face_sources = Vec::new();
        let reps = vec![mesh_rep(50, Vec::new(), mesh)];
        let scene = highlight(&[reference(50, Vec::new(), Some(face("face-a")))], &reps);
        assert!(scene.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.face-missing"));
        assert!(matches!(scene.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn hidden_selection_contributes_nothing_and_is_not_a_gap() {
        let reps = [line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        )];
        let selection = [reference(1, Vec::new(), None)];
        let scene = highlight_batches(
            &selection,
            reps.iter(),
            |_| false,
            &HighlightOptions::default(),
        );
        assert!(scene.is_empty());
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.diagnostics.is_empty());
    }

    #[test]
    fn fully_transparent_sub_element_is_reported_but_contributes_no_batch() {
        let reps = vec![rep(
            50,
            Vec::new(),
            DisplayPrimitive::Mesh(Arc::new(two_face_mesh())),
            0.0,
        )];
        let scene = highlight(&[reference(50, Vec::new(), Some(face("face-a")))], &reps);
        assert!(
            scene.is_empty(),
            "an invisible sub-element must not be fabricated"
        );
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.invisible"));
    }

    #[test]
    fn selection_absent_from_the_representations_is_reported() {
        let reps = vec![line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        )];
        let scene = highlight(&[reference(999, Vec::new(), None)], &reps);
        assert!(scene.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unresolved"));
        assert!(matches!(scene.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn insert_instances_are_entity_level_and_produce_no_overlay() {
        // Two placements of the same block child are distinct entity-level refs;
        // both are the flags path, so neither emits an overlay batch. The
        // per-placement distinction is exercised by the renderer's entity-flag
        // resolution (the PickTable sources carry the instance path).
        let reps = vec![
            line_rep(20, vec![10], vec![p(10.0, 0.0, 0.0), p(10.0, 5.0, 0.0)]),
            line_rep(20, vec![20], vec![p(20.0, 0.0, 0.0), p(20.0, 5.0, 0.0)]),
        ];
        let scene = highlight(&[reference(20, vec![10], None)], &reps);
        assert!(scene.is_empty());
        assert!(scene.diagnostics.is_empty());
    }

    #[test]
    fn sub_element_of_an_unsupported_primitive_is_reported_not_faked() {
        let reps = vec![rep(
            7,
            Vec::new(),
            DisplayPrimitive::Instance {
                block: cad_domain::BlockId(0),
                transform: cad_domain::Transform3::identity(),
            },
            1.0,
        )];
        let scene = highlight(&[reference(7, Vec::new(), Some(face("face-a")))], &reps);
        assert!(scene.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unsupported"));
    }

    #[test]
    fn overlay_alpha_is_sanitised() {
        let reps = [mesh_rep(50, Vec::new(), two_face_mesh())];
        let options = HighlightOptions {
            draw_order: HIGHLIGHT_DRAW_ORDER,
            alpha: 2.0,
            color: DEFAULT_HIGHLIGHT_COLOR,
        };
        let scene = highlight_batches(
            &[reference(50, Vec::new(), Some(face("face-a")))],
            reps.iter(),
            |_| true,
            &options,
        );
        assert_eq!(scene.batches[0].alpha, 1.0);
    }
}
