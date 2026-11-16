//! `XCLIP`: read a block reference's clip boundary from the source document.
//!
//! # What the source carries (acadrust 0.6.8, unmodified)
//!
//! An XCLIP is not an INSERT group code. It is an OBJECTS-section object
//! `SPATIAL_FILTER` (`AcDbFilter` / `AcDbSpatialFilter`) reached from the
//! INSERT's extension dictionary:
//!
//! ```text
//! INSERT  --{ACAD_XDICTIONARY 360-->  DICTIONARY      (owner = the INSERT)
//!                                       entry "ACAD_FILTER" 350/360 -->
//!                                     DICTIONARY      (owner = the first)
//!                                       entry "SPATIAL"     350/360 -->
//!                                     SPATIAL_FILTER
//! ```
//!
//! `objects::SpatialFilter` models every field of that object: the boundary
//! points (code 70 count + `10/20` pairs), the boundary plane normal (`210`),
//! the boundary's local origin (`11`), the display-enabled flag (`71`), the
//! front/back clip flags and distances (`72`/`40`, `73`/`41`), and the two
//! 4×3 transforms (`40` ×12 each, read/written by the library's
//! `read_spatial_filter` / `write_spatial_filter`).
//!
//! # Coordinate convention
//!
//! The boundary points are **WCS** points: the first transform is the inverse
//! block transform (WCS → block definition space), which is how ACAD stores an
//! XCLIP of an INSERT at (5, 5) — `[1,0,0,-5, 0,1,0,-5, 0,0,1,0]` — and how the
//! library's own `spatial_filter_from_native_dxf_keeps_its_boundary_area` test
//! reads it. [`cad_domain::BlockClip`] wants the ring in the block definition's
//! local space, so every point is mapped through that transform.
//!
//! # Declared boundaries
//!
//! * **Clip mode.** `AcDbSpatialFilter` as modelled (and as written by ACAD —
//!   groups 70/10/20/210/11/71/72/73/40/41 only) has no inside/inverted field,
//!   so every imported clip is stored as inside-mode and the model gap is
//!   reported once per import (`import.xclip_mode_unmodelled`). An inverted clip
//!   cannot be distinguished from an inside clip in this build.
//! * **Depth planes.** `BlockClip` has no front/back plane, so a filter that
//!   sets one is reported (`import.xclip_depth_planes`) and the stored clip is
//!   the 2D region only.
//! * **Off-plane boundaries.** A boundary whose block-space points do not share
//!   one z is projected onto the block XY plane and reported
//!   (`import.xclip_plane_projected`); the region is still 2D.

use super::*;

use acadrust::objects::{ObjectType, SpatialFilter};
use acadrust::types::{Handle, Vector3};

/// Stable importer codes for a clip this build cannot store faithfully.
pub(crate) mod xclip_reason {
    /// The `SPATIAL` entry resolved to an object that is not a `SPATIAL_FILTER`.
    pub const UNSUPPORTED_FILTER: &str = "import.xclip_filter_unsupported";
    /// A boundary point maps to a non-finite block-space point.
    pub const NON_FINITE: &str = "import.xclip_non_finite";
    /// Fewer than two boundary points: there is no region to store.
    pub const ARITY: &str = "import.xclip_boundary_arity";
    /// The boundary is not parallel to the block XY plane; its ring was projected.
    pub const PLANE: &str = "import.xclip_plane_projected";
    /// The filter sets a front/back clipping plane, which `BlockClip` cannot hold.
    pub const DEPTH: &str = "import.xclip_depth_planes";
    /// The inside/inverted clip mode is not a field of the source model.
    pub const MODE: &str = "import.xclip_mode_unmodelled";
}

/// The clip a source INSERT carries, plus every gap found while reading it.
pub(crate) struct ReadInsertClip {
    /// The boundary in block-definition local space, when one is usable.
    pub(crate) clip: Option<BlockClip>,
    /// One `(code, message)` per declared loss (see [`xclip_reason`]).
    pub(crate) gaps: Vec<(&'static str, String)>,
}

/// Read one INSERT's `XCLIP` boundary.
///
/// `None` for `clip` means the reference really is unclipped: no extension
/// dictionary, no `ACAD_FILTER` entry, no `SPATIAL` entry, or an empty boundary.
/// A boundary that exists but is unusable is refused by name in `gaps` instead
/// of being stored as a fabricated region.
pub(crate) fn read_insert_clip(
    acad: &acadrust::CadDocument,
    insert: &acadrust::entities::Insert,
) -> ReadInsertClip {
    let mut gaps = Vec::new();
    let filter = match spatial_filter(acad, insert.common.handle) {
        Ok(Some(filter)) => filter,
        Ok(None) => {
            return ReadInsertClip { clip: None, gaps };
        }
        Err(class_name) => {
            gaps.push((
                xclip_reason::UNSUPPORTED_FILTER,
                format!(
                    "the insert's ACAD_FILTER/SPATIAL entry is a {class_name}, not a \
                     SPATIAL_FILTER; its clip region is unknown"
                ),
            ));
            return ReadInsertClip { clip: None, gaps };
        }
    };

    // No points at all is the "filter object exists, boundary never set" state,
    // which really is no clip.
    if filter.boundary_points.is_empty() {
        return ReadInsertClip { clip: None, gaps };
    }
    if filter.boundary_points.len() < 2 {
        gaps.push((
            xclip_reason::ARITY,
            format!(
                "the clip boundary has {} point(s); a rectangle needs two opposite corners \
                 and a polygon at least three, so no region is stored",
                filter.boundary_points.len()
            ),
        ));
        return ReadInsertClip { clip: None, gaps };
    }

    let mut vertices = Vec::with_capacity(filter.boundary_points.len());
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    for point in &filter.boundary_points {
        let mapped = filter
            .inverse_block_transform
            .transform_point(Vector3::new(point.x, point.y, 0.0));
        if !mapped.x.is_finite() || !mapped.y.is_finite() || !mapped.z.is_finite() {
            gaps.push((
                xclip_reason::NON_FINITE,
                "a clip boundary point maps to a non-finite block-space point; the whole \
                 boundary is refused rather than stored partially"
                    .into(),
            ));
            return ReadInsertClip { clip: None, gaps };
        }
        min_z = min_z.min(mapped.z);
        max_z = max_z.max(mapped.z);
        vertices.push([mapped.x, mapped.y]);
    }
    let extent = vertices
        .iter()
        .fold(0.0f64, |acc, p| acc.max(p[0].abs()).max(p[1].abs()))
        .max(1.0);
    if max_z - min_z > 1e-9 * extent {
        gaps.push((
            xclip_reason::PLANE,
            format!(
                "the clip boundary is not parallel to the block's XY plane (z spans \
                 {min_z} to {max_z}); the stored ring is its XY projection"
            ),
        ));
    }
    if filter.front_clip.is_some() || filter.back_clip.is_some() {
        gaps.push((
            xclip_reason::DEPTH,
            "the spatial filter sets a front/back clipping plane; this build stores the 2D \
             boundary only, so content outside that depth range stays visible"
                .into(),
        ));
    }
    gaps.push((
        xclip_reason::MODE,
        "the source's clip mode (inside/inverted) is not a field of the \
         AcDbSpatialFilter model (acadrust 0.6.8 exposes no clip-mode flag), so this clip is \
         read as an inside clip; an inverted clip cannot be distinguished"
            .into(),
    ));

    ReadInsertClip {
        clip: Some(BlockClip {
            vertices,
            // No clip-mode field exists in the source model; see `MODE` above.
            inside: true,
            visible: filter.display_enabled,
        }),
        gaps,
    }
}

/// Walk `INSERT → ACAD_FILTER → SPATIAL` and return the filter object.
///
/// `Err(class_name)` is the DXF class of whatever the `SPATIAL` entry pointed at
/// when it is not a `SPATIAL_FILTER`, so the caller can name it.
fn spatial_filter(
    acad: &acadrust::CadDocument,
    insert: Handle,
) -> Result<Option<&SpatialFilter>, &'static str> {
    let Some(xdictionary) = acad.extension_dictionary_handle(insert) else {
        return Ok(None);
    };
    let Some(filter_dict) = dictionary_entry(acad, xdictionary, "ACAD_FILTER") else {
        return Ok(None);
    };
    let Some(spatial) = dictionary_entry(acad, filter_dict, "SPATIAL") else {
        return Ok(None);
    };
    match acad.objects.get(&spatial) {
        Some(ObjectType::SpatialFilter(filter)) => Ok(Some(filter)),
        Some(other) => Err(class_name(other)),
        None => Ok(None),
    }
}

/// A named entry of a DICTIONARY, matched case-insensitively.
fn dictionary_entry(acad: &acadrust::CadDocument, dictionary: Handle, key: &str) -> Option<Handle> {
    let Some(ObjectType::Dictionary(dictionary)) = acad.objects.get(&dictionary) else {
        return None;
    };
    dictionary
        .entries
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, handle)| *handle)
}

/// The DXF class name of a non-filter object, for the refusal message.
fn class_name(object: &ObjectType) -> &'static str {
    match object {
        ObjectType::Dictionary(_) => "DICTIONARY",
        ObjectType::XRecord(_) => "XRECORD",
        ObjectType::RasterVariables(_) => "RASTERVARIABLES",
        ObjectType::GeoData(_) => "GEODATA",
        _ => "non-SPATIAL_FILTER object",
    }
}

impl<'a> ImporterBuilder<'a> {
    /// The clip an entity carries, in the database's storage shape.
    ///
    /// Only an `INSERT` owns an XCLIP (the same restriction the `XCLIP` command
    /// enforces); every other entity has no clip.
    pub(crate) fn entity_clip(
        &self,
        entity: &EntityType,
    ) -> (Option<BlockClip>, Vec<(&'static str, String)>) {
        match entity {
            EntityType::Insert(insert) => {
                let read = read_insert_clip(self.acad, insert);
                (read.clip, read.gaps)
            }
            _ => (None, Vec::new()),
        }
    }
}
