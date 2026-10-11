//! Unit tests.
//!
//! The host-specific adapters (the `InputPolicy` pointer state machine, the
//! sample-path file probe, the surface slots) are exercised on the host target;
//! the asynchronous-open funnel is Android-only (the Slint poll timer exists
//! there) and is compiled but not run on the host.

use super::*;

use std::sync::Arc;

use cad_app::host::HostController;
use cad_host::{HostCore, HostOptions, HostPlatform};
use cad_platform::{ConfigStore, FilePurpose, HostCapabilities, PlatformFiles};
use cad_ui_slint::{MessageSource, ViewInput};

/// A core over the Android platform impls, with the demo surface applied.
///
/// The handle slot stays empty (no Slint window in a unit test); `metrics` maps
/// the surface without a handle, and a command's push failure is ignored, so the
/// authoritative controller state is still observable.
fn test_core(files: Rc<AndroidFiles>) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(AndroidConfigStore),
        surface: Rc::new(AndroidSurface),
        resources: Rc::new(AndroidResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: true,
            plot: false,
            import_worker: true,
        },
    };
    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: DEMO_LOGICAL_SIZE,
        locale: "zh-CN".into(),
    };
    let core = HostCore::new(options, Rc::new(RefCell::new(None)), platform).unwrap();
    crate::platform::set_surface_slot(DEMO_LOGICAL_SIZE, 1.0);
    core
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

fn input_harness() -> (HostCore, AndroidViewInput) {
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    let viewport = core.controller.borrow().viewport_id;
    let input = AndroidViewInput::new(core.clone(), viewport);
    (core, input)
}

#[test]
fn demo_database_has_geometry_and_bounds() {
    let controller = HostController::with_demo_document([1080.0, 1920.0]).unwrap();
    let db = controller.drawing().unwrap();
    assert!(db.entity_count() >= 6);
    let (min, max) = db.bounds().unwrap();
    assert!(max.x - min.x > 0.0);
}

#[test]
fn android_view_input_drag_pans_the_authoritative_camera() {
    let (core, input) = input_harness();
    let before = camera_target(&core.controller);
    input.pointer(0, 1, 100.0, 100.0); // down
    input.pointer(2, 1, 140.0, 100.0); // move 40 logical px right
    input.pointer(1, 1, 140.0, 100.0); // up
    let after = camera_target(&core.controller);
    // Dragging right must move the camera left so the content follows the
    // finger. This is the regression that made the Android canvas inert.
    assert!(
        after.x < before.x,
        "camera did not pan: {before:?} -> {after:?}"
    );
}

/// Scroll direction follows the shared host formula
/// (`factor = (1 - dy * 0.0015).clamp(0.2, 5.0)`, identical on web/Linux) and
/// `Camera::zoom_at`'s contract that `factor > 1` zooms in: scrolling down
/// (positive dy) yields `factor < 1` = zoom out, scrolling up = zoom in.
#[test]
fn android_view_input_scroll_zooms_the_camera() {
    let (core, input) = input_harness();
    let before = world_per_px(&core.controller);
    input.scroll(0.0, 200.0); // scroll down => factor 0.7 < 1 => zoom out
    let after_down = world_per_px(&core.controller);
    assert!(
        after_down > before,
        "scroll down must zoom out: {before} -> {after_down}"
    );
    input.scroll(0.0, -200.0); // scroll up => factor 1.3 > 1 => zoom in
    let after_up = world_per_px(&core.controller);
    assert!(
        after_up < after_down,
        "scroll up must zoom in: {after_down} -> {after_up}"
    );
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
    let (core, input) = input_harness();
    // The bottom edge of the demo room runs along world y = 0, x = 0..4000.
    let pixel = pixel_of(
        &core.controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    input.pointer(0, 1, pixel[0], pixel[1]); // down
    input.pointer(1, 1, pixel[0], pixel[1]); // up in place => tap

    let selection = core.controller.borrow().session.selection.clone();
    assert_eq!(selection.len(), 1, "a tap on geometry must select it");
    let selected = selection.refs()[0].entity;
    // The hit is a real entity of the demo drawing (never a fabricated ref).
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(selected)
        .is_some());
}

#[test]
fn android_tap_on_empty_space_clears_the_selection() {
    let (core, input) = input_harness();
    // Put a real selection in place first.
    let pixel = pixel_of(
        &core.controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    input.pointer(0, 1, pixel[0], pixel[1]);
    input.pointer(1, 1, pixel[0], pixel[1]);
    assert_eq!(core.controller.borrow().session.selection.len(), 1);

    // A corner far away from every line: no hit, so the selection is cleared
    // explicitly rather than kept or faked.
    input.pointer(0, 1, 2.0, 2.0);
    input.pointer(1, 1, 2.0, 2.0);
    assert!(core.controller.borrow().session.selection.is_empty());
}

#[test]
fn android_drag_pans_without_selecting() {
    let (core, input) = input_harness();
    let pixel = pixel_of(
        &core.controller,
        Point3 {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let target_before = camera_target(&core.controller);
    input.pointer(0, 1, pixel[0], pixel[1]);
    input.pointer(2, 1, pixel[0] + 60.0, pixel[1]); // past the drag threshold
    input.pointer(1, 1, pixel[0] + 60.0, pixel[1]);
    // The camera moved (pan) and the drag did not become a selection.
    assert_ne!(camera_target(&core.controller), target_before);
    assert!(core.controller.borrow().session.selection.is_empty());
}

#[test]
fn android_files_probe_queues_opened_bytes() {
    let dir = std::env::temp_dir().join(format!("yacr-android-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("yacr-sample.dwg");
    std::fs::write(&path, [1u8, 2, 3, 4]).unwrap();
    let files = AndroidFiles::new(vec![path.to_string_lossy().to_string()]);

    files.begin_open().unwrap();
    let events = files.poll();
    assert_eq!(events.len(), 1);
    match &events[0] {
        cad_platform::FileEvent::Opened { target, bytes } => {
            assert_eq!(target.display_name, path.to_string_lossy().to_string());
            assert_eq!(&bytes[..], &[1, 2, 3, 4]);
        }
        other => panic!("expected an opened event, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn android_files_missing_sample_is_an_explicit_failure() {
    let files = AndroidFiles::new(vec!["/definitely/not/a/drawing.dwg".to_string()]);
    files.begin_open().unwrap();
    let events = files.poll();
    assert_eq!(events.len(), 1);
    match &events[0] {
        cad_platform::FileEvent::Failed { purpose, reason } => {
            assert_eq!(purpose, &cad_platform::FilePurpose::Open);
            assert_eq!(
                reason,
                &MessageSource::from_request("zh-CN").text("android.no_sample", &[])
            );
        }
        other => panic!("expected an explicit failure, got {other:?}"),
    }
}

#[test]
fn host_fallback_save_is_explicit_unsupported() {
    // Off the Android target there is no writer: save is an explicit refusal.
    let files = AndroidFiles::new(Vec::new());
    assert!(matches!(
        files.begin_save(cad_platform::SavePurpose::SaveDxf, "drawing.dxf"),
        Err(CadError::Unsupported(_))
    ));
    assert!(matches!(
        files.write_atomic(&cad_platform::FileTarget::from_path("/tmp/x.dxf"), b"x"),
        Err(CadError::Unsupported(_))
    ));
}

/// The pure SAF outcome→event mapping (the part of `drain_saf` that does not
/// need JNI) is unit-tested here on the host target.
#[test]
fn saf_event_maps_pending_cancel_fail_open_and_save() {
    use crate::platform::{saf_event, SafResult};

    assert!(saf_event(FilePurpose::Open, SafResult::Pending).is_none());

    assert!(matches!(
        saf_event(FilePurpose::Open, SafResult::Cancelled),
        Some(cad_platform::FileEvent::Cancelled {
            purpose: FilePurpose::Open
        })
    ));

    assert!(matches!(
        saf_event(FilePurpose::Open, SafResult::Failed("boom".into())),
        Some(cad_platform::FileEvent::Failed {
            purpose: FilePurpose::Open,
            ..
        })
    ));

    match saf_event(
        FilePurpose::Open,
        SafResult::Opened {
            uri: "content://doc/1".into(),
            name: "plan.dxf".into(),
            bytes: vec![1, 2, 3],
        },
    ) {
        Some(cad_platform::FileEvent::Opened { target, bytes }) => {
            assert_eq!(target.display_name, "plan.dxf");
            assert_eq!(target.opaque, "content://doc/1");
            assert_eq!(&bytes[..], &[1, 2, 3]);
        }
        other => panic!("expected an opened event, got {other:?}"),
    }

    match saf_event(
        FilePurpose::Save(cad_platform::SavePurpose::SaveDxf),
        SafResult::SaveTarget {
            uri: "content://doc/2".into(),
            name: "out.dxf".into(),
        },
    ) {
        Some(cad_platform::FileEvent::SaveTarget { purpose, target }) => {
            assert_eq!(purpose, cad_platform::SavePurpose::SaveDxf);
            assert_eq!(target.display_name, "out.dxf");
            assert_eq!(target.opaque, "content://doc/2");
        }
        other => panic!("expected a save-target event, got {other:?}"),
    }
}

/// A result whose kind contradicts the armed purpose is an explicit failure, not
/// a fabricated open/save (an open-chosen URI must never be written as a DXF).
#[test]
fn saf_event_rejects_a_cross_wired_result() {
    use crate::platform::{saf_event, SafResult};

    assert!(matches!(
        saf_event(
            FilePurpose::Open,
            SafResult::SaveTarget {
                uri: "content://doc/3".into(),
                name: "x.dxf".into()
            }
        ),
        Some(cad_platform::FileEvent::Failed {
            purpose: FilePurpose::Open,
            ..
        })
    ));
    assert!(matches!(
        saf_event(
            FilePurpose::Save(cad_platform::SavePurpose::SaveDxf),
            SafResult::Opened {
                uri: "content://doc/4".into(),
                name: "y.dxf".into(),
                bytes: vec![9]
            }
        ),
        Some(cad_platform::FileEvent::Failed {
            purpose: FilePurpose::Save(_),
            ..
        })
    ));
}

#[test]
fn android_config_store_has_no_durable_store() {
    let store = AndroidConfigStore;
    assert_eq!(store.load_config(), cad_platform::ConfigLoad::Absent);
    assert_eq!(store.load_preference(), cad_platform::ConfigLoad::Absent);
    assert!(matches!(
        store.persist_preference("{}"),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn android_surface_reports_the_forwarded_metrics() {
    use cad_platform::SurfaceSource;
    crate::platform::set_surface_slot([800.0, 600.0], 2.0);
    crate::platform::set_safe_insets([24.0, 8.0, 16.0, 4.0]);
    let surface = AndroidSurface.surface().expect("a real surface");
    assert_eq!(surface.logical, [800.0, 600.0]);
    assert_eq!(surface.scale, 2.0);
    assert_eq!(surface.safe_insets, [24.0, 8.0, 16.0, 4.0]);
}

#[test]
fn android_resource_sections_leave_proxy_and_images_unpushed() {
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    // No font or import report yet: the open document's references source is the
    // only real one (empty keys -> the explicit "none" row). Proxy is not
    // surfaced by the importer, and Android does not model images.
    let sections = resource_sections(&core.controller.borrow(), None);
    assert!(sections.fonts.is_none());
    assert!(sections.import.is_none());
    assert!(sections.proxy.is_none());
    assert_eq!(
        sections.references.as_ref().unwrap().keys,
        Vec::<String>::new()
    );
    assert!(!sections.images_modeled);

    let report = cad_platform::fonts::FontLoadReport {
        catalog_entries: 3,
        requested: vec!["simplex".into()],
        planned: vec!["simplex.shx".into()],
        registered: vec!["simplex".into()],
        failed: Vec::new(),
        unresolved: Vec::new(),
        default_face: Some("osifont".into()),
    };
    let sections = resource_sections(&core.controller.borrow(), Some(&report));
    let fonts = sections.fonts.expect("a real font report is pushed");
    assert_eq!(fonts.catalog_entries, 3);
    assert_eq!(fonts.requested, 1);
    assert_eq!(fonts.registered, 1);
    assert_eq!(fonts.default_face.as_deref(), Some("osifont"));
}

#[test]
fn android_surface_size_updates_the_viewport_and_keeps_the_camera_target() {
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    // Move the camera off the drawing centre so a resize that recentres would
    // be caught.
    let target = Point3 {
        x: 123.0,
        y: -45.0,
        z: 0.0,
    };
    {
        let mut controller = core.controller.borrow_mut();
        let viewport = controller
            .application
            .workspace
            .viewports
            .get_mut(&ViewportId(1))
            .unwrap();
        viewport.camera.target = target;
    }
    install_runtime(core.clone());

    set_surface_size(800.0, 600.0, 2.0).unwrap();
    let controller_ref = core.controller.borrow();
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
    assert!(set_surface_size(0.0, 600.0, 1.0).is_err());
    assert!(set_surface_size(800.0, 600.0, f64::NAN).is_err());
}

#[test]
fn android_safe_insets_shrink_the_canvas_and_degenerate_input_is_refused() {
    // Safe insets are the system-bar/soft-keyboard margins; the canvas rect is
    // the surface minus the safe area (audit U07). Zeros are the honest default
    // until a platform source provides them.
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    install_runtime(core.clone());

    set_surface_size(800.0, 600.0, 2.0).unwrap();
    // [top, right, bottom, left] = 24, 8, 16, 4 -> 788 x 560.
    set_surface_insets(24.0, 8.0, 16.0, 4.0).unwrap();
    let logical_size = core
        .controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .logical_size;
    assert_eq!(logical_size, [800.0 - 8.0 - 4.0, 600.0 - 24.0 - 16.0]);

    // Degenerate insets are refused and the applied size is left unchanged.
    assert!(set_surface_insets(-1.0, 0.0, 0.0, 0.0).is_err());
    assert!(set_surface_insets(f64::NAN, 0.0, 0.0, 0.0).is_err());
    let unchanged = core
        .controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .logical_size;
    assert_eq!(unchanged, logical_size);
}

#[test]
fn android_surface_entry_point_applies_after_runtime_install() {
    // The exported Activity entry point reaches the live host through the
    // registered runtime and applies the same resize; before `start` (no
    // runtime on this thread) it is an explicit error, never a silent no-op.
    assert!(set_surface_size(800.0, 600.0, 2.0).is_err());

    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    install_runtime(core.clone());
    set_surface_size(800.0, 600.0, 2.0).unwrap();
    assert_eq!(
        core.controller
            .borrow()
            .application
            .workspace
            .viewports
            .get(&ViewportId(1))
            .unwrap()
            .logical_size,
        [800.0, 600.0]
    );
    // Non-finite input is refused before touching the viewport.
    assert!(set_surface_size(f64::NAN, 600.0, 2.0).is_err());
}

/// A controller whose document carries one supported paper layout (`LayoutId(7)`).
///
/// The demo drawing has none, so the layout-switch routing test needs a real
/// layout the importer/representation layer accepts as drawable.
fn controller_with_one_layout() -> HostController {
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
                    // The legacy three-point clip is two *opposite* rectangle
                    // corners plus the model view centre; adjacent corners leave
                    // the paper height unrecoverable and the layout undrawable.
                    Point3 {
                        x: 100.0,
                        y: 50.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 50.0,
                        y: 25.0,
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
    controller
}

fn core_with_one_layout() -> HostCore {
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    *core.controller.borrow_mut() = controller_with_one_layout();
    core
}

#[test]
fn android_layout_selection_routes_through_switch_space() {
    // The host installs `AndroidSink` as the adapter's `LayoutSwitchSink`; a
    // layout selection is dispatched as `CommandId::SwitchSpace` through the same
    // command funnel. This asserts the sink executes it.
    let core = core_with_one_layout();
    let mut sink = AndroidSink { core: core.clone() };

    cad_ui_slint::LayoutSwitchSink::select(
        &mut sink,
        cad_representation::SpaceSelection::Paper(LayoutId(7)),
    );
    assert_eq!(
        core.controller.borrow().session.active_space,
        SpaceId::Paper(LayoutId(7))
    );

    // Switching back to model space goes through the same path.
    cad_ui_slint::LayoutSwitchSink::select(&mut sink, cad_representation::SpaceSelection::Model);
    assert_eq!(
        core.controller.borrow().session.active_space,
        SpaceId::Model
    );
}

#[test]
fn android_layout_switch_to_an_unknown_layout_is_refused() {
    // The sink dispatches through the command path, which validates against the
    // real layout table: a bogus layout is an explicit failure that keeps the
    // current space, not a silent success.
    let core = core_with_one_layout();
    let mut sink = AndroidSink { core: core.clone() };
    cad_ui_slint::LayoutSwitchSink::select(
        &mut sink,
        cad_representation::SpaceSelection::Paper(LayoutId(99)),
    );
    assert_eq!(
        core.controller.borrow().session.active_space,
        SpaceId::Model
    );
}

// --- Async open (F01): poll → publish-once, cancel, progress panel -------------

/// A synthetic, writer-produced AC1032 DWG with four LINE entities.
///
/// The same committed contract fixture `cad-app` uses (`fixtures/manifest`); a
/// real byte stream through the single importer, not a mock.
///
/// Android-only: the Slint poll timer exists only there, so these tests are
/// gated with it rather than breaking the host test build.
#[cfg(target_os = "android")]
fn synthetic_dwg_bytes() -> Vec<u8> {
    include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg").to_vec()
}

/// A temp sample path holding `bytes`, for the Android sample-path probe.
#[cfg(target_os = "android")]
fn write_sample(name: &str, bytes: &[u8]) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("yacr-android-async-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    (path.clone(), path.to_string_lossy().to_string())
}

/// Open the sample through the shared funnel and poll to a terminal.
#[cfg(target_os = "android")]
fn open_and_poll(core: &HostCore) {
    let _ = core.command(
        cad_app::CommandId::OpenDrawing,
        cad_app::CommandPayload::None,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while core.loading() {
        assert!(
            std::time::Instant::now() < deadline,
            "import worker did not finish"
        );
        let _ = core.tick();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn open_with_no_sample_surfaces_the_failure_not_completed() {
    // Host-target: the Android probe resolves synchronously, so the failure must
    // be graded within the gesture — never overwritten by the generic "completed"
    // and never left as a silent success.
    let core = test_core(Rc::new(AndroidFiles::new(Vec::new())));
    let demo_id = core.controller.borrow().drawing().unwrap().id();
    let (document, viewport) = {
        let c = core.controller.borrow();
        (c.document_id, c.viewport_id)
    };
    let mut sink = AndroidSink { core: core.clone() };
    // The headless test core has no shell handle, so the shared `push()` fails
    // and `execute` returns that error even though the command succeeded; the
    // graded status is what this test asserts.
    let _ = sink.send(Command {
        schema_version: 1,
        id: CommandId::OpenDrawing,
        document,
        viewport,
        payload: cad_app::CommandPayload::None,
    });

    let expected = core.message(
        "command.failed",
        &[("error", &core.message("android.no_sample", &[]))],
    );
    assert_eq!(
        core.last_status().as_deref(),
        Some(expected.as_str()),
        "the open failure must be the surfaced status"
    );
    assert_ne!(
        core.last_status().unwrap(),
        core.message("command.completed", &[]),
        "a failed open must not report completion"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().id(),
        demo_id,
        "the demo document must be retained"
    );
    assert!(!core.loading(), "no open may be left pending");
}

#[cfg(target_os = "android")]
#[test]
fn android_async_open_publishes_once_and_leaves_no_running_job() {
    let (dir, sample) = write_sample("synthetic.dwg", &synthetic_dwg_bytes());
    let core = test_core(Rc::new(AndroidFiles::new(vec![sample])));
    open_and_poll(&core);
    assert_eq!(
        core.controller.borrow().drawing().unwrap().entity_count(),
        4
    );
    // Publishing happened exactly once: the terminal snapshot is not running.
    let terminal = core.controller.borrow().async_open_snapshot().unwrap();
    assert!(!terminal.running);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "android")]
#[test]
fn android_cancel_command_keeps_the_document() {
    let (dir, sample) = write_sample("cancelled.dwg", &synthetic_dwg_bytes());
    let core = test_core(Rc::new(AndroidFiles::new(vec![sample])));
    let demo_id = core.controller.borrow().drawing().unwrap().id();
    // Start the background open, then cancel through the ordinary command path;
    // the host must not touch the document.
    let _ = core.command(
        cad_app::CommandId::OpenDrawing,
        cad_app::CommandPayload::None,
    );
    let _ = core.command(
        cad_app::CommandId::CancelLoading,
        cad_app::CommandPayload::None,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while core.loading() {
        assert!(std::time::Instant::now() < deadline);
        let _ = core.tick();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(core.controller.borrow().drawing().unwrap().id(), demo_id);
    let _ = std::fs::remove_dir_all(&dir);
}
