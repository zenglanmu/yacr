//! Products and outcomes of a solid modelling operation.
//!
//! There is deliberately no "success with no body" variant and no `aborted`
//! flag: an operation that produces nothing usable is
//! [`SolidOutcome::Unsupported`] or [`SolidOutcome::Failed`], and a cancelled
//! call is [`cad_domain::CadError::Cancelled`], not an outcome (ADR-0036 D1).

use cad_domain::{BrepData, Diagnostic, Point3, TaskStamp};

use super::SolidOperationFamily;

/// Why a modeler refused an operation, kept structured so callers act on the
/// category instead of parsing prose.
///
/// This is intentionally **not** [`crate::UnsupportedReason`]: that type
/// classifies an import *exchange payload* (`exchange: ExchangeKind`, `len`,
/// `is_empty`), which has no meaning for a typed modelling command. The solid
/// seam keeps the same honesty contract with a stable `code`, the refused
/// operation family and a human-facing detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolidUnsupportedReason {
    /// Stable [`crate::codes`] value (for example `solid.unsupported`).
    pub code: &'static str,
    /// Which operation family was refused.
    pub operation: SolidOperationFamily,
    /// Human-facing detail (localised by the UI, never parsed).
    pub detail: String,
}

/// Accuracy of a mass-properties result.
///
/// Only the **requested** relative accuracy is available: the kernel estimates
/// mass properties numerically, so there is no proved bound to report. Callers
/// must not promote this to [`cad_domain::Precision::Analytic`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MassAccuracy {
    /// The estimate was requested at this relative accuracy (`1e-6` = 0.0001 %).
    Requested { relative: f64 },
}

/// Mass properties of a closed body.
///
/// Lengths are model units (drawing units), so `volume` is units³, `surface_area`
/// units², `centroid` in model coordinates and `density` in mass per unit³. The
/// seam performs no unit conversion; `cad-db`/`cad-app` own the unit context.
/// `inertia` is the 3×3 inertia tensor about the centroid, when the kernel could
/// provide it (mass · units²).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolidMassProperties {
    pub volume: f64,
    pub surface_area: f64,
    pub centroid: Point3,
    pub inertia: Option<[[f64; 3]; 3]>,
    pub density: f64,
    pub accuracy: MassAccuracy,
}

/// What a successful operation produced.
#[derive(Debug, Clone, PartialEq)]
pub enum SolidProduct {
    /// A new exact body plus any non-fatal diagnostics.
    Body {
        brep: BrepData,
        diagnostics: Vec<Diagnostic>,
    },
    /// Mass properties of the requested body.
    Mass(SolidMassProperties),
}

impl SolidProduct {
    /// The produced exact body, if this product is a body.
    pub fn body(&self) -> Option<&BrepData> {
        match self {
            SolidProduct::Body { brep, .. } => Some(brep),
            SolidProduct::Mass(_) => None,
        }
    }

    /// The mass properties, if this product is a mass-properties result.
    pub fn mass(&self) -> Option<&SolidMassProperties> {
        match self {
            SolidProduct::Mass(mass) => Some(mass),
            SolidProduct::Body { .. } => None,
        }
    }

    /// Non-fatal diagnostics attached to a produced body.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            SolidProduct::Body { diagnostics, .. } => diagnostics,
            SolidProduct::Mass(_) => &[],
        }
    }
}

/// The outcome of a solid operation.
///
/// ```text
/// Ok(Success(Body))       kernel produced an exact body
/// Ok(Success(Mass))       kernel produced mass properties
/// Ok(Unsupported)         declared capability or operand shape is missing
/// Ok(Failed)              the kernel ran but produced no usable product
/// Err(InvalidInput)       request did not validate (see SolidRequest::validate)
/// Err(Cancelled)          cancelled before or after the kernel call
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum SolidOutcome {
    /// An exact product.
    Success(SolidProduct),
    /// The seam cannot evaluate this operation in this build. Never a product.
    Unsupported { reason: SolidUnsupportedReason },
    /// The operation was evaluable in principle but produced no usable product
    /// (for example an empty boolean result). Never an empty success.
    Failed { diagnostics: Vec<Diagnostic> },
}

impl SolidOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, SolidOutcome::Success(_))
    }

    /// The produced exact body, if any. `None` for unsupported/failed outcomes,
    /// so callers cannot mistake "no kernel" for "empty body".
    pub fn body(&self) -> Option<&BrepData> {
        self.product()?.body()
    }

    /// The produced mass properties, if any.
    pub fn mass(&self) -> Option<&SolidMassProperties> {
        self.product()?.mass()
    }

    /// The product, if the operation succeeded.
    pub fn product(&self) -> Option<&SolidProduct> {
        match self {
            SolidOutcome::Success(product) => Some(product),
            SolidOutcome::Unsupported { .. } | SolidOutcome::Failed { .. } => None,
        }
    }

    /// The structured refusal, if the outcome is `Unsupported`.
    pub fn unsupported(&self) -> Option<&SolidUnsupportedReason> {
        match self {
            SolidOutcome::Unsupported { reason } => Some(reason),
            _ => None,
        }
    }

    /// Every diagnostic carried by this outcome (empty for `Unsupported`, whose
    /// reason is structured instead).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            SolidOutcome::Success(product) => product.diagnostics(),
            SolidOutcome::Failed { diagnostics } => diagnostics,
            SolidOutcome::Unsupported { .. } => &[],
        }
    }
}

/// Result of a [`SolidModeler::execute`](crate::SolidModeler::execute) call.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidResult {
    pub stamp: TaskStamp,
    pub outcome: SolidOutcome,
}
