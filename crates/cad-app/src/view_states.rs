//! The bounded, session-only named-view store behind `VIEW` / `VIEWGO`.
//!
//! A named view is **session state**: the viewport camera, the 2D/3D
//! observation mode, the plan work plane and the active drawing space. It is
//! deliberately not a DWG object: this build has no named-view table in
//! `cad-db`, so nothing here is written to the drawing and nothing survives a
//! session rebuild.
//!
//! ## Bounds and names
//!
//! [`VIEW_STATE_CAP`] = **32** views. When the store is full the oldest view is
//! dropped, exactly like [`crate::layer_states`]. Names are stored trimmed and
//! resolved **exactly first, then by a unique ASCII case-insensitive match**.
//! Saving an existing name (case-insensitively) replaces it and moves it to the
//! newest position, so a name always addresses one view.
//!
//! ## Not a second view authority
//!
//! `SAVE` captures [`ViewSnapshot`], the same value `ZOOM PREVIOUS` stores;
//! `RESTORE` applies it to the viewport and the session's active space. There is
//! no parallel camera model.

use std::collections::VecDeque;

use cad_domain::SpaceId;

use crate::zoom_history::ViewSnapshot;
use crate::Viewport;

/// How many named views a session retains.
pub const VIEW_STATE_CAP: usize = 32;

/// One saved named view.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedView {
    /// Stored name, trimmed (the command-line parser refuses an empty one).
    pub name: String,
    /// The viewport camera, mode and work plane at save time.
    pub view: ViewSnapshot,
    /// The session's active drawing space at save time.
    pub space: SpaceId,
}

/// A plain value store: no database, no history, no I/O, no persistence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewStates {
    views: VecDeque<NamedView>,
}

impl ViewStates {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum number of views retained.
    pub fn cap(&self) -> usize {
        VIEW_STATE_CAP
    }

    /// How many views are currently retained.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    /// Whether no view is retained.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The retained views in save order, oldest first (for reports/tests).
    pub fn views(&self) -> impl Iterator<Item = &NamedView> + '_ {
        self.views.iter()
    }

    /// Resolve a name: exact match first, then a unique case-insensitive one.
    pub fn resolve(&self, name: &str) -> Option<&NamedView> {
        let name = name.trim();
        if let Some(found) = self.views.iter().find(|view| view.name == name) {
            return Some(found);
        }
        let mut matches = self
            .views
            .iter()
            .filter(|view| view.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Save (or replace, case-insensitively) one view and return whether an
    /// existing view was replaced.
    ///
    /// A replacement moves to the newest position; at [`VIEW_STATE_CAP`] the
    /// oldest remaining view is dropped.
    pub fn save(&mut self, name: &str, view: ViewSnapshot, space: SpaceId) -> bool {
        let name = name.trim();
        if let Some(index) = self
            .views
            .iter()
            .position(|candidate| candidate.name.eq_ignore_ascii_case(name))
        {
            self.views.remove(index);
            self.views.push_back(NamedView {
                name: name.to_string(),
                view,
                space,
            });
            return true;
        }
        if self.views.len() >= VIEW_STATE_CAP {
            self.views.pop_front();
        }
        self.views.push_back(NamedView {
            name: name.to_string(),
            view,
            space,
        });
        false
    }

    /// Remove one view, returning whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let name = name.trim();
        let Some(index) = self
            .views
            .iter()
            .position(|view| view.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.views.remove(index);
        true
    }
}

impl Viewport {
    /// Capture this viewport's current view exactly as `VIEW SAVE` records it.
    pub fn view_snapshot(&self) -> ViewSnapshot {
        ViewSnapshot::capture(self)
    }
}
