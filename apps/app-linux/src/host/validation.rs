//! Acceptance runs the Linux host, not a test-only sink or an alternate UI.
//!
//! The acceptance path drives the offscreen software-Vulkan platform, which is a
//! Linux/Windows capability only; on macOS the module is compiled out and
//! `--headless` is rejected in `entry.rs`.
#![cfg(any(target_os = "linux", target_os = "windows"))]

use super::*;
use cad_render_wgpu::headless::{encode_png, RgbaImage};

impl LinuxApp {
    fn snapshot(&self) -> CadResult<RgbaImage> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let frame = loop {
            self.runtime.metrics()?;
            self.runtime.sync_view()?;
            let frame = cad_ui_slint::offscreen::snapshot(self.adapter.window())
                .map_err(|e| CadError::Unsupported(e.to_string()))?;
            if self
                .runtime
                .view
                .borrow()
                .as_ref()
                .is_some_and(CadView::current_drawing_presented)
            {
                break frame;
            }
            if std::time::Instant::now() >= deadline {
                return Err(CadError::Unsupported(
                    "CAD preparation/presentation timed out".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let view = self.runtime.view.borrow();
        let view = view.as_ref().ok_or(CadError::Cancelled)?;
        if let Some(error) = view.last_error() {
            return Err(CadError::Unsupported(error));
        }
        if view.frames_rendered() == 0 {
            return Err(CadError::Unsupported("CAD did not render".into()));
        }
        Ok(frame)
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
        let mut last_tick = if open_after_start {
            None
        } else {
            Some(started)
        };
        let mut max_gap = Duration::ZERO;
        let mut open_requested = false;
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
            if probe.runtime.controller.borrow().last_import_report.is_none()
                || !probe.runtime.view.borrow().as_ref().unwrap().current_drawing_presented() {
                assert!(started.elapsed() < Duration::from_secs(60), "new drawing never rendered");
                return;
            }
            last_tick = Some(now);
            let tick = tick_count.get() + 1;
            tick_count.set(tick);
            if tick == 1 {
                let progress = probe.runtime.view.borrow().as_ref().unwrap().drawing_progress().expect("progressive drawing progress is required");
                assert_eq!(progress.0, progress.1, "a truncated prefix must not be marked presented");
                eprintln!("desktop complete drawing batches={progress:?}");
                eprintln!("desktop drawing bounds={:?}, camera={:?}, loading max_gap={max_gap:?}", probe.runtime.controller.borrow().drawing().unwrap().bounds(), probe.runtime.view.borrow().as_ref().unwrap().camera3d());
                if open_after_start {
                    eprintln!("desktop current DWG presented: elapsed={:?}", started.elapsed());
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
                if let Ok(directory) = std::env::var("YACR_DESKTOP_EVIDENCE") {
                    let directory = PathBuf::from(directory);
                    std::fs::create_dir(&directory).expect("evidence directory must be new");
                    std::fs::write(directory.join("initial.png"), encode_png(before.as_ref().unwrap()).unwrap()).unwrap();
                }
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

    /// Copy the CAD-canvas pixels (RGBA bytes) inside the logical `rect`.
    fn canvas_region(image: &RgbaImage, rect: [f64; 4], scale: f64) -> Vec<u8> {
        let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
        let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
        let x1 = ((rect[0] + rect[2]) * scale)
            .floor()
            .min(image.width as f64) as u32;
        let y1 = ((rect[1] + rect[3]) * scale)
            .floor()
            .min(image.height as f64) as u32;
        let mut out = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let index = ((y * image.width + x) * 4) as usize;
                out.extend_from_slice(&image.pixels[index..index + 4]);
            }
        }
        out
    }

    /// Number of RGBA pixels that differ between two equal-length regions.
    fn changed_region(before: &[u8], after: &[u8]) -> usize {
        let (before_chunks, _) = before.as_chunks::<4>();
        let (after_chunks, _) = after.as_chunks::<4>();
        before_chunks
            .iter()
            .zip(after_chunks)
            .filter(|(a, b)| a != b)
            .count()
    }

    /// Real-GPU end-to-end acceptance for the three reported desktop defects, on
    /// the real winit window + hardware GPU (never the lavapipe/offscreen path):
    ///
    /// 1. the status-bar grid toggle must actually reach the renderer;
    /// 2. wheel-up must zoom in (smaller world-per-pixel);
    /// 3. the origin axes must render (checked visually via the PNG evidence).
    ///
    /// Evidence PNGs are written under `YACR_REAL_E2E_OUTPUT` (default
    /// `/tmp/opencode/yacr-real-gpu`). Set `YACR_TEST_DWG` to open a specific
    /// drawing; otherwise the committed flange DXF fixture is used.
    #[test]
    #[ignore = "requires a real display + GPU; run in a dedicated process"]
    #[allow(unused_assignments)]
    fn real_gpu_grid_zoom_axes_acceptance() {
        use std::cell::Cell;
        use std::time::Duration;

        if std::env::var("WAYLAND_DISPLAY").is_err() && std::env::var("DISPLAY").is_err() {
            panic!("no display available (WAYLAND_DISPLAY/DISPLAY unset)");
        }
        let path = std::env::var("YACR_TEST_DWG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("fixtures/dxf/qcad-flange/flange.dxf")
            });
        let out = std::env::var("YACR_REAL_E2E_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/opencode/yacr-real-gpu"));
        std::fs::create_dir_all(&out).unwrap();

        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(
            LinuxApp::new(
                LinuxOptions::parse(["--open".into(), path.display().to_string()]).unwrap(),
            )
            .unwrap(),
        );

        let ticks = Rc::new(Cell::new(0u32));
        let timer = slint::Timer::default();
        let probe = app.clone();
        let tick_counter = ticks.clone();
        let mut before_grid: Option<Vec<u8>> = None;
        let mut grid_changed = 0usize;
        let mut zoom_before = 0.0;
        let mut zoom_after = 0.0;

        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                {
                    let view = probe.runtime.view.borrow();
                    let Some(view) = view.as_ref() else { return };
                    if view.frames_rendered() == 0 || !view.current_drawing_presented() {
                        return;
                    }
                }
                let tick = tick_counter.get() + 1;
                tick_counter.set(tick);
                let shot = || -> RgbaImage {
                    let pixels = probe.adapter.window().take_snapshot().unwrap();
                    RgbaImage {
                        width: pixels.width(),
                        height: pixels.height(),
                        pixels: pixels.as_bytes().to_vec(),
                    }
                };
                match tick {
                    3 => {
                        let image = shot();
                        std::fs::write(
                            out.join("01-axes-initial.png"),
                            encode_png(&image).unwrap(),
                        )
                        .unwrap();
                        let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                        let scale = probe.adapter.window().scale_factor() as f64;
                        before_grid = Some(canvas_region(&image, rect, scale));
                        probe
                            .adapter
                            .component()
                            .invoke_overlay_toggled("grid".into(), true);
                    }
                    8 => {
                        let image = shot();
                        std::fs::write(out.join("02-grid-on.png"), encode_png(&image).unwrap())
                            .unwrap();
                        let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                        let scale = probe.adapter.window().scale_factor() as f64;
                        grid_changed = changed_region(
                            before_grid.as_ref().unwrap(),
                            &canvas_region(&image, rect, scale),
                        );
                        {
                            let c = probe.runtime.controller.borrow();
                            zoom_before =
                                c.application.workspace.viewports[&c.viewport_id].world_per_px();
                        }
                        let center = slint::LogicalPosition::new(
                            (rect[0] + rect[2] * 0.5) as f32,
                            (rect[1] + rect[3] * 0.5) as f32,
                        );
                        probe.adapter.window().dispatch_event(
                            slint::platform::WindowEvent::PointerScrolled {
                                position: center,
                                delta_x: 0.0,
                                delta_y: 120.0, // wheel up
                            },
                        );
                    }
                    12 => {
                        {
                            let c = probe.runtime.controller.borrow();
                            zoom_after =
                                c.application.workspace.viewports[&c.viewport_id].world_per_px();
                        }
                        let image = shot();
                        std::fs::write(out.join("03-wheel-up.png"), encode_png(&image).unwrap())
                            .unwrap();
                        let backend = probe
                            .runtime
                            .view
                            .borrow()
                            .as_ref()
                            .map(|v| v.backend_label())
                            .unwrap_or_default();
                        assert!(
                            grid_changed > 0,
                            "the grid toggle did not change the canvas on the real GPU"
                        );
                        assert!(
                            zoom_after < zoom_before,
                            "wheel-up must zoom in: world_per_px {zoom_before} -> {zoom_after}"
                        );
                        eprintln!(
                            "real GPU E2E: backend={backend:?} grid_changed_pixels={grid_changed} \
                             zoom world_per_px {zoom_before} -> {zoom_after}"
                        );
                        slint::quit_event_loop().unwrap();
                    }
                    _ => {}
                }
            },
        );
        app.run().unwrap();
        assert!(
            ticks.get() >= 12,
            "the real-GPU acceptance probe did not run to completion"
        );
    }
}
