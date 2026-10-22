/* Private native Wayland protocol shim for the app-linux host.
 *
 * The host needs an **app-owned** child `wl_surface` + `wl_subsurface` layered
 * under the GDK toplevel's `wl_surface`, but GDK exposes no public getter for
 * `wl_subcompositor` and libwayland ships the protocol wrappers as header-only
 * inline functions (not dynamic exports). This tiny C translation unit wraps the
 * system `wayland-client-protocol.h` wrappers so the Rust side never re-implements
 * raw proxy marshalling.
 *
 * Hard rules (see `.slim/deepwork/native-wayland/design-inputs.md`):
 * - The `wl_display` is **borrowed** from GDK. This shim never creates, flushes,
 *   dispatches, round-trips or destroys a display/connection. GTK remains the sole
 *   dispatcher of the shared default event queue.
 * - Only `wl_compositor` + `wl_subcompositor` are bound (with version checks); the
 *   GDK-owned parent `wl_surface` is only used as the subsurface parent and is
 *   never committed, scaled, re-regioned or destroyed here.
 * - Every failure is explicit (facts + reason string); there is no silent success
 *   and no abort on a missing global.
 *
 * This shim deliberately makes **no readiness decision**: it reports raw facts
 * (hard-failed, discovery-done, which globals are bound, their versions) and the
 * Rust wrapper classifies them. That keeps the state machine unit-testable
 * without a live protocol connection.
 */

#ifndef YACR_WAYLAND_SHIM_H
#define YACR_WAYLAND_SHIM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct wl_display;
struct wl_surface;

/* Opaque bridge state. All callbacks run from the owner's (GTK's) dispatch. */
typedef struct yacr_wl_shim yacr_wl_shim;

/* Create the shim over a borrowed `wl_display`.
 *
 * Queues `wl_display_get_registry` followed by `wl_display_sync` (the discovery
 * barrier: every initial global is delivered before the sync `done`) and installs
 * asynchronous listeners. It does **not** flush, dispatch or round-trip; the
 * globals and `done` are only delivered when the owner (GTK) next dispatches the
 * shared default queue.
 *
 * Returns NULL on allocation failure or a NULL display. */
yacr_wl_shim *yacr_wl_shim_new(struct wl_display *display);

/* Tear down: destroys a still-pending discovery callback proxy (before the
 * userdata is freed), then any child `wl_surface`/`wl_subsurface`, then the owned
 * registry/compositor/subcompositor proxies (the registry destroy removes its
 * listener association). The borrowed display and parent surface are never
 * touched. */
void yacr_wl_shim_free(yacr_wl_shim *shim);

/* 1 if a C/protocol-level failure was recorded (bind/listener failure, or a
 * removal of a global we bound), 0 otherwise. A **missing** global is not a hard
 * failure: it is reported through `discovery_done` + `has_*` and classified by
 * Rust. */
int yacr_wl_shim_hard_failed(const yacr_wl_shim *shim);

/* Failure reason; never NULL (empty string while not failed). */
const char *yacr_wl_shim_failure(const yacr_wl_shim *shim);

/* 1 once the initial global burst has been delivered (`wl_display_sync` `done`). */
int yacr_wl_shim_discovery_done(const yacr_wl_shim *shim);

/* The borrowed `wl_display`'s error state (`wl_display_get_error`): 0 when fine,
 * a nonzero errno after a fatal protocol/display error. */
int yacr_wl_shim_display_error(const yacr_wl_shim *shim);

int yacr_wl_shim_has_compositor(const yacr_wl_shim *shim);
int yacr_wl_shim_has_subcompositor(const yacr_wl_shim *shim);
uint32_t yacr_wl_shim_compositor_version(const yacr_wl_shim *shim);
uint32_t yacr_wl_shim_subcompositor_version(const yacr_wl_shim *shim);

/* Create the app-owned child `wl_surface` + `wl_subsurface` under `parent`.
 *
 * Requires both globals bound with a compositor version >= 3. On success the child
 * is configured for desync, an integer `buffer_scale`, the logical position and an
 * **empty input region** (click-through; `NULL` would mean whole-surface input).
 *
 * The **position is parent-relative double-buffered state and is applied on the
 * parent's next commit** — the bridge requests a GTK toplevel `queue_draw` for
 * that; it never commits the GDK surface itself. The child's own commit (performed
 * by the Vulkan swapchain present) carries only its buffer and integer
 * buffer-scale. Returns 0 on success, -1 on any error (with a failure reason
 * recorded). `below_parent` is only enabled by the opt-in DOM-on-top probe;
 * stacking, like position, takes effect on the GTK-owned parent's commit. */
int yacr_wl_shim_create_child(yacr_wl_shim *shim, struct wl_surface *parent,
			      int32_t x, int32_t y, int32_t buffer_scale, int below_parent);

/* Owned child `wl_surface` (NULL until created or after destroy). */
struct wl_surface *yacr_wl_shim_child_surface(const yacr_wl_shim *shim);

/* Destroy the child `wl_surface` + `wl_subsurface` (idempotent). Must be called
 * after the GPU surface is released and before the GDK parent is unrealized. */
void yacr_wl_shim_destroy_child(yacr_wl_shim *shim);

/* Update double-buffered surface state (no commit): `set_position` is
 * parent-relative and applied on the parent commit; `set_buffer_scale` is child
 * state applied on the child commit. Return 0/-1. */
int yacr_wl_shim_set_position(yacr_wl_shim *shim, int32_t x, int32_t y);
int yacr_wl_shim_set_buffer_scale(yacr_wl_shim *shim, int32_t scale);
int yacr_wl_shim_set_empty_input(yacr_wl_shim *shim);

#ifdef __cplusplus
}
#endif

#endif /* YACR_WAYLAND_SHIM_H */
