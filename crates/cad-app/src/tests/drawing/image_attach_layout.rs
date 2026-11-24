//! `IMAGEATTACH` rotation / paper-layout / target-refusal contracts.
//!
//! Split out of the sibling `image` module so each file stays inside the
//! code-size gate. The model-space attach and clip contracts stay there; these
//! tests pin the optional `+Z` rotation basis, the active paper layout write
//! path and the named `block_space`/`unknown_layout`/`invalid_rotation`
//! refusals.

use super::*;

use super::image::{image_app, image_geometry, stored_image, undo_depth};
use crate::command_payload::ImageAttachCommand;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout};

/// A Work-mode drawing with layer `0`/`WALLS` plus the empty paper layout
/// `Sheet1` (id 1), so the image attach can target a real layout write path.
fn image_app_with_layout() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
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
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.active_layer = LayerId(7);
    (app, session)
}

fn image_attach_payload(
    file: &str,
    origin: Point3,
    pixels: [f64; 2],
    units_per_pixel: f64,
    rotation: f64,
) -> CommandPayload {
    CommandPayload::ImageAttach(Box::new(ImageAttachCommand {
        file: file.into(),
        origin,
        pixels,
        units_per_pixel,
        rotation,
    }))
}

#[test]
fn imageattach_bakes_the_optional_rotation_into_the_stored_basis() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        image_attach_payload("logo.png", p3(1.0, 2.0, 0.0), [10.0, 20.0], 2.0, 90.0),
    )
    .expect("IMAGEATTACH accepts a rotation");
    let id = EntityId(outcome.objects[0].0);
    let SemanticGeometry::Image { u, v, origin, .. } = image_geometry(&app, id) else {
        panic!("the stored row is an image");
    };
    assert_eq!(origin, p3(1.0, 2.0, 0.0), "the origin is unchanged");
    assert!(u.x.abs() < 1e-12, "u.x: {}", u.x);
    assert!((u.y - 2.0).abs() < 1e-12, "u.y: {}", u.y);
    assert!((v.x + 2.0).abs() < 1e-12, "v.x: {}", v.x);
    assert!(v.y.abs() < 1e-12, "v.y: {}", v.y);
    assert_eq!(u.z, 0.0);
    assert_eq!(v.z, 0.0);
}

#[test]
fn imageattach_targets_the_active_paper_layout_through_the_layout_write_path() {
    let (mut app, mut session) = image_app_with_layout();
    session.active_space = SpaceId::Paper(LayoutId(1));
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        image_attach_payload("logo.png", p3(3.0, 4.0, 0.0), [8.0, 6.0], 1.0, 0.0),
    )
    .expect("IMAGEATTACH writes into the active layout");
    let id = EntityId(outcome.objects[0].0);
    let entity = stored_image(&app, id);
    assert_eq!(entity.space, SpaceId::Paper(LayoutId(1)));
    assert_eq!(entity.layer, LayerId(7));
    assert_eq!(entity.draw_order, 0, "the layout has no earlier row");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .layout_entities(LayoutId(1))
            .len(),
        1,
        "the image is a real layout entity"
    );
}

#[test]
fn imageattach_refuses_a_block_space_and_an_unknown_layout_by_name() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let before = revision(&app);
    session.active_space = SpaceId::Block(cad_domain::BlockId(0));
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        image_attach_payload("logo.png", p3(0.0, 0.0, 0.0), [1.0, 1.0], 1.0, 0.0),
    )
    .expect_err("a block definition is not an attach target");
    assert!(
        format!("{error:?}").contains("imageattach.block_space"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));

    let (mut app, mut session) = image_app_with_layout();
    let before = revision(&app);
    session.active_space = SpaceId::Paper(LayoutId(9));
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        image_attach_payload("logo.png", p3(0.0, 0.0, 0.0), [1.0, 1.0], 1.0, 0.0),
    )
    .expect_err("a stale layout id is refused");
    assert!(
        format!("{error:?}").contains("imageattach.unknown_layout"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)), "no refusal wrote anything");
}

#[test]
fn imageattach_refuses_a_non_finite_rotation_before_writing() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageAttach),
        image_attach_payload(
            "logo.png",
            p3(0.0, 0.0, 0.0),
            [1.0, 1.0],
            1.0,
            f64::INFINITY,
        ),
    )
    .expect_err("a non-finite rotation is refused");
    assert!(
        format!("{error:?}").contains("imageattach.invalid_rotation"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
