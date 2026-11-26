//! Typed mesh operations and the pre-flight validation every implementation
//! runs before any kernel work.

use cad_domain::{CadResult, Point3, TaskStamp};

use crate::codes;

/// A plain triangle mesh: positions plus 3-byte-index triangles.
///
/// This is deliberately not [`cad_domain::Mesh`]: the seam carries only the
/// geometry a kernel can consume, with no normals, UVs, colours, per-face
/// sources or body display metadata. Callers convert at the application
/// boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    /// Vertex positions in model coordinates.
    pub vertices: Vec<Point3>,
    /// Triangle vertex indices, three per triangle, CCW from outside.
    pub triangles: Vec<[u32; 3]>,
}

impl MeshData {
    /// An empty mesh (no vertices, no triangles).
    pub fn new() -> Self {
        MeshData {
            vertices: Vec::new(),
            triangles: Vec::new(),
        }
    }

    /// Build from owned buffers without validation. Call [`Self::validate`] (or
    /// [`MeshRequest::validate`]) before handing it to a kernel.
    pub fn from_parts(vertices: Vec<Point3>, triangles: Vec<[u32; 3]>) -> Self {
        MeshData {
            vertices,
            triangles,
        }
    }

    /// No vertices or no triangles.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.triangles.is_empty()
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Validate the invariants a kernel may rely on:
    ///
    /// * at least one vertex and one triangle;
    /// * every position is finite;
    /// * every triangle index is in range;
    /// * a triangle never repeats a vertex index.
    ///
    /// Degenerate (zero-area) triangles are **not** rejected here: real SketchUp
    /// / scan meshes contain them, and dropping or repairing them is a kernel or
    /// edit-layer decision, not a reason to refuse the whole mesh.
    pub fn validate(&self) -> CadResult<()> {
        if self.is_empty() {
            return Err(cad_domain::CadError::InvalidInput(format!(
                "{}: a mesh needs at least one vertex and one triangle",
                codes::MESH_EMPTY
            )));
        }
        for (index, vertex) in self.vertices.iter().enumerate() {
            if !vertex.x.is_finite() || !vertex.y.is_finite() || !vertex.z.is_finite() {
                return Err(cad_domain::CadError::InvalidInput(format!(
                    "{}: vertex {index} is not finite",
                    codes::MESH_INVALID
                )));
            }
        }
        let count = self.vertices.len();
        for (index, triangle) in self.triangles.iter().enumerate() {
            for corner in triangle {
                if *corner as usize >= count {
                    return Err(cad_domain::CadError::InvalidInput(format!(
                        "{}: triangle {index} references vertex {corner}, but the mesh has \
                         {count} vertices",
                        codes::MESH_INVALID
                    )));
                }
            }
            if triangle[0] == triangle[1]
                || triangle[1] == triangle[2]
                || triangle[0] == triangle[2]
            {
                return Err(cad_domain::CadError::InvalidInput(format!(
                    "{}: triangle {index} repeats a vertex index",
                    codes::MESH_INVALID
                )));
            }
        }
        Ok(())
    }
}

impl Default for MeshData {
    fn default() -> Self {
        Self::new()
    }
}

/// Boolean operation on two triangle meshes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshBooleanOp {
    /// `A ∪ B`.
    Union,
    /// `A − B`.
    Subtract,
    /// `A ∩ B`.
    Intersect,
}

impl MeshBooleanOp {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshBooleanOp::Union => "union",
            MeshBooleanOp::Subtract => "subtract",
            MeshBooleanOp::Intersect => "intersect",
        }
    }
}

/// Which winding-number rule a rebuild repair uses.
///
/// Mirrors the kernel's two interpretations of an inconsistently wound closed
/// surface; it is not a tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshWindingRule {
    /// Keep `w >= 1`: an inside-out body is not material and disappears.
    Positive,
    /// Keep `w != 0`: an inside-out body is read as solid and rewound.
    Nonzero,
}

impl MeshWindingRule {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshWindingRule::Positive => "positive",
            MeshWindingRule::Nonzero => "nonzero",
        }
    }
}

/// What a repair actually asks the kernel to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshRepairMode {
    /// Flip inside-out shells; keep triangle count and properties.
    Orientation,
    /// Rebuild a closed solid from the winding numbers; may retriangulate.
    Rebuild,
}

impl MeshRepairMode {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshRepairMode::Orientation => "orientation",
            MeshRepairMode::Rebuild => "rebuild",
        }
    }
}

/// The operation family, used for capability probes and refusal reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshOperationFamily {
    Boolean,
    Repair,
}

impl MeshOperationFamily {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshOperationFamily::Boolean => "boolean",
            MeshOperationFamily::Repair => "repair",
        }
    }
}

/// One mesh operation. Operands are borrowed; the kernel returns a new mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeshOperation<'a> {
    /// Boolean union/subtract/intersect of two closed meshes.
    Boolean {
        op: MeshBooleanOp,
        left: &'a MeshData,
        right: &'a MeshData,
    },
    /// Repair a closed mesh.
    Repair {
        mesh: &'a MeshData,
        mode: MeshRepairMode,
        rule: MeshWindingRule,
    },
}

impl MeshOperation<'_> {
    /// The operation family.
    pub fn family(&self) -> MeshOperationFamily {
        match self {
            MeshOperation::Boolean { .. } => MeshOperationFamily::Boolean,
            MeshOperation::Repair { .. } => MeshOperationFamily::Repair,
        }
    }
}

/// One request across the seam.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshRequest<'a> {
    /// Document/generation identity used by the host for stale-result checks.
    pub stamp: TaskStamp,
    /// The operation to run.
    pub operation: MeshOperation<'a>,
}

impl MeshRequest<'_> {
    /// Validate every operand before any kernel work. Implementations must call
    /// this (or the equivalent) and map the error to `InvalidInput`.
    pub fn validate(&self) -> CadResult<()> {
        match &self.operation {
            MeshOperation::Boolean { left, right, .. } => {
                left.validate()?;
                right.validate()
            }
            MeshOperation::Repair { mesh, .. } => mesh.validate(),
        }
    }
}
