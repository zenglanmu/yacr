//! The `ACADINFO` information snapshot and its versioned file shape.
//!
//! AutoCAD's `ACADINFO` creates a file that describes the installation and the
//! current setup. This build has no licence, hardware probe or installed-module
//! registry (those are global-scope omissions, named in the report), but it does
//! have a real product/version/build identity and a live document header, so the
//! representable part is one small versioned JSON document written through the
//! host's named-file bridge (`cad-ui-core` owns the bytes/write; this module is
//! the one reader of the facts).
//!
//! The file is **not** AutoCAD's prose `acadinfo.txt`: the format tag
//! (`yacr.acadinfo`) and version are part of the contract, and an unsupported
//! version is refused by the reader instead of being guessed at.

use cad_domain::UnitContext;
use serde_json::{json, Value};

use crate::{DocumentId, DrawingDatabase};

/// The tag carried by an `ACADINFO` information file.
pub const ACADINFO_FILE_FORMAT: &str = "yacr.acadinfo";

/// The only accepted information-file version.
pub const ACADINFO_FILE_VERSION: u32 = 1;

/// The facts this build can honestly report about its installation and the
/// current document.
#[derive(Debug, Clone, PartialEq)]
pub struct AcadInfoSnapshot {
    pub document: DocumentId,
    pub revision: u64,
    pub entities: usize,
    /// The declared display unit label (`drawing units`, `mm`, …).
    pub units: &'static str,
    pub decimal_places: u8,
    /// The document's interface domain token, or `None` when unnamed.
    pub application_domain: Option<&'static str>,
    pub product_version: &'static str,
    pub build: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
}

impl AcadInfoSnapshot {
    /// Read the live document header.
    pub fn from_parts(
        document: DocumentId,
        drawing: &DrawingDatabase,
        units: &UnitContext,
    ) -> Self {
        AcadInfoSnapshot {
            document,
            revision: drawing.revision().0,
            entities: drawing.entity_count(),
            units: units.label(),
            decimal_places: units.decimal_places,
            application_domain: drawing.application_domain().map(|domain| domain.as_str()),
            product_version: env!("CARGO_PKG_VERSION"),
            build: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
        }
    }

    /// The bare-command report line (the same facts the file carries).
    pub fn report_line(&self) -> String {
        format!(
            "ACADINFO: product yacr (cad-app) version {} build {}; target {} {}; document {} \
             revision {} entities {}; units {} ({} decimal place(s)); application domain {}",
            self.product_version,
            self.build,
            self.target_os,
            self.target_arch,
            self.document.0,
            self.revision,
            self.entities,
            self.units,
            self.decimal_places,
            self.application_domain.unwrap_or("none")
        )
    }

    /// The versioned information file.
    ///
    /// Stable keys, no timestamps (this build tracks no document creation or
    /// update clock, so writing one would invent a value) and no licence or
    /// hardware rows: `notCollected` names those omissions instead of omitting
    /// them silently.
    pub fn to_file_value(&self) -> Value {
        json!({
            "format": ACADINFO_FILE_FORMAT,
            "version": ACADINFO_FILE_VERSION,
            "product": {
                "name": "yacr",
                "component": "cad-app",
                "version": self.product_version,
                "build": self.build,
                "targetOs": self.target_os,
                "targetArch": self.target_arch,
            },            "document": {
                "id": self.document.0.to_string(),
                "revision": self.revision,
                "entities": self.entities,
                "units": self.units,
                "decimalPlaces": self.decimal_places,
                "applicationDomain": self.application_domain,
            },
            "notCollected": ["licence", "hardware", "installedModules"],
        })
    }
}

impl crate::Application {
    /// The live `ACADINFO` facts for one document, or `None` when it is not
    /// open.
    ///
    /// `cad-ui-core` calls this for the information-file form so the report and
    /// the file read the same snapshot instead of building two copies.
    pub fn acad_info_snapshot(&self, document: DocumentId) -> Option<AcadInfoSnapshot> {
        let document_state = self.workspace.documents.get(&document)?;
        Some(AcadInfoSnapshot::from_parts(
            document,
            &document_state.drawing,
            &document_state.units,
        ))
    }
}
