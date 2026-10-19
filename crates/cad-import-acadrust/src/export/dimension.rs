//! DIMENSION export: a real DXF DIMENSION with a generated `*D...` content
//! block, group 42 measurement and a DIMSTYLE record (ADR 0017 A.5).
//!
//! Submodule of [`crate::export`]. AutoCAD requires a DIMENSION's graphical
//! representation (the content block); the `ExportReport` records every
//! conversion and the deliberate drops (groups 51/52/71/72, group 70 bit 32).

use acadrust::entities::{
    Dimension, DimensionAligned, DimensionAngular2Ln, DimensionAngular3Pt, DimensionBase,
    DimensionDiameter, DimensionLinear, DimensionOrdinate, DimensionRadius, EntityType,
};
use acadrust::tables::{BlockRecord, DimStyle};
use acadrust::types::Vector3;
use acadrust::CadDocument;
use cad_db::DbEntity;
use cad_domain::*;

use super::entity::{common_for, emit, v3, EntityContext, ExportAccumulator};
use super::report::MappingOutcome;
use super::tables::ensure_text_style;

/// Write a DIMENSION with a generated `*D...` content block, group 42
/// measurement and a DIMSTYLE record (ADR 0017 A.5).
pub(super) fn emit_dimension(
    doc: &mut CadDocument,
    entity: &DbEntity,
    dimension: &DimensionGeometry,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    let definition = dimension.definition();

    // A DIMENSION without a content block is not opened by AutoCAD, so always
    // generate one whose members are the display children.
    let block_name = {
        let counter = acc.dim_block_counter;
        acc.dim_block_counter += 1;
        format!("*D{counter}")
    };
    let block_handle = doc.allocate_handle();
    let block_entity_handle = doc.allocate_handle();
    let block_end_handle = doc.allocate_handle();
    let mut record = BlockRecord::new(block_name.clone());
    record.handle = block_handle;
    record.block_entity_handle = block_entity_handle;
    record.block_end_handle = block_end_handle;
    record.base_point = Vector3::ZERO;
    doc.block_records.add_or_replace(record);

    // Content-block members: the display children, already in the dimension's
    // own (placed) coordinates. Their records are suppressed because the
    // dimension is a single source geometry with a single outcome.
    let previous_suppress = acc.suppress_records;
    acc.suppress_records = true;
    let mut member_result: CadResult<()> = Ok(());
    for member in dimension.display() {
        let child_entity = DbEntity {
            geometry: member.clone(),
            ..entity.clone()
        };
        let start = acc.entities.len();
        let mut stack = Vec::new();
        if let Err(error) = emit(
            doc,
            &child_entity,
            &Transform3::identity(),
            0,
            ctx,
            acc,
            &mut stack,
        ) {
            member_result = Err(error);
            break;
        }
        for written in &mut acc.entities[start..] {
            written.common_mut().owner_handle = block_handle;
        }
    }
    acc.suppress_records = previous_suppress;
    member_result?;

    // Write the DIMSTYLE record so group 3 does not dangle; ensure the text
    // style it names also exists so group 340 resolves.
    let style_name = dimension.style_name();
    let (_, text_reasons) = ensure_text_style(
        doc,
        &dimension.style().text_style_name,
        dimension.style().text_font.as_deref(),
    );
    // Write (or replace) the DIMSTYLE record with the resolved snapshot, so
    // group 3 always resolves to the values the entity actually used rather
    // than to a stale table entry.
    doc.dim_styles
        .add_or_replace(dimstyle_entry(style_name, dimension.style()));
    let mut style_reason = format!("DIMSTYLE record '{style_name}' written so group 3 resolves");
    if !text_reasons.is_empty() {
        style_reason.push_str(&format!(" ({})", text_reasons.join("; ")));
    }
    let style_reason = Some(style_reason);

    let (common, reasons) = common_for(doc, entity, ctx);
    // `text`/`user_text` mirror `DimensionBase::set_text_override`: the group-1
    // override is stored in both.
    let text_override = dimension.text_override().map(str::to_string);
    let base = DimensionBase {
        common,
        text_middle_point: v3(definition.text_middle),
        text_rotation: definition.text_rotation,
        style_name: style_name.to_string(),
        block_name: block_name.clone(),
        actual_measurement: dimension.measurement(),
        normal: Vector3::UNIT_Z,
        text: text_override.clone().unwrap_or_default(),
        user_text: text_override,
        ..DimensionBase::default()
    };

    let dxf = build_dimension(definition, base)?;
    acc.push(EntityType::Dimension(dxf), ctx)?;

    let mut conversion = vec![
        format!(
            "content block '{block_name}' generated (AutoCAD requires a DIMENSION's graphical \
             representation)"
        ),
        // The writer recomputes group 42 for Ordinate as the datum-relative
        // projection; our measurement is reconciled to the same value (review
        // B2), so the written group 42 and the drawn text agree. For every other
        // subtype the writer writes our measurement verbatim.
        match definition.kind {
            DimensionKind::Ordinate { .. } => "group 42 written; the writer recomputes it for \
                 Ordinate as the datum-relative projection, which equals our measurement"
                .to_string(),
            _ => "group 42 measurement written explicitly".to_string(),
        },
        "non-associative: no ACAD_DIMASSOC is written".to_string(),
        "group 51/52 (horizontal direction / oblique angle) dropped (not modelled)".to_string(),
        "group 71/72 (text attachment / line spacing) written as the fixed writer defaults"
            .to_string(),
        "group 70 bit 32 (block referenced only by this dimension) is not written (acadrust \
         writer constraint)"
            .to_string(),
        // N2: the 0x80 bit (text positioned at a user-defined location) is never
        // written because we never user-position text; an imported dimension that
        // had it set loses it.
        "group 70 bit 128 (0x80, text user-positioned) is not modelled and never written; an \
         imported dimension that had it set loses it"
            .to_string(),
        "DIMSCALE is not preserved; the resolved values are written with DIMSCALE=1".to_string(),
    ];
    // N1: the export forces the world-Z normal; a dimension on a non-+Z plane
    // loses group 210 rather than being silently re-oriented.
    if (definition.normal.z - 1.0).abs() > 1e-9
        || definition.normal.x.abs() > 1e-9
        || definition.normal.y.abs() > 1e-9
    {
        conversion.push(
            "group 210 (extrusion normal) dropped: the export writes the world-Z normal, so a \
             dimension on a non-+Z plane loses its orientation"
                .to_string(),
        );
    }
    conversion.extend(reasons);
    if let Some(reason) = style_reason {
        conversion.push(reason);
    }
    acc.record(object, "dimension", MappingOutcome::Converted(conversion));
    Ok(())
}

/// Build the acadrust dimension subtype from a domain definition and base.
fn build_dimension(definition: &DimensionDefinition, base: DimensionBase) -> CadResult<Dimension> {
    Ok(match (definition.kind, &definition.points) {
        (
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => Dimension::Aligned(DimensionAligned {
            base,
            first_point: v3(*first),
            second_point: v3(*second),
            definition_point: v3(*dimension_line),
            ..DimensionAligned::default()
        }),
        (
            DimensionKind::Linear { rotation },
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => Dimension::Linear(DimensionLinear {
            base,
            first_point: v3(*first),
            second_point: v3(*second),
            definition_point: v3(*dimension_line),
            rotation,
            ..DimensionLinear::default()
        }),
        (
            DimensionKind::Radius,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                leader_length,
            },
        ) => Dimension::Radius(DimensionRadius {
            base,
            angle_vertex: v3(*center),
            definition_point: v3(*point_on_arc),
            leader_length: *leader_length,
        }),
        (
            DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                leader_length,
            },
        ) => Dimension::Diameter(DimensionDiameter {
            base,
            angle_vertex: v3(*center),
            definition_point: v3(*point_on_arc),
            leader_length: *leader_length,
        }),
        (
            DimensionKind::Angular2Ln,
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            },
        ) => Dimension::Angular2Ln(DimensionAngular2Ln {
            base,
            first_point: v3(*line1_start),
            second_point: v3(*line1_end),
            angle_vertex: v3(*line2_start),
            definition_point: v3(*line2_end),
            dimension_arc: v3(*arc_point),
        }),
        (
            DimensionKind::Angular3Pt,
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            },
        ) => Dimension::Angular3Pt(DimensionAngular3Pt {
            base,
            angle_vertex: v3(*vertex),
            first_point: v3(*first),
            second_point: v3(*second),
            definition_point: v3(*arc_point),
        }),
        (
            DimensionKind::Ordinate { x_ordinate },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base: datum,
            },
        ) => Dimension::Ordinate(DimensionOrdinate {
            base,
            definition_point: v3(*datum),
            feature_location: v3(*feature),
            leader_endpoint: v3(*leader_end),
            is_ordinate_type_x: x_ordinate,
        }),
        _ => {
            return Err(CadError::InvalidInput(
                "dimension subtype and point slots disagree".to_string(),
            ));
        }
    })
}

/// A DIMSTYLE table record carrying the resolved (already-scaled) values, with
/// `DIMSCALE=1` so reading it back reproduces the same display.
fn dimstyle_entry(name: &str, style: &DimensionStyleValues) -> DimStyle {
    let mut entry = DimStyle::new(name);
    entry.dimscale = 1.0;
    entry.dimasz = style.arrow_size;
    entry.dimexo = style.ext_offset;
    entry.dimexe = style.ext_beyond;
    entry.dimtxt = style.text_height;
    entry.dimgap = style.text_gap;
    entry.dimtad = if style.text_above { 1 } else { 0 };
    entry.dimlfac = style.linear_factor;
    entry.dimdec = style.decimals as i16;
    entry.dimzin = if style.suppress_trailing_zeros { 8 } else { 0 };
    entry.dimtxsty = style.text_style_name.clone();
    entry
}
