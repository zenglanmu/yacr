//! `LAYFRZ`'s **Viewports** form: the per-viewport freeze (`Vpfreeze`).
//!
//! AutoCAD's `LAYFRZ` Settings ▸ Viewports option freezes the selected layers in
//! chosen viewports instead of at drawing level. This build stores exactly that
//! state where `VPLAYER` / `LAYVPI` already put it:
//! [`cad_db::PaperViewport::layer_overrides`] — `true` means frozen in **that
//! viewport only**, an absent key means the layer keeps its global state. No
//! layer row and no schema is touched, so a layer frozen here still draws in
//! every other viewport, `LAYTHW` clears these overrides together with the
//! drawing-level flags, and `VPLAYER ?` reports them unchanged.
//!
//! # Scope and refusals (nothing written, no transaction, no undo entry)
//!
//! * `layfrz.paper_space_required` — the active space is not a paper layout: a
//!   per-viewport override needs a viewport to live in (AutoCAD's Viewports
//!   option is likewise unavailable from model space);
//! * `layfrz.no_viewport` — the layout has no viewport at all;
//! * `layfrz.unknown_viewport` — an explicit index outside
//!   [`cad_db::Layout::viewports`]; never clamped to the first viewport.
//!
//! The empty index list is the documented "every viewport of the active layout"
//! scope. An already-frozen state is `layfrz.unchanged` and opens no
//! transaction. The whole form is **one** `set_layout_viewports` call recorded
//! as one [`cad_db::LayoutPatch`], so the command is one revision / one undo
//! step like the drawing-level form.
//!
//! The current layer is refused by the caller before this write (see
//! `Application::layfrz`): `VPLAYER` / `LAYVPI` remain the per-viewport route for
//! it.

use super::*;

use cad_history::layout_patch;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};

impl Application {
    /// Freeze `layers` in `targets` viewports of the active paper layout.
    ///
    /// `targets` are zero-based indices into the layout's viewport list; an empty
    /// list means every viewport. Every index is validated before the first
    /// write, so a bad index leaves the drawing untouched.
    pub(super) fn freeze_layers_in_viewports(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[LayerId],
        targets: &[usize],
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "layfrz")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let viewport_count = viewports.len();
        if viewport_count == 0 {
            return Err(unsupported(
                "layfrz",
                "no_viewport",
                format!(
                    "layout {} has no viewport; the Viewports form freezes layers per viewport \
                     and there is no viewport to receive the override",
                    layout_id.0
                ),
            ));
        }
        let mut wanted: Vec<usize> = if targets.is_empty() {
            (0..viewport_count).collect()
        } else {
            targets.to_vec()
        };
        for index in &wanted {
            if *index >= viewport_count {
                return Err(invalid(
                    "layfrz",
                    "unknown_viewport",
                    format!(
                        "layout {} has {viewport_count} viewport(s); there is no viewport {index} \
                         to freeze (viewport indices are zero-based)",
                        layout_id.0
                    ),
                ));
            }
        }
        wanted.sort_unstable();
        wanted.dedup();

        let mut changed = false;
        for index in &wanted {
            let viewport = &mut viewports[*index];
            for layer in layers {
                // Only a real state change counts: re-freezing an already-frozen
                // override must not open a transaction.
                if viewport.layer_overrides.insert(*layer, true) != Some(true) {
                    changed = true;
                }
            }
        }
        let scope = if targets.is_empty() {
            format!("every viewport of layout {}", layout_id.0)
        } else {
            format!(
                "viewport(s) {} of layout {}",
                wanted
                    .iter()
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                layout_id.0
            )
        };
        if !changed {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layfrz.unchanged".into(),
                    message: format!(
                        "LAYFRZ: {scope} already freezes every selected layer; no transaction was \
                         opened"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "layfrz")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "layfrz",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layfrz.applied".into(),
                message: format!(
                    "LAYFRZ: froze {} layer(s) in {scope} as per-viewport overrides; the layer \
                     table is unchanged, so the layers still draw in the other viewports (one \
                     transaction / one undo step)",
                    layers.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
