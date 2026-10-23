//! Window styles and probe knobs for the CAD child (pure, testable).
//!
//! Split out of `win32_surface.rs` to stay under the code-file size limit.

use super::*;

/// Whether `YACR_PROBE_NO_CLIPSIBLINGS` is present (diagnostic only).
///
/// RC2 discriminator: MSDN says a child window may clip the visual tree, and all
/// composition layers are clipped to the window's visible region. Dropping
/// `WS_CLIPSIBLINGS` tests whether the full-client `WRY_WEBVIEW` sibling is what
/// hides the CAD content. Read once per process (a creation-time style).
pub(super) fn child_no_clipsiblings_requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("YACR_PROBE_NO_CLIPSIBLINGS").is_some())
}

/// Whether `YACR_PROBE_GDI_MARKER` is present (diagnostic only).
///
/// V1 calibrator: a GDI fill in the child's paint path, to establish whether any
/// child-window content composites/appears at all in a given environment.
pub(super) fn gdi_marker_requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("YACR_PROBE_GDI_MARKER").is_some())
}

/// The CAD child's `WS_*` style: child, visible, `WS_DISABLED` (display-only;
/// the DOM is the single OS-input funnel) and — unless the RC2 diagnostic knob is
/// present — `WS_CLIPSIBLINGS`.
pub(super) fn child_style(no_clipsiblings: bool) -> WINDOW_STYLE {
    let base = WS_CHILD | WS_VISIBLE | WS_DISABLED;
    if no_clipsiblings {
        base
    } else {
        base | WS_CLIPSIBLINGS
    }
}

/// The CAD child's `CreateWindowExW` ex-style.
///
/// `WS_EX_NOREDIRECTIONBITMAP` is a **creation-time-only** ex-style: setting it
/// afterwards fails `SetWindowLongPtrW` with `ERROR_INVALID_PARAMETER`. The
/// top-level window therefore requests it at creation through Tauri's builder
/// (verified by [`no_redirection_bitmap_state`]); this child knob requests the
/// same flag for the CAD child, which Tauri does not create. Presence-based diagnostic knob `YACR_PROBE_CHILD_NO_REDIRECT`
/// (same family as `YACR_PROBE_NO_MAXIMIZE`: the variable's *existence* enables
/// it); without it the ex-style is `WINDOW_EX_STYLE::default()`, unchanged.
pub(super) fn child_ex_style(no_redirect: bool) -> WINDOW_EX_STYLE {
    if no_redirect {
        WINDOW_EX_STYLE::default() | WS_EX_NOREDIRECTIONBITMAP
    } else {
        WINDOW_EX_STYLE::default()
    }
}

/// Whether `YACR_PROBE_CHILD_NO_REDIRECT` is present (diagnostic only).
///
/// Read once per process: the knob is a startup decision (the ex-style can only
/// be set at creation), never a per-frame branch.
pub(super) fn child_no_redirect_requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("YACR_PROBE_CHILD_NO_REDIRECT").is_some())
}

/// Whether `YACR_PROBE_SOLID_FRAME` is present (diagnostic only).
///
/// Read once per process and cached: `frame_cad` consults it every tick, and an
/// environment lookup per frame would be pure overhead for a startup decision.
pub(super) fn solid_frame_requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("YACR_PROBE_SOLID_FRAME").is_some())
}

/// Report whether the creation-time `WS_EX_NOREDIRECTIONBITMAP` request took
/// effect, read back from the live child window.
pub(super) fn log_child_ex_style(cad: HWND) {
    let applied = unsafe { GetWindowLongPtrW(cad, GWL_EXSTYLE) } as u32;
    if applied & WS_EX_NOREDIRECTIONBITMAP.0 != 0 {
        eprintln!("[probe] win32 cad-child ex-style: WS_EX_NOREDIRECTIONBITMAP set");
    } else {
        // The window was created, so `GetLastError` is not authoritative here;
        // the read-back is what proves the style did not stick.
        let error = unsafe { GetLastError() };
        eprintln!(
            "[probe] win32 cad-child ex-style: WS_EX_NOREDIRECTIONBITMAP failed: WIN32_ERROR({})",
            error.0
        );
    }
}

/// The verified state of the top-level window's creation-time
/// `WS_EX_NOREDIRECTIONBITMAP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoRedirectionBitmapState {
    /// The ex-style is present on the created window.
    Set,
    /// The ex-style is absent: the runtime ignored the creation-time request.
    Absent {
        /// The window's actual ex-style (evidence).
        ex_style: u32,
        /// `GetLastError` observed right after the read-back (supporting detail
        /// only; the read-back itself is authoritative).
        last_error: u32,
    },
}

/// Whether an ex-style value carries `WS_EX_NOREDIRECTIONBITMAP`.
///
/// Pure bit test, so the verification cannot drift from the flag it verifies.
pub fn ex_style_is_no_redirection_bitmap(ex_style: u32) -> bool {
    ex_style & WS_EX_NOREDIRECTIONBITMAP.0 != 0
}

/// Read `WS_EX_NOREDIRECTIONBITMAP` back from the **created** top-level window.
///
/// The flag is requested at creation by the Tauri window builder
/// ([`tauri::window::WindowBuilder::no_redirection_bitmap`], tauri >= 2.12). It is
/// **creation-time only**: a post-creation `SetWindowLongPtrW` always fails
/// `ERROR_INVALID_PARAMETER`, which is why the former post-creation helper was
/// deleted rather than kept. This function only verifies what the runtime
/// actually created - it never mutates the window.
pub fn no_redirection_bitmap_state(window: HWND) -> NoRedirectionBitmapState {
    let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32;
    let last_error = unsafe { GetLastError() }.0;
    if ex_style_is_no_redirection_bitmap(ex_style) {
        NoRedirectionBitmapState::Set
    } else {
        NoRedirectionBitmapState::Absent {
            ex_style,
            last_error,
        }
    }
}

/// Paint the diagnostic GDI marker: a 40x40 solid red square at the child's
/// top-left (`YACR_PROBE_GDI_MARKER`, presence-based).
///
/// This is the V1 calibrator from the composition investigation: it answers "does
/// any of OUR child-window content composite / appear in a capture at all?" with
/// GDI instead of the GPU path. Non-zero output is *calibration*, never acceptance
/// evidence.
pub(super) fn paint_gdi_marker(hwnd: HWND) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, FillRect, GetDC, ReleaseDC, HGDIOBJ,
    };
    let hdc = unsafe { GetDC(Some(hwnd)) };
    if hdc.is_invalid() {
        return;
    }
    let brush = unsafe { CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x0000_00FF)) };
    if !brush.is_invalid() {
        let rect = RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 40,
        };
        unsafe {
            FillRect(hdc, &rect, brush);
        }
        let _ = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
    }
    let _ = unsafe { ReleaseDC(Some(hwnd), hdc) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The child's create-time styles are a contract: a missing `WS_CHILD`
    /// would orphan it, a missing `WS_DISABLED`/`WS_CLIPSIBLINGS` would break the
    /// display-only, CAD-below-DOM composition.
    #[test]
    fn cad_child_creation_styles_are_pinned() {
        let style = child_style(false);
        assert!(style.contains(WS_CHILD), "the CAD window must be a child");
        assert!(style.contains(WS_VISIBLE), "it must be visible");
        assert!(
            style.contains(WS_DISABLED),
            "it must never take OS input (the DOM is the only funnel)"
        );
        assert!(
            style.contains(WS_CLIPSIBLINGS),
            "it must never paint over the WebView container"
        );
        assert!(
            !style.contains(windows::Win32::UI::WindowsAndMessaging::WS_POPUP),
            "a popup would escape the top-level window"
        );
    }

    /// RC2 discriminator: the knob drops exactly `WS_CLIPSIBLINGS` and nothing
    /// else, so a capture difference is attributable to that one bit.
    #[test]
    fn no_clipsiblings_knob_drops_exactly_that_style_bit() {
        let with = child_style(false);
        let without = child_style(true);
        assert!(with.contains(WS_CLIPSIBLINGS));
        assert!(!without.contains(WS_CLIPSIBLINGS));
        // Every other contract bit is unchanged.
        for bit in [WS_CHILD, WS_VISIBLE, WS_DISABLED] {
            assert!(with.contains(bit), "default style keeps every contract bit");
            assert!(without.contains(bit), "the knob drops only CLIPSIBLINGS");
        }
        let removed = with.0 ^ without.0;
        assert_eq!(
            removed, WS_CLIPSIBLINGS.0,
            "exactly one style bit differs between the two variants"
        );
    }

    /// `WS_EX_NOREDIRECTIONBITMAP` is creation-time-only, so the knob is exactly
    /// "add it to the child's ex-style" (and the default stays empty).
    #[test]
    fn child_ex_style_is_empty_by_default_and_adds_no_redirection_on_request() {
        assert_eq!(child_ex_style(false), WINDOW_EX_STYLE::default());
        let requested = child_ex_style(true);
        assert!(requested.contains(WS_EX_NOREDIRECTIONBITMAP));
        assert!(
            !WINDOW_EX_STYLE::default().contains(WS_EX_NOREDIRECTIONBITMAP),
            "the flag is never requested by default"
        );
    }
    /// The read-back predicate is the exact bit it verifies, so the verification
    /// cannot drift from the flag.
    #[test]
    fn no_redirection_bitmap_readback_predicate_is_the_winsdk_bit() {
        assert!(!ex_style_is_no_redirection_bitmap(0));
        assert!(!ex_style_is_no_redirection_bitmap(0x0000_0100)); // WS_EX_WINDOWEDGE
        assert!(ex_style_is_no_redirection_bitmap(
            WS_EX_NOREDIRECTIONBITMAP.0
        ));
        assert!(ex_style_is_no_redirection_bitmap(
            WS_EX_NOREDIRECTIONBITMAP.0 | 0x0000_0008 // | WS_EX_TOPMOST
        ));
    }

    /// The TOP-LEVEL window must request the ex-style **at creation**.
    ///
    /// The Tauri builder call is opaque (no way to read the built window's
    /// requested flags from a unit test), so the contract is pinned at the source
    /// level: the request exists on the builder and the always-failing
    /// post-creation `SetWindowLongPtrW` path is gone. The runtime truth is the
    /// read-back in `init.rs` (`[probe] win32 ex-style: ... set|FAILED`), which is
    /// real-Windows evidence and therefore NOT verified here.
    #[test]
    fn top_level_window_requests_no_redirection_bitmap_at_creation() {
        let init = include_str!("../host/init.rs");
        assert!(
            init.contains(".no_redirection_bitmap(true)"),
            "the top-level window must request WS_EX_NOREDIRECTIONBITMAP at creation"
        );
        assert!(
            init.contains("no_redirection_bitmap_state"),
            "the creation-time request must be verified by an ex-style read-back"
        );
        assert!(
            !init.contains("set_no_redirection_bitmap"),
            "the post-creation SetWindowLongPtrW hack must not come back"
        );
    }
}
