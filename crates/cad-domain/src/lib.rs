//! Platform-independent identities and semantic contracts.
use std::fmt;
macro_rules! ids {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u128);
    )*};
}
ids!(
    DocumentId,
    DatabaseId,
    ObjectId,
    EntityId,
    LayerId,
    BlockId,
    LayoutId,
    StyleId,
    LinetypeId,
    ScaleId,
    ViewportId,
    TransactionId,
    RequestId,
    ResourceId
);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);

/// Content identity of a drawing database for cache/scene staleness checks.
///
/// Two opens with the same `DatabaseId` but different content produce different
/// identities, so derived render state is rebuilt instead of showing stale
/// geometry (audit B04).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneIdentity {
    pub database: DatabaseId,
    pub revision: Revision,
    pub entities: u64,
    pub layers: u64,
    pub bounds: [Point3; 2],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadError {
    NotImplemented(&'static str),
    InvalidInput(String),
    Unsupported(String),
    ResourceMissing(String),
    CorruptData(String),
    GpuFailure(String),
    Invariant(String),
    PermissionDenied,
    Cancelled,
    StaleResult,
}
impl fmt::Display for CadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CadError {}
pub type CadResult<T> = Result<T, CadError>;
pub fn pending<T>(feature: &'static str) -> CadResult<T> {
    Err(CadError::NotImplemented(feature))
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform3 {
    pub matrix: [[f64; 4]; 4],
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds3 {
    pub min: Point3,
    pub max: Point3,
}
/// Clip boundary of a raster `IMAGE`, in the image's own pixel coordinate space.
///
/// `vertices` is a closed polygon; `inside` mirrors DXF clip_mode: `true` keeps
/// the region inside the boundary, `false` keeps the region outside it (which is
/// what a WIPEOUT-style inverted mask expresses).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageClip {
    pub vertices: Vec<[f64; 2]>,
    pub inside: bool,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkPlane {
    pub origin: Point3,
    pub u: Point3,
    pub v: Point3,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray3 {
    pub origin: Point3,
    pub direction: Point3,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceId {
    Model,
    Paper(LayoutId),
    /// Geometry owned by a block definition. Not drawn directly; it is only
    /// reached through an `INSERT` (see audit B15).
    Block(BlockId),
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstancePath(pub Vec<EntityId>);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubElementId {
    pub source_key: String,
    pub topology_revision: Revision,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRef {
    pub document: DocumentId,
    pub entity: EntityId,
    pub instance: InstancePath,
    pub sub_element: Option<SubElementId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometrySource {
    Analytic,
    DirectMesh,
    ProxyCache,
    KernelMesh,
    UserPoints,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Precision {
    Analytic,
    Approximate { error_bound: Option<f64> },
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportStatus {
    NotImplemented,
    Unsupported,
    Unverified,
    Partial,
    Verified,
}
#[derive(Debug, Clone)]
pub struct EntityCapability {
    pub type_key: String,
    pub read: SupportStatus,
    pub semantic: SupportStatus,
    pub render: SupportStatus,
    pub pick: SupportStatus,
    pub measure: SupportStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Partial(Vec<String>),
    Missing(Vec<String>),
    Unverified,
}

impl Completeness {
    fn severity(&self) -> u8 {
        match self {
            Completeness::Complete => 0,
            Completeness::Partial(_) => 1,
            Completeness::Unverified => 2,
            Completeness::Missing(_) => 3,
        }
    }

    fn reasons(&self) -> &[String] {
        match self {
            Completeness::Partial(v) | Completeness::Missing(v) => v,
            _ => &[],
        }
    }

    /// Combine two completeness verdicts, keeping the more severe and merging
    /// any reasons. Used when one source entity yields several primitives.
    pub fn combine(self, other: Completeness) -> Completeness {
        if self.severity() < other.severity() {
            return other.combine(self);
        }
        match self {
            Completeness::Partial(mut v) => {
                v.extend_from_slice(other.reasons());
                Completeness::Partial(v)
            }
            Completeness::Missing(mut v) => {
                v.extend_from_slice(other.reasons());
                Completeness::Missing(v)
            }
            keep => keep,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub object: Option<ObjectId>,
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone)]
pub struct Registration {
    pub type_key: String,
    pub version: u32,
    pub priority: i32,
    pub entity_types: Vec<String>,
    pub capabilities: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    DrawingUnits,
    Millimeter,
    Meter,
    Inch,
    Foot,
}
#[derive(Debug, Clone, PartialEq)]
pub struct UnitContext {
    pub source: Unit,
    pub display: Unit,
    pub display_per_source: Option<f64>,
    pub decimal_places: u8,
}
#[derive(Debug, Clone, Copy)]
pub struct TolerancePolicy {
    pub computation_world: f64,
    pub topology_world: f64,
    pub display_pixels: f64,
    pub interaction_logical_pixels: f64,
    pub formatting_decimals: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentIdentity {
    Temporary(u128),
    Sha256([u8; 32]),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStamp {
    pub document: DocumentId,
    pub generation: u64,
    pub object_revision: Revision,
    pub dependency_version: u64,
    pub configuration_version: u64,
}
impl TaskStamp {
    pub fn validate(&self, current: &Self) -> CadResult<()> {
        if self == current {
            Ok(())
        } else {
            Err(CadError::StaleResult)
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlignH {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlignV {
    #[default]
    Baseline,
    Bottom,
    Middle,
    Top,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticGeometry {
    Line {
        start: Point3,
        end: Point3,
    },
    Polyline {
        points: Vec<Point3>,
        bulges: Vec<f64>,
        closed: bool,
    },
    Circle {
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        center: Point3,
        /// Plane normal (extrusion). The ellipse lies in the plane through
        /// `center` with this normal; the minor axis is
        /// `cross(normal, major_axis)`, so an ellipse on an arbitrary OCS plane
        /// keeps its true orientation instead of being folded into world XY
        /// (audit B23).
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Spline {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<Point3>,
        weights: Vec<f64>,
    },
    Point(Point3),
    Mesh(Mesh),
    Insert {
        block: BlockId,
        transform: Transform3,
    },
    Text {
        text: String,
        position: Point3,
        style: StyleId,
        height: f64,
        rotation: f64,
        /// Resource key of the font this text uses (for example `arial.ttf`),
        /// when the source style declares one. Display providers resolve it
        /// through the font catalog; `None` means the font is unknown.
        font: Option<String>,
        /// Horizontal placement of the run relative to `position`.
        h_align: TextAlignH,
        /// Vertical placement of the run relative to `position`.
        v_align: TextAlignV,
    },
    /// A SHAPE entity: one glyph of a compiled SHX shape font, referenced by
    /// shape code (or, when `code` is 0, by `shape_name`). `font` is the style's
    /// font key; the representation layer resolves and outlines it.
    Shape {
        shape_name: String,
        code: u32,
        position: Point3,
        size: f64,
        rotation: f64,
        font: Option<String>,
    },
    Opaque {
        type_key: String,
        version: u32,
        payload: Vec<u8>,
    },
    /// Sub-geometries belonging to a single source entity, rendered in order.
    ///
    /// Used where one imported entity yields several primitives (for example a
    /// HATCH's boundary loops plus its fill or pattern). Consumers recurse.
    Compound(Vec<SemanticGeometry>),
    /// A raster image (DXF `IMAGE` / `AcDbRasterImage`).
    ///
    /// The image is placed by `origin` and two edge vectors: `u` and `v` span
    /// the image's local axes, and their lengths are one pixel's world size, so
    /// the full extent is `pixels.0 * |u|` by `pixels.1 * |v|`. `pixels` is
    /// `(width, height)` in pixels. `file` is a *logical* resource key resolved
    /// by the host, never a platform path (the same role as `Text::font`).
    /// `visible` mirrors the SHOW_IMAGE flag.
    Image {
        origin: Point3,
        u: Point3,
        v: Point3,
        pixels: [f64; 2],
        file: Option<String>,
        clip: Option<ImageClip>,
        visible: bool,
    },
    /// An erasing mask (DXF `WIPEOUT`).
    ///
    /// `boundary` is a closed world-space polygon; `inverted` corresponds to
    /// clip_mode Inside (mask the outside). The renderer floods it with the
    /// viewport background colour, so it carries no colour of its own and the
    /// representation layer maps it to a mask primitive rather than a coloured
    /// mesh.
    Mask {
        boundary: Vec<Point3>,
        inverted: bool,
    },
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    pub normals: Vec<Point3>,
    pub face_sources: Vec<Option<SubElementId>>,
    /// Optional per-vertex sRGB colours, one byte per channel.
    ///
    /// Empty (the default) means "no vertex colour": the renderer falls back to
    /// the batch's uniform colour. When non-empty it must be exactly
    /// `vertices.len()` long; this is the channel a gradient HATCH uses to bake
    /// its per-stop colours into geometry so no texture path is needed. Bytes
    /// are used rather than floats so the domain stays independent of the
    /// renderer's colour conventions.
    pub colors: Vec<[u8; 3]>,
}

// ---------------------------------------------------------------------------
// Defaults and helpers.
//
// The contract types above are intentionally plain data; these impls provide
// the neutral defaults the rest of the system builds on. They deliberately do
// not encode policy (e.g. no unit is assumed to be millimetres).
// ---------------------------------------------------------------------------

impl Default for TolerancePolicy {
    fn default() -> Self {
        TolerancePolicy {
            // world-space predicate tolerance; scales with coordinate magnitude
            computation_world: 1e-9,
            // topology join tolerance (larger than the predicate tolerance)
            topology_world: 1e-6,
            // display discretisation budget in logical pixels
            display_pixels: 0.6,
            // interaction/snap radius in logical pixels
            interaction_logical_pixels: 12.0,
            // decimal places for user-facing numbers
            formatting_decimals: 3,
        }
    }
}

impl Default for UnitContext {
    fn default() -> Self {
        UnitContext {
            source: Unit::DrawingUnits,
            display: Unit::DrawingUnits,
            display_per_source: None,
            decimal_places: 3,
        }
    }
}

impl UnitContext {
    /// Unknown units: values are drawing units, never assumed millimetres.
    pub fn drawing_units() -> Self {
        Self::default()
    }

    /// A known unit context where one source unit equals one metre.
    pub fn from_meter() -> Self {
        UnitContext {
            source: Unit::Meter,
            display: Unit::Meter,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }
    }

    /// Human-facing unit label.
    pub fn label(&self) -> &'static str {
        match self.display {
            Unit::DrawingUnits => "drawing units",
            Unit::Millimeter => "mm",
            Unit::Meter => "m",
            Unit::Inch => "in",
            Unit::Foot => "ft",
        }
    }

    /// Convert a finite source length using a known, finite, positive ratio.
    /// Unknown or invalid ratios and non-finite results return `None`.
    pub fn to_display(&self, source_value: f64) -> Option<f64> {
        let ratio = self.display_per_source?;
        if !source_value.is_finite() || !ratio.is_finite() || ratio <= 0.0 {
            return None;
        }
        let value = source_value * ratio;
        value.is_finite().then_some(value)
    }
}

impl Transform3 {
    pub fn identity() -> Self {
        let mut m = [[0.0f64; 4]; 4];
        m[0][0] = 1.0;
        m[1][1] = 1.0;
        m[2][2] = 1.0;
        m[3][3] = 1.0;
        Transform3 { matrix: m }
    }

    pub fn translation(t: Point3) -> Self {
        let mut m = Self::identity().matrix;
        m[0][3] = t.x;
        m[1][3] = t.y;
        m[2][3] = t.z;
        Transform3 { matrix: m }
    }

    /// Uniform scale about the origin.
    pub fn scale(s: f64) -> Self {
        let mut m = Self::identity().matrix;
        m[0][0] = s;
        m[1][1] = s;
        m[2][2] = s;
        Transform3 { matrix: m }
    }

    /// Build a transform from two linear basis vectors and an origin.
    ///
    /// Column 0 is `x`, column 1 is `y`, the translation is `origin`, and column
    /// 2 is `normalize(cross(x, y))` so the frame is right-handed with a unit
    /// plane normal. The unit square `(s, t)` with `s, t` in `[0, 1]` therefore
    /// maps to `origin + x*s + y*t`.
    ///
    /// A degenerate pair (zero-length or non-finite cross product) leaves the Z
    /// column at zero rather than inventing an axis; callers that need a defined
    /// plane normal must reject that case before using the result.
    pub fn from_basis(x: Point3, y: Point3, origin: Point3) -> Self {
        let normal = {
            let c = Point3 {
                x: x.y * y.z - x.z * y.y,
                y: x.z * y.x - x.x * y.z,
                z: x.x * y.y - x.y * y.x,
            };
            let len = c.x.hypot(c.y).hypot(c.z);
            if len.is_finite() && len > 1e-24 {
                Point3 {
                    x: c.x / len,
                    y: c.y / len,
                    z: c.z / len,
                }
            } else {
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                }
            }
        };
        let mut m = [[0.0f64; 4]; 4];
        m[0][0] = x.x;
        m[1][0] = x.y;
        m[2][0] = x.z;
        m[0][1] = y.x;
        m[1][1] = y.y;
        m[2][1] = y.z;
        m[0][2] = normal.x;
        m[1][2] = normal.y;
        m[2][2] = normal.z;
        m[0][3] = origin.x;
        m[1][3] = origin.y;
        m[2][3] = origin.z;
        m[3][3] = 1.0;
        Transform3 { matrix: m }
    }

    /// Compose: apply `rhs` first, then `self`.
    pub fn matrix_mul(&self, rhs: &Transform3) -> Transform3 {
        let mut out = [[0.0f64; 4]; 4];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                let mut acc = 0.0;
                for k in 0..4 {
                    acc += self.matrix[r][k] * rhs.matrix[k][c];
                }
                *cell = acc;
            }
        }
        Transform3 { matrix: out }
    }

    pub fn apply_point(&self, p: Point3) -> Point3 {
        let m = &self.matrix;
        Point3 {
            x: m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
            y: m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
            z: m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
        }
    }

    /// Determinant of the 3x3 linear part (translation excluded).
    ///
    /// Used to reject singular mappings that cannot be inverted (audit B10).
    pub fn determinant(&self) -> f64 {
        let m = &self.matrix;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Whether the 3x3 linear part is a similarity (uniform scale + rotation +
    /// optional mirror), i.e. it maps every circle to a circle.
    ///
    /// Judgment is ratio-based so it is independent of the transform's scale:
    /// the three column lengths must agree with each other and the columns must
    /// be mutually orthogonal. A non-uniform scale, shear or singular matrix is
    /// not uniform. This is the predicate a caller must use before assuming a
    /// `Circle`/`Arc` stays a circle (audit B23).
    /// Invalid (negative or non-finite) relative tolerances are rejected.
    pub fn is_uniform_scale(&self, tolerance: f64) -> bool {
        let scale = self.max_scale();
        if !scale.is_finite() || scale <= 0.0 || !tolerance.is_finite() || tolerance < 0.0 {
            return false;
        }
        let m = &self.matrix;
        // Normalise before squaring to avoid overflow and underflow at extreme
        // finite scales. Every normalised component has magnitude at most one.
        let col = |i: usize| Point3 {
            x: m[0][i] / scale,
            y: m[1][i] / scale,
            z: m[2][i] / scale,
        };
        let c0 = col(0);
        let c1 = col(1);
        let c2 = col(2);
        let dot3 = |a: Point3, b: Point3| a.x * b.x + a.y * b.y + a.z * b.z;
        let l0 = dot3(c0, c0).sqrt();
        let l1 = dot3(c1, c1).sqrt();
        let l2 = dot3(c2, c2).sqrt();
        if l0 == 0.0 || l1 == 0.0 || l2 == 0.0 {
            return false;
        }
        let det = c0.x * (c1.y * c2.z - c1.z * c2.y) - c1.x * (c0.y * c2.z - c0.z * c2.y)
            + c2.x * (c0.y * c1.z - c0.z * c1.y);
        if det == 0.0 {
            return false;
        }
        let rel = |a: f64, b: f64| (a - b).abs();
        if rel(l0, l1) > tolerance || rel(l1, l2) > tolerance {
            return false;
        }
        // The columns are already relative to the overall scale.
        let ortho = |a: Point3, b: Point3| dot3(a, b).abs() <= tolerance;
        ortho(c0, c1) && ortho(c1, c2) && ortho(c0, c2)
    }

    /// Largest absolute coefficient of the 3x3 linear part.
    /// This normalises scale comparisons; it is not an operator-norm bound for
    /// arbitrary rotations or shears. Non-finite coefficients are propagated.
    pub fn max_scale(&self) -> f64 {
        let m = &self.matrix;
        let mut max = 0.0f64;
        for (i, j) in [
            (0, 0),
            (0, 1),
            (0, 2),
            (1, 0),
            (1, 1),
            (1, 2),
            (2, 0),
            (2, 1),
            (2, 2),
        ] {
            if m[i][j].is_nan() {
                return f64::NAN;
            }
            max = max.max(m[i][j].abs());
        }
        max
    }
}

impl Default for Transform3 {
    fn default() -> Self {
        Self::identity()
    }
}

impl Default for Registration {
    fn default() -> Self {
        Registration {
            type_key: String::new(),
            version: 1,
            priority: 0,
            entity_types: Vec::new(),
            capabilities: Vec::new(),
        }
    }
}

impl TaskStamp {
    pub fn new(document: DocumentId, generation: u64) -> Self {
        TaskStamp {
            document,
            generation,
            object_revision: Revision(0),
            dependency_version: 0,
            configuration_version: 0,
        }
    }
}

impl SpaceId {
    pub fn is_model(&self) -> bool {
        matches!(self, SpaceId::Model)
    }
}

impl DocumentIdentity {
    /// Short human-facing identity for diagnostics.
    pub fn short(&self) -> String {
        match self {
            DocumentIdentity::Temporary(id) => format!("tmp-{id:032x}"),
            DocumentIdentity::Sha256(bytes) => {
                let mut s = String::with_capacity(16);
                for b in bytes.iter().take(8) {
                    s.push_str(&format!("{b:02x}"));
                }
                s
            }
        }
    }
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn sample_image() -> SemanticGeometry {
        SemanticGeometry::Image {
            origin: point(1.0, 2.0, 0.0),
            u: point(0.5, 0.0, 0.0),
            v: point(0.0, 0.25, 0.0),
            pixels: [640.0, 480.0],
            file: Some("logo.png".to_string()),
            clip: Some(ImageClip {
                vertices: vec![[0.0, 0.0], [640.0, 0.0], [640.0, 480.0]],
                inside: true,
            }),
            visible: true,
        }
    }

    fn sample_mask() -> SemanticGeometry {
        SemanticGeometry::Mask {
            boundary: vec![
                point(0.0, 0.0, 0.0),
                point(10.0, 0.0, 0.0),
                point(10.0, 10.0, 0.0),
                point(0.0, 10.0, 0.0),
            ],
            inverted: false,
        }
    }

    #[test]
    fn image_variant_constructs_and_compares() {
        let image = sample_image();
        assert_eq!(image.clone(), image);
        match &image {
            SemanticGeometry::Image {
                origin,
                pixels,
                file,
                clip,
                visible,
                ..
            } => {
                assert_eq!(*origin, point(1.0, 2.0, 0.0));
                assert_eq!(*pixels, [640.0, 480.0]);
                assert_eq!(file.as_deref(), Some("logo.png"));
                assert!(clip.as_ref().expect("clip").inside);
                assert!(*visible);
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn image_clip_constructs_and_compares() {
        let clip = ImageClip {
            vertices: vec![[1.0, 2.0], [3.0, 4.0]],
            inside: false,
        };
        assert_eq!(clip.clone(), clip);
        assert_eq!(
            clip,
            ImageClip {
                vertices: vec![[1.0, 2.0], [3.0, 4.0]],
                inside: false,
            }
        );
        assert_ne!(
            clip,
            ImageClip {
                vertices: vec![[1.0, 2.0], [3.0, 4.0]],
                inside: true,
            }
        );
    }

    #[test]
    fn mask_variant_constructs_and_compares() {
        let mask = sample_mask();
        assert_eq!(mask.clone(), mask);
        match &mask {
            SemanticGeometry::Mask { boundary, inverted } => {
                assert_eq!(boundary.len(), 4);
                assert!(!*inverted);
            }
            other => panic!("expected Mask, got {other:?}"),
        }
    }
}
