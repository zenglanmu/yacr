//! The POINT display style sysvars at the **host** boundary: a real SAVE→OPEN
//! cycle.
//!
//! The command layer's contracts live in `cad_app::tests::point_display_style`
//! and the text grammar in `cad_ui_core`'s `point_style` session tests. This
//! module pins the one claim that needs the real save funnel: a `PDMODE`/`PDSIZE`
//! write produced by the command line is **written by the DXF exporter** and read
//! back by the importer, so the value survives a save/reopen instead of arriving
//! only with the source drawing.
//!
//! The evidence level is our writer plus **our** reader (internal consistency); it
//! is not a vendor, AutoCAD or third-party compatibility claim.

use super::*;

#[test]
fn a_pdmode_pdsize_write_survives_a_real_save_and_reopen() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));

    core.command(
        CommandId::SetPointDisplayMode,
        CommandPayload::PointDisplayMode(35),
    )
    .expect("PDMODE is a document write");
    core.command(
        CommandId::SetPointDisplaySize,
        CommandPayload::PointDisplaySize(-20.0),
    )
    .expect("PDSIZE is a document write");
    let saved = core.controller.borrow().drawing().unwrap();
    assert_eq!(saved.point_display_mode(), 35);
    assert_eq!(saved.point_display_size(), -20.0);

    // The real save gesture: the platform writes the exported DXF bytes.
    core.request_save().expect("the synchronous save completes");
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    let bytes = written[0].1.clone();
    drop(written);

    // Reopen the bytes the save really wrote, through the same import funnel.
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "point-style.dxf")
        .expect("our own export re-imports");
    let reopened = core.controller.borrow().drawing().unwrap();
    assert_eq!(
        reopened.point_display_mode(),
        35,
        "the mode is exported and read back (writer + our reader)"
    );
    assert_eq!(
        reopened.point_display_size(),
        -20.0,
        "the negative viewport-percentage size survives"
    );
}

#[test]
fn the_style_write_bumps_the_revision_and_records_no_undo_step() {
    let core = core();
    let document = core.controller.borrow().document_id;
    let before = core.controller.borrow().drawing().unwrap().revision();
    core.command(
        CommandId::SetPointDisplayMode,
        CommandPayload::PointDisplayMode(3),
    )
    .expect("PDMODE is a document write");
    let after = core.controller.borrow().drawing().unwrap().revision();
    assert_eq!(
        after.0,
        before.0 + 1,
        "the revision bump is the display refresh signal"
    );
    let c = core.controller.borrow();
    assert!(
        !c.application.can_undo(&document),
        "there is no header patch kind, so no undo step is recorded"
    );
    assert!(!c.application.can_redo(&document));
}
