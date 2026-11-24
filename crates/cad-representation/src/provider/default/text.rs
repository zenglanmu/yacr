//! The text/SHAPE half of the default provider.
//!
//! Split out of `default.rs` for the file-size ceiling (`check-file-size.py`).
//! Everything here turns one annotation into fragments: the annotative
//! placement of TEXT/MTEXT, the glyph shaping and its honest `Partial`/`Missing`
//! verdicts, the `QTEXT` placeholder rectangle, and one SHX SHAPE glyph. The
//! [`TextGeometry`] / [`ShapeGeometry`] structs are exactly the fields
//! `default.rs`'s dispatch already matched, so the arm bodies moved verbatim.

use crate::*;

use super::super::display_modes;
use super::TextScale;

/// The matched fields of one `SemanticGeometry::Text`, borrowed from the entity.
///
/// The field names are the dispatch pattern's bindings, so the body that moved
/// out of `default.rs` still reads exactly as it did there.
pub(super) struct TextGeometry<'a> {
    pub(super) text: &'a String,
    pub(super) position: &'a Point3,
    pub(super) style: &'a StyleId,
    pub(super) height: &'a f64,
    pub(super) rotation: &'a f64,
    pub(super) font: &'a Option<String>,
    pub(super) h_align: &'a TextAlignH,
    pub(super) v_align: &'a TextAlignV,
    pub(super) wrap_width: &'a Option<f64>,
    pub(super) width_factor: &'a f64,
    pub(super) line_spacing: &'a TextLineSpacing,
    pub(super) mirrored: &'a bool,
}

/// Build a TEXT/MTEXT fragment, applying the annotation-scale adjustment.
///
/// Shaping that only approximates the formatting is reported in the
/// completeness verdict and diagnostics; an unusable font reports a `Partial`
/// text and keeps the unshaped `Text` primitive rather than dropping the
/// annotation.
pub(super) fn text_fragment(
    entity: &DbEntity,
    context: &RepresentationContext,
    annotative: Option<TextScale>,
    geometry: TextGeometry<'_>,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let TextGeometry {
        text,
        position,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
        width_factor,
        line_spacing,
        mirrored,
    } = geometry;
    let font_key = font.as_deref();
    // Annotative TEXT/MTEXT: the active annotation scale factor
    // scales the glyph size about the anchor, and a per-scale
    // placement override (when imported) replaces the base
    // position/rotation. Non-annotative entities use the base
    // geometry unchanged.
    let (eff_position, eff_height, eff_rotation) = match annotative {
        Some(a) => (
            a.position.unwrap_or(*position),
            a.height.unwrap_or(height.abs() * a.factor),
            a.rotation.unwrap_or(*rotation),
        ),
        None => (*position, height.abs(), *rotation),
    };
    // The wrap boundary is a world-space length like the glyph
    // height, so the annotation scale factor scales it too; leaving
    // it fixed would re-flow the text at every annotation scale.
    let eff_wrap_width = match annotative {
        Some(a) => wrap_width.map(|width| width * a.factor),
        None => *wrap_width,
    };
    let mut shaped = false;
    // A style that names no font still shapes through the engine's
    // fallback chain: dropping the text because the drawing has no
    // font-bearing STYLE entry would silently hide every created
    // dimension's measurement.
    if let Some(engine) = &context.fonts {
        let font_label = font_key.unwrap_or("the text style's font");
        let result = match font_key {
            Some(key) => engine.shape(
                key,
                text,
                eff_position,
                eff_height,
                eff_rotation,
                *h_align,
                *v_align,
                eff_wrap_width,
                *width_factor,
                *line_spacing,
            ),
            None => engine.shape_unstyled(
                text,
                eff_position,
                eff_height,
                eff_rotation,
                *h_align,
                *v_align,
                eff_wrap_width,
                *width_factor,
                *line_spacing,
            ),
        };
        match result {
            Ok(shaped_text) => {
                // `mirrored` is the stored glyph-frame reflection (`MIRRTEXT=1`):
                // the glyphs are composed normally, then reflected across the
                // text's own vertical axis through the anchor. Doing it here needs
                // no second layout pass and keeps anchors/alignment untouched.
                let shaped_text = if *mirrored {
                    mirror_shaped_text(shaped_text, eff_position, eff_rotation)
                } else {
                    shaped_text
                };
                let has_renderable_content = shaped_text.has_renderable_content;
                for mesh in shaped_text.meshes {
                    representation.fragments.push(DisplayFragment {
                        source: source.clone(),
                        geometry_source: geometry_source.clone(),
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        draw_order: entity.draw_order,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(mesh)),
                    });
                }
                for polyline in shaped_text.polylines {
                    if polyline.len() >= 2 {
                        representation.fragments.push(DisplayFragment {
                            source: source.clone(),
                            geometry_source: geometry_source.clone(),
                            precision: Precision::Analytic,
                            alpha: 1.0,
                            draw_order: entity.draw_order,
                            color: DEFAULT_RENDER_COLOR,
                            color_unresolved: true,
                            lineweight: DEFAULT_LINEWEIGHT_MM,
                            lineweight_unresolved: true,
                            linetype: LinetypePattern::continuous(),
                            linetype_unresolved: true,
                            linetype_scale: 1.0,
                            primitive: DisplayPrimitive::Lines(Arc::from(
                                polyline.into_boxed_slice(),
                            )),
                        });
                    }
                }
                shaped = true;
                if representation.fragments.is_empty() && has_renderable_content {
                    representation.completeness =
                        Completeness::Missing(vec!["text produced no drawable outlines".into()]);
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "text.empty_outline".into(),
                        message: format!("font '{font_label}' produced no outline for the text"),
                    });
                }
                // MTEXT formatting that shaped only approximately
                // (color, decorations, stacked fractions, ...) is
                // reported in the completeness verdict and
                // diagnostics rather than silently ignored.
                if !shaped_text.issues.is_empty() {
                    let reasons: Vec<String> = shaped_text
                        .issues
                        .iter()
                        .map(|issue| issue.message.clone())
                        .collect();
                    representation.completeness = representation
                        .completeness
                        .clone()
                        .combine(Completeness::Partial(reasons));
                    for issue in &shaped_text.issues {
                        representation.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: issue.code.into(),
                            message: issue.message.clone(),
                        });
                    }
                }
            }
            Err(error) => {
                representation.completeness = Completeness::Partial(vec![format!(
                    "text font '{font_label}' unavailable: {error}"
                )]);
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "text.font_unavailable".into(),
                    message: error.to_string(),
                });
            }
        }
    }
    if !shaped {
        representation.fragments.push(DisplayFragment {
            source,
            geometry_source,
            precision: Precision::Analytic,
            alpha: 1.0,
            draw_order: entity.draw_order,
            color: DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Text {
                text: text.clone(),
                origin: eff_position,
                font: ResourceKey(
                    font_key
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("style:{}", style.0)),
                ),
                height: eff_height,
            },
        });
    }
    // `QTEXT`: replace the glyph geometry that was just built with
    // one placeholder rectangle derived from it. The text is shaped
    // either way — this build claims a display change, not a
    // redraw-speed improvement — and a text whose extent cannot be
    // derived keeps its own draw plus a `qtext.extent_unknown`
    // diagnostic (see `display_modes::apply_qtext`).
    if context.qtext {
        display_modes::apply_qtext(representation, entity.id.0);
    }
}

/// Reflect shaped text across its own vertical axis through the anchor.
///
/// This is the display half of `SemanticGeometry::Text.mirrored` (AutoCAD
/// `MIRRTEXT=1`). `rotation` is the stored text rotation; the mirror axis is the
/// local +Y direction (`v(rotation) = (-sin, cos)`), so the glyph run advances
/// the other way while the anchor and the stored rotation stay where the model
/// put them. Mesh winding is reversed because the reflection has determinant
/// `-1`; otherwise back-face culling would drop the fill.
fn mirror_shaped_text(mut shaped: ShapedText, origin: Point3, rotation: f64) -> ShapedText {
    let axis = Point3 {
        x: -rotation.sin(),
        y: rotation.cos(),
        z: 0.0,
    };
    let reflect_point = |point: Point3| {
        let dx = point.x - origin.x;
        let dy = point.y - origin.y;
        let k = 2.0 * (dx * axis.x + dy * axis.y);
        Point3 {
            x: origin.x + k * axis.x - dx,
            y: origin.y + k * axis.y - dy,
            z: point.z,
        }
    };
    let reflect_normal = |normal: Point3| {
        let k = 2.0 * (normal.x * axis.x + normal.y * axis.y);
        Point3 {
            x: k * axis.x - normal.x,
            y: k * axis.y - normal.y,
            z: normal.z,
        }
    };
    for polyline in shaped
        .polylines
        .iter_mut()
        .chain(shaped.outlines.iter_mut())
    {
        for point in polyline.iter_mut() {
            *point = reflect_point(*point);
        }
    }
    for mesh in &mut shaped.meshes {
        for point in &mut mesh.vertices {
            *point = reflect_point(*point);
        }
        for normal in &mut mesh.normals {
            *normal = reflect_normal(*normal);
        }
        for triangle in &mut mesh.triangles {
            triangle.swap(1, 2);
        }
    }
    shaped
}

/// The matched fields of one `SemanticGeometry::Shape`, borrowed from the entity.
pub(super) struct ShapeGeometry<'a> {
    pub(super) shape_name: &'a String,
    pub(super) code: &'a u32,
    pub(super) position: &'a Point3,
    pub(super) size: &'a f64,
    pub(super) rotation: &'a f64,
    pub(super) font: &'a Option<String>,
}

/// Outline one SHX SHAPE glyph, or report why it could not be outlined.
///
/// A shape referenced only by name (`code == 0`) is not guessed into a code, and
/// a style with no resolvable SHX shape font is `Partial`, never a silent drop.
pub(super) fn shape_fragment(
    entity: &DbEntity,
    context: &RepresentationContext,
    geometry: ShapeGeometry<'_>,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let ShapeGeometry {
        shape_name,
        code,
        position,
        size,
        rotation,
        font,
    } = geometry;
    if *code == 0 {
        representation.completeness = Completeness::Partial(vec![format!(
            "shape '{shape_name}' is referenced by name; only shape codes are resolved"
        )]);
    } else if let (Some(engine), Some(key)) = (&context.fonts, font.as_deref()) {
        match engine.shape_glyph(key, *code, *position, size.abs(), *rotation) {
            Ok(polylines) => {
                for polyline in polylines {
                    if polyline.len() >= 2 {
                        representation.fragments.push(DisplayFragment {
                            source: source.clone(),
                            geometry_source: geometry_source.clone(),
                            precision: Precision::Analytic,
                            alpha: 1.0,
                            draw_order: entity.draw_order,
                            color: DEFAULT_RENDER_COLOR,
                            color_unresolved: true,
                            lineweight: DEFAULT_LINEWEIGHT_MM,
                            lineweight_unresolved: true,
                            linetype: LinetypePattern::continuous(),
                            linetype_unresolved: true,
                            linetype_scale: 1.0,
                            primitive: DisplayPrimitive::Lines(Arc::from(
                                polyline.into_boxed_slice(),
                            )),
                        });
                    }
                }
            }
            Err(error) => {
                representation.completeness =
                    Completeness::Partial(vec![format!("shape font '{key}' unavailable: {error}")]);
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "shape.font_unavailable".into(),
                    message: error.to_string(),
                });
            }
        }
    } else {
        representation.completeness =
            Completeness::Partial(vec!["shape has no resolvable SHX shape font".into()]);
    }
}
