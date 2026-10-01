//! Host file closure: unsaved-decision orchestration, atomic export and the
//! recovery cache (spec §9, F01/F09/F12; audit B04/B05/B06/B07).
//!
//! This module is the host-agnostic half of "a host is about to replace the
//! document with a new file". It contains no platform API: the durable write is
//! supplied by the host as a closure (browser download, Android file write, a
//! test spy) and the recovery cache goes through the shared
//! [`cad_platform::Persistence`] contract. Both hosts therefore drive the *same*
//! business path and cannot drift apart.
//!
//! Invariants encoded here (all covered by tests):
//!
//! * **No silent default decision.** A host that needs to replace a dirty
//!   document must supply an [`UnsavedDecision`]; [`UnsavedDecisionSource`]
//!   returns `None` when the host cannot ask, and the caller must refuse the
//!   open rather than guess.
//! * **A failed save never replaces the document.** [`export_annotations_atomically`]
//!   only confirms the export revision after the host reports the write
//!   succeeded; a failed or unconfirmed write leaves the document dirty.
//! * **Cancel keeps everything.** `resolve_leave` never writes for `Cancel`.
//! * **A host without recovery storage is explicit.** `PreserveRecovery` on a
//!   host with `persistence == None` fails with `Unsupported`; it is never
//!   treated as "recovered".

use std::sync::Arc;

use cad_domain::{CadError, CadResult, DocumentId};
use cad_platform::Persistence;

use crate::host::HostController;
use crate::recovery::{RecoverySnapshot, UnsavedDecision};

/// What a dirty document needs before the host may replace it.
///
/// This is the *write plan*, distinct from [`crate::recovery::UnsavedFlow`]
/// which is the *outcome model*: the plan tells the host which host-owned write
/// to perform, and the flow validates the confirmed result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeavePlan {
    /// Nothing to persist: clean document, or `Discard`.
    Proceed,
    /// Persist the annotation sidecar through the atomic export, then proceed.
    Save,
    /// Persist a recovery snapshot through [`Persistence`], then proceed.
    PreserveRecovery,
    /// The user cancelled; keep the document and any recovery data.
    Cancel,
}

/// The pure mapping from dirty state + decision to the required host write.
///
/// A clean document never needs a write, so every decision plans `Proceed`
/// (including `Cancel`, which would otherwise block an open for no reason).
pub fn plan_leave(dirty: bool, decision: UnsavedDecision) -> LeavePlan {
    if !dirty {
        return LeavePlan::Proceed;
    }
    match decision {
        UnsavedDecision::Save => LeavePlan::Save,
        UnsavedDecision::PreserveRecovery => LeavePlan::PreserveRecovery,
        UnsavedDecision::Discard => LeavePlan::Proceed,
        UnsavedDecision::Cancel => LeavePlan::Cancel,
    }
}

/// Human-facing state of a document that a host is about to leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsavedSignal {
    pub dirty: bool,
    pub annotation_count: usize,
    pub name_hint: String,
}

/// Host-supplied source of an unsaved-work decision.
///
/// `None` means the host could not obtain a decision (no dialog, cancelled
/// prompt, lost permission). The caller must then keep the current document:
/// there is deliberately no `Default`/`unwrap` fallback here.
pub trait UnsavedDecisionSource {
    fn decide(&self, signal: &UnsavedSignal) -> Option<UnsavedDecision>;
}

/// Confirmed results of the host writes performed for a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaveResolution {
    /// The atomic annotation export was confirmed durably written.
    pub saved: bool,
    /// A recovery snapshot was persisted through [`Persistence`].
    pub recovery_persisted: bool,
    /// The snapshot that was persisted, for status/diagnostics.
    pub snapshot: Option<RecoverySnapshot>,
}

impl LeaveResolution {
    /// Nothing needed writing (clean document or `Discard`).
    pub fn proceed() -> Self {
        LeaveResolution {
            saved: false,
            recovery_persisted: false,
            snapshot: None,
        }
    }
}

/// Serialize a snapshot to the bytes [`Persistence`] stores.
pub fn recovery_bytes(snapshot: &RecoverySnapshot) -> Arc<[u8]> {
    Arc::from(snapshot.encode().into_bytes().into_boxed_slice())
}

/// Parse bytes read back from [`Persistence`].
///
/// Corrupt or truncated bytes are [`CadError::CorruptData`], never a fake empty
/// snapshot (a lost recovery copy must be visible, not silent).
pub fn parse_recovery(bytes: &[u8]) -> CadResult<RecoverySnapshot> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| CadError::CorruptData(format!("recovery snapshot is not UTF-8: {e}")))?;
    RecoverySnapshot::decode(text).ok_or_else(|| {
        CadError::CorruptData("recovery snapshot is malformed or version-unknown".into())
    })
}

/// Persist a snapshot through the shared recovery cache.
pub async fn persist_recovery(
    persistence: &dyn Persistence,
    document: DocumentId,
    snapshot: &RecoverySnapshot,
) -> CadResult<()> {
    persistence
        .save_recovery(document, recovery_bytes(snapshot))
        .await
}

/// Load and decode a recovery snapshot, if one is stored.
pub async fn load_recovery(
    persistence: &dyn Persistence,
    document: DocumentId,
) -> CadResult<Option<RecoverySnapshot>> {
    match persistence.load_recovery(document).await? {
        Some(bytes) => parse_recovery(&bytes).map(Some),
        None => Ok(None),
    }
}

/// Drop the recovery cache only after the durable write of the real file.
pub async fn discard_recovery(
    persistence: &dyn Persistence,
    document: DocumentId,
) -> CadResult<()> {
    persistence
        .discard_recovery_after_confirmation(document)
        .await
}

/// Perform the atomic annotation export: prepare (pure) → host write → confirm.
///
/// `write` receives the encoded sidecar JSON and returns whether the host
/// *confirmed* the durable write. Only a confirmed write marks the exact
/// exported revision saved; a failed or unconfirmed write returns an error and
/// leaves the document dirty (audit B07).
pub fn export_annotations_atomically<W>(controller: &mut HostController, write: W) -> CadResult<()>
where
    W: FnOnce(&str) -> bool,
{
    let (json, revision) = controller.prepare_annotation_export()?;
    if !write(&json) {
        return Err(CadError::Unsupported(
            "host write was not confirmed; annotations remain unsaved".into(),
        ));
    }
    controller.confirm_annotation_export(revision)
}

/// Execute the host writes a decision requires, returning the confirmed results
/// the host passes to [`HostController::open_bytes_leaving`].
///
/// `save` is the platform's atomic annotation writer (typically
/// [`export_annotations_atomically`]); it is only invoked for a `Save` plan.
/// `persistence` is `None` for a host without a recovery cache; requesting
/// `PreserveRecovery` there is an explicit `Unsupported`, never a silent no-op.
pub async fn resolve_leave<F>(
    controller: &mut HostController,
    decision: UnsavedDecision,
    camera_center: [f64; 3],
    camera_world_per_px: f64,
    persistence: Option<&dyn Persistence>,
    save: F,
) -> CadResult<LeaveResolution>
where
    F: FnOnce(&mut HostController) -> CadResult<()>,
{
    let dirty = controller
        .workspace_annotations()
        .map(|a| a.is_dirty())
        .unwrap_or(false);
    match plan_leave(dirty, decision) {
        LeavePlan::Proceed => Ok(LeaveResolution::proceed()),
        LeavePlan::Cancel => Err(CadError::Cancelled),
        LeavePlan::Save => {
            save(controller)?;
            Ok(LeaveResolution {
                saved: true,
                recovery_persisted: false,
                snapshot: None,
            })
        }
        LeavePlan::PreserveRecovery => {
            let persistence = persistence.ok_or_else(|| {
                CadError::Unsupported(
                    "host has no recovery storage; cannot preserve unsaved work".into(),
                )
            })?;
            let snapshot =
                controller.capture_recovery_snapshot(camera_center, camera_world_per_px)?;
            persist_recovery(persistence, controller.document_id, &snapshot).await?;
            Ok(LeaveResolution {
                saved: false,
                recovery_persisted: true,
                snapshot: Some(snapshot),
            })
        }
    }
}

/// Parse a decision name supplied by a host bridge (`"save"`, `"recovery"`,
/// `"discard"`, `"cancel"`, case-insensitive).
pub fn parse_decision(name: &str) -> Option<UnsavedDecision> {
    match name.trim().to_ascii_lowercase().as_str() {
        "save" => Some(UnsavedDecision::Save),
        "recovery" | "preserve" | "preservereecovery" => Some(UnsavedDecision::PreserveRecovery),
        "discard" => Some(UnsavedDecision::Discard),
        "cancel" => Some(UnsavedDecision::Cancel),
        _ => None,
    }
}

/// Stable lowercase name for a decision, for host bridges and status lines.
pub fn decision_name(decision: UnsavedDecision) -> &'static str {
    match decision {
        UnsavedDecision::Save => "save",
        UnsavedDecision::PreserveRecovery => "recovery",
        UnsavedDecision::Discard => "discard",
        UnsavedDecision::Cancel => "cancel",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_annotations::AnnotationCommand;
    use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle};
    use cad_domain::{AnnotationId, Point3, Precision, SpaceId};
    use std::cell::RefCell;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn note(id: u128, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            text: text.into(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    fn dirty_controller() -> HostController {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(note(1, "unsaved")))
            .unwrap();
        controller
    }

    /// In-memory [`Persistence`] that records what was written per document.
    #[derive(Default)]
    struct FakePersistence {
        stored: RefCell<std::collections::BTreeMap<u128, Vec<u8>>>,
        fail_save: bool,
    }

    impl Persistence for FakePersistence {
        fn save_recovery(
            &self,
            document: DocumentId,
            bytes: Arc<[u8]>,
        ) -> cad_platform::HostFuture<'_, ()> {
            Box::pin(async move {
                if self.fail_save {
                    return Err(CadError::ResourceMissing("disk full".into()));
                }
                self.stored.borrow_mut().insert(document.0, bytes.to_vec());
                Ok(())
            })
        }

        fn load_recovery(
            &self,
            document: DocumentId,
        ) -> cad_platform::HostFuture<'_, Option<Arc<[u8]>>> {
            Box::pin(async move {
                Ok(self
                    .stored
                    .borrow()
                    .get(&document.0)
                    .map(|b| Arc::from(b.clone().into_boxed_slice())))
            })
        }

        fn discard_recovery_after_confirmation(
            &self,
            document: DocumentId,
        ) -> cad_platform::HostFuture<'_, ()> {
            Box::pin(async move {
                self.stored.borrow_mut().remove(&document.0);
                Ok(())
            })
        }
    }

    #[test]
    fn plan_matches_the_decision_model() {
        assert_eq!(plan_leave(false, UnsavedDecision::Save), LeavePlan::Proceed);
        assert_eq!(
            plan_leave(false, UnsavedDecision::Cancel),
            LeavePlan::Proceed,
            "nothing unsaved to protect on a clean document"
        );
        assert_eq!(plan_leave(true, UnsavedDecision::Save), LeavePlan::Save);
        assert_eq!(
            plan_leave(true, UnsavedDecision::PreserveRecovery),
            LeavePlan::PreserveRecovery
        );
        assert_eq!(
            plan_leave(true, UnsavedDecision::Discard),
            LeavePlan::Proceed
        );
        assert_eq!(plan_leave(true, UnsavedDecision::Cancel), LeavePlan::Cancel);
    }

    #[test]
    fn atomic_export_confirms_only_on_a_successful_write() {
        let mut controller = dirty_controller();
        // A writer that reports failure must not mark the document saved.
        let result = export_annotations_atomically(&mut controller, |_json| false);
        assert!(matches!(result, Err(CadError::Unsupported(_))));
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        assert_eq!(controller.workspace_annotations().unwrap().len(), 1);
    }

    #[test]
    fn atomic_export_marks_saved_after_a_confirmed_write() {
        let mut controller = dirty_controller();
        let mut seen_json = String::new();
        export_annotations_atomically(&mut controller, |json| {
            assert!(json.contains("schema_version"));
            seen_json = json.to_string();
            true
        })
        .unwrap();
        assert!(seen_json.contains("document_fingerprint"));
        assert!(!controller.workspace_annotations().unwrap().is_dirty());
    }

    #[test]
    fn recovery_bytes_round_trip_and_reject_corruption() {
        let snapshot = RecoverySnapshot {
            identity: cad_domain::DocumentIdentity::Temporary(7),
            name_hint: "plan.dwg".into(),
            annotations_json: "{}".into(),
            camera_center: [1.0, 2.0, 3.0],
            camera_world_per_px: 0.25,
        };
        let bytes = recovery_bytes(&snapshot);
        let decoded = parse_recovery(&bytes).unwrap();
        assert_eq!(decoded, snapshot);
        assert!(matches!(
            parse_recovery(b"not json"),
            Err(CadError::CorruptData(_))
        ));
        assert!(matches!(
            parse_recovery(&[0xff, 0xfe]),
            Err(CadError::CorruptData(_))
        ));
    }

    #[test]
    fn persist_then_load_recovery_through_persistence() {
        let persistence = FakePersistence::default();
        let snapshot = RecoverySnapshot {
            identity: cad_domain::DocumentIdentity::Temporary(1),
            name_hint: "demo".into(),
            annotations_json: "{}".into(),
            camera_center: [0.0, 0.0, 0.0],
            camera_world_per_px: 1.0,
        };
        block_on(persist_recovery(&persistence, DocumentId(1), &snapshot)).unwrap();
        let loaded = block_on(load_recovery(&persistence, DocumentId(1)))
            .unwrap()
            .expect("stored");
        assert_eq!(loaded, snapshot);
        // A different document has its own slot.
        assert!(block_on(load_recovery(&persistence, DocumentId(2)))
            .unwrap()
            .is_none());
        block_on(discard_recovery(&persistence, DocumentId(1))).unwrap();
        assert!(block_on(load_recovery(&persistence, DocumentId(1)))
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolve_leave_cancel_writes_nothing_and_keeps_the_document() {
        let mut controller = dirty_controller();
        let persistence = FakePersistence::default();
        let result = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::Cancel,
            [0.0, 0.0, 0.0],
            1.0,
            Some(&persistence),
            |_| panic!("save must not run for Cancel"),
        ));
        assert!(matches!(result, Err(CadError::Cancelled)));
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        assert!(persistence.stored.borrow().is_empty());
    }

    #[test]
    fn resolve_leave_save_failure_is_not_reported_as_saved() {
        let mut controller = dirty_controller();
        let result = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::Save,
            [0.0, 0.0, 0.0],
            1.0,
            None,
            |ctrl| export_annotations_atomically(ctrl, |_| false),
        ));
        assert!(result.is_err());
        assert!(controller.workspace_annotations().unwrap().is_dirty());
    }

    #[test]
    fn resolve_leave_preserve_recovery_persists_a_snapshot() {
        let mut controller = dirty_controller();
        let persistence = FakePersistence::default();
        let resolution = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::PreserveRecovery,
            [5.0, 6.0, 0.0],
            0.5,
            Some(&persistence),
            |_| panic!("save must not run for PreserveRecovery"),
        ))
        .unwrap();
        assert!(resolution.recovery_persisted);
        assert!(!resolution.saved);
        let stored = persistence.stored.borrow();
        let bytes = stored.get(&1).expect("snapshot for document 1");
        let decoded = parse_recovery(bytes).unwrap();
        assert_eq!(decoded.camera_center, [5.0, 6.0, 0.0]);
        // Persisting a recovery copy does not mark the annotations saved.
        assert!(controller.workspace_annotations().unwrap().is_dirty());
    }

    #[test]
    fn resolve_leave_without_storage_is_explicit() {
        let mut controller = dirty_controller();
        let result = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::PreserveRecovery,
            [0.0, 0.0, 0.0],
            1.0,
            None,
            |_| panic!("save must not run for PreserveRecovery"),
        ));
        assert!(matches!(result, Err(CadError::Unsupported(_))));
    }

    #[test]
    fn resolve_leave_recovery_write_failure_does_not_proceed() {
        let mut controller = dirty_controller();
        let persistence = FakePersistence {
            fail_save: true,
            ..FakePersistence::default()
        };
        let result = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::PreserveRecovery,
            [0.0, 0.0, 0.0],
            1.0,
            Some(&persistence),
            |_| panic!("save must not run for PreserveRecovery"),
        ));
        assert!(matches!(result, Err(CadError::ResourceMissing(_))));
    }

    #[test]
    fn clean_document_leaves_without_any_write() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let resolution = block_on(resolve_leave(
            &mut controller,
            UnsavedDecision::Save,
            [0.0, 0.0, 0.0],
            1.0,
            None,
            |_| panic!("nothing to save on a clean document"),
        ))
        .unwrap();
        assert_eq!(resolution, LeaveResolution::proceed());
    }

    #[test]
    fn decision_names_round_trip() {
        for decision in [
            UnsavedDecision::Save,
            UnsavedDecision::PreserveRecovery,
            UnsavedDecision::Discard,
            UnsavedDecision::Cancel,
        ] {
            assert_eq!(parse_decision(decision_name(decision)), Some(decision));
        }
        assert_eq!(
            parse_decision("preserve"),
            Some(UnsavedDecision::PreserveRecovery)
        );
        assert_eq!(parse_decision("nonsense"), None);
    }
}
