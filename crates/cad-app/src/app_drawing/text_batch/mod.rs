//! The 2026-11-07 2D text batch:
//! `JUSTIFYTEXT`/`TJUST`, `TSCALE`, `TEXTALIGN`, `TCOUNT`, `TCIRCLE`,
//! `TEXTMASK` and `TEXTUNMASK`.
//!
//! Every command reads the **current session selection** (the declared
//! divergence from AutoCAD's interactive `Select objects` prompt, the same one
//! `TORIENT`/`TCASE`/`TXT2MTXT`/`SCALETEXT`/`TEXTTOFRONT` carry). Only directly
//! addressable **model-space** `SemanticGeometry::Text` entities (`AcDbText` and
//! `AcDbMText` share the variant) are text targets.
//!
//! ## One estimate, one write discipline
//!
//! `cad-app` has no glyph metric, so [`estimate_text_bounds`] is the single
//! declared estimate: width `height × char_count`, height `height`, anchored by
//! the stored alignment, axis-aligned in world XY and ignoring rotation. Every
//! command that needs extents uses it; none claims glyph accuracy.
//!
//! Every refusal happens **before** `begin_drawing_transaction`, so a refused
//! command writes nothing. A successful write is one transaction / one undo step
//! and the committed rows are read back for the undo patch. A change-free result
//! is the command's `*.unchanged` diagnostic with no transaction (never an empty
//! undo entry).
//!
//! Full contracts: [`crate::command_id::text_batch`].

use super::*;

// The estimate and the transaction/selection helpers (split out for the
// code-size gate).
mod shared;
// JUSTIFYTEXT/TJUST and TSCALE.
mod justify_scale;
// TEXTALIGN and TCOUNT.
mod align_count;
// TCIRCLE, TEXTMASK and TEXTUNMASK.
mod geometry;

pub(crate) use shared::estimate_text_bounds;
