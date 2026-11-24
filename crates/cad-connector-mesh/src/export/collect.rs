//! Source geometry collection shared by every writer.
//!
//! `cad-db` stores entities in a `BTreeMap<EntityId, _>`, so the iteration
//! order is already deterministic; the collection is sorted by `EntityId`
//! anyway as a defensive guarantee that the output byte order does not depend
//! on a container detail. Only two geometries are writable here — a standalone
//! [`SemanticGeometry::Mesh`] and the display cache of a
//! [`SemanticGeometry::Body3d`] — and every other entity produces one explicit
//! drop entry instead of being skipped. Model space is the only space written;
//! paper layouts and block definitions are dropped by name.
//!
//! A [`SemanticGeometry::Body3d`] may also carry stored display-edge polylines
//! (`BodyDisplay::edges`). The neutral-mesh formats have no edge channel, so a
//! body that still writes its triangles keeps a count of those polylines and
//! segments here for the shared entry point to report once per entity; a body
//! with edges but no triangles is a wire body and is dropped as a whole with
//! its own truthful code, never reported as edge loss on written geometry.

use cad_connector_core::ExportJob;
use cad_db::DbEntity;
use cad_domain::{CadError, CadResult, ObjectId, Point3, SemanticGeometry, SpaceId};

/// One writable source entity, copied out of the read-only database.
pub(crate) struct SourceMesh {
    pub object: ObjectId,
    pub type_key: String,
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Empty or one finite normal per vertex (the `cad-db` invariant).
    pub normals: Vec<Point3>,
    /// Empty or one sRGB colour per vertex (the `cad-db` invariant).
    pub colors: Vec<[u8; 3]>,
    /// The document texture key, when the source mesh references a base-colour
    /// texture. The neutral-mesh formats have no texture channel, so a writer
    /// that writes the geometry still reports the texture as a conversion.
    pub texture: Option<String>,
    /// Stored display-edge polylines carried by the source body, `0` for a
    /// standalone [`SemanticGeometry::Mesh`] (which has no edge cache).
    pub edge_polylines: usize,
    /// Total stored display-edge segments, `Σ(polyline.len() − 1)`. The
    /// neutral-mesh formats write surfaces only, so a written body with a
    /// non-zero count reports one conversion naming this number.
    pub edge_segments: usize,
}

/// One source entity that cannot be written, with its stable marker.
pub(crate) struct DropEntry {
    pub object: ObjectId,
    pub type_key: String,
    /// The format-scoped code (`export.<format_id>.<reason>`), so it is built
    /// rather than borrowed.
    pub code: String,
    pub message: String,
}

/// Everything a writer needs from the source document.
pub(crate) struct Collected {
    pub meshes: Vec<SourceMesh>,
    pub drops: Vec<DropEntry>,
}

impl Collected {
    pub(crate) fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|mesh| mesh.triangles.len()).sum()
    }

    /// Whether any source mesh carries per-vertex colours.
    pub(crate) fn has_any_colors(&self) -> bool {
        self.meshes.iter().any(|mesh| !mesh.colors.is_empty())
    }

    /// Whether every included vertex has a colour.
    ///
    /// A partially coloured source cannot be written as a colour format without
    /// inventing a colour for the remaining vertices, so the writers that do
    /// support colour require this instead.
    pub(crate) fn colors_complete(&self) -> bool {
        !self.meshes.is_empty()
            && self
                .meshes
                .iter()
                .all(|mesh| !mesh.colors.is_empty() && mesh.colors.len() == mesh.vertices.len())
    }

    /// Whether any source mesh carries per-vertex normals.
    pub(crate) fn has_any_normals(&self) -> bool {
        self.meshes.iter().any(|mesh| !mesh.normals.is_empty())
    }

    /// Whether every included vertex has a finite normal.
    pub(crate) fn normals_complete(&self) -> bool {
        !self.meshes.is_empty()
            && self.meshes.iter().all(|mesh| {
                !mesh.normals.is_empty()
                    && mesh.normals.len() == mesh.vertices.len()
                    && mesh.normals.iter().all(|normal| {
                        normal.x.is_finite() && normal.y.is_finite() && normal.z.is_finite()
                    })
            })
    }
}

/// Collect the writable meshes, enforcing the entity and triangle budgets.
///
/// The entity limit is checked against the *source* entity count and the
/// triangle limit incrementally against every triangle that would be written,
/// **before** the entity is copied out of the database: an over-budget document
/// fails without allocating the copies. Exceeding either is an explicit error
/// naming the writer; nothing is truncated. A document with no writable mesh is
/// refused instead of producing an empty file.
///
/// `report_prefix` is the writer's report-code prefix (`stl`, `obj`, `ply`,
/// `off`); the two STL containers share `stl`, matching their other codes.
pub(crate) fn collect(
    job: &ExportJob<'_>,
    format_id: &str,
    report_prefix: &str,
) -> CadResult<Collected> {
    let entity_count = job.database.entity_count();
    if entity_count > job.limits.max_output_entities {
        return Err(CadError::InvalidInput(format!(
            "export.{format_id}.limits: {entity_count} source entities exceed the {} entity export limit",
            job.limits.max_output_entities
        )));
    }

    let mut entities: Vec<&DbEntity> = job.database.entities().collect();
    entities.sort_by_key(|entity| entity.id);

    let mut meshes = Vec::new();
    let mut drops = Vec::new();
    let mut triangles = 0usize;
    for entity in entities {
        let object = entity.object.id;
        let type_key = entity.object.type_key.clone();
        if entity.space != SpaceId::Model {
            drops.push(DropEntry {
                object,
                type_key,
                code: format!("export.{format_id}.space_not_exported"),
                message: format!(
                    "entity #{} is outside model space ({:?}); the neutral-mesh writers export model space only",
                    object.0, entity.space
                ),
            });
            continue;
        }
        let (mesh, edge_polylines, edge_segments) = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => {
                if mesh.triangles.is_empty() {
                    drops.push(DropEntry {
                        object,
                        type_key,
                        code: "export.mesh.empty_mesh".to_string(),
                        message: "Mesh has no triangles and cannot be written".to_string(),
                    });
                    continue;
                }
                (mesh, 0, 0)
            }
            SemanticGeometry::Body3d(body) => {
                if body.display.mesh.triangles.is_empty() {
                    // A wire body is a body with stored edges and no surface.
                    // The format has no edge channel, so nothing of it can be
                    // written: drop it as a whole with its real segment count
                    // instead of reporting edge loss on geometry that is absent.
                    if !body.display.edges.is_empty() {
                        let segments = count_edge_segments(&body.display.edges);
                        drops.push(DropEntry {
                            object,
                            type_key,
                            code: format!("export.{report_prefix}.wire_body_not_written"),
                            message: format!(
                                "entity #{} ({}) carries {segments} stored display edge \
                                 segment(s) and no triangles; this format has no edge channel, so \
                                 the wire body was not written",
                                object.0, entity.object.type_key
                            ),
                        });
                        continue;
                    }
                    drops.push(DropEntry {
                        object,
                        type_key,
                        code: "export.mesh.empty_mesh".to_string(),
                        message: "Body3d has no display mesh; only exact geometry is stored"
                            .to_string(),
                    });
                    continue;
                }
                (
                    &body.display.mesh,
                    body.display.edges.len(),
                    count_edge_segments(&body.display.edges),
                )
            }
            other => {
                let variant = geometry_variant(other);
                drops.push(DropEntry {
                    object,
                    type_key,
                    code: "export.mesh.unsupported_geometry".to_string(),
                    message: format!(
                        "{variant} is not mesh geometry and cannot be written to a neutral-mesh format"
                    ),
                });
                continue;
            }
        };
        // Count before cloning: refusing an over-budget document must not first
        // allocate a copy of every mesh.
        triangles = triangles.saturating_add(mesh.triangles.len());
        if triangles > job.limits.max_output_triangles {
            return Err(CadError::InvalidInput(format!(
                "export.{format_id}.limits: {triangles} triangles exceed the {} triangle export limit",
                job.limits.max_output_triangles
            )));
        }
        meshes.push(SourceMesh {
            object,
            type_key,
            vertices: mesh.vertices.clone(),
            triangles: mesh.triangles.clone(),
            normals: mesh.normals.clone(),
            colors: mesh.colors.clone(),
            texture: mesh.texture.clone(),
            edge_polylines,
            edge_segments,
        });
    }

    if meshes.is_empty() {
        return Err(CadError::Unsupported(format!(
            "export.{format_id}.no_geometry: {} source entity(ies) produced no writable mesh geometry, so no file was written",
            drops.len()
        )));
    }

    Ok(Collected { meshes, drops })
}

/// The number of segments in a set of stored display-edge polylines.
///
/// One polyline of `n` points is `n − 1` segments (a stored edge polyline has
/// at least two points; the saturating subtraction keeps a defensive zero for a
/// hand-built shorter one instead of underflowing).
fn count_edge_segments(polylines: &[Vec<Point3>]) -> usize {
    polylines
        .iter()
        .map(|polyline| polyline.len().saturating_sub(1))
        .sum()
}

/// The variant name reported for an unsupported geometry.
fn geometry_variant(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "Line",
        SemanticGeometry::Polyline { .. } => "Polyline",
        SemanticGeometry::Circle { .. } => "Circle",
        SemanticGeometry::Arc { .. } => "Arc",
        SemanticGeometry::Ellipse { .. } => "Ellipse",
        SemanticGeometry::Spline { .. } => "Spline",
        SemanticGeometry::Point(_) => "Point",
        SemanticGeometry::Mesh(_) => "Mesh",
        SemanticGeometry::Body3d(_) => "Body3d",
        SemanticGeometry::Insert { .. } => "Insert",
        SemanticGeometry::Text { .. } => "Text",
        SemanticGeometry::Shape { .. } => "Shape",
        SemanticGeometry::Opaque { .. } => "Opaque",
        SemanticGeometry::Compound(_) => "Compound",
        SemanticGeometry::Dimension(_) => "Dimension",
        SemanticGeometry::Hatch(_) => "Hatch",
        SemanticGeometry::MultiLeader(_) => "MultiLeader",
        SemanticGeometry::Table(_) => "Table",
        SemanticGeometry::Image { .. } => "Image",
        SemanticGeometry::Mask { .. } => "Mask",
        SemanticGeometry::AttributeDefinition(_) => "AttributeDefinition",
    }
}
