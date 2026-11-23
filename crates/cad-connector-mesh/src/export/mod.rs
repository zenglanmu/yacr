//! Neutral-mesh export writers (STL, OBJ, PLY, OFF).
//!
//! The source is always the read-only `cad-db` display model: every
//! [`SemanticGeometry::Mesh`](cad_domain::SemanticGeometry::Mesh) entity plus
//! the display mesh of every [`Body3d`](cad_domain::Body3d). Everything else is
//! reported as a named drop instead of being silently skipped, and every
//! conversion (dropped vertex colours, dropped normals, dropped degenerate
//! facets) is reported as well.
//!
//! These formats write **surfaces only**. A body that still writes its
//! triangles but also carries stored display-edge polylines
//! (`BodyDisplay::edges`) gets one per-entity `Partial` conversion
//! (`export.<format>.stored_edges_not_written`) naming the real segment count;
//! the written bytes stay identical to the same mesh without stored edges. A
//! body with stored edges but no triangles is a wire body this format cannot
//! carry at all, so it is dropped as a whole
//! (`export.<format>.wire_body_not_written`) and is never double-reported as
//! written-geometry edge loss. A body whose every facet is dropped by the writer
//! (STL zero-area triangles) likewise wrote no surface, so only its facet-drop
//! entry is reported and no edge-loss entry is added.
//!
//! All writers are deterministic: source entities are visited in ascending
//! `EntityId` order (the database itself stores them in a hash map), and no
//! writer emits a timestamp, a clock reading or a hash-order-dependent value.
//! Two exports of the same database therefore produce byte-identical files.
//!
//! These formats store no unit header. Coordinates are written **unscaled** and
//! the report carries one note saying whether the source units were known.
//!
//! The descriptor is supplied by the caller ([`MeshExporter::new`]) so the
//! writer never invents connector metadata: `cad-app` passes the same
//! `DESCRIPTOR_*` constant the matching reader declares.

mod collect;
mod obj;
mod off;
mod ply;
mod report;
mod stl;

use cad_connector_core::{ConnectorDescriptor, ExportJob, ExportOutput, Exporter};
use cad_domain::{CadError, CadResult};

/// One concrete neutral-mesh wire format.
///
/// `StlBinary` and `StlAscii` share the `stl` descriptor but are distinct
/// registry keys (`stl` / `stl-ascii`), so each keeps its own output and report
/// id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFormat {
    /// Binary STL (80-byte header, `u32` facet count, 50 bytes per facet).
    StlBinary,
    /// ASCII STL (`solid yacr` … `endsolid yacr`).
    StlAscii,
    /// Wavefront OBJ (`v`/`vn`/`f`, one `o` group per source entity).
    Obj,
    /// Binary little-endian PLY 1.0.
    Ply,
    /// ASCII OFF/COFF.
    Off,
}

impl MeshFormat {
    /// The registry key this format writes.
    pub fn format_id(self) -> &'static str {
        match self {
            MeshFormat::StlBinary => "stl",
            MeshFormat::StlAscii => "stl-ascii",
            MeshFormat::Obj => "obj",
            MeshFormat::Ply => "ply",
            MeshFormat::Off => "off",
        }
    }

    /// The `export.<prefix>.*` report-code prefix.
    ///
    /// Both STL containers share `stl` on purpose, matching their colour,
    /// texture and degenerate-facet codes.
    pub(crate) fn report_prefix(self) -> &'static str {
        match self {
            MeshFormat::StlBinary | MeshFormat::StlAscii => "stl",
            MeshFormat::Obj => "obj",
            MeshFormat::Ply => "ply",
            MeshFormat::Off => "off",
        }
    }

    /// The capitalised format name used inside loss messages.
    pub(crate) fn report_label(self) -> &'static str {
        match self {
            MeshFormat::StlBinary | MeshFormat::StlAscii => "STL",
            MeshFormat::Obj => "OBJ",
            MeshFormat::Ply => "PLY",
            MeshFormat::Off => "OFF",
        }
    }

    /// The stable marker for a written body whose stored edge set was not
    /// written (`export.<prefix>.stored_edges_not_written`).
    pub(crate) fn stored_edges_not_written_code(self) -> &'static str {
        match self {
            MeshFormat::StlBinary | MeshFormat::StlAscii => stl::STORED_EDGES_NOT_WRITTEN,
            MeshFormat::Obj => obj::STORED_EDGES_NOT_WRITTEN,
            MeshFormat::Ply => ply::STORED_EDGES_NOT_WRITTEN,
            MeshFormat::Off => off::STORED_EDGES_NOT_WRITTEN,
        }
    }
}

/// A writer for one neutral-mesh format.
///
/// The descriptor is the matching reader's own `const` (`DESCRIPTOR_STL`,
/// `DESCRIPTOR_OBJ`, `DESCRIPTOR_PLY`, `DESCRIPTOR_OFF`, all declared in this
/// crate); it is borrowed for the whole exporter life so the report can name the
/// connector without inventing metadata here.
pub struct MeshExporter {
    format: MeshFormat,
    descriptor: &'static ConnectorDescriptor,
}

impl MeshExporter {
    /// Build a writer for `format` using `descriptor`'s connector id.
    pub fn new(format: MeshFormat, descriptor: &'static ConnectorDescriptor) -> Self {
        MeshExporter { format, descriptor }
    }

    /// The concrete format this exporter writes.
    pub fn format(&self) -> MeshFormat {
        self.format
    }
}

impl Exporter for MeshExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        self.descriptor
    }

    fn format_id(&self) -> &'static str {
        self.format.format_id()
    }

    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput> {
        // The registry key must name the wire format this exporter actually
        // writes. Accepting another key would stamp a report id that does not
        // describe the bytes, so the mismatch is refused by name (the same
        // contract as the acadrust adapter).
        if format_id != self.format.format_id() {
            return Err(CadError::Unsupported(format!(
                "export.{format_id}.mismatched_descriptor: exporter writes {}",
                self.format.format_id()
            )));
        }

        let mut collected = collect::collect(job, format_id, self.format.report_prefix())?;
        let mut report = report::ReportBuilder::new(self.descriptor.id);
        for drop in collected.drops.drain(..) {
            report.lost_entity(drop);
        }

        let bytes = match self.format {
            MeshFormat::StlBinary => stl::write_binary(&collected, &mut report),
            MeshFormat::StlAscii => stl::write_ascii(&collected, &mut report),
            MeshFormat::Obj => obj::write(&collected, &mut report),
            MeshFormat::Ply => ply::write(&collected, &mut report),
            MeshFormat::Off => off::write(&collected, &mut report),
        };
        // Every writer in this module writes surfaces only. A body that still
        // wrote its triangles keeps a count of the stored display edges it could
        // not carry, reported once per affected entity by the shared entry point
        // so all four formats agree. It is a conversion, not a drop, and never
        // subtracts from `exact`.
        let stored_edges_code = self.format.stored_edges_not_written_code();
        let format_label = self.format.report_label();
        for mesh in &collected.meshes {
            report.stored_edges_not_written(stored_edges_code, format_label, mesh);
        }
        // A source mesh that lost at least one facet (STL drops zero-area
        // triangles) was not written exactly, so it must not be counted as such.
        report.set_exact(
            collected
                .meshes
                .len()
                .saturating_sub(report.degraded_meshes()),
        );
        report.units_note(job.units, format_id);

        Ok(ExportOutput {
            report: report.finish(bytes.len()),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests;
