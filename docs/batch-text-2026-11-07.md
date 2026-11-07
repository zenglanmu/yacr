# 2D text batch (2026-11-07): JUSTIFYTEXT / TJUST / TSCALE / TEXTALIGN / TCOUNT / TCIRCLE / TEXTMASK / TEXTUNMASK (+ MTEDIT / -MTEXT)

Status: **partial, contract-tested**. Rust command layer + session typed-argument
grammar + frontend catalog/inspection tables are wired. This file is the honest
boundary record; `docs/compatibility.md`, `docs/handoff.md` and
`docs/autocad-2026-command-coverage.md` were deliberately **not** touched by this
batch.

Authorities:
- command catalogue and long contracts: `crates/cad-app/src/command_id.rs`,
  `crates/cad-app/src/command_id/text_batch.rs`
- write paths: `crates/cad-app/src/app_drawing/text_batch/`
- typed grammar: `crates/cad-ui-core/src/session/text_batch.rs`
- frontend admission: `apps/desktop-ui/src/commands/{catalog,gating}.ts`
- contract tests: `crates/cad-app/src/tests/drawing/text_batch.rs`,
  `crates/cad-ui-core/src/session/tests/tests_commands/text_batch.rs`

## Shared rules

- All seven canonical commands are **Work-only** (`requires_work_mode`). Viewer
  refuses them at the command layer with `PermissionDenied`.
- The **current session selection** is the whole input. There is no interactive
  `Select objects` prompt — the same declared divergence `SCALETEXT`, `TORIENT`,
  `TCASE`, `TXT2MTXT` and `TEXTTOFRONT` already carry in this build.
- Only **directly addressable model-space** `SemanticGeometry::Text` entities are
  text targets. `AcDbText` and `AcDbMText` share that variant; the stored
  `type_key` is preserved. A block-instance member, a sub-element, a
  non-model-space row, another document and a missing ref are named refusals (or
  named per-entity skips where the command documents one).
- Every refusal happens **before** a transaction opens: no revision bump, no
  `ChangeSet`, no undo entry. A successful write is **one transaction / one undo
  step**; the committed rows are read back for the undo patch. A change-free
  result is the command's `*.unchanged` diagnostic and opens **no** transaction.

### Estimated text extents (the one declared approximation)

`cad-app` has **no glyph-extents API**. Every command that needs a box uses
`app_drawing::text_batch::estimate_text_bounds`:

- width = `height × text.chars().count()`, height = `height` (world units);
- the anchor follows the stored alignment (`Left`/`Center`/`Right`,
  `Baseline`/`Bottom`/`Middle`/`Top`);
- the box is axis-aligned in world XY at `position.z`;
- **rotation is ignored**; proportional-font metrics are not modelled.

No pixel, golden, font-metric or AutoCAD-identical claim is made from this
estimate. It is stated in the diagnostics (`textmask.estimated_bounds`) and in
the affected contracts where it matters.

## Per-command status

| Command | Status | Input | Boundary |
|---|---|---|---|
| `JUSTIFYTEXT <H> <V>` | implemented | selection + typed `<H> <V>` | only `h_align`/`v_align` change; **position is never moved**; non-text targets are per-entity `justifytext.not_text` diagnostics (the TORIENT policy), never partially rewritten |
| `TJUST <H> <V>` | alias | same | `COMMAND_ALIASES` row; same `CommandId`, same grammar, same write path — no second implementation |
| `TSCALE <factor>` | implemented | selection + positive decimal | delegates to the one `scaled_text` rule `SCALETEXT` uses (height + MTEXT `wrap_width`); a `1.0` factor is `tscale.unchanged` with no transaction; non-text selection refuses the whole command |
| `TEXTALIGN LEFT\|RIGHT\|CENTER\|TOP\|MIDDLE\|BOTTOM` | implemented | selection (≥ 2 text objects) + edge word | shifts only the affected axis to the extreme/centre of the **estimated** union box; `< 2` targets is `textalign.needs_two`; non-text refuses the whole command; rotation ignored |
| `TCOUNT <start> <step> [prefix] [suffix]` | implemented | selection + 2–4 tokens | deterministic order top-to-bottom (descending Y) then left-to-right (ascending X), stable; `step` must be > 0 (`tcount.invalid_step`); no AutoCAD dialog modes |
| `TCIRCLE CIRCLE\|RECTANGLE` | implemented | selection + shape word | `CIRCLE` = real `Circle` at the estimated centre with radius = half the box diagonal; `RECTANGLE` = closed 4-vertex straight polyline; drawn on the **text's layer**, text retained; no associativity |
| `TEXTMASK` | implemented | selection only | one independent `SemanticGeometry::Mask { inverted: false }` (`AcDbWipeout`) per text from the estimated rectangle, on the text's layer, `draw_order = text.draw_order − 1` (saturating); no association model |
| `TEXTUNMASK` | implemented (heuristic) | selection of text and/or masks | deletes **selected** masks only: a selected mask is deleted when every boundary vertex lies inside the estimated bounds of a selected text; with no text selected, every selected mask is deleted; text-only selection is `textunmask.unchanged`; provenance cannot be proven |
| `MTEDIT` | alias | current single selection | `COMMAND_ALIASES` row onto `TEXTEDIT`; starts the existing pick tool; **no second edit path** |
| `-MTEXT` | alias | — | `COMMAND_ALIASES` row onto `MTEXT`; starts the existing two-corner tool; **no second capture path** |

## Explicitly NOT implemented

- `JUSTIFYTEXT`/`TJUST`: AutoCAD's re-justification that re-derives the
  justified insertion point from font metrics. This build has no metrics, so the
  insertion point is kept and nothing is re-derived.
- `TSCALE`: no base point, no position change, no uniform transform — it is the
  glyph-height scaler, exactly like `SCALETEXT` (which is not `SCALE`).
- `TEXTALIGN`: no interactive "specify base alignment point" prompt and no
  glyph-accurate alignment; the box is the declared estimate and rotation is
  ignored.
- `TCOUNT`: AutoCAD's dialog modes are **NOT IMPLEMENTED** — no per-item
  prefix/suffix editing, no "Find & replace", no "Select objects" prompt inside
  the command, no number padding/leading zeros, no decimal numbering.
- `TCIRCLE`: AutoCAD's `TCircle` dialog (style/offset, Object/Multiple modes) is
  **NOT IMPLEMENTED**; there is no associativity and no update when the text
  moves.
- `TEXTMASK`: no text frame/background association, no frames/display-not-plot
  options, and no "mask the text automatically when created" behaviour.
- `TEXTUNMASK`: cannot prove a mask came from `TEXTMASK`; unselected masks are
  never scanned; a rotated or proportionally wider text can leave a real mask
  outside the estimate.
- No ribbon button is added for the five argument-taking commands
  (`JUSTIFYTEXT`, `TSCALE`, `TEXTALIGN`, `TCOUNT`, `TCIRCLE`): a bare click would
  always be a `*.needs_argument` refusal. They are command-line first (catalog
  entries + `ARGUMENT_COMMANDS` grammar). `TEXTMASK`/`TEXTUNMASK` are bare and
  have real ribbon buttons. `MTEDIT`/`-MTEXT` are catalog entries for the
  existing tools.
- `TJUST` is admitted by alias normalisation only; no second `CommandId` or
  `COMMAND_TABLE` row exists (the table is one row per variant and is
  contract-tested).

## Verification (this worktree, 2026-11-07)

All paths relative to the worktree; every cargo command used
`CARGO_TARGET_DIR=/tmp/opencode/target-text-batch`.

- `cargo test -p cad-app -p cad-ui-core --locked` — passed
  (cad-app: 1788 lib tests + integration suites; cad-ui-core: 401 tests).
- `cargo check -p cad-app -p cad-ui-core --locked` — passed.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings`
  — passed.
- `python3 scripts/check-file-size.py` — passed.
- `python3 scripts/check-i18n.py` — passed.
- `python3 scripts/check-command-docs.py` — passed (152 canonical selectors,
  60 aliases; `docs/shortcuts.md` §2b updated for the three new aliases).
- Frontend tests: **NOT RUN** — `apps/desktop-ui/node_modules` is absent in this
  worktree. The TS changes are catalog/gating data only and were not executed.
- Linux GUI / real GPU / real DWG **NOT RUN**: this batch is a command-layer
  contract change; no runtime, pixel, font-metric or performance claim is made.
