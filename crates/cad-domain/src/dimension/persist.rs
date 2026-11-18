//! The persisted restore constructor for [`super::DimensionGeometry`]
//! (ADR-0037 D8).
//!
//! A dimension's `display` and `measurement` are **derived** on every other
//! write path: [`super::DimensionGeometry::synthesized`] rebuilds the display
//! from `definition + style` and recomputes the measurement, and
//! [`super::DimensionGeometry::from_picture`] recomputes the measurement even
//! though its display comes from a source picture.
//!
//! Phase 1 persists both fields **verbatim** (ADR-0037 D8: the embedded
//! `display` stays authoritative until a replacement cache can reproduce it
//! byte for byte). Restoring through either constructor would therefore
//! silently replace stored display data on the first save and break the
//! `ymodel → format → ymodel` fixed point. It would also reject a legacy row
//! whose stored floats are non-finite, which the database is explicitly allowed
//! to keep (`RestoreStored`). So this constructor takes every field as stored:
//! it re-synthesizes nothing and validates nothing.
//!
//! The fields stay private: this module is a child of `dimension`, so it can
//! reach them without widening them to `pub`.

use super::{DimensionDefinition, DimensionGeometry, DimensionStyleValues, DisplaySource};
use crate::{SemanticGeometry, TextAlignH};

impl DimensionGeometry {
    /// Rebuild a dimension exactly as it was persisted.
    ///
    /// No synthesis, no measurement recomputation, no validation: the caller
    /// (the ymodel payload reader) has already accepted the stored object.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        definition: DimensionDefinition,
        style_name: String,
        style: DimensionStyleValues,
        measurement: f64,
        text_override: Option<String>,
        display: Vec<SemanticGeometry>,
        text_h_align: TextAlignH,
        source: DisplaySource,
    ) -> Self {
        DimensionGeometry {
            definition,
            style_name,
            style,
            measurement,
            text_override,
            display,
            text_h_align,
            source,
        }
    }

    /// The stored display provenance, so the payload writer can persist it.
    pub(crate) fn display_source(&self) -> DisplaySource {
        self.source
    }
}
