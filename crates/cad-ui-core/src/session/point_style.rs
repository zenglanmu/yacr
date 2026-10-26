//! The typed `PDMODE <n>` / `PDSIZE <n>` lines: the drawing POINT display style
//! sysvars.
//!
//! Both are **document writes**, unlike the session-scoped `SNAPUNIT`: the value
//! goes into the open drawing's header (`$PDMODE` / `$PDSIZE`) through the
//! ordinary command funnel, so it bumps the document revision (which is what
//! refreshes the cached marker style) and records **no** undo entry (`cad-history`
//! has no header patch kind). The command layer owns the validation; this module
//! only parses the typed argument with the one grammar in
//! [`super::parse_pdmode_argument`] / [`super::parse_pdsize_argument`] and
//! dispatches the corresponding payload.
//!
//! `PDMODE` is an **integer** field: a fractional `PDMODE 3.5` is refused with a
//! stable reason rather than rounded, because `$PDMODE` is a bit field and
//! rounding would draw a marker nobody asked for.
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget; the argument-selector dispatch
//! ([`CadSession::argument_command`]) lives here for the same reason.

use cad_app::{CommandId, CommandPayload};

use super::diagnostics::cad_error_text;
use super::{CadSession, ARGUMENT_SELECTOR, PDMODE_SELECTOR, PDSIZE_SELECTOR, SNAPUNIT_SELECTOR};

impl CadSession {
    /// Dispatch one typed `SELECTOR <argument>` line to its own command path.
    ///
    /// The splitter ([`super::typed_command_argument`]) has already admitted the
    /// base selector and kept the argument text verbatim, so this is the single
    /// place that decides which path an argument-taking selector takes. An
    /// unlisted selector keeps the `ZOOM` path, which is the only remaining
    /// argument-taking selector by contract (`ARGUMENT_SELECTORS`).
    pub(super) fn argument_command(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            SNAPUNIT_SELECTOR => self.snap_unit_argument(argument, text),
            PDMODE_SELECTOR | PDSIZE_SELECTOR => {
                self.point_style_argument(selector, argument, text)
            }
            _ => {
                debug_assert_eq!(selector, ARGUMENT_SELECTOR);
                self.typed_argument_command(selector, argument, text)
            }
        }
    }

    /// Apply one typed `PDMODE <n>` / `PDSIZE <n>` line.
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | `PDMODE` whole number in `i16` (`0`, `3`, `35`, `-1`) | store the **raw** `$PDMODE` in the drawing header |
    /// | `PDSIZE` finite decimal (`2.5`, `0`, `-20`) | store the **raw** `$PDSIZE` in the drawing header (`> 0` absolute, `0` viewport-relative, `< 0` percent of the viewport height) |
    /// | anything else (`PDMODE 3.5`, `PDMODE 40000`, `PDMODE abc`, `PDSIZE NaN`, `PDSIZE 2 3`) | the stable [`super::invalid_pdmode_argument_reason`] / [`super::invalid_pdsize_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// A bare `PDMODE`/`PDSIZE` never reaches this method: it has no argument, so
    /// it keeps the name-only payload refusal of the application layer (an
    /// explicit "needs one numeric argument" error, never a fabricated default).
    fn point_style_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let payload = if selector == PDMODE_SELECTOR {
            match super::parse_pdmode_argument(argument) {
                Some(mode) => CommandPayload::PointDisplayMode(mode),
                None => return self.refuse_argument(super::invalid_pdmode_argument_reason(text)),
            }
        } else {
            match super::parse_pdsize_argument(argument) {
                Some(size) => CommandPayload::PointDisplaySize(size),
                None => return self.refuse_argument(super::invalid_pdsize_argument_reason(text)),
            }
        };
        let id = if selector == PDMODE_SELECTOR {
            CommandId::SetPointDisplayMode
        } else {
            CommandId::SetPointDisplaySize
        };
        // Echo the admitted selector name (never the typed value), exactly like
        // the name-only path.
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Record a stable argument refusal and return it (nothing changes).
    fn refuse_argument(&self, reason: String) -> Result<(), String> {
        *self.error.borrow_mut() = Some(reason.clone());
        Err(reason)
    }
}
