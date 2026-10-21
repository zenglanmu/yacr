//! An **oblique** 3D view of a closed solid must show a cube, not one square
//! face.
//!
//! Regression (measured on the live host): a tessellated `[-1, 1]³` ACIS cube
//! was switched to the 3D view and orbited with a primary drag, and the CAD grab
//! came back as a **rotated square**: uniform white, no internal edges, no
//! perspective. The camera defect was in `cad_app::Camera::orbit` (an
//! incremental drag cancelled its own pitch, leaving the eye a few milliradians
//! off `+Z`, so the `+Z` face filled the frame). These tests fence the *renderer*
//! half: with an explicitly oblique `Camera3d` the 12-triangle cube must project
//! to a six-vertex silhouette, keep its mesh edge overlay, and show measurable
//! perspective convergence. The top-down control below is the non-vacuous
//! reference: the same measurements report a four-cornered square there.

use super::*;

use solid_fill::acis_cube;

use cad_render_wgpu::headless::RgbaImage;
use cad_render_wgpu::RenderTier;

/// Side of the rendered target, in pixels.
const SIZE: u32 = 256;

/// A camera in front of (`-Y`) and above (`+Z`) the cube, looking at the origin
/// with world `+Z` as screen up, and no sideways offset, so the silhouette is
/// symmetric: two vertical cube edges form the left/right extremes and the two
/// horizontal faces' edges cut the top/bottom corners.
fn oblique_camera() -> Camera3d {
    Camera3d {
        eye: Point3 {
            x: 0.0,
            y: -4.0,
            z: 3.0,
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

/// Count of interior pixels with a mid-tone luminance, i.e. the anti-aliased
/// mesh edge overlay drawn *inside* the silhouette.
///
/// The shape mask is eroded by `radius` so the silhouette's own anti-aliased
/// border cannot be counted; the two flat faces render at their own constant
/// values (the headlight-lit `+Z` face and the ambient-only `-Y` face) and are
/// excluded by the band.
fn interior_edge_pixels(image: &RgbaImage, radius: u32, band: (u8, u8)) -> usize {
    let mut count = 0;
    for y in radius..image.height.saturating_sub(radius) {
        for x in radius..image.width.saturating_sub(radius) {
            let all_in = (y - radius..=y + radius)
                .all(|ny| (x - radius..=x + radius).all(|nx| is_shape(image, nx, ny)));
            if !all_in {
                continue;
            }
            let v = lum(image, x, y);
            if v >= band.0 && v <= band.1 {
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
        let edge_pixels = interior_edge_pixels(&image, 2, (145, 250));
        let upper_width = width_at(&image, (y0, y1), 0.25);
        let lower_width = width_at(&image, (y0, y1), 0.75);
        println!(
            "{tier:?}: shape px={} bbox={}x{} corners={} angles={:?} edge_px={} \
             width_upper={} width_lower={}",
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
    }
}
