//! Reverse map for a [`SemanticGeometry::Body3d`] and the polyface writer it
//! shares with a plain [`SemanticGeometry::Mesh`].
//!
//! An exact, single-shell, planar, line-bounded solid is written as a real
//! `3DSOLID` carrying ACIS built by [`super::acis`] — in **both** formats, with
//! only the payload encoding differing: a R2010 DXF carries ACIS 7.0 SAT text
//! inline, a R2013/R2018 DXF carries the chosen ACIS version (7.0 or 21.0/ASM) as
//! a binary SAB blob in `ACDSDATA`, and a DWG is AC1032 and carries the same
//! topology as a binary SAB blob in the AcDs (AcDsPrototype_1b) section, which is
//! what R2013+ stores. Everything else is written as the honest part — the
//! derived display cache:
//!
//! * a body with a facet mesh becomes a `POLYFACE` mesh, byte for byte the same
//!   record a standalone [`SemanticGeometry::Mesh`] writes today;
//! * a body with only boundary polylines (a wire/edge-only body) becomes
//!   `LINE` / `POLYLINE3D` entities, one per polyline;
//! * a body with neither is dropped with a named reason.
//!
//! The ACIS writer refuses every B-rep it cannot represent exactly (a curved
//! face, a non-line boundary curve, a void/multi-shell body, a shear or mirrored
//! placement); that refusal is recorded with its stable `acis.*` code and the
//! display cache is written instead — the same code in either format, because the
//! refusal is a property of the body, not of the container.
//!
//! # Document-global consequence of writing a 3DSOLID
//!
//! The DXF version is one header value for the whole file, not a per-entity
//! choice. A `3DSOLID` payload is written inline as ACIS 7.0 SAT text only in a
//! pre-AC1027 container; R2013/R2018 carry it as a binary SAB blob in
//! `ACDSDATA`. So the presence of **any** `3DSOLID` (in model space or inside a
//! written block definition) pins the DXF output to the requested target and adds
//! `export.note.acis_sat_text_version` (R2010) or `export.note.acis_sab_dxf`
//! (R2013/R2018). That is a document-wide constraint on future entity work: a
//! feature that needs a different container cannot be added without revisiting
//! this pin. The pin is a DXF rule only — a DWG is written as AC1032 (so the
//! upstream writer emits the AcDs section) and never inherits it.
//!
//! Every outcome is recorded in the [`ExportReport`](super::report::ExportReport),
//! so the report never claims exact geometry was written when it was not — and
//! when it was, it also states that no third-party CAD reader verified it.

use acadrust::entities::acis::SabWriter;
use acadrust::entities::{
    EntityType, Line as DxfLine, PolyfaceFace, PolyfaceMesh, PolyfaceVertex, Polyline3D, Solid3D,
};
use acadrust::types::Vector3;
use acadrust::CadDocument;
use cad_db::DbEntity;
use cad_domain::{Body3d, BodyExact, BodyKind, CadResult, Mesh, Point3};

use super::accumulator::ExportAccumulator;
use super::acis::brep_to_sat;
use super::common::{common_for, v3};
use super::entity::EntityContext;
use super::report::{ExportFormat, MappingOutcome};

/// The largest polyface vertex index that fits `PolyfaceFace`'s signed `i16`
/// fields (1-based, so a 0-based index must be `< 32767`).
pub(super) const MAX_POLYFACE_INDEX: usize = 32766;

/// The generic per-vertex loss of the polyface display cache.
const POLYFACE_DISPLAY_REASON: &str =
    "mesh written as a polyface mesh; per-vertex colours and normals are dropped";

/// Why a body's exact geometry is not in the output.
///
/// This is the reason for a body whose exact B-rep is absent, or whose exact
/// B-rep is outside the ACIS writer's planar single-shell subset. It names the
/// two real causes instead of the older blanket "no encoder" claim.
pub(super) const BODY_EXACT_NOT_WRITTEN_REASON: &str =
    "this 3D body's exact geometry is not written as a 3DSOLID: either it has no exact B-Rep, or \
     its B-Rep is outside the planar single-shell subset the ACIS writer emits (see the named \
     refusal below), and the original SAT/SAB source is not retained, so the body is exported as \
     its display cache, not as a 3DSOLID";

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
    let Some(mut reasons) = push_polyface_mesh(doc, entity, mesh, ctx, acc)? else {
        return Ok(());
    };
    reasons.extend(base_reasons);
    reasons.push(POLYFACE_DISPLAY_REASON.into());
    acc.record(object, "mesh", MappingOutcome::Converted(reasons));
    Ok(())
}

/// Build and queue the polyface record for `mesh`.
///
/// Returns the `common_for` reasons on success, or `None` when the mesh is
/// outside the polyface index range (already recorded as a named drop).
fn push_polyface_mesh(
    doc: &mut CadDocument,
    entity: &DbEntity,
    mesh: &Mesh,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<Option<Vec<String>>> {
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
        return Ok(None);
    }
    let (common, reasons) = common_for(doc, entity, ctx);
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
    Ok(Some(reasons))
}

/// Write a [`Body3d`]: the exact `3DSOLID` when the ACIS writer accepts it, the
/// derived display cache otherwise (with the named reason).
pub(super) fn emit_body3d(
    doc: &mut CadDocument,
    entity: &DbEntity,
    body: &Body3d,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    // The exact payload is only ever a *solid* body's B-rep. A sheet's B-rep is
    // not a closed shell, and the ACIS writer would refuse it anyway.
    let exact = match (&body.exact, &body.kind) {
        (BodyExact::Exact(brep), BodyKind::Solid) => Some(brep.as_ref()),
        _ => None,
    };
    let mut refusal: Option<(&'static str, String)> = None;
    if let Some(brep) = exact {
        match brep_to_sat(brep, ctx.acis_version) {
            Ok(sat) => return emit_acis_solid3d(doc, entity, body, &sat, ctx, acc),
            Err(error) => refusal = Some((error.code(), error.message())),
        }
    }

    if !body.display.mesh.triangles.is_empty() {
        let mut reasons = vec![BODY_EXACT_NOT_WRITTEN_REASON.to_string()];
        if let Some((_, detail)) = &refusal {
            reasons.push(detail.clone());
        }
        if !body.display.edges.is_empty() {
            reasons.push(format!(
                "the body's {} display edge polyline(s) are not part of the polyface mesh and \
                 are not written separately",
                body.display.edges.len()
            ));
        }
        return match refusal {
            Some((code, _)) => {
                let Some(mut recorded) =
                    push_polyface_mesh(doc, entity, &body.display.mesh, ctx, acc)?
                else {
                    return Ok(());
                };
                recorded.extend(reasons);
                recorded.push(POLYFACE_DISPLAY_REASON.into());
                acc.record_converted_code(object, code, recorded);
                Ok(())
            }
            None => emit_polyface_mesh(doc, entity, &body.display.mesh, reasons, ctx, acc),
        };
    }

    let polylines: Vec<&Vec<Point3>> = body
        .display
        .edges
        .iter()
        .filter(|polyline| polyline.len() >= 2)
        .collect();
    if polylines.is_empty() {
        let mut reasons = vec![format!(
            "3D body has no display mesh or edge polylines to write; its exact ACIS \
             geometry is not written and no 3DSOLID/BODY/REGION record is fabricated \
             (display tolerance {:.6e})",
            body.display.tolerance.unwrap_or(0.0)
        )];
        if let Some((_, detail)) = &refusal {
            reasons.push(detail.clone());
        }
        return match refusal {
            // A refusal still names itself on the drop, so the "no exact record"
            // fact and the reason stay one outcome under one stable code.
            Some((code, _)) => {
                acc.record_dropped_code(object, code, reasons);
                Ok(())
            }
            None => {
                acc.record(object, "body3d", MappingOutcome::Dropped(reasons));
                Ok(())
            }
        };
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
    if let Some((_, detail)) = &refusal {
        reasons.push(detail.clone());
    }
    reasons.push(format!(
        "the edge-only body was written as its {lines} LINE and {polyline_records} POLYLINE3D \
         display edge record(s); the body entity type is not preserved"
    ));
    match refusal {
        // Every branch of an exact body keeps the named refusal as the one
        // outcome code, so both formats report the same body-level reason
        // (`acis.curved_face_not_written`, ...) whether the body was meshed,
        // written as edges, or dropped.
        Some((code, _)) => acc.record_converted_code(object, code, reasons),
        None => acc.record(object, "wire_body", MappingOutcome::Converted(reasons)),
    }
    Ok(())
}

/// Write the exact planar B-rep as a `3DSOLID`, in the combination's own ACIS
/// encoding.
///
/// * R2010 DXF (AC1024): inline SAT text via [`Solid3D::from_sat`], ACIS 7.0 only
///   (an ASM 21800 payload is refused before this point).
/// * R2013/R2018 DXF (AC1027/AC1032) and DWG (AC1032): the same [`SatDocument`]
///   converted to a binary SAB blob with [`SabWriter::write`] and attached as
///   `sab_data` (`Solid3D::from_sab`), so the DXF writer queues that blob
///   verbatim into `ACDSDATA` and the DWG writer puts it into the
///   `AcDsPrototype_1b` section.
///
/// Why the SAB is built here instead of handing the writer SAT text: the
/// writer's data-store flag comes from
/// `AcisData::contributes_sab()` (`acadrust-0.6.8/src/entities/solid3d.rs:398`),
/// which is `(is_binary && !sab_data.is_empty()) || !sat_data.is_empty()` — so
/// *both* payload shapes satisfy it. The SAB shape is chosen for the AC1027+
/// containers because its blob is decided by the bytes we hold: `queue_sab_data`
/// (`section_writer.rs:12098`) pushes `sab_for_save()` for binary data, but for
/// SAT text it parses the text a **second** time and pushes nothing on error,
/// which would orphan the solid in the file. Attaching the SAB removes that
/// silent failure mode and keeps the chosen payload version byte-exact.
fn emit_acis_solid3d(
    doc: &mut CadDocument,
    entity: &DbEntity,
    body: &Body3d,
    sat: &acadrust::entities::acis::SatDocument,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    let (common, mut reasons) = common_for(doc, entity, ctx);
    let inline_sat = ctx.format == ExportFormat::DxfText && !ctx.dxf_target.stores_sab();
    let mut solid = if inline_sat {
        Solid3D::from_sat(&sat.to_sat_string())
    } else {
        Solid3D::from_sab(SabWriter::write(sat))
    };
    solid.common = common;
    acc.push(EntityType::Solid3D(solid), ctx)?;
    let payload = if inline_sat {
        format!(
            "exact planar B-rep written as ACIS SAT text in a 3DSOLID (SAB/SAT header version {}, \
             ACIS {}) in the {container} DXF (single closed shell, plane surfaces, straight-line \
             boundary curves)",
            ctx.acis_version.number(),
            ctx.acis_version.label(),
            container = ctx.dxf_target.dxf_version().as_str(),
        )
    } else if ctx.format == ExportFormat::DxfText {
        format!(
            "exact planar B-rep written as a 3DSOLID whose ACIS {} payload is a binary SAB blob \
             (header version {}) in the {container} DXF ACDSDATA section (single closed shell, \
             plane surfaces, straight-line boundary curves)",
            ctx.acis_version.label(),
            ctx.acis_version.number(),
            container = ctx.dxf_target.dxf_version().as_str(),
        )
    } else {
        format!(
            "exact planar B-rep written as a 3DSOLID whose ACIS {} payload is a binary SAB blob \
             (header version {}) in the DWG AC1032 AcDs (AcDsPrototype_1b) section (single closed \
             shell, plane surfaces, straight-line boundary curves)",
            ctx.acis_version.label(),
            ctx.acis_version.number(),
        )
    };
    reasons.push(payload);
    reasons.push(
        "the payload is rebuilt from the retained exact B-Rep, not the original SAT/SAB byte \
         stream: source records, history and ACIS attributes are not retained"
            .into(),
    );
    reasons.push(
        "no third-party CAD reader was used to verify the payload: only this exporter's own ACIS \
         reader (acadrust SAT/SAB) round-trips it"
            .into(),
    );
    let domains = body.display.mesh.triangles.len() + body.display.edges.len();
    if domains > 0 {
        reasons.push(format!(
            "the derived display cache ({} triangle(s), {} edge polyline(s)) is not written: the \
             3DSOLID carries the exact ACIS topology instead",
            body.display.mesh.triangles.len(),
            body.display.edges.len()
        ));
    }
    acc.record(object, "acis_solid", MappingOutcome::Converted(reasons));
    Ok(())
}
