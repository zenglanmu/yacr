//! Dimension display geometry synthesis.
//!
//! A DXF/DWG DIMENSION may carry a pre-rendered anonymous block (`*D...`); when
//! it does, the importer expands it as an INSERT. Many producers (including the
//! committed QCAD `flange` sample) store only the definition points and omit
//! the block. Without synthesis those dimensions have no display geometry and
//! are reported as unsupported.
//!
//! This module builds the standard linear/aligned and radius/diameter geometry
//! from the dimension points and the resolved DIMSTYLE: extension lines, the
//! dimension line, solid arrowheads and the measurement text. Subtypes that are
//! not synthesized (angular, ordinate, arc-length) stay explicit `Partial`
//! rather than being drawn as a guess. Nothing here claims compatibility: it is
//! a display representation for geometry that is already in the database.

use super::*;
use acadrust::entities::{Dimension, DimensionBase, Leader, LeaderPathType};
use acadrust::tables::DimStyle;

/// DIMSTYLE values needed to draw a synthesized dimension, already scaled by
/// `DIMSCALE` where AutoCAD scales them.
#[derive(Debug, Clone)]
pub(crate) struct DimStyleValues {
    /// `DIMASZ`: arrowhead length in drawing units.
    pub arrow_size: f64,
    /// `DIMEXO`: gap between the measured point and the extension line.
    pub ext_offset: f64,
    /// `DIMEXE`: how far the extension line runs past the dimension line.
    pub ext_beyond: f64,
    /// `DIMTXT`: measurement text height.
    pub text_height: f64,
    /// `DIMGAP`: gap between the dimension line and the measurement text.
    pub text_gap: f64,
    /// `DIMTAD`: place text above (true) rather than centred on the line.
    pub text_above: bool,
    /// `DIMLFAC`: linear measurement scale factor.
    pub linear_factor: f64,
    /// `DIMDEC`: decimal places for the measurement.
    pub decimals: usize,
    /// `DIMZIN` bit 3: suppress trailing zeros in the measurement.
    pub suppress_trailing_zeros: bool,
    /// Resolved `DIMTXSTY` name, used to pick the text font.
    pub text_style: String,
}

fn positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

impl DimStyleValues {
    pub(crate) fn from_style(style: &DimStyle, text_style: String) -> Self {
        let scale = positive(style.dimscale, 1.0);
        Self {
            arrow_size: positive(style.dimasz, 2.5) * scale,
            ext_offset: positive(style.dimexo, 0.625) * scale,
            ext_beyond: positive(style.dimexe, 1.25) * scale,
            text_height: positive(style.dimtxt, 2.5) * scale,
            text_gap: positive(style.dimgap, 0.625) * scale,
            text_above: style.dimtad != 0,
            linear_factor: {
                let factor = style.dimlfac;
                if factor.is_finite() && factor.abs() > 1e-12 {
                    factor.abs()
                } else {
                    1.0
                }
            },
            decimals: style.dimdec.clamp(0, 8) as usize,
            suppress_trailing_zeros: style.dimzin & 8 != 0,
            text_style,
        }
    }
}

/// Geometry derived from one dimension, before the text primitive is attached.
#[derive(Debug)]
struct DimensionGeometry {
    children: Vec<SemanticGeometry>,
    measurement: f64,
    /// Default text anchor when the entity has no explicit middle point.
    text_position: Point3,
    /// Default text rotation when the entity has no explicit rotation.
    text_rotation: f64,
}

fn perpendicular(v: Point3) -> Point3 {
    Point3 {
        x: -v.y,
        y: v.x,
        z: 0.0,
    }
}

fn project_onto_line(origin: Point3, direction: Point3, point: Point3) -> Point3 {
    let t = cad_geometry::dot(cad_geometry::sub(point, origin), direction);
    cad_geometry::add(origin, cad_geometry::scale(direction, t))
}

fn line_between(start: Point3, end: Point3) -> Option<SemanticGeometry> {
    (cad_geometry::distance(start, end) > 1e-9).then_some(SemanticGeometry::Line { start, end })
}

/// A solid arrowhead: `tip` is the point, `outward` the direction it points.
fn arrowhead(tip: Point3, outward: Point3, size: f64) -> SemanticGeometry {
    let outward = cad_geometry::normalize(outward);
    let base_center = cad_geometry::sub(tip, cad_geometry::scale(outward, size));
    let side = cad_geometry::scale(perpendicular(outward), size / 6.0);
    let corner_a = cad_geometry::add(base_center, side);
    let corner_b = cad_geometry::sub(base_center, side);
    SemanticGeometry::Mesh(Mesh {
        vertices: vec![tip, corner_a, corner_b],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    })
}

/// Rotate a text angle into the readable range `(-90°, 90°]`.
fn readable_angle(mut angle: f64) -> f64 {
    use std::f64::consts::PI;
    while angle > PI / 2.0 {
        angle -= PI;
    }
    while angle <= -PI / 2.0 {
        angle += PI;
    }
    angle
}

/// Build the linear/aligned dimension geometry (no text).
fn linear_geometry(
    first: Point3,
    second: Point3,
    definition: Point3,
    direction: Point3,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(first)
        || !cad_geometry::is_finite(second)
        || !cad_geometry::is_finite(definition)
        || !cad_geometry::is_finite(direction)
        || cad_geometry::length(direction) < 1e-9
    {
        return None;
    }
    let direction = cad_geometry::normalize(direction);
    let normal = perpendicular(direction);
    let q1 = project_onto_line(definition, direction, first);
    let q2 = project_onto_line(definition, direction, second);
    let mut children = Vec::new();
    for (point, foot) in [(first, q1), (second, q2)] {
        let v = cad_geometry::sub(foot, point);
        let len = cad_geometry::length(v);
        if len > 1e-9 {
            let u = cad_geometry::scale(v, 1.0 / len);
            let start = cad_geometry::add(point, cad_geometry::scale(u, style.ext_offset));
            let end = cad_geometry::add(foot, cad_geometry::scale(u, style.ext_beyond));
            if let Some(line) = line_between(start, end) {
                children.push(line);
            }
        }
    }
    if let Some(line) = line_between(q1, q2) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(
            q1,
            cad_geometry::scale(direction, -1.0),
            style.arrow_size,
        ));
        children.push(arrowhead(q2, direction, style.arrow_size));
    }
    // Text sits on the side the dimension line is offset toward, `DIMGAP` plus
    // half the text height away from the line (matches QCAD/AutoCAD default).
    let mid = cad_geometry::scale(cad_geometry::add(q1, q2), 0.5);
    let measured_mid = cad_geometry::scale(cad_geometry::add(first, second), 0.5);
    let mut side = cad_geometry::dot(cad_geometry::sub(measured_mid, mid), normal);
    if side.abs() < 1e-9 {
        side = 1.0;
    } else {
        side = side.signum();
    }
    let offset = if style.text_above {
        style.text_gap + style.text_height * 0.5
    } else {
        0.0
    };
    let text_position = cad_geometry::add(mid, cad_geometry::scale(normal, side * offset));
    let text_rotation = readable_angle(direction.y.atan2(direction.x));
    let measurement = cad_geometry::dot(cad_geometry::sub(second, first), direction).abs();
    Some(DimensionGeometry {
        children,
        measurement,
        text_position,
        text_rotation,
    })
}

/// Build a radius/diameter leader with arrow(s) (no text).
fn radial_geometry(
    from: Point3,
    to: Point3,
    two_arrows: bool,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(from) || !cad_geometry::is_finite(to) {
        return None;
    }
    let v = cad_geometry::sub(to, from);
    let len = cad_geometry::length(v);
    if len < 1e-9 {
        return None;
    }
    let u = cad_geometry::scale(v, 1.0 / len);
    let mut children = Vec::new();
    if let Some(line) = line_between(from, to) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(to, u, style.arrow_size));
        if two_arrows {
            children.push(arrowhead(
                from,
                cad_geometry::scale(u, -1.0),
                style.arrow_size,
            ));
        }
    }
    Some(DimensionGeometry {
        children,
        measurement: len,
        text_position: cad_geometry::scale(cad_geometry::add(from, to), 0.5),
        text_rotation: readable_angle(u.y.atan2(u.x)),
    })
}

fn format_measurement(value: f64, style: &DimStyleValues) -> String {
    let mut text = format!("{:.*}", style.decimals, value.abs());
    if style.suppress_trailing_zeros && text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

/// Resolve the displayed text: a user override with `<>` substituted, or the
/// subtype's default prefix/suffix plus the formatted measurement.
fn resolve_measurement_text(
    base: &DimensionBase,
    prefix: &str,
    suffix: &str,
    measurement: f64,
    style: &DimStyleValues,
) -> String {
    let value = format_measurement(measurement * style.linear_factor, style);
    match base.text_override() {
        Some(template) => {
            if template.contains("<>") {
                template.replace("<>", &value)
            } else {
                template.to_string()
            }
        }
        None => format!("{prefix}{value}{suffix}"),
    }
}

/// Angular dimension: an arc between two rays from `vertex`, with extension
/// lines, tangential arrows and the angle text.
fn angular_geometry(
    vertex: Point3,
    point1: Point3,
    point2: Point3,
    radius: f64,
    normal: Point3,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    let d1 = cad_geometry::sub(point1, vertex);
    let d2 = cad_geometry::sub(point2, vertex);
    if cad_geometry::length(d1) < 1e-9 || cad_geometry::length(d2) < 1e-9 {
        return None;
    }
    let u1 = cad_geometry::normalize(d1);
    let u2 = cad_geometry::normalize(d2);
    let a1 = u1.y.atan2(u1.x);
    let a2 = u2.y.atan2(u2.x);
    let mut sweep = a2 - a1;
    use std::f64::consts::PI;
    while sweep > PI {
        sweep -= 2.0 * PI;
    }
    while sweep < -PI {
        sweep += 2.0 * PI;
    }
    let r = if radius.is_finite() && radius > 1e-9 {
        radius
    } else {
        1.0
    };
    let dir_at = |angle: f64| Point3 {
        x: angle.cos(),
        y: angle.sin(),
        z: 0.0,
    };
    let end1 = cad_geometry::add(vertex, cad_geometry::scale(u1, r));
    let end2 = cad_geometry::add(vertex, cad_geometry::scale(dir_at(a1 + sweep), r));
    let mut children = vec![SemanticGeometry::Arc {
        center: vertex,
        normal,
        radius: r,
        start: a1,
        sweep,
    }];
    if let Some(line) = line_between(point1, end1) {
        children.push(line);
    }
    if let Some(line) = line_between(point2, end2) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        // Tangential arrowheads at both arc ends.
        let sign = if sweep >= 0.0 { -1.0 } else { 1.0 };
        let t1 = Point3 {
            x: -u1.y,
            y: u1.x,
            z: 0.0,
        };
        let t2 = Point3 {
            x: -dir_at(a1 + sweep).y,
            y: dir_at(a1 + sweep).x,
            z: 0.0,
        };
        children.push(arrowhead(
            end1,
            cad_geometry::scale(t1, sign),
            style.arrow_size,
        ));
        children.push(arrowhead(
            end2,
            cad_geometry::scale(t2, -sign),
            style.arrow_size,
        ));
    }
    let mid = dir_at(a1 + sweep / 2.0);
    let text_position = cad_geometry::add(
        vertex,
        cad_geometry::scale(mid, r + style.text_gap + style.text_height * 0.5),
    );
    Some(DimensionGeometry {
        children,
        measurement: sweep.abs().to_degrees(),
        text_position,
        text_rotation: readable_angle(a1 + sweep / 2.0),
    })
}

/// Ordinate dimension: a leader from the feature point to the dimension line,
/// with the feature coordinate as text.
fn ordinate_geometry(feature: Point3, leader: Point3, is_x: bool) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(feature) || !cad_geometry::is_finite(leader) {
        return None;
    }
    let mut children = Vec::new();
    if let Some(line) = line_between(feature, leader) {
        children.push(line);
    }
    Some(DimensionGeometry {
        children,
        measurement: if is_x { feature.x } else { feature.y },
        text_position: leader,
        text_rotation: 0.0,
    })
}

/// Arc-length dimension: the measured arc plus extension lines and arrows.
fn arc_length_geometry(
    center: Point3,
    first_extension: Point3,
    second_extension: Point3,
    start: f64,
    end: f64,
    normal: Point3,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    let radius = cad_geometry::distance(center, first_extension);
    if !cad_geometry::is_finite(center) || radius < 1e-9 {
        return None;
    }
    let sweep = normalize_sweep(end - start);
    let dir_at = |angle: f64| Point3 {
        x: angle.cos(),
        y: angle.sin(),
        z: 0.0,
    };
    let mut children = vec![SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    }];
    let end1 = cad_geometry::add(center, cad_geometry::scale(dir_at(start), radius));
    let end2 = cad_geometry::add(center, cad_geometry::scale(dir_at(start + sweep), radius));
    for (point, foot) in [(first_extension, end1), (second_extension, end2)] {
        if let Some(line) = line_between(point, foot) {
            children.push(line);
        }
    }
    if style.arrow_size > 1e-9 {
        let sign = if sweep >= 0.0 { -1.0 } else { 1.0 };
        let t1 = Point3 {
            x: -dir_at(start).y,
            y: dir_at(start).x,
            z: 0.0,
        };
        let t2 = Point3 {
            x: -dir_at(start + sweep).y,
            y: dir_at(start + sweep).x,
            z: 0.0,
        };
        children.push(arrowhead(
            end1,
            cad_geometry::scale(t1, sign),
            style.arrow_size,
        ));
        children.push(arrowhead(
            end2,
            cad_geometry::scale(t2, -sign),
            style.arrow_size,
        ));
    }
    let mid = dir_at(start + sweep / 2.0);
    Some(DimensionGeometry {
        children,
        measurement: radius * sweep.abs(),
        text_position: cad_geometry::add(
            center,
            cad_geometry::scale(mid, radius + style.text_gap + style.text_height * 0.5),
        ),
        text_rotation: readable_angle(start + sweep / 2.0),
    })
}

/// Jogged / large-radius radial dimension: the leader with its jog, plus the
/// chord arrow. The jog geometry is approximate and reported `Partial`.
fn large_radial_geometry(center: Point3, chord: Point3, jog: Point3) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(center)
        || !cad_geometry::is_finite(chord)
        || !cad_geometry::is_finite(jog)
    {
        return None;
    }
    let mut children = Vec::new();
    if let Some(line) = line_between(jog, chord) {
        children.push(line);
    }
    if let Some(line) = line_between(center, jog) {
        children.push(line);
    }
    Some(DimensionGeometry {
        children,
        measurement: cad_geometry::distance(center, chord),
        text_position: jog,
        text_rotation: 0.0,
    })
}

impl ImporterBuilder<'_> {
    /// Resolve the DIMSTYLE for a dimension, falling back to the acadrust
    /// defaults when the table entry is missing.
    fn dim_style_values(&self, name: &str) -> DimStyleValues {
        let text_style = self.dim_text_style(name);
        match self.acad.dim_styles.get(name) {
            Some(style) => DimStyleValues::from_style(style, text_style),
            None => {
                let standard = DimStyle::standard();
                DimStyleValues::from_style(&standard, text_style)
            }
        }
    }

    /// The TEXTSTYLE table name a handle points at, if any.
    pub(crate) fn text_style_name_for_handle(&self, handle: acadrust::Handle) -> Option<String> {
        if handle.is_null() {
            return None;
        }
        self.acad
            .text_styles
            .iter()
            .find(|style| style.handle.value() == handle.value())
            .map(|style| style.name.clone())
    }

    /// The text-style name a dimension style points at.
    ///
    /// The DXF reader fills `dimtxsty_handle` (group 340) but may leave the name
    /// at its default; resolve the handle against the TEXTSTYLE table so the
    /// measurement text uses the right font instead of the Standard SHX style.
    pub(crate) fn dim_text_style(&self, name: &str) -> String {
        let Some(style) = self.acad.dim_styles.get(name) else {
            return "Standard".to_string();
        };
        if let Some(resolved) = self.text_style_name_for_handle(style.dimtxsty_handle) {
            return resolved;
        }
        if !style.dimtxsty.is_empty() {
            return style.dimtxsty.clone();
        }
        "Standard".to_string()
    }

    /// Convert a MULTILEADER: its leader-root polylines plus the annotation
    /// text. Block-content multileaders are reported `Partial` because the
    /// content block is not expanded here.
    pub(crate) fn multileader_semantics(
        &self,
        ml: &acadrust::entities::MultiLeader,
    ) -> (SemanticGeometry, Completeness) {
        let context = &ml.context;
        let mut children: Vec<SemanticGeometry> = Vec::new();
        for root in &context.leader_roots {
            for line in &root.lines {
                if line.points.len() >= 2 {
                    children.push(polyline_semantics(
                        line.points.iter().map(|p| p3(*p)).collect(),
                        false,
                    ));
                }
            }
        }
        let mut completeness = Completeness::Complete;
        if context.has_text_contents && !context.text_string.is_empty() {
            let style_name = context
                .text_style_handle
                .and_then(|handle| self.text_style_name_for_handle(handle))
                .unwrap_or_else(|| "Standard".to_string());
            let height = if context.text_height.is_finite() && context.text_height > 0.0 {
                context.text_height
            } else {
                ml.text_height
            };
            children.push(SemanticGeometry::Text {
                text: context.text_string.clone(),
                position: p3(context.text_location),
                style: self.style_id(&style_name),
                height,
                rotation: context.text_rotation,
                font: self.style_font(&style_name),
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Middle,
            });
        }
        if context.has_block_contents {
            completeness = completeness.combine(Completeness::Partial(vec![
                "mleader block content is not expanded".into(),
            ]));
        }
        if children.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbMultiLeader".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["multileader has no leader lines or text".into()]),
            );
        }
        (SemanticGeometry::Compound(children), completeness)
    }

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
            ),
            Dimension::LargeRadial(lr) => {
                let override_center = p3(lr.override_center);
                let center = if cad_geometry::length(override_center) > 1e-9 {
                    override_center
                } else {
                    p3(lr.definition_point)
                };
                large_radial_geometry(center, p3(lr.chord_point), p3(lr.jog_point))
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
                Completeness::Partial(vec!["large-radial jog is approximated".into()]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> DimStyleValues {
        DimStyleValues::from_style(&DimStyle::standard(), "Standard".to_string())
    }

    fn line_ends(children: &[SemanticGeometry]) -> Vec<(Point3, Point3)> {
        children
            .iter()
            .filter_map(|child| match child {
                SemanticGeometry::Line { start, end } => Some((*start, *end)),
                _ => None,
            })
            .collect()
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn approx_point(a: Point3, b: Point3) -> bool {
        approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
    }

    #[test]
    fn aligned_dimension_builds_extension_lines_dimension_line_and_arrows() {
        let first = Point3 {
            x: -21.0,
            y: 0.0,
            z: 0.0,
        };
        let second = Point3 {
            x: 21.0,
            y: 0.0,
            z: 0.0,
        };
        let definition = Point3 {
            x: -21.0,
            y: -30.0,
            z: 0.0,
        };
        // Use the QCAD flange's DIMTXT/DIMGAP so the placement matches.
        let mut style = style();
        style.text_height = 2.5;
        style.text_gap = 0.625;
        let geometry = linear_geometry(
            first,
            second,
            definition,
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            &style,
        )
        .expect("synthesized");
        assert!(approx(geometry.measurement, 42.0));
        // Two extension lines plus the dimension line.
        assert_eq!(line_ends(&geometry.children).len(), 3);
        // Two solid arrowheads.
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
        // Default text sits just above the dimension line, toward the geometry.
        let expected = Point3 {
            x: 0.0,
            y: -28.125,
            z: 0.0,
        };
        assert!(
            approx_point(geometry.text_position, expected),
            "{:?}",
            geometry.text_position
        );
        assert!(approx(geometry.text_rotation, 0.0));
    }

    #[test]
    fn vertical_linear_dimension_rotates_text() {
        let first = Point3 {
            x: 30.0,
            y: 3.0,
            z: 0.0,
        };
        let second = Point3 {
            x: 30.0,
            y: -3.0,
            z: 0.0,
        };
        let definition = Point3 {
            x: 45.0,
            y: 3.0,
            z: 0.0,
        };
        // A 90° linear dimension measures along +Y.
        let direction = Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let geometry =
            linear_geometry(first, second, definition, direction, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 6.0));
        assert!(
            approx(geometry.text_rotation, 90.0_f64.to_radians()),
            "{:?}",
            geometry.text_rotation
        );
    }

    #[test]
    fn radius_dimension_measures_the_leader() {
        let center = Point3 {
            x: 30.0,
            y: 0.0,
            z: 0.0,
        };
        let point = Point3 {
            x: 33.0,
            y: -4.0,
            z: 0.0,
        };
        let geometry = radial_geometry(center, point, false, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 5.0));
        // One leader line and one arrow.
        assert_eq!(line_ends(&geometry.children).len(), 1);
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            1
        );
    }

    #[test]
    fn diameter_dimension_has_two_arrows() {
        let a = Point3 {
            x: -13.64147673972468,
            y: -3.14803309378262,
            z: 0.0,
        };
        let b = Point3 {
            x: 13.64147673972468,
            y: 3.14803309378262,
            z: 0.0,
        };
        let geometry = radial_geometry(a, b, true, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 28.0));
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
    }

    #[test]
    fn measurement_text_suppresses_trailing_zeros_and_replaces_placeholder() {
        let mut style = style();
        style.decimals = 2;
        style.suppress_trailing_zeros = true;
        let base = DimensionBase::new(acadrust::entities::DimensionType::Aligned);
        assert_eq!(resolve_measurement_text(&base, "", "", 42.0, &style), "42");
        assert_eq!(resolve_measurement_text(&base, "R", "", 5.0, &style), "R5");
        assert_eq!(
            resolve_measurement_text(&base, "", "\u{00B0}", 45.0, &style),
            "45°"
        );
        let mut overriding = base.clone();
        overriding.set_text_override(Some("R<>".to_string()));
        assert_eq!(
            resolve_measurement_text(&overriding, "R", "", 5.0, &style),
            "R5"
        );
    }

    #[test]
    fn angular_dimension_measures_the_arc_and_draws_arrows() {
        let vertex = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let geometry = angular_geometry(
            vertex,
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
            5.0,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            &style(),
        )
        .expect("angular geometry");
        assert!(
            approx(geometry.measurement, 90.0),
            "{}",
            geometry.measurement
        );
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Arc { .. }))
                .count(),
            1
        );
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
    }

    #[test]
    fn ordinate_dimension_reports_the_feature_coordinate() {
        let geometry = ordinate_geometry(
            Point3 {
                x: 10.0,
                y: 3.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 8.0,
                z: 0.0,
            },
            false,
        )
        .expect("ordinate geometry");
        assert!(approx(geometry.measurement, 3.0));
    }

    #[test]
    fn arc_length_dimension_measures_the_arc() {
        let geometry = arc_length_geometry(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 5.0,
                z: 0.0,
            },
            0.0,
            std::f64::consts::FRAC_PI_2,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            &style(),
        )
        .expect("arc length geometry");
        assert!(approx(
            geometry.measurement,
            5.0 * std::f64::consts::FRAC_PI_2
        ));
    }
}
