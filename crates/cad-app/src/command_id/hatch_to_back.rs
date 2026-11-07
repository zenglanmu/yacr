//! Long-form notes for [`crate::CommandId::HatchToBack`] (`HATCHTOBACK`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the implementation lives in `app_drawing/hatch_utilities.rs`.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::None`], and the current selection is
//! ignored. The command needs an open drawing (the desktop marks it
//! document-scoped); it takes no argument and has no option prompt.
//!
//! ## Ordering rule
//!
//! Every `SemanticGeometry::Hatch` entity in **model space** is moved behind all
//! non-hatch model-space entities by rewriting only the stored `draw_order`:
//!
//! * `min_order` is the minimum `draw_order` over the non-hatch model-space
//!   entities;
//! * the hatches keep their existing **relative order** (ties broken by entity
//!   id) and receive the contiguous block `min_order - n ..= min_order - 1`,
//!   where `n` is the hatch count;
//! * the subtraction is checked. If the target range would underflow `i64` the
//!   command is refused as `hatchtoback.order_exhausted` — no wrap, no partial
//!   write;
//! * with no hatches at all the command is `hatchtoback.none` and opens no
//!   transaction; if every hatch is already strictly below `min_order` (including
//!   the only-hatches case, which is vacuously behind every non-hatch) it is
//!   `hatchtoback.unchanged`, also with no transaction.
//!
//! Geometry, display geometry and every other field are unchanged; only
//! `draw_order` differs. All changed rows are staged with **full entity patches**
//! in one transaction / one undo step, so undo restores the exact previous
//! orders. Success is `hatchtoback.applied`.
//!
//! ## Scope
//!
//! Paper-space and block-definition hatches are **not** reordered: this build has
//! no paper-space draw-order model in scope, and silently touching layout rows
//! would be cross-space ordering this command cannot represent. The operation is
//! non-associative and uses the stored draw order (there is no separate
//! AutoCAD-style draw-order table).
//!
//! Work-only (Viewer mode refuses it at the command layer).
