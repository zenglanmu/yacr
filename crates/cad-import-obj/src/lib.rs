//! Wavefront OBJ import adapter.
//!
//! OBJ is a triangle/polygon soup with no document model, no layers, no units
//! and no external references. Every `tobj` model becomes one
//! [`SemanticGeometry::Mesh`] entity on layer `0` in model space, with
//! `UnitContext::drawing_units()` (OBJ is unitless) and empty
//! [`ImportedDrawing::vendor_records`]. The shared database scaffolding lives in
//! [`cad_import_core::MeshImport`], so STL and a future STEP tessellation path
//! publish identical capability rows.
//!
//! Losses are never silent:
//! * `tobj` is asked to `triangulate`, so it fan-triangulates every face with
//!   more than three vertices and leaves `face_arities` empty. The raw OBJ text
//!   is therefore scanned for `f` records with more than three corners; finding
//!   any is recorded as [`DIAG_NGON_FAN`] and makes the import
//!   `Completeness::Partial`, because fan triangulation can be wrong for
//!   concave n-gons.
//! * Degenerate (repeated-index or zero-area) triangles are dropped and
//!   reported as [`DIAG_DEGENERATE_TRIANGLE`] rather than written to `cad-db`.
//! * A referenced `mtllib` is not loaded: the import contract is bytes-only and
//!   has no file source. `tobj` returns the geometry with the material `Result`
//!   in error; that loss is reported as [`DIAG_MTL_NOT_LOADED`] and the entity
//!   colour falls back to `ByLayer`.

use std::io::Cursor;

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, Mesh, Point3, UnitContext};
use cad_import_core::{
    compute_identity, ImportLimits, ImportPhase, ImportProgress, ImportProgressSink, ImportReport,
    ImportRequest, ImportedDrawing, Importer, MeshImport, NoopProgress,
};
use tobj::{LoadOptions, Material, Model};

/// Stable diagnostic code: an n-gon was fan-triangulated.
pub const DIAG_NGON_FAN: &str = "import.obj.ngon_fan_triangulated";
/// Stable diagnostic code: a degenerate triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.obj.degenerate_triangle";
/// Stable diagnostic code: a referenced material library could not be loaded.
pub const DIAG_MTL_NOT_LOADED: &str = "import.obj.mtl_not_loaded";
/// Stable diagnostic code: an object produced no usable triangles.
pub const DIAG_EMPTY_OBJECT: &str = "import.obj.empty_object";

/// The OBJ entity `type_key` reported in capabilities and stored rows.
pub const OBJ_MESH_TYPE_KEY: &str = "WavefrontObjMesh";

/// The Wavefront OBJ importer.
#[derive(Default)]
pub struct ObjImporter;

impl ObjImporter {
    pub fn new() -> Self {
        Self
    }
}

/// One object/material group: models split by a mid-object `usemtl` share a
/// name but carry different material ids, so the material is part of the key.
struct Group {
    object: cad_domain::ObjectId,
    name: String,
    material: Option<usize>,
    mesh: Mesh,
}

impl Importer for ObjImporter {
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

        // `triangulate` is required: it guarantees every `indices` triple is a
        // triangle. `single_index` keeps positions/normals/colours aligned by
        // one index, which is the shape `cad_domain::Mesh` consumes. `tobj`
        // empties `face_arities` when triangulating, so n-gons are detected from
        // the raw text (`count_ngon_faces`) instead.
        let options = LoadOptions {
            single_index: true,
            triangulate: true,
            ..Default::default()
        };
        let (models, materials) = tobj::load_obj_buf(
            &mut Cursor::new(request.bytes.as_ref()),
            &options,
            // Bytes-only import: there is no file source for `mtllib`.
            |_| Err(tobj::LoadError::OpenFileFailed),
        )
        .map_err(|error| CadError::CorruptData(format!("Wavefront OBJ parse failed: {error}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if models.len() > request.limits.max_entities {
            return Err(CadError::InvalidInput(format!(
                "OBJ has {} object/model group(s), over the {} entity import limit",
                models.len(),
                request.limits.max_entities
            )));
        }

        let mut import = MeshImport::new(request, OBJ_MESH_TYPE_KEY)?;
        let materials = match materials {
            Ok(materials) => materials,
            Err(_) => {
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_MTL_NOT_LOADED.into(),
                        message: "the OBJ references a material library, but this bytes-only \
                                  import has no file source for it; material colours were not \
                                  applied"
                            .into(),
                    },
                    Some(
                        "a referenced OBJ material library was not loaded; colours fall back to \
                         ByLayer"
                            .to_string(),
                    ),
                );
                Vec::new()
            }
        };

        let ngons = count_ngon_faces(&request.bytes);
        if ngons > 0 {
            let message = format!(
                "{ngons} face(s) with more than 3 vertices were fan-triangulated by the OBJ \
                 reader; concave n-gons may be incorrect"
            );
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_NGON_FAN.into(),
                    message: message.clone(),
                },
                Some(message),
            );
        }

        progress.report(ImportProgress::at(
            ImportPhase::Parsing,
            0,
            Some(models.len()),
        ));

        let mut groups: Vec<Group> = Vec::new();
        for model in &models {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let Some(mesh) = build_mesh(model, &request.limits, &mut import)? else {
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_EMPTY_OBJECT.into(),
                        message: format!(
                            "object '{}' contained no valid triangles and was not imported",
                            model.name
                        ),
                    },
                    Some("at least one object contained no valid triangles".to_string()),
                );
                continue;
            };
            let material = model.mesh.material_id;
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.name == model.name && group.material == material)
            {
                merge_mesh(&mut group.mesh, mesh);
            } else {
                let object = import.reserve_object()?;
                groups.push(Group {
                    object,
                    name: model.name.clone(),
                    material,
                    mesh,
                });
            }
        }

        // Grouping has to see every model before an entity can be inserted, so
        // the entity phase advances once, with the real inserted total.
        let entity_total = groups.len();
        for group in groups {
            check_mesh_limits(&group.name, group.mesh.triangles.len(), &request.limits)?;
            let color = material_color(group.material, &materials);
            import.push_mesh(group.object, group.mesh, color)?;
        }

        if import.entity_count() == 0 {
            return Err(CadError::InvalidInput(
                "OBJ file produced no drawable mesh geometry".into(),
            ));
        }

        progress.report(ImportProgress::at(
            ImportPhase::Entities,
            import.entity_count(),
            Some(entity_total),
        ));
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
            // OBJ is unitless: never assume millimetres.
            units: UnitContext::drawing_units(),
            report,
            vendor_records: Vec::new(),
        })
    }
}

/// Build one `tobj` model's neutral mesh.
///
/// `tobj` has already fan-triangulated, so every `indices` triple is a face.
/// Returning `Ok(None)` means nothing survived validation.
fn build_mesh(
    model: &Model,
    limits: &ImportLimits,
    import: &mut MeshImport,
) -> CadResult<Option<Mesh>> {
    let raw = &model.mesh;
    if !raw.indices.len().is_multiple_of(3) {
        return Err(CadError::CorruptData(format!(
            "OBJ object '{}' has {} corner indices, not a multiple of 3",
            model.name,
            raw.indices.len()
        )));
    }
    if !raw.positions.len().is_multiple_of(3) {
        return Err(CadError::CorruptData(format!(
            "OBJ object '{}' has {} position component(s), not a multiple of 3",
            model.name,
            raw.positions.len()
        )));
    }
    let vertex_count = raw.positions.len() / 3;
    let triangles_total = raw.indices.len() / 3;
    check_mesh_limits(&model.name, triangles_total, limits)?;

    let mut vertices = Vec::with_capacity(vertex_count);
    let mut non_finite = 0usize;
    for chunk in raw.positions.as_chunks::<3>().0 {
        let point = Point3 {
            x: chunk[0] as f64,
            y: chunk[1] as f64,
            z: chunk[2] as f64,
        };
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            non_finite += 1;
        }
        vertices.push(point);
    }
    // A non-finite coordinate cannot be "fixed" without inventing geometry.
    if non_finite > 0 {
        return Err(CadError::CorruptData(format!(
            "OBJ object '{}' has {non_finite} non-finite vertex coordinate(s)",
            model.name
        )));
    }

    let mut triangles = Vec::with_capacity(triangles_total);
    let mut degenerate = 0usize;
    for corner in raw.indices.as_chunks::<3>().0 {
        push_triangle(
            [corner[0], corner[1], corner[2]],
            &vertices,
            &mut triangles,
            &mut degenerate,
            &model.name,
        )?;
    }
    if degenerate > 0 {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: format!(
                    "object '{}': {degenerate} degenerate triangle(s) (repeated vertex or zero area) were dropped",
                    model.name
                ),
            },
            Some(format!(
                "object '{}': {degenerate} degenerate triangle(s) were dropped",
                model.name
            )),
        );
    }
    if triangles.is_empty() {
        return Ok(None);
    }

    Ok(Some(Mesh {
        vertices,
        triangles,
        // Empty means "no normals"; a partial list would be ambiguous and
        // consumers index it by vertex, so it is dropped as a whole.
        normals: map_normals(raw, vertex_count),
        face_sources: Vec::new(),
        colors: map_colors(raw, vertex_count),
    }))
}

fn check_mesh_limits(name: &str, triangles: usize, limits: &ImportLimits) -> CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if triangles > max {
            return Err(CadError::InvalidInput(format!(
                "OBJ object '{name}' has {triangles} triangle(s), over the {max} face import limit"
            )));
        }
    }
    let indices = triangles.saturating_mul(3);
    if let Some(max) = limits.max_mesh_face_indices {
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "OBJ object '{name}' has {indices} face corner indices, over the {max} face-index import limit"
            )));
        }
    }
    Ok(())
}

/// Reject a triangle that would violate `cad-db`'s mesh invariants.
fn push_triangle(
    triangle: [u32; 3],
    vertices: &[Point3],
    triangles: &mut Vec<[u32; 3]>,
    degenerate: &mut usize,
    object_name: &str,
) -> CadResult<()> {
    let count = vertices.len() as u32;
    if triangle.iter().any(|i| *i >= count) {
        return Err(CadError::CorruptData(format!(
            "OBJ object '{object_name}' face references a vertex outside the {count}-vertex mesh"
        )));
    }
    if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
        *degenerate += 1;
        return Ok(());
    }
    if is_zero_area(triangle, vertices) {
        *degenerate += 1;
        return Ok(());
    }
    triangles.push(triangle);
    Ok(())
}

fn is_zero_area(triangle: [u32; 3], vertices: &[Point3]) -> bool {
    let p0 = vertices[triangle[0] as usize];
    let p1 = vertices[triangle[1] as usize];
    let p2 = vertices[triangle[2] as usize];
    let ux = p1.x - p0.x;
    let uy = p1.y - p0.y;
    let uz = p1.z - p0.z;
    let vx = p2.x - p0.x;
    let vy = p2.y - p0.y;
    let vz = p2.z - p0.z;
    let cx = uy * vz - uz * vy;
    let cy = uz * vx - ux * vz;
    let cz = ux * vy - uy * vx;
    cx == 0.0 && cy == 0.0 && cz == 0.0
}

fn map_normals(raw: &tobj::Mesh, vertex_count: usize) -> Vec<Point3> {
    if raw.normals.len() != vertex_count * 3 {
        // Mixed per-face normals can leave a shorter list under `single_index`.
        // A short/partial list would be indexed out of bounds by consumers, so
        // follow the established "empty means none" convention.
        return Vec::new();
    }
    let mut normals = Vec::with_capacity(vertex_count);
    for chunk in raw.normals.as_chunks::<3>().0 {
        let normal = Point3 {
            x: chunk[0] as f64,
            y: chunk[1] as f64,
            z: chunk[2] as f64,
        };
        if !normal.x.is_finite() || !normal.y.is_finite() || !normal.z.is_finite() {
            return Vec::new();
        }
        normals.push(normal);
    }
    normals
}

fn map_colors(raw: &tobj::Mesh, vertex_count: usize) -> Vec<[u8; 3]> {
    if raw.vertex_color.len() != vertex_count * 3 {
        return Vec::new();
    }
    raw.vertex_color
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            [
                unit_to_u8(chunk[0]),
                unit_to_u8(chunk[1]),
                unit_to_u8(chunk[2]),
            ]
        })
        .collect()
}

fn unit_to_u8(value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The entity colour a model's material provides, if any.
///
/// The bytes-only import never loads an external `mtllib`, so this normally
/// falls back to `ByLayer`. Keeping the mapping explicit means a future
/// in-memory MTL source cannot silently change the convention.
fn material_color(material_id: Option<usize>, materials: &[Material]) -> EntityColor {
    let Some(id) = material_id else {
        return EntityColor::ByLayer;
    };
    let Some(diffuse) = materials.get(id).and_then(|material| material.diffuse) else {
        return EntityColor::ByLayer;
    };
    if !diffuse.iter().all(|channel| channel.is_finite()) {
        return EntityColor::ByLayer;
    }
    EntityColor::Explicit([
        unit_to_u8(diffuse[0]),
        unit_to_u8(diffuse[1]),
        unit_to_u8(diffuse[2]),
    ])
}

/// Count raw `f` records with more than three corners.
///
/// `tobj` is asked to triangulate, which discards the original arity, so this
/// conservative text scan recovers exactly the fact the report needs. It walks
/// the whole file (the import already holds it) and ignores blank lines and
/// `#` comments.
fn count_ngon_faces(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    let mut count = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        if tokens.next() != Some("f") {
            continue;
        }
        let corners = tokens.take_while(|token| !token.starts_with('#')).count();
        if corners > 3 {
            count += 1;
        }
    }
    count
}

/// Merge a split model back into its object/material group.
fn merge_mesh(target: &mut Mesh, source: Mesh) {
    let target_vertices = target.vertices.len();
    let source_vertices = source.vertices.len();
    let target_has_normals = target.normals.len() == target_vertices;
    let source_has_normals = source.normals.len() == source_vertices;
    let target_has_colors = target.colors.len() == target_vertices;
    let source_has_colors = source.colors.len() == source_vertices;

    let offset = target_vertices as u32;
    target.vertices.extend(source.vertices);
    target
        .triangles
        .extend(source.triangles.into_iter().map(|triangle| {
            [
                triangle[0] + offset,
                triangle[1] + offset,
                triangle[2] + offset,
            ]
        }));

    if target_has_normals && source_has_normals {
        target.normals.extend(source.normals);
    } else {
        target.normals.clear();
    }
    if target_has_colors && source_has_colors {
        target.colors.extend(source.colors);
    } else {
        target.colors.clear();
    }
}

#[cfg(test)]
mod tests;
