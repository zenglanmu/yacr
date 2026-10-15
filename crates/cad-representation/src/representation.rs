//! Display representations and the representation context.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Optional diagnostic counters a host attaches to a [`RepresentationContext`]
/// to observe how many analytic conics the provider produced and how many
/// satisfy the eligibility facts for a future SDF-instance draw (continuous
/// linetype, opaque, non-degenerate axes).
///
/// Purely observational: a context without one records nothing, and attaching
/// one never changes the built geometry. The counters are monotonic.
#[derive(Debug, Default)]
pub struct ConicStats {
    produced: AtomicU64,
    sdf_eligible: AtomicU64,
}

impl ConicStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Conics produced by `build_expanded` since construction.
    pub fn produced(&self) -> u64 {
        self.produced.load(Ordering::Relaxed)
    }

    /// Produced conics that are eligible for the future SDF gate.
    pub fn sdf_eligible(&self) -> u64 {
        self.sdf_eligible.load(Ordering::Relaxed)
    }

    pub(crate) fn record(&self, eligible: bool) {
        self.produced.fetch_add(1, Ordering::Relaxed);
        if eligible {
            self.sdf_eligible.fetch_add(1, Ordering::Relaxed);
        }
    }
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
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
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

#[derive(Clone)]
pub struct RepresentationContext {
    pub document: DocumentId,
    pub tolerance: TolerancePolicy,
    pub stamp: TaskStamp,
    /// Optional font set for shaping text into line geometry. Without it, text
    /// stays an unshaped `DisplayPrimitive::Text` (not drawn by the scene).
    pub fonts: Option<Arc<FontEngine>>,
    /// Optional cache of decoded raster images, keyed by [`ResourceKey`].
    ///
    /// Without it (or without an entry for an image's key), a raster `Image`
    /// stays a `DisplayPrimitive::Lines` frame plus an `image.*` diagnostic;
    /// it never becomes a fabricated texture.
    pub images: Option<Arc<DecodedImageCache>>,
    /// Active annotation scale used to scale annotative entities.
    ///
    /// `None` means no annotation scaling is applied (the pre-annotative
    /// behaviour), so existing callers are unaffected. When set, an entity the
    /// importer marked annotative has its glyph geometry scaled about its
    /// anchor by [`AnnotationScaleRef::factor`], and any imported per-scale
    /// placement override is used as-is.
    pub annotation_scale: Option<AnnotationScaleRef>,
    /// Pack opaque dash runs as independent endpoint pairs instead of allocating
    /// a fragment and copying provenance/style for every microscopic dash.
    pub packed_line_segments: bool,
    /// Optional Conic plumbing counters (see [`ConicStats`]). `None` records
    /// nothing; attaching one never changes the built geometry.
    pub conic_stats: Option<Arc<ConicStats>>,
}

/// The active annotation scale a host applies while building representations.
///
/// `name` matches against per-scale placement overrides; `factor` is the
/// paper/drawing ratio from the drawing's Scale table
/// ([`cad_db::Scale::factor`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationScaleRef {
    pub name: String,
    pub factor: f64,
}

impl AnnotationScaleRef {
    pub fn new(name: impl Into<String>, factor: f64) -> Self {
        AnnotationScaleRef {
            name: name.into(),
            factor,
        }
    }
}

impl RepresentationContext {
    pub fn new(document: DocumentId, tolerance: TolerancePolicy, stamp: TaskStamp) -> Self {
        RepresentationContext {
            document,
            tolerance,
            stamp,
            fonts: None,
            images: None,
            annotation_scale: None,
            packed_line_segments: false,
            conic_stats: None,
        }
    }

    /// Attach a font set so text can be outlined into drawable polylines.
    pub fn with_fonts(mut self, fonts: Arc<FontEngine>) -> Self {
        self.fonts = Some(fonts);
        self
    }

    /// Attach decoded raster images so an `Image` can become a texture
    /// primitive instead of an outline-only frame.
    pub fn with_images(mut self, images: Arc<DecodedImageCache>) -> Self {
        self.images = Some(images);
        self
    }

    pub fn with_packed_line_segments(mut self) -> Self {
        self.packed_line_segments = true;
        self
    }

    /// Attach Conic plumbing counters so a caller can observe how many conics
    /// were produced and how many are SDF-eligible.
    pub fn with_conic_stats(mut self, stats: Arc<ConicStats>) -> Self {
        self.conic_stats = Some(stats);
        self
    }

    /// Apply the drawing's active annotation scale while building.
    pub fn with_annotation_scale(mut self, scale: AnnotationScaleRef) -> Self {
        self.annotation_scale = Some(scale);
        self
    }
}
