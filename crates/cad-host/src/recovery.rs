//! The recovery-snapshot lifecycle (ADR-0037 D12 "恢复快照生命周期").
//!
//! An explicit-save document loses every edit since the last save if the process
//! dies. A recovery snapshot bounds that window: while a document is dirty the
//! host writes the same archive it *would* have saved into a carrier-private
//! directory, at most once per [`RECOVERY_INTERVAL`]. Restore needs no command
//! and no dialog — the snapshot is an ordinary `.ymodel`, so the user opens it
//! through the existing OPEN path.
//!
//! The rules that keep this from becoming a rollback mechanism:
//!
//! * written **only while dirty**, at most once per interval, from the same
//!   `snapshot_document()` the save path uses;
//! * **discarded after a successful save** (a stale snapshot offered at the next
//!   start would silently undo saved work);
//! * offered only when `record.revision > saved_revision`; a snapshot at or
//!   below the saved revision is stale by definition and is discarded.
//!
//! Timing follows the hover dwell: a deadline checked once per tick, no timer
//! thread, on the crate's monotonic [`Instant`](crate::Instant).
//!
//! Nothing here ever writes the document's own file: the store is a separate
//! `cad_platform::Persistence` capability, and a failure is a status (and, for
//! the periodic write, one report per document) — never a failed command.

use super::*;
use cad_platform::{Persistence, RecoveryKey, RecoveryRecord};
use std::time::Duration;

/// The documented **default** recovery interval: 60 s, the ADR's "at most once
/// per minute" (short enough that a crash costs a minute of work, long enough
/// that a normal editing session does not rewrite a multi-megabyte archive on
/// every tick).
///
/// It is no longer the value the writer uses; the effective preference
/// (`ui.recovery.intervalSeconds`, [`cad_app::viewer_config::DEFAULT_RECOVERY_INTERVAL_SECONDS`])
/// is read per tick, and `0` disables the writes. This constant stays as the
/// schema default and the value a config-less caller documents.
pub const RECOVERY_INTERVAL: Duration =
    Duration::from_secs(cad_app::viewer_config::DEFAULT_RECOVERY_INTERVAL_SECONDS as u64);

impl HostCore {
    /// Install (or clear) the carrier's recovery-snapshot store.
    ///
    /// Carrier-owned, exactly like [`Self::set_document_archive`]: a browser or
    /// Android SAF carrier has no durable private directory, so it installs
    /// nothing and the host writes no snapshot. There is deliberately no
    /// [`cad_platform::NoRecovery`] default installed here — "this carrier has no
    /// recovery" and "recovery broke here" are different facts, and only the
    /// latter can be reported honestly from this layer.
    pub fn set_persistence(&self, persistence: Option<Arc<dyn Persistence>>) {
        *self.persistence.borrow_mut() = persistence;
    }

    /// The installed recovery store, if the carrier supplied one.
    pub fn persistence_store(&self) -> Option<Arc<dyn Persistence>> {
        self.persistence.borrow().clone()
    }

    /// Every snapshot the installed store offers, newest first (possibly empty).
    ///
    /// A carrier that installed no store has nothing to list, so the answer is
    /// the empty list — the same true answer [`Persistence::list_recovery`]'s
    /// default gives, and not a refusal: the caller renders a list, and "no
    /// snapshots here" is one of its states.
    ///
    /// A store that cannot read its own directory propagates its error: that is
    /// "there may be work that cannot be seen", which must never be shown as
    /// "nothing to recover".
    pub fn list_recovery(&self) -> CadResult<Vec<RecoveryRecord>> {
        let Some(store) = self.persistence_store() else {
            return Ok(Vec::new());
        };
        store.list_recovery()
    }

    /// Start opening the snapshot filed under `key`.
    ///
    /// `key` is the **only** input: it is validated as a [`RecoveryKey`] (hex
    /// only, so it can never name a path) and then looked up in the store's own
    /// list, so a crafted key cannot make the host open an arbitrary file. The
    /// path comes from [`Persistence::recovery_hint`] — the store's answer, never
    /// the caller's — and the open follows the ordinary `begin_open_target`
    /// funnel with the same admission rules as [`Self::open_recent`].
    pub fn open_recovery(&self, key: &str) -> CadResult<()> {
        let key = RecoveryKey::from_hex(key).ok_or_else(|| {
            CadError::InvalidInput(format!("recovery.invalid_key: {key:?} is not a hex key"))
        })?;
        let available = self.list_recovery()?;
        if !available.iter().any(|record| record.key == key.as_str()) {
            return Err(CadError::InvalidInput(format!(
                "recovery.unknown: {} is not an available snapshot",
                key.as_str()
            )));
        }
        let Some(store) = self.persistence_store() else {
            // Unreachable while `list_recovery` returned the key from a store, but
            // keep the explicit refusal instead of unwrapping.
            return Err(CadError::Unsupported(
                "recovery.unknown: no recovery store is installed".into(),
            ));
        };
        let Some(path) = store.recovery_hint(&key) else {
            // No filesystem path (a browser handle, a SAF URI): the host cannot
            // turn this key into something the open funnel can read, so it says
            // so rather than inventing a path.
            return Err(CadError::Unsupported(format!(
                "recovery.no_hint: {} has no openable location on this carrier",
                key.as_str()
            )));
        };
        // Same admission rules as the chooser funnel: an open never races a save
        // and never overlaps another load.
        if self.save_pending() {
            return Err(CadError::Unsupported(
                "file operation already in progress".into(),
            ));
        }
        if !self.options.headless && self.loading() {
            return Ok(());
        }
        let target = FileTarget::from_path(&path);
        self.begin_open_attempt();
        self.open_pending.set(true);
        if let Err(error) = self.files.begin_open_target(&target) {
            self.open_pending.set(false);
            return Err(error);
        }
        self.drain_files()?;
        if self.open_pending.get() {
            if !self.options.headless {
                self.set_catalog_status("linux.picker_waiting", &[])?;
            }
        } else {
            self.grade_pending_open()?;
        }
        Ok(())
    }

    /// The stable key this document's snapshot is filed under, when there is one.    ///
    /// A hash of the opened target's opaque handle first — that is the file the
    /// user names, and it survives a restart — then a hash of the document uuid
    /// (a never-saved document has no file yet). `None` means nothing stable to
    /// key on, so no snapshot is written: an unnamed document must not collide
    /// with another unnamed one.
    ///
    /// The key is hex because it becomes a file name on every native carrier
    /// ([`RecoveryKey::from_hex`] rejects anything else, so it cannot escape the
    /// store's directory).
    pub(crate) fn recovery_key(&self) -> Option<RecoveryKey> {
        let target = self.source_target.borrow().clone();
        if let Some(target) = target.filter(|target| !target.opaque.is_empty()) {
            return RecoveryKey::from_hex(&sha256_hex(target.opaque.as_bytes()));
        }
        let uuid = {
            let controller = self.controller.borrow();
            controller.persistence()?.document_uuid.clone()?
        };
        RecoveryKey::from_hex(&sha256_hex(format!("uuid:{uuid}").as_bytes()))
    }

    /// The effective recovery interval from the preference (ADR-0037 D12).
    ///
    /// Read per tick, so a settings write takes effect on the next tick without
    /// rescheduling anything. `0` means "off" and is returned as
    /// [`Duration::ZERO`].
    fn recovery_interval(&self) -> Duration {
        Duration::from_secs(u64::from(
            self.viewer_config
                .borrow()
                .effective()
                .ui
                .recovery
                .interval_seconds,
        ))
    }

    /// One tick of the periodic snapshot writer.
    ///
    /// Infallible by design: it runs inside the timer tick, where an error would
    /// turn a routine "the recovery directory is not writable" into a failed
    /// tick. A failure is reported **once per document per key** (see
    /// [`Self::note_recovery_failure`]) and then latched, so a read-only home
    /// directory cannot spam the status line every interval.
    ///
    /// The interval is armed by the first dirty tick and enforced per tick after
    /// it, which is what makes "nothing at t=0, one snapshot at t=<interval>"
    /// true without a background timer. `intervalSeconds = 0` disables the
    /// writes entirely and leaves no interval armed.
    pub(crate) fn tick_recovery(&self) {
        let interval = self.recovery_interval();
        if interval.is_zero() {
            // "Off" writes nothing and keeps nothing armed, so turning it back
            // on starts from a full interval rather than an inherited deadline.
            self.recovery.borrow_mut().last_attempt = None;
            return;
        }
        if !self.document_ready() {
            // The vendor pass of a newly installed document has not persisted its
            // rows yet; a snapshot now would be a baseline missing them
            // (ADR-0030 D9). Do not arm the interval until the document is ready.
            return;
        }
        if !self.controller.borrow().is_dirty() {
            // Clean: nothing to snapshot, and the next edit starts a fresh
            // interval instead of inheriting an expired one.
            self.recovery.borrow_mut().last_attempt = None;
            return;
        }
        let Some(store) = self.persistence_store() else {
            return;
        };
        let Some(archive) = self.document_archive() else {
            return;
        };
        let Some(key) = self.recovery_key() else {
            return;
        };
        let now = (self.recovery_now)();
        {
            let mut state = self.recovery.borrow_mut();
            match state.last_attempt {
                Some(at) if now.saturating_duration_since(at) < interval => return,
                Some(_) => {}
                None => {
                    // Arm the interval; the first snapshot is one interval away.
                    state.last_attempt = Some(now);
                    return;
                }
            }
            // Record the attempt before the (fallible) encode, so a failing
            // store is retried once per interval, not once per tick.
            state.last_attempt = Some(now);
        }
        let (snapshot, persistence, hint) = {
            let controller = self.controller.borrow();
            let Some(persistence) = controller.persistence().cloned() else {
                return;
            };
            let Ok(snapshot) = controller.snapshot_document() else {
                let error = CadError::InvalidInput("no document is open".into());
                self.note_recovery_failure(&key, &error);
                return;
            };
            (snapshot, persistence, controller.document_name_hint.clone())
        };
        // ADR-0040 D11: a recovery snapshot never packs the document's external
        // resources. It is written on a timer from the same snapshot the save path
        // uses, and inheriting the document's policy would re-encode up to 64 MiB
        // every interval — with no limit the native carriers enforce. The omission
        // is published (once per document) instead of being silent.
        let (snapshot, resources_omitted) = recovery_snapshot(snapshot);
        let report_omitted =
            resources_omitted && !self.recovery.borrow().resources_omitted_reported;
        if report_omitted {
            self.recovery.borrow_mut().resources_omitted_reported = true;
            // A technical status, not a catalog key: this lane does not own the
            // i18n catalog, and the stable marker is what a diagnostic keys on.
            // The line says what was left out and what still keeps the bytes.
            let text = format!(
                "{}: recovery snapshots never pack external resources; \
                 the document's own .ymodel keeps them",
                cad_domain::ymodel::markers::RECOVERY_RESOURCES_OMITTED
            );
            if let Err(error) = self.set_external_status(text) {
                self.note_post_commit_error(&error);
            }
        }
        // A recovery file is standalone: `previous: None`, because the
        // document's own archive is not this file's previous version and an
        // in-place foreign-entry copy must never read the user's file.
        let dirty = cad_db::ChangeScope::default();
        let open_revision = persistence
            .open_revision
            .map_or(u64::MAX, |revision| revision.0);
        let request = cad_platform::ArchiveWrite {
            snapshot: &snapshot,
            dirty: &dirty,
            previous: None,
            document_uuid: persistence.document_uuid.as_deref(),
            open_revision,
            // The shared host has no wall clock (its `Instant` is monotonic), so
            // the manifest carries no timestamp.
            saved_at: None,
        };
        let encoded = match archive.write(&request) {
            Ok(encoded) => encoded,
            Err(error) => {
                self.note_recovery_failure(
                    &key,
                    &CadError::Unsupported(format!("{}: {}", error.marker, error.detail)),
                );
                return;
            }
        };
        let record = RecoveryRecord {
            key: key.as_str().to_string(),
            document_uuid: persistence.document_uuid.clone(),
            // The revision the *bytes* claim, not the one requested: the record
            // must never advertise a newer revision than the archive holds.
            revision: encoded.saved_revision,
            display_name: (!hint.is_empty()).then_some(hint),
            // No clock in the shared host; the carrier may stamp its own.
            saved_at_ms: 0,
        };
        match store.save_recovery(&record, &encoded.bytes) {
            Ok(()) => self.recovery.borrow_mut().failed_key = None,
            Err(error) => self.note_recovery_failure(&key, &error),
        }
    }

    /// Discard the snapshot filed under `key` after a durable save.
    ///
    /// Returns the store's reason when it refused. The caller keeps the save a
    /// success regardless — the bytes are on disk, and a snapshot left behind can
    /// only be discarded as stale at the next open, never offered (the revision
    /// rule in [`Self::observe_recovery_on_install`]) — but it reports the
    /// refusal instead of hiding it.
    ///
    /// `key` is the key the snapshot was written under, captured **before** the
    /// save mutates `source_target`: a first save (uuid key → path key) and a
    /// Save As would otherwise look for the snapshot under the new name and leave
    /// the old one behind.
    pub(crate) fn discard_recovery(&self, key: Option<&RecoveryKey>) -> Option<String> {
        let store = self.persistence_store()?;
        let key = key?;
        match store.discard_recovery(key) {
            Ok(()) => {
                self.recovery.borrow_mut().failed_key = None;
                None
            }
            Err(error) => Some(error_text(&error)),
        }
    }

    /// Offer (or discard) an existing snapshot for the document just installed.
    ///
    /// Called once per document install, after `note_document_replaced` **and**
    /// after `source_target` names the new document (the key is derived from it).
    ///
    /// * `record.revision > saved_revision` — the snapshot holds work the file
    ///   does not: report it, naming the file to open.
    /// * `record.revision <= saved_revision` — stale; discard silently. Offering
    ///   it would be the rollback D12 forbids.
    /// * a store that cannot read it — an explicit status: there *may* be work
    ///   that cannot be read, which is never silence.
    ///
    /// A carrier with no store is silent: there is nothing to read, and calling
    /// that "unavailable" on every open of every carrier would be noise.
    pub(crate) fn observe_recovery_on_install(&self) {
        let Some(store) = self.persistence_store() else {
            return;
        };
        let Some(key) = self.recovery_key() else {
            return;
        };
        let saved_revision = {
            let controller = self.controller.borrow();
            match controller.persistence() {
                Some(persistence) => persistence.saved_revision,
                None => return,
            }
        };
        let snapshot = match store.load_recovery(&key) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.record_status("recovery.unavailable", &[("error", &error_text(&error))]);
                return;
            }
        };
        let Some(snapshot) = snapshot else {
            return;
        };
        if snapshot.record.revision > saved_revision {
            let revision = snapshot.record.revision.0.to_string();
            match store.recovery_hint(&key) {
                Some(path) => {
                    self.record_status(
                        "recovery.available",
                        &[("path", &path), ("revision", &revision)],
                    );
                }
                None => {
                    // No filesystem path (a SAF URI, a browser handle): name the
                    // snapshot rather than inventing a location for it.
                    let name = snapshot
                        .record
                        .display_name
                        .clone()
                        .unwrap_or_else(|| key.as_str().to_string());
                    self.record_status(
                        "recovery.available_no_hint",
                        &[("name", &name), ("revision", &revision)],
                    );
                }
            }
            return;
        }
        if let Err(error) = store.discard_recovery(&key) {
            self.record_status("recovery.discard_failed", &[("error", &error_text(&error))]);
        }
    }

    /// Report one snapshot failure per document, then stay quiet.
    ///
    /// The latch is keyed on the document's recovery key and cleared by a
    /// successful write, a successful discard, or a document replacement — so a
    /// repaired store reports again, while a permanently broken one costs one
    /// status line.
    fn note_recovery_failure(&self, key: &RecoveryKey, error: &CadError) {
        {
            let mut state = self.recovery.borrow_mut();
            if state.failed_key.as_ref() == Some(key) {
                return;
            }
            state.failed_key = Some(key.clone());
        }
        self.record_status("recovery.failed", &[("error", &error_text(error))]);
    }
}

/// The snapshot a recovery write stores: the document **without** its bundled
/// external resources (ADR-0040 D11).
///
/// Returns the snapshot plus whether anything was dropped, so the caller can
/// publish `recovery.resources_omitted` rather than dropping it silently. The
/// self-containment flag is cleared too: the snapshot packs nothing, and a
/// `true` there would claim otherwise (and would force a minor-5 archive for a
/// file with no resources, D12).
///
/// The mapping table is removed with the payloads: leaving its rows behind would
/// name `resources/<kind>/<key>` entries the archive no longer carries, which the
/// reader refuses as `ymodel.invalid_document`.
fn recovery_snapshot(mut snapshot: cad_db::DocumentSnapshot) -> (cad_db::DocumentSnapshot, bool) {
    let omitted = snapshot.header.embed_resources || !snapshot.resources.is_empty();
    snapshot.header.embed_resources = false;
    snapshot.resources.clear();
    snapshot.resource_names.clear();
    snapshot.resource_sources.clear();
    snapshot.tables.rows.remove(&cad_db::TableKind::Resources);
    (snapshot, omitted)
}
