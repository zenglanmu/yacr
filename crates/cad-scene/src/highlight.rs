//! Selection highlight overlay (spec F05 / F14).
//!
//! A selection (entity / INSERT instance / mesh face) has to be drawable
//! *distinctly* from the authoritative scene, without touching the database or
//! the scene cache. This module turns a list of [`SelectionRef`]s plus the
//! already-built [`DisplayRepresentation`]s into **overlay batches** that reuse
//! the existing [`RenderBatch`] type — there is deliberately no second
//! renderer and no mutation of the authoritative scene.
//!
//! ## What is converted
//!
//! | selection | overlay |
//! |---|---|
//! | whole entity, line/curve fragment | the fragment's polyline |
//! | whole entity, mesh fragment | the mesh's triangles |
//! | mesh **face** sub-element | only the triangles whose `face_sources` match |
//!
//! The emitted batches use a distinct [`HighlightOptions::draw_order`] (default
//! [`HIGHLIGHT_DRAW_ORDER`], above the drawing but below the annotation overlay
//! so committed markup stays on top) and a configurable
//! [`HighlightOptions::alpha`] so a host can composite them after the scene.
//!
//! ## Explicit, non-silent limitations
//!
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
//!   exactly like the annotation overlay). A visible ref whose fragments are all
//!   fully transparent (`alpha <= 0`, which the renderer classifies invisible) is
//!   reported as `highlight.invisible` and still contributes no batch.
//! - **Text / image / instance primitives are not batched** by the scene (same
//!   as [`crate::SceneCache::build`]); selecting one is an explicit
//!   `highlight.unsupported` diagnostic rather than a fake batch.

use cad_domain::{Completeness, Diagnostic, Mesh, Point3, SelectionRef, SubElementId};
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
pub const DEFAULT_HIGHLIGHT_COLOR: [f32; 3] = [1.0, 0.62, 0.19];

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
    };
    let mut order = options.draw_order;

    for reference in selection {
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
            let batch = match &reference.sub_element {
                None => whole_fragment(reference, fragment, order, options.alpha, options.color),
                Some(id) => face_only(reference, fragment, id, order, options.alpha, options.color),
            };
            match batch {
                Some(batch) => {
                    scene.batches.push(batch);
                    order += 1;
                    drawn += 1;
                }
                None => {
                    if matches!(
                        &fragment.primitive,
                        DisplayPrimitive::Text { .. }
                            | DisplayPrimitive::Image { .. }
                            | DisplayPrimitive::Instance { .. }
                    ) {
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

        if drawn == 0 && !unsupported {
            if opaque == 0 {
                scene.partial(
                    "highlight.invisible",
                    format!(
                        "selection {} is fully transparent and contributes no batch",
                        SelectionLabel(reference)
                    ),
                );
            } else if reference.sub_element.is_some() {
                scene.missing(
                    "highlight.face-missing",
                    format!(
                        "selection {}: no mesh triangle carries that stable source id",
                        SelectionLabel(reference)
                    ),
                );
            } else {
                scene.partial(
                    "highlight.empty",
                    format!(
                        "selection {} has no drawable geometry in the representations",
                        SelectionLabel(reference)
                    ),
                );
            }
        }
    }

    scene
}

/// Whole-fragment overlay for an entity-level selection.
fn whole_fragment(
    reference: &SelectionRef,
    fragment: &DisplayFragment,
    draw_order: i64,
    alpha: f32,
    color: [f32; 3],
) -> Option<RenderBatch> {
    match &fragment.primitive {
        DisplayPrimitive::Lines(points) => line_batch(reference, points, draw_order, alpha, color),
        DisplayPrimitive::LineSegments(points) => {
            line_batch(reference, points, draw_order, alpha, color)
        }
        DisplayPrimitive::Mesh(mesh) => mesh_batch(reference, mesh, None, draw_order, alpha, color),
        DisplayPrimitive::Text { .. }
        | DisplayPrimitive::Image { .. }
        | DisplayPrimitive::Instance { .. } => None,
    }
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
        DisplayPrimitive::Mesh(mesh) => {
            mesh_batch(reference, mesh, Some(id), draw_order, alpha, color)
        }
        _ => None,
    }
}

fn line_batch(
    reference: &SelectionRef,
    points: &[Point3],
    draw_order: i64,
    alpha: f32,
    color: [f32; 3],
) -> Option<RenderBatch> {
    if points.len() < 2 {
        return None;
    }
    let origin = points[0];
    let vertices = points
        .iter()
        .map(|p| [p.x - origin.x, p.y - origin.y, p.z - origin.z].map(|v| v as f32))
        .collect();
    Some(RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
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

/// Build an overlay mesh batch.
///
/// `only == None` selects every in-range triangle; `Some(id)` selects only the
/// triangles whose `face_sources` equal `id`. Vertices are compacted to those
/// referenced by the selected triangles and rebased on a local origin. Normals
/// are carried only when the mesh has one per vertex; otherwise they are left
/// empty for the renderer's repair path.
fn mesh_batch(
    reference: &SelectionRef,
    mesh: &Mesh,
    only: Option<&SubElementId>,
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
            match only {
                None => true,
                Some(id) => matches!(mesh.face_sources.get(*index), Some(Some(s)) if s == id),
            }
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
    use cad_domain::{DocumentId, EntityId, GeometrySource, InstancePath, Precision, Revision};
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
    fn entity_selection_emits_an_overlay_batch_with_a_distinct_draw_order() {
        let reps = vec![line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0)],
        )];
        let selection = [reference(1, Vec::new(), None)];
        let scene = highlight(&selection, &reps);
        assert_eq!(scene.drawn(), 1);
        let batch = &scene.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(batch.vertices.len(), 2);
        assert_eq!(batch.draw_order, HIGHLIGHT_DRAW_ORDER);
        assert!(
            batch.draw_order > 0,
            "after the base drawing geometry at order 0"
        );
        assert!(
            batch.draw_order < 1_000_000,
            "below the tool-preview order so transient markup stays on top"
        );
        assert!((batch.alpha - DEFAULT_HIGHLIGHT_ALPHA).abs() < 1e-6);
        assert_eq!(batch.sources, vec![reference(1, Vec::new(), None)]);
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.diagnostics.is_empty());
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
    fn whole_mesh_selection_emits_both_triangles() {
        let reps = vec![mesh_rep(50, Vec::new(), two_face_mesh())];
        let scene = highlight(&[reference(50, Vec::new(), None)], &reps);
        assert_eq!(scene.drawn(), 1);
        assert_eq!(scene.batches[0].indices.len(), 2);
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
    fn fully_transparent_selection_is_reported_but_contributes_no_batch() {
        let reps = vec![rep(
            1,
            Vec::new(),
            DisplayPrimitive::Lines(Arc::from(
                vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)].into_boxed_slice(),
            )),
            0.0,
        )];
        let scene = highlight(&[reference(1, Vec::new(), None)], &reps);
        assert!(
            scene.is_empty(),
            "invisible selection must not be fabricated"
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
    fn insert_instances_highlight_independently() {
        let reps = vec![
            line_rep(20, vec![10], vec![p(10.0, 0.0, 0.0), p(10.0, 5.0, 0.0)]),
            line_rep(20, vec![20], vec![p(20.0, 0.0, 0.0), p(20.0, 5.0, 0.0)]),
        ];
        // Only the first instance is selected.
        let scene = highlight(&[reference(20, vec![10], None)], &reps);
        assert_eq!(scene.drawn(), 1);
        assert_eq!(scene.batches[0].local_origin, p(10.0, 0.0, 0.0));
        assert_eq!(
            scene.batches[0].sources[0].instance,
            InstancePath(vec![EntityId(10)])
        );
    }

    #[test]
    fn unsupported_primitive_is_reported_not_faked() {
        let reps = vec![rep(
            7,
            Vec::new(),
            DisplayPrimitive::Instance {
                block: cad_domain::BlockId(0),
                transform: cad_domain::Transform3::identity(),
            },
            1.0,
        )];
        let scene = highlight(&[reference(7, Vec::new(), None)], &reps);
        assert!(scene.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unsupported"));
    }

    #[test]
    fn overlay_alpha_is_sanitised() {
        let reps = [line_rep(
            1,
            Vec::new(),
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        )];
        let options = HighlightOptions {
            draw_order: HIGHLIGHT_DRAW_ORDER,
            alpha: 2.0,
            color: DEFAULT_HIGHLIGHT_COLOR,
        };
        let scene = highlight_batches(
            &[reference(1, Vec::new(), None)],
            reps.iter(),
            |_| true,
            &options,
        );
        assert_eq!(scene.batches[0].alpha, 1.0);
    }
}
