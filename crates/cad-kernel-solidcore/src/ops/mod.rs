//! The typed operations, each a thin sequence of upstream calls.
//!
//! Every kernel-facing operation passes the same gate: neutral body →
//! [`lift`] (record) → `Body::from_record(.., false)` → the upstream call →
//! `Body::to_record` → [`unlift`] (neutral body). A refusal at the gate is an
//! `Unsupported` outcome with a stable code, never a made-up body.

pub(crate) mod boolean;
pub(crate) mod mass;
pub(crate) mod primitive;
pub(crate) mod transform;

use cad_domain::{BrepData, Diagnostic};
use cad_kernel_adapter::{SolidOutcome, SolidProduct};
use solidcore_record::brep::Brep as Record;

use crate::error::LiftRefusal;
use crate::record;

/// A neutral body as the seam's success product (no non-fatal diagnostics in P1).
pub(crate) fn body_outcome(brep: BrepData) -> SolidOutcome {
    SolidOutcome::Success(SolidProduct::Body {
        brep,
        diagnostics: Vec::<Diagnostic>::new(),
    })
}

/// Neutral body → kernel record: the single lift gate of this crate.
pub(crate) fn lift(brep: &BrepData) -> Result<Record, LiftRefusal> {
    record::to_record(brep, record::DEFAULT_TOLERANCE)
}

/// Kernel record → neutral body.
pub(crate) fn unlift(record: &Record) -> Result<BrepData, LiftRefusal> {
    record::from_record(record)
}
