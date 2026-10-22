/* Private native Wayland protocol shim for the app-linux host.
 * See `wayland_shim.h` for the contract and the hard rules.
 *
 * Threading: every entry point must be called from the single owner thread that
 * also dispatches the borrowed display (the GTK main thread). libwayland proxies
 * are not thread-safe and are never shared here.
 */

#include "wayland_shim.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#include <wayland-client.h>
#include <wayland-client-protocol.h>

/* `wl_surface.set_buffer_scale` needs wl_surface v3; `damage_buffer` needs v4.
 * We cap the compositor bind at 4 and require at least 3. */
#define YACR_WL_COMPOSITOR_MAX_VERSION 4u
#define YACR_WL_COMPOSITOR_MIN_VERSION 3u
#define YACR_WL_SUBCOMPOSITOR_VERSION 1u

struct yacr_wl_shim {
	/* Borrowed, never destroyed by this shim. */
	struct wl_display *display;

	/* Owned proxies. */
	struct wl_registry *registry;
	struct wl_compositor *compositor;
	struct wl_subcompositor *subcompositor;

	uint32_t compositor_version;
	uint32_t subcompositor_version;

	/* Global names, so a `global_remove` can be attributed to our globals. */
	uint32_t compositor_name;
	uint32_t subcompositor_name;
	int have_compositor_name;
	int have_subcompositor_name;

	/* Discovery barrier: the registry request is queued before this sync, so its
	 * `done` means every initial global has already been delivered. */
	struct wl_callback *sync_callback;
	int discovery_done;

	int hard_failed;
	char failure[256];

	/* Owned child surface tree. */
	struct wl_surface *child;
	struct wl_subsurface *subsurface;

	/* Kept alive for the lifetime of `registry`; removed with it. */
	struct wl_registry_listener registry_listener;
	struct wl_callback_listener sync_listener;
};

static void yacr_set_failed(yacr_wl_shim *shim, const char *why)
{
	if (shim == NULL)
		return;
	if (!shim->hard_failed) {
		shim->hard_failed = 1;
		snprintf(shim->failure, sizeof(shim->failure), "%s",
			 why != NULL ? why : "wayland shim failure");
	}
}

/* wl_surface events for the child surface: all four handlers are explicit no-ops
 * so there is never an unhandled `enter`/`leave` (the surface listener array must
 * match the interface's event count). We deliberately do not honour the
 * compositor's preferred buffer scale/transform: the host drives an integer
 * buffer scale from GTK's integer scale factor (fractional scale is not
 * verified). */
static void yacr_child_surface_enter(void *data, struct wl_surface *surface,
				     struct wl_output *output)
{
	(void)data;
	(void)surface;
	(void)output;
}

static void yacr_child_surface_leave(void *data, struct wl_surface *surface,
				     struct wl_output *output)
{
	(void)data;
	(void)surface;
	(void)output;
}

static void yacr_child_surface_preferred_buffer_scale(void *data,
						      struct wl_surface *surface,
						      int32_t factor)
{
	(void)data;
	(void)surface;
	(void)factor;
}

static void yacr_child_surface_preferred_buffer_transform(void *data,
							  struct wl_surface *surface,
							  uint32_t transform)
{
	(void)data;
	(void)surface;
	(void)transform;
}

static const struct wl_surface_listener YACR_CHILD_SURFACE_LISTENER = {
	.enter = yacr_child_surface_enter,
	.leave = yacr_child_surface_leave,
	.preferred_buffer_scale = yacr_child_surface_preferred_buffer_scale,
	.preferred_buffer_transform = yacr_child_surface_preferred_buffer_transform,
};

static void yacr_registry_global(void *data, struct wl_registry *registry,
				 uint32_t name, const char *interface,
				 uint32_t version)
{
	yacr_wl_shim *shim = data;

	if (shim->hard_failed)
		return;

	if (strcmp(interface, wl_compositor_interface.name) == 0 &&
	    shim->compositor == NULL) {
		uint32_t bind_version =
			version < YACR_WL_COMPOSITOR_MAX_VERSION ? version
								 : YACR_WL_COMPOSITOR_MAX_VERSION;
		shim->compositor = wl_registry_bind(
			registry, name, &wl_compositor_interface, bind_version);
		if (shim->compositor == NULL) {
			yacr_set_failed(shim, "wl_registry_bind(wl_compositor) failed");
			return;
		}
		shim->compositor_version = bind_version;
		shim->compositor_name = name;
		shim->have_compositor_name = 1;
	} else if (strcmp(interface, wl_subcompositor_interface.name) == 0 &&
		   shim->subcompositor == NULL) {
		shim->subcompositor = wl_registry_bind(
			registry, name, &wl_subcompositor_interface,
			YACR_WL_SUBCOMPOSITOR_VERSION);
		if (shim->subcompositor == NULL) {
			yacr_set_failed(shim,
					"wl_registry_bind(wl_subcompositor) failed");
			return;
		}
		shim->subcompositor_version = YACR_WL_SUBCOMPOSITOR_VERSION;
		shim->subcompositor_name = name;
		shim->have_subcompositor_name = 1;
	}
}

static void yacr_registry_global_remove(void *data, struct wl_registry *registry,
					uint32_t name)
{
	yacr_wl_shim *shim = data;

	(void)registry;

	/* Only a removal of a global we actually bound invalidates the bridge
	 * (the protocol gives us the name, not the interface). */
	if ((shim->have_compositor_name && name == shim->compositor_name) ||
	    (shim->have_subcompositor_name && name == shim->subcompositor_name)) {
		yacr_set_failed(
			shim,
			"wayland global removed (wl_compositor/wl_subcompositor invalidated)");
	}
}

/* Discovery barrier: fires once the server has processed all requests queued
 * before `wl_display_sync`, i.e. the whole initial registry burst. The callback
 * proxy is destroyed here (it is a one-shot, terminal event) before any userdata
 * free could observe it. */
static void yacr_sync_done(void *data, struct wl_callback *callback,
			   uint32_t callback_data)
{
	yacr_wl_shim *shim = data;

	(void)callback_data;

	if (shim->sync_callback == callback)
		shim->sync_callback = NULL;
	wl_callback_destroy(callback);
	shim->discovery_done = 1;
}

static const struct wl_callback_listener YACR_SYNC_LISTENER = {
	.done = yacr_sync_done,
};

yacr_wl_shim *yacr_wl_shim_new(struct wl_display *display)
{
	yacr_wl_shim *shim;

	if (display == NULL)
		return NULL;

	shim = calloc(1, sizeof(*shim));
	if (shim == NULL)
		return NULL;

	shim->display = display;
	shim->registry_listener.global = yacr_registry_global;
	shim->registry_listener.global_remove = yacr_registry_global_remove;
	shim->sync_listener.done = yacr_sync_done;

	shim->registry = wl_display_get_registry(display);
	if (shim->registry == NULL) {
		yacr_set_failed(shim, "wl_display_get_registry failed");
		return shim;
	}
	if (wl_registry_add_listener(shim->registry, &shim->registry_listener,
				     shim) != 0) {
		yacr_set_failed(shim, "wl_registry_add_listener failed");
	}

	shim->sync_callback = wl_display_sync(display);
	if (shim->sync_callback == NULL) {
		yacr_set_failed(shim, "wl_display_sync failed");
		return shim;
	}
	if (wl_callback_add_listener(shim->sync_callback, &YACR_SYNC_LISTENER,
				     shim) != 0) {
		wl_callback_destroy(shim->sync_callback);
		shim->sync_callback = NULL;
		yacr_set_failed(shim, "wl_callback_add_listener failed");
	}

	/* No flush/dispatch/roundtrip: the owner (GTK) flushes the shared display
	 * and dispatches the default queue, which delivers our registry events and
	 * the sync `done`. */
	return shim;
}

void yacr_wl_shim_destroy_child(yacr_wl_shim *shim)
{
	if (shim == NULL)
		return;
	/* Destroy the subsurface first (it references the surface), then the
	 * surface itself. The borrowed parent is untouched. */
	if (shim->subsurface != NULL) {
		wl_subsurface_destroy(shim->subsurface);
		shim->subsurface = NULL;
	}
	if (shim->child != NULL) {
		wl_surface_destroy(shim->child);
		shim->child = NULL;
	}
}

void yacr_wl_shim_free(yacr_wl_shim *shim)
{
	if (shim == NULL)
		return;

	/* A still-pending discovery callback must be destroyed before the userdata
	 * (`shim`) is freed, so a later `done` can never reach freed memory. */
	if (shim->sync_callback != NULL) {
		wl_callback_destroy(shim->sync_callback);
		shim->sync_callback = NULL;
	}

	yacr_wl_shim_destroy_child(shim);

	/* Destroying the registry proxy also drops its listener association, so no
	 * event can reach `shim` after this point. */
	if (shim->registry != NULL) {
		wl_registry_destroy(shim->registry);
		shim->registry = NULL;
	}
	if (shim->subcompositor != NULL) {
		wl_subcompositor_destroy(shim->subcompositor);
		shim->subcompositor = NULL;
	}
	if (shim->compositor != NULL) {
		wl_compositor_destroy(shim->compositor);
		shim->compositor = NULL;
	}

	free(shim);
}

int yacr_wl_shim_hard_failed(const yacr_wl_shim *shim)
{
	return shim == NULL ? 1 : shim->hard_failed;
}

const char *yacr_wl_shim_failure(const yacr_wl_shim *shim)
{
	if (shim == NULL)
		return "wayland shim is null";
	return shim->failure;
}

int yacr_wl_shim_discovery_done(const yacr_wl_shim *shim)
{
	return shim == NULL ? 0 : shim->discovery_done;
}

int yacr_wl_shim_display_error(const yacr_wl_shim *shim)
{
	if (shim == NULL || shim->display == NULL)
		return -1;
	return wl_display_get_error(shim->display);
}

int yacr_wl_shim_has_compositor(const yacr_wl_shim *shim)
{
	return shim != NULL && shim->compositor != NULL;
}

int yacr_wl_shim_has_subcompositor(const yacr_wl_shim *shim)
{
	return shim != NULL && shim->subcompositor != NULL;
}

uint32_t yacr_wl_shim_compositor_version(const yacr_wl_shim *shim)
{
	return shim == NULL ? 0 : shim->compositor_version;
}

uint32_t yacr_wl_shim_subcompositor_version(const yacr_wl_shim *shim)
{
	return shim == NULL ? 0 : shim->subcompositor_version;
}

int yacr_wl_shim_set_position(yacr_wl_shim *shim, int32_t x, int32_t y)
{
	if (shim == NULL || shim->subsurface == NULL)
		return -1;
	/* Parent-relative double-buffered state: applied on the parent commit. */
	wl_subsurface_set_position(shim->subsurface, x, y);
	return 0;
}

int yacr_wl_shim_set_buffer_scale(yacr_wl_shim *shim, int32_t scale)
{
	if (shim == NULL || shim->child == NULL)
		return -1;
	if (scale < 1)
		return -1;
	/* Child state: applied on the child commit. */
	wl_surface_set_buffer_scale(shim->child, scale);
	return 0;
}

int yacr_wl_shim_set_empty_input(yacr_wl_shim *shim)
{
	struct wl_region *region;

	if (shim == NULL || shim->child == NULL || shim->compositor == NULL)
		return -1;

	/* An empty region (no rectangles) makes the surface click-through to the
	 * surfaces below. A NULL region would mean whole-surface input. */
	region = wl_compositor_create_region(shim->compositor);
	if (region == NULL)
		return -1;
	wl_surface_set_input_region(shim->child, region);
	/* Requests are processed in order, so the compositor has already copied the
	 * (empty) region into the surface state by the time this destroy is seen. */
	wl_region_destroy(region);
	return 0;
}

int yacr_wl_shim_create_child(yacr_wl_shim *shim, struct wl_surface *parent,
			      int32_t x, int32_t y, int32_t buffer_scale)
{
	struct wl_surface *child;
	struct wl_subsurface *subsurface;

	if (shim == NULL)
		return -1;
	if (shim->hard_failed)
		return -1;
	if (shim->compositor == NULL || shim->subcompositor == NULL) {
		yacr_set_failed(shim, "wayland globals not bound");
		return -1;
	}
	if (shim->compositor_version < YACR_WL_COMPOSITOR_MIN_VERSION) {
		yacr_set_failed(
			shim,
			"wl_compositor version < 3 (no wl_surface.set_buffer_scale)");
		return -1;
	}
	if (parent == NULL) {
		yacr_set_failed(shim, "parent wl_surface is NULL");
		return -1;
	}
	if (shim->child != NULL) {
		yacr_set_failed(shim, "child wl_surface already exists");
		return -1;
	}

	child = wl_compositor_create_surface(shim->compositor);
	if (child == NULL) {
		yacr_set_failed(shim, "wl_compositor_create_surface failed");
		return -1;
	}
	/* The child is app-owned: it must never alias the GDK parent surface. */
	if (child == parent) {
		wl_surface_destroy(child);
		yacr_set_failed(shim, "child wl_surface equals the GDK parent surface");
		return -1;
	}

	subsurface = wl_subcompositor_get_subsurface(shim->subcompositor, child,
						     parent);
	if (subsurface == NULL) {
		wl_surface_destroy(child);
		yacr_set_failed(shim, "wl_subcompositor_get_subsurface failed");
		return -1;
	}

	if (wl_surface_add_listener(child, &YACR_CHILD_SURFACE_LISTENER, NULL) != 0) {
		wl_subsurface_destroy(subsurface);
		wl_surface_destroy(child);
		yacr_set_failed(shim, "wl_surface_add_listener failed");
		return -1;
	}

	shim->child = child;
	shim->subsurface = subsurface;

	/* Desync: the child commits independently of the parent. */
	wl_subsurface_set_desync(subsurface);
	/* Always layer the CAD child below the GDK toplevel: the WebView chrome is
	 * the parent surface and must stay on top. Never attach/commit a buffer to
	 * the borrowed parent: GTK owns its alpha, painting, opaque region and
	 * commit. */
	wl_subsurface_place_below(subsurface, parent);
	/* Parent-relative position: applied on the parent's next commit (the bridge
	 * requests a GTK toplevel queue_draw). */
	wl_subsurface_set_position(subsurface, x, y);
	/* Child buffer scale: applied on the child's next commit (Vulkan present). */
	if (buffer_scale >= 1)
		wl_surface_set_buffer_scale(child, buffer_scale);

	if (yacr_wl_shim_set_empty_input(shim) != 0) {
		yacr_wl_shim_destroy_child(shim);
		yacr_set_failed(shim, "wl_compositor_create_region failed");
		return -1;
	}

	return 0;
}

struct wl_surface *yacr_wl_shim_child_surface(const yacr_wl_shim *shim)
{
	return shim == NULL ? NULL : shim->child;
}
