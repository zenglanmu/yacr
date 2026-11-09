//! Long-form notes for [`crate::CommandId::HatchEdit`] (`HATCHEDIT` /
//! `-HATCHEDIT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the implementation lives in `app_drawing/hatch_edit.rs`.
//!
//! ## Input
//!
//! The current selection is the whole input: **one or more** directly
//! addressable, model-space `AcDbHatch` entities. An empty selection is
//! `hatchedit.selection`; a duplicate ref is `hatchedit.duplicate`; a ref through
//! a block instance or sub-element, another document, a missing id or a
//! non-model-space entity is `hatchedit.not_addressable`; any other geometry kind
//! is `hatchedit.not_hatch`. A payload of [`crate::CommandPayload::None`] is the
//! bare selector and is refused as `hatchedit.needs_option`; any other payload is
//! `hatchedit.payload`.
//!
//! ## Typed option grammar
//!
//! This build has no interactive Hatch Edit dialog, so the option is typed on
//! the command line (case-insensitive):
//!
//! | option | behaviour |
//! | --- | --- |
//! | `STYLE IGNORE/OUTER/NORMAL` | set the DXF group 75 island style |
//! | `ANGLE <degrees>` | set the pattern angle (pattern fill only) |
//! | `SCALE <factor>` | set the pattern scale > 0 (pattern fill only) |
//! | `PATTERNTYPE USER/PREDEFINED/CUSTOM` | set DXF group 76 |
//! | `SOLID` / `GRADIENT` | replace the fill with the solid / default linear ramp |
//! | `COLOR <aci>` / `COLOR BYLAYER` | set the entity display colour |
//! | `TRANSPARENCY <0..=90>` / `TRANSPARENCY BYLAYER` | set the entity opacity |
//! | `LAYER <name>` | move every target to that drawing layer |
//! | `DRAWORDER BEHIND/FRONT/BOUNDARY` | model-space draw order (`BOUNDARY` refused) |
//! | `RECREATEBOUNDARY` | one closed polyline per stored loop |
//! | `SEPARATE` | split a multi-loop hatch into one hatch per loop |
//!
//! A bare `HATCHEDIT` is admitted by name and refused by the application with
//! `hatchedit.needs_option`. An argument outside the grammar is refused before
//! IPC with `invalid_hatchedit_argument_reason`.
//!
//! ## Named refusals (global scope)
//!
//! * `ORIGIN` — `hatchedit.missing_field`: the domain stores no pattern origin
//!   (group 43/44/45 are derived, not an editable field here);
//! * `ASSOCIATE` / `DISASSOCIATE` — `hatchedit.no_association`: this build has
//!   no hatch/boundary association record;
//! * `ANNOTATIVE` — `hatchedit.annotative_unsupported`: no annotative-scale
//!   model;
//! * `ADDBOUNDARIES` / `REMOVEBOUNDARIES` — `hatchedit.boundaries_unsupported`:
//!   the loop set cannot be rebuilt safely from added/removed picks, so the
//!   command refuses rather than fake a boundary edit.
//!
//! These refusals are answered before the selection is read, and none of them
//! opens a transaction.
//!
//! ## Rebuild
//!
//! ANGLE/SCALE/PATTERNTYPE/STYLE and SOLID/GRADIENT keep the stored loops,
//! plane, seeds, pixel size and every property the option does not name, and
//! rebuild the display through the domain constructor
//! (`HatchGeometry::from_import` / `with_style_and_fill`) — never by hand-editing
//! the display. `RECREATEBOUNDARY` writes non-associative closed polylines on the
//! hatch's layer/space; `SEPARATE` keeps the first loop in the original entity and
//! inserts one hatch per remaining loop (nested-island semantics are not
//! preserved — declared). Colour/transparency are entity display attributes;
//! layer and draw order are entity row fields.
//!
//! Every changed target (and every created entity) is written in **one
//! transaction / one undo step**, so one undo restores all of them. A run that
//! changes nothing is the named `hatchedit.unchanged` outcome with **no**
//! transaction, revision bump or undo entry.
//!
//! ## NOT IMPLEMENTED
//!
//! Pattern origin, association/disassociation, annotative scaling, add/remove
//! boundaries, draw order relative to a boundary, a per-loop pattern origin and
//! the interactive dialog. Work-only (Viewer mode refuses it at the command
//! layer).
