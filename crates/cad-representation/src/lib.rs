//! Disposable CPU display descriptions; no surfaces or GPU commands.
//!
//! Spec v2.0 §4.8: providers turn a read-only entity view plus resource/style
//! context into display primitives and a completeness report. They never touch
//! Slint, GPU objects or the event loop, and they never mutate the database.

use cad_db::{DbEntity, DrawingDatabase, EntityColor, EntityLineWeight, EntityTransparency};
use cad_domain::*;
use cad_geometry::{tessellate_geometry, TessellationParams};
use cad_kernel_adapter::{TessellationMesh, TessellationOutcome, TessellationResult};
use cad_resources::ResourceKey;
use std::sync::Arc;

/// Default display colour when a fragment carries no resolved source colour.
///
/// `[1, 1, 1]` is white, AutoCAD's nominal default entity colour (ACI 7), so the
/// fallback is a documented convention rather than an invented value.
pub const DEFAULT_RENDER_COLOR: [f32; 3] = [1.0, 1.0, 1.0];

/// Default lineweight in millimetres for an unresolved value or acadrust's
/// `LineWeight::Default`. AutoCAD's nominal default is 0.25 mm. The renderer
/// does **not** draw this width yet; it is carried so the gap is reported
/// (`docs/entity-style.md`).
pub const DEFAULT_LINEWEIGHT_MM: f32 = 0.25;

pub mod text;

pub mod shx;

pub mod layout;

pub use layout::{
    clip_polyline_to_rect, enumerate_layouts, paper_per_model_from_view, viewport_reason,
    viewport_transform, LayoutDescriptor, SpaceSelection, ViewportState, ViewportTransform,
    ViewportUnsupported,
};
pub use text::{sanitize_text, FontEngine};

/// One drawable piece of an entity, in world coordinates.
pub enum DisplayPrimitive {
    Lines(Arc<[Point3]>),
    Mesh(Arc<Mesh>),
    Text {
        text: String,
        origin: Point3,
        font: ResourceKey,
        height: f64,
    },
    Image {
        resource: ResourceKey,
        transform: Transform3,
    },
    Instance {
        block: BlockId,
        transform: Transform3,
    },
}

impl DisplayPrimitive {
    /// Apply `transform` to a primitive's world-space geometry.
    pub fn transformed(&self, transform: &Transform3) -> DisplayPrimitive {
        match self {
            DisplayPrimitive::Lines(points) => DisplayPrimitive::Lines(Arc::from(
                points
                    .iter()
                    .map(|p| transform.apply_point(*p))
                    .collect::<Vec<Point3>>()
                    .into_boxed_slice(),
            )),
            DisplayPrimitive::Mesh(mesh) => {
                DisplayPrimitive::Mesh(Arc::new(transform_mesh(mesh, transform)))
            }
            DisplayPrimitive::Text {
                text,
                origin,
                font,
                height,
            } => DisplayPrimitive::Text {
                text: text.clone(),
                origin: transform.apply_point(*origin),
                font: font.clone(),
                height: (height * transform_scale(transform)).abs(),
            },
            DisplayPrimitive::Image {
                resource,
                transform: local,
            } => DisplayPrimitive::Image {
                resource: resource.clone(),
                transform: transform.matrix_mul(local),
            },
            DisplayPrimitive::Instance {
                block,
                transform: local,
            } => DisplayPrimitive::Instance {
                block: *block,
                transform: transform.matrix_mul(local),
            },
        }
    }
}

/// A primitive plus the source reference needed for picking after batching.
pub struct DisplayFragment {
    pub source: SelectionRef,
    pub geometry_source: GeometrySource,
    /// How exact the fragment's geometry is. Proxy-cache geometry is a
    /// vendor-provided approximation with no exposed error bound; analytic
    /// geometry is the source representation itself.
    pub precision: Precision,
    /// Effective per-entity opacity in `[0, 1]` (1.0 opaque, 0.0 transparent).
    ///
    /// `build` cannot resolve transparency (it only sees the entity), so it
    /// emits `1.0`; [`ProviderRegistry::build_expanded`] applies the value the
    /// importer resolved and stored in the database, including `ByBlock`
    /// inheritance through INSERT expansion. The scene carries this straight
    /// into `RenderBatch::alpha`.
    pub alpha: f32,
    /// Resolved display colour as normalized sRGB in `[0, 1]` per channel.
    ///
    /// `build`/`from_tessellation` cannot reach the layer table, so they emit
    /// [`DEFAULT_RENDER_COLOR`] and set [`Self::color_unresolved`];
    /// [`ProviderRegistry::build_expanded`] replaces it with the importer's
    /// resolved value, including `ByBlock` inheritance. Alpha stays in
    /// [`Self::alpha`] so the two channels cannot diverge.
    pub color: [f32; 3],
    /// `true` when the source colour was symbolic (`ByLayer`/`ByBlock`) and no
    /// concrete value was available at this layer, so `color` is a fallback.
    pub color_unresolved: bool,
    /// Resolved lineweight in millimetres (see [`DEFAULT_LINEWEIGHT_MM`]).
    pub lineweight: f32,
    /// `true` when the source lineweight was symbolic and unresolved.
    pub lineweight_unresolved: bool,
    pub primitive: DisplayPrimitive,
}

/// Resolve an entity's stored colour against an enclosing INSERT's resolved
/// colour.
///
/// An explicit sRGB value is concrete. `ByBlock` inherits the enclosing block
/// reference's colour when one is threaded down, otherwise it is unresolved and
/// the fallback default is used (never a fabricated source colour). `ByLayer`
/// is unresolved here because this layer has no layer table; the importer has
/// already substituted the layer colour for real imports.
pub fn resolve_color(color: EntityColor, parent: Option<[f32; 3]>) -> ([f32; 3], bool) {
    match color {
        EntityColor::Explicit([r, g, b]) => (
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
            false,
        ),
        EntityColor::ByBlock => match parent {
            Some(parent) => (parent, false),
            None => (DEFAULT_RENDER_COLOR, true),
        },
        EntityColor::ByLayer => (DEFAULT_RENDER_COLOR, true),
    }
}

/// Resolve an entity's stored lineweight (millimetres) against an enclosing
/// block reference's resolved value, mirroring [`resolve_color`].
pub fn resolve_lineweight(weight: EntityLineWeight, parent: Option<f32>) -> (f32, bool) {
    match weight {
        EntityLineWeight::Explicit(mm) => (if mm.is_finite() { mm.max(0.0) } else { 0.0 }, false),
        EntityLineWeight::Default => (DEFAULT_LINEWEIGHT_MM, false),
        EntityLineWeight::ByBlock => match parent {
            Some(parent) => (parent, false),
            None => (DEFAULT_LINEWEIGHT_MM, true),
        },
        EntityLineWeight::ByLayer => (DEFAULT_LINEWEIGHT_MM, true),
    }
}

/// Precision implied by how a fragment's geometry was produced.
///
/// `KernelMesh` is deliberately *not* analytic: a kernel tessellation is an
/// approximation whose bound is carried by the tessellation result, not known
/// from the source tag alone. Use [`precision_for_kernel_mesh`] when the result
/// (and therefore the error bound) is available.
pub fn precision_for_source(source: &GeometrySource) -> Precision {
    match source {
        GeometrySource::ProxyCache | GeometrySource::KernelMesh => {
            Precision::Approximate { error_bound: None }
        }
        GeometrySource::Analytic | GeometrySource::DirectMesh | GeometrySource::UserPoints => {
            Precision::Analytic
        }
    }
}

/// Precision carried by a kernel tessellation: exact for planar facets, bounded
/// for a curved approximation. Never invented here — the kernel reports it.
pub fn precision_for_kernel_mesh(mesh: &TessellationMesh) -> Precision {
    mesh.precision.clone()
}

pub struct DisplayRepresentation {
    pub fragments: Vec<DisplayFragment>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
}

impl DisplayRepresentation {
    pub fn empty(completeness: Completeness) -> Self {
        DisplayRepresentation {
            fragments: Vec::new(),
            completeness,
            diagnostics: Vec::new(),
        }
    }

    /// Turn a kernel tessellation result into display primitives.
    ///
    /// A produced mesh becomes a [`DisplayPrimitive::Mesh`] with
    /// [`GeometrySource::KernelMesh`] and the precision the kernel reported
    /// (analytic for planar facets, a chordal bound for curved ones). An
    /// unsupported or failed tessellation stays missing geometry with the
    /// kernel's own diagnostics; it never becomes an empty-success fragment.
    pub fn from_tessellation(
        result: &TessellationResult,
        source: SelectionRef,
        alpha: f32,
    ) -> Self {
        let fragment = |geometry: &TessellationMesh| DisplayFragment {
            source: source.clone(),
            geometry_source: GeometrySource::KernelMesh,
            precision: precision_for_kernel_mesh(geometry),
            alpha,
            color: DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            primitive: DisplayPrimitive::Mesh(Arc::new(geometry.mesh.clone())),
        };
        match &result.outcome {
            TessellationOutcome::Success {
                geometry,
                diagnostics,
            } => {
                if geometry.is_empty() {
                    return DisplayRepresentation {
                        fragments: Vec::new(),
                        completeness: Completeness::Missing(vec![
                            "kernel reported success but produced no facets".into(),
                        ]),
                        diagnostics: vec![Diagnostic {
                            object: Some(ObjectId(source.entity.0)),
                            code: "kernel.empty_mesh".into(),
                            message: "success outcome carried no triangles".into(),
                        }],
                    };
                }
                DisplayRepresentation {
                    fragments: vec![fragment(geometry)],
                    completeness: Completeness::Complete,
                    diagnostics: diagnostics.clone(),
                }
            }
            TessellationOutcome::Partial {
                geometry,
                degradation,
                diagnostics,
            } => {
                let kernel_diagnostics = degradation.diagnostics();
                let mut all = kernel_diagnostics.clone();
                all.extend(diagnostics.clone());
                let reasons: Vec<String> = kernel_diagnostics
                    .iter()
                    .map(|d| format!("{}: {}", d.code, d.message))
                    .collect();
                let mut fragments = Vec::new();
                if !geometry.is_empty() {
                    fragments.push(fragment(geometry));
                }
                DisplayRepresentation {
                    fragments,
                    completeness: if reasons.is_empty() {
                        Completeness::Partial(vec!["kernel reported degradation".into()])
                    } else {
                        Completeness::Partial(reasons)
                    },
                    diagnostics: all,
                }
            }
            TessellationOutcome::Unsupported { reason } => DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Missing(vec![reason.detail.clone()]),
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(source.entity.0)),
                    code: "kernel.unsupported".into(),
                    message: format!("{}: {}", reason.code, reason.detail),
                }],
            },
            TessellationOutcome::Failed { diagnostics } => DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Missing(
                    diagnostics.iter().map(|d| d.message.clone()).collect(),
                ),
                diagnostics: diagnostics.clone(),
            },
        }
    }
}

pub struct RepresentationContext {
    pub document: DocumentId,
    pub tolerance: TolerancePolicy,
    pub stamp: TaskStamp,
    /// Optional font set for shaping text into line geometry. Without it, text
    /// stays an unshaped `DisplayPrimitive::Text` (not drawn by the scene).
    pub fonts: Option<Arc<FontEngine>>,
}

impl RepresentationContext {
    pub fn new(document: DocumentId, tolerance: TolerancePolicy, stamp: TaskStamp) -> Self {
        RepresentationContext {
            document,
            tolerance,
            stamp,
            fonts: None,
        }
    }

    /// Attach a font set so text can be outlined into drawable polylines.
    pub fn with_fonts(mut self, fonts: Arc<FontEngine>) -> Self {
        self.fonts = Some(fonts);
        self
    }
}

pub trait RepresentationProvider {
    fn registration(&self) -> Registration;
    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation>;
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
                let child_representation = self.build(&child_entity, context)?;
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
                let mut shaped = false;
                if let (Some(engine), Some(key)) = (&context.fonts, font_key) {
                    match engine.outline(
                        key,
                        &sanitize_text(text),
                        *position,
                        height.abs(),
                        *rotation,
                        *h_align,
                        *v_align,
                    ) {
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
                        primitive: DisplayPrimitive::Text {
                            text: text.clone(),
                            origin: *position,
                            font: ResourceKey(
                                font_key
                                    .map(str::to_string)
                                    .unwrap_or_else(|| format!("style:{}", style.0)),
                            ),
                            height: height.abs(),
                        },
                    });
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
        let entity_type = &entity.object.type_key;
        let mut matching: Vec<&Box<dyn RepresentationProvider>> = self
            .providers
            .iter()
            .filter(|p| Self::matches(&p.registration(), entity_type))
            .collect();
        if matching.is_empty() {
            return Ok(DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Missing(vec![
                    "no representation provider matched".into()
                ]),
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
                top.iter()
                    .map(|p| p.registration().type_key)
                    .collect::<Vec<_>>()
            )));
        }
        top[0].build(entity, context)
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
            &mut stack,
            &mut out,
        )?;
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
        stack: &mut Vec<BlockId>,
        out: &mut DisplayRepresentation,
    ) -> CadResult<()> {
        let representation = self.build(entity, context)?;
        // The importer resolved this entity's effective opacity, colour and
        // lineweight into the database. `ByBlock` inherits the containing
        // INSERT's value, threaded down through the `parent_*` arguments; at the
        // model root those fall back to the documented defaults.
        let attributes = database.entity_render_attributes(entity.id);
        let own_alpha = match attributes.transparency {
            EntityTransparency::Explicit(alpha) => alpha,
            EntityTransparency::ByBlock => parent_alpha,
        };
        let (own_color, color_unresolved) = resolve_color(attributes.color, parent_color);
        let (own_lineweight, lineweight_unresolved) =
            resolve_lineweight(attributes.lineweight, parent_lineweight);
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
                            stack,
                            out,
                        )?;
                    }
                    stack.pop();
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
                        primitive: primitive.transformed(transform),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Maximum INSERT nesting expanded into display fragments.
pub const MAX_INSTANCE_DEPTH: usize = cad_db::MAX_INSTANCE_DEPTH;

fn completeness_rank(c: &Completeness) -> u8 {
    match c {
        Completeness::Complete => 3,
        Completeness::Unverified => 2,
        Completeness::Partial(_) => 1,
        Completeness::Missing(_) => 0,
    }
}

fn weaker_completeness(a: &Completeness, b: &Completeness) -> Completeness {
    if completeness_rank(a) <= completeness_rank(b) {
        a.clone()
    } else {
        b.clone()
    }
}

fn normalize(v: Point3) -> Point3 {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len > 1e-12 {
        Point3 {
            x: v.x / len,
            y: v.y / len,
            z: v.z / len,
        }
    } else {
        v
    }
}

/// Conservative upper bound on a transform's linear scale.
fn transform_scale(t: &Transform3) -> f64 {
    let m = &t.matrix;
    let mut sum = 0.0;
    for row in m.iter().take(3) {
        for value in row.iter().take(3) {
            sum += value * value;
        }
    }
    sum.sqrt().max(1e-12)
}

fn transform_direction(t: &Transform3, v: Point3) -> Point3 {
    let m = &t.matrix;
    normalize(Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    })
}

fn transform_mesh(mesh: &Mesh, t: &Transform3) -> Mesh {
    let vertices: Vec<Point3> = mesh.vertices.iter().map(|p| t.apply_point(*p)).collect();
    let normals = if mesh.normals.len() == mesh.vertices.len() {
        mesh.normals
            .iter()
            .map(|n| transform_direction(t, *n))
            .collect()
    } else {
        mesh.normals.clone()
    };
    Mesh {
        vertices,
        triangles: mesh.triangles.clone(),
        normals,
        face_sources: mesh.face_sources.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{
        BlockDefinition, DbObject, DrawingDatabaseBuilder, EntityRenderAttributes,
        EntityTransparency, Layer,
    };

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
        RepresentationContext::new(
            DocumentId(1),
            TolerancePolicy::default(),
            TaskStamp::new(DocumentId(1), 0),
        )
    }

    #[test]
    fn line_becomes_a_lines_primitive() {
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            1,
            SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
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
    fn multi_ring_hatch_fill_flows_through_the_mesh_path() {
        // Donut: 10x10 outer with a 4x4 hole; the hole-aware fill from
        // cad-geometry is what the importer feeds into the compound.
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]];
        let fill = cad_geometry::fill_rings(&[outer.clone(), hole.clone()]).expect("donut fill");
        assert!((fill.area() - 84.0).abs() < 1e-6, "area {}", fill.area());
        let to_world = |p: [f64; 2]| Point3 {
            x: p[0],
            y: p[1],
            z: 0.0,
        };
        let vertices: Vec<Point3> = fill.vertices.iter().map(|p| to_world(*p)).collect();
        let normals = vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            };
            vertices.len()
        ];
        let mut children = Vec::new();
        for ring in [&outer, &hole] {
            children.push(SemanticGeometry::Polyline {
                points: ring.iter().map(|p| to_world(*p)).collect(),
                bulges: Vec::new(),
                closed: true,
            });
        }
        children.push(SemanticGeometry::Mesh(Mesh {
            vertices,
            triangles: fill.triangles,
            normals,
            face_sources: Vec::new(),
        }));
        let e = entity(9, SemanticGeometry::Compound(children));
        let registry = ProviderRegistry::with_default_provider();
        let r = registry.build(&e, &context()).unwrap();
        assert_eq!(r.completeness, Completeness::Complete);
        let meshes = r
            .fragments
            .iter()
            .filter(|f| matches!(f.primitive, DisplayPrimitive::Mesh(_)))
            .count();
        let lines = r
            .fragments
            .iter()
            .filter(|f| matches!(f.primitive, DisplayPrimitive::Lines(_)))
            .count();
        assert_eq!(meshes, 1, "the hole-aware fill becomes one mesh primitive");
        assert_eq!(lines, 2, "both boundary loops stay outlined");
    }

    #[test]
    fn opaque_geometry_is_reported_unsupported_not_empty_success() {
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            2,
            SemanticGeometry::Opaque {
                type_key: "ACIS".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        );
        let r = registry.build(&e, &context()).unwrap();
        assert!(matches!(r.completeness, Completeness::Missing(_)));
        assert!(!r.diagnostics.is_empty());
    }

    #[test]
    fn duplicate_provider_type_key_is_rejected() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(Box::new(DefaultRepresentationProvider))
            .unwrap();
        assert!(registry
            .register(Box::new(DefaultRepresentationProvider))
            .is_err());
    }

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry: SemanticGeometry::Line { start: a, end: b },
            draw_order: id as i64,
        }
    }

    fn insert_entity(id: u128, space: SpaceId, block: u128, dx: f64) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry: SemanticGeometry::Insert {
                block: BlockId(block),
                transform: Transform3::translation(p(dx, 0.0)),
            },
            draw_order: id as i64,
        }
    }

    fn empty_db() -> DrawingDatabaseBuilder {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b
    }

    #[test]
    fn insert_instances_expand_with_transform_and_instance_path() {
        let mut b = empty_db();
        // Block 2: one line (0,0)-(0,1).
        b.insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(12)],
        })
        .unwrap();
        b.insert_entity(line_entity(
            12,
            SpaceId::Block(BlockId(2)),
            p(0.0, 0.0),
            p(0.0, 1.0),
        ))
        .unwrap();
        // Block 1: line (0,0)-(1,0) plus a nested insert of block 2 at y=5.
        b.insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(11), EntityId(13)],
        })
        .unwrap();
        b.insert_entity(line_entity(
            11,
            SpaceId::Block(BlockId(1)),
            p(0.0, 0.0),
            p(1.0, 0.0),
        ))
        .unwrap();
        let mut nested = insert_entity(13, SpaceId::Block(BlockId(1)), 2, 0.0);
        nested.geometry = SemanticGeometry::Insert {
            block: BlockId(2),
            transform: Transform3::translation(p(0.0, 5.0)),
        };
        b.insert_entity(nested).unwrap();
        // Model: a single insert of block 1 at x=10.
        b.insert_entity(insert_entity(1, SpaceId::Model, 1, 10.0))
            .unwrap();
        let db = b.finish().unwrap();

        let registry = ProviderRegistry::with_default_provider();
        let rep = registry
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.completeness, Completeness::Complete);
        assert_eq!(rep.fragments.len(), 2);

        let mut direct = None;
        let mut deep = None;
        for fragment in &rep.fragments {
            match &fragment.primitive {
                DisplayPrimitive::Lines(pts) => match fragment.source.instance.0.len() {
                    1 => direct = Some((pts[0], pts[1])),
                    2 => deep = Some((pts[0], pts[1])),
                    other => panic!("unexpected instance path depth {other}"),
                },
                _ => panic!("expected lines"),
            }
        }
        let (a, z) = direct.expect("direct block line");
        assert_eq!((a.x, a.y), (10.0, 0.0));
        assert_eq!((z.x, z.y), (11.0, 0.0));
        // Nested line shifted by block 1's insert (0,5) then model insert (10,0).
        let (a, z) = deep.expect("nested block line");
        assert_eq!((a.x, a.y), (10.0, 5.0));
        assert_eq!((z.x, z.y), (10.0, 6.0));
    }

    #[test]
    fn missing_block_definition_is_reported_missing() {
        let mut b = empty_db();
        b.insert_entity(insert_entity(1, SpaceId::Model, 99, 0.0))
            .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = registry
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert!(matches!(rep.completeness, Completeness::Missing(_)));
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == "representation.missing_block"));
        assert!(rep.fragments.is_empty());
    }

    #[test]
    fn self_referencing_block_is_cut_with_partial_report() {
        let mut b = empty_db();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(2)],
        })
        .unwrap();
        b.insert_entity(insert_entity(2, SpaceId::Block(BlockId(0)), 0, 1.0))
            .unwrap();
        b.insert_entity(insert_entity(1, SpaceId::Model, 0, 0.0))
            .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = registry
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == "representation.instance_cycle"));
    }

    fn attrs(transparency: EntityTransparency, source: GeometrySource) -> EntityRenderAttributes {
        EntityRenderAttributes {
            transparency,
            color: cad_db::EntityColor::ByLayer,
            lineweight: cad_db::EntityLineWeight::ByLayer,
            geometry_source: source,
        }
    }

    #[test]
    fn build_expanded_carries_resolved_transparency_and_geometry_source() {
        let mut b = empty_db();
        // Model line 1: explicit 0.5 opacity, surviving from a proxy cache.
        b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(1),
            attrs(
                EntityTransparency::Explicit(0.5),
                GeometrySource::ProxyCache,
            ),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();

        let rep = registry
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.fragments.len(), 1);
        assert_eq!(rep.fragments[0].alpha, 0.5);
        assert_eq!(rep.fragments[0].geometry_source, GeometrySource::ProxyCache);
        assert_eq!(
            rep.fragments[0].precision,
            Precision::Approximate { error_bound: None },
            "proxy-cache geometry is an approximation, not an exact source value"
        );
    }

    #[test]
    fn build_expanded_resolves_byblock_from_the_containing_insert() {
        let mut b = empty_db();
        // Block 2 holds a ByBlock line; the INSERT carries 0.25.
        b.insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(12)],
        })
        .unwrap();
        b.insert_entity(line_entity(
            12,
            SpaceId::Block(BlockId(2)),
            p(0.0, 0.0),
            p(0.0, 1.0),
        ))
        .unwrap();
        b.set_entity_render_attributes(
            EntityId(12),
            attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
        )
        .unwrap();
        b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(2),
            attrs(EntityTransparency::Explicit(0.25), GeometrySource::Analytic),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();

        let rep = registry
            .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.fragments.len(), 1);
        // The child has no opacity of its own: it inherits the INSERT's.
        assert_eq!(rep.fragments[0].alpha, 0.25);
    }

    #[test]
    fn byblock_at_the_model_root_falls_back_to_opaque() {
        let mut b = empty_db();
        b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(1),
            attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = registry
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.fragments[0].alpha, 1.0);
    }

    fn attrs_styled(
        color: cad_db::EntityColor,
        lineweight: cad_db::EntityLineWeight,
    ) -> EntityRenderAttributes {
        EntityRenderAttributes {
            transparency: EntityTransparency::Explicit(1.0),
            color,
            lineweight,
            geometry_source: GeometrySource::Analytic,
        }
    }

    #[test]
    fn build_expanded_carries_resolved_color_and_lineweight() {
        let mut b = empty_db();
        b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(1),
            attrs_styled(
                cad_db::EntityColor::Explicit([10, 20, 30]),
                cad_db::EntityLineWeight::Explicit(0.5),
            ),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let rep = ProviderRegistry::with_default_provider()
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.fragments.len(), 1);
        let fragment = &rep.fragments[0];
        assert!((fragment.color[0] - 10.0 / 255.0).abs() < 1e-6);
        assert!((fragment.color[1] - 20.0 / 255.0).abs() < 1e-6);
        assert!((fragment.color[2] - 30.0 / 255.0).abs() < 1e-6);
        assert!(!fragment.color_unresolved, "explicit colour is resolved");
        assert_eq!(fragment.lineweight, 0.5);
        assert!(!fragment.lineweight_unresolved);
    }

    #[test]
    fn build_expanded_resolves_byblock_color_and_lineweight_from_the_insert() {
        let mut b = empty_db();
        b.insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(12)],
        })
        .unwrap();
        b.insert_entity(line_entity(
            12,
            SpaceId::Block(BlockId(2)),
            p(0.0, 0.0),
            p(0.0, 1.0),
        ))
        .unwrap();
        b.set_entity_render_attributes(
            EntityId(12),
            attrs_styled(
                cad_db::EntityColor::ByBlock,
                cad_db::EntityLineWeight::ByBlock,
            ),
        )
        .unwrap();
        b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(2),
            attrs_styled(
                cad_db::EntityColor::Explicit([200, 100, 50]),
                cad_db::EntityLineWeight::Explicit(0.8),
            ),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let rep = ProviderRegistry::with_default_provider()
            .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
            .unwrap();
        assert_eq!(rep.fragments.len(), 1);
        let fragment = &rep.fragments[0];
        // The child is fully ByBlock: it inherits the INSERT's colour and weight.
        assert!((fragment.color[0] - 200.0 / 255.0).abs() < 1e-6);
        assert!((fragment.color[1] - 100.0 / 255.0).abs() < 1e-6);
        assert!((fragment.color[2] - 50.0 / 255.0).abs() < 1e-6);
        assert!(!fragment.color_unresolved);
        assert_eq!(fragment.lineweight, 0.8);
        assert!(!fragment.lineweight_unresolved);
    }

    #[test]
    fn byblock_color_at_the_model_root_is_marked_unresolved() {
        let mut b = empty_db();
        b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
            .unwrap();
        b.set_entity_render_attributes(
            EntityId(1),
            attrs_styled(
                cad_db::EntityColor::ByBlock,
                cad_db::EntityLineWeight::ByBlock,
            ),
        )
        .unwrap();
        let db = b.finish().unwrap();
        let rep = ProviderRegistry::with_default_provider()
            .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
            .unwrap();
        let fragment = &rep.fragments[0];
        // The fallback default is used, but the fragment is honest that it was
        // never resolved from a concrete source value.
        assert_eq!(fragment.color, DEFAULT_RENDER_COLOR);
        assert!(fragment.color_unresolved);
        assert_eq!(fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
        assert!(fragment.lineweight_unresolved);
    }

    #[test]
    fn build_without_import_attributes_marks_style_unresolved() {
        // A hand-built database (or the non-expanded `build`) has no import
        // attributes, so the style falls back to the documented defaults and is
        // explicitly marked unresolved rather than claiming a source value.
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            7,
            SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(1.0, 0.0),
            },
        );
        let rep = registry.build(&e, &context()).unwrap();
        assert_eq!(rep.fragments.len(), 1);
        assert_eq!(rep.fragments[0].color, DEFAULT_RENDER_COLOR);
        assert!(rep.fragments[0].color_unresolved);
        assert!(rep.fragments[0].lineweight_unresolved);
    }

    /// A hand-built database (or the non-expanded `build`) has no import
    /// attributes, so it must stay fully opaque rather than guessing.
    #[test]
    fn build_without_import_attributes_is_opaque_analytic() {
        let registry = ProviderRegistry::with_default_provider();
        let e = entity(
            7,
            SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
        );
        let r = registry.build(&e, &context()).unwrap();
        assert_eq!(r.fragments[0].alpha, 1.0);
        assert_eq!(r.fragments[0].geometry_source, GeometrySource::Analytic);
        assert_eq!(r.fragments[0].precision, Precision::Analytic);
    }

    // ---- Kernel-mesh display path (F15 seam) ----

    use cad_kernel_adapter::{
        BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, BrepTessellator,
        GeometryHandle, SolidExchange, SolidTessellator, TessellationBudget, TessellationRequest,
        TessellationTolerance,
    };

    fn selection() -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(42),
            instance: InstancePath::default(),
            sub_element: None,
        }
    }

    fn square_brep(with_unsupported: bool) -> BrepData {
        let loop_ = BrepLoop {
            edges: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .iter()
                .map(|q| BrepCurve::Line {
                    start: Point3 {
                        x: q[0],
                        y: q[1],
                        z: 0.0,
                    },
                    end: Point3 {
                        x: q[0],
                        y: q[1],
                        z: 0.0,
                    },
                })
                .collect(),
        };
        let mut faces = vec![BrepFace {
            id: 0,
            surface: BrepSurface::Plane {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                u_dir: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            reversed: false,
            loops: vec![loop_],
        }];
        if with_unsupported {
            faces.push(BrepFace {
                id: 9,
                surface: BrepSurface::Unsupported {
                    type_key: "nurbs-surface".into(),
                },
                reversed: false,
                loops: Vec::new(),
            });
        }
        BrepData {
            shells: vec![BrepShell { id: 0, faces }],
            placement: None,
        }
    }

    fn tessellate(exchange: SolidExchange) -> cad_kernel_adapter::TessellationResult {
        let request = TessellationRequest {
            geometry: GeometryHandle::Resolved(ObjectId(7)),
            exchange,
            tolerance: TessellationTolerance::default(),
            budget: TessellationBudget::default(),
            stamp: TaskStamp::new(DocumentId(1), 0),
        };
        BrepTessellator.tessellate(&request, &|| false).unwrap()
    }

    #[test]
    fn kernel_mesh_becomes_a_display_mesh_with_kernel_source() {
        // A lone planar square is an open sheet: usable facets, reported as
        // Partial with open edges, never as a complete solid.
        let result = tessellate(SolidExchange::Brep(square_brep(false)));
        let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert_eq!(rep.fragments.len(), 1);
        assert_eq!(rep.fragments[0].geometry_source, GeometrySource::KernelMesh);
        // Planar facets are exact, so the kernel reports analytic precision.
        assert_eq!(rep.fragments[0].precision, Precision::Analytic);
        match &rep.fragments[0].primitive {
            DisplayPrimitive::Mesh(mesh) => assert_eq!(mesh.triangles.len(), 2),
            _ => panic!("expected a mesh primitive"),
        }
    }

    #[test]
    fn unsupported_kernel_result_stays_missing_not_empty_success() {
        let result = tessellate(SolidExchange::Sat(b"ACIS payload".to_vec()));
        let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
        assert!(matches!(rep.completeness, Completeness::Missing(_)));
        assert!(rep.fragments.is_empty());
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == "kernel.unsupported"));
    }

    #[test]
    fn partial_kernel_result_reports_degradation_and_keeps_facets() {
        let result = tessellate(SolidExchange::Brep(square_brep(true)));
        let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert_eq!(rep.fragments.len(), 1);
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == cad_kernel_adapter::codes::KERNEL_MISSING_FACE));
    }
}
