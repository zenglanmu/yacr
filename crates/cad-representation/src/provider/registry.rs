use crate::*;

use super::default::DefaultRepresentationProvider;

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
            entity.draw_order,
            // One layer-table scan per expansion: the layer-`0` id is resolved
            // here and carried down, never per entity.
            database.root_layer_scope(),
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
}
