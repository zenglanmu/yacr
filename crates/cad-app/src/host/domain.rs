//! The document's application domain (ADR-0053): the one document-level scalar
//! that names which interface profile the chrome renders.
//!
//! The field is document state, not a carrier preference, so the write goes
//! through the same header rewrite (`ChangeScope::header`) as the resource
//! self-containment flag: one revision, no row changes, and the next save carries
//! it. A switch from the chrome is therefore an ordinary dirty-document edit.
//!
//! The import path seeds the field from the connector before the document is
//! published; this module owns the *user-facing* write, which validates the
//! token and refuses an unknown one instead of normalizing it.

use cad_domain::ApplicationDomain;

use super::*;

impl HostController {
    /// The domain this document names, or `None` for an unnamed document.
    ///
    /// This is the stored value, not the derived one: a caller that needs a
    /// definite interface reads [`cad_db::DrawingDatabase::effective_application_domain`].
    pub fn application_domain(&self) -> Option<ApplicationDomain> {
        self.drawing()
            .and_then(|drawing| drawing.application_domain())
    }

    /// The domain the chrome should render, deriving one when the document has
    /// never named it.
    pub fn effective_application_domain(&self) -> Option<ApplicationDomain> {
        self.drawing()
            .map(|drawing| drawing.effective_application_domain())
    }

    /// Set the document's application domain from a wire token.
    ///
    /// One revision and a dirty `document.json`; an unchanged value is a
    /// documented no-op. An unknown token is refused with the stable
    /// `domain.unknown` marker — the interface this build cannot render is not a
    /// drafting document. No document is an explicit refusal (there is nothing
    /// to write).
    pub fn set_application_domain(&mut self, token: &str) -> CadResult<ApplicationDomain> {
        let domain = ApplicationDomain::from_token(token)
            .ok_or_else(|| CadError::Unsupported(format!("domain.unknown: {token}")))?;
        if self.drawing().is_none() {
            return Err(CadError::InvalidInput("document not open".into()));
        }
        if self.application_domain() == Some(domain) {
            return Ok(domain);
        }
        let mut scope = cad_db::ChangeScope::default();
        // The value is a `document.json` scalar, not a table row.
        scope.header();
        self.rewrite_document(
            move |snapshot| {
                snapshot.header.application_domain = Some(domain);
                Ok(())
            },
            &scope,
        )?;
        Ok(domain)
    }
}
