//! Read-only reports for the extended command batch.
//!
//! `BCOUNT`, `COUNT`, `EDITTIME`, `JULIAN` and `XLIST` open no transaction, bump
//! no revision, record no `cad-history` entry and publish no `ChangeSet`, so all
//! five are Viewer-legal. Each handler states its declared divergence or missing
//! source explicitly instead of inventing data.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::app_utility::{document_drawing, format_utc, require_none_payload};

pub(crate) fn report_outcome(diagnostics: Vec<Diagnostic>) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

pub(crate) fn diagnostic(code: &str, message: String) -> Diagnostic {
    Diagnostic {
        object: None,
        code: code.into(),
        message,
    }
}

impl Application {
    /// Dispatch one read-only extended report.
    pub(crate) fn extended_report(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::BlockCount => self.block_count(command),
            ExtendedCommandId::CountObjects => self.count_objects(session, command),
            ExtendedCommandId::EditTime => self.edit_time(command),
            ExtendedCommandId::JulianDate => self.julian_date(command),
            ExtendedCommandId::XList => self.xlist(command),
            other => Err(CadError::Invariant(format!(
                "not an extended report: {other:?}"
            ))),
        }
    }

    /// `BCOUNT`: count model-space INSERT instances grouped by block name.
    ///
    /// Declared divergence: this counts **whole model space**, never a
    /// `Select objects` prompt. Block-instance members are not expanded.
    fn block_count(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "BCOUNT")?;
        let drawing = document_drawing(self, command)?;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entity in drawing.model_space() {
            let SemanticGeometry::Insert { block, .. } = &entity.geometry else {
                continue;
            };
            let name = drawing
                .block_name(*block)
                .map(str::to_string)
                .unwrap_or_else(|| format!("block#{}", block.0));
            *counts.entry(name).or_default() += 1;
        }
        let total: usize = counts.values().sum();
        let distinct = counts.len();
        let mut diagnostics = Vec::with_capacity(distinct + 1);
        for (name, instances) in &counts {
            diagnostics.push(diagnostic(
                "bcount.block",
                format!("BCOUNT: block '{name}' instances={instances}"),
            ));
        }
        diagnostics.push(diagnostic(
            "bcount.summary",
            format!(
                "BCOUNT: {total} block instance(s) across {distinct} distinct block(s) in the \
                 whole model space (declared divergence: no Select objects prompt)"
            ),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `COUNT`: count the current selection, or all model-space entities when the
    /// selection is empty. The selection is never mutated.
    fn count_objects(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "COUNT")?;
        let drawing = document_drawing(self, command)?;
        let using_selection = !session.selection.is_empty();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut unresolved = 0usize;
        if using_selection {
            for reference in session.selection.refs() {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    unresolved += 1;
                    continue;
                }
                match drawing.entity(reference.entity) {
                    Some(entity) => {
                        *counts.entry(entity.object.type_key.clone()).or_default() += 1;
                    }
                    None => unresolved += 1,
                }
            }
        } else {
            for entity in drawing.model_space() {
                *counts.entry(entity.object.type_key.clone()).or_default() += 1;
            }
        }
        let total: usize = counts.values().sum();
        let distinct = counts.len();
        let scope = if using_selection {
            "the current selection"
        } else {
            "model space"
        };
        let mut diagnostics = Vec::with_capacity(distinct + 2);
        for (type_key, count) in &counts {
            diagnostics.push(diagnostic(
                "count.type",
                format!("COUNT: type {type_key} = {count}"),
            ));
        }
        diagnostics.push(diagnostic(
            "count.summary",
            format!("COUNT: {total} object(s) in {scope} across {distinct} type(s)"),
        ));
        if unresolved > 0 {
            diagnostics.push(diagnostic(
                "count.unresolved",
                format!(
                    "COUNT: {unresolved} selected reference(s) are block-instance members, \
                     sub-elements or absent entities and were not counted"
                ),
            ));
        }
        Ok(report_outcome(diagnostics))
    }

    /// `EDITTIME`: read-only clock/document report.
    ///
    /// The database stores no creation/edit timestamps and this build tracks no
    /// session uptime, so both are named `edittime.not_tracked` diagnostics.
    fn edit_time(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "EDITTIME")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::with_capacity(3);
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => {
                let seconds = elapsed.as_secs();
                diagnostics.push(diagnostic(
                    "edittime.report",
                    format!(
                        "EDITTIME: current UTC {} (unix {seconds}); document {} revision {} \
                         entities {}",
                        format_utc(seconds),
                        command.document.0,
                        drawing.revision().0,
                        drawing.entity_count()
                    ),
                ));
            }
            Err(_) => diagnostics.push(diagnostic(
                "edittime.clock_unavailable",
                "EDITTIME: the system clock is before the Unix epoch; no UTC time was fabricated"
                    .into(),
            )),
        }
        diagnostics.push(diagnostic(
            "edittime.not_tracked",
            "EDITTIME: drawing creation/edit timestamps are not tracked by this database".into(),
        ));
        diagnostics.push(diagnostic(
            "edittime.not_tracked",
            "EDITTIME: session uptime is not tracked by this build".into(),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `JULIAN`: the Julian day number of the current UTC instant.
    ///
    /// No date argument is parsed (declared partial); only "now" is reported.
    fn julian_date(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "JULIAN")?;
        // A read-only report still validates the document exists.
        let _drawing = document_drawing(self, command)?;
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => {
                let seconds =
                    elapsed.as_secs() as f64 + f64::from(elapsed.subsec_nanos()) / 1_000_000_000.0;
                // Julian Day Number at the Unix epoch is 2440587.5.
                let julian = 2_440_587.5 + seconds / 86_400.0;
                let day = julian.floor();
                let fraction = julian - day;
                Ok(report_outcome(vec![diagnostic(
                    "julian.value",
                    format!(
                        "JULIAN: {julian:.6} (integer day {}, fraction {fraction:.6}); only the \
                         current UTC instant is supported, date arguments are NOT IMPLEMENTED",
                        day as i64
                    ),
                )]))
            }
            Err(_) => Ok(report_outcome(vec![diagnostic(
                "julian.unavailable",
                "JULIAN: the system clock is before the Unix epoch; no Julian day was fabricated"
                    .into(),
            )])),
        }
    }

    /// `XLIST`: list every block definition with its member count.
    ///
    /// XDATA is **NOT RETAINED** by this database, so the report says so instead
    /// of inventing an empty list.
    fn xlist(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "XLIST")?;
        let drawing = document_drawing(self, command)?;
        let mut definitions = 0usize;
        let mut diagnostics = Vec::new();
        for block in drawing.blocks() {
            definitions += 1;
            let name = drawing
                .block_name(block.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("block#{}", block.id.0));
            diagnostics.push(diagnostic(
                "xlist.block",
                format!("XLIST: block '{name}' members={}", block.entities.len()),
            ));
        }
        diagnostics.push(diagnostic(
            "xlist.summary",
            format!("XLIST: {definitions} block definition(s)"),
        ));
        diagnostics.push(diagnostic(
            "xlist.xdata_not_retained",
            "XLIST: entity XDATA is not retained by this database and is not listed".into(),
        ));
        Ok(report_outcome(diagnostics))
    }
}
