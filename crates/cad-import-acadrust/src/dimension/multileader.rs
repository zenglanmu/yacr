//! MULTILEADER path, arrowhead, dogleg and annotation-text synthesis.
//!
//! Submodule of [`crate::dimension`].
//!
//! P7b-1 (ADR 0019 decision C) migrates **MText content and content-less
//! leaders** to the first-class [`SemanticGeometry::MultiLeader`] variant. Block
//! and tolerance content deliberately stay on the pre-P7b `Compound` path
//! (decision C.3) so their rendering does not regress; the variant's owner-
//! derived display is a verbatim move of that synthesis (`cad-domain`'s
//! `multileader` module).

use super::*;
use acadrust::entities::{
    LeaderLinePropertyOverrideFlags, MultiLeaderPathType as AcadPathType,
    MultiLeaderPropertyOverrideFlags, TextAttachmentPointType, TextAttachmentType,
};

/// Build a MULTILEADER leader path honoring its `path_type` (pre-P7b helper,
/// kept for the block/tolerance `Compound` path only).
///
/// `Invisible` draws nothing, `StraightLineSegments` a polyline, and `Spline` a
/// smooth B-spline through the same points (tessellated by the representation
/// layer), never a straight polyline.
fn multileader_path(points: &[Point3], path_type: AcadPathType) -> Option<SemanticGeometry> {
    match path_type {
        AcadPathType::Invisible => None,
        AcadPathType::StraightLineSegments => Some(polyline_semantics(points.to_vec(), false)),
        AcadPathType::Spline => {
            let degree = 3.min(points.len().saturating_sub(1).max(1)) as u32;
            Some(SemanticGeometry::Spline {
                degree,
                knots: Vec::new(),
                control_points: points.to_vec(),
                weights: Vec::new(),
            })
        }
    }
}

/// Map the MULTILEADER horizontal attachment point to a text alignment.
fn multileader_h_align(point: TextAttachmentPointType) -> TextAlignH {
    match point {
        TextAttachmentPointType::Left => TextAlignH::Left,
        TextAttachmentPointType::Center => TextAlignH::Center,
        TextAttachmentPointType::Right => TextAlignH::Right,
    }
}

/// Map a MULTILEADER text attachment (the enum carries top/middle/bottom line
/// positions) to a vertical text alignment.
fn multileader_v_align(attachment: TextAttachmentType) -> TextAlignV {
    match attachment {
        TextAttachmentType::TopOfTopLine | TextAttachmentType::MiddleOfTopLine => TextAlignV::Top,
        TextAttachmentType::BottomOfBottomLine
        | TextAttachmentType::BottomLine
        | TextAttachmentType::BottomOfTopLineUnderlineBottomLine
        | TextAttachmentType::BottomOfTopLineUnderlineTopLine
        | TextAttachmentType::BottomOfTopLineUnderlineAll => TextAlignV::Bottom,
        TextAttachmentType::MiddleOfText
        | TextAttachmentType::MiddleOfBottomLine
        | TextAttachmentType::CenterOfText
        | TextAttachmentType::CenterOfTextOverline => TextAlignV::Middle,
    }
}

/// A `true` identity check for a column-major 4x4 transform.
fn is_identity_transform(matrix: &[f64; 16]) -> bool {
    const IDENTITY: [f64; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    matrix
        .iter()
        .zip(IDENTITY.iter())
        .all(|(value, expected)| (value - expected).abs() < 1e-12)
}

/// The dogleg / landing segment at the content end of a leader root (pre-P7b
/// helper, kept for the block/tolerance `Compound` path only).
///
/// It starts at the last leader-line vertex and runs toward the root's content
/// connection point for `landing_distance` (falling back to the entity/style
/// dogleg length), stopping `landing_gap` short of the content.
fn multileader_dogleg(
    root: &acadrust::entities::LeaderRoot,
    default_length: f64,
    landing_gap: f64,
) -> Option<SemanticGeometry> {
    let start = p3(root
        .lines
        .iter()
        .find_map(|line| line.points.last().copied())?);
    if !cad_geometry::is_finite(start) {
        return None;
    }
    let length = if root.landing_distance.is_finite() && root.landing_distance > 1e-9 {
        root.landing_distance
    } else if default_length.is_finite() {
        default_length
    } else {
        0.0
    };
    let reach = (length - landing_gap.max(0.0)).max(0.0);
    if reach < 1e-9 {
        return None;
    }
    let connection = p3(root.connection_point);
    let toward_content = cad_geometry::sub(connection, start);
    let direction =
        if cad_geometry::is_finite(connection) && cad_geometry::length(toward_content) > 1e-9 {
            cad_geometry::normalize(toward_content)
        } else {
            let fallback = p3(root.direction);
            if cad_geometry::is_finite(fallback) && cad_geometry::length(fallback) > 1e-9 {
                cad_geometry::normalize(fallback)
            } else {
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                }
            }
        };
    let end = cad_geometry::add(start, cad_geometry::scale(direction, reach));
    line_between(start, end)
}

/// The effective style object, if the entity references a resolvable
/// `MultiLeaderStyle` object.
fn style_object<'a>(
    acad: &'a acadrust::CadDocument,
    ml: &acadrust::entities::MultiLeader,
) -> Option<&'a acadrust::objects::MultiLeaderStyle> {
    // MLEADER styles live in the document's object map, not a typed table.
    let handle = ml.style_handle?;
    if handle.is_null() {
        return None;
    }
    match acad.objects.get(&handle) {
        Some(acadrust::objects::ObjectType::MultiLeaderStyle(style)) => Some(style),
        _ => None,
    }
}

/// The domain path-type mirror of the acadrust entity enum.
fn domain_path_type(path_type: AcadPathType) -> MLeaderPathType {
    match path_type {
        AcadPathType::Invisible => MLeaderPathType::Invisible,
        AcadPathType::StraightLineSegments => MLeaderPathType::StraightLineSegments,
        AcadPathType::Spline => MLeaderPathType::Spline,
    }
}

/// A source colour as the domain's raw CCR/CMC pair.
fn domain_color(color: acadrust::types::Color) -> MLeaderColor {
    MLeaderColor {
        index: color.index(),
        true_color: color.to_true_color_value(),
    }
}

/// A non-null source handle as its raw value.
fn handle_value(handle: Option<acadrust::types::Handle>) -> Option<u64> {
    handle
        .filter(|handle| !handle.is_null())
        .map(|handle| handle.value())
}

/// Map source break pairs into the domain's verbatim record.
fn domain_breaks(points: &[acadrust::entities::StartEndPointPair]) -> Vec<MLeaderBreak> {
    points
        .iter()
        .map(|pair| MLeaderBreak {
            start: p3(pair.start_point),
            end: p3(pair.end_point),
        })
        .collect()
}

/// Background-fill parameters, when the source carried any non-default value.
///
/// The `reserved` partition rule forbids synthesised entries, so an entirely
/// default background is reported as absent. acadrust's defaults are
/// indistinguishable from a source that set the same values.
fn background_params(
    context: &acadrust::entities::MultiLeaderAnnotContext,
) -> Option<MLeaderBackground> {
    let defaults = acadrust::entities::MultiLeaderAnnotContext::new();
    let touched = context.background_fill_enabled
        || context.background_mask_fill_on
        || context.background_fill_color != defaults.background_fill_color
        || context.background_scale_factor != defaults.background_scale_factor
        || context.background_transparency != defaults.background_transparency;
    touched.then(|| MLeaderBackground {
        fill_enabled: context.background_fill_enabled,
        mask_fill_on: context.background_mask_fill_on,
        fill_color: domain_color(context.background_fill_color),
        scale_factor: context.background_scale_factor,
        transparency: context.background_transparency,
    })
}

/// Column-layout parameters, when the source carried any non-default value.
fn column_params(context: &acadrust::entities::MultiLeaderAnnotContext) -> Option<MLeaderColumns> {
    let touched = context.column_type != 0
        || context.column_width != 0.0
        || context.column_gutter != 0.0
        || context.column_flow_reversed
        || !context.column_sizes.is_empty();
    touched.then(|| MLeaderColumns {
        column_type: context.column_type,
        width: context.column_width,
        gutter: context.column_gutter,
        flow_reversed: context.column_flow_reversed,
        sizes: context.column_sizes.clone(),
    })
}

/// Named deviation reused verbatim from the pre-P7b `Compound` path: a custom
/// arrowhead block is not expanded.
const CUSTOM_ARROWHEAD_REASON: &str = "custom multileader arrowhead blocks are not expanded";

/// The variant path's named deviations for custom arrowhead blocks.
///
/// The source carries a custom arrowhead block handle at two levels: the
/// entity-level handle (entity-tail `342`) and the per-line handles (`341`).
/// The display honours neither as a block — it always draws the built-in
/// arrowhead mesh — but only the per-line handles reach the model
/// ([`MLeaderLine::arrowhead_handle`]). The entity-level handle has no model
/// home yet, and the pre-P7b `Compound` path named exactly that loss, so the
/// variant reports it too instead of dropping it silently. When per-line
/// handles disagree with the entity-level one, both are named rather than
/// silently picking one.
fn custom_arrowhead_reasons(ml: &acadrust::entities::MultiLeader) -> Vec<String> {
    let Some(entity_handle) = handle_value(ml.arrowhead_handle) else {
        return Vec::new();
    };
    let mut line_handles: Vec<u64> = ml
        .context
        .leader_roots
        .iter()
        .flat_map(|root| root.lines.iter())
        .filter_map(|line| handle_value(line.arrowhead_handle))
        .collect();
    line_handles.sort_unstable();
    line_handles.dedup();

    let mut reasons = vec![format!(
        "{CUSTOM_ARROWHEAD_REASON}: the entity-level arrowhead block handle {entity_handle:X} \
         (entity-tail 342) is not modelled, so the built-in default arrowhead is rendered \
         instead"
    )];
    let disagreeing: Vec<u64> = line_handles
        .into_iter()
        .filter(|handle| *handle != entity_handle)
        .collect();
    if !disagreeing.is_empty() {
        let listed = disagreeing
            .iter()
            .map(|handle| format!("{handle:X}"))
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!(
            "entity-level arrowhead block handle {entity_handle:X} and per-line arrowhead block \
             handle(s) {listed} disagree; the per-line handles are preserved verbatim and the \
             entity-level handle is not modelled, so the built-in default arrowhead is rendered"
        ));
    }
    reasons
}

impl ImporterBuilder<'_> {
    /// Convert a MULTILEADER: its leader-root paths (honoring `path_type`),
    /// arrowheads, dogleg/landing and annotation text.
    ///
    /// ADR 0019 C.3 migrates only MText content and content-less leaders into
    /// the first-class [`SemanticGeometry::MultiLeader`] variant. Block and
    /// tolerance content stay on the pre-P7b `Compound` synthesis so their
    /// rendering does not regress.
    pub(crate) fn multileader_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        let is_block_or_tolerance = matches!(
            ml.content_type,
            acadrust::entities::LeaderContentType::Block
                | acadrust::entities::LeaderContentType::Tolerance
        ) || ml.context.has_block_contents;
        if is_block_or_tolerance {
            return self.multileader_compound_semantics(ml);
        }
        self.multileader_variant_semantics(ml)
    }

    /// The pre-P7b `Compound` synthesis, kept **unchanged** for block and
    /// tolerance content (ADR 0019 C.3): those entities still render exactly as
    /// before and still cannot round-trip as MULTILEADER (same loss as today).
    fn multileader_compound_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        let context = &ml.context;
        let style = style_object(self.acad, ml);

        // Effective defaults: a set override flag wins over the style, which
        // wins over the entity's own stored value.
        let entity_path_type = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::PATH_TYPE)
        {
            ml.path_type
        } else {
            match style {
                // The style object's path type is a distinct enum; convert by
                // its repr value.
                Some(style) => AcadPathType::from(style.path_type as i16),
                None => ml.path_type,
            }
        };
        let enable_dogleg = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::ENABLE_DOGLEG)
        {
            ml.enable_dogleg
        } else {
            style.map(|s| s.enable_dogleg).unwrap_or(ml.enable_dogleg)
        };
        let default_dogleg_length = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::LANDING_DISTANCE)
        {
            ml.dogleg_length
        } else {
            style
                .map(|s| s.landing_distance)
                .unwrap_or(ml.dogleg_length)
        };
        let landing_gap = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::LANDING_GAP)
        {
            context.landing_gap
        } else {
            style.map(|s| s.landing_gap).unwrap_or(context.landing_gap)
        };
        let entity_arrowhead_size = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::ARROWHEAD_SIZE)
        {
            ml.arrowhead_size
        } else {
            style.map(|s| s.arrowhead_size).unwrap_or(ml.arrowhead_size)
        };
        let mut custom_arrowhead = ml.arrowhead_handle.is_some_and(|handle| !handle.is_null());

        let mut children: Vec<SemanticGeometry> = Vec::new();
        for root in &context.leader_roots {
            for line in &root.lines {
                if line.points.len() < 2 {
                    continue;
                }
                let points: Vec<Point3> = line.points.iter().map(|p| p3(*p)).collect();
                let path_type = if line
                    .override_flags
                    .contains(LeaderLinePropertyOverrideFlags::PATH_TYPE)
                {
                    line.path_type
                } else {
                    entity_path_type
                };
                if let Some(path) = multileader_path(&points, path_type) {
                    children.push(path);
                }
                if path_type == AcadPathType::Invisible {
                    continue;
                }
                if line
                    .arrowhead_handle
                    .is_some_and(|handle| !handle.is_null())
                {
                    custom_arrowhead = true;
                }
                let arrow_size = if line
                    .override_flags
                    .contains(LeaderLinePropertyOverrideFlags::ARROWHEAD_SIZE)
                    && line.arrowhead_size.is_finite()
                    && line.arrowhead_size > 0.0
                {
                    line.arrowhead_size
                } else {
                    entity_arrowhead_size
                };
                let outward = cad_geometry::sub(points[0], points[1]);
                if arrow_size.is_finite()
                    && arrow_size > 1e-9
                    && cad_geometry::length(outward) > 1e-9
                {
                    children.push(arrowhead(points[0], outward, arrow_size));
                }
            }
            if enable_dogleg {
                if let Some(dogleg) = multileader_dogleg(root, default_dogleg_length, landing_gap) {
                    children.push(dogleg);
                }
            }
        }

        let mut reasons: Vec<String> = vec![
            "multileader colour, lineweight, linetype and property overrides are resolved but not \
             applied (the representation carries one colour per entity)"
                .into(),
        ];
        if context.background_fill_enabled {
            reasons.push("multileader background fill is not applied".into());
        }
        if context.column_type != 0 || !context.column_sizes.is_empty() {
            reasons.push("multileader column layout is not applied".into());
        }
        if !ml.block_attributes.is_empty() {
            reasons.push("multileader block attributes are not applied".into());
        }
        if !is_identity_transform(&context.transform_matrix) {
            reasons.push("multileader transform matrix is not applied".into());
        }
        if custom_arrowhead {
            reasons.push("custom multileader arrowhead blocks are not expanded".into());
        }

        if context.has_text_contents && !context.text_string.is_empty() {
            let style_name = context
                .text_style_handle
                .and_then(|handle| self.text_style_name_for_handle(handle))
                .unwrap_or_else(|| "Standard".to_string());
            let height = if context.text_height.is_finite() && context.text_height > 0.0 {
                context.text_height
            } else {
                ml.text_height
            };
            // Prefer a non-default entity attachment; otherwise the context's.
            let attachment_point = if ml.text_attachment_point != TextAttachmentPointType::default()
            {
                ml.text_attachment_point
            } else {
                context.text_attachment_point
            };
            let left_attachment = if ml.text_left_attachment != TextAttachmentType::default() {
                ml.text_left_attachment
            } else {
                context.text_left_attachment
            };
            children.push(SemanticGeometry::Text {
                text: context.text_string.clone(),
                position: p3(context.text_location),
                style: self.style_id(&style_name),
                height,
                rotation: context.text_rotation,
                font: self.style_font(&style_name),
                h_align: multileader_h_align(attachment_point),
                v_align: multileader_v_align(left_attachment),
            });
        }
        if context.has_block_contents {
            // Expand the content block when its handle resolves to a known
            // block; otherwise report it rather than silently dropping it.
            let expanded = context.block_content_handle.and_then(|handle| {
                let record = self
                    .acad
                    .block_records
                    .iter()
                    .find(|record| record.handle.value() == handle.value())?;
                self.block_ids
                    .contains_key(&record.name)
                    .then(|| SemanticGeometry::Insert {
                        block: self.block_id(&record.name),
                        transform: placement_transform(
                            context.block_content_location,
                            context.block_rotation,
                            context.block_content_scale,
                        ),
                    })
            });
            match expanded {
                Some(insert) => children.push(insert),
                None => {
                    reasons.push("mleader block content handle did not resolve to a block".into());
                }
            }
        }
        if children.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbMultiLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["multileader has no leader lines or text".into()]),
            );
        }
        (
            SemanticGeometry::Compound(children),
            Completeness::Partial(reasons),
        )
    }

    /// Migrate an MText/content-less MULTILEADER into the first-class variant
    /// (ADR 0019 decision C).
    ///
    /// Every modelled field keeps its verbatim source value; unmodelled source
    /// data (breaks, background/column parameters, per-line colour/linetype/
    /// lineweight) goes into the opaque `reserved` bag exactly once. The derived
    /// display is built by `cad-domain` and is pixel-identical to the pre-P7b
    /// `Compound` synthesis. The entity-level custom arrowhead handle
    /// (entity-tail `342`) has no model home yet, so it is not retained — it is
    /// reported as a named completeness reason instead (see
    /// [`custom_arrowhead_reasons`]).
    fn multileader_variant_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        let context = &ml.context;
        let style = style_object(self.acad, ml);
        let mask = ml.property_override_flags.bits();

        let entity_path_type = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::PATH_TYPE)
        {
            ml.path_type
        } else {
            match style {
                Some(style) => AcadPathType::from(style.path_type as i16),
                None => ml.path_type,
            }
        };
        let enable_landing = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::ENABLE_LANDING)
        {
            ml.enable_landing
        } else {
            style.map(|s| s.enable_landing).unwrap_or(ml.enable_landing)
        };
        let enable_dogleg = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::ENABLE_DOGLEG)
        {
            ml.enable_dogleg
        } else {
            style.map(|s| s.enable_dogleg).unwrap_or(ml.enable_dogleg)
        };
        let default_dogleg_length = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::LANDING_DISTANCE)
        {
            ml.dogleg_length
        } else {
            style
                .map(|s| s.landing_distance)
                .unwrap_or(ml.dogleg_length)
        };
        let landing_gap = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::LANDING_GAP)
        {
            context.landing_gap
        } else {
            style.map(|s| s.landing_gap).unwrap_or(context.landing_gap)
        };
        let arrowhead_size = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::ARROWHEAD_SIZE)
        {
            ml.arrowhead_size
        } else {
            style.map(|s| s.arrowhead_size).unwrap_or(ml.arrowhead_size)
        };
        let style_name = style
            .map(|style| style.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Standard".to_string());
        let text_style_name = context
            .text_style_handle
            .and_then(|handle| self.text_style_name_for_handle(handle))
            .unwrap_or_else(|| "Standard".to_string());
        let text_height = if context.text_height.is_finite() && context.text_height > 0.0 {
            context.text_height
        } else {
            ml.text_height
        };
        let attachment_point = if ml.text_attachment_point != TextAttachmentPointType::default() {
            ml.text_attachment_point
        } else {
            context.text_attachment_point
        };
        let left_attachment = if ml.text_left_attachment != TextAttachmentType::default() {
            ml.text_left_attachment
        } else {
            context.text_left_attachment
        };

        let mut roots = Vec::with_capacity(context.leader_roots.len());
        let mut lines = Vec::new();
        let mut root_extras = Vec::with_capacity(context.leader_roots.len());
        let mut line_extras = Vec::new();
        for (root_index, root) in context.leader_roots.iter().enumerate() {
            roots.push(MLeaderRoot {
                leader_index: root.leader_index,
                landing_point: p3(root.connection_point),
                dogleg_vector: p3(root.direction),
                dogleg_length: root.landing_distance,
            });
            // The root's own unmodelled flags and break pairs (ADR 0019 C.2).
            root_extras.push(MLeaderRootExtras {
                root_index,
                content_valid: root.content_valid,
                unknown: root.unknown,
                text_attachment_direction: root.text_attachment_direction as i16,
                breaks: domain_breaks(&root.break_points),
            });
            for line in &root.lines {
                let line_index = lines.len();
                lines.push(MLeaderLine {
                    root_index,
                    index: line.index,
                    points: line.points.iter().map(|point| p3(*point)).collect(),
                    path_type: domain_path_type(line.path_type),
                    override_flags: line.override_flags.bits(),
                    arrowhead_size: line.arrowhead_size,
                    arrowhead_handle: handle_value(line.arrowhead_handle),
                });
                let breaks: Vec<MLeaderLineBreak> = line
                    .break_infos
                    .iter()
                    .map(|info| MLeaderLineBreak {
                        segment_index: info.segment_index,
                        points: domain_breaks(&info.break_points),
                    })
                    .collect();
                // The legacy mirrors are derived from `break_infos`; store them
                // only when the complete records are absent, so no source pair is
                // written twice (ADR 0019 C.2).
                let legacy_breaks = if breaks.is_empty() {
                    domain_breaks(&line.break_points)
                } else {
                    Vec::new()
                };
                line_extras.push(MLeaderLineExtras {
                    root_index,
                    line_index,
                    segment_index: line.segment_index,
                    breaks,
                    break_info_count: line.break_info_count,
                    legacy_breaks,
                    line_color: domain_color(line.line_color),
                    line_type_handle: handle_value(line.line_type_handle),
                    line_weight: line.line_weight.value(),
                });
            }
        }
        let reserved = MLeaderReserved {
            roots: root_extras,
            lines: line_extras,
            background: background_params(context),
            columns: column_params(context),
        };

        let parts = MultiLeaderParts {
            style: MLeaderStyleValues {
                path_type: domain_path_type(entity_path_type),
                enable_landing,
                enable_dogleg,
                landing_distance: default_dogleg_length,
                landing_gap,
                arrowhead_size,
                text_height,
                text_style: self.style_id(&text_style_name),
                text_style_name: text_style_name.clone(),
                text_font: self.style_font(&text_style_name),
                scale_factor: ml.scale_factor,
            },
            style_name,
            override_mask: mask,
            content: if context.has_text_contents && !context.text_string.is_empty() {
                MLeaderContent::Text
            } else {
                MLeaderContent::None
            },
            text: MLeaderText {
                string: context.text_string.clone(),
                location: p3(context.text_location),
                rotation: context.text_rotation,
                height: text_height,
                width: context.text_width,
                attachment_point: attachment_point as i16,
                left_attachment: left_attachment as i16,
            },
            roots,
            lines,
            reserved,
            plane: PlaneFrame {
                origin: p3(context.base_point),
                u: p3(context.base_direction),
                v: p3(context.base_vertical),
                normal_reversed: context.normal_reversed,
            },
        };
        let (geometry, reasons) = MultiLeaderGeometry::from_import(parts);
        let mut reasons = reasons;
        reasons.extend(custom_arrowhead_reasons(ml));
        if geometry.display().is_empty() {
            reasons.push("multileader has no leader lines or text".into());
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbMultiLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(reasons),
            );
        }
        (
            SemanticGeometry::MultiLeader(Box::new(geometry)),
            Completeness::Partial(reasons),
        )
    }
}
