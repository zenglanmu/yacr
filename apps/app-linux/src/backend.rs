//! Display backend policy (`GDK_BACKEND`) selection.
//!
//! The host supports two GTK backends: **X11/XWayland** and **native Wayland**.
//! GTK silently falls back between backends, which would make a "real GPU X11"
//! or "native Wayland" result ambiguous, so the effective backend is chosen
//! explicitly here — before any GTK/Tauri initialization — and never left to
//! GDK's own fallback:
//!
//! - `GDK_BACKEND=x11` / `GDK_BACKEND=wayland` → accepted intentionally and
//!   **not** rewritten (the user's explicit backend is preserved);
//! - unset (`Auto`) → derived from the available session:
//!   `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR` → native Wayland (preferred),
//!   otherwise `DISPLAY`-only → X11; with neither there is **no headless
//!   success** — startup aborts with a precise error;
//! - unsupported or ambiguous values (unknown names, `broadway`, an ordered
//!   `wayland,x11` list, empty) → a hard error, never a silent fallback.
//!
//! **X11/XWayland is explicitly unsupported since DOM-on-top (2026-10-22):** the
//! WebView-over-CAD arrangement is native-Wayland only, and the X11 XWayland
//! compositing attempt failed. Whatever selects X11 (an explicit
//! `GDK_BACKEND=x11` or an `Auto` derivation from `DISPLAY`), the backend is
//! refused here — before GTK/Tauri initialization — with
//! [`UNSUPPORTED_BACKEND_MARKER`] and [`UNSUPPORTED_BACKEND_EXIT_CODE`]. There is
//! no fallback X11 arrangement and no silent success.
//!
//! When the policy is `Auto`, the resolved backend is exported as
//! `GDK_BACKEND` so GTK cannot fall back on its own. An explicit user value is
//! never touched. `prepare_backend` must run from `main` before the GTK event
//! loop starts (and before any other thread exists).

use std::ffi::OsString;

/// Stable stderr marker for the refused X11/XWayland backend.
///
/// Evidence collection classifies this line **first**; the exit code is only
/// supporting information (an evidence driver may override the process exit
/// code). Keep the literal byte-for-byte stable.
pub const UNSUPPORTED_BACKEND_MARKER: &str = "yacr: unsupported-backend: x11";

/// Process exit code for the refused X11/XWayland backend.
///
/// Registry: `0` success, `1` generic failure (incl. the Windows/macOS stubs),
/// `2` display-backend policy error, `3` postload-pan driver failure, `4` this
/// explicit unsupported backend. The marker line — not the code — is the primary
/// classification key.
pub const UNSUPPORTED_BACKEND_EXIT_CODE: i32 = 4;

/// Why [`prepare_backend`] refused to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendRefusal {
    /// The resolved backend is not supported by this host (X11/XWayland).
    /// Carries the resolved backend so the caller can name it in the diagnostic.
    Unsupported(ResolvedBackend),
    /// The policy/session input was invalid or ambiguous (exit code `2`).
    Invalid(String),
}

impl BackendRefusal {
    /// The stable machine marker line for a refusal.
    ///
    /// Only the unsupported-backend case has one; an invalid policy is an ordinary
    /// startup error and must not be classified as an unsupported backend.
    pub fn marker(self) -> Option<&'static str> {
        match self {
            BackendRefusal::Unsupported(ResolvedBackend::X11) => Some(UNSUPPORTED_BACKEND_MARKER),
            BackendRefusal::Unsupported(ResolvedBackend::Wayland) | BackendRefusal::Invalid(_) => {
                None
            }
        }
    }
}

/// Requested display backend policy, parsed from `GDK_BACKEND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPolicy {
    /// `GDK_BACKEND` was unset: derive the backend from the available session.
    Auto,
    /// `GDK_BACKEND=x11` was requested explicitly (X11 or XWayland).
    X11,
    /// `GDK_BACKEND=wayland` was requested explicitly (native Wayland).
    Wayland,
}

/// Effective backend the process will run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackend {
    /// X11 or XWayland (an X11 server, possibly nested under a Wayland session).
    X11,
    /// Native Wayland (never XWayland).
    Wayland,
}

impl ResolvedBackend {
    /// The `GDK_BACKEND` string that selects this backend.
    pub fn gdk_backend(self) -> &'static str {
        match self {
            ResolvedBackend::X11 => "x11",
            ResolvedBackend::Wayland => "wayland",
        }
    }
}

/// The outcome of [`resolve_backend`] / [`prepare_backend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendPlan {
    /// The policy that was requested (explicit or `Auto`).
    pub policy: BackendPolicy,
    /// The effective backend the process runs on.
    pub resolved: ResolvedBackend,
    /// `true` when the host must export `GDK_BACKEND` for [`Self::resolved`]
    /// (i.e. the policy was `Auto`); `false` when an explicit user value is
    /// preserved unchanged.
    pub exports_gdk_backend: bool,
}

/// Parse an explicit `GDK_BACKEND` value into a policy.
///
/// `None` (unset) is [`BackendPolicy::Auto`]. The value must be **exactly** `x11`
/// or `wayland`: it is matched verbatim (case-sensitive, no trimming), so a value
/// with surrounding whitespace, an empty/whitespace-only value, an ordered
/// multi-backend list such as `wayland,x11`, or any other name is rejected with a
/// user-facing message. The original value is never silently normalized, so the
/// host never falls back between backends silently.
pub fn parse_backend_policy(current: Option<&str>) -> Result<BackendPolicy, String> {
    let Some(value) = current else {
        return Ok(BackendPolicy::Auto);
    };
    if value.is_empty() {
        return Err(
            "GDK_BACKEND is set but empty; set GDK_BACKEND=x11 or GDK_BACKEND=wayland.".to_string(),
        );
    }
    if value != value.trim() {
        return Err(format!(
            "GDK_BACKEND={value:?} has surrounding whitespace; set it to exactly GDK_BACKEND=x11 \
             or GDK_BACKEND=wayland with no padding. Refusing to start rather than fall back \
             silently."
        ));
    }
    if value.contains(',') || value.split_whitespace().count() > 1 {
        return Err(format!(
            "GDK_BACKEND={value} is an ambiguous multi-backend list; this host does not fall back \
             between backends. Set exactly one of GDK_BACKEND=x11 or GDK_BACKEND=wayland."
        ));
    }
    match value {
        "x11" => Ok(BackendPolicy::X11),
        "wayland" => Ok(BackendPolicy::Wayland),
        other => Err(format!(
            "GDK_BACKEND={other} is not supported; set GDK_BACKEND=x11 or GDK_BACKEND=wayland. \
             Refusing to start rather than fall back silently."
        )),
    }
}

/// `true` when a session variable is present and non-empty.
fn present(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Resolve the effective backend from the requested policy and the session.
///
/// This is pure (no environment access) so the exact choices are unit-testable
/// without mutating the process environment.
///
/// - [`BackendPolicy::Auto`]: native Wayland when both `WAYLAND_DISPLAY` and
///   `XDG_RUNTIME_DIR` are present, else X11 when `DISPLAY` is present, else a
///   precise error (no headless success).
/// - [`BackendPolicy::Wayland`]: requires the native Wayland session; if it is
///   unavailable the call fails rather than silently falling back to X11.
/// - [`BackendPolicy::X11`]: resolved as-is here; [`ensure_supported_backend`]
///   then refuses it explicitly (X11 is not a supported production backend).
pub fn resolve_backend(
    policy: BackendPolicy,
    wayland_display: Option<&str>,
    xdg_runtime_dir: Option<&str>,
    display: Option<&str>,
) -> Result<BackendPlan, String> {
    let native_wayland = present(wayland_display) && present(xdg_runtime_dir);
    let resolved = match policy {
        BackendPolicy::X11 => ResolvedBackend::X11,
        BackendPolicy::Wayland => {
            if native_wayland {
                ResolvedBackend::Wayland
            } else {
                return Err(
                    "GDK_BACKEND=wayland was requested but no native Wayland session is available \
                     (WAYLAND_DISPLAY and/or XDG_RUNTIME_DIR is unset); refusing to fall back to X11."
                        .to_string(),
                );
            }
        }
        BackendPolicy::Auto => {
            if native_wayland {
                ResolvedBackend::Wayland
            } else if present(display) {
                ResolvedBackend::X11
            } else {
                return Err(
                    "no display session detected: set WAYLAND_DISPLAY (with XDG_RUNTIME_DIR) for \
                     native Wayland, or DISPLAY for X11/XWayland. This host does not run headless."
                        .to_string(),
                );
            }
        }
    };
    Ok(BackendPlan {
        policy,
        resolved,
        exports_gdk_backend: policy == BackendPolicy::Auto,
    })
}

/// Read an environment variable, rejecting a non-Unicode value explicitly.
///
/// Kept separate from [`resolve_backend`] so the non-Unicode diagnostic is
/// testable without touching the process environment.
fn os_to_utf8(key: &str, value: OsString) -> Result<String, String> {
    value.into_string().map_err(|_| {
        format!("{key} is set but is not valid Unicode; refusing to start with an ambiguous display backend.")
    })
}

fn read_env(key: &str) -> Result<Option<String>, String> {
    match std::env::var_os(key) {
        None => Ok(None),
        Some(value) => os_to_utf8(key, value).map(Some),
    }
}

/// Refuse a resolved backend this host does not support (pure; unit-tested).
///
/// Native Wayland is accepted unchanged. X11/XWayland — whether requested
/// explicitly or derived by `Auto` — is [`BackendRefusal::Unsupported`], so the
/// caller can print the stable marker and exit with code
/// [`UNSUPPORTED_BACKEND_EXIT_CODE`] before GTK is initialized.
pub fn ensure_supported_backend(plan: BackendPlan) -> Result<BackendPlan, BackendRefusal> {
    match plan.resolved {
        ResolvedBackend::Wayland => Ok(plan),
        ResolvedBackend::X11 => Err(BackendRefusal::Unsupported(plan.resolved)),
    }
}

/// Decide and apply the display backend policy.
///
/// Reads the real environment, resolves the [`BackendPlan`], refuses an
/// unsupported backend ([`ensure_supported_backend`]), and — only for an `Auto`
/// policy that was accepted — exports the resolved `GDK_BACKEND` so GTK cannot
/// fall back on its own. Returns [`BackendRefusal::Invalid`] with a user-facing
/// message when the input is unsupported/ambiguous or the selected session is
/// unavailable (caller exits `2`), or [`BackendRefusal::Unsupported`] for
/// X11/XWayland (caller prints the marker and exits `4`). Must run before any
/// GTK/Tauri initialization.
pub fn prepare_backend() -> Result<BackendPlan, BackendRefusal> {
    let gdk_backend = read_env("GDK_BACKEND").map_err(BackendRefusal::Invalid)?;
    let policy = parse_backend_policy(gdk_backend.as_deref()).map_err(BackendRefusal::Invalid)?;
    let wayland_display = read_env("WAYLAND_DISPLAY").map_err(BackendRefusal::Invalid)?;
    let xdg_runtime_dir = read_env("XDG_RUNTIME_DIR").map_err(BackendRefusal::Invalid)?;
    let display = read_env("DISPLAY").map_err(BackendRefusal::Invalid)?;

    let plan = resolve_backend(
        policy,
        wayland_display.as_deref(),
        xdg_runtime_dir.as_deref(),
        display.as_deref(),
    )
    .map_err(BackendRefusal::Invalid)?;
    let plan = ensure_supported_backend(plan)?;

    if plan.exports_gdk_backend {
        // SAFETY: called from `main` before the GTK event loop starts and before
        // any other thread exists, so there is no concurrent environment reader.
        unsafe {
            std::env::set_var("GDK_BACKEND", plan.resolved.gdk_backend());
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_backend_policy -------------------------------------------------

    #[test]
    fn unset_gdk_backend_is_auto() {
        assert_eq!(parse_backend_policy(None), Ok(BackendPolicy::Auto));
    }

    #[test]
    fn explicit_x11_and_wayland_are_accepted() {
        assert_eq!(parse_backend_policy(Some("x11")), Ok(BackendPolicy::X11));
        assert_eq!(
            parse_backend_policy(Some("wayland")),
            Ok(BackendPolicy::Wayland)
        );
    }

    #[test]
    fn matching_is_exact_and_case_sensitive() {
        assert!(parse_backend_policy(Some("X11")).is_err());
        assert!(parse_backend_policy(Some("Wayland")).is_err());
    }

    #[test]
    fn unsupported_backend_is_a_hard_error() {
        let error = parse_backend_policy(Some("broadway")).expect_err("broadway must be rejected");
        assert!(error.contains("GDK_BACKEND=broadway"));
        assert!(error.contains("x11"));
        assert!(error.contains("wayland"));
    }

    #[test]
    fn empty_backend_is_rejected() {
        assert!(parse_backend_policy(Some("")).is_err());
        assert!(parse_backend_policy(Some("   ")).is_err());
    }

    #[test]
    fn surrounding_whitespace_is_rejected_not_trimmed() {
        // The value is matched verbatim: leading/trailing spaces or tabs must not
        // be silently trimmed (the untrimmed value would otherwise reach GDK).
        for value in [
            " x11",
            "x11 ",
            " x11 ",
            "\tx11",
            "x11\t",
            "\tx11\t",
            " wayland",
            "wayland ",
            "wayland\t",
            "\twayland",
            "x11 \t",
            "\t x11",
        ] {
            let error = parse_backend_policy(Some(value))
                .expect_err("surrounding whitespace must be rejected");
            assert!(
                error.contains("whitespace"),
                "diagnostic must mention whitespace for {value:?}, got: {error}"
            );
        }
    }

    #[test]
    fn ambiguous_multi_backend_list_is_rejected() {
        // GDK accepts an ordered list, but that would silently fall back.
        for value in ["wayland,x11", "x11,wayland", "wayland x11"] {
            let error = parse_backend_policy(Some(value)).expect_err("list must be rejected");
            assert!(error.contains(value), "diagnostic must quote {value}");
            assert!(error.contains("multi-backend"));
        }
    }

    // --- resolve_backend ------------------------------------------------------

    #[test]
    fn auto_prefers_native_wayland_when_session_is_complete() {
        let plan = resolve_backend(
            BackendPolicy::Auto,
            Some("wayland-0"),
            Some("/run/user/1"),
            None,
        )
        .expect("complete Wayland session must resolve");
        assert_eq!(plan.resolved, ResolvedBackend::Wayland);
        assert!(plan.exports_gdk_backend, "Auto must export GDK_BACKEND");
    }

    #[test]
    fn auto_prefers_wayland_over_a_coexisting_display() {
        let plan = resolve_backend(
            BackendPolicy::Auto,
            Some("wayland-0"),
            Some("/run/user/1"),
            Some(":0"),
        )
        .expect("Wayland is preferred even with DISPLAY set");
        assert_eq!(plan.resolved, ResolvedBackend::Wayland);
    }

    #[test]
    fn auto_without_xdg_runtime_dir_falls_back_to_displays_x11() {
        // libwayland cannot connect without XDG_RUNTIME_DIR, so an X11 display
        // is the only usable session here.
        let plan = resolve_backend(BackendPolicy::Auto, Some("wayland-0"), None, Some(":0"))
            .expect("X11 fallback with a display");
        assert_eq!(plan.resolved, ResolvedBackend::X11);
    }

    #[test]
    fn auto_display_only_is_x11() {
        let plan =
            resolve_backend(BackendPolicy::Auto, None, None, Some(":0")).expect("X11 session");
        assert_eq!(plan.resolved, ResolvedBackend::X11);
        assert!(plan.exports_gdk_backend);
    }

    #[test]
    fn auto_with_empty_env_values_is_not_a_session() {
        assert!(resolve_backend(BackendPolicy::Auto, Some(""), Some(""), Some("")).is_err());
    }

    #[test]
    fn auto_with_no_session_is_a_precise_error_no_headless_success() {
        let error =
            resolve_backend(BackendPolicy::Auto, None, None, None).expect_err("headless must fail");
        assert!(error.contains("WAYLAND_DISPLAY"));
        assert!(error.contains("DISPLAY"));
        assert!(error.contains("headless"));
    }

    #[test]
    fn explicit_wayland_requires_a_native_session() {
        let plan = resolve_backend(
            BackendPolicy::Wayland,
            Some("wayland-0"),
            Some("/run/user/1"),
            None,
        )
        .expect("native Wayland session");
        assert_eq!(plan.resolved, ResolvedBackend::Wayland);
        assert!(
            !plan.exports_gdk_backend,
            "explicit policy is not rewritten"
        );

        let error = resolve_backend(BackendPolicy::Wayland, Some("wayland-0"), None, Some(":0"))
            .expect_err("no silent fallback to X11");
        assert!(error.contains("GDK_BACKEND=wayland"));
        assert!(error.contains("X11"));
    }

    #[test]
    fn explicit_x11_is_resolved_and_then_refused() {
        // The historically supported path is no longer a production backend: it
        // still resolves (so the diagnostic can name it) but is refused.
        let plan = resolve_backend(BackendPolicy::X11, None, None, None)
            .expect("explicit X11 resolves before the support check");
        assert_eq!(plan.resolved, ResolvedBackend::X11);
        assert!(!plan.exports_gdk_backend);
        assert_eq!(
            ensure_supported_backend(plan),
            Err(BackendRefusal::Unsupported(ResolvedBackend::X11))
        );
    }

    #[test]
    fn gdk_backend_strings_are_stable() {
        assert_eq!(ResolvedBackend::X11.gdk_backend(), "x11");
        assert_eq!(ResolvedBackend::Wayland.gdk_backend(), "wayland");
    }

    // --- unsupported backend refusal -----------------------------------------

    #[test]
    fn x11_is_refused_before_gtk_initialization() {
        // Explicit X11 and an Auto fallback to X11 are both refused identically:
        // there is no fallback arrangement and no silent success.
        for policy in [BackendPolicy::X11, BackendPolicy::Auto] {
            let plan = resolve_backend(policy, None, None, Some(":0"))
                .expect("resolve_backend still resolves X11; the refusal is separate");
            assert_eq!(plan.resolved, ResolvedBackend::X11);
            assert_eq!(
                ensure_supported_backend(plan),
                Err(BackendRefusal::Unsupported(ResolvedBackend::X11))
            );
        }
    }

    #[test]
    fn native_wayland_plan_is_accepted_unchanged() {
        let plan = resolve_backend(
            BackendPolicy::Wayland,
            Some("wayland-0"),
            Some("/run/user/1"),
            Some(":0"),
        )
        .expect("native Wayland session");
        assert_eq!(ensure_supported_backend(plan), Ok(plan));
    }

    #[test]
    fn refusal_marker_and_exit_code_are_stable() {
        assert_eq!(UNSUPPORTED_BACKEND_MARKER, "yacr: unsupported-backend: x11");
        assert_eq!(UNSUPPORTED_BACKEND_EXIT_CODE, 4);
        assert_eq!(
            BackendRefusal::Unsupported(ResolvedBackend::X11).marker(),
            Some(UNSUPPORTED_BACKEND_MARKER)
        );
    }

    #[test]
    fn invalid_policy_is_not_classified_as_unsupported_backend() {
        // Exit code 2 errors must never be reported as an unsupported backend:
        // the marker is what evidence collection classifies on.
        let refusal = parse_backend_policy(Some("broadway"))
            .map_err(BackendRefusal::Invalid)
            .expect_err("invalid policy must be refused");
        assert_eq!(refusal.clone().marker(), None);
        assert!(matches!(refusal, BackendRefusal::Invalid(_)));
    }

    #[test]
    fn refusal_reports_readable_x11_identity() {
        let refusal = ensure_supported_backend(
            resolve_backend(BackendPolicy::X11, None, None, None).expect("resolved"),
        )
        .expect_err("X11 refused");
        assert_eq!(refusal.clone().marker(), Some(UNSUPPORTED_BACKEND_MARKER));
        // The diagnostic can reuse the stable backend name.
        let BackendRefusal::Unsupported(backend) = refusal else {
            panic!("expected Unsupported");
        };
        assert_eq!(backend, ResolvedBackend::X11);
        assert_eq!(backend.gdk_backend(), "x11");
    }

    // --- non-Unicode env value ------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn non_unicode_env_value_is_rejected() {
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(vec![0xff, 0xfe]);
        let error = os_to_utf8("GDK_BACKEND", value).expect_err("non-Unicode must be rejected");
        assert!(error.contains("GDK_BACKEND"));
        assert!(error.contains("Unicode"));
    }
}
