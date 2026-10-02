//! Database-building lifecycle for [`ImporterBuilder`].

use super::*;

/// How many walked source entities pass between entity-batch progress ticks.
///
/// A bounded batch keeps a million-entity drawing from emitting a million
/// events while still advancing a progress bar several times per second. The
/// value is a real work quantum, not a timer.
pub(crate) const ENTITY_PROGRESS_BATCH: usize = 512;

impl<'a> ImporterBuilder<'a> {
    pub(crate) fn new(
        request: &'a ImportRequest,
        acad: &'a acadrust::CadDocument,
        stats: ReadStats,
        identity: DocumentIdentity,
    ) -> Self {
        let proxy = ProxyPlayer::new(DecodeLimits {
            max_bytes: request.limits.max_file_bytes,
            ..DecodeLimits::default()
        });
        ImporterBuilder {
            request,
            acad,
            stats,
            identity,
            cancelled: &NEVER_CANCELLED,
            progress: &NoopProgress,
            builder: DrawingDatabaseBuilder::new(request.database),
            diagnostics: Vec::new(),
            capabilities: HashMap::new(),
            layer_ids: HashMap::new(),
            layer_transparency: HashMap::new(),
            layer_colors: HashMap::new(),
            layer_lineweights: HashMap::new(),
            style_ids: HashMap::new(),
            style_fonts: HashMap::new(),
            block_ids: HashMap::new(),
            block_base_points: HashMap::new(),
            layout_ids: HashMap::new(),
            next_entity: 1,
            next_object: 1,
            entity_total: 0,
            dropped: 0,
            proxy,
            model_layout: LayoutId(0),
            block_members: HashMap::new(),
            block_status: HashMap::new(),
            model_render: SupportStatus::Verified,
            model_drawable: false,
            model_render_types: BTreeSet::new(),
        }
    }

    pub(crate) fn with_progress(
        mut self,
        cancelled: &'a dyn Fn() -> bool,
        progress: &'a dyn ImportProgressSink,
    ) -> Self {
        self.cancelled = cancelled;
        self.progress = progress;
        self
    }

    /// Abort with [`CadError::Cancelled`] when the host has cancelled.
    pub(crate) fn check_cancelled(&self) -> CadResult<()> {
        if (self.cancelled)() {
            Err(CadError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Emit a real progress tick at an entity batch boundary.
    pub(crate) fn report_entities(&self, entities_total: usize) {
        self.progress.report(ImportProgress {
            phase: ImportPhase::Entities,
            entities_done: self.entity_total,
            entities_total: Some(entities_total),
            bytes: None,
            message: None,
        });
    }

    pub(crate) fn run(mut self) -> CadResult<ImportedDrawing> {
        self.check_cancelled()?;
        self.progress
            .report(ImportProgress::at(ImportPhase::Tables, 0, None));
        self.read_layers()?;
        self.read_styles()?;
        self.read_layouts()?;
        self.read_blocks()?;
        self.check_cancelled()?;
        self.progress.report(ImportProgress::at(
            ImportPhase::Entities,
            0,
            Some(self.entity_total_expected()),
        ));
        self.read_entities()?;
        self.check_cancelled()?;
        self.progress.report(ImportProgress::at(
            ImportPhase::Resolving,
            self.entity_total,
            Some(self.entity_total_expected()),
        ));
        self.resolve_block_status();
        self.check_cancelled()?;
        self.progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            self.entity_total,
            Some(self.entity_total_expected()),
        ));

        // Capture everything we need before the builder is consumed.
        let units = self.read_units();
        let capabilities: Vec<EntityCapability> = {
            let mut v: Vec<_> = std::mem::take(&mut self.capabilities)
                .into_values()
                .collect();
            v.sort_by(|a, b| a.type_key.cmp(&b.type_key));
            v
        };
        let mut diagnostics = std::mem::take(&mut self.diagnostics);
        let dropped = self.dropped;
        let stream_completed = self.stats.stream_completed;
        let model_render = self.model_render;
        let model_drawable = self.model_drawable;
        let model_render_types: Vec<String> = std::mem::take(&mut self.model_render_types)
            .into_iter()
            .collect();
        let database = self.builder.finish()?;

        if dropped > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "import.entity_limit".into(),
                message: format!("{dropped} entities were skipped at the import limit"),
            });
        }
        if !stream_completed {
            diagnostics.push(Diagnostic {
                object: None,
                code: "import.incomplete_stream".into(),
                message: "source stream did not complete; recovery data may be partial".into(),
            });
        }
        // Completeness reflects what can actually be drawn, not merely whether
        // parsing succeeded (audit B20). A drawing whose content cannot be
        // rendered must not report `Complete`.
        let import_items: Vec<String> = diagnostics
            .iter()
            .filter(|d| d.code.starts_with("import."))
            .map(|d| d.message.clone())
            .collect();
        let completeness = aggregate_completeness(
            model_render,
            model_drawable,
            model_render_types,
            import_items,
        );
        Ok(ImportedDrawing {
            database,
            units,
            report: ImportReport {
                identity: self.identity.clone(),
                capabilities,
                completeness,
                diagnostics,
                parse_ms: None,
            },
        })
    }

    pub(crate) fn read_units(&self) -> UnitContext {
        match self.acad.header.insertion_units {
            1 => UnitContext {
                source: Unit::Inch,
                display: Unit::Inch,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            2 => UnitContext {
                source: Unit::Foot,
                display: Unit::Foot,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            4 => UnitContext {
                source: Unit::Millimeter,
                display: Unit::Millimeter,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            5 => UnitContext {
                source: Unit::Meter,
                display: Unit::Meter,
                display_per_source: Some(0.01),
                decimal_places: 3,
            },
            6 => UnitContext {
                source: Unit::Meter,
                display: Unit::Meter,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            _ => UnitContext::drawing_units(),
        }
    }

    pub(crate) fn read_layers(&mut self) -> CadResult<()> {
        for (index, layer) in self.acad.layers.iter().enumerate() {
            let id = LayerId(index as u128);
            self.layer_ids.insert(layer.name.clone(), id);
            self.layer_transparency
                .insert(id, layer_opacity(layer.transparency));
            self.layer_colors.insert(id, layer_rgb(layer.color));
            if let Some(mm) = lineweight_mm(layer.line_weight) {
                self.layer_lineweights.insert(id, mm);
            }
            let visible = !layer.flags.off && !layer.flags.frozen;
            self.builder.insert_layer(Layer {
                id,
                name: layer.name.clone(),
                visible,
            })?;
        }
        Ok(())
    }

    pub(crate) fn read_styles(&mut self) -> CadResult<()> {
        for (index, style) in self.acad.text_styles.iter().enumerate() {
            let id = StyleId(index as u128);
            self.style_ids.insert(style.name.clone(), id);
            let mut keys = Vec::new();
            if !style.font_file.is_empty() {
                keys.push(style.font_file.clone());
            }
            if !style.big_font_file.is_empty() {
                keys.push(style.big_font_file.clone());
            }
            if !style.true_type_font.is_empty() {
                keys.push(style.true_type_font.clone());
            }
            // Primary font for this style, used to shape text geometry. Prefer
            // the TrueType face when the drawing names one explicitly.
            let primary = if !style.true_type_font.is_empty() {
                Some(style.true_type_font.clone())
            } else if !style.font_file.is_empty() {
                Some(style.font_file.clone())
            } else {
                None
            };
            if let Some(font) = primary {
                self.style_fonts
                    .insert(style.name.to_ascii_lowercase(), font);
            }
            self.builder.insert_style(Style {
                id,
                name: style.name.clone(),
                resource_keys: keys,
            })?;
        }
        Ok(())
    }

    pub(crate) fn read_layouts(&mut self) -> CadResult<()> {
        let mut order = 0u128;
        for block in self.acad.block_records.iter() {
            if !is_paper_space_name(&block.name) {
                continue;
            }
            let id = LayoutId(order + 1);
            order += 1;
            self.layout_ids.insert(block.name.clone(), id);
            let mut viewports = Vec::new();
            for entity in self.acad.entities_in_block(&block.name) {
                if let EntityType::Viewport(v) = entity {
                    // The sheet viewport (`id == 1`) frames the paper itself, not
                    // a model-space window, and an off viewport draws nothing;
                    // neither is a content viewport (audit B22).
                    let Some(viewport) = paper_viewport(v) else {
                        continue;
                    };
                    if let Completeness::Partial(reasons) = &viewport.completeness {
                        let why = reasons
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "unsupported viewport state".to_string());
                        self.diagnostics.push(Diagnostic {
                            object: None,
                            code: "import.viewport_unsupported".into(),
                            message: format!("layout {} viewport {}: {why}", block.name, v.id),
                        });
                    }
                    viewports.push(viewport);
                }
            }
            self.builder.insert_layout(Layout {
                id,
                name: block.name.clone(),
                viewports,
            })?;
        }
        if self.layout_ids.is_empty() {
            let id = LayoutId(0);
            self.builder.insert_layout(Layout {
                id,
                name: "Model".into(),
                viewports: Vec::new(),
            })?;
            self.model_layout = id;
        }
        Ok(())
    }

    pub(crate) fn read_blocks(&mut self) -> CadResult<()> {
        let mut next = 0u128;
        for block in self.acad.block_records.iter() {
            if is_space_block_name(&block.name) {
                continue;
            }
            let id = BlockId(next);
            next += 1;
            self.block_ids.insert(block.name.clone(), id);
            self.block_base_points
                .insert(block.name.clone(), p3(block.base_point));
            self.builder.insert_block(BlockDefinition {
                id,
                entities: Vec::new(),
            })?;
        }
        Ok(())
    }

    pub(crate) fn read_entities(&mut self) -> CadResult<()> {
        let total = self.entity_total_expected();
        // Report in bounded batches so a large drawing streams progress without
        // one tick per entity (and without pretending the count is finer than it
        // is). Cancellation is polled at each batch boundary.
        let batch = ENTITY_PROGRESS_BATCH;
        let mut since_report = 0usize;

        // Block definitions first, so model-space INSERTs can resolve the
        // display status of the block they reference (audit B15/B20).
        for block in self.acad.block_records.iter() {
            if is_space_block_name(&block.name) {
                continue;
            }
            let Some(id) = self.block_ids.get(&block.name).copied() else {
                continue;
            };
            let mut entity_ids = Vec::new();
            for entity in self.acad.entities_in_block(&block.name) {
                if let Some(e) = self.push_entity(entity, SpaceId::Block(id), self.model_layout) {
                    entity_ids.push(e);
                }
                since_report += 1;
                if since_report >= batch {
                    since_report = 0;
                    self.check_cancelled()?;
                    self.report_entities(total);
                }
            }
            self.check_cancelled()?;
            self.builder.insert_block(BlockDefinition {
                id,
                entities: entity_ids,
            })?;
        }

        // Model space: the primary drawable set.
        let model_space = self.model_space_id();
        for entity in self.acad.model_space_entities() {
            self.push_entity(entity, SpaceId::Model, model_space);
            since_report += 1;
            if since_report >= batch {
                since_report = 0;
                self.check_cancelled()?;
                self.report_entities(total);
            }
        }
        // Paper space.
        for (name, layout) in self.layout_ids.clone() {
            for entity in self.acad.entities_in_block(&name) {
                self.push_entity(entity, SpaceId::Paper(layout), layout);
                since_report += 1;
                if since_report >= batch {
                    since_report = 0;
                    self.check_cancelled()?;
                    self.report_entities(total);
                }
            }
        }
        // Final tick for the tail of the last batch (never a zero-work tick when
        // the drawing was empty: that boundary was already reported by `run`).
        if since_report > 0 {
            self.check_cancelled()?;
            self.report_entities(total);
        }
        Ok(())
    }

    /// Resolve each block's render status from its children, iterating to a
    /// fixpoint so nested INSERTs are accounted for. Cycles settle at
    /// `Unverified` instead of looping.
    pub(crate) fn resolve_block_status(&mut self) {
        let ids: Vec<BlockId> = self.block_members.keys().copied().collect();
        for _ in 0..=ids.len() {
            let mut changed = false;
            for id in &ids {
                let members = self.block_members.get(id).cloned().unwrap_or_default();
                let mut status = SupportStatus::Verified;
                for member in &members {
                    // A member that references blocks inherits the weakest of
                    // its dependencies; a plain member uses its own status.
                    let child = if member.insert_blocks.is_empty() {
                        member.render
                    } else {
                        member
                            .insert_blocks
                            .iter()
                            .map(|block| {
                                self.block_status
                                    .get(block)
                                    .copied()
                                    .unwrap_or(SupportStatus::Unverified)
                            })
                            .fold(SupportStatus::Verified, weaker)
                    };
                    status = weaker(status, child);
                }
                if self.block_status.get(id).copied() != Some(status) {
                    self.block_status.insert(*id, status);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub(crate) fn model_space_id(&self) -> LayoutId {
        self.layout_ids
            .get("*Model_Space")
            .copied()
            .unwrap_or(self.model_layout)
    }

    /// Entities the builder is expected to walk, from the parsed document.
    ///
    /// Matches the walk in [`ImporterBuilder::read_entities`] (user blocks, then
    /// model space, then paper space). This is a *real* count from the parsed
    /// document, never a fabricated total; it excludes block/end markers only
    /// indirectly because they are skipped during the walk itself.
    pub(crate) fn entity_total_expected(&self) -> usize {
        let mut total = 0usize;
        for block in self.acad.block_records.iter() {
            if is_space_block_name(&block.name) {
                continue;
            }
            total += self.acad.entities_in_block(&block.name).count();
        }
        total += self.acad.model_space_entities().count();
        for block in self.acad.block_records.iter() {
            if is_paper_space_name(&block.name) {
                total += self.acad.entities_in_block(&block.name).count();
            }
        }
        total
    }
}
