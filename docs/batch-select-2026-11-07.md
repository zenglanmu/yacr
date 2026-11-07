# 2026-11-07 selection / filter / find / visibility batch

Status: **implemented at contract level**, verified only by the Rust tests listed
below. No GUI run, no frontend test run, no real-DWG run and no GPU/pixel
evidence was produced for this batch. This document is the per-command record;
`docs/compatibility.md`, `docs/handoff.md` and
`docs/autocad-2026-command-coverage.md` are deliberately not updated by this
lane.

Ten AutoCAD-2026 names are wired end to end:

| command | status | writes drawing? |
|---|---|---|
| `QSELECT <PROPERTY> <VALUE>` | implemented (v1 declarative filter) | no |
| `FILTER <PROPERTY> <VALUE>` | implemented (v1: same filter, ADD/union) | no |
| `GETSEL <LAYER\|TYPE> <name>` | implemented | no |
| `SSX` | implemented (five-property match) | no |
| `SELECTCOUNT` | implemented (report only) | no |
| `-FIND <needle>` | implemented (report only) | no |
| `FIND <needle>` / `FIND <needle> => <text>` | implemented | yes, replacing form only |
| `HIDEOBJECTS` | implemented (session hidden set) | no |
| `ISOLATEOBJECTS` | implemented (session hidden set) | no |
| `UNISOLATEOBJECTS` | implemented (clears hidden set) | no |

All ten are canonical selectors in `COMMAND_TABLE` (not aliases), carry a
`CommandId`, dispatch through `Application::execute`, are listed in the React
catalog, and have bilingual i18n keys. `SSX`, `SELECTCOUNT`, `HIDEOBJECTS`,
`ISOLATEOBJECTS` and `UNISOLATEOBJECTS` also have ribbon buttons; the typed
forms (`QSELECT`, `FILTER`, `GETSEL`, `-FIND`, `FIND`) deliberately have no
button because a button would have to invent the filter/search text.

## Shared grammar and engine

- **Typed argument** (session, `cad-ui-core/src/session/commands/filters.rs`):
  the five selectors are in `ARGUMENT_SELECTORS`; the property word is
  `TYPE|LAYER|COLOR|LINETYPE` (ASCII case-insensitive), the value is the trimmed
  remainder (spaces allowed, non-empty). `GETSEL` additionally accepts only
  `LAYER`/`TYPE`; `COLOR`/`LINETYPE` are refused locally and by the application.
- **Matching engine** (`cad-app/src/select_filter.rs`):
  - candidate set = every **top-level model-space entity**
    (`DrawingDatabase::model_space`); block-definition members are not enumerated;
  - layer off/frozen state is **not** a filter: `QSELECT` is a data query, not a
    screen query;
  - `TYPE` matches the stored `object.type_key` case-insensitively **and** a
    friendly geometry alias (`LINE`, `CIRCLE`, `ARC`, `POLYLINE`, `ELLIPSE`,
    `SPLINE`, `POINT`, `TEXT`, `MTEXT`, `INSERT`, `DIMENSION`, `HATCH`,
    `MULTILEADER`, `IMAGE`, `SHAPE`, `MESH`, `MASK`/`WIPEOUT`, `OPAQUE`,
    `COMPOUND`). `TEXT`/`MTEXT` are separate aliases even though both are
    `SemanticGeometry::Text`; the stored type key decides;
  - `LAYER` matches the stored layer name; `COLOR`/`LINETYPE` match the canonical
    projection of `cad_app::property_value` (`ByLayer`, `ByBlock`, `aci:N`,
    `#RRGGBB` / the drawing's linetype name), case-insensitively;
  - zero matches is **not** an error: `QSELECT`/`GETSEL` produce an empty
    selection, `FILTER` adds nothing, each reports a `.result` diagnostic.
- **All selection/filter commands are session-only**: no transaction, no
  revision bump, no undo entry, no `ChangeSet`; they call
  `SessionState::begin_selecting`.
- **ASCII case folding only** (the repository-wide convention): non-ASCII case
  pairs are not folded.

## Per-command boundaries

### `QSELECT <PROPERTY> <VALUE>`

REPLACE the selection with the match. Unknown property / empty value /
non-model space are named refusals. A payload whose merge mode is not `Replace`
is refused (`qselect.invalid_mode`). AutoCAD's dialog, named filter lists,
`Select All`/`Remove from list`, `Multiple` and saved `.flt` files are **NOT
IMPLEMENTED**; this is the declarative one-shot form.

### `FILTER <PROPERTY> <VALUE>`

Same grammar, ADD/union into the existing selection. **ADD to an empty selection
is valid** and selects the match (documented divergence from the dialog form,
which requires an existing selection to preview). A non-`Add` payload is
refused. Named-filter creation/editing and the dialog are **NOT IMPLEMENTED**.

### `GETSEL <LAYER|TYPE> <name>`

REPLACE, reusing the same engine. AutoCAD's `GETSEL` accepts more property
letters; this build implements only `LAYER` and `TYPE`, and names the refusal
for the others.

### `SSX`

No payload. The **first selected entity** is the template; every model-space
entity whose **type key + geometry discriminant, stored layer, canonical colour,
canonical linetype and space** all agree is unioned into the selection. Empty
selection (`ssx.selection_required`), a template with an instance path /
sub-element (`ssx.template_not_addressable`), a missing template entity and a
non-model active space are refused. Unlike SELECTSIMILAR (geometry kind only),
all five properties must match.

### `SELECTCOUNT`

Read-only. Same five-property template as `SSX`, reports the count in a
`selectcount.result` diagnostic, and never changes the selection, drawing
revision or undo history. Empty selection is refused.

### `-FIND <needle>`

Report only. ASCII case-insensitive substring search over model-space
`AcDbText`, `AcDbMText` (both `SemanticGeometry::Text`) and text-bearing
`AcDbMultiLeader` entities. Reports the match count, up to ten entity ids and a
short preview; writes nothing. A `=> replacement` is refused by name
(`-FIND` never replaces). Empty needle is refused.

### `FIND <needle>` / `FIND <needle> => <replacement>`

Without `=>`, behaves exactly like `-FIND`. With `=>`, every occurrence on every
match is replaced in **one transaction / one undo step**, with one before/after
patch per changed entity. Boundaries:

- the replacement is a **plain substring replacement**; MTEXT formatting runs
  are not parsed (`\P`, `\f…;`, `{`/`}` are ordinary characters and pass
  through);
- an `AcDbText` target refuses a replacement containing `\r`/`\n`
  (`text.single_line_newline`); MTEXT/MULTILEADER store line breaks as `\P`
  through the shared text policy;
- an empty replacement (`FIND x =>`) deletes the match; a MULTILEADER that would
  become empty is refused by `mleader.text_required` before the transaction
  opens;
- a MULTILEADER goes through `MultiLeaderGeometry::with_text`, which rebuilds
  its derived display; a TEXT/MTEXT keeps every other field;
- `FIND` is **Work-only as a selector** because the same name can replace;
  the Viewer-legal report form is the separate `-FIND` selector. `FIND <needle>`
  without `=>` therefore also requires Work mode (a deliberate, stable refusal,
  not a silent downgrade).

### `HIDEOBJECTS` / `ISOLATEOBJECTS` / `UNISOLATEOBJECTS`

Session-only hidden set (`SessionState::hidden_entities`); no database write,
revision bump, undo entry or persistence.

- `HIDEOBJECTS` inserts the current selection into the hidden set and clears the
  selection; empty selection refused (`hideobjects.selection_required`).
- `ISOLATEOBJECTS` replaces the hidden set with every **top-level model-space**
  entity except the current selection; the selection is kept; empty selection
  refused. Consequence of the top-level rule: selecting only a block member
  keeps nothing visible (its containing INSERT is hidden), and hiding a block
  member hides the addressed expansion, not every instance of the containing
  INSERT.
- `UNISOLATEOBJECTS` clears the hidden set; no selection required.

Integration:

- **Picking/selection**: the hidden set is part of the shared
  `picking::ref_visibility` predicate (with the layer overrides), so click, hover,
  box selection, `SELECTALL`, `SELECTSIMILAR`, grip candidates, stretch/hatch
  discovery and cross-reference inquiries all exclude hidden refs. The pick
  index is keyed by scene identity, so the hidden filter is applied at query
  time (same as layer overrides).
- **Rendering**: the host publishes the hidden set as `OverlayInputs.hidden`, so
  the renderer resolves `FLAG_HIDDEN`, which **discards the entity fragments
  in-shader**. Declared boundary: hidden entities are **not removed from the
  resident base scene** (fragment suppression until a full rebuild), and the
  screen-space POINT-marker overlay is derived from layer visibility only, so a
  hidden POINT's marker can still draw. No claim of a compacted/absent scene or
  of a hidden-marker fix is made.

## Verification (this worktree, 2026-11-07)

Run with `CARGO_TARGET_DIR=/tmp/opencode/target-select-batch`:

- `cargo test -p cad-app --lib --locked` → **ok, 1787 passed, 0 failed, 3 ignored**
  (includes the 15 new `tests::select_filter` contract tests)
- `cargo test -p cad-ui-core --lib --locked` → **ok, 398 passed, 0 failed, 1
  ignored** (includes 4 new `tests_commands::select_filter` grammar/vocabulary
  tests)
- `cargo check -p cad-app -p cad-ui-core --locked` → **ok**
- `cargo fmt --all -- --check` → **ok**
- `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings`
  → **ok**
- `python3 scripts/check-file-size.py` → **ok** (1471 files, limit 1000)
- `python3 scripts/check-i18n.py` → **ok** (1242 keys, both catalogs)
- `python3 scripts/check-command-docs.py` → **ok** (155 selectors)
- `python3 scripts/check-architecture.py` → **ok**

**NOT RUN**:

- frontend `npm test` / `npm run build` (`apps/desktop-ui/node_modules` is absent
  in this worktree);
- `cargo test --workspace`, `cargo check --workspace --all-targets`,
  `cargo check --workspace --lib --target wasm32-unknown-unknown`;
- `cargo check`/`test` for `app-*` host crates beyond `cad-host`'s share of the
  cad-app/cad-ui-core build;
- any GUI run, real-DWG run, GPU/pixel/performance evidence, Wayland/X11 visual
  check, simulator/device check.

Consequently the frontend changes (catalog, gating, ribbon, i18n, admission
mirror tests) are **contract-level only** here; the React test suite is the
backstop and it was not executed in this worktree.

## Files

- `crates/cad-app/src/command_id.rs` + `command_id/{ids,app_mode,capabilities,select_filter}.rs`
  (enum split for the size gate, new variants, Work-only `FindReplace`).
- `crates/cad-app/src/select_filter.rs` (engine + `QSELECT`/`FILTER`/`GETSEL`/
  `SSX`/`SELECTCOUNT`/hide-isolate handlers).
- `crates/cad-app/src/app_drawing/find_text.rs` (`-FIND`/`FIND`).
- `crates/cad-app/src/command_payload.rs` (`SelectFilter`, `TextFind`).
- `crates/cad-app/src/application.rs` (dispatch arm).
- `crates/cad-app/src/session_state.rs` + `session_state/toggles.rs`
  (`hidden_entities`; switch methods split for the size gate).
- `crates/cad-app/src/picking/mod.rs` and callers (hidden predicate in
  `ref_visibility`), `crates/cad-host/src/{state,input/*}.rs` (`FLAG_HIDDEN`
  publish + hidden-aware pick).
- `crates/cad-ui-core/src/session/commands.rs` + `commands/{arguments,filters}.rs`
  (selectors/grammar; parser block split for the size gate),
  `crates/cad-ui-core/src/session/select_filter.rs` (typed dispatch),
  `session/point_style.rs`, `session/mod.rs`.
- `apps/desktop-ui/src/{commands/catalog.ts,commands/gating.ts,ribbon/ribbon-modify.ts}`,
  `crates/cad-ui-core/i18n/{zh-CN,en}.json`.
- Tests: `crates/cad-app/src/tests/select_filter.rs`,
  `crates/cad-ui-core/src/session/tests/tests_commands/select_filter.rs`,
  plus the vocabulary/ribbon-tail contract updates.
