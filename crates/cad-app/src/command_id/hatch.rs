//! Long-form notes for [`crate::CommandId::Hatch`] (`HATCH`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here.
//!
//! ## Input
//!
//! HATCH fills the **pre-selected** boundary entities (ADR 0018 decision D,
//! P6b). Payload is `Hatch { refs }` — the selection snapshot taken when the tool
//! started; the tool itself captures **no points** (its arity is the zero-point
//! exact complement, like ARRAYRECT) and takes no in-command picks.
//!
//! ## Boundary subset
//!
//! Each accepted selected entity becomes exactly one loop in the hatch plane's
//! OCS 2D frame: a `Circle` (an exact four-quarter-arc bulged loop), a full
//! `Ellipse` (sampled chords — declared, a true ellipse is not a circular arc) or
//! a **closed** `Polyline` (its own points and bulges, including a 3D entity that
//! lies in one plane).
//!
//! Selected `Line`/`Arc` entities (in any mix with the kinds above) are
//! **stitched** into one loop per closed chain: endpoints are matched at the
//! absolute `1e-6` world-unit tolerance, an edge may be consumed in either
//! direction, and every `Arc` stays an **exact bulge segment**
//! (`bulge = tan(sweep / 4)` — never sampled). A node where more than two edges
//! meet is refused as `hatch.boundary_branch` (no pairing is guessed), edges left
//! outside every loop as `hatch.boundary_open` (naming how many), and a
//! zero-length `Line` / zero-sweep `Arc` as `hatch.boundary_degenerate`. An open
//! polyline, spline, text, block, dimension or any other kind is an explicit
//! refusal that opens no transaction; so is a selection that is not coplanar with
//! a **single consistent normal**.
//!
//! ## Parameters
//!
//! The fill, spacing, angle (degrees on the channel), double flag and style are
//! the session typed channel
//! (`crate::draw_tool::ToolParameter::HatchPatternType` … `HatchStyle`), read
//! here so the session stays the single authority. A user-defined pattern is
//! synthesized from angle/spacing/double exactly like the import path (one
//! definition row at offset `(0, 1)` with the spacing as the pattern scale) and
//! recorded as `cad_domain::HatchPatternType::UserDefined`; a solid fill is
//! recorded as `Predefined` (`76=1`, matching the committed SOLID fixtures).
//! Pattern type code `2` (predefined) takes the `hatchPattern` **1-based** index
//! into the implemented `ANSI31`/`ANSI32`/`ANSI37` table, records `Predefined`
//! (`76=1`) and refuses an index outside that table by name.
//!
//! ## Commit
//!
//! The display is derived by the unique constructor `HatchGeometry::new`, which
//! rejects the ADR A.3 creation-path defects (self-intersecting,
//! touching/shared vertex, duplicate, partially overlapping, unclosed,
//! degenerate, non-finite) as a **whole-command** refusal. One entity
//! (`type_key`/`class_name` = `AcDbHatch`) is inserted through the shared create
//! funnel in one transaction / one undo step on the active layer.
//!
//! ## NOT IMPLEMENTED
//!
//! Associativity, boundary-from-an-internal-point (that is
//! [`crate::CommandId::HatchPoint`]), self-intersecting boundary loops (refused,
//! never trimmed or repaired), custom (`76=2`) pattern names and `.pat` loading,
//! a pattern picker dialog, gradient creation, a live preview and a per-span dash
//! phase reset. Work-only.
