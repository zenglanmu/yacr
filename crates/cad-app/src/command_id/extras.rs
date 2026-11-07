//! The 2026-11-07 AutoCAD-2026 extras batch: `SKETCH`, `TRACE`, `MOCORO`,
//! `MSTRETCH`, `-PAN`, `VIEWRES`, `VSCURRENT`, `REGEN3`, `LINETYPE` and
//! `-LINETYPE`.
//!
//! # Types that do not exist
//!
//! AutoCAD's `SKETCH` and `TRACE` are legacy entity types whose interactive
//! behaviour (timer sampling, a `TRACE` entity with a width) this build does not
//! model. They are therefore implemented as **ordinary LWPOLYLINE geometry**:
//! `SKETCH` stores one open `Polyline` from a point sequence and `TRACE` stores
//! one closed 4-vertex band. The entity `type_key` is the polyline's own
//! (`AcDbPolyline`); no `SKETCH`/`TRACE` type is invented and a later reader must
//! not infer one.
//!
//! # Interactive halves that are the host's job
//!
//! * `SKETCH`'s freehand sampler is **NOT IMPLEMENTED** in `cad-app`: the
//!   application takes the already sampled `CommandPayload::Points` sequence.
//!   There is no timer, no pointer capture and no auto-close.
//! * `MOCORO` is the **typed** combined transform only. AutoCAD's in-command
//!   gizmo/prompt loop is NOT IMPLEMENTED; the caller supplies the complete
//!   operation (`MOVE`/`COPY` delta, or a `ROTATE`/`SCALE` base plus amount).
//! * `MSTRETCH` takes the windows in a payload. Its command-line form has no
//!   parser here (the rectangle list is not expressible in the one-argument
//!   selector grammar without inventing a delimiter); hosts/IPC supply
//!   `CommandPayload::MStretch`.
//!
//! # View-only commands
//!
//! `-PAN` is the typed argument form of the existing `PAN` path (same
//! `CommandId::Pan`, same camera shift): no database write, revision bump, undo
//! entry or `ZOOM PREVIOUS` step. `VIEWRES` stores a session percentage in
//! `[1, 1000]` and `VSCURRENT WIREFRAME` stores a session visual-style token;
//! neither touches the drawing. `REGEN3` is an **alias** of `REGEN` (same
//! `CommandId`), not a second regeneration path.
//!
//! `VSCURRENT` accepts only `WIREFRAME`; every 3D/shaded style is refused by
//! name (`vscurrent.style_not_implemented`). No renderer change is made in this
//! lane: the token is published as `SessionState::view_extras.visual_style` for
//! a host to consume, and no pixel/GPU effect is verified.
//!
//! # LINETYPE
//!
//! `LINETYPE LIST` is a read-only report of the drawing's linetype table.
//! `LINETYPE SET <name>` writes the named linetype to every currently selected
//! entity through the existing `SETENTITYPROPERTY` / `property_value` path (one
//! transaction, one undo step). An unknown name is the property path's own
//! `property.unknown_linetype` refusal. There is **no** linetype-table write or
//! load API: `LOAD`, `CREATE`, `DELETE`, `RENAME` and `SET` on a linetype table
//! row are NOT IMPLEMENTED. `-LINETYPE` normalizes onto the same command.
//!
//! # Layer / history discipline
//!
//! Every write command validates its whole input before opening a transaction,
//! commits on the session's active layer, and records exactly one transaction /
//! one undo step. Refusals happen before the transaction opens, so a refused
//! command never leaves an empty undo entry.

use crate::MocoroCommand;

/// The semantic input a `MOCORO` command expects.
pub type Mocoro = MocoroCommand;
