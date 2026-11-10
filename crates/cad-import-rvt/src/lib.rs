//! Autodesk Revit (`.rvt` / `.rte`) **preview** import adapter.
//!
//! This is a preview path, not a measurement path. [`rvt`] 0.17.0 recovers
//! Revit element records and exports them as glTF; on Revit **2008–2023** an
//! element's body is its **bounding box** (cut at readable joins), 2024/2025
//! are `partial`, 2026/2027 are `experimental`, and `.rfa`/`.rft` family files
//! carry **no family geometry at all**. Its own README says not to rely on the
//! output for measurement or coordination, so:
//!
//! * the GLB is an **implementation detail** (not a user-visible format and
//!   never in detection or pickers);
//! * `rvt`'s per-release status is this import's **status ceiling** — `partial`
//!   becomes [`SupportStatus::Partial`], `experimental`/`unknown` become
//!   [`SupportStatus::Unverified`], never a flattened "supported";
//! * every successful import is `Completeness::Partial`, measure is always
//!   [`SupportStatus::Unsupported`], and a decode with no geometry is an
//!   explicit refusal, never an empty success.
//!
//! The lane is split into two independently testable halves:
//!
//! 1. [`decode`] — Revit bytes -> GLB bytes + coverage facts (tested with the
//!    upstream `rvt` scaffold fixture, which honestly yields **zero** geometry);
//! 2. [`glb`] — GLB bytes -> world-space [`Mesh`]es (tested with a synthetic
//!    two-node GLB authored in this repository, which proves node world
//!    transforms are applied and the nodes do not stack at the origin).
//!
//! Units are metres: glTF 2.0 is metres/+Y up and `rvt`'s root matrix applies
//! its `0.3048` feet-to-metre scale. See [`DIAG_UNITS_METRES`].

mod decode;
mod glb;

#[cfg(test)]
mod tests;

pub use decode::{
    classify_document, decode_to_glb, part_atom_declares_family_table, DocumentKind, RvtDecode,
};
pub use glb::{read_glb, GlbScene, PlacedMesh};

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, EntityCapability, SupportStatus, UnitContext};
use cad_import_core::{
    compute_identity, ImportPhase, ImportProgress, ImportProgressSink, ImportReport, ImportRequest,
    ImportedDrawing, Importer, MeshImport, NoopProgress,
};

/// Stable refusal code: the file is a Revit family/family template, which has
/// metadata, previews and types but no geometry.
pub const FAMILY_NO_GEOMETRY: &str = "import.rvt.family_no_geometry";
/// Stable refusal code: a compound file without the invariant Revit streams.
pub const NOT_A_REVIT_FILE: &str = "import.rvt.not_a_revit_file";
/// Stable refusal code: `rvt` 0.17.0 could not open or export the file.
pub const DECODE_FAILED: &str = "import.rvt.decode_failed";
/// Stable refusal code: a decodable project produced no drawable geometry.
pub const NO_GEOMETRY: &str = "import.rvt.no_geometry";
/// Stable refusal code: the file's Revit release has no element-record reader.
pub const RELEASE_UNSUPPORTED: &str = "import.rvt.release_unsupported";
/// Stable refusal code: `rvt`'s own resource limits were exceeded.
pub const RVT_OPEN_LIMITS: &str = "import.rvt.limits_exceeded";

/// Informational code: stored coordinates are metres (glTF is metres).
pub const DIAG_UNITS_METRES: &str = "import.rvt.units_metres";
/// Always-partial reason: Revit bodies are preview geometry.
pub const DIAG_PREVIEW_GEOMETRY: &str = "import.rvt.preview_geometry";
/// Informational code: the release's status in `rvt`'s support matrix.
pub const DIAG_RELEASE_STATUS: &str = "import.rvt.release_status";
/// Informational code: `rvt`'s export confidence level/score/geometry count.
pub const DIAG_CONFIDENCE: &str = "import.rvt.confidence";
/// Informational code: `rvt`'s source-coverage status/fractions.
pub const DIAG_COVERAGE: &str = "import.rvt.coverage";
/// Diagnostic code: `rvt` left element records out.
pub const DIAG_SKIPPED_ELEMENTS: &str = "import.rvt.skipped_elements";
/// Partial reason: a resource limit left source streams unread.
pub const DIAG_UNREAD_STREAMS: &str = "import.rvt.unread_streams";
/// Diagnostic code: `rvt` listed an unsupported feature.
pub const DIAG_UNSUPPORTED_FEATURES: &str = "import.rvt.unsupported_features";
/// Diagnostic code: `rvt` emitted export warnings.
pub const DIAG_EXPORT_WARNINGS: &str = "import.rvt.export_warnings";
/// Informational code: `rvt`'s per-step export notes.
pub const DIAG_EXPORT_NOTES: &str = "import.rvt.export_notes";
/// Diagnostic code: degenerate triangles were dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.rvt.degenerate_triangle";
/// Diagnostic code: an unusable normal list was dropped.
pub const DIAG_NORMALS_DROPPED: &str = "import.rvt.normals_dropped";
/// Diagnostic code: a non-triangle primitive was skipped.
pub const DIAG_PRIMITIVE_MODE_SKIPPED: &str = "import.rvt.primitive_mode_skipped";
/// Diagnostic code: a node's mesh produced no triangle.
pub const DIAG_EMPTY_NODE: &str = "import.rvt.empty_node";

/// Stable error code: the intermediate GLB is missing the `glTF` magic.
pub const EMPTY_GLB: &str = "import.rvt.invalid_glb";
/// Stable error code: the intermediate GLB failed validation.
pub const GLB_INVALID: &str = "import.rvt.invalid_glb";
/// Stable error code: a GLB accessor carries no buffer view.
pub const MESH_WITHOUT_POSITIONS: &str = "import.rvt.mesh_without_positions";
/// Stable error code: a triangle indexes outside its primitive.
pub const INDEX_OUT_OF_RANGE: &str = "import.rvt.index_out_of_range";
/// Stable error code: a node transform is not finite.
pub const INVALID_TRANSFORM: &str = "import.rvt.invalid_transform";
/// Stable error code: the node hierarchy closes a cycle.
pub const NODE_CYCLE: &str = "import.rvt.node_cycle";
/// Stable error code: the GLB declares no scene.
pub const NO_SCENE: &str = "import.rvt.no_scene";
/// Stable refusal code: a non-finite vertex after the node transform.
pub const NON_FINITE_VERTEX: &str = "import.rvt.non_finite_vertex";
/// Stable refusal code: a sparse accessor is not supported.
pub const SPARSE_ACCESSOR: &str = "import.rvt.sparse_accessor";
/// Stable refusal code: an external (non-BIN) buffer is not fetched.
pub const EXTERNAL_BUFFER: &str = "import.rvt.external_buffer";

/// The entity `type_key` reported in capabilities and stored rows.
pub const RVT_MESH_TYPE_KEY: &str = "RvtPreviewMesh";

/// The Autodesk Revit preview importer.
#[derive(Default)]
pub struct RvtImporter;

impl RvtImporter {
    pub fn new() -> Self {
        Self
    }
}

impl Importer for RvtImporter {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing> {
        self.import_with_progress(request, cancelled, &NoopProgress)
    }

    fn import_with_progress(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, None));
        // `rvt::RevitFile::open_bytes` consumes an owned buffer; the import
        // contract hands out an `Arc<[u8]>`, so this is the one deliberate
        // copy. The file-size limit above bounds it.
        let decoded = decode_to_glb(request.bytes.to_vec(), &request.limits, cancelled)?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if !decoded.has_geometry {
            return Err(CadError::Unsupported(format!(
                "{NO_GEOMETRY}: rvt 0.17.0 decoded Revit {} with confidence level {:?} \
                 ({} building element(s), {} with geometry) but produced no drawable body; \
                 this is a metadata/scaffold result, not an empty success",
                decoded
                    .revit_version
                    .map(|version| version.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                decoded.confidence_level,
                decoded.building_elements,
                decoded.geometry_elements
            )));
        }

        let scene = read_glb(&decoded.glb, &request.limits, cancelled)?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if scene.meshes.is_empty() {
            return Err(CadError::Unsupported(format!(
                "{NO_GEOMETRY}: rvt exported Revit {} as geometry ({} element(s) with a body), \
                 but the intermediate GLB has no drawable node/mesh instance",
                decoded
                    .revit_version
                    .map(|version| version.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                decoded.geometry_elements
            )));
        }
        let entity_total = scene.meshes.len();
        if entity_total > request.limits.max_entities {
            return Err(CadError::InvalidInput(format!(
                "the Revit file has {entity_total} drawable node/mesh instance(s), over the {} \
                 entity import limit",
                request.limits.max_entities
            )));
        }

        let ceiling = ceiling_for(&decoded.release_status);
        if ceiling == SupportStatus::Unsupported {
            // `decode_to_glb` refuses the known `unsupported` value; this also
            // covers a status word 0.17.0 did not document.
            return Err(CadError::Unsupported(format!(
                "{RELEASE_UNSUPPORTED}: rvt 0.17.0 reports release status {:?}, which this \
                 adapter cannot rate above Unsupported",
                decoded.release_status
            )));
        }
        let mut import = MeshImport::new(request, RVT_MESH_TYPE_KEY)?;
        record_decode(&mut import, &decoded);
        record_scene(&mut import, &scene);
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_UNITS_METRES.into(),
                message: "rvt 0.17.0 writes glTF with one root matrix of 0.3048 that scales its \
                          feet-based body coordinates to metres and swaps to +Y up; this adapter \
                          applies every node's world transform and maps the glTF +Y-up frame to \
                          CAD +Z up, so stored coordinates are metres"
                    .into(),
            },
            None,
        );

        progress.report(ImportProgress::at(
            ImportPhase::Entities,
            0,
            Some(entity_total),
        ));
        for placed in scene.meshes {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let object = import.reserve_object()?;
            import.push_mesh(object, placed.mesh, EntityColor::ByLayer)?;
            progress.report(ImportProgress::at(
                ImportPhase::Entities,
                import.entity_count(),
                Some(entity_total),
            ));
        }

        // A bounding-box preview can never be a lossless Revit import: the
        // release status caps the capability rows, and the completeness is
        // always Partial for the reason below.
        let preview_reason = format!(
            "Revit bodies are preview geometry (bounding boxes on Revit 2008-2023, partial on \
             2024/2025, experimental on 2026/2027); rvt 0.17.0's release status is {:?} and its \
             documentation says not to rely on the output for measurement or coordination",
            decoded.release_status
        );
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_PREVIEW_GEOMETRY.into(),
                message: preview_reason.clone(),
            },
            Some(preview_reason),
        );

        progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            import.entity_count(),
            Some(entity_total),
        ));
        let outcome = import.finish()?;
        let mut capabilities = outcome.capabilities;
        apply_ceiling(&mut capabilities, ceiling);
        let report = ImportReport {
            identity,
            capabilities,
            completeness: outcome.completeness,
            diagnostics: outcome.diagnostics,
            parse_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            object_xdata: Vec::new(),
        };

        Ok(ImportedDrawing {
            database: outcome.database,
            // Stored coordinates are metres (see DIAG_UNITS_METRES above).
            // Never assume the Revit display unit or millimetres.
            units: UnitContext::from_meter(),
            report,
            // Revit projects carry no Tianzheng records and the import contract
            // has no xref source; `ImportRequest::xref` is ignored.
            vendor_records: Vec::new(),
        })
    }
}

/// Map `rvt`'s release status to the strongest status this import may claim.
///
/// `verified` is the only value that maps to [`SupportStatus::Verified`];
/// `partial` maps to `Partial`, `experimental`/`unknown` to `Unverified`, and
/// anything else (including `unsupported`) to `Unsupported`.
pub fn ceiling_for(status: &str) -> SupportStatus {
    match status {
        "verified" => SupportStatus::Verified,
        "partial" => SupportStatus::Partial,
        "experimental" | "unknown" => SupportStatus::Unverified,
        _ => SupportStatus::Unsupported,
    }
}

fn apply_ceiling(capabilities: &mut [EntityCapability], ceiling: SupportStatus) {
    for capability in capabilities {
        capability.read = weaker(capability.read, ceiling);
        capability.semantic = weaker(capability.semantic, ceiling);
        capability.render = weaker(capability.render, ceiling);
        capability.pick = weaker(capability.pick, ceiling);
        // A mesh is never a MEASURE target, whatever the release status says.
        capability.measure = SupportStatus::Unsupported;
    }
}

fn weaker(left: SupportStatus, right: SupportStatus) -> SupportStatus {
    let rank = |status: &SupportStatus| match status {
        SupportStatus::Verified => 4,
        SupportStatus::Partial => 3,
        SupportStatus::Unverified => 2,
        SupportStatus::Unsupported => 1,
        SupportStatus::NotImplemented => 0,
    };
    if rank(&left) <= rank(&right) {
        left
    } else {
        right
    }
}

fn record_decode(import: &mut MeshImport, decoded: &RvtDecode) {
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_RELEASE_STATUS.into(),
            message: format!(
                "Revit release {:?} has rvt 0.17.0 status {:?} (capability {:?}); read, render \
                 and pick are capped at that status, and measure stays Unsupported",
                decoded.revit_version, decoded.release_status, decoded.release_capability
            ),
        },
        None,
    );
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_CONFIDENCE.into(),
            message: format!(
                "rvt export confidence: level {:?}, score {:.2}, {} building element(s), {} with \
                 geometry, {} stream(s) left unread",
                decoded.confidence_level,
                decoded.confidence_score,
                decoded.building_elements,
                decoded.geometry_elements,
                decoded.unread_streams
            ),
        },
        None,
    );

    import.record(
        Diagnostic {
            object: None,
            code: DIAG_COVERAGE.into(),
            message: format!(
                "rvt source coverage is {:?} (decoded {:?}, exported {:?}, with geometry {:?}){}",
                decoded.coverage_status,
                decoded.coverage_decoded,
                decoded.coverage_exported,
                decoded.coverage_geometry,
                decoded
                    .coverage_notes
                    .as_deref()
                    .map(|notes| format!("; {notes}"))
                    .unwrap_or_default()
            ),
        },
        None,
    );

    if decoded.unread_streams > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_UNREAD_STREAMS.into(),
                message: format!(
                    "{} Revit stream(s) were not read whole because a resource limit was hit; the \
                     model is missing what they hold",
                    decoded.unread_streams
                ),
            },
            Some(format!(
                "{} Revit stream(s) were left unread by rvt's resource limits",
                decoded.unread_streams
            )),
        );
    }
    if !decoded.skipped.is_empty() {
        let total: usize = decoded.skipped.iter().map(|(_, count)| count).sum();
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_SKIPPED_ELEMENTS.into(),
                message: format!(
                    "rvt left {total} element record(s) out of the export by reason: {}",
                    summarize(
                        &decoded
                            .skipped
                            .iter()
                            .map(|(reason, count)| format!("{reason}={count}"))
                            .collect::<Vec<_>>()
                    )
                ),
            },
            Some(format!(
                "{total} Revit element record(s) were not exported by rvt"
            )),
        );
    }
    if !decoded.unsupported_features.is_empty() {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_UNSUPPORTED_FEATURES.into(),
                message: format!(
                    "rvt listed {} unsupported feature(s): {}",
                    decoded.unsupported_features.len(),
                    summarize(&decoded.unsupported_features)
                ),
            },
            None,
        );
    }
    if decoded.warning_count > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EXPORT_WARNINGS.into(),
                message: format!(
                    "rvt emitted {} export warning(s); see the export notes for the steps it ran",
                    decoded.warning_count
                ),
            },
            None,
        );
    }
    if !decoded.notes.is_empty() {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EXPORT_NOTES.into(),
                message: format!("rvt export steps: {}", summarize(&decoded.notes)),
            },
            None,
        );
    }
}

fn record_scene(import: &mut MeshImport, scene: &GlbScene) {
    if scene.degenerate_triangles > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: format!(
                    "{} degenerate triangle(s) (repeated index or zero area) were dropped from \
                     the intermediate glTF meshes",
                    scene.degenerate_triangles
                ),
            },
            Some(format!(
                "{} degenerate Revit preview triangle(s) were dropped",
                scene.degenerate_triangles
            )),
        );
    }
    if scene.normals_dropped > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_NORMALS_DROPPED.into(),
                message: format!(
                    "{} node mesh(es) carried a missing, short or non-finite normal list; they \
                     were stored without normals",
                    scene.normals_dropped
                ),
            },
            None,
        );
    }
    if scene.primitives_skipped > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_PRIMITIVE_MODE_SKIPPED.into(),
                message: format!(
                    "{} glTF primitive(s) were not TRIANGLES and were skipped; rvt 0.17.0 only \
                     emits triangle lists, so this is unexpected input",
                    scene.primitives_skipped
                ),
            },
            Some(format!(
                "{} non-triangle glTF primitive(s) were skipped",
                scene.primitives_skipped
            )),
        );
    }
    if scene.empty_nodes > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EMPTY_NODE.into(),
                message: format!(
                    "{} glTF node(s) had a mesh but no triangle; they were not inserted",
                    scene.empty_nodes
                ),
            },
            None,
        );
    }
}

/// Join a list for a diagnostic message, naming the elided tail by count.
fn summarize(items: &[String]) -> String {
    const LIMIT: usize = 6;
    if items.len() <= LIMIT {
        return items.join(", ");
    }
    format!(
        "{} and {} more",
        items[..LIMIT].join(", "),
        items.len() - LIMIT
    )
}
