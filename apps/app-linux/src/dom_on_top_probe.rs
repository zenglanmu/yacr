//! Opt-in compositor experiment; never a product stacking policy.

pub(crate) fn enabled() -> bool {
    requested(std::env::var_os("YACR_PROBE_DOM_ON_TOP").as_deref())
}

fn requested(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

pub(crate) fn validate(requested: bool, explicit_probe: bool) -> Result<(), &'static str> {
    if requested && !explicit_probe {
        Err("YACR_PROBE_DOM_ON_TOP=1 requires explicit --probe (experimental marker only)")
    } else {
        Ok(())
    }
}

pub(crate) fn initialization_script(requested: bool) -> &'static str {
    if requested {
        "window.__YACR_PROBE_DOM_ON_TOP__ = true;"
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
        for value in ["", "0", "true", "yes", " 1", "1 "] {
            assert!(!requested(Some(std::ffi::OsStr::new(value))));
        }
        assert!(requested(Some(std::ffi::OsStr::new("1"))));
    }

    #[test]
    fn experiment_requires_explicit_marker_mode() {
        assert!(validate(true, false).is_err());
        assert!(validate(true, true).is_ok());
        assert!(validate(false, false).is_ok());
        assert!(validate(false, true).is_ok());
    }

    #[test]
    fn normal_startup_does_not_inject_experiment_global() {
        assert_eq!(initialization_script(false), "");
        assert_eq!(
            initialization_script(true),
            "window.__YACR_PROBE_DOM_ON_TOP__ = true;"
        );
    }
}
