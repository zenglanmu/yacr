//! Typed command-window echo wire shapes (`status.commandLog`).
//!
//! Split out of [`super::status`] to keep that module inside the code-size limit;
//! the types and the projection are unchanged and re-exported from the session
//! root, so every existing path still resolves.
//!
//! Every field is a stable machine token or a real number: there is deliberately
//! **no** message/text field, so a `Diagnostic::message` (partly Chinese today) can
//! never reach the wire.

use serde::Serialize;

/// Which kind of echo a [`CadCommandLogEntry`] is.
///
/// Serialized as a lowercase string (`"command"`, `"result"`, `"error"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CadCommandLogKind {
    /// A named selector was admitted by the router.
    Command,
    /// A command produced a typed measurement result.
    Result,
    /// A command was refused.
    Error,
}

/// Typed measurement echo carried by a `Result` entry.
///
/// Every field is a stable machine key or a real measured number: `algorithm`
/// is one of `distance`/`polyline`/`angle`/`area`, `unit` is a known display
/// symbol (`mm`/`m`/`in`/`ft`) or `null` for drawing units, and `precision` is
/// the modelled precision token. There is deliberately **no** text field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadMeasurementEcho {
    /// Stable machine algorithm key (never translated).
    pub algorithm: String,
    /// The measured value in the record's own unit context.
    pub value: f64,
    /// A known display unit symbol, or `null` for drawing units.
    pub unit: Option<String>,
    /// The modelled precision token (`analytic`/`approximate`/`unknown`).
    pub precision: Option<String>,
    /// The picked points, in capture order, so the UI can format deltas.
    pub points: Vec<[f64; 3]>,
}

/// Typed point-coordinate echo (`ID`) carried by a `Result` entry.
///
/// Every field is a stable machine token or a real coordinate: `space` is
/// `model`/`paper`, `snap` is `endpoint`/`midpoint`/`center`/`quadrant`/
/// `perpendicular`/`intersection` (or `null`), `precision` is the modelled
/// precision token, `unit` a known display symbol or `null` for drawing units, and
/// `pixelDistance` the logical-pixel distance from the raw pick to the snapped
/// point. Wire shape: `{ position, space, snap, precision, unit, pixelDistance }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadCommandLogPoint {
    /// The reported world coordinate (the snapped point when a snap resolved).
    pub position: [f64; 3],
    /// The observation space: `model` or `paper`.
    pub space: &'static str,
    /// The object snap that produced the point, or `null` for a bare pick.
    pub snap: Option<&'static str>,
    /// The modelled precision token, or `null` when unknown to this layer.
    pub precision: Option<&'static str>,
    /// A known display unit symbol, or `null` for drawing units.
    pub unit: Option<String>,
    /// Logical-pixel distance from the raw pick to the snapped point.
    pub pixel_distance: Option<f64>,
}

/// One entry of the command-window echo (AutoCAD-style command history).
///
/// The type has **no** message/text field by construction: the host only ever
/// supplies stable codes/keys, and `Diagnostic::message` (partly Chinese today)
/// can never reach the wire. `command` is present only on `Command` entries and
/// `measurement` only on `Result` entries; an `Error` carries only `code`.
/// `point` is present only on an `ID` result — `measurement` stays strictly
/// measurement-only, so the two payloads are mutually exclusive.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadCommandLogEntry {
    /// Monotonic sequence, unique for the host log's lifetime.
    pub sequence: u64,
    pub kind: CadCommandLogKind,
    /// Stable selector name (`DIST`, `LINE`, …) for a `Command` entry.
    pub command: Option<String>,
    /// Stable diagnostic code for an `Error` entry.
    pub code: Option<String>,
    /// Typed measurement echo for a `Result` entry.
    pub measurement: Option<CadMeasurementEcho>,
    /// Typed point-coordinate echo for an `ID` result.
    pub point: Option<CadCommandLogPoint>,
}

impl CadCommandLogEntry {
    /// Project one host log entry onto the wire shape.
    pub(super) fn from_host(entry: &cad_host::CommandLogEntry) -> Self {
        Self {
            sequence: entry.sequence,
            kind: match entry.kind {
                cad_host::CommandLogKind::Command => CadCommandLogKind::Command,
                cad_host::CommandLogKind::Result => CadCommandLogKind::Result,
                cad_host::CommandLogKind::Error => CadCommandLogKind::Error,
            },
            command: entry.command.clone(),
            code: entry.code.clone(),
            measurement: entry
                .measurement
                .as_ref()
                .map(|measurement| CadMeasurementEcho {
                    algorithm: measurement.algorithm.clone(),
                    value: measurement.value,
                    unit: measurement.unit.clone(),
                    precision: measurement.precision.clone(),
                    points: measurement.points.clone(),
                }),
            point: entry.point.as_ref().map(|point| CadCommandLogPoint {
                position: point.position,
                space: point.space,
                snap: point.snap,
                precision: point.precision,
                unit: point.unit.clone(),
                pixel_distance: point.pixel_distance,
            }),
        }
    }
}
