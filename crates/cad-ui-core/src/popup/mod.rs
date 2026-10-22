//! The modal-popup state machine (ADR-0021 D1/D2), UI-framework-free.
//!
//! A host owns at most **one** modal popup at a time. Since WP3 the popup is an
//! in-DOM overlay in the single UI tree, so this module no longer owns any
//! window lifecycle: it owns only the parts every host must agree on and that
//! need no windowing toolkit:
//!
//! - the [`PopupKind`] allowlist (a caller can never choose a label, URL or
//!   script; an unknown kind is refused before anything is touched);
//! - the [`PopupAck`] wire shape, including the additive host-derived
//!   [`PopupPayload`];
//! - the **modal input gate** ([`crate::shell::ShellState::popup`]): set only
//!   after the open request was fully validated, cleared by one of the four
//!   explicit lifecycle triggers ([`PopupReset`] plus [`close_popup`]);
//! - the open/close/reset transitions.
//!
//! The remaining platform seam ([`PopupOps`]) only derives the `textEdit`
//! payload from the host's live CAD selection: rendering, focus and teardown
//! belong to the UI/host.
//!
//! The gate is deliberately **not** UI state: it is independent of any UI-side
//! pointer suspension, whose blur path may release at any time.

use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use crate::session::CadPopup;
use crate::shell::ShellState;

/// Frozen legacy wire label of the single popup.
///
/// The popup is a DOM overlay now, so there is no window to label. The value is
/// still reported as `popup_label`/`CadPopup::label` because the `cad_status`
/// wire shape is frozen; it is never used to address a window.
pub const POPUP_LABEL: &str = "popup-1";

/// The popup kinds the host will open.
///
/// This enum is the allowlist: a string outside it is refused **before** any
/// state change, so a caller cannot choose a label, URL or script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// The trivial "About" dialog that proves the pipeline.
    About,
    /// Edit the current single selection's text (ADR-0021 D4). The **host**
    /// derives the payload from the selection; the caller supplies no content.
    TextEdit,
    /// The Options dialog (the settings dialog). It carries **no payload**: the
    /// UI reads `cad_settings` itself after mount, so no stale copy can be shown.
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
}

/// The one host refusal for an open where no single editable text is selected.
///
/// Technical English only: it is a host/transport caption the UI maps to catalog
/// copy; it is not UI prose.
pub const NO_TEXT_SELECTION: &str =
    "popup.no_text_selection: select exactly one TEXT, MTEXT or MULTILEADER text to edit";

/// The host-derived payload of the `textEdit` kind (ADR-0021 D4).
///
/// Both fields are always present in the acknowledgement payload. `text` is the
/// raw stored string with only `\P` shown as `\n` (the one mapping the ADR
/// allows, so the dialog displays natural line breaks); `multiline` is `true`
/// for `AcDbMText` and MULTILEADER text, `false` for `AcDbText`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditPayload {
    pub text: String,
    pub multiline: bool,
}

impl TextEditPayload {
    /// Assemble the dialog payload from one selected text entity.
    ///
    /// `selected` is the platform host's read of the live CAD selection:
    /// `Some((stored_text, multiline))` for exactly one editable text, `None`
    /// otherwise. A `None` is refused with [`NO_TEXT_SELECTION`] **before** any
    /// modal gate is set, so the host never opens an editor with no content.
    ///
    /// The multiline branch is the only place the one ADR-permitted mapping is
    /// applied: an unescaped `\P` becomes a newline. Single-line `TEXT` has no
    /// such code and passes through byte-identically. This is deliberately
    /// platform-independent, so every desktop host shares it instead of
    /// re-implementing the branch.
    pub fn from_selection(selected: Option<(String, bool)>) -> Result<Self, String> {
        let Some((text, multiline)) = selected else {
            return Err(NO_TEXT_SELECTION.to_string());
        };
        Ok(Self {
            text: if multiline {
                paragraph_breaks_to_newlines(&text)
            } else {
                text
            },
            multiline,
        })
    }
}

/// Show an **unescaped** `\P` as a newline, so the dialog edits natural line
/// breaks.
///
/// Only `\P` is touched. `\\P` (an escaped backslash followed by `P`), `{`, `}`,
/// `\f…;`, `\W…;` and every other code pass through byte-identically — this is
/// not a rich-text parser.
fn paragraph_breaks_to_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('P') => out.push('\n'),
            // Keep the escape pair verbatim: `\\` is a literal backslash, and
            // `\X` is some other code that this dialog must not interpret.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The additive `cad_open_popup` payload, emitted on a fresh open only.
///
/// Serialized with a `kind` tag (`{"kind":"about","version":…,"build":…}`), so a
/// consumer can never mistake one kind's fields for another's. The tag is
/// redundant with the ack's own `kind` on purpose: a payload that disagrees with
/// the requested kind is rejected by the UI instead of rendering guessed content.
///
/// `options` has no variant: the settings dialog reads `cad_settings` itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PopupPayload {
    /// Static host facts: the build's version and profile.
    About { version: String, build: String },
    /// The host-derived text-edit content.
    TextEdit { text: String, multiline: bool },
}

impl PopupPayload {
    /// The About payload of this build (workspace version + build profile).
    fn about() -> Self {
        Self::About {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build: if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
        }
    }
}

/// The `cad_open_popup` acknowledgement (frozen wire shape + additive payload).
///
/// `accepted` is always `true` on `Ok`; `existing` says whether the host found an
/// already-open popup instead of opening one. The wire field is `popup_label`
/// (the frozen legacy label), never a caller-supplied label. `payload` is
/// present only on a fresh open of a kind that carries one; it is skipped
/// entirely when absent, so a consumer that ignores it sees the old shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PopupAck {
    pub accepted: bool,
    pub kind: String,
    pub popup_label: String,
    pub existing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<PopupPayload>,
}

/// Why the host cleared the modal gate without an explicit close.
///
/// Each variant is one of the four authoritative clear triggers; the other is
/// [`close_popup`] (explicit close / Esc / close button).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupReset {
    /// The UI document reloaded or navigated: the overlay no longer exists.
    Reload,
    /// The webview renderer/process failed: the overlay can never render again.
    RendererCrash,
    /// The host process is exiting.
    ProcessExit,
}

impl PopupReset {
    /// The stable machine key used in the one diagnostic log line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Reload => "reload",
            Self::RendererCrash => "renderer_crash",
            Self::ProcessExit => "process_exit",
        }
    }
}

/// The host effects an open request needs (one seam, so the state machine is
/// testable without a display).
pub trait PopupOps {
    /// Derive the `textEdit` payload from the **current** CAD selection.
    ///
    /// A caller can never supply content; a selection that is not exactly one
    /// editable text is refused with `popup.no_text_selection: …` **before** the
    /// modal gate is set.
    fn text_edit_payload(&self) -> Result<TextEditPayload, String>;
}

/// The host-owned modal gate, as the state machine sees it.
///
/// Every operation maps onto the host's own `ShellState::popup` slot under
/// whatever lock protects it; the state machine never holds a host lock across
/// an effect.
pub trait PopupGate {
    /// The popup currently recorded as open, if any.
    fn current(&self) -> Option<CadPopup>;
    /// Overwrite the recorded popup (used after a successful open).
    fn replace(&self, next: Option<CadPopup>);
    /// Atomically clear and return the recorded popup (every clear trigger).
    fn take(&self) -> Option<CadPopup>;
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
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Open the allowlisted popup, or report the one already open.
///
/// Order matters:
/// 1. validate the kind (an unknown kind touches nothing);
/// 2. derive the host payload (a refusal leaves no gate behind);
/// 3. set the gate **once**, then answer with the additive payload.
pub fn open_popup(
    gate: &dyn PopupGate,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    let kind = PopupKind::from_key(requested)?;

    if let Some(open) = gate.current() {
        // Single instance: the caller gets the existing instance and no new
        // payload, so it must not overwrite the content already on screen.
        return Ok(PopupAck {
            accepted: true,
            kind: open.kind,
            popup_label: POPUP_LABEL.to_string(),
            existing: true,
            payload: None,
        });
    }

    // Derive the payload **before** the gate is set: a refusal
    // (`popup.no_text_selection`) leaves nothing to undo.
    let payload = match kind {
        PopupKind::About => Some(PopupPayload::about()),
        PopupKind::TextEdit => {
            let derived = ops.text_edit_payload()?;
            Some(PopupPayload::TextEdit {
                text: derived.text,
                multiline: derived.multiline,
            })
        }
        PopupKind::Options => None,
    };

    gate.replace(Some(open_state(kind)));
    Ok(PopupAck {
        accepted: true,
        kind: kind.key().to_string(),
        popup_label: POPUP_LABEL.to_string(),
        existing: false,
        payload,
    })
}

/// The one place the frozen legacy wire label is written.
///
/// The label addresses no window any more; it is kept because `cad_status`'s
/// block is frozen. Centralising the construction here is what "no dual-write"
/// means: no other module builds a popup state from its own label.
#[allow(deprecated)]
fn open_state(kind: PopupKind) -> CadPopup {
    CadPopup {
        kind: kind.key().to_string(),
        label: POPUP_LABEL.to_string(),
        modal: true,
    }
}

/// Close the open popup and clear the modal gate (idempotent).
///
/// This is trigger ① of the four-trigger lifecycle: an explicit close (the
/// close button or Esc) clears the gate directly, because the overlay and the
/// gate are owned by the same document. Closing when nothing is open is a
/// success, not an error.
pub fn close_popup(gate: &dyn PopupGate) -> Result<(), String> {
    let cleared = gate.take();
    if let Some(open) = cleared {
        eprintln!(
            "[probe] popup close trigger=explicit label={} kind={}; modal input gate cleared",
            POPUP_LABEL, open.kind
        );
    }
    Ok(())
}

/// Clear the modal gate after a non-explicit lifecycle event (triggers ②–④).
///
/// A reset while nothing is open is a no-op that still logs, so a reload, a
/// renderer crash or a process exit is never silently claimed to have cleared a
/// gate that was not set.
pub fn on_popup_reset(gate: &dyn PopupGate, reason: PopupReset) {
    match gate.take() {
        Some(open) => eprintln!(
            "[probe] popup reset trigger={} kind={}; modal input gate cleared",
            reason.key(),
            open.kind
        ),
        None => eprintln!(
            "[probe] popup reset trigger={}; no popup was open",
            reason.key()
        ),
    }
}

#[cfg(test)]
mod tests;
