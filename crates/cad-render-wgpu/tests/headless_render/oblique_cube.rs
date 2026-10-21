//! An **oblique** 3D view of a closed solid must show a cube, not one square
//! face - and its faces must be *distinguishable* from each other.
//!
//! Regression 1 (measured on the live host): a tessellated `[-1, 1]³` ACIS cube
//! was switched to the 3D view and orbited with a primary drag, and the CAD grab
//! came back as a **rotated square**: uniform white, no internal edges, no
//! perspective. The camera defect was in `cad_app::Camera::orbit` (an
//! incremental drag cancelled its own pitch, leaving the eye a few milliradians
//! off `+Z`, so the `+Z` face filled the frame). These tests fence the *renderer*
//! half: with an explicitly oblique `Camera3d` the 12-triangle cube must project
//! to a six-vertex silhouette, keep its mesh edge overlay, and show measurable
//! perspective convergence. The top-down control below is the non-vacuous
//! reference: the same measurements report a four-cornered square there.
//!
//! Regression 2 (measured on a real oblique frame): the mesh shader shaded with
//! a **world-fixed** light, so `diffuse == n.z` and every non-horizontal face
//! collapsed to the same ambient-only value (`sRGB(0.25) == 137`): a 3-face cube
//! read as one bright quad plus two identical slivers. The shading is now a
//! camera-relative headlight in 3D, and the tests below require three distinct
//! flat face shades and require the *assignment* to follow a second azimuth, so
//! a constant light (or a Rust-side per-face tint) cannot pass.

use super::*;

use solid_fill::acis_cube;

use cad_render_wgpu::headless::RgbaImage;
use cad_render_wgpu::RenderTier;

/// Side of the rendered target, in pixels.
const SIZE: u32 = 256;

/// An **asymmetric** oblique camera: azimuth `27°` from `-Y` toward `+X`,
/// elevation `32°` above the horizon, so the three visible faces (`+Z`, `-Y`,
/// `+X`) receive three *different* amounts of light. A symmetric isometric view
/// (`45°`/`35.26°`) would give the two side faces equal light and could not
/// distinguish a camera-relative headlight from a world-fixed one.
fn oblique_camera() -> Camera3d {
    oblique_camera_at(27.0, 32.0)
}

/// The oblique camera at an explicit azimuth/elevation, at a fixed distance.
///
/// Azimuth is measured from the `-Y` axis toward `+X`; elevation raises the eye
/// toward `+Z`. World `+Z` stays the screen up vector.
fn oblique_camera_at(azimuth_degrees: f64, elevation_degrees: f64) -> Camera3d {
    let azimuth = azimuth_degrees.to_radians();
    let elevation = elevation_degrees.to_radians();
    let distance = 5.0;
    Camera3d {
        eye: Point3 {
            x: distance * azimuth.sin() * elevation.cos(),
            y: -distance * azimuth.cos() * elevation.cos(),
            z: distance * elevation.sin(),
        },
        target: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        fov_y: std::f64::consts::FRAC_PI_4,
        near: 0.1,
        far: 100.0,
    }
}

/// Sample point strictly inside the `-Y` face, away from its triangulation
/// diagonal (`z == x`) and from the cube edges, so the pixel is flat fill.
const FRONT_FACE_SAMPLE: [f64; 3] = [0.5, -1.0, -0.5];
/// Sample point strictly inside the `+X` face (same rule as above).
const SIDE_FACE_SAMPLE: [f64; 3] = [1.0, 0.5, -0.5];
/// Sample point strictly inside the `+Z` face (same rule as above).
const TOP_FACE_SAMPLE: [f64; 3] = [0.55, 0.05, 1.0];

/// The control: the same distance and field of view looking straight down `-Z`
/// (the plan orientation). This must still be a square - if it were not, the
/// "hexagon" measurement below would prove nothing.
fn top_down_camera() -> Camera3d {
    Camera3d {
        eye: Point3 {
            x: 0.0,
            y: 0.0,
            z: 5.0,
        },
        target: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        fov_y: std::f64::consts::FRAC_PI_4,
        near: 0.1,
        far: 100.0,
    }
}

fn lum(image: &RgbaImage, x: u32, y: u32) -> u8 {
    image.pixel(x, y)[0]
}

/// The pixel `point` (world space) projects to, using the *same* view-projection
/// the renderer used. wgpu's viewport maps clip `+Y` to the top of the image.
fn project(camera: Camera3d, point: [f64; 3]) -> (u32, u32) {
    let vp = camera
        .view_projection(1.0)
        .expect("the test camera must have a usable projection");
    let mut clip = [0.0f64; 4];
    for (row, value) in clip.iter_mut().enumerate() {
        *value = (0..4)
            .map(|col| f64::from(vp[col][row]) * if col == 3 { 1.0 } else { point[col] })
            .sum();
    }
    let ndc_x = clip[0] / clip[3];
    let ndc_y = clip[1] / clip[3];
    let x = ((ndc_x + 1.0) * 0.5 * f64::from(SIZE)).round() as i64;
    let y = ((1.0 - ndc_y) * 0.5 * f64::from(SIZE)).round() as i64;
    (
        x.clamp(0, i64::from(SIZE) - 1) as u32,
        y.clamp(0, i64::from(SIZE) - 1) as u32,
    )
}

/// The flat shade of the face containing `sample`, read from the rendered frame.
fn face_shade(image: &RgbaImage, camera: Camera3d, sample: [f64; 3]) -> u8 {
    let (x, y) = project(camera, sample);
    lum(image, x, y)
}

fn is_shape(image: &RgbaImage, x: u32, y: u32) -> bool {
    lum(image, x, y) > 8
}

/// Every pixel of the shape (anything above the black clear colour).
fn shape_points(image: &RgbaImage) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    for y in 0..image.height {
        for x in 0..image.width {
            if is_shape(image, x, y) {
                out.push([f64::from(x), f64::from(y)]);
            }
        }
    }
    out
}

/// Monotone-chain convex hull (strictly convex: collinear points are dropped).
fn hull(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p: Vec<[f64; 2]> = points.to_vec();
    p.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap()
            .then(a[1].partial_cmp(&b[1]).unwrap())
    });
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Perpendicular distance from `p` to the line through `a` and `c`.
fn line_distance(p: [f64; 2], a: [f64; 2], c: [f64; 2]) -> f64 {
    let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
}

/// The polygon's corners: hull vertices that are at least `tol_px` away from
/// the chord through their neighbours.
///
/// Pixel-quantised, anti-aliased edges contribute hundreds of near-collinear
/// boundary points, so counting *strict* hull vertices measures the rasteriser,
/// not the silhouette. `tol_px` is therefore a real geometric tolerance (well
/// below the ~20 px corner cuts an oblique cube produces and well above the
/// sub-pixel staircase of a straight edge).
fn corner_polygon(points: &[[f64; 2]], tol_px: f64) -> Vec<[f64; 2]> {
    let mut h = hull(points);
    if h.len() < 3 {
        return h;
    }
    // Remove one vertex at a time (the flattest), recomputing the neighbours
    // each time. A parallel pass is order-dependent and wrong: an anti-aliased
    // corner is often flanked by two near-duplicate pixels of the *same* edge,
    // so all three look collinear and the real corner is dropped with them.
    loop {
        let n = h.len();
        if n <= 3 {
            return h;
        }
        let mut flattest = (f64::INFINITY, 0usize);
        for i in 0..n {
            let distance = line_distance(h[i], h[(i + n - 1) % n], h[(i + 1) % n]);
            if distance < flattest.0 {
                flattest = (distance, i);
            }
        }
        if flattest.0 >= tol_px {
            return h;
        }
        h.remove(flattest.1);
    }
}

/// Interior angles of a polygon, in degrees.
fn interior_angles(poly: &[[f64; 2]]) -> Vec<f64> {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let prev = poly[(i + n - 1) % n];
            let here = poly[i];
            let next = poly[(i + 1) % n];
            let a = [prev[0] - here[0], prev[1] - here[1]];
            let b = [next[0] - here[0], next[1] - here[1]];
            let dot = a[0] * b[0] + a[1] * b[1];
            let cross = a[0] * b[1] - a[1] * b[0];
            cross.atan2(dot).to_degrees().abs()
        })
        .collect()
}

/// Bounding box of the shape as `(min_x, min_y, max_x, max_y)`.
fn bbox(points: &[[f64; 2]]) -> (f64, f64, f64, f64) {
    points.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), p| (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])),
    )
}

/// Width (in pixels) of the shape on the scanline at `fraction` of its height.
fn width_at(image: &RgbaImage, (y0, y1): (f64, f64), fraction: f64) -> usize {
    let y = (y0 + (y1 - y0) * fraction).round() as u32;
    (0..image.width).filter(|&x| is_shape(image, x, y)).count()
}

/// Count of interior pixels that are **not** one of the flat face shades, i.e.
/// the anti-aliased mesh edge overlay drawn *inside* the silhouette.
///
/// The shape mask is eroded by `radius` so the silhouette's own anti-aliased
/// border cannot be counted. The flat faces render at constant, known values
/// (one per face, sampled from the frame itself), so an interior pixel whose
/// value is not one of those is either an edge or a blend between an edge and a
/// face. Bands cannot be used: with a camera-relative light the face shades
/// themselves sit in the mid-tones.
fn interior_edge_pixels(image: &RgbaImage, radius: u32, face_shades: &[u8]) -> usize {
    let mut count = 0;
    for y in radius..image.height.saturating_sub(radius) {
        for x in radius..image.width.saturating_sub(radius) {
            let all_in = (y - radius..=y + radius)
                .all(|ny| (x - radius..=x + radius).all(|nx| is_shape(image, nx, ny)));
            if !all_in {
                continue;
            }
            let v = lum(image, x, y);
            if !face_shades.contains(&v) {
                count += 1;
            }
        }
    }
    count
}

/// Render the cube with `camera` on `tier` and read the frame back.
fn render_cube(camera: Camera3d, tier: RenderTier) -> Option<RgbaImage> {
    let gpu = gpu()?;
    if tier == RenderTier::Accelerated {
        let limits = gpu.device.limits();
        if limits.max_storage_buffers_per_shader_stage < 3
            || limits.max_storage_buffer_binding_size < 64 * 1024
        {
            eprintln!(
                "skipping accelerated-tier oblique cube: adapter has no vertex-stage storage"
            );
            return None;
        }
    }
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(tier == RenderTier::Accelerated));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    assert_eq!(
        renderer.render_tier(),
        tier,
        "the requested render tier must be the one under test"
    );
    renderer
        .upload(&scene(vec![acis_cube(true)]))
        .expect("upload the cube mesh");
    let stats = renderer
        .render_3d(camera, &RenderTarget::new(SIZE, SIZE))
        .expect("render the cube");
    assert!(
        stats.triangles >= 12,
        "{tier:?}: the 3D frame must submit the 12 fill triangles plus its overlay, got {}",
        stats.triangles
    );
    Some(renderer.read_target_rgba().expect("read the frame"))
}

/// An oblique cube is a **hexagon** with its internal edges and visible
/// perspective - never a square face.
#[test]
fn oblique_cube_renders_as_a_hexagon_with_internal_edges() {
    for tier in [RenderTier::Universal, RenderTier::Accelerated] {
        let Some(image) = render_cube(oblique_camera(), tier) else {
            return;
        };
        let points = shape_points(&image);
        let (x0, y0, x1, y1) = bbox(&points);
        let poly = corner_polygon(&points, 2.0);
        let angles = interior_angles(&poly);
        // The three visible faces, sampled at points strictly inside each one.
        let camera = oblique_camera();
        let front = face_shade(&image, camera, FRONT_FACE_SAMPLE);
        let side = face_shade(&image, camera, SIDE_FACE_SAMPLE);
        let top = face_shade(&image, camera, TOP_FACE_SAMPLE);
        let shades = [front, side, top];
        let edge_pixels = interior_edge_pixels(&image, 2, &shades);
        let upper_width = width_at(&image, (y0, y1), 0.25);
        let lower_width = width_at(&image, (y0, y1), 0.75);
        println!(
            "{tier:?}: shape px={} bbox={}x{} corners={} angles={:?} edge_px={} \
             width_upper={} width_lower={} shades(-Y,+X,+Z)={shades:?}",
            points.len(),
            x1 - x0,
            y1 - y0,
            poly.len(),
            angles
                .iter()
                .map(|a| (a * 10.0).round() / 10.0)
                .collect::<Vec<_>>(),
            edge_pixels,
            upper_width,
            lower_width
        );

        // A tilted cube shows more than one face and a 6-vertex silhouette. A
        // square (the measured defect: one face seen along its normal) has 4.
        assert_eq!(
            poly.len(),
            6,
            "{tier:?}: an oblique cube must project to a 6-vertex hexagon, got {} corners at {poly:?}",
            poly.len()
        );
        // Not a rectangle: the corner cuts of the hexagon are not right angles.
        assert!(
            angles.iter().any(|a| (a - 90.0).abs() > 10.0),
            "{tier:?}: a square would have four ~90 deg corners, got {angles:?}"
        );
        assert!(
            angles.iter().all(|a| (30.0..=170.0).contains(a)),
            "{tier:?}: degenerate silhouette corner angles {angles:?}"
        );

        // All three visible faces must be **distinct**: a solid has to be
        // readable, and one bright face plus two ambient-only slivers (the
        // measured world-fixed-light defect: `255` and `137`, `137`) is not.
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            assert!(
                shades[i].abs_diff(shades[j]) >= 2,
                "{tier:?}: faces {i} and {j} are not distinguishable \
                 (-Y/+X/+Z shades {shades:?}); this is the world-fixed-light \
                 collapse the camera-relative headlight removes"
            );
        }
        // With the light at `normalize(eye - target)` and azimuth 27 deg /
        // elevation 32 deg, the `-Y` face (dot ~0.76) is brighter than `+Z`
        // (~0.53), which is brighter than `+X` (~0.39). This pins the actual
        // orientation, not merely "some three values".
        assert!(
            front > top && top > side,
            "{tier:?}: expected -Y > +Z > +X, got -Y={front} +Z={top} +X={side}"
        );

        // The mesh edge overlay must draw the 12 triangles' edges in 3D too.
        // The `+Z` face is split corner to corner, so at least that diagonal is
        // strictly interior.
        assert!(
            edge_pixels > 100,
            "{tier:?}: the 3D frame has no internal mesh-edge pixels ({edge_pixels})"
        );

        // Perspective convergence: the eye is above the cube, so the `+Z` face
        // (which provides the upper part of the silhouette) is 1.2 world units
        // nearer than the `-Z` face (the lower part) and must project wider. A
        // parallel projection would make the two scanlines equal - the top-down
        // control below shows exactly that.
        assert!(
            (upper_width as f64) > (lower_width as f64) * 1.03,
            "{tier:?}: no measurable perspective convergence \
             (upper/nearer {upper_width} px, lower/farther {lower_width} px)"
        );
    }
}

/// The headlight is **camera-relative**, not a world constant: orbiting the same
/// cube to a second azimuth must re-assign which face is brightest.
///
/// At azimuth 62 deg (same 32 deg elevation) the light direction turns toward
/// `+X`, so `+X` becomes the brightest face and `-Y` the darkest - the exact
/// opposite of the 27 deg frame. A fixed world light cannot produce this swap,
/// and neither can a per-face tint chosen in Rust: the shade is computed in the
/// shader from the frame's camera.
#[test]
fn oblique_cube_shading_follows_the_camera() {
    const FIRST_AZIMUTH: f64 = 27.0;
    const SECOND_AZIMUTH: f64 = 62.0;
    const ELEVATION: f64 = 32.0;
    for tier in [RenderTier::Universal, RenderTier::Accelerated] {
        let Some(first) = render_cube(oblique_camera_at(FIRST_AZIMUTH, ELEVATION), tier) else {
            return;
        };
        let Some(second) = render_cube(oblique_camera_at(SECOND_AZIMUTH, ELEVATION), tier) else {
            return;
        };
        let camera_a = oblique_camera_at(FIRST_AZIMUTH, ELEVATION);
        let camera_b = oblique_camera_at(SECOND_AZIMUTH, ELEVATION);
        let a = [
            face_shade(&first, camera_a, FRONT_FACE_SAMPLE),
            face_shade(&first, camera_a, SIDE_FACE_SAMPLE),
            face_shade(&first, camera_a, TOP_FACE_SAMPLE),
        ];
        let b = [
            face_shade(&second, camera_b, FRONT_FACE_SAMPLE),
            face_shade(&second, camera_b, SIDE_FACE_SAMPLE),
            face_shade(&second, camera_b, TOP_FACE_SAMPLE),
        ];
        println!(
            "{tier:?}: -Y/+X/+Z shades at {FIRST_AZIMUTH} deg {a:?}, at {SECOND_AZIMUTH} deg {b:?}"
        );

        // Both frames still show three distinguishable faces.
        for shades in [a, b] {
            for (i, j) in [(0, 1), (0, 2), (1, 2)] {
                assert!(
                    shades[i].abs_diff(shades[j]) >= 2,
                    "{tier:?}: faces {i} and {j} collapsed in {shades:?}"
                );
            }
        }
        // First azimuth: `-Y` is the brightest face. Second: `+X` is, and `-Y`
        // has fallen below it. The eye moved toward `+X`, so the light did too.
        assert!(
            a[0] > a[1] + 5,
            "{tier:?}: at {FIRST_AZIMUTH} deg the -Y face ({} ) must be well \
             brighter than +X ({}), got {a:?}",
            a[0],
            a[1]
        );
        assert!(
            b[1] > b[0] + 5,
            "{tier:?}: at {SECOND_AZIMUTH} deg the light must have followed the \
             camera to +X: expected +X ({}) well above -Y ({}), got {b:?}",
            b[1],
            b[0]
        );
    }
}

/// Non-vacuous control: the *same* cube looked at along `-Z` is a square with
/// four corners, so the hexagon assertions above can actually fail.
#[test]
fn top_down_cube_is_a_square_control() {
    for tier in [RenderTier::Universal, RenderTier::Accelerated] {
        let Some(image) = render_cube(top_down_camera(), tier) else {
            return;
        };
        let points = shape_points(&image);
        let (x0, y0, x1, y1) = bbox(&points);
        let poly = corner_polygon(&points, 2.0);
        let upper_width = width_at(&image, (y0, y1), 0.25);
        let lower_width = width_at(&image, (y0, y1), 0.75);
        println!(
            "{tier:?}: control shape px={} bbox={}x{} corners={} width_upper={} width_lower={}",
            points.len(),
            x1 - x0,
            y1 - y0,
            poly.len(),
            upper_width,
            lower_width
        );
        assert_eq!(
            poly.len(),
            4,
            "{tier:?}: the plan view of a cube must be a square, got {} corners at {poly:?}",
            poly.len()
        );
        assert_eq!(
            upper_width, lower_width,
            "{tier:?}: the control must be an unbiased (non-converging) square"
        );
        // The top-down view looks straight along `-Z`, so the camera-relative
        // light is exactly world `+Z`: the visible face must be fully lit (the
        // ambient floor must not dim a face the camera faces head-on).
        let face = face_shade(&image, top_down_camera(), TOP_FACE_SAMPLE);
        assert_eq!(
            face, 255,
            "{tier:?}: a top-down 3D view must show the +Z face fully lit, got {face}"
        );
    }
}
