//! UI-free synthetic contracts for the host boundary.
//!
//! These drive `HostCore` directly through the platform contracts and the
//! `EngineClient` surface (no Slint handle, no window, no GPU), so they are
//! platform-general. They are not render/run acceptance evidence.
use super::*;
use cad_app::{DrawToolKind, Viewport};
use cad_platform::images::ImageLoadReport;
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use std::collections::BTreeMap;

// --- synthetic platform contracts -------------------------------------------

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

/// A config store that records every persisted preference projection, so a
/// clear's durable write can be asserted without a UI.
#[derive(Default)]
struct RecordingConfig {
    persisted: RefCell<Vec<String>>,
}

impl ConfigStore for RecordingConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, text: &str) -> CadResult<()> {
        self.persisted.borrow_mut().push(text.to_string());
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

fn options() -> HostOptions {
    HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [800.0, 600.0],
    }
}

fn capabilities(import_worker: bool) -> HostCapabilities {
    HostCapabilities {
        open: true,
        new: true,
        save: true,
        plot: true,
        import_worker,
    }
}

fn core_with_caps(files: Rc<dyn PlatformFiles>, capabilities: HostCapabilities) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities,
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap()
}

fn core_with(files: Rc<FakeFiles>, import_worker: bool) -> HostCore {
    core_with_caps(files, capabilities(import_worker))
}

fn core() -> HostCore {
    core_with(Rc::new(FakeFiles::default()), false)
}

fn core_with_config(config_store: Rc<dyn ConfigStore>) -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store,
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap()
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
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

fn catalog(status: &StatusMessage) -> (&str, BTreeMap<String, String>) {
    match status {
        StatusMessage::Catalog { key, args } => (
            key.as_str(),
            args.iter()
                .map(|arg| (arg.key.clone(), arg.value.clone()))
                .collect(),
        ),
        other => panic!("expected a catalog status, got {other:?}"),
    }
}

// --- EngineClient surface: render identity, config authority, updates -------

#[test]
fn engine_client_uses_the_supplied_render_session_and_publishes_typed_state() {
    let core = core();
    let client: &dyn EngineClient = &core;
    assert!(client.render_session().same_session(&core.render));
    let updates = client.take_updates().unwrap();
    assert!(updates.full.is_some());
    assert!(updates.navigation.is_some());
    assert!(updates.config.is_some());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
}

#[test]
fn config_mutations_go_through_the_one_authority_store() {
    let core = core();
    core.update_config_json("{}").unwrap();
    let snapshot = core.config_snapshot();
    // The inherent accessor and the EngineClient surface expose one store.
    assert_eq!(EngineClient::config_snapshot(&core), snapshot);
    let updates = core.take_updates().unwrap();
    assert_eq!(updates.config.expect("config update is dirty"), snapshot);
    // A second drain reports no config change (unchanged is `None`, not empty).
    assert!(core.take_updates().unwrap().config.is_none());
}

#[test]
fn invalid_canvas_metrics_are_rejected_without_advancing_revision() {
    let core = core();
    let before = core.request_context().metrics_revision;
    assert!(core
        .set_canvas_metrics(CanvasMetrics::new([5.0, 0.0], [800.0, 600.0], 1.0))
        .is_err());
    assert_eq!(core.request_context().metrics_revision, before);
}

#[test]
fn a_failed_derivation_keeps_dirty_markers_for_retry() {
    let core = core();
    let (viewport_id, document_id) = {
        let c = core.controller.borrow();
        (c.viewport_id, c.document_id)
    };
    // Force the derived-state query to fail by removing the viewport.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .remove(&viewport_id);
    assert!(
        core.take_updates().is_err(),
        "a failed derivation must be Err, never an empty success"
    );
    // Restore and retry: the dirty markers were not consumed by the failure.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .insert(
            viewport_id,
            Viewport::new(viewport_id, document_id, [800.0, 600.0]),
        );
    let updates = core
        .take_updates()
        .expect("the retry succeeds once restored");
    assert!(
        updates.full.is_some(),
        "the full state is re-derived after a failed attempt"
    );
}

#[test]
fn interaction_changes_tool_generation_without_touching_the_document_or_session() {
    let core = core();
    let client: &dyn EngineClient = &core;
    assert!(client.render_session().same_session(&core.render));
    let before = client.request_context();
    let drawing_before = core.controller.borrow().drawing().unwrap();

    assert!(matches!(
        client
            .dispatch(EngineRequest::Interaction(InteractionRequest::BeginDraw(
                DrawToolKind::Line
            )))
            .unwrap(),
        RequestResult::Interaction(InteractionOutcome::ToolStarted)
    ));
    let after = client.request_context();
    assert!(
        after.tool_generation.unwrap() > before.tool_generation.unwrap(),
        "starting a tool advances the tool generation"
    );
    assert_eq!(after.document, before.document);
    assert_eq!(after.document_generation, before.document_generation);
    assert!(Arc::ptr_eq(
        &drawing_before,
        &core.controller.borrow().drawing().unwrap()
    ));
    assert!(client.render_session().same_session(&core.render));

    // Cancel returns the session to Idle without touching the document.
    assert!(matches!(
        client
            .dispatch(EngineRequest::Interaction(InteractionRequest::Cancel))
            .unwrap(),
        RequestResult::Interaction(InteractionOutcome::Cancelled)
    ));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(Arc::ptr_eq(
        &drawing_before,
        &core.controller.borrow().drawing().unwrap()
    ));
}

// --- command contracts: MOVE snapshot + history -----------------------------

#[test]
fn move_uses_the_tool_start_selection_and_history_round_trips() {
    let core = core();
    let (a, b) = add_two_lines(&core);
    select_entity(&core, a);
    let original_a = line_endpoints(&core, a);
    let original_b = line_endpoints(&core, b);

    // The MOVE tool snapshots the selection at start; a later selection change
    // must not re-target the operation.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Move)
        .unwrap();
    select_entity(&core, b);
    core.controller
        .borrow_mut()
        .capture_tool_point(p(0.0, 0.0))
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(5.0, 2.0))
        .unwrap();
    core.controller.borrow_mut().confirm_draw().unwrap();

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
fn a_failed_draw_confirmation_preserves_the_capture() {
    let core = core();
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 2.0))
        .unwrap();
    let before = core.controller.borrow().drawing().unwrap().revision();
    // One point is not enough to define a LINE: the confirmation is explicit.
    assert!(core.controller.borrow_mut().confirm_draw().is_err());
    assert!(core.controller.borrow().session.draw_preview().is_some());
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        before
    );
}

// --- file contracts: open/save/plot -----------------------------------------

#[test]
fn a_synchronous_save_completes_inside_the_gesture() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    // Save As DXF: the synchronous platform synthesizes the target in
    // `begin_save`, so the export must run inside the gesture.
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
        written[0].1.windows(7).any(|window| window == b"SECTION"),
        "the written save is an ASCII DXF document"
    );
    let status = core.last_status().expect("the save grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.exported");
    assert_eq!(args.get("path"), Some(&written[0].0));
    drop(written);

    // Plot on a document with no paper layout: the funnel still completes in the
    // gesture and grades the explicit no-layout failure, never picker-waiting.
    core.request_plot().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("plot grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "plot.export_failed");
    assert!(
        args.get("reason")
            .is_some_and(|reason| reason.contains("plot.no_layout")),
        "the graded reason names the real no-layout failure: {args:?}"
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
    let core = core_with_caps(files.clone(), capabilities(false));
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
        written[0].1.windows(4).any(|window| window == b"<svg"),
        "the written plot is a real SVG document, not an empty file"
    );
    let has = |needle: &[u8]| {
        written[0]
            .1
            .windows(needle.len())
            .any(|window| window == needle)
    };
    assert!(
        has(b"<polyline") || has(b"<polygon") || has(b"<path"),
        "the written plot contains real stroke geometry, not just a prolog"
    );
}

#[test]
fn an_async_save_waits_for_the_picker_then_exports_on_the_tick() {
    // `FakeFiles::begin_save` is a no-op (a desktop dialog is asynchronous): the
    // core leaves the interaction pending and exports only once the platform
    // delivers the chosen target on a later drain.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.request_save().unwrap();
    assert!(
        core.save_pending.get(),
        "an async picker leaves the interaction pending"
    );
    assert!(core.pending_save.borrow().is_none());
    files.events.borrow_mut().push(FileEvent::SaveTarget {
        purpose: SavePurpose::SaveDxf,
        target: FileTarget::from_path("/tmp/chosen.dxf"),
    });
    core.drain_files().unwrap();
    core.poll_pending_save().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("the export grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.exported");
    assert_eq!(args.get("path"), Some(&"/tmp/chosen.dxf".to_string()));
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

// --- open funnel: cancel/supersession/async publish -------------------------

#[test]
fn cancelled_open_is_discarded_and_progress_is_cancellable_then_idle() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    core.begin_open_attempt();
    core.open_pending.set(true);

    // While the platform read runs, the drawer shows a running, cancellable job.
    let running = core.import_snapshot().expect("running snapshot");
    assert!(running.running && running.cancellable);

    // Cancel: the read is still shown, but the cancel affordance is disabled.
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

// --- navigation contracts ---------------------------------------------------

#[test]
fn navigation_changes_only_camera_and_invalid_pan_stays_an_error() {
    let core = core();
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
    run_controller_command(
        &core,
        CommandId::Pan,
        CommandPayload::Points(vec![p(15.0, -7.0)]),
    );
    run_controller_command(
        &core,
        CommandId::Zoom,
        CommandPayload::ZoomAt {
            factor: 1.5,
            cursor: [400.0, 300.0],
        },
    );

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
    // An invalid navigation command must not silently no-op.
    assert!(matches!(
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(InteractionRequest::Scroll {
                delta_logical: [0.0, f64::NAN],
                position: PointInput::CanvasLocal {
                    logical_position: [0.0, 0.0],
                    metrics_revision: 0,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            })
        ),
        Err(RequestError::Execution(CadError::InvalidInput(_)))
    ));
}

#[test]
fn off_center_scroll_zoom_keeps_the_anchor_and_uses_the_wheel_factor() {
    let core = core();
    let cursor = [123.0, 177.0];
    let (anchor_before, scale_before) = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        (
            viewport
                .screen_to_world(cursor, viewport.logical_size)
                .unwrap(),
            viewport.world_per_px(),
        )
    };
    let revision = core.request_context().metrics_revision.unwrap();
    let outcome = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical: [0.0, 200.0],
            position: PointInput::CanvasLocal {
                logical_position: cursor,
                metrics_revision: revision,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        }),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        RequestResult::Interaction(InteractionOutcome::ScrollObserved)
    ));
    let (anchor_after, scale_after) = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        (
            viewport
                .screen_to_world(cursor, viewport.logical_size)
                .unwrap(),
            viewport.world_per_px(),
        )
    };
    assert!((scale_after / scale_before - 1.0 / 1.3).abs() < 1e-9);
    assert!((anchor_after.x - anchor_before.x).abs() < 1e-9);
    assert!((anchor_after.y - anchor_before.y).abs() < 1e-9);
}

#[test]
fn pointer_navigation_pans_and_multi_touch_never_commits() {
    let core = core();
    let revision = core.request_context().metrics_revision.unwrap();
    let origin = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        viewport
            .screen_to_world([100.0, 100.0], viewport.logical_size)
            .unwrap()
    };
    let pointer = |id, phase, button, position| InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button,
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    };
    // A left drag pans the view; the world point under the drag start shifts.
    assert!(matches!(
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(pointer(
                1,
                cad_app::interaction::PointerPhase::Down,
                Some(PointerButton::Primary),
                [100.0, 100.0],
            ))
        )
        .unwrap(),
        RequestResult::Interaction(InteractionOutcome::PointerObserved)
    ));
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pointer(
            1,
            cad_app::interaction::PointerPhase::Move,
            Some(PointerButton::Primary),
            [160.0, 100.0],
        )),
    )
    .unwrap();
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pointer(
            1,
            cad_app::interaction::PointerPhase::Up,
            Some(PointerButton::Primary),
            [160.0, 100.0],
        )),
    )
    .unwrap();
    let origin_after = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        viewport
            .screen_to_world([100.0, 100.0], viewport.logical_size)
            .unwrap()
    };
    assert_ne!(origin, origin_after, "a left drag must pan the camera");
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));

    // A second contact replaces an active capture instead of committing it.
    let revision = core.request_context().metrics_revision.unwrap();
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 1.0))
        .unwrap();
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Drawing(_)
    ));
    for (id, phase, position) in [
        (1u64, cad_app::interaction::PointerPhase::Down, [10.0, 10.0]),
        (2u64, cad_app::interaction::PointerPhase::Down, [40.0, 10.0]),
    ] {
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(InteractionRequest::Pointer {
                id: PointerId(id),
                kind: cad_app::interaction::PointerKind::Touch,
                phase,
                button: None,
                position: PointInput::CanvasLocal {
                    logical_position: position,
                    metrics_revision: revision,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            }),
        )
        .unwrap();
    }
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "multi-touch cancels the active tool, never commits it"
    );
    assert!(core.controller.borrow().session.draw_preview().is_none());
}

// --- P2-B compatibility follow-up contracts --------------------------------

fn touch(
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
    metrics_revision: u64,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Touch,
        phase,
        button: None,
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn mouse(
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
    metrics_revision: u64,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn dispatch_pointer(core: &HostCore, request: InteractionRequest) {
    EngineClient::dispatch(core, EngineRequest::Interaction(request)).unwrap();
}

#[test]
fn clear_user_preference_empties_the_one_authority_store() {
    let store = Rc::new(RecordingConfig::default());
    let core = core_with_config(store.clone());
    let client: &dyn EngineClient = &core;

    // Apply an allowed preference through the single authority; it flips a
    // default-on flag and persists a non-empty projection.
    core.apply_user_preference_json(
        r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#,
    )
    .unwrap();
    let with_preference = core.config_snapshot();
    assert!(
        !store.persisted.borrow().is_empty() && store.persisted.borrow().last().unwrap() != "{}",
        "applying a preference persists its projection"
    );

    let persisted_before = store.persisted.borrow().len();
    client.clear_user_preference().unwrap();

    // The store is really empty: the effective config returns to the host
    // default, and the allowed projection is persisted as an empty object.
    let default_effective = core_with_config(Rc::new(RecordingConfig::default()))
        .config_snapshot()
        .effective;
    let cleared = core.config_snapshot();
    assert_eq!(cleared.effective, default_effective);
    assert_ne!(cleared.effective, with_preference.effective);
    let persisted = store.persisted.borrow();
    assert!(persisted.len() > persisted_before);
    assert_eq!(persisted.last().unwrap(), "{}");
    drop(persisted);

    // Inherent and EngineClient surfaces share the one authority, and the
    // change is published as a config delta (not a full state query).
    assert_eq!(EngineClient::config_snapshot(&core), cleared);
    let updates = client.take_updates().unwrap();
    assert_eq!(updates.config.expect("config delta"), cleared);

    // A clear is idempotent and never inherits the trait's refusing default.
    client.clear_user_preference().unwrap();
    assert!(client.clear_user_preference().is_ok());
}

#[test]
fn backend_lifecycle_delta_is_published_without_a_full_query() {
    use cad_app::recovery::BackendOutcome;

    let core = core();
    let client: &dyn EngineClient = &core;

    // The first drain publishes the current actual backend (no device yet).
    let first = client.take_updates().unwrap();
    assert!(
        first.backend.is_some(),
        "the first drain publishes the actual backend state"
    );
    // Nothing changed: no backend delta, and the whole batch is empty.
    let second = client.take_updates().unwrap();
    assert!(second.backend.is_none());
    assert_eq!(second, HostUpdates::default());

    // A real device failure changes the actual backend lifecycle. The delta is
    // published even though every other dirty flag is clear, and it must not
    // force a full `HostState` derivation.
    core.render.fail_device("synthetic device failure", false);
    let updates = client.take_updates().unwrap();
    assert!(
        updates.full.is_none(),
        "a backend delta must not force a full state query"
    );
    let backend = updates.backend.expect("backend delta");
    match backend.outcome {
        Some(BackendOutcome::Failed { .. }) => {}
        other => panic!("expected a Failed backend outcome, got {other:?}"),
    }
    assert_eq!(backend.api, None);

    // Stable again once published.
    assert!(client.take_updates().unwrap().backend.is_none());
}

#[test]
fn full_state_reports_the_document_unit_context() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let document_units = {
        let c = core.controller.borrow();
        c.application
            .workspace
            .documents
            .get(&c.document_id)
            .unwrap()
            .units
            .clone()
    };
    let full = client.take_updates().unwrap().full.expect("full state");
    assert_eq!(
        full.units,
        Some(document_units),
        "units mirror the document context, never None while a document exists"
    );
}

#[test]
fn host_state_selected_draw_kind_is_none_idle_and_some_during_capture() {
    let core = core();
    let client: &dyn EngineClient = &core;

    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        None,
        "idle must not synthesize a Line"
    );
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::BeginDraw(
            DrawToolKind::Line,
        )))
        .unwrap();
    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        Some(DrawToolKind::Line)
    );
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::Cancel))
        .unwrap();
    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        None,
        "after cancel the idle state is None again"
    );
}

#[test]
fn pointer_contact_lifecycle_two_ids_repeat_down_cancel_and_last_release() {
    use cad_app::interaction::PointerPhase;

    let core = core();
    let revision = core.request_context().metrics_revision.unwrap();
    let revision_before = core.controller.borrow().drawing().unwrap().revision();

    // Repeat Down for the same contact id is idempotent: it must not register a
    // second contact and cancel the active capture.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 1.0))
        .unwrap();
    dispatch_pointer(&core, touch(1, PointerPhase::Down, [10.0, 10.0], revision));
    dispatch_pointer(&core, touch(1, PointerPhase::Down, [10.0, 10.0], revision));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Drawing(_)),
        "a repeated Down must not count as a second contact"
    );
    assert!(core.controller.borrow().session.draw_preview().is_some());

    // A genuine second contact id enters multi-touch and cancels the capture,
    // never committing it.
    dispatch_pointer(&core, touch(2, PointerPhase::Down, [40.0, 10.0], revision));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "the second contact cancels the active tool"
    );
    assert!(core.controller.borrow().session.draw_preview().is_none());
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "multi-touch must never commit a drawing"
    );

    // Releasing one of two contacts keeps the gesture; the last release ends it.
    dispatch_pointer(&core, touch(2, PointerPhase::Up, [40.0, 10.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    dispatch_pointer(&core, touch(1, PointerPhase::Up, [10.0, 10.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));

    // Cancel identity: cancelling one contact leaves the other; cancelling the
    // last empties the gesture and cancels any tool.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    dispatch_pointer(&core, touch(3, PointerPhase::Down, [1.0, 1.0], revision));
    dispatch_pointer(&core, touch(4, PointerPhase::Down, [2.0, 2.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    dispatch_pointer(&core, touch(3, PointerPhase::Cancel, [1.0, 1.0], revision));
    dispatch_pointer(&core, touch(4, PointerPhase::Cancel, [2.0, 2.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(core.controller.borrow().session.draw_preview().is_none());
}

#[test]
fn sync_view_does_not_churn_render_revisions_on_a_stable_tick() {
    let core = core();
    // Settle the first (changing) sync, then a stable tick must be idempotent:
    // the overlay bounds are built once, so the render input/scene revision
    // cannot grow just because the 50 ms timer ran again.
    core.sync_view().unwrap();
    let input = core.render.input_revision();
    let overlay = core.render.overlay_revision();
    for _ in 0..4 {
        core.sync_view().unwrap();
    }
    assert_eq!(
        core.render.input_revision(),
        input,
        "a stable sync must not advance the render input revision"
    );
    assert_eq!(
        core.render.overlay_revision(),
        overlay,
        "a stable sync must not churn the overlay revision"
    );

    // The same holds for repeated update drains in the stable tick.
    let client: &dyn EngineClient = &core;
    assert!(client.take_updates().unwrap().full.is_some());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
    assert_eq!(core.render.input_revision(), input);
    assert_eq!(core.render.overlay_revision(), overlay);
}

#[test]
fn missing_and_failed_images_surface_raw_resource_diagnostics() {
    let core = core();
    *core.last_image_report.borrow_mut() = Some(ImageLoadReport {
        requested: vec!["a.png".into(), "b.png".into()],
        loaded: Vec::new(),
        unresolved: vec!["a.png".into()],
        failed: vec!["b.png: decode error".into()],
        bytes: 0,
    });
    core.full_dirty.set(true);

    let full = EngineClient::take_updates(&core)
        .unwrap()
        .full
        .expect("full state");
    let missing: Vec<_> = full
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "resource.missing")
        .collect();
    assert_eq!(
        missing.len(),
        2,
        "one diagnostic per unresolved and per failed image"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.object.is_none()),
        "image diagnostics are document-level"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.severity
            == Some(cad_engine::protocol::DiagnosticSeverity::Error)),
        "missing resources are error-severity, mirroring the pre-decoupling model"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.completeness
            == Some(cad_domain::Completeness::Missing(Vec::new()))),
        "missing resources participate in the Missing completeness summary"
    );
    let keys: Vec<String> = missing
        .iter()
        .filter_map(|diagnostic| match diagnostic.parameters.first() {
            Some(RawDiagnosticParameter::Key(key)) => Some(key.clone()),
            _ => None,
        })
        .collect();
    assert!(
        keys.contains(&"a.png".to_string()),
        "the unresolved key must travel raw for the drawer: {keys:?}"
    );
    assert!(
        keys.contains(&"b.png: decode error".to_string()),
        "the failed key must travel raw for the drawer: {keys:?}"
    );

    // The raw report is still published in full for the resources drawer.
    let images = full.resources.images.expect("image report published");
    assert_eq!(images.unresolved, vec!["a.png".to_string()]);
    assert_eq!(images.failed, vec!["b.png: decode error".to_string()]);
}

#[test]
fn legitimate_three_d_orthographic_view_stays_three_d_in_snapshot_and_navigation() {
    let core = core();
    let (viewport_id, camera) = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        (c.viewport_id, viewport.camera)
    };
    // A legitimate 3D view may keep an orthographic camera. The reported mode
    // must follow the session view mode, not a guess from the projection variant.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .view_mode = cad_app::ViewMode2d3d::ThreeD { saved_2d: camera };

    core.sync_view().unwrap();
    assert_eq!(
        core.render.view_snapshot().mode,
        cad_app::ProjectionKind::ThreeD,
        "the render snapshot must report the real 3D view mode"
    );
    assert!(
        core.render.overlays().viewport_bounds.is_none(),
        "a 3D view must not be given a fabricated 2D overlay extent"
    );
    let navigation = EngineClient::take_updates(&core)
        .unwrap()
        .navigation
        .expect("navigation state");
    assert_eq!(
        navigation.mode,
        cad_app::ProjectionKind::ThreeD,
        "navigation must agree with the snapshot's 3D mode"
    );

    // A typed Primary drag in the 3D (orthographic) view must orbit, not pan:
    // pan translates the camera target, while orbit keeps the target (and the
    // eye-target distance) and only rotates the eye.
    let revision = core.request_context().metrics_revision.unwrap();
    let before = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].camera
    };
    let distance_before = before.distance();
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Down,
            [100.0, 100.0],
            revision,
        ),
    );
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Move,
            [160.0, 120.0],
            revision,
        ),
    );
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Up,
            [160.0, 120.0],
            revision,
        ),
    );
    let after = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].camera
    };
    assert_ne!(after, before, "a 3D Primary drag must move the camera");
    assert_eq!(
        after.target, before.target,
        "orbit preserves the camera target (a pan would translate it)"
    );
    assert!(
        (after.distance() - distance_before).abs() < 1e-9,
        "orbit preserves the eye-target distance"
    );
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "a 3D orbit must not enter the panning tool state"
    );
}

#[test]
fn two_d_overlay_bounds_are_ordered_and_contain_the_viewport_corners() {
    let core = core();
    core.sync_view().unwrap();
    let (min, max) = core
        .render
        .overlays()
        .viewport_bounds
        .expect("a 2D plan view has an overlay extent");
    assert!(
        min.x <= max.x && min.y <= max.y && min.z <= max.z,
        "the overlay extent must be axis-ordered, not top-left/bottom-right: {min:?}..{max:?}"
    );
    assert!(
        max.x > min.x && max.y > min.y,
        "the 2D extent must be non-degenerate on both screen axes: {min:?}..{max:?}"
    );

    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    let [width, height] = viewport.logical_size;
    let top_left = viewport
        .screen_to_world([0.0, 0.0], viewport.logical_size)
        .unwrap();
    let bottom_left = viewport
        .screen_to_world([0.0, height], viewport.logical_size)
        .unwrap();
    assert!(
        top_left.y > bottom_left.y,
        "screen Y is inverted; the extent must be normalized, not passed through as-is"
    );
    for corner in [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]] {
        let point = viewport
            .screen_to_world(corner, viewport.logical_size)
            .unwrap();
        assert!(
            point.x >= min.x - 1e-9 && point.x <= max.x + 1e-9,
            "corner {point:?} x is outside {min:?}..{max:?}"
        );
        assert!(
            point.y >= min.y - 1e-9 && point.y <= max.y + 1e-9,
            "corner {point:?} y is outside {min:?}..{max:?}"
        );
    }
}

// --- P3 bounded request queue contracts -------------------------------------

fn command(core: &HostCore, id: CommandId, payload: CommandPayload) -> Command {
    let c = core.controller.borrow();
    Command {
        schema_version: 1,
        id,
        document: c.document_id,
        viewport: c.viewport_id,
        payload,
    }
}

fn move_cursor_world(x: f64, y: f64) -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::World(p(x, y))))
}

#[test]
fn queue_completes_a_submitted_request_and_wakes_once() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let wakes = Rc::new(Cell::new(0usize));
    {
        let wakes = wakes.clone();
        let waker: Rc<dyn Fn()> = Rc::new(move || wakes.set(wakes.get() + 1));
        client.set_request_waker(Some(waker));
    }

    // MoveCursor needs an active cursor tool to execute cleanly.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    let context = client.request_context();
    let receipt = client.submit(context, move_cursor_world(1.0, 2.0)).unwrap();
    assert_eq!(wakes.get(), 1, "empty -> nonempty wakes exactly once");

    // A submission into a non-empty queue is not a new edge. Use a distinct
    // context so the two MoveCursors do not coalesce.
    let mut second_context = client.request_context();
    second_context.metrics_revision =
        Some(second_context.metrics_revision.unwrap().saturating_add(1));
    client
        .submit(second_context, move_cursor_world(3.0, 4.0))
        .unwrap();
    assert_eq!(wakes.get(), 1);

    let report = client.pump().unwrap();
    assert!(report.processed >= 1);
    assert!(!report.pending, "the pass drained the queued work");

    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 2);
    assert_eq!(completions[0].id, receipt.id);
    assert!(matches!(
        completions[0].result,
        Ok(RequestResult::Interaction(InteractionOutcome::CursorMoved))
    ));
    // Draining to completions-only must not wake again (completion != pending).
    assert_eq!(wakes.get(), 1);
    client.set_request_waker(None);
}

#[test]
fn dispatch_is_busy_while_queued_but_not_for_completions_only() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    // Queued work: the synchronous path must not jump the queue.
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Err(RequestError::Busy)
    ));

    client.pump().unwrap();
    // Completions-only is not pending: the synchronous path is allowed again.
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Ok(RequestResult::Interaction(InteractionOutcome::Cancelled))
    ));
    assert_eq!(client.poll_completions(8).len(), 1);
}

#[test]
fn submit_rejects_gesture_commands_without_allocating() {
    let core = core();
    let client: &dyn EngineClient = &core;
    for id in [
        CommandId::OpenDrawing,
        CommandId::PlotDrawing,
        CommandId::SaveDrawingAs,
        CommandId::SwitchBackend,
    ] {
        let request = EngineRequest::Command(command(&core, id, CommandPayload::None));
        assert!(
            matches!(
                client.submit(client.request_context(), request),
                Err(RequestError::Execution(CadError::InvalidInput(_)))
            ),
            "{id:?} must be refused by submit"
        );
    }
    assert_eq!(core.queue.outstanding(), 0, "a refusal allocates nothing");
}

#[test]
fn config_write_is_busy_while_queue_pending() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();

    assert!(matches!(
        core.update_config_json("{}"),
        Err(CadError::InvalidInput(reason)) if reason == "engine is busy"
    ));

    client.pump().unwrap();
    client.poll_completions(8);
    assert!(core.update_config_json("{}").is_ok());
}

#[test]
fn ordinary_edit_does_not_make_a_queued_command_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let epoch_before = context.document_generation;
    client
        .submit(
            context,
            EngineRequest::Command(command(&core, CommandId::FitDrawing, CommandPayload::None)),
        )
        .unwrap();

    // An ordinary edit bumps the session generation but must not bump the host
    // document epoch, so the queued command stays valid.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 0.0), p(10.0, 0.0))
        .unwrap();
    assert_eq!(
        client.request_context().document_generation,
        epoch_before,
        "an edit must not change the document epoch"
    );

    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert!(
        matches!(completions[0].result, Ok(RequestResult::Command(_))),
        "the ordinary edit must not make the queued command stale"
    );
}

#[test]
fn document_replacement_makes_a_queued_request_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();

    // Replace the authoritative document and record it (as the open/new funnel
    // does): the epoch must advance.
    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert!(client.request_context().document_generation > context.document_generation);

    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn tool_metrics_and_config_staleness_are_checked() {
    // Tool generation: a queued tool request is stale after the tool restarts.
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        let context = client.request_context();
        client.submit(context, move_cursor_world(1.0, 2.0)).unwrap();
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }

    // Metrics: a canvas-local coordinate is stale after the metrics revision moves.
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        let revision = client.request_context().metrics_revision.unwrap();
        client
            .submit(
                client.request_context(),
                EngineRequest::Interaction(InteractionRequest::MoveCursor(
                    PointInput::CanvasLocal {
                        logical_position: [1.0, 2.0],
                        metrics_revision: revision,
                    },
                )),
            )
            .unwrap();
        core.set_canvas_metrics(CanvasMetrics::new([0.0; 2], [1000.0, 800.0], 1.0))
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }

    // Config: a gesture captured before a config change is stale at execution.
    // (A config write is refused while work is queued, so the change happens
    // before the submit.)
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        let context = client.request_context();
        core.update_config_json(r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#)
            .unwrap();
        assert!(client.request_context().config_revision > context.config_revision);
        client
            .submit(
                context,
                EngineRequest::Interaction(InteractionRequest::Pointer {
                    id: PointerId(0),
                    kind: cad_app::interaction::PointerKind::Mouse,
                    phase: cad_app::interaction::PointerPhase::Down,
                    button: Some(PointerButton::Primary),
                    position: PointInput::World(p(0.0, 0.0)),
                    modifiers: cad_app::interaction::Modifiers::default(),
                }),
            )
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }
}

#[test]
fn stale_pointer_up_clears_pointer_life_without_navigating() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let revision = client.request_context().metrics_revision.unwrap();
    // Start a real gesture so there is pointer life to clear.
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::Pointer {
            id: PointerId(1),
            kind: cad_app::interaction::PointerKind::Mouse,
            phase: cad_app::interaction::PointerPhase::Down,
            button: Some(PointerButton::Primary),
            position: PointInput::CanvasLocal {
                logical_position: [10.0, 10.0],
                metrics_revision: revision,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        }))
        .unwrap();
    assert!(core.navigation.active.borrow().contains(&PointerId(1)));

    // Submit a stale Up (wrong epoch) while the pointer life is still open.
    let mut context = client.request_context();
    context.document_generation += 1;
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Pointer {
                id: PointerId(1),
                kind: cad_app::interaction::PointerKind::Mouse,
                phase: cad_app::interaction::PointerPhase::Up,
                button: Some(PointerButton::Primary),
                position: PointInput::CanvasLocal {
                    logical_position: [10.0, 10.0],
                    metrics_revision: revision,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            }),
        )
        .unwrap();
    assert!(
        core.navigation.active.borrow().contains(&PointerId(1)),
        "submission alone must not clear the pointer life"
    );

    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
    assert!(
        core.navigation.active.borrow().is_empty(),
        "a stale Up must clear the pointer life"
    );
    assert!(!core.navigation.navigating.get());
}

#[test]
fn tick_does_not_implicitly_pump_the_queue() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let queued = core.queue.queued();
    assert_eq!(queued, 1);

    // A timer tick must not drain the queue: only an explicit pump may.
    core.tick().unwrap();
    assert_eq!(core.queue.queued(), queued, "tick must not pump");
    assert_eq!(core.queue.completion_count(), 0);
    assert!(core.queue.pending());
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Err(RequestError::Busy)
    ));
}

// --- P3 clock contracts ------------------------------------------------------

fn install_fake_clock(core: &HostCore, value: Rc<Cell<f64>>) {
    let clock: Rc<dyn Fn() -> f64> = Rc::new(move || value.get());
    core.set_request_clock(clock).unwrap();
}

#[test]
fn pump_step_budget_bounds_a_pass_with_a_controlled_clock() {
    let core = core();
    let client: &dyn EngineClient = &core;
    install_fake_clock(&core, Rc::new(Cell::new(0.0)));
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..70u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(1_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let report = client.pump().unwrap();
    assert_eq!(report.processed, 64, "one pass runs at most PUMP_STEPS");
    assert!(report.pending);
    assert_eq!(core.queue.queued(), 6);
    client.poll_completions(256);
}

#[test]
fn pump_time_budget_spans_requests_not_a_single_request() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // Each sample advances 1 ms: starting after the setter's single sample, the
    // pass runs 3 requests before the 4 ms window closes.
    let value = Rc::new(Cell::new(0.0));
    {
        let value = value.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let next = value.get() + 1.0;
            value.set(next);
            next
        });
        core.set_request_clock(clock).unwrap();
    }
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..10u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(2_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let report = client.pump().unwrap();
    assert_eq!(report.processed, 3, "the 4 ms window spans the pass");
    assert!(report.pending);
    assert_eq!(core.queue.queued(), 7);
    client.poll_completions(256);
}

#[test]
fn pump_refuses_non_finite_or_negative_clock_samples() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let core = core();
        let client: &dyn EngineClient = &core;
        let value = Rc::new(Cell::new(0.0));
        install_fake_clock(&core, value.clone());
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        client
            .submit(client.request_context(), move_cursor_world(1.0, 2.0))
            .unwrap();
        // A bad sample only appears once work is pending.
        value.set(bad);

        let error = client.pump().unwrap_err();
        assert!(
            matches!(error, RequestError::Execution(CadError::InvalidInput(_))),
            "{bad} must be refused, not faked"
        );
        assert_eq!(core.queue.queued(), 1, "no request is taken on a bad clock");
        assert_eq!(core.queue.completion_count(), 0);
        assert_eq!(core.queue.outstanding(), 1);
    }
}

#[test]
fn pump_rejects_a_backwards_clock_after_preserving_completed_work() {
    // A clock that turns invalid after one executed request.
    let host = core();
    let client: &dyn EngineClient = &host;
    let calls = Rc::new(Cell::new(0usize));
    {
        let calls = calls.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let n = calls.get() + 1;
            calls.set(n);
            if n <= 3 {
                0.0
            } else {
                f64::NAN
            }
        });
        host.set_request_clock(clock).unwrap();
    }
    host.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..3u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(3_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let error = client.pump().unwrap_err();
    assert!(matches!(
        error,
        RequestError::Execution(CadError::InvalidInput(_))
    ));
    assert_eq!(
        host.queue.completion_count(),
        1,
        "the already-executed completion must not be lost"
    );
    assert_eq!(host.queue.queued(), 2, "the rest stays queued");

    // A backwards sample is likewise an explicit failure.
    let backwards = core();
    let value = Rc::new(Cell::new(100.0));
    {
        let value = value.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let now = value.get();
            value.set(now - 10.0);
            now
        });
        backwards.set_request_clock(clock).unwrap();
    }
    backwards
        .controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    EngineClient::submit(
        &backwards,
        EngineClient::request_context(&backwards),
        move_cursor_world(1.0, 2.0),
    )
    .unwrap();
    assert!(matches!(
        EngineClient::pump(&backwards).unwrap_err(),
        RequestError::Execution(CadError::InvalidInput(_))
    ));
}

#[test]
fn set_request_clock_refuses_an_invalid_first_sample_without_installing() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // NaN and negative first samples are refused; the default clock stays.
    let nan = Rc::new(Cell::new(f64::NAN));
    {
        let nan = nan.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || nan.get());
        assert!(core.set_request_clock(clock).is_err());
    }
    assert!(core.set_request_clock(Rc::new(|| -1.0)).is_err());

    // The default native clock is still in place: a normal pass works.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    assert_eq!(client.pump().unwrap().processed, 1);
}

#[test]
fn pump_without_a_clock_needs_no_clock_when_idle_and_refuses_pending() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.clear_request_clock_for_test();

    // Empty queue: 0 processed, not pending, no clock needed.
    assert_eq!(core.pump().unwrap(), PumpReport::default());

    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let error = client.pump().unwrap_err();
    assert!(matches!(
        error,
        RequestError::Execution(CadError::Unsupported(_))
    ));
    assert_eq!(
        core.queue.queued(),
        1,
        "no request is taken without a clock"
    );
    assert_eq!(core.queue.completion_count(), 0);
}

#[test]
fn clock_and_business_callbacks_reenter_as_busy_without_panicking() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let nested_pump_busy = Rc::new(Cell::new(false));
    let nested_set_busy = Rc::new(Cell::new(false));
    let nested_dispatch_busy = Rc::new(Cell::new(false));
    {
        let core_for_clock = core.clone();
        let nested_pump_busy = nested_pump_busy.clone();
        let nested_set_busy = nested_set_busy.clone();
        let nested_dispatch_busy = nested_dispatch_busy.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            nested_pump_busy.set(matches!(
                EngineClient::pump(&core_for_clock),
                Err(RequestError::Busy)
            ));
            nested_set_busy.set(core_for_clock.set_request_clock(Rc::new(|| 0.0)).is_err());
            nested_dispatch_busy.set(matches!(
                EngineClient::dispatch(
                    &core_for_clock,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            0.0
        });
        core.set_request_clock(clock).unwrap();
    }
    assert!(nested_pump_busy.get(), "re-entrant pump is Busy");
    assert!(
        nested_set_busy.get(),
        "re-entrant set_request_clock is Busy"
    );
    assert!(
        nested_dispatch_busy.get(),
        "re-entrant dispatch during a clock sample is Busy"
    );

    // The same guard holds while the clock runs inside a pump pass.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    assert_eq!(client.pump().unwrap().processed, 1);
    assert!(nested_dispatch_busy.get());
    core.clear_request_clock_for_test();
}

#[test]
fn waker_runs_with_borrows_released_and_rearms_only_while_queued() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let calls = Rc::new(Cell::new(0usize));
    let nested_busy = Rc::new(Cell::new(false));
    {
        let core_for_waker = core.clone();
        let calls = calls.clone();
        let nested_busy = nested_busy.clone();
        let waker: Rc<dyn Fn()> = Rc::new(move || {
            calls.set(calls.get() + 1);
            // A nested call from inside the waker must not panic: all queue
            // borrows are released, and the queue is pending so dispatch is Busy.
            let nested = EngineClient::dispatch(
                &core_for_waker,
                EngineRequest::Interaction(InteractionRequest::Cancel),
            );
            nested_busy.set(matches!(nested, Err(RequestError::Busy)));
        });
        client.set_request_waker(Some(waker));
    }

    client
        .submit(
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(calls.get(), 1, "empty -> nonempty wakes once");
    assert!(nested_busy.get(), "the waker runs with borrows released");

    // A second submit while the queue is non-empty is not a new edge.
    client
        .submit(
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(calls.get(), 1);

    // Draining everything leaves no executable pending: no further wake.
    client.pump().unwrap();
    assert_eq!(calls.get(), 1, "draining to completions must not re-arm");
    client.poll_completions(8);
    client.set_request_waker(None);
}

// --- P3 gate remediation: public sync guard / config stale / epoch ordering ---

/// A platform whose `begin_open` re-enters the engine, to prove a service
/// callback cannot jump the synchronous write guard.
#[derive(Default)]
struct NestedProbeFiles {
    core: RefCell<Option<HostCore>>,
    reentries: Rc<Cell<usize>>,
}

impl PlatformFiles for NestedProbeFiles {
    fn begin_open(&self) -> CadResult<()> {
        if let Some(core) = self.core.borrow().as_ref() {
            assert!(matches!(
                EngineClient::dispatch(
                    core,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            assert!(matches!(
                EngineClient::submit(
                    core,
                    EngineClient::request_context(core),
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            assert!(matches!(EngineClient::pump(core), Err(RequestError::Busy)));
            self.reentries.set(self.reentries.get() + 1);
        }
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
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
    }
}

#[test]
fn public_command_and_execute_are_busy_while_queued_without_side_effects() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let revision_before = core.controller.borrow().drawing().unwrap().revision();

    assert!(matches!(
        core.command(CommandId::FitDrawing, CommandPayload::None),
        Err(CadError::InvalidInput(reason)) if reason == "engine is busy"
    ));
    let cmd = command(&core, CommandId::FitDrawing, CommandPayload::None);
    assert!(matches!(core.execute(cmd), Err(CadError::InvalidInput(_))));
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "a refused synchronous write must have no side effects"
    );

    client.pump().unwrap();
    client.poll_completions(8);
    assert!(core
        .command(CommandId::FitDrawing, CommandPayload::None)
        .is_ok());
}

#[test]
fn sync_service_callback_reentry_is_busy_without_panicking() {
    let files = Rc::new(NestedProbeFiles::default());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap();
    *files.core.borrow_mut() = Some(core.clone());

    // A public execute triggers the platform begin_open callback mid-write.
    let cmd = command(&core, CommandId::OpenDrawing, CommandPayload::None);
    core.execute(cmd).unwrap();
    assert_eq!(files.reentries.get(), 1, "the callback ran exactly once");

    // Break the Rc cycle.
    *files.core.borrow_mut() = None;
}

#[test]
fn coordinate_requests_with_an_old_config_revision_are_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // Capture a context with current metrics but the old config revision.
    let context = client.request_context();
    core.update_config_json(r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#)
        .unwrap();
    assert!(client.request_context().config_revision > context.config_revision);

    let revision_before = core.controller.borrow().drawing().unwrap().revision();
    for request in [
        move_cursor_world(1.0, 2.0),
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
            1.0, 2.0,
        )))),
        EngineRequest::Interaction(InteractionRequest::SelectAt {
            point: PointInput::World(p(1.0, 2.0)),
            mode: cad_app::interaction::SelectionMode::Replace,
        }),
    ] {
        client.submit(context, request).unwrap();
    }
    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 3);
    assert!(
        completions
            .iter()
            .all(|completion| matches!(&completion.result, Err(RequestError::StaleContext))),
        "MoveCursor/CapturePoint/SelectAt must all be config-stale"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "stale coordinate requests must change nothing"
    );
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
}

#[test]
fn same_document_id_replacement_still_advances_the_epoch() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let document = core.controller.borrow().document_id;
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();

    // New keeps the same DocumentId but replaces the drawing: the epoch must
    // still advance, and the queued request becomes stale.
    core.request_new().unwrap();
    assert_eq!(core.controller.borrow().document_id, document);
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn failed_open_does_not_advance_the_epoch() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    let client: &dyn EngineClient = &core;
    let epoch = client.request_context().document_generation;
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        bytes: Arc::from(vec![0u8, 1, 2, 3]),
    });
    assert!(
        core.drain_files().is_err(),
        "the invalid bytes fail the open"
    );
    assert_eq!(
        client.request_context().document_generation,
        epoch,
        "a failed open must not advance the epoch (no fake publish)"
    );
}

#[test]
fn async_publish_advances_the_epoch_even_when_post_steps_fail() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/dwg.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    // Drive ticks until the async publish records the new epoch. `presenting_open`
    // may outlive the publish, so wait on the epoch — the property under test —
    // while polling render preparation like `tick` does.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.request_context().document_generation == context.document_generation {
        assert!(
            std::time::Instant::now() < deadline,
            "import did not publish"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // Resources are Unsupported here (post-publish install fails non-fatally),
    // yet the publish already replaced the document, so the epoch advanced.
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

// --- P3 platform gate: with_sync_request + deliver_external_open --------------

#[test]
fn with_sync_request_is_busy_while_queued_without_side_effects() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let status_before = core.last_status();
    let input_before = core.render.input_revision();

    let calls = Rc::new(Cell::new(0usize));
    {
        let calls = calls.clone();
        let result = core.with_sync_request(move || -> Result<(), RequestError> {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert!(matches!(result, Err(RequestError::Busy)));
    }
    assert_eq!(calls.get(), 0, "the action must not run while busy");
    assert_eq!(core.last_status(), status_before);
    assert_eq!(core.render.input_revision(), input_before);
}

#[test]
fn with_sync_request_runs_the_action_under_the_guard_without_probe_side_effects() {
    let core = core();
    let status_before = core.last_status();
    let input_before = core.render.input_revision();
    let action_calls = Rc::new(Cell::new(0usize));
    let nested = Rc::new(Cell::new([false; 3]));
    {
        let core_for_action = core.clone();
        let action_calls = action_calls.clone();
        let nested = nested.clone();
        let result: Result<u32, RequestError> = core.with_sync_request(move || {
            action_calls.set(action_calls.get() + 1);
            let dispatch_busy = matches!(
                EngineClient::dispatch(
                    &core_for_action,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            );
            let submit_busy = matches!(
                EngineClient::submit(
                    &core_for_action,
                    EngineClient::request_context(&core_for_action),
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            );
            let pump_busy = matches!(
                EngineClient::pump(&core_for_action),
                Err(RequestError::Busy)
            );
            nested.set([dispatch_busy, submit_busy, pump_busy]);
            Ok(7)
        });
        assert_eq!(result.unwrap(), 7);
    }
    assert_eq!(action_calls.get(), 1);
    assert_eq!(
        nested.get(),
        [true, true, true],
        "all three nested entries Busy"
    );
    assert_eq!(core.last_status(), status_before, "no status side effect");
    assert_eq!(
        core.render.input_revision(),
        input_before,
        "no execution probe / sync_render side effect"
    );
}

#[test]
fn deliver_external_open_replaces_and_stales_old_queued_requests() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    // Queued old input is allowed (the IO replace makes it stale).
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    let document = core.controller.borrow().document_id;
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    core.deliver_external_open(FileTarget::from_path("/tmp/external.dxf"), Arc::from(bytes))
        .unwrap();
    assert_eq!(
        core.controller.borrow().document_id,
        document,
        "external replace keeps the same DocumentId"
    );
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn deliver_external_open_failure_does_not_publish_or_advance_epoch() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let epoch = client.request_context().document_generation;
    let error = core.deliver_external_open(
        FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        Arc::from(vec![0u8, 1, 2, 3]),
    );
    assert!(error.is_err(), "invalid bytes must not fake success");
    assert_eq!(
        client.request_context().document_generation,
        epoch,
        "a failed external open must not advance the epoch"
    );
}

#[test]
fn deliver_external_open_is_busy_during_active_work() {
    let core = core();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    let inside_refused = Rc::new(Cell::new(false));
    {
        let core_for_action = core.clone();
        let inside_refused = inside_refused.clone();
        let result = core.with_sync_request(move || {
            let refused = core_for_action
                .deliver_external_open(FileTarget::from_path("/tmp/x.dxf"), Arc::from(bytes))
                .is_err();
            inside_refused.set(refused);
            Ok(())
        });
        result.unwrap();
    }
    assert!(
        inside_refused.get(),
        "an active sync window must refuse external delivery"
    );
}

#[test]
fn deliver_external_open_async_returns_unit_and_marks_loading() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    core.deliver_external_open(FileTarget::from_path("/tmp/external.dxf"), Arc::from(bytes))
        .unwrap();
    // Ok(_) means the delivery started, not that the document is opened; the
    // caller distinguishes completion via loading()/status.
    assert!(core.loading(), "an async start is observable via loading()");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.request_context().document_generation == context.document_generation {
        assert!(
            std::time::Instant::now() < deadline,
            "import did not publish"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(client.request_context().document_generation > context.document_generation);
}

// --- P3 platform gate: synchronous open terminal snapshot --------------------

#[test]
fn external_open_ends_a_pending_picker_and_reports_opened_terminal() {
    let core = core();
    // A real OpenDrawing picker leaves a loading state pending.
    core.request_open().unwrap();
    assert!(core.loading(), "the picker read is pending");

    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/ext.dxf"), Arc::from(bytes))
        .unwrap();

    assert!(
        !core.loading(),
        "external delivery must end the pending picker/read"
    );
    let snapshot = core
        .import_snapshot()
        .expect("a terminal snapshot is retained");
    assert!(!snapshot.running);
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Opened { entities }) if *entities > 0),
        "the real opened entity count is retained: {snapshot:?}"
    );
}

#[test]
fn external_open_failure_reports_a_failed_terminal() {
    let core = core();
    core.request_open().unwrap();
    let error = core.deliver_external_open(
        FileTarget::from_path("/tmp/bad.dwg"),
        Arc::from(vec![0u8, 1, 2, 3]),
    );
    assert!(error.is_err(), "invalid bytes must not fake success");
    assert!(!core.loading());
    let snapshot = core
        .import_snapshot()
        .expect("a terminal snapshot is retained");
    assert!(!snapshot.running);
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Failed { .. })),
        "the original failure is retained: {snapshot:?}"
    );
}

#[test]
fn sync_drain_open_and_external_open_share_the_terminal_snapshot() {
    // The external-delivery path.
    let core = core();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/a.dxf"), Arc::from(bytes))
        .unwrap();
    assert!(matches!(
        core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));

    // The `drain_files` (FileEvent::Opened) path uses the same pipeline, so it
    // must produce the same terminal snapshot rather than a separate/omitted one.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.begin_open_attempt();
    core.open_pending.set(true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/b.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert!(matches!(
        core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));
}

#[test]
fn worker_running_snapshot_wins_over_any_sync_terminal() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/w.dxf"), Arc::from(bytes))
        .unwrap();

    let snapshot = core.import_snapshot().expect("async running snapshot");
    assert!(
        snapshot.running,
        "the real async snapshot wins and is not polluted by a sync terminal"
    );
}

// --- P3 platform latest: snapshot scoped to the current open attempt ---------

fn drive_until_idle(core: &HostCore) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while core.loading() {
        assert!(
            std::time::Instant::now() < deadline,
            "open did not reach a terminal"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn old_async_terminal_does_not_mask_a_new_open_attempt() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    // Attempt 1: a worker publishes a terminal.
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/one.dxf"),
        bytes: Arc::from(bytes.clone()),
    });
    core.drain_files().unwrap();
    drive_until_idle(&core);
    assert!(
        matches!(
            &core.import_snapshot().unwrap().terminal,
            Some(ImportTerminal::Opened { .. })
        ),
        "attempt 1 shows its own terminal"
    );

    // Attempt 2: a new picker must show a real running snapshot, never attempt
    // 1's retained worker terminal.
    core.request_open().unwrap();
    let snapshot = core.import_snapshot().expect("new picker snapshot");
    assert!(
        snapshot.running,
        "the new picker hides the old worker terminal: {snapshot:?}"
    );

    // Attempt 2 continues into a worker: running, then its own terminal.
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/two.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert!(
        core.import_snapshot().unwrap().running,
        "the new worker reports a real running snapshot"
    );
    drive_until_idle(&core);
    assert!(matches!(
        &core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));
}

#[test]
fn new_sync_terminal_is_not_masked_by_an_ineligible_old_async_terminal() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    // Attempt 1: worker terminal retained in the controller.
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/one.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    drive_until_idle(&core);
    assert!(matches!(
        &core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));

    // A new attempt with its own real synchronous terminal must be shown even
    // though the controller still retains attempt 1's async terminal.
    core.begin_open_attempt();
    *core.sync_terminal.borrow_mut() =
        Some(ImportProgressSnapshot::terminal(ImportTerminal::Opened {
            entities: 3,
        }));
    let snapshot = core.import_snapshot().expect("new sync terminal");
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Opened { entities }) if *entities == 3),
        "the new attempt's own terminal wins over the ineligible old async one: {snapshot:?}"
    );
}

// --- P4: origin dispatch, API routing and bounded event trace ----------------

#[test]
fn api_commit_is_one_transaction_and_the_entities_event_shares_the_change_arc() {
    use cad_domain::{EntityId, LayerId, Point3, Revision as Rev, SpaceId};
    use cad_engine::api::{ApiRequest, ApiTarget, EntityEdit, NewEntity};
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, database, viewport, revision) = {
        let c = core.controller.borrow();
        let drawing = c.drawing().unwrap();
        (
            c.document_id,
            drawing.id(),
            c.viewport_id,
            drawing.revision(),
        )
    };
    let target = ApiTarget {
        document,
        database,
        viewport,
    };
    let edit = EntityEdit::Insert {
        layer: LayerId(0),
        space: SpaceId::Model,
        entity: NewEntity::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    };
    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Api,
            EngineRequest::Api {
                target,
                request: ApiRequest::Commit {
                    expected_revision: Rev(revision.0),
                    edits: vec![edit],
                },
            },
        )
        .unwrap();
    let outcome = match &completion.result {
        Ok(RequestResult::Api(outcome)) => outcome.clone(),
        other => panic!("expected an api outcome, got {other:?}"),
    };
    let changes = outcome.changes.clone().expect("a real ChangeSet");
    assert_eq!(outcome.created.len(), 1);

    // One history step; the created entity exists; Undo removes it.
    assert_eq!(
        core.controller.borrow().application.history[&document].undo_depth(),
        1
    );
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(outcome.created[0].0))
        .is_some());
    core.command(CommandId::Undo, CommandPayload::None).unwrap();
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(outcome.created[0].0))
        .is_none());

    // The EntitiesChanged event shares the SAME ChangeSet Arc (no deep copy).
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let shared = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("an EntitiesChanged event");
    assert!(
        Arc::ptr_eq(&shared, &changes),
        "the event shares the command's ChangeSet Arc"
    );
}

#[test]
fn failed_api_write_emits_no_entity_event_and_keeps_the_revision() {
    use cad_domain::{LayerId, Point3, Revision as Rev, SpaceId};
    use cad_engine::api::{ApiRequest, ApiTarget, EntityEdit, NewEntity};
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, database, viewport, revision) = {
        let c = core.controller.borrow();
        let drawing = c.drawing().unwrap();
        (
            c.document_id,
            drawing.id(),
            c.viewport_id,
            drawing.revision(),
        )
    };
    let target = ApiTarget {
        document,
        database,
        viewport,
    };
    let edit = EntityEdit::Insert {
        layer: LayerId(0),
        space: SpaceId::Model,
        entity: NewEntity::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    };
    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Api,
            EngineRequest::Api {
                target,
                request: ApiRequest::Commit {
                    // A stale expected revision must be refused.
                    expected_revision: Rev(revision.0 + 1),
                    edits: vec![edit],
                },
            },
        )
        .unwrap();
    assert!(completion.result.is_err(), "a stale commit is refused");

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(
        events
            .iter()
            .all(|envelope| !matches!(envelope.event, EngineEvent::EntitiesChanged { .. })),
        "a failed write must not emit an entity event"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision,
        "a failed write keeps the revision"
    );
}

#[test]
fn sync_and_queued_requests_allocate_distinct_ids_and_rejects_leak_no_permit() {
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;

    let sync = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(
        core.events.outstanding_reserved(),
        0,
        "sync permit released"
    );

    let receipt = client
        .submit_with_origin(
            RequestOrigin::Ui,
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_ne!(sync.id, receipt.id, "ids are unique across both paths");
    assert_eq!(core.events.outstanding_reserved(), 16, "one live permit");

    client.pump().unwrap();
    assert_eq!(
        core.events.outstanding_reserved(),
        0,
        "an executed request releases its permit"
    );
    assert_eq!(client.poll_completions(8).len(), 1);

    // A refused submission consumes no id and leaks no permit.
    let before = core.events.outstanding_reserved();
    assert!(client
        .submit_with_origin(
            RequestOrigin::Ui,
            client.request_context(),
            EngineRequest::Command(command(&core, CommandId::OpenDrawing, CommandPayload::None)),
        )
        .is_err());
    assert_eq!(core.events.outstanding_reserved(), before);
}

#[test]
fn subscriptions_watermark_and_unknown_ids_are_real() {
    use cad_engine::events::EventPoll;
    use cad_engine::events::SubscriptionId;

    let core = core();
    let client: &dyn EngineClient = &core;
    let watermark = client.event_watermark();
    assert_eq!(watermark.capacity, 1024);
    assert_eq!(watermark.next_sequence, 0);

    let subscription = client.subscribe().unwrap();
    assert!(matches!(
        client.poll_events(subscription, 0).unwrap(),
        EventPoll::Events(ref events) if events.is_empty()
    ));
    client.unsubscribe(subscription).unwrap();
    assert!(client.unsubscribe(subscription).is_err(), "unknown id");
    assert!(client.poll_events(SubscriptionId(9999), 8).is_err());
}

#[test]
fn coalesced_superseded_finishes_at_submit_without_polling_completions() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let context = client.request_context();
    let first = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(1.0, 2.0))
        .unwrap();
    let second = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(3.0, 4.0))
        .unwrap();
    assert_ne!(first.id, second.id);

    // The displaced request's Finished is published at submit time, before any
    // completion polling, and it never claimed a Started.
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let displaced = events
        .iter()
        .find(|envelope| envelope.request_id == Some(first.id))
        .expect("the displaced request has a terminal event");
    assert!(matches!(
        &displaced.event,
        EngineEvent::RequestFinished { result }
            if matches!(**result, Err(RequestError::Superseded { .. }))
    ));
    assert!(
        events
            .iter()
            .all(|envelope| !(envelope.request_id == Some(first.id)
                && matches!(envelope.event, EngineEvent::RequestStarted { .. }))),
        "a superseded request never emits Started"
    );

    // Its real completion is still queued, untouched.
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].id, first.id);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::Superseded { .. })
    ));
}

#[test]
fn stale_queued_request_publishes_only_a_finished() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let context = client.request_context();
    let receipt = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(1.0, 2.0))
        .unwrap();
    // Replace the document: the epoch advances and the queued request is stale.
    core.request_new().unwrap();
    client.pump().unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let mine: Vec<_> = events
        .iter()
        .filter(|envelope| envelope.request_id == Some(receipt.id))
        .collect();
    assert_eq!(
        mine.len(),
        1,
        "a stale request has exactly one terminal event"
    );
    assert!(matches!(
        &mine[0].event,
        EngineEvent::RequestFinished { result } if matches!(**result, Err(RequestError::StaleContext))
    ));
    assert!(
        !mine
            .iter()
            .any(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. })),
        "a stale request never emits Started"
    );
}

#[test]
fn public_command_emits_a_real_command_lifecycle() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    core.command(CommandId::FitDrawing, CommandPayload::None)
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let has = |predicate: &dyn Fn(&EngineEvent) -> bool| events.iter().any(|e| predicate(&e.event));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::RequestStarted { .. }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::CommandStarted {
            command: CommandId::FitDrawing
        }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::CommandFinished {
            command: CommandId::FitDrawing,
            ..
        }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::RequestFinished { .. }
    )));
}

#[test]
fn dispatch_external_with_origin_preflight_rejects_without_action_or_id() {
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let calls = Rc::new(Cell::new(0usize));

    // Wrong command variant.
    {
        let calls = calls.clone();
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(&core, CommandId::OpenDrawing, CommandPayload::None)),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }
    // Stale document preflight.
    {
        let calls = calls.clone();
        let mut cmd = command(
            &core,
            CommandId::SwitchBackend,
            CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
        );
        cmd.document = cad_domain::DocumentId(cmd.document.0 + 999);
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(cmd),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }
    // A `None` payload (compat shim) is refused: the real request carries a
    // Backend(choice) payload.
    {
        let calls = calls.clone();
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(
                &core,
                CommandId::SwitchBackend,
                CommandPayload::None,
            )),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }

    assert_eq!(calls.get(), 0, "preflight must not run the action");
    assert_eq!(core.events.outstanding_reserved(), 0);
    assert_eq!(core.queue.outstanding(), 0);
}

#[test]
fn dispatch_external_with_origin_runs_a_real_traced_switch_backend() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let completion = core
        .dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(
                &core,
                CommandId::SwitchBackend,
                CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
            )),
            || Ok(Arc::new(CommandOutcome::none())),
        )
        .unwrap();
    assert!(matches!(&completion.result, Ok(RequestResult::Command(_))));

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandFinished {
            command: CommandId::SwitchBackend,
            ..
        }
    )));
}

#[test]
fn confirm_surfaces_its_trace_changeset_as_an_entities_event_sharing_the_arc() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::RequestOrigin;

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let world = |x: f64, y: f64| PointInput::World(p(x, y));
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(world(0.0, 0.0))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(world(10.0, 0.0))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Confirm),
        )
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let trace_changes = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.changes.clone()),
            _ => None,
        })
        .expect("the confirm command trace carries a ChangeSet");
    let entities_change = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("the confirm trace surfaces an EntitiesChanged event");
    assert!(
        Arc::ptr_eq(&trace_changes, &entities_change),
        "the entity event shares the trace's ChangeSet Arc (no deep copy)"
    );
}

#[test]
fn post_commit_render_failure_keeps_a_successful_interaction_and_diagnoses() {
    use cad_engine::protocol::{RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    // Force `sync_render` to fail by moving the session to a block space.
    core.controller.borrow_mut().session.active_space =
        cad_domain::SpaceId::Block(cad_domain::BlockId(1));

    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert!(
        matches!(
            completion.result,
            Ok(RequestResult::Interaction(InteractionOutcome::Cancelled))
        ),
        "a post-commit render failure must not reclassify a successful interaction"
    );
    assert!(
        matches!(core.last_status(), Some(StatusMessage::ExternalText(_))),
        "the render failure is surfaced as an explicit diagnostic"
    );
}

#[test]
fn host_funnel_command_records_a_real_command_trace() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    core.command(CommandId::OpenDrawing, CommandPayload::None)
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::OpenDrawing
        }
    )));
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandFinished {
            command: CommandId::OpenDrawing,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|envelope| matches!(envelope.event, EngineEvent::RequestFinished { .. })));
}

#[test]
fn document_replacement_trace_uses_pre_and_post_context() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let pre_epoch = client.request_context().document_generation;
    let pre_revision = core.controller.borrow().drawing().unwrap().revision();
    core.command(CommandId::NewDrawing, CommandPayload::None)
        .unwrap();
    let post_epoch = client.request_context().document_generation;
    let post_revision = core.controller.borrow().drawing().unwrap().revision();
    assert_eq!(post_epoch, pre_epoch + 1, "New advances the document epoch");

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let started = events
        .iter()
        .find(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. }))
        .expect("a RequestStarted");
    let finished = events
        .iter()
        .find(|envelope| matches!(envelope.event, EngineEvent::RequestFinished { .. }))
        .expect("a RequestFinished");
    assert_eq!(started.generation, pre_epoch, "Started is pre-state");
    assert_eq!(
        started.revision, pre_revision,
        "Started revision is pre-state"
    );
    assert_eq!(
        finished.generation, post_epoch,
        "Finished is terminal-state"
    );
    assert_eq!(
        finished.revision, post_revision,
        "Finished revision is terminal-state"
    );
    assert_eq!(started.document, finished.document, "same DocumentId");
}

#[test]
fn a_real_tool_change_emits_a_tool_event() {
    use cad_engine::events::EngineEvent;

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    client
        .dispatch_with_origin(
            cad_engine::protocol::RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();

    let cad_engine::events::EventPoll::Events(events) =
        client.poll_events(subscription, 256).unwrap()
    else {
        panic!("expected events");
    };
    assert!(
        events
            .iter()
            .any(|envelope| matches!(envelope.event, EngineEvent::ToolChanged { .. })),
        "a real tool generation change emits a ToolChanged event"
    );
}

#[test]
fn confirm_commits_then_a_render_failure_keeps_confirmation_and_undo_restores() {
    use cad_domain::EntityId;
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, viewport_id) = {
        let c = core.controller.borrow();
        (c.document_id, c.viewport_id)
    };
    let original_camera = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&viewport_id].camera
    };
    let undo_depth = || {
        core.controller
            .borrow()
            .application
            .history
            .get(&document)
            .map_or(0, |record| record.undo_depth())
    };
    let pre_revision = core.controller.borrow().drawing().unwrap().revision();
    let pre_depth = undo_depth();

    // A real LINE draw: BeginDraw -> two captured points -> Confirm.
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
                0.0, 0.0,
            )))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
                10.0, 0.0,
            )))),
        )
        .unwrap();

    // Inject a real `sync_render` failure (an invalid camera projection) that
    // must not reclassify the successful confirmation.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .camera = cad_app::Camera {
        projection: cad_app::Projection::Orthographic { scale: 0.0 },
        ..original_camera
    };

    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Confirm),
        )
        .unwrap();
    assert!(
        matches!(
            &completion.result,
            Ok(RequestResult::Interaction(InteractionOutcome::Confirmed))
        ),
        "the business confirmation stays successful through a render failure"
    );

    // Real database/history effects (not fabricated): revision + one history step.
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision().0,
        pre_revision.0 + 1
    );
    assert_eq!(undo_depth(), pre_depth + 1);
    // Capture is cleared after a successful confirmation.
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(core.controller.borrow().session.draw_preview().is_none());

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    // The actually created entity - read from the event, then checked in the DB.
    let created = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.objects.first().copied()),
            _ => None,
        })
        .expect("the confirm command created an entity");
    assert!(
        core.controller
            .borrow()
            .drawing()
            .unwrap()
            .entity(EntityId(created.0))
            .is_some(),
        "the created entity really exists in the database"
    );
    assert!(
        matches!(core.last_status(), Some(StatusMessage::ExternalText(_))),
        "the render failure is an explicit diagnostic"
    );

    // The trace shares one ChangeSet Arc between CommandFinished and EntitiesChanged.
    let trace_changes = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|o| o.changes.clone()),
            _ => None,
        })
        .expect("ChangeSet");
    let entities_change = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("EntitiesChanged");
    assert!(Arc::ptr_eq(&trace_changes, &entities_change));
    let finished = events.iter().find(|envelope| {
        envelope.request_id == Some(completion.id)
            && matches!(envelope.event, EngineEvent::RequestFinished { .. })
    });
    assert!(
        finished.is_some_and(|envelope| matches!(
            &envelope.event,
            EngineEvent::RequestFinished { result } if result.as_ref().is_ok()
        )),
        "the request terminal is a success"
    );

    // Restore the camera and Undo once: the real entity is restored.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .camera = original_camera;
    core.command(CommandId::Undo, CommandPayload::None).unwrap();
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(created.0))
        .is_none());
    // Undo restores the history depth (a new monotonic revision is expected; the
    // revision number is not rewound).
    assert_eq!(undo_depth(), pre_depth);
    assert_eq!(
        core.controller
            .borrow()
            .application
            .history
            .get(&document)
            .map_or(0, |record| record.redo_depth()),
        1,
        "exactly one Redo remains available"
    );
}

#[test]
fn dispatch_external_with_origin_failure_returns_a_real_id_error_completion() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let completion = core
        .dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(
                &core,
                CommandId::SwitchBackend,
                CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
            )),
            || {
                Err(RequestError::Execution(CadError::InvalidInput(
                    "backend failed".into(),
                )))
            },
        )
        .unwrap();

    assert!(
        matches!(
            &completion.result,
            Err(RequestError::Execution(CadError::InvalidInput(_)))
        ),
        "the failure is carried in the real-id completion"
    );

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let mine: Vec<_> = events
        .iter()
        .filter(|envelope| envelope.request_id == Some(completion.id))
        .collect();
    assert!(mine
        .iter()
        .any(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. })));
    assert!(mine.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
    assert!(mine.iter().any(|envelope| matches!(
        &envelope.event,
        EngineEvent::CommandFinished { result, .. } if result.as_ref().is_err()
    )));
    assert!(mine.iter().any(|envelope| matches!(
        &envelope.event,
        EngineEvent::RequestFinished { result } if result.as_ref().is_err()
    )));
    assert!(
        mine.iter()
            .all(|envelope| envelope.origin == RequestOrigin::Platform),
        "the trace keeps the external origin"
    );
    assert!(
        mine.iter()
            .all(|envelope| !matches!(envelope.event, EngineEvent::EntitiesChanged { .. })),
        "a failed external command emits no entity event"
    );
}

#[test]
fn dispatch_external_with_origin_action_reentry_is_busy_without_panicking() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestError, RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let nested_busy = Rc::new(Cell::new(false));
    {
        let core_for_action = core.clone();
        let nested_busy = nested_busy.clone();
        let completion = core
            .dispatch_external_with_origin(
                RequestOrigin::Platform,
                EngineRequest::Command(command(
                    &core,
                    CommandId::SwitchBackend,
                    CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
                )),
                move || {
                    let dispatch_busy = matches!(
                        EngineClient::dispatch(
                            &core_for_action,
                            EngineRequest::Interaction(InteractionRequest::Cancel)
                        ),
                        Err(RequestError::Busy)
                    );
                    let submit_busy = matches!(
                        EngineClient::submit(
                            &core_for_action,
                            EngineClient::request_context(&core_for_action),
                            EngineRequest::Interaction(InteractionRequest::Cancel)
                        ),
                        Err(RequestError::Busy)
                    );
                    let pump_busy = matches!(
                        EngineClient::pump(&core_for_action),
                        Err(RequestError::Busy)
                    );
                    nested_busy.set(dispatch_busy && submit_busy && pump_busy);
                    Ok(Arc::new(CommandOutcome::none()))
                },
            )
            .unwrap();
        assert!(matches!(&completion.result, Ok(RequestResult::Command(_))));
    }
    assert!(
        nested_busy.get(),
        "nested dispatch/submit/pump from the action are all Busy"
    );
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
}
