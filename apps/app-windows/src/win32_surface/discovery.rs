//! Discovery of the wry-owned WebView container HWND (the z-order anchor).
//!
//! Split out of `win32_surface.rs` to stay under the code-file size limit.

use super::*;

/// Find the wry-created `WRY_WEBVIEW` container among the top-level children.
///
/// wry 0.55 exposes the WebView2 controller/environment but not the container
/// HWND, so the container is discovered by class name.
pub(super) fn find_webview_container(top: HWND) -> Option<HWND> {
    struct Search {
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Search` passed by `find_webview_container`
        // and this callback only runs inside that `EnumChildWindows` call.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        if window_class(hwnd).as_deref() == Some(WEBVIEW_CONTAINER_CLASS) {
            search.found = Some(hwnd);
            BOOL(0)
        } else {
            BOOL(1)
        }
    }

    let mut search = Search { found: None };
    unsafe {
        let _ = EnumChildWindows(
            Some(top),
            Some(callback),
            LPARAM(std::ptr::addr_of_mut!(search) as isize),
        );
    }
    search.found
}

/// The window class name of `hwnd`, if readable.
pub(super) fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}
