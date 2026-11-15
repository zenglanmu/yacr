//! `LAYVPI`: freeze layers in the active paper layout's viewports.
//!
//! The overrides live in [`cad_db::PaperViewport::layer_overrides`] (per
//! viewport, never the global layer table), so the write is one validated
//! `set_layout_viewports` call recorded as one [`cad_db::LayoutPatch`] — one
//! revision and one undo step.
//!
//! # Scope (AutoCAD's own default, now real)
//!
//! The no-argument form freezes the named layers in **every viewport except the
//! current one**. The current viewport is the session `CVPORT` marker of the
//! active layout ([`crate::command_id::extras`]); setting that marker is session
//! state and never part of this write.
//!
//! The targeted form (`FreezeViewportLayersIn`) takes an explicit zero-based
//! viewport index and always wins over the marker, so a caller can still freeze
//! the current viewport itself. Both forms are documented in
//! [`crate::command_id::layout`].
//!
//! # Refusals (nothing frozen, no transaction, no undo entry)
//!
//! * `layvpi.paper_space_required` — the active space is not a paper layout;
//! * `layvpi.invalid_argument` — no layer name;
//! * `layvpi.unknown_layer` / `layvpi.ambiguous_layer` — a name that does not
//!   resolve uniquely (resolved **before** anything mutates);
//! * `layvpi.no_viewport` — the layout has no viewport at all;
//! * `layvpi.no_other_viewport` — the no-argument form has nothing to freeze
//!   because the layout's only viewport **is** the current one; the explicit
//!   index form is the documented way to freeze it;
//! * `layvpi.current_viewport_out_of_range` — the stored `CVPORT` marker no
//!   longer names a viewport of this layout (never clamped to the first);
//! * `layvpi.unknown_viewport` — an explicit index outside the viewport count.
//!
//! An already-frozen state is `layvpi.unchanged` and opens no transaction.

use super::*;

use cad_history::layout_patch;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_names::resolve_layer_name;
use crate::session_state::target_viewport;

impl Application {
    /// Handle `LayoutPayload::FreezeViewportLayers`: every viewport except the
    /// session `CVPORT` current one.
    pub(super) fn freeze_viewport_layers(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
    ) -> CadResult<CommandOutcome> {
        self.freeze_viewport_layers_impl(session, command, layers, None)
    }

    /// Handle `LayoutPayload::FreezeViewportLayersIn`: one viewport index only.
    pub(super) fn freeze_viewport_layers_at(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
        viewport: usize,
    ) -> CadResult<CommandOutcome> {
        self.freeze_viewport_layers_impl(session, command, layers, Some(viewport))
    }

    /// The one `LAYVPI` write: `only = None` is "every viewport but the current
    /// one" (AutoCAD's default), `Some(i)` is the explicit index.
    fn freeze_viewport_layers_impl(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
        only: Option<usize>,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "layvpi")?;
        if layers.is_empty() {
            return Err(CadError::InvalidInput(
                "layvpi.invalid_argument: LAYVPI needs at least one layer name".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        // Resolve every name first: an unknown or ambiguous name refuses before
        // anything mutates, so a bad argument never freezes a subset. Repeated
        // names collapse to one override.
        let mut frozen: Vec<(LayerId, String)> = Vec::with_capacity(layers.len());
        for name in layers {
            let id = resolve_layer_name(&drawing, name)?;
            if frozen.iter().any(|(known, _)| *known == id) {
                continue;
            }
            let canonical = drawing
                .layers()
                .find(|layer| layer.id == id)
                .map(|layer| layer.name.clone())
                .unwrap_or_else(|| name.clone());
            frozen.push((id, canonical));
        }
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        if viewports.is_empty() {
            return Err(CadError::Unsupported(format!(
                "layvpi.no_viewport: layout {} has no viewport; LAYVPI freezes layers per \
                 viewport and there is no viewport to receive the override",
                layout_id.0
            )));
        }
        let viewport_count = viewports.len();
        // The current viewport is the session `CVPORT` marker. The no-argument
        // form needs it to exclude it; the explicit form still validates it so a
        // stale marker is reported rather than silently ignored.
        let current = target_viewport(
            layout_id,
            only,
            session.viewport_marker(layout_id),
            viewport_count,
            "layvpi",
        )?;
        let scope = match only {
            Some(index) => Scope::Only(index),
            None => {
                if viewport_count == 1 {
                    return Err(CadError::Unsupported(format!(
                        "layvpi.no_other_viewport: layout {} has a single viewport and it is the \
                         current one (CVPORT {}), so \"every viewport except the current one\" is \
                         empty; use the explicit viewport-index form to freeze the current \
                         viewport itself, or add another viewport first",
                        layout_id.0,
                        current + 1
                    )));
                }
                Scope::EveryExcept(current)
            }
        };
        let mut changed = false;
        for (index, viewport) in viewports.iter_mut().enumerate() {
            if !scope.targets(index) {
                continue;
            }
            for (id, _) in &frozen {
                if viewport.layer_overrides.get(id) != Some(&true) {
                    viewport.layer_overrides.insert(*id, true);
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layvpi.unchanged".into(),
                    message: format!(
                        "LAYVPI: {} already freezes the named layer(s); no transaction was opened",
                        scope.describe(layout_id)
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
            candidate.set_layout_viewports(layout_id, viewports, transaction, "layvpi")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "layvpi",
            vec![layout_patch(layout_id, before, after)],
        )?;
        let names = frozen
            .iter()
            .map(|(_, name)| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layvpi.frozen".into(),
                message: match scope {
                    Scope::Only(index) => format!(
                        "LAYVPI: froze layer(s) {names} in viewport {index} of layout {} (position \
                         {} of {viewport_count}; the explicit index overrides the CVPORT marker and \
                         the global layer table is unchanged)",
                        layout_id.0,
                        index + 1
                    ),
                    Scope::EveryExcept(current) => format!(
                        "LAYVPI: froze layer(s) {names} in every viewport of layout {} except the \
                         current one (CVPORT {}, zero-based index {current}): {} of {viewport_count} \
                         viewport(s) received the override, the current viewport and the global \
                         layer table are unchanged",
                        layout_id.0,
                        current + 1,
                        viewport_count - 1
                    ),
                },
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Which viewports one `LAYVPI` write touches.
#[derive(Debug, Clone, Copy)]
enum Scope {
    /// Exactly one viewport (the explicit-index form).
    Only(usize),
    /// Every viewport but this one (AutoCAD's no-argument default).
    EveryExcept(usize),
}

impl Scope {
    /// Whether `index` receives the override.
    fn targets(self, index: usize) -> bool {
        match self {
            Self::Only(wanted) => index == wanted,
            Self::EveryExcept(current) => index != current,
        }
    }

    /// The scope sentence the `layvpi.unchanged` report uses.
    fn describe(self, layout: LayoutId) -> String {
        match self {
            Self::Only(index) => format!("viewport {index} of layout {}", layout.0),
            Self::EveryExcept(current) => format!(
                "every viewport of layout {} except the current one (CVPORT {})",
                layout.0,
                current + 1
            ),
        }
    }
}
