//! `WIPEOUTFRAME` at the host boundary: the same session token maps to the
//! display base and to the vector-plot context with **different** rules.
//!
//! `ON` and `Display but not plot` both draw the boundary frame on screen; only
//! `ON` plots it. These are CPU/session contracts (no GPU), so they are not
//! pixel or plot acceptance evidence.

use super::*;
use cad_app::WipeoutFrames;

fn set_frames(core: &HostCore, mode: WipeoutFrames) {
    core.controller
        .borrow_mut()
        .session
        .drafting_extras
        .set_wipeout_frames(mode);
}

/// The display base maps `ON` and `Display but not plot` to `true`, `OFF` to
/// `false`; the plot context maps only `ON` to `true`.
#[test]
fn wipeout_frames_reach_the_display_and_the_plot_with_distinct_mappings() {
    let core = core();
    // The default session state is ON; sync once so the baseline is published.
    core.sync_render().expect("the baseline sync publishes");
    let on_token = core.render.input_revision();
    assert!(
        core.plot_context()
            .expect("the plot context builds")
            .wipeout_frames,
        "WIPEOUTFRAME defaults to ON, so the plot context draws the frame"
    );

    // OFF: the display drops the frame, so the base must be rebuilt; the plot
    // context drops it too.
    set_frames(&core, WipeoutFrames::Off);
    core.sync_render().expect("the OFF sync publishes");
    let off_token = core.render.input_revision();
    assert!(
        off_token > on_token,
        "OFF removes frame geometry, so the display base must be rebuilt"
    );
    assert!(
        !core
            .plot_context()
            .expect("the plot context builds")
            .wipeout_frames,
        "OFF must not plot the frame"
    );

    // ON again: the frame returns to the display and to the plot.
    set_frames(&core, WipeoutFrames::On);
    core.sync_render().expect("the ON sync publishes");
    let on_again = core.render.input_revision();
    assert!(
        on_again > off_token,
        "ON adds frame geometry, so the display base must be rebuilt"
    );
    assert!(
        core.plot_context()
            .expect("the plot context builds")
            .wipeout_frames,
        "ON must plot the frame"
    );

    // A repeat sync of the same state is a no-op.
    core.sync_render().expect("the repeat sync publishes");
    assert_eq!(
        core.render.input_revision(),
        on_again,
        "an unchanged WIPEOUTFRAME must not churn the base"
    );

    // DISPLAY: the display already shows the frame, so the base does **not**
    // change; the plot context alone drops it. This is the display/plot split.
    set_frames(&core, WipeoutFrames::DisplayNotPlot);
    core.sync_render().expect("the DISPLAY sync publishes");
    assert_eq!(
        core.render.input_revision(),
        on_again,
        "DISPLAY draws the same frame in the display as ON, so no rebuild"
    );
    assert!(
        !core
            .plot_context()
            .expect("the plot context builds")
            .wipeout_frames,
        "DISPLAY must not plot the frame"
    );
}
