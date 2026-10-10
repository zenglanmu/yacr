//! Mouse navigation and tool capture use authoritative controller metrics and commands.
use super::*;
use cad_ui_slint::{CanvasPickMapper, DrawCommandSink, DrawPreviewSink, ViewInput};
use std::cell::Cell;

pub(super) struct Navigation {
    runtime: Runtime,
    last: Cell<Option<[f64; 2]>>,
    dragged: Cell<bool>,
}
impl Navigation {
    pub(super) fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            last: Cell::new(None),
            dragged: Cell::new(false),
        }
    }
    fn report(&self, result: CadResult<()>) {
        if let Err(error) = result {
            if let Some(handle) = self.runtime.handle.borrow().as_ref() {
                let _ = handle.set_status(
                    self.runtime
                        .message("linux.failed", &[("error", &error.to_string())]),
                );
            }
        }
    }
    fn select(&self, point: [f64; 2]) -> CadResult<()> {
        self.runtime.metrics()?;
        let c = self.runtime.controller.borrow();
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
        self.runtime
            .command(CommandId::Select, CommandPayload::Selection(refs))
    }
}
impl ViewInput for Navigation {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        let point = [x, y];
        match kind {
            0 if button == 1 || button == 3 => {
                self.last.set(Some(point));
                self.dragged.set(false);
            }
            2 => {
                if let Some(last) = self.last.get() {
                    if (x - last[0]).hypot(y - last[1]) >= 3.0 || self.dragged.get() {
                        self.dragged.set(true);
                        let c = self.runtime.controller.borrow();
                        let wpp = c.application.workspace.viewports[&c.viewport_id].world_per_px();
                        drop(c);
                        self.report(self.runtime.command(
                            CommandId::Pan,
                            CommandPayload::Points(vec![Point3 {
                                x: (x - last[0]) * wpp,
                                y: -(y - last[1]) * wpp,
                                z: 0.0,
                            }]),
                        ));
                        self.last.set(Some(point));
                    }
                }
            }
            1 => {
                if self.last.take().is_some() && !self.dragged.get() && button == 1 {
                    self.report(self.select(point));
                }
            }
            3 => {
                self.last.set(None);
                self.dragged.set(false);
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
        self.report(self.runtime.command(
            CommandId::Zoom,
            CommandPayload::Points(vec![Point3 {
                x: (1.0 + dy * 0.0015).clamp(0.2, 5.0),
                y: 0.0,
                z: 0.0,
            }]),
        ));
    }
}
impl CanvasPickMapper for Runtime {
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
impl DrawPreviewSink for Runtime {
    fn set_preview(&self, preview: Option<cad_app::DrawPreview>) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.set_draw_preview(preview);
        }
    }
}
impl DrawCommandSink for Runtime {
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
            cad_app::DrawIntent::Move { delta } => {
                let refs = self.controller.borrow().selection().refs().to_vec();
                self.command(
                    CommandId::MoveEntities,
                    CommandPayload::Move { refs, delta },
                )
            }
            cad_app::DrawIntent::Trim { .. } => {
                Err(CadError::Unsupported(self.message("linux.trim", &[])))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use slint::ComponentHandle;

    fn world_per_px(app: &crate::LinuxApp) -> f64 {
        let c = app.runtime.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].world_per_px()
    }

    /// The desktop wheel comes from Slint's winit backend, whose positive
    /// `delta_y` is a wheel-up rotation (winit documents positive as content
    /// moving down / revealing content above). Wheel-up must therefore zoom in
    /// (a smaller world-per-pixel), matching every other host's "up = closer".
    #[test]
    fn desktop_wheel_up_zooms_in() {
        cad_ui_slint::offscreen::install().unwrap();
        let app = crate::LinuxApp::new(crate::LinuxOptions {
            headless: true,
            ..Default::default()
        })
        .unwrap();
        app.adapter.component().show().unwrap();
        let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
        let center = slint::LogicalPosition::new(
            (rect[0] + rect[2] * 0.5) as f32,
            (rect[1] + rect[3] * 0.5) as f32,
        );

        let before = world_per_px(&app);
        app.adapter
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: center,
                delta_x: 0.0,
                delta_y: 120.0, // wheel up
            });
        let after_up = world_per_px(&app);
        assert!(
            after_up < before,
            "wheel up must zoom in: {before} -> {after_up}"
        );

        app.adapter
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: center,
                delta_x: 0.0,
                delta_y: -120.0, // wheel down
            });
        let after_down = world_per_px(&app);
        assert!(
            after_down > after_up,
            "wheel down must zoom out: {after_up} -> {after_down}"
        );
    }
}
