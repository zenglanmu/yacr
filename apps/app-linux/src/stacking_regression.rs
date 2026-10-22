//! DOM-on-top stacking **regression fixture** trigger — not a product switch.
//!
//! DOM-on-top is the only production arrangement since 2026-10-22: the WebView
//! chrome is the GDK toplevel surface and the app-owned CAD `wl_subsurface` is
//! always placed below it. There is no mode that puts the CAD surface back on
//! top, so this module must never be read as "enable DOM-on-top".
//!
//! What it gates is the cheap regression fixture that keeps the arrangement
//! testable across GTK/Tauri/compositor upgrades: an opaque DOM button, a
//! translucent panel and a popup that crosses the chrome/CAD edge, drawn over a
//! real drawing. The trigger contract is pinned to three parts, and a missing one
//! is refused with a diagnostic instead of silently running a weaker test:
//!
//! 1. `YACR_PROBE_STACKING=1` (strict: exactly `"1"`, presence is not enough);
//! 2. the init-script flag `window.__YACR_STACKING_REGRESSION__ === true`
//!    (strict boolean, injected before the document is parsed);
//! 3. a **real** `--open <drawing>` session (a fixture over fake/empty CAD
//!    content cannot test composition).
//!
//! `--probe` is explicitly excluded: it is a fake open (`cad_mode=false`) and
//! would defeat (3). The two together are refused at startup.

use std::ffi::OsStr;

/// The environment variable that enables the fixture.
pub(crate) const ENV: &str = "YACR_PROBE_STACKING";

/// The exact init-script assignment the frontend requires (`=== true`).
pub(crate) const INIT_SCRIPT: &str = "window.__YACR_STACKING_REGRESSION__ = true;";

/// Whether the stacking regression fixture is requested (`YACR_PROBE_STACKING=1`).
pub(crate) fn enabled() -> bool {
    requested(std::env::var_os(ENV).as_deref())
}

fn requested(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// Validate the pinned trigger contract (pure; unit-tested).
///
/// `explicit_probe` is `--probe`/`YACR_PROBE_ONLY`; `has_explicit_drawing` must
/// be `launch.drawing.is_some()` taken **before** the `YACR_TEST_DWG` fallback,
/// so an environment-only path never satisfies the "real `--open`" part.
pub(crate) fn validate(
    requested: bool,
    explicit_probe: bool,
    has_explicit_drawing: bool,
) -> Result<(), String> {
    if !requested {
        return Ok(());
    }
    if explicit_probe {
        return Err(format!(
            "{ENV}=1 is the DOM-on-top stacking regression fixture and cannot run with --probe: \
             --probe is a fake open (no CAD session), while the fixture must overlay a real \
             drawing. Pass --open <drawing> instead."
        ));
    }
    if !has_explicit_drawing {
        return Err(format!(
            "{ENV}=1 requires a real drawing via --open <drawing> (or a positional path); \
             the stacking fixture is refused on an empty/fake CAD session."
        ));
    }
    Ok(())
}

/// The init script for the fixture flag (empty unless requested).
pub(crate) fn initialization_script(requested: bool) -> &'static str {
    if requested {
        INIT_SCRIPT
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_in_is_off_by_default_and_exact() {
        assert!(!requested(None));
        for value in ["", "0", "true", "yes", " 1", "1 ", "2"] {
            assert!(!requested(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(requested(Some(OsStr::new("1"))));
    }

    #[test]
    fn fixture_requires_a_real_open_drawing() {
        assert!(validate(true, false, true).is_ok());
        let error = validate(true, false, false).expect_err("empty CAD session must be refused");
        assert!(error.contains(ENV));
        assert!(error.contains("--open"));
        // A disabled fixture never fails startup for unrelated invocations.
        assert!(validate(false, false, false).is_ok());
        assert!(validate(false, true, false).is_ok());
    }

    #[test]
    fn fixture_refuses_probe_mode_explicitly() {
        let error = validate(true, true, true).expect_err("--probe must be refused");
        assert!(error.contains(ENV));
        assert!(error.contains("--probe"));
    }

    #[test]
    fn normal_startup_does_not_inject_the_fixture_flag() {
        assert_eq!(initialization_script(false), "");
        assert_eq!(initialization_script(true), INIT_SCRIPT);
        assert!(INIT_SCRIPT.contains("window.__YACR_STACKING_REGRESSION__ = true;"));
    }
}
