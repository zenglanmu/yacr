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
        None
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
