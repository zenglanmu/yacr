# 3D view: camera, projection and work plane (`cad-app`)

Spec v2.0 §3.3 / audit **F13**. This note records what the *application* side of
3D observation now does — camera state, projection, standard views, orbit,
zoom-to-cursor and the work plane consumed by planar measurement — and what is
still handled by the render/picking workstream.

Scope: `crates/cad-app/src/camera/` (types, view math, tests), the camera/space
commands in `crates/cad-app/src/application.rs`, and the host wire-up
`crates/cad-host/src/state.rs` (the `ViewSnapshot` built from the viewport's
`camera` + `view_mode`) feeding `crates/cad-engine/src/render/runtime/session.rs`,
which dispatches `Renderer::render(Camera2d, ..)` or `Renderer::render_3d(Camera3d, ..)`.
No GPU object, depth buffer, framebuffer or scene graph is touched by `cad-app`
(constraint: business layers do not submit GPU work); `cad-engine` only maps the
app-derived eye/target/up/fov/near/far onto `cad-render-wgpu::Camera3d`.

## Conventions

All conventions are fixed by `camera.rs` and asserted by its unit tests.

- **Handedness**: right-handed world coordinates. `+X` right, `+Y` forward
  (plan "up"), `+Z` up. The default 2D view looks down `-Z` with screen
  `+x = +X` and screen `+up = +Y`.
- **View basis**: `right`, `up`, `forward` are unit vectors with `forward`
  pointing from the eye toward the target. `(right, up, -forward)` is
  right-handed: `right × up = -forward`. `ViewBasis::is_orthonormal` checks this.
- **Units**: world coordinates are in drawing units; screen coordinates are
  logical pixels with `y` growing **downwards** (pointer/Slint convention), so
  `world_to_screen` flips the sign.
- **Angles**: radians unless a name says otherwise. A perspective field of view
  is the full **vertical** angle.
- **Depth range**: projection matrices map clip `z` to `[0, 1]` (WebGPU), not
  `[-1, 1]`.
- **2D plan mapping** (`Camera::screen_to_plan_world`) is only defined for the
  orthographic top view; it is the legacy mapping measurement picks use.

## Projection

`Projection` is either `Orthographic { scale }` (world units per logical pixel)
or `Perspective { vertical_fov_radians }`. Both are validated:

| Input | Result |
|---|---|
| `scale < MIN_ORTHO_SCALE` (1e-9), `NaN`, `±inf` | `InvalidInput` |
| `vertical_fov < 1e-3`, `> π − 1e-3`, `NaN`, `±inf` | `InvalidInput` |
| valid values | accepted, round-tripped by `Projection::orthographic`/`perspective` |

`Camera::projection_matrix(size)` returns a column-major `[[f64; 4]; 4]`
(view→clip). Orthographic half-extents are `pixels * scale / 2`; perspective
uses the full vertical FOV and a near/far derived from the camera distance, so
the target always stays inside the frustum. A degenerate viewport size or
projection is an error, never a `NaN` matrix.

## 2D ↔ 3D switch, orbit, standard views

`CommandId::Switch2d3d` toggles between the 2D plan view and a 3D perspective
view. The transition is **lossless**:

- entering 3D snapshots the exact 2D `Camera` and promotes the projection to
  `DEFAULT_PERSPECTIVE_FOV_RADIANS` (45°);
- leaving 3D restores the snapshotted camera bit-for-bit
  (`Viewport::set_view_mode`), which the `switch_2d3d_and_orbit_are_real_transitions`
  test asserts with a full `Camera` equality.

`CommandId::Orbit` takes `CommandPayload::Orbit { yaw, pitch }` (radians):

- yaw rotates about world `+Z`; pitch rotates about the current horizontal
  (right) axis;
- the eye-target distance is preserved;
- the polar angle is clamped to `[1e-3, π − 1e-3]`, so the view never collapses
  onto a pole and the basis stays orthonormal (the **near-plane guard**);
- non-finite angles are rejected, and orbiting outside 3D mode is an error so an
  accidental drag never disturbs the exact 2D view.

`CommandId::StandardView` with `CommandPayload::StandardView(view)` places the
camera at `target + offset * distance`, using the standard view's unit eye
offset and the current camera distance. `StandardView::{Top, Bottom, Front,
Back, Left, Right, Isometric}` all produce a right-handed orthonormal basis
(tested for every variant). The **Top** view is the canonical plan: it forces an
orthographic projection and returns the viewport to 2D mode; every other
standard view is a 3D perspective view.

`CommandId::SwitchProjection` explicitly toggles orthographic↔perspective while
keeping the target, and keeps `view_mode` consistent (perspective ⇒ 3D,
orthographic ⇒ restored 2D).

## Zoom-to-cursor

`CommandPayload::ZoomAt { factor, cursor }` zooms in both projections:

- **orthographic**: multiplies the world-per-pixel scale and shifts the
  eye+target so the world point under the cursor is unchanged (asserted to
  1e-9);
- **perspective**: dolls the eye and target together along the cursor ray, which
  preserves every pixel ray line, so the world point under the cursor stays put;
- `factor ≤ 0` or non-finite is rejected.

The legacy `CommandPayload::Points` factor-only zoom is retained for CLI/direct
callers and uses the canvas centre as the cursor.

## ZOOM WINDOW / ZOOM PREVIOUS (2026-10-28)

Both are **typed argument forms of `ZOOM`** — no new `CommandId`, no second zoom path.

* **`ZOOM WINDOW`** frames a two-corner rectangle **exactly**. The host arms its
  box gesture (`begin_zoom_window`), the next 2D primary drag supplies the two
  corners in canvas logical pixels, and the release dispatches one
  `CommandPayload::ZoomWindow`. It is **2D orthographic only**: a 3D view is refused
  by name (`zoom.window.requires_2d`) at arming, and a capture or modal pan that
  already owns the pointer refuses with `zoom.window.capture_busy`. A degenerate or
  non-finite rectangle dispatches **nothing**. The fit reuses `framing_rect`'s
  tighter-axis rule with margin `WINDOW_MARGIN = 1.0` (FIT uses `1.05`; the padding
  is **our choice**, AutoCAD's is undocumented), leaves `view_mode` in 2D, writes no
  document history and does not bump `session.generation`.
* **`ZOOM PREVIOUS`** restores this viewport's previous **discrete** view from a
  per-viewport stack capped at `ZOOM_HISTORY_CAP = 16` (oldest dropped). A snapshot is
  `camera + view_mode + work_plane`; it is pushed **before** each discrete change
  (FIT/`ZOOM ALL`/`EXTENTS` when it really frames, `ZOOM <factor>`/`ZOOMIN`/`ZOOMOUT`/
  legacy `Points`, `ZOOM WINDOW`, `STANDARDVIEW` except the no-op plan view while
  already 2D, `SWITCH2D3D`, `PROJECTION`, `RESETVIEW`). **Pans, wheel zooms and orbit
  drags never record a step** — a declared divergence from AutoCAD, whose history is
  interactive. An empty stack is not an error: it reports the `zoom.previous.empty`
  diagnostic and changes nothing.

## Work plane

`camera::orthonormal_work_plane(origin, u, v)` (and `xy_work_plane(z)`) build the
right-handed, orthonormal `WorkPlane` that `cad-measure` consumes. The outward
normal `normalize(u × v)` points toward the plan viewer (`+Z`). The plane is
re-orthogonalised so it always satisfies the measure engine's checks
(non-degenerate, orthogonal, unit-scaled); a parallel/degenerate basis is an
error. `Viewport::new` and `ResetView` install the `z = 0` XY plane, so area and
polyline measurements keep a valid explicit plane after any view transition.

## Host wiring and reachability (F02/F04/F13/F14)

The 3D camera is wired end to end in the **React/Tauri** host, but the chrome
exposes **no 3D control**:

- `crates/cad-host/src/state.rs` builds `ViewSnapshot` from
  `viewport.camera` and `viewport.view_mode.kind()` — the session view mode is
  authoritative, so a 3D view with an orthographic camera stays `ThreeD`. In a 3D
  view the pointer-cursor overlay input is explicitly empty (no planar cursor).
- `crates/cad-engine/src/render/runtime/session.rs` branches on that mode:
  `TwoD` → `camera2d_from_params` + `render`; `ThreeD` → `camera3d_from_params`
  + `render_3d`. `camera3d_from_params` (`crates/cad-engine/src/render/camera.rs`)
  maps `Camera3dParams` field-for-field, and `Camera::camera3d_params` derives
  near/far with the same rule as `Camera::projection_matrix`, refusing an
  orthographic, coincident or pole-locked camera.
- **Reaching 3D today**: `SWITCH2D3D` is a payload-free selector in the
  `cad-ui-core` command table, so typing it in the command line reaches the real
  session (`cad_command`); entering 3D frames the model (`Camera::enter_3d`,
  distance from the document's 3D bounds, see below) and returning to 2D restores
  the snapshotted camera bit-for-bit.
- **Orbiting**: in a 3D view a primary-button canvas drag emits
  `CommandId::Orbit` (`crates/cad-host/src/input/pointer.rs`); it is deliberately
  *not* a button payload — `ORBIT` is listed in
  `apps/desktop-ui/src/commands/commands.ts`'s `GESTURE_ONLY_COMMANDS`, so the
  affordance stays disabled with a gesture reason (`view.orbit.gesture`, returned
  by `unavailableReasonKey`) rather than fabricating a camera move.
- **Standard views**: `STANDARDVIEW` carries a view payload, and the project
  rule is a dedicated **typed IPC** rather than text parsing on `cad_command`, so
  the host exposes `cad_standard_view { view: "top" | "bottom" | "front" | "back"
  | "left" | "right" | "isometric" }`. It is queued (bounded, same gates as
  `admit_command`), drained on the owner tick, and dispatched as the real
  `CommandId::StandardView` + `CommandPayload::StandardView`. Names parse
  case-insensitively; an unknown or empty view is refused by name.
- **`PROJECTION` needs no payload** and was already reachable through the
  command line (the application arm ignores the payload), so it needed no
  channel.
- **Ribbon (added 2026-10-21)**: the `view` tab now leads with a **3D panel** —
  a large `2D/3D` toggle (`SWITCH2D3D`), a `Projection` toggle (`PROJECTION`), and
  an **Orbit** item that is visible but **disabled**, because orbiting is a canvas
  drag and a button would have to fabricate a payload — and a **Standard view**
  panel with the seven views, each dispatching the typed IPC above. Labels come
  from the existing catalog keys (`view.*`, `ribbon.command.*`); no new strings
  were needed. The toggles do **not** claim a state readout, because `cad_status`
  carries no 3D/projection state.

## Shading convention

- **2D**: a world-fixed headlight along `-Z`, i.e. `diffuse = n.z`; unchanged
  byte-for-byte by the 3D work.
- **3D**: a **camera-relative key light offset from the view axis**:
  `light = normalize(toward_eye + KEY_LIGHT_RIGHT * right + KEY_LIGHT_UP * up)`
  with `toward_eye = normalize(eye - target)` and `right`/`up` the camera's own
  basis vectors; the constants are `KEY_LIGHT_RIGHT = -0.10` (screen left) and
  `KEY_LIGHT_UP = 0.35` (above), i.e. about **20° above-left** of the sight line
  (`crates/cad-render-wgpu/src/geometry/camera.rs`).
  A pure headlight (`light = toward_eye`) was tried first and **collapsed on
  isometric views**: the three visible faces are symmetric about that axis, so
  all three got the same shade (measured: three identical `215` fills). The
  offset keeps symmetric views readable (measured: `204 / 189 / 238`).
  `lit = tint * (0.25 + 0.75 * max(dot(n, light), 0))` in both cases — ambient
  0.25, no specular, no shadows, no environment light. This is a **display
  convention, not physically based shading**; albedo, exposure and material
  colour are not modelled, and two faces can still end up close in value when
  their orientations relative to the light are similar.

## What is not verified here

- **GPU execution has been observed** on a real adapter (Quadro P620, Vulkan)
  through the React/Tauri host: the 2D path renders the drawing and the 3D path
  renders a real frame. Pixel-level correctness of the **3D image** is a separate
  question and is tracked in `docs/handoff.md`; 2D↔3D round-trip and the camera
  math are unit-tested (`cargo test -p cad-app`).
- **Platform placeholders**: `apps/app-web` and `apps/app-android` are explicit
  NOT IMPLEMENTED stubs, so 3D and paper-space runtime reachability there is not
  claimed.
- **3D picking is wired and measured** (corrected 2026-10-21; the previous text
  said no ray/scene intersection existed, which was wrong).
  `cad_app::picking::pick_at_screen_with_visibility` is the single funnel for
  both hover (`crates/cad-host/src/input/hover.rs`) and click selection
  (`crates/cad-host/src/input/interaction.rs`), it calls `Camera::screen_to_ray`
  → `pick_ray`, and `pick_tolerance` derives a **perspective** world-per-pixel
  from depth and FOV, so a 3D view picks along its true view ray.
  Measured on a real 3D frame: a synthetic click on an ACIS solid selects it
  (`AcDb3dSolid`, 24 vertices / 12 triangles), a click on empty space returns
  **0** (so the hit is fresh, not a surviving selection), and a 2D control click
  selects the same entity.
  Limits: `cad_status` exposes no 3D camera and no hovered-entity field, so a 3D
  hover is observable only as `hoverPreview`/`snapKind`; picking targets the CPU
  ray funnel, which is what the host uses — the GPU ID pass
  (`docs/picking-gpu-id-pass.md`) is a separate path.
- **Perspective fitting on entry**: entering 3D now frames the document
  (`Camera::enter_3d`: target at the 3D bounds centre, distance
  `radius / tan(fov/2) * 1.2` with `radius` = half the bounding-box diagonal,
  view direction preserved). `FIT` itself still returns to the 2D plan; a
  re-fit while already in 3D is not implemented.

These are explicit gaps, not empty successes: a refused space or a degenerate
3D camera produces a diagnostic and no frame, never a blank "success".
