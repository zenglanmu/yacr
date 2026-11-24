//! The document application-domain write path (ADR-0053 D2).
//!
//! A chrome switch is a document edit: one revision, a dirty `document.json`, and
//! a value that the next snapshot carries. An unknown token is a named refusal,
//! and re-setting the current value is a no-op (no revision, no dirty scope).

use cad_domain::ApplicationDomain;

use super::*;

fn blank() -> HostController {
    HostController::with_blank_document([800.0, 600.0]).expect("a blank document opens")
}

#[test]
fn setting_the_domain_rewrites_the_header_and_marks_it_dirty() {
    let mut controller = blank();
    assert_eq!(controller.application_domain(), None);
    assert_eq!(
        controller.effective_application_domain(),
        Some(ApplicationDomain::Cad2d),
        "an unnamed blank drawing renders as drafting"
    );

    let revision = controller.snapshot_document().expect("snapshot").revision;
    assert_eq!(
        controller.set_application_domain("bim").expect("accepted"),
        ApplicationDomain::Bim
    );
    assert_eq!(
        controller.application_domain(),
        Some(ApplicationDomain::Bim)
    );
    assert!(
        controller.dirty_scope().header,
        "the domain is a `document.json` scalar"
    );
    let snapshot = controller.snapshot_document().expect("snapshot");
    assert_eq!(
        snapshot.header.application_domain,
        Some(ApplicationDomain::Bim)
    );
    assert!(
        snapshot.revision.0 > revision.0,
        "the write is one document revision"
    );

    // Re-setting the current value is a no-op, not a second revision.
    let revision = snapshot.revision;
    controller
        .set_application_domain("bim")
        .expect("unchanged is accepted");
    assert_eq!(
        controller.snapshot_document().expect("snapshot").revision,
        revision
    );
}

#[test]
fn an_unknown_domain_token_is_refused_by_name_and_writes_nothing() {
    let mut controller = blank();
    let error = controller
        .set_application_domain("revit")
        .expect_err("an unknown token is refused");
    let text = format!("{error:?}");
    assert!(text.contains("domain.unknown"), "{text}");
    assert_eq!(controller.application_domain(), None);
    assert!(!controller.dirty_scope().header);
}
