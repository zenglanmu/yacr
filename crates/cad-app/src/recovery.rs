//! Backend outcome + unsaved-work decision model (spec F12, audit B07/U09).
//!
//! This module is the *pure* half of the app/bridge contract: it models what a
//! backend selection attempt produced, and what the user decided about unsaved
//! annotation work. It performs no GPU work and no platform I/O, so it is
//! testable on the host and on wasm32. The Slint host and the bridge only map
//! these values to real device initialization and file writes.
//!
//! Two invariants the audit calls out are encoded here:
//!
//! 1. **A failed backend is never reported as success.** [`BackendOutcome`]
//!    separates "initialized" from "failed" and always names the backend that
//!    was actually active, never the preference that was merely requested.
//! 2. **A failed save is never reported as saved.** [`UnsavedFlow`] only reaches
//!    [`UnsavedOutcome::Proceed`] for a `Save` decision after the host confirms
//!    the durable write; a failed write stays at [`UnsavedOutcome::SaveFailed`]
//!    and keeps the document and its recovery data.

use cad_domain::DocumentIdentity;

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
            BackendFailure::InitFailed { .. } => {
                "设备初始化失败；请重试或切换后端（未保存批注仍受保护）"
            }
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

/// The user's decision about unsaved annotation work at a document switch/close
/// (spec §16.3, audit U09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedDecision {
    /// Persist through the host's real export path, then proceed. A failed write
    /// must not be treated as saved.
    Save,
    /// Keep a recovery copy, then proceed. If the recovery write fails the flow
    /// must not proceed.
    PreserveRecovery,
    /// Drop the unsaved work on purpose.
    Discard,
    /// Do nothing: keep the current document and its recovery data.
    Cancel,
}

/// The result of applying an [`UnsavedDecision`] to a dirty document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedOutcome {
    /// The decision is satisfied; the caller may replace/close the document.
    Proceed,
    /// `Save` was chosen but the durable write has not been confirmed. The
    /// document and its recovery data are untouched; the UI must report the
    /// failure and keep the decision prompt open (audit B07).
    SaveFailed,
    /// A recovery copy was required but could not be written durably. Nothing
    /// is discarded.
    RecoveryFailed,
    /// `Cancel`: the current document and any recovery data are retained.
    Cancelled,
}

/// The decision flow for one dirty document.
///
/// The flow has no side effects of its own: the host supplies the results of the
/// real write/recovery operations, so this stays pure and testable while the
/// actual atomic `cad-annotations` export happens in `HostController`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsavedFlow {
    dirty: bool,
}

impl UnsavedFlow {
    /// Start a flow for a document that is (or is not) dirty.
    pub fn new(dirty: bool) -> Self {
        UnsavedFlow { dirty }
    }

    /// Apply a decision, given whether the host's `Save` write succeeded and
    /// whether the recovery snapshot was persisted.
    ///
    /// * A clean document always proceeds, regardless of decision.
    /// * `Save` on a dirty document proceeds only when `save_succeeded` is true.
    /// * `PreserveRecovery` on a dirty document proceeds only when
    ///   `recovery_succeeded` is true.
    /// * `Discard` always proceeds (the user asked to drop the work).
    /// * `Cancel` on a dirty document never proceeds; on a clean document there
    ///   is no unsaved work to protect, so it proceeds too.
    pub fn apply(
        self,
        decision: UnsavedDecision,
        save_succeeded: bool,
        recovery_succeeded: bool,
    ) -> UnsavedOutcome {
        if !self.dirty {
            // Nothing unsaved, so nothing needs protecting: a caller who has not
            // asked to overwrite unsaved work proceeds either way.
            return UnsavedOutcome::Proceed;
        }
        match decision {
            UnsavedDecision::Save => {
                if save_succeeded {
                    UnsavedOutcome::Proceed
                } else {
                    UnsavedOutcome::SaveFailed
                }
            }
            UnsavedDecision::PreserveRecovery => {
                if recovery_succeeded {
                    UnsavedOutcome::Proceed
                } else {
                    UnsavedOutcome::RecoveryFailed
                }
            }
            UnsavedDecision::Discard => UnsavedOutcome::Proceed,
            UnsavedDecision::Cancel => UnsavedOutcome::Cancelled,
        }
    }
}

/// A recovery snapshot of unsaved annotation work.
///
/// The snapshot is the versioned annotation sidecar JSON (produced by the same
/// atomic `cad-annotations` encoder the export uses) plus the document identity
/// it belongs to and the camera, so a restored session rebuilds the scene from
/// the database rather than from any lost GPU state. It is deliberately a plain
/// struct so hosts can persist it in localStorage / a recovery file without
/// `cad-app` depending on any platform API.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoverySnapshot {
    pub identity: DocumentIdentity,
    pub name_hint: String,
    /// Versioned annotation sidecar JSON (schema-checked on restore).
    pub annotations_json: String,
    pub camera_center: [f64; 3],
    pub camera_world_per_px: f64,
    /// Unknown top-level recovery fields, preserved verbatim so a newer writer's
    /// data survives an older reader instead of being silently dropped.
    pub extensions_json: std::collections::BTreeMap<String, String>,
}

impl Default for RecoverySnapshot {
    fn default() -> Self {
        Self {
            identity: DocumentIdentity::Temporary(0),
            name_hint: String::new(),
            annotations_json: String::new(),
            camera_center: [0.0; 3],
            camera_world_per_px: 1.0,
            extensions_json: std::collections::BTreeMap::new(),
        }
    }
}

/// Top-level recovery snapshot fields the current schema defines.
const KNOWN_RECOVERY_FIELDS: [&str; 6] = [
    "version",
    "identity",
    "name_hint",
    "camera_center",
    "camera_world_per_px",
    "annotations",
];

impl RecoverySnapshot {
    /// Serialize to a single JSON object for storage.
    pub fn encode(&self) -> String {
        let identity = match self.identity {
            DocumentIdentity::Temporary(id) => format!("tmp:{id}"),
            DocumentIdentity::Sha256(bytes) => {
                let mut s = String::with_capacity(64);
                for byte in bytes {
                    s.push_str(&format!("{byte:02x}"));
                }
                format!("sha256:{s}")
            }
        };
        let camera_center = format!(
            "[{},{},{}]",
            self.camera_center[0], self.camera_center[1], self.camera_center[2]
        );
        // The annotation JSON is already a JSON object; embed the metadata
        // around it without re-escaping the payload by storing it as a nested
        // object under a dedicated key.
        let mut out = format!(
            "{{\"version\":1,\"identity\":{},\"name_hint\":{},\"camera_center\":{},\"camera_world_per_px\":{},\"annotations\":{}",
            json_string(&identity),
            json_string(&self.name_hint),
            camera_center,
            json_number(self.camera_world_per_px),
            self.annotations_json,
        );
        // Preserved unknown top-level fields are appended verbatim as raw JSON.
        for (key, raw) in &self.extensions_json {
            out.push(',');
            out.push_str(&json_string(key));
            out.push(':');
            out.push_str(raw);
        }
        out.push('}');
        out
    }

    /// Parse a snapshot produced by [`RecoverySnapshot::encode`].
    ///
    /// Returns `None` for malformed input rather than fabricating an empty
    /// snapshot, so a corrupt recovery copy is never silently treated as "no
    /// unsaved work". A version *newer* than this build understands is also
    /// refused (never silently downgraded); unknown top-level fields are
    /// preserved verbatim.
    pub fn decode(text: &str) -> Option<RecoverySnapshot> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let version = object.get("version")?.as_u64()?;
        if version != 1 {
            return None;
        }
        let identity = parse_identity(object.get("identity")?.as_str()?)?;
        let name_hint = object.get("name_hint")?.as_str()?.to_string();
        let camera = object.get("camera_center")?.as_array()?;
        if camera.len() != 3 {
            return None;
        }
        let camera_center = [
            camera[0].as_f64()?,
            camera[1].as_f64()?,
            camera[2].as_f64()?,
        ];
        let camera_world_per_px = object.get("camera_world_per_px")?.as_f64()?;
        let annotations = object.get("annotations")?;
        let annotations_json = serde_json::to_string(annotations).ok()?;
        let mut extensions_json = std::collections::BTreeMap::new();
        for (key, value) in object {
            if !KNOWN_RECOVERY_FIELDS.contains(&key.as_str()) {
                extensions_json.insert(key.clone(), value.to_string());
            }
        }
        Some(RecoverySnapshot {
            identity,
            name_hint,
            annotations_json,
            camera_center,
            camera_world_per_px,
            extensions_json,
        })
    }
}

fn parse_identity(text: &str) -> Option<DocumentIdentity> {
    if let Some(hex) = text.strip_prefix("tmp:") {
        return hex.parse::<u128>().ok().map(DocumentIdentity::Temporary);
    }
    let hex = text.strip_prefix("sha256:")?;
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(DocumentIdentity::Sha256(bytes))
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn json_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "0".into()
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

    #[test]
    fn dirty_save_failure_does_not_proceed() {
        let flow = UnsavedFlow::new(true);
        assert_eq!(
            flow.apply(UnsavedDecision::Save, false, false),
            UnsavedOutcome::SaveFailed
        );
        assert_eq!(
            flow.apply(UnsavedDecision::Save, true, false),
            UnsavedOutcome::Proceed
        );
    }

    #[test]
    fn recovery_failure_does_not_proceed_but_success_does() {
        let flow = UnsavedFlow::new(true);
        assert_eq!(
            flow.apply(UnsavedDecision::PreserveRecovery, false, false),
            UnsavedOutcome::RecoveryFailed
        );
        assert_eq!(
            flow.apply(UnsavedDecision::PreserveRecovery, false, true),
            UnsavedOutcome::Proceed
        );
    }

    #[test]
    fn cancel_never_proceeds_and_discard_always_does() {
        let flow = UnsavedFlow::new(true);
        assert_eq!(
            flow.apply(UnsavedDecision::Cancel, true, true),
            UnsavedOutcome::Cancelled
        );
        assert_eq!(
            flow.apply(UnsavedDecision::Discard, false, false),
            UnsavedOutcome::Proceed
        );
    }

    #[test]
    fn clean_document_proceeds_for_every_decision() {
        let flow = UnsavedFlow::new(false);
        // With nothing unsaved there is nothing Cancel could protect, so every
        // decision proceeds rather than blocking the caller.
        for decision in [
            UnsavedDecision::Save,
            UnsavedDecision::PreserveRecovery,
            UnsavedDecision::Discard,
            UnsavedDecision::Cancel,
        ] {
            assert_eq!(
                flow.apply(decision, false, false),
                UnsavedOutcome::Proceed,
                "{decision:?}"
            );
        }
    }

    #[test]
    fn recovery_snapshot_round_trips_every_field() {
        let snapshot = RecoverySnapshot {
            identity: DocumentIdentity::Sha256([0xAB; 32]),
            name_hint: "plan.dwg".into(),
            annotations_json: "{\"schema_version\":1,\"annotations\":[]}".into(),
            camera_center: [1.5, -2.0, 3.25],
            camera_world_per_px: 0.125,
            ..Default::default()
        };
        let encoded = snapshot.encode();
        let decoded = RecoverySnapshot::decode(&encoded).expect("round trip");
        // Annotation JSON is compared semantically: key order in a JSON object
        // is not significant, and the payload legitimately re-serializes in the
        // parser's own order.
        let before: serde_json::Value = serde_json::from_str(&snapshot.annotations_json).unwrap();
        let after: serde_json::Value = serde_json::from_str(&decoded.annotations_json).unwrap();
        assert_eq!(before, after);
        assert_eq!(decoded.identity, snapshot.identity);
        assert_eq!(decoded.name_hint, snapshot.name_hint);
        assert_eq!(decoded.camera_center, snapshot.camera_center);
        assert_eq!(decoded.camera_world_per_px, snapshot.camera_world_per_px);
    }

    #[test]
    fn recovery_snapshot_round_trips_a_temporary_identity() {
        let snapshot = RecoverySnapshot {
            identity: DocumentIdentity::Temporary(42),
            name_hint: "tmp".into(),
            annotations_json: "{}".into(),
            camera_center: [0.0, 0.0, 0.0],
            camera_world_per_px: 1.0,
            ..Default::default()
        };
        let decoded = RecoverySnapshot::decode(&snapshot.encode()).unwrap();
        assert_eq!(decoded.identity, DocumentIdentity::Temporary(42));
    }

    #[test]
    fn malformed_recovery_snapshot_is_rejected_not_faked() {
        assert!(RecoverySnapshot::decode("not json").is_none());
        assert!(RecoverySnapshot::decode("{}").is_none());
        assert!(RecoverySnapshot::decode("{\"version\":2}").is_none());
        // A truncated identity must not silently become "no unsaved work".
        assert!(RecoverySnapshot::decode(
            "{\"version\":1,\"identity\":\"sha256:ab\",\"name_hint\":\"x\",\"camera_center\":[0,0,0],\"camera_world_per_px\":1,\"annotations\":{}}"
        )
        .is_none());
    }

    #[test]
    fn recovery_snapshot_preserves_unknown_top_level_fields() {
        // A newer writer's extra metadata must survive a decode -> encode cycle
        // instead of being silently dropped by this build.
        let text = "{\"version\":1,\"identity\":\"tmp:7\",\"name_hint\":\"plan.dwg\",\
            \"camera_center\":[0,0,0],\"camera_world_per_px\":1,\
            \"annotations\":{\"schema_version\":1,\"annotations\":[]},\
            \"future_meta\":{\"pinned\":true},\"future_flag\":3}";
        let snapshot = RecoverySnapshot::decode(text).expect("decode");
        assert!(snapshot.extensions_json.contains_key("future_meta"));
        assert!(snapshot.extensions_json.contains_key("future_flag"));
        let reencoded = snapshot.encode();
        let again = RecoverySnapshot::decode(&reencoded).expect("re-decode");
        assert_eq!(again.extensions_json, snapshot.extensions_json);
        assert!(reencoded.contains("future_meta"));
        assert!(reencoded.contains("future_flag"));
    }

    #[test]
    fn recovery_snapshot_refuses_a_newer_version_instead_of_downgrading() {
        // Version 2 is not fabricated into a v1 snapshot; callers must treat it
        // as unreadable rather than "no unsaved work".
        assert!(RecoverySnapshot::decode(
            "{\"version\":2,\"identity\":\"tmp:1\",\"name_hint\":\"x\",\"camera_center\":[0,0,0],\"camera_world_per_px\":1,\"annotations\":{}}"
        )
        .is_none());
    }
}
