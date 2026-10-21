//! Display primitives and their scalar resolution helpers.

use super::*;

/// One vertex of a viewport-clipped image: a paper-space position plus the
/// texture coordinate that maps to it.
///
/// An unclipped [`DisplayPrimitive::Image`] maps the texture's unit square
/// through its `transform`. When a viewport clip cuts that square, the surviving
/// polygon is carried in [`DisplayPrimitive::Image::clip`] so the texture is
/// interpolated correctly (not stretched) and the hidden part is discarded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageVertex {
    pub position: Point3,
    /// Normalized texture coordinate for this vertex.
    pub uv: [f64; 2],
}

/// An analytic conic in world space: a full circle or an elliptical/arc sweep,
/// parameterised as `p(t) = center + major*cos(t) + minor*sin(t)`.
///
/// `major`/`minor` are conjugate semi-diameter vectors (not necessarily
/// orthogonal or equal length) — the affine image of a circle's two axes. Under
/// an affine map `p -> M*p + c` the parameterisation becomes
/// `(M*center + c) + (M*major)*cos(t) + (M*minor)*sin(t)`, so the angles `start`
/// and `sweep` are unchanged: only `center`/`major`/`minor` transform. That
/// property is what makes a conic the natural carrier for a future analytic
/// (SDF-style) draw; today it is chord-tessellated exactly like the curves it
/// replaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConicSegment {
    pub center: Point3,
    /// Semi-diameter vector along `cos(t)`.
    pub major: Point3,
    /// Conjugate semi-diameter vector along `sin(t)`.
    pub minor: Point3,
    /// Parameter angle (radians) at `t = 0`.
    pub start: f64,
    /// Signed sweep (radians); a negative sweep draws a clockwise/reflected arc.
    pub sweep: f64,
    /// `true` when the conic is a closed full circle (its `start`/`sweep` span a
    /// whole turn).
    pub full: bool,
}

impl ConicSegment {
    /// Whether the semi-diameter axes are finite and non-degenerate, i.e. the
    /// conic actually sweeps an area (not a point or a straight line). This is
    /// the axis half of the future SDF eligibility gate.
    pub fn axes_are_non_degenerate(&self) -> bool {
        let finite = |p: &Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        if !finite(&self.center)
            || !finite(&self.major)
            || !finite(&self.minor)
            || !self.start.is_finite()
            || !self.sweep.is_finite()
        {
            return false;
        }
        let major_len = cad_geometry::length(self.major);
        let minor_len = cad_geometry::length(self.minor);
        // Components are finite here, so the lengths are finite: `<=` rejects a
        // zero axis without letting a NaN through.
        if major_len <= 1e-12 || minor_len <= 1e-12 {
            return false;
        }
        // The axes must span a plane: a parallel pair collapses to a segment.
        let cross_len = cad_geometry::length(cad_geometry::cross(self.major, self.minor));
        cross_len > 1e-9 * major_len * minor_len
    }

    /// Whether this conic is a **slice-1 SDF candidate**: a circle or circular
    /// arc whose two semi-diameter axes are equal-length and orthogonal, with a
    /// non-degenerate angular span.
    ///
    /// Slice 1 (see `docs/adr/0008-sdf-conic-rendering.md`) renders with an
    /// orthonormal `u`/`v̂` frame and a radial distance
    /// `length(vec2(x, y)) - r`; that is the correct signed distance only when
    /// `|major| == |minor|` and `major ⟂ minor` (a circle). An **ellipse**
    /// (`|major| != |minor|`) is slice 2 and stays chord-tessellated, because
    /// the normalized-`v` radial form would draw a circle of the major radius.
    ///
    /// A zero-length arc (`sweep == 0 && !full`) is degenerate and excluded so
    /// it takes the chord path (which reports it as `Missing`), never a
    /// fabricated SDF instance.
    pub fn is_sdf_circle_arc(&self) -> bool {
        if !self.axes_are_non_degenerate() {
            return false;
        }
        if !self.full && self.sweep == 0.0 {
            return false;
        }
        let major_len = cad_geometry::length(self.major);
        let minor_len = cad_geometry::length(self.minor);
        let scale = major_len.max(minor_len);
        if (major_len - minor_len).abs() > 1e-6 * scale {
            return false;
        }
        let u = cad_geometry::scale(self.major, 1.0 / major_len);
        let v = cad_geometry::scale(self.minor, 1.0 / minor_len);
        cad_geometry::dot(u, v).abs() <= 1e-6
    }

    /// The equivalent [`SemanticGeometry::Ellipse`] so the legacy chord
    /// tessellation can be reused unchanged.
    ///
    /// The minor axis is rebuilt from the plane normal, matching
    /// [`cad_geometry::tessellate_geometry`]'s own ellipse frame, so the chord
    /// points are the same as the pre-conic path.
    pub fn to_semantic_geometry(&self) -> SemanticGeometry {
        let major_len = cad_geometry::length(self.major);
        let minor_len = cad_geometry::length(self.minor);
        let normal = {
            let c = cad_geometry::cross(self.major, self.minor);
            let len = cad_geometry::length(c);
            if len >= 1e-24 {
                cad_geometry::scale(c, 1.0 / len)
            } else {
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                }
            }
        };
        SemanticGeometry::Ellipse {
            center: self.center,
            normal,
            major_axis: self.major,
            ratio: if major_len > 0.0 {
                minor_len / major_len
            } else {
                0.0
            },
            start: self.start,
            sweep: if self.full {
                std::f64::consts::TAU
            } else {
                self.sweep
            },
        }
    }

    /// Chord-tessellate this conic with `params`, reusing the legacy curve
    /// tessellation so the current (chord) draw is unchanged.
    pub fn tessellate(&self, params: TessellationParams) -> Vec<Point3> {
        tessellate_geometry(&self.to_semantic_geometry(), params)
    }

    /// Chord-tessellate with the default tolerance, for consumers that have no
    /// tolerance policy of their own (the scene cache / vector export safety
    /// net). Normal builds never reach them: `build_expanded` already expands a
    /// conic to chords with the build's tolerance.
    pub fn chord_points(&self) -> Vec<Point3> {
        self.tessellate(TessellationParams::default())
    }
}

/// One drawable piece of an entity, in world coordinates.
pub enum DisplayPrimitive {
    Lines(Arc<[Point3]>),
    /// Independent endpoint pairs. Unlike `Lines`, adjacent pairs are not joined.
    LineSegments(Arc<[Point3]>),
    /// An analytic conic (circle/arc/ellipse). Named `Conic`, not `Arc`, to
    /// avoid colliding with [`std::sync::Arc`] in this enum.
    Conic(Arc<ConicSegment>),
    Mesh(Arc<Mesh>),
    /// Filled mesh that deliberately omits the per-triangle-edge overlay.
    /// Used for triangulated font glyphs, where the overlay would expose
    /// internal tessellation edges.
    MeshWithoutEdgeOverlay(Arc<Mesh>),
    Text {
        text: String,
        origin: Point3,
        font: ResourceKey,
        height: f64,
    },
    Image {
        resource: ResourceKey,
        transform: Transform3,
        /// `None` while the whole image is drawn through its unit-square
        /// `transform`. `Some` when the image is bounded by an entity clip
        /// and/or a viewport cut: a convex polygon (paper-space positions for a
        /// viewport cut, world space otherwise) with per-vertex texture
        /// coordinates, so only the visible texture region is mapped and its
        /// UVs are interpolated at the new vertices.
        clip: Option<Arc<[ImageVertex]>>,
    },
    Instance {
        block: BlockId,
        transform: Transform3,
    },
    /// An erasing mask (DXF `WIPEOUT`): a **colourless** closed world-space
    /// boundary polygon whose interior (or, when `inverted`, exterior) hides
    /// everything painted beneath it.
    ///
    /// It carries no colour because "the background" belongs to the renderer
    /// (`cad-representation` stays GPU/host independent) and no depth because the
    /// mask pass paints last with `depth_compare: Always` (see
    /// `docs/adr/0022-wipeout-mask-rendering.md`). The renderer fills the region
    /// with its own clear colour, tessellating concave boundaries with the same
    /// even-odd fill solid HATCH uses.
    ///
    /// `inverted` mirrors the DXF clip mode (Inside = mask the outside). The
    /// renderer expresses the complement without a stencil: it adds a
    /// viewport-covering ring to the same even-odd fill, so points inside
    /// `boundary` are covered twice and stay visible.
    Mask {
        boundary: Arc<[Point3]>,
        inverted: bool,
    },
}

/// The named completeness reason for a WIPEOUT whose boundary cannot be filled
/// (fewer than three vertices).
pub const MASK_BOUNDARY_DEGENERATE: &str =
    "wipeout mask boundary has fewer than three vertices; the mask fill is not drawn";

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
            DisplayPrimitive::LineSegments(points) => DisplayPrimitive::LineSegments(Arc::from(
                points
                    .iter()
                    .map(|p| transform.apply_point(*p))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )),
            // An affine map sends `center + major*cos t + minor*sin t` to
            // `(M*center + c) + (M*major)*cos t + (M*minor)*sin t`: the angles
            // pass through unchanged, only the center and semi-diameters move.
            DisplayPrimitive::Conic(conic) => DisplayPrimitive::Conic(Arc::new(ConicSegment {
                center: transform.apply_point(conic.center),
                major: transform.apply_vector(conic.major),
                minor: transform.apply_vector(conic.minor),
                start: conic.start,
                sweep: conic.sweep,
                full: conic.full,
            })),
            DisplayPrimitive::Mesh(mesh) => {
                DisplayPrimitive::Mesh(Arc::new(transform_mesh(mesh, transform)))
            }
            DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(transform_mesh(mesh, transform)))
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
                clip,
            } => DisplayPrimitive::Image {
                resource: resource.clone(),
                transform: transform.matrix_mul(local),
                clip: clip.as_ref().map(|vertices| {
                    Arc::from(
                        vertices
                            .iter()
                            .map(|vertex| ImageVertex {
                                position: transform.apply_point(vertex.position),
                                uv: vertex.uv,
                            })
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    )
                }),
            },
            DisplayPrimitive::Instance {
                block,
                transform: local,
            } => DisplayPrimitive::Instance {
                block: *block,
                transform: transform.matrix_mul(local),
            },
            // A mask boundary is a world-space polygon: the affine map is applied
            // per vertex. The fill is derived from these vertices by the
            // renderer, so no other state changes (`inverted` is view-independent).
            DisplayPrimitive::Mask { boundary, inverted } => DisplayPrimitive::Mask {
                boundary: Arc::from(
                    boundary
                        .iter()
                        .map(|p| transform.apply_point(*p))
                        .collect::<Vec<Point3>>()
                        .into_boxed_slice(),
                ),
                inverted: *inverted,
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
    /// Resolved dash pattern. An empty `elements` list means continuous.
    ///
    /// The importer resolves an explicit entity linetype and a reachable layer
    /// linetype into a concrete pattern; `build` has no database access so it
    /// emits [`LinetypePattern::continuous`]. This field is carried onto the
    /// scene batch for diagnostics and, for line geometry, drives the
    /// arc-length dash subdivision performed by
    /// [`ProviderRegistry::build_expanded`].
    pub linetype: LinetypePattern,
    /// `true` when the source linetype was symbolic (`ByLayer`/`ByBlock`) and
    /// no concrete pattern was available, so the fragment is drawn continuous.
    pub linetype_unresolved: bool,
    /// Entity linetype scale factor, applied with the drawing's global LTSCALE
    /// when subdividing. Sanitised to `>= 0`.
    pub linetype_scale: f32,
    /// Display order of the source entity, i.e. the same key `cad-db` sorts a
    /// space by (`DbEntity::draw_order`). Larger values paint later.
    ///
    /// Threaded here so the scene can carry it onto
    /// `RenderBatch::draw_order`/[`crate::DisplayPrimitive::Mask`] batches
    /// instead of emitting a constant. The renderer still paints every mask in
    /// its final pass (ADR 0022 D2); this field only removes the data loss so a
    /// follow-up can interleave masks with geometry at their real order.
    ///
    /// INSERT expansion makes every child fragment inherit the **INSERT's**
    /// order, because the database orders by the INSERT entity, not by the
    /// block's children.
    pub draw_order: i64,
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

/// Resolve an entity's stored linetype against an enclosing INSERT's resolved
/// pattern, mirroring [`resolve_color`].
///
/// Returns the concrete pattern, whether it is unresolved (drawn continuous),
/// and the entity's own linetype scale. An explicit pattern is concrete;
/// `ByBlock` inherits the enclosing reference's pattern when one is threaded
/// down, otherwise it is unresolved. `ByLayer` is unresolved here because this
/// layer has no layer table; the importer already substituted the layer pattern
/// for real imports, so this is only reached for hand-built databases.
pub fn resolve_linetype(
    linetype: EntityLineType,
    parent: Option<&LinetypePattern>,
) -> (LinetypePattern, bool, f32) {
    match linetype {
        EntityLineType::Explicit {
            pattern,
            scale,
            name: _,
        } => {
            let scale = if scale.is_finite() && scale > 0.0 {
                scale as f32
            } else {
                1.0
            };
            (pattern, false, scale)
        }
        EntityLineType::ByBlock => match parent {
            Some(parent) => (parent.clone(), false, 1.0),
            None => (LinetypePattern::continuous(), true, 1.0),
        },
        EntityLineType::ByLayer => (LinetypePattern::continuous(), true, 1.0),
    }
}

/// Subdivide a world-space polyline into dash sub-polylines.
///
/// The pattern is scaled by `scale * global_scale`. When the pattern is
/// continuous, degenerate, all-gap or the scale is invalid, the input is
/// returned unchanged together with an optional reason string so the caller can
/// report a `Partial` completeness. `None` means "continuous, nothing to
/// report" (the exact/expected case); `Some(reason)` means the caller asked for
/// dashes but they could not be produced exactly.
///
/// Runs shorter than two distinct points are dropped; a dot collapses to
/// nothing (a hairline renderer has no length to draw).
pub fn subdivide_dashes(
    points: &[Point3],
    pattern: &LinetypePattern,
    scale: f64,
    global_scale: f64,
) -> (Vec<Vec<Point3>>, Option<String>) {
    if pattern.is_continuous() {
        return (vec![points.to_vec()], None);
    }
    let combined = scale * global_scale;
    match cad_geometry::dash_polyline(points, &pattern.elements, combined) {
        cad_geometry::DashOutcome::Dashed(runs) => (runs, None),
        cad_geometry::DashOutcome::Continuous(issue) => {
            (vec![points.to_vec()], Some(issue.reason().to_string()))
        }
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
