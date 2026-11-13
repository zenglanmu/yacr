//! The owner tick's **native pinch intake** (Linux: a trackpad pinch that WebKit
//! never reports as a DOM event).
//!
//! `gtk_bridge::pinch` pushes [`NativePinch`] samples in the gesture's own
//! widget-local GTK logical px; only the tick knows the CAD partition rect and
//! the CSS↔GTK ratio **at the instant the batch is applied**. So this module is
//! the one place that converts them (through the shared
//! [`cad_ui_core::session::native_pinch_input`]) and appends the successful ones
//! to the tick's existing ordered input batch, which the caller then applies with
//! **one** `CadSession::handle_inputs` publication.
//!
//! Ordering contract (frozen): the DOM inputs are already in
//! `shell.cad_inputs` in admission order when this runs, so appending here makes
//! the batch **DOM inputs first, then this tick's native pinches**, in queue
//! order. Every sample keeps its own incremental `scale`; they are never merged,
//! summed or reordered. A refused sample is logged once per distinct reason (the
//! tick would otherwise repeat it for every frame of a gesture) and never
//! silently dropped into a plausible value.
//!
//! A native pinch has **no terminal event**: it is stateless navigation (it never
//! touches the retained pointer press), so the gesture simply stops sending
//! samples and there is no second pointer stream to close.

use std::cell::RefCell;

use cad_ui_core::session::{native_pinch_input, PinchViewport};
use cad_ui_core::shell::CAD_INPUT_CAP;

use super::admission::{bridge_buffer_scale, css_to_gtk};
use super::Shared;

thread_local! {
    /// The last pinch-intake state reported, so a continuous gesture logs its
    /// refusal once instead of once per tick (main thread only, like the tick).
    static LAST_PINCH_NOTE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Whether the sample's CSS point falls inside the accepted CAD partition.
///
/// Edges are inclusive: a pinch anchored exactly on the partition border is a
/// CAD pinch (the partition owns that pixel row).
fn inside_partition(
    rect: &cad_ui_core::layout::CadMetrics,
    x: Option<f32>,
    y: Option<f32>,
) -> bool {
    let (Some(x), Some(y)) = (x, y) else {
        // No position at all: `native_pinch_input` always sets one, so this is a
        // programming error; the sample is refused rather than guessed.
        return false;
    };
    let left = f64::from(rect.x);
    let top = f64::from(rect.y);
    let right = left + f64::from(rect.width);
    let bottom = top + f64::from(rect.height);
    let (x, y) = (f64::from(x), f64::from(y));
    x >= left && x <= right && y >= top && y <= bottom
}

/// Log `note` unless it is the note already reported.
fn note_once(note: &str) {
    LAST_PINCH_NOTE.with(|last| {
        if last.borrow().as_deref() == Some(note) {
            return;
        }
        eprintln!("[probe] pinch: {note}");
        *last.borrow_mut() = Some(note.to_string());
    });
}

/// Drain `Shared::cad_pinches` into this tick's ordered input batch.
///
/// Called by the tick right before it takes `shell.cad_inputs`, so both queues
/// are read under the same lock and the batch keeps a single publication.
///
/// The mapping uses the **same** numbers the tick's layout path uses
/// (`css_to_gtk(rect.dpr, bridge_buffer_scale())` for the ratio and the accepted
/// rect's CSS origin), so a native pinch anchors exactly where a DOM pinch at the
/// same CSS point would.
pub(super) fn append_native_pinches(state: &mut Shared) {
    if state.cad_pinches.is_empty() {
        return;
    }
    // Take the samples in one go, so the loop below only touches `cad_inputs`.
    let pinches: Vec<_> = state.cad_pinches.drain(..).collect();
    let Some(rect) = state.shell.input.last_rect() else {
        // No accepted CAD rect yet: there is no canvas CSS space to map into.
        // The samples cannot be applied and are not retried (their incremental
        // scales would be meaningless later), so say so once.
        note_once("dropped native samples: no accepted CAD rect yet");
        return;
    };
    let viewport = PinchViewport {
        rect_x: f64::from(rect.x),
        rect_y: f64::from(rect.y),
        ratio: css_to_gtk(f64::from(rect.dpr), bridge_buffer_scale()),
    };
    for pinch in pinches {
        if state.shell.cad_inputs.len() >= CAD_INPUT_CAP {
            note_once("dropped native samples: the CAD input queue is full");
            break;
        }
        match native_pinch_input(&pinch, viewport) {
            Ok(input) => {
                // A pinch over chrome (ribbon, dock, a floating palette) is not a
                // CAD gesture. The GDK handler swallows it so WebKit cannot zoom
                // the page, and it is dropped here so it cannot zoom the drawing
                // either. The partition rect is the one authority for "inside".
                if !inside_partition(&rect, input.x, input.y) {
                    note_once("dropped native samples outside the CAD partition");
                    continue;
                }
                state.shell.cad_inputs.push(input);
            }
            Err(reason) => {
                let note = format!("refused native sample: {reason}");
                note_once(&note);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_ui_core::layout::CadMetrics;

    fn rect() -> CadMetrics {
        CadMetrics {
            x: 100.0,
            y: 50.0,
            width: 400.0,
            height: 300.0,
            dpr: 2.0,
            sequence: 7,
        }
    }

    #[test]
    fn the_partition_owns_its_edges_and_nothing_outside() {
        let rect = rect();
        assert!(inside_partition(&rect, Some(100.0), Some(50.0)));
        assert!(inside_partition(&rect, Some(500.0), Some(350.0)));
        assert!(inside_partition(&rect, Some(300.0), Some(200.0)));
        assert!(!inside_partition(&rect, Some(99.9), Some(200.0)));
        assert!(!inside_partition(&rect, Some(300.0), Some(350.1)));
        assert!(
            !inside_partition(&rect, None, Some(200.0)),
            "no position is never guessed"
        );
    }
}
