//! SketchUp (`.skp`) **preview** import adapter.
//!
//! This is a preview path, not a measurement path. [`skpwright`] 0.6.0 is a
//! clean-room reader (no Trimble SDK, no SDK-derived knowledge) for both
//! containers SketchUp has used: the stream container of releases 3–8 and
//! 2013–2020, and the ZIP/tagged-record container of 2026 and 2027. It returns
//! one materialized mesh per geometry run with **vertices already in metres**,
//! faces as ordered vertex rings with holes, and a composed scene of instances
//! with row-major world transforms.
//!
//! What that buys, and what it does not:
//!
//! * **Read** is real: model geometry runs, instances/components and groups,
//!   per-instance transforms, tags (layers), hidden flags, materials with
//!   colour and opacity.
//! * **Not imported:** edges (SketchUp's line/curve geometry), texture image
//!   sampling, UV placement, scenes/pages, section planes, annotations,
//!   dynamic-component attributes, and any CAD semantics. A file whose only
//!   geometry is edges is an **explicit refusal**
//!   ([`EDGE_ONLY_GEOMETRY`]), never an empty success.
//! * Tags are flattened onto layer `0` (the shared mesh-import contract inserts
//!   a single layer) but **visibility is honoured**: hidden instances, hidden
//!   faces and invisible tags are pruned, and the counts are reported.
//! * Every successful import is `Completeness::Partial` and `measure` is
//!   always [`SupportStatus::Unsupported`].
//!
//! The status ceiling follows the release, because the reader's own support
//! matrix does: releases with observed corpus files (3–8, 2013–2020, 2026,
//! 2027) may claim `Verified`; 2021–2025 are read through the 2026 container by
//! **container equivalence only** (no observed sample), so they are
//! `Unverified`; anything else is refused.
//!
//! Units are metres: `skpwright` converts the stored inches and the composed
//! node translations to metres. See [`DIAG_UNITS_METRES`].

mod convert;

#[cfg(test)]
mod tests;

pub use convert::{
    determinant3, material_style, read_scene, style_of, FaceSkip, PlacedMesh, SkpScene, SkpStats,
    MAX_DEPTH,
};

use cad_db::{EntityColor, EntityRenderAttributes, EntityTransparency};
use cad_domain::{CadError, CadResult, Diagnostic, EntityCapability, SupportStatus, UnitContext};
use cad_import_core::{
    compute_identity, ImportPhase, ImportProgress, ImportProgressSink, ImportReport, ImportRequest,
    ImportedDrawing, Importer, MeshImport, NoopProgress,
};

/// Stable refusal code: the fixed header is missing or truncated, so the file
/// never was a SketchUp model.
pub const NOT_A_SKETCHUP_FILE: &str = "import.skp.not_a_sketchup_file";
/// Stable refusal code: `skpwright` could not read a file whose header matched.
pub const PARSE_FAILED: &str = "import.skp.parse_failed";
/// Stable refusal code: the file reads but its release is outside every
/// container the reader models.
pub const VERSION_UNSUPPORTED: &str = "import.skp.version_unsupported";
/// Stable refusal code: nothing visible has face geometry.
pub const NO_GEOMETRY: &str = "import.skp.no_geometry";
/// Stable refusal code: the visible geometry is **edges only** (SketchUp line
/// and curve work), which this triangle-only adapter cannot draw.
pub const EDGE_ONLY_GEOMETRY: &str = "import.skp.edge_only_geometry";
/// Stable refusal code: a caller limit (`max_file_bytes`, `max_entities`,
/// `max_mesh_faces`, `max_mesh_face_indices`) was exceeded.
pub const LIMITS_EXCEEDED: &str = "import.skp.limits_exceeded";

/// Informational code: coordinates and translations are metres, `+Z` up.
pub const DIAG_UNITS_METRES: &str = "import.skp.units_metres";
/// Informational code: the file's release, container and model identity.
pub const DIAG_VERSION: &str = "import.skp.version";
/// Informational code: what the scene walk saw (runs, nodes, faces, edges).
pub const DIAG_DOCUMENT: &str = "import.skp.document";
/// Informational code: tags are flattened onto layer `0` (visibility honoured).
pub const DIAG_LAYERS_FLATTENED: &str = "import.skp.layers_flattened";
/// Informational code: what WYSIWYG pruning removed.
pub const DIAG_HIDDEN_PRUNED: &str = "import.skp.hidden_pruned";
/// Partial code: material textures are not sampled.
pub const DIAG_TEXTURES_NOT_SAMPLED: &str = "import.skp.textures_not_sampled";
/// Partial code: faces whose rings could not be triangulated were dropped.
pub const DIAG_FACES_DROPPED: &str = "import.skp.faces_dropped";
/// Partial code: the release is read by container equivalence only.
pub const DIAG_VERSION_UNVERIFIED: &str = "import.skp.version_unverified";
/// Always-partial code: this is preview triangle geometry, not CAD semantics.
pub const DIAG_PREVIEW_GEOMETRY: &str = "import.skp.preview_geometry";

/// Entity type key every inserted mesh carries.
pub const SKP_MESH_TYPE_KEY: &str = "SkpPreviewMesh";

/// SketchUp `.skp` preview importer.
#[derive(Debug, Default, Clone, Copy)]
pub struct SkpImporter;

impl SkpImporter {
    pub fn new() -> Self {
        SkpImporter
    }
}

impl Importer for SkpImporter {
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
        let length = request.bytes.len();
        if length > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "{LIMITS_EXCEEDED}: {} bytes over the {} byte budget",
                length, request.limits.max_file_bytes
            )));
        }

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(length as u64),
            message: None,
        });
        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, None));

        let (version, _) = skpwright::header_info(&request.bytes).ok_or_else(|| {
            CadError::CorruptData(format!(
                "{NOT_A_SKETCHUP_FILE}: the fixed header is missing or truncated"
            ))
        })?;
        let model = skpwright::Model::parse(&request.bytes).map_err(|error| {
            CadError::CorruptData(format!("{PARSE_FAILED}: {}: {error}", version))
        })?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }

        let ceiling = ceiling_for(&model.version, model.container);
        if ceiling == SupportStatus::Unsupported {
            return Err(CadError::Unsupported(format!(
                "{VERSION_UNSUPPORTED}: release {} ({:?} container) is outside the containers \
                 skpwright models",
                model.version, model.container
            )));
        }

        let scene = read_scene(&model, &request.limits, cancelled)?;
        let stats = &scene.stats;
        if scene.placed.is_empty() {
            let code = if stats.edges_not_imported > 0 {
                EDGE_ONLY_GEOMETRY
            } else {
                NO_GEOMETRY
            };
            return Err(CadError::Unsupported(format!(
                "{code}: release {}, {} placed run(s), {} face(s) emitted, {} edge(s) not \
                 importable, {} face(s) dropped as untriangulable",
                model.version,
                stats.placed_runs,
                stats.faces_emitted,
                stats.edges_not_imported,
                stats.faces_untriangulable + stats.faces_malformed
            )));
        }
        if scene.entity_count() > request.limits.max_entities {
            return Err(CadError::InvalidInput(format!(
                "{LIMITS_EXCEEDED}: {} entities over the {} entity budget",
                scene.entity_count(),
                request.limits.max_entities
            )));
        }

        let entity_total = scene.entity_count();
        progress.report(ImportProgress::at(
            ImportPhase::Entities,
            0,
            Some(entity_total),
        ));
        let mut import = MeshImport::new(request, SKP_MESH_TYPE_KEY)?;
        record_version(&mut import, &model);
        record_document(&mut import, stats);
        record_losses(&mut import, stats);

        for placed in scene.placed {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let object = import.reserve_object()?;
            import.push_mesh_with_attributes(
                object,
                placed.mesh,
                EntityRenderAttributes {
                    clip: None,
                    color: placed
                        .color
                        .map_or(EntityColor::ByLayer, EntityColor::Explicit),
                    transparency: placed
                        .opacity
                        .map_or(EntityTransparency::ByLayer, |opacity| {
                            EntityTransparency::Explicit(opacity)
                        }),
                    ..Default::default()
                },
            )?;
            progress.report(ImportProgress::at(
                ImportPhase::Entities,
                import.entity_count(),
                Some(entity_total),
            ));
        }

        import.record(
            Diagnostic {
                object: None,
                code: DIAG_PREVIEW_GEOMETRY.into(),
                message: "SketchUp preview: triangles, instance transforms, tags and materials \
                          are read; edges, textures, scenes, sections and annotations are not. \
                          Not a measurement source."
                    .into(),
            },
            Some(
                "SketchUp preview geometry: triangle meshes only, no edges, textures, scenes or \
                 measurement semantics"
                    .into(),
            ),
        );
        if ceiling == SupportStatus::Unverified {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_VERSION_UNVERIFIED.into(),
                    message: format!(
                        "release {} is read through container equivalence only; no sample of \
                         releases 2021-2025 was observed.",
                        model.version
                    ),
                },
                Some("SketchUp 2021-2025 files are read by container equivalence only".into()),
            );
        }
        if stats.faces_untriangulable + stats.faces_malformed > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_FACES_DROPPED.into(),
                    message: format!(
                        "{} face(s) could not be triangulated ({} malformed, {} degenerate); \
                         they are omitted rather than approximated.",
                        stats.faces_untriangulable + stats.faces_malformed,
                        stats.faces_malformed,
                        stats.faces_untriangulable
                    ),
                },
                Some("some SketchUp faces could not be triangulated and are omitted".into()),
            );
        }

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
            units: UnitContext::from_meter(),
            report,
            vendor_records: Vec::new(),
        })
    }
}

/// Strongest status a file of this release and container may claim.
///
/// Releases with observed corpus files: 3–8 and 2013–2020 in the stream
/// container, 2026 and 2027 in the ZIP container. Releases 2021–2025 share the
/// ZIP container but have **no observed sample**, so they are `Unverified`.
/// Anything else (a header this reader cannot place) is `Unsupported`.
pub fn ceiling_for(version: &str, container: skpwright::Container) -> SupportStatus {
    let Some(major) = major_release(version) else {
        return SupportStatus::Unsupported;
    };
    match container {
        skpwright::Container::Stream => match major {
            3..=8 | 13..=20 => SupportStatus::Verified,
            9..=12 | 21..=25 => SupportStatus::Unverified,
            _ => SupportStatus::Unsupported,
        },
        skpwright::Container::Zip => match major {
            26 | 27 => SupportStatus::Verified,
            21..=25 => SupportStatus::Unverified,
            _ => SupportStatus::Unsupported,
        },
        skpwright::Container::Unknown => SupportStatus::Unsupported,
    }
}

/// `"{17.3.116}"` -> `17`.
pub fn major_release(version: &str) -> Option<u32> {
    let inner = version.trim().trim_start_matches('{').trim_end_matches('}');
    inner.split('.').next()?.trim().parse().ok()
}

fn apply_ceiling(capabilities: &mut [EntityCapability], ceiling: SupportStatus) {
    for capability in capabilities {
        capability.read = weaker(capability.read, ceiling);
        capability.semantic = weaker(capability.semantic, ceiling);
        capability.render = weaker(capability.render, ceiling);
        capability.pick = weaker(capability.pick, ceiling);
        // A preview mesh is never a MEASURE target.
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

fn record_version(import: &mut MeshImport, model: &skpwright::Model) {
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_VERSION.into(),
            message: format!(
                "SketchUp {} ({:?} container){}; coordinates in metres, +Z up",
                model.version,
                model.container,
                model
                    .model_guid
                    .as_ref()
                    .map(|guid| format!(", model GUID {guid}"))
                    .unwrap_or_default()
            ),
        },
        None,
    );
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_UNITS_METRES.into(),
            message: "skpwright reports run vertices and node translations in metres; the \
                      drawing unit context is derived from the metre."
                .into(),
        },
        None,
    );
}

fn record_document(import: &mut MeshImport, stats: &SkpStats) {
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_DOCUMENT.into(),
            message: format!(
                "{} run(s), {} placed, {} scene node(s), {} face(s) emitted, {} edge(s) not \
                 importable, {} material(s), {} tag(s) ({} textured)",
                stats.runs,
                stats.placed_runs,
                stats.nodes,
                stats.faces_emitted,
                stats.edges_not_imported,
                stats.materials,
                stats.layers,
                stats.textured_materials
            ),
        },
        None,
    );
    import.record(
        Diagnostic {
            object: None,
            code: DIAG_LAYERS_FLATTENED.into(),
            message: format!(
                "{} SketchUp tag(s) are flattened onto layer 0; tag visibility is honoured, tag \
                 names and colours are not carried.",
                stats.layers
            ),
        },
        None,
    );
    if stats.hidden_instances_pruned
        + stats.layer_instances_pruned
        + stats.hidden_faces_skipped
        + stats.invisible_layer_faces_skipped
        > 0
    {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_HIDDEN_PRUNED.into(),
                message: format!(
                    "WYSIWYG pruning: {} hidden instance(s), {} instance(s) on invisible tags, \
                     {} hidden face(s), {} face(s) on invisible tags",
                    stats.hidden_instances_pruned,
                    stats.layer_instances_pruned,
                    stats.hidden_faces_skipped,
                    stats.invisible_layer_faces_skipped
                ),
            },
            None,
        );
    }
}

fn record_losses(import: &mut MeshImport, stats: &SkpStats) {
    if stats.textured_materials > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_TEXTURES_NOT_SAMPLED.into(),
                message: format!(
                    "{} textured material(s): the average colour is used as the face colour; \
                     texture images and UV placement are not imported.",
                    stats.textured_materials
                ),
            },
            Some("SketchUp textures are reduced to their average colour".into()),
        );
    }
}
