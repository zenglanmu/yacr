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
