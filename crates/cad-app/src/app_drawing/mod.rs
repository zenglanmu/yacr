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
//! * TRIM/EXTEND cover the documented LINE/ARC/CIRCLE-target subset plus SPLINE
//!   targets for TRIM only: an unsupported *geometry* (INSERT/Opaque target, a
//!   SPLINE EXTEND target, or an unsupported boundary) is an explicit
//!   `Unsupported`, while a plan-level refusal (degenerate, no intersection,
//!   already touching) is an explicit `InvalidInput`, and neither touches the
//!   drawing.
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
pub(crate) mod annot_batch;
mod arc_length;
mod array;
// The dynamic PSet property write path (`bim.pset.{set}.{prop}`): the property
// command dispatches these keys here, never to the display-property grammar.
mod bim_properties;
mod block_define;
mod boundary;
mod break_curve;
mod chamfer_fillet;
mod circle_options;
mod clipboard;
mod construction_line;
mod dimension;
mod dimension_angular;
mod dimension_chain;
mod dimension_edit;
mod dimension_ordinate;
mod dimension_override;
mod dimension_qdim;
mod dimension_space;
mod dimension_style;
mod divide;
mod drawing_create;
// ADR-0043 D8/D9: the `PARAM` structural command and the `param.<name>` value
// write dispatched from `SETENTITYPROPERTY`.
mod param_command;
mod param_properties;
// The 2026-11-07 draw variants (WIPEOUT, POLYGON CIRCUMSCRIBED): split out of
// `drawing_create` so both files stay inside the code-size limit.
mod draw_variants;
// The 2026-11-07 draw/modify batch: DONUT/SOLID/HELIX/MLINE/MLSTYLE create
// entities and one session store; TREX/EXTRIM/MPEDIT/DRAWORDER edit them.
mod draw2_batch;
mod draw2_edit;
mod drawing_edit;
mod drawing_transform;
// The shared create funnel's input bundle and session-COLOUR resolution.
mod create_inputs;
// `ELEV`: the session elevation the shared create funnel offsets new entities by
// (one rule, one transform authority; copies are deliberately excluded).
mod elevation;
// SPLINE EXTEND: one declared tangent-continuation rule (TRIM of a SPLINE lives
// in `trim_spline`; this is the continuation half, not a second trim model).
mod extend_spline;
// The 2026-11-07 extras batch write paths (SKETCH/TRACE/MOCORO/LINETYPE).
mod explode;
mod extras_batch;
mod find_text;
mod grip;
mod hatch;
mod hatch_boundary;
mod hatch_edit;
mod hatch_edit_boundaries;
mod hatch_edit_draw_order;
mod hatch_edit_labels;
mod hatch_edit_pattern_file;
mod hatch_edit_pattern_name;
mod hatch_internal_point;
mod hatch_parameters;
// The AutoCAD `.pat` pattern library (HATCH predefined/custom definitions).
mod hatch_pattern;
mod hatch_stitch;
mod hatch_utilities;
mod helpers;
mod image_edit;
// The `FS` contact passes: the `FSMODE=OFF` direct pass and the `FSMODE=ON`
// tangent chain (`inquiry` keeps the command shell).
mod fs_chain;
mod inquiry;
mod insert_block;
mod join;
mod layer_commands;
mod layer_edit;
mod style_tables;
mod xclip_edit;
// `CLIP`/`CLIPIT`: the generic boundary commands over the IMAGECLIP/XCLIP models.
mod clip_edit;
// The typed `-LAYER <option> …` write line (one option per line), split out so
// both layer modules stay inside the code-size limit.
mod dash_layer_write;
mod layer_extended;
mod layer_record;
mod layout_record;
mod lengthen;
mod match_prop;
mod mirror_curves;
mod mleader;
// The 2026-11-12 MULTILEADER edits: `MLEADERALIGN`, `MLEADEREDIT` and
// `MLEADERCOLLECT` rebuild the stored definition through `MultiLeaderGeometry`.
mod mleader_collect;
mod mleader_edit;
// `MLEADEREDIT`'s two per-line overrides (path type `170`, arrowhead size `40`).
mod mleader_line_override;
// `MLEDIT` / `-MLEDIT`: the two-LINE subset of multiline editing.
mod mledit;
// `TOLERANCE`: one feature-control frame as separate border/divider/text rows.
mod tolerance;
// `MSTRETCH`: the one STRETCH planner over several crossing windows.
mod modify_extra;
// OVERKILL (Delete Duplicate Objects): the tolerance/ignore/option comparison
// engine, split out of `modify_extra` so both files stay inside the budget.
mod overkill;
pub use overkill::OverkillSettings;
mod mstretch;
mod mtext_create;
mod offset_polyline;
mod offset_polyline_bulge;
// OFFSET with an explicit fixed distance (the bare `Through` planner plus the
// requested distance), split out of `trim_extend` where it had drifted.
mod offset_fixed;
// The `OFFSETGAPTYPE` fillet/chamfer fill geometry, split out so both offset
// arms stay inside the code-size limit.
mod offset_gap;
mod paste_block;
mod pedit;
mod point_style;
mod polyline_curve;
mod polyline_fillet;
mod properties;
mod revision_cloud;
mod scale_text;
mod stretch;
// The STRETCH containment (rectangle + crossing polygon/lasso) and per-vertex
// planner, split out so `stretch.rs` stays inside the code-size limit.
mod stretch_vertices;
mod stretch_window;
// The 2026-11-07 2D text batch (JUSTIFYTEXT/TJUST/TSCALE/TEXTALIGN/TCOUNT/
// TCIRCLE/TEXTMASK/TEXTUNMASK); one estimate and one write discipline.
pub(crate) mod text_batch;
mod text_edit;
// TABLE / -TABLE creation and TABLEDIT cell-text editing (2026-11-10).
mod table_edit;
// The table commands' shared report/write-diagnostic labels, split out so
// `table_ops` stays inside the code-size limit.
mod table_format;
mod table_ops;
pub mod table_payload;
// The 2026-11-12 SHAPE family: `SHAPE` glyph creation and the `SHP2BLK`
// shape→outline-block conversion.
mod shape;
// ADR-0036 phase C1: BOX/WEDGE/PYRAMID/CYLINDER/SPHERE/CONE/TORUS create one
// real exact `Body3d` through the injected `SolidModeler`; phase C2 adds the
// background `UNION`/`SUBTRACT`/`INTERSECT` job (`tasks::solid`).
pub(crate) mod solids;
// The typed-construction half of those seven commands: payload → `PrimitiveSpec`
// with every arity/finiteness/option refusal (split out of `solids`).
pub(crate) mod solid_spec;
// The 2026-11-12 block/copy writes: `MINSERT` (block array) and `ADDSELECTED`.
mod addselected;
mod minsert;
// TEXTTOFRONT (2026-11-07): rewrite the selected text/leader/dimension draw order.
mod text_front;
// DRAWORDER (split out of `draw2_edit`): the per-`SpaceId` reorder planner.
mod draw_order_scope;
// PURGE (split out of `app_utility`): the block/style/linetype table half.
mod purge_named;
// The three selection-based text commands (TORIENT / TCASE / TXT2MTXT) and
// their one pure case transformer.
mod text_commands;
// TXT2MTXT's settings-driven combine/split path, split out so `text_commands`
// stays inside the file-size gate.
mod txt2mtext;
// The single line-break / stored-type-key authority MTEXT creation and TEXTEDIT
// share; `pub(crate)` because the TEXT capture channel refuses a newline too.
pub(crate) mod text_policy;
// The session-side resolution of `textRotation` / `mtextAttachment`; shared by
// TEXT creation (which reads the rotation directly, its payload has no rotation
// field) and the interactive MTEXT capture (which fills its payload from both).
mod text_params;
pub(crate) use text_params::{mtext_attachment_align, text_rotation_radians};
mod trim_boundary;
mod trim_curves;
// The cut-resolution helpers, split out of `trim_curves` so both files stay
// inside the code-size limit.
mod trim_cuts;
mod trim_extend;
// TRIM of a HATCH target: the boundary loop through the shared polyline planner,
// re-closed along the straight cutting edge (split out so both files stay inside
// the code-size limit).
mod trim_hatch;
mod trim_polyline;
mod trim_spline;

pub(crate) use helpers::{apply_drawing_patches, class_name, next_transaction_id};
pub(crate) use layout_record::layout_row;
pub(crate) use style_tables::resolve_unique;

/// The 2026-11 text/draw utility payload, if this command carries one.
///
/// The four utilities travel through `CommandPayload::Annotation` to keep the
/// top-level payload enum inside its file-size budget; every handler reads only
/// its own [`crate::command_payload::UtilityPayload`] arm.
pub(crate) fn utility_payload(
    command: &Command,
) -> Option<&crate::command_payload::UtilityPayload> {
    match &command.payload {
        CommandPayload::Annotation(batch) => match batch.as_ref() {
            crate::AnnotationBatchPayload::Utility(utility) => Some(utility.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

// The Wave 4 extended edits reuse the existing planner/write surfaces rather
// than a second copy: these re-exports are crate-wide only.
pub(crate) use draw2_edit::CurveChange;
pub(crate) use properties::plan_entity;
pub(crate) use trim_boundary::{collect_boundaries, BoundarySet};
pub(crate) use trim_curves::{
    cut_tolerance, plan_extend, plan_trim, PlanRefusal, TargetCurve, TrimPlan,
};
// The POLYLINE-mode FILLET/CHAMFER planner, its spec and the two-line parity
// reference: exposed to the crate's tests only (`tests::drawing::polyline_fillet`
// drives the pure planner and compares it against the two-line `plan_fillet`).
#[cfg(test)]
pub(crate) use polyline_fillet::{
    plan_polyline_corners, polyline_corner_tolerance, two_line_fillet_plan, PolylineCornerSpec,
};
// The single base-dimension resolution the DIMBASELINE/DIMCONTINUE capture check
// and the command both use, so a wrong pick is refused with the same reason at
// capture time and at apply time (ADR 0017 decision C, P5f).
pub(crate) use dimension_chain::ensure_chain_base;
pub(crate) use text_edit::resolve_text_edit_target;
// The two pure transformers (`most_readable_rotation` / `apply_text_case`) are
// exposed to the crate's tests only: the command paths below pin them end to
// end, and the unit tests pin the escape-preserving grammar directly.
#[cfg(test)]
pub(crate) use text_commands::{apply_text_case, most_readable_rotation};
// The explicit ARRAY entity cap (ADR 0016 decision D); re-exported so the
// command docs (`crate::app_drawing::MAX_ARRAY_ENTITIES`) resolve. It is a doc
// anchor only — the value is used inside the `array` module.
#[allow(unused_imports)]
pub use array::MAX_ARRAY_ENTITIES;

use helpers::{
    distance, geometry_kind, is_finite_point, is_multiline_text_source, offset_arc, offset_circle,
    offset_line, resolve_created_text_style,
};
use mirror_curves::mirror_geometry_with_options;
use offset_polyline::offset_polyline_with_distance;
