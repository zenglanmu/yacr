//! Long-form contract for [`crate::CommandId::CreateXLine`] (`XLINE`) and
//! [`crate::CommandId::CreateRay`] (`RAY`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variants keep a compact doc and link here. The command body lives
//! in `app_drawing::construction_line`.
//!
//! ## What is stored (read this before trusting the command)
//!
//! `cad-db` has **no infinite-line geometry**. Neither command therefore stores
//! an infinite line: each creates exactly one finite
//! [`cad_domain::SemanticGeometry::Line`] that is the construct line **clipped to
//! an XY box**, and the stored entity keeps its source class name in
//! `DbObject::type_key` (`AcDbXline` / `AcDbRay`). The reduction is the same one
//! the DWG importer performs
//! (`cad_import_acadrust::construct_line::ray_semantics`), so a command-created
//! XLINE/RAY and an imported one are interchangeable in the database, the
//! renderer and the exporter.
//!
//! Consequences a caller must not paper over:
//!
//! * the stored segment's length follows the clip box, not the (infinite) source
//!   construct line;
//! * moving, gripping or trimming it edits that finite segment;
//! * export writes a `LINE` (the `AcDbXline`/`AcDbRay` key is preserved, but the
//!   DXF entity is a LINE) — a round-trip is lossy, never an exact XLINE/RAY.
//!
//! Every successful command pushes a `drawing.xline` / `drawing.ray` diagnostic
//! that states exactly this, so the loss is visible rather than implied.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::XLine`] or [`crate::CommandPayload::Ray`]
//! `{ base, through }`, i.e. **two picked points**: the base point (a point the
//! construct line passes through) and the through point (a second point on it,
//! which fixes the direction `through − base`). The interactive capture is
//! [`crate::draw_tool::DrawToolKind::XLine`] / `Ray` (`PointArity::Exact(2)`, no
//! entity picks, no selection, no typed text), and the host maps the confirmed
//! [`crate::DrawIntent::XLine`] / `Ray` onto this payload unchanged.
//!
//! ## Clip box (identical to the importer)
//!
//! * When the drawing has model bounds `(lo, hi)`, the margin is
//!   `(distance(lo, hi) * 0.05).max(1.0)` and the box is those bounds expanded by
//!   the margin in X and Y, with `z = 0`.
//! * An empty drawing (no bounds) uses a default 1000-unit box around `base`, with
//!   `z = 0` — the importer's own fallback.
//!
//! The segment span is `max(|max.x − min.x|, |max.y − min.y|) * 4.0`, at least
//! `1.0`. XLINE starts at `base − direction * span` and ends at
//! `base + direction * span`; RAY starts at `base` and only extends along
//! `+direction` (its negative half is never drawn or stored). Both are then
//! clipped with [`cad_geometry::clip_segment_to_xy_rect`]. A straight line enters
//! and leaves an axis-aligned box at most once, so the result is one segment —
//! never a stitched multi-run approximation.
//!
//! ## Refusals (all before the transaction; nothing is written)
//!
//! * `xline.invalid_point` / `ray.invalid_point` — a non-finite `base` or
//!   `through`;
//! * `xline.degenerate_direction` / `ray.degenerate_direction` — the direction is
//!   non-finite or its length is `<= 1e-12` (the two picks coincide);
//! * `xline.outside_bounds` / `ray.outside_bounds` — the clipped result is empty
//!   or shorter than `1e-9` (a RAY pointing away from the box never enters it);
//! * the shared create funnel's own refusals (a missing or locked active layer,
//!   database geometry validation).
//!
//! A successful command is one entity on the session's active layer in model
//! space, one transaction and one undo step.
//!
//! Work-only: both commands write the drawing, so Viewer mode refuses them
//! through [`crate::CommandId::requires_work_mode`] +
//! [`crate::SessionState::authorize`] (`CadError::PermissionDenied`).
