//! Backend selection outcome model (spec F12).
//!
//! This module is the *pure* half of the app/bridge contract: it models what a
//! backend selection attempt produced. It performs no GPU work and no platform
//! I/O, so it is testable on the host and on wasm32. The Slint host and the
//! bridge only map these values to real device initialization.
//!
//! **A failed backend is never reported as success.** [`BackendOutcome`]
//! separates "initialized" from "failed" and always names the backend that was
//! actually active, never the preference that was merely requested.

use crate::BackendChoice;

/// A render backend that is actually active, labelled truthfully.
///
/// This mirrors `cad_render_wgpu::ActiveBackend` without depending on the
/// renderer crate from `cad-app` (the app layer is platform- and GPU-free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackendKind {
    WebGpu,
    WebGl2,
    /// Native wgpu (desktop/Android shared device); not a browser tier.
    Native,
}

impl ActiveBackendKind {
    /// Stable lowercase label for UI/CLI/diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            ActiveBackendKind::WebGpu => "webgpu",
            ActiveBackendKind::WebGl2 => "webgl2",
            ActiveBackendKind::Native => "native",
        }
    }
}

/// Why a backend selection attempt could not produce a usable device.
///
/// The variants are deliberately distinguishable: the UI must be able to tell a
/// *forced* backend that was rejected from *Auto* finding nothing at all, and
/// from a device that initialized and then failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendFailure {
    /// `Auto` probed both tiers and neither could be created.
    NoBackendAvailable,
    /// A forced backend was requested but the platform refused the adapter or
    /// context. Carries the reason verbatim.
    ForcedUnavailable { reason: String },
    /// The preference was accepted but device/renderer initialization failed.
    InitFailed { reason: String },
}

impl BackendFailure {
    /// Human-facing recovery action the UI should offer, never a bare "error".
    pub fn recovery_hint(&self) -> &'static str {
        match self {
            BackendFailure::NoBackendAvailable => {
                "没有可用的 WebGPU/WebGL2 设备；请更新浏览器或返回 Auto"
            }
            BackendFailure::ForcedUnavailable { .. } => {
                "强制后端不可用；可回退到 Auto 或改用另一后端"
            }
            BackendFailure::InitFailed { .. } => "设备初始化失败；请重试或切换后端",
        }
    }
}

/// The outcome of a backend selection/initialization attempt.
///
/// This replaces the previous generic `String` error slot on the bridge: callers
/// can now branch on whether a backend is live, which one it is, and — when it
/// is not live — the exact failure and the preference that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendOutcome {
    /// A device is live and the renderer initialized. `actual` is the backend
    /// that really activated; `preference` is what was asked for.
    Initialized {
        preference: BackendChoice,
        actual: ActiveBackendKind,
    },
    /// No live device. The reason is explicit and never a generic string.
    Failed {
        preference: BackendChoice,
        failure: BackendFailure,
    },
}

impl BackendOutcome {
    pub fn is_live(&self) -> bool {
        matches!(self, BackendOutcome::Initialized { .. })
    }

    /// The backend that is actually active, if any.
    pub fn actual(&self) -> Option<ActiveBackendKind> {
        match self {
            BackendOutcome::Initialized { actual, .. } => Some(*actual),
            BackendOutcome::Failed { .. } => None,
        }
    }

    /// One-line diagnostic that labels the real backend truthfully.
    ///
    /// A forced WebGL2 preference that actually ran on a WebGPU device is
    /// reported as WebGPU: the label follows the device, not the wish.
    pub fn label(&self) -> String {
        match self {
            BackendOutcome::Initialized { preference, actual } => {
                if preference_matches(*preference, *actual) {
                    format!("后端 {}（实际运行）", actual.as_str())
                } else {
                    format!(
                        "后端 {}（请求 {preference:?}，实际运行 {}）",
                        actual.as_str(),
                        actual.as_str()
                    )
                }
            }
            BackendOutcome::Failed {
                preference,
                failure,
            } => {
                format!(
                    "后端失败（请求 {preference:?}）：{}",
                    failure_label(failure)
                )
            }
        }
    }
}

fn preference_matches(preference: BackendChoice, actual: ActiveBackendKind) -> bool {
    match preference {
        BackendChoice::Auto => true,
        BackendChoice::WebGpu => actual == ActiveBackendKind::WebGpu,
        BackendChoice::WebGl2 => actual == ActiveBackendKind::WebGl2,
    }
}

fn failure_label(failure: &BackendFailure) -> String {
    match failure {
        BackendFailure::NoBackendAvailable => "无可用的 WebGPU/WebGL2".into(),
        BackendFailure::ForcedUnavailable { reason } => format!("强制后端不可用：{reason}"),
        BackendFailure::InitFailed { reason } => format!("设备初始化失败：{reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome_initialized(preference: BackendChoice, actual: ActiveBackendKind) -> BackendOutcome {
        BackendOutcome::Initialized { preference, actual }
    }

    #[test]
    fn initialized_outcome_is_live_and_names_the_actual_backend() {
        let outcome = outcome_initialized(BackendChoice::WebGpu, ActiveBackendKind::WebGpu);
        assert!(outcome.is_live());
        assert_eq!(outcome.actual(), Some(ActiveBackendKind::WebGpu));
        assert_eq!(outcome.label(), "后端 webgpu（实际运行）");
    }

    #[test]
    fn forced_webgl2_that_ran_on_webgpu_is_labelled_webgpu() {
        // The truth follows the device: a forced preference that actually got a
        // WebGPU device must not be labelled WebGL2 (audit F12 "GL 桥仍标 WebGPU").
        let outcome = outcome_initialized(BackendChoice::WebGl2, ActiveBackendKind::WebGpu);
        assert!(outcome.is_live());
        assert_eq!(outcome.actual(), Some(ActiveBackendKind::WebGpu));
        assert!(outcome.label().contains("webgpu"));
        assert!(!outcome.label().contains("webgl2"));
    }

    #[test]
    fn failed_outcome_is_not_live_and_carries_a_specific_reason() {
        let auto = BackendOutcome::Failed {
            preference: BackendChoice::Auto,
            failure: BackendFailure::NoBackendAvailable,
        };
        assert!(!auto.is_live());
        assert_eq!(auto.actual(), None);
        assert!(auto.label().contains("无可用的 WebGPU/WebGL2"));

        let forced = BackendOutcome::Failed {
            preference: BackendChoice::WebGl2,
            failure: BackendFailure::ForcedUnavailable {
                reason: "webgl2 context rejected".into(),
            },
        };
        assert_ne!(auto, forced);
        assert!(forced.label().contains("webgl2 context rejected"));
        assert!(
            forced.label().contains(
                BackendFailure::ForcedUnavailable {
                    reason: String::new()
                }
                .recovery_hint()
            ) || !forced.label().contains("设备初始化失败")
        );
    }

    #[test]
    fn init_failure_is_distinct_from_forced_unavailable() {
        let a = BackendOutcome::Failed {
            preference: BackendChoice::WebGpu,
            failure: BackendFailure::InitFailed {
                reason: "pipeline creation failed".into(),
            },
        };
        let b = BackendOutcome::Failed {
            preference: BackendChoice::WebGpu,
            failure: BackendFailure::ForcedUnavailable {
                reason: "pipeline creation failed".into(),
            },
        };
        assert_ne!(a, b, "failure kind must be distinguishable");
        assert!(a.label().contains("设备初始化失败"));
    }
}
