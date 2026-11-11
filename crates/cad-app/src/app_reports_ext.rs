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

/// The extended half of `STATUS`: the document/session statistics this build
/// really stores, stated as facts and never as guesses.
///
/// `STATUS`'s handler lives in `app_drawing::inquiry`; this function owns only
/// the additional report lines (the caller appends them to its own
/// `status.summary`/`status.layers`/… set) so the two modules stay inside the
/// code-size limit and the arithmetic exists exactly once.
///
/// What is reported here is **measured**, not modelled-by-assumption:
///
/// * `status.current_space` — the session's active space, resolved through the
///   drawing's own layout/block tables (an unresolved row is named, not hidden);
/// * `status.space_usage` — entity counts per paper-space layout plus the count
///   of block-definition rows, from one walk over the drawing;
/// * `status.layer_usage` — how many distinct layers actually carry entities,
///   against the layer-table size;
/// * `status.modes` — the stored `PDMODE`/`PDSIZE`/`LTSCALE`, the retained
///   measurement kind, the recorded backend preference and the real undo/redo
///   availability;
/// * `status.selection` — selection/hidden/temporary-override/named-group/named
///   view counts (all session state, none of it drawing data).
///
/// Everything AutoCAD's `STATUS` reports that this build does not track stays a
/// `status.not_tracked` line in the caller (limits/base point, creation/edit
/// time, memory); nothing here invents a substitute value.
pub(crate) fn status_extended(
    app: &Application,
    session: &SessionState,
    command: &Command,
    drawing: &cad_db::DrawingDatabase,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::with_capacity(5);

    let current_space = match &session.active_space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => format!("Paper({})", layout_label(drawing, *layout)),
        SpaceId::Block(block) => format!("Block({})", block_label(drawing, *block)),
    };
    let units = app
        .workspace
        .documents
        .get(&command.document)
        .map(|document| format!("{:?}", document.units.source))
        .unwrap_or_else(|| "unavailable".to_string());
    diagnostics.push(diagnostic(
        "status.current_space",
        format!("STATUS current space={current_space} units={units}"),
    ));

    let mut paper_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut block_rows = 0usize;
    let mut used_layers: std::collections::BTreeSet<LayerId> = std::collections::BTreeSet::new();
    for entity in drawing.entities() {
        used_layers.insert(entity.layer);
        match entity.space {
            SpaceId::Model => {}
            SpaceId::Paper(layout) => {
                *paper_counts
                    .entry(layout_label(drawing, layout))
                    .or_default() += 1;
            }
            SpaceId::Block(_) => block_rows += 1,
        }
    }
    let paper_total: usize = paper_counts.values().sum();
    let layouts = if paper_counts.is_empty() {
        "no paper-space entities".to_string()
    } else {
        paper_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    diagnostics.push(diagnostic(
        "status.space_usage",
        format!(
            "STATUS paper-space entities={paper_total} ({layouts}); block-definition rows={block_rows}"
        ),
    ));

    let layer_rows = drawing.layers().count();
    diagnostics.push(diagnostic(
        "status.layer_usage",
        format!(
            "STATUS layers carrying entities={} of {layer_rows} layer row(s)",
            used_layers.len()
        ),
    ));

    diagnostics.push(diagnostic(
        "status.modes",
        format!(
            "STATUS pdmode={} pdsize={} ltscale={} measurement={} backend={:?} undo={} redo={}",
            drawing.point_display_mode(),
            drawing.point_display_size(),
            drawing.linetype_scale(),
            session.selected_measurement_kind().key(),
            session.backend,
            app.can_undo(&command.document),
            app.can_redo(&command.document)
        ),
    ));

    diagnostics.push(diagnostic(
        "status.selection",
        format!(
            "STATUS selection={} hidden={} layer_overrides={} groups={} named_views={}",
            session.selection.len(),
            session.hidden_entities.len(),
            session.layer_overrides.len(),
            session.groups.len(),
            session.view_states.len()
        ),
    ));
    diagnostics
}

/// A layout's display name, or its id when the table row is missing.
fn layout_label(drawing: &cad_db::DrawingDatabase, layout: LayoutId) -> String {
    drawing
        .layout(layout)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| format!("layout#{}", layout.0))
}

/// A block definition's name, or its id when the table row is missing.
fn block_label(drawing: &cad_db::DrawingDatabase, block: BlockId) -> String {
    drawing
        .block_name(block)
        .map(str::to_string)
        .unwrap_or_else(|| format!("block#{}", block.0))
}
