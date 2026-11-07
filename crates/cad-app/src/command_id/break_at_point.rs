//! Long-form notes for [`crate::CommandId::BreakAtPoint`] (`BREAKATPOINT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. The implementation is
//! `app_drawing::break_curve::break_at_point`.
//!
//! ## Interactive shape
//!
//! `BREAKATPOINT` is an **interactive tool**, not a selection-based instant
//! command: the first click picks the object, the second specifies the break
//! point (which may lie off the object — it is **projected** onto it). The tool
//! is [`crate::draw_tool::DrawToolKind::BreakAtPoint`]; the host maps its
//! confirmed intent onto [`crate::CommandPayload::BreakAtPoint`]
//! `{ target, point }`.
//!
//! ## Supported geometry (exact)
//!
//! * LINE → two LINEs in the source's direction order;
//! * ARC → two ARCs whose sweeps sum to the source sweep;
//! * **open** LWPOLYLINE → two open polylines, split at a vertex or on a
//!   segment (a bulged segment is split into two arcs with matching bulges).
//!
//! AutoCAD's BREAKATPOINT refuses closed objects, and so does this build. The
//! refusals are named and change nothing:
//!
//! | code | meaning |
//! | --- | --- |
//! | `breakatpoint.payload` | a payload other than `BreakAtPoint` |
//! | `breakatpoint.invalid_point` | a non-finite break point |
//! | `breakatpoint.not_addressable` | foreign document, block-instance member, sub-element or non-model space |
//! | `breakatpoint.target_missing` | the target ref does not resolve |
//! | `breakatpoint.closed` | a CIRCLE or a **closed** polyline |
//! | `breakatpoint.spline_unsupported` | a SPLINE (`Unsupported`) |
//! | `breakatpoint.unsupported` | any other geometry, or a non-`+Z` bulged polyline |
//! | `breakatpoint.projection_failed` | the point has no projection (degenerate target / on the axis) |
//! | `breakatpoint.point_outside_span` | the projection falls outside an open target's span |
//! | `breakatpoint.point_at_endpoint` | the break point projects onto an endpoint |
//! | `breakatpoint.degenerate_piece` | one side would be empty / has no representable arc |
//!
//! ## The write
//!
//! The source is retained as the **first** piece (same id, layer, space, draw
//! order and display state; geometry and type key change) and the second piece
//! is inserted with a **fresh id** copying the source's display attributes. One
//! transaction / one undo step; undo restores the source and removes the
//! inserted piece. Work-only.
//!
//! Polyline splitting is a declared v1 capability for the **open** case; a
//! closed polyline is refused by name (AutoCAD's own BREAKATPOINT rule), never
//! silently opened.
