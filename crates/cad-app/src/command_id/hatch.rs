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
//! ## Typed pattern name (`HATCH PATTERN` / `HATCH PATTERNFILE`)
//!
//! The `hatchPattern` channel above is a **closed numeric** set, so it cannot
//! carry a name. The command line therefore has a second, **one-shot** spelling
//! that travels on [`crate::CommandPayload::HatchPattern`] instead of the
//! channel:
//!
//! * `HATCH PATTERN <name>` — a **built-in** `.pat` library name (the same table
//!   `HATCHEDIT PATTERN` resolves);
//! * `HATCH PATTERNFILE <path.pat> [<name>]` — a **user** pattern file. The host
//!   read the text (this crate never opens a file), and the resolution is
//!   exactly `HATCHEDIT PATTERNFILE`'s: the file is searched first, the built-in
//!   library second, and without a name the file must hold exactly one pattern.
//!
//! Creation keeps the rest of the session channel: spacing is the pattern scale
//! (`HPSCALE`), `hatchAngle` the hatch-level rotation and `hatchDouble` the
//! perpendicular family, so a built-in name and the same name via the index
//! produce the same rows. The recorded DXF pattern type follows the source: a
//! built-in definition is `Predefined` (`76 = 1`), a definition that came from
//! the user's file is `Custom` (`76 = 2`).
//!
//! Named refusals (nothing written, no transaction):
//! `hatch.needs_pattern_name` (a bare `PATTERN`), `hatch.unknown_pattern`
//! (naming whether the file and/or the built-in table was searched),
//! `hatch.pattern_file_invalid` / `hatch.pattern_file_ambiguous` (the parser's
//! own `.pat` codes), the session-side `hatch.pattern_file_missing` /
//! `_unreadable` / `_encoding` / `_unavailable` (the host could not deliver the
//! file) and `hatch.fill_not_pattern` (a typed `HATCH SOLID` / `HATCH GRADIENT`
//! request: the typed form carries a pattern name only, and a gradient has its
//! own `GRADIENT` command). `-HATCH` accepts the same two forms.
//!
//! ## NOT IMPLEMENTED
//!
//! Associativity, boundary-from-an-internal-point (that is
//! [`crate::CommandId::HatchPoint`]), self-intersecting boundary loops (refused,
//! never trimmed or repaired), a pattern picker dialog, gradient creation from
//! `HATCH` (that is [`crate::CommandId::Gradient`]), a live preview and a
//! per-span dash phase reset. Work-only. Associativity is the one remaining
//! **model** gap: a hatch stores its derived loops only, with no
//! boundary-handle/reactor record to rebuild from.

use crate::PatternFileText;

/// The pattern a typed `HATCH PATTERN` / `HATCH PATTERNFILE` line selected.
///
/// It is the **creation-time** counterpart of
/// [`crate::HatchEditOption::Pattern`]/`PatternFile`: the same `.pat` library,
/// the same by-name resolution and the same source rules, carried on
/// [`crate::CommandPayload::HatchPattern`] because the closed numeric
/// `hatchPattern` channel has no room for a name.
///
/// The file variant carries the text the **host** read (this crate never touches
/// the filesystem), so a request that reaches the command always has content: a
/// read the host could not perform is refused in the session, before any
/// command is dispatched.
#[derive(Debug, Clone, PartialEq)]
pub enum HatchPatternRequest {
    /// A built-in `.pat` library name (matched ASCII case-insensitively).
    Named(String),
    /// A user `.pat` file, searched first; `name` selects one entry and its
    /// absence requires the file to hold exactly one pattern.
    File {
        /// The path as typed (echoed in diagnostics/refusals).
        path: String,
        /// The optional entry name inside the file.
        name: Option<String>,
        /// The file content the host read.
        text: PatternFileText,
    },
}
