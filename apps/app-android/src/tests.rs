//! Unit tests.

use super::*;

#[test]
fn demo_database_has_geometry_and_bounds() {
    let controller = HostController::with_demo_document([1080.0, 1920.0]).unwrap();
    let db = controller.drawing().unwrap();
    assert!(db.entity_count() >= 6);
    let (min, max) = db.bounds().unwrap();
    assert!(max.x - min.x > 0.0);
}

fn camera_target(controller: &Rc<RefCell<HostController>>) -> Point3 {
    controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .camera
        .target
}

fn world_per_px(controller: &Rc<RefCell<HostController>>) -> f64 {
    controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .world_per_px()
}

fn input_harness() -> (Rc<RefCell<HostController>>, AndroidViewInput) {
    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    controller.borrow_mut().fit().unwrap();
    let input = AndroidViewInput {
        controller: controller.clone(),
        handle: Rc::new(RefCell::new(None)),
        view: Rc::new(RefCell::new(None)),
        viewport: ViewportId(1),
        last: Cell::new([0.0, 0.0]),
        dragging: Cell::new(false),
        policy: RefCell::new(InputPolicy::new()),
    };
    (controller, input)
}

#[test]
fn android_view_input_drag_pans_the_authoritative_camera() {
    let (controller, input) = input_harness();
    let before = camera_target(&controller);
    input.pointer(0, 1, 100.0, 100.0); // down
    input.pointer(2, 1, 140.0, 100.0); // move 40 logical px right
    input.pointer(1, 1, 140.0, 100.0); // up
    let after = camera_target(&controller);
    // Dragging right must move the camera left so the content follows the
    // finger. This is the regression that made the Android canvas inert.
    assert!(
        after.x < before.x,
        "camera did not pan: {before:?} -> {after:?}"
    );
}

#[test]
fn android_view_input_scroll_zooms_the_camera() {
    let (controller, input) = input_harness();
    let before = world_per_px(&controller);
    input.scroll(0.0, 200.0); // scroll down => factor 0.7 => zoom in
    let after = world_per_px(&controller);
    assert!(after < before, "camera did not zoom: {before} -> {after}");
}

/// Screen pixel of a world point on the active viewport, for tap tests.
fn pixel_of(controller: &Rc<RefCell<HostController>>, world: Point3) -> [f64; 2] {
    let controller = controller.borrow();
    let viewport = controller
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap();
    viewport
        .camera
        .world_to_screen(world, viewport.logical_size)
        .unwrap()
}

#[test]
fn android_tap_selects_the_entity_under_the_finger() {
    let (controller, input) = input_harness();
    // The bottom edge of the demo room runs along world y = 0, x = 0..4000.
    let pixel = pixel_of(
        &controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    input.pointer(0, 1, pixel[0], pixel[1]); // down
    input.pointer(1, 1, pixel[0], pixel[1]); // up in place => tap

    let selection = controller.borrow().session.selection.clone();
    assert_eq!(selection.len(), 1, "a tap on geometry must select it");
    let selected = selection.refs()[0].entity;
    // The hit is a real entity of the demo drawing (never a fabricated ref).
    assert!(controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(selected)
        .is_some());
}

#[test]
fn android_tap_on_empty_space_clears_the_selection() {
    let (controller, input) = input_harness();
    // Put a real selection in place first.
    let pixel = pixel_of(
        &controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    input.pointer(0, 1, pixel[0], pixel[1]);
    input.pointer(1, 1, pixel[0], pixel[1]);
    assert_eq!(controller.borrow().session.selection.len(), 1);

    // A corner far away from every line: no hit, so the selection is cleared
    // explicitly rather than kept or faked.
    input.pointer(0, 1, 2.0, 2.0);
    input.pointer(1, 1, 2.0, 2.0);
    assert!(controller.borrow().session.selection.is_empty());
}

#[test]
fn android_drag_pans_without_selecting() {
    let (controller, input) = input_harness();
    let pixel = pixel_of(
        &controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let target_before = camera_target(&controller);
    input.pointer(0, 1, pixel[0], pixel[1]);
    input.pointer(2, 1, pixel[0] + 60.0, pixel[1]); // past the drag threshold
    input.pointer(1, 1, pixel[0] + 60.0, pixel[1]);
    // The camera moved (pan) and the drag did not become a selection.
    assert_ne!(camera_target(&controller), target_before);
    assert!(controller.borrow().session.selection.is_empty());
}

#[test]
fn android_surface_size_updates_the_viewport_and_keeps_the_camera_target() {
    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    // Move the camera off the drawing centre so a resize that recentres would
    // be caught.
    let target = Point3 {
        x: 123.0,
        y: -45.0,
        z: 0.0,
    };
    {
        let mut controller = controller.borrow_mut();
        let viewport = controller
            .application
            .workspace
            .viewports
            .get_mut(&ViewportId(1))
            .unwrap();
        viewport.camera.target = target;
    }

    apply_surface_size(&controller, [800.0, 600.0], 2.0).unwrap();
    let controller_ref = controller.borrow();
    let viewport = controller_ref
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap();
    assert_eq!(viewport.logical_size, [800.0, 600.0]);
    assert_eq!(viewport.dpi_scale, 2.0);
    // Camera centre preserved across a resize/rotation (audit U07).
    assert_eq!(viewport.camera.target, target);

    // Degenerate metrics are refused, not silently applied.
    assert!(apply_surface_size(&controller, [0.0, 600.0], 1.0).is_err());
    assert!(apply_surface_size(&controller, [800.0, 600.0], f64::NAN).is_err());
}

#[test]
fn android_panel_snapshot_is_derived_from_real_state() {
    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    controller.borrow_mut().fit().unwrap();
    let messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::ZhCn);
    let snapshot = snapshot(&controller, &messages);

    // Fresh demo: no history yet, but both flags are explicit.
    assert!(!snapshot.history.can_undo);
    assert!(!snapshot.history.can_redo);
    assert!(!snapshot.measurement.active);

    // The demo database has two real layers; the ordered ids match the rows.
    assert_eq!(snapshot.layers.rows.len(), 2);
    assert_eq!(snapshot.layer_ids, vec![LayerId(0), LayerId(1)]);

    // No selection and no annotations => explicit empty panels, no fake rows.
    assert!(snapshot.properties.rows.is_empty());
    assert_eq!(snapshot.properties.count, 0);
    assert!(snapshot.annotations.rows.is_empty());
    assert!(snapshot.annotation_ids.is_empty());
    // The demo has no paper layouts; the panel is explicitly empty.
    assert!(snapshot.layouts.rows.is_empty());
    assert!(snapshot.layout_ids.is_empty());
    assert_eq!(snapshot.layouts.empty_label, "无布局");

    // No import report yet => the drawer is empty and unverified, never clean.
    assert!(snapshot.diagnostics.rows.is_empty());
    assert!(snapshot.diagnostics.summary.contains("未验证"));
}

#[test]
fn android_import_diagnostics_surface_every_real_row() {
    use cad_import_acadrust::ImportReport;

    let messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::ZhCn);

    // No report is explicitly empty and unverified.
    let empty = import_diagnostics(None, &messages);
    assert!(empty.rows.is_empty());
    assert!(empty.summary.contains("未验证"));

    // A report with two diagnostics keeps both, with real code and object.
    let report = ImportReport {
        identity: DocumentIdentity::Temporary(0),
        capabilities: Vec::new(),
        completeness: Completeness::Partial(vec!["unknown".to_string()]),
        diagnostics: vec![
            cad_domain::Diagnostic {
                object: Some(cad_domain::ObjectId(7)),
                code: "import.linetype_unknown".into(),
                message: "线型未知".into(),
            },
            cad_domain::Diagnostic {
                object: None,
                code: "import.incomplete_stream".into(),
                message: "流不完整".into(),
            },
        ],
        parse_ms: None,
    };
    let state = import_diagnostics(Some(&report), &messages);
    assert_eq!(state.rows.len(), 2, "every diagnostic row is kept");
    assert_eq!(state.rows[0].code, "import.linetype_unknown");
    assert_eq!(state.rows[0].object, "7");
    assert_eq!(state.rows[0].description, "线型未知");
    assert_eq!(state.rows[1].object, "");
    assert_eq!(state.summary, report.completeness_label());
    assert!(state.summary.contains("部分"));
}

#[test]
fn android_pick_mapper_is_absent_without_a_surface() {
    use cad_ui_slint::CanvasPickMapper;

    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    // No UI handle means no known surface: the mapper must report no point
    // rather than fabricate one.
    let mapper = AndroidCanvasPickMapper::new(controller, Rc::new(RefCell::new(None)));
    assert!(mapper.to_world([10.0, 10.0]).is_none());
    assert!(mapper.to_world([f64::NAN, 10.0]).is_none());
}

/// A controller whose document carries one supported paper layout (`LayoutId(7)`).
///
/// The demo drawing has none, so the layout-switch routing test needs a real
/// layout the importer/representation layer accepts as drawable.
fn controller_with_one_layout() -> Rc<RefCell<HostController>> {
    use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

    let mut controller = HostController::with_demo_document([1080.0, 1920.0]).unwrap();
    let document_id = controller.document_id;
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(7),
            name: "Sheet1".into(),
            viewports: vec![PaperViewport {
                clip: vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 100.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 100.0,
                        y: 50.0,
                        z: 0.0,
                    },
                ],
                model_to_paper: Transform3::scale(100.0),
                completeness: Completeness::Complete,
            }],
        })
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    Rc::new(RefCell::new(controller))
}

/// A `HostSink` with no UI handle/view: enough to exercise the command path.
fn host_sink(controller: &Rc<RefCell<HostController>>) -> HostSink {
    HostSink {
        controller: controller.clone(),
        handle: Rc::new(RefCell::new(None)),
        view: Rc::new(RefCell::new(None)),
        incoming: Rc::new(RefCell::new(controller.borrow().drawing())),
        configuration: AndroidHostConfiguration::default(),
    }
}

#[test]
fn android_snapshot_carries_an_empty_highlight_for_an_empty_selection() {
    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    let messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::ZhCn);
    let snapshot = snapshot(&controller, &messages);
    // No selection => explicit empty highlight; no tool => no preview overlay.
    assert!(snapshot.selection.is_empty());
    assert!(snapshot.measurement_preview.is_none());
    assert!(snapshot.annotation_preview.is_none());
}

#[test]
fn android_snapshot_carries_the_real_selection_as_a_highlight() {
    let (controller, _input) = input_harness();
    // Select a real demo entity through the documented command path.
    let entity = controller
        .borrow()
        .drawing()
        .unwrap()
        .entities()
        .next()
        .expect("demo drawing has entities")
        .id;
    let document = controller.borrow().document_id;
    controller
        .borrow_mut()
        .set_selection(vec![SelectionRef {
            document,
            entity,
            instance: InstancePath::default(),
            sub_element: None,
        }])
        .unwrap();

    let messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::ZhCn);
    let snapshot = snapshot(&controller, &messages);
    // The highlight mirrors `HostController::selection()` exactly, and reflects
    // the non-empty selection (not the empty default).
    assert_eq!(snapshot.selection, *controller.borrow().selection());
    assert_eq!(snapshot.selection.len(), 1);
    assert_eq!(snapshot.selection.refs()[0].entity, entity);
    // A selection is not a tool: no preview appears just because something is
    // selected.
    assert!(snapshot.measurement_preview.is_none());
    assert!(snapshot.annotation_preview.is_none());
}

#[test]
fn android_layout_selection_routes_through_switch_space() {
    // The adapter has no `LayoutSwitchSink` installed, so `on_layout_selected`
    // emits `CommandId::SwitchSpace`. This asserts the host's command path
    // executes it and re-syncs the space-derived state.
    let controller = controller_with_one_layout();
    let mut sink = host_sink(&controller);

    let command = Command {
        schema_version: 1,
        id: CommandId::SwitchSpace,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload: CommandPayload::Space(SpaceId::Paper(LayoutId(7))),
    };
    sink.send(command).unwrap();

    // The authoritative session records the switch...
    assert_eq!(
        controller.borrow().session.active_space,
        SpaceId::Paper(LayoutId(7))
    );
    // ...and the layout panel derivation now reports it active (not model space,
    // not a fabricated row).
    let messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::ZhCn);
    let paper = snapshot(&controller, &messages);
    assert_eq!(paper.layout_ids, vec![LayoutId(7)]);
    assert_eq!(paper.layouts.active_index, Some(0));

    // Switching back to model space goes through the same path.
    sink.send(Command {
        schema_version: 1,
        id: CommandId::SwitchSpace,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload: CommandPayload::Space(SpaceId::Model),
    })
    .unwrap();
    assert_eq!(controller.borrow().session.active_space, SpaceId::Model);
    assert_eq!(snapshot(&controller, &messages).layouts.active_index, None);
}

#[test]
fn android_layout_switch_to_an_unknown_layout_is_refused() {
    // The command path validates against the real layout table: a bogus layout is
    // an explicit failure that keeps the current space, not a silent success.
    let controller = controller_with_one_layout();
    let mut sink = host_sink(&controller);
    sink.send(Command {
        schema_version: 1,
        id: CommandId::SwitchSpace,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload: CommandPayload::Space(SpaceId::Paper(LayoutId(99))),
    })
    .unwrap();
    assert_eq!(controller.borrow().session.active_space, SpaceId::Model);
}
