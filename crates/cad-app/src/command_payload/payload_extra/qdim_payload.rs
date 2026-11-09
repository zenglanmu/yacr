//! The typed `QDIM` payload data, split out of `payload_extra.rs` to keep every
//! code file inside the 1000-line gate.

use cad_domain::Point3;

/// The object-snap priority QDIM uses for extension-line origins (the
/// documented `Settings` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QDimSnap {
    /// Use the selected line/dimension endpoints (the default).
    #[default]
    Endpoints,
    /// Use the pairwise intersections of the selected LINEs.
    Intersections,
}

/// A documented QDIM generation mode. Angular is deliberately absent: AutoCAD
/// 2026 does not offer an angular QDIM mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QDimMode {
    /// End-to-end linear dimensions on one dimension line (the default).
    #[default]
    Continuous,
    /// Linear dimensions offset by a constant increment (`DIMDLI` by default).
    Staggered,
    /// Linear dimensions sharing one extension line, stacked by `DIMDLI`.
    Baseline,
    /// Ordinate dimensions from one datum point.
    Ordinate,
    /// Radius dimensions for selected circles and arcs.
    Radius,
    /// Diameter dimensions for selected circles.
    Diameter,
}

/// The typed options of one `QDIM` line.
///
/// The nine documented QDIM modes map onto this payload as: the six generation
/// modes in [`QDimMode`]; **Datum Point** as `datum` (the shared origin for
/// `Baseline`/`Ordinate`); **Edit** as `remove` (0-based origin indices dropped
/// before generation); and **Settings** as `snap` (the extension-line origin
/// priority). The selection is still the rest of the input.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QDimCommand {
    pub mode: QDimMode,
    pub snap: QDimSnap,
    /// The Datum Point option: the common origin for `Baseline` and the datum
    /// for `Ordinate`. `None` uses the first ordered origin for `Baseline` and
    /// the drawing origin `(0, 0, 0)` for `Ordinate` (AutoCAD's UCS origin).
    pub datum: Option<Point3>,
    /// The Edit option: 0-based indices into the ordered origin list (linear
    /// modes) or the target list (radial modes) removed before generation.
    pub remove: Vec<usize>,
    /// The Staggered increment. `None` uses the resolved `DIMDLI` (declared
    /// choice: AutoCAD 2026 does not document the increment source).
    pub increment: Option<f64>,
}
