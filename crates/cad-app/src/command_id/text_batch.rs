//! Long-form contracts for the 2026-11-07 2D text batch:
//! `JUSTIFYTEXT`/`TJUST`, `TSCALE`, `TEXTALIGN`, `TCOUNT`, `TCIRCLE`,
//! `TEXTMASK` and `TEXTUNMASK` (plus the `MTEDIT` / `-MTEXT` spellings).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variants keep compact docs and link here. Nothing is implemented
//! in this module — the write paths live in `crate::app_drawing::text_batch`.
//!
//! # Shared boundaries (all seven commands)
//!
//! * **Work-only**: Viewer mode refuses every one of them at the command layer.
//! * The **current session selection** is the input. There is no interactive
//!   `Select objects` prompt (the declared divergence this build already carries
//!   for `TORIENT`/`TCASE`/`TXT2MTXT`/`SCALETEXT`/`TEXTTOFRONT`).
//! * Every refusal happens **before** a transaction opens: a refused command
//!   bumps no revision, publishes no `ChangeSet` and records no undo entry.
//! * A successful write is **one** transaction / **one** undo step; the
//!   committed rows are read back so each undo patch carries the exact stored
//!   value.
//! * Only directly addressable **model-space** entities are targets. A ref with
//!   an instance path or sub-element, a ref from another document, a missing ref
//!   and a non-model-space row are refusals (or named diagnostics, where the
//!   command documents a per-entity skip).
//! * All text size/extent numbers are **estimates**. `cad-app` has no glyph
//!   extents API, so every width is `height × character_count` and every height
//!   is `height`. The estimate is axis-aligned in world XY and **ignores stored
//!   rotation**. No pixel/golden claim is made from it.
//!
//! # Estimated text bounds
//!
//! `estimate_text_bounds` (in `crate::app_drawing::text_batch`) is the single
//! estimate:
//!
//! * width = `height × text.chars().count()`, height = `height` (world units);
//! * the anchor follows the stored alignment: `Left` puts `position.x` on the
//!   left edge, `Center` on the centre, `Right` on the right edge;
//!   `Baseline`/`Bottom` put `position.y` on the bottom edge, `Middle` on the
//!   centre, `Top` on the top edge;
//! * the `z` is `position.z`.
//!
//! Because there is no glyph metric, a proportional font's real extent differs
//! from this box; the docs (`docs/batch-text-2026-11-07.md`) state that boundary
//! explicitly and no command claims otherwise.

/// `JUSTIFYTEXT <H> <V>` / `TJUST <H> <V>`: change **only** the alignment of the
/// selected Text/MText.
///
/// Payload is [`crate::CommandPayload::JustifyText`] with one [`cad_domain::TextAlignH`]
/// and one [`cad_domain::TextAlignV`]. The typed grammar accepts
/// `LEFT|CENTER|RIGHT` for H and `BASELINE|BOTTOM|MIDDLE|TOP` for V
/// (case-insensitive, whitespace separated). A bare `JUSTIFYTEXT` (the name-only
/// funnel) is the named `justifytext.needs_argument` refusal: this build has no
/// in-command "Enter an option" prompt and never invents a default alignment.
///
/// **`TJUST` is an alias** of this canonical selector
/// ([`crate::CommandId::JustifyText`]); it shares the same typed grammar and the
/// same write path — there is no second implementation.
///
/// ## What changes
///
/// Only `h_align` and `v_align` are rewritten. `position`, `text`, `height`,
/// `rotation`, `style`, `font`, `wrap_width`, `type_key`, layer, space and draw
/// order are preserved **byte for byte**; the entity keeps its id (an update,
/// not a replacement). **The insertion position is not moved** — AutoCAD's
/// `JUSTIFYTEXT` also keeps the insertion point and re-derives the justified
/// location; this build has no glyph metrics, so nothing is re-derived.
///
/// ## Refusals and skip policy
///
/// * `justifytext.needs_selection` — the selection is empty;
/// * `justifytext.invalid_payload` — an unexpected payload variant;
/// * a selected non-text entity, a block-instance member/sub-element, a ref from
///   another document and a missing ref are reported as a per-entity
///   `justifytext.not_text` diagnostic and **skipped**, exactly like `TORIENT`.
///   The command does not fail when at least one Text/MText changed; it never
///   partially rewrites a non-text kind.
/// * `justifytext.unchanged` — no Text/MText target would change (every target
///   already has the requested pair). This is a success with **no transaction**.
///
/// One transaction / one undo step covers every changed entity.
pub struct JustifyTextCommand;

/// `TSCALE <factor>`: scale the glyph height (and the MTEXT wrap width) of the
/// selected Text/MText by one positive finite factor.
///
/// Payload is [`crate::CommandPayload::TextScale`]. The typed grammar is one
/// positive decimal factor (`2`, `0.5`, `1.25`) parsed by the session; a bare
/// `TSCALE` is the named `tscale.needs_factor` refusal (no fabricated default).
/// A non-finite or non-positive factor is `tscale.invalid_factor`.
///
/// The scale rule is the **same pure `scaled_text` function `SCALETEXT` uses**
/// (`crate::app_drawing::scale_text`): `height → height × factor`, an MTEXT
/// `wrap_width: Some(w)` → `Some(w × factor)`, `None` stays `None`. Nothing else
/// changes (position, rotation, alignment, style, font, string, layer, space,
/// draw order and the entity id). This is not `SCALE`: there is no base point
/// and no position change.
///
/// A factor of exactly `1.0` is `tscale.unchanged` with no transaction. TSCALE
/// differs from `SCALETEXT` only in **where the factor comes from** (the typed
/// argument instead of the session `scaleTextFactor` tool parameter); the write
/// path and every refusal code are shared.
///
/// Refusals: `tscale.needs_selection`, `tscale.not_addressable`,
/// `tscale.target_missing`, `tscale.invalid_factor`, `tscale.degenerate`,
/// `tscale.invalid_result`; a non-text selection is the whole-command
/// `Unsupported` (a mixed selection is never partially scaled). Work-only.
pub struct TextScaleCommand;

/// The alignment modes `TEXTALIGN <mode>` accepts.
///
/// The first six are the single-axis words this build already shipped (kept for
/// compatibility with existing callers and lines); the nine combined words are
/// AutoCAD 2026's `TEXTALIGNMODE` family. Together they are the 15 admitted
/// spellings (`TEXTALIGN LEFT|RIGHT|CENTER|TOP|MIDDLE|BOTTOM|TOPLEFT|TOPCENTER|
/// TOPRIGHT|MIDDLELEFT|MIDDLECENTER|MIDDLERIGHT|BOTTOMLEFT|BOTTOMCENTER|
/// BOTTOMRIGHT`). The host parses them case-insensitively; no abbreviation is
/// admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignEdge {
    Left,
    Right,
    Center,
    Top,
    Middle,
    Bottom,
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// The axis of `TEXTALIGN DISTRIBUTE [X|Y]` (the default is `Y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignAxis {
    X,
    Y,
}

/// The `TEXTALIGN` spacing options.
///
/// `Distribute` is implemented. `SetSpacing` / `CurrentVertical` /
/// `CurrentHorizontal` are **accepted by the typed grammar and refused by the
/// application** with their own stable codes (never a silent default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignSpacing {
    Distribute(TextAlignAxis),
    SetSpacing,
    CurrentVertical,
    CurrentHorizontal,
}

/// `TEXTALIGN <mode>`: bring the selected Text/MText to one common alignment
/// edge/centre measured from the **estimated** extents.
///
/// Payload is [`crate::CommandPayload::TextAlignEdges`]. A bare `TEXTALIGN` is
/// the named `textalign.needs_argument` refusal; any other word is refused by
/// the session parser before IPC.
///
/// ## Alignment modes (15 admitted spellings)
///
/// * single-axis, kept from the original six: `LEFT`, `RIGHT`, `CENTER` (X only)
///   and `TOP`, `MIDDLE`, `BOTTOM` (Y only);
/// * combined, AutoCAD 2026's nine `TEXTALIGNMODE` variants: `TOPLEFT`,
///   `TOPCENTER`, `TOPRIGHT`, `MIDDLELEFT`, `MIDDLECENTER`, `MIDDLERIGHT`,
///   `BOTTOMLEFT`, `BOTTOMCENTER`, `BOTTOMRIGHT`.
///
/// ## Rule (estimated extents)
///
/// For every selected Text/MText the **estimated** bounds are computed first
/// (see the module docs). The **union bounding box** of those extents is the
/// reference frame. A single-axis mode uses one extreme of that frame — `LEFT`
/// → the minimum left edge, `RIGHT` → the maximum right edge, `TOP` → the maximum
/// top edge, `BOTTOM` → the minimum bottom edge; `CENTER`/`MIDDLE` → the frame
/// centre — and shifts each entity **only on that axis**. A combined mode uses
/// the named horizontal extreme/centre **and** the named vertical
/// extreme/centre of the same frame and shifts each entity on **both** axes so
/// its own bounds edge/centre matches.
///
/// This is the deterministic non-interactive form of AutoCAD's base-object plus
/// second-point alignment vector: AutoCAD picks a base object and a direction;
/// this build instead uses the whole selection's union box, so no pick step and
/// no invented base object exists. Only reference points move — text content,
/// height, rotation, justification, style and width are untouched.
///
/// Because extents are estimated (`height × char_count` and `height`), the
/// aligned result is the best available approximation, not a glyph-accurate
/// alignment. Rotation is ignored by the estimate.
///
/// ## Refusals
///
/// * `textalign.needs_selection` — empty selection;
/// * `textalign.needs_two` — fewer than **two** Text/MText targets (aligning one
///   object to itself cannot produce a real result);
/// * `textalign.not_addressable` / `textalign.target_missing` — an
///   instance-path/sub-element ref, a non-model-space row, another document or a
///   missing ref;
/// * `textalign.degenerate` — a stored height that is non-finite/non-positive;
/// * a non-text selection refuses the **whole** command with the shared
///   `Unsupported` (never a partial alignment);
/// * `textalign.unchanged` — every target already sits on the common line(s); a
///   success with no transaction.
///
/// One transaction / one undo step. Work-only.
pub struct TextAlignCommand;

/// `TEXTALIGN DISTRIBUTE [X|Y]`: even **gap** spacing along one axis.
///
/// Payload is [`crate::CommandPayload::TextAlignSpacing`] carrying
/// [`TextAlignSpacing::Distribute`]. The typed grammar is `DISTRIBUTE` (Y by
/// default) or `DISTRIBUTE X` / `DISTRIBUTE Y`; the axis word is optional and
/// case-insensitive.
///
/// ## Rule (gap-based, estimated extents)
///
/// * The selection is read in **reading order**: for `Y`, top-to-bottom
///   (descending `max_y`) then left-to-right (ascending `min_x`); for `X`,
///   left-to-right (ascending `min_x`) then top-to-bottom (descending `max_y`).
///   Equal keys break on the entity id, so the order never depends on selection
///   order.
/// * The **first and last** extents in that order stay fixed. Their free span is
///   the distance from the first leading edge to the last trailing edge minus
///   the sum of all extents; dividing it by `count - 1` gives the common **gap
///   between consecutive extents**. Each interior text is shifted so the gap
///   before and after it equals that value. This is gap distribution, **not**
///   centre distribution (texts of different widths/heights keep their size).
/// * A selection that already has equal gaps stages nothing
///   (`textalign.unchanged`, no transaction).
/// * An overlapping selection can yield a negative gap (the interiors overlap by
///   the same amount); the rule is still deterministic.
///
/// ## Refusals
///
/// * `textalign.needs_selection` — empty selection;
/// * `textalign.distribute_needs_three` — fewer than **three** Text/MText
///   targets (with two there is no interior text to distribute);
/// * `textalign.degenerate` — a non-finite/non-positive stored height or a
///   non-finite gap;
/// * `textalign.not_addressable` / `textalign.target_missing` — as above;
/// * a non-text selection refuses the whole command (`Unsupported`);
/// * `textalign.unchanged` — already evenly spaced; no transaction.
///
/// One transaction / one undo step. Work-only.
pub struct TextAlignSpacingCommand;

/// The three named spacing refusals of `TEXTALIGN` (documented, never faked).
///
/// The typed grammar admits `SETSPACING`, `CURRENTVERTICAL` and
/// `CURRENTHORIZONTAL` so the host can answer with its own stable code:
///
/// * `textalign.set_spacing_unsupported` — `TEXTALIGN SETSPACING` needs a
///   numeric spacing value and a per-object base it does not model;
/// * `textalign.current_vertical_unsupported` — `CURRENTVERTICAL`;
/// * `textalign.current_horizontal_unsupported` — `CURRENTHORIZONTAL`.
///
/// Every one writes nothing (no revision bump, no undo entry). Only `DISTRIBUTE`
/// is implemented.
pub struct TextAlignUnsupportedSpacingCommand;

/// `TCOUNT <start> <step> [prefix] [suffix]`: replace each selected Text/MText
/// string with `<prefix><number><suffix>`.
///
/// Payload is [`crate::CommandPayload::TextCount`]. The typed grammar is two to
/// four whitespace-separated tokens: `start` and `step` are whole numbers;
/// `prefix` is the optional third token and `suffix` the optional fourth (a
/// suffix cannot be given without a prefix). `start` is a signed `i64`; `step`
/// must be a **strictly positive** `i64` (zero and negative are refused:
/// `tcount.invalid_step`). A 2/3/4-token argument outside that shape is
/// `tcount.invalid_argument`; a bare `TCOUNT` is `tcount.needs_argument`.
///
/// ## Ordering
///
/// Sources are sorted **top-to-bottom (descending Y) then left-to-right
/// (ascending X)**, stably, using the same sort `TXT2MTXT` uses. Row `i` (0-based)
/// becomes `<prefix><start + i × step><suffix>`. The order is deterministic for a
/// given selection and geometry — no selection-order dependence.
///
/// ## Divergences (declared)
///
/// AutoCAD's `TCOUNT` presents a dialog with per-mode options (Select objects,
/// number order, per-item prefix/suffix editing, "Find & replace", "Add
/// incrementing number to text"). Those dialog modes are **NOT IMPLEMENTED**:
/// this build has the typed positional grammar only, and prefix/suffix apply to
/// every item uniformly. Number formatting is plain `i64` `Display` (no padding,
/// no leading zeros and no decimal form).
///
/// ## Refusals
///
/// `tcount.needs_selection`, `tcount.invalid_payload`, `tcount.invalid_step`,
/// `tcount.not_addressable`, `tcount.target_missing`, and the whole-command
/// `Unsupported` for any non-text target. Every refusal writes nothing. When every new string equals the stored
/// one the command reports `tcount.unchanged` and opens no transaction; otherwise
/// all replacements are staged in one transaction / one undo step. Work-only.
pub struct TCountCommand;

/// The two boundary shapes `TCIRCLE CIRCLE|RECTANGLE` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBoundaryShape {
    Circle,
    Rectangle,
}

/// `TCIRCLE CIRCLE|RECTANGLE`: draw a **closed boundary** around the estimated
/// bounds of every selected Text/MText, keeping the text.
///
/// Payload is [`crate::CommandPayload::TextCircle`]. A bare `TCIRCLE` is
/// `tcircle.needs_argument`; any other word is refused by the session parser.
///
/// ## Geometry (estimated)
///
/// * `CIRCLE` — one `SemanticGeometry::Circle` centred on the **estimated bounds
///   centre** with `radius = half the box diagonal`
///   (`0.5 × hypot(width, height)`), normal `+Z`;
/// * `RECTANGLE` — one closed four-vertex straight-segment
///   `SemanticGeometry::Polyline` on the four estimated box corners
///   (`(min_x,min_y) → (max_x,min_y) → (max_x,max_y) → (min_x,max_y)`).
///
/// Both are drawn on the **text's own layer** and in the text's space (model
/// space only), at the model-space draw order above the current maximum. The
/// source text is **retained** — this is not a conversion and there is no
/// associativity: moving/deleting the text later does not update the boundary.
/// AutoCAD's dialog (style/offset, "Object"/"Multiple" modes) is **NOT
/// IMPLEMENTED**.
///
/// ## Refusals
///
/// `tcircle.needs_selection`, `tcircle.invalid_payload`,
/// `tcircle.not_addressable`, `tcircle.target_missing`, `tcircle.degenerate`
/// (a stored height that is non-finite/non-positive, or an estimate with no
/// positive radius/extent), and the whole-command `Unsupported` for a non-text
/// target. One transaction / one undo step inserts **all** boundaries; a
/// refusal inserts none. Work-only.
pub struct TCircleCommand;

/// `TEXTMASK`: insert an `AcDbWipeout` mask behind every selected Text/MText.
///
/// Payload is [`crate::CommandPayload::None`] (documented defaults) or
/// `UtilityPayload::TextMask`; the selection is the
/// input. The offset factor (default **0.35**) grows the estimated text box by
/// `factor × stored height` on every side. Mask types: `Wipeout` is stored as
/// `SemanticGeometry::Mask { boundary, inverted: false }` (`AcDbWipeout`);
/// `3dface` and `Solid` have no entity model and are named `Unsupported`
/// refusals (`textmask.mask_type`).
///
/// The text and its new mask are added to the existing session group store under
/// a generated `*TEXTMASK-<n>` name, so group-aware move/copy/erase sees them
/// together. The mask remains a **plain independent entity** in the DWG (no
/// owner/back-reference); re-running TEXTMASK deletes the previous
/// `*TEXTMASK-*` group mask for the selected text and creates the new one in the
/// same transaction. A full group store is `textmask.group_cap`, never a silent
/// drop.
///
/// Refusals: `textmask.needs_selection`, `textmask.invalid_payload`,
/// `textmask.invalid_offset`, `textmask.mask_type`,
/// `textmask.not_addressable`, `textmask.target_missing`,
/// `textmask.degenerate`, `textmask.group_cap`, and the whole-command
/// `Unsupported` for a non-text target. One transaction / one undo step inserts
/// every mask and deletes the replaced ones. Work-only.
pub struct TextMaskCommand;

/// `TEXTUNMASK`: delete masks belonging to the selected text.
///
/// Payload is [`crate::CommandPayload::None`]; the selection is the input.
///
/// ## Deletion rule
///
/// * A selected text's `*TEXTMASK-*` group names the exact mask to remove, so
///   `TEXTUNMASK` deletes that mask even when the mask itself is not selected.
///   Only groups created by TEXTMASK are consulted; unrelated drawing masks are
///   never scanned.
/// * Selected `SemanticGeometry::Mask` entities continue to use the declared
///   estimated-bounds heuristic: a mask is deleted when every boundary vertex
///   lies inside the estimated bounds of at least one selected text (with a
///   small world tolerance).
/// * When no text is selected, every selected mask is deleted.
///
/// The TEXTMASK group is removed after its mask is deleted. A selection with no
/// deletable mask is `textunmask.unchanged` with no transaction.
///
/// ## Honest limits
///
/// The DWG stores no association record; this command cannot prove provenance
/// for a selected mask outside a TEXTMASK group. The estimate is
/// `height × char_count` and ignores rotation. Non-mask selected entities are
/// `textunmask.skipped`.
///
/// Refusals: `textunmask.needs_selection`, `textunmask.invalid_payload`. One
/// transaction / one undo step deletes every mask. Work-only.
pub struct TextUnmaskCommand;
