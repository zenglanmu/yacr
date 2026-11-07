//! Long-form notes for [`crate::CommandId::HatchGenerateBoundary`]
//! (`HATCHGENERATEBOUNDARY`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the implementation lives in `app_drawing/hatch_utilities.rs`.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::None`]: the current selection is the whole
//! input, exactly like `AREA OBJECT`. At least one selected entity is required
//! (`hatchgenerateboundary.selection`); every selected ref must be a **directly
//! addressable model-space** entity — a block-instance member, a sub-element, a
//! missing ref or another document's ref is
//! `hatchgenerateboundary.not_addressable`. A selected entity of any other
//! geometry kind is `hatchgenerateboundary.not_hatch` (the kind is named). Every
//! check runs before the transaction is opened, so a refusal writes nothing,
//! bumps no revision and records no undo entry.
//!
//! ## Output
//!
//! For **every loop** of **every selected hatch** (`HatchGeometry::loops()`) one
//! closed `AcDbPolyline` is inserted:
//!
//! * the loop's OCS 2D points are mapped to world space with
//!   [`cad_domain::HatchPlane::to_world`] — the same arbitrary-axis projection
//!   the hatch fill/display uses, so a tilted hatch's boundary lands where the
//!   fill is drawn;
//! * the loop's `bulges` are copied verbatim (an empty bulge list becomes one
//!   zero bulge per vertex), so arcs remain exact;
//! * the polyline is `closed == true` and the **repeated last point** is dropped
//!   (with the trailing degenerate bulge when the source stored one bulge per
//!   vertex) — the stored closed ring never carries a duplicate vertex;
//! * the new entity keeps the source hatch's **layer and space**; the boundary
//!   is a snapshot, never an associative boundary. If the source is a layout
//!   hatch this command still refuses it (model-space input only).
//!
//! All polylines from all selected hatches are inserted in **one** transaction /
//! one undo step. The source hatches are retained. The success diagnostic is
//! `hatchgenerateboundary.applied` and names the loop count.
//!
//! ## Divergences / NOT IMPLEMENTED
//!
//! * AutoCAD's command is invoked by selecting a hatch interactively; this build
//!   reads the **current selection** (the `AREA OBJECT` / `MATCHPROP` shape) and
//!   implements no pick prompt and no option prompt.
//! * Associativity, source-edge export on the generated polyline, and any update
//!   of already-generated boundaries when the hatch changes are NOT IMPLEMENTED.
//! * Work-only (Viewer mode refuses it at the command layer).
