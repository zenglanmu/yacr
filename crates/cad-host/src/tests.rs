//! cad-host unit tests: the open-cancel discard + progress seam (F1) and the
//! platform-target round-trip (F2), driven through a fake `PlatformFiles`.
//!
//! These exercise the orchestration core directly (no Slint handle, no GPU), so
//! they are platform-general and do not stand in for real host acceptance.
use super::*;
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, PlatformFiles, SavePurpose, SurfaceMetrics,
    SurfaceSource,
};

/// A fake platform whose file events the test enqueues directly.
#[derive(Default)]
struct FakeFiles {
    events: RefCell<Vec<FileEvent>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for FakeFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Ok(())
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

struct FakeConfig;
impl ConfigStore for FakeConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, _text: &str) -> CadResult<()> {
        Ok(())
    }
}

struct FakeSurface;
impl SurfaceSource for FakeSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        Some(SurfaceMetrics {
            logical: [800.0, 600.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        })
    }
}

fn core_with_caps(files: Rc<FakeFiles>, capabilities: HostCapabilities) -> HostCore {
    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [800.0, 600.0],
        locale: "zh-CN".into(),
    };
    let platform = HostPlatform {
        files: files as Rc<dyn PlatformFiles>,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities,
    };
    HostCore::new(options, Rc::new(RefCell::new(None)), platform).unwrap()
}

fn core_with(files: Rc<FakeFiles>, import_worker: bool) -> HostCore {
    core_with_caps(
        files,
        HostCapabilities {
            open: true,
            new: true,
            save: true,
            plot: true,
            import_worker,
        },
    )
}

fn p(x: f64, y: f64) -> cad_domain::Point3 {
    cad_domain::Point3 { x, y, z: 0.0 }
}

fn select_entity(core: &HostCore, entity: cad_domain::EntityId) {
    let document = core.controller.borrow().document_id;
    core.controller
        .borrow_mut()
        .session
        .selection
        .replace([cad_app::selection::model_ref(document, entity)]);
}

fn line_endpoints(
    core: &HostCore,
    entity: cad_domain::EntityId,
) -> (cad_domain::Point3, cad_domain::Point3) {
    let drawing = core.controller.borrow().drawing().unwrap();
    match &drawing.entity(entity).unwrap().geometry {
        cad_domain::SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected LINE geometry, got {other:?}"),
    }
}

fn add_two_lines(core: &HostCore) -> (cad_domain::EntityId, cad_domain::EntityId) {
    let mut controller = core.controller.borrow_mut();
    let first = controller.create_line(p(0.0, 0.0), p(1.0, 0.0)).unwrap();
    let second = controller.create_line(p(0.0, 10.0), p(1.0, 10.0)).unwrap();
    (
        cad_domain::EntityId(first.objects[0].0),
        cad_domain::EntityId(second.objects[0].0),
    )
}

fn run_controller_command(core: &HostCore, id: CommandId, payload: CommandPayload) {
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    controller
        .execute(Command {
            schema_version: 1,
            id,
            document,
            viewport,
            payload,
        })
        .unwrap();
}

#[test]
fn move_uses_tool_start_selection_and_history_round_trips() {
    use cad_ui_slint::DrawCommandSink;

    let core = core_with(Rc::new(FakeFiles::default()), false);
    let (a, b) = add_two_lines(&core);
    select_entity(&core, a);
    let original_a = line_endpoints(&core, a);
    let original_b = line_endpoints(&core, b);

    let mut sink = core.clone();
    sink.begin(cad_app::DrawToolKind::Move);
    select_entity(&core, b);
    let command = core.move_command(p(5.0, 2.0)).unwrap();
    core.controller.borrow_mut().execute(command).unwrap();

    assert_eq!(line_endpoints(&core, a), (p(5.0, 2.0), p(6.0, 2.0)));
    assert_eq!(line_endpoints(&core, b), original_b);

    run_controller_command(&core, CommandId::Undo, CommandPayload::None);
    assert_eq!(line_endpoints(&core, a), original_a);
    assert_eq!(line_endpoints(&core, b), original_b);
    run_controller_command(&core, CommandId::Redo, CommandPayload::None);
    assert_eq!(line_endpoints(&core, a), (p(5.0, 2.0), p(6.0, 2.0)));
    assert_eq!(line_endpoints(&core, b), original_b);
}

#[test]
fn move_selection_snapshot_resets_on_cancel_and_tool_restart() {
    use cad_ui_slint::DrawCommandSink;

    let core = core_with(Rc::new(FakeFiles::default()), false);
    let (a, b) = add_two_lines(&core);
    let original_a = line_endpoints(&core, a);
    let original_b = line_endpoints(&core, b);
    let mut sink = core.clone();

    select_entity(&core, a);
    sink.begin(cad_app::DrawToolKind::Move);
    select_entity(&core, b);
    sink.begin(cad_app::DrawToolKind::Move); // restarting captures the new selection
    let command = core.move_command(p(0.0, 3.0)).unwrap();
    core.controller.borrow_mut().execute(command).unwrap();
    assert_eq!(line_endpoints(&core, a), original_a);
    assert_eq!(line_endpoints(&core, b), (p(0.0, 13.0), p(1.0, 13.0)));

    run_controller_command(&core, CommandId::Undo, CommandPayload::None);
    assert_eq!(line_endpoints(&core, b), original_b);
    select_entity(&core, a);
    sink.begin(cad_app::DrawToolKind::Move);
    sink.cancel();
    assert!(matches!(
        sink.commit(cad_app::DrawIntent::Move { delta: p(1.0, 0.0) }),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(line_endpoints(&core, a), original_a);

    select_entity(&core, b);
    sink.begin(cad_app::DrawToolKind::Move);
    let command = core.move_command(p(1.0, 0.0)).unwrap();
    core.controller.borrow_mut().execute(command).unwrap();
    assert_eq!(line_endpoints(&core, a), original_a);
    assert_eq!(line_endpoints(&core, b), (p(1.0, 10.0), p(2.0, 10.0)));
}

/// A fake platform that synthesizes the save target synchronously, the way the
/// browser host does (no native dialog).
#[derive(Default)]
struct SyncSaveFiles {
    events: RefCell<Vec<FileEvent>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
}

impl PlatformFiles for SyncSaveFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.events.borrow_mut().push(FileEvent::SaveTarget {
            purpose,
            target: FileTarget {
                display_name: default_name.to_string(),
                opaque: format!("download://{default_name}"),
            },
        });
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

fn core_with_sync_save(files: Rc<SyncSaveFiles>) -> HostCore {
    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [800.0, 600.0],
        locale: "zh-CN".into(),
    };
    let platform = HostPlatform {
        files: files as Rc<dyn PlatformFiles>,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: true,
            plot: true,
            import_worker: false,
        },
    };
    HostCore::new(options, Rc::new(RefCell::new(None)), platform).unwrap()
}

#[test]
fn a_synchronous_save_completes_inside_the_gesture() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_sync_save(files.clone());
    // Save As DXF: the browser synthesizes the target in `begin_save`, so the
    // export must run in the gesture rather than wait for a timer tick.
    core.request_save().unwrap();
    assert!(
        !core.save_pending.get(),
        "the synchronous target is drained"
    );
    assert!(
        core.pending_save.borrow().is_none(),
        "a completed save leaves no pending interaction"
    );
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    assert!(
        written[0].0.ends_with(".dxf"),
        "save names a .dxf target: {}",
        written[0].0
    );
    assert!(
        written[0].1.windows(7).any(|w| w == b"SECTION"),
        "the written save is an ASCII DXF document"
    );
    let status = core.last_status().expect("the save grades a status");
    assert!(
        status.contains(&written[0].0),
        "the graded status names the exported target: {status}"
    );
    drop(written);

    // Plot on a document with no paper layout: the funnel still completes in the
    // gesture and grades the explicit no-layout failure — never the picker-waiting
    // text that an async platform would show.
    core.request_plot().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("plot grades a status");
    let waiting = core.message("plot.picker_waiting", &[]);
    assert_ne!(
        status, waiting,
        "a synchronous plot must not report picker-waiting"
    );
    assert!(
        status.starts_with(&core.message("plot.export_failed", &[("reason", "")]))
            && status.contains(&core.message("plot.no_layout", &[])),
        "a synchronous plot grades the real export failure: {status}"
    );
}

#[test]
fn a_synchronous_plot_writes_real_vector_content() {
    // A committed fixture with one paper-space layout, so the vector plot has
    // real stroke geometry. Opening is synchronous on this fake platform.
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg");
    let bytes = std::fs::read(&fixture).expect("committed plot fixture");
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_sync_save(files.clone());
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "a4-layout.dwg")
        .unwrap();
    core.request_plot().unwrap();
    let written = files.written.borrow();
    assert_eq!(written.len(), 1);
    assert!(
        written[0].0.ends_with(".svg"),
        "the default plot format is svg: {}",
        written[0].0
    );
    assert!(
        written[0].1.windows(4).any(|w| w == b"<svg"),
        "the written plot is a real SVG document, not an empty file"
    );
    // A real document with stroke geometry, not just an empty prolog.
    let has = |needle: &[u8]| written[0].1.windows(needle.len()).any(|w| w == needle);
    assert!(
        has(b"<polyline") || has(b"<polygon") || has(b"<path"),
        "the written plot contains real stroke geometry, not just a prolog"
    );
}

#[test]
fn an_async_save_waits_for_the_picker_then_exports_on_the_tick() {
    // `FakeFiles::begin_save` is a no-op (a desktop dialog is asynchronous): the
    // core must leave the interaction pending and only export once the platform
    // delivers the chosen target on a later drain.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.request_save().unwrap();
    assert!(
        core.save_pending.get(),
        "an async picker leaves the interaction pending"
    );
    assert!(core.pending_save.borrow().is_none());
    // The platform delivers the chosen target; the core's own drain + save poll
    // performs the export on the next tick.
    files.events.borrow_mut().push(FileEvent::SaveTarget {
        purpose: SavePurpose::SaveDxf,
        target: FileTarget::from_path("/tmp/chosen.dxf"),
    });
    core.drain_files().unwrap();
    core.poll_pending_save().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("the export grades a status");
    assert!(
        status.contains("/tmp/chosen.dxf"),
        "the graded status names the chosen target: {status}"
    );
}

#[test]
fn a_clone_locale_switch_reaches_other_clones_text() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files, false);
    // WebSink / WebInput / the layout sink each hold a clone; a runtime locale
    // switch must reach the shared-catalog text produced by every clone.
    let other = core.clone();
    assert_eq!(other.message("command.completed", &[]), "命令完成");
    core.set_locale("en");
    assert_eq!(
        other.message("command.completed", &[]),
        "Operation completed",
        "set_locale on one clone must change another clone's text"
    );
}

#[test]
fn unbacked_new_plot_save_are_refused_without_side_effects() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with_caps(
        files,
        HostCapabilities {
            open: true,
            new: false,
            save: false,
            plot: false,
            import_worker: false,
        },
    );
    // New must not replace the drawing.
    let before = core.controller.borrow().drawing();
    assert!(matches!(
        core.request_new().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(
        Arc::ptr_eq(
            &before.unwrap(),
            &core.controller.borrow().drawing().unwrap()
        ),
        "an unbacked New must not replace the document"
    );
    // Plot/Save must not queue a target that will never be exported.
    assert!(matches!(
        core.request_plot().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(matches!(
        core.request_save().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(core.pending_save.borrow().is_none());
    assert!(!core.save_pending.get());
}

#[test]
fn error_text_unwraps_unsupported_and_not_implemented() {
    assert_eq!(
        error_text(&CadError::Unsupported(
            "OFFSET supports LINE and CIRCLE only".into()
        )),
        "OFFSET supports LINE and CIRCLE only"
    );
    assert_eq!(
        error_text(&CadError::NotImplemented("web save")),
        "web save"
    );
    // Other variants keep their Display (Debug) form.
    assert_eq!(
        error_text(&CadError::InvalidInput("bad".into())),
        "InvalidInput(\"bad\")"
    );
}

#[test]
fn cancelled_open_is_discarded_and_progress_is_cancellable_then_idle() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    // Start an open attempt exactly as `request_open` does before the platform
    // read begins.
    core.begin_open_attempt();
    core.open_pending.set(true);

    // While the platform read runs, the drawer shows a running, cancellable job.
    let running = core.import_snapshot().expect("running snapshot");
    assert!(running.running && running.cancellable);

    // Cancel: the read is still shown, but the cancel affordance is now disabled.
    core.cancel_loading();
    let cancelled = core.import_snapshot().expect("running snapshot");
    assert!(cancelled.running && !cancelled.cancellable);

    // The read completes *after* the cancel: the bytes must be discarded.
    let drawing_before = core.controller.borrow().drawing();
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/after-cancel.dwg"),
        bytes: Arc::from(vec![1u8, 2, 3, 4]),
    });
    core.drain_files().unwrap();

    assert!(
        !core.open_pending.get(),
        "the delivery clears the pending read"
    );
    assert_eq!(
        core.import_snapshot(),
        None,
        "nothing is pending once the cancelled read is discarded"
    );
    let drawing_after = core.controller.borrow().drawing();
    assert!(
        Arc::ptr_eq(&drawing_before.unwrap(), &drawing_after.unwrap()),
        "a cancelled open must not replace the document"
    );
    assert_eq!(
        files.source.borrow().as_ref(),
        None,
        "a cancelled open must not arm the source-overwrite guard"
    );
}

#[test]
fn delivered_open_retains_the_platform_target_until_publish() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    core.begin_open_attempt();
    core.open_pending.set(true);
    let target = FileTarget {
        display_name: "drawing.dwg".into(),
        opaque: "saf://handle/42".into(),
    };
    files.events.borrow_mut().push(FileEvent::Opened {
        target: target.clone(),
        bytes: Arc::from(vec![1u8, 2, 3]),
    });
    core.drain_files().unwrap();
    // The worker path retains the platform's own target (its opaque handle), not
    // a path re-derived from the display name, so the publish step arms the
    // source guard against the right handle.
    assert_eq!(core.open_target.borrow().as_ref(), Some(&target));
}

#[test]
fn failed_async_publish_keeps_the_current_document() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let demo_id = core.controller.borrow().drawing().unwrap().id();
    core.begin_open_attempt();
    core.open_pending.set(true);
    // Invalid bytes: the background import must fail and publish nothing.
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        bytes: Arc::from(vec![0u8, 1, 2, 3]),
    });
    core.drain_files().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while core.loading() {
        assert!(
            std::time::Instant::now() < deadline,
            "import worker did not finish"
        );
        core.poll_loading().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        core.controller.borrow().drawing().unwrap().id(),
        demo_id,
        "a failed import must keep the current document"
    );
    assert!(!core.presenting_open.get(), "presentation must be cleared");
}

#[test]
fn synchronous_delivered_open_arms_the_guard_with_the_platform_target() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    // Valid, importable bytes: round-trip the demo drawing through the lossy DXF
    // export so the synchronous install can succeed.
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.begin_open_attempt();
    core.open_pending.set(true);
    let target = FileTarget {
        display_name: "drawing.dxf".into(),
        opaque: "saf://handle/42".into(),
    };
    files.events.borrow_mut().push(FileEvent::Opened {
        target: target.clone(),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert_eq!(
        files.source.borrow().as_ref(),
        Some(&target),
        "the guard arms with the platform's opaque handle, not the display name"
    );
}

#[test]
fn navigation_changes_only_camera_and_invalid_pan_stays_an_error() {
    let core = core_with(Rc::new(FakeFiles::default()), false);
    let (entity, _) = add_two_lines(&core);
    select_entity(&core, entity);
    run_controller_command(
        &core,
        CommandId::Measure,
        CommandPayload::MeasureTool(cad_app::MeasurementToolKind::Distance),
    );
    core.controller
        .borrow_mut()
        .session
        .set_measurement_cursor(Some(p(0.0, 0.0)))
        .unwrap();
    let snap_hints_before = core.controller.borrow().snap_hints_near_cursor().unwrap();
    assert!(!snap_hints_before.is_empty());

    let before_drawing = core.controller.borrow().drawing().unwrap();
    let (
        document,
        viewport,
        generation,
        selection,
        history,
        measurement_preview,
        has_measurement,
        camera,
        saved_camera,
    ) = {
        let c = core.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .unwrap();
        (
            c.document_id,
            c.viewport_id,
            c.session.generation,
            c.session.selection.refs().to_vec(),
            c.history_availability(),
            c.session.measurement_preview(),
            c.session.has_last_measurement(),
            viewport.camera,
            viewport.view_mode.saved_2d_camera(),
        )
    };
    let pan = Command {
        schema_version: 1,
        id: CommandId::Pan,
        document,
        viewport,
        payload: CommandPayload::Points(vec![p(15.0, -7.0)]),
    };
    core.controller.borrow_mut().execute(pan).unwrap();
    let zoom = Command {
        schema_version: 1,
        id: CommandId::Zoom,
        document,
        viewport,
        payload: CommandPayload::ZoomAt {
            factor: 1.5,
            cursor: [400.0, 300.0],
        },
    };
    core.controller.borrow_mut().execute(zoom).unwrap();

    let c = core.controller.borrow();
    let current = c.application.workspace.viewports.get(&viewport).unwrap();
    assert_ne!(current.camera, camera);
    assert_eq!(
        current.view_mode.saved_2d_camera(),
        current.camera,
        "2D saved camera tracks navigation"
    );
    assert_ne!(current.camera, saved_camera);
    assert_eq!(c.session.generation, generation);
    assert_eq!(c.session.selection.refs(), selection);
    assert_eq!(c.history_availability(), history);
    assert!(matches!(c.session.tool, cad_app::ToolState::Measuring(_)));
    assert_eq!(c.session.measurement_preview(), measurement_preview);
    assert_eq!(c.session.has_last_measurement(), has_measurement);
    assert!(!c.snap_hints_near_cursor().unwrap().is_empty());
    assert!(Arc::ptr_eq(&before_drawing, &c.drawing().unwrap()));
    drop(c);

    let invalid = core.command(CommandId::Pan, CommandPayload::None);
    assert!(matches!(invalid, Err(CadError::InvalidInput(_))));
}

#[test]
fn light_navigation_push_qualification_matrix() {
    let eligible =
        || use_light_navigation_push(CommandId::Pan, true, true, true, true, true, false);
    assert!(eligible());
    for (id, succeeded, viewport, drawing, handle, view, loading) in [
        (CommandId::Zoom, false, true, true, true, true, false),
        (CommandId::FitDrawing, true, true, true, true, true, false),
        (CommandId::Pan, true, false, true, true, true, false),
        (CommandId::Pan, true, true, false, true, true, false),
        (CommandId::Pan, true, true, true, false, true, false),
        (CommandId::Pan, true, true, true, true, false, false),
        (CommandId::Pan, true, true, true, true, true, true),
    ] {
        assert!(
            !use_light_navigation_push(id, succeeded, viewport, drawing, handle, view, loading),
            "unexpected light push for {id:?}"
        );
    }
}
