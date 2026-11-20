//! The `cad_pick_directory` shell capability's shared vocabulary.
//!
//! The Options > Files page asks the host for a **local absolute directory**
//! (the fonts directory row). That is not a CAD wire command: it carries no
//! document request and the answer is a path the carrier either can or cannot
//! produce. Three facts are shared by every carrier and every transport, so they
//! live here instead of being re-spelled in each host:
//!
//! * the command name ([`PICK_DIRECTORY_COMMAND`]);
//! * the stable refusal of a carrier with no local folder chooser
//!   ([`UNSUPPORTED_PICK_DIRECTORY`]) — `dom-canvas` (Web **and** X11/XWayland)
//!   and every native carrier that has not wired one yet. A refusal is never a
//!   silent `null`: cancellation and "this carrier cannot pick" are different
//!   facts;
//! * the explicit failure marker ([`pick_directory_failed`]) for a chooser that
//!   ran and failed (a portal error, a non-local URI, a thread that could not
//!   start). The UI reports it in the Options footer instead of staging a value
//!   the user never chose.
//!
//! The in-flight bookkeeping (single-flight admission, the pending slot the owner
//! tick drains) is transport state and lives in `cad-desktop-shell::directory`.

/// The IPC name the Options dialog invokes.
pub const PICK_DIRECTORY_COMMAND: &str = "cad_pick_directory";

/// The refusal of a carrier that has no local folder chooser.
///
/// Prefix + selector, exactly the shape the other carrier refusals use
/// (`unsupported-runtime: fontsDir`, `unsupported-runtime: ymodel`), so a caller
/// keys on the marker rather than on prose.
pub const UNSUPPORTED_PICK_DIRECTORY: &str = "unsupported-runtime: pick_directory";

/// Refusal while another pick is still in flight.
///
/// One chooser at a time: a second request would either steal the open dialog or
/// queue a pick nobody asked for. The bound is explicit, like the file
/// interaction admission in `apps/app-linux`.
pub const PICK_DIRECTORY_BUSY: &str = "pick_directory.busy: a directory chooser is already open";

/// The chooser stopped without publishing an answer (its worker never reported).
///
/// A dropped channel must never be read as "the user cancelled".
pub const PICK_DIRECTORY_LOST: &str = "pick_directory.lost: the chooser stopped without an answer";

/// The failure marker of a pick that ran and failed.
///
/// The reason is the host's own technical text (a portal error, a non-local
/// URI); the prefix is stable so the chrome and the logs can tell a failure from
/// a cancellation.
pub fn pick_directory_failed(reason: &str) -> String {
    format!("pick_directory.failed: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_name_is_the_one_the_chrome_invokes() {
        assert_eq!(PICK_DIRECTORY_COMMAND, "cad_pick_directory");
        // The TS transport pins the same literal (`options/fonts-dir.ts`).
        assert!(PICK_DIRECTORY_COMMAND.starts_with("cad_"));
    }

    #[test]
    fn the_refusal_is_a_stable_carrier_marker() {
        assert_eq!(
            UNSUPPORTED_PICK_DIRECTORY,
            "unsupported-runtime: pick_directory"
        );
        assert!(UNSUPPORTED_PICK_DIRECTORY.starts_with("unsupported-runtime:"));
        // Never the generic "no such wire command": the row exists on every
        // carrier and the answer must name the capability.
        assert!(UNSUPPORTED_PICK_DIRECTORY.contains("pick_directory"));
    }

    #[test]
    fn failures_carry_their_own_prefix_and_are_not_cancellations() {
        let error = pick_directory_failed("portal is unavailable");
        assert_eq!(
            error, "pick_directory.failed: portal is unavailable",
            "the reason is relayed verbatim under a stable prefix"
        );
        assert_ne!(error, PICK_DIRECTORY_BUSY);
        assert_ne!(error, PICK_DIRECTORY_LOST);
        // The three failure markers share the `pick_directory.` namespace and
        // none of them is empty prose.
        for marker in [PICK_DIRECTORY_BUSY, PICK_DIRECTORY_LOST] {
            assert!(marker.starts_with("pick_directory."), "{marker}");
        }
    }
}
