//! Wave 3 **reports / session** handlers for the extended command batch.
//!
//! `MSPACE` and `PSPACE` are session-only space switches that reuse the one
//! `switch_space_command` path `MODEL`/`SPACE` already own — there is no second
//! space authority. `STYLE`, `-STYLE`, `MLEADERSTYLE` and `BLOCK?` are read-only
//! reports over tables this build really stores; a missing table is named, never
//! invented. `DIMREASSOC` is a real entity write (restore the measured dimension
//! text; see `app_drawing::dimension_reassoc`), while `DIMREASSOCIATE` keeps
//! `DIMREGEN`'s contract: this build's dimensions are non-associative, so it is
//! an explicit no-op.
//!
//! Apart from `DIMREASSOC`, none of these opens a transaction, bumps the
//! revision, records a `cad-history` entry or publishes a `ChangeSet`; all of
//! them are Viewer-legal except the Work-only `DIMREASSOC`.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 3 report/session command.
    pub(crate) fn extended_report3(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::Mspace => self.mspace(session, command),
            ExtendedCommandId::Pspace => self.pspace(session, command),
            ExtendedCommandId::TextStyle => {
                self.text_style_report(session, command, TextStyleReport::Style)
            }
            ExtendedCommandId::DashStyle => {
                self.text_style_report(session, command, TextStyleReport::DashStyle)
            }
            ExtendedCommandId::MLeaderStyleList => self.mleader_style_report(session, command),
            ExtendedCommandId::BlockList => self.block_list_report(command),
            ExtendedCommandId::DimReassoc => self.dim_reassoc(session, command),
            ExtendedCommandId::DimReassociate => self.non_associative_report(
                command,
                "DIMREASSOCIATE",
                "dimreassociate.non_associative",
            ),
            other => Err(CadError::Invariant(format!(
                "not a Wave 3 report: {other:?}"
            ))),
        }
    }

    /// `MSPACE`: switch to model space through the existing `MODEL` path.
    ///
    /// Already in model space is a named no-op, not a second switch.
    fn mspace(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "MSPACE")?;
        let _drawing = document_drawing(self, command)?;
        if matches!(session.active_space, SpaceId::Model) {
            return Ok(report_outcome(vec![diagnostic(
                "mspace.noop",
                "MSPACE: the session is already in model space; nothing changed".into(),
            )]));
        }
        let mut outcome = self.switch_space_command(session, command, SpaceId::Model)?;
        outcome.diagnostics.push(diagnostic(
            "mspace.switched",
            "MSPACE: switched to model space (session state only)".into(),
        ));
        Ok(outcome)
    }

    /// `PSPACE`: switch to paper space through the existing `SPACE` path.
    ///
    /// From model space this lands on the drawing's first paper layout (id order
    /// is tab order); a drawing with no paper layout is a named report, not a
    /// blank success. Already being in a paper layout is a **named no-op**
    /// (`pspace.noop`): the command's contract is "from model space to paper
    /// space", so a second PSPACE must not jump to the *first* layout and re-fit
    /// the camera of the sheet the user is standing on. `MSPACE` has the
    /// symmetric no-op.
    fn pspace(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "PSPACE")?;
        if let SpaceId::Paper(current) = session.active_space.clone() {
            let name = document_drawing(self, command)?
                .layout(current)
                .map(|layout| layout.name.clone());
            let where_ = match name {
                Some(name) => format!("layout {current:?} ({name:?})"),
                None => format!("layout {current:?} (which this drawing no longer stores)"),
            };
            return Ok(report_outcome(vec![diagnostic(
                "pspace.noop",
                format!(
                    "PSPACE: the session is already in paper space on {where_}; nothing changed"
                ),
            )]));
        }
        let mut target = None;
        if let Some(layout) = document_drawing(self, command)?.layouts().next() {
            target = Some((layout.id, layout.name.clone()));
        }
        let Some((layout, name)) = target else {
            return Ok(report_outcome(vec![diagnostic(
                "pspace.no_layout",
                "PSPACE: this drawing has no paper layout, so there is nowhere to switch; \
                 nothing changed"
                    .into(),
            )]));
        };
        let mut outcome = self.switch_space_command(session, command, SpaceId::Paper(layout))?;
        outcome.diagnostics.push(diagnostic(
            "pspace.switched",
            format!("PSPACE: switched to paper layout '{name}' (session state only)"),
        ));
        Ok(outcome)
    }

    /// `STYLE` / `-STYLE`: the read-only list, or a typed table write.
    ///
    /// A bare selector (or the `-STYLE` alias) lists the drawing's style table —
    /// the authority (`drawing.styles()`). A typed payload
    /// (`NEW`/`RENAME`/`DELETE`/`SET`) goes to the shared post-import writer
    /// ([`Application::style_table_command`]), one transaction / one undo step.
    fn text_style_report(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        report: TextStyleReport,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => self.list_text_styles(command, report),
            CommandPayload::StyleTable(_) => self.style_table_command(session, command),
            _ => Err(CadError::InvalidInput(format!(
                "{} takes no payload or its typed style-table form",
                report.selector()
            ))),
        }
    }

    /// The read-only text-style list.
    fn list_text_styles(
        &self,
        command: &Command,
        report: TextStyleReport,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for style in drawing.styles() {
            count += 1;
            diagnostics.push(diagnostic(
                report.entry_code(),
                format!(
                    "{}: text style '{}' id {} resource(s) {}",
                    report.selector(),
                    style.name,
                    style.id.0,
                    style.resource_keys.len()
                ),
            ));
        }
        diagnostics.push(diagnostic(
            report.summary_code(),
            format!(
                "{}: {count} text style(s) from the drawing's style table; write forms are \
                 NEW/RENAME/DELETE/SET (post-import table writes, one undo step each)",
                report.selector()
            ),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `MLEADERSTYLE`: the read-only list, or a typed table/session write.
    ///
    /// `SET <name>` writes the session's current multileader style (creation
    /// reads it); `NEW`/`RENAME`/`DELETE`/`MODIFY` write the table through the
    /// shared post-import writer.
    fn mleader_style_report(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => self.list_mleader_styles(command),
            CommandPayload::StyleTable(_) => self.style_table_command(session, command),
            _ => Err(CadError::InvalidInput(
                "MLEADERSTYLE takes no payload or its typed style-table form".into(),
            )),
        }
    }

    /// The read-only multileader-style list.
    fn list_mleader_styles(&self, command: &Command) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for style in drawing.mleader_styles() {
            count += 1;
            diagnostics.push(diagnostic(
                "mleaderstyle.entry",
                format!(
                    "MLEADERSTYLE: style '{}' content_type {} text_height {} arrowhead {} \
                     text_style '{}'",
                    style.name,
                    style.content_type,
                    style.text_height,
                    style.arrowhead_size,
                    style.text_style_name
                ),
            ));
        }
        diagnostics.push(diagnostic(
            "mleaderstyle.summary",
            format!(
                "MLEADERSTYLE: {count} multileader style(s) from the drawing's table; SET makes \
                 one current, and NEW/RENAME/DELETE/MODIFY write the table (one undo step each)"
            ),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `BLOCK?`: read-only list of block definitions and their member counts.
    ///
    /// Reuses the same `drawing.blocks()` access `XLIST` reads. This is a
    /// listing, not the block editor.
    fn block_list_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "BLOCK?")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for block in drawing.blocks() {
            count += 1;
            let name = drawing
                .block_name(block.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("block#{}", block.id.0));
            diagnostics.push(diagnostic(
                "block.entry",
                format!("BLOCK?: block '{name}' members={}", block.entities.len()),
            ));
        }
        diagnostics.push(diagnostic(
            "block.summary",
            format!(
                "BLOCK?: {count} block definition(s); this is a read-only listing, not the \
                 block editor"
            ),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `DIMREASSOCIATE`: the non-associative no-op report (its own code; the
    /// text-restoring `DIMREASSOC` is a real write and is not an alias of it).
    fn non_associative_report(
        &self,
        command: &Command,
        selector: &str,
        code: &str,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, selector)?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            code,
            format!(
                "{selector}: this build stores non-associative dimensions; there is no \
                 association to restore or create and nothing changed"
            ),
        )]))
    }
}

/// Which text-style report is running (`STYLE` vs `-STYLE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextStyleReport {
    /// `STYLE`: `style.entry` / `style.summary`.
    Style,
    /// `-STYLE`: `-style.entry` / `-style.summary`.
    DashStyle,
}

impl TextStyleReport {
    fn selector(self) -> &'static str {
        match self {
            Self::Style => "STYLE",
            Self::DashStyle => "-STYLE",
        }
    }

    fn entry_code(self) -> &'static str {
        match self {
            Self::Style => "style.entry",
            Self::DashStyle => "-style.entry",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::Style => "style.summary",
            Self::DashStyle => "-style.summary",
        }
    }
}
