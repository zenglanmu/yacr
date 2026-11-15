//! The `REFSET` / `REFCLOSE` payload options, split out of `command_payload.rs`
//! so that file stays inside the 1000-line code gate.
//!
//! Both are session-only inputs of the bounded in-place block edit
//! ([`crate::app_refedit`]); the public paths (`cad_app::RefSetOption`,
//! `cad_app::RefCloseOption`) are unchanged.

/// The direction of one `REFSET ADD|REMOVE` working-set edit.
///
/// Session-only: `REFSET` never writes the drawing, it only records which
/// extracted entities (and which extra host objects) the in-place block edit
/// owns. [`crate::CommandId::RefSet`] with a [`crate::CommandPayload::None`]
/// means `Add` (AutoCAD's bare prompt default); the typed `REFSET REMOVE` line
/// reaches this enum through the argument channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefSetOption {
    /// Add the current selection to the working set.
    Add,
    /// Remove the current selection from the working set.
    Remove,
}

/// The disposition of an active in-place block edit (`REFCLOSE SAVE|DISCARD`).
///
/// `Save` writes the working set back into the block definition; `Discard`
/// deletes the extracted model-space clones and leaves the definition untouched.
/// A bare `REFCLOSE` carries no option and is refused by name
/// (`refclose.needs_option`), never defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefCloseOption {
    /// Write the working set back into the block definition.
    Save,
    /// Delete the clones and leave the block definition unchanged.
    Discard,
}
