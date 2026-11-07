# View/space batch (2026-11-07): REDRAW, REGEN, VIEW, VIEWGO, VIEWBACK, VIEWFORWARD, VPOINT

Status: **contract-level implementation**. No GUI, GPU, pixel or performance
claim is made here. All ten commands in this batch are **session view state
only**: they never write the drawing, bump its revision, create a
`cad-history`/undo entry or open a transaction.

## Commands and boundaries

| Selector | `CommandId` | Input | Behaviour | Declared limits |
|---|---|---|---|---|
| `REDRAW` | `Redraw` | none | Advances the current viewport's `refresh_generation` and the session `generation`; reports `redraw`. | No camera move, no `ZOOM PREVIOUS` step, no database write. The host re-reads derived status from the stamp; the command itself renders nothing. |
| `REDRAWALL` | `RedrawAll` | none | Same stamp for every viewport of the open document; reports `redraw.all`. | Cannot repair a corrupt display; it is a refresh request, not a renderer call. |
| `REGEN` | `Regen` | none | Advances the current viewport's `regen_generation` and the session `regen_generation`; reports `regen`. | The derived scene cache is owned by the render session; this command publishes the regeneration stamp the host consumes. It does not reparse the DWG. |
| `REGENALL` | `RegenAll` | none | Same stamp for every viewport of the document; reports `regen.all`. | Same boundary as `REGEN`. |
| `VIEW` / `-VIEW` | `View` | typed `SAVE <name>` / `RESTORE <name>` / `LIST` / `DELETE <name>` | Session-only named-view store (cap **32**, case-insensitively unique names). `SAVE` records camera + 2D/3D mode + work plane + active space; `RESTORE` applies them; `LIST`/`DELETE` report/drop. | **Not** DWG persistence: nothing is written to the file. `-VIEW` is an alias onto the same `CommandId`/path, not a second gallery/dialog. A bare `VIEW` is refused for its missing option. |
| `VIEWGO` | `ViewGo` | typed `<name>` | The same restore path as `VIEW RESTORE`; reports `view.restored`. | Unknown names are `view.unknown` refusals. A saved paper space is re-validated against the current drawing. |
| `VIEWBACK` | `ViewBack` | none | Pops the existing per-viewport `ZOOM PREVIOUS` stack, records the view being left on the forward stack, restores the popped view; reports `view.back`. | Empty stack is the named `view.back.empty` no-op. |
| `VIEWFORWARD` | `ViewForward` | none | Pops the bounded forward stack, re-records the current view on the back stack, restores; reports `view.forward`. | Empty stack is `view.forward.empty`. See the deviation below. |
| `VPOINT` | `VPoint` | typed `<x> <y> <z>` or `TOP\|BOTTOM\|FRONT\|BACK\|LEFT\|RIGHT` | Standard words reuse `apply_standard_view` (`TOP` is the 2D plan). A vector sets a 3D perspective eye direction, preserving the current distance (or the bounds-derived `enter_3d` distance from 2D). | A zero or non-finite vector is `vpoint.invalid_vector`, refused before any camera change. The model is never modified. The vector form always produces a 3D observation; it does not switch back to plan for `0 0 1`. |

## Deliberate deviation: forward stack

`ZOOM PREVIOUS` previously only popped the bounded back stack. This batch adds a
per-viewport **forward** stack (`ZoomHistory::push_forward`/`pop_forward`, same
cap of 16). `VIEWBACK` and `ZOOM PREVIOUS` push the view they leave; any new
discrete view change clears the forward stack. `VIEWFORWARD` walks it. This is
an explicit extension of the existing `crate::zoom_history` value, not a second
history: there is still one author of the view order.

## Registration and gating

- `CommandId` variants + `COMMAND_TABLE` rows; `-VIEW` alias in
  `COMMAND_ALIASES` and `docs/shortcuts.md` §2b.
- Typed grammar: `ARGUMENT_SELECTORS` + `parse_view_argument` /
  `parse_viewgo_argument` / `parse_vpoint_argument`; session dispatch in
  `session/view_arguments.rs` (mirrors `layer_arguments.rs`).
- None of the ten is in `requires_work_mode`, so Viewer mode admits them all.
- UI: catalog rows, `ARGUMENT_COMMANDS` for `VIEW`/`VIEWGO`/`VPOINT`,
  `DOCUMENT_SCOPED_COMMANDS` for all nine selectors, and a View tab
  `viewstate` panel for the payload-free `REDRAW`/`REDRAWALL`/`REGEN`/
  `REGENALL`/`VIEWBACK`/`VIEWFORWARD` buttons.
- Both i18n catalogs carry every new label and hint.

## Tests

- `crates/cad-app/src/tests/view_batch.rs`: stamps vs. drawing/undo, named-view
  round trip and unknown refusal, cap 32, back/forward order, empty stacks,
  `ZOOM PREVIOUS` → forward, `VPOINT` words/vector/zero refusal, payload refusal.
- `crates/cad-ui-core/src/session/tests/tests_commands/view_arguments.rs`:
  grammar, `-VIEW` alias, session round trip, unknown name, empty stacks,
  zero-vector refusal.
- `apps/desktop-ui/test/view-batch.test.ts`: catalog, grammars, document gate,
  View tab view-state panel.

## NOT VERIFIED

- No real GUI/GPU run, no screenshot/pixel/performance evidence.
- `REGEN`/`REGENALL` publish a regeneration stamp; whether a given host rebuilds
  its derived scene cache from it is host-side behaviour and is **not**
  exercised here.
