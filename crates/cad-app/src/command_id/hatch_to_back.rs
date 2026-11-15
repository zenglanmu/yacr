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
//! Every `SemanticGeometry::Hatch` entity is moved behind all non-hatch entities
//! **of its own draw-order scope** by rewriting only the stored `draw_order`. A
//! scope is one `SpaceId` value: model space, one paper layout, or one block
//! definition — the sequences every consumer of `DbEntity::draw_order` already
//! groups by, so there is no second ordering mechanism:
//!
//! * `min_order` is the minimum `draw_order` over the **non-hatch entities of
//!   that scope**;
//! * that scope's hatches keep their existing **relative order** (ties broken by
//!   entity id) and receive the contiguous block
//!   `min_order - n ..= min_order - 1`, where `n` is the number of hatches in the
//!   scope;
//! * the arithmetic is checked per scope. If any scope's target range would
//!   underflow `i64` the whole run is refused as `hatchtoback.order_exhausted` —
//!   no wrap, no partial write (every scope is planned before anything is
//!   staged);
//! * a scope with no hatch is reported and left alone (a no-op, never an error);
//!   with no hatches anywhere the command is `hatchtoback.none` and opens no
//!   transaction; if every hatch is already strictly below its scope's
//!   `min_order` (including the only-hatches scope, which is vacuously behind
//!   every non-hatch) it is `hatchtoback.unchanged`, also with no transaction.
//!
//! Geometry, display geometry and every other field are unchanged; only
//! `draw_order` differs. All changed rows are staged with **full entity patches**
//! in one transaction / one undo step, so undo restores the exact previous
//! orders. Success is `hatchtoback.applied`, and its one diagnostic lists per
//! space/block how many hatches moved (or that the scope has none / was already
//! behind).
//!
//! ## Scope
//!
//! Paper-space and block-definition hatches are reordered **within their own
//! scope**, never across spaces: a layout draw order is only meaningful against
//! that layout's rows, and a block definition's members only against each other.
//! The operation is non-associative and uses the stored draw order (there is no
//! separate AutoCAD-style draw-order table).
//!
//! Work-only (Viewer mode refuses it at the command layer).
