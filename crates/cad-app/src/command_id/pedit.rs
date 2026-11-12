//! Long-form notes for [`crate::CommandId::Pedit`] (`PEDIT`), AutoCAD 2026.
//!
//! The variant itself keeps a compact doc and links here; the implementation
//! lives in `app_drawing::pedit`. This is a **declared v1 subset**, not a port of
//! the whole command: the core options below are implemented (the full typed
//! option table lives in `app_drawing::pedit`), and every other AutoCAD `PEDIT`
//! surface is refused by name.
//!
//! ## Why an option argument (and not an option prompt)
//!
//! AutoCAD's `PEDIT` picks one polyline and then shows an option prompt
//! (`Close/Open/Join/Width/Edit vertex/Fit/Spline/Decurve/Ltype gen/Undo`). This
//! build has **no in-command option prompt**: the fixed-step capture machine and
//! the command line cannot pause a command to ask for a sub-option. The option is
//! therefore typed as part of the command line, `PEDIT CLOSE|OPEN|REVERSE`
//! (case-insensitive, with the single letters `C`, `O`, `R` accepted), exactly
//! the way `ZOOM WINDOW` and `TORIENT <angle>` carry their own argument forms.
//!
//! A bare `PEDIT` and an unknown argument are **explicit named refusals** that
//! list the implemented options (`pedit.needs_option` /
//! `invalid_pedit_argument_reason`). Nothing is defaulted to Close and no
//! fabricated prompt is shown.
//!
//! ## Input
//!
//! The current session selection is the whole input: **exactly one** directly
//! addressable model-space `LWPOLYLINE` (`SemanticGeometry::Polyline`) for every
//! option except `WIDTH`, which accepts **one or more** selected polylines (its
//! contract is "set this uniform width on each of them, in one transaction").
//!
//! * `pedit.selection` — none or more than one selected entity (AutoCAD's
//!   `Multiple` mode and its "Select polyline" prompt are NOT IMPLEMENTED);
//! * `pedit.not_addressable` — another document's ref, a block-instance member, a
//!   sub-element, a missing entity, or an entity outside model space;
//! * `pedit.not_polyline` — the selected entity is not an LWPOLYLINE
//!   (`Unsupported`; the line/arc conversion prompt `PEDITACCEPT` is NOT
//!   IMPLEMENTED).
//!
//! ## The core implemented options
//!
//! | option | precondition | write | already in that state |
//! | --- | --- | --- | --- |
//! | `CLOSE` (`C`) | open, at least 3 vertices | `closed = true` | `pedit.already_closed`, no transaction |
//! | `OPEN` (`O`) | closed | `closed = false` | `pedit.already_open`, no transaction |
//! | `REVERSE` (`R`) | any polyline (open or closed) | the `REVERSE` geometry rule | — (a reverse always changes the stored order) |
//! | `WIDTH <w>` | one or more polylines; finite `w >= 0` | `width = w` on every selected polyline | `pedit.width_unchanged`, no transaction |
//!
//! `CLOSE` refuses an open polyline with fewer than three vertices
//! (`pedit.too_few_vertices`): two points cannot form a closed ring. `OPEN`
//! needs no vertex count — an already-open polyline is the `pedit.already_open`
//! no-op, and a closed two-point polyline is opened exactly as stored.
//!
//! `REVERSE` reuses the **same pure rule** as [`crate::CommandId::Reverse`]
//! (`app_drawing::modify_extra::reverse_geometry`, exposed crate-wide rather than
//! duplicated): the point order is reversed and the bulges are reversed and
//! negated with `bulge'[i] = −bulge[(n − 2 − i) mod n]`, so every segment keeps
//! its exact arc geometry (midpoint invariance) with the opposite direction.
//! See [`crate::command_id::reverse`] for the full mapping, including the unused
//! trailing entry of an open per-vertex bulge array.
//!
//! ## `WIDTH`
//!
//! The typed width is the entity model's **uniform** polyline width, centred on
//! the vertices (AutoCAD's constant LWPOLYLINE width, DXF group 43): `0` is the
//! hairline/default draw and a positive value is a real wide stroke. `WIDTH`
//! only rewrites that one field, so every selected polyline keeps its id, type
//! key, layer, space, draw order, points, bulges and closed flag.
//!
//! A negative or non-finite value is `pedit.width_invalid` and writes nothing.
//! Per-vertex start/end width **taper** (AutoCAD's `Edit vertex` `WIDTH`
//! sub-option) is a different model and stays a named capability refusal
//! (`pedit.vertex_width_unsupported`); a uniform `WIDTH` never fabricates a
//! taper.
//!
//! ## The write
//!
//! `CLOSE`, `OPEN` and `REVERSE` each stage **one** `update_entity` in **one**
//! transaction / one undo step through the shared publish path. `WIDTH` stages
//! **one** `update_entity` per selected polyline but all in the **same**
//! transaction, so undoing it is one step. The entity keeps its id, type key,
//! layer, space and draw order; only `points`/`bulges`/`closed` change (and, for
//! `WIDTH`, only `width`). A no-op and every refusal open no transaction, so no
//! revision is bumped and no undo entry is recorded.
//!
//! ## NOT IMPLEMENTED (named, not implied)
//!
//! * `Join` — no polyline join in this build (the separate `JOIN` command has
//!   its own LINE/ARC subset).
//! * `Halfwidth` and per-segment start/end **taper** — the entity model stores
//!   one uniform width (`WIDTH` above); the `Edit vertex` `WIDTH` sub-option
//!   stays `pedit.vertex_width_unsupported`, and there is no thickness model
//!   either.
//! * `Fit`, `Spline`, `Decurve` — no fit/spline conversion for polylines.
//!   `FIT` keeps the uniform width; `SPLINE` stores an `AcDbSpline`, which has no
//!   width field, so a later `DECURVE` restores the frame at width `0` (declared
//!   divergence, not a silent width edit).
//! * `Ltype gen` — no linetype-generation flag in the entity model.
//! * `Undo` — the session `UNDO` command is the undo authority; `PEDIT`'s
//!   in-command option stack is not modelled.
//! * `Multiple` — the selection is exactly one polyline and there is no
//!   interactive "Select polyline"/`Multiple` prompt.
//! * `Edit vertex` — no vertex-edit sub-menu (`Next`/`Previous`/`Break`/…).
//! * `Reverse` on non-polylines — the separate `REVERSE` command covers LINE and
//!   SPLINE; `PEDIT` itself is polyline-only.
//! * The convert-a-line/arc prompt (`PEDITACCEPT`) — a non-polyline is
//!   `pedit.not_polyline`, never converted.
//!
//! All of the above are Work-only gaps: the command requires Work mode, so
//! Viewer mode is refused before any input is read.
