//! `IMAGE` / `-IMAGE` / `IMAGEATTACH` / `IMAGECLIP` contract tests.
//!
//! The report is read-only (no transaction, revision, history or `ChangeSet`) and
//! Viewer-legal; attach inserts one real `SemanticGeometry::Image` through the
//! shared create funnel; clip rewrites one selected image's pixel-space clip in
//! one transaction / one undo step. Every refusal is checked to leave the
//! revision and undo stack untouched.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_history::History;

use crate::command_payload::{ImageAttachCommand, ImageClipCommand};

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(History::undo_depth)
        .unwrap_or(0)
}

/// A work- or viewer-mode drawing with layer `0` plus `WALLS`, `WALLS` current.
fn image_app(mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([77u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), mode);
    session.active_layer = LayerId(7);
    (app, session)
}

/// The one stored `SemanticGeometry::Image` row.
fn stored_image(app: &Application, id: EntityId) -> cad_db::DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .cloned()
        .expect("the stored image")
}

fn attach_payload(
    file: &str,
    origin: Point3,
    pixels: [f64; 2],
    units_per_pixel: f64,
) -> CommandPayload {
    CommandPayload::ImageAttach(Box::new(ImageAttachCommand {
        file: file.into(),
        origin,
        pixels,
        units_per_pixel,
    }))
}

fn clip_payload(rect: Option<[f64; 4]>) -> CommandPayload {
    CommandPayload::ImageClip(Box::new(ImageClipCommand { rect }))
}

fn attach_default(
    app: &mut Application,
    session: &mut SessionState,
    file: &str,
    pixels: [f64; 2],
    units_per_pixel: f64,
) -> EntityId {
    let outcome = execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        attach_payload(file, p3(10.0, 20.0, 0.0), pixels, units_per_pixel),
    )
    .expect("IMAGEATTACH creates one image");
    assert_eq!(outcome.objects.len(), 1);
    EntityId(outcome.objects[0].0)
}

fn image_geometry(app: &Application, id: EntityId) -> SemanticGeometry {
    stored_image(app, id).geometry
}

#[test]
fn imageattach_builds_the_exact_image_entity_in_one_undo_step() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        attach_payload(
            "Textures/Logo.PNG",
            p3(10.0, 20.0, 0.0),
            [640.0, 480.0],
            0.05,
        ),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one image entity");
    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    let id = EntityId(outcome.objects[0].0);
    let entity = stored_image(&app, id);
    let stored = entity.clone();
    assert_eq!(entity.layer, LayerId(7), "attached on the active layer");
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbRasterImage");
    match entity.geometry {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
        } => {
            assert_eq!(origin, p3(10.0, 20.0, 0.0));
            assert_eq!(u, p3(0.05, 0.0, 0.0), "u is the per-pixel edge vector");
            assert_eq!(v, p3(0.0, 0.05, 0.0), "v is the per-pixel edge vector");
            assert_eq!(pixels, [640.0, 480.0]);
            assert_eq!(
                file.as_deref(),
                Some("textures/logo.png"),
                "the stored key is normalised (relative, lower-case), never the raw path"
            );
            assert!(clip.is_none());
            assert!(visible);
        }
        other => panic!("expected an image, got {other:?}"),
    }

    // The created diagnostic names the file, pixel size and extent.
    let created = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "imageattach.created")
        .expect("imageattach.created");
    assert!(
        created.message.contains("textures/logo.png"),
        "{}",
        created.message
    );
    assert!(created.message.contains("640 x 480"), "{}", created.message);
    assert!(created.message.contains("32 x 24"), "{}", created.message);

    // UNDO removes it and REDO restores the byte-identical row.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(id)
            .is_none(),
        "undo removes the image"
    );
    assert_eq!(undo_depth(&app), 0);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(id)
            .unwrap(),
        &stored,
        "redo restores the exact entity"
    );
}

#[test]
fn imageattach_refusals_write_nothing() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let before = revision(&app);
    for (payload, code) in [
        (
            attach_payload("/etc/passwd", point(0.0, 0.0), [1.0, 1.0], 1.0),
            "imageattach.unsafe_reference",
        ),
        (
            attach_payload("../secret.png", point(0.0, 0.0), [1.0, 1.0], 1.0),
            "imageattach.unsafe_reference",
        ),
        (
            attach_payload("https://host/logo.png", point(0.0, 0.0), [1.0, 1.0], 1.0),
            "imageattach.unsafe_reference",
        ),
        (
            attach_payload(
                "logo.png",
                Point3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
                [1.0, 1.0],
                1.0,
            ),
            "imageattach.invalid_origin",
        ),
        (
            attach_payload("logo.png", point(0.0, 0.0), [0.0, 1.0], 1.0),
            "imageattach.invalid_pixels",
        ),
        (
            attach_payload("logo.png", point(0.0, 0.0), [1.0, -2.0], 1.0),
            "imageattach.invalid_pixels",
        ),
        (
            attach_payload("logo.png", point(0.0, 0.0), [1.0, 1.0], 0.0),
            "imageattach.invalid_units_per_pixel",
        ),
        (
            attach_payload("logo.png", point(0.0, 0.0), [1.0, 1.0], f64::INFINITY),
            "imageattach.invalid_units_per_pixel",
        ),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::ImageAttach),
            payload,
        )
        .expect_err("the refusal never creates an image");
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before, "a refusal bumps no revision");
        assert_eq!(undo_depth(&app), 0, "a refusal records no undo entry");
    }
}

#[test]
fn imageclip_sets_a_two_vertex_pixel_space_clip_in_one_undo_step() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    session.selection.replace([ref_of(id.0)]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 320.0, 240.0])),
    )
    .unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(id.0)]);
    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.after.0, before.0 + 1);
    assert_eq!(changes.changes.len(), 1);
    let depth = undo_depth(&app);
    assert_eq!(depth, 2, "attach + clip = two undo steps");
    match image_geometry(&app, id) {
        SemanticGeometry::Image { clip, .. } => assert_eq!(
            clip,
            Some(ImageClip {
                vertices: vec![[0.0, 0.0], [320.0, 240.0]],
                inside: true,
            })
        ),
        other => panic!("expected an image, got {other:?}"),
    }
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "imageclip.applied")
        .expect("imageclip.applied");
    assert!(applied.message.contains("320"), "{}", applied.message);

    // UNDO restores the previous (no) clip.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match image_geometry(&app, id) {
        SemanticGeometry::Image { clip, .. } => assert!(clip.is_none()),
        other => panic!("expected an image, got {other:?}"),
    }

    // REDO re-applies the same clip.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    match image_geometry(&app, id) {
        SemanticGeometry::Image { clip, .. } => assert_eq!(
            clip,
            Some(ImageClip {
                vertices: vec![[0.0, 0.0], [320.0, 240.0]],
                inside: true,
            })
        ),
        other => panic!("expected an image, got {other:?}"),
    }
}

#[test]
fn imageclip_off_clears_the_clip_and_an_identical_clip_is_unchanged() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 320.0, 240.0])),
    )
    .unwrap();

    // The identical clip writes nothing and opens no transaction.
    let before = revision(&app);
    let depth = undo_depth(&app);
    let unchanged = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 320.0, 240.0])),
    )
    .unwrap();
    assert!(unchanged.changes.is_none(), "no transaction");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert!(unchanged
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageclip.unchanged"));

    // OFF clears it in one transaction / one undo step.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(None),
    )
    .unwrap();
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), depth + 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageclip.removed"));
    match image_geometry(&app, id) {
        SemanticGeometry::Image { clip, .. } => assert!(clip.is_none()),
        other => panic!("expected an image, got {other:?}"),
    }

    // OFF again is the identical (empty) clip: unchanged, no transaction.
    let unchanged = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(None),
    )
    .unwrap();
    assert!(unchanged.changes.is_none());
    assert_eq!(undo_depth(&app), depth + 1);
}

#[test]
fn imageclip_refusals_write_nothing() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let first = attach_default(&mut app, &mut session, "a.png", [10.0, 10.0], 1.0);
    let second = attach_default(&mut app, &mut session, "b.png", [10.0, 10.0], 1.0);
    let before = revision(&app);
    let depth = undo_depth(&app);

    // No selection.
    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 5.0, 5.0])),
    )
    .expect_err("no selection is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageclip.no_selection")),
        "{error:?}"
    );

    // Two selected images.
    session
        .selection
        .replace([ref_of(first.0), ref_of(second.0)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 5.0, 5.0])),
    )
    .expect_err("two selected images are refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageclip.multiple")),
        "{error:?}"
    );

    assert_eq!(revision(&app), before, "no selection/multiple writes");
    assert_eq!(undo_depth(&app), depth, "no undo step is recorded");

    // One selected non-image: insert a LINE through the real create funnel and
    // select it.
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
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 5.0, 5.0])),
    )
    .expect_err("a non-image is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageclip.not_image")),
        "{error:?}"
    );

    // Inverted / degenerate rectangles on a valid selected image.
    session.selection.replace([ref_of(first.0)]);
    for rect in [
        [10.0, 0.0, 0.0, 10.0],
        [5.0, 0.0, 5.0, 10.0],
        [0.0, 5.0, 10.0, 5.0],
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::ImageClip),
            clip_payload(Some(rect)),
        )
        .expect_err("an invalid rectangle is refused");
        assert!(
            matches!(&error, CadError::InvalidInput(message)
                if message.starts_with("imageclip.invalid_rect")),
            "{error:?}"
        );
    }

    assert_eq!(revision(&app), before, "no refusal writes");
    assert_eq!(undo_depth(&app), depth, "no undo step is recorded");
}

#[test]
fn image_report_lists_model_and_paper_images_and_writes_nothing() {
    // One model-space image through the real command, one paper-space image in
    // the fixture, so both spaces are covered.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(90),
            type_key: "AcDbRasterImage".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(90),
        layer: LayerId(0),
        space: SpaceId::Paper(LayoutId(1)),
        geometry: SemanticGeometry::Image {
            origin: p3(0.0, 0.0, 0.0),
            u: p3(1.0, 0.0, 0.0),
            v: p3(0.0, 1.0, 0.0),
            pixels: [100.0, 50.0],
            file: Some("sheet.png".into()),
            clip: None,
            visible: false,
        },
        draw_order: 0,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([78u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.active_layer = LayerId(0);
    let id = attach_default(&mut app, &mut session, "model.png", [640.0, 480.0], 0.5);
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 320.0, 240.0])),
    )
    .unwrap();

    // The report is Viewer-legal: switch mode and prove nothing moves.
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Image),
        CommandPayload::None,
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);

    let entries: Vec<&Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "image.entry")
        .collect();
    assert_eq!(entries.len(), 2, "one entry per stored image");
    let model_entry = entries
        .iter()
        .find(|entry| entry.message.contains("model.png"))
        .expect("the model-space image is listed");
    assert!(
        model_entry.message.contains("Model"),
        "{}",
        model_entry.message
    );
    assert!(
        model_entry.message.contains("pixels 640 x 480"),
        "{}",
        model_entry.message
    );
    assert!(
        model_entry.message.contains("visible true"),
        "{}",
        model_entry.message
    );
    assert!(
        model_entry.message.contains("2-vertex rectangle (inside)"),
        "{}",
        model_entry.message
    );
    let paper_entry = entries
        .iter()
        .find(|entry| entry.message.contains("sheet.png"))
        .expect("the paper-space image is listed");
    assert!(
        paper_entry.message.contains("Paper(1)"),
        "{}",
        paper_entry.message
    );
    assert!(
        paper_entry.message.contains("visible false"),
        "{}",
        paper_entry.message
    );
    assert!(
        paper_entry.message.contains("clip none"),
        "{}",
        paper_entry.message
    );
    let summary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "image.report")
        .expect("image.report");
    assert!(
        summary.message.contains("2 raster image(s)"),
        "{}",
        summary.message
    );

    // `-IMAGE` is the same report with the dash codes.
    let dash = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::DashImage),
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        dash.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "-image.entry")
            .count(),
        2
    );
    assert!(dash
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "-image.report"));
}

#[test]
fn empty_image_report_is_a_successful_image_none() {
    let (mut app, mut session) = image_app(AppMode::Viewer);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Image),
        CommandPayload::None,
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "image.none");
}

#[test]
fn an_image_survives_move_with_its_non_geometric_state_intact() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(Some([0.0, 0.0, 100.0, 100.0])),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(id.0)],
            delta: p3(5.0, -2.0, 0.0),
        },
    )
    .unwrap();
    match image_geometry(&app, id) {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
        } => {
            assert_eq!(origin, p3(15.0, 18.0, 0.0), "origin translates as a point");
            assert_eq!(u, p3(0.05, 0.0, 0.0), "one pixel's width is unchanged");
            assert_eq!(v, p3(0.0, 0.05, 0.0), "one pixel's height is unchanged");
            assert_eq!(pixels, [640.0, 480.0]);
            assert_eq!(file.as_deref(), Some("logo.png"));
            assert_eq!(
                clip,
                Some(ImageClip {
                    vertices: vec![[0.0, 0.0], [100.0, 100.0]],
                    inside: true,
                }),
                "the pixel-space clip is not geometry and must survive"
            );
            assert!(visible);
        }
        other => panic!("expected an image, got {other:?}"),
    }
}
