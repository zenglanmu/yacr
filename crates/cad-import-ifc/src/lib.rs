//! IFC (ISO 10303-21, `IFC2X3` / `IFC4` / `IFC4X3`) import adapter.
//!
//! IFC carries a parametric/B-rep building model. This adapter uses
//! [`ifc_lite_core`] to scan and decode the Part 21 records and
//! [`ifc_lite_geometry`]'s [`GeometryRouter`] to mesh every geometry-bearing
//! product. The resulting triangles are written through the shared
//! [`cad_import_core::MeshImport`] scaffold, so IFC publishes the same
//! capability rows as OBJ/STL/STEP: draw/pick `Verified`, measure `Unsupported`
//! (a mesh is not a MEASURE target, and the exact B-rep is discarded).
//!
//! The four facts below were read from the verified `ifc-lite-core` /
//! `ifc-lite-geometry` **28.2.0** source, not assumed:
//!
//! * **API sequence.** `build_entity_index` → `EntityDecoder::with_index` →
//!   `EntityScanner::next_entity` (`src/parser/scanner.rs:116`) filtered by
//!   `is_geometry_job` → `GeometryRouter::with_units` →
//!   `process_element_with_voids_parts` (`src/router/voids/mod.rs:130`).
//!   A returned `Mesh` is `positions: Vec<f32>`, `normals: Vec<f32>` (parallel
//!   to positions, i.e. one normal per vertex), `indices: Vec<u32>`.
//! * **Units.** The router's `unit_scale` is applied by `scale_positions`
//!   (`src/router/processing.rs:55`: "Scale mesh positions from file units to
//!   meters"), and placements are scaled by the same factor
//!   (`src/router/transforms/mod.rs:113`). `with_units`
//!   (`src/router/mod.rs:280`) resolves the project's declared length unit to a
//!   metre scale via `project_unit_scale` (`src/router/unit_admission.rs:75`). Returned positions are therefore **metres**, not
//!   the file's declared unit. An unresolved unit chain leaves the scale at
//!   `1.0`, so the coordinates are raw file units and are reported as
//!   [`UnitContext::drawing_units`] — never silently as millimetres.
//! * **Placement.** `process_element*` walks the element's
//!   `IfcLocalPlacement` chain (`resolve_object_placement`,
//!   `src/router/transforms/mod.rs:105`) and bakes it into the vertices
//!   (`place_mesh` / `place_sub_meshes`, same file). The returned mesh is in
//!   **world** metres; the test `two_walls_keep_distinct_placed_bounds` pins
//!   that two placed walls do not stack at the origin. A mesh may still carry a
//!   non-zero `Mesh::origin` when the router relativised f32 coordinates for
//!   precision; this adapter folds `origin` back into every vertex, so the
//!   database always holds absolute world coordinates.
//! * **Void subtraction.** `process_element_with_voids_parts` subtracts the
//!   host's `IfcRelVoidsElement` openings (`void_index_from_content` +
//!   `propagate_voids_to_parts`); the test
//!   `opening_subtraction_adds_triangles` pins that a wall with an opening
//!   produces more triangles than the same wall without one. CSG is best-effort:
//!   boolean failures are drained from the router and reported as
//!   [`DIAG_CSG_FAILURE`], never presented as an uncut success.
//!
//! Losses are never silent:
//! * An element whose mesh fails to build is reported with
//!   [`DIAG_ELEMENT_FAILED`] and makes the import `Partial`; an element that
//!   meshes to no triangle is [`DIAG_ELEMENT_EMPTY`].
//! * A file whose every geometry-bearing product fails is
//!   [`CadError::Unsupported`] with [`ALL_ELEMENTS_FAILED`], never an empty
//!   success. A file with no geometry-bearing product at all is
//!   [`CadError::CorruptData`] with [`NO_GEOMETRY_PRODUCTS`].
//! * Style/material colour (`IfcStyledItem`) is not imported: entity colour
//!   falls back to `ByLayer`, reported by [`DIAG_STYLE_NOT_IMPORTED`] and
//!   `Partial`.
//! * `vendor_records` is empty and `ImportRequest::xref` is ignored: IFC has
//!   neither Tianzheng records nor CAD external references.
//!
//! There is **no IFC export** and no glTF exposure in this crate.

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, Mesh, Point3, Unit, UnitContext};
use cad_import_core::{
    compute_identity, ifc_schema as ifc_file_schema, ImportLimits, ImportPhase, ImportProgress,
    ImportProgressSink, ImportReport, ImportRequest, ImportedDrawing, Importer, MeshImport,
    NoopProgress,
};
use ifc_lite_core::{
    build_entity_index, is_geometry_job, resolve_length_unit, EntityDecoder, EntityScanner, IfcType,
};
use ifc_lite_geometry::{
    propagate_voids_to_parts, void_index_from_content, GeometryRouter, Mesh as IfcMesh,
};

/// Stable diagnostic code: the declared length unit was canonicalised to metres.
pub const DIAG_UNITS_CANONICALISED: &str = "import.ifc.units_canonicalised";
/// Stable diagnostic code: the declared length unit could not be resolved.
pub const DIAG_UNITS_UNKNOWN: &str = "import.ifc.units_unknown";
/// Stable refusal code: the declared length unit has no usable metre factor.
pub const UNITS_REFUSED: &str = "import.ifc.units_refused";
/// Stable diagnostic code: one product's geometry could not be built.
pub const DIAG_ELEMENT_FAILED: &str = "import.ifc.element_failed";
/// Stable diagnostic code: one product produced no triangle.
pub const DIAG_ELEMENT_EMPTY: &str = "import.ifc.element_empty";
/// Stable diagnostic code: a degenerate triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.ifc.degenerate_triangle";
/// Stable diagnostic code: a mesh was stored without normals.
pub const DIAG_NORMALS_DROPPED: &str = "import.ifc.normals_dropped";
/// Stable diagnostic code: an opening/CSG operation failed.
pub const DIAG_CSG_FAILURE: &str = "import.ifc.csg_failure";
/// Stable diagnostic code: `IfcStyledItem` colours/materials are not imported.
pub const DIAG_STYLE_NOT_IMPORTED: &str = "import.ifc.style_not_imported";
/// Stable diagnostic code: an opening is not referenced by any host.
pub const DIAG_OPENING_WITHOUT_HOST: &str = "import.ifc.opening_without_host";
/// Stable diagnostic code: a geometry record could not be decoded.
pub const DIAG_ENTITY_UNDECODABLE: &str = "import.ifc.entity_undecodable";
/// Stable diagnostic code: a geometry reference above the u32 index range was
/// refused by the router's content dedup.
pub const DIAG_CONTENT_HASH_DROP: &str = "import.ifc.content_hash_drop";
/// Stable diagnostic code: one representation item had no processor or failed.
pub const DIAG_ITEM_UNSUPPORTED: &str = "import.ifc.item_unsupported";
/// Stable refusal code: the file has no geometry-bearing IFC product.
pub const NO_GEOMETRY_PRODUCTS: &str = "import.ifc.no_geometry_products";
/// Stable refusal code: every geometry-bearing product failed to mesh.
pub const ALL_ELEMENTS_FAILED: &str = "import.ifc.all_elements_failed";

/// The IFC entity `type_key` reported in capabilities and stored rows.
pub const IFC_MESH_TYPE_KEY: &str = "IfcTessellatedMesh";

/// The IFC import adapter.
#[derive(Default)]
pub struct IfcImporter;

impl IfcImporter {
    pub fn new() -> Self {
        Self
    }
}

/// One geometry-bearing product scheduled for meshing.
struct Job {
    entity: ifc_lite_core::DecodedEntity,
}

impl Job {
    fn label(&self) -> String {
        format!("{} #{}", self.entity.ifc_type.as_str(), self.entity.id)
    }
}

/// How the project's declared length unit resolved.
#[derive(Clone, Copy)]
enum Units {
    /// Metres per file unit, finite and positive.
    Canonical(f64),
    /// A declared conversion factor is not a usable number.
    Refused,
    /// No project, or the unit chain did not decode.
    Unknown,
}

/// Counters shared by the per-element conversion.
#[derive(Default)]
struct Stats {
    failed: usize,
    empty: usize,
    first_failure: Option<String>,
}

impl Stats {
    fn note_failure(&mut self, label: &str, reason: String) {
        self.failed += 1;
        if self.first_failure.is_none() {
            self.first_failure = Some(format!("{label}: {reason}"));
        }
    }
}

impl Importer for IfcImporter {
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
        // A direct caller can hand this adapter anything. IFC is distinguished
        // from AP203/AP214/AP242 by its FILE_SCHEMA record, so a missing IFC
        // schema is a corrupt/mismatched input, not an empty IFC file.
        let Some(declared_schema) = ifc_file_schema(&request.bytes) else {
            return Err(CadError::CorruptData(format!(
                "{NO_GEOMETRY_PRODUCTS}: the input has no ISO 10303-21 IFC FILE_SCHEMA record; \
                 it is not an IFC file"
            )));
        };

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
        let content: &[u8] = &request.bytes;
        let index = build_entity_index(content);
        let mut decoder = EntityDecoder::with_index(content, index).with_strict_reals();
        let units = resolve_units(&mut decoder);
        let mut void_index = void_index_from_content(content, &mut decoder);
        let _ = propagate_voids_to_parts(&mut void_index, content, &mut decoder);

        let mut jobs = Vec::<Job>::new();
        let mut styled_items = 0usize;
        let mut undecodable = 0usize;
        let mut orphan_openings = 0usize;
        let cutters: std::collections::BTreeSet<u32> =
            void_index.values().flatten().copied().collect();
        let mut scanner = EntityScanner::new(content);
        while let Some((id, type_name, start, end)) = scanner.next_entity() {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            if type_name.eq_ignore_ascii_case("IFCSTYLEDITEM") {
                styled_items += 1;
            }
            let record = content.get(start..end).unwrap_or(&[]);
            if !is_geometry_job(type_name, record) {
                continue;
            }
            match decoder.decode_at_with_id(id, start, end) {
                Ok(entity) => {
                    // Openings are cutters, not visible products: their solid is
                    // subtracted from the host and must not be inserted itself.
                    if entity
                        .ifc_type
                        .is_subtype_of(IfcType::IfcFeatureElementSubtraction)
                    {
                        // An opening that no `IfcRelVoidsElement` references has
                        // no host to cut; it is skipped (not a visible product)
                        // and reported, never drawn as a solid.
                        if !cutters.contains(&entity.id) {
                            orphan_openings += 1;
                        }
                        continue;
                    }
                    jobs.push(Job { entity });
                }
                Err(_) => undecodable += 1,
            }
        }

        if jobs.len() > request.limits.max_entities {
            return Err(CadError::InvalidInput(format!(
                "IFC has {} geometry-bearing product(s), over the {} entity import limit \
                 (declared schema: {declared_schema})",
                jobs.len(),
                request.limits.max_entities
            )));
        }
        if jobs.is_empty() {
            return Err(CadError::CorruptData(format!(
                "{NO_GEOMETRY_PRODUCTS}: the {declared_schema} file holds no drawable, \
                 geometry-bearing IFC product ({undecodable} geometry record(s) did not decode)"
            )));
        }

        let router = GeometryRouter::with_units(content, &mut decoder);
        if matches!(units, Units::Refused) || router.length_unit_refused() {
            return Err(CadError::Unsupported(format!(
                "{UNITS_REFUSED}: the IFC project declares a length-unit conversion factor that \
                 is not a positive finite number, so no metre scale is right for this file; it \
                 is refused rather than meshed at a guessed size"
            )));
        }

        let mut import = MeshImport::new(request, IFC_MESH_TYPE_KEY)?;
        record_units(&mut import, units);
        if styled_items > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_STYLE_NOT_IMPORTED.into(),
                    message: format!(
                        "the IFC file carries {styled_items} IfcStyledItem record(s); surface \
                         colours and materials are not imported, so geometry falls back to ByLayer"
                    ),
                },
                Some(format!(
                    "{styled_items} IFC style record(s) were not imported; colours fall back to \
                     ByLayer"
                )),
            );
        }
        if undecodable > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_ENTITY_UNDECODABLE.into(),
                    message: format!(
                        "{undecodable} geometry-bearing IFC record(s) could not be decoded and \
                         were not imported"
                    ),
                },
                Some(format!(
                    "{undecodable} geometry-bearing IFC record(s) could not be decoded"
                )),
            );
        }
        if orphan_openings > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_OPENING_WITHOUT_HOST.into(),
                    message: format!(
                        "{orphan_openings} IFC opening/voiding element(s) are referenced by no \
                         IfcRelVoidsElement and were not inserted; an opening has no visible \
                         geometry of its own"
                    ),
                },
                Some(format!(
                    "{orphan_openings} IFC opening(s) had no host to cut"
                )),
            );
        }

        let entity_total = jobs.len();
        progress.report(ImportProgress::at(
            ImportPhase::Parsing,
            0,
            Some(entity_total),
        ));

        let mut stats = Stats::default();
        for job in &jobs {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let label = job.label();
            let parts = match router.process_element_with_voids_parts(
                &job.entity,
                &mut decoder,
                &void_index,
            ) {
                Ok(parts) => parts,
                Err(error) => {
                    let reason = error.to_string();
                    stats.note_failure(&label, reason.clone());
                    import.record(
                        Diagnostic {
                            object: None,
                            code: DIAG_ELEMENT_FAILED.into(),
                            message: format!(
                                "IFC product {label} could not be meshed and was not imported: \
                                 {reason}"
                            ),
                        },
                        None,
                    );
                    continue;
                }
            };
            let Some(mesh) = element_mesh(&parts, &label, &request.limits, &mut import)? else {
                stats.empty += 1;
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_ELEMENT_EMPTY.into(),
                        message: format!(
                            "IFC product {label} produced no triangle; a curve/axis-only \
                             representation is not drawable mesh geometry"
                        ),
                    },
                    None,
                );
                continue;
            };
            let object = import.reserve_object()?;
            import.push_mesh(object, mesh, EntityColor::ByLayer)?;
            progress.report(ImportProgress::at(
                ImportPhase::Entities,
                import.entity_count(),
                Some(entity_total),
            ));
        }

        drain_router_diagnostics(&router, &mut import);

        if import.entity_count() == 0 {
            let first = stats
                .first_failure
                .as_deref()
                .unwrap_or("no product produced a triangle");
            return Err(CadError::Unsupported(format!(
                "{ALL_ELEMENTS_FAILED}: none of the {entity_total} geometry-bearing IFC \
                 product(s) produced a drawable triangle ({} failed, {} empty; first: {first})",
                stats.failed, stats.empty
            )));
        }
        if stats.failed > 0 || stats.empty > 0 {
            let reason = format!(
                "{} of {entity_total} IFC product(s) contributed no drawable mesh ({} failed, \
                 {} empty)",
                stats.failed + stats.empty,
                stats.failed,
                stats.empty
            );
            import.note_partial(reason);
        }

        progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            import.entity_count(),
            Some(entity_total),
        ));
        let outcome = import.finish()?;
        let report = ImportReport {
            identity,
            capabilities: outcome.capabilities,
            completeness: outcome.completeness,
            diagnostics: outcome.diagnostics,
            parse_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            object_xdata: Vec::new(),
        };

        Ok(ImportedDrawing {
            database: outcome.database,
            units: unit_context(units),
            report,
            // IFC has no Tianzheng records and no CAD external references.
            vendor_records: Vec::new(),
        })
    }
}

/// Resolve the project's declared length unit without inventing a default.
fn resolve_units(decoder: &mut EntityDecoder<'_>) -> Units {
    let Some(project) = decoder.project_id() else {
        return Units::Unknown;
    };
    let declared = declares_length_unit(decoder, project);
    match resolve_length_unit(decoder, project) {
        Ok(unit) if unit.unusable_factor.is_some() => Units::Refused,
        // `extract_length_unit_scale` answers `1.0` both for a declared metre
        // and for a project with no length unit at all. Only the former is a
        // known unit; an absent assignment stays unknown (raw file numbers).
        Ok(unit) if declared && unit.scale.is_finite() && unit.scale > 0.0 => {
            Units::Canonical(unit.scale)
        }
        _ => Units::Unknown,
    }
}

/// Whether the project's `IfcUnitAssignment` actually lists a length unit.
///
/// `ifc_lite_core::extract_length_unit_scale` defaults to `1.0` when the
/// assignment is absent; a metre default there is not a declaration, and this
/// adapter must not report it as one.
fn declares_length_unit(decoder: &mut EntityDecoder<'_>, project: u32) -> bool {
    use ifc_lite_core::slot;
    let Ok(entity) = decoder.decode_by_id(project) else {
        return false;
    };
    let Some(assignment_ref) = entity.ref_at(slot::IfcProject::UNITS_IN_CONTEXT) else {
        return false;
    };
    let Ok(assignment) = decoder.decode_by_id(assignment_ref) else {
        return false;
    };
    let Some(units) = assignment
        .attr(slot::IfcUnitAssignment::UNITS)
        .and_then(|value| value.as_list())
    else {
        return false;
    };
    let references: Vec<u32> = units
        .iter()
        .filter_map(|value| value.as_entity_ref())
        .collect();
    for unit_ref in references {
        let Ok(unit) = decoder.decode_by_id(unit_ref) else {
            continue;
        };
        if matches!(
            unit.ifc_type.as_str(),
            "IFCSIUNIT" | "IFCCONVERSIONBASEDUNIT"
        ) && unit.enum_at(slot::IfcNamedUnit::UNIT_TYPE) == Some("LENGTHUNIT")
        {
            return true;
        }
    }
    false
}

/// Stored coordinates are metres; display follows the authored unit when it is
/// representable by [`Unit`], otherwise it stays metres.
fn unit_context(units: Units) -> UnitContext {
    let Units::Canonical(scale) = units else {
        return UnitContext::drawing_units();
    };
    let (display, display_per_source, _) = unit_display(scale);
    UnitContext {
        source: Unit::Meter,
        display,
        display_per_source: Some(display_per_source),
        decimal_places: 3,
    }
}

/// The display unit and its ratio from stored metres, for the four units the
/// domain models. The flag is `false` for a declared unit outside that set
/// (yard, kilometre, …), whose lengths then display as metres.
fn unit_display(scale: f64) -> (Unit, f64, bool) {
    let matches = |target: f64| (scale - target).abs() <= target.abs() * 1e-9;
    if matches(0.001) {
        (Unit::Millimeter, 1000.0, true)
    } else if matches(1.0) {
        (Unit::Meter, 1.0, true)
    } else if matches(0.0254) {
        (Unit::Inch, 1.0 / 0.0254, true)
    } else if matches(0.3048) {
        (Unit::Foot, 1.0 / 0.3048, true)
    } else {
        (Unit::Meter, 1.0, false)
    }
}

/// Record the unit behaviour as a diagnostic, marking `Partial` when unknown.
fn record_units(import: &mut MeshImport, units: Units) {
    match units {
        Units::Canonical(scale) => {
            let (_, _, representable) = unit_display(scale);
            let note = if representable {
                String::new()
            } else {
                "; this declared unit is not representable by the domain's unit set, so lengths \
                 display as metres"
                    .to_string()
            };
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UNITS_CANONICALISED.into(),
                    message: format!(
                        "ifc-lite-geometry scales every position and placement from the IFC \
                         declared length unit ({scale} m per file unit) to metres; the database \
                         stores metres{note}"
                    ),
                },
                None,
            );
        }
        Units::Refused => {
            // The caller refuses the whole import before this point.
        }
        Units::Unknown => {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UNITS_UNKNOWN.into(),
                    message: "the IFC project's length unit did not resolve; coordinates were \
                              left unscaled in the file's own numbers and are reported as \
                              drawing units, never assumed to be millimetres"
                        .into(),
                },
                Some(
                    "the IFC length unit did not resolve; coordinates are drawing units, not a \
                     known length unit"
                        .to_string(),
                ),
            );
        }
    }
}

/// Drain the router's own failure channel into stable diagnostics.
fn drain_router_diagnostics(router: &GeometryRouter, import: &mut MeshImport) {
    // Item-level drops inside a partially successful element are invisible in
    // the returned mesh; the router counts them by IFC type.
    let mut unsupported: Vec<(String, u64)> = router.take_unsupported_items().into_iter().collect();
    unsupported.sort_by(|left, right| left.0.cmp(&right.0));
    let mut unsupported_total = 0u64;
    for (type_name, count) in unsupported {
        unsupported_total += count;
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_ITEM_UNSUPPORTED.into(),
                message: format!(
                    "{count} {type_name} representation item(s) had no geometry processor or \
                     failed to mesh and were dropped from their product"
                ),
            },
            None,
        );
    }
    if unsupported_total > 0 {
        import.note_partial(format!(
            "{unsupported_total} IFC representation item(s) were unsupported or failed to mesh"
        ));
    }

    let mut failures = 0usize;
    let mut products = 0usize;
    for (product, product_failures) in router.take_csg_failures() {
        products += 1;
        for failure in product_failures {
            failures += 1;
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_CSG_FAILURE.into(),
                    message: format!("IFC product #{product}: {failure}"),
                },
                None,
            );
        }
    }
    if failures > 0 {
        import.note_partial(format!(
            "{failures} IFC opening/CSG operation(s) failed on {products} product(s); the \
             affected meshes may be uncut or partially cut"
        ));
    }
    let oversized = router.take_content_hash_oversized_ref_drops();
    if oversized > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_CONTENT_HASH_DROP.into(),
                message: format!(
                    "{oversized} IFC geometry reference(s) above the u32 mesh index range were \
                     refused by content dedup; the affected geometry may be incomplete"
                ),
            },
            Some(format!(
                "{oversized} IFC geometry reference(s) were refused above the u32 index range"
            )),
        );
    }
}

/// Convert one product's `ifc-lite` f32 parts into one validated domain mesh.
///
/// `parts` are already world-placed by the router; `Mesh::origin` (set when the
/// router relativised coordinates for f32 precision) is folded back in f64 so
/// the database never holds origin-relative coordinates.
fn element_mesh(
    parts: &[IfcMesh],
    label: &str,
    limits: &ImportLimits,
    import: &mut MeshImport,
) -> CadResult<Option<Mesh>> {
    let mut vertices = Vec::<Point3>::new();
    let mut normals = Vec::<Point3>::new();
    let mut normals_usable = true;
    let mut triangles = Vec::<[u32; 3]>::new();
    let mut degenerate = 0usize;

    for part in parts {
        if part.positions.len() % 3 != 0 {
            return Err(CadError::CorruptData(format!(
                "IFC product {label}: the geometry router returned {} position component(s), \
                 not a multiple of 3",
                part.positions.len()
            )));
        }
        if part.indices.len() % 3 != 0 {
            return Err(CadError::CorruptData(format!(
                "IFC product {label}: the geometry router returned {} triangle indices, not a \
                 multiple of 3",
                part.indices.len()
            )));
        }
        let part_vertices = part.positions.len() / 3;
        let base = vertices.len();
        if base + part_vertices > u32::MAX as usize {
            return Err(CadError::InvalidInput(format!(
                "IFC product {label} needs more than {} vertices, over the u32 mesh index range",
                u32::MAX
            )));
        }

        let origin = part.origin;
        for chunk in part.positions.as_chunks::<3>().0 {
            let point = Point3 {
                x: origin[0] + chunk[0] as f64,
                y: origin[1] + chunk[1] as f64,
                z: origin[2] + chunk[2] as f64,
            };
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return Err(CadError::CorruptData(format!(
                    "IFC product {label}: the geometry router produced a non-finite vertex"
                )));
            }
            vertices.push(point);
        }

        if normals_usable {
            if part.normals.len() == part.positions.len() {
                for chunk in part.normals.as_chunks::<3>().0 {
                    let normal = Point3 {
                        x: chunk[0] as f64,
                        y: chunk[1] as f64,
                        z: chunk[2] as f64,
                    };
                    if !normal.x.is_finite() || !normal.y.is_finite() || !normal.z.is_finite() {
                        normals_usable = false;
                        break;
                    }
                    normals.push(normal);
                }
            } else {
                normals_usable = false;
            }
            if !normals_usable {
                normals.clear();
            }
        }

        for corner in part.indices.as_chunks::<3>().0 {
            if corner.iter().any(|index| *index as usize >= part_vertices) {
                return Err(CadError::CorruptData(format!(
                    "IFC product {label}: a triangle references a vertex outside its {part_vertices}-vertex mesh"
                )));
            }
            let placed = [
                index_to_u32(base, corner[0], label)?,
                index_to_u32(base, corner[1], label)?,
                index_to_u32(base, corner[2], label)?,
            ];
            if placed[0] == placed[1] || placed[1] == placed[2] || placed[0] == placed[2] {
                degenerate += 1;
                continue;
            }
            triangles.push(placed);
            if let Some(max) = limits.max_mesh_faces {
                if triangles.len() > max {
                    return Err(CadError::InvalidInput(format!(
                        "IFC product {label} has more than {max} triangle(s), over the face \
                         import limit"
                    )));
                }
            }
        }
    }

    if degenerate > 0 {
        let reason = format!(
            "IFC product {label}: {degenerate} triangle(s) with repeated vertex indices were \
             dropped"
        );
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: reason.clone(),
            },
            Some(reason),
        );
    }
    if !normals_usable && !vertices.is_empty() {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_NORMALS_DROPPED.into(),
                message: format!(
                    "IFC product {label}: the router returned a missing, short or non-finite \
                     normal list; the mesh was stored without normals"
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
                "IFC product {label} has {indices} face corner indices, over the {max} \
                 face-index import limit"
            )));
        }
    }
    Ok(Some(Mesh {
        vertices,
        triangles,
        normals: if normals_usable { normals } else { Vec::new() },
        face_sources: Vec::new(),
        colors: Vec::new(),
    }))
}

fn index_to_u32(base: usize, index: u32, label: &str) -> CadResult<u32> {
    u32::try_from(base + index as usize).map_err(|_| {
        CadError::InvalidInput(format!(
            "IFC product {label} exceeds the u32 mesh index range"
        ))
    })
}

#[cfg(test)]
mod tests;
