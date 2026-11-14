# DGN import — Linux native Wayland, real NVIDIA GPU

Date: 2026-11-14 (local host).

This is a real-GPU visual run of the new `cad-import-dgn` lane, not a headless
software-rendering probe.

## Environment

- Host: existing GNOME/Mutter Wayland session (`/run/user/1000/wayland-0`,
  `XDG_RUNTIME_DIR=/run/user/1000`).
- Compositor for the run: **Weston 14.0.2**, nested Wayland backend with the GL
  renderer (`--backend=wayland --renderer=gl --socket=wayland-dgn
  --width=1600 --height=1000 --debug`). Weston itself is a real Wayland client
  of the GNOME session; it is not Xvfb and not a software framebuffer.
- GPU: **NVIDIA Quadro P620** (GP107GLM), driver 580.178.04,
  `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/nvidia_icd.json`,
  `DRI_PRIME=1`, `__NV_PRIME_RENDER_OFFLOAD=1`.
- App: `target/debug/yacr --open fixtures/dgn/{smalltest,test_dgnv8}.dgn`,
  `GDK_BACKEND=wayland`, `WGPU_BACKEND=vulkan`.
- Screenshot: `weston-screenshooter` against the nested Weston socket (the
  compositor's own capture path; no GNOME Shell screenshot/portal).

## Probe evidence

The host's own probe reported the real adapter and a progressing present loop:

```
[probe] bridge wayland presenter ready generation=1 backend=vulkan
        adapter=Quadro P620 device_type=discrete_gpu present_mode=Mailbox
[probe] status backend=Some("vulkan") adapter=Some("Quadro P620")
        size=1841x983 frames=100 ticks=173 present_mode=Some("Mailbox") ...
```

The exact excerpt is in `probe.txt`. `frames` increases over time; the window is
the real `native-below` Wayland host path (`bridge wayland`, native CAD surface
below the transparent WebView), not the X11 `dom-canvas` path.

## Screenshots

- `dgn-v7-smalltest.png` — V7 `fixtures/dgn/smalltest.dgn`: `Demo Text`, the
  large circle, the closed red polyline and the line are visible in the CAD
  viewport.
- `dgn-v8-test.png` — V8 `fixtures/dgn/test_dgnv8.dgn`: `myTéxt`, arcs, the
  ellipse, lines and the shared-cell/point-string geometry are visible. The tab
  title names the opened file.

Both screenshots show the normal React chrome and the tab
`fixtures/dgn/<file>.dgn`; the viewport content is the imported DGN database
rendered by wgpu/Vulkan.

## Boundary

This proves:

- a DGN V7 and a DGN V8 file open through the real dispatcher and are drawn by
  the native Linux host on a **real discrete GPU** (Vulkan P620);
- the import produces visible vector geometry (text, circle/arc/ellipse,
  polyline/line, shared-cell children) rather than an empty success.

This does **not** prove:

- Bentley/MicroStation acceptance or V8i/CONNECT/ORD compatibility: the files
  are the GDAL V7 toy and the ODA-Teigha-written V8 toy;
- measurement correctness, pixel/golden equality, performance, or release-build
  behaviour (the run used the debug binary);
- Windows/macOS/Android/Web behaviour, or the GNOME-native compositor path
  without the nested Weston compositor. A nested compositor adds one GPU copy;
  the app's own wgpu adapter is still the real P620.
- The import-report `Partial` diagnostics are not asserted by the screenshot;
  the crate contract tests cover those separately.
