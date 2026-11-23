//! SketchUp (`.skp`) export writer.
//!
//! `skpwright` 0.6.0 authors the **2026 container** (`{26.x}`, a ZIP archive of
//! tagged records) from a [`skpwright::Model`]. Its mesh vertices are metres and
//! the crate converts them to the inches the container stores, so this writer
//! must scale `cad-db` coordinates by the document's **declared** source unit.
//! There is no default unit: a job without a known unit is refused
//! ([`UNITS_UNKNOWN`]) rather than guessed.
//!
//! Scope: a projection of the `cad-db` authority into SketchUp faces and
//! free-standing edges.
//!
//! * **One** root [`skpwright::GeometryRun`] (`def_index: None`) holds every
//!   written source. The pinned writer emits only the *first* root run
//!   (`build26.rs`: `model.geometry.iter().find(|r| r.def_index.is_none())`), so
//!   the earlier one-run-per-source layout silently dropped every source but the
//!   first while still reporting them exact. Sources are merged in deterministic
//!   database order and each source's triangle indices are offset by the running
//!   vertex base; grouping is the price, named once by
//!   [`ENTITY_GROUPING_FLATTENED`] when more than one source is written.
//! * Every triangle becomes one triangular [`skpwright::MeshFace`] with its own
//!   unit Newell normal. Collinear/zero-area triangles are dropped by name
//!   ([`DEGENERATE_TRIANGLE`]). Each written triangle's three sides are
//!   regenerated as `MeshEdge`s (one canonical pair each, shared across sources).
//! * A display body with **no** triangles but non-empty `display.edges` is
//!   written as free edges: each polyline appends its own endpoints (no welding
//!   to mesh vertices or other polylines), chains its N>2 points into N-1
//!   segments with the interior indices shared, and each segment becomes a
//!   default [`skpwright::MeshEdge`] (`curve: None`, layer 0, no material,
//!   default flags). Repeated endpoint pairs inside one polyline are written
//!   once; zero-length segments are skipped by name ([`ZERO_LENGTH_EDGE`]).
//!   Collapsing a retraced duplicate pair is coordinate normalization (the
//!   written edge set is unchanged), so such a source still counts `exact`; a
//!   source with a skipped zero-length segment does not, because that segment
//!   is geometry the file does not carry. A wire body whose segments are **all**
//!   zero-length contributes nothing at all, so the entity itself is dropped by
//!   name ([`WIRE_BODY_NO_WRITABLE_EDGES`]) instead of leaving an empty written
//!   model behind a `Partial` note.
//!   Curve/arc identity, per-edge flags, material and layer have no carrier in
//!   a display polyline and are reported once ([`EDGE_SEMANTICS_NOT_WRITTEN`]).
//! * A **mesh-bearing** display body's stored edge set/segmentation is not
//!   copied (ADR-0049 §2): its triangles are written and only their boundaries
//!   are regenerated, so a stored face-boundary edge survives as a triangle
//!   boundary while loose or separately segmented stored edges do not. Each
//!   affected entity gets a `Partial` [`STORED_EDGES_NOT_WRITTEN`] entry. When
//!   every triangle is degenerate and the body therefore writes no surface
//!   geometry, only the drop is reported ([`DEGENERATE_TRIANGLE`]); no
//!   stored-edge loss is claimed for an entity that wrote nothing.
//! * Layers, materials, colours and every other non-mesh semantic are **not**
//!   written; when a source entity carries one, the report says so
//!   ([`LAYER_MATERIAL_FLATTENED`]). Per-vertex colours and normals are named
//!   separately ([`COLORS_DROPPED`], [`NORMALS_DROPPED`]) because neither has a
//!   SketchUp face carrier. Geometry the writer cannot represent is a
//!   dropped entry ([`UNSUPPORTED_GEOMETRY`]), never a silent omission.
//! * Byte-identical output for the same document: the save time and the GUID
//!   seed are fixed ([`export_env`]) instead of taken from the system clock.
//!
//! Evidence boundary: the bytes are a container the pinned **reader** can parse
//! back (self-consistency only). No SketchUp/Trimble product was used to open
//! them, and this is not a vendor acceptance claim.
//!
//! The descriptor is supplied by the caller ([`SkpExporter::new`]) so this
//! writer half does not declare connector metadata of its own: the merged crate
//! root declares the single [`crate::DESCRIPTOR_SKP`] (it describes both
//! directions) and the registry passes it in.
//!
//! ```text
//! cad-db (source units) --× metres per source unit--> skpwright::Model (metres)
//!     --Document::from_model_with(WriteEnv)--> 2026 container bytes
//! ```

mod write;

#[cfg(test)]
mod tests;

use cad_connector_core::{
    CompatCounts, CompatEntry, CompatReport, ConnectorDescriptor, ExportJob, ExportOutput,
    ExportSpace, Exporter, FidelityLevel, ReportDirection,
};
use cad_domain::{CadError, CadResult, Completeness, EntityCapability, SupportStatus};

pub use write::{
    export_env, metres_per_source_unit, COLORS_DROPPED, DEGENERATE_TRIANGLE,
    EDGE_SEMANTICS_NOT_WRITTEN, ENTITY_GROUPING_FLATTENED, LAYER_MATERIAL_FLATTENED,
    LIMITS_EXCEEDED, MISMATCHED_DESCRIPTOR, MODEL_REJECTED, NORMALS_DROPPED, NO_GEOMETRY,
    PAPER_SPACE_DROPPED, STORED_EDGES_NOT_WRITTEN, TEXTURE_NOT_WRITTEN, UNITS_UNKNOWN,
    UNSUPPORTED_GEOMETRY, WIRE_BODY_NO_WRITABLE_EDGES, WRITE_FAILED, WRITE_SEED, WRITE_UNIX_TIME,
    ZERO_LENGTH_EDGE,
};

/// SketchUp `.skp` writer for the connector whose descriptor it was built with.
///
/// Holds only the descriptor: the document comes from every [`ExportJob`], so
/// one exporter instance is reusable across exports.
pub struct SkpExporter {
    descriptor: &'static ConnectorDescriptor,
}

impl SkpExporter {
    /// Build a writer for `descriptor` (normally [`crate::DESCRIPTOR_SKP`],
    /// the single declaration of the `skp` connector).
    pub fn new(descriptor: &'static ConnectorDescriptor) -> Self {
        SkpExporter { descriptor }
    }
}

impl Exporter for SkpExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        self.descriptor
    }

    /// The descriptor's registry key: `"skp"`.
    fn format_id(&self) -> &'static str {
        self.descriptor.id
    }

    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput> {
        let expected = self.format_id();
        if format_id != expected {
            return Err(write::refusal(
                MISMATCHED_DESCRIPTOR,
                format!("requested \"{format_id}\", this exporter writes \"{expected}\""),
            ));
        }
        // Only model space exists in this build; the match states that instead of
        // silently projecting whatever `space` names.
        match job.space {
            ExportSpace::Model => {}
        }
        let metres_per_unit = write::metres_per_source_unit(job.units)?;

        let mut report = Report::new(expected);
        let sources = report.collect(job);
        if sources.is_empty() && report.dropped == 0 {
            // An empty model would be an empty success: refuse instead of writing
            // a file that says nothing about a document that holds nothing.
            return Err(write::refusal(
                NO_GEOMETRY,
                "the database has no entities to write",
            ));
        }
        let model = report.build_model(&sources, metres_per_unit)?;
        report.check_limits(job, &model)?;
        let bytes = write::write_model(&model, job)?;
        if bytes.is_empty() {
            return Err(write::refusal(
                WRITE_FAILED,
                "skpwright wrote zero bytes for a non-empty model",
            ));
        }
        let counts = report.finish_counts(bytes.len());
        Ok(ExportOutput {
            bytes,
            report: report.finish(counts),
        })
    }
}

/// One source entity selected for writing.
///
/// A source with triangles is a mesh source; one with no triangles but display
/// polylines is a free-edge (wire) source. `edges` is empty for mesh sources:
/// their stored edges are reported, not written (ADR-0049 §2).
struct SourceGeometry<'a> {
    object: cad_domain::ObjectId,
    type_key: &'a str,
    mesh: &'a cad_domain::Mesh,
    edges: &'a [Vec<cad_domain::Point3>],
}

/// The single merged root run under construction.
#[derive(Default)]
struct MergedRun {
    vertices: Vec<[f64; 3]>,
    faces: Vec<skpwright::MeshFace>,
    edges: Vec<skpwright::MeshEdge>,
    /// Canonical vertex-index pairs already written: one set for regenerated
    /// triangle boundaries and free edges, so no pair is written twice.
    edge_ends: std::collections::HashSet<(u32, u32)>,
    /// Free (wire) edge segments appended, for the edge-semantics note.
    free_edges: usize,
}

impl MergedRun {
    /// The running vertex base, refusing a run that outgrows `u32` indices.
    fn vertex_base(&self) -> CadResult<u32> {
        u32::try_from(self.vertices.len()).map_err(|_| {
            write::refusal(
                MODEL_REJECTED,
                "the merged geometry run holds more than u32::MAX vertices",
            )
        })
    }

    /// Append one source's scaled vertices, refusing an index overflow.
    fn append_vertices(&mut self, scaled: Vec<[f64; 3]>) -> CadResult<u32> {
        let base = self.vertex_base()?;
        if scaled.len() > u32::MAX as usize - base as usize {
            return Err(write::refusal(
                MODEL_REJECTED,
                "the merged geometry run holds more than u32::MAX vertices",
            ));
        }
        self.vertices.extend(scaled);
        Ok(base)
    }

    /// Write one canonical edge pair unless the run already holds it.
    ///
    /// Returns whether a new `MeshEdge` was appended.
    fn push_edge(&mut self, a: u32, b: u32) -> bool {
        let ends = (a.min(b), a.max(b));
        if self.edge_ends.insert(ends) {
            self.edges.push(skpwright::MeshEdge {
                v0: ends.0,
                v1: ends.1,
                ..Default::default()
            });
            true
        } else {
            false
        }
    }

    /// The finished run, or `None` when no face or edge survived.
    fn into_run(self) -> Option<skpwright::GeometryRun> {
        if self.faces.is_empty() && self.edges.is_empty() {
            return None;
        }
        Some(skpwright::GeometryRun {
            def_index: None,
            mesh: skpwright::Mesh {
                vertex_pids: vec![0; self.vertices.len()],
                vertices: self.vertices,
                faces: self.faces,
                edges: self.edges,
                vertices_in: Vec::new(),
            },
            ..Default::default()
        })
    }
}

/// Canonical key of one segment's endpoint coordinates, for de-duplicating a
/// retraced segment inside a polyline.
type CoordinatePair = ([u64; 3], [u64; 3]);

/// Canonical (order-independent) coordinate key of one point.
fn point_key(point: [f64; 3]) -> [u64; 3] {
    [point[0].to_bits(), point[1].to_bits(), point[2].to_bits()]
}

/// Canonical coordinate pair of a segment.
fn coordinate_pair(a: [f64; 3], b: [f64; 3]) -> CoordinatePair {
    let (a, b) = (point_key(a), point_key(b));
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Accumulates the compatibility report while the model is built.
struct Report {
    connector: &'static str,
    entries: Vec<CompatEntry>,
    capabilities: Vec<EntityCapability>,
    /// Geometries that produced no output at all.
    dropped: usize,
    /// Geometries written in full.
    exact: usize,
    /// Every source entity `collect` selected, including ones that later
    /// contribute no geometry (all triangles degenerate, every wire segment
    /// zero-length). This is what `max_output_entities` budgets: a source that
    /// writes nothing still consumed budget, so counting only written sources
    /// would let a document of dropped bodies bypass the limit.
    sources_seen: usize,
    /// Geometries that contributed geometry to the merged run. Kept apart from
    /// [`Report::sources_seen`] because it drives exact reporting and the
    /// one-run grouping note, neither of which should count a silent source.
    sources_written: usize,
    /// Human-facing reasons behind a `Missing`/`Partial` verdict.
    reasons: Vec<String>,
    /// Set when layers/materials/colours had to be flattened.
    flattened: bool,
    /// A source mesh carried per-vertex colours, which a face cannot hold.
    colours_dropped: bool,
    /// A source mesh carried per-vertex normals; faces get their own Newell
    /// normal instead.
    normals_dropped: bool,
    /// Free wire-edge segments written into the merged run.
    free_edges_written: usize,
    /// Zero-length display segments skipped while writing free edges.
    zero_length_edges: usize,
    /// Written meshes that reference a document base-colour texture, one entry
    /// per source entity (the message names the entity and the texture key).
    texture_losses: Vec<(cad_domain::ObjectId, String)>,
    /// How many of those texture entries were recorded by the last model build.
    texture_count: usize,
    /// Mesh-bearing display bodies whose stored edges were not written, one
    /// entry per source entity (ADR-0049 §2).
    stored_edge_losses: Vec<(cad_domain::ObjectId, usize)>,
    /// How many of those edge entries were recorded by the last model build.
    stored_edge_count: usize,
}

impl Report {
    fn new(connector: &'static str) -> Self {
        Report {
            connector,
            entries: Vec::new(),
            capabilities: Vec::new(),
            dropped: 0,
            exact: 0,
            sources_seen: 0,
            sources_written: 0,
            reasons: Vec::new(),
            flattened: false,
            colours_dropped: false,
            normals_dropped: false,
            free_edges_written: 0,
            zero_length_edges: 0,
            texture_losses: Vec::new(),
            texture_count: 0,
            stored_edge_losses: Vec::new(),
            stored_edge_count: 0,
        }
    }

    /// Select the writable sources and record every drop.
    ///
    /// A geometry that is neither a non-empty `Mesh` nor a `Body3d` with a
    /// non-empty display mesh or display edge set is dropped by name; a
    /// non-model-space entity is dropped for its space (the writer has no paper
    /// space).
    fn collect<'db>(&mut self, job: &'db ExportJob<'_>) -> Vec<SourceGeometry<'db>> {
        let mut sources = Vec::new();
        for entity in job.database.entities() {
            if entity.space != cad_domain::SpaceId::Model {
                self.drop_entity(
                    entity,
                    PAPER_SPACE_DROPPED,
                    format!("{:?} space is not written", entity.space),
                );
                continue;
            }
            let (mesh, edges): (&cad_domain::Mesh, &[Vec<cad_domain::Point3>]) = match &entity
                .geometry
            {
                cad_domain::SemanticGeometry::Mesh(mesh) if !mesh.triangles.is_empty() => {
                    (mesh, &[])
                }
                cad_domain::SemanticGeometry::Body3d(body)
                    if !body.display.mesh.triangles.is_empty() =>
                {
                    // The triangles are written; the stored display edges
                    // are not, so they are reported per entity instead of
                    // being dropped silently (ADR-0049 §2).
                    if !body.display.edges.is_empty() {
                        self.stored_edge_losses
                            .push((entity.object.id, body.display.edges.len()));
                    }
                    (&body.display.mesh, &[])
                }
                cad_domain::SemanticGeometry::Body3d(body) if !body.display.edges.is_empty() => {
                    // A wire-only body: its display polylines become free
                    // SketchUp edges in the merged run.
                    (&body.display.mesh, body.display.edges.as_slice())
                }
                // A body with neither triangles nor display edges (and every
                // other geometry) has no SketchUp representation at all.
                _ => {
                    self.drop_entity(
                        entity,
                        UNSUPPORTED_GEOMETRY,
                        "no SketchUp mesh or display-edge representation".to_string(),
                    );
                    continue;
                }
            };
            let attributes = job.database.entity_render_attributes(entity.id);
            if entity.layer.0 != 0 || attributes.color != cad_db::EntityColor::ByLayer {
                self.flattened = true;
            }
            if !mesh.colors.is_empty() {
                self.colours_dropped = true;
            }
            if !mesh.normals.is_empty() {
                self.normals_dropped = true;
            }
            sources.push(SourceGeometry {
                object: entity.object.id,
                type_key: &entity.object.type_key,
                mesh,
                edges,
            });
        }
        self.sources_seen = sources.len();
        sources
    }

    fn drop_entity(&mut self, entity: &cad_db::DbEntity, code: &str, detail: String) {
        self.drop_source(entity.object.id, &entity.object.type_key, code, detail);
    }

    /// Record one named `Unsupported` drop for a source entity.
    ///
    /// Shared by the collection stage (no representation at all) and the build
    /// stage (a wire body whose segments are all zero-length): either way the
    /// entity wrote nothing, so it must count as `dropped` — never `exact`, and
    /// never a silent omission behind a `Partial` note.
    fn drop_source(
        &mut self,
        object: cad_domain::ObjectId,
        type_key: &str,
        code: &str,
        detail: String,
    ) {
        self.dropped += 1;
        let message = format!("{type_key}: {detail}");
        self.reasons.push(message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: Some(object),
            type_key: Some(type_key.to_string()),
            status: SupportStatus::Unsupported,
            code: code.to_string(),
            message,
        });
    }

    /// Turn the selected sources into the **single** merged `skpwright` model.
    ///
    /// `metres_per_unit` scales the database's source units into the metres
    /// `skpwright` expects. Every source shares one root run because the pinned
    /// writer only authors the first root run (`build26.rs`), which is why the
    /// run's vertex and triangle indices carry a running base.
    fn build_model(
        &mut self,
        sources: &[SourceGeometry<'_>],
        metres_per_unit: f64,
    ) -> CadResult<skpwright::Model> {
        let mut run = MergedRun::default();
        let scale = |point: &cad_domain::Point3| {
            [
                point.x * metres_per_unit,
                point.y * metres_per_unit,
                point.z * metres_per_unit,
            ]
        };
        for source in sources {
            if source.mesh.triangles.is_empty() {
                self.append_wire_source(source, &scale, &mut run)?;
            } else {
                self.append_mesh_source(source, &scale, &mut run)?;
            }
        }
        self.free_edges_written = run.free_edges;
        if self.flattened {
            let message = "layers, materials and colours are flattened: every face is written on \
                           the default tag with no material"
                .to_string();
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Unverified,
                code: LAYER_MATERIAL_FLATTENED.to_string(),
                message,
            });
        }
        // Per-vertex attributes have no SketchUp face carrier. Each is reported
        // once, and only when a written source mesh actually carried it.
        if self.colours_dropped {
            let message = "per-vertex colours are dropped: a SketchUp face is coloured by its \
                           material, and this writer assigns none"
                .to_string();
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: COLORS_DROPPED.to_string(),
                message,
            });
        }
        if self.normals_dropped {
            let message = "per-vertex normals are dropped: every written face gets its own \
                           unit Newell normal from its vertices"
                .to_string();
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: NORMALS_DROPPED.to_string(),
                message,
            });
        }
        // The geometry is written; only the base-colour texture is not. One
        // entry per affected source entity, naming the entity and the key.
        let losses = std::mem::take(&mut self.texture_losses);
        for (object, message) in &losses {
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: Some(*object),
                type_key: None,
                status: SupportStatus::Partial,
                code: TEXTURE_NOT_WRITTEN.to_string(),
                message: message.clone(),
            });
        }
        self.texture_count = losses.len();
        // The triangles were written and their boundaries were regenerated as
        // `MeshEdge`s; a mesh-bearing body's stored edge *set/segmentation* is
        // not copied. One entry per affected source entity, real segment count.
        let edge_losses = std::mem::take(&mut self.stored_edge_losses);
        for (object, segments) in &edge_losses {
            let message = format!(
                "entity #{} carries {segments} stored display edge segment(s); its triangles \
                 are written and the writer does not copy the stored edge set or its \
                 segmentation — it regenerates each written triangle's boundary as MeshEdges, so \
                 a face-ring boundary edge survives as a triangle boundary while loose or \
                 separately segmented stored edges do not",
                object.0
            );
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: Some(*object),
                type_key: None,
                status: SupportStatus::Partial,
                code: STORED_EDGES_NOT_WRITTEN.to_string(),
                message,
            });
        }
        self.stored_edge_count = edge_losses.len();
        // One root run is a grouping loss, not a geometry loss: the sources are
        // merged because the writer cannot author a second root run.
        if self.sources_written > 1 {
            let message = format!(
                "{} source entities written into one SketchUp geometry run; per-entity \
                 grouping is not preserved",
                self.sources_written
            );
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: ENTITY_GROUPING_FLATTENED.to_string(),
                message,
            });
        }
        // A free display edge is a polyline segment, not a SketchUp edge record
        // with curve identity, flags, material or layer.
        if self.free_edges_written > 0 {
            let message = format!(
                "{} free display edge segment(s) were written as plain MeshEdges: curve/arc \
                 identity, per-edge soft/smooth/hidden flags, material and layer are not \
                 representable from a display polyline",
                self.free_edges_written
            );
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: EDGE_SEMANTICS_NOT_WRITTEN.to_string(),
                message,
            });
        }
        if self.zero_length_edges > 0 {
            let message = format!(
                "{} zero-length display edge segment(s) were skipped: both endpoints coincide, \
                 which cannot be a SketchUp edge",
                self.zero_length_edges
            );
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: ZERO_LENGTH_EDGE.to_string(),
                message,
            });
        }
        let model = match run.into_run() {
            Some(run) => skpwright::Model {
                geometry: vec![run],
                ..Default::default()
            },
            None => skpwright::Model::default(),
        };
        Ok(model)
    }

    /// Append one triangle-bearing source to the merged run.
    fn append_mesh_source(
        &mut self,
        source: &SourceGeometry<'_>,
        scale: &dyn Fn(&cad_domain::Point3) -> [f64; 3],
        run: &mut MergedRun,
    ) -> CadResult<()> {
        let vertices: Vec<[f64; 3]> = source.mesh.vertices.iter().map(scale).collect();
        Self::check_finite(source, &vertices)?;
        let mut local_faces: Vec<(Vec<u32>, [f64; 3])> =
            Vec::with_capacity(source.mesh.triangles.len());
        let mut boundary: Vec<(u32, u32)> = Vec::new();
        let mut boundary_seen: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::new();
        let mut degenerate = 0usize;
        for triangle in &source.mesh.triangles {
            let ring = [triangle[0], triangle[1], triangle[2]];
            if ring.iter().any(|index| *index as usize >= vertices.len()) {
                return Err(CadError::Invariant(format!(
                    "export.skp.invalid_triangle: object {} references {ring:?} of {}",
                    source.object.0,
                    vertices.len()
                )));
            }
            let a = vertices[ring[0] as usize];
            let b = vertices[ring[1] as usize];
            let c = vertices[ring[2] as usize];
            let Some(normal) = write::triangle_normal(a, b, c) else {
                degenerate += 1;
                continue;
            };
            local_faces.push((ring.to_vec(), normal));
            // `skpwright` refuses a face ring without the edges that close it,
            // so every face boundary becomes one `MeshEdge` per unique
            // undirected vertex pair (shared edges are written once).
            for corner in 0..3 {
                let (a, b) = (ring[corner], ring[(corner + 1) % 3]);
                let ends = (a.min(b), a.max(b));
                if boundary_seen.insert(ends) {
                    boundary.push(ends);
                }
            }
        }
        if degenerate > 0 {
            self.dropped += 1;
            let message = format!(
                "{}: {degenerate} of {} triangles are degenerate (zero area) and were dropped",
                source.type_key,
                source.mesh.triangles.len()
            );
            self.reasons.push(message.clone());
            self.entries.push(CompatEntry {
                connector: self.connector,
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: Some(source.object),
                type_key: Some(source.type_key.to_string()),
                status: SupportStatus::Unsupported,
                code: DEGENERATE_TRIANGLE.to_string(),
                message,
            });
        }
        if local_faces.is_empty() {
            // Nothing survived: contribute no vertices and no run content. The
            // body wrote no surface geometry, so it must not also claim the
            // stored-edge loss collected up front — that would report a loss for
            // an entity whose triangles never reached the file, double-counting
            // the drop.
            self.stored_edge_losses
                .retain(|(object, _)| *object != source.object);
            return Ok(());
        }
        let base = run.append_vertices(vertices)?;
        for (ring, normal) in local_faces {
            run.faces.push(skpwright::MeshFace {
                pid: 0,
                outer: ring.iter().map(|index| index + base).collect(),
                holes: Vec::new(),
                normal,
                front_material: None,
                back_material: None,
                hidden: false,
                casts_shadows: true,
                receives_shadows: true,
                layer: 0,
                texture: None,
            });
        }
        for (a, b) in boundary {
            run.push_edge(base + a, base + b);
        }
        self.sources_written += 1;
        if degenerate == 0 {
            self.exact += 1;
        }
        // The faces are written; a base-colour texture has no carrier here.
        if let Some(key) = source.mesh.texture.as_deref() {
            self.texture_losses.push((
                source.object,
                format!(
                    "entity #{} ({}) references base-colour texture \"{key}\", which the \
                     SketchUp writer does not carry; the faces were written without it",
                    source.object.0, source.type_key
                ),
            ));
        }
        Ok(())
    }

    /// Append one wire-only source's display polylines as free edges.
    ///
    /// Each polyline keeps its own vertices (no welding to mesh vertices or
    /// other polylines); N points become N-1 segments with the interior indices
    /// shared. A polyline that retraces an endpoint pair writes that pair once.
    /// A source that contributes **no** writable segment is a dropped entity
    /// ([`WIRE_BODY_NO_WRITABLE_EDGES`]), not an inexact write.
    fn append_wire_source(
        &mut self,
        source: &SourceGeometry<'_>,
        scale: &dyn Fn(&cad_domain::Point3) -> [f64; 3],
        run: &mut MergedRun,
    ) -> CadResult<()> {
        let mut scaled: Vec<[f64; 3]> = Vec::new();
        let mut segments: Vec<(usize, usize)> = Vec::new();
        let mut zero_length = 0usize;
        for polyline in source.edges {
            let offset = scaled.len();
            let points: Vec<[f64; 3]> = polyline.iter().map(scale).collect();
            Self::check_finite(source, &points)?;
            scaled.extend(points.iter().copied());
            let mut seen: std::collections::HashSet<CoordinatePair> =
                std::collections::HashSet::new();
            for index in 0..points.len().saturating_sub(1) {
                if polyline[index] == polyline[index + 1] {
                    zero_length += 1;
                    continue;
                }
                if !seen.insert(coordinate_pair(points[index], points[index + 1])) {
                    continue;
                }
                segments.push((offset + index, offset + index + 1));
            }
        }
        self.zero_length_edges += zero_length;
        if segments.is_empty() {
            // Nothing writable contributed: every segment was zero-length (or a
            // polyline had a single point). The entity writes no edge at all, so
            // it is a **drop** in addition to the zero-length note: leaving it
            // out of `dropped` would let a document of such bodies export an
            // empty model behind a `Partial` note only.
            self.drop_source(
                source.object,
                source.type_key,
                WIRE_BODY_NO_WRITABLE_EDGES,
                "entity carries only zero-length display edge segments; no edge was written"
                    .to_string(),
            );
            return Ok(());
        }
        let base = run.append_vertices(scaled)?;
        for (a, b) in segments {
            let a = u32::try_from(a).map_err(|_| {
                write::refusal(
                    MODEL_REJECTED,
                    "the merged geometry run holds more than u32::MAX vertices",
                )
            })?;
            let b = u32::try_from(b).map_err(|_| {
                write::refusal(
                    MODEL_REJECTED,
                    "the merged geometry run holds more than u32::MAX vertices",
                )
            })?;
            if run.push_edge(base + a, base + b) {
                run.free_edges += 1;
            }
        }
        self.sources_written += 1;
        // A retraced duplicate pair is dropped by coordinate normalization: the
        // written edge set is unchanged, so that case stays exact. A skipped
        // zero-length segment is real geometry lost, so the source is not exact.
        if zero_length == 0 {
            self.exact += 1;
        }
        Ok(())
    }

    /// Refuse a source whose scaled vertices are not finite.
    fn check_finite(source: &SourceGeometry<'_>, vertices: &[[f64; 3]]) -> CadResult<()> {
        if let Some(bad) = vertices
            .iter()
            .position(|v| v.iter().any(|c| !c.is_finite()))
        {
            return Err(write::refusal(
                MODEL_REJECTED,
                format!(
                    "object {} vertex {bad} is not finite after unit conversion",
                    source.object.0
                ),
            ));
        }
        Ok(())
    }

    /// Refuse by name when the model exceeds the job's explicit budgets.
    ///
    /// `max_output_entities` budgets **collected source entities** (every source
    /// `collect` selected, whether or not it later writes geometry); the merged
    /// model is always one geometry run, so counting runs would hide a blow-up.
    fn check_limits(&self, job: &ExportJob<'_>, model: &skpwright::Model) -> CadResult<()> {
        let entities = self.sources_seen;
        if entities > job.limits.max_output_entities {
            return Err(write::refusal(
                LIMITS_EXCEEDED,
                format!(
                    "{entities} source entities over the {} entity budget",
                    job.limits.max_output_entities
                ),
            ));
        }
        let triangles: usize = model.geometry.iter().map(|run| run.mesh.faces.len()).sum();
        if triangles > job.limits.max_output_triangles {
            return Err(write::refusal(
                LIMITS_EXCEEDED,
                format!(
                    "{triangles} triangles over the {} triangle budget",
                    job.limits.max_output_triangles
                ),
            ));
        }
        Ok(())
    }

    fn finish_counts(&self, bytes: usize) -> CompatCounts {
        CompatCounts {
            exact: self.exact,
            // One conversion per attribute kind actually lost, plus one per
            // affected body whose stored edge set was not copied (so multiple
            // affected bodies count once each, not as a single boolean), plus
            // the merged-run grouping loss and each free-edge note.
            converted: usize::from(self.colours_dropped)
                + usize::from(self.normals_dropped)
                + self.texture_count
                + self.stored_edge_count
                + usize::from(self.sources_written > 1)
                + usize::from(self.free_edges_written > 0)
                + usize::from(self.zero_length_edges > 0),
            dropped: self.dropped,
            bytes,
        }
    }

    /// The report: `Missing` on any drop, `Partial` on notes only, else
    /// `Complete`.
    fn finish(self, counts: CompatCounts) -> CompatReport {
        let completeness = if counts.dropped > 0 {
            Completeness::Missing(self.reasons)
        } else if !self.entries.is_empty() {
            Completeness::Partial(self.reasons)
        } else {
            Completeness::Complete
        };
        CompatReport {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            counts,
            entries: self.entries,
            capabilities: self.capabilities,
            completeness,
        }
    }
}
