//! Display primitives and their scalar resolution helpers.

use super::*;

/// One drawable piece of an entity, in world coordinates.
pub enum DisplayPrimitive {
    Lines(Arc<[Point3]>),
    /// Independent endpoint pairs. Unlike `Lines`, adjacent pairs are not joined.
    LineSegments(Arc<[Point3]>),
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
            DisplayPrimitive::LineSegments(points) => DisplayPrimitive::LineSegments(Arc::from(
                points
                    .iter()
                    .map(|p| transform.apply_point(*p))
                    .collect::<Vec<_>>()
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
