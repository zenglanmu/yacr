//! Bounded named layer-state storage (`LAYERSTATE`) and the `LAYERPMODE` gate.
//!
//! A named layer state is a **session snapshot**: for every layer row it records
//! that row's [`LayerAttributes`], plus the session's active layer id and its
//! temporary visibility overrides. It is deliberately not a DWG round-trip
//! object: this build has no named layer-state table in `cad-db`, so nothing here
//! is written to the drawing and nothing survives a session rebuild.
//!
//! ## Bounds
//!
//! [`LAYER_STATE_CAP`] = **32** snapshots. When the store is full the oldest
//! snapshot is dropped. A session therefore cannot grow an unbounded store by
//! saving states in a loop.
//!
//! ## Names
//!
//! Names are stored trimmed and resolved **exactly first, then by a unique ASCII
//! case-insensitive match**, the same rule the layer table's own name resolution
//! uses. Saving an existing name (case-insensitively) replaces that snapshot and
//! moves it to the newest position, so a name always addresses one state.
//!
//! ## `LAYERPMODE`
//!
//! [`LayerStates::snapshot_enabled`] is the session flag behind `LAYERPMODE`.
//! `ON` (the default) preserves the existing behaviour: every real layer-override
//! mutation records one `LAYERP` step. `OFF` suppresses those snapshots without
//! changing which overrides are applied. The flag gates
//! [`crate::SessionState::push_layer_override_snapshot`], the single push site.
//!
//! ## Restore writes record one undo step
//!
//! `RESTORE` writes the layer **table** through
//! [`cad_db::DrawingDatabase::set_layer_attributes`] (one revision bump per
//! changed row) and records every changed before/after row as one layer-patch
//! undo step. The restored active layer and override set are session state and
//! are not part of history.

use std::collections::{BTreeMap, VecDeque};

use cad_db::LayerAttributes;
use cad_domain::LayerId;

use crate::layers::LayerOverrideSet;

/// How many named layer states a session retains (`LAYERSTATE SAVE`).
pub const LAYER_STATE_CAP: usize = 32;

/// One saved named layer state.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedLayerState {
    /// Stored name, trimmed (never empty: the command line parser refuses that).
    pub name: String,
    /// The attributes of every layer row at save time, keyed by layer id.
    pub attributes: BTreeMap<LayerId, LayerAttributes>,
    /// The session's active layer at save time.
    pub active_layer: LayerId,
    /// The session's temporary visibility overrides at save time.
    pub overrides: LayerOverrideSet,
}

/// The bounded named-layer-state store plus the `LAYERPMODE` gate.
///
/// A plain value type: no database, no history, no I/O, no persistence.
#[derive(Debug, Clone)]
pub struct LayerStates {
    snapshots: VecDeque<NamedLayerState>,
    snapshot_enabled: bool,
}

impl Default for LayerStates {
    fn default() -> Self {
        Self {
            snapshots: VecDeque::new(),
            snapshot_enabled: true,
        }
    }
}

impl LayerStates {
    /// An empty store with `LAYERPMODE ON` (the preserved default behaviour).
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum number of snapshots retained.
    pub fn cap(&self) -> usize {
        LAYER_STATE_CAP
    }

    /// How many snapshots are currently retained.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether no snapshot is retained.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Whether layer-override mutations record `LAYERP` snapshots (`LAYERPMODE`).
    pub fn snapshot_enabled(&self) -> bool {
        self.snapshot_enabled
    }

    /// Set the `LAYERPMODE` gate. Session state only; no drawing write.
    pub fn set_snapshot_enabled(&mut self, enabled: bool) {
        self.snapshot_enabled = enabled;
    }

    /// The retained snapshots in save order, oldest first (for reports/tests).
    pub fn snapshots(&self) -> impl Iterator<Item = &NamedLayerState> + '_ {
        self.snapshots.iter()
    }

    /// Resolve a name: exact match first, then a unique case-insensitive one.
    ///
    /// `None` covers "unknown" and, defensively, "ambiguous" — the store never
    /// creates case-only duplicates through [`Self::save`], so a real caller only
    /// ever sees the unknown case.
    pub fn resolve(&self, name: &str) -> Option<&NamedLayerState> {
        let name = name.trim();
        if let Some(found) = self.snapshots.iter().find(|snapshot| snapshot.name == name) {
            return Some(found);
        }
        let mut matches = self
            .snapshots
            .iter()
            .filter(|snapshot| snapshot.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Save (or replace, case-insensitively) one named state and return whether
    /// an existing snapshot was replaced.
    ///
    /// A replacement is removed from its old position and appended as the newest
    /// snapshot, so `LIST` reflects save order. When the store is at
    /// [`LAYER_STATE_CAP`] the oldest remaining snapshot is dropped.
    pub fn save(
        &mut self,
        name: &str,
        attributes: BTreeMap<LayerId, LayerAttributes>,
        active_layer: LayerId,
        overrides: LayerOverrideSet,
    ) -> bool {
        let name = name.trim();
        if let Some(index) = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.name.eq_ignore_ascii_case(name))
        {
            self.snapshots.remove(index);
            self.snapshots.push_back(NamedLayerState {
                name: name.to_string(),
                attributes,
                active_layer,
                overrides,
            });
            return true;
        }
        if self.snapshots.len() >= LAYER_STATE_CAP {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(NamedLayerState {
            name: name.to_string(),
            attributes,
            active_layer,
            overrides,
        });
        false
    }

    /// Remove one named state, returning whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.name.eq_ignore_ascii_case(name.trim()))
        else {
            return false;
        };
        self.snapshots.remove(index);
        true
    }
}
