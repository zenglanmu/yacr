# Layer-management batch (2026-11-07): LAYMCH / LAYMRG / LAYDEL / LAYERSTATE / LAYERPMODE / LTSCALE / LAYWALK

Status: **partial, contract-level evidence only.** Everything here is implemented
and pinned by Rust contract tests (`crates/cad-app/src/tests/layer_commands/*`,
`crates/cad-ui-core/src/session/tests/tests_commands/layer_arguments.rs`).
No GUI, GPU, pixel, real-DWG or cross-platform run was performed for this batch.

This file is the per-command boundary record for this round. `docs/compatibility.md`,
`docs/handoff.md` and `docs/autocad-2026-command-coverage.md` are owned by the
orchestrator and deliberately not edited here.

## Canonical paths and official names

| Canonical `CommandId` | Official names | Kind |
|---|---|---|
| `LayMCh` | `LAYMCH`, `-LAYMCH` | selection-based entity write |
| `LayMrg` | `LAYMRG`, `-LAYMRG` | selection-based entity + layer-table write |
| `LayDel` | `LAYDEL`, `-LAYDEL` | selection-based entity + layer-table write |
| `LayerState` | `LAYERSTATE` | typed `SAVE`/`RESTORE`/`LIST`/`DELETE` |
| `LayerPmode` | `LAYERPMODE` | session flag (`ON`/`OFF`, bare reports) |
| `SetLinetypeScale` | `LTSCALE` | typed positive decimal, drawing-header write |
| `LayWalk` | `LAYWALK` | typed comma-separated names, session overrides |

The `-` spellings are rows in `COMMAND_ALIASES` mapping onto the same
`CommandId`; they are **aliases, not second commands** (no dialog exists in this
build, so the dialog-free form is the whole behaviour).

All seven selectors are in `COMMAND_TABLE`, each canonical path is Work-only
except `LAYERPMODE` and `LAYWALK` (session/view operations, Viewer-legal).
`LAYMCH`/`LAYMRG`/`LAYDEL` take `CommandPayload::None` and read the session
selection.

## Per-command boundaries

### `LAYMCH` / `-LAYMCH`

- Destination = the **first** selected entity's stored layer. Every other
  selected **model-space** entity is moved there in **one transaction / one undo
  step**. This is intentionally not `LAYCUR` (which targets the session's current
  layer).
- Refusals before any write: empty selection (`laymch.selection`), a selection of
  one (`laymch.single_selection` — nothing to move), any payload
  (`laymch.payload`), a foreign/instance/sub-element ref or a non-model-space
  entity (`laymch.not_addressable`), a missing destination layer row
  (`laymch.destination_layer_missing`).
- All entities already on the destination is `laymch.unchanged` with **no**
  transaction and no undo entry.
- A locked destination is refused by `cad-db`'s own entity-write rule
  (`layer.locked`).
- Not implemented: AutoCAD's interactive "Select objects" prompt (the current
  selection is the input) and its layer picker.

### `LAYMRG` / `-LAYMRG`

- **Declared deviation:** sources are the distinct layers of the current
  selection; the target is the session's **active layer**. There are no
  interactive source/target layer prompts in this build.
- Every entity on a source layer (model space, paper space **and** block
  members) is rewritten to the active layer in one transaction / one undo step;
  then each source layer row is deleted with `DrawingDatabase::delete_layer`.
- Refusals before any write: empty selection (`laymrg.selection`), any payload
  (`laymrg.payload`), non-addressable selection (`laymrg.not_addressable`), a
  source equal to the active layer (`laymrg.source_is_active`), a source named
  `0` (`laymrg.default_layer`), a missing active/source row
  (`laymrg.active_layer_missing` / `laymrg.unknown_layer`).
- One published `ChangeSet` spans the entity commit and the layer deletions. The
  undo record carries **only entity patches**: layer-table edits are not
  undoable. Undo therefore restores the entities' layers but **not** the deleted
  layer rows (an undo can leave an entity pointing at a layer that no longer
  exists). This is a documented boundary, not silent success.
- Not implemented: AutoCAD's `-LAYMGR` prompts, undo of the layer-table half.

### `LAYDEL` / `-LAYDEL`

- Source layers are the distinct layers of the current selection. Every
  **model-space** entity on those layers is deleted through the one shared delete
  body (`delete_entity_refs`, one transaction / one undo step); the layer rows are
  then deleted (not undoable) and their session overrides dropped.
- Refusals before any write: empty selection (`laydel.selection`), any payload
  (`laydel.payload`), non-addressable selection (`laydel.not_addressable`), the
  active layer (`laydel.current_layer`), layer `0` (`laydel.default_layer`), and a
  source that still has paper-space or block-definition members
  (`laydel.layer_in_use`). The last check exists so the post-delete
  `delete_layer` cannot fail halfway.
- Entities in blocks/paper space are **not** deleted; a layer containing such
  members is refused instead. AutoCAD deletes them; that part is NOT IMPLEMENTED.
- Undo restores the deleted entities but not the layer rows (same boundary as
  `LAYMRG`). The deleted refs are dropped from the session selection.

### `LAYERSTATE`

- Typed grammar: `LAYERSTATE SAVE <name>`, `LAYERSTATE RESTORE <name>`,
  `LAYERSTATE LIST`, `LAYERSTATE DELETE <name>`. Bare `LAYERSTATE` is admitted and
  refused with `layerstate.needs_option`; an argument outside the grammar is
  refused before dispatch.
- Storage is the bounded session store `LayerStates` (`LAYER_STATE_CAP = 32`,
  oldest dropped). A snapshot records per-`LayerId` `LayerAttributes`, the active
  layer id and the session `LayerOverrideSet`. Nothing is written to the DWG and
  nothing persists across a session rebuild.
- `SAVE` replaces an existing name case-insensitively and moves it to the newest
  position. `LIST` reports names; `DELETE` removes one. Neither touches the
  drawing.
- `RESTORE` validates **every** stored layer id before any write
  (`layerstate.dangling_layer`), validates the saved active layer
  (`layerstate.dangling_active_layer`), and refuses a saved active layer that is
  off/frozen (`layerstate.active_layer_hidden`). It then applies attributes with
  `set_layer_attributes` (one revision bump per changed layer) and replaces the
  active layer and overrides.
- Layer-table writes are **not undoable** and record no `cad-history` entry.
- Not implemented: DWG/DXF named-layer-state table round-trip; AutoCAD's
  `LAYRSTATE`/`LAYRDO` surface beyond this store; per-viewport state.

### `LAYERPMODE`

- `LAYERPMODE ON` / `LAYERPMODE OFF`; a bare `LAYERPMODE` reports the current
  setting (`layerpmode.report`) and writes nothing. Other arguments are refused
  before dispatch.
- The flag defaults to **ON**, preserving the previous behaviour. When OFF, a
  real layer-override mutation still applies the override but does not push a
  `LAYERP` snapshot (`SessionState::push_layer_override_snapshot` is the single
  gated push site).
- Session state only: no database write, revision bump, history entry or
  preference.

### `LTSCALE`

- Typed value must be a positive finite plain decimal; zero, negatives, signs,
  exponents and non-finite values are refused before IPC, and
  `DrawingDatabase::set_linetype_scale` repeats the positivity/finiteness check.
- The write stores `$LTSCALE` in the open drawing, raises the revision exactly
  once and publishes an empty-object-changes `ChangeSet`. Writing the stored value
  is a reported no-op. **No undo entry** (cad-history has no header patch kind).
- Boundary: the DXF/DWG exporter writes per-entity `linetype_scale` only; the
  drawing-global `$LTSCALE` header value is **not** emitted by this build's
  exporter, so SAVE→OPEN does not round-trip it. This is NOT IMPLEMENTED, not a
  success.

### `LAYWALK`

- Typed `LAYWALK <name>[,<name>...]`: names are trimmed, the list must be
  non-empty and every entry non-empty. Names are resolved against the drawing
  (exact first, then unique case-insensitive); an unknown name refuses the whole
  command (`layers.walk.unknown_layer`).
- The keep-set is the named layers **plus the session active layer**. Everything
  else is hidden through the same validated atomic override batch as `LAYISO`
  (`hide_layers_atomically`), so a refusal changes nothing. No database write,
  revision bump or history entry. Idempotent all-kept runs report
  `layers.walk.noop`.
- `LAYUNISO` / `RESTORELAYERS` clears the result; `LAYERP` can walk it back
  (subject to `LAYERPMODE`).
- Not implemented: AutoCAD's LayerWalk dialog, per-object walking, and
  `Regen`/`Unreconciled` options. This is overrides-only.

## Verification evidence (this round)

Run with `CARGO_TARGET_DIR=/tmp/opencode/target-layer-batch`:

- `cargo test -p cad-app --lib --locked layer_commands` — **54 passed, 0 failed**
  (includes all new `laymch`/`laymrg`/`laydel`/`layerstate`/`layerpmode`/
  `ltscale`/`laywalk` contracts).
- `cargo test -p cad-ui-core --lib --locked layer_arguments` — **5 passed**.
- `cargo test -p cad-ui-core --lib --locked alias_table` — **5 passed**.
- `cargo test -p cad-ui-core --lib --locked command_normalization_covers_every_variant`
  — **1 passed** (`COMMAND_TABLE` now 152 selectors, 60 aliases).
- `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings`
  — passed.
- `cargo fmt --all -- --check` — passed.
- `python3 scripts/check-file-size.py` — passed.
- `python3 scripts/check-i18n.py` — passed (1237 keys).
- `python3 scripts/check-command-docs.py` — `OK: 0 mismatches`.
- Frontend tests: **NOT RUN** (`apps/desktop-ui/node_modules` is absent in this
  worktree). The catalog/gating/ribbon/i18n edits are type-checked only by
  inspection, not by `tsc`/`node --test`.
- No GUI/GPU/pixel/real-DWG/platform run; all conclusions are contract-level.

## Known not-run / not-implemented summary

- `-LAYMCH`/`-LAYMRG`/`-LAYDEL` interactive prompts: NOT IMPLEMENTED (the
  aliases reuse the selection-based commands).
- Undo of any layer-table edit (merge/delete/state restore): NOT IMPLEMENTED
  (`cad-history` has no layer patch kind).
- `LAYDEL` of paper-space/block members: NOT IMPLEMENTED (refused instead).
- `$LTSCALE` export/round-trip: NOT IMPLEMENTED.
- `LAYERSTATE` persistence / DWG table round-trip: NOT IMPLEMENTED.
- Windows/macOS/Android/Web behaviour: NOT VERIFIED; only the shared
  `cad-app`/`cad-ui-core` contracts above were exercised on Linux.
