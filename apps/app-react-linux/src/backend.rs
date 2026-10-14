//! X11 backend enforcement.
//!
//! The probe targets X11/XWayland exclusively (the GTK child composition bridge
//! is X11-specific). GTK silently falls back between backends, which would make
//! a "real GPU X11" result ambiguous, so the backend is required to be `x11`
//! explicitly:
//! - already `x11` → accepted;
//! - unset → the host sets it to `x11` for the process;
//! - set to anything else → a hard error, never a silent fallback.

/// The outcome of checking `GDK_BACKEND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendDecision {
    /// `GDK_BACKEND` was already `x11`.
    AlreadyX11,
    /// `GDK_BACKEND` was unset and the host enforced `x11`.
    EnforcedX11,
}

/// The value the host requires.
pub const REQUIRED_GDK_BACKEND: &str = "x11";

/// Decide what to do about a `GDK_BACKEND` value.
///
/// `Err` carries a user-facing message and means startup must abort.
pub fn check_gdk_backend(current: Option<&str>) -> Result<BackendDecision, String> {
    match current {
        Some(REQUIRED_GDK_BACKEND) => Ok(BackendDecision::AlreadyX11),
        None => Ok(BackendDecision::EnforcedX11),
        Some(other) => Err(format!(
            "GDK_BACKEND={other} is not supported by the probe; set GDK_BACKEND={REQUIRED_GDK_BACKEND} \
             (X11/XWayland is required). Refusing to start rather than fall back silently."
        )),
    }
}

/// Enforce `GDK_BACKEND=x11` for this process.
///
/// Returns `Err` with a user-facing message when the environment requests a
/// different backend. Must run before any GTK/Tauri initialization.
pub fn enforce_x11_backend() -> Result<BackendDecision, String> {
    let current = std::env::var("GDK_BACKEND").ok();
    let decision = check_gdk_backend(current.as_deref())?;
    if decision == BackendDecision::EnforcedX11 {
        // SAFETY: called from `main` before the GTK event loop starts and before
        // any other thread exists, so there is no concurrent environment reader.
        unsafe {
            std::env::set_var("GDK_BACKEND", REQUIRED_GDK_BACKEND);
        }
    }
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_x11_is_accepted() {
        assert_eq!(
            check_gdk_backend(Some("x11")),
            Ok(BackendDecision::AlreadyX11)
        );
    }

    #[test]
    fn unset_is_enforced_to_x11() {
        assert_eq!(check_gdk_backend(None), Ok(BackendDecision::EnforcedX11));
    }

    #[test]
    fn a_different_backend_is_a_hard_error() {
        let error = check_gdk_backend(Some("wayland")).expect_err("wayland must be rejected");
        assert!(error.contains("GDK_BACKEND=wayland"));
        assert!(error.contains("x11"));
        assert!(check_gdk_backend(Some("broadway")).is_err());
        assert!(check_gdk_backend(Some("X11")).is_err(), "matching is exact");
    }
}
