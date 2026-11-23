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
//! * **Depth planes.** The front/back distances (flags `72`/`73`, groups
//!   `40`/`41`) are read into `BlockClip` as a block-local `z` band. A filter
//!   whose normal is not the block's `+Z`, whose inverse transform has no usable
//!   `z` scale, or whose pair is non-finite/`front > back` cannot be stored
//!   faithfully; it is reported (`import.xclip_depth_normal`/`_scale`/
//!   `_direction`/`_invalid`) and the 2D region alone is stored.
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
    /// The clipping plane normal is not the block's `+Z`, so the distances
    /// measure another axis and are not stored.
    pub const DEPTH_NORMAL: &str = "import.xclip_depth_normal";
    /// The inverse block transform has no usable `z` scale for the distances.
    pub const DEPTH_SCALE: &str = "import.xclip_depth_scale";
    /// The inverse block transform reverses the block's `+Z`.
    pub const DEPTH_DIRECTION: &str = "import.xclip_depth_direction";
    /// The source pair is non-finite or `front > back`, so it names no band.
    pub const DEPTH_INVALID: &str = "import.xclip_depth_invalid";
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
    let (front_clip, back_clip) = read_depth(filter, &mut gaps);
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
            front_clip,
            back_clip,
        }),
        gaps,
    }
}

/// The clip depths a source filter carries, in block-local `+Z`.
///
/// `SPATIAL_FILTER` groups `40`/`41` hold the front/back distances along the
/// boundary plane's normal (`210`), each enabled by its own flag (`72`/`73`);
/// acadrust turns a set flag plus its distance into an `Option`, so an unset flag
/// and an absent distance are the same `None` and a distance cannot be read
/// without its flag. [`cad_domain::BlockClip`] stores the pair as the block-local
/// **z band** `[front, back]`, so the direction and the unit are checked here
/// rather than assumed:
///
/// * the normal must be the block's `+Z` (`(0, 0, +n)`). A normal in, or against,
///   the block plane means the two distances measure another axis; they are
///   reported (`import.xclip_depth_normal`) and not stored;
/// * the distances are WCS lengths, so they are converted to block-local `z`
///   through the filter's own inverse block transform (its `z` scale). A
///   singular or mirrored `z` scale is reported (`import.xclip_depth_scale` /
///   `import.xclip_depth_direction`), never guessed;
/// * the pair must be finite and ordered `front <= back` — the rule
///   `cad_domain::BlockClip::with_depth` enforces. A source pair that fails it is
///   reported (`import.xclip_depth_invalid`) and the 2D boundary is stored alone.
///
/// A one-sided filter is honoured: the missing side is `None` (unbounded), not
/// zero, which is exactly what a single set flag means.
fn read_depth(
    filter: &SpatialFilter,
    gaps: &mut Vec<(&'static str, String)>,
) -> (Option<f64>, Option<f64>) {
    if filter.front_clip.is_none() && filter.back_clip.is_none() {
        return (None, None);
    }
    let normal = filter.normal;
    let length = normal.x.hypot(normal.y).hypot(normal.z);
    let along_plane = length.is_finite() && length > 1e-12;
    if !along_plane
        || normal.x.abs() > 1e-9 * length
        || normal.y.abs() > 1e-9 * length
        || normal.z < 0.0
    {
        gaps.push((
            xclip_reason::DEPTH_NORMAL,
            format!(
                "the filter's clipping plane normal is ({}, {}, {}), not the block's +Z; its \
                 front/back distances measure another axis, so they are not stored and only the \
                 2D boundary is kept",
                normal.x, normal.y, normal.z
            ),
        ));
        return (None, None);
    }
    // WCS distance → block-local +Z distance. The boundary points sit on the WCS
    // z = 0 plane, so the difference of the two mapped points is the scale.
    let origin_z = filter
        .inverse_block_transform
        .transform_point(Vector3::new(0.0, 0.0, 0.0))
        .z;
    let unit_z = filter
        .inverse_block_transform
        .transform_point(Vector3::new(0.0, 0.0, 1.0))
        .z;
    let scale = unit_z - origin_z;
    if !scale.is_finite() || scale.abs() <= 1e-12 {
        gaps.push((
            xclip_reason::DEPTH_SCALE,
            format!(
                "the filter's inverse block transform maps +Z to a {} scale, so its front/back \
                 distances have no block-local length; the 2D boundary alone is stored",
                if scale.is_finite() {
                    "zero".to_string()
                } else {
                    "non-finite".to_string()
                }
            ),
        ));
        return (None, None);
    }
    if scale < 0.0 {
        gaps.push((
            xclip_reason::DEPTH_DIRECTION,
            "the filter's inverse block transform reverses the block's +Z, so the front/back \
             distances run the other way; they are not stored and only the 2D boundary is kept"
                .into(),
        ));
        return (None, None);
    }
    let local = |distance: f64| distance * scale;
    let front = filter.front_clip.map(local);
    let back = filter.back_clip.map(local);
    if let Err(error) = cad_domain::validate_clip_depth(front, back) {
        gaps.push((
            xclip_reason::DEPTH_INVALID,
            format!(
                "the filter's front/back distances are not a storable depth band ({error}); the 2D \
                 boundary is stored without them"
            ),
        ));
        return (None, None);
    }
    (front, back)
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
