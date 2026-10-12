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
            let _ =
                self.set_status(self.message("command.failed", &[("error", &error_text(&error))]));
        }
    }

    /// Pick the entity under `point` and select it (repeatable).
    fn select_at(&self, point: [f64; 2]) -> CadResult<()> {
        let profile = super::pick_profile_enabled();
        let started = profile.then(std::time::Instant::now);
        let metrics_result = self.metrics();
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK select_metrics elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        metrics_result?;
        let started = profile.then(std::time::Instant::now);
        let c = self.controller.borrow();
        // `Select` itself moves the session to `ToolState::Selecting`, which is a
        // normal, repeatable selection state: a later click must be able to
        // replace the selection. Only a measurement/annotation/pan tool blocks a
        // pick; treating `Selecting` as busy made the very first click permanent.
        if !matches!(
            c.session.tool,
            cad_app::ToolState::Idle | cad_app::ToolState::Selecting
        ) {
            if let Some(started) = started {
                eprintln!(
                    "YACR_PROFILE_PICK select_context elapsed_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            return Ok(());
        }
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK select_context elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
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
        let started = profile.then(std::time::Instant::now);
        let result = self.command(CommandId::Select, CommandPayload::Selection(refs));
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK select_command elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        result
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

    fn scroll(&self, _dx: f64, dy: f64, cursor: [f64; 2]) {
        // The desktop wheel arrives through Slint's winit backend, where a
        // positive `delta_y` is a wheel-up rotation (winit: positive means the
        // content moves down, revealing content above). Wheel-up must zoom in
        // (`Camera::zoom_at`: factor > 1 zooms in), so the sign is `+ dy`. This
        // is deliberately not the shared web/Android `1 - dy * k` (those hosts
        // deliver browser/gesture deltas whose positive axis points down).
        self.report_nav(self.command(
            CommandId::Zoom,
            CommandPayload::ZoomAt {
                factor: (1.0 + dy * 0.0015).clamp(0.2, 5.0),
                cursor,
            },
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
    fn begin(&mut self, kind: cad_app::DrawToolKind) {
        let selection = if kind == cad_app::DrawToolKind::Move {
            Some(self.controller.borrow().selection().refs().to_vec())
        } else {
            None
        };
        *self.move_selection.borrow_mut() = selection;
    }

    fn cancel(&mut self) {
        self.move_selection.borrow_mut().take();
    }

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
                let result = self.execute(self.move_command(delta)?);
                if result.is_ok() {
                    self.move_selection.borrow_mut().take();
                }
                result
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

impl HostCore {
    /// Build the confirmed MOVE command from the active tool's captured refs.
    /// Kept separate from execution so the payload contract can be tested
    /// without constructing a window or renderer.
    pub(crate) fn move_command(&self, delta: Point3) -> CadResult<Command> {
        let refs = self
            .move_selection
            .borrow()
            .clone()
            .ok_or_else(|| CadError::InvalidInput("MOVE selection was not captured".into()))?;
        let c = self.controller.borrow();
        Ok(Command {
            schema_version: 1,
            id: CommandId::MoveEntities,
            document: c.document_id,
            viewport: c.viewport_id,
            payload: CommandPayload::Move { refs, delta },
        })
    }
}

#[cfg(test)]
mod wheel_anchor_tests {
    use super::*;
    use cad_platform::{
        ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles,
        SavePurpose, SurfaceMetrics, SurfaceSource,
    };

    struct WheelFiles;
    impl PlatformFiles for WheelFiles {
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
        fn set_source(&self, _target: Option<&FileTarget>) {}
        fn poll(&self) -> Vec<FileEvent> {
            Vec::new()
        }
    }

    struct WheelConfig;
    impl ConfigStore for WheelConfig {
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

    struct WheelSurface;
    impl SurfaceSource for WheelSurface {
        fn surface(&self) -> Option<SurfaceMetrics> {
            Some(SurfaceMetrics {
                logical: [800.0, 600.0],
                scale: 1.0,
                safe_insets: [0.0; 4],
            })
        }
    }

    fn core() -> HostCore {
        let options = HostOptions {
            fonts: Vec::new(),
            fonts_dir: None,
            images_dir: None,
            headless: false,
            drawing: None,
            size: [800.0, 600.0],
            locale: "en".into(),
        };
        let platform = HostPlatform {
            files: Rc::new(WheelFiles),
            config_store: Rc::new(WheelConfig),
            surface: Rc::new(WheelSurface),
            resources: Rc::new(UnsupportedResources),
            capabilities: HostCapabilities {
                open: true,
                new: true,
                save: true,
                plot: false,
                import_worker: false,
            },
        };
        HostCore::new(options, Rc::new(RefCell::new(None)), platform).unwrap()
    }

    #[test]
    fn desktop_wheel_uses_off_center_canvas_cursor_and_preserves_legacy_sign() {
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
        HostCore::scroll(&core, 0.0, 200.0, cursor);
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
}
