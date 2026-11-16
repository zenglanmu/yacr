//! Long-form notes for the `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` family, plus the
//! **one** array bound the typed command line reuses.
//!
//! ## One implementation, three selectors
//!
//! `ARRAY`, `-ARRAY` and `ARRAYCLASSIC` are **routers**, not a second array
//! planner (`ExtendedCommandId::Array` / `DashArray` / `ArrayClassic` all reach
//! the same `array_router` in the application): a
//! [`crate::CommandPayload::ArrayRect`] payload becomes the existing `ARRAYRECT`
//! plan and an [`crate::CommandPayload::ArrayPolar`] payload the existing
//! `ARRAYPOLAR` plan, so the copies, the placement rule, the cap and the single
//! transaction / undo step are literally the same code. With no payload the
//! router reports `array.needs_payload` and writes nothing.
//!
//! AutoCAD's `ARRAYCLASSIC` dialog (the pre-2012 tabbed grid array) is **NOT
//! IMPLEMENTED** — this build has no dialog surface at all — but the *command*
//! is the same rectangular/polar planner, so it shares the typed line instead of
//! growing a second one. `ARRAYPATH` is a separate selector with its own
//! `ARRAYPATH <count> [ALIGN]` grammar.
//!
//! ## Typed line (this build's spelling)
//!
//! ```text
//! ARRAY RECT  <rows> <columns> <row-spacing> <column-spacing>
//! ARRAY POLAR <items> <fill-angle> <center-x> <center-y>
//! ```
//!
//! `-ARRAY` and `ARRAYCLASSIC` accept the same two forms. The numbers are the
//! values the planners already read from the session tool-parameter channel
//! (`arrayRows`/`arrayColumns`/`arrayRowSpacing`/`arrayColumnSpacing` and
//! `arrayItems`/`arrayFillAngle`), so the typed line **writes that one channel**
//! and then dispatches the ordinary `ArrayRect`/`ArrayPolar` payload: the
//! palette and the typed line cannot drift into two value stores. `rows`,
//! `columns` and `items` count the **source object** (a 1×1/1-item array inserts
//! nothing and reports it), spacing is the distance between base points (a
//! negative value arrays in the negative direction) and `fill-angle` is in
//! degrees with AutoCAD's `0 < |angle| <= 360` rule.
//!
//! Named refusals — all before any session write and nothing dispatched:
//! `array.count` (a non-integer or `< 1` count, which is why zero/negative
//! counts never reach the planner), `array.spacing` (a zero spacing would put
//! every cell on top of the source: a degenerate array, refused instead of
//! stored), `array.angle` (`0` or `|angle| > 360`), `array.center` (a non-finite
//! centre) and `array.cap` (the requested array would exceed
//! [`MAX_ARRAY_ENTITIES`] for one source; the planner re-checks the total against
//! the same constant). A malformed line is the stable
//! `unsupported CAD command: …` refusal.
//!
//! Every grammar refusal above happens **before** the channel write, so a
//! malformed/over-bound line leaves the session untouched. A **planner-level**
//! refusal (an untransformable source kind, the total multi-source cap, a locked
//! layer) happens after it: the channel keeps the values — they are valid session
//! settings the palette shows — while the drawing, the revision and the undo
//! history are untouched.
//!
//! Associativity, `Levels` and a rotated grid (`ANGLE`) remain outside the
//! model: the domain/store carries independent copies only.

/// The total array-copy bound (`MAXARRAY` order of magnitude), re-exported so
/// the typed line refuses an over-bound request with the **same** constant the
/// planner enforces instead of a second transcribed number.
pub use crate::app_drawing::MAX_ARRAY_ENTITIES;
