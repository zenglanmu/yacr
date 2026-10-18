//! Dimension and LEADER entry points: subtype dispatch, text placement and the
//! unsupported/degenerate fallbacks.
//!
//! Submodule of [`crate::dimension`].

use super::*;
use acadrust::entities::{Dimension, DimensionBase, Leader, LeaderPathType};

impl ImporterBuilder<'_> {
    fn dimension_text(
        &self,
        content: &str,
        position: Point3,
        rotation: f64,
        style: &DimStyleValues,
    ) -> SemanticGeometry {
        SemanticGeometry::Text {
            text: content.to_string(),
            position,
            style: self.style_id(&style.text_style),
            height: style.text_height,
            rotation,
            font: self.style_font(&style.text_style),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
        }
    }

    /// Preferred text anchor: the entity's explicit middle point when set, else
    /// the placement the geometry derives from the dimension line.
    fn place_dimension_text(
        &self,
        base: &DimensionBase,
        geometry: &DimensionGeometry,
        style: &DimStyleValues,
        prefix: &str,
        suffix: &str,
    ) -> SemanticGeometry {
        let explicit = p3(base.text_middle_point);
        let position = if cad_geometry::length(explicit) > 1e-9 {
            explicit
        } else {
            geometry.text_position
        };
        let rotation = if base.text_rotation.abs() > 1e-12 {
            base.text_rotation
        } else {
            geometry.text_rotation
        };
        let content = resolve_measurement_text(base, prefix, suffix, geometry.measurement, style);
        self.dimension_text(&content, position, rotation, style)
    }

    fn unsupported_dimension(
        base: &DimensionBase,
        reason: String,
    ) -> (SemanticGeometry, Completeness) {
        let kind = base.dimension_type;
        (
            SemanticGeometry::Opaque {
                type_key: "AcDbDimension".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec![reason, format!("dimension subtype {kind:?}")]),
        )
    }

    /// Convert a dimension without a persisted anonymous block into drawable
    /// geometry. The 2D synthesis assumes the world-XY plane; a tilted plane is
    /// reported `Partial` instead of being flattened incorrectly.
    pub(crate) fn dimension_semantics(&self, d: &Dimension) -> (SemanticGeometry, Completeness) {
        let base = d.base();
        let normal = p3(base.normal);
        if !cad_geometry::is_finite(normal) || (normal.z.abs() - 1.0).abs() > 1e-6 {
            return Self::unsupported_dimension(
                base,
                "dimension lies off the world-XY plane; synthesis only handles XY".into(),
            );
        }
        let style = self.dim_style_values(&base.style_name);
        let geometry = match d {
            Dimension::Aligned(a) => {
                let first = p3(a.first_point);
                let second = p3(a.second_point);
                let direction = cad_geometry::normalize(cad_geometry::sub(second, first));
                linear_geometry(first, second, p3(a.definition_point), direction, &style)
            }
            Dimension::Linear(l) => {
                let first = p3(l.first_point);
                let second = p3(l.second_point);
                let direction = Point3 {
                    x: l.rotation.cos(),
                    y: l.rotation.sin(),
                    z: 0.0,
                };
                linear_geometry(first, second, p3(l.definition_point), direction, &style)
            }
            Dimension::Radius(r) => {
                radial_geometry(p3(r.angle_vertex), p3(r.definition_point), false, &style)
            }
            Dimension::Diameter(dd) => {
                radial_geometry(p3(dd.angle_vertex), p3(dd.definition_point), true, &style)
            }
            Dimension::Angular2Ln(a) => angular_geometry(
                p3(a.angle_vertex),
                p3(a.first_point),
                p3(a.definition_point),
                cad_geometry::distance(p3(a.angle_vertex), p3(a.dimension_arc)),
                p3(a.base.normal),
                &style,
            ),
            Dimension::Angular3Pt(a) => angular_geometry(
                p3(a.angle_vertex),
                p3(a.first_point),
                p3(a.second_point),
                cad_geometry::distance(p3(a.angle_vertex), p3(a.definition_point)),
                p3(a.base.normal),
                &style,
            ),
            Dimension::Ordinate(o) => ordinate_geometry(
                p3(o.feature_location),
                p3(o.leader_endpoint),
                o.is_ordinate_type_x,
            ),
            Dimension::Arc(a) => arc_length_geometry(
                p3(a.center_point),
                p3(a.first_extension_point),
                p3(a.second_extension_point),
                a.arc_start_parameter,
                a.arc_end_parameter,
                p3(a.base.normal),
                &style,
                a.has_leader
                    .then(|| (p3(a.first_leader_point), p3(a.second_leader_point))),
            ),
            Dimension::LargeRadial(lr) => {
                let override_center = p3(lr.override_center);
                let center = if cad_geometry::length(override_center) > 1e-9 {
                    override_center
                } else {
                    p3(lr.definition_point)
                };
                large_radial_geometry(center, p3(lr.chord_point), p3(lr.jog_point), lr.jog_angle)
            }
        };
        let Some(geometry) = geometry else {
            return Self::unsupported_dimension(base, "dimension points are degenerate".into());
        };
        let (prefix, suffix, completeness) = match d {
            Dimension::Radius(_) => ("R", "", Completeness::Complete),
            Dimension::Diameter(_) => ("\u{2300}", "", Completeness::Complete),
            Dimension::Angular2Ln(_) | Dimension::Angular3Pt(_) => {
                ("", "\u{00B0}", Completeness::Complete)
            }
            Dimension::LargeRadial(_) => (
                "R",
                "",
                Completeness::Partial(vec![
                    "large-radial jog is oriented by jog_angle; AutoCAD's true zigzag jog and text \
                     landing are not reproduced"
                        .into(),
                ]),
            ),
            Dimension::Arc(a) => (
                "",
                "",
                if a.is_partial {
                    Completeness::Partial(vec![
                        "partial arc dimension; partial-arc marker/leader arrangement not reproduced"
                            .into(),
                    ])
                } else {
                    Completeness::Complete
                },
            ),
            _ => ("", "", Completeness::Complete),
        };
        let text = self.place_dimension_text(base, &geometry, &style, prefix, suffix);
        let mut children = geometry.children;
        children.push(text);
        (SemanticGeometry::Compound(children), completeness)
    }

    /// Convert a LEADER into its polyline path plus the arrowhead at the first
    /// vertex (the arrow point). A spline path is drawn as straight segments and
    /// reported `Partial`; an enabled hookline is reported rather than guessed.
    pub(crate) fn leader_semantics(&self, l: &Leader) -> (SemanticGeometry, Completeness) {
        let points: Vec<Point3> = l.vertices.iter().map(|v| p3(*v)).collect();
        if points.len() < 2 || !points.iter().all(|p| cad_geometry::is_finite(*p)) {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["leader has fewer than two valid vertices".into()]),
            );
        }
        let normal = p3(l.normal);
        if !cad_geometry::is_finite(normal) || (normal.z.abs() - 1.0).abs() > 1e-6 {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec![
                    "leader lies off the world-XY plane; synthesis only handles XY".into(),
                ]),
            );
        }
        let mut completeness = Completeness::Complete;
        if l.path_type == LeaderPathType::Spline {
            completeness =
                Completeness::Partial(vec!["leader spline path drawn as straight segments".into()]);
        }
        if l.hookline_enabled {
            completeness = completeness.combine(Completeness::Partial(vec![
                "leader hookline is not drawn".into(),
            ]));
        }
        let mut children = vec![SemanticGeometry::Polyline {
            points: points.clone(),
            bulges: Vec::new(),
            closed: false,
        }];
        if l.arrow_enabled {
            let style = self.dim_style_values(&l.dimension_style);
            let outward = cad_geometry::sub(points[0], points[1]);
            if style.arrow_size > 1e-9 && cad_geometry::length(outward) > 1e-9 {
                children.push(arrowhead(points[0], outward, style.arrow_size));
            }
        }
        (SemanticGeometry::Compound(children), completeness)
    }
}
