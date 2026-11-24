//! The bounded layer-state history behind `LAYERP` (spec F03, AutoCAD's
//! "previous layer state").
//!
//! A snapshot carries two halves that together are one "layer setting" change:
//!
//! * the session **view overrides** ([`crate::layers::LayerOverrideSet`]) — a
//!   temporary hide/show that never writes the DWG layer table, never bumps the
//!   revision and is never a `cad-history` entry;
//! * the pre-change **layer-table attributes** of every row the command actually
//!   rewrites (`off`/`frozen`/`locked`/colour/linetype/lineweight/… as
//!   [`cad_db::LayerAttributes`] stores them).
//!
//! Only the attribute half is drawing data. `LAYERP` replays it through the same
//! validated `set_layer_attributes` path every other layer write uses and records
//! the before/after [`cad_db::LayerPatch`] rows as **one** undo step; the override
//! half is session state as before.
//!
//! ## What is recorded, and when
//!
//! Exactly one rule: **before** a layer-setting mutation that really changes
//! something, the pre-change state is pushed.
//!
//! Override mutation sites (push with an empty attribute map):
//!
//! * [`crate::CommandId::ToggleLayer`] (both its payload form and the layer
//!   panel's `cad_set_layer_visible`),
//! * [`crate::CommandId::SetLayerVisibilities`] (the atomic batch),
//! * [`crate::CommandId::IsolateLayers`] / [`crate::CommandId::LayOff`] (they
//!   publish through that same batch),
//! * [`crate::CommandId::RestoreLayers`] (`RESTORELAYERS` / `LAYUNISO` / `LAYON`).
//!
//! Layer-table attribute mutation sites (push with the changed rows' **before**
//! attributes):
//!
//! * `LAYFRZ` / `LAYLCK` / `LAYULK` (the shared `write_layer_flags` batch),
//! * `LAYTHW` (drawing-level `frozen` rows; per-viewport VP-freeze is layout
//!   state and is deliberately **not** in this layer-table snapshot),
//! * `SETLAYERPROPERTY` (the layer panel / IPC write) and the typed `-LAYER`
//!   option lines (`ON`/`OFF`/`FREEZE`/`THAW`/`LOCK`/`UNLOCK`/`COLOR`/`LTYPE`/
//!   `LWEIGHT`/`PLOT`),
//! * `LAYERSTATE RESTORE` when it really rewrites a row or the override set.
//!
//! A mutation that changes nothing pushes **nothing**: otherwise every no-op (a
//! layer already hidden asked to hide again, a restore with no overrides, an
//! `LAYISO` whose keep-set already covers the drawing, a `-LAYER` line that
//! writes the stored value) would plant a phantom step that `LAYERP` would burn
//! one press to "restore".
//!
//! [`crate::CommandId::LayerPrevious`] itself only **pops**. It never pushes, so a
//! held-down `LAYERP` walks monotonically back through the recorded states instead
//! of ping-ponging between the last two. A snapshot whose recorded layer row has
//! since been deleted is refused as a whole (`layers.previous.dangling_layer`)
//! before any write, and the snapshot stays on the stack.
//!
//! ## Bound
//!
//! [`LAYER_OVERRIDE_HISTORY_CAP`] = **16** snapshots. The cap is deliberately small
//! and fixed: one snapshot holds only the rows that changed, so the memory is
//! proportional to one command's layer write set, and a session cannot grow an
//! unbounded history by toggling layers. When the history is full the **oldest**
//! snapshot is dropped (a ring buffer): the most recent states — the ones
//! `LAYERP` is for — always survive.
//!
//! There is no persistence: a new document / session rebuild starts with an empty
//! history, and nothing is written to config.

use std::collections::{BTreeMap, VecDeque};

use cad_db::LayerAttributes;
use cad_domain::LayerId;

use crate::layers::LayerOverrideSet;

/// How many previous layer override states `LAYERP` can walk back through.
///
/// See the module docs for why the history is bounded and why the oldest entry is
/// the one dropped.
pub const LAYER_OVERRIDE_HISTORY_CAP: usize = 16;

/// One `LAYERP` snapshot: the session overrides plus the pre-change attributes of
/// every layer row the recorded command rewrote.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerPreviousState {
    /// The session's temporary visibility overrides at snapshot time.
    pub overrides: LayerOverrideSet,
    /// The pre-change attributes of each layer row the command rewrote, keyed by
    /// layer id. Empty for a pure override change; a layer deleted since the
    /// snapshot makes the restore refuse instead of inventing a row.
    pub attributes: BTreeMap<LayerId, LayerAttributes>,
}

impl LayerPreviousState {
    /// A pure override snapshot (the original `LAYERP` behaviour).
    pub fn new(overrides: LayerOverrideSet) -> Self {
        Self {
            overrides,
            attributes: BTreeMap::new(),
        }
    }

    /// A snapshot that also restores `attributes` on the layer table.
    pub fn with_attributes(
        overrides: LayerOverrideSet,
        attributes: BTreeMap<LayerId, LayerAttributes>,
    ) -> Self {
        Self {
            overrides,
            attributes,
        }
    }

    /// How many layer rows this snapshot would restore.
    pub fn attribute_count(&self) -> usize {
        self.attributes.len()
    }
}

/// A bounded, oldest-dropped stack of layer-state snapshots for `LAYERP`.
///
/// The type is a plain value: no database, no history entry, no I/O. It is
/// deliberately the only owner of the snapshot order, so the cap rule exists once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerOverrideHistory {
    snapshots: VecDeque<LayerPreviousState>,
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
    /// The caller pushes only when a mutation really changes something (see the
    /// module docs), so no phantom steps are recorded.
    pub fn push(&mut self, snapshot: LayerPreviousState) {
        if self.snapshots.len() >= LAYER_OVERRIDE_HISTORY_CAP {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    /// The newest snapshot, without consuming it.
    ///
    /// `LAYERP` validates the whole snapshot (including layer rows that may have
    /// been deleted) before it writes anything, so it peeks first and only pops
    /// once the restore has been published.
    pub fn latest(&self) -> Option<&LayerPreviousState> {
        self.snapshots.back()
    }

    /// Remove and return the **newest** snapshot, or `None` when the history is
    /// empty (an `LAYERP` with nothing to restore).
    pub fn pop(&mut self) -> Option<LayerPreviousState> {
        self.snapshots.pop_back()
    }

    /// The retained snapshots, oldest first (read-only; for contracts).
    pub fn snapshots(&self) -> impl Iterator<Item = &LayerPreviousState> + '_ {
        self.snapshots.iter()
    }
}

impl crate::SessionState {
    /// Record the current override set as one `LAYERP` step with no layer-table
    /// attributes (the original pure-override behaviour).
    ///
    /// Called by the command layer **before** applying a real override change; see
    /// the module docs for the push rule. `enabled` is the document's
    /// `LAYERPMODE` ([`cad_db::DrawingDatabase::layer_pmode`]), read by the caller
    /// from the authoritative drawing.
    pub(crate) fn push_layer_override_snapshot(&mut self, enabled: bool) {
        self.push_layer_previous_snapshot(BTreeMap::new(), enabled);
    }

    /// Record the current override set plus the pre-change attributes of the rows
    /// a layer-table command is about to rewrite.
    ///
    /// `attributes` is empty for a pure override change. `LAYERPMODE OFF`
    /// (`enabled == false`, the document header value) suppresses the whole step
    /// (override half included) without changing which mutation is applied.
    pub(crate) fn push_layer_previous_snapshot(
        &mut self,
        attributes: BTreeMap<LayerId, LayerAttributes>,
        enabled: bool,
    ) {
        if enabled {
            self.layer_history.push(LayerPreviousState::with_attributes(
                self.layer_overrides.clone(),
                attributes,
            ));
        }
    }

    /// The number of `LAYERP` steps recorded in this session.
    ///
    /// Session state only; reading it never mutates anything.
    pub fn layer_history_len(&self) -> usize {
        self.layer_history.len()
    }
}
