//! Document-level summary metadata (`DWGPROPS`: title, author, subject,
//! keywords, comments and custom properties).
//!
//! These are **document scalars**, not element or table rows, so they live in
//! the drawing header beside `$LTSCALE`/`$PDMODE` and are encoded into
//! `document.json` → `variables` by the snapshot header codec. The model is
//! deliberately closed: exactly the five named summary fields AutoCAD's
//! Drawing Properties dialog exposes plus an ordered custom-property map. Any
//! other metadata (statistics, hyperlink base, custom object properties) is not
//! modeled and is never fabricated.
//!
//! # Storage rules
//!
//! * a `Some("")` value is **not** normalized to `None` here; the command layer
//!   decides that an empty write means "clear" and passes `None`;
//! * the setter compares the whole value: writing the stored metadata is an
//!   authoritative no-op (no revision bump);
//! * a real change is exactly one revision bump, an empty-object [`ChangeSet`]
//!   and a header-only [`ChangeScope`]. `cad-history` has no document-metadata
//!   patch kind, so the command layer records no undo step rather than
//!   fabricating one it could not replay;
//! * the value is persisted in `document.json`, and a document without metadata
//!   writes no `metadata` key, so its archive is byte-identical to a pre-metadata
//!   archive.

use std::collections::BTreeMap;

use cad_domain::{Revision, TransactionId};

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::scope::ChangeScope;

/// One of the five named summary fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentMetadataField {
    Title,
    Author,
    Subject,
    Keywords,
    Comments,
}

impl DocumentMetadataField {
    /// The stable lowercase key used by `document.json` and by the command
    /// layer's diagnostics. It is the one spelling both sides share.
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Author => "author",
            Self::Subject => "subject",
            Self::Keywords => "keywords",
            Self::Comments => "comments",
        }
    }
}

/// AutoCAD Drawing Properties ▸ Summary plus custom properties.
///
/// `Default` is "no metadata stored": every named field is `None` and the
/// custom map is empty. That is the state of every document this build opens
/// from a source that carries no summary information.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub comments: Option<String>,
    /// Custom properties, sorted by name. Names are unique by construction
    /// (`BTreeMap`); the command layer refuses an empty name.
    pub custom: BTreeMap<String, String>,
}

impl DocumentMetadata {
    /// Whether nothing at all is stored.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.subject.is_none()
            && self.keywords.is_none()
            && self.comments.is_none()
            && self.custom.is_empty()
    }

    /// Read one named field.
    pub fn field(&self, field: DocumentMetadataField) -> Option<&str> {
        match field {
            DocumentMetadataField::Title => self.title.as_deref(),
            DocumentMetadataField::Author => self.author.as_deref(),
            DocumentMetadataField::Subject => self.subject.as_deref(),
            DocumentMetadataField::Keywords => self.keywords.as_deref(),
            DocumentMetadataField::Comments => self.comments.as_deref(),
        }
    }

    /// Set (or clear, with `None`) one named field.
    pub fn set_field(&mut self, field: DocumentMetadataField, value: Option<String>) {
        match field {
            DocumentMetadataField::Title => self.title = value,
            DocumentMetadataField::Author => self.author = value,
            DocumentMetadataField::Subject => self.subject = value,
            DocumentMetadataField::Keywords => self.keywords = value,
            DocumentMetadataField::Comments => self.comments = value,
        }
    }
}

impl DrawingDatabase {
    /// The document's summary metadata, or the empty value when it stores none.
    pub fn document_metadata(&self) -> &DocumentMetadata {
        &self.document_metadata
    }

    /// Replace the document's summary metadata.
    ///
    /// The caller validates the text bounds before calling this (the command
    /// layer owns the user-facing refusals); this is the single header write.
    /// Writing the stored value is a no-op: `before == after`, no revision bump
    /// and no dirty header. A real change is one revision and a header-only
    /// scope, exactly like the other `document.json` scalars.
    pub fn set_document_metadata(
        &mut self,
        metadata: DocumentMetadata,
        transaction: TransactionId,
        reason: &str,
    ) -> ChangeSet {
        let before = self.revision;
        if self.document_metadata == metadata {
            return ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
                scope: ChangeScope::default(),
            };
        }
        self.document_metadata = metadata;
        // The metadata is not geometry, but every header write follows the same
        // "clear the revision-tagged derived caches" rule.
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.header();
        ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        }
    }
}
