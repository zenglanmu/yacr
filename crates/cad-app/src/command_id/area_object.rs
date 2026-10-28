//! Long-form notes for [`crate::CommandId::AreaObject`] (`AREA OBJECT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. Nothing is
//! implemented in this module — the exact-area computation lives in
//! `app_measure_object.rs`.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::None`]: the session's current selection
//! **is** the input. Exactly one selected entity is required — zero
//! (`area.object.selection_empty`) and more than one
//! (`area.object.selection_multiple`) are named refusals before any geometry is
//! read, so there is never a partial or "first one wins" result. A ref with an
//! instance path (block-instance member), a sub-element or another document's
//! id is refused (`area.object.not_addressable` / `area.object.target_missing`),
//! exactly like the property write.
//!
//! **Declared divergence from AutoCAD**: AutoCAD's `AREA`/`Object` is an
//! interactive *pick* prompt; this build reads the **current selection**
//! instead, the same shape as `MATCHPROP`'s selection-based form. No pick
//! prompt is implemented.
//!
//! ## Accepted kinds and the exact value (no polygon approximation anywhere)
//!
//! * `Circle` — `π r²` (the entity's own radius; no plane needed);
//! * `Ellipse` — `π a b` with `a = |major_axis|` and `b = a · ratio`, i.e. the
//!   stored `ratio` is honoured **verbatim** (no axis is swapped when a source
//!   stores a ratio above 1). Only a **full** ellipse (sweep equal to one turn
//!   within [`crate::FULL_TURN_TOLERANCE_RADIANS`]) is accepted; an elliptical
//!   arc is refused (`area.object.ellipse_arc`) rather than reported as a whole
//!   ellipse;
//! * closed `Polyline` — the bulge-aware exact area in the polyline's **own
//!   plane** (`cad_geometry::measure_bulged_ring_area`): chord shoelace plus one
//!   exact circular-segment area per bulge, measured in the same plane the
//!   display tessellator draws the bulge in. A polyline that is not `closed`
//!   (`area.object.polyline_open`), whose plane cannot be derived
//!   (`area.object.polyline_plane`), or that is non-planar
//!   (`area.object.polyline_non_planar`) is refused.
//!
//! Every other geometry kind (`Spline`, `Arc`, `Line`, text, hatch, dimension,
//! block, image, opaque, …) is an explicit [`cad_domain::CadError::Unsupported`]
//! refusal naming the kind (`area.object.unsupported_kind`) — a capability gap,
//! never a zero or an approximation. A degenerate radius/axis
//! (`area.object.degenerate`) and a zero-area result (`area.object.zero_area`)
//! are refusals too.
//!
//! ## Result
//!
//! The result is a [`crate::CommandOutcome::measurement`] whose algorithm is
//! [`cad_db::MeasurementAlgorithm::ObjectArea`] — its own variant, because the
//! value did not come from a picked polygon ring. Its `inputs`/`plane` carry:
//!
//! * `Circle` / `Ellipse` — `inputs: []` and `plane: None`. A closed curve has
//!   no vertices; inventing picked points or a plane would be a fabrication, and
//!   there is no ring to project;
//! * closed `Polyline` — `inputs` are the ring's **own stored vertices** in
//!   world coordinates (the entity's real geometry, closed ring without a
//!   duplicated last vertex; never synthesised points) and `plane` is the ring's
//!   own plane (`cad_geometry::BulgedRingArea::plane`: origin at the first
//!   vertex, right-handed in-plane basis).
//!
//! ## Mode and writes
//!
//! Work-only like the other measurement commands, and — because measuring
//! writes nothing — it opens **no transaction**, records **no undo entry** and
//! bumps no revision. The record is stored as the session's last measurement
//! (the same field the interactive measurement paths use).
