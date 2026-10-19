//! Dimension family: subtype, DXF-equivalent definition points, the resolved
//! style snapshot that drives display, and the derived display geometry.
//!
//! This module is the **single** builder of a dimension's display geometry
//! (`display = f(definition, style)`). Both the creation path and the block-less
//! import path go through [`DimensionGeometry::synthesized`], so a caller cannot
//! hand the entity a display that disagrees with its definition (ADR 0017 A.4
//! invariant 1). A dimension whose source carries a pre-rendered anonymous block
//! uses [`DimensionGeometry::from_picture`], whose display is the external block
//! picture and therefore is not our synthesis.
//!
//! The synthesis lives in `cad-domain` (not the importer) so creation and import
//! share one builder; it was moved verbatim from the former
//! `cad-import-acadrust::dimension` module, and the numbers it produces are
//! unchanged. `cad-domain` is the lowest layer, so the small amount of vector
//! algebra it needs is implemented locally rather than pulled from
//! `cad-geometry` (which depends on this crate).

use crate::dimension_synthesis::{
    compute_measurement, is_finite_point, kind_affix, length, normalize, resolve_measurement_text,
    synthesize, SynthesizedDimension,
};
use crate::{CadError, CadResult, Mesh, Point3, SemanticGeometry, StyleId, Transform3};

// ---------------------------------------------------------------------------
// Subtype and definition points (ADR 0017 A.3).
// ---------------------------------------------------------------------------

/// Which dimension subtype this entity is, with the kind-specific scalars the
/// DXF record carries (Linear's group 50 rotation, Ordinate's X/Y datum flag).
///
/// The subtype set is exactly ADR 0017 A.1/A.3's table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimensionKind {
    /// Rotated/horizontal/vertical linear dimension (DXF type 0); group 50.
    Linear { rotation: f64 },
    /// Aligned dimension (DXF type 1).
    Aligned,
    /// Angular dimension between two lines (DXF type 2).
    Angular2Ln,
    /// Angular dimension through three points (DXF type 5).
    Angular3Pt,
    /// Radius dimension (DXF type 4).
    Radius,
    /// Diameter dimension (DXF type 3).
    Diameter,
    /// Ordinate dimension (DXF type 6); `x_ordinate` mirrors group 70 bit 0x40.
    Ordinate { x_ordinate: bool },
}

/// The DXF-equivalent definition points, grouped by the subtype that consumes
/// them (ADR 0017 A.3). Every variant's `text_middle` (group 11) and
/// `text_rotation` (group 53) live on [`DimensionDefinition`] because every
/// subtype carries them.
#[derive(Debug, Clone, PartialEq)]
pub enum DimensionPoints {
    /// Groups `13`/`14` (extension-line origins) and `10` (dimension line).
    Linear {
        first: Point3,
        second: Point3,
        dimension_line: Point3,
    },
    /// Groups `15` (point on the circle/arc), `10` (centre for Radius or the
    /// opposite chord point for Diameter) and `40` (leader length).
    Radial {
        center: Point3,
        point_on_arc: Point3,
        leader_length: f64,
    },
    /// Two lines: `13`->`14` and `15`->`10`; `16` is the dimension arc location.
    Angular2Ln {
        line1_start: Point3,
        line1_end: Point3,
        line2_start: Point3,
        line2_end: Point3,
        arc_point: Point3,
    },
    /// `15` vertex, `13`/`14` extension endpoints, `10` dimension arc location.
    Angular3Pt {
        vertex: Point3,
        first: Point3,
        second: Point3,
        arc_point: Point3,
    },
    /// `13` feature point, `14` leader endpoint, `10` creation datum.
    Ordinate {
        feature: Point3,
        leader_end: Point3,
        base: Point3,
    },
}

/// A dimension's subtype plus its DXF-equivalent definition points and the two
/// common text fields (group 11 middle point, group 53 rotation).
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionDefinition {
    pub kind: DimensionKind,
    pub points: DimensionPoints,
    /// Group 11: text middle point.
    pub text_middle: Point3,
    /// Group 53: explicit text rotation in radians (`0.0` = derive from the
    /// dimension line).
    pub text_rotation: f64,
    /// The dimension plane's extrusion normal (DXF group 210).
    ///
    /// The importer carries the source `base.normal` so an angular dimension's
    /// arc keeps the winding the pre-P5a synthesis gave it (a `-Z` extrusion is
    /// not flattened onto `+Z`). The export writes the world-Z normal and reports
    /// a drop when this differs (ADR 0017 A.3 group 210; review N1).
    pub normal: Point3,
}

impl DimensionDefinition {
    /// Whether the subtype and the point slot agree, and every point is finite.
    pub fn is_consistent(&self) -> bool {
        let kind_matches = matches!(
            (self.kind, &self.points),
            (
                DimensionKind::Linear { .. } | DimensionKind::Aligned,
                DimensionPoints::Linear { .. }
            ) | (
                DimensionKind::Radius | DimensionKind::Diameter,
                DimensionPoints::Radial { .. }
            ) | (
                DimensionKind::Angular2Ln,
                DimensionPoints::Angular2Ln { .. }
            ) | (
                DimensionKind::Angular3Pt,
                DimensionPoints::Angular3Pt { .. }
            ) | (
                DimensionKind::Ordinate { .. },
                DimensionPoints::Ordinate { .. }
            )
        );
        kind_matches
            && self.points_are_finite()
            && is_finite_point(self.text_middle)
            && is_finite_point(self.normal)
    }

    fn points_are_finite(&self) -> bool {
        let all = |points: &[Point3]| points.iter().all(|p| is_finite_point(*p));
        match &self.points {
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            } => all(&[*first, *second, *dimension_line]),
            DimensionPoints::Radial {
                center,
                point_on_arc,
                ..
            } => all(&[*center, *point_on_arc]),
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            } => all(&[
                *line1_start,
                *line1_end,
                *line2_start,
                *line2_end,
                *arc_point,
            ]),
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            } => all(&[*vertex, *first, *second, *arc_point]),
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base,
            } => all(&[*feature, *leader_end, *base]),
        }
    }

    /// Transform every definition point by a similarity and add the rotation
    /// delta to the kind's group-50 rotation and the group-53 text rotation.
    ///
    /// The caller has already checked the transform is a rotation + uniform
    /// scale (no mirror); a non-uniform or mirrored transform is refused before
    /// this is reached so no distorted definition points are produced.
    fn transformed(&self, transform: &Transform3, delta: f64) -> Self {
        let p = |point: Point3| transform.apply_point(point);
        let points = match &self.points {
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            } => DimensionPoints::Linear {
                first: p(*first),
                second: p(*second),
                dimension_line: p(*dimension_line),
            },
            DimensionPoints::Radial {
                center,
                point_on_arc,
                leader_length,
            } => DimensionPoints::Radial {
                center: p(*center),
                point_on_arc: p(*point_on_arc),
                leader_length: *leader_length,
            },
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            } => DimensionPoints::Angular2Ln {
                line1_start: p(*line1_start),
                line1_end: p(*line1_end),
                line2_start: p(*line2_start),
                line2_end: p(*line2_end),
                arc_point: p(*arc_point),
            },
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            } => DimensionPoints::Angular3Pt {
                vertex: p(*vertex),
                first: p(*first),
                second: p(*second),
                arc_point: p(*arc_point),
            },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base,
            } => DimensionPoints::Ordinate {
                feature: p(*feature),
                leader_end: p(*leader_end),
                base: p(*base),
            },
        };
        let kind = match self.kind {
            DimensionKind::Linear { rotation } => DimensionKind::Linear {
                rotation: rotation + delta,
            },
            other => other,
        };
        // Group 11 and group 53 use zero as a "not set / derive" sentinel; a
        // transform must preserve that meaning rather than turn an unset field
        // into an explicit one.
        let text_middle = if length(self.text_middle) < 1e-9 {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }
        } else {
            p(self.text_middle)
        };
        let text_rotation = if self.text_rotation.abs() < 1e-12 {
            0.0
        } else {
            self.text_rotation + delta
        };
        DimensionDefinition {
            kind,
            points,
            text_middle,
            text_rotation,
            normal: mapped_normal(self.normal, transform),
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved style snapshot (ADR 0017 B.2(a) — the display-driving subset).
// ---------------------------------------------------------------------------

/// The resolved DIMSTYLE values a synthesized dimension consumes, already scaled
/// by `DIMSCALE` where AutoCAD scales them (ADR 0017 B.2(a)).
///
/// This is a **snapshot**: editing the DIMSTYLE table does not retroactively
/// update an existing dimension (ADR 0017 B.1). The importer resolves the source
/// `DimStyle` into this struct; the creation path builds it from the active
/// style. Only the variables the synthesis actually consumes are carried — the
/// remaining B.2(a) variables and the whole B.2(b) "snapshot only" set are the
/// DIMSTYLE stage's concern (P5b).
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionStyleValues {
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
    /// `DIMTAD`: place text above (`true`) rather than centred on the line.
    pub text_above: bool,
    /// `DIMLFAC`: linear measurement scale factor.
    pub linear_factor: f64,
    /// `DIMDEC`: decimal places for the measurement.
    pub decimals: usize,
    /// `DIMZIN` bit 3: suppress trailing zeros in the measurement.
    pub suppress_trailing_zeros: bool,
    /// Resolved `DIMTXSTY` name (DXF group 340 reference), kept so the DIMSTYLE
    /// table can be written without a dangling text-style reference.
    pub text_style_name: String,
    /// The text style identity the measurement `Text` child carries.
    pub text_style: StyleId,
    /// The font resource key the measurement `Text` child carries, when known.
    pub text_font: Option<String>,
}

// ---------------------------------------------------------------------------
// The entity type.
// ---------------------------------------------------------------------------

/// The display provenance of a dimension: whether `display` is our synthesis or
/// an external picture block's expanded members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplaySource {
    Synthesized,
    Picture,
}

/// A dimension entity: subtype, definition points, resolved style snapshot,
/// measurement and derived display geometry.
///
/// All fields are private; the only writers are [`DimensionGeometry::synthesized`],
/// [`DimensionGeometry::from_picture`] and [`DimensionGeometry::transformed`],
/// so a caller cannot store a display that disagrees with the definition (ADR
/// 0017 A.1/A.4).
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionGeometry {
    definition: DimensionDefinition,
    style_name: String,
    style: DimensionStyleValues,
    measurement: f64,
    text_override: Option<String>,
    display: Vec<SemanticGeometry>,
    source: DisplaySource,
}

impl DimensionGeometry {
    /// Build a dimension whose display is our synthesis of `definition` under
    /// `style` (ADR 0017 A.4 invariant 1).
    pub fn synthesized(
        definition: DimensionDefinition,
        style_name: String,
        style: DimensionStyleValues,
        text_override: Option<String>,
    ) -> CadResult<Self> {
        let synthesized = synthesize(&definition, &style)?;
        let (prefix, suffix) = kind_affix(definition.kind);
        let text = place_text(
            &definition,
            &synthesized,
            &style,
            text_override.as_deref(),
            prefix,
            suffix,
        );
        let measurement = synthesized.measurement;
        let mut display = synthesized.children;
        display.push(text);
        Ok(DimensionGeometry {
            definition,
            style_name,
            style,
            measurement,
            text_override,
            display,
            source: DisplaySource::Synthesized,
        })
    }

    /// Build a dimension whose display is an external picture block's expanded
    /// members (ADR 0017 A.6, the anonymous-block branch). The measurement is
    /// still computed from `definition`; the display is not our synthesis, so
    /// invariant 1 does not apply to it.
    pub fn from_picture(
        definition: DimensionDefinition,
        style_name: String,
        style: DimensionStyleValues,
        text_override: Option<String>,
        members: Vec<SemanticGeometry>,
    ) -> CadResult<Self> {
        let measurement = compute_measurement(&definition, &style)?;
        Ok(DimensionGeometry {
            definition,
            style_name,
            style,
            measurement,
            text_override,
            display: members,
            source: DisplaySource::Picture,
        })
    }

    /// Transform definition points, display children (including `Text`) and the
    /// measurement together (ADR 0017 A.5).
    ///
    /// Refuses the whole entity (never a partial transform) when the transform
    /// is not a rotation + uniform scale (mirror/non-uniform scale), or when a
    /// picture-branch child cannot be transformed (`Spline`/`Shape`/`Opaque`).
    pub fn transformed(&self, transform: &Transform3) -> CadResult<Self> {
        if !transform.is_uniform_scale(1e-9) || transform.determinant() <= 0.0 {
            return Err(CadError::Unsupported(
                "dimension transform requires a rotation and uniform scale; mirror or \
                 non-uniform scale cannot be represented"
                    .to_string(),
            ));
        }
        let scale_factor = transform.max_scale();
        let delta = rotation_delta(transform);
        let definition = self.definition.transformed(transform, delta);
        // ADR 0017 A.5 (F-2) / review B1: the synthesized branch recomputes the
        // measurement from the transformed definition and re-derives the text so
        // group 42 and the drawn number stay in step. The picture branch freezes
        // both: the picture is the authoritative display, and a uniform scale is
        // the only case where they could diverge (these measurements are rigid
        // under translation and rotation; the ordinate projection is
        // datum-relative and translation-invariant), so freezing keeps
        // `measurement == drawn number` rather than inventing a value the picture
        // does not show.
        let measurement = if self.source == DisplaySource::Synthesized {
            compute_measurement(&definition, &self.style)?
        } else {
            self.measurement
        };
        let (prefix, suffix) = kind_affix(definition.kind);
        let mut display = Vec::with_capacity(self.display.len());
        for child in &self.display {
            let mut child = transform_child(child, transform, scale_factor, delta)?;
            if self.source == DisplaySource::Synthesized {
                if let SemanticGeometry::Text { text, .. } = &mut child {
                    *text = resolve_measurement_text(
                        self.text_override.as_deref(),
                        prefix,
                        suffix,
                        measurement,
                        &self.style,
                    );
                }
            }
            display.push(child);
        }
        Ok(DimensionGeometry {
            definition,
            style_name: self.style_name.clone(),
            style: self.style.clone(),
            measurement,
            text_override: self.text_override.clone(),
            display,
            source: self.source,
        })
    }

    /// The subtype with its kind-specific scalars.
    pub fn kind(&self) -> DimensionKind {
        self.definition.kind
    }

    /// The DXF-equivalent definition points and common text fields.
    pub fn definition(&self) -> &DimensionDefinition {
        &self.definition
    }

    /// The derived display geometry (extension lines, dimension line, arrows,
    /// text).
    pub fn display(&self) -> &[SemanticGeometry] {
        &self.display
    }

    /// The raw measurement (before `DIMLFAC`/formatting), written to DXF group 42.
    pub fn measurement(&self) -> f64 {
        self.measurement
    }

    /// The dimension style name (DXF group 3).
    pub fn style_name(&self) -> &str {
        &self.style_name
    }

    /// The resolved style snapshot.
    pub fn style(&self) -> &DimensionStyleValues {
        &self.style
    }

    /// The user text override (DXF group 1), if any.
    pub fn text_override(&self) -> Option<&str> {
        self.text_override.as_deref()
    }
}

pub(crate) fn place_text(
    definition: &DimensionDefinition,
    synthesized: &SynthesizedDimension,
    style: &DimensionStyleValues,
    text_override: Option<&str>,
    prefix: &str,
    suffix: &str,
) -> SemanticGeometry {
    let explicit = definition.text_middle;
    let position = if length(explicit) > 1e-9 {
        explicit
    } else {
        synthesized.text_position
    };
    let rotation = if definition.text_rotation.abs() > 1e-12 {
        definition.text_rotation
    } else {
        synthesized.text_rotation
    };
    let content = resolve_measurement_text(
        text_override,
        prefix,
        suffix,
        synthesized.measurement,
        style,
    );
    SemanticGeometry::Text {
        text: content,
        position,
        style: style.text_style,
        height: style.text_height,
        rotation,
        font: style.text_font.clone(),
        h_align: crate::TextAlignH::Center,
        v_align: crate::TextAlignV::Middle,
    }
}

/// The rotation the transform applies to the XY plane, in radians.
fn rotation_delta(transform: &Transform3) -> f64 {
    let v = transform.apply_vector(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    v.y.atan2(v.x)
}

/// The plane normal a similarity maps `normal` onto.
fn mapped_normal(normal: Point3, transform: &Transform3) -> Point3 {
    normalize(transform.apply_vector(normal))
}

/// Transform one display child. `Spline`/`Shape`/`Opaque` and other kinds this
/// build cannot place under a similarity are refused, so the whole dimension is
/// refused rather than partially moved.
fn transform_child(
    child: &SemanticGeometry,
    transform: &Transform3,
    scale_factor: f64,
    delta: f64,
) -> CadResult<SemanticGeometry> {
    let p = |point: Point3| transform.apply_point(point);
    Ok(match child {
        SemanticGeometry::Line { start, end } => SemanticGeometry::Line {
            start: p(*start),
            end: p(*end),
        },
        SemanticGeometry::Point(point) => SemanticGeometry::Point(p(*point)),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => SemanticGeometry::Polyline {
            points: points.iter().map(|point| p(*point)).collect(),
            bulges: bulges.clone(),
            closed: *closed,
        },
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => SemanticGeometry::Circle {
            center: p(*center),
            normal: mapped_normal(*normal, transform),
            radius: radius.abs() * scale_factor,
        },
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => SemanticGeometry::Arc {
            center: p(*center),
            normal: mapped_normal(*normal, transform),
            radius: radius.abs() * scale_factor,
            start: start + delta,
            sweep: *sweep,
        },
        SemanticGeometry::Mesh(mesh) => {
            let normals = if mesh.normals.is_empty() {
                Vec::new()
            } else {
                mesh.normals
                    .iter()
                    .map(|n| mapped_normal(*n, transform))
                    .collect()
            };
            SemanticGeometry::Mesh(Mesh {
                vertices: mesh.vertices.iter().map(|point| p(*point)).collect(),
                triangles: mesh.triangles.clone(),
                normals,
                face_sources: mesh.face_sources.clone(),
                colors: mesh.colors.clone(),
            })
        }
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
        } => SemanticGeometry::Text {
            text: text.clone(),
            position: p(*position),
            style: *style,
            height: height * scale_factor,
            rotation: rotation + delta,
            font: font.clone(),
            h_align: *h_align,
            v_align: *v_align,
        },
        other => {
            return Err(CadError::Unsupported(format!(
                "dimension display child {} cannot be transformed",
                child_kind(other)
            )));
        }
    })
}

fn child_kind(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "Line",
        SemanticGeometry::Polyline { .. } => "Polyline",
        SemanticGeometry::Circle { .. } => "Circle",
        SemanticGeometry::Arc { .. } => "Arc",
        SemanticGeometry::Ellipse { .. } => "Ellipse",
        SemanticGeometry::Spline { .. } => "Spline",
        SemanticGeometry::Point(_) => "Point",
        SemanticGeometry::Mesh(_) => "Mesh",
        SemanticGeometry::Insert { .. } => "Insert",
        SemanticGeometry::Text { .. } => "Text",
        SemanticGeometry::Shape { .. } => "Shape",
        SemanticGeometry::Opaque { .. } => "Opaque",
        SemanticGeometry::Compound(_) => "Compound",
        SemanticGeometry::Image { .. } => "Image",
        SemanticGeometry::Mask { .. } => "Mask",
        SemanticGeometry::Dimension(_) => "Dimension",
    }
}
