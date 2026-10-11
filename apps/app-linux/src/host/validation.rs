//! Acceptance runs the Linux host, not a test-only sink or an alternate UI.
//!
//! The acceptance path drives the offscreen software-Vulkan platform, which is a
//! Linux/Windows capability only; on macOS the module is compiled out and
//! `--headless` is rejected in `entry.rs`.
#![cfg(any(target_os = "linux", target_os = "windows"))]

use super::*;
use cad_render_wgpu::headless::{encode_png, RgbaImage};
use cad_ui_slint::CadView;

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

/// Host-side validation is test-only; it owns all real-window regression cases.
#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::host::HostController;

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
        eprintln!(
            "DWG CPU stages: import={:?}, fit={:?}, scene={:?}, entities={}, batches={}, vertices={}",
            imported.duration_since(started),
            fitted.duration_since(imported),
            fitted.elapsed(),
            drawing.entities().count(),
            scene.added.len(),
            scene.added.iter().map(|b| b.vertices.len()).sum::<usize>()
        );
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

    /// Absolute screenshot-pixel bounds of canvas pixels that differ from the
    /// canvas background sample. These bounds are descriptive evidence only.
    fn canvas_non_background_bounds(
        image: &RgbaImage,
        rect: [f64; 4],
        scale: f64,
    ) -> Option<[u32; 4]> {
        let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
        let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
        let x1 = ((rect[0] + rect[2]) * scale)
            .floor()
            .min(image.width as f64) as u32;
        let y1 = ((rect[1] + rect[3]) * scale)
            .floor()
            .min(image.height as f64) as u32;
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let background = image.pixel(x0, y0);
        let mut bounds = [u32::MAX, u32::MAX, 0, 0];
        for y in y0..y1 {
            for x in x0..x1 {
                if image.pixel(x, y) != background {
                    bounds[0] = bounds[0].min(x);
                    bounds[1] = bounds[1].min(y);
                    bounds[2] = bounds[2].max(x + 1);
                    bounds[3] = bounds[3].max(y + 1);
                }
            }
        }
        (bounds[0] != u32::MAX).then_some(bounds)
    }

    /// Compare adaptive-width strips along all four CAD canvas edges. The
    /// evidence is a grid-off/grid-on delta, so axes and drawing pixels cancel.
    fn grid_edge_deltas(
        before: &RgbaImage,
        after: &RgbaImage,
        rect: [f64; 4],
        scale: f64,
    ) -> [usize; 4] {
        if before.width != after.width || before.height != after.height {
            return [0; 4];
        }
        let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
        let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
        let x1 = ((rect[0] + rect[2]) * scale)
            .floor()
            .min(before.width as f64) as u32;
        let y1 = ((rect[1] + rect[3]) * scale)
            .floor()
            .min(before.height as f64) as u32;
        if x0 >= x1 || y0 >= y1 {
            return [0; 4];
        }
        let band = (((x1 - x0).min(y1 - y0)) / 10).max(8);
        let mut deltas = [0usize; 4];
        for y in y0..y1 {
            for x in x0..x1 {
                let edge = if y < y0 + band {
                    Some(0)
                } else if y + band >= y1 {
                    Some(1)
                } else if x < x0 + band {
                    Some(2)
                } else if x + band >= x1 {
                    Some(3)
                } else {
                    None
                };
                if let Some(edge) = edge {
                    deltas[edge] += usize::from(before.pixel(x, y) != after.pixel(x, y));
                }
            }
        }
        deltas
    }

    /// Count positions along one row/column whose physical-pixel neighborhood
    /// changed after an off-center pointer move. A continuous crosshair must
    /// span nearly the complete logical canvas in both directions.
    fn crosshair_coverage(
        before: &RgbaImage,
        after: &RgbaImage,
        rect: [f64; 4],
        scale: f64,
        local: [f64; 2],
    ) -> [usize; 2] {
        if before.width != after.width || before.height != after.height {
            return [0; 2];
        }
        let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
        let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
        let x1 = ((rect[0] + rect[2]) * scale)
            .floor()
            .min(before.width as f64) as u32;
        let y1 = ((rect[1] + rect[3]) * scale)
            .floor()
            .min(before.height as f64) as u32;
        if x0 >= x1 || y0 >= y1 {
            return [0; 2];
        }
        let radius = (scale.ceil() as u32).max(1);
        let row = ((rect[1] + local[1]) * scale).round() as i64;
        let column = ((rect[0] + local[0]) * scale).round() as i64;
        let row_changed = (x0..x1)
            .filter(|&x| {
                let ya = (row - i64::from(radius)).max(i64::from(y0)) as u32;
                let yb = (row + i64::from(radius)).min(i64::from(y1 - 1)) as u32;
                (ya..=yb).any(|y| before.pixel(x, y) != after.pixel(x, y))
            })
            .count();
        let column_changed = (y0..y1)
            .filter(|&y| {
                let xa = (column - i64::from(radius)).max(i64::from(x0)) as u32;
                let xb = (column + i64::from(radius)).min(i64::from(x1 - 1)) as u32;
                (xa..=xb).any(|x| before.pixel(x, y) != after.pixel(x, y))
            })
            .count();
        [row_changed, column_changed]
    }

    fn real_window_readiness_diagnostic(app: &LinuxApp) -> String {
        let loading = app.runtime.loading();
        let controller = app.runtime.controller.borrow();
        let entities = controller
            .drawing()
            .map(|drawing| drawing.entities().count());
        let import_report = controller
            .last_import_report
            .as_ref()
            .map(|report| (&report.completeness, report.diagnostics.len()));
        let import_snapshot = controller.async_open_snapshot();
        let view = app.runtime.view.borrow();
        let view_state = view.as_ref().map(|view| {
            format!(
                "frames={}, presented={}, progress={:?}, loading_phase={:?}, last_error={:?}, scene_error={:?}, backend={:?}",
                view.frames_rendered(),
                view.current_drawing_presented(),
                view.drawing_progress(),
                view.loading_phase(),
                view.last_error(),
                view.scene_error(),
                view.backend_label()
            )
        });
        format!(
            "loading={loading}, import_report={import_report:?}, import_snapshot={import_snapshot:?}, entities={entities:?}, view={view_state:?}"
        )
    }

    /// Count CAD-canvas pixels differing from the canvas background sample.
    fn canvas_content_pixels(canvas: &[u8]) -> usize {
        let Some(background) = canvas.get(..4) else {
            return 0;
        };
        canvas
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel.as_slice() != background)
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
        let ready_deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut before_grid: Option<Vec<u8>> = None;
        let mut grid_changed = 0usize;
        let mut zoom_before = 0.0;
        let mut zoom_after = 0.0;
        let mut drawing_content_pixels = 0usize;
        let mut screenshot_size = [0u32; 2];
        let mut cad_rect = [0.0; 4];
        let mut initial_canvas_bounds = None;
        let mut grid_pixel_bounds = None;

        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let ready = {
                    let view = probe.runtime.view.borrow();
                    view.as_ref().is_some_and(|view| {
                        let progress = view.drawing_progress();
                        let controller = probe.runtime.controller.borrow();
                        controller.last_import_report.is_some()
                            && controller
                                .drawing()
                                .is_some_and(|drawing| drawing.entities().next().is_some())
                            && view.frames_rendered() > 0
                            && view.current_drawing_presented()
                            && view.last_error().is_none()
                            && view.scene_error().is_none()
                            && progress
                                .is_some_and(|(complete, total)| complete == total && total > 0)
                    })
                };
                if !ready {
                    assert!(
                        std::time::Instant::now() < ready_deadline,
                        "drawing import/preparation/presentation did not complete before acceptance deadline; {}",
                        real_window_readiness_diagnostic(&probe)
                    );
                    return;
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
                        screenshot_size = [image.width, image.height];
                        cad_rect = rect;
                        initial_canvas_bounds = canvas_non_background_bounds(&image, rect, scale);
                        before_grid = Some(canvas_region(&image, rect, scale));
                        drawing_content_pixels =
                            canvas_content_pixels(before_grid.as_ref().unwrap());
                        assert!(
                            drawing_content_pixels > 0,
                            "CAD canvas is blank; refusing to capture grid/zoom evidence"
                        );
                        let progress = probe.runtime.view.borrow().as_ref().unwrap().drawing_progress();
                        assert!(
                            progress.is_some_and(|(complete, total)| complete == total && total > 0),
                            "initial capture requires all drawing batches to be presented: {progress:?}"
                        );
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
                        grid_pixel_bounds = canvas_non_background_bounds(&image, rect, scale);
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
                            "real GPU E2E: backend={backend:?} screenshot={screenshot_size:?} cad_rect_logical={cad_rect:?} initial_canvas_non_background_bounds_px={initial_canvas_bounds:?} grid_on_canvas_non_background_bounds_px={grid_pixel_bounds:?} completed drawing content sample changes={drawing_content_pixels} grid_changed_pixels={grid_changed} \
                             zoom world_per_px {zoom_before} -> {zoom_after}; pixel deltas confirm canvas change, not lattice geometry (review grid PNG manually)"
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

    /// Real-window acceptance for maximization, full-canvas grid coverage,
    /// pointer crosshair, zoom-out, pan, and restore/resize/re-maximize.
    #[test]
    #[ignore = "requires a real display + hardware GPU; run in a dedicated process"]
    fn real_gpu_maximized_grid_crosshair_navigation_acceptance() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        assert!(
            std::env::var_os("VK_ICD_FILENAMES").is_none(),
            "refusing forced Vulkan ICD"
        );
        assert!(
            std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok(),
            "no display available"
        );
        let out = PathBuf::from(
            std::env::var("YACR_REAL_MAXIMIZED_OUTPUT")
                .expect("runner must provide fresh evidence path"),
        );
        std::fs::create_dir(&out).expect("evidence directory must be new");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures/dxf/qcad-flange/flange.dxf");
        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(
            LinuxApp::new(
                LinuxOptions::parse(["--open".into(), fixture.display().to_string()]).unwrap(),
            )
            .unwrap(),
        );

        let tick_counter = Rc::new(Cell::new(0u32));
        let ticks = tick_counter.clone();
        let probe = app.clone();
        let timer = slint::Timer::default();
        let open_deadline = Instant::now() + Duration::from_secs(120);
        let maximize_deadline = Instant::now() + Duration::from_secs(15);
        let completed_phase = Rc::new(Cell::new(0u8));
        let completion = completed_phase.clone();
        let mut phase = 0u8;
        let mut phase_deadline = open_deadline;
        let mut frame_baseline = 0u64;
        let mut initial: Option<RgbaImage> = None;
        let mut grid: Option<RgbaImage> = None;
        let mut zoom_out_camera = None;
        let mut initial_world_per_px = 0.0;
        let mut resized_size = None;
        let mut canvas = [0.0; 4];
        let mut scale = 1.0;
        let mut hover = [0.0; 2];
        let mut requested_size = None;
        let mut pre_resize_size = None;
        let mut pre_resize_canvas = None;
        let mut restore_configure: Option<(slint::PhysicalSize, [f64; 4])> = None;
        let mut restore_stable_ticks = 0u8;
        let mut resize_configure: Option<(slint::PhysicalSize, [f64; 4])> = None;
        let mut resize_stable_ticks = 0u8;
        let mut initial_bounds = None;
        let mut grid_bounds = None;
        let mut edge_deltas = [0usize; 4];

        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            let view_ready = {
                let view = probe.runtime.view.borrow();
                view.as_ref().is_some_and(|view| {
                    let controller = probe.runtime.controller.borrow();
                    controller.last_import_report.is_some()
                        && controller.drawing().is_some_and(|drawing| drawing.entities().next().is_some())
                        && view.frames_rendered() > 0
                        && view.current_drawing_presented()
                        && view.last_error().is_none()
                        && view.scene_error().is_none()
                        && view.drawing_progress().is_some_and(|(done,total)| total > 0 && done == total)
                })
            };
            if phase == 0 && !view_ready {
                assert!(Instant::now() < open_deadline, "open/presentation deadline: {}", real_window_readiness_diagnostic(&probe));
                return;
            }
            if phase == 0 && !probe.adapter.window().is_maximized() {
                assert!(Instant::now() < maximize_deadline, "startup window was not maximized after event-loop presentation");
                return;
            }
            if phase == 2 {
                let view = probe.runtime.view.borrow();
                if !view
                    .as_ref()
                    .is_some_and(|view| view.current_drawing_presented() && view.last_error().is_none() && view.scene_error().is_none())
                {
                    assert!(Instant::now() < phase_deadline, "phase 2 lost CAD presentation/error-free state; {}", real_window_readiness_diagnostic(&probe));
                    return;
                }
            } else if matches!(phase, 1 | 3 | 4 | 6 | 7) {
                let view = probe.runtime.view.borrow();
                let view = view.as_ref().unwrap();
                if view.frames_rendered() <= frame_baseline || !view.current_drawing_presented() {
                    assert!(Instant::now() < phase_deadline, "phase {phase} did not render; {}", real_window_readiness_diagnostic(&probe));
                    return;
                }
            }
            ticks.set(ticks.get() + 1);
            let capture = || -> RgbaImage {
                let pixels = probe.adapter.window().take_snapshot().unwrap();
                RgbaImage { width: pixels.width(), height: pixels.height(), pixels: pixels.as_bytes().to_vec() }
            };
            let save = |name: &str| {
                let image = capture();
                std::fs::write(out.join(name), encode_png(&image).unwrap()).unwrap();
                image
            };
            let camera = || {
                let c = probe.runtime.controller.borrow();
                c.application.workspace.viewports[&c.viewport_id].camera
            };
            let mut reset_frame_deadline = || {
                frame_baseline = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                phase_deadline = Instant::now() + Duration::from_secs(20);
            };

            match phase {
                0 => {
                    assert!(probe.adapter.window().is_maximized(), "maximized state must hold after window/event pump");
                    let image = save("01-maximized-initial.png");
                    let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                    canvas = rect;
                    scale = probe.adapter.window().scale_factor() as f64;
                    initial_bounds = canvas_non_background_bounds(&image, canvas, scale);
                    assert!(initial_bounds.is_some(), "maximized initial CAD canvas has no visible pixels; screenshot={:?}, canvas={canvas:?}, scale={scale}", [image.width, image.height]);
                    initial_world_per_px = {
                        let c = probe.runtime.controller.borrow();
                        c.application.workspace.viewports[&c.viewport_id].world_per_px()
                    };
                    initial = Some(image);
                    probe.adapter.component().invoke_overlay_toggled("grid".into(), true);
                    phase = 1;
                    reset_frame_deadline();
                }
                1 => {
                    let image = save("02-maximized-grid.png");
                    edge_deltas = grid_edge_deltas(initial.as_ref().unwrap(), &image, canvas, scale);
                    assert!(edge_deltas.iter().all(|count| *count > 0), "grid toggle changed no pixels in one or more adaptive edge bands (top,bottom,left,right)={edge_deltas:?}; canvas={canvas:?}, scale={scale}");
                    grid_bounds = canvas_non_background_bounds(&image, canvas, scale);
                    grid = Some(image);
                    hover = [canvas[2] * 0.37, canvas[3] * 0.63];
                    let position = slint::LogicalPosition::new(
                        (canvas[0] + hover[0]) as f32,
                        (canvas[1] + hover[1]) as f32,
                    );
                    probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
                    phase = 2;
                    reset_frame_deadline();
                }
                2 => {
                    // Pointer hover is Slint/UI-only; it need not submit another
                    // CAD GPU frame. Poll actual window readback until the
                    // crosshair pixels appear or the phase deadline expires.
                    let image = capture();
                    let coverage = crosshair_coverage(grid.as_ref().unwrap(), &image, canvas, scale, hover);
                    let physical_canvas = [canvas[2] * scale, canvas[3] * scale];
                    if (coverage[0] as f64) < physical_canvas[0] * 0.80
                        || (coverage[1] as f64) < physical_canvas[1] * 0.80
                    {
                        assert!(
                            Instant::now() < phase_deadline,
                            "off-center pointer crosshair did not span the canvas row/column after UI event processing: coverage={coverage:?}, physical_canvas={physical_canvas:?}, hover={hover:?}, scale={scale}, frames={}, {}",
                            probe.runtime.view.borrow().as_ref().unwrap().frames_rendered(),
                            real_window_readiness_diagnostic(&probe)
                        );
                        return;
                    }
                    std::fs::write(
                        out.join("03-maximized-crosshair-off-center.png"),
                        encode_png(&image).unwrap(),
                    )
                    .unwrap();
                    let center = slint::LogicalPosition::new((canvas[0] + canvas[2] * 0.5) as f32, (canvas[1] + canvas[3] * 0.5) as f32);
                    probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position: center, delta_x: 0.0, delta_y: -120.0 });
                    phase = 3;
                    reset_frame_deadline();
                }
                3 => {
                    save("04-maximized-zoom-out.png");
                    let after = {
                        let c = probe.runtime.controller.borrow();
                        c.application.workspace.viewports[&c.viewport_id].world_per_px()
                    };
                    assert!(after > initial_world_per_px, "wheel-down must zoom out: world_per_px {initial_world_per_px} -> {after}");
                    zoom_out_camera = Some(camera());
                    probe.adapter.component().invoke_pan_requested();
                    let from = slint::LogicalPosition::new((canvas[0] + canvas[2] * 0.42) as f32, (canvas[1] + canvas[3] * 0.44) as f32);
                    let to = slint::LogicalPosition::new((canvas[0] + canvas[2] * 0.58) as f32, (canvas[1] + canvas[3] * 0.56) as f32);
                    probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: from, button: slint::platform::PointerEventButton::Left });
                    probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: to });
                    probe.adapter.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: to, button: slint::platform::PointerEventButton::Left });
                    probe.adapter.component().invoke_pan_requested();
                    phase = 4;
                    reset_frame_deadline();
                }
                4 => {
                    save("05-maximized-pan.png");
                    assert_ne!(camera(), *zoom_out_camera.as_ref().unwrap(), "real window pan did not change camera");
                    probe.adapter.window().set_maximized(false);
                    phase = 5;
                    phase_deadline = Instant::now() + Duration::from_secs(15);
                }
                5 => {
                    if probe.adapter.window().is_maximized() {
                        restore_configure = None;
                        restore_stable_ticks = 0;
                        assert!(Instant::now() < phase_deadline, "window did not restore from maximized state");
                        return;
                    }
                    let before = probe.adapter.window().size();
                    let rect = probe.adapter.handle().shell_geometry().unwrap().0;
                    if restore_configure == Some((before, rect)) {
                        restore_stable_ticks = restore_stable_ticks.saturating_add(1);
                    } else {
                        restore_configure = Some((before, rect));
                        restore_stable_ticks = 1;
                    }
                    if restore_stable_ticks < 3 {
                        assert!(Instant::now() < phase_deadline, "restored window configure did not stabilize: size={before:?}, canvas={rect:?}");
                        return;
                    }
                    pre_resize_size = Some(before);
                    pre_resize_canvas = Some(rect);
                    let size = slint::PhysicalSize::new(
                        (f64::from(before.width) * 0.85).round() as u32,
                        (f64::from(before.height) * 0.85).round() as u32,
                    );
                    requested_size = Some(size);
                    frame_baseline = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                    probe.adapter.window().set_size(size);
                    phase = 6;
                    phase_deadline = Instant::now() + Duration::from_secs(15);
                }
                6 => {
                    let size = probe.adapter.window().size();
                    let before = pre_resize_size.unwrap();
                    let rect = probe.adapter.handle().shell_geometry().unwrap().0;
                    let before_canvas = pre_resize_canvas.unwrap();
                    if resize_configure == Some((size, rect)) {
                        resize_stable_ticks = resize_stable_ticks.saturating_add(1);
                    } else {
                        resize_configure = Some((size, rect));
                        resize_stable_ticks = 1;
                    }
                    let physical_changed = size.width.abs_diff(before.width) > 2
                        || size.height.abs_diff(before.height) > 2;
                    let canvas_changed = (rect[2] - before_canvas[2]).abs() > 2.0
                        || (rect[3] - before_canvas[3]).abs() > 2.0;
                    if !physical_changed || !canvas_changed || resize_stable_ticks < 3 {
                        assert!(
                            Instant::now() < phase_deadline,
                            "restored-window resize did not produce a stable compositor configure: requested={:?}, before={before:?}, actual={size:?}, stable_ticks={resize_stable_ticks}, before_canvas={before_canvas:?}, actual_canvas={rect:?}",
                            requested_size.unwrap()
                        );
                        return;
                    }
                    assert!(!probe.adapter.window().is_maximized(), "resize evidence must be in restored state");
                    let image = save("06-restored-resized.png");
                    let resized_bounds = canvas_non_background_bounds(&image, rect, probe.adapter.window().scale_factor() as f64);
                    assert!(resized_bounds.is_some(), "resized CAD canvas has no rendered pixels: size={size:?}, rect={rect:?}");
                    resized_size = Some(size);
                    frame_baseline = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                    probe.adapter.window().set_maximized(true);
                    phase = 7;
                    phase_deadline = Instant::now() + Duration::from_secs(15);
                }
                7 => {
                    if !probe.adapter.window().is_maximized() {
                        assert!(Instant::now() < phase_deadline, "window did not re-maximize after resize");
                        return;
                    }
                    let image = save("07-maximized-after-resize.png");
                    assert_ne!(probe.adapter.window().size(), resized_size.unwrap(), "re-maximized dimensions did not change from restored resize");
                    let maximized_size = probe.adapter.window().size();
                    let bounds = canvas_non_background_bounds(&image, probe.adapter.handle().shell_geometry().unwrap().0, probe.adapter.window().scale_factor() as f64);
                    let initial = initial.as_ref().unwrap();
                    let backend = probe.runtime.view.borrow().as_ref().unwrap().backend_label();
                    let backend_lower = backend.as_deref().unwrap_or_default().to_ascii_lowercase();
                    assert!(!["lavapipe", "swiftshader", "software"].iter().any(|name| backend_lower.contains(name)), "software backend is not accepted: {backend:?}");
                    eprintln!("real-GPU maximized/grid/crosshair evidence: backend={backend:?}, initial_screenshot={:?}, final_screenshot={:?}, requested_restored_size={:?}, actual_restored_size={:?}, actual_maximized_size={maximized_size:?}, initial_cad_rect_logical={canvas:?}, scale={scale}, initial_canvas_non_background_bounds_px={initial_bounds:?}, grid_canvas_non_background_bounds_px={grid_bounds:?}, final_canvas_non_background_bounds_px={bounds:?}, grid_edge_deltas_top_bottom_left_right={edge_deltas:?}, crosshair_hover_canvas_local={hover:?}; artifacts=01-maximized-initial.png,02-maximized-grid.png,03-maximized-crosshair-off-center.png,04-maximized-zoom-out.png,05-maximized-pan.png,06-restored-resized.png,07-maximized-after-resize.png", [initial.width, initial.height], [image.width, image.height], requested_size.unwrap(), resized_size.unwrap());
                    assert!(probe.adapter.window().is_maximized());
                    completion.set(8);
                    slint::quit_event_loop().unwrap();
                    phase = 8;
                }
                _ => {}
            }
        });
        app.run().unwrap();
        assert_eq!(
            completed_phase.get(),
            8,
            "maximized/grid/crosshair acceptance did not reach final state"
        );
    }

    /// Exercise a real-window selection and MOVE history round trip on the
    /// committed flange DXF. This intentionally uses the winit window input,
    /// Slint command callbacks and the installed host command/draw sinks.
    #[test]
    #[ignore = "requires a real display + hardware GPU; run in a dedicated process"]
    fn real_gpu_select_move_undo_redo_acceptance() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        assert!(
            std::env::var_os("VK_ICD_FILENAMES").is_none(),
            "real-GPU regression refuses a forced Vulkan ICD"
        );
        assert!(
            std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok(),
            "no display available (WAYLAND_DISPLAY/DISPLAY unset)"
        );
        let out = PathBuf::from(
            std::env::var("YACR_REAL_FUNCTIONAL_OUTPUT")
                .expect("runner must provide fresh YACR_REAL_FUNCTIONAL_OUTPUT"),
        );
        std::fs::create_dir(&out).expect("evidence directory must be new");
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures/dxf/qcad-flange/flange.dxf");

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
        let tick_count = ticks.clone();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut action_deadline = deadline;
        let mut ready = false;
        let mut target = None;
        let mut original = None;
        let mut expected_delta = None;
        let mut render_frames = 0;
        let mut selected_pixels = None;
        let mut moved_pixels = None;

        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            let view_ready = {
                let view = probe.runtime.view.borrow();
                view.as_ref().is_some_and(|view| {
                    let progress = view.drawing_progress();
                    let controller = probe.runtime.controller.borrow();
                    controller.last_import_report.is_some()
                        && controller.drawing().is_some_and(|drawing| drawing.entities().next().is_some())
                        && view.frames_rendered() > 0
                        && view.current_drawing_presented()
                        && view.last_error().is_none()
                        && view.scene_error().is_none()
                        && progress.is_some_and(|(done, total)| total > 0 && done == total)
                })
            };
            if !view_ready {
                let state_deadline = if ready { action_deadline } else { deadline };
                assert!(
                    Instant::now() < state_deadline,
                    "DXF open and presentation timed out; {}",
                    real_window_readiness_diagnostic(&probe)
                );
                return;
            }
            if ready {
                let view = probe.runtime.view.borrow();
                let view = view.as_ref().unwrap();
                if view.frames_rendered() <= render_frames || !view.current_drawing_presented() {
                    assert!(Instant::now() < action_deadline, "functional state did not render before deadline");
                    return;
                }
            }
            let tick = tick_count.get() + 1;
            tick_count.set(tick);
            let snapshot = || -> RgbaImage {
                let pixels = probe.adapter.window().take_snapshot().unwrap();
                RgbaImage { width: pixels.width(), height: pixels.height(), pixels: pixels.as_bytes().to_vec() }
            };
            let save = |name: &str| {
                let image = snapshot();
                std::fs::write(out.join(name), encode_png(&image).unwrap()).unwrap();
                image
            };

            if !ready {
                let image = save("01-initial.png");
                let (canvas_rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                let initial_canvas = canvas_region(
                    &image,
                    canvas_rect,
                    probe.adapter.window().scale_factor() as f64,
                );
                assert!(canvas_content_pixels(&initial_canvas) > 0, "initial CAD canvas is blank");
                let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                let (viewport_size, camera, mut candidates) = {
                    let c = probe.runtime.controller.borrow();
                    let vp = &c.application.workspace.viewports[&c.viewport_id];
                    let drawing = c.drawing().unwrap();
                    let candidates = drawing.entities()
                        .filter(|entity| entity.space == cad_domain::SpaceId::Model)
                        .filter(|entity| c.session.layer_overrides.is_entity_visible(&drawing, entity))
                        .filter_map(|entity| match entity.geometry {
                            cad_domain::SemanticGeometry::Line { start, end } => {
                                let length = (end.x-start.x).hypot(end.y-start.y);
                                let midpoint = cad_domain::Point3 { x: (start.x+end.x)*0.5, y: (start.y+end.y)*0.5, z: (start.z+end.z)*0.5 };
                                vp.camera.world_to_screen(midpoint, vp.logical_size)
                                    .filter(|p| p[0] > 2.0 && p[1] > 2.0 && p[0] < vp.logical_size[0]-2.0 && p[1] < vp.logical_size[1]-2.0)
                                    .filter(|p| p[0] < rect[2] - 2.0 && p[1] < rect[3] - 2.0)
                                    .filter(|p| {
                                        cad_app::picking::pick_at_screen(
                                            &drawing,
                                            c.document_id,
                                            &vp.camera,
                                            *p,
                                            vp.logical_size,
                                            &cad_domain::TolerancePolicy::default(),
                                            cad_app::BackFacePolicy::Cull,
                                        )
                                        .is_ok_and(|report| {
                                            report
                                                .hit
                                                .is_some_and(|hit| hit.source.entity == entity.id)
                                        })
                                    })
                                    .map(|p| (length, entity.id, start, end, p))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    (vp.logical_size, vp.camera, candidates)
                };
                candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                assert!(!candidates.is_empty(), "fixture has no in-canvas model LINE candidate; rect={rect:?}, viewport={viewport_size:?}");
                let mut attempts = Vec::new();
                for (_, entity_id, start, end, screen) in candidates {
                    let position = slint::LogicalPosition::new(
                        (rect[0] + screen[0]) as f32,
                        (rect[1] + screen[1]) as f32,
                    );
                    probe.adapter.window().dispatch_event(
                        slint::platform::WindowEvent::PointerPressed {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        },
                    );
                    probe.adapter.window().dispatch_event(
                        slint::platform::WindowEvent::PointerReleased {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        },
                    );
                    let refs = probe.runtime.controller.borrow().selection().refs().to_vec();
                    attempts.push(format!("expected={entity_id:?}, local={screen:?}, window={position:?}, actual={refs:?}"));
                    if refs.iter().any(|reference| reference.entity == entity_id) {
                        eprintln!(
                            "real window selection: expected={entity_id:?}, actual={refs:?}, line=({start:?},{end:?}), canvas_local={screen:?}, window_position={position:?}, cad_rect={rect:?}, viewport={viewport_size:?}, camera={camera:?}"
                        );
                        target = Some(entity_id);
                        original = Some((start, end));
                        break;
                    }
                }
                let diagnostic_camera = {
                    let c = probe.runtime.controller.borrow();
                    c.application.workspace.viewports[&c.viewport_id]
                        .camera
                };
                assert!(
                    target.is_some(),
                    "real window pointer did not select any preflight-confirmed visible LINE; rect={rect:?}, viewport={viewport_size:?}, camera={diagnostic_camera:?}, attempts={attempts:?}"
                );
                ready = true;
                render_frames = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                action_deadline = Instant::now() + Duration::from_secs(15);
                return;
            }

            match tick {
                2 => {
                    let selected = probe.runtime.controller.borrow().selection().refs().iter().any(|reference| reference.entity == target.unwrap());
                    assert!(selected, "real window click did not select the chosen fixture LINE");
                    let selected_image = save("02-selected.png");
                    let (canvas_rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                    selected_pixels = Some(canvas_region(
                        &selected_image,
                        canvas_rect,
                        probe.adapter.window().scale_factor() as f64,
                    ));
                    let c = probe.runtime.controller.borrow();
                    let vp = &c.application.workspace.viewports[&c.viewport_id];
                    let screen = {
                        let drawing = c.drawing().unwrap();
                        let entity = drawing.entity(target.unwrap()).unwrap();
                        let (start,end) = match &entity.geometry { cad_domain::SemanticGeometry::Line {start,end} => (start,end), _ => panic!("selected entity ceased to be a LINE") };
                        let midpoint = cad_domain::Point3 { x:(start.x+end.x)*0.5, y:(start.y+end.y)*0.5, z:(start.z+end.z)*0.5 };
                        vp.camera.world_to_screen(midpoint, vp.logical_size).unwrap()
                    };
                    let first = [screen[0], screen[1]];
                    let second = [screen[0] + 40.0, screen[1] - 25.0];
                    let metrics = cad_app::input::ViewMetrics::new(
                        cad_app::input::CanvasMetrics::new([0.0;2], vp.logical_size, vp.dpi_scale), vp);
                    let first = [first[0] as f32 as f64, first[1] as f32 as f64];
                    let second = [second[0] as f32 as f64, second[1] as f32 as f64];
                    let p0 = metrics.surface_to_world(first).unwrap();
                    let p1 = metrics.surface_to_world(second).unwrap();
                    expected_delta = Some(cad_domain::Point3 { x:p1.x-p0.x, y:p1.y-p0.y, z:p1.z-p0.z });
                    drop(c);
                    let ui = probe.adapter.component();
                    assert!(ui.invoke_command_submitted("MOVE".into()));
                    ui.invoke_canvas_pick(first[0] as f32, first[1] as f32);
                    ui.invoke_canvas_pick(second[0] as f32, second[1] as f32);
                    assert!(ui.invoke_command_submitted("CONFIRM".into()));
                    render_frames = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                    action_deadline = Instant::now() + Duration::from_secs(15);
                }
                3 => {
                    let id = target.unwrap();
                    let (a,b) = match probe.runtime.controller.borrow().drawing().unwrap().entity(id).unwrap().geometry {
                        cad_domain::SemanticGeometry::Line {start,end} => (start,end), _ => panic!("MOVE changed LINE type")
                    };
                    let (before_a,before_b) = original.unwrap();
                    let d = expected_delta.unwrap();
                    for (actual,before) in [(a,before_a),(b,before_b)] {
                        assert!((actual.x-before.x-d.x).abs() < 1e-5 && (actual.y-before.y-d.y).abs() < 1e-5, "MOVE geometry mismatch: {actual:?} vs {before:?} + {d:?}");
                    }
                    assert!(probe.adapter.component().get_can_undo(), "MOVE did not enable undo in UI");
                    let image = save("03-moved.png");
                    let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                    let pixels = canvas_region(&image, rect, probe.adapter.window().scale_factor() as f64);
                    assert!(changed_region(selected_pixels.as_ref().unwrap(), &pixels) > 0, "MOVE did not change rendered CAD pixels");
                    moved_pixels = Some(pixels);
                    probe.adapter.component().invoke_undo_requested();
                    render_frames = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                    action_deadline = Instant::now() + Duration::from_secs(15);
                }
                4 => {
                    let (a,b) = match probe.runtime.controller.borrow().drawing().unwrap().entity(target.unwrap()).unwrap().geometry {
                        cad_domain::SemanticGeometry::Line {start,end} => (start,end), _ => panic!("undo changed LINE type")
                    };
                    assert_eq!((a,b), original.unwrap(), "undo did not restore database geometry");
                    assert!(probe.adapter.component().get_can_redo(), "undo did not enable redo in UI");
                    let image = save("04-undone.png");
                    let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                    let pixels = canvas_region(&image, rect, probe.adapter.window().scale_factor() as f64);
                    assert!(changed_region(moved_pixels.as_ref().unwrap(), &pixels) > 0, "undo did not change rendered CAD pixels");
                    probe.adapter.component().invoke_redo_requested();
                    render_frames = probe.runtime.view.borrow().as_ref().unwrap().frames_rendered();
                    action_deadline = Instant::now() + Duration::from_secs(15);
                }
                5 => {
                    let (a,b) = match probe.runtime.controller.borrow().drawing().unwrap().entity(target.unwrap()).unwrap().geometry {
                        cad_domain::SemanticGeometry::Line {start,end} => (start,end), _ => panic!("redo changed LINE type")
                    };
                    let (before_a,before_b) = original.unwrap();
                    let d = expected_delta.unwrap();
                    assert!((a.x-before_a.x-d.x).abs() < 1e-5 && (a.y-before_a.y-d.y).abs() < 1e-5);
                    assert!((b.x-before_b.x-d.x).abs() < 1e-5 && (b.y-before_b.y-d.y).abs() < 1e-5);
                    assert!(probe.adapter.component().get_can_undo(), "redo did not restore UI undo availability");
                    assert!(!probe.adapter.component().get_can_redo(), "redo remained available after redo");
                    let image = save("05-redone.png");
                    let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
                    let pixels = canvas_region(&image, rect, probe.adapter.window().scale_factor() as f64);
                    assert_eq!(changed_region(moved_pixels.as_ref().unwrap(), &pixels), 0, "redo render did not reproduce moved pixels");
                    let backend = probe.runtime.view.borrow().as_ref().unwrap().backend_label();
                    assert!(!backend.as_deref().unwrap_or_default().to_ascii_lowercase().contains("lavapipe"), "software Vulkan backend is not accepted: {backend:?}");
                    eprintln!("real GPU functional regression complete: selected entity={:?}, MOVE/undo/redo DB and UI state verified, backend={backend:?}", target.unwrap());
                    slint::quit_event_loop().unwrap();
                }
                _ => {}
            }
        });
        app.run().unwrap();
        assert!(
            ticks.get() >= 5,
            "functional regression exited before redo assertions"
        );
    }
}
