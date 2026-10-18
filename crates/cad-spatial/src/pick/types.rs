use cad_domain::*;

use crate::PickHit;

/// Ray directions shorter than this are considered degenerate.
pub const MIN_RAY_DIRECTION: f64 = 1e-12;
/// Maximum number of tessellation segments for one curve during picking, so a
/// pathological curve can never make a single pick unbounded.
pub const MAX_PICK_SEGMENTS: usize = 8192;

/// How a ray treats the back face of a triangle.
///
/// The renderer uses `cull_mode = Back` with a mirrored-front-face variant, so
/// a pick that must agree with what the user sees has to state its policy
/// explicitly instead of leaving it implicit in the winding sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackFacePolicy {
    /// Ignore triangles whose outward side faces away from the ray origin.
    Cull,
    /// Accept both sides of a triangle (for open sheets / single-sided meshes).
    DoubleSided,
}

/// World-space picking parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickOptions {
    /// Maximum world distance from the ray at which a segment/curve counts as
    /// hit. Derived from the [`TolerancePolicy`] by the caller (see
    /// `cad_app::picking::pick_tolerance`).
    pub tolerance: f64,
    /// Back-face treatment for mesh triangles.
    pub back_faces: BackFacePolicy,
}

impl PickOptions {
    /// Validated options with back-face culling (matching the renderer default).
    pub fn new(tolerance: f64) -> CadResult<Self> {
        let options = PickOptions {
            tolerance,
            back_faces: BackFacePolicy::Cull,
        };
        options.validate()?;
        Ok(options)
    }

    /// Override the back-face policy.
    pub fn with_back_faces(mut self, policy: BackFacePolicy) -> Self {
        self.back_faces = policy;
        self
    }

    /// Reject a non-finite or non-positive tolerance (audit: never fake a hit
    /// with a degenerate tolerance).
    pub fn validate(&self) -> CadResult<()> {
        if !self.tolerance.is_finite() || self.tolerance <= 0.0 {
            return Err(CadError::InvalidInput(
                "pick tolerance must be finite and > 0".into(),
            ));
        }
        Ok(())
    }
}

/// One object offered to the precise pick stage.
#[derive(Debug, Clone)]
pub struct PickItem {
    /// The identity a hit reports (carries the `InstancePath` for INSERTs).
    pub source: SelectionRef,
    /// The geometry in its own local coordinates.
    pub geometry: SemanticGeometry,
    /// The accumulated world transform for this item (identity for a
    /// top-level model-space entity; the composed insert chain otherwise).
    pub transform: Transform3,
    /// Where the geometry came from, carried through to the hit.
    pub geometry_source: GeometrySource,
}

/// Machine key: a mesh carried no `face_sources` list at all, so a triangle
/// index cannot be mapped to a stable sub-element.
pub const REASON_FACE_SOURCES_ABSENT: &str = "mesh-face-sources-absent";
/// Machine key: the mesh has a `face_sources` list but it is shorter than its
/// triangle list, so the hit triangle has no corresponding entry.
pub const REASON_FACE_SOURCE_OUT_OF_RANGE: &str = "mesh-face-source-index-out-of-range";
/// Machine key: `face_sources[triangle]` is explicitly `None`: the producer did
/// not attach a stable id to that face.
pub const REASON_FACE_SOURCE_MISSING: &str = "mesh-face-source-missing";

/// A precise hit with its depth along the ray.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryHit {
    /// The world point on the geometry closest to the ray.
    pub point: Point3,
    /// Distance along the (unit) ray direction. Always `>= 0`.
    pub distance: f64,
    /// Perpendicular (screen-space) distance from the ray to the hit point,
    /// used only to break a tie between hits at the same depth.
    pub offset: f64,
    pub precision: Precision,
    pub geometry_source: GeometrySource,
    /// The stable sub-element of the hit geometry, when the geometry carries
    /// one. Only meshes are sub-element-addressable here: the hit triangle is
    /// mapped through [`Mesh::face_sources`]. Lines/curves/points always yield
    /// `None` and are not sub-element-addressable.
    pub sub_element: Option<SubElementId>,
    /// Machine key explaining `sub_element == None` for a mesh face that has no
    /// stable source (`face_sources` absent / short / entry `None`). `None`
    /// when a sub-element was resolved or when the geometry has no faces.
    pub sub_element_reason: Option<&'static str>,
}

/// The verdict of a precise test for one item.
#[derive(Debug, Clone, PartialEq)]
pub enum PickOutcome {
    /// The ray hit the geometry within tolerance.
    Hit(GeometryHit),
    /// The ray did not hit within tolerance.
    Miss,
    /// The geometry cannot be tested precisely; `reason` is a machine key.
    Unsupported(&'static str),
}

/// An item that was skipped because it cannot be picked precisely.
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedGeometry {
    pub source: SelectionRef,
    /// Machine-readable reason (never translated prose).
    pub reason: &'static str,
}

/// The result of a closest-hit query over many items.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickReport {
    /// The closest hit, or `None` when nothing was hit.
    pub hit: Option<PickHit>,
    /// Items that could not be tested precisely, with the reason each was
    /// skipped. Never conflated with a `Miss`.
    pub skipped: Vec<SkippedGeometry>,
}
