//! Payload structs for the 2026-11-07 2D text batch
//! (`JUSTIFYTEXT`/`TJUST`/`TSCALE`/`TEXTALIGN`/`TCOUNT`/`TCIRCLE`).
//!
//! Split out of `command_payload.rs` so the enum file stays inside the
//! code-size limit. Each variant's full contract is documented once in
//! `crate::command_id::text_batch`; this module only carries the data.

use cad_domain::{TextAlignH, TextAlignV};

pub use crate::command_id::text_batch::{
    TextAlignAxis, TextAlignEdge, TextAlignSpacing, TextBoundaryShape,
};

/// `JUSTIFYTEXT` / `TJUST`: the requested alignment pair. Both fields are
/// mandatory (the typed grammar requires `<H> <V>`); nothing else about the text
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JustifyTextCommand {
    pub h_align: TextAlignH,
    pub v_align: TextAlignV,
}

/// `TSCALE`: one positive finite glyph-height factor. The write path is the
/// shared `SCALETEXT` `scaled_text` rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextScaleCommand {
    pub factor: f64,
}

/// `TEXTALIGN`: the one edge/centre every selected text is aligned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextAlignCommand {
    pub edge: TextAlignEdge,
}

/// `TEXTALIGN DISTRIBUTE [X|Y]` / the named spacing options: the spacing mode
/// the selected texts are laid out with (only `Distribute` writes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextAlignSpacingCommand {
    pub spacing: TextAlignSpacing,
}

/// `TCOUNT`: the numbering run and its uniform affixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TCountCommand {
    pub start: i64,
    /// Strictly positive; the session parser and the command both refuse
    /// zero/negative (`tcount.invalid_step`).
    pub step: i64,
    /// Text placed before the number (empty when not supplied).
    pub prefix: String,
    /// Text placed after the number (empty when not supplied).
    pub suffix: String,
}

/// `TCIRCLE`: which closed boundary is drawn around each text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TCircleCommand {
    pub shape: TextBoundaryShape,
}
