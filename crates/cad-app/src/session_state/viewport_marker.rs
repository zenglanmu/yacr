//! The session `CVPORT` marker: the current viewport of each paper layout.
//!
//! AutoCAD's `CVPORT` system variable names the viewport that viewport-scoped
//! commands act on (`LAYVPI`'s "all viewports except the current one",
//! `VPSCALE`, `VPCLIP`). This build stores it as **session state only** — one
//! zero-based `Layout::viewports` index per paper layout, keyed by [`LayoutId`]
//! — so setting it is never a database write, a revision bump, a `cad-history`
//! entry, an undo step or a `ChangeSet`. Each layout reports its own stored
//! marker, so switching layouts switches the reported value, and a document
//! replacement (a new session) drops the whole map.
//!
//! Two numbering schemes meet here, deliberately:
//!
//! * the stored marker and every payload index below the command boundary are
//!   **zero-based** viewport indices, the same indexing
//!   `cad_db::Layout::viewports` and `LayoutPayload::FreezeViewportLayersIn`
//!   use;
//! * the `CVPORT` command line is **one-based** (AutoCAD's own numbering): the
//!   first viewport of a layout is `CVPORT 1`. Only the command boundary
//!   converts between the two; nothing below it speaks positions.
//!
//! Nothing here fabricates a default: a layout with no viewport at all has no
//! current viewport and is reported as `cvport.no_viewports`, never as `1`.

use cad_domain::{CadError, CadResult, LayoutId};

use std::collections::BTreeMap;

/// Per-layout `CVPORT` markers: the zero-based current viewport index.
///
/// The documented default of a layout with no stored entry is index `0` (its
/// first viewport) — the same "first viewport" answer every other command gave
/// before the marker existed. The default is only ever applied to a layout that
/// **has** a viewport; the command boundary refuses an empty layout first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewportMarkers {
    current: BTreeMap<LayoutId, usize>,
}

impl ViewportMarkers {
    /// The stored zero-based current viewport index of `layout` (default `0`).
    pub fn current(&self, layout: LayoutId) -> usize {
        self.current.get(&layout).copied().unwrap_or(0)
    }

    /// Store the zero-based current viewport index of `layout`.
    pub fn set_current(&mut self, layout: LayoutId, index: usize) {
        self.current.insert(layout, index);
    }

    /// Forget one layout's marker (`LAYOUT DELETE` / `LAYOUTMERGE`).
    ///
    /// Session state only; the layout ids are never reused, so a stale entry
    /// could otherwise outlive its layout for the rest of the session.
    pub fn forget(&mut self, layout: LayoutId) {
        self.current.remove(&layout);
    }
}

impl crate::SessionState {
    /// The stored zero-based current viewport index of `layout` (default `0`).
    ///
    /// An unvalidated read: the caller knows the layout's viewport count and
    /// resolves it through `target_viewport`, which refuses a stale marker by
    /// name instead of clamping it.
    pub fn viewport_marker(&self, layout: LayoutId) -> usize {
        self.viewport_markers.current(layout)
    }

    /// Store the zero-based current viewport index of `layout` (`CVPORT <n>`).
    ///
    /// Session state only: no database write, revision bump, `cad-history`
    /// entry, undo step or `ChangeSet`. Range validation is the command
    /// boundary's job (it is the only layer that knows the viewport count).
    pub fn set_viewport_marker(&mut self, layout: LayoutId, index: usize) {
        self.viewport_markers.set_current(layout, index);
    }

    /// Forget one layout's marker (`LAYOUT DELETE` / `LAYOUTMERGE`).
    pub fn forget_viewport_marker(&mut self, layout: LayoutId) {
        self.viewport_markers.forget(layout);
    }
}

/// Resolve the zero-based viewport index a viewport-scoped command targets.
///
/// An explicit index always wins (the caller's own authority). Otherwise the
/// session `CVPORT` marker is used. The marker is **validated** against `count`
/// and refused by name (`{prefix}.current_viewport_out_of_range`) when it names
/// a viewport this layout no longer has — a silent clamp to the first viewport
/// would make a stale marker look like a successful retarget.
///
/// Callers check the "no viewport at all" case first with their own code
/// (`{prefix}.no_viewport`), because a layout without viewports has no current
/// viewport to fall back to at all.
pub fn target_viewport(
    layout: LayoutId,
    explicit: Option<usize>,
    marker: usize,
    count: usize,
    prefix: &str,
) -> CadResult<usize> {
    match explicit {
        Some(index) if index < count => Ok(index),
        Some(index) => Err(CadError::InvalidInput(format!(
            "{prefix}.unknown_viewport: layout {} has {count} viewport(s); there is no viewport \
             {index} to target (viewport indices are zero-based)",
            layout.0
        ))),
        None if marker < count => Ok(marker),
        None => Err(CadError::InvalidInput(format!(
            "{prefix}.current_viewport_out_of_range: the session CVPORT marker names viewport {} \
             (zero-based index {marker}), but layout {} has {count} viewport(s); set it again with \
             `CVPORT <n>` (1..={count}) — nothing was changed and nothing was clamped",
            marker + 1,
            layout.0
        ))),
    }
}
