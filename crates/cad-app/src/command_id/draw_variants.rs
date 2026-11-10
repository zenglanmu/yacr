//! Draw-command contracts that share a command module without sharing a shape.
//!
//! Kept as a doc/spec module so `command_id.rs` stays inside the code-size limit;
//! the commands themselves are dispatched by
//! [`crate::app_drawing::drawing_create`].

/// `WIPEOUT` — a closed polygon mask.
///
/// # What is implemented
///
/// The free point-run form is [`crate::CommandPayload::Points`] or
/// `WipeoutPayload::Points` with the **closed implicit
/// ring** of the mask boundary: at least three finite, `+Z`-coplanar vertices in
/// click order. The ring is implicit, so the first vertex is **not** repeated at
/// the end; a payload that repeats it (a duplicate closing point) is refused
/// rather than silently deduplicated. The stored geometry is
/// `SemanticGeometry::Mask { boundary, inverted: false }` and the entity's stored
/// type key is `AcDbWipeout`. One entity, one transaction and one undo step on
/// the session's active layer.
///
/// `WipeoutPayload::Polyline` is the documented
/// `Polyline` → `Erase Polyline [Yes/No]` form: the target must be a closed,
/// straight-segment `LWPOLYLINE` (zero width is the only width the model has).
/// `erase = true` deletes the source in the same transaction; `false` keeps it.
///
/// `WipeoutPayload::Frames` stores the session
/// `WIPEOUTFRAME` state (`0` off, `1` on, `2` display-not-plot) and applies it to
/// every wipeout in the drawing. `ON` draws and plots the boundary frame, `OFF`
/// draws only the fill, and `2` draws the frame on screen while the vector plot
/// path (built with the frame flag clear) omits it. The state never writes the
/// drawing.
///
/// Refusals (all before any write, all with a stable machine prefix):
///
/// * `wipeout.payload` — the payload is not a point list / supported option;
/// * `wipeout.too_few_points` — fewer than three vertices;
/// * `wipeout.invalid_point` — a non-finite vertex;
/// * `wipeout.not_planar` — the vertices are not on one `+Z` plane;
/// * `wipeout.duplicate_vertex` — two consecutive vertices coincide;
/// * `wipeout.zero_area` — the ring encloses no area;
/// * `wipeout.polyline_open`, `wipeout.polyline_bulged`,
///   `wipeout.polyline_not_polyline`, `wipeout.polyline_not_addressable`,
///   `wipeout.polyline_target_missing` — the `Polyline` option's target checks.
///
/// # NOT IMPLEMENTED (named, not implied)
///
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
