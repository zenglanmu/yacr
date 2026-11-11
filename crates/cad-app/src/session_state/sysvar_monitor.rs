//! The session-only `SYSVARMONITOR` watch set.
//!
//! The monitor stores one **modelled** system-variable name (`SETVAR`'s closed
//! set) with the value seen when it was last added or queried. `LIST` compares
//! the live value with that snapshot and reports a change; it is a **polled**
//! report, never a background notification, because this build has no timer or
//! observer channel that could push a change.
//!
//! Session state only: nothing here is written to the drawing, persisted, or
//! shared between documents. The ordered map keeps `LIST` deterministic.

use std::collections::BTreeMap;

/// A bounded, session-only set of watched system-variable names.
#[derive(Debug, Clone, Default)]
pub struct SysVarMonitor {
    watched: BTreeMap<String, String>,
}

impl SysVarMonitor {
    /// Start watching `name` with `value` as the initial snapshot.
    ///
    /// Returns `false` when the name was already watched; the existing snapshot
    /// is deliberately kept so a repeat `ADD` cannot hide a pending change.
    pub fn add(&mut self, name: String, value: String) -> bool {
        if self.watched.contains_key(&name) {
            return false;
        }
        self.watched.insert(name, value);
        true
    }

    /// Stop watching `name`; `false` when it was not watched.
    pub fn remove(&mut self, name: &str) -> bool {
        self.watched.remove(name).is_some()
    }

    /// Whether `name` is watched.
    pub fn contains(&self, name: &str) -> bool {
        self.watched.contains_key(name)
    }

    /// The recorded snapshot of `name`, when watched.
    pub fn snapshot(&self, name: &str) -> Option<&str> {
        self.watched.get(name).map(String::as_str)
    }

    /// Replace the snapshot of a watched name with its live value.
    pub fn record(&mut self, name: &str, value: String) {
        if let Some(snapshot) = self.watched.get_mut(name) {
            *snapshot = value;
        }
    }

    /// The watched names in deterministic (sorted) order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.watched.keys().map(String::as_str)
    }

    /// The number of watched names.
    pub fn len(&self) -> usize {
        self.watched.len()
    }

    /// Whether the watch set is empty.
    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }
}
