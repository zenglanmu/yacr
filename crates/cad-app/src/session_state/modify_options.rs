//! Session-only stored options for the interactive `MIRROR` / `OFFSET` captures.
//!
//! AutoCAD keeps the same values as drawing sysvars (`MIRRTEXT`, `OFFSETDIST`,
//! `OFFSETGAPTYPE`, `TRIMMODE`-adjacent erasure choices). This build stores them
//! in the **session** only: no drawing-header write, no revision bump, no undo
//! entry and no persisted preference. They start at the documented legacy
//! defaults (`MIRRTEXT = 0`, no erase, `Through`, source layer), so a capture
//! with no typed option produces exactly the legacy payload and every existing
//! call site is unchanged.
//!
//! The typed command line is the single writer: `MIRROR TEXT 0|1`,
//! `MIRROR ERASE YES|NO`, `OFFSET <distance>`, `OFFSET THROUGH`,
//! `OFFSET ERASE`, `OFFSET LAYER CURRENT|SOURCE` and `OFFSET MULTIPLE` parse in
//! `cad-ui-core` and call these setters. The capture map
//! (`host::draw_intent_map`) reads them back and emits
//! `CommandPayload::MirrorOptions` / `CommandPayload::OffsetOptions` when the
//! stored set differs from the legacy defaults; a default set keeps the legacy
//! `Mirror` / `Offset` payload.
//!
//! `OFFSET MULTIPLE` has no field here: the fixed-step capture takes exactly one
//! side point, so a multiple-offset loop is not expressible interactively. The
//! typed line is a **named refusal** (see `cad-ui-core`), and the loop is only
//! reachable through an `OffsetOptions` payload with two or more side points.

/// The stored MIRROR and OFFSET capture options (session state only).
///
/// The derived [`Default`] is exactly the legacy set: `MIRRTEXT = 0`, no erase,
/// `Through` (the pick defines the distance) and the source layer. A capture
/// with this set produces the legacy `Mirror` / `Offset` payload unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModifyOptions {
    mirrortext: bool,
    mirror_erase: bool,
    offset_distance: Option<f64>,
    offset_erase: bool,
    offset_current_layer: bool,
}

impl ModifyOptions {
    /// `MIRRTEXT`: `false` (default) keeps mirrored text readable.
    pub fn mirrortext(&self) -> bool {
        self.mirrortext
    }

    /// `Erase source objects?` for MIRROR (default `false`).
    pub fn mirror_erase(&self) -> bool {
        self.mirror_erase
    }

    /// The stored OFFSET distance; `None` is AutoCAD's `Through`.
    pub fn offset_distance(&self) -> Option<f64> {
        self.offset_distance
    }

    /// Whether OFFSET erases the source after the insert (default `false`).
    pub fn offset_erase(&self) -> bool {
        self.offset_erase
    }

    /// Whether an OFFSET lands on the current layer (default `false` = source).
    pub fn offset_current_layer(&self) -> bool {
        self.offset_current_layer
    }

    /// Whether the stored set is the legacy default set (no option payload).
    pub fn mirror_is_default(&self) -> bool {
        !self.mirrortext && !self.mirror_erase
    }

    /// Whether the stored set is the legacy default set (no option payload).
    pub fn offset_is_default(&self) -> bool {
        self.offset_distance.is_none() && !self.offset_erase && !self.offset_current_layer
    }

    /// Store the `MIRRTEXT` answer.
    pub fn set_mirrortext(&mut self, value: bool) {
        self.mirrortext = value;
    }

    /// Store the MIRROR `Erase source objects?` answer.
    pub fn set_mirror_erase(&mut self, value: bool) {
        self.mirror_erase = value;
    }

    /// Store a fixed OFFSET distance (`Some`) or `Through` (`None`).
    pub fn set_offset_distance(&mut self, value: Option<f64>) {
        self.offset_distance = value;
    }

    /// Store the OFFSET erase-source answer.
    pub fn set_offset_erase(&mut self, value: bool) {
        self.offset_erase = value;
    }

    /// Store the OFFSET target-layer choice (`true` = current layer).
    pub fn set_offset_current_layer(&mut self, value: bool) {
        self.offset_current_layer = value;
    }
}
