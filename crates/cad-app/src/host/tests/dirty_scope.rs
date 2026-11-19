//! The host's accumulated commit scope (ADR-0037 D8/D12 Phase 2).
//!
//! Every commit's [`cad_db::ChangeScope`] must reach the one place a later save
//! reads it from, and a durable save must clear it. These contracts cover the
//! two controller wrappers (`execute`, `execute_traced`) and `mark_saved`; the
//! engine request funnel's separate accumulation is covered by `cad-host`'s
//! `.ymodel` contracts.

use super::*;

fn blank() -> HostController {
    HostController::with_blank_document([800.0, 600.0]).expect("a blank document opens")
}

#[test]
fn execute_accumulates_the_commit_scope() {
    let mut controller = blank();
    assert!(
        controller.dirty_scope().is_empty(),
        "a new document starts clean"
    );
    let outcome = controller
        .create_line(p(0.0, 0.0), p(10.0, 0.0))
        .expect("the line commits");
    let committed = outcome
        .changes
        .as_ref()
        .expect("a committed edit publishes its change set")
        .scope
        .clone();
    assert!(!committed.is_empty(), "the edit is persistable");
    assert_eq!(
        controller.dirty_scope(),
        &committed,
        "the controller accumulated exactly the published scope"
    );
}

#[test]
fn execute_traced_accumulates_the_commit_scope_once() {
    let mut controller = blank();
    let (document, viewport) = (controller.document_id, controller.viewport_id);
    let traced = controller
        .execute_traced(Command {
            schema_version: 1,
            id: crate::CommandId::CreateLine,
            document,
            viewport,
            payload: crate::CommandPayload::Points(vec![p(0.0, 0.0), p(2.0, 0.0)]),
        })
        .expect("the traced command runs");
    let outer = traced
        .result
        .as_ref()
        .expect("the command succeeded")
        .changes
        .as_ref()
        .expect("a committed edit publishes its change set")
        .scope
        .clone();
    assert!(!outer.is_empty());
    // The same transaction appears in the command trace and the outer result;
    // merging it twice is a union, so the accumulated set is unchanged.
    assert_eq!(
        controller.dirty_scope(),
        &outer,
        "double counting the same commit must not grow the scope"
    );
}

#[test]
fn mark_saved_clears_the_accumulated_scope() {
    let mut controller = blank();
    let outcome = controller
        .create_line(p(0.0, 1.0), p(1.0, 1.0))
        .expect("the line commits");
    assert!(!controller.dirty_scope().is_empty());

    // Only a durable save may call this; the scope is part of that same fact.
    let saved = outcome
        .changes
        .as_ref()
        .map(|changes| changes.after)
        .unwrap_or(controller.drawing().expect("drawing").revision());
    controller.mark_saved(saved, None, Vec::new());
    assert!(
        controller.dirty_scope().is_empty(),
        "a durable save clears the accumulated scope"
    );

    // The next edit accumulates again from empty.
    controller
        .create_line(p(0.0, 2.0), p(2.0, 2.0))
        .expect("the second line commits");
    assert!(!controller.dirty_scope().is_empty());
}

#[test]
fn note_change_scope_unions_across_callers() {
    let mut controller = blank();
    let mut first = cad_db::ChangeScope::default();
    first.table(cad_db::TableKind::Layers);
    let mut second = cad_db::ChangeScope::default();
    second.header();
    controller.note_change_scope(&first);
    controller.note_change_scope(&second);
    assert!(controller
        .dirty_scope()
        .tables
        .contains(&cad_db::TableKind::Layers));
    assert!(controller.dirty_scope().header);
}
