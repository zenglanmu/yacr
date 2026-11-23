//! Assembly of the shared [`CompatReport`] for a glTF export.
//!
//! One written source mesh is one `exact` count. An attribute the file cannot
//! carry (a partially present normal/colour set, an explicit entity colour
//! masked by per-vertex colours, UV0 or a texture that could not be written) is
//! one `converted` entry; a dropped degenerate triangle is counted per triangle;
//! a source entity that could not be written is one `dropped` entry. Notes (the
//! unit/frame statement, the absent layer appearance) never change the verdict.
//!
//! `exact` is set from the writer's own count of meshes that lost no triangle
//! and no UV/texture; a drop entry never subtracts from it.

use cad_connector_core::{CompatCounts, CompatEntry, CompatReport, FidelityLevel, ReportDirection};
use cad_domain::{Completeness, ObjectId, SupportStatus};

use crate::export::collect::DropEntry;

pub(crate) struct ReportBuilder {
    connector: &'static str,
    entries: Vec<CompatEntry>,
    exact: usize,
    converted: usize,
    dropped: usize,
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
            drop_reasons: Vec::new(),
            convert_reasons: Vec::new(),
        }
    }

    /// Record how many source meshes were written exactly.
    pub(crate) fn set_exact(&mut self, exact: usize) {
        self.exact = exact;
    }

    /// Record one source entity that could not be written.
    ///
    /// `exact` is not derived from this: the writer reports the meshes it wrote
    /// without losing a triangle through [`Self::set_exact`], so a drop never
    /// has to be subtracted from a running total.
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

    /// Record `count` triangles dropped from one source mesh.
    ///
    /// A degenerate triangle is lost geometry, so it counts as dropped and makes
    /// the report `Missing`; it is aggregated per source mesh rather than
    /// emitting one entry per triangle.
    pub(crate) fn lost_triangles(
        &mut self,
        object: ObjectId,
        type_key: &str,
        count: usize,
        code: &'static str,
        message: String,
    ) {
        self.dropped += count;
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

    /// Record one reported conversion (an attribute glTF cannot carry for this
    /// document).
    pub(crate) fn converted(&mut self, code: &'static str, message: String) {
        self.converted += 1;
        self.convert_reasons.push(message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: None,
            type_key: None,
            status: SupportStatus::Partial,
            code: code.to_string(),
            message,
        });
    }

    /// Record one reported conversion tied to a source entity.
    ///
    /// The geometry **is** written; only an attribute it carries is not, so this
    /// is one conversion per affected entity (never a drop).
    pub(crate) fn converted_entity(
        &mut self,
        code: &'static str,
        object: ObjectId,
        type_key: &str,
        message: String,
    ) {
        self.converted += 1;
        self.convert_reasons.push(message.clone());
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: Some(object),
            type_key: Some(type_key.to_string()),
            status: SupportStatus::Partial,
            code: code.to_string(),
            message,
        });
    }

    /// Record one provenance/evidence note. Notes never change completeness.
    pub(crate) fn note(&mut self, code: &'static str, message: String) {
        self.entries.push(CompatEntry {
            connector: self.connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: None,
            type_key: None,
            status: SupportStatus::Unverified,
            code: code.to_string(),
            message,
        });
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
