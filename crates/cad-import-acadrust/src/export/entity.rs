//! Reverse mapping: domain [`SemanticGeometry`] → acadrust entities.
//!
//! This inverts `crate::entity`'s import mapping. It is **exhaustive with no
//! catch-all arm**: adding a new `SemanticGeometry` variant must be handled here
//! (a compile error), never silently dropped. Every approximation is recorded in
//! the [`ExportReport`](super::report::ExportReport).
//!
//! "Exact" means **faithful to the domain database**, not faithful to the source
//! DWG: the database has already dropped information (per-entity `ByLayer`
//! transparency, spline closed/periodic flags), and a `Spline` is therefore
//! reported as `Converted`, never `Exact`. Colour, lineweight and linetype
//! **provenance** is not among the losses: a symbolic `ByLayer`/`ByBlock` value
//! is written back symbolically (see [`common_for`]). Block identity is no
//! longer a loss either: a representable reference is written as a real
//! `INSERT` record naming a written `BLOCK_RECORD` (see [`super::insert`]).

use acadrust::entities::{
    Arc as DxfArc, Circle as DxfCircle, Ellipse as DxfEllipse, EntityType, Line as DxfLine,
    LwPolyline, LwVertex, MText, Point as DxfPoint, Polyline3D, Solid as DxfSolid,
    Spline as DxfSpline, Text as DxfText,
};
use acadrust::types::{Vector2, Vector3};
use acadrust::CadDocument;
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::*;

pub(crate) use super::accumulator::ExportAccumulator;
use super::body::{emit_body3d, emit_polyface_mesh};
use super::common::{
    common_for, map_h_align, map_v_align, mtext_attachment, ocs_from_wcs, v3, with_reasons,
};
use super::report::{ExportLimits, MappingOutcome};
use super::solid_record::{face_class, face_record};
use super::tables::ensure_text_style;

/// Source entity classes whose ACIS payload an *older* import tessellated into a
/// plain [`SemanticGeometry::Mesh`]. Current imports produce a
/// [`SemanticGeometry::Body3d`] instead, so this list only keeps legacy database
/// content honest: its original SAT/SAB bytes are gone, so an export writes a
/// polyface and must say so rather than imply a 3DSOLID round trip.
const ACIS_TESSELLATED_TYPE_KEYS: [&str; 4] =
    ["AcDb3dSolid", "AcDbRegion", "AcDbBody", "AcDbSurface"];

/// Immutable inputs shared by the whole export.
pub(crate) struct EntityContext<'a> {
    pub database: &'a DrawingDatabase,
    pub limits: ExportLimits,
}

/// Type keys whose source entity is an infinite construct line.
///
/// The importer stores both as a finite [`SemanticGeometry::Line`] clipped to
/// the drawing extents (`support::XLINE_SEGMENT_REASON`), so they reach the
/// `Line` export arm with their original class name intact.
const FINITE_CONSTRUCT_LINE_TYPE_KEYS: [&str; 2] = ["AcDbXline", "AcDbRay"];

/// Outcome for a `Line` whose stored source type key is a construct line.
///
/// Writing an XLINE/RAY as a DXF `LINE` is a real approximation: the entity type
/// changes and the line is no longer infinite, so the round-trip is reported as
/// `Converted` with a named reason rather than `Exact`. The written DXF stays a
/// faithful `LINE` of the clipped segment — no fake XLINE payload is emitted.
/// Deliberately limited to these two classes: no other source entity is expected
/// to reach the `Line` arm through a class name that lies about the geometry.
fn finite_construct_line_outcome(type_key: &str) -> MappingOutcome {
    if !FINITE_CONSTRUCT_LINE_TYPE_KEYS
        .iter()
        .any(|key| key.eq_ignore_ascii_case(type_key))
    {
        return MappingOutcome::Exact;
    }
    MappingOutcome::Converted(vec![format!(
        "{type_key} was imported as a finite LINE clipped to the drawing extents; \
         the DXF entity type changes and the line is no longer infinite"
    )])
}

/// Map one model-space entity's geometry, expanding INSERTs and flattening
/// compounds. `transform` is the accumulated block placement (identity at the
/// model root).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit(
    doc: &mut CadDocument,
    entity: &DbEntity,
    transform: &Transform3,
    depth: usize,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    // A clip is only ever stored on a block reference (the `XCLIP` command and
    // the importer both enforce that). The Insert and Compound arms report their
    // own clip loss on the paths that do not write the reference; anything else
    // carrying one is a database inconsistency and must not be dropped silently.
    if !matches!(
        entity.geometry,
        SemanticGeometry::Insert { .. } | SemanticGeometry::Compound(_)
    ) {
        acc.record_clip_dropped(
            entity,
            ctx,
            "only a block reference can own an XCLIP boundary, and this entity is not one",
        );
    }
    match &entity.geometry {
        SemanticGeometry::Insert {
            block,
            transform: insert_transform,
        } => {
            let object = Some(entity.object.id);
            if stack.contains(block) {
                acc.record(
                    object,
                    "insert_cycle",
                    MappingOutcome::Dropped(vec![format!(
                        "block reference cycle at {block:?}; not expanded"
                    )]),
                );
                acc.record_clip_dropped(
                    entity,
                    ctx,
                    "the reference is not written (block-reference cycle)",
                );
                return Ok(());
            }
            // An xref-owned block's members are the resolved external drawing,
            // not host geometry. v1 DXF export cannot write an xref attachment
            // record or its path, so expanding them would silently flatten the
            // attachment into ordinary model-space entities with no way back.
            // Refuse the whole INSERT (never emit its members, never write a
            // fake empty block) and mark the block as handled so it is not also
            // reported as an unrelated unreferenced definition.
            if let Some(reference) = ctx.database.external_reference_for_block(*block) {
                acc.expanded_blocks.insert(*block);
                acc.record_dropped_code(
                    object,
                    "export.xref_content_dropped",
                    vec![format!(
                        "external reference '{}' (raw path '{}', status '{}') is not written: \
                         the v1 DXF exporter emits no xref attachment record or path, so its \
                         content cannot be round-tripped and is not flattened into ordinary \
                         geometry",
                        reference.name,
                        reference.raw_path,
                        reference.status.as_str()
                    )],
                );
                acc.record_clip_dropped(
                    entity,
                    ctx,
                    "the reference is not written (an external reference attachment has no \
                     output record)",
                );
                return Ok(());
            }
            // Real `INSERT` + `BLOCK_RECORD` when the reference can be one record.
            let placement = transform.matrix_mul(insert_transform);
            let reason =
                match super::insert::emit_insert(doc, entity, *block, &placement, ctx, acc, stack)?
                {
                    super::insert::InsertEmission::Written => return Ok(()),
                    super::insert::InsertEmission::Explode(reason) => reason,
                };
            // Declared fallback: this reference genuinely cannot be one `INSERT`,
            // so its members are expanded in place and the reason is recorded.
            if depth + 1 > ctx.limits.max_explode_depth {
                return Err(CadError::InvalidInput(format!(
                    "insert explode depth exceeds the {} limit",
                    ctx.limits.max_explode_depth
                )));
            }
            let Some(definition) = ctx.database.block(*block) else {
                acc.record(
                    object,
                    "insert",
                    MappingOutcome::Dropped(vec![
                        "insert references a block definition that is not in the database".into(),
                    ]),
                );
                acc.record_clip_dropped(
                    entity,
                    ctx,
                    "the reference is not written (its block definition is not in the database)",
                );
                return Ok(());
            };
            // Dynamic-block visibility: when a descriptor exists, export only
            // the members the active state makes visible.
            let (members, visibility_reason) = match ctx.database.block_visible_entities(*block) {
                Some(visible) => (
                    visible,
                    Some(
                        "dynamic-block visibility applied: only the active state's members \
                             are exported"
                            .to_string(),
                    ),
                ),
                None => (definition.entities.clone(), None),
            };
            acc.expanded_blocks.insert(*block);
            stack.push(*block);
            for member in &members {
                match ctx.database.entity(*member) {
                    Some(child) => emit(doc, child, &placement, depth + 1, ctx, acc, stack)?,
                    // A member id that no longer resolves is explicit, never a
                    // silent omission (mirrors the missing-definition arm). The
                    // database builder and validated change path keep block
                    // membership consistent, so this is a defensive branch.
                    None => acc.record(
                        object,
                        "insert_member",
                        MappingOutcome::Dropped(vec![format!(
                            "block member {member:?} is not in the database; skipped"
                        )]),
                    ),
                }
            }
            stack.pop();
            acc.record_clip_dropped(
                entity,
                ctx,
                &format!("the reference is exported exploded ({reason})"),
            );
            let mut reasons = vec![format!(
                "block reference exploded: {reason}; block identity, name and base point are lost"
            )];
            if let Some(reason) = visibility_reason {
                reasons.push(reason);
            }
            acc.record(object, "insert", MappingOutcome::Converted(reasons));
            Ok(())
        }
        SemanticGeometry::Compound(children) => {
            let object = Some(entity.object.id);
            // An array (`MINSERT`) reference reaches this arm as one compound of
            // one `Insert` per cell, so each cell is written as its own `INSERT`
            // record by the arm above. The array header (row/column count and
            // spacing) is not reconstructed, which the outcome states.
            let all_cells = !children.is_empty()
                && children
                    .iter()
                    .all(|child| matches!(child, SemanticGeometry::Insert { .. }));
            let before = acc.written_inserts;
            flatten_children(doc, entity, children, transform, depth, ctx, acc, stack)?;
            let written_cells = acc.written_inserts - before;
            let reason = if all_cells && written_cells == children.len() {
                format!(
                    "{} array cell(s) written as independent INSERT records; the MINSERT \
                     row/column count and spacing are not reconstructed",
                    children.len()
                )
            } else {
                "compound flattened into separate output entities".to_string()
            };
            acc.record(object, "compound", MappingOutcome::Converted(vec![reason]));
            Ok(())
        }
        // A first-class MULTILEADER is written as a real `MULTILEADER` entity
        // plus its `MLEADERSTYLE` record and `CLASSES` entries (ADR 0019
        // decision E). The derived display is never written as the entity: it
        // is our rendering, and AutoCAD recomputes it.
        SemanticGeometry::MultiLeader(multileader) => super::multileader::emit_multileader(
            doc,
            entity,
            multileader,
            transform,
            depth,
            ctx,
            acc,
            stack,
        ),
        // A first-class TABLE is written as a real `ACAD_TABLE` entity from the
        // domain grid; the derived display is never written as the entity. A
        // block-embedded table is placed first with the accumulated INSERT
        // transform, exactly like every other leaf.
        SemanticGeometry::Table(table) => {
            super::table::emit_table(doc, entity, table, transform, ctx, acc)
        }
        leaf => {
            let owned;
            let placed: &SemanticGeometry = if transform == &Transform3::identity() {
                leaf
            } else {
                match cad_db::transform_geometry(leaf, transform) {
                    Ok(geometry) => {
                        owned = geometry;
                        &owned
                    }
                    Err(error) => {
                        acc.record(
                            Some(entity.object.id),
                            "transform",
                            MappingOutcome::Dropped(vec![format!(
                                "cannot place {} under an insert transform: {error}",
                                kind_name(leaf)
                            )]),
                        );
                        return Ok(());
                    }
                }
            };
            emit_leaf(doc, entity, placed, ctx, acc)
        }
    }
}

/// Emit each child of a flattened parent as its own entity, with the parent's
/// identity and the accumulated placement.
///
/// Shared by the `Compound` arm and a MULTILEADER that cannot be written as an
/// entity (a scaling/mirroring insert placement, ADR 0019 C.5), so a child
/// records **its own** outcome — including its own named refusal, such as a
/// `Text` child under a mirror.
#[allow(clippy::too_many_arguments)]
pub(super) fn flatten_children(
    doc: &mut CadDocument,
    entity: &DbEntity,
    children: &[SemanticGeometry],
    transform: &Transform3,
    depth: usize,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    for child in children {
        let child_entity = DbEntity {
            geometry: child.clone(),
            ..entity.clone()
        };
        emit(doc, &child_entity, transform, depth, ctx, acc, stack)?;
    }
    Ok(())
}

fn emit_leaf(
    doc: &mut CadDocument,
    entity: &DbEntity,
    geometry: &SemanticGeometry,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let (common, reasons) = common_for(doc, entity, ctx);
            let mut line = DxfLine::new();
            line.start = v3(*start);
            line.end = v3(*end);
            line.normal = Vector3::UNIT_Z;
            line.common = common;
            acc.push(EntityType::Line(line), ctx)?;
            // The database has no infinite construct-line geometry, so an
            // imported XLINE/RAY reaches this arm as a finite `Line`. The writer
            // keeps writing a real LINE (never a fake XLINE payload), but the
            // source entity is *not* recovered: the round-trip is a conversion,
            // not an exact mapping.
            acc.record(
                object,
                "line",
                with_reasons(
                    finite_construct_line_outcome(&entity.object.type_key),
                    reasons,
                ),
            );
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width,
        } => emit_polyline(doc, entity, points, bulges, *closed, *width, ctx, acc)?,
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (common, reasons) = common_for(doc, entity, ctx);
            let mut circle = DxfCircle::new();
            circle.center = ocs_from_wcs(*normal, *center);
            circle.normal = v3(*normal);
            circle.radius = *radius;
            circle.common = common;
            acc.push(EntityType::Circle(circle), ctx)?;
            acc.record(
                object,
                "circle",
                with_reasons(MappingOutcome::Exact, reasons),
            );
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (common, reasons) = common_for(doc, entity, ctx);
            let mut arc = DxfArc::new();
            arc.center = ocs_from_wcs(*normal, *center);
            arc.normal = v3(*normal);
            arc.radius = *radius;
            // DXF ARC group 50/51 are radians here; acadrust's writer converts
            // them to degrees (`section_writer.rs` writes `.to_degrees()`).
            //
            // A DXF ARC is an undirected point set swept CCW from 50 to 51. A
            // negative (clockwise) sweep covers the same points as the CCW arc
            // between the two endpoints, so emit the angles in the opposite order
            // rather than mirroring the arc into its complement.
            let tau = std::f64::consts::TAU;
            if *sweep < 0.0 {
                arc.start_angle = (*start + *sweep).rem_euclid(tau);
                arc.end_angle = start.rem_euclid(tau);
            } else {
                arc.start_angle = *start;
                arc.end_angle = (*start + *sweep).rem_euclid(tau);
            }
            arc.common = common;
            acc.push(EntityType::Arc(arc), ctx)?;
            acc.record(object, "arc", with_reasons(MappingOutcome::Exact, reasons));
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let (common, reasons) = common_for(doc, entity, ctx);
            let mut ellipse = DxfEllipse::new();
            ellipse.center = v3(*center);
            ellipse.normal = v3(*normal);
            ellipse.major_axis = v3(*major_axis);
            ellipse.minor_axis_ratio = *ratio;
            ellipse.start_parameter = *start;
            ellipse.end_parameter = *start + *sweep;
            ellipse.common = common;
            acc.push(EntityType::Ellipse(ellipse), ctx)?;
            acc.record(
                object,
                "ellipse",
                with_reasons(MappingOutcome::Exact, reasons),
            );
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
            closed,
        } => {
            let (common, mut reasons) = common_for(doc, entity, ctx);
            let mut spline = DxfSpline::new();
            spline.degree = *degree as i32;
            spline.knots = knots.clone();
            spline.control_points = control_points.iter().map(|p| v3(*p)).collect();
            spline.weights = weights.clone();
            // Round-trip the one flag the domain model carries (DXF group 70
            // bit 1). The control polygon is written exactly as stored: a closed
            // spline is *not* closed by appending a duplicate first point.
            spline.flags.closed = *closed;
            spline.normal = Vector3::UNIT_Z;
            spline.common = common;
            acc.push(EntityType::Spline(spline), ctx)?;
            // `periodic`/`planar`/`linear`/`rational` stay derived from the data
            // (the writer recomputes them from weights and control points); only
            // `closed` is carried as entity metadata.
            reasons.push(
                "SPLINE periodic/planar/linear flags are not carried in the domain model; written \
                 as false"
                    .into(),
            );
            acc.record(object, "spline", MappingOutcome::Converted(reasons));
        }
        SemanticGeometry::Point(position) => {
            let (common, reasons) = common_for(doc, entity, ctx);
            let mut point = DxfPoint::new();
            point.location = v3(*position);
            point.normal = Vector3::UNIT_Z;
            point.common = common;
            acc.push(EntityType::Point(point), ctx)?;
            acc.record(
                object,
                "point",
                with_reasons(MappingOutcome::Exact, reasons),
            );
        }
        SemanticGeometry::Mesh(mesh) => {
            // A mesh with a SOLID/TRACE source class (created by the domain
            // `SOLID` command, or imported from DXF) whose 3–4 coplanar corners
            // and implied triangle fan are exactly a filled-face record is
            // written as that record — SOLID for SOLID, TRACE for TRACE — instead
            // of a polyface mesh. Anything else about that mesh (more corners,
            // per-vertex colours, a different triangulation, a tilted plane)
            // keeps the polyface path below and is declared there with the class
            // it wanted.
            let mut face_refusal: Option<String> = None;
            if let Some(class) = face_class(&entity.object.type_key) {
                match face_record(mesh, class) {
                    Ok(record) => {
                        let (common, reasons) = common_for(doc, entity, ctx);
                        let mut solid = DxfSolid::new(
                            v3(record.corners[0]),
                            v3(record.corners[1]),
                            v3(record.corners[2]),
                            v3(record.corners[3]),
                        );
                        // A filled-face record is planar in its own OCS; both the
                        // created and the imported face are in the +Z plane, so the
                        // extrusion is identity.
                        solid.normal = Vector3::UNIT_Z;
                        // acadrust keeps the SOLID/TRACE class in this one flag and
                        // the writer picks the record type from it.
                        solid.is_trace = record.class.is_trace();
                        solid.common = common;
                        acc.push(EntityType::Solid(solid), ctx)?;
                        acc.record(
                            object,
                            "filled_face",
                            with_reasons(MappingOutcome::Exact, reasons),
                        );
                        return Ok(());
                    }
                    Err(reason) => face_refusal = Some(reason),
                }
            }
            let mut base_reasons = Vec::new();
            if let Some(reason) = face_refusal {
                base_reasons.push(format!(
                    "this source could not be written as a filled-face record ({reason}); its \
                     class is not preserved and it is written as a polyface mesh"
                ));
            }
            if ACIS_TESSELLATED_TYPE_KEYS.contains(&entity.object.type_key.as_str()) {
                base_reasons.push(
                    "this mesh was tessellated from ACIS at import; the original SAT/SAB source \
                     is not retained, so it is exported as a polyface mesh, not as a 3DSOLID"
                        .into(),
                );
            }
            emit_polyface_mesh(doc, entity, mesh, base_reasons, ctx, acc)?;
        }
        // A 3D body keeps its exact B-rep in the database, but this exporter has
        // no ACIS encoder: only the derived display cache is written (a polyface
        // mesh for facets, LINE/POLYLINE3D for edge-only bodies). Never a
        // fabricated `3DSOLID`/`BODY`/`REGION` record; the loss is recorded.
        SemanticGeometry::Body3d(body) => emit_body3d(doc, entity, body, ctx, acc)?,
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
            width_factor,
            line_spacing,
        } => {
            let style_name = ctx
                .database
                .style(*style)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let (style_name, mut reasons) = ensure_text_style(doc, &style_name, font.as_deref());
            let (common, extra) = common_for(doc, entity, ctx);
            reasons.extend(extra);
            // Branch precedence: the stored entity type key wins, so an entity
            // imported as MTEXT (`type_key == "AcDbMText"`) re-exports as MTEXT
            // even when its value has no paragraph break. The content
            // heuristics after it are only the compatibility path for text
            // created in the domain database, which has no MTEXT type key of
            // its own (`class_name()` maps every `Text` to `AcDbText`) and
            // stores multi-line content as a literal newline; the `\P`/`\p`
            // checks also catch raw DXF paragraph marks where the type key is
            // `AcDbText`. Deciding by content alone would silently downgrade an
            // imported MTEXT to TEXT, emitting its `\P` as visible text and
            // dropping the wrap boundary.
            let mtext_by_type = entity.object.type_key == "AcDbMText";
            let is_mtext = mtext_by_type
                || text.contains("\\P")
                || text.contains("\\p")
                || text.contains('\n');
            if is_mtext {
                if *width_factor != 1.0 {
                    reasons.push(
                        "MTEXT has no width-factor group (DXF 41 is its reference-rectangle \
                         width here); the horizontal text stretch is not exported"
                            .into(),
                    );
                }
                let mut mtext = MText::new();
                // Verbatim: DXF MTEXT wants the raw `\P` paragraph mark, so the
                // value is never rewritten to newlines here.
                mtext.value = text.clone();
                mtext.insertion_point = v3(*position);
                mtext.height = *height;
                mtext.rotation = *rotation;
                mtext.style = style_name;
                mtext.normal = Vector3::UNIT_Z;
                // The wrap boundary is MTEXT's reference-rectangle width
                // (group 41); a single-line TEXT has no equivalent, which the
                // other branch reports. `MText::new()` seeds a placeholder
                // 10.0, so an absent wrap boundary must be written as the DXF
                // "no width limit" 0.0 rather than that fabricated value
                // (acadrust's DXF writer always emits group 41).
                mtext.rectangle_width = wrap_width.unwrap_or(0.0);
                let (attachment, align_reason) = mtext_attachment(*h_align, *v_align);
                mtext.attachment_point = attachment;
                if let Some(reason) = align_reason {
                    reasons.push(reason);
                }
                // DXF groups 73 (style) and 44 (factor). The writer always emits
                // both, so the neutral stored value writes exactly the bytes the
                // pre-line-spacing exporter wrote (`1` / `1.0`, the DXF defaults).
                mtext.line_spacing_style = match line_spacing.style {
                    TextLineSpacingStyle::AtLeast => acadrust::entities::LineSpacingStyle::AtLeast,
                    TextLineSpacingStyle::Exactly => acadrust::entities::LineSpacingStyle::Exactly,
                };
                mtext.line_spacing_factor = line_spacing.effective_factor();
                mtext.common = common;
                acc.push(EntityType::MText(Box::new(mtext)), ctx)?;
                // An entity that already was MTEXT and whose value needs no
                // rewrite is faithful to the database (the unmodelled MTEXT
                // fields were reported at import, not lost here), so it is
                // `Exact` unless the attachment folding above raised a reason.
                // Every other MTEXT here is a TEXT → MTEXT conversion.
                let base = if mtext_by_type && !text.contains('\n') && !text.contains('\r') {
                    MappingOutcome::Exact
                } else {
                    reasons.push("multi-line text written as MTEXT".into());
                    MappingOutcome::Converted(Vec::new())
                };
                acc.record(object, "text_mtext", with_reasons(base, reasons));
            } else {
                if wrap_width.is_some() {
                    reasons.push(
                        "single-line TEXT has no wrap boundary; the width is not exported".into(),
                    );
                }
                if !line_spacing.is_default() {
                    reasons.push(
                        "single-line TEXT has no line-spacing groups (DXF 44/73); the stored \
                         factor and style are not exported"
                            .into(),
                    );
                }
                let mut dxf_text = DxfText::new();
                dxf_text.value = text.clone();
                dxf_text.insertion_point = v3(*position);
                dxf_text.height = *height;
                dxf_text.rotation = *rotation;
                // DXF TEXT group 41 is the horizontal width scale; the stored
                // factor is written verbatim (the neutral 1.0 round-trips).
                dxf_text.width_factor = *width_factor;
                dxf_text.style = style_name;
                dxf_text.horizontal_alignment = map_h_align(*h_align);
                dxf_text.vertical_alignment = map_v_align(*v_align);
                // Inverse of the import rule: a non-default alignment is anchored
                // at the alignment point (group 11), so the export must write it.
                if (*h_align, *v_align) != (TextAlignH::Left, TextAlignV::Baseline) {
                    dxf_text.alignment_point = Some(v3(*position));
                }
                dxf_text.normal = Vector3::UNIT_Z;
                dxf_text.common = common;
                acc.push(EntityType::Text(dxf_text), ctx)?;
                acc.record(object, "text", with_reasons(MappingOutcome::Exact, reasons));
            }
        }
        SemanticGeometry::Shape { .. } => acc.record(
            object,
            "shape",
            MappingOutcome::Dropped(vec![
                "SHX shape glyph has no DXF representation without its shape file".into(),
            ]),
        ),
        SemanticGeometry::Image { adjust, .. } => acc.record(
            object,
            "image",
            MappingOutcome::Dropped(vec![
                "raster image is not exported (external resource), so its display transfer \
                 is not written either"
                    .into(),
                format!(
                    "the image display transfer (brightness {}, contrast {}, fade {}, \
                     monochrome {}) is not exported with the dropped raster image",
                    adjust.brightness, adjust.contrast, adjust.fade, adjust.monochrome
                ),
            ]),
        ),
        SemanticGeometry::Mask { .. } => acc.record(
            object,
            "mask",
            MappingOutcome::Dropped(vec![
                "wipeout/mask has no DXF representation in this export".into(),
            ]),
        ),
        SemanticGeometry::Opaque { type_key, .. } => acc.record(
            object,
            "opaque",
            MappingOutcome::Dropped(vec![format!(
                "opaque/proxy geometry '{type_key}' has no DXF representation"
            )]),
        ),
        SemanticGeometry::Dimension(dimension) => {
            super::dimension::emit_dimension(doc, entity, dimension, ctx, acc)?;
        }
        SemanticGeometry::Hatch(hatch) => {
            super::hatch::emit_hatch(doc, entity, hatch, ctx, acc)?;
        }
        // A MULTILEADER is written by the dedicated entity writer before this
        // point; reaching here is a programming error, not a silent drop.
        SemanticGeometry::MultiLeader(_) => {
            acc.record(
                object,
                "internal",
                MappingOutcome::Dropped(vec![
                    "multileader reached the leaf mapper; it is written by the dedicated \
                     MULTILEADER writer"
                        .into(),
                ]),
            );
        }
        // Insert and Compound are handled before this point; a leaf here would be
        // a programming error, and must not be silently dropped.
        SemanticGeometry::Insert { .. } | SemanticGeometry::Compound(_) => {
            acc.record(
                object,
                "internal",
                MappingOutcome::Dropped(vec![
                    "insert/compound reached the leaf mapper; not emitted".into(),
                ]),
            );
        }
        // A TABLE is written by the dedicated entity writer before this point.
        SemanticGeometry::Table(_) => {
            acc.record(
                object,
                "internal",
                MappingOutcome::Dropped(vec![
                    "table reached the leaf mapper; it is written by the dedicated TABLE writer"
                        .into(),
                ]),
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_polyline(
    doc: &mut CadDocument,
    entity: &DbEntity,
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    width: f64,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    let z0 = points.first().map(|p| p.z).unwrap_or(0.0);
    let z_constant = points.iter().all(|p| (p.z - z0).abs() <= 1e-9);
    let has_bulge = bulges.iter().any(|b| b.abs() > 1e-12);
    let (common, reasons) = common_for(doc, entity, ctx);
    if z_constant {
        // Planar, z-constant: an exact LWPOLYLINE (bulges preserved).
        let mut lw = LwPolyline::new();
        lw.elevation = z0;
        lw.normal = Vector3::UNIT_Z;
        lw.is_closed = closed;
        // DXF group 43 (constant width). `0` is the writer's own "omit" state,
        // so a hairline polyline writes exactly the pre-width file. Per-vertex
        // 40/41 start/end widths stay 0: the domain models no taper.
        lw.constant_width = width;
        lw.vertices = points
            .iter()
            .enumerate()
            .map(|(index, p)| LwVertex {
                location: Vector2::new(p.x, p.y),
                bulge: bulges.get(index).copied().unwrap_or(0.0),
                start_width: 0.0,
                end_width: 0.0,
                vertex_id: 0,
            })
            .collect();
        lw.common = common;
        acc.push(EntityType::LwPolyline(lw), ctx)?;
        acc.record(
            object,
            "polyline",
            with_reasons(MappingOutcome::Exact, reasons),
        );
    } else {
        let mut poly = Polyline3D::from_points(points.iter().map(|p| v3(*p)).collect());
        poly.flags.closed = closed;
        poly.common = common;
        acc.push(EntityType::Polyline3D(poly), ctx)?;
        let mut reasons = reasons;
        if has_bulge {
            reasons.push("non-planar polyline bulges linearized to straight segments".into());
        }
        if width > 0.0 {
            // A POLYLINE3D has no constant-width group; keep the width in the
            // domain and report the export gap instead of dropping it silently.
            reasons
                .push("uniform polyline width is not exported for a non-planar POLYLINE3D".into());
        }
        if has_bulge {
            acc.record(object, "polyline_bulge", MappingOutcome::Converted(reasons));
        } else if width > 0.0 {
            // The width is the only lossy part: POLYLINE3D has no width group.
            acc.record(object, "polyline", MappingOutcome::Converted(reasons));
        } else {
            acc.record(
                object,
                "polyline",
                with_reasons(MappingOutcome::Exact, reasons),
            );
        }
    }
    Ok(())
}

/// The `SemanticGeometry` variant name, for diagnostics.
pub(crate) fn kind_name(geometry: &SemanticGeometry) -> &'static str {
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
    }
}
