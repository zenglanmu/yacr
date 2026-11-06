# Edit-commit performance: two scene-sized allocations removed

Status: **partial, unmeasured**. This note records two narrowly scoped
optimizations in the edit/commit path. It does **not** claim that the
`complex-test.dwg` zoom-in/draw-line/Enter freeze is fixed.

## The report this responds to

User-visible freeze scenario (2026-11, Linux `app-linux` / `native-below`):

1. open `complex-test.dwg`;
2. zoom into a detail;
3. draw a LINE;
4. press Enter to commit.

Nothing in this change alters the React DOM + wgpu architecture, the
`cad-db` authority model, or the renderer. The renderer-side commit cost
(pulled rebuild, pick-table updates, batch/pick scans, full stream-buffer
writes) is owned by a separate investigation and is **not** addressed here.

## 1. `cad-db`: entity-only batches no longer build a whole-scene block view

`crates/cad-db/src/drawing/block_edit.rs`
(`apply_drawing_changes_with_blocks`):

Before, every call built a `BTreeMap` over the entity batch **and** ran
`post_batch_entities`, which:

- walks every stored entity,
- collects a `Vec<&DbEntity>` of the whole scene minus the batch,
- `sort_by_key(draw_order)` over that `Vec` (`O(N log N)`).

That view exists solely to validate `block_changes` (member list, delete
safety). The ordinary edit commit — a draw/trim/move/grip update — passes
`block_changes = &[]`, so the whole-scene clone+sort was pure overhead per
commit, growing with the drawing size rather than with the edit.

Now the batch map and the post-batch view are built only when
`!block_changes.is_empty()`. An entity-only batch uses an empty staged-block
set and goes straight to the unchanged per-change
`ensure_change_allowed_on_locked_layer` + `validate_entity_in` chokepoint.

Unchanged on purpose:

- lock/identity validation still runs first and per entity;
- a non-empty block batch still validates member lists and delete safety
  against the full post-batch view;
- revision/history semantics (one bump, one change set) are untouched.

## 2. `cad-app`: create funnel takes the draw-order maximum in one pass

`crates/cad-app/src/app_drawing/trim_extend.rs` (`insert_new_entities`, the
shared create funnel used by CREATE, OFFSET, DIMCENTER, MTEXT, INSERT, ...):

Before:

```rust
drawing.model_space().iter().map(|e| e.draw_order).max()
```

`model_space()` allocates a `Vec` of every model entity and sorts it by
`draw_order` — `O(N log N)` plus a scene-sized allocation — even though the
caller only needs the maximum.

Now the same maximum is taken from a single pass over `drawing.entities()`
with the identical filter (`SpaceId::Model`). Semantics are deliberately
**unchanged**: paper-space and block-definition rows do not participate, even
when the new row is created in another space (OFFSET preserves the source
space). The base is still `max(model draw_order) + 1` (`saturating_add`), and
the per-batch `+ index` increment is unchanged.

## What is not claimed

- The freeze is **not** fixed or reproduced here. No renderer timing, GPU
  trace, or wall-clock measurement was taken.
- Renderer commit costs are out of scope: pulled rebuild / checkpoint reuse,
  pick-table rebuild and `replace_suffix`, GC, batch scans
  (`rebuild_lineweight_cache` etc.), and any full stream-buffer writes.
- `cad-db` remains authoritative and COW/copy-on-write is unchanged: the
  create funnel still clones the `Arc<DrawingDatabase>` (`as_ref().clone()`)
  before staging. That is a scene-sized copy and is not addressed here.
- No architectural or history/undo redesign.

## Regression tests added

`crates/cad-db/src/tests/block_edit.rs`:

- `an_entity_only_batch_inserts_updates_and_deletes_in_one_revision` — the
  guarded empty-block path still applies insert/update/delete in caller order
  with exactly one revision and no invented block row.
- `an_invalid_entity_only_batch_rolls_back_without_revision_or_allocator_moves`
  — missing-entity delete, duplicate entity id, and a dangling
  `SpaceId::Block` reference are refused with revision, allocators and tables
  unchanged (the skipped block view cannot admit a dangling reference).
- `a_combined_block_batch_is_still_validated_when_entities_are_staged` — a
  non-empty block batch is still checked against the post-batch member list
  (mismatch refused, unchanged), and the matching combined batch commits once.
- `assert_refused_unchanged` now also asserts that neither allocator advanced.

`crates/cad-app/src/tests/drawing/create_trim.rs`:

- `an_invalid_create_writes_no_revision_and_no_undo_entry` — a non-finite line
  endpoint is refused by `validate_geometry`, with the revision unchanged and
  **no history entry / undo step** recorded (the history map is populated
  lazily, so a refusal leaves it absent).

`crates/cad-app/src/tests/drawing/create_draw_order.rs` (new module; the
existing `create_trim.rs` was already at 969 lines and must stay under the
1000-line code-file limit):

- `create_line_draw_order_ignores_paper_block_rows_and_id_order` — model rows
  are stored in id order with unsorted draw orders (7/2/11); paper (900) and
  block (700) rows are present; the new line gets `12`.
- `offset_a_paper_space_entity_keeps_the_model_draw_order_base` — OFFSET of a
  paper-space line stays in paper space but still uses the model-space base
  (`12`), pinning that the optimization did not silently switch to
  target-space behaviour.

## Verification performed

Targeted only; all commands ran with `--locked`:

```bash
cargo test -p cad-db --locked block_edit
# 10 passed

cargo test -p cad-app --locked --lib create_draw_order
# 2 passed

cargo test -p cad-app --locked --lib create_trim
# 23 passed

rustfmt --edition 2021 --check <touched files>
# no diff
```

NOT RUN: full workspace test suite, clippy gate, `check-file-size.py`
(manually kept under the 1000-line limit: the new app contract test lives in
its own module `create_draw_order.rs`), renderer/GPU evidence, and any real
`complex-test.dwg` freeze reproduction or timing.

