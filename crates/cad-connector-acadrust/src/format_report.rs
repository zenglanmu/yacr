//! Source-format fidelity and object-level EED reporting.
//!
//! Split out of `builder.rs` to stay inside the per-file line budget; the
//! methods are inherent on [`ImporterBuilder`] and are called from
//! [`ImporterBuilder::run`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// Record source-format fidelity notes: declared version family and code
    /// page exactness.
    ///
    /// These codes deliberately do **not** start with `import.`. [`Self::run`]
    /// folds every `import.` diagnostic into `Completeness::Partial` because that
    /// prefix means "a declared loss", while these two are informational notes:
    /// the R11/R12 family has no version-specific conversion here and an
    /// unfaithful code page may decode non-ASCII text approximately. Neither
    /// proves that content was dropped, so turning them into `Partial` would be a
    /// false loss report. Do not "fix" the prefix.
    pub(crate) fn report_format_fidelity(&mut self) {
        if self.acad.version.is_r11_r12() {
            self.diagnostics.push(Diagnostic {
            object: None,
            code: "format.r11_r12".into(),
            message: format!(
                "source drawing is {} (the R11/R12 AC1009 format family); no version-specific conversion is applied",
                self.acad.version.as_str()
            ),
        });
        }
        let code_page = self.acad.header.code_page.as_str();
        if !code_page.is_empty() && !acadrust::io::dxf::code_page::code_page_is_faithful(code_page)
        {
            self.diagnostics.push(Diagnostic {
            object: None,
            code: "format.code_page_approximate".into(),
            message: format!(
                "code page {code_page} has no exact decoding table; non-ASCII text may be approximate"
            ),
        });
        }
    }

    /// Summarise decoded object-level EED (XDATA) per application.
    ///
    /// Reported only: nothing here changes an entity, layer or table. The count
    /// is objects-per-application (not records), and an application name is
    /// counted at most once per object even when a malformed file repeats it, so
    /// a duplicate record cannot inflate the summary. `BTreeMap` gives the
    /// deterministic name order the report promises. An object whose decoded
    /// data holds no records contributes nothing (and produces no diagnostic).
    pub(crate) fn read_object_xdata(&mut self) -> Vec<ObjectXdataApplication> {
        let mut objects_by_application: BTreeMap<String, usize> = BTreeMap::new();
        let mut objects_with_xdata = 0usize;
        for xdata in self.acad.object_xdata.values() {
            let applications: BTreeSet<&str> = xdata
                .records()
                .iter()
                .map(|record| record.application_name.as_str())
                .collect();
            if applications.is_empty() {
                continue;
            }
            objects_with_xdata += 1;
            for application in applications {
                *objects_by_application
                    .entry(application.to_string())
                    .or_insert(0) += 1;
            }
        }
        if !objects_by_application.is_empty() {
            // At most five names, then an ellipsis, so the message stays bounded
            // for a file with many registered applications.
            let mut listed = Vec::new();
            for application in objects_by_application.keys().take(5) {
                listed.push(application.as_str());
            }
            let mut apps = listed.join(", ");
            if objects_by_application.len() > listed.len() {
                apps.push_str(", …");
            }
            self.diagnostics.push(Diagnostic {
            object: None,
            code: "xdata.object_ignored".into(),
            message: format!(
                "{objects_with_xdata} objects carry decoded object xdata (apps: {apps}); it is reported only and not applied"
            ),
        });
        }
        objects_by_application
            .into_iter()
            .map(|(application, objects)| ObjectXdataApplication {
                application,
                objects,
            })
            .collect()
    }
}
