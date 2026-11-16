//! Reverse map for a [`SemanticGeometry::Body3d`] and the polyface writer it
//! shares with a plain [`SemanticGeometry::Mesh`].
//!
//! The DXF exporter has no ACIS encoder, so a body's **exact** B-rep is never
//! serialised: writing a `3DSOLID`/`BODY`/`REGION` record would fabricate a
//! payload this exporter cannot produce. What is written is the honest part —
//! the derived display cache:
//!
//! * a body with a facet mesh becomes a `POLYFACE` mesh, byte for byte the same
//!   record a standalone [`SemanticGeometry::Mesh`] writes today;
//! * a body with only boundary polylines (a wire/edge-only body) becomes
//!   `LINE` / `POLYLINE3D` entities, one per polyline;
//! * a body with neither is dropped with a named reason.
//!
//! Every outcome is recorded in the [`ExportReport`](super::report::ExportReport),
//! so the report never claims exact geometry was written.

use acadrust::entities::{
    EntityType, Line as DxfLine, PolyfaceFace, PolyfaceMesh, PolyfaceVertex, Polyline3D,
};
use acadrust::types::Vector3;
use acadrust::CadDocument;
use cad_db::DbEntity;
use cad_domain::{Body3d, CadResult, Mesh, Point3};

use super::accumulator::ExportAccumulator;
use super::common::{common_for, v3};
use super::entity::EntityContext;
use super::report::MappingOutcome;

/// The largest polyface vertex index that fits `PolyfaceFace`'s signed `i16`
/// fields (1-based, so a 0-based index must be `< 32767`).
pub(super) const MAX_POLYFACE_INDEX: usize = 32766;

/// Why a body's exact geometry is not in the output.
///
/// The database keeps the lifted B-rep, but this exporter has no `3DSOLID`
/// encoder and the source SAT/SAB bytes were never retained, so only the
/// derived display is written.
pub(super) const BODY_EXACT_NOT_WRITTEN_REASON: &str =
    "this 3D body keeps its exact B-Rep in the database, but the DXF exporter has no 3DSOLID \
     encoder and the original SAT/SAB source is not retained: the exact geometry is not written, \
     so the body is exported as its display cache, not as a 3DSOLID";

/// Write one triangle mesh as a DXF `POLYFACE` mesh.
///
/// Shared by the plain `Mesh` arm and a body's display cache so both write the
/// same bytes for the same facets. `base_reasons` carries the source-specific
/// losses to record; the generic per-vertex loss is appended here.
pub(super) fn emit_polyface_mesh(
    doc: &mut CadDocument,
    entity: &DbEntity,
    mesh: &Mesh,
    base_reasons: Vec<String>,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    // Polyface face indices are signed i16 (1-based). A mesh outside that range
    // must not be emitted with wrapped indices, which would corrupt the file;
    // drop it explicitly instead of erroring the whole export.
    let index_in_range = mesh.vertices.len() <= MAX_POLYFACE_INDEX + 1
        && mesh
            .triangles
            .iter()
            .all(|triangle| triangle.iter().all(|i| (*i as usize) <= MAX_POLYFACE_INDEX));
    if !index_in_range {
        acc.record(
            object,
            "polyface_index_range",
            MappingOutcome::Dropped(vec![format!(
                "mesh has {} vertices or a face index beyond the polyface mesh i16 range \
                 (max {MAX_POLYFACE_INDEX}); not emitted",
                mesh.vertices.len()
            )]),
        );
        return Ok(());
    }
    let (common, mut reasons) = common_for(doc, entity, ctx);
    let mut polyface = PolyfaceMesh::new();
    for vertex in &mesh.vertices {
        polyface.add_vertex(PolyfaceVertex::new(v3(*vertex)));
    }
    for triangle in &mesh.triangles {
        polyface.add_face(PolyfaceFace {
            index1: triangle[0] as i16 + 1,
            index2: triangle[1] as i16 + 1,
            index3: triangle[2] as i16 + 1,
            index4: 0,
            ..PolyfaceFace::default()
        });
    }
    polyface.common = common;
    acc.push(EntityType::PolyfaceMesh(polyface), ctx)?;
    reasons.extend(base_reasons);
    reasons
        .push("mesh written as a polyface mesh; per-vertex colours and normals are dropped".into());
    acc.record(object, "mesh", MappingOutcome::Converted(reasons));
    Ok(())
}

/// Write a [`Body3d`] through its derived display cache, never its exact B-rep.
pub(super) fn emit_body3d(
    doc: &mut CadDocument,
    entity: &DbEntity,
    body: &Body3d,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    if !body.display.mesh.triangles.is_empty() {
        let mut reasons = vec![BODY_EXACT_NOT_WRITTEN_REASON.to_string()];
        if !body.display.edges.is_empty() {
            reasons.push(format!(
                "the body's {} display edge polyline(s) are not part of the polyface mesh and \
                 are not written separately",
                body.display.edges.len()
            ));
        }
        return emit_polyface_mesh(doc, entity, &body.display.mesh, reasons, ctx, acc);
    }

    let polylines: Vec<&Vec<Point3>> = body
        .display
        .edges
        .iter()
        .filter(|polyline| polyline.len() >= 2)
        .collect();
    if polylines.is_empty() {
        acc.record(
            object,
            "body3d",
            MappingOutcome::Dropped(vec![format!(
                "3D body has no display mesh or edge polylines to write; its exact ACIS \
                 geometry is not written and no 3DSOLID/BODY/REGION record is fabricated \
                 (display tolerance {:.6e})",
                body.display.tolerance.unwrap_or(0.0)
            )]),
        );
        return Ok(());
    }

    let (common, mut reasons) = common_for(doc, entity, ctx);
    let mut lines = 0usize;
    let mut polyline_records = 0usize;
    for polyline in &polylines {
        if polyline.len() == 2 {
            let mut line = DxfLine::new();
            line.start = v3(polyline[0]);
            line.end = v3(polyline[1]);
            line.normal = Vector3::UNIT_Z;
            line.common = common.clone();
            acc.push(EntityType::Line(line), ctx)?;
            lines += 1;
        } else {
            let mut poly = Polyline3D::from_points(polyline.iter().map(|p| v3(*p)).collect());
            poly.common = common.clone();
            acc.push(EntityType::Polyline3D(poly), ctx)?;
            polyline_records += 1;
        }
    }
    // The body's exact geometry (when it has any) is still not written.
    reasons.push(BODY_EXACT_NOT_WRITTEN_REASON.to_string());
    reasons.push(format!(
        "the edge-only body was written as its {lines} LINE and {polyline_records} POLYLINE3D \
         display edge record(s); the body entity type is not preserved"
    ));
    acc.record(object, "wire_body", MappingOutcome::Converted(reasons));
    Ok(())
}
