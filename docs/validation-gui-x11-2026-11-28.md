# Linux X11/XWayland (dom-canvas) evidence attempt (2026-11-28)

This run exercised the **X11/XWayland `dom-canvas`** carrier in the same real
GNOME/Mutter session. It ended in the carrier's **named refusal**, not a drawn
frame: WebKitGTK could not create a usable WebGL2 context, so the app refused
before opening a CAD canvas. No screenshot exists because there was no carrier
frame to capture; no blank-canvas frame was fabricated.

## 1. Environment and command

- Same host as the Wayland run: GNOME/Mutter, XWayland on `DISPLAY=:0`,
  `GDK_BACKEND=x11`, eDP-1 1920x1080, Quadro P620.
- The session was unlocked for the attempt (ScreenCast is inhibited while
  locked) and re-locked afterwards with `loginctl lock-session 1`.
- Command (fresh output directory, **no build**: the current release binary was
  built by the immediately preceding Wayland run):

```bash
bash scripts/collect-evidence.sh \
  --backend x11 --no-build \
  --out /tmp/opencode/yacr-evidence-20261128-x11 \
  --capture-at 10 --timeout 80
```

Result row (`docs/evidence/linux-x11-2026-11-28/results.tsv`):

```
x11  production  UNSUPPORTED_RUNTIME  6  missing  -  marker='yacr: unsupported-runtime: webgl2' (expected exit 6); dom-canvas WebGL2 carrier cannot draw; refused before any capture; no carrier frame exists
```

## 2. Exact refusal

From `docs/evidence/linux-x11-2026-11-28/run-excerpt.log`:

```
[probe] display backend policy=X11 resolved=X11 source=from environment
[probe] surfaceMode=dom-canvas (wasm CAD session in the WebView owns the pixels; no native CAD surface)
[probe] startup drawing: .../fixtures/dwg/synthetic-four-lines.dwg
yacr: unsupported-runtime: webgl2
yacr 无法在此会话中创建 CAD 画布（WebGL2）
...
[probe] canvas runtime failed step=ui-module-load detail=Error: script
/canvas/bootstrap.js failed: error event [origin=tauri://localhost ... fetch=200 text/html]
```

The refusal is marker-first (`yacr: unsupported-runtime: webgl2`, exit 6), and it
happens before any canvas is shown. The X11 policy itself resolved correctly to
`dom-canvas`; the blocker is the WebView's WebGL2/GL support in this session.

## 3. What this proves and does not prove

- Proves: the X11/XWayland launch path is taken and reaches `dom-canvas`; a
  missing WebGL2 context is refused by name and exit code 6, not silently
  downgraded or shown as an empty canvas.
- Does **not** prove: the `dom-canvas` renderer works, pixel output, GPU usage,
  input, or any SketchUp UI interaction on X11. X11/`dom-canvas` therefore
  remains **NOT VERIFIED** in this environment; no screenshot is claimed.
- The refusal is not a software-rendering result and was not forced; no virtual/
  headless display or fake marker mode was used.
- One fixture, one session. Windows/macOS/Android/Web and pixel accuracy remain
  NOT VERIFIED.

## 4. Reproduce

Run the command in §1 in an unlocked GNOME session where the WebView has WebGL2.
If WebGL2 is unavailable, the same stable marker/exit 6 is expected; the runner
records `screenshot=missing` and the exact marker rather than a fabricated frame.
