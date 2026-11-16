//! The 3D body model: topology class, exact geometry when available, and the
//! derived display cache.
//!
//! A body is the domain-level container for imported 3D geometry (ACIS solids,
//! surfaces and wires, and planar `REGION` bodies). The exact B-rep lives in
//! [`crate::brep`]; [`BodyDisplay`] is the tessellation cache built from it.
//! Body geometry never appears here as an invented mesh: when the exact shape
//! is missing or unsupported, [`BodyExact::Unavailable`] says so.

use crate::brep::BrepData;
use crate::{Completeness, Mesh, Point3, Precision};

/// The topological class of a body, mirroring ACIS 1D/2D/3D topology.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyKind {
    /// A 1D body: curves only, no surface. It may display only as `edges`.
    Wire,
    /// A 2D body: a (possibly zero-thickness) surface. Covers both zero-thickness
    /// ACIS surfaces and planar `REGION` bodies; the AutoCAD source class stays in
    /// the entity's `DbObject.type_key`, not here.
    Sheet,
    /// A closed volumetric B-Rep. **Requires** [`BodyExact::Exact`]; `cad-db`
    /// enforces that a solid never stores missing or empty exact geometry.
    Solid,
    /// The topology class is not known (for example a body whose exact geometry
    /// is unavailable). Reported honestly instead of being guessed.
    Unknown,
}

/// The exact geometry of a body, or the reason it is missing.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyExact {
    /// The neutral B-rep that `cad-kernel-adapter` can evaluate and tessellate.
    Exact(Box<BrepData>),
    /// Honest missing geometry: no B-rep exists for this body (unparsed payload,
    /// unsupported kernel operation, or a downgrade after a non-uniform
    /// transform). Never paired with a fabricated mesh.
    Unavailable { reason: String },
}

/// The derived display cache of a body.
///
/// Rebuilt from the exact geometry; commands never edit it in place. It is
/// reported separately from [`BodyExact`] because a body can be drawable without
/// being exact (edges only, or a mesh the kernel produced but cannot keep as a
/// B-rep).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyDisplay {
    /// Cached tessellation. Empty when the body has only edge geometry.
    pub mesh: Mesh,
    /// Boundary/feature polylines in world space. Each polyline has at least two
    /// points; `cad-db` rejects anything shorter.
    pub edges: Vec<Vec<Point3>>,
    /// Linear deflection the mesh was built at, when known.
    pub tolerance: Option<f64>,
    /// How exact the display is.
    pub precision: Precision,
    /// What the display is missing, if anything.
    pub completeness: Completeness,
}

/// A 3D body: its topological class, its exact geometry (or the honest reason it
/// is missing) and the derived display cache.
#[derive(Debug, Clone, PartialEq)]
pub struct Body3d {
    pub kind: BodyKind,
    pub exact: BodyExact,
    pub display: BodyDisplay,
}

impl Body3d {
    /// The exact B-rep, when this body has one.
    pub fn exact_brep(&self) -> Option<&BrepData> {
        match &self.exact {
            BodyExact::Exact(brep) => Some(brep.as_ref()),
            BodyExact::Unavailable { .. } => None,
        }
    }

    /// Whether this body carries exact geometry (see [`Body3d::exact_brep`]).
    pub fn is_exact(&self) -> bool {
        self.exact_brep().is_some()
    }
}
