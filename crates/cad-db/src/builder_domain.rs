//! The builder's document-level scalars: external-resource self-containment
//! (ADR-0040 D3) and the application domain (ADR-0053 D2). Split out of
//! `builder.rs` (at the file-size gate).
//!
//! Both values are document state: the builder seeds them before `finish()`, and
//! `snapshot/build.rs` carries them into `document.json`. `false`/`None` are not
//! written, so a document that never opted in keeps the older bytes.

use cad_domain::ApplicationDomain;

use crate::DrawingDatabaseBuilder;

impl DrawingDatabaseBuilder {
    /// Set whether this document embeds external resources (ADR-0040 D3).
    ///
    /// Document state, not a carrier preference: it is written into
    /// `document.json` and seeded back on load. `false` (the default) is not
    /// written at all, so a document that packs nothing keeps the bytes of a
    /// minor ≤ 4 archive.
    pub fn set_embed_resources(&mut self, embed: bool) {
        self.database.embed_resources = embed;
    }

    /// Name this document's interface domain, or `None` for an unnamed document.
    ///
    /// `None` keeps the document at its previous minor (the key is not written)
    /// and lets [`crate::DrawingDatabase::effective_application_domain`] derive a
    /// conservative answer.
    pub fn set_application_domain(&mut self, domain: Option<ApplicationDomain>) {
        self.database.application_domain = domain;
    }
}
