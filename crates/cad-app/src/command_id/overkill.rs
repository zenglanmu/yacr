//! Long-form notes for [`crate::CommandId::Overkill`] (`OVERKILL` / `-OVERKILL`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. The implementation is
//! `app_drawing::overkill` ([`crate::OverkillSettings`] is its typed payload).
//!
//! ## Input
//!
//! The current selection is the whole input (the declared divergence from
//! AutoCAD's interactive Delete Duplicate Objects dialog). `CommandPayload::None`
//! applies the documented defaults; the typed
//! [`crate::SelectionToolPayload::Overkill`] form carries the dialog's tolerance,
//! ignore toggles and options. An empty selection is `overkill.selection`. A ref
//! that is not directly addressable (another document, a block-instance member,
//! a sub-element) is `overkill.not_addressable`; a missing entity is
//! `overkill.target_missing`. A negative/non-finite tolerance is
//! `overkill.tolerance`. Every refusal happens before a write.
//!
//! ## Comparison model
//!
//! AutoCAD documents **one** comparison precision, the dialog's *Tolerance*
//! (`0` = objects must match exactly). There is no per-class fuzz. Two entities
//! are compared only in the **same space** and when every property whose ignore
//! toggle is off agrees:
//!
//! | property | comparison |
//! | --- | --- |
//! | Layer | the stored layer id |
//! | Color | the stored display colour (`EntityColor`) |
//! | Linetype | the stored linetype name / ByLayer / ByBlock (not the embedded scale) |
//! | Linetype scale | the entity's own factor, within the tolerance |
//! | Lineweight | the stored display lineweight |
//! | Transparency | the stored display transparency |
//!
//! Thickness, plot style and material are **not modelled** on a `cad-db` entity;
//! the defaults ignore them, and asking to compare one is the named
//! `overkill.<property>_not_modelled` refusal (never a silently weaker match).
//!
//! ## What is removed / combined
//!
//! | operation | option gate | result |
//! | --- | --- | --- |
//! | duplicate kinds (LINE, LWPOLYLINE, CIRCLE, ARC, SPLINE) | always | keep the lowest `(draw_order, id)`, delete the rest |
//! | zero-length / repeated straight polyline segments | `optimize_segments` and not `do_not_break_polylines` | rewrite the polyline (delete it if fewer than two vertices remain) |
//! | collinear LINEs that partially overlap (containment included) | `combine_partial_overlap` | keeper stores the union, other deleted |
//! | collinear LINEs that touch end to end | `combine_end_to_end` | keeper stores the union, other deleted |
//! | ARC on a comparable CIRCLE's circumference | always | delete the redundant ARC |
//! | standalone LINE/ARC duplicated by a polyline segment | `optimize_segments` | delete the standalone object; the polyline is the keeper and is never modified by this comparison |
//!
//! The keeper of every duplicate/merge is the **lowest `(draw_order, id)`**, so a
//! delete can never pick the wrong copy. `Maintain Associative Objects` is a
//! named no-op (`overkill.associative_noop`: no association model) and
//! `Ignore Polyline Segment Width` is reported as a no-op
//! (`overkill.segment_width_not_modelled`: no per-segment width field).
//!
//! Every successful write is **one transaction / one undo step** with full
//! before/after patches (deletes capture the display state). Success is
//! `overkill.applied` with the per-operation counts; nothing to do creates **no**
//! transaction and reports `overkill.unchanged`. Work-only.
