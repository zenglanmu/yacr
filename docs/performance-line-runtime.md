# LINE commit: incremental render-window plumbing (partial, unmeasured)

Status: **plumbing + bounded runtime admission only**. This does not claim the
reported `complex-test.dwg` zoom/draw/Enter freeze is fixed. No real-GPU,
pixel, wall-clock or user-input evidence was produced by this change.

## What is built

`DrawingEditReceipt` (owned by `cad-app`) is constructed only by the trusted
command funnel (`HostController::confirm_draw_traced_with_edit` /
`execute_traced_with_edit`) from the exact before/after `Arc<DrawingDatabase>`
and the actual `RawCommandTrace`. It is an opaque proof of a successful,
single `CreateLine` insert into model space; callers cannot build one from an
arbitrary insert list.

Host (`cad-host`):

- `confirm_tool` uses `confirm_draw_traced_with_edit`; the receipt is forwarded
  atomically with the new drawing through `sync_render_with_edit`.
- All other syncs call `sync_render` (`None`), so no receipts are invented.
- A failed confirmation supplies no receipt, keeps the capture and leaves the
  database/revision untouched.

Engine (`cad-engine::render`):

- The session retains the latest receipt. A same-`Arc` sync keeps it (a later
  post-commit sync cannot erase it), a new/unrelated drawing clears it, and a
  receipt that arrives after the drawing publication advances only the
  preparation content revision so the append hint is never silently dropped.
- Native worker, synchronous and wasm preparation all carry the receipt into
  `prepare_shared_with_overlays_and_images_and_edit`. The latest mailbox does
  not consume it; overlay-only submissions leave it intact until the drawing
  target changes.

Runtime (`cad-engine::render::runtime`):

- The exact committed base `SceneDelta` `Arc` is retained (reset on detach).
- A `BaseAppend` is admitted only when: the device is ready, the base revision
  changed, the predecessor is pointer-identical to the retained committed base,
  `base_batches == keep_batches`, and at least one new base batch exists.
- On admission only `base[keep_batches..]` is staged (group 0 starts at
  `keep_batches`), followed by the complete current highlight. Commit keeps the
  resident prefix and sets `base_end` to the complete new base count, so the
  renderer classifies it as a base change (not an overlay-only suffix).
- Any failed guard falls back to a full replacement. A pending upload for a
  superseded revision is reset and re-planned, so a stale append can never mix.
- An admitted append does not re-upload images or wipeout masks and preserves
  their earlier diagnostics. Entity flags are applied after the commit as
  before, so new pick keys and preview removal stay correct.
- The full `PreparedScene` is retained in the pending upload, so a device
  reattach/restore still performs a complete upload.

## Remaining costs (explicitly not addressed)

- `cad-app` still clones the whole `DrawingDatabase` (COW copy) per commit.
- The CPU base-scene append still copies the predecessor batch `Vec` prefix in
  the controller's representation; this change removes the GPU re-upload and
  renderer-side prefix work, not every CPU allocation.
- Renderer GC, pulled-segment rebuild/checkpoint, pick-table work and the base
  progressive-plan reset are unchanged. The append is a real base change for
  the renderer, so those base costs still apply in proportion to the appended
  window; they are not implemented incrementally here.
- No fixture timing, GPU trace, screenshot or real-device run is included.

## Contract tests added

- `cad-engine` runtime: `base_append_admission_requires_the_exact_resident_predecessor`,
  `base_append_plan_drives_prefix_and_classification`.
- `cad-engine` session: receipt retention across overlay/same-`Arc` syncs, clear
  on a new drawing, and late-receipt rescheduling (the prepared scene carries
  `base_append`).
- `cad-host`: successful LINE confirm supplies a receipt; failed confirm keeps
  the capture and supplies none.

GPU upload/commit behaviour (prefix retention, image/mask retention, fallback
after a device change) is covered by the runtime admission contract and must be
exercised by the parent's hardware GPU run; it is **NOT RUN** here.
