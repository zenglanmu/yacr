//! MULTILEADER path, arrowhead, dogleg and annotation-content synthesis.
//!
//! Submodule of [`crate::dimension`].
//!
//! The entity migrates into the first-class [`SemanticGeometry::MultiLeader`]
//! variant for **every** content kind: MText (`172 = 2`), content-less
//! (`172 = 0`), block (`172 = 1`) and tolerance (`172 = 3`). The variant's
//! owner-derived display lives in `cad-domain`'s `multileader` modules, and this
//! module only maps the source record onto the domain definition — it synthesises
//! no display geometry of its own.
//!
//! ## Declared dependency limit (tolerance content)
//!
//! acadrust 0.6.8's DXF reader maps entity `172 = 3` (tolerance) to
//! `LeaderContentType::None` (its `read_multileader` matches only `1` and `2`,
//! although the same crate's `LeaderContentType::from(i16)` knows `3`). The
//! tolerance kind is therefore resolved from the **style** record (`170 = 3`,
//! which that reader does map) plus the `CONTEXT_DATA` text payload: a leader
//! whose entity kind reads as "none", whose style declares tolerance and which
//! carries a non-empty text string is a tolerance leader. That is the only
//! encoding DXF has (there is no separate tolerance payload — the frame string
//! lives in the text fields), and it is what this build itself writes, so a
//! created tolerance leader reloads as a tolerance leader.
//!
//! Residual: a **vendor** file whose entity says `172 = 3` while its
//! `MLEADERSTYLE 170` says MText is indistinguishable from `172 = 0` after the
//! reader collapses both to "none"; it is imported as text/none content and the
//! frame markup only survives inside the string. That loss is upstream, not a
//! property of this mapping.

use super::*;
use crate::mleader_style::domain_text_angle;
use acadrust::entities::{
    MultiLeaderPathType as AcadPathType, MultiLeaderPropertyOverrideFlags, TextAttachmentPointType,
    TextAttachmentType,
};

/// The domain content kind of a source entity `172` value.
///
/// `LeaderContentType::None` is what acadrust 0.6.8's reader reports for **both**
/// `172 = 0` and `172 = 3` (see the module docs); the tolerance kind is recovered
/// from the style record and the text payload instead of guessing here.
fn entity_content_kind(
    content: acadrust::entities::LeaderContentType,
) -> Option<MLeaderContentKind> {
    match content {
        acadrust::entities::LeaderContentType::None => None,
        acadrust::entities::LeaderContentType::Block => Some(MLeaderContentKind::Block),
        acadrust::entities::LeaderContentType::MText => Some(MLeaderContentKind::Text),
        acadrust::entities::LeaderContentType::Tolerance => Some(MLeaderContentKind::Tolerance),
    }
}

/// The domain content kind of a source **style** `170` value.
fn style_content_kind(content: acadrust::objects::LeaderContentType) -> MLeaderContentKind {
    match content {
        acadrust::objects::LeaderContentType::None => MLeaderContentKind::None,
        acadrust::objects::LeaderContentType::Block => MLeaderContentKind::Block,
        acadrust::objects::LeaderContentType::MText => MLeaderContentKind::Text,
        acadrust::objects::LeaderContentType::Tolerance => MLeaderContentKind::Tolerance,
    }
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
    /// arrowheads, dogleg/landing and its annotation content (MText, block,
    /// tolerance or none).
    ///
    /// Every content kind goes through the one first-class
    /// [`SemanticGeometry::MultiLeader`] path; the only difference between the
    /// kinds is the [`cad_domain::MLeaderContent`] payload this function maps.
    pub(crate) fn multileader_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        self.multileader_variant_semantics(ml)
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
        // Entity `174` / style `175`: the entity value wins only when the `90`
        // mask carries the text-angle override bit, exactly like the other
        // overridable fields above.
        let text_angle_type = if ml
            .property_override_flags
            .contains(MultiLeaderPropertyOverrideFlags::TEXT_ANGLE)
        {
            domain_text_angle(ml.text_angle_type as i16)
        } else {
            match style {
                Some(style) => domain_text_angle(style.text_angle_type as i16),
                None => domain_text_angle(ml.text_angle_type as i16),
            }
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

        // The annotation content. Entity `172` wins; a kind the reader could not
        // report (`None`, which is what `172 = 3` becomes in acadrust 0.6.8)
        // falls back to the style record's `170` and the payload evidence:
        // block contents for a block, a tolerance frame when the style declares
        // tolerance and a text string is present.
        let has_text = context.has_text_contents && !context.text_string.is_empty();
        let entity_kind = entity_content_kind(ml.content_type);
        let kind = match entity_kind {
            // A block kind is carried by its own flag/handle, not by the string.
            Some(MLeaderContentKind::Block) => MLeaderContentKind::Block,
            // Both text-bearing kinds need a string: an empty one keeps the
            // pre-existing content-less normalisation (the exporter names it).
            Some(MLeaderContentKind::Text) | Some(MLeaderContentKind::Tolerance) if has_text => {
                entity_kind.expect("a text-bearing kind")
            }
            _ => {
                if context.has_block_contents {
                    MLeaderContentKind::Block
                } else if style.map(|style| style_content_kind(style.content_type))
                    == Some(MLeaderContentKind::Tolerance)
                    && has_text
                {
                    MLeaderContentKind::Tolerance
                } else if has_text {
                    MLeaderContentKind::Text
                } else {
                    MLeaderContentKind::None
                }
            }
        };
        let content = match kind {
            MLeaderContentKind::None => MLeaderContent::None,
            MLeaderContentKind::Text => MLeaderContent::Text,
            MLeaderContentKind::Tolerance => MLeaderContent::Tolerance(MLeaderToleranceContent {
                text: context.text_string.clone(),
                height: text_height,
            }),
            MLeaderContentKind::Block => {
                let handle = handle_value(context.block_content_handle)
                    .or_else(|| handle_value(ml.block_content_handle));
                let record = handle.and_then(|handle| {
                    self.acad
                        .block_records
                        .iter()
                        .find(|record| record.handle.value() == handle)
                });
                let name = record.map(|record| record.name.clone()).unwrap_or_default();
                // An unresolved handle keeps the kind and the raw handle; the
                // display reports that it cannot draw the block.
                let block = record.and_then(|record| {
                    self.block_ids
                        .contains_key(&record.name)
                        .then(|| self.block_id(&record.name))
                });
                // The definition's own base point is what the placement puts on
                // `CONTEXT_DATA 15` (the same rule an `AcDbBlockReference` uses),
                // so the display stores it instead of re-reading the definition.
                let base_point = self.block_base_points.get(&name).copied();
                MLeaderContent::Block(MLeaderBlockContent {
                    block,
                    name,
                    handle,
                    base_point,
                    location: p3(context.block_content_location),
                    scale: p3(context.block_content_scale),
                    rotation: context.block_rotation,
                    connection: context.block_connection_type as i16,
                })
            }
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
                text_angle_type,
            },
            style_name,
            override_mask: mask,
            content,
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
        if !ml.block_attributes.is_empty() {
            reasons.push("multileader block attributes are not applied".into());
        }
        if kind == MLeaderContentKind::Block && has_text {
            reasons.push(
                "a multileader that carries both block and text content is drawn as its block \
                 content (DXF entity 172 names one kind)"
                    .into(),
            );
        }
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
