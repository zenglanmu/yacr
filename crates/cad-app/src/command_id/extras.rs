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
//! `WIPEOUTFRAME [0|1|2]` is the **system-variable spelling** of the
//! `WIPEOUT Frames` option: `0` hides every frame (the mask still masks), `1`
//! displays and plots them, `2` displays without plotting. It writes the same
//! `SessionState::drafting_extras.wipeout_frames` token through the same setter
//! the option and the drawing-settings panel use, so the three paths cannot
//! disagree — and, unlike `VSCURRENT`, that token has real consumers: the
//! display scene and the vector-plot context. The bare selector reports the
//! value; a number outside `0..=2` is `wipeoutframe.out_of_range` (never a
//! clamp). `SETVAR WIPEOUTFRAME [<n>]` reads/writes the same variable.
//!
//! # `CVPORT`: the session current-viewport marker
//!
//! `CVPORT` is the system variable that names **which viewport of the active
//! paper layout** the viewport-scoped commands act on. It is session state only:
//! the marker lives in `SessionState::viewport_markers` as one zero-based
//! `cad_db::Layout::viewports` index per paper layout, so `CVPORT <n>` performs
//! **no** database write, opens no transaction, bumps no revision, records no
//! `cad-history` entry and produces no `ChangeSet` (and therefore no undo step).
//!
//! * Bare `CVPORT` reports the stored marker of the active layout (its position
//!   and the layout's viewport count) and names the viewport `LAYVPI`,
//!   `VPSCALE` and `VPCLIP` will target.
//! * `CVPORT <n>` validates `n` against the active layout's viewport count and
//!   refuses anything outside `1..=<count>` by name (`cvport.out_of_range`,
//!   never clamped). The command line is **one-based** (AutoCAD's numbering: the
//!   first viewport of a layout is `1`); the stored marker and every payload
//!   index remain zero-based.
//! * The marker is **per layout**: switching layouts reports that layout's own
//!   stored marker, or the documented default `1` (its first viewport) when the
//!   layout has never been pointed at one. A layout with **no viewport** has no
//!   current viewport at all and reports `cvport.no_viewports` instead of a
//!   fabricated `1`.
//! * Model space is refused by name (`cvport.paper_space_required`): this build
//!   stores the marker per paper layout only, and inventing a model-space
//!   viewport id would be a fake.
//!
//! A stored marker that no longer names a viewport (the layout's viewport list
//! shrank) is refused by every reader (`cvport.out_of_range` for `CVPORT`,
//! `<command>.current_viewport_out_of_range` for the three commands) instead of
//! silently falling back to the first viewport.
//!
//! Consumers:
//!
//! * `LAYVPI` (no index) freezes the named layers in every viewport **except**
//!   the current one ([`crate::command_id::layout`]);
//! * `VPSCALE` (no index) rescales the current viewport;
//! * `VPCLIP` (no index) replaces the current viewport's clip.
//!
//! All three keep their explicit-index forms, and an explicit index always wins
//! over the marker.
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
