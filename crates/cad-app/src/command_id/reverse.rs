//! Long-form notes for [`crate::CommandId::Reverse`] (`REVERSE`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. The
//! implementation lives in `app_drawing::modify_extra`.
//!
//! ## Input
//!
//! `REVERSE` is **selection-based**: payload is [`crate::CommandPayload::None`]
//! and the current session selection is the whole input. An empty selection is
//! an explicit `reverse.selection` refusal; a supplied payload is
//! `reverse.payload`. This is the declared divergence from AutoCAD's interactive
//! "Select objects" prompt — this build's fixed-step capture machine cannot
//! express it.
//!
//! Every selected reference must be a **directly addressable model-space**
//! entity (`reverse.not_addressable` otherwise: another document, a block
//! instance member, a sub-element, paper space).
//!
//! ## Supported geometry (exact)
//!
//! `Line { start, end }` — swap the endpoints.
//!
//! `Polyline { points, bulges, closed }` — reverse the point order and reverse
//! and negate the bulges with the segment mapping
//! `bulge'[i] = −bulge[(n − 2 − i) mod n]` (open and closed; both the
//! one-bulge-per-vertex and the one-bulge-per-segment storage shapes). The
//! unused trailing entry of an open per-vertex bulge array describes no
//! segment and is carried through unchanged.
//!
//! `Spline { degree, knots, control_points, weights }` — reverse the control
//! points and weights; the reversed knot vector is
//! `v[i] = u[last] − u[last − i]`, which keeps a clamped/uniform curve's shape
//! (`new(t) == old(1 − t)` for normalised parameters).
//!
//! Every other geometry kind (`Arc`, `Circle`, `Text`, `Insert`, `Dimension`,
//! ellipse, hatch, …) is an explicit [`cad_domain::CadError::Unsupported`]
//! `reverse.unsupported` refusal. The **whole command** then changes nothing:
//! this is not a per-entity partial success.
//!
//! ## The write
//!
//! All reversed entities are staged in **one transaction / one undo step** and
//! published together. There is no canonical direction, so even an entity that
//! already appears reversed is written again (the command always stages the
//! changed rows). The success diagnostic is `reverse.applied` with the count.
//! Work-only.
