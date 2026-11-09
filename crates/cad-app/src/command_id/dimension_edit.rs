//! Long-form contracts for the 2026-11-07 dimension-editing batch:
//! `DIMSTYLE`/`-DIMSTYLE`, `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`, `DIMREGEN`,
//! `DIMDISASSOCIATE`, `QDIM`, `DIMJOGLINE` and `DIMBREAK`.
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit. The implementation lives in [`crate::app_drawing::dimension_edit`];
//! this module is the contract, including the exact domain boundaries.
//!
//! # `DIMSTYLE` / `-DIMSTYLE`
//!
//! Payload is [`crate::DimensionBatchPayload::Style`] with three typed forms:
//!
//! | argument | behaviour |
//! | --- | --- |
//! | `DIMSTYLE LIST` | one `dimstyle.entry` diagnostic per stored DIMSTYLE row (read-only) |
//! | `DIMSTYLE CURRENT` | report the style dimension creation resolves (`Standard`, ADR 0017 B.3) |
//! | `DIMSTYLE SET <name>` | named refusal: `cad-db` has **no post-import dim-style write API** |
//!
//! `-DIMSTYLE` is an alias in
//! [`cad_ui_core::session::COMMAND_ALIASES`](https://docs.rs/cad-ui-core), not a
//! second `CommandId`: it normalizes onto the same selector and therefore the
//! same path. Bare `DIMSTYLE` (no argument) is a named
//! `dimstyle.needs_option` refusal, never a defaulted `LIST`.
//!
//! `LIST`/`CURRENT` write nothing (no revision bump, no history step), so Viewer
//! mode admits them. `SET` is refused before any write with
//! `dimstyle.set_unavailable`: the live [`cad_db::DrawingDatabase`] exposes
//! read-only `dim_styles()`/`dim_style_by_name()` accessors, and only
//! `DrawingDatabaseBuilder::insert_dim_style` can add a row. A requested name
//! that does not exist is the distinct `dimstyle.unknown` refusal. No command in
//! this module edits the DIMSTYLE table; a style row is never invented.
//!
//! # `DIMTEDIT`
//!
//! Payload is [`crate::DimensionBatchPayload::TextEdit`], applied to **exactly
//! one** directly addressable selected `DIMENSION` (model or paper space; the
//! one-entity selection is the input). The documented AutoCAD 2026 option set is
//! `Left`, `Right`, `Center`, `Home`, `Angle` plus the free text location;
//! **there is no `Rotate` option** (that is `DIMEDIT`'s Rotate).
//!
//! * `DIMTEDIT MOVE <x> <y>` sets group 11 (`DimensionDefinition::text_middle`)
//!   to the typed drawing-plane X/Y. The Z is the dimension's own definition
//!   plane Z, never typed, so the text stays in plane.
//! * `DIMTEDIT LEFT|RIGHT|CENTER` justifies the text **along the dimension
//!   line**, only for linear/aligned, radius and diameter dimensions. The domain
//!   stores a centered text anchor (group 11) and no per-entity justification
//!   field, so this build's declared rule is: the anchor moves to the
//!   start (minimum projection), end (maximum projection) or midpoint of the
//!   dimension line. For a linear/aligned dimension those are the two
//!   extension-line feet; for radius/diameter the centre-side and arc-side ends
//!   of the radial leader. Angular and ordinate dimensions are refused by name
//!   (`dimtedit.justify_unsupported`, naming the kind).
//! * `DIMTEDIT ANGLE <degrees>` sets group 53
//!   (`DimensionDefinition::text_rotation`) to the typed angle in radians about
//!   the UCS X axis; the text centre does not move and `0` restores the derived
//!   orientation (group 53's `0` is the derive sentinel).
//! * `DIMTEDIT HOME` restores group 11 to the derive sentinel; the rotation is
//!   untouched.
//!
//! Each real change rebuilds the dimension through the shared
//! [`cad_domain::DimensionGeometry::synthesized`] from the mutated definition,
//! the stored style snapshot and the stored text override, and is written with
//! one `update_entity` in **one transaction / one undo step**. A command that
//! changes nothing opens no transaction and reports `dimtedit.unchanged`.
//!
//! Refusals (nothing written): `dimtedit.payload`, `dimtedit.selection` (not
//! exactly one), `dimtedit.duplicate`, `dimtedit.not_addressable` (block member
//! or sub-element), `dimtedit.target_missing`, `dimtedit.not_dimension`,
//! `dimtedit.bad_position`, `dimtedit.bad_rotation`, `dimtedit.degenerate`,
//! `dimtedit.justify_unsupported`.
//!
//! **NOT IMPLEMENTED**: glyph-level justification (the centered anchor is the
//! only text alignment the domain stores), the oblique dimension-line edit and a
//! `RESET` spelling (use `HOME` for position and `ANGLE 0` for orientation).
//!
//! ## Declared display-source consequence
//!
//! `cad-domain` keeps the display provenance private and no constructor rebuilds
//! the imported anonymous-block picture branch from a mutated definition
//! (the same declared limitation as the dimension grip drag). Rebuilding a
//! dimension here therefore re-synthesizes its display; an imported picture
//! dimension becomes a synthesized one after a successful edit. No measurement
//! is invented: the stored definition still drives the synthesis.
//!
//! # `DIMEDIT`
//!
//! Payload is [`crate::DimensionBatchPayload::Edit`], applied to every selected
//! directly addressable dimension:
//!
//! * `DIMEDIT TEXT <override>` stores the override string verbatim (group 1,
//!   [`cad_domain::DimensionGeometry::text_override`]); `<>` is the measurement
//!   placeholder. The string must be non-empty and at most
//!   [`crate::MAX_TOOL_TEXT_CHARS`] characters (the TEXT channel's bound).
//! * `DIMEDIT RESET` drops the override, so the formatted measurement renders
//!   again.
//!
//! The same rebuild/one-transaction discipline as `DIMTEDIT` applies; no change
//! is `dimedit.unchanged`. Refusals: `dimedit.payload`, `dimedit.selection`,
//! `dimedit.duplicate`, `dimedit.not_addressable`, `dimedit.target_missing`,
//! `dimedit.not_dimension`, `dimedit.bad_text`, `dimedit.degenerate`.
//!
//! **NOT IMPLEMENTED**: the AutoCAD `Home`/`New`/`Rotate`/`Oblique` extension-line
//! edits; only the text override is modelled on the entity.
//!
//! # `DIMOVERRIDE`
//!
//! Payload is [`crate::DimensionBatchPayload::Override`] (`variable`, `value`),
//! applied to every selected dimension's **entity style snapshot**
//! ([`cad_domain::DimensionStyleValues`]) — never to the DIMSTYLE table. The
//! closed set is exactly the snapshot fields the synthesis consumes:
//!
//! `DIMTXT`, `DIMASZ`, `DIMEXO`, `DIMEXE`, `DIMGAP`, `DIMCEN`, `DIMDLI`,
//! `DIMTAD`, `DIMTVP`, `DIMLFAC`, `DIMDEC`, `DIMRND`, `DIMPOST`, `DIMZIN`,
//! `DIMSE1`, `DIMSE2`, `DIMSD1`, `DIMSD2`, `DIMDSEP`, `DIMLUNIT`.
//!
//! Values are validated, never clamped. `DIMZIN` admits only bit fields whose
//! feet/inch bits (1/2) are clear (`0`, `4`, `8`, `12`): those bits are declared
//! unimplemented by the formatter. `DIMLUNIT` admits only `2` (decimal); other
//! formats are a declared degradation, so they are refused instead of stored.
//! Variables the `DimStyle` table has but the entity snapshot cannot carry
//! (`DIMCLRD`/`DIMCLRE`/`DIMCLRT`, `DIMTXSTY`, `DIMJUST`, `DIMATFIT`, …) are
//! `dimoverride.unsupported_variable`; an unrecognised name is
//! `dimoverride.unknown_variable`; a malformed value is
//! `dimoverride.bad_value`. All refusals happen before any write.
//!
//! Every changed dimension is rebuilt with the same definition/text override and
//! the mutated snapshot, then written in **one transaction / one undo step**. An
//! all-no-op run reports `dimoverride.unchanged` and opens none.
//!
//! **NOT IMPLEMENTED**: a DIMSTYLE **table** override, per-dimension override
//! records (`ACAD_DIMASSOC` does not exist here) and the variables listed above.
//!
//! # `DIMREGEN`
//!
//! Payload-free. This build creates only **non-associative** dimensions, so there
//! is no association to regenerate. The command changes nothing and reports
//! `dimregen.non_associative`; it never claims an update, bumps no revision and
//! records no history. A supplied payload is `dimregen.payload`. Viewer-legal.
//!
//! # `DIMDISASSOCIATE`
//!
//! Payload-free. Dimensions are already non-associative, so there is nothing to
//! remove. The command changes nothing and reports
//! `dimdisassociate.already_non_associative`; a supplied payload is
//! `dimdisassociate.payload`. Viewer-legal.
//!
//! # `QDIM`
//!
//! Payload is [`crate::DimensionBatchPayload::QDim`]
//! ([`crate::QDimCommand`]); the current selection is the rest of the input. A
//! bare `QDIM` defaults to **Continuous**. The nine documented AutoCAD 2026 modes
//! map onto the payload as follows:
//!
//! | documented mode | payload |
//! | --- | --- |
//! | Continuous (default) | [`crate::QDimMode::Continuous`] |
//! | Staggered | [`crate::QDimMode::Staggered`] |
//! | Baseline | [`crate::QDimMode::Baseline`] |
//! | Ordinate | [`crate::QDimMode::Ordinate`] |
//! | Radius | [`crate::QDimMode::Radius`] |
//! | Diameter | [`crate::QDimMode::Diameter`] |
//! | Datum Point | `QDimCommand::datum` (common baseline origin / ordinate datum) |
//! | Edit | `QDimCommand::remove` (0-based origin/target indices) |
//! | Settings | `QDimCommand::snap` (`Endpoints`/`Intersections`) |
//!
//! Angular is deliberately absent: AutoCAD 2026 does not document an angular
//! QDIM mode. Existing dimensions may be selected; a linear/aligned dimension
//! contributes its two extension origins, an ordinate dimension its feature
//! point and a radius/diameter dimension its arc point. Radial modes consume
//! selected `CIRCLE`s and `ARC`s instead.
//!
//! Linear modes order the origins by projection on a chain direction (the first
//! selected LINE's direction, else the first linear dimension's rotation, else
//! the extreme origin pair): **Continuous** puts consecutive origins on one
//! dimension line at `+DIMDLI`; **Staggered** steps `i · increment` (the
//! increment is `DIMDLI` unless `INCREMENT` supplies one — a declared reading,
//! since AutoCAD 2026 does not document the source); **Baseline** measures from
//! the datum (or the first origin) with rows stacked by `DIMDLI` (an origin with
//! the datum's own projection measures zero and is skipped); **Ordinate**
//! measures the dominant axis of `origin − datum` with the leader on the chain
//! normal. The default ordinate datum is the drawing origin `(0, 0, 0)`.
//!
//! **Per-item skip** is the declared policy for unsupported selected entities:
//! each one produces the stable `qdim.skipped` diagnostic (object + reason) and
//! the rest of the selection is still dimensioned. A selection with no usable
//! target is the named `qdim.no_targets` refusal. Whole-run refusals (nothing
//! written): `qdim.payload`, `qdim.selection`, `qdim.duplicate`,
//! `qdim.not_addressable`, `qdim.target_missing`, `qdim.space`, `qdim.no_style`,
//! `qdim.bad_spacing`, `qdim.bad_increment`, `qdim.off_plane`, `qdim.degenerate`,
//! `qdim.no_intersections`, `qdim.edit_range`, `qdim.no_targets`.
//!
//! All created dimensions land in **one transaction / one undo step** on the
//! active layer in model space. They are ordinary non-associative dimensions
//! synthesized by the shared builder, so the measurement is the true projected
//! distance and no second geometry path exists.
//!
//! **NOT IMPLEMENTED**: AutoCAD's interactive location pick and option loop
//! (the location is style-driven), a persistent QDIM session state (Datum/Edit/
//! Settings are per-invocation payload fields), angular QDIM and point-only
//! input.
//!
//! # `DIMJOGLINE`
//!
//! Payload is [`crate::DimensionBatchPayload::JogLine`] (`ADD <x> <y>` or
//! `REMOVE`), parsed so the typed form is modelled, but the command is an
//! unconditional named refusal: `cad_domain::DimensionDefinition` has **no jog
//! field** and [`cad_domain::DimensionGeometry`] cannot store a jog. The refusal
//! is `dimjogline.unsupported` and names that missing field; nothing is read from
//! the selection, nothing is written, and no history entry is opened. The bare
//! selector is `dimjogline.needs_option`. Viewer-legal (it can never write).
//!
//! # `DIMBREAK`
//!
//! Payload-free (the selection is the input). Unconditional named refusal:
//! `cad_domain::DimensionGeometry` stores **no dimension-line break** — the
//! display is a synthesized or imported picture and there is no break record.
//! The refusal is `dimbreak.unsupported`; a supplied payload is
//! `dimbreak.payload`. The command never deletes, trims or shortens the
//! dimension entity. Viewer-legal (it can never write).
