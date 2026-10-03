//! Conversions for the remaining drawable primitive entity kinds.
//!
//! Each function returns neutral [`SemanticGeometry`] plus an honest
//! [`Completeness`]. Kinds with no analytic display path (shapes, mline,
//! multileader, table, tolerance, raster reference) stay out of this module and
//! remain explicit `Partial` at the call site.

use super::*;

/// A polyline through already-world-space points.
pub(crate) fn polyline_semantics(points: Vec<Point3>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges: Vec::new(),
        closed,
    }
}

/// Build a triangle mesh, dropping degenerate/out-of-range faces.
pub(crate) fn triangle_mesh(vertices: Vec<Point3>, faces: &[Vec<usize>]) -> SemanticGeometry {
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for face in faces {
        let mut fan = face
            .iter()
            .copied()
            .filter(|index| *index < vertices.len())
            .collect::<Vec<_>>();
        fan.dedup();
        if fan.len() < 3 {
            continue;
        }
        for i in 1..fan.len() - 1 {
            triangles.push([fan[0] as u32, fan[i] as u32, fan[i + 1] as u32]);
        }
    }
    SemanticGeometry::Mesh(Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    })
}

fn polyface_index(raw: i16, len: usize) -> Option<usize> {
    if raw == 0 {
        return None;
    }
    // Polyface indices are 1-based; a negative value marks an invisible edge.
    let index = (raw.unsigned_abs() as usize).checked_sub(1)?;
    (index < len).then_some(index)
}

pub(crate) fn polyface_mesh_semantics(m: &acadrust::entities::PolyfaceMesh) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(v.location)).collect();
    let len = vertices.len();
    let faces: Vec<Vec<usize>> = m
        .faces
        .iter()
        .map(|face| {
            [face.index1, face.index2, face.index3, face.index4]
                .into_iter()
                .filter_map(|raw| polyface_index(raw, len))
                .collect()
        })
        .collect();
    triangle_mesh(vertices, &faces)
}

pub(crate) fn polygon_mesh_semantics(
    m: &acadrust::entities::PolygonMeshEntity,
) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(v.location)).collect();
    let columns = m.m_vertex_count.max(0) as usize;
    let rows = m.n_vertex_count.max(0) as usize;
    let mut faces: Vec<Vec<usize>> = Vec::new();
    if columns >= 2 && rows >= 2 && vertices.len() >= columns * rows {
        for row in 0..rows - 1 {
            for column in 0..columns - 1 {
                let a = row * columns + column;
                let b = a + 1;
                let c = a + columns + 1;
                let d = a + columns;
                faces.push(vec![a, b, c, d]);
            }
        }
    }
    triangle_mesh(vertices, &faces)
}

pub(crate) fn subd_mesh_semantics(m: &acadrust::entities::Mesh) -> SemanticGeometry {
    let vertices: Vec<Point3> = m.vertices.iter().map(|v| p3(*v)).collect();
    let faces: Vec<Vec<usize>> = m.faces.iter().map(|face| face.vertices.clone()).collect();
    triangle_mesh(vertices, &faces)
}

/// A WIPEOUT's clip boundary is drawn as a closed polyline; the masking effect
/// itself is reported `Partial` because it needs a draw-order/background path.
pub(crate) fn wipeout_semantics(
    w: &acadrust::entities::Wipeout,
) -> (SemanticGeometry, Completeness) {
    let origin = p3(w.insertion_point);
    let u = p3(w.u_vector);
    let v = p3(w.v_vector);
    let mut points: Vec<Point3> = w
        .clip_boundary_vertices
        .iter()
        .map(|p| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(u, p.x), cad_geometry::scale(v, p.y)),
            )
        })
        .collect();
    if points.len() < 3 {
        points = vec![
            origin,
            cad_geometry::add(origin, cad_geometry::scale(u, w.size.x)),
            cad_geometry::add(
                origin,
                cad_geometry::add(
                    cad_geometry::scale(u, w.size.x),
                    cad_geometry::scale(v, w.size.y),
                ),
            ),
            cad_geometry::add(origin, cad_geometry::scale(v, w.size.y)),
        ];
    }
    (
        polyline_semantics(points, true),
        Completeness::Partial(vec![
            "wipeout clip boundary drawn; the masking fill is not applied".into(),
        ]),
    )
}

/// A paper-space VIEWPORT's border rectangle. The viewport's model content is
/// assembled by the plot path, not here; the border is drawable geometry.
pub(crate) fn viewport_semantics(v: &acadrust::entities::Viewport) -> SemanticGeometry {
    let c = p3(v.center);
    let hw = v.width.abs() / 2.0;
    let hh = v.height.abs() / 2.0;
    polyline_semantics(
        vec![
            Point3 {
                x: c.x - hw,
                y: c.y - hh,
                z: c.z,
            },
            Point3 {
                x: c.x + hw,
                y: c.y - hh,
                z: c.z,
            },
            Point3 {
                x: c.x + hw,
                y: c.y + hh,
                z: c.z,
            },
            Point3 {
                x: c.x - hw,
                y: c.y + hh,
                z: c.z,
            },
        ],
        true,
    )
}

/// A geometric TOLERANCE: a frame rectangle plus the (font-dependent) text.
/// The frame width is estimated from the text length because the exact extent
/// needs shaped glyph metrics; the estimate is reported `Partial`.
pub(crate) fn tolerance_frame(t: &acadrust::entities::Tolerance) -> (SemanticGeometry, f64, f64) {
    let origin = p3(t.insertion_point);
    let height = if t.text_height.is_finite() && t.text_height > 0.0 {
        t.text_height
    } else {
        2.5
    };
    let frame_height = height * 1.6;
    let frame_width = (t.text.chars().count().max(1) as f64) * height * 0.62 + height;
    let direction = cad_geometry::normalize(p3(t.direction));
    let angle = direction.y.atan2(direction.x);
    let ux = Point3 {
        x: angle.cos(),
        y: angle.sin(),
        z: 0.0,
    };
    let uy = Point3 {
        x: -angle.sin(),
        y: angle.cos(),
        z: 0.0,
    };
    let corner = |u: f64, w: f64| {
        cad_geometry::add(
            origin,
            cad_geometry::add(cad_geometry::scale(ux, u), cad_geometry::scale(uy, w)),
        )
    };
    let mut points = vec![
        corner(0.0, 0.0),
        corner(frame_width, 0.0),
        corner(frame_width, frame_height),
        corner(0.0, frame_height),
    ];
    points.dedup();
    (polyline_semantics(points, true), frame_width, frame_height)
}

/// A RASTERIMAGE frame. The pixel data needs a host image decoder and a texture
/// pipeline, which this build does not have, so the frame (and clip boundary)
/// is drawn and the texture is reported `Partial`.
pub(crate) fn raster_image_semantics(
    img: &acadrust::entities::RasterImage,
) -> (SemanticGeometry, Completeness) {
    let origin = p3(img.insertion_point);
    let u = p3(img.u_vector);
    let v = p3(img.v_vector);
    let mut points = vec![
        origin,
        cad_geometry::add(origin, cad_geometry::scale(u, img.size.x)),
        cad_geometry::add(
            origin,
            cad_geometry::add(
                cad_geometry::scale(u, img.size.x),
                cad_geometry::scale(v, img.size.y),
            ),
        ),
        cad_geometry::add(origin, cad_geometry::scale(v, img.size.y)),
    ];
    points.dedup();
    (
        polyline_semantics(points, true),
        Completeness::Partial(vec![
            "raster image texture is not decoded; the image frame is drawn".into(),
        ]),
    )
}

/// An infinite construct line (RAY / XLINE) clipped to an XY box.
///
/// A fixed-viewport render has no per-view clipping, so the line is clipped to
/// the drawing's model bounds (tracked as entities are imported). When no other
/// geometry has been seen yet a default 1000-unit box is used and the result is
/// reported `Partial`, because the clip length is then a guess rather than the
/// real drawing extent.
pub(crate) fn ray_semantics(
    base: Point3,
    direction: Point3,
    is_ray: bool,
    bounds: Option<(Point3, Point3)>,
) -> (SemanticGeometry, Completeness) {
    let opaque = || SemanticGeometry::Opaque {
        type_key: if is_ray {
            "AcDbRay".into()
        } else {
            "AcDbXline".into()
        },
        version: 1,
        payload: Vec::new(),
    };
    if !cad_geometry::is_finite(base)
        || !cad_geometry::is_finite(direction)
        || cad_geometry::length(direction) < 1e-12
    {
        return (
            opaque(),
            Completeness::Partial(vec!["ray/xline direction is degenerate".into()]),
        );
    }
    let direction = cad_geometry::normalize(direction);
    let (min, max, margin, note) = match bounds {
        Some((lo, hi)) => {
            let margin = (cad_geometry::distance(lo, hi) * 0.05).max(1.0);
            (lo, hi, margin, None)
        }
        None => (
            Point3 {
                x: base.x - 1.0e3,
                y: base.y - 1.0e3,
                z: 0.0,
            },
            Point3 {
                x: base.x + 1.0e3,
                y: base.y + 1.0e3,
                z: 0.0,
            },
            0.0,
            Some("ray/xline clipped to a default 1000-unit box; no drawing bounds were available"),
        ),
    };
    let min = Point3 {
        x: min.x - margin,
        y: min.y - margin,
        z: 0.0,
    };
    let max = Point3 {
        x: max.x + margin,
        y: max.y + margin,
        z: 0.0,
    };
    let span = ((max.x - min.x).abs().max((max.y - min.y).abs()) * 4.0).max(1.0);
    let a = if is_ray {
        base
    } else {
        cad_geometry::add(base, cad_geometry::scale(direction, -span))
    };
    let b = cad_geometry::add(base, cad_geometry::scale(direction, span));
    let completeness = match note {
        Some(reason) => Completeness::Partial(vec![reason.into()]),
        None => Completeness::Complete,
    };
    match cad_geometry::clip_segment_to_xy_rect(a, b, (min.x, min.y), (max.x, max.y)) {
        Some((start, end)) if cad_geometry::distance(start, end) > 1e-9 => {
            (SemanticGeometry::Line { start, end }, completeness)
        }
        _ => (
            opaque(),
            Completeness::Partial(vec![
                "ray/xline does not intersect the drawing bounds".into()
            ]),
        ),
    }
}

/// An MLINE as its parallel element polylines.
///
/// The per-vertex `segments[].parameters` carry the element offsets, so the
/// parallel lines can be reconstructed without the MLINESTYLE table (joins and
/// caps are still approximated, hence `Partial`). When the parameters are
/// unavailable the vertex centerline is drawn as an explicit fallback.
pub(crate) fn mline_semantics(m: &acadrust::entities::MLine) -> (SemanticGeometry, Completeness) {
    let count = m.vertices.len();
    let scale = if m.scale_factor.is_finite() && m.scale_factor.abs() > 1e-9 {
        m.scale_factor
    } else {
        1.0
    };
    // Direction per vertex: the stored direction, else the neighbour chord.
    let directions: Vec<Point3> = (0..count)
        .map(|i| {
            let given = cad_geometry::normalize(p3(m.vertices[i].direction));
            if cad_geometry::length(given) > 1e-9 {
                return given;
            }
            let prev = if i > 0 {
                p3(m.vertices[i - 1].position)
            } else {
                p3(m.vertices[i].position)
            };
            let next = if i + 1 < count {
                p3(m.vertices[i + 1].position)
            } else {
                p3(m.vertices[i].position)
            };
            cad_geometry::normalize(cad_geometry::sub(next, prev))
        })
        .collect();
    let element_count = m
        .vertices
        .iter()
        .find_map(|v| v.segments.first().map(|s| s.parameters.len()))
        .unwrap_or(0);
    let mut children: Vec<SemanticGeometry> = Vec::new();
    for element in 0..element_count {
        let points: Vec<Point3> = m
            .vertices
            .iter()
            .enumerate()
            .filter_map(|(i, vertex)| {
                let offset = vertex.segments.first()?.parameters.get(element).copied()?;
                let perp = Point3 {
                    x: -directions[i].y,
                    y: directions[i].x,
                    z: 0.0,
                };
                Some(cad_geometry::add(
                    p3(vertex.position),
                    cad_geometry::scale(perp, offset * scale),
                ))
            })
            .collect();
        if points.len() >= 2 {
            children.push(polyline_semantics(points, false));
        }
    }
    if children.is_empty() {
        let mut points: Vec<Point3> = m.vertices.iter().map(|v| p3(v.position)).collect();
        if let Some(first) = points.first().copied() {
            if cad_geometry::distance(p3(m.start_point), first) > 1e-9 {
                points.insert(0, p3(m.start_point));
            }
        }
        return (
            polyline_semantics(points, false),
            Completeness::Partial(vec![
                "mline drawn as its centerline; per-element offsets were unavailable".into(),
            ]),
        );
    }
    (
        SemanticGeometry::Compound(children),
        Completeness::Partial(vec![
            "mline element offsets drawn; joins and caps are approximated".into(),
        ]),
    )
}

/// The frame rectangle of an OLE2FRAME. The embedded object is not decoded, so
/// the frame is drawn and the content reported `Partial`.
pub(crate) fn ole2frame_semantics(
    f: &acadrust::entities::Ole2Frame,
) -> (SemanticGeometry, Completeness) {
    let a = p3(f.upper_left_corner);
    let b = p3(f.lower_right_corner);
    let points = vec![
        Point3 {
            x: a.x,
            y: a.y,
            z: a.z,
        },
        Point3 {
            x: b.x,
            y: a.y,
            z: a.z,
        },
        Point3 {
            x: b.x,
            y: b.y,
            z: b.z,
        },
        Point3 {
            x: a.x,
            y: b.y,
            z: a.z,
        },
    ];
    (
        polyline_semantics(points, true),
        Completeness::Partial(vec![
            "OLE object frame drawn; embedded content is not decoded".into(),
        ]),
    )
}

/// A view border rectangle (paper coordinates), honouring its rotation/scale
/// about its own centre.
pub(crate) fn view_border_semantics(
    v: &acadrust::entities::ViewBorder,
) -> (SemanticGeometry, Completeness) {
    let cx = (v.min[0] + v.max[0]) / 2.0;
    let cy = (v.min[1] + v.max[1]) / 2.0;
    let (sin, cos) = v.rotation_angle.sin_cos();
    let corner = |x: f64, y: f64| {
        let (dx, dy) = ((x - cx) * v.scale, (y - cy) * v.scale);
        Point3 {
            x: cx + dx * cos - dy * sin,
            y: cy + dx * sin + dy * cos,
            z: 0.0,
        }
    };
    let points = vec![
        corner(v.min[0], v.min[1]),
        corner(v.max[0], v.min[1]),
        corner(v.max[0], v.max[1]),
        corner(v.min[0], v.max[1]),
    ];
    (
        polyline_semantics(points, true),
        Completeness::Partial(vec!["view border rectangle only".into()]),
    )
}

/// A section symbol as the polyline through its points plus the end line.
pub(crate) fn section_symbol_semantics(
    s: &acadrust::entities::SectionSymbol,
) -> (SemanticGeometry, Completeness) {
    let mut children: Vec<SemanticGeometry> = Vec::new();
    let points: Vec<Point3> = s.points.iter().map(|p| p3(p.point)).collect();
    if points.len() >= 2 {
        children.push(polyline_semantics(points, false));
    }
    let a = Point3 {
        x: s.end_a[0],
        y: s.end_a[1],
        z: 0.0,
    };
    let b = Point3 {
        x: s.end_b[0],
        y: s.end_b[1],
        z: 0.0,
    };
    if let Some(line) = line_between(a, b) {
        children.push(line);
    }
    if children.is_empty() {
        return (
            SemanticGeometry::Opaque {
                type_key: "AcDbSectionSymbol".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec!["section symbol has no drawable points".into()]),
        );
    }
    (
        SemanticGeometry::Compound(children),
        Completeness::Partial(vec![
            "section symbol geometry drawn; labels/arrows are approximated".into(),
        ]),
    )
}

/// An underlay reference: its clip boundary when present. The referenced PDF/
/// DWF/DGN content is external and is not loaded, so this is `Partial`.
pub(crate) fn underlay_semantics(
    u: &acadrust::entities::Underlay,
) -> (SemanticGeometry, Completeness) {
    let boundary: Vec<Point3> = u.world_clip_boundary().iter().map(|p| p3(*p)).collect();
    if boundary.len() >= 3 {
        return (
            polyline_semantics(boundary, true),
            Completeness::Partial(vec![
                "underlay clip boundary drawn; external content is not loaded".into(),
            ]),
        );
    }
    (
        SemanticGeometry::Opaque {
            type_key: "AcDbUnderlayReference".into(),
            version: 1,
            payload: Vec::new(),
        },
        Completeness::Partial(vec![
            "underlay has no clip boundary; external content is not loaded".into(),
        ]),
    )
}

/// A light's optional plot glyph: a small cross at its position. Lights have no
/// solid geometry; without the glyph this would have no representation.
pub(crate) fn light_semantics(l: &acadrust::entities::Light) -> (SemanticGeometry, Completeness) {
    if !l.plot_glyph || !cad_geometry::is_finite(p3(l.position)) {
        return (
            SemanticGeometry::Opaque {
                type_key: "AcDbLight".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec!["light has no plotted geometry".into()]),
        );
    }
    let p = p3(l.position);
    let size = 0.5;
    let children = vec![
        SemanticGeometry::Line {
            start: Point3 {
                x: p.x - size,
                y: p.y,
                z: p.z,
            },
            end: Point3 {
                x: p.x + size,
                y: p.y,
                z: p.z,
            },
        },
        SemanticGeometry::Line {
            start: Point3 {
                x: p.x,
                y: p.y - size,
                z: p.z,
            },
            end: Point3 {
                x: p.x,
                y: p.y + size,
                z: p.z,
            },
        },
    ];
    (
        SemanticGeometry::Compound(children),
        Completeness::Partial(vec!["light plot glyph only; no light model".into()]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn quad_face_fans_into_two_triangles_and_degenerate_faces_are_dropped() {
        let vertices = vec![
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(1.0, 1.0),
            point(0.0, 1.0),
        ];
        let geometry = triangle_mesh(
            vertices.clone(),
            &[vec![0, 1, 2, 3], vec![0, 1], vec![0, 1, 9]],
        );
        let SemanticGeometry::Mesh(mesh) = geometry else {
            panic!("expected a mesh");
        };
        assert_eq!(mesh.triangles.len(), 2, "{:?}", mesh.triangles);
    }

    #[test]
    fn xline_clips_to_the_box_and_ray_keeps_only_the_positive_half() {
        let bounds = Some((point(-10.0, -10.0), point(10.0, 10.0)));
        let (xline, completeness) = ray_semantics(point(0.0, 0.0), point(1.0, 0.0), false, bounds);
        assert_eq!(completeness, Completeness::Complete);
        let SemanticGeometry::Line { start, end } = xline else {
            panic!("expected a clipped line");
        };
        assert!(approx(start.x, -end.x), "{start:?} {end:?}");
        assert!(approx(start.y, 0.0) && approx(end.y, 0.0));

        let (ray, _) = ray_semantics(point(0.0, 0.0), point(1.0, 0.0), true, bounds);
        let SemanticGeometry::Line { start, .. } = ray else {
            panic!("expected a clipped ray");
        };
        assert!(approx(start.x, 0.0), "ray must not start behind its base");
    }

    #[test]
    fn degenerate_infinite_line_direction_is_partial_not_a_fake_line() {
        let (geometry, completeness) = ray_semantics(point(0.0, 0.0), point(0.0, 0.0), true, None);
        assert!(matches!(geometry, SemanticGeometry::Opaque { .. }));
        assert!(matches!(completeness, Completeness::Partial(_)));
    }

    #[test]
    fn polyface_indices_are_one_based_and_tolerate_negative_edges() {
        assert_eq!(polyface_index(1, 3), Some(0));
        assert_eq!(polyface_index(-3, 3), Some(2));
        assert_eq!(polyface_index(0, 3), None);
        assert_eq!(polyface_index(4, 3), None);
    }
}
