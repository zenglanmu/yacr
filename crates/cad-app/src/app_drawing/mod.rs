//! Drawing creation and edit commands for [`Application`] (spec §4.7, F-EDIT).
//!
//! These are the real, transactional implementations behind
//! [`CommandId::CreateLine`], [`CommandId::CreateCircle`], [`CommandId::CreateArc`],
//! [`CommandId::MoveEntities`], [`CommandId::TrimEntity`], [`CommandId::ExtendEntity`],
//! [`CommandId::EraseEntities`] and [`CommandId::CopyEntities`]. The contract is
//! pinned in `docs/drawing-edit.md` §2/§3; this module implements exactly that:
//!
//! * every command is Work-only (the command layer refuses Viewer mode);
//! * every command commits through the drawing database's validated write path
//!   and records exactly one undo step with full before/after entity patches;
//! * a failure changes no state (the database transaction is all-or-nothing)
//!   and records no history;
//! * TRIM/EXTEND cover the documented LINE-vs-LINE/LWPOLYLINE subset only: an
//!   unsupported *geometry* (non-LINE target, curved boundary) is an explicit
//!   `Unsupported`, while a plan-level refusal (degenerate or no intersection) is
//!   an explicit `InvalidInput`, and neither touches the drawing.
//!
//! Nothing here builds UI strings or dispatches tools; that is workstream C.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::app_drawing::…` (and therefore the crate-internal surface) is
//! unchanged.

use super::*;
use cad_db::{transform_geometry, DbEntity};
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};

mod array;
mod chamfer_fillet;
mod clipboard;
mod drawing_create;
mod drawing_edit;
mod drawing_transform;
mod helpers;
mod trim_extend;

pub(crate) use helpers::{apply_drawing_patches, class_name, next_transaction_id};
// The explicit ARRAY entity cap (ADR 0016 decision D); re-exported so the
// command docs (`crate::app_drawing::MAX_ARRAY_ENTITIES`) resolve. It is a doc
// anchor only — the value is used inside the `array` module.
#[allow(unused_imports)]
pub use array::MAX_ARRAY_ENTITIES;

use helpers::{
    collect_boundary_segments, distance, geometry_kind, is_finite_point, offset_circle,
    offset_line, plan_extend, plan_trim, TrimPlan,
};
