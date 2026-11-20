//! The connector registry: one ordered dispatch table for every drawing format.
//!
//! Imports are decided by an **ordered probe list** (first claiming connector
//! wins) and exports by a `format_id` lookup. The order preserves exactly the
//! decisions the old hardcoded `match` in [`crate::import`] made:
//!
//! `SKP → DGN V7 → OLE (DGN V8 via the DGN adapter's own `sniff`, else RVT) →
//! DWG → DXF → IFC (Part 21 `FILE_SCHEMA`) → STEP → glTF → STL → PLY → OFF → OBJ`,
//! and nothing claims bytes it cannot read (the caller then gets
//! [`crate::import::UNSUPPORTED_INPUT`]).
//!
//! Descriptors are `const`s owned by whichever crate implements the format; this
//! module only wires them together, so the UI can list capabilities without
//! naming an adapter crate.
//!
//! Export keys are split in two: `exports` holds the lanes that write real bytes,
//! and `refusals` holds the keys with no writer in this build. A refusal is an
//! **explicit** `export.<id>.not_implemented`, never an empty success, and a
//! descriptor may only declare `export: Some(..)` when its key is in `exports`.
//!
//! Evidence note: the descriptors' version strings are documentation. The only
//! verification this repository has for the readers is the committed fixture /
//! contract tests and the bounded runs listed in `docs/compatibility.md`; no
//! entry here is a vendor-compatibility claim.

use cad_domain::{CadError, CadResult};
use cad_export_gltf::{GltfContainer, GltfExporter};
use cad_export_ifc::IfcExporter;
use cad_export_mesh::{MeshExporter, MeshFormat};
use cad_export_skp::SkpExporter;
use cad_import_acadrust::export::{AcadrustExporter, AcisVersion, DxfTarget, ExportFormat};
use cad_import_acadrust::AcadrustImporter;
use cad_import_core::{
    detect_format, ConnectorDescriptor, DrawingFormat, ExportJob, ExportOutput, Exporter,
    ImportProgressSink, ImportRequest, ImportedDrawing, Importer,
};
use cad_import_dgn::DgnImporter;
use cad_import_gltf::GltfImporter;
use cad_import_ifc::IfcImporter;
use cad_import_mesh::{OffImporter, PlyImporter};
use cad_import_obj::ObjImporter;
use cad_import_rvt::RvtImporter;
use cad_import_skp::SkpImporter;
use cad_import_step::StepImporter;
use cad_import_stl::StlImporter;
use std::sync::LazyLock;

// Each format crate owns its descriptor; this module only wires the constants
// into the registry. `pub use` keeps the historical `cad_app::DESCRIPTOR_*`
// paths (and the crate-root re-exports) working.
pub use cad_import_acadrust::export::{DESCRIPTOR_DWG, DESCRIPTOR_DXF};
pub use cad_import_dgn::DESCRIPTOR_DGN;
pub use cad_import_gltf::DESCRIPTOR_GLTF;
pub use cad_import_ifc::DESCRIPTOR_IFC;
pub use cad_import_mesh::{DESCRIPTOR_OFF, DESCRIPTOR_PLY};
pub use cad_import_obj::DESCRIPTOR_OBJ;
pub use cad_import_rvt::DESCRIPTOR_RVT;
pub use cad_import_skp::DESCRIPTOR_SKP;
pub use cad_import_step::DESCRIPTOR_STEP;
pub use cad_import_stl::DESCRIPTOR_STL;

/// The production acadrust reader. Not a `const`: it owns decode limits, so it
/// is built once on first use and shared for the process lifetime.
static ACADRUST: LazyLock<AcadrustImporter> = LazyLock::new(AcadrustImporter::new);
static SKP: SkpImporter = SkpImporter;
static DGN: DgnImporter = DgnImporter;
static RVT: RvtImporter = RvtImporter;
static STEP: StepImporter = StepImporter;
static IFC: IfcImporter = IfcImporter;
static GLTF: GltfImporter = GltfImporter;
static STL: StlImporter = StlImporter;
static PLY: PlyImporter = PlyImporter;
static OFF: OffImporter = OffImporter;
static OBJ: ObjImporter = ObjImporter;

/// One import lane: descriptor, reader and its bytes probe.
pub struct ImportConnector {
    pub descriptor: &'static ConnectorDescriptor,
    pub importer: &'static dyn Importer,
    /// `true` when this lane owns the bytes, and an earlier lane's `true` shadows
    /// every later lane.
    ///
    /// Probes are bounded reads, not necessarily header-only:
    /// [`is_dgn_v8`] reads the OLE compound-file directory, so callers of
    /// [`ConnectorRegistry::route`] must apply
    /// [`ImportRequest::limits`]`::max_file_bytes` first.
    pub probe: fn(&[u8]) -> bool,
}

/// One export lane: descriptor, registry key and its writer entry point.
pub struct ExportConnector {
    pub descriptor: &'static ConnectorDescriptor,
    /// The registry key (`"stl"` and `"stl-ascii"` are two keys of one
    /// descriptor), also the id the report is stamped with.
    pub format_id: &'static str,
    /// The writer, called with the concrete registry key so it can stamp the
    /// exact variant into its report instead of only the descriptor's default.
    pub export: fn(&'static str, &ExportJob<'_>, &ExportSettings) -> CadResult<ExportOutput>,
}

/// Export keys that have **no** writer in this build.
///
/// Every pair's descriptor has `export: None` (the tests pin that invariant), so
/// the refusal is the only honest answer. The mesh, IFC and SKP writers are
/// wired as real lanes below; only STEP, DGN and RVT still have no writer.
static REFUSALS: &[(&str, &ConnectorDescriptor)] = &[
    ("step", &DESCRIPTOR_STEP),
    ("dgn", &DESCRIPTOR_DGN),
    ("rvt", &DESCRIPTOR_RVT),
];

/// Host-settable export choices.
///
/// This is `cad-app` state so the neutral contract never names an acadrust type.
/// The default is the pre-selector behaviour (`ACIS 7.0` in a `R2010` DXF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportSettings {
    pub acis_version: AcisVersion,
    pub dxf_target: DxfTarget,
}

/// The ordered connector table used by every carrier.
pub struct ConnectorRegistry {
    imports: Vec<ImportConnector>,
    exports: Vec<ExportConnector>,
    refusals: &'static [(&'static str, &'static ConnectorDescriptor)],
}

impl ConnectorRegistry {
    /// The one production registry: every reader and every export key.
    ///
    /// Cheap to build (small vectors of `&'static` entries), so a carrier may
    /// keep one for the process or build one per call site.
    pub fn builtin() -> Self {
        let imports = vec![
            // The `.skp` header only says "SketchUp model"; the release and its
            // status ceiling are the adapter's answer.
            ImportConnector {
                descriptor: &DESCRIPTOR_SKP,
                importer: &SKP,
                probe: is_skp,
            },
            // DGN V7 has a fixed ISFF header; V8 is an OLE container and is
            // claimed by the OLE probe below through the DGN adapter's `sniff`.
            ImportConnector {
                descriptor: &DESCRIPTOR_DGN,
                importer: &DGN,
                probe: is_dgn,
            },
            // OLE is the one ambiguity: the DGN adapter reads the compound-file
            // directory for its `Dgn~H` root storage, and everything else goes to
            // the RVT adapter (which refuses families by name, never by suffix).
            ImportConnector {
                descriptor: &DESCRIPTOR_DGN,
                importer: &DGN,
                probe: is_dgn_v8,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_RVT,
                importer: &RVT,
                probe: is_ole,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_DWG,
                importer: &*ACADRUST,
                probe: is_dwg,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_DXF,
                importer: &*ACADRUST,
                probe: is_dxf,
            },
            // IFC and STEP share the Part 21 container; `FILE_SCHEMA` is the
            // discriminator, so the STEP lane never sees IFC bytes.
            ImportConnector {
                descriptor: &DESCRIPTOR_IFC,
                importer: &IFC,
                probe: is_ifc,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_STEP,
                importer: &STEP,
                probe: is_step,
            },
            // glTF is self-identifying (GLB header, or a JSON object with the
            // `asset` block), so it runs before the magic-less mesh formats.
            ImportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                importer: &GLTF,
                probe: is_gltf,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_STL,
                importer: &STL,
                probe: is_stl,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_PLY,
                importer: &PLY,
                probe: is_ply,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_OFF,
                importer: &OFF,
                probe: is_off,
            },
            ImportConnector {
                descriptor: &DESCRIPTOR_OBJ,
                importer: &OBJ,
                probe: is_obj,
            },
        ];
        let exports = vec![
            ExportConnector {
                descriptor: &DESCRIPTOR_DXF,
                format_id: "dxf",
                export: export_dxf,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_DWG,
                format_id: "dwg",
                export: export_dwg,
            },
            // Neutral mesh: `stl` and `stl-ascii` are two keys of the one STL
            // descriptor (binary vs. ASCII container).
            ExportConnector {
                descriptor: &DESCRIPTOR_STL,
                format_id: "stl",
                export: export_stl_binary,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_STL,
                format_id: "stl-ascii",
                export: export_stl_ascii,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_OBJ,
                format_id: "obj",
                export: export_obj,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_PLY,
                format_id: "ply",
                export: export_ply,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_OFF,
                format_id: "off",
                export: export_off,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_IFC,
                format_id: "ifc",
                export: export_ifc,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_SKP,
                format_id: "skp",
                export: export_skp,
            },
            // glTF: two containers of one connector (`glb` binary, `gltf` JSON
            // with an embedded base64 buffer).
            ExportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                format_id: "glb",
                export: export_glb,
            },
            ExportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                format_id: "gltf",
                export: export_gltf,
            },
        ];
        ConnectorRegistry {
            imports,
            exports,
            refusals: REFUSALS,
        }
    }

    /// The first lane that claims `bytes`, without importing them.
    ///
    /// This looks only at the bytes: a request is not an argument, so no limit
    /// is applied here and a `Some` is *not* an admission decision. **The caller
    /// must apply [`ImportRequest::limits`]`::max_file_bytes` before calling
    /// this**, because probes are bounded reads rather than header-only sniffs —
    /// the DGN V8 probe ([`is_dgn_v8`]) reads the OLE compound-file directory.
    /// The guard itself lives only in [`Self::import`], which refuses an
    /// over-limit OLE container before that probe runs.
    pub fn route(&self, bytes: &[u8]) -> Option<&'static ConnectorDescriptor> {
        self.imports
            .iter()
            .find(|connector| (connector.probe)(bytes))
            .map(|connector| connector.descriptor)
    }

    /// Import `request.bytes` through the first lane that claims them.
    ///
    /// One probe parses rather than sniffs: the DGN V8 probe reads the OLE
    /// compound-file directory for its `Dgn~H` root storage. An over-limit OLE
    /// container is therefore refused **before** that probe runs. Every other
    /// lane keeps its adapter's own limit check (and its stable limit code), so
    /// this guard is deliberately narrow. Bytes no lane claims are the single
    /// explicit [`crate::import::UNSUPPORTED_INPUT`] refusal.
    pub fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        if is_ole(&request.bytes) && request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }
        let connector = self
            .imports
            .iter()
            .find(|connector| (connector.probe)(&request.bytes))
            .ok_or_else(|| CadError::CorruptData(crate::import::UNSUPPORTED_INPUT.into()))?;
        connector
            .importer
            .import_with_progress(request, cancelled, progress)
    }

    /// Export `job` with the lane registered for `format_id`.
    ///
    /// Resolution order is fixed:
    ///
    /// 1. a registered lane writes real bytes through its writer;
    /// 2. a key in the refusal table fails with
    ///    `export.<id>.not_implemented: writer is not wired in this build`;
    /// 3. anything else is `export.unknown_format`.
    ///
    /// A key is registered **iff** its descriptor declares
    /// [`ExportCapability`](cad_import_core::ExportCapability) and the call
    /// produces bytes; no key silently writes nothing and no descriptor
    /// advertises a writer this build does not have.
    pub fn export(
        &self,
        format_id: &str,
        job: &ExportJob<'_>,
        settings: &ExportSettings,
    ) -> CadResult<ExportOutput> {
        if let Some(connector) = self
            .exports
            .iter()
            .find(|connector| connector.format_id == format_id)
        {
            return (connector.export)(connector.format_id, job, settings);
        }
        if self.refusals.iter().any(|(key, _)| *key == format_id) {
            return Err(CadError::Unsupported(format!(
                "export.{format_id}.not_implemented: writer is not wired in this build"
            )));
        }
        Err(CadError::Unsupported(format!(
            "export.unknown_format: {format_id}"
        )))
    }

    /// Every descriptor, once each, in import-probe order.
    ///
    /// DGN appears twice in the probe list (V7 header, V8 OLE storage), so the
    /// list is de-duplicated by id. Every registered descriptor has an import
    /// lane (a read-only format included), so probing order covers the set.
    pub fn descriptors(&self) -> impl Iterator<Item = &'static ConnectorDescriptor> {
        let mut descriptors: Vec<&'static ConnectorDescriptor> =
            Vec::with_capacity(self.imports.len());
        for connector in &self.imports {
            if !descriptors
                .iter()
                .any(|seen| seen.id == connector.descriptor.id)
            {
                descriptors.push(connector.descriptor);
            }
        }
        descriptors.into_iter()
    }

    /// The descriptor registered under `id`, when the registry knows it.
    ///
    /// Resolves every import descriptor, every registered export key (including
    /// `"stl-ascii"`, which has no descriptor of its own) and every refusal key,
    /// so a caller can ask about any id it can pass to [`Self::export`].
    pub fn descriptor(&self, id: &str) -> Option<&'static ConnectorDescriptor> {
        if let Some(connector) = self
            .exports
            .iter()
            .find(|connector| connector.format_id == id)
        {
            return Some(connector.descriptor);
        }
        if let Some((_, descriptor)) = self.refusals.iter().find(|(key, _)| *key == id) {
            return Some(descriptor);
        }
        self.imports
            .iter()
            .map(|connector| connector.descriptor)
            .find(|descriptor| descriptor.id == id)
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

// Probes. Each one is a bounded header read through the shared detector, so the
// registry and `detect_format` cannot drift into two different answers.

fn is_skp(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Skp)
}

fn is_dgn(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dgn)
}

fn is_ole(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ole)
}

/// The DGN V8 lane: an OLE container whose compound-file directory carries the
/// `Dgn~H` root storage. It must run before the RVT lane.
fn is_dgn_v8(bytes: &[u8]) -> bool {
    is_ole(bytes) && DgnImporter::sniff(bytes)
}

fn is_dwg(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dwg)
}

fn is_dxf(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dxf)
}

fn is_ifc(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ifc)
}

fn is_step(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Step)
}

/// The glTF lane asks the adapter's own sniffer: it already owns the exact GLB
/// header and JSON-object rules, so the registry cannot drift from the reader.
fn is_gltf(bytes: &[u8]) -> bool {
    cad_import_gltf::sniff(bytes)
}

fn is_stl(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Stl)
}

fn is_ply(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ply)
}

fn is_off(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Off)
}

fn is_obj(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Obj)
}

/// DXF through the shared contract.
fn export_dxf(
    format_id: &'static str,
    job: &ExportJob<'_>,
    settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    AcadrustExporter::new(
        ExportFormat::DxfText,
        settings.acis_version,
        settings.dxf_target,
    )
    .export(format_id, job)
}

/// DWG through the shared contract.
///
/// A DWG is always AC1032 by the upstream writer, whose version comes from the
/// document; the configured DXF container is a DXF-only choice, so the request
/// names `R2018` exactly as the host does today instead of asking the writer for
/// a version it would ignore.
fn export_dwg(
    format_id: &'static str,
    job: &ExportJob<'_>,
    settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    AcadrustExporter::new(ExportFormat::Dwg, settings.acis_version, DxfTarget::R2018)
        .export(format_id, job)
}

/// One neutral-mesh writer lane.
///
/// The exporter is built per call: it holds only the descriptor reference, so
/// there is no state to cache and no way to dispatch to the wrong container.
fn export_mesh(
    format: MeshFormat,
    descriptor: &'static ConnectorDescriptor,
    format_id: &'static str,
    job: &ExportJob<'_>,
) -> CadResult<ExportOutput> {
    MeshExporter::new(format, descriptor).export(format_id, job)
}

/// Binary STL (`stl`).
fn export_stl_binary(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::StlBinary, &DESCRIPTOR_STL, format_id, job)
}

/// ASCII STL (`stl-ascii`, a second key of the STL descriptor).
fn export_stl_ascii(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::StlAscii, &DESCRIPTOR_STL, format_id, job)
}

/// Wavefront OBJ.
fn export_obj(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Obj, &DESCRIPTOR_OBJ, format_id, job)
}

/// Binary little-endian PLY.
fn export_ply(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Ply, &DESCRIPTOR_PLY, format_id, job)
}

/// ASCII OFF/COFF.
fn export_off(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Off, &DESCRIPTOR_OFF, format_id, job)
}

/// IFC4 (`IfcTriangulatedFaceSet`) through the shared contract.
fn export_ifc(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    IfcExporter::new(&DESCRIPTOR_IFC).export(format_id, job)
}

/// SketchUp 2026/2027 container through the shared contract.
fn export_skp(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    SkpExporter::new(&DESCRIPTOR_SKP).export(format_id, job)
}

/// Binary glTF (`glb`).
fn export_glb(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_gltf_container(GltfContainer::Glb, format_id, job)
}

/// JSON glTF (`gltf`, buffer embedded as a base64 data URI).
fn export_gltf(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_gltf_container(GltfContainer::Gltf, format_id, job)
}

/// One glTF container lane. Built per call like the mesh writers: the exporter
/// holds only the container and the descriptor reference.
fn export_gltf_container(
    container: GltfContainer,
    format_id: &'static str,
    job: &ExportJob<'_>,
) -> CadResult<ExportOutput> {
    GltfExporter::new(container, &DESCRIPTOR_GLTF).export(format_id, job)
}

#[cfg(test)]
mod tests;
