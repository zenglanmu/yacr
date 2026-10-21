//! Dimension and LEADER entry points: subtype dispatch, text placement and the
//! unsupported/degenerate fallbacks.
//!
//! Submodule of [`crate::dimension`].
//!
//! The seven modelled subtypes (ADR 0017 A.1/A.3) map to the domain
//! [`DimensionGeometry`] via [`DimensionGeometry::synthesized`], so the
//! block-less import and the creation path share one display builder (ADR 0017
//! A.4 invariant 1). Arc-length and large-radial dimensions are **not** in the
//! ADR kind set; they keep the pre-P5a compound synthesis (declared in the
//! capability table), using the shared synthesis helpers the domain exposes.

use super::*;
use acadrust::entities::{Dimension, DimensionBase, Leader, LeaderPathType};

impl ImporterBuilder<'_> {
    fn dimension_text(
        &self,
        content: &str,
        position: Point3,
        rotation: f64,
        style: &DimensionStyleValues,
    ) -> SemanticGeometry {
        SemanticGeometry::Text {
            text: content.to_string(),
            position,
            style: style.text_style,
            height: style.text_height,
            rotation,
            font: style.text_font.clone(),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
            // Dimension text has no source width boundary.
            wrap_width: None,
        }
    }

    /// Preferred text anchor: the entity's explicit middle point when set, else
    /// the placement the geometry derives from the dimension line.
    fn place_dimension_text(
        &self,
        base: &DimensionBase,
        geometry: &SynthesizedDimension,
        style: &DimensionStyleValues,
        prefix: &str,
        suffix: &str,
    ) -> SemanticGeometry {
        let explicit = p3(base.text_middle_point);
        let position = if cad_geometry::length(explicit) > 1e-9 {
            explicit
        } else {
            geometry.text_position
        };
        let rotation = if base.text_rotation.abs() > 1e-12 {
            base.text_rotation
        } else {
            geometry.text_rotation
        };
        let content = resolve_measurement_text(
            base.text_override(),
            prefix,
            suffix,
            geometry.measurement,
            style,
        );
        self.dimension_text(&content, position, rotation, style)
    }

    fn unsupported_dimension(
        base: &DimensionBase,
        reason: String,
    ) -> (SemanticGeometry, Completeness) {
        let kind = base.dimension_type;
        (
            SemanticGeometry::Opaque {
                type_key: "AcDbDimension".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec![reason, format!("dimension subtype {kind:?}")]),
        )
    }

    /// Convert a dimension into drawable geometry. The 2D synthesis assumes the
    /// world-XY plane; a tilted plane is reported `Partial` instead of being
    /// flattened incorrectly.
    ///
    /// When the source carries a pre-rendered anonymous block (`*D...`), its
    /// expanded members become the dimension's `display` via
    /// [`DimensionGeometry::from_picture`] (ADR 0017 A.6). Otherwise the display
    /// is our shared synthesis.
    pub(crate) fn dimension_semantics(
        &mut self,
        d: &Dimension,
    ) -> (SemanticGeometry, Completeness) {
        let base = d.base();
        let normal = p3(base.normal);
        if !cad_geometry::is_finite(normal) || (normal.z.abs() - 1.0).abs() > 1e-6 {
            return Self::unsupported_dimension(
                base,
                "dimension lies off the world-XY plane; synthesis only handles XY".into(),
            );
        }
        let style = self.dim_style_values(&base.style_name);
        let has_block =
            !base.block_name.is_empty() && self.block_ids.contains_key(&base.block_name);
        if has_block {
            // Recursion guard (review B3): a `*D` block may contain a DIMENSION
            // whose own block reference is already on the expansion stack
            // (directly self-referential, or through a cycle). Without this the
            // member conversion would recurse without bound and overflow the
            // stack. The export side already guards with a block stack
            // (`export/entity.rs`); this is the import-side equivalent. Beyond
            // the guard the whole entity falls back to `Opaque` + `Partial`,
            // never a panic and never a silent success.
            if self
                .dimension_picture_stack
                .iter()
                .any(|name| name == &base.block_name)
            {
                return Self::unsupported_dimension(
                    base,
                    "dimension picture block recursion (cycle) detected; block not expanded".into(),
                );
            }
            if self.dimension_picture_stack.len() >= self.request.limits.max_block_depth {
                return Self::unsupported_dimension(
                    base,
                    "dimension picture block nesting exceeds the import block-depth limit".into(),
                );
            }
            let Some(definition) = dimension_definition(d) else {
                return Self::unsupported_dimension(
                    base,
                    "dimension subtype is not modelled by the domain variant".into(),
                );
            };
            let text_override = base.text_override().map(str::to_string);
            let (members, member_completeness) = match self.dimension_picture_members(base) {
                Ok(result) => result,
                Err(reason) => return Self::unsupported_dimension(base, reason),
            };
            return match DimensionGeometry::from_picture(
                definition,
                base.style_name.clone(),
                style,
                text_override,
                members,
            ) {
                Ok(geometry) => {
                    // N9: fold in the members' own completeness so a picture
                    // built from lossy members is not reported `Complete`. P5b:
                    // also fold in a declared non-decimal DIMLUNIT degradation.
                    let completeness =
                        member_completeness.combine(unit_degradation(geometry.style()));
                    (
                        SemanticGeometry::Dimension(Box::new(geometry)),
                        completeness,
                    )
                }
                Err(_) => {
                    Self::unsupported_dimension(base, "dimension points are degenerate".into())
                }
            };
        }
        if let Some(definition) = dimension_definition(d) {
            let text_override = base.text_override().map(str::to_string);
            return match DimensionGeometry::synthesized(
                definition,
                base.style_name.clone(),
                style,
                text_override,
            ) {
                Ok(geometry) => {
                    let completeness = unit_degradation(geometry.style());
                    (
                        SemanticGeometry::Dimension(Box::new(geometry)),
                        completeness,
                    )
                }
                Err(_) => {
                    Self::unsupported_dimension(base, "dimension points are degenerate".into())
                }
            };
        }
        self.dimension_semantics_unmodelled(d, base, &style)
    }

    /// Expand the anonymous `*D...` picture block into the dimension's display
    /// members, placed by the dimension's block-insertion transform.
    ///
    /// Returns `Err(reason)` when a member cannot be placed under the transform
    /// (for example a `Spline`/`Shape`/`Opaque`); the caller then refuses the
    /// whole entity rather than store a partial picture (ADR 0017 A.5). The
    /// members' combined completeness is returned alongside them so a lossy
    /// picture is not reported `Complete` (review N9).
    ///
    /// The block name is pushed onto [`ImporterBuilder::dimension_picture_stack`]
    /// before the members are converted, so a member dimension that references a
    /// block already being expanded is caught by the guard in
    /// [`ImporterBuilder::dimension_semantics`] instead of recursing (review B3).
    fn dimension_picture_members(
        &mut self,
        base: &DimensionBase,
    ) -> Result<(Vec<SemanticGeometry>, Completeness), String> {
        let name = base.block_name.clone();
        let transform = dimension_transform(base);
        let entities: Vec<EntityType> = self.acad.entities_in_block(&name).cloned().collect();
        self.dimension_picture_stack.push(name);
        let mut members = Vec::with_capacity(entities.len());
        let mut completeness = Completeness::Complete;
        let mut failure: Option<String> = None;
        for entity in entities {
            let common = entity.common().clone();
            let (geometry, member_completeness) = self.convert(&entity, &common);
            match place_dimension_member(geometry, &transform) {
                Some(placed) => {
                    members.push(placed);
                    completeness = completeness.combine(member_completeness);
                }
                None => {
                    failure =
                        Some("dimension picture block has geometry that cannot be placed".into());
                    break;
                }
            }
        }
        self.dimension_picture_stack.pop();
        match failure {
            Some(reason) => Err(reason),
            None => Ok((members, completeness)),
        }
    }

    /// The pre-P5a compound synthesis for the subtypes the domain variant does
    /// not model (arc-length and large-radial).
    fn dimension_semantics_unmodelled(
        &self,
        d: &Dimension,
        base: &DimensionBase,
        style: &DimensionStyleValues,
    ) -> (SemanticGeometry, Completeness) {
        let (geometry, prefix, suffix, completeness) = match d {
            Dimension::Arc(a) => (
                arc_length_geometry(
                    p3(a.center_point),
                    p3(a.first_extension_point),
                    p3(a.second_extension_point),
                    a.arc_start_parameter,
                    a.arc_end_parameter,
                    p3(a.base.normal),
                    style,
                    a.has_leader
                        .then(|| (p3(a.first_leader_point), p3(a.second_leader_point))),
                ),
                "",
                "",
                if a.is_partial {
                    Completeness::Partial(vec![
                        "partial arc dimension; partial-arc marker/leader arrangement not reproduced"
                            .into(),
                    ])
                } else {
                    Completeness::Complete
                },
            ),
            Dimension::LargeRadial(lr) => {
                let override_center = p3(lr.override_center);
                let center = if cad_geometry::length(override_center) > 1e-9 {
                    override_center
                } else {
                    p3(lr.definition_point)
                };
                (
                    large_radial_geometry(center, p3(lr.chord_point), p3(lr.jog_point), lr.jog_angle),
                    "R",
                    "",
                    Completeness::Partial(vec![
                        "large-radial jog is oriented by jog_angle; AutoCAD's true zigzag jog and text \
                         landing are not reproduced"
                            .into(),
                    ]),
                )
            }
            _ => {
                return Self::unsupported_dimension(base, "dimension points are degenerate".into());
            }
        };
        let Some(geometry) = geometry else {
            return Self::unsupported_dimension(base, "dimension points are degenerate".into());
        };
        let text = self.place_dimension_text(base, &geometry, style, prefix, suffix);
        let mut children = geometry.children;
        children.push(text);
        (SemanticGeometry::Compound(children), completeness)
    }

    /// Convert a LEADER into its polyline path plus the arrowhead at the first
    /// vertex (the arrow point). A spline path is drawn as straight segments and
    /// reported `Partial`; an enabled hookline is reported rather than guessed.
    pub(crate) fn leader_semantics(&self, l: &Leader) -> (SemanticGeometry, Completeness) {
        let points: Vec<Point3> = l.vertices.iter().map(|v| p3(*v)).collect();
        if points.len() < 2 || !points.iter().all(|p| cad_geometry::is_finite(*p)) {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["leader has fewer than two valid vertices".into()]),
            );
        }
        let normal = p3(l.normal);
        if !cad_geometry::is_finite(normal) || (normal.z.abs() - 1.0).abs() > 1e-6 {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec![
                    "leader lies off the world-XY plane; synthesis only handles XY".into(),
                ]),
            );
        }
        let mut completeness = Completeness::Complete;
        if l.path_type == LeaderPathType::Spline {
            completeness =
                Completeness::Partial(vec!["leader spline path drawn as straight segments".into()]);
        }
        if l.hookline_enabled {
            completeness = completeness.combine(Completeness::Partial(vec![
                "leader hookline is not drawn".into(),
            ]));
        }
        let mut children = vec![SemanticGeometry::Polyline {
            points: points.clone(),
            bulges: Vec::new(),
            closed: false,
        }];
        if l.arrow_enabled {
            let style = self.dim_style_values(&l.dimension_style);
            let outward = cad_geometry::sub(points[0], points[1]);
            if style.arrow_size > 1e-9 && cad_geometry::length(outward) > 1e-9 {
                children.push(arrowhead(points[0], outward, style.arrow_size));
            }
        }
        (SemanticGeometry::Compound(children), completeness)
    }
}

/// The declared degradation of a non-decimal `DIMLUNIT`, folded into the
/// dimension's completeness so a scientific/architectural/fractional unit is
/// reported `Partial` rather than silently rendered as decimal (ADR 0017 F.4).
fn unit_degradation(style: &DimensionStyleValues) -> Completeness {
    match style.measurement_unit_degradation() {
        Some(reason) => Completeness::Partial(vec![reason]),
        None => Completeness::Complete,
    }
}

/// Place one expanded picture-block member under the dimension's block
/// insertion transform.
///
/// `Text` is placed here because the general transform surface refuses a
/// standalone `Text` entity; the other kinds use the shared transform. A kind
/// the shared transform refuses yields `None`, so the caller refuses the whole
/// dimension (never a partially placed picture).
fn place_dimension_member(
    geometry: SemanticGeometry,
    transform: &Transform3,
) -> Option<SemanticGeometry> {
    if let SemanticGeometry::Text {
        text,
        position,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
    } = geometry
    {
        let scale = transform.max_scale();
        if !scale.is_finite() {
            return None;
        }
        let v = transform.apply_vector(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        let delta = v.y.atan2(v.x);
        return Some(SemanticGeometry::Text {
            text,
            position: transform.apply_point(position),
            style,
            height: height * scale,
            rotation: rotation + delta,
            font,
            h_align,
            v_align,
            // Keep the boundary on the same footing as `height`: it is a
            // world-space length, so the placement scale applies to it.
            wrap_width: wrap_width.map(|width| width * scale),
        });
    }
    cad_db::transform_geometry(&geometry, transform).ok()
}

/// Map an acadrust dimension to the domain definition points (ADR 0017 A.3).
///
/// Returns `None` for the subtypes the domain variant does not model
/// (arc-length, large-radial).
fn dimension_definition(d: &Dimension) -> Option<DimensionDefinition> {
    let base = d.base();
    let text_middle = p3(base.text_middle_point);
    let text_rotation = base.text_rotation;
    let normal = p3(base.normal);
    let definition = match d {
        Dimension::Aligned(a) => DimensionDefinition {
            kind: DimensionKind::Aligned,
            points: DimensionPoints::Linear {
                first: p3(a.first_point),
                second: p3(a.second_point),
                dimension_line: p3(a.definition_point),
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Linear(l) => DimensionDefinition {
            kind: DimensionKind::Linear {
                rotation: l.rotation,
            },
            points: DimensionPoints::Linear {
                first: p3(l.first_point),
                second: p3(l.second_point),
                dimension_line: p3(l.definition_point),
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Radius(r) => DimensionDefinition {
            kind: DimensionKind::Radius,
            points: DimensionPoints::Radial {
                center: p3(r.angle_vertex),
                point_on_arc: p3(r.definition_point),
                leader_length: r.leader_length,
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Diameter(dd) => DimensionDefinition {
            kind: DimensionKind::Diameter,
            points: DimensionPoints::Radial {
                // ADR 0017 A.3 group order: acadrust's reader fills
                // `definition_point` from group 10 (the chord point's
                // **opposite**) and `angle_vertex` from group 15 (the first
                // chord point), so the domain's group-10 slot (`center`) takes
                // `definition_point` and `point_on_arc` takes `angle_vertex`.
                center: p3(dd.definition_point),
                point_on_arc: p3(dd.angle_vertex),
                leader_length: dd.leader_length,
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Angular2Ln(a) => DimensionDefinition {
            kind: DimensionKind::Angular2Ln,
            points: DimensionPoints::Angular2Ln {
                line1_start: p3(a.first_point),
                line1_end: p3(a.second_point),
                line2_start: p3(a.angle_vertex),
                line2_end: p3(a.definition_point),
                arc_point: p3(a.dimension_arc),
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Angular3Pt(a) => DimensionDefinition {
            kind: DimensionKind::Angular3Pt,
            points: DimensionPoints::Angular3Pt {
                vertex: p3(a.angle_vertex),
                first: p3(a.first_point),
                second: p3(a.second_point),
                arc_point: p3(a.definition_point),
            },
            text_middle,
            text_rotation,
            normal,
        },
        Dimension::Ordinate(o) => DimensionDefinition {
            kind: DimensionKind::Ordinate {
                x_ordinate: o.is_ordinate_type_x,
            },
            points: DimensionPoints::Ordinate {
                feature: p3(o.feature_location),
                leader_end: p3(o.leader_endpoint),
                base: p3(o.definition_point),
            },
            text_middle,
            text_rotation,
            normal,
        },
        _ => return None,
    };
    Some(definition)
}
