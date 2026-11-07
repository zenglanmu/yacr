# 2026-11-07 utility/named-object batch (`PURGE` / `RENAME` / `SETVAR` / `TIME` / `DBLIST` / `TREESTAT` / `MREDO` / `MULTIPLE`)

This lane adds ten command spellings (eight canonical selectors plus two
command-line aliases) to the shared command layer. All behavior below is
**contract-level** evidence: `cad-app` and `cad-ui-core` tests on Linux. No GPU,
device, browser, or platform-host acceptance is claimed.

## Selectors and aliases

| canonical | aliases | payload | mode |
|---|---|---|---|
| `PURGE` | `-PURGE` | none (all kinds) or `Purge(PurgeTarget)` | Work |
| `RENAME` | `-RENAME` | `Rename(RenameCommand)` | Work |
| `SETVAR` | — | `SetVar(String)` (report) | Viewer + Work |
| `TIME` | — | none | Viewer + Work |
| `DBLIST` | — | none | Viewer + Work |
| `TREESTAT` | — | none | Viewer + Work |
| `MREDO` | — | none (1) or `MRedo(u32)` | Work |
| `MULTIPLE` | — | none or `Multiple(Option<String>)` | Viewer + Work |

`-PURGE` and `-RENAME` are `COMMAND_ALIASES` rows, not second `CommandId`s: they
normalize onto the same command and the same write path. `docs/shortcuts.md`
§2b lists them so the command-docs gate and the alias table cannot drift.

## Boundaries (explicit, not hidden)

### `PURGE` / `-PURGE`

* Layers are the **only** named object with a delete API in this build
  (`DrawingDatabase::delete_layer`). Every non-default layer is offered to the
  existing `DELETELAYER` command; an unused row is removed, a referenced row is
  refused by the database (`layer.in_use`, the count is named), and each
  removed/refused row is reported (`purge.layer_removed` /
  `purge.layer_refused`).
* `delete_layer` is the authority: PURGE never re-points an entity's layer and
  never deletes the default layer `0`.
* Block definitions, text styles and linetypes have **no delete API**. An
  explicit `PURGE BLOCK|STYLE|LINETYPE` is a named `Unsupported`
  (`purge.block.unsupported`, etc.); the bare `PURGE` reports each unsupported
  kind (`purge.kind_unsupported`) and still purges layers.
* Layer-table writes are **not undoable** (`cad-history` stores entity patches
  only). PURGE records no undo entry and says so.
* No `-PURGE` option surface (e.g. nested `All`/`Zero` prompts) is implemented.

### `RENAME` / `-RENAME`

* `RENAME LAYER <old> <new>` is routed to the existing `RENAMELAYER` command, so
  the database's name rules apply unchanged: trimmed, non-empty, AutoCAD-illegal
  characters refused, case-insensitive duplicates refused. Unknown and ambiguous
  names are the existing `layer.unknown` / `layer.ambiguous_name` refusals.
* `RENAME BLOCK|STYLE|LINETYPE` is a named `Unsupported`
  (`rename.block.unsupported`, etc.): no rename API exists.
* Names containing whitespace must be double-quoted on the command line. Exactly
  three quote-aware tokens are accepted; an unquoted extra token is refused
  rather than guessed.
* Layer renames are **not undoable** and the rename never retargets entities
  (entity references are by id).

### `SETVAR`

* The closed set is `PDMODE`, `PDSIZE`, `SNAPUNIT`, `LTSCALE`, `GRID`, `SNAP`,
  `ORTHO`, `TEXTSIZE`. Unknown names are `setvar.unknown_variable`.
* `SETVAR <name>` is a **read-only report** (`setvar.value`), including for
  drawing-header and session variables.
* `SETVAR <name> <value>` writes through the **existing** handlers; there is no
  second write path:
  * `PDMODE` → `SetPointDisplayMode`
  * `PDSIZE` → `SetPointDisplaySize`
  * `SNAPUNIT` → `SetSnapSpacing`
  * `LTSCALE` → `SetLinetypeScale`
  * `GRID` / `SNAP` / `ORTHO` → the ordinary toggle selector, but only when the
    requested value differs from the current session state (set-to-value
    semantics; an equal value is reported, never flipped away)
  * `TEXTSIZE` → the existing session `set_tool_parameter(TextHeight)` API
* Value grammar is each variable's existing grammar (`PDMODE` an `i16` integer,
  `PDSIZE` a signed finite decimal, `SNAPUNIT`/`LTSCALE`/`TEXTSIZE` positive
  decimals, the three switches `ON|OFF|1|0`). Out-of-grammar values are named
  refusals (`invalid SETVAR <NAME> value ...`), never clamped.

### `TIME`

* Reports the open document's revision and entity count and the current **UTC**
  clock (`time.document`, `time.utc`).
* Drawing creation/edit timestamps are **NOT TRACKED** by this database
  (`time.not_tracked`); no date is invented.
* Session uptime is **NOT TRACKED** (`time.uptime_not_tracked`).
* Local-time rendering is **NOT TRACKED** (`time.local_not_tracked`): this build
  has no local-offset dependency, so only UTC is printed. If the system clock is
  before the Unix epoch, no UTC value is fabricated (`time.utc_unavailable`).

### `DBLIST`

* Lists every **top-level** entity (model space and paper space) with id, handle,
  type key, layer, space and draw order, the same fields `LIST` uses.
* Block-definition members are not top-level: they are counted and reported as
  skipped (`dblist.scope`), never silently mixed into the list.
* Read-only: no revision bump, no undo entry, no `ChangeSet`.

### `TREESTAT`

* The spatial index is **NOT TRACKED** (`treestat.not_tracked`). The application
  session retains no persistent index; `cad-spatial`'s `GridSpatialIndex` is
  built per pick query, so there is no tree size/depth to report. No fake entry
  count is produced.

### `MREDO`

* `MREDO` (no argument) applies **one** redo step; `MREDO <n>` applies up to `n`.
* Each step is one ordinary existing `CommandId::Redo` history move. One call
  therefore creates **one history entry per redone step** (the existing redo
  granularity), not a merged multi-step record.
* An empty redo stack is refused (`mredo.empty`); `n = 0`, a sign, a fraction or
  a non-integer is refused (`mredo.count` / `invalid MREDO argument`).
* If fewer steps are available than requested, the available steps are applied
  and reported as `mredo.partial` rather than pretending the full count ran.
* Work-only.

### `MULTIPLE`

* `MULTIPLE` or `MULTIPLE <command>` arms a real session flag
  (`MultipleRepeat::NextCommand` / `MultipleRepeat::Command`). An unknown target
  name is refused (`multiple.unknown_command`) before dispatch.
* **This host has no command repeat loop.** `MULTIPLE` therefore reports
  `multiple.armed` plus the named boundary
  `multiple.repeat_not_implemented`; it never claims that a command was repeated.
  A caller that wants real repetition must implement the loop and consume the
  flag. This is the exact implemented boundary, not a placeholder success.
* The flag is session state only: no database write, no history entry, no
  revision bump.

## Wiring

* `cad-app`: `CommandId` variants, `COMMAND_TABLE` rows, `Application::execute`
  arms, capability rows, payload types, session flag, and the command
  implementations in `app_utility.rs` (writes/reports) with `cad-db` as the only
  authority. PURGE/RENAME layer paths reuse the existing `DELETELAYER` /
  `RENAMELAYER` command handlers.
* `cad-ui-core`: canonical selectors, `-PURGE`/`-RENAME` aliases,
  `ARGUMENT_SELECTORS` and parsers, and the typed session handler
  `session/util_arguments.rs`.
* React: eight `COMMAND_CATALOG` rows, `ARGUMENT_COMMANDS` local grammar
  predicates, and `DOCUMENT_SCOPED_COMMANDS` entries in
  `apps/desktop-ui/src/commands/{catalog,gating}.ts`.
* i18n: `draw.kind.purge|rename|setvar|time|dblist|treestat|mredo|multiple` in
  both `zh-CN.json` and `en.json`.

## Tests

* `crates/cad-app/src/tests/utility.rs`: PURGE removes only unused layers and
  refuses referenced ones; explicit unsupported kinds; RENAME
  success/duplicate/unknown/illegal and unsupported kinds; SETVAR report /
  unknown / missing name; TIME/DBLIST/TREESTAT read-only (no revision or undo);
  MREDO count and empty stack; MULTIPLE flag and named refusal.
* `crates/cad-ui-core/src/session/tests/tests_commands/utility_arguments.rs`:
  typed SETVAR read/write routing (including no second write path), bad value and
  unknown variable refusals, PURGE/RENAME/MREDO/MULTIPLE grammar and named
  refusals.
* `command_vocabulary.rs` and `snap_grid.rs` were extended append-only for the
  eight new canonical selectors and five new argument selectors.

## Evidence status

| Check | Status |
|---|---|
| `cargo test -p cad-app --locked` | PASS (Linux, this worktree) |
| `cargo test -p cad-ui-core --locked` | PASS (Linux, this worktree) |
| `cargo check -p cad-app -p cad-ui-core --locked` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS |
| `python3 scripts/check-command-docs.py` | PASS |
| `python3 scripts/check-architecture.py` | PASS |
| Frontend `npm test` | NOT RUN (`apps/desktop-ui/node_modules` absent) |
| Windows / macOS / Android / Web runtime | NOT VERIFIED (shared-layer contract only) |
| GPU / pixel / performance | NOT APPLICABLE / NOT RUN |
