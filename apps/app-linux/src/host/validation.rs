//! Acceptance runs the Linux host, not a test-only sink or an alternate UI.
use super::*;
use cad_render_wgpu::headless::{encode_png, RgbaImage};

impl LinuxApp {
    fn snapshot(&self) -> CadResult<RgbaImage> {
        let mut frame = None;
        for _ in 0..4 {
            self.runtime.metrics()?;
            self.runtime.sync_view()?;
            frame = Some(
                cad_ui_slint::offscreen::snapshot(self.adapter.window())
                    .map_err(|e| CadError::Unsupported(e.to_string()))?,
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let view = self.runtime.view.borrow();
        let view = view.as_ref().ok_or(CadError::Cancelled)?;
        if let Some(error) = view.last_error() {
            return Err(CadError::Unsupported(error));
        }
        if view.frames_rendered() == 0 {
            return Err(CadError::Unsupported("CAD did not render".into()));
        }
        frame.ok_or(CadError::Cancelled)
    }
    pub fn acceptance(&self, output: &Path) -> CadResult<()> {
        if !self.runtime.options.headless {
            return Err(CadError::Unsupported(
                "acceptance requires headless software-Vulkan mode".into(),
            ));
        }
        // Refuse existing evidence directories; failures are retained for diagnosis.
        std::fs::create_dir(output).map_err(|e| CadError::InvalidInput(e.to_string()))?;
        self.adapter
            .component()
            .show()
            .map_err(|e| CadError::Unsupported(e.to_string()))?;
        let before = self.snapshot()?;
        std::fs::write(
            output.join("initial.png"),
            encode_png(&before).map_err(CadError::InvalidInput)?,
        )
        .map_err(|e| CadError::InvalidInput(e.to_string()))?;
        let initial_camera = self
            .runtime
            .controller
            .borrow()
            .application
            .workspace
            .viewports[&self.runtime.controller.borrow().viewport_id]
            .camera;
        let (rect, _) = self.adapter.handle().shell_geometry()?;
        let position = slint::LogicalPosition::new(
            (rect[0] + rect[2] * 0.5) as f32,
            (rect[1] + rect[3] * 0.5) as f32,
        );
        self.adapter
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: -120.0,
            });
        let after = self.snapshot()?;
        let c = self.runtime.controller.borrow();
        let camera = c.application.workspace.viewports[&c.viewport_id].camera;
        let scale = self.adapter.window().scale_factor() as f64;
        let changed_pixels = changed_cad_pixels(&before, &after, rect, scale);
        if camera == initial_camera || changed_pixels == 0 {
            return Err(CadError::InvalidInput(
                "Linux scroll did not change camera and pixels".into(),
            ));
        }
        drop(c);
        std::fs::write(
            output.join("navigation.png"),
            encode_png(&after).map_err(CadError::InvalidInput)?,
        )
        .map_err(|e| CadError::InvalidInput(e.to_string()))?;
        let view = self.runtime.view.borrow();
        let view = view.as_ref().ok_or(CadError::Cancelled)?;
        let report = serde_json::json!({ "schemaVersion": 1, "host": "app-linux", "softwareGpu": true,
            "source": if self.runtime.options.drawing.is_some() { "external-drawing" } else { "synthetic" },
            "size": [before.width,before.height], "cadRect": rect, "cadFrames": view.frames_rendered(),
            "backend": view.backend_label(), "renderError": view.last_error(),
            "navigationCameraChanged": true, "navigationPixelsChanged": true, "changedCadPixels": changed_pixels,
            "visualAcceptance": "manual-review-required", "desktopWindow": "not-run" });
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&report)
                .map_err(|e| CadError::InvalidInput(e.to_string()))?,
        )
        .map_err(|e| CadError::InvalidInput(e.to_string()))?;
        println!("{}", report);
        Ok(())
    }
}

fn changed_cad_pixels(before: &RgbaImage, after: &RgbaImage, rect: [f64; 4], scale: f64) -> usize {
    if before.width != after.width || before.height != after.height {
        return 0;
    }
    let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
    let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
    let x1 = ((rect[0] + rect[2]) * scale)
        .floor()
        .min(before.width as f64) as u32;
    let y1 = ((rect[1] + rect[3]) * scale)
        .floor()
        .min(before.height as f64) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| before.pixel(x, y) != after.pixel(x, y))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a desktop display/GPU and YACR_TEST_DWG; run in a dedicated process"]
    fn desktop_dwg_event_loop_and_navigation_remain_responsive() {
        desktop_dwg_probe(false);
    }

    #[test]
    #[ignore = "requires a desktop display/GPU and YACR_TEST_DWG; run in a dedicated process"]
    fn desktop_dwg_open_after_start_and_interaction_remain_responsive() {
        desktop_dwg_probe(true);
    }

    #[test]
    #[ignore = "requires YACR_TEST_DWG; profiles real import/fit/CPU preparation without a GPU"]
    fn real_dwg_cpu_load_stages() {
        use std::time::Instant;
        let path = std::env::var("YACR_TEST_DWG").expect("YACR_TEST_DWG is required");
        let bytes = Arc::from(std::fs::read(&path).unwrap());
        let mut controller = HostController::with_demo_document([1280.0, 800.0]).unwrap();
        let started = Instant::now();
        controller.open_bytes(bytes, "external-drawing").unwrap();
        let imported = Instant::now();
        controller.fit().unwrap();
        let fitted = Instant::now();
        let drawing = controller.drawing().unwrap();
        let scene = cad_app::render_scene::build_scene_with_overrides(
            &drawing,
            cad_domain::TaskStamp::new(controller.document_id, 0),
            None,
            &controller.session.layer_overrides,
        )
        .unwrap();
        eprintln!("DWG CPU stages: import={:?}, fit={:?}, scene={:?}, entities={}, batches={}, vertices={}", imported.duration_since(started), fitted.duration_since(imported), fitted.elapsed(), drawing.entities().count(), scene.added.len(), scene.added.iter().map(|b| b.vertices.len()).sum::<usize>());
        assert!(!scene.added.is_empty());
    }

    fn desktop_dwg_probe(open_after_start: bool) {
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        let path = std::env::var("YACR_TEST_DWG").expect("YACR_TEST_DWG is required");
        cad_ui_slint::select_wgpu_backend().unwrap();
        let started = Instant::now();
        let app = if open_after_start {
            let path = PathBuf::from(path);
            Rc::new(
                LinuxApp::new_with_file_picker(
                    LinuxOptions::parse([]).unwrap(),
                    Arc::new(move || Ok(path.clone())),
                )
                .unwrap(),
            )
        } else {
            Rc::new(LinuxApp::new(LinuxOptions::parse(["--open".into(), path]).unwrap()).unwrap())
        };
        let ticks = Rc::new(Cell::new(0));
        let timer = slint::Timer::default();
        let tick_count = ticks.clone();
        let probe = app.clone();
        let mut before = None;
        let mut initial_camera = None;
        let mut last_tick = None;
        let mut max_gap = Duration::ZERO;
        let mut open_requested = false;
        let mut demo_frames = 0;
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            let now = Instant::now();
            // Once opening begins, count every callback gap, including import
            // and CPU preparation before a new CAD frame is available.
            if let Some(last) = last_tick {
                let gap = now.duration_since(last);
                max_gap = max_gap.max(gap);
                assert!(gap < Duration::from_secs(2), "desktop event loop stalled: tick={}, gap={gap:?}", tick_count.get());
                last_tick = Some(now);
            }
            if probe.runtime.view.borrow().as_ref().unwrap().frames_rendered() == 0 {
                assert!(started.elapsed() < Duration::from_secs(60), "drawing never rendered");
                return;
            }
            if open_after_start && !open_requested {
                demo_frames = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                eprintln!("desktop DWG opening after startup: elapsed={:?}", started.elapsed());
                last_tick = Some(now);
                probe.adapter.component().invoke_open_requested();
                open_requested = true;
                last_tick = Some(now);
                return;
            }
            // Loading is part of the post-start responsiveness contract; measure
            // heartbeat gaps even before a new drawing frame is available.
            if open_after_start {
                if let Some(last) = last_tick {
                    let gap = now.duration_since(last);
                    assert!(gap < Duration::from_secs(2), "desktop loading stalled: gap={gap:?}");
                }
                last_tick = Some(now);
            }
            if open_after_start && (probe.runtime.controller.borrow().last_import_report.is_none()
                || probe.runtime.view.borrow().as_ref().unwrap().frames_rendered() <= demo_frames) {
                assert!(started.elapsed() < Duration::from_secs(60), "new drawing never rendered");
                return;
            }
            last_tick = Some(now);
            let tick = tick_count.get() + 1;
            tick_count.set(tick);
            if tick == 1 {
                if open_after_start {
                    eprintln!("desktop post-open probe began: elapsed={:?}; frame-count gate is not new-scene freshness evidence", started.elapsed());
                } else {
                if open_after_start {
                    eprintln!("desktop open heartbeat (not new-scene readiness): elapsed={:?}", started.elapsed());
                } else {
                    eprintln!("desktop DWG ready: elapsed={:?}", started.elapsed());
                }
                }
            }
            if tick == 10 {
                {
                    let view = probe.runtime.view.borrow();
                    let view = view.as_ref().unwrap();
                    assert!(view.frames_rendered() > 0, "drawing never rendered");
                    assert!(view.last_error().is_none(), "GPU initialization failed");
                    initial_camera = Some(view.camera3d());
                }
                let pixels = probe.adapter.window().take_snapshot().unwrap();
                before = Some(RgbaImage {
                    width: pixels.width(), height: pixels.height(), pixels: pixels.as_bytes().to_vec(),
                });
                let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                // Exclude snapshot readback, but include synchronous input
                // handling and the following CAD redraw in the next gap.
                last_tick = Some(Instant::now());
                probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new((rect[0] + rect[2] * 0.5) as f32, (rect[1] + rect[3] * 0.5) as f32),
                    delta_x: 0.0, delta_y: -120.0,
                });
            }
            if tick == 20 {
                let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                let position = slint::LogicalPosition::new(
                    (rect[0] + rect[2] * 0.5) as f32,
                    (rect[1] + rect[3] * 0.5) as f32,
                );
                let interaction_started = Instant::now();
                eprintln!("desktop DWG interaction: move and select started");
                probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
                probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerPressed {
                    position, button: slint::platform::PointerEventButton::Left,
                });
                probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerReleased {
                    position, button: slint::platform::PointerEventButton::Left,
                });
                eprintln!("desktop DWG interaction: move and select elapsed={:?}", interaction_started.elapsed());
                assert!(interaction_started.elapsed() < Duration::from_secs(2), "desktop selection stalled");
            }
            if tick == 30 {
                let pixels = probe.adapter.window().take_snapshot().unwrap();
                let after = RgbaImage {
                    width: pixels.width(), height: pixels.height(), pixels: pixels.as_bytes().to_vec(),
                };
                let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                let changed = changed_cad_pixels(before.as_ref().unwrap(), &after, rect, probe.adapter.window().scale_factor() as f64);
                let view = probe.runtime.view.borrow();
                let view = view.as_ref().unwrap();
                assert_ne!(Some(view.camera3d()), initial_camera);
                assert!(changed > 0, "navigation did not change CAD pixels");
                assert!(view.frames_rendered() >= 2);
                assert!(view.view_diagnostic().is_none(), "{:?}", view.view_diagnostic());
                eprintln!("desktop DWG probe: elapsed={:?}, ticks={tick}, max event gap={max_gap:?}, CAD frames={}, changed CAD pixels={changed}", started.elapsed(), view.frames_rendered());
                slint::quit_event_loop().unwrap();
            }
        });
        app.run().unwrap();
        assert_eq!(ticks.get(), 30);
    }

    #[test]
    fn chrome_only_changes_cannot_pass_cad_navigation() {
        let before = RgbaImage {
            width: 4,
            height: 4,
            pixels: vec![0; 64],
        };
        let mut after = before.clone();
        after.pixels[0] = 255;
        assert_eq!(
            changed_cad_pixels(&before, &after, [1.0, 1.0, 2.0, 2.0], 1.0),
            0
        );
        after.pixels[20] = 255;
        assert_eq!(
            changed_cad_pixels(&before, &after, [1.0, 1.0, 2.0, 2.0], 1.0),
            1
        );
    }
}
