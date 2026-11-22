//! The connector registry: one ordered dispatch table for every drawing format.
//!
//! Imports are decided by an **ordered probe list** (first claiming connector
//! wins) and exports by a `format_id` lookup. The order preserves exactly the
//! decisions the old hardcoded `match` in [`crate::import`] made:
//!
//! `SKP → DGN V7 → OLE (DGN V8 via the DGN adapter's own `sniff`, else RVT) →
//! DWG → DXF → IFC (Part 21 `FILE_SCHEMA`) → STEP → glTF → STL → PLY → OFF → OBJ`,
//! and nothing claims bytes it cannot read (the caller then gets
//! [`unclaimed_input`]).
//!
//! A build may compile a subset of the lanes out (`formats-*` features). Every
//! absent adapter is gated here, and the bytes it would have claimed are answered
//! by [`absent_import_reason`] instead: a compiled-out format is a named
//! `unsupported-runtime` refusal, never a generic "not a drawing".
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
#[cfg(feature = "formats-gltf")]
use cad_export_gltf::{GltfContainer, GltfExporter};
#[cfg(feature = "formats-ifc")]
use cad_export_ifc::IfcExporter;
#[cfg(feature = "formats-mesh")]
use cad_export_mesh::{MeshExporter, MeshFormat};
#[cfg(feature = "formats-skp")]
use cad_export_skp::SkpExporter;
use cad_import_acadrust::export::{AcadrustExporter, AcisVersion, DxfTarget, ExportFormat};
use cad_import_acadrust::AcadrustImporter;
use cad_import_core::{
    detect_format, ConnectorDescriptor, DrawingFormat, ExportJob, ExportOutput, Exporter,
    ImportProgressSink, ImportRequest, ImportedDrawing, Importer, SpecVisibility,
};
#[cfg(feature = "formats-dgn")]
use cad_import_dgn::DgnImporter;
#[cfg(feature = "formats-gltf")]
use cad_import_gltf::GltfImporter;
#[cfg(feature = "formats-ifc")]
use cad_import_ifc::IfcImporter;
#[cfg(feature = "formats-mesh")]
use cad_import_mesh::{OffImporter, PlyImporter};
#[cfg(feature = "formats-mesh")]
use cad_import_obj::ObjImporter;
#[cfg(feature = "formats-rvt")]
use cad_import_rvt::RvtImporter;
#[cfg(feature = "formats-skp")]
use cad_import_skp::SkpImporter;
#[cfg(feature = "formats-step")]
use cad_import_step::StepImporter;
#[cfg(feature = "formats-mesh")]
use cad_import_stl::StlImporter;
use std::sync::LazyLock;

// Each format crate owns its descriptor; this module only wires the constants
// into the registry. `pub use` keeps the historical `cad_app::DESCRIPTOR_*`
// paths (and the crate-root re-exports) working. A descriptor is only visible
// when its adapter is compiled in, so a slim registry cannot name a format it
// cannot back.
pub use cad_import_acadrust::export::{DESCRIPTOR_DWG, DESCRIPTOR_DXF};
#[cfg(feature = "formats-dgn")]
pub use cad_import_dgn::DESCRIPTOR_DGN;
#[cfg(feature = "formats-gltf")]
pub use cad_import_gltf::DESCRIPTOR_GLTF;
#[cfg(feature = "formats-ifc")]
pub use cad_import_ifc::DESCRIPTOR_IFC;
#[cfg(feature = "formats-mesh")]
pub use cad_import_mesh::{DESCRIPTOR_OFF, DESCRIPTOR_PLY};
#[cfg(feature = "formats-mesh")]
pub use cad_import_obj::DESCRIPTOR_OBJ;
#[cfg(feature = "formats-rvt")]
pub use cad_import_rvt::DESCRIPTOR_RVT;
#[cfg(feature = "formats-skp")]
pub use cad_import_skp::DESCRIPTOR_SKP;
#[cfg(feature = "formats-step")]
pub use cad_import_step::DESCRIPTOR_STEP;
#[cfg(feature = "formats-mesh")]
pub use cad_import_stl::DESCRIPTOR_STL;

/// Whether this build compiled every format lane.
///
/// Hosts and contract tests read it to state capabilities; the shipped
/// `dom-canvas` artifact is the one build where it is `false`.
pub const ALL_FORMATS_COMPILED: bool = cfg!(feature = "formats-all");

/// The production acadrust reader. Not a `const`: it owns decode limits, so it
/// is built once on first use and shared for the process lifetime.
static ACADRUST: LazyLock<AcadrustImporter> = LazyLock::new(AcadrustImporter::new);
#[cfg(feature = "formats-skp")]
static SKP: SkpImporter = SkpImporter;
#[cfg(feature = "formats-dgn")]
static DGN: DgnImporter = DgnImporter;
#[cfg(feature = "formats-rvt")]
static RVT: RvtImporter = RvtImporter;
#[cfg(feature = "formats-step")]
static STEP: StepImporter = StepImporter;
#[cfg(feature = "formats-ifc")]
static IFC: IfcImporter = IfcImporter;
#[cfg(feature = "formats-gltf")]
static GLTF: GltfImporter = GltfImporter;
#[cfg(feature = "formats-mesh")]
static STL: StlImporter = StlImporter;
#[cfg(feature = "formats-mesh")]
static PLY: PlyImporter = PlyImporter;
#[cfg(feature = "formats-mesh")]
static OFF: OffImporter = OffImporter;
#[cfg(feature = "formats-mesh")]
static OBJ: ObjImporter = ObjImporter;

/// One import lane: descriptor, reader and its bytes probe.
pub struct ImportConnector {
    pub descriptor: &'static ConnectorDescriptor,
    pub importer: &'static dyn Importer,
    /// `true` when this lane owns the bytes, and an earlier lane's `true` shadows
    /// every later lane.
    ///
    /// Probes are bounded reads, not necessarily header-only: the DGN V8 probe
    /// reads the OLE compound-file directory, so callers of
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

/// Every export key this product knows, whether or not this build compiles a
/// writer for it.
///
/// A slim build has no descriptor for a compiled-out format, but the key is
/// still a format the product supports elsewhere. Naming it here keeps STEP, DGN
/// and RVT from degrading to `export.unknown_format` when their writers are
/// compiled out.
pub const KNOWN_EXPORT_KEYS: &[&str] = &[
    "dxf",
    "dwg",
    "stl",
    "stl-ascii",
    "obj",
    "ply",
    "off",
    "ifc",
    "skp",
    "glb",
    "gltf",
    "step",
    "dgn",
    "rvt",
];

/// The stable refusal for an import format this build did not compile in.
///
/// `compiled` is the ids of the lanes the registry actually holds. The match is
/// exhaustive on purpose: a new [`DrawingFormat`] variant must fail to compile
/// here instead of silently falling through to a generic message. `Unknown` is
/// not a format, so it has no refusal.
pub fn absent_import_reason(format: DrawingFormat, compiled: &[&str]) -> Option<String> {
    let id = match format {
        DrawingFormat::Dwg => "dwg",
        DrawingFormat::Dxf => "dxf",
        DrawingFormat::Step => "step",
        DrawingFormat::Ifc => "ifc",
        DrawingFormat::Obj => "obj",
        DrawingFormat::Stl => "stl",
        DrawingFormat::Ply => "ply",
        DrawingFormat::Off => "off",
        DrawingFormat::Gltf => "gltf",
        DrawingFormat::Dgn => "dgn",
        DrawingFormat::Ole => "ole",
        DrawingFormat::Skp => "skp",
        DrawingFormat::Unknown => return None,
    };
    (!compiled.contains(&id)).then(|| format!("unsupported-runtime: import.{id}"))
}

/// The stable refusal for a known export key this build has no writer for.
///
/// A key in `compiled_keys` resolves to `None` (the lane is expected to run);
/// every other [`KNOWN_EXPORT_KEYS`] entry is `unsupported-runtime: export.<key>`.
/// An unknown key keeps the registry's `export.unknown_format` answer.
pub fn absent_export_reason(key: &str, compiled_keys: &[&str]) -> Option<String> {
    if compiled_keys.contains(&key) {
        return None;
    }
    KNOWN_EXPORT_KEYS
        .contains(&key)
        .then(|| format!("unsupported-runtime: export.{key}"))
}

/// The refusal for bytes no compiled lane claims.
///
/// A full build returns [`crate::import::UNSUPPORTED_INPUT`] verbatim, so the
/// message every carrier has always produced is unchanged. A slim build names
/// only the extensions it actually compiled in: telling a user about a format
/// this artifact cannot read would be a claim the build cannot honour.
pub fn unclaimed_input() -> &'static str {
    #[cfg(feature = "formats-all")]
    {
        crate::import::UNSUPPORTED_INPUT
    }
    #[cfg(not(feature = "formats-all"))]
    {
        static SLIM: LazyLock<String> = LazyLock::new(|| {
            let extensions: Vec<&str> = ConnectorRegistry::builtin()
                .descriptors()
                .flat_map(|descriptor| descriptor.extensions.iter().copied())
                .collect();
            format!(
                "input is not a supported drawing: this build reads only {}",
                extensions.join(", ")
            )
        });
        SLIM.as_str()
    }
}

/// Host-settable export choices.
///
/// This is `cad-app` state so the neutral contract never names an acadrust type.
/// The default is the pre-selector behaviour (`ACIS 7.0` in a `R2010` DXF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportSettings {
    pub acis_version: AcisVersion,
    pub dxf_target: DxfTarget,
}

/// The provenance of the connector that opened the current document (ADR-0041
/// D5), for the `cad_status.openedSource` notice.
///
/// Only a [`SpecVisibility::ReverseEngineered`] connector produces one: a
/// published-spec format has nothing to warn about, so the mapping in
/// [`ConnectorRegistry::opened_source`] returns `None` for it and the wire key
/// is omitted. `can_write`/`experimental` are the descriptor's own export
/// capability — never inferred from the file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedSource {
    /// The stable registry connector id (`"dwg"`, `"dgn"`, `"skp"`, `"rvt"`).
    pub connector: &'static str,
    /// The format's spec visibility (always ReverseEngineered for a `Some`).
    pub spec: SpecVisibility,
    /// Whether the connector declares an export direction (ADR-0041 D1).
    pub can_write: bool,
    /// Whether that writer is unverified/experimental (ADR-0041 D1).
    pub experimental: bool,
}

impl OpenedSource {
    /// Whether the format's specification is published (never `true` for a
    /// value this host produces: a public source yields no `openedSource`).
    ///
    /// Exposes the classification to the wire layer without making it depend on
    /// `cad-import-core` just to name the enum.
    pub fn spec_is_public(&self) -> bool {
        matches!(self.spec, SpecVisibility::Public)
    }
}

/// The ordered connector table used by every carrier.
pub struct ConnectorRegistry {
    imports: Vec<ImportConnector>,
    exports: Vec<ExportConnector>,
    /// Export keys with no writer **in this build**. A slim build cfg-pushes
    /// only the descriptors it compiled, so it never refuses a format by a
    /// `not_implemented` code it does not know.
    refusals: Vec<(&'static str, &'static ConnectorDescriptor)>,
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
            #[cfg(feature = "formats-skp")]
            ImportConnector {
                descriptor: &DESCRIPTOR_SKP,
                importer: &SKP,
                probe: is_skp,
            },
            // DGN V7 has a fixed ISFF header; V8 is an OLE container and is
            // claimed by the OLE probe below through the DGN adapter's `sniff`.
            #[cfg(feature = "formats-dgn")]
            ImportConnector {
                descriptor: &DESCRIPTOR_DGN,
                importer: &DGN,
                probe: is_dgn,
            },
            // OLE is the one ambiguity: the DGN adapter reads the compound-file
            // directory for its `Dgn~H` root storage, and everything else goes to
            // the RVT adapter (which refuses families by name, never by suffix).
            #[cfg(feature = "formats-dgn")]
            ImportConnector {
                descriptor: &DESCRIPTOR_DGN,
                importer: &DGN,
                probe: is_dgn_v8,
            },
            #[cfg(feature = "formats-rvt")]
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
            #[cfg(feature = "formats-ifc")]
            ImportConnector {
                descriptor: &DESCRIPTOR_IFC,
                importer: &IFC,
                probe: is_ifc,
            },
            #[cfg(feature = "formats-step")]
            ImportConnector {
                descriptor: &DESCRIPTOR_STEP,
                importer: &STEP,
                probe: is_step,
            },
            // glTF is self-identifying (GLB header, or a JSON object with the
            // `asset` block), so it runs before the magic-less mesh formats.
            #[cfg(feature = "formats-gltf")]
            ImportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                importer: &GLTF,
                probe: is_gltf,
            },
            #[cfg(feature = "formats-mesh")]
            ImportConnector {
                descriptor: &DESCRIPTOR_STL,
                importer: &STL,
                probe: is_stl,
            },
            #[cfg(feature = "formats-mesh")]
            ImportConnector {
                descriptor: &DESCRIPTOR_PLY,
                importer: &PLY,
                probe: is_ply,
            },
            #[cfg(feature = "formats-mesh")]
            ImportConnector {
                descriptor: &DESCRIPTOR_OFF,
                importer: &OFF,
                probe: is_off,
            },
            #[cfg(feature = "formats-mesh")]
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
            #[cfg(feature = "formats-mesh")]
            ExportConnector {
                descriptor: &DESCRIPTOR_STL,
                format_id: "stl",
                export: export_stl_binary,
            },
            #[cfg(feature = "formats-mesh")]
            ExportConnector {
                descriptor: &DESCRIPTOR_STL,
                format_id: "stl-ascii",
                export: export_stl_ascii,
            },
            #[cfg(feature = "formats-mesh")]
            ExportConnector {
                descriptor: &DESCRIPTOR_OBJ,
                format_id: "obj",
                export: export_obj,
            },
            #[cfg(feature = "formats-mesh")]
            ExportConnector {
                descriptor: &DESCRIPTOR_PLY,
                format_id: "ply",
                export: export_ply,
            },
            #[cfg(feature = "formats-mesh")]
            ExportConnector {
                descriptor: &DESCRIPTOR_OFF,
                format_id: "off",
                export: export_off,
            },
            #[cfg(feature = "formats-ifc")]
            ExportConnector {
                descriptor: &DESCRIPTOR_IFC,
                format_id: "ifc",
                export: export_ifc,
            },
            #[cfg(feature = "formats-skp")]
            ExportConnector {
                descriptor: &DESCRIPTOR_SKP,
                format_id: "skp",
                export: export_skp,
            },
            // glTF: two containers of one connector (`glb` binary, `gltf` JSON
            // with an embedded base64 buffer).
            #[cfg(feature = "formats-gltf")]
            ExportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                format_id: "glb",
                export: export_glb,
            },
            #[cfg(feature = "formats-gltf")]
            ExportConnector {
                descriptor: &DESCRIPTOR_GLTF,
                format_id: "gltf",
                export: export_gltf,
            },
        ];
        // Every pair's descriptor has `export: None` (the tests pin that
        // invariant), so the refusal is the only honest answer. The mesh, IFC
        // and SKP writers are real lanes above; only STEP, DGN and RVT have no
        // writer, and each is pushed only when its reader is compiled in.
        let refusals: Vec<(&'static str, &'static ConnectorDescriptor)> = vec![
            #[cfg(feature = "formats-step")]
            ("step", &DESCRIPTOR_STEP),
            #[cfg(feature = "formats-dgn")]
            ("dgn", &DESCRIPTOR_DGN),
            #[cfg(feature = "formats-rvt")]
            ("rvt", &DESCRIPTOR_RVT),
        ];
        ConnectorRegistry {
            imports,
            exports,
            refusals,
        }
    }

    /// The first lane that claims `bytes`, without importing them.
    ///
    /// This looks only at the bytes: a request is not an argument, so no limit
    /// is applied here and a `Some` is *not* an admission decision. **The caller
    /// must apply [`ImportRequest::limits`]`::max_file_bytes` before calling
    /// this**, because probes are bounded reads rather than header-only sniffs —
    /// the DGN V8 probe reads the OLE compound-file directory. The guard itself
    /// lives only in [`Self::import`], which refuses an over-limit OLE container
    /// before that probe runs.
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
    /// container is therefore refused **before** that probe runs, but only when
    /// an OLE lane is compiled in — a slim build lets the bytes fall through to
    /// the named [`absent_import_reason`] refusal instead. Every other lane keeps
    /// its adapter's own limit check (and its stable limit code), so this guard is
    /// deliberately narrow.
    ///
    /// Bytes no lane claims are answered in two honest ways: a format the shared
    /// detector still identifies (its probe is compiled out) is
    /// `unsupported-runtime: import.<id>`, and truly unidentifiable bytes are
    /// [`unclaimed_input`].
    pub fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        #[cfg(any(feature = "formats-dgn", feature = "formats-rvt"))]
        if is_ole(&request.bytes) && request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }
        let Some(connector) = self
            .imports
            .iter()
            .find(|connector| (connector.probe)(&request.bytes))
        else {
            // A compiled-out probe cannot claim bytes, so the shared detector is
            // the only honest source for "which format did this look like". It
            // never parses, and every arm it returns is a real format. The one
            // ambiguity it cannot resolve is OLE: splitting DGN V8 from RVT reads
            // the compound-file directory, which is the adapter's work, so those
            // bytes are named `import.ole`.
            let compiled: Vec<&str> = self
                .imports
                .iter()
                .map(|connector| connector.descriptor.id)
                .collect();
            if let Some(reason) = absent_import_reason(detect_format(&request.bytes), &compiled) {
                return Err(CadError::Unsupported(reason));
            }
            return Err(CadError::CorruptData(unclaimed_input().into()));
        };
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
    /// 3. a known key with no compiled writer fails with
    ///    `unsupported-runtime: export.<id>`;
    /// 4. anything else is `export.unknown_format`.
    ///
    /// A key is registered **iff** its descriptor declares
    /// [`ExportCapability`](cad_import_core::ExportCapability) and the call
    /// produces bytes; no key silently writes nothing and no descriptor
    /// advertises a writer this build does not have. Step 3 is what keeps a slim
    /// build from reporting STEP/DGN/RVT as if the product had never heard of
    /// them.
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
        let compiled: Vec<&str> = self
            .exports
            .iter()
            .map(|connector| connector.format_id)
            .collect();
        if let Some(reason) = absent_export_reason(format_id, &compiled) {
            return Err(CadError::Unsupported(reason));
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

    /// Project one import report's connector id onto the `openedSource` block
    /// (ADR-0041 D5).
    ///
    /// `None` for **no** connector (blank/New, a native `.ymodel` open, an
    /// importer called directly) and for every [`SpecVisibility::Public`]
    /// format: the wire object exists only for a reverse-engineered source, so
    /// a public connector must never produce it. An unknown id is likewise
    /// `None` — this maps only what the registry actually declares, never a
    /// guess from the file name.
    pub fn opened_source(&self, connector: Option<&str>) -> Option<OpenedSource> {
        let descriptor = self.descriptor(connector?)?;
        if descriptor.spec != SpecVisibility::ReverseEngineered {
            return None;
        }
        Some(OpenedSource {
            connector: descriptor.id,
            spec: descriptor.spec,
            can_write: descriptor.export.is_some(),
            experimental: descriptor
                .export
                .map(|export| export.experimental)
                .unwrap_or(false),
        })
    }
}

/// The `openedSource` projection for a document's import report.
///
/// Resolves against the one production registry, so a host never builds a
/// second table just to describe its source. Building the registry is cheap
/// (small vectors of `&'static` entries), and this runs once per publish/read.
pub fn opened_source(connector: Option<&str>) -> Option<OpenedSource> {
    ConnectorRegistry::builtin().opened_source(connector)
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

// Probes. Each one is a bounded header read through the shared detector, so the
// registry and `detect_format` cannot drift into two different answers.

#[cfg(feature = "formats-skp")]
fn is_skp(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Skp)
}

#[cfg(feature = "formats-dgn")]
fn is_dgn(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dgn)
}

/// The OLE container probe: the RVT lane's claim, the DGN V8 discriminator and
/// the size guard all share it.
#[cfg(any(feature = "formats-dgn", feature = "formats-rvt"))]
fn is_ole(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ole)
}

/// The DGN V8 lane: an OLE container whose compound-file directory carries the
/// `Dgn~H` root storage. It must run before the RVT lane.
#[cfg(feature = "formats-dgn")]
fn is_dgn_v8(bytes: &[u8]) -> bool {
    is_ole(bytes) && DgnImporter::sniff(bytes)
}

fn is_dwg(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dwg)
}

fn is_dxf(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Dxf)
}

#[cfg(feature = "formats-ifc")]
fn is_ifc(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ifc)
}

#[cfg(feature = "formats-step")]
fn is_step(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Step)
}

/// The glTF lane asks the adapter's own sniffer: it already owns the exact GLB
/// header and JSON-object rules, so the registry cannot drift from the reader.
#[cfg(feature = "formats-gltf")]
fn is_gltf(bytes: &[u8]) -> bool {
    cad_import_gltf::sniff(bytes)
}

#[cfg(feature = "formats-mesh")]
fn is_stl(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Stl)
}

#[cfg(feature = "formats-mesh")]
fn is_ply(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Ply)
}

#[cfg(feature = "formats-mesh")]
fn is_off(bytes: &[u8]) -> bool {
    matches!(detect_format(bytes), DrawingFormat::Off)
}

#[cfg(feature = "formats-mesh")]
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
#[cfg(feature = "formats-mesh")]
fn export_mesh(
    format: MeshFormat,
    descriptor: &'static ConnectorDescriptor,
    format_id: &'static str,
    job: &ExportJob<'_>,
) -> CadResult<ExportOutput> {
    MeshExporter::new(format, descriptor).export(format_id, job)
}

/// Binary STL (`stl`).
#[cfg(feature = "formats-mesh")]
fn export_stl_binary(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::StlBinary, &DESCRIPTOR_STL, format_id, job)
}

/// ASCII STL (`stl-ascii`, a second key of the STL descriptor).
#[cfg(feature = "formats-mesh")]
fn export_stl_ascii(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::StlAscii, &DESCRIPTOR_STL, format_id, job)
}

/// Wavefront OBJ.
#[cfg(feature = "formats-mesh")]
fn export_obj(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Obj, &DESCRIPTOR_OBJ, format_id, job)
}

/// Binary little-endian PLY.
#[cfg(feature = "formats-mesh")]
fn export_ply(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Ply, &DESCRIPTOR_PLY, format_id, job)
}

/// ASCII OFF/COFF.
#[cfg(feature = "formats-mesh")]
fn export_off(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_mesh(MeshFormat::Off, &DESCRIPTOR_OFF, format_id, job)
}

/// IFC4 (`IfcTriangulatedFaceSet`) through the shared contract.
#[cfg(feature = "formats-ifc")]
fn export_ifc(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    IfcExporter::new(&DESCRIPTOR_IFC).export(format_id, job)
}

/// SketchUp 2026/2027 container through the shared contract.
#[cfg(feature = "formats-skp")]
fn export_skp(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    SkpExporter::new(&DESCRIPTOR_SKP).export(format_id, job)
}

/// Binary glTF (`glb`).
#[cfg(feature = "formats-gltf")]
fn export_glb(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_gltf_container(GltfContainer::Glb, format_id, job)
}

/// JSON glTF (`gltf`, buffer embedded as a base64 data URI).
#[cfg(feature = "formats-gltf")]
fn export_gltf(
    format_id: &'static str,
    job: &ExportJob<'_>,
    _settings: &ExportSettings,
) -> CadResult<ExportOutput> {
    export_gltf_container(GltfContainer::Gltf, format_id, job)
}

/// One glTF container lane. Built per call like the mesh writers: the exporter
/// holds only the container and the descriptor reference.
#[cfg(feature = "formats-gltf")]
fn export_gltf_container(
    container: GltfContainer,
    format_id: &'static str,
    job: &ExportJob<'_>,
) -> CadResult<ExportOutput> {
    GltfExporter::new(container, &DESCRIPTOR_GLTF).export(format_id, job)
}

#[cfg(test)]
mod slim_tests;
#[cfg(test)]
mod tests;
