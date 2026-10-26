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
//! * TRIM/EXTEND cover the documented LINE/ARC/CIRCLE-target subset: an
//!   unsupported *geometry* (SPLINE/INSERT/Opaque target or boundary) is an
//!   explicit `Unsupported`, while a plan-level refusal (degenerate, no
//!   intersection, already touching) is an explicit `InvalidInput`, and neither
//!   touches the drawing.
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

mod align;
mod array;
mod chamfer_fillet;
mod clipboard;
mod dimension;
mod dimension_angular;
mod dimension_chain;
mod dimension_ordinate;
mod drawing_create;
mod drawing_edit;
mod drawing_transform;
mod explode;
mod grip;
mod hatch;
mod hatch_boundary;
mod hatch_parameters;
mod hatch_stitch;
mod helpers;
mod join;
mod layer_edit;
mod match_prop;
mod mleader;
mod mtext_create;
mod properties;
mod stretch;
mod text_edit;
// The single line-break / stored-type-key authority MTEXT creation and TEXTEDIT
// share; `pub(crate)` because the TEXT capture channel refuses a newline too.
pub(crate) mod text_policy;
mod trim_boundary;
mod trim_curves;
mod trim_extend;

pub(crate) use helpers::{apply_drawing_patches, class_name, next_transaction_id};
// The single base-dimension resolution the DIMBASELINE/DIMCONTINUE capture check
// and the command both use, so a wrong pick is refused with the same reason at
// capture time and at apply time (ADR 0017 decision C, P5f).
pub(crate) use dimension_chain::ensure_chain_base;
pub(crate) use text_edit::resolve_text_edit_target;
// The explicit ARRAY entity cap (ADR 0016 decision D); re-exported so the
// command docs (`crate::app_drawing::MAX_ARRAY_ENTITIES`) resolve. It is a doc
// anchor only — the value is used inside the `array` module.
#[allow(unused_imports)]
pub use array::MAX_ARRAY_ENTITIES;

use helpers::{
    distance, geometry_kind, is_finite_point, is_multiline_text_source, offset_circle, offset_line,
    resolve_created_text_style,
};
