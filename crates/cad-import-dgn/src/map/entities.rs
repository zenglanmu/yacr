//! Entity conversion and insertion for the DGN mapper.

use cad_db::{
    validate_geometry, DbEntity, DbObject, EntityColor, EntityLineType, EntityLineWeight,
    EntityRenderAttributes, LinetypePattern,
};
use cad_domain::{
    CadError, CadResult, EntityId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
    TextAlignH, TextAlignV,
};
use cadkit_core::{
    Attachment, Color as KitColor, Entity, EntityKind, HAlign, Lineweight as KitLineweight, VAlign,
};

use super::{domain_point, domain_vec, Inherit, LinetypeChoice, Mapped, Mapper, Resolved};
use crate::transform::{conic_sweep, insert_transform, is_world_z, length, normal_or_z};
use crate::{BLOCK_MISSING, ENTITY_LIMIT, GEOMETRY_REJECTED, SPLINE_FIT_POINTS};

impl Mapper<'_> {
    pub(super) fn resolve_style(
        &mut self,
        entity: &Entity,
        inherit: &Inherit,
    ) -> CadResult<Resolved> {
        let layer_name = entity.layer.clone().or_else(|| inherit.layer_name.clone());
        let layer = self.layer_id(layer_name.as_deref())?;
        let layer_name = self
            .layer_names
            .get(&layer)
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        let color = match entity.color {
            KitColor::ByLayer => inherit.color,
            KitColor::ByBlock => EntityColor::ByBlock,
            // The DGN reader resolves its palette to `Rgb`; an ACI index here
            // has no ACI->sRGB table in this crate, so it stays symbolic
            // instead of inventing a colour.
            KitColor::Aci { .. } => EntityColor::ByLayer,
            KitColor::Rgb { r, g, b } => EntityColor::Explicit([r, g, b]),
        };
        let linetype = match &entity.linetype {
            None => inherit.linetype.clone(),
            Some(name) if name.eq_ignore_ascii_case("bylayer") => inherit.linetype.clone(),
            Some(name) if name.eq_ignore_ascii_case("byblock") => LinetypeChoice::ByBlock,
            Some(name) => LinetypeChoice::Named(name.clone()),
        };
        let lineweight = match entity.lineweight {
            KitLineweight::ByLayer => inherit.lineweight,
            KitLineweight::ByBlock => EntityLineWeight::ByBlock,
            KitLineweight::Default => EntityLineWeight::Default,
            KitLineweight::Millimeters(value) if value.is_finite() && value >= 0.0 => {
                EntityLineWeight::Explicit(value as f32)
            }
            KitLineweight::Millimeters(_) => EntityLineWeight::Default,
        };
        Ok(Resolved {
            layer,
            layer_name,
            color,
            linetype,
            lineweight,
            visible: entity.visible && inherit.visible,
        })
    }

    pub(super) fn map_one(
        &mut self,
        entity: &Entity,
        space: &SpaceId,
        inherit: &Inherit,
    ) -> CadResult<Option<EntityId>> {
        if self.inserted >= self.max_entities {
            return Err(CadError::InvalidInput(format!(
                "{ENTITY_LIMIT}: the DGN file has more drawable entities than the {} entity \
                 import limit",
                self.max_entities
            )));
        }
        let object = ObjectId(self.next_object);
        self.next_object += 1;
        let resolved = self.resolve_style(entity, inherit)?;
        let Some(mapped) = self.geometry(entity, &resolved, object)? else {
            return Ok(None);
        };
        if let Err(error) = validate_geometry(&mapped.geometry) {
            self.rejected += 1;
            self.note(
                GEOMETRY_REJECTED,
                format!(
                    "{} was skipped: cad-db refused its geometry ({error})",
                    mapped.type_key
                ),
                Some(object),
            );
            return Ok(None);
        }

        // `ObjectId` and `EntityId` share the numeric value by contract.
        let id = EntityId(object.0);
        let linetype = self.render_linetype(&resolved.linetype);
        let attributes = EntityRenderAttributes {
            clip: None,
            color: resolved.color,
            lineweight: resolved.lineweight,
            linetype,
            linetype_scale: 1.0,
            ..Default::default()
        };
        let type_key = mapped.type_key;
        self.builder.insert_entity(DbEntity {
            object: DbObject {
                id: object,
                type_key: type_key.to_string(),
                revision: Revision(0),
                source_handle: entity.id.map(|id| format!("{id:X}")),
            },
            id,
            layer: resolved.layer,
            space: space.clone(),
            geometry: mapped.geometry,
            draw_order: self.draw_order,
        })?;
        self.builder.set_entity_render_attributes(id, attributes)?;
        self.draw_order += 1;
        self.inserted += 1;
        if !resolved.visible {
            self.hidden.push(id);
        }
        let partial = self.kinds.entry(type_key).or_insert(false);
        *partial |= mapped.partial;
        Ok(Some(id))
    }

    fn render_linetype(&mut self, choice: &LinetypeChoice) -> EntityLineType {
        match choice {
            LinetypeChoice::ByLayer => EntityLineType::ByLayer,
            LinetypeChoice::ByBlock => EntityLineType::ByBlock,
            LinetypeChoice::Named(name) => {
                let key = name.to_ascii_lowercase();
                match self.linetype_patterns.get(&key) {
                    Some(pattern) => EntityLineType::Explicit {
                        name: name.clone(),
                        pattern: pattern.clone(),
                        scale: 1.0,
                    },
                    None => {
                        self.unknown_linetypes.insert(name.clone());
                        EntityLineType::Explicit {
                            name: name.clone(),
                            pattern: LinetypePattern::continuous(),
                            scale: 1.0,
                        }
                    }
                }
            }
        }
    }

    /// Convert one source entity into neutral geometry.
    ///
    /// `Ok(None)` means the entity was skipped and a diagnostic was recorded;
    /// it is never silently ignored.
    fn geometry(
        &mut self,
        entity: &Entity,
        resolved: &Resolved,
        object: ObjectId,
    ) -> CadResult<Option<Mapped>> {
        let mapped = match &entity.kind {
            EntityKind::Point { position } => Mapped {
                geometry: SemanticGeometry::Point(domain_point(*position)),
                partial: false,
                type_key: "DgnPoint",
            },
            EntityKind::Line { start, end } => Mapped {
                geometry: SemanticGeometry::Line {
                    start: domain_point(*start),
                    end: domain_point(*end),
                },
                partial: false,
                type_key: "DgnLine",
            },
            EntityKind::Polyline {
                vertices,
                closed,
                normal,
            } => {
                let points: Vec<Point3> = vertices
                    .iter()
                    .map(|vertex| domain_point(vertex.position))
                    .collect();
                let bulges: Vec<f64> = vertices.iter().map(|vertex| vertex.bulge).collect();
                let has_bulge = bulges.iter().any(|bulge| *bulge != 0.0);
                let plane_loss = has_bulge && !is_world_z(domain_vec(*normal));
                Mapped {
                    geometry: SemanticGeometry::Polyline {
                        points,
                        bulges: if has_bulge { bulges } else { Vec::new() },
                        closed: *closed,
                        width: 0.0,
                    },
                    partial: plane_loss,
                    type_key: "DgnPolyline",
                }
            }
            EntityKind::Circle {
                center,
                radius,
                normal,
            } => Mapped {
                geometry: SemanticGeometry::Circle {
                    center: domain_point(*center),
                    normal: normal_or_z(domain_vec(*normal)),
                    radius: *radius,
                },
                partial: false,
                type_key: "DgnCircle",
            },
            EntityKind::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                normal,
            } => {
                let sweep = conic_sweep(*start_angle, *end_angle);
                if !sweep.is_finite() || sweep.abs() < 1e-12 {
                    self.reject(object, "arc has a zero sweep after normalisation");
                    return Ok(None);
                }
                Mapped {
                    geometry: SemanticGeometry::Arc {
                        center: domain_point(*center),
                        normal: normal_or_z(domain_vec(*normal)),
                        radius: *radius,
                        start: *start_angle,
                        sweep,
                    },
                    partial: false,
                    type_key: "DgnArc",
                }
            }
            EntityKind::Ellipse {
                center,
                major_axis,
                ratio,
                start_param,
                end_param,
                normal,
            } => {
                let normal = normal_or_z(domain_vec(*normal));
                let major = domain_vec(*major_axis);
                let major_len = length(major);
                let perpendicular = crate::transform::cross(normal, major);
                if !ratio.is_finite()
                    || *ratio <= 0.0
                    || *ratio > 1.0 + 1e-9
                    || major_len < 1e-12
                    || length(perpendicular) < 1e-12
                {
                    self.reject(
                        object,
                        "ellipse has an unusable axis ratio, collapsed major axis or a normal \
                         parallel to that axis",
                    );
                    return Ok(None);
                }
                Mapped {
                    geometry: SemanticGeometry::Ellipse {
                        center: domain_point(*center),
                        normal,
                        major_axis: major,
                        ratio: ratio.min(1.0),
                        start: *start_param,
                        sweep: conic_sweep(*start_param, *end_param),
                    },
                    partial: false,
                    type_key: "DgnEllipse",
                }
            }
            EntityKind::Spline {
                degree,
                knots,
                control_points,
                weights,
                fit_points,
                closed,
            } => {
                if control_points.len() > *degree as usize {
                    Mapped {
                        geometry: SemanticGeometry::Spline {
                            degree: *degree,
                            knots: knots.clone(),
                            control_points: control_points
                                .iter()
                                .map(|point| domain_point(*point))
                                .collect(),
                            weights: weights.clone(),
                        },
                        partial: false,
                        type_key: "DgnSpline",
                    }
                } else if fit_points.len() >= 2 {
                    self.note(
                        SPLINE_FIT_POINTS,
                        format!(
                            "spline has {} fit point(s) but no usable control polygon; it was \
                             stored as a polyline through the fit points",
                            fit_points.len()
                        ),
                        Some(object),
                    );
                    Mapped {
                        geometry: SemanticGeometry::Polyline {
                            points: fit_points
                                .iter()
                                .map(|point| domain_point(*point))
                                .collect(),
                            bulges: Vec::new(),
                            closed: *closed,
                            width: 0.0,
                        },
                        partial: true,
                        type_key: "DgnPolyline",
                    }
                } else {
                    self.reject(
                        object,
                        "spline has neither a control polygon nor fit points",
                    );
                    return Ok(None);
                }
            }
            EntityKind::Text {
                position,
                height,
                rotation,
                width_factor,
                value,
                style,
                halign,
                valign,
                normal,
                ..
            } => {
                if !height.is_finite() || *height <= 0.0 {
                    self.reject(object, "text height is not positive and finite");
                    return Ok(None);
                }
                let (h_align, align_loss) = map_halign(*halign);
                if align_loss {
                    self.text_align_losses += 1;
                }
                let plane_loss = !is_world_z(domain_vec(*normal));
                if plane_loss {
                    self.text_plane_losses += 1;
                }
                let (width_factor, width_loss) = if width_factor.is_finite() && *width_factor > 0.0
                {
                    (*width_factor, false)
                } else {
                    self.text_width_losses += 1;
                    (cad_domain::DEFAULT_TEXT_WIDTH_FACTOR, true)
                };
                Mapped {
                    geometry: SemanticGeometry::Text {
                        text: value.clone(),
                        position: domain_point(*position),
                        style: self.style_id(style.as_deref())?,
                        height: *height,
                        rotation: *rotation,
                        font: self.style_font(style.as_deref()),
                        h_align,
                        v_align: map_valign(*valign),
                        wrap_width: None,
                        width_factor,
                    },
                    partial: align_loss || plane_loss || width_loss,
                    type_key: "DgnText",
                }
            }
            EntityKind::MText {
                position,
                height,
                width,
                rotation,
                plain,
                style,
                attachment,
                normal,
                ..
            } => {
                if !height.is_finite() || *height <= 0.0 {
                    self.reject(object, "multiline text height is not positive and finite");
                    return Ok(None);
                }
                let (h_align, v_align) = map_attachment(*attachment);
                self.mtext_flattened += 1;
                let plane_loss = !is_world_z(domain_vec(*normal));
                if plane_loss {
                    self.text_plane_losses += 1;
                }
                Mapped {
                    geometry: SemanticGeometry::Text {
                        text: plain.clone(),
                        position: domain_point(*position),
                        style: self.style_id(style.as_deref())?,
                        height: *height,
                        rotation: *rotation,
                        font: self.style_font(style.as_deref()),
                        h_align,
                        v_align,
                        wrap_width: cad_domain::wrap_width_from_dxf(*width),
                        width_factor: cad_domain::DEFAULT_TEXT_WIDTH_FACTOR,
                    },
                    // The stored markup is flattened to its plain text, so the
                    // row is always partial; `plane_loss` has its own diagnostic
                    // and does not need to widen this flag.
                    partial: true,
                    type_key: "DgnText",
                }
            }
            EntityKind::Insert {
                block,
                position,
                scale,
                rotation,
                normal,
                ..
            } => {
                let Some((block, base)) = self.block(block) else {
                    self.missing_blocks += 1;
                    self.note(
                        BLOCK_MISSING,
                        format!(
                            "shared cell instance names block '{block}', which has no definition \
                             in this file; the instance was skipped rather than expanded as an \
                             empty block"
                        ),
                        Some(object),
                    );
                    return Ok(None);
                };
                if self.is_active_block(block) {
                    self.note_depth_limit();
                    return Ok(None);
                }
                let transform = insert_transform(
                    base,
                    domain_point(*position),
                    domain_vec(*scale),
                    *rotation,
                    domain_vec(*normal),
                );
                let determinant = transform.determinant();
                let mirrored = determinant.is_finite() && determinant < 0.0;
                if mirrored {
                    self.mirrored_cells += 1;
                }
                Mapped {
                    geometry: SemanticGeometry::Insert { block, transform },
                    partial: mirrored,
                    type_key: "DgnInsert",
                }
            }
            EntityKind::Image {
                path,
                position,
                u_vector,
                v_vector,
                size_px,
            } => {
                self.rasters += 1;
                let pixels = size_px
                    .filter(|pixels| pixels[0] > 0 && pixels[1] > 0)
                    .map(|pixels| [f64::from(pixels[0]), f64::from(pixels[1])]);
                let origin = domain_point(*position);
                let u = domain_vec(*u_vector);
                let v = domain_vec(*v_vector);
                match pixels {
                    Some(pixels) if length(u) > 0.0 && length(v) > 0.0 => {
                        // cadkit's vectors span the full image; the domain's
                        // `u`/`v` are one pixel's world size.
                        let per_pixel = |vector: Point3, pixels: f64| Point3 {
                            x: vector.x / pixels,
                            y: vector.y / pixels,
                            z: vector.z / pixels,
                        };
                        Mapped {
                            geometry: SemanticGeometry::Image {
                                origin,
                                u: per_pixel(u, pixels[0]),
                                v: per_pixel(v, pixels[1]),
                                pixels,
                                file: path.clone(),
                                clip: None,
                                visible: resolved.visible,
                                adjust: cad_domain::ImageAdjust::default(),
                            },
                            partial: true,
                            type_key: "DgnRaster",
                        }
                    }
                    _ => {
                        let payload = path
                            .as_deref()
                            .map(str::as_bytes)
                            .map(<[u8]>::to_vec)
                            .unwrap_or_default();
                        Mapped {
                            geometry: SemanticGeometry::Opaque {
                                type_key: "DgnRasterReference".into(),
                                version: 1,
                                payload,
                            },
                            partial: true,
                            type_key: "DgnRaster",
                        }
                    }
                }
            }
            EntityKind::Unknown { type_name } => {
                *self.unknown_types.entry(type_name.clone()).or_insert(0) += 1;
                return Ok(None);
            }
            EntityKind::Face { .. } => {
                self.unmodelled("dgn.face", object);
                return Ok(None);
            }
            EntityKind::Polygon { .. } => {
                self.unmodelled("dgn.polygon", object);
                return Ok(None);
            }
            EntityKind::Mesh { .. } => {
                self.unmodelled("dgn.mesh", object);
                return Ok(None);
            }
            EntityKind::Hatch { .. } => {
                self.unmodelled("dgn.hatch", object);
                return Ok(None);
            }
            EntityKind::Leader { .. } => {
                self.unmodelled("dgn.leader", object);
                return Ok(None);
            }
            EntityKind::Viewport { .. } => {
                self.unmodelled("dgn.viewport", object);
                return Ok(None);
            }
            EntityKind::Dimension { .. } => {
                self.unmodelled("dgn.dimension", object);
                return Ok(None);
            }
            // `EntityKind` is `#[non_exhaustive]`: a future variant must be
            // reported, never `unreachable!()`.
            _ => {
                self.unmodelled("dgn.unknown_kind", object);
                return Ok(None);
            }
        };
        Ok(Some(mapped))
    }

    fn unmodelled(&mut self, label: &str, object: ObjectId) {
        *self.unmodelled_types.entry(label.to_string()).or_insert(0) += 1;
        self.note(
            crate::ELEMENT_NOT_MODELLED,
            format!("{label} has no cad-db vector mapping and was skipped"),
            Some(object),
        );
    }

    fn reject(&mut self, object: ObjectId, reason: &str) {
        self.rejected += 1;
        self.note(
            GEOMETRY_REJECTED,
            format!("element was skipped: {reason}"),
            Some(object),
        );
    }
}

fn map_halign(align: HAlign) -> (TextAlignH, bool) {
    match align {
        HAlign::Left => (TextAlignH::Left, false),
        HAlign::Center => (TextAlignH::Center, false),
        HAlign::Right => (TextAlignH::Right, false),
        // The domain has no aligned/fit/middle variants; left is the honest
        // neutral fallback and the loss is reported.
        HAlign::Aligned | HAlign::Fit | HAlign::Middle => (TextAlignH::Left, true),
    }
}

fn map_valign(align: VAlign) -> TextAlignV {
    match align {
        VAlign::Baseline => TextAlignV::Baseline,
        VAlign::Bottom => TextAlignV::Bottom,
        VAlign::Middle => TextAlignV::Middle,
        VAlign::Top => TextAlignV::Top,
    }
}

fn map_attachment(attachment: Attachment) -> (TextAlignH, TextAlignV) {
    use Attachment as A;
    let h = match attachment {
        A::TopLeft | A::MiddleLeft | A::BottomLeft => TextAlignH::Left,
        A::TopCenter | A::MiddleCenter | A::BottomCenter => TextAlignH::Center,
        A::TopRight | A::MiddleRight | A::BottomRight => TextAlignH::Right,
    };
    let v = match attachment {
        A::TopLeft | A::TopCenter | A::TopRight => TextAlignV::Top,
        A::MiddleLeft | A::MiddleCenter | A::MiddleRight => TextAlignV::Middle,
        A::BottomLeft | A::BottomCenter | A::BottomRight => TextAlignV::Bottom,
    };
    (h, v)
}
