//! Wave 7 **dynamic-block visibility reports** for the extended batch.
//!
//! `BVSTATE` and `-BVSTATE` are read-only: they report the active
//! dynamic-block visibility state of every directly selected INSERT through
//! `DrawingDatabase::block_dynamic_visibility`, the read half of the existing
//! `set_block_visibility_state` authority. They never switch a state, create a
//! state or write the block table.
//!
//! A selection with no INSERT is `*.no_selection`; an INSERT whose block records
//! no visibility descriptor, or whose descriptor has no resolved active state,
//! is `*.not_set`. Nothing is invented: a `None` active state is reported as
//! unresolved rather than guessed.
//!
//! Neither command opens a transaction, bumps the revision, records a
//! `cad-history` entry or publishes a `ChangeSet`; both are Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 7 dynamic-block visibility report.
    pub(crate) fn extended_dyn7(
        &self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::BvState => self.bvstate(session, command, BvStateReport::BvState),
            ExtendedCommandId::DashBvState => {
                self.bvstate(session, command, BvStateReport::DashBvState)
            }
            other => Err(CadError::Invariant(format!(
                "not a Wave 7 visibility report: {other:?}"
            ))),
        }
    }

    /// `BVSTATE` / `-BVSTATE`: report each selected INSERT's active visibility
    /// state, or name why there is nothing to report.
    fn bvstate(
        &self,
        session: &SessionState,
        command: &Command,
        report: BvStateReport,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, report.selector())?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut inserts = 0usize;
        for reference in session.selection.refs() {
            // A nested member selection is not a directly selected INSERT; this
            // report does not resolve instance paths and does not guess one.
            if !reference.instance.0.is_empty() {
                continue;
            }
            let Some(entity) = drawing.entity(reference.entity) else {
                continue;
            };
            let SemanticGeometry::Insert { block, .. } = &entity.geometry else {
                continue;
            };
            inserts += 1;
            let name = drawing
                .block_name(*block)
                .map(str::to_string)
                .unwrap_or_else(|| format!("block#{}", block.0));
            let Some(visibility) = drawing.block_dynamic_visibility(*block) else {
                diagnostics.push(diagnostic(
                    report.not_set_code(),
                    format!(
                        "{}: selected INSERT entity {} references block '{name}' id {}, but that \
                         block stores no dynamic-block visibility descriptor; no state is set and \
                         nothing was invented",
                        report.selector(),
                        entity.id.0,
                        block.0
                    ),
                ));
                continue;
            };
            let Some(active) = visibility.active_state.as_deref() else {
                diagnostics.push(diagnostic(
                    report.not_set_code(),
                    format!(
                        "{}: selected INSERT entity {} references block '{name}' id {}, but no \
                         active visibility state was resolved; nothing was invented and no state \
                         was set",
                        report.selector(),
                        entity.id.0,
                        block.0
                    ),
                ));
                continue;
            };
            let states = visibility.state_names().join(", ");
            diagnostics.push(diagnostic(
                report.report_code(),
                format!(
                    "{}: selected INSERT entity {} references block '{name}' id {} with active \
                     visibility state '{active}' (available: {states}); read-only report, the \
                     state was not changed",
                    report.selector(),
                    entity.id.0,
                    block.0
                ),
            ));
        }
        if inserts == 0 {
            diagnostics.push(diagnostic(
                report.no_selection_code(),
                format!(
                    "{}: no INSERT is selected, so there is no dynamic-block visibility state to \
                     report; nothing was changed",
                    report.selector()
                ),
            ));
        }
        Ok(report_outcome(diagnostics))
    }
}

/// Which visibility-state report is running (`BVSTATE` vs `-BVSTATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BvStateReport {
    /// `BVSTATE`: `bvstate.report` / `bvstate.no_selection` / `bvstate.not_set`.
    BvState,
    /// `-BVSTATE`: the same report under `-bvstate.*` codes.
    DashBvState,
}

impl BvStateReport {
    fn selector(self) -> &'static str {
        match self {
            Self::BvState => "BVSTATE",
            Self::DashBvState => "-BVSTATE",
        }
    }

    fn report_code(self) -> &'static str {
        match self {
            Self::BvState => "bvstate.report",
            Self::DashBvState => "-bvstate.report",
        }
    }

    fn no_selection_code(self) -> &'static str {
        match self {
            Self::BvState => "bvstate.no_selection",
            Self::DashBvState => "-bvstate.no_selection",
        }
    }

    fn not_set_code(self) -> &'static str {
        match self {
            Self::BvState => "bvstate.not_set",
            Self::DashBvState => "-bvstate.not_set",
        }
    }
}
