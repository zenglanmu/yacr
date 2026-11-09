//! Long-form notes for the three selection-based text commands:
//! [`crate::CommandId::Torient`] (`TORIENT`), [`crate::CommandId::TCase`]
//! (`TCASE`) and [`crate::CommandId::Txt2MText`] (`TXT2MTXT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the three variants keep compact docs and link here. Nothing is
//! implemented in this module — the write paths live in
//! `crate::app_drawing::text_commands`.
//!
//! All three are **Work-only**, **selection-based** commands: the current
//! session selection is the whole input, exactly like `SCALETEXT`/`MATCHPROP`.
//! There is no interactive "Select objects" prompt in this build (the declared
//! divergence); the frontend's selection preconditions are what stop an empty
//! click before IPC.
//!
//! ## `TORIENT` (Express Tools)
//!
//! AutoCAD selects text/mtext/attribute definitions/blocks with attributes and
//! then asks for `New absolute rotation` or `Most Readable`. This build applies
//! both modes to `Text` geometry only (`AcDbText` and `AcDbMText` share that
//! variant; the stored `type_key` distinguishes them):
//!
//! * **bare** `TORIENT` is Most Readable — each rotation is folded into
//!   `(-π/2, π/2]` by adding/subtracting π (180° steps), which is the "keep
//!   upright" rule;
//! * `TORIENT <angle-degrees>` sets that **absolute** rotation (finite decimal
//!   degrees, stored as radians);
//! * `TORIENT REVERSE` / `TORIENT R` adds exactly 180° to each rotation
//!   (`CommandPayload::TorientReverse`), which flips the direction without
//!   changing the text's position or content.
//!
//! Only `rotation` changes: position, height, style, font, alignment, layer,
//! space and draw order are preserved. A selected non-Text entity — including a
//! MULTILEADER (whose text rotation has no setter in this build) and an INSERT
//! (attributes are not modelled) — is reported as a `torient.not_reset`
//! diagnostic and does **not** fail the command when at least one Text changed.
//! If no Text changed the command reports `torient.unchanged` and opens **no**
//! transaction. One transaction / one undo step covers every changed Text.
//!
//! ## `TCASE` (Express Tools)
//!
//! AutoCAD's `-TCASE` asks
//! `Select case [Sentence/Lower/Upper/Title/toGgle] <TITLE>`. The bare command
//! applies **Title** (the documented current default); the typed
//! `TCASE <mode>` form accepts `UPPER|LOWER|TITLE|SENTENCE|TOGGLE` and the
//! single letters `U|L|T|S|G` (case-insensitive). An argument outside that
//! grammar is the stable `invalid_tcase_argument_reason`.
//!
//! Case conversion is one pure transformer
//! (`crate::app_drawing::text_commands::apply_text_case`) that copies MTEXT
//! formatting escapes (`\P`, `\f…;`, `\H…;`, `\C…;`, `{}` grouping, `\~`,
//! `\\`) verbatim and changes only visible characters. It applies to
//! `AcDbText`/`AcDbMText` and MULTILEADER text (through
//! `MultiLeaderGeometry::with_text`). Attributes and dimension text are not
//! modelled, and every other selected kind is a `tcase.not_reset` diagnostic;
//! no selection is partially claimed. One transaction / one undo step; no
//! change is `tcase.unchanged` with no transaction.
//!
//! ## `TXT2MTXT`
//!
//! AutoCAD combines selected TEXT/MTEXT objects into MTEXT with a settings
//! dialog. This build implements the four documented toggles as a typed payload
//! ([`crate::command_payload::Txt2MTextOptions`]); the bare command uses the
//! defaults:
//!
//! * **Combine into a single mtext object** (default on): raw strings are joined
//!   with `\P`; the first source in the chosen order supplies style/font/height/
//!   rotation/alignment/position/layer/draw order. Per-source font/size
//!   differences cannot survive one object (declared).
//! * **Combine off**: each source becomes its own new `AcDbMText` at its own
//!   position/style, and every source is still deleted in the same transaction
//!   (the declared one-per-source rule).
//! * **Text ordering**: `TopDown` sorts descending Y then ascending X, stably;
//!   `Selection` keeps selection order.
//! * **Word-wrap**: uses the existing `wrap_width`; off stores `None`, on
//!   preserves each source's boundary (including `None` = no wrap).
//! * **Force uniform line spacing**: AutoCAD's MTEXT line-spacing factor has no
//!   field in this build (`\P` is only a paragraph break), so the setting is a
//!   named `txt2mtxt.uniform_line_spacing` `Unsupported` refusal; nothing is
//!   half-applied.
//!
//! Every source is deleted and the replacement(s) inserted in **one**
//! transaction / undo step. A non-text source is `txt2mtxt.unsupported` and a
//! block-instance member/sub-element is `txt2mtxt.not_addressable`; both refuse
//! the whole command, never a partial conversion.
