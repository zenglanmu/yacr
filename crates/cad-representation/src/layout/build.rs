use std::collections::BTreeSet;

use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::*;

use crate::{
    DisplayFragment, DisplayPrimitive, DisplayRepresentation, ImageVertex, ProviderRegistry,
    RepresentationContext,
};

use super::clip::{clip_conic_to_rect, clip_mesh_to_rect, clip_polyline_to_rect};
use super::clip_reason;
use super::image::{
    clip_image_polygon_to_rect, clip_image_quad_to_rect, image_polygon_is_convex, image_quad_inside,
};
use super::selection::{ClipRefusal, ViewportState};
use super::view::viewport_transform;

/// Rebuild a borrowed fragment with a new primitive, preserving all source/style
/// fields.
///
/// The model representation is built once and reused by every supported
/// viewport, so its fragments cannot be moved; the borrowing form is the one
/// the per-viewport pass uses.
fn fragment_like(fragment: &DisplayFragment, primitive: DisplayPrimitive) -> DisplayFragment {
    DisplayFragment {
        source: fragment.source.clone(),
        geometry_source: fragment.geometry_source.clone(),
        precision: fragment.precision.clone(),
        alpha: fragment.alpha,
        color: fragment.color,
        color_unresolved: fragment.color_unresolved,
        lineweight: fragment.lineweight,
        lineweight_unresolved: fragment.lineweight_unresolved,
        linetype: fragment.linetype.clone(),
        linetype_unresolved: fragment.linetype_unresolved,
        linetype_scale: fragment.linetype_scale,
        draw_order: fragment.draw_order,
        primitive,
    }
}

/// Append a diagnostic unless an identical one (same code, message and source
/// object) is already present.
///
/// The source object is part of the key so two different entities that emit the
/// same generic explanation are both reported; only a truly repeated diagnostic
/// is collapsed.
fn push_unique_diagnostic(
    out: &mut DisplayRepresentation,
    seen: &mut BTreeSet<(String, String, Option<ObjectId>)>,
    diagnostic: Diagnostic,
) {
    let key = (
        diagnostic.code.clone(),
        diagnostic.message.clone(),
        diagnostic.object,
    );
    if seen.insert(key) {
        out.diagnostics.push(diagnostic);
    }
}

/// Record a clip degradation on the paper build: weaken completeness and attach
/// a diagnostic naming the stable reason.
///
/// The same clip limitation is reported once per layout build, not once per
/// viewport: the first viewport that hits it is the one named in the message.
/// Completeness weakening stays unconditional (it is idempotent).
fn report_clip_degradation(
    out: &mut DisplayRepresentation,
    seen: &mut BTreeSet<(String, String)>,
    refusal: &ClipRefusal,
    layout_id: LayoutId,
    viewport_index: usize,
) {
    let reason = format!("{}: {}", refusal.reason, refusal.message);
    out.completeness = weaker(
        &out.completeness,
        &Completeness::Partial(vec![reason.clone()]),
    );
    if seen.insert((refusal.reason.to_string(), refusal.message.clone())) {
        out.diagnostics.push(Diagnostic {
            object: None,
            code: "representation.viewport_clip_partial".into(),
            message: format!("layout {} viewport {viewport_index}: {reason}", layout_id.0),
        });
    }
}

/// Build the display representation for one paper-space layout (spec §3.3, F04).
///
/// This is an **on-demand** entry point: the host calls it when it switches to
/// the layout. Nothing here precomputes or caches representations for other
/// layouts, and no paper layout is built at import time; each call rebuilds only
/// the requested sheet from the database (the single authority).
///
/// The result contains, in this order:
///
/// 1. the layout's own paper-space entities, drawn as-is;
/// 2. for each **supported** rectangular viewport, the model-space entities
///    mapped onto the sheet and clipped to the viewport rectangle.
///
/// Each visible model entity is built **once** per call and the same
/// representation is reused by every supported viewport, so tessellation and
/// provider diagnostics are not repeated per viewport. Identical diagnostics
/// (same code, message and source object) are reported once; a clip limitation
/// shared by several viewports is reported once, naming the first viewport that
/// hit it.
///
/// A viewport's own [`cad_db::PaperViewport::layer_overrides`] are applied on
/// top of the caller's `visible` predicate: an entity whose layer is frozen
/// (`Some(&true)`) in that viewport contributes no geometry to it, while the
/// same layer still draws in a viewport that did not freeze it. The sheet's own
/// paper entities are never affected by a viewport override. The shared
/// `model_reps` fast path is kept: entities are built once and only the
/// per-viewport fragment pass filters, so a frozen layer costs no extra
/// tessellation.
///
/// A viewport whose state cannot be drawn exactly contributes no geometry and a
/// `Partial` report plus a `representation.viewport_unsupported` diagnostic
/// naming the stable reason — never a guessed transform. A layout that does not
/// exist is `Missing`. `visible` is the caller's layer-visibility predicate
/// (kept as a closure so this crate stays independent of `cad-app`).
///
/// For **line/fill** geometry each segment is clipped exactly (Liang–Barsky);
/// for a **mesh** each triangle is clipped exactly (Sutherland–Hodgman) with
/// per-vertex position, z, normal and colour interpolated at the new vertices;
/// for an **image** the unit-square quad is clipped and its texture coordinates
/// interpolated. When an image also carries an entity clip, that convex polygon
/// is intersected with the viewport rectangle (its UVs interpolated at the new
/// vertices) so the entity clip is never dropped or widened. Geometry the layer
/// window removes completely simply disappears. What cannot be clipped exactly
/// without faking it is kept unclipped and reported `Partial` with a stable
/// [`clip_reason`] code (an unshaped `Text` placeholder has no glyph geometry
/// until a font shapes it; a mesh with a bad index or non-finite vertex; a
/// non-convex entity image clip; an unexpanded instance) — never dropped
/// silently and never claimed as clipped.
pub fn build_paper_space(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout_id: LayoutId,
    context: &RepresentationContext,
    visible: &dyn Fn(&DbEntity) -> bool,
) -> CadResult<DisplayRepresentation> {
    let Some(layout) = database.layout(layout_id) else {
        return Ok(DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Missing(vec![format!(
                "layout {} does not exist",
                layout_id.0
            )]),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layout.missing".into(),
                message: format!("layout {} does not exist", layout_id.0),
            }],
        });
    };

    let mut out = DisplayRepresentation {
        fragments: Vec::new(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    };
    let mut diagnostic_seen: BTreeSet<(String, String, Option<ObjectId>)> = BTreeSet::new();
    let mut clip_seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut unsupported_seen: BTreeSet<(String, String)> = BTreeSet::new();

    // 1. The sheet's own geometry, already in paper coordinates.
    for entity in database.layout_entities(layout_id) {
        if !visible(entity) {
            continue;
        }
        let rep = registry.build_expanded(database, entity, context)?;
        merge(&mut out, &mut diagnostic_seen, rep);
    }

    // 2. Resolve each viewport once. An unsupported viewport contributes no
    // model geometry; identical unsupported reasons across viewports are
    // reported once, naming the first such viewport.
    let mut supported_viewports = Vec::new();
    for (index, viewport) in layout.viewports.iter().enumerate() {
        match viewport_transform(viewport) {
            ViewportState::Supported(t) => supported_viewports.push((index, t)),
            ViewportState::Unsupported(why) => {
                // The sheet itself is still drawable, so the layout is Partial,
                // not Missing; the viewport's content is simply absent.
                out.completeness = weaker(
                    &out.completeness,
                    &Completeness::Partial(vec![why.to_string()]),
                );
                let key = (
                    "representation.viewport_unsupported".to_string(),
                    why.to_string(),
                );
                if unsupported_seen.insert(key) {
                    out.diagnostics.push(Diagnostic {
                        object: None,
                        code: "representation.viewport_unsupported".into(),
                        message: format!("layout {} viewport {index}: {why}", layout_id.0),
                    });
                }
            }
        }
    }

    // 3. Build each visible model entity once, only when at least one supported
    // viewport can actually show it. With no supported viewport the model
    // contributes no geometry and no diagnostics, exactly as before. A layer
    // frozen in *every* supported viewport is skipped here (nothing can show
    // it), but a layer frozen in only some of them is still built once and
    // filtered per viewport below, so the shared-rep fast path survives.
    let model_reps: Vec<(LayerId, DisplayRepresentation)> = if supported_viewports.is_empty() {
        Vec::new()
    } else {
        database
            .model_space()
            .into_iter()
            .filter(|entity| visible(entity))
            .filter(|entity| {
                supported_viewports.iter().any(|(index, _)| {
                    layout.viewports[*index].layer_overrides.get(&entity.layer) != Some(&true)
                })
            })
            .map(|entity| {
                registry
                    .build_expanded(database, entity, context)
                    .map(|rep| (entity.layer, rep))
            })
            .collect::<CadResult<Vec<_>>>()?
    };

    for (_, rep) in &model_reps {
        out.completeness = weaker(&out.completeness, &rep.completeness);
        for diagnostic in rep.diagnostics.iter().cloned() {
            push_unique_diagnostic(&mut out, &mut diagnostic_seen, diagnostic);
        }
    }

    // 4. Each supported viewport's window into model space, reusing the model
    // representations above and dropping the layers this viewport froze.
    // Output order is sheet, viewport, model entity, fragment.
    for (index, transform) in &supported_viewports {
        let index = *index;
        let transform = transform.as_ref();
        let overrides = &layout.viewports[index].layer_overrides;
        let matrix = transform.as_transform3();
        let center = transform.paper_center;
        let half = transform.paper_half;
        for (layer, rep) in &model_reps {
            if overrides.get(layer) == Some(&true) {
                continue;
            }
            for fragment in &rep.fragments {
                let transformed = fragment.primitive.transformed(&matrix);
                match transformed {
                    DisplayPrimitive::LineSegments(points) => {
                        if points.len() % 2 != 0 {
                            return Err(CadError::InvalidInput(
                                "line segments require endpoint pairs".into(),
                            ));
                        }
                        let mut clipped = Vec::new();
                        for pair in points.chunks_exact(2) {
                            for run in clip_polyline_to_rect(pair, center, half) {
                                clipped.extend(run);
                            }
                        }
                        if !clipped.is_empty() {
                            out.fragments.push(fragment_like(
                                fragment,
                                DisplayPrimitive::LineSegments(std::sync::Arc::from(
                                    clipped.into_boxed_slice(),
                                )),
                            ));
                        }
                    }
                    DisplayPrimitive::Lines(points) => {
                        for run in clip_polyline_to_rect(&points, center, half) {
                            out.fragments.push(fragment_like(
                                fragment,
                                DisplayPrimitive::Lines(std::sync::Arc::from(
                                    run.into_boxed_slice(),
                                )),
                            ));
                        }
                    }
                    DisplayPrimitive::Mesh(mesh)
                    | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                        let overlay = matches!(&fragment.primitive, DisplayPrimitive::Mesh(_));
                        match clip_mesh_to_rect(&mesh, center, half) {
                            Ok(Some(clipped)) => {
                                out.fragments.push(fragment_like(
                                    fragment,
                                    if overlay {
                                        DisplayPrimitive::Mesh(std::sync::Arc::new(clipped))
                                    } else {
                                        DisplayPrimitive::MeshWithoutEdgeOverlay(
                                            std::sync::Arc::new(clipped),
                                        )
                                    },
                                ));
                            }
                            // Every triangle lies outside the window: an exact
                            // omission, not a degradation.
                            Ok(None) => {}
                            Err(refusal) => {
                                report_clip_degradation(
                                    &mut out,
                                    &mut clip_seen,
                                    &refusal,
                                    layout_id,
                                    index,
                                );
                                out.fragments.push(fragment_like(
                                    fragment,
                                    if overlay {
                                        DisplayPrimitive::Mesh(mesh)
                                    } else {
                                        DisplayPrimitive::MeshWithoutEdgeOverlay(mesh)
                                    },
                                ));
                            }
                        }
                    }
                    DisplayPrimitive::Text {
                        text,
                        origin,
                        font,
                        height,
                    } => {
                        // Glyph geometry exists only once a font shapes the
                        // text. An unshaped placeholder cannot be clipped here,
                        // so it is kept and reported, never dropped or faked.
                        let refusal = ClipRefusal::new(
                            clip_reason::TEXT_FONT_DEPENDENT,
                            "unshaped text glyph geometry depends on a font; the viewport cannot clip it exactly",
                        );
                        report_clip_degradation(
                            &mut out,
                            &mut clip_seen,
                            &refusal,
                            layout_id,
                            index,
                        );
                        out.fragments.push(fragment_like(
                            fragment,
                            DisplayPrimitive::Text {
                                text,
                                origin,
                                font,
                                height,
                            },
                        ));
                    }
                    DisplayPrimitive::Image {
                        resource,
                        transform: local,
                        clip,
                        adjust,
                    } => match clip_image_quad_to_rect(&local, center, half) {
                        Ok(vertices) if vertices.is_empty() => {
                            // Fully outside the window: exact omission.
                        }
                        Ok(vertices) => {
                            // A clipped textured quad is represented exactly:
                            // positions plus interpolated UVs, so it is not a
                            // degradation. A wholly visible image keeps its
                            // simple unit-square transform. When an entity clip
                            // and a viewport cut both apply, the two convex
                            // polygons are intersected so the entity clip is
                            // never silently widened or dropped; `None` (outer)
                            // means the intersection is empty — an exact
                            // omission.
                            let effective: Option<Option<std::sync::Arc<[ImageVertex]>>> =
                                if image_quad_inside(&local, center, half) {
                                    // The whole quad is visible; any entity clip
                                    // still applies unchanged.
                                    Some(clip)
                                } else if let Some(entity_clip) = clip {
                                    if image_polygon_is_convex(&entity_clip) {
                                        let intersected = clip_image_polygon_to_rect(
                                            entity_clip.to_vec(),
                                            center,
                                            half,
                                        );
                                        if intersected.is_empty() {
                                            None
                                        } else {
                                            Some(Some(std::sync::Arc::from(
                                                intersected.into_boxed_slice(),
                                            )))
                                        }
                                    } else {
                                        // A provider handed us a non-convex clip
                                        // (its primitive contract says convex);
                                        // half-plane intersection would be wrong,
                                        // so keep the entity clip and report the
                                        // limitation rather than widen or drop it.
                                        let refusal = ClipRefusal::new(
                                            clip_reason::IMAGE_INTERSECTION_UNSUPPORTED,
                                            "a non-convex image clip cannot be intersected with the viewport window exactly; the entity clip is kept unclipped",
                                        );
                                        report_clip_degradation(
                                            &mut out,
                                            &mut clip_seen,
                                            &refusal,
                                            layout_id,
                                            index,
                                        );
                                        Some(Some(entity_clip))
                                    }
                                } else {
                                    Some(Some(std::sync::Arc::from(vertices.into_boxed_slice())))
                                };
                            if let Some(clip) = effective {
                                out.fragments.push(fragment_like(
                                    fragment,
                                    DisplayPrimitive::Image {
                                        resource,
                                        transform: local,
                                        clip,
                                        adjust,
                                    },
                                ));
                            }
                        }
                        Err(refusal) => {
                            report_clip_degradation(
                                &mut out,
                                &mut clip_seen,
                                &refusal,
                                layout_id,
                                index,
                            );
                            out.fragments.push(fragment_like(
                                fragment,
                                DisplayPrimitive::Image {
                                    resource,
                                    transform: local,
                                    clip,
                                    adjust,
                                },
                            ));
                        }
                    },
                    DisplayPrimitive::Conic(conic) => {
                        match clip_conic_to_rect(&conic, center, half) {
                            Ok(pieces) => {
                                for piece in pieces {
                                    out.fragments.push(fragment_like(
                                        fragment,
                                        DisplayPrimitive::Conic(std::sync::Arc::new(piece)),
                                    ));
                                }
                            }
                            Err(refusal) => {
                                // Non-finite geometry: keep it whole and report,
                                // exactly like the other unclippable classes.
                                report_clip_degradation(
                                    &mut out,
                                    &mut clip_seen,
                                    &refusal,
                                    layout_id,
                                    index,
                                );
                                out.fragments
                                    .push(fragment_like(fragment, DisplayPrimitive::Conic(conic)));
                            }
                        }
                    }
                    other => {
                        // An unexpanded instance has no geometry to clip; keep
                        // it and report rather than fake a windowed version.
                        let refusal = ClipRefusal::new(
                            clip_reason::UNSUPPORTED_PRIMITIVE,
                            "this primitive cannot be clipped against the viewport window",
                        );
                        report_clip_degradation(
                            &mut out,
                            &mut clip_seen,
                            &refusal,
                            layout_id,
                            index,
                        );
                        out.fragments.push(fragment_like(fragment, other));
                    }
                }
            }
        }
    }

    Ok(out)
}

fn merge(
    out: &mut DisplayRepresentation,
    seen: &mut BTreeSet<(String, String, Option<ObjectId>)>,
    rep: DisplayRepresentation,
) {
    out.completeness = weaker(&out.completeness, &rep.completeness);
    for diagnostic in rep.diagnostics {
        push_unique_diagnostic(out, seen, diagnostic);
    }
    out.fragments.extend(rep.fragments);
}

fn weaker(a: &Completeness, b: &Completeness) -> Completeness {
    fn rank(c: &Completeness) -> u8 {
        match c {
            Completeness::Complete => 3,
            Completeness::Partial(_) => 1,
            Completeness::Missing(_) => 0,
            Completeness::Unverified => 2,
        }
    }
    if rank(a) <= rank(b) {
        a.clone()
    } else {
        b.clone()
    }
}
