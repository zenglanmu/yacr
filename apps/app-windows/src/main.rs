//! Windows desktop binary (`yacr-windows`) — explicit NOT IMPLEMENTED stub.
//!
//! The desktop UI was replaced by a React/Tauri host, delivered for Linux first
//! (`apps/app-linux`, bin `yacr`); the old shared Slint host no longer exists.
//! Windows support is **not implemented** in this round: it waits until the
//! Linux desktop app passes user validation.
//!
//! The binary name is deliberately `yacr-windows`, distinct from the Linux
//! product binary `yacr` (`apps/app-linux`): both packages compile for the same
//! host target during workspace checks, so a shared name would make this
//! unavailable stub compete with the real Linux executable for the same output
//! artifact path. See `Cargo.toml` `[[bin]]`.
//!
//! Until then this binary refuses to run: it prints a diagnostic to stderr and
//! exits with a non-zero status rather than pretending to start a host it does
//! not have. The PE icon resource is still embedded via `build.rs` so the
//! (deliberately failing) binary keeps the product icon.

/// Deliberate "not implemented" diagnostic. Kept as a constant so the contract
/// test can pin it without spawning the process. The `yacr-windows` prefix
/// reports the binary's own (distinct) identity.
const NOT_IMPLEMENTED: &str =
    "yacr-windows: Windows host NOT IMPLEMENTED \u{2014} the React/Tauri desktop route is \
     Linux-only in this round; see docs/handoff.md";

/// Exit status for the deliberate "not implemented" failure.
const NOT_IMPLEMENTED_EXIT: i32 = 1;

fn main() {
    eprintln!("{NOT_IMPLEMENTED}");
    std::process::exit(NOT_IMPLEMENTED_EXIT);
}

#[cfg(test)]
mod tests {
    /// The stub must stay an explicit, non-zero, visible failure: a user (or a
    /// packaging smoke test) has to see "NOT IMPLEMENTED" rather than a silent
    /// success. This pins the wording, the binary's own distinct identity
    /// (`yacr-windows`, never the Linux product `yacr`), and the non-zero exit
    /// status.
    #[test]
    fn stub_reports_not_implemented_and_fails() {
        assert!(
            super::NOT_IMPLEMENTED.contains("NOT IMPLEMENTED"),
            "diagnostic must state NOT IMPLEMENTED: {}",
            super::NOT_IMPLEMENTED
        );
        assert!(
            super::NOT_IMPLEMENTED.starts_with("yacr-windows:"),
            "the stub must report its own distinct identity: {}",
            super::NOT_IMPLEMENTED
        );
        assert_ne!(super::NOT_IMPLEMENTED_EXIT, 0);
    }
}
