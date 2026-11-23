//! glTF 2.0 (`.gltf`) and binary glTF (`.glb`) export writer.
//!
//! The source is always the read-only `cad-db` display model: every
//! [`SemanticGeometry::Mesh`](cad_domain::SemanticGeometry::Mesh) entity plus the
//! display mesh of every [`Body3d`](cad_domain::Body3d) in model space. Everything
//! else is reported as a named drop instead of being silently skipped, and every
//! conversion (dropped normals, dropped colours, dropped degenerate triangles) is
//! reported as well. Per-vertex colours are written as linear `COLOR_0` floats; a
//! mesh without vertex colours writes its explicit entity colour/opacity as a
//! deduplicated glTF material, and a layer-driven (`ByLayer`/`ByBlock`) colour is
//! reported as a note because glTF has no layers. `COLOR_0` alpha is always
//! opaque, so a per-vertex-coloured entity with an explicit opacity below `1.0`
//! is reported as a conversion rather than silently flattened, and its
//! layer-driven opacity joins the same layer note.
//!
//! A mesh that carries UV0 **and** a document-owned base-colour texture writes
//! `TEXCOORD_0`, one `images[]` entry (a bufferView into the same single buffer,
//! so `.gltf` stays one data-URI buffer and GLB stays one BIN chunk), one
//! `textures[]`/`samplers[]` pair and a material with `baseColorTexture`
//! (ADR-0038 D11). A mesh whose UVs or texture could not be written is reported
//! as `export.gltf.texture_not_written` and is not counted as written exactly;
//! nothing is ever sampled from an invented or placeholder image.
//!
//! A mesh that declares PBR channels (`Mesh::pbr`, ADR-0039 D5) writes the
//! metallic-roughness factors, the four texture maps (`metallicRoughnessTexture`,
//! `normalTexture`, `occlusionTexture`, `emissiveTexture`) and the linear
//! `emissiveFactor` on its material, with one `images[]`/`textures[]` pair per
//! distinct document texture key. The material dedup key includes the whole PBR
//! tuple, so two meshes never share a material unless every channel matches. A
//! mesh with `pbr: None` writes **no** PBR key, which keeps its output
//! byte-identical to the pre-ADR-0039 writer; a PBR slot whose bytes the document
//! lacks is the same `texture_not_written` conversion as a lost base-colour
//! texture, and the remaining channels are still written.
//!
//! glTF is **metres, +Y up**; `cad-db` coordinates are +Z up in the document's
//! own source unit. The scale and the frame rotation are applied exactly once,
//! in [`write`], and both are reported as provenance notes. Unknown units are
//! refused rather than assumed.
//!
//! Output is deterministic: source entities are visited in ascending `EntityId`
//! order and no clock, random id or hash-order-dependent value reaches the file,
//! so two exports of one database are byte-identical.
//!
//! The descriptor is supplied by the caller ([`GltfExporter::new`]) so this
//! writer half does not declare connector metadata of its own: the merged crate
//! root declares the single [`crate::DESCRIPTOR_GLTF`] (it describes both
//! directions) and the registry passes it in.

mod collect;
mod pbr;
mod report;
mod texture;
mod write;

use cad_domain::{CadError, CadResult};
use cad_connector_core::{ConnectorDescriptor, ExportJob, ExportOutput, Exporter};

use write::{Written, MISMATCHED_DESCRIPTOR};

/// One glTF 2.0 container spelling.
///
/// Both spellings are the same connector ([`crate::DESCRIPTOR_GLTF`]) but
/// distinct registry keys, so each keeps its own output bytes and report id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GltfContainer {
    /// Binary glTF: 12-byte header, JSON chunk, BIN chunk.
    Glb,
    /// JSON `.gltf` with the buffer embedded as a base64 data URI.
    Gltf,
}

impl GltfContainer {
    /// The registry key this container writes.
    pub fn format_id(self) -> &'static str {
        match self {
            GltfContainer::Glb => "glb",
            GltfContainer::Gltf => "gltf",
        }
    }
}

/// A writer for one glTF container spelling.
///
/// The descriptor is borrowed for the exporter's whole life so the report can
/// name the connector without inventing metadata here. The single glTF
/// descriptor is [`crate::DESCRIPTOR_GLTF`]; this writer declares none.
pub struct GltfExporter {
    container: GltfContainer,
    descriptor: &'static ConnectorDescriptor,
}

impl GltfExporter {
    /// Build a writer for `container` using `descriptor`'s connector id.
    pub fn new(container: GltfContainer, descriptor: &'static ConnectorDescriptor) -> Self {
        GltfExporter {
            container,
            descriptor,
        }
    }

    /// The concrete container this exporter writes.
    pub fn container(&self) -> GltfContainer {
        self.container
    }
}

impl Exporter for GltfExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        self.descriptor
    }

    fn format_id(&self) -> &'static str {
        self.container.format_id()
    }

    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput> {
        // The registry key must name the wire format this exporter actually
        // writes; accepting another key would stamp a report id that does not
        // describe the bytes.
        if format_id != self.container.format_id() {
            return Err(CadError::Unsupported(format!(
                "{MISMATCHED_DESCRIPTOR}: exporter writes {}, not {format_id}",
                self.container.format_id()
            )));
        }

        // glTF positions are metres: an unknown unit is refused before any work.
        let metres_per_unit = write::metres_per_source_unit(job.units)?;

        let mut collected = collect::collect(job, self.descriptor.id)?;
        let mut report = report::ReportBuilder::new(self.descriptor.id);
        for drop in collected.drops.drain(..) {
            report.lost_entity(drop);
        }

        let Written {
            bytes,
            exact_meshes,
        } = write::write(&collected, metres_per_unit, self.container, &mut report)?;
        report.set_exact(exact_meshes);
        report.note(
            write::UNITS_TO_METRES,
            format!(
                "source coordinates were multiplied by {metres_per_unit} to store metres \
                 (glTF positions are metres); the source unit is {:?}",
                job.units.map(|units| units.source.clone())
            ),
        );
        report.note(
            write::FRAME_TO_Y_UP,
            "CAD +Z up coordinates were rotated to glTF +Y up as (x, y, z) -> (x, z, -y); \
             normals use the same rotation without scaling"
                .to_string(),
        );

        Ok(ExportOutput {
            report: report.finish(bytes.len()),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests;
