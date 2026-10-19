//! Entity insertion into the database plus per-class capability accounting.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    pub(crate) fn push_entity(
        &mut self,
        entity: &EntityType,
        space: SpaceId,
        _layout: LayoutId,
    ) -> Option<EntityId> {
        if matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            return None;
        }
        if self.entity_total >= self.request.limits.max_entities {
            self.dropped += 1;
            return None;
        }
        let common = entity.common();
        let class_name = entity_class_name(entity);
        let layer = self
            .layer_ids
            .get(&common.layer)
            .copied()
            .unwrap_or_else(|| self.layer_ids.get("0").copied().unwrap_or(LayerId(0)));

        let (geometry, completeness) = self.convert(entity, common);

        // Track the model-space extent as entities are imported so RAY/XLINE
        // construct lines can be clipped to it (a fixed viewport has no clip).
        if matches!(space, SpaceId::Model) {
            let mut accumulator = cad_geometry::BoundsAccumulator::new();
            if let Some((min, max)) = self.model_bounds {
                accumulator.union_point(min);
                accumulator.union_point(max);
            }
            accumulator.add_geometry(&geometry);
            if let Some(bounds) = accumulator.finish() {
                self.model_bounds = Some((bounds.min, bounds.max));
            }
        }

        // Render/pick are judged from the drawn result, not from parse success
        // (audit B20). An INSERT inherits the resolved status of its block; an
        // array INSERT is a Compound of Instances that all reference one block.
        let (mut render, mut pick) = display_support(&geometry);
        let insert_blocks = referenced_blocks(&geometry);
        for block in &insert_blocks {
            if let Some(status) = self.block_status.get(block) {
                render = weaker(render, *status);
                pick = weaker(pick, *status);
            } else {
                render = weaker(render, SupportStatus::Unverified);
                pick = weaker(pick, SupportStatus::Unverified);
            }
        }
        if let SpaceId::Block(block) = &space {
            self.block_members
                .entry(*block)
                .or_default()
                .push(BlockMember {
                    render,
                    insert_blocks,
                });
        }
        if matches!(&space, SpaceId::Model) {
            self.model_render = weaker(self.model_render, render);
            if render == SupportStatus::Verified {
                self.model_drawable = true;
            } else {
                self.model_render_types.insert(class_name.clone());
            }
        }

        let entity_id = EntityId(self.next_entity);
        let object_id = ObjectId(self.next_object);
        self.next_entity += 1;
        self.next_object += 1;
        self.entity_total += 1;

        self.note_capability(&class_name, &geometry, &completeness, render, pick);

        // Resolve the entity's effective display opacity, colour, lineweight
        // and linetype so the representation/scene layers can carry real style
        // instead of always drawing opaque grey (audit F14 / §2.1.3). ByObject
        // wins over ByLayer; ByBlock is kept symbolic for INSERT expansion.
        let layer_alpha = self.layer_transparency.get(&layer).copied().unwrap_or(1.0);
        let layer_color = self.layer_colors.get(&layer).copied();
        let layer_lineweight = self.layer_lineweights.get(&layer).copied();
        let layer_pattern = self
            .layer_linetypes
            .get(&layer)
            .and_then(|name| self.linetype_pattern(name));
        // An explicit entity linetype is looked up in the drawing's table; a
        // missing name is reported, never silently turned into dashes. R13/R14
        // entities may carry only a handle, so fall back to the table's handle
        // index when the name is absent.
        let handle_name = common
            .linetype_handle
            .filter(|h| !h.is_null())
            .and_then(|h| self.linetype_names_by_handle.get(&h.value()).cloned());
        let source_linetype: String = if is_bylayer_linetype(&common.linetype) {
            match handle_name {
                Some(name) => name,
                None => common.linetype.clone(),
            }
        } else {
            common.linetype.clone()
        };
        let named_key = source_linetype.trim().to_ascii_lowercase();
        let named_pattern =
            if is_bylayer_linetype(&source_linetype) || is_byblock_linetype(&source_linetype) {
                None
            } else {
                self.linetype_patterns.get(&named_key).cloned()
            };
        if !named_key.is_empty()
            && !is_bylayer_linetype(&source_linetype)
            && !is_byblock_linetype(&source_linetype)
            && named_pattern.is_none()
        {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.linetype_unknown".into(),
                message: format!(
                    "linetype '{}' is not in the drawing's linetype table; line drawn continuous",
                    source_linetype
                ),
            });
        }
        let resolved_linetype = resolve_entity_linetype(
            &source_linetype,
            common.linetype_scale,
            named_pattern.clone(),
            layer_pattern.clone(),
        );
        // A `ByLayer` entity whose layer names a non-continuous linetype that is
        // missing from the table cannot resolve; report rather than silently
        // drawing the layer solid.
        if is_bylayer_linetype(&source_linetype) {
            if let Some(layer_name) = self.layer_linetypes.get(&layer) {
                let key = layer_name.trim().to_ascii_lowercase();
                if !key.is_empty()
                    && !is_continuous_linetype(layer_name)
                    && !self.linetype_patterns.contains_key(&key)
                {
                    self.diagnostics.push(Diagnostic {
                        object: Some(object_id),
                        code: "import.linetype_unknown".into(),
                        message: format!(
                            "layer linetype '{}' is not in the drawing's linetype table; line drawn continuous",
                            layer_name
                        ),
                    });
                }
            }
        }
        // A complex linetype still dashes by its segment lengths, but its
        // shape/text glyphs are not drawn; report the omission.
        let effective_key = if is_bylayer_linetype(&source_linetype) {
            self.layer_linetypes
                .get(&layer)
                .map(|n| n.trim().to_ascii_lowercase())
        } else if is_byblock_linetype(&source_linetype) {
            None
        } else {
            Some(named_key)
        };
        if let Some(key) = &effective_key {
            if self.complex_linetypes.contains(key) {
                self.diagnostics.push(Diagnostic {
                    object: Some(object_id),
                    code: "import.linetype_complex".into(),
                    message: format!(
                        "linetype '{key}' has shape/text elements; only its dash segments are drawn"
                    ),
                });
            }
        }
        let (annotative, annotative_reason) =
            annotative_attributes(self.acad, entity, &self.scale_names);
        if let Some(reason) = annotative_reason {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "annotative.context_unsupported".into(),
                message: reason,
            });
        }
        let attributes = EntityRenderAttributes {
            transparency: resolve_entity_transparency(common.transparency, layer_alpha),
            color: resolve_entity_color(common.color, layer_color),
            lineweight: resolve_entity_lineweight(common.line_weight, layer_lineweight),
            linetype: resolved_linetype,
            geometry_source: if proxy_geometry_allowed(entity) {
                GeometrySource::ProxyCache
            } else {
                GeometrySource::Analytic
            },
            annotative,
        };

        let record = DbEntity {
            object: DbObject {
                id: object_id,
                type_key: class_name.clone(),
                revision: Revision(0),
                source_handle: Some(format!("{:X}", common.handle.value())),
            },
            id: entity_id,
            layer,
            space,
            geometry,
            draw_order: self.entity_total as i64,
        };
        if let Err(e) = self.builder.insert_entity(record) {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.insert_failed".into(),
                message: e.to_string(),
            });
            return None;
        }
        if let Err(e) = self
            .builder
            .set_entity_render_attributes(entity_id, attributes)
        {
            // The entity was just inserted, so this cannot fail today; report
            // rather than pretend the transparency was recorded.
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.render_attributes_failed".into(),
                message: e.to_string(),
            });
        }
        Some(entity_id)
    }

    pub(crate) fn note_capability(
        &mut self,
        class_name: &str,
        geometry: &SemanticGeometry,
        completeness: &Completeness,
        render: SupportStatus,
        pick: SupportStatus,
    ) {
        let semantic = match completeness {
            Completeness::Complete => SupportStatus::Verified,
            Completeness::Partial(_) => SupportStatus::Partial,
            Completeness::Missing(_) => SupportStatus::Unsupported,
            Completeness::Unverified => SupportStatus::Unverified,
        };
        let measure = match geometry {
            SemanticGeometry::Mesh(_)
            | SemanticGeometry::Opaque { .. }
            | SemanticGeometry::Text { .. } => SupportStatus::Unsupported,
            // A dimension is not a MEASURE target (ADR 0017 A.8); reporting it
            // as measurable would be a silent over-claim.
            SemanticGeometry::Dimension(_) => SupportStatus::Unsupported,
            // A hatch has no MEASURE target either: area/loop measurement is not
            // implemented, so claiming the completeness-derived status would
            // over-claim (ADR 0018 E).
            SemanticGeometry::Hatch(_) => SupportStatus::Unsupported,
            _ => semantic,
        };
        let entry = self
            .capabilities
            .entry(class_name.to_string())
            .or_insert(EntityCapability {
                type_key: class_name.to_string(),
                read: SupportStatus::Verified,
                semantic,
                render,
                pick,
                measure,
            });
        // Keep the weakest status observed for the class.
        entry.semantic = weaker(entry.semantic, semantic);
        entry.render = weaker(entry.render, render);
        entry.pick = weaker(entry.pick, pick);
        entry.measure = weaker(entry.measure, measure);
    }
}
