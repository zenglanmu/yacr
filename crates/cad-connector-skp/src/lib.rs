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
//!   colour, opacity, and front-face base-colour textures (ADR-0038): an
//!   embedded image is registered as a document asset and the face group carries
//!   §4v UV0. SketchUp's own line/curve geometry is carried as real edges
//!   (ADR-0049): a material group whose visible face rings are all backed by
//!   `MeshEdge` records becomes a display body with those boundary edges, and
//!   the remaining visible edges become standalone wire bodies.
//! * **Not imported:** back-face textures, images that only exist as external
//!   paths, scenes/pages, section planes, annotations, dynamic-component
//!   attributes, and any CAD semantics. Edge soft/smooth flags, per-edge
//!   materials on face boundaries, and curve geometry (flattened to its chord)
//!   are reported rather than modelled. A file with neither triangles nor edges
//!   is an **explicit refusal** ([`NO_GEOMETRY`]); an edge-only file imports as
//!   `Partial` wire bodies ([`DIAG_EDGES_ONLY`]), never as an empty success.
//! * Tags are flattened onto layer `0` (the shared mesh-import contract inserts
//!   a single layer) but **visibility is honoured**: hidden instances, hidden
//!   faces, hidden/soft edges and invisible tags are pruned, and the counts are
//!   reported.
//! * Every successful import is `Completeness::Partial`. `measure` is
//!   `Unsupported` for triangle-only carriers and `Partial` for a carrier that
//!   stored real visible display edges (endpoint/midpoint snap only, no
//!   area/volume).
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
// Real `MeshEdge` records: visibility, canonical endpoints and the
// face-boundary / standalone-wire split (ADR-0049). Split out for the code-size
// gate.
mod edges;
// The textured-material bridge (asset registration + §4v UV planning), split
// out for the code-size gate.
mod texture;

/// The writer half of this connector (the 2026/2027 container).
pub mod export;

#[cfg(test)]
mod tests;

pub use convert::{
    determinant3, material_style, read_scene, style_of, FaceSkip, PlacedEdges, PlacedMesh,
    SkpScene, SkpStats, DEFAULT_MAX_MESH_EDGES, MAX_DEPTH,
};
pub use edges::EdgeStats;
pub use export::SkpExporter;

use cad_connector_core::{
    compute_identity, ConnectorDescriptor, ImportCapability, ImportPhase, ImportProgress,
    ImportProgressSink, ImportReport, ImportRequest, ImportedDrawing, Importer, MeshImport,
    NoopProgress,
};
use cad_db::{EntityColor, EntityRenderAttributes, EntityTransparency};
use cad_domain::{
    Body3d, BodyDisplay, BodyExact, BodyKind, CadError, CadResult, Completeness, Diagnostic,
    EntityCapability, Mesh, Point3, Precision, SupportStatus, UnitContext,
};

/// The `BodyDisplay` completeness reason every preview body carries.
const PREVIEW_BODY_REASON: &str =
    "import.skp.preview_geometry: SketchUp supplies display geometry only, no exact B-rep";
/// The `BodyExact::Unavailable` reason of a preview body that carries a mesh.
const PREVIEW_EXACT_REASON: &str = "the SketchUp source supplied display geometry only";
/// The `BodyExact::Unavailable` reason of a wire-only preview body.
const PREVIEW_WIRE_EXACT_REASON: &str = "the SketchUp source supplied edges only";

/// Stable refusal code: the fixed header is missing or truncated, so the file
/// never was a SketchUp model.
pub const NOT_A_SKETCHUP_FILE: &str = "import.skp.not_a_sketchup_file";
/// Stable refusal code: `skpwright` could not read a file whose header matched.
pub const PARSE_FAILED: &str = "import.skp.parse_failed";
/// Stable refusal code: the file reads but its release is outside every
/// container the reader models.
pub const VERSION_UNSUPPORTED: &str = "import.skp.version_unsupported";
/// Stable refusal code: nothing visible has face geometry or edges.
pub const NO_GEOMETRY: &str = "import.skp.no_geometry";
/// Stable refusal code: a caller limit (`max_file_bytes`, `max_entities`,
/// `max_mesh_faces`, `max_mesh_face_indices`, `max_mesh_edges`) was exceeded.
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
/// Partial code: a textured material group still falls back to its average colour
/// (no usable image, so no texture could be attached).
pub const DIAG_TEXTURES_NOT_SAMPLED: &str = "import.skp.textures_not_sampled";
/// Partial code: a textured material references an image file outside the model,
/// whose bytes are not in the `.skp`.
pub const DIAG_TEXTURE_UNAVAILABLE: &str = "import.skp.texture_unavailable";
/// Partial code: a textured face has no §4v UV transform (or no applied size), so
/// its group stays untextured.
pub const DIAG_UV_UNAVAILABLE: &str = "import.skp.uv_unavailable";
/// Informational code: back-face textures are not imported (the front side is).
pub const DIAG_BACK_TEXTURE_NOT_IMPORTED: &str = "import.skp.back_texture_not_imported";
/// Partial code: faces whose rings could not be triangulated were dropped.
pub const DIAG_FACES_DROPPED: &str = "import.skp.faces_dropped";
/// Partial code: the release is read by container equivalence only.
pub const DIAG_VERSION_UNVERIFIED: &str = "import.skp.version_unverified";
/// Always-partial code: this is preview triangle/edge display geometry, not CAD
/// semantics (no exact B-rep, no measurement source).
pub const DIAG_PREVIEW_GEOMETRY: &str = "import.skp.preview_geometry";
/// Informational code: the file's visible geometry is edges only, imported as
/// `Partial` wire bodies (ADR-0049).
pub const DIAG_EDGES_ONLY: &str = "import.skp.edges_only";
/// Informational code: which edge record flags/semantics are not modelled
/// (soft/hidden pruning, smooth smoothing, per-edge material on face boundaries,
/// layer colour). Emitted only when at least one record actually carried such a
/// flag, not merely because records exist.
pub const DIAG_EDGE_FLAGS_NOT_MODELED: &str = "import.skp.edge_flags_not_modeled";
/// Informational code: curve edges are stored as their chord.
pub const DIAG_EDGE_CURVES_FLATTENED: &str = "import.skp.edge_curves_flattened";
/// Informational code: material groups that kept the mesh carrier because a
/// face-ring segment had no `MeshEdge` record.
pub const DIAG_GROUPS_WITHOUT_EDGE_RECORDS: &str = "import.skp.groups_without_edge_records";
/// Partial code: a face group had a complete `MeshEdge` record set but every
/// boundary record was pruned (hidden/soft/invisible tag), so the group fell
/// back to the plain mesh carrier instead of an edge-less body (ADR-0049 D3).
pub const DIAG_EDGES_HIDDEN_FALLBACK: &str = "import.skp.edges_hidden_fallback";
/// Informational code: stored visible display edges are a narrow MEASURE source.
///
/// Endpoint and midpoint snap only, on the stored edges; no face, triangle,
/// center, quadrant or intersection snap and no area/volume.
pub const DIAG_MEASURE_EDGES_PARTIAL: &str = "import.skp.measure_edges_partial";

/// Entity type key every inserted face/material group carries (mesh or body).
pub const SKP_MESH_TYPE_KEY: &str = "SkpPreviewMesh";
/// Entity type key every standalone wire body carries.
pub const SKP_EDGE_TYPE_KEY: &str = "SkpPreviewEdges";

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

        let mut import = MeshImport::new(request, SKP_MESH_TYPE_KEY)?;
        import.declare_edge_snap_measure(true);
        let scene = read_scene(&model, &request.limits, cancelled, &mut import)?;
        let stats = &scene.stats;
        if scene.placed.is_empty() && scene.edges.is_empty() {
            return Err(CadError::Unsupported(format!(
                "{NO_GEOMETRY}: release {}, {} placed run(s), {} face(s) emitted, {} edge \
                 record(s) with {} stored segment(s), {} face(s) dropped as untriangulable",
                model.version,
                stats.placed_runs,
                stats.faces_emitted,
                stats.edges.records,
                stats.edges.emitted,
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
        record_version(&mut import, &model);
        record_document(&mut import, stats);
        record_edges(&mut import, stats);
        record_losses(&mut import, stats);
        if stats.edges.emitted > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_MEASURE_EDGES_PARTIAL.into(),
                    message: format!(
                        "{} stored visible display edge segment(s) are a MEASURE source for \
                         endpoint and midpoint snap only; there is no face, triangle, center, \
                         quadrant or intersection snap and no area/volume.",
                        stats.edges.emitted
                    ),
                },
                None,
            );
        }
        let edge_only = scene.placed.is_empty();
        let wire_groups = scene.edges.len();

        for placed in scene.placed {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let object = import.reserve_object()?;
            let attributes = EntityRenderAttributes {
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
            };
            if placed.edges.is_empty() {
                import.push_mesh_with_attributes(object, placed.mesh, attributes)?;
            } else {
                import.push_body_with_attributes(
                    object,
                    preview_body(placed.mesh, placed.edges, BodyKind::Unknown),
                    SKP_MESH_TYPE_KEY,
                    attributes,
                )?;
            }
            progress.report(ImportProgress::at(
                ImportPhase::Entities,
                import.entity_count(),
                Some(entity_total),
            ));
        }

        for wires in scene.edges {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let object = import.reserve_object()?;
            import.push_body_with_attributes(
                object,
                preview_body(Mesh::default(), wires.edges, BodyKind::Wire),
                SKP_EDGE_TYPE_KEY,
                EntityRenderAttributes {
                    clip: None,
                    color: wires
                        .color
                        .map_or(EntityColor::ByLayer, EntityColor::Explicit),
                    transparency: wires
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
                message: "SketchUp preview: triangles, real boundary/standalone edges, instance \
                          transforms, tags, materials and front-face base-colour textures are \
                          read as display geometry; back-face textures, scenes, sections, \
                          annotations and any exact B-rep are not. Triangles are not a \
                          measurement source; stored visible edges allow endpoint/midpoint \
                          snap only."
                    .into(),
            },
            Some(
                "SketchUp preview geometry: display meshes and edges only, no exact B-rep, scenes \
                 or CAD measurement semantics beyond endpoint/midpoint snap on stored edges"
                    .into(),
            ),
        );
        if edge_only {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_EDGES_ONLY.into(),
                    message: format!(
                        "the visible geometry is edges only ({} stored segment(s) in {} wire \
                         group(s)); it is imported as Partial wire bodies, not as an empty \
                         document.",
                        stats.edges.emitted, wire_groups
                    ),
                },
                None,
            );
        }
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
            connector: None,
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

/// The display-only body ADR-0049 inserts for a group with real boundary edges
/// (`BodyKind::Unknown`) or for standalone wire edges (`BodyKind::Wire`).
fn preview_body(mesh: Mesh, edges: Vec<Vec<Point3>>, kind: BodyKind) -> Body3d {
    let reason = if kind == BodyKind::Wire {
        PREVIEW_WIRE_EXACT_REASON
    } else {
        PREVIEW_EXACT_REASON
    };
    Body3d {
        kind,
        exact: BodyExact::Unavailable {
            reason: reason.into(),
        },
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Partial(vec![PREVIEW_BODY_REASON.into()]),
        },
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
        // Never force the MEASURE claim to a fixed status: the import has
        // already decided it (Unsupported for triangle-only carriers, Partial
        // for stored display edges), and the release ceiling can only narrow
        // it (Partial + Unverified → Unverified). Unsupported stays Unsupported.
        capability.measure = weaker(capability.measure, ceiling);
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
                "{} run(s), {} placed, {} scene node(s), {} face(s) emitted ({} textured), {} \
                 edge record(s) with {} stored segment(s) ({} face boundary, {} standalone wire, \
                 {} skipped, {} stored on a preview body, {} drawn by a mesh's derived boundary), \
                 {} material(s), {} tag(s) ({} textured), {} textured group(s)",
                stats.runs,
                stats.placed_runs,
                stats.nodes,
                stats.faces_emitted,
                stats.textured_faces,
                stats.edges.records,
                stats.edges.emitted,
                stats.edges.boundary,
                stats.edges.wire,
                stats.edges.skipped(),
                stats.edges.covered_by_body,
                stats.edges.covered_by_mesh,
                stats.materials,
                stats.layers,
                stats.textured_materials,
                stats.textured_groups
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
        + stats.edges.hidden_skipped
        + stats.edges.soft_skipped
        + stats.edges.invisible_layer_skipped
        > 0
    {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_HIDDEN_PRUNED.into(),
                message: format!(
                    "WYSIWYG pruning: {} hidden instance(s), {} instance(s) on invisible tags, \
                     {} hidden face(s), {} face(s) on invisible tags, {} hidden edge(s), {} soft \
                     edge(s), {} edge(s) on invisible tags",
                    stats.hidden_instances_pruned,
                    stats.layer_instances_pruned,
                    stats.hidden_faces_skipped,
                    stats.invisible_layer_faces_skipped,
                    stats.edges.hidden_skipped,
                    stats.edges.soft_skipped,
                    stats.edges.invisible_layer_skipped
                ),
            },
            None,
        );
    }
}

/// Edge records: what was stored, what the flags mean, and what is not modelled.
fn record_edges(import: &mut MeshImport, stats: &SkpStats) {
    let edges = &stats.edges;
    if edges.records == 0 {
        return;
    }
    // A plain visible record has no story to tell: the note is emitted only when
    // a record really carried a flag this preview cannot model, or when malformed
    // records were dropped (zero-length/collapsed and unresolved-index). The
    // document totals stay in `import.skp.document`; this note explains them.
    if edges.flag_records > 0 || edges.zero_length_skipped > 0 || edges.index_skipped > 0 {
        let mut message = if edges.flag_records > 0 {
            format!(
                "{} of {} edge record(s) carry a flag or semantic this preview does not \
                 model: soft/hidden/invisible-tag edges are pruned ({} pruned here); `smooth` \
                 edges keep no smoothed normals; a per-edge material selects the wire group \
                 colour only (a face-boundary edge keeps its group's colour); edge layer \
                 colour and cast/receive-shadow flags are not carried.",
                edges.flag_records,
                edges.records,
                edges.hidden_skipped + edges.soft_skipped + edges.invisible_layer_skipped,
            )
        } else {
            "Malformed edge records are dropped, not approximated.".to_string()
        };
        message.push_str(&format!(
            " {} zero-length or collapsed and {} unresolved-index record(s) are dropped.",
            edges.zero_length_skipped, edges.index_skipped
        ));
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EDGE_FLAGS_NOT_MODELED.into(),
                message,
            },
            None,
        );
    }
    if edges.curves_flattened > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EDGE_CURVES_FLATTENED.into(),
                message: format!(
                    "{} stored edge segment(s) belong to a SketchUp curve; each is stored as the \
                     chord between its two endpoints, not as an arc.",
                    edges.curves_flattened
                ),
            },
            None,
        );
    }
    if edges.groups_without_records > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_GROUPS_WITHOUT_EDGE_RECORDS.into(),
                message: format!(
                    "{} material group(s) kept the plain mesh carrier because {} visible face-ring \
                     segment(s) have no MeshEdge record; their wireframe is the triangulated \
                     surface boundary (ADR-0045), not the source's own edges.",
                    edges.groups_without_records, edges.unrecorded_boundary_segments
                ),
            },
            None,
        );
    }
    if edges.groups_empty_boundary > 0 {
        // Complete records, empty visible set: the group fell back to the mesh
        // carrier. That is a real loss of the source's edges, not a silent
        // degradation, so it is named and marks the import Partial.
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_EDGES_HIDDEN_FALLBACK.into(),
                message: format!(
                    "{} material group(s) had a complete MeshEdge record set but every boundary \
                     record was hidden, soft or on an invisible tag, so they kept the plain mesh \
                     carrier; their wireframe is the triangulated surface boundary (ADR-0045), \
                     not the source's own hidden edges.",
                    edges.groups_empty_boundary
                ),
            },
            Some(
                "some SketchUp groups fell back to the mesh carrier because every boundary \
                 edge record was hidden, soft or on an invisible tag"
                    .into(),
            ),
        );
    }
}

fn record_losses(import: &mut MeshImport, stats: &SkpStats) {
    if stats.external_textures > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_TEXTURE_UNAVAILABLE.into(),
                message: format!(
                    "{} textured material(s) reference an image file outside the model; those \
                     images are not in the .skp, so the affected faces keep the material's \
                     average colour.",
                    stats.external_textures
                ),
            },
            Some("a SketchUp texture references an external image file".into()),
        );
    }
    if let Some(refusal) = &stats.texture_refusal {
        // The marker is the code: `import.texture.too_large`,
        // `import.texture.budget_exceeded`, `import.texture.too_many` or
        // `import.texture.unsupported_format`.
        let code = refusal
            .split(':')
            .next()
            .map(str::trim)
            .filter(|marker| marker.starts_with("import."))
            .unwrap_or("import.texture.refused")
            .to_string();
        import.record(
            Diagnostic {
                object: None,
                code,
                message: format!(
                    "a SketchUp texture could not be stored as a document asset ({refusal}); the \
                     affected faces keep the material's average colour."
                ),
            },
            Some("a SketchUp texture was refused and its faces keep the average colour".into()),
        );
    }
    if stats.uv_unavailable > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_UV_UNAVAILABLE.into(),
                message: format!(
                    "{} face(s) paint a textured material but have no §4v texture placement (or \
                     their material declares no applied size), so their group stays untextured \
                     instead of guessing a mapping.",
                    stats.uv_unavailable
                ),
            },
            Some("some textured SketchUp faces have no texture placement".into()),
        );
    }
    if stats.back_textured_faces > 0 {
        // A note, not a loss of the front-side texture that was imported.
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_BACK_TEXTURE_NOT_IMPORTED.into(),
                message: format!(
                    "{} face(s) carry a textured back-face material; back faces are not \
                     imported, only the front side is.",
                    stats.back_textured_faces
                ),
            },
            None,
        );
    }
    if stats.untextured_textured_groups > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_TEXTURES_NOT_SAMPLED.into(),
                message: format!(
                    "{} textured material group(s) could not be given their image, so they keep \
                     the material's average colour as the face colour.",
                    stats.untextured_textured_groups
                ),
            },
            Some("some SketchUp textured material groups fall back to their average colour".into()),
        );
    }
}

/// The SketchUp connector descriptor.
///
/// Owned by this crate (one declaration per format): the reader is this preview
/// importer and the writer is the [`export`] module, which authors the
/// 2026/2027 container at L1 (lossy: triangles, tags/materials flattened).
///
/// `spec` is [`cad_connector_core::SpecVisibility::ReverseEngineered`]:
/// `skpwright` is a clean-room reader with no Trimble SDK or SDK-derived
/// knowledge (this module's header, lines 3-6, and
/// `THIRD_PARTY_NOTICES.md`:87), and the 2026/2027 writer is NOT VERIFIED
/// against SketchUp itself, so its export is `experimental`.
pub const DESCRIPTOR_SKP: ConnectorDescriptor = ConnectorDescriptor {
    id: "skp",
    name_key: "connector.skp.name",
    extensions: &["skp"],
    spec: cad_connector_core::SpecVisibility::ReverseEngineered,
    import: Some(ImportCapability {
        versions: "3-8, 2013-2020, 2026, 2027 (2021-2025 read as container-equivalent, unverified)",
        container: "skp",
    }),
    export: Some(cad_connector_core::ExportCapability {
        versions: "SketchUp 2026/2027 (written container)",
        level: cad_connector_core::FidelityLevel::L1,
        lossy: true,
        experimental: true,
    }),
    declared_classes: &[],
};
