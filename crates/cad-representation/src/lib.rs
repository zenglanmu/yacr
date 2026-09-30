//! Disposable CPU display descriptions; no surfaces or GPU commands.
//!
//! Spec v2.0 §4.8: providers turn a read-only entity view plus resource/style
//! context into display primitives and a completeness report. They never touch
//! Slint, GPU objects or the event loop, and they never mutate the database.

use cad_db::DbEntity;
use cad_domain::*;
use cad_geometry::{tessellate_geometry, TessellationParams};
use cad_resources::ResourceKey;
use std::sync::Arc;

/// One drawable piece of an entity, in world coordinates.
pub enum DisplayPrimitive {
    Lines(Arc<[Point3]>),
    Mesh(Arc<Mesh>),
    Text { text: String, origin: Point3, font: ResourceKey, height: f64 },
    Image { resource: ResourceKey, transform: Transform3 },
    Instance { block: BlockId, transform: Transform3 },
}

/// A primitive plus the source reference needed for picking after batching.
pub struct DisplayFragment {
    pub source: SelectionRef,
    pub geometry_source: GeometrySource,
    pub primitive: DisplayPrimitive,
}

pub struct DisplayRepresentation {
    pub fragments: Vec<DisplayFragment>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
}

impl DisplayRepresentation {
    pub fn empty(completeness: Completeness) -> Self {
        DisplayRepresentation { fragments: Vec::new(), completeness, diagnostics: Vec::new() }
    }
}

pub struct RepresentationContext {
    pub document: DocumentId,
    pub tolerance: TolerancePolicy,
    pub stamp: TaskStamp,
}

impl RepresentationContext {
    pub fn new(document: DocumentId, tolerance: TolerancePolicy, stamp: TaskStamp) -> Self {
        RepresentationContext { document, tolerance, stamp }
    }
}

pub trait RepresentationProvider {
    fn registration(&self) -> Registration;
    fn build(&self, entity: &DbEntity, context: &RepresentationContext) -> CadResult<DisplayRepresentation>;
}

/// The built-in provider for the core entity set (spec §3.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRepresentationProvider;

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
            capabilities: vec!["lines".into(), "mesh".into(), "text".into(), "instance".into()],
        }
    }

    fn build(&self, entity: &DbEntity, context: &RepresentationContext) -> CadResult<DisplayRepresentation> {
        let source = SelectionRef {
            document: context.document.clone(),
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
                    primitive: DisplayPrimitive::Mesh(Arc::new(mesh.clone())),
                });
                if mesh.triangles.is_empty() {
                    representation.completeness = Completeness::Partial(vec!["mesh has no triangles".into()]);
                }
            }
            SemanticGeometry::Insert { block, transform } => {
                representation.fragments.push(DisplayFragment {
                    source,
                    geometry_source,
                    primitive: DisplayPrimitive::Instance { block: *block, transform: *transform },
                });
            }
            SemanticGeometry::Text { text, position, style, height, .. } => {
                representation.fragments.push(DisplayFragment {
                    source,
                    geometry_source,
                    primitive: DisplayPrimitive::Text {
                        text: text.clone(),
                        origin: *position,
                        font: ResourceKey(format!("style:{}", style.0)),
                        height: height.abs(),
                    },
                });
            }
            SemanticGeometry::Opaque { type_key, .. } => {
                representation.completeness = Completeness::Missing(vec!["no display representation in this build".into()]);
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
                        primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
                    });
                } else {
                    representation.completeness = Completeness::Missing(vec!["no display representation in this build".into()]);
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
            return Err(CadError::Invariant("provider registration needs a type key".to_string()));
        }
        if self.providers.iter().any(|p| p.registration().type_key == new_reg.type_key) {
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
            || registration.entity_types.iter().any(|t| t == entity_type_key)
    }

    /// Build a representation using the highest-priority matching provider.
    pub fn build(&self, entity: &DbEntity, context: &RepresentationContext) -> CadResult<DisplayRepresentation> {
        let entity_type = &entity.object.type_key;
        let mut matching: Vec<&Box<dyn RepresentationProvider>> = self
            .providers
            .iter()
            .filter(|p| Self::matches(&p.registration(), entity_type))
            .collect();
        if matching.is_empty() {
            return Ok(DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Missing(vec!["no representation provider matched".into()]),
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "representation.no_provider".into(),
                    message: format!("no representation provider matches '{entity_type}'"),
                }],
            });
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
                top.iter().map(|p| p.registration().type_key).collect::<Vec<_>>()
            )));
        }
        top[0].build(entity, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::DbObject;

    fn entity(id: u128, geometry: SemanticGeometry) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: Some("1A".into()),
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: 0,
        }
    }

    fn context() -> RepresentationContext {
        RepresentationContext::new(DocumentId(1), TolerancePolicy::default(), TaskStamp::new(DocumentId(1), 0))
    }

    #[test]
    fn line_becomes_a_lines_primitive() {
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            1,
            SemanticGeometry::Line {
                start: Point3 { x: 0.0, y: 0.0, z: 0.0 },
                end: Point3 { x: 10.0, y: 0.0, z: 0.0 },
            },
        );
        let r = registry.build(&e, &context()).unwrap();
        assert_eq!(r.fragments.len(), 1);
        assert_eq!(r.completeness, Completeness::Complete);
        match &r.fragments[0].primitive {
            DisplayPrimitive::Lines(pts) => assert_eq!(pts.len(), 2),
            _ => panic!("expected lines"),
        }
    }

    #[test]
    fn opaque_geometry_is_reported_unsupported_not_empty_success() {
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            2,
            SemanticGeometry::Opaque { type_key: "ACIS".into(), version: 1, payload: vec![1, 2, 3] },
        );
        let r = registry.build(&e, &context()).unwrap();
        assert!(matches!(r.completeness, Completeness::Missing(_)));
        assert!(!r.diagnostics.is_empty());
    }

    #[test]
    fn duplicate_provider_type_key_is_rejected() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(DefaultRepresentationProvider)).unwrap();
        assert!(registry.register(Box::new(DefaultRepresentationProvider)).is_err());
    }
}
