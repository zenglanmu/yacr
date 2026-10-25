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
//! **X11/XWayland selects the `dom-canvas` carrier (ADR 0024 stage ④,
//! 2026-10-24):** the native-surface arrangement is native-Wayland only, so X11
//! runs the wasm CAD session inside the WebView and draws it into a DOM
//! `<canvas>` ([`SurfaceMode::DomCanvas`]). This module still decides the backend
//! once, before GTK; it no longer refuses X11.
//!
//! **Historical contract (kept, not reused).** Between 2026-10-22 and 2026-10-24
//! X11/XWayland was refused here with [`UNSUPPORTED_BACKEND_MARKER`] and
//! [`UNSUPPORTED_BACKEND_EXIT_CODE`] (ADR 0023 D3). That refusal is no longer
//! produced: the constants, the [`BackendRefusal::Unsupported`] variant and
//! [`BackendRefusal::marker`] stay so the recorded exit-code-4 contract is still
//! documented in code and pinned by tests, and so a future host that has to
//! refuse a backend again does not invent a new code or marker. Code 4 must
//! **not** be reused for anything else.
//!
//! **The `dom-canvas` path can still fail at runtime** (no usable WebGL2 in the
//! WebView). That is a different fault domain with its own marker and exit code
//! (`crate::canvas_runtime`: `yacr: unsupported-runtime: webgl2`, exit code `6`);
//! it can only be detected after the WebView exists, so it is deliberately not
//! part of this pre-GTK policy decision.
//!
//! When the policy is `Auto`, the resolved backend is exported as
//! `GDK_BACKEND` so GTK cannot fall back on its own. An explicit user value is
//! never touched. `prepare_backend` must run from `main` before the GTK event
//! loop starts (and before any other thread exists).

use std::ffi::OsString;
use std::sync::OnceLock;

/// Stable stderr marker of the **historical** X11/XWayland refusal.
///
/// Evidence collection classifies this line **first**; the exit code is only
/// supporting information (an evidence driver may override the process exit
/// code). Keep the literal byte-for-byte stable.
///
/// **Not emitted since ADR 0024 stage ④**: X11 starts the `dom-canvas` path. The
/// constant is retained as the recorded historical contract (exit code `4`) and
/// is pinned by a unit test; evidence scripts that still classify on it must be
/// updated by their owner rather than have the string changed here.
pub const UNSUPPORTED_BACKEND_MARKER: &str = "yacr: unsupported-backend: x11";

/// Process exit code of the **historical** X11/XWayland refusal (`4`).
///
/// Registry: `0` success, `1` generic failure (incl. the Windows/macOS stubs),
/// `2` display-backend policy error, `3` postload-pan driver failure, `4` the
/// historical unsupported backend (no longer produced), `5` Windows WebView2
/// missing, `6` `dom-canvas` runtime missing WebGL2. The marker line — not the
/// code — is the primary classification key.
pub const UNSUPPORTED_BACKEND_EXIT_CODE: i32 = 4;

/// Why [`prepare_backend`] refused to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendRefusal {
    /// **Historical (ADR 0023 D3, 2026-10-22 – 2026-10-24).** X11/XWayland was
    /// refused because the native-surface arrangement failed on XWayland.
    ///
    /// Nothing constructs this variant any more: since ADR 0024 stage ④ X11
    /// resolves to [`SurfaceMode::DomCanvas`]. It is kept so the exit-code-4 /
    /// marker contract stays expressible and testable; a new refusal must not
    /// reuse it without an ADR.
    Unsupported(ResolvedBackend),
    /// The policy/session input was invalid or ambiguous (exit code `2`).
    Invalid(String),
}

impl BackendRefusal {
    /// The stable machine marker line for a refusal.
    ///
    /// Only the historical unsupported-backend case has one; an invalid policy is
    /// an ordinary startup error and must not be classified as an unsupported
    /// backend. Kept with [`BackendRefusal::Unsupported`].
    pub fn marker(self) -> Option<&'static str> {
        match self {
            BackendRefusal::Unsupported(ResolvedBackend::X11) => Some(UNSUPPORTED_BACKEND_MARKER),
            BackendRefusal::Unsupported(ResolvedBackend::Wayland) | BackendRefusal::Invalid(_) => {
                None
            }
        }
    }
}

/// The **pixel carrier** this process runs on (ADR 0024 D1).
///
/// Two carriers are modelled and neither is inferred at runtime by the UI: the
/// host reports the mode as a data flag and the React tree renders the carrier it
/// names (never a platform check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMode {
    /// The host's own wgpu presenter composites a native surface **below** the
    /// WebView (Linux native Wayland, Windows).
    NativeBelow,
    /// The wasm CAD session inside the WebView draws into a DOM `<canvas>`
    /// (X11/XWayland; the browser host shares this path). **No second native
    /// surface exists**, so the native shell must not create a CAD session, a GTK
    /// drawing area or a presenter (iron rule 8 / ADR 0024 D2).
    DomCanvas,
}

/// Wire value of [`SurfaceMode::NativeBelow`] (`cad_status.surfaceMode`).
pub const NATIVE_BELOW: &str = "native-below";
/// Wire value of [`SurfaceMode::DomCanvas`] (`cad_status.surfaceMode`).
pub const DOM_CANVAS: &str = "dom-canvas";

impl SurfaceMode {
    /// The exact string the host reports to the UI for this carrier.
    pub fn wire_name(self) -> &'static str {
        match self {
            SurfaceMode::NativeBelow => NATIVE_BELOW,
            SurfaceMode::DomCanvas => DOM_CANVAS,
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

    /// The pixel carrier this backend runs (ADR 0024 D1).
    ///
    /// Native Wayland keeps the native `wl_subsurface` below the WebView; X11
    /// (and XWayland) has no native-surface arrangement since DOM-on-top, so it
    /// runs the in-DOM `<canvas>` carrier instead.
    pub fn surface_mode(self) -> SurfaceMode {
        match self {
            ResolvedBackend::Wayland => SurfaceMode::NativeBelow,
            ResolvedBackend::X11 => SurfaceMode::DomCanvas,
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

impl BackendPlan {
    /// The pixel carrier this plan runs (see [`SurfaceMode`]).
    pub fn surface_mode(self) -> SurfaceMode {
        self.resolved.surface_mode()
    }
}

/// The one surface mode this process installed (ADR 0024 D4: no hot switching).
///
/// The carrier is decided together with the backend, before GTK initializes, and
/// never changes for the life of the process: there is no runtime switch between
/// `native-below` and `dom-canvas` in either direction.
static SURFACE_MODE: OnceLock<SurfaceMode> = OnceLock::new();

/// Install the process-wide surface mode (called once, from `run`, before GTK).
///
/// A second call never changes the installed mode: it is logged as a programming
/// error on stderr and ignored, because a mid-run carrier change would mean two
/// presenters could own the same pixels (ADR 0024 D1).
pub fn install_surface_mode(mode: SurfaceMode) {
    if SURFACE_MODE.set(mode).is_err() && surface_mode() != mode {
        eprintln!(
            "[probe] surface mode already installed as {:?}; ignoring {:?}",
            surface_mode(),
            mode
        );
    }
}

/// The installed surface mode.
///
/// Defaults to [`SurfaceMode::NativeBelow`] only because that is the carrier of
/// every code path that does not go through [`install_surface_mode`] (unit tests,
/// and the Windows host's own copy of this policy). The Linux host always
/// installs the resolved mode from `run` before any webview or command exists.
pub fn surface_mode() -> SurfaceMode {
    SURFACE_MODE
        .get()
        .copied()
        .unwrap_or(SurfaceMode::NativeBelow)
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
/// - [`BackendPolicy::X11`]: resolved as-is here; X11 selects the `dom-canvas`
///   carrier ([`SurfaceMode::DomCanvas`], ADR 0024), not a refusal.
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

/// Decide and apply the display backend policy.
///
/// Reads the real environment, resolves the [`BackendPlan`] (both backends are
/// supported; the difference is the carrier the plan names) and — only for an
/// `Auto` policy — exports the resolved `GDK_BACKEND` so GTK cannot fall back on
/// its own. Returns [`BackendRefusal::Invalid`] with a user-facing message when
/// the input is unsupported/ambiguous or the selected session is unavailable
/// (caller exits `2`). Must run before any GTK/Tauri initialization.
///
/// The caller must then install the plan's [`BackendPlan::surface_mode`] with
/// [`install_surface_mode`] before building the Tauri app.
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
    fn explicit_x11_resolves_to_the_dom_canvas_carrier() {
        // X11/XWayland is a supported backend again, but it is a different
        // **carrier**, not the native-surface arrangement: the plan names
        // `dom-canvas` (ADR 0024 D1/D4) instead of being refused.
        let plan =
            resolve_backend(BackendPolicy::X11, None, None, None).expect("explicit X11 resolves");
        assert_eq!(plan.resolved, ResolvedBackend::X11);
        assert!(!plan.exports_gdk_backend);
        assert_eq!(plan.surface_mode(), SurfaceMode::DomCanvas);
    }

    #[test]
    fn gdk_backend_strings_are_stable() {
        assert_eq!(ResolvedBackend::X11.gdk_backend(), "x11");
        assert_eq!(ResolvedBackend::Wayland.gdk_backend(), "wayland");
    }

    // --- surface mode (ADR 0024 D1): the resolution table ---------------------

    #[test]
    fn backend_resolution_table_maps_every_backend_to_its_carrier() {
        // The complete table, in one place: the two supported backends and the
        // two refusals. Nothing here may be inferred from a platform probe at
        // runtime — the carrier is a decision, made once.
        let wayland = resolve_backend(
            BackendPolicy::Wayland,
            Some("wayland-0"),
            Some("/run/user/1"),
            None,
        )
        .expect("native Wayland session");
        assert_eq!(wayland.surface_mode(), SurfaceMode::NativeBelow);
        assert_eq!(wayland.surface_mode().wire_name(), "native-below");

        let x11 =
            resolve_backend(BackendPolicy::Auto, None, None, Some(":0")).expect("X11 session");
        assert_eq!(x11.resolved, ResolvedBackend::X11);
        assert_eq!(x11.surface_mode(), SurfaceMode::DomCanvas);
        assert_eq!(x11.surface_mode().wire_name(), "dom-canvas");

        // Unknown policy value → explicit refusal (`Invalid`, caller exits 2).
        let unknown = parse_backend_policy(Some("broadway"))
            .map_err(BackendRefusal::Invalid)
            .expect_err("unknown backend must be refused");
        assert_eq!(unknown.marker(), None);

        // Absent session (no WAYLAND_DISPLAY+XDG_RUNTIME_DIR, no DISPLAY) →
        // explicit refusal, never a headless success.
        let absent = resolve_backend(BackendPolicy::Auto, None, None, None)
            .map_err(BackendRefusal::Invalid)
            .expect_err("no session must be refused");
        assert_eq!(absent.marker(), None);
    }

    #[test]
    fn surface_mode_wire_names_are_the_frontend_contract() {
        // `apps/desktop-ui/src/surface.ts` reads exactly these strings, and only
        // the exact `dom-canvas` value selects the in-DOM carrier.
        assert_eq!(SurfaceMode::NativeBelow.wire_name(), NATIVE_BELOW);
        assert_eq!(SurfaceMode::DomCanvas.wire_name(), DOM_CANVAS);
        assert_eq!(NATIVE_BELOW, "native-below");
        assert_eq!(DOM_CANVAS, "dom-canvas");
    }

    // --- historical unsupported-backend refusal (exit 4) ---------------------

    #[test]
    fn x11_is_no_longer_refused_but_the_historical_contract_is_intact() {
        // Behaviour since ADR 0024 stage ④: X11 starts, on `dom-canvas`. The
        // historical refusal is *not* produced by any resolution path.
        for policy in [BackendPolicy::X11, BackendPolicy::Auto] {
            let plan = resolve_backend(policy, None, None, Some(":0"))
                .expect("X11 resolves instead of being refused");
            assert_eq!(plan.surface_mode(), SurfaceMode::DomCanvas);
        }
        // The recorded exit-code-4 contract stays byte-stable and documented.
        assert_eq!(UNSUPPORTED_BACKEND_MARKER, "yacr: unsupported-backend: x11");
        assert_eq!(UNSUPPORTED_BACKEND_EXIT_CODE, 4);
        assert_eq!(
            BackendRefusal::Unsupported(ResolvedBackend::X11).marker(),
            Some(UNSUPPORTED_BACKEND_MARKER)
        );
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
        assert_eq!(plan.resolved, ResolvedBackend::Wayland);
        assert_eq!(plan.surface_mode(), SurfaceMode::NativeBelow);
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
    fn historical_refusal_reports_readable_x11_identity() {
        // The historical variant is not constructed by the host any more; when an
        // evidence script (or a future host) reports it, the diagnostic can still
        // name the backend.
        let refusal = BackendRefusal::Unsupported(ResolvedBackend::X11);
        assert_eq!(refusal.clone().marker(), Some(UNSUPPORTED_BACKEND_MARKER));
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
