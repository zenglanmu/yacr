//! ISO 10303-21 (STEP) import adapter for AP203/AP214/AP242.
//!
//! STEP carries an exact B-rep, not a mesh. This adapter uses
//! [`cadmpeg_codec_step`] to decode the Part 21 file into `cadmpeg-ir`'s neutral
//! topology and [`cadmpeg_tessellate`] to turn each face into triangles. The
//! neutral meshes are written through the shared
//! [`cad_import_core::MeshImport`] scaffold, so STEP publishes exactly the same
//! capability rows as OBJ/STL: draw/pick verified, measure unsupported (a mesh
//! is not a MEASURE target, and the exact B-rep is not exposed to the domain).
//!
//! Losses are never silent:
//! * Every per-face tessellation refusal becomes one diagnostic with
//!   [`DIAG_FACE_REFUSED`] and makes the import `Partial`. Refusals are real:
//!   on the reference samples they are a minority of faces (for example 155 of
//!   156 on NIST `nist-ctc05`, 22 of 25 on an ABC AP214 file), never a
//!   guarantee.
//! * Every `DecodeReport::losses` entry is propagated under
//!   `import.step.loss.<codec code>` with its severity in the message.
//!   Warning/Error/Blocking losses make the import `Partial`.
//! * A body whose faces all tessellate to nothing, unattributed faces, dropped
//!   normals and uninstantiated bodies are each reported by their own code.
//! * A file whose every face is refused is a [`CadError::Unsupported`], never an
//!   empty success.
//!
//! Two facts come from the verified `cadmpeg-ir` 0.6.11 source, not from
//! guesses:
//!
//! * **Units.** The IR stores every length in **millimetres** and its canonical
//!   unit wire has exactly one variant (`Millimeter`); it does not expose the
//!   declared STEP unit. The codec rescales each source record to millimetres,
//!   so this adapter reports millimetres and records
//!   [`DIAG_UNITS_CANONICALISED`]. If the decoder's own
//!   `geometry.document-length-unit-unresolved` loss is present the coordinates
//!   are unscaled, so the adapter falls back to
//!   [`UnitContext::drawing_units`] and records [`DIAG_UNITS_UNKNOWN`] instead
//!   of assuming millimetres.
//! * **Placement.** Per-face meshes are in local body coordinates. Assembly
//!   placement comes from `Model::occurrences` (`Occurrence::transform` composed
//!   through `OccurrenceParent`) and `ProductDefinition::bodies`; see
//!   [`placement`]. If any placement is unresolved/ambiguous in the decode
//!   report, a multi-body file is refused with
//!   [`ASSEMBLY_PLACEMENT_UNSUPPORTED`] rather than stacking every body at the
//!   origin.
//!
//! IFC files also start with `ISO-10303-21;`. `detect_format` separates them
//! into `DrawingFormat::Ifc`, and the dispatcher routes them to
//! `cad-import-ifc`. This adapter repeats the `FILE_SCHEMA` check before
//! invoking cadmpeg and refuses with [`IFC_SCHEMA_REFUSED`], so a direct caller
//! cannot decode IFC as AP203/AP214/AP242 either.

mod placement;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, Mesh, Point3, Unit, UnitContext};
use cad_import_core::{
    compute_identity, ifc_schema, ImportLimits, ImportPhase, ImportProgress, ImportProgressSink,
    ImportReport, ImportRequest, ImportedDrawing, Importer, MeshImport, NoopProgress,
    IFC_SCHEMA_REFUSED,
};
use cadmpeg_codec_step::StepCodec;
use cadmpeg_ir::document::CadIr;
use cadmpeg_ir::ids::FaceId;
use cadmpeg_ir::report::decode::DecodeReport;
use cadmpeg_ir::report::Severity;
use cadmpeg_ir::topology::{Body, Region, Shell};
use cadmpeg_ir::{Codec, DecodeOptions};
use cadmpeg_tessellate::{Options as TessellationOptions, Tessellator};

use placement::{placements_of, resolve_body_placements, Affine, PlacementPlan};

/// Stable diagnostic code: the tessellator refused one face.
pub const DIAG_FACE_REFUSED: &str = "import.step.face_refused";
/// Stable diagnostic code: a face tessellated to no triangle.
pub const DIAG_FACE_EMPTY: &str = "import.step.face_empty";
/// Stable diagnostic code: a repeated-index triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.step.degenerate_triangle";
/// Stable diagnostic code: a body's normals were dropped as unusable.
pub const DIAG_NORMALS_DROPPED: &str = "import.step.normals_dropped";
/// Stable diagnostic code: a body contributed no tessellated triangle.
pub const DIAG_EMPTY_BODY: &str = "import.step.empty_body";
/// Stable diagnostic code: a face was not reachable from any body.
pub const DIAG_FACE_UNATTRIBUTED: &str = "import.step.face_unattributed";
/// Stable diagnostic code: a body's product definition has no occurrence.
pub const DIAG_BODY_UNINSTANTIATED: &str = "import.step.body_uninstantiated";
/// Stable diagnostic code: the declared source length unit is not exposed.
pub const DIAG_UNITS_CANONICALISED: &str = "import.step.units_canonicalised";
/// Stable diagnostic code: the decode could not resolve the length unit.
pub const DIAG_UNITS_UNKNOWN: &str = "import.step.units_unknown";
/// Stable refusal code: a body placement could not be resolved honestly.
pub const ASSEMBLY_PLACEMENT_UNSUPPORTED: &str = "import.step.assembly_placement_unsupported";
/// Stable refusal code: the file holds no B-rep face to tessellate.
pub const NO_BREP_FACES: &str = "import.step.no_brep_faces";
/// Stable refusal code: every B-rep face was refused by the tessellator.
pub const NO_TESSELLATED_FACES: &str = "import.step.no_tessellated_faces";

/// The STEP entity `type_key` reported in capabilities and stored rows.
pub const STEP_MESH_TYPE_KEY: &str = "StepTessellatedMesh";

/// Decoder loss local codes that make occurrence placement untrustworthy.
const PLACEMENT_LOSS_CODES: &[&str] = &[
    "product.nauo-placement-unresolved",
    "product.nauo-placement-ambiguous",
    "product.body-conflicting-mapped-placements",
    "occurrence.root-placement-not-representable",
    "occurrence.unresolved-parent",
    "occurrence.no-local-product",
    "assembly.graph-invalid",
];

/// The decoder loss local code for an unresolved document length unit.
const LENGTH_UNIT_LOSS: &str = "geometry.document-length-unit-unresolved";

/// The ISO 10303-21 import adapter.
#[derive(Default)]
pub struct StepImporter;

impl StepImporter {
    pub fn new() -> Self {
        Self
    }
}

/// One body's faces and resolved placements.
struct BodyGroup {
    label: String,
    color: EntityColor,
    faces: Vec<FaceId>,
    placements: Vec<Affine>,
    /// Faces reachable from no body at all. They are imported as one entity,
    /// but the lost attribution is reported.
    unattributed: bool,
}

/// Counters shared by the per-body tessellation passes.
#[derive(Default)]
struct TessellationStats {
    refused: usize,
    empty: usize,
    first_refusal: Option<String>,
}

impl TessellationStats {
    fn note_refusal(&mut self, face: &FaceId, reason: &str) {
        self.refused += 1;
        if self.first_refusal.is_none() {
            self.first_refusal = Some(format!("{}: {reason}", face.as_str()));
        }
    }
}

/// One decoded loss, ready to become a diagnostic.
struct LossView {
    code: String,
    local: String,
    severity: Severity,
    message: String,
}

impl Importer for StepImporter {
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
        // IFC shares the ISO 10303-21 container. Detection routes it to
        // `DrawingFormat::Ifc`, but a direct caller can still hand this adapter
        // IFC bytes; refuse by name before cadmpeg is invoked at all.
        if let Some(schema) = ifc_schema(&request.bytes) {
            return Err(CadError::Unsupported(format!(
                "{IFC_SCHEMA_REFUSED}: FILE_SCHEMA names the IFC schema {schema}; IFC import is \
                 a separate lane and the STEP adapter does not decode it"
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

        let decoded = StepCodec::default()
            .decode(
                &mut Cursor::new(request.bytes.as_ref()),
                &DecodeOptions::default(),
            )
            .map_err(|error| CadError::CorruptData(format!("STEP decode failed: {error}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        let ir = decoded.ir();
        let report = decoded.report();
        if report.container_only() {
            return Err(CadError::CorruptData(
                "STEP decode stopped at the container layer and transferred no geometry".into(),
            ));
        }

        let losses = decode_losses(report);
        let length_unit_unresolved = losses.iter().any(|loss| loss.local == LENGTH_UNIT_LOSS);
        let units = if length_unit_unresolved {
            UnitContext::drawing_units()
        } else {
            millimetre_units()
        };
        let placed_loss = losses
            .iter()
            .any(|loss| PLACEMENT_LOSS_CODES.iter().any(|code| loss.local == *code));

        if ir.model.faces.is_empty() {
            return Err(CadError::Unsupported(format!(
                "{NO_BREP_FACES}: the file holds {} body/bodies but no B-rep face; AP242 \
                 tessellated-only geometry is not imported by the B-rep tessellation path",
                ir.model.bodies.len()
            )));
        }

        let plan = resolve_body_placements(ir).map_err(|reason| {
            CadError::Unsupported(format!(
                "{ASSEMBLY_PLACEMENT_UNSUPPORTED}: {reason}; the file is not imported rather \
                 than placing bodies at a guessed location"
            ))
        })?;
        let groups = body_groups(ir, &plan);
        let body_count = ir.model.bodies.len();
        if body_count > 1 {
            if placed_loss {
                return Err(CadError::Unsupported(format!(
                    "{ASSEMBLY_PLACEMENT_UNSUPPORTED}: the decode report records an unresolved \
                     or ambiguous occurrence placement while the file has {body_count} bodies; \
                     importing them would stack bodies at the origin"
                )));
            }
            if !plan.uninstantiated.is_empty() {
                return Err(CadError::Unsupported(format!(
                    "{ASSEMBLY_PLACEMENT_UNSUPPORTED}: {} body/bodies are owned by a product \
                     definition that no occurrence instantiates ({}); their placement relative \
                     to the other {} bodies is unknown",
                    plan.uninstantiated.len(),
                    plan.uninstantiated.join(", "),
                    body_count
                )));
            }
        }

        let face_total = ir.model.faces.len();
        progress.report(ImportProgress::at(
            ImportPhase::Parsing,
            0,
            Some(face_total),
        ));

        let mut import = MeshImport::new(request, STEP_MESH_TYPE_KEY)?;
        if length_unit_unresolved {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UNITS_UNKNOWN.into(),
                    message: "the STEP document length unit did not resolve; cadmpeg left the \
                              coordinates unscaled and this import cannot know their unit, so \
                              they are reported as drawing units"
                        .into(),
                },
                Some(
                    "the STEP length unit did not resolve; coordinates are drawing units, not \
                     millimetres"
                        .to_string(),
                ),
            );
        } else {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UNITS_CANONICALISED.into(),
                    message: "cadmpeg-ir stores every STEP length in canonical millimetres and \
                              does not expose the declared source unit; the decoder already \
                              rescaled the coordinates, so they are reported as millimetres"
                        .into(),
                },
                None,
            );
        }
        for loss in &losses {
            let message = format!("[{}] {}", loss.severity, loss.message);
            let partial = (loss.severity >= Severity::Warning).then(|| message.clone());
            import.record(
                Diagnostic {
                    object: None,
                    code: loss.code.clone(),
                    message,
                },
                partial,
            );
        }
        if let Some(group) = groups.iter().find(|group| group.unattributed) {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_FACE_UNATTRIBUTED.into(),
                    message: format!(
                        "{} face(s) are not reachable from any body through region/shell \
                         ownership and were imported as one entity in model space",
                        group.faces.len()
                    ),
                },
                Some("face-to-body attribution was unavailable for at least one face".to_string()),
            );
        }
        for body in &plan.uninstantiated {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BODY_UNINSTANTIATED.into(),
                    message: format!(
                        "body {body} is owned by a product definition that no occurrence \
                         instantiates; it was imported in its own product frame"
                    ),
                },
                Some(format!(
                    "body {body} has no occurrence placement; its product frame was used"
                )),
            );
        }

        let mut tessellator = Tessellator::new(ir, TessellationOptions::default());
        let mut stats = TessellationStats::default();
        let entity_total: usize = groups.iter().map(|group| group.placements.len()).sum();

        for group in &groups {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let Some(mesh) = tessellate_body(
                &mut tessellator,
                group,
                &request.limits,
                &mut import,
                cancelled,
                &mut stats,
            )?
            else {
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_EMPTY_BODY.into(),
                        message: format!(
                            "STEP body '{}' contributed no tessellated triangle from its {} \
                             face(s); wire/vertex-only topology is not imported",
                            group.label,
                            group.faces.len()
                        ),
                    },
                    Some(format!(
                        "STEP body '{}' contributed no tessellated triangle",
                        group.label
                    )),
                );
                continue;
            };
            for matrix in &group.placements {
                if cancelled() {
                    return Err(CadError::Cancelled);
                }
                if import.entity_count() >= request.limits.max_entities {
                    return Err(CadError::InvalidInput(format!(
                        "STEP would insert more than {} mesh entities, over the {} entity import \
                         limit",
                        entity_total, request.limits.max_entities
                    )));
                }
                let placed = place_mesh(&mesh, matrix, &group.label)?;
                if placed.normals_dropped {
                    import.record(
                        Diagnostic {
                            object: None,
                            code: DIAG_NORMALS_DROPPED.into(),
                            message: format!(
                                "body '{}': transformed normals were non-finite or the placement \
                                 is singular, so the mesh was stored without normals",
                                group.label
                            ),
                        },
                        None,
                    );
                }
                let object = import.reserve_object()?;
                import.push_mesh(object, placed.mesh, group.color)?;
                progress.report(ImportProgress::at(
                    ImportPhase::Entities,
                    import.entity_count(),
                    Some(entity_total),
                ));
            }
        }

        if import.entity_count() == 0 {
            let refusal = stats
                .first_refusal
                .as_deref()
                .unwrap_or("no face produced a triangle");
            return Err(CadError::Unsupported(format!(
                "{NO_TESSELLATED_FACES}: none of the {face_total} B-rep face(s) produced a \
                 drawable triangle ({} refused, {} empty; first: {refusal})",
                stats.refused, stats.empty
            )));
        }
        if stats.refused > 0 || stats.empty > 0 {
            import.note_partial(format!(
                "{} of {face_total} STEP face(s) were not tessellated ({} refused, {} \
                 empty)",
                stats.refused + stats.empty,
                stats.refused,
                stats.empty
            ));
        }

        progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            import.entity_count(),
            Some(entity_total),
        ));
        let outcome = import.finish()?;
        let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = ImportReport {
            identity,
            capabilities: outcome.capabilities,
            completeness: outcome.completeness,
            diagnostics: outcome.diagnostics,
            parse_ms: Some(parse_ms),
            object_xdata: Vec::new(),
        };
        Ok(ImportedDrawing {
            database: outcome.database,
            units,
            report,
            // STEP has no xrefs and no vendor (Tianzheng) records.
            vendor_records: Vec::new(),
        })
    }
}

/// A mesh plus whether normals had to be dropped while placing it.
struct PlacedMesh {
    mesh: Mesh,
    normals_dropped: bool,
}

/// Canonical millimetres: `cadmpeg-ir`'s documented length unit.
fn millimetre_units() -> UnitContext {
    UnitContext {
        source: Unit::Millimeter,
        display: Unit::Millimeter,
        display_per_source: Some(1.0),
        decimal_places: 3,
    }
}

/// Extract the decode losses as owned, diagnostic-ready views.
fn decode_losses(report: &DecodeReport) -> Vec<LossView> {
    report
        .losses
        .iter()
        .map(|note| {
            let local = note.code.local_code().to_string();
            LossView {
                code: format!("import.step.loss.{local}"),
                local,
                severity: note.severity,
                message: note.message.clone(),
            }
        })
        .collect()
}

/// Attribute every face to its body and resolve that body's placements.
///
/// Faces reachable from no body are kept together in one entity instead of
/// being dropped; that is reported as [`DIAG_FACE_UNATTRIBUTED`] by the caller
/// through the group label.
fn body_groups(ir: &CadIr, plan: &PlacementPlan) -> Vec<BodyGroup> {
    let mut regions = BTreeMap::<&str, &Region>::new();
    for region in &ir.model.regions {
        regions.insert(region.id.as_str(), region);
    }
    let mut shells = BTreeMap::<&str, &Shell>::new();
    for shell in &ir.model.shells {
        shells.insert(shell.id.as_str(), shell);
    }

    let mut attributed = BTreeSet::<String>::new();
    let mut groups = Vec::new();
    for (index, body) in ir.model.bodies.iter().enumerate() {
        let faces = body_faces(body, &regions, &shells);
        for face in &faces {
            attributed.insert(face.as_str().to_string());
        }
        if faces.is_empty() {
            continue;
        }
        groups.push(BodyGroup {
            label: body
                .name
                .clone()
                .unwrap_or_else(|| format!("body {}", index + 1)),
            color: body_color(body),
            faces,
            placements: placements_of(plan, body.id.as_str()),
            unattributed: false,
        });
    }

    let unattributed: Vec<FaceId> = ir
        .model
        .faces
        .iter()
        .filter(|face| !attributed.contains(face.id.as_str()))
        .map(|face| face.id.clone())
        .collect();
    if !unattributed.is_empty() {
        groups.push(BodyGroup {
            label: "unattributed faces".into(),
            color: EntityColor::ByLayer,
            faces: unattributed,
            placements: vec![placement::identity()],
            unattributed: true,
        });
    }
    groups
}

fn body_faces(
    body: &Body,
    regions: &BTreeMap<&str, &Region>,
    shells: &BTreeMap<&str, &Shell>,
) -> Vec<FaceId> {
    let mut seen = BTreeSet::<&str>::new();
    let mut faces = Vec::new();
    for region_id in &body.regions {
        let Some(region) = regions.get(region_id.as_str()) else {
            continue;
        };
        for shell_id in &region.shells {
            let Some(shell) = shells.get(shell_id.as_str()) else {
                continue;
            };
            for face in shell.faces() {
                if seen.insert(face.as_str()) {
                    faces.push(face.clone());
                }
            }
        }
    }
    faces
}

/// Map a body's display colour when it carries one.
fn body_color(body: &Body) -> EntityColor {
    let Some(color) = body.color else {
        return EntityColor::ByLayer;
    };
    let channel = |value: f32| {
        if value.is_finite() {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            0
        }
    };
    EntityColor::Explicit([channel(color.r()), channel(color.g()), channel(color.b())])
}

/// Tessellate every face of one body into its local-coordinate mesh.
///
/// Returns `None` when no triangle survived (the caller reports the body).
fn tessellate_body(
    tessellator: &mut Tessellator<'_>,
    group: &BodyGroup,
    limits: &ImportLimits,
    import: &mut MeshImport,
    cancelled: &dyn Fn() -> bool,
    stats: &mut TessellationStats,
) -> CadResult<Option<Mesh>> {
    let mut vertices = Vec::<Point3>::new();
    let mut normals = Vec::<Point3>::new();
    let mut normals_usable = true;
    let mut triangles = Vec::<[u32; 3]>::new();
    let mut degenerate = 0usize;

    for face in &group.faces {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        let face_mesh = match tessellator.face(face) {
            Ok(mesh) => mesh,
            Err(refusal) => {
                stats.note_refusal(face, refusal.reason());
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_FACE_REFUSED.into(),
                        message: format!(
                            "face {} was refused by the STEP tessellator: {}",
                            face.as_str(),
                            refusal.reason()
                        ),
                    },
                    None,
                );
                continue;
            }
        };
        if face_mesh.triangles.is_empty() || face_mesh.positions.is_empty() {
            stats.empty += 1;
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_FACE_EMPTY.into(),
                    message: format!(
                        "face {} tessellated to no triangle and contributed no geometry",
                        face.as_str()
                    ),
                },
                None,
            );
            continue;
        }

        let base = vertices.len();
        let total_vertices = base
            .checked_add(face_mesh.positions.len())
            .ok_or_else(|| CadError::Invariant("STEP vertex count overflow".into()))?;
        if total_vertices > u32::MAX as usize {
            return Err(CadError::InvalidInput(format!(
                "STEP body '{}' needs {total_vertices} vertices, over the u32 mesh index range",
                group.label
            )));
        }
        for position in &face_mesh.positions {
            if !position.is_finite() {
                return Err(CadError::CorruptData(format!(
                    "body '{}': the STEP tessellator produced a non-finite vertex",
                    group.label
                )));
            }
            vertices.push(Point3 {
                x: position.x,
                y: position.y,
                z: position.z,
            });
        }
        if normals_usable {
            if face_mesh.normals.len() == face_mesh.positions.len() {
                for normal in &face_mesh.normals {
                    if !normal.is_finite() {
                        normals_usable = false;
                        break;
                    }
                    normals.push(Point3 {
                        x: normal.x,
                        y: normal.y,
                        z: normal.z,
                    });
                }
            } else {
                normals_usable = false;
            }
        }
        if !normals_usable {
            normals.clear();
        }

        for triangle in &face_mesh.triangles {
            if triangle
                .iter()
                .any(|index| *index >= face_mesh.positions.len())
            {
                return Err(CadError::CorruptData(format!(
                    "body '{}': a STEP face triangle references a vertex outside the face mesh",
                    group.label
                )));
            }
            let placed = [
                index_to_u32(base, triangle[0], &group.label)?,
                index_to_u32(base, triangle[1], &group.label)?,
                index_to_u32(base, triangle[2], &group.label)?,
            ];
            if placed[0] == placed[1] || placed[1] == placed[2] || placed[0] == placed[2] {
                degenerate += 1;
                continue;
            }
            triangles.push(placed);
        }

        if let Some(max) = limits.max_mesh_faces {
            if triangles.len() > max {
                return Err(CadError::InvalidInput(format!(
                    "STEP body '{}' has more than {max} triangle(s) from {} face(s), over the \
                     face import limit",
                    group.label,
                    group.faces.len()
                )));
            }
        }
    }

    if degenerate > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: format!(
                    "body '{}': {degenerate} triangle(s) with repeated vertex indices were \
                     dropped",
                    group.label
                ),
            },
            None,
        );
    }
    if !normals_usable && !vertices.is_empty() {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_NORMALS_DROPPED.into(),
                message: format!(
                    "body '{}': at least one face returned a missing, short or non-finite normal \
                     list; the body was stored without normals",
                    group.label
                ),
            },
            None,
        );
    }
    if triangles.is_empty() {
        return Ok(None);
    }
    if let Some(max) = limits.max_mesh_face_indices {
        let indices = triangles.len().saturating_mul(3);
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "STEP body '{}' has {indices} face corner indices, over the {max} face-index \
                 import limit",
                group.label
            )));
        }
    }
    if degenerate > 0 {
        import.note_partial(format!(
            "body '{}': {degenerate} degenerate triangle(s) were dropped",
            group.label
        ));
    }
    Ok(Some(Mesh {
        vertices,
        triangles,
        normals,
        face_sources: Vec::new(),
        colors: Vec::new(),
    }))
}

fn index_to_u32(base: usize, index: usize, label: &str) -> CadResult<u32> {
    u32::try_from(base + index).map_err(|_| {
        CadError::InvalidInput(format!(
            "STEP body '{label}' exceeds the u32 mesh index range"
        ))
    })
}

/// Place one local body mesh in world coordinates.
fn place_mesh(mesh: &Mesh, matrix: &Affine, label: &str) -> CadResult<PlacedMesh> {
    let mut vertices = Vec::with_capacity(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let point = placement::transform_point(matrix, [vertex.x, vertex.y, vertex.z]).ok_or_else(
            || {
                CadError::CorruptData(format!(
                    "body '{label}': the occurrence placement produced a non-finite vertex"
                ))
            },
        )?;
        vertices.push(Point3 {
            x: point[0],
            y: point[1],
            z: point[2],
        });
    }

    let mut normals_dropped = false;
    let normals = if mesh.normals.is_empty() {
        Vec::new()
    } else {
        match placement::normal_matrix(matrix) {
            Some(normal_matrix) => {
                let mut normals = Vec::with_capacity(mesh.normals.len());
                for normal in &mesh.normals {
                    let x = normal_matrix[0][0] * normal.x
                        + normal_matrix[0][1] * normal.y
                        + normal_matrix[0][2] * normal.z;
                    let y = normal_matrix[1][0] * normal.x
                        + normal_matrix[1][1] * normal.y
                        + normal_matrix[1][2] * normal.z;
                    let z = normal_matrix[2][0] * normal.x
                        + normal_matrix[2][1] * normal.y
                        + normal_matrix[2][2] * normal.z;
                    let length = (x * x + y * y + z * z).sqrt();
                    if !length.is_finite() || length == 0.0 {
                        normals_dropped = true;
                        break;
                    }
                    normals.push(Point3 {
                        x: x / length,
                        y: y / length,
                        z: z / length,
                    });
                }
                if normals_dropped {
                    Vec::new()
                } else {
                    normals
                }
            }
            None => {
                normals_dropped = true;
                Vec::new()
            }
        }
    };

    Ok(PlacedMesh {
        mesh: Mesh {
            vertices,
            triangles: mesh.triangles.clone(),
            normals,
            face_sources: Vec::new(),
            colors: Vec::new(),
        },
        normals_dropped,
    })
}

#[cfg(test)]
mod tests;
