//! Pure geometry contracts for the macOS CAD child view (no AppKit, no GPU).
//!
//! Split out of `macos_surface.rs` so the runtime surface code stays under the
//! code-file size limit; these are the decisions the runtime applies. The
//! contracts are identical to the Windows host's
//! `win32_surface/geometry.rs`, because the size authority is platform-neutral.

/// The surface-reconfigure decision for one accepted layout.
///
/// `None` means nothing to do (no presenter yet, or already at the authority
/// size); `Some((from, to))` is the reconfigure that must happen. Kept pure so
/// the "a layout change after presenter init reconfigures to the authority size"
/// contract is testable without a window or a GPU.
pub(super) fn reconfigure_plan(
    surface: Option<(u32, u32)>,
    authority: (u32, u32),
) -> Option<((u32, u32), (u32, u32))> {
    match surface {
        Some(current) if current != authority => Some((current, authority)),
        _ => None,
    }
}

/// The `size_authority=` verdict for the `[probe] geometry` line.
///
/// `ok` only when the child is sized by the authority *and* (once a surface
/// exists) the swapchain reports that same size. `FAIL` means the presenter's
/// size guard will refuse every frame — the CAD region cannot show content.
pub fn size_authority_verdict(
    child_desired: (u32, u32),
    native_surface: Option<(u32, u32)>,
    authority: (u32, u32),
) -> &'static str {
    if child_desired == authority && native_surface.is_none_or(|size| size == authority) {
        "ok"
    } else {
        "FAIL"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE runtime invariant: once a presenter exists, a layout change MUST
    /// reconfigure the swapchain to the authority size.
    #[test]
    fn layout_change_after_presenter_init_reconfigures_to_the_authority_size() {
        assert_eq!(
            reconfigure_plan(Some((626, 341)), (926, 341)),
            Some(((626, 341), (926, 341)))
        );
        // Both axes, and a shrink as well as a grow.
        assert_eq!(
            reconfigure_plan(Some((926, 341)), (626, 500)),
            Some(((926, 341), (626, 500)))
        );
        // Already at the authority size: nothing to do (idempotent backstop).
        assert_eq!(reconfigure_plan(Some((926, 341)), (926, 341)), None);
        // No presenter yet: init sizes it from the authority, so no reconfigure.
        assert_eq!(reconfigure_plan(None, (926, 341)), None);
    }

    /// The verdict token only says `ok` when the authority holds end-to-end.
    #[test]
    fn size_authority_verdict_requires_child_and_surface_to_match() {
        let authority = (926, 341);
        assert_eq!(
            size_authority_verdict((926, 341), Some((926, 341)), authority),
            "ok"
        );
        // Before the presenter exists there is no swapchain to disagree with.
        assert_eq!(size_authority_verdict((926, 341), None, authority), "ok");
        // Stale surface: the exact runtime state the fix eliminates.
        assert_eq!(
            size_authority_verdict((926, 341), Some((626, 341)), authority),
            "FAIL"
        );
        // A child that is not the authority size is a failure regardless.
        assert_eq!(
            size_authority_verdict((626, 341), Some((926, 341)), authority),
            "FAIL"
        );
    }
}
