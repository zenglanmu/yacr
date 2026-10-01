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
    AnnotationId,
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
    },
    Opaque {
        type_key: String,
        version: u32,
        payload: Vec<u8>,
    },
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    pub normals: Vec<Point3>,
    pub face_sources: Vec<Option<SubElementId>>,
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

    /// Convert a source length to display units, when the ratio is known.
    pub fn to_display(&self, source_value: f64) -> Option<f64> {
        self.display_per_source.map(|r| source_value * r)
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
