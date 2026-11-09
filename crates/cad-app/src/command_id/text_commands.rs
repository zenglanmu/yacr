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
//! dialog (single mtext, sort top-down, word-wrap, force uniform line spacing).
//! **v1 declares that dialog NOT IMPLEMENTED** and reports
//! `txt2mtxt.settings_dialog` / `txt2mtxt.uniform_line_spacing`; nothing is
//! silently applied.
//!
//! The command requires at least one directly addressable model-space `Text`
//! entity. Every source must be a Text/MText; a non-text kind is
//! `txt2mtxt.unsupported` and a block-instance member/sub-element is
//! `txt2mtxt.not_addressable` — both refuse the **whole** command, never a
//! partial conversion. Sources are sorted top-to-bottom (descending Y) then
//! left-to-right (ascending X), stably, and their **raw** strings are joined
//! with `\P` without re-processing escapes. One new `AcDbMText` entity is
//! created at the first sorted source's position, carrying its style/font/
//! height/rotation/h_align/v_align, layer, space, draw order and `wrap_width`
//! (`None` stays `None`; no width is invented). All sources are deleted in the
//! **same** transaction / undo step, so one UNDO restores every source and
//! removes the new MTEXT.
