//! Class histograms: fragment-level (display representation) and batch-level
//! (built scene).

use cad_render_wgpu::geometry::{classify_alpha, AlphaClass};
use cad_representation::DisplayPrimitive;
use cad_scene::{RenderTopology, SceneDelta};

/// Fragment-level classes from the display representation (font-aware: a
/// tessellated glyph is `MeshWithoutEdgeOverlay`).
#[derive(Default, Clone)]
pub(crate) struct FragmentHistogram {
    pub(crate) lines: usize,
    pub(crate) line_vertices: usize,
    pub(crate) meshes: usize,
    pub(crate) mesh_triangles: usize,
    pub(crate) glyph_meshes: usize,
    pub(crate) glyph_triangles: usize,
    pub(crate) texts: usize,
    pub(crate) instances: usize,
    pub(crate) images: usize,
}

impl FragmentHistogram {
    pub(crate) fn tally(&mut self, rep: &cad_representation::DisplayRepresentation) {
        for fragment in &rep.fragments {
            match &fragment.primitive {
                DisplayPrimitive::Lines(points) | DisplayPrimitive::LineSegments(points) => {
                    self.lines += 1;
                    self.line_vertices += points.len();
                }
                DisplayPrimitive::Mesh(mesh) => {
                    self.meshes += 1;
                    self.mesh_triangles += mesh.triangles.len();
                }
                DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                    self.glyph_meshes += 1;
                    self.glyph_triangles += mesh.triangles.len();
                }
                DisplayPrimitive::Conic(conic) => {
                    // Slice B: an eligible circle/arc is drawn as an SDF conic
                    // batch (tallied in `conic_batches`); an ineligible one
                    // (ellipse/dashed/transparent) is chord-expanded. This
                    // representation-level tally is a rough upper bound and is
                    // superseded by the batch histogram below.
                    self.lines += 1;
                    self.line_vertices += conic.chord_points().len();
                }
                DisplayPrimitive::Text { .. } => self.texts += 1,
                DisplayPrimitive::Instance { .. } => self.instances += 1,
                DisplayPrimitive::Image { .. } => self.images += 1,
            }
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.lines + self.meshes + self.glyph_meshes + self.texts + self.instances + self.images
    }
}

/// Batch-level classes from the built scene. A `Mesh` batch with an edge overlay
/// is a solid mesh (`DisplayPrimitive::Mesh`); one without is a glyph
/// (`DisplayPrimitive::MeshWithoutEdgeOverlay`) — the cache sets the overlay
/// exactly for the former (see `cad_scene::SceneCache::build_internal`).
#[derive(Default, Clone)]
pub(crate) struct BatchHistogram {
    pub(crate) line_batches: usize,
    pub(crate) line_vertices: usize,
    pub(crate) mesh_batches: usize,
    pub(crate) mesh_triangles: usize,
    pub(crate) glyph_batches: usize,
    pub(crate) glyph_triangles: usize,
    pub(crate) mesh_edge_batches: usize,
    /// Extra edge-overlay draw calls a `Mesh` batch issues beyond its triangle
    /// draw (each mesh-wireframe draw is a second `draw_indexed`).
    pub(crate) overlay_draws: usize,
    pub(crate) opaque: usize,
    pub(crate) transparent: usize,
    pub(crate) invisible: usize,
    pub(crate) images: usize,
    /// Analytic SDF conic batches (slice B).
    pub(crate) conic_batches: usize,
}

impl BatchHistogram {
    pub(crate) fn tally(&mut self, delta: &SceneDelta) {
        for batch in &delta.added {
            match batch.topology {
                RenderTopology::Lines => {
                    self.line_batches += 1;
                    self.line_vertices += batch.vertices.len();
                }
                RenderTopology::MeshEdges => self.mesh_edge_batches += 1,
                RenderTopology::Conics => self.conic_batches += 1,
                RenderTopology::Mesh => {
                    if batch.edges.is_empty() {
                        self.glyph_batches += 1;
                        self.glyph_triangles += batch.triangle_count();
                    } else {
                        self.mesh_batches += 1;
                        self.mesh_triangles += batch.triangle_count();
                        self.overlay_draws += 1;
                    }
                }
            }
            match classify_alpha(batch.alpha) {
                AlphaClass::Opaque => self.opaque += 1,
                AlphaClass::Transparent => self.transparent += 1,
                AlphaClass::Invisible => self.invisible += 1,
            }
        }
        self.images = delta.images.len();
    }

    pub(crate) fn total_batches(&self) -> usize {
        self.line_batches + self.mesh_batches + self.glyph_batches + self.mesh_edge_batches
    }
}
