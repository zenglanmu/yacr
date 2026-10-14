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
        let view = self.view.borrow();
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
        let view = self.view.borrow();
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

    /// Resolve a host status the same way the UI handle does, so a status
    /// assertion compares the real localized label rather than a raw variant.
    fn resolve_status(status: &cad_engine::protocol::StatusMessage, locale: &str) -> String {
        match status {
            cad_engine::protocol::StatusMessage::ExternalText(text) => text.clone(),
            cad_engine::protocol::StatusMessage::Catalog { key, args } => {
                let values = args
                    .iter()
                    .map(|arg| (arg.key.as_str(), arg.value.as_str()))
                    .collect::<Vec<_>>();
                cad_ui_slint::MessageSource::from_request(locale).text(key, &values)
            }
        }
    }

    /// UI-free contract over the *actual* reference-grid overlay geometry.
    ///
    /// Returns `(vertical_spans_viewport, horizontal_spans_viewport)`: a
    /// direction is `true` only when the overlay carries at least one segment of
    /// that direction whose two endpoints reach both viewport edges on the
    /// perpendicular axis. It reads the real batch geometry produced by
    /// [`cad_app::render_scene::grid_overlay`] (no renderer algorithm is
    /// re-implemented and no grid is fabricated). This is the scene-level guard
    /// for the real-window four-edge `grid_edge_deltas` acceptance.
    fn grid_axes_cover(
        overlay: &cad_app::render_scene::VisualOverlay,
        viewport: (cad_domain::Point3, cad_domain::Point3),
    ) -> (bool, bool) {
        let (min, max) = viewport;
        let tol = 1e-3_f64.max((max.x - min.x).abs().max((max.y - min.y).abs()) * 1e-6);
        let mut vertical = false;
        let mut horizontal = false;
        for batch in &overlay.batches {
            if batch.color != cad_app::render_scene::GRID_COLOR {
                continue;
            }
            let origin = batch.local_origin;
            for pair in batch.vertices.as_chunks::<2>().0 {
                let a = [
                    origin.x + f64::from(pair[0][0]),
                    origin.y + f64::from(pair[0][1]),
                ];
                let b = [
                    origin.x + f64::from(pair[1][0]),
                    origin.y + f64::from(pair[1][1]),
                ];
                let dx = (a[0] - b[0]).abs();
                let dy = (a[1] - b[1]).abs();
                if dx <= tol && dy > tol {
                    let ys = [a[1], b[1]];
                    if ys.iter().any(|y| (y - min.y).abs() <= tol)
                        && ys.iter().any(|y| (y - max.y).abs() <= tol)
                    {
                        vertical = true;
                    }
                } else if dy <= tol && dx > tol {
                    let xs = [a[0], b[0]];
                    if xs.iter().any(|x| (x - min.x).abs() <= tol)
                        && xs.iter().any(|x| (x - max.x).abs() <= tol)
                    {
                        horizontal = true;
                    }
                }
            }
        }
        (vertical, horizontal)
    }

    /// Positive scene contract: the actual grid overlay has both a vertical
    /// family spanning the viewport's vertical extent and a horizontal family
    /// spanning its horizontal extent.
    #[test]
    fn grid_overlay_spans_both_axes_across_the_viewport() {
        let viewport = (
            cad_domain::Point3 {
                x: -6.0,
                y: -4.0,
                z: 0.0,
            },
            cad_domain::Point3 {
                x: 6.0,
                y: 4.0,
                z: 0.0,
            },
        );
        let overlay = cad_app::render_scene::grid_overlay(viewport, cad_domain::DocumentId(1));
        let (vertical, horizontal) = grid_axes_cover(&overlay, viewport);
        assert!(
            vertical,
            "grid must contain vertical lines spanning the viewport's vertical extent"
        );
        assert!(
            horizontal,
            "grid must contain horizontal lines spanning the viewport's horizontal extent"
        );
    }

    /// Negative scene contract: inverted bounds (the historic host regression
    /// that wrote screen top-left/bottom-right unnormalized) and a shortened
    /// span must fail the same predicate in the affected direction.
    #[test]
    fn grid_contract_rejects_inverted_or_shortened_bounds() {
        let p = |x: f64, y: f64| cad_domain::Point3 { x, y, z: 0.0 };
        let viewport = (p(-6.0, -4.0), p(6.0, 4.0));

        // Inverted Y: the grid builder drops the horizontal family.
        let inverted_y = cad_app::render_scene::grid_overlay(
            (p(-6.0, 4.0), p(6.0, -4.0)),
            cad_domain::DocumentId(1),
        );
        let (vertical, horizontal) = grid_axes_cover(&inverted_y, viewport);
        assert!(vertical, "inverted-Y bounds still emit vertical lines");
        assert!(
            !horizontal,
            "inverted-Y bounds must fail the horizontal coverage contract"
        );

        // Inverted X: the vertical family is dropped.
        let inverted_x = cad_app::render_scene::grid_overlay(
            (p(6.0, -4.0), p(-6.0, 4.0)),
            cad_domain::DocumentId(1),
        );
        let (vertical, horizontal) = grid_axes_cover(&inverted_x, viewport);
        assert!(
            !vertical,
            "inverted-X bounds must fail the vertical coverage contract"
        );
        assert!(horizontal, "inverted-X bounds still emit horizontal lines");

        // Shortened span: lines exist but do not reach the viewport edges.
        let shortened = cad_app::render_scene::grid_overlay(
            (p(-6.0, -4.0), p(0.0, 2.0)),
            cad_domain::DocumentId(1),
        );
        let (vertical, horizontal) = grid_axes_cover(&shortened, viewport);
        assert!(
            !vertical && !horizontal,
            "a shortened span must fail edge coverage on both axes"
        );
    }

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
            if probe.view.borrow().as_ref().unwrap().frames_rendered() == 0 {
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
                || !probe.view.borrow().as_ref().unwrap().current_drawing_presented() {
                assert!(started.elapsed() < Duration::from_secs(60), "new drawing never rendered");
                return;
            }
            last_tick = Some(now);
            let tick = tick_count.get() + 1;
            tick_count.set(tick);
            if tick == 1 {
                let progress = probe.view.borrow().as_ref().unwrap().drawing_progress().expect("progressive drawing progress is required");
                assert_eq!(progress.0, progress.1, "a truncated prefix must not be marked presented");
                eprintln!("desktop complete drawing batches={progress:?}");
                eprintln!("desktop drawing bounds={:?}, camera={:?}, loading max_gap={max_gap:?}", probe.runtime.controller.borrow().drawing().unwrap().bounds(), probe.view.borrow().as_ref().unwrap().camera3d());
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
                    let view = probe.view.borrow();
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
                let view = probe.view.borrow();
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
        let view = app.view.borrow();
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
                    let view = probe.view.borrow();
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
                        let progress = probe.view.borrow().as_ref().unwrap().drawing_progress();
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

    /// Run the HostCore navigation-to-Slint bridge assertions on the real
    /// desktop renderer. Unlike the cad-host synthetic tests, this owns the
    /// native event loop and quits from a bounded Slint timer callback.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a real Wayland/X11 window and NVIDIA hardware Vulkan"]
    fn real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        assert!(
            std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some(),
            "real-window acceptance requires WAYLAND_DISPLAY or DISPLAY"
        );
        for variable in [
            "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES",
            "VK_ADD_DRIVER_FILES",
            "VK_LOADER_DRIVERS_SELECT",
            "WGPU_ADAPTER_NAME",
            "MESA_VK_DEVICE_SELECT",
            "MESA_LOADER_DRIVER_OVERRIDE",
        ] {
            assert!(
                std::env::var_os(variable).is_none(),
                "refusing forced/software GPU selection from {variable}"
            );
        }
        let output = PathBuf::from(
            std::env::var_os("YACR_NAVIGATION_OUTPUT")
                .expect("set a fresh YACR_NAVIGATION_OUTPUT evidence directory"),
        );
        std::fs::create_dir(&output).expect("evidence directory must be new");

        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(LinuxApp::new(LinuxOptions::parse([]).unwrap()).unwrap());
        let core = &app.runtime;
        let point = |x: f64, y: f64| cad_domain::Point3 { x, y, z: 0.0 };
        let first = {
            let mut controller = core.controller.borrow_mut();
            let created = controller
                .create_line(point(0.0, 0.0), point(1.0, 0.0))
                .unwrap();
            let second = controller
                .create_line(point(0.0, 10.0), point(1.0, 10.0))
                .unwrap();
            let entity = cad_domain::EntityId(created.objects[0].0);
            assert_ne!(entity, cad_domain::EntityId(second.objects[0].0));
            let document = controller.document_id;
            controller
                .session
                .selection
                .replace([cad_app::selection::model_ref(document, entity)]);
            entity
        };
        app.binding.flush_updates().unwrap();
        core.command(
            cad_app::CommandId::Measure,
            cad_app::CommandPayload::MeasureTool(cad_app::MeasurementToolKind::Distance),
        )
        .unwrap();

        let (viewport, initial_world_per_px) = {
            let controller = core.controller.borrow();
            (
                controller.viewport_id,
                controller.application.workspace.viewports[&controller.viewport_id].world_per_px(),
            )
        };
        assert!(initial_world_per_px.is_finite() && initial_world_per_px > 0.0);

        // Find the real endpoint's candidate cutoff through HostCore's
        // production snap getter, then keep the cursor inside before zoom and
        // outside after its 4x world-per-pixel reduction.
        let mut low = 0.0;
        let mut high = initial_world_per_px;
        let set_cursor_and_check = |distance: f64| {
            core.controller
                .borrow_mut()
                .session
                .set_measurement_cursor(Some(point(distance, 0.0)))
                .unwrap();
            core.controller
                .borrow()
                .snap_hints_near_cursor()
                .unwrap()
                .iter()
                .any(|hint| hint.source.entity == first && hint.point == point(0.0, 0.0))
        };
        if !set_cursor_and_check(high) {
            for _ in 0..48 {
                let middle = (low + high) * 0.5;
                if set_cursor_and_check(middle) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
        } else {
            for _ in 0..48 {
                high *= 2.0;
                assert!(high.is_finite(), "snap cutoff search overflowed");
                if !set_cursor_and_check(high) {
                    break;
                }
                low = high;
            }
            assert!(!set_cursor_and_check(high), "could not bound snap cutoff");
            for _ in 0..48 {
                let middle = (low + high) * 0.5;
                if set_cursor_and_check(middle) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
        }
        let cursor_distance = low * 0.75;
        assert!(cursor_distance > 0.0 && set_cursor_and_check(cursor_distance));
        app.binding.flush_updates().unwrap();

        let drawing_before = core.controller.borrow().drawing().unwrap();
        let (generation, selection, history, measurement, camera_before) = {
            let controller = core.controller.borrow();
            (
                controller.session.generation,
                controller.session.selection.refs().to_vec(),
                controller.history_availability(),
                controller.session.measurement_preview(),
                controller.application.workspace.viewports[&viewport].camera,
            )
        };
        let view = app
            .view
            .borrow()
            .clone()
            .expect("LinuxApp installs its CadView");
        assert_eq!(view.camera3d(), camera_before);
        let snap_before = view.snap_hints();
        assert!(snap_before
            .iter()
            .any(|hint| hint.source.entity == first && hint.point == point(0.0, 0.0)));
        assert_eq!(
            snap_before,
            core.controller.borrow().snap_hints_near_cursor().unwrap()
        );
        assert_eq!(app.adapter.component().get_selection_count(), 1);
        assert!(app.adapter.component().get_measurement_active());

        core.command(
            cad_app::CommandId::Pan,
            cad_app::CommandPayload::Points(vec![point(15.0, -7.0)]),
        )
        .expect("successful Pan must complete its light push");
        {
            let controller = core.controller.borrow();
            let viewport_state = &controller.application.workspace.viewports[&viewport];
            assert_eq!(view.camera3d(), viewport_state.camera);
            assert_eq!(view.camera3d(), viewport_state.view_mode.saved_2d_camera());
            assert_eq!(view.selection_highlight().refs(), selection.as_slice());
            assert_eq!(controller.session.generation, generation);
            assert_eq!(controller.session.selection.refs(), selection);
            assert_eq!(controller.history_availability(), history);
            assert_eq!(controller.session.measurement_preview(), measurement);
            assert!(Arc::ptr_eq(&drawing_before, &controller.drawing().unwrap()));
        }
        let pan_hints = view.snap_hints();
        assert!(pan_hints
            .iter()
            .any(|hint| hint.source.entity == first && hint.point == point(0.0, 0.0)));

        core.command(
            cad_app::CommandId::Zoom,
            cad_app::CommandPayload::ZoomAt {
                factor: 4.0,
                cursor: [400.0, 300.0],
            },
        )
        .expect("successful Zoom must complete its light push");
        let camera_after_zoom =
            core.controller.borrow().application.workspace.viewports[&viewport].camera;
        let zoom_hints = view.snap_hints();
        assert_eq!(view.camera3d(), camera_after_zoom);
        assert!(!zoom_hints
            .iter()
            .any(|hint| hint.source.entity == first && hint.point == point(0.0, 0.0)));
        assert!(app.adapter.component().get_measurement_active());
        {
            let controller = core.controller.borrow();
            assert_eq!(controller.session.generation, generation);
            assert_eq!(controller.session.selection.refs(), selection);
            assert_eq!(controller.history_availability(), history);
            assert_eq!(controller.session.measurement_preview(), measurement);
            assert!(Arc::ptr_eq(&drawing_before, &controller.drawing().unwrap()));
            assert_eq!(
                controller.application.workspace.viewports[&viewport]
                    .view_mode
                    .saved_2d_camera(),
                camera_after_zoom
            );
        }

        core.command(
            cad_app::CommandId::Select,
            cad_app::CommandPayload::Selection(Vec::new()),
        )
        .unwrap();
        app.binding.flush_updates().unwrap();
        assert_eq!(app.adapter.component().get_selection_count(), 0);
        assert!(view.selection_highlight().is_empty());
        let error = core
            .command(cad_app::CommandId::Pan, cad_app::CommandPayload::None)
            .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)));
        app.binding.flush_updates().unwrap();
        let expected = resolve_status(
            &core.last_status().unwrap(),
            &LinuxOptions::default().locale,
        );
        assert_eq!(
            app.adapter.component().get_status_label().to_string(),
            expected
        );
        std::fs::write(
            output.join("host-navigation.txt"),
            format!(
                "display={} wayland={:?}\ninitial_world_per_px={initial_world_per_px}\nsnap_cursor_distance={cursor_distance}\nsnap_before={} snap_after_pan={} snap_after_zoom={}\nfull_push_selection_count={} measurement_active={}\n",
                std::env::var("DISPLAY").unwrap_or_default(),
                std::env::var("WAYLAND_DISPLAY").ok(),
                snap_before.len(),
                pan_hints.len(),
                zoom_hints.len(),
                app.adapter.component().get_selection_count(),
                app.adapter.component().get_measurement_active(),
            ),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        let completed = Rc::new(Cell::new(false));
        let timer = slint::Timer::default();
        let probe = app.clone();
        let completed_probe = completed.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
            let view = probe.view.borrow();
            let Some(view) = view.as_ref() else {
                assert!(Instant::now() < deadline, "CadView was not installed before deadline");
                return;
            };
            if view.backend_is_live() && view.frames_rendered() >= 1 {
                assert_eq!(view.backend_label().as_deref(), Some("vulkan"));
                let backend = format!("{:?}", view.active_backend());
                assert!(backend.contains("Native"), "unexpected renderer backend: {backend}");
                let fds = std::fs::read_dir("/proc/self/fd")
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                    .collect::<Vec<_>>();
                assert!(
                    fds.iter().any(|path| path == Path::new("/dev/nvidia0")),
                    "no NVIDIA device fd in the live renderer process: {fds:?}"
                );
                std::fs::write(
                    output.join("renderer.txt"),
                    format!(
                        "backend={backend}\napi={:?}\nframes_rendered={}\nnvidia_device_fd=true\n",
                        view.backend_label(),
                        view.frames_rendered(),
                    ),
                )
                .unwrap();
                completed_probe.set(true);
                slint::quit_event_loop().unwrap();
            } else if Instant::now() >= deadline {
                panic!(
                    "real GPU renderer did not become live and render a frame within 90s: backend={:?}, frames={}, last_error={:?}, scene_error={:?}",
                    view.backend_label(),
                    view.frames_rendered(),
                    view.last_error(),
                    view.scene_error()
                );
            }
        });
        app.run().unwrap();
        assert!(
            completed.get(),
            "Slint event loop quit without renderer acceptance"
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
                let view = probe.view.borrow();
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
                let view = probe.view.borrow();
                if !view
                    .as_ref()
                    .is_some_and(|view| view.current_drawing_presented() && view.last_error().is_none() && view.scene_error().is_none())
                {
                    assert!(Instant::now() < phase_deadline, "phase 2 lost CAD presentation/error-free state; {}", real_window_readiness_diagnostic(&probe));
                    return;
                }
            } else if matches!(phase, 1 | 3 | 4 | 6 | 7) {
                let view = probe.view.borrow();
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
                frame_baseline = probe.view.borrow().as_ref().unwrap().frames_rendered();
                phase_deadline = Instant::now() + Duration::from_secs(20);
            };
            // Controller viewport size vs the shared render session's canvas
            // size, recorded at every phase that needs it. They are derived from
            // the same logical viewport, so a settled frame must report equal
            // sizes; a disagreement is a wiring/size bug, not a grid artifact.
            let size_state = || {
                let c = probe.runtime.controller.borrow();
                let controller_logical =
                    c.application.workspace.viewports[&c.viewport_id].logical_size;
                let render_canvas = probe
                    .view
                    .borrow()
                    .as_ref()
                    .map(|view| view.render_session().view_snapshot().canvas_size)
                    .unwrap_or([0.0, 0.0]);
                (controller_logical, render_canvas)
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
                    let (controller_logical, render_canvas) = size_state();
                    eprintln!(
                        "maximized initial sizes: controller_logical={controller_logical:?}, render_session_canvas={render_canvas:?}, shell_cad_rect={canvas:?}"
                    );
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
                            probe.view.borrow().as_ref().unwrap().frames_rendered(),
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
                    frame_baseline = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    frame_baseline = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    let backend = probe.view.borrow().as_ref().unwrap().backend_label();
                    let backend_lower = backend.as_deref().unwrap_or_default().to_ascii_lowercase();
                    assert!(!["lavapipe", "swiftshader", "software"].iter().any(|name| backend_lower.contains(name)), "software backend is not accepted: {backend:?}");
                    let (controller_logical, render_canvas) = size_state();
                    assert!(
                        (controller_logical[0] - render_canvas[0]).abs() <= 1.0
                            && (controller_logical[1] - render_canvas[1]).abs() <= 1.0,
                        "controller viewport size {controller_logical:?} and render session canvas {render_canvas:?} disagree after re-maximize"
                    );
                    eprintln!("real-GPU maximized/grid/crosshair evidence: backend={backend:?}, initial_screenshot={:?}, final_screenshot={:?}, requested_restored_size={:?}, actual_restored_size={:?}, actual_maximized_size={maximized_size:?}, initial_cad_rect_logical={canvas:?}, scale={scale}, controller_logical={controller_logical:?}, render_session_canvas={render_canvas:?}, initial_canvas_non_background_bounds_px={initial_bounds:?}, grid_canvas_non_background_bounds_px={grid_bounds:?}, final_canvas_non_background_bounds_px={bounds:?}, grid_edge_deltas_top_bottom_left_right={edge_deltas:?}, crosshair_hover_canvas_local={hover:?}; artifacts=01-maximized-initial.png,02-maximized-grid.png,03-maximized-crosshair-off-center.png,04-maximized-zoom-out.png,05-maximized-pan.png,06-restored-resized.png,07-maximized-after-resize.png", [initial.width, initial.height], [image.width, image.height], requested_size.unwrap(), resized_size.unwrap());
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
                let view = probe.view.borrow();
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
                let view = probe.view.borrow();
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
                render_frames = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    render_frames = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    render_frames = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    render_frames = probe.view.borrow().as_ref().unwrap().frames_rendered();
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
                    let backend = probe.view.borrow().as_ref().unwrap().backend_label();
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

    /// Render synthetic Chinese SFNT text through the real Linux window and
    /// hardware Vulkan path. This is intentionally separate from the grid test.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a real display, NVIDIA hardware Vulkan, and YACR_CJK_FONT"]
    fn real_gpu_cjk_sfnt_text_fill_acceptance() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        assert!(
            std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
        );
        for variable in [
            "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES",
            "VK_ADD_DRIVER_FILES",
            "VK_LOADER_DRIVERS_SELECT",
            "WGPU_ADAPTER_NAME",
            "MESA_VK_DEVICE_SELECT",
            "MESA_LOADER_DRIVER_OVERRIDE",
        ] {
            assert!(
                std::env::var_os(variable).is_none(),
                "refusing forced/software GPU selection from {variable}"
            );
        }
        let font = PathBuf::from(
            std::env::var("YACR_CJK_FONT")
                .expect("set YACR_CJK_FONT to a locally licensed CJK SFNT font"),
        );
        assert!(
            font.is_file(),
            "CJK font does not exist: {}",
            font.display()
        );
        let output = PathBuf::from(
            std::env::var("YACR_CJK_OUTPUT")
                .expect("set a fresh YACR_CJK_OUTPUT evidence directory"),
        );
        std::fs::create_dir(&output).expect("evidence directory must be new");
        let drawing = output.join("synthetic-cjk.dxf");
        std::fs::write(&drawing, "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n0\nSTYLE\n2\nStandard\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n24\n3\ncjk.ttf\n4\n\n0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nTEXT\n8\n0\n10\n0\n20\n0\n30\n0\n40\n24\n1\n中文空心字\n7\nStandard\n0\nENDSEC\n0\nEOF\n").unwrap();
        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(
            LinuxApp::new(
                LinuxOptions::parse([
                    "--open".into(),
                    drawing.display().to_string(),
                    "--font".into(),
                    format!("cjk.ttf={}", font.display()),
                ])
                .unwrap(),
            )
            .unwrap(),
        );
        let done = Rc::new(Cell::new(false));
        let completed = done.clone();
        let probe = app.clone();
        let deadline = Instant::now() + Duration::from_secs(120);
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            let ready = probe.view.borrow().as_ref().is_some_and(|view| {
                view.frames_rendered() > 0 && view.current_drawing_presented() && view.last_error().is_none() && view.scene_error().is_none()
            });
            if !ready {
                assert!(Instant::now() < deadline, "CJK drawing did not reach a clean presented frame: {}", real_window_readiness_diagnostic(&probe));
                return;
            }
            let image = probe.adapter.window().take_snapshot().unwrap();
            let (rect, _) = probe.adapter.handle().shell_geometry().unwrap();
            let pixels = canvas_region(&RgbaImage { width: image.width(), height: image.height(), pixels: image.as_bytes().to_vec() }, rect, probe.adapter.window().scale_factor() as f64);
            assert!(canvas_content_pixels(&pixels) > 20, "synthetic Chinese text produced no visible CAD pixels");
            let view = probe.view.borrow();
            let view = view.as_ref().unwrap();
            assert_eq!(view.backend_label().as_deref(), Some("vulkan"));
            let backend = format!("{:?}", view.active_backend());
            assert!(backend.contains("Native"), "expected native Vulkan desktop renderer, got {backend}");
            let fds = std::fs::read_dir("/proc/self/fd").unwrap().filter_map(Result::ok).filter_map(|entry| std::fs::read_link(entry.path()).ok()).collect::<Vec<_>>();
            assert!(fds.iter().any(|path| path == Path::new("/dev/nvidia0")), "renderer has no NVIDIA device fd: {fds:?}");
            std::fs::write(output.join("cjk-text.png"), encode_png(&RgbaImage { width: image.width(), height: image.height(), pixels: image.as_bytes().to_vec() }).unwrap()).unwrap();
            std::fs::write(output.join("renderer.txt"), format!("backend={backend}\napi=vulkan\nfont={}\ntext=中文空心字\nframes={}\nvisible_cad_pixels={}\nnvidia_device_fd=true\n", font.display(), view.frames_rendered(), canvas_content_pixels(&pixels))).unwrap();
            completed.set(true);
            slint::quit_event_loop().unwrap();
        });
        app.run().unwrap();
        assert!(done.get(), "CJK real-GPU acceptance exited before capture");
    }

    /// Real-window acceptance for the P3 **asynchronous cursor** path.
    ///
    /// The UI's own pointer-move callback must route `MoveCursor` through the
    /// engine `submit`/edge-waker (not a synchronous dispatch): at `BeginDraw`
    /// the capture has no cursor yet, and a `PointerMoved` must leave the cursor
    /// unset in the same call stack (a synchronous dispatch would set it
    /// inline). The queued request must then execute while the real event loop
    /// pumps (the app's waker/50 ms cycle, never a manual pump in the test), and
    /// the authoritative session preview must reach the shared render facade.
    /// A `Cancel` must clear the capture with **zero database writes**, and the
    /// settled window must not keep waking (`redraw_requests`/input token stay
    /// flat).
    ///
    /// Real winit window + hardware Vulkan; synthetic authorized input
    /// (`fixtures/dwg/synthetic-four-lines.dwg`); no vendor/golden claim.
    #[test]
    #[ignore = "requires a real display + hardware GPU; run in a dedicated process"]
    fn real_gpu_async_cursor_submit_wake_and_cancel_acceptance() {
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
            std::env::var("YACR_REAL_ASYNC_CURSOR_OUTPUT")
                .expect("runner must provide a fresh YACR_REAL_ASYNC_CURSOR_OUTPUT"),
        );
        std::fs::create_dir(&out).expect("evidence directory must be new");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures/dwg/synthetic-four-lines.dwg");
        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(
            LinuxApp::new(
                LinuxOptions::parse(["--open".into(), fixture.display().to_string()]).unwrap(),
            )
            .unwrap(),
        );

        let phase = Rc::new(Cell::new(0u8));
        let ticks = Rc::new(Cell::new(0u32));
        let completion = Rc::new(Cell::new(false));
        let mut deadline = Instant::now() + Duration::from_secs(120);
        let mut rect = [0.0_f64; 4];
        let mut local = [0.0_f64; 2];
        let mut baseline_entities = 0usize;
        let mut cursor_revision = 0u64;
        let mut cursor_frames = 0u64;
        let mut idle_redraws = 0u64;
        let mut idle_revision = 0u64;
        let mut cursor_checked = false;

        let timer = slint::Timer::default();
        let probe = app.clone();
        let phase_slot = phase.clone();
        let tick_slot = ticks.clone();
        let completion_slot = completion.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let ready = {
                    let view = probe.view.borrow();
                    view.as_ref().is_some_and(|view| {
                        let controller = probe.runtime.controller.borrow();
                        controller
                            .drawing()
                            .is_some_and(|drawing| drawing.entities().next().is_some())
                            && view.frames_rendered() > 0
                            && view.current_drawing_presented()
                            && view.last_error().is_none()
                            && view.scene_error().is_none()
                    })
                };
                if !ready {
                    assert!(
                        Instant::now() < deadline,
                        "async-cursor open/presentation deadline: {}",
                        real_window_readiness_diagnostic(&probe)
                    );
                    return;
                }
                match phase_slot.get() {
                    0 => {
                        // Start a LINE capture through the real UI callback
                        // (BeginDraw goes through the engine's sync barrier).
                        let (r, _) = probe.adapter.handle().shell_geometry().unwrap();
                        rect = r;
                        local = [r[2] * 0.40, r[3] * 0.40];
                        baseline_entities = probe
                            .runtime
                            .controller
                            .borrow()
                            .drawing()
                            .map(|drawing| drawing.entities().count())
                            .unwrap_or(0);
                        let label = cad_ui_slint::MessageSource::from_request("zh-CN")
                            .text("draw.kind.line", &[]);
                        probe.adapter.component().invoke_begin_draw_tool(label.into());
                        deadline = Instant::now() + Duration::from_secs(15);
                        phase_slot.set(1);
                    }
                    1 => {
                        let capture_active = {
                            let controller = probe.runtime.controller.borrow();
                            controller.session.draw_preview().is_some()
                        };
                        if !capture_active {
                            assert!(
                                Instant::now() < deadline,
                                "BeginDraw did not start a session capture: {}",
                                real_window_readiness_diagnostic(&probe)
                            );
                            return;
                        }
                        {
                            let controller = probe.runtime.controller.borrow();
                            let cursor = controller
                                .session
                                .draw_preview()
                                .and_then(|preview| preview.cursor);
                            assert!(
                                cursor.is_none(),
                                "a fresh draw capture must have no cursor yet"
                            );
                        }
                        {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            cursor_revision = view.render_session().input_revision();
                            cursor_frames = view.frames_rendered();
                        }
                        let position = slint::LogicalPosition::new(
                            (rect[0] + local[0]) as f32,
                            (rect[1] + local[1]) as f32,
                        );
                        probe.adapter.window().dispatch_event(
                            slint::platform::WindowEvent::PointerMoved { position },
                        );
                        // The UI must have submitted the cursor asynchronously:
                        // a synchronous dispatch would have set it inline.
                        let immediate = probe
                            .runtime
                            .controller
                            .borrow()
                            .session
                            .draw_preview()
                            .and_then(|preview| preview.cursor);
                        assert!(
                            immediate.is_none(),
                            "the pointer-move must submit MoveCursor asynchronously, not dispatch it synchronously"
                        );
                        deadline = Instant::now() + Duration::from_secs(15);
                        phase_slot.set(2);
                    }
                    2 => {
                        let session_preview = probe
                            .runtime
                            .controller
                            .borrow()
                            .session
                            .draw_preview();
                        let Some(preview) = session_preview else {
                            assert!(
                                Instant::now() < deadline,
                                "the queued MoveCursor was not executed by the event loop: {}",
                                real_window_readiness_diagnostic(&probe)
                            );
                            return;
                        };
                        if preview.cursor.is_none() {
                            assert!(
                                Instant::now() < deadline,
                                "async MoveCursor did not set a preview cursor: {}",
                                real_window_readiness_diagnostic(&probe)
                            );
                            return;
                        }
                        if !cursor_checked {
                            let view_preview = probe
                                .view
                                .borrow()
                                .as_ref()
                                .unwrap()
                                .draw_preview();
                            assert_eq!(
                                view_preview,
                                Some(preview.clone()),
                                "the render overlay preview must mirror the authoritative session preview"
                            );
                            let (expected, tolerance) = {
                                let controller = probe.runtime.controller.borrow();
                                let viewport = &controller.application.workspace
                                    .viewports[&controller.viewport_id];
                                (
                                    viewport.screen_to_world(local, viewport.logical_size),
                                    viewport.world_per_px() * 0.5,
                                )
                            };
                            let expected = expected.expect("cursor local point must map into the viewport");
                            let actual = preview.cursor.unwrap();
                            assert!(
                                (actual.x - expected.x).abs() <= tolerance
                                    && (actual.y - expected.y).abs() <= tolerance,
                                "async cursor world point {actual:?} must match the viewport mapping of local {local:?} -> {expected:?} (tol {tolerance})"
                            );
                            {
                                let view = probe.view.borrow();
                                let view = view.as_ref().unwrap();
                                assert!(
                                    view.render_session().input_revision() > cursor_revision,
                                    "the async cursor preview must advance the render input token"
                                );
                                assert!(
                                    view.frames_rendered() > cursor_frames,
                                    "the waker/event loop must render the cursor preview frame"
                                );
                                let image = probe.adapter.window().take_snapshot().unwrap();
                                std::fs::write(
                                    out.join("01-async-cursor-preview.png"),
                                    encode_png(&RgbaImage {
                                        width: image.width(),
                                        height: image.height(),
                                        pixels: image.as_bytes().to_vec(),
                                    })
                                    .unwrap(),
                                )
                                .unwrap();
                            }
                            // Observation only (not the event source): the
                            // queued async request must be drained.
                            let report = probe.binding.pump_requests().unwrap();
                            assert!(
                                !report.pending,
                                "the async cursor request must be drained"
                            );
                            cursor_checked = true;
                        }
                        let entities = probe
                            .runtime
                            .controller
                            .borrow()
                            .drawing()
                            .map(|drawing| drawing.entities().count())
                            .unwrap_or(0);
                        assert_eq!(
                            entities, baseline_entities,
                            "moving the draw cursor must not write to the database"
                        );
                        // Cancel (sync barrier) clears the capture, zero writes.
                        probe.adapter.component().invoke_cancel_draw_requested();
                        {
                            let controller = probe.runtime.controller.borrow();
                            assert!(
                                controller.session.draw_preview().is_none(),
                                "Cancel must clear the draw capture"
                            );
                        }
                        let entities_after = probe
                            .runtime
                            .controller
                            .borrow()
                            .drawing()
                            .map(|drawing| drawing.entities().count())
                            .unwrap_or(0);
                        assert_eq!(
                            entities_after, baseline_entities,
                            "Cancel must not write to the database"
                        );
                        std::fs::write(
                            out.join("async-cursor.txt"),
                            format!(
                                "fixture={}\nlocal_cursor={local:?}\nbaseline_entities={baseline_entities}\nbackend=native-vulkan\n",
                                fixture.display()
                            ),
                        )
                        .unwrap();
                        // Settle one tick, then require a flat idle window.
                        tick_slot.set(0);
                        deadline = Instant::now() + Duration::from_secs(5);
                        phase_slot.set(3);
                    }
                    3 => {
                        let t = tick_slot.get() + 1;
                        tick_slot.set(t);
                        if t == 1 {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            idle_redraws = view.redraw_requests();
                            idle_revision = view.render_session().input_revision();
                            return;
                        }
                        if t < 9 {
                            return;
                        }
                        {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            assert_eq!(
                                view.redraw_requests(),
                                idle_redraws,
                                "a settled window must not keep waking/redrawing"
                            );
                            assert_eq!(
                                view.render_session().input_revision(),
                                idle_revision,
                                "a settled window must not churn the render input token"
                            );
                            // Hardware proof: real native Vulkan, never software.
                            assert_eq!(
                                view.backend_label().as_deref(),
                                Some("vulkan"),
                                "async-cursor acceptance requires the real Vulkan backend"
                            );
                            assert!(
                                format!("{:?}", view.active_backend()).contains("Native"),
                                "expected the native desktop Vulkan renderer"
                            );
                        }
                        let fds = std::fs::read_dir("/proc/self/fd")
                            .unwrap()
                            .filter_map(Result::ok)
                            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                            .collect::<Vec<_>>();
                        assert!(
                            fds.iter().any(|path| path == Path::new("/dev/nvidia0")),
                            "async-cursor acceptance requires the NVIDIA device fd: {fds:?}"
                        );
                        completion_slot.set(true);
                        slint::quit_event_loop().unwrap();
                        phase_slot.set(4);
                    }
                    _ => {}
                }
            },
        );
        app.run().unwrap();
        assert!(
            completion.get(),
            "async-cursor acceptance did not reach the idle check"
        );
        assert_eq!(phase.get(), 4);
    }

    /// Drain the bounded engine event ring for `sub` until it is empty.
    fn drain_engine_events(
        app: &LinuxApp,
        sub: cad_engine::events::SubscriptionId,
    ) -> Vec<std::sync::Arc<cad_engine::events::EventEnvelope>> {
        use cad_engine::events::EventPoll;
        use cad_engine::protocol::EngineClient;
        let mut events = Vec::new();
        for _ in 0..16 {
            match app.runtime.poll_events(sub, 256).expect("poll_events") {
                EventPoll::Events(batch) if batch.is_empty() => break,
                EventPoll::Events(batch) => events.extend(batch),
                EventPoll::Lagged { .. } => break,
            }
        }
        events
    }

    /// Real-window acceptance for the P4 controlled API and bounded event stream.
    ///
    /// Runs the Host `EngineClient` in the real window through
    /// `dispatch_with_origin(Api, …)`: a controlled `Commit { Insert Line }` must
    /// create a real entity/revision/history step and surface its `ChangeSet` as
    /// an `EntitiesChanged` event sharing the exact same `Arc` (with the request
    /// id/origin/order carried on the envelopes); `SelectByObjectIds` must produce
    /// a real selection whose renderer highlight appears only after the event
    /// loop; typed `Undo`/`Redo` must reflect the same transaction in the
    /// database, revision, history and events; an unknown subscription must be
    /// explicitly refused. No raw database writes, no fake ids, no no-op events.
    ///
    /// Real winit window + hardware Vulkan; synthetic authorized input
    /// (`fixtures/dwg/synthetic-four-lines.dwg`); no vendor/golden claim.
    #[test]
    #[ignore = "requires a real display + hardware GPU; run in a dedicated process"]
    fn real_gpu_controlled_api_events_and_history_acceptance() {
        use cad_domain::{EntityId, LayerId, ObjectId, Point3, SpaceId};
        use cad_engine::api::{ApiRequest, ApiTarget, EntityEdit, NewEntity, SelectionMode};
        use cad_engine::events::{EngineEvent, SubscriptionId};
        use cad_engine::protocol::{EngineClient, EngineRequest, RequestOrigin, RequestResult};
        use std::cell::Cell;
        use std::sync::Arc;
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
            std::env::var("YACR_REAL_P4_OUTPUT")
                .expect("runner must provide a fresh YACR_REAL_P4_OUTPUT"),
        );
        std::fs::create_dir(&out).expect("evidence directory must be new");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures/dwg/synthetic-four-lines.dwg");
        cad_ui_slint::select_wgpu_backend_with(cad_render_wgpu::GpuSelection::Auto).unwrap();
        let app = Rc::new(
            LinuxApp::new(
                LinuxOptions::parse(["--open".into(), fixture.display().to_string()]).unwrap(),
            )
            .unwrap(),
        );

        let phase = Rc::new(Cell::new(0u8));
        let ticks = Rc::new(Cell::new(0u32));
        let done = Rc::new(Cell::new(false));
        let mut deadline = Instant::now() + Duration::from_secs(120);
        let mut subscription: Option<SubscriptionId> = None;
        let mut created: Option<ObjectId> = None;
        let mut pre_select_frames = 0u64;
        let mut idle_redraws = 0u64;
        let mut idle_revision = 0u64;

        let timer = slint::Timer::default();
        let probe = app.clone();
        let phase_slot = phase.clone();
        let tick_slot = ticks.clone();
        let done_slot = done.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let ready = {
                    let view = probe.view.borrow();
                    view.as_ref().is_some_and(|view| {
                        let controller = probe.runtime.controller.borrow();
                        controller
                            .drawing()
                            .is_some_and(|drawing| drawing.entities().next().is_some())
                            && view.frames_rendered() > 0
                            && view.current_drawing_presented()
                            && view.last_error().is_none()
                            && view.scene_error().is_none()
                    })
                };
                if !ready {
                    assert!(
                        Instant::now() < deadline,
                        "P4 open/presentation deadline: {}",
                        real_window_readiness_diagnostic(&probe)
                    );
                    return;
                }
                match phase_slot.get() {
                    0 => {
                        let (doc, db, viewport, revision, before_count, undo_before) = {
                            let controller = probe.runtime.controller.borrow();
                            let drawing = controller.drawing().unwrap();
                            let undo_before = controller
                                .application
                                .history
                                .get(&controller.document_id)
                                .map_or(0, |h| h.undo_depth());
                            (
                                controller.document_id,
                                drawing.id(),
                                controller.viewport_id,
                                drawing.revision(),
                                drawing.entities().count(),
                                undo_before,
                            )
                        };
                        let target = ApiTarget {
                            document: doc,
                            database: db,
                            viewport,
                        };
                        let sub = probe.runtime.subscribe().expect("subscribe must succeed");
                        subscription = Some(sub);

                        // 1. Controlled API commit creates a real entity/revision/history.
                        let line = EntityEdit::Insert {
                            layer: LayerId(0),
                            space: SpaceId::Model,
                            entity: NewEntity::Line {
                                start: Point3 {
                                    x: -1.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                                end: Point3 { x: 1.0, y: 0.0, z: 0.0 },
                            },
                        };
                        let completion = probe
                            .runtime
                            .dispatch_with_origin(
                                RequestOrigin::Api,
                                EngineRequest::Api {
                                    target,
                                    request: ApiRequest::Commit {
                                        expected_revision: revision,
                                        edits: vec![line],
                                    },
                                },
                            )
                            .expect("api commit must dispatch");
                        let request_id = completion.id;
                        let outcome = match completion.result {
                            Ok(RequestResult::Api(outcome)) => outcome,
                            other => panic!("unexpected API commit result: {other:?}"),
                        };
                        assert_eq!(
                            outcome.created.len(),
                            1,
                            "commit must create exactly one entity"
                        );
                        let object = outcome.created[0];
                        created = Some(object);
                        // The completion shares an `Arc<ApiOutcome>`; the `changes`
                        // field is an `Arc<ChangeSet>` clone, not a deep copy.
                        let changes = outcome
                            .changes
                            .clone()
                            .expect("commit must report a ChangeSet");
                        assert_eq!(
                            changes.after.0,
                            revision.0 + 1,
                            "commit must bump the revision by exactly one"
                        );
                        {
                            let controller = probe.runtime.controller.borrow();
                            let drawing = controller.drawing().unwrap();
                            assert!(
                                drawing.entity(EntityId(object.0)).is_some(),
                                "the created entity must exist in the authoritative database"
                            );
                            assert_eq!(drawing.revision().0, revision.0 + 1);
                            assert_eq!(
                                drawing.entities().count(),
                                before_count + 1,
                                "exactly one entity must be added"
                            );
                            let undo = controller
                                .application
                                .history
                                .get(&doc)
                                .map_or(0, |h| h.undo_depth());
                            assert_eq!(
                                undo,
                                undo_before + 1,
                                "the commit must record exactly one history step"
                            );
                        }

                        // 2. The event stream shares the exact ChangeSet Arc with the
                        //    request id/origin/order.
                        let events = drain_engine_events(&probe, sub);
                        let entities_index = events
                            .iter()
                            .position(|env| {
                                matches!(&env.event, EngineEvent::EntitiesChanged { changes: ev } if Arc::ptr_eq(ev, &changes))
                            })
                            .expect("commit must publish an EntitiesChanged sharing the outcome Arc");
                        let entities_env = &events[entities_index];
                        assert_eq!(entities_env.origin, RequestOrigin::Api);
                        assert_eq!(entities_env.request_id, Some(request_id));
                        assert_eq!(
                            entities_env.transaction,
                            Some(changes.transaction),
                            "the event must carry the shared transaction"
                        );
                        let started_index = events
                            .iter()
                            .position(|env| {
                                env.request_id == Some(request_id)
                                    && matches!(&env.event, EngineEvent::RequestStarted { .. })
                            })
                            .expect("the API request must publish RequestStarted");
                        let finished_index = events
                            .iter()
                            .position(|env| {
                                env.request_id == Some(request_id)
                                    && matches!(&env.event, EngineEvent::RequestFinished { .. })
                            })
                            .expect("the API request must publish RequestFinished");
                        assert!(
                            started_index < entities_index && entities_index < finished_index,
                            "the API request events must be ordered Started < EntitiesChanged < Finished"
                        );
                        assert!(
                            events.windows(2).all(|w| w[0].sequence < w[1].sequence),
                            "event sequences must be strictly increasing"
                        );

                        // 3. Controlled selection by the created object id.
                        pre_select_frames = probe.view.borrow().as_ref().unwrap().frames_rendered();
                        let select = probe
                            .runtime
                            .dispatch_with_origin(
                                RequestOrigin::Api,
                                EngineRequest::Api {
                                    target,
                                    request: ApiRequest::SelectByObjectIds {
                                        ids: vec![object],
                                        mode: SelectionMode::Replace,
                                    },
                                },
                            )
                            .expect("api select must dispatch");
                        let select_id = select.id;
                        match select.result {
                            Ok(RequestResult::Api(outcome)) => {
                                assert!(
                                    outcome.created.is_empty() && outcome.changes.is_none(),
                                    "a selection carries no created entities or changes"
                                );
                            }
                            other => panic!("unexpected API select result: {other:?}"),
                        }
                        let select_events = drain_engine_events(&probe, sub);
                        assert!(
                            select_events.iter().any(|env| {
                                env.request_id == Some(select_id)
                                    && env.origin == RequestOrigin::Api
                                    && matches!(&env.event, EngineEvent::SelectionChanged { refs, .. } if refs.iter().any(|r| r.entity == EntityId(object.0)))
                            }),
                            "the API selection must publish a SelectionChanged for the created entity"
                        );
                        deadline = Instant::now() + Duration::from_secs(15);
                        phase_slot.set(1);
                    }
                    1 => {
                        let selected = {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            view.selection_highlight()
                                .refs()
                                .iter()
                                .any(|reference| reference.entity == EntityId(created.unwrap().0))
                        };
                        if !selected {
                            assert!(
                                Instant::now() < deadline,
                                "the API selection did not reach the renderer highlight: {}",
                                real_window_readiness_diagnostic(&probe)
                            );
                            return;
                        }
                        {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            assert!(
                                view.frames_rendered() > pre_select_frames,
                                "the selection highlight must render a new frame"
                            );
                            let image = probe.adapter.window().take_snapshot().unwrap();
                            std::fs::write(
                                out.join("01-p4-api-select-highlight.png"),
                                encode_png(&RgbaImage {
                                    width: image.width(),
                                    height: image.height(),
                                    pixels: image.as_bytes().to_vec(),
                                })
                                .unwrap(),
                            )
                            .unwrap();
                        }
                        let object = created.unwrap();
                        let document = probe.runtime.controller.borrow().document_id;

                        // 4. Typed Undo/Redo reflect the shared transaction.
                        probe
                            .runtime
                            .command(cad_app::CommandId::Undo, cad_app::CommandPayload::None)
                            .expect("typed Undo must run through the host funnel");
                        {
                            let controller = probe.runtime.controller.borrow();
                            let drawing = controller.drawing().unwrap();
                            assert!(
                                drawing.entity(EntityId(object.0)).is_none(),
                                "Undo must remove the API-created entity"
                            );
                            assert!(
                                drawing.revision().0 > 1,
                                "Undo must advance the revision"
                            );
                            let history = controller.application.history.get(&document).unwrap();
                            assert_eq!(history.redo_depth(), 1, "Undo must leave one redo step");
                        }
                        let undo_events = drain_engine_events(&probe, subscription.unwrap());
                        assert!(
                            undo_events.iter().any(|env| matches!(
                                &env.event,
                                EngineEvent::CommandStarted { command } if *command == cad_app::CommandId::Undo
                            )),
                            "typed Undo must emit CommandStarted"
                        );
                        assert!(
                            undo_events.iter().any(|env| matches!(
                                &env.event,
                                EngineEvent::CommandFinished { command, .. } if *command == cad_app::CommandId::Undo
                            )),
                            "typed Undo must emit CommandFinished"
                        );

                        probe
                            .runtime
                            .command(cad_app::CommandId::Redo, cad_app::CommandPayload::None)
                            .expect("typed Redo must run through the host funnel");
                        {
                            let controller = probe.runtime.controller.borrow();
                            let drawing = controller.drawing().unwrap();
                            assert!(
                                drawing.entity(EntityId(object.0)).is_some(),
                                "Redo must restore the API-created entity"
                            );
                            let history = controller.application.history.get(&document).unwrap();
                            assert_eq!(history.redo_depth(), 0, "Redo consumes the redo step");
                            assert!(history.undo_depth() >= 1);
                        }
                        let redo_events = drain_engine_events(&probe, subscription.unwrap());
                        assert!(
                            redo_events.iter().any(|env| matches!(
                                &env.event,
                                EngineEvent::CommandFinished { command, .. } if *command == cad_app::CommandId::Redo
                            )),
                            "typed Redo must emit CommandFinished"
                        );

                        // 5. Unknown subscriptions are refused explicitly.
                        let unknown = SubscriptionId(u64::MAX);
                        assert!(
                            probe.runtime.unsubscribe(unknown).is_err(),
                            "unsubscribing an unknown id must be an explicit error"
                        );
                        probe
                            .runtime
                            .unsubscribe(subscription.unwrap())
                            .expect("unsubscribing the live id must succeed");

                        tick_slot.set(0);
                        deadline = Instant::now() + Duration::from_secs(5);
                        phase_slot.set(2);
                    }
                    2 => {
                        let t = tick_slot.get() + 1;
                        tick_slot.set(t);
                        if t == 1 {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            idle_redraws = view.redraw_requests();
                            idle_revision = view.render_session().input_revision();
                            return;
                        }
                        if t < 9 {
                            return;
                        }
                        {
                            let view = probe.view.borrow();
                            let view = view.as_ref().unwrap();
                            assert_eq!(
                                view.redraw_requests(),
                                idle_redraws,
                                "a settled window must not keep waking/redrawing"
                            );
                            assert_eq!(
                                view.render_session().input_revision(),
                                idle_revision,
                                "a settled window must not churn the render input token"
                            );
                            assert_eq!(
                                view.backend_label().as_deref(),
                                Some("vulkan"),
                                "P4 acceptance requires the real Vulkan backend"
                            );
                            assert!(
                                format!("{:?}", view.active_backend()).contains("Native"),
                                "expected the native desktop Vulkan renderer"
                            );
                        }
                        let fds = std::fs::read_dir("/proc/self/fd")
                            .unwrap()
                            .filter_map(Result::ok)
                            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                            .collect::<Vec<_>>();
                        assert!(
                            fds.iter().any(|path| path == Path::new("/dev/nvidia0")),
                            "P4 acceptance requires the NVIDIA device fd: {fds:?}"
                        );
                        assert!(
                            !probe.binding.pump_requests().unwrap().pending,
                            "no queued request may remain after the controlled API flow"
                        );
                        std::fs::write(
                            out.join("p4-api-events.txt"),
                            format!(
                                "fixture={}\ncreated_object={:?}\nbackend=native-vulkan\n",
                                fixture.display(),
                                created
                            ),
                        )
                        .unwrap();
                        done_slot.set(true);
                        slint::quit_event_loop().unwrap();
                        phase_slot.set(3);
                    }
                    _ => {}
                }
            },
        );
        app.run().unwrap();
        assert!(done.get(), "P4 acceptance did not reach the idle check");
        assert_eq!(phase.get(), 3);
    }
}
