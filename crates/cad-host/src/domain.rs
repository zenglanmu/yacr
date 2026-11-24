//! The document's application domain over the host seam (ADR-0053 D2/D4).
//!
//! The document owns the value (`document.json`), `cad-app` owns the header
//! rewrite and `cad-db` owns the derived default. This module is only the
//! `HostCore` face the session/wire layer calls, so no carrier re-implements the
//! rule and a `dom-canvas` session answers exactly like a native one.

use cad_domain::{ApplicationDomain, CadResult};

use crate::HostCore;

impl HostCore {
    /// The domain the document names, or `None` when it never named one.
    pub fn application_domain(&self) -> Option<ApplicationDomain> {
        self.controller.borrow().application_domain()
    }

    /// The domain the chrome should render, deriving a conservative one when the
    /// document never named a domain.
    pub fn effective_application_domain(&self) -> Option<ApplicationDomain> {
        self.controller.borrow().effective_application_domain()
    }

    /// Set the document's domain from a stable token (`cad2d`/`bim`/`concept`/
    /// `mechanical`). One revision when it changes; `domain.unknown` for anything
    /// else.
    pub fn set_application_domain(&self, token: &str) -> CadResult<ApplicationDomain> {
        self.controller.borrow_mut().set_application_domain(token)
    }
}
