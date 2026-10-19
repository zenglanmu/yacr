//! HATCH boundary and fill conversion.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// Convert a HATCH into a compound of boundary loops plus a solid fill or
    /// pattern lines. Boundaries are always emitted; fills that cannot be
    /// generated are reported as Partial rather than faked.
    pub(crate) fn hatch_geometry(h: &Hatch) -> (SemanticGeometry, Completeness) {
        let normal = p3(h.normal);
        // A degenerate normal cannot define a hatch plane; the boundary is still
        // emitted, but any fill is honestly reported as boundary-only.
        let plane_ok = cad_geometry::is_finite(normal) && cad_geometry::length(normal) > 1e-12;
        let (ux, uy, un) = arbitrary_axis(normal);
        let origin = cad_geometry::scale(un, h.elevation);
        let to_world = |p: [f64; 2]| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(ux, p[0]), cad_geometry::scale(uy, p[1])),
            )
        };
        let params = TessellationParams {
            tolerance: hatch_tolerance(h),
            ..TessellationParams::default()
        };
        let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut children: Vec<SemanticGeometry> = Vec::new();
        for path in &h.paths {
            let mut points: Vec<[f64; 2]> = Vec::new();
            for edge in &path.edges {
                append_boundary_edge(edge, &mut points, &params);
            }
            dedup_loop(&mut points);
            if points.len() < 3 {
                continue;
            }
            children.push(SemanticGeometry::Polyline {
                points: points.iter().map(|p| to_world(*p)).collect(),
                bulges: Vec::new(),
                closed: true,
            });
            loops.push(points);
        }
        if loops.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbHatch".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["hatch has no usable boundary".into()]),
            );
        }
        let mut completeness = Completeness::Complete;
        let solid = h.is_solid || h.pattern.name.eq_ignore_ascii_case("SOLID");

        // Simplify every ring with a tolerance scaled to the hatch's own extent.
        // This keeps the even-odd fill bounded (spec §3.2) and shared by the
        // solid and gradient paths.
        let fill_tolerance = {
            let (min, max) = loops.iter().flatten().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(mut lo, mut hi), p| {
                    lo[0] = lo[0].min(p[0]);
                    lo[1] = lo[1].min(p[1]);
                    hi[0] = hi[0].max(p[0]);
                    hi[1] = hi[1].max(p[1]);
                    (lo, hi)
                },
            );
            let diagonal = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt();
            (diagonal * 1e-3).max(1e-9)
        };
        let simplified: Vec<Vec<[f64; 2]>> = loops
            .iter()
            .filter_map(|loop2| {
                let mut closed = loop2.clone();
                closed.push(closed[0]);
                let mut s = cad_domain::hatch::simplify(&closed, fill_tolerance);
                if s.len() > 1 && s.last() == s.first() {
                    s.pop();
                }
                (s.len() >= 3).then_some(s)
            })
            .collect();

        // A gradient is checked before the solid flag: a gradient HATCH is
        // stored as a "solid" hatch with gradient metadata, so testing `solid`
        // first would silently draw it as a single flat colour.
        match translate_gradient(h) {
            GradientTranslation::Unsupported(code) => {
                // Never fake a gradient as solid or boundary-only-success.
                completeness = Completeness::Partial(vec![format!(
                    "gradient hatch not rendered ({code}); boundary only"
                )]);
            }
            GradientTranslation::Supported(def) => {
                if !plane_ok {
                    completeness = Completeness::Partial(vec![
                        "hatch plane normal is degenerate; boundary only".into(),
                    ]);
                } else {
                    match cad_domain::hatch::fill_rings(&simplified) {
                        Ok(fill) => {
                            let colors = cad_domain::hatch::gradient_vertex_colors(&fill, &def);
                            if colors.len() != fill.vertices.len() {
                                completeness = Completeness::Partial(vec![
                                    "gradient hatch could not be baked; boundary only".into(),
                                ]);
                            } else {
                                let vertices: Vec<Point3> =
                                    fill.vertices.iter().map(|p| to_world(*p)).collect();
                                let normals = vec![un; vertices.len()];
                                children.push(SemanticGeometry::Mesh(Mesh {
                                    vertices,
                                    triangles: fill.triangles,
                                    normals,
                                    face_sources: Vec::new(),
                                    colors,
                                }));
                            }
                        }
                        Err(error) => {
                            completeness = Completeness::Partial(vec![format!(
                                "gradient hatch could not be filled: {}",
                                error.reason()
                            )]);
                        }
                    }
                }
            }
            GradientTranslation::Disabled if solid => {
                if !plane_ok {
                    completeness = Completeness::Partial(vec![
                        "hatch plane normal is degenerate; boundary only".into(),
                    ]);
                } else {
                    // Multi-ring holes/islands go through the even-odd fill; a
                    // failure is reported as Partial (boundary only), never faked.
                    match cad_domain::hatch::fill_rings(&simplified) {
                        Ok(fill) => {
                            let vertices: Vec<Point3> =
                                fill.vertices.iter().map(|p| to_world(*p)).collect();
                            let normals = vec![un; vertices.len()];
                            children.push(SemanticGeometry::Mesh(Mesh {
                                vertices,
                                triangles: fill.triangles,
                                normals,
                                face_sources: Vec::new(),
                                colors: Vec::new(),
                            }));
                        }
                        Err(error) => {
                            completeness = Completeness::Partial(vec![format!(
                                "solid hatch could not be filled: {}",
                                error.reason()
                            )]);
                        }
                    }
                }
            }
            GradientTranslation::Disabled => {
                let families = pattern_families(h);
                if families.is_empty() {
                    completeness =
                        Completeness::Partial(vec!["hatch pattern has no line families".into()]);
                } else {
                    for line in cad_domain::hatch::pattern_polylines(&loops, &families) {
                        if line.len() >= 2 {
                            children.push(SemanticGeometry::Polyline {
                                points: line.iter().map(|p| to_world(*p)).collect(),
                                bulges: Vec::new(),
                                closed: false,
                            });
                        }
                    }
                }
            }
        }
        (SemanticGeometry::Compound(children), completeness)
    }
}
