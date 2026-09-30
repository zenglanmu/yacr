//! Platform-independent identities and semantic contracts.
use std::fmt;
macro_rules! ids {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u128);
    )*};
}
ids!(DocumentId, DatabaseId, ObjectId, EntityId, LayerId, BlockId, LayoutId,
    StyleId, AnnotationId, ViewportId, TransactionId, RequestId, ResourceId);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadError {
    NotImplemented(&'static str), InvalidInput(String), Unsupported(String),
    ResourceMissing(String), CorruptData(String), GpuFailure(String), Invariant(String),
    PermissionDenied, Cancelled, StaleResult,
}
impl fmt::Display for CadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for CadError {}
pub type CadResult<T> = Result<T, CadError>;
pub fn pending<T>(feature: &'static str) -> CadResult<T> { Err(CadError::NotImplemented(feature)) }
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point3 { pub x: f64, pub y: f64, pub z: f64 }
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform3 { pub matrix: [[f64; 4]; 4] }
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds3 { pub min: Point3, pub max: Point3 }
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkPlane { pub origin: Point3, pub u: Point3, pub v: Point3 }
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray3 { pub origin: Point3, pub direction: Point3 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceId { Model, Paper(LayoutId) }
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstancePath(pub Vec<EntityId>);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubElementId { pub source_key: String, pub topology_revision: Revision }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRef {
    pub document: DocumentId, pub entity: EntityId,
    pub instance: InstancePath, pub sub_element: Option<SubElementId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometrySource { Analytic, DirectMesh, ProxyCache, KernelMesh, UserPoints }
#[derive(Debug, Clone, PartialEq)]
pub enum Precision { Analytic, Approximate { error_bound: Option<f64> }, Unknown }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportStatus { NotImplemented, Unsupported, Unverified, Partial, Verified }
#[derive(Debug, Clone)]
pub struct EntityCapability {
    pub type_key: String, pub read: SupportStatus, pub semantic: SupportStatus,
    pub render: SupportStatus, pub pick: SupportStatus, pub measure: SupportStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completeness { Complete, Partial(Vec<String>), Missing(Vec<String>), Unverified }
#[derive(Debug, Clone)]
pub struct Diagnostic { pub object: Option<ObjectId>, pub code: String, pub message: String }
#[derive(Debug, Clone)]
pub struct Registration {
    pub type_key: String, pub version: u32, pub priority: i32,
    pub entity_types: Vec<String>, pub capabilities: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Unit { DrawingUnits, Millimeter, Meter, Inch, Foot }
#[derive(Debug, Clone, PartialEq)]
pub struct UnitContext {
    pub source: Unit, pub display: Unit, pub display_per_source: Option<f64>, pub decimal_places: u8,
}
#[derive(Debug, Clone, Copy)]
pub struct TolerancePolicy {
    pub computation_world: f64, pub topology_world: f64, pub display_pixels: f64,
    pub interaction_logical_pixels: f64, pub formatting_decimals: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentIdentity { Temporary(u128), Sha256([u8; 32]) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStamp {
    pub document: DocumentId, pub generation: u64, pub object_revision: Revision,
    pub dependency_version: u64, pub configuration_version: u64,
}
impl TaskStamp {
    pub fn validate(&self, current: &Self) -> CadResult<()> {
        if self == current { Ok(()) } else { Err(CadError::StaleResult) }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticGeometry {
    Line { start: Point3, end: Point3 },
    Polyline { points: Vec<Point3>, bulges: Vec<f64>, closed: bool },
    Circle { center: Point3, normal: Point3, radius: f64 },
    Arc { center: Point3, normal: Point3, radius: f64, start: f64, sweep: f64 },
    Ellipse { center: Point3, major_axis: Point3, ratio: f64, start: f64, sweep: f64 },
    Spline { degree: u32, knots: Vec<f64>, control_points: Vec<Point3>, weights: Vec<f64> },
    Point(Point3), Mesh(Mesh), Insert { block: BlockId, transform: Transform3 },
    Text { text: String, position: Point3, style: StyleId, height: f64, rotation: f64 },
    Opaque { type_key: String, version: u32, payload: Vec<u8> },
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Point3>, pub triangles: Vec<[u32; 3]>, pub normals: Vec<Point3>,
    pub face_sources: Vec<Option<SubElementId>>,
}
