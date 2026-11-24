//! The `cad_set_application_domain` session surface (ADR-0053 D2).
//!
//! The domain is **document** metadata, not a carrier preference: the session
//! forwards the token to the one host method that rewrites `document.json`
//! (`ChangeScope::header`) and surfaces a real refusal through `cad_status.error`.
//! A carrier with no document lane refuses by name through the sink default
//! ([`super::wire::APPLICATION_DOMAIN_UNAVAILABLE`]); the session never invents a
//! success for a write it did not make.
//!
//! The token table lives in `cad-domain`; an unknown value is a named refusal
//! (`domain.unknown: <token>`), never a normalized default.

use cad_domain::ApplicationDomain;

use super::admission::check_application_domain;
use super::wire::{AdmissionTarget, CadCommandAck, Channel};
use super::{cad_error_text, CadSession};

/// `cad_set_application_domain`: validate and queue/apply one domain token.
///
/// The shared gate refuses a modal popup (`domain.busy`), an empty token and a
/// full queue; the session owns the token table and refuses an unknown one with
/// `domain.unknown`. The acknowledgement means queued (desktop) or applied
/// (`dom-canvas`), exactly like every other sink write.
pub fn set_application_domain(
    target: &mut dyn AdmissionTarget,
    token: &str,
) -> Result<CadCommandAck, String> {
    check_application_domain(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::ApplicationDomain),
        token,
    )?;
    target.application_domain(token.to_string())?;
    Ok(CadCommandAck::accepted())
}

impl CadSession {
    /// Store the document's application domain from a stable token.
    ///
    /// One document revision when the value changes; an unchanged value is a
    /// no-op. The result is the accepted domain, so a caller can see exactly
    /// which profile the host now holds.
    pub fn set_application_domain(&self, token: &str) -> Result<ApplicationDomain, String> {
        match self.core.set_application_domain(token) {
            Ok(domain) => {
                *self.error.borrow_mut() = None;
                Ok(domain)
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// The domain the chrome renders for the current document, or `None` without
    /// a document.
    pub fn application_domain(&self) -> Option<ApplicationDomain> {
        self.core.effective_application_domain()
    }
}
