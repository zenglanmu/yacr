//! ADR-0036 phase C2 host contracts: the boolean poll's **status** arms.
//!
//! The end-to-end poll (worker, commit, staleness, cancel) is covered by
//! `cad-app`'s host contracts, which own `HostController::poll_solid_boolean`.
//! This file pins the `cad-host` half — what `HostCore` publishes for each poll
//! answer — through [`HostCore::apply_solid_boolean_poll`], the same function the
//! tick calls after the controller poll.
//!
//! Synthetic contract over `HostCore` (no window, no GPU); not visual evidence.

use super::*;
use cad_app::host::SolidBooleanPoll;
use cad_app::tasks::SOLID_BOOLEAN_WORKER_PANICKED;

fn revision(core: &HostCore) -> cad_domain::Revision {
    core.controller
        .borrow()
        .drawing()
        .expect("a document is open")
        .revision()
}

fn entity_count(core: &HostCore) -> usize {
    core.controller
        .borrow()
        .drawing()
        .expect("a document is open")
        .entities()
        .count()
}

fn catalog_key(core: &HostCore) -> Option<String> {
    match core.last_status() {
        Some(StatusMessage::Catalog { key, .. }) => Some(key),
        _ => None,
    }
}

fn error_argument(core: &HostCore) -> Option<String> {
    match core.last_status() {
        Some(StatusMessage::Catalog { key, args }) if key == "solid.boolean_failed" => args
            .iter()
            .find(|argument| argument.key == "error")
            .map(|argument| argument.value.clone()),
        _ => None,
    }
}

#[test]
fn a_stale_boolean_is_an_explicit_status_not_a_silent_drop() {
    let core = core();
    let revision_before = revision(&core);
    let count_before = entity_count(&core);

    core.apply_solid_boolean_poll(SolidBooleanPoll::Stale)
        .expect("stale grades a status");

    assert_eq!(catalog_key(&core).as_deref(), Some("solid.boolean_stale"));
    assert_eq!(revision(&core), revision_before, "the drawing is unchanged");
    assert_eq!(
        entity_count(&core),
        count_before,
        "the drawing is unchanged"
    );
}

#[test]
fn idle_and_running_publish_no_status() {
    let core = core();
    let status_before = core.last_status();
    core.apply_solid_boolean_poll(SolidBooleanPoll::Idle)
        .expect("idle is not an error");
    core.apply_solid_boolean_poll(SolidBooleanPoll::Running)
        .expect("running is not an error");
    assert_eq!(
        core.last_status(),
        status_before,
        "an in-flight/idle poll must not overwrite the last real status"
    );
}

#[test]
fn a_worker_panic_is_a_failure_never_a_cancel() {
    let core = core();
    let revision_before = revision(&core);
    let count_before = entity_count(&core);

    // The task seam maps a panicked worker to this stable Invariant; the host
    // must publish the generic code only (never the panic payload, never
    // `solid.boolean_cancelled`).
    let error = CadError::Invariant(format!(
        "{SOLID_BOOLEAN_WORKER_PANICKED}: the worker stopped; payload: secret-internal-text"
    ));
    core.apply_solid_boolean_poll(SolidBooleanPoll::Failed { error })
        .expect("a failure grades a status");

    assert_eq!(catalog_key(&core).as_deref(), Some("solid.boolean_failed"));
    let message = error_argument(&core).expect("a failed status carries the reason");
    assert_eq!(
        message, "the solid boolean worker stopped unexpectedly; the drawing is unchanged",
        "the host publishes a generic panic sentence"
    );
    assert!(
        !message.contains("secret-internal-text"),
        "the panic payload is never surfaced"
    );
    assert_eq!(revision(&core), revision_before, "the drawing is unchanged");
    assert_eq!(
        entity_count(&core),
        count_before,
        "the drawing is unchanged"
    );
}

#[test]
fn a_real_boolean_failure_keeps_its_named_reason() {
    let core = core();
    core.apply_solid_boolean_poll(SolidBooleanPoll::Failed {
        error: CadError::Unsupported(
            "solid.lift_requires_planar: the operand has a curved face".into(),
        ),
    })
    .expect("a failure grades a status");

    assert_eq!(catalog_key(&core).as_deref(), Some("solid.boolean_failed"));
    assert_eq!(
        error_argument(&core).as_deref(),
        Some("solid.lift_requires_planar: the operand has a curved face")
    );
}
