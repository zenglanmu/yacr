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
            ExtendedCommandId::BlockCount => self.block_count(session, command),
            ExtendedCommandId::CountObjects => self.count_objects(session, command),
            ExtendedCommandId::EditTime => self.edit_time(command),
            ExtendedCommandId::JulianDate => self.julian_date(command),
            ExtendedCommandId::XList => self.xlist(command),
            other => Err(CadError::Invariant(format!(
                "not an extended report: {other:?}"
            ))),
        }
    }

    /// `BCOUNT`: count placed INSERT instances grouped by block name.
    ///
    /// Two documented scopes are honoured without an interactive prompt:
    ///
    /// * a non-empty selection counts only the INSERTs among the selected
    ///   top-level references (the `Select objects` prompt's result);
    /// * an empty selection counts the **entire drawing** — model space and
    ///   every paper-space layout.
    ///
    /// Nested references inside block definitions are deliberately **not**
    /// expanded, matching AutoCAD's own documented limit ("BCOUNT does not
    /// select or count nested blocks"); they are reported by name instead of
    /// silently disappearing into the total.
    fn block_count(&self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "BCOUNT")?;
        let drawing = document_drawing(self, command)?;
        let using_selection = !session.selection.is_empty();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut nested: BTreeMap<String, usize> = BTreeMap::new();
        let mut unresolved = 0usize;
        let mut non_block = 0usize;
        if using_selection {
            for reference in session.selection.refs() {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    unresolved += 1;
                    continue;
                }
                match drawing.entity(reference.entity) {
                    Some(entity) => match &entity.geometry {
                        SemanticGeometry::Insert { block, .. } => {
                            *counts.entry(block_label(&drawing, *block)).or_default() += 1;
                        }
                        _ => non_block += 1,
                    },
                    None => unresolved += 1,
                }
            }
        } else {
            for entity in drawing.entities() {
                let SemanticGeometry::Insert { block, .. } = &entity.geometry else {
                    continue;
                };
                match entity.space {
                    SpaceId::Model | SpaceId::Paper(_) => {
                        *counts.entry(block_label(&drawing, *block)).or_default() += 1;
                    }
                    // Block-definition members are definitions, not placed
                    // instances: named, never counted.
                    SpaceId::Block(_) => {
                        *nested.entry(block_label(&drawing, *block)).or_default() += 1;
                    }
                }
            }
        }
        let total: usize = counts.values().sum();
        let distinct = counts.len();
        let mut diagnostics = Vec::with_capacity(distinct + 4);
        for (name, instances) in &counts {
            diagnostics.push(diagnostic(
                "bcount.block",
                format!("BCOUNT: block '{name}' instances={instances}"),
            ));
        }
        let scope = if using_selection {
            "the current selection"
        } else {
            "the whole drawing (model space and paper space)"
        };
        diagnostics.push(diagnostic(
            "bcount.summary",
            format!(
                "BCOUNT: {total} block instance(s) across {distinct} distinct block(s) in {scope}"
            ),
        ));
        if !nested.is_empty() {
            let nested_total: usize = nested.values().sum();
            let rows = nested
                .iter()
                .map(|(name, instances)| format!("{name}={instances}"))
                .collect::<Vec<_>>()
                .join(" ");
            diagnostics.push(diagnostic(
                "bcount.nested_not_counted",
                format!(
                    "BCOUNT: {nested_total} block reference(s) inside block definitions ({rows}) \
                     are not counted; BCOUNT does not expand nested blocks"
                ),
            ));
        }
        if non_block > 0 {
            diagnostics.push(diagnostic(
                "bcount.non_block_ignored",
                format!("BCOUNT: {non_block} selected non-block object(s) were ignored"),
            ));
        }
        if unresolved > 0 {
            diagnostics.push(diagnostic(
                "bcount.unresolved",
                format!(
                    "BCOUNT: {unresolved} selected reference(s) are block-instance members, \
                     sub-elements or absent entities and were not counted"
                ),
            ));
        }
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

    /// `JULIAN`: a Julian day number, for the current UTC instant or one typed
    /// calendar date.
    ///
    /// The date form accepts `YYYY-MM-DD` or `YYYY/MM/DD` with an optional
    /// `[T ]HH:MM[:SS]` (UTC). An impossible calendar date, an out-of-range time
    /// or unparsable text is a named `julian.invalid_date` refusal — nothing is
    /// clamped or invented.
    fn julian_date(&self, command: &Command) -> CadResult<CommandOutcome> {
        // A read-only report still validates the document exists.
        let _drawing = document_drawing(self, command)?;
        match &command.payload {
            CommandPayload::None => match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(elapsed) => {
                    let seconds = elapsed.as_secs() as f64
                        + f64::from(elapsed.subsec_nanos()) / 1_000_000_000.0;
                    // Julian Day Number at the Unix epoch is 2440587.5.
                    let julian = 2_440_587.5 + seconds / 86_400.0;
                    let day = julian.floor();
                    let fraction = julian - day;
                    Ok(report_outcome(vec![diagnostic(
                        "julian.value",
                        format!(
                            "JULIAN: {julian:.6} (integer day {}, fraction {fraction:.6}) for the \
                             current UTC instant",
                            day as i64
                        ),
                    )]))
                }
                Err(_) => Ok(report_outcome(vec![diagnostic(
                    "julian.unavailable",
                    "JULIAN: the system clock is before the Unix epoch; no Julian day was \
                     fabricated"
                        .into(),
                )])),
            },
            CommandPayload::JulianDate(text) => {
                let (julian, normalized) = parse_julian_date(text)?;
                let day = julian.floor();
                let fraction = julian - day;
                Ok(report_outcome(vec![diagnostic(
                    "julian.value",
                    format!(
                        "JULIAN: {normalized} = {julian:.6} (integer day {}, fraction \
                         {fraction:.6})",
                        day as i64
                    ),
                )]))
            }
            _ => Err(CadError::InvalidInput(
                "julian.invalid_payload: JULIAN takes one calendar date (`JULIAN YYYY-MM-DD`) or \
                 no argument"
                    .into(),
            )),
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
            drawing.named_views().len()
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

/// One parsed UTC calendar date/time for `JULIAN <date>`.
struct JulianCalendar {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

/// Parse `JULIAN <date>` and return `(julian_day, normalized_text)`.
///
/// Accepted grammar (UTC, proleptic Gregorian):
///
/// * `YYYY-MM-DD` / `YYYY/MM/DD` — midnight;
/// * the same date followed by `T` or one space and `HH:MM` or `HH:MM:SS`.
///
/// Everything else (including an impossible day such as `2026-02-30`, an hour
/// outside `0..=23`, or an unpadded decimal) is the stable
/// `julian.invalid_date` refusal. The date is never clamped to a nearby real
/// day.
fn parse_julian_date(text: &str) -> CadResult<(f64, String)> {
    let trimmed = text.trim();
    let invalid = || {
        CadError::InvalidInput(format!(
            "julian.invalid_date: '{trimmed}' is not a calendar date; expected `YYYY-MM-DD` or \
             `YYYY/MM/DD` with an optional `[T ]HH:MM[:SS]` (UTC)"
        ))
    };
    if trimmed.is_empty() {
        return Err(invalid());
    }

    // Split the date from an optional time. `T` is the ISO separator; one run of
    // whitespace is the command-line spelling.
    let (date_text, time_text) = if let Some((date, time)) = trimmed.split_once('T') {
        (date.trim(), Some(time.trim()))
    } else if let Some(index) = trimmed.find(char::is_whitespace) {
        (
            trimmed[..index].trim(),
            Some(trimmed[index..].trim()).filter(|time| !time.is_empty()),
        )
    } else {
        (trimmed, None)
    };

    let (year_text, month_text, day_text) = if let Some((year, rest)) = date_text.split_once('-') {
        if rest.contains('-') {
            let (month, day) = rest.split_once('-').ok_or_else(invalid)?;
            (year, month, day)
        } else {
            return Err(invalid());
        }
    } else if let Some((year, rest)) = date_text.split_once('/') {
        let (month, day) = rest.split_once('/').ok_or_else(invalid)?;
        (year, month, day)
    } else {
        // Compact `YYYYMMDD` (the calendar spelling used by `CDATE` without its
        // time part); an 8-digit argument is never ambiguous with a Julian day
        // in this command's grammar.
        if date_text.len() == 8 && date_text.bytes().all(|byte| byte.is_ascii_digit()) {
            (&date_text[..4], &date_text[4..6], &date_text[6..])
        } else {
            return Err(invalid());
        }
    };
    let year = parse_calendar_number(year_text, 4, invalid)? as i64;
    let month = parse_calendar_number(month_text, 2, invalid)?;
    let day = parse_calendar_number(day_text, 2, invalid)?;

    let (hour, minute, second) = match time_text {
        None => (0, 0, 0),
        Some(time) => {
            let mut parts = time.split(':');
            let hour = parts.next().ok_or_else(invalid)?;
            let minute = parts.next().ok_or_else(invalid)?;
            let second = parts.next().unwrap_or("00");
            if parts.next().is_some() {
                return Err(invalid());
            }
            (
                parse_calendar_number(hour, 2, invalid)?,
                parse_calendar_number(minute, 2, invalid)?,
                parse_calendar_number(second, 2, invalid)?,
            )
        }
    };

    let calendar = JulianCalendar {
        year,
        month,
        day,
        hour,
        minute,
        second,
    };
    if !calendar.is_real() {
        return Err(invalid());
    }
    let julian = julian_day_number(&calendar);
    let normalized = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        calendar.year,
        calendar.month,
        calendar.day,
        calendar.hour,
        calendar.minute,
        calendar.second
    );
    Ok((julian, normalized))
}

/// Parse one bounded decimal field (digits only, exactly `width` characters).
fn parse_calendar_number(
    text: &str,
    width: usize,
    invalid: impl Fn() -> CadError + Copy,
) -> CadResult<u32> {
    if text.len() != width || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    text.parse().map_err(|_| invalid())
}

impl JulianCalendar {
    /// Whether this is a real proleptic-Gregorian date/time.
    fn is_real(&self) -> bool {
        if !matches!(self.year, 1..=9999)
            || !matches!(self.month, 1..=12)
            || self.hour > 23
            || self.minute > 59
            || self.second > 59
        {
            return false;
        }
        let leap = self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0);
        let days = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return false,
        };
        self.day >= 1 && self.day <= days
    }
}

/// The Julian day number/fraction for one UTC calendar instant (proleptic
/// Gregorian; Howard Hinnant's civil-to-days formula).
fn julian_day_number(calendar: &JulianCalendar) -> f64 {
    let a = (14 - calendar.month as i64) / 12;
    let y = calendar.year + 4800 - a;
    let m = calendar.month as i64 + 12 * a - 3;
    let day = calendar.day as i64 + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045;
    day as f64
        + (calendar.hour as f64 - 12.0) / 24.0
        + calendar.minute as f64 / 1440.0
        + calendar.second as f64 / 86_400.0
}
