//! `IMAGEADJUST` contract tests: the real per-image display-transfer write.
//!
//! The command sets brightness/contrast/fade/monochrome on exactly one selected
//! raster image in one transaction / one undo step; every refusal happens before
//! that transaction, so nothing is written, no revision is bumped and no history
//! entry appears. `RESET` restores the neutral 50/50/0/colour transfer and an
//! identical set is the `imageadjust.unchanged` no-op. The display path is
//! checked end to end (entity -> primitive -> `ImageBatch`); the GPU pixel
//! contract lives in `cad-render-wgpu` (software-GPU `headless_render`).

use super::image::{attach_default, image_app, image_geometry, undo_depth};
use super::*;
use crate::command_payload::ImageAdjustCommand;

/// `IMAGEADJUST` payload: the typed form (`monochrome = None` keeps the state).
fn adjust_payload(
    brightness: u8,
    contrast: u8,
    fade: u8,
    monochrome: Option<bool>,
) -> CommandPayload {
    CommandPayload::ImageAdjust(Box::new(ImageAdjustCommand::Set {
        brightness,
        contrast,
        fade,
        monochrome,
    }))
}

/// `IMAGEADJUST RESET` payload.
fn adjust_reset_payload() -> CommandPayload {
    CommandPayload::ImageAdjust(Box::new(ImageAdjustCommand::Reset))
}

/// The stored display transfer of one image entity.
fn image_adjust_of(app: &Application, id: EntityId) -> ImageAdjust {
    match image_geometry(app, id) {
        SemanticGeometry::Image { adjust, .. } => adjust,
        other => panic!("expected an image, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// IMAGEADJUST: the real display-transfer write.
// ---------------------------------------------------------------------------

/// Build the real scene for the drawing with one decoded texture, so the image
/// primitive and its `ImageBatch` can be inspected. The renderer's own pixel
/// contract lives in `cad-render-wgpu` (software-GPU `headless_render`).
fn built_image_batches(app: &Application, key: &str) -> Vec<cad_scene::ImageBatch> {
    let drawing = Arc::clone(&app.workspace.documents[&DocumentId(1)].drawing);
    let mut images = cad_resources::DecodedImageCache::new();
    images
        .insert(
            cad_resources::ResourceKey(key.into()),
            cad_resources::DecodedImage {
                width: 1,
                height: 1,
                rgba: Arc::from(vec![255u8, 255, 255, 255]),
            },
            &cad_resources::ResourceLimits::default(),
        )
        .expect("insert the decoded texture");
    let delta = crate::render_scene::build_scene_with_space_and_images(
        &drawing,
        TaskStamp::new(DocumentId(1), 0),
        None,
        Some(Arc::new(images)),
        &crate::layers::LayerOverrideSet::new(),
        cad_representation::SpaceSelection::Model,
    )
    .expect("build the real scene");
    delta.images
}

#[test]
fn imageadjust_writes_the_exact_transfer_in_one_undo_step() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "textures/logo.png", [8.0, 8.0], 1.0);
    session.selection.replace([ref_of(id.0)]);
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, Some(true)),
    )
    .unwrap();

    let expected = ImageAdjust {
        brightness: 30,
        contrast: 70,
        fade: 20,
        monochrome: true,
    };
    assert_eq!(
        image_adjust_of(&app, id),
        expected,
        "the exact stored transfer"
    );
    let changes = outcome.changes.as_ref().expect("one change set");
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one entity patch");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    assert_eq!(outcome.objects, vec![ObjectId(id.0)]);
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "imageadjust.applied")
        .unwrap_or_else(|| panic!("no imageadjust.applied in {:?}", outcome.diagnostics));
    for needle in ["brightness 30", "contrast 70", "fade 20", "monochrome"] {
        assert!(
            applied.message.contains(needle),
            "{needle:?} missing from {:?}",
            applied.message
        );
    }

    // The display path carries the same value: the provider puts it on the
    // primitive and the scene cache copies it onto the `ImageBatch`.
    let batches = built_image_batches(&app, "textures/logo.png");
    assert_eq!(batches.len(), 1, "one textured image batch");
    assert_eq!(
        batches[0].adjust, expected,
        "the batch carries the transfer"
    );
}

#[test]
fn imageadjust_undo_and_redo_restore_the_transfer() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [8.0, 8.0], 1.0);
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, Some(true)),
    )
    .unwrap();
    assert_eq!(image_adjust_of(&app, id).brightness, 30);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        image_adjust_of(&app, id),
        ImageAdjust::default(),
        "UNDO restores the neutral transfer"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        image_adjust_of(&app, id),
        ImageAdjust {
            brightness: 30,
            contrast: 70,
            fade: 20,
            monochrome: true,
        },
        "REDO reapplies it"
    );
}

#[test]
fn imageadjust_omitted_word_keeps_monochrome_and_reset_restores_neutral() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [8.0, 8.0], 1.0);
    session.selection.replace([ref_of(id.0)]);

    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(40, 60, 10, Some(true)),
    )
    .unwrap();
    // No MONO/COLOR word: the stored `monochrome` survives the new percentages.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(45, 55, 5, None),
    )
    .unwrap();
    assert_eq!(
        image_adjust_of(&app, id),
        ImageAdjust {
            brightness: 45,
            contrast: 55,
            fade: 5,
            monochrome: true,
        }
    );

    // COLOR clears it explicitly.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(45, 55, 5, Some(false)),
    )
    .unwrap();
    assert!(!image_adjust_of(&app, id).monochrome);

    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_reset_payload(),
    )
    .unwrap();
    assert_eq!(image_adjust_of(&app, id), ImageAdjust::default());
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), depth + 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageadjust.applied"));

    // RESET again on the neutral transfer is the unchanged no-op.
    let unchanged = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_reset_payload(),
    )
    .unwrap();
    assert!(
        unchanged.changes.is_none(),
        "identical values open no transaction"
    );
    assert_eq!(undo_depth(&app), depth + 1);
    assert!(unchanged
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageadjust.unchanged"));
}

#[test]
fn imageadjust_identical_values_are_a_no_op() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [8.0, 8.0], 1.0);
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, Some(true)),
    )
    .unwrap();
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, Some(true)),
    )
    .unwrap();
    assert!(outcome.changes.is_none(), "no transaction");
    assert!(outcome.objects.is_empty(), "no change set objects");
    assert_eq!(revision(&app), before, "no revision bump");
    assert_eq!(undo_depth(&app), depth, "no undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageadjust.unchanged"));
}

#[test]
fn imageadjust_refusals_write_nothing() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let first = attach_default(&mut app, &mut session, "a.png", [8.0, 8.0], 1.0);
    let second = attach_default(&mut app, &mut session, "b.png", [8.0, 8.0], 1.0);

    // An out-of-range percentage is refused before the selection is read.
    session.selection.clear();
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(150, 50, 0, None),
    )
    .expect_err("150 is out of range");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageadjust.invalid_value")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);

    // No selection.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, None),
    )
    .expect_err("no selection is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageadjust.no_selection")),
        "{error:?}"
    );

    // Two selected images.
    session
        .selection
        .replace([ref_of(first.0), ref_of(second.0)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, None),
    )
    .expect_err("two selected images are refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageadjust.multiple")),
        "{error:?}"
    );

    // One selected non-image.
    let line = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
    )
    .unwrap();
    let line_id = EntityId(line.objects[0].0);
    session.selection.replace([ref_of(line_id.0)]);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, None),
    )
    .expect_err("a non-image is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageadjust.not_image")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before, "no refusal writes");
    assert_eq!(undo_depth(&app), depth, "no refusal records undo");
    assert_eq!(
        image_adjust_of(&app, first),
        ImageAdjust::default(),
        "the image keeps its neutral transfer"
    );
}

#[test]
fn imageadjust_is_work_only() {
    let (mut app, mut session) = image_app(AppMode::Viewer);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAdjust),
        adjust_payload(30, 70, 20, None),
    )
    .expect_err("a Viewer session may not write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert!(ExtendedCommandId::ImageAdjust.requires_work_mode());
    assert!(ExtendedCommandId::DashImageAdjust.requires_work_mode());
}
