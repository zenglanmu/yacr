//! The persisted restore constructor for [`super::MultiLeaderGeometry`]
//! (ADR-0037 D8).
//!
//! Every other write path derives the display: [`super::MultiLeaderGeometry::new`]
//! rejects structural defects and then builds it, `from_import` builds it while
//! degrading honestly, and `transformed`/`with_text` rebuild it after their
//! edit.
//!
//! Phase 1 persists the embedded `display` **verbatim** (ADR-0037 D8), so a
//! restore must not re-derive it: the derivation would replace stored display
//! data on the first save (breaking the L1 fixed point), and `new` would reject
//! a legacy row whose stored values this build's structural rules no longer
//! accept. This constructor takes every field as stored — no validation, no
//! rebuild.
//!
//! The fields stay private: this module is a child of `multileader`, so it can
//! reach them without widening them to `pub`.

use super::{
    MLeaderContent, MLeaderLine, MLeaderReserved, MLeaderRoot, MLeaderStyleValues, MLeaderText,
    MultiLeaderGeometry, PlaneFrame,
};
use crate::SemanticGeometry;

impl MultiLeaderGeometry {
    /// Rebuild a multileader exactly as it was persisted.
    ///
    /// No structural validation, no display rebuild: the caller (the ymodel
    /// payload reader) has already accepted the stored object.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        style: MLeaderStyleValues,
        style_name: String,
        override_mask: u32,
        content: MLeaderContent,
        text: MLeaderText,
        roots: Vec<MLeaderRoot>,
        lines: Vec<MLeaderLine>,
        reserved: MLeaderReserved,
        plane: PlaneFrame,
        display: Vec<SemanticGeometry>,
    ) -> Self {
        MultiLeaderGeometry {
            style,
            style_name,
            override_mask,
            content,
            text,
            roots,
            lines,
            reserved,
            plane,
            display,
        }
    }
}
