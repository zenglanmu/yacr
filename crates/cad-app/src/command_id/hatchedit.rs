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
//! The two boundary options are the exception to "one or more hatches": their
//! selection is exactly one hatch plus the closed entities that supply or remove
//! loops (see below).
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
//! | `PATTERN <name>` | replace the fill with that built-in `.pat` pattern (pattern fill only) |
//! | `PATTERNFILE <path.pat> [<name>]` | the same from a user `.pat` file the host read (pattern fill only) |
//! | `PATTERNTYPE USER/PREDEFINED/CUSTOM` | set DXF group 76 |
//! | `SOLID` / `GRADIENT` | replace the fill with the solid / default linear ramp |
//! | `COLOR <aci>` / `COLOR BYLAYER` | set the entity display colour |
//! | `TRANSPARENCY <0..=90>` / `TRANSPARENCY BYLAYER` | set the entity opacity |
//! | `LAYER <name>` | move every target to that drawing layer |
//! | `ORIGIN <x> <y>` | move the pattern origin (pattern fill only) |
//! | `DRAWORDER BEHIND/FRONT/BOUNDARY` | model-space draw order (`BOUNDARY` refused) |
//! | `RECREATEBOUNDARY` | one closed polyline per stored loop |
//! | `SEPARATE` | split a multi-loop hatch into one hatch per loop |
//! | `ADDBOUNDARIES` | append the other selected closed boundaries as loops |
//! | `REMOVEBOUNDARIES` | remove stored loops matching the other selected boundaries |
//!
//! A bare `HATCHEDIT` is admitted by name and refused by the application with
//! `hatchedit.needs_option`. An argument outside the grammar is refused before
//! IPC with `invalid_hatchedit_argument_reason`.
//!
//! ## Named refusals (global scope)
//!
//! * `ASSOCIATE` / `DISASSOCIATE` — `hatchedit.no_association`: this build has
//!   no hatch/boundary association record;
//! * `ANNOTATIVE` — `hatchedit.annotative_unsupported`: no annotative-scale
//!   model.
//!
//! `ORIGIN` is implemented as a **pattern phase displacement**: neither the DXF
//! nor the DWG hatch record has a dedicated pattern-origin group — both store
//! the phase in each definition row's base point (groups 43/44) — so the domain
//! ([`cad_domain::HatchGeometry::pattern_origin`]) keeps the displacement from
//! those rows and the export folds it back into them. `ORIGIN 0 0` is therefore
//! the source file's own origin. A Solid/Gradient fill is the per-target
//! `hatchedit.not_pattern` refusal (no definition rows exist to move).
//!
//! `PATTERN <name>` has three named refusals of its own, all writing nothing:
//! an empty name is `hatchedit.needs_pattern_name`, a name the built-in `.pat`
//! library does not hold is `hatchedit.unknown_pattern` (the message lists every
//! accepted name), and a Solid/Gradient fill is
//! `hatchedit.pattern_requires_pattern` — a solid is never silently converted
//! into a pattern. The two name refusals are also answered **before IPC** by
//! `cad-ui-core` (the grammar validates the name against
//! `cad_domain::hatch::predefined_patterns`, the one library table) and before
//! the selection is read; the fill refusal is per target, after it is resolved.
//!
//! ## `PATTERNFILE` (a user `.pat` library)
//!
//! `PATTERNFILE <path.pat> [<name>]` applies a pattern from a user file. The
//! **host** reads the file, not this command: `cad-ui-core` asks
//! `cad_host::HostResources::read_named_file` (native hosts read through
//! `cad_platform::read_named_file`; a browser inherits the trait's
//! `Unavailable` default) and the text travels in the payload. The command parses
//! it with the same `PatternLibrary` the built-in table and the HATCH creation
//! path use, keeps the hatch's stored angle/scale/`double`, and writes group 76
//! as `Custom` (`2`).
//!
//! Refusals, none of which writes: the path is missing/unreadable
//! (`hatchedit.pattern_file_missing`/`_unreadable`), the runtime cannot read
//! named files (`hatchedit.pattern_file_unavailable`, with the stable
//! `unsupported-runtime: pattern-file` marker), the bytes are not UTF-8
//! (`hatchedit.pattern_file_encoding`), the payload arrived without the host read
//! (`hatchedit.pattern_file_not_read`), the file does not parse or holds no
//! definition (`hatchedit.pattern_file_invalid`, carrying the parser's own code
//! in the detail), it holds several patterns and none was named
//! (`hatchedit.pattern_file_ambiguous`), or the named pattern is in neither the
//! file nor the built-in table
//! (`hatchedit.unknown_pattern`, naming both sources). Search order is file first,
//! built-in second.
//!
//! The global refusals above are answered before the selection is read, and
//! none of them opens a transaction.
//!
//! ## Boundary edits (pre-selection model)
//!
//! `ADDBOUNDARIES` / `REMOVEBOUNDARIES` do not open an interactive pick prompt:
//! the selection is the input. It must hold exactly **one** hatch (the target)
//! plus one or more closed boundary entities (`Polyline`, `Circle` or a full
//! `Ellipse`) on that hatch's plane. Each non-hatch selection is converted with
//! the same OCS projection as `HATCHSETBOUNDARY`, so a non-coplanar or
//! unsupported shape is refused by that code. `ADDBOUNDARIES` appends the
//! converted loops (a set that adds nothing is `hatchedit.add_boundaries_invalid`);
//! `REMOVEBOUNDARIES` removes stored loops matching a converted loop by
//! **points + bulges** (`1e-9` tolerance, flags/source edges ignored) and refuses
//! an unmatched set (`hatchedit.remove_no_match`) or removing the last loop
//! (`hatchedit.remove_last_loop`). Exactly one hatch is required:
//! `hatchedit.multiple_hatches` / `hatchedit.selection`; a missing boundary
//! source is `hatchedit.no_boundary_source`.
//!
//! The rebuilt loop set goes through `HatchGeometry::new` (creation-path
//! validation), so duplicate/crossing rings are `hatchedit.add_boundaries_invalid`
//! / `hatchedit.remove_boundaries_invalid`. The hatch plane, style, fill, seeds,
//! pixel size, pattern type and pattern origin are preserved, and the edit is
//! one transaction / one undo step.
//!
//! ## Rebuild
//!
//! ANGLE/SCALE/ORIGIN/PATTERN/PATTERNTYPE/STYLE and SOLID/GRADIENT keep the
//! stored loops, plane, seeds, pixel size and every property the option does not
//! name, and rebuild the display through the domain constructor
//! (`HatchGeometry::from_import` / `with_style_and_fill` / `with_pattern_origin`)
//! — never by hand-editing the display. `PATTERN <name>` / `PATTERNFILE …` take
//! the name and definition rows from the resolved library entry while keeping the
//! hatch's stored pattern angle (radians, verbatim), scale and `HPDOUBLE`, and
//! record the fill as `Predefined` (`76=1`, the classification the HATCH creation
//! path writes for a built-in name) or `Custom` (`76=2`, a user-file pattern);
//! `PATTERNTYPE` can still override the stored group 76 afterwards.
//! `RECREATEBOUNDARY` writes non-associative closed polylines on
//! the hatch's layer/space; `SEPARATE` keeps the first loop in the original
//! entity and inserts one hatch per remaining loop (nested-island semantics are
//! not preserved — declared). Colour/transparency are entity display attributes;
//! layer and draw order are entity row fields.
//!
//! Every changed target (and every created entity) is written in **one
//! transaction / one undo step**, so one undo restores all of them. A run that
//! changes nothing is the named `hatchedit.unchanged` outcome with **no**
//! transaction, revision bump or undo entry.
//!
//! ## NOT IMPLEMENTED
//!
//! The interactive dialog, association/disassociation, annotative scaling, draw
//! order relative to a boundary (the hatch stores no boundary entity ids, and
//! modelling them is the associativity/reactor model above) and a per-loop
//! pattern origin.
//! Work-only (Viewer mode refuses it at the command layer).
