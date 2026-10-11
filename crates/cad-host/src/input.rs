//! Mouse navigation and tool capture use authoritative controller metrics and
//! commands. The core is the shell's `ViewInput`, `CanvasPickMapper`,
//! `DrawCommandSink` and `DrawPreviewSink`.
use super::*;
use cad_domain::Point3;
use cad_ui_slint::{CanvasPickMapper, DrawCommandSink, DrawPreviewSink, ViewInput};

impl HostCore {
    /// Report a navigation command's failure on the status line.
    fn report_nav(&self, result: CadResult<()>) {
        if let Err(error) = result {
            if let Some(handle) = self.handle.borrow().as_ref() {
                let _ = handle
                    .set_status(self.message("command.failed", &[("error", &error_text(&error))]));
            }
        }
    }

    /// Pick the entity under `point` and select it (repeatable).
    fn select_at(&self, point: [f64; 2]) -> CadResult<()> {
        self.metrics()?;
        let c = self.controller.borrow();
        // `Select` itself moves the session to `ToolState::Selecting`, which is a
        // normal, repeatable selection state: a later click must be able to
        // replace the selection. Only a measurement/annotation/pan tool blocks a
        // pick; treating `Selecting` as busy made the very first click permanent.
        if !matches!(
            c.session.tool,
            cad_app::ToolState::Idle | cad_app::ToolState::Selecting
        ) {
            return Ok(());
        }
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        let report = cad_app::picking::pick_at_screen(
            &drawing,
            c.document_id,
            &viewport.camera,
            point,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
        )?;
        let refs = report.hit.map(|h| vec![h.source]).unwrap_or_default();
        drop(c);
        self.command(CommandId::Select, CommandPayload::Selection(refs))
    }
}

impl ViewInput for HostCore {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        let point = [x, y];
        let nav = &self.navigation;
        match kind {
            0 if button == 1 || button == 3 => {
                nav.last.set(Some(point));
                nav.dragged.set(false);
            }
            2 => {
                if let Some(last) = nav.last.get() {
                    if (x - last[0]).hypot(y - last[1]) >= 3.0 || nav.dragged.get() {
                        nav.dragged.set(true);
                        let c = self.controller.borrow();
                        let wpp = c.application.workspace.viewports[&c.viewport_id].world_per_px();
                        drop(c);
                        self.report_nav(self.command(
                            CommandId::Pan,
                            CommandPayload::Points(vec![Point3 {
                                x: (x - last[0]) * wpp,
                                y: -(y - last[1]) * wpp,
                                z: 0.0,
                            }]),
                        ));
                        nav.last.set(Some(point));
                    }
                }
            }
            1 => {
                if nav.last.take().is_some() && !nav.dragged.get() && button == 1 {
                    self.report_nav(self.select_at(point));
                }
            }
            3 => {
                nav.last.set(None);
                nav.dragged.set(false);
            }
            _ => {}
        }
    }

    fn scroll(&self, _dx: f64, dy: f64) {
        // The desktop wheel arrives through Slint's winit backend, where a
        // positive `delta_y` is a wheel-up rotation (winit: positive means the
        // content moves down, revealing content above). Wheel-up must zoom in
        // (`Camera::zoom_at`: factor > 1 zooms in), so the sign is `+ dy`. This
        // is deliberately not the shared web/Android `1 - dy * k` (those hosts
        // deliver browser/gesture deltas whose positive axis points down).
        self.report_nav(self.command(
            CommandId::Zoom,
            CommandPayload::Points(vec![Point3 {
                x: (1.0 + dy * 0.0015).clamp(0.2, 5.0),
                y: 0.0,
                z: 0.0,
            }]),
        ));
    }
}

impl CanvasPickMapper for HostCore {
    fn to_world(&self, logical: [f64; 2]) -> Option<Point3> {
        self.metrics().ok()?;
        let c = self.controller.borrow();
        let vp = c.application.workspace.viewports.get(&c.viewport_id)?;
        cad_app::input::ViewMetrics::new(
            cad_app::input::CanvasMetrics::new([0.0; 2], vp.logical_size, vp.dpi_scale),
            vp,
        )
        .surface_to_world(logical)
    }
}

impl DrawPreviewSink for HostCore {
    fn set_preview(&self, preview: Option<cad_app::DrawPreview>) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.set_draw_preview(preview);
        }
    }
}

impl DrawCommandSink for HostCore {
    fn commit(&mut self, intent: cad_app::DrawIntent) -> CadResult<()> {
        match intent {
            cad_app::DrawIntent::Line { start, end } => self.command(
                CommandId::CreateLine,
                CommandPayload::Points(vec![start, end]),
            ),
            cad_app::DrawIntent::Circle { center, edge } => self.command(
                CommandId::CreateCircle,
                CommandPayload::Points(vec![center, edge]),
            ),
            cad_app::DrawIntent::Arc { points } => self.command(
                CommandId::CreateArc,
                CommandPayload::Points(points.to_vec()),
            ),
            cad_app::DrawIntent::Move { delta } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::MoveEntities,
                    CommandPayload::Move { refs, delta },
                )
            }
            cad_app::DrawIntent::Copy { delta } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::CopyEntities,
                    CommandPayload::Copy { refs, delta },
                )
            }
            cad_app::DrawIntent::Rotate { base, angle } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::RotateEntities,
                    CommandPayload::Rotate { refs, base, angle },
                )
            }
            cad_app::DrawIntent::Scale { base, factor } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::ScaleEntities,
                    CommandPayload::Scale { refs, base, factor },
                )
            }
            cad_app::DrawIntent::Mirror { p1, p2 } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::MirrorEntities,
                    CommandPayload::Mirror {
                        refs,
                        axis_start: p1,
                        axis_end: p2,
                    },
                )
            }
            cad_app::DrawIntent::Offset { side_point } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::OffsetEntity,
                    CommandPayload::Offset { refs, side_point },
                )
            }
            cad_app::DrawIntent::Trim { .. } => {
                Err(CadError::Unsupported(self.message("linux.trim", &[])))
            }
            cad_app::DrawIntent::Extend { .. } => {
                Err(CadError::Unsupported(self.message("linux.extend", &[])))
            }
        }
    }
}
