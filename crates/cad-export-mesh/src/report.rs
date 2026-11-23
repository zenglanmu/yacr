//! Assembly of the shared [`CompatReport`] for a mesh export.
//!
//! One written source mesh is one `exact` count. A conversion (dropped vertex
//! colours, dropped normals, a dropped degenerate facet) is one `converted`
//! entry — or, for degenerate facets, one `dropped` per facet — and a source
//! entity that could not be written is one `dropped` entry. Notes (the unit
//! statement) never change the verdict.

use cad_domain::{Completeness, ObjectId, SupportStatus, UnitContext};
use cad_connector_core::{CompatCounts, CompatEntry, CompatReport, FidelityLevel, ReportDirection};

use crate::collect::{DropEntry, SourceMesh};

pub(crate) struct ReportBuilder {
    connector: &'static str,
    entries: Vec<CompatEntry>,
    exact: usize,
    converted: usize,
    dropped: usize,
    /// Source meshes that lost at least one facet; they are not `exact`.
    degraded_meshes: std::collections::BTreeSet<ObjectId>,
    drop_reasons: Vec<String>,
    convert_reasons: Vec<String>,
}

impl ReportBuilder {
    pub(crate) fn new(connector: &'static str) -> Self {
        ReportBuilder {
            connector,
            entries: Vec::new(),
            exact: 0,
            converted: 0,
            dropped: 0,
            degraded_meshes: std::collections::BTreeSet::new(),
            drop_reasons: Vec::new(),
            convert_reasons: Vec::new(),
        }
    }

    /// Record how many source meshes were written exactly.
    pub(crate) fn set_exact(&mut self, exact: usize) {
        self.exact = exact;
    }

    /// Source meshes that lost at least one facet (and so are not `exact`).
    pub(crate) fn degraded_meshes(&self) -> usize {
        self.degraded_meshes.len()
    }

    /// Record one source entity that could not be written.
    pub(crate) fn lost_entity(&mut self, drop: DropEntry) {
        self.dropped += 1;
        self.drop_reasons.push(drop.message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: Some(drop.object),
            type_key: Some(drop.type_key),
            status: SupportStatus::Unsupported,
            code: drop.code,
            message: drop.message,
        });
    }

    /// Record `count` facets dropped from one source mesh.
    ///
    /// A degenerate facet is lost geometry, so it counts as dropped and makes
    /// the report `Missing`; it is aggregated per source mesh rather than
    /// emitting one entry per facet.
    pub(crate) fn lost_triangles(
        &mut self,
        object: ObjectId,
        type_key: &str,
        count: usize,
        code: &'static str,
        message: String,
    ) {
        self.dropped += count;
        self.degraded_meshes.insert(object);
        self.drop_reasons.push(message.clone());
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

    /// Record one reported conversion (an attribute the format cannot store).
    pub(crate) fn converted(&mut self, code: String, message: String) {
        self.converted += 1;
        self.convert_reasons.push(message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: None,
            type_key: None,
            status: SupportStatus::Partial,
            code,
            message,
        });
    }

    /// Record that one written source mesh's base-colour texture has no channel
    /// in this format.
    ///
    /// The mesh geometry **is** written; only the texture is not, so this is one
    /// conversion per affected source entity (never a drop, never `Missing`).
    /// Neutral-mesh formats carry no texture reference at all and the bytes are
    /// identical to the same mesh exported untextured.
    pub(crate) fn texture_not_written(
        &mut self,
        code: &str,
        format_label: &str,
        mesh: &SourceMesh,
    ) {
        let Some(key) = mesh.texture.as_deref() else {
            return;
        };
        self.converted(
            code.to_string(),
            format!(
                "entity #{} ({}) references base-colour texture \"{key}\", which {format_label} \
                 cannot store; the mesh geometry was written without it",
                mesh.object.0, mesh.type_key
            ),
        );
    }

    /// Record one provenance/evidence note. Notes never change completeness.
    pub(crate) fn note(&mut self, code: String, message: String) {
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: None,
            type_key: None,
            status: SupportStatus::Unverified,
            code,
            message,
        });
    }

    /// Add the one unit note every mesh export carries.
    ///
    /// Neutral-mesh formats have no unit header, so the coordinates stay exactly
    /// as stored; the note states whether the source unit context was known.
    pub(crate) fn units_note(&mut self, units: Option<&UnitContext>, format_id: &str) {
        match units {
            Some(units) => self.note(
                format!("export.{format_id}.units_not_stored"),
                format!(
                    "{format_id} stores no unit header; coordinates were written unscaled in {} ({:?})",
                    units.label(),
                    units.source
                ),
            ),
            None => self.note(
                format!("export.{format_id}.units_unknown"),
                format!(
                    "{format_id} stores no unit header and the source units are unknown; coordinates were written unscaled"
                ),
            ),
        }
    }

    pub(crate) fn finish(self, bytes: usize) -> CompatReport {
        let completeness = if !self.drop_reasons.is_empty() {
            Completeness::Missing(self.drop_reasons)
        } else if !self.convert_reasons.is_empty() {
            Completeness::Partial(self.convert_reasons)
        } else {
            Completeness::Complete
        };
        CompatReport {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            counts: CompatCounts {
                exact: self.exact,
                converted: self.converted,
                dropped: self.dropped,
                bytes,
            },
            entries: self.entries,
            capabilities: Vec::new(),
            completeness,
        }
    }
}
