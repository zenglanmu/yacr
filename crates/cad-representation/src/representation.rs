//! Display representations and the representation context.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Optional diagnostic counters a host attaches to a [`RepresentationContext`]
/// to observe how many analytic conics the provider produced and how many
/// satisfy the slice-1 SDF eligibility facts (continuous linetype, opaque, a
/// circular/arc slice-1 frame; see `docs/adr/0008-sdf-conic-rendering.md`).
///
/// Purely observational: a context without one records nothing, and attaching
/// one never changes the built geometry. The counters are monotonic.
#[derive(Debug, Default)]
pub struct ConicStats {
    produced: AtomicU64,
    sdf_geometric: AtomicU64,
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

    /// Produced conics whose **geometry** is slice-1 SDF-shaped (a circle or
    /// circular arc: equal, orthogonal semi-axes, non-degenerate span), ignoring
    /// style and the force-chord hook. This is the geometric fact; it stays
    /// meaningful even when a style gate (dashed, transparent) or the soft-revert
    /// hook routes the conic to chords.
    pub fn sdf_geometric(&self) -> u64 {
        self.sdf_geometric.load(Ordering::Relaxed)
    }

    /// Produced conics that are **actually eligible** for the slice-1 SDF gate:
    /// geometrically SDF-shaped **and** opaque, continuous-lined and not forced to
    /// chords. An ineligible conic (ellipse, dashed, transparent, hook-forced or
    /// degenerate) is chord-expanded and still counted in [`ConicStats::produced`],
    /// never dropped silently.
    pub fn sdf_eligible(&self) -> u64 {
        self.sdf_eligible.load(Ordering::Relaxed)
    }

    pub(crate) fn record(&self, geometric: bool, eligible: bool) {
        debug_assert!(!eligible || geometric, "eligible conics are geometric");
        self.produced.fetch_add(1, Ordering::Relaxed);
        if geometric {
            self.sdf_geometric.fetch_add(1, Ordering::Relaxed);
        }
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
            // A kernel tessellation result carries no source entity, so there
            // is no display order to read. `0` is the neutral order and is
            // explicit here rather than silently defaulted.
            draw_order: 0,
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

/// The `ATTDISP` display state for block-attribute values (ADR-0055 M2).
///
/// The value is a **display** setting, not drawing data: it changes which
/// attribute text fragments the representation provider emits and nothing else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AttributeDisplay {
    /// `ATTDISP NORMAL` (the default): visible values draw; `invisible` values
    /// do not; a `constant` definition draws regardless.
    #[default]
    Normal,
    /// `ATTDISP ON`: every value draws, including `invisible` ones.
    On,
    /// `ATTDISP OFF`: every non-constant value is hidden. A `constant`
    /// definition still draws (its value is fixed at definition time).
    Off,
}

impl AttributeDisplay {
    /// Whether a value with `flags` is drawn in this mode.
    pub fn shows(self, flags: cad_domain::AttributeFlags) -> bool {
        if flags.constant {
            return true;
        }
        match self {
            AttributeDisplay::On => true,
            AttributeDisplay::Normal => !flags.invisible,
            AttributeDisplay::Off => false,
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
    /// Test/revert hook: force every conic through the **chord** path even when
    /// it is slice-1 SDF-eligible. This is the documented soft-revert lever for
    /// the SDF arc slice (see `docs/adr/0008-sdf-conic-rendering.md`); it never
    /// changes eligibility, only which draw a conic takes.
    pub force_chord_conics: bool,
    /// Whether a WIPEOUT mask carries a drawable **boundary frame** in addition
    /// to its fill (`WIPEOUTFRAME` 1/2 versus 0).
    ///
    /// The display path is built with this set for `ON` and `DisplayNotPlot`
    /// (both display the frame); the vector-plot path is built with it set only
    /// for `ON`, which is exactly what makes `DisplayNotPlot` display-only. The
    /// primitive carries the resolved flag so the renderer/plot consumer never
    /// re-reads session state.
    pub wipeout_frames: bool,
    /// Whether filled areas are drawn **filled** (`FILL`, AutoCAD's `FILLMODE`).
    ///
    /// `true` is AutoCAD's documented initial value and the pre-`FILL` build's
    /// behaviour: a HATCH draws its solid/gradient fill mesh or its pattern
    /// lines, a 2D SOLID draws its triangles and a wide polyline draws its
    /// filled stroke band.
    ///
    /// `false` draws the **same area's outline** instead, using only geometry
    /// the source already carries: the hatch boundary loops, the SOLID's mesh
    /// boundary edges and the stroke band's closed offset outline. Nothing is
    /// written to `cad-db` and no shape is changed — only which primitive the
    /// provider emits. See `provider::display_modes` for the per-primitive
    /// rule and for what `FILL OFF` deliberately does **not** switch.
    pub fill: bool,
    /// Whether text draws as its **placeholder rectangle** (`QTEXT`).
    ///
    /// The rectangle is derived from the shaped glyph geometry this build would
    /// have drawn, so it is an honest placeholder for *this* text at *this*
    /// height, not an estimate. Text whose extent cannot be derived (no font
    /// engine, or an unshaped [`DisplayPrimitive::Text`]) is left as it is and
    /// named in a `qtext.extent_unknown` diagnostic rather than given a
    /// fabricated box. `QTEXT` is a display mode: no database write, revision,
    /// history entry or `ChangeSet`, and this build claims no redraw-speed
    /// benefit from it.
    pub qtext: bool,
    /// How block-attribute values are displayed (`ATTDISP`, ADR-0055 M2).
    ///
    /// A **constant** definition draws its value in every mode; a non-constant
    /// value follows this token: `Normal` hides an `invisible` value, `On` shows
    /// every value, `Off` hides every non-constant value. This is a display
    /// mode: no database write, revision, history entry or `ChangeSet`.
    pub attdisp: AttributeDisplay,
    /// `VIEWRES`: a **ceiling** on chord segments per curve, or `None` for the
    /// build's own [`TessellationParams`] default.
    ///
    /// This is a real threshold, and a deliberately partial one: circles, arcs
    /// and ellipses are emitted as analytic [`DisplayPrimitive::Conic`] values
    /// and are *not* chord-tessellated for the display, so `VIEWRES` cannot
    /// change them. It does bound every chord path the provider builds —
    /// splines, polyline bulge arcs, the chord fallback for a conic that cannot
    /// take the analytic path, and the wide-polyline stroke band. See
    /// `provider::default::chord_tessellation_params`.
    pub curve_segments: Option<usize>,
    /// Draw `SemanticGeometry::Body3d` as **2D wireframe for bodies**: edges
    /// only (`VSCURRENT WIREFRAME`/`2DWIREFRAME`/`2D`).
    ///
    /// `false` (the session default, `VSCURRENT SHADED`) keeps the filled mesh
    /// plus the stored-edge overlay — exactly the pre-`VSCURRENT` body display.
    /// `true` makes the body fragment emit
    /// [`DisplayPrimitive::LineSegments`] from the body's stored edges, or from
    /// unique triangle-derived segments when a primitive (a sphere/torus) has no
    /// stored boundary edges, so the body is never invisible.
    ///
    /// The flag is **body-only**: every other entity takes its normal path,
    /// there is no hidden-line removal, and no database write, revision,
    /// history entry or `ChangeSet` comes from it.
    pub body_wireframe: bool,
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
            force_chord_conics: false,
            wipeout_frames: false,
            // AutoCAD's documented `FILLMODE`/`QTEXT` initial values: filled
            // areas on, quick text off.
            fill: true,
            qtext: false,
            // `ATTDISP` starts at Normal: visible values draw, invisible ones do
            // not (a constant definition still draws in every mode).
            attdisp: AttributeDisplay::Normal,
            // `None` keeps the build's own tessellation budget, so a caller
            // that never received a `VIEWRES` value builds exactly the scene it
            // built before the token was consumed.
            curve_segments: None,
            // `false` keeps the filled-mesh + stored-edge body display, which is
            // the pre-`VSCURRENT` scene (the session default is `SHADED`).
            body_wireframe: false,
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

    /// Force every conic through the chord path (the SDF soft-revert lever).
    pub fn with_force_chord_conics(mut self) -> Self {
        self.force_chord_conics = true;
        self
    }

    /// Apply the drawing's active annotation scale while building.
    pub fn with_annotation_scale(mut self, scale: AnnotationScaleRef) -> Self {
        self.annotation_scale = Some(scale);
        self
    }

    /// Emit a drawable boundary frame on every WIPEOUT mask (`WIPEOUTFRAME`
    /// 1/2). The display build sets this for `ON` and `DisplayNotPlot`; the
    /// vector-plot build sets it only for `ON`, which keeps `DisplayNotPlot`
    /// out of the plot without a second flag.
    pub fn with_wipeout_frames(mut self, enabled: bool) -> Self {
        self.wipeout_frames = enabled;
        self
    }

    /// Draw filled areas filled (`true`) or outline-only (`false`): `FILL`.
    ///
    /// The default is `true`; a caller passes the session `FILL` token straight
    /// through, so `FILL OFF` reaches the provider as this one flag.
    pub fn with_fill(mut self, enabled: bool) -> Self {
        self.fill = enabled;
        self
    }

    /// Draw text as its placeholder rectangle: `QTEXT`.
    ///
    /// The default is `false` (glyphs as usual). A caller passes the session
    /// `QTEXT` token straight through.
    pub fn with_qtext(mut self, enabled: bool) -> Self {
        self.qtext = enabled;
        self
    }

    /// Set the `ATTDISP` display state for block-attribute values.
    ///
    /// The default is [`AttributeDisplay::Normal`]; a caller passes the session
    /// token straight through. The plot path always uses `Normal`, so a screen
    /// `ATTDISP OFF` cannot hide plotted attribute text.
    pub fn with_attdisp(mut self, mode: AttributeDisplay) -> Self {
        self.attdisp = mode;
        self
    }

    /// Cap chord segments per curve: `VIEWRES`.
    ///
    /// `None` (the default) keeps the build's own tessellation budget; a value
    /// is the session `VIEWRES` percentage turned into a segment ceiling by
    /// [`crate::provider::viewres_segment_ceiling`]. Only chord-tessellated
    /// curves are affected — see [`Self::curve_segments`].
    pub fn with_curve_segments(mut self, ceiling: Option<usize>) -> Self {
        self.curve_segments = ceiling;
        self
    }

    /// Draw 3D bodies as edges only: `VSCURRENT WIREFRAME`.
    ///
    /// The default is `false` (`SHADED`, the pre-`VSCURRENT` filled display). A
    /// caller passes the session visual-style token through as this one flag;
    /// it affects `Body3d` fragments only — see [`Self::body_wireframe`].
    pub fn with_body_wireframe(mut self, enabled: bool) -> Self {
        self.body_wireframe = enabled;
        self
    }
}
