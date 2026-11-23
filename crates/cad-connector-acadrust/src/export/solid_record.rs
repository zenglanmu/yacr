//! The filled-face (`SOLID` / `TRACE`) reverse map.
//!
//! Kept out of [`super::entity`] only to keep that file inside the repository's
//! size limit; the mapping is part of the same leaf mapper and is reported
//! through the same [`super::report`] entries.
//!
//! Both records share one 3/4-corner geometry and the same group layout; the
//! **class** is the difference (`SOLID` fills, `TRACE` is a filled outline with
//! its own plot/colour rules). acadrust 0.6.8 models both with
//! `entities::Solid` plus an `is_trace` flag, and `write_solid` emits the
//! `SOLID` or `TRACE` record type from that flag (`src/io/dxf/writer/
//! section_writer.rs`), so neither class has to be faked as the other.

use cad_domain::{Mesh, Point3};

/// Type key of a DXF `SOLID`: a created filled 3/4-corner face.
pub(super) const SOLID_TYPE_KEY: &str = "AcDbSolid";

/// Type key of a DXF `TRACE`: the same geometry under a different class.
pub(super) const TRACE_TYPE_KEY: &str = "AcDbTrace";

/// Which filled-face record a stored type key maps to.
///
/// `None` means the key is not a filled face at all. The distinction is kept
/// because relabelling a `TRACE` as a `SOLID` (or the reverse) changes how the
/// source drawing plots, so the wrong record is never better than a declared
/// polyface mesh.
pub(super) fn face_class(type_key: &str) -> Option<FaceClass> {
    match type_key {
        SOLID_TYPE_KEY => Some(FaceClass::Solid),
        TRACE_TYPE_KEY => Some(FaceClass::Trace),
        _ => None,
    }
}

/// The DXF class of a filled-face record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FaceClass {
    Solid,
    Trace,
}

impl FaceClass {
    /// The DXF record/subclass name (`AcDbTrace` subclasses both).
    pub(super) fn record_name(self) -> &'static str {
        match self {
            FaceClass::Solid => "SOLID",
            FaceClass::Trace => "TRACE",
        }
    }

    pub(super) fn is_trace(self) -> bool {
        matches!(self, FaceClass::Trace)
    }
}

/// A filled-face record: the class to write and its four **stored** corners.
pub(super) struct FaceRecord {
    pub class: FaceClass,
    pub corners: [Point3; 4],
}

/// The four DXF corners (stored order, groups 10/11/12/13) for a domain mesh
/// whose vertices are in boundary order, or the reason the mesh cannot be
/// written as a `SOLID`/`TRACE` record.
///
/// DXF stores the corners as first/second/third/fourth, but the drawn boundary
/// is first → second → fourth → third: acadrust keeps that same convention
/// (`Solid::boundary_corners`) and its writer emits groups 10/11/12/13 straight
/// from the four stored corners. The domain mesh stores the boundary cycle in
/// order — both the `SOLID` create path and the importer's `quad_mesh` keep the
/// points as given with triangles `[0,1,2]`/`[0,2,3]` — so the reverse map swaps
/// the last two corners: group 12 is boundary[3] and group 13 is boundary[2].
/// Getting this backwards would emit a bow-tie.
///
/// A filled-face record encodes corners only: it has no triangle list, no
/// per-vertex colours and no plane of its own (it is planar `+Z` OCS here). A
/// mesh that relies on any of those keeps the polyface path and is reported.
pub(super) fn face_record(mesh: &Mesh, class: FaceClass) -> Result<FaceRecord, String> {
    let name = class.record_name();
    let vertices = &mesh.vertices;
    if !(3..=4).contains(&vertices.len()) {
        return Err(format!(
            "a {name} record has 3 or 4 corners, this mesh has {}",
            vertices.len()
        ));
    }
    if !mesh.colors.is_empty() {
        return Err(format!(
            "a {name} record has no per-vertex colours, this mesh has {}",
            mesh.colors.len()
        ));
    }
    let z0 = vertices[0].z;
    if vertices.iter().any(|v| (v.z - z0).abs() > 1e-9) {
        return Err(format!(
            "a {name} record is a planar face in the +Z OCS and this mesh's corners are not \
             coplanar"
        ));
    }
    // A triangular record repeats its last corner in group 13. The importer keeps
    // that as a fourth vertex equal to the third (`quad_mesh`), so both spellings
    // of a triangle are the same shape here.
    let triangle = vertices.len() == 3 || vertices[2] == vertices[3];
    // The record's `[first, second, fourth, third]` cycle *is* the fan
    // `[0,1,2]`/`[0,2,3]`; any other triangulation describes a different filled
    // polygon, so it must not be flattened into four corners.
    let mut actual: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let mut sorted = *triangle;
            sorted.sort_unstable();
            sorted
        })
        .collect();
    actual.sort_unstable();
    let mut implied: Vec<[u32; 3]> = if triangle {
        vec![[0, 1, 2]]
    } else {
        vec![[0, 1, 2], [0, 2, 3]]
    };
    implied.sort_unstable();
    if actual != implied {
        return Err(format!(
            "a {name} record draws the boundary fan {implied:?}, this mesh has the triangles \
             {actual:?}"
        ));
    }
    let corners = if triangle {
        [vertices[0], vertices[1], vertices[2], vertices[2]]
    } else {
        [vertices[0], vertices[1], vertices[3], vertices[2]]
    };
    Ok(FaceRecord { class, corners })
}
