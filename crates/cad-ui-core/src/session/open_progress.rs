//! Open-pipeline progress and the terminal open summary (ADR-0030 D9).
//!
//! One open pipeline replaces the per-stage ad-hoc affordances: `import` (the
//! connector's entity pass, the only stage with a real `done`/`total`),
//! `vendor` (the host's vendor-record decode/apply pass) and `representations`
//! (the derived-snapshot rebuild). The host publishes the live stage; a carrier
//! overlays it onto [`super::CadStatus::open_progress`].
//!
//! Every value is a real observation. A stage without a per-item tick keeps
//! `total: null` and is rendered indeterminate; the previous stage's counts are
//! **never** carried into a new unit, and nothing is extrapolated to 100%.
//!
//! Vocabulary is string-based on purpose: `stage`/`unit` are stable machine keys
//! ([`CadOpenStage`]) so a new stage is additive on the wire instead of a
//! breaking enum change, and the UI maps them to catalog labels.

use serde::Serialize;

/// Bound on the diagnostics retained in a terminal [`CadOpenSummary`].
///
/// A vendor pass can emit one diagnostic per rejected record; the summary is a
/// status projection, not a log, so it keeps the first few and the wire stays
/// bounded.
pub const OPEN_SUMMARY_DIAGNOSTIC_CAP: usize = 32;

/// Stable machine keys of the open pipeline (`openProgress.stage`/`unit`).
///
/// A namespace rather than an enum: the wire carries plain strings, so a future
/// stage or counting unit is an additive value and an older consumer treats it
/// as unknown instead of failing to deserialize.
pub struct CadOpenStage;

impl CadOpenStage {
    /// Connector import: entity normalisation/insertion (the only stage with a
    /// real entity `done`/`total`).
    pub const IMPORT: &'static str = "import";
    /// Vendor-record decode/apply pass for the newly installed document.
    pub const VENDOR: &'static str = "vendor";
    /// Derived-snapshot rebuild after the import/vendor passes.
    pub const REPRESENTATIONS: &'static str = "representations";

    /// Counting unit of the import stage.
    pub const UNIT_ENTITIES: &'static str = "entities";
    /// Counting unit of the vendor stage.
    pub const UNIT_VENDOR_RECORDS: &'static str = "vendorRecords";

    /// Every stage this build knows, in pipeline order.
    pub const STAGES: [&'static str; 3] = [Self::IMPORT, Self::VENDOR, Self::REPRESENTATIONS];
    /// Every counting unit this build knows.
    pub const UNITS: [&'static str; 2] = [Self::UNIT_ENTITIES, Self::UNIT_VENDOR_RECORDS];

    /// Whether `stage` is a stage this build defines.
    pub fn is_known_stage(stage: &str) -> bool {
        Self::STAGES.contains(&stage)
    }

    /// Whether `unit` is a counting unit this build defines.
    pub fn is_known_unit(unit: &str) -> bool {
        Self::UNITS.contains(&unit)
    }
}

/// Stable wire values of `openSummary.vendor.status` (ADR-0030 D9).
///
/// String-based for the same additive reason as [`CadOpenStage`]. Every value is
/// a **terminal** state: an unsupported/failed/timed-out vendor pass is still a
/// finished open (`documentReady` becomes true), never a permanently pending one.
pub struct CadVendorOpenStatus;

impl CadVendorOpenStatus {
    /// The document has no vendor records, so no vendor pass ran.
    pub const NONE: &'static str = "none";
    /// The vendor pass decoded/applied successfully.
    pub const OK: &'static str = "ok";
    /// This carrier/build cannot decode vendor records (explicit skip).
    pub const UNSUPPORTED: &'static str = "unsupported";
    /// The vendor pass failed; `diagnostics` carries the stable reasons.
    pub const FAILED: &'static str = "failed";
    /// The host watchdog expired the vendor pass; never an infinite pending.
    pub const TIMEOUT: &'static str = "timeout";

    /// Every terminal state this build defines.
    pub const ALL: [&'static str; 5] = [
        Self::NONE,
        Self::OK,
        Self::UNSUPPORTED,
        Self::FAILED,
        Self::TIMEOUT,
    ];

    /// Whether `status` is a terminal vendor state this build defines.
    pub fn is_terminal(status: &str) -> bool {
        Self::ALL.contains(&status)
    }
}

/// Live open-pipeline progress for the frontend progress affordance.
///
/// Present only while the open pipeline is genuinely running: once it completes
/// the status reports `openProgress: null`, `documentReady: true` and keeps the
/// [`CadOpenSummary`]. `phase` is a stable machine key (`records`, `decode`,
/// `apply`, `rebuild`, or the import phase); `total` stays `null` for a stage
/// that exposes no per-item count, which the UI renders as an indeterminate bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadOpenProgress {
    /// One of [`CadOpenStage`]'s stage keys.
    pub stage: String,
    /// Stable phase key within the stage, or `null` before the first tick.
    pub phase: Option<String>,
    /// Real items processed so far in this stage's `unit` (`0` before a tick).
    pub done: u64,
    /// Real items expected, or `null` when the stage does not expose a count.
    pub total: Option<u64>,
    /// One of [`CadOpenStage`]'s counting-unit keys.
    pub unit: String,
    /// Whether a cancel request can still be issued for this pipeline.
    pub cancellable: bool,
}

impl CadOpenProgress {
    /// Project the live import block into the open-pipeline shape.
    ///
    /// This is the one place the import stage's `phase`/counts are translated;
    /// the carrier only overlays the later `vendor`/`representations` stages.
    pub fn from_import(import: &super::CadImportProgress) -> Self {
        Self {
            stage: CadOpenStage::IMPORT.to_string(),
            phase: import.phase.clone(),
            done: import.entities_done,
            total: import.entities_total,
            unit: CadOpenStage::UNIT_ENTITIES.to_string(),
            cancellable: import.cancellable,
        }
    }
}

/// The vendor block of a terminal [`CadOpenSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadVendorSummary {
    /// A [`CadVendorOpenStatus`] value string.
    pub status: String,
    /// Primitives the host actually wrote (never an invented count).
    pub applied: u32,
    /// Bounded stable diagnostics (see [`OPEN_SUMMARY_DIAGNOSTIC_CAP`]).
    pub diagnostics: Vec<String>,
}

impl CadVendorSummary {
    /// A terminal vendor summary whose diagnostics are bounded.
    pub fn new(status: &str, applied: u32, diagnostics: Vec<String>) -> Self {
        Self {
            status: status.to_string(),
            applied,
            diagnostics: bounded(diagnostics),
        }
    }

    /// A summary for a document that ran no vendor pass.
    pub fn none() -> Self {
        Self::new(CadVendorOpenStatus::NONE, 0, Vec::new())
    }
}

/// Terminal summary of the last successful open (ADR-0030 D9).
///
/// Retained after the pipeline finishes so a late-connecting consumer reads the
/// outcome from `cad_status.openSummary` instead of relying on event replay (the
/// event ring is bounded). `vendor.status` is terminal even when the vendor pass
/// was unsupported, failed or timed out: the document is still open and ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadOpenSummary {
    /// The label installed for the document.
    pub label: String,
    /// Real entity count of the installed document.
    pub entities: u64,
    /// The vendor pass outcome that finished this open.
    pub vendor: CadVendorSummary,
}

impl CadOpenSummary {
    /// A summary whose vendor diagnostics are bounded.
    pub fn new(label: String, entities: u64, vendor: CadVendorSummary) -> Self {
        Self {
            label,
            entities,
            vendor,
        }
    }
}

/// Keep at most [`OPEN_SUMMARY_DIAGNOSTIC_CAP`] diagnostics.
fn bounded(diagnostics: Vec<String>) -> Vec<String> {
    diagnostics
        .into_iter()
        .take(OPEN_SUMMARY_DIAGNOSTIC_CAP)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendor_summary_bounds_its_diagnostics() {
        let diagnostics = (0..OPEN_SUMMARY_DIAGNOSTIC_CAP + 5)
            .map(|index| format!("vendor.diagnostic.{index}"))
            .collect();
        let summary = CadVendorSummary::new(CadVendorOpenStatus::FAILED, 2, diagnostics);
        assert_eq!(summary.diagnostics.len(), OPEN_SUMMARY_DIAGNOSTIC_CAP);
        assert_eq!(summary.status, "failed");
    }

    #[test]
    fn the_stage_and_vendor_vocabulary_is_known_but_open_ended() {
        assert!(CadOpenStage::is_known_stage("vendor"));
        assert!(
            !CadOpenStage::is_known_stage("future-stage"),
            "an unknown stage is not claimed to be known"
        );
        assert!(CadOpenStage::is_known_unit("vendorRecords"));
        for status in CadVendorOpenStatus::ALL {
            assert!(CadVendorOpenStatus::is_terminal(status));
        }
        assert!(!CadVendorOpenStatus::is_terminal("pending"));
    }
}
