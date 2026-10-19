use super::*;
use std::ffi::c_void;
use std::ptr::NonNull;
use wgpu::rwh::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle, XlibDisplayHandle, XlibWindowHandle,
};

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(name: *const i8) -> *mut c_void;
    fn XDefaultRootWindow(display: *mut c_void) -> u64;
    fn XCreateSimpleWindow(
        display: *mut c_void,
        parent: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        border_width: u32,
        border: u64,
        background: u64,
    ) -> u64;
    fn XMapWindow(display: *mut c_void, window: u64) -> i32;
    fn XResizeWindow(display: *mut c_void, window: u64, width: u32, height: u32) -> i32;
    fn XFlush(display: *mut c_void) -> i32;
    fn XDestroyWindow(display: *mut c_void, window: u64) -> i32;
    fn XCloseDisplay(display: *mut c_void) -> i32;
}

/// A minimal owning Xlib connection + window, exposed through
/// raw-window-handle so `SurfaceProbePresenter` can own it.
struct XlibWindow {
    display: *mut c_void,
    window: u64,
    screen: i32,
}

// The connection is only used from the single test thread that owns the
// window; the probe requires `Send + Sync` providers, so assert it here.
unsafe impl Send for XlibWindow {}
unsafe impl Sync for XlibWindow {}

impl XlibWindow {
    fn open(width: u32, height: u32) -> Option<Self> {
        unsafe {
            let display = XOpenDisplay(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let root = XDefaultRootWindow(display);
            let window = XCreateSimpleWindow(display, root, 0, 0, width, height, 0, 0, 0);
            if window == 0 {
                XCloseDisplay(display);
                return None;
            }
            XMapWindow(display, window);
            XFlush(display);
            Some(Self {
                display,
                window,
                screen: 0,
            })
        }
    }
}

impl Drop for XlibWindow {
    fn drop(&mut self) {
        unsafe {
            XDestroyWindow(self.display, self.window);
            XCloseDisplay(self.display);
        }
    }
}

impl HasWindowHandle for XlibWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Xlib(XlibWindowHandle::new(self.window));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for XlibWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Xlib(XlibDisplayHandle::new(
            NonNull::new(self.display),
            self.screen,
        ));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

#[test]
#[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
fn probe_presents_grid_and_rebuilds() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
        return;
    }
    let window = match XlibWindow::open(320, 240) {
        Some(window) => window,
        None => {
            eprintln!("SKIP: could not open an Xlib display");
            return;
        }
    };
    let raw_display = window.display;
    let raw_window = window.window;

    let mut presenter = SurfaceProbePresenter::new(window, 320, 240)
        .expect("the surface probe must initialize on a real GPU");
    eprintln!(
        "probe adapter: backend={} name={} device_type={}",
        presenter.adapter().backend,
        presenter.adapter().name,
        presenter.adapter().device_type
    );
    assert_eq!(
        presenter.adapter().backend,
        "vulkan",
        "the probe forces Vulkan"
    );
    assert_ne!(
        presenter.adapter().device_type,
        "cpu",
        "the probe must reject software/fallback adapters"
    );
    assert_eq!(presenter.surface_size(), (320, 240));

    let first = presenter
        .frame(0.0)
        .expect("first probe frame must present");
    let second = presenter
        .frame(0.5)
        .expect("second probe frame must present");
    assert_eq!(second.frame_index, first.frame_index + 1);
    assert_eq!(second.marker_offset, 0.5);

    // A real host resizes its window together with the surface; keep them in
    // step so the swapchain is not reported `Outdated`.
    unsafe {
        XResizeWindow(raw_display, raw_window, 640, 480);
        XFlush(raw_display);
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    presenter
        .resize(640, 480)
        .expect("resize must reconfigure the surface");
    let resized = presenter
        .frame(0.25)
        .expect("frame after resize must present");
    assert_eq!(resized.surface_size, (640, 480));

    assert!(
        presenter.resize(0, 480).is_err(),
        "a zero dimension must be an explicit error, not a configure panic"
    );

    // Destroy and rebuild the whole lifecycle from a fresh window.
    drop(presenter);
    let window = XlibWindow::open(160, 120).expect("second Xlib window");
    let mut rebuilt =
        SurfaceProbePresenter::new(window, 160, 120).expect("the probe must rebuild cleanly");
    assert!(rebuilt.frame(0.1).is_ok());

    rebuilt.stop();
    assert!(rebuilt.is_stopped());
    assert!(matches!(rebuilt.frame(0.2), Err(ProbeError::Stopped)));
}

/// Stable control: run continuously for 10 s on a plain Xlib window and
/// report per-stage timings. At least 300 frames must present (>=30 fps).
#[test]
#[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
fn probe_continuous_frames_stage_timing() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
        return;
    }
    let window = match XlibWindow::open(320, 240) {
        Some(window) => window,
        None => {
            eprintln!("SKIP: could not open an Xlib display");
            return;
        }
    };
    let present_mode = match std::env::var("YACR_PROBE_PRESENT_MODE").as_deref() {
        Ok("mailbox") => ProbePresentMode::MailboxElseImmediate,
        _ => ProbePresentMode::Fifo,
    };
    let mut presenter =
        SurfaceProbePresenter::new_with_present_mode(window, 320, 240, present_mode)
            .expect("the surface probe must initialize on a real GPU");
    eprintln!(
        "[xlib-continuous] adapter={} device_type={} present_mode={:?}",
        presenter.adapter().name,
        presenter.adapter().device_type,
        presenter.present_mode()
    );

    fn summarize(name: &str, mut samples: Vec<u64>) {
        samples.sort_unstable();
        let n = samples.len();
        let median = if n == 0 { 0 } else { samples[n / 2] };
        let p90 = if n == 0 { 0 } else { samples[(n * 9) / 10] };
        let max = samples.last().copied().unwrap_or(0);
        eprintln!("[xlib-continuous] {name}: n={n} median_us={median} p90_us={p90} max_us={max}");
    }

    let duration = std::time::Duration::from_secs(10);
    let started = std::time::Instant::now();
    let mut frames = 0u64;
    let mut marker = 0.0f32;
    let mut write = Vec::new();
    let mut acquire = Vec::new();
    let mut submit = Vec::new();
    let mut scope = Vec::new();
    let mut present = Vec::new();
    let mut total = Vec::new();
    while started.elapsed() < duration {
        let stats = presenter
            .frame(marker)
            .expect("continuous frame must present");
        frames += 1;
        write.push(stats.timings.write_buffer_us);
        acquire.push(stats.timings.acquire_us);
        submit.push(stats.timings.submit_us);
        scope.push(stats.timings.scope_pop_us);
        present.push(stats.timings.present_us);
        total.push(stats.timings.total_us);
        marker = (marker + 0.01).rem_euclid(1.0);
    }
    eprintln!(
        "[xlib-continuous] frames={frames} in {:?}",
        started.elapsed()
    );
    summarize("write_buffer", write);
    summarize("acquire", acquire);
    summarize("submit", submit);
    summarize("scope_pop", scope);
    summarize("present", present);
    summarize("total", total);
    assert!(
        frames >= 300,
        "expected >=300 frames in 10s on a plain Xlib window, got {frames}"
    );
}

/// A synthetic checkerboard texture on the **same device** is presented.
/// This is a texture-present check, **not** CAD acceptance and not a
/// visual-accuracy claim; no GPU->CPU readback is used.
#[test]
#[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
fn present_native_texture_checkerboard() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
        return;
    }
    let window = match XlibWindow::open(320, 240) {
        Some(window) => window,
        None => {
            eprintln!("SKIP: could not open an Xlib display");
            return;
        }
    };
    let mut presenter =
        SurfaceProbePresenter::new(window, 320, 240).expect("presenter must initialize");
    let (device, queue) = presenter.device_queue();
    // `present_texture` requires the texture to match the configured surface
    // size (it refuses to stretch a mismatched texture), so the checkerboard is
    // the surface size.
    let (size_w, size_h) = presenter.surface_size();
    let mut pixels = vec![0u8; (size_w * size_h * 4) as usize];
    for y in 0..size_h {
        for x in 0..size_w {
            let c = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 0 };
            let i = ((y * size_w + x) * 4) as usize;
            pixels[i] = c;
            pixels[i + 1] = c;
            pixels[i + 2] = c;
            pixels[i + 3] = 255;
        }
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("checkerboard"),
        size: wgpu::Extent3d {
            width: size_w,
            height: size_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size_w * 4),
            rows_per_image: Some(size_h),
        },
        wgpu::Extent3d {
            width: size_w,
            height: size_h,
            depth_or_array_layers: 1,
        },
    );

    let key = NativeTextureKey {
        device_epoch: 1,
        texture_revision: 1,
        size: (size_w, size_h),
    };
    let first = presenter
        .present_texture(&texture, key)
        .expect("checkerboard must present");
    assert_eq!(first.frame_index, 1);
    let second = presenter
        .present_texture(&texture, key)
        .expect("second checkerboard frame must present");
    assert_eq!(second.frame_index, 2);

    // A changed key forces a rebind (same texture, new epoch).
    let rebound = NativeTextureKey {
        device_epoch: 2,
        texture_revision: 1,
        size: (size_w, size_h),
    };
    assert!(presenter.present_texture(&texture, rebound).is_ok());

    // A key whose size disagrees with the texture is an explicit error.
    let bad = NativeTextureKey {
        device_epoch: 2,
        texture_revision: 2,
        size: (128, 128),
    };
    assert!(matches!(
        presenter.present_texture(&texture, bad),
        Err(ProbeError::TextureInvalid(_))
    ));

    // A texture that matches its key but NOT the configured surface size is
    // rejected rather than stretched anisotropically: the blit covers the whole
    // surface with no aspect correction. The guard runs before the surface is
    // touched, so a 64x64 texture with a 64x64 key is refused on a 320x240
    // surface.
    let small = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mismatched-surface"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mismatched_surface = NativeTextureKey {
        device_epoch: 3,
        texture_revision: 1,
        size: (64, 64),
    };
    assert!(matches!(
        presenter.present_texture(&small, mismatched_surface),
        Err(ProbeError::TextureInvalid(_))
    ));
}
