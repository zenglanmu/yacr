#![cfg(target_os = "linux")]
//! Adapter interaction gating against the real Slint component.
//!
//! Runs on the software-Vulkan offscreen platform, separate from
//! `concept_offscreen.rs` so each binary owns its single `set_platform` call.
use cad_app::viewer_config::ViewerConfig;
use cad_domain::{CadResult, Point3};
use cad_ui_slint::{
    offscreen, CanvasPickMapper, MeasurementUiState, UiAdapter, UiCommandSink, UiConfiguration,
    ViewInput,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Install the offscreen platform exactly once per process.
///
/// The Slint offscreen platform is thread-local and installs at most once, so
/// this binary keeps a **single** test that installs it and exercises every
/// adapter on that one thread. A second `#[test]` would run on another thread
/// with no platform (see the same constraint in `host_config_disk.rs`).
fn install_offscreen() {
    offscreen::install().unwrap();
}

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

/// Records the command ids a shell dispatch produced, in order.
struct RecordingSink(Rc<RefCell<Vec<cad_app::CommandId>>>);
impl UiCommandSink for RecordingSink {
    fn send(&mut self, command: cad_app::Command) -> CadResult<()> {
        self.0.borrow_mut().push(command.id);
        Ok(())
    }
}

#[derive(Default)]
struct Counts {
    pointer: Cell<usize>,
    scroll: Cell<usize>,
    pick: Cell<usize>,
}

struct CountingInput(Rc<Counts>);
impl ViewInput for CountingInput {
    fn pointer(&self, _kind: i32, _button: i32, _x: f64, _y: f64) {
        self.0.pointer.set(self.0.pointer.get() + 1);
    }
    fn scroll(&self, _dx: f64, _dy: f64) {
        self.0.scroll.set(self.0.scroll.get() + 1);
    }
}

struct CountingMapper(Rc<Counts>);
impl CanvasPickMapper for CountingMapper {
    fn to_world(&self, _logical: [f64; 2]) -> Option<Point3> {
        self.0.pick.set(self.0.pick.get() + 1);
        Some(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })
    }
}

#[test]
fn pointer_flag_gates_view_input_and_configured_ribbon_commands_dispatch() {
    install_offscreen();
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    let counts = Rc::new(Counts::default());
    adapter.set_view_input(Rc::new(CountingInput(counts.clone())));
    adapter.set_canvas_pick_mapper(Rc::new(CountingMapper(counts.clone())));
    // A running measurement is what makes a canvas pick consume the mapper at
    // all, so the pick gate is observable rather than shadowed by the idle rule.
    adapter
        .handle()
        .set_measurement_state(&MeasurementUiState {
            active: true,
            ..Default::default()
        })
        .unwrap();

    // Defaults: every enabled path is delivered.
    adapter.component().invoke_pointer_input(2, 0, 10.0, 10.0);
    adapter.component().invoke_scroll_input(1.0, 0.0);
    adapter.component().invoke_canvas_pick(5.0, 5.0);
    assert_eq!(counts.pointer.get(), 1);
    assert_eq!(counts.scroll.get(), 1);
    assert_eq!(counts.pick.get(), 1);

    let mut disabled = ViewerConfig::default();
    disabled.interaction.pointer = false;
    adapter.handle().set_config(disabled).unwrap();

    adapter.component().invoke_pointer_input(2, 0, 20.0, 20.0);
    adapter.component().invoke_scroll_input(1.0, 0.0);
    adapter.component().invoke_canvas_pick(6.0, 6.0);
    assert_eq!(
        counts.pointer.get(),
        1,
        "disabled pointer must not reach ViewInput"
    );
    assert_eq!(
        counts.scroll.get(),
        1,
        "disabled pointer must not reach scroll"
    );
    assert_eq!(
        counts.pick.get(),
        1,
        "disabled pointer must not reach the pick mapper"
    );

    // The gate reads the store at event time, so re-enabling restores delivery
    // without reinstalling any callback.
    adapter
        .handle()
        .set_config(ViewerConfig::default())
        .unwrap();
    adapter.component().invoke_pointer_input(2, 0, 30.0, 30.0);
    adapter.component().invoke_scroll_input(1.0, 0.0);
    adapter.component().invoke_canvas_pick(7.0, 7.0);
    assert_eq!(counts.pointer.get(), 2);
    assert_eq!(counts.scroll.get(), 2);
    assert_eq!(counts.pick.get(), 2);

    // Configured-ribbon dispatch, on the same platform-owning thread.
    use cad_app::CommandId;

    // `view.reset` is rendered as a plain configured-ribbon button but the
    // shipped chrome has no reset control, so dispatch emits the payload-free
    // `ResetView` straight through the shared command sink.
    let recorded = Rc::new(RefCell::new(Vec::new()));
    let ribbon = UiAdapter::new(
        UiConfiguration::default(),
        RecordingSink(recorded.clone()),
        true,
    )
    .unwrap();
    ribbon
        .component()
        .invoke_ribbon_command("view.reset".into());
    assert_eq!(recorded.borrow().as_slice(), &[CommandId::ResetView]);

    // A deliberately ambiguous id still produces no command and reports
    // instead: `layer.toggle` needs a layer row the ribbon cannot supply.
    recorded.borrow_mut().clear();
    ribbon
        .component()
        .invoke_ribbon_command("layer.toggle".into());
    assert!(
        recorded.borrow().is_empty(),
        "an ambiguous id must not fabricate a command"
    );
}
