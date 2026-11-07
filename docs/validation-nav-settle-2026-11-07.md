# Pan/zoom settle: run-free suffix skip, camera-order reuse, idle page budget

Status: **implemented + contract/GPU tests green + real-host before/after runs, including a 120 ms
navigation pause, a text-glyph navigation LOD, the actual-VRAM pulled pre-flight with a one-shot
warning, and per-tick input publication batching**. This is not a performance acceptance and not a
cross-platform claim.

Fixture `fixtures/test-tangent-1.dwg`: 170,757 entities, 167,125 resident base batches, **34.0M
triangles** (33.95M from 70,785 text glyph mesh batches), opaque/transparent = 167,064/0. The pulled
accelerated copy requires **2.699 GB**; the Quadro P620 reports `Out_of_Memory`, so the renderer runs
the universal per-stream fallback (`pulled=skipped-out-of-memory`). That fallback is expected on this
card and is **not** changed here.

## What changed

1. **Run-free suffix commits no longer rebuild the pulled/universal organisation.**
   `Renderer::try_skip_pulled_rebuild` now also admits the skip when the universal per-stream line
   fallback is known-valid (`line_fallback_built`), not only when the pulled state is coherent. A
   run-free suffix stages no instance/arc/mesh run and keeps the base prefix, so neither the pulled
   buffers nor the fallback can change; re-running a failed pulled build only re-ran
   `build_line_fallback` (measured **2.0–2.2 s**) and could not change a pixel.
2. **Opaque-only camera restarts reuse the base order/page index.** When the `FrameKey` changes and
   `DrawOrderCache::transparent().is_empty()`, `restart_progressive_frame` keeps `ordered`,
   `opaque_count`, `invisible` and `OrderedPageIndex` and resets only the cursor. Opaque order and the
   opaque/invisible partition are camera-independent; a base change drops the frame entirely.
   `progressive_frame` is now invalidated together with `draw_plan`/`batch_facts` on base change,
   device rebuild, device loss and re-initialisation. New counters:
   `progressive_order_reuses` / `progressive_order_rebuilds`.
3. **The settling walk uses the un-paced page ceiling.** When the camera settle is due (250 ms),
   `CadRenderRuntime::boost_page_budget()` raises the adaptive `scale` to `>= 1.0`; the measured cost
   EMA/seeded/target are retained and the controller shrinks again on two over-target frames.
   Rationale: camera motion had driven the scale to `0.05–0.1`, and the 8-under-frame slow-up could
   not recover during a single settle, so the full walk ran at ~10 % of the ceiling.
4. **The pacing signal uses the encode mask.** `accepted_with_work` now counts
   `EncodeEmitMask::participates` instead of dereferencing every accepted `GpuBatch` (a deep-zoom
   page accepts the whole ordered set while drawing a few runs; the scattered ~200 B reads were
   measurable). The mask is the same participation predicate the encode loop already uses.
5. **Wheel zoom no longer cancels the settle.** `set_overlays` is called in the same `sync_render` as
   the camera snapshot and carries the camera-derived `view_scale`. With a live cursor it used to take
   `bounds_only -> advance_content` and clear the just-armed settle on every notch. It now keeps the
   armed deferral (a standalone bounds-only overlay change with no armed settle still goes immediate).
6. **Navigation pause (120 ms, native only).** `State` gains `navigation_pause_pending` and a timeout
   separate from the 250 ms grid settle. A camera-only change arms it; a non-camera input or a content
   advance clears it. While armed, `render` draws **at most one progressive page per camera change**
   and marks the frame clean instead of continuing the walk between input events. A same-camera
   (cursor-only) invalidation uses `Renderer::render_retained_overlay`: it keeps `cursor > 0`, draws no
   base page, and only recomposites the transient overlay over the retained base. A camera/target
   change still restarts and draws page 1. After 120 ms of quiet the settle pump clears the pause and
   invalidates the frame so the walk resumes from the retained cursor. `Duration::ZERO` is the
   kill-switch; the pause is off on wasm.

## Navigation pause — final run

Two 8 s gestures on the final release (same fixture / P620 / Wayland / isolated `leftDragPans=true`
config). The driver is still a synthetic DOM primary drag, not a real OS mouse.

| metric | gesture 1 | gesture 2 |
|---|---:|---:|
| in-gesture samples with `nav_pause=true` | 216 / 217 | 200 / 201 |
| in-gesture actual renders | 217 (~27/s) | 201 (~25/s) |
| distinct view revisions | 215 | 200 |
| in-gesture `prog` cursor min / median / max | 1858 / **4428** / 167125 | 816 / **2879** / 167125 |
| settle tail renders | 24 | 47 |
| settle tail `budget_scale` median | 0.750 | 0.342 |
| `order_reuses` / `order_rebuilds` | 413 / 2 (whole run) | |
| `pulled_suffix_skips` | 4 (whole run) | |

The in-gesture `prog` median (2.9–4.4k of 167,125 batches) is the pause working: the walk is not
consuming pages between input events. The occasional `max=167125` is a page that accepted the whole
ordered set because the cull zero-charged it (no base work). The only frame over 1 s is the initial
base commit (2.98 s, `base_changed=true`, open/load); the largest **interaction** frame is 55 ms.
Settle tails are 24 / 47 renders instead of the 161-render tail measured before the pacing boost.

7. **Text-glyph navigation LOD.** `RenderBatch::is_text_glyph()` is derived from the representation
   primitive (`Mesh`, non-empty indices, empty edge overlay; only text shaping emits
   `MeshWithoutEdgeOverlay`). It is carried as `GpuBatch.text_glyph` and a `BatchFacts` bitset.
   `ChunkCull.hide_text` is a display policy: glyph batches charge 0 vertices/triangles, the encode
   mask drops their draws, and the pulled organisation is disabled while it is on. Pick keeps
   `ChunkCull::all_visible` (`hide_text=false`), so text stays pickable. `Renderer::set_navigation_lod`
   drops `progressive_frame` on a mode change; the engine turns it on with the navigation pause and
   off at settle, so the retained text-free colour is always repainted.
8. **Actual-VRAM pulled pre-flight.** `rebuild_pulled` now honors `GpuMemoryLedger::admit`'s refusal
   instead of testing the pulled requirement alone. It skips with
   `SkippedInsufficientVram { available = limit - (resident - replaced_pulled) }` and never attempts
   the allocation. `SkippedInsufficientVram.budget_bytes` is documented as the available budget.
   `CadRenderRuntime` latches a one-time `gpu.pulled.insufficient-vram: ...` warning on the pulled
   state transition and exposes it through `RuntimeDiagnostics`/`view_diagnostic`; the React notice
   (`src/host/notice.ts`, `HostNotice`) opens once, respects dismissal, and re-arms only after the
   token clears.
9. **Input publication batching.** `HostCore` has a render-batch depth and a deferred edit receipt.
   `CadSession::handle_inputs` admits/executes all inputs in order, then flushes one
   `sync_render`/`request_preparation`; `handle_input` keeps its per-call flush. Linux/Windows/macOS/
   Android web and `app-web` call the batched entry. Wheel deltas are never merged in Rust and
   pointer phases are never dropped.

## Final integrated run (glyph LOD + VRAM pre-flight + input batching)

Same release/host/fixture, two 8 s gestures, synthetic DOM primary drag. This is the final tree.

| metric | value |
|---|---:|
| initial base commit | **1.04 s** (was 2.7–3.0 s; the pulled copy is refused before allocating) |
| pulled verdict | `skipped-insufficient-vram(required=2,699,194,780, available=1,913,456,682)` |
| warning | `gpu_warning` / `cad_status.error` = `gpu.pulled.insufficient-vram: required=… available=…` |
| in-gesture `hide_text=true` | **219/225** and **214/215** samples |
| in-gesture `nav_pause=true` | 219/225 and 214/215 |
| in-gesture `prog` cursor median | **17,782 / 47,113** of 167,125; complete 103/225 and 100/215 |
| in-gesture page triangles median | **179k / 215k** (glyph triangles zero-charged) |
| settle tail | **22 / 22** frames, `hide_text=false`, `prog` 18,049→167,125 / 1,569→167,125 |
| settle `budget_scale` | 0.78 / 1.00 (boost) |
| frames > 100 ms | **1**: the 1.04 s initial base commit; interaction frames < 100 ms |
| `order_reuses / rebuilds` | 429 / 5 (LOD on/off drops a frame; camera changes reuse) |
| `pulled_suffix_skips` | 5 |
| in-gesture renders | 225 / 215 (218 / 214 distinct view revisions) |

`hide_text=true` removes the ~33.95M glyph triangles during motion; on culled pages the page can even
accept the whole ordered set. On settle the mode change drops the retained text-free frame and the
boosted walk repaints full text in ~22 pages. Pick/measurement use `all_visible`, so hidden text
remains pickable.

Input batching: the synthetic driver already coalesces pointer moves to one queued input per tick
(`queued=1`), so this run does not exercise a wheel burst. The contracts (`cad-host/render_batch`,
`cad-ui-core/tests_batch`) pin one publication for N inputs, the same final camera/overlays, and
pending-edit survival. `input_us` median ~6.8 ms is present in the pre-batching run too (pre-existing
snap/prepare cost), not a batching regression.

No real-window screenshot of hidden text / text reappearance; the LOD pixel fence is the
`navigation_lod` GPU test. The UI notice is covered by `npm test`/build + i18n; no screenshot.

## Measured before/after (real host)

Environment: release `app-linux`, Wayland, Quadro P620 / Vulkan 580.178.04, same fixture, isolated
`XDG_CONFIG_HOME` with `interaction.leftDragPans=true`, env-gated post-load synthetic DOM drag (not a
real OS mouse). Two 8 s gestures on the after binary; one 8 s gesture on the before binary.

| metric | before | after |
|---|---|---|
| in-gesture actual renders / 8 s | 92 (~11.5/s) | 206 / 196 (~25/s) |
| in-gesture `plan_ms` median | 1.87 ms | 0.53 ms |
| in-gesture adaptive `scale` median / min | 0.073 / 0.050 | gesture 1 0.392/0.229; gesture 2 0.243/0.144 |
| in-gesture `wait_ms` max | **339.5 ms** | 15.2 / 31.0 ms |
| frames > 2 s | **4** (2.34–2.71 s), every one a settle suffix commit | **1**: the initial base commit (2.697 s, `base_changed=true`, load, not interaction) |
| settle suffix `commit[pulled]` | 1.50–2.17 s | **0 µs** (`pulled_suffix_skips` +1 per settle; `commit total` 1.58 ms) |
| post-gesture walk (camera fixed) | 161 renders, scale median 0.100, `prog 4015 -> 167125` | 36 renders (gesture 2), scale median 0.456, `prog 5160 -> 167125` |
| post-gesture wall estimate | ~1.6 s | ~0.6 s |
| camera restarts | O(167,125) sweep per change (not counted) | `order_reuses=396`, `order_rebuilds=2` |

Immediately after the 250 ms settle: `budget_scale=1.000`, `budget_verts=8,000,000`,
`budget_tris=2,000,000`. The controller then shrank to 0.75 / 0.562 / 0.422 / 0.456 as pages exceeded
the 13 ms target. The boost is a one-shot policy step, not a cost-model reset.

The remaining one-time 2.697 s in that intermediate run is the **initial base commit**: pulled attempt
OOM (`pulled=1.487 s`) + fallback + pick. In the final integrated run the pre-flight refuses the
pulled copy before allocating, reducing the initial commit to **1.04 s** (see the final table above).
The P620 cannot hold the 2.699 GB pulled copy; the fallback remains explicit.

## Contracts / tests run

- `cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1` **321 passed / 4 ignored**,
  including `pulled_budget_gpu_tests` **6 passed** (pre-flight netting/refusal + OOM-fallback suffix
  skip), the LOD zero-charge/encode-mask contracts, and the pacing boost.
- `cargo test -p cad-render-wgpu --test headless_render --locked navigation_lod --
  --test-threads=1` **1 passed** (real P620: glyph pixels disappear during LOD, non-text stays, full
  text returns after the mode clears).
- `cargo test -p cad-render-wgpu --test headless_render --locked progressive_pages_match --
  --test-threads=1` **1 passed** (progressive pages equal the full scene; a transparent camera change
  rebuilds the order).
- `cargo test -p cad-render-wgpu --test progressive_restart_flash --locked -- --test-threads=1`
  **4 passed** (flag/suffix/retained-overlay pixel fences).
- `cargo test -p cad-engine --lib --locked` **121 passed / 15 ignored**; includes the 3 navigation
  pause contracts, the 2 `gpu_warning` transition/never-overwrite contracts, and the zoom
  `view_scale` settle fence.
- `cargo test -p cad-host --lib --locked` **270 passed**, including `render_batch` **3 passed**
  (deferral, nesting, pending-edit survival) and the live-cursor wheel-zoom settle fence.
- `cargo test -p cad-ui-core --lib --locked` **387 passed / 1 ignored**, including `tests_batch`
  **3 passed** (batch vs individual same camera/overlays, one `view_revision` for N moves, one
  publication, first-error/flush-error, per-call `handle_input` flush).
- `cargo test -p app-android --lib --locked` **48 passed** (updated batch call-site contract);
  `cargo test -p app-web --lib --locked` passed; Linux/Web/Android target checks passed.
- `(cd apps/desktop-ui && npm test && npm run build)` **1137 tests passed / build OK**; the one-shot
  GPU-fallback notice store and i18n key parity are covered.
- `cargo clippy` (`cad-scene`, `cad-representation`, `cad-render-wgpu`, `cad-engine`, `cad-host`,
  `cad-ui-core`, `app-linux`, `app-web`) **-D warnings 0**; `cargo fmt --all -- --check`,
  `check-file-size`, `check-architecture`, `check-i18n` (1102 keys) **exit 0**.
- Full workspace gates (`cargo test --workspace`, full CI) and Windows/macOS real-target
  compile/run **NOT RUN** (Windows `llvm-rc` / macOS `cc -arch` toolchains unavailable).

## Honest boundaries

- Synthetic DOM primary drag with `leftDragPans=true`, not a real OS pointer/trackpad; the gesture is a
  bounded evidence driver, not user acceptance.
- One machine, one adapter, one run each side. No GPU timestamps (`gpu_ms=None`); `frame_us`,
  `cpu_ms` and `wait_ms` are host/renderer wall times, not GPU execution time.
- The after run's "post-gesture" window is the second gesture's tail; the first tail overlaps the
  driver re-arming the next gesture, so it is not reported as a completion time.
- No screenshot/pixel judgement beyond the existing GPU pixel fences; no Windows/macOS/Android/Web
  runtime evidence; no performance acceptance.
