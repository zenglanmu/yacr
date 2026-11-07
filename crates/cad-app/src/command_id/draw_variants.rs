//! Draw-command contracts that share a command module without sharing a shape.
//!
//! Kept as a doc/spec module so `command_id.rs` stays inside the code-size limit;
//! the commands themselves are dispatched by
//! [`crate::app_drawing::drawing_create`].

/// `WIPEOUT` — a closed polygon mask.
///
/// # What is implemented
///
/// Payload is [`crate::CommandPayload::Points`] with the **closed implicit ring**
/// of the mask boundary: at least three finite, `+Z`-coplanar vertices in click
/// order. The ring is implicit, so the first vertex is **not** repeated at the
/// end; a payload that repeats it (a duplicate closing point) is refused rather
/// than silently deduplicated. The stored geometry is
/// `SemanticGeometry::Mask { boundary, inverted: false }` and the entity's stored
/// type key is `AcDbWipeout` (the domain's own `class_name` mapping). One entity,
/// one transaction and one undo step on the session's active layer, through the
/// shared create funnel.
///
/// The interactive form is the open-ended [`crate::draw_tool::DrawToolKind::Wipeout`]
/// capture: first point, then every further point, finished with Enter — which is
/// AutoCAD's default polygon flow, and which the existing open-ended capture
/// machine expresses without a new state. At least three points are required
/// before the capture is confirmable.
///
/// Refusals (all before any write, all with a stable machine prefix):
///
/// * `wipeout.payload` — the payload is not a point list;
/// * `wipeout.too_few_points` — fewer than three vertices;
/// * `wipeout.invalid_point` — a non-finite vertex;
/// * `wipeout.not_planar` — the vertices are not on one `+Z` plane (the same
///   flat work-plane subset RECTANG/ELLIPSE/POLYGON use);
/// * `wipeout.duplicate_vertex` — two consecutive vertices (the closing pair
///   included) coincide, so the ring has no real edge there;
/// * `wipeout.zero_area` — the ring encloses no area (all vertices collinear).
///
/// # NOT IMPLEMENTED (named, not implied)
///
/// * **`Frames` ON/OFF/`Display-not-plot`** — there is no wipeout-frame model at
///   all (no frame entity, no `WIPEOUTFRAME` sysvar, no plot/display split), so
///   the command authors no frame;
/// * the **`Polyline`** and **`Erase Polyline`** options — no existing polyline
///   can be converted or consumed, and no temp polyline is created;
/// * **self-intersection detection** — a self-crossing ring is stored as given
///   (the renderer fills it with the even-odd rule), never rejected;
/// * **picking/frame selection** of an existing wipeout, and `WIPEOUT` editing;
/// * `inverted: true` (mask the outside) — the create path always stores
///   `inverted: false`; the inverted mode is import/display only.
///
/// Work-only: Viewer mode refuses it at the command layer.
pub struct WipeoutCommand;
