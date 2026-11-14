//! The single contract for the host **evidence-log gate**.
//!
//! Probe diagnostics (`[probe] …` lines) are evidence, never UI text. Most of
//! them are per-frame or per-input rate, so an always-on `eprintln!` is both log
//! spam and avoidable work on the render/IPC path in normal use. Every host
//! therefore shares this one presence-based gate: the variable's *existence*
//! enables the diagnostics (the value is irrelevant, `=1` and `=` are
//! equivalent), and an absent variable keeps a normal session's stderr quiet.
//!
//! Evidence runs already set it: `scripts/collect-evidence.sh` and
//! `scripts/host-smoke-linux.sh` export `YACR_PROBE_CAD_LOG=1`, so gating a line
//! here changes nothing for the evidence drivers.

/// The evidence-log gate. Presence-based, exactly like the other `YACR_PROBE_*`
/// host knobs.
pub const EVIDENCE_LOG_ENV: &str = "YACR_PROBE_CAD_LOG";

/// Whether the host should emit its probe evidence lines.
pub fn evidence_log_enabled() -> bool {
    env_flag_present(std::env::var_os(EVIDENCE_LOG_ENV))
}

/// Pure presence test behind [`evidence_log_enabled`].
///
/// Split out so the documented semantics (presence, not truthiness) are testable
/// without mutating the process environment.
fn env_flag_present(value: Option<std::ffi::OsString>) -> bool {
    value.is_some()
}

#[cfg(test)]
mod tests {
    use super::env_flag_present;

    /// `YACR_PROBE_CAD_LOG` is presence-based (like `YACR_PROBE_NO_MAXIMIZE`):
    /// any value — including an empty one — enables it, and an absent variable
    /// keeps a normal session quiet.
    #[test]
    fn evidence_log_gate_is_presence_based() {
        assert!(!env_flag_present(None), "absent must keep the log quiet");
        assert!(
            env_flag_present(Some(std::ffi::OsString::new())),
            "an empty value still counts as present"
        );
        assert!(
            env_flag_present(Some(std::ffi::OsString::from("0"))),
            "the value is irrelevant; only presence counts"
        );
    }
}
