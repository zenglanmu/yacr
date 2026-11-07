//! Long-form notes for the two POINT display-style sysvars.
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variants keep compact docs and link here. Nothing here is
//! implemented in this module — the header write lives in
//! `app_drawing::point_style`.
//!
//! ## `PDMODE <n>`
//!
//! Writes the drawing header's `$PDMODE` (the drawing-global POINT display mode)
//! from the command line.
//!
//! Payload is [`crate::CommandPayload::PointDisplayMode`] `i16` and is stored
//! **raw** in [`cad_db::DrawingDatabase`]: `0` dot, `1` none, `2` plus, `3`
//! cross, `4` vertical tick, plus the `32`/`64`/`96` surrounding-shape bits and
//! any value this build cannot express. Interpretation (including the
//! `overlay.point-mode-*` diagnostics) stays in the overlay; the database never
//! invents a marker shape and never rewrites the header.
//!
//! One real write is **one revision bump** and no entity change (published as a
//! `ChangeSet` through the ordinary "replace the document's
//! `Arc<DrawingDatabase>`" path), which is exactly what refreshes the display:
//! the render scene caches the POINT display style per base revision. Writing
//! the already stored value is an authoritative no-op (`before == after`, no
//! bump, a `(already stored, no change)` diagnostic).
//!
//! **No undo entry** — there is no header-variable patch kind in `cad-history`,
//! so this command deliberately records no history step rather than fabricating
//! an entity patch. That is a documented exception to "one command = one undo
//! step": UNDO cannot restore the previous mode.
//!
//! **Not a round-trip claim**: the DXF exporter does write `$PDMODE` (see
//! [`crate::app_drawing`]), so a SAVE→OPEN cycle keeps the value through our own
//! writer + reader — internal consistency only, never a vendor compatibility
//! claim. A source drawing's value is still overwritten by this write, exactly
//! as AutoCAD's `PDMODE` does.
//!
//! Work-only: it writes the document, so Viewer mode refuses it through
//! [`crate::CommandId::requires_work_mode`] + [`crate::SessionState::authorize`]
//! (`CadError::PermissionDenied`). A bare `PDMODE` (no argument) is refused by
//! name — a missing payload is an explicit `InvalidInput`, never a fabricated
//! default. See [`crate::app_drawing`]'s `point_style` module.
//!
//! ## `PDSIZE <n>`
//!
//! Writes the drawing header's `$PDSIZE` (the drawing-global POINT display size)
//! from the command line. Payload is
//! [`crate::CommandPayload::PointDisplaySize`] `f64`, stored **raw**: `> 0` an
//! absolute drawing-unit size, `0` the viewport-relative DWG default, `< 0` a
//! percentage of the viewport height. A non-finite value is refused as
//! `InvalidInput` by both this command and the database write.
//!
//! One real write is one revision bump with no entity change (the display
//! refresh signal, see [`crate::CommandId::SetPointDisplayMode`]); the stored
//! value is a no-op, and **no undo entry** is recorded (no header patch kind).
//! The DXF exporter writes `$PDSIZE`, so a SAVE→OPEN cycle keeps it.
//!
//! Work-only (a document mutation; Viewer is `PermissionDenied`). A bare
//! `PDSIZE` is refused by name.
