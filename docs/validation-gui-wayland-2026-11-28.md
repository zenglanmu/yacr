# Linux native Wayland host — real GUI/GPU evidence (2026-11-28)

This is a **real same-UID GNOME/Mutter Wayland session run** of the release
`yacr` binary on the primary monitor, not a headless/virtual display and not a
software renderer. It is bounded evidence for one fixture and one carrier; it is
not a product acceptance or a pixel/performance claim.

## 1. Environment and command

- Host: GNOME/Mutter Wayland, `WAYLAND_DISPLAY=wayland-0`, primary `eDP-1`
  1920x1080, NVIDIA driver 580.178.04, Quadro P620.
- The session was initially **locked**: Mutter ScreenCast answered
  `Session creation inhibited` and `gnome-screenshot` fell back to a black X11
  read. After `loginctl unlock-session 1` (same user; the environment's standing
  authorization permits session unlocking) ScreenCast succeeded. No credentials
  were entered by the harness and no portal permission was changed.
- Command (fresh output directory, frontend + release build included):

```bash
bash scripts/collect-evidence.sh \
  --backend wayland \
  --out /tmp/opencode/yacr-evidence-20261128-wayland3 \
  --capture-at 10 --timeout 80
```

Exact result row (`docs/evidence/linux-wayland-2026-11-28/results.tsv`):

```
wayland  production  NATURAL_EXIT  0  captured  2026-11-27T19:34:56Z  app exited on its own; surfaceMode=native-below; production chrome over CAD; compositor frame (Mutter ScreenCast) of the full monitor
```

## 2. Runtime/render evidence

From the same run's log excerpt (`docs/evidence/linux-wayland-2026-11-28/run-excerpt.log`):

```
display backend policy=Wayland resolved=Wayland source=from environment
cad session created
initial configure took 9115 us surface=hwnd
bridge wayland presenter ready generation=1 backend=vulkan adapter=Quadro P620 device_type=discrete_gpu present_mode=Mailbox configure_us=9115
actual-render seq=1 ... draw_calls=2 verts=24 tris=8 ... complete=true
status backend=Some("vulkan") adapter=Some("Quadro P620") ... frames=... present_mode=Some("Mailbox")
```

The process exited naturally with code 0 after the screenshot window. The final
status reported Vulkan on the discrete Quadro P620 (Mailbox present mode) and
thousands of presented frames from the native wgpu surface.

## 3. Visual inspection

The full-monitor frame was inspected with the model's vision capability. The
committed `docs/evidence/linux-wayland-2026-11-28/wayland-app-crop.png` is a
**crop of that compositor frame** to the Yacr window (the full monitor frame
remains at `/tmp/opencode/yacr-evidence-20261128-wayland3/wayland/wayland-frame.png`
and is not committed because it contains unrelated desktop windows).

Observed in the crop:

- The real Yacr window: title bar, zh-CN chrome, dark theme, ribbon groups
  (`常用`/`插入`/`注释`/`参数化`/`视图`/`管理`/`输出`/`附加模块`), model/Layout1 tabs,
  command line, status bar and the ViewCube on the right.
- The CAD area is black (the dark theme) and shows the four white lines of
  `synthetic-four-lines.dwg` (three vertical, one horizontal) with no visible
  blank-canvas refusal or renderer error banner.
- The app was launched with the evidence runner's no-maximize setting, so the
  window is intentionally not full-screen.

## 4. What this does NOT prove

- It is **one carrier** (Linux native Wayland `native-below`) and **one DWG
  fixture**; X11/`dom-canvas`, Windows, macOS, Android and Web are NOT VERIFIED.
- A compositor screenshot is not evidence of pixel accuracy, anti-aliasing,
  color management, performance, input handling or stacking correctness.
- The screenshot is a full-monitor compositor capture, then cropped for the
  committed artifact; unrelated desktop windows were present in the original.
- It does not prove any of the recent material/guide/Outliner/scene protocol
  slices work through real UI clicks; their evidence remains contract tests.

## 5. Reproduce

Run the command in §1 after unlocking a real GNOME Wayland session. If ScreenCast
still reports `Session creation inhibited`, the session is locked or another
screencast holds Mutter's single-session gate; the runner will record
`screenshot=missing` rather than fake a frame.
