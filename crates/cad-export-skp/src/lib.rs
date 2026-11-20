//! SketchUp (`.skp`) export writer.
//!
//! `skpwright` 0.6.0 authors the **2026 container** (`{26.x}`, a ZIP archive of
//! tagged records) from a [`skpwright::Model`]. Its mesh vertices are metres and
//! the crate converts them to the inches the container stores, so this writer
//! must scale `cad-db` coordinates by the document's **declared** source unit.
//! There is no default unit: a job without a known unit is refused
//! ([`UNITS_UNKNOWN`]) rather than guessed.
//!
//! Scope: a projection of the `cad-db` authority into SketchUp faces.
//!
//! * One [`skpwright::GeometryRun`] per source mesh entity (`SemanticGeometry::Mesh`,
//!   or a body's non-empty `display.mesh`), root run (`def_index: None`).
//! * Every triangle becomes one triangular [`skpwright::MeshFace`] with its own
//!   unit Newell normal. Collinear/zero-area triangles are dropped by name
//!   ([`DEGENERATE_TRIANGLE`]).
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
//! ```text
//! cad-db (source units) --× metres per source unit--> skpwright::Model (metres)
//!     --Document::from_model_with(WriteEnv)--> 2026 container bytes
//! ```

mod write;

#[cfg(test)]
mod tests;

use cad_domain::{CadError, CadResult, Completeness, EntityCapability, SupportStatus};
use cad_import_core::{
    CompatCounts, CompatEntry, CompatReport, ConnectorDescriptor, ExportJob, ExportOutput,
    ExportSpace, Exporter, FidelityLevel, ReportDirection,
};

pub use write::{
    export_env, metres_per_source_unit, COLORS_DROPPED, DEGENERATE_TRIANGLE,
    LAYER_MATERIAL_FLATTENED, LIMITS_EXCEEDED, MISMATCHED_DESCRIPTOR, MODEL_REJECTED,
    NORMALS_DROPPED, NO_GEOMETRY, PAPER_SPACE_DROPPED, TEXTURE_NOT_WRITTEN, UNITS_UNKNOWN,
    UNSUPPORTED_GEOMETRY, WRITE_FAILED, WRITE_SEED, WRITE_UNIX_TIME,
};

/// SketchUp `.skp` writer for the connector whose descriptor it was built with.
///
/// Holds only the descriptor: the document comes from every [`ExportJob`], so
/// one exporter instance is reusable across exports.
pub struct SkpExporter {
    descriptor: &'static ConnectorDescriptor,
}

impl SkpExporter {
    /// Build a writer for `descriptor` (normally `cad_import_skp::DESCRIPTOR_SKP`,
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

/// One source mesh entity selected for writing.
struct SourceMesh<'a> {
    object: cad_domain::ObjectId,
    type_key: &'a str,
    mesh: &'a cad_domain::Mesh,
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
    /// Human-facing reasons behind a `Missing`/`Partial` verdict.
    reasons: Vec<String>,
    /// Set when layers/materials/colours had to be flattened.
    flattened: bool,
    /// A source mesh carried per-vertex colours, which a face cannot hold.
    colours_dropped: bool,
    /// A source mesh carried per-vertex normals; faces get their own Newell
    /// normal instead.
    normals_dropped: bool,
    /// Written meshes that reference a document base-colour texture, one entry
    /// per source entity (the message names the entity and the texture key).
    texture_losses: Vec<(cad_domain::ObjectId, String)>,
    /// How many of those texture entries were recorded by the last model build.
    texture_count: usize,
}

impl Report {
    fn new(connector: &'static str) -> Self {
        Report {
            connector,
            entries: Vec::new(),
            capabilities: Vec::new(),
            dropped: 0,
            exact: 0,
            reasons: Vec::new(),
            flattened: false,
            colours_dropped: false,
            normals_dropped: false,
            texture_losses: Vec::new(),
            texture_count: 0,
        }
    }

    /// Select the writable meshes and record every drop.
    ///
    /// A geometry that is neither a non-empty `Mesh` nor a body with a non-empty
    /// display mesh is dropped by name; a non-model-space entity is dropped for
    /// its space (the writer has no paper space).
    fn collect<'db>(&mut self, job: &'db ExportJob<'_>) -> Vec<SourceMesh<'db>> {
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
            let mesh = match &entity.geometry {
                cad_domain::SemanticGeometry::Mesh(mesh) if !mesh.triangles.is_empty() => mesh,
                cad_domain::SemanticGeometry::Body3d(body)
                    if !body.display.mesh.triangles.is_empty() =>
                {
                    &body.display.mesh
                }
                _ => {
                    self.drop_entity(
                        entity,
                        UNSUPPORTED_GEOMETRY,
                        "no SketchUp mesh representation".to_string(),
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
            sources.push(SourceMesh {
                object: entity.object.id,
                type_key: &entity.object.type_key,
                mesh,
            });
        }
        sources
    }

    fn drop_entity(&mut self, entity: &cad_db::DbEntity, code: &str, detail: String) {
        self.dropped += 1;
        let type_key = entity.object.type_key.clone();
        let message = format!("{type_key}: {detail}");
        self.reasons.push(message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: Some(entity.object.id),
            type_key: Some(type_key),
            status: SupportStatus::Unsupported,
            code: code.to_string(),
            message,
        });
    }

    /// Turn the selected meshes into the `skpwright` model.
    ///
    /// `metres_per_unit` scales the database's source units into the metres
    /// `skpwright` expects.
    fn build_model(
        &mut self,
        sources: &[SourceMesh<'_>],
        metres_per_unit: f64,
    ) -> CadResult<skpwright::Model> {
        let mut model = skpwright::Model::default();
        for source in sources {
            let vertices: Vec<[f64; 3]> = source
                .mesh
                .vertices
                .iter()
                .map(|point| {
                    [
                        point.x * metres_per_unit,
                        point.y * metres_per_unit,
                        point.z * metres_per_unit,
                    ]
                })
                .collect();
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
            let mut faces = Vec::with_capacity(source.mesh.triangles.len());
            let mut edges = Vec::new();
            let mut edge_ends: std::collections::HashSet<(u32, u32)> =
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
                faces.push(skpwright::MeshFace {
                    pid: 0,
                    outer: ring.to_vec(),
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
                // `skpwright` refuses a face ring without the edges that close it,
                // so every face boundary becomes one `MeshEdge` per unique
                // undirected vertex pair (shared edges are written once).
                for corner in 0..3 {
                    let (a, b) = (ring[corner], ring[(corner + 1) % 3]);
                    let ends = (a.min(b), a.max(b));
                    if edge_ends.insert(ends) {
                        edges.push(skpwright::MeshEdge {
                            v0: ends.0,
                            v1: ends.1,
                            ..Default::default()
                        });
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
            if faces.is_empty() {
                // Nothing survived: do not append an empty geometry run.
                continue;
            }
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
            model.geometry.push(skpwright::GeometryRun {
                def_index: None,
                mesh: skpwright::Mesh {
                    vertex_pids: vec![0; vertices.len()],
                    vertices,
                    faces,
                    edges,
                    vertices_in: Vec::new(),
                },
                ..Default::default()
            });
        }
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
        Ok(model)
    }

    /// Refuse by name when the model exceeds the job's explicit budgets.
    fn check_limits(&self, job: &ExportJob<'_>, model: &skpwright::Model) -> CadResult<()> {
        let runs = model.geometry.len();
        if runs > job.limits.max_output_entities {
            return Err(write::refusal(
                LIMITS_EXCEEDED,
                format!(
                    "{runs} geometry runs over the {} entity budget",
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
            // One conversion per attribute kind actually lost, matching the
            // `Partial` entries appended in `model_and_report`.
            converted: usize::from(self.colours_dropped)
                + usize::from(self.normals_dropped)
                + self.texture_count,
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
