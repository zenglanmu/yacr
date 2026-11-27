//! Session display modes that change **how** existing geometry is drawn.
//!
//! Three tokens live here, each consumed by [`DefaultRepresentationProvider`]
//! through [`RepresentationContext`](crate::RepresentationContext):
//!
//! * `FILL` ([`DisplaySettings`](crate::RepresentationContext::fill)): a filled
//!   area draws its outline instead of its fill.
//! * `QTEXT` ([`crate::RepresentationContext::qtext`]): shaped text draws one
//!   placeholder rectangle.
//! * `VIEWRES` ([`crate::RepresentationContext::curve_segments`]): a ceiling on
//!   chord segments per curve.
//!
//! All three are **display-only**: nothing here reads or writes `cad-db`, bumps
//! a revision, records a history entry or produces a `ChangeSet`. They are the
//! derived-display half of a session token, so the same entity builds different
//! primitives without the drawing changing at all.
//!
//! # The `FILL OFF` rule
//!
//! `FILL` (`FILLMODE`) switches the *display* of a filled area, and AutoCAD's
//! documented list is multilines, traces, solids, all hatches (solid-fill
//! included) and wide polylines. This build outlines exactly the ones it stores
//! as one area with a derivable boundary:
//!
//! * a HATCH: every boundary loop the hatch itself carries (style filters the
//!   *fill* only, so the outline is the full loop set — the same rule the
//!   existing `boundary_display` derivation uses);
//! * a 2D SOLID (`AcDbSolid`): the undirected boundary edges of its triangle
//!   mesh, i.e. the edges used an odd number of times. The internal diagonals
//!   are used twice and drop out, so the outline never shows a tessellation
//!   artifact;
//! * a wide polyline (constant `width` > 0, which is also what this build's
//!   `TRACE` and `DONUT` produce): the closed offset outline of the stroke band
//!   — the same vertices the filled band is built from, joined into one loop.
//!
//! Deliberately **not** switched, and declared rather than silently kept:
//!
//! * a bare [`SemanticGeometry::Mesh`] that is not a 2D SOLID (kernel/B-rep
//!   display meshes, dimension arrowheads, gradient-internal fills). A generic
//!   triangle mesh has no "the outline" this module could name without
//!   inventing one, and the entity type is what tells a 2D SOLID apart.
//! * a WIPEOUT mask: it is not a filled area but an erase, and it has its own
//!   token (`WIPEOUTFRAME`), so `FILL` must not silently reveal it.
//! * an IMAGE, a TEXT/MTEXT glyph face, and a vendor proxy mesh: they are not
//!   filled *areas* the token governs.
//!
//! Whether a *dashed* or transparent outline keeps the hatch's resolved style is
//! not a `FILL` question: the fragments this module returns go through the same
//! `expand_entity` style resolution as every other fragment, so the outline
//! inherits the entity's colour/linetype/alpha exactly like the filled draw.

use cad_domain::{HatchGeometry, Mesh, Point3, SemanticGeometry};

use crate::{Diagnostic, DisplayFragment, DisplayPrimitive, DisplayRepresentation, ObjectId};

/// Code of the diagnostic a `QTEXT` placeholder reports when the text has no
/// derivable extent.
pub(crate) const QTEXT_EXTENT_UNKNOWN: &str = "qtext.extent_unknown";

/// The `VIEWRES` range this build accepts (AutoCAD's own 1..=1000 percentage).
pub const VIEWRES_MIN: u32 = 1;
/// The `VIEWRES` maximum.
pub const VIEWRES_MAX: u32 = 1000;
/// Floor of the segment ceiling `VIEWRES` can impose.
///
/// A closed curve needs three chords to enclose any area, so one or two
/// segments would make a circle or an ellipse vanish instead of becoming
/// coarse — a silent disappearance, not a low-resolution draw. `VIEWRES 1` and
/// `VIEWRES 2` therefore behave as `3` and the report states that floor.
pub const VIEWRES_MIN_SEGMENTS: usize = 3;

/// Turn a validated `VIEWRES` percentage into a chord-segment ceiling.
///
/// The value is clamped into [`VIEWRES_MIN_SEGMENTS`]..=[`VIEWRES_MAX`]; the
/// caller validates the `1..=1000` range first and refuses anything outside it,
/// so this is the second half of one rule, not a silent clamp of a bad value.
pub fn viewres_segment_ceiling(percent: u32) -> usize {
    (percent as usize).clamp(VIEWRES_MIN_SEGMENTS, VIEWRES_MAX as usize)
}

/// The boundary loops of a hatch as display geometry, in world space.
///
/// Identical to the boundary half of [`HatchGeometry::display`] (every loop that
/// is not a text-box/text-island fill hole, closed, in plane coordinates mapped
/// through [`cad_domain::HatchPlane`]), so an outline-only `FILL OFF` draw keeps
/// exactly the vertices — and therefore the snap points and pick parity — of the
/// filled draw.
pub(crate) fn hatch_outline_geometry(hatch: &HatchGeometry) -> Vec<SemanticGeometry> {
    let plane = hatch.plane();
    hatch
        .loops()
        .iter()
        .filter(|loop_| !loop_.is_text_island())
        .map(|loop_| SemanticGeometry::Polyline {
            points: loop_
                .points
                .iter()
                .map(|point| plane.to_world(*point))
                .collect(),
            bulges: loop_.bulges.clone(),
            closed: true,
            width: 0.0,
        })
        .collect()
}

/// The boundary of a triangle mesh as independent endpoint pairs.
///
/// Edges are matched by **vertex index**, so no floating-point equality is
/// involved: an edge is a boundary edge exactly when the triangles of the mesh
/// use it an odd number of times. A consistent closed mesh therefore keeps only
/// its outer edges (a shared internal edge cancels at two uses), and a
/// non-manifold edge (three uses) stays part of the outline rather than being
/// dropped. The pairs are deliberately unordered — they are returned as
/// [`DisplayPrimitive::LineSegments`], which draws each pair and never joins
/// neighbours, so no invented ordering can cross the area.
pub(crate) fn mesh_boundary_pairs(mesh: &Mesh) -> Vec<Point3> {
    use std::collections::BTreeMap;

    let mut uses: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for triangle in &mesh.triangles {
        for (first, second) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            if first == second {
                continue;
            }
            let key = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    let mut pairs = Vec::new();
    for ((first, second), count) in uses {
        if count % 2 == 0 {
            continue;
        }
        let (Some(a), Some(b)) = (
            mesh.vertices.get(first as usize),
            mesh.vertices.get(second as usize),
        ) else {
            // A triangle index outside the vertex list is a malformed mesh; the
            // caller keeps the filled draw and reports the degeneracy instead of
            // this module inventing a vertex.
            return Vec::new();
        };
        pairs.push(*a);
        pairs.push(*b);
    }
    pairs
}

/// A 2D SOLID mask carries an outline only when its mesh has boundary edges.
pub(crate) fn solid_outline_primitive(mesh: &Mesh) -> Option<DisplayPrimitive> {
    let pairs = mesh_boundary_pairs(mesh);
    (!pairs.is_empty()).then(|| DisplayPrimitive::LineSegments(pairs.into_boxed_slice().into()))
}

/// Replace a text entity's shaped glyph fragments with one placeholder
/// rectangle (`QTEXT`).
///
/// The rectangle is the world-axis-aligned bounding rectangle of the geometry
/// the provider was about to draw, so it is derived from the same shaped
/// outlines — no font metric is invented and no advance width is guessed. Its
/// plane sits at the minimum `z` of that extent, which is exact for the flat
/// text this build lays out (a text block on an OCS plane keeps a rectangle in
/// the world box of its glyphs; the deviation is declared, not hidden).
///
/// Returns `true` when the placeholder replaced the glyph fragments. It returns
/// `false` — keeping whatever the provider built and adding a named diagnostic —
/// when the extent cannot be derived:
///
/// * the entity produced no fragments at all (nothing to bound);
/// * the only fragment is an unshaped [`DisplayPrimitive::Text`], which carries
///   an origin and a height but no advance width, so a box around it would be a
///   fabricated width;
/// * a fragment is not a drawable point set (an image or a mask).
pub(crate) fn apply_qtext(representation: &mut DisplayRepresentation, object: u128) -> bool {
    if representation.fragments.is_empty() {
        report_extent_unknown(
            representation,
            object,
            "the text produced no geometry, so QTEXT has no extent to place a placeholder on",
        );
        return false;
    }
    let mut min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for fragment in &representation.fragments {
        let points: Vec<Point3> = match &fragment.primitive {
            DisplayPrimitive::Lines(points) | DisplayPrimitive::LineSegments(points) => {
                points.to_vec()
            }
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                mesh.vertices.clone()
            }
            DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
                let mut points = mesh.vertices.clone();
                points.extend_from_slice(edges);
                points
            }
            DisplayPrimitive::Text { .. } => {
                report_extent_unknown(
                    representation,
                    object,
                    "the text is unshaped (no font engine produced outlines), so its drawn width \
                     is unknown; QTEXT keeps the text primitive instead of inventing a box",
                );
                return false;
            }
            DisplayPrimitive::Conic(conic) => {
                // A glyph face is never a conic; refusing here keeps the rule
                // "only a drawable point set has an extent" honest.
                let _ = conic;
                report_extent_unknown(
                    representation,
                    object,
                    "the text produced analytic conic geometry, which has no chord extent to bound; \
                     QTEXT keeps it",
                );
                return false;
            }
            _ => {
                report_extent_unknown(
                    representation,
                    object,
                    "the text produced a non-geometric primitive (image/mask/instance), so QTEXT has \
                     no extent to place a placeholder on",
                );
                return false;
            }
        };
        for point in points {
            if !(point.x.is_finite() && point.y.is_finite() && point.z.is_finite()) {
                continue;
            }
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
    }
    if !(min.x.is_finite() && min.y.is_finite() && min.z.is_finite()) {
        report_extent_unknown(
            representation,
            object,
            "every shaped text vertex is non-finite, so QTEXT has no extent to place a placeholder \
             on",
        );
        return false;
    }
    let z = min.z;
    let corners = [
        Point3 {
            x: min.x,
            y: min.y,
            z,
        },
        Point3 {
            x: max.x,
            y: min.y,
            z,
        },
        Point3 {
            x: max.x,
            y: max.y,
            z,
        },
        Point3 {
            x: min.x,
            y: max.y,
            z,
        },
        Point3 {
            x: min.x,
            y: min.y,
            z,
        },
    ];
    let first = &representation.fragments[0];
    let placeholder = DisplayFragment {
        source: first.source.clone(),
        geometry_source: first.geometry_source.clone(),
        precision: first.precision.clone(),
        alpha: first.alpha,
        color: first.color,
        color_unresolved: first.color_unresolved,
        lineweight: first.lineweight,
        lineweight_unresolved: first.lineweight_unresolved,
        linetype: first.linetype.clone(),
        linetype_unresolved: first.linetype_unresolved,
        linetype_scale: first.linetype_scale,
        draw_order: first.draw_order,
        primitive: DisplayPrimitive::Lines(corners.to_vec().into_boxed_slice().into()),
    };
    representation.fragments = vec![placeholder];
    true
}

fn report_extent_unknown(representation: &mut DisplayRepresentation, object: u128, detail: &str) {
    representation.diagnostics.push(Diagnostic {
        object: Some(ObjectId(object)),
        code: QTEXT_EXTENT_UNKNOWN.into(),
        message: detail.into(),
    });
}

/// The `FILL OFF` outline of a wide polyline, or `None` when the stroke is too
/// degenerate to outline (the caller then keeps the centreline draw).
///
/// Filled and outline draws share one geometry rule (`super::stroke`): the same
/// tessellated centreline and the same mitre-clamped offsets, so the outline is
/// exactly the boundary of the band `FILL ON` fills — never a second, slightly
/// different stroke.
pub(crate) fn wide_polyline_outline(
    geometry: &SemanticGeometry,
    params: cad_geometry::TessellationParams,
    width: f64,
) -> Option<Vec<Point3>> {
    super::stroke::band_outline(geometry, params, width)
}
