//! The open-pipeline readiness gate (ADR-0030 D9): SAVE/EXPORT/PLOT refuse by
//! name until the vendor pass has settled, without turning `loading` into a
//! second readiness state.

use super::*;

#[test]
fn a_write_before_the_open_pipeline_settles_is_refused_by_name() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    assert!(
        core.document_ready(),
        "a published document starts ready until a vendor pass begins"
    );

    core.set_document_ready(false);
    assert!(!core.document_ready());
    assert!(
        !core.loading(),
        "readiness is not `loading`: NEW/OPEN stay available"
    );

    // SAVE, EXPORT and PLOT all refuse with the stable marker before any chooser.
    let error = core
        .request_save()
        .expect_err("SAVE refuses while not ready");
    assert!(
        matches!(&error, CadError::Unsupported(reason) if reason == "save.document_not_ready"),
        "{error:?}"
    );
    let (key, args) = {
        let status = core.last_status().expect("SAVE grades a status");
        let (key, args) = catalog(&status);
        (key.to_string(), args)
    };
    assert_eq!(key, "save.export_failed");
    assert_eq!(
        args.get("reason").map(String::as_str),
        Some("save.document_not_ready")
    );

    let error = core
        .request_export()
        .expect_err("EXPORT refuses while not ready");
    assert!(
        matches!(&error, CadError::Unsupported(reason) if reason == "save.document_not_ready"),
        "{error:?}"
    );
    let (key, _) = {
        let status = core.last_status().expect("EXPORT grades a status");
        let (key, args) = catalog(&status);
        (key.to_string(), args)
    };
    assert_eq!(key, "save.export_failed");

    let error = core
        .request_plot()
        .expect_err("PLOT refuses while not ready");
    assert!(
        matches!(&error, CadError::Unsupported(reason) if reason == "save.document_not_ready"),
        "{error:?}"
    );
    let (key, args) = {
        let status = core.last_status().expect("PLOT grades a status");
        let (key, args) = catalog(&status);
        (key.to_string(), args)
    };
    assert_eq!(key, "plot.export_failed");
    assert_eq!(
        args.get("reason").map(String::as_str),
        Some("save.document_not_ready")
    );

    assert!(
        files.written.borrow().is_empty(),
        "a refused write writes nothing"
    );
    assert!(
        files.events.borrow().is_empty(),
        "no save chooser was ever opened"
    );

    // Once the pipeline settles the funnels proceed exactly as before.
    core.set_document_ready(true);
    assert!(core.document_ready());
    core.request_export().expect("EXPORT proceeds once ready");
    assert_eq!(
        files.written.borrow().len(),
        1,
        "the settled export writes its file"
    );
}
