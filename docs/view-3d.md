# 3D view: camera, projection and work plane (`cad-app`)

Spec v2.0 §3.3 / audit **F13**. This note records what the *application* side of
3D observation now does — camera state, projection, standard views, orbit,
zoom-to-cursor and the work plane consumed by planar measurement — and what is
still handled by the render/picking workstream.

Scope: `crates/cad-app/src/camera.rs`, the camera/space commands in
`crates/cad-app/src/lib.rs`, and the host wire-up in
`crates/cad-ui-slint/src/{bridge.rs,lib.rs}` + `ui/app.slint`. No GPU object,
depth buffer, framebuffer or scene graph is touched by `cad-app` (constraint:
business layers do not submit GPU work); the bridge only selects and passes
cameras to `cad-render-wgpu`.

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

## Host UI wiring (F02/F04/F13/F14)

The Slint host is now wired to the app-level camera and space state:

- `cad-ui-slint/src/bridge.rs` keeps, per frame, the active space
  (`SpaceSelection::Model | Paper(LayoutId)`), the observation mode (2D/3D), the
  full application `Camera` and the 2D centre/scale mirror. `BeforeRendering`
  builds the drawing with `build_scene_with_annotations_in_space` (the model or
  paper path plus the annotation overlay in one delta) and dispatches to
  `Renderer::render(Camera2d, ..)` or `Renderer::render_3d(Camera3d, ..)`.
- The conversion `cad_app::camera::{Camera, Projection} → cad_render_wgpu::Camera3d`
  lives only in the bridge (`camera3d_from_params`), mapping the app-derived
  eye/target/up/fov/near/far field-for-field. `Camera::camera3d_params` derives
  the near/far planes with the same rule as `Camera::projection_matrix`, and
  refuses an orthographic, coincident or pole-locked camera.
- `CadView` exposes `set_space`, `set_view_mode`, `set_camera3d` and
  `sync_from_viewport(&Viewport)`. `set_space` validates the layout against the
  real layout table (`cad_app::validate_space`) and records an explicit
  diagnostic on refusal; `BeforeRendering` re-validates every frame.
- The shell (`ui/app.slint` + `UiAdapter`) has a 2D/3D toggle, an explicit
  projection toggle and standard-view controls generated from `StandardView::ALL`,
  routed through `CommandId::{Switch2d3d, SwitchProjection, StandardView}`. The
  layout selector routes through `CommandId::SwitchSpace` (validated in
  `cad-app`). A left-button drag while the pushed view state is 3D emits
  `CommandId::Orbit` from the canvas callback.

## What is not verified here

- **GPU execution was not observed in this environment.** The native
  `cad-ui-slint` build cannot be linked here (no `pkg-config`/fontconfig), so the
  bridge was compiled only for `wasm32-unknown-unknown` and
  `aarch64-linux-android`. No window was opened and no frame was rendered: the
  `render` / `render_3d` dispatch, the depth buffer and perspective correctness
  on a real adapter remain unverified. `cad-app`'s camera/space logic is the
  part that is unit-tested on the host (`cargo test -p cad-app`).
- **Host glue is not in this workstream.** `apps/app-web` and `apps/app-android`
  still call `CadView::set_camera` only; the orchestrator must switch them to
  `CadView::sync_from_viewport` (and handle `SwitchSpace`) for 3D and paper
  space to be reachable at runtime. `LayoutSwitchSink` remains as a
  source-compatible override; when installed it takes precedence over the
  `SwitchSpace` command so the two paths cannot fight.
- **3D picking/depth test** still lives with the render workstream
  (`Camera::screen_to_ray` exists, but no ray/scene intersection or depth
  readback and no `PickHit` selection).
- **Perspective viewport fitting** is still not implemented: `fit_viewport`
  returns to the 2D plan, and there is no bounds-framing 3D fit.

These are explicit gaps, not empty successes: a refused space or a degenerate
3D camera produces a diagnostic and no frame, never a blank "success".
