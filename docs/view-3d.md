# 3D view: camera, projection and work plane (`cad-app`)

Spec v2.0 §3.3 / audit **F13**. This note records what the *application* side of
3D observation now does — camera state, projection, standard views, orbit,
zoom-to-cursor and the work plane consumed by planar measurement — and what is
still handled by the render/picking workstream.

Scope: `crates/cad-app/src/camera.rs` and the camera commands in
`crates/cad-app/src/lib.rs`. No GPU object, depth buffer, framebuffer or scene
graph is touched here (constraint: business layers do not submit GPU work).

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

## What remains (render workstream)

Out of scope here and **not** claimed as done:

- **GPU rendering**: there is still only a 2D `Camera2d` on the render side; the
  projection matrices above are not yet uploaded, and there is no depth buffer,
  MSAA, sRGB or transparent-pass handling for 3D.
- **3D picking/depth test**: `Camera::screen_to_ray` produces a ray, but no ray
  scene intersection, depth readback or `PickHit` selection exists in the app;
  that lives with the renderer (F14).
- **Perspective viewport fitting**: `fit_viewport` returns to the 2D plan; a
  perspective-specific fit (framing a 3D bounds) is not implemented.
- **Host UI wiring**: this is camera *state* only. The Slint/Android/Web shells
  do not yet expose 3D buttons or feed `Orbit`/`ZoomAt` from gestures (F02/U01).

These are explicit gaps, not empty successes: no command returns a fake
completion for them.
