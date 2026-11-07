//! Long-form contract for [`crate::CommandId::MeasureGeom`] (`MEASUREGEOM`,
//! AutoCAD 2026 inquiry router, 2026-11-07).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. Nothing here starts a
//! tool: the typed router lives in `cad_ui_core::session::measuregeom`, and the
//! report-only `RADIUS` half lives in `crate::app_measure_object`.
//!
//! ## The one router, no second measurement engine
//!
//! AutoCAD's `MEASUREGEOM` prompts for an option and then runs one of the
//! measurement tools. This build already has those tools, so `MEASUREGEOM` is a
//! **selector router** over them; it never adds a second algorithm:
//!
//! | typed line | routed to | result |
//! | --- | --- | --- |
//! | `MEASUREGEOM DISTANCE` | `DIST` | `BeginMeasurement(MeasurementToolKind::Distance)` |
//! | `MEASUREGEOM ANGLE` | `MEASUREANGLE` | `BeginMeasurement(Angle)` |
//! | `MEASUREGEOM AREA` | `AREA` | `BeginMeasurement(Area)` |
//! | `MEASUREGEOM POLYLINE` | `MEASUREPOLYLINE` | `BeginMeasurement(PolylineLength)` |
//! | `MEASUREGEOM RADIUS` | this command, `Radius` payload | a `measuregeom.radius` diagnostic |
//! | `MEASUREGEOM VOLUME` | this command, `Volume` payload | explicit `Unsupported` |
//!
//! The first four start *the same* interaction request and selector the base
//! command uses (`DIST`/`MEASUREANGLE`/`AREA`/`MEASUREPOLYLINE`), so the capture,
//! preview, commit and `cad_command_log` entry are the existing engine's, never a
//! parallel implementation.
//!
//! ## Typed argument grammar
//!
//! `MEASUREGEOM <option>`, case-insensitive, surrounding whitespace trimmed. The
//! **full words only** are admitted: `DISTANCE`, `ANGLE`, `AREA`, `POLYLINE`,
//! `RADIUS`, `VOLUME`. No abbreviation is admitted (there is no `D`/`A`/`R`
//! short form), because an abbreviation would have to guess which option the
//! typist meant and the command line must not guess. The selector itself is
//! canonical and the option travels in the same typed line, exactly like
//! `ZOOM WINDOW`; it is **not** a new selector and not a second IPC.
//!
//! A bare `MEASUREGEOM` is the named `measuregeom.needs_option` refusal that
//! lists all six implemented options. An unknown word is
//! `invalid_measuregeom_argument_reason`, which echoes the whole typed line and
//! names the same six options. Neither is a silent `DISTANCE`.
//!
//! ## `RADIUS` (read-only)
//!
//! The current selection must be **exactly one** `Circle` or `Arc` (payload
//! [`crate::MeasureGeomOption::Radius`]). It reports the centre,
//! radius, diameter and circumference as a `measuregeom.radius` diagnostic. For
//! an `Arc` the diagnostic also carries the arc's **own length** (a full-circle
//! circumference would otherwise hide the fact that the object is a partial
//! curve). Refusals: `measuregeom.radius.selection_empty`,
//! `.selection_multiple`, `.not_addressable`, `.target_missing`,
//! `.not_arc_circle`, `.degenerate`.
//!
//! No transaction, `ChangeSet`, history entry or revision bump: `RADIUS` is a
//! report, exactly like `AREA OBJECT`. Viewer and Work both allow it.
//!
//! ## `VOLUME` and everything else not implemented
//!
//! `MEASUREGEOM VOLUME` is [`crate::CadError::Unsupported`] with the stable
//! `measuregeom.volume_unsupported` code: this build has no 3D-solid/volume
//! model, so no number is fabricated. AutoCAD's further `MEASUREGEOM` options
//! (options from the full product outside the six named above, dynamic
//! measurement, multi-point sequences beyond the existing tools) are not
//! implemented and are not admitted by the grammar; the router never maps an
//! unknown word onto a guess.
