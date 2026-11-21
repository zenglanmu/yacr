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
//! * `MSTRETCH` takes the windows in a payload, and its typed command line
//!   builds that same payload:
//!   `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> […]`
//!   (world base/delta, then one or more logical-pixel crossing windows). An odd
//!   window-token count is `mstretch.argument_odd` and a zero-size window is the
//!   handler's `mstretch.degenerate_window` — both before any write.
//!
//! # View-only commands
//!
//! `-PAN` is the typed argument form of the existing `PAN` path (same
//! `CommandId::Pan`, same camera shift): no database write, revision bump, undo
//! entry or `ZOOM PREVIOUS` step. `REGEN3` is an **alias** of `REGEN` (same
//! `CommandId`), not a second regeneration path.
//!
//! `VIEWRES <1..=1000>` stores a session percentage and it **reaches the
//! display**: the base build turns it into a ceiling on chord segments per curve
//! ([`cad_representation::viewres_segment_ceiling`]). The command reports the
//! honest boundary with the value — circles/arcs/ellipses are drawn as analytic
//! conics and have no chords to bound, and the value is a ceiling on the display
//! tolerance rather than a smoothness setting.
//!
//! `VSCURRENT` stores a session-only 3D **body** visual style. `WIREFRAME`
//! (also spelled `2DWIREFRAME`/`2D`) draws bodies as edges only; `SHADED` (also
//! `SHADEDWITHEDGES`, and the session default) keeps the filled mesh plus its
//! stored edges. Two declared divergences from AutoCAD: the `SHADED` token
//! **includes** the stored-edge overlay (AutoCAD's shaded style has no edges, and
//! this build has no separate edge-free spelling), and a curved body shows only
//! its stored edge loops in wireframe (it has no silhouette generator, so the
//! outer contour can differ from AutoCAD's). Every other style is refused by
//! name (`vscurrent.style_not_implemented`), never mapped onto either token. The
//! token reaches the representation as
//! [`cad_representation::RepresentationContext::body_wireframe`], changes no
//! other geometry and writes no database row, revision or history entry.
//!
//! `IBL [ON|OFF]` toggles (bare) or sets the session PBR environment-lighting
//! switch. `IBL`/`ON`/`1`/`IMAGEBASED` store `PbrEnvironment::ImageBased` (the
//! default); `LEGACY`/`OFF`/`0` store `PbrEnvironment::Legacy`. Any other typed
//! argument is refused by name at the command-line layer (`ibl.invalid_value`)
//! instead of being treated as a toggle. It is session-only: no database write,
//! revision bump, history entry, `ChangeSet` or persisted preference. The status
//! wire projects it as `iblEnabled`.
//!
//! `WIPEOUTFRAME [0|1|2]` is the **system-variable spelling** of the
//! `WIPEOUT Frames` option: `0` hides every frame (the mask still masks), `1`
//! displays and plots them, `2` displays without plotting. It writes the same
//! `SessionState::drafting_extras.wipeout_frames` token through the same setter
//! the option and the drawing-settings panel use, so the three paths cannot
//! disagree — and the token has two distinct consumers: the display scene (which
//! draws the frame for `1` and `2`) and the vector-plot context (which plots it
//! only for `1`). The bare selector reports the value; a number outside `0..=2`
//! is `wipeoutframe.out_of_range` (never a clamp). `SETVAR WIPEOUTFRAME [<n>]`
//! reads/writes the same variable.
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
//! `property.unknown_linetype` refusal.
//!
//! `LINETYPE CREATE <name> <dash>,<gap>,…` and `LINETYPE DELETE <name>` write the
//! **table** (one transaction / one undo step each) through the database's
//! single-purpose APIs. The name grammar, the case-insensitive uniqueness rule
//! (`linetype.exists`), the reserved default (`linetype.continuous_reserved`) and
//! the renderable pattern subset (`linetype.bad_pattern`: at least one element,
//! every element finite and non-zero, at least one visible dash — positive is a
//! dash, negative a gap, DXF group 49) are validated before anything is staged.
//! `DELETE` uses the same in-use rules as `PURGE LINETYPE`
//! (`linetype.in_use` — a layer row or an entity name still references it;
//! `linetype.continuous_undeletable`). A non-empty `DESCRIPTION` is the named
//! `linetype.description_not_modelled` refusal: this build's linetype row stores
//! name + pattern + `complex` only, so the value is never silently dropped.
//! `RENAME` keeps its cascade. `-LINETYPE` normalizes onto the same command, so
//! it shares every arm. `LOAD` (a `.lin` file) remains NOT IMPLEMENTED: there is
//! no file-loading channel in this build.
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
