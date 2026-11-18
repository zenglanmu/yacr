//! The persisted restore constructor for [`super::HatchGeometry`]
//! (ADR-0037 D8).
//!
//! Every other write path derives the display: [`super::HatchGeometry::new`]
//! validates the boundary and rebuilds it, `from_import` degrades honestly and
//! rebuilds it, and `transformed`/`with_style_and_fill`/`with_pattern_origin`
//! rebuild it after their edit.
//!
//! Phase 1 persists the embedded `display` **verbatim** (ADR-0037 D8), so a
//! restore must not run those rebuilds: `new` would reject a legacy boundary
//! this build cannot re-validate, and either constructor would replace stored
//! display data with a fresh derivation on the first save, breaking the L1
//! fixed point. This constructor takes every field as stored — no validation,
//! no rebuild.
//!
//! The fields stay private: this module is a child of `hatch_entity`, so it can
//! reach them without widening them to `pub`.

use super::{HatchFill, HatchGeometry, HatchLoop, HatchPatternType, HatchPlane, HatchStyle};
use crate::SemanticGeometry;

impl HatchGeometry {
    /// Rebuild a hatch exactly as it was persisted.
    ///
    /// No validation, no display rebuild: the caller (the ymodel payload
    /// reader) has already accepted the stored object.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        loops: Vec<HatchLoop>,
        plane: HatchPlane,
        style: HatchStyle,
        fill: HatchFill,
        seeds: Vec<[f64; 2]>,
        pixel_size: Option<f64>,
        pattern_type: HatchPatternType,
        pattern_origin: [f64; 2],
        display: Vec<SemanticGeometry>,
    ) -> Self {
        HatchGeometry {
            loops,
            plane,
            style,
            fill,
            seeds,
            pixel_size,
            pattern_type,
            pattern_origin,
            display,
        }
    }
}
