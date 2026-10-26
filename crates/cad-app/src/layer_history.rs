//! The bounded layer-override history behind `LAYERP` (spec F03, AutoCAD's
//! "previous layer state").
//!
//! Layer overrides are **session view state** ([`crate::layers::LayerOverrideSet`]):
//! they never write the DWG layer table, never bump the database revision and are
//! never a `cad-history` entry. `LAYERP` therefore cannot use the undo stack; it
//! needs its own small stack of override snapshots, which lives here.
//!
//! ## What is recorded, and when
//!
//! Exactly one rule: **before** an override mutation that really changes the
//! override set, the pre-change set is pushed. The mutation sites are the ones
//! that touch [`crate::SessionState::layer_overrides`]:
//!
//! * [`crate::CommandId::ToggleLayer`] (both its payload form and the layer
//!   panel's `cad_set_layer_visible`),
//! * [`crate::CommandId::SetLayerVisibilities`] (the atomic batch),
//! * [`crate::CommandId::IsolateLayers`] / [`crate::CommandId::LayOff`] (they
//!   publish through that same batch),
//! * [`crate::CommandId::RestoreLayers`] (`RESTORELAYERS` / `LAYUNISO` / `LAYON`).
//!
//! A mutation that changes nothing pushes **nothing**: otherwise every no-op (a
//! layer already hidden asked to hide again, a restore with no overrides, an
//! `LAYISO` whose keep-set already covers the drawing) would plant a phantom step
//! that `LAYERP` would burn one press to "restore".
//!
//! [`crate::CommandId::LayerPrevious`] itself only **pops**. It never pushes, so a
//! held-down `LAYERP` walks monotonically back through the recorded states instead
//! of ping-ponging between the last two.
//!
//! ## Bound
//!
//! [`LAYER_OVERRIDE_HISTORY_CAP`] = **16** snapshots. The cap is deliberately small
//! and fixed: one snapshot is a `BTreeMap` of `(LayerId, bool)` overrides, so the
//! memory is proportional to the number of overridden layers, and a session cannot
//! grow an unbounded history by toggling layers. When the history is full the
//! **oldest** snapshot is dropped (a ring buffer): the most recent states — the
//! ones `LAYERP` is for — always survive.
//!
//! There is no persistence: a new document / session rebuild starts with an empty
//! history, and nothing is written to config.

use std::collections::VecDeque;

use crate::layers::LayerOverrideSet;

/// How many previous layer override states `LAYERP` can walk back through.
///
/// See the module docs for why the history is bounded and why the oldest entry is
/// the one dropped.
pub const LAYER_OVERRIDE_HISTORY_CAP: usize = 16;

/// A bounded, oldest-dropped stack of layer-override snapshots for `LAYERP`.
///
/// The type is a plain value: no database, no history entry, no I/O. It is
/// deliberately the only owner of the snapshot order, so the cap rule exists once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerOverrideHistory {
    snapshots: VecDeque<LayerOverrideSet>,
}

impl LayerOverrideHistory {
    /// An empty history (no previous state to restore).
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum number of snapshots retained.
    pub fn cap(&self) -> usize {
        LAYER_OVERRIDE_HISTORY_CAP
    }

    /// How many snapshots are currently retained.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether there is no snapshot to restore.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Record one pre-change snapshot, dropping the oldest entry when full.
    ///
    /// The caller pushes only when a mutation really changes the override set (see
    /// the module docs), so no phantom steps are recorded.
    pub fn push(&mut self, snapshot: LayerOverrideSet) {
        if self.snapshots.len() >= LAYER_OVERRIDE_HISTORY_CAP {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    /// Remove and return the **newest** snapshot, or `None` when the history is
    /// empty (an `LAYERP` with nothing to restore).
    pub fn pop(&mut self) -> Option<LayerOverrideSet> {
        self.snapshots.pop_back()
    }

    /// The retained snapshots, oldest first (read-only; for contracts).
    pub fn snapshots(&self) -> impl Iterator<Item = &LayerOverrideSet> + '_ {
        self.snapshots.iter()
    }
}

impl crate::SessionState {
    /// Record the current override set as one `LAYERP` step.
    ///
    /// Called by the command layer **before** applying a real override change; see
    /// the module docs for the push rule.
    pub(crate) fn push_layer_override_snapshot(&mut self) {
        self.layer_history.push(self.layer_overrides.clone());
    }

    /// The number of `LAYERP` steps recorded in this session.
    ///
    /// Session state only; reading it never mutates anything.
    pub fn layer_history_len(&self) -> usize {
        self.layer_history.len()
    }
}
