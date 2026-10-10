//! Reverse mapping: domain [`SemanticGeometry`] → acadrust entities.
//!
//! This inverts `crate::entity`'s import mapping. It is **exhaustive with no
//! catch-all arm**: adding a new `SemanticGeometry` variant must be handled here
//! (a compile error), never silently dropped. Every approximation is recorded in
//! the [`ExportReport`](super::report::ExportReport).
//!
//! "Exact" means **faithful to the domain database**, not faithful to the source
//! DWG: the database has already dropped information (ByLayer resolution, block
//! names, spline closed/periodic flags), and a `Spline` is therefore reported as
//! `Converted`, never `Exact`.

use std::collections::BTreeSet;

use acadrust::entities::{
    Arc as DxfArc, AttachmentPoint, Circle as DxfCircle, Ellipse as DxfEllipse, EntityCommon,
    EntityType, Line as DxfLine, LwPolyline, LwVertex, MText, Point as DxfPoint, PolyfaceFace,
    PolyfaceMesh, PolyfaceVertex, Polyline3D, Spline as DxfSpline, Text as DxfText,
};
use acadrust::tables::{LineType as DxfLineType, LineTypeElement};
use acadrust::types::{Color, LineWeight, Matrix3, Vector2, Vector3};
use acadrust::{CadDocument, Transparency};
use cad_db::{
    DbEntity, DrawingDatabase, EntityColor, EntityLineType, EntityLineWeight, EntityTransparency,
    LinetypePattern,
};
use cad_domain::*;

use super::report::{ExportCounts, ExportDiagnostic, ExportLimits, MappingOutcome};
use super::tables::ensure_text_style;

/// The largest polyface vertex index that fits `PolyfaceFace`'s signed `i16`
/// fields (1-based, so a 0-based index must be `< 32767`).
const MAX_POLYFACE_INDEX: usize = 32766;

/// Immutable inputs shared by the whole export.
pub(crate) struct EntityContext<'a> {
    pub database: &'a DrawingDatabase,
    pub limits: ExportLimits,
}

/// Mutable output accumulated while mapping.
pub(crate) struct ExportAccumulator {
    pub entities: Vec<EntityType>,
    pub counts: ExportCounts,
    pub entries: Vec<ExportDiagnostic>,
    pub notes: Vec<ExportDiagnostic>,
    /// Block definitions reached by an INSERT expansion.
    pub expanded_blocks: BTreeSet<BlockId>,
}

impl ExportAccumulator {
    pub(crate) fn new() -> Self {
        ExportAccumulator {
            entities: Vec::new(),
            counts: ExportCounts::default(),
            entries: Vec::new(),
            notes: Vec::new(),
            expanded_blocks: BTreeSet::new(),
        }
    }

    pub(crate) fn record(&mut self, object: Option<ObjectId>, kind: &str, outcome: MappingOutcome) {
        match outcome {
            MappingOutcome::Exact => self.counts.exact += 1,
            MappingOutcome::Converted(reasons) => {
                self.counts.converted += 1;
                for reason in reasons {
                    self.entries.push(ExportDiagnostic {
                        object,
                        code: format!("export.converted.{kind}"),
                        message: reason,
                    });
                }
            }
            MappingOutcome::Dropped(reasons) => {
                self.counts.dropped += 1;
                for reason in reasons {
                    self.entries.push(ExportDiagnostic {
                        object,
                        code: format!("export.dropped.{kind}"),
                        message: reason,
                    });
                }
            }
        }
    }

    fn push(&mut self, entity: EntityType, ctx: &EntityContext) -> CadResult<()> {
        if self.entities.len() >= ctx.limits.max_output_entities {
            return Err(CadError::InvalidInput(format!(
                "export exceeds the {} entity output limit",
                ctx.limits.max_output_entities
            )));
        }
        self.entities.push(entity);
        Ok(())
    }
}

/// Fold extra per-entity reasons into a base outcome.
///
/// An `Exact` outcome with reasons becomes `Converted`, so one geometry is never
/// counted as both faithful and lossy.
fn with_reasons(base: MappingOutcome, extra: Vec<String>) -> MappingOutcome {
    if extra.is_empty() {
        return base;
    }
    match base {
        MappingOutcome::Exact => MappingOutcome::Converted(extra),
        MappingOutcome::Converted(mut reasons) => {
            reasons.extend(extra);
            MappingOutcome::Converted(reasons)
        }
        MappingOutcome::Dropped(mut reasons) => {
            reasons.extend(extra);
            MappingOutcome::Dropped(reasons)
        }
    }
}

/// Map one model-space entity's geometry, expanding INSERTs and flattening
/// compounds. `transform` is the accumulated block placement (identity at the
/// model root).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit(
    doc: &mut CadDocument,
    entity: &DbEntity,
    transform: &Transform3,
    depth: usize,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
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
                return Ok(());
            }
            if depth + 1 > ctx.limits.max_explode_depth {
                return Err(CadError::InvalidInput(format!(
                    "insert explode depth exceeds the {} limit",
                    ctx.limits.max_explode_depth
                )));
            }
            let composed = transform.matrix_mul(insert_transform);
            let Some(definition) = ctx.database.block(*block) else {
                acc.record(
                    object,
                    "insert",
                    MappingOutcome::Dropped(vec![
                        "insert references a block definition that is not in the database".into(),
                    ]),
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
                    Some(child) => emit(doc, child, &composed, depth + 1, ctx, acc, stack)?,
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
            let mut reasons = vec![
                "block reference exploded; block identity, name and base point are lost"
                    .to_string(),
            ];
            if let Some(reason) = visibility_reason {
                reasons.push(reason);
            }
            acc.record(object, "insert", MappingOutcome::Converted(reasons));
            Ok(())
        }
        SemanticGeometry::Compound(children) => {
            let object = Some(entity.object.id);
            for child in children {
                // A compound is a single source entity; its children inherit the
                // entity's identity and placement.
                let child_entity = DbEntity {
                    geometry: child.clone(),
                    ..entity.clone()
                };
                emit(doc, &child_entity, transform, depth, ctx, acc, stack)?;
            }
            acc.record(
                object,
                "compound",
                MappingOutcome::Converted(vec![
                    "compound flattened into separate output entities".into()
                ]),
            );
            Ok(())
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
            acc.record(object, "line", with_reasons(MappingOutcome::Exact, reasons));
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => emit_polyline(doc, entity, points, bulges, *closed, ctx, acc)?,
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
            arc.start_angle = *start;
            arc.end_angle = (*start + *sweep).rem_euclid(std::f64::consts::TAU);
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
        } => {
            let (common, mut reasons) = common_for(doc, entity, ctx);
            let mut spline = DxfSpline::new();
            spline.degree = *degree as i32;
            spline.knots = knots.clone();
            spline.control_points = control_points.iter().map(|p| v3(*p)).collect();
            spline.weights = weights.clone();
            spline.normal = Vector3::UNIT_Z;
            spline.common = common;
            acc.push(EntityType::Spline(spline), ctx)?;
            // The domain model has no closed/periodic flags; the output writes
            // them false. That is a conversion, not an exact copy.
            reasons.push(
                "SPLINE closed/periodic flags are not carried in the domain model; written as \
                 false"
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
            // Polyface face indices are signed i16 (1-based). A mesh outside that
            // range must not be emitted with wrapped indices, which would corrupt
            // the file; drop it explicitly instead of erroring the whole export.
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
            let mut reasons = reasons;
            reasons.push(
                "mesh written as a polyface mesh; per-vertex colours and normals are dropped"
                    .into(),
            );
            acc.record(object, "mesh", MappingOutcome::Converted(reasons));
        }
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
        } => {
            let style_name = ctx
                .database
                .style(*style)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let (style_name, mut reasons) = ensure_text_style(doc, &style_name, font.as_deref());
            let (common, extra) = common_for(doc, entity, ctx);
            reasons.extend(extra);
            if text.contains('\n') {
                let mut mtext = MText::new();
                mtext.value = text.clone();
                mtext.insertion_point = v3(*position);
                mtext.height = *height;
                mtext.rotation = *rotation;
                mtext.style = style_name;
                mtext.normal = Vector3::UNIT_Z;
                let (attachment, align_reason) = mtext_attachment(*h_align, *v_align);
                mtext.attachment_point = attachment;
                if let Some(reason) = align_reason {
                    reasons.push(reason);
                }
                mtext.common = common;
                acc.push(EntityType::MText(mtext), ctx)?;
                reasons.push("multi-line text written as MTEXT".into());
                acc.record(object, "text_mtext", MappingOutcome::Converted(reasons));
            } else {
                let mut dxf_text = DxfText::new();
                dxf_text.value = text.clone();
                dxf_text.insertion_point = v3(*position);
                dxf_text.height = *height;
                dxf_text.rotation = *rotation;
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
        SemanticGeometry::Image { .. } => acc.record(
            object,
            "image",
            MappingOutcome::Dropped(vec![
                "raster image is not exported (external resource)".into()
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
        // Insert and Compound are handled before this point; a leaf here would
        // be a programming error, and must not be silently dropped.
        SemanticGeometry::Insert { .. } | SemanticGeometry::Compound(_) => {
            acc.record(
                object,
                "internal",
                MappingOutcome::Dropped(vec![
                    "insert/compound reached the leaf mapper; not emitted".into(),
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
        if has_bulge {
            let mut reasons = reasons;
            reasons.push("non-planar polyline bulges linearized to straight segments".into());
            acc.record(object, "polyline_bulge", MappingOutcome::Converted(reasons));
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

/// Build the common entity data and collect any per-entity conversion reasons.
fn common_for(
    doc: &mut CadDocument,
    entity: &DbEntity,
    ctx: &EntityContext,
) -> (EntityCommon, Vec<String>) {
    let attributes = ctx.database.entity_render_attributes(entity.id);
    let mut reasons: Vec<String> = Vec::new();
    let mut common = EntityCommon::new();
    common.layer = ctx
        .database
        .layer(entity.layer)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "0".to_string());
    common.color = match attributes.color {
        EntityColor::Explicit([r, g, b]) => Color::Rgb { r, g, b },
        EntityColor::ByBlock => Color::ByBlock,
        EntityColor::ByLayer => Color::ByLayer,
    };
    common.line_weight = match attributes.lineweight {
        EntityLineWeight::Explicit(mm) => {
            LineWeight::Value((mm * 100.0).round().clamp(0.0, 211.0) as i16)
        }
        EntityLineWeight::Default => LineWeight::Default,
        EntityLineWeight::ByBlock => LineWeight::ByBlock,
        EntityLineWeight::ByLayer => LineWeight::ByLayer,
    };
    // Transparency: the domain stores a resolved opacity in `[0, 1]`; DXF group
    // 440 stores transparency (0 = opaque, 255 = transparent).
    common.transparency = match attributes.transparency {
        EntityTransparency::Explicit(opacity) => Transparency::Explicit(
            ((1.0 - opacity.clamp(0.0, 1.0)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8,
        ),
        EntityTransparency::ByBlock => Transparency::ByBlock,
    };
    // Linetype scale (group 48): an explicit entity scale is carried; inherited
    // values keep the 1.0 default.
    common.linetype_scale = match &attributes.linetype {
        EntityLineType::Explicit { scale, .. } => *scale,
        _ => 1.0,
    };
    common.linetype = match &attributes.linetype {
        EntityLineType::Explicit { name, pattern, .. } if !name.trim().is_empty() => {
            if doc.line_types.get(name).is_none() {
                doc.line_types
                    .add_or_replace(linetype_from_pattern(name, pattern));
            }
            name.clone()
        }
        EntityLineType::Explicit { pattern, .. } => {
            if !pattern.elements.is_empty() {
                reasons.push(
                    "linetype has an empty name; its dash pattern is not written (continuous)"
                        .into(),
                );
            }
            String::new()
        }
        EntityLineType::ByLayer => String::new(),
        EntityLineType::ByBlock => "ByBlock".to_string(),
    };
    (common, reasons)
}

fn linetype_from_pattern(name: &str, pattern: &LinetypePattern) -> DxfLineType {
    let mut entry = DxfLineType::new(name);
    entry.elements = pattern
        .elements
        .iter()
        .map(|length| LineTypeElement {
            length: *length,
            complex: None,
        })
        .collect();
    entry.pattern_length = pattern.cycle;
    entry
}

/// Back-project a WCS point into the OCS frame `arbitrary_axis` rebuilds from
/// `normal`. Import maps a CIRCLE/ARC centre to WCS via
/// `Matrix3::arbitrary_axis(normal) * center`; the export must therefore apply
/// the transpose, or a tilted circle is silently displaced.
fn ocs_from_wcs(normal: Point3, wcs: Point3) -> Vector3 {
    let basis = Matrix3::arbitrary_axis(v3(normal));
    basis.transpose() * v3(wcs)
}

fn map_h_align(align: TextAlignH) -> acadrust::entities::TextHorizontalAlignment {
    use acadrust::entities::TextHorizontalAlignment as H;
    match align {
        TextAlignH::Left => H::Left,
        TextAlignH::Center => H::Center,
        TextAlignH::Right => H::Right,
    }
}

fn map_v_align(align: TextAlignV) -> acadrust::entities::TextVerticalAlignment {
    use acadrust::entities::TextVerticalAlignment as V;
    match align {
        TextAlignV::Baseline => V::Baseline,
        TextAlignV::Bottom => V::Bottom,
        TextAlignV::Middle => V::Middle,
        TextAlignV::Top => V::Top,
    }
}

/// Inverse of `crate::style::attach_align`: map a domain text alignment onto an
/// MTEXT attachment point. MTEXT has no baseline row, so a baseline alignment is
/// approximated as bottom and reported.
fn mtext_attachment(align_h: TextAlignH, align_v: TextAlignV) -> (AttachmentPoint, Option<String>) {
    use AttachmentPoint::*;
    let (row, reason) = match align_v {
        TextAlignV::Top => (0, None),
        TextAlignV::Middle => (1, None),
        TextAlignV::Bottom => (2, None),
        TextAlignV::Baseline => (
            2,
            Some(
                "MTEXT has no baseline attachment; vertical alignment written as bottom"
                    .to_string(),
            ),
        ),
    };
    let attachment = match (align_h, row) {
        (TextAlignH::Left, 0) => TopLeft,
        (TextAlignH::Center, 0) => TopCenter,
        (TextAlignH::Right, 0) => TopRight,
        (TextAlignH::Left, 1) => MiddleLeft,
        (TextAlignH::Center, 1) => MiddleCenter,
        (TextAlignH::Right, 1) => MiddleRight,
        (TextAlignH::Left, _) => BottomLeft,
        (TextAlignH::Center, _) => BottomCenter,
        (TextAlignH::Right, _) => BottomRight,
    };
    (attachment, reason)
}

fn v3(p: Point3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
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
        SemanticGeometry::Insert { .. } => "Insert",
        SemanticGeometry::Text { .. } => "Text",
        SemanticGeometry::Shape { .. } => "Shape",
        SemanticGeometry::Opaque { .. } => "Opaque",
        SemanticGeometry::Compound(_) => "Compound",
        SemanticGeometry::Image { .. } => "Image",
        SemanticGeometry::Mask { .. } => "Mask",
    }
}
