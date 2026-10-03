//! Representation providers and the provider registry.

use super::*;

pub trait RepresentationProvider {
    fn registration(&self) -> Registration;
    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation>;

    /// Build a representation with the importer-resolved attributes for the
    /// entity.
    ///
    /// [`ProviderRegistry::build_expanded`] calls this so a provider can act on
    /// data that lives beside the entity (currently annotative scaling). The
    /// default ignores the attributes and delegates to
    /// [`build`](Self::build), so existing providers keep compiling and behave
    /// exactly as before.
    fn build_with_attributes(
        &self,
        entity: &DbEntity,
        attributes: &EntityRenderAttributes,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let _ = attributes;
        self.build(entity, context)
    }
}

/// The built-in provider for the core entity set (spec §3.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRepresentationProvider;

/// The text/MTEXT annotation-scale adjustment for one build.
///
/// `factor` is the active scale's paper/drawing factor; `position`/`rotation`/
/// `height` are an optional per-scale override (applied as-is when present).
#[derive(Debug, Clone, Copy)]
struct TextScale {
    factor: f64,
    position: Option<Point3>,
    rotation: Option<f64>,
    height: Option<f64>,
}

impl DefaultRepresentationProvider {
    /// Tolerance used for CPU display discretisation at the current zoom.
    ///
    /// The engine's `tessellate_curve` interprets the policy's pixel budget as
    /// world units when no camera is available; callers that know the zoom
    /// should scale `display_pixels` before building.
    fn params(context: &RepresentationContext) -> TessellationParams {
        TessellationParams {
            tolerance: context.tolerance.display_pixels.max(1e-12),
            ..TessellationParams::default()
        }
    }
}

impl RepresentationProvider for DefaultRepresentationProvider {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.representation.default".into(),
            version: 1,
            priority: 0,
            entity_types: vec![], // matches any database entity
            capabilities: vec![
                "lines".into(),
                "mesh".into(),
                "text".into(),
                "instance".into(),
            ],
        }
    }

    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        self.build_inner(entity, context, None)
    }

    /// Apply the imported annotative state: an annotative text/MTEXT entity is
    /// scaled by the active annotation scale factor about its anchor, and a
    /// per-scale placement override (when imported) replaces the base
    /// position/rotation. A non-annotative entity, or a context without an
    /// active annotation scale, builds exactly as before.
    fn build_with_attributes(
        &self,
        entity: &DbEntity,
        attributes: &EntityRenderAttributes,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let annotative = attributes.annotative.annotative;
        let Some(scale) = context.annotation_scale.as_ref().filter(|_| annotative) else {
            return self.build_inner(entity, context, None);
        };
        let factor = scale.factor;
        if !factor.is_finite() || factor <= 0.0 {
            // An unusable factor must not silently scale by 1.0; report it and
            // draw the base geometry so the entity is still visible.
            let mut representation = self.build_inner(entity, context, None)?;
            representation.completeness =
                representation
                    .completeness
                    .combine(Completeness::Partial(vec![format!(
                    "annotative scale '{}' has an invalid factor {factor}; drawn at its base size",
                    scale.name
                )]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "annotative.invalid_scale".into(),
                message: format!("active annotation scale factor {factor} is not usable"),
            });
            return Ok(representation);
        }
        let over = attributes.annotative.override_for(&scale.name);
        let mut representation = self.build_inner(
            entity,
            context,
            Some(TextScale {
                factor,
                position: over.map(|o| o.position),
                rotation: over.map(|o| o.rotation),
                height: over.and_then(|o| o.height),
            }),
        )?;

        // Annotative scaling is implemented for text/MTEXT only. Any other
        // annotative geometry (dimension styles, multileader, hatch, blocks)
        // is left at its base placement and reported `Partial`, never silently
        // scaled as if it were exact.
        if !matches!(entity.geometry, SemanticGeometry::Text { .. }) {
            representation.completeness =
                representation
                    .completeness
                    .combine(Completeness::Partial(vec![format!(
                        "annotative scaling for {} is not implemented; drawn at its base size",
                        entity.object.type_key
                    )]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "annotative.unsupported_entity".into(),
                message: format!(
                    "{} is annotative but only TEXT/MTEXT annotative scaling is supported",
                    entity.object.type_key
                ),
            });
        }
        Ok(representation)
    }
}

impl DefaultRepresentationProvider {
    fn build_inner(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
        annotative: Option<TextScale>,
    ) -> CadResult<DisplayRepresentation> {
        // A compound entity (for example a HATCH) renders as a group; recurse
        // per child and merge the results in order.
        if let SemanticGeometry::Compound(children) = &entity.geometry {
            let mut representation = DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Complete,
                diagnostics: Vec::new(),
            };
            for child in children {
                let mut child_entity = entity.clone();
                child_entity.geometry = child.clone();
                let child_representation = self.build_inner(&child_entity, context, annotative)?;
                representation
                    .fragments
                    .extend(child_representation.fragments);
                representation
                    .diagnostics
                    .extend(child_representation.diagnostics);
                representation.completeness = representation
                    .completeness
                    .combine(child_representation.completeness);
            }
            return Ok(representation);
        }
        let source = SelectionRef {
            document: context.document,
            entity: entity.id,
            instance: InstancePath::default(),
            sub_element: None,
        };
        let mut representation = DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        };
        let geometry_source = GeometrySource::Analytic;

        match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => {
                representation.fragments.push(DisplayFragment {
                    source,
                    geometry_source,
                    precision: Precision::Analytic,
                    alpha: 1.0,
                    color: DEFAULT_RENDER_COLOR,
                    color_unresolved: true,
                    lineweight: DEFAULT_LINEWEIGHT_MM,
                    lineweight_unresolved: true,
                    linetype: LinetypePattern::continuous(),
                    linetype_unresolved: true,
                    linetype_scale: 1.0,
                    primitive: DisplayPrimitive::Mesh(Arc::new(mesh.clone())),
                });
                if mesh.triangles.is_empty() {
                    representation.completeness =
                        Completeness::Partial(vec!["mesh has no triangles".into()]);
                }
            }
            SemanticGeometry::Insert { block, transform } => {
                representation.fragments.push(DisplayFragment {
                    source,
                    geometry_source,
                    precision: Precision::Analytic,
                    alpha: 1.0,
                    color: DEFAULT_RENDER_COLOR,
                    color_unresolved: true,
                    lineweight: DEFAULT_LINEWEIGHT_MM,
                    lineweight_unresolved: true,
                    linetype: LinetypePattern::continuous(),
                    linetype_unresolved: true,
                    linetype_scale: 1.0,
                    primitive: DisplayPrimitive::Instance {
                        block: *block,
                        transform: *transform,
                    },
                });
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
                let font_key = font.as_deref();
                // Annotative TEXT/MTEXT: the active annotation scale factor
                // scales the glyph size about the anchor, and a per-scale
                // placement override (when imported) replaces the base
                // position/rotation. Non-annotative entities use the base
                // geometry unchanged.
                let (eff_position, eff_height, eff_rotation) = match annotative {
                    Some(a) => (
                        a.position.unwrap_or(*position),
                        a.height.unwrap_or(height.abs() * a.factor),
                        a.rotation.unwrap_or(*rotation),
                    ),
                    None => (*position, height.abs(), *rotation),
                };
                let mut shaped = false;
                if let (Some(engine), Some(key)) = (&context.fonts, font_key) {
                    match engine.shape(
                        key,
                        text,
                        eff_position,
                        eff_height,
                        eff_rotation,
                        *h_align,
                        *v_align,
                    ) {
                        Ok(shaped_text) => {
                            for polyline in shaped_text.polylines {
                                if polyline.len() >= 2 {
                                    representation.fragments.push(DisplayFragment {
                                        source: source.clone(),
                                        geometry_source: geometry_source.clone(),
                                        precision: Precision::Analytic,
                                        alpha: 1.0,
                                        color: DEFAULT_RENDER_COLOR,
                                        color_unresolved: true,
                                        lineweight: DEFAULT_LINEWEIGHT_MM,
                                        lineweight_unresolved: true,
                                        linetype: LinetypePattern::continuous(),
                                        linetype_unresolved: true,
                                        linetype_scale: 1.0,
                                        primitive: DisplayPrimitive::Lines(Arc::from(
                                            polyline.into_boxed_slice(),
                                        )),
                                    });
                                }
                            }
                            shaped = true;
                            if representation.fragments.is_empty() {
                                representation.completeness = Completeness::Missing(vec![
                                    "text produced no drawable outlines".into(),
                                ]);
                                representation.diagnostics.push(Diagnostic {
                                    object: Some(ObjectId(entity.id.0)),
                                    code: "text.empty_outline".into(),
                                    message: format!(
                                        "font '{key}' produced no outline for the text"
                                    ),
                                });
                            }
                            // MTEXT formatting that shaped only approximately
                            // (color, decorations, stacked fractions, ...) is
                            // reported in the completeness verdict and
                            // diagnostics rather than silently ignored.
                            if !shaped_text.issues.is_empty() {
                                let reasons: Vec<String> = shaped_text
                                    .issues
                                    .iter()
                                    .map(|issue| issue.message.clone())
                                    .collect();
                                representation.completeness = representation
                                    .completeness
                                    .combine(Completeness::Partial(reasons));
                                for issue in &shaped_text.issues {
                                    representation.diagnostics.push(Diagnostic {
                                        object: Some(ObjectId(entity.id.0)),
                                        code: issue.code.into(),
                                        message: issue.message.clone(),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            representation.completeness = Completeness::Partial(vec![format!(
                                "text font '{key}' unavailable: {error}"
                            )]);
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: "text.font_unavailable".into(),
                                message: error.to_string(),
                            });
                        }
                    }
                }
                if !shaped {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::Text {
                            text: text.clone(),
                            origin: eff_position,
                            font: ResourceKey(
                                font_key
                                    .map(str::to_string)
                                    .unwrap_or_else(|| format!("style:{}", style.0)),
                            ),
                            height: eff_height,
                        },
                    });
                }
            }
            SemanticGeometry::Shape {
                shape_name,
                code,
                position,
                size,
                rotation,
                font,
            } => {
                if *code == 0 {
                    representation.completeness = Completeness::Partial(vec![format!(
                        "shape '{shape_name}' is referenced by name; only shape codes are resolved"
                    )]);
                } else if let (Some(engine), Some(key)) = (&context.fonts, font.as_deref()) {
                    match engine.shape_glyph(key, *code, *position, size.abs(), *rotation) {
                        Ok(polylines) => {
                            for polyline in polylines {
                                if polyline.len() >= 2 {
                                    representation.fragments.push(DisplayFragment {
                                        source: source.clone(),
                                        geometry_source: geometry_source.clone(),
                                        precision: Precision::Analytic,
                                        alpha: 1.0,
                                        color: DEFAULT_RENDER_COLOR,
                                        color_unresolved: true,
                                        lineweight: DEFAULT_LINEWEIGHT_MM,
                                        lineweight_unresolved: true,
                                        linetype: LinetypePattern::continuous(),
                                        linetype_unresolved: true,
                                        linetype_scale: 1.0,
                                        primitive: DisplayPrimitive::Lines(Arc::from(
                                            polyline.into_boxed_slice(),
                                        )),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            representation.completeness = Completeness::Partial(vec![format!(
                                "shape font '{key}' unavailable: {error}"
                            )]);
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: "shape.font_unavailable".into(),
                                message: error.to_string(),
                            });
                        }
                    }
                } else {
                    representation.completeness =
                        Completeness::Partial(
                            vec!["shape has no resolvable SHX shape font".into()],
                        );
                }
            }
            SemanticGeometry::Opaque { type_key, .. } => {
                representation.completeness =
                    Completeness::Missing(vec!["no display representation in this build".into()]);
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "representation.opaque".into(),
                    message: format!("{type_key} has no display representation in this build"),
                });
            }
            other => {
                let points = tessellate_geometry(other, Self::params(context));
                if points.len() >= 2 {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
                    });
                } else {
                    representation.completeness = Completeness::Missing(vec![
                        "no display representation in this build".into(),
                    ]);
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "representation.empty".into(),
                        message: format!("{} produced no drawable points", entity.object.type_key),
                    });
                }
            }
        }
        Ok(representation)
    }
}

/// Ordered provider registry; ambiguous matches are rejected, never guessed.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Vec<Box<dyn RepresentationProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_provider() -> Self {
        let mut registry = Self::new();
        registry
            .register(Box::new(DefaultRepresentationProvider))
            .expect("default provider cannot conflict with an empty registry");
        registry
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Register a provider, rejecting a same-priority ambiguity for one type.
    pub fn register(&mut self, provider: Box<dyn RepresentationProvider>) -> CadResult<()> {
        let new_reg = provider.registration();
        if new_reg.type_key.is_empty() {
            return Err(CadError::Invariant(
                "provider registration needs a type key".to_string(),
            ));
        }
        if self
            .providers
            .iter()
            .any(|p| p.registration().type_key == new_reg.type_key)
        {
            return Err(CadError::Invariant(format!(
                "a provider for type key '{}' is already registered",
                new_reg.type_key
            )));
        }
        self.providers.push(provider);
        Ok(())
    }

    fn matches(registration: &Registration, entity_type_key: &str) -> bool {
        registration.entity_types.is_empty()
            || registration
                .entity_types
                .iter()
                .any(|t| t == entity_type_key)
    }

    /// Build a representation using the highest-priority matching provider.
    pub fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        match self.select_provider(entity.id, &entity.object.type_key)? {
            Ok(provider) => provider.build(entity, context),
            Err(missing) => Ok(missing),
        }
    }

    /// Like [`build`](Self::build) but passes the entity's imported render
    /// attributes through to the provider (annotative scaling). Selection is
    /// identical, so the two never disagree about which provider wins.
    pub fn build_with_attributes(
        &self,
        entity: &DbEntity,
        attributes: &EntityRenderAttributes,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        match self.select_provider(entity.id, &entity.object.type_key)? {
            Ok(provider) => provider.build_with_attributes(entity, attributes, context),
            Err(missing) => Ok(missing),
        }
    }

    /// Resolve the single highest-priority provider for `entity_type`.
    ///
    /// `Ok(Ok(provider))` selects one; `Ok(Err(representation))` is the
    /// documented no-match result; an ambiguous top priority is an invariant
    /// error, never a guess.
    #[allow(clippy::type_complexity)]
    fn select_provider<'a>(
        &'a self,
        entity_id: EntityId,
        entity_type: &str,
    ) -> CadResult<Result<&'a dyn RepresentationProvider, DisplayRepresentation>> {
        let mut matching: Vec<&Box<dyn RepresentationProvider>> = self
            .providers
            .iter()
            .filter(|p| Self::matches(&p.registration(), entity_type))
            .collect();
        if matching.is_empty() {
            return Ok(Err(DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Missing(vec![
                    "no representation provider matched".into()
                ]),
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(entity_id.0)),
                    code: "representation.no_provider".into(),
                    message: format!("no representation provider matches '{entity_type}'"),
                }],
            }));
        }
        matching.sort_by_key(|p| std::cmp::Reverse(p.registration().priority));
        let top_priority = matching[0].registration().priority;
        let top: Vec<_> = matching
            .iter()
            .filter(|p| p.registration().priority == top_priority)
            .collect();
        if top.len() > 1 {
            return Err(CadError::Invariant(format!(
                "ambiguous providers for '{entity_type}' at priority {top_priority}: {:?}",
                top.iter()
                    .map(|p| p.registration().type_key)
                    .collect::<Vec<_>>()
            )));
        }
        Ok(Ok(&***top[0]))
    }
}

/// Flatten INSERT instances through their block definitions (audit B15).
impl ProviderRegistry {
    /// Build a representation with `INSERT` instances expanded into the block's
    /// child geometry, transformed into model space.
    ///
    /// Nesting is bounded by [`MAX_INSTANCE_DEPTH`](crate::MAX_INSTANCE_DEPTH)
    /// and cycles are cut with a `Partial` report instead of looping. Nested
    /// fragments carry their `InstancePath` so picking can map back to the
    /// insert chain. Missing block definitions become `Missing`, never a silent
    /// empty success.
    pub fn build_expanded(
        &self,
        database: &DrawingDatabase,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let mut out = DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        };
        let mut stack: Vec<BlockId> = Vec::new();
        self.expand_entity(
            database,
            entity,
            context,
            &Transform3::identity(),
            &InstancePath::default(),
            0,
            1.0,
            None,
            None,
            None,
            database.linetype_scale(),
            &mut stack,
            &mut out,
        )?;
        Ok(out)
    }

    /// Rebuild the representations of every annotative entity under `scale`.
    ///
    /// This is the incremental update a host performs when the active
    /// annotation scale changes: only entities the importer marked annotative
    /// are re-derived, so an unrelated drawing does not get rebuilt. The
    /// returned vector is keyed by entity so a caller can replace exactly the
    /// affected scene chunks.
    ///
    /// `context` supplies the document, tolerance, stamp and fonts; its
    /// `annotation_scale` is overridden by `scale`.
    pub fn rebuild_annotative(
        &self,
        database: &DrawingDatabase,
        context: &RepresentationContext,
        scale: AnnotationScaleRef,
    ) -> CadResult<Vec<(EntityId, DisplayRepresentation)>> {
        let mut scoped = context.clone();
        scoped.annotation_scale = Some(scale);
        let mut out = Vec::new();
        for entity in database.entities() {
            if !database
                .entity_render_attributes(entity.id)
                .annotative
                .annotative
            {
                continue;
            }
            out.push((entity.id, self.build_expanded(database, entity, &scoped)?));
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_entity(
        &self,
        database: &DrawingDatabase,
        entity: &DbEntity,
        context: &RepresentationContext,
        transform: &Transform3,
        path: &InstancePath,
        depth: usize,
        parent_alpha: f32,
        parent_color: Option<[f32; 3]>,
        parent_lineweight: Option<f32>,
        parent_linetype: Option<&LinetypePattern>,
        global_lt_scale: f64,
        stack: &mut Vec<BlockId>,
        out: &mut DisplayRepresentation,
    ) -> CadResult<()> {
        // The importer resolved this entity's effective opacity, colour,
        // lineweight and linetype into the database. `ByBlock` inherits the
        // containing INSERT's value, threaded down through the `parent_*`
        // arguments; at the model root those fall back to the documented
        // defaults.
        let attributes = database.entity_render_attributes(entity.id);
        // The provider sees the attributes too, so annotative text can be
        // scaled while it is shaped (the geometry is not post-processed).
        let representation = self.build_with_attributes(entity, &attributes, context)?;
        let own_alpha = match attributes.transparency {
            EntityTransparency::Explicit(alpha) => alpha,
            EntityTransparency::ByBlock => parent_alpha,
        };
        let (own_color, color_unresolved) = resolve_color(attributes.color, parent_color);
        let (own_lineweight, lineweight_unresolved) =
            resolve_lineweight(attributes.lineweight, parent_lineweight);
        let (own_linetype, linetype_unresolved, linetype_scale) =
            resolve_linetype(attributes.linetype.clone(), parent_linetype);
        out.completeness = weaker_completeness(&out.completeness, &representation.completeness);
        out.diagnostics.extend(representation.diagnostics);
        for fragment in representation.fragments {
            match fragment.primitive {
                DisplayPrimitive::Instance {
                    block,
                    transform: insert,
                } => {
                    if depth >= MAX_INSTANCE_DEPTH || stack.contains(&block) {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Partial(vec!["block nesting limit or cycle".into()]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.instance_cycle".into(),
                            message: format!(
                                "insert of block {:?} hit the nesting limit or a cycle",
                                block
                            ),
                        });
                        continue;
                    }
                    if database.block(block).is_none() {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Missing(vec!["missing block definition".into()]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.missing_block".into(),
                            message: format!("insert references unknown block {:?}", block),
                        });
                        continue;
                    }
                    let composed = transform.matrix_mul(&insert);
                    let mut child_path = path.clone();
                    child_path.0.push(entity.id);
                    stack.push(block);
                    for child in database.block_entities(block) {
                        self.expand_entity(
                            database,
                            child,
                            context,
                            &composed,
                            &child_path,
                            depth + 1,
                            own_alpha,
                            Some(own_color),
                            Some(own_lineweight),
                            Some(&own_linetype),
                            global_lt_scale,
                            stack,
                            out,
                        )?;
                    }
                    stack.pop();
                }
                DisplayPrimitive::Lines(points) => {
                    // Dash subdivision happens on the transformed world-space
                    // polyline so arc length is measured in final units.
                    let world: Vec<Point3> =
                        points.iter().map(|p| transform.apply_point(*p)).collect();
                    let (runs, fallback_reason) = subdivide_dashes(
                        &world,
                        &own_linetype,
                        linetype_scale as f64,
                        global_lt_scale,
                    );
                    if let Some(reason) = fallback_reason {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Partial(vec![format!(
                                "linetype dashes could not be generated: {reason}"
                            )]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.linetype_fallback".into(),
                            message: format!("linetype dashes fell back to continuous: {reason}"),
                        });
                    }
                    for run in runs {
                        if run.len() < 2 {
                            continue;
                        }
                        let mut source = fragment.source.clone();
                        source.instance = path.clone();
                        out.fragments.push(DisplayFragment {
                            source,
                            geometry_source: attributes.geometry_source.clone(),
                            precision: precision_for_source(&attributes.geometry_source),
                            alpha: own_alpha,
                            color: own_color,
                            color_unresolved,
                            lineweight: own_lineweight,
                            lineweight_unresolved,
                            linetype: own_linetype.clone(),
                            linetype_unresolved,
                            linetype_scale,
                            primitive: DisplayPrimitive::Lines(Arc::from(run.into_boxed_slice())),
                        });
                    }
                }
                primitive => {
                    let mut source = fragment.source;
                    source.instance = path.clone();
                    out.fragments.push(DisplayFragment {
                        source,
                        geometry_source: attributes.geometry_source.clone(),
                        precision: precision_for_source(&attributes.geometry_source),
                        alpha: own_alpha,
                        color: own_color,
                        color_unresolved,
                        lineweight: own_lineweight,
                        lineweight_unresolved,
                        linetype: own_linetype.clone(),
                        linetype_unresolved,
                        linetype_scale,
                        primitive: primitive.transformed(transform),
                    });
                }
            }
        }
        Ok(())
    }
}
