//! NVIDIA + Wayland workaround for WebKitGTK 2.54.x explicit-sync crashes.
//!
//! **Symptom**: on a real 5K/`scale=2` NVIDIA Wayland host the Flatpak
//! (`org.gnome.Platform//51`) dies at startup or when the window moves to the
//! 2× output with
//!
//! ```text
//! Gdk-Message: ... Error 71 (Protocol error) dispatching to Wayland display
//! ```
//!
//! **Root cause**: the runtime currently ships **WebKitGTK 2.54.1** (verified in
//! the sandbox via `webkit_get_major_version`). Its DMA-BUF renderer enables
//! Wayland explicit sync (`wp_linux_drm_syncobj`) but commits buffers without a
//! `set_acquire_point`; Mutter rejects that commit with
//! `wl_display.error(..., 4, no_acquire_point)` and GDK aborts the client. Host
//! WebKitGTK 2.52.6 does not reproduce it. Upstream: WebKit bug 280210.
//!
//! **Why this is explicit and narrow**: the workaround only runs when the
//! resolved backend is native Wayland, an NVIDIA driver is actually present, and
//! the user has not already chosen a WebKit/NVIDIA behaviour themselves. It
//! keeps the DMA-BUF renderer and acceleration (unlike
//! `WEBKIT_DISABLE_DMABUF_RENDERER=1`) and always logs one stable `[probe]` line,
//! so no crash mitigation happens invisibly. X11/XWayland and non-NVIDIA hosts
//! are untouched. `WEBKIT_DISABLE_DMABUF_RENDERER=1` remains the documented
//! fallback if the user wants it.
//!
//! This is a workaround for a confirmed trigger, **not** a claim that the
//! `scale=1`→`scale=2` 5K setup is verified: it is still pending a re-run on the
//! failing real machine (see `docs/flatpak.md` §6).

use crate::backend::ResolvedBackend;

/// Variables a user (or a packaging layer) can set to pick their own behaviour.
/// When any of them is present the quirk must not override that choice.
const OVERRIDE_VARS: [&str; 4] = [
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "__NV_DISABLE_EXPLICIT_SYNC",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
    "WEBKIT_DMABUF_RENDERER_FORCE_SHM",
];

/// Apply the workaround for the resolved backend.
///
/// Must be called **before** any GTK/WebKit/EGL initialization (in `entry::run`
/// right after the backend policy is resolved): the environment is read by the
/// GL/EGL stack and WebKit when they first initialize.
pub(crate) fn apply(resolved: ResolvedBackend) {
    let wayland = resolved == ResolvedBackend::Wayland;
    let nvidia = nvidia_driver_present();
    // Stay silent unless both conditions hold: there is nothing to work around
    // on X11 and nothing to work around without an NVIDIA driver.
    if !wayland || !nvidia {
        return;
    }
    let override_present = override_present_from(|name| std::env::var_os(name).is_some());
    if needs_workaround(wayland, nvidia, override_present) {
        // Edition 2021: `set_var` is still a safe call. This runs before the
        // renderer threads exist, so no concurrent environment access.
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        eprintln!(
            "[probe] nvidia-quirk: __NV_DISABLE_EXPLICIT_SYNC=1 \
             reason=wayland+nvidia+webkit-explicit-sync (upstream WebKitGTK bug 280210)"
        );
    } else {
        eprintln!("[probe] nvidia-quirk: not applied (webkit/nvidia env override present)");
    }
}

/// Pure decision function (unit-tested without touching the process env).
fn needs_workaround(wayland: bool, nvidia: bool, override_present: bool) -> bool {
    wayland && nvidia && !override_present
}

/// Whether any of [`OVERRIDE_VARS`] satisfies the lookup (pure; testable).
fn override_present_from(is_set: impl Fn(&str) -> bool) -> bool {
    OVERRIDE_VARS.iter().any(|name| is_set(name))
}

/// Whether the running kernel exposes the NVIDIA driver (visible inside the
/// Flatpak sandbox; verified).
fn nvidia_driver_present() -> bool {
    let text = std::fs::read_to_string("/proc/driver/nvidia/version").ok();
    nvidia_present_from(text.as_deref())
}

/// Pure interpretation of `/proc/driver/nvidia/version` (missing/blank = absent).
fn nvidia_present_from(text: Option<&str>) -> bool {
    text.is_some_and(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_on_wayland_with_nvidia_and_no_override() {
        assert!(needs_workaround(true, true, false));
    }

    #[test]
    fn never_applies_on_x11() {
        assert!(!needs_workaround(false, true, false));
    }

    #[test]
    fn never_applies_without_nvidia() {
        assert!(!needs_workaround(true, false, false));
    }

    #[test]
    fn every_override_var_disables_the_workaround() {
        for name in OVERRIDE_VARS {
            // Only this one variable is "set"; the lookup must see it.
            let present = override_present_from(|candidate| candidate == name);
            assert!(present, "override var {name} must be recognized");
            assert!(
                !needs_workaround(true, true, present),
                "override var {name} must disable the workaround"
            );
        }
    }

    #[test]
    fn nvidia_version_probe_requires_non_blank_text() {
        assert!(!nvidia_present_from(None));
        assert!(nvidia_present_from(Some(
            "NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.95.05\n"
        )));
        assert!(!nvidia_present_from(Some("   \n\t ")));
    }
}
