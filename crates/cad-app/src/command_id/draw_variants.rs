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

/// `POLYGON CIRCUMSCRIBED` — AutoCAD's `POLYGON` → `Circumscribed about circle`
/// (`C`) construction, as a **canonical selector of its own**.
///
/// # Why not an alias
///
/// `POLYGON` is the inscribed construction: the second pick is a **vertex**, so it
/// lies on the polygon. This command's second pick lies on the **inscribed
/// (tangent) circle** — the circle every edge touches — and is the midpoint of the
/// first edge. The same two clicks therefore produce different rings, so the names
/// are two distinct capture paths, never one alias of the other.
///
/// # What is implemented
///
/// Payload is [`crate::CommandPayload::Points`] `[center, tangent_point]`, exactly
/// the two clicks. The side count is the session sysvar
/// [`crate::SessionState::polygon_sides`] (`MIN_POLYGON_SIDES..=MAX_POLYGON_SIDES`,
/// i.e. `3..=1024`, AutoCAD's `POLYSIDES` range), never a payload field.
///
/// Construction ([`crate::draw_tool::circumscribed_polygon_vertices`]):
///
/// * `r = |tangent_point − center|` is the tangent-circle (apothem) radius;
/// * the vertex radius is `R = r / cos(π/n)`;
/// * `θ = atan2(tangent_point − center)` is the **midpoint of the first edge**, so
///   the vertices sit at `θ − π/n + i·2π/n` (`i = 0..n`), stepping
///   counter-clockwise about `+Z`;
/// * every edge midpoint is exactly `r` from the centre, the ring is **closed**
///   with one zero bulge per vertex (`AcDbPolyline`, the same LWPOLYLINE the
///   inscribed variant stores), and the first vertex is not repeated.
///
/// One entity, one transaction, one undo step on the active layer, through the
/// shared create funnel.
///
/// Refusals (all before any write, all with a stable machine prefix):
///
/// * `polygoncircumscribed.invalid_point` — a non-finite pick;
/// * `polygoncircumscribed.sides` — a stored side count outside `3..=1024` (the
///   defensive guard against a corrupted sysvar; the session setter already
///   refuses such a value);
/// * `polygoncircumscribed.not_planar` — the tangent point is off the centre's
///   `+Z` plane (the flat work-plane subset);
/// * `polygoncircumscribed.zero_radius` — the picks coincide (no tangent circle).
///
/// # NOT IMPLEMENTED (named, not implied)
///
/// * the `Edge` construction — there is no first-edge-endpoint capture;
/// * a **numeric radius** orientation — AutoCAD rotates the first edge to the snap
///   rotation angle when a number is typed; the pointer direction is the only
///   rotation control here;
/// * a persisted side count (`POLYSIDES` is session state, reset on a session
///   rebuild) and 3D/OCS input.
///
/// Work-only: Viewer mode refuses it at the command layer.
pub struct PolygonCircumscribedCommand;
