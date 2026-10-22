//! The modal-popup state machine (ADR-0021 D1/D2), UI-framework-free.
//!
//! A host owns at most **one** modal popup at a time. This module owns the
//! parts every host must agree on and that need no windowing toolkit:
//!
//! - the [`PopupKind`] allowlist (a caller can never choose a window label, URL
//!   or script; an unknown kind is refused before anything is touched);
//! - the [`PopupAck`] wire shape and the host-derived [`TextEditPayload`];
//! - the **modal input gate** ([`crate::shell::ShellState::popup`]): set only
//!   after the window was built, cleared by the window lifecycle;
//! - the open/close/destroyed transitions, including stale-state
//!   reconciliation.
//!
//! The window layer is a seam ([`PopupOps`]): building, focusing and closing the
//! native window, and deriving the `textEdit` payload from the host's live CAD
//! selection, stay in the concrete host. The gate itself is reached through
//! [`PopupGate`], so the state machine never owns the host's lock.
//!
//! The gate is deliberately **not** UI state: it is independent of any UI-side
//! pointer suspension, whose blur path may release at any time.

use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use crate::session::CadPopup;
use crate::shell::ShellState;

/// Label of the single popup window.
pub const POPUP_LABEL: &str = "popup-1";
/// Popup width in logical pixels (`about`).
pub const POPUP_WIDTH: f64 = 520.0;
/// Popup height in logical pixels (`about`).
pub const POPUP_HEIGHT: f64 = 360.0;
/// Text-edit popup width in logical pixels: wide enough for a full annotation
/// line without wrapping every few words.
pub const TEXT_EDIT_WIDTH: f64 = 640.0;
/// Text-edit popup height in logical pixels: the `about` card is far too short
/// for a multi-line MTEXT / MULTILEADER annotation.
pub const TEXT_EDIT_HEIGHT: f64 = 480.0;
/// Options popup width in logical pixels: a 156px tab rail plus a label column
/// and a control column wide enough for a `#rrggbb` value and a unit suffix.
pub const OPTIONS_WIDTH: f64 = 720.0;
/// Options popup height in logical pixels: tall enough for the longest tab
/// (Display, seven rows) without scrolling.
pub const OPTIONS_HEIGHT: f64 = 560.0;

/// The popup kinds the host will build.
///
/// This enum is the allowlist: a string outside it is refused **before** any
/// window or state change, so a caller cannot choose a label, URL or script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// The trivial "About" dialog that proves the pipeline.
    About,
    /// Edit the current single selection's text (ADR-0021 D4). The **host**
    /// derives the payload from the selection; the caller supplies no content.
    TextEdit,
    /// The Options dialog (the settings window). It carries **no payload**: the
    /// UI reads `cad_settings` itself after mount, so nothing is baked into the
    /// boot script and no stale copy can be shown.
    Options,
}

impl PopupKind {
    /// Resolve one wire kind key against the allowlist.
    ///
    /// The refusal names the rejected kind (`popup.unknown_kind: …`) and is
    /// never a silent fallback to a default dialog.
    pub fn from_key(key: &str) -> Result<Self, String> {
        match key {
            "about" => Ok(Self::About),
            "textEdit" => Ok(Self::TextEdit),
            "options" => Ok(Self::Options),
            other => Err(format!("popup.unknown_kind: {other:?}")),
        }
    }

    /// The stable machine key of this kind.
    pub fn key(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::TextEdit => "textEdit",
            Self::Options => "options",
        }
    }

    /// The logical window size of this kind. Every popup is non-resizable; the
    /// text editor needs more room than the `about` card, and the Options dialog
    /// needs a rail plus a full settings column.
    pub fn size(self) -> (f64, f64) {
        match self {
            Self::About => (POPUP_WIDTH, POPUP_HEIGHT),
            Self::TextEdit => (TEXT_EDIT_WIDTH, TEXT_EDIT_HEIGHT),
            Self::Options => (OPTIONS_WIDTH, OPTIONS_HEIGHT),
        }
    }
}

/// The host-derived payload of the `textEdit` kind (ADR-0021 D4).
///
/// Both fields are always present in the boot payload. `text` is the raw stored
/// string with only `\P` shown as `\n` (the one mapping the ADR allows, so the
/// dialog displays natural line breaks); `multiline` is `true` for `AcDbMText`
/// and MULTILEADER text, `false` for `AcDbText`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditPayload {
    pub text: String,
    pub multiline: bool,
}

/// The `cad_open_popup` acknowledgement (frozen wire shape).
///
/// `accepted` is always `true` on `Ok`; `existing` says whether the host focused
/// an already-open popup instead of building a second one. The wire field is
/// `popup_label` (host window label), never a caller-supplied label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PopupAck {
    pub accepted: bool,
    pub kind: String,
    pub popup_label: String,
    pub existing: bool,
}

/// The window effects an open/close request needs (one seam, so the state
/// machine is testable without a display).
pub trait PopupOps {
    /// Build the single popup window for `kind` (and register its `Destroyed`
    /// handler in the production implementation). `payload` is the host-derived
    /// text payload, present exactly for [`PopupKind::TextEdit`].
    fn create(&self, kind: PopupKind, payload: Option<&TextEditPayload>) -> Result<(), String>;
    /// Focus an existing popup window.
    fn focus(&self, label: &str) -> Result<(), String>;
    /// Ask an existing popup window to close. A missing window is not an error.
    fn close(&self, label: &str) -> Result<(), String>;
    /// Whether the window currently exists (stale-state reconciliation).
    fn exists(&self, label: &str) -> bool;
    /// Derive the `textEdit` payload from the **current** CAD selection.
    ///
    /// A caller can never supply content; a selection that is not exactly one
    /// editable text is refused with `popup.no_text_selection: …` **before** any
    /// window exists.
    fn text_edit_payload(&self) -> Result<TextEditPayload, String>;
}

/// The host-owned modal gate, as the state machine sees it.
///
/// Every operation maps onto the host's own `ShellState::popup` slot under
/// whatever lock protects it; the state machine never holds a host lock across a
/// window effect.
pub trait PopupGate {
    /// The popup currently recorded as open, if any.
    fn current(&self) -> Option<CadPopup>;
    /// Overwrite the recorded popup (used after a successful build).
    fn replace(&self, next: Option<CadPopup>);
    /// Atomically clear and return the recorded popup (the `Destroyed` path).
    fn take(&self) -> Option<CadPopup>;
    /// Clear **only** when the recorded popup is still `expected`, so a
    /// concurrent open is never erased by a stale reconciliation.
    fn clear_if(&self, expected: &CadPopup);
}

/// A gate over a bare popup slot (tests and hosts that do not wrap
/// [`ShellState`]).
impl PopupGate for Mutex<Option<CadPopup>> {
    fn current(&self) -> Option<CadPopup> {
        lock(self).clone()
    }

    fn replace(&self, next: Option<CadPopup>) {
        *lock(self) = next;
    }

    fn take(&self) -> Option<CadPopup> {
        lock(self).take()
    }

    fn clear_if(&self, expected: &CadPopup) {
        let mut current = lock(self);
        if current.as_ref() == Some(expected) {
            *current = None;
        }
    }
}

/// A gate over the shell state itself (the host's gate and its consumers share
/// one state object).
impl PopupGate for Mutex<ShellState> {
    fn current(&self) -> Option<CadPopup> {
        lock(self).popup.clone()
    }

    fn replace(&self, next: Option<CadPopup>) {
        lock(self).popup = next;
    }

    fn take(&self) -> Option<CadPopup> {
        lock(self).popup.take()
    }

    fn clear_if(&self, expected: &CadPopup) {
        let mut state = lock(self);
        if state.popup.as_ref() == Some(expected) {
            state.popup = None;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Open the allowlisted popup (or focus the existing single instance).
///
/// Order matters:
/// 1. validate the kind (an unknown kind touches nothing);
/// 2. reconcile stale state (the window may have gone away before its
///    `Destroyed` event was observed);
/// 3. focus an existing popup, or build exactly one and only then set the gate.
pub fn open_popup(
    gate: &dyn PopupGate,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    let kind = PopupKind::from_key(requested)?;
    reconcile_popup_state(gate, ops);

    match gate.current() {
        Some(open) => {
            // Single instance: never build a second window and never re-set the
            // gate; only raise the one that is already there.
            ops.focus(&open.label)?;
            Ok(PopupAck {
                accepted: true,
                kind: open.kind,
                popup_label: open.label,
                existing: true,
            })
        }
        None => {
            // The payload is derived **before** the window exists: a refusal
            // (`popup.no_text_selection`) leaves nothing to undo and creates no
            // window/gate.
            let payload = match kind {
                PopupKind::About | PopupKind::Options => None,
                PopupKind::TextEdit => Some(ops.text_edit_payload()?),
            };
            ops.create(kind, payload.as_ref())?;
            let open = CadPopup {
                kind: kind.key().to_string(),
                label: POPUP_LABEL.to_string(),
                modal: true,
            };
            // Set the gate **once**, and only after the window was built: a
            // failed build must never leave CAD input refused forever.
            gate.replace(Some(open.clone()));
            Ok(PopupAck {
                accepted: true,
                kind: open.kind,
                popup_label: open.label,
                existing: false,
            })
        }
    }
}

/// Close the open popup (idempotent).
///
/// The gate is intentionally **not** cleared here: the authoritative clear is
/// the window's `Destroyed` event, which also covers Esc and the window manager.
/// A window that is already gone is reconciled instead, so the call stays an
/// idempotent success and nothing is left dangling.
pub fn close_popup(gate: &dyn PopupGate, ops: &dyn PopupOps) -> Result<(), String> {
    reconcile_popup_state(gate, ops);
    match gate.current() {
        None => Ok(()),
        Some(open) => ops.close(&open.label),
    }
}

/// Clear the modal input gate after the popup window is destroyed.
///
/// Called from the popup window's `Destroyed` event, so IPC close, Esc from the
/// popup UI and a window-manager close all release input by the same path.
pub fn on_popup_destroyed(gate: &dyn PopupGate) {
    let cleared = gate.take();
    if let Some(open) = cleared {
        eprintln!(
            "[probe] popup destroyed label={} kind={}; modal input gate cleared",
            open.label, open.kind
        );
    }
}

/// Re-validate the recorded popup against a window that truly exists.
///
/// A `Destroyed` event is the normal clear path, but a state left over from an
/// already-gone window must never wedge CAD input: `open`/`close` reconcile
/// first, so the next call rebuilds a fresh window and the gate stays honest.
fn reconcile_popup_state(gate: &dyn PopupGate, ops: &dyn PopupOps) {
    let Some(open) = gate.current() else {
        return;
    };
    if ops.exists(&open.label) {
        return;
    }
    eprintln!(
        "[probe] popup state was stale (window {} is gone); clearing the modal input gate",
        open.label
    );
    // Clear only the same instance: a concurrent open must not be erased.
    gate.clear_if(&open);
}

#[cfg(test)]
mod tests;
