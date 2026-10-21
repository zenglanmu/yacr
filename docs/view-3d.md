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
  `apps/app-linux/ui/src/commands.mjs`'s `GESTURE_ONLY_COMMANDS`, so the
  affordance stays disabled with a gesture reason rather than fabricating a
  camera move.
- **No ribbon 3D toggle and no standard-view controls**: the React chrome has
  neither, so `STANDARDVIEW` (which needs a view payload) and `PROJECTION` are
  reachable only through channels that do not exist yet. This is an explicit UI
  gap, not a hidden one.

## What is not verified here

- **GPU execution has been observed** on a real adapter (Quadro P620, Vulkan)
  through the React/Tauri host: the 2D path renders the drawing and the 3D path
  renders a real frame. Pixel-level correctness of the **3D image** is a separate
  question and is tracked in `docs/handoff.md`; 2D↔3D round-trip and the camera
  math are unit-tested (`cargo test -p cad-app`).
- **Platform placeholders**: `apps/app-web` and `apps/app-android` are explicit
  NOT IMPLEMENTED stubs, so 3D and paper-space runtime reachability there is not
  claimed.
- **3D picking/depth test** still lives with the render workstream
  (`Camera::screen_to_ray` exists, but no ray/scene intersection or depth
  readback and no `PickHit` selection).
- **Perspective fitting on entry**: entering 3D now frames the document
  (`Camera::enter_3d`: target at the 3D bounds centre, distance
  `radius / tan(fov/2) * 1.2` with `radius` = half the bounding-box diagonal,
  view direction preserved). `FIT` itself still returns to the 2D plan; a
  re-fit while already in 3D is not implemented.

These are explicit gaps, not empty successes: a refused space or a degenerate
3D camera produces a diagnostic and no frame, never a blank "success".
