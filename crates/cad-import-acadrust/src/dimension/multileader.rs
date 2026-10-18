//! MULTILEADER path, arrowhead, dogleg and annotation-text synthesis.
//!
//! Submodule of [`crate::dimension`].

use super::*;
use acadrust::entities::{
    LeaderLinePropertyOverrideFlags, MultiLeaderPathType, MultiLeaderPropertyOverrideFlags,
    TextAttachmentPointType, TextAttachmentType,
};

/// Build a MULTILEADER leader path honoring its `path_type`.
///
/// `Invisible` draws nothing, `StraightLineSegments` a polyline, and `Spline` a
/// smooth B-spline through the same points (tessellated by the representation
/// layer), never a straight polyline.
fn multileader_path(points: &[Point3], path_type: MultiLeaderPathType) -> Option<SemanticGeometry> {
    match path_type {
        MultiLeaderPathType::Invisible => None,
        MultiLeaderPathType::StraightLineSegments => {
            Some(polyline_semantics(points.to_vec(), false))
        }
        MultiLeaderPathType::Spline => {
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

/// The dogleg / landing segment at the content end of a leader root.
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

impl ImporterBuilder<'_> {
    /// Convert a MULTILEADER: its leader-root paths (honoring `path_type`),
    /// arrowheads, dogleg/landing and annotation text.
    ///
    /// The MLEADERSTYLE object (reached through `ml.style_handle`, since styles
    /// are not in a typed table) supplies the defaults for path type, arrowhead
    /// size and the dogleg/landing; per-entity and per-line override flags win.
    /// The result stays `Partial`: the representation carries one colour per
    /// entity, so line colour/lineweight/linetype and property overrides are
    /// resolved but not applied, and background fill, columns, block attributes
    /// and the transform matrix are enumerated rather than faked.
    pub(crate) fn multileader_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        let context = &ml.context;
        // MLEADER styles live in the document's object map, not a typed table.
        let style = ml.style_handle.and_then(|handle| {
            if handle.is_null() {
                return None;
            }
            match self.acad.objects.get(&handle) {
                Some(acadrust::objects::ObjectType::MultiLeaderStyle(style)) => Some(style),
                _ => None,
            }
        });

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
                Some(style) => MultiLeaderPathType::from(style.path_type as i16),
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
                if path_type == MultiLeaderPathType::Invisible {
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
}
