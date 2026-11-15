use super::*;

fn axis(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn a_scaled_basis_that_overflows_is_degenerate() {
    let u = axis(1.0, 0.0);
    let v = axis(0.0, 1.0);
    assert!(!image_basis_is_degenerate(u, v, [2.0, 3.0]));
    // The unscaled `u`/`v` are finite, but `u * pixels[0]` overflows the
    // world-space extent, so no valid texture transform exists.
    assert!(image_basis_is_degenerate(u, v, [f64::MAX, 2.0]));
}

#[test]
fn a_clipped_polygon_never_escapes_the_image_rectangle() {
    let pixels = [2.0, 3.0];
    let clipped = clip_polygon_to_image(
        &[[-1.0, -1.0], [1.0, -1.0], [1.0, 4.0], [-1.0, 4.0]],
        pixels,
    )
    .expect("the intersection is the full-height strip");
    assert_eq!(
        clipped,
        vec![[0.0, 3.0], [0.0, 0.0], [1.0, 0.0], [1.0, 3.0]]
    );
    assert!(clipped.iter().all(
        |point| (0.0..=pixels[0]).contains(&point[0]) && (0.0..=pixels[1]).contains(&point[1])
    ));
}

// ---- Concave (`IMAGECLIP`) clips -------------------------------------

/// The identity pixel→world basis: world XY equals pixel XY, so the tests
/// below can talk about areas and containment in one coordinate system.
fn identity_frame() -> (Point3, Point3, Point3, [f64; 2]) {
    (
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        [10.0, 10.0],
    )
}

fn inside_clip(vertices: &[[f64; 2]]) -> ImageClip {
    ImageClip {
        vertices: vertices.to_vec(),
        inside: true,
    }
}

fn pieces(clip: &ImageClip) -> Vec<Vec<ImageVertex>> {
    let (origin, u, v, pixels) = identity_frame();
    image_clip_pieces(clip, origin, u, v, pixels)
        .expect("the clip is usable")
        .expect("the clip has visible area")
}

fn refusal_code(clip: &ImageClip) -> &'static str {
    let (origin, u, v, pixels) = identity_frame();
    image_clip_pieces(clip, origin, u, v, pixels)
        .expect_err("the clip must be refused")
        .code
}

/// A world-space triangle of one emitted piece.
fn triangle(piece: &[ImageVertex]) -> [[f64; 2]; 3] {
    assert_eq!(piece.len(), 3, "an ear inside the image rect is a triangle");
    [
        [piece[0].position.x, piece[0].position.y],
        [piece[1].position.x, piece[1].position.y],
        [piece[2].position.x, piece[2].position.y],
    ]
}

fn triangle_area(t: &[[f64; 2]; 3]) -> f64 {
    (turn(t[0], t[1], t[2])).abs() * 0.5
}

/// Area of the convex intersection of two triangles, by clipping `a` with
/// every edge of `b` and taking the area of what survives.
fn overlap_area(a: &[[f64; 2]; 3], b: &[[f64; 2]; 3]) -> f64 {
    let mut subject: Vec<[f64; 2]> = a.to_vec();
    for index in 0..3 {
        let (from, to) = (b[index], b[(index + 1) % 3]);
        // Keep the left of `from→to`, which is `b`'s own side (CCW).
        let mut kept: Vec<[f64; 2]> = Vec::with_capacity(subject.len() + 1);
        for position in 0..subject.len() {
            let current = subject[position];
            let previous = subject[(position + subject.len() - 1) % subject.len()];
            let inside = |point: [f64; 2]| turn(from, to, point) >= 0.0;
            if inside(current) {
                if !inside(previous) {
                    kept.push(segment_crossing(previous, current, from, to));
                }
                kept.push(current);
            } else if inside(previous) {
                kept.push(segment_crossing(previous, current, from, to));
            }
        }
        subject = kept;
        if subject.len() < 3 {
            return 0.0;
        }
    }
    let mut sum = 0.0;
    for index in 0..subject.len() {
        let point = subject[index];
        let next = subject[(index + 1) % subject.len()];
        sum += point[0] * next[1] - next[0] * point[1];
    }
    (sum * 0.5).abs()
}

fn segment_crossing(a: [f64; 2], b: [f64; 2], from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let denominator = turn(from, to, a) - turn(from, to, b);
    if denominator == 0.0 {
        return b;
    }
    let t = turn(from, to, a) / denominator;
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
}

/// Whether `point` lies inside or on the boundary of a triangle.
fn triangle_holds(t: &[[f64; 2]; 3], point: [f64; 2]) -> bool {
    let area = turn(t[0], t[1], t[2]);
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    sign * turn(t[0], t[1], point) >= -1e-9
        && sign * turn(t[1], t[2], point) >= -1e-9
        && sign * turn(t[2], t[0], point) >= -1e-9
}

/// An L-shaped (notched) clip: the big 8x8 square with its 5x5 top-right
/// quadrant removed, so the notch (`x > 4`, `y > 4`) must stay empty.
fn l_shaped_clip() -> ImageClip {
    inside_clip(&[
        [1.0, 1.0],
        [9.0, 1.0],
        [9.0, 4.0],
        [4.0, 4.0],
        [4.0, 9.0],
        [1.0, 9.0],
    ])
}

#[test]
fn a_concave_clip_is_ear_clipped_into_a_disjoint_exact_tiling() {
    let pieces = pieces(&l_shaped_clip());
    let triangles: Vec<[[f64; 2]; 3]> = pieces.iter().map(|piece| triangle(piece)).collect();
    // A hexagon ear-clips into n-2 = 4 triangles, none of which the image
    // rectangle cuts.
    assert_eq!(triangles.len(), 4);

    // Every ear is real area and follows the stored boundary: all three
    // vertices are clip vertices (no invented, chorded or boxed corners).
    let ring = l_shaped_clip().vertices;
    for t in &triangles {
        assert!(triangle_area(t) > 0.0);
        for vertex in t {
            assert!(
                ring.iter().any(|p| p[0] == vertex[0] && p[1] == vertex[1]),
                "{vertex:?} is not a clip boundary vertex"
            );
        }
    }

    // The ears tile the polygon exactly: no area is missing or counted twice.
    let polygon_area = 8.0 * 8.0 - 5.0 * 5.0;
    let emitted: f64 = triangles.iter().map(triangle_area).sum();
    assert!(
        (emitted - polygon_area).abs() < 1e-9,
        "emitted {emitted}, polygon {polygon_area}"
    );

    // No two ears overlap.
    for first in 0..triangles.len() {
        for second in (first + 1)..triangles.len() {
            let overlap = overlap_area(&triangles[first], &triangles[second]);
            assert!(
                overlap < 1e-9,
                "ears {first} and {second} overlap by {overlap}"
            );
        }
    }

    // The union covers the interior and nothing in the notch.
    for interior in [[2.0, 2.0], [8.0, 2.0], [2.0, 8.0], [3.0, 3.0]] {
        assert!(
            triangles.iter().any(|t| triangle_holds(t, interior)),
            "{interior:?} is interior but no ear covers it"
        );
    }
    for notch in [[5.0, 5.0], [8.0, 8.0], [4.5, 6.0], [6.0, 8.5]] {
        assert!(
            !triangles.iter().any(|t| triangle_holds(t, notch)),
            "{notch:?} is in the notch but an ear covers it"
        );
    }
}

#[test]
fn a_concave_clip_survives_either_winding() {
    let mut clockwise = l_shaped_clip();
    clockwise.vertices.reverse();
    let triangles: Vec<[[f64; 2]; 3]> = pieces(&clockwise)
        .iter()
        .map(|piece| triangle(piece))
        .collect();
    let emitted: f64 = triangles.iter().map(triangle_area).sum();
    assert!((emitted - 39.0).abs() < 1e-9, "area {emitted}");
}

#[test]
fn a_concave_clip_partly_outside_the_image_is_clipped_per_ear() {
    // The same L, shifted so its left arm leaves the 10x10 image rectangle.
    let clip = inside_clip(&[
        [-2.0, 1.0],
        [9.0, 1.0],
        [9.0, 4.0],
        [4.0, 4.0],
        [4.0, 9.0],
        [-2.0, 9.0],
    ]);
    let pieces = pieces(&clip);
    let mut emitted = 0.0;
    for piece in &pieces {
        // Every piece stays convex (a triangle cut by the rectangle is a
        // convex polygon), so the renderer's fan is still exact.
        let positions: Vec<[f64; 2]> = piece
            .iter()
            .map(|vertex| [vertex.position.x, vertex.position.y])
            .collect();
        assert!(polygon_is_convex(&positions));
        assert!(positions
            .iter()
            .all(|p| (0.0..=10.0).contains(&p[0]) && (0.0..=10.0).contains(&p[1])));
        assert!(piece.iter().all(
            |vertex| (0.0..=1.0).contains(&vertex.uv[0]) && (0.0..=1.0).contains(&vertex.uv[1])
        ));
        let mut sum = 0.0;
        for index in 0..positions.len() {
            let point = positions[index];
            let next = positions[(index + 1) % positions.len()];
            sum += point[0] * next[1] - next[0] * point[1];
        }
        emitted += (sum * 0.5).abs();
    }
    // x in [0,4] x y in [1,9] (32) plus x in [4,9] x y in [1,4] (15): the
    // part of the L that lies inside the image rectangle.
    assert!((emitted - (32.0 + 15.0)).abs() < 1e-9, "area {emitted}");
}

#[test]
fn a_convex_clip_is_one_piece_byte_identical_to_the_previous_output() {
    // The rectangle case frozen by `image_with_a_rectangular_clip_maps_the_visible_polygon`
    // before concave clips were supported: the same four vertices, values
    // and order, so the convex path cannot drift.
    let clip = inside_clip(&[[0.0, 0.0], [2.0, 3.0]]);
    let pixels = [2.0, 3.0];
    let pieces = image_clip_pieces(
        &clip,
        Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        pixels,
    )
    .expect("a rectangle clip is usable")
    .expect("a rectangle clip has visible area");
    assert_eq!(pieces.len(), 1, "a convex clip stays one fan primitive");
    assert_eq!(
        pieces[0],
        vec![
            ImageVertex {
                position: Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0
                },
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 3.0,
                    y: 2.0,
                    z: 0.0
                },
                uv: [1.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 3.0,
                    y: 5.0,
                    z: 0.0
                },
                uv: [1.0, 1.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 1.0,
                    y: 5.0,
                    z: 0.0
                },
                uv: [0.0, 1.0],
            },
        ]
    );
}

#[test]
fn a_convex_polygon_clip_stays_a_single_piece_in_stored_order() {
    let clip = inside_clip(&[[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]]);
    let pieces = pieces(&clip);
    assert_eq!(pieces.len(), 1);
    let stored: Vec<[f64; 2]> = pieces[0]
        .iter()
        .map(|vertex| [vertex.position.x, vertex.position.y])
        .collect();
    assert_eq!(stored, clip.vertices);
}

#[test]
fn a_sixteen_gon_circle_clip_is_one_convex_piece() {
    let center = 5.0;
    let radius = 4.0;
    let mut vertices = Vec::with_capacity(16);
    for index in 0..16 {
        let angle = std::f64::consts::TAU * index as f64 / 16.0;
        vertices.push([center + radius * angle.cos(), center + radius * angle.sin()]);
    }
    let clip = inside_clip(&vertices);
    let pieces = pieces(&clip);
    assert_eq!(pieces.len(), 1, "a 16-gon is convex");
    assert_eq!(pieces[0].len(), 16);
    assert!(pieces[0]
        .iter()
        .all(|vertex| (0.0..=1.0).contains(&vertex.uv[0])));
    // The piece covers the polygon's own area, so no chord or corner was
    // invented: compare with the shoelace formula.
    let mut sum = 0.0;
    for index in 0..16 {
        let point = vertices[index];
        let next = vertices[(index + 1) % 16];
        sum += point[0] * next[1] - next[0] * point[1];
    }
    let piece_area = {
        let mut sum = 0.0;
        for index in 0..pieces[0].len() {
            let point = pieces[0][index].position;
            let next = pieces[0][(index + 1) % pieces[0].len()].position;
            sum += point.x * next.y - next.x * point.y;
        }
        (sum * 0.5).abs()
    };
    assert!((piece_area - (sum * 0.5).abs()).abs() < 1e-9);
}

#[test]
fn a_convex_clip_with_a_collinear_vertex_keeps_every_stored_vertex() {
    // The midpoint of the left edge is stored but adds no area; it must stay
    // in the single emitted piece so the boundary is followed exactly.
    let clip = inside_clip(&[[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0], [1.0, 5.0]]);
    let pieces = pieces(&clip);
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].len(), 5);
}

#[test]
fn a_concave_clip_with_a_collinear_vertex_tiles_the_same_area() {
    let clip = inside_clip(&[
        [1.0, 1.0],
        [5.0, 1.0],
        [9.0, 1.0],
        [9.0, 4.0],
        [4.0, 4.0],
        [4.0, 9.0],
        [1.0, 9.0],
    ]);
    let triangles: Vec<[[f64; 2]; 3]> = pieces(&clip).iter().map(|piece| triangle(piece)).collect();
    assert_eq!(triangles.len(), 5, "seven vertices ear-clip into five ears");
    let emitted: f64 = triangles.iter().map(triangle_area).sum();
    assert!((emitted - 39.0).abs() < 1e-9, "area {emitted}");
}

#[test]
fn a_self_intersecting_clip_is_refused_by_name() {
    // Two crossing lobes with different areas: the signed area is non-zero,
    // so only the simplicity test can catch it.
    let crossing = inside_clip(&[[1.0, 1.0], [9.0, 9.0], [9.0, 5.0], [1.0, 3.0]]);
    assert_eq!(
        refusal_code(&crossing),
        image_clip_reason::SELF_INTERSECTING
    );
    // A pentagram has a consistent turn sign (so it would pass a convexity
    // test) but crosses itself: simplicity is checked first.
    let pentagram = inside_clip(&[[5.0, 1.0], [7.0, 8.0], [1.0, 4.0], [9.0, 4.0], [3.0, 8.0]]);
    assert_eq!(
        refusal_code(&pentagram),
        image_clip_reason::SELF_INTERSECTING
    );
}

#[test]
fn a_clip_with_too_few_points_is_refused_by_name() {
    assert_eq!(
        refusal_code(&inside_clip(&[[1.0, 1.0]])),
        image_clip_reason::ARITY
    );
    // Two "corners" that share an x coordinate do not span a rectangle.
    assert_eq!(
        refusal_code(&inside_clip(&[[1.0, 1.0], [1.0, 5.0]])),
        image_clip_reason::ARITY
    );
}

#[test]
fn a_zero_area_clip_is_refused_by_name() {
    // Three collinear points describe no interior.
    assert_eq!(
        refusal_code(&inside_clip(&[[1.0, 1.0], [5.0, 1.0], [9.0, 1.0]])),
        image_clip_reason::DEGENERATE
    );
    // Every vertex coincides.
    assert_eq!(
        refusal_code(&inside_clip(&[[2.0, 2.0], [2.0, 2.0], [2.0, 2.0]])),
        image_clip_reason::DEGENERATE
    );
}

#[test]
fn an_outside_clip_is_refused_by_name() {
    let mut clip = inside_clip(&[[1.0, 1.0], [9.0, 1.0], [9.0, 9.0]]);
    clip.inside = false;
    assert_eq!(refusal_code(&clip), image_clip_reason::UNSUPPORTED);
}

#[test]
fn a_well_formed_clip_with_no_area_in_the_image_is_not_drawn() {
    let clip = inside_clip(&[[20.0, 20.0], [30.0, 20.0], [30.0, 30.0], [20.0, 30.0]]);
    let (origin, u, v, pixels) = identity_frame();
    assert!(
        image_clip_pieces(&clip, origin, u, v, pixels)
            .expect("the ring itself is well formed")
            .is_none(),
        "a clip entirely outside the image rectangle emits no piece"
    );
}
