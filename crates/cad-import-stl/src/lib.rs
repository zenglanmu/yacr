//! STereoLithography (STL) import adapter.
//!
//! STL is a bare triangle soup: no layers, no units, no colours, no document
//! model and no external references. Both ASCII and binary variants are read by
//! `stl_io` (pure Rust, no dependencies), and the result becomes one mesh entity
//! on layer `0` in model space with `UnitContext::drawing_units()`.
//!
//! STL normals are **per face**, while the neutral [`Mesh`] stores per-vertex
//! normals. Vertices are therefore expanded per triangle (flat shading) and
//! each corner carries its face normal. A stored normal that is non-finite or
//! zero is derived from the triangle geometry and reported as
//! [`DIAG_NORMAL_RECOMPUTED`]. Degenerate facets are dropped and reported as
//! [`DIAG_DEGENERATE_TRIANGLE`]; a truncated binary count is a hard
//! [`CadError::CorruptData`], never an empty success.

use std::io::Cursor;

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, Mesh, Point3, UnitContext};
use cad_import_core::{
    compute_identity, ImportLimits, ImportPhase, ImportProgress, ImportProgressSink, ImportReport,
    ImportRequest, ImportedDrawing, Importer, MeshImport, NoopProgress,
};

/// Stable diagnostic code: a degenerate facet was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.stl.degenerate_triangle";
/// Stable diagnostic code: an unusable stored normal was derived from geometry.
pub const DIAG_NORMAL_RECOMPUTED: &str = "import.stl.normal_recomputed";

/// The STL entity `type_key` reported in capabilities and stored rows.
pub const STL_MESH_TYPE_KEY: &str = "StlMesh";

/// Which STL container the bytes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StlVariant {
    Ascii,
    Binary,
}

impl StlVariant {
    pub fn label(self) -> &'static str {
        match self {
            StlVariant::Ascii => "ASCII",
            StlVariant::Binary => "binary",
        }
    }
}

/// Conservatively identify the STL container.
///
/// A binary STL has no magic; the standard definition is an 80-byte header plus
/// a little-endian `u32` facet count, so an exact `84 + 50*count` length is the
/// strongest signal. ASCII STL must begin with `solid ` (the exact prefix
/// `stl_io` itself probes for). Anything else is `None`, never a guess.
pub fn detect_variant(bytes: &[u8]) -> Option<StlVariant> {
    if let Some(count) = binary_facet_count(bytes) {
        let expected = 84u64.saturating_add(u64::from(count).saturating_mul(50));
        if count > 0 && expected == bytes.len() as u64 {
            return Some(StlVariant::Binary);
        }
    }
    if ascii_header(bytes) {
        return Some(StlVariant::Ascii);
    }
    None
}

fn binary_facet_count(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 84 {
        return None;
    }
    Some(u32::from_le_bytes([
        bytes[80], bytes[81], bytes[82], bytes[83],
    ]))
}

fn ascii_header(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).starts_with("solid ")
}

/// The STL importer.
#[derive(Default)]
pub struct StlImporter;

impl StlImporter {
    pub fn new() -> Self {
        Self
    }
}

impl Importer for StlImporter {
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
        if request.limits.max_entities == 0 {
            return Err(CadError::InvalidInput(
                "STL is one mesh entity, over the 0 entity import limit".into(),
            ));
        }

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        let variant = detect_variant(&request.bytes);
        let variant_label = variant.map(StlVariant::label).unwrap_or("unknown");
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        let parsed =
            stl_io::read_stl(&mut Cursor::new(request.bytes.as_ref())).map_err(|error| {
                CadError::CorruptData(format!("STL ({variant_label}) read failed: {error}"))
            })?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }

        let facet_count = parsed.faces.len();
        check_mesh_limits(facet_count, &request.limits)?;
        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, Some(1)));

        let non_finite = parsed
            .vertices
            .iter()
            .filter(|vertex| {
                let [x, y, z] = vertex.0;
                !x.is_finite() || !y.is_finite() || !z.is_finite()
            })
            .count();
        if non_finite > 0 {
            return Err(CadError::CorruptData(format!(
                "STL has {non_finite} non-finite vertex coordinate(s)"
            )));
        }

        let mut import = MeshImport::new(request, STL_MESH_TYPE_KEY)?;
        let object = import.reserve_object()?;
        let mut vertices = Vec::with_capacity(facet_count.saturating_mul(3));
        let mut normals = Vec::with_capacity(facet_count.saturating_mul(3));
        let mut triangles = Vec::with_capacity(facet_count);
        let mut degenerate = 0usize;
        let mut recomputed = 0usize;

        for face in &parsed.faces {
            let indices = face.vertices;
            if indices.iter().any(|index| *index >= parsed.vertices.len()) {
                return Err(CadError::CorruptData(
                    "STL facet references a vertex outside the vertex list".into(),
                ));
            }
            let points = [
                to_point(parsed.vertices[indices[0]]),
                to_point(parsed.vertices[indices[1]]),
                to_point(parsed.vertices[indices[2]]),
            ];
            if indices[0] == indices[1] || indices[1] == indices[2] || indices[0] == indices[2] {
                degenerate += 1;
                continue;
            }
            let Some(geometric) = triangle_normal(points) else {
                degenerate += 1;
                continue;
            };
            let normal = match stored_normal(face.normal) {
                Some(normal) => normal,
                None => {
                    recomputed += 1;
                    geometric
                }
            };
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&points);
            normals.extend_from_slice(&[normal, normal, normal]);
            triangles.push([base, base + 1, base + 2]);
        }

        if triangles.is_empty() {
            return Err(CadError::CorruptData(
                "STL contains no non-degenerate triangle".into(),
            ));
        }
        if degenerate > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_DEGENERATE_TRIANGLE.into(),
                    message: format!(
                        "{degenerate} degenerate STL facet(s) (repeated vertex or zero area) were dropped"
                    ),
                },
                Some(format!(
                    "{degenerate} degenerate STL facet(s) were dropped"
                )),
            );
        }
        if recomputed > 0 {
            // A missing/zero normal is recoverable from the triangle itself;
            // this is a normal note, not lost geometry, so it does not make the
            // import Partial.
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_NORMAL_RECOMPUTED.into(),
                    message: format!(
                        "{recomputed} STL face normal(s) were missing, zero or non-finite and were derived from the triangle geometry"
                    ),
                },
                None,
            );
        }

        import.push_mesh(
            object,
            Mesh {
                vertices,
                triangles,
                normals,
                face_sources: Vec::new(),
                colors: Vec::new(),
            },
            EntityColor::ByLayer,
        )?;
        progress.report(ImportProgress::at(ImportPhase::Entities, 1, Some(1)));
        progress.report(ImportProgress::at(ImportPhase::Finishing, 1, Some(1)));

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
            units: UnitContext::drawing_units(),
            report,
            vendor_records: Vec::new(),
        })
    }
}

fn check_mesh_limits(triangles: usize, limits: &ImportLimits) -> CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if triangles > max {
            return Err(CadError::InvalidInput(format!(
                "STL has {triangles} triangle(s), over the {max} face import limit"
            )));
        }
    }
    let indices = triangles.saturating_mul(3);
    if let Some(max) = limits.max_mesh_face_indices {
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "STL has {indices} face corner indices, over the {max} face-index import limit"
            )));
        }
    }
    Ok(())
}

fn to_point(vertex: stl_io::Vertex) -> Point3 {
    let [x, y, z] = vertex.0;
    Point3 {
        x: x as f64,
        y: y as f64,
        z: z as f64,
    }
}

/// A usable stored normal, or `None` when it must be derived.
fn stored_normal(normal: stl_io::Normal) -> Option<Point3> {
    let point = Point3 {
        x: normal.0[0] as f64,
        y: normal.0[1] as f64,
        z: normal.0[2] as f64,
    };
    if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
        return None;
    }
    if point.x == 0.0 && point.y == 0.0 && point.z == 0.0 {
        return None;
    }
    Some(point)
}

/// Normalised triangle normal, or `None` for a zero-area triangle.
fn triangle_normal(points: [Point3; 3]) -> Option<Point3> {
    let ux = points[1].x - points[0].x;
    let uy = points[1].y - points[0].y;
    let uz = points[1].z - points[0].z;
    let vx = points[2].x - points[0].x;
    let vy = points[2].y - points[0].y;
    let vz = points[2].z - points[0].z;
    let x = uy * vz - uz * vy;
    let y = uz * vx - ux * vz;
    let z = ux * vy - uy * vx;
    let length = (x * x + y * y + z * z).sqrt();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    Some(Point3 {
        x: x / length,
        y: y / length,
        z: z / length,
    })
}

#[cfg(test)]
mod tests;
