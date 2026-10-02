//! Display representations and the representation context.

use super::*;

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
