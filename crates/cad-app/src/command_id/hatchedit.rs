//! Long-form notes for [`crate::CommandId::HatchEdit`] (`HATCHEDIT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the implementation lives in `app_drawing/hatch_edit.rs`.
//!
//! ## Input
//!
//! The current selection is the whole input: **exactly one** directly
//! addressable, model-space `AcDbHatch` entity. A different count is
//! `hatchedit.selection`; a ref through a block instance or sub-element, another
//! document, a missing id or a non-model-space entity is
//! `hatchedit.not_addressable`; any other geometry kind is `hatchedit.not_hatch`.
//! A payload of [`crate::CommandPayload::None`] is the bare selector and is
//! refused as `hatchedit.needs_option`; any other payload is
//! `hatchedit.payload`.
//!
//! ## Typed option grammar (the only v1 surface)
//!
//! This build has no interactive Hatch Edit dialog, so the option is typed on
//! the command line (case-insensitive):
//!
//! * `HATCHEDIT ANGLE <degrees>` — set the stored pattern angle. The value is a
//!   finite decimal, converted to radians exactly once, and is applied only to a
//!   `HatchFill::Pattern` fill; a Solid/Gradient fill is `hatchedit.not_pattern`.
//! * `HATCHEDIT SCALE <factor>` — set the stored pattern scale. The factor must
//!   be finite and strictly positive (`hatchedit.scale` otherwise), and is
//!   likewise pattern-only.
//! * `HATCHEDIT SOLID` — replace the fill with `HatchFill::Solid`.
//! * `HATCHEDIT GRADIENT` — replace the fill with the default two-colour linear
//!   ramp (angle 0, ACI 1 red → ACI 5 blue).
//!
//! A bare `HATCHEDIT` is admitted by name and refused by the application with
//! `hatchedit.needs_option`, which lists the four implemented forms. An
//! argument outside the grammar is refused before IPC with
//! `invalid_hatchedit_argument_reason`.
//!
//! ## Rebuild
//!
//! ANGLE/SCALE keep the same loops, plane, style, seeds, pixel size and
//! `pattern_type`, and only the pattern's `angle`/`scale` change. SOLID/GRADIENT
//! keep loops, plane, style, seeds and pixel size, and replace the fill; both
//! record `HatchPatternType::Predefined` (`76=1`), matching what the HATCH
//! creation path writes for those fill kinds. The display is always rebuilt by
//! the domain constructor (`HatchGeometry::from_import`), never hand-edited.
//! The entity keeps its id, layer, space, draw order and `AcDbHatch` type key,
//! and is written with **one** `update_entity` in one transaction / one undo
//! step.
//!
//! A no-op — the same angle, the same scale, or the same fill variant — is the
//! named `hatchedit.unchanged` outcome with **no** transaction, revision bump or
//! undo entry.
//!
//! ## NOT IMPLEMENTED
//!
//! The options AutoCAD's dialog offers beyond this v1 subset: pattern name /
//! `Pattern`, colour, layer/transparency/lineweight `Properties`, draw order,
//! `Add`/`Remove` boundaries, re-associativity, annotative scaling, origin, and
//! the `-HATCHEDIT` command-line variant. Work-only (Viewer mode refuses it at
//! the command layer).
