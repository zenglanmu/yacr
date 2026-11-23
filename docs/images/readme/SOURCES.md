# README screenshots — real Linux/Wayland runs

These are **real** screenshots of the `native-below` Linux host (`apps/app-linux`, binary
`yacr`) on this machine's GNOME/Mutter Wayland session, drawn by the app's own wgpu
presenter on the discrete **NVIDIA Quadro P620 (Vulkan)**. They are product screenshots,
not renders, mockups or headless captures.

They are **not** an acceptance record: no pixel/golden comparison, no performance claim,
and nothing about Windows/macOS/Android/Web. It is one Linux host with one GPU.

## How the frames were taken

1. Launch the real release binary with the resolved Wayland environment (the same
   environment `scripts/collect-evidence.sh --dry-run` resolves):

   ```bash
   env -u DISPLAY -u XAUTHORITY \
       GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
       YACR_PROBE_CAD_LOG=1 \
       ./target/release/yacr --open <sample> --fonts-dir fonts
   ```

   The binary was `target/release/yacr` (built 2026-11-22). `YACR_PROBE_LIFECYCLE` was
   **not** set: that driver resizes the window at 8 s/19 s and calls `app.exit` at 32 s,
   which would destroy a mid-run capture. The app was therefore launched directly instead
   of through `collect-evidence.sh`, with the identical env and the identical real
   `--open` path; `collect-evidence.sh`'s own screenshot step was only used while
   debugging and is **not** the source of any image here.

2. Capture the compositor's own ScreenCast frame (`scripts/screencast-capture.py`,
   same-UID Mutter), which yields a full **1920×1080** monitor frame.

3. Crop to the app window with PIL (`python3` + Pillow) and downscale to 1600 px wide.
   Crop box for every image: **`(66,32,1854,1048)` = the whole maximised window**
   (title bar included, GNOME dock excluded). Verified per source frame: the window's
   1 px top border sits at `y=32`, left border at `x=66`, and the window runs to the
   monitor's right/bottom edges.

### Why not a per-window screenshot

The brief asked for `org.gnome.Shell.Screenshot.ScreenshotWindow`. **That API does not
exist on this host**: it is GNOME Shell 50.1, where `/org/gnome/Screenshot` is an empty
node and `/org/gnome/Shell/Screenshot` exposes only the D-Bus core interfaces — all
screenshot methods were removed. `gnome-screenshot -w` falls back to X11 read-back and
fails (`gdk_pixbuf_get_from_surface: assertion 'width > 0 && height > 0' failed`; all
black on this compositor). The remaining portal (`org.freedesktop.portal.Screenshot`,
v2) is full-screen-only or interactive-picker, so it offers nothing beyond the ScreenCast
path already used. Hence **every image below is the documented fallback: full-screen
ScreenCast frame + crop**, and this file says so per row.

### UI language

All images are **English** (`Language: en` is visible in the title bar). The locale is
stored in the WebKit localStorage key `yacr.locale`; it was switched to `en` before
capturing and the UI was checked by eye in every frame that was kept.

## Images

Common environment for every row: `GDK_BACKEND=wayland`, `WAYLAND_DISPLAY=wayland-0`,
`XDG_RUNTIME_DIR=/run/user/1000`, `surfaceMode = native-below`, adapter line
`[probe] bridge wayland presenter ready generation=1 backend=vulkan adapter=Quadro P620
device_type=discrete_gpu present_mode=Mailbox` (read from that run's `run.log`).
"run command" below is the `./target/release/yacr …` part only; the env prefix is the
block above.

| File (1600×904) | Drawing opened | Backend / surfaceMode | GPU (from run.log) | Run command | Capture method | Suggested honest caption |
|---|---|---|---|---|---|---|
| `01-desktop-hero.png` | `fixtures/dxf/qcad-flange/flange.dxf` | Wayland, `native-below` | Quadro P620, Vulkan, `Mailbox` | `yacr --open fixtures/dxf/qcad-flange/flange.dxf --fonts-dir fonts` | full-screen ScreenCast + crop `(66,32,1854,1048)` | "yacr on Linux (native Wayland, real NVIDIA GPU): 2D DXF drawing with ribbon, command line and status bar." |
| `02-gltf-helmet.png` | `/tmp/opencode/samples/DamagedHelmet.gltf` | Wayland, `native-below` | Quadro P620, Vulkan, `Mailbox` | `yacr --open /tmp/opencode/samples/DamagedHelmet.gltf --fonts-dir fonts` (+ CAD script = 2× `ZOOMIN`) | full-screen ScreenCast + crop `(66,32,1854,1048)` | "glTF PBR import — DamagedHelmet with base-colour/normal/emissive textures, rendered by the native wgpu path." |
| `03-step-3d.png` | `/tmp/opencode/samples/weldment_asm_solid.stp` | Wayland, `native-below` | Quadro P620, Vulkan, `Mailbox` | `yacr --open /tmp/opencode/samples/weldment_asm_solid.stp --fonts-dir fonts` (+ CAD script = 2× `ZOOMIN`) | full-screen ScreenCast + crop `(66,32,1854,1048)` | "STEP AP203/214 assembly imported as a shaded 3D solid (weldment), whole part in view." |
| `06-skp-preview.png` | `/tmp/opencode/samples/house.skp` | Wayland, `native-below` | Quadro P620, Vulkan, `Mailbox` | `yacr --open /tmp/opencode/samples/house.skp --fonts-dir fonts` | full-screen ScreenCast + crop `(66,32,1854,1048)` | "SketchUp import: rooms, openings and textured floor as a shaded 3D preview." |
| `07-rvt-preview.png` | `/tmp/opencode/samples/rmeadvancedsampleproject.rvt` | Wayland, `native-below` | Quadro P620, Vulkan, `Mailbox` | `yacr --open /tmp/opencode/samples/rmeadvancedsampleproject.rvt --fonts-dir fonts` | full-screen ScreenCast + crop `(66,32,1854,1048)` | "RVT preview: decoded massing of an Autodesk sample project (white preview geometry, RVT coverage still `Unverified`)." |

## Per-image evidence lines

Adapter/surfaceMode lines actually present in each run's `run.log`:

```
01 cap-hero-en2 : [probe] display backend policy=Wayland resolved=Wayland source=from environment
                  [probe] bridge wayland presenter ready generation=1 backend=vulkan adapter=Quadro P620 device_type=discrete_gpu present_mode=Mailbox configure_us=11424
02 cap-helmet-4 : same policy line; presenter ready … adapter=Quadro P620 … configure_us=11755
                  [probe] cad script: t=7s window.__yacr_cad_script -> {"zoom":[{"accepted":true},{"accepted":true}],…,"loaded":"/tmp/opencode/samples/DamagedHelmet.gltf"}
03 cap-step-3   : same policy line; presenter ready … adapter=Quadro P620 … configure_us=12424
                  [probe] cad script: t=7s window.__yacr_cad_script -> {"zoom":[],"evalResult":"undefined"}
                  (the 2 s read-back landed while the load-wait poll was still running, so the
                   issued ZOOMIN calls are NOT in the logged read-back; the framing visible in
                   the image is the only evidence that they applied)
06 cap-skp-2    : same policy line; presenter ready … adapter=Quadro P620 … configure_us=11866
07 cap-rvt-4    : same policy line; presenter ready … adapter=Quadro P620 … configure_us=12409
```

Every row is `backend=vulkan`, `surfaceMode=native-below`, `adapter=Quadro P620`,
`device_type=discrete_gpu`. Software rendering (SwiftShader/lavapipe) was **not** used.

## Round 2 result

Requested: re-shoot 02/03/06/07 from the ASCII sample copies in `/tmp/opencode/samples/`
(round-1 tabs showed the Chinese `下载` path), delete 04/05, add `08-detail-2d.png` (dropped
afterwards — see the limitations below).

Status: **done.** All five images were re-taken in this round and the tabs now show
`/tmp/opencode/samples/…` (or `fixtures/…` for 08). `04-ifc-3d.png` and `05-dgn.png` were
deleted; `01-desktop-hero.png` is untouched.

An earlier attempt at this round failed with `Session creation inhibited` (Mutter allows
one ScreenCast session at a time and another one was held); once that recording was
stopped, capture worked and everything below was produced. Frames were taken as
5–6-frame bursts per run so a frame without the mouse pointer could be chosen.

## Known limitations / what was skipped

- **Long straight lines over the 3D views.** 02/03/06/07 still show one or two thin white
  lines crossing the model. They are **not** the crosshair: the crosshair was reduced to
  `sizePercent = 5` for this round and the lines did not change, and issuing `UCSICON`
  through the CAD script did not remove them either. They are drawn by the app's own
  viewport (world-axis / construction lines) on the black background; they were left
  as-is rather than retouched.
- **No mouse pointer in any shipped frame.** Each run produced a 5–6 frame burst and a
  frame whose window contained no system pointer was chosen; no frame needed the
  `cursor visible` caveat.
- **No 2D-detail image.** An `08-detail-2d.png` was produced once from
  `fixtures/test-hatch-render.dwg`, but its status bar carried a real
  `resource.missing …` diagnostic (the DWG references an AutoCAD support path that does
  not exist here), so it was **deleted** instead of shipped. The replacement (a zoomed
  flange view) could not be captured: Mutter refused every ScreenCast session with
  `Session creation inhibited` for the whole attempt, and this host has no per-window
  screenshot API. The README therefore shows five screenshots, with 2D covered by
  `01-desktop-hero.png`.
- **03 zoom evidence.** The `cad script` read-back is taken 2 s after the script starts,
  while the load-wait poll is still running, so it logs `{"zoom":[]}`; the issued ZOOMIN
  calls are only evidenced by the framing in the image itself.
- **07 is un-zoomed** (`全景` as requested): the decoded massing is honest but small in the
  viewport, and the preview geometry is white because the RVT materials are not mapped.
- **Per-window capture is unavailable** on this host (see above), so each frame is a
  cropped full-screen capture. A desktop window covering yacr would have corrupted the
  crop; frames were screened for the app being frontmost and polluted bursts were
  discarded and retaken.
- **Not captured:** `complex-test.dwg` (explicitly out of scope), `04-ifc-3d`/`05-dgn`
  (dropped by request), and Android/Web (reusing existing assets).
- These files are **brand-new** screenshots; the older
  `apps/landing/assets/screenshots/desktop-flange-2026-11-08.png` and
  `docs/evidence/dgn-linux-gpu/dgn-v8-test.png` were **not** reused, because both show a
  Chinese UI and the new set must be English.
