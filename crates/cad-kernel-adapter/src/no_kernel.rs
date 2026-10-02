//! The honest default tessellator: no ACIS kernel is linked, so every non-empty
//! payload is reported as unsupported with a stable code instead of a fake mesh.

use super::*;

/// The honest default: no ACIS kernel is linked, so every non-empty payload is
/// reported as unsupported with a stable code instead of a fake mesh.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoKernelTessellator;

/// Backwards-compatible name for the default (previously `PendingTessellator`).
pub type PendingTessellator = NoKernelTessellator;

impl NoKernelTessellator {
    fn failed(stamp: &TaskStamp, code: &str, message: String) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    code: code.to_string(),
                    object: None,
                    message,
                }],
            },
        }
    }

    fn unsupported(
        stamp: &TaskStamp,
        exchange: ExchangeKind,
        code: &str,
        detail: String,
    ) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Unsupported {
                reason: UnsupportedReason {
                    code: code.to_string(),
                    exchange,
                    detail,
                },
            },
        }
    }
}

impl SolidTessellator for NoKernelTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.kernel.unsupported".into(),
            version: 1,
            priority: 0,
            entity_types: vec![
                "3DSOLID".into(),
                "BODY".into(),
                "REGION".into(),
                "SURFACE".into(),
            ],
            // No capability is claimed until an ACIS kernel is actually linked.
            capabilities: vec![],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|e| CadError::InvalidInput(e.to_string()))?;

        // Handle resolution is reported explicitly, never swallowed.
        if let GeometryHandle::Missing { key } = &request.geometry {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_MISSING_HANDLE,
                format!("geometry handle '{key}' could not be resolved"),
            ));
        }

        // An empty payload is not an empty-but-successful solid.
        if request.exchange.is_empty() {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_EMPTY_GEOMETRY,
                format!("{} payload carried no bytes", request.exchange.type_key()),
            ));
        }

        // A payload we cannot classify is its own category.
        if request.exchange.kind() == ExchangeKind::Unsupported {
            return Ok(Self::unsupported(
                &request.stamp,
                ExchangeKind::Unsupported,
                codes::KERNEL_UNSUPPORTED_EXCHANGE,
                format!(
                    "no decoder registered for exchange type '{}'",
                    request.exchange.type_key()
                ),
            ));
        }

        // Any remaining non-empty payload (raw SAT/SAB bytes or a neutral
        // B-rep): this conservative default evaluates none of them and never
        // fabricates a mesh. Opting into the documented subset means using
        // `BrepTessellator`.
        Ok(Self::unsupported(
            &request.stamp,
            request.exchange.kind(),
            codes::KERNEL_NO_ACIS_KERNEL,
            format!(
                "this build's default kernel evaluates no ACIS payload; {} payload left unsupported",
                request.exchange.kind().as_str()
            ),
        ))
    }
}
