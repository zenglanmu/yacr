#![cfg(target_os = "linux")]
//! Adapter interaction gating against the real Slint component.
//!
//! Runs on the software-Vulkan offscreen platform, separate from
//! `concept_offscreen.rs` so each binary owns its single `set_platform` call.
//!
//! Canvas *pick* routing is not observable on a detached adapter any more: after
//! the engine decoupling the shell forwards canvas-local points to the attached
//! engine and has no host to route to when none is attached. The pick and
//! touch-guard wiring contracts are covered by `engine_adapter_contract.rs`; the
//! pure guard state machine stays unit-tested in `src/tests.rs`.
use cad_app::viewer_config::ViewerConfig;
use cad_domain::CadResult;
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration, ViewInput};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Install the offscreen platform exactly once per process.
///
/// The Slint offscreen platform is thread-local and installs at most once, so
/// this binary keeps a **single** test that installs it and exercises every
/// adapter on that one thread.
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
    scroll_cursor: Cell<Option<[f64; 2]>>,
    pinch: Cell<Option<(i32, f64, f64, f64)>>,
}

struct CountingInput(Rc<Counts>);
impl ViewInput for CountingInput {
    fn pointer(&self, _kind: i32, _button: i32, _x: f64, _y: f64) {
        self.0.pointer.set(self.0.pointer.get() + 1);
    }
    fn scroll(&self, _dx: f64, _dy: f64, cursor: [f64; 2]) {
        self.0.scroll.set(self.0.scroll.get() + 1);
        self.0.scroll_cursor.set(Some(cursor));
    }
    fn pinch(&self, phase: i32, scale: f64, x: f64, y: f64) {
        self.0.pinch.set(Some((phase, scale, x, y)));
    }
}

#[test]
fn pointer_flag_gates_view_input_and_configured_ribbon_commands_dispatch() {
    install_offscreen();
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    let counts = Rc::new(Counts::default());
    adapter.set_view_input(Rc::new(CountingInput(counts.clone())));

    // Defaults: every enabled path is delivered to the host view input.
    adapter
        .component()
        .invoke_pointer_input(2, 0, 10.0, 10.0, 0);
    adapter
        .component()
        .invoke_scroll_input(1.0, 0.0, 42.0, 73.0);
    adapter.component().invoke_pinch_input(0, 1.0, 42.0, 73.0);
    adapter.component().invoke_pinch_input(1, 1.5, 42.0, 73.0);
    assert_eq!(counts.pointer.get(), 1);
    assert_eq!(counts.scroll.get(), 1);
    assert_eq!(counts.scroll_cursor.get(), Some([42.0, 73.0]));
    assert_eq!(counts.pinch.get(), Some((1, 1.5, 42.0, 73.0)));

    // A disabled pointer stops pointer and scroll delivery without any callback
    // reinstall.
    let pointer_before_disable = counts.pointer.get();
    let scroll_before_disable = counts.scroll.get();
    let mut disabled = ViewerConfig::default();
    disabled.interaction.pointer = false;
    adapter.handle().set_config(disabled).unwrap();

    adapter
        .component()
        .invoke_pointer_input(2, 0, 20.0, 20.0, 0);
    adapter
        .component()
        .invoke_scroll_input(1.0, 0.0, 20.0, 30.0);
    assert_eq!(
        counts.pointer.get(),
        pointer_before_disable,
        "disabled pointer must not reach ViewInput"
    );
    assert_eq!(
        counts.scroll.get(),
        scroll_before_disable,
        "disabled pointer must not reach scroll"
    );

    // The gate reads the store at event time, so re-enabling restores delivery.
    adapter
        .handle()
        .set_config(ViewerConfig::default())
        .unwrap();
    adapter
        .component()
        .invoke_pointer_input(2, 0, 30.0, 30.0, 0);
    adapter
        .component()
        .invoke_scroll_input(1.0, 0.0, 30.0, 40.0);
    assert_eq!(counts.pointer.get(), pointer_before_disable + 1);
    assert_eq!(counts.scroll.get(), scroll_before_disable + 1);

    // An in-flight pinch is cancelled exactly once when the gate closes, and a
    // re-enabled update cannot invent a baseline.
    adapter.component().invoke_pinch_input(0, 1.0, 3.0, 4.0);
    let mut disabled = ViewerConfig::default();
    disabled.interaction.pointer = false;
    adapter.handle().set_config(disabled).unwrap();
    adapter.component().invoke_pinch_input(1, 1.2, 3.0, 4.0);
    let Some((phase, scale, x, y)) = counts.pinch.get() else {
        panic!("cancel callback missing");
    };
    assert_eq!(phase, 3, "disable cancels forwarded pinch");
    assert!((scale - 1.2).abs() < 1e-6 && x == 3.0 && y == 4.0);
    adapter
        .handle()
        .set_config(ViewerConfig::default())
        .unwrap();
    adapter.component().invoke_pinch_input(1, 1.5, 3.0, 4.0);
    assert_eq!(
        counts.pinch.get().unwrap().0,
        3,
        "re-enable update cannot invent a baseline"
    );
    adapter.component().invoke_pinch_input(0, 1.0, 3.0, 4.0);
    adapter.component().invoke_pinch_input(1, 1.2, 3.0, 4.0);
    let (phase, scale, x, y) = counts.pinch.get().unwrap();
    assert_eq!(phase, 1);
    assert!((scale - 1.2).abs() < 1e-6 && x == 3.0 && y == 4.0);
    let mut disabled = ViewerConfig::default();
    disabled.interaction.pointer = false;
    adapter.handle().set_config(disabled).unwrap();
    adapter.component().invoke_pinch_input(2, 1.2, 3.0, 4.0);
    assert_eq!(
        counts.pinch.get().unwrap().0,
        3,
        "disabled end cancels exactly once"
    );
    adapter
        .handle()
        .set_config(ViewerConfig::default())
        .unwrap();

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

    // A deliberately ambiguous id still produces no command and reports instead:
    // `layer.toggle` needs a layer row the ribbon cannot supply.
    recorded.borrow_mut().clear();
    ribbon
        .component()
        .invoke_ribbon_command("layer.toggle".into());
    assert!(
        recorded.borrow().is_empty(),
        "an ambiguous id must not fabricate a command"
    );
}
