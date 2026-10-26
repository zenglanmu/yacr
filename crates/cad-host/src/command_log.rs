//! Bounded, typed command echo (AutoCAD-style command-window history).
//!
//! The log is **observational only**: nothing here feeds an admission, queue,
//! execution or render path. It carries no human-readable text at all — only
//! stable machine keys (`algorithm`, diagnostic `code`, unit symbol, precision
//! token) and the typed measurement numbers — so localized strings can never
//! leak into the payload. The UI layer maps every key through its catalogs.
//!
//! The log is bounded by [`COMMAND_LOG_CAPACITY`] with the **oldest entry
//! dropped** first. Sequences are strictly increasing for the lifetime of the
//! log and are **not** reset when the log is cleared, so two entries can never
//! share a sequence even across a document replacement.

use std::collections::VecDeque;

use cad_db::{MeasurementAlgorithm, MeasurementRecord};
use cad_domain::{CadError, Precision, UnitContext};

/// Maximum number of retained entries; pushing past it drops the oldest.
pub const COMMAND_LOG_CAPACITY: usize = 200;

/// Which kind of echo an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandLogKind {
    /// A named selector was admitted by the router (`DIST`, `LINE`, `OPEN`, …).
    Command,
    /// A command produced a typed measurement result.
    Result,
    /// A command was refused; the entry carries a stable diagnostic code.
    Error,
}

/// Typed measurement echo a `Result` entry carries.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandLogMeasurement {
    /// Stable machine algorithm key: `distance`, `polyline`, `angle`, `area`,
    /// `area_object`.
    pub algorithm: String,
    /// The measured value in the record's own unit context.
    pub value: f64,
    /// A known display unit symbol (`mm`, `m`, `in`, `ft`), or `None` for
    /// drawing units — an unknown unit is never fabricated.
    pub unit: Option<String>,
    /// The modelled precision token (`analytic`, `approximate`, `unknown`).
    pub precision: Option<String>,
    /// The picked points, in capture order.
    pub points: Vec<[f64; 3]>,
}

/// Typed point-coordinate echo (`ID`) a `Result` entry carries.
///
/// Every field is a stable machine token or a real coordinate: `space` is
/// `model`/`paper`, `snap` is `SnapKind::as_str()` (`endpoint`, `midpoint`,
/// `center`, `quadrant`, `perpendicular`, `intersection`), `precision` is the
/// modelled precision token and `unit` is a known display symbol. No text, no
/// label: the UI localizes from these keys.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandLogPoint {
    /// The reported world coordinate (the snapped point when a snap resolved).
    pub position: [f64; 3],
    /// The observation space the pick was made in: `model` or `paper`.
    pub space: &'static str,
    /// The object snap that produced the point, or `None` for a bare pick.
    pub snap: Option<&'static str>,
    /// The modelled precision token (`analytic`, `approximate`, `unknown`).
    pub precision: Option<&'static str>,
    /// A known display unit symbol, or `None` for drawing units.
    pub unit: Option<String>,
    /// Perpendicular distance from the raw pick, in logical pixels.
    pub pixel_distance: Option<f64>,
}

/// One command-window echo entry.
///
/// Deliberately has **no** message/text field: a `Diagnostic::message` (partly
/// Chinese today) can never be copied here, and the UI localizes only from the
/// stable keys.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandLogEntry {
    /// Monotonic sequence, unique for the lifetime of the log.
    pub sequence: u64,
    pub kind: CommandLogKind,
    /// Stable selector name for a `Command` entry; `None` for results/errors.
    pub command: Option<String>,
    /// Stable diagnostic code for an `Error` entry; `None` otherwise.
    pub code: Option<String>,
    /// Typed measurement echo for a `Result` entry; `None` otherwise.
    ///
    /// Strictly measurement-only: an `ID` result carries [`Self::point`] instead.
    pub measurement: Option<CommandLogMeasurement>,
    /// Typed point-coordinate echo for an `ID` result; `None` otherwise.
    pub point: Option<CommandLogPoint>,
}

/// The bounded ring plus its monotonic sequence counter.
#[derive(Debug)]
pub struct CommandLog {
    next_sequence: u64,
    entries: VecDeque<CommandLogEntry>,
}

impl Default for CommandLog {
    fn default() -> Self {
        Self {
            next_sequence: 1,
            entries: VecDeque::new(),
        }
    }
}

impl CommandLog {
    /// Record an admitted selector. The entry is recorded at admission, so a
    /// later refusal of the same selector is a following `Error` entry.
    pub fn push_command(&mut self, name: &str) {
        self.push(CommandLogEntry {
            sequence: 0,
            kind: CommandLogKind::Command,
            command: Some(name.to_string()),
            code: None,
            measurement: None,
            point: None,
        });
    }

    /// Record a typed measurement result.
    pub fn push_result(&mut self, record: &MeasurementRecord) {
        self.push(CommandLogEntry {
            sequence: 0,
            kind: CommandLogKind::Result,
            command: None,
            code: None,
            measurement: Some(measurement_echo(record)),
            point: None,
        });
    }

    /// Record a typed point-coordinate result (`ID`).
    ///
    /// Same `Result` kind as a measurement: it is a real, typed answer of a
    /// read-only query, not a command admission or a refusal. The two payloads are
    /// mutually exclusive by construction (`measurement` stays measurement-only).
    pub fn push_point_result(&mut self, report: &cad_app::PointReport) {
        self.push(CommandLogEntry {
            sequence: 0,
            kind: CommandLogKind::Result,
            command: None,
            code: None,
            measurement: None,
            point: Some(CommandLogPoint {
                position: [report.position.x, report.position.y, report.position.z],
                space: report.space.as_str(),
                snap: report.snap.map(|kind| kind.as_str()),
                precision: Some(precision_key(&report.precision)),
                unit: report.unit.clone(),
                pixel_distance: report.logical_pixel_distance,
            }),
        });
    }

    /// Record a refusal by its stable diagnostic code (never its message).
    pub fn push_error(&mut self, code: &str) {
        self.push(CommandLogEntry {
            sequence: 0,
            kind: CommandLogKind::Error,
            command: None,
            code: Some(code.to_string()),
            measurement: None,
            point: None,
        });
    }

    /// Every retained entry, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &CommandLogEntry> {
        self.entries.iter()
    }

    /// Drop every entry but keep the sequence counter monotonic.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn push(&mut self, mut entry: CommandLogEntry) {
        entry.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        if self.entries.len() >= COMMAND_LOG_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

/// The stable algorithm key of a measured record.
pub fn algorithm_key(algorithm: &MeasurementAlgorithm) -> &'static str {
    match algorithm {
        MeasurementAlgorithm::Distance2d | MeasurementAlgorithm::Distance3d => "distance",
        MeasurementAlgorithm::PolylineLength => "polyline",
        MeasurementAlgorithm::Angle3Points => "angle",
        MeasurementAlgorithm::PlanarPolygonArea => "area",
        // `AREA OBJECT`: its own key, because the value came from one entity's
        // stored geometry rather than from a picked polygon ring. A consumer
        // that wants the two areas grouped can group them, but the log never
        // claims a circle's area was a polygon.
        MeasurementAlgorithm::ObjectArea => "area_object",
    }
}

/// The stable code of a refusal.
///
/// These are machine tokens, not prose: the `CadError` payload (which may carry
/// localized text) is deliberately discarded.
pub fn error_code(error: &CadError) -> &'static str {
    match error {
        CadError::NotImplemented(_) => "not_implemented",
        CadError::InvalidInput(_) => "invalid_input",
        CadError::Unsupported(_) => "unsupported",
        CadError::ResourceMissing(_) => "resource_missing",
        CadError::CorruptData(_) => "corrupt_data",
        CadError::GpuFailure(_) => "gpu_failure",
        CadError::Invariant(_) => "invariant",
        CadError::PermissionDenied => "permission_denied",
        CadError::Cancelled => "cancelled",
        CadError::StaleResult => "stale_result",
    }
}

fn measurement_echo(record: &MeasurementRecord) -> CommandLogMeasurement {
    CommandLogMeasurement {
        algorithm: algorithm_key(&record.algorithm).to_string(),
        value: record.value,
        unit: unit_symbol(&record.units),
        precision: precision_token(&record.precision),
        points: record
            .inputs
            .iter()
            .map(|point| [point.x, point.y, point.z])
            .collect(),
    }
}

/// A known display unit symbol, or `None` for drawing units.
///
/// Delegates to [`cad_app::unit_symbol`] — the one projection `cad-app`'s point
/// report also uses — so the two wire echoes cannot drift apart.
/// `DrawingUnits` is unknown by definition, so it maps to `None` instead of
/// inventing millimetres.
fn unit_symbol(units: &UnitContext) -> Option<String> {
    cad_app::unit_symbol(units).map(str::to_string)
}

/// The modelled precision state as a stable token (no new label is invented).
///
/// The one authority for the three tokens: the measurement echo (owned `String`)
/// and the point echo (`&'static str`) both read it.
fn precision_key(precision: &Precision) -> &'static str {
    match precision {
        Precision::Analytic => "analytic",
        Precision::Approximate { .. } => "approximate",
        Precision::Unknown => "unknown",
    }
}

fn precision_token(precision: &Precision) -> Option<String> {
    Some(precision_key(precision).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_newest_entries_and_monotonic_sequences() {
        let mut log = CommandLog::default();
        for index in 0..(COMMAND_LOG_CAPACITY + 5) {
            log.push_command(&format!("CMD{index}"));
        }
        let entries: Vec<&CommandLogEntry> = log.entries().collect();
        assert_eq!(entries.len(), COMMAND_LOG_CAPACITY);
        assert_eq!(entries.first().unwrap().sequence, 6);
        assert_eq!(
            entries.last().unwrap().sequence,
            (COMMAND_LOG_CAPACITY + 5) as u64
        );
        assert_eq!(entries.first().unwrap().command.as_deref(), Some("CMD5"));
        for pair in entries.windows(2) {
            assert!(
                pair[0].sequence < pair[1].sequence,
                "sequences must stay strictly increasing"
            );
        }
    }

    #[test]
    fn clear_drops_entries_but_keeps_sequences_monotonic() {
        let mut log = CommandLog::default();
        log.push_command("LINE");
        log.push_error("unsupported");
        log.clear();
        assert_eq!(log.entries().count(), 0);
        log.push_command("CIRCLE");
        assert_eq!(log.entries().next().unwrap().sequence, 3);
    }

    #[test]
    fn an_error_entry_carries_only_a_code() {
        let mut log = CommandLog::default();
        log.push_error(error_code(&CadError::Unsupported("localized".into())));
        let entry = log.entries().next().unwrap();
        assert_eq!(entry.kind, CommandLogKind::Error);
        assert_eq!(entry.code.as_deref(), Some("unsupported"));
        assert!(entry.command.is_none());
        assert!(entry.measurement.is_none());
    }
}
