# 2026-11-07 layout / viewport batch

Scope: `LAYOUT`, `LAYOUTMERGE`, `LAYOUTWIZARD`, `VPORTS` (`-VPORTS` alias),
`VPMAX`, `VPMIN`, `VPCLIP`, `VPSCALE`, `LAYVPI`.

This is a **contract-level partial batch**. The read/list and session-space paths
are real. Every layout/viewport **write** is a named refusal because `cad-db`
exposes the layout table and `PaperViewport`s only through post-import **read**
accessors (`DrawingDatabase::layout`, `::layouts`, `::layout_entities`); the
builder is the only writer. Mutating a cloned `DrawingDatabase` would publish no
`ChangeSet`, leave the session/renderer on stale data and fake a success, so it
is deliberately not done.

## What each command does

| Selector | Real behaviour | Named refusals / boundary |
| --- | --- | --- |
| `LAYOUT LIST` | Reads the authoritative paper layouts in `LayoutId` order and emits one `layout.list` diagnostic per row (`layout.list.empty` when there are none). Read-only: no revision, no undo. | None. |
| `LAYOUT NEW <name>` | Validates the name and duplicate (case-insensitive). | `layout.invalid_name`, `layout.duplicate_name`, then `layout.write_unsupported` (no layout insert API). |
| `LAYOUT RENAME <old> <new>` | Resolves `old`, validates `new`, refuses a duplicate. | `layout.unknown`, `layout.ambiguous`, `layout.invalid_name`, `layout.duplicate_name`, `layout.write_unsupported` (no layout rename API). |
| `LAYOUT DELETE <name>` | Resolves the name and protects the last paper layout. | `layout.unknown`, `layout.last_layout`, `layout.write_unsupported` (no layout delete API). |
| `LAYOUT SET <name>` | Resolves the paper layout and switches the session's active space through the one `SwitchSpace` path (same fit/diagnostics as `SPACE`). Session state only: no database write, revision bump or undo entry. | `layout.unknown`, `layout.ambiguous`; an undrawable layout keeps `SPACE`'s own `Unsupported` reason. |
| `LAYOUTMERGE <source> <target>` | Resolves both layouts. | Missing (`layout.unknown`), same (`layoutmerge.same_layout`), then `layoutmerge.write_unsupported`: no entity-space move API and no layout delete API, so neither the move transaction nor the layout delete exists. |
| `LAYOUTWIZARD <name> [<paper> <width> <height>]` | Validates the name and optional paper fields (non-empty name, finite width/height `> 0`). | `layout.invalid_name`, `layout.duplicate_name`, `layoutwizard.invalid_paper`, then `layoutwizard.write_unsupported`: no layout creation and no page-setup write API; a typed paper size is **not applied**. |
| `VPORTS <rows> <cols>` / `-VPORTS` | Validates both counts `>= 1` and the total. | `vports.invalid_grid`; `vports.paper_space_required` outside a paper layout; `vports.write_unsupported` (no `PaperViewport` creation API). `-VPORTS` normalizes to the same selector/refusal — it is an alias, not a second path. |
| `VPMAX` | Payload-free. In the active paper layout it records a **session-only** maximized-viewport marker (`SessionState::maximized_viewport()`), naming the layout and viewport index `0`. Reports `vpmax.maximized`, or `vpmax.already_maximized`. | `vpmax.invalid_payload`, `vpmax.paper_space_required`, `vpmax.no_viewports`; a layout with no viewport is refused. |
| `VPMIN` | Payload-free. Clears the marker (`vpmin.restored`). | `vpmin.invalid_payload`; an empty marker is the named no-op `vpmin.nothing_to_restore` (success, nothing changed). |
| `VPCLIP RECT <minx> <miny> <maxx> <maxy>` | Validates a finite, non-degenerate rectangle. | `vpclip.polygonal_not_implemented` for any non-`RECT` boundary; `vpclip.invalid_argument`; `vpclip.degenerate_rect`; after validation `vpclip.viewport_selection_unsupported` because there is no selected-viewport state and no clip write API. Polygonal/object clips are **NOT IMPLEMENTED**. |
| `VPSCALE <scale>` | Resolves the argument against the drawing's scale table; a finite non-zero ratio is accepted without a table row. | `vpscale.zero_scale`, `vpscale.non_finite_scale`, `vpscale.invalid_scale`, `vpscale.unknown_scale`, then `vpscale.write_unsupported` (no viewport scale write API). The refusal message reports the resolved name/factor. |
| `LAYVPI <layer>[,<layer>...]` | Parses and resolves every layer name. | Empty list/empty comma entry `layvpi.invalid_argument`; `layvpi.unknown_layer`/`layvpi.ambiguous_layer`; then `layvpi.viewport_layer_state_unsupported` because this build has **no per-viewport layer state**. Layers are never frozen globally as a fallback. |

## Name and argument grammar

* Layout names are trimmed; empty, multi-line, over 255 characters or containing
  an AutoCAD-illegal character (`<>/\":;?*|=` and backtick) is
  `layout.invalid_name`.
* Duplicate checks compare case-insensitively. Resolution is exact first, then a
  **unique** case-insensitive match (`layout.unknown` / `layout.ambiguous`).
* A single-name form (`LAYOUT NEW|DELETE|SET`, `LAYOUTWIZARD`) takes the whole
  remainder, so names with spaces work. The two-name forms
  (`LAYOUT RENAME <old> <new>`, `LAYOUTMERGE <source> <target>`) split at the
  first whitespace; their **left-hand** name cannot contain a space. This is a
  documented command-line grammar boundary, not a guessed layout.
* `LAYOUTWIZARD`'s name is the first token because the optional paper triple
  follows it.
* `VPORTS` counts are `u32 >= 1`; the product is overflow-checked.
* `VPCLIP` admits only the `RECT` form. Any other first word returns
  `vpclip.polygonal_not_implemented`.
* `VPSCALE` accepts one token: a table name or an `a:b` ratio. Zero, negative and
  non-finite values are refused before dispatch.
* `LAYVPI` takes a non-empty comma-separated list; empty entries are refused.

## Undo / authority

* `LAYOUT SET`, `VPMAX` and `VPMIN` write no drawing data: no revision bump, no
  `ChangeSet`, no `cad-history` entry and no undo step.
* No layout-table or viewport write exists in this build, so the question of
  "undoable or not" for those writes is moot; when such an API is added, the
  layout-table delete still has no patch kind in `cad-history` and must be
  documented as not undoable.
* `VPMAX`/`VPMIN` are session display hints only. The host may read
  `cad_app::SessionState::maximized_viewport()`; no second viewport authority is
  created and no renderer state is mutated here.

## Tests and evidence

Run in this worktree with `CARGO_TARGET_DIR=/tmp/opencode/target-layout-batch`:

* `cargo test -p cad-app --locked` — includes 12 `layout_commands` contracts;
* `cargo test -p cad-ui-core --locked` — includes the typed grammar/routing
  contracts and the exhaustive `CommandId` selector table;
* `cargo check -p cad-app -p cad-ui-core --all-targets --locked`;
* `cargo fmt --all -- --check`, `cargo clippy -p cad-app -p cad-ui-core
  --all-targets --locked -- -D warnings`;
* `python3 scripts/check-file-size.py`, `check-i18n.py`,
  `check-command-docs.py`, `check-architecture.py`.

Frontend `npm test` / `npm run build`: **NOT RUN** — this worktree has no
`apps/desktop-ui/node_modules`. The frontend catalog/gating rows, i18n keys and
contract assertions were added and reviewed statically; that is **not** a
frontend test result.

No GUI was launched. No real DWG, GPU, pixel, performance or platform claim is
made. Windows/macOS/Android/Web: **NOT VERIFIED / contract only**. `acadrust`
and the Cargo patch table are untouched.
